//! #1776: transaction Bundles beyond `max_concurrent_transaction_bundles` wait
//! for an admission slot instead of each opening a transaction at once, so the
//! server never holds more than the limit's worth of open bundle transactions
//! from one backend (and WiredTiger's cache is not exhausted by many large
//! uncommitted write sets).
//!
//! Child module of the `mongodb_tests` root — `use super::*;` reaches its
//! private harness (`create_tenant`, `build_backend`, `count_docs`,
//! `raw_test_client`, `transactions_required`, plus `BundleEntry`/`doc`/`json`/
//! `FhirVersion`), the same arrangement as `tests/mongodb/transaction_retry.rs`.
//!
//! The open transactions are counted from the server's side: `$currentOp` lists
//! a transaction that is open between operations as an `idleSession` carrying
//! the session's `appName` and a `transaction` sub-document. Each test gives its
//! backend a unique `appName`, because `$currentOp` is server-global and the
//! suite's tests share one container and run in parallel.

use super::*;

use std::sync::atomic::{AtomicBool, Ordering};

/// Entries per bundle: enough that every bundle writes real search rows and
/// stays open long enough for concurrent bundles to overlap.
const BUNDLE_ENTRIES: usize = 60;

/// A backend with `limit` as its admission limit, under a unique `appName`.
/// `None` when the shared Mongo is unavailable, so the caller skips.
async fn backend_with_limit(test: &str, app: &str, limit: usize) -> Option<MongoBackend> {
    let config = MongoBackendConfig {
        connection_string: shared_mongo::connection_string().await?,
        database_name: build_test_database_name(test),
        app_name: app.to_string(),
        data_dir: Some(repo_data_dir()),
        max_concurrent_transaction_bundles: limit,
        ..Default::default()
    };
    build_backend(config).await
}

fn unique_app(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
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

/// How many distinct sessions of `app` currently hold an open transaction.
async fn open_transactions(client: &Client, app: &str) -> usize {
    use futures::stream::TryStreamExt;

    let pipeline = vec![
        doc! {"$currentOp": {"allUsers": true, "idleSessions": true, "idleConnections": false}},
        doc! {"$match": {"appName": app, "transaction": {"$exists": true}}},
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
    let Some(backend) = backend_with_limit("txn_gate_limit", &app, LIMIT).await else {
        return;
    };
    let backend = Arc::new(backend);

    let (max_open, results) = run(backend.clone(), tenant, BUNDLES, &app).await;
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
    let Some(backend) = backend_with_limit("txn_gate_nolimit", &app, 0).await else {
        return;
    };

    let (max_open, results) = run(Arc::new(backend), tenant, BUNDLES, &app).await;
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
