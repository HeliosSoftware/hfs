//! Bulk-submit ingest benchmark for the MongoDB backend (#1000).
//!
//! The MongoDB counterpart of [`bulk_submit_bench`], which measures the same
//! call — `StreamingBulkSubmitProvider::process_ndjson_stream`, the one the
//! bulk-submit worker makes per manifest file — against a real `mongod`.
//!
//! It exists because #1000's numbers are *round-trip* numbers, not CPU or disk
//! numbers: the ingest path is what it is because of how many times it talks to
//! the server per resource. That is only visible against a live server, so this
//! bench needs one; there is no in-process MongoDB to fake it with.
//!
//! The bench drops and recreates its database on every run, so the measured
//! rate is a cold-index rate and two runs are comparable.
//!
//! ```text
//! cargo run --release -p helios-persistence --features mongodb \
//!   --example bulk_submit_mongo_bench -- \
//!     --uri mongodb://localhost:27017 --db hfs_bench --limit 20000 \
//!     /path/to/Condition.ndjson /path/to/Patient.ndjson
//! ```
//!
//! Options:
//!
//! * `--uri URI`      connection string (default: `mongodb://localhost:27017`)
//! * `--db NAME`      database to create and drop (default: `hfs_bulk_submit_bench`)
//! * `--limit N`      resources to ingest per input file (default: all)
//! * `--batch N`      entries per ingest batch (default: the server default, 100)
//! * `--defer-index`  set `defer_indexing`, the `HFS_BULK_SUBMIT_DEFER_INDEXING` path
//! * `--data-dir DIR` directory holding `search-parameters-r4.json` (default: `./data`)
//! * `--keep`         leave the database behind for inspection

use std::path::{Path, PathBuf};
use std::time::Instant;

use helios_persistence::backends::mongodb::{MongoBackend, MongoBackendConfig};
use helios_persistence::core::{
    Backend, BulkProcessingOptions, BulkSubmitProvider, StreamingBulkSubmitProvider, SubmissionId,
};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

struct Args {
    uri: String,
    db: String,
    files: Vec<PathBuf>,
    limit: Option<usize>,
    batch: Option<u32>,
    defer_index: bool,
    data_dir: PathBuf,
    keep: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        uri: std::env::var("HFS_MONGO_URI")
            .unwrap_or_else(|_| "mongodb://localhost:27017".to_string()),
        db: "hfs_bulk_submit_bench".to_string(),
        files: Vec::new(),
        limit: None,
        batch: None,
        defer_index: false,
        data_dir: PathBuf::from("data"),
        keep: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--uri" => args.uri = it.next().expect("--uri needs a connection string"),
            "--db" => args.db = it.next().expect("--db needs a database name"),
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
        "usage: bulk_submit_mongo_bench [options] FILE.ndjson [FILE.ndjson ...]"
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
        if let Some(limit) = limit
            && lines >= limit
        {
            break;
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if line.trim().is_empty() {
                    continue;
                }
                buf.extend_from_slice(line.as_bytes());
                lines += 1;
            }
            Err(e) => panic!("read {}: {e}", path.display()),
        }
    }
    (buf, lines)
}

/// A receipt is keyed by `(manifest, file_url, line_number)` (#457), so every
/// input needs its own `file_url` — two files of the same type would otherwise
/// have their receipts overwrite each other line for line.
fn file_url_of(path: &Path) -> String {
    format!(
        "bench://{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("input")
    )
}

/// `.../Observation.ndjson` and `.../Observation.1787920785840.ndjson` both
/// name `Observation`: the bulk-export file naming puts the type first.
fn resource_type_of(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.split('.').next())
        .expect("input file name names a resource type")
        .to_string()
}

#[tokio::main]
async fn main() {
    let args = parse_args();

    let config = MongoBackendConfig {
        connection_string: args.uri.clone(),
        database_name: args.db.clone(),
        data_dir: Some(args.data_dir.clone()),
        ..Default::default()
    };
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
    backend.initialize().await.expect("init schema");

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
    let inputs: Vec<(String, String, Vec<u8>, usize)> = args
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
            (resource_type_of(path), file_url_of(path), bytes, lines)
        })
        .collect();

    let total_lines: usize = inputs.iter().map(|(_, _, _, n)| *n).sum();
    let total_bytes: usize = inputs.iter().map(|(_, _, b, _)| b.len()).sum();

    let mut options = BulkProcessingOptions::new();
    options.defer_indexing = args.defer_index;
    if let Some(batch) = args.batch {
        options.batch_size = batch;
    }

    let started = Instant::now();

    for (resource_type, file_url, bytes, lines) in &inputs {
        let file_options = options.clone().with_file_url(file_url);
        let reader: Box<dyn tokio::io::AsyncBufRead + Send + Unpin> = Box::new(
            tokio::io::BufReader::new(std::io::Cursor::new(bytes.clone())),
        );
        let file_started = Instant::now();
        let result = backend
            .process_ndjson_stream(
                &tenant,
                &submission,
                &manifest.manifest_id,
                resource_type,
                reader,
                &file_options,
            )
            .await
            .expect("ingest");
        let file_wall = file_started.elapsed();
        println!(
            "  {resource_type:<24} {:>7} in {:>7.2}s = {:>8.0} res/s",
            lines,
            file_wall.as_secs_f64(),
            *lines as f64 / file_wall.as_secs_f64()
        );
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
        "batch_size={} defer_indexing={}",
        options.batch_size, options.defer_indexing
    );

    // Document counts, so the write volume behind the rate is visible: #1000 is
    // a write-volume report before it is a latency report.
    println!();
    let db = backend.get_database().await.expect("connect");
    for collection in [
        "resources",
        "resource_history",
        "search_index",
        "bulk_entry_results",
        "bulk_submission_changes",
    ] {
        let count = db
            .collection::<mongodb::bson::Document>(collection)
            .estimated_document_count()
            .await
            .unwrap_or(0);
        let per_resource = count as f64 / total_lines.max(1) as f64;
        println!("{collection:<26} {count:>12}  ({per_resource:>6.2} per resource)");
    }

    if !args.keep {
        db.drop().await.expect("drop bench database");
    } else {
        println!("\nkept database `{}`", args.db);
    }
}
