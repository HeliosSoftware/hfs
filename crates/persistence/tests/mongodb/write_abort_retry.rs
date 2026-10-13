//! A single-resource write whose transaction the server aborts without a
//! conflicting writer is run again. The case this pins: a chunk migration
//! holding the critical section of `search_index` or `resources`, which the
//! server reports as `StaleConfig` (13388) labelled `TransientTransactionError`.
//! Before, that label alone was read as a lost race: a `PUT` ended as a 500
//! when the abort hit the search-index delete, or as a 409 "expected version
//! 1, but found unknown" when it hit the resource update, with nobody having
//! won anything.
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

/// The failpoint's `data` for a `failCommand` that aborts `command` the way a
/// chunk migration's critical section does.
fn placement_change_on(command: &str) -> Document {
    doc! {
        "failCommands": [command],
        "errorCode": STALE_CONFIG,
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

fn observation(id: &str, status: &str) -> serde_json::Value {
    json!({
        "resourceType": "Observation",
        "id": id,
        "status": status,
        "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
    })
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

    assert!(
        matches!(
            result,
            Err(StorageError::Backend(BackendError::Unavailable { .. }))
        ),
        "{result:?}"
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
#[tokio::test]
async fn a_lost_race_is_still_a_version_conflict_without_a_second_attempt() {
    let app = "fp-write-abort-lost-race";
    let Some((backend, tenant)) = backend_with_observation("write_abort_lost_race", app).await
    else {
        return;
    };
    let Some(fail_point) =
        FailPoint::enable(app, write_conflict_on("update"), doc! { "times": 1 }).await
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
            Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict { .. }
            ))
        ),
        "{result:?}"
    );
    assert_eq!(entered, 1, "a lost race is not run again");
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

#[tokio::test]
async fn an_unknown_create_commit_is_not_replayed() {
    let app = "fp-write-abort-unknown-commit";
    let Some(backend) = create_backend_with_app_name("write_abort_unknown_commit", app).await
    else {
        return;
    };
    let tenant = create_tenant("write_abort_unknown_commit");
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! {
            "failCommands": ["commitTransaction"],
            "writeConcernError": {
                "code": 64,
                "codeName": "WriteConcernFailed",
                "errmsg": "waiting for replication timed out",
                "errInfo": { "wtimeout": true },
            },
        },
        doc! { "times": 100 },
    )
    .await
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
    fail_point.off().await;

    let Err(StorageError::Backend(BackendError::Internal {
        source: Some(source),
        ..
    })) = result
    else {
        panic!("the original ambiguous commit error must propagate, not a replay's result");
    };
    let driver = source
        .downcast_ref::<mongodb::error::Error>()
        .expect("the driver error is retained");
    assert!(driver.contains_label(mongodb::error::UNKNOWN_TRANSACTION_COMMIT_RESULT));
    let scope = doc! {"tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation"};
    assert_eq!(count_docs(&backend, "resources", scope.clone()).await, 1);
    assert_eq!(count_docs(&backend, "resource_history", scope).await, 1);
}
