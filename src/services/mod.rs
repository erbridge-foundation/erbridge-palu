//! Service layer: business logic that composes the graph, overlay, and
//! routing. Services never import `axum` — they return domain results and
//! `AppError`, and the handler layer maps those to HTTP.

pub mod blops;
pub mod route;
