//! HTTP handler layer. Handlers load per-request snapshots from `AppState`,
//! delegate to the service layer, and return DTOs. They never contain routing
//! logic and never touch the SDE/graph internals directly.

pub mod health;
pub mod route;
