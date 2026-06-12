//! Error types for the FHIR REST API.
//!
//! This module defines all error types used throughout the REST API layer,
//! with automatic conversion to FHIR OperationOutcome responses.
//!
//! # Error Mapping
//!
//! Storage errors from the persistence layer are automatically mapped to
//! appropriate HTTP status codes and FHIR issue codes:
//!
//! | Storage Error | HTTP Status | FHIR Issue Code |
//! |--------------|-------------|-----------------|
//! | NotFound | 404 | not-found |
//! | Gone | 410 | deleted |
//! | VersionConflict | 409 | conflict |
//! | OptimisticLockFailure | 412 | conflict |
//! | MultipleMatches | 412 | multiple-matches |
//! | ValidationError | 400 | invalid |
//! | UnsupportedResourceType | 400 | not-supported |
//! | AccessDenied | 403 | forbidden |
//! | BackendError | 500 | exception |

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use helios_persistence::error::{
    BackendError, ConcurrencyError, ResourceError, SearchError, StorageError, TenantError,
    TransactionError, ValidationError,
};
use std::fmt;

/// The primary error type for REST API operations.
///
/// This enum provides semantic error types that map cleanly to HTTP status codes
/// and FHIR OperationOutcome issue codes.
#[derive(Debug)]
pub enum RestError {
    /// Resource not found (HTTP 404).
    NotFound {
        /// The resource type (e.g., "Patient").
        resource_type: String,
        /// The resource ID.
        id: String,
    },

    /// Resource was deleted (HTTP 410 Gone).
    Gone {
        /// The resource type.
        resource_type: String,
        /// The resource ID.
        id: String,
    },

    /// Version not found for vread (HTTP 404).
    VersionNotFound {
        /// The resource type.
        resource_type: String,
        /// The resource ID.
        id: String,
        /// The version ID.
        version_id: String,
    },

    /// Version conflict during update (HTTP 409).
    VersionConflict {
        /// The resource type.
        resource_type: String,
        /// The resource ID.
        id: String,
        /// Message describing the conflict.
        message: String,
    },

    /// Precondition failed - If-Match or If-None-Match (HTTP 412).
    PreconditionFailed {
        /// Message describing why the precondition failed.
        message: String,
    },

    /// Multiple resources matched conditional operation (HTTP 412).
    MultipleMatches {
        /// The operation being performed.
        operation: String,
        /// Number of matching resources.
        count: usize,
    },

    /// Bad request - validation error (HTTP 400).
    BadRequest {
        /// Error message.
        message: String,
    },

    /// Unsupported media type (HTTP 415).
    UnsupportedMediaType {
        /// The unsupported content type.
        content_type: String,
    },

    /// Request body exceeds the configured size limit (HTTP 413).
    PayloadTooLarge {
        /// Error message.
        message: String,
    },

    /// Unprocessable entity - semantic error (HTTP 422).
    UnprocessableEntity {
        /// Error message.
        message: String,
    },

    /// Unauthorized — missing or invalid authentication (HTTP 401).
    Unauthorized {
        /// Error message.
        message: String,
    },

    /// Access denied (HTTP 403).
    Forbidden {
        /// Error message.
        message: String,
    },

    /// Method not allowed (HTTP 405).
    MethodNotAllowed {
        /// The method that was attempted.
        method: String,
        /// The resource type.
        resource_type: String,
    },

    /// Conflict — e.g. duplicate submission or invalid state transition (HTTP 409).
    Conflict {
        /// Error message.
        message: String,
    },

    /// Too many requests — e.g. a concurrent submission is in progress (HTTP 429).
    TooManyRequests {
        /// Error message.
        message: String,
    },

    /// Not implemented (HTTP 501).
    NotImplemented {
        /// Description of what's not implemented.
        feature: String,
    },

    /// Internal server error (HTTP 500).
    InternalError {
        /// Error message.
        message: String,
    },

    /// Not acceptable - requested version not available (HTTP 406).
    NotAcceptable {
        /// Error message.
        message: String,
    },

    /// Invalid search parameter (HTTP 400).
    InvalidParameter {
        /// The parameter name.
        param: String,
        /// Error message.
        message: String,
    },
}

impl fmt::Display for RestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RestError::NotFound { resource_type, id } => {
                write!(f, "Resource not found: {}/{}", resource_type, id)
            }
            RestError::Gone { resource_type, id } => {
                write!(f, "Resource deleted: {}/{}", resource_type, id)
            }
            RestError::VersionNotFound {
                resource_type,
                id,
                version_id,
            } => {
                write!(
                    f,
                    "Version not found: {}/{}/_history/{}",
                    resource_type, id, version_id
                )
            }
            RestError::VersionConflict { message, .. } => {
                write!(f, "Version conflict: {}", message)
            }
            RestError::PreconditionFailed { message } => {
                write!(f, "Precondition failed: {}", message)
            }
            RestError::MultipleMatches { operation, count } => {
                write!(
                    f,
                    "Multiple matches ({}) for conditional {}",
                    count, operation
                )
            }
            RestError::BadRequest { message } => {
                write!(f, "Bad request: {}", message)
            }
            RestError::UnsupportedMediaType { content_type } => {
                write!(f, "Unsupported media type: {}", content_type)
            }
            RestError::PayloadTooLarge { message } => {
                write!(f, "Payload too large: {}", message)
            }
            RestError::UnprocessableEntity { message } => {
                write!(f, "Unprocessable entity: {}", message)
            }
            RestError::Unauthorized { message } => {
                write!(f, "Unauthorized: {}", message)
            }
            RestError::Forbidden { message } => {
                write!(f, "Forbidden: {}", message)
            }
            RestError::MethodNotAllowed {
                method,
                resource_type,
            } => {
                write!(f, "Method {} not allowed on {}", method, resource_type)
            }
            RestError::Conflict { message } => {
                write!(f, "Conflict: {}", message)
            }
            RestError::TooManyRequests { message } => {
                write!(f, "Too many requests: {}", message)
            }
            RestError::NotImplemented { feature } => {
                write!(f, "Not implemented: {}", feature)
            }
            RestError::InternalError { message } => {
                write!(f, "Internal error: {}", message)
            }
            RestError::NotAcceptable { message } => {
                write!(f, "Not acceptable: {}", message)
            }
            RestError::InvalidParameter { param, message } => {
                write!(f, "Invalid parameter '{}': {}", param, message)
            }
        }
    }
}

impl std::error::Error for RestError {}

impl IntoResponse for RestError {
    fn into_response(self) -> Response {
        let (status, code, details) = match &self {
            RestError::NotFound { resource_type, id } => (
                StatusCode::NOT_FOUND,
                "not-found",
                format!("Resource {}/{} not found", resource_type, id),
            ),
            RestError::Gone { resource_type, id } => (
                StatusCode::GONE,
                "deleted",
                format!("Resource {}/{} has been deleted", resource_type, id),
            ),
            RestError::VersionNotFound {
                resource_type,
                id,
                version_id,
            } => (
                StatusCode::NOT_FOUND,
                "not-found",
                format!(
                    "Version {} of {}/{} not found",
                    version_id, resource_type, id
                ),
            ),
            RestError::VersionConflict { message, .. } => {
                (StatusCode::CONFLICT, "conflict", message.clone())
            }
            RestError::PreconditionFailed { message } => {
                (StatusCode::PRECONDITION_FAILED, "conflict", message.clone())
            }
            RestError::MultipleMatches { operation, count } => (
                StatusCode::PRECONDITION_FAILED,
                "multiple-matches",
                format!(
                    "Conditional {} matched {} resources, expected at most 1",
                    operation, count
                ),
            ),
            RestError::BadRequest { message } => {
                (StatusCode::BAD_REQUEST, "invalid", message.clone())
            }
            RestError::UnsupportedMediaType { content_type } => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "not-supported",
                format!("Content type '{}' is not supported", content_type),
            ),
            RestError::PayloadTooLarge { message } => {
                (StatusCode::PAYLOAD_TOO_LARGE, "too-long", message.clone())
            }
            RestError::UnprocessableEntity { message } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "processing",
                message.clone(),
            ),
            RestError::Unauthorized { message } => {
                let operation_outcome = create_operation_outcome("error", "login", message);
                return (
                    StatusCode::UNAUTHORIZED,
                    [(
                        axum::http::header::WWW_AUTHENTICATE,
                        axum::http::HeaderValue::from_static("Bearer"),
                    )],
                    Json(operation_outcome),
                )
                    .into_response();
            }
            RestError::Forbidden { message } => {
                (StatusCode::FORBIDDEN, "forbidden", message.clone())
            }
            RestError::MethodNotAllowed {
                method,
                resource_type,
            } => (
                StatusCode::METHOD_NOT_ALLOWED,
                "not-supported",
                format!("Method {} not allowed on {}", method, resource_type),
            ),
            RestError::Conflict { message } => (StatusCode::CONFLICT, "conflict", message.clone()),
            RestError::TooManyRequests { message } => {
                (StatusCode::TOO_MANY_REQUESTS, "throttled", message.clone())
            }
            RestError::NotImplemented { feature } => (
                StatusCode::NOT_IMPLEMENTED,
                "not-supported",
                format!("Feature '{}' is not implemented", feature),
            ),
            RestError::InternalError { message } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "exception",
                message.clone(),
            ),
            RestError::NotAcceptable { message } => {
                (StatusCode::NOT_ACCEPTABLE, "not-supported", message.clone())
            }
            RestError::InvalidParameter { param, message } => (
                StatusCode::BAD_REQUEST,
                "invalid",
                format!("Invalid parameter '{}': {}", param, message),
            ),
        };

        let operation_outcome = create_operation_outcome("error", code, &details);
        (status, Json(operation_outcome)).into_response()
    }
}

/// Creates a FHIR OperationOutcome resource.
///
/// # Arguments
///
/// * `severity` - The issue severity (fatal, error, warning, information)
/// * `code` - The FHIR issue code
/// * `details` - Human-readable details
fn create_operation_outcome(severity: &str, code: &str, details: &str) -> serde_json::Value {
    serde_json::json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": severity,
            "code": code,
            "details": {
                "text": details
            }
        }]
    })
}

/// Creates an OperationOutcome with multiple issues.
///
/// This is useful for validation errors where multiple problems may be reported.
pub fn create_operation_outcome_multi(issues: Vec<(String, String, String)>) -> serde_json::Value {
    let issues: Vec<_> = issues
        .into_iter()
        .map(|(severity, code, details)| {
            serde_json::json!({
                "severity": severity,
                "code": code,
                "details": {
                    "text": details
                }
            })
        })
        .collect();

    serde_json::json!({
        "resourceType": "OperationOutcome",
        "issue": issues
    })
}

// Implement conversion from AuthError

impl From<helios_auth::AuthError> for RestError {
    fn from(err: helios_auth::AuthError) -> Self {
        match err {
            helios_auth::AuthError::Forbidden {
                resource_type,
                operation,
            } => RestError::Forbidden {
                message: format!("Insufficient scope for {} on {}", operation, resource_type),
            },
            helios_auth::AuthError::InternalError(msg) => RestError::InternalError { message: msg },
            other => RestError::Unauthorized {
                message: other.to_string(),
            },
        }
    }
}

// Implement conversions from storage errors

impl From<StorageError> for RestError {
    fn from(err: StorageError) -> Self {
        match err {
            StorageError::Resource(e) => e.into(),
            StorageError::Concurrency(e) => e.into(),
            StorageError::Tenant(e) => e.into(),
            StorageError::Validation(e) => e.into(),
            StorageError::Search(e) => e.into(),
            StorageError::Transaction(e) => e.into(),
            StorageError::Backend(e) => e.into(),
            StorageError::BulkExport(e) => RestError::InternalError {
                message: e.to_string(),
            },
            StorageError::BulkSubmit(e) => {
                use helios_persistence::error::BulkSubmitError as B;
                let msg = e.to_string();
                match e {
                    B::SubmissionNotFound { .. } => RestError::NotFound {
                        resource_type: "Submission".to_string(),
                        id: msg,
                    },
                    B::ManifestNotFound { .. } => RestError::NotFound {
                        resource_type: "SubmissionManifest".to_string(),
                        id: msg,
                    },
                    B::DuplicateSubmission { .. }
                    | B::AlreadyComplete { .. }
                    | B::InvalidState { .. }
                    | B::Aborted { .. }
                    | B::ManifestReplacementError { .. } => RestError::Conflict { message: msg },
                    B::ParseError { .. }
                    | B::InvalidResource { .. }
                    | B::MaxErrorsExceeded { .. } => {
                        RestError::UnprocessableEntity { message: msg }
                    }
                    B::RollbackFailed { .. } => RestError::InternalError { message: msg },
                }
            }
        }
    }
}

impl From<ResourceError> for RestError {
    fn from(err: ResourceError) -> Self {
        match err {
            ResourceError::NotFound { resource_type, id } => {
                RestError::NotFound { resource_type, id }
            }
            ResourceError::AlreadyExists { resource_type, id } => RestError::VersionConflict {
                resource_type: resource_type.clone(),
                id: id.clone(),
                message: format!("Resource {}/{} already exists", resource_type, id),
            },
            ResourceError::Gone {
                resource_type, id, ..
            } => RestError::Gone { resource_type, id },
            ResourceError::VersionNotFound {
                resource_type,
                id,
                version_id,
            } => RestError::VersionNotFound {
                resource_type,
                id,
                version_id,
            },
        }
    }
}

impl From<ConcurrencyError> for RestError {
    fn from(err: ConcurrencyError) -> Self {
        match err {
            ConcurrencyError::VersionConflict {
                resource_type,
                id,
                expected_version,
                actual_version,
            } => RestError::VersionConflict {
                resource_type,
                id,
                message: format!(
                    "Expected version {}, but found {}",
                    expected_version, actual_version
                ),
            },
            ConcurrencyError::OptimisticLockFailure {
                resource_type,
                id,
                expected_etag,
                ..
            } => RestError::PreconditionFailed {
                message: format!(
                    "Resource {}/{} was modified (expected ETag: {})",
                    resource_type, id, expected_etag
                ),
            },
            ConcurrencyError::Deadlock {
                resource_type, id, ..
            } => RestError::InternalError {
                message: format!("Deadlock detected for {}/{}", resource_type, id),
            },
            ConcurrencyError::LockTimeout {
                resource_type,
                id,
                timeout_ms,
            } => RestError::InternalError {
                message: format!(
                    "Lock timeout ({}ms) for {}/{}",
                    timeout_ms, resource_type, id
                ),
            },
        }
    }
}

impl From<TenantError> for RestError {
    fn from(err: TenantError) -> Self {
        match err {
            TenantError::AccessDenied { .. } => RestError::Forbidden {
                message: err.to_string(),
            },
            TenantError::InvalidTenant { .. } => RestError::BadRequest {
                message: err.to_string(),
            },
            TenantError::TenantSuspended { .. } => RestError::Forbidden {
                message: err.to_string(),
            },
            TenantError::CrossTenantReference { .. } => RestError::BadRequest {
                message: err.to_string(),
            },
            TenantError::OperationNotPermitted { .. } => RestError::Forbidden {
                message: err.to_string(),
            },
        }
    }
}

impl From<ValidationError> for RestError {
    fn from(err: ValidationError) -> Self {
        match err {
            ValidationError::InvalidResource { message, .. } => RestError::BadRequest { message },
            ValidationError::InvalidSearchParameter { parameter, message } => {
                RestError::BadRequest {
                    message: format!("Invalid search parameter '{}': {}", parameter, message),
                }
            }
            ValidationError::UnsupportedResourceType { resource_type } => RestError::BadRequest {
                message: format!("Unsupported resource type: {}", resource_type),
            },
            ValidationError::MissingRequiredField { field } => RestError::BadRequest {
                message: format!("Missing required field: {}", field),
            },
            ValidationError::InvalidReference { reference, message } => RestError::BadRequest {
                message: format!("Invalid reference '{}': {}", reference, message),
            },
        }
    }
}

impl From<SearchError> for RestError {
    fn from(err: SearchError) -> Self {
        match err {
            SearchError::UnsupportedParameterType { .. }
            | SearchError::UnsupportedModifier { .. }
            | SearchError::InvalidComposite { .. }
            | SearchError::QueryParseError { .. }
            | SearchError::InvalidCursor { .. } => RestError::BadRequest {
                message: err.to_string(),
            },
            SearchError::ChainedSearchNotSupported { .. }
            | SearchError::ReverseChainNotSupported
            | SearchError::IncludeNotSupported { .. }
            | SearchError::TextSearchNotAvailable => RestError::NotImplemented {
                feature: err.to_string(),
            },
            SearchError::TooManyResults { count, max } => RestError::UnprocessableEntity {
                message: format!("Search returned {} results, maximum is {}", count, max),
            },
        }
    }
}

impl From<TransactionError> for RestError {
    fn from(err: TransactionError) -> Self {
        match err {
            TransactionError::MultipleMatches { operation, count } => {
                RestError::MultipleMatches { operation, count }
            }
            TransactionError::BundleError { index, message } => RestError::BadRequest {
                message: format!("Bundle entry {}: {}", index, message),
            },
            TransactionError::Timeout { .. }
            | TransactionError::RolledBack { .. }
            | TransactionError::InvalidTransaction
            | TransactionError::NestedNotSupported
            | TransactionError::UnsupportedIsolationLevel { .. } => RestError::InternalError {
                message: err.to_string(),
            },
        }
    }
}

impl From<BackendError> for RestError {
    fn from(err: BackendError) -> Self {
        match err {
            BackendError::UnsupportedCapability { capability, .. } => RestError::NotImplemented {
                feature: capability,
            },
            _ => RestError::InternalError {
                message: err.to_string(),
            },
        }
    }
}

impl From<serde_json::Error> for RestError {
    fn from(err: serde_json::Error) -> Self {
        RestError::BadRequest {
            message: format!("Invalid JSON: {}", err),
        }
    }
}

/// Result type alias for REST operations.
pub type RestResult<T> = Result<T, RestError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_found_display() {
        let err = RestError::NotFound {
            resource_type: "Patient".to_string(),
            id: "123".to_string(),
        };
        assert_eq!(err.to_string(), "Resource not found: Patient/123");
    }

    #[test]
    fn test_gone_display() {
        let err = RestError::Gone {
            resource_type: "Patient".to_string(),
            id: "123".to_string(),
        };
        assert_eq!(err.to_string(), "Resource deleted: Patient/123");
    }

    #[test]
    fn test_version_conflict_display() {
        let err = RestError::VersionConflict {
            resource_type: "Patient".to_string(),
            id: "123".to_string(),
            message: "Expected version 1, found 2".to_string(),
        };
        assert!(err.to_string().contains("Version conflict"));
    }

    #[test]
    fn test_multiple_matches_display() {
        let err = RestError::MultipleMatches {
            operation: "update".to_string(),
            count: 3,
        };
        assert!(err.to_string().contains("3"));
        assert!(err.to_string().contains("update"));
    }

    #[test]
    fn test_create_operation_outcome() {
        let outcome = create_operation_outcome("error", "not-found", "Resource not found");
        assert_eq!(outcome["resourceType"], "OperationOutcome");
        assert_eq!(outcome["issue"][0]["severity"], "error");
        assert_eq!(outcome["issue"][0]["code"], "not-found");
    }

    #[test]
    fn test_create_operation_outcome_multi() {
        let issues = vec![
            (
                "error".to_string(),
                "invalid".to_string(),
                "First error".to_string(),
            ),
            (
                "warning".to_string(),
                "informational".to_string(),
                "A warning".to_string(),
            ),
        ];
        let outcome = create_operation_outcome_multi(issues);
        assert_eq!(outcome["resourceType"], "OperationOutcome");
        assert_eq!(outcome["issue"].as_array().unwrap().len(), 2);
    }

    // ── Display for the bulk-submit error variants ────────────────

    #[test]
    fn test_conflict_display() {
        let err = RestError::Conflict {
            message: "duplicate submission".to_string(),
        };
        assert_eq!(err.to_string(), "Conflict: duplicate submission");
    }

    #[test]
    fn test_too_many_requests_display() {
        let err = RestError::TooManyRequests {
            message: "submission in progress".to_string(),
        };
        assert_eq!(err.to_string(), "Too many requests: submission in progress");
    }

    // ── into_response() status codes ──────────────────────────────

    #[test]
    fn test_conflict_into_response_status() {
        let resp = RestError::Conflict {
            message: "x".to_string(),
        }
        .into_response();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[test]
    fn test_too_many_requests_into_response_status() {
        let resp = RestError::TooManyRequests {
            message: "x".to_string(),
        }
        .into_response();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    // ── StorageError::BulkSubmit → RestError mapping ──────────────

    fn status_of(err: StorageError) -> StatusCode {
        RestError::from(err).into_response().status()
    }

    #[test]
    fn test_bulk_submit_submission_not_found_maps_to_404() {
        use helios_persistence::error::BulkSubmitError;
        let err = StorageError::BulkSubmit(BulkSubmitError::SubmissionNotFound {
            submitter: "acme".to_string(),
            submission_id: "s1".to_string(),
        });
        assert_eq!(status_of(err), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_bulk_submit_manifest_not_found_maps_to_404() {
        use helios_persistence::error::BulkSubmitError;
        let err = StorageError::BulkSubmit(BulkSubmitError::ManifestNotFound {
            submission_id: "s1".to_string(),
            manifest_id: "m1".to_string(),
        });
        assert_eq!(status_of(err), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_bulk_submit_conflict_variants_map_to_409() {
        use helios_persistence::error::BulkSubmitError;
        let cases = vec![
            StorageError::BulkSubmit(BulkSubmitError::DuplicateSubmission {
                submitter: "acme".to_string(),
                submission_id: "s1".to_string(),
            }),
            StorageError::BulkSubmit(BulkSubmitError::AlreadyComplete {
                submission_id: "s1".to_string(),
            }),
            StorageError::BulkSubmit(BulkSubmitError::InvalidState {
                submission_id: "s1".to_string(),
                expected: "in-progress".to_string(),
                actual: "complete".to_string(),
            }),
            StorageError::BulkSubmit(BulkSubmitError::Aborted {
                submission_id: "s1".to_string(),
                reason: "cancelled".to_string(),
            }),
            StorageError::BulkSubmit(BulkSubmitError::ManifestReplacementError {
                manifest_url: "http://x".to_string(),
                reason: "in-flight".to_string(),
            }),
        ];
        for err in cases {
            assert_eq!(status_of(err), StatusCode::CONFLICT);
        }
    }

    #[test]
    fn test_bulk_submit_processing_variants_map_to_422() {
        use helios_persistence::error::BulkSubmitError;
        let cases = vec![
            StorageError::BulkSubmit(BulkSubmitError::ParseError {
                line: 3,
                message: "bad json".to_string(),
            }),
            StorageError::BulkSubmit(BulkSubmitError::InvalidResource {
                line: 7,
                message: "missing resourceType".to_string(),
            }),
            StorageError::BulkSubmit(BulkSubmitError::MaxErrorsExceeded {
                submission_id: "s1".to_string(),
                max_errors: 10,
            }),
        ];
        for err in cases {
            assert_eq!(status_of(err), StatusCode::UNPROCESSABLE_ENTITY);
        }
    }

    #[test]
    fn test_bulk_submit_rollback_failed_maps_to_500() {
        use helios_persistence::error::BulkSubmitError;
        let err = StorageError::BulkSubmit(BulkSubmitError::RollbackFailed {
            submission_id: "s1".to_string(),
            message: "db down".to_string(),
        });
        assert_eq!(status_of(err), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
