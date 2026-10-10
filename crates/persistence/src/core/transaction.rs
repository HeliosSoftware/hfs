//! Transaction traits for ACID operations.
//!
//! This module defines traits for transactional storage operations,
//! including support for FHIR transaction and batch bundles.

#[cfg(any(feature = "sqlite", feature = "mongodb"))]
use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mongodb"))]
use crate::error::ConcurrencyError;
use crate::error::{StorageError, StorageResult, TransactionError};
use crate::tenant::TenantContext;
use crate::types::{SearchParameter, StoredResource, new_resource_id};

#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mongodb"))]
use super::patch::PatchError;
use super::storage::{
    ConditionalCreateResult, ConditionalDeleteResult, ConditionalUpdateResult, ResourceStorage,
};

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

/// Conditional interactions inside an open transaction.
///
/// [`Transaction`] addresses resources by id. FHIR's conditional interactions
/// address them by search criteria, and a transaction Bundle may carry them
/// (`PUT [type]?[criteria]`, `DELETE [type]?[criteria]`, `ifNoneExist`). The
/// backend-level [`ConditionalStorage`](super::storage::ConditionalStorage)
/// cannot serve those: it searches and writes through the backend's own
/// connection, outside the open transaction, so a bundle that used it would
/// commit one entry while the rest could still roll back (#859).
///
/// This is a separate trait rather than a widening of [`Transaction`] — the
/// shape design discussion #28 proposed — so a backend adopts it once it has a
/// transaction-scoped search, instead of every implementor of the base trait
/// being widened at once. Only [`find_matching`](Self::find_matching) is
/// required: it is the search surface, resolving criteria against *this
/// transaction's* view, earlier writes of the same transaction included. The
/// three interactions are defaulted on top of it and [`Transaction`]'s
/// id-addressed writes, answering with the same outcome enums the
/// backend-level trait returns.
///
/// `criteria` is typed, not a `k=v&k=v` string: the caller has parsed and
/// validated it with the search parser, so modifiers and chains reach the
/// query builder intact (#861, #865). Empty criteria match nothing — matching
/// everything would be the literal reading, but no conditional interaction
/// means that.
#[async_trait]
pub trait ConditionalTransaction: Transaction {
    /// Resolves `criteria` against `resource_type` as this transaction sees
    /// it.
    ///
    /// Returns every match up to the backend's conditional match limit, so a
    /// caller can distinguish none, one, and several.
    async fn find_matching(
        &mut self,
        resource_type: &str,
        criteria: &[SearchParameter],
    ) -> StorageResult<Vec<StoredResource>>;

    /// Conditional create: creates `resource` only when `criteria` match
    /// nothing; one match is answered as it stands.
    async fn create_if_none_exist(
        &mut self,
        resource_type: &str,
        resource: Value,
        criteria: &[SearchParameter],
    ) -> StorageResult<ConditionalCreateResult> {
        let mut matches = self.find_matching(resource_type, criteria).await?;
        match matches.len() {
            0 => Ok(ConditionalCreateResult::Created(
                self.create(resource_type, resource).await?,
            )),
            1 => Ok(ConditionalCreateResult::Exists(matches.remove(0))),
            n => Ok(ConditionalCreateResult::MultipleMatches(n)),
        }
    }

    /// Conditional update with upsert: one match is updated, no match creates
    /// `resource`, several matches are refused.
    async fn update_conditional(
        &mut self,
        resource_type: &str,
        resource: Value,
        criteria: &[SearchParameter],
    ) -> StorageResult<ConditionalUpdateResult> {
        let mut matches = self.find_matching(resource_type, criteria).await?;
        match matches.len() {
            0 => Ok(ConditionalUpdateResult::Created(
                self.create(resource_type, resource).await?,
            )),
            1 => {
                let existing = matches.remove(0);
                Ok(ConditionalUpdateResult::Updated(
                    self.update(&existing, resource).await?,
                ))
            }
            n => Ok(ConditionalUpdateResult::MultipleMatches(n)),
        }
    }

    /// Conditional delete: one match is deleted, no match is not an error,
    /// several matches are refused (this server elects
    /// `conditionalDelete: "single"`).
    async fn delete_conditional(
        &mut self,
        resource_type: &str,
        criteria: &[SearchParameter],
    ) -> StorageResult<ConditionalDeleteResult> {
        let mut matches = self.find_matching(resource_type, criteria).await?;
        match matches.len() {
            0 => Ok(ConditionalDeleteResult::NoMatch),
            1 => {
                let existing = matches.remove(0);
                self.delete(resource_type, existing.id()).await?;
                Ok(ConditionalDeleteResult::Deleted(existing))
            }
            n => Ok(ConditionalDeleteResult::MultipleMatches(n)),
        }
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
    /// Typed criteria of a `PUT [type]?[criteria]` or `DELETE [type]?[criteria]`
    /// entry (#859); `None` for every other entry.
    ///
    /// Set by the caller, which percent-decodes and parses the entry URL's
    /// query with the same parser the search endpoint uses, so modifiers,
    /// chains and prefixes reach the backend typed rather than as a `k=v&k=v`
    /// string it would have to re-parse (#861, #865). `url` keeps its
    /// `[type]?[criteria]` form for audit and messages; a transaction executor
    /// takes the type from the text before `?` and never routes such a URL
    /// through its instance parser.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criteria: Option<Vec<SearchParameter>>,
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

impl BundleEntry {
    /// Pins the `Type/id` this POST or PUT entry will write under, so its
    /// `fullUrl` can be resolved before any entry executes (see
    /// `pin_forward_references`). `matched_id` is the resource the
    /// entry's criteria selected, if any; without one the body's own id is
    /// used, or a new id is minted into the body for the create to write under.
    /// An entry that would create without an object body is rejected here with
    /// the validation error the write itself would raise, so no id is ever
    /// minted into a body the caller did not send.
    pub fn pin_reference(
        &mut self,
        resource_type: &str,
        matched_id: Option<&str>,
    ) -> StorageResult<String> {
        let id = match matched_id {
            Some(id) => id.to_string(),
            None => match self.resource.as_mut() {
                Some(Value::Object(body)) => match body.get("id").and_then(Value::as_str) {
                    Some(id) if !id.is_empty() => id.to_string(),
                    _ => {
                        let id = new_resource_id();
                        body.insert("id".to_string(), Value::String(id.clone()));
                        id
                    }
                },
                Some(_) => {
                    return Err(StorageError::Validation(
                        crate::error::ValidationError::InvalidResource {
                            message: "Bundle entry resource must be a JSON object".to_string(),
                            details: Vec::new(),
                        },
                    ));
                }
                None => {
                    return Err(StorageError::Validation(
                        crate::error::ValidationError::MissingRequiredField {
                            field: "resource".to_string(),
                        },
                    ));
                }
            },
        };
        Ok(format!("{resource_type}/{id}"))
    }
}

/// Where a PUT, PATCH or DELETE entry is addressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleEntryTarget {
    /// `Type/id`. A query on an instance URL qualifies the address, as it
    /// does on the instance endpoints, and is ignored.
    Instance {
        /// The addressed resource type.
        resource_type: String,
        /// The addressed logical id.
        id: String,
    },
    /// `Type?criteria`: a conditional interaction whose target is found by
    /// searching inside the transaction.
    Conditional {
        /// The resource type the criteria select from.
        resource_type: String,
        /// The raw query string, decoded by the shared criteria builder.
        criteria: String,
    },
}

impl BundleEntryTarget {
    /// The resource type of either target shape.
    pub fn resource_type(&self) -> &str {
        match self {
            Self::Instance { resource_type, .. } | Self::Conditional { resource_type, .. } => {
                resource_type
            }
        }
    }
}

/// Splits an entry URL into its target before a backend's `Type/id` parser
/// sees it. Those parsers split on `/` alone, so criteria would otherwise be
/// stored as part of the resource type (#503). Accepts the same shapes as
/// the parsers: relative, leading slash, or absolute with a base path.
pub fn parse_bundle_entry_target(url: &str) -> StorageResult<BundleEntryTarget> {
    let (path, query) = match url.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (url, None),
    };
    let path = path
        .strip_prefix("http://")
        .or_else(|| path.strip_prefix("https://"))
        .map(|rest| rest.find('/').map_or("", |start| &rest[start..]))
        .unwrap_or(path);
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let invalid = |message: &str| {
        StorageError::Validation(crate::error::ValidationError::InvalidReference {
            reference: url.to_string(),
            message: message.to_string(),
        })
    };
    // A known penultimate resource type identifies an instance, even when its
    // ID is itself a resource-type name (Patient/Observation?_format=json).
    let types = crate::search::ResourceTypeScope::any_enabled();
    let is_instance = segments.len() >= 2 && types.contains(segments[segments.len() - 2]);
    if let (Some(criteria), Some(resource_type)) = (query, segments.last())
        && !is_instance
        && (segments.len() == 1 || types.contains(resource_type))
    {
        if crate::search::parse_conditional_criteria(criteria).is_empty() {
            return Err(invalid("Conditional URL carries no usable criteria"));
        }
        return Ok(BundleEntryTarget::Conditional {
            resource_type: (*resource_type).to_string(),
            criteria: criteria.to_string(),
        });
    }
    match (segments.as_slice(), query) {
        ([.., resource_type, id], _) => Ok(BundleEntryTarget::Instance {
            resource_type: resource_type.to_string(),
            id: id.to_string(),
        }),
        _ => Err(invalid(
            "URL must be in format ResourceType/id or ResourceType?criteria",
        )),
    }
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

    /// The `Type/id` a later `urn:uuid` reference to this entry resolves to:
    /// the location without its version, or the returned resource's identity
    /// when an update answers 200 without a location.
    pub fn reference(&self) -> Option<String> {
        if let Some(location) = &self.location {
            let reference = location.split("/_history").next().unwrap_or(location);
            return Some(reference.to_string());
        }
        let resource = self.resource.as_ref()?;
        let resource_type = resource.get("resourceType")?.as_str()?;
        let id = resource.get("id")?.as_str()?;
        Some(format!("{resource_type}/{id}"))
    }
}

/// Tracks resolved identities inside one atomic bundle. No-op conditional
/// creates may share an identity; two writes may not. A changed conditional
/// target cannot invalidate references already written by an earlier entry.
#[cfg(any(feature = "sqlite", feature = "mongodb"))]
pub(crate) struct BundleTransactionState {
    references: HashMap<String, String>,
    used_references: HashSet<String>,
    written: HashSet<String>,
}

#[cfg(any(feature = "sqlite", feature = "mongodb"))]
impl BundleTransactionState {
    pub(crate) fn new(references: HashMap<String, String>) -> Self {
        Self {
            references,
            used_references: HashSet::new(),
            written: HashSet::new(),
        }
    }

    pub(crate) fn resolve(&self, value: &mut Value) -> HashSet<String> {
        fn visit(
            value: &mut Value,
            references: &HashMap<String, String>,
            used: &mut HashSet<String>,
        ) {
            match value {
                Value::Object(map) => {
                    if let Some(Value::String(reference)) = map.get_mut("reference")
                        && reference.starts_with("urn:uuid:")
                        && let Some(resolved) = references.get(reference)
                    {
                        used.insert(reference.clone());
                        *reference = resolved.clone();
                    }
                    for nested in map.values_mut() {
                        visit(nested, references, used);
                    }
                }
                Value::Array(values) => {
                    for nested in values {
                        visit(nested, references, used);
                    }
                }
                _ => {}
            }
        }
        let mut used = HashSet::new();
        visit(value, &self.references, &mut used);
        used
    }

    pub(crate) fn record(
        &mut self,
        entry: &BundleEntry,
        result: &BundleEntryResult,
        delete_target: Option<String>,
        used_references: HashSet<String>,
    ) -> Result<(), String> {
        let reference = result.reference();
        let target = if result.effect.is_write() {
            reference.clone().or(delete_target)
        } else {
            delete_target
        };
        if let Some(target) = target
            && !self.written.insert(target.clone())
        {
            return Err(format!("Transaction entries overlap on resource {target}"));
        }
        if result.effect.is_write() {
            self.used_references.extend(used_references);
        }
        if matches!(entry.method, BundleMethod::Post | BundleMethod::Put)
            && let Some(full_url) = &entry.full_url
            && let Some(reference) = reference
        {
            if self
                .references
                .get(full_url)
                .is_some_and(|pinned| pinned != &reference)
                && self.used_references.contains(full_url)
            {
                return Err(format!(
                    "Conditional target for {full_url} changed after a bundle reference was written"
                ));
            }
            self.references.insert(full_url.clone(), reference);
        }
        Ok(())
    }
}

/// The in-transaction search reference pinning uses to resolve a POST
/// entry's `ifNoneExist` before any entry executes.
#[cfg(any(feature = "sqlite", feature = "mongodb"))]
#[async_trait]
pub(crate) trait BundleMatchSource: Send {
    /// The resources `criteria` select inside the open transaction, or `None`
    /// when this backend cannot search there; the entry's own arm then
    /// refuses it.
    async fn find_matches(
        &mut self,
        resource_type: &str,
        criteria: &str,
    ) -> StorageResult<Option<Vec<StoredResource>>>;
}

/// Pins the `Type/id` of each POST or PUT entry whose `fullUrl` is referenced
/// by itself or by an entry that executes before it. Entries run in DELETE,
/// POST, PUT order, so such a reference would otherwise reach storage as the
/// literal `urn:uuid`. Every other `fullUrl` is resolved by
/// [`BundleTransactionState::record`] once its entry has run, so only forward
/// references pay for a pin, and only a forward-referenced `ifNoneExist`
/// searches twice (here and when its entry executes).
///
/// An instance PUT pins its URL id and a conditional PUT its resolved target;
/// otherwise [`BundleEntry::pin_reference`] uses the body id or mints one.
#[cfg(any(feature = "sqlite", feature = "mongodb"))]
pub(crate) async fn pin_forward_references(
    entries: &mut [BundleEntry],
    targets: &HashMap<usize, super::bundle_conditionals::ConditionalTarget>,
    source: &mut dyn BundleMatchSource,
) -> Result<HashMap<String, String>, (usize, StorageError)> {
    fn visit<'a>(value: &'a Value, found: &mut Vec<&'a str>) {
        match value {
            Value::Object(map) => {
                if let Some(Value::String(reference)) = map.get("reference")
                    && reference.starts_with("urn:uuid:")
                {
                    found.push(reference);
                }
                map.values().for_each(|nested| visit(nested, found));
            }
            Value::Array(values) => values.iter().for_each(|nested| visit(nested, found)),
            _ => {}
        }
    }
    let mut first_use: HashMap<String, usize> = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        let mut found = Vec::new();
        if let Some(resource) = &entry.resource {
            visit(resource, &mut found);
        }
        for reference in found {
            first_use.entry(reference.to_string()).or_insert(index);
        }
    }

    let mut references = HashMap::new();
    for (index, entry) in entries.iter_mut().enumerate() {
        let Some(full_url) = entry.full_url.clone() else {
            continue;
        };
        if !first_use.get(&full_url).is_some_and(|&user| user <= index) {
            continue;
        }
        let pinned = match entry.method {
            BundleMethod::Put => match targets.get(&index) {
                Some(target) => entry.pin_reference(
                    &target.resource_type,
                    target.resolved.as_ref().map(|resource| resource.id()),
                ),
                None => match parse_bundle_entry_target(&entry.url) {
                    Ok(BundleEntryTarget::Instance { resource_type, id }) => {
                        entry.pin_reference(&resource_type, Some(&id))
                    }
                    // The entry's own arm reports a URL it cannot address.
                    _ => continue,
                },
            },
            BundleMethod::Post => {
                let Some(resource_type) = entry
                    .resource
                    .as_ref()
                    .and_then(|resource| resource.get("resourceType"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                else {
                    continue;
                };
                let matched = match entry.if_none_exist.as_deref() {
                    None => None,
                    Some(criteria) => {
                        match source
                            .find_matches(&resource_type, criteria)
                            .await
                            .map_err(|e| (index, e))?
                            .as_deref()
                        {
                            None => continue,
                            Some([]) => None,
                            Some([only]) => Some(only.id().to_string()),
                            // Several matches fail the entry when it runs.
                            Some(_) => continue,
                        }
                    }
                };
                entry.pin_reference(&resource_type, matched.as_deref())
            }
            _ => continue,
        };
        references.insert(full_url, pinned.map_err(|e| (index, e))?);
    }
    Ok(references)
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

    /// Whether this provider resolves conditional interactions inside the
    /// transaction it opens for a Bundle (#28's
    /// `supports_conditional_in_transaction`; #859).
    ///
    /// Covers `PUT [type]?[criteria]`, `DELETE [type]?[criteria]` and
    /// `ifNoneExist`. A backend whose local search index is empty because
    /// search is offloaded to a secondary (composite SQLite/PostgreSQL +
    /// Elasticsearch) answers `false`: its transaction-scoped search would
    /// find nothing, and "no match" on a conditional write is a create, so
    /// the bundle would duplicate exactly what the criteria exist to prevent.
    /// The REST layer consults this before anything executes, so such a
    /// bundle is declined intact with `501` rather than failing at the entry.
    ///
    /// Required, not defaulted, for the reason given on
    /// [`supports_atomic_transactions`](Self::supports_atomic_transactions).
    fn supports_conditional_in_transaction(&self) -> bool;

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
    fn entry_target_splits_criteria_before_the_path() {
        let instance = |resource_type: &str, id: &str| BundleEntryTarget::Instance {
            resource_type: resource_type.to_string(),
            id: id.to_string(),
        };
        assert_eq!(
            parse_bundle_entry_target("Patient/p1").unwrap(),
            instance("Patient", "p1")
        );
        assert_eq!(
            parse_bundle_entry_target("http://example.org/fhir/Patient/p1").unwrap(),
            instance("Patient", "p1")
        );
        // A query on an instance URL qualifies the address and is dropped.
        assert_eq!(
            parse_bundle_entry_target("Patient/p1?_format=json").unwrap(),
            instance("Patient", "p1")
        );
        // The `//` inside the criteria must not become a path segment.
        assert_eq!(
            parse_bundle_entry_target("Patient?identifier=http://example.org|12345").unwrap(),
            BundleEntryTarget::Conditional {
                resource_type: "Patient".to_string(),
                criteria: "identifier=http://example.org|12345".to_string(),
            }
        );
        assert!(parse_bundle_entry_target("Patient?").is_err());
        assert!(parse_bundle_entry_target("Patient?&").is_err());
        assert!(parse_bundle_entry_target("Patient").is_err());
        assert!(parse_bundle_entry_target("").is_err());
    }

    #[test]
    fn entry_target_accepts_absolute_conditional_urls_without_confusing_instance_ids() {
        for url in [
            "http://example.org/fhir/Patient?identifier=urn:mrn|123",
            "https://example.org/tenants/acme/fhir/Patient/?identifier=urn:mrn|123",
            "/fhir/Patient?identifier=urn:mrn|123",
        ] {
            assert_eq!(
                parse_bundle_entry_target(url).unwrap(),
                BundleEntryTarget::Conditional {
                    resource_type: "Patient".into(),
                    criteria: "identifier=urn:mrn|123".into(),
                },
                "{url}"
            );
        }
        assert_eq!(
            parse_bundle_entry_target("https://example.org/fhir/Patient/Observation?_format=json")
                .unwrap(),
            BundleEntryTarget::Instance {
                resource_type: "Patient".into(),
                id: "Observation".into()
            }
        );
        assert!(parse_bundle_entry_target("https://example.org/fhir/Patient?").is_err());
    }

    #[test]
    fn entry_result_reference_falls_back_to_the_returned_resource() {
        let created = BundleEntryResult {
            location: Some("Patient/p1/_history/1".to_string()),
            ..BundleEntryResult::deleted()
        };
        assert_eq!(created.reference().as_deref(), Some("Patient/p1"));
        let updated = BundleEntryResult {
            resource: Some(serde_json::json!({"resourceType": "Patient", "id": "p2"})),
            ..BundleEntryResult::deleted()
        };
        assert_eq!(updated.reference().as_deref(), Some("Patient/p2"));
        assert_eq!(BundleEntryResult::deleted().reference(), None);
    }

    #[test]
    fn pin_reference_requires_an_object_body_to_mint_an_id() {
        let mut entry = BundleEntry {
            method: BundleMethod::Put,
            url: "Patient?identifier=http://example.org|12345".to_string(),
            full_url: Some("urn:uuid:patient".to_string()),
            ..Default::default()
        };
        assert!(matches!(
            entry.pin_reference("Patient", None),
            Err(StorageError::Validation(
                crate::error::ValidationError::MissingRequiredField { ref field }
            )) if field == "resource"
        ));
        assert_eq!(entry.resource, None, "no body may be fabricated");

        entry.resource = Some(serde_json::json!("not an object"));
        assert!(matches!(
            entry.pin_reference("Patient", None),
            Err(StorageError::Validation(
                crate::error::ValidationError::InvalidResource { .. }
            ))
        ));

        entry.resource = Some(serde_json::json!({"resourceType": "Patient"}));
        let pinned = entry.pin_reference("Patient", None).unwrap();
        let minted = entry.resource.as_ref().unwrap()["id"].as_str().unwrap();
        assert_eq!(pinned, format!("Patient/{minted}"));
        assert_eq!(
            entry.pin_reference("Patient", Some("matched")).unwrap(),
            "Patient/matched"
        );
    }

    /// Counts the searches pinning asks for and answers with one fixed match.
    #[cfg(any(feature = "sqlite", feature = "mongodb"))]
    struct CountingMatches {
        searches: usize,
        matched: Option<&'static str>,
    }

    #[cfg(any(feature = "sqlite", feature = "mongodb"))]
    #[async_trait]
    impl BundleMatchSource for CountingMatches {
        async fn find_matches(
            &mut self,
            resource_type: &str,
            _criteria: &str,
        ) -> StorageResult<Option<Vec<StoredResource>>> {
            self.searches += 1;
            Ok(Some(
                self.matched
                    .map(|id| {
                        StoredResource::new(
                            resource_type,
                            id,
                            crate::tenant::TenantId::new("t"),
                            serde_json::json!({"resourceType": resource_type, "id": id}),
                            FhirVersion::R4,
                        )
                    })
                    .into_iter()
                    .collect(),
            ))
        }
    }

    #[cfg(any(feature = "sqlite", feature = "mongodb"))]
    fn post(resource: Value, full_url: Option<&str>, if_none_exist: Option<&str>) -> BundleEntry {
        BundleEntry {
            method: BundleMethod::Post,
            url: resource["resourceType"].as_str().unwrap().to_string(),
            resource: Some(resource),
            full_url: full_url.map(str::to_string),
            if_none_exist: if_none_exist.map(str::to_string),
            ..Default::default()
        }
    }

    #[cfg(any(feature = "sqlite", feature = "mongodb"))]
    fn observation_of(subject: &str) -> Value {
        serde_json::json!({"resourceType": "Observation", "subject": {"reference": subject}})
    }

    /// A `fullUrl` referenced only by entries that run after its own is
    /// resolved when the entry runs, so pinning neither searches for its
    /// `ifNoneExist` nor mints an id into its body.
    #[cfg(any(feature = "sqlite", feature = "mongodb"))]
    #[tokio::test]
    async fn pinning_skips_entries_without_a_forward_reference() {
        let mut entries = vec![
            post(
                serde_json::json!({"resourceType": "Patient"}),
                Some("urn:uuid:patient"),
                Some("identifier=x|1"),
            ),
            post(observation_of("urn:uuid:patient"), None, None),
        ];
        let mut source = CountingMatches {
            searches: 0,
            matched: None,
        };
        let pinned = pin_forward_references(&mut entries, &HashMap::new(), &mut source)
            .await
            .unwrap();
        assert!(pinned.is_empty());
        assert_eq!(source.searches, 0);
        assert!(entries[0].resource.as_ref().unwrap().get("id").is_none());
    }

    /// A forward-referenced `ifNoneExist` pins its single match; a forward-
    /// referenced plain create pins an id minted into its body; an instance
    /// PUT pins its URL id.
    #[cfg(any(feature = "sqlite", feature = "mongodb"))]
    #[tokio::test]
    async fn pinning_resolves_forward_references_before_execution() {
        let mut entries = vec![
            post(
                serde_json::json!({"resourceType": "Observation", "subject": {"reference": "urn:uuid:patient"},
                    "performer": [{"reference": "urn:uuid:practitioner"}, {"reference": "urn:uuid:put"}]}),
                None,
                None,
            ),
            post(
                serde_json::json!({"resourceType": "Practitioner"}),
                Some("urn:uuid:practitioner"),
                Some("identifier=x|1"),
            ),
            post(
                serde_json::json!({"resourceType": "Patient"}),
                Some("urn:uuid:patient"),
                None,
            ),
            BundleEntry {
                method: BundleMethod::Put,
                url: "Practitioner/p9".to_string(),
                resource: Some(serde_json::json!({"resourceType": "Practitioner", "id": "p9"})),
                full_url: Some("urn:uuid:put".to_string()),
                ..Default::default()
            },
        ];
        let mut source = CountingMatches {
            searches: 0,
            matched: Some("existing"),
        };
        let pinned = pin_forward_references(&mut entries, &HashMap::new(), &mut source)
            .await
            .unwrap();
        assert_eq!(source.searches, 1);
        assert_eq!(pinned["urn:uuid:practitioner"], "Practitioner/existing");
        let minted = entries[2].resource.as_ref().unwrap()["id"]
            .as_str()
            .unwrap();
        assert_eq!(pinned["urn:uuid:patient"], format!("Patient/{minted}"));
        assert_eq!(pinned["urn:uuid:put"], "Practitioner/p9");
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
        // One stored resource for both: two `stored_patient()` calls stamp
        // separate `Utc::now()` values and flake across a millisecond tick.
        let resource = stored_patient();
        let read = BundleEntryResult::ok(resource.clone());
        let updated = BundleEntryResult::updated(resource);
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
