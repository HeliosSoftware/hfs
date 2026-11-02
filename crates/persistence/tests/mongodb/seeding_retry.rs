//! The seeding helper of #2435: a test's setup write is run again when, and
//! only when, the database rolled it back under cache pressure.
//!
//! The harness's shared mongod has a 0.25 GB cache. The product answers a
//! single-resource write that WiredTiger rolled back for eviction with
//! `Unavailable` ("... rolled back by the database under cache pressure; retry
//! the request") and does not run it again on purpose (#1895), so a setup
//! `.unwrap()` used to panic before the test tested anything. `SeedingStorage`
//! (`seed_create`, `seed_update`, `seed_put`, `seed_delete`, in
//! `common/seeding.rs`) is the bounded retry that setup uses; the write a test
//! asserts about stays bare.
//!
//! The pure tests pin the match rule and the retry loop; the failpoint tests
//! make the server roll a real create back (365 on its `insert`) and pin that
//! the predicate matches what the product sends, that the seeding create is
//! run again and lands once, that it gives up after its attempts, and that
//! another server error is not run again.
//!
//! Child module of the `mongodb_tests` root, like `write_abort_retry.rs`:
//! `use super::*;` reaches its harness.

use super::*;

use crate::bulk_submit::FailPoint;
use crate::seeding::{
    EVICTION_ROLLBACK_WRITES, SEED_EVICTION_ATTEMPTS, forget_forced_seed_eviction_retries,
    is_eviction_rollback_unavailable, retry_eviction_rollback, retry_eviction_rollback_counting,
    seed_eviction_retries,
};
use std::sync::atomic::{AtomicU64, Ordering};

/// The `Unavailable` the product answers a `what` write with after an
/// eviction rollback.
fn eviction(what: &str) -> StorageError {
    StorageError::Backend(BackendError::Unavailable {
        backend_name: "mongodb".into(),
        message: format!(
            "The {what} was rolled back by the database under cache pressure; retry the request"
        ),
    })
}

/// An `Unavailable` from the MongoDB backend with `message`.
fn mongodb_unavailable(message: &str) -> StorageError {
    StorageError::Backend(BackendError::Unavailable {
        backend_name: "mongodb".into(),
        message: message.into(),
    })
}

/// The failpoint's `data` for a `failCommand` that rolls the transaction back
/// on its first `command` the way WiredTiger's `TemporarilyUnavailable` does, as
/// `write_abort_retry.rs`'s eviction test sends it.
fn eviction_on(command: &str) -> Document {
    doc! {
        "failCommands": [command],
        "errorCode": 365,
        "errorLabels": ["TransientTransactionError"],
    }
}

/// Only the eviction rollback of a single-resource write is recognised: not
/// another "retry the request" 503, not another backend, not another error.
#[test]
fn only_the_eviction_rollback_unavailable_is_recognised() {
    for what in EVICTION_ROLLBACK_WRITES {
        let err = eviction(what);
        assert!(is_eviction_rollback_unavailable(&err), "{err:?}");
    }

    let create_text =
        "The create was rolled back by the database under cache pressure; retry the request";
    let others = [
        StorageError::Backend(BackendError::Unavailable {
            backend_name: "postgres".into(),
            message: create_text.into(),
        }),
        StorageError::Backend(BackendError::ConnectionFailed {
            backend_name: "mongodb".into(),
            message: create_text.into(),
        }),
        mongodb_unavailable(
            "The create was aborted by the database 4 times without a conflicting writer; retry the request",
        ),
        mongodb_unavailable("The create was aborted by the database 4 times; retry the request"),
        mongodb_unavailable(
            "The create found no usable database server; the database is temporarily unavailable; retry the request",
        ),
        mongodb_unavailable(&format!("{create_text} (again)")),
        mongodb_unavailable(
            "The patch was rolled back by the database under cache pressure; retry the request",
        ),
        StorageError::Resource(ResourceError::AlreadyExists {
            resource_type: "Patient".into(),
            id: "p".into(),
        }),
    ];
    for err in others {
        assert!(!is_eviction_rollback_unavailable(&err), "{err:?}");
    }
}

/// The loop runs a write again after an eviction rollback, at most
/// `SEED_EVICTION_ATTEMPTS` times in all, and returns anything else at once.
#[tokio::test(start_paused = true)]
async fn a_seeding_write_is_run_again_only_after_an_eviction_rollback() {
    // Two rollbacks, then a success: run three times.
    let retries = AtomicU64::new(0);
    let mut runs = 0_u32;
    let result = retry_eviction_rollback_counting(&retries, "script", || {
        runs += 1;
        std::future::ready(if runs <= 2 {
            Err(eviction("create"))
        } else {
            Ok(7)
        })
    })
    .await;
    assert_eq!(result.expect("the third run succeeds"), 7);
    assert_eq!(runs, 3, "two rollbacks are run again");
    assert_eq!(
        retries.load(Ordering::Relaxed),
        2,
        "each run again is counted"
    );

    // A rollback on every run: the attempts are bounded, the last one is returned.
    let retries = AtomicU64::new(0);
    let mut runs = 0_u32;
    let result: Result<u32, StorageError> =
        retry_eviction_rollback_counting(&retries, "script", || {
            runs += 1;
            std::future::ready(Err(eviction("create")))
        })
        .await;
    let err = result.expect_err("a rollback on every run ends as the rollback");
    assert!(is_eviction_rollback_unavailable(&err), "{err:?}");
    assert_eq!(runs, SEED_EVICTION_ATTEMPTS, "the attempts are bounded");
    assert_eq!(
        retries.load(Ordering::Relaxed),
        u64::from(SEED_EVICTION_ATTEMPTS - 1),
        "the last run is not a run again"
    );

    // Any other error is returned after the first run.
    type Other = fn() -> StorageError;
    let others: [(&str, Other); 3] = [
        ("already exists", || {
            StorageError::Resource(ResourceError::AlreadyExists {
                resource_type: "Patient".into(),
                id: "p".into(),
            })
        }),
        ("replay exhausted", || {
            mongodb_unavailable(
                "The create was aborted by the database 4 times without a conflicting writer; retry the request",
            )
        }),
        ("another backend", || {
            StorageError::Backend(BackendError::Unavailable {
                backend_name: "postgres".into(),
                message: "The create was rolled back by the database under cache pressure; retry the request".into(),
            })
        }),
    ];
    for (name, build) in others {
        let retries = AtomicU64::new(0);
        let mut runs = 0_u32;
        let result: Result<u32, StorageError> =
            retry_eviction_rollback_counting(&retries, "script", || {
                runs += 1;
                std::future::ready(Err(build()))
            })
            .await;
        assert!(result.is_err(), "{name}: {result:?}");
        assert_eq!(runs, 1, "{name} is not run again");
        assert_eq!(retries.load(Ordering::Relaxed), 0, "{name} is not counted");
    }
}

/// A seeding retry is counted in the process-wide counter that `db_cleanup`
/// prints on its end-of-run line; a test that forces one takes it back out, so
/// the number is how often eviction really hit a test's setup.
#[tokio::test(start_paused = true)]
async fn a_seeding_retry_is_counted_for_the_end_of_run_report() {
    let mut runs = 0_u32;
    let result = retry_eviction_rollback("script", || {
        runs += 1;
        std::future::ready(if runs == 1 {
            Err(eviction("create"))
        } else {
            Ok(runs)
        })
    })
    .await;
    assert_eq!(result.expect("the second run succeeds"), 2);
    // Other tests take their own forced retries out, never this one's, so the
    // process-wide count is at least this test's retry.
    assert!(seed_eviction_retries() >= 1, "{}", seed_eviction_retries());
    forget_forced_seed_eviction_retries(1);
}

/// A real eviction rollback: what the product sends is what the predicate
/// matches (a positive control, not a string copied into the test), the
/// seeding create runs the create again and it lands once, and it gives up
/// after its attempts.
#[tokio::test]
async fn a_seeding_create_rolled_back_for_eviction_is_run_again() {
    let app = "fp-seed-eviction";
    let Some(backend) = create_backend_with_app_name("seed_eviction", app).await else {
        return;
    };
    let tenant = create_tenant("seed-eviction");
    let patient = |id: &str| json!({"resourceType": "Patient", "id": id});

    // Part 1: the bare create meets one rollback and the backend does not run
    // it again; the predicate matches the error as the product sends it.
    let Some(fail_point) = FailPoint::enable(app, eviction_on("insert"), doc! { "times": 1 }).await
    else {
        return;
    };
    let err = backend
        .create(
            &tenant,
            "Patient",
            patient("bare-eviction"),
            FhirVersion::default(),
        )
        .await
        .expect_err("the create is rolled back");
    let entered = fail_point.off_and_count().await;
    assert!(is_eviction_rollback_unavailable(&err), "{err:?}");
    assert_eq!(
        entered, 1,
        "an eviction rollback is not run again by the backend"
    );
    assert_eq!(
        count_docs(&backend, "resources", doc! { "id": "bare-eviction" }).await,
        0
    );

    // Part 2: the seeding create is run again after one rollback and lands once.
    let Some(fail_point) = FailPoint::enable(app, eviction_on("insert"), doc! { "times": 1 }).await
    else {
        return;
    };
    let stored = backend
        .seed_create(
            &tenant,
            "Patient",
            patient("seeded-eviction"),
            FhirVersion::default(),
        )
        .await
        .expect("a seeding create rolled back for eviction is run again");
    let entered = fail_point.off_and_count().await;
    assert_eq!(stored.version_id(), "1");
    assert_eq!(entered, 1, "the rollback happened once");
    forget_forced_seed_eviction_retries(1);
    assert_eq!(
        count_docs(&backend, "resources", doc! { "id": "seeded-eviction" }).await,
        1
    );
    assert_eq!(
        count_docs(
            &backend,
            "resource_history",
            doc! { "id": "seeded-eviction" }
        )
        .await,
        1
    );

    // Part 3: a rollback on every attempt ends as the rollback after the
    // bounded number of attempts.
    let Some(fail_point) = FailPoint::enable(
        app,
        eviction_on("insert"),
        doc! { "times": i32::try_from(SEED_EVICTION_ATTEMPTS).unwrap() },
    )
    .await
    else {
        return;
    };
    let err = backend
        .seed_create(
            &tenant,
            "Patient",
            patient("exhausted-eviction"),
            FhirVersion::default(),
        )
        .await
        .expect_err("a rollback on every attempt ends the seeding create");
    let entered = fail_point.off_and_count().await;
    assert!(is_eviction_rollback_unavailable(&err), "{err:?}");
    assert_eq!(
        entered,
        i64::from(SEED_EVICTION_ATTEMPTS),
        "the seeding create gives up after its attempts"
    );
    assert_eq!(
        count_docs(&backend, "resources", doc! { "id": "exhausted-eviction" }).await,
        0
    );
    forget_forced_seed_eviction_retries(u64::from(SEED_EVICTION_ATTEMPTS - 1));
}

/// The other seeding writes meet a real eviction rollback too: `seed_put` (a
/// create, an update and a restore), `seed_update` and `seed_delete` are each
/// run again after one 365 on the command that write reaches, and land once.
/// That each lands after one rollback is the positive control that the
/// predicate matches the update, delete and restore messages the product sends.
#[tokio::test]
async fn the_other_seeding_writes_rolled_back_for_eviction_are_run_again() {
    let app = "fp-seed-writes";
    let Some(backend) = create_backend_with_app_name("seed_writes_eviction", app).await else {
        return;
    };
    let tenant = create_tenant("seed-writes-eviction");
    let id = "seeded-writes";
    let patient =
        |family: &str| json!({"resourceType": "Patient", "id": id, "name": [{"family": family}]});
    let history = || count_docs(&backend, "resource_history", doc! { "id": id });

    // A PUT of a new id is a create: `insert`.
    let Some(fail_point) = FailPoint::enable(app, eviction_on("insert"), doc! { "times": 1 }).await
    else {
        return;
    };
    let (v1, created) = backend
        .seed_put(
            &tenant,
            "Patient",
            id,
            patient("one"),
            FhirVersion::default(),
        )
        .await
        .expect("a seeding PUT that creates is run again after a rollback");
    let entered = fail_point.off_and_count().await;
    assert!(created);
    assert_eq!(v1.version_id(), "1");
    assert_eq!(entered, 1, "the rollback happened once");
    assert_eq!(history().await, 1);

    // An update: `update`.
    let Some(fail_point) = FailPoint::enable(app, eviction_on("update"), doc! { "times": 1 }).await
    else {
        return;
    };
    let v2 = backend
        .seed_update(&tenant, &v1, patient("two"))
        .await
        .expect("a seeding update is run again after a rollback");
    let entered = fail_point.off_and_count().await;
    assert_eq!(v2.version_id(), "2");
    assert_eq!(entered, 1, "the rollback happened once");
    assert_eq!(history().await, 2);

    // A PUT of an existing id is an update: `update`.
    let Some(fail_point) = FailPoint::enable(app, eviction_on("update"), doc! { "times": 1 }).await
    else {
        return;
    };
    let (v3, created) = backend
        .seed_put(
            &tenant,
            "Patient",
            id,
            patient("three"),
            FhirVersion::default(),
        )
        .await
        .expect("a seeding PUT that updates is run again after a rollback");
    let entered = fail_point.off_and_count().await;
    assert!(!created);
    assert_eq!(v3.version_id(), "3");
    assert_eq!(entered, 1, "the rollback happened once");
    assert_eq!(history().await, 3);

    // A delete is an `update` of the live row.
    let Some(fail_point) = FailPoint::enable(app, eviction_on("update"), doc! { "times": 1 }).await
    else {
        return;
    };
    backend
        .seed_delete(&tenant, "Patient", id)
        .await
        .expect("a seeding delete is run again after a rollback");
    let entered = fail_point.off_and_count().await;
    assert_eq!(entered, 1, "the rollback happened once");
    assert_eq!(history().await, 4);
    assert!(matches!(
        backend.read(&tenant, "Patient", id).await,
        Err(StorageError::Resource(ResourceError::Gone { .. }))
    ));

    // A PUT onto the tombstone is a restore: `update`.
    let Some(fail_point) = FailPoint::enable(app, eviction_on("update"), doc! { "times": 1 }).await
    else {
        return;
    };
    let (v5, created) = backend
        .seed_put(
            &tenant,
            "Patient",
            id,
            patient("five"),
            FhirVersion::default(),
        )
        .await
        .expect("a seeding PUT that restores is run again after a rollback");
    let entered = fail_point.off_and_count().await;
    assert!(created);
    assert_eq!(v5.version_id(), "5");
    assert_eq!(entered, 1, "the rollback happened once");
    assert_eq!(history().await, 5);

    // Five forced rollbacks, each counted; take them back out.
    assert!(seed_eviction_retries() >= 5, "{}", seed_eviction_retries());
    forget_forced_seed_eviction_retries(5);
}

/// The retry is for the eviction rollback only: another server error ends the
/// seeding create at once.
#[tokio::test]
async fn a_seeding_create_is_not_run_again_after_another_server_error() {
    let app = "fp-seed-other-error";
    let Some(backend) = create_backend_with_app_name("seed_other_error", app).await else {
        return;
    };
    let tenant = create_tenant("seed-other-error");
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! { "failCommands": ["insert"], "errorCode": 2 },
        doc! { "times": 2 },
    )
    .await
    else {
        return;
    };

    let err = backend
        .seed_create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "id": "other-error"}),
            FhirVersion::default(),
        )
        .await
        .expect_err("another server error ends the seeding create");
    let entered = fail_point.off_and_count().await;

    assert!(!is_eviction_rollback_unavailable(&err), "{err:?}");
    assert_eq!(entered, 1, "another error ends the seeding create at once");
    assert_eq!(
        count_docs(&backend, "resources", doc! { "id": "other-error" }).await,
        0
    );
}
