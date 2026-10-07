//! #1776: measurement harness for concurrent transaction Bundles under
//! WiredTiger cache pressure. Ignored by default.
//!
//! Many large transaction Bundles open at once pin more uncommitted writes than
//! a small WiredTiger cache can hold; the server then rolls transactions back
//! for eviction (`WriteConflict` / `TransientTransactionError`), and the
//! backend's 3-attempt retry re-collides until it gives up with
//! `TransactionError::Transient`. This test replays that shape (with the
//! default limit the run measures the fix; with `LIMIT=0` it reproduces the
//! failure) — the US Core
//! Synthea-style fixture `hfs/tests/inferno/uscore_bundle_patient_85.json`
//! (267 entries) repeated `COPIES` times per Bundle, `BUNDLES` Bundles pushed
//! through `CONCURRENCY` virtual users — against its own mongo:7.0 replica set
//! with a chosen cache size, and prints one `BUNDLE_LOAD` line with the
//! ok / transient / other counts and the server's eviction-rollback counter.
//!
//! Run with:
//! `cargo test -p helios-persistence --features mongodb --test mongodb_tests
//! transaction_bundle_load -- --ignored --nocapture`
//!
//! Env knobs (defaults in parentheses):
//! - `HFS_TEST_BUNDLE_LOAD_WT_CACHE_GB` ("1"): `--wiredTigerCacheSizeGB`.
//! - `HFS_TEST_BUNDLE_LOAD_CONCURRENCY` (20): simultaneous Bundles.
//! - `HFS_TEST_BUNDLE_LOAD_BUNDLES` (24): total Bundles.
//! - `HFS_TEST_BUNDLE_LOAD_COPIES` (3): fixture copies per Bundle.
//! - `HFS_TEST_BUNDLE_LOAD_LIMIT`: `max_concurrent_transaction_bundles` (the
//!   backend default, 4; `0` removes the limit and reproduces the #1776
//!   exhaustion).
//! - `HFS_TEST_BUNDLE_LOAD_WEIGHT_ENTRIES`: `transaction_bundle_weight_entries`,
//!   the entry count of one standard Bundle (the backend default, 1000; `0`
//!   counts each Bundle as one slot, the #1776 count-only gate).
//! - `HFS_TEST_BUNDLE_LOAD_MONGODB_URL`: an external replica-set URL; no
//!   container is started and the cache size is whatever that server has.
//! - `HFS_TEST_BUNDLE_LOAD_EXPECT_ALL_OK` ("1"/"true"): assert `transient == 0`.
//!
//! The harness starts its own container rather than using the shared one,
//! whose cache is fixed at 0.25GB.

use super::*;

use std::time::{Duration, Instant};

use serde_json::Value;
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::mongo::Mongo;
use tokio::sync::Semaphore;

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn env_usize(name: &str, default: usize) -> usize {
    env_or(name, &default.to_string())
        .parse()
        .unwrap_or_else(|_| panic!("{name} must be an unsigned integer"))
}

/// `copies` copies of the fixture's entries, each with fresh `urn:uuid`s.
///
/// POST entries lose their `id`, as the REST layer strips it (batch.rs
/// `parse_entry`, http.html#create, #647): left in, every Bundle would POST the
/// fixture's fixed Location/Organization ids and the run would measure
/// duplicate-key conflicts instead of cache pressure. The PUT gets a fresh id
/// for the same reason.
fn bundle_entries(fixture: &Value, copies: usize) -> Vec<BundleEntry> {
    let mut out = Vec::new();
    for _ in 0..copies {
        let mut text = serde_json::to_string(&fixture["entry"]).unwrap();
        let entries = fixture["entry"].as_array().expect("fixture entry array");
        // Every fullUrl is rewritten wherever it appears, so references follow.
        for entry in entries {
            if let Some(old) = entry["fullUrl"].as_str() {
                text = text.replace(old, &format!("urn:uuid:{}", uuid::Uuid::new_v4()));
            }
        }
        let fresh: Vec<Value> = serde_json::from_str(&text).unwrap();
        for mut entry in fresh {
            let method = match entry["request"]["method"].as_str() {
                Some("POST") => BundleMethod::Post,
                Some("PUT") => BundleMethod::Put,
                other => panic!("unexpected fixture method {other:?}"),
            };
            let full_url = entry["fullUrl"].as_str().map(str::to_string);
            let mut resource = entry["resource"].take();
            let mut url = entry["request"]["url"].as_str().unwrap_or("").to_string();
            match method {
                BundleMethod::Post => {
                    resource.as_object_mut().unwrap().remove("id");
                }
                _ => {
                    let id = uuid::Uuid::new_v4().to_string();
                    let ty = resource["resourceType"].as_str().unwrap().to_string();
                    url = format!("{ty}/{id}");
                    resource["id"] = Value::String(id);
                }
            }
            out.push(BundleEntry {
                method,
                url,
                resource: Some(resource),
                if_match: None,
                if_none_match: None,
                if_none_exist: None,
                criteria: None,
                full_url,
            });
        }
    }
    out
}

/// The server's count of transactions rolled back because they pinned the
/// oldest transaction ID under cache pressure — the root cause of #1776.
async fn eviction_rollbacks(url: &str) -> i64 {
    let client = raw_test_client(url).await.expect("raw client");
    let status = client
        .database("admin")
        .run_command(doc! {"serverStatus": 1})
        .await
        .expect("serverStatus");
    let tx = status
        .get_document("wiredTiger")
        .and_then(|w| w.get_document("transaction"))
        .expect("wiredTiger.transaction");
    match tx.get("oldest pinned transaction ID rolled back for eviction") {
        Some(Bson::Int32(v)) => *v as i64,
        Some(Bson::Int64(v)) => *v,
        Some(Bson::Double(v)) => *v as i64,
        other => panic!("unexpected rollback counter {other:?}"),
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "#1776 measurement harness; run with --ignored"]
async fn bundle_load_reports_transient_failures() {
    let cache_gb = env_or("HFS_TEST_BUNDLE_LOAD_WT_CACHE_GB", "1");
    let concurrency = env_usize("HFS_TEST_BUNDLE_LOAD_CONCURRENCY", 20);
    let bundles = env_usize("HFS_TEST_BUNDLE_LOAD_BUNDLES", 24);
    let copies = env_usize("HFS_TEST_BUNDLE_LOAD_COPIES", 3);
    let expect_all_ok = matches!(
        env_or("HFS_TEST_BUNDLE_LOAD_EXPECT_ALL_OK", "").as_str(),
        "1" | "true"
    );

    // Keep the container alive (and removed on drop) for the whole test.
    let _container;
    let (url, cache_label) = match std::env::var("HFS_TEST_BUNDLE_LOAD_MONGODB_URL") {
        Ok(url) => (url, "external".to_string()),
        Err(_) => {
            // Mirrors .github/scripts/fhir-bench/start-mongodb.sh: mongo:7.0,
            // single-member rs0, a 900s transaction lifetime.
            let started = super::container_cleanup::with_cleanup_label(
                Mongo::default()
                    .with_tag("7.0")
                    .with_cmd([
                        "mongod",
                        "--bind_ip_all",
                        "--wiredTigerCacheSizeGB",
                        cache_gb.as_str(),
                        "--replSet",
                        "rs0",
                        "--oplogSize",
                        "1024",
                        "--setParameter",
                        "transactionLifetimeLimitSeconds=900",
                    ])
                    .with_ulimit("nofile", 64000, Some(64000))
                    .with_startup_timeout(Duration::from_secs(120)),
            )
            .start()
            .await;
            let container = match started {
                Ok(c) => c,
                Err(err) => {
                    eprintln!("skip: could not start a Mongo container: {err}");
                    return;
                }
            };
            shared_mongo::initiate_replica_set(&container).await;
            shared_mongo::wait_for_writable_primary(&container).await;
            let host = container.get_host().await.expect("container host");
            let port = container
                .get_host_port_ipv4(27017)
                .await
                .expect("container port");
            _container = container;
            (
                format!("mongodb://{host}:{port}/?directConnection=true"),
                cache_gb.clone(),
            )
        }
    };

    // Built directly: `build_backend` caps the pool at 4, which would stretch
    // transactions while operations wait for connections.
    let mut config = MongoBackendConfig {
        connection_string: url.clone(),
        database_name: build_test_database_name("bundle_load"),
        max_connections: 32,
        data_dir: Some(repo_data_dir()),
        index_build: IndexBuildMode::Inline,
        ..Default::default()
    };
    if let Ok(raw) = std::env::var("HFS_TEST_BUNDLE_LOAD_LIMIT") {
        let raw = raw.trim();
        if !raw.is_empty() {
            config.max_concurrent_transaction_bundles = raw
                .parse()
                .expect("HFS_TEST_BUNDLE_LOAD_LIMIT must be an unsigned integer");
        }
    }
    if let Ok(raw) = std::env::var("HFS_TEST_BUNDLE_LOAD_WEIGHT_ENTRIES") {
        let raw = raw.trim();
        if !raw.is_empty() {
            config.transaction_bundle_weight_entries = raw
                .parse()
                .expect("HFS_TEST_BUNDLE_LOAD_WEIGHT_ENTRIES must be an unsigned integer");
        }
    }
    let limit = config.max_concurrent_transaction_bundles;
    let weight_entries = config.transaction_bundle_weight_entries;
    let backend = Arc::new(MongoBackend::new(config).unwrap());
    backend.initialize().await.unwrap();

    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../hfs/tests/inferno/uscore_bundle_patient_85.json");
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture_path).expect("read fixture"))
            .expect("parse fixture");

    // Built up front so entry construction is not part of the timed run.
    let all_entries: Vec<Vec<BundleEntry>> = (0..bundles)
        .map(|_| bundle_entries(&fixture, copies))
        .collect();
    let per_bundle = all_entries.first().map_or(0, Vec::len);

    let before = eviction_rollbacks(&url).await;

    // The permits play the k6 virtual users.
    let vus = Arc::new(Semaphore::new(concurrency));
    let wall = Instant::now();
    let mut tasks = Vec::new();
    for entries in all_entries {
        let backend = Arc::clone(&backend);
        let vus = Arc::clone(&vus);
        tasks.push(tokio::spawn(async move {
            let _vu = vus.acquire_owned().await.unwrap();
            let t0 = Instant::now();
            let result = backend
                .process_transaction(
                    &create_tenant("bundle-load"),
                    entries,
                    FhirVersion::default(),
                )
                .await;
            (t0.elapsed(), result)
        }));
    }

    let (mut ok, mut transient, mut other) = (0usize, 0usize, 0usize);
    let mut first_other: Option<String> = None;
    let mut latencies = Vec::new();
    for task in tasks {
        let (elapsed, result) = task.await.expect("bundle task");
        latencies.push(elapsed.as_secs_f64());
        match result {
            Ok(_) => ok += 1,
            Err(TransactionError::Transient { .. }) => transient += 1,
            Err(err) => {
                other += 1;
                first_other.get_or_insert_with(|| err.to_string());
            }
        }
    }
    let wall_s = wall.elapsed().as_secs_f64();
    let rollbacks = eviction_rollbacks(&url).await - before;
    latencies.sort_by(f64::total_cmp);

    println!(
        "BUNDLE_LOAD wt_cache_gb={cache_label} entries={per_bundle} concurrency={concurrency} \
         bundles={bundles} limit={limit} \
         weight_entries={weight_entries} ok={ok} transient={transient} other={other} \
         wall_s={wall_s:.1} p50_s={:.1} p95_s={:.1} wt_rollbacks={rollbacks}",
        percentile(&latencies, 0.50),
        percentile(&latencies, 0.95),
    );

    assert_eq!(
        other,
        0,
        "unexpected non-transient failure: {}",
        first_other.unwrap_or_default()
    );
    assert_eq!(ok + transient, bundles);
    if expect_all_ok {
        assert_eq!(transient, 0, "{transient} bundles exhausted their retries");
    }
}
