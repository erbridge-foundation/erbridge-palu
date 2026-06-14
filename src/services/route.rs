//! Gate-routing service: resolve endpoints, assemble the per-request overlay
//! (avoid set + wormhole edges), run Dijkstra, and shape the result. No HTTP
//! types here — the handler maps `AppError` and the result DTO.

use chrono::Utc;
use petgraph::graph::NodeIndex;
use rustc_hash::FxHashSet;

use crate::dto::{GateRouteRequest, GateRouteResponse, RouteStep, SystemRef};
use crate::error::AppError;
use crate::eve_scout::{self, EveScoutSnapshot};
use crate::model::GraphData;
use crate::routing::{Path, Preference, RouteContext, shortest_path, with_scratch};

/// Zarzakh's gate-lock mechanic strands transiting ships, so it is excluded
/// from transit by default (added to the avoid set unless opted in).
pub const ZARZAKH_SYSTEM_ID: i64 = 30100000;

/// Compute a gate route for `req` over `graph`, drawing EVE-Scout overlay edges
/// from `scout`. Pure given its inputs — the handler supplies the per-request
/// snapshots so this stays testable without HTTP or shared state.
pub fn compute_gate_route(
    graph: &GraphData,
    scout: &EveScoutSnapshot,
    req: &GateRouteRequest,
) -> Result<GateRouteResponse, AppError> {
    let preference: Preference = req.preference.into();

    let from = resolve_system(graph, &req.from)?;
    let to = resolve_system(graph, &req.to)?;

    let mut avoid = assemble_avoid_set(graph, req)?;
    // The avoid set is about *transit*: the route's own endpoints are never
    // transit hops, so a system is usable as `from`/`to` even when it would
    // otherwise be excluded (e.g. Zarzakh by default, or a user-avoided system).
    avoid.remove(&from);
    avoid.remove(&to);
    let mut ctx = RouteContext::new(graph, avoid, preference);

    apply_overlay(graph, scout, req, from, to, &mut ctx)?;

    let path = with_scratch(|scratch| shortest_path(&ctx, from, to, scratch))
        .ok_or(AppError::Unreachable)?;

    Ok(build_response(graph, &ctx, &path))
}

/// User `avoid[]` ∪ Zarzakh (unless `include_zarzakh`). Zarzakh is added only
/// when the SDE actually contains it, so a fixture without it is unaffected.
fn assemble_avoid_set(
    graph: &GraphData,
    req: &GateRouteRequest,
) -> Result<FxHashSet<NodeIndex>, AppError> {
    let mut avoid = req
        .avoid
        .iter()
        .map(|r| resolve_system(graph, r))
        .collect::<Result<FxHashSet<_>, _>>()?;
    if !req.include_zarzakh
        && let Some(&zar) = graph.id_to_idx.get(&ZARZAKH_SYSTEM_ID)
    {
        avoid.insert(zar);
    }
    Ok(avoid)
}

/// Add wormhole edges to the overlay from user connections and live EVE-Scout
/// Thera/Turnur signatures. Expired sigs are dropped here.
///
/// A hub's signatures are added when its `include_*` flag is set OR when that
/// hub is the route's own `from`/`to`. The flag governs using the hub as a
/// mid-route shortcut; reaching the hub as an endpoint must not require opting
/// in — Thera in particular is a gateless wormhole, so without its sigs it would
/// be unreachable as a source or destination.
fn apply_overlay(
    graph: &GraphData,
    scout: &EveScoutSnapshot,
    req: &GateRouteRequest,
    from: NodeIndex,
    to: NodeIndex,
    ctx: &mut RouteContext<'_>,
) -> Result<(), AppError> {
    if req.use_wormholes {
        for c in &req.connections {
            let a = resolve_system(graph, &c.from)?;
            let b = resolve_system(graph, &c.to)?;
            ctx.add_connection(a, b);
        }
    }

    let now = Utc::now();
    let is_endpoint = |hub_id: i64| {
        graph
            .id_to_idx
            .get(&hub_id)
            .is_some_and(|&idx| idx == from || idx == to)
    };
    if req.include_thera || is_endpoint(eve_scout::THERA_SYSTEM_ID) {
        add_scout_edges(graph, ctx, &scout.thera, now);
    }
    if req.include_turnur || is_endpoint(eve_scout::TURNUR_SYSTEM_ID) {
        add_scout_edges(graph, ctx, &scout.turnur, now);
    }
    Ok(())
}

/// Add live signatures as overlay edges. Unknown systems and expired sigs are
/// silently skipped — EVE-Scout occasionally reports systems or stale sigs we
/// can't or shouldn't route through; failing the whole request would be wrong.
fn add_scout_edges(
    graph: &GraphData,
    ctx: &mut RouteContext<'_>,
    sigs: &[crate::eve_scout::Signature],
    now: chrono::DateTime<Utc>,
) {
    for sig in sigs {
        if !sig.is_live(now) {
            continue;
        }
        let (Some(&out), Some(&in_)) = (
            graph.id_to_idx.get(&sig.out_system_id),
            graph.id_to_idx.get(&sig.in_system_id),
        ) else {
            continue;
        };
        ctx.add_connection(out, in_);
    }
}

/// Resolve a system reference to its node index, case-insensitively for names.
fn resolve_system(graph: &GraphData, r: &SystemRef) -> Result<NodeIndex, AppError> {
    match r {
        SystemRef::Id(id) => graph
            .id_to_idx
            .get(id)
            .copied()
            .ok_or_else(|| AppError::UnknownSystem(id.to_string())),
        SystemRef::Name(name) => graph
            .name_to_idx
            .get(&name.to_ascii_lowercase())
            .copied()
            .ok_or_else(|| AppError::UnknownSystem(name.clone())),
    }
}

fn build_response(graph: &GraphData, ctx: &RouteContext<'_>, path: &Path) -> GateRouteResponse {
    let steps: Vec<RouteStep> = path
        .iter()
        .enumerate()
        .map(|(i, idx)| {
            let s = &graph.systems[idx.index()];
            let via = if i == 0 {
                "start"
            } else {
                ctx.step_via(path[i - 1], *idx)
            };
            RouteStep {
                system: s.name.clone(),
                system_id: s.id,
                security: s.security,
                sec_class: s.sec_class.label().to_string(),
                via: via.to_string(),
            }
        })
        .collect();
    // jumps = edges traversed = path.len() - 1 (0 for a same-system route).
    let jumps = path.len().saturating_sub(1);
    GateRouteResponse { jumps, path: steps }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{RoutePreference, WhConnection};
    use crate::eve_scout::Signature;
    use crate::graph::build_graph_data;
    use crate::model::{RawSdeData, SecClass, System};

    fn sys(id: i64, name: &str, sec_class: SecClass) -> System {
        let (security, region_id) = match sec_class {
            SecClass::Highsec => (0.9, 10000002),
            SecClass::Lowsec => (0.3, 10000002),
            SecClass::Nullsec => (-0.5, 10000012),
            SecClass::Wormhole => (-0.99, 11000001),
        };
        System {
            id,
            name: name.into(),
            security,
            sec_class,
            coords: [id as f64, 0.0, 0.0],
            region_id,
            constellation_id: 1,
        }
    }

    /// G(null) — Zarzakh — H(null), plus isolated Jita for unknown/unreachable.
    fn zarzakh_graph() -> GraphData {
        let raw = RawSdeData {
            systems: vec![
                sys(1, "G", SecClass::Nullsec),
                sys(ZARZAKH_SYSTEM_ID, "Zarzakh", SecClass::Nullsec),
                sys(2, "H", SecClass::Nullsec),
                sys(30000142, "Jita", SecClass::Highsec),
            ],
            gate_pairs: vec![(1, ZARZAKH_SYSTEM_ID), (ZARZAKH_SYSTEM_ID, 2)],
            hulls: Default::default(),
        };
        build_graph_data(raw, 1)
    }

    fn req(from: &str, to: &str) -> GateRouteRequest {
        GateRouteRequest {
            from: SystemRef::Name(from.into()),
            to: SystemRef::Name(to.into()),
            preference: RoutePreference::Shortest,
            avoid: vec![],
            use_wormholes: false,
            connections: vec![],
            include_thera: false,
            include_turnur: false,
            include_zarzakh: false,
        }
    }

    #[test]
    fn resolvable_route_by_name() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let mut r = req("G", "H");
        r.include_zarzakh = true;
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        assert_eq!(resp.jumps, 2);
        assert_eq!(resp.path[0].via, "start");
        assert_eq!(resp.path[1].via, "stargate");
        assert_eq!(resp.path[1].system, "Zarzakh");
    }

    #[test]
    fn unknown_system_errors() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let err = compute_gate_route(&gd, &scout, &req("Nowhere", "H")).unwrap_err();
        assert!(matches!(err, AppError::UnknownSystem(s) if s == "Nowhere"));
    }

    #[test]
    fn unreachable_errors() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let err = compute_gate_route(&gd, &scout, &req("Jita", "H")).unwrap_err();
        assert!(matches!(err, AppError::Unreachable));
    }

    #[test]
    fn zarzakh_excluded_by_default() {
        // G→H only routes via Zarzakh; default excludes it → unreachable.
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let err = compute_gate_route(&gd, &scout, &req("G", "H")).unwrap_err();
        assert!(matches!(err, AppError::Unreachable));
    }

    #[test]
    fn zarzakh_opt_in_allows_transit() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let mut r = req("G", "H");
        r.include_zarzakh = true;
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        assert!(resp.path.iter().any(|s| s.system == "Zarzakh"));
    }

    #[test]
    fn zarzakh_usable_as_destination_without_opt_in() {
        // Excluding Zarzakh is about transit, not endpoints: routing TO Zarzakh
        // must succeed even with the default exclusion.
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let resp = compute_gate_route(&gd, &scout, &req("G", "Zarzakh")).unwrap();
        assert_eq!(resp.jumps, 1);
        assert_eq!(resp.path.last().unwrap().system, "Zarzakh");
    }

    #[test]
    fn zarzakh_usable_as_source_without_opt_in() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let resp = compute_gate_route(&gd, &scout, &req("Zarzakh", "H")).unwrap();
        assert_eq!(resp.jumps, 1);
        assert_eq!(resp.path[0].system, "Zarzakh");
    }

    #[test]
    fn zarzakh_endpoint_still_not_a_transit_hop() {
        // Zarzakh as an endpoint is fine, but it must still never be transited:
        // G→H (Zarzakh the only bridge) is unreachable by default.
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let err = compute_gate_route(&gd, &scout, &req("G", "H")).unwrap_err();
        assert!(matches!(err, AppError::Unreachable));
    }

    #[test]
    fn user_avoid_routes_around() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let mut r = req("G", "H");
        r.include_zarzakh = true;
        r.avoid = vec![SystemRef::Name("Zarzakh".into())];
        // Avoiding the only bridge → unreachable even with include_zarzakh.
        let err = compute_gate_route(&gd, &scout, &r).unwrap_err();
        assert!(matches!(err, AppError::Unreachable));
    }

    #[test]
    fn avoided_endpoint_is_still_usable() {
        // The from/to exemption applies to any avoided system, not just Zarzakh:
        // routing TO an explicitly-avoided endpoint still resolves.
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let mut r = req("G", "Zarzakh");
        r.include_zarzakh = true;
        r.avoid = vec![SystemRef::Name("Zarzakh".into())];
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        assert_eq!(resp.path.last().unwrap().system, "Zarzakh");
    }

    #[test]
    fn user_connection_creates_shortcut_labelled_wormhole() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let mut r = req("Jita", "G");
        r.use_wormholes = true;
        r.connections = vec![WhConnection {
            from: SystemRef::Name("Jita".into()),
            to: SystemRef::Name("G".into()),
            max_size: Some("large".into()),
        }];
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        assert_eq!(resp.jumps, 1);
        assert_eq!(resp.path[1].via, "wormhole");
    }

    #[test]
    fn connections_ignored_when_use_wormholes_false() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let mut r = req("Jita", "G");
        r.use_wormholes = false;
        r.connections = vec![WhConnection {
            from: SystemRef::Name("Jita".into()),
            to: SystemRef::Name("G".into()),
            max_size: None,
        }];
        // Without use_wormholes the edge is not added → still unreachable.
        assert!(matches!(
            compute_gate_route(&gd, &scout, &r).unwrap_err(),
            AppError::Unreachable
        ));
    }

    #[test]
    fn connection_unknown_system_errors() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let mut r = req("Jita", "G");
        r.use_wormholes = true;
        r.connections = vec![WhConnection {
            from: SystemRef::Name("Jita".into()),
            to: SystemRef::Name("Nowhere".into()),
            max_size: None,
        }];
        assert!(matches!(
            compute_gate_route(&gd, &scout, &r).unwrap_err(),
            AppError::UnknownSystem(_)
        ));
    }

    fn live_sig(out: i64, in_id: i64) -> Signature {
        Signature {
            out_system_id: out,
            in_system_id: in_id,
            in_system_name: format!("S{in_id}"),
            max_ship_size: None,
            expires_at: Utc::now() + chrono::Duration::hours(2),
        }
    }

    #[test]
    fn include_thera_injects_overlay_edge() {
        // Jita is isolated; a Thera→Jita sig is meaningless without Thera in
        // the graph, so instead model Thera(out)→G(in) and route Thera→H.
        let raw = RawSdeData {
            systems: vec![
                sys(31000005, "Thera", SecClass::Wormhole),
                sys(1, "G", SecClass::Nullsec),
                sys(ZARZAKH_SYSTEM_ID, "Zarzakh", SecClass::Nullsec),
                sys(2, "H", SecClass::Nullsec),
            ],
            gate_pairs: vec![(1, ZARZAKH_SYSTEM_ID), (ZARZAKH_SYSTEM_ID, 2)],
            hulls: Default::default(),
        };
        let gd = build_graph_data(raw, 1);
        let scout = EveScoutSnapshot {
            thera: vec![live_sig(31000005, 1)],
            turnur: vec![],
            fetched_at: Some(Utc::now()),
        };
        let mut r = req("Thera", "H");
        r.include_thera = true;
        r.include_zarzakh = true;
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        // Thera→G via wormhole, then G→Zarzakh→H via gates.
        assert_eq!(resp.path[1].via, "wormhole");
        assert_eq!(resp.path[1].system, "G");
    }

    #[test]
    fn expired_thera_sig_is_dropped() {
        let raw = RawSdeData {
            systems: vec![
                sys(31000005, "Thera", SecClass::Wormhole),
                sys(1, "G", SecClass::Nullsec),
            ],
            gate_pairs: vec![],
            hulls: Default::default(),
        };
        let gd = build_graph_data(raw, 1);
        let expired = Signature {
            expires_at: Utc::now() - chrono::Duration::hours(1),
            ..live_sig(31000005, 1)
        };
        let scout = EveScoutSnapshot {
            thera: vec![expired],
            turnur: vec![],
            fetched_at: Some(Utc::now()),
        };
        let mut r = req("Thera", "G");
        r.include_thera = true;
        assert!(matches!(
            compute_gate_route(&gd, &scout, &r).unwrap_err(),
            AppError::Unreachable
        ));
    }

    /// Thera (gateless WH) and Turnur (k-space) plus two K-space systems.
    /// EVE-Scout: Thera→G and Turnur→H. Used by the endpoint-exemption tests.
    fn scout_hub_graph() -> (GraphData, EveScoutSnapshot) {
        let raw = RawSdeData {
            systems: vec![
                sys(eve_scout::THERA_SYSTEM_ID, "Thera", SecClass::Wormhole),
                sys(eve_scout::TURNUR_SYSTEM_ID, "Turnur", SecClass::Lowsec),
                sys(1, "G", SecClass::Nullsec),
                sys(2, "H", SecClass::Nullsec),
            ],
            gate_pairs: vec![(1, 2)], // G—H gate; Thera/Turnur reachable only via sigs here
            hulls: Default::default(),
        };
        let gd = build_graph_data(raw, 1);
        let scout = EveScoutSnapshot {
            thera: vec![live_sig(eve_scout::THERA_SYSTEM_ID, 1)], // Thera↔G
            turnur: vec![live_sig(eve_scout::TURNUR_SYSTEM_ID, 2)], // Turnur↔H
            fetched_at: Some(Utc::now()),
        };
        (gd, scout)
    }

    #[test]
    fn thera_reachable_as_destination_without_include_flag() {
        // Thera is a gateless wormhole; routing TO it must add its sigs even
        // though include_thera is false (endpoint != mid-route shortcut).
        let (gd, scout) = scout_hub_graph();
        let r = req("G", "Thera"); // include_thera defaults to false
        assert!(!r.include_thera);
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        assert_eq!(resp.jumps, 1);
        assert_eq!(resp.path.last().unwrap().system, "Thera");
        assert_eq!(resp.path.last().unwrap().via, "wormhole");
    }

    #[test]
    fn thera_usable_as_source_without_include_flag() {
        let (gd, scout) = scout_hub_graph();
        let r = req("Thera", "G");
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        assert_eq!(resp.jumps, 1);
        assert_eq!(resp.path[0].system, "Thera");
    }

    #[test]
    fn turnur_endpoint_adds_its_sigs_without_include_flag() {
        // Turnur has K-space gates, but its EVE-Scout edges should still be
        // available when it is an endpoint: H→Turnur resolves via the sig.
        let (gd, scout) = scout_hub_graph();
        let r = req("H", "Turnur");
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        assert_eq!(resp.path.last().unwrap().system, "Turnur");
        assert_eq!(resp.path.last().unwrap().via, "wormhole");
    }

    #[test]
    fn thera_not_a_mid_route_shortcut_without_include_flag() {
        // When Thera is NOT an endpoint and include_thera is false, its sigs are
        // not added, so it can't be used as a transit shortcut. Routing G→H must
        // use the gate (1 jump), never detour through Thera.
        let (gd, scout) = scout_hub_graph();
        let resp = compute_gate_route(&gd, &scout, &req("G", "H")).unwrap();
        assert_eq!(resp.jumps, 1);
        assert!(!resp.path.iter().any(|s| s.system == "Thera"));
    }

    #[test]
    fn same_system_route_is_zero_jumps() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let resp = compute_gate_route(&gd, &scout, &req("Jita", "Jita")).unwrap();
        assert_eq!(resp.jumps, 0);
        assert_eq!(resp.path.len(), 1);
    }
}
