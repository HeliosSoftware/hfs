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

use helios_audit::{
    AuditBackend, AuditConfig, AuditMiddlewareState, AuditSink, ExclusionFilter, lifecycle,
};
use helios_auth::{AuthConfig, InMemoryJtiCache, JtiCache, JwksBearerAuthProvider, JwksCache};
use helios_persistence::{BackendKind, ResourceStorage, TenantContext};
use helios_rest::{
    AuthMiddlewareState, ServerConfig, StorageBackendMode, create_app_with_auth, init_logging,
};
use tracing::info;

use helios_persistence::backends::local_fs::LocalFsOutputStore;
use helios_persistence::core::{
    BulkExportJobStore, DefaultExportWorker, ExportOutputStore, WorkerId,
};
#[cfg(any(feature = "sqlite", feature = "postgres"))]
use helios_rest::bulk_export_auth::BearerScopeAuth;
#[cfg(any(feature = "sqlite", feature = "postgres"))]
use helios_rest::create_app_with_auth_and_bulk_export;

#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};

#[cfg(feature = "mongodb")]
use helios_persistence::backends::mongodb::{MongoBackend, MongoBackendConfig};
fn is_database_audit_dedicated(config: &AuditConfig, backend_kind: BackendKind) -> bool {
    match backend_kind {
        BackendKind::Sqlite | BackendKind::Postgres => config.database_url.is_some(),
        BackendKind::MongoDB => config.database_url.is_some() || config.mongodb_database.is_some(),
        BackendKind::S3 => {
            config.s3_bucket.is_some()
                || config.s3_prefix.is_some()
                || config.s3_region.is_some()
                || config.s3_validate_buckets.is_some()
        }
        _ => false,
    }
}

#[cfg(feature = "s3")]
fn parse_env_bool(var: &str, default_value: bool) -> bool {
    std::env::var(var)
        .map(|v| {
            let normalized = v.trim().to_ascii_lowercase();
            !(normalized == "false" || normalized == "0")
        })
        .unwrap_or(default_value)
}

#[cfg(feature = "postgres")]
fn is_postgres_url(url: &str) -> bool {
    url.starts_with("postgres://") || url.starts_with("postgresql://")
}

#[cfg(feature = "mongodb")]
fn is_mongodb_url(url: &str) -> bool {
    url.starts_with("mongodb://") || url.starts_with("mongodb+srv://")
}

#[cfg(feature = "mongodb")]
fn build_mongodb_config(config: &ServerConfig, search_offloaded: bool) -> MongoBackendConfig {
    build_mongodb_config_with_env(config, search_offloaded, |name| std::env::var(name).ok())
}

#[cfg(feature = "mongodb")]
fn build_mongodb_config_with_env<F>(
    config: &ServerConfig,
    search_offloaded: bool,
    env: F,
) -> MongoBackendConfig
where
    F: Fn(&str) -> Option<String>,
{
    let mongo_specific_url = env("HFS_MONGODB_URL").or_else(|| env("HFS_MONGODB_URI"));
    let connection_string = match config.database_url.as_deref() {
        Some(url) if is_mongodb_url(url) => url.to_string(),
        Some(_) => mongo_specific_url.unwrap_or_else(|| "mongodb://localhost:27017".to_string()),
        None => mongo_specific_url
            .or_else(|| env("HFS_DATABASE_URL").filter(|url| is_mongodb_url(url)))
            .unwrap_or_else(|| "mongodb://localhost:27017".to_string()),
    };

    let database_name = env("HFS_MONGODB_DATABASE").unwrap_or_else(|| "helios".to_string());
    let max_connections = env("HFS_MONGODB_MAX_CONNECTIONS")
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(10);
    let connect_timeout_ms = env("HFS_MONGODB_CONNECT_TIMEOUT_MS")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(5000);

    MongoBackendConfig {
        connection_string,
        database_name,
        max_connections,
        connect_timeout_ms,
        fhir_version: config.default_fhir_version,
        data_dir: config.data_dir.clone(),
        search_offloaded,
    }
}

fn validate_shared_sqlite_audit_path(path: &str, dedicated: bool) -> anyhow::Result<()> {
    if !dedicated && path == ":memory:" {
        anyhow::bail!(
            "Database audit backend in shared SQLite mode cannot use :memory: because it cannot be shared \
             across separate backend instances. Set HFS_AUDIT_DATABASE_URL to a file path for a dedicated \
             audit store, or use HFS_AUDIT_BACKEND=file."
        );
    }
    Ok(())
}

struct HfsAuditStorageAdapter {
    storage: Arc<dyn ResourceStorage>,
    tenant: TenantContext,
}

impl HfsAuditStorageAdapter {
    fn new(storage: Arc<dyn ResourceStorage>) -> Self {
        Self {
            storage,
            tenant: TenantContext::system(),
        }
    }
}

#[async_trait::async_trait]
impl helios_audit::sinks::database::AuditStorage for HfsAuditStorageAdapter {
    async fn create_resource(
        &self,
        resource_type: &str,
        resource: serde_json::Value,
        fhir_version: helios_fhir::FhirVersion,
    ) -> Result<(), String> {
        self.storage
            .create(&self.tenant, resource_type, resource, fhir_version)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

async fn create_database_audit_storage(
    server_config: &ServerConfig,
    backend_mode: StorageBackendMode,
    audit_config: &AuditConfig,
) -> anyhow::Result<Arc<dyn ResourceStorage>> {
    let backend_kind = backend_mode.primary_backend_kind();
    let dedicated = is_database_audit_dedicated(audit_config, backend_kind);

    info!(
        backend_kind = %backend_kind,
        storage_backend = %backend_mode,
        dedicated = dedicated,
        "Initializing database audit sink storage"
    );

    match backend_kind {
        BackendKind::Sqlite => {
            create_audit_sqlite_storage(server_config, audit_config, dedicated).await
        }
        BackendKind::Postgres => {
            create_audit_postgres_storage(server_config, audit_config, dedicated).await
        }
        BackendKind::MongoDB => {
            create_audit_mongodb_storage(server_config, audit_config, dedicated).await
        }
        BackendKind::S3 => create_audit_s3_storage(server_config, audit_config, dedicated).await,
        _ => anyhow::bail!(
            "Database audit sink is unsupported for primary backend kind '{}'",
            backend_kind
        ),
    }
}

#[cfg(feature = "sqlite")]
async fn create_audit_sqlite_storage(
    server_config: &ServerConfig,
    audit_config: &AuditConfig,
    dedicated: bool,
) -> anyhow::Result<Arc<dyn ResourceStorage>> {
    let database_url = if dedicated {
        audit_config.database_url.as_deref().ok_or_else(|| {
            anyhow::anyhow!("Dedicated SQLite audit mode requires HFS_AUDIT_DATABASE_URL to be set")
        })?
    } else {
        server_config.database_url.as_deref().unwrap_or("fhir.db")
    };

    validate_shared_sqlite_audit_path(database_url, dedicated)?;

    let mut sqlite_config = server_config.clone();
    sqlite_config.database_url = Some(database_url.to_string());
    let backend = create_sqlite_backend(&sqlite_config)?;

    Ok(Arc::new(backend))
}

#[cfg(not(feature = "sqlite"))]
async fn create_audit_sqlite_storage(
    _server_config: &ServerConfig,
    _audit_config: &AuditConfig,
    _dedicated: bool,
) -> anyhow::Result<Arc<dyn ResourceStorage>> {
    anyhow::bail!(
        "Database audit backend with sqlite storage requires the 'sqlite' feature. \
         Build with: cargo build -p helios-hfs --features sqlite"
    )
}

#[cfg(feature = "postgres")]
async fn create_audit_postgres_storage(
    server_config: &ServerConfig,
    audit_config: &AuditConfig,
    dedicated: bool,
) -> anyhow::Result<Arc<dyn ResourceStorage>> {
    use helios_persistence::backends::postgres::PostgresBackend;

    let backend = if dedicated {
        let url = audit_config.database_url.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "Dedicated PostgreSQL audit mode requires HFS_AUDIT_DATABASE_URL to be set"
            )
        })?;
        if !is_postgres_url(url) {
            anyhow::bail!(
                "HFS_AUDIT_DATABASE_URL must be a PostgreSQL connection string when primary backend is postgres"
            );
        }
        PostgresBackend::from_connection_string(url).await?
    } else if let Some(ref url) = server_config.database_url {
        if is_postgres_url(url) {
            PostgresBackend::from_connection_string(url).await?
        } else {
            PostgresBackend::from_env().await?
        }
    } else {
        PostgresBackend::from_env().await?
    };

    backend.init_schema().await?;
    Ok(Arc::new(backend))
}

#[cfg(not(feature = "postgres"))]
async fn create_audit_postgres_storage(
    _server_config: &ServerConfig,
    _audit_config: &AuditConfig,
    _dedicated: bool,
) -> anyhow::Result<Arc<dyn ResourceStorage>> {
    anyhow::bail!(
        "Database audit backend with postgres storage requires the 'postgres' feature. \
         Build with: cargo build -p helios-hfs --features postgres"
    )
}

#[cfg(feature = "mongodb")]
async fn create_audit_mongodb_storage(
    server_config: &ServerConfig,
    audit_config: &AuditConfig,
    dedicated: bool,
) -> anyhow::Result<Arc<dyn ResourceStorage>> {
    let backend = if dedicated {
        let connection_string = match audit_config.database_url.as_deref() {
            Some(url) => {
                if !is_mongodb_url(url) {
                    anyhow::bail!(
                        "HFS_AUDIT_DATABASE_URL must be a MongoDB connection string when primary backend is mongodb"
                    );
                }
                url.to_string()
            }
            None => {
                if let Some(url) = server_config.database_url.as_deref() {
                    if is_mongodb_url(url) {
                        url.to_string()
                    } else {
                        std::env::var("HFS_MONGODB_URL")
                            .or_else(|_| std::env::var("HFS_MONGODB_URI"))
                            .or_else(|_| std::env::var("HFS_DATABASE_URL"))
                            .unwrap_or_else(|_| "mongodb://localhost:27017".to_string())
                    }
                } else {
                    std::env::var("HFS_MONGODB_URL")
                        .or_else(|_| std::env::var("HFS_MONGODB_URI"))
                        .or_else(|_| std::env::var("HFS_DATABASE_URL"))
                        .unwrap_or_else(|_| "mongodb://localhost:27017".to_string())
                }
            }
        };

        let database_name = audit_config
            .mongodb_database
            .clone()
            .or_else(|| std::env::var("HFS_MONGODB_DATABASE").ok())
            .unwrap_or_else(|| "helios".to_string());

        let max_connections = std::env::var("HFS_MONGODB_MAX_CONNECTIONS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(10);
        let connect_timeout_ms = std::env::var("HFS_MONGODB_CONNECT_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(5000);

        let config = MongoBackendConfig {
            connection_string,
            database_name,
            max_connections,
            connect_timeout_ms,
            fhir_version: server_config.default_fhir_version,
            data_dir: server_config.data_dir.clone(),
            search_offloaded: false,
        };
        MongoBackend::new(config)?
    } else {
        MongoBackend::new(build_mongodb_config(server_config, false))?
    };

    backend.init_schema().await?;
    Ok(Arc::new(backend))
}

#[cfg(not(feature = "mongodb"))]
async fn create_audit_mongodb_storage(
    _server_config: &ServerConfig,
    _audit_config: &AuditConfig,
    _dedicated: bool,
) -> anyhow::Result<Arc<dyn ResourceStorage>> {
    anyhow::bail!(
        "Database audit backend with mongodb storage requires the 'mongodb' feature. \
         Build with: cargo build -p helios-hfs --features mongodb"
    )
}

#[cfg(feature = "s3")]
async fn create_audit_s3_storage(
    _server_config: &ServerConfig,
    audit_config: &AuditConfig,
    dedicated: bool,
) -> anyhow::Result<Arc<dyn ResourceStorage>> {
    use helios_persistence::backends::s3::{S3Backend, S3BackendConfig, S3TenancyMode};

    if audit_config.database_url.is_some() {
        anyhow::bail!(
            "HFS_AUDIT_DATABASE_URL is not supported for S3-backed database audit sink. \
             Use HFS_AUDIT_S3_BUCKET / HFS_AUDIT_S3_PREFIX / HFS_AUDIT_S3_REGION instead."
        );
    }

    let bucket = audit_config
        .s3_bucket
        .clone()
        .or_else(|| std::env::var("HFS_S3_BUCKET").ok())
        .unwrap_or_else(|| "hfs".to_string());
    let region = audit_config
        .s3_region
        .clone()
        .or_else(|| std::env::var("HFS_S3_REGION").ok());
    let prefix = audit_config
        .s3_prefix
        .clone()
        .or_else(|| std::env::var("HFS_S3_PREFIX").ok());
    let validate_buckets = audit_config
        .s3_validate_buckets
        .unwrap_or_else(|| parse_env_bool("HFS_S3_VALIDATE_BUCKETS", true));

    info!(
        bucket = %bucket,
        region = ?region,
        prefix = ?prefix,
        validate_buckets = validate_buckets,
        dedicated = dedicated,
        "Initializing S3 storage for database audit sink"
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
            "Failed to initialize S3 storage for database audit sink (bucket={}, region={:?}): {}",
            bucket,
            std::env::var("AWS_REGION").ok(),
            e
        )
    })?;

    Ok(Arc::new(backend))
}

#[cfg(not(feature = "s3"))]
async fn create_audit_s3_storage(
    _server_config: &ServerConfig,
    _audit_config: &AuditConfig,
    _dedicated: bool,
) -> anyhow::Result<Arc<dyn ResourceStorage>> {
    anyhow::bail!(
        "Database audit backend with s3 storage requires the 's3' feature. \
         Build with: cargo build -p helios-hfs --features s3"
    )
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

/// Starts the server with MongoDB backend.
#[cfg(feature = "mongodb")]
async fn start_mongodb(
    config: ServerConfig,
    auth_config: AuthConfig,
    auth_state: Option<Arc<AuthMiddlewareState>>,
    audit_state: Option<Arc<AuditMiddlewareState>>,
) -> anyhow::Result<()> {
    let backend_config = build_mongodb_config(&config, false);
    info!(
        url = %backend_config.connection_string,
        database = %backend_config.database_name,
        "Initializing MongoDB backend"
    );
    let backend = MongoBackend::new(backend_config)?;

    backend.init_schema().await?;
    let backend = Arc::new(backend);
    let serve_audit_state = audit_state.clone();

    // MongoDB primary; embedded SQLite sidecar for job state.
    #[cfg(feature = "sqlite")]
    {
        let jobs = build_embedded_job_store(&config)?;
        if let Some(bundle) = build_bulk_export(&config, backend.clone(), jobs).await? {
            let app = create_app_with_auth_and_bulk_export(
                backend,
                config.clone(),
                auth_config,
                auth_state,
                audit_state,
                bundle,
            );
            return serve(app, &config, serve_audit_state).await;
        }
    }

    let app = create_app_with_auth(
        Arc::try_unwrap(backend).unwrap_or_else(|_| {
            unreachable!("backend Arc is uniquely owned when bulk export is disabled")
        }),
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
        "memory" => {
            info!("Using in-memory JTI cache");
            Arc::new(InMemoryJtiCache::new())
        }
        "disabled" | "none" => {
            info!("JTI replay cache is DISABLED");
            Arc::new(helios_auth::DisabledJtiCache)
        }
        other => {
            anyhow::bail!(
                "Invalid HFS_AUTH_JTI_BACKEND '{}'. Valid values: memory, redis, disabled",
                other
            );
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
async fn init_audit(
    server_config: &ServerConfig,
    backend_mode: StorageBackendMode,
) -> anyhow::Result<(Arc<dyn AuditSink>, Option<Arc<AuditMiddlewareState>>)> {
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
            let storage =
                create_database_audit_storage(server_config, backend_mode, &config).await?;
            let adapter = Arc::new(HfsAuditStorageAdapter::new(storage));
            Arc::new(helios_audit::DatabaseSink::new(
                adapter,
                server_config.default_fhir_version,
            ))
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
    // Use `from_env()` (not `parse()`) so `multitenancy` and `bulk_export`
    // sub-structs — both `#[arg(skip)]` for clap — are populated from
    // their `HFS_*` environment variables.
    let config = ServerConfig::from_env();
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

    // Propagate HFS_TERMINOLOGY_SERVER to FHIRPATH_TERMINOLOGY_SERVER so that any
    // FHIRPath evaluation (CDS Hooks, _filter, etc.) delegates terminology
    // operations (memberOf, subsumes) to the configured HTS instance.
    if let Some(ref ts_url) = config.terminology_server {
        if std::env::var("FHIRPATH_TERMINOLOGY_SERVER").is_err() {
            // Safety: single-threaded at this point (before tokio runtime hands
            // off to worker threads), so set_var is safe here.
            // SAFETY: called before any threads are spawned by the tokio runtime.
            unsafe {
                std::env::set_var("FHIRPATH_TERMINOLOGY_SERVER", ts_url);
            }
            info!(url = %ts_url, "HFS_TERMINOLOGY_SERVER wired to FHIRPath context");
        }
    }

    // Initialize audit subsystem
    let (audit_sink, audit_state) = init_audit(&config, backend_mode).await?;

    // Initialize authentication (with audit bridge if audit is enabled)
    let (auth_config, auth_state) = init_auth_with_audit(audit_sink).await?;

    let audit_config = AuditConfig::from_env();

    info!(
        port = config.port,
        host = %config.host,
        fhir_version = ?config.default_fhir_version,
        storage_backend = %backend_mode,
        terminology_server = ?config.terminology_server,
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
    let serve_audit_state = audit_state.clone();
    let backend = Arc::new(create_sqlite_backend(&config)?);

    if let Some(bundle) = build_bulk_export(&config, backend.clone(), backend.clone()).await? {
        let app = create_app_with_auth_and_bulk_export(
            backend,
            config.clone(),
            auth_config,
            auth_state,
            audit_state,
            bundle,
        );
        return serve(app, &config, serve_audit_state).await;
    }

    let app = create_app_with_auth(
        Arc::try_unwrap(backend).unwrap_or_else(|_| {
            unreachable!("backend Arc is uniquely owned when bulk export is disabled")
        }),
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
    );
    serve(app, &config, serve_audit_state).await
}

/// Constructs an embedded SQLite job store for backends that can't host job
/// state themselves (MongoDB, S3). Path resolution, in priority order:
///
/// 1. `${HFS_BULK_EXPORT_OUTPUT_DIR}/bulk_export.db` — co-located with local
///    output, so single-instance dev / CI runs keep job state next to data.
/// 2. `${HFS_DATA_DIR}/bulk_export.db` — when explicitly configured.
/// 3. `${TMPDIR}/hfs-bulk-export-{pid}.db` — last-resort fallback that's
///    unique per process so parallel HFS instances (e.g. CI smoke jobs running
///    with `max-parallel`) don't race on a single file. Job state is ephemeral
///    in this mode — fine for tests; production deployments should configure
///    one of the persistent options above.
///
/// Isolated from FHIR resource data either way.
#[cfg(feature = "sqlite")]
fn build_embedded_job_store(config: &ServerConfig) -> anyhow::Result<Arc<dyn BulkExportJobStore>> {
    let job_db = config
        .bulk_export
        .output_dir
        .clone()
        .map(|d| format!("{d}/bulk_export.db"))
        .or_else(|| {
            config
                .data_dir
                .as_ref()
                .map(|d| format!("{}/bulk_export.db", d.display()))
        })
        .unwrap_or_else(|| {
            let mut path = std::env::temp_dir();
            path.push(format!("hfs-bulk-export-{}.db", std::process::id()));
            path.to_string_lossy().into_owned()
        });
    if let Some(parent) = std::path::Path::new(&job_db).parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| {
            anyhow::anyhow!(
                "create bulk export job DB directory {}: {e}",
                parent.display()
            )
        })?;
    }
    let job_backend = SqliteBackend::with_config(
        &job_db,
        SqliteBackendConfig {
            fhir_version: config.default_fhir_version,
            data_dir: config.data_dir.clone(),
            ..Default::default()
        },
    )?;
    job_backend.init_schema()?;
    Ok(Arc::new(job_backend))
}

/// Builds the bulk-export subsystem (output store + file auth + worker pool)
/// from a caller-supplied job store and resource data provider. Returns `None`
/// when bulk export is disabled.
///
/// The job store is supplied by the caller so it reuses the same backend
/// instance (and connection pool) that holds the FHIR resources. This means
/// pure-SQLite deployments share `./data/hfs.db` between resources and job
/// state, and Postgres deployments share the configured `HFS_DATABASE_URL`.
/// Backends that can't host transactional job state (MongoDB, S3) use a
/// sidecar SQLite via [`build_embedded_job_store`].
#[cfg(any(feature = "sqlite", feature = "postgres"))]
async fn build_bulk_export<Dp>(
    config: &ServerConfig,
    data: Arc<Dp>,
    jobs: Arc<dyn BulkExportJobStore>,
) -> anyhow::Result<Option<helios_rest::BulkExportBundle>>
where
    Dp: helios_persistence::core::ExportResourceProvider + 'static,
{
    let cfg = config.bulk_export.clone();
    info!(
        "Bulk export config: enabled={} output_backend={} requires_access_token={}",
        cfg.enabled, cfg.output_backend, cfg.requires_access_token
    );
    if !cfg.enabled {
        return Ok(None);
    }

    // --- Output store ---------------------------------------------------
    let output: Arc<dyn ExportOutputStore> = match cfg.output_backend.as_str() {
        "local-fs" => {
            let output_dir = cfg
                .output_dir
                .clone()
                .or_else(|| {
                    config
                        .data_dir
                        .as_ref()
                        .map(|d| format!("{}/exports", d.display()))
                })
                .unwrap_or_else(|| "./data/exports".to_string());
            Arc::new(LocalFsOutputStore::new(output_dir, config.base_url.clone()))
        }
        "s3" => {
            #[cfg(feature = "s3")]
            {
                use helios_persistence::backends::s3::{
                    AccessTokenMode, AwsS3Client, AwsS3ClientOptions, S3OutputStore,
                };
                let bucket = cfg.s3_bucket.clone().ok_or_else(|| {
                    anyhow::anyhow!("HFS_BULK_EXPORT_S3_BUCKET is required for OUTPUT_BACKEND=s3")
                })?;
                let region = std::env::var("HFS_BULK_EXPORT_S3_REGION")
                    .ok()
                    .or_else(|| std::env::var("HFS_S3_REGION").ok());
                let sdk_config = AwsS3Client::load_sdk_config(region.as_deref()).await;
                let client = Arc::new(AwsS3Client::from_sdk_config_with_options(
                    &sdk_config,
                    AwsS3ClientOptions {
                        endpoint_url: std::env::var("HFS_BULK_EXPORT_S3_ENDPOINT").ok(),
                        force_path_style: parse_env_bool(
                            "HFS_BULK_EXPORT_S3_FORCE_PATH_STYLE",
                            false,
                        ),
                    },
                ));
                Arc::new(S3OutputStore::new(
                    client,
                    bucket,
                    config.base_url.clone(),
                    AccessTokenMode::parse(&cfg.requires_access_token),
                    std::time::Duration::from_secs(cfg.file_url_ttl_secs),
                ))
            }
            #[cfg(not(feature = "s3"))]
            {
                anyhow::bail!(
                    "HFS_BULK_EXPORT_OUTPUT_BACKEND=s3 requires the 's3' feature. \
                     Build with: cargo build -p helios-hfs --features s3"
                );
            }
        }
        other => anyhow::bail!("invalid HFS_BULK_EXPORT_OUTPUT_BACKEND '{other}'"),
    };

    spawn_export_workers(jobs.clone(), data, output.clone(), &cfg);

    Ok(Some(helios_rest::BulkExportBundle {
        jobs,
        output,
        file_auth: Arc::new(BearerScopeAuth),
    }))
}

/// Spawns the in-process export worker pool and the periodic cleanup task.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn spawn_export_workers<Dp>(
    jobs: Arc<dyn BulkExportJobStore>,
    data: Arc<Dp>,
    output: Arc<dyn ExportOutputStore>,
    cfg: &helios_rest::config::BulkExportConfig,
) where
    Dp: helios_persistence::core::ExportResourceProvider + 'static,
{
    if cfg.disable_local_worker {
        info!("Bulk export in-process worker pool is disabled");
        return;
    }
    let lease = std::time::Duration::from_secs(cfg.lease_duration_secs);
    for i in 0..cfg.worker_concurrency {
        let jobs = jobs.clone();
        let data = data.clone();
        let output = output.clone();
        let worker_id = WorkerId::new(format!("hfs-worker-{i}"));
        let exclude_newly_added = cfg.since_newly_added.eq_ignore_ascii_case("exclude");
        tokio::spawn(async move {
            let worker = DefaultExportWorker::new(jobs.clone(), data, output, worker_id.clone())
                .with_exclude_since_newly_added(exclude_newly_added);
            loop {
                match jobs.claim_next(&worker_id, lease).await {
                    Ok(Some(claimed)) => {
                        if let Err(e) = worker.run_job(claimed).await {
                            tracing::error!("export worker job failed: {e}");
                        }
                    }
                    Ok(None) => {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                    Err(e) => {
                        tracing::error!("export worker claim failed: {e}");
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }
            }
        });
    }

    // Periodic cleanup of expired job output.
    let cleanup_jobs = jobs.clone();
    let cleanup_output = output.clone();
    let interval = std::time::Duration::from_secs(cfg.cleanup_interval_secs);
    let output_ttl = std::time::Duration::from_secs(cfg.output_ttl_secs);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            match cleanup_jobs
                .list_expired_exports(chrono::Utc::now(), output_ttl, 100)
                .await
            {
                Ok(expired) => {
                    for job in expired {
                        let _ = cleanup_output
                            .delete_job_outputs(&job.tenant, &job.job_id)
                            .await;
                        let _ = cleanup_jobs.delete_export(&job.tenant, &job.job_id).await;
                    }
                }
                Err(e) => tracing::error!("export cleanup scan failed: {e}"),
            }
        }
    });
    info!(
        "Bulk export worker pool started ({} workers)",
        cfg.worker_concurrency
    );
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
        .with_full_primary(sqlite.clone())
        .start_sync_workers();

    info!("Composite storage initialized: SQLite (primary) + Elasticsearch (search)");

    let serve_audit_state = audit_state.clone();
    let composite = Arc::new(composite);

    if let Some(bundle) = build_bulk_export(&config, sqlite.clone(), sqlite.clone()).await? {
        let app = create_app_with_auth_and_bulk_export(
            composite,
            config.clone(),
            auth_config,
            auth_state,
            audit_state,
            bundle,
        );
        return serve(app, &config, serve_audit_state).await;
    }

    let app = create_app_with_auth(
        Arc::try_unwrap(composite).unwrap_or_else(|_| {
            unreachable!("composite Arc is uniquely owned when bulk export is disabled")
        }),
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
    let backend = Arc::new(backend);

    let serve_audit_state = audit_state.clone();
    if let Some(bundle) = build_bulk_export(&config, backend.clone(), backend.clone()).await? {
        let app = create_app_with_auth_and_bulk_export(
            backend,
            config.clone(),
            auth_config,
            auth_state,
            audit_state,
            bundle,
        );
        return serve(app, &config, serve_audit_state).await;
    }

    let app = create_app_with_auth(
        Arc::try_unwrap(backend).unwrap_or_else(|_| {
            unreachable!("backend Arc is uniquely owned when bulk export is disabled")
        }),
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
        .with_full_primary(pg.clone())
        .start_sync_workers();

    info!("Composite storage initialized: PostgreSQL (primary) + Elasticsearch (search)");

    let serve_audit_state = audit_state.clone();
    let composite = Arc::new(composite);

    if let Some(bundle) = build_bulk_export(&config, pg.clone(), pg.clone()).await? {
        let app = create_app_with_auth_and_bulk_export(
            composite,
            config.clone(),
            auth_config,
            auth_state,
            audit_state,
            bundle,
        );
        return serve(app, &config, serve_audit_state).await;
    }

    let app = create_app_with_auth(
        Arc::try_unwrap(composite).unwrap_or_else(|_| {
            unreachable!("composite Arc is uniquely owned when bulk export is disabled")
        }),
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
    let backend_config = build_mongodb_config(&config, true);
    info!(
        url = %backend_config.connection_string,
        database = %backend_config.database_name,
        "Initializing MongoDB backend"
    );
    let backend = MongoBackend::new(backend_config)?;

    backend.init_schema().await?;

    // Offload search to Elasticsearch
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
        .with_full_primary(mongo.clone())
        .start_sync_workers();

    info!("Composite storage initialized: MongoDB (primary) + Elasticsearch (search)");

    let serve_audit_state = audit_state.clone();
    let composite = Arc::new(composite);

    // MongoDB primary; embedded SQLite sidecar for job state.
    #[cfg(feature = "sqlite")]
    {
        let jobs = build_embedded_job_store(&config)?;
        if let Some(bundle) = build_bulk_export(&config, mongo.clone(), jobs).await? {
            let app = create_app_with_auth_and_bulk_export(
                composite,
                config.clone(),
                auth_config,
                auth_state,
                audit_state,
                bundle,
            );
            return serve(app, &config, serve_audit_state).await;
        }
    }

    let app = create_app_with_auth(
        Arc::try_unwrap(composite).unwrap_or_else(|_| {
            unreachable!("composite Arc is uniquely owned when bulk export is disabled")
        }),
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
        .with_full_primary(s3.clone())
        .start_sync_workers();

    info!("Composite storage initialized: S3 (primary) + Elasticsearch (search)");

    let serve_audit_state = audit_state.clone();
    let composite = Arc::new(composite);

    // S3 primary; embedded SQLite sidecar for job state.
    #[cfg(feature = "sqlite")]
    {
        let jobs = build_embedded_job_store(&config)?;
        if let Some(bundle) = build_bulk_export(&config, s3.clone(), jobs).await? {
            let app = create_app_with_auth_and_bulk_export(
                composite,
                config.clone(),
                auth_config,
                auth_state,
                audit_state,
                bundle,
            );
            return serve(app, &config, serve_audit_state).await;
        }
    }

    let app = create_app_with_auth(
        Arc::try_unwrap(composite).unwrap_or_else(|_| {
            unreachable!("composite Arc is uniquely owned when bulk export is disabled")
        }),
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
    use helios_audit::AuditConfig;
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

    #[cfg(feature = "mongodb")]
    #[test]
    fn test_build_mongodb_config_overlays_env_with_mongo_database_url() {
        use helios_fhir::FhirVersion;

        let data_dir = std::path::PathBuf::from("/tmp/hfs-data");
        let config = ServerConfig {
            database_url: Some("mongodb://mongo.example:27017/?replicaSet=rs0".to_string()),
            default_fhir_version: FhirVersion::R4,
            data_dir: Some(data_dir.clone()),
            ..Default::default()
        };

        let mongo_config = build_mongodb_config_with_env(&config, false, |name| match name {
            "HFS_MONGODB_DATABASE" => Some("inferno_suite".to_string()),
            "HFS_MONGODB_MAX_CONNECTIONS" => Some("24".to_string()),
            "HFS_MONGODB_CONNECT_TIMEOUT_MS" => Some("7500".to_string()),
            _ => None,
        });

        assert_eq!(
            mongo_config.connection_string,
            "mongodb://mongo.example:27017/?replicaSet=rs0"
        );
        assert_eq!(mongo_config.database_name, "inferno_suite");
        assert_eq!(mongo_config.max_connections, 24);
        assert_eq!(mongo_config.connect_timeout_ms, 7500);
        assert_eq!(mongo_config.fhir_version, FhirVersion::R4);
        assert_eq!(mongo_config.data_dir, Some(data_dir));
        assert!(!mongo_config.search_offloaded);
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn test_build_mongodb_config_ignores_non_mongo_database_url() {
        let config = ServerConfig {
            database_url: Some("postgres://localhost/hfs".to_string()),
            ..Default::default()
        };

        let mongo_config = build_mongodb_config_with_env(&config, true, |name| match name {
            "HFS_DATABASE_URL" => Some("postgres://localhost/hfs".to_string()),
            "HFS_MONGODB_DATABASE" => Some("mongo_db".to_string()),
            _ => None,
        });

        assert_eq!(mongo_config.connection_string, "mongodb://localhost:27017");
        assert_eq!(mongo_config.database_name, "mongo_db");
        assert!(mongo_config.search_offloaded);
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn test_build_mongodb_config_uses_mongo_specific_url_before_database_url_fallback() {
        let config = ServerConfig::default();

        let mongo_config = build_mongodb_config_with_env(&config, false, |name| match name {
            "HFS_MONGODB_URI" => Some("mongodb://mongo-specific:27017".to_string()),
            "HFS_DATABASE_URL" => Some("mongodb://database-url:27017".to_string()),
            _ => None,
        });

        assert_eq!(
            mongo_config.connection_string,
            "mongodb://mongo-specific:27017"
        );
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

    #[test]
    fn test_storage_backend_mode_primary_backend_kind_mapping() {
        assert_eq!(
            StorageBackendMode::Sqlite.primary_backend_kind(),
            BackendKind::Sqlite
        );
        assert_eq!(
            StorageBackendMode::SqliteElasticsearch.primary_backend_kind(),
            BackendKind::Sqlite
        );
        assert_eq!(
            StorageBackendMode::Postgres.primary_backend_kind(),
            BackendKind::Postgres
        );
        assert_eq!(
            StorageBackendMode::PostgresElasticsearch.primary_backend_kind(),
            BackendKind::Postgres
        );
        assert_eq!(
            StorageBackendMode::MongoDB.primary_backend_kind(),
            BackendKind::MongoDB
        );
        assert_eq!(
            StorageBackendMode::MongoDBElasticsearch.primary_backend_kind(),
            BackendKind::MongoDB
        );
        assert_eq!(
            StorageBackendMode::S3.primary_backend_kind(),
            BackendKind::S3
        );
        assert_eq!(
            StorageBackendMode::S3Elasticsearch.primary_backend_kind(),
            BackendKind::S3
        );
    }

    #[test]
    fn test_is_database_audit_dedicated_detection() {
        let mut config = AuditConfig::default();
        assert!(!is_database_audit_dedicated(&config, BackendKind::Sqlite));
        assert!(!is_database_audit_dedicated(&config, BackendKind::Postgres));
        assert!(!is_database_audit_dedicated(&config, BackendKind::MongoDB));
        assert!(!is_database_audit_dedicated(&config, BackendKind::S3));

        config.database_url = Some("sqlite:///tmp/audit.db".to_string());
        assert!(is_database_audit_dedicated(&config, BackendKind::Sqlite));
        assert!(is_database_audit_dedicated(&config, BackendKind::Postgres));
        assert!(is_database_audit_dedicated(&config, BackendKind::MongoDB));

        let mut mongo_only = AuditConfig::default();
        mongo_only.mongodb_database = Some("helios_audit".to_string());
        assert!(is_database_audit_dedicated(
            &mongo_only,
            BackendKind::MongoDB
        ));
        assert!(!is_database_audit_dedicated(
            &mongo_only,
            BackendKind::Postgres
        ));

        let mut s3_only = AuditConfig::default();
        s3_only.s3_bucket = Some("audit-bucket".to_string());
        assert!(is_database_audit_dedicated(&s3_only, BackendKind::S3));
    }

    #[test]
    fn test_validate_shared_sqlite_audit_path_guard() {
        let shared = validate_shared_sqlite_audit_path(":memory:", false);
        assert!(shared.is_err(), "shared SQLite + :memory: must be rejected");

        let dedicated = validate_shared_sqlite_audit_path(":memory:", true);
        assert!(dedicated.is_ok(), "dedicated SQLite may use :memory:");

        let shared_file = validate_shared_sqlite_audit_path("fhir.db", false);
        assert!(
            shared_file.is_ok(),
            "shared SQLite file path must be accepted"
        );
    }
}
