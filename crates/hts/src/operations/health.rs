//! `GET /health` — liveness probe endpoint.
//!
//! Returns a JSON body with status, service name, crate version, active
//! backend, and the current UTC timestamp.  No authentication is required so
//! the endpoint can be used as a Kubernetes liveness/readiness probe.  Does
//! **not** check database connectivity; use [`GET /metadata`] for a deeper
//! capability check.
//!
//! [`GET /metadata`]: super::metadata::metadata_handler

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde_json::json;

use crate::state::AppState;
use crate::traits::TerminologyBackend;

/// `GET /health` — returns service health status.
///
/// Always returns HTTP 200 with a JSON body containing:
/// - `status` — always `"ok"` while the process is running
/// - `service` — `"hts"`
/// - `version` — crate version (`CARGO_PKG_VERSION`)
/// - `backend` — active backend name (e.g. `"sqlite"`, `"postgres"`)
/// - `timestamp` — current UTC time as RFC 3339
pub async fn health_handler<B>(State(state): State<AppState<B>>) -> impl IntoResponse
where
    B: TerminologyBackend + Clone,
{
    (
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "service": "hts",
            "version": env!("CARGO_PKG_VERSION"),
            "backend": state.backend().backend_name(),
            "uptime_seconds": helios_observability::uptime::uptime_seconds(),
            "started_at": helios_observability::uptime::started_at_rfc3339(),
            "timestamp": chrono::Utc::now().to_rfc3339(),
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::sqlite::SqliteTerminologyBackend;
    use axum::{Router, body::Body, http::Request, routing::get};
    use tower::ServiceExt;

    fn make_app() -> Router {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let state = AppState::new(backend);
        Router::new()
            .route("/health", get(health_handler::<SqliteTerminologyBackend>))
            .with_state(state)
    }

    #[tokio::test]
    async fn test_health_returns_200() {
        let response = make_app()
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
        let response = make_app()
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
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(body["backend"], "sqlite");
        let ts = body["timestamp"]
            .as_str()
            .expect("timestamp must be string");
        chrono::DateTime::parse_from_rfc3339(ts).expect("timestamp must be valid RFC 3339");
    }
}
