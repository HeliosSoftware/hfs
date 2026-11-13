//! #2146: `purge` and `purge_all` remove a resource from four collections
//! (`resources`, `resource_history`, `search_index`, `search_index_contained`)
//! and clear its bulk-submit `previous_content` and its sync failures. They used
//! to send those commands one at a time with no transaction, so a failed command
//! part-way left a resource without its history, or without its rows, and a
//! repeated `purge` answered `NotFound` and never reached the rest.
//!
//! `purge` is now one transaction where the deployment has them, a repeated
//! `purge` finishes a half-done one (`NotFound` is answered only when the six
//! writes changed nothing), a resource with too many versions is purged without
//! a transaction (and so is one whose transaction is too large for the cache, or
//! is rolled back under cache pressure twice: a purge never answers `503` for
//! the cache), and `purge_all` removes a type in batches of ids, one transaction
//! per batch, halving a batch that cannot run in one. Whether a fallback really
//! ran without a transaction is read off the oplog of the test's own database
//! (`Oplog`): the deletes of a committed transaction are one `applyOps` entry with
//! a `txnNumber`, those of a purge without one are entries of their own (or a
//! batched delete's `applyOps`, which has no `txnNumber`).
//!
//! Child module of the `mongodb_tests` root, like `write_abort_retry.rs`:
//! `use super::*;` reaches its harness. The failure tests drive the server's
//! `failCommand` failpoint, scoped to the backend's `appName` and, where it
//! matters, to one collection, against a backend with `retryWrites=false` so the
//! driver's own retries do not muddy the counts. A standalone `mongod` has no
//! transactions, and the fallback that runs a purge without one cannot be
//! exercised here: the harness's server is a replica set.

use super::*;

use crate::bulk_submit::FailPoint;
use futures::TryStreamExt;
use mongodb::bson::Timestamp;
use std::collections::BTreeSet;
use std::time::Duration;

/// Attempts a write makes before it gives up (`WRITE_ABORT_RETRY`).
const WRITE_ABORT_ATTEMPTS: i64 = 4;

/// What the four collections hold for a purge's scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Held {
    resources: u64,
    history: u64,
    rows: u64,
    contained: u64,
}

impl Held {
    fn is_empty(self) -> bool {
        self == Self {
            resources: 0,
            history: 0,
            rows: 0,
            contained: 0,
        }
    }
}

async fn count_in(db: &mongodb::Database, collection: &str, filter: Document) -> u64 {
    db.collection::<Document>(collection)
        .count_documents(filter)
        .await
        .unwrap()
}

/// What the four collections hold for `id` of `resource_type` in `tenant`, or for
/// every id of the type when `id` is `None`. Counted through a plain driver
/// client, never subject to a failpoint.
async fn held(db: &mongodb::Database, tenant: &str, resource_type: &str, id: Option<&str>) -> Held {
    let mut by_id = doc! { "tenant_id": tenant, "resource_type": resource_type };
    let mut by_resource_id = by_id.clone();
    if let Some(id) = id {
        by_id.insert("id", id);
        by_resource_id.insert("resource_id", id);
    }
    Held {
        resources: count_in(db, "resources", by_id.clone()).await,
        history: count_in(db, "resource_history", by_id).await,
        rows: count_in(db, "search_index", by_resource_id.clone()).await,
        contained: count_in(db, "search_index_contained", by_resource_id).await,
    }
}

/// The distinct ids `collection` holds for the type, from `field` (`id` for the
/// resource collections, `resource_id` for the rows).
async fn ids_in(
    db: &mongodb::Database,
    collection: &str,
    field: &str,
    tenant: &str,
    resource_type: &str,
) -> BTreeSet<String> {
    db.collection::<Document>(collection)
        .distinct(
            field,
            doc! { "tenant_id": tenant, "resource_type": resource_type },
        )
        .await
        .unwrap()
        .into_iter()
        .filter_map(|id| id.as_str().map(str::to_string))
        .collect()
}

/// An Observation with a contained Patient, so a resource of it has rows in
/// `search_index` and in `search_index_contained`.
fn holder_json(id: &str, family: &str) -> serde_json::Value {
    json!({
        "resourceType": "Observation",
        "id": id,
        "status": "final",
        "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
        "subject": {"reference": "#p"},
        "contained": [{
            "resourceType": "Patient",
            "id": "p",
            "name": [{"family": family}]
        }]
    })
}

/// A Patient with a name, so it has `search_index` rows.
fn patient_json(id: &str, family: &str) -> serde_json::Value {
    json!({
        "resourceType": "Patient",
        "id": id,
        "name": [{"family": family}]
    })
}

/// Stores `holder_json(id)` in two versions: two history rows, a `resources`
/// document, rows and contained rows.
async fn seed_holder(backend: &MongoBackend, tenant: &TenantContext, id: &str) {
    for (family, version) in [("First", "1"), ("Second", "2")] {
        let (stored, _) = backend
            .seed_put(
                tenant,
                "Observation",
                id,
                holder_json(id, family),
                FhirVersion::default(),
            )
            .await
            .expect("seeding a holder");
        assert_eq!(stored.version_id(), version);
    }
}

/// `backend` with a database of its own, and the raw handle on it.
async fn purge_backend(test: &str, app: &str) -> Option<(MongoBackend, mongodb::Database)> {
    purge_backend_with(test, app, |_| {}).await
}

/// [`purge_backend`] with `configure` applied to the config just before the
/// backend is built. The config is the one [`create_backend_with_app_name`]
/// builds: `retryWrites=false` and the harness's settling-read bound.
async fn purge_backend_with(
    test: &str,
    app: &str,
    configure: impl FnOnce(&mut MongoBackendConfig),
) -> Option<(MongoBackend, mongodb::Database)> {
    let connection_string = shared_mongo::connection_string().await?;
    let mut config = MongoBackendConfig {
        connection_string: append_query_param(&connection_string, "retryWrites=false"),
        database_name: build_test_database_name(test),
        app_name: Some(app.to_string()),
        data_dir: Some(repo_data_dir()),
        settle_read_max_time_ms: TEST_SETTLE_READ_MAX_TIME_MS,
        ..Default::default()
    };
    configure(&mut config);
    let backend = build_backend(config).await?;
    let db = purge_raw_db(&backend).await;
    Some((backend, db))
}

/// A bulk-submit change document for `id` of `resource_type`, with the prior body
/// `previous_content` (a body, or `Bson::Null` for a change that was blanked).
async fn plant_change_row(
    db: &mongodb::Database,
    tenant: &str,
    resource_type: &str,
    id: &str,
    previous_content: Bson,
) {
    db.collection::<Document>("bulk_submission_changes")
        .insert_one(doc! {
            "tenant_id": tenant,
            "submitter": "sub",
            "submission_id": "s1",
            "change_id": format!("c-{resource_type}-{id}"),
            "manifest_id": "m1",
            "change_type": "update",
            "resource_type": resource_type,
            "resource_id": id,
            "previous_version": "1",
            "new_version": "2",
            "previous_content": previous_content,
        })
        .await
        .unwrap();
}

/// A change row with a prior body, and a sync failure, for `id` of
/// `resource_type`. A purge of that resource clears both, and nothing else does.
async fn plant_side_rows(
    backend: &MongoBackend,
    db: &mongodb::Database,
    tenant: &str,
    resource_type: &str,
    id: &str,
) {
    plant_change_row(
        db,
        tenant,
        resource_type,
        id,
        Bson::String("{\"resourceType\":\"Patient\",\"name\":[{\"family\":\"Secret\"}]}".into()),
    )
    .await;
    plant_sync_failure(backend, tenant, resource_type, id).await;
}

/// A sync failure of the secondary for `id` of `resource_type`.
async fn plant_sync_failure(backend: &MongoBackend, tenant: &str, resource_type: &str, id: &str) {
    use helios_persistence::composite::{
        SecondarySyncFailureLedger, SyncFailureKey, SyncFailureReport, SyncOperation,
    };
    let report = SyncFailureReport {
        key: SyncFailureKey {
            tenant_id: tenant.to_string(),
            resource_type: resource_type.to_string(),
            resource_id: id.to_string(),
            backend_id: "es".to_string(),
        },
        operation: SyncOperation::Update,
        attempts: 3,
        error: "boom".to_string(),
        failed_at: chrono::Utc::now(),
    };
    assert!(backend.record_sync_failure(&report).await.unwrap());
}

/// What a purge leaves of the rows [`plant_side_rows`] planted: whether the change
/// row still holds a prior body (`None` when the row is gone, which a purge never
/// does), and how many sync failures are left.
async fn side_rows(
    db: &mongodb::Database,
    tenant: &str,
    resource_type: &str,
    id: &str,
) -> (Option<bool>, u64) {
    let change = db
        .collection::<Document>("bulk_submission_changes")
        .find_one(doc! { "tenant_id": tenant, "resource_type": resource_type, "resource_id": id })
        .await
        .unwrap()
        .map(|change| matches!(change.get("previous_content"), Some(Bson::String(_))));
    let failures = count_in(
        db,
        "secondary_sync_failures",
        doc! {
            "_id.tenant_id": tenant,
            "_id.resource_type": resource_type,
            "_id.resource_id": id,
        },
    )
    .await;
    (change, failures)
}

/// Rows planted by [`plant_side_rows`] and left alone.
const SIDE_ROWS_INTACT: (Option<bool>, u64) = (Some(true), 1);
/// Rows planted by [`plant_side_rows`] and cleared by a purge: the change row
/// stays, with its prior body blanked.
const SIDE_ROWS_PURGED: (Option<bool>, u64) = (Some(false), 0);
/// Rows planted by [`plant_side_rows`] after a `purge_all` batch: the prior body is
/// blanked, and the sync failure is left to the sweep that follows the batches.
const SIDE_ROWS_BLANKED: (Option<bool>, u64) = (Some(false), 1);

/// The failpoint's `data` for a `delete` into `collection` of the backend's
/// database that fails with `error_code`, and no other delete.
fn delete_fails_on(backend: &MongoBackend, collection: &str, error_code: i32) -> Document {
    doc! {
        "failCommands": ["delete"],
        "namespace": format!("{}.{collection}", backend.config().database_name),
        "errorCode": error_code,
    }
}

/// [`delete_fails_on`] with the labelled `WriteConflict` of a lost race.
fn delete_conflicts_on(backend: &MongoBackend, collection: &str) -> Document {
    let mut data = delete_fails_on(backend, collection, 112);
    data.insert("errorLabels", vec!["TransientTransactionError"]);
    data
}

/// A scoped subscriber for one test that records the backend's WARN lines into a
/// fresh buffer. See `transaction_retry::capture_bundle_warnings`.
fn capture_purge_warnings() -> (
    &'static std::sync::Mutex<Vec<u8>>,
    tracing::dispatcher::DefaultGuard,
) {
    capture_purge_logs("warn")
}

/// [`capture_purge_warnings`] at `level` (`"debug"` also records the line of each
/// `purge_all` batch).
fn capture_purge_logs(
    level: &str,
) -> (
    &'static std::sync::Mutex<Vec<u8>>,
    tracing::dispatcher::DefaultGuard,
) {
    super::deferred_search_index::keep_two_dispatchers_registered();
    let buf: &'static std::sync::Mutex<Vec<u8>> =
        Box::leak(Box::new(std::sync::Mutex::new(Vec::new())));
    let writer = tracing_test::internal::MockWriter::new(buf);
    let dispatch = tracing_test::internal::get_subscriber(
        writer,
        &format!("helios_persistence::backends::mongodb::storage={level}"),
    );
    let guard = tracing::dispatcher::set_default(&dispatch);
    (buf, guard)
}

/// The number after `name=` in a captured log line.
fn number_field(line: &str, name: &str) -> Option<u64> {
    let (_, rest) = line.split_once(&format!("{name}="))?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The numbers after `name=` in the captured lines that contain `needle`, in the
/// order they were logged.
fn logged_numbers(buf: &std::sync::Mutex<Vec<u8>>, needle: &str, name: &str) -> Vec<u64> {
    captured_lines(buf, needle)
        .iter()
        .map(|line| number_field(line, name).unwrap_or_else(|| panic!("no {name}= in {line}")))
        .collect()
}

/// The captured lines that contain `needle`.
fn captured_lines(buf: &std::sync::Mutex<Vec<u8>>, needle: &str) -> Vec<String> {
    let buf = buf.lock().unwrap();
    String::from_utf8_lossy(&buf)
        .lines()
        .filter(|line| line.contains(needle))
        .map(str::to_string)
        .collect()
}

/// The WARN a purge logs when it runs without a transaction after a 388, two
/// rollbacks under cache pressure, or a history over the version limit.
const PURGING_WITHOUT_A_TRANSACTION: &str = "purging without a transaction";
/// The INFO a purge of one resource (or of a lone id) logs when its transaction
/// was rolled back under cache pressure and it runs the transaction once more.
const RUNNING_IT_ONCE_MORE: &str = "running it once more";

// ---------------------------------------------------------------------------
// The oplog: is it a transaction, or a command of its own?
// ---------------------------------------------------------------------------

/// The newest oplog timestamp, or `None` when the oplog cannot be read (an
/// external standalone server).
async fn oplog_head(client: &Client) -> Option<Timestamp> {
    let head = client
        .database("local")
        .collection::<Document>("oplog.rs")
        .find_one(doc! {})
        .sort(doc! { "$natural": -1_i32 })
        .await
        .ok()??;
    head.get_timestamp("ts").ok()
}

/// How the deletes of one collection of the test's database reached the oplog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Deleted {
    /// Documents deleted by a transaction that committed: its deletes are the
    /// `applyOps` of one `op: "c"` entry that carries a `txnNumber`. A transaction
    /// that was rolled back leaves nothing.
    in_transactions: usize,
    /// Documents deleted by a command of their own: an `op: "d"` entry each, or,
    /// for a `delete_many` of several documents, the `applyOps` of a batched
    /// delete, which has no `txnNumber`.
    sessionless: usize,
}

/// A window on the oplog that opens when it is taken, filtered by the name of the
/// test's own database, so tests running in parallel do not see each other. It is
/// the server's own record of whether a purge ran in a transaction: a test that
/// asserts a fallback "ran without a transaction" from a WARN line alone would pass
/// if the fallback were a second transaction.
struct Oplog {
    client: Client,
    db_name: String,
    since: Timestamp,
}

impl Oplog {
    /// Marks the current end of the oplog. On the harness's own replica set the
    /// oplog must be readable, and a failure to read it fails the test: the oplog
    /// assertions are the proof, and they must not vanish quietly. Against an
    /// external `HFS_TEST_MONGODB_URL`, which may be a standalone server, it
    /// returns `None` (after saying so) and the caller skips its oplog assertions.
    async fn begin(backend: &MongoBackend) -> Option<Oplog> {
        let client = raw_test_client(&backend.config().connection_string)
            .await
            .ok();
        let since = match &client {
            Some(client) => oplog_head(client).await,
            None => None,
        };
        match (client, since) {
            (Some(client), Some(since)) => Some(Oplog {
                client,
                db_name: backend.config().database_name.clone(),
                since,
            }),
            _ => {
                assert!(
                    !transactions_required(),
                    "the harness's own Mongo replica set must expose local.oplog.rs to the \
                     test client; the oplog assertions would otherwise be skipped silently"
                );
                eprintln!("the oplog cannot be read here; skipping the oplog assertions");
                None
            }
        }
    }

    /// The deletes of `collection` since [`Oplog::begin`].
    ///
    /// A committed transaction is an `op: "c"` entry with an `applyOps` array and a
    /// `txnNumber`. A `delete_many` of more than one document without a session
    /// is not one `op: "d"` entry per document either: the server batches such
    /// deletes into an `applyOps` entry of its own, which has no `txnNumber`. So
    /// the `txnNumber` is what tells a transaction from a command of its own.
    async fn deleted(&self, collection: &str) -> Deleted {
        let ns = format!("{}.{collection}", self.db_name);
        let oplog = self
            .client
            .database("local")
            .collection::<Document>("oplog.rs");
        let single: Vec<Document> = oplog
            .find(doc! { "ts": { "$gt": self.since }, "op": "d", "ns": &ns })
            .await
            .expect("read the oplog")
            .try_collect()
            .await
            .expect("read the oplog");
        let batched: Vec<Document> = oplog
            .find(doc! { "ts": { "$gt": self.since }, "op": "c", "o.applyOps.ns": &ns })
            .await
            .expect("read the oplog")
            .try_collect()
            .await
            .expect("read the oplog");
        let deletes_in = |entry: &Document| {
            entry
                .get_document("o")
                .ok()
                .and_then(|o| o.get_array("applyOps").ok())
                .map(|ops| {
                    ops.iter()
                        .filter_map(|op| op.as_document())
                        .filter(|op| {
                            op.get_str("op") == Ok("d") && op.get_str("ns") == Ok(ns.as_str())
                        })
                        .count()
                })
                .unwrap_or(0)
        };
        let (transactions, own_batches): (Vec<&Document>, Vec<&Document>) = batched
            .iter()
            .partition(|entry| entry.contains_key("txnNumber"));
        Deleted {
            in_transactions: transactions.iter().map(|entry| deletes_in(entry)).sum(),
            sessionless: single.len()
                + own_batches
                    .iter()
                    .map(|entry| deletes_in(entry))
                    .sum::<usize>(),
        }
    }

    /// Asserts that the documents of `collection` went in `deleted` (a count of
    /// documents of the transaction, a count of documents deleted by commands of
    /// their own). Does nothing when there is no oplog to read.
    async fn assert_deleted(
        oplog: &Option<Oplog>,
        collection: &str,
        in_transactions: usize,
        sessionless: usize,
        what: &str,
    ) {
        if let Some(oplog) = oplog {
            assert_eq!(
                oplog.deleted(collection).await,
                Deleted {
                    in_transactions,
                    sessionless
                },
                "{what}: how the deletes of {collection} reached the oplog"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// purge: one transaction
// ---------------------------------------------------------------------------

/// The deletes are one transaction: a `delete` that fails for good in the third
/// collection (`search_index`) leaves the resource, both versions of its history
/// and its rows where they were. Before #2146 the `resources` and
/// `resource_history` deletes had already been applied, and the resource was gone
/// from the type but for its rows. The last two writes are covered by
/// `a_purge_that_fails_on_its_last_write_changes_nothing`.
#[tokio::test]
async fn a_purge_that_fails_part_way_changes_nothing() {
    let app = "fp-purge-atomic";
    let Some((backend, db)) = purge_backend("purge_atomic", app).await else {
        return;
    };
    let tenant = create_tenant("purge-atomic");
    seed_holder(&backend, &tenant, "holder").await;
    plant_side_rows(&backend, &db, "purge-atomic", "Observation", "holder").await;
    let before = held(&db, "purge-atomic", "Observation", Some("holder")).await;
    assert_eq!(before.resources, 1);
    assert_eq!(before.history, 2);
    assert!(before.rows > 0, "{before:?}");
    assert!(before.contained > 0, "{before:?}");

    let Some(fail_point) = FailPoint::enable(
        app,
        delete_fails_on(&backend, "search_index", 2),
        doc! { "times": 100 },
    )
    .await
    else {
        return;
    };
    let result = backend.purge(&tenant, "Observation", "holder").await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Internal { message, .. })) = &result else {
        panic!("expected the search index delete's error, got {result:?}");
    };
    assert!(
        message.starts_with("Failed to purge search index"),
        "{message}"
    );
    assert_eq!(
        entered, 1,
        "an error that is not transient is not run again"
    );
    assert_eq!(
        held(&db, "purge-atomic", "Observation", Some("holder")).await,
        before,
        "a purge that failed changed something"
    );
    assert_eq!(
        side_rows(&db, "purge-atomic", "Observation", "holder").await,
        SIDE_ROWS_INTACT
    );

    backend
        .purge(&tenant, "Observation", "holder")
        .await
        .expect("with the failpoint off the purge succeeds");
    assert!(
        held(&db, "purge-atomic", "Observation", Some("holder"))
            .await
            .is_empty()
    );
    assert_eq!(
        side_rows(&db, "purge-atomic", "Observation", "holder").await,
        SIDE_ROWS_PURGED
    );
}

/// The sixth and last write of a purge, the delete of the resource's sync
/// failures, is in the transaction too: when it fails for good, the five writes
/// before it are rolled back, the blanking of the prior body included, and the
/// resource, its history, its rows, its prior body and its sync failure are all
/// where they were. A purge whose blanking ran outside the transaction would have
/// blanked the body. A failure of the sixth write cannot show that the sixth write
/// itself is in the transaction (the purge fails before it commits either way), so
/// a second phase fails the commit for good instead: the sync failure is still
/// there, which a delete outside the transaction would already have removed.
#[tokio::test]
async fn a_purge_that_fails_on_its_last_write_changes_nothing() {
    let app = "fp-purge-sixth";
    let Some((backend, db)) = purge_backend("purge_sixth", app).await else {
        return;
    };
    let tenant = create_tenant("purge-sixth");
    seed_holder(&backend, &tenant, "holder").await;
    plant_side_rows(&backend, &db, "purge-sixth", "Observation", "holder").await;
    let before = held(&db, "purge-sixth", "Observation", Some("holder")).await;
    assert!(!before.is_empty());
    assert_eq!(
        side_rows(&db, "purge-sixth", "Observation", "holder").await,
        SIDE_ROWS_INTACT
    );

    let Some(fail_point) = FailPoint::enable(
        app,
        delete_fails_on(&backend, "secondary_sync_failures", 2),
        doc! { "times": 100 },
    )
    .await
    else {
        return;
    };
    let result = backend.purge(&tenant, "Observation", "holder").await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Internal { message, .. })) = &result else {
        panic!("expected the sync failure delete's error, got {result:?}");
    };
    assert!(
        message.starts_with("Failed to purge sync failures"),
        "{message}"
    );
    assert_eq!(
        entered, 1,
        "an error that is not transient is not run again"
    );
    assert_eq!(
        held(&db, "purge-sixth", "Observation", Some("holder")).await,
        before,
        "the four deletes were rolled back"
    );
    assert_eq!(
        side_rows(&db, "purge-sixth", "Observation", "holder").await,
        SIDE_ROWS_INTACT,
        "the blanking of the prior body was rolled back, and the failure is still there"
    );

    // And the sixth write itself is in the transaction: a commit that fails for
    // good leaves the sync failure too, which a delete outside the transaction
    // would already have removed. (The failure of the sixth write above cannot
    // tell: the purge fails before it commits either way.)
    let Some(fail_point) = FailPoint::enable(app, commit_fails(), doc! { "times": 100 }).await
    else {
        return;
    };
    let result = backend.purge(&tenant, "Observation", "holder").await;
    let entered = fail_point.off_and_count().await;
    assert!(
        result.is_err(),
        "a commit that fails for good fails the purge: {result:?}"
    );
    assert!(entered >= 1, "the purge sent a commit");
    assert_eq!(
        held(&db, "purge-sixth", "Observation", Some("holder")).await,
        before,
        "nothing was committed"
    );
    assert_eq!(
        side_rows(&db, "purge-sixth", "Observation", "holder").await,
        SIDE_ROWS_INTACT,
        "the sync failure delete and the blanking are in the transaction that was not committed"
    );

    backend
        .purge(&tenant, "Observation", "holder")
        .await
        .expect("with the failpoint off the purge succeeds");
    assert!(
        held(&db, "purge-sixth", "Observation", Some("holder"))
            .await
            .is_empty()
    );
    assert_eq!(
        side_rows(&db, "purge-sixth", "Observation", "holder").await,
        SIDE_ROWS_PURGED
    );
}

/// A purge that stopped after its `resources` and `resource_history` deletes
/// (planted through the raw database) leaves rows, a prior body and a sync
/// failure for an id that is gone. A repeated purge finishes it; it used to
/// answer `NotFound` and never reach them (#2146).
#[tokio::test]
async fn a_repeated_purge_finishes_a_half_done_one() {
    let Some((backend, db)) = purge_backend("purge_half_done", "fp-purge-half-done").await else {
        return;
    };
    let tenant = create_tenant("purge-half-done");
    seed_holder(&backend, &tenant, "holder").await;
    plant_side_rows(&backend, &db, "purge-half-done", "Observation", "holder").await;
    let key = doc! {
        "tenant_id": "purge-half-done",
        "resource_type": "Observation",
        "id": "holder",
    };
    db.collection::<Document>("resources")
        .delete_many(key.clone())
        .await
        .unwrap();
    db.collection::<Document>("resource_history")
        .delete_many(key)
        .await
        .unwrap();
    let half_done = held(&db, "purge-half-done", "Observation", Some("holder")).await;
    assert_eq!((half_done.resources, half_done.history), (0, 0));
    assert!(
        half_done.rows > 0 && half_done.contained > 0,
        "{half_done:?}"
    );

    backend
        .purge(&tenant, "Observation", "holder")
        .await
        .expect("a repeated purge finishes the job");

    assert!(
        held(&db, "purge-half-done", "Observation", Some("holder"))
            .await
            .is_empty(),
        "the rows of the half-done purge are gone"
    );
    assert_eq!(
        side_rows(&db, "purge-half-done", "Observation", "holder").await,
        SIDE_ROWS_PURGED
    );
    let again = backend.purge(&tenant, "Observation", "holder").await;
    assert!(
        matches!(
            again,
            Err(StorageError::Resource(ResourceError::NotFound { .. }))
        ),
        "{again:?}"
    );
}

/// A resource whose `resources` document is gone but whose history is not (the
/// state a failed `resources`-then-history purge left) is purged, and the same
/// id can then be created again: stale history rows made the create fail on the
/// unique `idx_history_identity` (a 500, #2146).
#[tokio::test]
async fn a_history_only_leftover_is_purged_and_its_id_can_be_created_again() {
    let Some((backend, db)) = purge_backend("purge_history_only", "fp-purge-history-only").await
    else {
        return;
    };
    let tenant = create_tenant("purge-history-only");
    seed_holder(&backend, &tenant, "holder").await;
    db.collection::<Document>("resources")
        .delete_many(doc! {
            "tenant_id": "purge-history-only",
            "resource_type": "Observation",
            "id": "holder",
        })
        .await
        .unwrap();
    let leftover = held(&db, "purge-history-only", "Observation", Some("holder")).await;
    assert_eq!((leftover.resources, leftover.history), (0, 2));

    backend
        .purge(&tenant, "Observation", "holder")
        .await
        .expect("a history-only leftover is purgeable");
    assert!(
        held(&db, "purge-history-only", "Observation", Some("holder"))
            .await
            .is_empty()
    );

    let created = backend
        .create(
            &tenant,
            "Observation",
            holder_json("holder", "Again"),
            FhirVersion::default(),
        )
        .await
        .expect("the id is free again");
    assert_eq!(created.version_id(), "1");
    let again = held(&db, "purge-history-only", "Observation", Some("holder")).await;
    assert_eq!((again.resources, again.history), (1, 1));
}

/// A key of which nothing is left is still `NotFound`, because its six writes
/// change nothing: one that never existed, and one that has just been purged.
#[tokio::test]
async fn a_key_with_no_trace_is_not_found() {
    let Some((backend, db)) = purge_backend("purge_no_trace", "fp-purge-no-trace").await else {
        return;
    };
    let tenant = create_tenant("purge-no-trace");

    let never = backend.purge(&tenant, "Observation", "never").await;
    assert!(
        matches!(
            never,
            Err(StorageError::Resource(ResourceError::NotFound { .. }))
        ),
        "{never:?}"
    );

    seed_holder(&backend, &tenant, "holder").await;
    backend
        .purge(&tenant, "Observation", "holder")
        .await
        .expect("the purge");
    let again = backend.purge(&tenant, "Observation", "holder").await;
    assert!(
        matches!(
            again,
            Err(StorageError::Resource(ResourceError::NotFound { .. }))
        ),
        "{again:?}"
    );
    assert!(
        held(&db, "purge-no-trace", "Observation", None)
            .await
            .is_empty()
    );
}

/// A resource that only has rows in `search_index_contained` is purged, and so is
/// one that only has rows in `search_index`: the purge changed something, so it is
/// not `NotFound`.
#[tokio::test]
async fn either_row_collection_alone_is_purged() {
    let Some((backend, db)) = purge_backend("purge_row_trace", "fp-purge-row-trace").await else {
        return;
    };
    let tenant = create_tenant("purge-row-trace");
    for (collection, document) in [
        (
            "search_index",
            doc! {
                "tenant_id": "purge-row-trace",
                "resource_type": "Observation",
                "resource_id": "own-rows",
                "param_name": "status",
                "value_token_code": "final",
            },
        ),
        (
            "search_index_contained",
            doc! {
                "tenant_id": "purge-row-trace",
                "resource_type": "Observation",
                "resource_id": "contained-rows",
                "contained_type": "Patient",
                "param_name": "family",
                "value_string": "Alone",
            },
        ),
    ] {
        db.collection::<Document>(collection)
            .insert_one(document)
            .await
            .unwrap();
    }

    for id in ["own-rows", "contained-rows"] {
        backend
            .purge(&tenant, "Observation", id)
            .await
            .unwrap_or_else(|e| panic!("{id}: rows alone are purged: {e:?}"));
    }
    assert!(
        held(&db, "purge-row-trace", "Observation", None)
            .await
            .is_empty()
    );
}

/// The purge of a resource that only has a prior body left in the bulk-submit
/// change log (PHI, #2279) answers `Ok` and blanks it. `NotFound` used to be
/// decided before the blanking, from four collections that held nothing. The next
/// purge, with the body blanked, changes nothing and answers `NotFound`, and so
/// does one for a change row that was blank already.
#[tokio::test]
async fn a_leftover_with_only_a_prior_body_is_purged_and_blanked() {
    let Some((backend, db)) = purge_backend("purge_body_only", "fp-purge-body-only").await else {
        return;
    };
    let tenant = create_tenant("purge-body-only");
    plant_change_row(
        &db,
        "purge-body-only",
        "Observation",
        "body",
        Bson::String("{\"resourceType\":\"Observation\",\"note\":\"Secret\"}".into()),
    )
    .await;
    plant_change_row(&db, "purge-body-only", "Observation", "blank", Bson::Null).await;
    assert!(
        held(&db, "purge-body-only", "Observation", None)
            .await
            .is_empty(),
        "nothing but the change rows is left"
    );
    assert_eq!(
        side_rows(&db, "purge-body-only", "Observation", "body").await,
        (Some(true), 0)
    );

    backend
        .purge(&tenant, "Observation", "body")
        .await
        .expect("a prior body alone is purged");
    assert_eq!(
        side_rows(&db, "purge-body-only", "Observation", "body").await,
        (Some(false), 0),
        "the body is blanked and the change row stays"
    );
    for id in ["body", "blank"] {
        let again = backend.purge(&tenant, "Observation", id).await;
        assert!(
            matches!(
                again,
                Err(StorageError::Resource(ResourceError::NotFound { .. }))
            ),
            "{id}: {again:?}"
        );
    }
}

/// The same for a resource of which only a sync failure is left: it is removed
/// and the purge answers `Ok`, then `NotFound`.
#[tokio::test]
async fn a_leftover_with_only_a_sync_failure_is_purged() {
    let Some((backend, db)) = purge_backend("purge_failure_only", "fp-purge-failure-only").await
    else {
        return;
    };
    let tenant = create_tenant("purge-failure-only");
    plant_sync_failure(&backend, "purge-failure-only", "Observation", "failed").await;
    assert!(
        held(&db, "purge-failure-only", "Observation", None)
            .await
            .is_empty()
    );
    assert_eq!(
        side_rows(&db, "purge-failure-only", "Observation", "failed").await,
        (None, 1)
    );

    backend
        .purge(&tenant, "Observation", "failed")
        .await
        .expect("a sync failure alone is purged");
    assert_eq!(
        side_rows(&db, "purge-failure-only", "Observation", "failed").await,
        (None, 0)
    );
    let again = backend.purge(&tenant, "Observation", "failed").await;
    assert!(
        matches!(
            again,
            Err(StorageError::Resource(ResourceError::NotFound { .. }))
        ),
        "{again:?}"
    );
}

/// `purge` and `purge_all` touch only their own tenant and their own type: the
/// same id in a second tenant, and another type with the same id in the first
/// tenant, keep all four collections, their prior bodies and their sync
/// failures.
#[tokio::test]
async fn a_purge_leaves_other_tenants_and_other_types_alone() {
    let Some((backend, db)) = purge_backend("purge_isolation", "fp-purge-isolation").await else {
        return;
    };
    let tenant_a = create_tenant("purge-iso-a");
    let tenant_b = create_tenant("purge-iso-b");
    seed_holder(&backend, &tenant_a, "shared").await;
    seed_holder(&backend, &tenant_b, "shared").await;
    backend
        .seed_put(
            &tenant_a,
            "Patient",
            "shared",
            patient_json("shared", "Bystander"),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    for (tenant, resource_type) in [
        ("purge-iso-a", "Observation"),
        ("purge-iso-b", "Observation"),
        ("purge-iso-a", "Patient"),
    ] {
        plant_side_rows(&backend, &db, tenant, resource_type, "shared").await;
    }
    let other_tenant = held(&db, "purge-iso-b", "Observation", Some("shared")).await;
    let other_type = held(&db, "purge-iso-a", "Patient", Some("shared")).await;
    assert!(other_tenant.rows > 0 && other_type.rows > 0);

    let assert_bystanders_untouched = |when: &'static str| {
        let db = db.clone();
        async move {
            assert_eq!(
                held(&db, "purge-iso-b", "Observation", Some("shared")).await,
                other_tenant,
                "{when}: another tenant's resource changed"
            );
            assert_eq!(
                held(&db, "purge-iso-a", "Patient", Some("shared")).await,
                other_type,
                "{when}: another type's resource changed"
            );
            for (tenant, resource_type) in
                [("purge-iso-b", "Observation"), ("purge-iso-a", "Patient")]
            {
                assert_eq!(
                    side_rows(&db, tenant, resource_type, "shared").await,
                    SIDE_ROWS_INTACT,
                    "{when}: the prior body or the sync failure of {tenant}/{resource_type}"
                );
            }
        }
    };

    backend
        .purge(&tenant_a, "Observation", "shared")
        .await
        .expect("purge");
    assert!(
        held(&db, "purge-iso-a", "Observation", Some("shared"))
            .await
            .is_empty()
    );
    assert_eq!(
        side_rows(&db, "purge-iso-a", "Observation", "shared").await,
        SIDE_ROWS_PURGED
    );
    assert_bystanders_untouched("after purge").await;

    seed_holder(&backend, &tenant_a, "shared").await;
    seed_holder(&backend, &tenant_a, "second").await;
    let purged = backend
        .purge_all(&tenant_a, "Observation")
        .await
        .expect("purge_all");
    assert_eq!(purged, 2);
    assert!(
        held(&db, "purge-iso-a", "Observation", None)
            .await
            .is_empty()
    );
    assert_bystanders_untouched("after purge_all").await;
}

/// A resource with a very long history can outgrow what one transaction may
/// write (`TransactionTooLargeForCache`, 388). Such a purge succeeded before
/// #2146, so it still does: the transaction is aborted and the purge runs
/// without one, and says so once. The oplog shows the
/// fallback is sessionless: its deletes are commands of their own, where a second
/// transaction would have left them as the `applyOps` of one commit.
#[tokio::test]
async fn a_purge_too_large_for_a_transaction_runs_without_one() {
    let app = "fp-purge-388";
    let Some((backend, db)) = purge_backend("purge_388", app).await else {
        return;
    };
    let tenant = create_tenant("purge-388");
    seed_holder(&backend, &tenant, "holder").await;
    let oplog = Oplog::begin(&backend).await;
    let Some(fail_point) = FailPoint::enable(
        app,
        delete_fails_on(&backend, "resources", 388),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let (logs, _guard) = capture_purge_warnings();
    let result = backend.purge(&tenant, "Observation", "holder").await;
    let entered = fail_point.off_and_count().await;

    result.expect("a purge too large for a transaction still succeeds");
    assert_eq!(entered, 1, "the transaction's delete met it, once");
    assert!(
        held(&db, "purge-388", "Observation", Some("holder"))
            .await
            .is_empty()
    );
    let warns = captured_lines(logs, PURGING_WITHOUT_A_TRANSACTION);
    assert_eq!(warns.len(), 1, "one WARN: {warns:?}");
    for field in ["purge-388", "Observation", "holder"] {
        assert!(warns[0].contains(field), "missing {field}: {}", warns[0]);
    }
    Oplog::assert_deleted(&oplog, "resources", 0, 1, "the 388 fallback").await;
    Oplog::assert_deleted(&oplog, "resource_history", 0, 2, "the 388 fallback").await;
}

/// A 388 that the server labels `TransientTransactionError` is still a 388: the
/// purge recognises it before the replay looks at labels, so it runs without a
/// transaction at once, instead of replaying the same writes into the same cache
/// four times and answering `503`. The WARN of the fallback is there, and none of
/// the replay's, and the oplog shows commands of their own, not a transaction.
#[tokio::test]
async fn a_388_that_carries_a_transient_label_is_run_without_a_transaction() {
    let app = "fp-purge-388-labelled";
    let Some((backend, db)) = purge_backend("purge_388_labelled", app).await else {
        return;
    };
    let tenant = create_tenant("purge-388-labelled");
    seed_holder(&backend, &tenant, "holder").await;
    let mut data = delete_fails_on(&backend, "resources", 388);
    data.insert("errorLabels", vec!["TransientTransactionError"]);
    let oplog = Oplog::begin(&backend).await;
    let Some(fail_point) = FailPoint::enable(app, data, doc! { "times": 1 }).await else {
        return;
    };

    let (logs, _guard) = capture_purge_warnings();
    let result = backend.purge(&tenant, "Observation", "holder").await;
    let entered = fail_point.off_and_count().await;

    result.expect("a labelled 388 still ends in the purge without a transaction");
    assert_eq!(entered, 1);
    assert!(
        held(&db, "purge-388-labelled", "Observation", Some("holder"))
            .await
            .is_empty()
    );
    assert_eq!(
        captured_lines(logs, PURGING_WITHOUT_A_TRANSACTION).len(),
        1,
        "the fallback's WARN"
    );
    assert!(
        captured_lines(logs, "running it again").is_empty(),
        "the replay never ran: {:?}",
        captured_lines(logs, "running it again")
    );
    Oplog::assert_deleted(&oplog, "resources", 0, 1, "the labelled 388 fallback").await;
    Oplog::assert_deleted(
        &oplog,
        "resource_history",
        0,
        2,
        "the labelled 388 fallback",
    )
    .await;
}

/// A purge whose transaction the database rolls back under cache pressure (365)
/// is not answered `503`: it gets one more transactional try after 100 ms, and
/// when that is rolled back too it runs without a transaction, with the one WARN.
/// The failpoint fires on the first two `search_index` deletes, the third write of
/// the transaction, so both transactions are rolled back (`entered == 2` is the
/// second try) and the fallback's deletes meet it spent. The oplog shows the
/// fallback's deletes as commands of their own: a third transaction would have
/// committed them as one `applyOps`. The first rollback is logged at INFO, once.
#[tokio::test]
async fn a_purge_rolled_back_under_cache_pressure_twice_runs_without_a_transaction() {
    let app = "fp-purge-pressure-twice";
    let Some((backend, db)) = purge_backend("purge_pressure_twice", app).await else {
        return;
    };
    let tenant = create_tenant("purge-pressure-twice");
    seed_holder(&backend, &tenant, "holder").await;
    plant_side_rows(
        &backend,
        &db,
        "purge-pressure-twice",
        "Observation",
        "holder",
    )
    .await;
    let oplog = Oplog::begin(&backend).await;
    let Some(fail_point) = FailPoint::enable(
        app,
        delete_fails_on(&backend, "search_index", 365),
        doc! { "times": 2 },
    )
    .await
    else {
        return;
    };

    let (logs, _guard) = capture_purge_logs("info");
    let result = backend.purge(&tenant, "Observation", "holder").await;
    let entered = fail_point.off_and_count().await;

    result.expect("a purge never answers 503 for cache pressure");
    assert_eq!(
        entered, 2,
        "the first transaction and its second try were both rolled back"
    );
    assert!(
        held(&db, "purge-pressure-twice", "Observation", Some("holder"))
            .await
            .is_empty()
    );
    assert_eq!(
        side_rows(&db, "purge-pressure-twice", "Observation", "holder").await,
        SIDE_ROWS_PURGED
    );
    let once_more = captured_lines(logs, RUNNING_IT_ONCE_MORE);
    assert_eq!(once_more.len(), 1, "one second try: {once_more:?}");
    let warns = captured_lines(logs, PURGING_WITHOUT_A_TRANSACTION);
    assert_eq!(warns.len(), 1, "one WARN: {warns:?}");
    for field in [
        "purge-pressure-twice",
        "Observation",
        "holder",
        "cache pressure",
    ] {
        assert!(warns[0].contains(field), "missing {field}: {}", warns[0]);
    }
    assert!(
        captured_lines(logs, "running it again").is_empty(),
        "the replay did not run it again: it hands a cache-pressure rollback back at once"
    );
    Oplog::assert_deleted(&oplog, "resources", 0, 1, "the fallback").await;
    Oplog::assert_deleted(&oplog, "resource_history", 0, 2, "the fallback").await;
}

/// When the second transactional try commits, that is the answer: `Ok`, no WARN,
/// and the oplog shows the deletes as one committed transaction, none of them as
/// a command of its own.
#[tokio::test]
async fn a_purge_rolled_back_once_under_cache_pressure_commits_on_its_second_try() {
    let app = "fp-purge-pressure-once";
    let Some((backend, db)) = purge_backend("purge_pressure_once", app).await else {
        return;
    };
    let tenant = create_tenant("purge-pressure-once");
    seed_holder(&backend, &tenant, "holder").await;
    let oplog = Oplog::begin(&backend).await;
    let Some(fail_point) = FailPoint::enable(
        app,
        delete_fails_on(&backend, "search_index", 365),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let (logs, _guard) = capture_purge_logs("info");
    let result = backend.purge(&tenant, "Observation", "holder").await;
    let entered = fail_point.off_and_count().await;

    result.expect("the second try purges");
    assert_eq!(entered, 1);
    assert!(
        held(&db, "purge-pressure-once", "Observation", Some("holder"))
            .await
            .is_empty()
    );
    assert_eq!(
        captured_lines(logs, RUNNING_IT_ONCE_MORE).len(),
        1,
        "the second try is logged once"
    );
    assert!(
        captured_lines(logs, PURGING_WITHOUT_A_TRANSACTION).is_empty(),
        "no fallback: {:?}",
        captured_lines(logs, PURGING_WITHOUT_A_TRANSACTION)
    );
    Oplog::assert_deleted(&oplog, "resources", 1, 0, "the second try").await;
    Oplog::assert_deleted(&oplog, "resource_history", 2, 0, "the second try").await;
}

/// The failpoint's `data` for a `commitTransaction` that fails for good. A purge
/// that sends one fails, so a test that expects a purge without a transaction to
/// succeed while it is on shows that no commit was sent.
fn commit_fails() -> Document {
    doc! { "failCommands": ["commitTransaction"], "errorCode": 2 }
}

/// A resource with more versions than `purge_transaction_max_versions` is purged
/// without a transaction, and says so once, with its version count. One with
/// exactly the limit still uses one. Without the guard a long history would be a
/// transaction that the request timeout drops, and the resource could never be
/// purged.
#[tokio::test]
async fn a_resource_with_more_versions_than_the_limit_is_purged_without_a_transaction() {
    let app = "fp-purge-versions";
    let Some((backend, db)) = purge_backend_with("purge_versions", app, |config| {
        config.purge_transaction_max_versions = 3;
    })
    .await
    else {
        return;
    };
    let tenant = create_tenant("purge-versions");
    for (id, versions) in [("long", 5), ("exact", 3)] {
        for version in 1..=versions {
            backend
                .seed_put(
                    &tenant,
                    "Patient",
                    id,
                    patient_json(id, &format!("Version{version}")),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
    }
    plant_side_rows(&backend, &db, "purge-versions", "Patient", "long").await;
    assert_eq!(
        held(&db, "purge-versions", "Patient", Some("long"))
            .await
            .history,
        5
    );

    // A purge that sent a commit would fail: this one is sessionless.
    let Some(fail_point) = FailPoint::enable(app, commit_fails(), doc! { "times": 100 }).await
    else {
        return;
    };
    let (logs, _guard) = capture_purge_warnings();
    let result = backend.purge(&tenant, "Patient", "long").await;
    let entered = fail_point.off_and_count().await;

    result.expect("a long history is purged without a transaction");
    assert_eq!(entered, 0, "no commit was sent");
    assert!(
        held(&db, "purge-versions", "Patient", Some("long"))
            .await
            .is_empty()
    );
    assert_eq!(
        side_rows(&db, "purge-versions", "Patient", "long").await,
        SIDE_ROWS_PURGED
    );
    let warns = captured_lines(logs, PURGING_WITHOUT_A_TRANSACTION);
    assert_eq!(warns.len(), 1, "one WARN: {warns:?}");
    for text in [
        "purge-versions",
        "Patient",
        "long",
        "versions=5",
        "purge_transaction_max_versions (3)",
    ] {
        assert!(warns[0].contains(text), "missing {text}: {}", warns[0]);
    }

    // Exactly the limit is not over it.
    backend
        .purge(&tenant, "Patient", "exact")
        .await
        .expect("a history of exactly the limit is purged in a transaction");
    assert!(
        held(&db, "purge-versions", "Patient", Some("exact"))
            .await
            .is_empty()
    );
    assert_eq!(
        captured_lines(logs, PURGING_WITHOUT_A_TRANSACTION).len(),
        1,
        "no second WARN"
    );
}

/// A `WriteConflict` on one of the purge's commands means a writer touched one of
/// its documents. A purge means the same after that writer commits, so it is run
/// again, as any single-resource write the server aborted is.
#[tokio::test]
async fn a_purge_aborted_by_a_write_conflict_is_run_again() {
    let app = "fp-purge-conflict";
    let Some((backend, db)) = purge_backend("purge_conflict", app).await else {
        return;
    };
    let tenant = create_tenant("purge-conflict");
    seed_holder(&backend, &tenant, "holder").await;
    let Some(fail_point) = FailPoint::enable(
        app,
        delete_conflicts_on(&backend, "resource_history"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend.purge(&tenant, "Observation", "holder").await;
    let entered = fail_point.off_and_count().await;

    result.expect("the second attempt purges");
    assert_eq!(entered, 1);
    assert!(
        held(&db, "purge-conflict", "Observation", Some("holder"))
            .await
            .is_empty()
    );
}

/// The purge's first read, the count of its versions, fails once with a state
/// change (11602, which the server labels a transient abort inside a transaction):
/// the purge is run again and succeeds. `read_errors.rs` covers the read that keeps
/// failing.
#[tokio::test]
async fn a_single_failed_version_count_is_run_again() {
    let app = "fp-purge-count-once";
    let Some((backend, db)) = purge_backend("purge_count_once", app).await else {
        return;
    };
    let tenant = create_tenant("purge-count-once");
    seed_holder(&backend, &tenant, "holder").await;
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! {
            "failCommands": ["aggregate"],
            "namespace": format!("{}.resource_history", backend.config().database_name),
            "errorCode": 11602,
        },
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend.purge(&tenant, "Observation", "holder").await;
    let entered = fail_point.off_and_count().await;

    result.expect("the second attempt purges");
    assert_eq!(entered, 1);
    assert!(
        held(&db, "purge-count-once", "Observation", Some("holder"))
            .await
            .is_empty()
    );
}

/// A purge that the server keeps aborting gives up after the attempts every
/// single-resource write makes, answers `Unavailable` (a retryable 503) and
/// changes nothing; once the writer is gone a repeat succeeds.
#[tokio::test]
async fn a_purge_that_keeps_being_aborted_is_unavailable_and_changes_nothing() {
    let app = "fp-purge-conflict-exhausted";
    let Some((backend, db)) = purge_backend("purge_conflict_exhausted", app).await else {
        return;
    };
    let tenant = create_tenant("purge-conflict-exhausted");
    seed_holder(&backend, &tenant, "holder").await;
    let before = held(
        &db,
        "purge-conflict-exhausted",
        "Observation",
        Some("holder"),
    )
    .await;
    let Some(fail_point) = FailPoint::enable(
        app,
        delete_conflicts_on(&backend, "search_index"),
        doc! { "times": 100 },
    )
    .await
    else {
        return;
    };

    let result = backend.purge(&tenant, "Observation", "holder").await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = &result else {
        panic!("expected Unavailable, got {result:?}");
    };
    assert_eq!(
        message,
        "The purge was aborted by the database 4 times; retry the request"
    );
    assert_eq!(entered, WRITE_ABORT_ATTEMPTS);
    assert_eq!(
        held(
            &db,
            "purge-conflict-exhausted",
            "Observation",
            Some("holder")
        )
        .await,
        before
    );

    backend
        .purge(&tenant, "Observation", "holder")
        .await
        .expect("the repeat succeeds");
    assert!(
        held(
            &db,
            "purge-conflict-exhausted",
            "Observation",
            Some("holder")
        )
        .await
        .is_empty()
    );
}

/// The failpoint's `data` for a `commitTransaction` that applies and then reports
/// a write-concern failure, which the driver labels `UnknownTransactionCommitResult`.
fn commit_with_unknown_result() -> Document {
    doc! {
        "failCommands": ["commitTransaction"],
        "writeConcernError": {
            "code": 64,
            "codeName": "WriteConcernFailed",
            "errmsg": "waiting for replication timed out",
            "errInfo": { "wtimeout": true },
        },
    }
}

/// A purge whose commit applied but whose outcome stayed unknown is settled by a
/// read: nothing of the resource is left, so it answers as committed.
#[tokio::test]
async fn a_purge_whose_commit_applied_but_stayed_unknown_is_settled_as_committed() {
    let app = "fp-purge-commit-applied";
    let Some((backend, db)) = purge_backend("purge_commit_applied", app).await else {
        return;
    };
    let tenant = create_tenant("purge-commit-applied");
    seed_holder(&backend, &tenant, "holder").await;
    let Some(fail_point) =
        FailPoint::enable(app, commit_with_unknown_result(), doc! { "times": 3 }).await
    else {
        return;
    };

    let result = backend.purge(&tenant, "Observation", "holder").await;
    let entered = fail_point.off_and_count().await;

    result.expect("the settling read sees the commit's effect");
    assert_eq!(entered, 3, "the first commit and the two re-sends");
    assert!(
        held(&db, "purge-commit-applied", "Observation", Some("holder"))
            .await
            .is_empty()
    );
}

/// The failpoint's `data` for a `commitTransaction` the server refuses without
/// running it, labelled `UnknownTransactionCommitResult` (the driver's own label
/// for code 50). Nothing is applied. The refused commit leaves its server-side
/// transaction open until `transactionLifetimeLimitSeconds` or the next
/// transaction on that pooled session, so a test that uses this writes nothing to
/// the same documents afterwards.
fn commit_refused_with_unknown_result() -> Document {
    doc! {
        "failCommands": ["commitTransaction"],
        "errorCode": 50,
        "errorLabels": ["UnknownTransactionCommitResult"],
    }
}

/// A commit that never applied and stays unknown is not turned into a success:
/// the read finds the resource, so the answer is `CommitOutcomeUnknown`.
#[tokio::test]
async fn a_purge_whose_commit_never_applied_stays_unknown() {
    let app = "fp-purge-commit-refused";
    let Some((backend, db)) = purge_backend("purge_commit_refused", app).await else {
        return;
    };
    let tenant = create_tenant("purge-commit-refused");
    seed_holder(&backend, &tenant, "holder").await;
    let before = held(&db, "purge-commit-refused", "Observation", Some("holder")).await;
    let Some(fail_point) = FailPoint::enable(
        app,
        commit_refused_with_unknown_result(),
        doc! { "times": 3 },
    )
    .await
    else {
        return;
    };

    let result = backend.purge(&tenant, "Observation", "holder").await;
    let entered = fail_point.off_and_count().await;

    assert!(
        matches!(
            result,
            Err(StorageError::Transaction(
                TransactionError::CommitOutcomeUnknown { .. }
            ))
        ),
        "{result:?}"
    );
    assert_eq!(entered, 3);
    assert_eq!(
        held(&db, "purge-commit-refused", "Observation", Some("holder")).await,
        before,
        "the commit never applied"
    );
}

/// The settling read looks at the bulk-submit change rows too: a resource of which
/// only a change row with a prior body is left is purged by blanking it, and when
/// that commit never applies and stays unknown the read finds the body still
/// there, so the answer is `CommitOutcomeUnknown`. A read that looked only at the
/// four collections would find nothing, answer as committed, and report a prior
/// body erased that was not (#2279).
#[tokio::test]
async fn a_purge_of_only_a_prior_body_whose_commit_never_applied_stays_unknown() {
    let app = "fp-purge-commit-refused-body";
    let Some((backend, db)) = purge_backend("purge_commit_refused_body", app).await else {
        return;
    };
    let tenant = create_tenant("purge-commit-refused-body");
    plant_change_row(
        &db,
        "purge-commit-refused-body",
        "Patient",
        "ghost",
        Bson::String("{\"resourceType\":\"Patient\",\"name\":[{\"family\":\"Secret\"}]}".into()),
    )
    .await;
    assert_eq!(
        held(&db, "purge-commit-refused-body", "Patient", Some("ghost")).await,
        Held {
            resources: 0,
            history: 0,
            rows: 0,
            contained: 0
        },
        "nothing of the resource is left but the change row"
    );
    let Some(fail_point) = FailPoint::enable(
        app,
        commit_refused_with_unknown_result(),
        doc! { "times": 3 },
    )
    .await
    else {
        return;
    };

    let result = backend.purge(&tenant, "Patient", "ghost").await;
    let entered = fail_point.off_and_count().await;

    assert!(
        matches!(
            result,
            Err(StorageError::Transaction(
                TransactionError::CommitOutcomeUnknown { .. }
            ))
        ),
        "the prior body is still there, so the outcome stays unknown: {result:?}"
    );
    assert_eq!(entered, 3, "the first commit and the two re-sends");
    assert_eq!(
        side_rows(&db, "purge-commit-refused-body", "Patient", "ghost").await,
        (Some(true), 0),
        "the commit never applied: the body is not blanked"
    );
}

// ---------------------------------------------------------------------------
// purge: the search rows once more, after the commit
// ---------------------------------------------------------------------------

/// The failpoint's `data` for a `command` on `collection` of the backend's
/// database that is held for `block_ms` and then runs as usual.
fn blocks_for(backend: &MongoBackend, command: &str, collection: &str, block_ms: i64) -> Document {
    doc! {
        "failCommands": [command],
        "namespace": format!("{}.{collection}", backend.config().database_name),
        "blockConnection": true,
        "blockTimeMS": block_ms,
    }
}

/// A transaction reads one snapshot, so its own delete of the search rows cannot
/// take a row written after it ran, as a `$reindex` page does without a
/// transaction. The sweep after the commit takes it. The fourth write of the
/// purge (the delete of the contained rows, after that of the rows) is held for a
/// second; a copy of the resource's search row, put in meanwhile through a plain
/// client, is what the transaction's delete of `search_index` has already missed.
/// Without the sweep the copy would be left for ever.
#[tokio::test]
async fn the_sweep_after_a_purge_removes_a_row_written_after_the_transaction_deleted_its_rows() {
    let app = "fp-purge-sweep-removes";
    let Some((backend, db)) = purge_backend("purge_sweep_removes", app).await else {
        return;
    };
    let tenant = create_tenant("purge-sweep-removes");
    seed_holder(&backend, &tenant, "holder").await;
    let rows = db.collection::<Document>("search_index");
    let mut stray = rows
        .find_one(doc! {
            "tenant_id": "purge-sweep-removes",
            "resource_type": "Observation",
            "resource_id": "holder",
        })
        .await
        .unwrap()
        .expect("the holder has a search row");
    stray.remove("_id");
    let Some(fail_point) = FailPoint::enable(
        app,
        blocks_for(&backend, "delete", "search_index_contained", 1_000),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let (logs, _guard) = capture_purge_logs("debug");
    let write_a_row = async {
        fail_point.wait_until_entered(1).await;
        rows.insert_one(stray.clone()).await.unwrap();
    };
    let (result, ()) = tokio::join!(backend.purge(&tenant, "Observation", "holder"), write_a_row);
    fail_point.off().await;

    result.expect("purge");
    assert!(
        held(&db, "purge-sweep-removes", "Observation", Some("holder"))
            .await
            .is_empty(),
        "the sweep removed the row the transaction did not see"
    );
    assert!(
        captured_lines(logs, "exists again").is_empty(),
        "the resource did not exist again, so the sweep ran"
    );
}

/// The purge has committed when the sweep runs, so a failure of the sweep does not
/// fail it: it answers `Ok`, with one WARN. `skip: 1` lets the transaction's delete
/// of `search_index` through and fails the sweep's.
#[tokio::test]
async fn a_failed_sweep_after_a_purge_is_logged_and_the_purge_still_answers_ok() {
    let app = "fp-purge-sweep-fails";
    let Some((backend, db)) = purge_backend("purge_sweep_fails", app).await else {
        return;
    };
    let tenant = create_tenant("purge-sweep-fails");
    seed_holder(&backend, &tenant, "holder").await;
    let Some(fail_point) = FailPoint::enable(
        app,
        delete_fails_on(&backend, "search_index", 2),
        doc! { "skip": 1 },
    )
    .await
    else {
        return;
    };

    let (logs, _guard) = capture_purge_warnings();
    let result = backend.purge(&tenant, "Observation", "holder").await;
    let entered = fail_point.off_and_count().await;

    result.expect("the purge committed, so the failed sweep does not fail it");
    assert!(entered >= 1, "the sweep's delete met the failpoint");
    let warns = captured_lines(logs, "removing its search rows once more failed");
    assert_eq!(warns.len(), 1, "one WARN: {warns:?}");
    for field in ["purge-sweep-fails", "Observation", "holder"] {
        assert!(warns[0].contains(field), "missing {field}: {}", warns[0]);
    }
    assert!(
        held(&db, "purge-sweep-fails", "Observation", Some("holder"))
            .await
            .is_empty(),
        "the transaction had removed everything"
    );
}

/// Between the commit and the sweep another client can create the same id again.
/// The sweep would take the new resource's rows, so it asks the primary first and
/// is skipped when the resource is there. The sweep's read of `resources` is the
/// only `find` the purge sends there, so a failpoint that holds it for three seconds
/// opens the window deterministically: the id is created again through a second
/// backend (another `appName`, so the failpoint does not touch it) while the read
/// is held, and the new resource's rows are still there afterwards.
#[tokio::test]
async fn the_sweep_after_a_purge_is_skipped_when_the_resource_exists_again() {
    let app = "fp-purge-exists-again";
    let Some((backend, db)) = purge_backend("purge_exists_again", app).await else {
        return;
    };
    let database_name = backend.config().database_name.clone();
    let Some((other, _)) =
        purge_backend_with("purge_exists_again_other", "fp-purge-exists-again-other", {
            let database_name = database_name.clone();
            move |config| config.database_name = database_name
        })
        .await
    else {
        return;
    };
    let tenant = create_tenant("purge-exists-again");
    seed_holder(&backend, &tenant, "holder").await;
    let Some(fail_point) = FailPoint::enable(
        app,
        blocks_for(&backend, "find", "resources", 3_000),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let (logs, _guard) = capture_purge_logs("debug");
    let create_it_again = async {
        fail_point.wait_until_entered(1).await;
        other
            .seed_put(
                &tenant,
                "Observation",
                "holder",
                holder_json("holder", "Reborn"),
                FhirVersion::default(),
            )
            .await
            .expect("the id is created again");
    };
    let (result, ()) = tokio::join!(
        backend.purge(&tenant, "Observation", "holder"),
        create_it_again
    );
    fail_point.off().await;

    result.expect("purge");
    let now = held(&db, "purge-exists-again", "Observation", Some("holder")).await;
    assert_eq!(
        (now.resources, now.history),
        (1, 1),
        "the new resource: {now:?}"
    );
    assert!(
        now.rows > 0 && now.contained > 0,
        "the sweep was skipped, so the new resource keeps its rows: {now:?}"
    );
    assert_eq!(
        captured_lines(logs, "exists again").len(),
        1,
        "the skip is logged"
    );
}

// ---------------------------------------------------------------------------
// purge_all: batches of ids
// ---------------------------------------------------------------------------

/// Patients stored in `chunks` of 100 per transaction Bundle (well below the
/// deferral threshold, so their rows are written with them): `p-0001` ..
/// `p-{count}`. Every tenth holds a contained Organization, so it has contained
/// rows. Returns `false` when the server has no transactions and the test skips.
async fn seed_patients(
    backend: &MongoBackend,
    tenant: &TenantContext,
    count: usize,
    test: &str,
) -> bool {
    for first in (1..=count).step_by(100) {
        let entries: Vec<BundleEntry> = (first..(first + 100).min(count + 1))
            .map(|n| {
                let id = format!("p-{n:04}");
                let mut patient = patient_json(&id, &format!("Family{n}"));
                if n % 10 == 0 {
                    patient["contained"] =
                        json!([{"resourceType": "Organization", "id": "o", "name": "Inner"}]);
                    patient["managingOrganization"] = json!({"reference": "#o"});
                }
                BundleEntry {
                    method: BundleMethod::Put,
                    url: format!("Patient/{id}"),
                    resource: Some(patient),
                    if_match: None,
                    if_none_match: None,
                    if_none_exist: None,
                    criteria: None,
                    full_url: None,
                }
            })
            .collect();
        let Some(result) = process_transaction_or_skip(backend, tenant, entries, test).await else {
            return false;
        };
        assert!(result.entries.iter().all(|entry| entry.status == 201));
    }
    true
}

/// A type is purged in batches of ids, one transaction per batch. 1,100 Patients
/// make three batches (500, 500, 100). The `search_index` delete of the second
/// batch, which comes after its `resources` and history deletes, fails for good:
/// the call returns the error, the first 500 ids are gone from all four
/// collections and the other 600 still hold their `resources` document, all of
/// their history and their rows, per id, not only in total. That the second batch's
/// earlier deletes were rolled back is what makes the batch atomic. A batch leaves
/// the sync failures to the sweep, so the first batch's are still there. The call
/// logs one WARN that says it had purged 500. With the failpoint off a repeat
/// purges the 600 and answers their count, the sweep removes the sync failures, and
/// a repeat on the empty type answers 0.
#[tokio::test]
async fn purge_all_removes_a_type_in_batches_and_a_repeat_continues() {
    const NAME: &str = "purge_all_batches";
    let app = "fp-purge-all-batches";
    let Some((backend, db)) = purge_backend(NAME, app).await else {
        return;
    };
    let tenant = create_tenant("purge-all-batches");
    if !seed_patients(&backend, &tenant, 1_100, NAME).await {
        return;
    }
    // A few resources with two versions, in each of the batches. None is one of
    // the tenth holding a contained Organization: the update drops it.
    for id in ["p-0007", "p-0601", "p-1051"] {
        let (stored, _) = backend
            .seed_put(
                &tenant,
                "Patient",
                id,
                patient_json(id, "Second"),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        assert_eq!(stored.version_id(), "2");
    }
    // A bystander of another type in the tenant, and the rows a purge clears or
    // leaves, planted for one id of the first batch and one of the last.
    seed_holder(&backend, &tenant, "bystander").await;
    let bystander = held(&db, "purge-all-batches", "Observation", None).await;
    plant_side_rows(&backend, &db, "purge-all-batches", "Patient", "p-0100").await;
    plant_side_rows(&backend, &db, "purge-all-batches", "Patient", "p-1000").await;

    let everything: BTreeSet<String> = (1..=1_100).map(|n| format!("p-{n:04}")).collect();
    let with_contained: BTreeSet<String> = (1..=110).map(|n| format!("p-{:04}", n * 10)).collect();
    let before = held(&db, "purge-all-batches", "Patient", None).await;
    assert_eq!(before.resources, 1_100);
    assert_eq!(before.history, 1_103);
    assert!(before.rows >= 1_100, "{before:?}");
    assert!(
        before.contained >= 110,
        "the contained Organizations have rows: {before:?}"
    );

    let Some(fail_point) = FailPoint::enable(
        app,
        delete_fails_on(&backend, "search_index", 2),
        doc! { "skip": 1 },
    )
    .await
    else {
        return;
    };
    let (logs, _guard) = capture_purge_warnings();
    let result = backend.purge_all(&tenant, "Patient").await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Internal { message, .. })) = &result else {
        panic!("expected the second batch's error, got {result:?}");
    };
    assert!(
        message.starts_with("Failed to purge search index"),
        "{message}"
    );
    assert!(entered >= 1, "the second batch's delete met the failpoint");
    let stopped = captured_lines(logs, "purge_all stopped");
    assert_eq!(stopped.len(), 1, "one WARN: {stopped:?}");
    assert_eq!(number_field(&stopped[0], "purged"), Some(500));
    for text in [
        "purge-all-batches",
        "Patient",
        "Failed to purge search index",
    ] {
        assert!(stopped[0].contains(text), "missing {text}: {}", stopped[0]);
    }
    let remaining: BTreeSet<String> = everything.iter().skip(500).cloned().collect();
    let remaining_contained: BTreeSet<String> =
        with_contained.intersection(&remaining).cloned().collect();
    for (collection, field, expected) in [
        ("resources", "id", &remaining),
        ("resource_history", "id", &remaining),
        ("search_index", "resource_id", &remaining),
        (
            "search_index_contained",
            "resource_id",
            &remaining_contained,
        ),
    ] {
        assert_eq!(
            &ids_in(&db, collection, field, "purge-all-batches", "Patient").await,
            expected,
            "{collection}: exactly the first batch is gone, and the rest is whole"
        );
    }
    assert_eq!(
        side_rows(&db, "purge-all-batches", "Patient", "p-0100").await,
        SIDE_ROWS_BLANKED,
        "the first batch's prior body is blanked; its sync failure waits for the sweep"
    );
    assert_eq!(
        side_rows(&db, "purge-all-batches", "Patient", "p-1000").await,
        SIDE_ROWS_INTACT,
        "the unfinished batches' are not"
    );
    assert_eq!(
        held(&db, "purge-all-batches", "Observation", None).await,
        bystander
    );

    let purged = backend
        .purge_all(&tenant, "Patient")
        .await
        .expect("with the failpoint off the repeat finishes the type");
    assert_eq!(purged, 600, "a repeat answers what it purged itself");
    assert!(
        held(&db, "purge-all-batches", "Patient", None)
            .await
            .is_empty()
    );
    for id in ["p-0100", "p-1000"] {
        assert_eq!(
            side_rows(&db, "purge-all-batches", "Patient", id).await,
            SIDE_ROWS_PURGED,
            "{id}: the sweep after the batches removed the sync failure"
        );
    }
    assert_eq!(
        held(&db, "purge-all-batches", "Observation", None).await,
        bystander,
        "another type of the tenant is untouched"
    );
    assert_eq!(
        backend.purge_all(&tenant, "Patient").await.expect("empty"),
        0
    );
}

/// The type-wide sweep after the batches removes what the batches cannot see:
/// the history, and the rows, of an id that is no longer in `resources` (the state
/// a purge of the old code left). Other types and other tenants are untouched.
#[tokio::test]
async fn purge_all_removes_history_only_leftovers() {
    let Some((backend, db)) = purge_backend("purge_all_leftovers", "fp-purge-all-leftovers").await
    else {
        return;
    };
    let tenant = create_tenant("purge-all-leftovers");
    let other_tenant = create_tenant("purge-all-leftovers-b");
    seed_holder(&backend, &tenant, "live").await;
    seed_holder(&backend, &tenant, "leftover").await;
    seed_holder(&backend, &other_tenant, "live").await;
    backend
        .seed_put(
            &tenant,
            "Patient",
            "bystander",
            patient_json("bystander", "Other"),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    plant_side_rows(
        &backend,
        &db,
        "purge-all-leftovers",
        "Observation",
        "leftover",
    )
    .await;
    plant_side_rows(&backend, &db, "purge-all-leftovers", "Patient", "bystander").await;
    db.collection::<Document>("resources")
        .delete_many(doc! {
            "tenant_id": "purge-all-leftovers",
            "resource_type": "Observation",
            "id": "leftover",
        })
        .await
        .unwrap();
    let leftover = held(&db, "purge-all-leftovers", "Observation", Some("leftover")).await;
    assert_eq!((leftover.resources, leftover.history), (0, 2));
    let bystander = held(&db, "purge-all-leftovers", "Patient", None).await;
    let other = held(&db, "purge-all-leftovers-b", "Observation", None).await;

    let purged = backend
        .purge_all(&tenant, "Observation")
        .await
        .expect("purge_all");

    assert_eq!(purged, 1, "only the live resource is counted");
    assert!(
        held(&db, "purge-all-leftovers", "Observation", None)
            .await
            .is_empty(),
        "the leftover's history and rows are gone too"
    );
    assert_eq!(
        side_rows(&db, "purge-all-leftovers", "Observation", "leftover").await,
        SIDE_ROWS_PURGED
    );
    assert_eq!(
        held(&db, "purge-all-leftovers", "Patient", None).await,
        bystander
    );
    assert_eq!(
        side_rows(&db, "purge-all-leftovers", "Patient", "bystander").await,
        SIDE_ROWS_INTACT
    );
    assert_eq!(
        held(&db, "purge-all-leftovers-b", "Observation", None).await,
        other
    );
}

/// A resource created after the last batch's ids were read, and before the sweep
/// that follows the batches, used to lose its history, rows and prior bodies to
/// the sweep and keep its `resources` document: a live resource with no versions
/// and no search rows. The sweep takes `resources` too now, so the resource is
/// purged whole and counted. The sweep's first command is a count on `resources`,
/// the only `aggregate` the call sends there, so a failpoint that holds it opens
/// the window deterministically: the resource is created through a second backend
/// (another `appName`) while the count is held.
#[tokio::test]
async fn purge_all_purges_whole_a_resource_created_after_its_last_read() {
    let app = "fp-purge-all-late";
    let Some((backend, db)) = purge_backend("purge_all_late", app).await else {
        return;
    };
    let database_name = backend.config().database_name.clone();
    let Some((other, _)) = purge_backend_with("purge_all_late_other", "fp-purge-all-late-other", {
        let database_name = database_name.clone();
        move |config| config.database_name = database_name
    })
    .await
    else {
        return;
    };
    let tenant = create_tenant("purge-all-late");
    for id in ["p1", "p2", "p3"] {
        backend
            .seed_put(
                &tenant,
                "Patient",
                id,
                patient_json(id, "Early"),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    let Some(fail_point) = FailPoint::enable(
        app,
        blocks_for(&backend, "aggregate", "resources", 3_000),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let create_late = async {
        fail_point.wait_until_entered(1).await;
        other
            .seed_put(
                &tenant,
                "Patient",
                "late",
                patient_json("late", "Late"),
                FhirVersion::default(),
            )
            .await
            .expect("the late resource is created");
        assert_eq!(
            held(&db, "purge-all-late", "Patient", Some("late"))
                .await
                .resources,
            1
        );
    };
    let (result, ()) = tokio::join!(backend.purge_all(&tenant, "Patient"), create_late);
    fail_point.off().await;

    assert_eq!(
        result.expect("purge_all"),
        4,
        "three purged by the batches and one by the sweep"
    );
    assert!(
        held(&db, "purge-all-late", "Patient", None)
            .await
            .is_empty(),
        "the late resource is purged whole: not its history and rows only"
    );
}

/// The REST layer drops the handler of a request after `HFS_REQUEST_TIMEOUT`, and
/// for a large type that is how every call but the last one ends. A call that is
/// dropped before it finished logs one WARN with the number of resources it had
/// purged. 600 Patients make two batches (500, 100); the `search_index` delete of
/// the second is held, and the call is dropped when the failpoint is entered (a
/// `select!`, not a timer). The first batch stays purged, the WARN says 500, and a
/// repeat finishes the type once the dropped transaction is gone.
#[tokio::test]
async fn a_purge_all_that_is_dropped_says_how_many_it_purged_and_a_repeat_finishes() {
    const NAME: &str = "purge_all_dropped";
    let app = "fp-purge-all-dropped";
    let Some((backend, db)) = purge_backend(NAME, app).await else {
        return;
    };
    let tenant = create_tenant("purge-all-dropped");
    if !seed_patients(&backend, &tenant, 600, NAME).await {
        return;
    }
    // The first batch's `search_index` delete is let through (`skip: 1`); the
    // second's is held.
    let Some(fail_point) = FailPoint::enable(
        app,
        blocks_for(&backend, "delete", "search_index", 1_500),
        doc! { "skip": 1 },
    )
    .await
    else {
        return;
    };

    let (logs, _guard) = capture_purge_warnings();
    let outcome = tokio::select! {
        result = backend.purge_all(&tenant, "Patient") => Some(result),
        () = fail_point.wait_until_entered(1) => None,
    };
    fail_point.off().await;

    assert!(
        outcome.is_none(),
        "the call was dropped while its second batch was held: {outcome:?}"
    );
    let dropped = captured_lines(logs, "dropped before it finished");
    assert_eq!(dropped.len(), 1, "one WARN: {dropped:?}");
    assert_eq!(number_field(&dropped[0], "purged"), Some(500));
    for text in ["purge-all-dropped", "Patient"] {
        assert!(dropped[0].contains(text), "missing {text}: {}", dropped[0]);
    }
    assert!(
        captured_lines(logs, "purge_all stopped").is_empty(),
        "a dropped call is not an error return"
    );
    assert_eq!(
        held(&db, "purge-all-dropped", "Patient", None)
            .await
            .resources,
        100,
        "the first batch stays purged and the second was not committed"
    );

    // The dropped transaction holds its documents until the held command ends and
    // its session's abort has run; a repeat that meets it is run again, then
    // answers 503, so repeat until it is through.
    let mut purged = None;
    for _ in 0..30 {
        match backend.purge_all(&tenant, "Patient").await {
            Ok(count) => {
                purged = Some(count);
                break;
            }
            Err(StorageError::Backend(BackendError::Unavailable { .. })) => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(err) => panic!("the repeat failed: {err:?}"),
        }
    }
    assert_eq!(purged, Some(100), "a repeat purges what is left");
    assert!(
        held(&db, "purge-all-dropped", "Patient", None)
            .await
            .is_empty()
    );
}

/// The line `purge_all` logs for each halving.
const HALVING: &str = "halving it";
/// The line `purge_all` logs (at DEBUG) for each batch it purged.
const BATCH_PURGED: &str = "purge_all batch purged";

/// A batch whose transaction cannot hold its write set (388) is halved, and the
/// batch size grows back only after four batches in a row were purged at the
/// current size. Failing the first two deletes of `resources` with 388 turns the
/// 500-id batch of 700 into 250 and then 125 ids: two "halving it" WARNs, naming
/// those sizes, which retrying the same 500 would not log. Four batches of 125 follow,
/// and the next read asks for 250 and finds the 200 left: a size that doubled after
/// every batch would have had batches of 125, 250 and 325, and one that never grew
/// batches of 125 ending in one of 75. The type is purged whole.
#[tokio::test]
async fn purge_all_halves_a_batch_that_is_too_large_and_grows_it_back() {
    const NAME: &str = "purge_all_halving";
    let app = "fp-purge-all-halving";
    let Some((backend, db)) = purge_backend(NAME, app).await else {
        return;
    };
    let tenant = create_tenant("purge-all-halving");
    if !seed_patients(&backend, &tenant, 700, NAME).await {
        return;
    }
    let Some(fail_point) = FailPoint::enable(
        app,
        delete_fails_on(&backend, "resources", 388),
        doc! { "times": 2 },
    )
    .await
    else {
        return;
    };
    let (logs, _guard) = capture_purge_logs("debug");

    let result = backend.purge_all(&tenant, "Patient").await;
    let entered = fail_point.off_and_count().await;

    assert_eq!(result.expect("the halved batches succeed"), 700);
    assert_eq!(entered, 2);
    assert!(
        held(&db, "purge-all-halving", "Patient", None)
            .await
            .is_empty()
    );
    assert_eq!(
        logged_numbers(logs, HALVING, "batch_ids"),
        [500, 250],
        "the batches that did not fit"
    );
    assert_eq!(
        logged_numbers(logs, HALVING, "next_batch_ids"),
        [250, 125],
        "each is halved, not retried"
    );
    assert_eq!(
        logged_numbers(logs, BATCH_PURGED, "batch_ids"),
        [125, 125, 125, 125, 200],
        "the size doubles after the fourth batch in a row, not before, and then takes what is left"
    );
    assert!(
        captured_lines(logs, PURGING_WITHOUT_A_TRANSACTION).is_empty(),
        "a batch of more than one id is halved, not run without a transaction"
    );
}

/// A batch that the server rolls back under cache pressure (365) is halved too,
/// as one too large for the cache is, and the call succeeds. A purge of one
/// resource, and a batch of one id, have nothing smaller to try: each gets one more
/// transactional try after 100 ms and, rolled back again, runs without a
/// transaction. None answers `503`.
#[tokio::test]
async fn purge_all_halves_a_batch_rolled_back_for_cache_pressure() {
    const NAME: &str = "purge_all_pressure";
    let app = "fp-purge-all-pressure";
    let Some((backend, db)) = purge_backend(NAME, app).await else {
        return;
    };
    let tenant = create_tenant("purge-all-pressure");
    for n in 1..=6 {
        let id = format!("p{n}");
        backend
            .seed_put(
                &tenant,
                "Patient",
                &id,
                patient_json(&id, "Pressure"),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    let Some(fail_point) = FailPoint::enable(
        app,
        delete_fails_on(&backend, "resources", 365),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };
    let (logs, _guard) = capture_purge_warnings();

    let result = backend.purge_all(&tenant, "Patient").await;
    let entered = fail_point.off_and_count().await;

    assert_eq!(result.expect("the halved batches succeed"), 6);
    assert_eq!(entered, 1);
    assert!(
        held(&db, "purge-all-pressure", "Patient", None)
            .await
            .is_empty()
    );
    assert_eq!(logged_numbers(logs, HALVING, "batch_ids"), [6]);
    assert_eq!(logged_numbers(logs, HALVING, "next_batch_ids"), [3]);
    let halving = captured_lines(logs, HALVING);
    assert!(halving[0].contains("cache pressure"), "{}", halving[0]);

    // One resource, and a batch of one id: the transaction is tried twice (the
    // failpoint fires on both), then the purge runs without one, which meets the
    // failpoint spent. Neither answers `503`.
    for id in ["single", "lone"] {
        backend
            .seed_put(
                &tenant,
                "Patient",
                id,
                patient_json(id, "Pressure"),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    let oplog = Oplog::begin(&backend).await;
    let Some(fail_point) = FailPoint::enable(
        app,
        delete_fails_on(&backend, "resources", 365),
        doc! { "times": 2 },
    )
    .await
    else {
        return;
    };
    let one = backend.purge(&tenant, "Patient", "single").await;
    let entered_by_one = fail_point.off_and_count().await;
    one.expect("a purge of one resource never answers 503 for cache pressure");
    assert_eq!(entered_by_one, 2, "its transaction and the second try");
    assert_eq!(
        held(&db, "purge-all-pressure", "Patient", Some("single")).await,
        Held {
            resources: 0,
            history: 0,
            rows: 0,
            contained: 0
        }
    );
    assert_eq!(
        captured_lines(logs, PURGING_WITHOUT_A_TRANSACTION).len(),
        1,
        "the purge of one resource ran without a transaction, with its WARN"
    );

    let Some(fail_point) = FailPoint::enable(
        app,
        delete_fails_on(&backend, "resources", 365),
        doc! { "times": 2 },
    )
    .await
    else {
        return;
    };
    let lone = backend.purge_all(&tenant, "Patient").await;
    let entered_by_lone = fail_point.off_and_count().await;
    assert_eq!(
        lone.expect("a batch of one id never answers 503 for cache pressure"),
        1
    );
    assert_eq!(entered_by_lone, 2, "its transaction and the second try");
    assert!(
        held(&db, "purge-all-pressure", "Patient", None)
            .await
            .is_empty()
    );
    assert_eq!(
        captured_lines(logs, PURGING_WITHOUT_A_TRANSACTION).len(),
        2,
        "and so did the batch of one id, with its own WARN"
    );
    // Each of the two went without a transaction: a delete of its own, none in a
    // committed transaction.
    Oplog::assert_deleted(&oplog, "resources", 0, 2, "the two lone purges").await;
}

/// A `purge_all` batch whose resources have more versions in all than
/// `purge_transaction_max_versions` is halved before it writes anything, and a lone
/// id over the limit is purged without a transaction, with the WARN. With the limit
/// at 3, `a-heavy` (4 versions) and five single-version resources: the batch of 6
/// has 9 versions and the first 3 have 6, so the batch goes 6, 3, 1; the lone id is
/// purged sessionless; then the size stays 1 for three more batches (b, c and d: with
/// the heavy id, the fourth purged batch in a row since the last refusal) and doubles
/// to 2, which takes the last two (2 versions). The first halving is logged at WARN and the second at
/// DEBUG: a type whose resources average over 20 versions would otherwise log a WARN
/// every fifth transaction.
#[tokio::test]
async fn purge_all_halves_a_batch_over_the_version_limit_and_purges_a_heavy_id_alone() {
    const NAME: &str = "purge_all_versions";
    let app = "fp-purge-all-versions";
    let Some((backend, db)) = purge_backend_with(NAME, app, |config| {
        config.purge_transaction_max_versions = 3;
    })
    .await
    else {
        return;
    };
    let tenant = create_tenant("purge-all-versions");
    for (id, versions) in [
        ("a-heavy", 4),
        ("b", 1),
        ("c", 1),
        ("d", 1),
        ("e", 1),
        ("f", 1),
    ] {
        for version in 1..=versions {
            backend
                .seed_put(
                    &tenant,
                    "Patient",
                    id,
                    patient_json(id, &format!("Version{version}")),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
    }
    let (logs, _guard) = capture_purge_logs("debug");

    let purged = backend
        .purge_all(&tenant, "Patient")
        .await
        .expect("purge_all");

    assert_eq!(purged, 6);
    assert!(
        held(&db, "purge-all-versions", "Patient", None)
            .await
            .is_empty()
    );
    assert_eq!(logged_numbers(logs, HALVING, "batch_ids"), [6, 3]);
    assert_eq!(logged_numbers(logs, HALVING, "next_batch_ids"), [3, 1]);
    assert_eq!(
        logged_numbers(logs, HALVING, "versions"),
        [9, 6],
        "the versions of the batches that were over the limit"
    );
    let lone = captured_lines(logs, PURGING_WITHOUT_A_TRANSACTION);
    assert_eq!(lone.len(), 1, "one WARN for the lone id: {lone:?}");
    for text in [
        "a-heavy",
        "versions=4",
        "purge_transaction_max_versions (3)",
    ] {
        assert!(lone[0].contains(text), "missing {text}: {}", lone[0]);
    }
    assert_eq!(
        logged_numbers(logs, BATCH_PURGED, "batch_ids"),
        [1, 1, 1, 1, 2],
        "the heavy id alone, three more singles, then the batch doubles"
    );
    let in_transaction: Vec<bool> = captured_lines(logs, BATCH_PURGED)
        .iter()
        .map(|line| line.contains("in_transaction=true"))
        .collect();
    assert_eq!(in_transaction, [false, true, true, true, true]);
    let halvings = captured_lines(logs, HALVING);
    assert_eq!(halvings.len(), 2);
    assert!(
        halvings[0].contains("WARN"),
        "the first version-limit halving of a call is a WARN: {}",
        halvings[0]
    );
    assert!(
        halvings[1].contains("DEBUG"),
        "the next is at DEBUG: {}",
        halvings[1]
    );
}

/// A batch leaves the sync failures to the sweep after the batches, which removes
/// those of the whole type in one command: after a `purge_all` they are gone for
/// every resource of the type, including one that was no longer in `resources`,
/// and those of another type and another tenant are untouched.
#[tokio::test]
async fn purge_all_removes_the_sync_failures_of_the_type_and_no_others() {
    let Some((backend, db)) = purge_backend("purge_all_failures", "fp-purge-all-failures").await
    else {
        return;
    };
    let tenant = create_tenant("purge-all-failures");
    for id in ["p1", "p2"] {
        backend
            .seed_put(
                &tenant,
                "Patient",
                id,
                patient_json(id, "Failure"),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        plant_sync_failure(&backend, "purge-all-failures", "Patient", id).await;
    }
    // A failure of a resource that is not in `resources` any more: the sweep is
    // the only thing that can name it.
    plant_sync_failure(&backend, "purge-all-failures", "Patient", "gone").await;
    plant_sync_failure(&backend, "purge-all-failures", "Observation", "p1").await;
    plant_sync_failure(&backend, "purge-all-failures-b", "Patient", "p1").await;

    let purged = backend
        .purge_all(&tenant, "Patient")
        .await
        .expect("purge_all");

    assert_eq!(purged, 2);
    for id in ["p1", "p2", "gone"] {
        assert_eq!(
            side_rows(&db, "purge-all-failures", "Patient", id).await.1,
            0,
            "{id}: the sweep removed it"
        );
    }
    assert_eq!(
        side_rows(&db, "purge-all-failures", "Observation", "p1")
            .await
            .1,
        1,
        "another type is untouched"
    );
    assert_eq!(
        side_rows(&db, "purge-all-failures-b", "Patient", "p1")
            .await
            .1,
        1,
        "another tenant is untouched"
    );
}

/// A batch of one id that is still too large for a transaction is purged without
/// one, with the WARN. The oplog shows that id's deletes as commands of their own,
/// and the other id's as a committed transaction.
#[tokio::test]
async fn purge_all_purges_a_lone_id_without_a_transaction_when_it_is_still_too_large() {
    const NAME: &str = "purge_all_lone_388";
    let app = "fp-purge-all-lone-388";
    let Some((backend, db)) = purge_backend(NAME, app).await else {
        return;
    };
    let tenant = create_tenant("purge-all-lone-388");
    for id in ["a", "b"] {
        backend
            .seed_put(
                &tenant,
                "Patient",
                id,
                patient_json(id, id),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    let oplog = Oplog::begin(&backend).await;
    // The batch of 2 ids fails with 388 and is halved to "a", which fails with 388
    // too and is purged without a transaction. The failpoint is then spent, so the
    // fallback's delete and the transaction that purges "b" are not failed.
    let Some(fail_point) = FailPoint::enable(
        app,
        delete_fails_on(&backend, "resources", 388),
        doc! { "times": 2 },
    )
    .await
    else {
        return;
    };
    let (logs, _guard) = capture_purge_warnings();

    let result = backend.purge_all(&tenant, "Patient").await;
    let entered = fail_point.off_and_count().await;

    assert_eq!(
        result.expect("the lone id is purged without a transaction"),
        2
    );
    assert_eq!(entered, 2);
    assert!(
        held(&db, "purge-all-lone-388", "Patient", None)
            .await
            .is_empty()
    );
    let warns = captured_lines(logs, PURGING_WITHOUT_A_TRANSACTION);
    assert_eq!(warns.len(), 1, "one WARN: {warns:?}");
    for field in ["purge-all-lone-388", "Patient"] {
        assert!(warns[0].contains(field), "missing {field}: {}", warns[0]);
    }
    // "a" went without a transaction (one delete of its own); "b" in one.
    Oplog::assert_deleted(&oplog, "resources", 1, 1, "the lone id and the rest").await;
}

// ---------------------------------------------------------------------------
// plans
// ---------------------------------------------------------------------------

/// The winning plan of an `explain` of `command`, as text, printed for the record.
async fn plan_of(db: &mongodb::Database, name: &str, command: Document) -> String {
    let explain = db
        .run_command(doc! { "explain": command, "verbosity": "queryPlanner" })
        .await
        .unwrap_or_else(|e| panic!("explain for {name} failed: {e}"));
    // An aggregate explains at the top level when the whole pipeline was pushed
    // down into the plan, and under `stages[].$cursor` when it was not.
    let source = explain
        .get_array("stages")
        .ok()
        .and_then(|stages| {
            stages
                .iter()
                .find_map(|stage| stage.as_document()?.get_document("$cursor").ok())
        })
        .unwrap_or(&explain);
    let plan = source
        .get_document("queryPlanner")
        .and_then(|planner| planner.get_document("winningPlan"))
        .unwrap_or_else(|e| panic!("explain for {name} has no winningPlan: {e}: {explain:?}"))
        .to_string();
    println!("PURGE_PLAN {name}: {plan}");
    plan
}

/// The commands of a purge are served by the indexes that key them, with no scan
/// of a collection, except one: the single-key reads (the version count, the read
/// of whether the resource exists again, the settling read's probes), the `$in`
/// deletes of a `purge_all` batch of 500 ids, and the id read that pages the
/// batches. The sync-failure delete of a purge of one resource scans the ledger:
/// no index serves its three `_id.*` fields. That is known and listed as a
/// follow-up (an index on them), and it is why a batch does not run it.
#[tokio::test]
async fn the_purge_commands_are_served_by_their_identity_indexes() {
    let Some((backend, db)) = purge_backend("purge_plans", "fp-purge-plans").await else {
        return;
    };
    // The ledger's collection exists once a failure is recorded.
    plant_sync_failure(&backend, "t", "Patient", "p-0001").await;
    let ids: Vec<String> = (1..=500).map(|n| format!("p-{n:04}")).collect();
    let one = "p-0001";
    let in_ids = doc! { "$in": ids.clone() };
    let by_id = |id: Bson| doc! { "tenant_id": "t", "resource_type": "Patient", "id": id };
    let by_resource_id =
        |id: Bson| doc! { "tenant_id": "t", "resource_type": "Patient", "resource_id": id };

    // (name, command, indexes of which the plan must name one; none = a scan)
    let cases: Vec<(&str, Document, &[&str])> = vec![
        (
            "count_versions_of_one",
            doc! {
                "aggregate": "resource_history",
                "pipeline": [
                    { "$match": by_id(Bson::String(one.into())) },
                    { "$group": { "_id": 1, "n": { "$sum": 1 } } },
                ],
                "cursor": {},
            },
            &["idx_history_identity"],
        ),
        (
            "count_versions_of_a_batch",
            doc! {
                "aggregate": "resource_history",
                "pipeline": [
                    { "$match": by_id(Bson::Document(in_ids.clone())) },
                    { "$group": { "_id": 1, "n": { "$sum": 1 } } },
                ],
                "cursor": {},
            },
            &["idx_history_identity"],
        ),
        (
            "resource_exists_again",
            doc! { "find": "resources", "filter": by_id(Bson::String(one.into())), "limit": 1 },
            &["idx_resources_identity"],
        ),
        (
            "settle_probe_search_index",
            doc! { "find": "search_index", "filter": by_resource_id(Bson::String(one.into())), "limit": 1 },
            &["idx_search_resource", "idx_search_composite"],
        ),
        (
            "settle_probe_search_index_contained",
            doc! {
                "find": "search_index_contained",
                "filter": by_resource_id(Bson::String(one.into())),
                "limit": 1
            },
            &["idx_search_contained_resource"],
        ),
        (
            "settle_probe_prior_body",
            doc! {
                "find": "bulk_submission_changes",
                "filter": {
                    "tenant_id": "t",
                    "resource_type": "Patient",
                    "resource_id": one,
                    "previous_content": { "$ne": Bson::Null },
                },
                "limit": 1
            },
            &["idx_bulk_changes_resource"],
        ),
        (
            "delete_sync_failures_of_one",
            doc! {
                "delete": "secondary_sync_failures",
                "deletes": [{
                    "q": {
                        "_id.tenant_id": "t",
                        "_id.resource_type": "Patient",
                        "_id.resource_id": one,
                    },
                    "limit": 0
                }]
            },
            &[],
        ),
        (
            "batch_read_ids",
            doc! {
                "find": "resources",
                "filter": doc! { "tenant_id": "t", "resource_type": "Patient" },
                "projection": { "_id": 0, "id": 1 },
                "sort": { "id": 1 },
                "limit": 500,
                "hint": "idx_resources_identity"
            },
            &["idx_resources_identity"],
        ),
        (
            "batch_delete_resources",
            doc! {
                "delete": "resources",
                "deletes": [{ "q": by_id(Bson::Document(in_ids.clone())), "limit": 0 }]
            },
            &["idx_resources_identity"],
        ),
        (
            "batch_delete_history",
            doc! {
                "delete": "resource_history",
                "deletes": [{ "q": by_id(Bson::Document(in_ids.clone())), "limit": 0 }]
            },
            &["idx_history_identity"],
        ),
        (
            "batch_delete_search_index",
            doc! {
                "delete": "search_index",
                "deletes": [{ "q": by_resource_id(Bson::Document(in_ids.clone())), "limit": 0 }]
            },
            &["idx_search_resource", "idx_search_composite"],
        ),
        (
            "batch_delete_search_index_contained",
            doc! {
                "delete": "search_index_contained",
                "deletes": [{ "q": by_resource_id(Bson::Document(in_ids.clone())), "limit": 0 }]
            },
            &["idx_search_contained_resource"],
        ),
        (
            "batch_clear_previous_content",
            doc! {
                "update": "bulk_submission_changes",
                "updates": [{
                    "q": by_resource_id(Bson::Document(in_ids.clone())),
                    "u": { "$set": { "previous_content": Bson::Null } },
                    "multi": true
                }]
            },
            &["idx_bulk_changes_resource"],
        ),
    ];
    for (name, command, indexes) in cases {
        let plan = plan_of(&db, name, command).await;
        if indexes.is_empty() {
            assert!(
                plan.contains("COLLSCAN"),
                "{name} no longer scans its collection (an index now serves it?): {plan}"
            );
            continue;
        }
        assert!(
            !plan.contains("COLLSCAN"),
            "{name} scans its collection: {plan}"
        );
        assert!(
            indexes.iter().any(|index| plan.contains(index)),
            "{name} does not use one of {indexes:?}: {plan}"
        );
        if name == "batch_read_ids" {
            assert!(
                !plan.contains("\"SORT\""),
                "the id read sorts in memory: {plan}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// measurement
// ---------------------------------------------------------------------------

/// #2146: how long the purge transaction of one resource with a long history
/// takes, which is what `DEFAULT_PURGE_TRANSACTION_MAX_VERSIONS` is chosen from.
/// For each size N (default 5,000, 20,000 and 50,000; `HFS_TEST_PURGE_HISTORY_SIZES`
/// is a comma-separated list) it stores a Patient, plants N - 1 more
/// `resource_history` documents for it with `insert_many` (copies of its real
/// history document with distinct versions, not N real updates), raises
/// `purge_transaction_max_versions` so that the guard does not apply, and times
/// `purge`, twice. Each run prints
///
/// `PURGE_TXN_TIMING n=.. run=.. seconds=.. docs_per_s=.. fell_back=.. build=..`
///
/// `fell_back` is whether the transaction hit a 388, or was rolled back under cache
/// pressure twice, and the purge ran without one (its time then includes both, and
/// the 100 ms pause between the two tries). The planted documents are one small Patient's
/// each (about 0.3 KB), so a real history of larger resources is slower per
/// document. Ignored: it plants up to 50,000 documents, twice each. Run it by
/// itself, against a mongod with a small cache to see where a 388 comes:
///
/// ```bash
/// HFS_TEST_MONGODB_URL='mongodb://localhost:27018/?directConnection=true' \
///   cargo test -p helios-persistence --features mongodb --test mongodb_tests \
///   the_transactional_purge_of_a_long_history_is_timed -- --ignored --nocapture
/// ```
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "#2146 measurement; run alone with --ignored --nocapture"]
async fn the_transactional_purge_of_a_long_history_is_timed() {
    const NAME: &str = "the_transactional_purge_of_a_long_history_is_timed";
    let Some((backend, db)) = purge_backend_with(NAME, "fp-purge-history-timing", |config| {
        config.purge_transaction_max_versions = u64::MAX;
    })
    .await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("purge-history-timing");
    let sizes: Vec<usize> = std::env::var("HFS_TEST_PURGE_HISTORY_SIZES")
        .ok()
        .map(|sizes| {
            sizes
                .split(',')
                .map(|n| n.trim().parse().expect("a size"))
                .collect()
        })
        .unwrap_or_else(|| vec![5_000, 20_000, 50_000]);
    let history = db.collection::<Document>("resource_history");
    for n in sizes {
        for run in 1..=2 {
            let id = format!("long-{n}-{run}");
            backend
                .seed_put(
                    &tenant,
                    "Patient",
                    &id,
                    patient_json(&id, "Timed"),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
            let key = doc! {
                "tenant_id": "purge-history-timing",
                "resource_type": "Patient",
                "id": &id,
            };
            let real = history
                .find_one(key.clone())
                .await
                .unwrap()
                .expect("the history document of version 1");
            for first in (2..=n).step_by(5_000) {
                let copies: Vec<Document> = (first..(first + 5_000).min(n + 1))
                    .map(|version| {
                        let mut copy = real.clone();
                        copy.remove("_id");
                        copy.insert("version_id", version.to_string());
                        copy
                    })
                    .collect();
                history.insert_many(copies).await.unwrap();
            }
            assert_eq!(history.count_documents(key).await.unwrap(), n as u64);

            let (logs, guard) = capture_purge_warnings();
            let started = std::time::Instant::now();
            let result = backend.purge(&tenant, "Patient", &id).await;
            let seconds = started.elapsed().as_secs_f64();
            let fell_back = !captured_lines(logs, PURGING_WITHOUT_A_TRANSACTION).is_empty();
            drop(guard);
            // A transaction the cache cannot hold is the finding, not a failure of
            // the measurement: say how it ended (a 388, or two rollbacks under
            // cache pressure, run the purge without one) and go on.
            let outcome = match &result {
                Ok(()) => {
                    assert!(
                        held(&db, "purge-history-timing", "Patient", Some(&id))
                            .await
                            .is_empty()
                    );
                    "purged".to_string()
                }
                Err(err) => format!("failed: {err}"),
            };
            println!(
                "PURGE_TXN_TIMING n={n} run={run} seconds={seconds:.2} docs_per_s={:.0} \
                 fell_back={fell_back} outcome={outcome:?} build={}",
                n as f64 / seconds,
                if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "release"
                }
            );
        }
    }
}
