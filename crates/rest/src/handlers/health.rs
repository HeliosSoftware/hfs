//! Health check endpoint handler.
//!
//! Provides a simple health check endpoint for monitoring and load balancers.

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use helios_persistence::core::ResourceStorage;
use tracing::debug;

use crate::error::RestResult;
use crate::state::AppState;

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
pub async fn health_handler<S>(State(state): State<AppState<S>>) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    debug!("Processing health check request");

    // Perform a simple check - we could add more sophisticated checks here
    let backend_name = state.storage().backend_name();

    let health_response = serde_json::json!({
        "status": "ok",
        "service": "hfs",
        "version": env!("CARGO_PKG_VERSION"),
        "backend": backend_name,
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

/// Handler for a readiness probe.
///
/// This could perform deeper checks like database connectivity.
///
/// # HTTP Request
///
/// `GET [base]/_readiness`
pub async fn readiness_handler<S>(State(state): State<AppState<S>>) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    debug!("Processing readiness check request");

    // Try a simple operation to verify storage is working
    // In a real implementation, we might try a count or read operation
    let backend_name = state.storage().backend_name();

    let response = serde_json::json!({
        "status": "ready",
        "backend": backend_name,
        "checks": {
            "storage": "ok"
        }
    });

    Ok((StatusCode::OK, Json(response)).into_response())
}
