//! Wire DTOs for the HTTP API, with `utoipa` derives so the OpenAPI schema is
//! generated from the same types the handlers use.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::routing::Preference;

/// A system reference: either a case-insensitive name or a numeric SDE id.
/// `serde(untagged)` lets clients send either form.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum SystemRef {
    #[schema(example = "Jita")]
    Name(String),
    #[schema(example = 30000142)]
    Id(i64),
}

/// A hull reference: either a case-insensitive name or a numeric SDE typeID.
/// Mirrors `SystemRef` — `serde(untagged)` lets clients send either form, and
/// it resolves against the hull catalog's name/typeID lookups. As with
/// `SystemRef`, a quoted numeric string is read as a *name*; ids are sent as
/// JSON numbers.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum ShipRef {
    #[schema(example = "Sin")]
    Name(String),
    #[schema(example = 22430)]
    Id(i64),
}

/// Routing preference. `prefer_gates` applies a small additive penalty per
/// wormhole hop so a wormhole is taken only when it shortens the route enough.
#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoutePreference {
    Shortest,
    Safest,
    PreferGates,
}

impl From<RoutePreference> for Preference {
    fn from(p: RoutePreference) -> Self {
        match p {
            RoutePreference::Shortest => Preference::Shortest,
            RoutePreference::Safest => Preference::Safest,
            RoutePreference::PreferGates => Preference::PreferGates,
        }
    }
}

fn default_preference() -> RoutePreference {
    RoutePreference::Shortest
}

fn default_jdc_level() -> u8 {
    crate::range::DEFAULT_JDC_LEVEL
}

/// A user-supplied wormhole connection added to the per-request overlay.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct WhConnection {
    pub from: SystemRef,
    pub to: SystemRef,
    /// `medium`/`large`/`xlarge`/`capital`. Reserved for future ship-fit
    /// filtering — parsed and stored but **not enforced** in this foundation.
    #[serde(default)]
    #[schema(example = "xlarge")]
    pub max_size: Option<String>,
}

/// Request body for `POST /api/v1/route/system`. The schema example shows a
/// minimal request; optional fields default as documented (notably
/// `include_zarzakh` defaults to `false`).
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[schema(example = json!({
    "from": "Jita",
    "to": "Amarr",
    "preference": "shortest"
}))]
pub struct GateRouteRequest {
    pub from: SystemRef,
    pub to: SystemRef,
    #[serde(default = "default_preference")]
    pub preference: RoutePreference,
    /// Systems to exclude from transit. Unknown systems yield a 400; if the
    /// only path runs through an avoided system the result is `unreachable`.
    #[serde(default)]
    pub avoid: Vec<SystemRef>,
    /// When true, `connections[]` wormhole edges are added to the overlay.
    #[serde(default)]
    #[schema(default = false, example = false)]
    pub use_wormholes: bool,
    /// Per-request wormhole connections (only used when `use_wormholes`).
    #[serde(default)]
    pub connections: Vec<WhConnection>,
    /// Add live EVE-Scout Thera signatures to the overlay.
    #[serde(default)]
    #[schema(default = false, example = false)]
    pub include_thera: bool,
    /// Add live EVE-Scout Turnur signatures to the overlay.
    #[serde(default)]
    #[schema(default = false, example = false)]
    pub include_turnur: bool,
    /// Allow Zarzakh (30100000) as a transit hop. Default `false` because its
    /// gate-lock mechanic strands ships; the caller owns the 6-hour warning.
    #[serde(default)]
    #[schema(default = false, example = false)]
    pub include_zarzakh: bool,
}

/// Response body for `POST /api/v1/route/system`.
#[derive(Debug, Serialize, ToSchema)]
pub struct GateRouteResponse {
    pub jumps: usize,
    pub path: Vec<RouteStep>,
}

/// One step in a route: the system reached and how it was reached.
#[derive(Debug, Serialize, ToSchema)]
pub struct RouteStep {
    pub system: String,
    pub system_id: i64,
    pub security: f32,
    pub sec_class: String,
    /// `start`, `stargate`, or `wormhole`.
    pub via: String,
}

/// Request body for `POST /api/v1/route/blops`. `from` is the fleet location A,
/// `to` the fixed cyno target B. `ship` and `jdc_level` are optional — when
/// omitted the handler defaults to the worst Black Ops hull at JDC 5. The
/// routing knobs (`preference`, `avoid`, wormhole overlay) apply to the A→★
/// gate leg exactly as for the system-route endpoint.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[schema(example = json!({
    "from": "B-E3KQ",
    "to": "Otanuomi",
    "ship": "Sin",
    "jdc_level": 5
}))]
pub struct BlopsRouteRequest {
    pub from: SystemRef,
    pub to: SystemRef,
    /// Bridging hull. When omitted, defaults to the worst (shortest-range)
    /// Black Ops hull in the catalog.
    #[serde(default)]
    pub ship: Option<ShipRef>,
    /// Jump Drive Calibration level (0..=5). Defaults to 5 (maxed). A value
    /// outside 0..=5 is rejected rather than clamped.
    #[serde(default = "default_jdc_level")]
    #[schema(default = 5, example = 5)]
    pub jdc_level: u8,
    #[serde(default = "default_preference")]
    pub preference: RoutePreference,
    /// Systems to exclude from the A→★ gate leg. Unknown systems yield a 400.
    #[serde(default)]
    pub avoid: Vec<SystemRef>,
    /// When true, `connections[]` wormhole edges are added to the overlay.
    #[serde(default)]
    #[schema(default = false, example = false)]
    pub use_wormholes: bool,
    /// Per-request wormhole connections (only used when `use_wormholes`).
    #[serde(default)]
    pub connections: Vec<WhConnection>,
    /// Add live EVE-Scout Thera signatures to the overlay.
    #[serde(default)]
    #[schema(default = false, example = false)]
    pub include_thera: bool,
    /// Add live EVE-Scout Turnur signatures to the overlay.
    #[serde(default)]
    #[schema(default = false, example = false)]
    pub include_turnur: bool,
    /// Allow Zarzakh as a transit hop on the gate leg (off by default).
    #[serde(default)]
    #[schema(default = false, example = false)]
    pub include_zarzakh: bool,
}

/// Response body for `POST /api/v1/route/blops`: the chosen staging route plus
/// ranked fallback candidates and the echoed bridge parameters.
#[derive(Debug, Serialize, ToSchema)]
pub struct BlopsRouteResponse {
    /// The chosen staging route: gate path A→★ and the bridge leg ★→B.
    pub chosen: BlopsChosen,
    /// Next-best in-range staging candidates beyond ★, ranked the same way
    /// (fewest gate jumps, then closest to B). Empty if ★ was the only one, or
    /// when ★ is reached in zero gate jumps (the fleet is already in bridge
    /// range, so there is nothing to fall back to).
    pub alternates: Vec<BlopsCandidate>,
    /// The JDC level used (echoed; reflects the default when omitted).
    pub jdc_level: u8,
    /// The effective bridge range in light-years used for the radius query.
    pub effective_ly: f64,
    /// True when the worst-Black-Ops-hull default was applied (no `ship` given).
    pub defaulted: bool,
}

/// The chosen staging route: the gate path to ★ and the bridge leg ★→B.
#[derive(Debug, Serialize, ToSchema)]
pub struct BlopsChosen {
    /// Gate route from A to the staging system ★ (inclusive of both ends).
    pub gate_path: Vec<RouteStep>,
    /// Gate jumps from A to ★ (`gate_path.len() - 1`).
    pub gate_jumps: usize,
    /// The bridge leg from ★ to the target B.
    pub bridge: BlopsBridge,
}

/// The bridge leg ★→B: origin, destination, light-year gap, and whether the
/// gap is within the effective range (always `true` for a chosen ★).
#[derive(Debug, Serialize, ToSchema)]
pub struct BlopsBridge {
    /// The staging system ★ (bridge origin).
    pub from: BlopsSystem,
    /// The target B (cyno destination).
    pub to: BlopsSystem,
    /// Light-year distance of the bridge jump ★→B, rounded to two decimals.
    pub jump_ly: f64,
    /// True iff `jump_ly` is within the effective bridge range.
    pub in_range: bool,
}

/// A fallback staging candidate: the system, its gate distance from A, and its
/// light-year gap to B.
#[derive(Debug, Serialize, ToSchema)]
pub struct BlopsCandidate {
    pub system: BlopsSystem,
    pub gate_jumps: usize,
    /// Light-year distance of the bridge jump from this candidate to B,
    /// rounded to two decimals.
    pub ly_to_b: f64,
}

/// A system named in a blops response: id, name, and security class.
#[derive(Debug, Serialize, ToSchema)]
pub struct BlopsSystem {
    pub system: String,
    pub system_id: i64,
    pub security: f32,
    pub sec_class: String,
}

/// Response body for `GET /health`.
#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    /// The loaded SDE build number (CCP's `buildNumber`).
    pub sde_version: u64,
    pub systems: usize,
    pub edges: usize,
    /// Number of jump-capable hulls in the loaded catalog (non-zero once the
    /// SDE type files are loaded). Confirms the catalog is populated.
    pub hull_count: usize,
    /// RFC3339 timestamp of the last successful SDE hot-reload swap; `null`
    /// until the first real swap.
    pub last_sde_reload_at: Option<chrono::DateTime<chrono::Utc>>,
    /// EVE-Scout signature count in the current snapshot (`0` until first fetch).
    pub sig_count: usize,
    /// When EVE-Scout was last fetched; `null` until the first successful fetch.
    pub last_evescout_fetch_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_ref_accepts_name_or_id() {
        let by_name: SystemRef = serde_json::from_str("\"Jita\"").unwrap();
        assert!(matches!(by_name, SystemRef::Name(n) if n == "Jita"));
        let by_id: SystemRef = serde_json::from_str("30000142").unwrap();
        assert!(matches!(by_id, SystemRef::Id(30000142)));
    }

    #[test]
    fn preference_maps_to_routing_enum() {
        assert_eq!(
            Preference::from(RoutePreference::Shortest),
            Preference::Shortest
        );
        assert_eq!(
            Preference::from(RoutePreference::Safest),
            Preference::Safest
        );
        assert_eq!(
            Preference::from(RoutePreference::PreferGates),
            Preference::PreferGates
        );
    }

    #[test]
    fn request_defaults_are_sane() {
        let req: GateRouteRequest =
            serde_json::from_str(r#"{"from":"Jita","to":"Urlen"}"#).unwrap();
        assert!(matches!(req.preference, RoutePreference::Shortest));
        assert!(req.avoid.is_empty());
        assert!(!req.use_wormholes);
        assert!(req.connections.is_empty());
        assert!(!req.include_thera && !req.include_turnur && !req.include_zarzakh);
    }

    #[test]
    fn ship_ref_accepts_name_or_id() {
        let by_name: ShipRef = serde_json::from_str("\"Sin\"").unwrap();
        assert!(matches!(by_name, ShipRef::Name(n) if n == "Sin"));
        let by_id: ShipRef = serde_json::from_str("22430").unwrap();
        assert!(matches!(by_id, ShipRef::Id(22430)));
    }

    #[test]
    fn blops_request_defaults_jdc_and_omits_ship() {
        let req: BlopsRouteRequest =
            serde_json::from_str(r#"{"from":"B-E3KQ","to":"Otanuomi"}"#).unwrap();
        assert!(req.ship.is_none(), "ship defaults to None");
        assert_eq!(req.jdc_level, 5, "jdc_level defaults to maxed (5)");
        assert!(matches!(req.preference, RoutePreference::Shortest));
        assert!(req.avoid.is_empty());
        assert!(!req.use_wormholes && req.connections.is_empty());
        assert!(!req.include_thera && !req.include_turnur && !req.include_zarzakh);
    }

    #[test]
    fn blops_request_accepts_explicit_ship_and_jdc() {
        let req: BlopsRouteRequest =
            serde_json::from_str(r#"{"from":"B-E3KQ","to":"Otanuomi","ship":22430,"jdc_level":3}"#)
                .unwrap();
        assert!(matches!(req.ship, Some(ShipRef::Id(22430))));
        assert_eq!(req.jdc_level, 3);
    }

    #[test]
    fn connection_max_size_is_optional() {
        let c: WhConnection = serde_json::from_str(r#"{"from":"Jita","to":"Urlen"}"#).unwrap();
        assert!(c.max_size.is_none());
        let c: WhConnection =
            serde_json::from_str(r#"{"from":"Jita","to":"Urlen","max_size":"large"}"#).unwrap();
        assert_eq!(c.max_size.as_deref(), Some("large"));
    }
}
