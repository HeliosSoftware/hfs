//! Entry point for the `hts` binary.

use clap::Parser;
use helios_hts::config::{Cli, Command, HtsConfig, ImportArgs, ImportFormat, detect_format};
use helios_hts::import::{BundleImportBackend, ImportResult, ImportStats, LanguageFilter};
use helios_persistence::tenant::TenantContext;
use tracing::info;

#[cfg(feature = "sqlite")]
use helios_hts::backends::SqliteTerminologyBackend;
#[cfg(feature = "sqlite")]
use helios_hts::state::AppState;

#[cfg(feature = "postgres")]
use helios_hts::backends::PostgresTerminologyBackend;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli
        .command
        .unwrap_or_else(|| Command::Run(HtsConfig::parse_from(["hts"])))
    {
        Command::Run(config) => {
            helios_observability::uptime::init();
            helios_observability::telemetry::init("hts", &config.log_level);
            helios_observability::metrics::init("hts");
            info!(
                port = config.port,
                host = %config.host,
                storage_backend = %config.storage_backend,
                database_url = %config.database_url,
                "Starting Helios Terminology Server"
            );
            run_server(config).await
        }
        Command::Import(args) => {
            let log_level = if args.verbose {
                "debug"
            } else {
                &args.log_level
            };
            helios_observability::telemetry::init("hts", log_level);
            let code = run_import(args).await?;
            std::process::exit(code);
        }
    }
}

// ── Server ────────────────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
async fn run_server(config: HtsConfig) -> anyhow::Result<()> {
    if config.storage_backend == "postgres" {
        #[cfg(feature = "postgres")]
        return run_server_postgres(config).await;
        #[cfg(not(feature = "postgres"))]
        anyhow::bail!(
            "postgres storage backend requested but the 'postgres' feature is not enabled. \
             Rebuild with `--features postgres`."
        );
    }

    use helios_persistence::backends::sqlite::SqliteBackend;

    // Defer the FTS prebuild until after bootstrap so the index is built once
    // on the final post-import data rather than on stale data the bootstrap
    // sync is about to change.
    let backend = SqliteTerminologyBackend::new_without_fts_prebuild(&config.database_url)?;
    let hts_pool = backend.pool().clone();

    bootstrap_sync_sqlite(&backend, &config).await?;

    // Rebuild closures invalidated by bootstrap re-import and materialize the
    // FTS index on the final data, matching the post-import steps the CLI
    // `hts import` path performs. Runs before the server accepts requests.
    backend
        .finalize_after_bootstrap()
        .map_err(|e| anyhow::anyhow!("post-bootstrap finalization failed: {e}"))?;

    let resource_store = SqliteBackend::open(&config.database_url)
        .map_err(|e| anyhow::anyhow!("Failed to open resource store: {e}"))?;
    resource_store
        .init_schema()
        .map_err(|e| anyhow::anyhow!("Failed to initialize resource store schema: {e}"))?;

    info!("Resource store (helios-persistence) initialized alongside HTS backend");

    let state = AppState::new(backend)
        .with_resource_store(resource_store)
        .with_hts_pool(hts_pool)
        .with_max_expansion_size(config.max_expansion_size);

    let app = helios_hts::server::create_app(&config, state);

    let addr = config.socket_addr();
    info!(address = %addr, "HTS listening");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(not(feature = "sqlite"))]
async fn run_server(config: HtsConfig) -> anyhow::Result<()> {
    if config.storage_backend == "postgres" {
        #[cfg(feature = "postgres")]
        return run_server_postgres(config).await;
        #[cfg(not(feature = "postgres"))]
        anyhow::bail!(
            "No storage backend feature is enabled. \
             Rebuild with `--features sqlite` or `--features postgres`."
        );
    }
    anyhow::bail!(
        "No storage backend feature is enabled. \
         Rebuild with `--features sqlite` (or another backend feature)."
    )
}

#[cfg(feature = "postgres")]
async fn run_server_postgres(config: HtsConfig) -> anyhow::Result<()> {
    use helios_persistence::backends::postgres::PostgresBackend;
    use std::sync::Arc;

    let backend = PostgresTerminologyBackend::new(&config.database_url).await?;

    bootstrap_sync_postgres(&backend, &config).await?;

    // Rebuild any concept closures invalidated by bootstrap re-import, matching
    // the post-import step the CLI `hts import` path performs. Without this a
    // bootstrapped server can come up with chunked SNOMED/LOINC closure data
    // wiped during import and never rebuilt. Idempotent — only systems missing
    // closure rows are recomputed.
    backend
        .rebuild_missing_closures()
        .await
        .map_err(|e| anyhow::anyhow!("post-bootstrap closure rebuild failed: {e}"))?;

    let resource_store = PostgresBackend::from_connection_string(&config.database_url)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to open PostgreSQL resource store: {e}"))?;
    resource_store.init_schema().await.map_err(|e| {
        anyhow::anyhow!("Failed to initialize PostgreSQL resource store schema: {e}")
    })?;

    info!("PostgreSQL resource store (helios-persistence) initialized");

    let state = helios_hts::state::AppState::new(backend.clone())
        .with_resource_store_pg(resource_store)
        .with_terminology_importer(Arc::new(backend))
        .with_max_expansion_size(config.max_expansion_size);

    let app = helios_hts::server::create_app(&config, state);

    let addr = config.socket_addr();
    info!(address = %addr, "HTS (PostgreSQL) listening");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ── Bootstrap ─────────────────────────────────────────────────────────────────

/// If `bootstrap_dir` is non-empty and points at an existing directory,
/// synchronize its contents into the connected database: every recognized file
/// is hashed (SHA-256) and imported only when its hash is absent from, or
/// differs from, the `bootstrap_imports` ledger. Unchanged files are skipped
/// without re-parsing. Otherwise log and no-op.
///
/// Unlike the previous first-run-only behavior, this runs on every startup, so
/// files added to the directory after the first boot — and updated terminology
/// releases that replace an existing file — are picked up automatically. The
/// underlying import is idempotent (upsert keyed on canonical `(url, version)`),
/// so re-importing a changed file refreshes it in place.
///
/// Triggered by `HTS_BOOTSTRAP_DIR`; the Docker image sets this to
/// `/app/terminology-data` so first `docker run` yields a populated server.
#[cfg(feature = "sqlite")]
async fn bootstrap_sync_sqlite(
    backend: &SqliteTerminologyBackend,
    config: &HtsConfig,
) -> anyhow::Result<()> {
    let Some(dir) = resolve_bootstrap_dir(&config.bootstrap_dir) else {
        return Ok(());
    };
    let tracker = BootstrapTracker::Sqlite(backend);
    bootstrap_sync(&tracker, backend, &dir, config).await
}

#[cfg(feature = "postgres")]
async fn bootstrap_sync_postgres(
    backend: &PostgresTerminologyBackend,
    config: &HtsConfig,
) -> anyhow::Result<()> {
    let Some(dir) = resolve_bootstrap_dir(&config.bootstrap_dir) else {
        return Ok(());
    };
    let tracker = BootstrapTracker::Postgres(backend);
    bootstrap_sync(&tracker, backend, &dir, config).await
}

/// Backend-agnostic bootstrap sync: import the directory, gating each file on
/// its content hash via `tracker`.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
async fn bootstrap_sync(
    tracker: &BootstrapTracker<'_>,
    backend: &dyn BundleImportBackend,
    dir: &std::path::Path,
    config: &HtsConfig,
) -> anyhow::Result<()> {
    let batch_size = config.bootstrap_batch_size;
    let languages = LanguageFilter::parse(&config.import_languages);
    info!(
        bootstrap_dir = %dir.display(),
        batch_size,
        import_languages = %languages.canonical_spec(),
        "bootstrap: syncing terminology directory"
    );
    let ctx = TenantContext::system();
    let (stats, imported, skipped) = import_directory_files(
        backend,
        &ctx,
        dir,
        batch_size,
        false,
        &languages,
        Some(tracker),
    )
    .await
    .map_err(|e| anyhow::anyhow!("bootstrap import failed: {e}"))?;
    info!(
        imported_files = imported,
        skipped_files = skipped,
        code_systems = stats.code_systems,
        value_sets = stats.value_sets,
        concept_maps = stats.concept_maps,
        concepts = stats.concepts,
        "bootstrap sync complete"
    );
    Ok(())
}

/// Parse `HTS_BOOTSTRAP_DIR`: empty string → no bootstrap, non-existent path
/// → warn + no bootstrap, otherwise return the resolved path.
fn resolve_bootstrap_dir(bootstrap_dir: &str) -> Option<std::path::PathBuf> {
    if bootstrap_dir.is_empty() {
        return None;
    }
    let dir = std::path::PathBuf::from(bootstrap_dir);
    if !dir.is_dir() {
        tracing::warn!(
            bootstrap_dir,
            "HTS_BOOTSTRAP_DIR is set but does not point at an existing directory; skipping"
        );
        return None;
    }
    Some(dir)
}

/// Ledger over the `bootstrap_imports` table, used by [`import_directory_files`]
/// to skip files that have already been imported with identical content.
///
/// Backend-specific because the SQLite pool is blocking (run on a
/// `spawn_blocking` worker) while the PostgreSQL pool is async.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
enum BootstrapTracker<'a> {
    #[cfg(feature = "sqlite")]
    Sqlite(&'a SqliteTerminologyBackend),
    #[cfg(feature = "postgres")]
    Postgres(&'a PostgresTerminologyBackend),
}

/// A previously-recorded bootstrap ledger row.
///
/// `mtime_unix` is `None` for rows written by an older HTS that predates the
/// column, forcing a content-hash comparison on first boot after upgrade.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
struct LedgerRecord {
    content_hash: String,
    size_bytes: i64,
    mtime_unix: Option<i64>,
    languages: String,
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
impl BootstrapTracker<'_> {
    /// The full ledger row previously recorded for `path` (file name), if any.
    async fn imported_record(&self, path: &str) -> anyhow::Result<Option<LedgerRecord>> {
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(backend) => {
                use rusqlite::OptionalExtension;
                let pool = backend.pool().clone();
                let key = path.to_string();
                let rec =
                    tokio::task::spawn_blocking(move || -> anyhow::Result<Option<LedgerRecord>> {
                        let conn = pool.get()?;
                        let r = conn
                            .query_row(
                                "SELECT content_hash, size_bytes, mtime_unix, languages \
                                 FROM bootstrap_imports WHERE path = ?1",
                                [&key],
                                |r| {
                                    Ok(LedgerRecord {
                                        content_hash: r.get(0)?,
                                        size_bytes: r.get(1)?,
                                        mtime_unix: r.get(2)?,
                                        languages: r.get(3)?,
                                    })
                                },
                            )
                            .optional()?;
                        Ok(r)
                    })
                    .await??;
                Ok(rec)
            }
            #[cfg(feature = "postgres")]
            Self::Postgres(backend) => {
                let client = backend.pool().get().await?;
                let row = client
                    .query_opt(
                        "SELECT content_hash, size_bytes, mtime_unix, languages \
                         FROM bootstrap_imports WHERE path = $1",
                        &[&path],
                    )
                    .await?;
                Ok(row.map(|r| LedgerRecord {
                    content_hash: r.get(0),
                    size_bytes: r.get(1),
                    mtime_unix: r.get(2),
                    languages: r.get(3),
                }))
            }
        }
    }

    /// Record (insert or update) the content hash, size, mtime and applied
    /// language filter for `path`.
    async fn record(
        &self,
        path: &str,
        hash: &str,
        size: i64,
        mtime_unix: Option<i64>,
        languages: &str,
    ) -> anyhow::Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(backend) => {
                let pool = backend.pool().clone();
                let (key, hash, languages) =
                    (path.to_string(), hash.to_string(), languages.to_string());
                tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                    let conn = pool.get()?;
                    conn.execute(
                        "INSERT INTO bootstrap_imports \
                             (path, content_hash, size_bytes, mtime_unix, languages) \
                         VALUES (?1, ?2, ?3, ?4, ?5) \
                         ON CONFLICT(path) DO UPDATE SET \
                             content_hash = excluded.content_hash, \
                             size_bytes   = excluded.size_bytes, \
                             mtime_unix   = excluded.mtime_unix, \
                             languages    = excluded.languages, \
                             imported_at  = datetime('now')",
                        rusqlite::params![key, hash, size, mtime_unix, languages],
                    )?;
                    Ok(())
                })
                .await??;
                Ok(())
            }
            #[cfg(feature = "postgres")]
            Self::Postgres(backend) => {
                let client = backend.pool().get().await?;
                client
                    .execute(
                        "INSERT INTO bootstrap_imports \
                             (path, content_hash, size_bytes, mtime_unix, languages) \
                         VALUES ($1, $2, $3, $4, $5) \
                         ON CONFLICT (path) DO UPDATE SET \
                             content_hash = EXCLUDED.content_hash, \
                             size_bytes   = EXCLUDED.size_bytes, \
                             mtime_unix   = EXCLUDED.mtime_unix, \
                             languages    = EXCLUDED.languages, \
                             imported_at  = now()",
                        &[&path, &hash, &size, &mtime_unix, &languages],
                    )
                    .await?;
                Ok(())
            }
        }
    }
}

/// Cheaply stat a regular file for its `(size, mtime as unix nanoseconds)`
/// without reading its contents. Returns `None` for anything that is not a
/// regular file (e.g. an RxNorm RRF directory) or when the metadata/mtime is
/// unavailable, in which case callers fall back to the full content-hash
/// comparison.
///
/// Nanosecond precision (not seconds) is used so that a same-size content
/// rewrite is reliably detected on filesystems with sub-second mtime
/// granularity (ext4, tmpfs, APFS, NTFS) — a seconds-resolution mtime could
/// collide for two writes in the same wall-clock second.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn cheap_file_stat(path: &std::path::Path) -> Option<(i64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos() as i64;
    Some((meta.len() as i64, mtime))
}

/// Compute a `(SHA-256 hex, size in bytes)` signature for a bootstrap entry.
///
/// For a regular file, this is the SHA-256 of its contents. For a directory
/// (e.g. an RxNorm RRF folder), hashing every byte on every startup would
/// dominate boot time, so the signature is derived from the sorted
/// `(name, size)` of its immediate children instead — a new release dropped in
/// (which changes file sizes) still re-triggers import, while an unchanged
/// folder is cheaply recognized.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn compute_signature(path: &std::path::Path) -> std::io::Result<(String, i64)> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    let size: u64 = if path.is_dir() {
        let mut entries: Vec<(String, u64)> = std::fs::read_dir(path)?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let len = e.metadata().ok()?.len();
                Some((name, len))
            })
            .collect();
        entries.sort();
        let mut total: u64 = 0;
        for (name, len) in &entries {
            hasher.update(name.as_bytes());
            hasher.update(b":");
            hasher.update(len.to_le_bytes());
            hasher.update(b"\n");
            total += len;
        }
        total
    } else {
        let mut file = std::fs::File::open(path)?;
        std::io::copy(&mut file, &mut hasher)?
    };
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    Ok((hex, size as i64))
}

// ── Import ────────────────────────────────────────────────────────────────────

/// Returns the process exit code:
/// - `0` — success, all resources imported
/// - `1` — fatal error (propagated as `Err` by `?`)
/// - `2` — success with non-fatal errors (some records skipped)
///
/// `args.path` may be a file (imports one format, optionally overridden by
/// `--format`) or a directory (iterates entries, auto-detecting each file's
/// format; `--format` is rejected in this mode).
#[cfg(any(feature = "sqlite", feature = "postgres"))]
async fn run_import(args: ImportArgs) -> anyhow::Result<i32> {
    if !args.path.exists() {
        anyhow::bail!("Path does not exist: '{}'", args.path.display());
    }

    let is_dir = args.path.is_dir();

    // RxNorm's importer takes a directory containing RXNCONSO.RRF and
    // friends, so treat such a directory as a single-format import. Every
    // other directory means "iterate and auto-detect per file."
    let rxnorm_dir =
        is_dir && (args.format == Some(ImportFormat::Rxnorm) || contains_rxnorm_rrf(&args.path));

    if is_dir && !rxnorm_dir && args.format.is_some() {
        anyhow::bail!(
            "--format is not compatible with a directory path; each file's \
             format is auto-detected. Point --format at a single file, or \
             remove --format to iterate the directory."
        );
    }

    if args.dry_run {
        eprintln!("[hts-import] dry-run mode — parsing only, no changes will be written");
    }
    if args.verbose {
        eprintln!("[hts-import] verbose mode enabled — debug output active");
    }

    info!(
        path = %args.path.display(),
        is_dir,
        database_url = %args.database_url,
        storage_backend = %args.storage_backend,
        dry_run = args.dry_run,
        verbose = args.verbose,
        "Starting bulk import"
    );

    let started = std::time::Instant::now();
    let ctx = TenantContext::system();

    let (stats, label) = if args.storage_backend == "postgres" {
        #[cfg(feature = "postgres")]
        {
            let backend = PostgresTerminologyBackend::new(&args.database_url).await?;
            let result = run_import_for_path(&backend, &ctx, &args, rxnorm_dir).await?;

            // Pre-build concept closures now so server startup only needs to
            // observe them present (idempotent migration sees no work). Without
            // this the first ?fhir_vs=isa request would block on a SNOMED
            // closure build, blowing past the 60 s health-check timeout.
            if !args.dry_run {
                info!("Building concept closures (this may take ~30–60 s for SNOMED CT)…");
                backend
                    .rebuild_missing_closures()
                    .await
                    .map_err(|e| anyhow::anyhow!("failed to build concept closures: {e}"))?;
                info!("Concept closures ready");
            }

            result
        }
        #[cfg(not(feature = "postgres"))]
        anyhow::bail!(
            "postgres storage backend requested but the 'postgres' feature is not enabled. \
             Rebuild with `--features postgres`."
        );
    } else {
        #[cfg(feature = "sqlite")]
        {
            // Dry-run uses an in-memory database so the pool opens without
            // requiring the target DB file or its parent directory.
            let database_url = if args.dry_run {
                ":memory:".to_string()
            } else {
                args.database_url.clone()
            };
            let backend = SqliteTerminologyBackend::new(&database_url)?;
            let result = run_import_for_path(&backend, &ctx, &args, rxnorm_dir).await?;

            // Pre-build concept closures now so server startup only needs to
            // rebuild the FTS index (~10–25 s) instead of also running
            // migrate_concept_closure (~40 s for SNOMED). Without this, the
            // combined startup time can exceed the 60-second health-check timeout.
            if !args.dry_run {
                info!("Building concept closures (this may take ~40 s for SNOMED CT)…");
                let pool = backend.pool().clone();
                tokio::task::spawn_blocking(move || {
                    let conn = pool.get().map_err(|e| {
                        rusqlite::Error::SqliteFailure(
                            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                            Some(format!("pool error: {e}")),
                        )
                    })?;
                    helios_hts::backends::sqlite::schema::migrate_concept_closure(&conn)
                })
                .await
                .map_err(|e| anyhow::anyhow!("task join error: {e}"))?
                .map_err(|e| anyhow::anyhow!("failed to build concept closures: {e}"))?;
                info!("Concept closures ready");
            }

            result
        }
        #[cfg(not(feature = "sqlite"))]
        anyhow::bail!(
            "sqlite storage backend requested but the 'sqlite' feature is not enabled. \
             Rebuild with `--features sqlite`."
        );
    };

    let result = ImportResult::new(stats, label.clone(), started.elapsed());
    print_import_summary(&result, args.dry_run, &label);

    if !result.stats.errors.is_empty() {
        return Ok(2);
    }
    Ok(0)
}

/// Returns `true` if `path` is a directory containing an `RXNCONSO.RRF`
/// file (case-insensitive) — the hallmark of an extracted RxNorm RRF
/// distribution. Used to keep the RxNorm importer reachable when the user
/// points `hts import` at a flat RRF directory.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn contains_rxnorm_rrf(path: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };
    entries.filter_map(|e| e.ok()).any(|e| {
        e.file_name()
            .to_str()
            .map(|n| n.eq_ignore_ascii_case("RXNCONSO.RRF"))
            .unwrap_or(false)
    })
}

/// Dispatch either a single-file or directory import against an open backend.
/// Returns `(aggregated-stats, label-for-summary)`.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
async fn run_import_for_path(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    args: &ImportArgs,
    rxnorm_dir: bool,
) -> Result<(ImportStats, String), helios_hts::error::HtsError> {
    use helios_hts::error::HtsError;

    let languages = LanguageFilter::parse(&args.languages);

    if args.path.is_dir() && !rxnorm_dir {
        return import_directory(backend, ctx, args, &languages).await;
    }

    let format = match args.format {
        Some(f) => f,
        None => detect_format(&args.path).ok_or_else(|| {
            HtsError::InvalidRequest(format!(
                "Cannot auto-detect format from '{}'. \
                 Use --format to specify one of: hl7-npm, snomed-rf2, loinc, \
                 icd10-cm, icd9-cm, rxnorm, ucum, nci-thesaurus, mesh, dicom, \
                 hl7-v2-tables, nucc, ndc, fhir-bundle. Note: .zip files may \
                 require --format if auto-detection is ambiguous.",
                args.path.display()
            ))
        })?,
    };

    let stats = dispatch_import(
        format,
        backend,
        ctx,
        &args.path,
        args.batch_size,
        args.dry_run,
        &languages,
    )
    .await?;
    Ok((stats, format.to_string()))
}

/// Iterate a directory, auto-detect each file's format, dispatch import for
/// each, and aggregate stats. Files whose format can't be auto-detected and
/// hidden files (dotfiles) are skipped with a warning.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
async fn import_directory(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    args: &ImportArgs,
    languages: &LanguageFilter,
) -> Result<(ImportStats, String), helios_hts::error::HtsError> {
    let (stats, imported_count, skipped_count) = import_directory_files(
        backend,
        ctx,
        &args.path,
        args.batch_size,
        args.dry_run,
        languages,
        None,
    )
    .await?;
    let label = format!("directory ({imported_count} imported, {skipped_count} skipped)");
    Ok((stats, label))
}

/// Shared directory-iteration loop used by both `hts import <dir>` and the
/// `HTS_BOOTSTRAP_DIR` sync path. Returns (aggregated stats, imported file
/// count, skipped file count).
///
/// When `tracker` is `Some`, each file's SHA-256 content signature is compared
/// against the `bootstrap_imports` ledger: files whose signature is unchanged
/// are skipped (counted as skipped), and successfully imported files have their
/// signature recorded. When `tracker` is `None` (CLI `hts import <dir>`), every
/// recognized file is imported unconditionally.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
async fn import_directory_files(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    dir: &std::path::Path,
    batch_size: usize,
    dry_run: bool,
    languages: &LanguageFilter,
    tracker: Option<&BootstrapTracker<'_>>,
) -> Result<(ImportStats, usize, usize), helios_hts::error::HtsError> {
    use helios_hts::error::HtsError;

    let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| {
            HtsError::InvalidRequest(format!("Cannot read directory '{}': {e}", dir.display()))
        })?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let hidden = p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'));
            !hidden
        })
        .collect();
    entries.sort();

    let mut stats = ImportStats::default();
    let mut imported_count = 0usize;
    let mut skipped_count = 0usize;
    let total = entries.len();

    for (idx, entry) in entries.iter().enumerate() {
        let fname = entry
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<non-utf8>");
        let is_dir = entry.is_dir();
        let suffix = if is_dir { "/" } else { "" };

        // Resolve the import format for this entry, skipping entries we don't
        // recognize (non-rxnorm directories, undetectable file formats).
        let format = if is_dir {
            if contains_rxnorm_rrf(entry) {
                ImportFormat::Rxnorm
            } else {
                eprintln!(
                    "[skip] {}/{total} {fname}/: nested directory ignored",
                    idx + 1
                );
                skipped_count += 1;
                continue;
            }
        } else {
            match detect_format(entry) {
                Some(f) => f,
                None => {
                    eprintln!(
                        "[skip] {}/{total} {fname}: format not auto-detected",
                        idx + 1
                    );
                    skipped_count += 1;
                    continue;
                }
            }
        };

        // Bootstrap-sync gating: skip files already imported with identical
        // content; otherwise remember the signature to record after a
        // successful import. A hashing or ledger-lookup error is non-fatal —
        // we log it and fall through to import the file unconditionally.
        //
        // Skip decision, cheapest-first:
        //   1. stat the file (no read) — if size + mtime + applied language
        //      filter all match the ledger, the file is unchanged → skip without
        //      ever reading it. This is what keeps restarts fast for multi-GB
        //      SNOMED/LOINC archives.
        //   2. only when the cheap stat disagrees (or the ledger predates the
        //      mtime column) fall back to a full content hash, so a file whose
        //      mtime was merely touched is still recognized as unchanged.
        let mut pending_signature: Option<(String, i64, Option<i64>, String)> = None;
        if let Some(tracker) = tracker {
            // The language filter applied to this file, stored separately from
            // the content hash so the cheap fast-path can detect a changed
            // HTS_IMPORT_LANGUAGES without re-hashing. Empty = no filtering
            // (non-language-sensitive format, or the allow-all default).
            let lang_sensitive = matches!(format, ImportFormat::SnomedRf2 | ImportFormat::Loinc);
            let applied_langs = if lang_sensitive && !languages.allows_all() {
                languages.canonical_spec()
            } else {
                String::new()
            };
            // Directories (RxNorm RRF) return None and fall through to the
            // metadata-based compute_signature, which is already cheap for them.
            let stat = cheap_file_stat(entry);

            match tracker.imported_record(fname).await {
                Ok(Some(rec)) => {
                    let fast_match = matches!((stat, rec.mtime_unix), (Some((sz, mt)), Some(rmt))
                        if sz == rec.size_bytes && mt == rmt)
                        && rec.languages == applied_langs;
                    if fast_match {
                        eprintln!(
                            "[skip] {}/{total} {fname}{suffix}: unchanged since last bootstrap (size+mtime)",
                            idx + 1
                        );
                        skipped_count += 1;
                        continue;
                    }
                    // Stat disagreed (or legacy row without mtime): hash to see
                    // whether the contents actually changed.
                    match compute_signature(entry) {
                        Ok((hash, size)) => {
                            let mtime = stat.map(|(_, m)| m);
                            if hash == rec.content_hash && rec.languages == applied_langs {
                                // Content identical — only the mtime was touched.
                                // Refresh the ledger so the next boot fast-paths,
                                // then skip.
                                if !dry_run
                                    && let Err(e) = tracker
                                        .record(fname, &hash, size, mtime, &applied_langs)
                                        .await
                                {
                                    tracing::warn!(
                                        file = fname,
                                        error = %e,
                                        "failed to refresh bootstrap ledger mtime"
                                    );
                                }
                                eprintln!(
                                    "[skip] {}/{total} {fname}{suffix}: unchanged since last bootstrap (content hash)",
                                    idx + 1
                                );
                                skipped_count += 1;
                                continue;
                            }
                            pending_signature = Some((hash, size, mtime, applied_langs));
                        }
                        Err(e) => tracing::warn!(
                            file = fname,
                            error = %e,
                            "could not hash bootstrap file; importing unconditionally"
                        ),
                    }
                }
                Ok(None) => match compute_signature(entry) {
                    Ok((hash, size)) => {
                        let mtime = stat.map(|(_, m)| m);
                        pending_signature = Some((hash, size, mtime, applied_langs));
                    }
                    Err(e) => tracing::warn!(
                        file = fname,
                        error = %e,
                        "could not hash bootstrap file; importing unconditionally"
                    ),
                },
                Err(e) => tracing::warn!(
                    file = fname,
                    error = %e,
                    "bootstrap ledger lookup failed; importing unconditionally"
                ),
            }
        }

        eprintln!(
            "[import] file {}/{total} ({format}): {fname}{suffix}",
            idx + 1
        );
        match dispatch_import(format, backend, ctx, entry, batch_size, dry_run, languages).await {
            Ok(file_stats) => {
                stats.merge(file_stats);
                imported_count += 1;
                if let (Some(tracker), Some((hash, size, mtime, langs))) =
                    (tracker, pending_signature.as_ref())
                    && !dry_run
                    && let Err(e) = tracker.record(fname, hash, *size, *mtime, langs).await
                {
                    tracing::warn!(
                        file = fname,
                        error = %e,
                        "failed to record bootstrap import; file will be re-imported next startup"
                    );
                }
            }
            Err(e) => {
                // In directory-iteration mode we deliberately keep going so
                // one broken file doesn't prevent the rest of the bundle
                // from loading. The failure is still reported via
                // ImportStats.errors, which drives exit code 2.
                let msg = format!("[{format}] '{fname}{suffix}' failed: {e}");
                eprintln!("[error] {msg}");
                stats.errors.push(msg);
                skipped_count += 1;
            }
        }
    }

    Ok((stats, imported_count, skipped_count))
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
async fn dispatch_import(
    format: ImportFormat,
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    path: &std::path::Path,
    batch_size: usize,
    dry_run: bool,
    languages: &LanguageFilter,
) -> Result<ImportStats, helios_hts::error::HtsError> {
    use helios_hts::import::{
        dicom::import_dicom, hl7_v2_tables::import_hl7_v2_tables, icd9_cm::import_icd9_cm,
        icd10_cm::import_icd10_cm, loinc_csv::import_loinc_csv, mesh::import_mesh,
        nci_thesaurus::import_nci_thesaurus, ndc::import_ndc, nucc::import_nucc,
        rxnorm_rrf::import_rxnorm_rrf, snomed_rf2::import_snomed_rf2, tgz::import_tgz,
        ucum::import_ucum,
    };

    match format {
        ImportFormat::Hl7Npm => import_tgz(backend, ctx, path, batch_size, dry_run).await,
        ImportFormat::SnomedRf2 => {
            import_snomed_rf2(backend, ctx, path, batch_size, dry_run, languages).await
        }
        ImportFormat::Loinc => {
            import_loinc_csv(backend, ctx, path, batch_size, dry_run, languages).await
        }
        ImportFormat::Icd10Cm => import_icd10_cm(backend, ctx, path, batch_size, dry_run).await,
        ImportFormat::Icd9Cm => import_icd9_cm(backend, ctx, path, batch_size, dry_run).await,
        ImportFormat::Rxnorm => import_rxnorm_rrf(backend, ctx, path, batch_size, dry_run).await,
        ImportFormat::Ucum => import_ucum(backend, ctx, path, batch_size, dry_run).await,
        ImportFormat::NciThesaurus => {
            import_nci_thesaurus(backend, ctx, path, batch_size, dry_run).await
        }
        ImportFormat::Mesh => import_mesh(backend, ctx, path, batch_size, dry_run).await,
        ImportFormat::Dicom => import_dicom(backend, ctx, path, batch_size, dry_run).await,
        ImportFormat::Hl7V2Tables => {
            import_hl7_v2_tables(backend, ctx, path, batch_size, dry_run).await
        }
        ImportFormat::Nucc => import_nucc(backend, ctx, path, batch_size, dry_run).await,
        ImportFormat::Ndc => import_ndc(backend, ctx, path, batch_size, dry_run).await,
        ImportFormat::FhirBundle => import_fhir_bundle_file(backend, ctx, path, dry_run).await,
    }
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
async fn import_fhir_bundle_file(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    path: &std::path::Path,
    dry_run: bool,
) -> Result<helios_hts::import::ImportStats, helios_hts::error::HtsError> {
    use helios_hts::error::HtsError;

    let data = tokio::fs::read(path)
        .await
        .map_err(|e| HtsError::InvalidRequest(format!("Cannot read '{}': {e}", path.display())))?;

    if dry_run {
        let parsed = helios_hts::import::bundle_parser::parse_bundle(&data)?;
        let concepts: usize = parsed.code_systems.iter().map(|cs| cs.concepts.len()).sum();
        let stats = helios_hts::import::ImportStats {
            code_systems: parsed.code_systems.len() as u32,
            value_sets: parsed.value_sets.len() as u32,
            concept_maps: parsed.concept_maps.len() as u32,
            concepts: concepts as u32,
            ..Default::default()
        };
        return Ok(stats);
    }

    backend.import_bundle(ctx, &data).await
}

#[cfg(not(any(feature = "sqlite", feature = "postgres")))]
async fn run_import(_args: ImportArgs) -> anyhow::Result<i32> {
    anyhow::bail!(
        "No storage backend feature is enabled. \
         Rebuild with `--features sqlite` or `--features postgres`."
    )
}

// ── Shared helpers ────────────────────────────────────────────────────────────

#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn print_import_summary(result: &helios_hts::import::ImportResult, dry_run: bool, format: &str) {
    let dry_label = if dry_run { " (dry-run)" } else { "" };
    println!(
        "Import complete{dry_label} [{format}] in {:.1}s: \
         {} CodeSystems, {} ValueSets, {} ConceptMaps, {} concepts",
        result.duration.as_secs_f64(),
        result.stats.code_systems,
        result.stats.value_sets,
        result.stats.concept_maps,
        result.stats.concepts,
    );
    if !result.stats.errors.is_empty() {
        eprintln!(
            "Non-fatal errors ({}); first few shown below:",
            result.stats.errors.len()
        );
        for e in result.stats.errors.iter().take(10) {
            eprintln!("  - {e}");
        }
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;

    /// Write a minimal FHIR Bundle wrapping a single CodeSystem to `dir/name`.
    fn write_bundle(dir: &std::path::Path, name: &str, cs_url: &str, version: &str) {
        let bundle = serde_json::json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [{
                "resource": {
                    "resourceType": "CodeSystem",
                    "url": cs_url,
                    "version": version,
                    "status": "active",
                    "content": "complete",
                    "concept": [{ "code": "A", "display": "Alpha" }]
                }
            }]
        });
        let path = dir.join(name);
        std::fs::write(&path, serde_json::to_vec(&bundle).unwrap()).unwrap();

        // Stamp a deterministic mtime keyed on (name, version) so the bootstrap
        // size+mtime fast-path reliably sees a content change as a change,
        // independent of the host filesystem's mtime granularity (some
        // platforms, e.g. the CI/WSL temp dirs, resolve mtime to whole
        // seconds, which would otherwise collide for two same-size rewrites in
        // the same wall-clock second). A real terminology release replaces the
        // file and so always lands a fresh mtime — this mirrors that.
        let key: u64 = name.bytes().chain(version.bytes()).map(u64::from).sum();
        let mtime = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000 + key);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
    }

    #[test]
    fn compute_signature_is_deterministic_and_content_sensitive() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.json");

        std::fs::write(&file, b"hello").unwrap();
        let (h1, s1) = compute_signature(&file).unwrap();
        let (h2, s2) = compute_signature(&file).unwrap();
        assert_eq!(h1, h2, "same content hashes identically");
        assert_eq!(s1, 5);
        assert_eq!(s2, 5);

        std::fs::write(&file, b"hello world").unwrap();
        let (h3, s3) = compute_signature(&file).unwrap();
        assert_ne!(h1, h3, "changed content yields a different hash");
        assert_eq!(s3, 11);
    }

    /// The bootstrap sync must import a file the first time, skip it while
    /// unchanged, re-import it when its content changes (e.g. a version bump),
    /// and pick up newly added files — without re-importing the unchanged ones.
    #[tokio::test]
    async fn bootstrap_sync_imports_new_and_changed_files_only() {
        let db = tempfile::NamedTempFile::new().unwrap();
        let backend = SqliteTerminologyBackend::new(db.path().to_str().unwrap()).unwrap();
        let ctx = TenantContext::system();
        let tracker = BootstrapTracker::Sqlite(&backend);
        let boot = tempfile::tempdir().unwrap();
        write_bundle(boot.path(), "cs.json", "http://example.org/cs", "1.0.0");

        // First run: imports the file.
        let (stats, imported, skipped) = import_directory_files(
            &backend,
            &ctx,
            boot.path(),
            500,
            false,
            &LanguageFilter::default(),
            Some(&tracker),
        )
        .await
        .unwrap();
        assert_eq!((imported, skipped), (1, 0));
        assert_eq!(stats.code_systems, 1);

        // Second run, file unchanged: skipped, nothing imported.
        let (stats, imported, skipped) = import_directory_files(
            &backend,
            &ctx,
            boot.path(),
            500,
            false,
            &LanguageFilter::default(),
            Some(&tracker),
        )
        .await
        .unwrap();
        assert_eq!((imported, skipped), (0, 1));
        assert_eq!(stats.code_systems, 0);

        // Bump the version: content changes, so the file is re-imported.
        write_bundle(boot.path(), "cs.json", "http://example.org/cs", "2.0.0");
        let (_stats, imported, skipped) = import_directory_files(
            &backend,
            &ctx,
            boot.path(),
            500,
            false,
            &LanguageFilter::default(),
            Some(&tracker),
        )
        .await
        .unwrap();
        assert_eq!((imported, skipped), (1, 0));

        // Add a brand-new file: only it is imported; cs.json stays skipped.
        write_bundle(boot.path(), "cs2.json", "http://example.org/cs2", "1.0.0");
        let (_stats, imported, skipped) = import_directory_files(
            &backend,
            &ctx,
            boot.path(),
            500,
            false,
            &LanguageFilter::default(),
            Some(&tracker),
        )
        .await
        .unwrap();
        assert_eq!((imported, skipped), (1, 1));
    }

    /// Without a tracker (the `hts import <dir>` CLI path) every recognized file
    /// is imported unconditionally on each invocation.
    #[tokio::test]
    async fn import_without_tracker_always_imports() {
        let db = tempfile::NamedTempFile::new().unwrap();
        let backend = SqliteTerminologyBackend::new(db.path().to_str().unwrap()).unwrap();
        let ctx = TenantContext::system();
        let boot = tempfile::tempdir().unwrap();
        write_bundle(boot.path(), "cs.json", "http://example.org/cs", "1.0.0");

        for _ in 0..2 {
            let (_stats, imported, skipped) = import_directory_files(
                &backend,
                &ctx,
                boot.path(),
                500,
                false,
                &LanguageFilter::default(),
                None,
            )
            .await
            .unwrap();
            assert_eq!((imported, skipped), (1, 0));
        }
    }
}
