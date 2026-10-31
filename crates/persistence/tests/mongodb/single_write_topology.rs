//! #2258: a single-resource write on a deployment with transactions never
//! turns into separate, non-atomic commands because a setup step failed.
//!
//! Before, `hello` (the topology check), `start_session` and
//! `start_transaction` were each `.ok()`: a `hello` that failed read as "no
//! `setName`", i.e. a standalone server, and the create, update, delete or
//! restore ran as separate commands, `resources` and `resource_history` apart,
//! with nothing logged. Now a failed check is an error (a dropped connection
//! is retried briefly and then answered as `Unavailable`), and only a
//! successful `hello` that names neither a replica set nor a mongos lets a
//! write run without a transaction.
//!
//! Child module of the `mongodb_tests` root: `use super::*;` reaches its
//! harness. The tests drive the server's `failCommand` failpoint on `hello`,
//! scoped to the backend's `appName`. The shared server is a single-node
//! replica set, so a write that succeeds here runs in a transaction.
//!
//! Only the write's own `hello` may match the failpoint, so the backends here
//! keep the driver's server monitor quiet: `serverMonitoringMode=poll` turns
//! off the streaming `hello` and the round-trip monitor, and
//! `heartbeatFrequencyMS=600000` leaves the periodic check ten minutes away.
//! Connection handshakes use the legacy `isMaster`, which `failCommands:
//! ["hello"]` does not match. A test that wants a failing `hello` to stay
//! failing does not count matches, because the monitor's check after a
//! dropped connection is one of them.
//!
//! `retryWrites=false`, as in `write_abort_retry.rs`, so a write sent outside
//! a transaction carries no `txnNumber`: the profiler tells it apart from a
//! transactional one.
//!
//! `start_session` and `start_transaction` send no command, and once a
//! `hello` has succeeded nothing server-side can make them fail, so their
//! failure branches are covered by the unit tests of `storage.rs`
//! (`single_write_topology_tests`).
//!
//! #2308: a backend keeps the transactional answer of its first successful
//! `hello`, so later writes and transaction Bundles send none; a standalone
//! answer is never kept. The tests count `hello` commands with a `failCommand`
//! on `hello` that blocks for 0 ms and fails nothing (`counting_hello`): the
//! failpoint's `count` is then the number of `hello` commands the scoped
//! `appName` sent. A test that wants a failing `hello` needs a backend that has
//! asked nothing yet, which is why those tests seed their fixture through a
//! second backend on the same database (`seeded_and_fresh_backends`). The
//! standalone test runs its own server and counts there, and an ignored harness
//! reports a host-local, indicative latency figure next to the server's own
//! `hello` counter.

use super::*;

use futures::TryStreamExt;
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::mongo::Mongo;

use crate::bulk_submit::FailPoint;

/// The config of a backend whose driver monitor is silent (see the module
/// doc), connected as `app`. `server_selection_timeout_ms` overrides the
/// default when a test wants a failing server selection to end quickly.
async fn hello_failpoint_config(
    test: &str,
    app: &str,
    server_selection_timeout_ms: Option<u64>,
) -> Option<MongoBackendConfig> {
    let connection_string = shared_mongo::connection_string().await?;
    let connection_string = append_query_param(
        &connection_string,
        "retryWrites=false&serverMonitoringMode=poll&heartbeatFrequencyMS=600000",
    );
    let mut config = MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name(test),
        app_name: app.to_string(),
        data_dir: Some(repo_data_dir()),
        ..Default::default()
    };
    if let Some(timeout) = server_selection_timeout_ms {
        config.server_selection_timeout_ms = timeout;
    }
    Some(config)
}

/// A backend whose driver monitor is silent (see the module doc), connected
/// as `app`.
async fn backend_for_hello_failpoints(
    test: &str,
    app: &str,
    server_selection_timeout_ms: Option<u64>,
) -> Option<MongoBackend> {
    build_backend(hello_failpoint_config(test, app, server_selection_timeout_ms).await?).await
}

/// Two backends on one database: the first seeds a test's fixture, the second
/// has asked nothing yet and runs the write under test (#2308). A backend keeps
/// a transactional answer after its first check, so a test that wants its
/// write's `hello` to fail must not have seeded through the backend it fails.
async fn seeded_and_fresh_backends(test: &str, app: &str) -> Option<(MongoBackend, MongoBackend)> {
    let config = hello_failpoint_config(test, app, None).await?;
    let seeder = build_backend(config.clone()).await?;
    let fresh = build_backend(config).await?;
    Some((seeder, fresh))
}

fn observation(id: &str, status: &str) -> serde_json::Value {
    json!({
        "resourceType": "Observation",
        "id": id,
        "status": status,
        "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
    })
}

/// A `hello` the server refuses with a non-transient error.
fn hello_error() -> Document {
    doc! { "failCommands": ["hello"], "errorCode": 8_i32 }
}

/// A `hello` whose connection the server drops: a transient network failure.
fn hello_dropped() -> Document {
    doc! { "failCommands": ["hello"], "closeConnection": true }
}

async fn resource_rows(backend: &MongoBackend, id: &str) -> u64 {
    count_docs(backend, "resources", doc! { "id": id }).await
}

async fn history_rows(backend: &MongoBackend, id: &str) -> u64 {
    count_docs(backend, "resource_history", doc! { "id": id }).await
}

/// The error the write `what` (`"create"`, `"update"`, `"delete"` or
/// `"restore"`) ends with when its topology check was refused.
fn topology_failure(err: &StorageError, what: &str) -> bool {
    matches!(
        err,
        StorageError::Backend(BackendError::Internal { message, .. })
            if message.contains(&format!("topology before the {what}"))
    )
}

/// Whether the server behind `backend` is a replica set or a mongos, asked
/// directly (not through the backend, so no failpoint applies).
async fn deployment_is_transactional(backend: &MongoBackend) -> bool {
    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("a raw client for the topology question");
    let hello = client
        .database("admin")
        .run_command(doc! { "hello": 1_i32 })
        .await
        .expect("hello");
    hello.contains_key("setName") || hello.get_str("msg").is_ok_and(|msg| msg == "isdbgrid")
}

// ---------------------------------------------------------------------------
// Counting `hello` commands (#2308)
// ---------------------------------------------------------------------------

/// A `failCommand` that matches every `hello` of the scoped `appName` and
/// changes nothing: it blocks the command for 0 ms and fails nothing, so the
/// failpoint's `count` is the number of `hello` commands the backend sent.
fn counting_hello() -> Document {
    doc! { "failCommands": ["hello"], "blockConnection": true, "blockTimeMS": 0_i32 }
}

/// A server counter (`count` of `configureFailPoint`, `total` of a
/// `serverStatus` metric) as a number: the server sends an `Int32` or an
/// `Int64`.
fn counter(value: Option<&Bson>) -> i64 {
    match value {
        Some(Bson::Int32(count)) => i64::from(*count),
        Some(Bson::Int64(count)) => *count,
        other => panic!("the server returned an invalid counter: {other:?}"),
    }
}

/// Runs `work` while a [`counting_hello`] failpoint is on for `app`, and
/// returns its result with the number of `hello` commands `app` sent meanwhile.
/// `None`, after printing why, when failpoints are not available.
async fn hellos_during<T>(
    app: &str,
    work: impl std::future::Future<Output = T>,
) -> Option<(T, i64)> {
    let fail_point =
        FailPoint::enable(app, counting_hello(), doc! { "times": 1_000_000_i32 }).await?;
    let output = work.await;
    let hellos = fail_point.off_and_count().await;
    Some((output, hellos))
}

/// [`hellos_during`] on a server of the test's own (the standalone container),
/// whose `failCommand` is not shared with the rest of the suite, so there is no
/// lock to hold.
async fn hellos_on<T>(
    admin: &mongodb::Database,
    app: &str,
    work: impl std::future::Future<Output = T>,
) -> (T, i64) {
    let mut data = counting_hello();
    data.insert("appName", app);
    let on = admin
        .run_command(doc! {
            "configureFailPoint": "failCommand",
            "mode": { "times": 1_000_000_i32 },
            "data": data,
        })
        .await
        .expect("configureFailPoint failCommand on");
    let output = work.await;
    let off = admin
        .run_command(doc! { "configureFailPoint": "failCommand", "mode": "off" })
        .await
        .expect("configureFailPoint failCommand off");
    (output, counter(off.get("count")) - counter(on.get("count")))
}

/// A one-entry `POST` of `resource` for a transaction Bundle.
fn bundle_post(resource: serde_json::Value) -> BundleEntry {
    BundleEntry {
        method: BundleMethod::Post,
        url: resource["resourceType"]
            .as_str()
            .expect("a resourceType")
            .to_string(),
        resource: Some(resource),
        ..Default::default()
    }
}

fn patient(family: &str) -> serde_json::Value {
    json!({ "resourceType": "Patient", "name": [{ "family": family }] })
}

// ---------------------------------------------------------------------------
// The profiler: which writes ran in a transaction
// ---------------------------------------------------------------------------

/// Turns the profiler on for the backend's database, with a `system.profile`
/// big enough to hold every command of the test. `false` when the server
/// refuses (an external server without the privilege); the harness's own
/// server must not.
async fn start_profiling(db: &mongodb::Database) -> bool {
    let started = async {
        db.run_command(doc! { "profile": 0_i32 }).await?;
        // An operation still in flight from an earlier capture recreates the
        // default collection when it finishes; drop it again and retry.
        for attempt in 1.. {
            db.collection::<Document>("system.profile").drop().await?;
            match db
                .create_collection("system.profile")
                .capped(true)
                .size(16 * 1024 * 1024)
                .await
            {
                Ok(()) => break,
                Err(e)
                    if attempt < 10
                        && matches!(
                            e.kind.as_ref(),
                            mongodb::error::ErrorKind::Command(c) if c.code == 48
                        ) => {}
                Err(e) => return Err(e),
            }
        }
        db.run_command(doc! { "profile": 2_i32 }).await
    }
    .await;
    match started {
        Ok(_) => true,
        Err(e) => {
            assert!(
                !transactions_required(),
                "the harness's own MongoDB must allow the profiler: {e}"
            );
            eprintln!("the profiler is refused here ({e}); skipping the transaction assertions");
            false
        }
    }
}

/// The `(lsid, txnNumber)` of the transaction a profile entry ran in, `Null`
/// for each part it has none of (a command outside a transaction has no
/// `txnNumber`). The server records them in the entry's `command`; later
/// versions also at the top level.
fn transaction_of(entry: &Document) -> (Bson, Bson) {
    let field = |name: &str| {
        entry
            .get(name)
            .or_else(|| entry.get_document("command").ok()?.get(name))
            .cloned()
            .unwrap_or(Bson::Null)
    };
    (field("lsid"), field("txnNumber"))
}

/// Stops the profiler and returns the inserts into `collection`, oldest first.
async fn profiled_inserts(db: &mongodb::Database, collection: &str) -> Vec<Document> {
    db.run_command(doc! { "profile": 0_i32 }).await.unwrap();
    let ns = format!("{}.{collection}", db.name());
    db.collection::<Document>("system.profile")
        .find(doc! { "op": "insert", "ns": ns })
        .sort(doc! { "ts": 1_i32 })
        .await
        .expect("read system.profile")
        .try_collect()
        .await
        .expect("read system.profile")
}

// ---------------------------------------------------------------------------
// A failed topology check ends the write
// ---------------------------------------------------------------------------

/// A create whose `hello` is refused is an error and writes nothing. Before,
/// the refusal read as a standalone server and the create went ahead as
/// separate commands.
#[tokio::test]
async fn a_create_whose_topology_check_fails_is_an_error_and_writes_nothing() {
    let app = "fp-2258-create";
    let Some(backend) = backend_for_hello_failpoints("single_write_create", app, None).await else {
        return;
    };
    let tenant = create_tenant("single_write_create");
    let Some(fail_point) = FailPoint::enable(app, hello_error(), doc! { "times": 1 }).await else {
        return;
    };

    let result = backend
        .create(
            &tenant,
            "Observation",
            observation("obs-new", "final"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let err = result.expect_err("a refused topology check ends the create");
    assert!(topology_failure(&err, "create"), "{err:?}");
    assert_eq!(entered, 1, "the create's own hello is the one that failed");
    assert_eq!(resource_rows(&backend, "obs-new").await, 0);
    assert_eq!(history_rows(&backend, "obs-new").await, 0);
}

/// An update whose `hello` is refused is an error: the resource stays at its
/// version and gets no new history row.
///
/// A second backend on the same database seeds the Observation (#2308): the
/// backend under test has asked nothing yet, so it still sends the `hello` the
/// failpoint refuses. A backend that had seeded the fixture would hold a
/// transactional answer and send none.
#[tokio::test]
async fn an_update_whose_topology_check_fails_is_an_error_and_writes_nothing() {
    let app = "fp-2258-update";
    let Some((seeder, backend)) = seeded_and_fresh_backends("single_write_update", app).await
    else {
        return;
    };
    let tenant = create_tenant("single_write_update");
    let (current, created) = seeder
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("seeding the Observation");
    assert!(created);
    assert_eq!(current.version_id(), "1");
    let Some(fail_point) = FailPoint::enable(app, hello_error(), doc! { "times": 1 }).await else {
        return;
    };

    let result = backend
        .update(&tenant, &current, observation("obs-1", "amended"))
        .await;
    let entered = fail_point.off_and_count().await;

    let err = result.expect_err("a refused topology check ends the update");
    assert!(topology_failure(&err, "update"), "{err:?}");
    assert_eq!(entered, 1);
    let stored = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("the resource is still there");
    assert_eq!(stored.version_id(), "1");
    assert_eq!(stored.content()["status"], "final");
    assert_eq!(history_rows(&backend, "obs-1").await, 1);
}

/// A delete whose `hello` is refused is an error: the resource stays live.
/// A second backend seeds the Observation, as in the update test (#2308).
#[tokio::test]
async fn a_delete_whose_topology_check_fails_is_an_error_and_writes_nothing() {
    let app = "fp-2258-delete";
    let Some((seeder, backend)) = seeded_and_fresh_backends("single_write_delete", app).await
    else {
        return;
    };
    let tenant = create_tenant("single_write_delete");
    seeder
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("seeding the Observation");
    let Some(fail_point) = FailPoint::enable(app, hello_error(), doc! { "times": 1 }).await else {
        return;
    };

    let result = backend.delete(&tenant, "Observation", "obs-1").await;
    let entered = fail_point.off_and_count().await;

    let err = result.expect_err("a refused topology check ends the delete");
    assert!(topology_failure(&err, "delete"), "{err:?}");
    assert_eq!(entered, 1);
    let stored = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("the resource is still live");
    assert_eq!(stored.version_id(), "1");
    assert_eq!(history_rows(&backend, "obs-1").await, 1);
}

/// A restore of a deleted id whose `hello` is refused is an error: the id
/// stays deleted. A second backend seeds and deletes the Observation, as in
/// the update test (#2308).
#[tokio::test]
async fn a_restore_whose_topology_check_fails_is_an_error_and_writes_nothing() {
    let app = "fp-2258-restore";
    let Some((seeder, backend)) = seeded_and_fresh_backends("single_write_restore", app).await
    else {
        return;
    };
    let tenant = create_tenant("single_write_restore");
    seeder
        .create_or_update(
            &tenant,
            "Observation",
            "obs-2",
            observation("obs-2", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("seeding the Observation");
    seeder
        .delete(&tenant, "Observation", "obs-2")
        .await
        .expect("deleting the Observation");
    assert_eq!(history_rows(&backend, "obs-2").await, 2);
    let Some(fail_point) = FailPoint::enable(app, hello_error(), doc! { "times": 1 }).await else {
        return;
    };

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-2",
            observation("obs-2", "amended"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let err = result.expect_err("a refused topology check ends the restore");
    assert!(topology_failure(&err, "restore"), "{err:?}");
    assert_eq!(entered, 1);
    assert!(
        matches!(
            backend.read(&tenant, "Observation", "obs-2").await,
            Err(StorageError::Resource(ResourceError::Gone { .. }))
        ),
        "the id is still deleted"
    );
    assert_eq!(history_rows(&backend, "obs-2").await, 2);
}

// ---------------------------------------------------------------------------
// A transient failure is retried; a persistent one is Unavailable
// ---------------------------------------------------------------------------

/// A `hello` whose connection drops once is asked again, and the create then
/// commits in one transaction: its `resources` and `resource_history` inserts
/// carry the same `lsid` and `txnNumber`. Before, the drop read as a
/// standalone server and both inserts ran outside a transaction.
#[tokio::test]
async fn a_dropped_topology_check_is_retried_and_the_create_commits_in_one_transaction() {
    let app = "fp-2258-retry";
    let Some(backend) = backend_for_hello_failpoints("single_write_retry", app, None).await else {
        return;
    };
    if !deployment_is_transactional(&backend).await {
        assert!(
            !transactions_required(),
            "the harness's own MongoDB is a replica set"
        );
        eprintln!("Skipping: this MongoDB is a standalone server");
        return;
    }
    let tenant = create_tenant("single_write_retry");
    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("a raw client for the profiler");
    let db = client.database(&backend.config().database_name);
    if !start_profiling(&db).await {
        return;
    }
    let Some(fail_point) = FailPoint::enable(app, hello_dropped(), doc! { "times": 1 }).await
    else {
        return;
    };

    let result = backend
        .create(
            &tenant,
            "Observation",
            observation("obs-1", "final"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;
    let resources = profiled_inserts(&db, "resources").await;
    let history = profiled_inserts(&db, "resource_history").await;

    let stored = result.expect("a dropped topology check is asked again and the create commits");
    assert_eq!(stored.version_id(), "1");
    assert_eq!(
        entered, 1,
        "the first hello was dropped, the second was not"
    );
    assert_eq!(resources.len(), 1, "{resources:?}");
    assert_eq!(history.len(), 1, "{history:?}");
    let (resource_txn, history_txn) = (transaction_of(&resources[0]), transaction_of(&history[0]));
    assert_ne!(
        resource_txn.1,
        Bson::Null,
        "the `resources` insert runs in a transaction: {:?}",
        resources[0]
    );
    assert_eq!(
        resource_txn, history_txn,
        "the resource and its history row are one transaction"
    );
    assert_eq!(resource_rows(&backend, "obs-1").await, 1);
    assert_eq!(history_rows(&backend, "obs-1").await, 1);
}

/// A `hello` that keeps failing is answered as `Unavailable` (503 with
/// `Retry-After` over HTTP) and writes nothing. Before, the create went ahead
/// without a transaction once the connection came back.
#[tokio::test]
async fn a_topology_check_that_keeps_failing_is_unavailable_and_writes_nothing() {
    let app = "fp-2258-persistent";
    let Some(backend) =
        backend_for_hello_failpoints("single_write_persistent", app, Some(2000)).await
    else {
        return;
    };
    let tenant = create_tenant("single_write_persistent");
    // The failpoint takes a mode document, so "always" is a budget no test
    // reaches: the monitor's check after each dropped connection spends some.
    let Some(fail_point) = FailPoint::enable(app, hello_dropped(), doc! { "times": 100_000 }).await
    else {
        return;
    };

    let result = backend
        .create(
            &tenant,
            "Observation",
            observation("obs-1", "final"),
            FhirVersion::default(),
        )
        .await;
    fail_point.off().await;

    let err = result.expect_err("a topology check that never succeeds ends the create");
    assert!(
        matches!(err, StorageError::Backend(BackendError::Unavailable { .. })),
        "{err:?}"
    );
    assert_eq!(resource_rows(&backend, "obs-1").await, 0);
    assert_eq!(history_rows(&backend, "obs-1").await, 0);
}

// ---------------------------------------------------------------------------
// Guard: on the replica set a write is atomic
// ---------------------------------------------------------------------------

/// With nothing wrong with the topology check, a create and an update on the
/// replica set are each one transaction: a history insert the server refuses
/// takes the resource write down with it. (The positive control for the
/// tests above: the rows they find absent are absent because nothing ran, not
/// because nothing could be written.)
#[tokio::test]
async fn a_create_and_an_update_on_the_replica_set_are_atomic() {
    let app = "fp-2258-atomic";
    let Some(backend) = backend_for_hello_failpoints("single_write_atomic", app, None).await else {
        return;
    };
    if !deployment_is_transactional(&backend).await {
        assert!(
            !transactions_required(),
            "the harness's own MongoDB is a replica set"
        );
        eprintln!("Skipping: this MongoDB is a standalone server");
        return;
    }
    let tenant = create_tenant("single_write_atomic");

    // Create: the `resources` insert succeeds inside the transaction, the
    // history insert behind it is refused, and nothing is left.
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! { "failCommands": ["insert"], "errorCode": 8_i32 },
        doc! { "skip": 1 },
    )
    .await
    else {
        return;
    };
    let result = backend
        .create(
            &tenant,
            "Observation",
            observation("obs-1", "final"),
            FhirVersion::default(),
        )
        .await;
    fail_point.off().await;
    assert!(
        matches!(
            result,
            Err(StorageError::Backend(BackendError::Internal { .. }))
        ),
        "{result:?}"
    );
    assert_eq!(resource_rows(&backend, "obs-1").await, 0);
    assert_eq!(history_rows(&backend, "obs-1").await, 0);

    // Update: the compare-and-swap on `resources` succeeds inside the
    // transaction, the history insert behind it is refused, and the resource
    // is still at version 1.
    let (current, _) = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-2",
            observation("obs-2", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("seeding the Observation");
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! { "failCommands": ["insert"], "errorCode": 8_i32 },
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };
    let result = backend
        .update(&tenant, &current, observation("obs-2", "amended"))
        .await;
    fail_point.off().await;
    assert!(
        matches!(
            result,
            Err(StorageError::Backend(BackendError::Internal { .. }))
        ),
        "{result:?}"
    );
    let stored = backend
        .read(&tenant, "Observation", "obs-2")
        .await
        .unwrap()
        .expect("the resource is still there");
    assert_eq!(stored.version_id(), "1");
    assert_eq!(stored.content()["status"], "final");
    assert_eq!(history_rows(&backend, "obs-2").await, 1);
}

// ---------------------------------------------------------------------------
// A kept transactional answer: no `hello` in the steady state (#2308)
// ---------------------------------------------------------------------------

/// The first write of a backend on a replica set asks `hello`; every write
/// after it (create, update, delete, restore and a transaction Bundle) sends
/// none. Counted with a `failCommand` on `hello` that blocks for 0 ms and fails
/// nothing. Before, each of the six steps sent one.
#[tokio::test]
async fn on_the_replica_set_only_the_first_write_asks_hello() {
    let app = "hello-2308-steady";
    let Some(backend) = backend_for_hello_failpoints("single_write_steady", app, None).await else {
        return;
    };
    if !deployment_is_transactional(&backend).await {
        assert!(
            !transactions_required(),
            "the harness's own MongoDB is a replica set"
        );
        eprintln!("Skipping: this MongoDB is a standalone server");
        return;
    }
    let tenant = create_tenant("single_write_steady");
    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("a raw client for the profiler");
    let db = client.database(&backend.config().database_name);
    let mut counts: Vec<(&str, i64)> = Vec::new();

    // 1. The first create asks.
    let Some((first, hellos)) = hellos_during(
        app,
        backend.create(
            &tenant,
            "Observation",
            observation("obs-1", "final"),
            FhirVersion::default(),
        ),
    )
    .await
    else {
        return;
    };
    let first = first.expect("the first create");
    counts.push(("create", hellos));

    // 2. A second create does not, and still runs in one transaction.
    let profiling = start_profiling(&db).await;
    let Some((second, hellos)) = hellos_during(
        app,
        backend.create(
            &tenant,
            "Observation",
            observation("obs-2", "final"),
            FhirVersion::default(),
        ),
    )
    .await
    else {
        return;
    };
    second.expect("the second create");
    counts.push(("second create", hellos));
    if profiling {
        let resources = profiled_inserts(&db, "resources").await;
        let history = profiled_inserts(&db, "resource_history").await;
        assert_eq!(resources.len(), 1, "{resources:?}");
        assert_eq!(history.len(), 1, "{history:?}");
        let (resource_txn, history_txn) =
            (transaction_of(&resources[0]), transaction_of(&history[0]));
        assert_ne!(
            resource_txn.1,
            Bson::Null,
            "a write that skips the check still runs in a transaction: {:?}",
            resources[0]
        );
        assert_eq!(
            resource_txn, history_txn,
            "the resource and its history row are one transaction"
        );
    }

    // 3. An update.
    let Some((updated, hellos)) = hellos_during(
        app,
        backend.update(&tenant, &first, observation("obs-1", "amended")),
    )
    .await
    else {
        return;
    };
    updated.expect("the update");
    counts.push(("update", hellos));

    // 4. A delete.
    let Some((deleted, hellos)) =
        hellos_during(app, backend.delete(&tenant, "Observation", "obs-1")).await
    else {
        return;
    };
    deleted.expect("the delete");
    counts.push(("delete", hellos));

    // 5. A restore: a write to a deleted id brings it back.
    let Some((restored, hellos)) = hellos_during(
        app,
        backend.create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "final"),
            FhirVersion::default(),
        ),
    )
    .await
    else {
        return;
    };
    let (_, restored_by_create) = restored.expect("the restore");
    assert!(restored_by_create, "a write to a deleted id is a restore");
    counts.push(("restore", hellos));

    // 6. A transaction Bundle.
    let Some((bundle, hellos)) = hellos_during(
        app,
        backend.process_transaction(
            &tenant,
            vec![bundle_post(patient("Steady"))],
            FhirVersion::default(),
        ),
    )
    .await
    else {
        return;
    };
    let bundle = bundle.expect("the transaction Bundle");
    assert_eq!(bundle.entries.len(), 1);
    assert_eq!(bundle.entries[0].status, 201);
    counts.push(("transaction bundle", hellos));

    eprintln!("#2308 hello per write: {counts:?}");
    assert_eq!(
        counts,
        [
            ("create", 1),
            ("second create", 0),
            ("update", 0),
            ("delete", 0),
            ("restore", 0),
            ("transaction bundle", 0),
        ],
        "only the first write of a backend asks `hello`"
    );
}

/// A transaction Bundle that asks `hello` teaches the writes after it, as a
/// single write teaches the Bundles after it.
#[tokio::test]
async fn a_transaction_bundle_learns_the_answer_for_the_writes_after_it() {
    let app = "hello-2308-bundle-first";
    let Some(backend) = backend_for_hello_failpoints("single_write_bundle_first", app, None).await
    else {
        return;
    };
    if !deployment_is_transactional(&backend).await {
        assert!(
            !transactions_required(),
            "the harness's own MongoDB is a replica set"
        );
        eprintln!("Skipping: this MongoDB is a standalone server");
        return;
    }
    let tenant = create_tenant("single_write_bundle_first");

    let Some((bundle, bundle_hellos)) = hellos_during(
        app,
        backend.process_transaction(
            &tenant,
            vec![bundle_post(patient("First"))],
            FhirVersion::default(),
        ),
    )
    .await
    else {
        return;
    };
    bundle.expect("the transaction Bundle");
    let Some((created, create_hellos)) = hellos_during(
        app,
        backend.create(
            &tenant,
            "Observation",
            observation("obs-1", "final"),
            FhirVersion::default(),
        ),
    )
    .await
    else {
        return;
    };
    created.expect("the create");

    eprintln!("#2308 hello per write: bundle {bundle_hellos}, create {create_hellos}");
    assert_eq!(
        [bundle_hellos, create_hellos],
        [1, 0],
        "the Bundle asked once, the create after it did not"
    );
}

/// A kept transactional answer only ever leads to a write in a transaction. If
/// the server refuses that transaction as an illegal operation (code 20, what
/// a server without transactions answers), nothing is written and the create is
/// answered as `Unavailable` (503 with `Retry-After`), not replayed. The kept
/// answer is forgotten: the next write asks `hello` again, the one after it
/// does not. Before, the refusal was an `Internal` error (500).
#[tokio::test]
async fn a_write_refused_as_an_illegal_operation_forgets_the_answer_and_is_unavailable() {
    let app = "fp-2308-refused-write";
    let Some(backend) = backend_for_hello_failpoints("single_write_refused", app, None).await
    else {
        return;
    };
    if !deployment_is_transactional(&backend).await {
        assert!(
            !transactions_required(),
            "the harness's own MongoDB is a replica set"
        );
        eprintln!("Skipping: this MongoDB is a standalone server");
        return;
    }
    let tenant = create_tenant("single_write_refused");
    // The seed teaches the backend the transactional answer.
    backend
        .create(
            &tenant,
            "Observation",
            observation("obs-seed", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("seeding the Observation");
    // The create's first command in its transaction is a `find`.
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! { "failCommands": ["find"], "errorCode": 20_i32 },
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .create(
            &tenant,
            "Observation",
            observation("obs-1", "final"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let err = result.expect_err("a refused transaction ends the create");
    assert!(
        matches!(
            &err,
            StorageError::Backend(BackendError::Unavailable { message, .. })
                if message.contains("create") && !message.contains("failCommand")
        ),
        "{err:?}"
    );
    assert_eq!(entered, 1, "the create ran once and was not replayed");
    assert_eq!(resource_rows(&backend, "obs-1").await, 0);
    assert_eq!(history_rows(&backend, "obs-1").await, 0);

    // The next write asks again, and runs in a transaction.
    let Some((retried, hellos)) = hellos_during(
        app,
        backend.create(
            &tenant,
            "Observation",
            observation("obs-1", "final"),
            FhirVersion::default(),
        ),
    )
    .await
    else {
        return;
    };
    retried.expect("the create after the refusal");
    assert_eq!(
        hellos, 1,
        "the answer was forgotten, so the next write asks"
    );
    assert_eq!(resource_rows(&backend, "obs-1").await, 1);
    assert_eq!(history_rows(&backend, "obs-1").await, 1);

    // And the one after it does not.
    let Some((next, hellos)) = hellos_during(
        app,
        backend.create(
            &tenant,
            "Observation",
            observation("obs-2", "final"),
            FhirVersion::default(),
        ),
    )
    .await
    else {
        return;
    };
    next.expect("the create after the one that asked");
    assert_eq!(hellos, 0, "the answer is kept again");
}

/// The same for a transaction Bundle: a refused transaction applies no entry,
/// is answered as `Transient` (503 with `Retry-After`) after one attempt, and
/// forgets the kept answer. Before, it was a `BundleError`.
#[tokio::test]
async fn a_transaction_bundle_refused_as_an_illegal_operation_forgets_the_answer_and_is_transient()
{
    let app = "fp-2308-refused-bundle";
    let Some(backend) =
        backend_for_hello_failpoints("single_write_refused_bundle", app, None).await
    else {
        return;
    };
    if !deployment_is_transactional(&backend).await {
        assert!(
            !transactions_required(),
            "the harness's own MongoDB is a replica set"
        );
        eprintln!("Skipping: this MongoDB is a standalone server");
        return;
    }
    let tenant = create_tenant("single_write_refused_bundle");
    backend
        .create(
            &tenant,
            "Observation",
            observation("obs-seed", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("seeding the Observation");
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! { "failCommands": ["insert"], "errorCode": 20_i32 },
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .process_transaction(
            &tenant,
            vec![bundle_post(patient("Refused"))],
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let err = result.expect_err("a refused transaction ends the Bundle");
    assert!(
        matches!(err, TransactionError::Transient { attempts: 1, .. }),
        "{err:?}"
    );
    assert_eq!(entered, 1, "the Bundle ran once and was not replayed");
    assert_eq!(
        count_docs(&backend, "resources", doc! { "resource_type": "Patient" }).await,
        0,
        "no entry was applied"
    );

    let Some((bundle, hellos)) = hellos_during(
        app,
        backend.process_transaction(
            &tenant,
            vec![bundle_post(patient("Refused"))],
            FhirVersion::default(),
        ),
    )
    .await
    else {
        return;
    };
    bundle.expect("the Bundle after the refusal");
    assert_eq!(
        hellos, 1,
        "the answer was forgotten, so the next Bundle asks"
    );
    assert_eq!(
        count_docs(&backend, "resources", doc! { "resource_type": "Patient" }).await,
        1,
        "the Bundle after the refusal was applied (the control for the 0 above)"
    );
    let Some((created, hellos)) = hellos_during(
        app,
        backend.create(
            &tenant,
            "Observation",
            observation("obs-1", "final"),
            FhirVersion::default(),
        ),
    )
    .await
    else {
        return;
    };
    created.expect("the create after the Bundle");
    assert_eq!(hellos, 0, "the answer is kept again");
}

/// An update, a delete and a restore whose transaction the server refuses as an
/// illegal operation (code 20) are answered like the create: `Unavailable`
/// after one attempt, nothing written, and the kept answer forgotten, so the
/// same write asks `hello` once when it is sent again. Each of the three
/// `map_err`s of [`MongoBackend::unavailable_write_error`] is covered: without
/// one, the refusal is an `Internal` error (500) and the answer stays kept.
///
/// The first command of each write's transaction is a `find`. A restore has an
/// earlier `find` outside any transaction, the `read` of `create_or_update` that
/// finds the tombstone, so its failpoint skips one `find` first.
#[tokio::test]
async fn an_update_a_delete_and_a_restore_refused_as_an_illegal_operation_are_unavailable() {
    let app = "fp-2308-refused-others";
    let Some(backend) =
        backend_for_hello_failpoints("single_write_refused_others", app, None).await
    else {
        return;
    };
    if !deployment_is_transactional(&backend).await {
        assert!(
            !transactions_required(),
            "the harness's own MongoDB is a replica set"
        );
        eprintln!("Skipping: this MongoDB is a standalone server");
        return;
    }
    let tenant = create_tenant("single_write_refused_others");
    let refuse_find = || doc! { "failCommands": ["find"], "errorCode": 20_i32 };
    let unavailable_for = |err: &StorageError, what: &str| {
        matches!(
            err,
            StorageError::Backend(BackendError::Unavailable { message, .. })
                if message.contains(what) && !message.contains("failCommand")
        )
    };

    // Update. The seeds teach the backend the transactional answer.
    let stored = backend
        .create(
            &tenant,
            "Observation",
            observation("obs-u", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("seeding obs-u");
    let Some(fail_point) = FailPoint::enable(app, refuse_find(), doc! { "times": 1 }).await else {
        return;
    };
    let result = backend
        .update(&tenant, &stored, observation("obs-u", "amended"))
        .await;
    let entered = fail_point.off_and_count().await;
    let err = result.expect_err("a refused transaction ends the update");
    assert!(unavailable_for(&err, "update"), "{err:?}");
    assert_eq!(entered, 1, "the update ran once and was not replayed");
    assert_eq!(history_rows(&backend, "obs-u").await, 1);
    let Some((retried, hellos)) = hellos_during(
        app,
        backend.update(&tenant, &stored, observation("obs-u", "amended")),
    )
    .await
    else {
        return;
    };
    retried.expect("the update after the refusal");
    assert_eq!(hellos, 1, "the answer was forgotten, so the update asks");
    assert_eq!(history_rows(&backend, "obs-u").await, 2);

    // Delete.
    backend
        .create(
            &tenant,
            "Observation",
            observation("obs-d", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("seeding obs-d");
    let Some(fail_point) = FailPoint::enable(app, refuse_find(), doc! { "times": 1 }).await else {
        return;
    };
    let result = backend.delete(&tenant, "Observation", "obs-d").await;
    let entered = fail_point.off_and_count().await;
    let err = result.expect_err("a refused transaction ends the delete");
    assert!(unavailable_for(&err, "delete"), "{err:?}");
    assert_eq!(entered, 1, "the delete ran once and was not replayed");
    backend
        .read(&tenant, "Observation", "obs-d")
        .await
        .expect("the resource reads back")
        .expect("the resource is still live");
    let Some((retried, hellos)) =
        hellos_during(app, backend.delete(&tenant, "Observation", "obs-d")).await
    else {
        return;
    };
    retried.expect("the delete after the refusal");
    assert_eq!(hellos, 1, "the answer was forgotten, so the delete asks");

    // Restore.
    backend
        .create(
            &tenant,
            "Observation",
            observation("obs-r", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("seeding obs-r");
    backend
        .delete(&tenant, "Observation", "obs-r")
        .await
        .expect("deleting obs-r");
    assert_eq!(history_rows(&backend, "obs-r").await, 2);
    // The `read` that finds the tombstone is the first `find`; the restore's is
    // the second.
    let Some(fail_point) = FailPoint::enable(app, refuse_find(), doc! { "skip": 1 }).await else {
        return;
    };
    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-r",
            observation("obs-r", "amended"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;
    let err = result.expect_err("a refused transaction ends the restore");
    assert!(unavailable_for(&err, "restore"), "{err:?}");
    eprintln!("#2308 refused restore: the failpoint fired {entered} time(s)");
    assert_eq!(entered, 1, "the restore ran once and was not replayed");
    assert_eq!(history_rows(&backend, "obs-r").await, 2);
    assert!(
        matches!(
            backend.read(&tenant, "Observation", "obs-r").await,
            Err(StorageError::Resource(ResourceError::Gone { .. }))
        ),
        "the id is still deleted"
    );
    let Some((retried, hellos)) = hellos_during(
        app,
        backend.create_or_update(
            &tenant,
            "Observation",
            "obs-r",
            observation("obs-r", "amended"),
            FhirVersion::default(),
        ),
    )
    .await
    else {
        return;
    };
    let (_, restored) = retried.expect("the restore after the refusal");
    assert!(restored, "a write to a deleted id is a restore");
    assert_eq!(hellos, 1, "the answer was forgotten, so the restore asks");
    assert_eq!(history_rows(&backend, "obs-r").await, 3);
}

// ---------------------------------------------------------------------------
// A deployment that is down: no usable server (#2308)
// ---------------------------------------------------------------------------

/// A failpoint that closes every connection of the scoped `appName` that runs a
/// data command (`find`, `insert`) or a topology check (`hello`, `isMaster`),
/// including the monitor's check and each new connection's `isMaster`
/// handshake. The driver then marks the server `Unknown`, and server selection
/// fails until the failpoint is off. It is an outage of this backend only.
fn no_server() -> Document {
    doc! {
        "failCommands": ["find", "insert", "hello", "isMaster"],
        "closeConnection": true,
    }
}

/// Waits, up to five seconds, until a read through the backend's own client
/// succeeds, so that the recovery write of an outage test does not depend on
/// how soon the driver rediscovers the server within one server-selection
/// timeout. The read reads the seeded `Observation/obs-seed`. Panics when the
/// server never comes back.
async fn wait_until_usable(backend: &MongoBackend, tenant: &TenantContext) {
    for _ in 0..50 {
        if backend
            .read(tenant, "Observation", "obs-seed")
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the server did not become usable again within 5 s of the failpoint being off");
}

/// A write that finds no usable server is answered as `Unavailable` as soon as
/// server selection gives up, after one attempt and one server-selection
/// timeout, and is not run again. A kept answer sends no `hello`, so
/// `start_transaction` sends nothing and the outage shows at the write's first
/// command; the driver labels that failure `TransientTransactionError`. Before
/// this was handled, the write was replayed four times, each attempt waiting out
/// the server-selection timeout: four times the timeout, which at the default
/// 15 s is past the REST request timeout (`408`, no `Retry-After`). The elapsed
/// time is bounded at 1750 ms for the 1000 ms timeout (measured 1.00-1.04 s).
#[tokio::test]
async fn a_write_that_finds_no_usable_server_is_unavailable_at_once() {
    let app = "fp-2308-outage-write";
    let Some(backend) = backend_for_hello_failpoints("single_write_outage", app, Some(1000)).await
    else {
        return;
    };
    if !deployment_is_transactional(&backend).await {
        assert!(
            !transactions_required(),
            "the harness's own MongoDB is a replica set"
        );
        eprintln!("Skipping: this MongoDB is a standalone server");
        return;
    }
    let tenant = create_tenant("single_write_outage");
    // The seed teaches the backend the transactional answer.
    backend
        .create(
            &tenant,
            "Observation",
            observation("obs-seed", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("seeding the Observation");
    let Some(fail_point) = FailPoint::enable(app, no_server(), doc! { "times": 100_000_i32 }).await
    else {
        return;
    };

    // A read primes the outage: it makes the driver mark the server `Unknown`,
    // so the create's first attempt fails at server selection.
    let primed = backend.read(&tenant, "Observation", "obs-seed").await;
    let started = std::time::Instant::now();
    let result = backend
        .create(
            &tenant,
            "Observation",
            observation("obs-1", "final"),
            FhirVersion::default(),
        )
        .await;
    let elapsed = started.elapsed();
    fail_point.off().await;

    assert!(primed.is_err(), "the outage is on: {primed:?}");
    let err = result.expect_err("a create with no usable server is an error");
    eprintln!("#2308 outage: create answered after {elapsed:?}: {err:?}");
    assert!(
        elapsed < std::time::Duration::from_millis(1750),
        "a create with no usable server answers after one server-selection timeout (1000 ms), not two: {elapsed:?}"
    );
    let StorageError::Backend(BackendError::Unavailable { message, .. }) = &err else {
        panic!("a create with no usable server is Unavailable (503): {err:?}");
    };
    assert!(message.contains("create"), "{message}");
    for text in ["aborted", "failCommand", "127.0.0.1"] {
        assert!(!message.contains(text), "{text:?} in {message}");
    }
    assert_eq!(resource_rows(&backend, "obs-1").await, 0);
    assert_eq!(history_rows(&backend, "obs-1").await, 0);

    // The outage is over: wait until the backend's own client can select a
    // server again, then the next write asks again and commits.
    wait_until_usable(&backend, &tenant).await;
    backend
        .create(
            &tenant,
            "Observation",
            observation("obs-1", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("the create after the outage");
    assert_eq!(resource_rows(&backend, "obs-1").await, 1);
    assert_eq!(history_rows(&backend, "obs-1").await, 1);
}

/// The same for a transaction Bundle: the outage shows inside its transaction,
/// and the Bundle ends as `Transient` (503 with `Retry-After`) after one
/// attempt and one server-selection timeout, with no entry applied. Before, the
/// labelled failure was replayed and each replay waited out the
/// server-selection timeout.
///
/// The elapsed time is bounded at 1750 ms for the 1000 ms timeout. An awaited
/// `abortTransaction` after the failed attempt used to select a server again
/// (the driver has already moved the transaction to in-progress, whatever the
/// result of its first operation) and doubled the wait (measured 2.02-2.03 s).
/// The Bundle now drops the session instead, whose `Drop` aborts in the
/// background (measured about 1.0 s).
#[tokio::test]
async fn a_transaction_bundle_that_finds_no_usable_server_is_transient_at_once() {
    let app = "fp-2308-outage-bundle";
    let Some(backend) =
        backend_for_hello_failpoints("single_write_outage_bundle", app, Some(1000)).await
    else {
        return;
    };
    if !deployment_is_transactional(&backend).await {
        assert!(
            !transactions_required(),
            "the harness's own MongoDB is a replica set"
        );
        eprintln!("Skipping: this MongoDB is a standalone server");
        return;
    }
    let tenant = create_tenant("single_write_outage_bundle");
    backend
        .create(
            &tenant,
            "Observation",
            observation("obs-seed", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("seeding the Observation");
    let Some(fail_point) = FailPoint::enable(app, no_server(), doc! { "times": 100_000_i32 }).await
    else {
        return;
    };

    let primed = backend.read(&tenant, "Observation", "obs-seed").await;
    let started = std::time::Instant::now();
    let result = backend
        .process_transaction(
            &tenant,
            vec![bundle_post(patient("Outage"))],
            FhirVersion::default(),
        )
        .await;
    let elapsed = started.elapsed();
    fail_point.off().await;

    assert!(primed.is_err(), "the outage is on: {primed:?}");
    let err = result.expect_err("a Bundle with no usable server is an error");
    eprintln!("#2308 outage: Bundle answered after {elapsed:?}: {err:?}");
    assert!(
        elapsed < std::time::Duration::from_millis(1750),
        "a Bundle with no usable server answers after one server-selection timeout (1000 ms), not two: {elapsed:?}"
    );
    assert!(
        matches!(err, TransactionError::Transient { attempts: 1, .. }),
        "a Bundle with no usable server is Transient after one attempt: {err:?}"
    );
    assert_eq!(
        count_docs(&backend, "resources", doc! { "resource_type": "Patient" }).await,
        0,
        "no entry was applied"
    );

    // The outage is over: wait until the backend's own client can select a
    // server again, then the next Bundle commits.
    wait_until_usable(&backend, &tenant).await;
    backend
        .process_transaction(
            &tenant,
            vec![bundle_post(patient("Outage"))],
            FhirVersion::default(),
        )
        .await
        .expect("the Bundle after the outage");
    assert_eq!(
        count_docs(&backend, "resources", doc! { "resource_type": "Patient" }).await,
        1
    );
}

// ---------------------------------------------------------------------------
// A standalone server keeps working
// ---------------------------------------------------------------------------

/// Only a successful `hello` that names neither a replica set nor a mongos
/// lets a write run without a transaction (#2258), and such a server must keep
/// accepting create, update, delete and restore. The harness's shared server
/// is a replica set, so this test starts its own standalone mongod (no
/// `--replSet`), as the `helios-rest` standalone tests do.
///
/// A standalone answer is never kept (#2308): every write and every Bundle
/// asks `hello` again, so a standalone server restarted as a replica set gets
/// transactional writes from its next write on. This test is a guard and passes
/// before the change as well: it counts one `hello` per write, which the change
/// must not reduce. The server runs with `enableTestCommands=1` so the
/// `failCommand` that counts them is available.
///
/// It also records what the first transactional command of a session gets from
/// a standalone server: `IllegalOperation` (code 20), the code a kept answer
/// relies on to find out that transactions are gone. `start_transaction` sends
/// no command, so the refusal comes with that first command.
#[tokio::test]
async fn single_resource_writes_still_work_on_a_standalone_server() {
    let started = super::container_cleanup::with_cleanup_label(
        Mongo::default()
            .with_tag("7.0")
            .with_cmd([
                "mongod",
                "--bind_ip_all",
                "--setParameter",
                "enableTestCommands=1",
            ])
            .with_startup_timeout(std::time::Duration::from_secs(120)),
    )
    .start()
    .await;
    let container = match started {
        Ok(container) => container,
        Err(error) => {
            skip_or_fail_unavailable(&format!(
                "the standalone Mongo container did not start ({error})"
            ));
            return;
        }
    };
    let (host, port) = match (
        container.get_host().await,
        container.get_host_port_ipv4(27017).await,
    ) {
        (Ok(host), Ok(port)) => (host, port),
        (Err(error), _) | (_, Err(error)) => {
            skip_or_fail_unavailable(&format!(
                "the standalone Mongo container has no reachable address ({error})"
            ));
            return;
        }
    };
    let url = format!("mongodb://{host}:{port}/?directConnection=true");

    // The test means nothing against a replica set or a mongos.
    let admin_client = raw_test_client(&url)
        .await
        .expect("a raw client for the standalone server");
    let admin = admin_client.database("admin");
    let hello = admin
        .run_command(doc! { "hello": 1_i32 })
        .await
        .expect("hello on the standalone server");
    assert!(
        !hello.contains_key("setName") && !hello.get_str("msg").is_ok_and(|m| m == "isdbgrid"),
        "the container is not a standalone server: {hello}"
    );

    // The backend's monitor is quiet, so only the writes' own `hello` is counted.
    let app = "single-write-standalone";
    let backend = build_backend(MongoBackendConfig {
        connection_string: append_query_param(
            &url,
            "serverMonitoringMode=poll&heartbeatFrequencyMS=600000",
        ),
        database_name: build_test_database_name("single_write_standalone"),
        app_name: app.into(),
        data_dir: Some(repo_data_dir()),
        ..Default::default()
    })
    .await
    .expect("a backend on the standalone container");
    let tenant = create_tenant("single_write_standalone");
    let mut counts: Vec<(&str, i64)> = Vec::new();

    // Create.
    let (created, hellos) = hellos_on(
        &admin,
        app,
        backend.create(
            &tenant,
            "Observation",
            observation("obs-s", "final"),
            FhirVersion::default(),
        ),
    )
    .await;
    let created = created.expect("a create on a standalone server");
    counts.push(("create", hellos));
    assert_eq!(created.version_id(), "1");
    assert_eq!(resource_rows(&backend, "obs-s").await, 1);
    assert_eq!(history_rows(&backend, "obs-s").await, 1);

    // A second create asks again: a standalone answer is never kept.
    let (second, hellos) = hellos_on(
        &admin,
        app,
        backend.create(
            &tenant,
            "Observation",
            observation("obs-t", "final"),
            FhirVersion::default(),
        ),
    )
    .await;
    second.expect("a second create on a standalone server");
    counts.push(("second create", hellos));

    // Update.
    let (updated, hellos) = hellos_on(
        &admin,
        app,
        backend.update(&tenant, &created, observation("obs-s", "amended")),
    )
    .await;
    let updated = updated.expect("an update on a standalone server");
    counts.push(("update", hellos));
    assert_eq!(updated.version_id(), "2");
    assert_eq!(history_rows(&backend, "obs-s").await, 2);

    // Delete.
    let (deleted, hellos) =
        hellos_on(&admin, app, backend.delete(&tenant, "Observation", "obs-s")).await;
    deleted.expect("a delete on a standalone server");
    counts.push(("delete", hellos));
    assert!(
        matches!(
            backend.read(&tenant, "Observation", "obs-s").await,
            Err(StorageError::Resource(ResourceError::Gone { .. }))
        ),
        "the id is deleted"
    );
    assert_eq!(history_rows(&backend, "obs-s").await, 3);

    // Restore: a write to a deleted id brings it back.
    let (restored, hellos) = hellos_on(
        &admin,
        app,
        backend.create_or_update(
            &tenant,
            "Observation",
            "obs-s",
            observation("obs-s", "final"),
            FhirVersion::default(),
        ),
    )
    .await;
    restored.expect("a restore on a standalone server");
    counts.push(("restore", hellos));
    let restored = backend
        .read(&tenant, "Observation", "obs-s")
        .await
        .unwrap()
        .expect("the restored resource is live");
    assert_eq!(restored.content()["status"], "final");
    assert_eq!(history_rows(&backend, "obs-s").await, 4);

    // A transaction Bundle is refused, after asking, each time.
    for family in ["StandaloneOne", "StandaloneTwo"] {
        let (bundle, hellos) = hellos_on(
            &admin,
            app,
            backend.process_transaction(
                &tenant,
                vec![bundle_post(patient(family))],
                FhirVersion::default(),
            ),
        )
        .await;
        assert!(
            matches!(
                bundle,
                Err(TransactionError::UnsupportedIsolationLevel { .. })
            ),
            "{bundle:?}"
        );
        counts.push(("transaction bundle", hellos));
    }

    eprintln!("#2308 hello per write on a standalone server: {counts:?}");
    assert_eq!(
        counts,
        [
            ("create", 1),
            ("second create", 1),
            ("update", 1),
            ("delete", 1),
            ("restore", 1),
            ("transaction bundle", 1),
            ("transaction bundle", 1),
        ],
        "a standalone answer is never kept: every write asks"
    );

    // What the first transactional command of a session gets from this server.
    let mut session = admin_client
        .start_session()
        .await
        .expect("a session on the standalone server");
    session
        .start_transaction()
        .await
        .expect("start_transaction sends no command, so a standalone server does not refuse it");
    let err = admin_client
        .database("txn_probe")
        .collection::<Document>("txn_probe")
        .insert_one(doc! { "probe": 1_i32 })
        .session(&mut session)
        .await
        .expect_err("a standalone server refuses a command in a transaction");
    eprintln!("#2308 first transactional command on a standalone server: {err}");
    match err.kind.as_ref() {
        mongodb::error::ErrorKind::Command(command) => {
            assert_eq!(command.code, 20, "IllegalOperation: {command:?}");
        }
        other => panic!("expected a command error, got {other:?}"),
    }

    // The container stays up until here.
    drop(session);
    drop(container);
}

// ---------------------------------------------------------------------------
// Measurement harness (#2308)
// ---------------------------------------------------------------------------

/// How long a create takes and how many `hello` commands the server counts
/// meanwhile: `HFS_TEST_WRITE_TOPOLOGY_WRITES` sequential creates (default
/// 300) after 20 untimed ones, on the harness's RAM-backed replica set.
///
/// Host-local and indicative only: the `hello` round trip on one host is far
/// below a millisecond and the figures move with the machine's load. The
/// `hello` count, read from the server's own `serverStatus` counter
/// (server-wide, so run this test alone), is the evidence.
///
/// Run alone: `cargo test -p helios-persistence --features mongodb --test
/// mongodb_tests -- --ignored --exact
/// single_write_topology::sequential_creates_report_their_latency_and_hello_commands
/// --nocapture`.
#[tokio::test]
#[ignore = "#2308 measurement harness; run alone with --ignored --exact"]
async fn sequential_creates_report_their_latency_and_hello_commands() {
    let writes: usize = std::env::var("HFS_TEST_WRITE_TOPOLOGY_WRITES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(300);
    let app = "hello-2308-latency";
    let Some(backend) = backend_for_hello_failpoints("single_write_latency", app, None).await
    else {
        return;
    };
    let tenant = create_tenant("single_write_latency");
    let create = |id: String| {
        let (backend, tenant) = (&backend, &tenant);
        async move {
            backend
                .create(
                    tenant,
                    "Observation",
                    observation(&id, "final"),
                    FhirVersion::default(),
                )
                .await
                .expect("a create")
        }
    };
    for i in 0..20 {
        create(format!("warm-{i}")).await;
    }

    // The backend's connection string keeps the monitor quiet (see the module
    // doc), so the raw client adds no `hello` to the count either.
    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("a raw client for serverStatus");
    let admin = client.database("admin");
    let hello_total = || async {
        let status = admin
            .run_command(doc! { "serverStatus": 1_i32 })
            .await
            .expect("serverStatus");
        counter(
            status
                .get_document("metrics")
                .and_then(|metrics| metrics.get_document("commands"))
                .and_then(|commands| commands.get_document("hello"))
                .expect("the hello counter")
                .get("total"),
        )
    };

    let before = hello_total().await;
    let mut times = Vec::with_capacity(writes);
    let started = std::time::Instant::now();
    for i in 0..writes {
        let at = std::time::Instant::now();
        create(format!("timed-{i}")).await;
        times.push(at.elapsed());
    }
    let total = started.elapsed();
    let after = hello_total().await;

    times.sort();
    let micros = |at: usize| times[at.min(times.len().saturating_sub(1))].as_micros();
    let mean = total.as_micros() / writes.max(1) as u128;
    let delta = after - before;
    eprintln!(
        "#2308 host-local, indicative: {writes} sequential creates in {total:?}; \
         per create mean {mean} us, p50 {} us, p95 {} us; \
         server hello counter +{delta} ({:.3} per create)",
        micros(writes / 2),
        micros(writes * 95 / 100),
        delta as f64 / writes.max(1) as f64,
    );
}
