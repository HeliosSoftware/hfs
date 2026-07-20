//! Elasticsearch backend implementation.

use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use elasticsearch::Elasticsearch;
use elasticsearch::auth::Credentials;
use elasticsearch::cert::CertificateValidation;
use elasticsearch::http::transport::{SingleNodeConnectionPool, TransportBuilder};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use helios_fhir::FhirVersion;

use crate::core::{Backend, BackendCapability, BackendKind};
use crate::error::{BackendError, StorageResult};
use crate::search::{
    SearchParameterExtractor, SearchParameterLoader, SearchParameterRegistry,
    TenantSearchRegistries,
};

/// Authentication configuration for Elasticsearch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ElasticsearchAuth {
    /// Basic username/password authentication.
    Basic {
        /// The username for basic auth.
        username: String,
        /// The password for basic auth.
        password: String,
    },
    /// Bearer token authentication.
    Bearer {
        /// The bearer token.
        token: String,
    },
}

/// Configuration for the Elasticsearch backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElasticsearchConfig {
    /// Elasticsearch node URLs (e.g., `["http://localhost:9200"]`).
    /// Currently uses the first node (single-node connection pool).
    pub nodes: Vec<String>,

    /// Index name prefix (default: `"hfs"`).
    /// Indices are named: `{prefix}_{tenant_id}_{resource_type_lowercase}`
    #[serde(default = "default_index_prefix")]
    pub index_prefix: String,

    /// Number of primary shards per index (default: 1).
    #[serde(default = "default_shards")]
    pub number_of_shards: u32,

    /// Number of replica shards per index (default: 1).
    #[serde(default = "default_replicas")]
    pub number_of_replicas: u32,

    /// Refresh interval (default: "1s").
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval: String,

    /// Maximum result window size (default: 10000).
    #[serde(default = "default_max_result_window")]
    pub max_result_window: u32,

    /// Request timeout in milliseconds (default: 30000).
    #[serde(default = "default_request_timeout_ms")]
    pub request_timeout_ms: u64,

    /// Optional authentication.
    #[serde(default)]
    pub auth: Option<ElasticsearchAuth>,

    /// Whether to disable certificate validation (default: false).
    /// Only use for development/testing.
    #[serde(default)]
    pub disable_certificate_validation: bool,

    /// FHIR version for SearchParameter loading.
    #[serde(default = "crate::default_fhir_version")]
    pub fhir_version: FhirVersion,
}

fn default_index_prefix() -> String {
    "hfs".to_string()
}

fn default_shards() -> u32 {
    1
}

fn default_replicas() -> u32 {
    1
}

fn default_refresh_interval() -> String {
    "1s".to_string()
}

fn default_max_result_window() -> u32 {
    10000
}

fn default_request_timeout_ms() -> u64 {
    30000
}

impl Default for ElasticsearchConfig {
    fn default() -> Self {
        Self {
            nodes: vec!["http://localhost:9200".to_string()],
            index_prefix: default_index_prefix(),
            number_of_shards: default_shards(),
            number_of_replicas: default_replicas(),
            refresh_interval: default_refresh_interval(),
            max_result_window: default_max_result_window(),
            request_timeout_ms: default_request_timeout_ms(),
            auth: None,
            disable_certificate_validation: false,
            fhir_version: FhirVersion::default_enabled(),
        }
    }
}

/// Elasticsearch backend for FHIR resource search.
///
/// This backend is designed as a search-optimized secondary in the composite
/// storage layer. It receives data via sync events from the primary backend
/// and provides efficient search capabilities.
pub struct ElasticsearchBackend {
    /// The Elasticsearch client.
    client: Elasticsearch,
    /// Configuration.
    config: ElasticsearchConfig,
    /// Per-tenant search parameter registries (a shared base plus per-tenant
    /// overlays). Shared with the primary backend for consistency.
    registries: Arc<TenantSearchRegistries>,
}

impl Debug for ElasticsearchBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ElasticsearchBackend")
            .field("config", &self.config)
            .field("base_registry_len", &self.registries.base().read().len())
            .finish_non_exhaustive()
    }
}

impl ElasticsearchBackend {
    /// Creates a new Elasticsearch backend with the given configuration.
    pub fn new(config: ElasticsearchConfig) -> StorageResult<Self> {
        let client = Self::build_client(&config)?;

        // Standalone ES has no store of its own, so tenants have no overlay:
        // every tenant sees the shared base (embedded params only).
        let registries = Arc::new(TenantSearchRegistries::base_only());
        {
            let loader = SearchParameterLoader::new(config.fhir_version);
            let mut registry = registries.base().write();

            // Load embedded fallback params
            match loader.load_embedded() {
                Ok(params) => {
                    for param in params {
                        let _ = registry.register(param);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to load embedded SearchParameters: {}", e);
                }
            }

            tracing::info!(
                "Elasticsearch SearchParameter registry initialized: {} params covering {} resource types",
                registry.len(),
                registry.resource_types().len()
            );
        }

        Ok(Self {
            client,
            config,
            registries,
        })
    }

    /// Creates a new backend with a shared per-tenant registry container.
    ///
    /// Use this when the ES backend should share its registries with a primary
    /// backend (composite deployments): the container's loader points at the
    /// primary's storage, so ES resolves per-tenant overlays without its own DB.
    pub fn with_shared_registry(
        config: ElasticsearchConfig,
        registries: Arc<TenantSearchRegistries>,
    ) -> StorageResult<Self> {
        let client = Self::build_client(&config)?;

        Ok(Self {
            client,
            config,
            registries,
        })
    }

    /// Builds the Elasticsearch client from configuration.
    fn build_client(config: &ElasticsearchConfig) -> StorageResult<Elasticsearch> {
        let url = config
            .nodes
            .first()
            .cloned()
            .unwrap_or_else(|| "http://localhost:9200".to_string());

        let parsed_url: elasticsearch::http::Url = url.parse().map_err(|e| {
            crate::error::StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: "elasticsearch".to_string(),
                message: format!("Invalid URL: {}", e),
            })
        })?;

        let conn_pool = SingleNodeConnectionPool::new(parsed_url);

        let mut builder = TransportBuilder::new(conn_pool)
            .timeout(Duration::from_millis(config.request_timeout_ms));

        if config.disable_certificate_validation {
            builder = builder.cert_validation(CertificateValidation::None);
        }

        if let Some(ref auth) = config.auth {
            builder = match auth {
                ElasticsearchAuth::Basic { username, password } => {
                    builder.auth(Credentials::Basic(username.clone(), password.clone()))
                }
                ElasticsearchAuth::Bearer { token } => {
                    builder.auth(Credentials::Bearer(token.clone()))
                }
            };
        }

        let transport = builder.build().map_err(|e| {
            crate::error::StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: "elasticsearch".to_string(),
                message: format!("Failed to build transport: {}", e),
            })
        })?;

        Ok(Elasticsearch::new(transport))
    }

    /// Returns the Elasticsearch client.
    pub(crate) fn client(&self) -> &Elasticsearch {
        &self.client
    }

    /// Returns the backend configuration.
    pub fn config(&self) -> &ElasticsearchConfig {
        &self.config
    }

    /// Returns the per-tenant search parameter registries (shared base + tenant
    /// overlays).
    pub fn tenant_registries(&self) -> &Arc<TenantSearchRegistries> {
        &self.registries
    }

    /// Returns the shared base registry (embedded/spec/custom, tenant-agnostic).
    #[allow(dead_code)]
    pub(crate) fn base_registry(&self) -> &Arc<RwLock<SearchParameterRegistry>> {
        self.registries.base()
    }

    /// Builds an extractor over the given tenant's registry.
    ///
    /// Public because the `s3`+Elasticsearch composite has no SQL primary to
    /// borrow an extractor from: S3 stores resources but maintains no search
    /// index, so Elasticsearch's extractor is the only one in that deployment.
    pub fn tenant_extractor(&self, tenant_id: &str) -> SearchParameterExtractor {
        SearchParameterExtractor::new(self.registries.for_tenant(tenant_id))
    }

    /// Returns the index name for a tenant and resource type.
    pub fn index_name(&self, tenant_id: &str, resource_type: &str) -> String {
        format!(
            "{}_{}_{}",
            self.config.index_prefix,
            tenant_id.to_lowercase(),
            resource_type.to_lowercase()
        )
    }

    /// Returns the ES document ID for a resource.
    pub(crate) fn document_id(resource_type: &str, resource_id: &str) -> String {
        format!("{}_{}", resource_type, resource_id)
    }

    /// Refreshes an index to make recently indexed documents searchable.
    ///
    /// Only needed for testing; in production ES refreshes automatically.
    pub async fn refresh_index(&self, tenant_id: &str, resource_type: &str) -> StorageResult<()> {
        let index = self.index_name(tenant_id, resource_type);
        self.client
            .indices()
            .refresh(elasticsearch::indices::IndicesRefreshParts::Index(&[
                &index,
            ]))
            .send()
            .await
            .map_err(|e| {
                crate::error::StorageError::Backend(BackendError::Internal {
                    backend_name: "elasticsearch".to_string(),
                    message: format!("Failed to refresh index {}: {}", index, e),
                    source: None,
                })
            })?;
        Ok(())
    }
}

/// Connection wrapper for Elasticsearch.
///
/// ES uses an HTTP client internally, so connections are managed by the transport.
/// This is a placeholder to satisfy the `Backend` trait's `Connection` associated type.
#[derive(Debug)]
pub struct ElasticsearchConnection;

#[async_trait]
impl Backend for ElasticsearchBackend {
    type Connection = ElasticsearchConnection;

    fn kind(&self) -> BackendKind {
        BackendKind::Elasticsearch
    }

    fn name(&self) -> &'static str {
        "elasticsearch"
    }

    fn supports(&self, capability: BackendCapability) -> bool {
        matches!(
            capability,
            BackendCapability::Crud
                | BackendCapability::BasicSearch
                | BackendCapability::DateSearch
                | BackendCapability::QuantitySearch
                | BackendCapability::ReferenceSearch
                | BackendCapability::FullTextSearch
                | BackendCapability::Sorting
                | BackendCapability::CursorPagination
                | BackendCapability::OffsetPagination
                | BackendCapability::Include
                | BackendCapability::Revinclude
                | BackendCapability::SharedSchema
        )
    }

    fn capabilities(&self) -> Vec<BackendCapability> {
        vec![
            BackendCapability::Crud,
            BackendCapability::BasicSearch,
            BackendCapability::DateSearch,
            BackendCapability::QuantitySearch,
            BackendCapability::ReferenceSearch,
            BackendCapability::FullTextSearch,
            BackendCapability::Sorting,
            BackendCapability::CursorPagination,
            BackendCapability::OffsetPagination,
            BackendCapability::Include,
            BackendCapability::Revinclude,
            BackendCapability::SharedSchema,
        ]
    }

    async fn acquire(&self) -> Result<Self::Connection, BackendError> {
        // ES client manages connections internally via HTTP transport
        Ok(ElasticsearchConnection)
    }

    async fn release(&self, _conn: Self::Connection) {
        // No-op: ES client manages connections internally
    }

    async fn health_check(&self) -> Result<(), BackendError> {
        let response = self
            .client
            .cluster()
            .health(elasticsearch::cluster::ClusterHealthParts::None)
            .send()
            .await
            .map_err(|e| BackendError::Unavailable {
                backend_name: "elasticsearch".to_string(),
                message: format!("Health check failed: {}", e),
            })?;

        let status = response.status_code();
        if !status.is_success() {
            return Err(BackendError::Unavailable {
                backend_name: "elasticsearch".to_string(),
                message: format!("Cluster health returned status {}", status),
            });
        }

        let body = response
            .json::<Value>()
            .await
            .map_err(|e| BackendError::Internal {
                backend_name: "elasticsearch".to_string(),
                message: format!("Failed to parse health response: {}", e),
                source: None,
            })?;

        let cluster_status = body
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown");

        if cluster_status == "red" {
            return Err(BackendError::Unavailable {
                backend_name: "elasticsearch".to_string(),
                message: format!("Cluster status is red: {:?}", body),
            });
        }

        Ok(())
    }

    async fn initialize(&self) -> Result<(), BackendError> {
        // Create index template for automatic index creation
        super::schema::create_index_template(self)
            .await
            .map_err(|e| BackendError::Internal {
                backend_name: "elasticsearch".to_string(),
                message: format!("Failed to create index template: {}", e),
                source: None,
            })
    }

    async fn migrate(&self) -> Result<(), BackendError> {
        // Re-apply index template (idempotent)
        self.initialize().await
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

impl SearchCapabilityProvider for ElasticsearchBackend {
    fn resource_search_capabilities(
        &self,
        resource_type: &str,
    ) -> Option<ResourceSearchCapabilities> {
        let params = {
            let registry = self.registries.base().read();
            registry.get_active_params(resource_type)
        };

        if params.is_empty() {
            let common_params = {
                let registry = self.registries.base().read();
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

        // Add common Resource-level parameters
        let common_params = {
            let registry = self.registries.base().read();
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
                    SpecialSearchParam::Text,
                    SpecialSearchParam::Content,
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
                SpecialSearchParam::Text,
                SpecialSearchParam::Content,
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

impl ElasticsearchBackend {
    /// Returns supported modifiers for a parameter type.
    ///
    /// ES supports more modifiers than SQLite, especially for full-text.
    pub(super) fn modifiers_for_type(param_type: SearchParamType) -> Vec<&'static str> {
        match param_type {
            SearchParamType::String => vec!["exact", "contains", "text", "missing"],
            SearchParamType::Token => {
                // `not-in` is intentionally omitted: it returns 501 (negated
                // value-set filtering is unimplemented), so it must not be
                // advertised as supported.
                vec![
                    "not",
                    "text",
                    "text-advanced",
                    "code-text",
                    "in",
                    "of-type",
                    "missing",
                ]
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
    use serde_json::json;

    #[test]
    fn test_config_defaults() {
        let config = ElasticsearchConfig::default();
        assert_eq!(config.index_prefix, "hfs");
        assert_eq!(config.number_of_shards, 1);
        assert_eq!(config.number_of_replicas, 1);
        assert_eq!(config.nodes, vec!["http://localhost:9200"]);
    }

    #[test]
    fn test_index_name() {
        let config = ElasticsearchConfig::default();
        let backend = ElasticsearchBackend::new(config).unwrap();
        assert_eq!(backend.index_name("acme", "Patient"), "hfs_acme_patient");
        assert_eq!(
            backend.index_name("ACME", "Observation"),
            "hfs_acme_observation"
        );
    }

    #[test]
    fn test_document_id() {
        assert_eq!(
            ElasticsearchBackend::document_id("Patient", "123"),
            "Patient_123"
        );
    }

    #[test]
    fn test_backend_capabilities() {
        let config = ElasticsearchConfig::default();
        let backend = ElasticsearchBackend::new(config).unwrap();

        assert!(backend.supports(BackendCapability::BasicSearch));
        assert!(backend.supports(BackendCapability::FullTextSearch));
        assert!(backend.supports(BackendCapability::CursorPagination));
        assert!(backend.supports(BackendCapability::Sorting));
        assert!(!backend.supports(BackendCapability::Transactions));
        assert!(!backend.supports(BackendCapability::ChainedSearch));
    }

    #[test]
    fn test_backend_kind() {
        let config = ElasticsearchConfig::default();
        let backend = ElasticsearchBackend::new(config).unwrap();
        assert_eq!(backend.kind(), BackendKind::Elasticsearch);
        assert_eq!(backend.name(), "elasticsearch");
    }

    #[test]
    fn test_with_shared_registry_reuses_container() {
        let config = ElasticsearchConfig::default();
        let shared = Arc::new(TenantSearchRegistries::base_only());

        let backend = ElasticsearchBackend::with_shared_registry(config, shared.clone()).unwrap();

        assert!(Arc::ptr_eq(backend.tenant_registries(), &shared));
    }

    #[test]
    fn test_with_shared_registry_reflects_base_updates() {
        let config = ElasticsearchConfig::default();
        let shared = Arc::new(TenantSearchRegistries::base_only());
        let backend = ElasticsearchBackend::with_shared_registry(config, shared.clone()).unwrap();

        let loader = SearchParameterLoader::new(FhirVersion::default());
        let definition = loader
            .parse_resource(&json!({
                "resourceType": "SearchParameter",
                "id": "es-shared-param",
                "url": "http://example.org/fhir/SearchParameter/es-shared-param",
                "name": "EsSharedParam",
                "status": "active",
                "code": "es-shared-code",
                "base": ["Patient"],
                "type": "token",
                "expression": "Patient.identifier"
            }))
            .expect("parse shared SearchParameter definition");

        // A param added to the shared base is visible to the ES backend through
        // the shared container (the first per-tenant build clones the base).
        shared
            .base()
            .write()
            .register(definition)
            .expect("register shared SearchParameter");

        let registry = backend.tenant_registries().for_tenant("default");
        assert!(
            registry
                .read()
                .get_param("Patient", "es-shared-code")
                .is_some(),
            "shared base updates should be visible to the Elasticsearch backend"
        );
    }
}
