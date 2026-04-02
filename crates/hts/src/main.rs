use clap::Parser;
use helios_hts::config::HtsConfig;
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
    let config = HtsConfig::parse();
    init_logging(&config.log_level);

    info!(
        port = config.port,
        host = %config.host,
        storage_backend = %config.storage_backend,
        database_url = %config.database_url,
        "Starting Helios Terminology Service"
    );

    run(config).await
}

#[cfg(feature = "sqlite")]
async fn run(config: HtsConfig) -> anyhow::Result<()> {
    use helios_persistence::backends::sqlite::SqliteBackend;

    // ── HTS terminology backend ──────────────────────────────────────────────
    // Manages the normalized terminology tables (code_systems, concepts, …).
    let backend = SqliteTerminologyBackend::new(&config.database_url)?;

    // Clone the pool so CRUD handlers can re-index normalized tables without
    // going through the TerminologyBackend trait.
    let hts_pool = backend.pool().clone();

    // ── helios-persistence raw resource store ────────────────────────────────
    // Shares the same SQLite file; uses separate tables (resources, resource_history).
    // Provides versioning, ETags, and soft-delete semantics for the CRUD API.
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
async fn run(_config: HtsConfig) -> anyhow::Result<()> {
    anyhow::bail!(
        "No storage backend feature is enabled. \
         Rebuild with `--features sqlite` (or another backend feature)."
    )
}
