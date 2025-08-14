//! Error types for FHIRPath CLI and server operations
//!
//! This module provides error types for the FHIRPath executables,
//! supporting both CLI and server error scenarios with appropriate
//! error messages and HTTP status codes.

use std::fmt;

/// Result type alias for FHIRPath operations
pub type FhirPathResult<T> = Result<T, FhirPathError>;

/// Error types for FHIRPath operations
#[derive(Debug)]
pub enum FhirPathError {
    /// Parse error with message
    ParseError(String),

    /// Evaluation error with message
    EvaluationError(String),

    /// IO error (file operations, etc.)
    IoError(std::io::Error),

    /// JSON serialization/deserialization error
    JsonError(serde_json::Error),

    /// Invalid input parameters
    InvalidInput(String),

    /// Resource not found
    NotFound(String),

    /// Feature not implemented
    NotImplemented(String),

    /// Server configuration error
    ConfigError(String),

    /// HTTP-specific error with status code
    HttpError(u16, String),
}

impl fmt::Display for FhirPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FhirPathError::ParseError(msg) => write!(f, "Parse error: {}", msg),
            FhirPathError::EvaluationError(msg) => write!(f, "Evaluation error: {}", msg),
            FhirPathError::IoError(err) => write!(f, "IO error: {}", err),
            FhirPathError::JsonError(err) => write!(f, "JSON error: {}", err),
            FhirPathError::InvalidInput(msg) => write!(f, "Invalid input: {}", msg),
            FhirPathError::NotFound(msg) => write!(f, "Not found: {}", msg),
            FhirPathError::NotImplemented(msg) => write!(f, "Not implemented: {}", msg),
            FhirPathError::ConfigError(msg) => write!(f, "Configuration error: {}", msg),
            FhirPathError::HttpError(code, msg) => write!(f, "HTTP {} error: {}", code, msg),
        }
    }
}

impl std::error::Error for FhirPathError {}

impl From<std::io::Error> for FhirPathError {
    fn from(err: std::io::Error) -> Self {
        FhirPathError::IoError(err)
    }
}

impl From<serde_json::Error> for FhirPathError {
    fn from(err: serde_json::Error) -> Self {
        FhirPathError::JsonError(err)
    }
}

impl From<String> for FhirPathError {
    fn from(err: String) -> Self {
        FhirPathError::InvalidInput(err)
    }
}

impl axum::response::IntoResponse for FhirPathError {
    fn into_response(self) -> axum::response::Response {
        self.into()
    }
}

impl From<FhirPathError> for axum::response::Response {
    fn from(err: FhirPathError) -> Self {
        use axum::Json;
        use axum::http::StatusCode;
        use axum::response::IntoResponse;

        let (status, message) = match err {
            FhirPathError::ParseError(msg) => (StatusCode::BAD_REQUEST, msg),
            FhirPathError::EvaluationError(msg) => (StatusCode::UNPROCESSABLE_ENTITY, msg),
            FhirPathError::InvalidInput(msg) => (StatusCode::BAD_REQUEST, msg),
            FhirPathError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            FhirPathError::NotImplemented(msg) => (StatusCode::NOT_IMPLEMENTED, msg),
            FhirPathError::ConfigError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
            FhirPathError::HttpError(code, msg) => (
                StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                msg,
            ),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
        };

        // Create FHIR OperationOutcome
        let operation_outcome = serde_json::json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": match status {
                    StatusCode::BAD_REQUEST => "invalid",
                    StatusCode::NOT_FOUND => "not-found",
                    StatusCode::UNPROCESSABLE_ENTITY => "processing",
                    StatusCode::NOT_IMPLEMENTED => "not-supported",
                    _ => "exception",
                },
                "diagnostics": message
            }]
        });

        (status, Json(operation_outcome)).into_response()
    }
}
