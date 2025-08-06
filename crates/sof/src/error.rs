//! Error handling for the SQL-on-FHIR server
//!
//! This module provides error types and conversion utilities for handling
//! various error conditions in the server, including proper FHIR OperationOutcome
//! generation for error responses.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use helios_sof::SofError;
use std::fmt;

/// Server-specific error type that can be converted to HTTP responses
#[derive(Debug)]
#[allow(dead_code)] // Some variants are reserved for future use
pub enum ServerError {
    /// Invalid request parameters or body
    BadRequest(String),

    /// Requested resource not found
    NotFound(String),

    /// Unsupported media type or format
    UnsupportedMediaType(String),

    /// Internal processing error from SOF engine
    ProcessingError(SofError),

    /// JSON parsing error
    JsonError(serde_json::Error),

    /// Generic internal server error
    InternalError(String),

    /// Feature not implemented
    NotImplemented(String),
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServerError::BadRequest(msg) => write!(f, "Bad request: {}", msg),
            ServerError::NotFound(msg) => write!(f, "Not found: {}", msg),
            ServerError::UnsupportedMediaType(msg) => write!(f, "Unsupported media type: {}", msg),
            ServerError::ProcessingError(err) => write!(f, "Processing error: {}", err),
            ServerError::JsonError(err) => write!(f, "JSON error: {}", err),
            ServerError::InternalError(msg) => write!(f, "Internal server error: {}", msg),
            ServerError::NotImplemented(msg) => write!(f, "Not implemented: {}", msg),
        }
    }
}

impl std::error::Error for ServerError {}

impl From<SofError> for ServerError {
    fn from(err: SofError) -> Self {
        match &err {
            SofError::UnsupportedContentType(_) => {
                ServerError::UnsupportedMediaType(err.to_string())
            }
            _ => ServerError::ProcessingError(err),
        }
    }
}

impl From<serde_json::Error> for ServerError {
    fn from(err: serde_json::Error) -> Self {
        ServerError::JsonError(err)
    }
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let (status, error_code, details) = match &self {
            ServerError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "invalid", msg.clone()),
            ServerError::NotFound(msg) => (StatusCode::NOT_FOUND, "not-found", msg.clone()),
            ServerError::UnsupportedMediaType(msg) => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "not-supported",
                msg.clone(),
            ),
            ServerError::ProcessingError(err) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "processing",
                err.to_string(),
            ),
            ServerError::JsonError(err) => (
                StatusCode::BAD_REQUEST,
                "invalid",
                format!("Invalid JSON: {}", err),
            ),
            ServerError::InternalError(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "exception", msg.clone())
            }
            ServerError::NotImplemented(msg) => {
                (StatusCode::NOT_IMPLEMENTED, "not-supported", msg.clone())
            }
        };

        // Create FHIR OperationOutcome
        let operation_outcome = create_operation_outcome(error_code, &details);

        (status, Json(operation_outcome)).into_response()
    }
}

/// Create a FHIR R4 OperationOutcome for error responses
fn create_operation_outcome(code: &str, details: &str) -> serde_json::Value {
    serde_json::json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": "error",
            "code": code,
            "details": {
                "text": details
            }
        }]
    })
}

/// Result type alias for server operations
pub type ServerResult<T> = Result<T, ServerError>;
