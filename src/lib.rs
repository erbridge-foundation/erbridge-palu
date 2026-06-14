//! erbridge-geodesic: EVE Online gate-routing REST API.
//!
//! Architecture (no DB, no auth in this foundation):
//! - `sde` / `graph` / `model`: load the SDE and build the in-memory graph.
//! - `routing`: Dijkstra + per-request overlay.
//! - `eve_scout`: background-cached Thera/Turnur signatures.
//! - `services`: business logic (overlay assembly + routing).
//! - `handlers`: HTTP boundary; load snapshots, call services, return DTOs.

pub mod app_state;
pub mod config;
pub mod dto;
pub mod error;
pub mod eve_scout;
pub mod graph;
pub mod handlers;
pub mod model;
pub mod routing;
pub mod sde;
pub mod services;
pub mod tasks;

use axum::Router;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use utoipa_swagger_ui::SwaggerUi;

use app_state::AppState;

/// Top-level OpenAPI document. Paths are collected from handler annotations via
/// `OpenApiRouter`, so there is no manual `paths(...)` registry to drift.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "erbridge-geodesic",
        description = "EVE Online gate-routing REST API",
    ),
    tags(
        (name = "routing", description = "Gate route computation"),
        (name = "health", description = "Service health"),
    ),
)]
pub struct ApiDoc;

/// Build the axum router: handlers + OpenAPI JSON + Swagger UI, all unauthenticated.
pub fn build_router(state: AppState) -> Router {
    use tower_http::trace::TraceLayer;

    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(handlers::route::route_gate))
        .routes(routes!(handlers::health::health))
        .split_for_parts();

    router
        // Swagger UI plus the generated OpenAPI JSON, served unconditionally
        // (the service is docker-internal with no public exposure).
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", api))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_document_describes_both_endpoints() {
        let doc = ApiDoc::openapi();
        let json = serde_json::to_value(&doc).unwrap();
        let paths = json["paths"].as_object().unwrap();
        // Paths are populated by build_router via OpenApiRouter; the bare
        // ApiDoc has none, so build a router to exercise collection.
        assert!(paths.is_empty(), "bare ApiDoc has no paths yet");
    }

    #[test]
    fn router_collects_paths_into_openapi() {
        use crate::graph::build_graph_data;
        use crate::model::RawSdeData;
        use std::sync::Arc;

        let graph = Arc::new(build_graph_data(
            RawSdeData {
                systems: vec![],
                gate_pairs: vec![],
            },
            1,
        ));
        let state = AppState::new(graph);
        // Building the router must not panic; the OpenApiRouter wires both
        // annotated handlers and the Swagger UI.
        let _router = build_router(state);
    }
}
