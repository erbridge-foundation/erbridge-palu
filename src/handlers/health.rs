//! `GET /health` handler. No auth, no envelope — a flat status document.

use axum::{Json, extract::State};

use crate::app_state::AppState;
use crate::dto::HealthResponse;

/// Service health: loaded SDE build, graph size, last reload swap, and
/// EVE-Scout freshness.
#[utoipa::path(
    get,
    path = "/health",
    responses((status = 200, description = "Service health", body = HealthResponse)),
    tag = "health",
)]
pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let graph = state.graph.load();
    let last_sde_reload_at = state.last_reload_at.load_full().map(|arc| *arc);
    let scout = state.eve_scout.load();

    Json(HealthResponse {
        status: "ok".to_string(),
        sde_version: graph.build_number,
        systems: graph.systems.len(),
        edges: graph.gate_graph.edge_count(),
        last_sde_reload_at,
        sig_count: scout.sig_count(),
        last_evescout_fetch_at: scout.fetched_at,
    })
}
