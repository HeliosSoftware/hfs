//! Sub-batch pipeline for MongoDB `$reindex` page preparation (#1403).
//!
//! While sub-batch *k*'s `insert_many` runs on a spawned task, the page's own
//! thread extracts sub-batch *k+1* inside `tokio::task::block_in_place`, so
//! HFS-side extraction overlaps MongoDB-side delete/insert instead of running
//! strictly after it. This module holds the planner that sizes sub-batches,
//! the serial and
//! overlapped writer bodies (the `block_in_place` + rayon extraction helper
//! they share with the Elasticsearch writer lives in
//! `search::reindex_prepare`); `storage.rs`'s
//! `write_search_entries_page_timed` dispatches between the two and keeps
//! the rest of the `ReindexTarget`/`ReindexSource` methods.
//!
//! See `docs/superpowers/specs/2026-09-23-mongodb-reindex-rebuild-design.md`
//! §4.4.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use futures::TryStreamExt;
use mongodb::bson::oid::ObjectId;
use mongodb::bson::{Bson, Document, doc};

use crate::error::{BackendError, StorageError, StorageResult, classify_mongodb_error};
use crate::search::reindex::ReindexPageStats;
use crate::types::StoredResource;

use super::retry::{REINDEX_PAGE_RETRY, full_jitter, is_transient_mongo_error, jitter_fraction};
use super::storage::{ExtractionReport, SearchIndexDocuments, internal_error};

/// A type's first page: too little is known yet to size from observation (#1403).
pub(super) const REINDEX_SUBBATCH_FIRST: usize = 32;
/// Own + contained index documents a sub-batch targets (#1403).
pub(super) const REINDEX_SUBBATCH_TARGET_DOCS: usize = 2_500;
/// Largest resources a single sub-batch may hold (#1403).
pub(super) const REINDEX_SUBBATCH_MAX: usize = 512;
/// Bounded cleanup attempts; survivors are reported for resource recovery,
/// regardless of how long a concurrent migration or writer takes.
const VERIFY_CLEANUP_ATTEMPTS: u32 = 3;
const VERIFY_CLEANUP_BACKOFF: Duration = Duration::from_millis(100);
/// Mirrors PostgreSQL's reindex prepare minimum page size: the smallest page
/// for which the serial writer still tries the rayon pool (#1403).
pub(super) const REINDEX_SERIAL_POOL_MIN_PAGE: usize = 16;

/// Sizes successive extraction sub-batches of one `$reindex` page so a
/// pipeline of extraction and insert fills and drains in about one sub-batch
/// each (#1403). `len` indices are covered exactly once, in order, by
/// [`Self::next_range`]; [`Self::record`] feeds back what was actually
/// extracted so later ranges in the *same* page size from real data instead
/// of the type-level seed.
pub(super) struct SubBatchPlanner {
    len: usize,
    next: usize,
    min_size: usize,
    /// (resources, docs) of this type's previous overlapped page, or `None`
    /// on a type's first page.
    seed: Option<(u64, u64)>,
    resources_done: u64,
    docs_done: u64,
}

impl SubBatchPlanner {
    pub(super) fn new(len: usize, seed: Option<(u64, u64)>, min_size: usize) -> Self {
        Self {
            len,
            next: 0,
            min_size,
            seed,
            resources_done: 0,
            docs_done: 0,
        }
    }

    /// `REINDEX_SUBBATCH_TARGET_DOCS` documents' worth of resources, at the
    /// observed `docs / resources` ratio; `MAX` when nothing has been
    /// observed yet (`docs == 0`), never zero (`div_ceil` on a positive
    /// numerator is at least 1).
    fn size_for(resources: u64, docs: u64) -> usize {
        if docs == 0 {
            return REINDEX_SUBBATCH_MAX;
        }
        let size = (REINDEX_SUBBATCH_TARGET_DOCS as u64)
            .saturating_mul(resources)
            .div_ceil(docs);
        usize::try_from(size)
            .unwrap_or(REINDEX_SUBBATCH_MAX)
            .min(REINDEX_SUBBATCH_MAX)
    }

    /// The next sub-batch's range, or `None` once `len` is covered. Sizes
    /// from this page's own running observation once it has one, else from
    /// the type-level seed, else `REINDEX_SUBBATCH_FIRST`; always clamps to
    /// `[min_size.clamp(1, MAX), MAX]` so a pool never starves and a range is
    /// never empty (guaranteeing termination).
    pub(super) fn next_range(&mut self) -> Option<Range<usize>> {
        if self.next >= self.len {
            return None;
        }
        let raw = if self.resources_done > 0 {
            Self::size_for(self.resources_done, self.docs_done)
        } else if let Some((r, d)) = self.seed
            && r > 0
        {
            Self::size_for(r, d)
        } else {
            REINDEX_SUBBATCH_FIRST
        };
        let size = raw.clamp(
            self.min_size.clamp(1, REINDEX_SUBBATCH_MAX),
            REINDEX_SUBBATCH_MAX,
        );
        let end = (self.next + size).min(self.len);
        let range = self.next..end;
        self.next = end;
        Some(range)
    }

    /// Feeds back what a just-extracted sub-batch actually held, so the next
    /// call to [`Self::next_range`] sizes from this page's own data instead
    /// of the seed.
    pub(super) fn record(&mut self, resources: usize, docs: usize) {
        self.resources_done += resources as u64;
        self.docs_done += docs as u64;
    }
}

pub(super) use crate::search::reindex_prepare::{
    PrepareEnv, extract_range, resolve_prepare_width, tokio_multi_thread_runtime,
};

/// One sub-batch's extracted documents, split by destination collection, with
/// page-wide resource-index owners for each document (#1403).
struct SubBatchDocs {
    own_owners: Vec<usize>,
    own_docs: Vec<Document>,
    contained_owners: Vec<usize>,
    contained_docs: Vec<Document>,
    missing_params: Vec<(usize, Option<Vec<String>>, Option<String>)>,
}

impl SubBatchDocs {
    fn len(&self) -> usize {
        self.own_docs.len() + self.contained_docs.len()
    }

    fn missing_updates(
        &mut self,
        tenant_id: &str,
        resources: &[StoredResource],
    ) -> Vec<MissingUpdate> {
        std::mem::take(&mut self.missing_params)
            .into_iter()
            .map(|(owner, names, fp)| {
                let resource = &resources[owner];
                MissingUpdate {
                    owner,
                    statement: doc! {
                        "q": resource_version_filter(tenant_id, resource),
                        "u": super::storage::with_missing_metadata_update(
                            doc! {},
                            names.as_deref(),
                            fp.as_deref(),
                        ),
                        "multi": false,
                        "upsert": false,
                    },
                    resource_type: resource.resource_type().to_owned(),
                    id: resource.id().to_owned(),
                    version_id: resource.version_id().to_owned(),
                    names,
                    fp,
                }
            })
            .collect()
    }
}

/// One resource's `missing_params` update statement, with the version it is
/// guarded on and the values it writes (the array and its `missing_params_fp`
/// stamp), so [`changed_missing_updates`] can
/// leave it out when the stored document already holds that value (#2005).
struct MissingUpdate {
    owner: usize,
    statement: Document,
    resource_type: String,
    id: String,
    version_id: String,
    names: Option<Vec<String>>,
    fp: Option<String>,
}

impl MissingUpdate {
    /// Whether `stored`, the live resource document, already holds what this
    /// update writes, array and stamp, at the version the update is guarded on. Anything else
    /// (another version, a deleted resource, a value of another shape) is
    /// written as before, and the version guard decides.
    fn is_unchanged(&self, stored: &Document) -> bool {
        if stored.get_str("version_id") != Ok(self.version_id.as_str())
            || stored.get_bool("is_deleted") != Ok(false)
        {
            return false;
        }
        // The statement also writes the stamp (#1999), so a stale or absent
        // stamp is due even when the array matches. Without an array the
        // statement unsets both.
        let fp = self.names.as_ref().and(self.fp.as_deref());
        if stored.get_str("missing_params_fp").ok() != fp {
            return false;
        }
        match (&self.names, stored.get("missing_params")) {
            (None, None) => true,
            (Some(names), Some(Bson::Array(current))) => {
                current.len() == names.len()
                    && current
                        .iter()
                        .zip(names)
                        .all(|(current, name)| current.as_str() == Some(name.as_str()))
            }
            _ => false,
        }
    }
}

/// Drops the updates whose resource already stores the same `missing_params`
/// at the same version, returning the rest and how many were dropped (#2005).
/// MongoDB already turns such an update into a no-op write, but each
/// statement still costs the server an index lookup and a document fetch,
/// about four times what reading the whole page costs, and on a routine
/// reindex that is most of the page. One projected read per resource type
/// replaces them. If that read fails, every update is sent, as before.
///
/// A writer that changes the resource after the read stores its own
/// `missing_params` under a new version, which the skipped update would not
/// have matched anyway.
async fn changed_missing_updates(
    resources: &mongodb::Collection<Document>,
    tenant_id: &str,
    updates: Vec<MissingUpdate>,
) -> (Vec<(usize, Document)>, u64) {
    let stored = match stored_missing_params(resources, tenant_id, &updates).await {
        Ok(stored) => stored,
        Err(error) => {
            tracing::debug!(
                tenant = %tenant_id,
                error = %error,
                "mongodb reindex could not read stored missing_params; sending every update"
            );
            HashMap::new()
        }
    };
    let mut skipped = 0;
    let changed = updates
        .into_iter()
        .filter(|update| {
            let unchanged = stored
                .get(&(update.resource_type.clone(), update.id.clone()))
                .is_some_and(|stored| update.is_unchanged(stored));
            skipped += u64::from(unchanged);
            !unchanged
        })
        .map(|update| (update.owner, update.statement))
        .collect();
    (changed, skipped)
}

/// Reads `version_id`, `is_deleted`, `missing_params` and `missing_params_fp`
/// of every resource
/// the updates name, keyed by (resource type, id), one query per type.
async fn stored_missing_params(
    resources: &mongodb::Collection<Document>,
    tenant_id: &str,
    updates: &[MissingUpdate],
) -> mongodb::error::Result<HashMap<(String, String), Document>> {
    let mut ids_by_type: HashMap<&str, Vec<&str>> = HashMap::new();
    for update in updates {
        ids_by_type
            .entry(update.resource_type.as_str())
            .or_default()
            .push(update.id.as_str());
    }
    let mut stored = HashMap::new();
    for (resource_type, ids) in ids_by_type {
        let mut cursor = resources
            .find(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": { "$in": ids },
            })
            .projection(doc! {
                "_id": 0,
                "id": 1,
                "version_id": 1,
                "is_deleted": 1,
                "missing_params": 1,
                "missing_params_fp": 1,
            })
            .await?;
        while let Some(row) = cursor.try_next().await? {
            if let Ok(id) = row.get_str("id") {
                stored.insert((resource_type.to_owned(), id.to_owned()), row);
            }
        }
    }
    Ok(stored)
}

/// Moves sub-batch `[start, start + prepared.len())`'s extracted documents
/// into one [`SubBatchDocs`], recording each resource's extraction failure
/// and document count at its **page-wide** index. Moves (never clones) its
/// documents, since each one is only needed here and copying it would be
/// wasted work (#1403).
fn flatten_sub_batch(
    start: usize,
    prepared: Vec<(SearchIndexDocuments, ExtractionReport)>,
    extract_failures: &mut [Option<String>],
    parameter_failures: &mut Vec<crate::search::ExtractionFailure>,
    doc_counts: &mut [usize],
) -> SubBatchDocs {
    let mut batch = SubBatchDocs {
        own_owners: Vec::new(),
        own_docs: Vec::new(),
        contained_owners: Vec::new(),
        contained_docs: Vec::new(),
        missing_params: Vec::new(),
    };
    for (offset, (docs, report)) in prepared.into_iter().enumerate() {
        let i = start + offset;
        doc_counts[i] = docs.own.len() + docs.contained.len();
        match report {
            ExtractionReport::Clean => {}
            ExtractionReport::FailedParameters(failures) => parameter_failures.extend(failures),
            ExtractionReport::FailedResource(message) => extract_failures[i] = Some(message),
        }
        batch
            .missing_params
            .push((i, docs.missing_params, docs.missing_fp));
        for d in docs.own {
            batch.own_owners.push(i);
            batch.own_docs.push(d);
        }
        for d in docs.contained {
            batch.contained_owners.push(i);
            batch.contained_docs.push(d);
        }
    }
    batch
}

/// Groups resources into one `{tenant_id, resource_type, resource_id: {$in:
/// ids}}` filter per distinct resource type in the page, in first-seen type
/// order (#1403).
fn delete_filters(tenant_id: &str, resources: &[StoredResource]) -> Vec<Document> {
    let mut order: Vec<&str> = Vec::new();
    let mut ids_by_type: HashMap<&str, Vec<Bson>> = HashMap::new();
    for resource in resources {
        let resource_type = resource.resource_type();
        ids_by_type
            .entry(resource_type)
            .or_insert_with(|| {
                order.push(resource_type);
                Vec::new()
            })
            .push(Bson::from(resource.id()));
    }
    order
        .into_iter()
        .map(|resource_type| {
            let ids = ids_by_type.remove(resource_type).unwrap_or_default();
            doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "resource_id": { "$in": ids },
            }
        })
        .collect()
}

fn resource_version_filter(tenant_id: &str, resource: &StoredResource) -> Document {
    doc! {
        "tenant_id": tenant_id,
        "resource_type": resource.resource_type(),
        "id": resource.id(),
        "version_id": resource.version_id(),
        "is_deleted": false,
    }
}

/// A failure whose scope is the page, kept distinct from attributable document
/// errors until the public writer boundary. The saved insert batch is never
/// sent again: a command may have applied despite an uncertain reply, and a
/// resend would double-insert. A transient failure instead re-runs the whole
/// page from its delete ([`MongoBackend::write_page_with_retry`]), which
/// extracts it afresh, removes what the failed run left and assigns new ids.
#[derive(Debug)]
pub(super) struct PageFailure {
    kind: Box<PageFailureKind>,
    permanent: HashMap<usize, String>,
}

#[derive(Debug)]
enum PageFailureKind {
    Driver {
        context: String,
        error: mongodb::error::Error,
    },
    Metadata(Document),
    Internal(String),
    Storage(Arc<StorageError>),
}

impl PageFailure {
    pub(super) fn driver(context: &str, error: mongodb::error::Error) -> Self {
        Self {
            kind: Box::new(PageFailureKind::Driver {
                context: context.into(),
                error,
            }),
            permanent: HashMap::new(),
        }
    }

    /// A driver failure that re-running the page from its delete can recover:
    /// transient, and with no per-resource permanent outcome attached (a
    /// document write error or an extraction failure is deterministic and has
    /// to reach the runner as it is).
    fn is_transient_page(&self) -> bool {
        self.permanent.is_empty()
            && matches!(
                self.kind.as_ref(),
                PageFailureKind::Driver { error, .. } if is_transient_mongo_error(error)
            )
    }

    pub(super) fn storage(error: StorageError) -> Self {
        Self {
            kind: Box::new(PageFailureKind::Storage(Arc::new(error))),
            permanent: HashMap::new(),
        }
    }

    /// Overlay known permanent failures that take precedence over those already
    /// attached to this later page failure. Callers add earlier sub-batches or
    /// destinations here; an own-row failure must replace a contained/metadata
    /// failure already present, so collisions deliberately use the incoming map.
    pub(super) fn with_permanent(mut self, failures: HashMap<usize, String>) -> Self {
        self.permanent.extend(failures);
        self
    }

    fn with_outcomes(
        mut self,
        extraction: Vec<Option<String>>,
        inserts: HashMap<usize, String>,
    ) -> Self {
        self = self.with_permanent(inserts);
        for (owner, message) in extraction.into_iter().enumerate() {
            if let Some(message) = message {
                self.permanent.insert(owner, message);
            }
        }
        self
    }

    fn classified(&self) -> StorageError {
        match self.kind.as_ref() {
            PageFailureKind::Driver { context, error } => {
                let classification = classify_mongodb_error(context, error.clone());
                let concern = match error.kind.as_ref() {
                    mongodb::error::ErrorKind::InsertMany(error) => {
                        error.write_concern_error.as_ref()
                    }
                    mongodb::error::ErrorKind::Write(
                        mongodb::error::WriteFailure::WriteConcernError(error),
                    ) => Some(error),
                    _ => None,
                };
                if concern.is_some_and(|error| {
                    matches!(error.code, 50 | 262)
                        || error
                            .details
                            .as_ref()
                            .and_then(|info| info.get_bool("wtimeout").ok())
                            == Some(true)
                }) {
                    return BackendError::Timeout {
                        backend_name: "mongodb".into(),
                        message: format!("{context}: {error}"),
                    }
                    .into();
                }
                // Keep server deadlines ahead of labels and leave the shared
                // CRUD/query classifier unchanged.
                if matches!(classification, BackendError::Internal { .. })
                    && (error.contains_label("RetryableWriteError")
                        || error.contains_label("RetryableError"))
                {
                    BackendError::Unavailable {
                        backend_name: "mongodb".into(),
                        message: format!("{context}: {error}"),
                    }
                    .into()
                } else {
                    classification.into()
                }
            }
            PageFailureKind::Metadata(reply) => {
                let concern = reply.get_document("writeConcernError").ok();
                let message = format!("Unacknowledged reindex missing metadata update: {reply}");
                let timeout = concern.is_some_and(|error| {
                    matches!(error.get_i32("code"), Ok(50 | 262))
                        || error
                            .get_document("errInfo")
                            .ok()
                            .and_then(|info| info.get_bool("wtimeout").ok())
                            == Some(true)
                });
                if timeout {
                    BackendError::Timeout {
                        backend_name: "mongodb".into(),
                        message,
                    }
                    .into()
                } else if has_retryable_label(reply) || concern.is_some_and(has_retryable_label) {
                    BackendError::Unavailable {
                        backend_name: "mongodb".into(),
                        message,
                    }
                    .into()
                } else {
                    internal_error(message)
                }
            }
            PageFailureKind::Internal(message) => internal_error(message.clone()),
            PageFailureKind::Storage(error) => copy_page_error(error),
        }
    }

    pub(super) fn fan_out(&self, n: usize) -> Vec<StorageResult<usize>> {
        // One page failure has one classification. StorageError is not Clone;
        // share its source chain while constructing each resource's outcome.
        let classified = match self.kind.as_ref() {
            PageFailureKind::Storage(error) => Arc::clone(error),
            _ => Arc::new(self.classified()),
        };
        (0..n)
            .map(|owner| {
                Err(match self.permanent.get(&owner) {
                    Some(message) => internal_error(message.clone()),
                    None => copy_page_error(&classified),
                })
            })
            .collect()
    }

    /// [`Self::fan_out`] for a metadata-only page (#2008): every resource
    /// fails and none had its metadata written.
    pub(super) fn fan_out_metadata(
        &self,
        n: usize,
    ) -> (
        Vec<StorageResult<()>>,
        u64,
        Vec<crate::search::ExtractionFailure>,
    ) {
        let results = self
            .fan_out(n)
            .into_iter()
            .map(|outcome| outcome.map(|_| ()))
            .collect();
        (results, 0, Vec::new())
    }
}

impl From<String> for PageFailure {
    fn from(message: String) -> Self {
        Self {
            kind: Box::new(PageFailureKind::Internal(message)),
            permanent: HashMap::new(),
        }
    }
}

impl From<&str> for PageFailure {
    fn from(message: &str) -> Self {
        Self::from(message.to_owned())
    }
}

/// Duplicate the classification, not the driver error or its source chain.
fn copy_page_error(error: &Arc<StorageError>) -> StorageError {
    match error.as_ref() {
        StorageError::Backend(BackendError::Unavailable {
            backend_name,
            message,
        }) => BackendError::Unavailable {
            backend_name: backend_name.clone(),
            message: message.clone(),
        }
        .into(),
        StorageError::Backend(BackendError::Timeout {
            backend_name,
            message,
        }) => BackendError::Timeout {
            backend_name: backend_name.clone(),
            message: message.clone(),
        }
        .into(),
        StorageError::Backend(BackendError::ConnectionFailed {
            backend_name,
            message,
        }) => BackendError::ConnectionFailed {
            backend_name: backend_name.clone(),
            message: message.clone(),
        }
        .into(),
        StorageError::Backend(BackendError::PoolExhausted { backend_name }) => {
            BackendError::PoolExhausted {
                backend_name: backend_name.clone(),
            }
            .into()
        }
        StorageError::Backend(BackendError::Internal {
            backend_name,
            message,
            source,
        }) => BackendError::Internal {
            backend_name: backend_name.clone(),
            message: message.clone(),
            // Wrapping the original can repeat its message but keeps its non-Clone source reachable.
            source: source
                .as_ref()
                .map(|_| Box::new(Arc::clone(error)) as Box<dyn std::error::Error + Send + Sync>),
        }
        .into(),
        _ => internal_error(error.to_string()),
    }
}

fn has_retryable_label(reply: &Document) -> bool {
    reply.get_array("errorLabels").is_ok_and(|labels| {
        labels.iter().any(|label| {
            matches!(
                label.as_str(),
                Some("RetryableWriteError" | "RetryableError")
            )
        })
    })
}

/// Parse the actual update-command reply, attributing statement errors to the
/// original page owners rather than the sub-batch's local statement positions.
fn metadata_update_result(
    reply: &Document,
    updates: &[(usize, Document)],
) -> Result<(usize, HashMap<usize, String>), PageFailure> {
    let mut failures = HashMap::new();
    if reply.contains_key("writeErrors") {
        let errors = reply
            .get_array("writeErrors")
            .map_err(|_| "Invalid reindex metadata write errors")?;
        for error in errors {
            let error = error
                .as_document()
                .ok_or("Invalid reindex metadata write error")?;
            let index = error
                .get_i32("index")
                .map_err(|_| "Missing reindex metadata error index")?;
            let owner = updates
                .get(usize::try_from(index).map_err(|_| "Invalid reindex metadata error index")?)
                .ok_or("Reindex metadata error index outside batch")?
                .0;
            failures.insert(owner, format!("Failed to update missing metadata: {error}"));
        }
    }
    if reply.contains_key("writeConcernError") {
        return Err(PageFailure {
            kind: Box::new(PageFailureKind::Metadata(reply.clone())),
            permanent: failures,
        });
    }
    let matched = reply
        .get_i64("n")
        .or_else(|_| reply.get_i32("n").map(i64::from))
        .map_err(|_| "Missing reindex metadata matched count")?;
    let matched = usize::try_from(matched).map_err(|_| "Invalid reindex metadata matched count")?;
    if matched > updates.len() {
        return Err("Reindex metadata matched count exceeds batch".into());
    }
    Ok((matched, failures))
}

/// Writes a page's `missing_params` once its search rows are written and
/// verified, leaving out every update the stored documents already satisfy
/// (#2005). Runs after verification so that verification's read of resource
/// versions stays the page's first read of `resources`. Returns per-resource
/// write errors; a command or acknowledgement error fails the page.
async fn write_page_metadata(
    db: &mongodb::Database,
    tenant_id: &str,
    updates: Vec<MissingUpdate>,
    stats: &mut ReindexPageStats,
) -> Result<HashMap<usize, String>, PageFailure> {
    let started = std::time::Instant::now();
    let resources = db.collection::<Document>(super::MongoBackend::RESOURCES_COLLECTION);
    let (updates, skipped) = changed_missing_updates(&resources, tenant_id, updates).await;
    stats.metadata_updates_skipped += skipped;
    let result = update_missing_metadata(db, &updates).await;
    stats.insert += started.elapsed();
    result
}

/// Update metadata in bounded commands, never overwriting a newer live version.
/// A nonmatch leaves superseded, deleted or absent resources alone. The existing
/// catch-up walk handles concurrent writes; command and acknowledgement errors fail.
async fn update_missing_metadata(
    db: &mongodb::Database,
    updates: &[(usize, Document)],
) -> Result<HashMap<usize, String>, PageFailure> {
    update_missing_metadata_counted(db, updates)
        .await
        .map(|(failures, _)| failures)
}

/// [`update_missing_metadata`], also returning how many resources the updates
/// matched: what a metadata-only run reports as written (#2008).
async fn update_missing_metadata_counted(
    db: &mongodb::Database,
    updates: &[(usize, Document)],
) -> Result<(HashMap<usize, String>, u64), PageFailure> {
    let mut failures = HashMap::new();
    let mut matched_total = 0u64;
    for chunk in updates.chunks(REINDEX_SUBBATCH_MAX) {
        let statements: Vec<_> = chunk.iter().map(|(_, update)| update.clone()).collect();
        let reply = db
            .run_command(doc! {
                "update": super::MongoBackend::RESOURCES_COLLECTION,
                "updates": statements,
                "ordered": false,
            })
            .await
            .map_err(|error| {
                PageFailure::driver("Failed to update reindex missing metadata", error)
                    .with_permanent(failures.clone())
            })?;
        let (matched, write_failures) = metadata_update_result(&reply, chunk)
            .map_err(|failure| failure.with_permanent(failures.clone()))?;
        matched_total += matched as u64;
        failures.extend(write_failures);
    }
    Ok((failures, matched_total))
}

/// One resource's metadata-only update (#2008): the rebuild's
/// [`SubBatchDocs::missing_updates`] statement, plus, with `skip_existing`, a
/// guard that leaves alone a resource whose `missing_params` already carries
/// this extraction's fingerprint (#1999). An unstamped array (written before
/// #1999) or a stale stamp is rewritten, since searches distrust both.
fn metadata_only_update(
    tenant_id: &str,
    resource: &StoredResource,
    missing_params: Option<&[String]>,
    missing_fp: Option<&str>,
    skip_existing: bool,
) -> Document {
    let mut filter = resource_version_filter(tenant_id, resource);
    if skip_existing {
        // Skip only arrays already stamped with this extraction's
        // fingerprint; unstamped (#1984-era) or stale stamps are rewritten.
        match missing_fp {
            Some(fp) => filter.insert(
                "$or",
                vec![
                    doc! { "missing_params": { "$exists": false } },
                    doc! { "missing_params_fp": { "$ne": fp } },
                ],
            ),
            None => filter.insert("missing_params", doc! { "$exists": false }),
        };
    }
    doc! {
        "q": filter,
        "u": super::storage::with_missing_metadata_update(doc! {}, missing_params, missing_fp),
        "multi": false,
        "upsert": false,
    }
}

/// What a database task hands back to the page's own thread when joined:
/// its own delete/insert phase timing and counts, and its outcome (#1403).
struct DbTask<T> {
    stats: ReindexPageStats,
    result: Result<T, PageFailure>,
}

/// Per-resource write errors for one sub-batch's insert, kept separate by
/// destination collection so [`merge_insert_failures`] can prefer an own-row
/// failure over a contained-row one (#1403).
struct InsertFailures {
    own: HashMap<usize, String>,
    contained: HashMap<usize, String>,
}

/// The `_id`s one page wrote, per collection, each with the page position of
/// the resource it belongs to. The page assigns them before the inserts, so
/// verification knows exactly which rows are its own whatever the insert
/// outcome was, and how many it wrote per resource.
#[derive(Default)]
struct InsertedIds {
    own: Vec<(usize, Bson)>,
    contained: Vec<(usize, Bson)>,
}

type ResourceKey = (String, String);
type ResourceRowIds = HashMap<ResourceKey, Vec<Bson>>;

/// Rows observed after both initial deletes and before any page insert.
/// Cleanup never acquires ownership of rows discovered after this snapshot.
#[derive(Default)]
struct PreviousIds {
    own: ResourceRowIds,
    contained: ResourceRowIds,
    /// Rows the page's initial deletes removed, across both collections.
    deleted: u64,
    /// A re-run of a page whose earlier run may have left an insert in
    /// flight: an empty snapshot no longer proves there is nothing to verify.
    force_verify: bool,
}

impl PreviousIds {
    /// The page found nothing to replace: its deletes removed no rows and
    /// none were left behind. A delete cannot miss rows that were never
    /// there, so post-insert verification has nothing to protect. This is
    /// the first build into an empty index, including the deferred rebuild
    /// after `$bulk-submit`.
    fn nothing_replaced(&self) -> bool {
        !self.force_verify && self.deleted == 0 && self.own.is_empty() && self.contained.is_empty()
    }
}

impl InsertedIds {
    fn extend(&mut self, other: InsertedIds) {
        self.own.extend(other.own);
        self.contained.extend(other.contained);
    }
}

/// One sub-batch's insert outcome: attributable failures and the ids written.
struct SubBatchInsert {
    failures: InsertFailures,
    inserted: InsertedIds,
}

/// Gives every document an `_id` now, so the page knows the ids it inserts
/// even when the driver reports an error for the command. `owners` holds the
/// page position of each document's resource.
fn assign_ids(docs: &mut [Document], owners: &[usize]) -> Vec<(usize, Bson)> {
    docs.iter_mut()
        .zip(owners)
        .map(|(doc, owner)| {
            let id = Bson::ObjectId(ObjectId::new());
            doc.insert("_id", id.clone());
            (*owner, id)
        })
        .collect()
}

/// Observed unresolved rows and resource-level verification failures. Missing
/// expected rows fail a resource without any rows remaining.
#[derive(Default)]
struct PageVerification {
    stale_rows_found: u64,
    stale_rows_remaining: u64,
    verification_failures: Vec<ResourceVerificationFailure>,
}

struct ResourceVerificationFailure {
    resource_type: String,
    resource_id: String,
    mismatch: RowMismatch,
}

/// Rows that verification could not account for after cleanup.
#[derive(Debug, PartialEq, Eq)]
struct RowMismatch {
    remaining: u64,
    unknown: u64,
    missing: u64,
}

/// Adds exactly `t`'s delete/insert phase fields to `stats`. Never calls
/// [`ReindexPageStats::accumulate`], which would fold busy time into
/// `db_wait`: the caller tracks `db_wait` itself, from the *join* wait via
/// [`add_db_wait`], not from a task's own busy time (#1403).
fn absorb_db_task(stats: &mut ReindexPageStats, t: &ReindexPageStats) {
    stats.delete += t.delete;
    stats.insert += t.insert;
    stats.deleted_entries += t.deleted_entries;
    stats.inserted_entries += t.inserted_entries;
    stats.insert_commands += t.insert_commands;
    stats.verify += t.verify;
    stats.stale_rows_found += t.stale_rows_found;
    stats.stale_rows_remaining += t.stale_rows_remaining;
}

/// Records `d` as time the page's thread spent waiting on a joined database
/// task, both in `stats.db_wait` and in the caller's running `page_wait`
/// total (#1403).
fn add_db_wait(stats: &mut ReindexPageStats, page_wait: &mut Duration, d: Duration) {
    stats.db_wait = Some(stats.db_wait.unwrap_or_default() + d);
    *page_wait += d;
}

/// Merges one sub-batch's insert failures into the page's running map,
/// keeping the first message recorded per resource: own-row failures first,
/// then contained-row failures (#1403).
fn merge_insert_failures(into: &mut HashMap<usize, String>, f: InsertFailures) {
    for (owner, msg) in f.own {
        into.entry(owner).or_insert(msg);
    }
    for (owner, msg) in f.contained {
        into.entry(owner).or_insert(msg);
    }
}

/// Combines each resource's extraction and insert outcomes into one result
/// per page-wide index: an extraction failure beats an insert failure, which
/// beats success (#1403).
fn page_outcomes(
    extract_failures: Vec<Option<String>>,
    mut insert_failures: HashMap<usize, String>,
    doc_counts: &[usize],
) -> Vec<StorageResult<usize>> {
    extract_failures
        .into_iter()
        .enumerate()
        .map(|(i, failure)| match failure {
            Some(msg) => Err(internal_error(msg)),
            None => match insert_failures.remove(&i) {
                Some(msg) => Err(internal_error(msg)),
                None => Ok(doc_counts[i]),
            },
        })
        .collect()
}

/// Fails the resources whose rows verification could not account for,
/// preserving earlier permanent errors.
fn apply_verification_outcomes(
    outcomes: &mut [StorageResult<usize>],
    verification_failures: Vec<ResourceVerificationFailure>,
    resources: &[StoredResource],
) {
    let positions: HashMap<(&str, &str), usize> = resources
        .iter()
        .enumerate()
        .map(|(i, resource)| ((resource.resource_type(), resource.id()), i))
        .collect();
    for stale in verification_failures {
        let key = (stale.resource_type.as_str(), stale.resource_id.as_str());
        if let Some(&i) = positions.get(&key)
            && outcomes[i].is_ok()
        {
            // Retryable: the bounded recovery rewrites the resource from a
            // fresh read, which is the repair for rows a cleanup could not remove.
            outcomes[i] = Err(BackendError::Unavailable {
                backend_name: "mongodb".into(),
                message: format!(
                    "search index mismatch for {}/{} after cleanup: {} unresolved rows, {} with uncertain ownership, {} missing expected rows",
                    stale.resource_type,
                    stale.resource_id,
                    stale.mismatch.remaining,
                    stale.mismatch.unknown,
                    stale.mismatch.missing
                ),
            }
            .into());
        }
    }
}

/// Turns a joined database task's `JoinError` into a page-level failure
/// message. A panic resumes here, exactly where an un-spawned call would
/// itself have panicked, so the panic still propagates instead of being
/// silently swallowed as an ordinary `Err` (#1403).
fn join_failure(e: tokio::task::JoinError, what: &str) -> PageFailure {
    if e.is_panic() {
        std::panic::resume_unwind(e.into_panic());
    }
    PageFailure::from(format!(
        "search index {what} task ended without a result: {e}"
    ))
}

/// Joins a spawned sub-batch insert, absorbing its stats into `stats` and its
/// wait time into `page_wait`, and folding its per-resource failures into
/// `insert_failures`. Used both to finish the previous sub-batch's insert
/// before starting the next one, and to drain the last one once every
/// sub-batch has been extracted. Returns the page-level failure message on a
/// join or insert error (#1403).
async fn join_insert(
    mut task: AbortOnDrop<DbTask<SubBatchInsert>>,
    stats: &mut ReindexPageStats,
    page_wait: &mut Duration,
    insert_failures: &mut HashMap<usize, String>,
    inserted: &mut InsertedIds,
) -> Result<(), PageFailure> {
    let waited = std::time::Instant::now();
    let joined = task.join().await;
    add_db_wait(stats, page_wait, waited.elapsed());
    let done = joined.map_err(|e| join_failure(e, "insert"))?;
    absorb_db_task(stats, &done.stats);
    match done.result {
        Ok(sub_batch) => {
            merge_insert_failures(insert_failures, sub_batch.failures);
            inserted.extend(sub_batch.inserted);
            Ok(())
        }
        Err(msg) => Err(msg),
    }
}

/// Records perf-phase durations and the inserted-document count for one page
/// whose delete and insert commands all completed without a page-level
/// error — both writers call this only then, never after a page-level
/// failure. Individual resources can still show up as extraction or insert
/// failures in the page's own returned outcomes; those per-resource
/// failures do not stop this call (#1403).
fn record_page_perf(stats: &ReindexPageStats, db_wait: Duration, docs: u64) {
    crate::perf::record_duration(crate::perf::Phase::ReindexExtract, stats.extract);
    crate::perf::record_duration(crate::perf::Phase::ReindexSearchDelete, stats.delete);
    crate::perf::record_duration(crate::perf::Phase::ReindexSearchInsert, stats.insert);
    crate::perf::record_duration(crate::perf::Phase::ReindexDbWait, db_wait);
    crate::perf::add_rows(crate::perf::Phase::ReindexSearchInsert, docs);
}

/// Aborts the spawned task, rather than merely dropping its handle, when this
/// wrapper goes away, so a run that stops mid-page (cancellation, a timeout
/// on the composite ingest sink) does not keep polling a delete or insert it
/// no longer needs. `abort()` only cancels the future client-side, at its
/// next yield point: a command the driver has already sent still runs to
/// completion on the MongoDB server (#1403).
pub(super) struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T: Send + 'static> AbortOnDrop<T> {
    fn spawn(fut: impl std::future::Future<Output = T> + Send + 'static) -> Self {
        Self(tokio::spawn(fut))
    }

    /// Awaits the task's result. Call at most once.
    async fn join(&mut self) -> Result<T, tokio::task::JoinError> {
        (&mut self.0).await
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Deletes a page's stale rows from both `search_index` and
/// `search_index_contained`: one `delete_many` per resource type in the page
/// (from `filters`), per collection. Runs before any insert is spawned for
/// the page. A delete failure reports `Err` for the whole page — the caller
/// cannot tell which resources' rows the delete would have reached, so it
/// fans that failure out to every resource rather than guessing (#1403).
async fn delete_page(
    own: mongodb::Collection<Document>,
    contained: mongodb::Collection<Document>,
    filters: Vec<Document>,
) -> DbTask<PreviousIds> {
    let started = std::time::Instant::now();
    let mut stats = ReindexPageStats::default();
    for filter in &filters {
        match own.delete_many(filter.clone()).await {
            Ok(result) => stats.deleted_entries += result.deleted_count,
            Err(e) => {
                stats.delete += started.elapsed();
                return DbTask {
                    stats,
                    result: Err(PageFailure::driver("Failed to delete search entries", e)),
                };
            }
        }
        match contained.delete_many(filter.clone()).await {
            Ok(result) => stats.deleted_entries += result.deleted_count,
            Err(e) => {
                stats.delete += started.elapsed();
                return DbTask {
                    stats,
                    result: Err(PageFailure::driver(
                        "Failed to delete search_index_contained entries",
                        e,
                    )),
                };
            }
        }
    }
    stats.delete += started.elapsed();
    let started = std::time::Instant::now();
    let result = async {
        let own = snapshot_row_ids(&own, &filters, "search index").await?;
        let contained = snapshot_row_ids(&contained, &filters, "search_index_contained").await?;
        Ok(PreviousIds {
            own,
            contained,
            deleted: stats.deleted_entries,
            force_verify: false,
        })
    }
    .await;
    stats.verify += started.elapsed();
    DbTask { stats, result }
}

async fn snapshot_row_ids(
    collection: &mongodb::Collection<Document>,
    filters: &[Document],
    context: &str,
) -> Result<ResourceRowIds, PageFailure> {
    let mut rows: ResourceRowIds = HashMap::new();
    for filter in filters {
        let mut cursor = collection
            .find(filter.clone())
            .projection(doc! { "_id": 1, "resource_type": 1, "resource_id": 1 })
            .await
            .map_err(|e| {
                PageFailure::driver(&format!("Failed to snapshot remaining {context} rows"), e)
            })?;
        while let Some(row) = cursor.try_next().await.map_err(|e| {
            PageFailure::driver(&format!("Failed to read remaining {context} rows"), e)
        })? {
            let key = (
                row.get_str("resource_type")
                    .map_err(|_| PageFailure::from("snapshot row without resource type"))?
                    .to_owned(),
                row.get_str("resource_id")
                    .map_err(|_| PageFailure::from("snapshot row without resource id"))?
                    .to_owned(),
            );
            let id = row.get("_id").ok_or("snapshot row without _id")?.clone();
            rows.entry(key).or_default().push(id);
        }
    }
    Ok(rows)
}

/// Inserts one sub-batch's extracted documents: `docs.own` into `own`
/// (`search_index`), then, if any, `docs.contained` into `contained`
/// (`search_index_contained`), each via
/// [`super::storage::insert_search_entries_chunk`]. A page-level error from
/// the own-collection insert returns as `Err` immediately, before the
/// contained collection is ever attempted; a page-level error from the
/// contained-collection insert instead returns only after the own insert has
/// already run (and succeeded). Per-document write errors instead come back
/// as [`InsertFailures`], attributable to individual resources (#1403).
async fn insert_sub_batch(
    own: mongodb::Collection<Document>,
    contained: mongodb::Collection<Document>,
    mut docs: SubBatchDocs,
) -> DbTask<SubBatchInsert> {
    let mut stats = ReindexPageStats::default();
    let inserted = InsertedIds {
        own: assign_ids(&mut docs.own_docs, &docs.own_owners),
        contained: assign_ids(&mut docs.contained_docs, &docs.contained_owners),
    };
    let started = std::time::Instant::now();
    let own_result = super::storage::insert_search_entries_chunk(
        &own,
        &docs.own_owners,
        &docs.own_docs,
        "Failed to insert search index entries",
        &mut stats,
    )
    .await;
    stats.insert += started.elapsed();
    let own_failures = match own_result {
        Ok(failures) => failures,
        Err(msg) => {
            return DbTask {
                stats,
                result: Err(msg),
            };
        }
    };
    let mut contained_failures = HashMap::new();
    if !docs.contained_docs.is_empty() {
        let started = std::time::Instant::now();
        let contained_result = super::storage::insert_search_entries_chunk(
            &contained,
            &docs.contained_owners,
            &docs.contained_docs,
            "Failed to insert search_index_contained entries",
            &mut stats,
        )
        .await;
        stats.insert += started.elapsed();
        contained_failures = match contained_result {
            Ok(failures) => failures,
            Err(msg) => {
                return DbTask {
                    stats,
                    result: Err(msg.with_permanent(own_failures)),
                };
            }
        };
    }
    DbTask {
        stats,
        result: Ok(SubBatchInsert {
            failures: InsertFailures {
                own: own_failures,
                contained: contained_failures,
            },
            inserted,
        }),
    }
}

/// Verifies ownership after all page inserts. The pre-insert snapshot bounds
/// cleanup ownership; rows discovered here never become deletion candidates.
async fn verify_page(
    own: mongodb::Collection<Document>,
    contained: mongodb::Collection<Document>,
    resources: mongodb::Collection<Document>,
    page: &PageRows<'_>,
    inserted: &InsertedIds,
    previous: &PreviousIds,
) -> DbTask<PageVerification> {
    let started = std::time::Instant::now();
    let mut stats = ReindexPageStats::default();
    let mut verification = PageVerification::default();
    let result = async {
        // Read versions after the survivor snapshot and all page writes. This
        // also selects changed/deleted owners whose row count did not grow.
        let states = resource_states(&resources, page).await?;
        for index in [
            IndexCollection {
                collection: &own,
                inserted: &inserted.own,
                previous: &previous.own,
                context: "search index",
            },
            IndexCollection {
                collection: &contained,
                inserted: &inserted.contained,
                previous: &previous.contained,
                context: "search_index_contained",
            },
        ] {
            verify_collection(index, &resources, page, &states, &mut verification).await?;
        }
        Ok(verification)
    }
    .await;
    stats.verify += started.elapsed();
    if let Ok(verification) = &result {
        stats.stale_rows_found = verification.stale_rows_found;
        stats.stale_rows_remaining = verification.stale_rows_remaining;
        if verification.stale_rows_found > 0 || verification.stale_rows_remaining > 0 {
            tracing::warn!(
                stale_rows_found = verification.stale_rows_found,
                stale_rows_remaining = verification.stale_rows_remaining,
                "reindex page found stale or uncertain rows"
            );
        }
    }
    DbTask { stats, result }
}

/// The page's resources as verification sees them: scope filters per type, the
/// resources with the versions the page read, and the positions whose insert
/// failed (a failed owner is reported already and is not verified).
struct PageRows<'a> {
    filters: &'a [Document],
    tenant_id: &'a str,
    resources: &'a [StoredResource],
    failed: &'a HashMap<usize, String>,
}

impl PageRows<'_> {
    fn position_of(&self, resource_type: &str, id: &str) -> Option<usize> {
        self.resources
            .iter()
            .position(|resource| resource.resource_type() == resource_type && resource.id() == id)
    }
}

/// One index collection as verification sees it: the rows this page wrote
/// into it and the rows left in it after the page's delete.
struct IndexCollection<'a> {
    collection: &'a mongodb::Collection<Document>,
    inserted: &'a [(usize, Bson)],
    previous: &'a ResourceRowIds,
    context: &'a str,
}

async fn verify_collection(
    index: IndexCollection<'_>,
    resources: &mongodb::Collection<Document>,
    page: &PageRows<'_>,
    states: &[ResourceState],
    verification: &mut PageVerification,
) -> Result<(), PageFailure> {
    let IndexCollection {
        collection,
        inserted,
        previous,
        context,
    } = index;
    let mut wrote: HashMap<usize, Vec<Bson>> = HashMap::new();
    for (owner, id) in inserted {
        wrote.entry(*owner).or_default().push(id.clone());
    }
    let mut present = snapshot_row_ids(collection, page.filters, context).await?;
    for (resource_type, id) in present.keys() {
        page.position_of(resource_type, id)
            .ok_or("index snapshot contains an owner outside the page")?;
    }
    for (position, resource) in page.resources.iter().enumerate() {
        if page.failed.contains_key(&position) {
            continue;
        }
        let key = (
            resource.resource_type().to_owned(),
            resource.id().to_owned(),
        );
        let old_ids = previous.get(&key).map(Vec::as_slice).unwrap_or_default();
        let page_ids = wrote.get(&position).map(Vec::as_slice).unwrap_or_default();
        let initial = classify_cleanup_rows(
            present.remove(&key).unwrap_or_default(),
            page_ids,
            old_ids,
            states[position],
        );
        let scope = doc! {
            "tenant_id": page.tenant_id,
            "resource_type": resource.resource_type(),
            "resource_id": resource.id(),
        };
        let mut cleanup = clean_resource(
            collection,
            &scope,
            page_ids,
            old_ids,
            states[position],
            context,
            initial,
        )
        .await?;
        if cleanup.mismatch.is_some()
            && matches!(
                states[position],
                ResourceState::Absent | ResourceState::Unchanged
            )
        {
            // A later update can change row ownership. Already changed/deleted
            // resources keep the same ownership and must not restart cleanup.
            let state = resource_state_now(resources, page.tenant_id, resource).await?;
            if matches!(state, ResourceState::Changed | ResourceState::Deleted) {
                let rows = classify_cleanup_rows(
                    row_ids(collection, &scope, context).await?,
                    page_ids,
                    old_ids,
                    state,
                );
                let again =
                    clean_resource(collection, &scope, page_ids, old_ids, state, context, rows)
                        .await?;
                cleanup = CleanupResult {
                    found: cleanup.found.max(again.found),
                    remaining: again.remaining,
                    mismatch: again.mismatch,
                };
            }
        }
        verification.stale_rows_found += cleanup.found;
        verification.stale_rows_remaining += cleanup.remaining;
        if let Some(mismatch) = cleanup.mismatch {
            verification
                .verification_failures
                .push(ResourceVerificationFailure {
                    resource_type: resource.resource_type().to_owned(),
                    resource_id: resource.id().to_owned(),
                    mismatch,
                });
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResourceState {
    /// ReindexTarget can receive resources from another store. Local absence
    /// is not a deletion signal; the supplied resource remains authoritative.
    Absent,
    Unchanged,
    Changed,
    Deleted,
}

/// Batched resource reads, one per type, rather than one query per owner.
async fn resource_states(
    resources: &mongodb::Collection<Document>,
    page: &PageRows<'_>,
) -> Result<Vec<ResourceState>, PageFailure> {
    // A hard purge is indistinguishable from an external source here. Only
    // an explicit local tombstone can authorize deletion of this page's IDs.
    let mut states = vec![ResourceState::Absent; page.resources.len()];
    for filter in page.filters {
        let mut filter = filter.clone();
        let ids = filter
            .remove("resource_id")
            .ok_or("page filter without resource ids")?;
        filter.insert("id", ids);
        let mut cursor = resources
            .find(filter)
            .projection(doc! { "resource_type": 1, "id": 1, "version_id": 1, "is_deleted": 1 })
            .await
            .map_err(|e| PageFailure::driver("Failed to read page resource versions", e))?;
        while let Some(row) = cursor
            .try_next()
            .await
            .map_err(|e| PageFailure::driver("Failed to read page resource version cursor", e))?
        {
            let resource_type = row
                .get_str("resource_type")
                .map_err(|_| "resource without type")?;
            let id = row.get_str("id").map_err(|_| "resource without id")?;
            let position = page
                .position_of(resource_type, id)
                .ok_or("resource version contains an owner outside the page")?;
            states[position] = resource_state(Some(&row), page.resources[position].version_id());
        }
    }
    Ok(states)
}

/// The state of one resource right now, for a mismatch found after the
/// page's batched version read.
async fn resource_state_now(
    resources: &mongodb::Collection<Document>,
    tenant_id: &str,
    resource: &StoredResource,
) -> Result<ResourceState, PageFailure> {
    let current = resources
        .find_one(doc! {
            "tenant_id": tenant_id,
            "resource_type": resource.resource_type(),
            "id": resource.id(),
        })
        .projection(doc! { "version_id": 1, "is_deleted": 1 })
        .await
        .map_err(|e| PageFailure::driver("Failed to re-read the resource version", e))?;
    Ok(resource_state(current.as_ref(), resource.version_id()))
}

/// A document without a readable version is treated like an absent one: the
/// page's copy stays authoritative.
fn resource_state(current: Option<&Document>, page_version: &str) -> ResourceState {
    let Some(current) = current else {
        return ResourceState::Absent;
    };
    if current.get_bool("is_deleted").unwrap_or(false) {
        return ResourceState::Deleted;
    }
    match current.get_str("version_id").ok() {
        None => ResourceState::Absent,
        Some(version) if version == page_version => ResourceState::Unchanged,
        Some(_) => ResourceState::Changed,
    }
}

struct CleanupRows {
    removable: Vec<Bson>,
    unknown: u64,
    missing: u64,
}

impl CleanupRows {
    fn unresolved(&self) -> u64 {
        self.removable.len() as u64 + self.unknown
    }

    /// Changed/deleted resources may retain another writer's rows, but never
    /// report clean while this page's obsolete rows remain.
    fn mismatch(&self, state: ResourceState) -> Option<RowMismatch> {
        match state {
            ResourceState::Changed | ResourceState::Deleted if !self.removable.is_empty() => {
                Some(RowMismatch {
                    remaining: self.removable.len() as u64,
                    unknown: 0,
                    missing: 0,
                })
            }
            ResourceState::Changed | ResourceState::Deleted => None,
            ResourceState::Absent | ResourceState::Unchanged
                if self.unresolved() > 0 || self.missing > 0 =>
            {
                Some(RowMismatch {
                    remaining: self.unresolved(),
                    unknown: self.unknown,
                    missing: self.missing,
                })
            }
            ResourceState::Absent | ResourceState::Unchanged => None,
        }
    }
}

/// Exact IDs captured before the page's inserts are the only older generation
/// it can remove. After a change only the page's own IDs are removed; the rest
/// belong to the newer version or to a generation its rewrite clears. Foreign
/// rows first seen after the inserts are never claimed by cleanup.
fn classify_cleanup_rows(
    ids: Vec<Bson>,
    page_ids: &[Bson],
    previous: &[Bson],
    state: ResourceState,
) -> CleanupRows {
    let mut result = CleanupRows {
        removable: Vec::new(),
        unknown: 0,
        missing: if matches!(state, ResourceState::Absent | ResourceState::Unchanged) {
            page_ids.iter().filter(|id| !ids.contains(id)).count() as u64
        } else {
            0
        },
    };
    for id in ids {
        let ours = page_ids.contains(&id);
        let captured = previous.contains(&id);
        let removable = match state {
            ResourceState::Absent | ResourceState::Unchanged => captured,
            ResourceState::Changed | ResourceState::Deleted => ours,
        };
        if removable {
            result.removable.push(id);
        } else if matches!(state, ResourceState::Absent | ResourceState::Unchanged) && !ours {
            result.unknown += 1;
        }
    }
    result
}

struct CleanupResult {
    found: u64,
    remaining: u64,
    mismatch: Option<RowMismatch>,
}

/// Retries only deletion of IDs with established provenance. Re-reading the
/// collection can identify an unresolved conflict, but cannot expand ownership.
async fn clean_resource(
    collection: &mongodb::Collection<Document>,
    scope: &Document,
    page_ids: &[Bson],
    previous: &[Bson],
    state: ResourceState,
    context: &str,
    mut rows: CleanupRows,
) -> Result<CleanupResult, PageFailure> {
    let mut found = 0;
    let mut attempt = 0;
    loop {
        found = found.max(rows.unresolved());
        if rows.removable.is_empty() || attempt == VERIFY_CLEANUP_ATTEMPTS {
            return Ok(CleanupResult {
                found,
                remaining: rows.unresolved(),
                mismatch: rows.mismatch(state),
            });
        }
        if attempt > 0 {
            tokio::time::sleep(VERIFY_CLEANUP_BACKOFF * attempt).await;
        }
        // Keep command size bounded even for resources with many search rows.
        for ids in rows.removable.chunks(1000) {
            let mut filter = scope.clone();
            filter.insert("_id", doc! { "$in": ids.to_vec() });
            collection.delete_many(filter).await.map_err(|e| {
                PageFailure::driver(&format!("Failed to remove stale {context} rows"), e)
            })?;
        }
        rows = classify_cleanup_rows(
            row_ids(collection, scope, context).await?,
            page_ids,
            previous,
            state,
        );
        attempt += 1;
    }
}

async fn row_ids(
    collection: &mongodb::Collection<Document>,
    scope: &Document,
    context: &str,
) -> Result<Vec<Bson>, PageFailure> {
    let mut cursor = collection
        .find(scope.clone())
        .projection(doc! { "_id": 1 })
        .await
        .map_err(|e| PageFailure::driver(&format!("Failed to list stale {context} rows"), e))?;
    let mut ids = Vec::new();
    while let Some(row) = cursor
        .try_next()
        .await
        .map_err(|e| PageFailure::driver(&format!("Failed to read stale {context} rows"), e))?
    {
        ids.push(
            row.get("_id")
                .ok_or("search index row without _id")?
                .clone(),
        );
    }
    Ok(ids)
}

/// Verifies a page after inserts and returns resources requiring recovery.
async fn verify_clean_page(
    own: mongodb::Collection<Document>,
    contained: mongodb::Collection<Document>,
    resources: mongodb::Collection<Document>,
    page: &PageRows<'_>,
    inserted: &InsertedIds,
    previous: &PreviousIds,
    stats: &mut ReindexPageStats,
) -> Result<Vec<ResourceVerificationFailure>, PageFailure> {
    // Skipping trades away detection of a concurrent update or delete
    // racing this page, which the job's catch-up rounds handle, as before
    // verification existed.
    if previous.nothing_replaced() {
        return Ok(Vec::new());
    }
    let done = verify_page(own, contained, resources, page, inserted, previous).await;
    absorb_db_task(stats, &done.stats);
    done.result
        .map(|verification| verification.verification_failures)
}

impl super::MongoBackend {
    /// Extracts one resource's search-index documents (#1403).
    fn extract_one(
        &self,
        tenant_id: &str,
        r: &StoredResource,
    ) -> (SearchIndexDocuments, ExtractionReport) {
        self.search_index_documents_checked(tenant_id, r.resource_type(), r.id(), r.content())
    }

    /// Metadata-only page write (#2008): extracts every resource exactly as a
    /// rebuild does, then writes only its `missing_params`, under the same
    /// version guard, and never touches `search_index` or
    /// `search_index_contained`. A resource whose whole extraction failed has
    /// its metadata removed rather than claimed, as in a rebuild, and is
    /// reported as failed; a single failing parameter leaves the resource OK
    /// and is tallied as a warning (#2002). Returns the per-resource outcomes,
    /// how many resources the updates matched, and the parameter failures.
    pub(super) async fn write_metadata_page(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resources: &[StoredResource],
        skip_existing: bool,
    ) -> Result<
        (
            Vec<StorageResult<()>>,
            u64,
            Vec<crate::search::ExtractionFailure>,
        ),
        PageFailure,
    > {
        let n = resources.len();
        let prepare = |i: usize| self.extract_one(tenant_id, &resources[i]);
        let pool = if tokio_multi_thread_runtime() && n >= REINDEX_SERIAL_POOL_MIN_PAGE {
            self.reindex_prepare_pool()
        } else {
            None
        };
        let prepared: Vec<(SearchIndexDocuments, ExtractionReport)> = if pool.is_some() {
            let env = PrepareEnv {
                pool,
                gate: self.reindex_prepare_gate(),
            };
            extract_range(&env, 0..n, &prepare).0
        } else {
            (0..n).map(&prepare).collect()
        };

        let mut extract_failures: Vec<Option<String>> = Vec::with_capacity(n);
        let mut parameter_failures: Vec<crate::search::ExtractionFailure> = Vec::new();
        let mut updates: Vec<(usize, Document)> = Vec::with_capacity(n);
        for (i, (docs, report)) in prepared.into_iter().enumerate() {
            updates.push((
                i,
                metadata_only_update(
                    tenant_id,
                    &resources[i],
                    docs.missing_params.as_deref(),
                    docs.missing_fp.as_deref(),
                    skip_existing,
                ),
            ));
            match report {
                ExtractionReport::Clean => extract_failures.push(None),
                ExtractionReport::FailedParameters(failures) => {
                    extract_failures.push(None);
                    parameter_failures.extend(failures);
                }
                ExtractionReport::FailedResource(message) => {
                    extract_failures.push(Some(message));
                }
            }
        }

        let (write_failures, matched) = update_missing_metadata_counted(db, &updates)
            .await
            .map_err(|failure| failure.with_outcomes(extract_failures.clone(), HashMap::new()))?;
        let outcomes = page_outcomes(extract_failures, write_failures, &vec![0; n])
            .into_iter()
            .map(|outcome| outcome.map(|_| ()))
            .collect();
        Ok((outcomes, matched, parameter_failures))
    }

    /// Writes one page, running it again from its delete when a run fails
    /// transiently (#2290). Every run extracts the page afresh, deletes its
    /// rows by resource id (which removes what the failed run landed), takes a
    /// new snapshot and assigns new `_id`s, so the failed run's insert batch is
    /// never resent. A run that fails with a document-level (permanent)
    /// outcome, or with anything but a transient driver error, ends the loop.
    ///
    /// A command of the failed run that the server applies after the next
    /// run's snapshot leaves a row neither run owns. Every re-run therefore
    /// verifies even when its delete found nothing, so the resource fails as
    /// retryable instead of keeping a silent duplicate. The pause before a
    /// re-run narrows that window. Nothing here observes cancellation (the
    /// writer has no token); the policy bounds the total sleep, and dropping
    /// the future ends the loop.
    pub(super) async fn write_page_with_retry(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resources: &[StoredResource],
        stats: &mut ReindexPageStats,
        multi_thread: bool,
        overlapped: bool,
    ) -> Result<Vec<StorageResult<usize>>, PageFailure> {
        let mut attempt = 1;
        loop {
            // Each run's timings add up, so the page reports what it cost.
            let mut run = ReindexPageStats::default();
            let outcome = if overlapped {
                self.write_page_overlapped(db, tenant_id, resources, &mut run, attempt > 1)
                    .await
            } else {
                self.write_page_serial(
                    db,
                    tenant_id,
                    resources,
                    &mut run,
                    multi_thread,
                    attempt > 1,
                )
                .await
            };
            stats.accumulate(&run);
            match outcome {
                Err(failure)
                    if attempt < REINDEX_PAGE_RETRY.max_attempts && failure.is_transient_page() =>
                {
                    let backoff = REINDEX_PAGE_RETRY.backoff(attempt);
                    let pause = backoff / 2 + full_jitter(backoff / 2, jitter_fraction());
                    tracing::warn!(
                        attempt,
                        max_attempts = REINDEX_PAGE_RETRY.max_attempts,
                        pause_ms = pause.as_millis() as u64,
                        resources = resources.len(),
                        "transient mongodb error writing a reindex page; running the page again: {}",
                        failure.classified()
                    );
                    tokio::time::sleep(pause).await;
                    attempt += 1;
                }
                other => return other,
            }
        }
    }

    /// Serial writer: extracts every resource, deletes the page's stale
    /// entries, then inserts the new ones, one command at a time (own rows,
    /// then contained rows). Uses the rayon pool only for pages of at least
    /// `REINDEX_SERIAL_POOL_MIN_PAGE` on a multi-thread runtime; the pool
    /// accessor is never called otherwise, so single-resource
    /// `write_search_entries` calls and current-thread tests never build one
    /// (#1403).
    pub(super) async fn write_page_serial(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resources: &[StoredResource],
        stats: &mut ReindexPageStats,
        multi_thread: bool,
        replay: bool,
    ) -> Result<Vec<StorageResult<usize>>, PageFailure> {
        let n = resources.len();

        let own = db.collection::<Document>(Self::SEARCH_INDEX_COLLECTION);
        let contained = db.collection::<Document>(Self::SEARCH_INDEX_CONTAINED_COLLECTION);
        let prepare = |i: usize| self.extract_one(tenant_id, &resources[i]);

        let started = std::time::Instant::now();
        let pool = if multi_thread && n >= REINDEX_SERIAL_POOL_MIN_PAGE {
            self.reindex_prepare_pool()
        } else {
            None
        };
        let (prepared, on_pool) = if pool.is_some() {
            let env = PrepareEnv {
                pool,
                gate: self.reindex_prepare_gate(),
            };
            extract_range(&env, 0..n, &prepare)
        } else {
            ((0..n).map(&prepare).collect(), false)
        };
        let mut extract_failures: Vec<Option<String>> = vec![None; n];
        let mut parameter_failures: Vec<crate::search::ExtractionFailure> = Vec::new();
        let mut doc_counts: Vec<usize> = vec![0; n];
        let mut batch = flatten_sub_batch(
            0,
            prepared,
            &mut extract_failures,
            &mut parameter_failures,
            &mut doc_counts,
        );
        let missing_updates = batch.missing_updates(tenant_id, resources);
        stats.extract += started.elapsed();
        stats.sub_batches += 1;
        stats.pool_sub_batches += u64::from(on_pool);

        let filters = delete_filters(tenant_id, resources);
        let done = delete_page(own.clone(), contained.clone(), filters.clone()).await;
        absorb_db_task(stats, &done.stats);
        let previous = match done.result {
            Ok(previous) => PreviousIds {
                force_verify: replay,
                ..previous
            },
            Err(msg) => {
                stats.db_wait = Some(stats.delete + stats.insert + stats.verify);
                return Err(msg.with_outcomes(extract_failures, HashMap::new()));
            }
        };

        let done = insert_sub_batch(own.clone(), contained.clone(), batch).await;
        absorb_db_task(stats, &done.stats);
        stats.db_wait = Some(stats.delete + stats.insert + stats.verify);
        let (mut insert_failures, inserted) = match done.result {
            Ok(sub_batch) => {
                let mut into = HashMap::new();
                merge_insert_failures(&mut into, sub_batch.failures);
                (into, sub_batch.inserted)
            }
            Err(msg) => return Err(msg.with_outcomes(extract_failures, HashMap::new())),
        };
        let page = PageRows {
            filters: &filters,
            tenant_id,
            resources,
            failed: &insert_failures,
        };
        let resources_collection = db.collection::<Document>(Self::RESOURCES_COLLECTION);
        let verification_failures = verify_clean_page(
            own,
            contained,
            resources_collection,
            &page,
            &inserted,
            &previous,
            stats,
        )
        .await;
        stats.db_wait = Some(stats.delete + stats.insert + stats.verify);
        let verification_failures = verification_failures
            .map_err(|msg| msg.with_outcomes(extract_failures.clone(), insert_failures.clone()))?;
        let metadata_failures = write_page_metadata(db, tenant_id, missing_updates, stats).await;
        stats.db_wait = Some(stats.delete + stats.insert + stats.verify);
        for (owner, message) in metadata_failures
            .map_err(|msg| msg.with_outcomes(extract_failures.clone(), insert_failures.clone()))?
        {
            insert_failures.entry(owner).or_insert(message);
        }

        let docs: u64 = doc_counts.iter().sum::<usize>() as u64;
        record_page_perf(stats, stats.db_wait.unwrap_or_default(), docs);
        stats.record_parameter_failures(parameter_failures);
        let mut outcomes = page_outcomes(extract_failures, insert_failures, &doc_counts);
        apply_verification_outcomes(&mut outcomes, verification_failures, resources);
        Ok(outcomes)
    }

    /// Overlapped writer (#1403): while sub-batch k's insert runs on a
    /// spawned task, the page's own thread extracts sub-batch k+1 inside
    /// `block_in_place`; the page's delete runs on its own task while
    /// sub-batch 1 is extracted. Caller guarantees a multi-thread runtime,
    /// `resources.len() > REINDEX_SUBBATCH_FIRST`, and search not offloaded.
    /// At most one insert is ever in flight: every loop iteration joins the
    /// previous sub-batch's insert before spawning the next, and the
    /// trailing drain after the loop joins the last one.
    pub(super) async fn write_page_overlapped(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resources: &[StoredResource],
        stats: &mut ReindexPageStats,
        replay: bool,
    ) -> Result<Vec<StorageResult<usize>>, PageFailure> {
        let n = resources.len();

        let own = db.collection::<Document>(Self::SEARCH_INDEX_COLLECTION);
        let contained = db.collection::<Document>(Self::SEARCH_INDEX_CONTAINED_COLLECTION);
        let pool = self.reindex_prepare_pool();
        let env = PrepareEnv {
            pool,
            gate: self.reindex_prepare_gate(),
        };
        let min_size = if pool.is_some() {
            resolve_prepare_width(self.config().reindex_prepare_threads)
        } else {
            1
        };
        let filters = delete_filters(tenant_id, resources);
        let page_type: Option<&str> = (filters.len() == 1).then(|| resources[0].resource_type());
        let seed = page_type.and_then(|t| self.reindex_docs_seed(t));
        let prepare = |i: usize| self.extract_one(tenant_id, &resources[i]);

        // The page's delete runs on its own task while sub-batch 1 is
        // extracted below.
        let mut delete = Some(AbortOnDrop::spawn(delete_page(
            own.clone(),
            contained.clone(),
            filters.clone(),
        )));
        let mut planner = SubBatchPlanner::new(n, seed, min_size);
        let mut extract_failures: Vec<Option<String>> = vec![None; n];
        let mut doc_counts: Vec<usize> = vec![0; n];
        let mut insert_failures: HashMap<usize, String> = HashMap::new();
        let mut parameter_failures: Vec<crate::search::ExtractionFailure> = Vec::new();
        let mut inserted = InsertedIds::default();
        let mut previous = PreviousIds::default();
        let mut in_flight: Option<AbortOnDrop<DbTask<SubBatchInsert>>> = None;
        let mut page_wait = Duration::ZERO;
        let mut missing_updates = Vec::with_capacity(n);

        while let Some(range) = planner.next_range() {
            // Extract this sub-batch; it overlaps the delete, or the
            // previous sub-batch's insert.
            let started = std::time::Instant::now();
            let (prepared, on_pool) = extract_range(&env, range.clone(), &prepare);
            let mut batch = flatten_sub_batch(
                range.start,
                prepared,
                &mut extract_failures,
                &mut parameter_failures,
                &mut doc_counts,
            );
            missing_updates.extend(batch.missing_updates(tenant_id, resources));
            stats.extract += started.elapsed();
            stats.sub_batches += 1;
            stats.pool_sub_batches += u64::from(on_pool);
            planner.record(range.len(), batch.len());

            // No insert may start before the page's delete has finished.
            if let Some(mut task) = delete.take() {
                let waited = std::time::Instant::now();
                let joined = task.join().await;
                add_db_wait(stats, &mut page_wait, waited.elapsed());
                let done = match joined {
                    Ok(done) => done,
                    Err(e) => {
                        return Err(join_failure(e, "delete")
                            .with_outcomes(extract_failures, insert_failures));
                    }
                };
                absorb_db_task(stats, &done.stats);
                previous = match done.result {
                    Ok(previous) => PreviousIds {
                        force_verify: replay,
                        ..previous
                    },
                    Err(msg) => return Err(msg.with_outcomes(extract_failures, insert_failures)),
                };
            }
            // At most one insert is ever in flight: finish the previous
            // sub-batch's insert before starting this one.
            if let Some(task) = in_flight.take()
                && let Err(msg) = join_insert(
                    task,
                    stats,
                    &mut page_wait,
                    &mut insert_failures,
                    &mut inserted,
                )
                .await
            {
                return Err(msg.with_outcomes(extract_failures, insert_failures));
            }
            in_flight = Some(AbortOnDrop::spawn(insert_sub_batch(
                own.clone(),
                contained.clone(),
                batch,
            )));
        }
        // Drain the last sub-batch's insert the same way.
        if let Some(task) = in_flight.take()
            && let Err(msg) = join_insert(
                task,
                stats,
                &mut page_wait,
                &mut insert_failures,
                &mut inserted,
            )
            .await
        {
            return Err(msg.with_outcomes(extract_failures, insert_failures));
        }
        let waited = std::time::Instant::now();
        let page = PageRows {
            filters: &filters,
            tenant_id,
            resources,
            failed: &insert_failures,
        };
        let resources_collection = db.collection::<Document>(Self::RESOURCES_COLLECTION);
        let verification_failures = verify_clean_page(
            own,
            contained,
            resources_collection,
            &page,
            &inserted,
            &previous,
            stats,
        )
        .await;
        add_db_wait(stats, &mut page_wait, waited.elapsed());
        let verification_failures = verification_failures
            .map_err(|msg| msg.with_outcomes(extract_failures.clone(), insert_failures.clone()))?;
        let waited = std::time::Instant::now();
        let metadata_failures = write_page_metadata(db, tenant_id, missing_updates, stats).await;
        add_db_wait(stats, &mut page_wait, waited.elapsed());
        for (owner, message) in metadata_failures
            .map_err(|msg| msg.with_outcomes(extract_failures.clone(), insert_failures.clone()))?
        {
            insert_failures.entry(owner).or_insert(message);
        }

        // Success only: remember the type's documents per resource, forward to perf.
        let docs: u64 = doc_counts.iter().sum::<usize>() as u64;
        if let Some(t) = page_type {
            self.record_reindex_docs(t, n as u64, docs);
        }
        record_page_perf(stats, page_wait, docs);
        stats.record_parameter_failures(parameter_failures);
        let mut outcomes = page_outcomes(extract_failures, insert_failures, &doc_counts);
        apply_verification_outcomes(&mut outcomes, verification_failures, resources);
        Ok(outcomes)
    }

    /// Logs `mongodb reindex writer configuration` once per backend instance,
    /// the first time a page of more than `REINDEX_SUBBATCH_FIRST` resources
    /// reaches the dispatcher (#1403).
    pub(super) fn log_reindex_mode_once(&self, multi_thread: bool, overlapped: bool) {
        if self
            .reindex_mode_logged()
            .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        let prepare_threads_configured = self.config().reindex_prepare_threads;
        let prepare_threads = resolve_prepare_width(prepare_threads_configured);
        let pool = if prepare_threads < 2 {
            "none"
        } else if !multi_thread {
            "unused"
        } else if self.reindex_prepare_pool().is_some() {
            "ready"
        } else {
            "unavailable"
        };
        let path = if overlapped { "overlapped" } else { "serial" };
        tracing::info!(
            overlap = self.config().reindex_overlap,
            prefetch = self.config().reindex_prefetch,
            prepare_threads_configured,
            prepare_threads,
            pool = %pool,
            multi_thread_runtime = multi_thread,
            path = %path,
            "mongodb reindex writer configuration"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_error(code: i32) -> mongodb::error::Error {
        let command: mongodb::error::CommandError = mongodb::bson::from_document(doc! {
            "code": code, "codeName": "test", "errmsg": "synthetic command failure"
        })
        .unwrap();
        mongodb::error::ErrorKind::Command(command).into()
    }

    fn concern_error(label: &str) -> mongodb::error::Error {
        let concern: mongodb::error::WriteConcernError = mongodb::bson::from_document(doc! {
            "code": 91, "errmsg": "synthetic acknowledgement failure", "errorLabels": [label]
        })
        .unwrap();
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteConcernError(concern))
            .into()
    }

    #[test]
    fn verification_is_skipped_only_when_the_page_replaced_nothing() {
        let key = || ("Patient".to_string(), "p1".to_string());
        let id = || vec![Bson::ObjectId(ObjectId::new())];
        assert!(PreviousIds::default().nothing_replaced());
        let deleted = PreviousIds {
            deleted: 1,
            ..Default::default()
        };
        assert!(!deleted.nothing_replaced());
        let own_left = PreviousIds {
            own: HashMap::from([(key(), id())]),
            ..Default::default()
        };
        assert!(!own_left.nothing_replaced());
        let contained_left = PreviousIds {
            contained: HashMap::from([(key(), id())]),
            ..Default::default()
        };
        assert!(!contained_left.nothing_replaced());
    }

    #[test]
    fn page_failures_classify_driver_errors_without_changing_shared_classifier() {
        for label in ["RetryableWriteError", "RetryableError"] {
            let mut error = concern_error(label);
            // A command failure with the same server label follows the same path.
            error.kind = command_error(91).kind;
            assert!(matches!(
                classify_mongodb_error("delete", error.clone()),
                BackendError::Internal { .. }
            ));
            let failure = PageFailure::driver("delete page", error);
            assert!(matches!(
                failure.kind.as_ref(),
                PageFailureKind::Driver { .. }
            ));
            for result in failure.fan_out(3) {
                assert!(matches!(
                    result,
                    Err(StorageError::Backend(BackendError::Unavailable { .. }))
                ));
            }
        }
        for code in [50, 262] {
            let mut error = concern_error("RetryableWriteError");
            error.kind = command_error(code).kind;
            assert!(matches!(
                PageFailure::driver("insert page", error).classified(),
                StorageError::Backend(BackendError::Timeout { .. })
            ));
        }
        assert!(matches!(
            PageFailure::driver("delete page", command_error(91)).classified(),
            StorageError::Backend(BackendError::Internal { .. })
        ));
        let io =
            mongodb::error::Error::from(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
        assert!(matches!(
            PageFailure::driver("insert page", io).classified(),
            StorageError::Backend(BackendError::Unavailable { .. })
        ));
        assert!(matches!(
            PageFailure::from("invalid reply").classified(),
            StorageError::Backend(BackendError::Internal { .. })
        ));
    }

    #[test]
    fn metadata_concern_preserves_reply_details_labels_and_permanent_owners() {
        for nested in [false, true] {
            let mut concern = doc! { "code": 91, "codeName": "ShutdownInProgress", "errmsg": "acknowledgement uncertain", "errInfo": { "detail": "retained" } };
            let mut reply = doc! { "n": 1, "writeErrors": [{ "index": 1, "code": 121, "errmsg": "permanent rejection" }] };
            if nested {
                concern.insert("errorLabels", vec!["RetryableError"]);
            } else {
                reply.insert("errorLabels", vec!["RetryableWriteError"]);
            }
            reply.insert("writeConcernError", concern);
            let failure =
                metadata_update_result(&reply, &[(0, doc! {}), (2, doc! {})]).unwrap_err();
            match failure.kind.as_ref() {
                PageFailureKind::Metadata(saved) => assert_eq!(saved, &reply),
                _ => panic!("raw acknowledgement evidence must survive"),
            }
            let results = failure.fan_out(3);
            assert!(matches!(
                &results[0],
                Err(StorageError::Backend(BackendError::Unavailable { .. }))
            ));
            assert!(matches!(
                &results[2],
                Err(StorageError::Backend(BackendError::Internal { .. }))
            ));
            assert!(
                results[2]
                    .as_ref()
                    .unwrap_err()
                    .to_string()
                    .contains("permanent rejection")
            );
            assert!(
                results[0]
                    .as_ref()
                    .unwrap_err()
                    .to_string()
                    .contains("retained")
            );
        }
    }

    #[test]
    fn write_concern_timeouts_and_permanent_failures_keep_their_classification() {
        for concern in [
            doc! { "code": 50, "errmsg": "deadline" },
            doc! { "code": 262, "errmsg": "deadline" },
            doc! { "code": 64, "errmsg": "replication wait", "errInfo": { "wtimeout": true } },
        ] {
            let reply = doc! { "n": 0, "writeConcernError": concern.clone(), "errorLabels": ["RetryableWriteError"] };
            let failure = metadata_update_result(&reply, &[(0, doc! {})]).unwrap_err();
            assert!(matches!(
                failure.classified(),
                StorageError::Backend(BackendError::Timeout { .. })
            ));
            let concern = mongodb::bson::from_document(concern).unwrap();
            let error = mongodb::error::ErrorKind::Write(
                mongodb::error::WriteFailure::WriteConcernError(concern),
            )
            .into();
            assert!(matches!(
                PageFailure::driver("insert", error).classified(),
                StorageError::Backend(BackendError::Timeout { .. })
            ));
        }
        let reply =
            doc! { "n": 0, "writeConcernError": { "code": 100, "errmsg": "unknown concern mode" } };
        assert!(matches!(
            metadata_update_result(&reply, &[(0, doc! {})])
                .unwrap_err()
                .classified(),
            StorageError::Backend(BackendError::Internal { .. })
        ));
        let reply = doc! { "n": 0, "errorLabels": ["RetryableWriteError"], "writeErrors": [{ "index": 0, "code": 121, "errmsg": "invalid document" }] };
        let (_, failures) = metadata_update_result(&reply, &[(0, doc! {})]).unwrap();
        let outcomes = page_outcomes(vec![None], failures, &[0]);
        assert!(matches!(
            &outcomes[0],
            Err(StorageError::Backend(BackendError::Internal { .. }))
        ));
    }

    #[test]
    fn page_failure_keeps_prior_permanent_errors_and_extraction_precedence() {
        let failure = PageFailure::driver("insert", concern_error("RetryableWriteError"))
            .with_permanent(HashMap::from([
                (0, "own document rejected".into()),
                (1, "contained rejected".into()),
            ]))
            .with_outcomes(
                vec![Some("extraction failed".into()), None, None],
                HashMap::new(),
            );
        let outcomes = failure.fan_out(3);
        assert!(
            outcomes[0]
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("extraction failed")
        );
        assert!(
            outcomes[1]
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("contained rejected")
        );
        assert!(matches!(
            &outcomes[2],
            Err(StorageError::Backend(BackendError::Unavailable { .. }))
        ));
        let failure = PageFailure::storage(
            BackendError::ConnectionFailed {
                backend_name: "mongodb".into(),
                message: "offline".into(),
            }
            .into(),
        );
        assert!(matches!(
            &failure.fan_out(2)[1],
            Err(StorageError::Backend(BackendError::ConnectionFailed { .. }))
        ));
    }

    #[test]
    fn permanent_overlays_replace_existing_lower_priority_errors() {
        // A failed metadata command already contains its own permanent owners.
        let failure = PageFailure {
            kind: Box::new(PageFailureKind::Metadata(doc! {
                "writeConcernError": { "code": 91 }, "errorLabels": ["RetryableWriteError"]
            })),
            permanent: HashMap::from([
                (0, "metadata zero".into()),
                (1, "metadata one".into()),
                (2, "metadata two".into()),
                (3, "metadata three".into()),
            ]),
        }
        .with_permanent(HashMap::from([
            (0, "contained zero".into()),
            (1, "contained one".into()),
            (2, "contained two".into()),
        ]))
        .with_permanent(HashMap::from([
            (0, "own zero".into()),
            (1, "own one".into()),
        ]));
        assert_eq!(failure.permanent[&0], "own zero");
        assert_eq!(failure.permanent[&1], "own one");
        assert_eq!(failure.permanent[&2], "contained two");
        assert_eq!(failure.permanent[&3], "metadata three");

        let failure = failure.with_outcomes(
            vec![Some("extraction zero".into()), None, None, None, None],
            HashMap::from([(1, "earlier sub-batch one".into())]),
        );
        assert_eq!(failure.permanent[&0], "extraction zero");
        assert_eq!(failure.permanent[&1], "earlier sub-batch one");
        let outcomes = failure.fan_out(5);
        assert!(outcomes[..4].iter().all(|outcome| matches!(
            outcome,
            Err(StorageError::Backend(BackendError::Internal { .. }))
        )));
        assert!(matches!(
            &outcomes[4],
            Err(StorageError::Backend(BackendError::Unavailable { .. }))
        ));
    }

    #[test]
    fn metadata_fan_out_fails_every_resource_and_writes_none() {
        let mut permanent = HashMap::new();
        permanent.insert(1, "metadata rejected".to_string());
        let failure =
            PageFailure::driver("metadata page", command_error(2)).with_permanent(permanent);
        let (results, written, parameter_failures) = failure.fan_out_metadata(3);
        assert!(parameter_failures.is_empty());
        assert_eq!(written, 0);
        assert_eq!(results.len(), 3);
        assert!(matches!(&results[0], Err(e) if e.to_string().contains("metadata page")));
        assert!(matches!(&results[1], Err(e) if e.to_string().contains("metadata rejected")));
        assert!(matches!(&results[2], Err(e) if e.to_string().contains("metadata page")));
    }

    #[test]
    fn fan_out_copies_one_classification_and_shares_its_original_source() {
        let failure = PageFailure::driver("delete page", command_error(2));
        let outcomes = failure.fan_out(3);
        let mut sources = Vec::new();
        for outcome in &outcomes {
            let Err(StorageError::Backend(BackendError::Internal {
                message,
                source: Some(source),
                ..
            })) = outcome
            else {
                panic!("permanent driver failure must retain its source");
            };
            assert!(message.starts_with("delete page:"));
            sources.push(source.downcast_ref::<Arc<StorageError>>().unwrap());
        }
        assert!(Arc::ptr_eq(sources[0], sources[1]));
        assert!(Arc::ptr_eq(sources[1], sources[2]));
        let StorageError::Backend(BackendError::Internal {
            source: Some(driver),
            ..
        }) = sources[0].as_ref()
        else {
            panic!("shared classification must retain the original driver error");
        };
        assert!(driver.downcast_ref::<mongodb::error::Error>().is_some());

        // Every supported transient category and its message survive copying.
        for error in [
            BackendError::Unavailable {
                backend_name: "mongodb".into(),
                message: "offline".into(),
            },
            BackendError::Timeout {
                backend_name: "mongodb".into(),
                message: "deadline".into(),
            },
            BackendError::ConnectionFailed {
                backend_name: "mongodb".into(),
                message: "connection".into(),
            },
            BackendError::PoolExhausted {
                backend_name: "mongodb".into(),
            },
        ] {
            let expected_kind = std::mem::discriminant(&error);
            let expected_message = error.to_string();
            for outcome in PageFailure::storage(error.into()).fan_out(2) {
                let error = outcome.unwrap_err();
                let StorageError::Backend(backend) = &error else {
                    panic!("backend classification must survive fan-out");
                };
                assert_eq!(std::mem::discriminant(backend), expected_kind);
                assert_eq!(error.to_string(), expected_message);
            }
        }

        let original = BackendError::Internal {
            backend_name: "mongodb".into(),
            message: "database setup failed".into(),
            source: Some(Box::new(std::io::Error::other("original setup source"))),
        };
        let expected_message = original.to_string();
        for outcome in PageFailure::storage(original.into()).fan_out(2) {
            let error = outcome.unwrap_err();
            assert_eq!(error.to_string(), expected_message);
            let mut source: &(dyn std::error::Error + 'static) = &error;
            while let Some(next) = source.source() {
                source = next;
            }
            let original_source = source.downcast_ref::<std::io::Error>().unwrap();
            assert_eq!(original_source.to_string(), "original setup source");
        }
    }

    #[test]
    fn metadata_update_errors_keep_page_owners_and_unmatched_counts() {
        let updates = vec![(7, doc! {}), (12, doc! {})];
        let (matched, failures) = metadata_update_result(
            &doc! {
                "n": 1_i32,
                "writeErrors": [{ "index": 1, "code": 121, "errmsg": "metadata rejected" }],
            },
            &updates,
        )
        .unwrap();
        assert_eq!(matched, 1);
        assert_eq!(failures.len(), 1);
        assert!(failures[&12].contains("metadata rejected"));
        assert!(!failures.contains_key(&7));

        let (matched, failures) = metadata_update_result(&doc! { "n": 0_i64 }, &updates).unwrap();
        assert_eq!(
            matched, 0,
            "superseded resources do not match the guarded update"
        );
        assert!(failures.is_empty());
    }

    #[test]
    fn metadata_update_requires_acknowledgement_and_valid_reply() {
        let updates = vec![(7, doc! {})];
        let reply = doc! {
            "n": 1,
            "writeConcernError": { "code": 64, "errmsg": "replication timed out" },
        };
        assert!(
            metadata_update_result(&reply, &updates)
                .unwrap_err()
                .classified()
                .to_string()
                .contains("Unacknowledged")
        );
        for reply in [
            doc! {},
            doc! { "n": -1 },
            doc! { "n": 2 },
            doc! { "n": 0, "writeErrors": [{ "index": 3, "errmsg": "out of range" }] },
            doc! { "n": 0, "writeErrors": [{ "index": -1, "errmsg": "negative index" }] },
            doc! { "n": 0, "writeErrors": "invalid" },
        ] {
            assert!(metadata_update_result(&reply, &updates).is_err(), "{reply}");
        }
    }

    #[test]
    fn planner_first_range_is_first_without_a_seed() {
        let mut planner = SubBatchPlanner::new(1000, None, 1);
        assert_eq!(planner.next_range(), Some(0..32));
    }

    #[test]
    fn planner_sizes_from_observed_docs() {
        let mut planner = SubBatchPlanner::new(1000, None, 1);
        planner.record(32, 800);
        assert_eq!(planner.next_range(), Some(0..100)); // ceil(2500 * 32 / 800)
    }

    #[test]
    fn planner_uses_the_seed_for_its_first_range() {
        let mut planner = SubBatchPlanner::new(1000, Some((1000, 24_000)), 1);
        assert_eq!(planner.next_range(), Some(0..105)); // ceil(2500 * 1000 / 24000)
    }

    #[test]
    fn planner_clamps() {
        let mut a = SubBatchPlanner::new(1000, Some((1, 1630)), 1);
        assert_eq!(
            a.next_range(),
            Some(0..2),
            "1,630 docs/resource: ceil(2500/1630) = 2"
        );

        let mut b = SubBatchPlanner::new(1000, Some((1, 1630)), 4);
        assert_eq!(
            b.next_range(),
            Some(0..4),
            "min_size 4 lifts the same ratio to 4"
        );

        let mut c = SubBatchPlanner::new(1000, Some((1, 0)), 1);
        assert_eq!(
            c.next_range(),
            Some(0..REINDEX_SUBBATCH_MAX),
            "0 documents gives MAX"
        );

        let mut d = SubBatchPlanner::new(1000, Some((1, 100_000)), 1);
        assert_eq!(
            d.next_range(),
            Some(0..1),
            "100,000 docs/resource: ceil(2500/100000) = 1"
        );

        let mut e = SubBatchPlanner::new(1000, Some((0, 500)), 1);
        assert_eq!(
            e.next_range(),
            Some(0..REINDEX_SUBBATCH_FIRST),
            "a seed with r == 0 is ignored"
        );
    }

    #[test]
    fn planner_covers_every_index_exactly_once_and_in_order() {
        for len in [0usize, 1, 31, 32, 33, 1000, 12345] {
            for seed in [None, Some((1, 1630)), Some((1000, 24_000)), Some((0, 500))] {
                for min_size in [1usize, 4, 16] {
                    let mut planner = SubBatchPlanner::new(len, seed, min_size);
                    let mut covered = 0usize;
                    while let Some(range) = planner.next_range() {
                        assert_eq!(
                            range.start, covered,
                            "len={len} seed={seed:?} min_size={min_size}: ranges must tile without gaps or overlap"
                        );
                        assert!(range.end > range.start, "a range must never be empty");
                        covered = range.end;
                        planner.record(range.len(), range.len() * 25);
                    }
                    assert_eq!(
                        covered, len,
                        "len={len} seed={seed:?} min_size={min_size}: must cover every index"
                    );
                }
            }
        }
    }

    #[test]
    fn flatten_moves_documents_with_page_global_owners() {
        let own_a = doc! {"a": 1};
        let contained_a = doc! {"ac": 1};
        let own_b = doc! {"b": 1};
        let prepared = vec![
            (
                SearchIndexDocuments {
                    own: vec![own_a.clone()],
                    contained: vec![contained_a.clone()],
                    missing_params: Some(vec!["absent".into()]),
                    missing_fp: Some("fp1".into()),
                },
                ExtractionReport::Clean,
            ),
            (
                SearchIndexDocuments {
                    own: vec![own_b.clone()],
                    contained: Vec::new(),
                    missing_params: None,
                    missing_fp: None,
                },
                ExtractionReport::FailedResource("boom".to_string()),
            ),
        ];
        let mut extract_failures: Vec<Option<String>> = vec![None; 7];
        let mut parameter_failures: Vec<crate::search::ExtractionFailure> = Vec::new();
        let mut doc_counts: Vec<usize> = vec![0; 7];

        // start = 5: this sub-batch is the third of a bigger page, so its
        // owners land at the page-wide indices 5 and 6, not 0 and 1.
        let batch = flatten_sub_batch(
            5,
            prepared,
            &mut extract_failures,
            &mut parameter_failures,
            &mut doc_counts,
        );

        assert_eq!(batch.own_owners, vec![5, 6]);
        assert_eq!(batch.own_docs, vec![own_a, own_b]);
        assert_eq!(batch.contained_owners, vec![5]);
        assert_eq!(batch.contained_docs, vec![contained_a]);
        assert_eq!(batch.len(), 3);
        assert_eq!(
            batch.missing_params,
            vec![
                (5, Some(vec!["absent".to_owned()]), Some("fp1".to_owned())),
                (6, None, None),
            ]
        );
        assert_eq!(doc_counts[5], 2);
        assert_eq!(doc_counts[6], 1);
        assert_eq!(doc_counts[0], 0);
        assert_eq!(extract_failures[5], None);
        assert_eq!(extract_failures[6], Some("boom".to_string()));
    }

    fn missing_update(version_id: &str, names: Option<&[&str]>) -> MissingUpdate {
        missing_update_stamped(version_id, names, None)
    }

    fn missing_update_stamped(
        version_id: &str,
        names: Option<&[&str]>,
        fp: Option<&str>,
    ) -> MissingUpdate {
        MissingUpdate {
            owner: 0,
            statement: doc! {},
            resource_type: "Patient".into(),
            id: "p".into(),
            version_id: version_id.into(),
            names: names.map(|names| names.iter().map(|name| (*name).to_owned()).collect()),
            fp: fp.map(str::to_owned),
        }
    }

    #[test]
    fn missing_update_is_unchanged_only_for_the_same_live_version_and_value() {
        let set = missing_update("2", Some(&["a", "b"]));
        let unset = missing_update("2", None);
        let live = |missing: Option<Bson>| {
            let mut stored = doc! { "id": "p", "version_id": "2", "is_deleted": false };
            if let Some(missing) = missing {
                stored.insert("missing_params", missing);
            }
            stored
        };
        let array = |names: &[&str]| Some(Bson::from(names.to_vec()));

        assert!(set.is_unchanged(&live(array(&["a", "b"]))));
        assert!(unset.is_unchanged(&live(None)));

        // A different value, order or shape is written.
        assert!(!set.is_unchanged(&live(array(&["a"]))));
        assert!(!set.is_unchanged(&live(array(&["a", "b", "c"]))));
        assert!(!set.is_unchanged(&live(array(&["b", "a"]))));
        assert!(!set.is_unchanged(&live(array(&[]))));
        assert!(!set.is_unchanged(&live(None)));
        assert!(!set.is_unchanged(&live(Some(Bson::Null))));
        assert!(!set.is_unchanged(&live(Some(Bson::from(vec![1, 2])))));
        assert!(!unset.is_unchanged(&live(array(&[]))));
        assert!(!unset.is_unchanged(&live(Some(Bson::Null))));
        assert!(missing_update("2", Some(&[])).is_unchanged(&live(array(&[]))));

        // Another version or a deleted resource is left to the version guard.
        let mut newer = live(array(&["a", "b"]));
        newer.insert("version_id", "3");
        assert!(!set.is_unchanged(&newer));
        let mut deleted = live(array(&["a", "b"]));
        deleted.insert("is_deleted", true);
        assert!(!set.is_unchanged(&deleted));
        let mut no_flag = live(array(&["a", "b"]));
        no_flag.remove("is_deleted");
        assert!(!set.is_unchanged(&no_flag));

        // The stamp is written with the array (#1999): a stale or absent stamp
        // is due, and so is one the update would unset.
        let stamped = missing_update_stamped("2", Some(&["a", "b"]), Some("fp2"));
        let with_fp = |fp: &str| {
            let mut stored = live(array(&["a", "b"]));
            stored.insert("missing_params_fp", fp);
            stored
        };
        assert!(stamped.is_unchanged(&with_fp("fp2")));
        assert!(!stamped.is_unchanged(&with_fp("fp1")));
        assert!(!stamped.is_unchanged(&live(array(&["a", "b"]))));
        assert!(!set.is_unchanged(&with_fp("fp2")));
        let unset_stamped = missing_update_stamped("2", None, Some("fp2"));
        assert!(unset_stamped.is_unchanged(&live(None)));
        let mut stale_stamp_only = live(None);
        stale_stamp_only.insert("missing_params_fp", "fp2");
        assert!(!unset_stamped.is_unchanged(&stale_stamp_only));
    }

    #[test]
    fn page_outcomes_prefers_extract_then_insert_failures() {
        let extract_failures = vec![
            Some("extract failed".to_string()),
            None,
            None,
            Some("extract failed too".to_string()),
        ];
        let mut insert_failures = HashMap::new();
        insert_failures.insert(1, "insert failed".to_string());
        // Index 3 has both an extraction and an insert failure, so this
        // proves precedence rather than just index 0 having no insert
        // failure to prefer over.
        insert_failures.insert(3, "insert failed too".to_string());
        let doc_counts = vec![0, 0, 4, 0];
        let outcomes = page_outcomes(extract_failures, insert_failures, &doc_counts);
        assert!(matches!(&outcomes[0], Err(e) if e.to_string().contains("extract failed")));
        assert!(matches!(&outcomes[1], Err(e) if e.to_string().contains("insert failed")));
        assert!(matches!(outcomes[2], Ok(4)));
        assert!(matches!(&outcomes[3], Err(e) if e.to_string().contains("extract failed too")));
    }

    #[test]
    fn merge_insert_failures_prefers_own_over_contained() {
        let mut own = HashMap::new();
        own.insert(3, "own failed".to_string());
        let mut contained = HashMap::new();
        contained.insert(3, "contained failed".to_string());
        contained.insert(4, "contained only".to_string());
        let mut into = HashMap::new();
        merge_insert_failures(&mut into, InsertFailures { own, contained });
        assert_eq!(into.get(&3), Some(&"own failed".to_string()));
        assert_eq!(into.get(&4), Some(&"contained only".to_string()));
    }

    #[test]
    fn assign_ids_writes_the_id_it_returns_into_each_document_with_its_owner() {
        let mut docs = vec![doc! { "a": 1 }, doc! { "b": 2 }, doc! { "c": 3 }];
        let ids = assign_ids(&mut docs, &[4, 4, 9]);
        for (doc, (_, id)) in docs.iter().zip(&ids) {
            assert_eq!(doc.get("_id"), Some(id));
            assert!(matches!(id, Bson::ObjectId(_)));
        }
        assert_eq!(
            ids.iter().map(|(owner, _)| *owner).collect::<Vec<_>>(),
            [4, 4, 9]
        );
        assert_ne!(ids[0].1, ids[1].1);
    }

    #[test]
    fn cleanup_never_claims_ids_discovered_after_the_snapshot() {
        let page = vec![Bson::ObjectId(ObjectId::new())];
        let previous = vec![Bson::ObjectId(ObjectId::new())];
        let foreign = Bson::ObjectId(ObjectId::new());
        let present = vec![page[0].clone(), previous[0].clone(), foreign.clone()];
        let unchanged =
            classify_cleanup_rows(present.clone(), &page, &previous, ResourceState::Unchanged);
        assert_eq!(unchanged.removable, previous);
        assert_eq!(unchanged.unknown, 1);
        assert_eq!(
            unchanged.mismatch(ResourceState::Unchanged),
            Some(RowMismatch {
                remaining: 2,
                unknown: 1,
                missing: 0,
            })
        );
        let changed =
            classify_cleanup_rows(present.clone(), &page, &previous, ResourceState::Changed);
        assert_eq!(changed.removable, page);
        assert_eq!(
            changed.unknown, 0,
            "after an update the other rows are its own or a generation its rewrite clears"
        );
        let deleted = classify_cleanup_rows(present, &page, &previous, ResourceState::Deleted);
        assert_eq!(deleted.removable, page);
        assert_eq!(deleted.unknown, 0);
        let current =
            classify_cleanup_rows(vec![foreign], &page, &previous, ResourceState::Changed);
        assert!(
            current.removable.is_empty(),
            "preserve foreign rows without claiming their generation"
        );
        assert_eq!(current.mismatch(ResourceState::Changed), None);
    }

    #[test]
    fn overlapping_same_version_pages_do_not_delete_each_others_new_rows() {
        let a = vec![Bson::ObjectId(ObjectId::new())];
        let b = vec![Bson::ObjectId(ObjectId::new())];
        let present = vec![a[0].clone(), b[0].clone()];
        for page in [&a, &b] {
            let rows = classify_cleanup_rows(present.clone(), page, &[], ResourceState::Unchanged);
            assert!(rows.removable.is_empty());
            assert_eq!(
                rows.unknown, 1,
                "overlap is a retryable conflict, never clean success"
            );
        }
        // If B captured A before B inserted, only B may remove A. A's
        // snapshot cannot have captured B, so ownership cannot form a cycle.
        let b_cleanup = classify_cleanup_rows(present.clone(), &b, &a, ResourceState::Unchanged);
        assert_eq!(b_cleanup.removable, a);
        let a_cleanup =
            classify_cleanup_rows(vec![b[0].clone()], &a, &[], ResourceState::Unchanged);
        assert!(a_cleanup.removable.is_empty());
        assert_eq!(a_cleanup.unknown, 1);
    }

    #[test]
    fn exact_identity_detects_same_count_replacement_and_missing_rows() {
        let page = vec![Bson::ObjectId(ObjectId::new())];
        let other = Bson::ObjectId(ObjectId::new());
        let replaced = classify_cleanup_rows(vec![other], &page, &[], ResourceState::Unchanged);
        assert!(replaced.removable.is_empty());
        assert_eq!(
            replaced.mismatch(ResourceState::Unchanged),
            Some(RowMismatch {
                remaining: 1,
                unknown: 1,
                missing: 1,
            })
        );
        let missing = classify_cleanup_rows(vec![], &page, &[], ResourceState::Unchanged);
        assert_eq!(
            missing.mismatch(ResourceState::Unchanged),
            Some(RowMismatch {
                remaining: 0,
                unknown: 0,
                missing: 1,
            })
        );
        let intact = classify_cleanup_rows(page.clone(), &page, &[], ResourceState::Unchanged);
        assert_eq!(intact.mismatch(ResourceState::Unchanged), None);
    }

    #[test]
    fn absent_local_resource_preserves_supplied_page_ids() {
        let page = vec![Bson::ObjectId(ObjectId::new())];
        let captured = vec![Bson::ObjectId(ObjectId::new())];
        let intact = classify_cleanup_rows(page.clone(), &page, &[], ResourceState::Absent);
        assert!(
            intact.removable.is_empty(),
            "external-source rows are not deleted"
        );
        assert_eq!(intact.mismatch(ResourceState::Absent), None);
        let leftovers = classify_cleanup_rows(
            vec![page[0].clone(), captured[0].clone()],
            &page,
            &captured,
            ResourceState::Absent,
        );
        assert_eq!(leftovers.removable, captured);
        let missing = classify_cleanup_rows(vec![], &page, &[], ResourceState::Absent);
        assert_eq!(
            missing.mismatch(ResourceState::Absent),
            Some(RowMismatch {
                remaining: 0,
                unknown: 0,
                missing: 1,
            })
        );
        // A confirmed tombstone, unlike absence, invalidates the supplied page.
        let deleted = classify_cleanup_rows(page.clone(), &page, &[], ResourceState::Deleted);
        assert_eq!(deleted.removable, page);
    }

    #[test]
    fn changed_and_deleted_resources_require_removal_of_page_owned_rows() {
        let page = vec![Bson::ObjectId(ObjectId::new())];
        let previous = vec![Bson::ObjectId(ObjectId::new())];
        let foreign = Bson::ObjectId(ObjectId::new());
        for state in [ResourceState::Changed, ResourceState::Deleted] {
            let remaining = classify_cleanup_rows(
                vec![page[0].clone(), previous[0].clone(), foreign.clone()],
                &page,
                &previous,
                state,
            );
            assert_eq!(remaining.removable, page);
            assert_eq!(
                remaining.mismatch(state),
                Some(RowMismatch {
                    remaining: 1,
                    unknown: 0,
                    missing: 0,
                }),
                "obsolete page-owned rows must request recovery: {state:?}"
            );
            for retained in [vec![], vec![previous[0].clone(), foreign.clone()]] {
                let clean = classify_cleanup_rows(retained, &page, &previous, state);
                assert!(clean.removable.is_empty());
                assert_eq!(clean.mismatch(state), None, "{state:?}");
            }
        }
    }

    #[test]
    fn unknown_rows_report_retryable_conflict_without_overriding_permanent_errors() {
        let tenant = crate::tenant::TenantId::new("t");
        let resource = StoredResource::new(
            "Observation",
            "a",
            tenant,
            serde_json::json!({ "resourceType": "Observation", "id": "a" }),
            helios_fhir::FhirVersion::default(),
        );
        for permanent in [false, true] {
            let mut outcomes = vec![if permanent {
                Err(internal_error("permanent rejection".into()))
            } else {
                Ok(1)
            }];
            apply_verification_outcomes(
                &mut outcomes,
                vec![ResourceVerificationFailure {
                    resource_type: "Observation".into(),
                    resource_id: "a".into(),
                    mismatch: RowMismatch {
                        remaining: 2,
                        unknown: 2,
                        missing: 0,
                    },
                }],
                std::slice::from_ref(&resource),
            );
            let error = outcomes[0].as_ref().unwrap_err();
            if permanent {
                assert!(error.to_string().contains("permanent rejection"));
            } else {
                assert!(matches!(
                    error,
                    StorageError::Backend(BackendError::Unavailable { .. })
                ));
                assert!(error.to_string().contains("uncertain ownership"));
            }
        }
    }

    #[test]
    fn stale_outcomes_fail_only_the_named_resources_that_were_ok() {
        let tenant = crate::tenant::TenantId::new("t");
        let resource = |resource_type: &str, id: &str| {
            StoredResource::new(
                resource_type,
                id,
                tenant.clone(),
                serde_json::json!({ "resourceType": resource_type, "id": id }),
                helios_fhir::FhirVersion::default(),
            )
        };
        let resources = [
            resource("Observation", "a"),
            resource("Observation", "b"),
            resource("Patient", "b"),
        ];
        let mut outcomes: Vec<StorageResult<usize>> = vec![
            Ok(3),
            Err(internal_error("extraction failed".into())),
            Ok(5),
        ];
        apply_verification_outcomes(
            &mut outcomes,
            vec![
                ResourceVerificationFailure {
                    resource_type: "Observation".into(),
                    resource_id: "b".into(),
                    mismatch: RowMismatch {
                        remaining: 4,
                        unknown: 0,
                        missing: 0,
                    },
                },
                ResourceVerificationFailure {
                    resource_type: "Patient".into(),
                    resource_id: "b".into(),
                    mismatch: RowMismatch {
                        remaining: 2,
                        unknown: 0,
                        missing: 0,
                    },
                },
                ResourceVerificationFailure {
                    resource_type: "Observation".into(),
                    resource_id: "unknown".into(),
                    mismatch: RowMismatch {
                        remaining: 1,
                        unknown: 0,
                        missing: 0,
                    },
                },
            ],
            &resources,
        );
        assert!(matches!(outcomes[0], Ok(3)));
        assert!(
            outcomes[1]
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("extraction failed"),
            "an earlier failure keeps its message"
        );
        let patient = outcomes[2].as_ref().unwrap_err();
        assert!(
            matches!(
                patient,
                StorageError::Backend(BackendError::Unavailable { .. })
            ),
            "a resource with rows left after cleanup is retryable: {patient:?}"
        );
        assert!(
            patient
                .to_string()
                .contains("search index mismatch for Patient/b after cleanup: 2 unresolved rows"),
            "{patient}"
        );
    }

    #[test]
    fn absorb_db_task_carries_the_verification_fields() {
        let mut stats = ReindexPageStats::default();
        let t = ReindexPageStats {
            verify: Duration::from_millis(11),
            stale_rows_found: 34,
            stale_rows_remaining: 17,
            ..ReindexPageStats::default()
        };
        absorb_db_task(&mut stats, &t);
        assert_eq!(stats.verify, Duration::from_millis(11));
        assert_eq!(stats.stale_rows_found, 34);
        assert_eq!(stats.stale_rows_remaining, 17);
        let mut total = ReindexPageStats::default();
        total.accumulate(&stats);
        total.accumulate(&stats);
        assert_eq!(total.stale_rows_found, 68);
        assert_eq!(total.verify, Duration::from_millis(22));
        assert_eq!(
            total.db_wait_or_busy(),
            Duration::from_millis(22),
            "the serial busy time includes verification"
        );
    }

    #[test]
    fn absorb_db_task_adds_only_the_db_fields() {
        let mut stats = ReindexPageStats {
            extract: Duration::from_millis(99),
            sub_batches: 3,
            ..ReindexPageStats::default()
        };
        let t = ReindexPageStats {
            delete: Duration::from_millis(5),
            insert: Duration::from_millis(7),
            deleted_entries: 2,
            inserted_entries: 9,
            insert_commands: 1,
            extract: Duration::from_millis(1000), // must NOT be absorbed
            sub_batches: 100,                     // must NOT be absorbed
            db_wait: Some(Duration::from_millis(1)), // must NOT be absorbed
            ..ReindexPageStats::default()
        };
        absorb_db_task(&mut stats, &t);
        assert_eq!(stats.delete, Duration::from_millis(5));
        assert_eq!(stats.insert, Duration::from_millis(7));
        assert_eq!(stats.deleted_entries, 2);
        assert_eq!(stats.inserted_entries, 9);
        assert_eq!(stats.insert_commands, 1);
        assert_eq!(
            stats.extract,
            Duration::from_millis(99),
            "extract must be untouched"
        );
        assert_eq!(stats.sub_batches, 3, "sub_batches must be untouched");
        assert_eq!(
            stats.db_wait, None,
            "db_wait is tracked only via add_db_wait"
        );
    }

    #[test]
    fn add_db_wait_turns_none_into_some() {
        let mut stats = ReindexPageStats::default();
        let mut page_wait = Duration::ZERO;
        add_db_wait(&mut stats, &mut page_wait, Duration::from_millis(10));
        assert_eq!(stats.db_wait, Some(Duration::from_millis(10)));
        assert_eq!(page_wait, Duration::from_millis(10));
        add_db_wait(&mut stats, &mut page_wait, Duration::from_millis(5));
        assert_eq!(stats.db_wait, Some(Duration::from_millis(15)));
        assert_eq!(page_wait, Duration::from_millis(15));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn abort_on_drop_aborts_a_pending_task() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        struct DropSignal(Option<tokio::sync::oneshot::Sender<()>>);
        impl Drop for DropSignal {
            fn drop(&mut self) {
                if let Some(tx) = self.0.take() {
                    let _ = tx.send(());
                }
            }
        }
        let guard = DropSignal(Some(tx));
        let task = AbortOnDrop::spawn(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        drop(task);
        tokio::time::timeout(Duration::from_secs(1), rx)
            .await
            .expect("dropping AbortOnDrop must abort the task within 1s")
            .expect("the guard's Drop must fire, delivering the oneshot");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_failure_resumes_a_panic() {
        let handle = tokio::spawn(async { panic!("boom") });
        let err = handle.await.expect_err("the spawned task panicked");
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| join_failure(err, "insert")));
        assert!(
            result.is_err(),
            "join_failure must resume the panic, not swallow it"
        );
    }
}
