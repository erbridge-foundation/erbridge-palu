//! End-to-end handler→service→graph tests against the cached SDE fixture in
//! `tests/fixtures/sde` (the full real SDE, loaded via the normal cache path).
//! No live CCP or EVE-Scout calls — the graph is loaded from disk and the
//! EVE-Scout snapshot is injected directly into `AppState`.
//!
//! Reference facts on the real SDE:
//! - Jita → Amarr is 11 gate jumps (shortest, one lowsec hop); safest stays
//!   all-highsec and is much longer.
//! - G-0Q86 → H-PA29 is 2 jumps through Zarzakh (the short bridge), which the
//!   default excludes so the route detours.
//! - J-space systems are gateless: unreachable without a wormhole overlay.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use chrono::Utc;
use tower::ServiceExt;

use erbridge_palu::app_state::AppState;
use erbridge_palu::build_router;
use erbridge_palu::eve_scout::{EveScoutSnapshot, Signature, THERA_SYSTEM_ID};
use erbridge_palu::graph::build_graph_data;
use erbridge_palu::sde::cache::SdeCache;

fn fixture_state() -> AppState {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sde");
    let cache = SdeCache::new(dir);
    let meta = cache.current_metadata().expect("fixture metadata present");
    let raw = cache.load_build(meta.build_number).expect("fixture loads");
    let graph = build_graph_data(raw, meta.build_number);
    AppState::new(Arc::new(graph))
}

async fn post_fanout(state: AppState, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    let app = build_router(state);
    let req = Request::builder()
        .uri("/api/v1/route/system")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

/// Post a single-destination fan-out (`to: [dest]`, every other field shared) and
/// return `(status, results[0])` — the per-destination entry. Most legacy
/// single-route tests assert against one destination, so this keeps them concise
/// while exercising the real `{ from, results }` envelope.
async fn post_route(
    state: AppState,
    mut body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    // Wrap the scalar `to` the legacy tests pass into the fan-out list shape.
    if let Some(to) = body.get("to").cloned() {
        body["to"] = serde_json::Value::Array(vec![to]);
    }
    let (status, full) = post_fanout(state, body).await;
    if status != StatusCode::OK {
        // A request-level error has no `results`; surface the error body as-is.
        return (status, full);
    }
    let entry = full["results"][0].clone();
    (status, entry)
}

// ── basic routing ────────────────────────────────────────────────────────────

#[tokio::test]
async fn shortest_jita_to_amarr_is_eleven_jumps() {
    let (status, body) = post_route(
        fixture_state(),
        serde_json::json!({ "from": "Jita", "to": "Amarr", "preference": "shortest" }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["jumps"], 11);
    let path = body["path"].as_array().unwrap();
    assert_eq!(path.first().unwrap()["system"], "Jita");
    assert_eq!(path.first().unwrap()["via"], "start");
    assert_eq!(path.last().unwrap()["system"], "Amarr");
}

#[tokio::test]
async fn safest_jita_to_amarr_stays_highsec_and_is_longer() {
    let (status, body) = post_route(
        fixture_state(),
        serde_json::json!({ "from": "Jita", "to": "Amarr", "preference": "safest" }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    let jumps = body["jumps"].as_u64().unwrap();
    // Safest detours around the lowsec hop, so it is much longer than shortest.
    assert!(jumps > 11, "safest ({jumps}) should exceed shortest (11)");
    // Every step on the safest route is highsec.
    for step in body["path"].as_array().unwrap() {
        assert_eq!(
            step["sec_class"], "Highsec",
            "safest must stay highsec, got {step}"
        );
    }
}

#[tokio::test]
async fn wormhole_shortcut_makes_jita_to_amarr_four_jumps() {
    // A user wormhole Jita↔Akhragan (Akhragan is 3 gate jumps from Amarr)
    // collapses the 11-jump trip to 4. The WH hop is labelled `wormhole`.
    let (status, body) = post_route(
        fixture_state(),
        serde_json::json!({
            "from": "Jita",
            "to": "Amarr",
            "preference": "shortest",
            "use_wormholes": true,
            "connections": [ { "from": "Jita", "to": "Akhragan" } ]
        }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["jumps"], 4, "WH shortcut should give a 4-jump route");
    let path = body["path"].as_array().unwrap();
    assert_eq!(path[0]["system"], "Jita");
    assert_eq!(path[1]["system"], "Akhragan");
    assert_eq!(path[1]["via"], "wormhole");
    assert_eq!(path.last().unwrap()["system"], "Amarr");
}

#[tokio::test]
async fn prefer_gates_keeps_gates_over_marginal_wormhole() {
    // A WH that saves only 1 jump is not worth the prefer_gates penalty.
    // Jita→Perimeter is 1 gate jump; a WH Jita↔Perimeter saves nothing, so
    // prefer_gates keeps the gate (still 1 jump, labelled stargate).
    let (status, body) = post_route(
        fixture_state(),
        serde_json::json!({
            "from": "Jita",
            "to": "Perimeter",
            "preference": "prefer_gates",
            "use_wormholes": true,
            "connections": [ { "from": "Jita", "to": "Perimeter" } ]
        }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["jumps"], 1);
    assert_eq!(body["path"][1]["via"], "stargate");
}

#[tokio::test]
async fn prefer_gates_takes_wormhole_that_saves_enough() {
    // Jita→Amarr via gates is 11; a WH Jita↔Akhragan cuts it to 4 (saves 7),
    // which beats the small additive penalty, so prefer_gates uses it.
    let (status, body) = post_route(
        fixture_state(),
        serde_json::json!({
            "from": "Jita",
            "to": "Amarr",
            "preference": "prefer_gates",
            "use_wormholes": true,
            "connections": [ { "from": "Jita", "to": "Akhragan" } ]
        }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["path"][1]["via"], "wormhole");
}

// ── avoid ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn avoid_reroutes_or_lengthens() {
    // Avoiding Perimeter (Jita's neighbour toward Amarr on the shortest path)
    // must still resolve, but not transit Perimeter.
    let (status, body) = post_route(
        fixture_state(),
        serde_json::json!({ "from": "Jita", "to": "Amarr", "avoid": ["Perimeter"] }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    for step in body["path"].as_array().unwrap() {
        assert_ne!(step["system"], "Perimeter");
    }
}

// ── Zarzakh ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn zarzakh_excluded_by_default() {
    // G-0Q86 → H-PA29 is 2 jumps through Zarzakh; by default Zarzakh is excluded
    // so the route detours the long way and never transits Zarzakh.
    let (status, body) = post_route(
        fixture_state(),
        serde_json::json!({ "from": "G-0Q86", "to": "H-PA29" }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    assert!(
        body["jumps"].as_u64().unwrap() > 2,
        "should detour around Zarzakh"
    );
    assert!(
        !body["path"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["system"] == "Zarzakh"),
        "default route must not transit Zarzakh"
    );
}

#[tokio::test]
async fn zarzakh_opt_in_allows_transit() {
    let (status, body) = post_route(
        fixture_state(),
        serde_json::json!({ "from": "G-0Q86", "to": "H-PA29", "include_zarzakh": true }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["jumps"], 2);
    assert!(
        body["path"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["system"] == "Zarzakh")
    );
}

// ── errors ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn unknown_system_is_400() {
    let (status, body) = post_route(
        fixture_state(),
        serde_json::json!({ "from": "Nowhere", "to": "Jita" }),
    )
    .await;
    assert_eq!(status, 400, "body={body}");
    assert_eq!(body["error"], "unknown_system");
}

#[tokio::test]
async fn unreachable_destination_is_in_slot_failure_at_200() {
    // J105443 is a gateless wormhole system with no overlay edge → unreachable.
    // A destination failure no longer fails the request: the fan-out returns 200
    // with the error carried in that destination's slot (still echoing its `to`).
    let (status, entry) = post_route(
        fixture_state(),
        serde_json::json!({ "from": "Jita", "to": "J105443" }),
    )
    .await;
    assert_eq!(status, 200, "entry={entry}");
    assert_eq!(entry["error"], "unreachable");
    assert!(entry["message"].is_string());
    assert_eq!(entry["to"], "J105443");
    assert!(entry.get("jumps").is_none());
}

#[tokio::test]
async fn accepts_numeric_system_id() {
    // Jita = 30000142, Perimeter = 30000144.
    let (status, body) = post_route(
        fixture_state(),
        serde_json::json!({ "from": 30000142_i64, "to": 30000144_i64 }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["jumps"], 1);
}

// ── fan-out (shared header + many destinations) ───────────────────────────────────

#[tokio::test]
async fn fanout_echoes_from_and_answers_in_request_order() {
    // One shared header, three resolvable destinations: the response echoes
    // `from` once and returns one route per destination, in request order.
    let (status, body) = post_fanout(
        fixture_state(),
        serde_json::json!({ "from": "Jita", "to": ["Amarr", "Perimeter", "Amarr"] }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["from"], "Jita");
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 3);
    // Each entry echoes its `to`, in order (duplicates answered positionally).
    assert_eq!(results[0]["to"], "Amarr");
    assert_eq!(results[1]["to"], "Perimeter");
    assert_eq!(results[2]["to"], "Amarr");
    // Jita→Amarr is 11; Jita→Perimeter is 1; the duplicate Amarr matches the first.
    assert_eq!(results[0]["jumps"], 11);
    assert_eq!(results[1]["jumps"], 1);
    assert_eq!(results[2]["jumps"], 11);
}

#[tokio::test]
async fn fanout_mixes_routes_and_per_destination_failures_at_200() {
    // Good + unknown + unreachable destinations in one request: still 200, with
    // failures isolated to their slots and the good route intact.
    let (status, body) = post_fanout(
        fixture_state(),
        serde_json::json!({ "from": "Jita", "to": ["Amarr", "Nowhere", "J105443"] }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 3);
    // Good destination: a route.
    assert_eq!(results[0]["jumps"], 11);
    // Unknown destination: in-slot unknown_system, echoing its `to`.
    assert_eq!(results[1]["error"], "unknown_system");
    assert_eq!(results[1]["to"], "Nowhere");
    // Unreachable destination: in-slot unreachable.
    assert_eq!(results[2]["error"], "unreachable");
    assert_eq!(results[2]["to"], "J105443");
}

#[tokio::test]
async fn fanout_bad_shared_from_is_request_level_400() {
    // A bad shared `from` fails the whole request (header tier) even though the
    // destinations are individually valid — no `results` are computed.
    let (status, body) = post_fanout(
        fixture_state(),
        serde_json::json!({ "from": "Nowhere", "to": ["Jita", "Amarr"] }),
    )
    .await;
    assert_eq!(status, 400, "body={body}");
    assert_eq!(body["error"], "unknown_system");
    assert!(body.get("results").is_none());
}

#[tokio::test]
async fn fanout_empty_to_is_request_level_400() {
    let (status, body) = post_fanout(
        fixture_state(),
        serde_json::json!({ "from": "Jita", "to": [] }),
    )
    .await;
    assert_eq!(status, 400, "body={body}");
    assert_eq!(body["error"], "invalid_param");
}

#[tokio::test]
async fn fanout_over_cap_to_is_request_level_400() {
    // 1001 destinations exceeds the sanity cap of 1000 → request-level 400.
    let over_cap: Vec<&str> = std::iter::repeat_n("Jita", 1001).collect();
    let (status, body) = post_fanout(
        fixture_state(),
        serde_json::json!({ "from": "Jita", "to": over_cap }),
    )
    .await;
    assert_eq!(status, 400, "body={body}");
    assert_eq!(body["error"], "invalid_param");
}

// ── EVE-Scout overlay (snapshot injected, no network) ─────────────────────────────

#[tokio::test]
async fn include_thera_uses_injected_snapshot() {
    let state = fixture_state();
    // Inject a live Thera→Akhragan signature, then route from Akhragan to
    // Amarr with include_thera enabled: the snapshot is consulted (no network)
    // and the request resolves normally.
    let snap = EveScoutSnapshot {
        thera: vec![Signature {
            out_system_id: THERA_SYSTEM_ID,
            in_system_id: 30002197, // Akhragan
            in_system_name: "Akhragan".into(),
            max_ship_size: Some("xlarge".into()),
            expires_at: Utc::now() + chrono::Duration::hours(2),
        }],
        turnur: vec![],
        fetched_at: Some(Utc::now()),
    };
    state.eve_scout.store(Arc::new(snap));

    let (status, body) = post_route(
        state,
        serde_json::json!({ "from": "Akhragan", "to": "Amarr", "include_thera": true }),
    )
    .await;
    // Akhragan→Amarr is 3 gate jumps regardless; the point is include_thera
    // reads the snapshot without a network call and the route still resolves.
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["jumps"], 3);
}

#[tokio::test]
async fn thera_reachable_as_dest_without_include_flag() {
    // Thera is a gateless wormhole. With a live Thera↔Akhragan sig, routing TO
    // Thera must resolve even though include_thera is omitted (defaults false) —
    // an EVE-Scout hub is usable as an endpoint without opting in.
    let state = fixture_state();
    let snap = EveScoutSnapshot {
        thera: vec![Signature {
            out_system_id: THERA_SYSTEM_ID,
            in_system_id: 30002197, // Akhragan
            in_system_name: "Akhragan".into(),
            max_ship_size: Some("xlarge".into()),
            expires_at: Utc::now() + chrono::Duration::hours(2),
        }],
        turnur: vec![],
        fetched_at: Some(Utc::now()),
    };
    state.eve_scout.store(Arc::new(snap));

    let (status, body) = post_route(
        state,
        serde_json::json!({ "from": "Akhragan", "to": "Thera" }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["jumps"], 1);
    assert_eq!(
        body["path"].as_array().unwrap().last().unwrap()["system"],
        "Thera"
    );
    assert_eq!(
        body["path"].as_array().unwrap().last().unwrap()["via"],
        "wormhole"
    );
}

// ── hull catalog ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn catalog_loads_blops_and_resolves_known_hull() {
    use erbridge_palu::model::HullCatalog;

    // Build the catalog from the fixture the same way the app does.
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sde");
    let cache = SdeCache::new(dir);
    let meta = cache.current_metadata().expect("fixture metadata present");
    let raw = cache.load_build(meta.build_number).expect("fixture loads");
    let catalog = HullCatalog::from_raw(raw.hulls);

    // Black Ops (group 898) min base range sits in a sane band (~4 LY) — a loose
    // guard against parsing the wrong attribute, deliberately not pinned exact.
    let blops_min = catalog
        .min_base_ly_for_group(898)
        .expect("fixture contains Black Ops hulls");
    assert!(
        (3.0..=5.0).contains(&blops_min),
        "Black Ops min base range {blops_min} LY outside sane band"
    );

    // A known Black Ops hull resolves by name (case-insensitive) and by typeID.
    let sin = catalog.by_name("sin").expect("Sin in catalog");
    assert_eq!(sin.group_id, 898);
    assert_eq!(catalog.by_type_id(22430), Some(sin));

    // The JDC per-level bonus was read from the SDE (attribute 870 = 20%).
    assert!((catalog.bonus_per_level() - 0.20).abs() < 1e-9);
}

// ── health + openapi ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn health_reports_graph_summary_without_auth() {
    let app = build_router(fixture_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["status"], "ok");
    // App version + commit are always present. Under `cargo test` (no
    // PALU_APP_VERSION) the version is the crate placeholder "0.0.0" and the
    // commit is "unknown"; in a CI image they are the git-derived values.
    assert_eq!(body["app_version"], "0.0.0");
    assert_eq!(body["git_commit"], "unknown");
    // sde_version is the fixture's cached SDE build (whatever metadata.json says).
    assert!(body["sde_version"].as_u64().unwrap() > 0);
    assert!(body["systems"].as_u64().unwrap() > 0);
    assert!(body["edges"].as_u64().unwrap() > 0);
    // The hull catalog loaded from the fixture's type files.
    assert!(body["hull_count"].as_u64().unwrap() > 0);
    // Freshness fields are null/0 before any reload swap or EVE-Scout fetch.
    assert!(body["last_sde_reload_at"].is_null());
    assert_eq!(body["sig_count"], 0);
    assert!(body["last_evescout_fetch_at"].is_null());
}

#[tokio::test]
async fn openapi_json_is_served() {
    let app = build_router(fixture_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api-docs/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(doc["openapi"].as_str().unwrap().starts_with("3."));
    assert!(doc["paths"]["/api/v1/route/system"].is_object());
    assert!(doc["paths"]["/health"].is_object());
}

#[tokio::test]
async fn swagger_ui_is_reachable() {
    let app = build_router(fixture_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/swagger-ui")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Swagger UI redirects to its index; either a redirect or 200 is fine.
    assert!(
        resp.status().is_success() || resp.status().is_redirection(),
        "unexpected status {}",
        resp.status()
    );
}

// ── blops staging (handler → service over the full SDE fixture) ───────────────
//
// Pinned in-fixture example: B-E3KQ (A) → Otanuomi (B) — both nullsec — bridged
// by a Sin at JDC 5 (effective range 8.0 LY). The fleet drives just two gate
// jumps (B-E3KQ → 5T-KM3 → 0R-F2F) to the fewest-gate in-range staging system
// 0R-F2F, then bridges the final ~5.81 LY to Otanuomi. This is the realistic
// black-ops scenario: a short nullsec drive to get in bridge range, not a
// cross-map haul. Pinned in the style of the Jita↔Amarr gate-count facts above;
// a fixture regen that moves the map is meant to surface here.

#[tokio::test]
async fn blops_happy_path_routes_to_expected_staging() {
    let (status, body) = post_blops(
        fixture_state(),
        serde_json::json!({ "from": "B-E3KQ", "to": "Otanuomi", "ship": "Sin", "jdc_level": 5 }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["effective_ly"], 8.0);
    assert_eq!(body["jdc_level"], 5);
    assert_eq!(body["defaulted"], false);

    let chosen = &body["chosen"];
    assert_eq!(chosen["gate_jumps"], 2, "two-jump drive into bridge range");
    // Gate path: A → one nullsec hop → ★, all by stargate.
    let path = chosen["gate_path"].as_array().unwrap();
    assert_eq!(path.len(), 3);
    assert_eq!(path.first().unwrap()["system"], "B-E3KQ");
    assert_eq!(path.first().unwrap()["via"], "start");
    assert_eq!(path[1]["system"], "5T-KM3");
    assert_eq!(path[1]["via"], "stargate");
    assert_eq!(path.last().unwrap()["system"], "0R-F2F");
    assert_eq!(path.last().unwrap()["via"], "stargate");
    // A and B are both nullsec (the realistic blops framing).
    assert_eq!(path.first().unwrap()["sec_class"], "Nullsec");
    assert_eq!(chosen["bridge"]["from"]["system"], "0R-F2F");
    assert_eq!(chosen["bridge"]["from"]["sec_class"], "Nullsec");
    assert_eq!(chosen["bridge"]["to"]["system"], "Otanuomi");
    assert_eq!(chosen["bridge"]["to"]["sec_class"], "Nullsec");
    assert_eq!(chosen["bridge"]["in_range"], true);
    // Jump distance is reported in light-years, rounded to two decimals.
    let jump_ly = chosen["bridge"]["jump_ly"].as_f64().unwrap();
    assert_eq!(jump_ly, 5.81, "pinned rounded bridge distance");
    assert!(jump_ly <= 8.0, "bridge must be within effective range");
    // Fallback candidates are present and ranked no-better than the chosen.
    let alts = body["alternates"].as_array().unwrap();
    assert!(!alts.is_empty(), "expected inline fallback candidates");
    assert!(alts[0]["gate_jumps"].as_u64().unwrap() >= 2);
}

#[tokio::test]
async fn blops_already_in_range_needs_zero_gate_jumps() {
    // Start from the staging system itself: 0R-F2F is ~5.81 LY from Otanuomi,
    // inside a Sin's 8.0 LY range, so the fleet is already in bridge range. The
    // chosen ★ is 0R-F2F itself at zero gate jumps (the gate path is just the
    // start system), and no fallback candidates are offered — there is nothing
    // to drive to.
    let (status, body) = post_blops(
        fixture_state(),
        serde_json::json!({ "from": "0R-F2F", "to": "Otanuomi", "ship": "Sin", "jdc_level": 5 }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    let chosen = &body["chosen"];
    assert_eq!(chosen["gate_jumps"], 0, "already in range → no driving");
    let path = chosen["gate_path"].as_array().unwrap();
    assert_eq!(path.len(), 1, "gate path is just the start system");
    assert_eq!(path[0]["system"], "0R-F2F");
    assert_eq!(path[0]["via"], "start");
    assert_eq!(chosen["bridge"]["from"]["system"], "0R-F2F");
    assert_eq!(chosen["bridge"]["to"]["system"], "Otanuomi");
    assert_eq!(chosen["bridge"]["in_range"], true);
    // No alternates when no gate jumps are required.
    assert_eq!(
        body["alternates"].as_array().unwrap().len(),
        0,
        "zero-jump staging offers no fallbacks"
    );
}

#[tokio::test]
async fn blops_defaults_to_worst_blops_hull() {
    // Omitting `ship` uses the catalog's worst Black Ops hull and flags it.
    let (status, body) = post_blops(
        fixture_state(),
        serde_json::json!({ "from": "B-E3KQ", "to": "Otanuomi" }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["defaulted"], true);
    assert_eq!(body["jdc_level"], 5, "jdc_level defaults to maxed");
    // The worst-hull effective range still resolves the same staging system.
    assert_eq!(body["chosen"]["bridge"]["from"]["system"], "0R-F2F");
}

#[tokio::test]
async fn blops_highsec_target_is_rejected() {
    // Amarr is highsec → a cyno cannot be lit there; rejected before routing
    // with the distinct error (not a generic unreachable).
    let (status, body) = post_blops(
        fixture_state(),
        serde_json::json!({ "from": "Jita", "to": "Amarr", "ship": "Sin" }),
    )
    .await;
    assert_eq!(status, 400, "body={body}");
    assert_eq!(body["error"], "cyno_target_highsec");
}

#[tokio::test]
async fn blops_no_candidate_in_range_is_404() {
    // J105443 (a J-space system) at JDC 1 (Sin 4.8 LY) has no K-space staging
    // system within range → distinct no_staging_in_range, not unreachable.
    // (J-space coordinates sit far from the K-space cluster, so even a maxed
    // bridge would not reach; JDC 1 is the minimum valid level.)
    let (status, body) = post_blops(
        fixture_state(),
        serde_json::json!({ "from": "Jita", "to": "J105443", "ship": "Sin", "jdc_level": 1 }),
    )
    .await;
    assert_eq!(status, 404, "body={body}");
    assert_eq!(body["error"], "no_staging_in_range");
}

#[tokio::test]
async fn blops_unknown_hull_is_400() {
    let (status, body) = post_blops(
        fixture_state(),
        serde_json::json!({ "from": "B-E3KQ", "to": "Otanuomi", "ship": "Rifter" }),
    )
    .await;
    assert_eq!(status, 400, "body={body}");
    assert_eq!(body["error"], "unknown_hull");
}

#[tokio::test]
async fn blops_invalid_jdc_level_is_400() {
    let (status, body) = post_blops(
        fixture_state(),
        serde_json::json!({ "from": "B-E3KQ", "to": "Otanuomi", "ship": "Sin", "jdc_level": 9 }),
    )
    .await;
    assert_eq!(status, 400, "body={body}");
    assert_eq!(body["error"], "invalid_param");
}

#[tokio::test]
async fn blops_accepts_numeric_system_and_hull_ids() {
    // B-E3KQ = 30000307, Otanuomi = 30000192, Sin = 22430. Numeric forms
    // resolve identically to names.
    let (status, body) = post_blops(
        fixture_state(),
        serde_json::json!({ "from": 30000307_i64, "to": 30000192_i64, "ship": 22430_i64, "jdc_level": 5 }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["chosen"]["bridge"]["from"]["system"], "0R-F2F");
}

// ── jump-range reachability (fan-out over the full SDE fixture) ───────────────
//
// Pinned in-fixture example: from Otanuomi (nullsec), a Sin at JDC 5 (effective
// range 8.0 LY) reaches a set of nearby K-space systems including 0R-F2F (~5.81
// LY — the same pair the blops staging test pins). No gate routing is involved;
// this is a pure spatial fan-out.

#[tokio::test]
async fn range_happy_path_lists_reachable_systems() {
    let (status, body) = post_range(
        fixture_state(),
        serde_json::json!({ "from": "Otanuomi", "ship": "Sin", "jdc_level": 5 }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    // Echoed inputs: Sin 4.0 base × (1 + 0.20×5) = 8.0 LY.
    assert_eq!(body["effective_ly"], 8.0);
    assert_eq!(body["jdc_level"], 5);
    assert_eq!(body["hull"]["name"], "Sin");
    assert_eq!(body["hull"]["type_id"], 22430);
    assert_eq!(body["hull"]["base_ly"], 4.0);
    assert_eq!(body["source"]["system"], "Otanuomi");

    let reachable = body["reachable"].as_array().unwrap();
    assert!(!reachable.is_empty(), "Otanuomi reaches systems at 8 LY");
    // The source is never listed among the reachable systems.
    assert!(
        reachable.iter().all(|r| r["system"] != "Otanuomi"),
        "source excluded from its own reachable set"
    );
    // 0R-F2F (the blops staging pin, ~5.81 LY) is reachable.
    let zero_r = reachable
        .iter()
        .find(|r| r["system"] == "0R-F2F")
        .expect("0R-F2F within 8 LY of Otanuomi");
    assert_eq!(zero_r["jump_ly"], 5.81, "pinned rounded jump distance");
    // Sorted ascending by jump distance.
    let lys: Vec<f64> = reachable
        .iter()
        .map(|r| r["jump_ly"].as_f64().unwrap())
        .collect();
    assert!(lys.windows(2).all(|w| w[0] <= w[1]), "sorted ascending");
    // No highsec system appears (a cyno cannot be lit in highsec).
    assert!(
        reachable.iter().all(|r| r["sec_class"] != "Highsec"),
        "highsec destinations excluded"
    );

    // Summary agrees with the list.
    let summary = &body["summary"];
    assert_eq!(
        summary["reachable_count"].as_u64().unwrap() as usize,
        reachable.len()
    );
    assert_eq!(
        summary["farthest_ly"].as_f64().unwrap(),
        *lys.last().unwrap()
    );
    assert!(summary["by_sec_class"].get("Highsec").is_none());
}

#[tokio::test]
async fn range_excludes_highsec_and_summary_omits_it() {
    // From a highsec hub (Jita), a Sin at JDC 5 covers a dense neighbourhood —
    // but every reachable system must be K-space and non-highsec, even though
    // Jita itself sits among highsec systems.
    let (status, body) = post_range(
        fixture_state(),
        serde_json::json!({ "from": "Jita", "ship": "Sin", "jdc_level": 5 }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    let reachable = body["reachable"].as_array().unwrap();
    assert!(
        reachable.iter().all(|r| r["sec_class"] != "Highsec"),
        "no highsec system is reachable (cyno rule)"
    );
    assert!(body["summary"]["by_sec_class"].get("Highsec").is_none());
}

#[tokio::test]
async fn range_missing_required_field_is_rejected() {
    // `ship` and `jdc_level` are required. A missing required field fails at the
    // JSON extractor (axum returns 422 Unprocessable Entity for a body that does
    // not deserialize), distinct from a present-but-invalid value which is our
    // own 400. Either way the request is rejected, never silently defaulted.
    let (no_jdc, _) = post_range(
        fixture_state(),
        serde_json::json!({ "from": "Otanuomi", "ship": "Sin" }),
    )
    .await;
    assert_eq!(no_jdc, 422, "missing jdc_level rejected by the extractor");

    let (no_ship, _) = post_range(
        fixture_state(),
        serde_json::json!({ "from": "Otanuomi", "jdc_level": 5 }),
    )
    .await;
    assert_eq!(no_ship, 422, "missing ship rejected by the extractor");
}

#[tokio::test]
async fn range_jdc_zero_is_400() {
    // Every jump-capable hull requires JDC 1; 0 is rejected.
    let (status, body) = post_range(
        fixture_state(),
        serde_json::json!({ "from": "Otanuomi", "ship": "Sin", "jdc_level": 0 }),
    )
    .await;
    assert_eq!(status, 400, "body={body}");
    assert_eq!(body["error"], "invalid_param");
}

#[tokio::test]
async fn range_unknown_hull_is_400() {
    let (status, body) = post_range(
        fixture_state(),
        serde_json::json!({ "from": "Otanuomi", "ship": "Rifter", "jdc_level": 5 }),
    )
    .await;
    assert_eq!(status, 400, "body={body}");
    assert_eq!(body["error"], "unknown_hull");
}

#[tokio::test]
async fn range_accepts_numeric_system_and_hull_ids() {
    // Otanuomi = 30000192, Sin = 22430. Numeric forms resolve identically.
    let (status, body) = post_range(
        fixture_state(),
        serde_json::json!({ "from": 30000192_i64, "ship": 22430_i64, "jdc_level": 5 }),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["hull"]["name"], "Sin");
    assert_eq!(body["source"]["system"], "Otanuomi");
}

async fn post_range(state: AppState, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    let app = build_router(state);
    let req = Request::builder()
        .uri("/api/v1/route/range")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

async fn post_blops(state: AppState, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    let app = build_router(state);
    let req = Request::builder()
        .uri("/api/v1/route/blops")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}
