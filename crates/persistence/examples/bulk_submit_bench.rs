//! Bulk-submit ingest benchmark for the SQLite backend (#947).
//!
//! Drives the real ingest path — `StreamingBulkSubmitProvider::process_ndjson_stream`,
//! the same call the bulk-submit worker makes per manifest file — against local
//! NDJSON files, on a fresh file-backed database, and reports the achieved rate
//! plus the per-phase breakdown collected by [`helios_persistence::perf`].
//!
//! The worker's HTTP fetch, lease keeper, and manifest polling are deliberately
//! out of the loop: this measures the write path #947 is about, with no network
//! or fixture-server variance between runs.
//!
//! The phase table needs `--cfg perf_phases`; without it the ingest still runs
//! and reports its rate, but every phase reads zero (see
//! [`helios_persistence::perf`] for why that is a cfg and not a feature).
//!
//! ```text
//! RUSTFLAGS='--cfg perf_phases' \
//!   cargo run --release -p helios-persistence --example bulk_submit_bench -- \
//!     --db /tmp/bench.db --limit 30000 \
//!     /path/to/CarePlan.ndjson /path/to/Condition.ndjson
//! ```
//!
//! Options:
//!
//! * `--db PATH`      database file to create (deleted first; default: a temp file)
//! * `--limit N`      resources to ingest per input file (default: all)
//! * `--batch N`      entries per transaction (default: the server default, 100)
//! * `--defer-index`  set `defer_indexing`, the `HFS_BULK_SUBMIT_DEFER_INDEXING` path
//! * `--data-dir DIR` directory holding `search-parameters-r4.json` (default: `./data`)
//! * `--keep`         leave the database behind for inspection

use std::path::{Path, PathBuf};
use std::time::Instant;

use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::{
    BulkProcessingOptions, BulkSubmitProvider, StreamingBulkSubmitProvider, SubmissionId,
};
use helios_persistence::perf;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

struct Args {
    db: PathBuf,
    files: Vec<PathBuf>,
    limit: Option<usize>,
    batch: Option<u32>,
    defer_index: bool,
    data_dir: PathBuf,
    keep: bool,
    /// Experiment switch: run with `PRAGMA foreign_keys = OFF`, to price the
    /// parent-row check `search_index`'s foreign key costs on every index row.
    no_fk: bool,
    /// Run with phase collection off — the rate an uninstrumented build
    /// achieves, and the way to price the instrumentation itself.
    no_phases: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        db: PathBuf::from("/tmp/hfs-bulk-submit-bench.db"),
        files: Vec::new(),
        limit: None,
        batch: None,
        defer_index: false,
        data_dir: PathBuf::from("data"),
        keep: false,
        no_fk: false,
        no_phases: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--db" => args.db = PathBuf::from(it.next().expect("--db needs a path")),
            "--limit" => {
                args.limit = Some(
                    it.next()
                        .expect("--limit needs a count")
                        .parse()
                        .expect("--limit must be a number"),
                )
            }
            "--batch" => {
                args.batch = Some(
                    it.next()
                        .expect("--batch needs a count")
                        .parse()
                        .expect("--batch must be a number"),
                )
            }
            "--defer-index" => args.defer_index = true,
            "--no-fk" => args.no_fk = true,
            "--no-phases" => args.no_phases = true,
            "--keep" => args.keep = true,
            "--data-dir" => {
                args.data_dir = PathBuf::from(it.next().expect("--data-dir needs a path"))
            }
            other if other.starts_with("--") => panic!("unknown option {other}"),
            other => args.files.push(PathBuf::from(other)),
        }
    }
    assert!(
        !args.files.is_empty(),
        "usage: bulk_submit_bench [options] FILE.ndjson [FILE.ndjson ...]"
    );
    args
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

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args = parse_args();

    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", args.db.display()));
    }

    let config = SqliteBackendConfig {
        data_dir: Some(args.data_dir.clone()),
        enable_foreign_keys: !args.no_fk,
        ..Default::default()
    };
    let backend = SqliteBackend::with_config(&args.db, config).expect("open backend");
    backend.init_schema().expect("init schema");

    let tenant = TenantContext::new(TenantId::new("bench"), TenantPermissions::full_access());
    let submission = SubmissionId::generate("bench-system");
    backend
        .create_submission(&tenant, &submission, None)
        .await
        .expect("create submission");
    let manifest = backend
        .add_manifest(&tenant, &submission, None, None)
        .await
        .expect("add manifest");

    // Load every input up front: the timed section then measures ingest only.
    let inputs: Vec<(String, Vec<u8>, usize)> = args
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
            (resource_type_of(path), bytes, lines)
        })
        .collect();

    let total_lines: usize = inputs.iter().map(|(_, _, n)| *n).sum();
    let total_bytes: usize = inputs.iter().map(|(_, b, _)| b.len()).sum();

    let mut options = BulkProcessingOptions::new();
    options.defer_indexing = args.defer_index;
    if let Some(batch) = args.batch {
        options.batch_size = batch;
    }

    perf::set_enabled(!args.no_phases);
    perf::reset();
    let started = Instant::now();

    for (resource_type, bytes, _) in &inputs {
        let options = options
            .clone()
            .with_file_url(format!("bench://{resource_type}"));
        let reader: Box<dyn tokio::io::AsyncBufRead + Send + Unpin> = Box::new(
            tokio::io::BufReader::new(std::io::Cursor::new(bytes.clone())),
        );
        let result = backend
            .process_ndjson_stream(
                &tenant,
                &submission,
                &manifest.manifest_id,
                resource_type,
                reader,
                &options,
            )
            .await
            .expect("ingest");
        assert_eq!(
            result.counts.error_count(),
            0,
            "{resource_type}: {} entries failed",
            result.counts.error_count()
        );
    }

    let wall = started.elapsed();

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
        "batch_size={} defer_indexing={} foreign_keys={}",
        options.batch_size, options.defer_indexing, !args.no_fk
    );
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

    // Row counts, so the write volume the phases describe is visible.
    println!();
    let conn = rusqlite::Connection::open(&args.db).expect("open db for counts");
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
        println!("{table:<26} {count:>12}");
    }
    let db_bytes = std::fs::metadata(&args.db).map(|m| m.len()).unwrap_or(0);
    println!("{:<26} {:>12}", "db bytes", db_bytes);
    drop(conn);

    if !args.keep {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", args.db.display()));
        }
    }
}
