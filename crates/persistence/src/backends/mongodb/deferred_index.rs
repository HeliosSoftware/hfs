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
//!   built the rows. A deferred create carries no `missing_params` until then:
//!   they, like its rows, are written by its batch (or the repair), which keeps
//!   the Bundle's write set to what an offloaded one writes (#1975). Its
//!   metadata is unknown during the lag, and a `:missing=true` on the metadata
//!   route leaves it out while it carries the marker (`missing_resource_condition`). A
//!   deferred update or delete still writes `missing_params` in the transaction,
//!   and the batch rewrites them after the SearchParameter reload.
//! - **Ledger.** The same transaction inserts the `search_index_pending`
//!   collection's documents: one per [`LEDGER_CHUNK_REFS`] resources, naming
//!   them as `Type/id`. They commit or abort with the Bundle and add a bounded
//!   number of documents, however many rows the Bundle would have written.
//! - **Post-commit step.** The request that ran the Bundle rewrites the rows
//!   and `missing_params` from the resources' current state, in batches of
//!   [`DEFERRED_INDEX_BATCH_RESOURCES`] resources, before it answers, and then
//!   deletes the ledger documents. It runs on the Bundle's own session and ends
//!   at the first batch that fails for good or at the Bundle's deadline,
//!   whichever comes first; what is left stays in the ledger.
//! - **Repair.** Ledger documents left behind by a crash, a cancelled request
//!   or a failed batch are finished by `repair_deferred_search_index`, which
//!   the server runs at startup and periodically. It claims a document before
//!   working on it and goes on past a failed batch.
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
use crate::error::StorageResult;

use super::MongoBackend;
use super::retry::retry_transient;
use super::storage::{
    SEARCH_INDEX_INSERT_CHUNK, chrono_to_bson, document_to_value, internal_driver_error,
    internal_error, with_missing_metadata_update,
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

/// The most resources one post-commit batch transaction rewrites: about 5,000
/// rows, far from the cache limit.
pub(super) const DEFERRED_INDEX_BATCH_RESOURCES: usize = 256;

/// How often one batch is tried before it is left to the repair. A batch is
/// idempotent, so a retry only repeats work.
pub(super) const DEFERRED_INDEX_BATCH_ATTEMPTS: u32 = 5;

/// How many batches in a row may run out of attempts before the repair gives up
/// on the rest of a ledger document. One poisoned resource fails one batch and
/// the healthy batches around it still commit; a MongoDB that is down fails
/// every batch, and this keeps the repair from paying every attempt of every
/// batch of the document.
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
    /// Batch transactions that committed.
    pub(super) batches: u64,
    /// Batch attempts that failed and were tried again.
    pub(super) retries: u64,
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

/// What [`MongoBackend::index_deferred_refs`] does once a batch has run out of
/// attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OnBatchFailure {
    /// Return the failure at once and start no further batch. The post-commit
    /// step uses it: the request is waiting, and a MongoDB that cannot take one
    /// batch will not take the next.
    Stop,
    /// Go on with the remaining batches and return the first failure at the
    /// end (or when [`MAX_CONSECUTIVE_BATCH_FAILURES`] batches in a row have
    /// failed). The repair uses it: one poisoned resource must not hold up the
    /// healthy ones in its ledger document.
    Continue,
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
}

impl MongoBackend {
    /// One attempt to write the search rows of up to
    /// [`DEFERRED_INDEX_BATCH_RESOURCES`] resources of one type, in one small
    /// transaction on `session`. Returns the resources rewritten and the rows
    /// inserted.
    ///
    /// The transaction reads the resources that still carry the
    /// [`SEARCH_ROWS_PENDING_FIELD`] marker, deletes their rows, inserts rows
    /// built from the stored documents (the builder the inline write and
    /// `$reindex` use; a deleted resource gets none) and clears the marker. The
    /// update that clears it also rewrites each resource's `missing_params` from
    /// the same extraction: the names, nothing when the extraction failed, and
    /// `[]` for a deleted resource. A created resource has none until this
    /// write (#1975). An updated one has those its Bundle's transaction wrote
    /// with the registry as it was before its SearchParameters were reloaded;
    /// the rows are built after that reload.
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
    /// repeated or racing run is a no-op.
    async fn index_deferred_batch_once(
        &self,
        db: &Database,
        session: &mut ClientSession,
        tenant_id: &str,
        resource_type: &str,
        ids: &[String],
    ) -> StorageResult<(usize, u64)> {
        session.start_transaction().await.map_err(|e| {
            internal_driver_error("Failed to start the deferred search rows transaction", e)
        })?;
        let outcome = self
            .rewrite_deferred_batch(db, session, tenant_id, resource_type, ids)
            .await;
        match outcome {
            Ok(counts) => match session.commit_transaction().await {
                Ok(()) => Ok(counts),
                Err(e) => {
                    let _ = session.abort_transaction().await;
                    Err(internal_driver_error(
                        "Failed to commit the deferred search rows transaction",
                        e,
                    ))
                }
            },
            Err(e) => {
                let _ = session.abort_transaction().await;
                Err(e)
            }
        }
    }

    /// The operations of [`Self::index_deferred_batch_once`], inside its open
    /// transaction.
    async fn rewrite_deferred_batch(
        &self,
        db: &Database,
        session: &mut ClientSession,
        tenant_id: &str,
        resource_type: &str,
        ids: &[String],
    ) -> StorageResult<(usize, u64)> {
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

        let mut marked: Vec<String> = Vec::new();
        let mut updates: Vec<Document> = Vec::new();
        let mut own: Vec<Document> = Vec::new();
        let mut contained: Vec<Document> = Vec::new();
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
                updates.push(deferred_resource_update(
                    tenant_id,
                    resource_type,
                    &id,
                    rows.missing_params.as_deref(),
                    rows.missing_fp.as_deref(),
                ));
                own.extend(rows.own);
                contained.extend(rows.contained);
            } else {
                updates.push(deferred_resource_update(
                    tenant_id,
                    resource_type,
                    &id,
                    Some(&[]),
                    Some(&self.stamp_missing_fingerprint(tenant_id, resource_type)),
                ));
            }
            marked.push(id);
        }
        drop(cursor);

        if marked.is_empty() {
            return Ok((0, 0));
        }

        let rows = (own.len() + contained.len()) as u64;
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
        check_deferred_update_reply(&reply, marked.len())?;

        Ok((marked.len(), rows))
    }

    /// [`Self::index_deferred_batch_once`], tried up to
    /// [`DEFERRED_INDEX_BATCH_ATTEMPTS`] times and counted into `stats`. Every
    /// error is retried: the batch is idempotent, so a retry after an abort, a
    /// write conflict or a commit of unknown outcome only repeats work (a batch
    /// that did commit finds no marker the second time). The last failure is
    /// returned as text.
    ///
    /// `stop_at` bounds the batch itself, not only the start of it. It is
    /// checked before every attempt, and an attempt still running at that
    /// instant is cut. Either way the batch notes `stopped_at_deadline` in
    /// `stats` and returns `Ok(false)` without starting another transaction on
    /// the session, and without awaiting an abort of the one it cut: that
    /// could hang past the deadline, and dropping the session aborts the
    /// server-side transaction. `Ok(true)` means the batch committed.
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
    ) -> Result<bool, String> {
        let mut message = String::new();
        for attempt in 1..=DEFERRED_INDEX_BATCH_ATTEMPTS {
            if stop_at.is_some_and(|at| tokio::time::Instant::now() >= at) {
                stats.stopped_at_deadline = true;
                return Ok(false);
            }
            let once = self.index_deferred_batch_once(db, session, tenant_id, resource_type, ids);
            let outcome = match stop_at {
                Some(at) => match tokio::time::timeout_at(at, once).await {
                    Ok(outcome) => outcome,
                    Err(_elapsed) => {
                        stats.stopped_at_deadline = true;
                        return Ok(false);
                    }
                },
                None => once.await,
            };
            match outcome {
                Ok((indexed, rows)) => {
                    stats.indexed += indexed;
                    stats.rows += rows;
                    stats.batches += 1;
                    return Ok(true);
                }
                Err(e) => {
                    message = e.to_string();
                    if attempt < DEFERRED_INDEX_BATCH_ATTEMPTS {
                        stats.retries += 1;
                        tokio::time::sleep(Duration::from_millis(25 * u64::from(attempt))).await;
                    }
                }
            }
        }
        Err(message)
    }

    /// Writes the search rows of `refs` (all of one tenant) in batches of
    /// [`DEFERRED_INDEX_BATCH_RESOURCES`] resources of one type each.
    ///
    /// `stop_at` is the live step's deadline (see `index_deferred_batch`): once
    /// it has passed no further batch starts, the one in flight is cut, and the
    /// result is `Ok(false)` with `stopped_at_deadline` noted in `stats`. The
    /// repair passes `None`. `Ok(true)` means every batch committed. An `Err` is
    /// the failure of a batch that ran out of attempts; the batches that
    /// committed stay committed. What follows such a failure is `on_failure`'s:
    /// [`OnBatchFailure::Stop`] returns it at once, [`OnBatchFailure::Continue`]
    /// goes on and returns the first one at the end.
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
            for batch in ids.chunks(DEFERRED_INDEX_BATCH_RESOURCES) {
                match self
                    .index_deferred_batch(
                        db,
                        session,
                        tenant_id,
                        &resource_type,
                        batch,
                        stop_at,
                        stats,
                    )
                    .await
                {
                    Ok(true) => failed_in_a_row = 0,
                    Ok(false) => return Ok(false),
                    Err(message) => {
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
    /// It never fails the Bundle, which has committed. A batch that cannot be
    /// written and the `stop_at` deadline both stop the step: the chunk in hand
    /// and every later chunk stay in the ledger for the repair, and are logged.
    /// A stalled MongoDB therefore costs one batch's attempts, not one per
    /// chunk. No attempt outlives `stop_at` (see `index_deferred_batch`). The
    /// session is not used again after that, so its transaction, if one is
    /// still open, is aborted by the drop. Dropping the future aborts only the
    /// batch in flight; the batches before it have committed and the ledger
    /// covers the rest.
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

        let elapsed_ms = started.elapsed().as_millis() as u64;
        tracing::info!(
            tenant = %tenant_id,
            bundle_id = %deferred.bundle_id,
            entries,
            resources = stats.resources,
            indexed = stats.indexed,
            rows = stats.rows,
            batches = stats.batches,
            retries = stats.retries,
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
        stats
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
    /// A batch that runs out of attempts does not stop the document: the healthy
    /// batches are still written, and the document is kept with `not_before` set
    /// to now, `last_error` set and `attempts` incremented, so it is tried again
    /// after another `min_age`. A document with no `refs` array, or with refs
    /// that cannot be read, is kept the same way after its readable refs are
    /// indexed. A document is never dropped without all of its resources being
    /// indexed.
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
    pub async fn repair_deferred_search_index(
        &self,
        min_age: Duration,
        max_chunks: usize,
    ) -> StorageResult<DeferredIndexRepairReport> {
        let db = self.get_database().await?;
        let mut report = DeferredIndexRepairReport::default();
        if self.is_search_offloaded() || max_chunks == 0 {
            report.remaining = count_ledger_documents(&db).await?;
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
        report.remaining = count_ledger_documents(&db).await?;
        Ok(report)
    }
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
}
