//! Bulk-submit ingest benchmark for every backend (#947, #1000, #945).
//!
//! Drives the real ingest path — `StreamingBulkSubmitProvider::process_ndjson_stream`,
//! the same call the bulk-submit worker makes per manifest file — against local
//! NDJSON files, on a fresh store, and reports the achieved rate plus the
//! per-phase breakdown collected by [`helios_persistence::perf`].
//!
//! The worker's HTTP fetch, lease keeper, and manifest polling are deliberately
//! out of the loop: this measures the write path #947 is about, with no network
//! or fixture-server variance between runs.
//!
//! Three more stages mirror what the `hfs` binary wires around that call:
//!
//! * `--reindex` runs the post-manifest rebuild the worker's deferred hook
//!   (`ReindexOnFinish`) fires: a `ReindexOperation` whose source is the
//!   primary and whose writers are the server's — the primary while it indexes
//!   locally, plus Elasticsearch under `--es` — timed and phased on its own.
//! * `--es URL` wraps the primary in the primary + Elasticsearch composite the
//!   server builds (sqlite-es, pg-es, mongo-es, s3-es): search offloaded on the
//!   primary, the Elasticsearch backend sharing its registry. Without
//!   `--defer-index` every committed batch is indexed into Elasticsearch
//!   during the ingest through the `IngestIndexSink` (`DEFER_INDEXING=false`),
//!   exactly as `IndexingSubmitJobs` attaches it; the sink is drained at the
//!   end, timed separately. With `--defer-index` Elasticsearch is filled by
//!   `--reindex`, as the server's deferred hook does.
//! * `--file-concurrency N` ingests the input files N at a time, the worker's
//!   per-file fan-out: independent Tokio tasks on standalone PostgreSQL
//!   (#1467), `buffer_unordered` from one task everywhere else; each file is
//!   its own `process_ndjson_stream` call with its own `file_url`.
//!
//! The phase table needs `--cfg perf_phases`; without it the ingest still runs
//! and reports its rate, but every phase reads zero (see
//! [`helios_persistence::perf`] for why that is a cfg and not a feature).
//!
//! ```text
//! RUSTFLAGS='--cfg perf_phases' \
//!   cargo run --release -p helios-persistence \
//!     --features sqlite,postgres,mongodb,elasticsearch,s3 \
//!     --example bulk_submit_bench -- \
//!     --backend postgres --es http://localhost:59200 --batch 1000 --limit 20000 \
//!     /path/to/Observation.ndjson /path/to/Condition.ndjson
//! ```
//!
//! Always run from the repository root (or pass `--data-dir`): with the full
//! `search-parameters-r4.json` missing, the registry silently falls back to a
//! handful of embedded params and every index count is meaningless.
//!
//! Options (every `HFS_*` default below is the server's own knob, read the same
//! way the server reads it; a flag overrides the variable):
//!
//! * `--backend sqlite|postgres|mongodb|s3` primary store (default: sqlite)
//! * `--db PATH`        SQLite database file to create (deleted first; default: a temp file)
//! * `--pg-url URL`     PostgreSQL admin connection (default: `HFS_PG_URL`, else
//!                      `postgres://hfs:hfs@localhost:55432/hfs`); a fresh
//!                      database `bench_<pid>` (`--pg-db NAME`) is created on
//!                      that server for the run and dropped at the end
//! * `--mongo-uri URI`  MongoDB connection string (default: `HFS_MONGODB_URL`,
//!                      `HFS_MONGODB_URI`, `HFS_MONGO_URI`, else `mongodb://localhost:27017`)
//! * `--mongo-db NAME`  MongoDB database, dropped before and after (default: `hfs_bulk_submit_bench`)
//! * `--s3-endpoint URL` S3-compatible endpoint, path-style (default: `HFS_S3_ENDPOINT`,
//!                      else `http://localhost:59000`, a local MinIO)
//! * `--s3-bucket NAME` bucket, created if missing (default: `HFS_S3_BUCKET`, else `bsperf`)
//! * `--s3-prefix P`    key prefix for the run (default: `bench/<pid>`); objects
//!                      under it are deleted at the end
//! * `--s3-access-key K --s3-secret-key S --s3-region R` credentials for the
//!                      SDK's provider chain when `AWS_*` are unset (default:
//!                      `minioadmin`/`minioadmin`, `us-east-1`)
//! * `--es URL`         Elasticsearch node: run the primary + Elasticsearch composite
//! * `--es-prefix P`    index prefix (default: `hfs_bench_<pid>`); the run's indices
//!                      are deleted at the end
//! * `--es-refresh-interval S` index `refresh_interval` (`HFS_ELASTICSEARCH_REFRESH_INTERVAL`, `1s`)
//! * `--es-write-refresh P` write refresh policy (`HFS_ELASTICSEARCH_WRITE_REFRESH`, `false`)
//! * `--es-reindex-refresh P` reindex refresh policy (`HFS_ELASTICSEARCH_REINDEX_REFRESH`, unset)
//! * `--es-bulk-max-bytes N` `_bulk` body cap (`HFS_ELASTICSEARCH_BULK_MAX_BYTES`, 10 MiB)
//! * `--es-bulk-concurrency N` concurrent `_bulk` requests (`HFS_ELASTICSEARCH_BULK_CONCURRENCY`, 1)
//! * `--es-prepare-threads N` extraction threads (`HFS_ELASTICSEARCH_REINDEX_PREPARE_THREADS`, 0 = auto)
//! * `--sink-queue N --sink-concurrency N --sink-coalesce N --sink-page-bytes N --sink-max-wait SECS`
//!                      the ingest-time index sink (`HFS_BULK_SUBMIT_INDEX_QUEUE` 16,
//!                      `_INDEX_CONCURRENCY` 4, `_INDEX_COALESCE` 4, `_INDEX_PAGE_BYTES` 32 MiB,
//!                      `_INDEX_MAX_WAIT` 30)
//! * `--file-concurrency N` files ingested at a time (`HFS_BULK_SUBMIT_FILE_CONCURRENCY`, 1;
//!                      forced to 1 on SQLite, as the worker does)
//! * `--limit N`        resources to ingest per input file (default: all)
//! * `--batch N`        entries per transaction (default: 100; the server's is 1000)
//! * `--defer-index`    set `defer_indexing`, the `HFS_BULK_SUBMIT_DEFER_INDEXING` path
//! * `--reindex`        after the ingest, run the post-manifest rebuild the submit
//!                      worker fires under deferred indexing, timed and phased on
//!                      its own; `--batch 1000 --defer-index --reindex` is the
//!                      server's default path end to end
//! * `--reindex-batch N` resources per rebuild page (`HFS_REINDEX_BATCH_SIZE`, 1000)
//! * `--reindex-bytes N` byte cap of one rebuild page (`HFS_REINDEX_BATCH_BYTES`, 32 MiB)
//! * `--seed`           seed the spec SearchParameters and CompartmentDefinitions
//!   into the bench tenant first, as `hfs` does for every provisioned tenant,
//!   so the rebuild reads a persisted overlay the way a real tenant's does
//!   (the seeding itself is not timed)
//! * `--reindex-streams N` page streams per type of the rebuild (`HFS_REINDEX_STREAMS`,
//!                      `auto`/0 = half the cores, at most 8; capped by every writer:
//!                      SQLite 1, PostgreSQL its reindex width, MongoDB/Elasticsearch 8)
//! * `--pg-reindex-width N` concurrent reindex write groups on standalone PostgreSQL
//!                      (`HFS_POSTGRES_REINDEX_WIDTH`, `auto`/0 = half the pool, at most 8;
//!                      pg-es keeps 1, as PostgreSQL is not a writer there)
//! * `--bulk-index-rebuild` drop the value indexes for the rebuild and build them
//!                      sorted at the end (`HFS_BULK_SUBMIT_BULK_INDEX_REBUILD`)
//! * `--second-manifest N` after the main stages, ingest N more resources of the
//!                      first input file's type (the lines after `--limit`, so
//!                      fresh ids) as a second manifest of the same submission and
//!                      run the rebuild the submit worker's hook would fire for it,
//!                      timed and phased on its own (#945 L3). Needs
//!                      `--defer-index --reindex`.
//! * `--second-manifest-scope types|auto|ids|since` how that rebuild is scoped:
//!                      `types` walks the manifest's types tenant-wide (what the
//!                      hook did before #945 L3); `auto` decides as the worker
//!                      does — the manifest's receipts name its resources when it
//!                      has at most `--reindex-ids-max` entries, otherwise a
//!                      `last_updated` floor of the manifest's registration time
//!                      less `--reindex-since-margin`; `ids` and `since` force
//!                      one of the two (default: `auto`)
//! * `--reindex-since-margin SECS` (`HFS_BULK_SUBMIT_REINDEX_SINCE_MARGIN`, 900)
//! * `--reindex-ids-max N` (`HFS_BULK_SUBMIT_REINDEX_IDS_MAX`, 50000)
//! * `--data-dir DIR`   directory holding `search-parameters-r4.json` (default: `./data`)
//! * `--keep`           leave the database / indices / objects behind for inspection
//! * `--no-fk`          `PRAGMA foreign_keys = OFF`, to price the index rows' parent check
//! * `--no-phases`      collection off, to price the instrumentation itself
//!
//! Also honoured, through the library itself: `HFS_INDEX_THREADS`,
//! `HFS_BULK_SUBMIT_S3_INGEST_CONCURRENCY`, `HFS_PG_*` pool knobs,
//! `HFS_MONGODB_*` (including `HFS_MONGODB_INDEX_BUILD` and the reindex/search
//! admission variables), `HFS_ELASTICSEARCH_{USERNAME,PASSWORD,NESTED_OBJECTS_LIMIT,
//! MAX_TERMS_COUNT,REQUEST_TIMEOUT_MS}`, `HFS_COMPOSITE_SYNC_MODE`, `HFS_PERF_PHASES`.

#[cfg(feature = "elasticsearch")]
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
#[cfg(feature = "elasticsearch")]
use helios_persistence::core::{BatchCommitObserver, ResourceStorage};
use helios_persistence::core::{
    BulkProcessingOptions, BulkSubmitJobStore, DEFAULT_REINDEX_IDS_MAX,
    DEFAULT_REINDEX_SINCE_MARGIN, SubmissionId, deferred_reindex_ledger_ids,
    deferred_reindex_since,
};
use helios_persistence::perf;
use helios_persistence::search::{
    DEFERRED_REINDEX_BATCH_SIZE, ReindexError, ReindexOperation, ReindexProgress, ReindexRequest,
    ReindexStatus, TenantSearchRegistries,
};
#[cfg(feature = "elasticsearch")]
use helios_persistence::search::{ReindexSource, ReindexTarget};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

#[cfg(feature = "elasticsearch")]
use helios_persistence::backends::elasticsearch::ElasticsearchBackend;
#[cfg(feature = "mongodb")]
use helios_persistence::backends::mongodb::MongoBackend;
#[cfg(feature = "postgres")]
use helios_persistence::backends::postgres::PostgresBackend;
#[cfg(feature = "s3")]
use helios_persistence::backends::s3::S3Backend;
#[cfg(feature = "elasticsearch")]
use helios_persistence::composite::{IngestIndexSink, IngestIndexSinkConfig};

/// `HFS_REINDEX_BATCH_BYTES`'s default: the server caps a rebuild page at 32 MiB.
const DEFAULT_REINDEX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Backend {
    Sqlite,
    Postgres,
    Mongodb,
    S3,
}

impl Backend {
    fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "sqlite" => Self::Sqlite,
            "postgres" | "postgresql" | "pg" => Self::Postgres,
            "mongodb" | "mongo" => Self::Mongodb,
            "s3" | "minio" => Self::S3,
            other => panic!("--backend must be sqlite|postgres|mongodb|s3, got `{other}`"),
        }
    }
}

struct Args {
    backend: Backend,
    db: PathBuf,
    files: Vec<PathBuf>,
    limit: Option<usize>,
    batch: Option<u32>,
    defer_index: bool,
    data_dir: PathBuf,
    keep: bool,
    /// Seed the spec conformance resources into the tenant before the ingest.
    seed: bool,
    /// Experiment switch: run with `PRAGMA foreign_keys = OFF`, to price the
    /// parent-row check `search_index`'s foreign key costs on every index row.
    no_fk: bool,
    /// Run with phase collection off — the rate an uninstrumented build
    /// achieves, and the way to price the instrumentation itself.
    no_phases: bool,
    /// After the ingest, run the post-manifest reindex the submit worker
    /// fires under `HFS_BULK_SUBMIT_DEFER_INDEXING`, timed and phased
    /// separately. With `--defer-index` this is the server's default path
    /// end to end.
    reindex: bool,
    /// Page size for `--reindex` (default: the hook's, `DEFERRED_REINDEX_BATCH_SIZE`).
    reindex_batch: Option<u32>,
    /// Byte cap of one `--reindex` page (`HFS_REINDEX_BATCH_BYTES`).
    reindex_bytes: u64,
    /// Page streams per type of the `--reindex` run (`HFS_REINDEX_STREAMS`; 0 = auto).
    reindex_streams: u32,
    /// Standalone PostgreSQL reindex width (`HFS_POSTGRES_REINDEX_WIDTH`; 0 = auto).
    pg_reindex_width: usize,
    /// `--reindex` in bulk index rebuild mode (`HFS_BULK_SUBMIT_BULK_INDEX_REBUILD`).
    bulk_index_rebuild: bool,
    /// Resources of a second manifest ingested after the main stages, whose
    /// own rebuild is then timed (#945 L3); `0` = no second manifest.
    second_manifest: usize,
    /// How the second manifest's rebuild is scoped.
    second_scope: SecondScope,
    /// `HFS_BULK_SUBMIT_REINDEX_SINCE_MARGIN`, in seconds.
    reindex_since_margin_secs: u64,
    /// `HFS_BULK_SUBMIT_REINDEX_IDS_MAX`.
    reindex_ids_max: u64,
    /// Files ingested at a time, the worker's per-file fan-out.
    file_concurrency: usize,
    pg_url: String,
    pg_db: Option<String>,
    mongo_uri: String,
    mongo_db: String,
    s3_endpoint: String,
    s3_bucket: String,
    s3_prefix: String,
    s3_region: String,
    s3_access_key: String,
    s3_secret_key: String,
    es: Option<String>,
    es_prefix: String,
    es_refresh_interval: String,
    es_write_refresh: String,
    es_reindex_refresh: Option<String>,
    es_bulk_max_bytes: usize,
    es_bulk_concurrency: usize,
    es_prepare_threads: usize,
    sink_queue: usize,
    sink_concurrency: usize,
    sink_coalesce: usize,
    sink_page_bytes: usize,
    sink_max_wait_secs: u64,
}

/// How `--second-manifest`'s rebuild is scoped (#945 L3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SecondScope {
    /// The manifest's types, tenant-wide: the hook before #945 L3.
    Types,
    /// The worker's decision: receipts up to `--reindex-ids-max`, else a floor.
    Auto,
    /// The resources the manifest's receipts name.
    Ids,
    /// A `last_updated` floor of the manifest's registration time less the margin.
    Since,
}

impl SecondScope {
    fn parse(s: &str) -> Self {
        match s {
            "types" => Self::Types,
            "auto" => Self::Auto,
            "ids" => Self::Ids,
            "since" => Self::Since,
            other => panic!("--second-manifest-scope must be types|auto|ids|since, not {other}"),
        }
    }
}

fn env_str(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_parse<T: std::str::FromStr>(name: &str) -> Option<T> {
    env_str(name).and_then(|v| v.parse().ok())
}

fn env_bool(name: &str, default: bool) -> bool {
    match env_str(name) {
        Some(v) => matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on"),
        None => default,
    }
}

/// `auto` (the server's default for these settings) or unset is `0`.
fn env_auto_or_count(name: &str) -> u32 {
    match env_str(name) {
        Some(v) if v.eq_ignore_ascii_case("auto") => 0,
        Some(v) => v
            .parse()
            .unwrap_or_else(|_| panic!("{name} must be `auto` or a number")),
        None => 0,
    }
}

fn parse_args() -> Args {
    let pid = std::process::id();
    let mut args = Args {
        backend: Backend::Sqlite,
        db: PathBuf::from("/tmp/hfs-bulk-submit-bench.db"),
        files: Vec::new(),
        limit: None,
        batch: None,
        defer_index: false,
        data_dir: PathBuf::from("data"),
        keep: false,
        seed: false,
        no_fk: false,
        no_phases: false,
        reindex: false,
        reindex_batch: env_parse("HFS_REINDEX_BATCH_SIZE"),
        reindex_bytes: env_parse("HFS_REINDEX_BATCH_BYTES").unwrap_or(DEFAULT_REINDEX_BYTES),
        reindex_streams: env_auto_or_count("HFS_REINDEX_STREAMS"),
        pg_reindex_width: env_auto_or_count("HFS_POSTGRES_REINDEX_WIDTH") as usize,
        bulk_index_rebuild: env_bool("HFS_BULK_SUBMIT_BULK_INDEX_REBUILD", false),
        second_manifest: 0,
        second_scope: SecondScope::Auto,
        reindex_since_margin_secs: env_parse("HFS_BULK_SUBMIT_REINDEX_SINCE_MARGIN")
            .unwrap_or(DEFAULT_REINDEX_SINCE_MARGIN.as_secs()),
        reindex_ids_max: env_parse("HFS_BULK_SUBMIT_REINDEX_IDS_MAX")
            .unwrap_or(DEFAULT_REINDEX_IDS_MAX),
        file_concurrency: env_parse("HFS_BULK_SUBMIT_FILE_CONCURRENCY").unwrap_or(1),
        pg_url: env_str("HFS_PG_URL")
            .or_else(|| env_str("HFS_DATABASE_URL").filter(|u| u.starts_with("postgres")))
            .unwrap_or_else(|| "postgres://hfs:hfs@localhost:55432/hfs".to_string()),
        pg_db: None,
        mongo_uri: env_str("HFS_MONGODB_URL")
            .or_else(|| env_str("HFS_MONGODB_URI"))
            .or_else(|| env_str("HFS_MONGO_URI"))
            .unwrap_or_else(|| "mongodb://localhost:27017".to_string()),
        mongo_db: "hfs_bulk_submit_bench".to_string(),
        s3_endpoint: env_str("HFS_S3_ENDPOINT")
            .unwrap_or_else(|| "http://localhost:59000".to_string()),
        s3_bucket: env_str("HFS_S3_BUCKET").unwrap_or_else(|| "bsperf".to_string()),
        s3_prefix: format!("bench/{pid}"),
        s3_region: env_str("AWS_REGION").unwrap_or_else(|| "us-east-1".to_string()),
        s3_access_key: env_str("AWS_ACCESS_KEY_ID").unwrap_or_else(|| "minioadmin".to_string()),
        s3_secret_key: env_str("AWS_SECRET_ACCESS_KEY").unwrap_or_else(|| "minioadmin".to_string()),
        es: None,
        es_prefix: format!("hfs_bench_{pid}"),
        es_refresh_interval: env_str("HFS_ELASTICSEARCH_REFRESH_INTERVAL")
            .unwrap_or_else(|| "1s".to_string()),
        es_write_refresh: env_str("HFS_ELASTICSEARCH_WRITE_REFRESH")
            .unwrap_or_else(|| "false".to_string()),
        es_reindex_refresh: env_str("HFS_ELASTICSEARCH_REINDEX_REFRESH"),
        es_bulk_max_bytes: env_parse("HFS_ELASTICSEARCH_BULK_MAX_BYTES")
            .unwrap_or(10 * 1024 * 1024),
        es_bulk_concurrency: env_parse("HFS_ELASTICSEARCH_BULK_CONCURRENCY").unwrap_or(1),
        es_prepare_threads: env_parse("HFS_ELASTICSEARCH_REINDEX_PREPARE_THREADS").unwrap_or(0),
        sink_queue: env_parse("HFS_BULK_SUBMIT_INDEX_QUEUE").unwrap_or(16),
        sink_concurrency: env_parse("HFS_BULK_SUBMIT_INDEX_CONCURRENCY").unwrap_or(4),
        sink_coalesce: env_parse("HFS_BULK_SUBMIT_INDEX_COALESCE").unwrap_or(4),
        sink_page_bytes: env_parse("HFS_BULK_SUBMIT_INDEX_PAGE_BYTES").unwrap_or(32 * 1024 * 1024),
        sink_max_wait_secs: env_parse("HFS_BULK_SUBMIT_INDEX_MAX_WAIT").unwrap_or(30),
    };
    let mut it = std::env::args().skip(1);
    fn value(it: &mut impl Iterator<Item = String>, flag: &str) -> String {
        it.next().unwrap_or_else(|| panic!("{flag} needs a value"))
    }
    fn number<T: std::str::FromStr>(it: &mut impl Iterator<Item = String>, flag: &str) -> T {
        value(it, flag)
            .parse()
            .unwrap_or_else(|_| panic!("{flag} must be a number"))
    }
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--backend" => args.backend = Backend::parse(&value(&mut it, "--backend")),
            "--db" => args.db = PathBuf::from(value(&mut it, "--db")),
            "--limit" => args.limit = Some(number(&mut it, "--limit")),
            "--batch" => args.batch = Some(number(&mut it, "--batch")),
            "--defer-index" => args.defer_index = true,
            "--no-fk" => args.no_fk = true,
            "--no-phases" => args.no_phases = true,
            "--reindex" => args.reindex = true,
            "--bulk-index-rebuild" => args.bulk_index_rebuild = true,
            "--reindex-batch" => args.reindex_batch = Some(number(&mut it, "--reindex-batch")),
            "--reindex-bytes" => args.reindex_bytes = number(&mut it, "--reindex-bytes"),
            "--reindex-streams" => args.reindex_streams = number(&mut it, "--reindex-streams"),
            "--pg-reindex-width" => args.pg_reindex_width = number(&mut it, "--pg-reindex-width"),
            "--second-manifest" => args.second_manifest = number(&mut it, "--second-manifest"),
            "--second-manifest-scope" => {
                args.second_scope = SecondScope::parse(&value(&mut it, "--second-manifest-scope"))
            }
            "--reindex-since-margin" => {
                args.reindex_since_margin_secs = number(&mut it, "--reindex-since-margin")
            }
            "--reindex-ids-max" => args.reindex_ids_max = number(&mut it, "--reindex-ids-max"),
            "--file-concurrency" => args.file_concurrency = number(&mut it, "--file-concurrency"),
            "--keep" => args.keep = true,
            "--seed" => args.seed = true,
            "--data-dir" => args.data_dir = PathBuf::from(value(&mut it, "--data-dir")),
            "--pg-url" => args.pg_url = value(&mut it, "--pg-url"),
            "--pg-db" => args.pg_db = Some(value(&mut it, "--pg-db")),
            "--mongo-uri" => args.mongo_uri = value(&mut it, "--mongo-uri"),
            "--mongo-db" => args.mongo_db = value(&mut it, "--mongo-db"),
            "--s3-endpoint" => args.s3_endpoint = value(&mut it, "--s3-endpoint"),
            "--s3-bucket" => args.s3_bucket = value(&mut it, "--s3-bucket"),
            "--s3-prefix" => args.s3_prefix = value(&mut it, "--s3-prefix"),
            "--s3-region" => args.s3_region = value(&mut it, "--s3-region"),
            "--s3-access-key" => args.s3_access_key = value(&mut it, "--s3-access-key"),
            "--s3-secret-key" => args.s3_secret_key = value(&mut it, "--s3-secret-key"),
            "--es" => args.es = Some(value(&mut it, "--es")),
            "--es-prefix" => args.es_prefix = value(&mut it, "--es-prefix"),
            "--es-refresh-interval" => {
                args.es_refresh_interval = value(&mut it, "--es-refresh-interval")
            }
            "--es-write-refresh" => args.es_write_refresh = value(&mut it, "--es-write-refresh"),
            "--es-reindex-refresh" => {
                args.es_reindex_refresh = Some(value(&mut it, "--es-reindex-refresh"))
            }
            "--es-bulk-max-bytes" => {
                args.es_bulk_max_bytes = number(&mut it, "--es-bulk-max-bytes")
            }
            "--es-bulk-concurrency" => {
                args.es_bulk_concurrency = number(&mut it, "--es-bulk-concurrency")
            }
            "--es-prepare-threads" => {
                args.es_prepare_threads = number(&mut it, "--es-prepare-threads")
            }
            "--sink-queue" => args.sink_queue = number(&mut it, "--sink-queue"),
            "--sink-concurrency" => args.sink_concurrency = number(&mut it, "--sink-concurrency"),
            "--sink-coalesce" => args.sink_coalesce = number(&mut it, "--sink-coalesce"),
            "--sink-page-bytes" => args.sink_page_bytes = number(&mut it, "--sink-page-bytes"),
            "--sink-max-wait" => args.sink_max_wait_secs = number(&mut it, "--sink-max-wait"),
            "--help" | "-h" => {
                println!("usage: bulk_submit_bench [options] FILE.ndjson [FILE.ndjson ...]");
                println!("see the module docs at the top of examples/bulk_submit_bench.rs");
                std::process::exit(0);
            }
            other if other.starts_with("--") => panic!("unknown option {other}"),
            other => args.files.push(PathBuf::from(other)),
        }
    }
    assert!(
        !args.files.is_empty(),
        "usage: bulk_submit_bench [options] FILE.ndjson [FILE.ndjson ...]"
    );
    assert!(
        args.second_manifest == 0 || (args.defer_index && args.reindex),
        "--second-manifest measures the deferred rebuild: it needs --defer-index --reindex"
    );
    args.file_concurrency = args.file_concurrency.max(1);
    args
}

/// Lines `[skip, skip + take)` of an NDJSON file (blank lines not counted):
/// the resources of a second manifest, distinct from the first `skip` lines
/// the main stage ingested.
fn read_slice(path: &Path, skip: usize, take: usize) -> (Vec<u8>, usize) {
    use std::io::{BufRead, BufReader};

    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut buf = Vec::new();
    let mut seen = 0usize;
    let mut lines = 0usize;
    let mut line = String::new();
    while lines < take {
        line.clear();
        if reader.read_line(&mut line).expect("read line") == 0 {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        seen += 1;
        if seen <= skip {
            continue;
        }
        buf.extend_from_slice(line.as_bytes());
        lines += 1;
    }
    (buf, lines)
}

/// A bounded prefix of an NDJSON file, held in memory so the measured run is
/// not paced by page-cache misses on a multi-GB source file.
fn read_prefix(path: &Path, limit: Option<usize>) -> (Vec<u8>, usize) {
    use std::io::{BufRead, BufReader};

    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut buf = Vec::new();
    let mut lines = 0usize;
    let mut line = String::new();
    loop {
        if let Some(limit) = limit {
            if lines >= limit {
                break;
            }
        }
        line.clear();
        if reader.read_line(&mut line).expect("read line") == 0 {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        buf.extend_from_slice(line.as_bytes());
        lines += 1;
    }
    (buf, lines)
}

fn resource_type_of(path: &Path) -> String {
    // Synthea names its exports `<ResourceType>.ndjson` or
    // `<ResourceType>.<timestamp>.ndjson`.
    path.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.split('.').next())
        .expect("file name")
        .to_string()
}

/// A receipt is keyed by `(manifest, file_url, line_number)` (#457), so every
/// input needs its own `file_url`. SQLite keeps the type-named URL it always
/// used, so its stored rows are byte for byte what earlier runs wrote.
fn file_url_of(backend: Backend, path: &Path, resource_type: &str) -> String {
    match backend {
        Backend::Sqlite => format!("bench://{resource_type}"),
        _ => format!(
            "bench://{}",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("input")
        ),
    }
}

struct Input {
    resource_type: String,
    file_url: String,
    bytes: Vec<u8>,
    lines: usize,
}

/// The primary store under test, held concretely so each backend's own
/// constructor, counters, and teardown stay reachable.
enum Primary {
    Sqlite(Arc<SqliteBackend>),
    #[cfg(feature = "postgres")]
    Postgres(Arc<PostgresBackend>),
    #[cfg(feature = "mongodb")]
    Mongodb(Arc<MongoBackend>),
    #[cfg(feature = "s3")]
    S3(Arc<S3Backend>),
}

impl Primary {
    /// The job store the worker drives: `create_submission`, `add_manifest`,
    /// and `process_ndjson_stream` all come through here.
    fn jobs(&self) -> Arc<dyn BulkSubmitJobStore> {
        match self {
            Self::Sqlite(b) => b.clone() as Arc<dyn BulkSubmitJobStore>,
            #[cfg(feature = "postgres")]
            Self::Postgres(b) => b.clone() as Arc<dyn BulkSubmitJobStore>,
            #[cfg(feature = "mongodb")]
            Self::Mongodb(b) => b.clone() as Arc<dyn BulkSubmitJobStore>,
            #[cfg(feature = "s3")]
            Self::S3(b) => b.clone() as Arc<dyn BulkSubmitJobStore>,
        }
    }

    fn registries(&self) -> Arc<TenantSearchRegistries> {
        match self {
            Self::Sqlite(b) => b.tenant_registries().clone(),
            #[cfg(feature = "postgres")]
            Self::Postgres(b) => b.tenant_registries().clone(),
            #[cfg(feature = "mongodb")]
            Self::Mongodb(b) => b.tenant_registries().clone(),
            #[cfg(feature = "s3")]
            Self::S3(b) => b.tenant_registries().clone(),
        }
    }

    /// The primary as the sink's read-back source and the composite's store.
    #[cfg(feature = "elasticsearch")]
    fn storage(&self) -> Arc<dyn ResourceStorage> {
        match self {
            Self::Sqlite(b) => b.clone() as Arc<dyn ResourceStorage>,
            #[cfg(feature = "postgres")]
            Self::Postgres(b) => b.clone() as Arc<dyn ResourceStorage>,
            #[cfg(feature = "mongodb")]
            Self::Mongodb(b) => b.clone() as Arc<dyn ResourceStorage>,
            #[cfg(feature = "s3")]
            Self::S3(b) => b.clone() as Arc<dyn ResourceStorage>,
        }
    }

    #[cfg(feature = "elasticsearch")]
    fn reindex_source(&self) -> Arc<dyn ReindexSource> {
        match self {
            Self::Sqlite(b) => b.clone() as Arc<dyn ReindexSource>,
            #[cfg(feature = "postgres")]
            Self::Postgres(b) => b.clone() as Arc<dyn ReindexSource>,
            #[cfg(feature = "mongodb")]
            Self::Mongodb(b) => b.clone() as Arc<dyn ReindexSource>,
            #[cfg(feature = "s3")]
            Self::S3(b) => b.clone() as Arc<dyn ReindexSource>,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Sqlite(_) => "sqlite",
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => "postgres",
            #[cfg(feature = "mongodb")]
            Self::Mongodb(_) => "mongodb",
            #[cfg(feature = "s3")]
            Self::S3(_) => "s3",
        }
    }
}

/// What has to be torn down after the run, besides the primary itself.
enum Teardown {
    None,
    #[cfg(feature = "postgres")]
    Postgres {
        admin: tokio_postgres::Client,
        dbname: String,
    },
    #[cfg(feature = "s3")]
    S3 {
        client: aws_sdk_s3::Client,
        bucket: String,
        prefix: String,
    },
}

fn setup_sqlite(args: &Args, offloaded: bool) -> Arc<SqliteBackend> {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", args.db.display()));
    }

    let config = SqliteBackendConfig {
        data_dir: Some(args.data_dir.clone()),
        enable_foreign_keys: !args.no_fk,
        ..Default::default()
    };
    let mut backend = SqliteBackend::with_config(&args.db, config).expect("open backend");
    backend.init_schema().expect("init schema");
    if offloaded {
        // sqlite-es: the server offloads search after the schema is in place
        // (`create_sqlite_backend`, then `set_search_offloaded(true)`).
        backend.set_search_offloaded(true);
    }
    let backend = Arc::new(backend);
    // Experiment hook: arbitrary SQL against the fresh schema before the
    // ingest (drop an index, recreate a trigger, tune FTS5), so a schema
    // idea can be priced without a build.
    if let Ok(sql) = std::env::var("HFS_EXPERIMENT_SQL") {
        let conn = rusqlite::Connection::open(&args.db).expect("open db");
        conn.execute_batch(&sql).expect("HFS_EXPERIMENT_SQL");
        println!("(experiment SQL applied: {})", sql.replace('\n', " "));
    }
    backend
}

/// A fresh database on the `--pg-url` server, the schema initialised the way
/// `start_postgres` / `start_postgres_elasticsearch` do it.
#[cfg(feature = "postgres")]
async fn setup_postgres(
    args: &Args,
    offloaded: bool,
    file_concurrency: usize,
) -> (Arc<PostgresBackend>, Teardown) {
    // The URL's pool/timeout knobs are the server's: `HFS_PG_*` overrides
    // apply exactly as in `create_postgres_backend`.
    let mut config =
        PostgresBackend::config_from_connection_string(&args.pg_url).expect("parse --pg-url");
    let dbname = args
        .pg_db
        .clone()
        .unwrap_or_else(|| format!("bench_{}", std::process::id()));
    assert!(
        dbname
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_'),
        "--pg-db must be [A-Za-z0-9_]+"
    );
    let admin_conn = format!(
        "host={} port={} user={} dbname={}{}",
        config.host,
        config.port,
        config.user,
        config.dbname,
        config
            .password
            .as_deref()
            .map(|p| format!(" password={p}"))
            .unwrap_or_default()
    );
    let (admin, connection) = tokio_postgres::connect(&admin_conn, tokio_postgres::NoTls)
        .await
        .unwrap_or_else(|e| panic!("connect to the PostgreSQL admin database: {e}"));
    tokio::spawn(async move {
        let _ = connection.await;
    });
    admin
        .batch_execute(&format!("DROP DATABASE IF EXISTS {dbname} WITH (FORCE)"))
        .await
        .expect("drop a stale bench database");
    admin
        .batch_execute(&format!("CREATE DATABASE {dbname}"))
        .await
        .expect("create the bench database");

    config.dbname = dbname.clone();
    config.data_dir = Some(args.data_dir.clone());
    let mut backend = PostgresBackend::new(config)
        .await
        .expect("connect to the bench database");
    if offloaded {
        // pg-es offloads search before the schema is initialised (#1663).
        backend.set_search_offloaded(true);
    } else {
        // Standalone PostgreSQL schedules the rebuild `HFS_POSTGRES_REINDEX_WIDTH`
        // groups wide — `auto` = half the pool, at most 8 — capped by the pool
        // (`postgres_reindex_selection`); pg-es keeps the default width of 1.
        // The width no longer follows the file concurrency.
        let _ = file_concurrency;
        let pool = backend.config().max_connections.max(1);
        let width = match args.pg_reindex_width {
            0 => (pool / 2).clamp(1, 8),
            requested => requested.min(pool),
        };
        backend = backend.with_reindex_concurrency(width);
    }
    backend.init_schema().await.expect("init schema");
    (Arc::new(backend), Teardown::Postgres { admin, dbname })
}

/// A dropped-and-recreated database on `--mongo-uri`, configured the way
/// `build_mongodb_config` does it from the same `HFS_MONGODB_*` variables.
#[cfg(feature = "mongodb")]
async fn setup_mongodb(args: &Args, offloaded: bool) -> Arc<MongoBackend> {
    use helios_persistence::backends::mongodb::{IndexBuildMode, MongoBackendConfig};

    let env = |name: &str| std::env::var(name).ok();
    let mut config = MongoBackendConfig {
        connection_string: args.mongo_uri.clone(),
        database_name: args.mongo_db.clone(),
        data_dir: Some(args.data_dir.clone()),
        search_offloaded: offloaded,
        ..Default::default()
    };
    if let Some(v) = env("HFS_MONGODB_MAX_CONNECTIONS").and_then(|v| v.parse().ok()) {
        config.max_connections = v;
    }
    if let Some(raw) = env("HFS_MONGODB_INDEX_BUILD") {
        config.index_build = raw
            .parse::<IndexBuildMode>()
            .unwrap_or_else(|e| panic!("HFS_MONGODB_INDEX_BUILD: {e}"));
    }
    config
        .apply_reindex_env(&env)
        .unwrap_or_else(|e| panic!("{e}"));
    config
        .apply_transaction_bundle_env(&env)
        .unwrap_or_else(|e| panic!("{e}"));
    config
        .apply_search_env(&env)
        .unwrap_or_else(|e| panic!("{e}"));
    let backend = MongoBackend::new(config).expect("open backend");

    // A fresh database per run: index state is what decays here (#1000), so a
    // run that inherited the previous one's indexes would not be comparable.
    backend
        .get_database()
        .await
        .expect("connect")
        .drop()
        .await
        .expect("drop bench database");
    backend.init_schema().await.expect("init schema");
    Arc::new(backend)
}

/// `--seed`: the spec SearchParameters and CompartmentDefinitions `hfs` seeds
/// into every provisioned tenant at startup (`seed_tenant_conformance`), so a
/// rebuild reads the tenant's persisted overlay the way a real tenant's does.
async fn seed_conformance(
    primary: &Primary,
    data_dir: &std::path::Path,
    tenant_id: &str,
) -> helios_persistence::search::seeder::SeedOutcome {
    use helios_persistence::search::seeder::seed_tenant_conformance;
    let version = helios_fhir::FhirVersion::default_enabled();
    match primary {
        Primary::Sqlite(b) => {
            seed_tenant_conformance(b.as_ref(), version, data_dir, tenant_id, None).await
        }
        #[cfg(feature = "postgres")]
        Primary::Postgres(b) => {
            seed_tenant_conformance(b.as_ref(), version, data_dir, tenant_id, None).await
        }
        #[cfg(feature = "mongodb")]
        Primary::Mongodb(b) => {
            seed_tenant_conformance(b.as_ref(), version, data_dir, tenant_id, None).await
        }
        #[cfg(feature = "s3")]
        Primary::S3(b) => {
            seed_tenant_conformance(b.as_ref(), version, data_dir, tenant_id, None).await
        }
    }
}

/// The three registry tiers `start_s3_elasticsearch` loads into S3's own
/// registries (S3 has no `data_dir` and loads none itself), shared with the
/// Elasticsearch backend so it extracts with the full parameter set.
#[cfg(feature = "s3")]
fn populate_registry(
    registry: &mut helios_persistence::search::SearchParameterRegistry,
    data_dir: &Path,
) {
    use helios_persistence::search::SearchParameterLoader;

    let loader = SearchParameterLoader::new(helios_fhir::FhirVersion::default_enabled());
    let mut counts = [0usize; 3];
    if let Ok(params) = loader.load_embedded() {
        for p in params {
            if registry.register(p).is_ok() {
                counts[0] += 1;
            }
        }
    }
    match loader.load_from_spec_file(data_dir) {
        Ok(params) => {
            for p in params {
                if registry.register(p).is_ok() {
                    counts[1] += 1;
                }
            }
        }
        Err(e) => eprintln!(
            "warning: could not load the spec SearchParameters from {}: {e}",
            data_dir.display()
        ),
    }
    if let Ok((params, _files)) = loader.load_custom_from_directory_with_files(data_dir) {
        for p in params {
            if registry.register(p).is_ok() {
                counts[2] += 1;
            }
        }
    }
    println!(
        "s3 search registry: {} embedded + {} spec + {} custom params",
        counts[0], counts[1], counts[2]
    );
}

/// A per-run prefix in `--s3-bucket` on `--s3-endpoint`, path-style, built
/// through `S3Backend::from_env_async` like `start_s3` /
/// `start_s3_elasticsearch` (credentials come from the `AWS_*` variables,
/// seeded from the flags in `main` before the runtime starts).
#[cfg(feature = "s3")]
async fn setup_s3(args: &Args, with_es: bool) -> (Arc<S3Backend>, Teardown) {
    use aws_config::{BehaviorVersion, Region};
    use helios_persistence::backends::s3::{S3BackendConfig, S3TenancyMode};

    let sdk_config = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new(args.s3_region.clone()))
        .endpoint_url(args.s3_endpoint.clone())
        .load()
        .await;
    let client = aws_sdk_s3::Client::from_conf(
        aws_sdk_s3::config::Builder::from(&sdk_config)
            .force_path_style(true)
            .build(),
    );
    if client
        .head_bucket()
        .bucket(&args.s3_bucket)
        .send()
        .await
        .is_err()
    {
        client
            .create_bucket()
            .bucket(&args.s3_bucket)
            .send()
            .await
            .unwrap_or_else(|e| panic!("create bucket {}: {e}", args.s3_bucket));
        println!("created bucket {}", args.s3_bucket);
    }

    let config = S3BackendConfig {
        tenancy_mode: S3TenancyMode::PrefixPerTenant {
            bucket: args.s3_bucket.clone(),
        },
        prefix: Some(args.s3_prefix.clone()),
        region: Some(args.s3_region.clone()),
        endpoint_url: Some(args.s3_endpoint.clone()),
        force_path_style: true,
        allow_http: true,
        validate_buckets_on_startup: true,
        ..Default::default()
    };
    let backend = S3Backend::from_env_async(config)
        .await
        .unwrap_or_else(|e| panic!("create S3 backend: {e}"));
    if with_es {
        populate_registry(
            &mut backend.tenant_registries().base().write(),
            &args.data_dir,
        );
    }
    (
        Arc::new(backend),
        Teardown::S3 {
            client,
            bucket: args.s3_bucket.clone(),
            prefix: args.s3_prefix.clone(),
        },
    )
}

async fn setup_primary(
    args: &Args,
    offloaded: bool,
    file_concurrency: usize,
) -> (Primary, Teardown) {
    match args.backend {
        Backend::Sqlite => (
            Primary::Sqlite(setup_sqlite(args, offloaded)),
            Teardown::None,
        ),
        Backend::Postgres => {
            #[cfg(feature = "postgres")]
            {
                let (backend, teardown) = setup_postgres(args, offloaded, file_concurrency).await;
                (Primary::Postgres(backend), teardown)
            }
            #[cfg(not(feature = "postgres"))]
            {
                let _ = (offloaded, file_concurrency);
                panic!("--backend postgres needs a build with --features postgres");
            }
        }
        Backend::Mongodb => {
            #[cfg(feature = "mongodb")]
            {
                (
                    Primary::Mongodb(setup_mongodb(args, offloaded).await),
                    Teardown::None,
                )
            }
            #[cfg(not(feature = "mongodb"))]
            {
                panic!("--backend mongodb needs a build with --features mongodb");
            }
        }
        Backend::S3 => {
            #[cfg(feature = "s3")]
            {
                let (backend, teardown) = setup_s3(args, args.es.is_some()).await;
                (Primary::S3(backend), teardown)
            }
            #[cfg(not(feature = "s3"))]
            {
                panic!("--backend s3 needs a build with --features s3");
            }
        }
    }
}

/// The Elasticsearch half of a composite run.
#[cfg(feature = "elasticsearch")]
struct EsSide {
    backend: Arc<ElasticsearchBackend>,
    /// A second client on the same node for the harness's own refresh,
    /// `_count`, `_stats`, and index deletion; the backend's is crate-private.
    client: elasticsearch::Elasticsearch,
    prefix: String,
}

#[cfg(feature = "elasticsearch")]
impl EsSide {
    fn pattern(&self) -> String {
        format!("{}_*", self.prefix)
    }

    async fn refresh(&self) {
        use elasticsearch::indices::IndicesRefreshParts;
        let pattern = self.pattern();
        let indices = [pattern.as_str()];
        let _ = self
            .client
            .indices()
            .refresh(IndicesRefreshParts::Index(&indices))
            .send()
            .await;
    }

    async fn count(&self, index: &str) -> i64 {
        use elasticsearch::CountParts;
        match self
            .client
            .count(CountParts::Index(&[index]))
            .ignore_unavailable(true)
            .send()
            .await
        {
            Ok(resp) => resp
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| v.get("count").and_then(|c| c.as_i64()))
                .unwrap_or(-1),
            Err(_) => -1,
        }
    }

    async fn store_bytes(&self) -> i64 {
        use elasticsearch::indices::IndicesStatsParts;
        let pattern = self.pattern();
        let indices = [pattern.as_str()];
        match self
            .client
            .indices()
            .stats(IndicesStatsParts::IndexMetric(&indices, &["store"]))
            .send()
            .await
        {
            Ok(resp) => resp
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| {
                    v.pointer("/_all/primaries/store/size_in_bytes")
                        .and_then(|b| b.as_i64())
                })
                .unwrap_or(-1),
            Err(_) => -1,
        }
    }

    async fn list_indices(&self) -> Vec<String> {
        use elasticsearch::cat::CatIndicesParts;
        let pattern = self.pattern();
        let indices = [pattern.as_str()];
        let Ok(resp) = self
            .client
            .cat()
            .indices(CatIndicesParts::Index(&indices))
            .format("json")
            .h(&["index"])
            .send()
            .await
        else {
            return Vec::new();
        };
        resp.json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|row| row.get("index").and_then(|i| i.as_str()).map(String::from))
            .collect()
    }

    async fn delete_indices(&self) -> usize {
        use elasticsearch::indices::IndicesDeleteParts;
        let names = self.list_indices().await;
        if names.is_empty() {
            return 0;
        }
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        match self
            .client
            .indices()
            .delete(IndicesDeleteParts::Index(&refs))
            .send()
            .await
        {
            Ok(resp) if resp.status_code().is_success() => names.len(),
            Ok(resp) => {
                eprintln!(
                    "warning: deleting {} indices: status {}",
                    names.len(),
                    resp.status_code()
                );
                0
            }
            Err(e) => {
                eprintln!("warning: deleting {} indices: {e}", names.len());
                0
            }
        }
    }
}

/// The Elasticsearch backend the server builds next to the primary, sharing
/// the primary's registry (`ElasticsearchBackend::with_shared_registry`) and
/// configured from the same `HFS_ELASTICSEARCH_*` knobs.
#[cfg(feature = "elasticsearch")]
fn setup_es(args: &Args, url: &str, primary: &Primary) -> EsSide {
    use helios_persistence::backends::elasticsearch::{
        ElasticsearchAuth, ElasticsearchConfig, WriteRefreshPolicy,
    };

    let auth = match (
        env_str("HFS_ELASTICSEARCH_USERNAME"),
        env_str("HFS_ELASTICSEARCH_PASSWORD"),
    ) {
        (Some(username), Some(password)) => Some(ElasticsearchAuth::Basic { username, password }),
        _ => None,
    };
    let config = ElasticsearchConfig {
        nodes: vec![url.to_string()],
        index_prefix: args.es_prefix.clone(),
        auth,
        refresh_interval: args.es_refresh_interval.clone(),
        write_refresh: args
            .es_write_refresh
            .parse::<WriteRefreshPolicy>()
            .unwrap_or_else(|e| panic!("--es-write-refresh: {e}")),
        nested_objects_limit: env_parse("HFS_ELASTICSEARCH_NESTED_OBJECTS_LIMIT").unwrap_or(50_000),
        max_terms_count: env_parse("HFS_ELASTICSEARCH_MAX_TERMS_COUNT").unwrap_or(65_536),
        request_timeout_ms: env_parse("HFS_ELASTICSEARCH_REQUEST_TIMEOUT_MS").unwrap_or(30_000),
        bulk_max_bytes: args.es_bulk_max_bytes,
        bulk_concurrency: args.es_bulk_concurrency,
        reindex_refresh: args.es_reindex_refresh.as_deref().map(|v| {
            v.parse::<WriteRefreshPolicy>()
                .unwrap_or_else(|e| panic!("--es-reindex-refresh: {e}"))
        }),
        reindex_prepare_threads: args.es_prepare_threads,
        ..Default::default()
    };
    let backend = Arc::new(
        ElasticsearchBackend::with_shared_registry(config, primary.registries())
            .expect("create Elasticsearch backend"),
    );
    let transport = elasticsearch::http::transport::Transport::single_node(url)
        .expect("Elasticsearch transport");
    EsSide {
        backend,
        client: elasticsearch::Elasticsearch::new(transport),
        prefix: args.es_prefix.clone(),
    }
}

/// `HFS_COMPOSITE_SYNC_MODE`, parsed as `composite_sync_mode_from_env` does.
#[cfg(feature = "elasticsearch")]
fn sync_mode_from_env() -> helios_persistence::composite::SyncMode {
    use helios_persistence::composite::SyncMode;
    match env_str("HFS_COMPOSITE_SYNC_MODE")
        .map(|v| v.to_ascii_lowercase())
        .as_deref()
    {
        Some("synchronous") | Some("sync") => SyncMode::Synchronous,
        Some("hybrid") => SyncMode::Hybrid {
            sync_for_search: true,
        },
        _ => SyncMode::Asynchronous,
    }
}

/// The `CompositeStorage` the server serves requests from. Nothing on the
/// measured path goes through it — `CompositeSubmitJobs::process_ndjson_stream`
/// forwards to the primary, and the sink and the rebuild take the primary and
/// the Elasticsearch backend directly — but it is built so the process holds
/// the same stack (sync workers included) the server does.
#[cfg(feature = "elasticsearch")]
fn build_composite(
    primary: &Primary,
    es: &Arc<ElasticsearchBackend>,
) -> Arc<helios_persistence::composite::CompositeStorage> {
    use helios_persistence::composite::{
        CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage,
    };
    use helios_persistence::core::BackendKind;

    let (id, kind) = match primary {
        Primary::Sqlite(_) => ("sqlite", BackendKind::Sqlite),
        #[cfg(feature = "postgres")]
        Primary::Postgres(_) => ("postgres", BackendKind::Postgres),
        #[cfg(feature = "mongodb")]
        Primary::Mongodb(_) => ("mongodb", BackendKind::MongoDB),
        #[cfg(feature = "s3")]
        Primary::S3(_) => ("s3", BackendKind::S3),
    };
    let config = CompositeConfig::builder()
        .primary(id, kind)
        .search_backend("es", BackendKind::Elasticsearch)
        .sync_mode(sync_mode_from_env())
        .build()
        .expect("composite config");
    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert(id.to_string(), primary.storage());
    backends.insert("es".to_string(), es.clone());
    let mut providers: HashMap<String, DynSearchProvider> = HashMap::new();
    providers.insert("es".to_string(), es.clone());
    let composite = CompositeStorage::new(config, backends).expect("composite storage");
    let composite = match primary {
        Primary::Sqlite(b) => {
            providers.insert(id.to_string(), b.clone());
            composite
                .with_search_providers(providers)
                .with_full_primary(b.clone())
        }
        #[cfg(feature = "postgres")]
        Primary::Postgres(b) => {
            providers.insert(id.to_string(), b.clone());
            composite
                .with_search_providers(providers)
                .with_full_primary(b.clone())
        }
        #[cfg(feature = "mongodb")]
        Primary::Mongodb(b) => {
            providers.insert(id.to_string(), b.clone());
            composite
                .with_search_providers(providers)
                .with_full_primary(b.clone())
        }
        #[cfg(feature = "s3")]
        Primary::S3(b) => {
            providers.insert(id.to_string(), b.clone());
            composite
                .with_search_providers(providers)
                .with_full_primary(b.clone())
        }
    };
    Arc::new(composite.start_sync_workers())
}

/// Row counts, so the write volume the phases describe is visible. Returns
/// the primary's `search_index` row count, or `None` where it keeps none.
async fn report_counts(primary: &Primary, teardown: &Teardown, args: &Args) -> Option<i64> {
    // Only the S3 arm reads it (for the SDK client); keep the parameter in
    // use in builds without that backend.
    let _ = teardown;
    match primary {
        Primary::Sqlite(_) => {
            let conn = rusqlite::Connection::open(&args.db).expect("open db for counts");
            let mut search_index = None;
            for table in [
                "resources",
                "resource_history",
                "search_index",
                "resource_fts",
                "bulk_entry_results",
                "bulk_submission_changes",
            ] {
                let count: i64 = conn
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                    .unwrap_or(-1);
                if table == "search_index" {
                    search_index = Some(count);
                }
                println!("{table:<26} {count:>12}");
            }
            let db_bytes = std::fs::metadata(&args.db).map(|m| m.len()).unwrap_or(0);
            println!("{:<26} {:>12}", "db bytes", db_bytes);
            search_index
        }
        #[cfg(feature = "postgres")]
        Primary::Postgres(backend) => {
            let client = backend.get_client().await.expect("pool client for counts");
            let mut search_index = None;
            for table in [
                "resources",
                "resource_history",
                "search_index",
                "resource_fts",
                "bulk_entry_results",
                "bulk_submission_changes",
            ] {
                let count: i64 = client
                    .query_one(&format!("SELECT COUNT(*) FROM {table}"), &[])
                    .await
                    .map(|row| row.get(0))
                    .unwrap_or(-1);
                if table == "search_index" {
                    search_index = Some(count);
                }
                println!("{table:<26} {count:>12}");
            }
            let db_bytes: i64 = client
                .query_one("SELECT pg_database_size(current_database())", &[])
                .await
                .map(|row| row.get(0))
                .unwrap_or(-1);
            println!("{:<26} {:>12}", "db bytes", db_bytes);
            search_index
        }
        #[cfg(feature = "mongodb")]
        Primary::Mongodb(backend) => {
            let db = backend.get_database().await.expect("connect for counts");
            let mut search_index = None;
            for collection in [
                "resources",
                "resource_history",
                "search_index",
                "search_index_contained",
                "bulk_entry_results",
                "bulk_submission_changes",
            ] {
                let count = db
                    .collection::<mongodb::bson::Document>(collection)
                    .estimated_document_count()
                    .await
                    .unwrap_or(0) as i64;
                if collection == "search_index" {
                    search_index = Some(count);
                }
                println!("{collection:<26} {count:>12}");
            }
            if let Ok(stats) = db.run_command(mongodb::bson::doc! { "dbStats": 1 }).await {
                let number = |key: &str| -> i64 {
                    stats
                        .get(key)
                        .and_then(|v| {
                            v.as_f64()
                                .map(|f| f as i64)
                                .or_else(|| v.as_i64())
                                .or_else(|| v.as_i32().map(i64::from))
                        })
                        .unwrap_or(-1)
                };
                println!("{:<26} {:>12}", "data bytes", number("dataSize"));
                println!("{:<26} {:>12}", "storage bytes", number("storageSize"));
                println!("{:<26} {:>12}", "index bytes", number("indexSize"));
            }
            search_index
        }
        #[cfg(feature = "s3")]
        Primary::S3(_) => {
            let Teardown::S3 {
                client,
                bucket,
                prefix,
            } = teardown
            else {
                unreachable!("the S3 primary carries its SDK client")
            };
            let (keys, bytes) = s3_list(client, bucket, prefix).await;
            println!("{:<26} {:>12}", "s3 objects", keys.len());
            println!("{:<26} {:>12}", "s3 bytes", bytes);
            println!("(S3 keeps no search index of its own)");
            None
        }
    }
}

/// Every key under `prefix`, with their total size.
#[cfg(feature = "s3")]
async fn s3_list(client: &aws_sdk_s3::Client, bucket: &str, prefix: &str) -> (Vec<String>, i64) {
    let mut keys = Vec::new();
    let mut bytes = 0i64;
    let mut token: Option<String> = None;
    loop {
        let mut request = client.list_objects_v2().bucket(bucket).prefix(prefix);
        if let Some(token) = &token {
            request = request.continuation_token(token);
        }
        let page = match request.send().await {
            Ok(page) => page,
            Err(e) => {
                eprintln!("warning: listing s3://{bucket}/{prefix}: {e}");
                break;
            }
        };
        for object in page.contents() {
            if let Some(key) = object.key() {
                keys.push(key.to_string());
                bytes += object.size().unwrap_or(0);
            }
        }
        match page.next_continuation_token() {
            Some(next) if page.is_truncated() == Some(true) => token = Some(next.to_string()),
            _ => break,
        }
    }
    (keys, bytes)
}

#[cfg(feature = "s3")]
async fn s3_delete(client: &aws_sdk_s3::Client, bucket: &str, keys: &[String]) -> usize {
    use aws_sdk_s3::types::{Delete, ObjectIdentifier};
    let mut deleted = 0usize;
    for chunk in keys.chunks(1000) {
        let objects: Vec<ObjectIdentifier> = chunk
            .iter()
            .map(|key| {
                ObjectIdentifier::builder()
                    .key(key)
                    .build()
                    .expect("object identifier")
            })
            .collect();
        let delete = Delete::builder()
            .set_objects(Some(objects))
            .quiet(true)
            .build()
            .expect("delete request");
        match client
            .delete_objects()
            .bucket(bucket)
            .delete(delete)
            .send()
            .await
        {
            Ok(_) => deleted += chunk.len(),
            Err(e) => eprintln!("warning: deleting {} objects: {e}", chunk.len()),
        }
    }
    deleted
}

fn main() {
    let args = parse_args();

    #[cfg(feature = "s3")]
    if args.backend == Backend::S3 {
        // The S3 backend takes credentials from the SDK's provider chain
        // (`S3Backend::from_env_async`), so they go into the environment —
        // here, before the runtime exists and any other thread could read it.
        for (name, value) in [
            ("AWS_ACCESS_KEY_ID", args.s3_access_key.as_str()),
            ("AWS_SECRET_ACCESS_KEY", args.s3_secret_key.as_str()),
            ("AWS_REGION", args.s3_region.as_str()),
            ("AWS_EC2_METADATA_DISABLED", "true"),
        ] {
            if std::env::var_os(name).is_none() {
                // SAFETY: single-threaded at this point; the runtime (and
                // every thread that could observe the environment) is built
                // below.
                unsafe { std::env::set_var(name, value) };
            }
        }
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(run(args));
}

async fn run(args: Args) {
    #[cfg(not(feature = "elasticsearch"))]
    assert!(
        args.es.is_none(),
        "--es needs a build with --features elasticsearch"
    );

    // The worker ingests one file at a time on SQLite whatever is configured
    // (`effective_file_concurrency`); every other backend takes the setting.
    let file_concurrency = if args.backend == Backend::Sqlite && args.file_concurrency > 1 {
        println!(
            "(--file-concurrency {} ignored on sqlite: the worker forces 1)",
            args.file_concurrency
        );
        1
    } else {
        args.file_concurrency
    };
    let offloaded = args.es.is_some();
    // The server's `use_independent_submit_files`: standalone PostgreSQL only.
    let independent_tasks = args.backend == Backend::Postgres && !offloaded && file_concurrency > 1;
    let (primary, teardown) = setup_primary(&args, offloaded, file_concurrency).await;

    #[cfg(feature = "elasticsearch")]
    let es = args.es.as_deref().map(|url| setup_es(&args, url, &primary));
    #[cfg(feature = "elasticsearch")]
    let _composite = es.as_ref().map(|es| build_composite(&primary, &es.backend));

    let jobs = primary.jobs();
    let tenant = TenantContext::new(TenantId::new("bench"), TenantPermissions::full_access());
    if args.seed {
        let started = Instant::now();
        let outcome = seed_conformance(&primary, &args.data_dir, tenant.tenant_id().as_str()).await;
        println!(
            "seeded conformance: created {} existing {} failed {} in {:.2}s (not timed)",
            outcome.created,
            outcome.existing,
            outcome.failed,
            started.elapsed().as_secs_f64()
        );
    }
    let submission = SubmissionId::generate("bench-system");
    jobs.create_submission(&tenant, &submission, None)
        .await
        .expect("create submission");
    let manifest = jobs
        .add_manifest(&tenant, &submission, None, None)
        .await
        .expect("add manifest");

    // Load every input up front: the timed section then measures ingest only.
    let inputs: Vec<Input> = args
        .files
        .iter()
        .map(|path| {
            let (bytes, lines) = read_prefix(path, args.limit);
            println!(
                "loaded {:>7} lines ({:>6.1} MB) from {}",
                lines,
                bytes.len() as f64 / 1e6,
                path.display()
            );
            let resource_type = resource_type_of(path);
            let file_url = file_url_of(args.backend, path, &resource_type);
            Input {
                resource_type,
                file_url,
                bytes,
                lines,
            }
        })
        .collect();

    let total_lines: usize = inputs.iter().map(|i| i.lines).sum();
    let total_bytes: usize = inputs.iter().map(|i| i.bytes.len()).sum();

    let mut options = BulkProcessingOptions::new();
    options.defer_indexing = args.defer_index;
    if let Some(batch) = args.batch {
        options.batch_size = batch;
    }

    // Index-during-ingest (`DEFER_INDEXING=false` on an ES composite): the
    // sink `IndexingSubmitJobs` attaches to every ingest, attached the same
    // way — as a batch-commit observer on the options.
    #[cfg(feature = "elasticsearch")]
    let sink = match &es {
        Some(es) if !args.defer_index => {
            let config = IngestIndexSinkConfig {
                queue: args.sink_queue,
                concurrency: args.sink_concurrency,
                coalesce: args.sink_coalesce,
                max_wait: Duration::from_secs(args.sink_max_wait_secs),
                page_bytes: args.sink_page_bytes,
            };
            let sink = Arc::new(IngestIndexSink::new(
                primary.storage(),
                vec![es.backend.clone() as Arc<dyn ReindexTarget>],
                config,
            ));
            sink.start_run(&tenant, &submission, &manifest.manifest_id);
            options = options.with_batch_observer(sink.clone() as Arc<dyn BatchCommitObserver>);
            Some(sink)
        }
        _ => None,
    };

    perf::set_enabled(!args.no_phases);
    perf::reset();
    let started = Instant::now();

    let per_file = inputs.iter().map(|input| {
        let jobs = jobs.clone();
        let tenant = tenant.clone();
        let submission = submission.clone();
        let manifest_id = manifest.manifest_id.clone();
        let options = options.clone().with_file_url(&input.file_url);
        let resource_type = input.resource_type.clone();
        let lines = input.lines;
        let bytes = input.bytes.clone();
        async move {
            let reader: Box<dyn tokio::io::AsyncBufRead + Send + Unpin> =
                Box::new(tokio::io::BufReader::new(std::io::Cursor::new(bytes)));
            let file_started = Instant::now();
            let result = jobs
                .process_ndjson_stream(
                    &tenant,
                    &submission,
                    &manifest_id,
                    &resource_type,
                    reader,
                    &options,
                )
                .await;
            (resource_type, lines, file_started.elapsed(), result)
        }
    });
    // Standalone PostgreSQL at a file concurrency above one runs every file
    // in its own Tokio task (`with_independent_file_tasks`, #1467), so the
    // files' parse and write work spreads over the runtime's threads; every
    // other stack polls the files from the worker's one task
    // (`buffer_unordered`), where only their waits overlap.
    let results: Vec<_> = if independent_tasks {
        futures::stream::iter(per_file.map(tokio::spawn))
            .buffer_unordered(file_concurrency)
            .map(|joined| joined.expect("a file task panicked"))
            .collect()
            .await
    } else {
        futures::stream::iter(per_file)
            .buffer_unordered(file_concurrency)
            .collect()
            .await
    };

    let wall = started.elapsed();

    for (resource_type, lines, file_wall, result) in &results {
        if args.backend != Backend::Sqlite {
            println!(
                "  {resource_type:<24} {:>7} in {:>7.2}s = {:>8.0} res/s",
                lines,
                file_wall.as_secs_f64(),
                *lines as f64 / file_wall.as_secs_f64()
            );
        }
        let result = result
            .as_ref()
            .unwrap_or_else(|e| panic!("{resource_type}: ingest: {e}"));
        assert!(
            !result.aborted,
            "{resource_type}: ingest aborted: {:?}",
            result.abort_reason
        );
        assert_eq!(
            result.counts.error_count(),
            0,
            "{resource_type}: {} entries failed",
            result.counts.error_count()
        );
    }

    println!();
    println!(
        "ingested {} resources ({:.1} MB) in {:.2}s = {:.0} resources/s, {:.1} MB/s",
        total_lines,
        total_bytes as f64 / 1e6,
        wall.as_secs_f64(),
        total_lines as f64 / wall.as_secs_f64(),
        total_bytes as f64 / 1e6 / wall.as_secs_f64()
    );
    println!(
        "batch_size={} defer_indexing={} foreign_keys={} seeded={}",
        options.batch_size, options.defer_indexing, !args.no_fk, args.seed
    );
    if args.backend != Backend::Sqlite || args.es.is_some() || file_concurrency > 1 {
        let mut line = format!(
            "backend={} file_concurrency={file_concurrency} scheduling={}",
            primary.name(),
            if independent_tasks {
                "independent-tasks"
            } else {
                "inline"
            }
        );
        if args.reindex {
            line.push_str(&format!(
                " reindex_streams={}",
                if args.reindex_streams == 0 {
                    format!(
                        "auto({})",
                        helios_persistence::search::auto_reindex_streams()
                    )
                } else {
                    args.reindex_streams.to_string()
                }
            ));
            #[cfg(feature = "postgres")]
            if let Primary::Postgres(pg) = &primary
                && !offloaded
            {
                line.push_str(&format!(" pg_reindex_width={}", pg.reindex_width()));
            }
        }
        if let Some(url) = &args.es {
            line.push_str(&format!(
                " es={url} index_prefix={} search_offloaded=true",
                args.es_prefix
            ));
            if !args.defer_index {
                line.push_str(&format!(
                    " sink(queue={} concurrency={} coalesce={} page_bytes={} max_wait={}s)",
                    args.sink_queue,
                    args.sink_concurrency,
                    args.sink_coalesce,
                    args.sink_page_bytes,
                    args.sink_max_wait_secs
                ));
            }
            line.push_str(&format!(
                " es_bulk_concurrency={} es_bulk_max_bytes={} es_refresh_interval={}",
                args.es_bulk_concurrency, args.es_bulk_max_bytes, args.es_refresh_interval
            ));
        }
        println!("{line}");
    }
    println!();
    if args.no_phases {
        println!("(phase collection off)");
    } else if !perf::enabled() {
        println!(
            "(no phase table: this binary was built without `--cfg perf_phases`, so the\n\
             counters are compiled out. Rebuild with\n\
             \x20   RUSTFLAGS='--cfg perf_phases' cargo run --release -p helios-persistence \\\n\
             \x20       --example bulk_submit_bench -- ...\n\
             The rate above is unaffected — that is the point of the flag.)"
        );
    } else {
        print!("{}", perf::report(total_lines as u64, wall));
    }

    // Everything the ingest's timed stages add up to, for the end-to-end line.
    let mut end_to_end = wall;

    // Drain the sink the way `IndexingSubmitJobs::sync_ingested` does once the
    // manifest's files are through: wait for the pages already handed over,
    // and learn what could not be indexed.
    #[cfg(feature = "elasticsearch")]
    if let Some(sink) = &sink {
        perf::reset();
        let drain_started = Instant::now();
        let drain = sink
            .drain(&tenant, &submission, &manifest.manifest_id)
            .await;
        let drain_wall = drain_started.elapsed();
        end_to_end += drain_wall;
        println!();
        println!(
            "drained the search-index sink in {:.2}s: {} written, {} rejected{}",
            drain_wall.as_secs_f64(),
            drain.written,
            drain.rejected.len(),
            if drain.timed_out { " (TIMED OUT)" } else { "" }
        );
        println!(
            "ingest + drain: {:.2}s = {:.0} resources/s end to end",
            end_to_end.as_secs_f64(),
            total_lines as f64 / end_to_end.as_secs_f64()
        );
        for rejected in drain.rejected.iter().take(5) {
            println!(
                "  rejected {}/{}: {}",
                rejected.resource_type, rejected.resource_id, rejected.reason
            );
        }
        if !args.no_phases && perf::enabled() {
            println!();
            print!("{}", perf::report(total_lines as u64, drain_wall));
        }
    }

    // Resources in the store at the end (the first manifest plus any second one).
    #[allow(unused_mut)]
    let mut stored = total_lines;
    if args.reindex {
        let mut types: Vec<String> = inputs.iter().map(|i| i.resource_type.clone()).collect();
        types.sort();
        types.dedup();
        // The writers the server's `ReindexOnFinish` reaches: the primary's
        // own index while it indexes locally (`ReindexOperation::new`), and on
        // an ES composite what `composite_ops` is given — Elasticsearch alone
        // for sqlite-es / pg-es / s3-es (search offloaded, #939 C3), both for
        // mongo-es, where the MongoDB writer sees `is_search_offloaded()` and
        // skips itself.
        let op: Option<(ReindexOperation, &'static str)> = {
            #[cfg(feature = "elasticsearch")]
            {
                match &es {
                    Some(es) => {
                        #[allow(unused_mut)]
                        let mut writers: Vec<Arc<dyn ReindexTarget>> = Vec::new();
                        #[allow(unused_mut)]
                        let mut names = "elasticsearch";
                        #[cfg(feature = "mongodb")]
                        if let Primary::Mongodb(mongo) = &primary {
                            writers.push(mongo.clone() as Arc<dyn ReindexTarget>);
                            names = "mongodb (no-op: offloaded) + elasticsearch";
                        }
                        writers.push(es.backend.clone() as Arc<dyn ReindexTarget>);
                        Some((
                            ReindexOperation::with_parts(
                                primary.reindex_source(),
                                writers,
                                primary.registries(),
                            ),
                            names,
                        ))
                    }
                    None => standalone_reindex(&primary),
                }
            }
            #[cfg(not(feature = "elasticsearch"))]
            {
                standalone_reindex(&primary)
            }
        };
        match op {
            None => println!(
                "\n(--reindex skipped: standalone S3 keeps no search index, and the server wires no reindex hook for it)"
            ),
            Some((op, writers)) => {
                // The same page the submit worker's hook (`ReindexOnFinish`)
                // uses, so this stage measures what the server does after a
                // manifest.
                let request = ReindexRequest::for_types(types)
                    .with_batch_size(args.reindex_batch.unwrap_or(DEFERRED_REINDEX_BATCH_SIZE))
                    .with_batch_bytes(args.reindex_bytes)
                    .with_streams(args.reindex_streams)
                    .with_bulk_index_rebuild(args.bulk_index_rebuild);
                let page = request.batch_size;
                perf::reset();
                let reindex_started = Instant::now();
                let job_id = op
                    .start(tenant.clone(), request, None)
                    .await
                    .expect("start reindex");
                let progress = loop {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    let progress = op
                        .get_progress(&job_id)
                        .await
                        .expect("reindex job vanished");
                    if progress.status.is_finished() {
                        break progress;
                    }
                };
                let reindex_wall = reindex_started.elapsed();
                assert!(
                    matches!(progress.status, ReindexStatus::Completed),
                    "reindex ended {:?}: {:?}",
                    progress.status,
                    progress.error_message
                );
                assert!(
                    progress.errors.is_empty(),
                    "reindex reported {} errors, first: {:?}",
                    progress.errors.len(),
                    progress.errors.first()
                );
                end_to_end += reindex_wall;
                println!();
                println!(
                    "reindexed {} resources in {:.2}s = {:.0} resources/s (page {}{})",
                    total_lines,
                    reindex_wall.as_secs_f64(),
                    total_lines as f64 / reindex_wall.as_secs_f64(),
                    page,
                    if args.bulk_index_rebuild {
                        ", bulk index rebuild"
                    } else {
                        ""
                    }
                );
                if args.backend != Backend::Sqlite || args.es.is_some() {
                    println!(
                        "reindex writers: {writers}; page_bytes={}; processed={} entries_created={}",
                        args.reindex_bytes, progress.processed_resources, progress.entries_created
                    );
                }
                println!(
                    "ingest + reindex: {:.2}s = {:.0} resources/s end to end",
                    end_to_end.as_secs_f64(),
                    total_lines as f64 / end_to_end.as_secs_f64()
                );
                println!();
                if !args.no_phases && perf::enabled() {
                    print!("{}", perf::report(total_lines as u64, reindex_wall));
                }

                if args.second_manifest > 0 {
                    stored += second_manifest(
                        &args,
                        &jobs,
                        &op,
                        &tenant,
                        &submission,
                        &inputs[0],
                        &options,
                    )
                    .await;
                }
            }
        }
    }

    // Row counts, so the write volume the phases describe is visible.
    println!();
    let search_index_rows = report_counts(&primary, &teardown, &args).await;
    if let Some(rows) = search_index_rows {
        if args.backend != Backend::Sqlite || args.es.is_some() {
            println!(
                "{:<26} {:>12.2}",
                "search_index per resource",
                rows as f64 / stored.max(1) as f64
            );
        }
    }

    #[cfg(feature = "elasticsearch")]
    if let Some(es) = &es {
        es.refresh().await;
        let mut types: Vec<&str> = inputs.iter().map(|i| i.resource_type.as_str()).collect();
        types.sort();
        types.dedup();
        for resource_type in &types {
            let index = es.backend.index_name("bench", resource_type);
            let count = es.count(&index).await;
            println!("{:<26} {count:>12}", format!("es docs {resource_type}"));
        }
        let total = es.count(&es.pattern()).await;
        println!(
            "{:<26} {total:>12}  ({:>6.2} per resource)",
            "es docs (total)",
            total as f64 / stored.max(1) as f64
        );
        println!("{:<26} {:>12}", "es store bytes", es.store_bytes().await);
    }

    // Teardown, unless the run is kept for inspection.
    if args.keep {
        match &primary {
            Primary::Sqlite(_) => println!("\nkept {}", args.db.display()),
            #[cfg(feature = "postgres")]
            Primary::Postgres(_) => {
                if let Teardown::Postgres { dbname, .. } = &teardown {
                    println!("\nkept PostgreSQL database `{dbname}`");
                }
            }
            #[cfg(feature = "mongodb")]
            Primary::Mongodb(_) => println!("\nkept MongoDB database `{}`", args.mongo_db),
            #[cfg(feature = "s3")]
            Primary::S3(_) => println!("\nkept s3://{}/{}", args.s3_bucket, args.s3_prefix),
        }
        #[cfg(feature = "elasticsearch")]
        if let Some(es) = &es {
            println!("kept Elasticsearch indices {}", es.pattern());
        }
        return;
    }

    #[cfg(feature = "elasticsearch")]
    if let Some(es) = &es {
        let deleted = es.delete_indices().await;
        println!(
            "\ndeleted {deleted} Elasticsearch indices ({})",
            es.pattern()
        );
    }
    match &primary {
        Primary::Sqlite(_) => {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", args.db.display()));
            }
        }
        #[cfg(feature = "postgres")]
        Primary::Postgres(_) => {}
        #[cfg(feature = "mongodb")]
        Primary::Mongodb(backend) => {
            backend
                .get_database()
                .await
                .expect("connect")
                .drop()
                .await
                .expect("drop bench database");
        }
        #[cfg(feature = "s3")]
        Primary::S3(_) => {}
    }
    match teardown {
        Teardown::None => {}
        #[cfg(feature = "postgres")]
        Teardown::Postgres { admin, dbname } => {
            drop(primary);
            if let Err(e) = admin
                .batch_execute(&format!("DROP DATABASE {dbname} WITH (FORCE)"))
                .await
            {
                eprintln!("warning: DROP DATABASE {dbname}: {e}");
            }
        }
        #[cfg(feature = "s3")]
        Teardown::S3 {
            client,
            bucket,
            prefix,
        } => {
            let (keys, _) = s3_list(&client, &bucket, &prefix).await;
            let deleted = s3_delete(&client, &bucket, &keys).await;
            println!("deleted {deleted} objects under s3://{bucket}/{prefix}");
        }
    }
}

/// Runs one rebuild to completion, polling as the server's status endpoint
/// would, and returns its final progress and wall time.
async fn run_reindex(
    op: &ReindexOperation,
    tenant: &TenantContext,
    request: ReindexRequest,
) -> Result<(ReindexProgress, Duration), ReindexError> {
    let started = Instant::now();
    let job_id = op.start(tenant.clone(), request, None).await?;
    loop {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let progress = op
            .get_progress(&job_id)
            .await
            .expect("reindex job vanished");
        if progress.status.is_finished() {
            return Ok((progress, started.elapsed()));
        }
    }
}

/// `--second-manifest N` (#945 L3): ingests the next `N` lines of the first
/// input file as a second manifest of the same submission, deferred, then
/// runs the rebuild the submit worker's hook fires for it, scoped as
/// `--second-manifest-scope` says. Returns the resources it added.
async fn second_manifest(
    args: &Args,
    jobs: &Arc<dyn BulkSubmitJobStore>,
    op: &ReindexOperation,
    tenant: &TenantContext,
    submission: &SubmissionId,
    first: &Input,
    options: &BulkProcessingOptions,
) -> usize {
    let path = &args.files[0];
    let (bytes, lines) = read_slice(path, args.limit.unwrap_or(0), args.second_manifest);
    assert!(
        lines > 0,
        "--second-manifest: {} has no lines after the first {:?}",
        path.display(),
        args.limit
    );
    let manifest = jobs
        .add_manifest(tenant, submission, None, None)
        .await
        .expect("add second manifest");
    let file_url = format!("{}#second-manifest", first.file_url);
    let options = options.clone().with_file_url(&file_url);
    let reader: Box<dyn tokio::io::AsyncBufRead + Send + Unpin> =
        Box::new(tokio::io::BufReader::new(std::io::Cursor::new(bytes)));
    let ingest_started = Instant::now();
    let result = jobs
        .process_ndjson_stream(
            tenant,
            submission,
            &manifest.manifest_id,
            &first.resource_type,
            reader,
            &options,
        )
        .await
        .expect("second manifest: ingest");
    let ingest_wall = ingest_started.elapsed();
    assert!(
        !result.aborted,
        "second manifest aborted: {:?}",
        result.abort_reason
    );
    assert_eq!(
        result.counts.error_count(),
        0,
        "second manifest: entries failed"
    );

    // What the hook is handed, decided as `fire_deferred_reindex` decides.
    perf::reset();
    let decide_started = Instant::now();
    let since = deferred_reindex_since(
        manifest.added_at,
        Duration::from_secs(args.reindex_since_margin_secs),
    );
    let types = vec![first.resource_type.clone()];
    let ledger = match args.second_scope {
        SecondScope::Ids | SecondScope::Auto => {
            let ids_max = if args.second_scope == SecondScope::Ids {
                u64::MAX
            } else {
                args.reindex_ids_max
            };
            deferred_reindex_ledger_ids(&**jobs, tenant, submission, &manifest.manifest_id, ids_max)
                .await
                .expect("read the second manifest's receipts")
                .filter(|resources| !resources.is_empty())
        }
        _ => None,
    };
    let (request, scope) = match (args.second_scope, ledger) {
        (SecondScope::Types, _) => (
            ReindexRequest::for_types(types).with_bulk_index_rebuild(args.bulk_index_rebuild),
            "types (tenant-wide)".to_string(),
        ),
        (_, Some(resources)) => {
            let n = resources.len();
            (
                ReindexRequest::for_resources(resources),
                format!("ids ({n} from receipts)"),
            )
        }
        (SecondScope::Ids, None) => {
            panic!("--second-manifest-scope ids: the receipts name nothing")
        }
        (_, None) => (
            ReindexRequest::for_types(types).with_updated_since(Some(since)),
            format!("since {}", since.to_rfc3339()),
        ),
    };
    let decide_wall = decide_started.elapsed();
    let request = request
        .with_batch_size(args.reindex_batch.unwrap_or(DEFERRED_REINDEX_BATCH_SIZE))
        .with_batch_bytes(args.reindex_bytes);
    let (progress, reindex_wall) = run_reindex(op, tenant, request)
        .await
        .expect("second manifest: reindex");
    assert!(
        matches!(progress.status, ReindexStatus::Completed),
        "second manifest reindex ended {:?}: {:?}",
        progress.status,
        progress.error_message
    );
    assert!(
        progress.errors.is_empty(),
        "second manifest reindex reported {} errors, first: {:?}",
        progress.errors.len(),
        progress.errors.first()
    );
    let rebuild_wall = decide_wall + reindex_wall;
    println!();
    println!(
        "second manifest: ingested {lines} {} in {:.2}s = {:.0} resources/s",
        first.resource_type,
        ingest_wall.as_secs_f64(),
        lines as f64 / ingest_wall.as_secs_f64()
    );
    println!(
        "second manifest: rebuild scope={scope} in {:.3}s (decide {:.3}s + reindex {:.3}s); processed={} entries_created={}",
        rebuild_wall.as_secs_f64(),
        decide_wall.as_secs_f64(),
        reindex_wall.as_secs_f64(),
        progress.processed_resources,
        progress.entries_created
    );
    println!();
    if !args.no_phases && perf::enabled() {
        print!("{}", perf::report(lines as u64, rebuild_wall));
    }
    lines
}

/// The rebuild a self-indexing primary runs: source and single writer are the
/// backend itself (`standalone_ops`). `None` where the server wires no
/// `$reindex` at all.
fn standalone_reindex(primary: &Primary) -> Option<(ReindexOperation, &'static str)> {
    match primary {
        Primary::Sqlite(b) => Some((
            ReindexOperation::new(b.clone(), b.tenant_registries().clone()),
            "sqlite",
        )),
        #[cfg(feature = "postgres")]
        Primary::Postgres(b) => Some((
            ReindexOperation::new(b.clone(), b.tenant_registries().clone()),
            "postgres",
        )),
        #[cfg(feature = "mongodb")]
        Primary::Mongodb(b) => Some((
            ReindexOperation::new(b.clone(), b.tenant_registries().clone()),
            "mongodb",
        )),
        #[cfg(feature = "s3")]
        Primary::S3(_) => None,
    }
}
