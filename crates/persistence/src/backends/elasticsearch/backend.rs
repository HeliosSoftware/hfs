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

/// The `refresh` query parameter applied to Elasticsearch index and delete operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteRefreshPolicy {
    /// Do not refresh on write (Elasticsearch default behavior).
    #[default]
    False,
    /// Wait for the next refresh to make the write visible before returning.
    WaitFor,
    /// Force a refresh of the affected shards immediately after the write.
    True,
}

impl WriteRefreshPolicy {
    pub(crate) fn as_refresh_param(self) -> Option<elasticsearch::params::Refresh> {
        match self {
            WriteRefreshPolicy::False => None,
            WriteRefreshPolicy::WaitFor => Some(elasticsearch::params::Refresh::WaitFor),
            WriteRefreshPolicy::True => Some(elasticsearch::params::Refresh::True),
        }
    }
}

impl std::str::FromStr for WriteRefreshPolicy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "false" => Ok(WriteRefreshPolicy::False),
            "wait_for" | "wait-for" => Ok(WriteRefreshPolicy::WaitFor),
            "true" => Ok(WriteRefreshPolicy::True),
            other => Err(format!(
                "Invalid Elasticsearch write refresh policy '{}'. Valid values: false, wait_for, true",
                other
            )),
        }
    }
}

impl std::fmt::Display for WriteRefreshPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteRefreshPolicy::False => write!(f, "false"),
            WriteRefreshPolicy::WaitFor => write!(f, "wait_for"),
            WriteRefreshPolicy::True => write!(f, "true"),
        }
    }
}

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

    /// Refresh behavior for index/delete operations (default: [`WriteRefreshPolicy::False`]).
    #[serde(default)]
    pub write_refresh: WriteRefreshPolicy,

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
            write_refresh: WriteRefreshPolicy::default(),
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
    /// The capabilities this backend declares.
    ///
    /// Invariant across every valid configuration, so it is exposed as a
    /// constructor-free associated function. Both [`Backend::supports`] and
    /// [`Backend::capabilities`] delegate here so the two answers cannot drift
    /// apart.
    ///
    /// # Tenancy
    ///
    /// `SharedSchema`, and the reasoning is worth recording because this
    /// backend looks superficially like a per-tenant topology.
    /// [`index_name`](Self::index_name) does give each tenant its own index
    /// (`{prefix}_{tenant}_{type}`), but every document also carries a
    /// `tenant_id` keyword field that every query filters on with a `term`
    /// clause. More importantly the separation is a *naming* convention within
    /// a single cluster reached by a single credential, with no per-index
    /// policy, quota, or storage boundary in play — a logical partition, not a
    /// physical one. It therefore declares `SharedSchema`, the weaker of the
    /// two candidate claims.
    ///
    /// Whether the enum should grow a "namespace-per-tenant within one service"
    /// topology that fits this case (and S3 `BucketPerTenant`) more precisely is
    /// tracked as a follow-up to issue #369; it is deliberately not decided
    /// here, since doing so would *strengthen* an isolation claim.
    pub fn declared_capabilities() -> Vec<BackendCapability> {
        vec![
            BackendCapability::Crud,
            BackendCapability::BasicSearch,
            BackendCapability::DateSearch,
            BackendCapability::QuantitySearch,
            BackendCapability::ReferenceSearch,
            BackendCapability::ChainedSearch,
            BackendCapability::ReverseChaining,
            BackendCapability::FullTextSearch,
            BackendCapability::Sorting,
            BackendCapability::CursorPagination,
            BackendCapability::OffsetPagination,
            BackendCapability::Include,
            BackendCapability::Revinclude,
            BackendCapability::SharedSchema,
        ]
    }

    /// Creates a new Elasticsearch backend with the given configuration.
    pub fn new(config: ElasticsearchConfig) -> StorageResult<Self> {
        Self::validate_config(&config)?;
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
        Self::validate_config(&config)?;
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

    pub(crate) fn write_refresh_param(&self) -> Option<elasticsearch::params::Refresh> {
        self.config.write_refresh.as_refresh_param()
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

    /// Validates configuration that the index-name legality proof depends on.
    ///
    /// [`super::naming`] guarantees the *tenant segment* is Elasticsearch-legal,
    /// but two of Elasticsearch's rules constrain the name as a whole: it must
    /// not begin with `-`, `_` or `+`, and must not be `.` or `..`. Both are
    /// discharged by the index prefix, since the prefix always comes first — so
    /// the prefix itself has to be well-formed. Checked once here rather than on
    /// every derivation.
    fn validate_config(config: &ElasticsearchConfig) -> StorageResult<()> {
        super::naming::validate_index_prefix(&config.index_prefix).map_err(|message| {
            crate::error::StorageError::Backend(BackendError::Internal {
                backend_name: "elasticsearch".to_string(),
                message,
                source: None,
            })
        })
    }

    /// Returns the index name for a tenant and resource type.
    ///
    /// Delegates to [`naming::index_name`](super::naming::index_name), the single
    /// injective tenant → index derivation in this backend. It used to lowercase
    /// the tenant id inline, which made tenants `ACME` and `acme` share an index
    /// and every `_id`-addressed write cross the tenant boundary (issue #384).
    pub fn index_name(&self, tenant_id: &str, resource_type: &str) -> String {
        super::naming::index_name(&self.config.index_prefix, tenant_id, resource_type)
    }

    /// Returns the glob matching every index belonging to `tenant_id`.
    ///
    /// Shares [`index_name`](Self::index_name)'s encoder by construction, so the
    /// glob can never stop matching the indices that exist. Four call sites used
    /// to hand-roll `format!("{prefix}_{tenant.to_lowercase()}_*")` instead; if
    /// one of them had drifted from the exact name, `purge_tenant_data` would
    /// have deleted nothing while reporting success.
    pub(crate) fn tenant_index_pattern(&self, tenant_id: &str) -> String {
        super::naming::tenant_index_pattern(&self.config.index_prefix, tenant_id)
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
        Self::declared_capabilities().contains(&capability)
    }

    fn capabilities(&self) -> Vec<BackendCapability> {
        Self::declared_capabilities()
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
            SearchParamType::Composite | SearchParamType::Special => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_is_not_advertised_for_composite_or_special_parameters() {
        assert!(ElasticsearchBackend::modifiers_for_type(SearchParamType::Composite).is_empty());
        assert!(ElasticsearchBackend::modifiers_for_type(SearchParamType::Special).is_empty());
        assert!(
            ElasticsearchBackend::modifiers_for_type(SearchParamType::String).contains(&"missing")
        );
    }

    #[test]
    fn test_config_defaults() {
        let config = ElasticsearchConfig::default();
        assert_eq!(config.index_prefix, "hfs");
        assert_eq!(config.number_of_shards, 1);
        assert_eq!(config.number_of_replicas, 1);
        assert_eq!(config.nodes, vec!["http://localhost:9200"]);
    }

    /// The method-level view of the injective derivation. The exhaustive
    /// property tests (injectivity over an adversarial corpus, Elasticsearch
    /// legality, round-tripping, glob/name agreement) live in
    /// [`super::super::naming`]; this asserts only that the backend method is
    /// actually wired to them.
    ///
    /// The test this replaces asserted `index_name("ACME", …) == "hfs_acme_…"`
    /// — i.e. it pinned the #384 defect as intended behaviour.
    #[test]
    fn index_name_delegates_to_the_injective_derivation() {
        let config = ElasticsearchConfig::default();
        let backend = ElasticsearchBackend::new(config).unwrap();
        // Identity on an already-safe id: conforming deployments see no rename.
        assert_eq!(backend.index_name("acme", "Patient"), "hfs_acme_patient");
        // Case variants must NOT share an index (issue #384).
        assert_ne!(
            backend.index_name("ACME", "Observation"),
            backend.index_name("acme", "Observation")
        );
        // The glob is derived from the same encoder as the exact name.
        assert_eq!(backend.tenant_index_pattern("acme"), "hfs_acme_*");
        assert_ne!(
            backend.tenant_index_pattern("ACME"),
            backend.tenant_index_pattern("acme")
        );
    }

    /// `document_id` deliberately carries **no** tenant component, and this test
    /// is the alarm if someone adds one.
    ///
    /// It is unnecessary: an Elasticsearch `_id` is unique only within its index,
    /// and an injective `index_name` means every index belongs to exactly one
    /// tenant — so no two tenants can ever contend for an `_id`. It would also be
    /// actively harmful: changing `_id` changes the address of every document in
    /// every deployment, including the conforming ones, and because `delete`
    /// removes only the new `_id`, pre-upgrade documents would linger as
    /// permanently-undeletable duplicate search hits. Issue #384 proposes this
    /// change; it was considered and rejected for those reasons.
    #[test]
    fn document_id_carries_no_tenant_component() {
        assert_eq!(
            ElasticsearchBackend::document_id("Patient", "123"),
            "Patient_123"
        );
    }

    /// The prefix is what keeps an index name from starting with a character
    /// Elasticsearch reserves, so a bad one must fail at construction rather than
    /// on the first write.
    #[test]
    fn construction_rejects_an_index_prefix_that_breaks_name_legality() {
        for bad in ["", "_hfs", "-hfs", "+hfs", "HFS", "hfs/prod"] {
            let config = ElasticsearchConfig {
                index_prefix: bad.to_string(),
                ..ElasticsearchConfig::default()
            };
            assert!(
                ElasticsearchBackend::new(config).is_err(),
                "index prefix {bad:?} must be rejected"
            );
        }
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
        // Chained/`_has` search resolves via the backend-agnostic resolver over
        // ES's `search()` (which honours `_id`), so ES declares it.
        assert!(backend.supports(BackendCapability::ChainedSearch));
        assert!(backend.supports(BackendCapability::ReverseChaining));
        // No history/versioning on ES, and never a tenancy topology beyond
        // shared-schema.
        assert!(!backend.supports(BackendCapability::Versioning));
        assert!(!backend.supports(BackendCapability::SchemaPerTenant));
        assert!(!backend.supports(BackendCapability::DatabasePerTenant));
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
