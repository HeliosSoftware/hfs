//! Error types for the Helios Terminology Server.
//!
//! All fallible operations in HTS return [`HtsError`].  The [`IntoResponse`]
//! implementation converts each variant to the appropriate HTTP status code and
//! a FHIR `OperationOutcome` response body, so handlers can propagate errors
//! with the `?` operator without any extra mapping.
//!
//! ## HTTP mapping
//!
//! | Variant | HTTP status | FHIR issue code | tx-issue-type |
//! |---------|-------------|-----------------|---------------|
//! | [`NotFound`] | 404 | `not-found` | `not-found` |
//! | [`NotSupported`] | 501 | `not-supported` | `not-supported` |
//! | [`InvalidRequest`] | 400 | `invalid` | `invalid` |
//! | [`VsInvalid`] | 400 | `invalid` | `vs-invalid` |
//! | [`Internal`] | 500 | `exception` | `exception` |
//! | [`StorageError`] | 500 | `exception` | `exception` |
//! | [`PreconditionFailed`] | 412 | `conflict` | `conflict` |
//! | [`TooCostly`] | 422 | `too-costly` | `too-costly` |
//!
//! [`NotFound`]: HtsError::NotFound
//! [`NotSupported`]: HtsError::NotSupported
//! [`InvalidRequest`]: HtsError::InvalidRequest
//! [`VsInvalid`]: HtsError::VsInvalid
//! [`Internal`]: HtsError::Internal
//! [`StorageError`]: HtsError::StorageError
//! [`PreconditionFailed`]: HtsError::PreconditionFailed
//! [`TooCostly`]: HtsError::TooCostly

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use thiserror::Error;

/// All errors that the HTS service can produce.
///
/// Each variant carries a human-readable diagnostic message that is embedded
/// verbatim in the `OperationOutcome.issue[0].diagnostics` field of the HTTP
/// response.  See the module-level documentation for the HTTP status code each
/// variant maps to.
#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum HtsError {
    /// A requested resource (CodeSystem, ValueSet, ConceptMap, or concept) does
    /// not exist in the store.  Maps to HTTP 404.
    #[error("Resource not found: {0}")]
    NotFound(String),

    /// The operation or parameter is valid FHIR but is not implemented by this
    /// server (e.g., SNOMED post-coordination expressions).  Maps to HTTP 501.
    #[error("Operation not supported: {0}")]
    NotSupported(String),

    /// The request body or parameters are malformed, missing required fields,
    /// or violate FHIR operation constraints.  Maps to HTTP 400.
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    /// A ValueSet definition is itself invalid — for example a compose filter
    /// that omits the required `value`, names an unknown operator, or supplies
    /// a regular expression that fails to compile.  Maps to HTTP 400 with FHIR
    /// issue code `invalid` and a `tx-issue-type=vs-invalid` coding so the
    /// HL7 tx-ecosystem fixtures can distinguish ValueSet-definition errors
    /// from other 400-class request problems.
    #[error("Invalid ValueSet: {0}")]
    VsInvalid(String),

    /// An unexpected server-side error that is not attributable to the caller.
    /// Maps to HTTP 500.
    #[error("Internal error: {0}")]
    Internal(String),

    /// A database or connection-pool error.  Maps to HTTP 500.
    #[error("Storage error: {0}")]
    StorageError(String),

    /// An optimistic-concurrency conflict: the `If-Match` version does not
    /// match the stored version.  Maps to HTTP 412.
    #[error("Precondition failed: {0}")]
    PreconditionFailed(String),

    /// The requested expansion would exceed the server's configured size limit
    /// (`HTS_MAX_EXPANSION_SIZE`).  Maps to HTTP 422 with FHIR issue code
    /// `too-costly`, per the FHIR `$expand` specification.
    #[error("Expansion too costly: {0}")]
    TooCostly(String),
}

/// Converts an [`HtsError`] into an Axum HTTP response.
///
/// The response body is always a FHIR `OperationOutcome` JSON object with a
/// single issue entry whose `diagnostics` field carries the error message.
/// [`HtsError::TooCostly`] is handled first because it has a distinct FHIR
/// issue code (`too-costly`) and HTTP status (422) that differ from the
/// generic 500 path.
impl IntoResponse for HtsError {
    fn into_response(self) -> Response {
        // `TooCostly` has its own tuple so we can't borrow `self` for the
        // diagnostics string in the same `match`; handle it separately.
        if let HtsError::TooCostly(ref msg) = self {
            // The IG `big/expand-no-limit-outcome` fixture expects:
            //   - `extension` with `operationoutcome-message-id` =
            //     `VALUESET_TOO_COSTLY` (optional for tx.fhir.org)
            //   - `details.text` only (no `details.coding`)
            //   - `diagnostics` carrying the same message (optional in fixture)
            // Other servers (e.g. tx.fhir.org) don't emit the message-id
            // extension, but the IG marks it `$optional$ : "!tx.fhir.org"` so
            // including it is fine for everyone else.
            let body = json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "extension": [{
                        "url": "http://hl7.org/fhir/StructureDefinition/operationoutcome-message-id",
                        "valueString": "VALUESET_TOO_COSTLY",
                    }],
                    "severity": "error",
                    "code": "too-costly",
                    "details": {
                        "text": msg,
                    },
                    "diagnostics": msg,
                }]
            });
            return (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response();
        }

        // (status, FHIR-issue-code, tx-issue-type, diagnostics)
        // The FHIR issue `code` and the `tx-issue-type` coding are usually
        // identical, but VsInvalid splits them: FHIR code stays `invalid`
        // (preserving the HTTP-level meaning) while tx-issue-type signals
        // `vs-invalid` so the IG validator can route the failure to a
        // ValueSet-definition diagnostic.
        let (status, code, tx_issue_type, diagnostics) = match &self {
            HtsError::NotFound(msg) => (
                StatusCode::NOT_FOUND,
                "not-found",
                "not-found",
                msg.as_str(),
            ),
            HtsError::NotSupported(msg) => (
                StatusCode::NOT_IMPLEMENTED,
                "not-supported",
                "not-supported",
                msg.as_str(),
            ),
            HtsError::InvalidRequest(msg) => {
                (StatusCode::BAD_REQUEST, "invalid", "invalid", msg.as_str())
            }
            HtsError::VsInvalid(msg) => (
                StatusCode::BAD_REQUEST,
                "invalid",
                "vs-invalid",
                msg.as_str(),
            ),
            HtsError::Internal(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "exception",
                "exception",
                msg.as_str(),
            ),
            HtsError::StorageError(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "exception",
                "exception",
                msg.as_str(),
            ),
            HtsError::PreconditionFailed(msg) => (
                StatusCode::PRECONDITION_FAILED,
                "conflict",
                "conflict",
                msg.as_str(),
            ),
            HtsError::TooCostly(_) => unreachable!("handled above"),
        };

        // The HL7 IG validator's TxTesterScrubbers strips OperationOutcome.issue
        // entries that have `diagnostics` but no `details` — every such issue
        // disappears from comparison. The IG fixtures additionally expect a
        // tx-issue-type coding inside details.
        let body = json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": code,
                "details": {
                    "coding": [{
                        "system": "http://hl7.org/fhir/tools/CodeSystem/tx-issue-type",
                        "code": tx_issue_type,
                    }],
                    "text": diagnostics,
                },
                "diagnostics": diagnostics,
            }]
        });

        (status, Json(body)).into_response()
    }
}
