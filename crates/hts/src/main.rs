use clap::Parser;
use helios_hts::config::{Cli, Command, HtsConfig, ImportArgs, ImportFormat, detect_format};
use helios_hts::import::ImportResult;
use tracing::info;

#[cfg(feature = "sqlite")]
use helios_hts::backend::SqliteTerminologyBackend;
#[cfg(feature = "sqlite")]
use helios_hts::state::AppState;

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
        .unwrap_or_else(|| Command::Serve(HtsConfig::default()))
    {
        Command::Serve(config) => {
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
async fn run_server(_config: HtsConfig) -> anyhow::Result<()> {
    anyhow::bail!(
        "No storage backend feature is enabled. \
         Rebuild with `--features sqlite` (or another backend feature)."
    )
}

// ── Import ────────────────────────────────────────────────────────────────────

/// Returns the process exit code:
/// - `0` — success, all resources imported
/// - `1` — fatal error (propagated as `Err` by `?`)
/// - `2` — success with non-fatal errors (some records skipped)
#[cfg(feature = "sqlite")]
async fn run_import(args: ImportArgs) -> anyhow::Result<i32> {
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
                 Use --format to specify one of: hl7-npm | snomed-rf2 | loinc | icd10-cm | rxnorm\n\
                 Note: .zip files require --format because SNOMED and LOINC share the same extension.",
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
        dry_run = args.dry_run,
        verbose = args.verbose,
        "Starting bulk import"
    );

    // ── Route to the correct importer ──────────────────────────────────────
    let started = std::time::Instant::now();

    let stats = match format {
        ImportFormat::Hl7Npm => {
            let backend = SqliteTerminologyBackend::new(&args.database_url)?;
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
            let backend = SqliteTerminologyBackend::new(&args.database_url)?;
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
            let backend = SqliteTerminologyBackend::new(&args.database_url)?;
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
            let backend = SqliteTerminologyBackend::new(&args.database_url)?;
            let pool = backend.pool().clone();
            let path = args.path.clone();
            let batch_size = args.batch_size;
            let dry_run = args.dry_run;

            tokio::task::spawn_blocking(move || import_icd10_cm(&pool, &path, batch_size, dry_run))
                .await
                .map_err(|e| anyhow::anyhow!("Import task panicked: {e}"))??
        }

        ImportFormat::Rxnorm => {
            use helios_hts::import::rxnorm_rrf::import_rxnorm_rrf;
            // ⚠️  LICENSE REQUIRED — real RxNorm data requires acceptance of
            // the NLM Terms of Service at https://www.nlm.nih.gov/databases/umls.html
            let backend = SqliteTerminologyBackend::new(&args.database_url)?;
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
    };

    let result = ImportResult::new(stats, format.to_string(), started.elapsed());

    // ── Summary ────────────────────────────────────────────────────────────
    let dry_label = if args.dry_run { " (dry-run)" } else { "" };
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
        return Ok(2);
    }

    Ok(0)
}

#[cfg(not(feature = "sqlite"))]
async fn run_import(_args: ImportArgs) -> anyhow::Result<i32> {
    anyhow::bail!(
        "No storage backend feature is enabled. \
         Rebuild with `--features sqlite` (or another backend feature)."
    )
}
