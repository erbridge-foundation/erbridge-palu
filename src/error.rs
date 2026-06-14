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
    /// No route exists under the given overlay and preference.
    #[error("no route between the requested systems")]
    Unreachable,
}

impl AppError {
    fn parts(&self) -> (StatusCode, &'static str, String) {
        match self {
            AppError::UnknownSystem(s) => (
                StatusCode::BAD_REQUEST,
                "unknown_system",
                format!("system not found: {s}"),
            ),
            AppError::Unreachable => (
                StatusCode::NOT_FOUND,
                "unreachable",
                "no gate route between the requested systems".to_string(),
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
}
