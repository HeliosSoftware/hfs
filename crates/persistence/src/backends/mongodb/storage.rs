//! ResourceStorage implementation for MongoDB.

use crate::core::transaction::BundleTransactionState;
use crate::core::{BundleEntryTarget, parse_bundle_entry_target};

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use helios_fhir::FhirVersion;
use mongodb::{
    ClientSession, Collection, Cursor, SessionCursor,
    bson::{self, Bson, DateTime as BsonDateTime, Document, doc},
    error::{
        Error as MongoError, ErrorKind as MongoErrorKind, TRANSIENT_TRANSACTION_ERROR,
        UNKNOWN_TRANSACTION_COMMIT_RESULT,
    },
    options::{FindOptions, Hint},
};
use serde_json::Value;

use crate::core::{
    BundleEntry, BundleEntryResult, BundleMethod, BundleProvider, BundleResult, BundleType,
    HistoryEntry, HistoryMethod, HistoryPage, HistoryParams, InstanceHistoryProvider,
    PatchCandidateValidator, PurgableStorage, ResourceStorage, SettingsStore,
    SystemHistoryProvider, TypeHistoryProvider, VersionedStorage, bundle_if_match_gate,
    bundle_if_none_exist_gate, if_match_field_satisfied, normalize_etag,
};
use crate::error::{
    BackendError, ConcurrencyError, QueryErrorExt, ResourceError, SearchError, StorageError,
    StorageResult, TransactionError,
};
use crate::search::converters::IndexValue;
use crate::search::extractor::ExtractedValue;
use crate::search::reindex::{
    ReindexIdRange, ReindexPageStats, ReindexSource, ReindexTarget, ResourcePage,
};
use crate::tenant::{Operation, TenantContext};
use crate::types::{
    CursorValue, Page, PageCursor, PageInfo, SearchParamType, SearchParameter, SearchPrefix,
    SearchQuery, StoredResource,
};

use super::MongoBackend;
use super::retry::{BUNDLE_TRANSACTION_RETRY, jitter_fraction};
use super::schema::{RESOURCES_IDENTITY_INDEX, RESOURCES_TYPE_SCAN_INDEX};

pub(super) fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "mongodb".to_string(),
        message,
        source: None,
    })
}

/// [`internal_error`] for a driver failure, keeping the driver's error as the
/// [`source`](std::error::Error::source) instead of flattening it to text.
///
/// The message is the same `"{context}: {err}"` the stringifying spelling
/// produced, so nothing that reads it changes. What is new is that the typed
/// error survives: a `WriteConflict` inside a bundle transaction carries a
/// `TransientTransactionError` label, and only a typed error lets
/// [`transient_transaction_abort`] see it and replay the bundle (#1586).
pub(super) fn internal_driver_error(context: &str, err: MongoError) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "mongodb".to_string(),
        message: format!("{context}: {err}"),
        source: Some(Box::new(err)),
    })
}

/// A SearchParameter mutation staged during a bundle transaction. The variant
/// records *that* a change occurred; the post-commit step only needs to know a
/// tenant's SearchParameter overlay changed (to invalidate its cached registry),
/// so no payload is carried.
#[derive(Debug, Clone)]
enum PendingSearchParameterChange {
    Create,
    Update,
    Delete,
}

/// Request context shared by every entry in one MongoDB Bundle transaction.
struct BundleEntryContext<'a> {
    tenant: &'a TenantContext,
    fhir_version: helios_fhir::FhirVersion,
    patch_validator: Option<&'a dyn PatchCandidateValidator>,
}

/// Commit retries after a commit whose outcome is unknown
/// (`UnknownTransactionCommitResult`), on top of the first commit. Only the
/// commit is re-sent: the transaction's writes may already be applied, so the
/// entries are never re-run.
const BUNDLE_COMMIT_RETRIES: u32 = 2;

/// Why one attempt at a bundle transaction did not commit (#1586).
#[derive(Debug)]
enum BundleAttemptError {
    /// The server aborted the attempt with a `TransientTransactionError`
    /// (`WriteConflict`, an eviction under cache pressure, a dropped
    /// connection): the transaction did not and will not commit, so running the
    /// bundle again from its original entries is safe.
    TransientAbort {
        /// Position of the entry that was running when the abort arrived, or
        /// `None` when it arrived at commit.
        entry: Option<usize>,
        /// The server's error code, when the driver error carries one.
        code: Option<i32>,
        /// Raw driver detail. Log-only.
        reason: String,
    },
    /// Anything else, already in its final form. Not retried.
    Failed(TransactionError),
}

/// The server error code a driver error carries, for the retry log line.
fn driver_error_code(err: &MongoError) -> Option<i32> {
    match err.kind.as_ref() {
        MongoErrorKind::Command(command) => Some(command.code),
        MongoErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) => Some(write.code),
        MongoErrorKind::Write(mongodb::error::WriteFailure::WriteConcernError(concern)) => {
            Some(concern.code)
        }
        _ => None,
    }
}

fn serialization_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::SerializationError { message })
}

pub(super) fn is_duplicate_key_error(err: &MongoError) -> bool {
    err.to_string().contains("E11000")
}

/// The server's `WriteConflict` code.
const WRITE_CONFLICT_CODE: i32 = 112;

/// The server's `TemporarilyUnavailable` code. Under WiredTiger cache pressure
/// the server rolls a transaction back with it ("oldest pinned transaction ID
/// rolled back for eviction") and, unlike `WriteConflict`, sends it with no
/// `TransientTransactionError` label, so the code is what identifies it (#1700).
const TEMPORARILY_UNAVAILABLE_CODE: i32 = 365;

/// The server's `TransactionTooLargeForCache` code: WiredTiger rolled the
/// transaction back because its own write set will not fit in the cache
/// ("transaction is too large and will not fit in the storage engine cache",
/// WiredTiger `-31800`). Sent with no `TransientTransactionError` label, and
/// unlike 365 a replay cannot help: the same writes meet the same cache (#1837).
const TRANSACTION_TOO_LARGE_FOR_CACHE_CODE: i32 = 388;

/// Pause before the single retry of an unconditional delete that hit a write
/// conflict: long enough for the winner's transaction to commit, so the retry
/// does not just collide with it again.
const WRITE_CONFLICT_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(25);

/// Why one attempt at a versioned write failed.
enum WriteAttemptError {
    /// The server refused the write because a concurrent writer got there
    /// first ([`is_write_conflict`]); nothing was written.
    Conflict(MongoError),
    /// Anything else, already in its final form.
    Storage(StorageError),
}

impl WriteAttemptError {
    /// Classifies a driver error raised inside the attempt's session.
    fn driver(context: &str, err: MongoError) -> Self {
        if is_write_conflict(&err) {
            Self::Conflict(err)
        } else {
            Self::Storage(internal_error(format!("{}: {}", context, err)))
        }
    }
}

impl<E: Into<StorageError>> From<E> for WriteAttemptError {
    fn from(err: E) -> Self {
        Self::Storage(err.into())
    }
}

/// True when the server refused (and rolled back) a write because another
/// operation got to the document first.
///
/// Inside a multi-document transaction MongoDB does not queue behind a
/// conflicting writer the way PostgreSQL queues behind a row lock: the loser's
/// write fails at once with `WriteConflict` (112), labelled
/// `TransientTransactionError`, and its transaction is aborted. The label on
/// its own means the same thing for our purposes — the transaction did not and
/// will not commit, and running it again is safe — so both are classified.
/// `UnknownTransactionCommitResult` is deliberately not: there the write may
/// have landed.
///
/// This used to reach the client as `BackendError::Internal` -> 500 (#1405):
/// "the server failed", for what is "you lost a race, read and retry".
pub(super) fn is_write_conflict(err: &MongoError) -> bool {
    err.contains_label(mongodb::error::TRANSIENT_TRANSACTION_ERROR)
        || has_server_code(err, WRITE_CONFLICT_CODE)
}

/// True when the server rolled a transaction back with `TemporarilyUnavailable`
/// (365), whatever labels came with it (#1700). Not part of [`is_write_conflict`]:
/// that one also drives the single-resource write paths, which this does not
/// change.
fn is_temporarily_unavailable(err: &MongoError) -> bool {
    has_server_code(err, TEMPORARILY_UNAVAILABLE_CODE)
}

/// True when `err` is a server failure with `code`, as a failed command or as a
/// per-write error.
fn has_server_code(err: &MongoError, code: i32) -> bool {
    match err.kind.as_ref() {
        MongoErrorKind::Command(command) => command.code == code,
        MongoErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) => {
            write.code == code
        }
        _ => false,
    }
}

/// True when `err` is `TransactionTooLargeForCache` (388): as a failed command,
/// a per-write error, or any per-document error of an `insert_many`, the shape
/// the search-index insert reports it in (#1837). Decided by the code, never by
/// the message text.
fn is_too_large_for_cache(err: &MongoError) -> bool {
    if let MongoErrorKind::InsertMany(insert_many) = err.kind.as_ref() {
        return insert_many.write_errors.as_ref().is_some_and(|errors| {
            errors
                .iter()
                .any(|error| error.code == TRANSACTION_TOO_LARGE_FOR_CACHE_CODE)
        });
    }
    has_server_code(err, TRANSACTION_TOO_LARGE_FOR_CACHE_CODE)
}

/// The driver error `err` kept as its [`source`](std::error::Error::source), the
/// first one found walking the chain. `None` when the failing site stringified
/// it ([`internal_error`]) or never had one.
fn kept_driver_error(err: &StorageError) -> Option<&MongoError> {
    let mut source = std::error::Error::source(err);
    while let Some(cause) = source {
        if let Some(driver) = cause.downcast_ref::<MongoError>() {
            return Some(driver);
        }
        source = cause.source();
    }
    None
}

/// The driver error behind `err` when it is one the server labelled as a
/// transient transaction abort — the transaction did not and will not commit,
/// and running it again from the start is safe (#1586).
///
/// Walks the [`source`](std::error::Error::source) chain, so it finds the
/// driver error only where the failing site kept it ([`internal_driver_error`],
/// `classify_mongodb_error`); a site that stringified it reads as an ordinary
/// failure. Decided by [`is_write_conflict`], which checks the label first and
/// deliberately excludes `UnknownTransactionCommitResult`, or by an unlabelled
/// code 365 ([`is_temporarily_unavailable`], #1700).
///
/// Not [`WriteAttemptError`]: its blanket `From` takes any typed error as final,
/// which is exactly what hid the label.
fn transient_transaction_abort(err: &StorageError) -> Option<&MongoError> {
    kept_driver_error(err)
        .filter(|driver| is_write_conflict(driver) || is_temporarily_unavailable(driver))
}

/// Why an error raised by a bundle entry counts as a transient abort.
#[derive(Debug)]
struct TransientEntryAbort {
    /// The server's error code, when a driver error that carries one was kept.
    code: Option<i32>,
    /// Raw detail for the retry log. Never shown to a client.
    reason: String,
}

/// Whether the error an entry raised means the server aborted the bundle
/// transaction for a transient reason, so the bundle may be run again (#1586).
///
/// The classifier the bundle loop uses; two shapes qualify:
///
/// - a driver error the failing site kept and the server labelled
///   ([`transient_transaction_abort`]);
/// - [`BackendError::Unavailable`]. The session-scoped search paths reached by
///   an `ifNoneExist` probe (`or_query_error`, so `classify_mongodb_error`)
///   turn a transport failure — an `Io` error, a cleared connection pool, a
///   failed server selection — into `Unavailable` and drop the driver error
///   with its labels, so the first shape cannot see it. Inside a transaction
///   the driver labels exactly those failures `TransientTransactionError`: the
///   transaction is gone and none of its writes can have committed.
///
/// Local to the bundle loop on purpose: `classify_mongodb_error` and
/// `BackendError` keep the shape the rest of the backend and the REST layer
/// rely on. A `Timeout` is not here; it is the server stopping work at its own
/// deadline, which a replay would only meet again.
fn transient_entry_abort(err: &StorageError) -> Option<TransientEntryAbort> {
    if let Some(driver) = transient_transaction_abort(err) {
        return Some(TransientEntryAbort {
            code: driver_error_code(driver),
            reason: format!("Entry processing failed: {err}"),
        });
    }
    // `Unavailable`'s own `Display` renders only the backend name, so the
    // detail for the log comes from its message.
    if let StorageError::Backend(BackendError::Unavailable { message, .. }) = err {
        return Some(TransientEntryAbort {
            code: None,
            reason: format!("Entry processing failed: {message}"),
        });
    }
    None
}

/// Why a bundle attempt failed, given the error entry `index` raised in a
/// transaction of `entries` entries (#1586, #1837).
///
/// - `TransactionTooLargeForCache` (388) comes first, whatever labels ride on
///   it: the Bundle's own write set does not fit WiredTiger's cache, so it is
///   [`TransactionError::TooLargeForCache`] and is never replayed, because every
///   replay would meet the same cache. No entry is blamed for it.
/// - A transient abort ([`transient_entry_abort`]) is handed back for a replay.
/// - Anything else is a [`TransactionError::BundleError`] at the entry.
fn entry_attempt_error(index: usize, entries: usize, err: &StorageError) -> BundleAttemptError {
    if kept_driver_error(err).is_some_and(is_too_large_for_cache) {
        return BundleAttemptError::Failed(TransactionError::TooLargeForCache {
            entries,
            reason: format!("Entry {index} processing failed: {err}"),
        });
    }
    if let Some(abort) = transient_entry_abort(err) {
        return BundleAttemptError::TransientAbort {
            entry: Some(index),
            code: abort.code,
            reason: abort.reason,
        };
    }
    BundleAttemptError::Failed(TransactionError::BundleError {
        index,
        message: format!("Entry processing failed: {err}"),
    })
}

/// What a failed `commitTransaction` means for the bundle (#1586).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitFailure {
    /// The server aborted the transaction at commit: nothing committed, so the
    /// bundle may be run again from its original entries.
    TransientAbort,
    /// The commit was sent and its outcome is unknown — it may have applied.
    /// Sending the commit again is safe; running the entries again is not.
    UnknownResult,
    /// An earlier commit's outcome was unknown and this one failed some other
    /// way. The earlier one may still have applied, so the outcome stays
    /// unknown: no replay, and no claim that anything was rolled back.
    StillUnknown,
    /// The commit failed in a way that leaves nothing applied.
    RolledBack,
}

/// The decision behind [`classify_commit_failure`], taken on the facts it reads
/// off the driver error, so it can be pinned without constructing a labelled
/// driver error (the driver offers no public way to attach a label).
///
/// A write-concern error is decided before any label: the server reports it
/// *after* applying the commit, so the commit applied whatever the error's code
/// and labels say. The driver labels only codes `{50, 64, 91}` (and the
/// retryable-write codes) `UnknownTransactionCommitResult`; any other code,
/// such as `UnknownReplWriteConcern` (79), arrives unlabelled and would
/// otherwise be read as "nothing was applied".
///
/// A transient-abort label is honoured only while no earlier commit is
/// unknown: after one, a replay could apply the bundle a second time.
///
/// An unlabelled `TemporarilyUnavailable` (365) rolled the commit back for cache
/// pressure and counts as a transient abort too (#1700). It is the weakest
/// signal: an explicit `UnknownTransactionCommitResult` label outranks it.
fn classify_commit_signals(
    transient_label: bool,
    unknown_result_label: bool,
    temporarily_unavailable: bool,
    write_concern_failed: bool,
    earlier_unknown: bool,
) -> CommitFailure {
    let transient = transient_label || (temporarily_unavailable && !unknown_result_label);
    if write_concern_failed {
        CommitFailure::UnknownResult
    } else if transient {
        if earlier_unknown {
            CommitFailure::StillUnknown
        } else {
            CommitFailure::TransientAbort
        }
    } else if unknown_result_label {
        CommitFailure::UnknownResult
    } else if earlier_unknown {
        CommitFailure::StillUnknown
    } else {
        CommitFailure::RolledBack
    }
}

/// Classifies a failed commit; `earlier_unknown` says a previous commit of the
/// same transaction already failed with an unknown result.
fn classify_commit_failure(err: &MongoError, earlier_unknown: bool) -> CommitFailure {
    classify_commit_signals(
        err.contains_label(TRANSIENT_TRANSACTION_ERROR),
        err.contains_label(UNKNOWN_TRANSACTION_COMMIT_RESULT),
        is_temporarily_unavailable(err),
        matches!(
            err.kind.as_ref(),
            MongoErrorKind::Write(mongodb::error::WriteFailure::WriteConcernError(_))
        ),
        earlier_unknown,
    )
}

/// What to do after a commit failed.
enum CommitStep {
    /// Send the commit again; the entries are not re-run.
    RetryCommit,
    /// Give up with this result.
    Stop(BundleAttemptError),
}

/// One bundle attempt's commit and its commit-only retries.
struct CommitAttempts {
    /// Commits sent so far, the one that just failed included.
    sent: u32,
    /// True once any commit failed with an unknown result.
    outcome_unknown: bool,
}

impl CommitAttempts {
    fn new() -> Self {
        Self {
            sent: 1,
            outcome_unknown: false,
        }
    }

    /// Decides what follows `err`, the failure of the commit just sent.
    ///
    /// The commit is re-sent while its outcome is unknown, up to
    /// [`BUNDLE_COMMIT_RETRIES`] times. Once any commit has had an unknown
    /// result the answer can only be a retry of the commit or
    /// [`TransactionError::CommitOutcomeUnknown`]: not a replay of the entries,
    /// and not `RolledBack`, which would tell the client nothing was applied
    /// when the earlier commit may have applied it.
    fn after_failure(&mut self, err: &MongoError) -> CommitStep {
        match classify_commit_failure(err, self.outcome_unknown) {
            CommitFailure::TransientAbort => CommitStep::Stop(BundleAttemptError::TransientAbort {
                entry: None,
                code: driver_error_code(err),
                reason: format!("Commit failed: {err}"),
            }),
            CommitFailure::UnknownResult => {
                self.outcome_unknown = true;
                if self.sent <= BUNDLE_COMMIT_RETRIES {
                    tracing::warn!(
                        commit_attempt = self.sent,
                        error_code = driver_error_code(err),
                        "transaction bundle commit outcome unknown; retrying the commit: {err}"
                    );
                    self.sent += 1;
                    CommitStep::RetryCommit
                } else {
                    CommitStep::Stop(BundleAttemptError::Failed(
                        TransactionError::CommitOutcomeUnknown {
                            reason: format!("Commit failed after {} attempts: {err}", self.sent),
                        },
                    ))
                }
            }
            CommitFailure::StillUnknown => CommitStep::Stop(BundleAttemptError::Failed(
                TransactionError::CommitOutcomeUnknown {
                    reason: format!(
                        "Commit failed after {} attempts, an earlier one with an unknown \
                         result: {err}",
                        self.sent
                    ),
                },
            )),
            CommitFailure::RolledBack => {
                CommitStep::Stop(BundleAttemptError::Failed(TransactionError::RolledBack {
                    reason: format!("Commit failed: {err}"),
                }))
            }
        }
    }
}

pub(super) fn ensure_resource_identity(resource_type: &str, id: &str, resource: &mut Value) {
    if let Some(obj) = resource.as_object_mut() {
        obj.insert(
            "resourceType".to_string(),
            Value::String(resource_type.to_string()),
        );
        obj.insert("id".to_string(), Value::String(id.to_string()));
    }
}

pub(super) fn value_to_document(value: &Value) -> StorageResult<Document> {
    let bson = bson::to_bson(value)
        .map_err(|e| serialization_error(format!("Failed to serialize resource: {}", e)))?;
    match bson {
        Bson::Document(doc) => Ok(doc),
        _ => Err(serialization_error(
            "Resource payload must serialize to a BSON document".to_string(),
        )),
    }
}

pub(super) fn document_to_value(doc: &Document) -> StorageResult<Value> {
    bson::from_bson::<Value>(Bson::Document(doc.clone()))
        .map_err(|e| serialization_error(format!("Failed to deserialize resource: {}", e)))
}

fn bson_to_chrono(dt: &BsonDateTime) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(dt.timestamp_millis()).unwrap_or_else(Utc::now)
}

pub(super) fn chrono_to_bson(dt: DateTime<Utc>) -> BsonDateTime {
    BsonDateTime::from_millis(dt.timestamp_millis())
}

/// The instant a stored date is indexed at, or `None` when it cannot be read
/// and the caller should skip the index entry.
///
/// The value is read with the search side's own `FhirDateValue` first, so
/// whatever that grammar accepts is indexed at exactly the first instant of the
/// range a search for the same text covers. That is what indexes a stored
/// `…T09:20` — minutes without seconds, which RFC 3339 does not allow and which
/// used to be skipped here although `date=…T09:20` is a valid search (#1315) —
/// and what puts a `:60` leap second on the next second, where the search side
/// looks for it.
///
/// Only the text as stored counts: the search-side repairs (trimming, and a
/// space read as a form-decoded `+`) do not apply to a resource, where a space
/// is simply not part of a date. Anything the strict grammar does not take
/// verbatim falls through to the lenient reading below, which is unchanged.
fn normalize_date_for_mongo(value: &str) -> Option<DateTime<Utc>> {
    if let Ok(parsed) = crate::search::FhirDateValue::parse(value) {
        if parsed.canonical() == value {
            return Some(parsed.start);
        }
    }
    normalize_date_for_mongo_lenient(value)
}

/// The reading [`normalize_date_for_mongo`] falls back to: complete the value
/// and take whatever chrono's RFC 3339 parser makes of it. Wider than the FHIR
/// grammar on purpose — it is what keeps an out-of-grammar value (`+14:30`, an
/// instant past the year 9999) indexed rather than dropped.
fn normalize_date_for_mongo_lenient(value: &str) -> Option<DateTime<Utc>> {
    let normalized = if value.contains('T') {
        if value.contains('Z') || value.contains('+') || value.matches('-').count() > 2 {
            value.to_string()
        } else {
            format!("{}+00:00", value)
        }
    } else if value.len() == 10 {
        format!("{}T00:00:00+00:00", value)
    } else if value.len() == 7 {
        format!("{}-01T00:00:00+00:00", value)
    } else if value.len() == 4 {
        format!("{}-01-01T00:00:00+00:00", value)
    } else {
        value.to_string()
    };

    DateTime::parse_from_rfc3339(&normalized)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

pub(super) fn next_version(version: &str) -> StorageResult<String> {
    let parsed = version
        .parse::<u64>()
        .map_err(|e| serialization_error(format!("Invalid version value '{}': {}", version, e)))?;
    Ok((parsed + 1).to_string())
}

fn extract_deleted_at(doc: &Document) -> Option<DateTime<Utc>> {
    match doc.get("deleted_at") {
        Some(Bson::DateTime(dt)) => Some(bson_to_chrono(dt)),
        _ => None,
    }
}

pub(super) fn extract_created_at(doc: &Document, fallback: DateTime<Utc>) -> DateTime<Utc> {
    doc.get_datetime("created_at")
        .map(bson_to_chrono)
        .unwrap_or(fallback)
}

fn extract_last_updated(doc: &Document, fallback: DateTime<Utc>) -> DateTime<Utc> {
    doc.get_datetime("last_updated")
        .map(bson_to_chrono)
        .unwrap_or(fallback)
}

pub(super) fn extract_fhir_version(doc: &Document, fallback: FhirVersion) -> FhirVersion {
    doc.get_str("fhir_version")
        .ok()
        .and_then(FhirVersion::from_storage)
        .unwrap_or(fallback)
}

fn parse_version_id(version_id: &str) -> i64 {
    version_id.parse::<i64>().unwrap_or(0)
}

fn history_method_for(version_id: &str, is_deleted: bool) -> HistoryMethod {
    if is_deleted {
        HistoryMethod::Delete
    } else if version_id == "1" {
        HistoryMethod::Post
    } else {
        HistoryMethod::Put
    }
}

fn apply_history_params_filter(filter: &mut Document, params: &HistoryParams) {
    if !params.include_deleted {
        filter.insert("is_deleted", false);
    }

    let mut last_updated = Document::new();
    if let Some(since) = params.since {
        last_updated.insert("$gte", chrono_to_bson(since));
    }
    if let Some(before) = params.before {
        last_updated.insert("$lt", chrono_to_bson(before));
    }

    if !last_updated.is_empty() {
        filter.insert("last_updated", Bson::Document(last_updated));
    }
}

async fn collect_documents(mut cursor: Cursor<Document>) -> StorageResult<Vec<Document>> {
    let mut docs = Vec::new();
    while cursor
        .advance()
        .await
        .map_err(|e| internal_error(format!("Failed to advance MongoDB cursor: {}", e)))?
    {
        let doc = cursor.deserialize_current().map_err(|e| {
            internal_error(format!("Failed to deserialize MongoDB document: {}", e))
        })?;
        docs.push(doc);
    }
    Ok(docs)
}

pub(super) async fn collect_session_documents(
    mut cursor: SessionCursor<Document>,
    session: &mut ClientSession,
) -> StorageResult<Vec<Document>> {
    let mut docs = Vec::new();
    while cursor
        .advance(session)
        .await
        .map_err(|e| internal_driver_error("Failed to advance MongoDB session cursor", e))?
    {
        let doc = cursor.deserialize_current().map_err(|e| {
            internal_error(format!(
                "Failed to deserialize MongoDB session document: {}",
                e
            ))
        })?;
        docs.push(doc);
    }
    Ok(docs)
}

fn parse_cursor_version(params: &HistoryParams) -> Option<i64> {
    let cursor = params.pagination.cursor_value()?;
    let value = cursor.sort_values().first()?;
    match value {
        CursorValue::String(version) => version.parse::<i64>().ok(),
        CursorValue::Number(version) => Some(*version),
        _ => None,
    }
}

fn parse_cursor_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn parse_type_history_cursor(params: &HistoryParams) -> Option<(DateTime<Utc>, String)> {
    let cursor = params.pagination.cursor_value()?;
    let sort_values = cursor.sort_values();
    if sort_values.len() < 2 {
        return None;
    }

    let timestamp = match sort_values.first()? {
        CursorValue::String(value) => parse_cursor_timestamp(value)?,
        _ => return None,
    };

    let id = match sort_values.get(1)? {
        CursorValue::String(value) => value.clone(),
        _ => return None,
    };

    Some((timestamp, id))
}

fn parse_system_history_cursor(params: &HistoryParams) -> Option<(DateTime<Utc>, String, String)> {
    let cursor = params.pagination.cursor_value()?;
    let sort_values = cursor.sort_values();
    if sort_values.len() < 3 {
        return None;
    }

    let timestamp = match sort_values.first()? {
        CursorValue::String(value) => parse_cursor_timestamp(value)?,
        _ => return None,
    };

    let resource_type = match sort_values.get(1)? {
        CursorValue::String(value) => value.clone(),
        _ => return None,
    };

    let id = match sort_values.get(2)? {
        CursorValue::String(value) => value.clone(),
        _ => return None,
    };

    Some((timestamp, resource_type, id))
}

/// Server-side keyset predicate for a `history_type` cursor page: matches the
/// same `last_updated < ts OR (last_updated == ts AND id < cursor_id)` shape
/// already used by the Rust-side comparison here and by the SQLite/Postgres
/// backends, but as a MongoDB `$or` so the index — not a full scan — can
/// serve it. `None` when there is no cursor (first page), an absent/malformed
/// cursor, or offset-mode pagination — in every such case the caller adds no
/// `$or` and the query is simply the first page.
fn type_history_cursor_or(params: &HistoryParams) -> Option<Vec<Document>> {
    let (ts, id) = parse_type_history_cursor(params)?;
    let bson_ts = chrono_to_bson(ts);
    Some(vec![
        doc! { "last_updated": { "$lt": bson_ts } },
        doc! { "last_updated": bson_ts, "id": { "$lt": id } },
    ])
}

/// Same as [`type_history_cursor_or`] but for `history_system`'s 3-key sort
/// (`last_updated`, `resource_type`, `id`).
fn system_history_cursor_or(params: &HistoryParams) -> Option<Vec<Document>> {
    let (ts, resource_type, id) = parse_system_history_cursor(params)?;
    let bson_ts = chrono_to_bson(ts);
    Some(vec![
        doc! { "last_updated": { "$lt": bson_ts } },
        doc! { "last_updated": bson_ts, "resource_type": { "$lt": &resource_type } },
        doc! {
            "last_updated": bson_ts,
            "resource_type": &resource_type,
            "id": { "$lt": id },
        },
    ])
}

/// Sort matching `idx_history_type_updated`'s key order after its two
/// equality-filtered prefix fields (`tenant_id`, `resource_type`).
fn type_history_sort() -> Document {
    doc! { "last_updated": -1_i32, "id": -1_i32 }
}

/// Sort matching `idx_history_system_updated`'s key order after its
/// equality-filtered prefix field (`tenant_id`).
fn system_history_sort() -> Document {
    doc! { "last_updated": -1_i32, "resource_type": -1_i32, "id": -1_i32 }
}

/// Documents to fetch for a history page: `count + 1` so the caller can
/// detect `has_next` by truncating back to `count`. Never 0 — MongoDB reads
/// `limit(0)` as "unlimited", which would silently undo the bound.
fn history_fetch_limit(count: u32) -> i64 {
    i64::from(count.saturating_add(1))
}

#[derive(Debug, Clone)]
struct ParsedHistoryRow {
    resource_type: String,
    id: String,
    version_id: String,
    content: Value,
    last_updated: DateTime<Utc>,
    is_deleted: bool,
    deleted_at: Option<DateTime<Utc>>,
    fhir_version: FhirVersion,
}

impl ParsedHistoryRow {
    fn into_stored_resource(self, tenant: &TenantContext) -> StoredResource {
        StoredResource::from_storage(
            &self.resource_type,
            &self.id,
            &self.version_id,
            tenant.tenant_id().clone(),
            self.content,
            self.last_updated,
            self.last_updated,
            self.deleted_at,
            self.fhir_version,
        )
    }

    fn into_history_entry(self, tenant: &TenantContext) -> HistoryEntry {
        let method = history_method_for(&self.version_id, self.is_deleted);
        let timestamp = self.last_updated;
        let resource = self.into_stored_resource(tenant);

        HistoryEntry {
            resource,
            method,
            timestamp,
        }
    }
}

fn parse_history_row(
    doc: &Document,
    fallback_resource_type: Option<&str>,
    fallback_id: Option<&str>,
) -> StorageResult<ParsedHistoryRow> {
    let resource_type = doc
        .get_str("resource_type")
        .ok()
        .map(str::to_string)
        .or_else(|| fallback_resource_type.map(str::to_string))
        .ok_or_else(|| internal_error("Missing resource_type in history document".to_string()))?;

    let id = doc
        .get_str("id")
        .ok()
        .map(str::to_string)
        .or_else(|| fallback_id.map(str::to_string))
        .ok_or_else(|| internal_error("Missing id in history document".to_string()))?;

    let version_id = doc
        .get_str("version_id")
        .map_err(|e| internal_error(format!("Missing history version_id: {}", e)))?
        .to_string();

    let payload = doc
        .get_document("data")
        .map_err(|e| internal_error(format!("Missing history payload: {}", e)))?;
    let content = document_to_value(payload)?;

    let now = Utc::now();
    let last_updated = extract_last_updated(doc, now);
    let is_deleted = doc.get_bool("is_deleted").unwrap_or(false);
    let deleted_at = extract_deleted_at(doc).or(if is_deleted { Some(last_updated) } else { None });
    let fhir_version = extract_fhir_version(doc, FhirVersion::default_enabled());

    Ok(ParsedHistoryRow {
        resource_type,
        id,
        version_id,
        content,
        last_updated,
        is_deleted,
        deleted_at,
        fhir_version,
    })
}

pub(super) fn document_to_stored_resource(
    doc: &Document,
    tenant: &TenantContext,
    fallback_resource_type: &str,
) -> StorageResult<StoredResource> {
    let resource_type = doc
        .get_str("resource_type")
        .ok()
        .unwrap_or(fallback_resource_type)
        .to_string();

    let id = doc
        .get_str("id")
        .map_err(|e| internal_error(format!("Missing resource id in MongoDB document: {}", e)))?
        .to_string();

    let version_id = doc
        .get_str("version_id")
        .map_err(|e| internal_error(format!("Missing version_id in MongoDB document: {}", e)))?
        .to_string();

    let payload = doc.get_document("data").map_err(|e| {
        internal_error(format!(
            "Missing resource payload in MongoDB document: {}",
            e
        ))
    })?;
    let content = document_to_value(payload)?;

    let now = Utc::now();
    let created_at = extract_created_at(doc, now);
    let last_updated = extract_last_updated(doc, now);
    let deleted_at = extract_deleted_at(doc);
    let fhir_version = extract_fhir_version(doc, FhirVersion::default_enabled());

    Ok(StoredResource::from_storage(
        resource_type,
        id,
        version_id,
        tenant.tenant_id().clone(),
        content,
        created_at,
        last_updated,
        deleted_at,
        fhir_version,
    ))
}

/// The in-process SQL-on-FHIR runner's view of the `resources` collection:
/// every live resource of one type for a tenant, as FHIR JSON with the
/// server's `meta.versionId`/`meta.lastUpdated` merged in (a `since` filter
/// reads the latter). Backs the compartment-filter fallback in
/// [`MongoBackend::sof_runner`](crate::core::ResourceStorage::sof_runner).
struct MongoResourceScan {
    client: std::sync::Arc<tokio::sync::OnceCell<mongodb::Client>>,
    config: super::backend::MongoBackendConfig,
}

#[async_trait]
impl crate::sof::in_process::ResourceScan for MongoResourceScan {
    async fn scan_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> Result<crate::sof::in_process::ResourceStream, crate::core::sof_runner::SofError> {
        use crate::core::sof_runner::SofError;
        use futures::stream;

        let client = self
            .client
            .get_or_try_init(|| super::backend::connect_client(&self.config))
            .await
            .map_err(|e| SofError::Storage(e.to_string()))?;

        let filter = doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": resource_type,
            "is_deleted": false,
        };
        let cursor = client
            .database(&self.config.database_name)
            .collection::<Document>(MongoBackend::RESOURCES_COLLECTION)
            .find(filter)
            .await
            .map_err(|e| SofError::Storage(e.to_string()))?;

        let tenant_owned = tenant.clone();
        let resource_type = resource_type.to_string();

        let scan_stream = stream::try_unfold(cursor, move |mut cursor| {
            let tenant_owned = tenant_owned.clone();
            let resource_type = resource_type.clone();
            async move {
                match cursor.advance().await {
                    Err(e) => Err(SofError::Storage(e.to_string())),
                    Ok(false) => Ok(None),
                    Ok(true) => {
                        let doc = cursor
                            .deserialize_current()
                            .map_err(|e| SofError::Storage(e.to_string()))?;
                        let value =
                            document_to_stored_resource(&doc, &tenant_owned, &resource_type)
                                .map(StoredResource::into_content_with_meta)
                                .map_err(|e| SofError::Storage(e.to_string()))?;
                        Ok(Some((value, cursor)))
                    }
                }
            }
        });

        Ok(Box::pin(scan_stream))
    }

    async fn read_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        ids: &[String],
    ) -> Result<Vec<Value>, crate::core::sof_runner::SofError> {
        use crate::core::sof_runner::SofError;

        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let client = self
            .client
            .get_or_try_init(|| super::backend::connect_client(&self.config))
            .await
            .map_err(|e| SofError::Storage(e.to_string()))?;

        // One `find` over the same `(tenant_id, resource_type, id)` index the
        // single-resource read uses; soft-deleted ids drop out with the filter.
        let filter = doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": resource_type,
            "is_deleted": false,
            "id": { "$in": ids },
        };
        let mut cursor = client
            .database(&self.config.database_name)
            .collection::<Document>(MongoBackend::RESOURCES_COLLECTION)
            .find(filter)
            .await
            .map_err(|e| SofError::Storage(e.to_string()))?;

        let mut out = Vec::with_capacity(ids.len());
        while cursor
            .advance()
            .await
            .map_err(|e| SofError::Storage(e.to_string()))?
        {
            let doc = cursor
                .deserialize_current()
                .map_err(|e| SofError::Storage(e.to_string()))?;
            let value = document_to_stored_resource(&doc, tenant, resource_type)
                .map(StoredResource::into_content_with_meta)
                .map_err(|e| SofError::Storage(e.to_string()))?;
            out.push(value);
        }
        Ok(out)
    }
}

async fn begin_required_bundle_transaction_session(
    db: &mongodb::Database,
) -> Result<ClientSession, TransactionError> {
    let mut session =
        db.client()
            .start_session()
            .await
            .map_err(|e| TransactionError::RolledBack {
                reason: format!("Failed to start MongoDB session: {}", e),
            })?;

    let hello = db.run_command(doc! { "hello": 1_i32 }).await.map_err(|e| {
        TransactionError::RolledBack {
            reason: format!("Failed to inspect MongoDB topology: {}", e),
        }
    })?;

    let supports_transactions = hello.contains_key("setName")
        || hello
            .get_str("msg")
            .map(|value| value == "isdbgrid")
            .unwrap_or(false);

    if !supports_transactions {
        return Err(TransactionError::UnsupportedIsolationLevel {
            level: "transaction bundles for mongodb require replica-set or sharded topology"
                .to_string(),
        });
    }

    session
        .start_transaction()
        .await
        .map_err(|e| TransactionError::RolledBack {
            reason: format!("Failed to start MongoDB transaction: {}", e),
        })?;

    Ok(session)
}

async fn begin_best_effort_multi_write_session(
    db: &mongodb::Database,
) -> (Option<ClientSession>, bool) {
    let mut session = db.client().start_session().await.ok();
    let mut transaction_active = false;

    if let Some(active_session) = session.as_mut() {
        // Transactions are only supported on replica sets and sharded deployments.
        // Fall back to non-transactional writes for standalone servers.
        let hello = db.run_command(doc! { "hello": 1_i32 }).await.ok();
        let supports_transactions = hello.as_ref().is_some_and(|doc| {
            doc.contains_key("setName")
                || doc
                    .get_str("msg")
                    .map(|value| value == "isdbgrid")
                    .unwrap_or(false)
        });

        if supports_transactions && active_session.start_transaction().await.is_ok() {
            transaction_active = true;
        } else {
            // Fallback to non-transactional writes on deployments that don't support transactions.
            session = None;
        }
    }

    (session, transaction_active)
}

async fn commit_best_effort_multi_write_session(
    session: &mut Option<ClientSession>,
    transaction_active: bool,
    operation: &str,
) -> StorageResult<()> {
    try_commit_best_effort_multi_write_session(session, transaction_active)
        .await
        .map_err(|e| {
            internal_error(format!(
                "Failed to commit MongoDB transaction after {}: {}",
                operation, e
            ))
        })
}

/// [`commit_best_effort_multi_write_session`] with the driver's error intact,
/// for the writes that classify a commit-time `WriteConflict`.
async fn try_commit_best_effort_multi_write_session(
    session: &mut Option<ClientSession>,
    transaction_active: bool,
) -> Result<(), MongoError> {
    if !transaction_active {
        return Ok(());
    }

    if let Some(active_session) = session.as_mut() {
        active_session.commit_transaction().await?;
    }

    Ok(())
}

#[async_trait]
impl ResourceStorage for MongoBackend {
    fn backend_name(&self) -> &'static str {
        "mongodb"
    }

    async fn readiness_check(&self) -> Result<(), BackendError> {
        <Self as crate::core::Backend>::health_check(self).await?;
        let database = self
            .get_database()
            .await
            .map_err(|error| BackendError::Unavailable {
                backend_name: "mongodb".to_string(),
                message: format!("Unable to check the required query indexes: {error}"),
            })?;
        super::schema::check_required_query_indexes(&database).await
    }

    fn bulk_write_concurrency(&self) -> usize {
        // Bulk seeding is round-trip bound; the driver pool absorbs parallel
        // writers.
        8
    }

    fn is_cluster_shared(&self) -> bool {
        true
    }

    fn sof_runner(&self) -> Option<std::sync::Arc<dyn crate::core::sof_runner::SofRunner>> {
        use crate::sof::in_process::{InProcessSofRunner, ResourceScan};
        use crate::sof::mongodb::MongoInDbRunner;
        // Native in-DB runner: compiles the ViewDefinition to an aggregation
        // pipeline executed against the `resources` collection. Patient/group
        // compartment filters are the one thing the pipeline compiler does not
        // cover, so runs carrying them go to the in-process engine over a scan
        // of the same collection — the runner S3 uses for everything — rather
        // than failing as uncompilable (the SQL Export UI's job-wide filters
        // ended every such job as `failed` on this backend).
        let scan: std::sync::Arc<dyn ResourceScan> = std::sync::Arc::new(MongoResourceScan {
            client: self.client_cell(),
            config: self.config().clone(),
        });
        let fallback =
            InProcessSofRunner::new(scan, self.config().fhir_version, "mongo-in-process");
        Some(std::sync::Arc::new(
            MongoInDbRunner::new(self.client_cell(), self.config().clone())
                .with_compartment_fallback(std::sync::Arc::new(fallback)),
        ))
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        tenant.check_permission(Operation::Create, resource_type)?;

        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let (mut session, transaction_active) = begin_best_effort_multi_write_session(&db).await;
        let tenant_id = tenant.tenant_id().as_str();

        // Extract or generate ID
        let id = resource
            .get("id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(crate::types::new_resource_id);

        // Check if resource already exists (including deleted resources).
        let identity_filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": &id,
        };

        let existing = if let Some(active_session) = session.as_mut() {
            resources
                .find_one(identity_filter.clone())
                .session(active_session)
                .await
                .map_err(|e| {
                    internal_error(format!("Failed to check existence (session): {}", e))
                })?
        } else {
            resources
                .find_one(identity_filter)
                .await
                .map_err(|e| internal_error(format!("Failed to check existence: {}", e)))?
        };

        if existing.is_some() {
            return Err(StorageError::Resource(ResourceError::AlreadyExists {
                resource_type: resource_type.to_string(),
                id,
            }));
        }

        let mut resource = resource;
        ensure_resource_identity(resource_type, &id, &mut resource);

        let payload = value_to_document(&resource)?;

        let now = Utc::now();
        let now_bson = chrono_to_bson(now);
        let version_id = "1".to_string();
        let fhir_version_str = fhir_version.as_mime_param().to_string();

        let resource_doc = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": &id,
            "version_id": &version_id,
            "data": Bson::Document(payload.clone()),
            "created_at": now_bson,
            "last_updated": now_bson,
            "is_deleted": false,
            "deleted_at": Bson::Null,
            "fhir_version": &fhir_version_str,
        };

        if let Some(active_session) = session.as_mut() {
            resources
                .insert_one(resource_doc.clone())
                .session(active_session)
                .await
                .map_err(|e| {
                    if is_duplicate_key_error(&e) {
                        StorageError::Resource(ResourceError::AlreadyExists {
                            resource_type: resource_type.to_string(),
                            id: id.clone(),
                        })
                    } else {
                        internal_error(format!("Failed to insert resource (session): {}", e))
                    }
                })?;
        } else {
            resources.insert_one(resource_doc).await.map_err(|e| {
                if is_duplicate_key_error(&e) {
                    StorageError::Resource(ResourceError::AlreadyExists {
                        resource_type: resource_type.to_string(),
                        id: id.clone(),
                    })
                } else {
                    internal_error(format!("Failed to insert resource: {}", e))
                }
            })?;
        }

        let history_doc = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": &id,
            "version_id": &version_id,
            "data": Bson::Document(payload),
            "created_at": now_bson,
            "last_updated": now_bson,
            "is_deleted": false,
            "deleted_at": Bson::Null,
            "fhir_version": fhir_version_str,
        };

        if let Some(active_session) = session.as_mut() {
            history
                .insert_one(history_doc)
                .session(active_session)
                .await
                .map_err(|e| {
                    internal_error(format!(
                        "Failed to insert resource history (session): {}",
                        e
                    ))
                })?;
        } else {
            history
                .insert_one(history_doc)
                .await
                .map_err(|e| internal_error(format!("Failed to insert resource history: {}", e)))?;
        }

        self.index_resource(&db, tenant_id, resource_type, &id, &resource, &mut session)
            .await?;

        commit_best_effort_multi_write_session(&mut session, transaction_active, "create").await?;

        // An overlay-affecting SearchParameter write: refresh the stored-param
        // cache (which the per-tenant loader reads) and drop the cached
        // registries. This must run after the commit above: the reload
        // reads the `resources` collection without the session, so while the
        // transaction is still open the write above is invisible to it. Seeded
        // spec copies never affect the overlay (see `create_affects_overlay`),
        // which keeps bulk seeding from triggering an O(n²) reload storm.
        if resource_type == "SearchParameter"
            && self.tenant_registries().create_affects_overlay(&resource)
        {
            if let Err(e) = self.reload_stored_cache_for_tenant(tenant_id).await {
                tracing::warn!("SearchParameter cache reload failed: {e}");
            }
        }

        Ok(StoredResource::from_storage(
            resource_type,
            &id,
            version_id,
            tenant.tenant_id().clone(),
            resource,
            now,
            now,
            None,
            fhir_version,
        ))
    }

    async fn create_or_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        // Check if exists
        match self.read(tenant, resource_type, id).await {
            // Update existing (preserves original FHIR version)
            Ok(Some(current)) => {
                let updated = self.update(tenant, &current, resource).await?;
                Ok((updated, false))
            }
            // Create new with specific ID
            Ok(None) => {
                let mut resource = resource;
                if let Some(obj) = resource.as_object_mut() {
                    obj.insert("id".to_string(), Value::String(id.to_string()));
                }
                let created = self
                    .create(tenant, resource_type, resource, fhir_version)
                    .await?;
                Ok((created, true))
            }
            // A deleted resource is brought back to life by a subsequent update
            // (FHIR http.html#delete), continuing the existing version chain
            // rather than being rejected with `Gone`.
            Err(StorageError::Resource(ResourceError::Gone { .. })) => {
                let restored = self
                    .restore_deleted(tenant, resource_type, id, resource)
                    .await?;
                Ok((restored, true))
            }
            Err(e) => Err(e),
        }
    }

    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let maybe_doc = resources
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
            })
            .await
            .map_err(|e| internal_error(format!("Failed to read resource: {}", e)))?;

        let Some(doc) = maybe_doc else {
            return Ok(None);
        };

        let is_deleted = doc.get_bool("is_deleted").unwrap_or(false);
        if is_deleted {
            return Err(StorageError::Resource(ResourceError::Gone {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
                deleted_at: extract_deleted_at(&doc),
            }));
        }

        let version_id = doc
            .get_str("version_id")
            .map_err(|e| internal_error(format!("Missing version_id: {}", e)))?
            .to_string();

        let payload = doc
            .get_document("data")
            .map_err(|e| internal_error(format!("Missing resource payload: {}", e)))?;
        let content = document_to_value(payload)?;

        let now = Utc::now();
        let created_at = extract_created_at(&doc, now);
        let last_updated = extract_last_updated(&doc, now);
        let fhir_version = extract_fhir_version(&doc, FhirVersion::default_enabled());

        Ok(Some(StoredResource::from_storage(
            resource_type,
            id,
            version_id,
            tenant.tenant_id().clone(),
            content,
            created_at,
            last_updated,
            None,
            fhir_version,
        )))
    }

    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        // No retry: `update` always carries a precondition (`current`'s
        // version), and a writer that beat us to the document has, or is about
        // to have, moved it on. The honest answer is the one PostgreSQL gives
        // for the same race — `VersionConflict` -> 409 — not a second attempt.
        match self.update_attempt(tenant, current, resource).await {
            Ok(stored) => Ok(stored),
            Err(WriteAttemptError::Storage(e)) => Err(e),
            Err(WriteAttemptError::Conflict(e)) => Err(self
                .lost_race(
                    tenant,
                    current.resource_type(),
                    current.id(),
                    current.version_id(),
                    &e,
                )
                .await),
        }
    }

    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        self.soft_delete(tenant, resource_type, id, None).await
    }

    async fn delete_versioned(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
    ) -> StorageResult<()> {
        self.soft_delete(tenant, resource_type, id, Some(expected_version))
            .await
    }

    async fn exists(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<bool> {
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let count = resources
            .count_documents(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
                "is_deleted": false,
            })
            .await
            .map_err(|e| internal_error(format!("Failed to check resource existence: {}", e)))?;

        Ok(count > 0)
    }

    async fn read_batch(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        ids: &[&str],
    ) -> StorageResult<Vec<StoredResource>> {
        let mut resources = Vec::with_capacity(ids.len());

        for id in ids {
            // A missing or soft-deleted (Gone) id is omitted, not fatal — one
            // deleted target must not fail the whole batch (matches the default
            // impl / #1119).
            match self.read(tenant, resource_type, id).await {
                Ok(Some(resource)) => resources.push(resource),
                Ok(None) => {}
                Err(StorageError::Resource(ResourceError::Gone { .. })) => {}
                Err(e) => return Err(e),
            }
        }

        Ok(resources)
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let mut filter = doc! {
            "tenant_id": tenant_id,
            "is_deleted": false,
        };

        if let Some(resource_type) = resource_type {
            filter.insert("resource_type", resource_type);
        }

        resources
            .count_documents(filter)
            .await
            .or_query_error("Failed to count resources")
    }

    async fn count_by_day(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        since: DateTime<Utc>,
    ) -> StorageResult<Vec<crate::core::DailyResourceCount>> {
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let since_bson = BsonDateTime::from_millis(since.timestamp_millis());

        // Bucket on the server: `$match` narrows to this tenant/type/window
        // (covered by the `(tenant_id, last_updated)` index), then `$group`
        // collapses to one document per UTC calendar day. `$dateToString` with an
        // explicit UTC timezone keeps day boundaries consistent with the SQL
        // backends.
        let pipeline = vec![
            doc! { "$match": {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "is_deleted": false,
                "last_updated": { "$gte": since_bson },
            }},
            doc! { "$group": {
                "_id": { "$dateToString": {
                    "format": "%Y-%m-%d",
                    "date": "$last_updated",
                    "timezone": "UTC",
                }},
                "n": { "$sum": 1 },
            }},
            doc! { "$sort": { "_id": 1 } },
        ];

        let mut cursor = resources
            .aggregate(pipeline)
            .await
            .or_query_error("Failed to aggregate count_by_day")?;

        let mut out = Vec::new();
        while cursor
            .advance()
            .await
            .or_query_error("count_by_day cursor advance")?
        {
            let doc = cursor
                .deserialize_current()
                .or_query_error("count_by_day cursor deserialize")?;
            let day_str = doc.get_str("_id").unwrap_or_default();
            // `$sum: 1` yields an int32 unless it overflows into int64.
            let n = doc
                .get_i32("n")
                .map(i64::from)
                .or_else(|_| doc.get_i64("n"))
                .unwrap_or(0);
            if let Ok(day) = chrono::NaiveDate::parse_from_str(day_str, "%Y-%m-%d") {
                out.push(crate::core::DailyResourceCount {
                    day,
                    count: n.max(0) as u64,
                });
            }
        }
        Ok(out)
    }

    async fn count_deltas_by_bucket(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        since: DateTime<Utc>,
        bucket_seconds: i64,
    ) -> StorageResult<Vec<crate::core::ResourceCountDelta>> {
        if bucket_seconds <= 0 {
            return Err(internal_error(
                "count_deltas_by_bucket: bucket_seconds must be positive".to_string(),
            ));
        }
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let bucket_ms = bucket_seconds * 1000;
        let since_bson = BsonDateTime::from_millis(
            crate::core::bucket_floor(since, bucket_seconds).timestamp_millis(),
        );

        // Bucket on the server: `$match` narrows to this tenant/type/window (covered
        // by the `(tenant_id, last_updated)` history index), then `$group` floors each
        // version to its epoch-aligned bucket by subtracting the remainder of its
        // epoch-millis modulo the bucket width — the same arithmetic the SQL backends
        // do, so bucket boundaries agree across backends. Delta rule per the trait
        // doc: creation `+1`, delete `-1`, plain update `0`.
        let epoch_ms = doc! { "$toLong": "$last_updated" };
        let pipeline = vec![
            doc! { "$match": {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "last_updated": { "$gte": since_bson },
            }},
            doc! { "$group": {
                "_id": { "$subtract": [
                    epoch_ms.clone(),
                    { "$mod": [epoch_ms, bucket_ms] },
                ]},
                "delta": { "$sum": { "$switch": {
                    "branches": [
                        { "case": { "$eq": ["$is_deleted", true] }, "then": -1 },
                        { "case": { "$eq": ["$version_id", "1"] }, "then": 1 },
                    ],
                    "default": 0,
                }}},
            }},
            doc! { "$match": { "delta": { "$ne": 0 } } },
            doc! { "$sort": { "_id": 1 } },
        ];

        let mut cursor = history
            .aggregate(pipeline)
            .await
            .or_query_error("Failed to aggregate count_deltas_by_bucket")?;

        let mut out = Vec::new();
        while cursor
            .advance()
            .await
            .or_query_error("count_deltas cursor advance")?
        {
            let doc = cursor
                .deserialize_current()
                .or_query_error("count_deltas cursor deserialize")?;
            let bucket_ms_start = doc.get_i64("_id").unwrap_or_default();
            // `$sum` yields an int32 for small totals and an int64 once it overflows,
            // so accept either width rather than assuming one.
            let delta = doc
                .get_i64("delta")
                .or_else(|_| doc.get_i32("delta").map(i64::from))
                .unwrap_or_default();
            if let Some(bucket_start) = DateTime::from_timestamp_millis(bucket_ms_start) {
                out.push(crate::core::ResourceCountDelta {
                    bucket_start,
                    delta,
                });
            }
        }
        Ok(out)
    }

    async fn count_deltas_by_type_and_bucket(
        &self,
        tenant: &TenantContext,
        resource_types: &[&str],
        since: DateTime<Utc>,
        bucket_seconds: i64,
    ) -> StorageResult<Vec<(String, crate::core::ResourceCountDelta)>> {
        if resource_types.is_empty() {
            return Ok(Vec::new());
        }
        if bucket_seconds <= 0 {
            return Err(internal_error(
                "count_deltas_by_type_and_bucket: bucket_seconds must be positive".to_string(),
            ));
        }
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let bucket_ms = bucket_seconds * 1000;
        let since_bson = BsonDateTime::from_millis(
            crate::core::bucket_floor(since, bucket_seconds).timestamp_millis(),
        );
        let mut types: Vec<&str> = resource_types.to_vec();
        types.sort_unstable();
        types.dedup();

        // One aggregation for every requested type (#1078): the same `$match`
        // window, epoch-millis bucket arithmetic and delta rule as
        // `count_deltas_by_bucket`, with `resource_type` narrowed by `$in` and
        // added to the group key, so each type's rows equal its per-type call's.
        // `idx_history_system_updated` (tenant_id, last_updated, resource_type)
        // serves the tenant + time-range scan.
        let epoch_ms = doc! { "$toLong": "$last_updated" };
        let pipeline = vec![
            doc! { "$match": {
                "tenant_id": tenant_id,
                "resource_type": { "$in": types },
                "last_updated": { "$gte": since_bson },
            }},
            doc! { "$group": {
                "_id": {
                    "resource_type": "$resource_type",
                    "bucket": { "$subtract": [
                        epoch_ms.clone(),
                        { "$mod": [epoch_ms, bucket_ms] },
                    ]},
                },
                "delta": { "$sum": { "$switch": {
                    "branches": [
                        { "case": { "$eq": ["$is_deleted", true] }, "then": -1 },
                        { "case": { "$eq": ["$version_id", "1"] }, "then": 1 },
                    ],
                    "default": 0,
                }}},
            }},
            doc! { "$match": { "delta": { "$ne": 0 } } },
            doc! { "$sort": { "_id.resource_type": 1, "_id.bucket": 1 } },
        ];

        let mut cursor = history
            .aggregate(pipeline)
            .await
            .or_query_error("Failed to aggregate count_deltas_by_type_and_bucket")?;

        let mut out = Vec::new();
        while cursor
            .advance()
            .await
            .or_query_error("count_deltas_by_type cursor advance")?
        {
            let doc = cursor
                .deserialize_current()
                .or_query_error("count_deltas_by_type cursor deserialize")?;
            let Ok(key) = doc.get_document("_id") else {
                continue;
            };
            let Ok(resource_type) = key.get_str("resource_type") else {
                continue;
            };
            let bucket_ms_start = key.get_i64("bucket").unwrap_or_default();
            // `$sum` yields an int32 for small totals and an int64 once it overflows,
            // so accept either width rather than assuming one.
            let delta = doc
                .get_i64("delta")
                .or_else(|_| doc.get_i32("delta").map(i64::from))
                .unwrap_or_default();
            if let Some(bucket_start) = DateTime::from_timestamp_millis(bucket_ms_start) {
                out.push((
                    resource_type.to_string(),
                    crate::core::ResourceCountDelta {
                        bucket_start,
                        delta,
                    },
                ));
            }
        }
        Ok(out)
    }

    async fn activity_histogram(
        &self,
        tenant: &TenantContext,
        since: DateTime<Utc>,
    ) -> StorageResult<Vec<crate::core::ActivityCell>> {
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let since_bson = BsonDateTime::from_millis(since.timestamp_millis());

        // `$dayOfWeek` returns 1=Sunday..7=Saturday and `$hour` 0..23, both in the
        // requested timezone. We subtract 1 from the weekday below to land on the
        // 0=Sunday..6=Saturday convention shared with the SQL backends.
        let pipeline = vec![
            doc! { "$match": {
                "tenant_id": tenant_id,
                "last_updated": { "$gte": since_bson },
            }},
            doc! { "$group": {
                "_id": {
                    "wd": { "$dayOfWeek": { "date": "$last_updated", "timezone": "UTC" } },
                    "hr": { "$hour": { "date": "$last_updated", "timezone": "UTC" } },
                },
                "n": { "$sum": 1 },
            }},
        ];

        let mut cursor = history
            .aggregate(pipeline)
            .await
            .or_query_error("Failed to aggregate activity histogram")?;

        let mut out = Vec::new();
        while cursor
            .advance()
            .await
            .or_query_error("activity cursor advance")?
        {
            let doc = cursor
                .deserialize_current()
                .or_query_error("activity cursor deserialize")?;
            let id = match doc.get_document("_id") {
                Ok(id) => id,
                Err(_) => continue,
            };
            let wd = id.get_i32("wd").unwrap_or(1); // 1=Sunday..7=Saturday
            let hr = id.get_i32("hr").unwrap_or(0);
            let n = doc
                .get_i32("n")
                .map(i64::from)
                .or_else(|_| doc.get_i64("n"))
                .unwrap_or(0);
            out.push(crate::core::ActivityCell {
                weekday: (wd - 1).clamp(0, 6) as u8,
                hour: hr.clamp(0, 23) as u8,
                count: n.max(0) as u64,
            });
        }
        Ok(out)
    }

    async fn count_all_types(&self, tenant: &TenantContext) -> StorageResult<Vec<(String, u64)>> {
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let pipeline = vec![
            doc! { "$match": { "tenant_id": tenant_id, "is_deleted": false } },
            doc! { "$group": { "_id": "$resource_type", "n": { "$sum": 1 } } },
        ];
        grouped_string_counts(resources, pipeline).await
    }

    async fn count_by_types(
        &self,
        tenant: &TenantContext,
        resource_types: &[&str],
    ) -> StorageResult<Vec<(String, u64)>> {
        // Nothing to match; avoid an empty `$in` round-trip.
        if resource_types.is_empty() {
            return Ok(Vec::new());
        }
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        // Same filters as `count_all_types` plus a `resource_type IN (...)` restriction.
        let pipeline = vec![
            doc! { "$match": {
                "tenant_id": tenant_id,
                "is_deleted": false,
                "resource_type": { "$in": resource_types.to_vec() },
            }},
            doc! { "$group": { "_id": "$resource_type", "n": { "$sum": 1 } } },
        ];
        grouped_string_counts(resources, pipeline).await
    }

    async fn count_by_tenant(&self) -> StorageResult<Vec<(String, u64)>> {
        // Cross-tenant admin aggregate (see trait docs): no tenant filter.
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let pipeline = vec![
            doc! { "$match": { "is_deleted": false } },
            doc! { "$group": { "_id": "$tenant_id", "n": { "$sum": 1 } } },
        ];
        grouped_string_counts(resources, pipeline).await
    }

    fn supports_type_counts(&self) -> bool {
        true
    }

    async fn latest_write_marker(
        &self,
        tenant: &TenantContext,
        recent_since: Option<DateTime<Utc>>,
    ) -> StorageResult<Option<crate::core::WriteMarker>> {
        /// Cap on `recent_writes`: a change detector, not a figure (#1078).
        const RECENT_CAP: u64 = 10_000;

        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        // Newest timestamp: the first key of `idx_history_system_updated`
        // (`tenant_id: 1, last_updated: -1, ...`) for this tenant. Projecting
        // only `last_updated` (and dropping `_id`) keeps it a covered query —
        // one index key, no document fetch, no in-memory sort.
        let newest = history
            .find_one(doc! { "tenant_id": tenant_id })
            .sort(doc! { "last_updated": -1_i32 })
            .projection(doc! { "_id": 0_i32, "last_updated": 1_i32 })
            .await
            .or_query_error("Failed to query latest write marker")?;
        let latest = newest
            .as_ref()
            .and_then(|doc| doc.get_datetime("last_updated").ok())
            .map(bson_to_chrono);

        // Recent writes: a range over the same index, stopped at the cap.
        let recent_writes = match recent_since {
            Some(since) => Some(
                history
                    .count_documents(doc! {
                        "tenant_id": tenant_id,
                        "last_updated": { "$gte": chrono_to_bson(since) },
                    })
                    .limit(RECENT_CAP)
                    .await
                    .or_query_error("Failed to count recent writes")?,
            ),
            None => None,
        };

        Ok(Some(crate::core::WriteMarker {
            latest,
            recent_writes,
        }))
    }

    fn supports_tenant_registry(&self) -> bool {
        true
    }

    async fn list_tenants(&self) -> StorageResult<Vec<crate::core::TenantRecord>> {
        let db = self.get_database().await?;
        let tenants = db.collection::<Document>(MongoBackend::TENANTS_COLLECTION);
        let mut cursor = tenants
            .find(doc! {})
            .sort(doc! { "created_at": 1, "id": 1 })
            .await
            .map_err(|e| internal_error(format!("query list_tenants: {}", e)))?;
        let mut out = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|e| internal_error(format!("list_tenants cursor advance: {}", e)))?
        {
            let doc = cursor
                .deserialize_current()
                .map_err(|e| internal_error(format!("list_tenants cursor deserialize: {}", e)))?;
            out.push(tenant_record_from_doc(&doc)?);
        }
        Ok(out)
    }

    async fn get_tenant(&self, id: &str) -> StorageResult<Option<crate::core::TenantRecord>> {
        let db = self.get_database().await?;
        let tenants = db.collection::<Document>(MongoBackend::TENANTS_COLLECTION);
        let doc = tenants
            .find_one(doc! { "id": id })
            .await
            .map_err(|e| internal_error(format!("query get_tenant: {}", e)))?;
        doc.map(|d| tenant_record_from_doc(&d)).transpose()
    }

    async fn register_tenant(
        &self,
        id: &str,
        display_name: Option<&str>,
    ) -> StorageResult<crate::core::TenantRecord> {
        // Backstop for the canonical tenant-id contract (issue #385). MongoDB
        // matches `tenant_id` as an exact BSON string under the default
        // (case-sensitive) collation, so it has no derivation to protect — this
        // keeps the precondition uniform across every implementation.
        self.ensure_canonical_tenant_id(id)?;
        let db = self.get_database().await?;
        let tenants = db.collection::<Document>(MongoBackend::TENANTS_COLLECTION);
        // RFC 3339 string, matching the SQLite registry's `created_at` format so
        // the admin API is byte-identical across backends.
        let created_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        // Plain insert so a duplicate id surfaces as a unique-index error; the
        // admin handler pre-checks existence and returns 409, so reaching here
        // with a duplicate is a race and a 500 is acceptable.
        tenants
            .insert_one(doc! {
                "id": id,
                "display_name": display_name,
                "created_at": &created_at,
            })
            .await
            .map_err(|e| internal_error(format!("register_tenant: {}", e)))?;
        Ok(crate::core::TenantRecord {
            id: id.to_string(),
            display_name: display_name.map(str::to_string),
            created_at,
        })
    }

    async fn deregister_tenant(&self, id: &str) -> StorageResult<bool> {
        crate::tenant::ensure_mutable_tenant(id)?;
        let db = self.get_database().await?;
        let tenants = db.collection::<Document>(MongoBackend::TENANTS_COLLECTION);
        let result = tenants
            .delete_one(doc! { "id": id })
            .await
            .map_err(|e| internal_error(format!("deregister_tenant: {}", e)))?;
        Ok(result.deleted_count > 0)
    }

    async fn purge_tenant_data(&self, id: &str) -> StorageResult<u64> {
        crate::tenant::ensure_mutable_tenant(id)?;
        let db = self.get_database().await?;
        // Count current-version docs first (soft-deleted included, mirroring the
        // SQLite purge) so we can report what was removed.
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let removed = resources
            .count_documents(doc! { "tenant_id": id })
            .await
            .or_query_error("purge count")?;
        for collection in [
            MongoBackend::SEARCH_INDEX_COLLECTION,
            MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION,
            MongoBackend::RESOURCE_HISTORY_COLLECTION,
            MongoBackend::RESOURCES_COLLECTION,
        ] {
            db.collection::<Document>(collection)
                .delete_many(doc! { "tenant_id": id })
                .await
                .or_query_error(&format!("purge delete ({collection})"))?;
        }
        // Provider-side Bulk Submit submissions are tenant-keyed documents (#772).
        db.collection::<Document>(
            crate::backends::mongodb::bulk_provider::BULK_PROVIDER_COLLECTION,
        )
        .delete_many(doc! { "tenant_id": id })
        .await
        .or_query_error("purge delete (bulk_provider_submissions)")?;
        // Per-user settings are keyed by user, not tenant, so the deletes above
        // do not reach them — but a client stores PHI-derived query strings in
        // them, which belong to this tenant (issue #313).
        let settings = SettingsStore::purge_tenant_settings(self, id).await?;
        if settings > 0 {
            tracing::info!(
                tenant = %id,
                documents = settings,
                "purged tenant-scoped content from user settings documents"
            );
        }
        Ok(removed)
    }
}

/// Reads a registry document into a [`TenantRecord`](crate::core::TenantRecord).
fn tenant_record_from_doc(doc: &Document) -> StorageResult<crate::core::TenantRecord> {
    let id = doc
        .get_str("id")
        .map_err(|e| internal_error(format!("tenant record missing id: {}", e)))?
        .to_string();
    let created_at = doc
        .get_str("created_at")
        .map_err(|e| internal_error(format!("tenant record missing created_at: {}", e)))?
        .to_string();
    Ok(crate::core::TenantRecord {
        id,
        display_name: doc.get_str("display_name").ok().map(str::to_string),
        created_at,
    })
}

/// Runs a `$group`-by-string aggregation and collects `(_id, n)` pairs, where
/// `_id` is a string key and `n` a `$sum` count. Shared by `count_all_types`
/// and `count_by_tenant`.
async fn grouped_string_counts(
    collection: mongodb::Collection<Document>,
    pipeline: Vec<Document>,
) -> StorageResult<Vec<(String, u64)>> {
    let mut cursor = collection
        .aggregate(pipeline)
        .await
        .or_query_error("Failed to aggregate grouped counts")?;
    let mut out = Vec::new();
    while cursor
        .advance()
        .await
        .or_query_error("grouped counts cursor advance")?
    {
        let doc = cursor
            .deserialize_current()
            .or_query_error("grouped counts cursor deserialize")?;
        let key = doc.get_str("_id").unwrap_or_default().to_string();
        if key.is_empty() {
            continue;
        }
        let n = doc
            .get_i32("n")
            .map(i64::from)
            .or_else(|_| doc.get_i64("n"))
            .unwrap_or(0);
        out.push((key, n.max(0) as u64));
    }
    Ok(out)
}

/// One resource's `search_index` contribution, split by destination
/// collection: its own rows go to `search_index`, and rows extracted from its
/// `contained` entries go to `search_index_contained`.
#[derive(Debug, Default)]
pub(super) struct SearchIndexDocuments {
    /// The resource's own rows, for `search_index`.
    pub own: Vec<Document>,
    /// Rows extracted from `contained` entries, for `search_index_contained`.
    pub contained: Vec<Document>,
}

impl SearchIndexDocuments {
    pub fn is_empty(&self) -> bool {
        self.own.is_empty() && self.contained.is_empty()
    }
}

impl MongoBackend {
    /// One attempt at [`ResourceStorage::update`]: the compare-and-swap on
    /// `current`'s version, the history row and the search index, in one
    /// transaction where the deployment has them. A write the server refused
    /// because another writer holds the document comes back as
    /// [`WriteAttemptError::Conflict`] rather than as an `Internal` error.
    async fn update_attempt(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> Result<StoredResource, WriteAttemptError> {
        let resource_type = current.resource_type();
        tenant.check_permission(Operation::Update, resource_type)?;

        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let (mut session, transaction_active) = begin_best_effort_multi_write_session(&db).await;
        let tenant_id = tenant.tenant_id().as_str();
        let id = current.id();

        let current_filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": id,
            "is_deleted": false,
        };

        let maybe_existing = if let Some(active_session) = session.as_mut() {
            resources
                .find_one(current_filter.clone())
                .session(active_session)
                .await
                .map_err(|e| {
                    WriteAttemptError::driver("Failed to load current resource (session)", e)
                })?
        } else {
            resources
                .find_one(current_filter)
                .await
                .map_err(|e| internal_error(format!("Failed to load current resource: {}", e)))?
        };

        let Some(existing_doc) = maybe_existing else {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            })
            .into());
        };

        let actual_version = existing_doc
            .get_str("version_id")
            .map_err(|e| internal_error(format!("Missing current version: {}", e)))?
            .to_string();

        if actual_version != current.version_id() {
            return Err(
                StorageError::Concurrency(ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: current.version_id().to_string(),
                    actual_version,
                })
                .into(),
            );
        }

        let new_version = next_version(current.version_id())?;

        let mut resource = resource;
        ensure_resource_identity(resource_type, id, &mut resource);
        let payload = value_to_document(&resource)?;

        let now = Utc::now();
        let now_bson = chrono_to_bson(now);
        let fhir_version = current.fhir_version();
        let fhir_version_str = fhir_version.as_mime_param().to_string();

        let update_filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": id,
            "version_id": current.version_id(),
            "is_deleted": false,
        };
        let update_doc = doc! {
            "$set": {
                "version_id": &new_version,
                "data": Bson::Document(payload.clone()),
                "last_updated": now_bson,
                "is_deleted": false,
                "deleted_at": Bson::Null,
                "fhir_version": &fhir_version_str,
            }
        };

        let update_result = if let Some(active_session) = session.as_mut() {
            resources
                .update_one(update_filter.clone(), update_doc.clone())
                .session(active_session)
                .await
                .map_err(|e| WriteAttemptError::driver("Failed to update resource (session)", e))?
        } else {
            resources
                .update_one(update_filter, update_doc)
                .await
                .map_err(|e| internal_error(format!("Failed to update resource: {}", e)))?
        };

        if update_result.matched_count == 0 {
            let latest = resources
                .find_one(doc! {
                    "tenant_id": tenant_id,
                    "resource_type": resource_type,
                    "id": id,
                })
                .await
                .map_err(|e| {
                    internal_error(format!("Failed to reload version conflict state: {}", e))
                })?;

            let actual = latest
                .as_ref()
                .and_then(|d| d.get_str("version_id").ok())
                .unwrap_or("unknown")
                .to_string();

            return Err(
                StorageError::Concurrency(ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: current.version_id().to_string(),
                    actual_version: actual,
                })
                .into(),
            );
        }

        let created_at = extract_created_at(&existing_doc, now);

        let history_doc = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": id,
            "version_id": &new_version,
            "data": Bson::Document(payload),
            "created_at": chrono_to_bson(created_at),
            "last_updated": now_bson,
            "is_deleted": false,
            "deleted_at": Bson::Null,
            "fhir_version": fhir_version_str,
        };

        if let Some(active_session) = session.as_mut() {
            history
                .insert_one(history_doc)
                .session(active_session)
                .await
                .map_err(|e| {
                    WriteAttemptError::driver("Failed to insert updated history row (session)", e)
                })?;
        } else {
            history.insert_one(history_doc).await.map_err(|e| {
                internal_error(format!("Failed to insert updated history row: {}", e))
            })?;
        }

        self.index_resource(&db, tenant_id, resource_type, id, &resource, &mut session)
            .await?;

        try_commit_best_effort_multi_write_session(&mut session, transaction_active)
            .await
            .map_err(|e| {
                WriteAttemptError::driver("Failed to commit MongoDB transaction after update", e)
            })?;

        // A SearchParameter update may change a tenant's overlay (status flips,
        // expression edits): refresh the stored-param cache and drop registries.
        // This must run after the commit above: the reload reads the
        // `resources` collection without the session, so it cannot observe the
        // update while the transaction is still open.
        if resource_type == "SearchParameter" {
            if let Err(e) = self.reload_stored_cache_for_tenant(tenant_id).await {
                tracing::warn!("SearchParameter cache reload failed: {e}");
            }
        }

        Ok(StoredResource::from_storage(
            resource_type,
            id,
            new_version,
            tenant.tenant_id().clone(),
            resource,
            created_at,
            now,
            None,
            fhir_version,
        ))
    }

    /// The version of the live (not deleted) resource, read outside any
    /// session — what a write that just lost a race reports as the version it
    /// lost to.
    async fn live_version(
        &self,
        resources: &Collection<Document>,
        tenant_id: &str,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<String>> {
        let live = resources
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
                "is_deleted": false,
            })
            .await
            .map_err(|e| internal_error(format!("Failed to reload current version: {}", e)))?;
        Ok(live.and_then(|d| d.get_str("version_id").ok().map(str::to_string)))
    }

    /// Soft-deletes a resource, optionally only at `expected_version`
    /// ([`ResourceStorage::delete`] / [`ResourceStorage::delete_versioned`]).
    ///
    /// The tombstone `update_one` carries the version in its filter, so the
    /// comparison and the delete are one conditional write whether or not the
    /// deployment supports transactions. `expected_version` is checked against
    /// the document that filter is then built from: a `DELETE` with `If-Match`
    /// used to be evaluated above this layer against an earlier read and then
    /// deleted whatever version was current by the time it got here (#1404).
    async fn soft_delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: Option<&str>,
    ) -> StorageResult<()> {
        let mut retried = false;
        loop {
            let conflict = match self
                .soft_delete_attempt(tenant, resource_type, id, expected_version)
                .await
            {
                Ok(()) => return Ok(()),
                Err(WriteAttemptError::Storage(e)) => return Err(e),
                Err(WriteAttemptError::Conflict(e)) => e,
            };

            // ONE more attempt, and only for a delete with no precondition:
            // "delete whatever is current" means the same thing after the
            // other writer commits, which is the driver's recommended handling
            // of a transient transaction error. A delete that names a version
            // is never run again — the writer it lost to has moved the resource
            // on, and the client must see that (`VersionConflict` -> 409).
            if expected_version.is_none() && !retried {
                retried = true;
                tracing::debug!(
                    resource_type,
                    id,
                    error = %conflict,
                    "MongoDB write conflict on an unconditional delete; retrying once"
                );
                tokio::time::sleep(WRITE_CONFLICT_RETRY_DELAY).await;
                continue;
            }

            return Err(self
                .lost_race(
                    tenant,
                    resource_type,
                    id,
                    expected_version.unwrap_or("unknown"),
                    &conflict,
                )
                .await);
        }
    }

    /// Reports a write the server refused with `WriteConflict` as the
    /// concurrency error it is: `VersionConflict` against whatever is live now,
    /// or `NotFound` when the winner was a delete. The winner may not have
    /// committed yet, in which case the live version still reads as the
    /// expected one and is reported as `unknown` rather than as a conflict of a
    /// version with itself. A failure of this read must not turn a 409 back
    /// into a 500, so it degrades to `unknown` too.
    async fn lost_race(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
        conflict: &MongoError,
    ) -> StorageError {
        tracing::debug!(
            resource_type,
            id,
            expected_version,
            error = %conflict,
            "MongoDB write conflict: a concurrent writer won"
        );

        let live = match self.get_database().await {
            Ok(db) => {
                let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
                self.live_version(&resources, tenant.tenant_id().as_str(), resource_type, id)
                    .await
            }
            Err(e) => Err(e),
        };

        match live {
            Ok(None) => StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }),
            Ok(Some(actual)) if actual != expected_version => {
                StorageError::Concurrency(ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: expected_version.to_string(),
                    actual_version: actual,
                })
            }
            Ok(Some(_)) | Err(_) => StorageError::Concurrency(ConcurrencyError::VersionConflict {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
                expected_version: expected_version.to_string(),
                actual_version: "unknown".to_string(),
            }),
        }
    }

    /// One attempt at [`Self::soft_delete`]; see [`Self::update_attempt`].
    async fn soft_delete_attempt(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: Option<&str>,
    ) -> Result<(), WriteAttemptError> {
        tenant.check_permission(Operation::Delete, resource_type)?;

        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let (mut session, transaction_active) = begin_best_effort_multi_write_session(&db).await;
        let tenant_id = tenant.tenant_id().as_str();

        let delete_lookup_filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": id,
            "is_deleted": false,
        };

        let maybe_existing = if let Some(active_session) = session.as_mut() {
            resources
                .find_one(delete_lookup_filter.clone())
                .session(active_session)
                .await
                .map_err(|e| {
                    WriteAttemptError::driver("Failed to check resource before delete (session)", e)
                })?
        } else {
            resources
                .find_one(delete_lookup_filter)
                .await
                .map_err(|e| {
                    internal_error(format!("Failed to check resource before delete: {}", e))
                })?
        };

        let Some(existing_doc) = maybe_existing else {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            })
            .into());
        };

        let current_version = existing_doc
            .get_str("version_id")
            .map_err(|e| internal_error(format!("Missing current version: {}", e)))?
            .to_string();
        if let Some(expected) = expected_version
            && expected != current_version
        {
            return Err(
                StorageError::Concurrency(ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: expected.to_string(),
                    actual_version: current_version,
                })
                .into(),
            );
        }
        let new_version = next_version(&current_version)?;

        let payload = existing_doc
            .get_document("data")
            .map_err(|e| internal_error(format!("Missing resource payload: {}", e)))?
            .clone();
        let fhir_version = existing_doc
            .get_str("fhir_version")
            .unwrap_or("4.0")
            .to_string();
        let created_at = extract_created_at(&existing_doc, Utc::now());

        let now = Utc::now();
        let now_bson = chrono_to_bson(now);

        let delete_update_filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": id,
            "version_id": &current_version,
            "is_deleted": false,
        };
        let delete_update_doc = doc! {
            "$set": {
                "version_id": &new_version,
                "is_deleted": true,
                "deleted_at": now_bson,
                "last_updated": now_bson,
            }
        };

        let update_result = if let Some(active_session) = session.as_mut() {
            resources
                .update_one(delete_update_filter.clone(), delete_update_doc.clone())
                .session(active_session)
                .await
                .map_err(|e| {
                    WriteAttemptError::driver("Failed to soft-delete resource (session)", e)
                })?
        } else {
            resources
                .update_one(delete_update_filter, delete_update_doc)
                .await
                .map_err(|e| internal_error(format!("Failed to soft-delete resource: {}", e)))?
        };

        if update_result.matched_count == 0 {
            // A writer got in after the read above (only possible without a
            // transaction). A versioned delete says which way it lost.
            if let Some(expected) = expected_version
                && let Some(actual) = self
                    .live_version(&resources, tenant_id, resource_type, id)
                    .await?
            {
                return Err(
                    StorageError::Concurrency(ConcurrencyError::VersionConflict {
                        resource_type: resource_type.to_string(),
                        id: id.to_string(),
                        expected_version: expected.to_string(),
                        actual_version: actual,
                    })
                    .into(),
                );
            }
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            })
            .into());
        }

        let history_doc = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": id,
            "version_id": &new_version,
            "data": Bson::Document(payload),
            "created_at": chrono_to_bson(created_at),
            "last_updated": now_bson,
            "is_deleted": true,
            "deleted_at": now_bson,
            "fhir_version": fhir_version,
        };

        if let Some(active_session) = session.as_mut() {
            history
                .insert_one(history_doc)
                .session(active_session)
                .await
                .map_err(|e| {
                    WriteAttemptError::driver("Failed to insert deletion history row (session)", e)
                })?;
        } else {
            history.insert_one(history_doc).await.map_err(|e| {
                internal_error(format!("Failed to insert deletion history row: {}", e))
            })?;
        }

        self.delete_search_index(&db, tenant_id, resource_type, id, &mut session)
            .await?;

        try_commit_best_effort_multi_write_session(&mut session, transaction_active)
            .await
            .map_err(|e| {
                WriteAttemptError::driver("Failed to commit MongoDB transaction after delete", e)
            })?;

        // A SearchParameter delete may remove a tenant's overlay entry: refresh
        // the stored-param cache and drop registries. This must run after the
        // commit above: the reload reads the `resources` collection
        // without the session, so it cannot observe the delete while the
        // transaction is still open.
        if resource_type == "SearchParameter" {
            if let Err(e) = self.reload_stored_cache_for_tenant(tenant_id).await {
                tracing::warn!("SearchParameter cache reload failed: {e}");
            }
        }

        Ok(())
    }

    /// Brings a soft-deleted resource back to life with new content.
    ///
    /// FHIR permits a deleted resource to be restored by a subsequent update
    /// ([http.html#delete](https://hl7.org/fhir/http.html#delete)), so a `PUT`
    /// onto a deleted id must succeed instead of failing with `Gone`. The
    /// restored resource continues the existing version chain (the deletion
    /// record keeps its version, the restore gets the next one) and keeps the
    /// FHIR version the resource was originally stored under.
    ///
    /// Returns `NotFound` if no deleted document is present — the caller has
    /// already established one exists, so that only happens under a concurrent
    /// write.
    async fn restore_deleted(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        tenant.check_permission(Operation::Update, resource_type)?;

        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let (mut session, transaction_active) = begin_best_effort_multi_write_session(&db).await;
        let tenant_id = tenant.tenant_id().as_str();

        let deleted_filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": id,
            "is_deleted": true,
        };

        let maybe_deleted = if let Some(active_session) = session.as_mut() {
            resources
                .find_one(deleted_filter.clone())
                .session(active_session)
                .await
                .map_err(|e| {
                    internal_error(format!("Failed to read deleted resource (session): {}", e))
                })?
        } else {
            resources
                .find_one(deleted_filter)
                .await
                .map_err(|e| internal_error(format!("Failed to read deleted resource: {}", e)))?
        };

        let Some(deleted_doc) = maybe_deleted else {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        };

        let deleted_version = deleted_doc
            .get_str("version_id")
            .map_err(|e| internal_error(format!("Missing current version: {}", e)))?
            .to_string();
        let new_version = next_version(&deleted_version)?;

        // The restore keeps the FHIR version the resource was stored under.
        let fhir_version_str = deleted_doc
            .get_str("fhir_version")
            .unwrap_or("4.0")
            .to_string();
        let fhir_version = FhirVersion::from_storage(&fhir_version_str)
            .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

        let mut resource = resource;
        ensure_resource_identity(resource_type, id, &mut resource);
        let payload = value_to_document(&resource)?;

        let now = Utc::now();
        let now_bson = chrono_to_bson(now);
        let created_at = extract_created_at(&deleted_doc, now);

        let restore_filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": id,
            "version_id": &deleted_version,
            "is_deleted": true,
        };
        let restore_doc = doc! {
            "$set": {
                "version_id": &new_version,
                "data": Bson::Document(payload.clone()),
                "last_updated": now_bson,
                "is_deleted": false,
                "deleted_at": Bson::Null,
            }
        };

        let update_result = if let Some(active_session) = session.as_mut() {
            resources
                .update_one(restore_filter.clone(), restore_doc.clone())
                .session(active_session)
                .await
                .map_err(|e| {
                    internal_error(format!("Failed to restore resource (session): {}", e))
                })?
        } else {
            resources
                .update_one(restore_filter, restore_doc)
                .await
                .map_err(|e| internal_error(format!("Failed to restore resource: {}", e)))?
        };

        if update_result.matched_count == 0 {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        }

        let history_doc = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": id,
            "version_id": &new_version,
            "data": Bson::Document(payload),
            "created_at": chrono_to_bson(created_at),
            "last_updated": now_bson,
            "is_deleted": false,
            "deleted_at": Bson::Null,
            "fhir_version": fhir_version_str,
        };

        if let Some(active_session) = session.as_mut() {
            history
                .insert_one(history_doc)
                .session(active_session)
                .await
                .map_err(|e| {
                    internal_error(format!(
                        "Failed to insert restore history row (session): {}",
                        e
                    ))
                })?;
        } else {
            history.insert_one(history_doc).await.map_err(|e| {
                internal_error(format!("Failed to insert restore history row: {}", e))
            })?;
        }

        // The delete dropped the search index entries; rebuild them for the
        // resource that is live again (`index_resource` clears stale rows first).
        self.index_resource(&db, tenant_id, resource_type, id, &resource, &mut session)
            .await?;

        commit_best_effort_multi_write_session(&mut session, transaction_active, "restore").await?;

        // A restored SearchParameter re-enters a tenant's overlay: refresh the
        // stored-param cache and drop registries. This must run after the
        // commit above: the reload reads the `resources` collection
        // without the session, so it cannot observe the restore while the
        // transaction is still open.
        if resource_type == "SearchParameter" {
            if let Err(e) = self.reload_stored_cache_for_tenant(tenant_id).await {
                tracing::warn!("SearchParameter cache reload failed: {e}");
            }
        }

        Ok(StoredResource::from_storage(
            resource_type,
            id,
            new_version,
            tenant.tenant_id().clone(),
            resource,
            created_at,
            now,
            None,
            fhir_version,
        ))
    }

    /// The `search_index`/`search_index_contained` documents one resource
    /// contributes — every value the extractor yields, split by destination
    /// collection, with no I/O of its own.
    ///
    /// Split out of [`Self::index_resource`] so the batched bulk-submit ingest
    /// (#1000) can build a whole batch's index documents and write them in one
    /// `insert_many`, instead of one `insert_many` per resource. Both callers
    /// therefore index a resource identically by construction.
    pub(super) fn search_index_documents(
        &self,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> SearchIndexDocuments {
        self.search_index_documents_checked(tenant_id, resource_type, resource_id, resource)
            .0
    }

    /// [`Self::search_index_documents`], plus the extraction failure message
    /// (if any) that made this resource fall back to minimal index rows.
    ///
    /// Used by [`ReindexTarget::write_search_entries_page`] so a page can
    /// still write every resource's fallback rows (matching what
    /// [`Self::index_resource`] already does for a single resource) while
    /// still reporting that resource as failed — the way the old, per-resource
    /// `write_search_entries` always did — instead of a batched rewrite
    /// silently turning a corrupt resource into a quiet `Ok`.
    pub(super) fn search_index_documents_checked(
        &self,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> (SearchIndexDocuments, Option<String>) {
        let (own, failure) = match self
            .tenant_extractor(tenant_id)
            .extract(resource, resource_type)
        {
            Ok(values) => (
                values
                    .iter()
                    .filter_map(|value| {
                        self.build_search_index_document(
                            tenant_id,
                            resource_type,
                            resource_id,
                            value,
                        )
                    })
                    .collect::<Vec<_>>(),
                None,
            ),
            Err(e) => {
                tracing::warn!(
                    "Search extraction failed for {}/{}: {}. Using minimal fallback index values.",
                    resource_type,
                    resource_id,
                    e
                );
                (
                    self.index_minimal_fallback_documents(
                        tenant_id,
                        resource_type,
                        resource_id,
                        resource,
                    ),
                    Some(format!("Search parameter extraction failed: {e}")),
                )
            }
        };

        // Also index any contained resources for `_contained` search, into
        // their own collection (`search_index_contained`) rather than mixed
        // into `own`. These rows share the container's
        // (resource_type, resource_id) — so the delete-by-(type,id) that
        // precedes a re-index cleans them too — and carry the contained
        // resource's type and local id.
        let mut contained = Vec::new();
        for c in self.tenant_extractor(tenant_id).extract_contained(resource) {
            for value in &c.values {
                if let Some(d) = self.build_contained_index_document(
                    tenant_id,
                    resource_type,
                    resource_id,
                    &c.contained_type,
                    &c.local_id,
                    value,
                ) {
                    contained.push(d);
                }
            }
        }

        (SearchIndexDocuments { own, contained }, failure)
    }

    pub(crate) async fn index_resource(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
        session: &mut Option<ClientSession>,
    ) -> StorageResult<()> {
        if self.is_search_offloaded() {
            return Ok(());
        }

        self.delete_search_index(db, tenant_id, resource_type, resource_id, session)
            .await?;

        let docs = self.search_index_documents(tenant_id, resource_type, resource_id, resource);

        if docs.is_empty() {
            return Ok(());
        }

        self.insert_search_index_documents(db, docs, session.as_mut())
            .await
    }

    /// Runs `insert_many(docs)` on `collection` through `session` when given,
    /// skipping the call entirely when `docs` is empty. Shared by
    /// [`Self::insert_search_index_documents`]'s two collection inserts so
    /// they read alike.
    async fn insert_indexed_docs(
        collection: &Collection<Document>,
        docs: Vec<Document>,
        session: &mut Option<&mut ClientSession>,
        error_prefix: &str,
    ) -> StorageResult<()> {
        if docs.is_empty() {
            return Ok(());
        }

        if let Some(active_session) = session.as_mut() {
            collection
                .insert_many(docs)
                .session(&mut **active_session)
                .await
                .map_err(|e| internal_driver_error(error_prefix, e))?;
        } else {
            collection
                .insert_many(docs)
                .await
                .map_err(|e| internal_driver_error(error_prefix, e))?;
        }

        Ok(())
    }

    /// Inserts one resource's [`SearchIndexDocuments`]: `own` rows into
    /// `search_index`, `contained` rows into `search_index_contained`, each
    /// only when non-empty, through `session` when given. Used by every
    /// insert path so a resource's own rows and its contained rows land in
    /// the right collection by construction.
    async fn insert_search_index_documents(
        &self,
        db: &mongodb::Database,
        docs: SearchIndexDocuments,
        session: Option<&mut ClientSession>,
    ) -> StorageResult<()> {
        let mut session = session;

        let own_collection = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
        Self::insert_indexed_docs(
            &own_collection,
            docs.own,
            &mut session,
            "Failed to insert search index entries",
        )
        .await?;

        let contained_collection =
            db.collection::<Document>(MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION);
        Self::insert_indexed_docs(
            &contained_collection,
            docs.contained,
            &mut session,
            "Failed to insert search_index_contained entries",
        )
        .await?;

        Ok(())
    }

    /// Runs `delete_many(filter)` on `collection` through `session` when
    /// given. Shared by [`Self::delete_search_index_rows_for`]'s two
    /// collection deletes so they read alike (and alongside
    /// [`Self::insert_indexed_docs`]).
    async fn delete_indexed_docs(
        collection: &Collection<Document>,
        filter: Document,
        session: &mut Option<&mut ClientSession>,
        error_prefix: &str,
    ) -> StorageResult<()> {
        if let Some(active_session) = session.as_mut() {
            collection
                .delete_many(filter)
                .session(&mut **active_session)
                .await
                .map_err(|e| internal_driver_error(error_prefix, e))?;
        } else {
            collection
                .delete_many(filter)
                .await
                .map_err(|e| internal_driver_error(error_prefix, e))?;
        }

        Ok(())
    }

    /// Deletes rows matching `{tenant_id, resource_type, resource_id:
    /// id_filter}` from `search_index` and then `search_index_contained`,
    /// through `session` when given. `id_filter` is either a single id
    /// (`Bson::String`) or an `$in` filter over multiple ids. Used by every
    /// delete path so a resource's own rows and its contained rows are
    /// removed together by construction.
    async fn delete_search_index_rows_for(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resource_type: &str,
        id_filter: Bson,
        session: Option<&mut ClientSession>,
    ) -> StorageResult<()> {
        let mut session = session;
        let filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "resource_id": id_filter,
        };

        let own_collection = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
        Self::delete_indexed_docs(
            &own_collection,
            filter.clone(),
            &mut session,
            "Failed to delete search index entries",
        )
        .await?;

        let contained_collection =
            db.collection::<Document>(MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION);
        Self::delete_indexed_docs(
            &contained_collection,
            filter,
            &mut session,
            "Failed to delete search_index_contained entries",
        )
        .await?;

        Ok(())
    }

    pub(crate) async fn delete_search_index(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        session: &mut Option<ClientSession>,
    ) -> StorageResult<()> {
        if self.is_search_offloaded() {
            return Ok(());
        }

        self.delete_search_index_rows_for(
            db,
            tenant_id,
            resource_type,
            Bson::String(resource_id.to_string()),
            session.as_mut(),
        )
        .await
    }

    fn build_search_index_document(
        &self,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        value: &ExtractedValue,
    ) -> Option<Document> {
        let mut doc = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "resource_id": resource_id,
            "param_name": &value.param_name,
            "param_url": &value.param_url,
        };

        match &value.value {
            IndexValue::String(v) => {
                // Stored as written: `:exact` is case-sensitive, and the
                // insensitive variants use a case-insensitive regex instead of
                // a pre-lowercased value.
                doc.insert("value_string", v.clone());
            }
            IndexValue::Token {
                system,
                code,
                display,
                identifier_type_system,
                identifier_type_code,
            } => {
                if let Some(system) = system {
                    doc.insert("value_token_system", system.clone());
                }
                doc.insert("value_token_code", code.clone());
                if let Some(display) = display {
                    doc.insert("value_token_display", display.clone());
                }
                if let Some(type_system) = identifier_type_system {
                    doc.insert("value_identifier_type_system", type_system.clone());
                }
                if let Some(type_code) = identifier_type_code {
                    doc.insert("value_identifier_type_code", type_code.clone());
                }
            }
            IndexValue::Date {
                value: date,
                precision,
                end,
            } => {
                let normalized = match normalize_date_for_mongo(date) {
                    Some(v) => v,
                    None => {
                        tracing::warn!(
                            "Skipping invalid date index value '{}' for parameter '{}'",
                            date,
                            value.param_name
                        );
                        return None;
                    }
                };
                // #1391: the row stores the range `[value_date, value_date_end)`
                // a range-aware search compares against. The shared reading
                // when the start is in the FHIR grammar; otherwise the end is
                // derived from the lenient start read above.
                let resolution = crate::search::StorageResolution::Millis;
                let range_end = crate::search::indexed_range(&value.value, resolution)
                    .map(|(_, end)| end)
                    .or_else(|| {
                        crate::search::indexed_end(normalized, *precision, end, resolution)
                    });
                let Some(range_end) = range_end else {
                    tracing::warn!(
                        "Skipping date index value '{}' for parameter '{}': its Period end is not a date",
                        date,
                        value.param_name
                    );
                    return None;
                };
                doc.insert("value_date", chrono_to_bson(normalized));
                doc.insert("value_date_end", chrono_to_bson(range_end));
                doc.insert("value_date_precision", precision.to_string());
            }
            IndexValue::Number(v) => {
                doc.insert("value_number", *v);
            }
            IndexValue::Quantity {
                value,
                unit,
                system,
                ..
            } => {
                doc.insert("value_quantity_value", *value);
                if let Some(unit) = unit {
                    doc.insert("value_quantity_unit", unit.clone());
                }
                if let Some(system) = system {
                    doc.insert("value_quantity_system", system.clone());
                }
            }
            IndexValue::Reference {
                reference, display, ..
            } => {
                doc.insert("value_reference", reference.clone());
                if let Some(d) = display {
                    doc.insert("value_reference_display", d.clone());
                }
            }
            IndexValue::Uri(uri) => {
                doc.insert("value_uri", uri.clone());
            }
        }

        if let Some(group) = value.composite_group {
            doc.insert("composite_group", group as i32);
        }
        if let Some(slot) = value.composite_slot {
            doc.insert("composite_slot", i32::from(slot));
        }

        Some(doc)
    }

    /// Builds a contained-resource search-index document (`_contained` search):
    /// the same value columns as [`Self::build_search_index_document`], with the
    /// container's `(resource_type, resource_id)`, carrying the contained
    /// resource's type and local id; written to `search_index_contained`.
    fn build_contained_index_document(
        &self,
        tenant_id: &str,
        container_type: &str,
        container_id: &str,
        contained_type: &str,
        contained_local_id: &str,
        value: &ExtractedValue,
    ) -> Option<Document> {
        let mut doc =
            self.build_search_index_document(tenant_id, container_type, container_id, value)?;
        doc.insert("contained_type", contained_type);
        doc.insert("contained_local_id", contained_local_id);
        Some(doc)
    }

    fn index_minimal_fallback_documents(
        &self,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> Vec<Document> {
        let mut docs = Vec::new();

        let resource_id_value = resource
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or(resource_id);

        docs.push(doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "resource_id": resource_id,
            "param_name": "_id",
            "param_url": "http://hl7.org/fhir/SearchParameter/Resource-id",
            "value_token_code": resource_id_value,
        });

        if let Some(last_updated) = resource
            .get("meta")
            .and_then(|meta| meta.get("lastUpdated"))
            .and_then(|v| v.as_str())
            .and_then(normalize_date_for_mongo)
        {
            docs.push(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "resource_id": resource_id,
                "param_name": "_lastUpdated",
                "param_url": "http://hl7.org/fhir/SearchParameter/Resource-lastUpdated",
                "value_date": chrono_to_bson(last_updated),
            });
        }

        docs
    }
}

#[async_trait]
impl VersionedStorage for MongoBackend {
    async fn vread(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        version_id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let maybe_doc = history
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
                "version_id": version_id,
            })
            .await
            .map_err(|e| internal_error(format!("Failed to read historical version: {}", e)))?;

        let Some(doc) = maybe_doc else {
            return Ok(None);
        };

        let row = parse_history_row(&doc, Some(resource_type), Some(id))?;
        Ok(Some(row.into_stored_resource(tenant)))
    }

    async fn update_with_match(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        let current = self.read(tenant, resource_type, id).await?.ok_or_else(|| {
            StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            })
        })?;

        // `expected_version` is the client's `If-Match` field value, which is a
        // LIST and is satisfied when any listed tag matches (issue #311).
        if !if_match_field_satisfied(expected_version, current.version_id()) {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: normalize_etag(expected_version).to_string(),
                    actual_version: normalize_etag(current.version_id()).to_string(),
                },
            ));
        }

        self.update(tenant, &current, resource).await
    }

    async fn delete_with_match(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
    ) -> StorageResult<()> {
        tenant.check_permission(Operation::Delete, resource_type)?;

        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let maybe_doc = resources
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
                "is_deleted": false,
            })
            .await
            .map_err(|e| {
                internal_error(format!(
                    "Failed to load resource for delete_with_match: {}",
                    e
                ))
            })?;

        let Some(doc) = maybe_doc else {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        };

        let actual = doc.get_str("version_id").map_err(|e| {
            internal_error(format!(
                "Missing current version for delete_with_match: {}",
                e
            ))
        })?;

        // List-aware `If-Match` comparison, shared with every other backend.
        if !if_match_field_satisfied(expected_version, actual) {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: normalize_etag(expected_version).to_string(),
                    actual_version: normalize_etag(actual).to_string(),
                },
            ));
        }

        // Delete exactly the version the precondition was evaluated against.
        // A plain `delete` here was check-then-act: a writer landing after the
        // read above was deleted along with the version the client named
        // (#1404).
        let actual = actual.to_string();
        self.delete_versioned(tenant, resource_type, id, &actual)
            .await
    }

    async fn list_versions(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Vec<String>> {
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let cursor = history
            .find(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
            })
            .await
            .or_query_error("Failed to query version history")?;

        let docs = collect_documents(cursor).await?;
        let mut versions = docs
            .iter()
            .filter_map(|doc| doc.get_str("version_id").ok().map(str::to_string))
            .collect::<Vec<_>>();

        versions.sort_by(|a, b| {
            parse_version_id(a)
                .cmp(&parse_version_id(b))
                .then_with(|| a.cmp(b))
        });

        Ok(versions)
    }
}

#[async_trait]
impl InstanceHistoryProvider for MongoBackend {
    async fn history_instance(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        params: &HistoryParams,
    ) -> StorageResult<HistoryPage> {
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let mut filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": id,
        };
        apply_history_params_filter(&mut filter, params);

        let cursor = history
            .find(filter)
            .await
            .or_query_error("Failed to query instance history")?;

        let docs = collect_documents(cursor).await?;
        let mut rows = docs
            .iter()
            .map(|doc| parse_history_row(doc, Some(resource_type), Some(id)))
            .collect::<StorageResult<Vec<_>>>()?;

        rows.sort_by(|a, b| {
            parse_version_id(&b.version_id)
                .cmp(&parse_version_id(&a.version_id))
                .then_with(|| b.last_updated.cmp(&a.last_updated))
        });

        if let Some(cursor_version) = parse_cursor_version(params) {
            rows.retain(|row| parse_version_id(&row.version_id) < cursor_version);
        }

        let page_len = params.pagination.count as usize;
        let has_more = rows.len() > page_len;
        if has_more {
            rows.truncate(page_len);
        }

        let page_info = if has_more {
            if let Some(last) = rows.last() {
                PageInfo::with_next(PageCursor::new(
                    vec![CursorValue::String(last.version_id.clone())],
                    id.to_string(),
                ))
            } else {
                PageInfo::end()
            }
        } else {
            PageInfo::end()
        };

        let entries = rows
            .into_iter()
            .map(|row| row.into_history_entry(tenant))
            .collect::<Vec<_>>();

        Ok(Page::new(entries, page_info))
    }

    async fn history_instance_count(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<u64> {
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        history
            .count_documents(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
            })
            .await
            .or_query_error("Failed to count instance history")
    }
}

#[async_trait]
impl TypeHistoryProvider for MongoBackend {
    async fn history_type(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        params: &HistoryParams,
    ) -> StorageResult<HistoryPage> {
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let mut filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
        };
        apply_history_params_filter(&mut filter, params);
        if let Some(or_branches) = type_history_cursor_or(params) {
            // A distinct top-level key from the `last_updated` range that
            // `apply_history_params_filter` may have just inserted, so the
            // two AND together implicitly rather than colliding.
            filter.insert("$or", or_branches);
        }

        let opts = FindOptions::builder()
            .sort(type_history_sort())
            .limit(history_fetch_limit(params.pagination.count))
            // Exclusion-style on the only two fields parse_history_row never
            // reads: an inclusion projection risks silently substituting a
            // default for a field it does read (e.g. fhir_version).
            .projection(doc! { "_id": 0, "created_at": 0 })
            .build();

        let cursor = history
            .find(filter)
            .with_options(opts)
            .await
            .or_query_error("Failed to query type history")?;

        let docs = collect_documents(cursor).await?;
        let mut rows = docs
            .iter()
            .map(|doc| parse_history_row(doc, Some(resource_type), None))
            .collect::<StorageResult<Vec<_>>>()?;

        // The server-side sort (matching idx_history_type_updated) already
        // orders the <= count+1 fetched rows by (last_updated desc, id desc);
        // only the version_id tie-break stays in Rust, because version_id is
        // in neither idx_history_type_updated nor idx_history_system_updated
        // (pushing it server-side would reintroduce a blocking SORT, and it
        // is stored as a string, so a server-side $lt/sort on it would
        // misorder "10" ahead of "9" anyway). When a (last_updated, id) tie
        // group straddles the fetch-limit boundary, the server's limit() may
        // now pick an arbitrary subset of that group before this sort runs,
        // so which member lands on the page is no longer guaranteed to be
        // the highest version_id — but any group member beyond the boundary
        // was already dropped by the (ts, id)-only cursor predicate above
        // (shared with SQLite/Postgres), so total loss is unchanged.
        rows.sort_by(|a, b| {
            b.last_updated
                .cmp(&a.last_updated)
                .then_with(|| b.id.cmp(&a.id))
                .then_with(|| parse_version_id(&b.version_id).cmp(&parse_version_id(&a.version_id)))
        });

        let page_len = params.pagination.count as usize;
        let has_more = rows.len() > page_len;
        if has_more {
            rows.truncate(page_len);
        }

        let page_info = if has_more {
            if let Some(last) = rows.last() {
                PageInfo::with_next(PageCursor::new(
                    vec![
                        CursorValue::String(last.last_updated.to_rfc3339()),
                        CursorValue::String(last.id.clone()),
                    ],
                    resource_type.to_string(),
                ))
            } else {
                PageInfo::end()
            }
        } else {
            PageInfo::end()
        };

        let entries = rows
            .into_iter()
            .map(|row| row.into_history_entry(tenant))
            .collect::<Vec<_>>();

        Ok(Page::new(entries, page_info))
    }

    async fn history_type_count(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64> {
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        history
            .count_documents(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
            })
            .await
            .or_query_error("Failed to count type history")
    }
}

#[async_trait]
impl SystemHistoryProvider for MongoBackend {
    async fn history_system(
        &self,
        tenant: &TenantContext,
        params: &HistoryParams,
    ) -> StorageResult<HistoryPage> {
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let mut filter = doc! {
            "tenant_id": tenant_id,
        };
        apply_history_params_filter(&mut filter, params);
        if let Some(or_branches) = system_history_cursor_or(params) {
            filter.insert("$or", or_branches);
        }

        let opts = FindOptions::builder()
            .sort(system_history_sort())
            .limit(history_fetch_limit(params.pagination.count))
            .projection(doc! { "_id": 0, "created_at": 0 })
            .build();

        let cursor = history
            .find(filter)
            .with_options(opts)
            .await
            .or_query_error("Failed to query system history")?;

        let docs = collect_documents(cursor).await?;
        let mut rows = docs
            .iter()
            .map(|doc| parse_history_row(doc, None, None))
            .collect::<StorageResult<Vec<_>>>()?;

        // See the matching comment in history_type: the server-side sort
        // (matching idx_history_system_updated) already orders the fetched
        // page; only the version_id tie-break stays in Rust, and a
        // (last_updated, resource_type, id) tie group straddling the
        // fetch-limit boundary can show an arbitrary member without
        // widening total loss versus today.
        rows.sort_by(|a, b| {
            b.last_updated
                .cmp(&a.last_updated)
                .then_with(|| b.resource_type.cmp(&a.resource_type))
                .then_with(|| b.id.cmp(&a.id))
                .then_with(|| parse_version_id(&b.version_id).cmp(&parse_version_id(&a.version_id)))
        });

        let page_len = params.pagination.count as usize;
        let has_more = rows.len() > page_len;
        if has_more {
            rows.truncate(page_len);
        }

        let page_info = if has_more {
            if let Some(last) = rows.last() {
                PageInfo::with_next(PageCursor::new(
                    vec![
                        CursorValue::String(last.last_updated.to_rfc3339()),
                        CursorValue::String(last.resource_type.clone()),
                        CursorValue::String(last.id.clone()),
                    ],
                    "system".to_string(),
                ))
            } else {
                PageInfo::end()
            }
        } else {
            PageInfo::end()
        };

        let entries = rows
            .into_iter()
            .map(|row| row.into_history_entry(tenant))
            .collect::<Vec<_>>();

        Ok(Page::new(entries, page_info))
    }

    async fn history_system_count(&self, tenant: &TenantContext) -> StorageResult<u64> {
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        history
            .count_documents(doc! {
                "tenant_id": tenant_id,
            })
            .await
            .or_query_error("Failed to count system history")
    }
}

#[async_trait]
impl BundleProvider for MongoBackend {
    /// MongoDB bundles run inside a server-side session transaction
    /// (`begin_required_bundle_transaction_session` … `commit_transaction`),
    /// so the server unwinds on abort or on a dropped session — including the
    /// cancellation case that defeats an in-process compensation log.
    ///
    /// Requires a replica set; `begin_required_bundle_transaction_session`
    /// fails the bundle when transactions are unavailable rather than silently
    /// degrading to non-atomic writes.
    fn supports_atomic_transactions(&self) -> bool {
        true
    }

    /// The in-transaction matcher is the session-scoped one `ifNoneExist`
    /// already uses: index-backed when search is local, and the
    /// `_id`/`identifier` collection scan when it is offloaded, which refuses
    /// the criteria shapes it cannot evaluate itself.
    fn supports_conditional_in_transaction(&self) -> bool {
        true
    }

    /// Runs the bundle in one server-side transaction, and runs it again when the
    /// server aborts it with a `TransientTransactionError` (#1586).
    ///
    /// Inside a transaction MongoDB does not queue a writer behind a conflicting
    /// one: the loser fails at once with `WriteConflict` (112), or is evicted
    /// under cache pressure, and the transaction is aborted. That is the driver's
    /// "run the whole transaction again" signal, and the transaction-bundle
    /// contract is all-or-nothing, so re-running it is invisible to the client:
    /// nothing committed, and the composite storage, the REST side effects and
    /// the audit trail all act only on the `Ok` this returns. A bundle that keeps
    /// losing ends as [`TransactionError::Transient`] (a retryable 503), not as
    /// a `BundleError` blaming the entry it happened to be on. A Bundle whose own
    /// write set does not fit WiredTiger's cache (`TransactionTooLargeForCache`,
    /// 388) is not run again: it ends at once as
    /// [`TransactionError::TooLargeForCache`] (#1837).
    ///
    /// Every attempt starts from the original entries (see
    /// [`Self::bundle_transaction_attempt`]) and reuses one session: the abort
    /// ends the server-side transaction, and `start_transaction` begins the next.
    /// Bounded by [`BUNDLE_TRANSACTION_RETRY`], the configured
    /// `MongoBackendConfig::bundle_transaction_budget` and
    /// `bundle_transaction_deadline`, which an embedder that serves requests
    /// under a timeout sets to fit inside it.
    ///
    /// Admission (#1776, #1806): Bundles are admitted while their entries fit
    /// in `max_concurrent_transaction_bundles` standard Bundles of
    /// `transaction_bundle_weight_entries` entries per backend, so their
    /// uncommitted writes fit WiredTiger's cache. A Bundle counts at least one
    /// entry and at most the whole room, so one larger than the room runs
    /// alone. Waiters are served first come first served and the wait counts
    /// against the deadline when one is set, else against the budget. The room
    /// is taken before the session starts, so a queued Bundle holds no session
    /// and no transaction, and it is kept across replays until the commit.
    /// Dropping a queued request gives up its place.
    ///
    /// Dropping this future — the request timed out, the client went away —
    /// drops the session, which aborts the transaction server-side. The slot
    /// frees immediately while that abort is still in flight, so under request
    /// timeouts the bound can briefly be exceeded by the cancelled Bundles.
    async fn process_transaction_with_patch_validator(
        &self,
        tenant: &TenantContext,
        entries: Vec<BundleEntry>,
        fhir_version: helios_fhir::FhirVersion,
        validator: Option<&dyn PatchCandidateValidator>,
    ) -> Result<BundleResult, TransactionError> {
        // Direct persistence callers may supply criteria only in the URL.
        // REST already sets entry.criteria and skips this compatibility parsing.
        let mut entries = entries;
        for (index, entry) in entries.iter_mut().enumerate() {
            if entry.criteria.is_none()
                && matches!(
                    entry.method,
                    BundleMethod::Put | BundleMethod::Patch | BundleMethod::Delete
                )
                && entry.url.contains('?')
            {
                let target = parse_bundle_entry_target(&entry.url).map_err(|error| {
                    TransactionError::BundleError {
                        index,
                        message: error.to_string(),
                    }
                })?;
                if let BundleEntryTarget::Conditional {
                    resource_type,
                    criteria,
                } = target
                {
                    let parsed = crate::search::parse_conditional_criteria(&criteria);
                    entry.criteria = Some(
                        self.build_search_parameters(tenant, &resource_type, &parsed)
                            .map_err(|error| TransactionError::BundleError {
                                index,
                                message: error.to_string(),
                            })?,
                    );
                }
            }
        }

        let db = self
            .get_database()
            .await
            .map_err(|e| TransactionError::RolledBack {
                reason: format!("Failed to acquire MongoDB database: {}", e),
            })?;

        // The admission carries both clocks: the replay budget counts from
        // admission and the deadline from the call, so the wait spends the
        // deadline but not the replays, and no replay starts that would end
        // past either (#1641, #1806). It also holds the room until the drop
        // after the commit.
        let gate = self.transaction_bundle_gate();
        let admission = gate.admit(entries.len()).await;
        let queued = admission.queued();
        if let Some(limit) = gate.limit() {
            tracing::debug!(
                limit,
                capacity = gate.capacity(),
                weight = gate.weight(entries.len()),
                queued_ms = queued.as_millis() as u64,
                entries = entries.len(),
                "transaction bundle admitted"
            );
        }

        let mut session = begin_required_bundle_transaction_session(&db).await?;

        let mut attempts: u32 = 1;
        let (results, pending_search_parameter_changes) = loop {
            let attempt_started = std::time::Instant::now();
            let (entry, code, reason) = match self
                .bundle_transaction_attempt(
                    &db,
                    &mut session,
                    tenant,
                    fhir_version,
                    validator,
                    &entries,
                )
                .await
            {
                Ok(committed) => break committed,
                Err(BundleAttemptError::Failed(error)) => return Err(error),
                Err(BundleAttemptError::TransientAbort {
                    entry,
                    code,
                    reason,
                }) => (entry, code, reason),
            };

            let attempt_duration = attempt_started.elapsed();
            let Some(backoff) = admission.next_attempt_delay(
                self.config(),
                attempts,
                attempt_duration,
                jitter_fraction(),
            ) else {
                return Err(TransactionError::Transient { attempts, reason });
            };

            tracing::warn!(
                attempt = attempts,
                max_attempts = BUNDLE_TRANSACTION_RETRY.max_attempts,
                entries = entries.len(),
                failed_entry = entry,
                error_code = code,
                backoff_ms = backoff.as_millis() as u64,
                attempt_ms = attempt_duration.as_millis() as u64,
                queued_ms = queued.as_millis() as u64,
                "transaction bundle aborted by a transient mongodb error; retrying: {reason}"
            );
            tokio::time::sleep(backoff).await;

            session
                .start_transaction()
                .await
                .map_err(|e| TransactionError::RolledBack {
                    reason: format!("Failed to start MongoDB transaction: {}", e),
                })?;
            attempts += 1;
        };

        if attempts > 1 {
            tracing::info!(
                attempts,
                entries = entries.len(),
                elapsed_ms = admission.elapsed().since_call.as_millis() as u64,
                queued_ms = queued.as_millis() as u64,
                "transaction bundle committed after a transient mongodb abort"
            );
        }

        drop(admission);

        // Any SearchParameter change in this transaction alters a tenant's
        // overlay — refresh the stored-param cache and drop the cached
        // registries so the next access reflects the committed writes. Once,
        // after the commit that counted, never after an aborted attempt.
        if !pending_search_parameter_changes.is_empty() {
            if let Err(e) = self
                .reload_stored_cache_for_tenant(tenant.tenant_id().as_str())
                .await
            {
                tracing::warn!("SearchParameter cache reload failed: {e}");
            }
        }

        Ok(BundleResult {
            bundle_type: BundleType::Transaction,
            entries: results,
        })
    }
}

impl MongoBackend {
    /// One attempt at a bundle transaction: every entry in order, then commit.
    ///
    /// Returns the entry results and the staged SearchParameter changes when
    /// the transaction committed. Otherwise it returns why it did not, with the
    /// server-side transaction already aborted (or failed at commit), so the
    /// caller may begin another on the same session.
    ///
    /// Each attempt clones the original entries before pinning references.
    /// Pins and payload rewrites stay in that attempt: a replay resolves its
    /// own targets and cannot retain identities from an aborted transaction.
    async fn bundle_transaction_attempt(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant: &TenantContext,
        fhir_version: helios_fhir::FhirVersion,
        validator: Option<&dyn PatchCandidateValidator>,
        entries: &[BundleEntry],
    ) -> Result<(Vec<BundleEntryResult>, Vec<PendingSearchParameterChange>), BundleAttemptError>
    {
        let mut entries = entries.to_vec();
        // URL-borne conditional entries (`PUT/DELETE [type]?[criteria]`)
        // resolve against the transaction's starting view before any entry is
        // written, and an overlap between resolved identities and the other
        // entries fails the bundle (R4 §3.1.0.11.2; #859). Each attempt
        // resolves afresh against its own transaction's view.
        let targets = match self
            .resolve_conditional_targets_in_bundle_transaction(db, session, tenant, &entries)
            .await
        {
            Ok(targets) => targets,
            Err(e) => {
                let _ = session.abort_transaction().await;
                return Err(e);
            }
        };

        let mut results = Vec::with_capacity(entries.len());
        let mut error_info: Option<(usize, String)> = None;
        let mut patch_error: Option<TransactionError> = None;
        let references = match self
            .pin_bundle_references(db, session, tenant, &mut entries, &targets)
            .await
        {
            Ok(references) => references,
            Err((index, error)) => {
                let _ = session.abort_transaction().await;
                return Err(entry_attempt_error(index, entries.len(), &error));
            }
        };
        let mut bundle_state = BundleTransactionState::new(references);
        let mut pending_search_parameter_changes: Vec<PendingSearchParameterChange> = Vec::new();

        let entry_count = entries.len();
        for (idx, entry) in entries.iter_mut().enumerate() {
            let mut resource = entry.resource.take();
            let used_references = resource
                .as_mut()
                .map(|resource| bundle_state.resolve(resource))
                .unwrap_or_default();
            let delete_target = (entry.method == BundleMethod::Delete)
                .then(|| {
                    targets
                        .get(&idx)
                        .and_then(|target| target.identity())
                        .or_else(|| {
                            self.parse_url(&entry.url)
                                .ok()
                                .map(|(kind, id)| format!("{kind}/{id}"))
                        })
                })
                .flatten();

            let result = self
                .process_bundle_entry_transaction(
                    db,
                    session,
                    BundleEntryContext {
                        tenant,
                        fhir_version,
                        patch_validator: validator,
                    },
                    entry,
                    resource,
                    &mut pending_search_parameter_changes,
                    targets.get(&idx),
                )
                .await;

            match result {
                Ok(entry_result) => {
                    if entry_result.status >= 400 {
                        if entry.method == BundleMethod::Patch {
                            patch_error = Some(TransactionError::PatchEntry {
                                index: idx,
                                status: entry_result.status,
                                outcome: entry_result.outcome.clone().unwrap_or_default(),
                            });
                        }
                        error_info = Some((
                            idx,
                            format!("Entry failed with status {}", entry_result.status),
                        ));
                        break;
                    }

                    if let Err(message) =
                        bundle_state.record(entry, &entry_result, delete_target, used_references)
                    {
                        error_info = Some((idx, message));
                        break;
                    }

                    results.push(entry_result);
                }
                Err(e) => {
                    let _ = session.abort_transaction().await;
                    return Err(entry_attempt_error(idx, entry_count, &e));
                }
            }
        }

        if let Some((index, message)) = error_info {
            let _ = session.abort_transaction().await;
            return Err(BundleAttemptError::Failed(
                patch_error.unwrap_or(TransactionError::BundleError { index, message }),
            ));
        }

        // A commit whose outcome is unknown may have applied. Sending the
        // commit again is safe (the driver's own documented recovery);
        // re-running the entries is not, since every POST would be stored a
        // second time under a new id. `CommitAttempts` keeps the verdicts
        // straight: once any commit has been unknown, no later failure may
        // read as "rolled back" or as grounds for a replay.
        let mut commit = CommitAttempts::new();
        while let Err(e) = session.commit_transaction().await {
            match commit.after_failure(&e) {
                CommitStep::RetryCommit => continue,
                CommitStep::Stop(error) => return Err(error),
            }
        }

        Ok((results, pending_search_parameter_changes))
    }

    async fn pin_bundle_references(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant: &TenantContext,
        entries: &mut [BundleEntry],
        targets: &HashMap<usize, crate::core::ConditionalTarget>,
    ) -> Result<HashMap<String, String>, (usize, StorageError)> {
        let mut references = HashMap::new();
        for (index, entry) in entries.iter_mut().enumerate() {
            let Some(full_url) = entry.full_url.clone() else {
                continue;
            };
            let (resource_type, criteria) = match entry.method {
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
                    (resource_type, entry.if_none_exist.clone())
                }
                BundleMethod::Put if targets.contains_key(&index) => {
                    let target = &targets[&index];
                    let matched = target.resolved.as_ref().map(|resource| resource.id());
                    references.insert(
                        full_url,
                        entry
                            .pin_reference(&target.resource_type, matched)
                            .map_err(|e| (index, e))?,
                    );
                    continue;
                }
                BundleMethod::Put => {
                    match parse_bundle_entry_target(&entry.url).map_err(|e| (index, e))? {
                        BundleEntryTarget::Instance { resource_type, id } => {
                            references.insert(
                                full_url,
                                entry
                                    .pin_reference(&resource_type, Some(&id))
                                    .map_err(|e| (index, e))?,
                            );
                            continue;
                        }
                        BundleEntryTarget::Conditional {
                            resource_type,
                            criteria,
                        } => (resource_type, Some(criteria)),
                    }
                }
                _ => continue,
            };
            let matched = match criteria {
                None => None,
                Some(criteria) => {
                    let matches = self
                        .find_matching_resources_in_bundle_transaction(
                            db,
                            session,
                            tenant,
                            &resource_type,
                            &criteria,
                        )
                        .await
                        .map_err(|e| (index, e))?;
                    match matches.as_slice() {
                        [] => None,
                        [only] => Some(only.id().to_string()),
                        _ => continue,
                    }
                }
            };
            references.insert(
                full_url,
                entry
                    .pin_reference(&resource_type, matched.as_deref())
                    .map_err(|e| (index, e))?,
            );
        }
        Ok(references)
    }

    /// Resolves every URL-borne conditional entry inside the session, before
    /// any write, and enforces the R4 §3.1.0.11.2 overlap rule (#859).
    ///
    /// The session-scoped matcher evaluates criteria in application memory
    /// and understands plain `name=value` only, so a criterion with a
    /// modifier, chain, prefix or OR-list refuses the entry with the `501`
    /// the offloaded-search case answers on the other backends; #709 owns
    /// an index-backed matcher.
    async fn resolve_conditional_targets_in_bundle_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant: &TenantContext,
        entries: &[BundleEntry],
    ) -> Result<HashMap<usize, crate::core::ConditionalTarget>, BundleAttemptError> {
        let mut targets = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let Some(criteria) = entry.criteria.as_deref() else {
                continue;
            };
            let resource_type = crate::core::conditional_resource_type(entry)
                .filter(|t| !t.is_empty())
                .ok_or_else(|| {
                    BundleAttemptError::Failed(TransactionError::BundleError {
                        index,
                        message: format!(
                            "Entry request.url '{}' names no resource type",
                            entry.url
                        ),
                    })
                })?
                .to_string();
            let matches = self
                .find_matching_typed_in_bundle_transaction(
                    db,
                    session,
                    tenant,
                    &resource_type,
                    criteria.to_vec(),
                )
                .await
                .map_err(|e| entry_attempt_error(index, entries.len(), &e))?;
            targets.push(
                crate::core::conditional_target(index, entry, &resource_type, matches)
                    .map_err(BundleAttemptError::Failed)?,
            );
        }
        crate::core::check_identity_overlap(entries, &targets)
            .map_err(BundleAttemptError::Failed)?;
        Ok(targets
            .into_iter()
            .map(|target| (target.entry_index, target))
            .collect())
    }

    /// Process a single bundle entry within the session's transaction.
    ///
    /// `target` is the pre-pass resolution of a URL-borne conditional entry
    /// (#859): its `PUT` updates the match or creates, its `DELETE` deletes
    /// the match or is a no-op `204`, without re-resolving the criteria.
    #[allow(clippy::too_many_arguments)]
    async fn process_bundle_entry_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        context: BundleEntryContext<'_>,
        entry: &BundleEntry,
        resource: Option<Value>,
        pending_search_parameter_changes: &mut Vec<PendingSearchParameterChange>,
        target: Option<&crate::core::ConditionalTarget>,
    ) -> StorageResult<BundleEntryResult> {
        let BundleEntryContext {
            tenant,
            fhir_version,
            patch_validator: validator,
        } = context;
        match entry.method {
            BundleMethod::Get => {
                let (resource_type, id) = self.parse_url(&entry.url)?;
                match self
                    .read_resource_in_bundle_transaction(db, session, tenant, &resource_type, &id)
                    .await?
                {
                    Some(resource) => Ok(BundleEntryResult::ok(resource)),
                    None => Ok(BundleEntryResult::error(
                        404,
                        serde_json::json!({
                            "resourceType": "OperationOutcome",
                            "issue": [{"severity": "error", "code": "not-found"}]
                        }),
                    )),
                }
            }
            BundleMethod::Post => {
                let resource = resource.ok_or_else(|| {
                    StorageError::Validation(crate::error::ValidationError::MissingRequiredField {
                        field: "resource".to_string(),
                    })
                })?;

                let resource_type = resource
                    .get("resourceType")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| {
                        StorageError::Validation(
                            crate::error::ValidationError::MissingRequiredField {
                                field: "resourceType".to_string(),
                            },
                        )
                    })?;

                if let Some(search_params) = entry.if_none_exist.as_ref() {
                    let matches = self
                        .find_matching_resources_in_bundle_transaction(
                            db,
                            session,
                            tenant,
                            &resource_type,
                            search_params,
                        )
                        .await?;

                    // Shared with the SQLite and PostgreSQL executors so all
                    // three answer 200-with-location / 412 identically, and so
                    // the matched id reaches the fullUrl reference map.
                    if let Some(gated) = bundle_if_none_exist_gate(matches) {
                        return Ok(gated);
                    }
                }

                let created = self
                    .create_resource_in_bundle_transaction(
                        db,
                        session,
                        tenant,
                        &resource_type,
                        resource,
                        fhir_version,
                        pending_search_parameter_changes,
                    )
                    .await?;
                Ok(BundleEntryResult::created(created))
            }
            BundleMethod::Put => {
                let resource = resource.ok_or_else(|| {
                    StorageError::Validation(crate::error::ValidationError::MissingRequiredField {
                        field: "resource".to_string(),
                    })
                })?;

                if let Some(target) = target {
                    return Ok(match &target.resolved {
                        Some(existing) => crate::core::conditional_update_entry(
                            self.update_resource_in_bundle_transaction(
                                db,
                                session,
                                tenant,
                                existing,
                                resource,
                                pending_search_parameter_changes,
                            )
                            .await?,
                        ),
                        None => BundleEntryResult::created(
                            self.create_resource_in_bundle_transaction(
                                db,
                                session,
                                tenant,
                                &target.resource_type,
                                resource,
                                fhir_version,
                                pending_search_parameter_changes,
                            )
                            .await?,
                        ),
                    });
                }

                let (resource_type, id) = self.parse_url(&entry.url)?;

                match self
                    .read_resource_in_bundle_transaction(db, session, tenant, &resource_type, &id)
                    .await?
                {
                    Some(existing) => {
                        // Shared, list-aware gate: `ifMatch` is satisfied when
                        // any listed tag matches. The hand-rolled single-value
                        // comparison here could never match a comma-separated
                        // header (issue #311).
                        if let Some(failure) = bundle_if_match_gate(
                            entry.if_match.as_deref(),
                            Some(existing.version_id()),
                        ) {
                            return Ok(failure);
                        }

                        let updated = self
                            .update_resource_in_bundle_transaction(
                                db,
                                session,
                                tenant,
                                &existing,
                                resource,
                                pending_search_parameter_changes,
                            )
                            .await?;
                        Ok(BundleEntryResult::updated(updated))
                    }
                    None => {
                        // A supplied `ifMatch` — including `*` — cannot be
                        // satisfied when there is no current representation, so
                        // it must fail rather than silently create.
                        if let Some(failure) = bundle_if_match_gate(entry.if_match.as_deref(), None)
                        {
                            return Ok(failure);
                        }

                        let mut resource_with_id = resource;
                        resource_with_id["id"] = serde_json::json!(id);

                        let created = self
                            .create_resource_in_bundle_transaction(
                                db,
                                session,
                                tenant,
                                &resource_type,
                                resource_with_id,
                                fhir_version,
                                pending_search_parameter_changes,
                            )
                            .await?;
                        Ok(BundleEntryResult::created(created))
                    }
                }
            }
            BundleMethod::Delete => {
                if let Some(target) = target {
                    return Ok(match &target.resolved {
                        Some(existing) => {
                            self.delete_resource_in_bundle_transaction(
                                db,
                                session,
                                tenant,
                                &target.resource_type,
                                existing.id(),
                                pending_search_parameter_changes,
                            )
                            .await?;
                            crate::core::conditional_delete_entry(existing)
                        }
                        None => BundleEntryResult::delete_not_found(),
                    });
                }

                let (resource_type, id) = self.parse_url(&entry.url)?;

                if let Some(if_match) = entry.if_match.as_ref() {
                    match self
                        .delete_with_match_resource_in_bundle_transaction(
                            db,
                            session,
                            tenant,
                            &resource_type,
                            &id,
                            if_match,
                            pending_search_parameter_changes,
                        )
                        .await
                    {
                        Ok(()) => Ok(BundleEntryResult::deleted()),
                        Err(StorageError::Resource(ResourceError::NotFound { .. })) => {
                            Ok(BundleEntryResult::error(
                                404,
                                serde_json::json!({
                                    "resourceType": "OperationOutcome",
                                    "issue": [{"severity": "error", "code": "not-found"}]
                                }),
                            ))
                        }
                        Err(e) => Err(e),
                    }
                } else {
                    match self
                        .delete_resource_in_bundle_transaction(
                            db,
                            session,
                            tenant,
                            &resource_type,
                            &id,
                            pending_search_parameter_changes,
                        )
                        .await
                    {
                        Ok(()) => Ok(BundleEntryResult::deleted()),
                        // Still 204 on the wire (delete is idempotent), but
                        // nothing live went away (#1078).
                        Err(StorageError::Resource(ResourceError::NotFound { .. })) => {
                            Ok(BundleEntryResult::delete_not_found())
                        }
                        Err(e) => Err(e),
                    }
                }
            }
            BundleMethod::Patch => {
                // A conditional patch (`PATCH [type]?[criteria]`) was resolved,
                // and its `ifMatch` gated, in the pre-pass (#1535); an instance
                // patch reads its row and gates here.
                let (resource_type, existing, missing) = match target {
                    Some(target) => (
                        target.resource_type.clone(),
                        target.resolved.clone(),
                        format!("no {} matches {}", target.resource_type, entry.url),
                    ),
                    None => {
                        let (resource_type, id) = self.parse_url(&entry.url)?;
                        let existing = self
                            .read_resource_in_bundle_transaction(
                                db,
                                session,
                                tenant,
                                &resource_type,
                                &id,
                            )
                            .await?;
                        if let Some(failure) = bundle_if_match_gate(
                            entry.if_match.as_deref(),
                            existing.as_ref().map(|r| r.version_id()),
                        ) {
                            return Ok(failure);
                        }
                        let missing = format!("{resource_type}/{id} not found");
                        (resource_type, existing, missing)
                    }
                };
                if resource_type == "AuditEvent" {
                    return Ok(BundleEntryResult::error(
                        405,
                        serde_json::json!({
                            "resourceType": "OperationOutcome",
                            "issue": [{"severity": "error", "code": "not-supported", "details": {"text": "AuditEvent resources are immutable"}}]
                        }),
                    ));
                }
                let Some(existing) = existing else {
                    return Ok(BundleEntryResult::error(
                        404,
                        serde_json::json!({
                            "resourceType": "OperationOutcome",
                            "issue": [{"severity": "error", "code": "not-found", "details": {"text": missing}}]
                        }),
                    ));
                };
                let candidate = match crate::core::transaction::prepare_bundle_patch(
                    tenant,
                    &resource_type,
                    &existing,
                    resource.as_ref(),
                    fhir_version,
                    validator,
                )
                .await
                {
                    Ok(candidate) => candidate,
                    Err(failure) => return Ok(*failure),
                };
                let update_result = self
                    .update_resource_in_bundle_transaction(
                        db,
                        session,
                        tenant,
                        &existing,
                        candidate,
                        pending_search_parameter_changes,
                    )
                    .await;
                crate::core::transaction::patch_update_result(update_result)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_resource_in_bundle_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: helios_fhir::FhirVersion,
        pending_search_parameter_changes: &mut Vec<PendingSearchParameterChange>,
    ) -> StorageResult<StoredResource> {
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let id = resource
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(crate::types::new_resource_id);

        let existing = resources
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": &id,
            })
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_driver_error("Failed to check resource existence in transaction", e)
            })?;

        if existing.is_some() {
            return Err(StorageError::Resource(ResourceError::AlreadyExists {
                resource_type: resource_type.to_string(),
                id,
            }));
        }

        let mut resource = resource;
        ensure_resource_identity(resource_type, &id, &mut resource);
        let payload = value_to_document(&resource)?;

        let now = Utc::now();
        let now_bson = chrono_to_bson(now);
        let version_id = "1".to_string();
        let fhir_version_str = fhir_version.as_mime_param().to_string();

        resources
            .insert_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": &id,
                "version_id": &version_id,
                "data": Bson::Document(payload.clone()),
                "created_at": now_bson,
                "last_updated": now_bson,
                "is_deleted": false,
                "deleted_at": Bson::Null,
                "fhir_version": &fhir_version_str,
            })
            .session(&mut *session)
            .await
            .map_err(|e| {
                if is_duplicate_key_error(&e) {
                    StorageError::Resource(ResourceError::AlreadyExists {
                        resource_type: resource_type.to_string(),
                        id: id.clone(),
                    })
                } else {
                    internal_driver_error("Failed to insert resource in transaction", e)
                }
            })?;

        history
            .insert_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": &id,
                "version_id": &version_id,
                "data": Bson::Document(payload),
                "created_at": now_bson,
                "last_updated": now_bson,
                "is_deleted": false,
                "deleted_at": Bson::Null,
                "fhir_version": &fhir_version_str,
            })
            .session(&mut *session)
            .await
            .map_err(|e| internal_driver_error("Failed to insert history in transaction", e))?;

        self.index_resource_in_bundle_transaction(
            db,
            session,
            tenant_id,
            resource_type,
            &id,
            &resource,
        )
        .await?;

        if resource_type == "SearchParameter" {
            pending_search_parameter_changes.push(PendingSearchParameterChange::Create);
        }

        Ok(StoredResource::from_storage(
            resource_type,
            &id,
            version_id,
            tenant.tenant_id().clone(),
            resource,
            now,
            now,
            None,
            fhir_version,
        ))
    }

    async fn update_resource_in_bundle_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
        pending_search_parameter_changes: &mut Vec<PendingSearchParameterChange>,
    ) -> StorageResult<StoredResource> {
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let resource_type = current.resource_type();
        let id = current.id();

        let existing_doc = resources
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
                "is_deleted": false,
            })
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_driver_error("Failed to load current resource in transaction", e)
            })?
            .ok_or_else(|| {
                StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                })
            })?;

        let actual_version = existing_doc
            .get_str("version_id")
            .map_err(|e| internal_error(format!("Missing current version in transaction: {}", e)))?
            .to_string();

        if actual_version != current.version_id() {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: current.version_id().to_string(),
                    actual_version,
                },
            ));
        }

        let new_version = next_version(current.version_id())?;
        let mut resource = resource;
        ensure_resource_identity(resource_type, id, &mut resource);
        let payload = value_to_document(&resource)?;

        let now = Utc::now();
        let now_bson = chrono_to_bson(now);
        let fhir_version = current.fhir_version();
        let fhir_version_str = fhir_version.as_mime_param().to_string();

        let update_result = resources
            .update_one(
                doc! {
                    "tenant_id": tenant_id,
                    "resource_type": resource_type,
                    "id": id,
                    "version_id": current.version_id(),
                    "is_deleted": false,
                },
                doc! {
                    "$set": {
                        "version_id": &new_version,
                        "data": Bson::Document(payload.clone()),
                        "last_updated": now_bson,
                        "is_deleted": false,
                        "deleted_at": Bson::Null,
                        "fhir_version": &fhir_version_str,
                    }
                },
            )
            .session(&mut *session)
            .await
            .map_err(|e| internal_driver_error("Failed to update resource in transaction", e))?;

        if update_result.matched_count == 0 {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: current.version_id().to_string(),
                    actual_version: "unknown".to_string(),
                },
            ));
        }

        let created_at = extract_created_at(&existing_doc, now);

        history
            .insert_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
                "version_id": &new_version,
                "data": Bson::Document(payload),
                "created_at": chrono_to_bson(created_at),
                "last_updated": now_bson,
                "is_deleted": false,
                "deleted_at": Bson::Null,
                "fhir_version": &fhir_version_str,
            })
            .session(&mut *session)
            .await
            .map_err(|e| internal_driver_error("Failed to insert history in transaction", e))?;

        self.index_resource_in_bundle_transaction(
            db,
            session,
            tenant_id,
            resource_type,
            id,
            &resource,
        )
        .await?;

        if resource_type == "SearchParameter" {
            pending_search_parameter_changes.push(PendingSearchParameterChange::Update);
        }

        Ok(StoredResource::from_storage(
            resource_type,
            id,
            new_version,
            tenant.tenant_id().clone(),
            resource,
            created_at,
            now,
            None,
            fhir_version,
        ))
    }

    async fn delete_resource_in_bundle_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        pending_search_parameter_changes: &mut Vec<PendingSearchParameterChange>,
    ) -> StorageResult<()> {
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let existing_doc = resources
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
                "is_deleted": false,
            })
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_driver_error("Failed to load resource for delete in transaction", e)
            })?
            .ok_or_else(|| {
                StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                })
            })?;

        let current_version = existing_doc
            .get_str("version_id")
            .map_err(|e| {
                internal_error(format!(
                    "Missing current version in transaction delete: {}",
                    e
                ))
            })?
            .to_string();
        let new_version = next_version(&current_version)?;

        let payload = existing_doc
            .get_document("data")
            .map_err(|e| {
                internal_error(format!(
                    "Missing resource payload in transaction delete: {}",
                    e
                ))
            })?
            .clone();
        let fhir_version = existing_doc
            .get_str("fhir_version")
            .unwrap_or("4.0")
            .to_string();
        let created_at = extract_created_at(&existing_doc, Utc::now());

        let now = Utc::now();
        let now_bson = chrono_to_bson(now);

        let update_result = resources
            .update_one(
                doc! {
                    "tenant_id": tenant_id,
                    "resource_type": resource_type,
                    "id": id,
                    "version_id": &current_version,
                    "is_deleted": false,
                },
                doc! {
                    "$set": {
                        "version_id": &new_version,
                        "is_deleted": true,
                        "deleted_at": now_bson,
                        "last_updated": now_bson,
                    }
                },
            )
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_driver_error("Failed to soft-delete resource in transaction", e)
            })?;

        if update_result.matched_count == 0 {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        }

        history
            .insert_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
                "version_id": &new_version,
                "data": Bson::Document(payload),
                "created_at": chrono_to_bson(created_at),
                "last_updated": now_bson,
                "is_deleted": true,
                "deleted_at": now_bson,
                "fhir_version": fhir_version,
            })
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_driver_error("Failed to insert delete history in transaction", e)
            })?;

        self.delete_search_index_in_bundle_transaction(db, session, tenant_id, resource_type, id)
            .await?;

        if resource_type == "SearchParameter" {
            pending_search_parameter_changes.push(PendingSearchParameterChange::Delete);
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn delete_with_match_resource_in_bundle_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
        pending_search_parameter_changes: &mut Vec<PendingSearchParameterChange>,
    ) -> StorageResult<()> {
        let existing = self
            .read_resource_in_bundle_transaction(db, session, tenant, resource_type, id)
            .await?
            .ok_or_else(|| {
                StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                })
            })?;

        // List-aware `If-Match` comparison, shared with every other backend.
        if !if_match_field_satisfied(expected_version, existing.version_id()) {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: normalize_etag(expected_version).to_string(),
                    actual_version: normalize_etag(existing.version_id()).to_string(),
                },
            ));
        }

        self.delete_resource_in_bundle_transaction(
            db,
            session,
            tenant,
            resource_type,
            id,
            pending_search_parameter_changes,
        )
        .await
    }

    async fn read_resource_in_bundle_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let maybe_doc = resources
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
                "is_deleted": false,
            })
            .session(&mut *session)
            .await
            .map_err(|e| internal_driver_error("Failed to read resource in transaction", e))?;

        maybe_doc
            .as_ref()
            .map(|doc| document_to_stored_resource(doc, tenant, resource_type))
            .transpose()
    }

    async fn find_matching_resources_in_bundle_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
    ) -> StorageResult<Vec<StoredResource>> {
        let parsed_params = crate::search::parse_conditional_criteria(search_params);
        if parsed_params.is_empty() {
            return Ok(Vec::new());
        }
        // Typed through the shared builder, which applies registry validation,
        // type-aware parsing, OR splitting and modifier rules (#1312, #1321,
        // #1323, #1360, #1366), so this path accepts and rejects the same
        // criteria as `If-None-Exist` on the resource endpoint.
        let typed_params = self.build_search_parameters(tenant, resource_type, &parsed_params)?;
        self.find_matching_typed_in_bundle_transaction(
            db,
            session,
            tenant,
            resource_type,
            typed_params,
        )
        .await
    }

    /// The session-scoped matcher both criteria forms end in: `ifNoneExist`'s
    /// string, typed above, and a URL-borne conditional entry's criteria,
    /// which the REST layer already typed (#859).
    async fn find_matching_typed_in_bundle_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant: &TenantContext,
        resource_type: &str,
        typed_params: Vec<SearchParameter>,
    ) -> StorageResult<Vec<StoredResource>> {
        // Criteria of only result parameters were refused upstream (#1542):
        // `ifNoneExist` by `build_search_parameters`, URL criteria by the REST
        // layer. An empty filter would match the whole type.
        if typed_params.is_empty() {
            return Ok(Vec::new());
        }

        if self.is_search_offloaded() {
            return self
                .if_none_exist_offloaded_scan(db, session, tenant, resource_type, &typed_params)
                .await;
        }

        self.preflight_legacy_composites(
            db,
            tenant.tenant_id().as_str(),
            resource_type,
            &typed_params,
            false,
            Some(&mut *session),
        )
        .await?;
        let index_params: Vec<_> = typed_params
            .iter()
            .filter(|p| !matches!(p.name.as_str(), "_id" | "_lastUpdated"))
            .collect();

        if index_params.is_empty() {
            let query = SearchQuery {
                resource_type: resource_type.to_string(),
                parameters: typed_params,
                count: Some(2),
                ..Default::default()
            };
            let tenant_id = tenant.tenant_id().as_str();
            let filter =
                self.build_resource_filter(tenant_id, resource_type, &query, None, None)?;
            let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
            let cursor = resources
                .find(filter)
                .limit(2)
                .session(&mut *session)
                .await
                .map_err(|e| {
                    internal_driver_error("Failed to query resources in transaction", e)
                })?;
            let docs = collect_session_documents(cursor, session).await?;
            return docs
                .into_iter()
                .map(|doc| document_to_stored_resource(&doc, tenant, resource_type))
                .collect();
        }

        let tenant_id = tenant.tenant_id().as_str();
        let search_index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);

        const PROBE_LIMIT: i64 = 2;
        const BATCH_SIZE: i64 = 128;

        // #1206: cache each composite's driver-arm probe (component filters +
        // counts already resolved) so the winning index, if composite,
        // doesn't re-probe below — mirrors `matching_resource_ids` in
        // `search_impl.rs`.
        let mut composite_probes: HashMap<usize, (Document, i64)> = HashMap::new();

        let driver_idx = {
            let mut best: Option<(usize, i64)> = None;
            for (i, param) in index_params.iter().enumerate() {
                let count = if param.param_type == SearchParamType::Composite {
                    match self
                        .composite_driver_probe(
                            &search_index,
                            tenant_id,
                            resource_type,
                            param,
                            PROBE_LIMIT as u64,
                            Some(&mut *session),
                        )
                        .await?
                    {
                        None => return Ok(Vec::new()),
                        Some((filter, count)) => {
                            let count = count as i64;
                            composite_probes.insert(i, (filter, count));
                            count
                        }
                    }
                } else {
                    let filter = self.build_search_index_filter(tenant_id, resource_type, param)?;
                    let pipeline = vec![
                        doc! { "$match": filter },
                        doc! { "$limit": PROBE_LIMIT },
                        doc! { "$group": { "_id": "$resource_id" } },
                        doc! { "$count": "n" },
                    ];
                    let cursor = search_index
                        .aggregate(pipeline)
                        .session(&mut *session)
                        .await
                        .map_err(|e| {
                            internal_driver_error("Failed probe for ifNoneExist driver", e)
                        })?;
                    let probe_docs = collect_session_documents(cursor, session).await?;
                    let count = probe_docs
                        .first()
                        .and_then(|d| d.get_i32("n").ok())
                        .map(|n| n as i64)
                        .unwrap_or(0);
                    if count == 0 {
                        return Ok(Vec::new());
                    }
                    count
                };
                if best.is_none_or(|(_, prev)| count < prev) {
                    best = Some((i, count));
                }
            }
            best.map(|(i, _)| i).unwrap_or(0)
        };

        // Every composite index visited above has its probe result cached,
        // so `driver_idx` pointing at a composite always finds an entry
        // here; a plain param never has one and falls through as before.
        let driver_filter = if let Some((filter, _)) = composite_probes.remove(&driver_idx) {
            filter
        } else {
            self.build_search_index_filter(tenant_id, resource_type, index_params[driver_idx])?
        };

        let mut last_index_id: Option<Bson> = None;
        let mut matches: Vec<StoredResource> = Vec::with_capacity(2);
        let mut matched_ids: HashSet<String> = HashSet::new();
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);

        loop {
            let page_filter = match &last_index_id {
                Some(last_id) => doc! {
                    "$and": [driver_filter.clone(), { "_id": { "$gt": last_id.clone() } }]
                },
                None => driver_filter.clone(),
            };

            let mut cursor = search_index
                .find(page_filter)
                .sort(doc! { "_id": 1 })
                .projection(doc! { "_id": 1, "resource_id": 1 })
                .limit(BATCH_SIZE)
                .session(&mut *session)
                .await
                .map_err(|e| {
                    internal_driver_error("Failed to page search_index for ifNoneExist", e)
                })?;

            let mut candidate_ids: HashSet<String> = HashSet::new();
            let mut docs_read: i64 = 0;

            while cursor
                .advance(&mut *session)
                .await
                .map_err(|e| internal_driver_error("Failed to advance search_index cursor", e))?
            {
                let doc = cursor.deserialize_current().map_err(|e| {
                    internal_error(format!("Failed to deserialize search_index doc: {}", e))
                })?;
                last_index_id = doc.get("_id").cloned();
                docs_read += 1;
                if let Ok(rid) = doc.get_str("resource_id") {
                    candidate_ids.insert(rid.to_string());
                }
            }

            if docs_read == 0 {
                break;
            }

            for (i, param) in index_params.iter().enumerate() {
                if candidate_ids.is_empty() {
                    continue;
                }
                // Same reasoning as `matching_resource_ids`: a composite's
                // driver arm only proves its most selective component
                // matched, so every composite here — including the driver —
                // still needs the grouped pair check (#1206).
                if param.param_type == SearchParamType::Composite {
                    let passing = self
                        .composite_pair_check(
                            &search_index,
                            tenant_id,
                            resource_type,
                            param,
                            &candidate_ids,
                            None,
                            Some(&mut *session),
                        )
                        .await?;
                    candidate_ids.retain(|id| passing.contains(id));
                    continue;
                }
                if i == driver_idx {
                    continue;
                }
                let param_filter =
                    self.build_search_index_filter(tenant_id, resource_type, param)?;
                let bounded_filter = doc! {
                    "$and": [
                        param_filter,
                        { "resource_id": { "$in": candidate_ids.iter().cloned().collect::<Vec<_>>() } }
                    ]
                };
                let passing: HashSet<String> = search_index
                    .distinct("resource_id", bounded_filter)
                    .session(&mut *session)
                    .await
                    .map_err(|e| {
                        internal_driver_error("Failed distinct for ifNoneExist intersection", e)
                    })?
                    .into_iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect();
                candidate_ids.retain(|id| passing.contains(id));
            }

            if !candidate_ids.is_empty() {
                let remaining = (2 - matches.len()) as i64;
                // #1602: `_id` / `_lastUpdated` live on the `resources`
                // document, not in `search_index` (they were filtered out of
                // `index_params` above), so the index intersection never
                // applied them. Apply them here through the same
                // `build_resource_filter` the `index_params.is_empty()` branch
                // uses. Filtering at the fetch keeps the two-match limit
                // exact: the paging loop continues until two candidates
                // survive or the index runs out.
                let resource_query = SearchQuery {
                    resource_type: resource_type.to_string(),
                    parameters: typed_params
                        .iter()
                        .filter(|p| matches!(p.name.as_str(), "_id" | "_lastUpdated"))
                        .cloned()
                        .collect(),
                    count: Some(2),
                    ..Default::default()
                };
                let resource_filter = self.build_resource_filter(
                    tenant_id,
                    resource_type,
                    &resource_query,
                    None,
                    None,
                )?;
                let fetch_filter = doc! {
                    "$and": [
                        resource_filter,
                        { "id": { "$in": candidate_ids.into_iter().collect::<Vec<_>>() } }
                    ]
                };
                let mut res_cursor = resources
                    .find(fetch_filter)
                    .limit(remaining)
                    .session(&mut *session)
                    .await
                    .map_err(|e| {
                        internal_driver_error("Failed to fetch resources for ifNoneExist", e)
                    })?;

                while res_cursor
                    .advance(&mut *session)
                    .await
                    .map_err(|e| internal_driver_error("Failed to advance resources cursor", e))?
                {
                    let doc = res_cursor.deserialize_current().map_err(|e| {
                        internal_error(format!("Failed to deserialize resource doc: {}", e))
                    })?;
                    let resource = document_to_stored_resource(&doc, tenant, resource_type)?;
                    if matched_ids.insert(resource.id().to_string()) {
                        matches.push(resource);
                    }
                }
            }

            if matches.len() >= 2 || docs_read < BATCH_SIZE {
                break;
            }
        }

        Ok(matches)
    }

    /// Matches `ifNoneExist` criteria against the raw `resources` documents.
    ///
    /// Search is offloaded, so there are no `search_index` rows to consult;
    /// the match runs inside the transaction session (read-your-writes).
    /// Criteria arrive typed by the shared conditional builder
    /// ([`MongoBackend::build_search_parameters`]), which already applied
    /// registry, empty-value, modifier and `:[type]` validation. This scan
    /// evaluates only the shapes provable against raw documents and fails
    /// closed on everything else — never silently ignoring a criterion
    /// (which widens the match) nor silently failing to match (which creates
    /// duplicates).
    ///
    /// Supported: `_id` / `_lastUpdated` (via [`MongoBackend::build_resource_filter`],
    /// the same predicates direct search uses) and plain `identifier` values
    /// in `code`, `|code`, or `system|code` form with a nonempty code (`|code`
    /// matches any system, same as Mongo direct search). Comma-separated
    /// values OR within one parameter; repeated parameters AND.
    async fn if_none_exist_offloaded_scan(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant: &TenantContext,
        resource_type: &str,
        params: &[SearchParameter],
    ) -> StorageResult<Vec<StoredResource>> {
        let tenant_id = tenant.tenant_id().as_str();

        // Anything outside the evaluatable set is rejected, not ignored:
        // silently dropping a criterion widens the match.
        for param in params {
            match param.name.as_str() {
                "_id" | "_lastUpdated" | "identifier" => {}
                other => {
                    return Err(StorageError::Search(
                        crate::error::SearchError::QueryParseError {
                            message: format!(
                                "ifNoneExist parameter '{other}' cannot be evaluated \
                                 against the resource collection when search is offloaded; \
                                 use a supported parameter (_id, _lastUpdated, identifier) \
                                 or disable search offloading"
                            ),
                        },
                    ));
                }
            }
        }

        let mut conditions: Vec<Document> = Vec::new();

        // `_id` / `_lastUpdated` reuse the resource-level predicates direct
        // search builds (prefix-aware, dates validated); they carry the
        // tenant / type / live-only base with them.
        let resource_params: Vec<SearchParameter> = params
            .iter()
            .filter(|p| matches!(p.name.as_str(), "_id" | "_lastUpdated"))
            .cloned()
            .collect();
        if resource_params.is_empty() {
            conditions.push(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "is_deleted": false,
            });
        } else {
            let query = SearchQuery {
                resource_type: resource_type.to_string(),
                parameters: resource_params,
                count: Some(2),
                ..Default::default()
            };
            conditions.push(self.build_resource_filter(
                tenant_id,
                resource_type,
                &query,
                None,
                None,
            )?);
        }

        // Plain `identifier` values against the raw `data.identifier` array.
        // One parameter's comma-separated values OR; repeated parameters AND
        // through the top-level `$and`.
        for param in params.iter().filter(|p| p.name.as_str() == "identifier") {
            Self::validate_offloaded_identifier_param(param)?;
            let mut branches: Vec<Bson> = Vec::with_capacity(param.values.len());
            for value in &param.values {
                if value.value.chars().filter(|c| *c == '|').count() > 1 {
                    return Err(StorageError::Search(
                        crate::error::SearchError::QueryParseError {
                            message: format!(
                                "Unsupported value '{}' for ifNoneExist parameter \
                                 'identifier' when search is offloaded: supported forms \
                                 are 'code', '|code' and 'system|code' with a single '|'",
                                value.value
                            ),
                        },
                    ));
                }
                let (system, code) = match value.value.split_once('|') {
                    Some((system, code)) => (system, code),
                    None => ("", value.value.as_str()),
                };
                if code.is_empty() {
                    return Err(StorageError::Search(
                        crate::error::SearchError::QueryParseError {
                            message: "Unsupported empty code for ifNoneExist parameter \
                                      'identifier' when search is offloaded: supported \
                                      forms are 'code', '|code' and 'system|code' with a \
                                      nonempty code"
                                .to_string(),
                        },
                    ));
                }
                let mut elem_match = Document::new();
                if !system.is_empty() {
                    elem_match.insert("system", system);
                }
                elem_match.insert("value", code);
                branches.push(Bson::Document(
                    doc! { "data.identifier": { "$elemMatch": elem_match } },
                ));
            }
            if branches.is_empty() {
                return Err(StorageError::Search(
                    crate::error::SearchError::QueryParseError {
                        message: "ifNoneExist parameter 'identifier' carries no value; \
                                  nothing was written"
                            .to_string(),
                    },
                ));
            } else if branches.len() == 1 {
                match branches.remove(0) {
                    Bson::Document(condition) => conditions.push(condition),
                    _ => unreachable!("identifier branches are documents"),
                }
            } else {
                conditions.push(doc! { "$or": Bson::Array(branches) });
            }
        }

        let filter = if conditions.len() == 1 {
            conditions.remove(0)
        } else {
            doc! { "$and": Bson::Array(conditions.into_iter().map(Bson::Document).collect()) }
        };

        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let mut matches: Vec<StoredResource> = Vec::with_capacity(2);
        let mut cursor = resources
            .find(filter)
            .limit(2)
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_driver_error("Failed to scan resources for offloaded ifNoneExist", e)
            })?;

        while cursor.advance(&mut *session).await.map_err(|e| {
            internal_driver_error(
                "Failed to advance resources cursor for offloaded ifNoneExist",
                e,
            )
        })? {
            let doc = cursor.deserialize_current().map_err(|e| {
                internal_error(format!(
                    "Failed to deserialize resource for offloaded ifNoneExist: {}",
                    e
                ))
            })?;
            matches.push(document_to_stored_resource(&doc, tenant, resource_type)?);
        }

        Ok(matches)
    }

    /// Keep the raw-document scan fail-closed if the conditional builder's
    /// typed-parameter contract changes. This check needs no database session.
    fn validate_offloaded_identifier_param(param: &SearchParameter) -> StorageResult<()> {
        if param.param_type != SearchParamType::Token {
            return Err(StorageError::Search(
                crate::error::SearchError::QueryParseError {
                    message: format!(
                        "ifNoneExist parameter 'identifier' cannot be evaluated \
                         against the resource collection when search is offloaded: \
                         unsupported parameter type '{}'",
                        param.param_type
                    ),
                },
            ));
        }
        if let Some(modifier) = param.modifier.as_ref() {
            return Err(StorageError::Search(
                crate::error::SearchError::UnsupportedModifier {
                    modifier: modifier.to_string(),
                    param_type: param.param_type.to_string(),
                },
            ));
        }
        for value in &param.values {
            if value.prefix != SearchPrefix::Eq {
                return Err(StorageError::Search(
                    crate::error::SearchError::QueryParseError {
                        message: format!(
                            "Unsupported prefix '{}' for ifNoneExist parameter \
                             'identifier' when search is offloaded",
                            value.prefix
                        ),
                    },
                ));
            }
        }
        Ok(())
    }

    async fn index_resource_in_bundle_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> StorageResult<()> {
        if self.is_search_offloaded() {
            return Ok(());
        }

        self.delete_search_index_in_bundle_transaction(
            db,
            session,
            tenant_id,
            resource_type,
            resource_id,
        )
        .await?;

        let (docs, _failure) =
            self.search_index_documents_checked(tenant_id, resource_type, resource_id, resource);

        self.insert_search_index_documents(db, docs, Some(session))
            .await
    }

    async fn delete_search_index_in_bundle_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<()> {
        if self.is_search_offloaded() {
            return Ok(());
        }

        self.delete_search_index_rows_for(
            db,
            tenant_id,
            resource_type,
            Bson::String(resource_id.to_string()),
            Some(session),
        )
        .await
    }

    fn parse_url(&self, url: &str) -> StorageResult<(String, String)> {
        match parse_bundle_entry_target(url)? {
            BundleEntryTarget::Instance { resource_type, id } => Ok((resource_type, id)),
            BundleEntryTarget::Conditional { .. } => Err(StorageError::Validation(
                crate::error::ValidationError::InvalidReference {
                    reference: url.to_string(),
                    message: "Expected an instance URL".to_string(),
                },
            )),
        }
    }
}

// ============================================================================
// PurgableStorage
//
// MongoDB stores resources across four collections — `resources`,
// `resource_history`, `search_index`, and `search_index_contained` — the same
// shape SQLite uses (plus the #1160 contained-rows split), so purge is the
// same four deletes keyed by (tenant_id, resource_type, id). Note that the
// ordinary `delete` is a *soft* delete: it flips `is_deleted` and writes a
// tombstone. Purge is the only path that removes the bytes.
// ============================================================================

#[async_trait]
impl PurgableStorage for MongoBackend {
    async fn purge(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        let db = self.get_database().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let search_index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);

        let key = doc! { "tenant_id": tenant_id, "resource_type": resource_type, "id": id };

        // A resource that exists only in history (already purged from the
        // current collection) is still purgeable; only a resource with no trace
        // at all is NotFound. Mirrors the SQLite backend.
        let in_resources = resources
            .count_documents(key.clone())
            .await
            .map_err(|e| internal_error(format!("Failed to check resource: {e}")))?;
        let in_history = history
            .count_documents(key.clone())
            .await
            .or_query_error("Failed to check resource history")?;
        if in_resources == 0 && in_history == 0 {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        }

        resources
            .delete_many(key.clone())
            .await
            .or_query_error("Failed to purge resource")?;
        history
            .delete_many(key)
            .await
            .or_query_error("Failed to purge resource history")?;

        // The search_index collection keys the resource as `resource_id`.
        search_index
            .delete_many(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "resource_id": id,
            })
            .await
            .or_query_error("Failed to purge search index")?;

        // Contained rows share the container's (tenant_id, resource_type,
        // resource_id), so they key the same way.
        let search_index_contained =
            db.collection::<Document>(MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION);
        search_index_contained
            .delete_many(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "resource_id": id,
            })
            .await
            .or_query_error("Failed to purge contained search index")?;

        Ok(())
    }

    async fn purge_all(&self, tenant: &TenantContext, resource_type: &str) -> StorageResult<u64> {
        let db = self.get_database().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let search_index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
        let search_index_contained =
            db.collection::<Document>(MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION);

        let key = doc! { "tenant_id": tenant_id, "resource_type": resource_type };

        // Counted before the delete, and counted over `resources` rather than
        // `resource_history`, so the returned figure is "resources purged", not
        // "versions purged".
        let count = resources
            .count_documents(key.clone())
            .await
            .or_query_error("Failed to count resources")?;

        resources
            .delete_many(key.clone())
            .await
            .or_query_error("Failed to purge resources")?;
        history
            .delete_many(key)
            .await
            .or_query_error("Failed to purge resource history")?;
        search_index
            .delete_many(doc! { "tenant_id": tenant_id, "resource_type": resource_type })
            .await
            .or_query_error("Failed to purge search index")?;
        // Contained rows share the container's (tenant_id, resource_type),
        // so a type-level purge keys the same way (#1160 Task 4).
        search_index_contained
            .delete_many(doc! { "tenant_id": tenant_id, "resource_type": resource_type })
            .await
            .or_query_error("Failed to purge contained search index")?;

        Ok(count)
    }
}

// ============================================================================
// ReindexSource / ReindexTarget
//
// MongoDB is a full primary with its own `search_index` collection, so it is
// both — it can reindex itself standalone.
// ============================================================================

/// Whether a reindex page that already holds `taken` rows totalling `bytes_taken`
/// admits a next row of `row_bytes` under `max_bytes` (`0` = no cap, #1499). The
/// first row is always admitted, so a page always advances; after it the page
/// never grows past the cap (PostgreSQL's rule, `PostgresBackend::fetch_resources_page_capped`).
fn reindex_page_admits(taken: usize, bytes_taken: u64, row_bytes: u64, max_bytes: u64) -> bool {
    max_bytes == 0 || taken == 0 || bytes_taken.saturating_add(row_bytes) <= max_bytes
}

/// What [`MongoBackend::reindex_find_page`] read (#1499): the rows it took, in scan
/// order, their raw BSON bytes, and whether the byte cap stopped it before `limit`.
struct ReindexFoundPage {
    docs: Vec<Document>,
    bytes: u64,
    capped: bool,
}

/// Logs a byte-capped reindex page read (#1499), in [`MongoBackend::fetch_reindex_page`]'s
/// id-phase and catch-up-round arms alike, so the two call sites share one log
/// line, one target and one field order instead of pasting the block twice.
fn log_capped_page_read(tenant_id: &str, resource_type: &str, found: &ReindexFoundPage) {
    tracing::debug!(
        tenant = %tenant_id,
        resource_type = %resource_type,
        rows = found.docs.len(),
        bytes = found.bytes,
        capped = found.capped,
        "mongodb reindex capped page read"
    );
}

impl MongoBackend {
    /// The newest-live probe (#1403): a covered reverse scan of
    /// `idx_resources_type_scan` for the `last_updated` of the newest live
    /// resource of `resource_type`, or `None` if it has no live resource.
    async fn reindex_newest_live_last_updated(
        &self,
        resources: &Collection<Document>,
        tenant_id: &str,
        resource_type: &str,
    ) -> StorageResult<Option<DateTime<Utc>>> {
        let found = resources
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "is_deleted": false,
            })
            .sort(doc! { "last_updated": -1, "id": -1 })
            .projection(doc! { "_id": 0, "last_updated": 1 })
            .hint(Hint::Name(RESOURCES_TYPE_SCAN_INDEX.to_string()))
            .await
            .map_err(|e| internal_error(format!("Failed to probe newest resource: {e}")))?;
        match found {
            Some(doc) => {
                let ts = doc
                    .get_datetime("last_updated")
                    .map_err(|e| internal_error(format!("Missing last_updated: {e}")))?;
                Ok(Some(bson_to_chrono(ts)))
            }
            None => Ok(None),
        }
    }

    /// One hinted, sorted, limited find (#1403). The page is drained up to
    /// `limit` rows, or until `max_bytes` (`0` = no cap, #1499) rejects the
    /// next row — the first row is always admitted.
    async fn reindex_find_page(
        &self,
        resources: &Collection<Document>,
        filter: Document,
        sort: Document,
        hint: &str,
        limit: u32,
        max_bytes: u64,
    ) -> StorageResult<ReindexFoundPage> {
        let mut stream = resources
            .find(filter)
            .sort(sort)
            .limit(limit as i64)
            .hint(Hint::Name(hint.to_string()))
            .await
            .map_err(|e| internal_error(format!("Failed to fetch resources: {e}")))?;
        let mut docs: Vec<Document> = Vec::new();
        let mut bytes: u64 = 0;
        let mut capped = false;
        while stream
            .advance()
            .await
            .map_err(|e| internal_error(format!("Failed to advance cursor: {e}")))?
        {
            let row_bytes = stream.current().as_bytes().len() as u64;
            if !reindex_page_admits(docs.len(), bytes, row_bytes, max_bytes) {
                capped = true;
                break;
            }
            bytes = bytes.saturating_add(row_bytes);
            docs.push(
                stream
                    .deserialize_current()
                    .map_err(|e| internal_error(format!("Failed to read resource: {e}")))?,
            );
        }
        drop(stream); // a capped read leaves server-side results; dropping kills the cursor
        Ok(ReindexFoundPage {
            docs,
            bytes,
            capped,
        })
    }

    /// Runs only the id-phase continuation query, for both the serial walk
    /// and the driver's ahead-of-time prefetch — so both paths build the same
    /// page from the same query (#1403). `Ok(None)` means the id phase is
    /// over; it logs nothing at all in that case (the empty check runs
    /// before the capped-page debug line, so that line is never emitted for
    /// an empty read, by either caller), per
    /// [`ReindexSource::fetch_resources_page_ahead`]'s doc contract that a
    /// source must not log or change state when it returns `Ok(None)`. The
    /// phase transition itself is left to whichever caller runs the query
    /// when it is *not* prefetched.
    #[allow(clippy::too_many_arguments)]
    async fn reindex_id_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        floor: DateTime<Utc>,
        after_id: Option<&str>,
        limit: u32,
        max_bytes: u64,
        range: Option<&ReindexIdRange>,
    ) -> StorageResult<Option<ResourcePage>> {
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(Self::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let found = self
            .reindex_find_page(
                &resources,
                reindex_id_page_filter(tenant_id, resource_type, floor, after_id, range),
                doc! { "id": 1 },
                RESOURCES_IDENTITY_INDEX,
                limit,
                max_bytes,
            )
            .await?;
        if found.docs.is_empty() {
            return Ok(None);
        }
        if max_bytes > 0 {
            log_capped_page_read(tenant_id, resource_type, &found);
        }
        let last_id = found
            .docs
            .last()
            .and_then(|d| d.get_str("id").ok())
            .map(str::to_string)
            .ok_or_else(|| {
                internal_error("Missing id on the last row of an id-phase page".to_string())
            })?;
        reindex_page_from_docs(
            &found.docs,
            resource_type,
            tenant,
            ReindexWalkCursor::Id {
                floor,
                after_id: last_id,
            },
        )
        .map(Some)
    }

    /// Pages `resource_type` in id order with catch-up rounds (#1403), bounded
    /// by `max_bytes` as well as by `limit` (`max_bytes == 0` is the id-order
    /// walk's uncapped page, #1499). A page the byte cap stops before `limit` is still non-empty
    /// (the first row is always admitted), so it continues its current walk phase
    /// exactly as a full page would — it never ends a phase and never returns
    /// `None` on its own account. `fetch_resources_page` and
    /// `fetch_resources_page_capped` are both thin calls to this method.
    /// `range` bounds the ids of every id-phase page and catch-up round
    /// (#1739).
    async fn fetch_reindex_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
        max_bytes: u64,
        range: Option<&ReindexIdRange>,
    ) -> StorageResult<ResourcePage> {
        let db = self.get_database().await?;
        let resources: Collection<Document> = db.collection(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let limit = limit.max(1); // MongoDB treats limit(0) as "no limit"
        let margin = reindex_catch_up_margin(self.config().reindex_catch_up_margin_ms);

        let mut step = match cursor {
            None => WalkStep::Start,
            Some(c) => WalkStep::from(ReindexWalkCursor::parse(c)?),
        };

        loop {
            step = match step {
                WalkStep::Start => {
                    let t0 = Utc::now();
                    let newest_live = self
                        .reindex_newest_live_last_updated(&resources, tenant_id, resource_type)
                        .await?;
                    let floor = reindex_catch_up_floor(t0, newest_live, margin);
                    tracing::info!(
                        tenant = %tenant_id,
                        resource_type = %resource_type,
                        t0 = %format_walk_instant(t0),
                        newest_live = %newest_live.map(format_walk_instant).unwrap_or_else(|| "none".to_string()),
                        floor = %format_walk_instant(floor),
                        "mongodb reindex walk started"
                    );
                    WalkStep::IdPhase {
                        floor,
                        after_id: None,
                    }
                }
                WalkStep::IdPhase { floor, after_id } => {
                    if let Some(page) = self
                        .reindex_id_page(
                            tenant,
                            resource_type,
                            floor,
                            after_id.as_deref(),
                            limit,
                            max_bytes,
                            range,
                        )
                        .await?
                    {
                        return Ok(page);
                    }
                    tracing::info!(
                        tenant = %tenant_id,
                        resource_type = %resource_type,
                        floor = %format_walk_instant(floor),
                        "mongodb reindex id phase finished"
                    );
                    WalkStep::RoundStart { round: 1, floor }
                }
                WalkStep::RoundStart { round, floor } => {
                    let now = Utc::now();
                    match reindex_round_start_decision(round, floor, now, margin) {
                        RoundStartDecision::Complete => {
                            tracing::debug!(
                                tenant = %tenant_id,
                                resource_type = %resource_type,
                                rounds = round - 1,
                                "mongodb reindex catch-up complete"
                            );
                            return Ok(ResourcePage {
                                resources: Vec::new(),
                                next_cursor: None,
                                skipped: Vec::new(),
                            });
                        }
                        RoundStartDecision::CapReached => {
                            tracing::warn!(
                                tenant = %tenant_id,
                                resource_type = %resource_type,
                                rounds = round - 1,
                                last_ceiling = %format_walk_instant(floor),
                                "mongodb reindex catch-up stopped at its round limit"
                            );
                            return Ok(ResourcePage {
                                resources: Vec::new(),
                                next_cursor: None,
                                skipped: Vec::new(),
                            });
                        }
                        RoundStartDecision::Run => {
                            let newest_live = self
                                .reindex_newest_live_last_updated(
                                    &resources,
                                    tenant_id,
                                    resource_type,
                                )
                                .await?;
                            let ceiling = reindex_catch_up_ceiling(now, newest_live, margin);
                            if let Some(newest_live) = newest_live {
                                let by_margin_only = truncate_to_millis(now + margin);
                                if ceiling > by_margin_only {
                                    tracing::warn!(
                                        tenant = %tenant_id,
                                        resource_type = %resource_type,
                                        round,
                                        newest_live = %format_walk_instant(newest_live),
                                        ceiling = %format_walk_instant(ceiling),
                                        "mongodb reindex found live resources stamped in the future"
                                    );
                                }
                            }
                            tracing::info!(
                                tenant = %tenant_id,
                                resource_type = %resource_type,
                                round,
                                floor = %format_walk_instant(floor),
                                ceiling = %format_walk_instant(ceiling),
                                "mongodb reindex catch-up round started"
                            );
                            WalkStep::Round {
                                round,
                                floor,
                                ceiling,
                                walked: 0,
                                after: None,
                            }
                        }
                    }
                }
                WalkStep::Round {
                    round,
                    floor,
                    ceiling,
                    walked,
                    after,
                } => {
                    let filter = reindex_catch_up_page_filter(
                        tenant_id,
                        resource_type,
                        floor,
                        ceiling,
                        after.as_ref().map(|(lu, id)| (*lu, id.as_str())),
                        range,
                    );
                    let found = self
                        .reindex_find_page(
                            &resources,
                            filter,
                            doc! { "last_updated": 1, "id": 1 },
                            RESOURCES_TYPE_SCAN_INDEX,
                            limit,
                            max_bytes,
                        )
                        .await?;
                    if max_bytes > 0 {
                        log_capped_page_read(tenant_id, resource_type, &found);
                    }
                    let scanned = found.docs;
                    if scanned.is_empty() {
                        tracing::info!(
                            tenant = %tenant_id,
                            resource_type = %resource_type,
                            round,
                            floor = %format_walk_instant(floor),
                            ceiling = %format_walk_instant(ceiling),
                            walked,
                            "mongodb reindex catch-up round finished"
                        );
                        WalkStep::RoundStart {
                            round: round + 1,
                            floor: ceiling,
                        }
                    } else {
                        let last = scanned.last().expect("non-empty");
                        let scanned_lu = last
                            .get_datetime("last_updated")
                            .map_err(|e| internal_error(format!("Missing last_updated: {e}")))?;
                        let scanned_id = last
                            .get_str("id")
                            .map_err(|e| internal_error(format!("Missing id: {e}")))?
                            .to_string();
                        let scanned_last_updated = bson_to_chrono(scanned_lu);
                        let docs = dedupe_reindex_page_keep_last(scanned);
                        let walked = walked + docs.len() as u64;
                        return reindex_page_from_docs(
                            &docs,
                            resource_type,
                            tenant,
                            ReindexWalkCursor::Round {
                                round,
                                floor,
                                ceiling,
                                walked,
                                after_last_updated: scanned_last_updated,
                                after_id: scanned_id,
                            },
                        );
                    }
                }
            };
        }
    }

    /// [`ReindexSource::fetch_resources_page_ahead`] for both the whole type
    /// and an id-range view of it (#1739). Only an id continuation runs
    /// ahead. Everything else, including the end of the id phase, is fetched
    /// serially after the page in flight is written.
    async fn fetch_reindex_page_ahead(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: &str,
        limit: u32,
        max_bytes: u64,
        range: Option<&ReindexIdRange>,
    ) -> StorageResult<Option<ResourcePage>> {
        let Ok(ReindexWalkCursor::Id { floor, after_id }) = ReindexWalkCursor::parse(cursor) else {
            return Ok(None);
        };
        self.reindex_id_page(
            tenant,
            resource_type,
            floor,
            Some(&after_id),
            limit.max(1),
            max_bytes,
            range,
        )
        .await
    }

    /// Live resources of `resource_type` with an id in `range` (#1739).
    async fn count_reindex_range(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        range: &ReindexIdRange,
    ) -> StorageResult<u64> {
        let db = self.get_database().await?;
        let mut filter = doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": resource_type,
            "is_deleted": false,
        };
        if let Some(bounds) = reindex_id_bounds(None, Some(range)) {
            filter.insert("id", bounds);
        }
        db.collection::<Document>(Self::RESOURCES_COLLECTION)
            .count_documents(filter)
            .await
            .or_query_error("Failed to count resources")
    }
}

/// An id-range view of a [`MongoBackend`] (#1739): every count, page,
/// prefetched page and catch-up round reads only ids in `range`. The walk and
/// its cursors are the backend's own.
struct RangedMongoReindexSource {
    backend: std::sync::Arc<MongoBackend>,
    range: ReindexIdRange,
}

#[async_trait]
impl ReindexSource for RangedMongoReindexSource {
    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>> {
        self.backend.list_resource_types(tenant).await
    }

    async fn count_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64> {
        self.backend
            .count_reindex_range(tenant, resource_type, &self.range)
            .await
    }

    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> StorageResult<ResourcePage> {
        self.fetch_resources_page_capped(tenant, resource_type, cursor, limit, 0)
            .await
    }

    async fn fetch_resources_page_capped(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
        max_bytes: u64,
    ) -> StorageResult<ResourcePage> {
        self.backend
            .fetch_reindex_page(
                tenant,
                resource_type,
                cursor,
                limit,
                max_bytes,
                Some(&self.range),
            )
            .await
    }

    fn may_prefetch_page(&self, cursor: &str) -> bool {
        self.backend.may_prefetch_page(cursor)
    }

    async fn fetch_resources_page_ahead(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: &str,
        limit: u32,
        max_bytes: u64,
    ) -> StorageResult<Option<ResourcePage>> {
        self.backend
            .fetch_reindex_page_ahead(
                tenant,
                resource_type,
                cursor,
                limit,
                max_bytes,
                Some(&self.range),
            )
            .await
    }

    async fn fetch_resources_by_ids(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        ids: &[String],
    ) -> StorageResult<Vec<StoredResource>> {
        let in_range: Vec<String> = ids
            .iter()
            .filter(|id| self.range.contains(id))
            .cloned()
            .collect();
        self.backend
            .fetch_resources_by_ids(tenant, resource_type, &in_range)
            .await
    }
}

/// One step of the walk inside a single call (#1403); never leaves the
/// call — only `ReindexWalkCursor::Id`/`Round` do, as an encoded cursor.
enum WalkStep {
    Start,
    IdPhase {
        floor: DateTime<Utc>,
        after_id: Option<String>,
    },
    RoundStart {
        round: u8,
        floor: DateTime<Utc>,
    },
    Round {
        round: u8,
        floor: DateTime<Utc>,
        ceiling: DateTime<Utc>,
        walked: u64,
        after: Option<(DateTime<Utc>, String)>,
    },
}

impl From<ReindexWalkCursor> for WalkStep {
    fn from(cursor: ReindexWalkCursor) -> Self {
        match cursor {
            ReindexWalkCursor::Id { floor, after_id } => WalkStep::IdPhase {
                floor,
                after_id: Some(after_id),
            },
            ReindexWalkCursor::Round {
                round,
                floor,
                ceiling,
                walked,
                after_last_updated,
                after_id,
            } => WalkStep::Round {
                round,
                floor,
                ceiling,
                walked,
                after: Some((after_last_updated, after_id)),
            },
        }
    }
}

/// Converts a returned page plus its next cursor into a [`ResourcePage`],
/// exactly as HEAD's `fetch_resources_page` did (`:4851-4863` at c86d0f08b).
fn reindex_page_from_docs(
    docs: &[Document],
    resource_type: &str,
    tenant: &TenantContext,
    next_cursor: ReindexWalkCursor,
) -> StorageResult<ResourcePage> {
    let resources = docs
        .iter()
        .map(|doc| {
            parse_history_row(doc, Some(resource_type), None)
                .map(|row| row.into_stored_resource(tenant))
        })
        .collect::<StorageResult<Vec<_>>>()?;
    Ok(ResourcePage {
        resources,
        next_cursor: Some(next_cursor.encode()),
        skipped: Vec::new(),
    })
}

/// Maximum number of distinct ids bound in one `fetch_resources_by_ids`
/// `$in` lookup (#1500). Mongo has no driver-imposed bind-count ceiling like
/// SQLite's; this mirrors PostgreSQL's constant of the same name and value.
const REINDEX_IDS_QUERY_SIZE: usize = 1000;

#[async_trait]
impl ReindexSource for MongoBackend {
    fn with_id_range(
        self: std::sync::Arc<Self>,
        range: ReindexIdRange,
    ) -> StorageResult<std::sync::Arc<dyn ReindexSource>> {
        Ok(std::sync::Arc::new(RangedMongoReindexSource {
            backend: self,
            range,
        }))
    }

    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>> {
        let db = self.get_database().await?;
        let resources: Collection<Document> = db.collection(MongoBackend::RESOURCES_COLLECTION);

        let types = resources
            .distinct(
                "resource_type",
                doc! { "tenant_id": tenant.tenant_id().as_str(), "is_deleted": false },
            )
            .await
            .map_err(|e| internal_error(format!("Failed to list resource types: {e}")))?;

        Ok(types
            .into_iter()
            .filter_map(|b| b.as_str().map(str::to_string))
            .collect())
    }

    async fn count_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64> {
        self.count(tenant, Some(resource_type)).await
    }

    /// Two phases per type (#1403): an id phase over live resources stamped
    /// before the floor, keyset on id and hinted to idx_resources_identity,
    /// then up to REINDEX_CATCH_UP_MAX_ROUNDS catch-up rounds over
    /// [floor, ceiling) in (last_updated, id) order on idx_resources_type_scan.
    /// A phase ends only on an empty query and the next phase starts in the
    /// same call, so the driver sees non-empty pages with Some(cursor) and one
    /// trailing empty page with None. The cursor is the versioned v2 grammar
    /// of ReindexWalkCursor.
    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> StorageResult<ResourcePage> {
        self.fetch_reindex_page(tenant, resource_type, cursor, limit, 0, None)
            .await
    }

    /// Pages by resource count and, when `max_bytes` is set, by the raw BSON
    /// bytes of the `resources` rows the page reads: a page never exceeds
    /// `max_bytes` unless it holds exactly one resource (PostgreSQL's strict
    /// rule, not SQLite's overshoot-by-one, #1499). A byte-capped page continues
    /// the walk's current phase — it is never empty, so it never ends a phase —
    /// and its cursor names the last row it *took*, never a row the cap rejected.
    async fn fetch_resources_page_capped(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
        max_bytes: u64,
    ) -> StorageResult<ResourcePage> {
        self.fetch_reindex_page(tenant, resource_type, cursor, limit, max_bytes, None)
            .await
    }

    /// Only an id-phase continuation cursor may run ahead of the write in
    /// flight (#1403): its query reads the `resources` collection, which the
    /// page being written never touches (only `search_index` and
    /// `search_index_contained` do), so prefetching it changes nothing the
    /// write could observe. A catch-up round's query instead reads up to a
    /// ceiling fixed when the round started, not "now" — but prefetching a
    /// round page before the previous page's write has ended could still let
    /// a write that lands between the two reads be missed by both the
    /// current round and the next one, so rounds are excluded too. A cursor
    /// that fails to parse is rejected the same way. `reindex_prefetch` gates
    /// all of this off.
    ///
    /// Search offload does not: on a composite deployment the page being
    /// written goes to Elasticsearch, which the id-phase query reads even
    /// less of than this backend's own index collections, so the same
    /// argument holds (#1250). The gate that #1403 placed here covered only
    /// what its bench measured, standalone MongoDB.
    fn may_prefetch_page(&self, cursor: &str) -> bool {
        self.config().reindex_prefetch
            && matches!(
                ReindexWalkCursor::parse(cursor),
                Ok(ReindexWalkCursor::Id { .. })
            )
    }

    async fn fetch_resources_page_ahead(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: &str,
        limit: u32,
        max_bytes: u64,
    ) -> StorageResult<Option<ResourcePage>> {
        self.fetch_reindex_page_ahead(tenant, resource_type, cursor, limit, max_bytes, None)
            .await
    }

    /// A direct, `$in`-bounded point lookup (#1500), replacing the default's
    /// full-type scan-and-filter, which on MongoDB re-runs the whole #1403
    /// walk (newest-live probe, id phase, catch-up rounds) from the start on
    /// every call because its cursor starts at `None`. Unordered per the
    /// trait contract, so no sort and no cursor; ids are deduped and chunked
    /// to keep each `$in` bounded, and the lookup is hinted to the same
    /// unique identity index the id phase uses. A row that fails to decode
    /// is skipped with a warning rather than failing the whole batch
    /// (mirrors SQLite's `fetch_resources_by_ids`) — a
    /// `GenerationScope::Resources` batch is never retried, so one bad row
    /// must not cost the other requested ids their reindex.
    async fn fetch_resources_by_ids(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        ids: &[String],
    ) -> StorageResult<Vec<StoredResource>> {
        let mut unique: Vec<&str> = ids.iter().map(String::as_str).collect();
        unique.sort_unstable();
        unique.dedup();
        if unique.is_empty() {
            return Ok(Vec::new());
        }

        let db = self.get_database().await?;
        let resources = db.collection::<Document>(Self::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let mut found = Vec::with_capacity(unique.len());
        let mut skipped = 0usize;
        for batch in unique.chunks(REINDEX_IDS_QUERY_SIZE) {
            let filter = doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "is_deleted": false,
                "id": { "$in": batch },
            };
            let mut cursor = resources
                .find(filter)
                .hint(Hint::Name(RESOURCES_IDENTITY_INDEX.to_string()))
                .await
                .map_err(|e| internal_error(format!("Failed to fetch resources by ids: {e}")))?;
            while cursor
                .advance()
                .await
                .map_err(|e| internal_error(format!("Failed to advance cursor: {e}")))?
            {
                let doc: Document = cursor
                    .deserialize_current()
                    .map_err(|e| internal_error(format!("Failed to read resource: {e}")))?;
                match parse_history_row(&doc, Some(resource_type), None) {
                    Ok(row) => found.push(row.into_stored_resource(tenant)),
                    Err(e) => {
                        skipped += 1;
                        let id = doc.get_str("id").unwrap_or("<unknown>");
                        tracing::warn!(
                            tenant = %tenant_id,
                            resource_type,
                            resource_id = %id,
                            error = %e,
                            "reindex source: stored resource row cannot be decoded; skipping it"
                        );
                    }
                }
            }
        }
        if skipped > 0 {
            tracing::warn!(
                tenant = %tenant_id,
                resource_type,
                skipped,
                requested = unique.len(),
                "reindex source: skipped undecodable rows while fetching resources by ids"
            );
        }
        Ok(found)
    }
}

#[async_trait]
impl ReindexTarget for MongoBackend {
    async fn delete_search_entries(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<u64> {
        // Honors `is_search_offloaded()`: when Elasticsearch owns search, this
        // backend keeps no index of its own and there is nothing to delete.
        if self.is_search_offloaded() {
            return Ok(0);
        }

        let db = self.get_database().await?;
        let filter = doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": resource_type,
            "resource_id": resource_id,
        };

        let result = db
            .collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION)
            .delete_many(filter.clone())
            .await
            .map_err(|e| internal_error(format!("Failed to delete search entries: {e}")))?;

        // Contained rows key the same way (#1160 Task 4), so this default
        // `ReindexTarget` per-resource delete clears both collections too —
        // keeping the invariant intact even though `write_search_entries_page`
        // below overrides the page-level caller, making this single-resource
        // path unreachable on MongoDB today.
        let contained_result = db
            .collection::<Document>(MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION)
            .delete_many(filter)
            .await
            .map_err(|e| {
                internal_error(format!(
                    "Failed to delete search_index_contained entries: {e}"
                ))
            })?;

        Ok(result.deleted_count + contained_result.deleted_count)
    }

    async fn write_search_entries(
        &self,
        tenant: &TenantContext,
        resource: &StoredResource,
    ) -> StorageResult<usize> {
        // Delegates to the page method (a slice of one) so the single-resource
        // and batched-reindex paths write and count identically by
        // construction, and so this no longer pays a redundant second
        // `extract()` purely to compute a count (#1064) — a count that also
        // omitted any `_contained` rows the write itself inserted.
        self.write_search_entries_page(tenant, std::slice::from_ref(resource))
            .await
            .pop()
            .unwrap_or(Ok(0))
    }

    async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
        self.clear_search_index_for_types(tenant, None).await
    }

    async fn clear_search_index_for_types(
        &self,
        tenant: &TenantContext,
        resource_types: Option<&[String]>,
    ) -> StorageResult<u64> {
        if resource_types.is_some_and(|types| types.is_empty()) {
            return Ok(0);
        }
        if self.is_search_offloaded() {
            return Ok(0);
        }

        let db = self.get_database().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let mut filter = doc! { "tenant_id": tenant_id };
        if let Some(types) = resource_types {
            filter.insert("resource_type", doc! { "$in": types });
        }
        let result = db
            .collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION)
            .delete_many(filter.clone())
            .await
            .or_query_error("Failed to clear search index")?;

        // Contained rows carry their container's `resource_type`, so the same
        // filter clears exactly the contained rows of the containers this run
        // rebuilds (#1160 Task 4: none are left behind as orphans).
        let contained_result = db
            .collection::<Document>(MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION)
            .delete_many(filter)
            .await
            .or_query_error("Failed to clear contained search index")?;

        Ok(result.deleted_count + contained_result.deleted_count)
    }

    /// Rebuilds a whole page in one `delete_many` plus one (possibly chunked)
    /// `insert_many`, instead of the default's two `delete_many` plus one
    /// `insert_many` PER RESOURCE — `delete_search_entries` above, then
    /// `write_search_entries` -> `index_resource`'s own delete-then-insert
    /// (#1064). For a 100-resource page that was 300 sequential round trips;
    /// this is two (three only if a page mixes resource types, which no
    /// production caller does — see the precondition below).
    ///
    /// Building every resource's documents through
    /// [`Self::search_index_documents_checked`] keeps a rebuild and the CRUD
    /// path (`index_resource`) indexing identically by construction, and
    /// lets one resource's bad content fall back to minimal rows (as
    /// `index_resource` already does) while still reporting that resource as
    /// failed — mirroring what the old, per-resource `write_search_entries`
    /// did, and what SQLite's `write_search_entries_on` and Elasticsearch's
    /// override still do for the same case.
    ///
    /// The count each `Ok` reports is the number of documents actually
    /// written for that resource, which — unlike the old
    /// `write_search_entries`'s `extract(..).len()` — includes any
    /// `_contained` rows. `$reindex-status.entries_created` will read higher
    /// for corpora with `contained` resources as a result; this is a more
    /// truthful count of what was written, and matches SQLite's
    /// `write_search_entries_on` (which also adds `index_contained_resources`'
    /// count).
    ///
    /// Precondition: `resources` must hold each `(resource_type, id)` at most
    /// once; unlike Elasticsearch's `_id`-keyed upsert, a repeated id here
    /// would double-insert, because the delete for the whole page runs once,
    /// up front. The production caller, `fetch_resources_page`, guarantees
    /// it: an id-phase page walks the unique `idx_resources_identity` in key
    /// order, and a catch-up page is de-duplicated by id (keeping the newest
    /// version) before it is returned (#1403). The same resource in two
    /// different calls is expected — a catch-up round rewrites what the id
    /// phase wrote.
    ///
    /// A page-level failure — getting the database handle, the grouped
    /// delete, or an insert error the driver does not attribute to a specific
    /// document — reports the same `Err` for every resource in the page,
    /// because the failure cannot be attributed to individual documents
    /// (mirroring SQLite's BEGIN/COMMIT fan-out and Elasticsearch's
    /// `ensure_index` fan-out for the same reason). An unordered
    /// `insert_many` write error IS attributed to just the document(s) it
    /// names, via the same per-op index mapping the batched bulk-submit
    /// ingest uses (`bulk_ingest.rs`'s create-batch path) and that
    /// Elasticsearch's `send_bulk_index` uses for the same purpose.
    ///
    /// A page of `REINDEX_SUBBATCH_FIRST` resources or fewer, and every page
    /// on a current-thread runtime, always runs through the serial writer:
    /// one delete phase, then one insert phase (own rows, then contained
    /// rows, each chunked into `SEARCH_INDEX_INSERT_CHUNK`-sized `insert_many`
    /// commands). The delete completes fully before the insert phase starts,
    /// but a chunked insert can still leave some rows behind even when it
    /// goes on to fail: an earlier chunk that already committed keeps its
    /// rows, and the chunk that actually errored may keep some, all, or none
    /// of its own — an unordered `insert_many` failure (for example a
    /// write-concern error reported after the documents were written, or a
    /// transport error after partial application) does not guarantee the
    /// failing chunk inserted nothing. A larger page on a multi-thread
    /// runtime with the overlap
    /// configuration on instead runs through the overlapped writer, which
    /// splits the page into several sub-batches and inserts one while
    /// extracting the next; there too, a sub-batch insert failure reports
    /// `Err` for every resource in the page, but rows the earlier,
    /// already-completed sub-batches — and any chunk of the failing
    /// sub-batch that committed before the failure — inserted remain in the
    /// database.
    async fn write_search_entries_page_timed(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        stats: &mut ReindexPageStats,
    ) -> Vec<StorageResult<usize>> {
        if resources.is_empty() {
            return Vec::new();
        }
        let _page_span = crate::perf::span(crate::perf::Phase::ReindexPage);
        // Honors `is_search_offloaded()`: when Elasticsearch owns search,
        // this backend keeps no index of its own, so there is nothing to
        // delete or insert and every resource reports 0 entries written.
        if self.is_search_offloaded() {
            return resources.iter().map(|_| Ok(0)).collect();
        }
        let db = match self.get_database().await {
            Ok(db) => db,
            Err(e) => {
                let msg = e.to_string();
                return resources
                    .iter()
                    .map(|_| Err(internal_error(msg.clone())))
                    .collect();
            }
        };
        let tenant_id = tenant.tenant_id().as_str();
        let multi_thread = super::reindex_pipeline::tokio_multi_thread_runtime();
        let overlapped = self.config().reindex_overlap
            && multi_thread
            && resources.len() > super::reindex_pipeline::REINDEX_SUBBATCH_FIRST;
        if resources.len() > super::reindex_pipeline::REINDEX_SUBBATCH_FIRST {
            self.log_reindex_mode_once(multi_thread, overlapped);
        }
        if overlapped {
            self.write_page_overlapped(&db, tenant_id, resources, stats)
                .await
        } else {
            self.write_page_serial(&db, tenant_id, resources, stats, multi_thread)
                .await
        }
    }

    /// Delegates to [`Self::write_search_entries_page_timed`] with a
    /// throwaway `ReindexPageStats`, so the two cannot diverge (#1403).
    async fn write_search_entries_page(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
    ) -> Vec<StorageResult<usize>> {
        let mut stats = ReindexPageStats::default();
        self.write_search_entries_page_timed(tenant, resources, &mut stats)
            .await
    }
}

/// Documents per `insert_many` when the serial or overlapped writer's
/// `insert_sub_batch` flattens a sub-batch's index documents into one
/// insert. Mirrors `bulk_ingest.rs`'s `INSERT_DOCS_PER_COMMAND` (same value,
/// same rationale: bound how much the driver serializes per command) without
/// depending on that module, since a sub-batch's `search_index` documents
/// are built the same way a batch's are.
pub(super) const SEARCH_INDEX_INSERT_CHUNK: usize = 5_000;

/// Chunked, unordered `insert_many` of `docs` into `collection`, attributing
/// each document to the resource index at the same position in `owners`.
///
/// Called by `insert_sub_batch` once per destination collection
/// (`search_index` for a sub-batch's own rows, `search_index_contained` for
/// its contained rows, if any) so a failed contained insert attributes back
/// to its resource exactly like a failed own insert.
///
/// Returns the per-resource write failures found, each message already
/// carrying `error_context` (so a `search_index_contained` failure reads as
/// that, not as "Failed to insert search index entries" regardless of which
/// collection actually failed). A page-level error — one the driver did not
/// attribute to specific documents — is returned as `Err`, for the caller to
/// fan out to every resource in the page. Also counts each command it issues,
/// and the documents in it, into `stats` (#1403).
pub(super) async fn insert_search_entries_chunk(
    collection: &mongodb::Collection<Document>,
    owners: &[usize],
    docs: &[Document],
    error_context: &str,
    stats: &mut ReindexPageStats,
) -> Result<HashMap<usize, String>, String> {
    let mut insert_failures: HashMap<usize, String> = HashMap::new();
    let mut offset = 0usize;
    for chunk in docs.chunks(SEARCH_INDEX_INSERT_CHUNK) {
        stats.insert_commands += 1;
        stats.inserted_entries += chunk.len() as u64;
        match collection.insert_many(chunk).ordered(false).await {
            Ok(_) => {}
            Err(e) => match e.kind.as_ref() {
                MongoErrorKind::InsertMany(insert_many) => {
                    let Some(write_errors) = insert_many.write_errors.as_ref() else {
                        return Err(format!("{error_context}: {e}"));
                    };
                    for write_error in write_errors {
                        let owner = owners[offset + write_error.index];
                        insert_failures
                            .entry(owner)
                            .or_insert_with(|| format!("{error_context}: {}", write_error.message));
                    }
                }
                _ => return Err(format!("{error_context}: {e}")),
            },
        }
        offset += chunk.len();
    }
    Ok(insert_failures)
}

/// Most catch-up rounds one type's `$reindex` walk runs (#1403).
const REINDEX_CATCH_UP_MAX_ROUNDS: u8 = 3;
/// Smallest catch-up margin honoured: below it every round would count as
/// "needed" and a quiescent type would run all rounds.
const REINDEX_CATCH_UP_MARGIN_MIN_MS: u64 = 1_000;
/// Largest catch-up margin honoured, so `t0 - margin` stays in range.
const REINDEX_CATCH_UP_MARGIN_MAX_MS: u64 = 86_400_000;

/// The walk position handed to the driver between calls (#1403). `v2|` and a
/// tag version the grammar; anything else is a foreign or corrupt cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ReindexWalkCursor {
    Id {
        floor: DateTime<Utc>,
        after_id: String,
    },
    Round {
        round: u8,
        floor: DateTime<Utc>,
        ceiling: DateTime<Utc>,
        walked: u64,
        after_last_updated: DateTime<Utc>,
        after_id: String,
    },
}

impl ReindexWalkCursor {
    fn encode(&self) -> String {
        match self {
            ReindexWalkCursor::Id { floor, after_id } => {
                format!("v2|i|{}|{}", format_walk_instant(*floor), after_id)
            }
            ReindexWalkCursor::Round {
                round,
                floor,
                ceiling,
                walked,
                after_last_updated,
                after_id,
            } => format!(
                "v2|c|{}|{}|{}|{}|{}|{}",
                round,
                format_walk_instant(*floor),
                format_walk_instant(*ceiling),
                walked,
                format_walk_instant(*after_last_updated),
                after_id
            ),
        }
    }

    /// Anything that does not exactly match the grammar (including HEAD's
    /// `<rfc3339>|<id>` and an empty string) is `SearchError::InvalidCursor`.
    /// A cursor this process did not produce means there is a bug; restarting
    /// the type could loop forever, so the run fails instead (#1403).
    fn parse(cursor: &str) -> StorageResult<Self> {
        let invalid = || {
            StorageError::Search(SearchError::InvalidCursor {
                cursor: cursor.to_string(),
            })
        };
        let rest = cursor.strip_prefix("v2|").ok_or_else(invalid)?;
        let (tag, rest) = rest.split_once('|').ok_or_else(invalid)?;
        match tag {
            "i" => {
                let (floor, after_id) = rest.split_once('|').ok_or_else(invalid)?;
                if after_id.is_empty() {
                    return Err(invalid());
                }
                let floor = DateTime::parse_from_rfc3339(floor)
                    .map_err(|_| invalid())?
                    .with_timezone(&Utc);
                Ok(ReindexWalkCursor::Id {
                    floor,
                    after_id: after_id.to_string(),
                })
            }
            "c" => {
                let fields: Vec<&str> = rest.splitn(6, '|').collect();
                let [round, floor, ceiling, walked, after_lu, after_id] = fields[..] else {
                    return Err(invalid());
                };
                if after_id.is_empty() {
                    return Err(invalid());
                }
                let round: u8 = round.parse().map_err(|_| invalid())?;
                if !(1..=REINDEX_CATCH_UP_MAX_ROUNDS).contains(&round) {
                    return Err(invalid());
                }
                let walked: u64 = walked.parse().map_err(|_| invalid())?;
                let floor = DateTime::parse_from_rfc3339(floor)
                    .map_err(|_| invalid())?
                    .with_timezone(&Utc);
                let ceiling = DateTime::parse_from_rfc3339(ceiling)
                    .map_err(|_| invalid())?
                    .with_timezone(&Utc);
                let after_last_updated = DateTime::parse_from_rfc3339(after_lu)
                    .map_err(|_| invalid())?
                    .with_timezone(&Utc);
                if !(floor < ceiling && floor <= after_last_updated && after_last_updated < ceiling)
                {
                    return Err(invalid());
                }
                Ok(ReindexWalkCursor::Round {
                    round,
                    floor,
                    ceiling,
                    walked,
                    after_last_updated,
                    after_id: after_id.to_string(),
                })
            }
            _ => Err(invalid()),
        }
    }
}

/// Whether round-start should run the round, declare the walk complete, or
/// stop at the round cap (#1403).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoundStartDecision {
    Run,
    Complete,
    CapReached,
}

fn truncate_to_millis(dt: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(dt.timestamp_millis()).unwrap_or(dt)
}

fn format_walk_instant(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Clamps a configured margin to `[REINDEX_CATCH_UP_MARGIN_MIN_MS,
/// REINDEX_CATCH_UP_MARGIN_MAX_MS]` (#1403).
fn reindex_catch_up_margin(configured_ms: u64) -> chrono::Duration {
    chrono::Duration::milliseconds(configured_ms.clamp(
        REINDEX_CATCH_UP_MARGIN_MIN_MS,
        REINDEX_CATCH_UP_MARGIN_MAX_MS,
    ) as i64)
}

/// `min(newest_live + 1 ms, t0 - margin)` (#1403).
fn reindex_catch_up_floor(
    t0: DateTime<Utc>,
    newest_live: Option<DateTime<Utc>>,
    margin: chrono::Duration,
) -> DateTime<Utc> {
    let fresh = truncate_to_millis(t0 - margin);
    match newest_live {
        Some(newest) => (newest + chrono::Duration::milliseconds(1)).min(fresh),
        None => fresh,
    }
}

/// `max(now + margin, newest_live + 1 ms)` (#1403).
fn reindex_catch_up_ceiling(
    now: DateTime<Utc>,
    newest_live: Option<DateTime<Utc>>,
    margin: chrono::Duration,
) -> DateTime<Utc> {
    let by_margin = truncate_to_millis(now + margin);
    match newest_live {
        Some(newest) => by_margin.max(newest + chrono::Duration::milliseconds(1)),
        None => by_margin,
    }
}

/// Whether the next round should run, or the walk is done (#1403).
fn reindex_round_start_decision(
    round: u8,
    floor: DateTime<Utc>,
    now: DateTime<Utc>,
    margin: chrono::Duration,
) -> RoundStartDecision {
    if round == 1 {
        return RoundStartDecision::Run;
    }
    if now < floor - margin / 2 {
        return RoundStartDecision::Complete;
    }
    if round > REINDEX_CATCH_UP_MAX_ROUNDS {
        return RoundStartDecision::CapReached;
    }
    RoundStartDecision::Run
}

/// The id phase's filter: live resources older than `floor`, keyset on `id`
/// (#1403), within `range` (#1739).
fn reindex_id_page_filter(
    tenant_id: &str,
    resource_type: &str,
    floor: DateTime<Utc>,
    after_id: Option<&str>,
    range: Option<&ReindexIdRange>,
) -> Document {
    let mut filter = doc! {
        "tenant_id": tenant_id,
        "resource_type": resource_type,
        "is_deleted": false,
        "last_updated": { "$lt": chrono_to_bson(floor) },
    };
    if let Some(bounds) = reindex_id_bounds(after_id, range) {
        filter.insert("id", bounds);
    }
    filter
}

/// The `id` condition of a reindex query: after `after_id` (the keyset) and
/// within `range` (#1739), or `None` when neither applies.
fn reindex_id_bounds(after_id: Option<&str>, range: Option<&ReindexIdRange>) -> Option<Document> {
    let mut bounds = Document::new();
    if let Some(after_id) = after_id {
        bounds.insert("$gt", after_id);
    }
    if let Some(start) = range.and_then(|range| range.start.as_deref()) {
        bounds.insert("$gte", start);
    }
    if let Some(end) = range.and_then(|range| range.end.as_deref()) {
        bounds.insert("$lt", end);
    }
    (!bounds.is_empty()).then_some(bounds)
}

/// A catch-up round's filter over `[floor, ceiling)`, keyset on
/// `(last_updated, id)` once a page has been returned (#1403), within
/// `range` (#1739).
fn reindex_catch_up_page_filter(
    tenant_id: &str,
    resource_type: &str,
    floor: DateTime<Utc>,
    ceiling: DateTime<Utc>,
    after: Option<(DateTime<Utc>, &str)>,
    range: Option<&ReindexIdRange>,
) -> Document {
    let mut filter = doc! {
        "tenant_id": tenant_id,
        "resource_type": resource_type,
        "is_deleted": false,
    };
    if let Some(bounds) = reindex_id_bounds(None, range) {
        filter.insert("id", bounds);
    }
    match after {
        None => {
            filter.insert(
                "last_updated",
                doc! { "$gte": chrono_to_bson(floor), "$lt": chrono_to_bson(ceiling) },
            );
        }
        Some((after_lu, after_id)) => {
            filter.insert(
                "$or",
                vec![
                    doc! { "last_updated": { "$gt": chrono_to_bson(after_lu), "$lt": chrono_to_bson(ceiling) } },
                    doc! { "last_updated": chrono_to_bson(after_lu), "id": { "$gt": after_id } },
                ],
            );
        }
    }
    filter
}

/// Keeps only the last (newest) occurrence of each `id` in `docs`, preserving
/// scan order otherwise; a document with no string `id` is kept in place and
/// left to fail parsing with HEAD's error (#1403).
fn dedupe_reindex_page_keep_last(docs: Vec<Document>) -> Vec<Document> {
    let mut last_index_for_id: HashMap<String, usize> = HashMap::new();
    for (i, doc) in docs.iter().enumerate() {
        if let Ok(id) = doc.get_str("id") {
            last_index_for_id.insert(id.to_string(), i);
        }
    }
    docs.into_iter()
        .enumerate()
        .filter(|(i, doc)| match doc.get_str("id").ok() {
            Some(id) => last_index_for_id.get(id) == Some(i),
            None => true,
        })
        .map(|(_, doc)| doc)
        .collect()
}

#[cfg(test)]
mod offloaded_identifier_guard_tests {
    use super::*;
    use crate::types::{SearchModifier, SearchValue};

    fn identifier_param() -> SearchParameter {
        SearchParameter {
            name: "identifier".to_string(),
            param_type: SearchParamType::Token,
            values: vec![SearchValue::eq("MRN-1")],
            ..Default::default()
        }
    }

    #[test]
    fn rejects_non_token_type() {
        let param = SearchParameter {
            param_type: SearchParamType::String,
            ..identifier_param()
        };
        let err = MongoBackend::validate_offloaded_identifier_param(&param).unwrap_err();
        assert!(
            err.to_string()
                .contains("unsupported parameter type 'string'")
        );
    }

    #[test]
    fn rejects_modifier() {
        let param = SearchParameter {
            modifier: Some(SearchModifier::Missing),
            ..identifier_param()
        };
        let err = MongoBackend::validate_offloaded_identifier_param(&param).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn rejects_non_eq_prefix() {
        let param = SearchParameter {
            values: vec![SearchValue::new(SearchPrefix::Ne, "MRN-1")],
            ..identifier_param()
        };
        let err = MongoBackend::validate_offloaded_identifier_param(&param).unwrap_err();
        assert!(err.to_string().contains("Unsupported prefix 'ne'"));
    }

    #[test]
    fn accepts_plain_token() {
        MongoBackend::validate_offloaded_identifier_param(&identifier_param()).unwrap();
    }
}

#[cfg(test)]
mod index_date_tests {
    use super::*;

    /// A search value and the stored value it should match must never be zoned
    /// differently. Both are read by the shared `FhirDateValue` (the writer
    /// since #1315): for every value the search grammar accepts, the instant
    /// indexed must be the start of the range searched (both are then cut to
    /// the millisecond a BSON date holds).
    ///
    /// That includes what a resource cannot validly carry but real data does —
    /// `hh:mm` without seconds — and a `:60` leap second, which the search
    /// side reads as the next second.
    #[test]
    fn search_and_index_agree_on_every_valid_value() {
        for value in [
            "2013",
            "2013-04",
            "2013-12",
            "2013-04-05",
            "2024-02-29",
            "2013-04-05T09:20:00",
            "2013-04-05T09:20:00Z",
            "2013-04-05T09:20:00-04:00",
            "2013-04-05T18:50:00+05:30",
            "2013-04-05T09:20:00-00:00",
            "2013-04-05T23:20:00+14:00",
            "2013-04-05T09:20:00.5Z",
            "2013-04-05T23:30:00.123-04:00",
            "2021-11-10T16:48:57.246958-08:00",
            // Minutes without seconds (#1315).
            "2013-04-05T09:20",
            "2013-04-05T09:20Z",
            "2013-04-05T09:20-04:00",
            "2013-04-05T18:50+05:30",
            "2013-04-05T09:20-00:00",
            "2013-04-05T23:59-14:00",
            // A leap second is the first instant of the next second.
            "2016-12-31T23:59:60Z",
            "2013-04-05T09:20:60",
            "2016-12-31T18:59:60-05:00",
            "2016-12-31T23:59:60.5Z",
            // Nine fraction digits, and digits past the ninth.
            "2013-04-05T09:20:00.123456789Z",
            "2013-04-05T09:20:00.1234567891Z",
            "2013-04-05T09:20:00.12345678912345-04:00",
            // The edges of the supported years.
            "0001",
            "0001-01-01T00:00:00Z",
            "0001-01-01T14:00:00+14:00",
            "9999",
            "9999-12-31",
            "9999-12-31T23:59",
            "9999-12-31T23:59:59Z",
            "9999-12-31T09:59:59-14:00",
        ] {
            let searched = crate::search::FhirDateValue::parse(value)
                .unwrap_or_else(|e| panic!("{value} is a valid search value: {e}"));
            assert_eq!(
                normalize_date_for_mongo(value),
                Some(searched.start),
                "{value}"
            );
            let (start, _) = searched.range_at(crate::search::StorageResolution::Millis);
            assert_eq!(
                normalize_date_for_mongo(value).map(chrono_to_bson),
                Some(chrono_to_bson(start)),
                "{value} at BSON resolution"
            );
        }
    }

    /// #1315 itself: minutes without seconds are not RFC 3339, so the lenient
    /// reading — all there was — dropped the value and the index document was
    /// skipped.
    #[test]
    fn minute_precision_values_are_indexed() {
        for (value, expected) in [
            ("2013-04-05T09:20", "2013-04-05T09:20:00+00:00"),
            ("2013-04-05T09:20Z", "2013-04-05T09:20:00+00:00"),
            ("2013-04-05T09:20-04:00", "2013-04-05T13:20:00+00:00"),
            ("2013-04-05T18:50+05:30", "2013-04-05T13:20:00+00:00"),
        ] {
            assert_eq!(
                normalize_date_for_mongo_lenient(value),
                None,
                "{value} before"
            );
            assert_eq!(
                normalize_date_for_mongo(value).map(|t| t.to_rfc3339()),
                Some(expected.to_string()),
                "{value}"
            );
        }
    }

    /// The only value both readings accept and disagree on. Chrono keeps a
    /// leap second as `:59` plus a second of nanoseconds, which a BSON date
    /// holds as `:59.999`-and-a-bit at best; the search side looks for it *at*
    /// the next second.
    #[test]
    fn leap_second_is_indexed_where_the_search_side_looks_for_it() {
        let lenient =
            normalize_date_for_mongo_lenient("2016-12-31T23:59:60Z").expect("chrono reads it");
        assert_eq!(lenient.timestamp(), 1_483_228_799, "lenient: still :59");
        let indexed = normalize_date_for_mongo("2016-12-31T23:59:60Z").expect("indexed");
        assert_eq!(indexed.to_rfc3339(), "2017-01-01T00:00:00+00:00");
    }

    /// The search side trims a value and reads a space in the zone-sign
    /// position as a form-decoded `+` (#1296). Neither applies to a stored
    /// value: there a space is not part of a date, and the value is skipped as
    /// it always was rather than indexed at a zone nobody wrote.
    #[test]
    fn search_side_repairs_do_not_apply_to_stored_values() {
        for value in [
            "2013-04-05T18:50:00 05:30",
            "2013-04-05T18:50 05:30",
            " 2013-04-05T09:20",
            "2013-04-05T09:20 ",
            " 2013-04-05 ",
        ] {
            assert!(
                crate::search::FhirDateValue::parse(value).is_ok(),
                "{value:?} is accepted as a search value"
            );
            assert_eq!(normalize_date_for_mongo(value), None, "{value:?}");
        }
    }

    fn date_document(value: IndexValue) -> Option<Document> {
        let backend = MongoBackend::new(super::super::backend::MongoBackendConfig::default())
            .expect("backend without a connection");
        let extracted = ExtractedValue::new(
            "date",
            "http://hl7.org/fhir/SearchParameter/clinical-date",
            crate::types::SearchParamType::Date,
            value,
        );
        backend.build_search_index_document("t1", "Encounter", "e1", &extracted)
    }

    fn stored(doc: &Document, field: &str) -> String {
        bson_to_chrono(doc.get_datetime(field).expect(field)).to_rfc3339()
    }

    /// #1391: every date row stores the range it covers — a point to the end
    /// of its precision, a `Period` to the end of its own `end`, and an open
    /// side at the edge of the supported years.
    #[test]
    fn date_rows_store_the_range_they_cover() {
        let point = date_document(IndexValue::date("2020-06")).expect("point");
        assert_eq!(stored(&point, "value_date"), "2020-06-01T00:00:00+00:00");
        assert_eq!(
            stored(&point, "value_date_end"),
            "2020-07-01T00:00:00+00:00"
        );

        let period = date_document(
            IndexValue::date_range(Some("2019-06-15"), Some("2020-03")).expect("period"),
        )
        .expect("period row");
        assert_eq!(stored(&period, "value_date"), "2019-06-15T00:00:00+00:00");
        assert_eq!(
            stored(&period, "value_date_end"),
            "2020-04-01T00:00:00+00:00"
        );

        let open_end = date_document(IndexValue::date_range(Some("2019"), None).expect("open"))
            .expect("open-ended row");
        assert_eq!(stored(&open_end, "value_date"), "2019-01-01T00:00:00+00:00");
        assert_eq!(
            *open_end.get_datetime("value_date_end").unwrap(),
            chrono_to_bson(crate::search::open_end(
                crate::search::StorageResolution::Millis
            ))
        );

        let open_start = date_document(IndexValue::date_range(None, Some("2020")).expect("open"))
            .expect("open-started row");
        assert_eq!(
            *open_start.get_datetime("value_date").unwrap(),
            chrono_to_bson(crate::search::open_start())
        );
        assert_eq!(
            stored(&open_start, "value_date_end"),
            "2021-01-01T00:00:00+00:00"
        );
    }

    /// A `Period` whose `end` is not a date is skipped whole, like any other
    /// unparseable date: indexing it as open would over-match.
    #[test]
    fn a_period_with_a_bad_end_is_skipped() {
        let bad = IndexValue::date_range(Some("2020-01-01"), Some("not-a-date")).expect("period");
        assert!(date_document(bad).is_none());
    }

    /// What the strict grammar rejects still goes through the lenient reading,
    /// exactly as before: the strict pass only ever adds index documents.
    #[test]
    fn values_outside_the_grammar_keep_the_lenient_reading() {
        for value in [
            // Offset beyond ±14:00.
            "2013-04-05T09:20:00+14:30",
            // Valid text whose UTC instant is past the year 9999.
            "9999-12-31T23:59:59-01:00",
            // Its range would have no width left inside the supported years.
            "9999-12-31T23:59:59.999999999Z",
        ] {
            assert!(
                crate::search::FhirDateValue::parse(value).is_err(),
                "{value} is outside the search grammar"
            );
            assert!(normalize_date_for_mongo_lenient(value).is_some(), "{value}");
            assert_eq!(
                normalize_date_for_mongo(value),
                normalize_date_for_mongo_lenient(value),
                "{value}"
            );
        }
    }

    /// Never a timestamp for something that is not a date.
    #[test]
    fn unparseable_values_are_dropped_not_substituted() {
        for value in ["", "not-a-date", "2024-13-45T99:99:99", "T00:00:00"] {
            assert_eq!(normalize_date_for_mongo(value), None, "{value:?}");
        }
    }
}

#[cfg(test)]
mod history_query_tests {
    //! Docker-free unit tests for the pure query builders behind #1053's
    //! fix: the server-side cursor predicate, sort, and fetch-limit that
    //! `history_type`/`history_system` now push to MongoDB instead of
    //! draining the whole history corpus into Rust before paging.

    use super::*;
    use crate::types::Pagination;

    fn cursor_params(sort_values: Vec<CursorValue>, resource_id: &str) -> HistoryParams {
        let cursor = PageCursor::new(sort_values, resource_id);
        HistoryParams {
            pagination: Pagination::with_cursor(10, cursor.encode()),
            ..HistoryParams::default()
        }
    }

    #[test]
    fn type_history_cursor_or_matches_expected_shape() {
        let ts = DateTime::parse_from_rfc3339("2024-01-01T00:00:10Z")
            .unwrap()
            .with_timezone(&Utc);
        let params = cursor_params(
            vec![
                CursorValue::String(ts.to_rfc3339()),
                CursorValue::String("obs-5".to_string()),
            ],
            "Observation",
        );

        let or_branches = type_history_cursor_or(&params).expect("expected a cursor predicate");
        let expected_ts = chrono_to_bson(ts);
        assert_eq!(
            or_branches,
            vec![
                doc! { "last_updated": { "$lt": expected_ts } },
                doc! { "last_updated": expected_ts, "id": { "$lt": "obs-5" } },
            ]
        );
    }

    #[test]
    fn system_history_cursor_or_matches_expected_3_branch_shape() {
        let ts = DateTime::parse_from_rfc3339("2024-01-01T00:00:10Z")
            .unwrap()
            .with_timezone(&Utc);
        let params = cursor_params(
            vec![
                CursorValue::String(ts.to_rfc3339()),
                CursorValue::String("Observation".to_string()),
                CursorValue::String("obs-5".to_string()),
            ],
            "system",
        );

        let or_branches = system_history_cursor_or(&params).expect("expected a cursor predicate");
        let expected_ts = chrono_to_bson(ts);
        assert_eq!(
            or_branches,
            vec![
                doc! { "last_updated": { "$lt": expected_ts } },
                doc! {
                    "last_updated": expected_ts,
                    "resource_type": { "$lt": "Observation" },
                },
                doc! {
                    "last_updated": expected_ts,
                    "resource_type": "Observation",
                    "id": { "$lt": "obs-5" },
                },
            ]
        );
    }

    #[test]
    fn cursor_or_is_none_for_absent_cursor() {
        let params = HistoryParams::default();
        assert!(type_history_cursor_or(&params).is_none());
        assert!(system_history_cursor_or(&params).is_none());
    }

    #[test]
    fn cursor_or_is_none_for_malformed_cursor() {
        // Only one sort value: type history needs 2, system needs 3.
        let ts = Utc::now();
        let params = cursor_params(vec![CursorValue::String(ts.to_rfc3339())], "Observation");
        assert!(type_history_cursor_or(&params).is_none());
        assert!(system_history_cursor_or(&params).is_none());

        // Two sort values: enough for type history, not for system history.
        let params2 = cursor_params(
            vec![
                CursorValue::String(ts.to_rfc3339()),
                CursorValue::String("obs-5".to_string()),
            ],
            "Observation",
        );
        assert!(type_history_cursor_or(&params2).is_some());
        assert!(system_history_cursor_or(&params2).is_none());
    }

    #[test]
    fn cursor_or_is_none_for_offset_mode_pagination() {
        let params = HistoryParams {
            pagination: Pagination::offset(5),
            ..HistoryParams::default()
        };
        assert!(type_history_cursor_or(&params).is_none());
        assert!(system_history_cursor_or(&params).is_none());
    }

    #[test]
    fn since_before_range_coexists_with_cursor_or_without_key_collision() {
        let since = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let before = DateTime::parse_from_rfc3339("2024-06-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let cursor_ts = DateTime::parse_from_rfc3339("2024-03-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let mut params = cursor_params(
            vec![
                CursorValue::String(cursor_ts.to_rfc3339()),
                CursorValue::String("obs-5".to_string()),
            ],
            "Observation",
        );
        params.since = Some(since);
        params.before = Some(before);

        let mut filter = doc! { "tenant_id": "tenant-1", "resource_type": "Observation" };
        apply_history_params_filter(&mut filter, &params);
        let or_branches = type_history_cursor_or(&params).expect("expected a cursor predicate");
        filter.insert("$or", or_branches);

        // Both the range (under "last_updated") and the cursor predicate
        // (under the distinct top-level "$or" key) are present — they AND
        // together implicitly rather than colliding on the same key.
        let range = filter
            .get_document("last_updated")
            .expect("expected the since/before range to remain under last_updated");
        assert_eq!(range.get("$gte"), Some(&Bson::from(chrono_to_bson(since))));
        assert_eq!(range.get("$lt"), Some(&Bson::from(chrono_to_bson(before))));
        assert!(filter.contains_key("$or"));
        // include_deleted defaults to false, so the filter also carries it.
        assert_eq!(filter.get_bool("is_deleted").ok(), Some(false));
    }

    #[test]
    fn history_fetch_limit_is_count_plus_one_and_never_zero() {
        assert_eq!(history_fetch_limit(10), 11);
        assert_eq!(history_fetch_limit(0), 1);
        assert_eq!(history_fetch_limit(99), 100);
    }

    #[test]
    fn sorts_match_the_serving_index_key_order() {
        // idx_history_type_updated = {tenant_id, resource_type, last_updated: -1, id: -1}
        assert_eq!(
            type_history_sort(),
            doc! { "last_updated": -1_i32, "id": -1_i32 }
        );
        // idx_history_system_updated = {tenant_id, last_updated: -1, resource_type: -1, id: -1}
        assert_eq!(
            system_history_sort(),
            doc! { "last_updated": -1_i32, "resource_type": -1_i32, "id": -1_i32 }
        );
    }
}

#[cfg(test)]
mod reindex_walk_tests {
    //! Docker-free unit tests for #1403's id-order `$reindex` walk: the
    //! cursor grammar, the pure floor/ceiling/round-decision rules, the
    //! filter builders, and the round-page dedupe. The walk itself
    //! (`fetch_resources_page`) is covered by the MongoDB integration suite
    //! in `tests/mongodb/reindex_id_walk.rs`, since it needs a live server.

    use super::*;

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    // --- Cursor grammar ---

    #[test]
    fn cursor_round_trips_every_state() {
        let id_cursor = ReindexWalkCursor::Id {
            floor: ts("2026-01-01T00:00:00.123Z"),
            after_id: "A-1.b".to_string(),
        };
        assert_eq!(
            ReindexWalkCursor::parse(&id_cursor.encode()).unwrap(),
            id_cursor
        );

        for round in [1u8, REINDEX_CATCH_UP_MAX_ROUNDS] {
            for walked in [0u64, u64::MAX] {
                let round_cursor = ReindexWalkCursor::Round {
                    round,
                    floor: ts("2026-01-01T00:00:00.000Z"),
                    ceiling: ts("2026-01-01T00:02:00.000Z"),
                    walked,
                    after_last_updated: ts("2026-01-01T00:01:00.500Z"),
                    after_id: "obs-017".to_string(),
                };
                assert_eq!(
                    ReindexWalkCursor::parse(&round_cursor.encode()).unwrap(),
                    round_cursor
                );
            }
        }
    }

    #[test]
    fn cursor_id_is_the_verbatim_remainder() {
        let id_cursor = ReindexWalkCursor::Id {
            floor: ts("2026-01-01T00:00:00.000Z"),
            after_id: "a|b".to_string(),
        };
        assert_eq!(
            ReindexWalkCursor::parse(&id_cursor.encode()).unwrap(),
            id_cursor
        );

        let round_cursor = ReindexWalkCursor::Round {
            round: 1,
            floor: ts("2026-01-01T00:00:00.000Z"),
            ceiling: ts("2026-01-01T00:02:00.000Z"),
            walked: 3,
            after_last_updated: ts("2026-01-01T00:01:00.000Z"),
            after_id: "a|b".to_string(),
        };
        assert_eq!(
            ReindexWalkCursor::parse(&round_cursor.encode()).unwrap(),
            round_cursor
        );
    }

    #[test]
    fn cursor_rejects_foreign_and_malformed_tokens() {
        let instant = "2026-01-01T00:00:00.000Z";
        let ceiling = "2026-01-01T00:02:00.000Z";
        let bad: Vec<String> = vec![
            "".to_string(),
            "2026-09-19T04:43:29.668+00:00|e357ce58-f379-216d-a369-99da40ff76ae".to_string(),
            format!("v1|i|{instant}|a"),
            format!("v3|i|{instant}|a"),
            format!("v2|x|{instant}|a"),
            format!("v2|s|1|{instant}"),
            format!("v2|i|{instant}|"),
            "v2|i|not-a-time|a".to_string(),
            // round 0 (below the 1..=MAX range)
            format!("v2|c|0|{instant}|{ceiling}|0|{instant}|a"),
            // round MAX + 1 (above the range)
            format!(
                "v2|c|{}|{instant}|{ceiling}|0|{instant}|a",
                REINDEX_CATCH_UP_MAX_ROUNDS + 1
            ),
            // five fields instead of six (missing after_lu)
            format!("v2|c|1|{instant}|{ceiling}|0|a"),
            // walked = -1
            format!("v2|c|1|{instant}|{ceiling}|-1|{instant}|a"),
            // floor == ceiling
            format!("v2|c|1|{instant}|{instant}|0|{instant}|a"),
            // after_lu < floor
            format!("v2|c|1|{instant}|{ceiling}|0|2025-12-31T23:59:59.000Z|a"),
            // after_lu == ceiling
            format!("v2|c|1|{instant}|{ceiling}|0|{ceiling}|a"),
        ];
        for cursor in bad {
            match ReindexWalkCursor::parse(&cursor) {
                Err(StorageError::Search(SearchError::InvalidCursor { .. })) => {}
                other => panic!("expected InvalidCursor for {cursor:?}, got {other:?}"),
            }
        }
    }

    // --- Floor / ceiling / margin / round decision ---

    #[test]
    fn floor_is_newest_plus_one_ms_for_old_data() {
        let t0 = ts("2026-01-01T01:00:00.000Z");
        let newest = ts("2026-01-01T00:00:00.000Z"); // far older than t0 - margin
        let margin = chrono::Duration::seconds(120);
        assert_eq!(
            reindex_catch_up_floor(t0, Some(newest), margin),
            newest + chrono::Duration::milliseconds(1)
        );
    }

    #[test]
    fn floor_is_t0_minus_margin_for_fresh_data() {
        let t0 = ts("2026-01-01T01:00:00.000Z");
        let newest = t0 - chrono::Duration::seconds(1); // inside the margin
        let margin = chrono::Duration::seconds(120);
        assert_eq!(
            reindex_catch_up_floor(t0, Some(newest), margin),
            t0 - margin
        );
    }

    #[test]
    fn floor_without_live_resources_is_t0_minus_margin() {
        let t0 = ts("2026-01-01T01:00:00.000Z");
        let margin = chrono::Duration::seconds(120);
        assert_eq!(reindex_catch_up_floor(t0, None, margin), t0 - margin);
    }

    #[test]
    fn floor_truncates_to_milliseconds() {
        let t0 = Utc::now(); // sub-millisecond precision on most platforms
        let margin = chrono::Duration::seconds(120);
        let floor = reindex_catch_up_floor(t0, None, margin);
        assert_eq!(floor.timestamp_subsec_nanos() % 1_000_000, 0);
    }

    #[test]
    fn ceiling_is_now_plus_margin_for_past_stamps() {
        let now = ts("2026-01-01T01:00:00.000Z");
        let margin = chrono::Duration::seconds(120);
        assert_eq!(
            reindex_catch_up_ceiling(now, Some(now - chrono::Duration::seconds(1)), margin),
            now + margin
        );
        assert_eq!(reindex_catch_up_ceiling(now, None, margin), now + margin);
    }

    #[test]
    fn ceiling_passes_a_future_stamp() {
        let now = ts("2026-01-01T01:00:00.000Z");
        let margin = chrono::Duration::seconds(120);
        let newest = now + margin + chrono::Duration::seconds(5);
        assert_eq!(
            reindex_catch_up_ceiling(now, Some(newest), margin),
            newest + chrono::Duration::milliseconds(1)
        );
    }

    #[test]
    fn round_start_decision_round_one_always_runs() {
        let floor = ts("2026-01-01T00:00:00.000Z");
        let margin = chrono::Duration::seconds(120);
        assert_eq!(
            reindex_round_start_decision(1, floor, floor - chrono::Duration::hours(1), margin),
            RoundStartDecision::Run
        );
        assert_eq!(
            reindex_round_start_decision(1, floor, floor + chrono::Duration::hours(1), margin),
            RoundStartDecision::Run
        );
    }

    #[test]
    fn round_start_decision_round_two_completes_or_runs_at_the_half_margin_boundary() {
        let floor = ts("2026-01-01T00:02:00.000Z");
        let margin = chrono::Duration::seconds(120);
        let boundary = floor - margin / 2;
        assert_eq!(
            reindex_round_start_decision(2, floor, boundary, margin),
            RoundStartDecision::Run
        );
        assert_eq!(
            reindex_round_start_decision(
                2,
                floor,
                boundary - chrono::Duration::milliseconds(1),
                margin
            ),
            RoundStartDecision::Complete
        );
    }

    #[test]
    fn round_start_decision_caps_or_completes_past_the_round_limit() {
        let floor = ts("2026-01-01T00:02:00.000Z");
        let margin = chrono::Duration::seconds(120);
        let boundary = floor - margin / 2;
        let round = REINDEX_CATCH_UP_MAX_ROUNDS + 1;
        assert_eq!(
            reindex_round_start_decision(round, floor, boundary, margin),
            RoundStartDecision::CapReached
        );
        assert_eq!(
            reindex_round_start_decision(
                round,
                floor,
                boundary - chrono::Duration::milliseconds(1),
                margin
            ),
            RoundStartDecision::Complete
        );
    }

    #[test]
    fn margin_is_clamped() {
        assert_eq!(reindex_catch_up_margin(0), chrono::Duration::seconds(1));
        assert_eq!(
            reindex_catch_up_margin(120_000),
            chrono::Duration::seconds(120)
        );
        assert_eq!(
            reindex_catch_up_margin(u64::MAX),
            chrono::Duration::hours(24)
        );
    }

    // --- Filter shapes ---

    #[test]
    fn id_page_filter_shape() {
        let floor = ts("2026-01-01T00:00:00.000Z");
        let first = reindex_id_page_filter("t1", "Observation", floor, None, None);
        assert!(!first.contains_key("id"));
        // Tenant/type scope: a regression here (e.g. PR2a/PR2b's
        // `reindex_find_page` refactor dropping a clause) would let the walk
        // read another tenant's or resource type's rows undetected by any
        // Docker-gated test (#1403 review finding).
        assert_eq!(first.get_str("tenant_id"), Ok("t1"));
        assert_eq!(first.get_str("resource_type"), Ok("Observation"));
        assert_eq!(first.get_bool("is_deleted"), Ok(false));
        assert_eq!(
            first.get_document("last_updated").unwrap().get("$lt"),
            Some(&Bson::from(chrono_to_bson(floor)))
        );

        let later = reindex_id_page_filter("t1", "Observation", floor, Some("obs-010"), None);
        assert_eq!(later.get_str("tenant_id"), Ok("t1"));
        assert_eq!(later.get_str("resource_type"), Ok("Observation"));
        assert_eq!(later.get_bool("is_deleted"), Ok(false));
        assert_eq!(
            later.get_document("last_updated").unwrap().get("$lt"),
            Some(&Bson::from(chrono_to_bson(floor)))
        );
        assert_eq!(
            later.get_document("id").unwrap().get_str("$gt"),
            Ok("obs-010")
        );
    }

    #[test]
    fn catch_up_filter_shape() {
        let floor = ts("2026-01-01T00:00:00.000Z");
        let ceiling = ts("2026-01-01T00:02:00.000Z");
        let first = reindex_catch_up_page_filter("t1", "Observation", floor, ceiling, None, None);
        assert!(!first.contains_key("$or"));
        // Tenant/type scope and the deleted-row exclusion: nothing else would
        // catch either clause silently dropping from the catch-up filter
        // (#1403 review finding) — the integration tests can't distinguish a
        // scoped catch-up round from an unscoped one that happens to see the
        // same rows.
        assert_eq!(first.get_str("tenant_id"), Ok("t1"));
        assert_eq!(first.get_str("resource_type"), Ok("Observation"));
        assert_eq!(first.get_bool("is_deleted"), Ok(false));
        let range = first.get_document("last_updated").unwrap();
        assert_eq!(range.get("$gte"), Some(&Bson::from(chrono_to_bson(floor))));
        assert_eq!(range.get("$lt"), Some(&Bson::from(chrono_to_bson(ceiling))));

        let after_lu = ts("2026-01-01T00:01:00.000Z");
        let continuation = reindex_catch_up_page_filter(
            "t1",
            "Observation",
            floor,
            ceiling,
            Some((after_lu, "obs-020")),
            None,
        );
        assert_eq!(continuation.get_str("tenant_id"), Ok("t1"));
        assert_eq!(continuation.get_str("resource_type"), Ok("Observation"));
        assert_eq!(continuation.get_bool("is_deleted"), Ok(false));
        assert!(!continuation.contains_key("last_updated"));
        let or = continuation.get_array("$or").unwrap();
        assert_eq!(or.len(), 2);
        let first_arm_doc = or[0].as_document().unwrap();
        assert_eq!(
            first_arm_doc.len(),
            1,
            "arm 0 must hold only `last_updated`: {first_arm_doc:?}"
        );
        let first_arm = first_arm_doc.get_document("last_updated").unwrap();
        assert_eq!(
            first_arm.get("$gt"),
            Some(&Bson::from(chrono_to_bson(after_lu)))
        );
        assert_eq!(
            first_arm.get("$lt"),
            Some(&Bson::from(chrono_to_bson(ceiling)))
        );
        let second_arm = or[1].as_document().unwrap();
        assert_eq!(
            second_arm.len(),
            2,
            "arm 1 must hold exactly `last_updated` and `id`: {second_arm:?}"
        );
        assert_eq!(
            second_arm.get("last_updated"),
            Some(&Bson::from(chrono_to_bson(after_lu)))
        );
        assert_eq!(
            second_arm.get_document("id").unwrap().get_str("$gt"),
            Ok("obs-020")
        );
    }

    #[test]
    fn ranged_filters_bound_the_id_keyset_and_the_catch_up_round() {
        let floor = ts("2026-01-01T00:00:00.000Z");
        let ceiling = ts("2026-01-01T00:02:00.000Z");
        let range = ReindexIdRange {
            start: Some("4".to_string()),
            end: Some("8".to_string()),
        };

        let first = reindex_id_page_filter("t1", "Observation", floor, None, Some(&range));
        let id = first.get_document("id").unwrap();
        assert_eq!(id.get_str("$gte"), Ok("4"));
        assert_eq!(id.get_str("$lt"), Ok("8"));
        assert!(!id.contains_key("$gt"));

        let later = reindex_id_page_filter("t1", "Observation", floor, Some("5"), Some(&range));
        let id = later.get_document("id").unwrap();
        assert_eq!(id.get_str("$gt"), Ok("5"));
        assert_eq!(id.get_str("$gte"), Ok("4"));
        assert_eq!(id.get_str("$lt"), Ok("8"));

        let open_end = ReindexIdRange {
            start: Some("c".to_string()),
            end: None,
        };
        let round = reindex_catch_up_page_filter(
            "t1",
            "Observation",
            floor,
            ceiling,
            Some((floor, "d")),
            Some(&open_end),
        );
        let id = round.get_document("id").unwrap();
        assert_eq!(id.get_str("$gte"), Ok("c"));
        assert!(!id.contains_key("$lt"));
        assert_eq!(round.get_array("$or").unwrap().len(), 2);

        let unranged =
            reindex_catch_up_page_filter("t1", "Observation", floor, ceiling, None, None);
        assert!(!unranged.contains_key("id"));
    }

    // --- Dedupe ---

    #[test]
    fn dedupe_keeps_the_last_occurrence_in_scan_order() {
        let docs = vec![
            doc! { "id": "a", "v": 1 },
            doc! { "note": "no id" },
            doc! { "id": "b", "v": 1 },
            doc! { "id": "a", "v": 2 },
        ];
        let deduped = dedupe_reindex_page_keep_last(docs);
        assert_eq!(deduped.len(), 3);
        assert!(!deduped[0].contains_key("id")); // "no id" doc kept in place
        assert_eq!(deduped[0].get_str("note"), Ok("no id"));
        assert_eq!(deduped[1].get_str("id"), Ok("b"));
        assert_eq!(deduped[2].get_str("id"), Ok("a"));
        assert_eq!(deduped[2].get_i32("v"), Ok(2));
    }
}

#[cfg(test)]
mod reindex_page_cap_tests {
    use super::*;

    #[test]
    fn admits_the_first_row_whatever_its_size() {
        assert!(reindex_page_admits(0, 0, 10_000, 1));
    }

    #[test]
    fn admits_up_to_and_including_the_cap() {
        assert!(reindex_page_admits(1, 100, 50, 150));
        assert!(!reindex_page_admits(1, 100, 50, 149));
    }

    #[test]
    fn zero_cap_admits_everything() {
        assert!(reindex_page_admits(7, u64::MAX, u64::MAX, 0));
    }

    #[test]
    fn saturating_sum_does_not_overflow() {
        assert!(reindex_page_admits(1, u64::MAX - 1, 10, u64::MAX));
    }
}

#[cfg(test)]
mod reindex_prefetch_tests {
    use super::*;
    use crate::backends::mongodb::MongoBackendConfig;
    use crate::search::reindex::ReindexSource;
    use crate::tenant::{TenantId, TenantPermissions};

    /// A config that can never reach a real server. `MongoBackendConfig::default()`'s
    /// connection string is `mongodb://localhost:27017`, which is a long-lived
    /// corpus container that must never be touched by a unit test — so every
    /// backend built in this module uses this instead, even where the code
    /// path never actually calls the database today, in case a future change
    /// moves a database call earlier (#1403).
    fn unreachable_config() -> MongoBackendConfig {
        MongoBackendConfig {
            connection_string: "mongodb://127.0.0.1:1".to_string(),
            server_selection_timeout_ms: 500,
            ..Default::default()
        }
    }

    fn id_cursor() -> String {
        ReindexWalkCursor::Id {
            floor: chrono::Utc::now(),
            after_id: "p1".to_string(),
        }
        .encode()
    }

    fn round_cursor() -> String {
        let t = chrono::Utc::now();
        ReindexWalkCursor::Round {
            round: 1,
            floor: t,
            ceiling: t + chrono::Duration::seconds(1),
            walked: 0,
            after_last_updated: t,
            after_id: "p1".to_string(),
        }
        .encode()
    }

    #[test]
    fn an_id_range_view_prefetches_as_the_backend_does() {
        let backend =
            std::sync::Arc::new(MongoBackend::new(unreachable_config()).expect("lazy client"));
        let ranged = backend
            .with_id_range(ReindexIdRange {
                start: Some("4".to_string()),
                end: None,
            })
            .expect("MongoDB supports id ranges");
        assert!(ranged.may_prefetch_page(&id_cursor()));
        assert!(!ranged.may_prefetch_page(&round_cursor()));
        assert!(!ranged.may_prefetch_page("garbage"));
    }

    #[test]
    fn may_prefetch_page_accepts_only_id_cursors() {
        assert!(matches!(
            ReindexWalkCursor::parse(&round_cursor()),
            Ok(ReindexWalkCursor::Round { .. })
        ));
        let backend = MongoBackend::new(unreachable_config()).expect("lazy client");
        assert!(backend.may_prefetch_page(&id_cursor()));
        assert!(!backend.may_prefetch_page(&round_cursor()));
        assert!(!backend.may_prefetch_page("garbage"));

        let no_prefetch = MongoBackend::new(MongoBackendConfig {
            reindex_prefetch: false,
            ..unreachable_config()
        })
        .expect("lazy client");
        assert!(!no_prefetch.may_prefetch_page(&id_cursor()));

        // A composite deployment (search offloaded to Elasticsearch) prefetches
        // too: the id-phase read touches only `resources`, which the
        // Elasticsearch writer never does (#1250).
        let offloaded = MongoBackend::new(MongoBackendConfig {
            search_offloaded: true,
            ..unreachable_config()
        })
        .expect("lazy client");
        assert!(offloaded.may_prefetch_page(&id_cursor()));
        assert!(!offloaded.may_prefetch_page(&round_cursor()));
    }

    #[tokio::test]
    async fn fetch_ahead_declines_round_and_malformed_cursors() {
        // Both cases return before any database call: the Round cursor parses
        // but does not match `Id`, and the malformed cursor fails to parse,
        // before `get_database` is ever reached, so `unreachable_config`'s
        // bogus connection string is exercised only as a defensive
        // belt-and-suspenders, not because either case connects.
        assert!(matches!(
            ReindexWalkCursor::parse(&round_cursor()),
            Ok(ReindexWalkCursor::Round { .. })
        ));
        let backend = MongoBackend::new(unreachable_config()).expect("lazy client");
        let tenant = TenantContext::new(
            TenantId::new("prefetch-test-tenant"),
            TenantPermissions::full_access(),
        );

        let result = backend
            .fetch_resources_page_ahead(&tenant, "Patient", &round_cursor(), 10, 0)
            .await
            .expect("no database error");
        assert!(
            result.is_none(),
            "a Round cursor must never be fetched ahead"
        );

        let result = backend
            .fetch_resources_page_ahead(&tenant, "Patient", "garbage", 10, 0)
            .await
            .expect("no database error");
        assert!(
            result.is_none(),
            "a cursor that fails to parse must never be fetched ahead"
        );
    }
}

/// #1586: a bundle transaction is replayed only for an error the server
/// labelled as a transient abort, and only when the failing site kept the driver
/// error. These pin the classifier and the source-keeping constructor without a
/// server.
#[cfg(test)]
mod transient_abort_tests {
    use super::*;

    /// A driver error for a server-side command failure with `code`, as the
    /// driver builds it from an error reply.
    fn command_error(code: i32, code_name: &str) -> MongoError {
        let command: mongodb::error::CommandError = bson::from_document(doc! {
            "code": code,
            "codeName": code_name,
            "errmsg": "Failing command via 'failCommand' failpoint",
        })
        .expect("a CommandError deserializes from a server error reply");
        MongoError::from(MongoErrorKind::Command(command))
    }

    /// The search-index insert's shape from the #1837 benchmark log: an
    /// `insert_many` whose per-document error carries `code`, as the driver
    /// builds it from a reply with `writeErrors` (no `codeName`).
    fn insert_many_write_error(code: i32) -> MongoError {
        let insert_many: mongodb::error::InsertManyError = bson::from_document(doc! {
            "writeErrors": [{
                "index": 0,
                "code": code,
                "errmsg": "WiredTigerRecordStore::insertRecord -31800: transaction is too large and will not fit in the storage engine cache",
            }],
        })
        .expect("an InsertManyError deserializes from a server reply");
        MongoError::from(MongoErrorKind::InsertMany(insert_many))
    }

    /// A `TransactionTooLargeForCache` (388) the way a single write reports it:
    /// as a per-write error rather than a failed command.
    fn too_large_write_error() -> MongoError {
        let write: mongodb::error::WriteError = bson::from_document(doc! {
            "code": TRANSACTION_TOO_LARGE_FOR_CACHE_CODE,
            "codeName": "TransactionTooLargeForCache",
            "errmsg": "WiredTigerRecordStore::insertRecord -31800: transaction is too large and will not fit in the storage engine cache",
        })
        .expect("a WriteError deserializes from a server reply");
        MongoError::from(MongoErrorKind::Write(
            mongodb::error::WriteFailure::WriteError(write),
        ))
    }

    #[test]
    fn a_write_conflict_that_kept_its_driver_error_is_a_transient_abort() {
        let err = internal_driver_error(
            "Failed to insert resource in transaction",
            command_error(WRITE_CONFLICT_CODE, "WriteConflict"),
        );
        let driver = transient_transaction_abort(&err).expect("a write conflict is transient");
        assert_eq!(driver_error_code(driver), Some(WRITE_CONFLICT_CODE));
    }

    #[test]
    fn a_driver_error_kept_by_the_search_paths_is_found_too() {
        // `or_query_error` / `From<MongoError>` classify through
        // `classify_mongodb_error`, which keeps the source for everything that
        // is not a timeout or an unreachable server.
        let err = StorageError::from(command_error(WRITE_CONFLICT_CODE, "WriteConflict"));
        assert!(transient_transaction_abort(&err).is_some());
    }

    #[test]
    fn other_driver_errors_are_not_transient_aborts() {
        // BadValue: a real failure, never replayed.
        let err = internal_driver_error("Failed to insert", command_error(2, "BadValue"));
        assert!(transient_transaction_abort(&err).is_none());
        // Not even a server error.
        let err = internal_driver_error("Failed to insert", MongoError::custom("boom"));
        assert!(transient_transaction_abort(&err).is_none());
    }

    #[test]
    fn a_flattened_error_is_invisible_to_the_classifier() {
        // The pre-#1586 spelling: same text, no source. This is why every
        // session-scoped site now uses `internal_driver_error`.
        let flattened = internal_error(format!(
            "Failed to insert resource in transaction: {}",
            command_error(WRITE_CONFLICT_CODE, "WriteConflict")
        ));
        assert!(transient_transaction_abort(&flattened).is_none());
    }

    #[test]
    fn errors_that_are_not_backend_errors_are_not_transient_aborts() {
        let err = StorageError::Resource(ResourceError::AlreadyExists {
            resource_type: "Patient".to_string(),
            id: "1".to_string(),
        });
        assert!(transient_transaction_abort(&err).is_none());
    }

    /// An `Io`-kind driver error as the session-scoped `or_query_error` sites
    /// (the composite `ifNoneExist` probes) classify it: through
    /// `classify_mongodb_error` into `BackendError::Unavailable`, which keeps no
    /// source.
    fn classified_io_error() -> StorageError {
        StorageError::from(MongoError::from(std::io::Error::from(
            std::io::ErrorKind::ConnectionReset,
        )))
    }

    #[test]
    fn a_classified_network_error_has_no_driver_error_to_find() {
        // The gap the bundle loop closes: `classify_mongodb_error` turns an
        // `Io` error into `Unavailable` and drops the driver error with its
        // labels, so the source-walking finder cannot see it.
        let err = classified_io_error();
        assert!(matches!(
            err,
            StorageError::Backend(BackendError::Unavailable { .. })
        ));
        assert!(transient_transaction_abort(&err).is_none());
    }

    #[test]
    fn the_bundle_classifier_treats_unavailable_as_a_transient_entry_abort() {
        // A transport failure inside the transaction: the driver itself
        // labels it TransientTransactionError, and the transaction is gone.
        let abort = transient_entry_abort(&classified_io_error())
            .expect("a network error mid-transaction is a transient abort");
        assert_eq!(abort.code, None, "a transport error carries no server code");
        assert!(
            abort.reason.contains("Entry processing failed"),
            "{}",
            abort.reason
        );

        // The reason keeps the detail `Unavailable`'s own Display drops.
        assert!(
            abort.reason.contains("connection reset") || abort.reason.contains("Connection reset"),
            "{}",
            abort.reason
        );

        // Whatever `classify_mongodb_error` maps to `Unavailable` reads the same
        // way: a `BackendError::Unavailable` built directly (the ServerSelection
        // and ConnectionPoolCleared kinds cannot be constructed outside the
        // driver, but they take this same arm).
        let unavailable = StorageError::Backend(BackendError::Unavailable {
            backend_name: "mongodb".to_string(),
            message: "no suitable server".to_string(),
        });
        let abort = transient_entry_abort(&unavailable).expect("Unavailable is transient");
        assert!(
            abort.reason.contains("no suitable server"),
            "{}",
            abort.reason
        );
    }

    #[test]
    fn the_bundle_classifier_keeps_a_labelled_driver_error_and_its_code() {
        let err = internal_driver_error(
            "Failed to insert resource in transaction",
            command_error(WRITE_CONFLICT_CODE, "WriteConflict"),
        );
        let abort = transient_entry_abort(&err).expect("a write conflict is transient");
        assert_eq!(abort.code, Some(WRITE_CONFLICT_CODE));
        assert!(abort.reason.contains("Entry processing failed"));
    }

    /// A `TemporarilyUnavailable` (365) the way a write inside the transaction
    /// reports it: as a per-write error rather than a failed command.
    fn temporarily_unavailable_write_error() -> MongoError {
        let write: mongodb::error::WriteError = bson::from_document(doc! {
            "code": 365,
            "codeName": "TemporarilyUnavailable",
            "errmsg": "oldest pinned transaction ID rolled back for eviction",
        })
        .expect("a WriteError deserializes from a server reply");
        MongoError::from(MongoErrorKind::Write(
            mongodb::error::WriteFailure::WriteError(write),
        ))
    }

    #[test]
    fn the_bundle_classifier_treats_an_unlabelled_code_365_entry_error_as_transient() {
        // #1700: the server's own label is not the only way this arrives, and
        // an entry failing with it did not and will not commit.
        for driver in [
            command_error(365, "TemporarilyUnavailable"),
            temporarily_unavailable_write_error(),
        ] {
            assert!(driver.labels().is_empty());
            let err = internal_driver_error("Failed to insert resource in transaction", driver);
            let abort = transient_entry_abort(&err).expect("code 365 is a transient abort");
            assert_eq!(abort.code, Some(365));
            assert!(abort.reason.contains("Entry processing failed"));
        }
        // And as the session-scoped search paths keep it.
        let err = StorageError::from(command_error(365, "TemporarilyUnavailable"));
        assert!(transient_transaction_abort(&err).is_some());
        // The single-resource write paths' conflict check is left as it was.
        assert!(!is_write_conflict(&temporarily_unavailable()));
    }

    #[test]
    fn the_bundle_classifier_still_does_not_retry_real_failures() {
        // BadValue, an unlabelled server error.
        let bad_value = internal_driver_error("Failed to insert", command_error(2, "BadValue"));
        assert!(transient_entry_abort(&bad_value).is_none());
        // A flattened error.
        assert!(transient_entry_abort(&internal_error("boom".to_string())).is_none());
        // A timeout is the server stopping work at its deadline, not a drop.
        let timeout = StorageError::Backend(BackendError::Timeout {
            backend_name: "mongodb".to_string(),
            message: "deadline".to_string(),
        });
        assert!(transient_entry_abort(&timeout).is_none());
        // Not a backend error at all.
        let exists = StorageError::Resource(ResourceError::AlreadyExists {
            resource_type: "Patient".to_string(),
            id: "1".to_string(),
        });
        assert!(transient_entry_abort(&exists).is_none());
    }

    #[test]
    fn code_388_from_any_bundle_write_is_too_large_for_cache_and_not_replayed() {
        for driver in [
            command_error(388, "TransactionTooLargeForCache"),
            too_large_write_error(),
            insert_many_write_error(388),
        ] {
            assert!(is_too_large_for_cache(&driver), "{driver}");
            assert!(
                !super::super::retry::is_transient_mongo_error(&driver),
                "{driver}"
            );
            let err = internal_driver_error("Failed to insert search index entries", driver);
            match entry_attempt_error(6959, 7412, &err) {
                BundleAttemptError::Failed(TransactionError::TooLargeForCache {
                    entries,
                    reason,
                }) => {
                    assert_eq!(entries, 7412);
                    assert!(reason.contains("388"), "{reason}");
                    assert!(reason.contains("6959"), "{reason}");
                }
                other => panic!("expected TooLargeForCache, got {other:?}"),
            }
        }
        // The path that keeps the source through `classify_mongodb_error`.
        let err = StorageError::from(command_error(388, "TransactionTooLargeForCache"));
        assert!(matches!(
            entry_attempt_error(0, 2, &err),
            BundleAttemptError::Failed(TransactionError::TooLargeForCache { entries: 2, .. })
        ));
    }

    #[test]
    fn other_codes_are_not_too_large_for_cache() {
        for driver in [
            command_error(WRITE_CONFLICT_CODE, "WriteConflict"),
            command_error(365, "TemporarilyUnavailable"),
            command_error(2, "BadValue"),
            insert_many_write_error(11000),
            MongoError::custom("boom"),
        ] {
            assert!(!is_too_large_for_cache(&driver), "{driver}");
        }
    }

    #[test]
    fn the_entry_classifier_keeps_its_other_verdicts_alongside_too_large_for_cache() {
        // A kept 112 is still a transient abort.
        let err = internal_driver_error(
            "Failed to insert resource in transaction",
            command_error(WRITE_CONFLICT_CODE, "WriteConflict"),
        );
        assert!(matches!(
            entry_attempt_error(3, 10, &err),
            BundleAttemptError::TransientAbort {
                entry: Some(3),
                code: Some(WRITE_CONFLICT_CODE),
                ..
            }
        ));
        // A real failure still blames the entry.
        let err = internal_driver_error("Failed to insert", command_error(2, "BadValue"));
        match entry_attempt_error(3, 10, &err) {
            BundleAttemptError::Failed(TransactionError::BundleError { index, message }) => {
                assert_eq!(index, 3);
                assert!(
                    message.starts_with("Entry processing failed: "),
                    "{message}"
                );
            }
            other => panic!("expected BundleError, got {other:?}"),
        }
        // A per-document error of another code is not a cache failure.
        let err = internal_driver_error(
            "Failed to insert search index entries",
            insert_many_write_error(11000),
        );
        assert!(matches!(
            entry_attempt_error(3, 10, &err),
            BundleAttemptError::Failed(TransactionError::BundleError { .. })
        ));
        // The text alone is not the code: a flattened error stays a BundleError.
        let err = internal_error(format!(
            "Failed to insert search index entries: {}",
            insert_many_write_error(388)
        ));
        assert!(matches!(
            entry_attempt_error(3, 10, &err),
            BundleAttemptError::Failed(TransactionError::BundleError { .. })
        ));
        // A classified Unavailable is still a transient abort.
        assert!(matches!(
            entry_attempt_error(3, 10, &classified_io_error()),
            BundleAttemptError::TransientAbort { .. }
        ));
    }

    /// A commit that failed with a write-concern error of `code`, as the driver
    /// builds it from a reply that carries `writeConcernError`. No label: the
    /// driver adds `UnknownTransactionCommitResult` only for codes
    /// `{50, 64, 91}` and the retryable-write codes.
    fn write_concern_error(code: i32, code_name: &str) -> MongoError {
        let concern: mongodb::error::WriteConcernError = bson::from_document(doc! {
            "code": code,
            "codeName": code_name,
            "errmsg": "write concern not satisfied",
        })
        .expect("a WriteConcernError deserializes from a server reply");
        MongoError::from(MongoErrorKind::Write(
            mongodb::error::WriteFailure::WriteConcernError(concern),
        ))
    }

    #[test]
    fn a_write_concern_error_is_an_unknown_commit_result_whatever_its_label() {
        // Code 79 is outside the driver's label set, so it carries no label.
        let err = write_concern_error(79, "UnknownReplWriteConcern");
        assert!(!err.contains_label(UNKNOWN_TRANSACTION_COMMIT_RESULT));
        assert!(!err.contains_label(TRANSIENT_TRANSACTION_ERROR));
        assert_eq!(
            classify_commit_failure(&err, false),
            CommitFailure::UnknownResult
        );
        // The same error is just as unknown on a later commit.
        assert_eq!(
            classify_commit_failure(&err, true),
            CommitFailure::UnknownResult
        );
        // A write-concern error outranks every label (pure form: a
        // transient-labelled one cannot be built through the driver's API).
        for earlier in [false, true] {
            for transient in [false, true] {
                for unknown in [false, true] {
                    assert_eq!(
                        classify_commit_signals(transient, unknown, false, true, earlier),
                        CommitFailure::UnknownResult,
                        "transient={transient} unknown={unknown} earlier={earlier}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_commit_failure_is_classified_by_its_labels_and_history() {
        use CommitFailure::*;
        // (transient, unknown-label, earlier-unknown) -> verdict, no write concern.
        let cases = [
            ((false, false, false), RolledBack),
            ((true, false, false), TransientAbort),
            ((false, true, false), UnknownResult),
            // Both labels never co-occur from the driver; the abort wins.
            ((true, true, false), TransientAbort),
            // After an unknown commit nothing may be replayed or called rolled back.
            ((false, false, true), StillUnknown),
            ((true, false, true), StillUnknown),
            ((true, true, true), StillUnknown),
            // ...but another unknown result is still just a commit to retry.
            ((false, true, true), UnknownResult),
        ];
        for ((transient, unknown, earlier), expected) in cases {
            assert_eq!(
                classify_commit_signals(transient, unknown, false, false, earlier),
                expected,
                "transient={transient} unknown={unknown} earlier={earlier}"
            );
        }
    }

    /// #1700: code 365 is read as a transient abort only where no stronger
    /// signal says otherwise.
    #[test]
    fn code_365_is_a_transient_abort_unless_a_stronger_signal_says_otherwise() {
        use CommitFailure::*;
        // (unknown-label, write-concern, earlier-unknown) -> verdict, with 365.
        let cases = [
            ((false, false, false), TransientAbort),
            ((false, false, true), StillUnknown),
            // An explicit unknown-result label keeps the commit-only path...
            ((true, false, false), UnknownResult),
            ((true, false, true), UnknownResult),
            // ...and so does a write-concern error, which always applied.
            ((false, true, false), UnknownResult),
            ((false, true, true), UnknownResult),
            ((true, true, false), UnknownResult),
        ];
        for ((unknown, write_concern, earlier), expected) in cases {
            assert_eq!(
                classify_commit_signals(false, unknown, true, write_concern, earlier),
                expected,
                "unknown={unknown} write_concern={write_concern} earlier={earlier}"
            );
        }
    }

    #[test]
    fn a_plain_commit_failure_is_a_rollback_only_while_no_commit_is_unknown() {
        let interrupted = command_error(11601, "Interrupted");
        assert_eq!(
            classify_commit_failure(&interrupted, false),
            CommitFailure::RolledBack
        );
        assert_eq!(
            classify_commit_failure(&interrupted, true),
            CommitFailure::StillUnknown
        );
        let client_side = MongoError::custom("boom");
        assert_eq!(
            classify_commit_failure(&client_side, false),
            CommitFailure::RolledBack
        );
        assert_eq!(
            classify_commit_failure(&client_side, true),
            CommitFailure::StillUnknown
        );
    }

    /// #1700: the eviction rollback the 0.2.4 benchmark saw at commit, "oldest
    /// pinned transaction ID rolled back for eviction": code 365
    /// (`TemporarilyUnavailable`) and no `TransientTransactionError` label. The
    /// literal is deliberate; it pins the number the server sends.
    fn temporarily_unavailable() -> MongoError {
        let err = command_error(365, "TemporarilyUnavailable");
        assert!(err.labels().is_empty(), "the server sent no label");
        err
    }

    #[test]
    fn an_unlabelled_commit_failure_with_code_365_is_a_transient_abort() {
        let err = temporarily_unavailable();
        assert_eq!(
            classify_commit_failure(&err, false),
            CommitFailure::TransientAbort
        );
        // Once a commit's outcome is unknown, a replay could store the bundle
        // twice: it stays unknown, as for any transient-labelled failure.
        assert_eq!(
            classify_commit_failure(&err, true),
            CommitFailure::StillUnknown
        );
    }

    #[test]
    fn the_commit_loop_hands_a_code_365_commit_failure_back_for_a_replay() {
        let steps = drive_commit(vec![temporarily_unavailable()]);
        assert_eq!(steps.len(), 1);
        match stopped_with(&steps[0]) {
            BundleAttemptError::TransientAbort {
                entry: None,
                code: Some(365),
                reason,
            } => assert!(reason.starts_with("Commit failed: "), "{reason}"),
            other => panic!("expected a transient abort at commit, got {other:?}"),
        }
    }

    #[test]
    fn a_code_365_commit_failure_after_an_unknown_commit_is_not_replayed() {
        let steps = drive_commit(vec![
            write_concern_error(79, "UnknownReplWriteConcern"),
            temporarily_unavailable(),
        ]);
        assert_eq!(steps.len(), 2);
        assert!(matches!(steps[0], CommitStep::RetryCommit));
        match stopped_with(&steps[1]) {
            BundleAttemptError::Failed(TransactionError::CommitOutcomeUnknown { .. }) => {}
            other => panic!("expected CommitOutcomeUnknown, got {other:?}"),
        }
    }

    /// Runs `failures` through a fresh [`CommitAttempts`], as the commit loop
    /// does, and returns what it decided at each one.
    fn drive_commit(failures: Vec<MongoError>) -> Vec<CommitStep> {
        let mut attempts = CommitAttempts::new();
        let mut steps = Vec::new();
        for failure in failures {
            let step = attempts.after_failure(&failure);
            let stop = matches!(step, CommitStep::Stop(_));
            steps.push(step);
            if stop {
                break;
            }
        }
        steps
    }

    fn stopped_with(step: &CommitStep) -> &BundleAttemptError {
        match step {
            CommitStep::Stop(error) => error,
            CommitStep::RetryCommit => panic!("expected the commit loop to stop"),
        }
    }

    #[test]
    fn a_commit_that_stays_unknown_is_retried_twice_then_reported_unknown() {
        let wc = || write_concern_error(79, "UnknownReplWriteConcern");
        let steps = drive_commit(vec![wc(), wc(), wc()]);
        assert_eq!(steps.len(), 3);
        assert!(matches!(steps[0], CommitStep::RetryCommit));
        assert!(matches!(steps[1], CommitStep::RetryCommit));
        match stopped_with(&steps[2]) {
            BundleAttemptError::Failed(TransactionError::CommitOutcomeUnknown { reason }) => {
                assert!(reason.contains("3 attempts"), "{reason}");
            }
            other => panic!("expected CommitOutcomeUnknown, got {other:?}"),
        }
    }

    /// The first commit's outcome is unknown; the retry then fails some other
    /// way (the server interrupted it, or the client could not send it). The
    /// first may still have applied, so the answer is `CommitOutcomeUnknown` —
    /// the false `RolledBack` this used to give told a client whose bundle was
    /// stored that nothing was applied.
    #[test]
    fn a_later_non_transient_commit_failure_stays_unknown_not_rolled_back() {
        let later_failures = [
            command_error(11601, "Interrupted"),
            MongoError::custom("client-side failure"),
            MongoError::from(std::io::Error::from(std::io::ErrorKind::ConnectionReset)),
        ];
        for later in later_failures {
            let steps = drive_commit(vec![
                write_concern_error(79, "UnknownReplWriteConcern"),
                later,
            ]);
            assert_eq!(steps.len(), 2);
            assert!(matches!(steps[0], CommitStep::RetryCommit));
            match stopped_with(&steps[1]) {
                BundleAttemptError::Failed(TransactionError::CommitOutcomeUnknown { reason }) => {
                    assert!(reason.contains("unknown"), "{reason}");
                }
                other => panic!("expected CommitOutcomeUnknown, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_first_commit_failure_that_is_not_unknown_is_a_rollback() {
        let steps = drive_commit(vec![command_error(11601, "Interrupted")]);
        assert_eq!(steps.len(), 1);
        match stopped_with(&steps[0]) {
            BundleAttemptError::Failed(TransactionError::RolledBack { reason }) => {
                assert!(reason.starts_with("Commit failed: "), "{reason}");
            }
            other => panic!("expected RolledBack, got {other:?}"),
        }
    }

    #[test]
    fn keeping_the_source_does_not_change_the_message() {
        let driver = command_error(WRITE_CONFLICT_CODE, "WriteConflict");
        let before = internal_error(format!(
            "Failed to insert resource in transaction: {}",
            driver
        ));
        let after = internal_driver_error("Failed to insert resource in transaction", driver);
        assert_eq!(before.to_string(), after.to_string());
        assert!(
            std::error::Error::source(&after).is_some(),
            "the typed error survives as the source"
        );
    }
}
