//! Gate-routing service: resolve endpoints, assemble the per-request overlay
//! (avoid set + wormhole edges), run Dijkstra, and shape the result. No HTTP
//! types here — the handler maps `AppError` and the result DTO.

use chrono::Utc;
use petgraph::graph::NodeIndex;
use rustc_hash::FxHashSet;

use crate::dto::{
    GateRouteOutcome, GateRouteRequest, GateRouteResponse, GateRouteResult, MAX_DESTINATIONS,
    RouteStep, SystemRef,
};
use crate::error::AppError;
use crate::eve_scout::{self, EveScoutSnapshot};
use crate::model::GraphData;
use crate::routing::{Path, Preference, RouteContext, shortest_path, with_scratch};

/// Zarzakh's gate-lock mechanic strands transiting ships, so it is excluded
/// from transit by default (added to the avoid set unless opted in).
pub const ZARZAKH_SYSTEM_ID: i64 = 30100000;

/// A view of the routing knobs shared by every endpoint that drives the gate
/// graph (system routing and blops staging). Both request DTOs project into
/// this so the overlay/avoid-set assembly lives in one place.
pub struct OverlayInputs<'a> {
    pub avoid: &'a [SystemRef],
    pub include_zarzakh: bool,
    pub use_wormholes: bool,
    pub connections: &'a [crate::dto::WhConnection],
    pub include_thera: bool,
    pub include_turnur: bool,
}

impl<'a> From<&'a GateRouteRequest> for OverlayInputs<'a> {
    fn from(r: &'a GateRouteRequest) -> Self {
        Self {
            avoid: &r.avoid,
            include_zarzakh: r.include_zarzakh,
            use_wormholes: r.use_wormholes,
            connections: &r.connections,
            include_thera: r.include_thera,
            include_turnur: r.include_turnur,
        }
    }
}

/// Compute the fan-out for `req` over `graph`, drawing EVE-Scout overlay edges
/// from `scout`. The shared header (`from`, avoid set, overlay, preference) is
/// resolved **once**; then each destination in `req.to` is routed through the
/// same prepared state, collecting `{ from, results }` in request order.
///
/// A failure in the shared header (`from`/connection/avoid unresolvable, or
/// out-of-bounds `to[]`) is returned as `Err` — a request-level error the
/// handler maps to a `400`. A per-destination failure (unknown or unreachable
/// destination) is *not* an `Err`: it is carried in that destination's result
/// slot, so one bad hub never sinks the routes that resolved. Pure given its
/// inputs — the handler supplies the per-request snapshots so this stays
/// testable without HTTP or shared state.
pub fn compute_gate_route(
    graph: &GraphData,
    scout: &EveScoutSnapshot,
    req: &GateRouteRequest,
) -> Result<GateRouteResponse, AppError> {
    // ── request-level (header/bounds) tier ───────────────────────────────────
    if req.to.is_empty() {
        return Err(AppError::InvalidParam(
            "to[] must contain at least one destination".to_string(),
        ));
    }
    if req.to.len() > MAX_DESTINATIONS {
        return Err(AppError::InvalidParam(format!(
            "to[] has {} destinations; the maximum is {MAX_DESTINATIONS}",
            req.to.len()
        )));
    }

    let preference: Preference = req.preference.into();
    let from = resolve_system(graph, &req.from)?;

    let inputs = OverlayInputs::from(req);
    // The avoid set is about *transit*: the route's endpoints are never transit
    // hops, so a system is usable as `from`/`to` even when otherwise excluded
    // (e.g. Zarzakh by default, or a user-avoided system). `from` is shared, so
    // remove it once here; each destination is removed per-iteration below.
    let mut shared_avoid = assemble_avoid_set(graph, &inputs)?;
    shared_avoid.remove(&from);

    // The user wormhole connections are part of the *shared* header, so an
    // unknown connection system is a request-level error — resolved (and
    // validated) once here, not folded into a per-destination slot. EVE-Scout
    // sigs are added per destination (their inclusion depends on the endpoint
    // set) and silently skip unknowns, so they never fail the request.
    let connections = resolve_connections(graph, &inputs)?;

    // ── per-destination tier ──────────────────────────────────────────────────
    // Each destination resolves and routes against the prepared shared state;
    // its failures stay local to its result slot.
    let results = req
        .to
        .iter()
        .map(|to_ref| {
            let outcome = compute_one(
                graph,
                scout,
                &inputs,
                preference,
                &shared_avoid,
                &connections,
                from,
                to_ref,
            )
            .unwrap_or_else(|e| {
                let (code, message) = e.code_message();
                GateRouteOutcome::Failure {
                    error: code.to_string(),
                    message,
                }
            });
            GateRouteResult {
                to: to_ref.clone(),
                outcome,
            }
        })
        .collect();

    Ok(GateRouteResponse {
        from: req.from.clone(),
        results,
    })
}

/// Resolve the shared user wormhole connections to node-index pairs once, so an
/// unknown connection system surfaces as a request-level error rather than being
/// charged to one destination. Empty when `use_wormholes` is off.
fn resolve_connections(
    graph: &GraphData,
    inputs: &OverlayInputs<'_>,
) -> Result<Vec<(NodeIndex, NodeIndex)>, AppError> {
    if !inputs.use_wormholes {
        return Ok(Vec::new());
    }
    inputs
        .connections
        .iter()
        .map(|c| {
            let a = resolve_system(graph, &c.from)?;
            let b = resolve_system(graph, &c.to)?;
            Ok((a, b))
        })
        .collect()
}

/// Route the shared `from` to one destination `to_ref` against the prepared
/// shared overlay/avoid/preference. Returns the route outcome on success, or an
/// `AppError` (unknown/unreachable destination) the caller folds into that
/// destination's result slot.
#[allow(clippy::too_many_arguments)]
fn compute_one(
    graph: &GraphData,
    scout: &EveScoutSnapshot,
    inputs: &OverlayInputs<'_>,
    preference: Preference,
    shared_avoid: &FxHashSet<NodeIndex>,
    connections: &[(NodeIndex, NodeIndex)],
    from: NodeIndex,
    to_ref: &SystemRef,
) -> Result<GateRouteOutcome, AppError> {
    let to = resolve_system(graph, to_ref)?;

    // Start from the shared avoid set and exempt this destination as an endpoint.
    let mut avoid = shared_avoid.clone();
    avoid.remove(&to);
    let mut ctx = RouteContext::new(graph, avoid, preference);

    // Pre-resolved shared connections, then the endpoint-dependent scout edges.
    for &(a, b) in connections {
        ctx.add_connection(a, b);
    }
    apply_scout_overlay(graph, scout, inputs, &[from, to], &mut ctx);

    let path = with_scratch(|scratch| shortest_path(&ctx, from, to, scratch))
        .ok_or(AppError::Unreachable)?;

    let steps = build_steps(graph, &ctx, &path);
    // jumps = edges traversed = path.len() - 1 (0 for a same-system route).
    let jumps = path.len().saturating_sub(1);
    Ok(GateRouteOutcome::Route { jumps, path: steps })
}

/// User `avoid[]` ∪ Zarzakh (unless `include_zarzakh`). Zarzakh is added only
/// when the SDE actually contains it, so a fixture without it is unaffected.
/// Shared by the system-route and blops-staging services.
pub fn assemble_avoid_set(
    graph: &GraphData,
    inputs: &OverlayInputs<'_>,
) -> Result<FxHashSet<NodeIndex>, AppError> {
    let mut avoid = inputs
        .avoid
        .iter()
        .map(|r| resolve_system(graph, r))
        .collect::<Result<FxHashSet<_>, _>>()?;
    if !inputs.include_zarzakh
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
/// hub is one of the route's own `endpoints`. The flag governs using the hub as
/// a mid-route shortcut; reaching the hub as an endpoint must not require opting
/// in — Thera in particular is a gateless wormhole, so without its sigs it would
/// be unreachable as a source or destination. Shared by both services;
/// `endpoints` is the set of nodes exempt from the include-flag requirement
/// (the system route's `from`/`to`; for blops, A and B).
pub fn apply_overlay(
    graph: &GraphData,
    scout: &EveScoutSnapshot,
    inputs: &OverlayInputs<'_>,
    endpoints: &[NodeIndex],
    ctx: &mut RouteContext<'_>,
) -> Result<(), AppError> {
    if inputs.use_wormholes {
        for c in inputs.connections {
            let a = resolve_system(graph, &c.from)?;
            let b = resolve_system(graph, &c.to)?;
            ctx.add_connection(a, b);
        }
    }
    apply_scout_overlay(graph, scout, inputs, endpoints, ctx);
    Ok(())
}

/// Add just the EVE-Scout Thera/Turnur signature edges to the overlay (the
/// endpoint-dependent half of `apply_overlay`). Split out so the system-route
/// fan-out can resolve the user `connections[]` once at the header tier — where
/// an unknown connection is a request-level error — while still applying the
/// scout edges per destination, whose inclusion depends on that destination's
/// endpoint set. Infallible: unknown systems and stale sigs are silently
/// skipped (see `add_scout_edges`), so this never fails the request.
pub fn apply_scout_overlay(
    graph: &GraphData,
    scout: &EveScoutSnapshot,
    inputs: &OverlayInputs<'_>,
    endpoints: &[NodeIndex],
    ctx: &mut RouteContext<'_>,
) {
    let now = Utc::now();
    let is_endpoint = |hub_id: i64| {
        graph
            .id_to_idx
            .get(&hub_id)
            .is_some_and(|&idx| endpoints.contains(&idx))
    };
    if inputs.include_thera || is_endpoint(eve_scout::THERA_SYSTEM_ID) {
        add_scout_edges(graph, ctx, &scout.thera, now);
    }
    if inputs.include_turnur || is_endpoint(eve_scout::TURNUR_SYSTEM_ID) {
        add_scout_edges(graph, ctx, &scout.turnur, now);
    }
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
pub fn resolve_system(graph: &GraphData, r: &SystemRef) -> Result<NodeIndex, AppError> {
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

/// Turn a node path into labelled `RouteStep`s (`start` / `stargate` /
/// `wormhole`). Shared so the blops gate leg labels steps identically.
pub fn build_steps(graph: &GraphData, ctx: &RouteContext<'_>, path: &Path) -> Vec<RouteStep> {
    path.iter()
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
        .collect()
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
            to: vec![SystemRef::Name(to.into())],
            preference: RoutePreference::Shortest,
            avoid: vec![],
            use_wormholes: false,
            connections: vec![],
            include_thera: false,
            include_turnur: false,
            include_zarzakh: false,
        }
    }

    /// Extract the single route outcome from a one-destination fan-out, or panic
    /// with the in-slot failure — most legacy single-route tests expect a route.
    fn one_route(resp: &GateRouteResponse) -> (usize, &[RouteStep]) {
        assert_eq!(
            resp.results.len(),
            1,
            "expected a single-destination fan-out"
        );
        match &resp.results[0].outcome {
            GateRouteOutcome::Route { jumps, path } => (*jumps, path),
            GateRouteOutcome::Failure { error, message } => {
                panic!("expected a route, got failure {error}: {message}")
            }
        }
    }

    /// Extract the single in-slot failure code from a one-destination fan-out.
    fn one_failure(resp: &GateRouteResponse) -> &str {
        assert_eq!(
            resp.results.len(),
            1,
            "expected a single-destination fan-out"
        );
        match &resp.results[0].outcome {
            GateRouteOutcome::Failure { error, .. } => error,
            GateRouteOutcome::Route { .. } => panic!("expected a failure, got a route"),
        }
    }

    #[test]
    fn resolvable_route_by_name() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let mut r = req("G", "H");
        r.include_zarzakh = true;
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        let (jumps, path) = one_route(&resp);
        assert_eq!(jumps, 2);
        assert_eq!(path[0].via, "start");
        assert_eq!(path[1].via, "stargate");
        assert_eq!(path[1].system, "Zarzakh");
    }

    #[test]
    fn unknown_source_is_request_level_error() {
        // A bad *shared* `from` is a header-tier failure: the whole request errs
        // (handler → 400), it is not folded into a per-destination slot.
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let err = compute_gate_route(&gd, &scout, &req("Nowhere", "H")).unwrap_err();
        assert!(matches!(err, AppError::UnknownSystem(s) if s == "Nowhere"));
    }

    #[test]
    fn unknown_destination_is_in_slot_failure() {
        // A bad *destination* stays local to its slot — the request still
        // succeeds (would be a 200 at the handler).
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let resp = compute_gate_route(&gd, &scout, &req("H", "Nowhere")).unwrap();
        assert_eq!(one_failure(&resp), "unknown_system");
    }

    #[test]
    fn unreachable_destination_is_in_slot_failure() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let resp = compute_gate_route(&gd, &scout, &req("Jita", "H")).unwrap();
        assert_eq!(one_failure(&resp), "unreachable");
    }

    #[test]
    fn zarzakh_excluded_by_default() {
        // G→H only routes via Zarzakh; default excludes it → unreachable, carried
        // in the destination's slot.
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let resp = compute_gate_route(&gd, &scout, &req("G", "H")).unwrap();
        assert_eq!(one_failure(&resp), "unreachable");
    }

    #[test]
    fn zarzakh_opt_in_allows_transit() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let mut r = req("G", "H");
        r.include_zarzakh = true;
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        let (_, path) = one_route(&resp);
        assert!(path.iter().any(|s| s.system == "Zarzakh"));
    }

    #[test]
    fn zarzakh_usable_as_destination_without_opt_in() {
        // Excluding Zarzakh is about transit, not endpoints: routing TO Zarzakh
        // must succeed even with the default exclusion.
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let resp = compute_gate_route(&gd, &scout, &req("G", "Zarzakh")).unwrap();
        let (jumps, path) = one_route(&resp);
        assert_eq!(jumps, 1);
        assert_eq!(path.last().unwrap().system, "Zarzakh");
    }

    #[test]
    fn zarzakh_usable_as_source_without_opt_in() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let resp = compute_gate_route(&gd, &scout, &req("Zarzakh", "H")).unwrap();
        let (jumps, path) = one_route(&resp);
        assert_eq!(jumps, 1);
        assert_eq!(path[0].system, "Zarzakh");
    }

    #[test]
    fn zarzakh_endpoint_still_not_a_transit_hop() {
        // Zarzakh as an endpoint is fine, but it must still never be transited:
        // G→H (Zarzakh the only bridge) is unreachable by default.
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let resp = compute_gate_route(&gd, &scout, &req("G", "H")).unwrap();
        assert_eq!(one_failure(&resp), "unreachable");
    }

    #[test]
    fn user_avoid_routes_around() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let mut r = req("G", "H");
        r.include_zarzakh = true;
        r.avoid = vec![SystemRef::Name("Zarzakh".into())];
        // Avoiding the only bridge → unreachable even with include_zarzakh.
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        assert_eq!(one_failure(&resp), "unreachable");
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
        let (_, path) = one_route(&resp);
        assert_eq!(path.last().unwrap().system, "Zarzakh");
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
        let (jumps, path) = one_route(&resp);
        assert_eq!(jumps, 1);
        assert_eq!(path[1].via, "wormhole");
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
        // Without use_wormholes the edge is not added → still unreachable (in slot).
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        assert_eq!(one_failure(&resp), "unreachable");
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
        let (_, path) = one_route(&resp);
        // Thera→G via wormhole, then G→Zarzakh→H via gates.
        assert_eq!(path[1].via, "wormhole");
        assert_eq!(path[1].system, "G");
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
        // The destination is unreachable once the stale sig is dropped → in slot.
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        assert_eq!(one_failure(&resp), "unreachable");
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
        let (jumps, path) = one_route(&resp);
        assert_eq!(jumps, 1);
        assert_eq!(path.last().unwrap().system, "Thera");
        assert_eq!(path.last().unwrap().via, "wormhole");
    }

    #[test]
    fn thera_usable_as_source_without_include_flag() {
        let (gd, scout) = scout_hub_graph();
        let r = req("Thera", "G");
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        let (jumps, path) = one_route(&resp);
        assert_eq!(jumps, 1);
        assert_eq!(path[0].system, "Thera");
    }

    #[test]
    fn turnur_endpoint_adds_its_sigs_without_include_flag() {
        // Turnur has K-space gates, but its EVE-Scout edges should still be
        // available when it is an endpoint: H→Turnur resolves via the sig.
        let (gd, scout) = scout_hub_graph();
        let r = req("H", "Turnur");
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        let (_, path) = one_route(&resp);
        assert_eq!(path.last().unwrap().system, "Turnur");
        assert_eq!(path.last().unwrap().via, "wormhole");
    }

    #[test]
    fn thera_not_a_mid_route_shortcut_without_include_flag() {
        // When Thera is NOT an endpoint and include_thera is false, its sigs are
        // not added, so it can't be used as a transit shortcut. Routing G→H must
        // use the gate (1 jump), never detour through Thera.
        let (gd, scout) = scout_hub_graph();
        let resp = compute_gate_route(&gd, &scout, &req("G", "H")).unwrap();
        let (jumps, path) = one_route(&resp);
        assert_eq!(jumps, 1);
        assert!(!path.iter().any(|s| s.system == "Thera"));
    }

    #[test]
    fn same_system_route_is_zero_jumps() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let resp = compute_gate_route(&gd, &scout, &req("Jita", "Jita")).unwrap();
        let (jumps, path) = one_route(&resp);
        assert_eq!(jumps, 0);
        assert_eq!(path.len(), 1);
    }

    // ── fan-out ───────────────────────────────────────────────────────────────

    /// Build a fan-out request with a shared `from` and several destinations.
    fn fanout(from: &str, to: &[&str]) -> GateRouteRequest {
        GateRouteRequest {
            to: to.iter().map(|t| SystemRef::Name((*t).into())).collect(),
            ..req(from, "unused")
        }
    }

    #[test]
    fn fanout_echoes_from_and_answers_in_request_order() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let mut r = fanout("G", &["Zarzakh", "G"]);
        r.include_zarzakh = true;
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        // `from` echoed once, as sent.
        assert!(matches!(&resp.from, SystemRef::Name(n) if n == "G"));
        // One entry per destination, in request order, each echoing its `to`.
        assert_eq!(resp.results.len(), 2);
        assert!(matches!(&resp.results[0].to, SystemRef::Name(n) if n == "Zarzakh"));
        assert!(matches!(&resp.results[1].to, SystemRef::Name(n) if n == "G"));
        // G→Zarzakh is 1 jump; G→G is 0.
        assert!(matches!(
            resp.results[0].outcome,
            GateRouteOutcome::Route { jumps: 1, .. }
        ));
        assert!(matches!(
            resp.results[1].outcome,
            GateRouteOutcome::Route { jumps: 0, .. }
        ));
    }

    #[test]
    fn fanout_mixes_success_and_per_destination_failure() {
        // A good destination, an unknown one, and an unreachable one — all in one
        // request that still succeeds; the failures stay in their own slots.
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let mut r = fanout("G", &["Zarzakh", "Nowhere", "Jita"]);
        r.include_zarzakh = true;
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        assert_eq!(resp.results.len(), 3);
        assert!(matches!(
            resp.results[0].outcome,
            GateRouteOutcome::Route { .. }
        ));
        assert!(matches!(
            &resp.results[1].outcome,
            GateRouteOutcome::Failure { error, .. } if error == "unknown_system"
        ));
        assert!(matches!(
            &resp.results[2].outcome,
            GateRouteOutcome::Failure { error, .. } if error == "unreachable"
        ));
        // The unknown destination still echoes its `to` as sent.
        assert!(matches!(&resp.results[1].to, SystemRef::Name(n) if n == "Nowhere"));
    }

    #[test]
    fn fanout_empty_to_is_request_level_error() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let err = compute_gate_route(&gd, &scout, &fanout("G", &[])).unwrap_err();
        assert!(matches!(err, AppError::InvalidParam(_)));
    }

    #[test]
    fn fanout_over_cap_is_request_level_error() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let mut r = req("G", "Zarzakh");
        r.to = vec![SystemRef::Name("Zarzakh".into()); MAX_DESTINATIONS + 1];
        let err = compute_gate_route(&gd, &scout, &r).unwrap_err();
        assert!(matches!(err, AppError::InvalidParam(_)));
    }

    #[test]
    fn fanout_at_cap_is_accepted() {
        // Exactly MAX_DESTINATIONS is within bounds (the cap rejects only `>`).
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let mut r = req("G", "Zarzakh");
        r.include_zarzakh = true;
        r.to = vec![SystemRef::Name("Zarzakh".into()); MAX_DESTINATIONS];
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        assert_eq!(resp.results.len(), MAX_DESTINATIONS);
    }

    #[test]
    fn fanout_bad_connection_is_request_level_error() {
        // A user connection is part of the *shared* overlay, so an unknown
        // connection system fails the whole request (header tier), not one slot —
        // even though every destination is individually valid.
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let mut r = fanout("Jita", &["G", "Zarzakh"]);
        r.use_wormholes = true;
        r.connections = vec![WhConnection {
            from: SystemRef::Name("Jita".into()),
            to: SystemRef::Name("Nowhere".into()),
            max_size: None,
        }];
        let err = compute_gate_route(&gd, &scout, &r).unwrap_err();
        assert!(matches!(err, AppError::UnknownSystem(s) if s == "Nowhere"));
    }

    #[test]
    fn fanout_bad_from_fails_whole_request_before_any_route() {
        // A bad shared `from` is a header-tier failure even though every `to`
        // is individually fine: the request errs rather than reporting per-slot.
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let r = fanout("Nowhere", &["G", "H", "Zarzakh"]);
        let err = compute_gate_route(&gd, &scout, &r).unwrap_err();
        assert!(matches!(err, AppError::UnknownSystem(s) if s == "Nowhere"));
    }

    #[test]
    fn fanout_duplicate_destinations_answered_positionally() {
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let mut r = fanout("G", &["Zarzakh", "Zarzakh"]);
        r.include_zarzakh = true;
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        assert_eq!(resp.results.len(), 2);
        assert!(matches!(
            resp.results[0].outcome,
            GateRouteOutcome::Route { jumps: 1, .. }
        ));
        assert!(matches!(
            resp.results[1].outcome,
            GateRouteOutcome::Route { jumps: 1, .. }
        ));
    }

    #[test]
    fn fanout_per_destination_avoid_exemption_is_independent() {
        // The endpoint avoid-exemption is per-destination: avoiding Zarzakh makes
        // G→H unreachable (the only bridge), yet G→Zarzakh still resolves because
        // Zarzakh is exempt *as that entry's destination*. One shared avoid set
        // must not let one destination's exemption leak into another's.
        let gd = zarzakh_graph();
        let scout = EveScoutSnapshot::default();
        let mut r = fanout("G", &["Zarzakh", "H"]);
        r.include_zarzakh = true;
        r.avoid = vec![SystemRef::Name("Zarzakh".into())];
        let resp = compute_gate_route(&gd, &scout, &r).unwrap();
        // Zarzakh as a destination: exempt → resolves.
        assert!(matches!(
            resp.results[0].outcome,
            GateRouteOutcome::Route { jumps: 1, .. }
        ));
        // H via Zarzakh transit: Zarzakh avoided → unreachable, in its own slot.
        assert!(matches!(
            &resp.results[1].outcome,
            GateRouteOutcome::Failure { error, .. } if error == "unreachable"
        ));
    }
}
