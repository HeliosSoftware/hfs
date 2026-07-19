//! PostgreSQL backend implementation.

use std::collections::HashMap;
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use deadpool_postgres::{Config, Pool, Runtime, SslMode};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio_postgres::NoTls;

use helios_fhir::FhirVersion;

use crate::core::{Backend, BackendCapability, BackendKind};
use crate::error::{BackendError, StorageResult};
use crate::search::{
    SearchParameterDefinition, SearchParameterExtractor, SearchParameterLoader,
    SearchParameterRegistry, TenantSearchRegistries,
};

/// Sync in-memory cache of each tenant's stored (POSTed) active SearchParameter
/// definitions, keyed by tenant id. Postgres queries are async but the per-tenant
/// registry loader must be sync, so async paths (startup, TTL refresh, and
/// SearchParameter writes) populate this map and the loader reads it.
type StoredByTenant = Arc<RwLock<HashMap<String, Vec<SearchParameterDefinition>>>>;

/// PostgreSQL backend for FHIR resource storage.
pub struct PostgresBackend {
    pool: Pool,
    config: PostgresConfig,
    /// Per-tenant search parameter registries (shared base + per-tenant overlay).
    registries: Arc<TenantSearchRegistries>,
    /// Sync cache of each tenant's stored params, read by the registry loader.
    stored_by_tenant: StoredByTenant,
    /// Memoized subscription event fan-out: one LISTEN connection per
    /// backend, spawned lazily by the first `subscription_fanout()` call
    /// (which must happen inside a Tokio runtime) and stopped on drop.
    subscription_fanout: std::sync::OnceLock<Arc<super::subscription_fanout::PgNotifyFanout>>,
}

impl Debug for PostgresBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresBackend")
            .field("config", &self.config)
            .field("base_registry_len", &self.registries.base().read().len())
            .finish_non_exhaustive()
    }
}

/// Configuration for the PostgreSQL backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostgresConfig {
    /// PostgreSQL host.
    #[serde(default = "default_host")]
    pub host: String,

    /// PostgreSQL port.
    #[serde(default = "default_port")]
    pub port: u16,

    /// Database name.
    #[serde(default = "default_dbname")]
    pub dbname: String,

    /// Database user.
    #[serde(default = "default_user")]
    pub user: String,

    /// Database password.
    #[serde(default)]
    pub password: Option<String>,

    /// SSL mode.
    #[serde(default)]
    pub ssl_mode: PostgresSslMode,

    /// Maximum number of connections in the pool.
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,

    /// Connection timeout in seconds.
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,

    /// Statement timeout in milliseconds.
    #[serde(default = "default_statement_timeout_ms")]
    pub statement_timeout_ms: u64,

    /// How long to wait for a free pooled connection before returning
    /// `BackendError::Unavailable`.
    #[serde(default = "default_pool_wait_timeout_secs")]
    pub pool_wait_timeout_secs: u64,

    /// FHIR version for this backend instance.
    #[serde(default = "crate::default_fhir_version")]
    pub fhir_version: FhirVersion,

    /// Directory containing FHIR SearchParameter spec files.
    #[serde(default)]
    pub data_dir: Option<PathBuf>,

    /// When true, search indexing is offloaded to a secondary backend.
    #[serde(default)]
    pub search_offloaded: bool,

    /// Optional schema name for schema-per-tenant isolation.
    #[serde(default)]
    pub schema_name: Option<String>,
}

/// SSL mode for PostgreSQL connections.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PostgresSslMode {
    /// Disable SSL.
    Disable,
    /// Prefer SSL, but allow non-SSL.
    #[default]
    Prefer,
    /// Require SSL.
    Require,
}

fn default_host() -> String {
    "localhost".to_string()
}

fn default_port() -> u16 {
    5432
}

fn default_dbname() -> String {
    "helios".to_string()
}

fn default_user() -> String {
    "helios".to_string()
}

/// Default pool size.
///
/// FHIR search is dominated by database round-trips rather than local CPU, so
/// connections spend most of their life waiting on the wire and a pool somewhat
/// larger than the core count is right. The ceiling matters more than the floor:
/// past roughly `db_cores * 2` extra connections do not add throughput, they just
/// move the queue out of the client (cheap: a semaphore) and into Postgres
/// (expensive: a backend process and its `work_mem`).
///
/// The old fixed default of 10 throttled search badly — at 30 concurrent clients
/// with multi-second queries, two-thirds of every request's latency was spent
/// waiting for a connection (issue #224).
fn default_max_connections() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (cores * 4).clamp(16, 64)
}

fn default_connect_timeout_secs() -> u64 {
    5
}

fn default_statement_timeout_ms() -> u64 {
    30000
}

/// How long a caller waits for a pooled connection before giving up.
///
/// deadpool's default is to wait forever, which turns database saturation into an
/// unbounded latency tail instead of an honest rejection.
fn default_pool_wait_timeout_secs() -> u64 {
    10
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            dbname: default_dbname(),
            user: default_user(),
            password: None,
            ssl_mode: PostgresSslMode::default(),
            max_connections: default_max_connections(),
            connect_timeout_secs: default_connect_timeout_secs(),
            statement_timeout_ms: default_statement_timeout_ms(),
            pool_wait_timeout_secs: default_pool_wait_timeout_secs(),
            fhir_version: FhirVersion::default_enabled(),
            data_dir: None,
            search_offloaded: false,
            schema_name: None,
        }
    }
}

impl PostgresConfig {
    /// Applies `HFS_PG_*` tuning overrides.
    ///
    /// Called from every construction path, not just `from_env`. The connection-URL
    /// path (`HFS_DATABASE_URL`, which is how Docker and Kubernetes deployments and
    /// the benchmark all configure the server) previously started from `Default` and
    /// only overwrote host/port/user/password/dbname — so the pool size was pinned
    /// at its default and *no* environment variable could change it (issue #224).
    fn apply_env_overrides(&mut self) {
        if let Some(v) = std::env::var("HFS_PG_MAX_CONNECTIONS")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            self.max_connections = v;
        }
        if let Some(v) = std::env::var("HFS_PG_STATEMENT_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            self.statement_timeout_ms = v;
        }
        if let Some(v) = std::env::var("HFS_PG_POOL_WAIT_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            self.pool_wait_timeout_secs = v;
        }
    }
}

impl PostgresBackend {
    /// Creates a new PostgreSQL backend with the given configuration.
    pub async fn new(config: PostgresConfig) -> StorageResult<Self> {
        let pool = Self::create_pool(&config)?;

        // Verify connectivity.
        //
        // `statement_timeout` is deliberately NOT set here. It is a session GUC, so
        // a `SET` on one pooled connection binds only that connection — the others,
        // created lazily as the pool grows, inherited the server default (usually
        // 0 = no limit). Under load that meant a runaway query was capped or not
        // depending on which connection it happened to draw. It is now sent in the
        // startup packet by `create_pool`, so every connection carries it.
        let client = pool.get().await.map_err(|e| {
            crate::error::StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: "postgres".to_string(),
                message: e.to_string(),
            })
        })?;
        drop(client);

        // Initialize the per-tenant search parameter registries. Base params
        // (embedded + spec + custom) go into the shared base; each tenant's
        // stored overlay is read from `stored_by_tenant`, populated by
        // `reload_stored_cache` at startup / refresh / SearchParameter writes.
        let stored_by_tenant: StoredByTenant = Arc::new(RwLock::new(HashMap::new()));
        let loader_cache = stored_by_tenant.clone();
        let registries = Arc::new(TenantSearchRegistries::new(Arc::new(
            move |tenant_id: &str| {
                loader_cache
                    .read()
                    .get(tenant_id)
                    .cloned()
                    .unwrap_or_default()
            },
        )));
        Self::initialize_search_registry(registries.base(), &config);

        Ok(Self {
            pool,
            config,
            registries,
            stored_by_tenant,
            subscription_fanout: std::sync::OnceLock::new(),
        })
    }

    /// Creates a backend from a connection string.
    pub async fn from_connection_string(url: &str) -> StorageResult<Self> {
        let config = Self::parse_connection_string(url)?;
        Self::new(config).await
    }

    /// Creates a backend from environment variables.
    ///
    /// Reads the following environment variables:
    /// - `HFS_PG_HOST` (default: "localhost")
    /// - `HFS_PG_PORT` (default: 5432)
    /// - `HFS_PG_DBNAME` (default: "helios")
    /// - `HFS_PG_USER` (default: "helios")
    /// - `HFS_PG_PASSWORD`
    /// - `HFS_PG_MAX_CONNECTIONS` (default: `cores * 4`, clamped to 16..=64)
    /// - `HFS_PG_STATEMENT_TIMEOUT_MS` (default: 30000)
    /// - `HFS_PG_POOL_WAIT_TIMEOUT_SECS` (default: 10)
    pub async fn from_env() -> StorageResult<Self> {
        let mut config = PostgresConfig {
            host: std::env::var("HFS_PG_HOST").unwrap_or_else(|_| default_host()),
            port: std::env::var("HFS_PG_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or_else(default_port),
            dbname: std::env::var("HFS_PG_DBNAME").unwrap_or_else(|_| default_dbname()),
            user: std::env::var("HFS_PG_USER").unwrap_or_else(|_| default_user()),
            password: std::env::var("HFS_PG_PASSWORD").ok(),
            ..Default::default()
        };
        // Pool/timeout knobs are applied through the shared helper so this path and
        // the connection-URL path cannot drift apart again.
        config.apply_env_overrides();
        Self::new(config).await
    }

    fn create_pool(config: &PostgresConfig) -> StorageResult<Pool> {
        let mut cfg = Config::new();
        cfg.host = Some(config.host.clone());
        cfg.port = Some(config.port);
        cfg.dbname = Some(config.dbname.clone());
        cfg.user = Some(config.user.clone());
        cfg.password = config.password.clone();
        cfg.ssl_mode = Some(match config.ssl_mode {
            PostgresSslMode::Disable => SslMode::Disable,
            PostgresSslMode::Prefer => SslMode::Prefer,
            PostgresSslMode::Require => SslMode::Require,
        });

        // Ship `statement_timeout` in the startup packet so it applies to every
        // connection the pool ever creates — including ones added as the pool grows
        // and ones replaced after a recycle. A post-connect `SET` on a single
        // borrowed client cannot make that guarantee (it binds only that session),
        // and a per-connection hook would cost an extra round-trip.
        cfg.options = Some(format!(
            "-c statement_timeout={}",
            config.statement_timeout_ms
        ));
        // Makes HFS connections identifiable in pg_stat_activity.
        cfg.application_name = Some("hfs".to_string());

        let pool = cfg
            .builder(NoTls)
            .map_err(|e| {
                crate::error::StorageError::Backend(BackendError::Internal {
                    backend_name: "postgres".to_string(),
                    message: format!("Failed to create pool builder: {}", e),
                    source: None,
                })
            })?
            .max_size(config.max_connections)
            // deadpool waits forever by default, so a saturated pool — or a database
            // that accepts TCP but stalls the startup handshake — becomes an
            // unbounded latency tail. Bound the wait and surface exhaustion as a
            // fast `Unavailable` (503) instead of a request that hangs for a minute.
            .timeouts(deadpool_postgres::Timeouts {
                wait: Some(std::time::Duration::from_secs(
                    config.pool_wait_timeout_secs,
                )),
                create: Some(std::time::Duration::from_secs(config.connect_timeout_secs)),
                recycle: Some(std::time::Duration::from_secs(config.connect_timeout_secs)),
            })
            .runtime(Runtime::Tokio1)
            .build()
            .map_err(|e| {
                crate::error::StorageError::Backend(BackendError::ConnectionFailed {
                    backend_name: "postgres".to_string(),
                    message: e.to_string(),
                })
            })?;

        Ok(pool)
    }

    fn parse_connection_string(url: &str) -> StorageResult<PostgresConfig> {
        // Parse postgres:// URL format
        // postgres://user:password@host:port/dbname[?params]
        let url = url
            .strip_prefix("postgres://")
            .or_else(|| url.strip_prefix("postgresql://"))
            .unwrap_or(url);

        // Strip any query string before splitting on '/', otherwise it lands in
        // `dbname` verbatim (`.../postgres?sslmode=require` → dbname
        // "postgres?sslmode=require", which no server will resolve).
        let url = url.split('?').next().unwrap_or(url);

        let mut config = PostgresConfig::default();

        // Split user:password@host:port/dbname
        if let Some((userinfo, rest)) = url.split_once('@') {
            if let Some((user, password)) = userinfo.split_once(':') {
                config.user = user.to_string();
                config.password = Some(password.to_string());
            } else {
                config.user = userinfo.to_string();
            }

            if let Some((hostport, dbname)) = rest.split_once('/') {
                if let Some((host, port)) = hostport.split_once(':') {
                    config.host = host.to_string();
                    config.port = port.parse().unwrap_or(5432);
                } else {
                    config.host = hostport.to_string();
                }
                config.dbname = dbname.to_string();
            } else if let Some((host, port)) = rest.split_once(':') {
                config.host = host.to_string();
                config.port = port.parse().unwrap_or(5432);
            } else {
                config.host = rest.to_string();
            }
        }

        config.apply_env_overrides();
        Ok(config)
    }

    fn initialize_search_registry(
        registry: &Arc<RwLock<SearchParameterRegistry>>,
        config: &PostgresConfig,
    ) {
        let loader = SearchParameterLoader::new(config.fhir_version);
        let mut reg = registry.write();

        let mut fallback_count = 0;
        let mut spec_count = 0;
        let mut spec_file: Option<PathBuf> = None;
        let mut custom_count = 0;
        let mut custom_files: Vec<String> = Vec::new();

        // 1. Load minimal embedded fallback params
        match loader.load_embedded() {
            Ok(params) => {
                for param in params {
                    if reg.register(param).is_ok() {
                        fallback_count += 1;
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to load embedded SearchParameters: {}", e);
            }
        }

        // 2. Load spec file params
        let data_dir = config
            .data_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("./data"));
        let spec_filename = loader.spec_filename();
        let spec_path = data_dir.join(spec_filename);
        match loader.load_from_spec_file(&data_dir) {
            Ok(params) => {
                for param in params {
                    if reg.register(param).is_ok() {
                        spec_count += 1;
                    }
                }
                if spec_count > 0 {
                    spec_file = Some(spec_path);
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Could not load spec SearchParameters from {}: {}. Using minimal fallback.",
                    spec_path.display(),
                    e
                );
            }
        }

        // 3. Load custom SearchParameters
        match loader.load_custom_from_directory_with_files(&data_dir) {
            Ok((params, files)) => {
                for param in params {
                    if reg.register(param).is_ok() {
                        custom_count += 1;
                    }
                }
                custom_files = files;
            }
            Err(e) => {
                tracing::warn!(
                    "Error loading custom SearchParameters from {}: {}",
                    data_dir.display(),
                    e
                );
            }
        }

        let resource_type_count = reg.resource_types().len();
        let spec_info = spec_file
            .map(|p| format!(" from {}", p.display()))
            .unwrap_or_default();
        let custom_info = if custom_files.is_empty() {
            String::new()
        } else {
            format!(" [{}]", custom_files.join(", "))
        };
        tracing::info!(
            "PostgreSQL SearchParameter registry initialized: {} total ({} spec{}, {} fallback, {} custom{}) covering {} resource types",
            reg.len(),
            spec_count,
            spec_info,
            fallback_count,
            custom_count,
            custom_info,
            resource_type_count
        );
    }

    /// Initialize the database schema.
    pub async fn init_schema(&self) -> StorageResult<()> {
        let client = self.get_client().await?;
        super::schema::initialize_schema(&client).await?;
        // Populate the per-tenant stored-param cache so the registries can build
        // each tenant's overlay lazily.
        self.reload_stored_cache().await?;
        Ok(())
    }

    /// Reloads every tenant's stored active SearchParameters into the sync
    /// `stored_by_tenant` cache (grouped by tenant), then drops the cached
    /// per-tenant registries so they rebuild against the fresh overlay.
    pub(crate) async fn reload_stored_cache(&self) -> StorageResult<usize> {
        use crate::search::registry::{SearchParameterSource, SearchParameterStatus};

        let client = self.get_client().await?;
        let rows = client
            .query(
                "SELECT tenant_id, data FROM resources \
                 WHERE resource_type = 'SearchParameter' AND is_deleted = FALSE",
                &[],
            )
            .await
            .map_err(|e| {
                crate::error::StorageError::Backend(BackendError::Internal {
                    backend_name: "postgres".to_string(),
                    message: format!("Failed to query SearchParameters: {}", e),
                    source: None,
                })
            })?;

        let loader = SearchParameterLoader::new(self.config.fhir_version);
        let mut by_tenant: HashMap<String, Vec<SearchParameterDefinition>> = HashMap::new();
        let mut count = 0;
        for row in rows {
            let tenant_id: String = row.get(0);
            let data: serde_json::Value = row.get(1);
            if let Ok(mut def) = loader.parse_resource(&data) {
                if def.status == SearchParameterStatus::Active {
                    def.source = SearchParameterSource::Stored;
                    by_tenant.entry(tenant_id).or_default().push(def);
                    count += 1;
                }
            }
        }
        *self.stored_by_tenant.write() = by_tenant;
        self.registries.invalidate_all();
        Ok(count)
    }

    /// TTL-cache refresh (#235): reload the stored-param cache from storage and
    /// drop the cached per-tenant registries. Returns the stored-param count.
    pub async fn refresh_stored_search_parameters(&self) -> StorageResult<usize> {
        self.reload_stored_cache().await
    }

    /// Get a client from the pool.
    pub(crate) async fn get_client(&self) -> StorageResult<deadpool_postgres::Client> {
        use deadpool_postgres::{PoolError, TimeoutType};

        self.pool.get().await.map_err(|e| match e {
            // Every connection is busy and the wait timeout elapsed. The database
            // is healthy — we are simply over capacity — so this is a retryable
            // 503, not an internal error. Reporting it as a 500 (the previous
            // behavior) both misleads clients and hides saturation in error logs.
            PoolError::Timeout(TimeoutType::Wait) => {
                crate::error::StorageError::Backend(BackendError::Unavailable {
                    backend_name: "postgres".to_string(),
                    message: "connection pool exhausted".to_string(),
                })
            }
            other => crate::error::StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: "postgres".to_string(),
                message: other.to_string(),
            }),
        })
    }

    /// The per-tenant registry container (shared with a co-located ES backend).
    pub fn tenant_registries(&self) -> &Arc<TenantSearchRegistries> {
        &self.registries
    }

    /// The shared base registry (embedded + spec + custom), tenant-independent.
    pub(crate) fn base_registry(&self) -> &Arc<RwLock<SearchParameterRegistry>> {
        self.registries.base()
    }

    /// The registry for a tenant (base + that tenant's stored overlay).
    pub(crate) fn tenant_registry(&self, tenant_id: &str) -> Arc<RwLock<SearchParameterRegistry>> {
        self.registries.for_tenant(tenant_id)
    }

    /// A value extractor over a tenant's registry.
    pub(crate) fn tenant_extractor(&self, tenant_id: &str) -> SearchParameterExtractor {
        SearchParameterExtractor::new(self.tenant_registry(tenant_id))
    }

    /// Returns the backend configuration.
    pub fn config(&self) -> &PostgresConfig {
        &self.config
    }

    /// Returns a clone of the underlying connection pool.
    ///
    /// `deadpool_postgres::Pool` is `Clone` (Arc-backed), so this is cheap.
    pub(crate) fn pool(&self) -> Pool {
        self.pool.clone()
    }

    /// Returns the memoization slot for the subscription event fan-out
    /// (see the field docs on [`PostgresBackend`]).
    pub(crate) fn fanout_slot(
        &self,
    ) -> &std::sync::OnceLock<Arc<super::subscription_fanout::PgNotifyFanout>> {
        &self.subscription_fanout
    }

    /// Returns whether search indexing is offloaded to a secondary backend.
    pub fn is_search_offloaded(&self) -> bool {
        self.config.search_offloaded
    }

    /// Sets the search offloaded flag.
    pub fn set_search_offloaded(&mut self, offloaded: bool) {
        self.config.search_offloaded = offloaded;
    }
}

/// Connection wrapper for PostgreSQL.
#[allow(dead_code)]
pub struct PostgresConnection(pub(crate) deadpool_postgres::Client);

impl Debug for PostgresConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresConnection").finish()
    }
}

#[async_trait]
impl Backend for PostgresBackend {
    type Connection = PostgresConnection;

    fn kind(&self) -> BackendKind {
        BackendKind::Postgres
    }

    fn name(&self) -> &'static str {
        "postgres"
    }

    fn supports(&self, capability: BackendCapability) -> bool {
        matches!(
            capability,
            BackendCapability::Crud
                | BackendCapability::Versioning
                | BackendCapability::InstanceHistory
                | BackendCapability::TypeHistory
                | BackendCapability::SystemHistory
                | BackendCapability::BasicSearch
                | BackendCapability::DateSearch
                | BackendCapability::ReferenceSearch
                | BackendCapability::FullTextSearch
                | BackendCapability::Sorting
                | BackendCapability::OffsetPagination
                | BackendCapability::CursorPagination
                | BackendCapability::Transactions
                | BackendCapability::OptimisticLocking
                | BackendCapability::PessimisticLocking
                | BackendCapability::BulkExport
                | BackendCapability::BulkSubmitIngest
                | BackendCapability::BulkSubmitRestWorker
                | BackendCapability::Include
                | BackendCapability::Revinclude
                | BackendCapability::SharedSchema
                | BackendCapability::SchemaPerTenant
                | BackendCapability::DatabasePerTenant
        )
    }

    fn capabilities(&self) -> Vec<BackendCapability> {
        vec![
            BackendCapability::Crud,
            BackendCapability::Versioning,
            BackendCapability::InstanceHistory,
            BackendCapability::TypeHistory,
            BackendCapability::SystemHistory,
            BackendCapability::BasicSearch,
            BackendCapability::DateSearch,
            BackendCapability::ReferenceSearch,
            BackendCapability::FullTextSearch,
            BackendCapability::Sorting,
            BackendCapability::OffsetPagination,
            BackendCapability::CursorPagination,
            BackendCapability::Transactions,
            BackendCapability::OptimisticLocking,
            BackendCapability::PessimisticLocking,
            BackendCapability::BulkExport,
            BackendCapability::BulkSubmitIngest,
            BackendCapability::BulkSubmitRestWorker,
            BackendCapability::Include,
            BackendCapability::Revinclude,
            BackendCapability::SharedSchema,
            BackendCapability::SchemaPerTenant,
            BackendCapability::DatabasePerTenant,
        ]
    }

    async fn acquire(&self) -> Result<Self::Connection, BackendError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| BackendError::ConnectionFailed {
                backend_name: "postgres".to_string(),
                message: e.to_string(),
            })?;
        Ok(PostgresConnection(client))
    }

    async fn release(&self, _conn: Self::Connection) {
        // Connection is automatically returned to pool when dropped
    }

    async fn health_check(&self) -> Result<(), BackendError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|_| BackendError::Unavailable {
                backend_name: "postgres".to_string(),
                message: "Failed to get connection".to_string(),
            })?;
        client
            .query_one("SELECT 1", &[])
            .await
            .map_err(|e| BackendError::Internal {
                backend_name: "postgres".to_string(),
                message: format!("Health check failed: {}", e),
                source: None,
            })?;
        Ok(())
    }

    async fn initialize(&self) -> Result<(), BackendError> {
        self.init_schema()
            .await
            .map_err(|e| BackendError::Internal {
                backend_name: "postgres".to_string(),
                message: format!("Failed to initialize schema: {}", e),
                source: None,
            })
    }

    async fn migrate(&self) -> Result<(), BackendError> {
        self.init_schema()
            .await
            .map_err(|e| BackendError::Internal {
                backend_name: "postgres".to_string(),
                message: format!("Failed to run migrations: {}", e),
                source: None,
            })
    }
}

// ============================================================================
// SearchCapabilityProvider Implementation
// ============================================================================

use crate::core::capabilities::{
    GlobalSearchCapabilities, ResourceSearchCapabilities, SearchCapabilityProvider,
};
use crate::types::{
    IncludeCapability, PaginationCapability, ResultModeCapability, SearchParamFullCapability,
    SearchParamType, SpecialSearchParam,
};

impl SearchCapabilityProvider for PostgresBackend {
    fn resource_search_capabilities(
        &self,
        resource_type: &str,
    ) -> Option<ResourceSearchCapabilities> {
        let params = {
            let registry = self.base_registry().read();
            registry.get_active_params(resource_type)
        };

        if params.is_empty() {
            let common_params = {
                let registry = self.base_registry().read();
                registry.get_active_params("Resource")
            };
            if common_params.is_empty() {
                return None;
            }
        }

        let mut search_params = Vec::new();
        for param in &params {
            let mut cap = SearchParamFullCapability::new(&param.code, param.param_type)
                .with_definition(&param.url);
            let modifiers = Self::modifiers_for_type(param.param_type);
            cap = cap.with_modifiers(modifiers);
            if let Some(ref targets) = param.target {
                cap = cap.with_targets(targets.iter().map(|s| s.as_str()));
            }
            search_params.push(cap);
        }

        let common_params = {
            let registry = self.base_registry().read();
            registry.get_active_params("Resource")
        };
        for param in &common_params {
            if !search_params.iter().any(|p| p.name == param.code) {
                let mut cap = SearchParamFullCapability::new(&param.code, param.param_type)
                    .with_definition(&param.url);
                cap = cap.with_modifiers(Self::modifiers_for_type(param.param_type));
                search_params.push(cap);
            }
        }

        Some(
            ResourceSearchCapabilities::new(resource_type)
                .with_special_params(vec![
                    SpecialSearchParam::Id,
                    SpecialSearchParam::LastUpdated,
                    SpecialSearchParam::Tag,
                    SpecialSearchParam::Profile,
                    SpecialSearchParam::Security,
                ])
                .with_include_capabilities(vec![
                    IncludeCapability::Include,
                    IncludeCapability::Revinclude,
                ])
                .with_pagination_capabilities(vec![
                    PaginationCapability::Count,
                    PaginationCapability::Offset,
                    PaginationCapability::Cursor,
                    PaginationCapability::MaxPageSize(1000),
                    PaginationCapability::DefaultPageSize(20),
                ])
                .with_result_mode_capabilities(vec![
                    ResultModeCapability::Total,
                    ResultModeCapability::TotalNone,
                    ResultModeCapability::TotalAccurate,
                    ResultModeCapability::SummaryCount,
                ])
                .with_param_list(search_params),
        )
    }

    fn global_search_capabilities(&self) -> GlobalSearchCapabilities {
        GlobalSearchCapabilities::new()
            .with_special_params(vec![
                SpecialSearchParam::Id,
                SpecialSearchParam::LastUpdated,
                SpecialSearchParam::Tag,
                SpecialSearchParam::Profile,
                SpecialSearchParam::Security,
            ])
            .with_pagination(vec![
                PaginationCapability::Count,
                PaginationCapability::Offset,
                PaginationCapability::Cursor,
                PaginationCapability::MaxPageSize(1000),
                PaginationCapability::DefaultPageSize(20),
            ])
            .with_system_search()
    }
}

impl PostgresBackend {
    /// Returns supported modifiers for a parameter type.
    pub(super) fn modifiers_for_type(param_type: SearchParamType) -> Vec<&'static str> {
        match param_type {
            SearchParamType::String => vec!["exact", "contains", "text", "missing"],
            // `not-in` is intentionally omitted: it returns 501 (negated
            // value-set filtering is unimplemented), so it must not be
            // advertised as supported.
            SearchParamType::Token => {
                vec!["not", "text", "code-text", "in", "of-type", "missing"]
            }
            SearchParamType::Reference => vec![
                "identifier",
                "contains",
                "text",
                "code-text",
                "below",
                "above",
                "missing",
            ],
            SearchParamType::Date => vec!["missing"],
            SearchParamType::Number => vec!["missing"],
            SearchParamType::Quantity => vec!["missing"],
            SearchParamType::Uri => vec!["contains", "below", "above", "missing"],
            SearchParamType::Composite => vec!["missing"],
            SearchParamType::Special => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_connection_string ───────────────────────────────────

    #[test]
    fn parses_full_url() {
        let cfg = PostgresBackend::parse_connection_string(
            "postgres://alice:s3cret@db.example.com:5433/clinical",
        )
        .unwrap();
        assert_eq!(cfg.user, "alice");
        assert_eq!(cfg.password.as_deref(), Some("s3cret"));
        assert_eq!(cfg.host, "db.example.com");
        assert_eq!(cfg.port, 5433);
        assert_eq!(cfg.dbname, "clinical");
    }

    #[test]
    fn strips_query_string_so_it_does_not_leak_into_dbname() {
        // Regression for #224: a `?sslmode=...` suffix was previously parsed as
        // part of the database name (`clinical?sslmode=require`), which no server
        // could resolve.
        let cfg = PostgresBackend::parse_connection_string(
            "postgres://alice:s3cret@db.example.com:5433/clinical?sslmode=require&connect_timeout=10",
        )
        .unwrap();
        assert_eq!(cfg.dbname, "clinical");
        assert_eq!(cfg.host, "db.example.com");
        assert_eq!(cfg.port, 5433);
    }

    #[test]
    fn accepts_postgresql_scheme_and_defaults_port() {
        let cfg = PostgresBackend::parse_connection_string("postgresql://bob:pw@localhost/helios")
            .unwrap();
        assert_eq!(cfg.user, "bob");
        assert_eq!(cfg.host, "localhost");
        assert_eq!(cfg.port, 5432); // no explicit port → default
        assert_eq!(cfg.dbname, "helios");
    }

    #[test]
    fn parses_url_without_password() {
        let cfg =
            PostgresBackend::parse_connection_string("postgres://svc@10.0.0.5:5432/db").unwrap();
        assert_eq!(cfg.user, "svc");
        assert_eq!(cfg.password, None);
        assert_eq!(cfg.host, "10.0.0.5");
        assert_eq!(cfg.dbname, "db");
    }

    #[test]
    fn invalid_port_falls_back_to_default() {
        let cfg =
            PostgresBackend::parse_connection_string("postgres://u:p@host:not-a-port/db").unwrap();
        assert_eq!(cfg.host, "host");
        assert_eq!(cfg.port, 5432);
        assert_eq!(cfg.dbname, "db");
    }

    #[test]
    fn parses_host_and_port_without_dbname() {
        // `rest` has no '/', so the host:port branch is taken and dbname keeps
        // its default.
        let cfg = PostgresBackend::parse_connection_string("postgres://u:p@myhost:6000").unwrap();
        assert_eq!(cfg.host, "myhost");
        assert_eq!(cfg.port, 6000);
        assert_eq!(cfg.dbname, default_dbname());
    }

    #[test]
    fn parses_bare_host_only() {
        // No port and no dbname after the host.
        let cfg = PostgresBackend::parse_connection_string("postgres://u:p@onlyhost").unwrap();
        assert_eq!(cfg.host, "onlyhost");
        assert_eq!(cfg.port, default_port());
    }

    // ── default tuning knobs ──────────────────────────────────────

    #[test]
    fn default_pool_size_is_bounded() {
        // Whatever the host's core count, the pool default stays within the band
        // #224 established as useful — and never the old fixed 10.
        let n = default_max_connections();
        assert!((16..=64).contains(&n), "pool default {n} out of band");
    }

    #[test]
    fn default_timeouts_match_documented_values() {
        assert_eq!(default_statement_timeout_ms(), 30_000);
        assert_eq!(default_pool_wait_timeout_secs(), 10);
        assert_eq!(default_connect_timeout_secs(), 5);
    }

    #[test]
    fn config_default_populates_new_knobs() {
        let cfg = PostgresConfig::default();
        assert_eq!(cfg.pool_wait_timeout_secs, default_pool_wait_timeout_secs());
        assert_eq!(cfg.statement_timeout_ms, default_statement_timeout_ms());
        assert!((16..=64).contains(&cfg.max_connections));
    }

    // ── apply_env_overrides ───────────────────────────────────────

    // Serializes the two env-mutating tests below against each other. Other
    // tests in this module never assert on the env-overridable fields
    // (max_connections / statement_timeout_ms / pool_wait_timeout_secs), so they
    // are unaffected by these vars being set transiently.
    static ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn env_overrides_apply_when_set() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());

        // SAFETY: serialized by ENV_GUARD, and every var is removed before the
        // lock is released, so no other thread observes a partial state.
        unsafe {
            std::env::set_var("HFS_PG_MAX_CONNECTIONS", "99");
            std::env::set_var("HFS_PG_STATEMENT_TIMEOUT_MS", "1234");
            std::env::set_var("HFS_PG_POOL_WAIT_TIMEOUT_SECS", "7");
        }

        let mut cfg = PostgresConfig::default();
        cfg.apply_env_overrides();
        assert_eq!(cfg.max_connections, 99);
        assert_eq!(cfg.statement_timeout_ms, 1234);
        assert_eq!(cfg.pool_wait_timeout_secs, 7);

        // SAFETY: see above.
        unsafe {
            std::env::remove_var("HFS_PG_MAX_CONNECTIONS");
            std::env::remove_var("HFS_PG_STATEMENT_TIMEOUT_MS");
            std::env::remove_var("HFS_PG_POOL_WAIT_TIMEOUT_SECS");
        }
    }

    #[test]
    fn env_overrides_ignore_unparseable_values() {
        let _g = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());

        // SAFETY: serialized by ENV_GUARD, removed before releasing the lock.
        unsafe {
            std::env::set_var("HFS_PG_MAX_CONNECTIONS", "not-a-number");
        }

        let mut cfg = PostgresConfig::default();
        let before = cfg.max_connections;
        cfg.apply_env_overrides();
        // A value that fails to parse is ignored (parse → None), leaving the
        // default untouched rather than zeroing the pool.
        assert_eq!(cfg.max_connections, before);

        // SAFETY: see above.
        unsafe {
            std::env::remove_var("HFS_PG_MAX_CONNECTIONS");
        }
    }
}
