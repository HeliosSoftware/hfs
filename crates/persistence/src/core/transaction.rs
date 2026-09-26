//! Transaction traits for ACID operations.
//!
//! This module defines traits for transactional storage operations,
//! including support for FHIR transaction and batch bundles.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mongodb"))]
use crate::error::{ConcurrencyError, StorageError};
use crate::error::{StorageResult, TransactionError};
use crate::tenant::TenantContext;
use crate::types::StoredResource;

#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mongodb"))]
use super::patch::PatchError;
use super::storage::ResourceStorage;

/// Checks the exact content a Bundle PATCH would write, while the target is
/// still inside its transaction. An error carries the complete FHIR outcome.
#[async_trait]
pub trait PatchCandidateValidator: Send + Sync {
    /// Return the full OperationOutcome when the candidate cannot be stored.
    async fn validate_patch_candidate(
        &self,
        tenant: &TenantContext,
        version: helios_fhir::FhirVersion,
        resource_type: &str,
        candidate: &Value,
    ) -> Result<(), Value>;
}

/// Render an unapplied Bundle PATCH as a typed entry refusal. The transaction
/// executors use its status and outcome after rolling back every sibling.
#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mongodb"))]
pub(crate) fn patch_failure_entry(error: PatchError) -> Box<BundleEntryResult> {
    let (status, code) = match error {
        PatchError::TestFailed { .. } => (422, "processing"),
        PatchError::UnsupportedFormat { .. } => (501, "not-supported"),
        _ => (400, "invalid"),
    };
    Box::new(BundleEntryResult::error(
        status,
        serde_json::json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": code,
                "details": {"text": error.to_string()}
            }]
        }),
    ))
}

/// Keep a PATCH update's concurrency refusal attached to its Bundle entry so
/// the transaction rolls back and returns the same status as a direct PATCH.
/// Other storage errors retain their normal backend error path.
#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mongodb"))]
pub(crate) fn patch_update_result(
    result: StorageResult<StoredResource>,
) -> StorageResult<BundleEntryResult> {
    match result {
        Ok(updated) => Ok(BundleEntryResult::updated(updated)),
        Err(error) => {
            let status = match &error {
                StorageError::Concurrency(ConcurrencyError::VersionConflict { .. }) => 409,
                StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure { .. }) => 412,
                _ => return Err(error),
            };
            Ok(BundleEntryResult::error(
                status,
                serde_json::json!({
                    "resourceType": "OperationOutcome",
                    "issue": [{
                        "severity": "error",
                        "code": "conflict",
                        "details": {"text": error.to_string()}
                    }]
                }),
            ))
        }
    }
}

/// Decode, apply and validate the candidate while its transaction remains
/// open. The wire format follows the Bundle version; path evaluation and
/// resource validation follow the stored target's FHIR version.
#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mongodb"))]
pub(crate) async fn prepare_bundle_patch(
    tenant: &TenantContext,
    resource_type: &str,
    current: &StoredResource,
    document: Option<&Value>,
    bundle_version: helios_fhir::FhirVersion,
    validator: Option<&dyn PatchCandidateValidator>,
) -> Result<Value, Box<BundleEntryResult>> {
    let document = document.ok_or_else(|| {
        patch_failure_entry(PatchError::MalformedDocument {
            format: "Bundle PATCH",
            message: "entry.resource is required".to_string(),
        })
    })?;
    let patch = super::decode_bundle_patch_resource(document, bundle_version)
        .map_err(patch_failure_entry)?;
    let candidate =
        super::apply_patch_for_version(current.content(), &patch, current.fhir_version())
            .map_err(patch_failure_entry)?;
    if let Some(validator) = validator {
        validator
            .validate_patch_candidate(tenant, current.fhir_version(), resource_type, &candidate)
            .await
            .map_err(|outcome| Box::new(BundleEntryResult::error(422, outcome)))?;
    }
    Ok(candidate)
}

/// Transaction isolation levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IsolationLevel {
    /// Read committed - sees only committed data.
    #[default]
    ReadCommitted,
    /// Repeatable read - consistent reads within transaction.
    RepeatableRead,
    /// Serializable - full isolation (may reduce concurrency).
    Serializable,
    /// Snapshot - point-in-time consistent view.
    Snapshot,
}

impl std::fmt::Display for IsolationLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IsolationLevel::ReadCommitted => write!(f, "read-committed"),
            IsolationLevel::RepeatableRead => write!(f, "repeatable-read"),
            IsolationLevel::Serializable => write!(f, "serializable"),
            IsolationLevel::Snapshot => write!(f, "snapshot"),
        }
    }
}

/// Locking strategy for concurrent access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LockingStrategy {
    /// Optimistic locking using version numbers (If-Match).
    #[default]
    Optimistic,
    /// Pessimistic locking with row-level locks.
    Pessimistic,
    /// No locking (for read-only transactions).
    None,
}

/// Options for starting a transaction.
#[derive(Debug, Clone, Default)]
pub struct TransactionOptions {
    /// The isolation level for the transaction.
    pub isolation_level: IsolationLevel,
    /// The locking strategy to use.
    pub locking_strategy: LockingStrategy,
    /// Timeout in milliseconds (0 = no timeout).
    pub timeout_ms: u64,
    /// Whether this is a read-only transaction.
    pub read_only: bool,
    /// The FHIR version resources written in this transaction are stamped
    /// with. A transaction serves one request, and a request negotiates one
    /// version, so it rides on the transaction rather than on every write.
    /// `None` falls back to the backend's configured version.
    pub fhir_version: Option<helios_fhir::FhirVersion>,
    /// Skip writing search-index and full-text rows for resources written in
    /// this transaction (bulk fast-load, #903). Stale index rows for updated
    /// resources are still deleted — a deferred index may miss a resource,
    /// never mislead about one. The caller owns rebuilding the index
    /// afterwards (`$reindex` / [`crate::search::ReindexOperation`]).
    pub defer_search_indexing: bool,
}

impl TransactionOptions {
    /// Creates new options with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the isolation level.
    pub fn isolation_level(mut self, level: IsolationLevel) -> Self {
        self.isolation_level = level;
        self
    }

    /// Sets the locking strategy.
    pub fn locking_strategy(mut self, strategy: LockingStrategy) -> Self {
        self.locking_strategy = strategy;
        self
    }

    /// Sets the timeout.
    pub fn timeout_ms(mut self, timeout: u64) -> Self {
        self.timeout_ms = timeout;
        self
    }

    /// Marks this as a read-only transaction.
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self.locking_strategy = LockingStrategy::None;
        self
    }

    /// Sets the FHIR version writes in this transaction are stamped with.
    pub fn fhir_version(mut self, version: helios_fhir::FhirVersion) -> Self {
        self.fhir_version = Some(version);
        self
    }

    /// Defers search-index and full-text writes to a later reindex (#903).
    pub fn defer_search_indexing(mut self, defer: bool) -> Self {
        self.defer_search_indexing = defer;
        self
    }
}

/// A database transaction.
///
/// This trait represents an active transaction that can perform CRUD operations
/// atomically. Changes are only persisted when `commit()` is called.
///
/// # Example
///
/// ```ignore
/// use helios_persistence::core::{TransactionProvider, Transaction};
///
/// async fn transfer_care<S: TransactionProvider>(
///     storage: &S,
///     tenant: &TenantContext,
/// ) -> Result<(), StorageError> {
///     let mut tx = storage.begin_transaction(tenant, TransactionOptions::new()).await?;
///
///     // Read patient
///     let patient = tx.read("Patient", "123").await?
///         .ok_or(StorageError::Resource(ResourceError::NotFound { ... }))?;
///
///     // Update patient
///     let mut content = patient.content().clone();
///     content["generalPractitioner"] = json!([{"reference": "Practitioner/456"}]);
///     tx.update(&patient, content).await?;
///
///     // Create an encounter
///     tx.create("Encounter", json!({
///         "resourceType": "Encounter",
///         "subject": {"reference": "Patient/123"}
///     })).await?;
///
///     // Commit all changes
///     tx.commit().await?;
///
///     Ok(())
/// }
/// ```
#[async_trait]
pub trait Transaction: Send + Sync {
    /// Creates a new resource within this transaction.
    async fn create(
        &mut self,
        resource_type: &str,
        resource: Value,
    ) -> StorageResult<StoredResource>;

    /// Reads a resource within this transaction.
    ///
    /// This sees uncommitted changes made within this transaction.
    async fn read(
        &mut self,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>>;

    /// Updates a resource within this transaction.
    async fn update(
        &mut self,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource>;

    /// Deletes a resource within this transaction.
    async fn delete(&mut self, resource_type: &str, id: &str) -> StorageResult<()>;

    /// Commits the transaction, persisting all changes.
    ///
    /// After calling this, the transaction is consumed and cannot be used again.
    async fn commit(self: Box<Self>) -> StorageResult<()>;

    /// Rolls back the transaction, discarding all changes.
    ///
    /// After calling this, the transaction is consumed and cannot be used again.
    async fn rollback(self: Box<Self>) -> StorageResult<()>;

    /// Returns the tenant context for this transaction.
    fn tenant(&self) -> &TenantContext;

    /// Returns whether this transaction is still active.
    fn is_active(&self) -> bool;
}

/// Provider for transaction support.
///
/// Backends that support ACID transactions implement this trait.
#[async_trait]
pub trait TransactionProvider: ResourceStorage {
    /// The transaction type returned by this provider.
    type Transaction: Transaction;

    /// Begins a new transaction.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for operations in this transaction
    /// * `options` - Transaction options (isolation level, timeout, etc.)
    ///
    /// # Returns
    ///
    /// An active transaction that must be committed or rolled back.
    ///
    /// # Errors
    ///
    /// * `StorageError::Transaction(UnsupportedIsolationLevel)` - If isolation level not supported
    /// * `StorageError::Backend` - If connection cannot be acquired
    async fn begin_transaction(
        &self,
        tenant: &TenantContext,
        options: TransactionOptions,
    ) -> StorageResult<Self::Transaction>;

    /// Executes a function within a transaction.
    ///
    /// This is a convenience method that handles commit/rollback automatically.
    /// If the function returns Ok, the transaction is committed.
    /// If the function returns Err or panics, the transaction is rolled back.
    ///
    /// # Example
    ///
    /// ```ignore
    /// storage.with_transaction(&tenant, TransactionOptions::new(), |tx| async move {
    ///     let patient = tx.read("Patient", "123").await?;
    ///     // ... more operations
    ///     Ok(())
    /// }).await?;
    /// ```
    async fn with_transaction<F, Fut, R>(
        &self,
        tenant: &TenantContext,
        options: TransactionOptions,
        f: F,
    ) -> StorageResult<R>
    where
        F: FnOnce(Self::Transaction) -> Fut + Send,
        Fut: std::future::Future<Output = StorageResult<R>> + Send,
        R: Send,
    {
        let tx = self.begin_transaction(tenant, options).await?;
        f(tx).await
    }
}

/// Entry in a FHIR transaction or batch bundle.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BundleEntry {
    /// The HTTP method for this entry.
    #[serde(default)]
    pub method: BundleMethod,
    /// The resource URL (relative or absolute).
    #[serde(default)]
    pub url: String,
    /// The resource content (for POST, PUT, PATCH).
    #[serde(default)]
    pub resource: Option<Value>,
    /// If-Match header value for conditional operations.
    #[serde(default)]
    pub if_match: Option<String>,
    /// If-None-Match header value for conditional creates.
    #[serde(default)]
    pub if_none_match: Option<String>,
    /// If-None-Exist header for conditional creates.
    #[serde(default)]
    pub if_none_exist: Option<String>,
    /// The fullUrl for this entry, used for reference resolution.
    /// Typically a urn:uuid: for new resources in transactions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_url: Option<String>,
}

/// HTTP method for bundle entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "UPPERCASE")]
pub enum BundleMethod {
    /// GET - Read operation.
    #[default]
    Get,
    /// POST - Create operation.
    Post,
    /// PUT - Update or create operation.
    Put,
    /// PATCH - Partial update operation.
    Patch,
    /// DELETE - Delete operation.
    Delete,
}

impl std::fmt::Display for BundleMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BundleMethod::Get => write!(f, "GET"),
            BundleMethod::Post => write!(f, "POST"),
            BundleMethod::Put => write!(f, "PUT"),
            BundleMethod::Patch => write!(f, "PATCH"),
            BundleMethod::Delete => write!(f, "DELETE"),
        }
    }
}

/// What a bundle entry actually did to stored state, independent of its HTTP
/// `status` (#1078).
///
/// The status alone cannot say it: a `200` is both a read and an update, and a
/// `204` is both a delete and — on some backends — a delete of a resource that
/// was not there. Consumers that count live resources read this instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BundleEntryEffect {
    /// A new live resource was stored.
    Created,
    /// A new version of an existing live resource was stored.
    Updated,
    /// A live resource was deleted.
    Deleted,
    /// A delete found nothing to delete (absent or already deleted, or a
    /// conditional delete without a match).
    NotFound,
    /// Nothing was written: a conditional create matched an existing resource.
    NoOp,
    /// The entry only read (a read or a search).
    #[default]
    Read,
    /// The entry failed.
    Failed,
}

impl BundleEntryEffect {
    /// Net change in live resources: `+1` created, `-1` deleted, else `0`.
    pub fn live_count_delta(self) -> i64 {
        match self {
            BundleEntryEffect::Created => 1,
            BundleEntryEffect::Deleted => -1,
            _ => 0,
        }
    }

    /// Whether stored state changed.
    pub fn is_write(self) -> bool {
        matches!(
            self,
            BundleEntryEffect::Created | BundleEntryEffect::Updated | BundleEntryEffect::Deleted
        )
    }
}

/// Result of a bundle entry execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleEntryResult {
    /// HTTP status code.
    pub status: u16,
    /// Location header (for creates).
    pub location: Option<String>,
    /// ETag header.
    pub etag: Option<String>,
    /// Last-Modified header.
    pub last_modified: Option<String>,
    /// Response resource (for reads, creates, updates).
    pub resource: Option<Value>,
    /// OperationOutcome for errors.
    pub outcome: Option<Value>,
    /// What the entry actually did to stored state; see [`BundleEntryEffect`].
    #[serde(default)]
    pub effect: BundleEntryEffect,
}

impl BundleEntryResult {
    /// Creates a successful result for a create operation.
    pub fn created(resource: StoredResource) -> Self {
        Self {
            status: 201,
            location: Some(resource.versioned_url()),
            etag: Some(resource.etag().to_string()),
            last_modified: Some(resource.last_modified().to_rfc3339()),
            resource: Some(resource.content_with_meta()),
            outcome: None,
            effect: BundleEntryEffect::Created,
        }
    }

    /// Creates a successful result for a read operation.
    pub fn ok(resource: StoredResource) -> Self {
        Self {
            status: 200,
            location: None,
            etag: Some(resource.etag().to_string()),
            last_modified: Some(resource.last_modified().to_rfc3339()),
            resource: Some(resource.content_with_meta()),
            outcome: None,
            effect: BundleEntryEffect::Read,
        }
    }

    /// Creates a successful result for an update that stored a new version of
    /// an existing live resource.
    ///
    /// Same `200` shape as [`ok`](Self::ok); only the effect differs.
    pub fn updated(resource: StoredResource) -> Self {
        Self {
            effect: BundleEntryEffect::Updated,
            ..Self::ok(resource)
        }
    }

    /// Creates the result for a conditional create (`ifNoneExist`) that
    /// matched exactly one existing resource, so nothing was written.
    ///
    /// Answers `200` with the match, and sets `location` to its versioned URL
    /// even though nothing was created: transaction loops map a POST entry's
    /// `fullUrl` to `Type/id` from `location`, and references to a
    /// conditionally created entry must resolve to the match.
    pub fn matched_existing(resource: StoredResource) -> Self {
        let location = resource.versioned_url();
        Self {
            location: Some(location),
            effect: BundleEntryEffect::NoOp,
            ..Self::ok(resource)
        }
    }

    /// Creates a result for a delete operation that removed a live resource.
    pub fn deleted() -> Self {
        Self {
            status: 204,
            location: None,
            etag: None,
            last_modified: None,
            resource: None,
            outcome: None,
            effect: BundleEntryEffect::Deleted,
        }
    }

    /// Creates the result for a delete that found nothing to delete (the
    /// resource is absent or already deleted).
    ///
    /// Same `204` as [`deleted`](Self::deleted) — deletes are idempotent on
    /// the wire — but the effect records that no live resource went away.
    pub fn delete_not_found() -> Self {
        Self {
            effect: BundleEntryEffect::NotFound,
            ..Self::deleted()
        }
    }

    /// Creates an error result.
    pub fn error(status: u16, outcome: Value) -> Self {
        Self {
            status,
            location: None,
            etag: None,
            last_modified: None,
            resource: None,
            outcome: Some(outcome),
            effect: BundleEntryEffect::Failed,
        }
    }
}

/// Result of processing a transaction or batch bundle.
#[derive(Debug, Clone)]
pub struct BundleResult {
    /// The bundle type.
    pub bundle_type: BundleType,
    /// Results for each entry.
    pub entries: Vec<BundleEntryResult>,
}

/// Type of bundle operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleType {
    /// Transaction - all-or-nothing semantics.
    Transaction,
    /// Batch - independent operations.
    Batch,
}

/// Provider for FHIR `transaction` bundle operations.
///
/// # Why `batch` is not here
///
/// This trait once carried a `process_batch` sibling, implemented by all five
/// backends and called by none of them: the REST layer runs its own entry loop
/// (`helios_rest::handlers::batch`). That is not an oversight to be corrected
/// by wiring the two together — batch requires two things this tier cannot see.
/// Each entry is authorized individually against the request's SMART scopes,
/// and each entry emits its own audit event; `POST [base]` has no other
/// authorization gate, so moving execution down here would move the only check
/// into a crate that has no notion of a principal.
///
/// A transaction has no such split, because it succeeds or fails as a unit and
/// is scope-checked as a unit before it is handed over.
///
/// Five unreachable copies were deleted in #501 rather than left to accumulate
/// fixes — #311's `ifMatch` handling had already landed in the half nothing
/// calls, leaving the behaviour broken on the wire for two releases.
#[async_trait]
pub trait BundleProvider: ResourceStorage {
    /// Whether this provider can honour FHIR transaction atomicity.
    ///
    /// A `transaction` bundle is all-or-nothing; a `batch` is not. Design
    /// discussion #28 draws the line at the trait boundary — "code that
    /// requires atomicity takes `&dyn TransactionProvider`, while code that can
    /// tolerate partial failures takes `&dyn ResourceStorage`" — but
    /// `process_transaction` lives here on `BundleProvider`, which every
    /// backend implements regardless of whether it can roll back. That let the
    /// S3 backend accept transaction bundles it could not unwind: a request
    /// cancelled by the HTTP timeout left 466 of 473 entries durably committed
    /// while the client was told the transaction failed (#489).
    ///
    /// This method restores the gate at the only place that can see the answer.
    /// It is deliberately **required, not defaulted**: a new backend must state
    /// its position rather than inherit one, because the wrong default here is
    /// silent data corruption in one direction and a needless 422 in the other.
    ///
    /// Returning `false` does not disable bundles — `process_batch` remains
    /// available, which is precisely what #28 prescribes for a backend without
    /// transaction support.
    fn supports_atomic_transactions(&self) -> bool;

    /// Processes a transaction bundle (all-or-nothing).
    ///
    /// All entries are processed atomically. If any entry fails,
    /// all changes are rolled back.
    ///
    /// Implementations that return `false` from
    /// [`supports_atomic_transactions`](Self::supports_atomic_transactions)
    /// must reject the call before performing any write, rather than making a
    /// best-effort attempt.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context
    /// * `entries` - The bundle entries to process
    /// * `fhir_version` - The version created/updated resources are stamped
    ///   with — the request's negotiated version (one bundle, one version)
    ///
    /// # Returns
    ///
    /// Results for each entry. On failure, all entries will have error status.
    async fn process_transaction(
        &self,
        tenant: &TenantContext,
        entries: Vec<BundleEntry>,
        fhir_version: helios_fhir::FhirVersion,
    ) -> Result<BundleResult, TransactionError> {
        self.process_transaction_with_patch_validator(tenant, entries, fhir_version, None)
            .await
    }

    /// Transaction execution with a write-path check on each patched
    /// candidate, after the in-transaction read and before the update.
    async fn process_transaction_with_patch_validator(
        &self,
        tenant: &TenantContext,
        entries: Vec<BundleEntry>,
        fhir_version: helios_fhir::FhirVersion,
        validator: Option<&dyn PatchCandidateValidator>,
    ) -> Result<BundleResult, TransactionError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use helios_fhir::FhirVersion;

    #[test]
    fn test_isolation_level_display() {
        assert_eq!(IsolationLevel::ReadCommitted.to_string(), "read-committed");
        assert_eq!(IsolationLevel::Serializable.to_string(), "serializable");
    }

    #[test]
    fn test_transaction_options_builder() {
        let opts = TransactionOptions::new()
            .isolation_level(IsolationLevel::Serializable)
            .timeout_ms(5000);

        assert_eq!(opts.isolation_level, IsolationLevel::Serializable);
        assert_eq!(opts.timeout_ms, 5000);
    }

    #[test]
    fn test_transaction_options_read_only() {
        let opts = TransactionOptions::new().read_only();

        assert!(opts.read_only);
        assert_eq!(opts.locking_strategy, LockingStrategy::None);
    }

    #[test]
    fn test_bundle_method_display() {
        assert_eq!(BundleMethod::Get.to_string(), "GET");
        assert_eq!(BundleMethod::Post.to_string(), "POST");
        assert_eq!(BundleMethod::Delete.to_string(), "DELETE");
    }

    #[test]
    fn test_bundle_entry_result_created() {
        let resource = StoredResource::new(
            "Patient",
            "123",
            crate::tenant::TenantId::new("t1"),
            serde_json::json!({}),
            FhirVersion::default(),
        );

        let result = BundleEntryResult::created(resource);
        assert_eq!(result.status, 201);
        assert!(result.location.is_some());
        assert!(result.etag.is_some());
    }

    #[test]
    fn test_bundle_entry_result_error() {
        let outcome = serde_json::json!({
            "resourceType": "OperationOutcome",
            "issue": [{"severity": "error", "code": "not-found"}]
        });

        let result = BundleEntryResult::error(404, outcome);
        assert_eq!(result.status, 404);
        assert!(result.outcome.is_some());
        assert!(result.resource.is_none());
        assert_eq!(result.effect, BundleEntryEffect::Failed);
    }

    #[cfg(any(feature = "sqlite", feature = "postgres", feature = "mongodb"))]
    #[test]
    fn patch_update_result_preserves_conflicts_and_other_errors() {
        let version = patch_update_result(Err(StorageError::Concurrency(
            ConcurrencyError::VersionConflict {
                resource_type: "Patient".to_string(),
                id: "123".to_string(),
                expected_version: "1".to_string(),
                actual_version: "2".to_string(),
            },
        )))
        .unwrap();
        assert_eq!(version.status, 409);
        assert_eq!(version.effect, BundleEntryEffect::Failed);
        assert_eq!(
            version.outcome.as_ref().unwrap()["issue"][0]["code"],
            "conflict"
        );
        assert!(
            version.outcome.unwrap()["issue"][0]["details"]["text"]
                .as_str()
                .unwrap()
                .contains("expected 1, found 2")
        );

        let etag = patch_update_result(Err(StorageError::Concurrency(
            ConcurrencyError::OptimisticLockFailure {
                resource_type: "Patient".to_string(),
                id: "123".to_string(),
                expected_etag: "W/\"1\"".to_string(),
                actual_etag: Some("W/\"2\"".to_string()),
            },
        )))
        .unwrap();
        assert_eq!(etag.status, 412);
        assert_eq!(etag.outcome.unwrap()["issue"][0]["code"], "conflict");

        let other = patch_update_result(Err(StorageError::Resource(
            crate::error::ResourceError::NotFound {
                resource_type: "Patient".to_string(),
                id: "123".to_string(),
            },
        )));
        assert!(matches!(
            other,
            Err(StorageError::Resource(
                crate::error::ResourceError::NotFound { .. }
            ))
        ));
    }

    fn stored_patient() -> StoredResource {
        StoredResource::new(
            "Patient",
            "123",
            crate::tenant::TenantId::new("t1"),
            serde_json::json!({"resourceType": "Patient", "id": "123"}),
            FhirVersion::default(),
        )
    }

    #[test]
    fn test_bundle_entry_result_constructor_effects() {
        let created = BundleEntryResult::created(stored_patient());
        assert_eq!(
            (created.status, created.effect),
            (201, BundleEntryEffect::Created)
        );

        let read = BundleEntryResult::ok(stored_patient());
        assert_eq!((read.status, read.effect), (200, BundleEntryEffect::Read));
        assert!(read.location.is_none());

        let deleted = BundleEntryResult::deleted();
        assert_eq!(
            (deleted.status, deleted.effect),
            (204, BundleEntryEffect::Deleted)
        );

        let failed = BundleEntryResult::error(412, serde_json::json!({}));
        assert_eq!(
            (failed.status, failed.effect),
            (412, BundleEntryEffect::Failed)
        );
    }

    #[test]
    fn test_bundle_entry_result_updated_matches_ok_shape() {
        let read = BundleEntryResult::ok(stored_patient());
        let updated = BundleEntryResult::updated(stored_patient());
        assert_eq!(updated.status, 200);
        assert_eq!(updated.effect, BundleEntryEffect::Updated);
        assert!(updated.location.is_none());
        assert_eq!(updated.etag, read.etag);
        assert_eq!(updated.resource, read.resource);
        assert!(updated.last_modified.is_some());
        assert!(updated.outcome.is_none());
    }

    #[test]
    fn test_bundle_entry_result_matched_existing() {
        let resource = stored_patient();
        let expected_location = resource.versioned_url();
        let matched = BundleEntryResult::matched_existing(resource);
        assert_eq!(matched.status, 200);
        assert_eq!(matched.effect, BundleEntryEffect::NoOp);
        assert_eq!(matched.location, Some(expected_location));
        assert!(matched.etag.is_some());
        assert!(matched.resource.is_some());
        assert!(matched.outcome.is_none());
    }

    #[test]
    fn test_bundle_entry_result_delete_not_found() {
        let result = BundleEntryResult::delete_not_found();
        assert_eq!(result.status, 204);
        assert_eq!(result.effect, BundleEntryEffect::NotFound);
        assert!(result.location.is_none());
        assert!(result.etag.is_none());
        assert!(result.last_modified.is_none());
        assert!(result.resource.is_none());
        assert!(result.outcome.is_none());
    }

    #[test]
    fn test_bundle_entry_effect_delta_and_is_write() {
        let table = [
            (BundleEntryEffect::Created, 1, true),
            (BundleEntryEffect::Updated, 0, true),
            (BundleEntryEffect::Deleted, -1, true),
            (BundleEntryEffect::NotFound, 0, false),
            (BundleEntryEffect::NoOp, 0, false),
            (BundleEntryEffect::Read, 0, false),
            (BundleEntryEffect::Failed, 0, false),
        ];
        for (effect, delta, is_write) in table {
            assert_eq!(effect.live_count_delta(), delta, "{effect:?} delta");
            assert_eq!(effect.is_write(), is_write, "{effect:?} is_write");
        }
        assert_eq!(BundleEntryEffect::default(), BundleEntryEffect::Read);
    }

    #[test]
    fn test_bundle_entry_result_effect_defaults_when_absent() {
        let result: BundleEntryResult = serde_json::from_value(serde_json::json!({
            "status": 200,
            "location": null,
            "etag": null,
            "last_modified": null,
            "resource": null,
            "outcome": null
        }))
        .unwrap();
        assert_eq!(result.effect, BundleEntryEffect::Read);
    }
}
