//! Entry point for the `hts` binary.

use clap::Parser;
use helios_hts::config::{Cli, Command, HtsConfig, ImportArgs, ImportFormat, detect_format};
use helios_hts::import::{BundleImportBackend, ImportResult, ImportStats};
use helios_persistence::tenant::TenantContext;
use tracing::info;

#[cfg(feature = "sqlite")]
use helios_hts::backends::SqliteTerminologyBackend;
#[cfg(feature = "sqlite")]
use helios_hts::state::AppState;

#[cfg(feature = "postgres")]
use helios_hts::backends::PostgresTerminologyBackend;

fn init_logging(log_level: &str) {
    use tracing_subscriber::{EnvFilter, fmt};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    fmt().with_env_filter(filter).with_target(false).init();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli
        .command
        .unwrap_or_else(|| Command::Run(HtsConfig::parse_from(["hts"])))
    {
        Command::Run(config) => {
            init_logging(&config.log_level);
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
            init_logging(log_level);
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

    let backend = SqliteTerminologyBackend::new(&config.database_url)?;
    let hts_pool = backend.pool().clone();

    bootstrap_if_empty_sqlite(&backend, &config.bootstrap_dir).await?;

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

    bootstrap_if_empty_postgres(&backend, &config.bootstrap_dir).await?;

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

/// If `bootstrap_dir` is non-empty and points at an existing directory, and
/// the connected database has no CodeSystems yet, import every recognized
/// file in the directory before returning. Otherwise log and no-op.
///
/// Triggered by `HTS_BOOTSTRAP_DIR`; the Docker image sets this to
/// `/app/terminology-data` so first `docker run` yields a populated server.
#[cfg(feature = "sqlite")]
async fn bootstrap_if_empty_sqlite(
    backend: &SqliteTerminologyBackend,
    bootstrap_dir: &str,
) -> anyhow::Result<()> {
    let Some(dir) = resolve_bootstrap_dir(bootstrap_dir) else {
        return Ok(());
    };

    let pool = backend.pool().clone();
    let count = tokio::task::spawn_blocking(move || -> anyhow::Result<i64> {
        let conn = pool.get()?;
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM code_systems", [], |r| r.get(0))?;
        Ok(n)
    })
    .await??;

    if count > 0 {
        info!(
            bootstrap_dir = %dir.display(),
            code_systems = count,
            "bootstrap skipped: database already populated"
        );
        return Ok(());
    }

    info!(bootstrap_dir = %dir.display(), "bootstrap: database empty, importing");
    let ctx = TenantContext::system();
    let (stats, imported, skipped) = import_directory_files(backend, &ctx, &dir, 500, false)
        .await
        .map_err(|e| anyhow::anyhow!("bootstrap import failed: {e}"))?;
    info!(
        imported_files = imported,
        skipped_files = skipped,
        code_systems = stats.code_systems,
        value_sets = stats.value_sets,
        concept_maps = stats.concept_maps,
        concepts = stats.concepts,
        "bootstrap complete"
    );
    Ok(())
}

#[cfg(feature = "postgres")]
async fn bootstrap_if_empty_postgres(
    backend: &PostgresTerminologyBackend,
    bootstrap_dir: &str,
) -> anyhow::Result<()> {
    let Some(dir) = resolve_bootstrap_dir(bootstrap_dir) else {
        return Ok(());
    };

    let client = backend.pool().get().await?;
    let row = client
        .query_one("SELECT COUNT(*)::bigint FROM code_systems", &[])
        .await?;
    let count: i64 = row.get(0);
    drop(client);

    if count > 0 {
        info!(
            bootstrap_dir = %dir.display(),
            code_systems = count,
            "bootstrap skipped: database already populated"
        );
        return Ok(());
    }

    info!(bootstrap_dir = %dir.display(), "bootstrap: database empty, importing");
    let ctx = TenantContext::system();
    let (stats, imported, skipped) = import_directory_files(backend, &ctx, &dir, 500, false)
        .await
        .map_err(|e| anyhow::anyhow!("bootstrap import failed: {e}"))?;
    info!(
        imported_files = imported,
        skipped_files = skipped,
        code_systems = stats.code_systems,
        value_sets = stats.value_sets,
        concept_maps = stats.concept_maps,
        concepts = stats.concepts,
        "bootstrap complete"
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

    if args.path.is_dir() && !rxnorm_dir {
        return import_directory(backend, ctx, args).await;
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
) -> Result<(ImportStats, String), helios_hts::error::HtsError> {
    let (stats, imported_count, skipped_count) =
        import_directory_files(backend, ctx, &args.path, args.batch_size, args.dry_run).await?;
    let label = format!("directory ({imported_count} imported, {skipped_count} skipped)");
    Ok((stats, label))
}

/// Shared directory-iteration loop used by both `hts import <dir>` and the
/// `HTS_BOOTSTRAP_DIR` first-run path. Returns (aggregated stats, imported
/// file count, skipped file count).
#[cfg(any(feature = "sqlite", feature = "postgres"))]
async fn import_directory_files(
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    dir: &std::path::Path,
    batch_size: usize,
    dry_run: bool,
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
        if entry.is_dir() {
            if contains_rxnorm_rrf(entry) {
                eprintln!("[import] file {}/{total} (rxnorm): {fname}/", idx + 1);
                match dispatch_import(
                    ImportFormat::Rxnorm,
                    backend,
                    ctx,
                    entry,
                    batch_size,
                    dry_run,
                )
                .await
                {
                    Ok(file_stats) => {
                        stats.merge(file_stats);
                        imported_count += 1;
                    }
                    Err(e) => {
                        let msg = format!("[rxnorm] '{fname}/' failed: {e}");
                        eprintln!("[error] {msg}");
                        stats.errors.push(msg);
                        skipped_count += 1;
                    }
                }
            } else {
                eprintln!(
                    "[skip] {}/{total} {fname}/: nested directory ignored",
                    idx + 1
                );
                skipped_count += 1;
            }
            continue;
        }
        let Some(format) = detect_format(entry) else {
            eprintln!(
                "[skip] {}/{total} {fname}: format not auto-detected",
                idx + 1
            );
            skipped_count += 1;
            continue;
        };
        eprintln!("[import] file {}/{total} ({format}): {fname}", idx + 1);
        match dispatch_import(format, backend, ctx, entry, batch_size, dry_run).await {
            Ok(file_stats) => {
                stats.merge(file_stats);
                imported_count += 1;
            }
            Err(e) => {
                // In directory-iteration mode we deliberately keep going so
                // one broken file doesn't prevent the rest of the bundle
                // from loading. The failure is still reported via
                // ImportStats.errors, which drives exit code 2.
                let msg = format!("[{format}] '{fname}' failed: {e}");
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
        ImportFormat::SnomedRf2 => import_snomed_rf2(backend, ctx, path, batch_size, dry_run).await,
        ImportFormat::Loinc => import_loinc_csv(backend, ctx, path, batch_size, dry_run).await,
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
