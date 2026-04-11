use clap::Parser;
use helios_hts::config::{Cli, Command, HtsConfig, ImportArgs, ImportFormat, detect_format};
use helios_hts::import::ImportResult;
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
                "Starting Helios Terminology Service"
            );
            run_server(config).await
        }
        Command::Import(args) => {
            // --verbose bumps the effective log level to debug so tracing
            // calls inside the importers are visible on stderr.
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

    // Initialize the helios-persistence PostgreSQL resource store (raw FHIR JSON CRUD).
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

// ── Import ────────────────────────────────────────────────────────────────────

/// Returns the process exit code:
/// - `0` — success, all resources imported
/// - `1` — fatal error (propagated as `Err` by `?`)
/// - `2` — success with non-fatal errors (some records skipped)
#[cfg(any(feature = "sqlite", feature = "postgres"))]
async fn run_import(args: ImportArgs) -> anyhow::Result<i32> {
    if args.storage_backend == "postgres" {
        #[cfg(feature = "postgres")]
        return run_import_postgres(args).await;
        #[cfg(not(feature = "postgres"))]
        anyhow::bail!(
            "postgres storage backend requested but the 'postgres' feature is not enabled. \
             Rebuild with `--features postgres`."
        );
    }

    #[cfg(not(feature = "sqlite"))]
    anyhow::bail!(
        "sqlite storage backend requested but the 'sqlite' feature is not enabled. \
         Rebuild with `--features sqlite`."
    );

    #[cfg(feature = "sqlite")]
    run_import_sqlite(args).await
}

#[cfg(feature = "postgres")]
async fn run_import_postgres(args: ImportArgs) -> anyhow::Result<i32> {
    use helios_hts::config::ImportFormat;
    use helios_hts::import::tgz::import_tgz_pg;

    let format = match args.format {
        Some(f) => f,
        None => {
            use helios_hts::config::detect_format;
            detect_format(&args.path).ok_or_else(|| {
                anyhow::anyhow!(
                    "Cannot auto-detect format from '{}'.\n\
                     Use --format to specify one of: hl7-npm | snomed-rf2 | loinc | icd10-cm | rxnorm\n\
                     Note: .zip files require --format because SNOMED and LOINC share the same extension.",
                    args.path.display()
                )
            })?
        }
    };

    if !matches!(format, ImportFormat::Hl7Npm) {
        anyhow::bail!(
            "'{format}' importer does not support the PostgreSQL backend. \
             Use HTS_STORAGE_BACKEND=sqlite."
        );
    }

    if !args.path.exists() {
        anyhow::bail!("Path does not exist: '{}'", args.path.display());
    }

    if args.dry_run {
        eprintln!(
            "[hl7-npm] dry-run mode — parsing only, no changes will be written to the database"
        );
    }

    info!(
        path = %args.path.display(),
        format = %format,
        database_url = %args.database_url,
        dry_run = args.dry_run,
        "Starting PostgreSQL bulk import"
    );

    let backend = PostgresTerminologyBackend::new(&args.database_url).await?;

    let started = std::time::Instant::now();
    let stats = import_tgz_pg(&backend, &args.path, args.batch_size, args.dry_run).await?;
    let result =
        helios_hts::import::ImportResult::new(stats, format.to_string(), started.elapsed());

    print_import_summary(&result, args.dry_run, &format.to_string());

    if !result.stats.errors.is_empty() {
        return Ok(2);
    }
    Ok(0)
}

#[cfg(feature = "sqlite")]
async fn run_import_sqlite(args: ImportArgs) -> anyhow::Result<i32> {
    use helios_hts::import::tgz::import_tgz;

    // ── Validate path ──────────────────────────────────────────────────────
    if !args.path.exists() {
        anyhow::bail!("Path does not exist: '{}'", args.path.display());
    }

    // ── Resolve format ─────────────────────────────────────────────────────
    let format = match args.format {
        Some(f) => f,
        None => detect_format(&args.path).ok_or_else(|| {
            anyhow::anyhow!(
                "Cannot auto-detect format from '{}'.\n\
                 Use --format to specify one of:\n\
                 hl7-npm | snomed-rf2 | loinc | icd10-cm | icd9-cm | rxnorm |\n\
                 ucum | nci-thesaurus | mesh | dicom | hl7-v2-tables | nucc\n\
                 Note: .zip files may require --format if auto-detection is ambiguous.",
                args.path.display()
            )
        })?,
    };

    // For dry-run, use an in-memory database so the pool opens even when the
    // target DB file or its parent directory does not yet exist.
    let database_url = if args.dry_run {
        ":memory:".to_string()
    } else {
        args.database_url.clone()
    };

    if args.dry_run {
        eprintln!(
            "[{format}] dry-run mode — parsing only, no changes will be written to the database"
        );
    }
    if args.verbose {
        eprintln!("[{format}] verbose mode enabled — debug output active");
    }

    info!(
        path = %args.path.display(),
        format = %format,
        database_url = %args.database_url,
        dry_run = args.dry_run,
        verbose = args.verbose,
        "Starting bulk import"
    );

    // ── Route to the correct importer ──────────────────────────────────────
    let started = std::time::Instant::now();

    let stats = match format {
        ImportFormat::Hl7Npm => {
            let backend = SqliteTerminologyBackend::new(&database_url)?;
            let pool = backend.pool().clone();
            let path = args.path.clone();
            let batch_size = args.batch_size;
            let dry_run = args.dry_run;

            tokio::task::spawn_blocking(move || import_tgz(&pool, &path, batch_size, dry_run))
                .await
                .map_err(|e| anyhow::anyhow!("Import task panicked: {e}"))??
        }

        ImportFormat::SnomedRf2 => {
            use helios_hts::import::snomed_rf2::import_snomed_rf2;
            // ⚠️  LICENSE REQUIRED — real SNOMED CT data requires a license
            // from SNOMED International / your national NRC before use.
            let backend = SqliteTerminologyBackend::new(&database_url)?;
            let pool = backend.pool().clone();
            let path = args.path.clone();
            let batch_size = args.batch_size;
            let dry_run = args.dry_run;

            tokio::task::spawn_blocking(move || {
                import_snomed_rf2(&pool, &path, batch_size, dry_run)
            })
            .await
            .map_err(|e| anyhow::anyhow!("Import task panicked: {e}"))??
        }

        ImportFormat::Loinc => {
            use helios_hts::import::loinc_csv::import_loinc_csv;
            // ⚠️  LICENSE REQUIRED — real LOINC data requires a free license
            // from the Regenstrief Institute (registration at loinc.org, ~5 min).
            let backend = SqliteTerminologyBackend::new(&database_url)?;
            let pool = backend.pool().clone();
            let path = args.path.clone();
            let batch_size = args.batch_size;
            let dry_run = args.dry_run;

            tokio::task::spawn_blocking(move || import_loinc_csv(&pool, &path, batch_size, dry_run))
                .await
                .map_err(|e| anyhow::anyhow!("Import task panicked: {e}"))??
        }

        ImportFormat::Icd10Cm => {
            use helios_hts::import::icd10_cm::import_icd10_cm;
            let backend = SqliteTerminologyBackend::new(&database_url)?;
            let pool = backend.pool().clone();
            let path = args.path.clone();
            let batch_size = args.batch_size;
            let dry_run = args.dry_run;

            tokio::task::spawn_blocking(move || import_icd10_cm(&pool, &path, batch_size, dry_run))
                .await
                .map_err(|e| anyhow::anyhow!("Import task panicked: {e}"))??
        }

        ImportFormat::Icd9Cm => {
            use helios_hts::import::icd9_cm::import_icd9_cm;
            let backend = SqliteTerminologyBackend::new(&database_url)?;
            let pool = backend.pool().clone();
            let path = args.path.clone();
            let batch_size = args.batch_size;
            let dry_run = args.dry_run;

            tokio::task::spawn_blocking(move || import_icd9_cm(&pool, &path, batch_size, dry_run))
                .await
                .map_err(|e| anyhow::anyhow!("Import task panicked: {e}"))??
        }

        ImportFormat::Rxnorm => {
            use helios_hts::import::rxnorm_rrf::import_rxnorm_rrf;
            // ⚠️  LICENSE REQUIRED — real RxNorm data requires acceptance of
            // the NLM Terms of Service at https://www.nlm.nih.gov/databases/umls.html
            let backend = SqliteTerminologyBackend::new(&database_url)?;
            let pool = backend.pool().clone();
            let path = args.path.clone();
            let batch_size = args.batch_size;
            let dry_run = args.dry_run;

            tokio::task::spawn_blocking(move || {
                import_rxnorm_rrf(&pool, &path, batch_size, dry_run)
            })
            .await
            .map_err(|e| anyhow::anyhow!("Import task panicked: {e}"))??
        }

        ImportFormat::Ucum => {
            use helios_hts::import::ucum::import_ucum;
            let backend = SqliteTerminologyBackend::new(&database_url)?;
            let pool = backend.pool().clone();
            let path = args.path.clone();
            let batch_size = args.batch_size;
            let dry_run = args.dry_run;

            tokio::task::spawn_blocking(move || import_ucum(&pool, &path, batch_size, dry_run))
                .await
                .map_err(|e| anyhow::anyhow!("Import task panicked: {e}"))??
        }

        ImportFormat::NciThesaurus => {
            use helios_hts::import::nci_thesaurus::import_nci_thesaurus;
            let backend = SqliteTerminologyBackend::new(&database_url)?;
            let pool = backend.pool().clone();
            let path = args.path.clone();
            let batch_size = args.batch_size;
            let dry_run = args.dry_run;

            tokio::task::spawn_blocking(move || {
                import_nci_thesaurus(&pool, &path, batch_size, dry_run)
            })
            .await
            .map_err(|e| anyhow::anyhow!("Import task panicked: {e}"))??
        }

        ImportFormat::Mesh => {
            use helios_hts::import::mesh::import_mesh;
            let backend = SqliteTerminologyBackend::new(&database_url)?;
            let pool = backend.pool().clone();
            let path = args.path.clone();
            let batch_size = args.batch_size;
            let dry_run = args.dry_run;

            tokio::task::spawn_blocking(move || import_mesh(&pool, &path, batch_size, dry_run))
                .await
                .map_err(|e| anyhow::anyhow!("Import task panicked: {e}"))??
        }

        ImportFormat::Dicom => {
            use helios_hts::import::dicom::import_dicom;
            let backend = SqliteTerminologyBackend::new(&database_url)?;
            let pool = backend.pool().clone();
            let path = args.path.clone();
            let batch_size = args.batch_size;
            let dry_run = args.dry_run;

            tokio::task::spawn_blocking(move || import_dicom(&pool, &path, batch_size, dry_run))
                .await
                .map_err(|e| anyhow::anyhow!("Import task panicked: {e}"))??
        }

        ImportFormat::Hl7V2Tables => {
            use helios_hts::import::hl7_v2_tables::import_hl7_v2_tables;
            let backend = SqliteTerminologyBackend::new(&database_url)?;
            let pool = backend.pool().clone();
            let path = args.path.clone();
            let batch_size = args.batch_size;
            let dry_run = args.dry_run;

            tokio::task::spawn_blocking(move || {
                import_hl7_v2_tables(&pool, &path, batch_size, dry_run)
            })
            .await
            .map_err(|e| anyhow::anyhow!("Import task panicked: {e}"))??
        }

        ImportFormat::Nucc => {
            use helios_hts::import::nucc::import_nucc;
            let backend = SqliteTerminologyBackend::new(&database_url)?;
            let pool = backend.pool().clone();
            let path = args.path.clone();
            let batch_size = args.batch_size;
            let dry_run = args.dry_run;

            tokio::task::spawn_blocking(move || import_nucc(&pool, &path, batch_size, dry_run))
                .await
                .map_err(|e| anyhow::anyhow!("Import task panicked: {e}"))??
        }
    };

    let result = ImportResult::new(stats, format.to_string(), started.elapsed());

    print_import_summary(&result, args.dry_run, &format.to_string());

    if !result.stats.errors.is_empty() {
        return Ok(2);
    }

    Ok(0)
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use super::run_import_postgres;
    use helios_hts::config::{ImportArgs, ImportFormat};
    use std::path::PathBuf;

    fn postgres_args(format: ImportFormat) -> ImportArgs {
        ImportArgs {
            path: PathBuf::from("/nonexistent/file.zip"),
            format: Some(format),
            database_url: "postgres://localhost/hts_test".into(),
            storage_backend: "postgres".into(),
            log_level: "info".into(),
            batch_size: 500,
            dry_run: false,
            verbose: false,
        }
    }

    #[tokio::test]
    async fn run_import_postgres_snomed_returns_error_message() {
        let err = run_import_postgres(postgres_args(ImportFormat::SnomedRf2))
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("does not support") && msg.contains("PostgreSQL"),
            "expected unsupported-format error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn run_import_postgres_loinc_returns_error_message() {
        let err = run_import_postgres(postgres_args(ImportFormat::Loinc))
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("does not support") && msg.contains("PostgreSQL"),
            "expected unsupported-format error, got: {msg}"
        );
    }
}
