//! #2006 measurement harness: what `resources.missing_params` and
//! `idx_resources_missing_params` (#1984) cost writers, storage and the
//! WiredTiger cache, measured by loading the same Synthea resources with
//! `missing_metadata` on and off (`HFS_MONGODB_MISSING_METADATA`).
//!
//! Each arm gets a fresh database and runs three phases:
//! `create` (concurrent single-resource creates), `update` (concurrent
//! updates of a sample of the created resources, identical content, so the
//! absent set is unchanged) and `bulk-submit` (`process_ndjson_stream` into a
//! second tenant). After the load it prints collection, index and cache
//! figures, and for the metadata arm the per-type length of the array and the
//! names that are absent from nearly every resource of a type.
//!
//! ```text
//! HFS_TEST_MISSING_COST_MONGODB_URL=mongodb://127.0.0.1:27206/?directConnection=true \
//! HFS_TEST_MISSING_COST_CORPUS=/path/to/synthea/ndjson \
//! HFS_TEST_MISSING_COST_ROWS=1000000 \
//!   cargo test -p helios-persistence --features mongodb --test mongodb_tests \
//!   missing_params_cost_bench -- --ignored --nocapture
//! ```
//!
//! Other knobs: `_CONCURRENCY` (writers, default 32), `_UPDATES` (default
//! 50 000), `_BULK_ROWS` (default 200 000), `_ARMS` (default `on,off`;
//! repeat names to interleave rounds, e.g. `on,off,off,on`).

use super::*;
use futures::StreamExt;
use helios_persistence::core::{
    BulkProcessingOptions, BulkSubmitProvider, StreamingBulkSubmitProvider, SubmissionId,
};
use helios_persistence::types::StoredResource;
use std::io::BufRead;
use std::time::{Duration, Instant};

/// The corpus's resource types and their line counts, which set each type's
/// share of a load so it keeps the corpus's natural mix.
const MIX: &[(&str, u64)] = &[
    ("Observation", 7_699_881),
    ("Procedure", 2_177_173),
    ("DiagnosticReport", 1_656_931),
    ("Claim", 1_605_515),
    ("ExplanationOfBenefit", 1_605_515),
    ("Encounter", 827_919),
    ("DocumentReference", 827_919),
    ("MedicationRequest", 777_596),
    ("Condition", 476_422),
    ("SupplyDelivery", 337_768),
    ("Medication", 272_640),
    ("MedicationAdministration", 272_640),
    ("Immunization", 170_475),
    ("Device", 75_076),
    ("ImagingStudy", 50_151),
    ("CarePlan", 41_419),
    ("CareTeam", 41_419),
    ("Patient", 11_704),
    ("Provenance", 11_704),
    ("AllergyIntolerance", 11_453),
];

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// How many of `rows` each type contributes, largest remainder first.
fn shares(rows: usize) -> Vec<(&'static str, usize)> {
    let total: u64 = MIX.iter().map(|(_, n)| n).sum();
    let mut shares: Vec<_> = MIX
        .iter()
        .map(|(t, n)| (*t, (rows as u64 * n / total) as usize))
        .collect();
    let short = rows - shares.iter().map(|(_, n)| n).sum::<usize>();
    for share in shares.iter_mut().take(short) {
        share.1 += 1;
    }
    shares
}

/// Streams `rows` corpus lines with the types interleaved in proportion
/// (smooth weighted round robin), from a blocking reader thread.
fn corpus_lines(corpus: &std::path::Path, rows: usize) -> tokio::sync::mpsc::Receiver<String> {
    let (tx, rx) = tokio::sync::mpsc::channel(8192);
    let corpus = corpus.to_owned();
    std::thread::spawn(move || {
        let mut sources: Vec<_> = shares(rows)
            .into_iter()
            .filter(|(_, n)| *n > 0)
            .map(|(t, n)| {
                let file = std::fs::File::open(corpus.join(format!("{t}.ndjson")))
                    .unwrap_or_else(|e| panic!("open {t}.ndjson: {e}"));
                (
                    std::io::BufReader::with_capacity(1 << 20, file).lines(),
                    n,
                    0_i64,
                )
            })
            .collect();
        let total: i64 = sources.iter().map(|(_, n, _)| *n as i64).sum();
        for _ in 0..rows {
            for source in &mut sources {
                if source.1 > 0 {
                    source.2 += source.1 as i64;
                }
            }
            let pick = sources
                .iter_mut()
                .filter(|source| source.1 > 0)
                .max_by_key(|source| source.2)
                .expect("a source with lines left");
            pick.2 -= total;
            pick.1 -= 1;
            let line = pick.0.next().expect("corpus line").expect("read corpus");
            if tx.blocking_send(line).is_err() {
                return;
            }
        }
    });
    rx
}

/// All lines of one type's share, for `process_ndjson_stream`.
fn first_lines(corpus: &std::path::Path, resource_type: &str, n: usize) -> Vec<u8> {
    let file = std::fs::File::open(corpus.join(format!("{resource_type}.ndjson"))).unwrap();
    let mut out = Vec::new();
    for line in std::io::BufReader::with_capacity(1 << 20, file)
        .lines()
        .take(n)
    {
        out.extend_from_slice(line.unwrap().as_bytes());
        out.push(b'\n');
    }
    out
}

struct Latencies(Vec<Duration>);

impl Latencies {
    fn summary(mut self, elapsed: Duration) -> String {
        self.0.sort_unstable();
        let at = |q: f64| {
            let i = ((self.0.len() as f64 * q).ceil() as usize).clamp(1, self.0.len()) - 1;
            self.0[i].as_secs_f64() * 1000.0
        };
        format!(
            "{} | {:.0} | {:.1} | {:.1} | {:.1}",
            self.0.len(),
            self.0.len() as f64 / elapsed.as_secs_f64(),
            at(0.50),
            at(0.99),
            at(0.999),
        )
    }
}

/// The WiredTiger cache counters this harness reports, read from `serverStatus`.
const CACHE_COUNTERS: &[&str] = &[
    "pages read into cache",
    "pages written from cache",
    "bytes read into cache",
    "bytes written from cache",
    "modified pages evicted",
    "unmodified pages evicted",
    "pages evicted by application threads",
];

async fn cache_counters(db: &mongodb::Database) -> Vec<(String, f64)> {
    let status = db
        .client()
        .database("admin")
        .run_command(doc! { "serverStatus": 1 })
        .await
        .unwrap();
    let cache = status
        .get_document("wiredTiger")
        .and_then(|wt| wt.get_document("cache"))
        .cloned()
        .unwrap_or_default();
    let number = |key: &str| match cache.get(key) {
        Some(Bson::Int32(v)) => Some(*v as f64),
        Some(Bson::Int64(v)) => Some(*v as f64),
        Some(Bson::Double(v)) => Some(*v),
        _ => None,
    };
    let mut out = Vec::new();
    for key in CACHE_COUNTERS
        .iter()
        .copied()
        .chain(["bytes currently in the cache", "maximum bytes configured"])
    {
        if let Some(value) = number(key) {
            out.push((key.to_owned(), value));
        }
    }
    out
}

fn print_cache_delta(phase: &str, before: &[(String, f64)], after: &[(String, f64)]) {
    for (key, value) in after {
        if !CACHE_COUNTERS.contains(&key.as_str()) {
            continue;
        }
        let start = before
            .iter()
            .find(|(k, _)| k == key)
            .map_or(0.0, |(_, v)| *v);
        println!("cache | {phase} | {key} | {:.0}", value - start);
    }
}

fn mib(bytes: &Bson) -> f64 {
    let bytes = match bytes {
        Bson::Int32(v) => *v as f64,
        Bson::Int64(v) => *v as f64,
        Bson::Double(v) => *v,
        _ => 0.0,
    };
    bytes / (1024.0 * 1024.0)
}

async fn print_storage(arm: &str, db: &mongodb::Database) {
    for collection in ["resources", "resource_history", "search_index"] {
        let mut cursor = db
            .collection::<Document>(collection)
            .aggregate([doc! { "$collStats": { "storageStats": {} } }])
            .await
            .unwrap();
        let Some(stats) = cursor.next().await else {
            continue;
        };
        let stats = stats.unwrap();
        let storage = stats.get_document("storageStats").unwrap();
        let get = |key: &str| storage.get(key).cloned().unwrap_or(Bson::Int32(0));
        println!(
            "storage | {arm} | {collection} | count {} | data {:.1} MiB | avg obj {:.0} B | on disk {:.1} MiB | indexes {:.1} MiB",
            get("count"),
            mib(&get("size")),
            mib(&get("avgObjSize")) * 1024.0 * 1024.0,
            mib(&get("storageSize")),
            mib(&get("totalIndexSize")),
        );
        if collection == "resources"
            && let Ok(sizes) = storage.get_document("indexSizes")
        {
            for (name, size) in sizes {
                println!("index | {arm} | resources.{name} | {:.1} MiB", mib(size));
            }
        }
    }
}

/// Per type: resources, mean array length, and the names absent from at least
/// 99% of that type's resources with the share of all entries they hold.
async fn print_missing_profile(db: &mongodb::Database, tenant: &str) {
    let resources = db.collection::<Document>("resources");
    let mut per_type = resources
        .aggregate([
            doc! { "$match": { "tenant_id": tenant, "missing_params": { "$exists": true } } },
            doc! { "$group": {
                "_id": "$resource_type",
                "resources": { "$sum": 1 },
                "entries": { "$sum": { "$size": "$missing_params" } },
            } },
            doc! { "$sort": { "resources": -1 } },
        ])
        .await
        .unwrap();
    let mut types = Vec::new();
    while let Some(row) = per_type.next().await {
        let row = row.unwrap();
        let resource_type = row.get_str("_id").unwrap().to_owned();
        let count = row
            .get_i32("resources")
            .map(i64::from)
            .unwrap_or_else(|_| row.get_i64("resources").unwrap());
        let entries = row
            .get_i32("entries")
            .map(i64::from)
            .unwrap_or_else(|_| row.get_i64("entries").unwrap());
        types.push((resource_type, count, entries));
    }
    let all_resources: i64 = types.iter().map(|t| t.1).sum();
    let all_entries: i64 = types.iter().map(|t| t.2).sum();
    println!(
        "profile | all | {all_resources} resources | {:.2} absent names each",
        all_entries as f64 / all_resources.max(1) as f64
    );

    let mut names = resources
        .aggregate([
            doc! { "$match": { "tenant_id": tenant, "missing_params": { "$exists": true } } },
            doc! { "$unwind": "$missing_params" },
            doc! { "$group": {
                "_id": { "t": "$resource_type", "n": "$missing_params" },
                "count": { "$sum": 1 },
            } },
        ])
        .with_options(
            mongodb::options::AggregateOptions::builder()
                .allow_disk_use(true)
                .build(),
        )
        .await
        .unwrap();
    let mut by_name: Vec<(String, String, i64)> = Vec::new();
    while let Some(row) = names.next().await {
        let row = row.unwrap();
        let id = row.get_document("_id").unwrap();
        let count = row
            .get_i32("count")
            .map(i64::from)
            .unwrap_or_else(|_| row.get_i64("count").unwrap());
        by_name.push((
            id.get_str("t").unwrap().to_owned(),
            id.get_str("n").unwrap().to_owned(),
            count,
        ));
    }
    let mut universal_entries = 0;
    for (resource_type, count, entries) in &types {
        let mut universal: Vec<_> = by_name
            .iter()
            .filter(|(t, _, c)| t == resource_type && *c * 100 >= *count * 99)
            .map(|(_, n, _)| n.as_str())
            .collect();
        universal.sort_unstable();
        let held: i64 = by_name
            .iter()
            .filter(|(t, n, _)| t == resource_type && universal.contains(&n.as_str()))
            .map(|(_, _, c)| c)
            .sum();
        universal_entries += held;
        println!(
            "profile | {resource_type} | {count} resources | {:.2} absent names each | {} absent from >=99%: {}",
            *entries as f64 / (*count).max(1) as f64,
            universal.len(),
            universal.join(" "),
        );
    }
    println!(
        "profile | all | names absent from >=99% of their type hold {:.1}% of entries",
        universal_entries as f64 * 100.0 / all_entries.max(1) as f64
    );
}

async fn run_arm(arm: &str, metadata: bool, round: usize, url: &str, corpus: &std::path::Path) {
    let rows = env_usize("HFS_TEST_MISSING_COST_ROWS", 1_000_000);
    let concurrency = env_usize("HFS_TEST_MISSING_COST_CONCURRENCY", 32);
    let updates = env_usize("HFS_TEST_MISSING_COST_UPDATES", 50_000).min(rows);
    let bulk_rows = env_usize("HFS_TEST_MISSING_COST_BULK_ROWS", 200_000);
    let database = format!("hfs_bench_2006_{arm}");

    Client::with_uri_str(url)
        .await
        .unwrap()
        .database(&database)
        .drop()
        .await
        .unwrap();
    // Built directly rather than through `build_backend`, which caps the pool
    // at four connections and would make the writer count meaningless.
    let backend = MongoBackend::new(MongoBackendConfig {
        connection_string: url.to_owned(),
        database_name: database.clone(),
        data_dir: Some(repo_data_dir()),
        missing_metadata: metadata,
        max_connections: (concurrency as u32 + 8).max(16),
        index_build: IndexBuildMode::Inline,
        ..Default::default()
    })
    .unwrap();
    backend.initialize().await.unwrap();
    let db = backend.get_database().await.unwrap();
    let tenant = create_tenant("bench-2006");
    let label = format!("{arm}#{round}");

    // Phase 1: concurrent creates in the corpus's type mix.
    let every = (rows / updates.max(1)).max(1);
    let cache_before = cache_counters(&db).await;
    let started = Instant::now();
    let rx = corpus_lines(corpus, rows);
    let lines = futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|line| (line, rx))
    });
    let results: Vec<(Duration, Option<StoredResource>)> = lines
        .enumerate()
        .map(|(number, line)| {
            let backend = &backend;
            let tenant = &tenant;
            let label = &label;
            async move {
                let resource: serde_json::Value = serde_json::from_str(&line).unwrap();
                let resource_type = resource["resourceType"].as_str().unwrap().to_owned();
                let op = Instant::now();
                let stored = backend
                    .create(tenant, &resource_type, resource, FhirVersion::default())
                    .await
                    .unwrap();
                let elapsed = op.elapsed();
                if number % 100_000 == 0 {
                    eprintln!("{label}: created {number} after {:?}", started.elapsed());
                }
                (elapsed, (number % every == 0).then_some(stored))
            }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;
    let create_elapsed = started.elapsed();
    let mut sample = Vec::new();
    let mut latencies = Vec::with_capacity(results.len());
    for (latency, stored) in results {
        latencies.push(latency);
        sample.extend(stored);
    }
    println!(
        "write | {label} | create | {}",
        Latencies(latencies).summary(create_elapsed)
    );
    let cache_after = cache_counters(&db).await;
    print_cache_delta(&format!("{label} create"), &cache_before, &cache_after);

    // Phase 2: concurrent updates of the sample with unchanged content.
    let cache_before = cache_after;
    let started = Instant::now();
    let latencies: Vec<Duration> = futures::stream::iter(sample.iter())
        .map(|current| {
            let backend = &backend;
            let tenant = &tenant;
            async move {
                let op = Instant::now();
                backend
                    .update(tenant, current, current.content().clone())
                    .await
                    .unwrap();
                op.elapsed()
            }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;
    println!(
        "write | {label} | update | {}",
        Latencies(latencies).summary(started.elapsed())
    );
    let cache_after = cache_counters(&db).await;
    print_cache_delta(&format!("{label} update"), &cache_before, &cache_after);
    drop(sample);

    // Phase 3: `$bulk-submit` ingestion of the same mix into a second tenant,
    // one stream per type, as the submit worker drives it.
    let bulk_tenant = create_tenant("bench-2006-bulk");
    let submission = SubmissionId::generate("bench-2006");
    backend
        .create_submission(&bulk_tenant, &submission, None)
        .await
        .unwrap();
    let manifest = backend
        .add_manifest(
            &bulk_tenant,
            &submission,
            Some("https://provider.example/manifest.json"),
            None,
        )
        .await
        .unwrap()
        .manifest_id;
    let cache_before = cache_after;
    let mut bulk_elapsed = Duration::ZERO;
    let mut bulk_count = 0;
    for (resource_type, n) in shares(bulk_rows) {
        if n == 0 {
            continue;
        }
        let ndjson = first_lines(corpus, resource_type, n);
        let reader = Box::new(tokio::io::BufReader::new(std::io::Cursor::new(ndjson)));
        let started = Instant::now();
        let result = backend
            .process_ndjson_stream(
                &bulk_tenant,
                &submission,
                &manifest,
                resource_type,
                reader,
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();
        bulk_elapsed += started.elapsed();
        assert_eq!(
            result.counts.success as usize, n,
            "{resource_type}: {result:?}"
        );
        bulk_count += n;
    }
    println!(
        "write | {label} | bulk-submit | {bulk_count} | {:.0} | - | - | -",
        bulk_count as f64 / bulk_elapsed.as_secs_f64()
    );
    let cache_after = cache_counters(&db).await;
    print_cache_delta(&format!("{label} bulk-submit"), &cache_before, &cache_after);
    for (key, value) in &cache_after {
        if !CACHE_COUNTERS.contains(&key.as_str()) {
            println!("cache | {label} | {key} | {:.0}", value);
        }
    }

    print_storage(&label, &db).await;
    if metadata {
        print_missing_profile(&db, tenant.tenant_id().as_str()).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 16)]
#[ignore = "#2006 measurement harness; run with --ignored"]
async fn missing_params_cost_bench() {
    let Ok(url) = std::env::var("HFS_TEST_MISSING_COST_MONGODB_URL") else {
        eprintln!("skip: HFS_TEST_MISSING_COST_MONGODB_URL is not set");
        return;
    };
    let corpus = PathBuf::from(
        std::env::var("HFS_TEST_MISSING_COST_CORPUS")
            .expect("HFS_TEST_MISSING_COST_CORPUS: a directory of Synthea <Type>.ndjson files"),
    );
    let arms = std::env::var("HFS_TEST_MISSING_COST_ARMS").unwrap_or_else(|_| "on,off".into());
    println!("| arm | phase | ops | ops/s | p50 ms | p99 ms | p99.9 ms |");
    for (round, arm) in arms.split(',').map(str::trim).enumerate() {
        let metadata = match arm {
            "on" => true,
            "off" => false,
            other => panic!("unknown arm {other:?}; use on or off"),
        };
        run_arm(arm, metadata, round, &url, &corpus).await;
    }
}
