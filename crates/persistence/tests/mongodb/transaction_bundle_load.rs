//! #1776: measurement harness for concurrent transaction Bundles under
//! WiredTiger cache pressure. Ignored by default.
//!
//! Many large transaction Bundles open at once pin more uncommitted writes than
//! a small WiredTiger cache can hold; the server then rolls transactions back
//! for eviction (`WriteConflict` / `TransientTransactionError`), and the
//! backend's 3-attempt retry re-collides until it gives up with
//! `TransactionError::Transient`. This test replays that shape (with the
//! default limit the run measures the fix; with `LIMIT=0` it reproduces the
//! failure) — the Inferno US Core test fixture
//! `hfs/tests/inferno/uscore_bundle_patient_85.json` (267 entries, one patient;
//! not the benchmark's 49,027-entry Synthea Bundle) repeated `COPIES` times per
//! Bundle with fresh identities ([`bundle_entries`]), `BUNDLES` Bundles pushed
//! through `CONCURRENCY` virtual users — against its own mongo:7.0 replica set
//! with a chosen cache size, and prints one `BUNDLE_LOAD` line with the
//! ok / transient / other counts and the server's eviction-rollback counter.
//! With a configuration where a Bundle takes the whole room (entries >=
//! `LIMIT` x `WEIGHT_ENTRIES`), an eviction rollback now ends as
//! `TooLargeForCache` instead of a replay (#1895); the `too_large` count shows it.
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
//! - `HFS_TEST_BUNDLE_LOAD_DEFERRED_INDEX_ENTRIES` ("off"):
//!   `transaction_bundle_deferred_index_entries` (#1975), the entry count from
//!   which a Bundle commits without its search rows and writes them after the
//!   commit. `off` (or `0`) never defers, so the run keeps measuring the
//!   in-transaction write set that #1776 and #1895 are about; `auto` is the
//!   backend default (the admission room in entries, 4,000); a number is that
//!   threshold. The `BUNDLE_LOAD` line reports the resolved threshold as
//!   `deferred_index_min_entries` (`None`: no Bundle defers).
//! - `HFS_TEST_BUNDLE_LOAD_MONGODB_URL`: an external replica-set URL; no
//!   container is started and the cache size is whatever that server has.
//! - `HFS_TEST_BUNDLE_LOAD_EXPECT_ALL_OK` ("1"/"true"): assert `transient == 0` and
//!   `too_large == 0`.
//!
//! The harness starts its own container rather than using the shared one,
//! whose cache is fixed at 0.25GB.

use super::*;

use std::collections::{HashMap, HashSet};
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

/// How [`transaction_entries`] treats the identities a Bundle carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Identities {
    /// Every copy gets fresh `urn:uuid` fullUrls, every string value equal to
    /// one of the Bundle's fullUrls takes that entry's new one so references
    /// follow, and a PUT gets a fresh id and url `{type}/{id}`.
    Fresh,
    /// fullUrls, references and PUT entries stay as in the Bundle.
    AsSent,
}

/// What [`transaction_entries`] builds.
#[derive(Clone, Copy, Debug)]
pub(super) struct EntryOptions {
    /// Copies of the entries, one after another; more than one needs
    /// [`Identities::Fresh`].
    pub(super) copies: usize,
    /// Only the first `limit` entries of the Bundle, in every copy;
    /// `usize::MAX` is all of them. With [`Identities::Fresh`], a reference to
    /// an entry the limit cuts keeps its sent fullUrl.
    pub(super) limit: usize,
    pub(super) identities: Identities,
}

/// `copies` copies of the fixture's entries, each with fresh `urn:uuid`s.
///
/// POST entries lose their `id`, as the REST layer strips it (batch.rs
/// `parse_bundle_entry`, http.html#create, #647): left in, every Bundle would
/// POST the fixture's fixed Location/Organization ids and the run would measure
/// duplicate-key conflicts instead of cache pressure. The PUT gets a fresh id
/// for the same reason. This is [`transaction_entries`] with
/// [`Identities::Fresh`].
pub(super) fn bundle_entries(fixture: &Value, copies: usize) -> Vec<BundleEntry> {
    transaction_entries(
        fixture,
        EntryOptions {
            copies,
            limit: usize::MAX,
            identities: Identities::Fresh,
        },
    )
}

/// A transaction Bundle's POST and PUT entries as the REST layer hands them to
/// the backend, built in time linear in the size of the entries.
///
/// With [`Identities::Fresh`], each copy builds one map from every old fullUrl
/// to a new `urn:uuid`, then walks each entry's values once (no whole-text
/// replace per fullUrl, which was quadratic in the entry count, #2434). Only a
/// string value equal to an old fullUrl is rewritten, not a fullUrl inside a
/// longer string or an object key. The US Core fixture, the only Bundle the
/// harnesses copy with fresh identities, has neither
/// (`the_fixture_names_full_urls_only_as_whole_strings`).
pub(super) fn transaction_entries(bundle: &Value, options: EntryOptions) -> Vec<BundleEntry> {
    let entries = bundle["entry"]
        .as_array()
        .expect("the Bundle has an entry array");
    let entries = &entries[..options.limit.min(entries.len())];
    assert!(
        options.copies <= 1 || options.identities == Identities::Fresh,
        "copies of a Bundle as sent would repeat its fullUrls"
    );
    let mut out = Vec::with_capacity(entries.len() * options.copies);
    for _ in 0..options.copies {
        let fresh: HashMap<&str, String> = match options.identities {
            Identities::Fresh => entries
                .iter()
                .filter_map(|entry| entry["fullUrl"].as_str())
                .map(|old| (old, format!("urn:uuid:{}", uuid::Uuid::new_v4())))
                .collect(),
            Identities::AsSent => HashMap::new(),
        };
        for entry in entries {
            let mut entry = entry.clone();
            if !fresh.is_empty() {
                rewrite_full_urls(&mut entry, &fresh);
            }
            out.push(to_bundle_entry(entry, options.identities));
        }
    }
    out
}

/// Replaces, in place, every string in `value` that is a key of `fresh` with its
/// value: one visit per JSON value.
fn rewrite_full_urls(value: &mut Value, fresh: &HashMap<&str, String>) {
    match value {
        Value::String(text) => {
            if let Some(new) = fresh.get(text.as_str()) {
                *text = new.clone();
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| rewrite_full_urls(item, fresh)),
        Value::Object(map) => map
            .values_mut()
            .for_each(|item| rewrite_full_urls(item, fresh)),
        _ => {}
    }
}

/// One Bundle entry as the backend takes it: a POST loses its `id`; with
/// [`Identities::Fresh`] a PUT gets a fresh id and url `{type}/{id}`.
fn to_bundle_entry(mut entry: Value, identities: Identities) -> BundleEntry {
    let method = match entry["request"]["method"].as_str() {
        Some("POST") => BundleMethod::Post,
        Some("PUT") => BundleMethod::Put,
        other => {
            panic!("unexpected method {other:?} in a transaction Bundle entry (POST and PUT only)")
        }
    };
    let full_url = entry["fullUrl"].as_str().map(str::to_string);
    let mut url = entry["request"]["url"].as_str().unwrap_or("").to_string();
    let mut resource = entry["resource"].take();
    match (method, identities) {
        (BundleMethod::Post, _) => {
            resource
                .as_object_mut()
                .expect("a POST entry carries a resource object")
                .remove("id");
        }
        (_, Identities::Fresh) => {
            let id = uuid::Uuid::new_v4().to_string();
            let ty = resource["resourceType"]
                .as_str()
                .expect("a PUT resource names its resourceType")
                .to_string();
            url = format!("{ty}/{id}");
            resource["id"] = Value::String(id);
        }
        (_, Identities::AsSent) => {}
    }
    BundleEntry {
        method,
        url,
        resource: Some(resource),
        if_match: None,
        if_none_match: None,
        if_none_exist: None,
        criteria: None,
        full_url,
    }
}

/// The server's count of transactions rolled back because they pinned the
/// oldest transaction ID under cache pressure — the root cause of #1776.
pub(super) async fn eviction_rollbacks(url: &str) -> i64 {
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
    // Off unless asked for: the harness exists to measure the write set inside
    // the transaction (#1975 would otherwise defer these Bundles' search rows).
    config.transaction_bundle_deferred_index_entries = match env_or(
        "HFS_TEST_BUNDLE_LOAD_DEFERRED_INDEX_ENTRIES",
        "off",
    )
    .trim()
    .to_ascii_lowercase()
    .as_str()
    {
        "" | "off" | "0" => Some(0),
        "auto" => None,
        raw => Some(raw.parse().expect(
            "HFS_TEST_BUNDLE_LOAD_DEFERRED_INDEX_ENTRIES must be off, auto or an unsigned integer",
        )),
    };
    let limit = config.max_concurrent_transaction_bundles;
    let weight_entries = config.transaction_bundle_weight_entries;
    let backend = Arc::new(MongoBackend::new(config).unwrap());
    backend.initialize().await.unwrap();
    // The resolved threshold: `None` when no Bundle defers its search rows.
    let deferred_index_min_entries = backend.deferred_search_index_min_entries();

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

    let (mut ok, mut transient, mut too_large, mut other) = (0usize, 0usize, 0usize, 0usize);
    let mut first_other: Option<String> = None;
    let mut latencies = Vec::new();
    for task in tasks {
        let (elapsed, result) = task.await.expect("bundle task");
        latencies.push(elapsed.as_secs_f64());
        match result {
            Ok(_) => ok += 1,
            Err(TransactionError::Transient { .. }) => transient += 1,
            Err(TransactionError::TooLargeForCache { .. }) => too_large += 1,
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
         weight_entries={weight_entries} deferred_index_min_entries={deferred_index_min_entries:?} ok={ok} transient={transient} too_large={too_large} other={other} \
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
    assert_eq!(ok + transient + too_large, bundles);
    if expect_all_ok {
        assert_eq!(transient, 0, "{transient} bundles exhausted their retries");
        assert_eq!(
            too_large, 0,
            "{too_large} bundles ended too large for the cache"
        );
    }
}

// ---- unit tests of bundle_entries (#2434); no mongod ----

fn us_core_fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../hfs/tests/inferno/uscore_bundle_patient_85.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("read fixture"))
        .expect("parse fixture")
}

/// How many string values in `value` are one of `full_urls`.
fn count_full_url_strings(value: &Value, full_urls: &HashSet<&str>) -> usize {
    match value {
        Value::String(text) => usize::from(full_urls.contains(text.as_str())),
        Value::Array(items) => items
            .iter()
            .map(|item| count_full_url_strings(item, full_urls))
            .sum(),
        Value::Object(map) => map
            .values()
            .map(|item| count_full_url_strings(item, full_urls))
            .sum(),
        _ => 0,
    }
}

/// Walks `sent` and `built` in parallel: a string of `sent` that is a key of
/// `fresh` must be its value in `built`, everything else must be equal. Returns
/// how many strings were rewritten.
fn assert_follows(
    sent: &Value,
    built: &Value,
    fresh: &HashMap<String, String>,
    path: &str,
) -> usize {
    match (sent, built) {
        (Value::String(old), Value::String(new)) if fresh.contains_key(old) => {
            assert_eq!(new, &fresh[old], "{path}");
            1
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.len(), b.len(), "{path}: keys");
            a.iter()
                .map(|(key, value)| {
                    let other = b
                        .get(key)
                        .unwrap_or_else(|| panic!("{path}: the built value lost `{key}`"));
                    assert_follows(value, other, fresh, &format!("{path}.{key}"))
                })
                .sum()
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{path}: length");
            a.iter()
                .zip(b)
                .enumerate()
                .map(|(i, (value, other))| {
                    assert_follows(value, other, fresh, &format!("{path}[{i}]"))
                })
                .sum()
        }
        _ => {
            assert_eq!(sent, built, "{path}");
            0
        }
    }
}

/// One copy of the fixture's entries, `built`, against the entries `sent`: new
/// `urn:uuid` fullUrls, references that follow them, a POST without its `id`, a
/// PUT with a fresh id and url. Returns how many strings were rewritten.
fn assert_copy_follows(sent: &[Value], built: &[BundleEntry]) -> usize {
    assert_eq!(sent.len(), built.len());
    let fresh: HashMap<String, String> = sent
        .iter()
        .zip(built)
        .map(|(s, b)| {
            (
                s["fullUrl"].as_str().expect("sent fullUrl").to_string(),
                b.full_url.clone().expect("built full_url"),
            )
        })
        .collect();
    for (old, new) in &fresh {
        let uuid = new.strip_prefix("urn:uuid:").expect("a urn:uuid fullUrl");
        uuid::Uuid::parse_str(uuid).expect("a UUID after urn:uuid:");
        assert_ne!(old, new, "the fullUrl is new");
    }
    let mut rewritten = 0;
    for (s, b) in sent.iter().zip(built) {
        let resource = b.resource.as_ref().expect("a resource");
        let mut expected = s["resource"].clone();
        match s["request"]["method"].as_str() {
            Some("POST") => {
                assert_eq!(b.method, BundleMethod::Post);
                assert_eq!(b.url, s["request"]["url"].as_str().unwrap());
                assert!(resource.get("id").is_none(), "a POST loses its id");
                expected.as_object_mut().unwrap().remove("id");
            }
            Some("PUT") => {
                assert_eq!(b.method, BundleMethod::Put);
                let id = resource["id"].as_str().expect("a PUT resource has an id");
                assert_ne!(Some(id), s["resource"]["id"].as_str(), "the PUT id is new");
                assert_eq!(
                    b.url,
                    format!("{}/{id}", s["resource"]["resourceType"].as_str().unwrap())
                );
                expected["id"] = Value::String(id.to_string());
            }
            other => panic!("unexpected fixture method {other:?}"),
        }
        assert!(b.if_match.is_none() && b.if_none_match.is_none());
        assert!(b.if_none_exist.is_none() && b.criteria.is_none());
        rewritten += assert_follows(&expected, resource, &fresh, "resource");
    }
    rewritten
}

/// References follow their entry's new fullUrl: every string in a resource
/// that was a fullUrl is the new one, and nothing else changed.
#[test]
fn bundle_entries_references_follow_their_fresh_full_urls() {
    let fixture = us_core_fixture();
    let sent = fixture["entry"].as_array().expect("entry array");
    let built = bundle_entries(&fixture, 1);
    let rewritten = assert_copy_follows(sent, &built);

    let full_urls: HashSet<&str> = sent
        .iter()
        .map(|e| e["fullUrl"].as_str().unwrap())
        .collect();
    // 1,127 on the fixture: 1,102 `reference` values and 25 identifier values.
    let expected: usize = sent
        .iter()
        .map(|e| count_full_url_strings(&e["resource"], &full_urls))
        .sum();
    assert_eq!(rewritten, expected);
    assert!(rewritten > 0, "the fixture has references to rewrite");
}

/// Copies do not share fullUrls, and a PUT gets a new id in each copy.
#[test]
fn bundle_entries_copies_get_distinct_full_urls_and_put_ids() {
    let fixture = us_core_fixture();
    let sent = fixture["entry"].as_array().expect("entry array");
    let n = sent.len();
    assert_eq!(n, 267);
    let built = bundle_entries(&fixture, 3);
    assert_eq!(built.len(), 3 * n);
    for copy in 0..3 {
        assert_copy_follows(sent, &built[copy * n..(copy + 1) * n]);
    }

    let full_urls: HashSet<&str> = built
        .iter()
        .map(|b| b.full_url.as_deref().expect("full_url"))
        .collect();
    assert_eq!(full_urls.len(), 3 * n, "every fullUrl is distinct");
    for e in sent {
        assert!(
            !full_urls.contains(e["fullUrl"].as_str().unwrap()),
            "no copy keeps a sent fullUrl"
        );
    }

    let put_ids: Vec<&str> = built
        .iter()
        .filter(|b| b.method == BundleMethod::Put)
        .map(|b| b.resource.as_ref().unwrap()["id"].as_str().unwrap())
        .collect();
    assert_eq!(put_ids.len(), 3, "one PUT per copy");
    assert_eq!(put_ids.iter().collect::<HashSet<_>>().len(), 3);
    assert!(!put_ids.contains(&"85"));
}

/// The fixture names a fullUrl only as a whole string value: none inside a
/// longer string and none as an object key. That is why `transaction_entries`'
/// exact-value rewrite gives what the old per-fullUrl replace over the
/// serialized entries gave on it.
#[test]
fn the_fixture_names_full_urls_only_as_whole_strings() {
    fn visit(value: &Value, full_urls: &HashSet<&str>, path: &str) {
        match value {
            Value::String(text) => assert!(
                full_urls.contains(text.as_str()) || !text.contains("urn:uuid:"),
                "{path}: `{text}` embeds a urn:uuid"
            ),
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    visit(item, full_urls, &format!("{path}[{i}]"));
                }
            }
            Value::Object(map) => {
                for (key, item) in map {
                    assert!(!key.contains("urn:uuid:"), "{path}: key `{key}`");
                    visit(item, full_urls, &format!("{path}.{key}"));
                }
            }
            _ => {}
        }
    }

    let fixture = us_core_fixture();
    let entries = fixture["entry"].as_array().expect("entry array");
    let full_urls: HashSet<&str> = entries
        .iter()
        .map(|e| e["fullUrl"].as_str().expect("fullUrl"))
        .collect();
    assert!(full_urls.iter().all(|u| u.starts_with("urn:uuid:")));
    for (i, entry) in entries.iter().enumerate() {
        visit(entry, &full_urls, &format!("entry[{i}]"));
    }
}

/// A transaction Bundle of `n` entries: a PUT Patient, then POST Observations
/// that refer back (`i / 2`) and forward (`i + 1`, wrapping) to other entries.
fn synthetic_bundle(n: usize) -> Value {
    let full_url = |i: usize| format!("urn:uuid:{}", uuid::Uuid::from_u128(i as u128 + 1));
    let mut entries = vec![json!({
        "fullUrl": full_url(0),
        "resource": {"resourceType": "Patient", "id": "p0"},
        "request": {"method": "PUT", "url": "Patient/p0"}
    })];
    for i in 1..n {
        entries.push(json!({
            "fullUrl": full_url(i),
            "resource": {
                "resourceType": "Observation",
                "id": format!("obs-{i}"),
                "status": "final",
                "code": {"text": "synthetic"},
                "subject": {"reference": full_url(i / 2)},
                "hasMember": [{"reference": full_url((i + 1) % n)}]
            },
            "request": {"method": "POST", "url": "Observation"}
        }));
    }
    json!({"resourceType": "Bundle", "type": "transaction", "entry": entries})
}

/// #2434: a benchmark-size Bundle converts, with every reference following.
/// No wall-time assertion; the printed `secs` goes into the PR notes.
#[test]
fn bundle_entries_converts_a_20000_entry_bundle() {
    const N: usize = 20_000;
    let bundle = synthetic_bundle(N);
    let sent: HashSet<&str> = bundle["entry"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["fullUrl"].as_str().unwrap())
        .collect();

    let started = Instant::now();
    let built = bundle_entries(&bundle, 1);
    let secs = started.elapsed().as_secs_f64();
    eprintln!(
        "BUNDLE_ENTRIES_SIZE entries={N} built={} secs={secs:.3}",
        built.len()
    );

    assert_eq!(built.len(), N);
    let full_urls: Vec<&str> = built
        .iter()
        .map(|b| b.full_url.as_deref().expect("full_url"))
        .collect();
    assert_eq!(full_urls.iter().collect::<HashSet<_>>().len(), N);
    assert!(full_urls.iter().all(|u| !sent.contains(u)));
    for (i, b) in built.iter().enumerate().skip(1) {
        let resource = b.resource.as_ref().unwrap();
        assert_eq!(b.method, BundleMethod::Post);
        assert_eq!(b.url, "Observation");
        assert!(resource.get("id").is_none());
        assert_eq!(
            resource["subject"]["reference"].as_str(),
            Some(full_urls[i / 2]),
            "entry {i} subject"
        );
        assert_eq!(
            resource["hasMember"][0]["reference"].as_str(),
            Some(full_urls[(i + 1) % N]),
            "entry {i} hasMember"
        );
    }
    let patient = built[0].resource.as_ref().unwrap();
    let id = patient["id"].as_str().unwrap();
    assert_eq!(built[0].method, BundleMethod::Put);
    assert_ne!(id, "p0");
    assert_eq!(built[0].url, format!("Patient/{id}"));
}
