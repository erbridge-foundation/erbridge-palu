//! Raw deserialization rows for the two SDE JSONL files we consume. Fields we
//! don't route on (planets, localised names beyond `en`, position2D, radius)
//! are dropped by serde so only ~2 MB stays live.

use serde::Deserialize;

/// Raw row from `mapSolarSystems.jsonl`.
#[derive(Debug, Deserialize)]
pub struct RawSystem {
    #[serde(rename = "_key")]
    pub id: i64,
    pub name: LocalisedName,
    #[serde(rename = "securityStatus")]
    pub security_status: f32,
    #[serde(rename = "regionID")]
    pub region_id: i64,
    #[serde(rename = "constellationID")]
    pub constellation_id: i64,
    pub position: Position,
}

/// Raw row from `mapStargates.jsonl`. We only need the `(system_a, system_b)`
/// pair to build undirected gate edges.
#[derive(Debug, Deserialize)]
pub struct RawStargate {
    #[serde(rename = "solarSystemID")]
    pub solar_system_id: i64,
    pub destination: StargateDestination,
}

#[derive(Debug, Deserialize)]
pub struct StargateDestination {
    #[serde(rename = "solarSystemID")]
    pub solar_system_id: i64,
}

/// Localised name. We read only `en`; serde silently ignores other locales.
#[derive(Debug, Deserialize)]
pub struct LocalisedName {
    pub en: String,
}

#[derive(Debug, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

// ── Hull catalog SDE constants ──────────────────────────────────────────────
//
// All verified against CCP's JSONL SDE by downloading types/typeDogma and
// cross-checking the in-game client (build 3393779, 2026-06-14). The
// fixture-regeneration script's trim predicate mirrors these so it cannot drift
// from what the parser imports.

/// `jumpDriveRange` dogma attribute (light-years). The base jump range of a
/// hull. NOT attribute 868 (`jumpDriveConsumptionAmount`, fuel) — a tempting
/// but wrong source for range.
pub const ATTR_JUMP_DRIVE_RANGE: i64 = 867;

/// `jumpDriveRangeBonus` dogma attribute (percent per skill level). Carried by
/// the Jump Drive Calibration skill at value 20.0 (+20% range per level).
pub const ATTR_JUMP_DRIVE_RANGE_BONUS: i64 = 870;

/// typeID of the Jump Drive Calibration skill — the only skill that affects
/// jump range, universal across all jump-capable hulls. Its attribute 870 is
/// the per-level range bonus.
pub const JDC_SKILL_TYPE_ID: i64 = 21611;

/// Black Ops `groupID`. Recorded so callers can ask for the group's minimum
/// base range (the conservative blops default); it is a label, never a filter
/// on catalog membership.
pub const BLACK_OPS_GROUP_ID: i64 = 898;

/// Raw row from `types.jsonl`. Lean: every item in EVE appears here, with no
/// dogma. We keep only the fields needed to build the catalog; serde drops the
/// rest (`mass`, `portionSize`, localised names beyond `en`, …).
#[derive(Debug, Deserialize)]
pub struct RawType {
    #[serde(rename = "_key")]
    pub id: i64,
    #[serde(rename = "groupID")]
    pub group_id: i64,
    pub name: LocalisedName,
    #[serde(default)]
    pub published: bool,
}

/// Raw row from `typeDogma.jsonl`: a type's dogma attributes. Range and the JDC
/// bonus are read by attribute ID from `dogma_attributes`.
#[derive(Debug, Deserialize)]
pub struct RawTypeDogma {
    #[serde(rename = "_key")]
    pub id: i64,
    #[serde(rename = "dogmaAttributes", default)]
    pub dogma_attributes: Vec<RawDogmaAttribute>,
}

#[derive(Debug, Deserialize)]
pub struct RawDogmaAttribute {
    #[serde(rename = "attributeID")]
    pub attribute_id: i64,
    pub value: f64,
}
