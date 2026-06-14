//! Routing core: a per-request `RouteContext` overlay over the base graph and
//! a Dijkstra with reusable scratch buffers.

use std::cell::RefCell;
use std::collections::BinaryHeap;

use petgraph::graph::NodeIndex;
use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

use crate::model::{GraphData, SecClass};

thread_local! {
    /// Reusable Dijkstra buffers, one per worker thread. Allocating fresh
    /// `Vec`s per request shows up at p99, so handlers borrow this instead.
    static DIJKSTRA_SCRATCH: RefCell<DijkstraScratch> = RefCell::new(DijkstraScratch::new());
}

/// Run `f` with the thread-local `DijkstraScratch`. The reset happens inside
/// `shortest_path`, so callers get a correctly-sized buffer.
pub fn with_scratch<R>(f: impl FnOnce(&mut DijkstraScratch) -> R) -> R {
    DIJKSTRA_SCRATCH.with(|cell| f(&mut cell.borrow_mut()))
}

/// Security penalty applied to entering a non-preferred system under `safest`.
/// Large but finite so a route still resolves when no highsec path exists —
/// never `INFINITY`, which would make unreachable-vs-expensive indistinguishable.
const SECURITY_PENALTY: f32 = 10_000.0;

/// Additive penalty per wormhole hop under `prefer_gates`. A wormhole is taken
/// only when it saves more than this many jumps. `2.0` means "a WH must shorten
/// the route by more than 2 jumps to be worth it."
const WORMHOLE_PENALTY: f32 = 2.0;

/// Routing preference controlling composable edge weights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preference {
    /// Minimise hop count: every edge costs 1.
    Shortest,
    /// Prefer highsec: a large penalty for entering low/null/wormhole.
    Safest,
    /// Prefer gates: a small additive penalty per wormhole hop.
    PreferGates,
}

/// Per-request routing context wrapping a reference to the base graph.
///
/// `avoid` holds nodes that must never appear in a path (including source or
/// destination — those simply yield no route). `connections` is a sparse,
/// mirrored adjacency overlay for per-request wormhole edges; the same
/// mechanism serves user `connections[]` and EVE-Scout signatures.
pub struct RouteContext<'a> {
    pub graph: &'a GraphData,
    avoid: FxHashSet<NodeIndex>,
    connections: FxHashMap<NodeIndex, SmallVec<[NodeIndex; 2]>>,
    pub preference: Preference,
}

impl<'a> RouteContext<'a> {
    pub fn new(graph: &'a GraphData, avoid: FxHashSet<NodeIndex>, preference: Preference) -> Self {
        Self {
            graph,
            avoid,
            connections: FxHashMap::default(),
            preference,
        }
    }

    /// Add an undirected per-request wormhole connection. Both directions are
    /// inserted; self-loops and duplicates are silently ignored.
    pub fn add_connection(&mut self, a: NodeIndex, b: NodeIndex) {
        if a == b {
            return;
        }
        let entry = self.connections.entry(a).or_default();
        if !entry.contains(&b) {
            entry.push(b);
        }
        let entry = self.connections.entry(b).or_default();
        if !entry.contains(&a) {
            entry.push(a);
        }
    }

    /// True iff `(a, b)` was added via `add_connection` (either direction).
    /// Used to label path steps `wormhole` vs `stargate`.
    pub fn is_wh_edge(&self, a: NodeIndex, b: NodeIndex) -> bool {
        self.connections
            .get(&a)
            .is_some_and(|nbrs| nbrs.contains(&b))
    }

    /// True iff a base-graph gate connects `a` and `b`. Used to label a step
    /// `stargate` even when a wormhole overlay edge also links the same pair —
    /// a real gate always takes labelling priority.
    pub fn has_gate_edge(&self, a: NodeIndex, b: NodeIndex) -> bool {
        self.graph.gate_graph.contains_edge(a, b)
    }

    /// How a step from `a` to `b` was reached: a real gate wins over an
    /// overlay wormhole when both connect the pair.
    pub fn step_via(&self, a: NodeIndex, b: NodeIndex) -> &'static str {
        if self.has_gate_edge(a, b) {
            "stargate"
        } else if self.is_wh_edge(a, b) {
            "wormhole"
        } else {
            // Shouldn't happen for adjacent path nodes, but stay honest.
            "stargate"
        }
    }

    /// Neighbours of `node` under this context: base-graph gate neighbours
    /// followed by overlay wormhole neighbours, with avoided nodes filtered so
    /// they are never enqueued.
    fn neighbors(&self, node: NodeIndex) -> impl Iterator<Item = NodeIndex> + '_ {
        let base = self.graph.gate_graph.neighbors(node);
        let overlay = self
            .connections
            .get(&node)
            .into_iter()
            .flat_map(|v| v.iter().copied());
        base.chain(overlay).filter(|nb| !self.avoid.contains(nb))
    }

    /// Composable edge weight to traverse from `src` into `dest`:
    /// `1 + security_penalty(pref, dest) + wh_penalty(pref, edge)`.
    fn edge_weight(&self, src: NodeIndex, dest: NodeIndex) -> f32 {
        let sec_class = self.graph.systems[dest.index()].sec_class;
        1.0 + self.security_penalty(sec_class) + self.wormhole_penalty(src, dest)
    }

    fn security_penalty(&self, dest: SecClass) -> f32 {
        match self.preference {
            Preference::Safest if dest != SecClass::Highsec => SECURITY_PENALTY,
            _ => 0.0,
        }
    }

    fn wormhole_penalty(&self, src: NodeIndex, dest: NodeIndex) -> f32 {
        // Penalise a hop only when it is genuinely a wormhole — i.e. an overlay
        // edge with no parallel gate. When a real gate also connects the pair,
        // that gate is the cheaper way across and takes priority (matching
        // `step_via`'s labelling), so no penalty applies.
        match self.preference {
            Preference::PreferGates
                if self.is_wh_edge(src, dest) && !self.has_gate_edge(src, dest) =>
            {
                WORMHOLE_PENALTY
            }
            _ => 0.0,
        }
    }
}

/// Returned path. Most EVE routes are < 32 jumps, so the inline buffer covers
/// the common case with no allocation.
pub type Path = SmallVec<[NodeIndex; 32]>;

/// Reusable scratch for Dijkstra. Allocating per request shows up at p99.
pub struct DijkstraScratch {
    dist: Vec<f32>,
    prev: Vec<u32>,
    visited: Vec<bool>,
}

const NO_PREV: u32 = u32::MAX;

impl DijkstraScratch {
    pub fn new() -> Self {
        Self {
            dist: Vec::new(),
            prev: Vec::new(),
            visited: Vec::new(),
        }
    }

    fn reset(&mut self, n: usize) {
        self.dist.clear();
        self.dist.resize(n, f32::INFINITY);
        self.prev.clear();
        self.prev.resize(n, NO_PREV);
        self.visited.clear();
        self.visited.resize(n, false);
    }
}

impl Default for DijkstraScratch {
    fn default() -> Self {
        Self::new()
    }
}

/// Min-heap entry. `f32` isn't `Ord`, so we wrap it and reverse the comparison
/// to turn `BinaryHeap` (a max-heap) into a min-heap.
#[derive(Copy, Clone)]
struct HeapItem {
    dist: f32,
    node: u32,
}

impl PartialEq for HeapItem {
    fn eq(&self, other: &Self) -> bool {
        self.dist == other.dist && self.node == other.node
    }
}
impl Eq for HeapItem {}
impl PartialOrd for HeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for HeapItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse: smaller distance is "greater" so BinaryHeap is a min-heap.
        // Distances are sums of finite non-negative weights, so never NaN;
        // total_cmp keeps clippy happy and is a safe fallback regardless.
        other
            .dist
            .total_cmp(&self.dist)
            .then_with(|| other.node.cmp(&self.node))
    }
}

/// Dijkstra from `from` to `to` over `ctx`. Returns the path (inclusive of
/// both endpoints) or `None` if `to` is unreachable. `from == to` returns a
/// one-element path.
pub fn shortest_path(
    ctx: &RouteContext<'_>,
    from: NodeIndex,
    to: NodeIndex,
    scratch: &mut DijkstraScratch,
) -> Option<Path> {
    let n = ctx.graph.systems.len();
    debug_assert!(from.index() < n && to.index() < n);
    scratch.reset(n);

    let from_u = from.index() as u32;
    let to_u = to.index() as u32;

    scratch.dist[from.index()] = 0.0;

    let mut heap: BinaryHeap<HeapItem> = BinaryHeap::new();
    heap.push(HeapItem {
        dist: 0.0,
        node: from_u,
    });

    while let Some(HeapItem { dist, node }) = heap.pop() {
        let idx = node as usize;
        if scratch.visited[idx] {
            continue;
        }
        scratch.visited[idx] = true;

        if node == to_u {
            return Some(reconstruct_path(&scratch.prev, from_u, to_u));
        }

        // Stale heap entry (a better one was pushed later); skip.
        if dist > scratch.dist[idx] {
            continue;
        }

        let cur = NodeIndex::new(idx);
        for nb in ctx.neighbors(cur) {
            let nb_idx = nb.index();
            if scratch.visited[nb_idx] {
                continue;
            }
            let new_dist = dist + ctx.edge_weight(cur, nb);
            if new_dist < scratch.dist[nb_idx] {
                scratch.dist[nb_idx] = new_dist;
                scratch.prev[nb_idx] = node;
                heap.push(HeapItem {
                    dist: new_dist,
                    node: nb_idx as u32,
                });
            }
        }
    }

    None
}

fn reconstruct_path(prev: &[u32], from: u32, to: u32) -> Path {
    let mut rev: Path = SmallVec::new();
    let mut cur = to;
    rev.push(NodeIndex::new(cur as usize));
    while cur != from {
        let p = prev[cur as usize];
        debug_assert!(p != NO_PREV, "broken predecessor chain in Dijkstra");
        rev.push(NodeIndex::new(p as usize));
        cur = p;
    }
    rev.reverse();
    rev
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::build_graph_data;
    use crate::model::{RawSdeData, SecClass, System};

    fn sys(id: i64, name: &str, sec_class: SecClass) -> System {
        let (security, region_id) = match sec_class {
            SecClass::Highsec => (0.9, 10000002),
            SecClass::Lowsec => (0.3, 10000002),
            SecClass::Nullsec => (-0.5, 10000002),
            SecClass::Wormhole => (-0.99, 11000001),
        };
        System {
            id,
            name: name.into(),
            security,
            sec_class,
            coords: [id as f64, 0.0, 0.0],
            region_id,
            constellation_id: 20000020,
        }
    }

    /// A — B — C, all highsec.
    fn linear_three() -> GraphData {
        let raw = RawSdeData {
            systems: vec![
                sys(1, "A", SecClass::Highsec),
                sys(2, "B", SecClass::Highsec),
                sys(3, "C", SecClass::Highsec),
            ],
            gate_pairs: vec![(1, 2), (2, 3)],
        };
        build_graph_data(raw, 0)
    }

    #[test]
    fn same_endpoint_returns_single_node_path() {
        let gd = linear_three();
        let ctx = RouteContext::new(&gd, FxHashSet::default(), Preference::Shortest);
        let mut s = DijkstraScratch::new();
        let a = gd.id_to_idx[&1];
        let path = shortest_path(&ctx, a, a, &mut s).unwrap();
        assert_eq!(path.len(), 1);
        assert_eq!(path[0], a);
    }

    #[test]
    fn finds_two_hop_path() {
        let gd = linear_three();
        let ctx = RouteContext::new(&gd, FxHashSet::default(), Preference::Shortest);
        let mut s = DijkstraScratch::new();
        let a = gd.id_to_idx[&1];
        let c = gd.id_to_idx[&3];
        let path = shortest_path(&ctx, a, c, &mut s).unwrap();
        assert_eq!(path.len(), 3);
        assert_eq!(path.first().copied(), Some(a));
        assert_eq!(path.last().copied(), Some(c));
    }

    #[test]
    fn unreachable_returns_none() {
        let raw = RawSdeData {
            systems: vec![
                sys(1, "A", SecClass::Highsec),
                sys(2, "B", SecClass::Highsec),
                sys(3, "C", SecClass::Highsec),
            ],
            gate_pairs: vec![(1, 2)],
        };
        let gd = build_graph_data(raw, 0);
        let ctx = RouteContext::new(&gd, FxHashSet::default(), Preference::Shortest);
        let mut s = DijkstraScratch::new();
        let a = gd.id_to_idx[&1];
        let c = gd.id_to_idx[&3];
        assert!(shortest_path(&ctx, a, c, &mut s).is_none());
    }

    #[test]
    fn picks_shorter_of_two_paths() {
        // A—B—C—D plus direct A—D: shortest A→D is 1 hop.
        let raw = RawSdeData {
            systems: vec![
                sys(1, "A", SecClass::Highsec),
                sys(2, "B", SecClass::Highsec),
                sys(3, "C", SecClass::Highsec),
                sys(4, "D", SecClass::Highsec),
            ],
            gate_pairs: vec![(1, 2), (2, 3), (3, 4), (1, 4)],
        };
        let gd = build_graph_data(raw, 0);
        let ctx = RouteContext::new(&gd, FxHashSet::default(), Preference::Shortest);
        let mut s = DijkstraScratch::new();
        let path = shortest_path(&ctx, gd.id_to_idx[&1], gd.id_to_idx[&4], &mut s).unwrap();
        assert_eq!(path.len(), 2);
    }

    #[test]
    fn scratch_is_reusable_across_runs() {
        let gd = linear_three();
        let ctx = RouteContext::new(&gd, FxHashSet::default(), Preference::Shortest);
        let mut s = DijkstraScratch::new();
        let (a, b, c) = (gd.id_to_idx[&1], gd.id_to_idx[&2], gd.id_to_idx[&3]);
        assert_eq!(shortest_path(&ctx, a, c, &mut s).unwrap().len(), 3);
        assert_eq!(shortest_path(&ctx, c, a, &mut s).unwrap().len(), 3);
        assert_eq!(shortest_path(&ctx, b, b, &mut s).unwrap().len(), 1);
    }

    // ── avoid ────────────────────────────────────────────────────────────────

    #[test]
    fn avoid_forces_detour() {
        // Diamond A→B→D and A→C→D; avoid B → route via C.
        let raw = RawSdeData {
            systems: vec![
                sys(1, "A", SecClass::Highsec),
                sys(2, "B", SecClass::Highsec),
                sys(3, "C", SecClass::Highsec),
                sys(4, "D", SecClass::Highsec),
            ],
            gate_pairs: vec![(1, 2), (2, 4), (1, 3), (3, 4)],
        };
        let gd = build_graph_data(raw, 0);
        let b = gd.id_to_idx[&2];
        let mut avoid = FxHashSet::default();
        avoid.insert(b);
        let ctx = RouteContext::new(&gd, avoid, Preference::Shortest);
        let mut s = DijkstraScratch::new();
        let path = shortest_path(&ctx, gd.id_to_idx[&1], gd.id_to_idx[&4], &mut s).unwrap();
        assert!(!path.contains(&b));
        assert_eq!(path.len(), 3);
    }

    #[test]
    fn avoid_returns_none_when_only_path_blocked() {
        let gd = linear_three();
        let b = gd.id_to_idx[&2];
        let mut avoid = FxHashSet::default();
        avoid.insert(b);
        let ctx = RouteContext::new(&gd, avoid, Preference::Shortest);
        let mut s = DijkstraScratch::new();
        assert!(shortest_path(&ctx, gd.id_to_idx[&1], gd.id_to_idx[&3], &mut s).is_none());
    }

    // ── safest ─────────────────────────────────────────────────────────────────

    /// Diamond: A(high)→B(null)→D(high) and A(high)→C(high)→D(high). Under
    /// `safest`, B costs the penalty so A→C→D wins.
    fn diamond_with_null() -> GraphData {
        let raw = RawSdeData {
            systems: vec![
                sys(1, "A", SecClass::Highsec),
                sys(2, "B", SecClass::Nullsec),
                sys(3, "C", SecClass::Highsec),
                sys(4, "D", SecClass::Highsec),
            ],
            gate_pairs: vec![(1, 2), (2, 4), (1, 3), (3, 4)],
        };
        build_graph_data(raw, 0)
    }

    #[test]
    fn safest_avoids_nullsec_when_highsec_path_exists() {
        let gd = diamond_with_null();
        let (b, c) = (gd.id_to_idx[&2], gd.id_to_idx[&3]);
        let ctx = RouteContext::new(&gd, FxHashSet::default(), Preference::Safest);
        let mut s = DijkstraScratch::new();
        let path = shortest_path(&ctx, gd.id_to_idx[&1], gd.id_to_idx[&4], &mut s).unwrap();
        assert!(path.contains(&c), "safest must use highsec C");
        assert!(!path.contains(&b), "safest must avoid nullsec B");
    }

    #[test]
    fn shortest_treats_both_diamond_paths_equally() {
        // Sanity: under shortest, both 2-hop paths are valid (no sec penalty).
        let gd = diamond_with_null();
        let ctx = RouteContext::new(&gd, FxHashSet::default(), Preference::Shortest);
        let mut s = DijkstraScratch::new();
        let path = shortest_path(&ctx, gd.id_to_idx[&1], gd.id_to_idx[&4], &mut s).unwrap();
        assert_eq!(path.len(), 3);
    }

    #[test]
    fn safest_still_resolves_when_no_highsec_path_exists() {
        // A(high)—B(null)—C(high), no bypass: safest must still route via B.
        let raw = RawSdeData {
            systems: vec![
                sys(1, "A", SecClass::Highsec),
                sys(2, "B", SecClass::Nullsec),
                sys(3, "C", SecClass::Highsec),
            ],
            gate_pairs: vec![(1, 2), (2, 3)],
        };
        let gd = build_graph_data(raw, 0);
        let ctx = RouteContext::new(&gd, FxHashSet::default(), Preference::Safest);
        let mut s = DijkstraScratch::new();
        let path = shortest_path(&ctx, gd.id_to_idx[&1], gd.id_to_idx[&3], &mut s).unwrap();
        assert_eq!(path.len(), 3, "must still find the only path");
    }

    #[test]
    fn safest_treats_wormhole_like_nullsec() {
        // A→B(wormhole)→D vs A→C(high)→D: safest prefers C.
        let raw = RawSdeData {
            systems: vec![
                sys(1, "A", SecClass::Highsec),
                sys(2, "B", SecClass::Wormhole),
                sys(3, "C", SecClass::Highsec),
                sys(4, "D", SecClass::Highsec),
            ],
            gate_pairs: vec![(1, 2), (2, 4), (1, 3), (3, 4)],
        };
        let gd = build_graph_data(raw, 0);
        let (wh, c) = (gd.id_to_idx[&2], gd.id_to_idx[&3]);
        let ctx = RouteContext::new(&gd, FxHashSet::default(), Preference::Safest);
        let mut s = DijkstraScratch::new();
        let path = shortest_path(&ctx, gd.id_to_idx[&1], gd.id_to_idx[&4], &mut s).unwrap();
        assert!(path.contains(&c));
        assert!(!path.contains(&wh));
    }

    // ── wormhole connections ────────────────────────────────────────────────────

    #[test]
    fn wh_connection_bridges_disconnected_components() {
        // A—B and C—D; add WH B↔C → A→D = 3 jumps.
        let raw = RawSdeData {
            systems: vec![
                sys(1, "A", SecClass::Highsec),
                sys(2, "B", SecClass::Highsec),
                sys(3, "C", SecClass::Highsec),
                sys(4, "D", SecClass::Highsec),
            ],
            gate_pairs: vec![(1, 2), (3, 4)],
        };
        let gd = build_graph_data(raw, 0);
        let mut ctx = RouteContext::new(&gd, FxHashSet::default(), Preference::Shortest);
        let (b, c) = (gd.id_to_idx[&2], gd.id_to_idx[&3]);
        ctx.add_connection(b, c);
        let mut s = DijkstraScratch::new();
        let path = shortest_path(&ctx, gd.id_to_idx[&1], gd.id_to_idx[&4], &mut s).unwrap();
        assert_eq!(path.len(), 4);
        assert!(ctx.is_wh_edge(b, c) && ctx.is_wh_edge(c, b));
        assert!(!ctx.is_wh_edge(gd.id_to_idx[&1], b));
    }

    #[test]
    fn wh_connection_respects_avoid() {
        let raw = RawSdeData {
            systems: vec![
                sys(1, "A", SecClass::Highsec),
                sys(2, "B", SecClass::Highsec),
                sys(3, "C", SecClass::Highsec),
                sys(4, "D", SecClass::Highsec),
            ],
            gate_pairs: vec![(1, 2), (3, 4)],
        };
        let gd = build_graph_data(raw, 0);
        let c = gd.id_to_idx[&3];
        let mut avoid = FxHashSet::default();
        avoid.insert(c);
        let mut ctx = RouteContext::new(&gd, avoid, Preference::Shortest);
        ctx.add_connection(gd.id_to_idx[&2], c);
        let mut s = DijkstraScratch::new();
        assert!(shortest_path(&ctx, gd.id_to_idx[&1], gd.id_to_idx[&4], &mut s).is_none());
    }

    #[test]
    fn step_via_prefers_gate_over_parallel_wormhole() {
        // A WH duplicating an existing gate edge must still label `stargate`.
        let gd = linear_three();
        let (a, b) = (gd.id_to_idx[&1], gd.id_to_idx[&2]);
        let mut ctx = RouteContext::new(&gd, FxHashSet::default(), Preference::Shortest);
        ctx.add_connection(a, b);
        assert!(ctx.is_wh_edge(a, b));
        assert!(ctx.has_gate_edge(a, b));
        assert_eq!(ctx.step_via(a, b), "stargate");
    }

    #[test]
    fn step_via_labels_pure_wormhole() {
        // No gate between A and C → a WH there labels `wormhole`.
        let gd = linear_three();
        let (a, c) = (gd.id_to_idx[&1], gd.id_to_idx[&3]);
        let mut ctx = RouteContext::new(&gd, FxHashSet::default(), Preference::Shortest);
        ctx.add_connection(a, c);
        assert!(!ctx.has_gate_edge(a, c));
        assert_eq!(ctx.step_via(a, c), "wormhole");
    }

    #[test]
    fn wh_self_loop_is_ignored() {
        let gd = linear_three();
        let mut ctx = RouteContext::new(&gd, FxHashSet::default(), Preference::Shortest);
        let a = gd.id_to_idx[&1];
        ctx.add_connection(a, a);
        assert!(!ctx.is_wh_edge(a, a));
    }

    // ── prefer_gates thresholds ─────────────────────────────────────────────────

    /// Linear gate chain A—B—C—D—E (4 gate jumps end to end) plus an optional
    /// WH shortcut from A to a chosen endpoint. Returns the graph.
    fn gate_chain(len: usize) -> GraphData {
        let systems: Vec<System> = (0..len)
            .map(|i| sys(i as i64 + 1, &format!("S{i}"), SecClass::Highsec))
            .collect();
        let gate_pairs: Vec<(i64, i64)> =
            (0..len - 1).map(|i| (i as i64 + 1, i as i64 + 2)).collect();
        build_graph_data(
            RawSdeData {
                systems,
                gate_pairs,
            },
            0,
        )
    }

    #[test]
    fn prefer_gates_skips_wormhole_that_saves_too_little() {
        // Chain A..C: A→C is 2 gate jumps. A WH A↔C saves 1 jump (2→1), which
        // is not more than the penalty (2), so prefer_gates keeps the gates.
        let gd = gate_chain(3);
        let (a, c) = (gd.id_to_idx[&1], gd.id_to_idx[&3]);
        let mut ctx = RouteContext::new(&gd, FxHashSet::default(), Preference::PreferGates);
        ctx.add_connection(a, c);
        let mut s = DijkstraScratch::new();
        let path = shortest_path(&ctx, a, c, &mut s).unwrap();
        assert_eq!(path.len(), 3, "should take the 2-gate path, not the WH");
        assert_eq!(path[1], gd.id_to_idx[&2], "middle hop is the gate system");
        assert!(!ctx.is_wh_edge(path[0], path[1]));
    }

    #[test]
    fn prefer_gates_does_not_penalise_a_gate_with_a_parallel_wormhole() {
        // Regression: a WH duplicating an existing 1-gate hop (A↔B) must not
        // make that hop cost 1 + penalty. The real gate wins, so A→B stays a
        // single stargate jump rather than detouring around.
        let gd = gate_chain(3); // A—B—C
        let (a, b) = (gd.id_to_idx[&1], gd.id_to_idx[&2]);
        let mut ctx = RouteContext::new(&gd, FxHashSet::default(), Preference::PreferGates);
        ctx.add_connection(a, b); // WH parallel to the A—B gate
        let mut s = DijkstraScratch::new();
        let path = shortest_path(&ctx, a, b, &mut s).unwrap();
        assert_eq!(path.len(), 2, "direct gate hop, no detour");
        assert_eq!(ctx.step_via(path[0], path[1]), "stargate");
    }

    #[test]
    fn prefer_gates_takes_wormhole_that_saves_enough() {
        // Chain A..E: A→E is 4 gate jumps. WH A↔E saves 3 jumps (4→1), which
        // exceeds the penalty (2), so prefer_gates takes the wormhole.
        let gd = gate_chain(5);
        let (a, e) = (gd.id_to_idx[&1], gd.id_to_idx[&5]);
        let mut ctx = RouteContext::new(&gd, FxHashSet::default(), Preference::PreferGates);
        ctx.add_connection(a, e);
        let mut s = DijkstraScratch::new();
        let path = shortest_path(&ctx, a, e, &mut s).unwrap();
        assert_eq!(path.len(), 2, "should take the WH shortcut");
        assert!(ctx.is_wh_edge(path[0], path[1]));
    }

    #[test]
    fn shortest_always_takes_wormhole_shortcut() {
        // Same 2-gate scenario, but shortest ignores the WH penalty and takes
        // the 1-jump shortcut.
        let gd = gate_chain(3);
        let (a, c) = (gd.id_to_idx[&1], gd.id_to_idx[&3]);
        let mut ctx = RouteContext::new(&gd, FxHashSet::default(), Preference::Shortest);
        ctx.add_connection(a, c);
        let mut s = DijkstraScratch::new();
        let path = shortest_path(&ctx, a, c, &mut s).unwrap();
        assert_eq!(path.len(), 2);
        assert!(ctx.is_wh_edge(path[0], path[1]));
    }
}
