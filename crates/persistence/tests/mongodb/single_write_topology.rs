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

use super::*;

use futures::TryStreamExt;
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::mongo::Mongo;

use crate::bulk_submit::FailPoint;

/// A backend whose driver monitor is silent (see the module doc), connected
/// as `app`. `server_selection_timeout_ms` overrides the default when a test
/// wants a failing server selection to end quickly.
async fn backend_for_hello_failpoints(
    test: &str,
    app: &str,
    server_selection_timeout_ms: Option<u64>,
) -> Option<MongoBackend> {
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
    build_backend(config).await
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
#[tokio::test]
async fn an_update_whose_topology_check_fails_is_an_error_and_writes_nothing() {
    let app = "fp-2258-update";
    let Some(backend) = backend_for_hello_failpoints("single_write_update", app, None).await else {
        return;
    };
    let tenant = create_tenant("single_write_update");
    let (current, created) = backend
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
#[tokio::test]
async fn a_delete_whose_topology_check_fails_is_an_error_and_writes_nothing() {
    let app = "fp-2258-delete";
    let Some(backend) = backend_for_hello_failpoints("single_write_delete", app, None).await else {
        return;
    };
    let tenant = create_tenant("single_write_delete");
    backend
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
/// stays deleted.
#[tokio::test]
async fn a_restore_whose_topology_check_fails_is_an_error_and_writes_nothing() {
    let app = "fp-2258-restore";
    let Some(backend) = backend_for_hello_failpoints("single_write_restore", app, None).await
    else {
        return;
    };
    let tenant = create_tenant("single_write_restore");
    backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-2",
            observation("obs-2", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("seeding the Observation");
    backend
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
// A standalone server keeps working
// ---------------------------------------------------------------------------

/// Only a successful `hello` that names neither a replica set nor a mongos
/// lets a write run without a transaction (#2258), and such a server must keep
/// accepting create, update, delete and restore. The harness's shared server
/// is a replica set, so this test starts its own standalone mongod (no
/// `--replSet`), as the `helios-rest` standalone tests do.
#[tokio::test]
async fn single_resource_writes_still_work_on_a_standalone_server() {
    let started = super::container_cleanup::with_cleanup_label(
        Mongo::default()
            .with_tag("7.0")
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
    let hello = raw_test_client(&url)
        .await
        .expect("a raw client for the standalone server")
        .database("admin")
        .run_command(doc! { "hello": 1_i32 })
        .await
        .expect("hello on the standalone server");
    assert!(
        !hello.contains_key("setName") && !hello.get_str("msg").is_ok_and(|m| m == "isdbgrid"),
        "the container is not a standalone server: {hello}"
    );

    let backend = build_backend(MongoBackendConfig {
        connection_string: url,
        database_name: build_test_database_name("single_write_standalone"),
        app_name: "single-write-standalone".into(),
        data_dir: Some(repo_data_dir()),
        ..Default::default()
    })
    .await
    .expect("a backend on the standalone container");
    let tenant = create_tenant("single_write_standalone");

    // Create.
    let created = backend
        .create(
            &tenant,
            "Observation",
            observation("obs-s", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("a create on a standalone server");
    assert_eq!(created.version_id(), "1");
    assert_eq!(resource_rows(&backend, "obs-s").await, 1);
    assert_eq!(history_rows(&backend, "obs-s").await, 1);

    // Update.
    let updated = backend
        .update(&tenant, &created, observation("obs-s", "amended"))
        .await
        .expect("an update on a standalone server");
    assert_eq!(updated.version_id(), "2");
    assert_eq!(history_rows(&backend, "obs-s").await, 2);

    // Delete.
    backend
        .delete(&tenant, "Observation", "obs-s")
        .await
        .expect("a delete on a standalone server");
    assert!(
        matches!(
            backend.read(&tenant, "Observation", "obs-s").await,
            Err(StorageError::Resource(ResourceError::Gone { .. }))
        ),
        "the id is deleted"
    );
    assert_eq!(history_rows(&backend, "obs-s").await, 3);

    // Restore: a write to a deleted id brings it back.
    backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-s",
            observation("obs-s", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("a restore on a standalone server");
    let restored = backend
        .read(&tenant, "Observation", "obs-s")
        .await
        .unwrap()
        .expect("the restored resource is live");
    assert_eq!(restored.content()["status"], "final");
    assert_eq!(history_rows(&backend, "obs-s").await, 4);

    // The container stays up until here.
    drop(container);
}
