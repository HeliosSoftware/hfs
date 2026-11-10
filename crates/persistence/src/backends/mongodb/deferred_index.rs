//! Deferred search indexing for large transaction Bundles (#1975).
//!
//! A transaction Bundle is one multi-document transaction, and the roughly 18.6
//! `search_index` rows each entry writes are most of what WiredTiger holds in
//! cache until the commit. A Bundle of at least
//! [`MongoBackend::deferred_search_index_min_entries`](super::MongoBackend::deferred_search_index_min_entries)
//! entries therefore writes only `resources` and `resource_history` inside the
//! transaction, and its search rows after the commit:
//!
//! - **Marker.** Each `resources` document whose rows are deferred carries
//!   `search_rows_pending: <bundle_id>` until its rows are written. The
//!   post-commit rewrite is gated on the marker and clears it in the same small
//!   transaction that writes the rows, so running it twice, or racing a client
//!   write or a repair, ends with one correct set of rows. The same update
//!   rewrites the document's `missing_params` (#1984) from the extraction that
//!   built the rows. A deferred create, update or restore carries no
//!   `missing_params` until then: they, like its rows, are written by its batch
//!   (or the repair) from one extraction, which keeps the Bundle's write set to
//!   what an offloaded one writes (#1975, #2035). An update or restore removes
//!   the metadata the previous version carried, so it is in the state a create
//!   is. Its metadata is unknown during the lag, and a `:missing=true` on the
//!   metadata route leaves it out while it carries the marker
//!   (`missing_resource_condition`). A deferred delete extracts nothing and
//!   still writes `missing_params: []` in the transaction, the smallest value
//!   of the index key; the batch writes the same `[]` with the fingerprint.
//! - **Ledger.** The same transaction inserts the `search_index_pending`
//!   collection's documents: one per [`LEDGER_CHUNK_REFS`] resources, naming
//!   them as `Type/id`. They commit or abort with the Bundle and add a bounded
//!   number of documents, however many rows the Bundle would have written.
//! - **Post-commit step.** The request that ran the Bundle rewrites the rows
//!   and `missing_params` from the resources' current state, in batches of up
//!   to [`DEFERRED_INDEX_BATCH_RESOURCES`] resources and about
//!   [`DEFERRED_INDEX_BATCH_ROWS`] rows (#2031), before it answers, and then
//!   deletes the ledger documents. It runs on the Bundle's own session and ends
//!   at the first chunk that fails for good or at the Bundle's deadline,
//!   whichever comes first; what is left stays in the ledger.
//! - **Repair.** Ledger documents left behind by a crash, a cancelled request
//!   or a failed batch are finished by `repair_deferred_search_index`, which
//!   the server runs at startup and periodically. It claims a document before
//!   working on it and goes on past a failed chunk.
//! - **Metrics (#2034, #2036).** Both are measured. Each post-commit step
//!   records its own duration, the rows it wrote and, when it left work to the
//!   repair, why (a deadline or a failure), as the
//!   `mongodb_deferred_search_index_step_*` series of
//!   `helios_observability::deferred_index_metrics`, and adds them to the
//!   per-instance totals that [`MongoBackend::deferred_index_step_totals`](super::MongoBackend::deferred_index_step_totals)
//!   reads back. Each repair pass publishes the ledger as it left it: its
//!   document count and the age of its oldest document (by `created_at`), the
//!   signal for "committed but not searchable for this long", and counts the
//!   documents it had to leave in place. A step whose request is dropped before
//!   it finishes records nothing; what it left is in the ledger, which the next
//!   pass reports. A Bundle that deferred nothing never runs a step.
//!
//! **Batches by rows, and a cache overflow (#2031).** A chunk of up to 256 ids
//! of one type is written by one or more batch transactions, each its own
//! transaction on the session:
//!
//! - A transaction takes the chunk's resources in the order its cursor returns
//!   them until the rows it has built reach [`DEFERRED_INDEX_BATCH_ROWS`]; the
//!   resource that reaches the budget is the last one it takes (a soft cut,
//!   and a resource with more rows than the budget is a batch of its own).
//!   The ids it did not take go to the next transaction. Each resource's rows
//!   are extracted once, by the transaction that takes it; the cursor is not
//!   read past the cut.
//! - A transaction of more than one resource that the server refuses because
//!   of the cache ([`is_cache_overflow_failure`]: `TransactionTooLargeForCache`
//!   (388), or a rollback for cache pressure: 365, or a `WriteConflict` naming
//!   the eviction) is not replayed as it was: the same writes meet the same
//!   cache. It is retried with half of the resources it held, down to one, and
//!   the smaller size is kept for the rest of the chunk.
//! - A single resource refused on its own cannot be split: its stale-row
//!   delete, its row insert and its marker clear are one transaction (see
//!   `index_deferred_batch_once`). What happens next depends on the refusal.
//!   388 is about that transaction's own write set ([`is_too_large_for_cache_failure`]),
//!   so a replay would fail the same way: it fails alone after one attempt,
//!   and the failure (the log, the ledger's `last_error`) says that
//!   `Type/id` with N rows is too large for one transaction. A cache-pressure
//!   rollback is the cache's load at that moment, which another writer's work
//!   changes, so it is replayed at the same size up to
//!   [`DEFERRED_INDEX_BATCH_ATTEMPTS`] times like any other failure, and only
//!   then left alone, with a failure that says the resource was rolled back
//!   for cache pressure that many times in a transaction of its own. Either
//!   way the chunk's other resources are written and the resource stays marked.
//! - A failure before the transaction has read its resources (starting it, the
//!   `find`), or any failure that is not a cache refusal, has nothing to
//!   shrink and is replayed at the same size, up to
//!   [`DEFERRED_INDEX_BATCH_ATTEMPTS`] times, as it always was.
//!
//! The live step and the repair share one implementation of all this
//! ([`MongoBackend::index_deferred_refs`]) and differ only in
//! [`OnBatchFailure`]: the step stops at the first chunk that did not finish,
//! the repair goes on.
//!
//! The transaction path lives in `storage.rs`. This module holds the constants
//! and the pure helpers, the post-commit step and the repair.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::{Bson, Document, doc, oid::ObjectId};
use mongodb::{ClientSession, Database};

use crate::core::{BundleEntry, BundleMethod};
use crate::error::{StorageError, StorageResult};

use helios_observability::deferred_index_metrics::IncompleteReason;

use super::MongoBackend;
use super::retry::retry_transient;
use super::storage::{
    SEARCH_INDEX_INSERT_CHUNK, chrono_to_bson, document_to_value, internal_driver_error,
    internal_error, is_cache_overflow_failure, is_too_large_for_cache_failure,
    with_missing_metadata_update,
};
use super::transaction_bundle_gate::{
    DEFAULT_MAX_CONCURRENT_TRANSACTION_BUNDLES, DEFAULT_TRANSACTION_BUNDLE_WEIGHT_ENTRIES,
};

/// The ledger of resources whose search rows are still to be written (#1975).
pub(crate) const SEARCH_INDEX_PENDING_COLLECTION: &str = "search_index_pending";

/// The `{not_before: 1}` index the repair reads the ledger through.
pub(super) const SEARCH_INDEX_PENDING_DUE_INDEX: &str = "idx_search_index_pending_due";

/// The `resources` field that names the Bundle whose rows for this resource
/// have not been written yet.
pub(super) const SEARCH_ROWS_PENDING_FIELD: &str = "search_rows_pending";

/// The most refs one ledger document holds. At 5,000 refs of 40-character
/// types and 64-character ids a document is about 600 KB, well under the 16 MB
/// limit, and a 50,000-entry Bundle adds ten of them.
pub(super) const LEDGER_CHUNK_REFS: usize = 5_000;

/// The most resources one post-commit batch transaction rewrites. It is a cap
/// on the count, not on the weight: [`DEFERRED_INDEX_BATCH_ROWS`] cuts a batch
/// of heavy resources earlier, and a cache overflow halves it
/// ([`halved_resource_limit`]).
pub(super) const DEFERRED_INDEX_BATCH_RESOURCES: usize = 256;

/// The rows a post-commit batch transaction may build before it takes no further
/// resource (#2031). The resource that reaches the budget is the last one the
/// transaction takes, so a batch holds at most one resource's rows more than
/// this, and a resource with more rows than this is a batch of its own.
///
/// Why 5,000: it is the rows of one `insert_many` (`SEARCH_INDEX_INSERT_CHUNK`,
/// the chunk the inline create path also sends), the size the README gives a
/// batch, and it keeps a batch of ordinary resources whole. 256 resources at
/// the benchmark's average of 18.6 rows are 4,762 rows, under it, so a typical
/// Bundle is cut by its resource count exactly as before and only rows-heavy
/// resources (a ValueSet expansion, a CodeSystem) are split. It is also a
/// fraction of the roughly 30,000 rows (1,846-1,851 entries at the ceiling
/// fixture's 16 rows each) that an inline Bundle held, with its resources and
/// history, when it stopped fitting a 0.5 GB cache in the README's ceiling
/// runs. The budget is not a guarantee: a resource can overflow the cache on
/// its own, which is the single-resource failure of
/// [`MongoBackend::index_deferred_refs`].
pub(super) const DEFERRED_INDEX_BATCH_ROWS: usize = 5_000;

/// How often one batch is tried before it is left to the repair. A batch is
/// idempotent, so a retry only repeats work. A replay is skipped only when it
/// cannot help ([`replay_can_help`]): a cache refusal of a transaction of more
/// than one resource is answered with a smaller transaction, and a
/// `TransactionTooLargeForCache` (388) of a lone resource is final, since the
/// same writes meet the same cache. Every other failure, a lone resource's
/// cache-pressure rollback and a failure before the resources were read
/// included, gets all of these attempts.
pub(super) const DEFERRED_INDEX_BATCH_ATTEMPTS: u32 = 5;

/// How many batches in a row may fail for good before the repair gives up on
/// the rest of a ledger document, and how many resources in a row may fail
/// alone, with no batch committing between them, before a chunk gives up on
/// the rest of its resources. One poisoned resource fails one batch and the
/// healthy batches around it still commit; a MongoDB that is down fails every
/// batch, and this keeps the repair from paying every attempt of every batch of
/// the document. A cache that is full of other work refuses every single
/// resource the same way, and this keeps a chunk from trying all 256 of them.
pub(super) const MAX_CONSECUTIVE_BATCH_FAILURES: u32 = 3;

/// The longest `last_error` a ledger document keeps, in characters.
pub(super) const MAX_LEDGER_ERROR_CHARS: usize = 1024;

/// The entry count from which a Bundle defers its search rows: `Some(0)` never
/// defers, `Some(n)` defers at `n` entries or more, and `None` follows the
/// admission room in entries (`entry_capacity`, set only for an entry-weighted
/// gate). With the gate off or counting Bundles there is no entry room, so the
/// room of the default gate applies.
pub(super) fn resolve_deferred_index_min_entries(
    configured: Option<usize>,
    entry_capacity: Option<usize>,
) -> Option<usize> {
    match configured {
        Some(0) => None,
        Some(n) => Some(n),
        None => Some(entry_capacity.unwrap_or(
            DEFAULT_MAX_CONCURRENT_TRANSACTION_BUNDLES * DEFAULT_TRANSACTION_BUNDLE_WEIGHT_ENTRIES,
        )),
    }
}

/// The resource types whose rows a deferred Bundle still writes inside the
/// transaction: the type of every POST entry that carries `ifNoneExist`.
///
/// The `ifNoneExist` check is matched in the Bundle's own session through the
/// search index, so it must see the rows of an earlier same-Bundle create of
/// the type it searches. The matcher reads rows of that type only, so indexing
/// just those types inline keeps it exactly as on the inline path.
pub(super) fn inline_index_types(entries: &[BundleEntry]) -> HashSet<String> {
    entries
        .iter()
        .filter(|entry| entry.method == BundleMethod::Post && entry.if_none_exist.is_some())
        .filter_map(|entry| {
            entry
                .resource
                .as_ref()?
                .get("resourceType")?
                .as_str()
                .map(str::to_string)
        })
        .collect()
}

/// What a Bundle that defers its search rows is called and which types stay
/// inline (see [`inline_index_types`]). Shared by every attempt of the Bundle.
pub(super) struct BundleDeferral {
    pub(super) bundle_id: String,
    pub(super) inline_types: HashSet<String>,
}

/// The resources one attempt of a deferred Bundle left without search rows, in
/// first-seen order.
pub(super) struct DeferredIndexSet {
    bundle_id: String,
    inline_types: HashSet<String>,
    refs: Vec<(String, String)>,
    seen: HashSet<(String, String)>,
}

impl DeferredIndexSet {
    pub(super) fn new(deferral: &BundleDeferral) -> Self {
        Self {
            bundle_id: deferral.bundle_id.clone(),
            inline_types: deferral.inline_types.clone(),
            refs: Vec::new(),
            seen: HashSet::new(),
        }
    }

    /// Whether a resource of this type has its rows deferred.
    pub(super) fn defers(&self, resource_type: &str) -> bool {
        !self.inline_types.contains(resource_type)
    }

    /// Notes a resource whose rows are deferred. A resource written twice by
    /// the Bundle is listed once.
    pub(super) fn record(&mut self, resource_type: &str, id: &str) {
        let key = (resource_type.to_string(), id.to_string());
        if self.seen.insert(key.clone()) {
            self.refs.push(key);
        }
    }

    pub(super) fn bundle_id(&self) -> &str {
        &self.bundle_id
    }

    pub(super) fn refs(&self) -> &[(String, String)] {
        &self.refs
    }

    pub(super) fn is_empty(&self) -> bool {
        self.refs.is_empty()
    }
}

/// One ledger document as the post-commit step works on it.
pub(super) struct LedgerChunk {
    pub(super) id: ObjectId,
    pub(super) refs: Vec<(String, String)>,
}

/// What a committed deferred Bundle left to do after the commit.
pub(super) struct DeferredCommit {
    pub(super) bundle_id: String,
    pub(super) chunks: Vec<LedgerChunk>,
    /// Resources named by the ledger (the sum of the chunks' refs).
    pub(super) resources: usize,
}

/// `Type/id`, the form a ledger document stores a ref in.
pub(super) fn format_ref(resource_type: &str, id: &str) -> String {
    format!("{resource_type}/{id}")
}

/// The inverse of [`format_ref`]: split at the first `/`, so an id may contain
/// one. `None` when either part is empty.
pub(super) fn parse_ref(stored: &str) -> Option<(String, String)> {
    let (resource_type, id) = stored.split_once('/')?;
    if resource_type.is_empty() || id.is_empty() {
        return None;
    }
    Some((resource_type.to_string(), id.to_string()))
}

/// The ledger documents for one attempt of a deferred Bundle, with the chunks
/// the post-commit step works from. Empty for no refs. Each document is
/// stamped due immediately; the repair only takes the ones older than its
/// minimum age.
pub(super) fn ledger_documents(
    tenant_id: &str,
    bundle_id: &str,
    entries: usize,
    refs: &[(String, String)],
    now: DateTime<Utc>,
) -> (Vec<Document>, Vec<LedgerChunk>) {
    if refs.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let chunks = refs.len().div_ceil(LEDGER_CHUNK_REFS);
    let stamp = chrono_to_bson(now);
    let mut documents = Vec::with_capacity(chunks);
    let mut ledger = Vec::with_capacity(chunks);
    for (n, part) in refs.chunks(LEDGER_CHUNK_REFS).enumerate() {
        let id = ObjectId::new();
        documents.push(doc! {
            "_id": id,
            "tenant_id": tenant_id,
            "bundle_id": bundle_id,
            "chunk": i32::try_from(n).unwrap_or(i32::MAX),
            "chunks": i32::try_from(chunks).unwrap_or(i32::MAX),
            "entries": i64::try_from(entries).unwrap_or(i64::MAX),
            "created_at": stamp,
            "not_before": stamp,
            "attempts": 0_i32,
            "refs": part
                .iter()
                .map(|(resource_type, id)| format_ref(resource_type, id))
                .collect::<Vec<_>>(),
        });
        ledger.push(LedgerChunk {
            id,
            refs: part.to_vec(),
        });
    }
    (documents, ledger)
}

/// Groups refs by resource type, types and ids each in first-seen order, an id
/// listed once.
pub(super) fn group_refs_by_type(refs: &[(String, String)]) -> Vec<(String, Vec<String>)> {
    let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
    let mut position: HashMap<&str, usize> = HashMap::new();
    let mut seen: HashSet<(&str, &str)> = HashSet::new();
    for (resource_type, id) in refs {
        if !seen.insert((resource_type.as_str(), id.as_str())) {
            continue;
        }
        let at = *position.entry(resource_type.as_str()).or_insert_with(|| {
            grouped.push((resource_type.clone(), Vec::new()));
            grouped.len() - 1
        });
        grouped[at].1.push(id.clone());
    }
    grouped
}

/// One statement of the batch's `update` command: it ends one resource's
/// deferral by removing the [`SEARCH_ROWS_PENDING_FIELD`] marker and writes its
/// `missing_params` as every other writer does (#1984). The names come from the
/// rebuilt extraction; the field is removed when that extraction failed
/// (`None`), and a deleted resource gets `[]`, as a soft delete writes it.
///
/// The filter still requires the marker, so a resource whose marker another
/// rewriter has already cleared is left alone.
pub(super) fn deferred_resource_update(
    tenant_id: &str,
    resource_type: &str,
    id: &str,
    missing_params: Option<&[String]>,
    missing_fp: Option<&str>,
) -> Document {
    doc! {
        "q": {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": id,
            SEARCH_ROWS_PENDING_FIELD: { "$exists": true },
        },
        "u": with_missing_metadata_update(
            doc! { "$unset": { SEARCH_ROWS_PENDING_FIELD: "" } },
            missing_params,
            missing_fp,
        ),
        "multi": false,
        "upsert": false,
    }
}

/// Checks the reply of the batch's `update` command: it must have no write
/// error and no write concern error, and it must have matched `expected`
/// statements (one per marked resource). Anything else is an error, so the
/// batch aborts and is tried again.
fn check_deferred_update_reply(reply: &Document, expected: usize) -> StorageResult<()> {
    if let Some(error) = reply.get("writeConcernError") {
        return Err(internal_error(format!(
            "The deferred search rows update was not acknowledged: {error}"
        )));
    }
    if let Some(errors) = reply.get("writeErrors") {
        return Err(internal_error(format!(
            "The deferred search rows update failed: {errors}"
        )));
    }
    let matched = match reply.get("n") {
        Some(Bson::Int32(n)) => i64::from(*n),
        Some(Bson::Int64(n)) => *n,
        _ => {
            return Err(internal_error(
                "The deferred search rows update reply has no matched count".to_string(),
            ));
        }
    };
    if usize::try_from(matched) != Ok(expected) {
        return Err(internal_error(format!(
            "The deferred search rows update matched {matched} resources, expected {expected}"
        )));
    }
    Ok(())
}

/// What the rewrite of deferred search rows did, for the log lines and the
/// repair report.
#[derive(Debug, Default, Clone)]
pub(super) struct DeferredIndexStats {
    /// Resources the ledger names.
    pub(super) resources: usize,
    /// Resources whose rows a batch rewrote and whose marker it cleared.
    pub(super) indexed: usize,
    /// Rows inserted (`search_index` and `search_index_contained`).
    pub(super) rows: u64,
    /// Batch transactions that committed and wrote at least one resource. A
    /// transaction that found every resource already done writes nothing and
    /// is not counted.
    pub(super) batches: u64,
    /// Batch attempts that failed and were tried again at the same size.
    pub(super) retries: u64,
    /// Batch transactions of more than one resource the cache refused, each
    /// followed by a smaller one ([`halved_resource_limit`]). A lone resource
    /// the cache refuses cannot be made smaller and is not counted.
    pub(super) shrinks: u64,
    /// Ledger documents deleted because every batch of theirs committed.
    pub(super) cleared_chunks: usize,
    /// Ledger documents left for the repair.
    pub(super) left_chunks: usize,
    /// Whether the rewrite stopped at its deadline: no further attempt was
    /// started, and one still running was cut.
    pub(super) stopped_at_deadline: bool,
    /// The message of the last batch failure.
    pub(super) last_error: Option<String>,
}

/// What [`MongoBackend::index_deferred_refs`] does once a chunk of up to
/// [`DEFERRED_INDEX_BATCH_RESOURCES`] ids has not finished: a batch ran out of
/// attempts, or the cache refuses a resource in a transaction of its own (a
/// [`ChunkEnd::Refused`] chunk). A chunk with such a resource is first finished
/// (its other resources are written) and a chunk whose batch ran out of
/// attempts is left where it stopped, whichever of the two this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OnBatchFailure {
    /// Return the failure at once and start no further chunk. The post-commit
    /// step uses it: the request is waiting, and a MongoDB that cannot take one
    /// batch will not take the next. The chunks after it, and the resource the
    /// cache refused, are left to the repair.
    Stop,
    /// Go on with the remaining chunks and return the first failure at the end
    /// (or when [`MAX_CONSECUTIVE_BATCH_FAILURES`] chunks in a row have run out
    /// of attempts). The repair uses it: one poisoned resource, or one the cache
    /// refuses, must not hold up the healthy ones in its ledger document. A
    /// chunk that failed only because the cache refused a resource shows the
    /// server is answering, so it does not count toward that limit.
    Continue,
}

/// What one batch transaction did, once committed.
struct BatchCommit {
    /// Resources whose rows it rewrote and whose marker it cleared.
    indexed: usize,
    /// Rows it inserted.
    rows: u64,
    /// The ids it was given and did not take, because the row budget
    /// ([`DEFERRED_INDEX_BATCH_ROWS`]) cut it. They are still marked, so the
    /// next transaction takes them. Empty when it read every marked resource
    /// of its ids.
    left: Vec<String>,
}

/// The resources a batch transaction held when it failed, once it had read
/// them: the ids it was writing and the rows it had built for them.
struct HeldResources {
    ids: Vec<String>,
    rows: u64,
}

/// A batch transaction that failed, with what it held.
struct BatchFailure {
    error: StorageError,
    /// `None` when it failed before it had read its resources (starting, or
    /// reading them): nothing says whether they fit.
    held: Option<HeldResources>,
}

impl BatchFailure {
    /// Boxed, since a `StorageError` is large and a batch's `Result` is not
    /// the place to carry it by value.
    fn boxed(error: StorageError, held: Option<HeldResources>) -> Box<Self> {
        Box::new(Self { error, held })
    }
}

/// What a batch's attempts came to ([`MongoBackend::index_deferred_batch_attempts`]).
enum BatchAttempt {
    /// A transaction committed; `left` is [`BatchCommit::left`].
    Committed { left: Vec<String> },
    /// The attempts ran out, or the failure is one that a replay cannot help
    /// ([`replay_can_help`]). `attempts` is how many transactions were tried.
    Failed {
        failure: Box<BatchFailure>,
        attempts: u32,
    },
    /// The deadline cut it: no further transaction starts.
    Deadline,
}

/// How one chunk of ids ended ([`MongoBackend::index_deferred_batch`]).
enum ChunkEnd {
    /// Every resource of the chunk is written (or was already).
    Done,
    /// The deadline cut it.
    Deadline,
    /// Every resource the cache lets through is written. The message names the
    /// ones it refuses in a transaction of their own, which stay marked: a
    /// resource refused as too large for one transaction ([`too_large_message`])
    /// and one rolled back for cache pressure on every attempt
    /// ([`cache_pressure_message`]).
    Refused(String),
    /// A batch ran out of attempts (or the cache refused too many single
    /// resources in a row). What it had not written yet stays marked.
    Failed(String),
}

/// The transaction size after the cache refused one that held `held`
/// resources: half of them, and at least one. Strictly smaller than `held`
/// from two resources up, so a chunk reaches one resource, where a failure is
/// the resource's own.
fn halved_resource_limit(held: usize) -> usize {
    (held / 2).max(1)
}

/// Whether running a failed batch transaction again at the same size can help.
/// `held` is the number of resources it had read when it failed (zero when it
/// failed before that), `too_large` that the failure is a
/// `TransactionTooLargeForCache` ([`is_too_large_for_cache_failure`]) and
/// `cache_refused` that it is any cache failure ([`is_cache_overflow_failure`]).
///
/// - More than one resource, refused by the cache: no. The caller halves the
///   transaction, which is what can help.
/// - One resource, refused with 388: no. It is that resource's own write set
///   against this cache, and the same writes fail the same way.
/// - Everything else: yes. A failure that says nothing about the cache is
///   transient like any other; so is a cache-pressure rollback of a lone
///   resource (365, a labelled 112), which is the cache's load at that moment,
///   not the resource's size; and a failure before the resources were read has
///   nothing to shrink.
fn replay_can_help(held: usize, too_large: bool, cache_refused: bool) -> bool {
    match held {
        0 => true,
        1 => !too_large,
        _ => !cache_refused,
    }
}

/// The ids of `ids` that a transaction did not take, in their order.
fn ids_not_taken(ids: &[String], taken: &[String]) -> Vec<String> {
    let taken: HashSet<&str> = taken.iter().map(String::as_str).collect();
    ids.iter()
        .filter(|id| !taken.contains(id.as_str()))
        .cloned()
        .collect()
}

/// What the log and the ledger say about a resource whose rows the server
/// refuses as too large for the cache (`TransactionTooLargeForCache`, 388) in a
/// transaction of their own: it cannot be made smaller, since its stale-row
/// delete, its row insert and its marker clear are one transaction. `error` is
/// the server's, which follows.
fn too_large_message(resource_type: &str, id: &str, rows: u64, error: &StorageError) -> String {
    format!(
        "{resource_type}/{id} with {rows} rows is too large for one transaction and cannot be \
         split: {error}"
    )
}

/// What the log and the ledger say about a resource that the cache rolled back
/// for pressure ([`is_cache_overflow_failure`] without 388) in a transaction of
/// its own on each of `attempts` attempts. Unlike 388 this does not say the
/// resource is too large: the rollback is about the cache's load, which a
/// writer elsewhere can cause, so the message says what was seen and that the
/// resource cannot be made smaller. `error` is the server's, which follows.
fn cache_pressure_message(
    resource_type: &str,
    id: &str,
    rows: u64,
    attempts: u32,
    error: &StorageError,
) -> String {
    format!(
        "{resource_type}/{id} with {rows} rows was rolled back for cache pressure in a \
         transaction of its own, {attempts} times, and cannot be split: {error}"
    )
}

/// The chunk's failure for the resources the cache refused in a transaction of
/// their own: the first one's message, and how many others there were.
fn refused_summary(messages: &[String]) -> String {
    match messages {
        [] => String::new(),
        [only] => only.clone(),
        [first, rest @ ..] => format!(
            "{first} (and {} more resources of the chunk were refused by the cache in a \
             transaction of their own)",
            rest.len()
        ),
    }
}

/// What a batch transaction reads, before it writes anything: the resources it
/// takes with the rows and the `update` statements built from them.
#[derive(Default)]
struct DeferredReads {
    /// The ids it takes, in the order the cursor returned them.
    marked: Vec<String>,
    /// One statement per taken resource ([`deferred_resource_update`]).
    updates: Vec<Document>,
    /// The `search_index` rows of the taken resources.
    own: Vec<Document>,
    /// Their `search_index_contained` rows.
    contained: Vec<Document>,
    /// The ids it did not take ([`BatchCommit::left`]).
    left: Vec<String>,
}

impl DeferredReads {
    fn rows(&self) -> usize {
        self.own.len() + self.contained.len()
    }
}

/// What [`MongoBackend::repair_deferred_search_index`] did in one pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeferredIndexRepairReport {
    /// Ledger documents this pass claimed and worked on, due and oldest first.
    pub examined: usize,
    /// Due ledger documents this pass did not work on, because another repair
    /// (or a concurrent call) claimed or finished the document first, or the
    /// claim could not be made. Not counted in `examined`; a document still
    /// there is due again after the next minimum age.
    pub skipped: usize,
    /// Resources those documents name.
    pub resources: usize,
    /// Resources whose search rows were rewritten by this pass. A resource
    /// whose rows a live request wrote first is not counted.
    pub reindexed: usize,
    /// Search rows inserted by this pass.
    pub rows: u64,
    /// Ledger documents deleted because all of their resources are indexed.
    pub cleared: usize,
    /// Ledger documents left in place, with the failure recorded on them.
    pub failed: usize,
    /// Ledger documents in the collection after the pass, due or not.
    pub remaining: u64,
    /// The age of the oldest ledger document after the pass, by its
    /// `created_at` (its `not_before` when it has none): how long the Bundle
    /// that left it has been committed and not fully searchable. `None` when
    /// the ledger was empty when it was read. It is read right after
    /// `remaining` and is not one atomic snapshot with it: a document another
    /// writer finishes in between can leave the two a document apart.
    pub oldest_pending: Option<Duration>,
}

impl MongoBackend {
    /// One attempt to write the search rows of the resources among `ids` (up to
    /// [`DEFERRED_INDEX_BATCH_RESOURCES`] of them, all of one type) that still
    /// carry the marker, in one small transaction on `session`. Returns what
    /// it did ([`BatchCommit`]) or what it held when it failed
    /// ([`BatchFailure`]).
    ///
    /// The transaction reads the resources that still carry the
    /// [`SEARCH_ROWS_PENDING_FIELD`] marker, deletes their rows, inserts rows
    /// built from the stored documents (the builder the inline write and
    /// `$reindex` use; a deleted resource gets none) and clears the marker. The
    /// update that clears it also rewrites each resource's `missing_params` from
    /// the same extraction: the names, nothing when the extraction failed, and
    /// `[]` for a deleted resource. A created, updated or restored resource
    /// has none until this write (#1975, #2035): its Bundle's transaction
    /// extracted nothing, so the metadata is built, like the rows, with the
    /// registry as it is after its SearchParameters were reloaded.
    ///
    /// It takes the resources in its cursor's order and stops reading once the
    /// rows built so far reach [`DEFERRED_INDEX_BATCH_ROWS`], so a batch of
    /// rows-heavy resources is several transactions (#2031). The resource that
    /// reaches the budget is the last one it takes, each resource is extracted
    /// once, and the cursor is not read past the cut: no resource is extracted
    /// and then left. The ids it did not take come back in
    /// [`BatchCommit::left`], for the next transaction.
    ///
    /// **Why this is safe next to other writers.** The marker is cleared, and
    /// `missing_params` written, in the same transaction that rewrites the
    /// rows, so every rewriter writes the resource document. A concurrent CRUD
    /// update (which replaces the rows from its own snapshot), another
    /// instance's post-commit step and the repair all conflict with it on that
    /// document: MongoDB lets the first writer win and the other aborts with a
    /// transient error, and its retry starts from the state the winner
    /// committed. Rows that were deleted from a stale snapshot or inserted
    /// twice cannot result. A resource whose marker is gone is skipped, so a
    /// repeated or racing run is a no-op. A tenant purge relies on the same
    /// write: `purge_tenant_data` deletes `resources` before the rows, so a
    /// batch that read a resource the purge then deletes cannot clear its
    /// marker (its update loses the write conflict on the deleted document),
    /// aborts, and finds no marker on its retry; a batch that has already
    /// cleared its markers holds those documents, and the purge waits for its
    /// commit before it deletes the rows (#2259).
    ///
    /// The same write is why one resource cannot be split across transactions:
    /// its stale-row delete, its row insert and its marker clear must commit
    /// together, or a reader or a racing writer could see its rows gone, or
    /// doubled, with the marker still set.
    async fn index_deferred_batch_once(
        &self,
        db: &Database,
        session: &mut ClientSession,
        tenant_id: &str,
        resource_type: &str,
        ids: &[String],
    ) -> Result<BatchCommit, Box<BatchFailure>> {
        session.start_transaction().await.map_err(|e| {
            BatchFailure::boxed(
                internal_driver_error("Failed to start the deferred search rows transaction", e),
                None,
            )
        })?;
        let mut reads = match self
            .read_deferred_batch(db, session, tenant_id, resource_type, ids)
            .await
        {
            Ok(reads) => reads,
            Err(error) => {
                let _ = session.abort_transaction().await;
                return Err(BatchFailure::boxed(error, None));
            }
        };
        let commit = BatchCommit {
            indexed: reads.marked.len(),
            rows: reads.rows() as u64,
            left: std::mem::take(&mut reads.left),
        };
        let held = HeldResources {
            ids: reads.marked.clone(),
            rows: commit.rows,
        };
        match self
            .write_deferred_batch(db, session, tenant_id, resource_type, reads)
            .await
        {
            Ok(()) => match session.commit_transaction().await {
                Ok(()) => Ok(commit),
                Err(e) => {
                    let _ = session.abort_transaction().await;
                    Err(BatchFailure::boxed(
                        internal_driver_error(
                            "Failed to commit the deferred search rows transaction",
                            e,
                        ),
                        Some(held),
                    ))
                }
            },
            Err(error) => {
                let _ = session.abort_transaction().await;
                Err(BatchFailure::boxed(error, Some(held)))
            }
        }
    }

    /// The reads of [`Self::index_deferred_batch_once`], inside its open
    /// transaction: the marked resources among `ids`, up to the row budget, with
    /// their rows built (once each) and their `update` statements.
    async fn read_deferred_batch(
        &self,
        db: &Database,
        session: &mut ClientSession,
        tenant_id: &str,
        resource_type: &str,
        ids: &[String],
    ) -> StorageResult<DeferredReads> {
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let mut cursor = resources
            .find(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": { "$in": ids },
                SEARCH_ROWS_PENDING_FIELD: { "$exists": true },
            })
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_driver_error("Failed to read the resources with deferred search rows", e)
            })?;

        let mut reads = DeferredReads::default();
        while cursor.advance(&mut *session).await.map_err(|e| {
            internal_driver_error("Failed to advance the deferred search rows cursor", e)
        })? {
            let stored = cursor.deserialize_current().map_err(|e| {
                internal_error(format!(
                    "Failed to deserialize a resource with deferred search rows: {e}"
                ))
            })?;
            let id = stored
                .get_str("id")
                .map_err(|e| {
                    internal_error(format!(
                        "A resource with deferred search rows has no id: {e}"
                    ))
                })?
                .to_string();
            if !stored.get_bool("is_deleted").unwrap_or(false) {
                let data = stored.get_document("data").map_err(|e| {
                    internal_error(format!(
                        "{resource_type}/{id} has no stored data to index: {e}"
                    ))
                })?;
                let content = document_to_value(data)?;
                let (rows, _failure) =
                    self.search_index_documents_checked(tenant_id, resource_type, &id, &content);
                reads.updates.push(deferred_resource_update(
                    tenant_id,
                    resource_type,
                    &id,
                    rows.missing_params.as_deref(),
                    rows.missing_fp.as_deref(),
                ));
                reads.own.extend(rows.own);
                reads.contained.extend(rows.contained);
            } else {
                reads.updates.push(deferred_resource_update(
                    tenant_id,
                    resource_type,
                    &id,
                    Some(&[]),
                    Some(&self.stamp_missing_fingerprint(tenant_id, resource_type)),
                ));
            }
            reads.marked.push(id);
            if reads.rows() >= DEFERRED_INDEX_BATCH_ROWS {
                // The budget is reached, and this resource, the one that
                // reached it, is in. The cursor is dropped unread: nothing
                // past the cut is extracted.
                reads.left = ids_not_taken(ids, &reads.marked);
                break;
            }
        }
        drop(cursor);
        Ok(reads)
    }

    /// The writes of [`Self::index_deferred_batch_once`], inside its open
    /// transaction, for the resources [`Self::read_deferred_batch`] took.
    async fn write_deferred_batch(
        &self,
        db: &Database,
        session: &mut ClientSession,
        tenant_id: &str,
        resource_type: &str,
        reads: DeferredReads,
    ) -> StorageResult<()> {
        let DeferredReads {
            marked,
            updates,
            own,
            contained,
            left: _,
        } = reads;
        if marked.is_empty() {
            return Ok(());
        }

        let stale = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "resource_id": { "$in": marked.as_slice() },
        };
        for (collection, context) in [
            (
                MongoBackend::SEARCH_INDEX_COLLECTION,
                "Failed to delete the stale search index rows of deferred resources",
            ),
            (
                MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION,
                "Failed to delete the stale search_index_contained rows of deferred resources",
            ),
        ] {
            db.collection::<Document>(collection)
                .delete_many(stale.clone())
                .session(&mut *session)
                .await
                .map_err(|e| internal_driver_error(context, e))?;
        }

        for (collection, documents, context) in [
            (
                MongoBackend::SEARCH_INDEX_COLLECTION,
                &own,
                "Failed to insert deferred search index entries",
            ),
            (
                MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION,
                &contained,
                "Failed to insert deferred search_index_contained entries",
            ),
        ] {
            let collection = db.collection::<Document>(collection);
            for part in documents.chunks(SEARCH_INDEX_INSERT_CHUNK) {
                collection
                    .insert_many(part)
                    .session(&mut *session)
                    .await
                    .map_err(|e| internal_driver_error(context, e))?;
            }
        }

        // One statement per resource, in one command: each clears the marker
        // and writes that resource's `missing_params`. It is the last write of
        // the batch, and the write that serializes it against other writers.
        let reply = db
            .run_command(doc! {
                "update": MongoBackend::RESOURCES_COLLECTION,
                "updates": updates,
                "ordered": true,
            })
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_driver_error(
                    "Failed to clear the deferred search rows marker and write missing_params",
                    e,
                )
            })?;
        check_deferred_update_reply(&reply, marked.len())
    }

    /// [`Self::index_deferred_batch_once`] for `ids`, tried up to
    /// [`DEFERRED_INDEX_BATCH_ATTEMPTS`] times and counted into `stats`. A
    /// failed attempt is retried at the same size, with a growing pause, unless
    /// a replay cannot help ([`replay_can_help`]): the batch is idempotent, so
    /// a retry after an abort, a write conflict or a commit of unknown outcome
    /// only repeats work (a batch that did commit finds no marker the second
    /// time). The failures handed back at once are the cache's refusals
    /// ([`is_cache_overflow_failure`]) of a transaction of more than one
    /// resource, for [`Self::index_deferred_batch`] to make smaller, and a
    /// lone resource's `TransactionTooLargeForCache`
    /// ([`is_too_large_for_cache_failure`]), whose same writes meet the same
    /// cache. Everything else gets every attempt: a failure that says nothing
    /// about the cache, a failure before the resources were read (there is
    /// nothing to shrink), and a lone resource's cache-pressure rollback (365,
    /// a labelled 112), which is the cache's load at that moment and not the
    /// resource's size. [`BatchAttempt::Failed`] says how many attempts it
    /// took.
    ///
    /// `stop_at` bounds the batch itself, not only the start of it. It is
    /// checked before every attempt, and an attempt still running at that
    /// instant is cut. Either way the batch notes `stopped_at_deadline` in
    /// `stats` and returns [`BatchAttempt::Deadline`] without starting another
    /// transaction on the session, and without awaiting an abort of the one it
    /// cut: that could hang past the deadline, and dropping the session aborts
    /// the server-side transaction.
    #[allow(clippy::too_many_arguments)]
    async fn index_deferred_batch_attempts(
        &self,
        db: &Database,
        session: &mut ClientSession,
        tenant_id: &str,
        resource_type: &str,
        ids: &[String],
        stop_at: Option<tokio::time::Instant>,
        stats: &mut DeferredIndexStats,
    ) -> BatchAttempt {
        let mut attempt = 0_u32;
        loop {
            attempt += 1;
            if stop_at.is_some_and(|at| tokio::time::Instant::now() >= at) {
                stats.stopped_at_deadline = true;
                return BatchAttempt::Deadline;
            }
            let once = self.index_deferred_batch_once(db, session, tenant_id, resource_type, ids);
            let outcome = match stop_at {
                Some(at) => match tokio::time::timeout_at(at, once).await {
                    Ok(outcome) => outcome,
                    Err(_elapsed) => {
                        stats.stopped_at_deadline = true;
                        return BatchAttempt::Deadline;
                    }
                },
                None => once.await,
            };
            match outcome {
                Ok(commit) => {
                    stats.indexed += commit.indexed;
                    stats.rows += commit.rows;
                    if commit.indexed > 0 {
                        stats.batches += 1;
                    }
                    return BatchAttempt::Committed { left: commit.left };
                }
                Err(failure) => {
                    let held = failure.held.as_ref().map_or(0, |held| held.ids.len());
                    if attempt >= DEFERRED_INDEX_BATCH_ATTEMPTS
                        || !replay_can_help(
                            held,
                            is_too_large_for_cache_failure(&failure.error),
                            is_cache_overflow_failure(&failure.error),
                        )
                    {
                        return BatchAttempt::Failed {
                            failure,
                            attempts: attempt,
                        };
                    }
                    stats.retries += 1;
                    tokio::time::sleep(Duration::from_millis(25 * u64::from(attempt))).await;
                }
            }
        }
    }

    /// Writes the search rows of one chunk: `ids` (at most
    /// [`DEFERRED_INDEX_BATCH_RESOURCES`], all of one type), in as many batch
    /// transactions as the row budget and the cache need (#2031).
    ///
    /// - A batch transaction takes resources until their rows reach
    ///   [`DEFERRED_INDEX_BATCH_ROWS`] (see [`Self::index_deferred_batch_once`]),
    ///   and the ids it leaves are the next transaction's.
    /// - A transaction of more than one resource that the cache refuses
    ///   ([`is_cache_overflow_failure`]) is followed by one with half of the
    ///   resources it held ([`halved_resource_limit`]), and so on down to one.
    ///   The smaller size is kept for the rest of the chunk: the cache said
    ///   that size does not fit, and nothing says it has changed.
    /// - A single resource that the cache refuses on its own cannot be made
    ///   smaller. With `TransactionTooLargeForCache` (388,
    ///   [`is_too_large_for_cache_failure`]) it fails after one attempt, since
    ///   the same writes meet the same cache; with a cache-pressure rollback
    ///   (365, a labelled 112), the cache's load and not the resource's size, it
    ///   gets the same attempts as any other failure
    ///   ([`Self::index_deferred_batch_attempts`]) and fails when they run out.
    ///   Either way it is left marked and the chunk goes on with the others,
    ///   which are written; the chunk ends as [`ChunkEnd::Refused`] and names it
    ///   and its rows, as [`too_large_message`] or, for the rollback,
    ///   [`cache_pressure_message`] (which does not call it too large).
    ///   [`MAX_CONSECUTIVE_BATCH_FAILURES`] of them in a row, with no
    ///   transaction committing between them, mean the cache is full of other
    ///   work, not that those resources are heavy: the chunk gives up the rest
    ///   ([`ChunkEnd::Failed`]).
    /// - Any other failure, and one before the transaction has read its
    ///   resources (nothing to shrink), is retried at the same size, and a
    ///   batch that runs out of attempts ends the chunk as [`ChunkEnd::Failed`]
    ///   with the rest of it still marked. Transactions that committed before
    ///   stay committed.
    ///
    /// A retry extracts its resources again, as a retry always has: the failed
    /// transaction's rows were never committed, and nothing is carried over.
    ///
    /// `stop_at` is the live step's deadline (see
    /// [`Self::index_deferred_batch_attempts`]); once it has passed the chunk
    /// ends as [`ChunkEnd::Deadline`].
    #[allow(clippy::too_many_arguments)]
    async fn index_deferred_batch(
        &self,
        db: &Database,
        session: &mut ClientSession,
        tenant_id: &str,
        resource_type: &str,
        ids: &[String],
        stop_at: Option<tokio::time::Instant>,
        stats: &mut DeferredIndexStats,
    ) -> ChunkEnd {
        let mut pending: Vec<String> = ids.to_vec();
        let mut limit = DEFERRED_INDEX_BATCH_RESOURCES;
        let mut refused: Vec<String> = Vec::new();
        let mut refused_in_a_row = 0_u32;
        while !pending.is_empty() {
            let take = limit.min(pending.len());
            let attempt = self
                .index_deferred_batch_attempts(
                    db,
                    session,
                    tenant_id,
                    resource_type,
                    &pending[..take],
                    stop_at,
                    stats,
                )
                .await;
            let (failure, attempts) = match attempt {
                BatchAttempt::Deadline => return ChunkEnd::Deadline,
                BatchAttempt::Committed { left } => {
                    // What the transaction took, and what it found already
                    // done, leaves `pending`; what it did not reach goes first.
                    refused_in_a_row = 0;
                    drop(pending.splice(..take, left));
                    continue;
                }
                BatchAttempt::Failed { failure, attempts } => (failure, attempts),
            };

            let cache_refused = is_cache_overflow_failure(&failure.error);
            let held = failure.held.as_ref().filter(|_| cache_refused);
            match held.map(|held| (held.ids.as_slice(), held.rows)) {
                Some((held_ids, rows)) if held_ids.len() > 1 => {
                    limit = halved_resource_limit(held_ids.len());
                    stats.shrinks += 1;
                    tracing::warn!(
                        tenant = %tenant_id,
                        resource_type,
                        resources = held_ids.len(),
                        rows,
                        next_batch_resources = limit,
                        error = %failure.error,
                        "deferred search rows batch refused by the cache; \
                         retrying with fewer resources"
                    );
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Some(([only], rows)) => {
                    // The one resource the cache refuses on its own: 388 after
                    // one attempt, a cache-pressure rollback after all of them.
                    let id = only.clone();
                    let too_large = is_too_large_for_cache_failure(&failure.error);
                    let message = if too_large {
                        too_large_message(resource_type, &id, rows, &failure.error)
                    } else {
                        cache_pressure_message(resource_type, &id, rows, attempts, &failure.error)
                    };
                    tracing::warn!(
                        tenant = %tenant_id,
                        resource = %format!("{resource_type}/{id}"),
                        rows,
                        attempts,
                        too_large,
                        error = %failure.error,
                        "deferred search rows of one resource are refused by the cache in a \
                         transaction of its own; it stays marked for the repair and the chunk \
                         goes on"
                    );
                    pending.retain(|pending_id| *pending_id != id);
                    refused.push(message);
                    refused_in_a_row += 1;
                    if refused_in_a_row >= MAX_CONSECUTIVE_BATCH_FAILURES {
                        return ChunkEnd::Failed(format!(
                            "{}; gave up the rest of the chunk after {refused_in_a_row} \
                             resources in a row that the cache refuses alone",
                            refused_summary(&refused)
                        ));
                    }
                }
                _ => {
                    let message = failure.error.to_string();
                    return ChunkEnd::Failed(if refused.is_empty() {
                        message
                    } else {
                        format!("{}; {message}", refused_summary(&refused))
                    });
                }
            }
        }
        if refused.is_empty() {
            ChunkEnd::Done
        } else {
            ChunkEnd::Refused(refused_summary(&refused))
        }
    }

    /// Writes the search rows of `refs` (all of one tenant) in chunks of up to
    /// [`DEFERRED_INDEX_BATCH_RESOURCES`] resources of one type each, each
    /// chunk in as many transactions as its rows and the cache need
    /// ([`Self::index_deferred_batch`]).
    ///
    /// `stop_at` is the live step's deadline (see
    /// `index_deferred_batch_attempts`): once it has passed no further batch
    /// starts, the one in flight is cut, and the result is `Ok(false)` with
    /// `stopped_at_deadline` noted in `stats`. The repair passes `None`.
    /// `Ok(true)` means every chunk is written. An `Err` is the failure of a
    /// chunk that did not finish: a batch that ran out of attempts, or a
    /// resource the cache refuses in a transaction of its own, as too large (388,
    /// after one attempt) or rolled back for cache pressure on every attempt
    /// (its message names it and its rows; its chunk's other resources are
    /// written). The
    /// transactions that committed stay committed. What follows such a failure
    /// is `on_failure`'s: [`OnBatchFailure::Stop`] returns it at once,
    /// [`OnBatchFailure::Continue`] goes on and returns the first one at the
    /// end.
    #[allow(clippy::too_many_arguments)]
    async fn index_deferred_refs(
        &self,
        db: &Database,
        session: &mut ClientSession,
        tenant_id: &str,
        refs: &[(String, String)],
        stop_at: Option<tokio::time::Instant>,
        on_failure: OnBatchFailure,
        stats: &mut DeferredIndexStats,
    ) -> Result<bool, String> {
        let mut first_failure: Option<String> = None;
        let mut failed_in_a_row = 0_u32;
        'types: for (resource_type, ids) in group_refs_by_type(refs) {
            for chunk in ids.chunks(DEFERRED_INDEX_BATCH_RESOURCES) {
                match self
                    .index_deferred_batch(
                        db,
                        session,
                        tenant_id,
                        &resource_type,
                        chunk,
                        stop_at,
                        stats,
                    )
                    .await
                {
                    ChunkEnd::Done => failed_in_a_row = 0,
                    ChunkEnd::Deadline => return Ok(false),
                    ChunkEnd::Refused(message) => {
                        if on_failure == OnBatchFailure::Stop {
                            return Err(message);
                        }
                        first_failure.get_or_insert(message);
                        // The chunk's other resources were written: the
                        // server is answering, and no poison is spreading.
                        failed_in_a_row = 0;
                    }
                    ChunkEnd::Failed(message) => {
                        if on_failure == OnBatchFailure::Stop {
                            return Err(message);
                        }
                        first_failure.get_or_insert(message);
                        failed_in_a_row += 1;
                        if failed_in_a_row >= MAX_CONSECUTIVE_BATCH_FAILURES {
                            break 'types;
                        }
                    }
                }
            }
        }
        match first_failure {
            None => Ok(true),
            Some(message) if failed_in_a_row >= MAX_CONSECUTIVE_BATCH_FAILURES => Err(format!(
                "{message}; gave up the rest after {failed_in_a_row} failed batches in a row"
            )),
            Some(message) => Err(message),
        }
    }

    /// The step a deferred Bundle runs after its commit, before the request
    /// answers: rewrites the search rows of every resource its ledger names,
    /// then deletes each ledger document whose resources are all done.
    ///
    /// It runs on the Bundle's own `session`, idle after the commit. An explicit
    /// session is causally consistent, so the first batch reads at or after the
    /// Bundle's commit: its marker query cannot miss markers because the commit
    /// is not yet visible to it (for example under `readConcernLevel=majority`
    /// with `w:1`), and a batch that finds no marker was finished by another
    /// rewriter.
    ///
    /// Each chunk of up to 256 ids is written by as many batch transactions as
    /// its rows and the cache need, and a batch the cache refuses is retried
    /// with fewer resources (see `index_deferred_batch`, #2031).
    ///
    /// It never fails the Bundle, which has committed. A batch that cannot be
    /// written, a resource the cache refuses in a transaction of its own (its
    /// chunk's other resources are written first; the log names it) and the
    /// `stop_at` deadline all stop the step: what the chunk in hand did not
    /// write and every later chunk stay in the ledger for the repair, and are
    /// logged. A stalled MongoDB therefore costs one batch's attempts, not one
    /// per chunk. No attempt outlives `stop_at` (see
    /// `index_deferred_batch_attempts`). The
    /// session is not used again after that, so its transaction, if one is
    /// still open, is aborted by the drop. Dropping the future aborts only the
    /// batch in flight; the batches before it have committed and the ledger
    /// covers the rest.
    ///
    /// When it ends it records its own wall time, the rows it wrote and whether
    /// it left work to the repair ([`Self::record_deferred_step`], #2036). A
    /// step that is dropped before it ends records nothing. A Bundle that
    /// deferred nothing never calls this, so it records nothing either.
    pub(super) async fn write_deferred_search_rows(
        &self,
        db: &Database,
        session: &mut ClientSession,
        tenant_id: &str,
        deferred: &DeferredCommit,
        entries: usize,
        stop_at: Option<tokio::time::Instant>,
    ) -> DeferredIndexStats {
        let started = std::time::Instant::now();
        let mut stats = DeferredIndexStats {
            resources: deferred.resources,
            ..Default::default()
        };

        let ledger = db.collection::<Document>(SEARCH_INDEX_PENDING_COLLECTION);
        // Set by the first batch that fails for good: no further batch starts.
        let mut abandoned = false;
        for chunk in &deferred.chunks {
            if stats.stopped_at_deadline || abandoned {
                stats.left_chunks += 1;
                continue;
            }
            match self
                .index_deferred_refs(
                    db,
                    session,
                    tenant_id,
                    &chunk.refs,
                    stop_at,
                    OnBatchFailure::Stop,
                    &mut stats,
                )
                .await
            {
                Ok(true) => {
                    match retry_transient(|| async {
                        ledger.delete_one(doc! { "_id": chunk.id }).await
                    })
                    .await
                    {
                        Ok(_) => stats.cleared_chunks += 1,
                        Err(e) => {
                            // The rows are written; the repair finds
                            // nothing marked and deletes the document.
                            tracing::warn!(
                                tenant = %tenant_id,
                                bundle_id = %deferred.bundle_id,
                                "could not delete a deferred search index ledger document: {e}"
                            );
                            stats.left_chunks += 1;
                        }
                    }
                }
                Ok(false) => stats.left_chunks += 1,
                Err(message) => {
                    stats.last_error = Some(message);
                    stats.left_chunks += 1;
                    abandoned = true;
                }
            }
        }

        let elapsed = started.elapsed();
        let elapsed_ms = elapsed.as_millis() as u64;
        tracing::info!(
            tenant = %tenant_id,
            bundle_id = %deferred.bundle_id,
            entries,
            resources = stats.resources,
            indexed = stats.indexed,
            rows = stats.rows,
            batches = stats.batches,
            retries = stats.retries,
            shrinks = stats.shrinks,
            ms = elapsed_ms,
            "transaction bundle search rows written after commit"
        );
        if stats.left_chunks > 0 {
            tracing::warn!(
                tenant = %tenant_id,
                bundle_id = %deferred.bundle_id,
                entries,
                left_chunks = stats.left_chunks,
                stopped_at_deadline = stats.stopped_at_deadline,
                last_error = ?stats.last_error,
                "transaction bundle search rows not all written after commit; \
                 the deferred index repair will finish them"
            );
        }
        self.record_deferred_step(elapsed, &stats);
        stats
    }

    /// Records a finished post-commit step (#2036) in the per-instance totals
    /// and as the `mongodb_deferred_search_index_step_*` metrics: its own wall
    /// time, the rows it wrote and, if it left work to the repair, why.
    fn record_deferred_step(&self, elapsed: Duration, stats: &DeferredIndexStats) {
        let incomplete = incomplete_reason(stats);
        self.deferred_step_counters
            .record(elapsed, stats.rows, incomplete.is_some());
        helios_observability::deferred_index_metrics::record_step(
            elapsed.as_secs_f64(),
            stats.rows,
            incomplete,
        );
    }

    /// The number of documents in the `search_index_pending` ledger: Bundles'
    /// search rows still owed, due or not. Zero when every deferred Bundle has
    /// been fully indexed.
    pub async fn count_deferred_search_index_chunks(&self) -> StorageResult<u64> {
        let db = self.get_database().await?;
        count_ledger_documents(&db).await
    }

    /// Finishes the search rows of deferred transaction Bundles (#1975) that a
    /// crash, a cancelled request, a deadline or a failed batch left behind.
    ///
    /// Takes the ledger documents whose `not_before` is at least `min_age` old,
    /// oldest first, at most `max_chunks` of them, across tenants. It claims
    /// each one before working on it by moving its `not_before` to now, so a
    /// second repair (another instance, or a concurrent call) that read the same
    /// document skips it ([`DeferredIndexRepairReport::skipped`]); a claimer
    /// that crashes leaves a document that is due again after `min_age`. For
    /// each claimed document it rewrites the rows of its resources from their
    /// current state with the same marker-gated batch transactions the
    /// post-commit step uses (no deadline), then deletes the document.
    ///
    /// The batches are cut by rows and shrunk on a cache overflow exactly as the
    /// step's are: both run `index_deferred_refs` (#2031).
    ///
    /// A batch that runs out of attempts does not stop the document: the healthy
    /// batches are still written, and the document is kept with `not_before` set
    /// to now, `last_error` set and `attempts` incremented, so it is tried again
    /// after another `min_age`. So does a resource the cache refuses in a
    /// transaction of its own, as too large for one transaction (388, after one
    /// attempt) or rolled back for cache pressure on each of its five attempts:
    /// the others are written, the document is kept, and `last_error` names it
    /// and its rows. Rebuilt from the ledger, its batch reads only the resources
    /// still marked, so on the next pass that resource is tried alone, in one
    /// transaction. A document with no `refs` array, or with refs that cannot be
    /// read, is kept the same way after its readable refs are indexed. A
    /// document is never dropped without all of its resources being indexed.
    ///
    /// Safe to call while a client write or a live request's post-commit step is
    /// working on the same resources: the batches are gated on the marker and
    /// serialized by the write conflict on the resource document (see
    /// `index_deferred_batch_once`), so the worst case is repeated work.
    /// `min_age` is a lease, not a lock. Set it to at least the longest a live
    /// post-commit step can run (the Bundle deadline) and a repair does not
    /// duplicate the step of a Bundle still being answered.
    ///
    /// With search offloaded to a secondary no deferred ledger is written; this
    /// only reports the count.
    ///
    /// **Metrics (#2034).** A pass that returns a report publishes it before
    /// returning: the `mongodb_deferred_search_index_pending_chunks` and
    /// `..._oldest_pending_seconds` gauges from `remaining` and
    /// `oldest_pending`, and `..._repair_failed_chunks_total` from `failed`
    /// (see `helios_observability::deferred_index_metrics`). A pass that
    /// returns an error publishes nothing, so the gauges keep what the last
    /// successful pass left. Whoever calls this periodically therefore also
    /// keeps those series current; a process that never calls it exports no
    /// ledger gauges.
    pub async fn repair_deferred_search_index(
        &self,
        min_age: Duration,
        max_chunks: usize,
    ) -> StorageResult<DeferredIndexRepairReport> {
        let report = self.repair_pass(min_age, max_chunks).await?;
        publish_repair_report(&report);
        Ok(report)
    }

    /// One repair pass, as [`Self::repair_deferred_search_index`] describes it,
    /// without publishing its report.
    async fn repair_pass(
        &self,
        min_age: Duration,
        max_chunks: usize,
    ) -> StorageResult<DeferredIndexRepairReport> {
        let db = self.get_database().await?;
        let mut report = DeferredIndexRepairReport::default();
        if self.is_search_offloaded() || max_chunks == 0 {
            (report.remaining, report.oldest_pending) = read_ledger_backlog(&db).await?;
            return Ok(report);
        }

        let ledger = db.collection::<Document>(SEARCH_INDEX_PENDING_COLLECTION);
        let cutoff = chrono_to_bson(repair_cutoff(Utc::now(), min_age));
        let limit = i64::try_from(max_chunks).unwrap_or(i64::MAX);
        let chunks: Vec<Document> = retry_transient(|| async {
            ledger
                .find(doc! { "not_before": { "$lte": cutoff } })
                .sort(doc! { "not_before": 1_i32, "_id": 1_i32 })
                .limit(limit)
                .await?
                .try_collect()
                .await
        })
        .await
        .map_err(|e| internal_driver_error("Failed to list the deferred search index ledger", e))?;

        let mut stats = DeferredIndexStats::default();
        if !chunks.is_empty() {
            let mut session = db.client().start_session().await.map_err(|e| {
                internal_driver_error("Failed to start a session for the deferred index repair", e)
            })?;
            for chunk in &chunks {
                let chunk_id = chunk.get("_id").cloned().unwrap_or(Bson::Null);
                if !claim_ledger_chunk(&ledger, chunk, &chunk_id).await {
                    report.skipped += 1;
                    continue;
                }
                report.examined += 1;
                let bundle_id = chunk.get_str("bundle_id").unwrap_or_default();
                let (refs, problem) = refs_from_ledger_document(chunk);
                report.resources += refs.len();
                if let Some(problem) = &problem {
                    tracing::warn!(
                        bundle_id,
                        "deferred search index repair: {problem}; the chunk is kept"
                    );
                }

                let outcome = match chunk.get_str("tenant_id") {
                    Ok(tenant_id) => {
                        let tenant_id = tenant_id.to_string();
                        self.index_deferred_refs(
                            &db,
                            &mut session,
                            &tenant_id,
                            &refs,
                            None,
                            OnBatchFailure::Continue,
                            &mut stats,
                        )
                        .await
                        .map(|_| ())
                    }
                    Err(_) => Err("the ledger document has no tenant_id".to_string()),
                };
                // The readable refs are indexed either way, but a document that
                // names resources the repair cannot read is not finished.
                let outcome = match (outcome, problem) {
                    (Ok(()), None) => Ok(()),
                    (Ok(()), Some(problem)) => Err(problem),
                    (Err(message), None) => Err(message),
                    (Err(message), Some(problem)) => Err(format!("{problem}; {message}")),
                };
                match outcome {
                    Ok(()) => {
                        match retry_transient(|| async {
                            ledger.delete_one(doc! { "_id": chunk_id.clone() }).await
                        })
                        .await
                        {
                            Ok(_) => report.cleared += 1,
                            Err(e) => {
                                note_ledger_failure(
                                    &ledger,
                                    chunk,
                                    &chunk_id,
                                    &format!("could not delete the finished ledger document: {e}"),
                                )
                                .await;
                                report.failed += 1;
                            }
                        }
                    }
                    Err(message) => {
                        note_ledger_failure(&ledger, chunk, &chunk_id, &message).await;
                        report.failed += 1;
                    }
                }
            }
        }
        report.reindexed = stats.indexed;
        report.rows = stats.rows;
        (report.remaining, report.oldest_pending) = read_ledger_backlog(&db).await?;
        Ok(report)
    }
}

/// Why a finished post-commit step left work to the repair, or `None` when it
/// left none: the Bundle's deadline cut it, or something else did (a batch that
/// ran out of attempts, a resource the cache refuses in a transaction of its
/// own, a finished ledger document that could not be deleted). The deadline
/// wins when both happened.
fn incomplete_reason(stats: &DeferredIndexStats) -> Option<IncompleteReason> {
    (stats.left_chunks > 0).then_some(if stats.stopped_at_deadline {
        IncompleteReason::Deadline
    } else {
        IncompleteReason::Failure
    })
}

/// Publishes what a repair pass found: the ledger's size and the age of its
/// oldest document (`0` for an empty ledger), and the documents it had to
/// leave in place.
fn publish_repair_report(report: &DeferredIndexRepairReport) {
    helios_observability::deferred_index_metrics::set_backlog(
        report.remaining,
        report.oldest_pending.map_or(0.0, |age| age.as_secs_f64()),
    );
    helios_observability::deferred_index_metrics::record_repair_failed_chunks(
        u64::try_from(report.failed).unwrap_or(u64::MAX),
    );
}

/// Records that the repair could not finish a ledger document: due again
/// after another minimum age, with the failure and the attempt count on it.
async fn note_ledger_failure(
    ledger: &mongodb::Collection<Document>,
    chunk: &Document,
    chunk_id: &Bson,
    message: &str,
) {
    let error: String = message.chars().take(MAX_LEDGER_ERROR_CHARS).collect();
    let attempts = chunk
        .get_i32("attempts")
        .map(i64::from)
        .or_else(|_| chunk.get_i64("attempts"))
        .unwrap_or(0)
        + 1;
    tracing::warn!(
        tenant = chunk.get_str("tenant_id").unwrap_or_default(),
        bundle_id = chunk.get_str("bundle_id").unwrap_or_default(),
        chunk = chunk.get_i32("chunk").unwrap_or_default(),
        attempts,
        error = %error,
        "deferred search index repair could not finish a ledger chunk"
    );
    let recorded = retry_transient(|| async {
        ledger
            .update_one(
                doc! { "_id": chunk_id.clone() },
                doc! {
                    "$set": {
                        "not_before": chrono_to_bson(Utc::now()),
                        "last_error": &error,
                    },
                    "$inc": { "attempts": 1_i32 },
                },
            )
            .await
    })
    .await;
    if let Err(e) = recorded {
        tracing::warn!("could not record a deferred search index repair failure: {e}");
    }
}

/// Counts the ledger's documents.
async fn count_ledger_documents(db: &Database) -> StorageResult<u64> {
    let ledger = db.collection::<Document>(SEARCH_INDEX_PENDING_COLLECTION);
    retry_transient(|| async { ledger.count_documents(doc! {}).await })
        .await
        .map_err(|e| internal_driver_error("Failed to count the deferred search index ledger", e))
}

/// The ledger as a repair pass leaves it: how many documents it holds, and the
/// age of the oldest (`None` for an empty ledger).
///
/// The oldest is one `find_one` sorted by `created_at`, with only the two
/// timestamps projected. The ledger is small (one document per 5,000 deferred
/// resources, deleted as soon as they are indexed, so a long ledger is a long
/// outage) and its only index is `not_before`, so this reads the ledger's
/// documents once, once per pass.
async fn read_ledger_backlog(db: &Database) -> StorageResult<(u64, Option<Duration>)> {
    let remaining = count_ledger_documents(db).await?;
    if remaining == 0 {
        return Ok((0, None));
    }
    let ledger = db.collection::<Document>(SEARCH_INDEX_PENDING_COLLECTION);
    let oldest = retry_transient(|| async {
        ledger
            .find_one(doc! {})
            .sort(doc! { "created_at": 1_i32, "_id": 1_i32 })
            .projection(doc! { "created_at": 1_i32, "not_before": 1_i32 })
            .await
    })
    .await
    .map_err(|e| {
        internal_driver_error(
            "Failed to read the oldest deferred search index ledger document",
            e,
        )
    })?;
    Ok((
        remaining,
        oldest.map(|d| ledger_document_age(&d, Utc::now())),
    ))
}

/// How old a ledger document is at `now`: from its `created_at`, or from its
/// `not_before` when it has none (a document the post-commit step did not
/// write). A document with neither, or one stamped in the future (a clock that
/// differs between instances), is `Duration::ZERO` old.
fn ledger_document_age(document: &Document, now: DateTime<Utc>) -> Duration {
    let stamp = document
        .get_datetime("created_at")
        .or_else(|_| document.get_datetime("not_before"));
    let Ok(stamp) = stamp else {
        return Duration::ZERO;
    };
    let age_ms = now
        .timestamp_millis()
        .saturating_sub(stamp.timestamp_millis());
    Duration::from_millis(u64::try_from(age_ms).unwrap_or(0))
}

/// The `not_before` time at or before which a ledger document is due for the
/// repair: `min_age` before `now`, and the earliest representable time when
/// `min_age` is too long to subtract (so a huge age selects nothing newer than
/// the beginning of time rather than overflowing).
fn repair_cutoff(now: DateTime<Utc>, min_age: Duration) -> DateTime<Utc> {
    chrono::Duration::from_std(min_age)
        .ok()
        .and_then(|age| now.checked_sub_signed(age))
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

/// Claims a ledger document for this repair pass: moves its `not_before` to
/// now, but only while it still holds the value the pass read. When another
/// repair got there first, or the document is gone, nothing matches and the
/// caller skips it. A claim that cannot be made at all is skipped too: the
/// document is untouched and due again after the next minimum age.
async fn claim_ledger_chunk(
    ledger: &mongodb::Collection<Document>,
    chunk: &Document,
    chunk_id: &Bson,
) -> bool {
    let read = chunk.get("not_before").cloned().unwrap_or(Bson::Null);
    let claimed = retry_transient(|| async {
        ledger
            .update_one(
                doc! { "_id": chunk_id.clone(), "not_before": read.clone() },
                doc! { "$set": { "not_before": chrono_to_bson(Utc::now()) } },
            )
            .await
    })
    .await;
    match claimed {
        Ok(result) => result.matched_count > 0,
        Err(e) => {
            tracing::warn!(
                tenant = chunk.get_str("tenant_id").unwrap_or_default(),
                bundle_id = chunk.get_str("bundle_id").unwrap_or_default(),
                "could not claim a deferred search index ledger chunk: {e}"
            );
            false
        }
    }
}

/// The refs a stored ledger document names, and what is wrong with it when it
/// cannot be read whole: no `refs` array, or entries that are not a readable
/// `Type/id` string. A document with a problem names resources the repair
/// cannot index, so it must not be deleted as finished.
fn refs_from_ledger_document(chunk: &Document) -> (Vec<(String, String)>, Option<String>) {
    let Ok(stored) = chunk.get_array("refs") else {
        return (
            Vec::new(),
            Some("the ledger document has no refs array".to_string()),
        );
    };
    let mut refs = Vec::with_capacity(stored.len());
    let mut unreadable = 0_usize;
    for entry in stored {
        match entry.as_str().and_then(parse_ref) {
            Some(parsed) => refs.push(parsed),
            None => unreadable += 1,
        }
    }
    let problem =
        (unreadable > 0).then(|| format!("the ledger document has {unreadable} unreadable refs"));
    (refs, problem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(
        method: BundleMethod,
        resource_type: &str,
        if_none_exist: Option<&str>,
    ) -> BundleEntry {
        BundleEntry {
            method,
            url: resource_type.to_string(),
            resource: Some(json!({ "resourceType": resource_type })),
            if_none_exist: if_none_exist.map(str::to_string),
            ..Default::default()
        }
    }

    fn refs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(t, i)| (t.to_string(), i.to_string()))
            .collect()
    }

    #[test]
    fn resolve_follows_the_setting_then_the_room() {
        // Zero is "never".
        assert_eq!(
            resolve_deferred_index_min_entries(Some(0), Some(4000)),
            None
        );
        assert_eq!(resolve_deferred_index_min_entries(Some(0), None), None);
        // An explicit size wins over the room.
        assert_eq!(
            resolve_deferred_index_min_entries(Some(5), Some(4000)),
            Some(5)
        );
        // Unset follows the room of an entry-weighted gate ...
        assert_eq!(
            resolve_deferred_index_min_entries(None, Some(4000)),
            Some(4000)
        );
        assert_eq!(resolve_deferred_index_min_entries(None, Some(2)), Some(2));
        // ... and the default room when the gate has none.
        assert_eq!(resolve_deferred_index_min_entries(None, None), Some(4000));
    }

    #[test]
    fn inline_types_are_the_targets_of_post_if_none_exist() {
        let entries = vec![
            entry(BundleMethod::Post, "Organization", Some("identifier=x")),
            entry(BundleMethod::Post, "Practitioner", Some("identifier=y")),
            // Not conditional creates: the matcher never reads their rows.
            entry(BundleMethod::Post, "Patient", None),
            entry(BundleMethod::Put, "Encounter", Some("identifier=z")),
            entry(BundleMethod::Delete, "Condition", Some("identifier=z")),
            // A repeated type is listed once.
            entry(BundleMethod::Post, "Organization", Some("identifier=w")),
            // No resource, or no resourceType: nothing to name.
            BundleEntry {
                method: BundleMethod::Post,
                if_none_exist: Some("identifier=v".to_string()),
                ..Default::default()
            },
            BundleEntry {
                method: BundleMethod::Post,
                resource: Some(json!({})),
                if_none_exist: Some("identifier=v".to_string()),
                ..Default::default()
            },
        ];
        let types = inline_index_types(&entries);
        let mut types: Vec<_> = types.into_iter().collect();
        types.sort();
        assert_eq!(types, ["Organization", "Practitioner"]);
        assert!(inline_index_types(&[]).is_empty());
    }

    #[test]
    fn the_set_dedupes_keeps_order_and_leaves_inline_types_out() {
        let deferral = BundleDeferral {
            bundle_id: "b-1".to_string(),
            inline_types: HashSet::from(["Organization".to_string()]),
        };
        let mut set = DeferredIndexSet::new(&deferral);
        assert!(set.is_empty());
        assert_eq!(set.bundle_id(), "b-1");
        assert!(set.defers("Patient"));
        assert!(!set.defers("Organization"));

        set.record("Patient", "p2");
        set.record("Observation", "o1");
        set.record("Patient", "p1");
        set.record("Patient", "p2");
        // The same id under another type is another resource.
        set.record("Observation", "p2");
        assert!(!set.is_empty());
        assert_eq!(
            set.refs(),
            refs(&[
                ("Patient", "p2"),
                ("Observation", "o1"),
                ("Patient", "p1"),
                ("Observation", "p2"),
            ])
        );
    }

    #[test]
    fn refs_round_trip_and_split_at_the_first_slash() {
        assert_eq!(format_ref("Patient", "abc"), "Patient/abc");
        assert_eq!(
            parse_ref("Patient/abc"),
            Some(("Patient".to_string(), "abc".to_string()))
        );
        assert_eq!(
            parse_ref(&format_ref("Observation", "o-1")),
            Some(("Observation".to_string(), "o-1".to_string()))
        );
        assert_eq!(parse_ref("Patient"), None);
        assert_eq!(parse_ref("Patient/"), None);
        assert_eq!(parse_ref("/abc"), None);
        assert_eq!(parse_ref(""), None);
        assert_eq!(
            parse_ref("Patient/a/b"),
            Some(("Patient".to_string(), "a/b".to_string()))
        );
    }

    #[test]
    fn ledger_documents_chunk_the_refs() {
        let all: Vec<(String, String)> = (0..12_001)
            .map(|n| ("Observation".to_string(), format!("o{n}")))
            .collect();
        let now = Utc::now();
        let (documents, chunks) = ledger_documents("t1", "b-1", 12_500, &all, now);
        assert_eq!(documents.len(), 3);
        assert_eq!(chunks.len(), 3);
        let sizes: Vec<_> = chunks.iter().map(|c| c.refs.len()).collect();
        assert_eq!(sizes, [5000, 5000, 2001]);

        for (n, (document, chunk)) in documents.iter().zip(&chunks).enumerate() {
            assert_eq!(document.get_object_id("_id").unwrap(), chunk.id);
            assert_eq!(document.get_str("tenant_id").unwrap(), "t1");
            assert_eq!(document.get_str("bundle_id").unwrap(), "b-1");
            assert_eq!(document.get_i32("chunk").unwrap(), n as i32);
            assert_eq!(document.get_i32("chunks").unwrap(), 3);
            assert_eq!(document.get_i64("entries").unwrap(), 12_500);
            assert_eq!(document.get_i32("attempts").unwrap(), 0);
            assert_eq!(
                document.get_datetime("created_at").unwrap(),
                &chrono_to_bson(now)
            );
            assert_eq!(
                document.get_datetime("not_before").unwrap(),
                &chrono_to_bson(now)
            );
            let stored: Vec<_> = document
                .get_array("refs")
                .unwrap()
                .iter()
                .map(|r| parse_ref(r.as_str().unwrap()).unwrap())
                .collect();
            assert_eq!(stored, chunk.refs);
        }
        // Every ref lands in exactly one chunk, in order.
        let joined: Vec<_> = chunks.iter().flat_map(|c| c.refs.clone()).collect();
        assert_eq!(joined, all);
        // Fresh ids for every document.
        let ids: HashSet<_> = chunks.iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), 3);

        let (documents, chunks) = ledger_documents("t1", "b-1", 0, &[], now);
        assert!(documents.is_empty() && chunks.is_empty());
    }

    #[test]
    fn a_large_bundles_ledger_stays_far_below_the_document_limit() {
        let resource_type = "R".repeat(40);
        let all: Vec<(String, String)> = (0..50_000)
            .map(|n| (resource_type.clone(), format!("{n:0>64}")))
            .collect();
        let (documents, _) = ledger_documents("tenant", "bundle", 50_000, &all, Utc::now());
        assert_eq!(documents.len(), 10);
        for document in &documents {
            let size = mongodb::bson::to_vec(document).unwrap().len();
            assert!(size < 1_048_576, "ledger document is {size} bytes");
        }
    }

    #[test]
    fn group_refs_by_type_keeps_first_seen_order_and_dedupes() {
        let grouped = group_refs_by_type(&refs(&[
            ("Patient", "p1"),
            ("Observation", "o1"),
            ("Patient", "p2"),
            ("Patient", "p1"),
            ("Observation", "o2"),
            ("Encounter", "e1"),
            ("Observation", "o1"),
        ]));
        assert_eq!(
            grouped,
            [
                (
                    "Patient".to_string(),
                    vec!["p1".to_string(), "p2".to_string()]
                ),
                (
                    "Observation".to_string(),
                    vec!["o1".to_string(), "o2".to_string()]
                ),
                ("Encounter".to_string(), vec!["e1".to_string()]),
            ]
        );
        assert!(group_refs_by_type(&[]).is_empty());
    }

    #[test]
    fn the_repair_cutoff_is_the_age_before_now_and_saturates() {
        let now = DateTime::parse_from_rfc3339("2026-10-13T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            repair_cutoff(now, Duration::from_secs(60)),
            DateTime::parse_from_rfc3339("2026-10-13T11:59:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
        assert_eq!(repair_cutoff(now, Duration::ZERO), now);
        // Longer than chrono can subtract: the earliest time, not a panic.
        assert_eq!(
            repair_cutoff(now, Duration::from_secs(u64::MAX)),
            DateTime::<Utc>::MIN_UTC
        );
        assert_eq!(
            repair_cutoff(now, Duration::from_secs(i64::MAX as u64 / 1000)),
            DateTime::<Utc>::MIN_UTC
        );
    }

    #[test]
    fn ledger_refs_are_read_back_and_unreadable_ones_are_a_problem() {
        let chunk = doc! {
            "refs": ["Patient/p1", "Observation/o/1", "nonsense", "Patient/", 7_i32, "/x"],
        };
        let (read, problem) = refs_from_ledger_document(&chunk);
        assert_eq!(read, refs(&[("Patient", "p1"), ("Observation", "o/1")]));
        assert_eq!(
            problem.as_deref(),
            Some("the ledger document has 4 unreadable refs")
        );

        // A document written by `ledger_documents` reads back as written.
        let all = refs(&[("Patient", "p1"), ("Encounter", "e1")]);
        let (documents, _) = ledger_documents("t1", "b-1", 2, &all, Utc::now());
        assert_eq!(refs_from_ledger_document(&documents[0]), (all, None));

        // An empty array is a readable, empty document.
        assert_eq!(
            refs_from_ledger_document(&doc! { "refs": [] }),
            (Vec::new(), None)
        );
    }

    #[test]
    fn the_deferred_update_clears_the_marker_and_writes_missing_params() {
        let names = vec!["a".to_string(), "b".to_string()];
        let cases = [
            (
                Some(names.as_slice()),
                Some("fp1"),
                doc! {
                    "$unset": { SEARCH_ROWS_PENDING_FIELD: "" },
                    "$set": { "missing_params": ["a", "b"], "missing_params_fp": "fp1" },
                },
            ),
            (
                Some(names.as_slice()),
                None,
                doc! {
                    "$unset": { SEARCH_ROWS_PENDING_FIELD: "", "missing_params_fp": "" },
                    "$set": { "missing_params": ["a", "b"] },
                },
            ),
            // A failed extraction removes the field instead of claiming absence.
            (
                None,
                None,
                doc! { "$unset": {
                    SEARCH_ROWS_PENDING_FIELD: "",
                    "missing_params": "",
                    "missing_params_fp": "",
                } },
            ),
            // A deleted resource has nothing missing, as a soft delete writes it.
            (
                Some(&[][..]),
                Some("fp1"),
                doc! {
                    "$unset": { SEARCH_ROWS_PENDING_FIELD: "" },
                    "$set": { "missing_params": Vec::<String>::new(), "missing_params_fp": "fp1" },
                },
            ),
        ];
        for (missing, fp, update) in cases {
            let statement = deferred_resource_update("t1", "Patient", "p1", missing, fp);
            assert_eq!(statement.get_document("u").unwrap(), &update, "{missing:?}");
            assert_eq!(
                statement.get_document("q").unwrap(),
                &doc! {
                    "tenant_id": "t1",
                    "resource_type": "Patient",
                    "id": "p1",
                    SEARCH_ROWS_PENDING_FIELD: { "$exists": true },
                },
            );
            assert_eq!(statement.get_bool("multi"), Ok(false));
            assert_eq!(statement.get_bool("upsert"), Ok(false));
        }
    }

    #[test]
    fn a_deferred_update_reply_must_match_every_resource() {
        assert!(check_deferred_update_reply(&doc! { "ok": 1.0, "n": 3_i32 }, 3).is_ok());
        assert!(check_deferred_update_reply(&doc! { "ok": 1.0, "n": 3_i64 }, 3).is_ok());
        let failures = [
            // Fewer resources matched than were rewritten.
            doc! { "ok": 1.0, "n": 2_i32 },
            // No count.
            doc! { "ok": 1.0 },
            doc! { "ok": 1.0, "n": "3" },
            // A write error or an unacknowledged write fails even a full count.
            doc! { "ok": 1.0, "n": 3_i32, "writeErrors": [{ "index": 0_i32, "code": 11_i32 }] },
            doc! { "ok": 1.0, "n": 3_i32, "writeConcernError": { "code": 64_i32 } },
        ];
        for reply in failures {
            assert!(check_deferred_update_reply(&reply, 3).is_err(), "{reply:?}");
        }
    }

    /// #2031: the budget leaves a batch of ordinary resources whole. 256
    /// resources at the benchmark's average of 18.6 rows are 4,762 rows, so a
    /// typical Bundle is still cut by its resource count alone; and the budget
    /// is one insert chunk, so a batch that stays under it is one `insert_many`.
    #[test]
    fn the_row_budget_keeps_an_average_batch_whole() {
        const AVERAGE_ROWS_PER_RESOURCE_TENTHS: usize = 186;
        let average_batch = DEFERRED_INDEX_BATCH_RESOURCES * AVERAGE_ROWS_PER_RESOURCE_TENTHS / 10;
        assert_eq!(average_batch, 4_761);
        assert!(
            average_batch < DEFERRED_INDEX_BATCH_ROWS,
            "{average_batch} rows of an average batch against a budget of {DEFERRED_INDEX_BATCH_ROWS}"
        );
        assert_eq!(DEFERRED_INDEX_BATCH_ROWS, SEARCH_INDEX_INSERT_CHUNK);
        const {
            assert!(DEFERRED_INDEX_BATCH_RESOURCES > 1 && DEFERRED_INDEX_BATCH_ROWS > 1);
        }
    }

    #[test]
    fn a_cache_refusal_halves_the_batch_down_to_one_resource() {
        assert_eq!(halved_resource_limit(256), 128);
        assert_eq!(halved_resource_limit(6), 3);
        assert_eq!(halved_resource_limit(3), 1);
        assert_eq!(halved_resource_limit(2), 1);
        // One resource cannot be made smaller; the caller treats it as the
        // resource's own failure and never asks.
        assert_eq!(halved_resource_limit(1), 1);
        assert_eq!(halved_resource_limit(0), 1);
        // From two resources up the limit strictly shrinks, so a chunk always
        // reaches one resource.
        let mut held = DEFERRED_INDEX_BATCH_RESOURCES;
        let mut steps = 0;
        while held > 1 {
            let next = halved_resource_limit(held);
            assert!(next < held);
            held = next;
            steps += 1;
        }
        assert_eq!(steps, 8, "256 resources reach one in eight halvings");
    }

    #[test]
    fn the_ids_a_cut_batch_leaves_are_the_ones_it_did_not_take_in_order() {
        let ids: Vec<String> = ["a", "b", "c", "d", "e"].map(String::from).to_vec();
        let taken = vec!["d".to_string(), "a".to_string()];
        assert_eq!(ids_not_taken(&ids, &taken), ["b", "c", "e"]);
        assert_eq!(ids_not_taken(&ids, &[]), ids);
        assert!(ids_not_taken(&ids, &ids).is_empty());
        assert!(ids_not_taken(&[], &taken).is_empty());
    }

    #[test]
    fn a_resource_that_does_not_fit_is_named_with_its_rows_and_the_servers_error() {
        let error =
            internal_error("Failed to insert deferred search index entries: 388".to_string());
        let message = too_large_message("ValueSet", "vs-1", 123_456, &error);
        assert!(
            message.starts_with("ValueSet/vs-1 with 123456 rows is too large"),
            "{message}"
        );
        assert!(message.contains("cannot be split"), "{message}");
        assert!(message.ends_with(&error.to_string()), "{message}");

        assert_eq!(refused_summary(&[]), "");
        assert_eq!(refused_summary(&["one".to_string()]), "one");
        assert_eq!(
            refused_summary(&["one".to_string(), "two".to_string(), "three".to_string()]),
            "one (and 2 more resources of the chunk were refused by the cache in a transaction \
             of their own)"
        );
    }

    /// A lone resource that a cache-pressure rollback left after all its
    /// attempts is named with its rows and the attempts, and is not called too
    /// large: the rollback is the cache's load, not the resource's size.
    #[test]
    fn a_resource_rolled_back_for_cache_pressure_is_not_called_too_large() {
        let error =
            internal_error("Failed to insert deferred search index entries: 365".to_string());
        let message = cache_pressure_message("Organization", "o-1", 20, 5, &error);
        assert!(
            message.starts_with("Organization/o-1 with 20 rows was rolled back for cache pressure"),
            "{message}"
        );
        assert!(message.contains("5 times"), "{message}");
        assert!(message.contains("cannot be split"), "{message}");
        assert!(!message.contains("too large"), "{message}");
        assert!(message.ends_with(&error.to_string()), "{message}");
        // Both messages begin `Type/id with N rows`.
        let too_large = too_large_message("Organization", "o-1", 20, &error);
        assert_eq!(
            too_large.split(" is ").next(),
            message.split(" was ").next(),
            "{too_large} / {message}"
        );
    }

    /// #2031: a replay is skipped only when it cannot help. Arguments: the
    /// resources the failed transaction held, whether it was 388, whether it was
    /// any cache failure (388 is one).
    #[test]
    fn a_failed_batch_is_replayed_unless_a_replay_cannot_help() {
        // More than one resource, any cache failure: hand it back to be halved.
        assert!(!replay_can_help(2, true, true));
        assert!(!replay_can_help(256, false, true));
        // A failure that is not the cache's is replayed at any size.
        assert!(replay_can_help(2, false, false));
        assert!(replay_can_help(256, false, false));
        // One resource: 388 is that resource's own write set, final at once.
        assert!(!replay_can_help(1, true, true));
        // A cache-pressure rollback of a lone resource is the cache's load.
        assert!(replay_can_help(1, false, true));
        // Any other failure of a lone resource.
        assert!(replay_can_help(1, false, false));
        // Failed before it had read its resources (nothing to shrink): replayed
        // whatever the failure was, a cache rollback included.
        assert!(replay_can_help(0, false, true));
        assert!(replay_can_help(0, false, false));
    }

    #[test]
    fn a_ledger_document_without_a_refs_array_is_a_problem_not_an_empty_one() {
        for chunk in [
            doc! {},
            doc! { "refs": "Patient/p1" },
            doc! { "refs": null },
        ] {
            let (read, problem) = refs_from_ledger_document(&chunk);
            assert!(read.is_empty());
            assert_eq!(
                problem.as_deref(),
                Some("the ledger document has no refs array"),
                "{chunk:?}"
            );
        }
    }

    #[test]
    fn a_step_is_incomplete_when_it_left_chunks_and_the_deadline_wins() {
        // Everything written: nothing left to the repair.
        assert_eq!(incomplete_reason(&DeferredIndexStats::default()), None);
        // A batch that failed for good: chunks left and an error.
        let failed = DeferredIndexStats {
            left_chunks: 2,
            last_error: Some("boom".to_string()),
            ..Default::default()
        };
        assert_eq!(incomplete_reason(&failed), Some(IncompleteReason::Failure));
        // Written, but the finished ledger document could not be deleted: a
        // chunk left with no batch error is still a failure, not a deadline.
        let undeleted = DeferredIndexStats {
            left_chunks: 1,
            ..Default::default()
        };
        assert_eq!(
            incomplete_reason(&undeleted),
            Some(IncompleteReason::Failure)
        );
        // Cut at the Bundle's deadline.
        let cut = DeferredIndexStats {
            left_chunks: 3,
            stopped_at_deadline: true,
            ..Default::default()
        };
        assert_eq!(incomplete_reason(&cut), Some(IncompleteReason::Deadline));
        // Both: the deadline is the reason.
        let both = DeferredIndexStats {
            last_error: Some("boom".to_string()),
            ..cut
        };
        assert_eq!(incomplete_reason(&both), Some(IncompleteReason::Deadline));
    }

    #[test]
    fn a_ledger_documents_age_comes_from_created_at_then_not_before() {
        let now = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
        let ago = |seconds: i64| chrono_to_bson(now - chrono::Duration::seconds(seconds));
        // `created_at` is the age of the Bundle's commit; a later `not_before`
        // (a claim or a recorded failure moves it) does not make it younger.
        let both = doc! { "created_at": ago(100), "not_before": ago(10) };
        assert_eq!(ledger_document_age(&both, now), Duration::from_secs(100));
        // A document without `created_at` falls back to its `not_before`.
        let only_not_before = doc! { "not_before": ago(30) };
        assert_eq!(
            ledger_document_age(&only_not_before, now),
            Duration::from_secs(30)
        );
        // Neither stamp, or a stamp in the future (clocks differ between
        // instances): zero, never a wrapped number.
        assert_eq!(ledger_document_age(&doc! {}, now), Duration::ZERO);
        let future = doc! { "created_at": ago(-5) };
        assert_eq!(ledger_document_age(&future, now), Duration::ZERO);
    }
}
