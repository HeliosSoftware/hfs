//! #1976: how many Bundle-create entries one lone MongoDB transaction holds
//! before WiredTiger aborts it, with and without the version-1
//! `resource_history` copy. Ignored by default.
//!
//! A transaction Bundle writes every create twice: the `resources` document and
//! the same payload as version `"1"` in `resource_history`. #1976 asks whether
//! taking the second write out of the transaction is worth a design change; the
//! answer depends on how much of the cache ceiling that copy costs, which this
//! harness measures. It decides nothing: it prints figures and asserts only that
//! the measurement is valid.
//!
//! # What it measures
//!
//! Raw `insert_many` calls inside one session transaction, in chunks of
//! `CHUNK` entries (100, the production `BUNDLE_CREATE_CHUNK_ENTRIES`), until
//! the server aborts it. It does not run the full Bundle path (entry parsing,
//! search extraction, the deferred-index ledger), so its abort position is
//! a property of the write set alone. Two variants run against the same
//! database, in alternating order:
//!
//! - `resource`: the `resources` `insert_many` only;
//! - `resource_history`: the `resources` `insert_many`, then the
//!   `resource_history` `insert_many`, per chunk, the order `send_create_group`
//!   uses (`bundle_creates.rs`).
//!
//! The documents are the exact shapes HFS writes. A template Bundle (the US Core
//! fixture `hfs/tests/inferno/uscore_bundle_patient_85.json`, 267 entries) goes
//! through `process_transaction` on a backend of the chosen `SHAPE`; every
//! `resources` document and its version-1 `resource_history` document are read
//! back with a raw client, checked (version `"1"`, `data` identical in both), and
//! become the templates the run cycles through. Each entry gets a fresh UUIDv7 id
//! (HFS ids are v7, which matters for index locality), the same id in `data.id`,
//! the run's tenant, and `created_at = last_updated = now`. The measurement
//! database is initialised through `MongoBackend::new` + `initialize()` with
//! `IndexBuildMode::Inline`, so the real `resources` and `resource_history`
//! indexes exist; their names and partial filters are printed in
//! `V1_CEILING_SETUP`.
//!
//! An aborted run leaves no data (asserted: its tenant has 0 documents in both
//! collections afterwards). Before each run the harness runs `fsync` and sleeps
//! `SETTLE_SECS`, so the run starts with a clean cache.
//!
//! The raw harness is a model of the Bundle, and `CONTROL_COPIES` is its
//! calibration: one real lone Bundle of that many fixture copies through
//! `process_transaction` on a backend of the same shape, whose own abort entry
//! ("Entry N" in `TooLargeForCache.reason`) is printed as `V1_CEILING_CONTROL`.
//! If the raw figure and the control's disagree, trust the control.
//!
//! # Run
//!
//! `cargo test -p helios-persistence --features mongodb --test mongodb_tests
//! v1_history_copy_ceiling -- --ignored --nocapture --test-threads=1`
//!
//! The harness starts its own mongo:7.0 single-member replica set (`rs0`) with
//! `--wiredTigerCacheSizeGB`, `--oplogSize 1024` and
//! `transactionLifetimeLimitSeconds=900`, as `deferred_search_index.rs` does for
//! #1975. One invocation is one cache size.
//!
//! # Env knobs (defaults in parentheses)
//!
//! - `HFS_TEST_V1_CEILING_WT_CACHE_GB` ("2"): `--wiredTigerCacheSizeGB`.
//! - `HFS_TEST_V1_CEILING_MONGODB_URL` (unset): an external replica set; no
//!   container is started and the cache is whatever that server has (its
//!   `maximum bytes configured` is printed).
//! - `HFS_TEST_V1_CEILING_SHAPE` ("offloaded"): the document shape. `offloaded`
//!   is the `mongodb-elasticsearch` leg (`search_offloaded`, no
//!   `missing_params`); `native` is a natively indexed backend (`missing_params`
//!   and `missing_params_fp` on the `resources` document); `native_deferred`
//!   is `native` plus the `search_rows_pending` marker #1975 stamps. Search rows
//!   are never written by the harness in any shape.
//! - `HFS_TEST_V1_CEILING_VARIANTS` ("resource,resource_history"): the variants.
//! - `HFS_TEST_V1_CEILING_REPEATS` (3): runs per variant; the variant order is
//!   reversed on even repeats.
//! - `HFS_TEST_V1_CEILING_CHUNK` (100): entries per `insert_many`; also the
//!   resolution of an abort position.
//! - `HFS_TEST_V1_CEILING_MAX_ENTRIES` (300000): a safety cap; a run that
//!   reaches it prints `outcome=no_abort` and its figures are lower bounds.
//! - `HFS_TEST_V1_CEILING_SETTLE_SECS` (5): sleep after `fsync` before a run.
//! - `HFS_TEST_V1_CEILING_COMMIT_PROBE` ("off"): `on` also finds, per variant,
//!   the largest entry count that commits (the commit writes the transaction's
//!   oplog entry, #1991), by bisection in fresh databases.
//! - `HFS_TEST_V1_CEILING_COMMIT_RESOLUTION` (500): the bisection stops when the
//!   bracket is this many entries wide or less.
//! - `HFS_TEST_V1_CEILING_CONTROL_COPIES` (0): above 0, one real lone Bundle of
//!   that many fixture copies (267 entries each) goes through
//!   `process_transaction`.
//!
//! # Output
//!
//! One line per event; `grep '^V1_CEILING'`. Sizes are bytes unless a name says
//! `mb` (10^6 bytes).
//!
//! - `V1_CEILING_SETUP`: `wt_cache_gb`, `cache_max_bytes` (the server's own
//!   figure), `mongod`, `shape`, `templates`, `avg_json_bytes`,
//!   `avg_resource_doc_bson_bytes`, `avg_history_doc_bson_bytes`,
//!   `resource_fields`, and the `resources_indexes` / `history_indexes` lists
//!   (`name[keys;unique;partial=...]`).
//! - `V1_CEILING_RUN`: one per run. `outcome` is a cache-class abort
//!   (`too_large` = code 388, `eviction` = 112 "rolled back for eviction",
//!   `unavailable` = 365), `other` (invalid; the test then fails) or
//!   `no_abort`. `phase` is the collection whose `insert_many` failed.
//!   `entries` counts acknowledged chunks only (a chunk is acknowledged when
//!   every `insert_many` of it was); `docs` and `doc_bson_bytes` also count
//!   the `resources` half of a chunk whose `resource_history` insert failed.
//!   `json_bytes` is the compact JSON of the resources of the acknowledged
//!   entries (the figure the issue quotes), `payload_bson_bytes` their BSON
//!   `data`, `doc_bson_bytes` the BSON of the inserted documents without the
//!   `_id` the driver adds.
//!   `dirty_start_bytes`, `dirty_bytes_last` and `update_bytes_last` are the
//!   server's `wiredTiger.cache` "tracked dirty bytes in the cache" before the
//!   run and "tracked dirty bytes" / "bytes allocated for updates" after the
//!   last acknowledged chunk (`na` when absent); `wt_rollbacks` is the delta of
//!   "oldest pinned transaction ID rolled back for eviction"; `loadavg` is the
//!   one-minute load average at the end (`loadavg_start` at the start).
//! - `V1_CEILING_SUMMARY`: per variant, the median, min and max of the entries
//!   and the JSON MB, the spread in percent, the median of dirty bytes per
//!   entry, and the outcome and code tallies.
//! - `V1_CEILING_GAIN`: when both variants ran, `entries_ratio` and
//!   `json_bytes_ratio` (the `resource` median over the `resource_history`
//!   median: how much more a transaction holds without the history copy), and
//!   two estimates of the copy's in-cache cost that do not depend on where the
//!   abort trigger fires: `dirty_per_entry_ratio` and the ratios of dirty bytes
//!   at the same entry count (`dirty_at_half_ratio`, `dirty_at_min_ratio`),
//!   `resource_history` over `resource`.
//! - `V1_CEILING_COMMIT` and `V1_CEILING_COMMIT_RESULT` (`COMMIT_PROBE=on`):
//!   each probe, and per variant `max_committed_entries` / `min_failed_entries`.
//! - `V1_CEILING_CONTROL` (`CONTROL_COPIES>0`): `entries`, `outcome`
//!   (`ok` / `too_large` / `transient` / `other`), `failed_entry` and `secs`.
//!
//! The test asserts only that the harness is valid: at least 200 templates, each
//! pair's `data` identical, every run a cache-class abort or `no_abort`, and no
//! `other` code. It never asserts the gain.

use super::*;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use futures::TryStreamExt;
use mongodb::ClientSession;
use mongodb::bson::{DateTime as BsonDateTime, RawDocumentBuf};
use mongodb::error::{Error as MongoError, ErrorKind as MongoErrorKind, WriteFailure};
use serde_json::Value;
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::mongo::Mongo;

/// The tenant of the template Bundle.
const TEMPLATE_TENANT: &str = "v1c-template";

/// At least this many templates must be captured (the fixture has 267 entries).
const MIN_TEMPLATES: usize = 200;

const TOO_LARGE_CODE: i32 = 388;
const WRITE_CONFLICT_CODE: i32 = 112;
const TEMPORARILY_UNAVAILABLE_CODE: i32 = 365;

/// The tail of mongod's message for a `WriteConflict` that is an eviction
/// rollback (`storage.rs` `EVICTION_ROLLBACK_REASON`).
const EVICTION_ROLLBACK_REASON: &str = "rolled back for eviction";

const CACHE_DIRTY: &str = "tracked dirty bytes in the cache";
const CACHE_UPDATES: &str = "bytes allocated for updates";
const CACHE_MAX: &str = "maximum bytes configured";

// ---------------------------------------------------------------------------
// Knobs
// ---------------------------------------------------------------------------

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn env_usize(name: &str, default: usize) -> usize {
    env_or(name, &default.to_string())
        .parse()
        .unwrap_or_else(|_| panic!("{name} must be an unsigned integer"))
}

fn env_f64(name: &str, default: f64) -> f64 {
    env_or(name, &default.to_string())
        .parse()
        .unwrap_or_else(|_| panic!("{name} must be a number"))
}

/// The document shape of the templates, and with it the backend that makes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// The `mongodb-elasticsearch` leg: search offloaded, no `missing_params`.
    Offloaded,
    /// A natively indexed backend: `missing_params` on the `resources` document.
    Native,
    /// `Native` plus the `search_rows_pending` marker of #1975.
    NativeDeferred,
}

impl Shape {
    fn parse(text: &str) -> Self {
        match text {
            "offloaded" => Self::Offloaded,
            "native" => Self::Native,
            "native_deferred" => Self::NativeDeferred,
            other => panic!(
                "HFS_TEST_V1_CEILING_SHAPE must be offloaded, native or native_deferred, not {other:?}"
            ),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Offloaded => "offloaded",
            Self::Native => "native",
            Self::NativeDeferred => "native_deferred",
        }
    }

    fn search_offloaded(self) -> bool {
        self == Self::Offloaded
    }
}

/// What one transaction writes per entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Variant {
    /// The `resources` document only.
    Resource,
    /// The `resources` document, then its version-1 `resource_history` copy.
    ResourceHistory,
}

impl Variant {
    fn parse(text: &str) -> Self {
        match text.trim() {
            "resource" => Self::Resource,
            "resource_history" => Self::ResourceHistory,
            other => panic!(
                "HFS_TEST_V1_CEILING_VARIANTS takes resource and resource_history, not {other:?}"
            ),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Resource => "resource",
            Self::ResourceHistory => "resource_history",
        }
    }
}

/// Everything a run needs that does not change between runs.
struct RunConfig {
    cache_label: String,
    url: String,
    shape: Shape,
    chunk: usize,
    max_entries: usize,
    settle: Duration,
    repeats: usize,
}

// ---------------------------------------------------------------------------
// Container and backends
// ---------------------------------------------------------------------------

/// A mongo:7.0 single-member replica set with a WiredTiger cache of `cache_gb`
/// (the cleanup-labelled pattern of `deferred_search_index.rs`), and its URL.
async fn start_container(
    cache_gb: &str,
) -> Result<(testcontainers::ContainerAsync<Mongo>, String), String> {
    let started = super::container_cleanup::with_cleanup_label(
        Mongo::default()
            .with_tag("7.0")
            .with_cmd([
                "mongod",
                "--bind_ip_all",
                "--wiredTigerCacheSizeGB",
                cache_gb,
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
    let container = started.map_err(|err| err.to_string())?;
    shared_mongo::initiate_replica_set(&container).await;
    shared_mongo::wait_for_writable_primary(&container).await;
    let host = container.get_host().await.map_err(|err| err.to_string())?;
    let port = container
        .get_host_port_ipv4(27017)
        .await
        .map_err(|err| err.to_string())?;
    Ok((
        container,
        format!("mongodb://{host}:{port}/?directConnection=true"),
    ))
}

/// The backend of `shape`: `Inline` index build, no Bundle deadline, and
/// `Some(0)` for the deferral threshold (a Bundle never defers), so the template
/// Bundle writes the full document shape. Built directly, not through
/// `build_backend`, which also caps the pool.
fn backend_config(url: &str, database: &str, shape: Shape) -> MongoBackendConfig {
    MongoBackendConfig {
        connection_string: url.to_string(),
        database_name: database.to_string(),
        max_connections: Some(8),
        data_dir: Some(repo_data_dir()),
        index_build: IndexBuildMode::Inline,
        bundle_transaction_deadline: None,
        search_offloaded: shape.search_offloaded(),
        transaction_bundle_deferred_index_entries: Some(0),
        ..Default::default()
    }
}

async fn build_backend_for(config: MongoBackendConfig) -> MongoBackend {
    let backend = MongoBackend::new(config).expect("backend config");
    backend.initialize().await.expect("backend initialize");
    backend
}

// ---------------------------------------------------------------------------
// Server readings
// ---------------------------------------------------------------------------

fn number(value: Option<&Bson>) -> Option<i64> {
    match value? {
        Bson::Int32(v) => Some(i64::from(*v)),
        Bson::Int64(v) => Some(*v),
        Bson::Double(v) => Some(*v as i64),
        _ => None,
    }
}

/// `serverStatus.wiredTiger.cache`, or `None` when the command or the section
/// is unavailable.
async fn cache_stats(client: &Client) -> Option<Document> {
    let status = client
        .database("admin")
        .run_command(doc! { "serverStatus": 1 })
        .await
        .ok()?;
    status
        .get_document("wiredTiger")
        .ok()?
        .get_document("cache")
        .ok()
        .cloned()
}

fn show<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "na".to_string(), |v| v.to_string())
}

/// The one-minute load average, `na` when unreadable. Read inside the container,
/// it is the host's (the kernel is shared), which is what #1953 is about.
fn loadavg() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|text| text.split_whitespace().next().map(str::to_string))
        .unwrap_or_else(|| "na".to_string())
}

/// Flushes dirty data and lets the server settle, so a run starts with a clean
/// cache. `fsync` also forces a checkpoint.
async fn settle(client: &Client, pause: Duration) {
    if let Err(err) = client
        .database("admin")
        .run_command(doc! { "fsync": 1 })
        .await
    {
        eprintln!("V1_CEILING_WARN fsync failed: {err}");
    }
    tokio::time::sleep(pause).await;
}

/// The names, keys, uniqueness and partial filter of `collection`'s indexes.
async fn index_summary(db: &mongodb::Database, collection: &str) -> String {
    let reply = db
        .run_command(doc! { "listIndexes": collection })
        .await
        .unwrap_or_else(|err| panic!("listIndexes {collection}: {err}"));
    let batch = reply
        .get_document("cursor")
        .and_then(|cursor| cursor.get_array("firstBatch"))
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for index in batch {
        let Bson::Document(index) = index else {
            continue;
        };
        let name = index.get_str("name").unwrap_or("?");
        let keys = index
            .get_document("key")
            .map(|keys| keys.keys().cloned().collect::<Vec<_>>().join(","))
            .unwrap_or_default();
        let mut parts = vec![keys];
        if index.get_bool("unique").unwrap_or(false) {
            parts.push("unique".to_string());
        }
        if let Ok(filter) = index.get_document("partialFilterExpression") {
            parts.push(format!(
                "partial={}",
                filter.to_string().replace(char::is_whitespace, "")
            ));
        }
        out.push(format!("{name}[{}]", parts.join(";")));
    }
    out.join(" | ")
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// The server's code and message behind a driver error, whichever shape the
/// driver reports it in.
fn server_error(err: &MongoError) -> (Option<i32>, String) {
    match err.kind.as_ref() {
        MongoErrorKind::Command(command) => (Some(command.code), command.message.clone()),
        MongoErrorKind::Write(WriteFailure::WriteError(write)) => {
            (Some(write.code), write.message.clone())
        }
        MongoErrorKind::Write(WriteFailure::WriteConcernError(write)) => {
            (Some(write.code), write.message.clone())
        }
        MongoErrorKind::InsertMany(insert) => {
            let errors = insert.write_errors.as_deref().unwrap_or(&[]);
            if let Some(error) = errors
                .iter()
                .find(|error| error.code == TOO_LARGE_CODE)
                .or_else(|| errors.first())
            {
                (Some(error.code), error.message.clone())
            } else if let Some(concern) = &insert.write_concern_error {
                (Some(concern.code), concern.message.clone())
            } else {
                (None, err.to_string())
            }
        }
        _ => (None, err.to_string()),
    }
}

/// Why a transaction ended, from the server's code: 388 `too_large`, 112 with
/// the eviction text `eviction`, 365 `unavailable`, anything else `other`.
struct Failure {
    /// The collection whose `insert_many` failed, or `commit`.
    phase: &'static str,
    class: &'static str,
    code: Option<i32>,
    message: String,
}

fn classify(phase: &'static str, err: &MongoError) -> Failure {
    let (code, message) = server_error(err);
    let class = match code {
        Some(TOO_LARGE_CODE) => "too_large",
        Some(WRITE_CONFLICT_CODE) if message.contains(EVICTION_ROLLBACK_REASON) => "eviction",
        Some(TEMPORARILY_UNAVAILABLE_CODE) => "unavailable",
        _ => "other",
    };
    Failure {
        phase,
        class,
        code,
        message,
    }
}

// ---------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------

/// One resource as HFS writes it: its `resources` document and its version-1
/// `resource_history` document, without `_id`.
struct Template {
    resource: Document,
    history: Document,
    /// Compact JSON bytes of `data`.
    json_bytes: usize,
    /// BSON bytes of `data`.
    payload_bson_bytes: usize,
    resource_bson_bytes: usize,
    history_bson_bytes: usize,
}

fn bson_len(document: &Document) -> usize {
    RawDocumentBuf::from_document(document)
        .expect("document serializes")
        .as_bytes()
        .len()
}

fn bson_len_all(documents: &[Document]) -> u64 {
    documents.iter().map(|d| bson_len(d) as u64).sum()
}

fn load_fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../hfs/tests/inferno/uscore_bundle_patient_85.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("read fixture"))
        .expect("parse fixture")
}

/// Runs one real transaction Bundle of the fixture on a backend of `shape` and
/// reads back what it wrote: the exact document shapes. Asserts that each
/// resource is version 1 and that its history copy holds the same `data`.
async fn capture_templates(url: &str, shape: Shape, fixture: &Value) -> Vec<Template> {
    let database = build_test_database_name("v1c_template");
    let backend = build_backend_for(backend_config(url, &database, shape)).await;
    backend
        .process_transaction(
            &create_tenant(TEMPLATE_TENANT),
            super::transaction_bundle_load::bundle_entries(fixture, 1),
            FhirVersion::default(),
        )
        .await
        .expect("the template Bundle commits");

    let client = raw_test_client(url).await.expect("raw client");
    let db = client.database(&database);
    let resources: Vec<Document> = db
        .collection::<Document>("resources")
        .find(doc! { "tenant_id": TEMPLATE_TENANT })
        .await
        .expect("find resources")
        .try_collect()
        .await
        .expect("collect resources");
    let histories: Vec<Document> = db
        .collection::<Document>("resource_history")
        .find(doc! { "tenant_id": TEMPLATE_TENANT })
        .await
        .expect("find history")
        .try_collect()
        .await
        .expect("collect history");
    drop(backend);
    db.drop().await.expect("drop template database");

    let identity = |document: &Document| {
        (
            document.get_str("resource_type").unwrap_or("").to_string(),
            document.get_str("id").unwrap_or("").to_string(),
        )
    };
    let mut history_by_identity: HashMap<(String, String), Document> = HashMap::new();
    for history in histories {
        assert_eq!(
            history.get_str("version_id").ok(),
            Some("1"),
            "the template Bundle only creates: every history row is version 1"
        );
        let key = identity(&history);
        assert!(
            history_by_identity.insert(key.clone(), history).is_none(),
            "two history rows for {key:?}"
        );
    }
    assert_eq!(
        history_by_identity.len(),
        resources.len(),
        "one version-1 history row per resource"
    );

    let mut templates = Vec::new();
    for mut resource in resources {
        let key = identity(&resource);
        assert_eq!(resource.get_str("version_id").ok(), Some("1"), "{key:?}");
        let mut history = history_by_identity
            .remove(&key)
            .unwrap_or_else(|| panic!("no version-1 history row for {key:?}"));
        resource.remove("_id");
        history.remove("_id");
        assert_eq!(
            resource.get("data"),
            history.get("data"),
            "the history copy of {key:?} is not the resource's payload"
        );
        if shape == Shape::NativeDeferred {
            // The marker `storage.rs` stamps on a deferred Bundle's resources.
            resource.insert("search_rows_pending", uuid::Uuid::new_v4().to_string());
        }
        let data = resource.get_document("data").expect("data").clone();
        let json_bytes =
            serde_json::to_string(&Bson::Document(data.clone()).into_relaxed_extjson())
                .expect("data as JSON")
                .len();
        templates.push(Template {
            json_bytes,
            payload_bson_bytes: bson_len(&data),
            resource_bson_bytes: bson_len(&resource),
            history_bson_bytes: bson_len(&history),
            resource,
            history,
        });
    }
    assert!(
        templates.len() >= MIN_TEMPLATES,
        "only {} templates captured",
        templates.len()
    );
    templates
}

/// A copy of `template` as a new entry: a fresh UUIDv7 id (in `data.id` too),
/// the run's tenant and `now` as `created_at` and `last_updated`. Fields keep
/// the template's order.
fn make_entry(template: &Template, tenant: &str, now: BsonDateTime) -> (Document, Document) {
    let id = uuid::Uuid::now_v7().to_string();
    let mut resource = template.resource.clone();
    let mut history = template.history.clone();
    for document in [&mut resource, &mut history] {
        document.insert("tenant_id", tenant);
        document.insert("id", id.as_str());
        document.insert("created_at", now);
        document.insert("last_updated", now);
        if let Ok(data) = document.get_document_mut("data") {
            data.insert("id", id.as_str());
        }
    }
    (resource, history)
}

// ---------------------------------------------------------------------------
// One transaction
// ---------------------------------------------------------------------------

/// A cache reading after an acknowledged chunk.
#[derive(Clone, Copy)]
struct Sample {
    entries: usize,
    dirty: Option<i64>,
    updates: Option<i64>,
}

/// What a transaction wrote before it ended: acknowledged chunks only.
#[derive(Default)]
struct Progress {
    entries: usize,
    docs: usize,
    json_bytes: u64,
    payload_bson_bytes: u64,
    doc_bson_bytes: u64,
    samples: Vec<Sample>,
}

/// One transaction to write: what, for whom, and how far.
struct Job {
    variant: Variant,
    tenant: String,
    /// Stop (without failure) after this many entries.
    limit: usize,
    /// Read the cache after every acknowledged chunk.
    sample: bool,
}

/// The outcome of one scan run.
struct RunResult {
    variant: Variant,
    /// `None` when the run reached `MAX_ENTRIES` without an abort.
    class: Option<&'static str>,
    code: Option<i32>,
    entries: usize,
    json_bytes: u64,
    dirty_start: Option<i64>,
    dirty_last: Option<i64>,
    samples: Vec<Sample>,
}

impl RunResult {
    /// Dirty cache bytes the run added per entry, from the last sample.
    fn dirty_per_entry(&self) -> Option<f64> {
        let (start, last) = (self.dirty_start?, self.dirty_last?);
        (self.entries > 0).then(|| (last - start) as f64 / self.entries as f64)
    }

    /// Dirty cache bytes the run had added once it had `entries` entries.
    fn dirty_delta_at(&self, entries: usize) -> Option<f64> {
        let start = self.dirty_start?;
        let sample = self.samples.iter().find(|s| s.entries == entries)?;
        Some((sample.dirty? - start) as f64)
    }

    fn json_mb(&self) -> f64 {
        self.json_bytes as f64 / 1e6
    }
}

/// The measurement database and everything needed to run transactions on it.
struct Measure {
    cfg: RunConfig,
    /// Runs the transactions.
    client: Client,
    /// Reads `serverStatus` and runs `fsync`, so a transaction's own pool is
    /// never starved by a sample.
    status: Client,
    db_name: String,
    templates: Vec<Template>,
}

impl Measure {
    fn db(&self) -> mongodb::Database {
        self.client.database(&self.db_name)
    }

    async fn sample(&self, entries: usize) -> Sample {
        let cache = cache_stats(&self.status).await;
        Sample {
            entries,
            dirty: cache.as_ref().and_then(|c| number(c.get(CACHE_DIRTY))),
            updates: cache.as_ref().and_then(|c| number(c.get(CACHE_UPDATES))),
        }
    }

    /// Inserts entries of `job` in chunks on `session` until `job.limit` or the
    /// first failed `insert_many`. Per chunk: `resources`, then (for
    /// `ResourceHistory`) `resource_history`.
    async fn write_entries(
        &self,
        session: &mut ClientSession,
        db: &mongodb::Database,
        job: &Job,
    ) -> (Progress, Option<Failure>) {
        let resources = db.collection::<Document>("resources");
        let histories = db.collection::<Document>("resource_history");
        let mut progress = Progress::default();
        while progress.entries < job.limit {
            let n = self.cfg.chunk.min(job.limit - progress.entries);
            let mut resource_docs = Vec::with_capacity(n);
            let mut history_docs = Vec::with_capacity(n);
            let (mut json, mut payload) = (0u64, 0u64);
            for k in 0..n {
                let template = &self.templates[(progress.entries + k) % self.templates.len()];
                let (resource, history) = make_entry(template, &job.tenant, BsonDateTime::now());
                json += template.json_bytes as u64;
                payload += template.payload_bson_bytes as u64;
                resource_docs.push(resource);
                if job.variant == Variant::ResourceHistory {
                    history_docs.push(history);
                }
            }
            let resource_bytes = bson_len_all(&resource_docs);
            if let Err(err) = resources
                .insert_many(resource_docs)
                .session(&mut *session)
                .await
            {
                return (progress, Some(classify("resources", &err)));
            }
            progress.docs += n;
            progress.doc_bson_bytes += resource_bytes;
            if job.variant == Variant::ResourceHistory {
                let history_bytes = bson_len_all(&history_docs);
                if let Err(err) = histories
                    .insert_many(history_docs)
                    .session(&mut *session)
                    .await
                {
                    return (progress, Some(classify("resource_history", &err)));
                }
                progress.docs += n;
                progress.doc_bson_bytes += history_bytes;
            }
            progress.entries += n;
            progress.json_bytes += json;
            progress.payload_bson_bytes += payload;
            if job.sample {
                let sample = self.sample(progress.entries).await;
                progress.samples.push(sample);
            }
        }
        (progress, None)
    }

    /// One scan run: write entries until the server aborts the transaction (or
    /// `MAX_ENTRIES`), abort it, and print `V1_CEILING_RUN`.
    async fn scan(&self, variant: Variant, repeat: usize) -> RunResult {
        let tenant = format!("v1c-run-{repeat}-{}", variant.label());
        settle(&self.status, self.cfg.settle).await;
        let rollbacks_before =
            super::transaction_bundle_load::eviction_rollbacks(&self.cfg.url).await;
        let load_start = loadavg();
        let dirty_start = cache_stats(&self.status)
            .await
            .and_then(|c| number(c.get(CACHE_DIRTY)));

        let db = self.db();
        let started = Instant::now();
        let mut session = self.client.start_session().await.expect("start session");
        session
            .start_transaction()
            .await
            .expect("start transaction");
        let job = Job {
            variant,
            tenant: tenant.clone(),
            limit: self.cfg.max_entries,
            sample: true,
        };
        let (progress, failure) = self.write_entries(&mut session, &db, &job).await;
        let _ = session.abort_transaction().await;
        let secs = started.elapsed().as_secs_f64();

        let rollbacks = super::transaction_bundle_load::eviction_rollbacks(&self.cfg.url).await
            - rollbacks_before;
        for collection in ["resources", "resource_history"] {
            let left = db
                .collection::<Document>(collection)
                .count_documents(doc! { "tenant_id": &tenant })
                .await
                .expect("count the aborted run's documents");
            assert_eq!(left, 0, "an aborted run leaves no {collection} documents");
        }

        let last = progress.samples.last().copied();
        let dirty_last = last.and_then(|s| s.dirty);
        let (outcome, code, phase, message) = match &failure {
            Some(f) => (f.class, f.code, f.phase, f.message.as_str()),
            None => ("no_abort", None, "na", ""),
        };
        println!(
            "V1_CEILING_RUN wt_cache_gb={} shape={} variant={} run={}/{} outcome={outcome} \
             code={} phase={phase} entries={} docs={} json_bytes={} payload_bson_bytes={} \
             doc_bson_bytes={} dirty_start_bytes={} dirty_bytes_last={} update_bytes_last={} \
             secs={secs:.1} wt_rollbacks={rollbacks} loadavg_start={load_start} loadavg={} \
             msg={:?}",
            self.cfg.cache_label,
            self.cfg.shape.label(),
            variant.label(),
            repeat,
            self.cfg.repeats,
            show(code),
            progress.entries,
            progress.docs,
            progress.json_bytes,
            progress.payload_bson_bytes,
            progress.doc_bson_bytes,
            show(dirty_start),
            show(dirty_last),
            show(last.and_then(|s| s.updates)),
            loadavg(),
            message.chars().take(200).collect::<String>(),
        );
        if let Some(f) = &failure
            && f.class == "other"
        {
            panic!(
                "the {} run ended with an unexpected code {}: {}",
                variant.label(),
                show(f.code),
                f.message
            );
        }

        RunResult {
            variant,
            class: failure.as_ref().map(|f| f.class),
            code,
            entries: progress.entries,
            json_bytes: progress.json_bytes,
            dirty_start,
            dirty_last,
            samples: progress.samples,
        }
    }
}

// ---------------------------------------------------------------------------
// Summaries
// ---------------------------------------------------------------------------

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    }
}

fn min_of(values: &[f64]) -> f64 {
    values.iter().copied().fold(f64::INFINITY, f64::min)
}

fn max_of(values: &[f64]) -> f64 {
    values.iter().copied().fold(f64::NEG_INFINITY, f64::max)
}

fn ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator > 0.0 {
        numerator / denominator
    } else {
        f64::NAN
    }
}

/// `value:count` pairs, sorted, for a tally of labels.
fn tally<T: ToString>(values: impl Iterator<Item = T>) -> String {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for value in values {
        *counts.entry(value.to_string()).or_default() += 1;
    }
    let mut pairs: Vec<_> = counts.into_iter().collect();
    pairs.sort();
    pairs
        .iter()
        .map(|(value, count)| format!("{value}:{count}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn of_variant(results: &[RunResult], variant: Variant) -> Vec<&RunResult> {
    results.iter().filter(|r| r.variant == variant).collect()
}

fn print_summary(cfg: &RunConfig, variant: Variant, runs: &[&RunResult]) {
    let entries: Vec<f64> = runs.iter().map(|r| r.entries as f64).collect();
    let json: Vec<f64> = runs.iter().map(|r| r.json_mb()).collect();
    let dirty: Vec<f64> = runs.iter().filter_map(|r| r.dirty_per_entry()).collect();
    let entries_median = median(&entries);
    let spread = 100.0 * (max_of(&entries) - min_of(&entries)) / entries_median;
    println!(
        "V1_CEILING_SUMMARY wt_cache_gb={} shape={} variant={} runs={} entries_median={:.0} \
         entries_min={:.0} entries_max={:.0} spread_pct={spread:.1} json_mb_median={:.2} \
         json_mb_min={:.2} json_mb_max={:.2} dirty_bytes_per_entry_median={:.0} \
         outcomes={} codes={}",
        cfg.cache_label,
        cfg.shape.label(),
        variant.label(),
        runs.len(),
        entries_median,
        min_of(&entries),
        max_of(&entries),
        median(&json),
        min_of(&json),
        max_of(&json),
        median(&dirty),
        tally(runs.iter().map(|r| r.class.unwrap_or("no_abort"))),
        tally(runs.iter().map(|r| show(r.code))),
    );
}

/// The in-cache cost at the same entry count: the median over runs of the dirty
/// bytes added by `entries` entries.
fn median_dirty_at(runs: &[&RunResult], entries: usize) -> f64 {
    let values: Vec<f64> = runs
        .iter()
        .filter_map(|r| r.dirty_delta_at(entries))
        .collect();
    median(&values)
}

fn print_gain(cfg: &RunConfig, results: &[RunResult]) {
    let without = of_variant(results, Variant::Resource);
    let with = of_variant(results, Variant::ResourceHistory);
    if without.is_empty() || with.is_empty() {
        return;
    }
    let med = |runs: &[&RunResult], f: fn(&RunResult) -> f64| {
        median(&runs.iter().map(|r| f(r)).collect::<Vec<_>>())
    };
    let entries_ratio = ratio(
        med(&without, |r| r.entries as f64),
        med(&with, |r| r.entries as f64),
    );
    let json_ratio = ratio(
        med(&without, RunResult::json_mb),
        med(&with, RunResult::json_mb),
    );
    let per_entry = |runs: &[&RunResult]| {
        median(
            &runs
                .iter()
                .filter_map(|r| r.dirty_per_entry())
                .collect::<Vec<_>>(),
        )
    };
    let dirty_per_entry_ratio = ratio(per_entry(&with), per_entry(&without));

    // A like-for-like reading: both variants' dirty bytes at the same entry
    // counts, taken at half of and at the smallest ceiling any run reached.
    let smallest = results.iter().map(|r| r.entries).min().unwrap_or(0);
    let grid = |entries: usize| entries / cfg.chunk * cfg.chunk;
    let at_min = grid(smallest);
    let at_half = grid(smallest / 2);
    let at_ratio = |entries: usize| {
        ratio(
            median_dirty_at(&with, entries),
            median_dirty_at(&without, entries),
        )
    };
    let censored = results.iter().any(|r| r.class.is_none());
    println!(
        "V1_CEILING_GAIN wt_cache_gb={} shape={} entries_ratio={entries_ratio:.3} \
         json_bytes_ratio={json_ratio:.3} dirty_per_entry_ratio={dirty_per_entry_ratio:.3} \
         dirty_at_half_ratio={:.3} dirty_at_min_ratio={:.3} ref_half={at_half} ref_min={at_min} \
         censored={censored}",
        cfg.cache_label,
        cfg.shape.label(),
        at_ratio(at_half),
        at_ratio(at_min),
    );
}

// ---------------------------------------------------------------------------
// The commit ceiling
// ---------------------------------------------------------------------------

impl Measure {
    /// Writes `n` entries in a fresh database and commits. `None` when the
    /// transaction committed, else why it did not.
    async fn probe(&self, variant: Variant, n: usize) -> Option<Failure> {
        let database = build_test_database_name("v1c_probe");
        let backend =
            build_backend_for(backend_config(&self.cfg.url, &database, self.cfg.shape)).await;
        let db = self.client.database(&database);
        let started = Instant::now();
        let mut session = self.client.start_session().await.expect("start session");
        session
            .start_transaction()
            .await
            .expect("start transaction");
        let job = Job {
            variant,
            tenant: "v1c-probe".to_string(),
            limit: n,
            sample: false,
        };
        let (progress, failure) = self.write_entries(&mut session, &db, &job).await;
        let failure = match failure {
            Some(failure) => {
                let _ = session.abort_transaction().await;
                Some(failure)
            }
            None => match session.commit_transaction().await {
                Ok(()) => None,
                Err(err) => {
                    let _ = session.abort_transaction().await;
                    Some(classify("commit", &err))
                }
            },
        };
        let secs = started.elapsed().as_secs_f64();
        println!(
            "V1_CEILING_COMMIT wt_cache_gb={} shape={} variant={} entries={n} written={} \
             outcome={} phase={} code={} secs={secs:.1} loadavg={}",
            self.cfg.cache_label,
            self.cfg.shape.label(),
            variant.label(),
            progress.entries,
            failure.as_ref().map_or("committed", |f| f.class),
            failure.as_ref().map_or("na", |f| f.phase),
            show(failure.as_ref().and_then(|f| f.code)),
            loadavg(),
        );
        drop(backend);
        db.drop().await.expect("drop probe database");
        settle(&self.status, self.cfg.settle).await;
        if let Some(f) = &failure
            && f.class == "other"
        {
            panic!(
                "a commit probe of {n} entries ended with an unexpected code {}: {}",
                show(f.code),
                f.message
            );
        }
        failure
    }

    /// The largest entry count that commits, by bisection. `ceiling` is the
    /// smallest abort position a scan run reached: the count to try first.
    async fn commit_ceiling(&self, variant: Variant, ceiling: usize, resolution: usize) {
        let chunk = self.cfg.chunk;
        let grid = |entries: usize| entries / chunk * chunk;
        let mut failed = Some(grid(ceiling));
        let mut committed = 0;
        if self.probe(variant, grid(ceiling)).await.is_none() {
            committed = grid(ceiling);
            failed = None;
        } else {
            let mut lo = grid(ceiling / 2);
            for _ in 0..3 {
                if lo < chunk {
                    break;
                }
                if self.probe(variant, lo).await.is_none() {
                    committed = lo;
                    break;
                }
                failed = Some(lo);
                lo = grid(lo / 2);
            }
        }
        while let Some(high) = failed {
            if committed == 0 || high - committed <= resolution {
                break;
            }
            let mid = grid((committed + high) / 2);
            if mid <= committed || mid >= high {
                break;
            }
            if self.probe(variant, mid).await.is_none() {
                committed = mid;
            } else {
                failed = Some(mid);
            }
        }
        println!(
            "V1_CEILING_COMMIT_RESULT wt_cache_gb={} shape={} variant={} \
             max_committed_entries={committed} min_failed_entries={} start={}",
            self.cfg.cache_label,
            self.cfg.shape.label(),
            variant.label(),
            show(failed),
            grid(ceiling),
        );
    }
}

// ---------------------------------------------------------------------------
// The control
// ---------------------------------------------------------------------------

/// The entry number in a `TooLargeForCache` reason ("Entry N ...").
fn failed_entry(reason: &str) -> Option<usize> {
    let rest = &reason[reason.find("Entry ")? + "Entry ".len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

impl Measure {
    /// One real lone Bundle of `copies` fixture copies through
    /// `process_transaction` on a backend of the run's shape: the position
    /// where the backend's own transaction aborts, to check the raw harness
    /// against.
    async fn control(&self, copies: usize, fixture: &Value) {
        let mut config = backend_config(
            &self.cfg.url,
            &build_test_database_name("v1c_control"),
            self.cfg.shape,
        );
        match self.cfg.shape {
            // The default threshold (the gate's room, 4,000 entries): a Bundle
            // of that size takes the deferred path.
            Shape::NativeDeferred => config.transaction_bundle_deferred_index_entries = None,
            Shape::Native => println!(
                "V1_CEILING_WARN shape=native writes the search rows inside the control \
                 Bundle's transaction; its abort position is not comparable with the raw \
                 harness, which writes none"
            ),
            Shape::Offloaded => {}
        }
        let database = config.database_name.clone();
        let backend = build_backend_for(config).await;
        let entries = super::transaction_bundle_load::bundle_entries(fixture, copies);
        let count = entries.len();
        settle(&self.status, self.cfg.settle).await;

        let started = Instant::now();
        let result = backend
            .process_transaction(
                &create_tenant("v1c-control"),
                entries,
                FhirVersion::default(),
            )
            .await;
        let secs = started.elapsed().as_secs_f64();
        let (outcome, failed) = match &result {
            Ok(_) => ("ok".to_string(), None),
            Err(TransactionError::TooLargeForCache { reason, .. }) => {
                ("too_large".to_string(), failed_entry(reason))
            }
            Err(TransactionError::Transient { .. }) => ("transient".to_string(), None),
            Err(other) => (format!("other:{other}"), None),
        };
        println!(
            "V1_CEILING_CONTROL wt_cache_gb={} shape={} copies={copies} entries={count} \
             outcome={outcome} failed_entry={} secs={secs:.1} loadavg={}",
            self.cfg.cache_label,
            self.cfg.shape.label(),
            show(failed),
            loadavg(),
        );
        drop(backend);
        self.client
            .database(&database)
            .drop()
            .await
            .expect("drop control database");
    }
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

/// #1976: the lone-transaction ceiling with and without the version-1
/// `resource_history` copy, for one WiredTiger cache size. Ignored by default;
/// see the module docs for the knobs and the output.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "#1976 ceiling harness; run with --ignored"]
async fn v1_history_copy_ceiling() {
    let cache_gb = env_or("HFS_TEST_V1_CEILING_WT_CACHE_GB", "2");
    assert!(
        env_f64("HFS_TEST_V1_CEILING_WT_CACHE_GB", 2.0) > 0.0,
        "HFS_TEST_V1_CEILING_WT_CACHE_GB must be positive"
    );
    let shape = Shape::parse(&env_or("HFS_TEST_V1_CEILING_SHAPE", "offloaded"));
    let variants: Vec<Variant> =
        env_or("HFS_TEST_V1_CEILING_VARIANTS", "resource,resource_history")
            .split(',')
            .map(Variant::parse)
            .collect();
    let repeats = env_usize("HFS_TEST_V1_CEILING_REPEATS", 3);
    let chunk = env_usize("HFS_TEST_V1_CEILING_CHUNK", 100);
    let max_entries = env_usize("HFS_TEST_V1_CEILING_MAX_ENTRIES", 300_000);
    let settle_secs = env_usize("HFS_TEST_V1_CEILING_SETTLE_SECS", 5);
    let commit_probe = matches!(
        env_or("HFS_TEST_V1_CEILING_COMMIT_PROBE", "off").as_str(),
        "on" | "1" | "true"
    );
    let commit_resolution = env_usize("HFS_TEST_V1_CEILING_COMMIT_RESOLUTION", 500);
    let control_copies = env_usize("HFS_TEST_V1_CEILING_CONTROL_COPIES", 0);
    assert!(
        !variants.is_empty() && chunk > 0 && repeats > 0 && max_entries >= chunk,
        "variants, repeats, chunk and max entries must be set"
    );

    // Keep the container alive (and removed on drop) for the whole test.
    let _container;
    let (url, cache_label) = match std::env::var("HFS_TEST_V1_CEILING_MONGODB_URL") {
        Ok(url) => (url, "external".to_string()),
        Err(_) => match start_container(&cache_gb).await {
            Ok((container, url)) => {
                _container = container;
                (url, cache_gb.clone())
            }
            Err(err) => {
                eprintln!("skip: could not start a Mongo container: {err}");
                return;
            }
        },
    };

    let fixture = load_fixture();
    let templates = capture_templates(&url, shape, &fixture).await;

    // The measurement database: the real indexes, through the backend.
    let db_name = build_test_database_name("v1c_measure");
    let _backend = build_backend_for(backend_config(&url, &db_name, shape)).await;
    let measure = Measure {
        cfg: RunConfig {
            cache_label,
            url: url.clone(),
            shape,
            chunk,
            max_entries,
            settle: Duration::from_secs(settle_secs as u64),
            repeats,
        },
        client: raw_test_client(&url).await.expect("raw client"),
        status: raw_test_client(&url).await.expect("status client"),
        db_name,
        templates,
    };

    let db = measure.db();
    let resources_indexes = index_summary(&db, "resources").await;
    let history_indexes = index_summary(&db, "resource_history").await;
    if shape != Shape::Offloaded && !resources_indexes.contains("idx_resources_missing_params") {
        println!(
            "V1_CEILING_WARN idx_resources_missing_params is absent from the resources collection"
        );
    }
    let cache = cache_stats(&measure.status).await;
    let mongod = measure
        .status
        .database("admin")
        .run_command(doc! { "buildInfo": 1 })
        .await
        .ok()
        .and_then(|info| info.get_str("version").ok().map(str::to_string));
    let count = measure.templates.len();
    let average = |f: fn(&Template) -> usize| {
        measure.templates.iter().map(f).sum::<usize>() as f64 / count as f64
    };
    println!(
        "V1_CEILING_SETUP wt_cache_gb={} cache_max_bytes={} mongod={} shape={} templates={count} \
         avg_json_bytes={:.0} avg_resource_doc_bson_bytes={:.0} avg_history_doc_bson_bytes={:.0} \
         chunk={chunk} max_entries={max_entries} repeats={repeats} settle_secs={settle_secs} \
         resource_fields={} resources_indexes={resources_indexes} history_indexes={history_indexes}",
        measure.cfg.cache_label,
        show(cache.as_ref().and_then(|c| number(c.get(CACHE_MAX)))),
        show(mongod),
        shape.label(),
        average(|t| t.json_bytes),
        average(|t| t.resource_bson_bytes),
        average(|t| t.history_bson_bytes),
        measure.templates[0]
            .resource
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(","),
    );

    let mut results: Vec<RunResult> = Vec::new();
    for repeat in 1..=repeats {
        let mut order = variants.clone();
        if repeat.is_multiple_of(2) {
            order.reverse();
        }
        for variant in order {
            results.push(measure.scan(variant, repeat).await);
        }
    }
    for variant in &variants {
        let runs = of_variant(&results, *variant);
        print_summary(&measure.cfg, *variant, &runs);
    }
    print_gain(&measure.cfg, &results);

    if commit_probe {
        for variant in &variants {
            let ceiling = of_variant(&results, *variant)
                .iter()
                .map(|r| r.entries)
                .min()
                .expect("a run per variant");
            measure
                .commit_ceiling(*variant, ceiling, commit_resolution)
                .await;
        }
    }
    if control_copies > 0 {
        measure.control(control_copies, &fixture).await;
    }

    // Harness validity only; the gain is read from the lines above.
    for result in &results {
        assert!(
            matches!(
                result.class,
                None | Some("too_large") | Some("eviction") | Some("unavailable")
            ),
            "a run ended with {:?} (code {})",
            result.class,
            show(result.code)
        );
    }
}
