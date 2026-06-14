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

/// Response body for `GET /health`.
#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    /// The loaded SDE build number (CCP's `buildNumber`).
    pub sde_version: u64,
    pub systems: usize,
    pub edges: usize,
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
    fn connection_max_size_is_optional() {
        let c: WhConnection = serde_json::from_str(r#"{"from":"Jita","to":"Urlen"}"#).unwrap();
        assert!(c.max_size.is_none());
        let c: WhConnection =
            serde_json::from_str(r#"{"from":"Jita","to":"Urlen","max_size":"large"}"#).unwrap();
        assert_eq!(c.max_size.as_deref(), Some("large"));
    }
}
