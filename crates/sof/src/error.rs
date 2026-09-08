//! Error handling for the SQL-on-FHIR server
//!
//! This module provides error types and conversion utilities for handling
//! various error conditions in the server, including proper FHIR OperationOutcome
//! generation for error responses.

use crate::SofError;
use crate::lint::{Diagnostic, lint_operation_outcome};
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::fmt;

/// Server-specific error type that can be converted to HTTP responses
#[derive(Debug)]
#[allow(dead_code)] // Some variants are reserved for future use
pub enum ServerError {
    /// Invalid request parameters or body
    BadRequest(String),

    /// A reference supplied in a request (e.g. `patient` / `group` on
    /// `$viewdefinition-run`) does not resolve. Surfaces as `400 Bad Request`
    /// + `OperationOutcome.issue.code = not-found`, per the SoF v2 spec's
    /// error table.
    ReferencedResourceNotFound(String),

    /// Requested resource not found
    NotFound(String),

    /// Unsupported media type or format
    UnsupportedMediaType(String),

    /// The representation requested via `Accept` cannot be produced
    /// (e.g. the `application/fhir+xml` `Binary` envelope form). Surfaces
    /// as `406 Not Acceptable` + `OperationOutcome` per the SoF v2 spec's
    /// content-negotiation rules.
    NotAcceptable(String),

    /// Internal processing error from SOF engine
    ProcessingError(SofError),

    /// An inline ViewDefinition failed `helios_sof::lint` (#821): every
    /// element is an error-severity [`Diagnostic`] `lint_view_definition`
    /// reported against the document exactly as the client sent it, before
    /// any typed parse. Surfaces as `422 Unprocessable Entity` with one
    /// `OperationOutcome.issue` per diagnostic — see
    /// [`invalid_view_definition_response`] for the exact shape.
    InvalidViewDefinition(Vec<Diagnostic>),

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
            ServerError::ReferencedResourceNotFound(msg) => {
                write!(f, "Referenced resource not found: {}", msg)
            }
            ServerError::NotFound(msg) => write!(f, "Not found: {}", msg),
            ServerError::UnsupportedMediaType(msg) => write!(f, "Unsupported media type: {}", msg),
            ServerError::NotAcceptable(msg) => write!(f, "Not acceptable: {}", msg),
            ServerError::ProcessingError(err) => write!(f, "Processing error: {}", err),
            ServerError::InvalidViewDefinition(diagnostics) => write!(
                f,
                "Invalid ViewDefinition: {} lint error(s)",
                diagnostics.len()
            ),
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
            // Spec (operations-common, Output Formats): an unsupported
            // `_format` value SHALL be rejected with 400 Bad Request +
            // OperationOutcome — 415 is reserved for transport-level
            // Content-Type/Content-Encoding problems.
            SofError::UnsupportedContentType(_) => ServerError::BadRequest(err.to_string()),
            SofError::InvalidSource(_)
            | SofError::SourceNotFound(_)
            | SofError::UnsupportedSourceProtocol(_) => ServerError::BadRequest(err.to_string()),
            SofError::ReferencedResourceNotFound(_) => {
                ServerError::ReferencedResourceNotFound(err.to_string())
            }
            SofError::SourceFetchError(_)
            | SofError::SourceReadError(_)
            | SofError::InvalidSourceContent(_) => ServerError::ProcessingError(err),
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
        // Multi-issue shape: every other variant below reduces to one
        // `OperationOutcome.issue`, but a lint failure reports one issue per
        // diagnostic, so it can't share `create_operation_outcome`'s single-issue
        // builder.
        if let ServerError::InvalidViewDefinition(diagnostics) = &self {
            return invalid_view_definition_response(diagnostics);
        }

        let (status, error_code, details) = match &self {
            ServerError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "invalid", msg.clone()),
            ServerError::ReferencedResourceNotFound(msg) => {
                (StatusCode::BAD_REQUEST, "not-found", msg.clone())
            }
            ServerError::NotFound(msg) => (StatusCode::NOT_FOUND, "not-found", msg.clone()),
            ServerError::UnsupportedMediaType(msg) => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "not-supported",
                msg.clone(),
            ),
            ServerError::NotAcceptable(msg) => {
                (StatusCode::NOT_ACCEPTABLE, "not-supported", msg.clone())
            }
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
            // Handled by the early return above; kept as an explicit arm
            // (rather than `_`) so a future variant added to this enum
            // fails to compile here instead of silently falling through.
            ServerError::InvalidViewDefinition(_) => unreachable!(
                "InvalidViewDefinition returns via invalid_view_definition_response above"
            ),
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

/// Builds the `422 Unprocessable Entity` response for
/// [`ServerError::InvalidViewDefinition`] (#821): one `OperationOutcome.issue`
/// per lint diagnostic, via [`lint_operation_outcome`] — the same builder
/// HFS's own `$sql-run` handler uses, so both servers render an identical
/// body for an identical set of diagnostics.
fn invalid_view_definition_response(diagnostics: &[Diagnostic]) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(lint_operation_outcome(diagnostics)),
    )
        .into_response()
}

/// Result type alias for server operations
pub type ServerResult<T> = Result<T, ServerError>;
