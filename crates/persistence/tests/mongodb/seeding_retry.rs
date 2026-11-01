//! The seeding helper of #2435: a test's setup write is run again when, and
//! only when, the database rolled it back under cache pressure.
//!
//! The harness's shared mongod has a 0.25 GB cache. The product answers a
//! single-resource write that WiredTiger rolled back for eviction with
//! `Unavailable` ("... rolled back by the database under cache pressure; retry
//! the request") and does not run it again on purpose (#1895), so a setup
//! `.unwrap()` used to panic before the test tested anything. `SeedingStorage`
//! (`seed_create`, `seed_delete`) is the bounded retry that setup uses; the
//! write a test asserts about stays bare.
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
/// on its first `insert` the way WiredTiger's `TemporarilyUnavailable` does, as
/// `write_abort_retry.rs`'s eviction test sends it.
fn eviction_on_insert() -> Document {
    doc! {
        "failCommands": ["insert"],
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
    let mut runs = 0_u32;
    let result = retry_eviction_rollback("script", || {
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

    // A rollback on every run: the attempts are bounded, the last one is returned.
    let mut runs = 0_u32;
    let result: Result<u32, StorageError> = retry_eviction_rollback("script", || {
        runs += 1;
        std::future::ready(Err(eviction("create")))
    })
    .await;
    let err = result.expect_err("a rollback on every run ends as the rollback");
    assert!(is_eviction_rollback_unavailable(&err), "{err:?}");
    assert_eq!(runs, SEED_EVICTION_ATTEMPTS, "the attempts are bounded");

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
        let mut runs = 0_u32;
        let result: Result<u32, StorageError> = retry_eviction_rollback("script", || {
            runs += 1;
            std::future::ready(Err(build()))
        })
        .await;
        assert!(result.is_err(), "{name}: {result:?}");
        assert_eq!(runs, 1, "{name} is not run again");
    }
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
    let Some(fail_point) = FailPoint::enable(app, eviction_on_insert(), doc! { "times": 1 }).await
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
    let Some(fail_point) = FailPoint::enable(app, eviction_on_insert(), doc! { "times": 1 }).await
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
        eviction_on_insert(),
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
