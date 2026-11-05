//! #2310: a tenant purge racing the writers that put rows into the search
//! collections, `$reindex` and `$bulk-submit` ingest.
//!
//! Child module of the `mongodb_tests` root; `use super::*;` reaches its
//! private harness, as in `deferred_search_index.rs`, whose #2259 tests are the
//! model (a held command, a purge on a second backend of the same database, a
//! kept tenant that must not change).
//!
//! Every test holds ONE command of ONE writer with a `failCommand` failpoint
//! (`blockConnection`, scoped by the writer's `appName` and by `namespace`, so
//! it needs no skip counting and no sleep), waits until the server reports the
//! command held, runs `purge_tenant_data` on a second backend, then lets the
//! writer finish. The purge therefore runs entirely between two commands of the
//! writer, which is the order the issue worries about: a writer that read its
//! page or batch before the purge and writes after it.
//!
//! What a test asserts, in this order:
//!
//! 1. no collection of the database holds a document of the purged tenant
//!    ([`tenant_rows`] scans every collection, so a collection added later
//!    cannot be missed);
//! 2. the kept tenant, which has the same resource ids, is unchanged;
//! 3. only then, how the writer ended.
//!
//! The hold is 10 s ([`HOLD_MS`]). `FailPoint::off` does not cut a hold short.
//! Its call returns only about when the hold ends (inferred: the tests run one
//! after the other, as the failpoint lock makes them, at about 10 s each; the
//! server appears to wait for the command inside the failpoint before it turns
//! the failpoint off). So a test's writer has finished, and its rows have
//! landed, by the time `off` returns. Each test asserts that the writer has not
//! finished when the action that races it has finished. That catches a writer
//! that was already done, not every action that outlasts the hold.

use super::*;

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::time::{Duration, Instant};

use futures::TryStreamExt;
use futures::future::{Either, select};
use helios_persistence::core::{
    BulkEntryResult, BulkProcessingOptions, BulkSubmitProvider, DefaultSubmitWorker, LeaseError,
    NdjsonEntry, RemoteFile, RemoteManifest, SubmissionId, SubmitClaimStrategy, SubmitFileRecord,
    SubmitInputFetcher, SubmitWorkerStorage, WorkerId,
};
use helios_persistence::error::{BulkSubmitError, StorageResult};
use helios_persistence::search::{
    ReindexOperation, ReindexPageStats, ReindexRequest, ReindexSource, ReindexStatus, ReindexTarget,
};
use helios_persistence::types::StoredResource;

use crate::bulk_submit::FailPoint;

/// How long a held command blocks (`blockTimeMS`).
const HOLD_MS: i64 = 10_000;

const RESOURCE_TYPE: &str = "MedicationRequest";

/// Where a bulk-submit batch's file url points; any string will do, the
/// receipts are keyed by it.
const FILE_URL: &str = "https://provider.example/p.ndjson";

// ---------------------------------------------------------------------------
// Backends and database reads
// ---------------------------------------------------------------------------

/// The writer: a backend on the shared replica set whose driver connections
/// carry `app`, so a failpoint scoped to `app` holds only its commands.
async fn writer_backend(
    test: &str,
    app: &str,
    configure: impl FnOnce(&mut MongoBackendConfig),
) -> Option<Arc<MongoBackend>> {
    super::deferred_search_index::deferred_backend(test, app, None, configure)
        .await
        .map(Arc::new)
}

/// A second backend on `backend`'s database under another `appName`: the
/// instance that runs the purge. It knows nothing of the writer's jobs, which
/// is the point (several instances can share one database).
async fn second_instance(backend: &MongoBackend, app: &str) -> Arc<MongoBackend> {
    let config = MongoBackendConfig {
        connection_string: backend.config().connection_string.clone(),
        database_name: backend.config().database_name.clone(),
        app_name: Some(app.to_string()),
        data_dir: Some(repo_data_dir()),
        ..Default::default()
    };
    let purger = build_backend(config)
        .await
        .expect("a second backend on the same database");
    // Connected before any hold, so the purge's own work is all that runs in it.
    purger
        .count_deferred_search_index_chunks()
        .await
        .expect("warm the purger's connections");
    Arc::new(purger)
}

/// Counts of the documents of `tenant` per collection, over every collection of
/// the database but `system.*`. A document belongs to the tenant when its
/// `tenant_id` or, for the sync-failure ledger, its `_id.tenant_id` names it.
/// Collections that hold nothing of the tenant are left out.
async fn tenant_rows(client: &Client, db_name: &str, tenant: &str) -> BTreeMap<String, u64> {
    let db = client.database(db_name);
    let mut rows = BTreeMap::new();
    for name in db
        .list_collection_names()
        .await
        .expect("list the database's collections")
    {
        if name.starts_with("system.") {
            continue;
        }
        let count = db
            .collection::<Document>(&name)
            .count_documents(doc! { "$or": [
                { "tenant_id": tenant },
                { "_id.tenant_id": tenant },
            ]})
            .await
            .expect("count the tenant's documents");
        if count > 0 {
            rows.insert(name, count);
        }
    }
    rows
}

/// The `_id`s of the tenant's documents in one collection.
async fn row_ids(client: &Client, db_name: &str, collection: &str, tenant: &str) -> Vec<Bson> {
    let docs: Vec<Document> = client
        .database(db_name)
        .collection::<Document>(collection)
        .find(doc! { "tenant_id": tenant })
        .projection(doc! { "_id": 1 })
        .await
        .expect("find the tenant's row ids")
        .try_collect()
        .await
        .expect("collect the tenant's row ids");
    docs.into_iter()
        .filter_map(|mut document| document.remove("_id"))
        .collect()
}

/// How many documents of `collection` have an `_id` in `ids`.
async fn rows_with_ids(client: &Client, db_name: &str, collection: &str, ids: &[Bson]) -> u64 {
    client
        .database(db_name)
        .collection::<Document>(collection)
        .count_documents(doc! { "_id": { "$in": ids.to_vec() } })
        .await
        .expect("count rows by _id")
}

/// The `(search_index, search_index_contained)` rows of one resource.
async fn rows_of(client: &Client, db_name: &str, tenant: &str, id: &str) -> (u64, u64) {
    let mut counts = [0_u64; 2];
    for (slot, collection) in counts
        .iter_mut()
        .zip(["search_index", "search_index_contained"])
    {
        *slot = client
            .database(db_name)
            .collection::<Document>(collection)
            .count_documents(doc! {
                "tenant_id": tenant,
                "resource_type": RESOURCE_TYPE,
                "resource_id": id,
            })
            .await
            .expect("count one resource's rows");
    }
    (counts[0], counts[1])
}

/// What the kept tenant holds, after checking that it holds rows in every
/// collection a writer fills; `also` names more collections that must hold some.
async fn kept_snapshot(
    client: &Client,
    db_name: &str,
    kept: &str,
    also: &[&str],
) -> BTreeMap<String, u64> {
    let rows = tenant_rows(client, db_name, kept).await;
    for collection in [
        "resources",
        "resource_history",
        "search_index",
        "search_index_contained",
    ]
    .iter()
    .chain(also)
    {
        assert!(
            rows.get(*collection).copied().unwrap_or(0) > 0,
            "the kept tenant must hold rows in {collection}, or the test proves nothing: {rows:?}"
        );
    }
    rows
}

/// The rows assertions every test starts with, in the order the module docs
/// give: the purged tenant holds nothing, the kept tenant is unchanged.
async fn assert_only_kept_rows_remain(
    client: &Client,
    db_name: &str,
    purged: &str,
    kept: &str,
    kept_before: &BTreeMap<String, u64>,
) {
    let left = tenant_rows(client, db_name, purged).await;
    eprintln!("rows of the purged tenant after the purge and the writer returned: {left:?}");
    for collection in left.keys() {
        let samples: Vec<Document> = client
            .database(db_name)
            .collection::<Document>(collection)
            .find(doc! { "$or": [{ "tenant_id": purged }, { "_id.tenant_id": purged }] })
            .projection(doc! { "data": 0, "previous_content": 0, "operation_outcome": 0 })
            .limit(2)
            .await
            .expect("sample the leftover documents")
            .try_collect()
            .await
            .expect("collect the leftover documents");
        eprintln!("  {collection}, first documents: {samples:?}");
    }
    assert!(
        left.is_empty(),
        "the purged tenant still holds documents after purge_tenant_data and the writer \
         both returned (collection: documents): {left:?}"
    );
    assert_eq!(
        &tenant_rows(client, db_name, kept).await,
        kept_before,
        "the kept tenant's documents changed"
    );
}

// ---------------------------------------------------------------------------
// Holding a writer
// ---------------------------------------------------------------------------

/// Sets a failpoint that holds the next `command` on `collection` (a
/// server-side block of [`HOLD_MS`]) for connections whose `appName` is `app`
/// only, and nothing else.
///
/// `None` means the failpoint could not be set (a server without
/// `enableTestCommands`); that is a skip, or a failure under
/// `HFS_TEST_REQUIRE_MONGODB`.
async fn hold_next(app: &str, db_name: &str, command: &str, collection: &str) -> Option<FailPoint> {
    let fail_point = FailPoint::enable(
        app,
        doc! {
            "failCommands": [command],
            "namespace": format!("{db_name}.{collection}"),
            "blockConnection": true,
            "blockTimeMS": HOLD_MS,
        },
        doc! { "times": 1 },
    )
    .await;
    if fail_point.is_none() {
        skip_or_fail_unavailable("tenant_purge_races tests need enableTestCommands=1");
    }
    fail_point
}

/// Starts `writer`, holds its first `command` on `collection` (see
/// [`hold_next`], scoped to `app`), runs `during` once the server reports the
/// command held, and hands back both outputs after letting the writer finish.
async fn while_held<D, T>(
    app: &str,
    db_name: &str,
    command: &str,
    collection: &str,
    during: impl Future<Output = D>,
    writer: impl Future<Output = T> + Send + 'static,
) -> Option<(D, T)>
where
    T: Send + 'static,
{
    let fail_point = hold_next(app, db_name, command, collection).await?;
    let writer = tokio::spawn(writer);
    // The server counts a command when it enters the failpoint; this is how the
    // test knows the writer is held, with no sleep. It panics after 10 s if the
    // writer never sent `command` to `collection`.
    fail_point.wait_until_entered(1).await;
    let held = Instant::now();
    let during = during.await;
    assert!(
        !writer.is_finished(),
        "the {HOLD_MS} ms hold of `{command}` on `{collection}` ended ({:?} after it began) \
         before the action finished, so the writer was not held while it ran",
        held.elapsed()
    );
    fail_point.off().await;
    let written = writer.await.expect("the writer task panicked");
    Some((during, written))
}

/// [`while_held`] with `purger.purge_tenant_data(purged)` as the action.
async fn purge_while_held<T>(
    app: &str,
    db_name: &str,
    command: &str,
    collection: &str,
    purger: &MongoBackend,
    purged: &str,
    writer: impl Future<Output = T> + Send + 'static,
) -> Option<T>
where
    T: Send + 'static,
{
    while_held(
        app,
        db_name,
        command,
        collection,
        async {
            purger.purge_tenant_data(purged).await.expect("purge");
        },
        writer,
    )
    .await
    .map(|((), written)| written)
}

// ---------------------------------------------------------------------------
// $reindex
// ---------------------------------------------------------------------------

/// A MedicationRequest with a contained Medication, so its rows land in both
/// `search_index` and `search_index_contained`.
fn medication_request(id: &str) -> serde_json::Value {
    json!({
        "resourceType": RESOURCE_TYPE,
        "id": id,
        "status": "active",
        "intent": "order",
        "medicationReference": {"reference": "#med"},
        "subject": {"reference": "Patient/p1"},
        "contained": [{
            "resourceType": "Medication",
            "id": "med",
            "code": {
                "coding": [{
                    "system": "http://www.nlm.nih.gov/research/umls/rxnorm",
                    "code": "314076"
                }],
                "text": "Lisinopril"
            }
        }],
    })
}

fn resource_id(n: usize) -> String {
    format!("mr-{n:03}")
}

/// What a reindex test starts from: a writer and a purger on one database, a
/// kept and a purged tenant with the same resource ids, and the purged
/// tenant's resources as one page read before anything is held.
struct ReindexRace {
    writer: Arc<MongoBackend>,
    purger: Arc<MongoBackend>,
    app: String,
    client: Client,
    db_name: String,
    purged: TenantContext,
    kept: String,
    kept_rows: BTreeMap<String, u64>,
    page: Vec<StoredResource>,
}

impl ReindexRace {
    /// `first_build` empties the purged tenant's search rows before the page is
    /// read, so the page writer finds nothing to replace.
    async fn start(
        name: &str,
        resources: usize,
        first_build: bool,
        configure: impl FnOnce(&mut MongoBackendConfig),
    ) -> Option<Self> {
        let app = format!("fp-tpr-{name}");
        let writer = writer_backend(&format!("tpr_{name}"), &app, configure).await?;
        let purger = second_instance(&writer, &format!("{app}-purger")).await;
        let db_name = writer.config().database_name.clone();
        let client = raw_test_client(&writer.config().connection_string)
            .await
            .expect("raw client");
        let purged = create_tenant(&format!("tpr-{name}-purged"));
        let kept = create_tenant(&format!("tpr-{name}-kept"));
        for tenant in [&kept, &purged] {
            for n in 0..resources {
                writer
                    .seed_create(
                        tenant,
                        RESOURCE_TYPE,
                        medication_request(&resource_id(n)),
                        FhirVersion::default(),
                    )
                    .await
                    .expect("seed a MedicationRequest");
            }
        }
        let kept_rows = kept_snapshot(&client, &db_name, kept.tenant_id().as_str(), &[]).await;
        if first_build {
            writer
                .clear_search_index(&purged)
                .await
                .expect("empty the purged tenant's search rows");
        }
        let page = writer
            .fetch_resources_page(&purged, RESOURCE_TYPE, None, 100)
            .await
            .expect("read the purged tenant's page")
            .resources;
        assert_eq!(
            page.len(),
            resources,
            "the page holds every seeded resource"
        );
        let purged_before = tenant_rows(&client, &db_name, purged.tenant_id().as_str()).await;
        assert!(
            purged_before.get("resources").copied().unwrap_or(0) > 0,
            "the purged tenant holds resources before the race: {purged_before:?}"
        );
        Some(Self {
            writer,
            purger,
            app,
            client,
            db_name,
            kept: kept.tenant_id().as_str().to_string(),
            purged,
            kept_rows,
            page,
        })
    }

    fn purged_id(&self) -> &str {
        self.purged.tenant_id().as_str()
    }

    /// The purge, run while the page writer's `command` on `collection` is held.
    async fn purge_while_page_is_written(
        &self,
        command: &str,
        collection: &str,
    ) -> Option<(Vec<StorageResult<usize>>, ReindexPageStats)> {
        let writer = Arc::clone(&self.writer);
        let tenant = self.purged.clone();
        let page = self.page.clone();
        purge_while_held(
            &self.app,
            &self.db_name,
            command,
            collection,
            &self.purger,
            self.purged_id(),
            async move {
                let mut stats = ReindexPageStats::default();
                let outcomes = writer
                    .write_search_entries_page_timed(&tenant, &page, &mut stats)
                    .await;
                (outcomes, stats)
            },
        )
        .await
    }

    async fn assert_only_kept_rows_remain(&self) {
        assert_only_kept_rows_remain(
            &self.client,
            &self.db_name,
            self.purged_id(),
            &self.kept,
            &self.kept_rows,
        )
        .await;
    }

    /// Marks the purged tenant's resources as having stale `missing_params`
    /// metadata, so that a page write sends metadata updates instead of
    /// skipping every one of them as unchanged.
    async fn make_metadata_stale(&self) {
        self.client
            .database(&self.db_name)
            .collection::<Document>("resources")
            .update_many(
                doc! { "tenant_id": self.purged_id() },
                doc! { "$set": { "missing_params_fp": "stale-fingerprint" } },
            )
            .await
            .expect("make the stored metadata stale");
    }
}

fn serial(config: &mut MongoBackendConfig) {
    config.reindex_overlap = false;
}

fn print_outcomes(outcomes: &[StorageResult<usize>]) {
    eprintln!("page outcomes: {outcomes:?}");
}

/// The page replaces existing rows, so the page writer verifies what it wrote.
/// Its `search_index` insert is held while the whole purge runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_held_serial_page_insert_leaves_no_rows() {
    let Some(race) = ReindexRace::start("serial", 6, false, serial).await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let Some((outcomes, _)) = race
        .purge_while_page_is_written("insert", "search_index")
        .await
    else {
        return;
    };
    race.assert_only_kept_rows_remain().await;
    print_outcomes(&outcomes);
}

/// The page is read by id (`ReindexSource::fetch_resources_by_ids`, what the
/// catch-up and id-list passes use) instead of by cursor. Its resources must be
/// recognised as read from `resources` too, or the page writer takes a resource
/// that is no longer there for one from another store and keeps its rows.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_held_page_read_by_id_leaves_no_rows() {
    let Some(mut race) = ReindexRace::start("byid", 6, false, serial).await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let ids: Vec<String> = (0..race.page.len()).map(resource_id).collect();
    race.page =
        ReindexSource::fetch_resources_by_ids(&*race.writer, &race.purged, RESOURCE_TYPE, &ids)
            .await
            .expect("read the purged tenant's page by id");
    assert_eq!(race.page.len(), ids.len(), "every id is read");
    let Some((outcomes, _)) = race
        .purge_while_page_is_written("insert", "search_index")
        .await
    else {
        return;
    };
    race.assert_only_kept_rows_remain().await;
    print_outcomes(&outcomes);
}

/// A first build: the page found no rows to replace, so it skips its
/// verification.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_held_first_build_page_insert_leaves_no_rows() {
    let Some(race) = ReindexRace::start("firstbuild", 6, true, serial).await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let Some((outcomes, _)) = race
        .purge_while_page_is_written("insert", "search_index")
        .await
    else {
        return;
    };
    race.assert_only_kept_rows_remain().await;
    print_outcomes(&outcomes);
}

/// The own rows are in before the purge starts and the purge removes them; the
/// contained rows are what the held insert lands afterwards.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_held_contained_page_insert_leaves_no_rows() {
    let Some(race) = ReindexRace::start("contained", 6, false, serial).await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let Some((outcomes, _)) = race
        .purge_while_page_is_written("insert", "search_index_contained")
        .await
    else {
        return;
    };
    race.assert_only_kept_rows_remain().await;
    print_outcomes(&outcomes);
}

/// The page's first write is its row delete. Held there, the whole purge runs
/// between the page's read and its first write, so the delete finds nothing and
/// the inserts follow.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_held_page_delete_leaves_no_rows() {
    let Some(race) = ReindexRace::start("delete", 6, false, serial).await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let Some((outcomes, _)) = race
        .purge_while_page_is_written("delete", "search_index")
        .await
    else {
        return;
    };
    race.assert_only_kept_rows_remain().await;
    print_outcomes(&outcomes);
}

/// More than `REINDEX_SUBBATCH_FIRST` resources on the overlapped writer: the
/// held insert is the first sub-batch's, and the second sub-batch inserts after
/// the purge as well.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_held_overlapped_sub_batch_leaves_no_rows() {
    let Some(race) = ReindexRace::start("overlap", 40, false, |c| {
        c.reindex_overlap = true;
        c.reindex_prepare_threads = 1;
    })
    .await
    else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let Some((outcomes, stats)) = race
        .purge_while_page_is_written("insert", "search_index")
        .await
    else {
        return;
    };
    race.assert_only_kept_rows_remain().await;
    print_outcomes(&outcomes);
    assert!(
        stats.sub_batches >= 2,
        "40 resources must split into at least two sub-batches on the overlapped path: {}",
        stats.sub_batches
    );
}

/// A whole `$reindex` job, with the page prefetch on, whose first page insert
/// is held. The job must end `Completed` and the tenant must leave no rows.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_reindex_job_leaves_no_rows_and_the_job_completes() {
    let Some(race) = ReindexRace::start("job", 12, false, |c| {
        c.reindex_prefetch = true;
        c.reindex_prepare_threads = 1;
    })
    .await
    else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let writer = Arc::clone(&race.writer);
    let tenant = race.purged.clone();
    let Some(progress) = purge_while_held(
        &race.app,
        &race.db_name,
        "insert",
        "search_index",
        &race.purger,
        race.purged_id(),
        async move {
            let op = ReindexOperation::new(writer.clone(), writer.tenant_registries().clone())
                .with_prefetch(true);
            let job = op
                .start(
                    tenant,
                    ReindexRequest::for_types([RESOURCE_TYPE]).with_batch_size(5),
                    None,
                )
                .await
                .expect("start the reindex job");
            super::reindex_id_walk::wait_for_terminal(&op, &job).await
        },
    )
    .await
    else {
        return;
    };
    race.assert_only_kept_rows_remain().await;
    eprintln!("reindex job ended: {progress:?}");
    assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
    assert!(!progress.has_errors(), "{progress:?}");
}

/// The metadata-only writer updates `resources`; its statement never inserts,
/// so a purge that ran first leaves it nothing to match.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_held_metadata_only_page_leaves_no_rows() {
    let Some(race) = ReindexRace::start("metaonly", 6, false, serial).await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    race.make_metadata_stale().await;
    let writer = Arc::clone(&race.writer);
    let tenant = race.purged.clone();
    let page = race.page.clone();
    let Some((results, written, failures)) = purge_while_held(
        &race.app,
        &race.db_name,
        "update",
        "resources",
        &race.purger,
        race.purged_id(),
        async move {
            writer
                .write_search_metadata_page(&tenant, &page, false)
                .await
        },
    )
    .await
    else {
        return;
    };
    race.assert_only_kept_rows_remain().await;
    eprintln!(
        "metadata-only page: {} results, {written} written, {} failures",
        results.len(),
        failures.len()
    );
}

/// The page writer's last step is a metadata update on `resources`, after its
/// rows are in. Held there, the purge removes the rows and the resources, and
/// the update then matches nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_held_page_metadata_update_leaves_no_rows() {
    let Some(race) = ReindexRace::start("pagemeta", 6, false, serial).await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    race.make_metadata_stale().await;
    let Some((outcomes, _)) = race
        .purge_while_page_is_written("update", "resources")
        .await
    else {
        return;
    };
    race.assert_only_kept_rows_remain().await;
    print_outcomes(&outcomes);
}

/// No purge here: one resource of the page is removed from `resources` while
/// the page's insert is held. Its rows must not outlive it, and its siblings,
/// which still exist, keep exactly the rows they had.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hard_removed_resource_during_a_held_page_loses_only_its_own_rows() {
    let Some(race) = ReindexRace::start("hardremove", 6, false, serial).await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = race.purged_id().to_string();
    let gone = resource_id(2);
    let mut before = HashMap::new();
    for n in 0..race.page.len() {
        let id = resource_id(n);
        let rows = rows_of(&race.client, &race.db_name, &tenant, &id).await;
        assert!(rows.0 > 0 && rows.1 > 0, "{id} has rows in both: {rows:?}");
        before.insert(id, rows);
    }

    let writer = Arc::clone(&race.writer);
    let page_tenant = race.purged.clone();
    let page = race.page.clone();
    let Some((removed, (outcomes, _))) = while_held(
        &race.app,
        &race.db_name,
        "insert",
        "search_index",
        async {
            race.client
                .database(&race.db_name)
                .collection::<Document>("resources")
                .delete_one(doc! {
                    "tenant_id": &tenant,
                    "resource_type": RESOURCE_TYPE,
                    "id": &gone,
                })
                .await
                .expect("remove one resource from `resources`")
                .deleted_count
        },
        async move {
            let mut stats = ReindexPageStats::default();
            let outcomes = writer
                .write_search_entries_page_timed(&page_tenant, &page, &mut stats)
                .await;
            (outcomes, stats)
        },
    )
    .await
    else {
        return;
    };
    assert_eq!(removed, 1, "one resource was removed");

    let gone_rows = rows_of(&race.client, &race.db_name, &tenant, &gone).await;
    eprintln!("rows of the removed resource {gone}: {gone_rows:?}");
    assert_eq!(
        gone_rows,
        (0, 0),
        "the removed resource still has rows (search_index, search_index_contained)"
    );
    for (id, rows) in &before {
        if *id != gone {
            assert_eq!(
                &rows_of(&race.client, &race.db_name, &tenant, id).await,
                rows,
                "{id} still exists, so its rows must be what they were"
            );
        }
    }
    print_outcomes(&outcomes);
}

/// The purge is followed by a new write of a resource the page also holds, made
/// by another writer while the page's insert is held. Rows that writer added
/// are not the page's rows, whatever happens to the page's own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_held_page_keeps_rows_another_writer_added_after_it() {
    let Some(race) = ReindexRace::start("recreate", 6, false, serial).await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = race.purged_id().to_string();
    let writer = Arc::clone(&race.writer);
    let page_tenant = race.purged.clone();
    let page = race.page.clone();
    let Some((added, (outcomes, _))) = while_held(
        &race.app,
        &race.db_name,
        "insert",
        "search_index",
        async {
            race.purger.purge_tenant_data(&tenant).await.expect("purge");
            race.purger
                .seed_create(
                    &race.purged,
                    RESOURCE_TYPE,
                    medication_request(&resource_id(0)),
                    FhirVersion::default(),
                )
                .await
                .expect("create the resource again after the purge");
            // The purge emptied both collections, so these rows are the new
            // resource's alone.
            let own = row_ids(&race.client, &race.db_name, "search_index", &tenant).await;
            let contained = row_ids(
                &race.client,
                &race.db_name,
                "search_index_contained",
                &tenant,
            )
            .await;
            (own, contained)
        },
        async move {
            let mut stats = ReindexPageStats::default();
            let outcomes = writer
                .write_search_entries_page_timed(&page_tenant, &page, &mut stats)
                .await;
            (outcomes, stats)
        },
    )
    .await
    else {
        return;
    };
    let (own, contained) = added;
    assert!(
        !own.is_empty() && !contained.is_empty(),
        "the new resource has rows of its own"
    );
    assert_eq!(
        rows_with_ids(&race.client, &race.db_name, "search_index", &own).await,
        own.len() as u64,
        "rows the other writer added to search_index are gone"
    );
    assert_eq!(
        rows_with_ids(
            &race.client,
            &race.db_name,
            "search_index_contained",
            &contained
        )
        .await,
        contained.len() as u64,
        "rows the other writer added to search_index_contained are gone"
    );
    print_outcomes(&outcomes);
}

// ---------------------------------------------------------------------------
// $bulk-submit ingest and the submit worker
// ---------------------------------------------------------------------------

/// `n` NDJSON lines of MedicationRequests with ids `mr-000..`; `note` makes
/// the content differ between two ingests of the same ids.
fn entries(n: usize, note: &str) -> Vec<NdjsonEntry> {
    (0..n)
        .map(|i| {
            let mut resource = medication_request(&resource_id(i));
            resource["note"] = json!([{ "text": note }]);
            NdjsonEntry::new(i as u64 + 1, RESOURCE_TYPE, resource)
        })
        .collect()
}

/// What a bulk-submit test starts from: a writer and a purger on one database,
/// a kept tenant that already ingested one batch (so it holds rows in every
/// `bulk_*` collection a batch fills), and a purged tenant with a submission
/// and one fetchable manifest.
struct BulkRace {
    writer: Arc<MongoBackend>,
    purger: Arc<MongoBackend>,
    app: String,
    client: Client,
    db_name: String,
    purged: TenantContext,
    kept: String,
    kept_rows: BTreeMap<String, u64>,
    submission: SubmissionId,
    manifest_id: String,
}

impl BulkRace {
    async fn start(name: &str) -> Option<Self> {
        let app = format!("fp-tpr-{name}");
        let writer = writer_backend(&format!("tpr_{name}"), &app, |_| {}).await?;
        let purger = second_instance(&writer, &format!("{app}-purger")).await;
        let db_name = writer.config().database_name.clone();
        let client = raw_test_client(&writer.config().connection_string)
            .await
            .expect("raw client");
        let kept = create_tenant(&format!("tpr-{name}-kept"));
        let purged = create_tenant(&format!("tpr-{name}-purged"));

        let (kept_submission, kept_manifest) = super::bulk_submit::seed(&writer, &kept).await;
        writer
            .process_entries(
                &kept,
                &kept_submission,
                &kept_manifest,
                entries(5, "kept"),
                &BulkProcessingOptions::new().with_file_url(FILE_URL),
            )
            .await
            .expect("ingest the kept tenant's batch");
        let kept_rows = kept_snapshot(
            &client,
            &db_name,
            kept.tenant_id().as_str(),
            &[
                "bulk_submissions",
                "bulk_manifests",
                "bulk_entry_results",
                "bulk_submission_changes",
            ],
        )
        .await;

        let (submission, manifest_id) = super::bulk_submit::seed(&writer, &purged).await;
        let purged_before = tenant_rows(&client, &db_name, purged.tenant_id().as_str()).await;
        assert!(
            purged_before.contains_key("bulk_manifests"),
            "the purged tenant holds its manifest before the race: {purged_before:?}"
        );
        Some(Self {
            writer,
            purger,
            app,
            client,
            db_name,
            kept: kept.tenant_id().as_str().to_string(),
            purged,
            kept_rows,
            submission,
            manifest_id,
        })
    }

    fn purged_id(&self) -> &str {
        self.purged.tenant_id().as_str()
    }

    /// The `appName` of the connections that run the purge.
    fn purger_app(&self) -> String {
        format!("{}-purger", self.app)
    }

    /// One batch through `process_entries` on the purged tenant's manifest.
    fn ingest(
        &self,
        entries: Vec<NdjsonEntry>,
        file_url: &str,
    ) -> impl Future<Output = StorageResult<Vec<BulkEntryResult>>> + Send + use<> {
        let writer = Arc::clone(&self.writer);
        let tenant = self.purged.clone();
        let submission = self.submission.clone();
        let manifest_id = self.manifest_id.clone();
        let file_url = file_url.to_string();
        async move {
            writer
                .process_entries(
                    &tenant,
                    &submission,
                    &manifest_id,
                    entries,
                    &BulkProcessingOptions::new().with_file_url(file_url),
                )
                .await
        }
    }

    async fn assert_only_kept_rows_remain(&self) {
        assert_only_kept_rows_remain(
            &self.client,
            &self.db_name,
            self.purged_id(),
            &self.kept,
            &self.kept_rows,
        )
        .await;
    }
}

/// One line for the log: how a batch ended.
fn describe(result: &StorageResult<Vec<BulkEntryResult>>) -> String {
    match result {
        Ok(results) => format!(
            "Ok: {} results, {} successful",
            results.len(),
            results.iter().filter(|r| r.is_success()).count()
        ),
        Err(err) => format!("Err: {err}"),
    }
}

/// A batch whose tenant was purged under it ends with the error the next batch
/// of its file gets: its manifest is gone.
fn assert_manifest_not_found(result: &StorageResult<Vec<BulkEntryResult>>) {
    assert!(
        matches!(
            result,
            Err(StorageError::BulkSubmit(
                BulkSubmitError::ManifestNotFound { .. }
            ))
        ),
        "a batch whose manifest was purged ends with ManifestNotFound: {}",
        describe(result)
    );
}

/// Holds the batch's first `command` on `collection` while the purge runs.
async fn ingest_race(name: &str, command: &str, collection: &str) {
    let Some(race) = BulkRace::start(name).await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let Some(result) = purge_while_held(
        &race.app,
        &race.db_name,
        command,
        collection,
        &race.purger,
        race.purged_id(),
        race.ingest(entries(5, "v1"), FILE_URL),
    )
    .await
    else {
        return;
    };
    eprintln!("the batch ended: {}", describe(&result));
    race.assert_only_kept_rows_remain().await;
    assert_manifest_not_found(&result);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_held_ingest_resource_insert_leaves_no_rows() {
    ingest_race("resins", "insert", "resources").await;
}

/// The ids exist before the batch, so it updates them; the update that is held
/// is the first of the batch's per-resource updates, and the others have
/// landed by the time the purge runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_held_ingest_resource_update_leaves_no_rows() {
    let Some(race) = BulkRace::start("resupd").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    race.ingest(entries(3, "v1"), "https://provider.example/first.ndjson")
        .await
        .expect("create the resources the held batch updates");
    let Some(result) = purge_while_held(
        &race.app,
        &race.db_name,
        "update",
        "resources",
        &race.purger,
        race.purged_id(),
        race.ingest(entries(3, "v2"), "https://provider.example/second.ndjson"),
    )
    .await
    else {
        return;
    };
    eprintln!("the batch ended: {}", describe(&result));
    race.assert_only_kept_rows_remain().await;
    assert_manifest_not_found(&result);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_held_ingest_history_insert_leaves_no_rows() {
    ingest_race("histins", "insert", "resource_history").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_held_ingest_search_insert_leaves_no_rows() {
    ingest_race("srchins", "insert", "search_index").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_held_ingest_contained_insert_leaves_no_rows() {
    ingest_race("contins", "insert", "search_index_contained").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_held_ingest_change_log_insert_leaves_no_rows() {
    ingest_race("chgins", "insert", "bulk_submission_changes").await;
}

/// The receipts are one `update` command with upserts, the last write of a batch.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_held_ingest_receipt_upsert_leaves_no_rows() {
    ingest_race("rcptupd", "update", "bulk_entry_results").await;
}

/// `record_submit_file` reads the manifest to check its fence and then upserts
/// the artifact row: a purge between the two is the race.
///
/// While the upsert is held, a heartbeat on the purged manifest is also asked
/// for. The lease keeper's heartbeat is what lets the worker drop work in
/// flight, and the upsert is work in flight, so the backend must answer it
/// with a renewal until the upsert has ended.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_held_submit_file_upsert_leaves_no_rows() {
    let Some(race) = BulkRace::start("subfile").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let lease = race
        .writer
        .claim_next_manifest(
            &WorkerId::new("tpr-subfile-worker"),
            Duration::from_secs(60),
        )
        .await
        .expect("claim")
        .expect("the purged tenant's manifest is claimable");
    assert_eq!(
        lease.tenant.tenant_id().as_str(),
        race.purged_id(),
        "the claimed manifest is the purged tenant's"
    );
    let record = SubmitFileRecord {
        manifest_url: Some("https://provider.example/manifest.json".to_string()),
        file_type: "output".to_string(),
        resource_type: Some(RESOURCE_TYPE.to_string()),
        part_index: 0,
        file_path: "tenant/job/output/MedicationRequest-0.ndjson".to_string(),
        line_count: 3,
        byte_count: 120,
        count_severity: None,
    };
    let writer = Arc::clone(&race.writer);
    let held_lease = lease.clone();
    let Some((heartbeat, result)) = while_held(
        &race.app,
        &race.db_name,
        "update",
        "bulk_submit_files",
        async {
            race.purger
                .purge_tenant_data(race.purged_id())
                .await
                .expect("purge");
            race.writer.heartbeat(&lease).await
        },
        async move { writer.record_submit_file(&held_lease, &record).await },
    )
    .await
    else {
        return;
    };
    eprintln!("record_submit_file ended: {result:?}; the heartbeat during the hold: {heartbeat:?}");
    race.assert_only_kept_rows_remain().await;
    assert!(
        matches!(result, Err(LeaseError::LeaseLost { .. })),
        "a submit file recorded for a purged manifest ends as a lost lease: {result:?}"
    );
    assert!(
        heartbeat.is_ok(),
        "a heartbeat for a purged manifest, asked for while a write of that manifest is \
         still running on this instance, must be answered with a renewal, or the worker \
         drops the write before its check: {heartbeat:?}"
    );
}

/// Serves one manifest and its files from memory (the harness's own
/// `MockFetcher` is private to its module).
struct SubmitInputFetcherOf {
    manifest: RemoteManifest,
    files: HashMap<String, Vec<u8>>,
}

#[async_trait::async_trait]
impl SubmitInputFetcher for SubmitInputFetcherOf {
    async fn fetch_manifest(
        &self,
        _url: &str,
        _headers: &[(String, String)],
        _oauth: &[String],
        _key: Option<&serde_json::Value>,
    ) -> StorageResult<RemoteManifest> {
        Ok(self.manifest.clone())
    }

    async fn open_file_stream(
        &self,
        url: &str,
        _headers: &[(String, String)],
        _requires_access_token: bool,
        _oauth: &[String],
        _key: Option<&serde_json::Value>,
    ) -> StorageResult<(Box<dyn tokio::io::AsyncBufRead + Send + Unpin>, Option<u64>)> {
        let data = self.files.get(url).cloned().unwrap_or_default();
        let len = data.len() as u64;
        Ok((
            Box::new(tokio::io::BufReader::new(std::io::Cursor::new(data))),
            Some(len),
        ))
    }
}

/// Every file under `dir`, relative to it, for the log.
fn files_under(dir: &std::path::Path) -> Vec<String> {
    fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if let Ok(relative) = path.strip_prefix(root) {
                out.push(relative.display().to_string());
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// A whole claimed manifest run by the submit worker, with the first insert of
/// its first batch held while the tenant is purged. The worker ends without a
/// panic and without error.
///
/// The lease is 6 s, so the lease keeper's heartbeat (a third of what is left
/// of the lease, a second at the least) falls into the 10 s hold. The purge has
/// deleted the manifest by then, so the heartbeat's fenced write matches
/// nothing. The worker drops the batch it has in flight when its keeper reports
/// a lost lease, and a dropped batch cannot run the manifest check that removes
/// what it wrote, so the one command already sent would land with nothing
/// after it. What this test pins is that the backend does not report the lease
/// lost to the worker while a batch of that manifest is still running on this
/// instance, so the batch ends by itself, with its check.
///
/// The lease is claimed inside the held task, after the failpoint lock was won,
/// so that the time spent waiting for the lock is not spent from the lease.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_a_submit_worker_leaves_no_rows_and_the_job_ends() {
    let Some(race) = BulkRace::start("worker").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let file = "https://provider.example/mr.ndjson";
    let ndjson: String = entries(5, "worker")
        .iter()
        .map(|entry| format!("{}\n", entry.resource))
        .collect();
    let fetcher = Arc::new(SubmitInputFetcherOf {
        manifest: RemoteManifest {
            requires_access_token: false,
            output: vec![RemoteFile {
                resource_type: Some(RESOURCE_TYPE.to_string()),
                url: file.to_string(),
                count: Some(5),
            }],
            deleted: vec![],
        },
        files: HashMap::from([(file.to_string(), ndjson.into_bytes())]),
    });
    let output_dir = tempfile::tempdir().expect("tempdir");
    let output = Arc::new(
        helios_persistence::backends::local_fs::LocalFsOutputStore::new(
            output_dir.path().to_path_buf(),
            "http://localhost:8080",
        ),
    );
    let worker_id = WorkerId::new("tpr-worker");
    let worker =
        DefaultSubmitWorker::new(Arc::clone(&race.writer), fetcher, output, worker_id.clone());
    let writer = Arc::clone(&race.writer);
    let claim_id = worker_id.clone();
    let Some((claimed, ended)) = purge_while_held(
        &race.app,
        &race.db_name,
        "insert",
        "resources",
        &race.purger,
        race.purged_id(),
        async move {
            let lease = writer
                .claim_next_manifest(&claim_id, Duration::from_secs(6))
                .await
                .expect("claim")
                .expect("the purged tenant's manifest is claimable");
            let claimed = lease.tenant.tenant_id().as_str().to_string();
            (claimed, worker.run_job(lease).await)
        },
    )
    .await
    else {
        return;
    };
    assert_eq!(
        claimed,
        race.purged_id(),
        "the claimed manifest is the purged tenant's"
    );
    eprintln!("run_job ended: {ended:?}");
    eprintln!(
        "the worker's output directory holds: {:?}",
        files_under(output_dir.path())
    );
    race.assert_only_kept_rows_remain().await;
    assert!(
        ended.is_ok(),
        "the worker ends a purged manifest's run without an error: {ended:?}"
    );
}

/// What a lost-lease answer for a purged manifest does while a batch of that
/// manifest is running (#2310).
///
/// The submit worker drops the batch it has in flight as soon as the backend
/// reports the lease lost, and a dropped batch never runs the manifest check
/// that removes what it wrote. So while a batch of the manifest is running on
/// this instance, the backend must not report the manifest lost because it was
/// purged: a heartbeat is answered with a renewal, and the progress write that
/// the worker's file tasks turn into a lost lease (and so into a drop of their
/// siblings' batches) waits until the batch has ended. A fence that another
/// worker moved on a manifest that still exists is a different matter and loses
/// at once.
///
/// The batch and the calls that follow the purge run in this test's task, and
/// are polled together with the batch first. A batch that ends drops its
/// in-flight mark in the poll that returns its result, and `select` returns at
/// the first future that is ready, so a call that waits for the batch can only
/// come second. No clock is involved. A call that comes back while the batch
/// is still running ends the test.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lost_lease_answer_waits_for_the_batch_of_a_purged_manifest() {
    let Some(race) = BulkRace::start("leasewait").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    // Long enough that no lease of its own can expire in the hold.
    let lease = race
        .writer
        .claim_next_manifest(
            &WorkerId::new("tpr-leasewait-worker"),
            Duration::from_secs(300),
        )
        .await
        .expect("claim")
        .expect("the purged tenant's manifest is claimable");
    assert_eq!(
        lease.tenant.tenant_id().as_str(),
        race.purged_id(),
        "the claimed manifest is the purged tenant's"
    );
    // The lease of a worker that was passed by a newer claim.
    let mut stale = lease.clone();
    stale.fencing_token -= 1;

    let Some(fail_point) = hold_next(&race.app, &race.db_name, "insert", "resources").await else {
        return;
    };
    let batch_done = std::sync::atomic::AtomicBool::new(false);
    let batch = race.ingest(entries(5, "v1"), FILE_URL);
    let batch = async {
        let result = batch.await;
        batch_done.store(true, std::sync::atomic::Ordering::SeqCst);
        result
    };
    let after = async {
        fail_point.wait_until_entered(1).await;
        let held = Instant::now();
        let stale_answer = race.writer.heartbeat(&stale).await;
        assert!(
            matches!(stale_answer, Err(LeaseError::LeaseLost { .. })),
            "a fence that someone else moved, on a manifest that still exists, loses at \
             once, with a batch running or not: {stale_answer:?}"
        );
        race.purger
            .purge_tenant_data(race.purged_id())
            .await
            .expect("purge");
        assert!(
            !batch_done.load(std::sync::atomic::Ordering::SeqCst),
            "the {HOLD_MS} ms hold ended ({:?} after it began) before the purge returned, \
             so the batch was not held while it ran",
            held.elapsed()
        );
        let renewal = race.writer.heartbeat(&lease).await;
        assert!(
            renewal.is_ok(),
            "a heartbeat for a purged manifest, asked for while a batch of that manifest is \
             running on this instance, must be answered with a renewal, or the worker drops \
             the batch before its check: {renewal:?}"
        );
        race.writer.add_manifest_progress(&lease, 0, 1, 0).await
    };
    let result = {
        let batch = std::pin::pin!(batch);
        let after = std::pin::pin!(after);
        match select(batch, after).await {
            Either::Left((result, _unfinished)) => result,
            Either::Right((progress, _unfinished)) => panic!(
                "the lost-lease answer of a progress write for a purged manifest came while \
                 its batch was still running: {progress:?}"
            ),
        }
    };
    fail_point.off().await;

    // The batch has ended, so nothing of this manifest is running any more and
    // the manifest is gone: both writes now report what the worker may act on.
    let progress = race.writer.add_manifest_progress(&lease, 0, 1, 0).await;
    let heartbeat = race.writer.heartbeat(&lease).await;
    eprintln!(
        "the batch ended: {}; afterwards the progress write answered {progress:?} and the \
         heartbeat {heartbeat:?}",
        describe(&result)
    );
    race.assert_only_kept_rows_remain().await;
    assert!(
        matches!(progress, Err(LeaseError::LeaseLost { .. })),
        "once the batch has ended, a progress write for the purged manifest is a lost lease: \
         {progress:?}"
    );
    assert!(
        matches!(heartbeat, Err(LeaseError::LeaseLost { .. })),
        "once the batch has ended, a heartbeat for the purged manifest is a lost lease: \
         {heartbeat:?}"
    );
    assert_manifest_not_found(&result);
}

/// The window the purge's early delete of the manifests closes: the purge is
/// held after its deletes of the ledger, the manifests and `resources`, and a
/// batch for the purged tenant runs, whole, in that gap.
///
/// The batch finds its manifest gone and writes nothing. Were the manifests
/// deleted only after `resources`, the batch would find its manifest, write
/// `resources` and everything else, and keep it: its check would see a
/// manifest, and the purge, which has deleted `resources` already, would not
/// look at `resources` again.
///
/// Here the held command is the purge's, so the hold is on the purger's
/// `appName`, and the purge runs in a task of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_batch_starting_while_the_purge_is_held_past_its_resources_delete_writes_nothing() {
    let Some(race) = BulkRace::start("purgerheld").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    // The first delete the purge sends to `search_index`: the ledger of
    // pending rows, the manifests and `resources` come before it.
    let Some(fail_point) =
        hold_next(&race.purger_app(), &race.db_name, "delete", "search_index").await
    else {
        return;
    };
    let purge = tokio::spawn({
        let purger = Arc::clone(&race.purger);
        let tenant = race.purged_id().to_string();
        async move { purger.purge_tenant_data(&tenant).await }
    });
    fail_point.wait_until_entered(1).await;
    let result = race.ingest(entries(5, "v1"), FILE_URL).await;
    assert!(
        !purge.is_finished(),
        "the purge ended before the batch did, so it was not held while the batch ran"
    );
    fail_point.off().await;
    purge
        .await
        .expect("the purge task panicked")
        .expect("purge");
    eprintln!("the batch ended: {}", describe(&result));
    race.assert_only_kept_rows_remain().await;
    assert_manifest_not_found(&result);
}

/// The purge is followed by a new write of one of the batch's ids by another
/// writer while the batch's resource insert is held. The batch lands around it;
/// what the other writer made stays and nothing else of the tenant does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_purge_racing_an_ingest_keeps_the_recreated_tenants_rows() {
    let Some(race) = BulkRace::start("ingrecreate").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = race.purged_id().to_string();
    let recreated_id = resource_id(0);
    let Some((recreated, result)) = while_held(
        &race.app,
        &race.db_name,
        "insert",
        "resources",
        async {
            race.purger.purge_tenant_data(&tenant).await.expect("purge");
            race.purger
                .seed_create(
                    &race.purged,
                    RESOURCE_TYPE,
                    medication_request(&recreated_id),
                    FhirVersion::default(),
                )
                .await
                .expect("create the resource again after the purge");
            let document = race
                .client
                .database(&race.db_name)
                .collection::<Document>("resources")
                .find_one(doc! {
                    "tenant_id": &tenant,
                    "resource_type": RESOURCE_TYPE,
                    "id": &recreated_id,
                })
                .await
                .expect("read the re-created resource")
                .expect("the re-created resource exists");
            let rows = tenant_rows(&race.client, &race.db_name, &tenant).await;
            (document, rows)
        },
        race.ingest(entries(5, "v1"), FILE_URL),
    )
    .await
    else {
        return;
    };
    eprintln!("the batch ended: {}", describe(&result));
    let (document, rows_before) = recreated;
    let after = race
        .client
        .database(&race.db_name)
        .collection::<Document>("resources")
        .find_one(doc! {
            "tenant_id": &tenant,
            "resource_type": RESOURCE_TYPE,
            "id": &recreated_id,
        })
        .await
        .expect("read the re-created resource again")
        .expect("the re-created resource survived");
    assert_eq!(
        (after.get("version_id"), after.get("last_updated")),
        (document.get("version_id"), document.get("last_updated")),
        "the re-created resource was replaced"
    );
    let rows_after = tenant_rows(&race.client, &race.db_name, &tenant).await;
    eprintln!("rows of the tenant: {rows_before:?} before the batch landed, {rows_after:?} after");
    assert_eq!(
        rows_after, rows_before,
        "the tenant holds more than the re-created resource and its rows (collection: documents)"
    );
    assert_eq!(
        tenant_rows(&race.client, &race.db_name, &race.kept).await,
        race.kept_rows,
        "the kept tenant's documents changed"
    );
    assert_manifest_not_found(&result);
}

// ---------------------------------------------------------------------------
// What the guards cost
// ---------------------------------------------------------------------------

/// Switches the server's profiler on for the database. `false` when the server
/// refuses (the tests that need it print why and return).
async fn start_profiling(db: &mongodb::Database) -> bool {
    db.run_command(doc! { "profile": 2_i32 }).await.is_ok()
}

/// Switches the profiler off, counts what it recorded for `namespace` with
/// operation `op`, and clears the record so the next measurement starts empty.
async fn stop_profiling_and_count(db: &mongodb::Database, namespace: &str, op: &str) -> u64 {
    db.run_command(doc! { "profile": 0_i32 })
        .await
        .expect("disable profiling");
    let profile = db.collection::<Document>("system.profile");
    let count = profile
        .count_documents(doc! { "ns": namespace, "op": op })
        .await
        .expect("read system.profile");
    profile.drop().await.expect("clear system.profile");
    count
}

/// The page writers read `resources` as often as they did before the guard
/// (#2310): once per type on a first build (the metadata read), twice on a
/// rebuild (verification's version read and the metadata read). The guard
/// reuses those reads, so a page whose resources all exist pays nothing for it.
/// A page that does meet a missing resource is not measured here.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reindex_page_reads_resources_as_often_as_before() {
    let Some(race) = ReindexRace::start("readcount", 6, true, serial).await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let db = race.client.database(&race.db_name);
    let namespace = format!("{}.resources", race.db_name);
    if !start_profiling(&db).await {
        skip_or_fail_unavailable("the server refused {profile: 2}");
        return;
    }
    let mut stats = ReindexPageStats::default();
    let first_build = race
        .writer
        .write_search_entries_page_timed(&race.purged, &race.page, &mut stats)
        .await;
    let first_build_reads = stop_profiling_and_count(&db, &namespace, "query").await;
    assert!(first_build.iter().all(Result::is_ok), "{first_build:?}");

    // The rows exist now, so the same page is a rebuild.
    assert!(start_profiling(&db).await);
    let rebuild = race
        .writer
        .write_search_entries_page_timed(&race.purged, &race.page, &mut stats)
        .await;
    let rebuild_reads = stop_profiling_and_count(&db, &namespace, "query").await;
    assert!(rebuild.iter().all(Result::is_ok), "{rebuild:?}");

    eprintln!(
        "reads of `resources` by one page of one type: first build {first_build_reads}, rebuild {rebuild_reads}"
    );
    assert_eq!(
        first_build_reads, 1,
        "a first-build page reads `resources` once"
    );
    assert_eq!(rebuild_reads, 2, "a rebuild page reads `resources` twice");
}

/// A batch reads its manifest three times: before it writes, after it writes
/// (the purge guard of #2310), and when it charges its counters. Without the
/// guard it is two.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_ingest_batch_reads_its_manifest_once_more_after_its_writes() {
    let Some(race) = BulkRace::start("readmanifest").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let db = race.client.database(&race.db_name);
    let namespace = format!("{}.bulk_manifests", race.db_name);
    if !start_profiling(&db).await {
        skip_or_fail_unavailable("the server refused {profile: 2}");
        return;
    }
    let result = race.ingest(entries(5, "v1"), FILE_URL).await;
    let manifest_reads = stop_profiling_and_count(&db, &namespace, "query").await;
    let results = result.expect("the batch ingests");
    assert!(results.iter().all(|r| r.is_success()), "{results:?}");

    eprintln!("reads of `bulk_manifests` by one batch with a file url: {manifest_reads}");
    assert_eq!(
        manifest_reads, 3,
        "the presence check, the guard after the writes, and the counter charge"
    );
}
