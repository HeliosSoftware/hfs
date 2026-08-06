//! Error types for the persistence layer.
//!
//! This module defines all error types used throughout the persistence layer,
//! following a hierarchy that separates storage errors, tenant errors, search errors,
//! and transaction errors.

use std::fmt;

use thiserror::Error;

use crate::tenant::{TenantId, TenantIdError};

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

    /// A tenant id offered for provisioning is not canonical.
    ///
    /// Distinct from [`InvalidTenant`](Self::InvalidTenant) because it carries
    /// the *reason*: this reaches an operator through the admin API, where "the
    /// id is 71 bytes" is actionable and "invalid tenant" is not.
    #[error("tenant id '{tenant_id}' is not valid: {reason}")]
    NonCanonicalTenantId {
        /// The rejected identifier, as supplied.
        tenant_id: String,
        /// Why [`TenantId::parse`](crate::tenant::TenantId::parse) rejected it.
        reason: TenantIdError,
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

    /// The backend cancelled the operation because it exceeded a server-side
    /// time limit.
    ///
    /// This is a *statement*-level deadline (PostgreSQL `statement_timeout`
    /// → SQLSTATE `57014`, MongoDB `maxTimeMS` → `MaxTimeMSExpired`, an
    /// explicit SQLite `interrupt`), not a connection-level failure: the
    /// backend is healthy and reachable, and it deliberately stopped *this*
    /// statement. That distinction is why it does not map onto
    /// [`Self::Unavailable`] — the server is fine, the query was too
    /// expensive — and why the REST layer answers `504` rather than `503`
    /// (see `helios_rest::error`, issue #353).
    ///
    /// Lock-wait expiry is deliberately **not** this variant. SQLite's
    /// `SQLITE_BUSY` after `busy_timeout` means "someone else held the write
    /// lock", which a retry genuinely resolves, so it classifies as
    /// [`Self::Unavailable`] (503 + `Retry-After`) instead.
    #[error("{backend_name} operation timed out: {message}")]
    Timeout {
        /// Backend identifier (e.g., `postgres`).
        backend_name: String,
        /// Human-readable failure detail, including the driver context. Never
        /// surfaced to HTTP clients — the REST layer logs it and replies with a
        /// fixed, backend-agnostic message.
        message: String,
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

/// Classifies a `rusqlite` error into a [`BackendError`], preserving the
/// driver's `ErrorCode` rather than collapsing everything to `Internal`.
///
/// `context` is prepended to the driver text so the caller's description of
/// *what* it was doing survives classification.
///
/// - `SQLITE_INTERRUPT` — the statement was deliberately cancelled
///   (`Connection::interrupt`) → [`BackendError::Timeout`] (504).
/// - `SQLITE_BUSY` / `SQLITE_LOCKED` — the `busy_timeout` elapsed waiting for
///   the write lock. The database is healthy and merely contended, and a retry
///   usually succeeds, so this is [`BackendError::Unavailable`] (503 +
///   `Retry-After`) — *not* `Timeout`. Before #353 it was a 500, which told
///   clients a transient lock conflict was a server defect.
/// - everything else — unchanged: [`BackendError::Internal`], byte-for-byte the
///   message this helper's callers produced before.
#[cfg(feature = "sqlite")]
pub fn classify_sqlite_error(context: &str, err: rusqlite::Error) -> BackendError {
    use rusqlite::ErrorCode;

    // `sqlite_error_code()` is rusqlite's own accessor for the primary result
    // code; it yields `None` for the non-`SqliteFailure` variants (e.g.
    // `QueryReturnedNoRows`, a type-conversion failure), which correctly fall
    // through to `Internal` below.
    let code = err.sqlite_error_code();
    let message = if context.is_empty() {
        err.to_string()
    } else {
        format!("{context}: {err}")
    };

    match code {
        Some(ErrorCode::OperationInterrupted) => BackendError::Timeout {
            backend_name: "sqlite".to_string(),
            message,
        },
        Some(ErrorCode::DatabaseBusy) | Some(ErrorCode::DatabaseLocked) => {
            BackendError::Unavailable {
                backend_name: "sqlite".to_string(),
                message,
            }
        }
        _ => BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message,
            source: Some(Box::new(err)),
        },
    }
}

#[cfg(feature = "sqlite")]
impl From<rusqlite::Error> for StorageError {
    fn from(err: rusqlite::Error) -> Self {
        StorageError::Backend(classify_sqlite_error("", err))
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

/// Classifies a `tokio_postgres` error into a [`BackendError`] by SQLSTATE,
/// preserving the code rather than collapsing everything to `Internal`.
///
/// `context` is prepended to the driver text so the caller's description of
/// *what* it was doing survives classification.
///
/// SQLSTATE is the only stable signal here: PostgreSQL localizes error
/// *messages* through `lc_messages`, so matching on the text
/// ("canceling statement due to statement timeout") breaks on any server not
/// running an English locale. Classification must therefore happen while the
/// typed error is still in hand — once it has been through `format!` the code
/// is gone (issue #353).
///
/// - `57014 query_canceled` — the statement exceeded `statement_timeout` (see
///   `HFS_PG_STATEMENT_TIMEOUT_MS`) or was cancelled by `pg_cancel_backend`
///   → [`BackendError::Timeout`] (504).
/// - `53300 too_many_connections`, `53400 configuration_limit_exceeded`,
///   `57P01 admin_shutdown`, `57P02 crash_shutdown`, `57P03 cannot_connect_now`
///   — the server is saturated or going away, and a retry may well land
///   → [`BackendError::Unavailable`] (503 + `Retry-After`).
/// - everything else — unchanged: [`BackendError::Internal`], byte-for-byte the
///   message this helper's callers produced before.
///
/// Deliberately **not** reclassified here: `40001 serialization_failure` and
/// `40P01 deadlock_detected`. Both are retryable, but deciding what a FHIR
/// client should see for a write conflict is a separate question from
/// timeouts, and silently 503-ing a deadlock could mask a real lock-ordering
/// defect.
#[cfg(feature = "postgres")]
pub fn classify_postgres_error(context: &str, err: tokio_postgres::Error) -> BackendError {
    use tokio_postgres::error::SqlState;

    // `SqlState` is compared with `==`, never matched as a pattern: it is a
    // newtype over an enum with an `Other(Box<str>)` variant, which makes it
    // non-structural-match, so `SqlState::QUERY_CANCELED` in a pattern position
    // does not compile.
    let code = err.code().cloned();
    let message = if context.is_empty() {
        err.to_string()
    } else {
        format!("{context}: {err}")
    };

    if code.as_ref() == Some(&SqlState::QUERY_CANCELED) {
        return BackendError::Timeout {
            backend_name: "postgres".to_string(),
            message,
        };
    }

    let unavailable = matches!(
        code.as_ref(),
        Some(c) if *c == SqlState::TOO_MANY_CONNECTIONS
            || *c == SqlState::CONFIGURATION_LIMIT_EXCEEDED
            || *c == SqlState::ADMIN_SHUTDOWN
            || *c == SqlState::CRASH_SHUTDOWN
            || *c == SqlState::CANNOT_CONNECT_NOW
    );
    if unavailable {
        return BackendError::Unavailable {
            backend_name: "postgres".to_string(),
            message,
        };
    }

    BackendError::Internal {
        backend_name: "postgres".to_string(),
        message,
        source: Some(Box::new(err)),
    }
}

#[cfg(feature = "postgres")]
impl From<tokio_postgres::Error> for StorageError {
    fn from(err: tokio_postgres::Error) -> Self {
        StorageError::Backend(classify_postgres_error("", err))
    }
}

/// MongoDB server error code for a query that exceeded `maxTimeMS`.
#[cfg(feature = "mongodb")]
const MONGO_MAX_TIME_MS_EXPIRED: i32 = 50;
/// MongoDB server error code for an operation that exceeded its time limit.
#[cfg(feature = "mongodb")]
const MONGO_EXCEEDED_TIME_LIMIT: i32 = 262;

/// Classifies a MongoDB driver error into a [`BackendError`], preserving the
/// server error code rather than collapsing everything to `Internal`.
///
/// `context` is prepended to the driver text so the caller's description of
/// *what* it was doing survives classification.
///
/// - `MaxTimeMSExpired` (50) / `ExceededTimeLimit` (262) — the server stopped
///   the operation at its deadline → [`BackendError::Timeout`] (504).
/// - `Io` / `ConnectionPoolCleared` / `ServerSelection` — transport or
///   topology failure → [`BackendError::Unavailable`] (503 + `Retry-After`).
/// - everything else — unchanged: [`BackendError::Internal`], byte-for-byte the
///   message this helper's callers produced before.
///
/// Note that HFS does not currently set `maxTimeMS` on its queries, so the
/// timeout arm fires only when the deadline comes from the server or a
/// connection-string option. Wiring an HFS-side query deadline is tracked
/// separately; the classification is in place either way.
#[cfg(feature = "mongodb")]
pub fn classify_mongodb_error(context: &str, err: mongodb::error::Error) -> BackendError {
    use mongodb::error::ErrorKind;

    let message = if context.is_empty() {
        err.to_string()
    } else {
        format!("{context}: {err}")
    };

    let timed_out = matches!(
        err.kind.as_ref(),
        ErrorKind::Command(cmd)
            if cmd.code == MONGO_MAX_TIME_MS_EXPIRED || cmd.code == MONGO_EXCEEDED_TIME_LIMIT
    );
    if timed_out {
        return BackendError::Timeout {
            backend_name: "mongodb".to_string(),
            message,
        };
    }

    let unreachable = matches!(
        err.kind.as_ref(),
        ErrorKind::Io(_)
            | ErrorKind::ConnectionPoolCleared { .. }
            | ErrorKind::ServerSelection { .. }
    );
    if unreachable {
        return BackendError::Unavailable {
            backend_name: "mongodb".to_string(),
            message,
        };
    }

    BackendError::Internal {
        backend_name: "mongodb".to_string(),
        message,
        source: Some(Box::new(err)),
    }
}

#[cfg(feature = "mongodb")]
impl From<mongodb::error::Error> for StorageError {
    fn from(err: mongodb::error::Error) -> Self {
        StorageError::Backend(classify_mongodb_error("", err))
    }
}

/// Classifies a raw driver error into a [`StorageError`] in place, tagging it
/// with `context` — what the caller was doing when the driver failed.
///
/// This is the form every backend call site uses. It replaces the pre-#353
/// spelling, which stringified the driver error and so threw away the SQLSTATE
/// / `ErrorCode` that distinguishes a cancelled statement from a real defect:
///
/// ```ignore
/// // before — classification impossible, everything is a 500
/// .map_err(|e| internal_error(format!("Failed to prepare count_by_types: {e}")))?
/// // after
/// .or_query_error("Failed to prepare count_by_types")?
/// ```
///
/// A method rather than a `.map_err(|e| …)` closure, for two reasons beyond
/// brevity. First, the driver error type is named once, here, instead of being
/// re-bound at ~150 call sites. Second, the conversion sits on the call chain
/// rather than inside a closure body that only ever runs on failure, so
/// coverage tooling attributes it to the enclosing function instead of marking
/// every error-handling site in the backends as unexecuted — #353 rewrote all
/// of them at once, which made that reporting artifact impossible to miss.
///
/// The classification itself is unchanged and still lives in
/// `classify_sqlite_error` / `classify_postgres_error` / `classify_mongodb_error`,
/// where it is unit tested: a server-side deadline becomes
/// [`BackendError::Timeout`] (504), an unreachable or saturated server becomes
/// [`BackendError::Unavailable`] (503 + `Retry-After`), and everything else
/// stays [`BackendError::Internal`] (500) with byte-identical text to the
/// `internal_error(format!(…))` these call sites used before.
///
/// Because each impl is written for one concrete driver error type, a site
/// whose `Result` carries some other error (serde, chrono, a parse) fails to
/// compile rather than being silently mis-converted.
pub trait QueryErrorExt<T> {
    /// Converts a driver failure into a classified [`StorageError`], prefixing
    /// the driver text with `context`.
    fn or_query_error(self, context: &str) -> Result<T, StorageError>;
}

#[cfg(feature = "sqlite")]
impl<T> QueryErrorExt<T> for Result<T, rusqlite::Error> {
    fn or_query_error(self, context: &str) -> Result<T, StorageError> {
        self.map_err(|err| StorageError::Backend(classify_sqlite_error(context, err)))
    }
}

#[cfg(feature = "postgres")]
impl<T> QueryErrorExt<T> for Result<T, tokio_postgres::Error> {
    fn or_query_error(self, context: &str) -> Result<T, StorageError> {
        self.map_err(|err| StorageError::Backend(classify_postgres_error(context, err)))
    }
}

#[cfg(feature = "mongodb")]
impl<T> QueryErrorExt<T> for Result<T, mongodb::error::Error> {
    fn or_query_error(self, context: &str) -> Result<T, StorageError> {
        self.map_err(|err| StorageError::Backend(classify_mongodb_error(context, err)))
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

    // ── Driver-error classification (issue #353) ────────────────────────────
    //
    // `rusqlite::Error` is constructible, so the SQLite classifier is unit
    // testable. `tokio_postgres::Error` and `mongodb::error::Error` have no
    // public constructors, so their classifiers are covered by the
    // testcontainer integration tests instead (see `tests/postgres_tests.rs`).

    /// Builds a `rusqlite::Error` carrying the given primary result code.
    #[cfg(feature = "sqlite")]
    fn sqlite_failure(primary_code: std::os::raw::c_int) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(primary_code), None)
    }

    /// An interrupted statement is a server-side deadline, not a server fault:
    /// it must classify as `Timeout` so the REST layer answers 504.
    #[cfg(feature = "sqlite")]
    #[test]
    fn test_classify_sqlite_interrupt_is_timeout() {
        // SQLITE_INTERRUPT == 9
        let err = classify_sqlite_error("Failed to execute search", sqlite_failure(9));
        assert!(
            matches!(err, BackendError::Timeout { ref backend_name, .. } if backend_name == "sqlite"),
            "SQLITE_INTERRUPT must classify as Timeout, got {err:?}"
        );
        // The caller's context survives classification.
        assert!(err.to_string().contains("Failed to execute search"));
    }

    /// A lock-wait expiry is contention, not an over-long query: a retry
    /// genuinely succeeds, so it must be `Unavailable` (503 + Retry-After),
    /// NOT `Timeout` (504, which advises against retrying). Before #353 this
    /// was a 500.
    #[cfg(feature = "sqlite")]
    #[test]
    fn test_classify_sqlite_busy_and_locked_are_unavailable() {
        // SQLITE_BUSY == 5, SQLITE_LOCKED == 6
        for code in [5, 6] {
            let err = classify_sqlite_error("Failed to insert resource", sqlite_failure(code));
            assert!(
                matches!(err, BackendError::Unavailable { .. }),
                "SQLite primary code {code} must classify as Unavailable, got {err:?}"
            );
        }
    }

    /// Everything else keeps today's behaviour exactly: `Internal`, with the
    /// same `"{context}: {err}"` message the call sites built before. This is
    /// the property that makes the call-site conversion safe — an unclassified
    /// error is byte-identical to the pre-#353 result.
    #[cfg(feature = "sqlite")]
    #[test]
    fn test_classify_sqlite_other_errors_are_unchanged_internal() {
        // SQLITE_CONSTRAINT == 19 — a genuine defect, must stay a 500.
        let raw = sqlite_failure(19);
        let expected = format!("Failed to insert resource: {raw}");
        let err = classify_sqlite_error("Failed to insert resource", sqlite_failure(19));
        match err {
            BackendError::Internal {
                backend_name,
                message,
                ..
            } => {
                assert_eq!(backend_name, "sqlite");
                assert_eq!(
                    message, expected,
                    "unclassified errors must keep the pre-#353 message verbatim"
                );
            }
            other => panic!("SQLITE_CONSTRAINT must stay Internal, got {other:?}"),
        }

        // A non-`SqliteFailure` variant has no code at all and must also pass
        // through untouched.
        let err = classify_sqlite_error("ctx", rusqlite::Error::QueryReturnedNoRows);
        assert!(matches!(err, BackendError::Internal { .. }));
    }

    /// The `From` impl (used by bare `?` call sites) classifies too, with no
    /// context prefix — so the message is the driver text alone, as before.
    #[cfg(feature = "sqlite")]
    #[test]
    fn test_sqlite_from_impl_classifies_without_context_prefix() {
        let raw = sqlite_failure(9);
        let expected = raw.to_string();
        let err: StorageError = sqlite_failure(9).into();
        match err {
            StorageError::Backend(BackendError::Timeout { message, .. }) => {
                assert_eq!(message, expected, "empty context must add no prefix");
            }
            other => panic!("expected a classified Timeout, got {other:?}"),
        }
    }

    /// Builds a `mongodb::error::Error` carrying a server command error with
    /// the given code.
    ///
    /// `CommandError` is `#[non_exhaustive]` and has a private field, so it
    /// cannot be built with a struct literal from outside the driver — but it
    /// derives `Deserialize`, which is exactly how the driver itself builds one
    /// from a server reply. Going through serde therefore constructs the same
    /// value the driver would, rather than a test-only approximation.
    #[cfg(feature = "mongodb")]
    fn mongo_command_error(code: i32, code_name: &str) -> mongodb::error::Error {
        let command_error: mongodb::error::CommandError =
            serde_json::from_value(serde_json::json!({
                "code": code,
                "codeName": code_name,
                "errmsg": "operation exceeded time limit",
                "topologyVersion": null,
            }))
            .expect("CommandError deserializes from a server-shaped reply");
        mongodb::error::ErrorKind::Command(command_error).into()
    }

    /// Both server-side deadline codes must classify as `Timeout` so the REST
    /// layer answers 504 rather than 500.
    #[cfg(feature = "mongodb")]
    #[test]
    fn test_classify_mongodb_deadline_codes_are_timeout() {
        for (code, name) in [
            (MONGO_MAX_TIME_MS_EXPIRED, "MaxTimeMSExpired"),
            (MONGO_EXCEEDED_TIME_LIMIT, "ExceededTimeLimit"),
        ] {
            let err =
                classify_mongodb_error("Failed to execute search", mongo_command_error(code, name));
            assert!(
                matches!(err, BackendError::Timeout { ref backend_name, .. } if backend_name == "mongodb"),
                "MongoDB {name} ({code}) must classify as Timeout, got {err:?}"
            );
            // The caller's context survives classification.
            assert!(err.to_string().contains("Failed to execute search"));
        }
    }

    /// A transport failure is the server being unreachable, not a statement
    /// running long: `Unavailable` (503 + `Retry-After`), not `Timeout` (504).
    #[cfg(feature = "mongodb")]
    #[test]
    fn test_classify_mongodb_io_is_unavailable() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "connection reset");
        let err: mongodb::error::Error =
            mongodb::error::ErrorKind::Io(std::sync::Arc::new(io)).into();
        let err = classify_mongodb_error("Failed to read resource", err);
        assert!(
            matches!(err, BackendError::Unavailable { ref backend_name, .. } if backend_name == "mongodb"),
            "a MongoDB I/O failure must classify as Unavailable, got {err:?}"
        );
    }

    /// Any other command failure keeps today's behaviour exactly: `Internal`,
    /// with the same `"{context}: {err}"` text the call sites built before —
    /// the property that makes converting ~150 call sites safe.
    #[cfg(feature = "mongodb")]
    #[test]
    fn test_classify_mongodb_other_errors_are_unchanged_internal() {
        // 11000 DuplicateKey — a genuine defect at these call sites, stays 500.
        let expected = format!(
            "Failed to insert resource: {}",
            mongo_command_error(11000, "DuplicateKey")
        );
        let err = classify_mongodb_error(
            "Failed to insert resource",
            mongo_command_error(11000, "DuplicateKey"),
        );
        match err {
            BackendError::Internal {
                backend_name,
                message,
                ..
            } => {
                assert_eq!(backend_name, "mongodb");
                assert_eq!(
                    message, expected,
                    "unclassified errors must keep the pre-#353 message verbatim"
                );
            }
            other => panic!("DuplicateKey must stay Internal, got {other:?}"),
        }
    }

    /// The `From` impl (used by bare `?` call sites) classifies too, with no
    /// context prefix — so the message is the driver text alone, as before.
    #[cfg(feature = "mongodb")]
    #[test]
    fn test_mongodb_from_impl_classifies_without_context_prefix() {
        let expected =
            mongo_command_error(MONGO_MAX_TIME_MS_EXPIRED, "MaxTimeMSExpired").to_string();
        let err: StorageError =
            mongo_command_error(MONGO_MAX_TIME_MS_EXPIRED, "MaxTimeMSExpired").into();
        match err {
            StorageError::Backend(BackendError::Timeout { message, .. }) => {
                assert_eq!(message, expected, "empty context must add no prefix");
            }
            other => panic!("expected a classified Timeout, got {other:?}"),
        }
    }

    /// `QueryErrorExt` is the spelling every backend call site uses, so the
    /// classification it performs is worth pinning independently of the free
    /// functions: an `Ok` passes through untouched, and an `Err` arrives
    /// classified and context-tagged.
    #[cfg(feature = "sqlite")]
    #[test]
    fn test_query_error_ext_classifies_in_place() {
        let ok: Result<u8, rusqlite::Error> = Ok(7);
        assert_eq!(ok.or_query_error("Failed to count resources").unwrap(), 7);

        // SQLITE_INTERRUPT == 9
        let err: Result<u8, rusqlite::Error> = Err(sqlite_failure(9));
        match err.or_query_error("Failed to count resources") {
            Err(StorageError::Backend(BackendError::Timeout { message, .. })) => {
                assert!(message.starts_with("Failed to count resources: "));
            }
            other => panic!("expected a context-tagged Timeout, got {other:?}"),
        }
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
