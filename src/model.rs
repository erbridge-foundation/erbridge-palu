//! Core domain types shared across SDE parsing, graph construction, and
//! routing. These are internal model types — DTOs in `dto.rs` own the wire
//! format.

use petgraph::graph::{NodeIndex, UnGraph};
use rustc_hash::FxHashMap;

/// A solar system. `sec_class` and the wormhole flag are derived during SDE
/// parsing from `securityStatus` and `region_id`.
#[derive(Debug, Clone)]
pub struct System {
    pub id: i64,
    pub name: String,
    pub security: f32,
    pub sec_class: SecClass,
    pub coords: [f64; 3],
    pub region_id: i64,
    pub constellation_id: i64,
}

impl System {
    /// True iff this is a wormhole (J-space) system. Authoritative source is
    /// `region_id >= 11_000_000`, which the parser folds into `sec_class`.
    pub fn is_wormhole(&self) -> bool {
        matches!(self.sec_class, SecClass::Wormhole)
    }
}

/// Security classification used for routing weights and response labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecClass {
    Highsec,
    Lowsec,
    Nullsec,
    Wormhole,
}

impl SecClass {
    /// Static label used in JSON responses.
    pub fn label(self) -> &'static str {
        match self {
            SecClass::Highsec => "Highsec",
            SecClass::Lowsec => "Lowsec",
            SecClass::Nullsec => "Nullsec",
            SecClass::Wormhole => "Wormhole",
        }
    }
}

/// Raw output from SDE parsing; graph construction consumes this.
#[derive(Debug)]
pub struct RawSdeData {
    pub systems: Vec<System>,
    /// Deduplicated `(system_a, system_b)` pairs with `a <= b`.
    pub gate_pairs: Vec<(i64, i64)>,
}

/// Fully-built graph data, shared behind an `ArcSwap`.
///
/// Load-bearing invariants the rest of the codebase relies on:
/// 1. `node_idx.index() == position in systems` — nodes are added in order
///    and never removed, so the routing layer indexes `systems[idx.index()]`
///    directly. Tests in `graph::tests` pin this.
/// 2. `name_to_idx` keys are ASCII-lowercased system names.
/// 3. `gate_graph` is undirected with at most one edge per system pair.
pub struct GraphData {
    /// All systems, indexed by `NodeIndex` (same indices as `gate_graph`).
    pub systems: Vec<System>,
    pub id_to_idx: FxHashMap<i64, NodeIndex>,
    /// ASCII-lowercased name → `NodeIndex`.
    pub name_to_idx: FxHashMap<String, NodeIndex>,
    /// Undirected adjacency graph. Node/edge weights are unit; a `NodeIndex`'s
    /// position already identifies its system via invariant 1.
    pub gate_graph: UnGraph<(), ()>,
    /// kd-tree over K-space system coordinates (meters). Items are
    /// `NodeIndex::index()` values. Type params `<f64, usize, 3, 32, u32>`:
    /// coord type, item type, 3 dimensions, 32-item buckets, u32 indices.
    /// Built now but unqueried in this foundation (reserved for blops LY
    /// distance queries).
    pub spatial_index: kiddo::float::kdtree::KdTree<f64, usize, 3, 32, u32>,
    pub build_number: u64,
}
