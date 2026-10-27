//! Health check endpoint handler.
//!
//! Provides a simple health check endpoint for monitoring and load balancers.

use std::time::Duration;

use axum::{
    Json,
    extract::State,
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use helios_persistence::core::ResourceStorage;
use tracing::{debug, warn};

use crate::error::{RestResult, SERVICE_UNAVAILABLE_RETRY_AFTER_SECS};
use crate::state::AppState;

/// Maximum time the readiness probe waits for the storage check before
/// declaring the backend not-ready. Kept below a typical orchestrator probe
/// `timeoutSeconds` (often 1s) so a hung backend yields a clean 503 rather than
/// a probe timeout, and so probe traffic cannot pile up on a stalled backend.
const READINESS_PROBE_TIMEOUT: Duration = Duration::from_millis(800);

/// Handler for the health check endpoint.
///
/// Returns a simple health status, useful for load balancers and
/// monitoring systems.
///
/// # HTTP Request
///
/// `GET [base]/health`
///
/// # Response
///
/// - `200 OK` - Server is healthy
/// - `503 Service Unavailable` - Server is unhealthy
pub async fn health_handler<S>(State(_state): State<AppState<S>>) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    debug!("Processing health check request");

    let health_response = serde_json::json!({
        "status": "ok",
        "service": "hfs",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": helios_observability::uptime::uptime_seconds(),
        "started_at": helios_observability::uptime::started_at_rfc3339(),
        "timestamp": chrono::Utc::now().to_rfc3339()
    });

    Ok((StatusCode::OK, Json(health_response)).into_response())
}

/// Handler for a more detailed liveness probe.
///
/// This could be used by Kubernetes liveness probes.
///
/// # HTTP Request
///
/// `GET [base]/_liveness`
pub async fn liveness_handler() -> impl IntoResponse {
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use async_trait::async_trait;
    use helios_fhir::FhirVersion;
    use helios_persistence::error::{BackendError, StorageResult};
    use helios_persistence::tenant::TenantContext;
    use helios_persistence::types::StoredResource;
    use serde_json::Value;

    use crate::config::ServerConfig;

    /// Minimal `ResourceStorage` whose `readiness_check` behaviour is
    /// configurable. Every other method is unused by the health handlers.
    /// Mirrors the required (non-defaulted) trait surface only.
    struct ProbeStorage {
        ready: bool,
        /// When set, `readiness_check` sleeps this long before returning, so a
        /// test can drive the probe past [`READINESS_PROBE_TIMEOUT`] and
        /// exercise the timeout arm. With a paused clock the sleep resolves
        /// instantly against the timeout's earlier deadline.
        delay: Option<Duration>,
    }

    #[async_trait]
    impl ResourceStorage for ProbeStorage {
        fn backend_name(&self) -> &'static str {
            "probe-mock"
        }

        async fn readiness_check(&self) -> Result<(), BackendError> {
            if let Some(delay) = self.delay {
                tokio::time::sleep(delay).await;
            }
            if self.ready {
                Ok(())
            } else {
                Err(BackendError::Unavailable {
                    backend_name: "probe-mock".to_string(),
                    message: "simulated outage".to_string(),
                })
            }
        }

        async fn create(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<StoredResource> {
            unimplemented!()
        }

        async fn create_or_update(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<(StoredResource, bool)> {
            unimplemented!()
        }

        async fn read(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<Option<StoredResource>> {
            unimplemented!()
        }

        async fn update(
            &self,
            _tenant: &TenantContext,
            _current: &StoredResource,
            _resource: Value,
        ) -> StorageResult<StoredResource> {
            unimplemented!()
        }

        async fn delete(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<()> {
            unimplemented!()
        }

        async fn count(
            &self,
            _tenant: &TenantContext,
            _resource_type: Option<&str>,
        ) -> StorageResult<u64> {
            unimplemented!()
        }
    }

    fn state_with(ready: bool) -> AppState<ProbeStorage> {
        AppState::new(
            Arc::new(ProbeStorage { ready, delay: None }),
            ServerConfig::default(),
        )
    }

    /// State whose storage probe never answers within [`READINESS_PROBE_TIMEOUT`].
    fn state_that_hangs() -> AppState<ProbeStorage> {
        AppState::new(
            Arc::new(ProbeStorage {
                ready: true,
                delay: Some(READINESS_PROBE_TIMEOUT * 4),
            }),
            ServerConfig::default(),
        )
    }

    async fn body_json(response: Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn readiness_returns_200_when_storage_healthy() {
        let response = readiness_handler(State(state_with(true))).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CACHE_CONTROL)
                .unwrap(),
            "no-store"
        );
        let json = body_json(response).await;
        assert_eq!(json["status"], "ready");
        assert_eq!(json["checks"]["storage"], "ok");
    }

    #[tokio::test]
    async fn readiness_returns_503_and_retry_after_when_storage_unreachable() {
        let response = readiness_handler(State(state_with(false))).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            response
                .headers()
                .get(axum::http::header::RETRY_AFTER)
                .is_some(),
            "readiness 503 must carry Retry-After"
        );
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CACHE_CONTROL)
                .unwrap(),
            "no-store"
        );
        let json = body_json(response).await;
        assert_eq!(json["status"], "not_ready");
        assert_eq!(json["checks"]["storage"], "unavailable");
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_returns_503_and_retry_after_when_storage_times_out() {
        // A backend that never answers must be treated exactly like an
        // unreachable one — a hung probe cannot be allowed to keep the pod in
        // rotation. `start_paused` auto-advances the clock to the timeout's
        // deadline, so this asserts the timeout arm without waiting in real time.
        let response = readiness_handler(State(state_that_hangs())).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            response
                .headers()
                .get(axum::http::header::RETRY_AFTER)
                .is_some(),
            "a timed-out readiness probe must still carry Retry-After"
        );
        let json = body_json(response).await;
        assert_eq!(json["status"], "not_ready");
        assert_eq!(json["checks"]["storage"], "unavailable");
    }

    #[tokio::test]
    async fn probe_storage_reports_its_backend_name() {
        // Guards the trait's `backend_name` accessor on the probe double so the
        // handler's `state.storage()` surface stays exercised.
        assert_eq!(state_with(true).storage().backend_name(), "probe-mock");
    }

    #[tokio::test]
    async fn liveness_returns_200_regardless_of_storage() {
        // Liveness takes no state and must never depend on the backend, so a
        // dependency outage does not trigger pod restarts.
        let response = liveness_handler().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
    }
}

/// Handler for a readiness probe.
///
/// Unlike liveness, readiness answers "should this instance receive traffic
/// right now?" It actively probes the storage backend (a cheap, timeout-bounded
/// [`ResourceStorage::readiness_check`]) and returns:
///
/// - `200 OK` with `{"status":"ready","checks":{"storage":"ok"}}` when storage
///   is reachable, or
/// - `503 Service Unavailable` with `{"status":"not_ready","checks":{"storage":
///   "unavailable"}}` (plus `Retry-After`) when the backend is unreachable or
///   does not answer within [`READINESS_PROBE_TIMEOUT`].
///
/// The response is always marked `Cache-Control: no-store` — a stale "ready"
/// during an outage would keep an orchestrator routing traffic to a pod that
/// cannot serve it. Liveness deliberately stays independent of storage so a
/// dependency outage does not trigger pod restarts (issue #286).
///
/// # HTTP Request
///
/// `GET [base]/_readiness`
pub async fn readiness_handler<S>(State(state): State<AppState<S>>) -> Response
where
    S: ResourceStorage + Send + Sync,
{
    debug!("Processing readiness check request");

    let (status, storage_status, body_status) = match tokio::time::timeout(
        READINESS_PROBE_TIMEOUT,
        state.storage().readiness_check(),
    )
    .await
    {
        Ok(Ok(())) => (StatusCode::OK, "ok", "ready"),
        Ok(Err(err)) => {
            warn!(error = %err, "readiness check failed: storage backend unavailable");
            (StatusCode::SERVICE_UNAVAILABLE, "unavailable", "not_ready")
        }
        Err(_elapsed) => {
            warn!(
                timeout_ms = READINESS_PROBE_TIMEOUT.as_millis() as u64,
                "readiness check timed out: storage backend unresponsive"
            );
            (StatusCode::SERVICE_UNAVAILABLE, "unavailable", "not_ready")
        }
    };

    let body = serde_json::json!({
        "status": body_status,
        "checks": { "storage": storage_status },
    });

    let mut response = (status, Json(body)).into_response();
    let headers = response.headers_mut();
    // Probe results must never be cached — a stale "ready" during an outage is
    // dangerous.
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    if status == StatusCode::SERVICE_UNAVAILABLE {
        headers.insert(
            axum::http::header::RETRY_AFTER,
            HeaderValue::from_static(SERVICE_UNAVAILABLE_RETRY_AFTER_SECS),
        );
    }
    response
}
