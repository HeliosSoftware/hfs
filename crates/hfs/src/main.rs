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
use helios_auth::{AuthConfig, JwksBearerAuthProvider, JwksCache};
use helios_persistence::{BackendKind, ResourceStorage, TenantContext};
use helios_rest::{AuthMiddlewareState, ServerConfig, StorageBackendMode};
use tracing::{info, warn};

#[cfg(any(feature = "sqlite", feature = "postgres"))]
use helios_persistence::backends::local_fs::LocalFsOutputStore;
use helios_persistence::core::SettingsStore;
// Bulk export/submit is only wired on backends that can host (or sidecar) job
// state; the imports are unused in standalone MongoDB/S3-only builds.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
use helios_persistence::core::{
    BulkExportJobStore, BulkSubmitJobStore, DefaultExportWorker, DefaultSubmitWorker,
    ExportOutputStore, SubmitInputFetcher, WorkerId,
};
#[cfg(any(feature = "sqlite", feature = "postgres"))]
use helios_rest::bulk_export_auth::BearerScopeAuth;
// Every startup path goes through one builder. The bundles it takes are all
// optional, so a backend that lacks a capability passes `None` rather than
// reaching for a different entry point: S3 has no search index to reindex,
// MongoDB rides an embedded SQLite sidecar for bulk-export job state, and so on.
// Every standalone primary backend (SQLite, PostgreSQL, MongoDB, and now S3)
// *does* host the per-user settings store, so all of them wire one.
use helios_rest::OperationsBundle;
use helios_rest::create_app_with_auth_bulk_settings_and_ops;

use helios_persistence::core::PurgableStorage;
// Only the reindex-capable ops bundles use this; the S3-only build has no
// reindex path.
#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "elasticsearch"
))]
use helios_persistence::search::ReindexOperation;

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

/// Resolves the composite sync mode from `HFS_COMPOSITE_SYNC_MODE`.
///
/// Composite stores route writes to a primary backend (e.g. SQLite, Postgres,
/// MongoDB, S3) and a search backend (Elasticsearch). `Asynchronous` (the
/// default) acks the write as soon as the primary commits and forwards the
/// search-index write on a background worker — lowest latency, but a
/// follow-up search can race the indexing. `Synchronous` blocks the write
/// until the search backend has indexed it, giving read-your-write
/// semantics at the cost of extra latency on each write.
#[cfg(feature = "elasticsearch")]
fn composite_sync_mode_from_env() -> helios_persistence::composite::SyncMode {
    use helios_persistence::composite::SyncMode;
    match std::env::var("HFS_COMPOSITE_SYNC_MODE") {
        Ok(v) => match v.trim().to_ascii_lowercase().as_str() {
            "synchronous" | "sync" => SyncMode::Synchronous,
            "asynchronous" | "async" => SyncMode::Asynchronous,
            "hybrid" => SyncMode::Hybrid {
                sync_for_search: true,
            },
            other => {
                tracing::warn!(
                    value = other,
                    "Unknown HFS_COMPOSITE_SYNC_MODE; defaulting to asynchronous"
                );
                SyncMode::Asynchronous
            }
        },
        Err(_) => SyncMode::Asynchronous,
    }
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
    // Bounds how long an operation waits for a usable server. `connect_timeout_ms`
    // only bounds a TCP handshake, so this is what actually decides how quickly an
    // unreachable MongoDB surfaces an error.
    let server_selection_timeout_ms = env("HFS_MONGODB_SERVER_SELECTION_TIMEOUT_MS")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(15_000);

    MongoBackendConfig {
        connection_string,
        database_name,
        max_connections,
        connect_timeout_ms,
        server_selection_timeout_ms,
        fhir_version: config.default_fhir_version,
        data_dir: config.data_dir.clone(),
        search_offloaded,
    }
}

#[cfg(feature = "sqlite")]
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
        let server_selection_timeout_ms = std::env::var("HFS_MONGODB_SERVER_SELECTION_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(15_000);

        let config = MongoBackendConfig {
            connection_string,
            database_name,
            max_connections,
            connect_timeout_ms,
            server_selection_timeout_ms,
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
    seed_conformance_resources(&*backend, &config).await;
    spawn_mongodb_search_param_refresh(backend.clone(), &config);
    let serve_audit_state = audit_state.clone();

    // MongoDB is a full standalone primary, so it also hosts the per-user
    // settings store: it keeps ownership of the backend Arc and wires the
    // settings-capable builder (like the SQLite/Postgres backends).
    let settings_store: Option<Arc<dyn SettingsStore>> = Some(backend.clone());
    let ui_settings = settings_store.clone();

    // MongoDB primary; embedded SQLite sidecar for bulk-export job state.
    let export_bundle = {
        #[cfg(feature = "sqlite")]
        {
            let jobs = build_embedded_job_store(&config)?;
            build_bulk_export(&config, backend.clone(), jobs).await?
        }
        #[cfg(not(feature = "sqlite"))]
        {
            None
        }
    };

    let ops = standalone_ops(
        backend.clone(),
        backend.tenant_registries().clone(),
        audit_state.as_ref(),
    );
    let app = create_app_with_auth_bulk_settings_and_ops(
        backend.clone(),
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
        export_bundle,
        None,
        settings_store,
        ops,
    );
    // Second handle to the same backend for the web UI's tenant-maintenance
    // read/write path (the FHIR app keeps its own).
    serve(app, &config, serve_audit_state, Some(backend), ui_settings).await
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
    ui_tenants: Option<Arc<dyn ResourceStorage>>,
    ui_settings: Option<Arc<dyn SettingsStore>>,
) -> anyhow::Result<()> {
    #[cfg(all(feature = "ui", not(feature = "headless")))]
    let app = {
        // The UI reads SearchParameter/CompartmentDefinition from the server's
        // own FHIR API over HTTP. It calls itself on the loopback address, with
        // the configured outbound service token (HFS_OUTBOUND_BEARER_TOKEN) when
        // set, or no credentials when auth is disabled.
        //
        // TODO(service-token): when auth is enabled, this relies on an operator
        // provisioning a valid, non-expiring bearer via HFS_OUTBOUND_BEARER_TOKEN;
        // without one the self-call is rejected and the conformance pages degrade
        // to a warning. The follow-up is to mint a short-lived, auto-refreshed
        // `system/SearchParameter.rs system/CompartmentDefinition.rs` token via
        // the planned `JwtAssertionOutboundAuthProvider` (SMART Backend Services
        // client_credentials + private_key_jwt; see crates/auth/src/outbound.rs)
        // configured from HFS_UI_* client credentials.
        let self_base_url = format!("http://127.0.0.1:{}", config.port);
        let outbound_auth = AuthConfig::from_env().outbound_provider();
        helios_ui::mount(
            app,
            env!("CARGO_PKG_VERSION"),
            config.data_dir.clone(),
            helios_ui::NlSearch {
                enabled: config.nl_search_enabled,
                configured: config.nl_search_api_key.is_some(),
                model: config.nl_search_model.clone(),
            },
            ui_tenants.clone(),
            ui_settings.clone(),
            self_base_url,
            outbound_auth,
            config.default_fhir_version,
        )
    };
    #[cfg(not(all(feature = "ui", not(feature = "headless"))))]
    let _ = (&ui_tenants, &ui_settings);

    let addr = config.socket_addr();
    info!(address = %addr, "Server listening");

    // Observability: expose `/metrics` (outside the auth layer, so scrapers
    // need no token) and instrument every request with metrics + a trace span.
    let app = app
        .merge(helios_observability::metrics::router())
        .layer(axum::middleware::from_fn(
            helios_observability::middleware::track,
        ));

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    // Peer address in request extensions: the natural-language search rate
    // limiter falls back to it when auth is disabled and there is no principal
    // to bill a request to.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = tokio::signal::ctrl_c().await;
        info!("Shutdown signal received, draining connections");
        if let Some(state) = audit_state {
            lifecycle::record_shutdown(&*state.sink, &state.config.source_observer).await;
            state.sink.flush().await;
        }
        // Flush any buffered OTLP spans (no-op without the `otel` feature).
        helios_observability::telemetry::shutdown();
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
    tenant_url_routing: bool,
) -> anyhow::Result<(AuthConfig, Option<Arc<AuthMiddlewareState>>)> {
    let auth_config = AuthConfig::from_env();

    if !auth_config.enabled {
        info!("Authentication is DISABLED");
        return Ok((auth_config, None));
    }

    // Every invariant of an enabled auth config now lives on the type, so an
    // embedder that builds one directly gets the same guarantees this binary
    // does. Issuer validation in particular is required, both to prevent
    // cross-service token reuse and because `iss` qualifies every per-user
    // identity (see `helios_rest::extractors::UserKey`).
    if let Err(errors) = auth_config.validate() {
        anyhow::bail!("Invalid auth configuration:\n  - {}", errors.join("\n  - "));
    }

    let jwks_url = auth_config
        .jwks_url
        .as_ref()
        .expect("validate() guarantees a JWKS URL when auth is enabled");

    // Audience stays optional so an open demo deployment can accept any token
    // from its issuer, but that also means a token minted for a *different*
    // client of the same issuer is accepted here. Make it noisy rather than silent.
    if auth_config.expected_audience.is_none() {
        warn!(
            "HFS_AUTH_AUDIENCE is not set: every token from this issuer will be accepted, \
             including tokens minted for other clients of the same issuer. \
             Set HFS_AUTH_AUDIENCE to restrict tokens to this server."
        );
    }

    // Create JWKS cache
    let jwks_cache = Arc::new(JwksCache::new(
        jwks_url,
        auth_config.jwks_min_refresh_interval,
    ));
    jwks_cache.initial_fetch().await?;

    // Create auth provider
    let provider = JwksBearerAuthProvider::new(jwks_cache, &auth_config);

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
        tenant_url_routing,
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
    helios_observability::uptime::init();
    helios_observability::telemetry::init("hfs", &config.log_level);
    helios_observability::metrics::init("hfs");
    // hfs is the one server that mounts the console traffic/tenants endpoints
    // backed by the reqlog ring buffer, so it opts into recording. Servers that
    // don't (hts, sof-server, fhirpath-server) leave it off and skip the cost.
    helios_observability::reqlog::enable();

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
    let (auth_config, auth_state) = init_auth_with_audit(
        audit_sink,
        config.multitenancy.routing_mode.supports_url_path(),
    )
    .await?;

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

/// Seeds storage with the spec SearchParameters (#235) and CompartmentDefinitions
/// (#237/#238), making primary storage the source of truth the FHIR routes and
/// web UI read. Seeds every provisioned tenant — auto-provisioning the default
/// tenant first — so `GET /SearchParameter` and `GET /CompartmentDefinition` are
/// populated for each valid tenant. A failed seed logs and boots anyway: the
/// in-memory registry still resolves searches; only API discovery is degraded.
///
/// Standalone backends seed themselves; Elasticsearch composites seed through
/// the composite so the writes also reach the search index. Standalone S3 is
/// the one deployment that skips seeding — it has no search index at all.
#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "elasticsearch"
))]
async fn seed_conformance_resources<S>(backend: &S, config: &ServerConfig)
where
    S: helios_persistence::core::ResourceStorage,
{
    if !config.seed_conformance {
        return;
    }

    let data_dir = config
        .data_dir
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("./data"));

    for tenant_id in provisioned_tenants(backend, config).await {
        helios_persistence::search::seed_tenant_conformance(
            backend,
            config.default_fhir_version,
            &data_dir,
            &tenant_id,
        )
        .await;
    }
}

/// The set of tenants to seed: the auto-provisioned default tenant plus every
/// registered tenant. Tenants are provisioned-only, so this is the complete set
/// of valid tenants. Falls back to just the default tenant when the backend has
/// no tenant registry (e.g. a minimal deployment).
#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "elasticsearch"
))]
async fn provisioned_tenants<S>(backend: &S, config: &ServerConfig) -> Vec<String>
where
    S: helios_persistence::core::ResourceStorage,
{
    let default = config.default_tenant.clone();
    if !backend.supports_tenant_registry() {
        return vec![default];
    }

    // Auto-provision the default tenant so it is a valid, enumerable tenant
    // (single-tenant and unauthenticated deployments read it).
    match backend.get_tenant(&default).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            if let Err(e) = backend.register_tenant(&default, None).await {
                tracing::warn!(tenant = %default, "Auto-provisioning default tenant failed: {e}");
            }
        }
        Err(e) => tracing::warn!(tenant = %default, "Checking default tenant failed: {e}"),
    }

    let mut ids: Vec<String> = match backend.list_tenants().await {
        Ok(records) => records.into_iter().map(|r| r.id).collect(),
        Err(e) => {
            tracing::warn!("Listing tenants for seeding failed: {e}");
            Vec::new()
        }
    };
    if !ids.iter().any(|id| id == &default) {
        ids.push(default);
    }
    ids
}

/// Spawns the periodic registry refresh from storage for the SQLite backend
/// (#235). `HFS_SEARCH_PARAM_CACHE_TTL=0` disables it. A failed pass keeps
/// serving the stale cache; the next tick retries.
#[cfg(feature = "sqlite")]
fn spawn_sqlite_search_param_refresh(backend: Arc<SqliteBackend>, config: &ServerConfig) {
    let ttl = config.search_param_cache_ttl;
    if ttl == 0 {
        return;
    }
    let interval = std::time::Duration::from_secs(ttl);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            let refresh = backend.clone();
            match tokio::task::spawn_blocking(move || refresh.refresh_stored_search_parameters())
                .await
            {
                Ok(Ok(stored)) => {
                    tracing::debug!(stored, "SearchParameter registry refreshed from storage")
                }
                Ok(Err(e)) => tracing::warn!(
                    "SearchParameter registry refresh failed; serving the stale cache: {e}"
                ),
                Err(e) => tracing::warn!("SearchParameter registry refresh task failed: {e}"),
            }
        }
    });
}

/// Postgres flavor of the periodic registry refresh (#235).
#[cfg(feature = "postgres")]
fn spawn_postgres_search_param_refresh(
    backend: Arc<helios_persistence::backends::postgres::PostgresBackend>,
    config: &ServerConfig,
) {
    let ttl = config.search_param_cache_ttl;
    if ttl == 0 {
        return;
    }
    let interval = std::time::Duration::from_secs(ttl);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            match backend.refresh_stored_search_parameters().await {
                Ok(stored) => {
                    tracing::debug!(stored, "SearchParameter registry refreshed from storage")
                }
                Err(e) => tracing::warn!(
                    "SearchParameter registry refresh failed; serving the stale cache: {e}"
                ),
            }
        }
    });
}

/// MongoDB flavor of the periodic registry refresh (#235).
#[cfg(feature = "mongodb")]
fn spawn_mongodb_search_param_refresh(backend: Arc<MongoBackend>, config: &ServerConfig) {
    let ttl = config.search_param_cache_ttl;
    if ttl == 0 {
        return;
    }
    let interval = std::time::Duration::from_secs(ttl);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            match backend.refresh_stored_search_parameters().await {
                Ok(stored) => {
                    tracing::debug!(stored, "SearchParameter registry refreshed from storage")
                }
                Err(e) => tracing::warn!(
                    "SearchParameter registry refresh failed; serving the stale cache: {e}"
                ),
            }
        }
    });
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
    seed_conformance_resources(&*backend, &config).await;
    spawn_sqlite_search_param_refresh(backend.clone(), &config);
    // Second handle to the same backend for the web UI's tenant-maintenance
    // read/write path (the FHIR app keeps its own). Cheap: the SQLite backend
    // shares one connection pool behind the Arc.
    let ui_tenants: Option<Arc<dyn ResourceStorage>> = Some(backend.clone());

    // The SQLite backend also hosts the per-user settings store, so it always
    // keeps ownership of the backend Arc and uses the settings-capable builder.
    let settings_store: Option<Arc<dyn SettingsStore>> = Some(backend.clone());
    let ui_settings = settings_store.clone();
    let export_bundle = build_bulk_export(&config, backend.clone(), backend.clone()).await?;
    let submit_bundle = build_bulk_submit(&config, backend.clone()).await?;
    let ops = standalone_ops(
        backend.clone(),
        backend.tenant_registries().clone(),
        audit_state.as_ref(),
    );
    let app = create_app_with_auth_bulk_settings_and_ops(
        backend,
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
        export_bundle,
        submit_bundle,
        settings_store,
        ops,
    );
    serve(app, &config, serve_audit_state, ui_tenants, ui_settings).await
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
///
/// Only compiled for backends that can't host job state themselves and so need
/// the sidecar (MongoDB, S3); pure-SQLite/Postgres deployments never call it.
#[cfg(all(
    feature = "sqlite",
    any(feature = "mongodb", all(feature = "s3", feature = "elasticsearch"))
))]
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

/// Attaches the audit sink to a reindex driver and hands back a shared handle.
/// Used by both the standalone and composite ops bundles; the S3-only build has
/// no reindex target and so never calls it.
#[cfg(any(
    feature = "sqlite",
    feature = "postgres",
    feature = "mongodb",
    feature = "elasticsearch"
))]
fn wire_reindex(
    op: ReindexOperation,
    audit_state: Option<&Arc<AuditMiddlewareState>>,
) -> Arc<ReindexOperation> {
    let op = match audit_state {
        Some(state) => op.with_audit(
            Arc::clone(&state.sink),
            state.config.source_observer.clone(),
        ),
        None => op,
    };
    Arc::new(op)
}

/// Ops bundle for a backend that indexes itself — the standalone deployments
/// (SQLite, PostgreSQL, MongoDB), where resources and search index share a home.
#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mongodb"))]
fn standalone_ops<B>(
    backend: Arc<B>,
    registries: Arc<helios_persistence::search::TenantSearchRegistries>,
    audit_state: Option<&Arc<AuditMiddlewareState>>,
) -> OperationsBundle
where
    B: PurgableStorage + helios_persistence::search::ReindexableStorage + 'static,
{
    OperationsBundle {
        purge: Some(backend.clone() as Arc<dyn PurgableStorage>),
        reindex: Some(wire_reindex(
            ReindexOperation::new(backend, registries),
            audit_state,
        )),
    }
}

/// Ops bundle for a composite deployment.
///
/// `purge` is the **composite**, not the primary: purging the primary alone
/// would leave the resource in the Elasticsearch index, still searchable and
/// still holding its content.
///
/// `reindex` reads from the primary and writes to every index in `targets` —
/// which must include the Elasticsearch secondary, since that is what actually
/// serves search here. Rebuilding only the primary's index would leave search
/// untouched by `$reindex`.
///
/// Composite deployments always pair a primary with the Elasticsearch
/// secondary, so this is only reachable when `elasticsearch` is enabled.
#[cfg(feature = "elasticsearch")]
fn composite_ops(
    composite: Arc<helios_persistence::composite::CompositeStorage>,
    source: Arc<dyn helios_persistence::search::ReindexSource>,
    targets: Vec<Arc<dyn helios_persistence::search::ReindexTarget>>,
    registries: Arc<helios_persistence::search::TenantSearchRegistries>,
    audit_state: Option<&Arc<AuditMiddlewareState>>,
) -> OperationsBundle {
    OperationsBundle {
        purge: Some(composite as Arc<dyn PurgableStorage>),
        reindex: Some(wire_reindex(
            ReindexOperation::with_parts(source, targets, registries),
            audit_state,
        )),
    }
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

/// Builds the bulk-submit subsystem (input fetcher + output store + file auth +
/// worker pool) from a caller-supplied job store. Returns `None` when bulk submit
/// is disabled. The job store is the same backend instance that holds the FHIR
/// resources (so ingestion writes go to the primary store).
#[cfg(any(feature = "sqlite", feature = "postgres"))]
async fn build_bulk_submit(
    config: &ServerConfig,
    jobs: Arc<dyn BulkSubmitJobStore>,
) -> anyhow::Result<Option<helios_rest::BulkSubmitBundle>> {
    let cfg = config.bulk_submit.clone();
    info!(
        "Bulk submit config: enabled={} output_backend={} requires_access_token={}",
        cfg.enabled, cfg.output_backend, cfg.requires_access_token
    );
    if !cfg.enabled {
        return Ok(None);
    }

    // --- Output store (status-manifest artifacts) -----------------------
    let output: Arc<dyn ExportOutputStore> = match cfg.output_backend.as_str() {
        "local-fs" => {
            let output_dir = cfg
                .output_dir
                .clone()
                .or_else(|| {
                    config
                        .data_dir
                        .as_ref()
                        .map(|d| format!("{}/submit", d.display()))
                })
                .unwrap_or_else(|| "./data/submit".to_string());
            Arc::new(LocalFsOutputStore::new(output_dir, config.base_url.clone()))
        }
        "s3" => {
            #[cfg(feature = "s3")]
            {
                use helios_persistence::backends::s3::{
                    AccessTokenMode, AwsS3Client, AwsS3ClientOptions, S3OutputStore,
                };
                let bucket = cfg.s3_bucket.clone().ok_or_else(|| {
                    anyhow::anyhow!("HFS_BULK_SUBMIT_S3_BUCKET is required for OUTPUT_BACKEND=s3")
                })?;
                let region = std::env::var("HFS_BULK_SUBMIT_S3_REGION")
                    .ok()
                    .or_else(|| std::env::var("HFS_S3_REGION").ok());
                let sdk_config = AwsS3Client::load_sdk_config(region.as_deref()).await;
                let client = Arc::new(AwsS3Client::from_sdk_config_with_options(
                    &sdk_config,
                    AwsS3ClientOptions {
                        endpoint_url: std::env::var("HFS_BULK_SUBMIT_S3_ENDPOINT").ok(),
                        force_path_style: parse_env_bool(
                            "HFS_BULK_SUBMIT_S3_FORCE_PATH_STYLE",
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
                    "HFS_BULK_SUBMIT_OUTPUT_BACKEND=s3 requires the 's3' feature. \
                     Build with: cargo build -p helios-hfs --features s3"
                );
            }
        }
        other => anyhow::bail!("invalid HFS_BULK_SUBMIT_OUTPUT_BACKEND '{other}'"),
    };

    // --- Remote input fetcher (+ optional outbound SMART client) --------
    // When a client_id + private key are configured, protected-file fetches use
    // a read-scoped `client_credentials` token; otherwise they surface a recorded
    // manifest-level error.
    let token_provider: Option<Arc<dyn helios_persistence::core::FileTokenProvider>> =
        match (cfg.client_id.as_deref(), cfg.private_key.as_deref()) {
            (Some(client_id), Some(pem)) => {
                match helios_rest::bulk_submit_oauth::JwtClientCredentialsTokenProvider::new(
                    client_id,
                    pem,
                    &cfg.signing_alg,
                ) {
                    Some(p) => Some(p),
                    None => anyhow::bail!(
                        "HFS_BULK_SUBMIT_PRIVATE_KEY could not be parsed as a {} PEM key",
                        cfg.signing_alg
                    ),
                }
            }
            _ => None,
        };
    let fetcher: Arc<dyn SubmitInputFetcher> = Arc::new(
        helios_rest::bulk_submit_fetcher::HttpSubmitInputFetcher::new(
            token_provider,
            cfg.outbound_scope.clone(),
        ),
    );

    spawn_submit_workers(jobs.clone(), fetcher.clone(), output.clone(), &cfg);

    Ok(Some(helios_rest::BulkSubmitBundle {
        jobs,
        fetcher,
        output,
        file_auth: Arc::new(BearerScopeAuth),
    }))
}

/// Spawns the in-process submit worker pool and the periodic cleanup task.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn spawn_submit_workers(
    jobs: Arc<dyn BulkSubmitJobStore>,
    fetcher: Arc<dyn SubmitInputFetcher>,
    output: Arc<dyn ExportOutputStore>,
    cfg: &helios_rest::config::BulkSubmitConfig,
) {
    if cfg.disable_local_worker {
        info!("Bulk submit in-process worker pool is disabled");
        return;
    }
    let lease = std::time::Duration::from_secs(cfg.lease_duration_secs);
    for i in 0..cfg.worker_concurrency {
        let jobs = jobs.clone();
        let fetcher = fetcher.clone();
        let output = output.clone();
        let worker_id = WorkerId::new(format!("hfs-submit-worker-{i}"));
        tokio::spawn(async move {
            let worker = DefaultSubmitWorker::new(jobs.clone(), fetcher, output, worker_id.clone());
            loop {
                match jobs.claim_next_manifest(&worker_id, lease).await {
                    Ok(Some(claimed)) => {
                        if let Err(e) = worker.run_job(claimed).await {
                            tracing::error!("submit worker job failed: {e}");
                        }
                    }
                    Ok(None) => {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                    Err(e) => {
                        tracing::error!("submit worker claim failed: {e}");
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }
            }
        });
    }
    // Periodic cleanup of expired submission artifacts.
    let cleanup_jobs = jobs.clone();
    let cleanup_output = output.clone();
    let interval = std::time::Duration::from_secs(cfg.cleanup_interval_secs);
    let output_ttl = std::time::Duration::from_secs(cfg.output_ttl_secs);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            match cleanup_jobs
                .list_expired_submissions(chrono::Utc::now(), output_ttl, 100)
                .await
            {
                Ok(expired) => {
                    for (tenant, sub_id) in expired {
                        let job_id = helios_persistence::core::submission_output_job_id(&sub_id);
                        let _ = cleanup_output.delete_job_outputs(&tenant, &job_id).await;
                        let _ = cleanup_jobs
                            .delete_submission_artifacts(&tenant, &sub_id)
                            .await;
                        let _ = cleanup_jobs.clear_poll_token(&tenant, &sub_id).await;
                    }
                }
                Err(e) => tracing::error!("submit cleanup scan failed: {e}"),
            }
        }
    });

    info!(
        "Bulk submit worker pool started ({} workers)",
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
    // Refresh reads from the primary; the ES backend shares its registry Arc.
    // Seeding waits for the composite below, so the writes also index into ES.
    spawn_sqlite_search_param_refresh(sqlite.clone(), &config);

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
        sqlite.tenant_registries().clone(),
    )?);

    // Build composite configuration
    let composite_config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .search_backend("es", BackendKind::Elasticsearch)
        .sync_mode(composite_sync_mode_from_env())
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
        // `$purge` must reach the Elasticsearch secondary too — purging only
        // the SQLite primary would leave the resource in the search index,
        // still searchable and still holding its content.
        .with_purgable_backends(
            sqlite.clone() as Arc<dyn PurgableStorage>,
            vec![("es".to_string(), es.clone() as Arc<dyn PurgableStorage>)],
        )
        .start_sync_workers();

    info!("Composite storage initialized: SQLite (primary) + Elasticsearch (search)");

    let serve_audit_state = audit_state.clone();
    let composite = Arc::new(composite);

    // Seed through the composite: the primary's own indexing is offloaded, so
    // seeding it directly would leave the conformance resources unsearchable
    // (empty /SearchParameter and /CompartmentDefinition, and empty UI viewers).
    seed_conformance_resources(&*composite, &config).await;

    // The per-user settings store lives on the SQLite primary (Elasticsearch is
    // search-only), so it is wired from the underlying `sqlite` backend even
    // though the app is served over the composite storage.
    let settings_store: Option<Arc<dyn SettingsStore>> = Some(sqlite.clone());
    let ui_settings = settings_store.clone();

    let export_bundle = build_bulk_export(&config, sqlite.clone(), sqlite.clone()).await?;
    let submit_bundle = build_bulk_submit(&config, sqlite.clone()).await?;
    // Reindex reads from the SQLite primary and rebuilds BOTH indexes: SQLite's
    // own search_index table and the Elasticsearch index that actually serves
    // search here.
    let ops = composite_ops(
        composite.clone(),
        sqlite.clone(),
        vec![sqlite.clone(), es.clone()],
        sqlite.tenant_registries().clone(),
        audit_state.as_ref(),
    );
    let app = create_app_with_auth_bulk_settings_and_ops(
        composite.clone(),
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
        export_bundle,
        submit_bundle,
        settings_store,
        ops,
    );
    // The UI's tenant-maintenance path goes through the composite (not the
    // bare primary) so a purge also clears the offloaded search documents.
    serve(
        app,
        &config,
        serve_audit_state,
        Some(composite),
        ui_settings,
    )
    .await
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
    seed_conformance_resources(&*backend, &config).await;
    spawn_postgres_search_param_refresh(backend.clone(), &config);

    let serve_audit_state = audit_state.clone();
    // The PostgreSQL backend also hosts the per-user settings store, so it always
    // keeps ownership of the backend Arc and uses the settings-capable builder.
    let settings_store: Option<Arc<dyn SettingsStore>> = Some(backend.clone());
    let ui_settings = settings_store.clone();
    let export_bundle = build_bulk_export(&config, backend.clone(), backend.clone()).await?;
    let submit_bundle = build_bulk_submit(&config, backend.clone()).await?;
    let ops = standalone_ops(
        backend.clone(),
        backend.tenant_registries().clone(),
        audit_state.as_ref(),
    );
    let app = create_app_with_auth_bulk_settings_and_ops(
        backend.clone(),
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
        export_bundle,
        submit_bundle,
        settings_store,
        ops,
    );
    // Second handle to the same backend for the web UI's tenant-maintenance
    // read/write path (the FHIR app keeps its own).
    serve(app, &config, serve_audit_state, Some(backend), ui_settings).await
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
    // Refresh reads from the primary; the ES backend shares its registry Arc.
    // Seeding waits for the composite below, so the writes also index into ES.
    spawn_postgres_search_param_refresh(pg.clone(), &config);

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
        pg.tenant_registries().clone(),
    )?);

    // Build composite configuration
    let composite_config = CompositeConfig::builder()
        .primary("postgres", BackendKind::Postgres)
        .search_backend("es", BackendKind::Elasticsearch)
        .sync_mode(composite_sync_mode_from_env())
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
        // See `start_sqlite_elasticsearch`: `$purge` must reach the search
        // secondary, not just the primary.
        .with_purgable_backends(
            pg.clone() as Arc<dyn PurgableStorage>,
            vec![("es".to_string(), es.clone() as Arc<dyn PurgableStorage>)],
        )
        .start_sync_workers();

    info!("Composite storage initialized: PostgreSQL (primary) + Elasticsearch (search)");

    let serve_audit_state = audit_state.clone();
    let composite = Arc::new(composite);

    // Seed through the composite: the primary's own indexing is offloaded, so
    // seeding it directly would leave the conformance resources unsearchable.
    seed_conformance_resources(&*composite, &config).await;

    // The per-user settings store lives on the PostgreSQL primary (Elasticsearch
    // is search-only), so it is wired from the underlying `pg` backend even
    // though the app is served over the composite storage.
    let settings_store: Option<Arc<dyn SettingsStore>> = Some(pg.clone());
    let ui_settings = settings_store.clone();

    let export_bundle = build_bulk_export(&config, pg.clone(), pg.clone()).await?;
    let submit_bundle = build_bulk_submit(&config, pg.clone()).await?;
    let ops = composite_ops(
        composite.clone(),
        pg.clone(),
        vec![pg.clone(), es.clone()],
        pg.tenant_registries().clone(),
        audit_state.as_ref(),
    );
    let app = create_app_with_auth_bulk_settings_and_ops(
        composite.clone(),
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
        export_bundle,
        submit_bundle,
        settings_store,
        ops,
    );
    // The UI's tenant-maintenance path goes through the composite (not the
    // bare primary) so a purge also clears the offloaded search documents.
    serve(
        app,
        &config,
        serve_audit_state,
        Some(composite),
        ui_settings,
    )
    .await
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
    // Refresh reads from the primary; the ES backend shares its registry Arc.
    // Seeding waits for the composite below, so the writes also index into ES.
    spawn_mongodb_search_param_refresh(mongo.clone(), &config);

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
        mongo.tenant_registries().clone(),
    )?);

    // Build composite configuration
    let composite_config = CompositeConfig::builder()
        .primary("mongodb", BackendKind::MongoDB)
        .search_backend("es", BackendKind::Elasticsearch)
        .sync_mode(composite_sync_mode_from_env())
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
        // See `start_sqlite_elasticsearch`: `$purge` must reach the search
        // secondary, not just the primary.
        .with_purgable_backends(
            mongo.clone() as Arc<dyn PurgableStorage>,
            vec![("es".to_string(), es.clone() as Arc<dyn PurgableStorage>)],
        )
        .start_sync_workers();

    info!("Composite storage initialized: MongoDB (primary) + Elasticsearch (search)");

    let serve_audit_state = audit_state.clone();
    let composite = Arc::new(composite);

    // Seed through the composite: the primary's own indexing is offloaded, so
    // seeding it directly would leave the conformance resources unsearchable.
    seed_conformance_resources(&*composite, &config).await;

    // The per-user settings store lives on the MongoDB primary (Elasticsearch is
    // search-only), so it is wired from the underlying `mongo` backend even
    // though the app is served over the composite storage.
    let settings_store: Option<Arc<dyn SettingsStore>> = Some(mongo.clone());
    let ui_settings = settings_store.clone();

    // MongoDB primary; embedded SQLite sidecar for bulk-export job state.
    let export_bundle = {
        #[cfg(feature = "sqlite")]
        {
            let jobs = build_embedded_job_store(&config)?;
            build_bulk_export(&config, mongo.clone(), jobs).await?
        }
        #[cfg(not(feature = "sqlite"))]
        {
            None
        }
    };

    let ops = composite_ops(
        composite.clone(),
        mongo.clone(),
        vec![mongo.clone(), es.clone()],
        mongo.tenant_registries().clone(),
        audit_state.as_ref(),
    );
    let app = create_app_with_auth_bulk_settings_and_ops(
        composite.clone(),
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
        export_bundle,
        None,
        settings_store,
        ops,
    );
    // The UI's tenant-maintenance path goes through the composite (not the
    // bare primary) so a purge also clears the offloaded search documents.
    serve(
        app,
        &config,
        serve_audit_state,
        Some(composite),
        ui_settings,
    )
    .await
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
    let endpoint_url = std::env::var("HFS_S3_ENDPOINT")
        .ok()
        .filter(|s| !s.is_empty());
    let force_path_style = parse_env_bool("HFS_S3_FORCE_PATH_STYLE", false);
    let allow_http = parse_env_bool("HFS_S3_ALLOW_HTTP", true);
    let validate_buckets = std::env::var("HFS_S3_VALIDATE_BUCKETS")
        .map(|s| s.to_lowercase() != "false" && s != "0")
        .unwrap_or(true);

    info!(
        bucket = %bucket,
        region = ?region,
        prefix = ?prefix,
        endpoint_url = ?endpoint_url,
        validate_buckets = validate_buckets,
        "Initializing S3 backend"
    );

    let s3_config = S3BackendConfig {
        tenancy_mode: S3TenancyMode::PrefixPerTenant {
            bucket: bucket.clone(),
        },
        region,
        prefix,
        endpoint_url,
        force_path_style,
        allow_http,
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

    let backend = Arc::new(backend);
    let serve_audit_state = audit_state.clone();

    // Second handle to the same backend (S3Backend clones share the client)
    // for the web UI's tenant-maintenance read/write path.
    let ui_tenants: Option<Arc<dyn ResourceStorage>> =
        Some(backend.clone() as Arc<dyn ResourceStorage>);

    // The S3 backend also hosts the per-user settings store (a compare-and-swap
    // over conditional PutObject). Bulk export/submit are not wired on a
    // standalone S3 primary, which has no job store.
    //
    // A bucket-per-tenant configuration with no `default_system_bucket` has
    // nowhere tenant-independent to keep a user-global document, so the store is
    // left unwired and `/_user/settings` reports the explained 501 rather than
    // failing every request.
    let settings_store: Option<Arc<dyn SettingsStore>> = if backend.supports_user_settings() {
        Some(backend.clone())
    } else {
        tracing::warn!(
            "S3 is configured bucket-per-tenant with no default system bucket; \
             per-user settings (/_user/settings) will report 501 Not Implemented"
        );
        None
    };
    let ui_settings = settings_store.clone();

    // S3 standalone can purge, but it has NO search index of any kind — its
    // SearchProvider reports search unsupported — so `$reindex` has nothing to
    // rebuild and the handler reports 501 rather than accepting a job that
    // would do nothing. This is the one deployment where a 501 is the honest
    // answer.
    let ops = OperationsBundle {
        purge: Some(backend.clone() as Arc<dyn PurgableStorage>),
        reindex: None,
    };

    let app = create_app_with_auth_bulk_settings_and_ops(
        backend,
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
        None,
        None,
        settings_store,
        ops,
    );
    serve(app, &config, serve_audit_state, ui_tenants, ui_settings).await
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
///
/// Only the composite S3 + Elasticsearch starter needs this; the other composite
/// starters get their registry from the primary backend.
#[cfg(all(feature = "s3", feature = "elasticsearch"))]
fn build_search_registry(
    fhir_version: helios_fhir::FhirVersion,
    data_dir: Option<&std::path::Path>,
) -> std::sync::Arc<helios_persistence::search::TenantSearchRegistries> {
    use helios_persistence::search::{SearchParameterLoader, TenantSearchRegistries};

    // S3 stores no SearchParameter resources of its own, so tenants have no
    // stored overlay — every tenant sees the shared base (embedded + spec).
    let registries = std::sync::Arc::new(TenantSearchRegistries::base_only());
    let loader = SearchParameterLoader::new(fhir_version);
    {
        let mut reg = registries.base().write();
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
    registries
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
    let endpoint_url = std::env::var("HFS_S3_ENDPOINT")
        .ok()
        .filter(|s| !s.is_empty());
    let force_path_style = parse_env_bool("HFS_S3_FORCE_PATH_STYLE", false);
    let allow_http = parse_env_bool("HFS_S3_ALLOW_HTTP", true);
    let validate_buckets = std::env::var("HFS_S3_VALIDATE_BUCKETS")
        .map(|s| s.to_lowercase() != "false" && s != "0")
        .unwrap_or(true);

    info!(
        bucket = %bucket,
        region = ?region,
        prefix = ?prefix,
        endpoint_url = ?endpoint_url,
        validate_buckets = validate_buckets,
        "Initializing S3 backend (primary)"
    );

    let s3_config = S3BackendConfig {
        tenancy_mode: S3TenancyMode::PrefixPerTenant {
            bucket: bucket.clone(),
        },
        region,
        prefix,
        endpoint_url,
        force_path_style,
        allow_http,
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
        .sync_mode(composite_sync_mode_from_env())
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
        // See `start_sqlite_elasticsearch`: `$purge` must reach the search
        // secondary, not just the primary.
        .with_purgable_backends(
            s3.clone() as Arc<dyn PurgableStorage>,
            vec![("es".to_string(), es.clone() as Arc<dyn PurgableStorage>)],
        )
        .start_sync_workers();

    info!("Composite storage initialized: S3 (primary) + Elasticsearch (search)");

    let serve_audit_state = audit_state.clone();
    let composite = Arc::new(composite);

    // Seed through the composite so the conformance resources land in the S3
    // primary and get indexed into Elasticsearch — the only search index here.
    seed_conformance_resources(&*composite, &config).await;

    // The per-user settings store lives on the S3 primary (Elasticsearch is
    // search-only), so it is wired from the underlying `s3` backend even though
    // the app is served over the composite storage. As in `start_s3`, a tenancy
    // mode with no tenant-independent bucket leaves it unwired (explained 501).
    let settings_store: Option<Arc<dyn SettingsStore>> = if s3.supports_user_settings() {
        Some(s3.clone())
    } else {
        tracing::warn!(
            "S3 is configured bucket-per-tenant with no default system bucket; \
             per-user settings (/_user/settings) will report 501 Not Implemented"
        );
        None
    };
    let ui_settings = settings_store.clone();

    // Reindex reads from the S3 primary and writes to Elasticsearch, which is
    // the only search index in this deployment — S3 maintains none. The
    // extractor is Elasticsearch's for the same reason.
    let ops = composite_ops(
        composite.clone(),
        s3.clone(),
        vec![es.clone()],
        es.tenant_registries().clone(),
        audit_state.as_ref(),
    );

    // S3 primary; embedded SQLite sidecar for bulk-export job state. A single
    // builder call covers both the bulk-enabled and bulk-disabled cases, so
    // `ops` and `settings_store` are each consumed exactly once.
    let bulk_export = {
        #[cfg(feature = "sqlite")]
        {
            let jobs = build_embedded_job_store(&config)?;
            build_bulk_export(&config, s3.clone(), jobs).await?
        }
        #[cfg(not(feature = "sqlite"))]
        {
            None
        }
    };

    let app = create_app_with_auth_bulk_settings_and_ops(
        composite.clone(),
        config.clone(),
        auth_config,
        auth_state,
        audit_state,
        bulk_export,
        None,
        settings_store,
        ops,
    );
    // The UI's tenant-maintenance path goes through the composite (not the bare
    // primary) so a purge also clears the offloaded search documents.
    serve(
        app,
        &config,
        serve_audit_state,
        Some(composite),
        ui_settings,
    )
    .await
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

    #[cfg(all(feature = "s3", feature = "elasticsearch"))]
    #[test]
    fn test_build_search_registry_returns_registry() {
        use helios_fhir::FhirVersion;
        let registries = build_search_registry(FhirVersion::R4, None);
        // The container's base should be populated and every tenant resolves.
        assert!(!registries.base().read().is_empty());
        let _guard = registries.for_tenant("default");
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
            "HFS_MONGODB_SERVER_SELECTION_TIMEOUT_MS" => Some("2500".to_string()),
            _ => None,
        });

        assert_eq!(
            mongo_config.connection_string,
            "mongodb://mongo.example:27017/?replicaSet=rs0"
        );
        assert_eq!(mongo_config.database_name, "inferno_suite");
        assert_eq!(mongo_config.max_connections, 24);
        assert_eq!(mongo_config.connect_timeout_ms, 7500);
        assert_eq!(mongo_config.server_selection_timeout_ms, 2500);
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

    #[cfg(feature = "sqlite")]
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
