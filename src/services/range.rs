//! Jump-range reachability service: the fan-out inverse of staging. Given a
//! source system and an effective jump range, return every K-space system
//! reachable in a single jump. Mechanic-agnostic — it knows nothing of
//! "jump"/"bridge"/"conduit"; the handler resolves the hull and range.
//!
//! Algorithm: one kd-tree radius query around the source (no gate routing, no
//! overlay — a jump does not traverse gates), then filter and shape. The set
//! excludes J-space (the kd-tree holds K-space only), highsec (a cyno cannot be
//! lit there, so no jump may land), and the source itself (a 0-LY non-result).

use std::collections::BTreeMap;

use petgraph::graph::NodeIndex;

use crate::dto::{RangeHull, RangeReachable, RangeResponse, RangeSummary, RangeSystem};
use crate::model::{GraphData, SecClass, System};
use crate::range::{ly_between, radius_m2};

/// Compute the reachable set from `source` at `effective_ly`. `hull` is echoed
/// into the response unchanged; `jdc_level`/`effective_ly` are echoed by the
/// caller. No `Result` — an empty reachable set is a valid response, not an
/// error.
pub fn compute_reachable(
    graph: &GraphData,
    source: NodeIndex,
    hull: RangeHull,
    jdc_level: u8,
    effective_ly: f64,
) -> RangeResponse {
    let src_sys = &graph.systems[source.index()];
    let src_coords = src_sys.coords;

    // Radius query around the source. The kd-tree is K-space only (J-space
    // already excluded), and `within_unsorted` returns squared-metre distances
    // to match `radius_m2`.
    let radius = radius_m2(effective_ly);
    let hits = graph
        .spatial_index
        .within_unsorted::<kiddo::SquaredEuclidean>(&src_coords, radius);

    let mut reachable: Vec<(RangeReachable, f64)> = hits
        .iter()
        .map(|nn| NodeIndex::new(nn.item))
        // Drop the source itself (0 LY — a non-result).
        .filter(|&idx| idx != source)
        .map(|idx| &graph.systems[idx.index()])
        // Drop highsec: a cyno cannot be lit there, so no jump may land.
        .filter(|s| s.sec_class != SecClass::Highsec)
        .map(|s| {
            let ly = ly_between(s.coords, src_coords);
            (
                RangeReachable {
                    system: s.name.clone(),
                    system_id: s.id,
                    security: s.security,
                    sec_class: s.sec_class.label().to_string(),
                    jump_ly: round2(ly),
                },
                ly,
            )
        })
        .collect();

    // Sort by full-precision distance so near-ties order correctly; the wire
    // value is rounded.
    reachable.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let summary = build_summary(&reachable);
    let reachable = reachable.into_iter().map(|(r, _)| r).collect();

    RangeResponse {
        source: range_system(src_sys),
        hull,
        jdc_level,
        effective_ly,
        summary,
        reachable,
    }
}

/// Summary over the sorted `(entry, full_ly)` pairs: count, farthest distance,
/// and the per-security-class tally. `farthest_ly` is the last element's
/// distance because the input is sorted ascending; `0.0` for an empty set.
fn build_summary(reachable: &[(RangeReachable, f64)]) -> RangeSummary {
    let farthest_ly = reachable.last().map(|(_, ly)| round2(*ly)).unwrap_or(0.0);
    let mut by_sec_class: BTreeMap<String, usize> = BTreeMap::new();
    for (r, _) in reachable {
        *by_sec_class.entry(r.sec_class.clone()).or_insert(0) += 1;
    }
    RangeSummary {
        reachable_count: reachable.len(),
        farthest_ly,
        by_sec_class,
    }
}

fn range_system(s: &System) -> RangeSystem {
    RangeSystem {
        system: s.name.clone(),
        system_id: s.id,
        security: s.security,
        sec_class: s.sec_class.label().to_string(),
    }
}

/// Round a light-year distance to two decimals for the wire response. Applied
/// only at the boundary — sorting and `farthest` use full precision.
fn round2(ly: f64) -> f64 {
    (ly * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::build_graph_data;
    use crate::model::{RawSdeData, SecClass};
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

    fn hull(base_ly: f64) -> RangeHull {
        RangeHull {
            name: "Test".into(),
            type_id: 1,
            base_ly,
        }
    }

    /// Resolve a node by name in a built graph (test convenience).
    fn node(gd: &GraphData, name: &str) -> NodeIndex {
        gd.name_to_idx[&name.to_ascii_lowercase()]
    }

    /// S(null) at origin; neighbours strung along X at increasing LY, with a mix
    /// of classes including a highsec one that must be excluded.
    fn fan_graph() -> GraphData {
        let raw = RawSdeData {
            systems: vec![
                sys_at(1, "S", SecClass::Nullsec, 0.0),
                sys_at(2, "Near", SecClass::Lowsec, 1.0),
                sys_at(3, "Mid", SecClass::Nullsec, 2.0),
                sys_at(4, "HighInRange", SecClass::Highsec, 1.5),
                sys_at(5, "FarOut", SecClass::Nullsec, 9.0),
            ],
            // Gate topology is irrelevant to a jump fan-out; include none.
            gate_pairs: vec![],
            hulls: Default::default(),
        };
        build_graph_data(raw, 0)
    }

    #[test]
    fn reaches_in_range_kspace_excluding_source_and_highsec() {
        let gd = fan_graph();
        // Range 3 LY covers Near(1), Mid(2), HighInRange(1.5) but not FarOut(9).
        let resp = compute_reachable(&gd, node(&gd, "S"), hull(3.0), 5, 3.0);
        let names: Vec<&str> = resp.reachable.iter().map(|r| r.system.as_str()).collect();
        // Source excluded; highsec excluded; FarOut out of range.
        assert_eq!(names, vec!["Near", "Mid"]);
        assert!(!names.contains(&"S"), "source excluded");
        assert!(!names.contains(&"HighInRange"), "highsec excluded");
        assert!(!names.contains(&"FarOut"), "out of range");
    }

    #[test]
    fn reachable_sorted_ascending_with_rounded_distance() {
        let gd = fan_graph();
        let resp = compute_reachable(&gd, node(&gd, "S"), hull(3.0), 5, 3.0);
        assert_eq!(resp.reachable[0].system, "Near");
        assert_eq!(resp.reachable[0].jump_ly, 1.0);
        assert_eq!(resp.reachable[1].system, "Mid");
        assert_eq!(resp.reachable[1].jump_ly, 2.0);
        for w in resp.reachable.windows(2) {
            assert!(w[0].jump_ly <= w[1].jump_ly);
        }
    }

    #[test]
    fn reachability_ignores_the_gate_graph() {
        // Mid is in range but gate-disconnected from S (no gate_pairs at all);
        // a jump does not use gates, so it must still be reachable.
        let gd = fan_graph();
        let resp = compute_reachable(&gd, node(&gd, "S"), hull(3.0), 5, 3.0);
        assert!(resp.reachable.iter().any(|r| r.system == "Mid"));
    }

    #[test]
    fn summary_reports_count_farthest_and_class_breakdown() {
        let gd = fan_graph();
        let resp = compute_reachable(&gd, node(&gd, "S"), hull(3.0), 5, 3.0);
        assert_eq!(resp.summary.reachable_count, 2);
        assert_eq!(resp.summary.farthest_ly, 2.0, "Mid at 2 LY is farthest");
        assert_eq!(resp.summary.by_sec_class.get("Lowsec"), Some(&1));
        assert_eq!(resp.summary.by_sec_class.get("Nullsec"), Some(&1));
        assert_eq!(resp.summary.by_sec_class.get("Highsec"), None);
    }

    #[test]
    fn empty_reachable_set_is_a_valid_response() {
        // Range 0.5 LY reaches nothing; the response is well-formed and empty,
        // not an error.
        let gd = fan_graph();
        let resp = compute_reachable(&gd, node(&gd, "S"), hull(0.25), 5, 0.5);
        assert!(resp.reachable.is_empty());
        assert_eq!(resp.summary.reachable_count, 0);
        assert_eq!(resp.summary.farthest_ly, 0.0);
        assert!(resp.summary.by_sec_class.is_empty());
        // The source and hull are still echoed.
        assert_eq!(resp.source.system, "S");
        assert_eq!(resp.effective_ly, 0.5);
    }

    #[test]
    fn effective_range_geometry_matches_catalog_math() {
        // 1.5 LY base at JDC 5 (+20%/lvl) → 3.0 LY, covering Near and Mid.
        let gd = fan_graph();
        let eff = effective_ly(1.5, 0.20, 5);
        assert_eq!(eff, 3.0);
        let resp = compute_reachable(&gd, node(&gd, "S"), hull(1.5), 5, eff);
        assert_eq!(resp.summary.reachable_count, 2);
    }

    #[test]
    fn round2_rounds_to_two_decimals() {
        assert_eq!(round2(5.806705857679252), 5.81);
        assert_eq!(round2(2.0), 2.0);
    }
}
