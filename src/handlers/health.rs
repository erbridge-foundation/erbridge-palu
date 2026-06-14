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
        hull_count: graph.hulls.len(),
        last_sde_reload_at,
        sig_count: scout.sig_count(),
        last_evescout_fetch_at: scout.fetched_at,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::graph::build_graph_data;
    use crate::model::{RawHull, RawHullCatalog, RawSdeData};

    #[tokio::test]
    async fn health_reports_loaded_hull_count() {
        let raw = RawSdeData {
            systems: vec![],
            gate_pairs: vec![],
            hulls: RawHullCatalog {
                hulls: vec![RawHull {
                    type_id: 22430,
                    name: "Sin".into(),
                    group_id: 898,
                    base_ly: 4.0,
                }],
                jdc_bonus_per_level: Some(0.20),
            },
        };
        let state = AppState::new(Arc::new(build_graph_data(raw, 7)));
        let Json(body) = health(State(state)).await;
        assert_eq!(body.status, "ok");
        assert_eq!(body.sde_version, 7);
        assert_eq!(body.hull_count, 1);
    }
}
