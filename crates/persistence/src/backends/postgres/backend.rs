//! PostgreSQL backend implementation.

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
use crate::search::{SearchParameterExtractor, SearchParameterLoader, SearchParameterRegistry};

/// PostgreSQL backend for FHIR resource storage.
pub struct PostgresBackend {
    pool: Pool,
    config: PostgresConfig,
    /// Search parameter registry (in-memory cache of active parameters).
    search_registry: Arc<RwLock<SearchParameterRegistry>>,
    /// Extractor for deriving searchable values from resources.
    search_extractor: Arc<SearchParameterExtractor>,
}

impl Debug for PostgresBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresBackend")
            .field("config", &self.config)
            .field("search_registry_len", &self.search_registry.read().len())
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

fn default_max_connections() -> usize {
    10
}

fn default_connect_timeout_secs() -> u64 {
    5
}

fn default_statement_timeout_ms() -> u64 {
    30000
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
            fhir_version: FhirVersion::default_enabled(),
            data_dir: None,
            search_offloaded: false,
            schema_name: None,
        }
    }
}

impl PostgresBackend {
    /// Creates a new PostgreSQL backend with the given configuration.
    pub async fn new(config: PostgresConfig) -> StorageResult<Self> {
        let pool = Self::create_pool(&config)?;

        // Verify connectivity
        let client = pool.get().await.map_err(|e| {
            crate::error::StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: "postgres".to_string(),
                message: e.to_string(),
            })
        })?;

        // Set statement timeout
        client
            .execute(
                &format!("SET statement_timeout = {}", config.statement_timeout_ms),
                &[],
            )
            .await
            .map_err(|e| {
                crate::error::StorageError::Backend(BackendError::Internal {
                    backend_name: "postgres".to_string(),
                    message: format!("Failed to set statement_timeout: {}", e),
                    source: None,
                })
            })?;

        drop(client);

        // Initialize the search parameter registry
        let search_registry = Arc::new(RwLock::new(SearchParameterRegistry::new()));
        Self::initialize_search_registry(&search_registry, &config);
        let search_extractor = Arc::new(SearchParameterExtractor::new(search_registry.clone()));

        Ok(Self {
            pool,
            config,
            search_registry,
            search_extractor,
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
    /// - `HFS_PG_MAX_CONNECTIONS` (default: 10)
    pub async fn from_env() -> StorageResult<Self> {
        let config = PostgresConfig {
            host: std::env::var("HFS_PG_HOST").unwrap_or_else(|_| default_host()),
            port: std::env::var("HFS_PG_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or_else(default_port),
            dbname: std::env::var("HFS_PG_DBNAME").unwrap_or_else(|_| default_dbname()),
            user: std::env::var("HFS_PG_USER").unwrap_or_else(|_| default_user()),
            password: std::env::var("HFS_PG_PASSWORD").ok(),
            max_connections: std::env::var("HFS_PG_MAX_CONNECTIONS")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or_else(default_max_connections),
            ..Default::default()
        };
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
        // postgres://user:password@host:port/dbname
        let url = url
            .strip_prefix("postgres://")
            .or_else(|| url.strip_prefix("postgresql://"))
            .unwrap_or(url);

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

        // Load stored SearchParameters from database
        let stored_count = self.load_stored_search_parameters().await?;
        if stored_count > 0 {
            let registry = self.search_registry.read();
            tracing::info!(
                "Loaded {} stored SearchParameters from database (total now: {})",
                stored_count,
                registry.len()
            );
        }

        Ok(())
    }

    /// Loads SearchParameter resources stored in the database into the registry.
    async fn load_stored_search_parameters(&self) -> StorageResult<usize> {
        use crate::search::registry::{SearchParameterSource, SearchParameterStatus};

        let client = self.get_client().await?;
        let rows = client
            .query(
                "SELECT data FROM resources WHERE resource_type = 'SearchParameter' AND is_deleted = FALSE",
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
        let mut registry = self.search_registry.write();
        let mut count = 0;

        for row in rows {
            let data: serde_json::Value = row.get(0);
            match loader.parse_resource(&data) {
                Ok(mut def) => {
                    if def.status == SearchParameterStatus::Active {
                        def.source = SearchParameterSource::Stored;
                        if registry.register(def).is_ok() {
                            count += 1;
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to parse stored SearchParameter: {}", e);
                }
            }
        }

        Ok(count)
    }

    /// Get a client from the pool.
    pub(crate) async fn get_client(&self) -> StorageResult<deadpool_postgres::Client> {
        self.pool.get().await.map_err(|e| {
            crate::error::StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: "postgres".to_string(),
                message: e.to_string(),
            })
        })
    }

    /// Get the search parameter registry.
    #[allow(dead_code)]
    pub(crate) fn get_search_registry(&self) -> Arc<RwLock<SearchParameterRegistry>> {
        Arc::clone(&self.search_registry)
    }

    /// Returns the backend configuration.
    pub fn config(&self) -> &PostgresConfig {
        &self.config
    }

    /// Returns a reference to the search parameter registry.
    pub fn search_registry(&self) -> &Arc<RwLock<SearchParameterRegistry>> {
        &self.search_registry
    }

    /// Returns a reference to the search parameter extractor.
    pub fn search_extractor(&self) -> &Arc<SearchParameterExtractor> {
        &self.search_extractor
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
            let registry = self.search_registry.read();
            registry.get_active_params(resource_type)
        };

        if params.is_empty() {
            let common_params = {
                let registry = self.search_registry.read();
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
            let registry = self.search_registry.read();
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
    fn modifiers_for_type(param_type: SearchParamType) -> Vec<&'static str> {
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
