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
