//! SQLite backend implementation.

use std::fmt::Debug;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use parking_lot::RwLock;
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use serde::{Deserialize, Serialize};

use helios_fhir::FhirVersion;

use crate::core::{Backend, BackendCapability, BackendKind};
use crate::error::{BackendError, StorageResult};
use crate::search::{SearchParameterExtractor, SearchParameterLoader, SearchParameterRegistry};

use super::schema;

/// Counter for generating unique in-memory database names.
static MEMORY_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

/// SQLite backend for FHIR resource storage.
pub struct SqliteBackend {
    pool: Pool<SqliteConnectionManager>,
    config: SqliteBackendConfig,
    is_memory: bool,
    /// Search parameter registry (in-memory cache of active parameters).
    search_registry: Arc<RwLock<SearchParameterRegistry>>,
    /// Extractor for deriving searchable values from resources.
    search_extractor: Arc<SearchParameterExtractor>,
}

impl Debug for SqliteBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteBackend")
            .field("config", &self.config)
            .field("is_memory", &self.is_memory)
            .field("search_registry_len", &self.search_registry.read().len())
            .finish_non_exhaustive()
    }
}

/// Configuration for the SQLite backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteBackendConfig {
    /// Maximum number of connections in the pool.
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,

    /// Minimum number of idle connections.
    #[serde(default = "default_min_connections")]
    pub min_connections: u32,

    /// Connection timeout in milliseconds.
    #[serde(default = "default_connection_timeout_ms")]
    pub connection_timeout_ms: u64,

    /// SQLite busy timeout in milliseconds.
    #[serde(default = "default_busy_timeout_ms")]
    pub busy_timeout_ms: u32,

    /// Enable WAL mode for better concurrency.
    #[serde(default = "default_true")]
    pub enable_wal: bool,

    /// Enable foreign key constraints.
    #[serde(default = "default_true")]
    pub enable_foreign_keys: bool,

    /// FHIR version for this backend instance.
    /// Used to load the appropriate SearchParameter definitions.
    #[serde(default = "crate::default_fhir_version")]
    pub fhir_version: FhirVersion,

    /// Directory containing FHIR SearchParameter spec files.
    /// If None, defaults to "./data" or the directory containing the executable.
    #[serde(default)]
    pub data_dir: Option<PathBuf>,

    /// When true, search indexing is offloaded to a secondary backend (e.g., Elasticsearch).
    /// The SQLite search_index and resource_fts tables will not be populated.
    #[serde(default)]
    pub search_offloaded: bool,
}

fn default_max_connections() -> u32 {
    10
}

fn default_min_connections() -> u32 {
    1
}

fn default_connection_timeout_ms() -> u64 {
    30000
}

fn default_busy_timeout_ms() -> u32 {
    5000
}

fn default_true() -> bool {
    true
}

impl Default for SqliteBackendConfig {
    fn default() -> Self {
        Self {
            max_connections: default_max_connections(),
            min_connections: default_min_connections(),
            connection_timeout_ms: default_connection_timeout_ms(),
            busy_timeout_ms: default_busy_timeout_ms(),
            enable_wal: true,
            enable_foreign_keys: true,
            fhir_version: FhirVersion::default_enabled(),
            data_dir: None,
            search_offloaded: false,
        }
    }
}

impl SqliteBackend {
    /// Creates a new in-memory SQLite backend.
    pub fn in_memory() -> StorageResult<Self> {
        Self::with_config(":memory:", SqliteBackendConfig::default())
    }

    /// Opens or creates a file-based SQLite database.
    pub fn open<P: AsRef<Path>>(path: P) -> StorageResult<Self> {
        Self::with_config(path, SqliteBackendConfig::default())
    }

    /// Creates a backend with custom configuration.
    pub fn with_config<P: AsRef<Path>>(
        path: P,
        config: SqliteBackendConfig,
    ) -> StorageResult<Self> {
        let path_str = path.as_ref().to_string_lossy();
        let is_memory = path_str == ":memory:";

        // For in-memory databases with connection pools, we need to use a shared-cache
        // URI so all connections access the same database. Otherwise, each connection
        // gets its own isolated in-memory database. Each backend instance gets a unique
        // database name to ensure test isolation.
        let manager = if is_memory {
            let db_id = MEMORY_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
            let uri = format!("file:hfs_mem_{}?mode=memory&cache=shared", db_id);
            SqliteConnectionManager::file(uri)
        } else {
            SqliteConnectionManager::file(path.as_ref())
        };

        let pool = Pool::builder()
            .max_size(config.max_connections)
            .min_idle(Some(config.min_connections))
            .connection_timeout(std::time::Duration::from_millis(
                config.connection_timeout_ms,
            ))
            .build(manager)
            .map_err(|e| {
                crate::error::StorageError::Backend(BackendError::ConnectionFailed {
                    backend_name: "sqlite".to_string(),
                    message: e.to_string(),
                })
            })?;

        // Initialize the search parameter registry
        let search_registry = Arc::new(RwLock::new(SearchParameterRegistry::new()));
        {
            let loader = SearchParameterLoader::new(config.fhir_version);
            let mut registry = search_registry.write();

            // Track counts and sources for summary
            let mut fallback_count = 0;
            let mut spec_count = 0;
            let mut spec_file: Option<PathBuf> = None;
            let mut custom_count = 0;
            let mut custom_files: Vec<String> = Vec::new();

            // 1. Load minimal embedded fallback params (always available)
            match loader.load_embedded() {
                Ok(params) => {
                    for param in params {
                        if registry.register(param).is_ok() {
                            fallback_count += 1;
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to load embedded SearchParameters: {}", e);
                }
            }

            // 2. Load spec file params (may fail if file missing)
            let data_dir = config
                .data_dir
                .clone()
                .unwrap_or_else(|| PathBuf::from("./data"));
            let spec_filename = loader.spec_filename();
            let spec_path = data_dir.join(spec_filename);
            match loader.load_from_spec_file(&data_dir) {
                Ok(params) => {
                    for param in params {
                        if registry.register(param).is_ok() {
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

            // 3. Load custom SearchParameters from data directory (optional)
            match loader.load_custom_from_directory_with_files(&data_dir) {
                Ok((params, files)) => {
                    for param in params {
                        if registry.register(param).is_ok() {
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

            // Log summary
            let resource_type_count = registry.resource_types().len();
            let spec_info = spec_file
                .map(|p| format!(" from {}", p.display()))
                .unwrap_or_default();
            let custom_info = if custom_files.is_empty() {
                String::new()
            } else {
                format!(" [{}]", custom_files.join(", "))
            };
            tracing::info!(
                "SearchParameter registry initialized: {} total ({} spec{}, {} fallback, {} custom{}) covering {} resource types",
                registry.len(),
                spec_count,
                spec_info,
                fallback_count,
                custom_count,
                custom_info,
                resource_type_count
            );
        }
        let search_extractor = Arc::new(SearchParameterExtractor::new(search_registry.clone()));

        let backend = Self {
            pool,
            config,
            is_memory,
            search_registry,
            search_extractor,
        };

        // Configure the connection
        backend.configure_connection()?;

        Ok(backend)
    }

    /// Initialize the database schema.
    ///
    /// This also loads any stored SearchParameter resources from the database
    /// into the registry.
    pub fn init_schema(&self) -> StorageResult<()> {
        let conn = self.get_connection()?;
        schema::initialize_schema(&conn)?;

        // Load stored (POSTed) SearchParameters from database
        let stored_count = self.load_stored_search_parameters()?;
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
    ///
    /// This is called during schema initialization to restore any custom
    /// SearchParameters that were POSTed to the server.
    fn load_stored_search_parameters(&self) -> StorageResult<usize> {
        use crate::search::registry::{SearchParameterSource, SearchParameterStatus};

        let conn = self.get_connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT data FROM resources WHERE resource_type = 'SearchParameter' AND is_deleted = 0",
            )
            .map_err(|e| {
                crate::error::StorageError::Backend(BackendError::Internal {
                    backend_name: "sqlite".to_string(),
                    message: format!("Failed to prepare SearchParameter query: {}", e),
                    source: None,
                })
            })?;

        let loader = SearchParameterLoader::new(self.config.fhir_version);
        let mut registry = self.search_registry.write();
        let mut count = 0;

        let rows = stmt
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .map_err(|e| {
                crate::error::StorageError::Backend(BackendError::Internal {
                    backend_name: "sqlite".to_string(),
                    message: format!("Failed to query SearchParameters: {}", e),
                    source: None,
                })
            })?;

        for row in rows {
            let data = match row {
                Ok(data) => data,
                Err(e) => {
                    tracing::warn!("Failed to read SearchParameter row: {}", e);
                    continue;
                }
            };

            let json: serde_json::Value = match serde_json::from_slice(&data) {
                Ok(json) => json,
                Err(e) => {
                    tracing::warn!("Failed to parse SearchParameter JSON: {}", e);
                    continue;
                }
            };

            match loader.parse_resource(&json) {
                Ok(mut def) => {
                    // Only register active parameters
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

    /// Get a connection from the pool.
    pub(crate) fn get_connection(
        &self,
    ) -> StorageResult<PooledConnection<SqliteConnectionManager>> {
        self.pool.get().map_err(|e| {
            crate::error::StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: "sqlite".to_string(),
                message: e.to_string(),
            })
        })
    }

    /// Get the search parameter registry.
    pub(crate) fn get_search_registry(&self) -> Arc<RwLock<SearchParameterRegistry>> {
        Arc::clone(&self.search_registry)
    }

    /// Configure connection settings.
    fn configure_connection(&self) -> StorageResult<()> {
        let conn = self.get_connection()?;

        conn.busy_timeout(std::time::Duration::from_millis(
            self.config.busy_timeout_ms as u64,
        ))
        .map_err(|e| {
            crate::error::StorageError::Backend(BackendError::Internal {
                backend_name: "sqlite".to_string(),
                message: format!("Failed to set busy timeout: {}", e),
                source: None,
            })
        })?;

        if self.config.enable_foreign_keys {
            conn.execute("PRAGMA foreign_keys = ON", []).map_err(|e| {
                crate::error::StorageError::Backend(BackendError::Internal {
                    backend_name: "sqlite".to_string(),
                    message: format!("Failed to enable foreign keys: {}", e),
                    source: None,
                })
            })?;
        }

        if self.config.enable_wal && !self.is_memory {
            // PRAGMA journal_mode returns a result, so we use query_row instead of execute
            conn.query_row("PRAGMA journal_mode = WAL", [], |_row| Ok(()))
                .map_err(|e| {
                    crate::error::StorageError::Backend(BackendError::Internal {
                        backend_name: "sqlite".to_string(),
                        message: format!("Failed to enable WAL mode: {}", e),
                        source: None,
                    })
                })?;
        }

        Ok(())
    }

    /// Returns whether this is an in-memory database.
    pub fn is_memory(&self) -> bool {
        self.is_memory
    }

    /// Returns the backend configuration.
    pub fn config(&self) -> &SqliteBackendConfig {
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
    ///
    /// When set to true, the SQLite backend will skip populating the
    /// `search_index` and `resource_fts` tables, as search is handled
    /// by a secondary backend (e.g., Elasticsearch).
    pub fn set_search_offloaded(&mut self, offloaded: bool) {
        self.config.search_offloaded = offloaded;
    }
}

/// Connection wrapper for SQLite.
#[allow(dead_code)]
pub struct SqliteConnection(pub(crate) PooledConnection<SqliteConnectionManager>);

impl Debug for SqliteConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteConnection").finish()
    }
}

#[async_trait]
impl Backend for SqliteBackend {
    type Connection = SqliteConnection;

    fn kind(&self) -> BackendKind {
        BackendKind::Sqlite
    }

    fn name(&self) -> &'static str {
        "sqlite"
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
                | BackendCapability::Sorting
                | BackendCapability::OffsetPagination
                | BackendCapability::Transactions
                | BackendCapability::OptimisticLocking
                | BackendCapability::Include
                | BackendCapability::Revinclude
                | BackendCapability::SharedSchema
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
            BackendCapability::Sorting,
            BackendCapability::OffsetPagination,
            BackendCapability::Transactions,
            BackendCapability::OptimisticLocking,
            BackendCapability::Include,
            BackendCapability::Revinclude,
            BackendCapability::SharedSchema,
        ]
    }

    async fn acquire(&self) -> Result<Self::Connection, BackendError> {
        let conn = self
            .pool
            .get()
            .map_err(|e| BackendError::ConnectionFailed {
                backend_name: "sqlite".to_string(),
                message: e.to_string(),
            })?;
        Ok(SqliteConnection(conn))
    }

    async fn release(&self, _conn: Self::Connection) {
        // Connection is automatically returned to pool when dropped
    }

    async fn health_check(&self) -> Result<(), BackendError> {
        let conn = self
            .get_connection()
            .map_err(|_| BackendError::Unavailable {
                backend_name: "sqlite".to_string(),
                message: "Failed to get connection".to_string(),
            })?;
        conn.query_row("SELECT 1", [], |_| Ok(()))
            .map_err(|e| BackendError::Internal {
                backend_name: "sqlite".to_string(),
                message: format!("Health check failed: {}", e),
                source: None,
            })?;
        Ok(())
    }

    async fn initialize(&self) -> Result<(), BackendError> {
        self.init_schema().map_err(|e| BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: format!("Failed to initialize schema: {}", e),
            source: None,
        })
    }

    async fn migrate(&self) -> Result<(), BackendError> {
        // Schema migrations are handled by initialize_schema
        self.init_schema().map_err(|e| BackendError::Internal {
            backend_name: "sqlite".to_string(),
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

impl SearchCapabilityProvider for SqliteBackend {
    fn resource_search_capabilities(
        &self,
        resource_type: &str,
    ) -> Option<ResourceSearchCapabilities> {
        // Get active parameters for this resource type from the registry
        let params = {
            let registry = self.search_registry.read();
            registry.get_active_params(resource_type)
        };

        if params.is_empty() {
            // Also check if there are Resource-level params
            let common_params = {
                let registry = self.search_registry.read();
                registry.get_active_params("Resource")
            };
            if common_params.is_empty() {
                return None;
            }
        }

        // Build search parameter capabilities from the registry
        let mut search_params = Vec::new();
        for param in &params {
            let mut cap = SearchParamFullCapability::new(&param.code, param.param_type)
                .with_definition(&param.url);

            // Add modifiers based on parameter type
            let modifiers = Self::modifiers_for_type(param.param_type);
            cap = cap.with_modifiers(modifiers);

            // Add target types for reference parameters
            if let Some(ref targets) = param.target {
                cap = cap.with_targets(targets.iter().map(|s| s.as_str()));
            }

            search_params.push(cap);
        }

        // Add common Resource-level parameters
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

impl SqliteBackend {
    /// Returns supported modifiers for a parameter type.
    fn modifiers_for_type(param_type: SearchParamType) -> Vec<&'static str> {
        match param_type {
            SearchParamType::String => vec!["exact", "contains", "text", "missing"],
            // `not-in` is intentionally omitted: the SQLite backend returns 501
            // for it (negated value-set filtering is unimplemented), so it must
            // not be advertised. `text-advanced` is implemented by the token
            // handler and was previously under-advertised.
            SearchParamType::Token => vec![
                "not",
                "text",
                "in",
                "of-type",
                "code-text",
                "text-advanced",
                "missing",
            ],
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

    #[test]
    fn test_in_memory_backend() {
        let backend = SqliteBackend::in_memory().unwrap();
        assert!(backend.is_memory());
        assert_eq!(backend.name(), "sqlite");
        assert_eq!(backend.kind(), BackendKind::Sqlite);
    }

    #[test]
    fn test_backend_initialization() {
        let backend = SqliteBackend::in_memory().unwrap();
        backend.init_schema().unwrap();
        backend.init_schema().unwrap(); // Should be idempotent
    }

    #[test]
    fn test_backend_capabilities() {
        let backend = SqliteBackend::in_memory().unwrap();

        assert!(backend.supports(BackendCapability::Crud));
        assert!(backend.supports(BackendCapability::BasicSearch));
        assert!(backend.supports(BackendCapability::Transactions));
        assert!(!backend.supports(BackendCapability::FullTextSearch));
    }

    #[tokio::test]
    async fn test_health_check() {
        let backend = SqliteBackend::in_memory().unwrap();
        backend.init_schema().unwrap();
        assert!(backend.health_check().await.is_ok());
    }

    #[tokio::test]
    async fn test_acquire_release() {
        let backend = SqliteBackend::in_memory().unwrap();
        let conn = backend.acquire().await.unwrap();
        backend.release(conn).await;
    }

    #[test]
    fn test_search_capability_provider_patient() {
        let backend = SqliteBackend::in_memory().unwrap();

        // Get capabilities for Patient resource type
        let caps = backend.resource_search_capabilities("Patient");
        assert!(caps.is_some(), "Should have capabilities for Patient");

        let caps = caps.unwrap();
        assert_eq!(caps.resource_type, "Patient");

        // Should have common special parameters
        assert!(caps.supports_special(SpecialSearchParam::Id));
        assert!(caps.supports_special(SpecialSearchParam::LastUpdated));

        // Should support includes
        assert!(caps.supports_include(IncludeCapability::Include));
        assert!(caps.supports_include(IncludeCapability::Revinclude));

        // Should have search parameters from the registry
        // The exact set depends on what's loaded from R4 parameters
        assert!(
            !caps.search_params.is_empty(),
            "Should have search parameters"
        );
    }

    #[test]
    fn test_global_search_capabilities() {
        let backend = SqliteBackend::in_memory().unwrap();

        let global = backend.global_search_capabilities();

        // Should have common special parameters
        assert!(
            global
                .common_special_params
                .contains(&SpecialSearchParam::Id)
        );
        assert!(
            global
                .common_special_params
                .contains(&SpecialSearchParam::LastUpdated)
        );

        // Should support system search
        assert!(global.supports_system_search);

        // Should have pagination capabilities
        assert!(!global.common_pagination_capabilities.is_empty());
    }

    #[test]
    fn test_modifiers_for_type() {
        // String modifiers
        let string_mods = SqliteBackend::modifiers_for_type(SearchParamType::String);
        assert!(string_mods.contains(&"exact"));
        assert!(string_mods.contains(&"contains"));
        assert!(string_mods.contains(&"text"));
        assert!(string_mods.contains(&"missing"));

        // Token modifiers
        let token_mods = SqliteBackend::modifiers_for_type(SearchParamType::Token);
        assert!(token_mods.contains(&"not"));
        assert!(token_mods.contains(&"text"));
        assert!(token_mods.contains(&"of-type"));
        // `:code` is a non-spec modifier and must not be advertised.
        assert!(!token_mods.contains(&"code"));
        assert!(token_mods.contains(&"text-advanced"));
        // `not-in` returns 501, so it must not be advertised as supported.
        assert!(!token_mods.contains(&"not-in"));

        // Reference modifiers
        let ref_mods = SqliteBackend::modifiers_for_type(SearchParamType::Reference);
        assert!(ref_mods.contains(&"identifier"));

        // URI modifiers
        let uri_mods = SqliteBackend::modifiers_for_type(SearchParamType::Uri);
        assert!(uri_mods.contains(&"below"));
        assert!(uri_mods.contains(&"above"));
    }
}
