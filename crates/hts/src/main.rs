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
    if !args.path.exists() {
        anyhow::bail!("Path does not exist: '{}'", args.path.display());
    }

    let format = match args.format {
        Some(f) => f,
        None => detect_format(&args.path).ok_or_else(|| {
            anyhow::anyhow!(
                "Cannot auto-detect format from '{}'.\n\
                 Use --format to specify one of:\n\
                 hl7-npm | snomed-rf2 | loinc | icd10-cm | icd9-cm | rxnorm |\n\
                 ucum | nci-thesaurus | mesh | dicom | hl7-v2-tables | nucc | ndc\n\
                 Note: .zip files may require --format if auto-detection is ambiguous.",
                args.path.display()
            )
        })?,
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
        storage_backend = %args.storage_backend,
        dry_run = args.dry_run,
        verbose = args.verbose,
        "Starting bulk import"
    );

    let started = std::time::Instant::now();
    let ctx = TenantContext::system();

    let stats = if args.storage_backend == "postgres" {
        #[cfg(feature = "postgres")]
        {
            let backend = PostgresTerminologyBackend::new(&args.database_url).await?;
            dispatch_import(format, &backend, &ctx, &args).await?
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
            dispatch_import(format, &backend, &ctx, &args).await?
        }
        #[cfg(not(feature = "sqlite"))]
        anyhow::bail!(
            "sqlite storage backend requested but the 'sqlite' feature is not enabled. \
             Rebuild with `--features sqlite`."
        );
    };

    let result = ImportResult::new(stats, format.to_string(), started.elapsed());

    print_import_summary(&result, args.dry_run, &format.to_string());

    if !result.stats.errors.is_empty() {
        return Ok(2);
    }
    Ok(0)
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
async fn dispatch_import(
    format: ImportFormat,
    backend: &dyn BundleImportBackend,
    ctx: &TenantContext,
    args: &ImportArgs,
) -> Result<ImportStats, helios_hts::error::HtsError> {
    use helios_hts::import::{
        dicom::import_dicom, hl7_v2_tables::import_hl7_v2_tables, icd9_cm::import_icd9_cm,
        icd10_cm::import_icd10_cm, loinc_csv::import_loinc_csv, mesh::import_mesh,
        nci_thesaurus::import_nci_thesaurus, ndc::import_ndc, nucc::import_nucc,
        rxnorm_rrf::import_rxnorm_rrf, snomed_rf2::import_snomed_rf2, tgz::import_tgz,
        ucum::import_ucum,
    };

    match format {
        ImportFormat::Hl7Npm => {
            import_tgz(backend, ctx, &args.path, args.batch_size, args.dry_run).await
        }
        ImportFormat::SnomedRf2 => {
            import_snomed_rf2(backend, ctx, &args.path, args.batch_size, args.dry_run).await
        }
        ImportFormat::Loinc => {
            import_loinc_csv(backend, ctx, &args.path, args.batch_size, args.dry_run).await
        }
        ImportFormat::Icd10Cm => {
            import_icd10_cm(backend, ctx, &args.path, args.batch_size, args.dry_run).await
        }
        ImportFormat::Icd9Cm => {
            import_icd9_cm(backend, ctx, &args.path, args.batch_size, args.dry_run).await
        }
        ImportFormat::Rxnorm => {
            import_rxnorm_rrf(backend, ctx, &args.path, args.batch_size, args.dry_run).await
        }
        ImportFormat::Ucum => {
            import_ucum(backend, ctx, &args.path, args.batch_size, args.dry_run).await
        }
        ImportFormat::NciThesaurus => {
            import_nci_thesaurus(backend, ctx, &args.path, args.batch_size, args.dry_run).await
        }
        ImportFormat::Mesh => {
            import_mesh(backend, ctx, &args.path, args.batch_size, args.dry_run).await
        }
        ImportFormat::Dicom => {
            import_dicom(backend, ctx, &args.path, args.batch_size, args.dry_run).await
        }
        ImportFormat::Hl7V2Tables => {
            import_hl7_v2_tables(backend, ctx, &args.path, args.batch_size, args.dry_run).await
        }
        ImportFormat::Nucc => {
            import_nucc(backend, ctx, &args.path, args.batch_size, args.dry_run).await
        }
        ImportFormat::Ndc => {
            import_ndc(backend, ctx, &args.path, args.batch_size, args.dry_run).await
        }
    }
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
