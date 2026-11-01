//! A single-resource write whose transaction the server aborts without a
//! conflicting writer is run again. The case this pins: a chunk migration
//! holding the critical section of `search_index` or `resources`, which the
//! server reports as `StaleConfig` (13388) labelled `TransientTransactionError`.
//! Before, that label alone was read as a lost race: a `PUT` ended as a 500
//! when the abort hit the search-index delete, or as a 409 "expected version
//! 1, but found unknown" when it hit the resource update, with nobody having
//! won anything.
//!
//! A plain `WriteConflict` (112) raised in the write's search-index step is run
//! again too (#2269): the rows of `search_index` are also written by writers
//! that leave the resource's version alone, so it does not say another write
//! won, and it was a 500 before. A commit whose outcome is unknown is sent again
//! while the write itself is never re-run (#2270); if it stays unknown the
//! answer is `CommitOutcomeUnknown`, no longer a generic 500. A commit that
//! stays unknown is settled by one majority read of the write's history row
//! (#2304): a stored row answers as committed, anything else stays
//! `CommitOutcomeUnknown`.
//!
//! A plain `WriteConflict` on a create's resource insert, history insert or
//! commit, or on a restore's history insert or commit, is run again too (#2300;
//! it was a 500). The rerun's existence check or tombstone read decides whether
//! another write of the same id won: a create whose id was stored in the
//! meantime is an `AlreadyExists`, and a `PUT` to a new id that another write
//! created in the meantime reads again and updates it.
//!
//! A plain `WriteConflict` on an update's or a delete's history insert or
//! commit is run again too (#2342). It used to be a 409 "expected version 1,
//! but found unknown" for a resource that had not moved, and a `PUT` or a
//! `DELETE` without `If-Match` retried it as a lost race. By then the update
//! or delete holds the resource document, so the conflict names no winner and
//! the rerun's version check decides. A `WriteConflict` on the resource
//! document itself is still a lost race.
//!
//! Child module of the `mongodb_tests` root, like `transaction_retry.rs`:
//! `use super::*;` reaches its harness. Every test drives the server's
//! `failCommand` failpoint, scoped to the backend's `appName`, against a
//! backend with `retryWrites=false` so the driver's own retries do not
//! muddy the counts.

use super::*;

use crate::bulk_submit::FailPoint;

/// The server's `StaleConfig` code.
const STALE_CONFIG: i32 = 13388;

/// Attempts a write makes before it gives up (`WRITE_ABORT_RETRY`).
const WRITE_ABORT_ATTEMPTS: i64 = 4;

/// Commits a write sends while the outcome stays unknown: the first and the
/// Bundle path's two re-sends (`SINGLE_WRITE_COMMIT_RETRIES`).
const COMMITS_WHILE_UNKNOWN: i64 = 3;

/// Longest a write whose commit never applied may take (#2419): a third of the
/// harness's settling-read bound ([`TEST_SETTLE_READ_MAX_TIME_MS`]). Such a
/// read has its floor near the majority point and returns as soon as the
/// majority catches up, so one that waited out the bound fails here instead of
/// passing 30 s later. Not tighter, because on the loaded CI host the majority
/// itself has lagged by more than a second.
const NEVER_APPLIED_MAX_ELAPSED: std::time::Duration =
    std::time::Duration::from_millis(TEST_SETTLE_READ_MAX_TIME_MS / 3);

/// A never-applied write came back well before the settling-read bound.
fn assert_did_not_wait_out_the_settle_bound(elapsed: std::time::Duration) {
    assert!(
        elapsed < NEVER_APPLIED_MAX_ELAPSED,
        "a never-applied commit's settling read must not wait out the bound: took {elapsed:?}"
    );
}

/// The failpoint's `data` for a `failCommand` that aborts `command` the way a
/// chunk migration's critical section does.
fn placement_change_on(command: &str) -> Document {
    doc! {
        "failCommands": [command],
        "errorCode": STALE_CONFIG,
        "errorLabels": ["TransientTransactionError"],
    }
}

/// The failpoint's `data` for a `failCommand` that aborts `command` the way an
/// index build's catalog change does on mongo 5.0: `SnapshotUnavailable`
/// (246) labelled `TransientTransactionError`. This is the abort behind the
/// 500s of #2045, reproduced on the parent of #1997's merge.
fn snapshot_unavailable_on(command: &str) -> Document {
    doc! {
        "failCommands": [command],
        "errorCode": 246,
        "errorLabels": ["TransientTransactionError"],
    }
}

/// The failpoint's `data` for the labelled `WriteConflict` of a lost race.
fn write_conflict_on(command: &str) -> Document {
    doc! {
        "failCommands": [command],
        "errorCode": 112,
        "errorLabels": ["TransientTransactionError"],
    }
}

/// [`write_conflict_on`], limited to one collection (`"<database>.<collection>"`),
/// so a command of the same name on another collection is not the one that fails.
fn write_conflict_on_namespace(command: &str, namespace: &str) -> Document {
    let mut data = write_conflict_on(command);
    data.insert("namespace", namespace);
    data
}

/// The failpoint's `data` for a `commitTransaction` that *applies* and then
/// reports a write-concern failure (`WriteConcernFailed`, 64), which the driver
/// labels `UnknownTransactionCommitResult`: the commit may have landed.
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

/// What the failpoint counts for the commits of [`COMMITS_WHILE_UNKNOWN`] sends
/// when each is delayed (see [`applied_commit_with_unknown_result`]): it counts a
/// delayed command twice.
const DELAYED_COMMITS_WHILE_UNKNOWN: i64 = 2 * COMMITS_WHILE_UNKNOWN;

/// [`commit_with_unknown_result`] with each reply delayed, the shape of a commit
/// that timed out on `wtimeout` and had replicated by the time the reply
/// arrived (#2304).
///
/// The server skips the wait for the commit to reach the majority when it injects
/// the write-concern error, so the commit can be a few tens of milliseconds
/// short of the majority snapshot when the plain failpoint's reply arrives (on
/// the harness's single-node replica set the row became visible to a majority
/// read about 60 ms later). The backend's settling read waits for the majority
/// itself, so it settles that case without any delay; this failpoint covers the
/// other one, where the majority has already reached the commit when the read
/// runs. The failpoint counts each delayed commit twice: expect
/// [`DELAYED_COMMITS_WHILE_UNKNOWN`] entries for the three commits sent.
fn applied_commit_with_unknown_result() -> Document {
    let mut data = commit_with_unknown_result();
    data.insert("blockConnection", true);
    data.insert("blockTimeMS", 300_i64);
    data
}

/// The failpoint's `data` for a `commitTransaction` the server refuses without
/// running it, so nothing is applied, but with the `UnknownTransactionCommitResult`
/// label (the driver's own for code 50, `classify_commit_signals`): the answer
/// says the commit may have been applied. Code 91 is not used: it is an SDAM
/// state-change error that marks the server Unknown.
///
/// The refused commit leaves its server-side transaction open until
/// `transactionLifetimeLimitSeconds` or the next transaction on that pooled
/// server session ends it, so a test that uses this must not write the same
/// documents afterwards. Until it ends, the test database's drop waits behind
/// that open transaction, and the cleanup sweeper's `outstanding=` count at
/// exit includes these tests.
fn commit_refused_with_unknown_result() -> Document {
    doc! {
        "failCommands": ["commitTransaction"],
        "errorCode": 50,
        "errorLabels": ["UnknownTransactionCommitResult"],
    }
}

/// An active SearchParameter on Patient with a url of its own.
fn search_parameter(id: &str, code: &str) -> serde_json::Value {
    json!({
        "resourceType": "SearchParameter",
        "id": id,
        "url": format!("http://example.org/fhir/SearchParameter/{id}"),
        "name": id,
        "status": "active",
        "code": code,
        "base": ["Patient"],
        "type": "string",
        "expression": "Patient.name.where(use='nickname').given"
    })
}

fn observation(id: &str, status: &str) -> serde_json::Value {
    json!({
        "resourceType": "Observation",
        "id": id,
        "status": status,
        "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
    })
}

/// An Observation with no id, as a `POST` hands it to `create`.
fn new_observation(status: &str) -> serde_json::Value {
    json!({
        "resourceType": "Observation",
        "status": status,
        "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
    })
}

/// [`write_conflict_on_namespace`] for the `insert` into `collection` of the
/// backend's database.
fn write_conflict_on_insert_into(backend: &MongoBackend, collection: &str) -> Document {
    write_conflict_on_namespace(
        "insert",
        &format!("{}.{collection}", backend.config().database_name),
    )
}

/// A backend with one Observation `obs-1` at version 1.
async fn backend_with_observation(test: &str, app: &str) -> Option<(MongoBackend, TenantContext)> {
    let backend = create_backend_with_app_name(test, app).await?;
    let tenant = create_tenant(test);
    let (stored, created) = backend
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
    assert_eq!(stored.version_id(), "1");
    Some((backend, tenant))
}

/// [`backend_with_observation`], with `obs-1` deleted (version 2 is its tombstone).
async fn backend_with_deleted_observation(
    test: &str,
    app: &str,
) -> Option<(MongoBackend, TenantContext)> {
    let (backend, tenant) = backend_with_observation(test, app).await?;
    backend
        .delete(&tenant, "Observation", "obs-1")
        .await
        .expect("deleting the seeded Observation");
    Some((backend, tenant))
}

/// The number of `resources` and `resource_history` rows the tenant holds for
/// Observations.
async fn tenant_rows(backend: &MongoBackend, tenant: &TenantContext) -> (u64, u64) {
    let scope = doc! {"tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation"};
    (
        count_docs(backend, "resources", scope.clone()).await,
        count_docs(backend, "resource_history", scope).await,
    )
}

/// The answer is `Unavailable` (a retryable 503) and its client-facing message
/// leaves the driver error to the log.
fn assert_unavailable_without_driver_text<T: std::fmt::Debug>(result: &Result<T, StorageError>) {
    let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = result else {
        panic!("expected Unavailable, got {result:?}");
    };
    assert!(
        !message.contains("112") && !message.contains("WriteConflict"),
        "the client-facing message leaves the driver error to the log: {message}"
    );
}

async fn index_rows(backend: &MongoBackend, id: &str) -> u64 {
    count_docs(backend, "search_index", doc! { "resource_id": id }).await
}

async fn history_rows(backend: &MongoBackend, id: &str) -> u64 {
    count_docs(backend, "resource_history", doc! { "id": id }).await
}

/// Check the committed payload and token values, not just unchanged row counts.
async fn assert_status(
    backend: &MongoBackend,
    tenant: &TenantContext,
    id: &str,
    expected: &str,
    absent: &str,
) {
    let stored = backend
        .read(tenant, "Observation", id)
        .await
        .unwrap()
        .expect("the committed resource exists");
    assert_eq!(stored.content()["status"], expected);
    let scope = doc! {
        "tenant_id": tenant.tenant_id().as_str(),
        "resource_type": "Observation",
        "resource_id": id,
        "param_name": "status",
    };
    let mut present = scope.clone();
    present.insert("value_token_code", expected);
    let mut stale = scope;
    stale.insert("value_token_code", absent);
    assert_eq!(count_docs(backend, "search_index", present).await, 1);
    assert_eq!(count_docs(backend, "search_index", stale).await, 0);
}

/// `PUT` of an existing resource, aborted on the search-index delete the way
/// a migration of `search_index` aborts it: the write is run again and
/// commits version 2. This was the 500 "Failed to delete search index
/// entries".
#[tokio::test]
async fn an_update_aborted_on_the_index_delete_is_run_again() {
    let app = "fp-write-abort-index-delete";
    let Some((backend, tenant)) = backend_with_observation("write_abort_index_delete", app).await
    else {
        return;
    };
    let Some(fail_point) =
        FailPoint::enable(app, placement_change_on("delete"), doc! { "times": 1 }).await
    else {
        return;
    };
    let rows_before = index_rows(&backend, "obs-1").await;
    assert!(rows_before > 0);

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "amended"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let (stored, created) = result.expect("an aborted write is run again and commits");
    assert!(!created);
    assert_eq!(stored.version_id(), "2");
    assert_eq!(
        entered, 1,
        "the failpoint fires once; the second attempt is clean"
    );
    assert_eq!(index_rows(&backend, "obs-1").await, rows_before);
    assert_eq!(history_rows(&backend, "obs-1").await, 2);
    assert_status(&backend, &tenant, "obs-1", "amended", "final").await;
}

/// The same abort on the resource's own `update_one`: before, the label was
/// read as a lost race and the client got a 409 "expected version 1, but
/// found unknown" although the resource had not moved.
#[tokio::test]
async fn an_update_aborted_on_the_resource_update_is_not_a_lost_race() {
    let app = "fp-write-abort-resource-update";
    let Some((backend, tenant)) =
        backend_with_observation("write_abort_resource_update", app).await
    else {
        return;
    };
    let Some(fail_point) =
        FailPoint::enable(app, placement_change_on("update"), doc! { "times": 1 }).await
    else {
        return;
    };

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "amended"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let (stored, _) = result.expect("nobody won a race; the write is run again");
    assert_eq!(stored.version_id(), "2");
    assert_eq!(entered, 1);
}

/// The abort arrives at commit, after every statement ran: nothing committed,
/// so the write is run again from its first read.
#[tokio::test]
async fn an_update_aborted_at_commit_is_run_again() {
    let app = "fp-write-abort-commit";
    let Some((backend, tenant)) = backend_with_observation("write_abort_commit", app).await else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        placement_change_on("commitTransaction"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "amended"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let (stored, _) = result.expect("an abort at commit is run again");
    assert_eq!(stored.version_id(), "2");
    assert_eq!(entered, 1);
    assert_eq!(history_rows(&backend, "obs-1").await, 2);
    assert_status(&backend, &tenant, "obs-1", "amended", "final").await;
}

/// A write the server keeps aborting gives up as `Unavailable` (a retryable
/// 503), after exactly the policy's attempts, with nothing committed.
#[tokio::test]
async fn an_update_that_keeps_being_aborted_gives_up_as_unavailable() {
    let app = "fp-write-abort-exhausted";
    let Some((backend, tenant)) = backend_with_observation("write_abort_exhausted", app).await
    else {
        return;
    };
    let Some(fail_point) =
        FailPoint::enable(app, placement_change_on("delete"), doc! { "times": 100 }).await
    else {
        return;
    };
    let rows_before = index_rows(&backend, "obs-1").await;

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "amended"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = &result else {
        panic!("expected Unavailable, got {result:?}");
    };
    assert!(
        !message.contains("StaleConfig") && !message.contains("13388"),
        "the client-facing message leaves the driver error to the log: {message}"
    );
    assert_eq!(
        entered, WRITE_ABORT_ATTEMPTS,
        "one abort per attempt, then give up"
    );
    let current = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("the resource is still there");
    assert_eq!(current.version_id(), "1", "nothing committed");
    assert_eq!(index_rows(&backend, "obs-1").await, rows_before);
    assert_eq!(history_rows(&backend, "obs-1").await, 1);
}

/// A real lost race keeps its contract: the labelled `WriteConflict` on the
/// resource update is a `VersionConflict` at once, with no second attempt.
///
/// Called through `update`, not `create_or_update`: an unconditional PUT
/// re-reads and retries a lost race by design (its `# Concurrency` contract),
/// so it would hide what this pins — that the write-abort replay does not.
#[tokio::test]
async fn a_lost_race_is_still_a_version_conflict_without_a_second_attempt() {
    let app = "fp-write-abort-lost-race";
    let Some((backend, tenant)) = backend_with_observation("write_abort_lost_race", app).await
    else {
        return;
    };
    let current = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("seeded");
    let Some(fail_point) =
        FailPoint::enable(app, write_conflict_on("update"), doc! { "times": 1 }).await
    else {
        return;
    };

    let result = backend
        .update(&tenant, &current, observation("obs-1", "amended"))
        .await;
    let entered = fail_point.off_and_count().await;

    assert!(
        matches!(
            result,
            Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict { .. }
            ))
        ),
        "{result:?}"
    );
    assert_eq!(entered, 1, "a lost race is not run again");
}

/// A rollback for eviction under cache pressure (365 inside the transaction)
/// is neither a lost race nor run again: the same write meets the same cache
/// (#1895). It ends as `Unavailable` after one attempt, with nothing committed.
#[tokio::test]
async fn an_update_rolled_back_for_eviction_is_unavailable_without_a_second_attempt() {
    let app = "fp-write-abort-eviction";
    let Some((backend, tenant)) = backend_with_observation("write_abort_eviction", app).await
    else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! {
            "failCommands": ["update"],
            "errorCode": 365,
            "errorLabels": ["TransientTransactionError"],
        },
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "amended"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    assert!(
        matches!(
            result,
            Err(StorageError::Backend(BackendError::Unavailable { .. }))
        ),
        "{result:?}"
    );
    assert_eq!(entered, 1, "an eviction rollback is not run again");
    let current = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("the resource is still there");
    assert_eq!(current.version_id(), "1", "nothing committed");
    assert_eq!(history_rows(&backend, "obs-1").await, 1);
}

/// `PUT` of a new id, aborted on the resource insert: the create is run again.
#[tokio::test]
async fn a_create_aborted_by_a_placement_change_is_run_again() {
    let app = "fp-write-abort-create";
    let Some(backend) = create_backend_with_app_name("write_abort_create", app).await else {
        return;
    };
    let tenant = create_tenant("write_abort_create");
    let Some(fail_point) =
        FailPoint::enable(app, placement_change_on("insert"), doc! { "times": 1 }).await
    else {
        return;
    };

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-new",
            observation("obs-new", "final"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let (stored, created) = result.expect("an aborted create is run again");
    assert!(created);
    assert_eq!(stored.version_id(), "1");
    assert_eq!(entered, 1);
    assert_eq!(
        count_docs(&backend, "resources", doc! { "id": "obs-new" }).await,
        1
    );
    assert_eq!(history_rows(&backend, "obs-new").await, 1);
    assert!(index_rows(&backend, "obs-new").await > 0);
}

/// `DELETE`, aborted on the tombstone update: the delete is run again.
#[tokio::test]
async fn a_delete_aborted_by_a_placement_change_is_run_again() {
    let app = "fp-write-abort-delete";
    let Some((backend, tenant)) = backend_with_observation("write_abort_delete", app).await else {
        return;
    };
    let Some(fail_point) =
        FailPoint::enable(app, placement_change_on("update"), doc! { "times": 1 }).await
    else {
        return;
    };

    let result = backend.delete(&tenant, "Observation", "obs-1").await;
    let entered = fail_point.off_and_count().await;

    result.expect("an aborted delete is run again");
    assert_eq!(entered, 1);
    assert!(
        matches!(
            backend.read(&tenant, "Observation", "obs-1").await,
            Err(StorageError::Resource(ResourceError::Gone { .. }))
        ),
        "the resource is deleted"
    );
    assert_eq!(index_rows(&backend, "obs-1").await, 0);
}

/// `PUT` of an existing resource, aborted on the search-index delete by an
/// index build's catalog change (#2045, the `PUT /Patient/p1` failures): the
/// write is run again and commits version 2.
#[tokio::test]
async fn an_update_aborted_on_the_index_delete_by_a_catalog_change_is_run_again() {
    let app = "fp-write-abort-index-delete-246";
    let Some((backend, tenant)) =
        backend_with_observation("write_abort_index_delete_246", app).await
    else {
        return;
    };
    let Some(fail_point) =
        FailPoint::enable(app, snapshot_unavailable_on("delete"), doc! { "times": 1 }).await
    else {
        return;
    };
    let rows_before = index_rows(&backend, "obs-1").await;
    assert!(rows_before > 0);

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "amended"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let (stored, created) = result.expect("an aborted write is run again and commits");
    assert!(!created);
    assert_eq!(stored.version_id(), "2");
    assert_eq!(
        entered, 1,
        "the failpoint fires once; the second attempt is clean"
    );
    assert_eq!(index_rows(&backend, "obs-1").await, rows_before);
    assert_eq!(history_rows(&backend, "obs-1").await, 2);
    assert_status(&backend, &tenant, "obs-1", "amended", "final").await;
}

/// `DELETE`, aborted on the search-index delete by an index build's catalog
/// change (#2045, the `DELETE /Observation/o3` failures): the delete is run
/// again. Its first `delete` command is the `search_index` one; the tombstone
/// is an `update` and the history row an `insert`.
#[tokio::test]
async fn a_delete_aborted_on_the_index_delete_is_run_again() {
    let app = "fp-write-abort-delete-index";
    let Some((backend, tenant)) = backend_with_observation("write_abort_delete_index", app).await
    else {
        return;
    };
    let Some(fail_point) =
        FailPoint::enable(app, snapshot_unavailable_on("delete"), doc! { "times": 1 }).await
    else {
        return;
    };
    assert!(index_rows(&backend, "obs-1").await > 0);

    let result = backend.delete(&tenant, "Observation", "obs-1").await;
    let entered = fail_point.off_and_count().await;

    result.expect("an aborted delete is run again");
    assert_eq!(
        entered, 1,
        "the failpoint fires once; the second attempt is clean"
    );
    assert!(
        matches!(
            backend.read(&tenant, "Observation", "obs-1").await,
            Err(StorageError::Resource(ResourceError::Gone { .. }))
        ),
        "the resource is deleted"
    );
    assert_eq!(index_rows(&backend, "obs-1").await, 0);
    assert_eq!(history_rows(&backend, "obs-1").await, 2);
}

#[tokio::test]
async fn a_restore_aborted_on_the_index_delete_is_run_again() {
    let app = "fp-write-abort-restore";
    let Some((backend, tenant)) = backend_with_observation("write_abort_restore", app).await else {
        return;
    };
    backend
        .delete(&tenant, "Observation", "obs-1")
        .await
        .unwrap();
    let Some(fail_point) =
        FailPoint::enable(app, placement_change_on("delete"), doc! { "times": 1 }).await
    else {
        return;
    };

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "amended"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let (stored, created) = result.expect("an aborted restore is run again");
    assert!(created, "restoring a deleted id is a create outcome");
    assert_eq!(stored.version_id(), "3");
    assert_eq!(entered, 1);
    assert_eq!(history_rows(&backend, "obs-1").await, 3);
    assert_status(&backend, &tenant, "obs-1", "amended", "final").await;
}

#[tokio::test]
async fn a_retried_update_keeps_its_original_version_precondition() {
    let app = "fp-write-abort-stale-version";
    let Some((backend, tenant)) = backend_with_observation("write_abort_stale_version", app).await
    else {
        return;
    };
    let current = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .unwrap();
    let mut failure = placement_change_on("update");
    failure.insert("blockConnection", true);
    failure.insert("blockTimeMS", 4000);
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 1 }).await else {
        return;
    };

    let attempted = backend.update(&tenant, &current, observation("obs-1", "amended"));
    let winner = async {
        fail_point.wait_until_entered(1).await;
        backend
            .update(&tenant, &current, observation("obs-1", "corrected"))
            .await
    };
    let (result, winner) = tokio::join!(attempted, winner);
    let entered = fail_point.off_and_count().await;

    assert_eq!(
        winner
            .expect("the other writer commits while the first is held")
            .version_id(),
        "2"
    );
    assert!(matches!(
        result,
        Err(StorageError::Concurrency(ConcurrencyError::VersionConflict {
            expected_version,
            actual_version,
            ..
        })) if expected_version == "1" && actual_version == "2"
    ));
    assert_eq!(entered, 1);
    assert_eq!(history_rows(&backend, "obs-1").await, 2);
    assert_status(&backend, &tenant, "obs-1", "corrected", "amended").await;
}

/// A search-index step that loses a race to a plain `WriteConflict` (112), the
/// way a writer of `search_index` rows that leaves the resource's version
/// alone (the deferred-rows batch, `$reindex`) makes it lose. The write is run
/// again from its first read (#2269). This used to be the 500 "Failed to
/// delete search index entries".
///
/// `PUT` of an existing resource, 112 on the search-index delete.
#[tokio::test]
async fn an_update_that_lost_a_race_on_its_search_index_delete_is_run_again() {
    let app = "fp-write-conflict-index-delete";
    let Some((backend, tenant)) =
        backend_with_observation("write_conflict_index_delete", app).await
    else {
        return;
    };
    let current = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("seeded");
    let rows_before = index_rows(&backend, "obs-1").await;
    assert!(rows_before > 0);
    let Some(fail_point) =
        FailPoint::enable(app, write_conflict_on("delete"), doc! { "times": 1 }).await
    else {
        return;
    };

    let result = backend
        .update(&tenant, &current, observation("obs-1", "amended"))
        .await;
    let entered = fail_point.off_and_count().await;

    let stored = result.expect("a 112 in the search-index step is run again, not a 500");
    assert_eq!(stored.version_id(), "2");
    assert_eq!(
        entered, 1,
        "the failpoint fires once; the second attempt is clean"
    );
    assert_eq!(index_rows(&backend, "obs-1").await, rows_before);
    assert_eq!(history_rows(&backend, "obs-1").await, 2);
    assert_status(&backend, &tenant, "obs-1", "amended", "final").await;
}

/// The same 112 on the search-index insert, scoped to the `search_index`
/// namespace so the history insert before it is not the one that fails.
#[tokio::test]
async fn an_update_that_lost_a_race_on_its_search_index_insert_is_run_again() {
    let app = "fp-write-conflict-index-insert";
    let Some((backend, tenant)) =
        backend_with_observation("write_conflict_index_insert", app).await
    else {
        return;
    };
    let current = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("seeded");
    let namespace = format!("{}.search_index", backend.config().database_name);
    let Some(fail_point) = FailPoint::enable(
        app,
        write_conflict_on_namespace("insert", &namespace),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .update(&tenant, &current, observation("obs-1", "amended"))
        .await;
    let entered = fail_point.off_and_count().await;

    let stored = result.expect("a 112 on the search-index insert is run again");
    assert_eq!(stored.version_id(), "2");
    assert_eq!(entered, 1);
    assert_eq!(history_rows(&backend, "obs-1").await, 2);
    assert_status(&backend, &tenant, "obs-1", "amended", "final").await;
}

/// A create of a new id, 112 on the search-index delete that precedes its
/// insert: the create is run again and stores version 1 once.
#[tokio::test]
async fn a_create_that_lost_a_race_on_its_search_index_delete_is_run_again() {
    let app = "fp-write-conflict-create";
    let Some(backend) = create_backend_with_app_name("write_conflict_create", app).await else {
        return;
    };
    let tenant = create_tenant("write_conflict_create");
    let Some(fail_point) =
        FailPoint::enable(app, write_conflict_on("delete"), doc! { "times": 1 }).await
    else {
        return;
    };

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-new",
            observation("obs-new", "final"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let (stored, created) = result.expect("a create whose index step lost a race is run again");
    assert!(created);
    assert_eq!(stored.version_id(), "1");
    assert_eq!(entered, 1);
    assert_eq!(
        count_docs(&backend, "resources", doc! { "id": "obs-new" }).await,
        1
    );
    assert_eq!(history_rows(&backend, "obs-new").await, 1);
    assert!(index_rows(&backend, "obs-new").await > 0);
}

/// `DELETE`, 112 on the search-index delete: the delete is run again.
#[tokio::test]
async fn a_delete_that_lost_a_race_on_its_search_index_delete_is_run_again() {
    let app = "fp-write-conflict-delete";
    let Some((backend, tenant)) = backend_with_observation("write_conflict_delete", app).await
    else {
        return;
    };
    let Some(fail_point) =
        FailPoint::enable(app, write_conflict_on("delete"), doc! { "times": 1 }).await
    else {
        return;
    };

    let result = backend.delete(&tenant, "Observation", "obs-1").await;
    let entered = fail_point.off_and_count().await;

    result.expect("a delete whose index step lost a race is run again");
    assert_eq!(entered, 1);
    assert!(
        matches!(
            backend.read(&tenant, "Observation", "obs-1").await,
            Err(StorageError::Resource(ResourceError::Gone { .. }))
        ),
        "the resource is deleted"
    );
    assert_eq!(index_rows(&backend, "obs-1").await, 0);
    assert_eq!(history_rows(&backend, "obs-1").await, 2);
}

/// Restoring a deleted id, 112 on the search-index delete that precedes the
/// rebuild: the restore is run again.
#[tokio::test]
async fn a_restore_that_lost_a_race_on_its_search_index_delete_is_run_again() {
    let app = "fp-write-conflict-restore";
    let Some((backend, tenant)) = backend_with_observation("write_conflict_restore", app).await
    else {
        return;
    };
    backend
        .delete(&tenant, "Observation", "obs-1")
        .await
        .unwrap();
    let Some(fail_point) =
        FailPoint::enable(app, write_conflict_on("delete"), doc! { "times": 1 }).await
    else {
        return;
    };

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "amended"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let (stored, created) = result.expect("a restore whose index step lost a race is run again");
    assert!(created, "restoring a deleted id is a create outcome");
    assert_eq!(stored.version_id(), "3");
    assert_eq!(entered, 1);
    assert_eq!(history_rows(&backend, "obs-1").await, 3);
    assert_status(&backend, &tenant, "obs-1", "amended", "final").await;
}

/// A search-index conflict that never clears gives up as `Unavailable` (a
/// retryable 503) after the policy's attempts, with nothing committed and none
/// of the driver text in the client-facing message.
#[tokio::test]
async fn a_search_index_write_conflict_that_persists_gives_up_as_unavailable() {
    let app = "fp-write-conflict-exhausted";
    let Some((backend, tenant)) = backend_with_observation("write_conflict_exhausted", app).await
    else {
        return;
    };
    let current = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("seeded");
    let rows_before = index_rows(&backend, "obs-1").await;
    let Some(fail_point) =
        FailPoint::enable(app, write_conflict_on("delete"), doc! { "times": 100 }).await
    else {
        return;
    };

    let result = backend
        .update(&tenant, &current, observation("obs-1", "amended"))
        .await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = &result else {
        panic!("expected Unavailable, got {result:?}");
    };
    assert!(
        !message.contains("112") && !message.contains("WriteConflict"),
        "the client-facing message leaves the driver error to the log: {message}"
    );
    assert_eq!(
        entered, WRITE_ABORT_ATTEMPTS,
        "one conflict per attempt, then give up"
    );
    let current = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("the resource is still there");
    assert_eq!(current.version_id(), "1", "nothing committed");
    assert_eq!(index_rows(&backend, "obs-1").await, rows_before);
    assert_eq!(history_rows(&backend, "obs-1").await, 1);
}

/// A commit that applied and then reported a write-concern failure is sent
/// again, and the update is stored once (#2270).
#[tokio::test]
async fn an_unknown_update_commit_is_re_sent_and_the_update_applies_once() {
    let app = "fp-write-unknown-update";
    let Some((backend, tenant)) = backend_with_observation("write_unknown_update", app).await
    else {
        return;
    };
    let current = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("seeded");
    let Some(fail_point) =
        FailPoint::enable(app, commit_with_unknown_result(), doc! { "times": 1 }).await
    else {
        return;
    };

    let result = backend
        .update(&tenant, &current, observation("obs-1", "amended"))
        .await;
    let entered = fail_point.off_and_count().await;

    let stored = result.expect("an unknown commit is re-sent, not reported as a failure");
    assert_eq!(stored.version_id(), "2");
    assert_eq!(entered, 1, "the failpoint fires on the first commit only");
    assert_eq!(
        history_rows(&backend, "obs-1").await,
        2,
        "the write was not run a second time"
    );
    assert_status(&backend, &tenant, "obs-1", "amended", "final").await;
}

/// A commit whose outcome never becomes known is sent three times in all (the
/// first plus the Bundle path's two re-sends). The write is not run again: it
/// was applied once. The history row is then found by a majority read, so the
/// update is answered as committed (#2304); it used to be reported as unknown.
#[tokio::test]
async fn an_update_commit_that_stays_unknown_but_applied_is_confirmed_by_a_read() {
    let app = "fp-write-unknown-update-exhausted";
    let Some((backend, tenant)) =
        backend_with_observation("write_unknown_update_exhausted", app).await
    else {
        return;
    };
    let current = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("seeded");
    let Some(fail_point) =
        FailPoint::enable(app, commit_with_unknown_result(), doc! { "times": 100 }).await
    else {
        return;
    };

    let result = backend
        .update(&tenant, &current, observation("obs-1", "amended"))
        .await;
    let entered = fail_point.off_and_count().await;

    let stored = result.expect("the read finds the version the commit stored");
    assert_eq!(stored.version_id(), "2");
    assert_eq!(
        entered, COMMITS_WHILE_UNKNOWN,
        "the commit is sent three times, and only the commit"
    );
    assert_eq!(
        history_rows(&backend, "obs-1").await,
        2,
        "the write was not run again"
    );
    assert_status(&backend, &tenant, "obs-1", "amended", "final").await;
}

/// An update whose commit the server refuses with the unknown-result label and
/// applies nothing: the settling read misses the row, so the answer stays
/// `CommitOutcomeUnknown` (#2304). The read runs outside the write's session:
/// inside it, it would see the uncommitted version 2.
#[tokio::test]
async fn an_update_commit_that_never_applied_stays_unknown() {
    let app = "fp-2304-update-never-applied";
    let Some((backend, tenant)) = backend_with_observation("wcu_update_never_applied", app).await
    else {
        return;
    };
    let current = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("seeded");
    let Some(fail_point) = FailPoint::enable(
        app,
        commit_refused_with_unknown_result(),
        doc! { "times": 100 },
    )
    .await
    else {
        return;
    };

    let started = std::time::Instant::now();
    let result = backend
        .update(&tenant, &current, observation("obs-1", "amended"))
        .await;
    let elapsed = started.elapsed();
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Transaction(TransactionError::CommitOutcomeUnknown { reason })) = &result
    else {
        panic!("expected CommitOutcomeUnknown, got {result:?}");
    };
    assert_did_not_wait_out_the_settle_bound(elapsed);
    assert!(!reason.is_empty());
    assert_eq!(entered, COMMITS_WHILE_UNKNOWN);
    let stored = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("the resource is still there");
    assert_eq!(stored.version_id(), "1", "nothing was applied");
    assert_eq!(history_rows(&backend, "obs-1").await, 1);
    assert_status(&backend, &tenant, "obs-1", "final", "amended").await;
}

/// A create whose commit never becomes known: the commit is sent again, the
/// create is not run again, and the version it stored is found by a majority
/// read, so the create is answered as committed (#2304). (Before #2270 this was
/// a generic `Internal` answer; before #2304 it was `CommitOutcomeUnknown`.)
#[tokio::test]
async fn a_create_commit_that_stays_unknown_but_applied_is_confirmed_by_a_read() {
    let app = "fp-write-abort-unknown-commit";
    let Some(backend) = create_backend_with_app_name("write_abort_unknown_commit", app).await
    else {
        return;
    };
    let tenant = create_tenant("write_abort_unknown_commit");
    let Some(fail_point) =
        FailPoint::enable(app, commit_with_unknown_result(), doc! { "times": 100 }).await
    else {
        return;
    };

    let result = backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "status": "final",
                "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
            }),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let stored = result.expect("the read finds the version the commit stored");
    assert_eq!(stored.version_id(), "1");
    assert_eq!(entered, COMMITS_WHILE_UNKNOWN);
    assert_eq!(tenant_rows(&backend, &tenant).await, (1, 1));
    assert!(
        backend
            .read(&tenant, "Observation", stored.id())
            .await
            .unwrap()
            .is_some()
    );
}

/// A create whose commit the server refuses with the unknown-result label and
/// applies nothing: the settling read misses the row, so the answer stays
/// `CommitOutcomeUnknown` and nothing is stored (#2304).
#[tokio::test]
async fn a_create_commit_that_never_applied_stays_unknown() {
    let app = "fp-2304-create-never-applied";
    let Some(backend) = create_backend_with_app_name("wcu_create_never_applied", app).await else {
        return;
    };
    let tenant = create_tenant("wcu_create_never_applied");
    let Some(fail_point) = FailPoint::enable(
        app,
        commit_refused_with_unknown_result(),
        doc! { "times": 100 },
    )
    .await
    else {
        return;
    };

    let started = std::time::Instant::now();
    let result = backend
        .create(
            &tenant,
            "Observation",
            new_observation("final"),
            FhirVersion::default(),
        )
        .await;
    let elapsed = started.elapsed();
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Transaction(TransactionError::CommitOutcomeUnknown { reason })) = &result
    else {
        panic!("expected CommitOutcomeUnknown, got {result:?}");
    };
    assert_did_not_wait_out_the_settle_bound(elapsed);
    assert!(!reason.is_empty());
    assert_eq!(entered, COMMITS_WHILE_UNKNOWN);
    assert_eq!(tenant_rows(&backend, &tenant).await, (0, 0));
}

/// A create whose first commit is unknown and whose re-sent commit succeeds
/// returns the stored resource, once.
#[tokio::test]
async fn an_unknown_create_commit_is_re_sent_and_the_create_applies_once() {
    let app = "fp-write-unknown-create";
    let Some(backend) = create_backend_with_app_name("write_unknown_create", app).await else {
        return;
    };
    let tenant = create_tenant("write_unknown_create");
    let Some(fail_point) =
        FailPoint::enable(app, commit_with_unknown_result(), doc! { "times": 1 }).await
    else {
        return;
    };

    let result = backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "status": "final",
                "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
            }),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let stored = result.expect("an unknown commit is re-sent, not reported as a failure");
    assert_eq!(stored.version_id(), "1");
    assert_eq!(entered, 1);
    let scope = doc! {"tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation"};
    assert_eq!(count_docs(&backend, "resources", scope.clone()).await, 1);
    assert_eq!(count_docs(&backend, "resource_history", scope).await, 1);
}

/// A delete whose first commit is unknown and whose re-sent commit succeeds is
/// stored once: the resource is gone and has one tombstone version.
#[tokio::test]
async fn an_unknown_delete_commit_is_re_sent_and_the_delete_applies_once() {
    let app = "fp-write-unknown-delete";
    let Some((backend, tenant)) = backend_with_observation("write_unknown_delete", app).await
    else {
        return;
    };
    let Some(fail_point) =
        FailPoint::enable(app, commit_with_unknown_result(), doc! { "times": 1 }).await
    else {
        return;
    };

    let result = backend.delete(&tenant, "Observation", "obs-1").await;
    let entered = fail_point.off_and_count().await;

    result.expect("an unknown commit is re-sent, not reported as a failure");
    assert_eq!(entered, 1);
    assert!(
        matches!(
            backend.read(&tenant, "Observation", "obs-1").await,
            Err(StorageError::Resource(ResourceError::Gone { .. }))
        ),
        "the resource is deleted"
    );
    assert_eq!(history_rows(&backend, "obs-1").await, 2);
    assert_eq!(index_rows(&backend, "obs-1").await, 0);
}

/// A delete whose commit never becomes known but applied: the tombstone is
/// found by a majority read, so the delete is answered as committed (#2304).
#[tokio::test]
async fn a_delete_commit_that_stays_unknown_but_applied_is_confirmed_by_a_read() {
    let app = "fp-2304-delete-applied";
    let Some((backend, tenant)) = backend_with_observation("wcu_delete_applied", app).await else {
        return;
    };
    let Some(fail_point) =
        FailPoint::enable(app, commit_with_unknown_result(), doc! { "times": 100 }).await
    else {
        return;
    };

    let result = backend.delete(&tenant, "Observation", "obs-1").await;
    let entered = fail_point.off_and_count().await;

    result.expect("the read finds the tombstone the commit stored");
    assert_eq!(entered, COMMITS_WHILE_UNKNOWN);
    assert!(
        matches!(
            backend.read(&tenant, "Observation", "obs-1").await,
            Err(StorageError::Resource(ResourceError::Gone { .. }))
        ),
        "the resource is deleted"
    );
    assert_eq!(history_rows(&backend, "obs-1").await, 2);
    assert_eq!(index_rows(&backend, "obs-1").await, 0);
}

/// A restore (a `PUT` onto a deleted id) whose commit never becomes known but
/// applied: the restored version is found by a majority read (#2304). The
/// tombstone is version 2, the restore version 3.
#[tokio::test]
async fn a_restore_commit_that_stays_unknown_but_applied_is_confirmed_by_a_read() {
    let app = "fp-2304-restore-applied";
    let Some((backend, tenant)) =
        backend_with_deleted_observation("wcu_restore_applied", app).await
    else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        applied_commit_with_unknown_result(),
        doc! { "times": 100 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "amended"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let (stored, created) = result.expect("the read finds the version the commit stored");
    assert!(created, "restoring a deleted id is a create outcome");
    assert_eq!(stored.version_id(), "3");
    assert_eq!(entered, DELAYED_COMMITS_WHILE_UNKNOWN);
    assert_eq!(history_rows(&backend, "obs-1").await, 3);
    assert_status(&backend, &tenant, "obs-1", "amended", "final").await;
}

/// A SearchParameter create whose commit never becomes known but applied: the
/// create is answered as committed and the parameter is registered, by the
/// create's own reload (#2304).
#[tokio::test]
async fn a_search_parameter_create_whose_unknown_commit_applied_is_confirmed_and_registered() {
    let app = "fp-2304-sp-create-applied";
    let Some(backend) = create_backend_with_app_name("wcu_sp_create_applied", app).await else {
        return;
    };
    let tenant = create_tenant("wcu-sp-create-applied");
    let Some(fail_point) =
        FailPoint::enable(app, commit_with_unknown_result(), doc! { "times": 100 }).await
    else {
        return;
    };

    let result = backend
        .create(
            &tenant,
            "SearchParameter",
            search_parameter("wcu-nickname-sp", "wcu-nickname"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    result.expect("the read finds the version the commit stored");
    assert_eq!(entered, COMMITS_WHILE_UNKNOWN);
    assert!(
        backend
            .search_param_registry(&tenant)
            .read()
            .get_param("Patient", "wcu-nickname")
            .is_some(),
        "the confirmed create reloads the SearchParameter cache"
    );
}

/// A SearchParameter create whose commit stays unknown (the server refuses it
/// and applies nothing) still reloads the tenant's SearchParameter cache, so a
/// write that did commit would take effect for search (#2304). The reload reads
/// storage: a parameter a cluster-mate wrote enters this backend's registry, and
/// the create's own parameter, which never applied, does not.
#[tokio::test]
async fn a_search_parameter_create_whose_commit_stays_unknown_still_reloads_the_cache() {
    let app = "fp-2304-sp-create-unknown";
    let Some(backend) = create_backend_with_app_name("wcu_sp_create_unknown", app).await else {
        return;
    };
    let tenant = create_tenant("wcu-sp-create-unknown");

    // A cluster-mate on the same database writes a SearchParameter this
    // backend's cache does not know about, as after a write that did commit.
    let Some(mate) = build_backend(MongoBackendConfig {
        app_name: Some(format!("{app}-mate")),
        ..backend.config().clone()
    })
    .await
    else {
        return;
    };
    mate.create(
        &tenant,
        "SearchParameter",
        search_parameter("wcu-mate-sp", "mate-nickname"),
        FhirVersion::default(),
    )
    .await
    .expect("the cluster-mate's SearchParameter");
    assert!(
        backend
            .search_param_registry(&tenant)
            .read()
            .get_param("Patient", "mate-nickname")
            .is_none(),
        "this backend's cache is stale"
    );

    let Some(fail_point) = FailPoint::enable(
        app,
        commit_refused_with_unknown_result(),
        doc! { "times": 100 },
    )
    .await
    else {
        return;
    };
    let started = std::time::Instant::now();
    let result = backend
        .create(
            &tenant,
            "SearchParameter",
            search_parameter("wcu-own-sp", "own-nickname"),
            FhirVersion::default(),
        )
        .await;
    let elapsed = started.elapsed();
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Transaction(TransactionError::CommitOutcomeUnknown { .. })) = &result
    else {
        panic!("expected CommitOutcomeUnknown, got {result:?}");
    };
    assert_did_not_wait_out_the_settle_bound(elapsed);
    assert_eq!(entered, COMMITS_WHILE_UNKNOWN);
    let registry = backend.search_param_registry(&tenant);
    let registry = registry.read();
    assert!(
        registry.get_param("Patient", "mate-nickname").is_some(),
        "the unknown commit reloaded the cache from storage"
    );
    assert!(
        registry.get_param("Patient", "own-nickname").is_none(),
        "the create's commit never applied"
    );
}

/// A create (`POST`, a server-assigned id) whose `resources` insert met a plain
/// `WriteConflict` (112) is run again and stores version 1 once (#2300). This
/// used to be a 500 "Failed to insert resource (session)".
#[tokio::test]
async fn a_create_that_hit_a_write_conflict_on_its_resource_insert_is_run_again() {
    let app = "fp-2300-create-resource-insert";
    let Some(backend) = create_backend_with_app_name("wc_create_resource_insert", app).await else {
        return;
    };
    let tenant = create_tenant("wc_create_resource_insert");
    let failure = write_conflict_on_insert_into(&backend, "resources");
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 1 }).await else {
        return;
    };

    let result = backend
        .create(
            &tenant,
            "Observation",
            new_observation("final"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let stored = result.expect("a 112 on the resource insert is run again, not a 500");
    assert_eq!(stored.version_id(), "1");
    assert_eq!(
        entered, 1,
        "the failpoint fires once; the second attempt is clean"
    );
    assert_eq!(tenant_rows(&backend, &tenant).await, (1, 1));
    assert_status(&backend, &tenant, stored.id(), "final", "amended").await;
}

/// The same 112 on the create's `resource_history` insert, scoped to that
/// namespace so the resource insert before it is not the one that fails.
#[tokio::test]
async fn a_create_that_hit_a_write_conflict_on_its_history_insert_is_run_again() {
    let app = "fp-2300-create-history-insert";
    let Some(backend) = create_backend_with_app_name("wc_create_history_insert", app).await else {
        return;
    };
    let tenant = create_tenant("wc_create_history_insert");
    let failure = write_conflict_on_insert_into(&backend, "resource_history");
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 1 }).await else {
        return;
    };

    let result = backend
        .create(
            &tenant,
            "Observation",
            new_observation("final"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let stored = result.expect("a 112 on the history insert is run again, not a 500");
    assert_eq!(stored.version_id(), "1");
    assert_eq!(entered, 1);
    assert_eq!(tenant_rows(&backend, &tenant).await, (1, 1));
    assert_status(&backend, &tenant, stored.id(), "final", "amended").await;
}

/// The same 112 at the create's commit, after every statement ran: nothing
/// committed, so the create is run again from its first read.
///
/// A failpoint refuses the commit before the server does anything, so the
/// server transaction stays open, holding the new id. A real 112 at commit has
/// already aborted it, so this is a failpoint artifact. The driver (mongodb
/// 3.7.0) marks the transaction committed before it sends `commitTransaction`,
/// so dropping the session after the refusal checks the server session back
/// into the pool without an `abortTransaction`. The open transaction ends when
/// a later transaction starts on that same server session. If the rerun gets a
/// different server session, every attempt meets the open transaction's insert
/// as a real 112, and the create gives up as `Unavailable`.
/// `begin_single_write_session` starts its session before the `hello` so that
/// the rerun takes that server session (#2305; see the note there). The create
/// seeded below was added on a base without that order, where a first write on
/// a fresh database sometimes missed the reuse (3 of 40 looped runs without the
/// seed, 0 of 60 with it); it stays as a guard.
/// `a_restore_that_hit_a_write_conflict_at_its_commit_is_run_again` and
/// `an_update_aborted_at_commit_is_run_again` rely on the same reuse, and their
/// fixtures' earlier writes play the seed's part.
#[tokio::test]
async fn a_create_that_hit_a_write_conflict_at_its_commit_is_run_again() {
    let app = "fp-2300-create-commit";
    let Some(backend) = create_backend_with_app_name("wc_create_commit", app).await else {
        return;
    };
    let tenant = create_tenant("wc_create_commit");
    backend
        .create(
            &tenant,
            "Observation",
            new_observation("preliminary"),
            FhirVersion::default(),
        )
        .await
        .expect("seeding a first write");
    let Some(fail_point) = FailPoint::enable(
        app,
        write_conflict_on("commitTransaction"),
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
            new_observation("final"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let stored = result.expect("a 112 at the commit is run again, not a 500");
    assert_eq!(stored.version_id(), "1");
    assert_eq!(entered, 1);
    assert_eq!(tenant_rows(&backend, &tenant).await, (2, 2));
    assert_status(&backend, &tenant, stored.id(), "final", "amended").await;
}

/// `PUT` of a new id: the create branch writes the path id, and a 112 on its
/// resource insert is run again.
#[tokio::test]
async fn a_put_create_that_hit_a_write_conflict_on_its_resource_insert_is_run_again() {
    let app = "fp-2300-put-create-resource-insert";
    let Some(backend) = create_backend_with_app_name("wc_put_create_resource_insert", app).await
    else {
        return;
    };
    let tenant = create_tenant("wc_put_create_resource_insert");
    let failure = write_conflict_on_insert_into(&backend, "resources");
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 1 }).await else {
        return;
    };

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-new",
            observation("obs-new", "final"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let (stored, created) = result.expect("a 112 on the resource insert is run again, not a 500");
    assert!(created);
    assert_eq!(stored.version_id(), "1");
    assert_eq!(entered, 1);
    assert_eq!(
        count_docs(&backend, "resources", doc! { "id": "obs-new" }).await,
        1
    );
    assert_eq!(history_rows(&backend, "obs-new").await, 1);
    assert_status(&backend, &tenant, "obs-new", "final", "amended").await;
}

/// Restoring a deleted id, 112 on its `resource_history` insert: the restore
/// holds the resource document by then, so the conflict names no winner and the
/// restore is run again. The tombstone is version 2, the restore version 3.
#[tokio::test]
async fn a_restore_that_hit_a_write_conflict_on_its_history_insert_is_run_again() {
    let app = "fp-2300-restore-history-insert";
    let Some((backend, tenant)) =
        backend_with_deleted_observation("wc_restore_history_insert", app).await
    else {
        return;
    };
    let failure = write_conflict_on_insert_into(&backend, "resource_history");
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 1 }).await else {
        return;
    };

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "amended"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let (stored, created) = result.expect("a 112 on the restore's history insert is run again");
    assert!(created, "restoring a deleted id is a create outcome");
    assert_eq!(stored.version_id(), "3");
    assert_eq!(entered, 1);
    assert_eq!(history_rows(&backend, "obs-1").await, 3);
    assert_status(&backend, &tenant, "obs-1", "amended", "final").await;
}

/// Restoring a deleted id, 112 at the commit: the restore is run again.
/// It relies on the same server-session reuse as
/// `a_create_that_hit_a_write_conflict_at_its_commit_is_run_again`; the create
/// and delete in `backend_with_deleted_observation` play the part of that
/// test's seeded create.
#[tokio::test]
async fn a_restore_that_hit_a_write_conflict_at_its_commit_is_run_again() {
    let app = "fp-2300-restore-commit";
    let Some((backend, tenant)) = backend_with_deleted_observation("wc_restore_commit", app).await
    else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        write_conflict_on("commitTransaction"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "amended"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let (stored, created) = result.expect("a 112 at the restore's commit is run again");
    assert!(created, "restoring a deleted id is a create outcome");
    assert_eq!(stored.version_id(), "3");
    assert_eq!(entered, 1);
    assert_eq!(history_rows(&backend, "obs-1").await, 3);
    assert_status(&backend, &tenant, "obs-1", "amended", "final").await;
}

/// A create whose resource insert keeps meeting a 112 gives up as `Unavailable`
/// (a retryable 503) after the policy's attempts, with nothing stored and none
/// of the driver text in the client-facing message.
#[tokio::test]
async fn a_create_that_keeps_hitting_a_write_conflict_gives_up_as_unavailable() {
    let app = "fp-2300-create-exhausted";
    let Some(backend) = create_backend_with_app_name("wc_create_exhausted", app).await else {
        return;
    };
    let tenant = create_tenant("wc_create_exhausted");
    let failure = write_conflict_on_insert_into(&backend, "resources");
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 100 }).await else {
        return;
    };

    let result = backend
        .create(
            &tenant,
            "Observation",
            new_observation("final"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    assert_unavailable_without_driver_text(&result);
    assert_eq!(
        entered, WRITE_ABORT_ATTEMPTS,
        "one conflict per attempt, then give up"
    );
    assert_eq!(tenant_rows(&backend, &tenant).await, (0, 0));
}

/// A `PUT` to a new id whose history insert keeps meeting a 112 gives up as
/// `Unavailable`, with nothing stored.
#[tokio::test]
async fn a_put_create_that_keeps_hitting_a_write_conflict_gives_up_as_unavailable() {
    let app = "fp-2300-put-create-exhausted";
    let Some(backend) = create_backend_with_app_name("wc_put_create_exhausted", app).await else {
        return;
    };
    let tenant = create_tenant("wc_put_create_exhausted");
    let failure = write_conflict_on_insert_into(&backend, "resource_history");
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 100 }).await else {
        return;
    };

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-new",
            observation("obs-new", "final"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    assert_unavailable_without_driver_text(&result);
    assert_eq!(entered, WRITE_ABORT_ATTEMPTS);
    assert_eq!(
        count_docs(&backend, "resources", doc! { "id": "obs-new" }).await,
        0
    );
    assert_eq!(history_rows(&backend, "obs-new").await, 0);
}

/// A restore whose history insert keeps meeting a 112 gives up as `Unavailable`:
/// the resource stays deleted, with its tombstone and no search-index rows.
#[tokio::test]
async fn a_restore_that_keeps_hitting_a_write_conflict_gives_up_as_unavailable() {
    let app = "fp-2300-restore-exhausted";
    let Some((backend, tenant)) =
        backend_with_deleted_observation("wc_restore_exhausted", app).await
    else {
        return;
    };
    let failure = write_conflict_on_insert_into(&backend, "resource_history");
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 100 }).await else {
        return;
    };

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "amended"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    assert_unavailable_without_driver_text(&result);
    assert_eq!(entered, WRITE_ABORT_ATTEMPTS);
    assert!(
        matches!(
            backend.read(&tenant, "Observation", "obs-1").await,
            Err(StorageError::Resource(ResourceError::Gone { .. }))
        ),
        "the resource is still deleted"
    );
    assert_eq!(history_rows(&backend, "obs-1").await, 2);
    assert_eq!(index_rows(&backend, "obs-1").await, 0);
}

/// A real winner of the same id: a create's resource insert is held and fails
/// with a 112 while another create of that id commits. The rerun's existence
/// check finds the winner, so the answer is `AlreadyExists` (a 409), not a 500
/// and not a second copy.
#[tokio::test]
async fn a_create_whose_id_another_create_won_after_a_write_conflict_already_exists() {
    let app = "fp-2300-create-race";
    let Some(backend) = create_backend_with_app_name("wc_create_race", app).await else {
        return;
    };
    let tenant = create_tenant("wc_create_race");
    let mut failure = write_conflict_on_insert_into(&backend, "resources");
    failure.insert("blockConnection", true);
    failure.insert("blockTimeMS", 4000);
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 1 }).await else {
        return;
    };

    let attempted = backend.create(
        &tenant,
        "Observation",
        observation("obs-race", "amended"),
        FhirVersion::default(),
    );
    let winner = async {
        fail_point.wait_until_entered(1).await;
        backend
            .create(
                &tenant,
                "Observation",
                observation("obs-race", "final"),
                FhirVersion::default(),
            )
            .await
    };
    let (result, winner) = tokio::join!(attempted, winner);
    let entered = fail_point.off_and_count().await;

    assert_eq!(
        winner
            .expect("the other create commits while the first is held")
            .version_id(),
        "1"
    );
    assert!(
        matches!(
            result,
            Err(StorageError::Resource(ResourceError::AlreadyExists { .. }))
        ),
        "{result:?}"
    );
    assert_eq!(entered, 1);
    assert_eq!(
        count_docs(&backend, "resources", doc! { "id": "obs-race" }).await,
        1
    );
    assert_eq!(history_rows(&backend, "obs-race").await, 1);
    assert_status(&backend, &tenant, "obs-race", "final", "amended").await;
}

/// The same race through `PUT` to a new id: the rerun's existence check answers
/// `AlreadyExists`, `create_or_update` reads again, finds the winner and updates
/// it, so the loser's content lands as version 2.
#[tokio::test]
async fn a_put_whose_new_id_another_put_won_after_a_write_conflict_updates_it() {
    let app = "fp-2300-put-race";
    let Some(backend) = create_backend_with_app_name("wc_put_race", app).await else {
        return;
    };
    let tenant = create_tenant("wc_put_race");
    let mut failure = write_conflict_on_insert_into(&backend, "resources");
    failure.insert("blockConnection", true);
    failure.insert("blockTimeMS", 4000);
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 1 }).await else {
        return;
    };

    let attempted = backend.create_or_update(
        &tenant,
        "Observation",
        "obs-race",
        observation("obs-race", "amended"),
        FhirVersion::default(),
    );
    let winner = async {
        fail_point.wait_until_entered(1).await;
        backend
            .create_or_update(
                &tenant,
                "Observation",
                "obs-race",
                observation("obs-race", "final"),
                FhirVersion::default(),
            )
            .await
    };
    let (result, winner) = tokio::join!(attempted, winner);
    let entered = fail_point.off_and_count().await;

    let (winner, winner_created) = winner.expect("the other PUT commits while the first is held");
    assert!(winner_created);
    assert_eq!(winner.version_id(), "1");
    let (stored, created) = result.expect("the loser reads the winner again and updates it");
    assert!(!created);
    assert_eq!(stored.version_id(), "2");
    assert_eq!(entered, 1);
    assert_eq!(history_rows(&backend, "obs-race").await, 2);
    assert_status(&backend, &tenant, "obs-race", "amended", "final").await;
}

// ---------------------------------------------------------------------------
// #2342: a plain WriteConflict on an update's or a delete's history insert or
// commit. The write holds the resource document by then (its compare-and-swap
// ran first in the same transaction), so the 112 names no winner: it is run
// again and the rerun's version check decides.
// ---------------------------------------------------------------------------

/// `obs-1` was updated once, to "amended": two history rows and the new token in
/// the index, the old one gone.
async fn assert_updated_once(backend: &MongoBackend, tenant: &TenantContext) {
    assert_eq!(history_rows(backend, "obs-1").await, 2);
    assert_status(backend, tenant, "obs-1", "amended", "final").await;
}

/// `obs-1` was deleted once: it reads as gone, its tombstone is the second
/// history row, and no search-index row is left.
async fn assert_deleted_once(backend: &MongoBackend, tenant: &TenantContext) {
    assert!(
        matches!(
            backend.read(tenant, "Observation", "obs-1").await,
            Err(StorageError::Resource(ResourceError::Gone { .. }))
        ),
        "the resource is deleted"
    );
    assert_eq!(history_rows(backend, "obs-1").await, 2);
    assert_eq!(index_rows(backend, "obs-1").await, 0);
}

/// A write that gave up committed nothing: `obs-1` is still version 1 with its
/// one history row, its status "final" and the index rows it had before.
async fn assert_untouched(backend: &MongoBackend, tenant: &TenantContext, rows_before: u64) {
    let current = backend
        .read(tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("the resource is still there");
    assert_eq!(current.version_id(), "1", "nothing committed");
    assert_eq!(history_rows(backend, "obs-1").await, 1);
    assert_eq!(index_rows(backend, "obs-1").await, rows_before);
    assert_status(backend, tenant, "obs-1", "final", "amended").await;
}

/// `If-Match` update (`update` with the version read before), 112 on its
/// `resource_history` insert, scoped to that namespace so the compare-and-swap
/// on `resources` before it is not the one that fails. The update holds the
/// resource document by then, so the conflict names no winner and the update is
/// run again. It used to answer 409 "expected version 1, but found unknown" for
/// a resource that had not moved.
#[tokio::test]
async fn an_if_match_update_that_met_a_write_conflict_on_its_history_row_is_run_again() {
    let app = "fp-2342-update-history";
    let Some((backend, tenant)) = backend_with_observation("wc2342_update_history", app).await
    else {
        return;
    };
    let current = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("seeded");
    let failure = write_conflict_on_insert_into(&backend, "resource_history");
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 1 }).await else {
        return;
    };

    let result = backend
        .update(&tenant, &current, observation("obs-1", "amended"))
        .await;
    let entered = fail_point.off_and_count().await;

    let stored = result.expect("a 112 on the history row is run again, not a 409");
    assert_eq!(stored.version_id(), "2");
    assert_eq!(
        entered, 1,
        "the failpoint fires once; the second attempt is clean"
    );
    assert_updated_once(&backend, &tenant).await;
}

/// The same 112 at the `If-Match` update's commit, after every statement ran:
/// nothing committed, so the update is run again from its first read.
///
/// It relies on the same server-session reuse as
/// `a_create_that_hit_a_write_conflict_at_its_commit_is_run_again`: a failpoint
/// refuses the commit before the server acts, so the server transaction stays
/// open holding the resource document until the rerun's transaction starts on
/// that same server session. The seeded `create_or_update` in
/// `backend_with_observation` plays the part of that test's seeded create.
#[tokio::test]
async fn an_if_match_update_that_met_a_write_conflict_at_its_commit_is_run_again() {
    let app = "fp-2342-update-commit";
    let Some((backend, tenant)) = backend_with_observation("wc2342_update_commit", app).await
    else {
        return;
    };
    let current = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("seeded");
    let Some(fail_point) = FailPoint::enable(
        app,
        write_conflict_on("commitTransaction"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .update(&tenant, &current, observation("obs-1", "amended"))
        .await;
    let entered = fail_point.off_and_count().await;

    let stored = result.expect("a 112 at the commit is run again, not a 409");
    assert_eq!(stored.version_id(), "2");
    assert_eq!(entered, 1);
    assert_updated_once(&backend, &tenant).await;
}

/// Versioned delete (`delete_versioned`, an `If-Match` `DELETE`), 112 on the
/// tombstone's `resource_history` insert: the delete holds the resource
/// document by then, so it is run again and its version check passes.
#[tokio::test]
async fn a_versioned_delete_that_met_a_write_conflict_on_its_history_row_is_run_again() {
    let app = "fp-2342-delete-history";
    let Some((backend, tenant)) = backend_with_observation("wc2342_delete_history", app).await
    else {
        return;
    };
    let failure = write_conflict_on_insert_into(&backend, "resource_history");
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 1 }).await else {
        return;
    };

    let result = backend
        .delete_versioned(&tenant, "Observation", "obs-1", "1")
        .await;
    let entered = fail_point.off_and_count().await;

    result.expect("a 112 on the history row is run again, not a 409");
    assert_eq!(
        entered, 1,
        "the failpoint fires once; the second attempt is clean"
    );
    assert_deleted_once(&backend, &tenant).await;
}

/// The same 112 at the versioned delete's commit. It relies on the same
/// server-session reuse as
/// `an_if_match_update_that_met_a_write_conflict_at_its_commit_is_run_again`.
#[tokio::test]
async fn a_versioned_delete_that_met_a_write_conflict_at_its_commit_is_run_again() {
    let app = "fp-2342-delete-commit";
    let Some((backend, tenant)) = backend_with_observation("wc2342_delete_commit", app).await
    else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        write_conflict_on("commitTransaction"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .delete_versioned(&tenant, "Observation", "obs-1", "1")
        .await;
    let entered = fail_point.off_and_count().await;

    result.expect("a 112 at the commit is run again, not a 409");
    assert_eq!(entered, 1);
    assert_deleted_once(&backend, &tenant).await;
}

/// `PUT` of an existing resource without `If-Match`, 112 on the history row.
/// This ends the same on the base: `create_or_update` read again and retried
/// what it took for a lost race, so it passes there too. What tells the two
/// apart is
/// `a_put_update_that_keeps_meeting_a_write_conflict_on_its_history_row_gives_up_as_unavailable`.
#[tokio::test]
async fn a_put_update_that_met_a_write_conflict_on_its_history_row_is_run_again() {
    let app = "fp-2342-put-history";
    let Some((backend, tenant)) = backend_with_observation("wc2342_put_history", app).await else {
        return;
    };
    let failure = write_conflict_on_insert_into(&backend, "resource_history");
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 1 }).await else {
        return;
    };

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "amended"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let (stored, created) = result.expect("a 112 on the history row is run again");
    assert!(!created);
    assert_eq!(stored.version_id(), "2");
    assert_eq!(entered, 1);
    assert_updated_once(&backend, &tenant).await;
}

/// `PUT` of an existing resource without `If-Match`, 112 at the commit. Unlike
/// the history-row case above, this is not a guard: on the base it ended as a
/// 409 ("version conflict: expected 1, found unknown"). It relies on the same
/// server-session reuse as
/// `an_if_match_update_that_met_a_write_conflict_at_its_commit_is_run_again`.
#[tokio::test]
async fn a_put_update_that_met_a_write_conflict_at_its_commit_is_run_again() {
    let app = "fp-2342-put-commit";
    let Some((backend, tenant)) = backend_with_observation("wc2342_put_commit", app).await else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        write_conflict_on("commitTransaction"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "amended"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    let (stored, created) = result.expect("a 112 at the commit is run again");
    assert!(!created);
    assert_eq!(stored.version_id(), "2");
    assert_eq!(entered, 1);
    assert_updated_once(&backend, &tenant).await;
}

/// `DELETE` without `If-Match`, 112 on the tombstone's history row. A guard: on
/// the base the delete tried once more as a lost race, so it passes there too.
/// `a_plain_delete_that_keeps_meeting_a_write_conflict_on_its_history_row_gives_up_as_unavailable`
/// tells the two apart.
#[tokio::test]
async fn a_plain_delete_that_met_a_write_conflict_on_its_history_row_is_run_again() {
    let app = "fp-2342-plain-delete-history";
    let Some((backend, tenant)) =
        backend_with_observation("wc2342_plain_delete_history", app).await
    else {
        return;
    };
    let failure = write_conflict_on_insert_into(&backend, "resource_history");
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 1 }).await else {
        return;
    };

    let result = backend.delete(&tenant, "Observation", "obs-1").await;
    let entered = fail_point.off_and_count().await;

    result.expect("a 112 on the history row is run again");
    assert_eq!(entered, 1);
    assert_deleted_once(&backend, &tenant).await;
}

/// `DELETE` without `If-Match`, 112 at the commit. A guard like the test above.
#[tokio::test]
async fn a_plain_delete_that_met_a_write_conflict_at_its_commit_is_run_again() {
    let app = "fp-2342-plain-delete-commit";
    let Some((backend, tenant)) = backend_with_observation("wc2342_plain_delete_commit", app).await
    else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        write_conflict_on("commitTransaction"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend.delete(&tenant, "Observation", "obs-1").await;
    let entered = fail_point.off_and_count().await;

    result.expect("a 112 at the commit is run again");
    assert_eq!(entered, 1);
    assert_deleted_once(&backend, &tenant).await;
}

/// An `If-Match` update whose history insert keeps meeting a 112 gives up as
/// `Unavailable` (a retryable 503) after the policy's attempts, with nothing
/// committed and none of the driver text in the client-facing message. On the
/// base this was a 409 after the first attempt.
#[tokio::test]
async fn an_if_match_update_that_keeps_meeting_a_write_conflict_on_its_history_row_gives_up_as_unavailable()
 {
    let app = "fp-2342-update-history-exhausted";
    let Some((backend, tenant)) =
        backend_with_observation("wc2342_update_history_exhausted", app).await
    else {
        return;
    };
    let current = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("seeded");
    let rows_before = index_rows(&backend, "obs-1").await;
    assert!(rows_before > 0);
    let failure = write_conflict_on_insert_into(&backend, "resource_history");
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 100 }).await else {
        return;
    };

    let result = backend
        .update(&tenant, &current, observation("obs-1", "amended"))
        .await;
    let entered = fail_point.off_and_count().await;

    assert_unavailable_without_driver_text(&result);
    assert_eq!(
        entered, WRITE_ABORT_ATTEMPTS,
        "one conflict per attempt, then give up"
    );
    assert_untouched(&backend, &tenant, rows_before).await;
}

/// An `If-Match` update whose commit keeps meeting a 112 gives up as
/// `Unavailable`, with nothing committed.
///
/// Like `an_update_commit_that_never_applied_stays_unknown`, the last refused
/// commit leaves its server transaction open until the next transaction on that
/// server session or `transactionLifetimeLimitSeconds`, so the test database's
/// drop waits behind it and the cleanup sweeper's `outstanding=` count at exit
/// includes this test.
#[tokio::test]
async fn an_if_match_update_that_keeps_meeting_a_write_conflict_at_its_commit_gives_up_as_unavailable()
 {
    let app = "fp-2342-update-commit-exhausted";
    let Some((backend, tenant)) =
        backend_with_observation("wc2342_update_commit_exhausted", app).await
    else {
        return;
    };
    let current = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("seeded");
    let rows_before = index_rows(&backend, "obs-1").await;
    assert!(rows_before > 0);
    let Some(fail_point) = FailPoint::enable(
        app,
        write_conflict_on("commitTransaction"),
        doc! { "times": 100 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .update(&tenant, &current, observation("obs-1", "amended"))
        .await;
    let entered = fail_point.off_and_count().await;

    assert_unavailable_without_driver_text(&result);
    assert_eq!(
        entered, WRITE_ABORT_ATTEMPTS,
        "one conflict per attempt, then give up"
    );
    assert_untouched(&backend, &tenant, rows_before).await;
}

/// A versioned delete whose history insert keeps meeting a 112 gives up as
/// `Unavailable`: the resource stays live at version 1.
#[tokio::test]
async fn a_versioned_delete_that_keeps_meeting_a_write_conflict_on_its_history_row_gives_up_as_unavailable()
 {
    let app = "fp-2342-delete-history-exhausted";
    let Some((backend, tenant)) =
        backend_with_observation("wc2342_delete_history_exhausted", app).await
    else {
        return;
    };
    let rows_before = index_rows(&backend, "obs-1").await;
    assert!(rows_before > 0);
    let failure = write_conflict_on_insert_into(&backend, "resource_history");
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 100 }).await else {
        return;
    };

    let result = backend
        .delete_versioned(&tenant, "Observation", "obs-1", "1")
        .await;
    let entered = fail_point.off_and_count().await;

    assert_unavailable_without_driver_text(&result);
    assert_eq!(
        entered, WRITE_ABORT_ATTEMPTS,
        "one conflict per attempt, then give up"
    );
    assert_untouched(&backend, &tenant, rows_before).await;
}

/// A versioned delete whose commit keeps meeting a 112 gives up as
/// `Unavailable`. The last refused commit leaves its server transaction open,
/// as described at `an_if_match_update_that_keeps_meeting_a_write_conflict_at_its_commit_gives_up_as_unavailable`.
#[tokio::test]
async fn a_versioned_delete_that_keeps_meeting_a_write_conflict_at_its_commit_gives_up_as_unavailable()
 {
    let app = "fp-2342-delete-commit-exhausted";
    let Some((backend, tenant)) =
        backend_with_observation("wc2342_delete_commit_exhausted", app).await
    else {
        return;
    };
    let rows_before = index_rows(&backend, "obs-1").await;
    assert!(rows_before > 0);
    let Some(fail_point) = FailPoint::enable(
        app,
        write_conflict_on("commitTransaction"),
        doc! { "times": 100 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .delete_versioned(&tenant, "Observation", "obs-1", "1")
        .await;
    let entered = fail_point.off_and_count().await;

    assert_unavailable_without_driver_text(&result);
    assert_eq!(
        entered, WRITE_ABORT_ATTEMPTS,
        "one conflict per attempt, then give up"
    );
    assert_untouched(&backend, &tenant, rows_before).await;
}

/// A `PUT` of an existing resource whose history insert keeps meeting a 112
/// gives up as `Unavailable`. On the base `create_or_update` took each 112 for a
/// lost race and ended as a 409 after its read-and-retry passes.
#[tokio::test]
async fn a_put_update_that_keeps_meeting_a_write_conflict_on_its_history_row_gives_up_as_unavailable()
 {
    let app = "fp-2342-put-history-exhausted";
    let Some((backend, tenant)) =
        backend_with_observation("wc2342_put_history_exhausted", app).await
    else {
        return;
    };
    let rows_before = index_rows(&backend, "obs-1").await;
    assert!(rows_before > 0);
    let failure = write_conflict_on_insert_into(&backend, "resource_history");
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 100 }).await else {
        return;
    };

    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "amended"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;

    assert_unavailable_without_driver_text(&result);
    assert_eq!(
        entered, WRITE_ABORT_ATTEMPTS,
        "one conflict per attempt, then give up"
    );
    assert_untouched(&backend, &tenant, rows_before).await;
}

/// A `DELETE` without `If-Match` whose history insert keeps meeting a 112 gives
/// up as `Unavailable`. On the base the delete tried once more as a lost race
/// and ended as a 409 "expected version unknown, but found 1".
#[tokio::test]
async fn a_plain_delete_that_keeps_meeting_a_write_conflict_on_its_history_row_gives_up_as_unavailable()
 {
    let app = "fp-2342-plain-delete-history-exhausted";
    let Some((backend, tenant)) =
        backend_with_observation("wc2342_plain_delete_history_exhausted", app).await
    else {
        return;
    };
    let rows_before = index_rows(&backend, "obs-1").await;
    assert!(rows_before > 0);
    let failure = write_conflict_on_insert_into(&backend, "resource_history");
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 100 }).await else {
        return;
    };

    let result = backend.delete(&tenant, "Observation", "obs-1").await;
    let entered = fail_point.off_and_count().await;

    assert_unavailable_without_driver_text(&result);
    assert_eq!(
        entered, WRITE_ABORT_ATTEMPTS,
        "one conflict per attempt, then give up"
    );
    assert_untouched(&backend, &tenant, rows_before).await;
}

/// While an update sits at its history insert, no other update can win the
/// resource. The held update's insert is blocked for 4 s and then fails with a
/// 112. By then its compare-and-swap has written the resource document in its
/// transaction, so a rival update from the same version 1 meets that write on
/// the resource document and loses (a `VersionConflict` that expected version
/// 1). The held update's own 112 names no winner and it is run again: it
/// commits version 2, and the rival's content never lands.
/// That a rerun still checks the version the update started from, so a winner
/// that committed in between answers 409, is pinned by
/// `a_retried_update_keeps_its_original_version_precondition`.
#[tokio::test]
async fn a_rival_update_cannot_win_while_an_update_holds_the_resource_at_its_history_row() {
    let app = "fp-2342-rival";
    let Some((backend, tenant)) = backend_with_observation("wc2342_rival", app).await else {
        return;
    };
    let current = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("seeded");
    let mut failure = write_conflict_on_insert_into(&backend, "resource_history");
    failure.insert("blockConnection", true);
    failure.insert("blockTimeMS", 4000);
    let Some(fail_point) = FailPoint::enable(app, failure, doc! { "times": 1 }).await else {
        return;
    };

    let held = backend.update(&tenant, &current, observation("obs-1", "amended"));
    let rival = async {
        fail_point.wait_until_entered(1).await;
        backend
            .update(&tenant, &current, observation("obs-1", "corrected"))
            .await
    };
    let (held, rival) = tokio::join!(held, rival);
    let entered = fail_point.off_and_count().await;

    assert!(
        matches!(
            &rival,
            Err(StorageError::Concurrency(ConcurrencyError::VersionConflict {
                expected_version,
                ..
            })) if expected_version == "1"
        ),
        "the rival met the held update's write on the resource document: {rival:?}"
    );
    let held = held.expect("the held update's 112 names no winner; it is run again");
    assert_eq!(held.version_id(), "2");
    assert_eq!(entered, 1);
    assert_updated_once(&backend, &tenant).await;
}

/// A 112 on the resource document itself keeps its lost-race classification: a
/// versioned delete whose tombstone update meets one is a `VersionConflict`
/// (a 409) at once, with no second attempt, and nothing committed. The tombstone
/// is the only `update` command a delete sends, so the failpoint does not touch
/// its history insert or its search-index delete.
#[tokio::test]
async fn a_versioned_delete_that_lost_a_race_on_the_resource_document_is_still_a_version_conflict()
{
    let app = "fp-2342-delete-lost-race";
    let Some((backend, tenant)) = backend_with_observation("wc2342_delete_lost_race", app).await
    else {
        return;
    };
    let rows_before = index_rows(&backend, "obs-1").await;
    assert!(rows_before > 0);
    let Some(fail_point) =
        FailPoint::enable(app, write_conflict_on("update"), doc! { "times": 1 }).await
    else {
        return;
    };

    let result = backend
        .delete_versioned(&tenant, "Observation", "obs-1", "1")
        .await;
    let entered = fail_point.off_and_count().await;

    assert!(
        matches!(
            &result,
            Err(StorageError::Concurrency(ConcurrencyError::VersionConflict {
                expected_version,
                ..
            })) if expected_version == "1"
        ),
        "{result:?}"
    );
    assert_eq!(entered, 1, "a lost race is not run again");
    assert_untouched(&backend, &tenant, rows_before).await;
}

// ---------------------------------------------------------------------------
// #2307: a failed attempt's transaction is aborted, and the abort awaited,
// before the write runs again. Before, only the dropped session aborted it, in
// the background, and the rerun started 100 ms after the failure without
// waiting for that abort. It recovered only when the driver's server-session
// pool (most recently returned first) happened to hand it the failed attempt's
// server session, which the abort had not released yet in these tests.
// ---------------------------------------------------------------------------

/// How long [`held_index_delete_and_abort_refused`] holds each of the two
/// commands it catches before it fails them, in milliseconds.
const ABORT_HOLD_MS: i64 = 1500;

/// Commands the #2307 failpoints catch: the write's search-index `delete` and
/// the first `abortTransaction` after it. The driver's second
/// `abortTransaction` and the rerun come after the failpoint's two `times`.
const HELD_COMMANDS: i64 = 2;

/// The failpoint's `data` for a write whose search-index `delete` is held for
/// [`ABORT_HOLD_MS`] and then fails with a `StaleConfig` that a transaction
/// retry replays, and whose first `abortTransaction` is held and refused the
/// same way.
///
/// The `delete` is the write's search-index step, sent after its
/// compare-and-swap (or resource insert) and its history insert, so the failed
/// attempt holds the resource document in its open transaction. The refused
/// `abortTransaction` carries `RetryableWriteError`, which makes the driver send
/// it once more whatever `retryWrites` says (mongodb 3.7
/// `operation/abort_transaction.rs`). That second abort is past the two `times`
/// and ends the transaction on the server. Until it does, a rerun on another
/// server session meets the open transaction's write on the resource document.
fn held_index_delete_and_abort_refused() -> Document {
    doc! {
        "failCommands": ["delete", "abortTransaction"],
        "errorCode": STALE_CONFIG,
        "errorLabels": ["TransientTransactionError", "RetryableWriteError"],
        "blockConnection": true,
        "blockTimeMS": ABORT_HOLD_MS,
    }
}

/// [`held_index_delete_and_abort_refused`] with a dropped connection in place
/// of the error. The network error marks the attempt's server session dirty,
/// so the driver's pool never hands it out again, and the driver labels the
/// dropped `abortTransaction` `RetryableWriteError` itself, so it is sent once
/// more.
fn held_index_delete_and_abort_dropping_the_connection() -> Document {
    doc! {
        "failCommands": ["delete", "abortTransaction"],
        "closeConnection": true,
        "blockConnection": true,
        "blockTimeMS": ABORT_HOLD_MS,
    }
}

/// A write that ran again after `elapsed` waited out both held commands of
/// [`held_index_delete_and_abort_refused`] first: the failed index `delete` and
/// the abort after it. A rerun that did not wait for the abort ran in 100 ms
/// plus its own work after the `delete` failed.
fn assert_waited_for_the_abort(elapsed: std::time::Duration) {
    assert!(
        elapsed >= std::time::Duration::from_millis(2 * ABORT_HOLD_MS as u64),
        "the write ran again only after the held abort had answered, but took {elapsed:?}"
    );
}

/// `If-Match` update whose index `delete` fails and whose abort is held. The
/// rerun waits for the abort, then runs on a clean document and commits
/// version 2.
///
/// Before, the failed attempt's session was dropped and its abort ran in the
/// background behind the held `abortTransaction`. The rerun started 100 ms
/// later on another server session and met the open transaction's write on the
/// resource document: a `VersionConflict` (a 409) for a resource nobody had
/// changed.
#[tokio::test]
async fn an_update_runs_again_only_after_its_failed_attempt_is_aborted() {
    let app = "fp-2307-update";
    let Some((backend, tenant)) = backend_with_observation("abort2307_update", app).await else {
        return;
    };
    let current = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("seeded");
    let Some(fail_point) = FailPoint::enable(
        app,
        held_index_delete_and_abort_refused(),
        doc! { "times": HELD_COMMANDS },
    )
    .await
    else {
        return;
    };

    let started = std::time::Instant::now();
    let result = backend
        .update(&tenant, &current, observation("obs-1", "amended"))
        .await;
    let elapsed = started.elapsed();
    let entered = fail_point.off_and_count().await;

    let stored = result.expect("the rerun waits for the abort and then commits");
    assert_eq!(stored.version_id(), "2");
    assert_eq!(
        entered, HELD_COMMANDS,
        "the index delete and the first abort were held"
    );
    assert_waited_for_the_abort(elapsed);
    assert_updated_once(&backend, &tenant).await;
}

/// [`an_update_runs_again_only_after_its_failed_attempt_is_aborted`] with a
/// dropped connection instead of an error. The attempt's server session is
/// marked dirty and the driver's pool never hands it out again, so the rerun
/// cannot reuse it: it takes another server session and can pass only because
/// the abort finished first.
///
/// Before, the rerun started 100 ms after the failure, behind the held abort,
/// and met the open transaction's write: a `VersionConflict`.
#[tokio::test]
async fn an_update_that_lost_its_connection_runs_again_only_after_its_failed_attempt_is_aborted() {
    let app = "fp-2307-update-closed";
    let Some((backend, tenant)) = backend_with_observation("abort2307_update_closed", app).await
    else {
        return;
    };
    let current = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .expect("seeded");
    let Some(fail_point) = FailPoint::enable(
        app,
        held_index_delete_and_abort_dropping_the_connection(),
        doc! { "times": HELD_COMMANDS },
    )
    .await
    else {
        return;
    };

    let started = std::time::Instant::now();
    let result = backend
        .update(&tenant, &current, observation("obs-1", "amended"))
        .await;
    let elapsed = started.elapsed();
    let entered = fail_point.off_and_count().await;

    let stored = result.expect("the rerun waits for the abort and then commits");
    assert_eq!(stored.version_id(), "2");
    assert_eq!(
        entered, HELD_COMMANDS,
        "the index delete and the first abort were held"
    );
    assert_waited_for_the_abort(elapsed);
    assert_updated_once(&backend, &tenant).await;
}

/// A create whose index `delete` fails and whose abort is held. The rerun waits
/// for the abort and stores the new resource.
///
/// Before, the rerun started 100 ms after the failure on another server session
/// and met the open transaction's insert on every attempt, so the create gave up
/// as `Unavailable` after the policy's four attempts. The first write is
/// seeded as in `a_create_that_hit_a_write_conflict_at_its_commit_is_run_again`.
#[tokio::test]
async fn a_create_runs_again_only_after_its_failed_attempt_is_aborted() {
    let app = "fp-2307-create";
    let Some(backend) = create_backend_with_app_name("abort2307_create", app).await else {
        return;
    };
    let tenant = create_tenant("abort2307_create");
    backend
        .create(
            &tenant,
            "Observation",
            new_observation("preliminary"),
            FhirVersion::default(),
        )
        .await
        .expect("seeding a first write");
    let Some(fail_point) = FailPoint::enable(
        app,
        held_index_delete_and_abort_refused(),
        doc! { "times": HELD_COMMANDS },
    )
    .await
    else {
        return;
    };

    let started = std::time::Instant::now();
    let result = backend
        .create(
            &tenant,
            "Observation",
            new_observation("final"),
            FhirVersion::default(),
        )
        .await;
    let elapsed = started.elapsed();
    let entered = fail_point.off_and_count().await;

    let stored = result.expect("the rerun waits for the abort and then commits");
    assert_eq!(stored.version_id(), "1");
    assert_eq!(
        entered, HELD_COMMANDS,
        "the index delete and the first abort were held"
    );
    assert_waited_for_the_abort(elapsed);
    assert_eq!(tenant_rows(&backend, &tenant).await, (2, 2));
    assert_status(&backend, &tenant, stored.id(), "final", "amended").await;
}

/// A versioned delete whose index `delete` fails and whose abort is held. The
/// rerun waits for the abort and deletes version 1.
///
/// Before, the rerun started 100 ms after the failure on another server session
/// and met the open transaction's tombstone write: a `VersionConflict` (a 409)
/// for a resource nobody had changed.
#[tokio::test]
async fn a_versioned_delete_runs_again_only_after_its_failed_attempt_is_aborted() {
    let app = "fp-2307-delete";
    let Some((backend, tenant)) = backend_with_observation("abort2307_delete", app).await else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        held_index_delete_and_abort_refused(),
        doc! { "times": HELD_COMMANDS },
    )
    .await
    else {
        return;
    };

    let started = std::time::Instant::now();
    let result = backend
        .delete_versioned(&tenant, "Observation", "obs-1", "1")
        .await;
    let elapsed = started.elapsed();
    let entered = fail_point.off_and_count().await;

    result.expect("the rerun waits for the abort and then deletes");
    assert_eq!(
        entered, HELD_COMMANDS,
        "the index delete and the first abort were held"
    );
    assert_waited_for_the_abort(elapsed);
    assert_deleted_once(&backend, &tenant).await;
}

/// Restoring a deleted id whose index `delete` fails and whose abort is held.
/// The rerun waits for the abort and restores it as version 3.
///
/// Before, the rerun started 100 ms after the failure on another server session
/// and met the open transaction's write on the resource document: the restore's
/// compare-and-swap lost, and it answered `VersionConflict`.
#[tokio::test]
async fn a_restore_runs_again_only_after_its_failed_attempt_is_aborted() {
    let app = "fp-2307-restore";
    let Some((backend, tenant)) = backend_with_deleted_observation("abort2307_restore", app).await
    else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        held_index_delete_and_abort_refused(),
        doc! { "times": HELD_COMMANDS },
    )
    .await
    else {
        return;
    };

    let started = std::time::Instant::now();
    let result = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            observation("obs-1", "amended"),
            FhirVersion::default(),
        )
        .await;
    let elapsed = started.elapsed();
    let entered = fail_point.off_and_count().await;

    let (stored, created) = result.expect("the rerun waits for the abort and then restores");
    assert!(created, "restoring a deleted id is a create outcome");
    assert_eq!(stored.version_id(), "3");
    assert_eq!(
        entered, HELD_COMMANDS,
        "the index delete and the first abort were held"
    );
    assert_waited_for_the_abort(elapsed);
    assert_eq!(history_rows(&backend, "obs-1").await, 3);
    assert_status(&backend, &tenant, "obs-1", "amended", "final").await;
}
