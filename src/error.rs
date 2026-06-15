//! Single application error type and its HTTP mapping. Services return
//! `AppError`; the `IntoResponse` impl owns status codes and the JSON body so
//! handlers never construct `StatusCode` directly.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// A `from`/`to`/`avoid`/connection system could not be resolved.
    #[error("system not found: {0}")]
    UnknownSystem(String),
    /// A `ship` hull (name or typeID) is not in the catalog.
    #[error("hull not found: {0}")]
    UnknownHull(String),
    /// A request parameter is present but out of its allowed range.
    #[error("{0}")]
    InvalidParam(String),
    /// No route exists under the given overlay and preference.
    #[error("no route between the requested systems")]
    Unreachable,
    /// The cyno target B is highsec, where a cyno cannot be lit. Distinct from
    /// the other staging failures: the *target* is wrong, not the ship or map.
    #[error("a cyno cannot be lit in highsec: {0}")]
    CynoTargetHighsec(String),
    /// No K-space system lies within the effective bridge range of B (the ship
    /// is too short-range, or B is too remote). Distinct from `Unreachable`.
    #[error("no staging system within bridge range of the target")]
    NoStagingInRange,
    /// In-range staging systems exist but none is gate-reachable from A under
    /// the request's overlay (the fleet is boxed in).
    #[error("no in-range staging system is gate-reachable from the fleet")]
    StagingUnreachable,
}

impl AppError {
    fn parts(&self) -> (StatusCode, &'static str, String) {
        match self {
            AppError::UnknownSystem(s) => (
                StatusCode::BAD_REQUEST,
                "unknown_system",
                format!("system not found: {s}"),
            ),
            AppError::UnknownHull(s) => (
                StatusCode::BAD_REQUEST,
                "unknown_hull",
                format!("hull not found: {s}"),
            ),
            AppError::InvalidParam(s) => (StatusCode::BAD_REQUEST, "invalid_param", s.clone()),
            AppError::Unreachable => (
                StatusCode::NOT_FOUND,
                "unreachable",
                "no gate route between the requested systems".to_string(),
            ),
            AppError::CynoTargetHighsec(s) => (
                StatusCode::BAD_REQUEST,
                "cyno_target_highsec",
                format!("a cyno cannot be lit in highsec: {s}"),
            ),
            AppError::NoStagingInRange => (
                StatusCode::NOT_FOUND,
                "no_staging_in_range",
                "no staging system within bridge range of the target".to_string(),
            ),
            AppError::StagingUnreachable => (
                StatusCode::NOT_FOUND,
                "staging_unreachable",
                "no in-range staging system is gate-reachable from the fleet".to_string(),
            ),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = self.parts();
        let body = serde_json::json!({ "error": code, "message": message });
        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn body_json(resp: Response) -> (StatusCode, serde_json::Value) {
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn unknown_system_maps_to_400() {
        let resp = AppError::UnknownSystem("Nowhere".into()).into_response();
        let (status, body) = body_json(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "unknown_system");
        assert!(body["message"].as_str().unwrap().contains("Nowhere"));
    }

    #[tokio::test]
    async fn unreachable_maps_to_404() {
        let resp = AppError::Unreachable.into_response();
        let (status, body) = body_json(resp).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "unreachable");
    }

    #[tokio::test]
    async fn unknown_hull_maps_to_400() {
        let resp = AppError::UnknownHull("Frigate".into()).into_response();
        let (status, body) = body_json(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "unknown_hull");
        assert!(body["message"].as_str().unwrap().contains("Frigate"));
    }

    #[tokio::test]
    async fn invalid_param_maps_to_400() {
        let resp = AppError::InvalidParam("jdc_level must be 0..=5".into()).into_response();
        let (status, body) = body_json(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_param");
        assert!(body["message"].as_str().unwrap().contains("jdc_level"));
    }

    #[tokio::test]
    async fn cyno_target_highsec_maps_to_400_distinctly() {
        let resp = AppError::CynoTargetHighsec("Jita".into()).into_response();
        let (status, body) = body_json(resp).await;
        // A wrong target, not an unreachable map: a 4xx with its own code.
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "cyno_target_highsec");
        assert!(body["message"].as_str().unwrap().contains("Jita"));
    }

    #[tokio::test]
    async fn no_staging_in_range_maps_to_404_distinctly() {
        let resp = AppError::NoStagingInRange.into_response();
        let (status, body) = body_json(resp).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "no_staging_in_range");
    }

    #[tokio::test]
    async fn staging_unreachable_maps_to_404_distinctly() {
        let resp = AppError::StagingUnreachable.into_response();
        let (status, body) = body_json(resp).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "staging_unreachable");
    }
}
