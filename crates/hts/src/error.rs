use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use thiserror::Error;

/// All errors that the HTS service can produce.
#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum HtsError {
    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("Operation not supported: {0}")]
    NotSupported(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Precondition failed: {0}")]
    PreconditionFailed(String),

    /// The requested expansion would exceed the server's configured size limit.
    #[error("Expansion too costly: {0}")]
    TooCostly(String),
}

impl IntoResponse for HtsError {
    fn into_response(self) -> Response {
        // `TooCostly` has its own tuple so we can't borrow `self` for the
        // diagnostics string in the same `match`; handle it separately.
        if let HtsError::TooCostly(ref msg) = self {
            let body = json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "error",
                    "code": "too-costly",
                    "diagnostics": msg
                }]
            });
            return (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response();
        }

        let (status, code, diagnostics) = match &self {
            HtsError::NotFound(msg) => (StatusCode::NOT_FOUND, "not-found", msg.as_str()),
            HtsError::NotSupported(msg) => {
                (StatusCode::NOT_IMPLEMENTED, "not-supported", msg.as_str())
            }
            HtsError::InvalidRequest(msg) => (StatusCode::BAD_REQUEST, "invalid", msg.as_str()),
            HtsError::Internal(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "exception", msg.as_str())
            }
            HtsError::StorageError(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "exception", msg.as_str())
            }
            HtsError::PreconditionFailed(msg) => {
                (StatusCode::PRECONDITION_FAILED, "conflict", msg.as_str())
            }
            HtsError::TooCostly(_) => unreachable!("handled above"),
        };

        let body = json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": code,
                "diagnostics": diagnostics
            }]
        });

        (status, Json(body)).into_response()
    }
}
