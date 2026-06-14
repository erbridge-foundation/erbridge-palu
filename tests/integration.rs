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

use erbridge_geodesic::app_state::AppState;
use erbridge_geodesic::build_router;
use erbridge_geodesic::eve_scout::{EveScoutSnapshot, Signature, THERA_SYSTEM_ID};
use erbridge_geodesic::graph::build_graph_data;
use erbridge_geodesic::sde::cache::SdeCache;

fn fixture_state() -> AppState {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sde");
    let cache = SdeCache::new(dir);
    let meta = cache.current_metadata().expect("fixture metadata present");
    let raw = cache.load_build(meta.build_number).expect("fixture loads");
    let graph = build_graph_data(raw, meta.build_number);
    AppState::new(Arc::new(graph))
}

async fn post_route(state: AppState, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    let app = build_router(state);
    let req = Request::builder()
        .uri("/route/gate")
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
async fn unreachable_is_404() {
    // J105443 is a gateless wormhole system with no overlay edge → unreachable.
    let (status, body) = post_route(
        fixture_state(),
        serde_json::json!({ "from": "Jita", "to": "J105443" }),
    )
    .await;
    assert_eq!(status, 404, "body={body}");
    assert_eq!(body["error"], "unreachable");
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
    // Build number is the fixture's cached SDE build (whatever metadata.json says).
    assert!(body["build_number"].as_u64().unwrap() > 0);
    assert!(body["systems"].as_u64().unwrap() > 0);
    assert!(body["edges"].as_u64().unwrap() > 0);
    // Freshness fields are null/0 before any reload swap or EVE-Scout fetch.
    assert!(body["last_reload_at"].is_null());
    assert_eq!(body["sig_count"], 0);
    assert!(body["last_fetch_at"].is_null());
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
    assert!(doc["paths"]["/route/gate"].is_object());
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
