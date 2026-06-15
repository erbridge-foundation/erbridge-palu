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
    /// Jump-capable hulls joined from `types.jsonl` + `typeDogma.jsonl`.
    pub hulls: RawHullCatalog,
}

/// One jump-capable hull: a published type carrying a `jumpDriveRange` (attr
/// 867). Produced by the `types`↔`typeDogma` join.
#[derive(Debug, Clone, PartialEq)]
pub struct RawHull {
    pub type_id: i64,
    pub name: String,
    pub group_id: i64,
    /// Base jump range in light-years (attribute 867).
    pub base_ly: f64,
}

/// Raw hull-catalog output from SDE parsing: the jump-capable hulls plus the
/// JDC per-level range bonus (attribute 870, as a fraction, e.g. 0.20).
/// `HullCatalog::from_raw` turns this into the indexed lookup structure.
#[derive(Debug, Default)]
pub struct RawHullCatalog {
    pub hulls: Vec<RawHull>,
    /// JDC `jumpDriveRangeBonus` as a per-level fraction (SDE value 20.0 → 0.20).
    /// `None` if the JDC skill row was absent from the build (treated as 0).
    pub jdc_bonus_per_level: Option<f64>,
}

/// An entry in the hull catalog: a jump-capable hull's identity, base range,
/// and group. Self-describing so a lookup by name or typeID can echo the
/// resolved hull back (canonical name + typeID), not just its range.
/// Mechanic-agnostic — no jump/bridge/conduit logic, just the SDE facts.
#[derive(Debug, Clone, PartialEq)]
pub struct HullEntry {
    /// Canonical hull name as it appears in the SDE `types` dataset.
    pub name: String,
    /// SDE typeID.
    pub type_id: i64,
    /// Base jump range in light-years (SDE attribute 867).
    pub base_ly: f64,
    pub group_id: i64,
}

/// In-memory catalog of jump-capable hulls, built from the SDE
/// (`types` + `typeDogma`). Keyed for lookup by ASCII-lowercased name and by
/// numeric typeID. Holds the JDC per-level range bonus (attribute 870) read
/// from the SDE rather than hardcoded.
///
/// Mechanic-agnostic: it stores ranges and groups only. Jump/bridge mechanics
/// live in consuming features (e.g. the follow-up blops staging endpoint).
#[derive(Debug, Default)]
pub struct HullCatalog {
    /// ASCII-lowercased hull name → entry.
    by_name: FxHashMap<String, HullEntry>,
    /// typeID → entry.
    by_type_id: FxHashMap<i64, HullEntry>,
    /// groupID → minimum base range over that group, computed at construction.
    min_base_ly_by_group: FxHashMap<i64, f64>,
    /// JDC `jumpDriveRangeBonus` as a per-level fraction (e.g. 0.20). `0.0` if
    /// the JDC skill row was absent from the build.
    bonus_per_level: f64,
}

impl HullCatalog {
    /// Build the indexed catalog from the raw join output. Computes the
    /// per-group minimum base range up front so callers can ask for a
    /// conservative default without enumerating hulls.
    pub fn from_raw(raw: RawHullCatalog) -> Self {
        let mut by_name = FxHashMap::with_capacity_and_hasher(raw.hulls.len(), Default::default());
        let mut by_type_id =
            FxHashMap::with_capacity_and_hasher(raw.hulls.len(), Default::default());
        let mut min_base_ly_by_group: FxHashMap<i64, f64> = FxHashMap::default();
        for hull in &raw.hulls {
            let entry = HullEntry {
                name: hull.name.clone(),
                type_id: hull.type_id,
                base_ly: hull.base_ly,
                group_id: hull.group_id,
            };
            by_name.insert(hull.name.to_ascii_lowercase(), entry.clone());
            by_type_id.insert(hull.type_id, entry);
            min_base_ly_by_group
                .entry(hull.group_id)
                .and_modify(|m| {
                    if hull.base_ly < *m {
                        *m = hull.base_ly;
                    }
                })
                .or_insert(hull.base_ly);
        }
        Self {
            by_name,
            by_type_id,
            min_base_ly_by_group,
            bonus_per_level: raw.jdc_bonus_per_level.unwrap_or(0.0),
        }
    }

    /// Number of catalogued hulls (surfaced in `/health`).
    pub fn len(&self) -> usize {
        self.by_type_id.len()
    }

    /// True if the catalog holds no hulls.
    pub fn is_empty(&self) -> bool {
        self.by_type_id.is_empty()
    }

    /// Look up a hull by case-insensitive name.
    pub fn by_name(&self, name: &str) -> Option<HullEntry> {
        self.by_name.get(&name.to_ascii_lowercase()).cloned()
    }

    /// Look up a hull by typeID.
    pub fn by_type_id(&self, type_id: i64) -> Option<HullEntry> {
        self.by_type_id.get(&type_id).cloned()
    }

    /// The minimum base range (light-years) over a `groupID`, computed at
    /// construction — the conservative default for "worst hull in the group".
    /// `None` if the group has no catalogued hulls.
    pub fn min_base_ly_for_group(&self, group_id: i64) -> Option<f64> {
        self.min_base_ly_by_group.get(&group_id).copied()
    }

    /// The JDC per-level range bonus (fraction), read from attribute 870.
    pub fn bonus_per_level(&self) -> f64 {
        self.bonus_per_level
    }
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
    /// Jump-capable hull catalog, built from the same SDE build as the map
    /// graph so a live snapshot never mixes systems and hulls across builds.
    /// Additive: it does not touch the node-index ordering invariants above.
    pub hulls: HullCatalog,
    pub build_number: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_catalog() -> RawHullCatalog {
        RawHullCatalog {
            hulls: vec![
                RawHull {
                    type_id: 22430,
                    name: "Sin".into(),
                    group_id: 898,
                    base_ly: 4.0,
                },
                RawHull {
                    type_id: 44996,
                    name: "Marshal".into(),
                    group_id: 898,
                    base_ly: 3.5,
                },
                RawHull {
                    type_id: 28850,
                    name: "Anshar".into(),
                    group_id: 902,
                    base_ly: 5.0,
                },
            ],
            jdc_bonus_per_level: Some(0.20),
        }
    }

    #[test]
    fn catalog_indexes_by_name_case_insensitively() {
        let cat = HullCatalog::from_raw(raw_catalog());
        let sin = cat.by_name("sIn").unwrap();
        assert_eq!(sin.base_ly, 4.0);
        assert_eq!(sin.group_id, 898);
    }

    #[test]
    fn catalog_indexes_by_type_id() {
        let cat = HullCatalog::from_raw(raw_catalog());
        assert_eq!(cat.by_type_id(28850).unwrap().base_ly, 5.0);
        assert!(cat.by_type_id(1).is_none());
    }

    #[test]
    fn catalog_reports_len_and_bonus() {
        let cat = HullCatalog::from_raw(raw_catalog());
        assert_eq!(cat.len(), 3);
        assert!(!cat.is_empty());
        assert_eq!(cat.bonus_per_level(), 0.20);
    }

    #[test]
    fn min_base_ly_for_group_is_the_smallest() {
        let cat = HullCatalog::from_raw(raw_catalog());
        // Black Ops group has Sin (4.0) and Marshal (3.5) → min 3.5.
        assert_eq!(cat.min_base_ly_for_group(898), Some(3.5));
        assert_eq!(cat.min_base_ly_for_group(902), Some(5.0));
        assert_eq!(cat.min_base_ly_for_group(12345), None);
    }

    #[test]
    fn empty_raw_yields_empty_catalog_with_zero_bonus() {
        let cat = HullCatalog::from_raw(RawHullCatalog::default());
        assert!(cat.is_empty());
        assert_eq!(cat.len(), 0);
        assert_eq!(cat.bonus_per_level(), 0.0);
        assert_eq!(cat.min_base_ly_for_group(898), None);
    }
}
