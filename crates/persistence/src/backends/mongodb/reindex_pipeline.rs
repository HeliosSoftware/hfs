//! Sub-batch pipeline for MongoDB `$reindex` page preparation (#1403).
//!
//! While sub-batch *k*'s `insert_many` runs on a spawned task, the page's own
//! thread extracts sub-batch *k+1* inside `tokio::task::block_in_place`, so
//! HFS-side extraction overlaps MongoDB-side delete/insert instead of running
//! strictly after it. This module holds the planner that sizes sub-batches,
//! the `block_in_place` + rayon extraction helper, and the serial and
//! overlapped writer bodies that call them; `storage.rs`'s
//! `write_search_entries_page_timed` dispatches between the two and keeps
//! the rest of the `ReindexTarget`/`ReindexSource` methods.
//!
//! See `docs/superpowers/specs/2026-09-23-mongodb-reindex-rebuild-design.md`
//! §4.4.

use std::collections::HashMap;
use std::ops::Range;
use std::time::Duration;

use mongodb::bson::{Bson, Document, doc};

use crate::error::StorageResult;
use crate::search::reindex::ReindexPageStats;
use crate::types::StoredResource;

use super::storage::{SearchIndexDocuments, internal_error};

/// A type's first page: too little is known yet to size from observation (#1403).
pub(super) const REINDEX_SUBBATCH_FIRST: usize = 32;
/// Own + contained index documents a sub-batch targets (#1403).
pub(super) const REINDEX_SUBBATCH_TARGET_DOCS: usize = 2_500;
/// Largest resources a single sub-batch may hold (#1403).
pub(super) const REINDEX_SUBBATCH_MAX: usize = 512;
/// Mirrors PostgreSQL's reindex prepare minimum page size: the smallest page
/// for which the serial writer still tries the rayon pool (#1403).
pub(super) const REINDEX_SERIAL_POOL_MIN_PAGE: usize = 16;
/// Ceiling on an explicit `HFS_MONGODB_REINDEX_PREPARE_THREADS` value (#1403).
const REINDEX_PREPARE_MAX_THREADS: usize = 64;

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

/// The pool and admission gate a page's extraction may use (#1403).
pub(super) struct PrepareEnv<'a> {
    pub pool: Option<&'a rayon::ThreadPool>,
    pub gate: &'a tokio::sync::Semaphore,
}

/// Runs `prepare` over `range` in input order, returning `true` when it ran
/// on the pool (#1403). The caller guarantees a multi-thread Tokio runtime.
///
/// Every sub-batch runs inside `block_in_place`, even with `pool: None`: a
/// task spawned from this worker (the page's insert of the *previous*
/// sub-batch) sits in this worker's LIFO slot until the worker either
/// finishes its own synchronous work or calls `block_in_place`, which hands
/// the LIFO slot and run queue to another worker. Without it, extraction
/// would silently never overlap the spawned insert, even at a prepare width
/// of one.
fn extract_range<T: Send, F: Fn(usize) -> T + Sync>(
    env: &PrepareEnv<'_>,
    range: Range<usize>,
    prepare: &F,
) -> (Vec<T>, bool) {
    use rayon::prelude::*;
    tokio::task::block_in_place(|| {
        if let Some(pool) = env.pool
            && range.len() >= 2
            && let Ok(permit) = env.gate.try_acquire()
        {
            let out = pool.install(|| range.into_par_iter().map(prepare).collect::<Vec<T>>());
            drop(permit);
            (out, true)
        } else {
            (range.map(prepare).collect(), false)
        }
    })
}

/// `available_parallelism − 1`, clamped to 1–4, when `configured` is `0`,
/// mirroring PostgreSQL's reindex prepare width rule; otherwise `configured`
/// clamped to at most `REINDEX_PREPARE_MAX_THREADS` (#1403).
pub(super) fn resolve_prepare_width(configured: usize) -> usize {
    if configured == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get().saturating_sub(1).clamp(1, 4))
            .unwrap_or(1)
    } else {
        configured.min(REINDEX_PREPARE_MAX_THREADS)
    }
}

/// Whether this thread is running a multi-thread Tokio runtime, which is
/// what lets `block_in_place` hand this worker's run queue to another worker
/// instead of just blocking it. `block_in_place` panics only on a
/// current-thread runtime; called with no runtime running at all, it just
/// runs its closure inline — this backend never calls it in that case, since
/// it always gates the pooled extraction path behind this check first.
/// Mirrors PostgreSQL's reindex prepare path; kept as MongoDB's own copy
/// because this backend's extraction also needs `block_in_place` for the
/// serial (non-pooled) path and a configurable prepare width, which
/// PostgreSQL's reindex path does not.
pub(super) fn tokio_multi_thread_runtime() -> bool {
    tokio::runtime::Handle::try_current().is_ok_and(|handle| {
        matches!(
            handle.runtime_flavor(),
            tokio::runtime::RuntimeFlavor::MultiThread
        )
    })
}

/// One sub-batch's extracted documents, split by destination collection, with
/// page-wide resource-index owners for each document (#1403).
struct SubBatchDocs {
    own_owners: Vec<usize>,
    own_docs: Vec<Document>,
    contained_owners: Vec<usize>,
    contained_docs: Vec<Document>,
}

impl SubBatchDocs {
    fn len(&self) -> usize {
        self.own_docs.len() + self.contained_docs.len()
    }
}

/// Moves sub-batch `[start, start + prepared.len())`'s extracted documents
/// into one [`SubBatchDocs`], recording each resource's extraction failure
/// and document count at its **page-wide** index. Moves (never clones) its
/// documents, since each one is only needed here and copying it would be
/// wasted work (#1403).
fn flatten_sub_batch(
    start: usize,
    prepared: Vec<(SearchIndexDocuments, Option<String>)>,
    extract_failures: &mut [Option<String>],
    doc_counts: &mut [usize],
) -> SubBatchDocs {
    let mut batch = SubBatchDocs {
        own_owners: Vec::new(),
        own_docs: Vec::new(),
        contained_owners: Vec::new(),
        contained_docs: Vec::new(),
    };
    for (offset, (docs, failure)) in prepared.into_iter().enumerate() {
        let i = start + offset;
        doc_counts[i] = docs.own.len() + docs.contained.len();
        extract_failures[i] = failure;
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

/// What a database task hands back to the page's own thread when joined:
/// its own delete/insert phase timing and counts, and its outcome (#1403).
struct DbTask<T> {
    stats: ReindexPageStats,
    result: Result<T, String>,
}

/// Per-resource write errors for one sub-batch's insert, kept separate by
/// destination collection so [`merge_insert_failures`] can prefer an own-row
/// failure over a contained-row one (#1403).
struct InsertFailures {
    own: HashMap<usize, String>,
    contained: HashMap<usize, String>,
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

/// `n` copies of `Err(msg)`, for a page-level failure that could not be
/// attributed to specific resources (#1403).
fn fan_out(n: usize, msg: &str) -> Vec<StorageResult<usize>> {
    (0..n)
        .map(|_| Err(internal_error(msg.to_string())))
        .collect()
}

/// Turns a joined database task's `JoinError` into a page-level failure
/// message. A panic resumes here, exactly where an un-spawned call would
/// itself have panicked, so the panic still propagates instead of being
/// silently swallowed as an ordinary `Err` (#1403).
fn join_failure(e: tokio::task::JoinError, what: &str) -> String {
    if e.is_panic() {
        std::panic::resume_unwind(e.into_panic());
    }
    format!("search index {what} task ended without a result: {e}")
}

/// Joins a spawned sub-batch insert, absorbing its stats into `stats` and its
/// wait time into `page_wait`, and folding its per-resource failures into
/// `insert_failures`. Used both to finish the previous sub-batch's insert
/// before starting the next one, and to drain the last one once every
/// sub-batch has been extracted. Returns the page-level failure message on a
/// join or insert error (#1403).
async fn join_insert(
    mut task: AbortOnDrop<DbTask<InsertFailures>>,
    stats: &mut ReindexPageStats,
    page_wait: &mut Duration,
    insert_failures: &mut HashMap<usize, String>,
) -> Result<(), String> {
    let waited = std::time::Instant::now();
    let joined = task.join().await;
    add_db_wait(stats, page_wait, waited.elapsed());
    let done = joined.map_err(|e| join_failure(e, "insert"))?;
    absorb_db_task(stats, &done.stats);
    match done.result {
        Ok(failures) => {
            merge_insert_failures(insert_failures, failures);
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
) -> DbTask<()> {
    let started = std::time::Instant::now();
    let mut stats = ReindexPageStats::default();
    for filter in filters {
        match own.delete_many(filter.clone()).await {
            Ok(result) => stats.deleted_entries += result.deleted_count,
            Err(e) => {
                stats.delete += started.elapsed();
                return DbTask {
                    stats,
                    result: Err(format!("Failed to delete search entries: {e}")),
                };
            }
        }
        match contained.delete_many(filter).await {
            Ok(result) => stats.deleted_entries += result.deleted_count,
            Err(e) => {
                stats.delete += started.elapsed();
                return DbTask {
                    stats,
                    result: Err(format!(
                        "Failed to delete search_index_contained entries: {e}"
                    )),
                };
            }
        }
    }
    stats.delete += started.elapsed();
    DbTask {
        stats,
        result: Ok(()),
    }
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
    docs: SubBatchDocs,
) -> DbTask<InsertFailures> {
    let mut stats = ReindexPageStats::default();
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
                    result: Err(msg),
                };
            }
        };
    }
    DbTask {
        stats,
        result: Ok(InsertFailures {
            own: own_failures,
            contained: contained_failures,
        }),
    }
}

impl super::MongoBackend {
    /// Extracts one resource's search-index documents (#1403).
    fn extract_one(
        &self,
        tenant_id: &str,
        r: &StoredResource,
    ) -> (SearchIndexDocuments, Option<String>) {
        self.search_index_documents_checked(tenant_id, r.resource_type(), r.id(), r.content())
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
    ) -> Vec<StorageResult<usize>> {
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
        let mut doc_counts: Vec<usize> = vec![0; n];
        let batch = flatten_sub_batch(0, prepared, &mut extract_failures, &mut doc_counts);
        stats.extract += started.elapsed();
        stats.sub_batches += 1;
        stats.pool_sub_batches += u64::from(on_pool);

        let filters = delete_filters(tenant_id, resources);
        let done = delete_page(own.clone(), contained.clone(), filters).await;
        absorb_db_task(stats, &done.stats);
        if let Err(msg) = done.result {
            // Serial db_wait is the awaited delete plus the awaited insert;
            // on this early return only the delete has run.
            stats.db_wait = Some(stats.delete + stats.insert);
            return fan_out(n, &msg);
        }

        let done = insert_sub_batch(own, contained, batch).await;
        absorb_db_task(stats, &done.stats);
        stats.db_wait = Some(stats.delete + stats.insert);
        let insert_failures = match done.result {
            Ok(failures) => {
                let mut into = HashMap::new();
                merge_insert_failures(&mut into, failures);
                into
            }
            Err(msg) => return fan_out(n, &msg),
        };

        let docs: u64 = doc_counts.iter().sum::<usize>() as u64;
        record_page_perf(stats, stats.db_wait.unwrap_or_default(), docs);
        page_outcomes(extract_failures, insert_failures, &doc_counts)
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
    ) -> Vec<StorageResult<usize>> {
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
            filters,
        )));
        let mut planner = SubBatchPlanner::new(n, seed, min_size);
        let mut extract_failures: Vec<Option<String>> = vec![None; n];
        let mut doc_counts: Vec<usize> = vec![0; n];
        let mut insert_failures: HashMap<usize, String> = HashMap::new();
        let mut in_flight: Option<AbortOnDrop<DbTask<InsertFailures>>> = None;
        let mut page_wait = Duration::ZERO;

        while let Some(range) = planner.next_range() {
            // Extract this sub-batch; it overlaps the delete, or the
            // previous sub-batch's insert.
            let started = std::time::Instant::now();
            let (prepared, on_pool) = extract_range(&env, range.clone(), &prepare);
            let batch = flatten_sub_batch(
                range.start,
                prepared,
                &mut extract_failures,
                &mut doc_counts,
            );
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
                    Err(e) => return fan_out(n, &join_failure(e, "delete")),
                };
                absorb_db_task(stats, &done.stats);
                if let Err(msg) = done.result {
                    return fan_out(n, &msg);
                }
            }
            // At most one insert is ever in flight: finish the previous
            // sub-batch's insert before starting this one.
            if let Some(task) = in_flight.take()
                && let Err(msg) =
                    join_insert(task, stats, &mut page_wait, &mut insert_failures).await
            {
                return fan_out(n, &msg);
            }
            in_flight = Some(AbortOnDrop::spawn(insert_sub_batch(
                own.clone(),
                contained.clone(),
                batch,
            )));
        }
        // Drain the last sub-batch's insert the same way.
        if let Some(task) = in_flight.take()
            && let Err(msg) = join_insert(task, stats, &mut page_wait, &mut insert_failures).await
        {
            return fan_out(n, &msg);
        }

        // Success only: remember the type's documents per resource, forward to perf.
        let docs: u64 = doc_counts.iter().sum::<usize>() as u64;
        if let Some(t) = page_type {
            self.record_reindex_docs(t, n as u64, docs);
        }
        record_page_perf(stats, page_wait, docs);
        page_outcomes(extract_failures, insert_failures, &doc_counts)
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
            prefetch = self.config().reindex_prefetch && !self.is_search_offloaded(),
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
    fn resolve_prepare_width_auto_explicit_and_capped() {
        let expected_auto = std::thread::available_parallelism()
            .map(|n| n.get().saturating_sub(1).clamp(1, 4))
            .unwrap_or(1);
        assert_eq!(resolve_prepare_width(0), expected_auto);
        assert_eq!(resolve_prepare_width(1), 1);
        assert_eq!(resolve_prepare_width(100), 64);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn extract_range_preserves_input_order_on_the_pool_and_inline() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(3)
            .build()
            .unwrap();
        let gate = tokio::sync::Semaphore::new(1);
        let prepare = |i: usize| i * 10;

        let pooled_env = PrepareEnv {
            pool: Some(&pool),
            gate: &gate,
        };
        let (out, on_pool) = extract_range(&pooled_env, 0..20, &prepare);
        assert!(
            on_pool,
            "a range of 20 with a pool and a free gate must run on the pool"
        );
        assert_eq!(out, (0..20).map(|i| i * 10).collect::<Vec<_>>());

        let inline_env = PrepareEnv {
            pool: None,
            gate: &gate,
        };
        let (out, on_pool) = extract_range(&inline_env, 0..20, &prepare);
        assert!(!on_pool);
        assert_eq!(out, (0..20).map(|i| i * 10).collect::<Vec<_>>());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn extract_range_runs_inline_when_the_gate_is_held() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(3)
            .build()
            .unwrap();
        let gate = tokio::sync::Semaphore::new(1);
        let _permit = gate
            .try_acquire()
            .expect("a fresh semaphore has a free permit");
        let env = PrepareEnv {
            pool: Some(&pool),
            gate: &gate,
        };
        let calling_thread = std::thread::current().id();
        let prepare = move |_i: usize| std::thread::current().id();

        let (out, on_pool) = extract_range(&env, 0..5, &prepare);
        assert!(!on_pool, "a held gate must fall back to the calling thread");
        assert!(out.iter().all(|id| *id == calling_thread));
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
                },
                None,
            ),
            (
                SearchIndexDocuments {
                    own: vec![own_b.clone()],
                    contained: Vec::new(),
                },
                Some("boom".to_string()),
            ),
        ];
        let mut extract_failures: Vec<Option<String>> = vec![None; 7];
        let mut doc_counts: Vec<usize> = vec![0; 7];

        // start = 5: this sub-batch is the third of a bigger page, so its
        // owners land at the page-wide indices 5 and 6, not 0 and 1.
        let batch = flatten_sub_batch(5, prepared, &mut extract_failures, &mut doc_counts);

        assert_eq!(batch.own_owners, vec![5, 6]);
        assert_eq!(batch.own_docs, vec![own_a, own_b]);
        assert_eq!(batch.contained_owners, vec![5]);
        assert_eq!(batch.contained_docs, vec![contained_a]);
        assert_eq!(batch.len(), 3);
        assert_eq!(doc_counts[5], 2);
        assert_eq!(doc_counts[6], 1);
        assert_eq!(doc_counts[0], 0);
        assert_eq!(extract_failures[5], None);
        assert_eq!(extract_failures[6], Some("boom".to_string()));
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
