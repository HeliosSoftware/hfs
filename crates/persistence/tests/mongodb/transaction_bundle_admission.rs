//! #1776: transaction Bundles beyond `max_concurrent_transaction_bundles` wait
//! for an admission slot instead of each opening a transaction at once, so the
//! server never holds more than the limit's worth of open bundle transactions
//! from one backend (and WiredTiger's cache is not exhausted by many large
//! uncommitted write sets).
//!
//! #1806: the gate's room is counted in entries, not Bundles. One standard
//! Bundle is `transaction_bundle_weight_entries` entries and a Bundle takes as
//! many entries' worth of room as it carries, so small Bundles share a slot and
//! large ones take more than one. The tests pin the weight explicitly because
//! the backend default (1000) is far above the 60-entry Bundles used here.
//!
//! Child module of the `mongodb_tests` root — `use super::*;` reaches its
//! private harness (`create_tenant`, `build_backend`, `count_docs`,
//! `raw_test_client`, `transactions_required`, plus `BundleEntry`/`doc`/`json`/
//! `FhirVersion`), the same arrangement as `tests/mongodb/transaction_retry.rs`.
//!
//! The open transactions are counted from the server's side: `$currentOp` lists
//! a transaction that is open between operations as an `idleSession` carrying
//! the session's `appName` and a `transaction` sub-document. `$currentOp` also
//! lists a retryable write on an implicit session with a `transaction`
//! sub-document that has no `parameters.autocommit`, so the sampler counts only
//! sessions whose `transaction.parameters.autocommit` is `false` (a
//! multi-document transaction). The background upsert of the `missing_params`
//! fingerprint catalog is such a write; since #1975 it can run while a deferred
//! Bundle's post-commit batch transaction is open. Each test gives its
//! backend a unique `appName`, because `$currentOp` is server-global and the
//! suite's tests share one container and run in parallel.
//!
//! A POST-only Bundle stages its creates and sends them in chunks (#1977), so
//! its server transaction opens at the first chunk and a small Bundle would be
//! open for a few milliseconds, too short for the sampler below. Each test holds
//! the Bundles' transactions open by construction instead: a `blockConnection`
//! failpoint, scoped to the test's `appName`, delays every `insert` by
//! [`INSERT_HOLD_MS`]. A Bundle sends at least three (`resources`,
//! `resource_history`, its search rows), so its transaction is open for about
//! three times that whatever the staging speed. The failpoint is server-global
//! and the harness serialises its users with a suite-wide lock, so these tests
//! also run one at a time with each other and with the other failpoint tests.

use super::*;

use std::sync::atomic::{AtomicBool, Ordering};

use crate::bulk_submit::FailPoint;

/// Entries per bundle: enough that every bundle writes real search rows. How
/// long its transaction stays open is not left to its size: see
/// [`hold_inserts_open`].
const BUNDLE_ENTRIES: usize = 60;

/// How long the failpoint of [`hold_inserts_open`] delays each `insert`.
const INSERT_HOLD_MS: i64 = 50;

/// A backend with `limit` standard Bundles of `weight_entries` entries each as
/// its admission room, under a unique `appName`. `None` when the shared Mongo
/// is unavailable, so the caller skips.
async fn backend_with_gate(
    test: &str,
    app: &str,
    limit: usize,
    weight_entries: usize,
) -> Option<MongoBackend> {
    let config = MongoBackendConfig {
        connection_string: shared_mongo::connection_string().await?,
        database_name: build_test_database_name(test),
        app_name: app.to_string(),
        data_dir: Some(repo_data_dir()),
        max_concurrent_transaction_bundles: limit,
        transaction_bundle_weight_entries: weight_entries,
        ..Default::default()
    };
    build_backend(config).await
}

fn unique_app(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
}

/// Delays every `insert` of the connections under `app` by [`INSERT_HOLD_MS`],
/// so each Bundle's transaction stays open across its inserts (see the module
/// doc). `None`, after saying why, when the failpoint is unavailable, so the
/// caller skips. The sampler's client has no `appName`, so it is not delayed.
/// The test holds the guard until `run` returns and then calls `off`.
async fn hold_inserts_open(app: &str) -> Option<FailPoint> {
    FailPoint::enable(
        app,
        doc! {
            "failCommands": ["insert"],
            "blockConnection": true,
            "blockTimeMS": INSERT_HOLD_MS,
        },
        doc! { "times": 1_000_000 },
    )
    .await
}

/// One Patient plus `n - 1` Observations that reference it by `urn:uuid`, each
/// with its own fresh `fullUrl` (no fixed ids, so bundles never collide).
fn patient_with_observations(n: usize) -> Vec<BundleEntry> {
    let patient_urn = format!("urn:uuid:{}", uuid::Uuid::new_v4());
    let post = |url: &str, resource: serde_json::Value, full_url: String| BundleEntry {
        method: BundleMethod::Post,
        url: url.to_string(),
        resource: Some(resource),
        if_match: None,
        if_none_match: None,
        if_none_exist: None,
        criteria: None,
        full_url: Some(full_url),
    };
    let mut entries = vec![post(
        "Patient",
        json!({"resourceType": "Patient", "name": [{"family": "TxnGate"}]}),
        patient_urn.clone(),
    )];
    for i in 1..n {
        entries.push(post(
            "Observation",
            json!({
                "resourceType": "Observation",
                "status": "final",
                "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
                "effectiveDateTime": "2024-01-01T00:00:00Z",
                "valueQuantity": {"value": i, "unit": "bpm"},
                "subject": {"reference": patient_urn}
            }),
            format!("urn:uuid:{}", uuid::Uuid::new_v4()),
        ));
    }
    entries
}

/// How many distinct sessions of `app` currently hold an open multi-document
/// transaction. `$currentOp` also lists a retryable write on an implicit
/// session with a `transaction` sub-document, but that one has no
/// `parameters.autocommit`, so the match below is on
/// `transaction.parameters.autocommit` being `false`, which only a
/// multi-document transaction carries.
async fn open_transactions(client: &Client, app: &str) -> usize {
    use futures::stream::TryStreamExt;

    let pipeline = vec![
        doc! {"$currentOp": {"allUsers": true, "idleSessions": true, "idleConnections": false}},
        doc! {"$match": {"appName": app, "transaction.parameters.autocommit": false}},
        doc! {"$group": {"_id": "$lsid.id"}},
    ];
    let cursor = client
        .database("admin")
        .aggregate(pipeline)
        .await
        .expect("$currentOp aggregate failed");
    cursor.try_collect::<Vec<Document>>().await.unwrap().len()
}

/// Runs `bundles` concurrent transaction bundles against `backend` while a
/// sampler polls the server for `app`'s open transactions. Returns the most
/// transactions seen open at once, and every bundle's result.
async fn run(
    backend: Arc<MongoBackend>,
    tenant: &str,
    bundles: usize,
    app: &str,
) -> (usize, Vec<Result<BundleResult, TransactionError>>) {
    let stop = Arc::new(AtomicBool::new(false));
    let sampler = {
        let stop = stop.clone();
        let app = app.to_string();
        let uri = backend.config().connection_string.clone();
        tokio::spawn(async move {
            let client = raw_test_client(&uri).await.expect("sampler client");
            let mut max_open = 0;
            while !stop.load(Ordering::Relaxed) {
                max_open = max_open.max(open_transactions(&client, &app).await);
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            max_open
        })
    };

    let tasks: Vec<_> = (0..bundles)
        .map(|_| {
            let backend = backend.clone();
            let tenant = create_tenant(tenant);
            tokio::spawn(async move {
                backend
                    .process_transaction(
                        &tenant,
                        patient_with_observations(BUNDLE_ENTRIES),
                        FhirVersion::default(),
                    )
                    .await
            })
        })
        .collect();
    let mut results = Vec::new();
    for task in tasks {
        results.push(task.await.expect("bundle task panicked"));
    }

    stop.store(true, Ordering::Relaxed);
    let max_open = sampler.await.expect("sampler task panicked");
    (max_open, results)
}

/// True when the topology cannot run transactions and the test must skip
/// (only possible against an external standalone `HFS_TEST_MONGODB_URL`); a
/// harness-owned replica set that reports it is a failure, as everywhere else.
fn topology_lacks_transactions(results: &[Result<BundleResult, TransactionError>]) -> bool {
    if !results
        .iter()
        .any(|r| matches!(r, Err(TransactionError::UnsupportedIsolationLevel { .. })))
    {
        return false;
    }
    assert!(
        !transactions_required(),
        "the harness's own Mongo container is a replica set and must support transactions"
    );
    eprintln!("Skipping (MongoDB topology does not support transactions)");
    true
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transaction_bundles_beyond_the_limit_wait_their_turn() {
    const LIMIT: usize = 2;
    const BUNDLES: usize = 6;
    let tenant = "txn-gate-limit";
    let app = unique_app("txn-gate-1776");
    let Some(backend) = backend_with_gate("txn_gate_limit", &app, LIMIT, BUNDLE_ENTRIES).await
    else {
        return;
    };
    let backend = Arc::new(backend);
    let Some(hold) = hold_inserts_open(&app).await else {
        return;
    };

    let (max_open, results) = run(backend.clone(), tenant, BUNDLES, &app).await;
    hold.off().await;
    if topology_lacks_transactions(&results) {
        return;
    }

    // Queued bundles are only delayed: every one still commits in full.
    for result in &results {
        let bundle = result.as_ref().expect("a queued bundle must still commit");
        assert_eq!(bundle.entries.len(), BUNDLE_ENTRIES);
        assert!(bundle.entries.iter().all(|e| e.status == 201));
    }
    assert_eq!(
        count_docs(&backend, "resources", doc! {"tenant_id": tenant}).await,
        (BUNDLES * BUNDLE_ENTRIES) as u64
    );

    assert!(
        max_open <= LIMIT,
        "saw {max_open} open bundle transactions with max_concurrent_transaction_bundles = {LIMIT}"
    );
    // The sampler really observed the transactions, so the bound above is
    // measured and not an artifact of never looking.
    assert!(max_open >= 1, "the sampler never saw an open transaction");
}

/// The control that makes the test above meaningful: the same load with the
/// gate off overlaps beyond the limit the gated test asserts.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_a_limit_the_same_bundles_overlap() {
    const BUNDLES: usize = 6;
    let tenant = "txn-gate-nolimit";
    let app = unique_app("txn-gate-1776-ctl");
    let weight = MongoBackendConfig::default().transaction_bundle_weight_entries;
    let Some(backend) = backend_with_gate("txn_gate_nolimit", &app, 0, weight).await else {
        return;
    };
    let Some(hold) = hold_inserts_open(&app).await else {
        return;
    };

    let (max_open, results) = run(Arc::new(backend), tenant, BUNDLES, &app).await;
    hold.off().await;
    if topology_lacks_transactions(&results) {
        return;
    }

    // The control shows overlap, not success: an ungated Bundle may be rolled
    // back for eviction.
    for result in &results {
        match result {
            Ok(_) | Err(TransactionError::Transient { .. }) => {}
            Err(e) => panic!("an ungated bundle failed unexpectedly: {e:?}"),
        }
    }
    assert!(
        max_open >= 3,
        "expected the ungated bundles to overlap beyond 2, saw at most {max_open}"
    );
}

/// #1806: the gate counts entries, so Bundles smaller than a standard one share
/// its room. With room for six 60-entry Bundles (`LIMIT` x weight), all six run
/// at once; the #1776 count-only gate would have held them to `LIMIT`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bundles_smaller_than_a_standard_one_share_its_room() {
    const LIMIT: usize = 2;
    const BUNDLES: usize = 6;
    let tenant = "txn-gate-small";
    let app = unique_app("txn-gate-1806-small");
    let Some(backend) = backend_with_gate("txn_gate_small", &app, LIMIT, 3 * BUNDLE_ENTRIES).await
    else {
        return;
    };
    let Some(hold) = hold_inserts_open(&app).await else {
        return;
    };

    let (max_open, results) = run(Arc::new(backend), tenant, BUNDLES, &app).await;
    hold.off().await;
    if topology_lacks_transactions(&results) {
        return;
    }

    // Like the control, this shows overlap, not success: the shared container's
    // 0.25 GB cache may roll a Bundle back for eviction.
    for result in &results {
        match result {
            Ok(_) | Err(TransactionError::Transient { .. }) => {}
            Err(e) => panic!("a Bundle failed unexpectedly: {e:?}"),
        }
    }
    assert!(
        max_open >= 3,
        "expected Bundles smaller than a standard one to share its room, saw at most \
         {max_open} open (the count-only gate would have held them to {LIMIT})"
    );
}

/// #1806: a Bundle larger than a standard one takes more than one slot. With a
/// weight of half a Bundle's entries, each 60-entry Bundle weighs the whole
/// room (`LIMIT` x weight), so they run one at a time.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bundle_larger_than_a_standard_one_takes_more_than_one_slot() {
    const LIMIT: usize = 2;
    const BUNDLES: usize = 4;
    let tenant = "txn-gate-large";
    let app = unique_app("txn-gate-1806-large");
    let Some(backend) = backend_with_gate("txn_gate_large", &app, LIMIT, BUNDLE_ENTRIES / 2).await
    else {
        return;
    };
    let backend = Arc::new(backend);
    let Some(hold) = hold_inserts_open(&app).await else {
        return;
    };

    let (max_open, results) = run(backend.clone(), tenant, BUNDLES, &app).await;
    hold.off().await;
    if topology_lacks_transactions(&results) {
        return;
    }

    // Queued Bundles are only delayed: every one still commits in full.
    for result in &results {
        let bundle = result.as_ref().expect("a queued bundle must still commit");
        assert_eq!(bundle.entries.len(), BUNDLE_ENTRIES);
        assert!(bundle.entries.iter().all(|e| e.status == 201));
    }
    assert_eq!(
        count_docs(&backend, "resources", doc! {"tenant_id": tenant}).await,
        (BUNDLES * BUNDLE_ENTRIES) as u64
    );

    assert!(
        max_open <= 1,
        "saw {max_open} open bundle transactions although each Bundle weighs the whole room"
    );
    assert!(max_open >= 1, "the sampler never saw an open transaction");
}
