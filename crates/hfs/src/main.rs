//! Helios FHIR Server (HFS)
//!
//! A high-performance FHIR R4/R4B/R5/R6 server with pluggable storage backends.
//!
//! # Storage Backends
//!
//! | Backend | Feature Flag | Description |
//! |---------|--------------|-------------|
//! | SQLite (default) | `sqlite` | Zero-config embedded database with FTS5 search |
//! | SQLite + Elasticsearch | `sqlite,elasticsearch` | SQLite for CRUD, Elasticsearch for search |
//! | PostgreSQL | `postgres` | Full-featured RDBMS with JSONB storage and tsvector search |
//! | PostgreSQL + Elasticsearch | `postgres,elasticsearch` | PostgreSQL for CRUD, Elasticsearch for search |
//! | S3 | `s3` | AWS S3 object storage for CRUD, versioning, history, and bulk ops (no search) |
//! | S3 + Elasticsearch | `s3,elasticsearch` | S3 for CRUD, Elasticsearch for search |
//!
//! Set `HFS_STORAGE_BACKEND` to `sqlite`, `sqlite-elasticsearch`, `postgres`, `postgres-elasticsearch`, `s3`, or `s3-elasticsearch`.

use clap::Parser;
use helios_rest::{ServerConfig, StorageBackendMode, create_app_with_config, init_logging};
use tracing::info;

#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};

/// Creates and initializes a SQLite backend from the server configuration.
#[cfg(feature = "sqlite")]
fn create_sqlite_backend(config: &ServerConfig) -> anyhow::Result<SqliteBackend> {
    let db_path = config.database_url.as_deref().unwrap_or("fhir.db");
    info!(database = %db_path, "Initializing SQLite backend");

    let backend_config = SqliteBackendConfig {
        fhir_version: config.default_fhir_version,
        data_dir: config.data_dir.clone(),
        ..Default::default()
    };

    let backend = if db_path == ":memory:" {
        SqliteBackend::with_config(":memory:", backend_config)?
    } else {
        SqliteBackend::with_config(db_path, backend_config)?
    };
    backend.init_schema()?;

    Ok(backend)
}

/// Starts the Axum HTTP server.
async fn serve(app: axum::Router, config: &ServerConfig) -> anyhow::Result<()> {
    let addr = config.socket_addr();
    info!(address = %addr, "Server listening");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = ServerConfig::parse();
    init_logging(&config.log_level);

    if let Err(errors) = config.validate() {
        for error in &errors {
            eprintln!("Configuration error: {}", error);
        }
        std::process::exit(1);
    }

    let backend_mode = config
        .storage_backend_mode()
        .map_err(|e| anyhow::anyhow!("Invalid storage backend configuration: {}", e))?;

    info!(
        port = config.port,
        host = %config.host,
        fhir_version = ?config.default_fhir_version,
        storage_backend = %backend_mode,
        "Starting Helios FHIR Server"
    );

    match backend_mode {
        StorageBackendMode::Sqlite => {
            start_sqlite(config).await?;
        }
        StorageBackendMode::SqliteElasticsearch => {
            start_sqlite_elasticsearch(config).await?;
        }
        StorageBackendMode::Postgres => {
            start_postgres(config).await?;
        }
        StorageBackendMode::PostgresElasticsearch => {
            start_postgres_elasticsearch(config).await?;
        }
        StorageBackendMode::S3 => {
            start_s3(config).await?;
        }
        StorageBackendMode::S3Elasticsearch => {
            start_s3_elasticsearch(config).await?;
        }
    }

    Ok(())
}

/// Starts the server with SQLite-only backend.
#[cfg(feature = "sqlite")]
async fn start_sqlite(config: ServerConfig) -> anyhow::Result<()> {
    let backend = create_sqlite_backend(&config)?;
    let app = create_app_with_config(backend, config.clone());
    serve(app, &config).await
}

/// Fallback when sqlite feature is not enabled.
#[cfg(not(feature = "sqlite"))]
async fn start_sqlite(_config: ServerConfig) -> anyhow::Result<()> {
    anyhow::bail!(
        "The sqlite backend requires the 'sqlite' feature. \
         Build with: cargo build -p helios-hfs --features sqlite"
    )
}

/// Starts the server with SQLite + Elasticsearch composite backend.
#[cfg(all(feature = "sqlite", feature = "elasticsearch"))]
async fn start_sqlite_elasticsearch(config: ServerConfig) -> anyhow::Result<()> {
    use std::collections::HashMap;
    use std::sync::Arc;

    use helios_persistence::backends::elasticsearch::{
        ElasticsearchAuth, ElasticsearchBackend, ElasticsearchConfig,
    };
    use helios_persistence::composite::{CompositeConfig, CompositeStorage};
    use helios_persistence::core::BackendKind;

    // Create SQLite backend with search offloaded to Elasticsearch
    let mut sqlite = create_sqlite_backend(&config)?;
    sqlite.set_search_offloaded(true);
    let sqlite = Arc::new(sqlite);
    info!("SQLite search indexing disabled (offloaded to Elasticsearch)");

    // Build Elasticsearch configuration from server config
    let es_nodes: Vec<String> = config
        .elasticsearch_nodes
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let es_auth = match (
        &config.elasticsearch_username,
        &config.elasticsearch_password,
    ) {
        (Some(username), Some(password)) => Some(ElasticsearchAuth::Basic {
            username: username.clone(),
            password: password.clone(),
        }),
        _ => None,
    };

    let es_config = ElasticsearchConfig {
        nodes: es_nodes.clone(),
        index_prefix: config.elasticsearch_index_prefix.clone(),
        auth: es_auth,
        fhir_version: config.default_fhir_version,
        ..Default::default()
    };

    info!(
        nodes = ?es_nodes,
        index_prefix = %config.elasticsearch_index_prefix,
        "Initializing Elasticsearch backend"
    );

    // Create ES backend sharing SQLite's search parameter registry
    let es = Arc::new(ElasticsearchBackend::with_shared_registry(
        es_config,
        sqlite.search_registry().clone(),
    )?);

    // Build composite configuration
    let composite_config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .search_backend("es", BackendKind::Elasticsearch)
        .build()?;

    // Build backends map for CompositeStorage
    let mut backends = HashMap::new();
    backends.insert(
        "sqlite".to_string(),
        sqlite.clone() as helios_persistence::composite::DynStorage,
    );
    backends.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynStorage,
    );

    // Build search providers map
    let mut search_providers = HashMap::new();
    search_providers.insert(
        "sqlite".to_string(),
        sqlite.clone() as helios_persistence::composite::DynSearchProvider,
    );
    search_providers.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynSearchProvider,
    );

    // Create composite storage with full primary capabilities
    let composite = CompositeStorage::new(composite_config, backends)?
        .with_search_providers(search_providers)
        .with_full_primary(sqlite);

    info!("Composite storage initialized: SQLite (primary) + Elasticsearch (search)");

    let app = create_app_with_config(composite, config.clone());
    serve(app, &config).await
}

/// Fallback when elasticsearch feature is not enabled.
#[cfg(not(all(feature = "sqlite", feature = "elasticsearch")))]
async fn start_sqlite_elasticsearch(_config: ServerConfig) -> anyhow::Result<()> {
    anyhow::bail!(
        "The sqlite-elasticsearch backend requires the 'elasticsearch' feature. \
         Build with: cargo build -p helios-hfs --features sqlite,elasticsearch"
    )
}

/// Starts the server with PostgreSQL backend.
#[cfg(feature = "postgres")]
async fn start_postgres(config: ServerConfig) -> anyhow::Result<()> {
    use helios_persistence::backends::postgres::PostgresBackend;

    let backend = if let Some(ref url) = config.database_url {
        if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            info!(url = %url, "Initializing PostgreSQL backend from connection string");
            PostgresBackend::from_connection_string(url).await?
        } else {
            info!("Initializing PostgreSQL backend from environment variables");
            PostgresBackend::from_env().await?
        }
    } else {
        info!("Initializing PostgreSQL backend from environment variables");
        PostgresBackend::from_env().await?
    };

    backend.init_schema().await?;

    let app = create_app_with_config(backend, config.clone());
    serve(app, &config).await
}

/// Fallback when postgres feature is not enabled.
#[cfg(not(feature = "postgres"))]
async fn start_postgres(_config: ServerConfig) -> anyhow::Result<()> {
    anyhow::bail!(
        "The postgres backend requires the 'postgres' feature. \
         Build with: cargo build -p helios-hfs --features postgres"
    )
}

/// Starts the server with PostgreSQL + Elasticsearch composite backend.
#[cfg(all(feature = "postgres", feature = "elasticsearch"))]
async fn start_postgres_elasticsearch(config: ServerConfig) -> anyhow::Result<()> {
    use std::collections::HashMap;
    use std::sync::Arc;

    use helios_persistence::backends::elasticsearch::{
        ElasticsearchAuth, ElasticsearchBackend, ElasticsearchConfig,
    };
    use helios_persistence::backends::postgres::PostgresBackend;
    use helios_persistence::composite::{CompositeConfig, CompositeStorage};
    use helios_persistence::core::BackendKind;

    // Create PostgreSQL backend
    let backend = if let Some(ref url) = config.database_url {
        if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            info!(url = %url, "Initializing PostgreSQL backend from connection string");
            PostgresBackend::from_connection_string(url).await?
        } else {
            info!("Initializing PostgreSQL backend from environment variables");
            PostgresBackend::from_env().await?
        }
    } else {
        info!("Initializing PostgreSQL backend from environment variables");
        PostgresBackend::from_env().await?
    };

    backend.init_schema().await?;

    // Offload search to Elasticsearch
    let mut backend = backend;
    backend.set_search_offloaded(true);
    let pg = Arc::new(backend);
    info!("PostgreSQL search indexing disabled (offloaded to Elasticsearch)");

    // Build Elasticsearch configuration from server config
    let es_nodes: Vec<String> = config
        .elasticsearch_nodes
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let es_auth = match (
        &config.elasticsearch_username,
        &config.elasticsearch_password,
    ) {
        (Some(username), Some(password)) => Some(ElasticsearchAuth::Basic {
            username: username.clone(),
            password: password.clone(),
        }),
        _ => None,
    };

    let es_config = ElasticsearchConfig {
        nodes: es_nodes.clone(),
        index_prefix: config.elasticsearch_index_prefix.clone(),
        auth: es_auth,
        fhir_version: config.default_fhir_version,
        ..Default::default()
    };

    info!(
        nodes = ?es_nodes,
        index_prefix = %config.elasticsearch_index_prefix,
        "Initializing Elasticsearch backend"
    );

    // Create ES backend sharing PostgreSQL's search parameter registry
    let es = Arc::new(ElasticsearchBackend::with_shared_registry(
        es_config,
        pg.search_registry().clone(),
    )?);

    // Build composite configuration
    let composite_config = CompositeConfig::builder()
        .primary("postgres", BackendKind::Postgres)
        .search_backend("es", BackendKind::Elasticsearch)
        .build()?;

    // Build backends map for CompositeStorage
    let mut backends = HashMap::new();
    backends.insert(
        "postgres".to_string(),
        pg.clone() as helios_persistence::composite::DynStorage,
    );
    backends.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynStorage,
    );

    // Build search providers map
    let mut search_providers = HashMap::new();
    search_providers.insert(
        "postgres".to_string(),
        pg.clone() as helios_persistence::composite::DynSearchProvider,
    );
    search_providers.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynSearchProvider,
    );

    // Create composite storage with full primary capabilities
    let composite = CompositeStorage::new(composite_config, backends)?
        .with_search_providers(search_providers)
        .with_full_primary(pg);

    info!("Composite storage initialized: PostgreSQL (primary) + Elasticsearch (search)");

    let app = create_app_with_config(composite, config.clone());
    serve(app, &config).await
}

/// Fallback when postgres+elasticsearch features are not both enabled.
#[cfg(not(all(feature = "postgres", feature = "elasticsearch")))]
async fn start_postgres_elasticsearch(_config: ServerConfig) -> anyhow::Result<()> {
    anyhow::bail!(
        "The postgres-elasticsearch backend requires both 'postgres' and 'elasticsearch' features. \
         Build with: cargo build -p helios-hfs --features postgres,elasticsearch"
    )
}

/// Starts the server with AWS S3 backend.
#[cfg(feature = "s3")]
async fn start_s3(config: ServerConfig) -> anyhow::Result<()> {
    use helios_persistence::backends::s3::{S3Backend, S3BackendConfig, S3TenancyMode};

    let bucket = std::env::var("HFS_S3_BUCKET").unwrap_or_else(|_| "hfs".to_string());
    let region = std::env::var("HFS_S3_REGION").ok();
    let prefix = std::env::var("HFS_S3_PREFIX").ok();
    let validate_buckets = std::env::var("HFS_S3_VALIDATE_BUCKETS")
        .map(|s| s.to_lowercase() != "false" && s != "0")
        .unwrap_or(true);

    info!(
        bucket = %bucket,
        region = ?region,
        prefix = ?prefix,
        validate_buckets = validate_buckets,
        "Initializing S3 backend"
    );

    let s3_config = S3BackendConfig {
        tenancy_mode: S3TenancyMode::PrefixPerTenant {
            bucket: bucket.clone(),
        },
        region,
        prefix,
        validate_buckets_on_startup: validate_buckets,
        ..Default::default()
    };

    let backend = S3Backend::from_env_async(s3_config).await.map_err(|e| {
        anyhow::anyhow!(
            "Failed to initialize S3 backend (bucket={}, region={:?}): {}",
            bucket,
            std::env::var("AWS_REGION").ok(),
            e
        )
    })?;

    let app = create_app_with_config(backend, config.clone());
    serve(app, &config).await
}

/// Fallback when s3 feature is not enabled.
#[cfg(not(feature = "s3"))]
async fn start_s3(_config: ServerConfig) -> anyhow::Result<()> {
    anyhow::bail!(
        "The s3 backend requires the 's3' feature. \
         Build with: cargo build -p helios-hfs --features s3"
    )
}

/// Starts the server with S3 + Elasticsearch composite backend.
#[cfg(all(feature = "s3", feature = "elasticsearch"))]
async fn start_s3_elasticsearch(config: ServerConfig) -> anyhow::Result<()> {
    use std::collections::HashMap;
    use std::sync::Arc;

    use helios_persistence::backends::elasticsearch::{
        ElasticsearchAuth, ElasticsearchBackend, ElasticsearchConfig,
    };
    use helios_persistence::backends::s3::{S3Backend, S3BackendConfig, S3TenancyMode};
    use helios_persistence::composite::{CompositeConfig, CompositeStorage};
    use helios_persistence::core::BackendKind;

    let bucket = std::env::var("HFS_S3_BUCKET").unwrap_or_else(|_| "hfs".to_string());
    let region = std::env::var("HFS_S3_REGION").ok();
    let prefix = std::env::var("HFS_S3_PREFIX").ok();
    let validate_buckets = std::env::var("HFS_S3_VALIDATE_BUCKETS")
        .map(|s| s.to_lowercase() != "false" && s != "0")
        .unwrap_or(true);

    info!(
        bucket = %bucket,
        region = ?region,
        prefix = ?prefix,
        validate_buckets = validate_buckets,
        "Initializing S3 backend"
    );

    let s3_config = S3BackendConfig {
        tenancy_mode: S3TenancyMode::PrefixPerTenant {
            bucket: bucket.clone(),
        },
        region,
        prefix,
        validate_buckets_on_startup: validate_buckets,
        ..Default::default()
    };

    let s3 = Arc::new(S3Backend::from_env_async(s3_config).await.map_err(|e| {
        anyhow::anyhow!(
            "Failed to initialize S3 backend (bucket={}, region={:?}): {}",
            bucket,
            std::env::var("AWS_REGION").ok(),
            e
        )
    })?);

    // Build Elasticsearch configuration from server config
    let es_nodes: Vec<String> = config
        .elasticsearch_nodes
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let es_auth = match (
        &config.elasticsearch_username,
        &config.elasticsearch_password,
    ) {
        (Some(username), Some(password)) => Some(ElasticsearchAuth::Basic {
            username: username.clone(),
            password: password.clone(),
        }),
        _ => None,
    };

    let es_config = ElasticsearchConfig {
        nodes: es_nodes.clone(),
        index_prefix: config.elasticsearch_index_prefix.clone(),
        auth: es_auth,
        fhir_version: config.default_fhir_version,
        ..Default::default()
    };

    info!(
        nodes = ?es_nodes,
        index_prefix = %config.elasticsearch_index_prefix,
        "Initializing Elasticsearch backend"
    );

    // S3 has no search parameter registry, so ES creates its own standalone registry.
    let es = Arc::new(ElasticsearchBackend::new(es_config)?);

    // Build composite configuration
    let composite_config = CompositeConfig::builder()
        .primary("s3", BackendKind::S3)
        .search_backend("es", BackendKind::Elasticsearch)
        .build()?;

    // Build backends map for CompositeStorage
    let mut backends = HashMap::new();
    backends.insert(
        "s3".to_string(),
        s3.clone() as helios_persistence::composite::DynStorage,
    );
    backends.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynStorage,
    );

    // Build search providers map
    let mut search_providers = HashMap::new();
    search_providers.insert(
        "s3".to_string(),
        s3.clone() as helios_persistence::composite::DynSearchProvider,
    );
    search_providers.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynSearchProvider,
    );

    // Create composite storage with full primary capabilities
    let composite = CompositeStorage::new(composite_config, backends)?
        .with_search_providers(search_providers)
        .with_full_primary(s3);

    info!("Composite storage initialized: S3 (primary) + Elasticsearch (search)");

    let app = create_app_with_config(composite, config.clone());
    serve(app, &config).await
}

/// Fallback when s3+elasticsearch features are not both enabled.
#[cfg(not(all(feature = "s3", feature = "elasticsearch")))]
async fn start_s3_elasticsearch(_config: ServerConfig) -> anyhow::Result<()> {
    anyhow::bail!(
        "The s3-elasticsearch backend requires both 's3' and 'elasticsearch' features. \
         Build with: cargo build -p helios-hfs --features s3,elasticsearch"
    )
}

#[cfg(not(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "s3"
)))]
compile_error!("At least one database backend feature must be enabled");
