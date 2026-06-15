//! Wire DTOs for the HTTP API, with `utoipa` derives so the OpenAPI schema is
//! generated from the same types the handlers use.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::routing::Preference;

/// A system reference: either a case-insensitive name or a numeric SDE id.
/// `serde(untagged)` lets clients send either form. `Serialize` is derived so
/// the fan-out response can echo each `to` back exactly as sent — a bare string
/// (original casing) or a bare number — with no wrapper.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
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

/// The maximum number of destinations a single fan-out request may carry. A
/// sanity cap against a runaway/accidental request (e.g. `to: [millions]`), not
/// a workload limit — any legitimate fan-out is far smaller. Abuse/DoS is owned
/// by edge auth + rate limiting if the service is ever exposed publicly.
pub const MAX_DESTINATIONS: usize = 1000;

/// Request body for `POST /api/v1/route/system`. A **fan-out**: one shared
/// header (everything that is the same for every destination — `from`, the
/// routing policy, and the wormhole overlay) plus a `to` list of destinations.
/// A single route is just `to: ["X"]`. The schema example shows a minimal
/// request; optional fields default as documented (notably `include_zarzakh`
/// defaults to `false`).
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[schema(example = json!({
    "from": "Jita",
    "to": ["Amarr"],
    "preference": "shortest"
}))]
pub struct GateRouteRequest {
    pub from: SystemRef,
    /// Destinations to route to from the shared `from`. Non-empty; bounded by a
    /// sanity cap of `MAX_DESTINATIONS`. Duplicates are permitted and answered
    /// positionally. One result is returned per destination, in request order.
    pub to: Vec<SystemRef>,
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

/// Response body for `POST /api/v1/route/system`: the shared `from` echoed once,
/// then one result per destination, in request order. The overlay/preference are
/// deliberately **not** echoed (the client already holds the chain it sent).
#[derive(Debug, Serialize, ToSchema)]
pub struct GateRouteResponse {
    /// The shared source, echoed once exactly as supplied in the request.
    pub from: SystemRef,
    /// One entry per destination, in request order.
    pub results: Vec<GateRouteResult>,
}

/// One destination's outcome in a fan-out response. Always echoes the `to` it
/// answered (exactly as sent), then flattens either a successful route
/// (`jumps`/`path`) or a failure (`error`/`message`) — the same code/message a
/// single-request `4xx` would carry for that `AppError`.
#[derive(Debug, Serialize, ToSchema)]
pub struct GateRouteResult {
    /// The destination this entry answers, echoed exactly as supplied.
    pub to: SystemRef,
    #[serde(flatten)]
    pub outcome: GateRouteOutcome,
}

/// A per-destination outcome: a route, or a failure. Untagged so a success
/// serialises flat as `{ jumps, path }` and a failure as `{ error, message }`,
/// both alongside the echoed `to`.
#[derive(Debug, Serialize, ToSchema)]
#[serde(untagged)]
pub enum GateRouteOutcome {
    Route {
        jumps: usize,
        path: Vec<RouteStep>,
    },
    Failure {
        /// Stable error code (e.g. `unknown_system`, `unreachable`).
        error: String,
        /// Human-readable message for the failure.
        message: String,
    },
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
    /// Jump Drive Calibration level (1..=5). Defaults to 5 (maxed). A value
    /// outside 1..=5 is rejected rather than clamped (every jump-capable hull
    /// requires JDC 1 at a minimum).
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
    /// The JDC level used (echoed; reflects the default when omitted).
    pub jdc_level: u8,
    /// The effective bridge range in light-years used for the radius query.
    pub effective_ly: f64,
    /// True when the worst-Black-Ops-hull default was applied (no `ship` given).
    pub defaulted: bool,
    /// The chosen staging route: gate path A→★ and the bridge leg ★→B.
    pub chosen: BlopsChosen,
    /// Next-best in-range staging candidates beyond ★, ranked the same way
    /// (fewest gate jumps, then closest to B). Empty if ★ was the only one, or
    /// when ★ is reached in zero gate jumps (the fleet is already in bridge
    /// range, so there is nothing to fall back to).
    pub alternates: Vec<BlopsCandidate>,
}

/// The chosen staging route: the gate path to ★ and the bridge leg ★→B.
#[derive(Debug, Serialize, ToSchema)]
pub struct BlopsChosen {
    /// Gate jumps from A to ★ (`gate_path.len() - 1`).
    pub gate_jumps: usize,
    /// Gate route from A to the staging system ★ (inclusive of both ends).
    pub gate_path: Vec<RouteStep>,
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

/// Request body for `POST /api/v1/route/range`. The jump-range reachability
/// fan-out: "from `from`, with `ship` at `jdc_level`, which systems can I reach
/// in one jump?". Unlike the blops endpoint this is planning-oriented: `ship`
/// and `jdc_level` are **required** (no worst-hull or default-level fallback),
/// and there are no gate/wormhole overlay knobs — a jump ignores gates.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[schema(example = json!({
    "from": "Otanuomi",
    "ship": "Sin",
    "jdc_level": 5
}))]
pub struct RangeRequest {
    /// Source system the jump originates from.
    pub from: SystemRef,
    /// Jumping hull, by case-insensitive name or numeric typeID. Required —
    /// there is no worst-hull default for this endpoint.
    pub ship: ShipRef,
    /// Jump Drive Calibration level (1..=5). Required — there is no default.
    /// Every jump-capable hull requires JDC 1 at a minimum, so `0` (and any
    /// value above 5) is rejected rather than clamped.
    #[schema(example = 5)]
    pub jdc_level: u8,
}

/// Response body for `POST /api/v1/route/range`: the source, the resolved hull,
/// the effective range used, a summary, and every reachable system sorted by
/// ascending light-year distance. An empty `reachable` list (with a zero
/// summary count) is a valid 200 answer, not an error.
#[derive(Debug, Serialize, ToSchema)]
pub struct RangeResponse {
    /// Aggregate view of the reachable set.
    pub summary: RangeSummary,
    /// The JDC level used (echoed from the request).
    pub jdc_level: u8,
    /// The effective jump range in light-years used for the radius query.
    pub effective_ly: f64,
    /// The source system the jump originates from.
    pub source: RangeSystem,
    /// The resolved jumping hull and its base range.
    pub hull: RangeHull,
    /// Reachable systems, sorted by ascending light-year distance from `source`.
    pub reachable: Vec<RangeReachable>,
}

/// The resolved jumping hull echoed in a range response.
#[derive(Debug, Serialize, ToSchema)]
pub struct RangeHull {
    pub name: String,
    pub type_id: i64,
    /// Base jump range in light-years (SDE attribute 867), before the JDC bonus.
    pub base_ly: f64,
}

/// Aggregate summary of a range response's reachable set.
#[derive(Debug, Serialize, ToSchema)]
pub struct RangeSummary {
    /// Number of reachable systems.
    pub reachable_count: usize,
    /// Light-year distance to the farthest reachable system, rounded to two
    /// decimals. `0.0` when the set is empty.
    pub farthest_ly: f64,
    /// Reachable-system count broken down by security class label
    /// (`Lowsec`/`Nullsec`/`Wormhole`). Highsec never appears — a cyno cannot
    /// be lit in highsec, so highsec systems are not reachable.
    pub by_sec_class: std::collections::BTreeMap<String, usize>,
}

/// One reachable system in a range response: identity plus the jump distance.
#[derive(Debug, Serialize, ToSchema)]
pub struct RangeReachable {
    pub system: String,
    pub system_id: i64,
    pub security: f32,
    pub sec_class: String,
    /// Light-year distance of the jump from `source` to this system, rounded to
    /// two decimals.
    pub jump_ly: f64,
}

/// A system named in a range response (source or reachable entry header).
#[derive(Debug, Serialize, ToSchema)]
pub struct RangeSystem {
    pub system: String,
    pub system_id: i64,
    pub security: f32,
    pub sec_class: String,
}

/// Response body for `GET /health`.
#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    /// Application version. CI derives this from git tags; local/dev builds
    /// report the crate placeholder (`0.0.0`).
    #[schema(example = "1.2.3")]
    pub app_version: String,
    /// Git commit the build was cut from; `"unknown"` for local builds.
    #[schema(example = "abc1234")]
    pub git_commit: String,
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
            serde_json::from_str(r#"{"from":"Jita","to":["Urlen"]}"#).unwrap();
        assert!(matches!(req.preference, RoutePreference::Shortest));
        assert!(req.avoid.is_empty());
        assert!(!req.use_wormholes);
        assert!(req.connections.is_empty());
        assert!(!req.include_thera && !req.include_turnur && !req.include_zarzakh);
    }

    #[test]
    fn fanout_request_deserializes_header_plus_to_list() {
        // The shared header is stated once; `to` is a list of destinations.
        let req: GateRouteRequest = serde_json::from_str(
            r#"{"from":"Jita","to":["Amarr","Hek",30000142],"preference":"safest"}"#,
        )
        .unwrap();
        assert!(matches!(req.from, SystemRef::Name(ref n) if n == "Jita"));
        assert_eq!(req.to.len(), 3);
        assert!(matches!(req.to[0], SystemRef::Name(ref n) if n == "Amarr"));
        // Mixed name/id destinations are accepted in one list.
        assert!(matches!(req.to[2], SystemRef::Id(30000142)));
        assert!(matches!(req.preference, RoutePreference::Safest));
    }

    #[test]
    fn fanout_response_echoes_from_once_with_results() {
        let resp = GateRouteResponse {
            from: SystemRef::Name("Jita".into()),
            results: vec![GateRouteResult {
                to: SystemRef::Name("Amarr".into()),
                outcome: GateRouteOutcome::Route {
                    jumps: 11,
                    path: vec![],
                },
            }],
        };
        let v = serde_json::to_value(&resp).unwrap();
        // `from` echoed once at the top level, as a bare string (untagged).
        assert_eq!(v["from"], "Jita");
        assert!(
            v.get("preference").is_none(),
            "overlay/policy is not echoed back"
        );
        assert_eq!(v["results"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn success_entry_serializes_as_route_with_echoed_to() {
        let entry = GateRouteResult {
            to: SystemRef::Name("Amarr".into()),
            outcome: GateRouteOutcome::Route {
                jumps: 2,
                path: vec![RouteStep {
                    system: "Jita".into(),
                    system_id: 30000142,
                    security: 0.9,
                    sec_class: "Highsec".into(),
                    via: "start".into(),
                }],
            },
        };
        let v = serde_json::to_value(&entry).unwrap();
        // Echoed `to` sits flat alongside the flattened route fields.
        assert_eq!(v["to"], "Amarr");
        assert_eq!(v["jumps"], 2);
        assert!(v["path"].is_array());
        assert!(v.get("error").is_none());
    }

    #[test]
    fn failure_entry_serializes_as_error_with_echoed_to() {
        let entry = GateRouteResult {
            to: SystemRef::Id(30000142),
            outcome: GateRouteOutcome::Failure {
                error: "unreachable".into(),
                message: "no gate route between the requested systems".into(),
            },
        };
        let v = serde_json::to_value(&entry).unwrap();
        // A failed destination echoes its `to` as sent (here a bare number) and
        // carries error/message instead of a route.
        assert_eq!(v["to"], 30000142);
        assert_eq!(v["error"], "unreachable");
        assert!(v["message"].is_string());
        assert!(v.get("jumps").is_none() && v.get("path").is_none());
    }

    #[test]
    fn echoed_to_preserves_as_sent_name_casing_and_id() {
        // Round-trip a request and a response: the echoed key must preserve the
        // client's original casing (a name) and bare-number form (an id).
        let by_name = serde_json::to_value(SystemRef::Name("aMaRr".into())).unwrap();
        assert_eq!(by_name, "aMaRr");
        let by_id = serde_json::to_value(SystemRef::Id(30002187)).unwrap();
        assert_eq!(by_id, 30002187);
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
    fn range_request_requires_ship_and_jdc() {
        // A full request parses.
        let req: RangeRequest =
            serde_json::from_str(r#"{"from":"Otanuomi","ship":"Sin","jdc_level":5}"#).unwrap();
        assert!(matches!(req.ship, ShipRef::Name(n) if n == "Sin"));
        assert_eq!(req.jdc_level, 5);
        // ship is not optional — omitting it fails to deserialize.
        assert!(
            serde_json::from_str::<RangeRequest>(r#"{"from":"Otanuomi","jdc_level":5}"#).is_err()
        );
        // jdc_level is not optional either.
        assert!(
            serde_json::from_str::<RangeRequest>(r#"{"from":"Otanuomi","ship":"Sin"}"#).is_err()
        );
    }

    #[test]
    fn range_request_accepts_numeric_ids() {
        let req: RangeRequest =
            serde_json::from_str(r#"{"from":30003549,"ship":22430,"jdc_level":3}"#).unwrap();
        assert!(matches!(req.from, SystemRef::Id(30003549)));
        assert!(matches!(req.ship, ShipRef::Id(22430)));
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
