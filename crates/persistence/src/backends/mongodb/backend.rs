//! MongoDB backend implementation.

use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mongodb::{Client, Database, bson::doc, options::ClientOptions};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;

use helios_fhir::FhirVersion;

use crate::core::{Backend, BackendCapability, BackendKind};
use crate::error::{BackendError, StorageError, StorageResult};
use crate::search::{SearchParameterExtractor, SearchParameterLoader, SearchParameterRegistry};

use super::schema;

/// Upper bound for selecting a usable server before an operation gives up.
const SERVER_SELECTION_TIMEOUT: Duration = Duration::from_secs(15);

/// Connections idle longer than this are closed rather than reused, so a
/// connection silently dropped by a NAT/firewall is never handed to a request.
const MAX_CONNECTION_IDLE_TIME: Duration = Duration::from_secs(60);

/// MongoDB backend for FHIR resource storage.
///
/// The Phase 4 implementation provides backend wiring, schema bootstrap,
/// core ResourceStorage behavior for CRUD/count + tenant isolation,
/// [`crate::core::VersionedStorage`] support, and history providers.
///
/// Basic search and conditional create/update/delete are available.
/// Advanced search/composite behavior remains in later phases.
pub struct MongoBackend {
    config: MongoBackendConfig,
    /// Lazily initialized MongoDB client. MongoDB clients own their connection
    /// pools, so each backend instance must reuse one client.
    client: OnceCell<Client>,
    /// Search parameter registry (in-memory cache of active parameters).
    search_registry: Arc<RwLock<SearchParameterRegistry>>,
    /// Extractor for deriving searchable values from resources.
    search_extractor: Arc<SearchParameterExtractor>,
}

impl Debug for MongoBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MongoBackend")
            .field("config", &self.config)
            .field("search_registry_len", &self.search_registry.read().len())
            .finish_non_exhaustive()
    }
}

/// Configuration for the MongoDB backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MongoBackendConfig {
    /// MongoDB connection string.
    #[serde(default = "default_connection_string")]
    pub connection_string: String,

    /// MongoDB database name used by this backend.
    #[serde(default = "default_database_name")]
    pub database_name: String,

    /// Maximum number of connections in the driver pool.
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,

    /// Connection timeout in milliseconds.
    #[serde(default = "default_connect_timeout_ms")]
    pub connect_timeout_ms: u64,

    /// FHIR version for this backend instance.
    #[serde(default)]
    pub fhir_version: FhirVersion,

    /// Directory containing FHIR SearchParameter spec files.
    #[serde(default)]
    pub data_dir: Option<PathBuf>,

    /// When true, search indexing is offloaded to a secondary backend.
    #[serde(default)]
    pub search_offloaded: bool,
}

fn default_connection_string() -> String {
    "mongodb://localhost:27017".to_string()
}

fn default_database_name() -> String {
    "helios".to_string()
}

fn default_max_connections() -> u32 {
    10
}

fn default_connect_timeout_ms() -> u64 {
    5000
}

impl Default for MongoBackendConfig {
    fn default() -> Self {
        Self {
            connection_string: default_connection_string(),
            database_name: default_database_name(),
            max_connections: default_max_connections(),
            connect_timeout_ms: default_connect_timeout_ms(),
            fhir_version: FhirVersion::default(),
            data_dir: None,
            search_offloaded: false,
        }
    }
}

impl MongoBackend {
    pub(crate) const RESOURCES_COLLECTION: &'static str = "resources";
    pub(crate) const RESOURCE_HISTORY_COLLECTION: &'static str = "resource_history";
    pub(crate) const SEARCH_INDEX_COLLECTION: &'static str = "search_index";

    /// Creates a new MongoDB backend from the provided configuration.
    pub fn new(config: MongoBackendConfig) -> StorageResult<Self> {
        Self::validate_connection_string(&config.connection_string)?;

        let search_registry = Arc::new(RwLock::new(SearchParameterRegistry::new()));
        Self::initialize_search_registry(&search_registry, &config);
        let search_extractor = Arc::new(SearchParameterExtractor::new(search_registry.clone()));

        Ok(Self {
            config,
            client: OnceCell::new(),
            search_registry,
            search_extractor,
        })
    }

    /// Creates a backend from a MongoDB connection string.
    pub fn from_connection_string(connection_string: impl Into<String>) -> StorageResult<Self> {
        let config = MongoBackendConfig {
            connection_string: connection_string.into(),
            ..Default::default()
        };
        Self::new(config)
    }

    /// Creates a backend from environment variables.
    ///
    /// Supported variables:
    /// - `HFS_MONGODB_URL` (preferred)
    /// - `HFS_MONGODB_URI` (alias)
    /// - `HFS_DATABASE_URL` (fallback)
    /// - `HFS_MONGODB_DATABASE` (default: `helios`)
    /// - `HFS_MONGODB_MAX_CONNECTIONS` (default: `10`)
    /// - `HFS_MONGODB_CONNECT_TIMEOUT_MS` (default: `5000`)
    pub fn from_env() -> StorageResult<Self> {
        let connection_string = std::env::var("HFS_MONGODB_URL")
            .or_else(|_| std::env::var("HFS_MONGODB_URI"))
            .or_else(|_| std::env::var("HFS_DATABASE_URL"))
            .unwrap_or_else(|_| default_connection_string());

        let database_name =
            std::env::var("HFS_MONGODB_DATABASE").unwrap_or_else(|_| default_database_name());

        let max_connections = std::env::var("HFS_MONGODB_MAX_CONNECTIONS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or_else(default_max_connections);

        let connect_timeout_ms = std::env::var("HFS_MONGODB_CONNECT_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or_else(default_connect_timeout_ms);

        let config = MongoBackendConfig {
            connection_string,
            database_name,
            max_connections,
            connect_timeout_ms,
            ..Default::default()
        };

        Self::new(config)
    }

    fn validate_connection_string(connection_string: &str) -> StorageResult<()> {
        let uri = connection_string.trim();
        if uri.is_empty() {
            return Err(StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: "mongodb".to_string(),
                message: "MongoDB connection string cannot be empty".to_string(),
            }));
        }

        if !Self::looks_like_mongodb_uri(uri) {
            tracing::warn!(
                uri = %uri,
                "MongoDB connection string does not start with mongodb:// or mongodb+srv://"
            );
        }

        Ok(())
    }

    fn looks_like_mongodb_uri(connection_string: &str) -> bool {
        connection_string.starts_with("mongodb://")
            || connection_string.starts_with("mongodb+srv://")
    }

    fn initialize_search_registry(
        registry: &Arc<RwLock<SearchParameterRegistry>>,
        config: &MongoBackendConfig,
    ) {
        let loader = SearchParameterLoader::new(config.fhir_version);
        let mut reg = registry.write();

        let mut fallback_count = 0;
        let mut spec_count = 0;
        let mut spec_file: Option<PathBuf> = None;
        let mut custom_count = 0;
        let mut custom_files: Vec<String> = Vec::new();

        // 1. Load minimal embedded fallback params.
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

        // 2. Load spec file params.
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

        // 3. Load custom SearchParameters.
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
            "MongoDB SearchParameter registry initialized: {} total ({} spec{}, {} fallback, {} custom{}) covering {} resource types",
            reg.len(),
            spec_count,
            spec_info,
            fallback_count,
            custom_count,
            custom_info,
            resource_type_count
        );
    }

    /// Initializes the MongoDB schema/index bootstrap for this backend.
    pub async fn init_schema(&self) -> StorageResult<()> {
        let db = self.get_database().await?;
        schema::initialize_schema_async(&db).await
    }

    /// Creates a MongoDB client from backend configuration.
    async fn create_client(&self) -> StorageResult<Client> {
        let mut client_options = ClientOptions::parse(&self.config.connection_string)
            .await
            .map_err(|e| {
                StorageError::Backend(BackendError::ConnectionFailed {
                    backend_name: "mongodb".to_string(),
                    message: e.to_string(),
                })
            })?;

        client_options.max_pool_size = Some(self.config.max_connections);
        client_options.connect_timeout =
            Some(Duration::from_millis(self.config.connect_timeout_ms));
        client_options.app_name = Some("helios-persistence".to_string());

        // Fail fast when no healthy server can be selected. `connect_timeout`
        // only covers new TCP handshakes; this caps requests once server
        // monitoring has marked the server unavailable.
        client_options.server_selection_timeout = Some(SERVER_SELECTION_TIMEOUT);
        // Recycle idle connections so stale ones (e.g. silently dropped by a
        // NAT/firewall on a long-lived network path) are not handed out.
        client_options.max_idle_time = Some(MAX_CONNECTION_IDLE_TIME);

        Client::with_options(client_options).map_err(|e| {
            StorageError::Backend(BackendError::Internal {
                backend_name: "mongodb".to_string(),
                message: format!("Failed to create MongoDB client: {}", e),
                source: None,
            })
        })
    }

    /// Returns the shared MongoDB client for this backend.
    pub(crate) async fn get_client(&self) -> StorageResult<Client> {
        self.client
            .get_or_try_init(|| self.create_client())
            .await
            .cloned()
    }

    /// Returns the configured MongoDB database handle.
    pub(crate) async fn get_database(&self) -> StorageResult<Database> {
        let client = self.get_client().await?;
        Ok(client.database(&self.config.database_name))
    }

    /// Returns the backend configuration.
    pub fn config(&self) -> &MongoBackendConfig {
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

    /// Sets the search-offloaded flag.
    pub fn set_search_offloaded(&mut self, offloaded: bool) {
        self.config.search_offloaded = offloaded;
    }
}

/// Connection wrapper for MongoDB.
#[derive(Clone)]
pub struct MongoConnection {
    pub(crate) database: Database,
}

impl Debug for MongoConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MongoConnection")
            .field("database", &self.database.name())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Backend for MongoBackend {
    type Connection = MongoConnection;

    fn kind(&self) -> BackendKind {
        BackendKind::MongoDB
    }

    fn name(&self) -> &'static str {
        "mongodb"
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
                | BackendCapability::CursorPagination
                | BackendCapability::Transactions
                | BackendCapability::OptimisticLocking
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
            BackendCapability::CursorPagination,
            BackendCapability::Transactions,
            BackendCapability::OptimisticLocking,
            BackendCapability::SharedSchema,
        ]
    }

    async fn acquire(&self) -> Result<Self::Connection, BackendError> {
        let client = self
            .get_client()
            .await
            .map_err(|e| BackendError::ConnectionFailed {
                backend_name: "mongodb".to_string(),
                message: e.to_string(),
            })?;
        let database = client.database(&self.config.database_name);
        Ok(MongoConnection { database })
    }

    async fn release(&self, _conn: Self::Connection) {
        // MongoDB connection pooling is managed by the client internally.
    }

    async fn health_check(&self) -> Result<(), BackendError> {
        if !Self::looks_like_mongodb_uri(&self.config.connection_string) {
            return Err(BackendError::Unavailable {
                backend_name: "mongodb".to_string(),
                message: "Invalid MongoDB connection string format".to_string(),
            });
        }

        let db = self
            .get_database()
            .await
            .map_err(|e| BackendError::Unavailable {
                backend_name: "mongodb".to_string(),
                message: format!("Unable to create database handle: {}", e),
            })?;

        db.run_command(doc! { "ping": 1_i32 })
            .await
            .map_err(|e| BackendError::Unavailable {
                backend_name: "mongodb".to_string(),
                message: format!("Health check failed: {}", e),
            })?;

        Ok(())
    }

    async fn initialize(&self) -> Result<(), BackendError> {
        self.init_schema()
            .await
            .map_err(|e| BackendError::Internal {
                backend_name: "mongodb".to_string(),
                message: format!("Failed to initialize schema: {}", e),
                source: None,
            })
    }

    async fn migrate(&self) -> Result<(), BackendError> {
        let db = self
            .get_database()
            .await
            .map_err(|e| BackendError::Internal {
                backend_name: "mongodb".to_string(),
                message: format!("Failed to acquire database for migration: {}", e),
                source: None,
            })?;

        schema::migrate_schema_async(&db)
            .await
            .map_err(|e| BackendError::Internal {
                backend_name: "mongodb".to_string(),
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

impl MongoBackend {
    /// Returns the search modifiers the MongoDB backend actually honors for a
    /// parameter type. Unlike the SQL backends, the Mongo search implementation
    /// does not implement `:missing`, token `:not`/`:of-type`, reference
    /// `:identifier`, or uri `:above`/`:below`; advertising only what
    /// `search_impl` accepts keeps the CapabilityStatement honest.
    fn modifiers_for_type(param_type: SearchParamType) -> Vec<&'static str> {
        match param_type {
            SearchParamType::String => vec!["exact", "contains", "text"],
            SearchParamType::Token => vec!["code", "text", "code-text"],
            SearchParamType::Reference => vec!["contains", "text", "code-text"],
            SearchParamType::Uri => vec!["exact", "contains"],
            SearchParamType::Date
            | SearchParamType::Number
            | SearchParamType::Quantity
            | SearchParamType::Composite
            | SearchParamType::Special => vec![],
        }
    }
}

impl SearchCapabilityProvider for MongoBackend {
    fn resource_search_capabilities(
        &self,
        resource_type: &str,
    ) -> Option<ResourceSearchCapabilities> {
        let params = {
            let registry = self.search_registry.read();
            registry.get_active_params(resource_type)
        };
        let common_params = {
            let registry = self.search_registry.read();
            registry.get_active_params("Resource")
        };
        if params.is_empty() && common_params.is_empty() {
            return None;
        }

        let mut search_params = Vec::new();
        for param in &params {
            let mut cap = SearchParamFullCapability::new(&param.code, param.param_type)
                .with_definition(&param.url)
                .with_modifiers(Self::modifiers_for_type(param.param_type));
            if let Some(ref targets) = param.target {
                cap = cap.with_targets(targets.iter().map(|s| s.as_str()));
            }
            search_params.push(cap);
        }
        for param in &common_params {
            if !search_params.iter().any(|p| p.name == param.code) {
                search_params.push(
                    SearchParamFullCapability::new(&param.code, param.param_type)
                        .with_definition(&param.url)
                        .with_modifiers(Self::modifiers_for_type(param.param_type)),
                );
            }
        }

        Some(
            ResourceSearchCapabilities::new(resource_type)
                .with_special_params(vec![SpecialSearchParam::Id])
                .with_include_capabilities(vec![
                    IncludeCapability::Include,
                    IncludeCapability::Revinclude,
                ])
                .with_pagination_capabilities(vec![
                    PaginationCapability::Count,
                    PaginationCapability::Offset,
                    PaginationCapability::MaxPageSize(1000),
                    PaginationCapability::DefaultPageSize(20),
                ])
                .with_result_mode_capabilities(vec![ResultModeCapability::Total])
                .with_param_list(search_params),
        )
    }

    fn global_search_capabilities(&self) -> GlobalSearchCapabilities {
        GlobalSearchCapabilities::new().with_special_params(vec![SpecialSearchParam::Id])
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;

    #[test]
    fn test_modifiers_for_type_reflects_mongo_support() {
        // String honors exact/contains/text but not :missing.
        let s = MongoBackend::modifiers_for_type(SearchParamType::String);
        assert!(s.contains(&"exact"));
        assert!(s.contains(&"text"));
        assert!(!s.contains(&"missing"));

        // Token honors code/text/code-text but not :not or :of-type.
        let t = MongoBackend::modifiers_for_type(SearchParamType::Token);
        assert!(t.contains(&"code"));
        assert!(t.contains(&"code-text"));
        assert!(!t.contains(&"not"));
        assert!(!t.contains(&"of-type"));

        // Reference honors contains/text/code-text but not :identifier.
        let r = MongoBackend::modifiers_for_type(SearchParamType::Reference);
        assert!(r.contains(&"contains"));
        assert!(r.contains(&"text"));
        assert!(!r.contains(&"identifier"));

        // Uri honors exact/contains but not :above/:below.
        let u = MongoBackend::modifiers_for_type(SearchParamType::Uri);
        assert!(u.contains(&"contains"));
        assert!(!u.contains(&"above"));
        assert!(!u.contains(&"below"));
    }
}
