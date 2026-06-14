//! Graph construction: assemble `GraphData` (systems, undirected gate graph,
//! id/name lookups, kd-tree) from parsed `RawSdeData`.

use petgraph::graph::{NodeIndex, UnGraph};
use rustc_hash::{FxHashMap, FxHashSet};
use tracing::warn;

use crate::model::{GraphData, HullCatalog, RawSdeData};

/// Build the in-memory `GraphData`. Wormhole systems carry coordinates too,
/// but the kd-tree indexes only K-space systems (the reserved blops query
/// stages from K-space).
pub fn build_graph_data(raw: RawSdeData, build_number: u64) -> GraphData {
    let n = raw.systems.len();
    let mut gate_graph: UnGraph<(), ()> = UnGraph::with_capacity(n, raw.gate_pairs.len());
    let mut id_to_idx: FxHashMap<i64, NodeIndex> =
        FxHashMap::with_capacity_and_hasher(n, Default::default());
    let mut name_to_idx: FxHashMap<String, NodeIndex> =
        FxHashMap::with_capacity_and_hasher(n, Default::default());
    let mut spatial_index: kiddo::float::kdtree::KdTree<f64, usize, 3, 32, u32> =
        kiddo::float::kdtree::KdTree::new();

    // Single pass: add node, populate lookups and kd-tree. Invariant:
    // `idx.index() == position in raw.systems`, since we add nodes in order
    // and never remove (pinned by tests).
    //
    // Names are folded with `to_ascii_lowercase`: EVE system names are ASCII
    // (e.g. "Jita", "C-J6MT", "J100001"), so the ASCII path is faster and
    // avoids Unicode quirks like the Turkish dotted-I.
    for system in &raw.systems {
        let idx = gate_graph.add_node(());
        id_to_idx.insert(system.id, idx);
        let prev = name_to_idx.insert(system.name.to_ascii_lowercase(), idx);
        if prev.is_some() {
            warn!(name = %system.name, "duplicate system name in SDE; later entry wins");
        }
        // kd-tree holds only K-space systems (reserved for blops staging,
        // which never stages from J-space).
        if !system.is_wormhole() {
            spatial_index.add(&system.coords, idx.index());
        }
    }

    // Dedupe edges defensively even though parse_gate_pairs already does:
    // petgraph's UnGraph happily stores parallel edges and we don't want them.
    let mut edge_seen: FxHashSet<(usize, usize)> =
        FxHashSet::with_capacity_and_hasher(raw.gate_pairs.len(), Default::default());
    for (a_id, b_id) in &raw.gate_pairs {
        match (id_to_idx.get(a_id), id_to_idx.get(b_id)) {
            (Some(&a_idx), Some(&b_idx)) => {
                let key = if a_idx.index() <= b_idx.index() {
                    (a_idx.index(), b_idx.index())
                } else {
                    (b_idx.index(), a_idx.index())
                };
                if edge_seen.insert(key) {
                    gate_graph.add_edge(a_idx, b_idx, ());
                }
            }
            _ => warn!(
                a = a_id,
                b = b_id,
                "gate pair references unknown system; skipping"
            ),
        }
    }

    // Build the hull catalog from the same raw SDE build. Folding it into
    // GraphData means it rides the same ArcSwap as the map graph — a live
    // snapshot's systems and hulls always come from one build (no skew).
    let hulls = HullCatalog::from_raw(raw.hulls);

    GraphData {
        systems: raw.systems,
        id_to_idx,
        name_to_idx,
        gate_graph,
        spatial_index,
        hulls,
        build_number,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RawSdeData, SecClass, System};
    use petgraph::visit::EdgeRef;

    fn make_system(id: i64, name: &str, security: f32, coords: [f64; 3], region_id: i64) -> System {
        let is_wormhole = region_id >= 11_000_000;
        let sec_class = if is_wormhole {
            SecClass::Wormhole
        } else if security >= 0.45 {
            SecClass::Highsec
        } else if security > 0.0 {
            SecClass::Lowsec
        } else {
            SecClass::Nullsec
        };
        System {
            id,
            name: name.to_string(),
            security,
            sec_class,
            coords,
            region_id,
            constellation_id: 20000000,
        }
    }

    fn fixture() -> RawSdeData {
        RawSdeData {
            systems: vec![
                make_system(
                    30000142,
                    "Jita",
                    0.945,
                    [-1.29e17, 6.07e16, 1.17e17],
                    10000002,
                ),
                make_system(
                    30000144,
                    "Perimeter",
                    0.953,
                    [-1.43e17, 6.49e16, 1.04e17],
                    10000002,
                ),
                make_system(
                    30000139,
                    "Urlen",
                    0.959,
                    [-1.39e17, 7.14e16, 9.95e16],
                    10000002,
                ),
            ],
            gate_pairs: vec![(30000142, 30000144), (30000139, 30000144)],
            hulls: Default::default(),
        }
    }

    #[test]
    fn node_count_matches_systems() {
        let gd = build_graph_data(fixture(), 0);
        assert_eq!(gd.gate_graph.node_count(), 3);
    }

    #[test]
    fn edge_count_matches_pairs() {
        let gd = build_graph_data(fixture(), 0);
        assert_eq!(gd.gate_graph.edge_count(), 2);
    }

    #[test]
    fn id_lookup_works() {
        let gd = build_graph_data(fixture(), 0);
        let idx = gd.id_to_idx[&30000142];
        assert_eq!(gd.systems[idx.index()].name, "Jita");
    }

    #[test]
    fn node_index_matches_systems_position() {
        // Load-bearing invariant: routing indexes systems[idx.index()].
        let gd = build_graph_data(fixture(), 0);
        for (i, system) in gd.systems.iter().enumerate() {
            let idx = gd.id_to_idx[&system.id];
            assert_eq!(idx.index(), i);
        }
    }

    #[test]
    fn name_lookup_case_insensitive() {
        let gd = build_graph_data(fixture(), 0);
        assert!(gd.name_to_idx.contains_key("jita"));
        assert!(gd.name_to_idx.contains_key("perimeter"));
        // Lookup folds the same regardless of caller case.
        assert_eq!(gd.name_to_idx.get("jita"), gd.id_to_idx.get(&30000142));
    }

    #[test]
    fn directed_pairs_become_one_undirected_edge() {
        let raw = RawSdeData {
            systems: vec![
                make_system(30000142, "Jita", 0.945, [0.0, 0.0, 0.0], 10000002),
                make_system(30000144, "Perimeter", 0.953, [1.0, 0.0, 0.0], 10000002),
            ],
            // Same pair twice plus reversed — all collapse to one edge.
            gate_pairs: vec![
                (30000142, 30000144),
                (30000142, 30000144),
                (30000144, 30000142),
            ],
            hulls: Default::default(),
        };
        let gd = build_graph_data(raw, 0);
        assert_eq!(gd.gate_graph.edge_count(), 1);
    }

    #[test]
    fn adjacency_is_correct() {
        let gd = build_graph_data(fixture(), 0);
        let jita = gd.id_to_idx[&30000142];
        let perimeter = gd.id_to_idx[&30000144];
        let urlen = gd.id_to_idx[&30000139];
        let neighbors: Vec<_> = gd
            .gate_graph
            .edges(jita)
            .map(|e| {
                if e.target() == jita {
                    e.source()
                } else {
                    e.target()
                }
            })
            .collect();
        assert!(neighbors.contains(&perimeter));
        assert!(!neighbors.contains(&urlen));
    }

    #[test]
    fn unknown_system_in_gate_pair_is_skipped() {
        let raw = RawSdeData {
            systems: vec![make_system(
                30000142,
                "Jita",
                0.945,
                [0.0, 0.0, 0.0],
                10000002,
            )],
            gate_pairs: vec![(30000142, 99999999)],
            hulls: Default::default(),
        };
        let gd = build_graph_data(raw, 0);
        assert_eq!(gd.gate_graph.node_count(), 1);
        assert_eq!(gd.gate_graph.edge_count(), 0);
    }

    #[test]
    fn kdtree_indexes_kspace_only() {
        let raw = RawSdeData {
            systems: vec![
                make_system(30000142, "Jita", 0.945, [0.0, 0.0, 0.0], 10000002),
                make_system(31000001, "J100001", -0.99, [1.0, 0.0, 0.0], 11000001),
            ],
            gate_pairs: vec![],
            hulls: Default::default(),
        };
        let gd = build_graph_data(raw, 0);
        let jita = gd.id_to_idx[&30000142];
        // Nearest to the wormhole's coords must still be Jita (the only
        // K-space point in the tree); the WH system was not indexed.
        let wh_coords = gd.systems[gd.id_to_idx[&31000001].index()].coords;
        let nearest = gd
            .spatial_index
            .nearest_one::<kiddo::SquaredEuclidean>(&wh_coords);
        assert_eq!(NodeIndex::new(nearest.item), jita);
    }

    #[test]
    fn build_number_preserved() {
        let gd = build_graph_data(fixture(), 42);
        assert_eq!(gd.build_number, 42);
    }

    #[test]
    fn hull_catalog_rides_into_graph_data() {
        use crate::model::{RawHull, RawHullCatalog};
        let raw = RawSdeData {
            systems: vec![make_system(
                30000142,
                "Jita",
                0.945,
                [0.0, 0.0, 0.0],
                10000002,
            )],
            gate_pairs: vec![],
            hulls: RawHullCatalog {
                hulls: vec![RawHull {
                    type_id: 22430,
                    name: "Sin".into(),
                    group_id: 898,
                    base_ly: 4.0,
                }],
                jdc_bonus_per_level: Some(0.20),
            },
        };
        let gd = build_graph_data(raw, 0);
        // The catalog is built from the same raw build and carried on GraphData.
        assert_eq!(gd.hulls.len(), 1);
        assert_eq!(gd.hulls.by_name("sin").unwrap().base_ly, 4.0);
        assert_eq!(gd.hulls.bonus_per_level(), 0.20);
    }
}
