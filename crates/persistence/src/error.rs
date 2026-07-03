//! Error types for the persistence layer.
//!
//! This module defines all error types used throughout the persistence layer,
//! following a hierarchy that separates storage errors, tenant errors, search errors,
//! and transaction errors.

use std::fmt;

use thiserror::Error;

use crate::tenant::TenantId;

/// The primary error type for all storage operations.
///
/// This enum encompasses all possible errors that can occur during persistence
/// operations, organized by category.
#[derive(Error, Debug)]
pub enum StorageError {
    /// Resource state errors
    #[error(transparent)]
    Resource(#[from] ResourceError),

    /// Concurrency and versioning errors
    #[error(transparent)]
    Concurrency(#[from] ConcurrencyError),

    /// Tenant isolation errors
    #[error(transparent)]
    Tenant(#[from] TenantError),

    /// Validation errors
    #[error(transparent)]
    Validation(#[from] ValidationError),

    /// Search operation errors
    #[error(transparent)]
    Search(#[from] SearchError),

    /// Transaction errors
    #[error(transparent)]
    Transaction(#[from] TransactionError),

    /// Backend-specific errors
    #[error(transparent)]
    Backend(#[from] BackendError),

    /// Bulk export errors
    #[error(transparent)]
    BulkExport(#[from] BulkExportError),

    /// Bulk submit errors
    #[error(transparent)]
    BulkSubmit(#[from] BulkSubmitError),
}

/// Errors related to resource state.
#[derive(Error, Debug)]
pub enum ResourceError {
    /// The requested resource was not found.
    #[error("resource not found: {resource_type}/{id}")]
    NotFound {
        /// FHIR resource type (e.g., `Patient`).
        resource_type: String,
        /// Logical id of the missing resource.
        id: String,
    },

    /// A resource with the given ID already exists.
    #[error("resource already exists: {resource_type}/{id}")]
    AlreadyExists {
        /// FHIR resource type.
        resource_type: String,
        /// Logical id that is already in use.
        id: String,
    },

    /// The resource has been deleted (HTTP 410 Gone).
    #[error("resource deleted: {resource_type}/{id}")]
    Gone {
        /// FHIR resource type of the deleted resource.
        resource_type: String,
        /// Logical id of the deleted resource.
        id: String,
        /// Timestamp at which the resource was deleted, when known.
        deleted_at: Option<chrono::DateTime<chrono::Utc>>,
    },

    /// The requested version of the resource was not found.
    #[error("version not found: {resource_type}/{id}/_history/{version_id}")]
    VersionNotFound {
        /// FHIR resource type.
        resource_type: String,
        /// Logical id of the resource.
        id: String,
        /// Version id that could not be located.
        version_id: String,
    },
}

/// Errors related to concurrency control.
#[derive(Error, Debug)]
pub enum ConcurrencyError {
    /// Version conflict detected during optimistic locking.
    #[error("version conflict: expected {expected_version}, found {actual_version}")]
    VersionConflict {
        /// FHIR resource type.
        resource_type: String,
        /// Logical id of the resource.
        id: String,
        /// Version id the client expected.
        expected_version: String,
        /// Version id currently stored.
        actual_version: String,
    },

    /// Optimistic lock failure (If-Match precondition failed).
    #[error("optimistic lock failure: resource {resource_type}/{id} has been modified")]
    OptimisticLockFailure {
        /// FHIR resource type.
        resource_type: String,
        /// Logical id of the resource.
        id: String,
        /// ETag value supplied by the client.
        expected_etag: String,
        /// Current ETag, if it could be read.
        actual_etag: Option<String>,
    },

    /// Deadlock detected during pessimistic locking.
    #[error("deadlock detected while accessing {resource_type}/{id}")]
    Deadlock {
        /// FHIR resource type.
        resource_type: String,
        /// Logical id of the resource.
        id: String,
    },

    /// Lock acquisition timed out.
    #[error("lock timeout after {timeout_ms}ms for {resource_type}/{id}")]
    LockTimeout {
        /// FHIR resource type.
        resource_type: String,
        /// Logical id of the resource.
        id: String,
        /// Lock-acquisition timeout that elapsed.
        timeout_ms: u64,
    },
}

/// Errors related to tenant isolation.
#[derive(Error, Debug)]
pub enum TenantError {
    /// Access to resource denied for the current tenant.
    #[error("access denied: tenant {tenant_id} cannot access {resource_type}/{resource_id}")]
    AccessDenied {
        /// Tenant attempting the access.
        tenant_id: TenantId,
        /// FHIR resource type.
        resource_type: String,
        /// Logical id of the protected resource.
        resource_id: String,
    },

    /// The specified tenant does not exist or is invalid.
    #[error("invalid tenant: {tenant_id}")]
    InvalidTenant {
        /// Tenant identifier that failed validation.
        tenant_id: TenantId,
    },

    /// Tenant is suspended and cannot perform operations.
    #[error("tenant suspended: {tenant_id}")]
    TenantSuspended {
        /// Identifier of the suspended tenant.
        tenant_id: TenantId,
    },

    /// Cross-tenant reference not allowed.
    #[error(
        "cross-tenant reference not allowed: resource in tenant {source_tenant} references resource in tenant {target_tenant}"
    )]
    CrossTenantReference {
        /// Tenant owning the referring resource.
        source_tenant: TenantId,
        /// Tenant owning the referenced resource.
        target_tenant: TenantId,
        /// Reference value that crossed the boundary.
        reference: String,
    },

    /// Operation not permitted for tenant.
    #[error("operation {operation} not permitted for tenant {tenant_id}")]
    OperationNotPermitted {
        /// Tenant attempting the operation.
        tenant_id: TenantId,
        /// Name of the operation that was rejected.
        operation: String,
    },
}

/// Errors related to resource validation.
#[derive(Error, Debug)]
pub enum ValidationError {
    /// The resource failed validation.
    #[error("invalid resource: {message}")]
    InvalidResource {
        /// Human-readable summary of the failure.
        message: String,
        /// Per-field validation details.
        details: Vec<ValidationDetail>,
    },

    /// The search parameter is invalid.
    #[error("invalid search parameter: {parameter}")]
    InvalidSearchParameter {
        /// Name of the offending search parameter.
        parameter: String,
        /// Human-readable explanation of the failure.
        message: String,
    },

    /// The resource type is not supported.
    #[error("unsupported resource type: {resource_type}")]
    UnsupportedResourceType {
        /// Unsupported FHIR resource type.
        resource_type: String,
    },

    /// Missing required field.
    #[error("missing required field: {field}")]
    MissingRequiredField {
        /// Name of the missing field.
        field: String,
    },

    /// Invalid reference format.
    #[error("invalid reference: {reference}")]
    InvalidReference {
        /// Reference string that failed parsing.
        reference: String,
        /// Human-readable failure detail.
        message: String,
    },
}

/// Detailed validation error information.
#[derive(Debug, Clone)]
pub struct ValidationDetail {
    /// The path to the field with the error (FHIRPath expression).
    pub path: String,
    /// A human-readable error message.
    pub message: String,
    /// The type of validation error.
    pub severity: ValidationSeverity,
}

/// Severity level for validation errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationSeverity {
    /// Fatal error - operation cannot proceed.
    Error,
    /// Warning - operation can proceed but with concerns.
    Warning,
    /// Informational - no action required.
    Information,
}

impl fmt::Display for ValidationSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationSeverity::Error => write!(f, "error"),
            ValidationSeverity::Warning => write!(f, "warning"),
            ValidationSeverity::Information => write!(f, "information"),
        }
    }
}

/// Errors related to search operations.
#[derive(Error, Debug)]
pub enum SearchError {
    /// The search parameter type is not supported.
    #[error("unsupported search parameter type: {param_type}")]
    UnsupportedParameterType {
        /// Unsupported parameter type label.
        param_type: String,
    },

    /// The search modifier is not supported for this parameter type.
    #[error("unsupported modifier '{modifier}' for parameter type '{param_type}'")]
    UnsupportedModifier {
        /// Modifier name (e.g., `contains`).
        modifier: String,
        /// Parameter type the modifier was applied to.
        param_type: String,
    },

    /// Chained search is not supported by this backend.
    #[error("chained search not supported: {chain}")]
    ChainedSearchNotSupported {
        /// Chain expression that was rejected.
        chain: String,
    },

    /// Reverse chaining (_has) is not supported by this backend.
    #[error("reverse chaining (_has) not supported")]
    ReverseChainNotSupported,

    /// Include/revinclude not supported.
    #[error("{operation} not supported by this backend")]
    IncludeNotSupported {
        /// Operation name (e.g., `_include`, `_revinclude`).
        operation: String,
    },

    /// Too many results to return.
    #[error("search result limit exceeded: found {count}, maximum is {max}")]
    TooManyResults {
        /// Number of matches the query produced.
        count: usize,
        /// Maximum allowed result count.
        max: usize,
    },

    /// Invalid cursor for pagination.
    #[error("invalid pagination cursor: {cursor}")]
    InvalidCursor {
        /// Cursor value that could not be decoded.
        cursor: String,
    },

    /// Search query parsing failed.
    #[error("failed to parse search query: {message}")]
    QueryParseError {
        /// Parser failure detail.
        message: String,
    },

    /// Composite search parameter error.
    #[error("invalid composite search parameter: {message}")]
    InvalidComposite {
        /// Human-readable failure detail.
        message: String,
    },

    /// Text search not available.
    #[error("full-text search not available")]
    TextSearchNotAvailable,
}

/// Errors related to transactions.
#[derive(Error, Debug)]
pub enum TransactionError {
    /// Transaction timed out.
    #[error("transaction timed out after {timeout_ms}ms")]
    Timeout {
        /// Timeout that elapsed before the transaction completed.
        timeout_ms: u64,
    },

    /// Transaction was rolled back.
    #[error("transaction rolled back: {reason}")]
    RolledBack {
        /// Human-readable explanation of why the transaction rolled back.
        reason: String,
    },

    /// Transaction is no longer valid (already committed or rolled back).
    #[error("transaction no longer valid")]
    InvalidTransaction,

    /// Nested transactions not supported.
    #[error("nested transactions not supported")]
    NestedNotSupported,

    /// Bundle processing error.
    #[error("bundle processing error at entry {index}: {message}")]
    BundleError {
        /// Zero-based index of the bundle entry that failed.
        index: usize,
        /// Human-readable failure detail.
        message: String,
    },

    /// Conditional operation matched multiple resources.
    #[error("conditional {operation} matched {count} resources, expected at most 1")]
    MultipleMatches {
        /// Conditional operation name (e.g., `update`, `delete`).
        operation: String,
        /// Number of matching resources found.
        count: usize,
    },

    /// Isolation level not supported.
    #[error("isolation level {level} not supported by this backend")]
    UnsupportedIsolationLevel {
        /// Isolation level requested but not supported.
        level: String,
    },
}

/// Errors originating from the database backend.
#[derive(Error, Debug)]
pub enum BackendError {
    /// The backend is currently unavailable.
    #[error("backend unavailable: {backend_name}")]
    Unavailable {
        /// Backend identifier (e.g., `postgres`).
        backend_name: String,
        /// Human-readable failure detail.
        message: String,
    },

    /// Connection to the backend failed.
    #[error("connection failed to {backend_name}: {message}")]
    ConnectionFailed {
        /// Backend identifier.
        backend_name: String,
        /// Underlying connection error message.
        message: String,
    },

    /// Connection pool exhausted.
    #[error("connection pool exhausted for {backend_name}")]
    PoolExhausted {
        /// Backend identifier whose pool was exhausted.
        backend_name: String,
    },

    /// The requested capability is not supported by this backend.
    #[error("capability '{capability}' not supported by {backend_name}")]
    UnsupportedCapability {
        /// Backend identifier.
        backend_name: String,
        /// Capability name that was requested.
        capability: String,
    },

    /// Schema migration error.
    #[error("schema migration failed: {message}")]
    MigrationError {
        /// Migration failure detail.
        message: String,
    },

    /// Internal backend error.
    #[error("internal error in {backend_name}: {message}")]
    Internal {
        /// Backend identifier.
        backend_name: String,
        /// Human-readable failure detail.
        message: String,
        /// Underlying error, when one is available.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Query execution error.
    #[error("query execution failed: {message}")]
    QueryError {
        /// Failure detail from the database driver.
        message: String,
    },

    /// Serialization/deserialization error.
    #[error("serialization error: {message}")]
    SerializationError {
        /// Failure detail from the serializer.
        message: String,
    },
}

/// Errors related to bulk export operations.
#[derive(Error, Debug)]
pub enum BulkExportError {
    /// The export job was not found.
    #[error("export job not found: {job_id}")]
    JobNotFound {
        /// Identifier of the export job.
        job_id: String,
    },

    /// The job is in an invalid state for the requested operation.
    #[error("invalid job state: job {job_id} is {actual}, expected {expected}")]
    InvalidJobState {
        /// Identifier of the export job.
        job_id: String,
        /// State required for the operation.
        expected: String,
        /// State the job is currently in.
        actual: String,
    },

    /// The resource type cannot be exported.
    #[error("resource type '{resource_type}' is not exportable")]
    TypeNotExportable {
        /// FHIR resource type that cannot be exported.
        resource_type: String,
    },

    /// Invalid export request.
    #[error("invalid export request: {message}")]
    InvalidRequest {
        /// Human-readable explanation of the failure.
        message: String,
    },

    /// The specified group was not found.
    #[error("group not found: {group_id}")]
    GroupNotFound {
        /// Identifier of the missing group.
        group_id: String,
    },

    /// The output format is not supported.
    #[error("unsupported export format: {format}")]
    UnsupportedFormat {
        /// Requested output format.
        format: String,
    },

    /// Invalid type filter.
    #[error("invalid type filter for {resource_type}: {message}")]
    InvalidTypeFilter {
        /// FHIR resource type the filter applied to.
        resource_type: String,
        /// Human-readable explanation of the failure.
        message: String,
    },

    /// The export was cancelled.
    #[error("export job {job_id} was cancelled")]
    Cancelled {
        /// Identifier of the cancelled job.
        job_id: String,
    },

    /// Error writing export output.
    #[error("export write error: {message}")]
    WriteError {
        /// Underlying write failure detail.
        message: String,
    },

    /// Too many concurrent exports.
    #[error("too many concurrent exports (maximum: {max_concurrent})")]
    TooManyConcurrentExports {
        /// Configured concurrency cap.
        max_concurrent: u32,
    },

    /// The worker lease for this job was lost (reclaimed by another worker).
    #[error("export job {job_id} lease lost (reclaimed by another worker)")]
    LeaseLost {
        /// Identifier of the job whose lease was lost.
        job_id: String,
    },
}

/// Errors related to bulk submit operations.
#[derive(Error, Debug)]
pub enum BulkSubmitError {
    /// The submission was not found.
    #[error("submission not found: {submitter}/{submission_id}")]
    SubmissionNotFound {
        /// Submitter identifier.
        submitter: String,
        /// Submission identifier.
        submission_id: String,
    },

    /// The manifest was not found.
    #[error("manifest not found: {submission_id}/{manifest_id}")]
    ManifestNotFound {
        /// Parent submission identifier.
        submission_id: String,
        /// Manifest identifier.
        manifest_id: String,
    },

    /// The submission is in an invalid state for the requested operation.
    #[error("invalid submission state: {submission_id} is {actual}, expected {expected}")]
    InvalidState {
        /// Submission identifier.
        submission_id: String,
        /// State required for the operation.
        expected: String,
        /// State the submission is currently in.
        actual: String,
    },

    /// The submission is already complete.
    #[error("submission {submission_id} is already complete")]
    AlreadyComplete {
        /// Submission identifier.
        submission_id: String,
    },

    /// The submission was aborted.
    #[error("submission {submission_id} was aborted: {reason}")]
    Aborted {
        /// Submission identifier.
        submission_id: String,
        /// Human-readable abort reason.
        reason: String,
    },

    /// Maximum errors exceeded.
    #[error("submission {submission_id} exceeded maximum errors ({max_errors})")]
    MaxErrorsExceeded {
        /// Submission identifier.
        submission_id: String,
        /// Configured per-submission error cap.
        max_errors: u32,
    },

    /// Error parsing NDJSON entry.
    #[error("parse error at line {line}: {message}")]
    ParseError {
        /// 1-based line number where parsing failed.
        line: u64,
        /// Parser failure detail.
        message: String,
    },

    /// Invalid resource in submission.
    #[error("invalid resource at line {line}: {message}")]
    InvalidResource {
        /// 1-based line number of the invalid resource.
        line: u64,
        /// Validation failure detail.
        message: String,
    },

    /// Duplicate submission ID.
    #[error("duplicate submission: {submitter}/{submission_id}")]
    DuplicateSubmission {
        /// Submitter identifier.
        submitter: String,
        /// Submission identifier that was reused.
        submission_id: String,
    },

    /// Error replacing manifest.
    #[error("cannot replace manifest {manifest_url}: {reason}")]
    ManifestReplacementError {
        /// URL of the manifest that could not be replaced.
        manifest_url: String,
        /// Human-readable reason for the failure.
        reason: String,
    },

    /// Rollback failed.
    #[error("rollback failed for submission {submission_id}: {message}")]
    RollbackFailed {
        /// Submission identifier.
        submission_id: String,
        /// Rollback failure detail.
        message: String,
    },
}

/// Result type alias for storage operations.
pub type StorageResult<T> = Result<T, StorageError>;

/// Result type alias for search operations.
pub type SearchResult<T> = Result<T, SearchError>;

/// Result type alias for transaction operations.
pub type TransactionResult<T> = Result<T, TransactionError>;

// Implement conversions from common error types

impl From<serde_json::Error> for StorageError {
    fn from(err: serde_json::Error) -> Self {
        StorageError::Backend(BackendError::SerializationError {
            message: err.to_string(),
        })
    }
}

impl From<std::io::Error> for BackendError {
    fn from(err: std::io::Error) -> Self {
        BackendError::Internal {
            backend_name: "unknown".to_string(),
            message: err.to_string(),
            source: Some(Box::new(err)),
        }
    }
}

#[cfg(feature = "sqlite")]
impl From<rusqlite::Error> for StorageError {
    fn from(err: rusqlite::Error) -> Self {
        StorageError::Backend(BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: err.to_string(),
            source: Some(Box::new(err)),
        })
    }
}

#[cfg(feature = "sqlite")]
impl From<r2d2::Error> for StorageError {
    fn from(_err: r2d2::Error) -> Self {
        StorageError::Backend(BackendError::PoolExhausted {
            backend_name: "sqlite".to_string(),
        })
    }
}

#[cfg(feature = "postgres")]
impl From<tokio_postgres::Error> for StorageError {
    fn from(err: tokio_postgres::Error) -> Self {
        StorageError::Backend(BackendError::Internal {
            backend_name: "postgres".to_string(),
            message: err.to_string(),
            source: Some(Box::new(err)),
        })
    }
}

#[cfg(feature = "mongodb")]
impl From<mongodb::error::Error> for StorageError {
    fn from(err: mongodb::error::Error) -> Self {
        StorageError::Backend(BackendError::Internal {
            backend_name: "mongodb".to_string(),
            message: err.to_string(),
            source: Some(Box::new(err)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_error_display() {
        let err = StorageError::Resource(ResourceError::NotFound {
            resource_type: "Patient".to_string(),
            id: "123".to_string(),
        });
        assert_eq!(err.to_string(), "resource not found: Patient/123");
    }

    #[test]
    fn test_concurrency_error_display() {
        let err = ConcurrencyError::VersionConflict {
            resource_type: "Patient".to_string(),
            id: "123".to_string(),
            expected_version: "1".to_string(),
            actual_version: "2".to_string(),
        };
        assert_eq!(err.to_string(), "version conflict: expected 1, found 2");
    }

    #[test]
    fn test_tenant_error_display() {
        let err = TenantError::AccessDenied {
            tenant_id: TenantId::new("tenant-a"),
            resource_type: "Patient".to_string(),
            resource_id: "123".to_string(),
        };
        assert!(err.to_string().contains("access denied"));
    }

    #[test]
    fn test_search_error_display() {
        let err = SearchError::UnsupportedModifier {
            modifier: "contains".to_string(),
            param_type: "token".to_string(),
        };
        assert!(err.to_string().contains("unsupported modifier"));
    }

    #[test]
    fn test_validation_severity_display() {
        assert_eq!(ValidationSeverity::Error.to_string(), "error");
        assert_eq!(ValidationSeverity::Warning.to_string(), "warning");
        assert_eq!(ValidationSeverity::Information.to_string(), "information");
    }

    #[test]
    fn test_bulk_export_error_display() {
        let err = BulkExportError::JobNotFound {
            job_id: "abc-123".to_string(),
        };
        assert_eq!(err.to_string(), "export job not found: abc-123");

        let err = BulkExportError::InvalidJobState {
            job_id: "abc-123".to_string(),
            expected: "in-progress".to_string(),
            actual: "complete".to_string(),
        };
        assert!(err.to_string().contains("invalid job state"));
    }

    #[test]
    fn test_bulk_submit_error_display() {
        let err = BulkSubmitError::SubmissionNotFound {
            submitter: "test-system".to_string(),
            submission_id: "sub-123".to_string(),
        };
        assert_eq!(err.to_string(), "submission not found: test-system/sub-123");

        let err = BulkSubmitError::ParseError {
            line: 42,
            message: "invalid JSON".to_string(),
        };
        assert!(err.to_string().contains("line 42"));
    }

    #[test]
    fn test_storage_error_from_bulk_errors() {
        let export_err = BulkExportError::JobNotFound {
            job_id: "test".to_string(),
        };
        let storage_err: StorageError = export_err.into();
        assert!(matches!(storage_err, StorageError::BulkExport(_)));

        let submit_err = BulkSubmitError::SubmissionNotFound {
            submitter: "test".to_string(),
            submission_id: "123".to_string(),
        };
        let storage_err: StorageError = submit_err.into();
        assert!(matches!(storage_err, StorageError::BulkSubmit(_)));
    }
}
