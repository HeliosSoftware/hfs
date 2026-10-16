//! #1586: a transaction bundle that the server aborts with a
//! `TransientTransactionError` (`WriteConflict` and its kin) is re-run by the
//! backend instead of failing the whole request with a 400.
//!
//! Child module of the `mongodb_tests` root — `use super::*;` reaches its
//! private harness (`create_tenant`, `create_backend_with_app_name`,
//! `count_docs`, `transactions_required`, plus `Document`/`doc`/`json`/
//! `FhirVersion`, all imported at the test-crate root), the same arrangement as
//! `tests/mongodb/reindex_pipeline.rs`.
//!
//! Every test drives the server's `failCommand` failpoint, scoped to the
//! backend's `appName`, through a bundle of one Patient and one Observation that
//! references it by `urn:uuid`. `create_backend_with_app_name` forces
//! `retryWrites=false`; retryable writes do not apply inside a transaction, so
//! this only keeps the driver's own commit retry from muddying the counts.
//!
//! The 365 tests use the default admission room (4,000 entries), where the
//! 2-entry Bundle shares the room and does not run alone, so they are the
//! evidence that a shared Bundle's eviction rollback is replayed; they must keep
//! the default gate. The lone-Bundle tests at the end of the file (#1895) use a
//! 1 x 2 gate, where the same Bundle holds the whole room.

use super::*;

use crate::bulk_submit::FailPoint;

const PATIENT_URN: &str = "urn:uuid:txn-retry-patient";

/// A Patient created under [`PATIENT_URN`], then an Observation whose
/// `subject` is that urn — so the second entry's reference is rewritten to the
/// id the backend assigned to the first (the state a replay must not reuse).
fn patient_and_observation() -> Vec<BundleEntry> {
    vec![
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "name": [{"family": "TxnRetry"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            criteria: None,
            full_url: Some(PATIENT_URN.to_string()),
        },
        BundleEntry {
            method: BundleMethod::Post,
            url: "Observation".to_string(),
            resource: Some(json!({
                "resourceType": "Observation",
                "status": "final",
                "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
                "subject": {"reference": PATIENT_URN}
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            criteria: None,
            full_url: Some("urn:uuid:txn-retry-observation".to_string()),
        },
    ]
}

/// The failpoint's `data` for a `failCommand` that fails `command` with the
/// labelled transient `WriteConflict` the server raises for a lost race.
fn write_conflict_on(command: &str) -> Document {
    doc! {
        "failCommands": [command],
        "errorCode": 112,
        "errorLabels": ["TransientTransactionError"],
    }
}

/// The failpoint's `data` for a `commitTransaction` that *applies* and then
/// reports a write-concern failure (`WriteConcernFailed`, 64) — which the
/// driver labels `UnknownTransactionCommitResult`: the commit may have landed.
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

/// Like [`commit_with_unknown_result`], but with a write-concern error code
/// (`UnknownReplWriteConcern`, 79) outside the driver's
/// `UnknownTransactionCommitResult` set `{50, 64, 91}` and not a retryable-write
/// code either, so the driver puts no label on the error at all. The commit
/// still applied: a write-concern error is reported *after* the write, so it
/// is just as unknown as the labelled kind.
fn commit_with_unlabelled_write_concern_error() -> Document {
    doc! {
        "failCommands": ["commitTransaction"],
        "writeConcernError": {
            "code": 79,
            "codeName": "UnknownReplWriteConcern",
            "errmsg": "No write concern mode named 'bogus' found in replica set configuration",
        },
    }
}

/// True when the topology cannot run transactions and the test must skip
/// (only possible against an external standalone `HFS_TEST_MONGODB_URL`); a
/// harness-owned replica set that reports it is a failure, as everywhere else.
fn topology_lacks_transactions(result: &Result<BundleResult, TransactionError>) -> bool {
    if !matches!(
        result,
        Err(TransactionError::UnsupportedIsolationLevel { .. })
    ) {
        return false;
    }
    assert!(
        !transactions_required(),
        "the harness's own Mongo container is a replica set and must support transactions"
    );
    eprintln!("Skipping (MongoDB topology does not support transactions)");
    true
}

/// `(resources, history)` document counts for `tenant_id`.
async fn stored_counts(backend: &MongoBackend, tenant_id: &str) -> (u64, u64) {
    let filter = doc! { "tenant_id": tenant_id };
    (
        count_docs(backend, "resources", filter.clone()).await,
        count_docs(backend, "resource_history", filter).await,
    )
}

/// The lone-Bundle tests below run on a 1 x 2 gate, where the default
/// threshold for deferring a Bundle's search rows (#1975) is the gate's room of
/// two entries, so their 2-entry Bundle takes the deferred path. Whatever the
/// outcome, nothing deferred may be left behind: an aborted attempt takes its
/// ledger document and its `search_rows_pending` markers with it, and a
/// committed Bundle clears both after writing its rows.
async fn assert_nothing_deferred_left(backend: &MongoBackend, tenant_id: &str) {
    assert_eq!(
        super::deferred_search_index::ledger_count(backend).await,
        0,
        "no search_index_pending ledger document is left"
    );
    assert_eq!(
        super::deferred_search_index::marker_count(backend, tenant_id).await,
        0,
        "no resource carries search_rows_pending"
    );
}

/// The `Patient` and `Observation` rows the bundle left behind, read straight
/// from the resources collection (not through the backend, so no failpoint and
/// no cache can colour the answer).
async fn stored_resources(
    backend: &MongoBackend,
    tenant_id: &str,
    resource_type: &str,
) -> Vec<Document> {
    use futures::TryStreamExt;
    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("raw client for transaction_retry assertions");
    client
        .database(&backend.config().database_name)
        .collection::<Document>("resources")
        .find(doc! { "tenant_id": tenant_id, "resource_type": resource_type })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

/// The server aborts the transaction with a `TransientTransactionError` at the
/// first write. The backend re-runs the bundle and the request succeeds: both
/// resources stored once, both with a history row, and the failpoint fired
/// exactly once — the second run was clean.
///
/// Before #1586 the labelled error was flattened into `BackendError::Internal`
/// and reached the client as `BundleError` -> 400 `processing`.
#[tokio::test]
async fn a_transient_abort_on_insert_is_replayed_and_the_bundle_commits() {
    let app = "fp-txn-retry-insert";
    let Some(backend) = create_backend_with_app_name("txn_retry_insert", app).await else {
        return;
    };
    let tenant = create_tenant("txn-retry-insert");
    let Some(fail_point) =
        FailPoint::enable(app, write_conflict_on("insert"), doc! { "times": 1 }).await
    else {
        return;
    };

    let result = backend
        .process_transaction(&tenant, patient_and_observation(), FhirVersion::default())
        .await;
    let entered = fail_point.off_and_count().await;
    if topology_lacks_transactions(&result) {
        return;
    }

    let bundle = result.expect("a transient abort is replayed; the bundle must commit");
    assert_eq!(bundle.entries.len(), 2);
    assert!(bundle.entries.iter().all(|entry| entry.status == 201));
    assert_eq!(entered, 1, "the failpoint fires once; the replay is clean");
    assert_eq!(
        stored_counts(&backend, "txn-retry-insert").await,
        (2, 2),
        "one Patient and one Observation, each with one history row"
    );
}

/// The transaction is aborted at *commit*, after every entry ran. By then the
/// Observation's `urn:uuid` reference has been rewritten to the first attempt's
/// Patient id, so a replay that reuses the mutated entries would point the
/// Observation at a Patient that was rolled back. The Observation must
/// reference the Patient that was actually committed.
#[tokio::test]
async fn a_transient_abort_on_commit_replays_from_the_original_entries() {
    let app = "fp-txn-retry-commit";
    let Some(backend) = create_backend_with_app_name("txn_retry_commit", app).await else {
        return;
    };
    let tenant = create_tenant("txn-retry-commit");
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
        .process_transaction(&tenant, patient_and_observation(), FhirVersion::default())
        .await;
    let entered = fail_point.off_and_count().await;
    if topology_lacks_transactions(&result) {
        return;
    }

    let bundle = result.expect("a transient abort at commit is replayed; the bundle must commit");
    assert_eq!(entered, 1);
    assert_eq!(stored_counts(&backend, "txn-retry-commit").await, (2, 2));

    let patients = stored_resources(&backend, "txn-retry-commit", "Patient").await;
    let observations = stored_resources(&backend, "txn-retry-commit", "Observation").await;
    assert_eq!(patients.len(), 1);
    assert_eq!(observations.len(), 1);
    let patient_id = patients[0].get_str("id").unwrap();
    let subject = observations[0]
        .get_document("data")
        .unwrap()
        .get_document("subject")
        .unwrap()
        .get_str("reference")
        .unwrap();
    assert_eq!(
        subject,
        format!("Patient/{patient_id}"),
        "the Observation must reference the committed Patient, not the rolled-back one"
    );

    // The result the caller sees agrees with what was stored.
    let location = bundle.entries[0].location.as_deref().unwrap();
    assert_eq!(extract_resource_id_from_location(location), patient_id);
}

/// A server that keeps aborting the transaction is not retried forever: after
/// the policy's attempts the request ends as `TransactionError::Transient`
/// (a retryable 503 for the client), nothing is committed, and the failpoint
/// fired once per attempt.
#[tokio::test]
async fn a_transaction_that_keeps_aborting_gives_up_as_transient() {
    let app = "fp-txn-retry-exhausted";
    let Some(backend) = create_backend_with_app_name("txn_retry_exhausted", app).await else {
        return;
    };
    let tenant = create_tenant("txn-retry-exhausted");
    let Some(fail_point) =
        FailPoint::enable(app, write_conflict_on("insert"), doc! { "times": 100 }).await
    else {
        return;
    };

    let result = backend
        .process_transaction(&tenant, patient_and_observation(), FhirVersion::default())
        .await;
    let entered = fail_point.off_and_count().await;
    if topology_lacks_transactions(&result) {
        return;
    }

    match result {
        Err(TransactionError::Transient { attempts, reason }) => {
            assert_eq!(attempts, 3, "the bundle policy allows three attempts");
            assert!(
                reason.contains("112") || reason.contains("WriteConflict"),
                "the log-only reason keeps the driver detail: {reason}"
            );
        }
        other => panic!("expected TransactionError::Transient, got {other:?}"),
    }
    assert_eq!(entered, 3, "one failpoint hit per attempt");
    assert_eq!(
        stored_counts(&backend, "txn-retry-exhausted").await,
        (0, 0),
        "an exhausted transaction leaves nothing behind"
    );
}

/// Regression guard: an error with no `TransientTransactionError` label is a
/// real failure and is not retried — the request still ends as a `BundleError`
/// naming the entry, with the failpoint hit exactly once.
#[tokio::test]
async fn a_non_transient_error_is_not_retried() {
    let app = "fp-txn-retry-nonlabelled";
    let Some(backend) = create_backend_with_app_name("txn_retry_nonlabelled", app).await else {
        return;
    };
    let tenant = create_tenant("txn-retry-nonlabelled");
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! { "failCommands": ["insert"], "errorCode": 2 },
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .process_transaction(&tenant, patient_and_observation(), FhirVersion::default())
        .await;
    let entered = fail_point.off_and_count().await;
    if topology_lacks_transactions(&result) {
        return;
    }

    match result {
        Err(TransactionError::BundleError { index, .. }) => assert_eq!(index, 0),
        other => panic!("expected TransactionError::BundleError, got {other:?}"),
    }
    assert_eq!(entered, 1, "an unlabelled error is not retried");
    assert_eq!(
        stored_counts(&backend, "txn-retry-nonlabelled").await,
        (0, 0)
    );
}

/// #1837: a transaction whose own write set does not fit WiredTiger's cache
/// fails with `TransactionTooLargeForCache` (388). No replay can fit it, so the
/// bundle ends at once as `TransactionError::TooLargeForCache`, not the
/// `BundleError` (400) it was, with the failpoint hit exactly once and nothing
/// stored. The server sends 388 unlabelled. The labelled run pins that the code
/// outranks a `TransientTransactionError` label.
#[tokio::test]
async fn a_transaction_too_large_for_cache_is_not_retried() {
    let app = "fp-txn-too-large-for-cache";
    let Some(backend) = create_backend_with_app_name("txn_too_large_for_cache", app).await else {
        return;
    };
    let tenant = create_tenant("txn-too-large-for-cache");
    for data in [
        doc! { "failCommands": ["insert"], "errorCode": 388 },
        doc! { "failCommands": ["insert"], "errorCode": 388, "errorLabels": ["TransientTransactionError"] },
    ] {
        let Some(fail_point) = FailPoint::enable(app, data, doc! { "times": 100 }).await else {
            return;
        };
        let result = backend
            .process_transaction(&tenant, patient_and_observation(), FhirVersion::default())
            .await;
        let entered = fail_point.off_and_count().await;
        if topology_lacks_transactions(&result) {
            return;
        }
        match result {
            Err(TransactionError::TooLargeForCache { entries, reason }) => {
                assert_eq!(entries, 2);
                assert!(
                    reason.contains("388"),
                    "the log-only reason keeps the driver detail: {reason}"
                );
            }
            other => panic!("expected TransactionError::TooLargeForCache, got {other:?}"),
        }
        assert_eq!(
            entered, 1,
            "a transaction too large for the cache is not replayed"
        );
        assert_eq!(
            stored_counts(&backend, "txn-too-large-for-cache").await,
            (0, 0)
        );
    }
}

/// The commit is applied but its acknowledgement is not (`UnknownTransactionCommitResult`).
/// Re-running the entries would apply the bundle twice — every POST gets a
/// fresh id, so nothing would collide — so only the *commit* is retried.
/// Exactly one Patient and one Observation must exist afterwards.
#[tokio::test]
async fn an_unknown_commit_result_retries_the_commit_not_the_entries() {
    let app = "fp-txn-retry-unknown-commit";
    let Some(backend) = create_backend_with_app_name("txn_retry_unknown_commit", app).await else {
        return;
    };
    let tenant = create_tenant("txn-retry-unknown-commit");
    let Some(fail_point) =
        FailPoint::enable(app, commit_with_unknown_result(), doc! { "times": 1 }).await
    else {
        return;
    };

    let result = backend
        .process_transaction(&tenant, patient_and_observation(), FhirVersion::default())
        .await;
    let entered = fail_point.off_and_count().await;
    if topology_lacks_transactions(&result) {
        return;
    }

    let bundle = result.expect("the commit is retried and acknowledged");
    assert_eq!(bundle.entries.len(), 2);
    assert_eq!(entered, 1);
    assert_eq!(
        stored_counts(&backend, "txn-retry-unknown-commit").await,
        (2, 2),
        "a replay of the entries would have stored the bundle twice"
    );
}

/// When the commit's outcome stays unknown through every commit retry, the
/// answer is `CommitOutcomeUnknown` — not `RolledBack`, which would tell the
/// client nothing was applied when the commit did land — and the entries are
/// still not re-run.
#[tokio::test]
async fn a_commit_that_stays_unknown_is_reported_as_unknown_not_rolled_back() {
    let app = "fp-txn-retry-unknown-exhausted";
    let Some(backend) = create_backend_with_app_name("txn_retry_unknown_exhausted", app).await
    else {
        return;
    };
    let tenant = create_tenant("txn-retry-unknown-exhausted");
    let Some(fail_point) =
        FailPoint::enable(app, commit_with_unknown_result(), doc! { "times": 100 }).await
    else {
        return;
    };

    let result = backend
        .process_transaction(&tenant, patient_and_observation(), FhirVersion::default())
        .await;
    let entered = fail_point.off_and_count().await;
    if topology_lacks_transactions(&result) {
        return;
    }

    match result {
        Err(TransactionError::CommitOutcomeUnknown { reason }) => {
            assert!(
                !reason.is_empty(),
                "the log-only reason carries the driver detail"
            );
        }
        other => panic!("expected TransactionError::CommitOutcomeUnknown, got {other:?}"),
    }
    assert_eq!(entered, 3, "the commit runs once and is retried twice");
    assert_eq!(
        stored_counts(&backend, "txn-retry-unknown-exhausted").await,
        (2, 2),
        "the first commit applied; the entries were not re-run"
    );
}

/// A write-concern error on the commit says the commit *applied* but was not
/// acknowledged as the write concern demands — whatever its code or label. Code
/// 79 is outside the driver's `UnknownTransactionCommitResult` set, so the driver
/// leaves the error unlabelled; the backend used to read an unlabelled commit
/// failure as "rolled back" and report that to a client whose bundle was in fact
/// stored. It must take the unknown-result path instead: retry the commit only,
/// and never re-run the entries (which would store every POST a second time).
#[tokio::test]
async fn an_unlabelled_write_concern_error_on_commit_retries_the_commit_not_the_entries() {
    let app = "fp-txn-retry-wc79";
    let Some(backend) = create_backend_with_app_name("txn_retry_wc79", app).await else {
        return;
    };
    let tenant = create_tenant("txn-retry-wc79");
    let Some(fail_point) = FailPoint::enable(
        app,
        commit_with_unlabelled_write_concern_error(),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .process_transaction(&tenant, patient_and_observation(), FhirVersion::default())
        .await;
    let entered = fail_point.off_and_count().await;
    if topology_lacks_transactions(&result) {
        return;
    }

    let bundle = result.expect("the commit is retried and acknowledged");
    assert_eq!(bundle.entries.len(), 2);
    assert_eq!(entered, 1, "the failpoint fires once; the retry is clean");
    assert_eq!(
        stored_counts(&backend, "txn-retry-wc79").await,
        (2, 2),
        "each resource exactly once: a replay of the entries would store the bundle twice"
    );
    assert_eq!(
        stored_resources(&backend, "txn-retry-wc79", "Patient")
            .await
            .len(),
        1
    );
    assert_eq!(
        stored_resources(&backend, "txn-retry-wc79", "Observation")
            .await
            .len(),
        1
    );
}

/// The same write-concern error on every commit: the answer is
/// `CommitOutcomeUnknown`, never `RolledBack` — the bundle was stored — and the
/// entries are still not re-run.
#[tokio::test]
async fn an_unlabelled_write_concern_error_that_persists_is_reported_as_unknown_not_rolled_back() {
    let app = "fp-txn-retry-wc79-exhausted";
    let Some(backend) = create_backend_with_app_name("txn_retry_wc79_exhausted", app).await else {
        return;
    };
    let tenant = create_tenant("txn-retry-wc79-exhausted");
    let Some(fail_point) = FailPoint::enable(
        app,
        commit_with_unlabelled_write_concern_error(),
        doc! { "times": 100 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .process_transaction(&tenant, patient_and_observation(), FhirVersion::default())
        .await;
    let entered = fail_point.off_and_count().await;
    if topology_lacks_transactions(&result) {
        return;
    }

    match result {
        Err(TransactionError::CommitOutcomeUnknown { reason }) => {
            assert!(
                !reason.is_empty(),
                "the log-only reason carries the driver detail"
            );
        }
        other => panic!("expected TransactionError::CommitOutcomeUnknown, got {other:?}"),
    }
    assert_eq!(entered, 3, "the commit runs once and is retried twice");
    assert_eq!(
        stored_counts(&backend, "txn-retry-wc79-exhausted").await,
        (2, 2),
        "the first commit applied; the entries were not re-run"
    );
}

/// The failpoint's `data` for a `failCommand` that fails `command` with code 365
/// (`TemporarilyUnavailable`) and, unlike [`write_conflict_on`], **no**
/// `errorLabels`. This is the eviction rollback of #1700: under WiredTiger cache
/// pressure the server rolled the commit back with "oldest pinned transaction ID
/// rolled back for eviction" and sent no `TransientTransactionError` label.
fn temporarily_unavailable_on(command: &str) -> Document {
    doc! {
        "failCommands": [command],
        "errorCode": 365,
    }
}

/// #1700: the commit is rolled back with an unlabelled code 365. Nothing
/// committed, so the backend replays the bundle from its original entries and
/// the request succeeds, with each resource stored exactly once.
///
/// Before the fix the unlabelled commit failure read as a plain rollback and
/// reached the client as a 500.
#[tokio::test]
async fn an_unlabelled_code_365_commit_failure_is_replayed_and_the_bundle_commits() {
    let app = "fp-txn-retry-commit-365";
    let Some(backend) = create_backend_with_app_name("txn_retry_commit_365", app).await else {
        return;
    };
    let tenant = create_tenant("txn-retry-commit-365");
    let Some(fail_point) = FailPoint::enable(
        app,
        temporarily_unavailable_on("commitTransaction"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .process_transaction(&tenant, patient_and_observation(), FhirVersion::default())
        .await;
    let entered = fail_point.off_and_count().await;
    if topology_lacks_transactions(&result) {
        return;
    }

    let bundle = result.expect("a code 365 commit failure is replayed; the bundle must commit");
    assert_eq!(bundle.entries.len(), 2);
    assert!(bundle.entries.iter().all(|entry| entry.status == 201));
    assert_eq!(entered, 1, "the failpoint fires once; the replay is clean");
    assert_eq!(
        stored_counts(&backend, "txn-retry-commit-365").await,
        (2, 2),
        "one Patient and one Observation, each with one history row"
    );
    assert_eq!(
        stored_resources(&backend, "txn-retry-commit-365", "Patient")
            .await
            .len(),
        1
    );
    assert_eq!(
        stored_resources(&backend, "txn-retry-commit-365", "Observation")
            .await
            .len(),
        1
    );
}

/// #1700: a server that keeps rolling the commit back with code 365 is not
/// retried forever. After the policy's attempts the request ends as
/// `TransactionError::Transient` (503 + Retry-After), nothing is committed, and
/// the failpoint fired once per attempt.
#[tokio::test]
async fn a_code_365_commit_failure_that_persists_gives_up_as_transient() {
    let app = "fp-txn-retry-commit-365-exhausted";
    let Some(backend) = create_backend_with_app_name("txn_retry_commit_365_exhausted", app).await
    else {
        return;
    };
    let tenant = create_tenant("txn-retry-commit-365-exhausted");
    let Some(fail_point) = FailPoint::enable(
        app,
        temporarily_unavailable_on("commitTransaction"),
        doc! { "times": 100 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .process_transaction(&tenant, patient_and_observation(), FhirVersion::default())
        .await;
    let entered = fail_point.off_and_count().await;
    if topology_lacks_transactions(&result) {
        return;
    }

    match result {
        Err(TransactionError::Transient { attempts, reason }) => {
            assert_eq!(attempts, 3, "the bundle policy allows three attempts");
            assert!(
                reason.contains("365") || reason.contains("TemporarilyUnavailable"),
                "the log-only reason keeps the driver detail: {reason}"
            );
        }
        other => panic!("expected TransactionError::Transient, got {other:?}"),
    }
    assert_eq!(entered, 3, "one failpoint hit per attempt");
    assert_eq!(
        stored_counts(&backend, "txn-retry-commit-365-exhausted").await,
        (0, 0),
        "an exhausted transaction leaves nothing behind"
    );
}

/// #1700, entry side: an entry's write failing with an unlabelled code 365 did
/// not commit either, so the bundle is replayed the same way. The server may or
/// may not label this code on an entry's reply; the backend must not depend on
/// it.
#[tokio::test]
async fn an_unlabelled_code_365_on_an_entry_write_is_replayed_and_the_bundle_commits() {
    let app = "fp-txn-retry-insert-365";
    let Some(backend) = create_backend_with_app_name("txn_retry_insert_365", app).await else {
        return;
    };
    let tenant = create_tenant("txn-retry-insert-365");
    let Some(fail_point) = FailPoint::enable(
        app,
        temporarily_unavailable_on("insert"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .process_transaction(&tenant, patient_and_observation(), FhirVersion::default())
        .await;
    let entered = fail_point.off_and_count().await;
    if topology_lacks_transactions(&result) {
        return;
    }

    let bundle = result.expect("a code 365 entry failure is replayed; the bundle must commit");
    assert_eq!(bundle.entries.len(), 2);
    assert_eq!(entered, 1, "the failpoint fires once; the replay is clean");
    assert_eq!(
        stored_counts(&backend, "txn-retry-insert-365").await,
        (2, 2),
        "one Patient and one Observation, each with one history row"
    );
}

/// A backend like [`create_backend_with_app_name`] with an admission room of
/// `limit` standard Bundles of `weight_entries` entries each (#1895). With
/// limit 1 and weight 2, [`patient_and_observation`]'s two entries hold the
/// whole room. `configure` adjusts the config just before the backend is built.
async fn create_backend_with_app_name_and_gate(
    test_name: &str,
    app_name: &str,
    limit: usize,
    weight_entries: usize,
    configure: impl FnOnce(&mut MongoBackendConfig),
) -> Option<MongoBackend> {
    let connection_string = shared_mongo::connection_string().await?;
    let connection_string = append_query_param(&connection_string, "retryWrites=false");
    let mut config = MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name(test_name),
        app_name: app_name.to_string(),
        data_dir: Some(repo_data_dir()),
        max_concurrent_transaction_bundles: limit,
        transaction_bundle_weight_entries: weight_entries,
        ..Default::default()
    };
    configure(&mut config);
    build_backend(config).await
}

/// A scoped (not global) subscriber for one test that records the backend's
/// WARN lines into a fresh buffer, so the #1895 tests can assert on what the
/// replay loop logged. `set_default` is thread-local and never replaces the
/// process-wide subscriber, which `capture_walk_logs` owns; that is safe here
/// because `#[tokio::test]` is current-thread and the bundle loop emits its
/// events on the test thread. Keep the guard alive until the log assertions
/// are done.
fn capture_bundle_warnings() -> (
    &'static std::sync::Mutex<Vec<u8>>,
    tracing::dispatcher::DefaultGuard,
) {
    let buf: &'static std::sync::Mutex<Vec<u8>> =
        Box::leak(Box::new(std::sync::Mutex::new(Vec::new())));
    let writer = tracing_test::internal::MockWriter::new(buf);
    let dispatch = tracing_test::internal::get_subscriber(
        writer,
        "helios_persistence::backends::mongodb::storage=warn",
    );
    let guard = tracing::dispatcher::set_default(&dispatch);
    (buf, guard)
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

/// The WARN the replay loop logs when it ends a lone eviction rollback.
const LONE_NOT_RETRYING: &str = "while running alone; not retrying:";

/// The WARN the replay loop logs before it replays a Bundle.
const TRANSIENT_RETRYING: &str = "aborted by a transient mongodb error; retrying:";

/// #1895: a Bundle that holds the whole admission room ran alone, so a
/// cache-pressure rollback at commit (code 365) is its own write set meeting the
/// cache and a replay would meet it again. It ends after one attempt as
/// `TooLargeForCache`, the outcome error 388 already has, and frees the room.
#[tokio::test]
async fn a_lone_bundle_rolled_back_for_eviction_at_commit_is_not_replayed() {
    let app = "fp-txn-lone-evict-commit";
    let Some(backend) =
        create_backend_with_app_name_and_gate("txn_lone_evict_commit", app, 1, 2, |_| {}).await
    else {
        return;
    };
    let tenant = create_tenant("txn-lone-evict-commit");
    let Some(fail_point) = FailPoint::enable(
        app,
        temporarily_unavailable_on("commitTransaction"),
        doc! { "times": 100 },
    )
    .await
    else {
        return;
    };

    let (logs, _guard) = capture_bundle_warnings();
    let result = backend
        .process_transaction(&tenant, patient_and_observation(), FhirVersion::default())
        .await;
    let entered = fail_point.off_and_count().await;
    if topology_lacks_transactions(&result) {
        return;
    }

    match result {
        Err(TransactionError::TooLargeForCache { entries, reason }) => {
            assert_eq!(entries, 2);
            assert!(
                reason.contains("365") || reason.contains("TemporarilyUnavailable"),
                "the log-only reason keeps the driver detail: {reason}"
            );
            assert!(
                reason.starts_with(
                    "Commit rolled back for eviction while the Bundle ran alone; not replayed: Commit failed: "
                ),
                "the reason names the commit and says the Bundle ran alone: {reason}"
            );
        }
        other => panic!("expected TransactionError::TooLargeForCache, got {other:?}"),
    }
    assert_eq!(entered, 1, "a lone eviction rollback is not replayed");
    let lone = captured_lines(logs, LONE_NOT_RETRYING);
    assert_eq!(lone.len(), 1, "one not-retrying WARN: {lone:?}");
    for field in ["lone=true", "attempt=1", "entries=2", "error_code=365"] {
        assert!(lone[0].contains(field), "missing {field}: {}", lone[0]);
    }
    assert!(
        !lone[0].contains("failed_entry="),
        "a commit-time rollback has no failed entry: {}",
        lone[0]
    );
    let retries = captured_lines(logs, TRANSIENT_RETRYING);
    assert!(retries.is_empty(), "no retrying WARN: {retries:?}");
    assert_eq!(
        stored_counts(&backend, "txn-lone-evict-commit").await,
        (0, 0),
        "a rolled-back transaction leaves nothing behind"
    );
    assert_nothing_deferred_left(&backend, "txn-lone-evict-commit").await;
}

/// #1895, entry side: the same, with the rollback arriving on an entry's write.
#[tokio::test]
async fn a_lone_bundle_rolled_back_for_eviction_on_an_entry_write_is_not_replayed() {
    let app = "fp-txn-lone-evict-insert";
    let Some(backend) =
        create_backend_with_app_name_and_gate("txn_lone_evict_insert", app, 1, 2, |_| {}).await
    else {
        return;
    };
    let tenant = create_tenant("txn-lone-evict-insert");
    let Some(fail_point) = FailPoint::enable(
        app,
        temporarily_unavailable_on("insert"),
        doc! { "times": 100 },
    )
    .await
    else {
        return;
    };

    let (logs, _guard) = capture_bundle_warnings();
    let result = backend
        .process_transaction(&tenant, patient_and_observation(), FhirVersion::default())
        .await;
    let entered = fail_point.off_and_count().await;
    if topology_lacks_transactions(&result) {
        return;
    }

    match result {
        Err(TransactionError::TooLargeForCache { entries, reason }) => {
            assert_eq!(entries, 2);
            assert!(
                reason.contains("365") || reason.contains("TemporarilyUnavailable"),
                "the log-only reason keeps the driver detail: {reason}"
            );
            assert!(
                reason.starts_with("Entry "),
                "the reason names the failed entry: {reason}"
            );
            assert!(
                reason.contains(
                    " rolled back for eviction while the Bundle ran alone; not replayed: Entry processing failed: "
                ),
                "the reason says the Bundle ran alone: {reason}"
            );
        }
        other => panic!("expected TransactionError::TooLargeForCache, got {other:?}"),
    }
    assert_eq!(entered, 1, "a lone eviction rollback is not replayed");
    let lone = captured_lines(logs, LONE_NOT_RETRYING);
    assert_eq!(lone.len(), 1, "one not-retrying WARN: {lone:?}");
    for field in [
        "lone=true",
        "attempt=1",
        "entries=2",
        "error_code=365",
        "failed_entry=",
    ] {
        assert!(lone[0].contains(field), "missing {field}: {}", lone[0]);
    }
    let retries = captured_lines(logs, TRANSIENT_RETRYING);
    assert!(retries.is_empty(), "no retrying WARN: {retries:?}");
    assert_eq!(
        stored_counts(&backend, "txn-lone-evict-insert").await,
        (0, 0),
        "a rolled-back transaction leaves nothing behind"
    );
    assert_nothing_deferred_left(&backend, "txn-lone-evict-insert").await;
}

/// #1895: only the eviction rollback is not replayed. A lone Bundle that keeps
/// losing a plain `WriteConflict` (the failpoint's message names no eviction)
/// is a lost race, which a replay can win, so it is still replayed up to the
/// policy's three attempts.
#[tokio::test]
async fn a_lone_bundle_that_keeps_losing_plain_write_conflicts_is_still_replayed() {
    let app = "fp-txn-lone-conflict";
    let Some(backend) =
        create_backend_with_app_name_and_gate("txn_lone_conflict", app, 1, 2, |_| {}).await
    else {
        return;
    };
    let tenant = create_tenant("txn-lone-conflict");
    let Some(fail_point) =
        FailPoint::enable(app, write_conflict_on("insert"), doc! { "times": 100 }).await
    else {
        return;
    };

    let (logs, _guard) = capture_bundle_warnings();
    let result = backend
        .process_transaction(&tenant, patient_and_observation(), FhirVersion::default())
        .await;
    let entered = fail_point.off_and_count().await;
    if topology_lacks_transactions(&result) {
        return;
    }

    match result {
        Err(TransactionError::Transient { attempts, .. }) => {
            assert_eq!(attempts, 3, "the bundle policy allows three attempts");
        }
        other => panic!("expected TransactionError::Transient, got {other:?}"),
    }
    assert_eq!(entered, 3, "one failpoint hit per attempt");
    let lone = captured_lines(logs, LONE_NOT_RETRYING);
    assert!(lone.is_empty(), "no not-retrying WARN: {lone:?}");
    let retries = captured_lines(logs, TRANSIENT_RETRYING);
    assert_eq!(
        retries.len(),
        2,
        "two replays between three attempts: {retries:?}"
    );
    for line in &retries {
        assert!(line.contains("error_code=112"), "{line}");
    }
    assert_eq!(
        stored_counts(&backend, "txn-lone-conflict").await,
        (0, 0),
        "an exhausted transaction leaves nothing behind"
    );
    assert_nothing_deferred_left(&backend, "txn-lone-conflict").await;
}

/// #1895: the lone check runs before `next_attempt_delay`, so a lone eviction
/// rollback ends as `TooLargeForCache` even when the replay budget is already
/// spent. With the check after the delay, a zero budget would end the Bundle as
/// `Transient { attempts: 1 }`.
#[tokio::test]
async fn a_lone_eviction_with_no_replay_budget_left_still_ends_as_too_large_for_cache() {
    let app = "fp-txn-lone-evict-nobudget";
    let Some(backend) =
        create_backend_with_app_name_and_gate("txn_lone_evict_nobudget", app, 1, 2, |c| {
            c.bundle_transaction_budget = std::time::Duration::ZERO;
        })
        .await
    else {
        return;
    };
    let tenant = create_tenant("txn-lone-evict-nobudget");
    let Some(fail_point) = FailPoint::enable(
        app,
        temporarily_unavailable_on("commitTransaction"),
        doc! { "times": 100 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .process_transaction(&tenant, patient_and_observation(), FhirVersion::default())
        .await;
    let entered = fail_point.off_and_count().await;
    if topology_lacks_transactions(&result) {
        return;
    }

    match result {
        Err(TransactionError::TooLargeForCache { entries, .. }) => assert_eq!(entries, 2),
        other => panic!("expected TransactionError::TooLargeForCache, got {other:?}"),
    }
    assert_eq!(entered, 1, "a lone eviction rollback is not replayed");
    assert_eq!(
        stored_counts(&backend, "txn-lone-evict-nobudget").await,
        (0, 0),
        "a rolled-back transaction leaves nothing behind"
    );
    assert_nothing_deferred_left(&backend, "txn-lone-evict-nobudget").await;
}
