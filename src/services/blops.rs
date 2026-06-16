//! Parameterized staging service: "get a fleet from A into bridge range of a
//! fixed target B". Blops-agnostic by design — it takes an effective range and
//! two security predicates (a "B must satisfy" check and a "★ acceptable"
//! check), never the word "blops". The handler owns the blops specifics (the
//! cyno-at-B rule, the worst-Black-Ops default). A future titan-bridge endpoint
//! is then another thin handler over this same service.
//!
//! Algorithm: validate B against its predicate, run a kd-tree radius query
//! around B to collect in-range K-space candidates, then a single multi-target
//! Dijkstra from A settles every candidate's gate distance in one pass. Rank by
//! (fewest gate jumps, then closest light-years to B); the head is ★.

use crate::dto::{
    BlopsBridge, BlopsCandidate, BlopsChosen, BlopsRouteResponse, BlopsSystem, SystemRef,
};
use crate::error::AppError;
use crate::eve_scout::EveScoutSnapshot;
use crate::model::{GraphData, System};
use crate::range::{ly_between, radius_m2};
use crate::routing::{Preference, RouteContext, shortest_path_multi, with_scratch};
use crate::services::route::{
    OverlayInputs, apply_overlay, assemble_avoid_set, build_steps, resolve_system,
};

/// How many fallback candidates (beyond ★) to return. Small so the hot path
/// stays lean; the FC just needs a couple of next-bests if ★ is blocked.
pub const MAX_ALTERNATES: usize = 5;

/// Inputs to the staging search, assembled by the handler. `dest_ok` is the
/// "B must satisfy" predicate (blops: B not highsec). `stage_ok` accepts a
/// candidate ★ (blops: any K-space, so always true). Keeping them as two
/// separate predicates is the whole point — the directional rule must never be
/// flattened into one filter.
pub struct StagingQuery<'a> {
    pub from: &'a SystemRef,
    pub to: &'a SystemRef,
    pub effective_ly: f64,
    pub overlay: OverlayInputs<'a>,
    pub preference: Preference,
}

/// Compute a staging route. `dest_ok(B)` gates the target up front (a failing
/// B yields `on_dest_reject`); `stage_ok(★candidate)` filters candidates.
///
/// The two predicates are deliberately independent: the caller supplies the
/// directional security rule as two functions, and this service never assumes
/// they are the same.
pub fn compute_staging<DestOk, StageOk>(
    graph: &GraphData,
    scout: &EveScoutSnapshot,
    query: &StagingQuery<'_>,
    dest_ok: DestOk,
    stage_ok: StageOk,
    on_dest_reject: impl FnOnce(&System) -> AppError,
) -> Result<BlopsRouteResponse, AppError>
where
    DestOk: Fn(&System) -> bool,
    StageOk: Fn(&System) -> bool,
{
    let from = resolve_system(graph, query.from)?;
    let to = resolve_system(graph, query.to)?;

    // 1. Validate B (the cyno destination) up front, before any routing — a
    //    wrong target should fail fast and distinctly.
    let b_sys = &graph.systems[to.index()];
    if !dest_ok(b_sys) {
        return Err(on_dest_reject(b_sys));
    }

    // 2. Radius query around B: in-range K-space candidates. The kd-tree holds
    //    K-space only, so J-space is already excluded. The radius is in squared
    //    metres to match the kd-tree's squared-distance metric.
    let b_coords = b_sys.coords;
    let radius = radius_m2(query.effective_ly);
    let in_range = graph
        .spatial_index
        .within_unsorted::<kiddo::SquaredEuclidean>(&b_coords, radius);

    // Apply the ★ predicate (blops: any K-space). B itself is always within
    // range of itself (distance 0); exclude it — B is the cyno destination,
    // never its own staging origin.
    let candidates: Vec<petgraph::graph::NodeIndex> = in_range
        .iter()
        .map(|nn| petgraph::graph::NodeIndex::new(nn.item))
        .filter(|&idx| idx != to)
        .filter(|idx| stage_ok(&graph.systems[idx.index()]))
        .collect();

    if candidates.is_empty() {
        return Err(AppError::NoStagingInRange);
    }

    // 3. Assemble the overlay (avoid set + wormhole edges) for the A→★ gate
    //    leg, exactly as the system route does. A and B are the exempt
    //    endpoints for the EVE-Scout include-flag rule.
    let mut avoid = assemble_avoid_set(graph, &query.overlay)?;
    // The fleet's own location is never a transit hop.
    avoid.remove(&from);
    let mut ctx = RouteContext::new(graph, avoid, query.preference);
    apply_overlay(graph, scout, &query.overlay, &[from, to], &mut ctx)?;

    // 4. One multi-target Dijkstra from A settles every candidate's gate
    //    distance. Candidates that are avoided/blocked simply don't come back.
    let mut settled = with_scratch(|scratch| shortest_path_multi(&ctx, from, &candidates, scratch));
    if settled.is_empty() {
        return Err(AppError::StagingUnreachable);
    }

    // 5. Rank by (fewest gate jumps, then closest light-years to B). Ranking
    //    uses full-precision distances so near-ties order correctly; the
    //    response rounds (see `round2`) only at the wire boundary.
    let ly_to_b =
        |node: petgraph::graph::NodeIndex| ly_between(graph.systems[node.index()].coords, b_coords);
    settled.sort_by(|a, b| {
        a.jumps.cmp(&b.jumps).then_with(|| {
            ly_to_b(a.node)
                .partial_cmp(&ly_to_b(b.node))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });

    // 6. Head is ★; reconstruct its gate path and attach the bridge leg.
    let chosen_settled = &settled[0];
    let star = chosen_settled.node;
    let gate_path = build_steps(graph, &ctx, &chosen_settled.path);
    let bridge = BlopsBridge {
        from: blops_system(&graph.systems[star.index()]),
        to: blops_system(b_sys),
        jump_ly: round2(ly_to_b(star)),
        // A chosen ★ came from the in-range query, so it is within range.
        in_range: true,
    };
    let chosen = BlopsChosen {
        gate_jumps: chosen_settled.jumps,
        gate_path,
        bridge,
    };

    // 7. The next-best candidates beyond ★ become inline fallbacks — but only
    //    when the fleet actually has to drive. A zero-jump ★ means A is already
    //    in bridge range: there is nothing to fall back *to* (the alternates
    //    exist to cover "★ is blocked, drive elsewhere instead"), so suppress
    //    them and just bridge.
    let alternates: Vec<BlopsCandidate> = if chosen_settled.jumps == 0 {
        Vec::new()
    } else {
        settled
            .iter()
            .skip(1)
            .take(MAX_ALTERNATES)
            .map(|t| BlopsCandidate {
                system: blops_system(&graph.systems[t.node.index()]),
                gate_jumps: t.jumps,
                ly_to_b: round2(ly_to_b(t.node)),
            })
            .collect()
    };

    Ok(BlopsRouteResponse {
        chosen,
        alternates,
        // These are echoed by the handler (it knows the defaulting); fill
        // placeholders the handler overwrites. Kept here so the service owns
        // the shape and the handler only annotates.
        jdc_level: 0,
        effective_ly: query.effective_ly,
        defaulted: false,
    })
}

fn blops_system(s: &System) -> BlopsSystem {
    BlopsSystem {
        system: s.name.clone(),
        system_id: s.id,
        security: s.security,
        sec_class: s.sec_class.label().to_string(),
    }
}

/// Round a light-year distance to two decimals for the wire response. Applied
/// only at the response boundary — ranking uses full precision.
fn round2(ly: f64) -> f64 {
    (ly * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{Connection, ConnectionKind};
    use crate::graph::build_graph_data;
    use crate::model::{RawSdeData, SecClass, System};
    use crate::range::{LY_IN_METERS, effective_ly};

    /// Place a system at `x` light-years along the X axis (other axes 0).
    fn sys_at(id: i64, name: &str, sec_class: SecClass, x_ly: f64) -> System {
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
            coords: [x_ly * LY_IN_METERS, 0.0, 0.0],
            region_id,
            constellation_id: 1,
        }
    }

    fn overlay_defaults<'a>() -> OverlayInputs<'a> {
        OverlayInputs {
            avoid: &[],
            include_zarzakh: false,
            use_wormholes: false,
            use_bridges: false,
            connections: &[],
            include_thera: false,
            include_turnur: false,
        }
    }

    // Blops predicates, as the handler supplies them.
    fn dest_not_highsec(s: &System) -> bool {
        s.sec_class != SecClass::Highsec
    }
    fn stage_any_kspace(_s: &System) -> bool {
        true // ★ is unrestricted across K-space
    }
    fn reject_highsec(s: &System) -> AppError {
        AppError::CynoTargetHighsec(s.name.clone())
    }

    fn query<'a>(from: &'a SystemRef, to: &'a SystemRef, eff_ly: f64) -> StagingQuery<'a> {
        StagingQuery {
            from,
            to,
            effective_ly: eff_ly,
            overlay: overlay_defaults(),
            preference: Preference::Shortest,
        }
    }

    /// A(high) — S1(low) — S2(low) — B(null), strung along the X axis so that
    /// S1 and S2 sit within bridge range of B but A is far. Gate jumps grow
    /// A→S1→S2→B. Coordinates double as the LY geometry for the bridge.
    fn staging_graph() -> GraphData {
        // B at x=0; S2 1 LY away; S1 2 LY; A 10 LY (out of range).
        let raw = RawSdeData {
            systems: vec![
                sys_at(1, "A", SecClass::Highsec, 10.0),
                sys_at(2, "S1", SecClass::Lowsec, 2.0),
                sys_at(3, "S2", SecClass::Lowsec, 1.0),
                sys_at(4, "B", SecClass::Nullsec, 0.0),
            ],
            gate_pairs: vec![(1, 2), (2, 3), (3, 4)],
            hulls: Default::default(),
        };
        build_graph_data(raw, 0)
    }

    #[test]
    fn fewest_jumps_wins_then_closest() {
        // Range 3 LY covers S1 (2 LY) and S2 (1 LY) but not A (10 LY). From A,
        // S1 is 1 gate jump, S2 is 2 — so S1 (fewer jumps) is ★ even though S2
        // is closer to B.
        let gd = staging_graph();
        let scout = EveScoutSnapshot::default();
        let (from, to) = (SystemRef::Name("A".into()), SystemRef::Name("B".into()));
        let resp = compute_staging(
            &gd,
            &scout,
            &query(&from, &to, 3.0),
            dest_not_highsec,
            stage_any_kspace,
            reject_highsec,
        )
        .unwrap();
        assert_eq!(resp.chosen.bridge.from.system, "S1");
        assert_eq!(resp.chosen.gate_jumps, 1);
        assert!(resp.chosen.bridge.in_range);
        // S2 is the fallback (2 jumps), closer to B but more jumps.
        assert_eq!(resp.alternates.len(), 1);
        assert_eq!(resp.alternates[0].system.system, "S2");
        assert_eq!(resp.alternates[0].gate_jumps, 2);
    }

    #[test]
    fn zero_jump_chosen_suppresses_alternates() {
        // A is already in bridge range of B (A = S1, 2 LY, range 3 LY): ★ is A
        // itself at zero gate jumps. There is nothing to drive to, so the
        // fallback list is suppressed — even though S2 (1 gate jump from S1)
        // would otherwise qualify as an alternate.
        let gd = staging_graph();
        let scout = EveScoutSnapshot::default();
        let (from, to) = (SystemRef::Name("S1".into()), SystemRef::Name("B".into()));
        let resp = compute_staging(
            &gd,
            &scout,
            &query(&from, &to, 3.0),
            dest_not_highsec,
            stage_any_kspace,
            reject_highsec,
        )
        .unwrap();
        assert_eq!(resp.chosen.gate_jumps, 0, "already in range");
        assert_eq!(resp.chosen.bridge.from.system, "S1");
        assert_eq!(resp.chosen.gate_path.len(), 1, "path is just A itself");
        assert!(resp.chosen.bridge.in_range);
        assert!(
            resp.alternates.is_empty(),
            "no alternates when no driving is required, got {:?}",
            resp.alternates
        );
    }

    #[test]
    fn ties_broken_by_closest_to_target() {
        // Two candidates equidistant in gate jumps from A; the closer-to-B wins.
        // A(high) gated to C1 and C2 (both 1 jump); C1 at 2 LY, C2 at 1 LY.
        let raw = RawSdeData {
            systems: vec![
                sys_at(1, "A", SecClass::Highsec, 10.0),
                sys_at(2, "C1", SecClass::Nullsec, 2.0),
                sys_at(3, "C2", SecClass::Nullsec, 1.0),
                sys_at(4, "B", SecClass::Nullsec, 0.0),
            ],
            gate_pairs: vec![(1, 2), (1, 3)],
            hulls: Default::default(),
        };
        let gd = build_graph_data(raw, 0);
        let scout = EveScoutSnapshot::default();
        let (from, to) = (SystemRef::Name("A".into()), SystemRef::Name("B".into()));
        let resp = compute_staging(
            &gd,
            &scout,
            &query(&from, &to, 3.0),
            dest_not_highsec,
            stage_any_kspace,
            reject_highsec,
        )
        .unwrap();
        // Both are 1 jump; C2 (1 LY) is closer than C1 (2 LY) → C2 is ★.
        assert_eq!(resp.chosen.bridge.from.system, "C2");
        assert_eq!(resp.alternates[0].system.system, "C1");
    }

    #[test]
    fn highsec_target_b_is_rejected_distinctly() {
        // The directional rule, half one: B highsec → the dest predicate fails.
        let raw = RawSdeData {
            systems: vec![
                sys_at(1, "A", SecClass::Lowsec, 10.0),
                sys_at(2, "S", SecClass::Lowsec, 1.0),
                sys_at(4, "B", SecClass::Highsec, 0.0),
            ],
            gate_pairs: vec![(1, 2), (2, 4)],
            hulls: Default::default(),
        };
        let gd = build_graph_data(raw, 0);
        let scout = EveScoutSnapshot::default();
        let (from, to) = (SystemRef::Name("A".into()), SystemRef::Name("B".into()));
        let err = compute_staging(
            &gd,
            &scout,
            &query(&from, &to, 3.0),
            dest_not_highsec,
            stage_any_kspace,
            reject_highsec,
        )
        .unwrap_err();
        assert!(matches!(err, AppError::CynoTargetHighsec(s) if s == "B"));
    }

    #[test]
    fn highsec_staging_origin_is_accepted() {
        // The directional rule, half two: a highsec ★ is fine (bridging OUT of
        // highsec is legal). B is null; the only in-range candidate is highsec.
        // Asserting this passes while the previous test rejects highsec B proves
        // the two predicates are separate — a single "exclude highsec" filter
        // could not satisfy both.
        let raw = RawSdeData {
            systems: vec![
                sys_at(1, "A", SecClass::Highsec, 10.0),
                sys_at(2, "S", SecClass::Highsec, 1.0), // highsec staging ★
                sys_at(4, "B", SecClass::Nullsec, 0.0),
            ],
            gate_pairs: vec![(1, 2), (2, 4)],
            hulls: Default::default(),
        };
        let gd = build_graph_data(raw, 0);
        let scout = EveScoutSnapshot::default();
        let (from, to) = (SystemRef::Name("A".into()), SystemRef::Name("B".into()));
        let resp = compute_staging(
            &gd,
            &scout,
            &query(&from, &to, 3.0),
            dest_not_highsec,
            stage_any_kspace,
            reject_highsec,
        )
        .unwrap();
        assert_eq!(resp.chosen.bridge.from.system, "S");
        assert_eq!(resp.chosen.bridge.from.sec_class, "Highsec");
    }

    #[test]
    fn no_candidate_in_range_is_distinct_from_ungateable() {
        // Range 0.5 LY: nothing is within range of B → NoStagingInRange.
        let gd = staging_graph();
        let scout = EveScoutSnapshot::default();
        let (from, to) = (SystemRef::Name("A".into()), SystemRef::Name("B".into()));
        let err = compute_staging(
            &gd,
            &scout,
            &query(&from, &to, 0.5),
            dest_not_highsec,
            stage_any_kspace,
            reject_highsec,
        )
        .unwrap_err();
        assert!(matches!(err, AppError::NoStagingInRange));
    }

    #[test]
    fn in_range_but_ungateable_is_staging_unreachable() {
        // S is in range of B but gate-disconnected from A → StagingUnreachable,
        // NOT NoStagingInRange (a candidate exists; the fleet just can't reach
        // it).
        let raw = RawSdeData {
            systems: vec![
                sys_at(1, "A", SecClass::Highsec, 10.0),
                sys_at(2, "S", SecClass::Nullsec, 1.0),
                sys_at(4, "B", SecClass::Nullsec, 0.0),
            ],
            gate_pairs: vec![(2, 4)], // A is isolated
            hulls: Default::default(),
        };
        let gd = build_graph_data(raw, 0);
        let scout = EveScoutSnapshot::default();
        let (from, to) = (SystemRef::Name("A".into()), SystemRef::Name("B".into()));
        let err = compute_staging(
            &gd,
            &scout,
            &query(&from, &to, 3.0),
            dest_not_highsec,
            stage_any_kspace,
            reject_highsec,
        )
        .unwrap_err();
        assert!(matches!(err, AppError::StagingUnreachable));
    }

    #[test]
    fn fallbacks_are_populated_and_capped() {
        // Many in-range candidates, all 1 gate jump from A → alternates list is
        // populated (and never exceeds MAX_ALTERNATES).
        let mut systems = vec![sys_at(1, "A", SecClass::Highsec, 10.0)];
        let mut gate_pairs = vec![];
        // 8 candidates at increasing LY, all gated directly to A.
        for i in 0..8 {
            let id = 100 + i;
            systems.push(sys_at(
                id,
                &format!("C{i}"),
                SecClass::Nullsec,
                0.5 + i as f64 * 0.1,
            ));
            gate_pairs.push((1, id));
        }
        systems.push(sys_at(999, "B", SecClass::Nullsec, 0.0));
        let gd = build_graph_data(
            RawSdeData {
                systems,
                gate_pairs,
                hulls: Default::default(),
            },
            0,
        );
        let scout = EveScoutSnapshot::default();
        let (from, to) = (SystemRef::Name("A".into()), SystemRef::Name("B".into()));
        let resp = compute_staging(
            &gd,
            &scout,
            &query(&from, &to, 5.0),
            dest_not_highsec,
            stage_any_kspace,
            reject_highsec,
        )
        .unwrap();
        // 8 candidates → 1 chosen + up to MAX_ALTERNATES fallbacks.
        assert_eq!(resp.alternates.len(), MAX_ALTERNATES);
        // The chosen one is the closest (C0 at 0.5 LY), fallbacks ascend in LY.
        assert_eq!(resp.chosen.bridge.from.system, "C0");
        for w in resp.alternates.windows(2) {
            assert!(w[0].ly_to_b <= w[1].ly_to_b);
        }
    }

    #[test]
    fn avoid_can_block_a_candidate_path() {
        // S1—S2—B with A—S1; range covers S1 and S2. Avoiding S1 makes S2
        // ungateable (its only gate path runs through S1) but S1 stays as ★.
        let gd = staging_graph();
        let avoid = [SystemRef::Name("S1".into())];
        let scout = EveScoutSnapshot::default();
        let (from, to) = (SystemRef::Name("A".into()), SystemRef::Name("B".into()));
        let mut q = query(&from, &to, 3.0);
        q.overlay.avoid = &avoid;
        // Avoiding S1 blocks the only route to both S1 and S2 → unreachable.
        let err = compute_staging(
            &gd,
            &scout,
            &q,
            dest_not_highsec,
            stage_any_kspace,
            reject_highsec,
        )
        .unwrap_err();
        assert!(matches!(err, AppError::StagingUnreachable));
    }

    #[test]
    fn effective_range_geometry_matches_catalog_math() {
        // Sanity wiring: a 1.5 LY base hull at JDC 5 (+20%/lvl) → 3.0 LY, which
        // covers S1 (2 LY) and S2 (1 LY). Confirms the range used by the kd
        // query is the catalog formula, not an ad-hoc number.
        let gd = staging_graph();
        let scout = EveScoutSnapshot::default();
        let eff = effective_ly(1.5, 0.20, 5);
        assert_eq!(eff, 3.0);
        let (from, to) = (SystemRef::Name("A".into()), SystemRef::Name("B".into()));
        let resp = compute_staging(
            &gd,
            &scout,
            &query(&from, &to, eff),
            dest_not_highsec,
            stage_any_kspace,
            reject_highsec,
        )
        .unwrap();
        assert_eq!(resp.chosen.bridge.from.system, "S1");
    }

    #[test]
    fn wormhole_overlay_applies_to_gate_leg() {
        // A is isolated from S by gates, but a user WH A↔S makes S reachable in
        // 1 jump, labelled wormhole. Proves the overlay reaches the staging leg.
        let raw = RawSdeData {
            systems: vec![
                sys_at(1, "A", SecClass::Highsec, 10.0),
                sys_at(2, "S", SecClass::Nullsec, 1.0),
                sys_at(4, "B", SecClass::Nullsec, 0.0),
            ],
            gate_pairs: vec![(2, 4)], // A isolated by gates
            hulls: Default::default(),
        };
        let gd = build_graph_data(raw, 0);
        let scout = EveScoutSnapshot::default();
        let conns = [Connection {
            kind: ConnectionKind::Wormhole,
            from: SystemRef::Name("A".into()),
            to: SystemRef::Name("S".into()),
            max_size: None,
        }];
        let (from, to) = (SystemRef::Name("A".into()), SystemRef::Name("B".into()));
        let mut q = query(&from, &to, 3.0);
        q.overlay.use_wormholes = true;
        q.overlay.connections = &conns;
        let resp = compute_staging(
            &gd,
            &scout,
            &q,
            dest_not_highsec,
            stage_any_kspace,
            reject_highsec,
        )
        .unwrap();
        assert_eq!(resp.chosen.bridge.from.system, "S");
        assert_eq!(resp.chosen.gate_jumps, 1);
        // The hop into S is a wormhole, not a gate.
        assert_eq!(resp.chosen.gate_path[1].via, "wormhole");
    }

    #[test]
    fn bridge_overlay_applies_to_gate_leg() {
        // Same isolated-A scenario, but the shortcut is a bridge gated by
        // use_bridges; the hop into S is labelled `bridge`.
        let raw = RawSdeData {
            systems: vec![
                sys_at(1, "A", SecClass::Highsec, 10.0),
                sys_at(2, "S", SecClass::Nullsec, 1.0),
                sys_at(4, "B", SecClass::Nullsec, 0.0),
            ],
            gate_pairs: vec![(2, 4)], // A isolated by gates
            hulls: Default::default(),
        };
        let gd = build_graph_data(raw, 0);
        let scout = EveScoutSnapshot::default();
        let conns = [Connection {
            kind: ConnectionKind::Bridge,
            from: SystemRef::Name("A".into()),
            to: SystemRef::Name("S".into()),
            max_size: None,
        }];
        let (from, to) = (SystemRef::Name("A".into()), SystemRef::Name("B".into()));
        let mut q = query(&from, &to, 3.0);
        q.overlay.use_bridges = true;
        q.overlay.connections = &conns;
        let resp = compute_staging(
            &gd,
            &scout,
            &q,
            dest_not_highsec,
            stage_any_kspace,
            reject_highsec,
        )
        .unwrap();
        assert_eq!(resp.chosen.bridge.from.system, "S");
        assert_eq!(resp.chosen.gate_jumps, 1);
        assert_eq!(resp.chosen.gate_path[1].via, "bridge");
    }

    #[test]
    fn round2_rounds_to_two_decimals() {
        assert_eq!(round2(5.806705857679252), 5.81);
        assert_eq!(round2(7.554), 7.55);
        assert_eq!(round2(7.555), 7.56);
        assert_eq!(round2(2.0), 2.0);
    }

    #[test]
    fn response_jump_ly_is_rounded() {
        // Place the staging candidate at a non-round distance (3.456 LY) and
        // confirm the response reports the two-decimal jump distance.
        let raw = RawSdeData {
            systems: vec![
                sys_at(1, "A", SecClass::Highsec, 10.0),
                sys_at(2, "S", SecClass::Nullsec, 3.456),
                sys_at(4, "B", SecClass::Nullsec, 0.0),
            ],
            gate_pairs: vec![(1, 2), (2, 4)],
            hulls: Default::default(),
        };
        let gd = build_graph_data(raw, 0);
        let scout = EveScoutSnapshot::default();
        let (from, to) = (SystemRef::Name("A".into()), SystemRef::Name("B".into()));
        let resp = compute_staging(
            &gd,
            &scout,
            &query(&from, &to, 5.0),
            dest_not_highsec,
            stage_any_kspace,
            reject_highsec,
        )
        .unwrap();
        assert_eq!(resp.chosen.bridge.jump_ly, 3.46, "rounded to two decimals");
    }
}
