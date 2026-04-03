//! `GET /health` — liveness probe endpoint.
//!
//! Returns a minimal JSON body `{"status": "ok", "service": "hts"}` with HTTP
//! 200.  No authentication or backend access is required so the endpoint can be
//! used as a Kubernetes liveness/readiness probe without side effects.

use axum::{Json, http::StatusCode, response::IntoResponse};
use serde_json::json;

/// `GET /health` — returns service health status.
///
/// Always returns HTTP 200 with `{"status": "ok", "service": "hts"}` as long
/// as the server process is running.  Does **not** check database connectivity;
/// use [`GET /metadata`] for a deeper capability check.
///
/// [`GET /metadata`]: super::metadata::metadata_handler
pub async fn health_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "service": "hts"
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::Request, routing::get};
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_health_returns_200() {
        let app = Router::new().route("/health", get(health_handler));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_health_response_body() {
        let app = Router::new().route("/health", get(health_handler));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

        assert_eq!(body["status"], "ok");
        assert_eq!(body["service"], "hts");
    }
}
