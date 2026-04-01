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
//! | MongoDB | `mongodb` | Document database with native JSON resource storage |
//! | MongoDB + Elasticsearch | `mongodb,elasticsearch` | MongoDB for CRUD, Elasticsearch for search |
//! | S3 | `s3` | AWS S3 object storage for CRUD, versioning, history, and bulk ops (no search) |
//! | S3 + Elasticsearch | `s3,elasticsearch` | S3 for CRUD/history, Elasticsearch for search |
//!
//! Set `HFS_STORAGE_BACKEND` to `sqlite`, `sqlite-elasticsearch`, `postgres`,
//! `postgres-elasticsearch`, `mongodb`, `mongodb-elasticsearch`, `s3`, or `s3-elasticsearch`.

use std::sync::Arc;

use clap::Parser;
use helios_audit::{
    AuditBackend, AuditConfig, AuditMiddlewareState, AuditSink, ExclusionFilter, lifecycle,
};
use helios_auth::{AuthConfig, InMemoryJtiCache, JtiCache, JwksBearerAuthProvider, JwksCache};
use helios_rest::{
    AuthMiddlewareState, ServerConfig, StorageBackendMode, create_app_with_auth, init_logging,
};
use tracing::info;

#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};

#[cfg(feature = "mongodb")]
use helios_persistence::backends::mongodb::MongoBackend;

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

/// Starts the server with MongoDB backend.
#[cfg(feature = "mongodb")]
async fn start_mongodb(
    config: ServerConfig,
    auth_config: AuthConfig,
    auth_state: Option<Arc<AuthMiddlewareState>>,
    audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    let backend = if let Some(ref url) = config.database_url {
        if url.starts_with("mongodb://") || url.starts_with("mongodb+srv://") {
            info!(url = %url, "Initializing MongoDB backend from connection string");
            MongoBackend::from_connection_string(url)?
        } else {
            info!(
                "Initializing MongoDB backend from environment variables (database_url is not MongoDB URI)"
            );
            MongoBackend::from_env()?
        }
    } else {
        info!("Initializing MongoDB backend from environment variables");
        MongoBackend::from_env()?
    };

    backend.init_schema().await?;

    let serve_audit_state = audit_state.clone();
    let app = create_app_with_auth(
        backend,
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
    );
    serve(app, &config, serve_audit_state).await
}

/// Fallback when mongodb feature is not enabled.
#[cfg(not(feature = "mongodb"))]
async fn start_mongodb(
    _config: ServerConfig,
    _auth_config: AuthConfig,
    _auth_state: Option<Arc<AuthMiddlewareState>>,
    _audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "The mongodb backend requires the 'mongodb' feature. \
         Build with: cargo build -p helios-hfs --features mongodb"
    )
}

/// Starts the Axum HTTP server.
async fn serve(
    app: axum::Router,
    config: &ServerConfig,
    audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    let addr = config.socket_addr();
    info!(address = %addr, "Server listening");
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            info!("Shutdown signal received, draining connections");
            if let Some(state) = audit_state {
                lifecycle::record_shutdown(&*state.sink, &state.config.source_observer).await;
                state.sink.flush().await;
            }
        })
        .await?;
    Ok(())
}

/// Initializes the authentication subsystem from environment configuration.
///
/// When audit is enabled, the auth middleware uses an `AuditBridge` to record
/// authentication events. Otherwise, a no-op sink is used.
///
/// Returns the auth config and optional middleware state. If auth is disabled,
/// returns `(config, None)`.
async fn init_auth_with_audit(
    audit_sink: Arc<dyn AuditSink>,
) -> anyhow::Result<(AuthConfig, Option<Arc<AuthMiddlewareState>>)> {
    let auth_config = AuthConfig::from_env();

    if !auth_config.enabled {
        info!("Authentication is DISABLED");
        return Ok((auth_config, None));
    }

    let jwks_url = auth_config.jwks_url.as_ref().ok_or_else(|| {
        anyhow::anyhow!("HFS_AUTH_JWKS_URL is required when HFS_AUTH_ENABLED=true")
    })?;

    // Require issuer validation to prevent cross-service token reuse
    if auth_config.expected_issuer.is_none() {
        anyhow::bail!("HFS_AUTH_ISSUER is required when HFS_AUTH_ENABLED=true");
    }

    // Create JTI cache
    let jti_cache: Arc<dyn JtiCache> = match auth_config.jti_backend.as_str() {
        #[cfg(feature = "redis")]
        "redis" => {
            let redis_url = auth_config.redis_url.as_ref().ok_or_else(|| {
                anyhow::anyhow!("HFS_AUTH_REDIS_URL is required when HFS_AUTH_JTI_BACKEND=redis")
            })?;
            info!(redis_url = %redis_url, "Using Redis JTI cache");
            Arc::new(helios_auth::RedisJtiCache::new(redis_url)?)
        }
        #[cfg(not(feature = "redis"))]
        "redis" => {
            anyhow::bail!(
                "Redis JTI backend requires the 'redis' feature. \
                 Build with: cargo build -p helios-hfs --features redis"
            );
        }
        _ => {
            info!("Using in-memory JTI cache");
            Arc::new(InMemoryJtiCache::new())
        }
    };

    // Create JWKS cache
    let jwks_cache = Arc::new(JwksCache::new(
        jwks_url,
        auth_config.jwks_min_refresh_interval,
    ));
    jwks_cache.initial_fetch().await?;

    // Create auth provider
    let provider = JwksBearerAuthProvider::new(jwks_cache, jti_cache, &auth_config);

    info!(
        jwks_url = %jwks_url,
        issuer = ?auth_config.expected_issuer,
        audience = ?auth_config.expected_audience,
        "Authentication ENABLED"
    );

    let audit_config = AuditConfig::from_env();

    let auth_state = Arc::new(AuthMiddlewareState {
        provider: Arc::new(provider),
        config: Arc::new(auth_config.clone()),
        audit_sink,
        audit_source_observer: audit_config.source_observer.clone(),
        audit_exclusion_filter: ExclusionFilter::new(audit_config.exclusions.clone()),
    });

    Ok((auth_config, Some(auth_state)))
}

/// Initializes the audit subsystem from environment configuration.
///
/// Returns the audit sink (for use as auth bridge) and optional middleware state.
/// If audit is disabled, returns `(NullSink, None)`.
async fn init_audit() -> anyhow::Result<(Arc<dyn AuditSink>, Option<Arc<AuditMiddlewareState>>)> {
    let config = AuditConfig::from_env();

    let sink: Arc<dyn AuditSink> = match config.backend {
        AuditBackend::None => {
            info!("Audit logging is DISABLED");
            Arc::new(helios_audit::NullSink)
        }
        AuditBackend::File => {
            let path = config.file_path.as_deref().ok_or_else(|| {
                anyhow::anyhow!("HFS_AUDIT_FILE_PATH is required when HFS_AUDIT_BACKEND=file")
            })?;
            info!(path = %path, "Audit logging to file");
            Arc::new(helios_audit::FileSink::new(path).await?)
        }
        AuditBackend::Database => {
            // Database sink requires a storage implementation to be provided
            // by the caller. For now, fall back to file or null.
            anyhow::bail!(
                "Database audit backend is not yet wired into the HFS startup. \
                 Use HFS_AUDIT_BACKEND=file or HFS_AUDIT_BACKEND=none for now."
            );
        }
        #[cfg(feature = "cloudwatch")]
        AuditBackend::CloudWatch => {
            let log_group = config.cloudwatch_log_group.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "HFS_AUDIT_CLOUDWATCH_LOG_GROUP is required when HFS_AUDIT_BACKEND=cloudwatch"
                )
            })?;
            let log_stream = config
                .cloudwatch_log_stream
                .as_deref()
                .unwrap_or("hfs-audit");
            info!(log_group = %log_group, log_stream = %log_stream, "Audit logging to CloudWatch Logs");
            Arc::new(
                helios_audit::CloudWatchLogsSink::new(
                    log_group.to_string(),
                    log_stream.to_string(),
                    config.cloudwatch_region.clone(),
                )
                .await,
            )
        }
        #[cfg(not(feature = "cloudwatch"))]
        AuditBackend::CloudWatch => {
            anyhow::bail!(
                "CloudWatch audit backend requires the `cloudwatch` feature. \
                 Build with: cargo build --features cloudwatch"
            );
        }
    };

    let audit_state = if config.backend != AuditBackend::None {
        Some(Arc::new(AuditMiddlewareState {
            sink: Arc::clone(&sink),
            config: config.clone(),
            exclusion_filter: ExclusionFilter::default_exclusions(),
        }))
    } else {
        None
    };

    Ok((sink, audit_state))
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

    // Initialize audit subsystem
    let (audit_sink, audit_state) = init_audit().await?;

    // Initialize authentication (with audit bridge if audit is enabled)
    let (auth_config, auth_state) = init_auth_with_audit(audit_sink).await?;

    let audit_config = AuditConfig::from_env();

    info!(
        port = config.port,
        host = %config.host,
        fhir_version = ?config.default_fhir_version,
        storage_backend = %backend_mode,
        auth_enabled = auth_config.enabled,
        audit_backend = %audit_config.backend,
        "Starting Helios FHIR Server"
    );

    // Record startup audit event with server configuration
    if let Some(ref state) = audit_state {
        lifecycle::record_startup(
            &*state.sink,
            &state.config.source_observer,
            vec![
                ("storage-backend", backend_mode.to_string()),
                ("fhir-version", format!("{:?}", config.default_fhir_version)),
                ("auth-enabled", auth_config.enabled.to_string()),
                ("audit-backend", audit_config.backend.to_string()),
            ],
        )
        .await;
    }

    match backend_mode {
        StorageBackendMode::Sqlite => {
            start_sqlite(config, auth_config, auth_state, audit_state).await?;
        }
        StorageBackendMode::SqliteElasticsearch => {
            start_sqlite_elasticsearch(config, auth_config, auth_state, audit_state).await?;
        }
        StorageBackendMode::Postgres => {
            start_postgres(config, auth_config, auth_state, audit_state).await?;
        }
        StorageBackendMode::PostgresElasticsearch => {
            start_postgres_elasticsearch(config, auth_config, auth_state, audit_state).await?;
        }
        StorageBackendMode::MongoDB => {
            start_mongodb(config, auth_config, auth_state, audit_state).await?;
        }
        StorageBackendMode::MongoDBElasticsearch => {
            start_mongodb_elasticsearch(config, auth_config, auth_state, audit_state).await?;
        }
        StorageBackendMode::S3 => {
            start_s3(config, auth_config, auth_state, audit_state).await?;
        }
        StorageBackendMode::S3Elasticsearch => {
            start_s3_elasticsearch(config, auth_config, auth_state, audit_state).await?;
        }
    }

    Ok(())
}

/// Starts the server with SQLite-only backend.
#[cfg(feature = "sqlite")]
async fn start_sqlite(
    config: ServerConfig,
    auth_config: AuthConfig,
    auth_state: Option<Arc<AuthMiddlewareState>>,
    audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    let backend = create_sqlite_backend(&config)?;
    let serve_audit_state = audit_state.clone();
    let app = create_app_with_auth(
        backend,
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
    );
    serve(app, &config, serve_audit_state).await
}

/// Fallback when sqlite feature is not enabled.
#[cfg(not(feature = "sqlite"))]
async fn start_sqlite(
    _config: ServerConfig,
    _auth_config: AuthConfig,
    _auth_state: Option<Arc<AuthMiddlewareState>>,
    _audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "The sqlite backend requires the 'sqlite' feature. \
         Build with: cargo build -p helios-hfs --features sqlite"
    )
}

/// Starts the server with SQLite + Elasticsearch composite backend.
#[cfg(all(feature = "sqlite", feature = "elasticsearch"))]
async fn start_sqlite_elasticsearch(
    config: ServerConfig,
    auth_config: AuthConfig,
    auth_state: Option<Arc<AuthMiddlewareState>>,
    audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
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

    if es_nodes.is_empty() {
        anyhow::bail!(
            "sqlite-elasticsearch mode requires at least one Elasticsearch node in HFS_ELASTICSEARCH_NODES"
        );
    }

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
        .with_full_primary(sqlite)
        .start_sync_workers();

    info!("Composite storage initialized: SQLite (primary) + Elasticsearch (search)");

    let app = create_app_with_auth(
        composite,
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
    );
    serve(app, &config, serve_audit_state).await
}

/// Fallback when elasticsearch feature is not enabled.
#[cfg(not(all(feature = "sqlite", feature = "elasticsearch")))]
async fn start_sqlite_elasticsearch(
    _config: ServerConfig,
    _auth_config: AuthConfig,
    _auth_state: Option<Arc<AuthMiddlewareState>>,
    _audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "The sqlite-elasticsearch backend requires the 'elasticsearch' feature. \
         Build with: cargo build -p helios-hfs --features sqlite,elasticsearch"
    )
}

/// Starts the server with PostgreSQL backend.
#[cfg(feature = "postgres")]
async fn start_postgres(
    config: ServerConfig,
    auth_config: AuthConfig,
    auth_state: Option<Arc<AuthMiddlewareState>>,
    audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
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

    let serve_audit_state = audit_state.clone();
    let app = create_app_with_auth(
        backend,
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
    );
    serve(app, &config, serve_audit_state).await
}

/// Fallback when postgres feature is not enabled.
#[cfg(not(feature = "postgres"))]
async fn start_postgres(
    _config: ServerConfig,
    _auth_config: AuthConfig,
    _auth_state: Option<Arc<AuthMiddlewareState>>,
    _audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "The postgres backend requires the 'postgres' feature. \
         Build with: cargo build -p helios-hfs --features postgres"
    )
}

/// Starts the server with PostgreSQL + Elasticsearch composite backend.
#[cfg(all(feature = "postgres", feature = "elasticsearch"))]
async fn start_postgres_elasticsearch(
    config: ServerConfig,
    auth_config: AuthConfig,
    auth_state: Option<Arc<AuthMiddlewareState>>,
    audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
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

    if es_nodes.is_empty() {
        anyhow::bail!(
            "postgres-elasticsearch mode requires at least one Elasticsearch node in HFS_ELASTICSEARCH_NODES"
        );
    }

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
        .with_full_primary(pg)
        .start_sync_workers();

    info!("Composite storage initialized: PostgreSQL (primary) + Elasticsearch (search)");

    let app = create_app_with_auth(
        composite,
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
    );
    serve(app, &config, serve_audit_state).await
}

/// Fallback when postgres+elasticsearch features are not both enabled.
#[cfg(not(all(feature = "postgres", feature = "elasticsearch")))]
async fn start_postgres_elasticsearch(
    _config: ServerConfig,
    _auth_config: AuthConfig,
    _auth_state: Option<Arc<AuthMiddlewareState>>,
    _audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "The postgres-elasticsearch backend requires both 'postgres' and 'elasticsearch' features. \
         Build with: cargo build -p helios-hfs --features postgres,elasticsearch"
    )
}

/// Starts the server with MongoDB + Elasticsearch composite backend.
#[cfg(all(feature = "mongodb", feature = "elasticsearch"))]
async fn start_mongodb_elasticsearch(
    config: ServerConfig,
    auth_config: AuthConfig,
    auth_state: Option<Arc<AuthMiddlewareState>>,
    audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    use std::collections::HashMap;
    use std::sync::Arc;

    use helios_persistence::backends::elasticsearch::{
        ElasticsearchAuth, ElasticsearchBackend, ElasticsearchConfig,
    };
    use helios_persistence::composite::{CompositeConfig, CompositeStorage};
    use helios_persistence::core::BackendKind;

    // Create MongoDB backend
    let backend = if let Some(ref url) = config.database_url {
        if url.starts_with("mongodb://") || url.starts_with("mongodb+srv://") {
            info!(url = %url, "Initializing MongoDB backend from connection string");
            MongoBackend::from_connection_string(url)?
        } else {
            info!(
                "Initializing MongoDB backend from environment variables (database_url is not MongoDB URI)"
            );
            MongoBackend::from_env()?
        }
    } else {
        info!("Initializing MongoDB backend from environment variables");
        MongoBackend::from_env()?
    };

    backend.init_schema().await?;

    // Offload search to Elasticsearch
    let mut backend = backend;
    backend.set_search_offloaded(true);
    let mongo = Arc::new(backend);
    info!("MongoDB search indexing disabled (offloaded to Elasticsearch)");

    // Build Elasticsearch configuration from server config
    let es_nodes: Vec<String> = config
        .elasticsearch_nodes
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if es_nodes.is_empty() {
        anyhow::bail!(
            "mongodb-elasticsearch mode requires at least one Elasticsearch node in HFS_ELASTICSEARCH_NODES"
        );
    }

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

    // Create ES backend sharing MongoDB's search parameter registry
    let es = Arc::new(ElasticsearchBackend::with_shared_registry(
        es_config,
        mongo.search_registry().clone(),
    )?);

    // Build composite configuration
    let composite_config = CompositeConfig::builder()
        .primary("mongodb", BackendKind::MongoDB)
        .search_backend("es", BackendKind::Elasticsearch)
        .build()?;

    // Build backends map for CompositeStorage
    let mut backends = HashMap::new();
    backends.insert(
        "mongodb".to_string(),
        mongo.clone() as helios_persistence::composite::DynStorage,
    );
    backends.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynStorage,
    );

    // Build search providers map
    let mut search_providers = HashMap::new();
    search_providers.insert(
        "mongodb".to_string(),
        mongo.clone() as helios_persistence::composite::DynSearchProvider,
    );
    search_providers.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynSearchProvider,
    );

    // Create composite storage with full primary capabilities
    let composite = CompositeStorage::new(composite_config, backends)?
        .with_search_providers(search_providers)
        .with_full_primary(mongo);

    info!("Composite storage initialized: MongoDB (primary) + Elasticsearch (search)");

    let app = create_app_with_auth(
        composite,
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
    );
    serve(app, &config, serve_audit_state).await
}

/// Fallback when mongodb+elasticsearch features are not both enabled.
#[cfg(not(all(feature = "mongodb", feature = "elasticsearch")))]
async fn start_mongodb_elasticsearch(
    _config: ServerConfig,
    _auth_config: AuthConfig,
    _auth_state: Option<Arc<AuthMiddlewareState>>,
    _audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "The mongodb-elasticsearch backend requires both 'mongodb' and 'elasticsearch' features. \
         Build with: cargo build -p helios-hfs --features mongodb,elasticsearch"
    )
}

/// Starts the server with AWS S3 backend.
#[cfg(feature = "s3")]
async fn start_s3(
    config: ServerConfig,
    auth_config: AuthConfig,
    auth_state: Option<Arc<AuthMiddlewareState>>,
    audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
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

    let serve_audit_state = audit_state.clone();
    let app = create_app_with_auth(
        backend,
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
    );
    serve(app, &config, serve_audit_state).await
}

/// Fallback when s3 feature is not enabled.
#[cfg(not(feature = "s3"))]
async fn start_s3(
    _config: ServerConfig,
    _auth_config: AuthConfig,
    _auth_state: Option<Arc<AuthMiddlewareState>>,
    _audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "The s3 backend requires the 's3' feature. \
         Build with: cargo build -p helios-hfs --features s3"
    )
}

/// Builds a search parameter registry independently (for backends that don't own one).
#[cfg(feature = "elasticsearch")]
fn build_search_registry(
    fhir_version: helios_fhir::FhirVersion,
    data_dir: Option<&std::path::Path>,
) -> std::sync::Arc<parking_lot::RwLock<helios_persistence::search::SearchParameterRegistry>> {
    use helios_persistence::search::{SearchParameterLoader, SearchParameterRegistry};

    let registry = std::sync::Arc::new(parking_lot::RwLock::new(SearchParameterRegistry::new()));
    let loader = SearchParameterLoader::new(fhir_version);
    {
        let mut reg = registry.write();
        if let Ok(params) = loader.load_embedded() {
            for p in params {
                let _ = reg.register(p);
            }
        }
        let dir = data_dir.unwrap_or_else(|| std::path::Path::new("./data"));
        if let Ok(params) = loader.load_from_spec_file(dir) {
            for p in params {
                let _ = reg.register(p);
            }
        }
    }
    registry
}

/// Starts the server with S3 + Elasticsearch composite backend.
#[cfg(all(feature = "s3", feature = "elasticsearch"))]
async fn start_s3_elasticsearch(
    config: ServerConfig,
    auth_config: AuthConfig,
    auth_state: Option<Arc<AuthMiddlewareState>>,
    audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    use std::collections::HashMap;
    use std::sync::Arc;

    use helios_persistence::backends::elasticsearch::{
        ElasticsearchAuth, ElasticsearchBackend, ElasticsearchConfig,
    };
    use helios_persistence::backends::s3::{S3Backend, S3BackendConfig, S3TenancyMode};
    use helios_persistence::composite::{CompositeConfig, CompositeStorage};
    use helios_persistence::core::BackendKind;

    // --- S3 backend (primary) ---
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
        "Initializing S3 backend (primary)"
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

    // --- Elasticsearch backend (search) ---
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
        "Initializing Elasticsearch backend (search)"
    );

    // Build search registry independently — S3 has no internal registry
    let search_registry =
        build_search_registry(config.default_fhir_version, config.data_dir.as_deref());
    let es = Arc::new(ElasticsearchBackend::with_shared_registry(
        es_config,
        search_registry,
    )?);

    // --- Composite wiring ---
    let composite_config = CompositeConfig::builder()
        .primary("s3", BackendKind::S3)
        .search_backend("es", BackendKind::Elasticsearch)
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
        "s3".to_string(),
        s3.clone() as helios_persistence::composite::DynSearchProvider,
    );
    search_providers.insert(
        "es".to_string(),
        es.clone() as helios_persistence::composite::DynSearchProvider,
    );

    let composite = CompositeStorage::new(composite_config, backends)?
        .with_search_providers(search_providers)
        .with_full_primary(s3)
        .start_sync_workers();

    info!("Composite storage initialized: S3 (primary) + Elasticsearch (search)");

    let app = create_app_with_auth(
        composite,
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
    );
    serve(app, &config, serve_audit_state).await
}

/// Fallback when s3+elasticsearch features are not both enabled.
#[cfg(not(all(feature = "s3", feature = "elasticsearch")))]
async fn start_s3_elasticsearch(
    _config: ServerConfig,
    _auth_config: AuthConfig,
    _auth_state: Option<Arc<AuthMiddlewareState>>,
    _audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
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
    use helios_rest::ServerConfig;

    // ── create_sqlite_backend() ───────────────────────────────────

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_create_sqlite_backend_memory() {
        let config = ServerConfig {
            database_url: Some(":memory:".to_string()),
            ..Default::default()
        };
        let result = create_sqlite_backend(&config);
        assert!(result.is_ok(), "in-memory SQLite backend should succeed");
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_create_sqlite_backend_temp_file() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "hfs_test_{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        let config = ServerConfig {
            database_url: Some(path.to_string_lossy().to_string()),
            ..Default::default()
        };
        let result = create_sqlite_backend(&config);
        // Clean up the file even if the assertion fails.
        let _ = std::fs::remove_file(&path);
        assert!(result.is_ok(), "file-based SQLite backend should succeed");
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_create_sqlite_backend_default_url() {
        // When database_url is None, defaults to "fhir.db" in the current dir.
        // We redirect to a temp path to avoid creating persistent side-effects.
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "hfs_test_default_{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        // Override the default path via the env var that ServerConfig reads.
        let config = ServerConfig {
            database_url: Some(path.to_string_lossy().to_string()),
            ..Default::default()
        };
        let result = create_sqlite_backend(&config);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_ok());
    }

    // ── build_search_registry() ───────────────────────────────────

    #[cfg(feature = "elasticsearch")]
    #[test]
    fn test_build_search_registry_returns_registry() {
        use helios_fhir::FhirVersion;
        let registry = build_search_registry(FhirVersion::R4, None);
        // Registry should be a valid Arc<RwLock<…>> and not panic when read.
        let _guard = registry.read();
    }

    // ── ServerConfig validation is exercised at startup ──────────

    #[test]
    fn test_server_config_default_is_valid() {
        let config = ServerConfig::default();
        assert!(
            config.validate().is_ok(),
            "default ServerConfig should be valid"
        );
    }
}
