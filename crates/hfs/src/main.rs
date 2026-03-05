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
//! | S3 + Elasticsearch | `s3,elasticsearch` | S3 for CRUD/history and Elasticsearch for search |
//!
//! Set `HFS_STORAGE_BACKEND` to `sqlite`, `sqlite-elasticsearch`, `postgres`,
//! `postgres-elasticsearch`, `s3`, or `s3-elasticsearch`.

use clap::Parser;
use helios_rest::{ServerConfig, StorageBackendMode, create_app_with_config, init_logging};
use tracing::info;

#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};

/// Parses common truthy/falsey strings and falls back to `default`.
fn parse_bool_flag(value: Option<&str>, default: bool) -> bool {
    match value.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => match v.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        },
        None => default,
    }
}

#[cfg(feature = "elasticsearch")]
fn parse_comma_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(feature = "elasticsearch")]
fn create_elasticsearch_config(
    config: &ServerConfig,
) -> (
    helios_persistence::backends::elasticsearch::ElasticsearchConfig,
    Vec<String>,
) {
    use helios_persistence::backends::elasticsearch::{ElasticsearchAuth, ElasticsearchConfig};

    let es_nodes = parse_comma_list(&config.elasticsearch_nodes);

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

    (
        ElasticsearchConfig {
            nodes: es_nodes.clone(),
            index_prefix: config.elasticsearch_index_prefix.clone(),
            auth: es_auth,
            fhir_version: config.default_fhir_version,
            ..Default::default()
        },
        es_nodes,
    )
}

#[cfg(feature = "elasticsearch")]
fn build_search_registry(
    fhir_version: helios_fhir::FhirVersion,
    data_dir: Option<std::path::PathBuf>,
) -> std::sync::Arc<parking_lot::RwLock<helios_persistence::search::SearchParameterRegistry>> {
    use std::path::PathBuf;
    use std::sync::Arc;

    use helios_persistence::search::{SearchParameterLoader, SearchParameterRegistry};
    use parking_lot::RwLock;

    let loader = SearchParameterLoader::new(fhir_version);
    let registry = Arc::new(RwLock::new(SearchParameterRegistry::new()));
    let data_dir = data_dir.unwrap_or_else(|| PathBuf::from("./data"));

    {
        let mut reg = registry.write();

        if let Ok(params) = loader.load_embedded() {
            for param in params {
                let _ = reg.register(param);
            }
        }

        if let Ok(params) = loader.load_from_spec_file(&data_dir) {
            for param in params {
                let _ = reg.register(param);
            }
        }

        if let Ok((params, _files)) = loader.load_custom_from_directory_with_files(&data_dir) {
            for param in params {
                let _ = reg.register(param);
            }
        }

        info!(
            params = reg.len(),
            resource_types = reg.resource_types().len(),
            data_dir = %data_dir.display(),
            "Initialized shared SearchParameter registry"
        );
    }

    registry
}

#[cfg(feature = "s3")]
fn parse_tenant_bucket_map(raw: &str) -> anyhow::Result<std::collections::HashMap<String, String>> {
    use std::collections::HashMap;

    let mut out = HashMap::new();
    for entry in raw.split(',').map(str::trim).filter(|v| !v.is_empty()) {
        let (tenant, bucket) = entry
            .split_once(':')
            .or_else(|| entry.split_once('='))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Invalid HFS_S3_TENANT_BUCKET_MAP entry '{entry}'. Expected tenant:bucket"
                )
            })?;

        let tenant = tenant.trim();
        let bucket = bucket.trim();
        if tenant.is_empty() || bucket.is_empty() {
            anyhow::bail!(
                "Invalid HFS_S3_TENANT_BUCKET_MAP entry '{entry}'. Tenant and bucket must be non-empty"
            );
        }
        out.insert(tenant.to_string(), bucket.to_string());
    }
    Ok(out)
}

#[cfg(feature = "s3")]
fn build_s3_backend_config_from_env_with<F>(
    env_get: F,
) -> anyhow::Result<helios_persistence::backends::s3::S3BackendConfig>
where
    F: Fn(&str) -> Option<String>,
{
    use helios_persistence::backends::s3::{S3BackendConfig, S3TenancyMode};

    let normalized_mode = env_get("HFS_S3_TENANCY_MODE")
        .unwrap_or_else(|| "prefix-per-tenant".to_string())
        .trim()
        .to_ascii_lowercase()
        .replace('_', "-");

    let tenancy_mode = match normalized_mode.as_str() {
        "prefix" | "prefix-per-tenant" | "prefixpertenant" => {
            let bucket = env_get("HFS_S3_BUCKET").unwrap_or_else(|| "hfs".to_string());
            S3TenancyMode::PrefixPerTenant { bucket }
        }
        "bucket" | "bucket-per-tenant" | "bucketpertenant" => {
            let tenant_bucket_map_raw = env_get("HFS_S3_TENANT_BUCKET_MAP").unwrap_or_default();
            let tenant_bucket_map = parse_tenant_bucket_map(&tenant_bucket_map_raw)?;
            let default_system_bucket = env_get("HFS_S3_DEFAULT_SYSTEM_BUCKET")
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty());

            S3TenancyMode::BucketPerTenant {
                tenant_bucket_map,
                default_system_bucket,
            }
        }
        _ => {
            anyhow::bail!(
                "Invalid HFS_S3_TENANCY_MODE '{normalized_mode}'. Expected prefix-per-tenant or bucket-per-tenant"
            );
        }
    };

    Ok(S3BackendConfig {
        tenancy_mode,
        prefix: env_get("HFS_S3_PREFIX")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        region: env_get("HFS_S3_REGION")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        endpoint_url: env_get("HFS_S3_ENDPOINT_URL")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        force_path_style: parse_bool_flag(env_get("HFS_S3_FORCE_PATH_STYLE").as_deref(), false),
        allow_http: parse_bool_flag(env_get("HFS_S3_ALLOW_HTTP").as_deref(), false),
        validate_buckets_on_startup: parse_bool_flag(
            env_get("HFS_S3_VALIDATE_BUCKETS").as_deref(),
            true,
        ),
        ..Default::default()
    })
}

#[cfg(feature = "s3")]
fn build_s3_backend_config_from_env()
-> anyhow::Result<helios_persistence::backends::s3::S3BackendConfig> {
    build_s3_backend_config_from_env_with(|key| std::env::var(key).ok())
}

#[cfg(all(feature = "s3", feature = "elasticsearch"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct S3EsReindexStartupOptions {
    enabled: bool,
    batch_size: usize,
    clear_existing: bool,
    resource_types: Option<Vec<String>>,
}

#[cfg(all(feature = "s3", feature = "elasticsearch"))]
fn parse_s3_es_reindex_startup_options<F>(env_get: F) -> S3EsReindexStartupOptions
where
    F: Fn(&str) -> Option<String>,
{
    let enabled = parse_bool_flag(env_get("HFS_S3_ES_REINDEX_ON_STARTUP").as_deref(), false);
    let clear_existing = parse_bool_flag(
        env_get("HFS_S3_ES_REINDEX_CLEAR_EXISTING").as_deref(),
        false,
    );
    let batch_size = env_get("HFS_S3_ES_REINDEX_BATCH_SIZE")
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(500);

    let resource_types = env_get("HFS_S3_ES_REINDEX_RESOURCE_TYPES").and_then(|raw| {
        let parsed = raw
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if parsed.is_empty() {
            None
        } else {
            Some(parsed)
        }
    });

    S3EsReindexStartupOptions {
        enabled,
        batch_size,
        clear_existing,
        resource_types,
    }
}

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

    use helios_persistence::backends::elasticsearch::ElasticsearchBackend;
    use helios_persistence::composite::{CompositeConfig, CompositeStorage};
    use helios_persistence::core::BackendKind;

    // Create SQLite backend with search offloaded to Elasticsearch.
    let mut sqlite = create_sqlite_backend(&config)?;
    sqlite.set_search_offloaded(true);
    let sqlite = Arc::new(sqlite);
    info!("SQLite search indexing disabled (offloaded to Elasticsearch)");

    let (es_config, es_nodes) = create_elasticsearch_config(&config);
    info!(
        nodes = ?es_nodes,
        index_prefix = %config.elasticsearch_index_prefix,
        "Initializing Elasticsearch backend"
    );

    let es = Arc::new(ElasticsearchBackend::with_shared_registry(
        es_config,
        sqlite.search_registry().clone(),
    )?);

    let composite_config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .search_backend("es", BackendKind::Elasticsearch)
        .build()?;

    let mut backends = HashMap::new();
    backends.insert(
        "sqlite".to_string(),
        sqlite.clone() as helios_persistence::composite::DynStorage,
    );
    backends.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynStorage,
    );

    let mut search_providers = HashMap::new();
    search_providers.insert(
        "sqlite".to_string(),
        sqlite.clone() as helios_persistence::composite::DynSearchProvider,
    );
    search_providers.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynSearchProvider,
    );

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

    use helios_persistence::backends::elasticsearch::ElasticsearchBackend;
    use helios_persistence::backends::postgres::PostgresBackend;
    use helios_persistence::composite::{CompositeConfig, CompositeStorage};
    use helios_persistence::core::BackendKind;

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

    let mut backend = backend;
    backend.set_search_offloaded(true);
    let pg = Arc::new(backend);
    info!("PostgreSQL search indexing disabled (offloaded to Elasticsearch)");

    let (es_config, es_nodes) = create_elasticsearch_config(&config);
    info!(
        nodes = ?es_nodes,
        index_prefix = %config.elasticsearch_index_prefix,
        "Initializing Elasticsearch backend"
    );

    let es = Arc::new(ElasticsearchBackend::with_shared_registry(
        es_config,
        pg.search_registry().clone(),
    )?);

    let composite_config = CompositeConfig::builder()
        .primary("postgres", BackendKind::Postgres)
        .search_backend("es", BackendKind::Elasticsearch)
        .build()?;

    let mut backends = HashMap::new();
    backends.insert(
        "postgres".to_string(),
        pg.clone() as helios_persistence::composite::DynStorage,
    );
    backends.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynStorage,
    );

    let mut search_providers = HashMap::new();
    search_providers.insert(
        "postgres".to_string(),
        pg.clone() as helios_persistence::composite::DynSearchProvider,
    );
    search_providers.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynSearchProvider,
    );

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
    use helios_persistence::backends::s3::S3Backend;

    let s3_config = build_s3_backend_config_from_env()?;
    info!(
        tenancy_mode = ?s3_config.tenancy_mode,
        region = ?s3_config.region,
        endpoint_url = ?s3_config.endpoint_url,
        validate_buckets = s3_config.validate_buckets_on_startup,
        "Initializing S3 backend"
    );

    let backend = S3Backend::new(s3_config).map_err(|e| {
        anyhow::anyhow!(
            "Failed to initialize S3 backend (aws_region={:?}): {}",
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

    use helios_persistence::backends::elasticsearch::ElasticsearchBackend;
    use helios_persistence::backends::s3::{S3Backend, S3ToElasticsearchReindexOptions};
    use helios_persistence::composite::{CompositeConfig, CompositeStorage, SyncMode};
    use helios_persistence::core::BackendKind;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

    let s3_config = build_s3_backend_config_from_env()?;
    info!(
        tenancy_mode = ?s3_config.tenancy_mode,
        region = ?s3_config.region,
        endpoint_url = ?s3_config.endpoint_url,
        validate_buckets = s3_config.validate_buckets_on_startup,
        "Initializing S3 primary backend"
    );
    let s3 = Arc::new(S3Backend::new(s3_config)?);

    let (es_config, es_nodes) = create_elasticsearch_config(&config);
    info!(
        nodes = ?es_nodes,
        index_prefix = %config.elasticsearch_index_prefix,
        "Initializing Elasticsearch search backend"
    );

    let shared_registry =
        build_search_registry(config.default_fhir_version, config.data_dir.clone());
    let es = Arc::new(ElasticsearchBackend::with_shared_registry(
        es_config,
        shared_registry,
    )?);

    let composite_config = CompositeConfig::builder()
        .primary("s3", BackendKind::S3)
        .search_backend("es", BackendKind::Elasticsearch)
        .sync_mode(SyncMode::Synchronous)
        .build()?;

    let mut backends = HashMap::new();
    backends.insert(
        "s3".to_string(),
        s3.clone() as helios_persistence::composite::DynStorage,
    );
    backends.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynStorage,
    );

    let mut search_providers = HashMap::new();
    search_providers.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynSearchProvider,
    );

    let composite = CompositeStorage::new(composite_config, backends)?
        .with_search_providers(search_providers)
        .with_full_primary(s3.clone());

    let reindex_opts = parse_s3_es_reindex_startup_options(|key| std::env::var(key).ok());
    if reindex_opts.enabled {
        let tenant = TenantContext::new(
            TenantId::new(&config.default_tenant),
            TenantPermissions::full_access(),
        );

        info!(
            tenant = %tenant.tenant_id(),
            batch_size = reindex_opts.batch_size,
            clear_existing = reindex_opts.clear_existing,
            resource_types = ?reindex_opts.resource_types,
            "Running startup S3->Elasticsearch reindex"
        );

        let report = s3
            .reindex_to_elasticsearch(
                &tenant,
                es.as_ref(),
                S3ToElasticsearchReindexOptions {
                    batch_size: reindex_opts.batch_size,
                    clear_existing: reindex_opts.clear_existing,
                    resource_types: reindex_opts.resource_types,
                },
            )
            .await?;

        info!(
            tenant = %tenant.tenant_id(),
            scanned = report.scanned,
            indexed = report.indexed,
            deleted = report.deleted,
            skipped_deleted = report.skipped_deleted,
            "Startup S3->Elasticsearch reindex completed"
        );
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bool_flag_defaults_and_overrides() {
        assert!(parse_bool_flag(None, true));
        assert!(!parse_bool_flag(None, false));
        assert!(parse_bool_flag(Some("true"), false));
        assert!(parse_bool_flag(Some("1"), false));
        assert!(!parse_bool_flag(Some("false"), true));
        assert!(!parse_bool_flag(Some("0"), true));
        assert!(parse_bool_flag(Some("unknown"), true));
        assert!(!parse_bool_flag(Some("unknown"), false));
    }

    #[cfg(feature = "s3")]
    #[test]
    fn parse_s3_prefix_mode_from_env() {
        let config = build_s3_backend_config_from_env_with(|key| match key {
            "HFS_S3_TENANCY_MODE" => Some("prefix-per-tenant".to_string()),
            "HFS_S3_BUCKET" => Some("tenant-shared-bucket".to_string()),
            "HFS_S3_ALLOW_HTTP" => Some("true".to_string()),
            _ => None,
        })
        .expect("valid s3 config");

        match config.tenancy_mode {
            helios_persistence::backends::s3::S3TenancyMode::PrefixPerTenant { bucket } => {
                assert_eq!(bucket, "tenant-shared-bucket");
            }
            _ => panic!("expected prefix-per-tenant mode"),
        }
        assert!(config.allow_http);
    }

    #[cfg(feature = "s3")]
    #[test]
    fn parse_s3_bucket_per_tenant_from_env() {
        let config = build_s3_backend_config_from_env_with(|key| match key {
            "HFS_S3_TENANCY_MODE" => Some("bucket-per-tenant".to_string()),
            "HFS_S3_TENANT_BUCKET_MAP" => Some("a:bucket-a,b=bucket-b".to_string()),
            "HFS_S3_DEFAULT_SYSTEM_BUCKET" => Some("system-bucket".to_string()),
            _ => None,
        })
        .expect("valid bucket-per-tenant config");

        match config.tenancy_mode {
            helios_persistence::backends::s3::S3TenancyMode::BucketPerTenant {
                tenant_bucket_map,
                default_system_bucket,
            } => {
                assert_eq!(tenant_bucket_map.get("a"), Some(&"bucket-a".to_string()));
                assert_eq!(tenant_bucket_map.get("b"), Some(&"bucket-b".to_string()));
                assert_eq!(default_system_bucket, Some("system-bucket".to_string()));
            }
            _ => panic!("expected bucket-per-tenant mode"),
        }
    }

    #[cfg(feature = "s3")]
    #[test]
    fn parse_s3_invalid_bucket_map_rejected() {
        let result = build_s3_backend_config_from_env_with(|key| match key {
            "HFS_S3_TENANCY_MODE" => Some("bucket-per-tenant".to_string()),
            "HFS_S3_TENANT_BUCKET_MAP" => Some("bad-entry-without-separator".to_string()),
            _ => None,
        });

        assert!(result.is_err());
    }

    #[cfg(all(feature = "s3", feature = "elasticsearch"))]
    #[test]
    fn parse_reindex_startup_options_defaults() {
        let opts = parse_s3_es_reindex_startup_options(|_| None);
        assert!(!opts.enabled);
        assert_eq!(opts.batch_size, 500);
        assert!(!opts.clear_existing);
        assert!(opts.resource_types.is_none());
    }

    #[cfg(all(feature = "s3", feature = "elasticsearch"))]
    #[test]
    fn parse_reindex_startup_options_custom_values() {
        let opts = parse_s3_es_reindex_startup_options(|key| match key {
            "HFS_S3_ES_REINDEX_ON_STARTUP" => Some("true".to_string()),
            "HFS_S3_ES_REINDEX_BATCH_SIZE" => Some("250".to_string()),
            "HFS_S3_ES_REINDEX_CLEAR_EXISTING" => Some("1".to_string()),
            "HFS_S3_ES_REINDEX_RESOURCE_TYPES" => Some("Patient,Observation".to_string()),
            _ => None,
        });

        assert!(opts.enabled);
        assert_eq!(opts.batch_size, 250);
        assert!(opts.clear_existing);
        assert_eq!(
            opts.resource_types,
            Some(vec!["Patient".to_string(), "Observation".to_string()])
        );
    }
}
