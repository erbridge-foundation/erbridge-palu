//! `POST /api/v1/route/system` handler.

use axum::{Json, extract::State};

use crate::app_state::AppState;
use crate::dto::{GateRouteRequest, GateRouteResponse};
use crate::error::AppError;
use crate::services::route::compute_gate_route;

/// Compute a system-to-system route over the SDE gate graph plus the
/// per-request wormhole overlay.
#[utoipa::path(
    post,
    path = "/api/v1/route/system",
    request_body = GateRouteRequest,
    responses(
        (status = 200, description = "Route found", body = GateRouteResponse),
        (status = 400, description = "Unknown system in the request"),
        (status = 404, description = "No route exists under the given overlay"),
    ),
    tag = "routing",
)]
pub async fn route_system(
    State(state): State<AppState>,
    Json(req): Json<GateRouteRequest>,
) -> Result<Json<GateRouteResponse>, AppError> {
    // Load both snapshots once so the whole computation sees a consistent view.
    let graph = state.graph.load();
    let scout = state.eve_scout.load();
    let resp = compute_gate_route(&graph, &scout, &req)?;
    Ok(Json(resp))
}
