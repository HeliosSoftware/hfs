//! CompositeStorage implementation.
//!
//! This module provides the main `CompositeStorage` struct that coordinates
//! multiple backends for FHIR resource storage and querying.
//!
//! # Overview
//!
//! `CompositeStorage` implements all storage traits by delegating to appropriate
//! backends based on operation type:
//!
//! - **Writes (CRUD)**: Always go to the primary backend
//! - **Reads**: Go to primary, with optional secondary enrichment
//! - **Search**: Routed based on query features to optimal backends
//!
//! # Example
//!
//! ```ignore
//! use helios_persistence::composite::{CompositeStorage, CompositeConfig};
//!
//! let config = CompositeConfig::builder()
//!     .primary("sqlite", BackendKind::Sqlite)
//!     .search_backend("es", BackendKind::Elasticsearch)
//!     .build()?;
//!
//! let storage = CompositeStorage::new(config, backends).await?;
//!
//! // All CRUD goes to primary (sqlite)
//! let patient = storage.create(&tenant, "Patient", patient_json).await?;
//!
//! // Search is routed based on features
//! // - Basic search → sqlite
//! // - Full-text (_text) → elasticsearch
//! let results = storage.search(&tenant, &query).await?;
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use helios_fhir::FhirVersion;
use parking_lot::RwLock;
use serde_json::Value;
use tracing::{debug, instrument, warn};

use crate::core::history::HistoryParams;
use crate::core::{
    BundleEntry, BundleProvider, BundleResult, CapabilityProvider, ChainedSearchProvider,
    ConditionalCreateResult, ConditionalDeleteResult, ConditionalPatchResult, ConditionalStorage,
    ConditionalUpdateResult, IncludeProvider, InstanceHistoryProvider, PatchFormat,
    ResourceStorage, RevincludeProvider, SearchProvider, SearchResult, StorageCapabilities,
    TerminologySearchProvider, TextSearchProvider, VersionedStorage,
};
use crate::error::{BackendError, StorageError, StorageResult, TransactionError};
use crate::tenant::TenantContext;
use crate::types::{
    IncludeDirective, Pagination, ReverseChainedParameter, SearchParamType, SearchParameter,
    SearchQuery, SearchValue, StoredResource,
};

use super::config::{CompositeConfig, SyncMode};
use super::merger::{MergeOptions, ResultMerger};
use super::router::{QueryRouter, RoutingDecision, RoutingError};
use super::sync::{SyncEvent, SyncManager};

/// A dynamically typed storage backend.
pub type DynStorage = Arc<dyn ResourceStorage + Send + Sync>;

/// A dynamically typed search provider.
pub type DynSearchProvider = Arc<dyn SearchProvider + Send + Sync>;

/// A dynamically typed conditional storage provider.
pub type DynConditionalStorage = Arc<dyn ConditionalStorage + Send + Sync>;

/// A dynamically typed versioned storage provider.
pub type DynVersionedStorage = Arc<dyn VersionedStorage + Send + Sync>;

/// A dynamically typed instance history provider.
pub type DynInstanceHistoryProvider = Arc<dyn InstanceHistoryProvider + Send + Sync>;

/// A dynamically typed bundle provider.
pub type DynBundleProvider = Arc<dyn BundleProvider + Send + Sync>;

/// Composite storage that coordinates multiple backends.
///
/// This is the main entry point for polyglot persistence. It implements
/// all storage traits and routes operations to appropriate backends.
///
/// # Full Primary Support
///
/// When the primary backend supports advanced traits (versioning, conditional
/// operations, history, bundles), use [`with_full_primary()`](Self::with_full_primary)
/// to enable delegation of these operations through the composite layer.
pub struct CompositeStorage {
    /// Configuration.
    config: CompositeConfig,

    /// Primary storage backend.
    primary: DynStorage,

    /// Secondary backends by ID.
    secondaries: HashMap<String, DynStorage>,

    /// Search providers by backend ID.
    search_providers: HashMap<String, DynSearchProvider>,

    /// Query router.
    router: QueryRouter,

    /// Result merger.
    merger: ResultMerger,

    /// Synchronization manager.
    sync_manager: Option<SyncManager>,

    /// Backend health status.
    health_status: Arc<RwLock<HashMap<String, BackendHealth>>>,

    // Typed trait objects for primary's advanced capabilities.
    // These are set via `with_full_primary()` to support delegation.
    /// Primary as ConditionalStorage (if supported).
    conditional_storage: Option<DynConditionalStorage>,

    /// Primary as VersionedStorage (if supported).
    versioned_storage: Option<DynVersionedStorage>,

    /// Primary as InstanceHistoryProvider (if supported).
    history_provider: Option<DynInstanceHistoryProvider>,

    /// Primary as BundleProvider (if supported).
    bundle_provider: Option<DynBundleProvider>,
}

/// Health status for a backend.
#[derive(Debug, Clone)]
pub struct BackendHealth {
    /// Whether the backend is healthy.
    pub healthy: bool,

    /// Last successful operation timestamp.
    pub last_success: Option<std::time::Instant>,

    /// Consecutive failure count.
    pub failure_count: u32,

    /// Last error message.
    pub last_error: Option<String>,
}

impl Default for BackendHealth {
    fn default() -> Self {
        Self {
            healthy: true,
            last_success: None,
            failure_count: 0,
            last_error: None,
        }
    }
}

impl CompositeStorage {
    fn has_dedicated_search_backend(&self) -> bool {
        self.config
            .backends_with_role(super::config::BackendRole::Search)
            .next()
            .is_some()
    }

    fn parse_simple_search_params(params: &str) -> Vec<(String, String)> {
        params
            .split('&')
            .filter_map(|pair| {
                let parts: Vec<&str> = pair.splitn(2, '=').collect();
                if parts.len() == 2 {
                    Some((parts[0].to_string(), parts[1].to_string()))
                } else {
                    None
                }
            })
            .collect()
    }

    fn infer_conditional_param_type(name: &str) -> SearchParamType {
        match name {
            "_id" => SearchParamType::Token,
            "_lastUpdated" => SearchParamType::Date,
            "_tag" | "_profile" | "_security" | "identifier" => SearchParamType::Token,
            "patient" | "subject" | "encounter" | "performer" | "author" | "requester"
            | "recorder" | "asserter" | "practitioner" | "organization" | "location" | "device" => {
                SearchParamType::Reference
            }
            _ => SearchParamType::String,
        }
    }

    async fn find_conditional_matches(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
    ) -> StorageResult<Vec<StoredResource>> {
        let parsed_params = Self::parse_simple_search_params(search_params);
        if parsed_params.is_empty() {
            return Ok(Vec::new());
        }

        let mut query = SearchQuery::new(resource_type);
        query.count = Some(1000);
        for (name, value) in parsed_params {
            query = query.with_parameter(SearchParameter {
                name: name.clone(),
                param_type: Self::infer_conditional_param_type(&name),
                modifier: None,
                values: vec![SearchValue::parse(&value)],
                chain: vec![],
                components: vec![],
            });
        }

        let result = self.search(tenant, &query).await?;
        Ok(result.resources.items)
    }

    /// Creates a new composite storage with the given configuration and backends.
    ///
    /// # Arguments
    ///
    /// * `config` - The composite storage configuration
    /// * `backends` - Map of backend ID to storage implementation
    ///
    /// # Errors
    ///
    /// Returns an error if the primary backend is not found in the backends map.
    pub fn new(
        config: CompositeConfig,
        backends: HashMap<String, DynStorage>,
    ) -> StorageResult<Self> {
        let primary_id = config.primary_id().ok_or_else(|| {
            StorageError::Backend(BackendError::Unavailable {
                backend_name: "primary".to_string(),
                message: "No primary backend configured".to_string(),
            })
        })?;

        let primary = backends.get(primary_id).cloned().ok_or_else(|| {
            StorageError::Backend(BackendError::Unavailable {
                backend_name: primary_id.to_string(),
                message: format!("Primary backend '{}' not found in backends map", primary_id),
            })
        })?;

        // Separate out secondaries
        let secondaries: HashMap<_, _> = backends
            .iter()
            .filter(|(id, _)| *id != primary_id)
            .map(|(id, backend)| (id.clone(), backend.clone()))
            .collect();

        // Initialize health status
        let mut health_status = HashMap::new();
        health_status.insert(primary_id.to_string(), BackendHealth::default());
        for id in secondaries.keys() {
            health_status.insert(id.clone(), BackendHealth::default());
        }

        let router = QueryRouter::new(config.clone());
        let merger = ResultMerger::new();

        // Create sync manager if we have secondaries
        let sync_manager = if !secondaries.is_empty() {
            Some(SyncManager::new(config.sync_config.clone()))
        } else {
            None
        };

        Ok(Self {
            config,
            primary,
            secondaries,
            search_providers: HashMap::new(),
            router,
            merger,
            sync_manager,
            health_status: Arc::new(RwLock::new(health_status)),
            conditional_storage: None,
            versioned_storage: None,
            history_provider: None,
            bundle_provider: None,
        })
    }

    /// Creates a composite storage with search providers.
    ///
    /// Search providers allow specialized search backends like Elasticsearch.
    pub fn with_search_providers(mut self, providers: HashMap<String, DynSearchProvider>) -> Self {
        self.search_providers = providers;
        self
    }

    /// Starts the background sync worker for secondary backends.
    ///
    /// Must be called from within a Tokio runtime. When the sync mode is
    /// `Asynchronous` or `Hybrid`, this spawns a background task that
    /// processes sync events without blocking write requests. Without this,
    /// every write falls back to synchronous secondary indexing which can
    /// cause timeouts on large transaction bundles.
    pub fn start_sync_workers(mut self) -> Self {
        if let Some(ref mut manager) = self.sync_manager {
            if matches!(
                self.config.sync_config.mode,
                SyncMode::Asynchronous | SyncMode::Hybrid { .. }
            ) {
                manager.start_async_worker(self.secondaries.clone());
            }
        }
        self
    }

    /// Registers the primary backend's advanced capabilities for delegation.
    ///
    /// When the primary backend implements traits beyond `ResourceStorage`
    /// (e.g., `ConditionalStorage`, `VersionedStorage`, `InstanceHistoryProvider`,
    /// `BundleProvider`), this method stores typed references so that
    /// `CompositeStorage` can delegate these operations to the primary.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let sqlite = Arc::new(SqliteBackend::new("fhir.db")?);
    /// let composite = CompositeStorage::new(config, backends)?
    ///     .with_full_primary(sqlite.clone());
    /// ```
    pub fn with_full_primary<T>(mut self, primary: Arc<T>) -> Self
    where
        T: ResourceStorage
            + ConditionalStorage
            + VersionedStorage
            + InstanceHistoryProvider
            + BundleProvider
            + Send
            + Sync
            + 'static,
    {
        self.conditional_storage = Some(primary.clone() as DynConditionalStorage);
        self.versioned_storage = Some(primary.clone() as DynVersionedStorage);
        self.history_provider = Some(primary.clone() as DynInstanceHistoryProvider);
        self.bundle_provider = Some(primary as DynBundleProvider);
        self
    }

    /// Returns the configuration.
    pub fn config(&self) -> &CompositeConfig {
        &self.config
    }

    /// Returns the primary backend.
    pub fn primary(&self) -> &DynStorage {
        &self.primary
    }

    /// Returns a secondary backend by ID.
    pub fn secondary(&self, id: &str) -> Option<&DynStorage> {
        self.secondaries.get(id)
    }

    /// Returns all secondary backends.
    pub fn secondaries(&self) -> &HashMap<String, DynStorage> {
        &self.secondaries
    }

    /// Returns the health status for a backend.
    pub fn backend_health(&self, id: &str) -> Option<BackendHealth> {
        self.health_status.read().get(id).cloned()
    }

    /// Returns true if a backend is healthy.
    pub fn is_backend_healthy(&self, id: &str) -> bool {
        self.health_status
            .read()
            .get(id)
            .map(|h| h.healthy)
            .unwrap_or(false)
    }

    /// Updates health status after an operation.
    fn update_health(&self, backend_id: &str, success: bool, error: Option<String>) {
        let mut status = self.health_status.write();
        if let Some(health) = status.get_mut(backend_id) {
            if success {
                health.healthy = true;
                health.last_success = Some(std::time::Instant::now());
                health.failure_count = 0;
                health.last_error = None;
            } else {
                health.failure_count += 1;
                health.last_error = error;

                // Mark unhealthy after threshold failures
                if health.failure_count >= self.config.health_config.failure_threshold {
                    health.healthy = false;
                    warn!(
                        backend_id = backend_id,
                        failures = health.failure_count,
                        "Backend marked unhealthy"
                    );
                }
            }
        }
    }

    /// Synchronizes a resource change to secondary backends.
    async fn sync_to_secondaries(&self, event: SyncEvent) -> StorageResult<()> {
        if let Some(ref sync_manager) = self.sync_manager {
            sync_manager.sync(&event, &self.secondaries).await?;
        }
        Ok(())
    }

    /// Routes and executes a search query.
    #[instrument(skip(self, tenant, query), fields(resource_type = %query.resource_type))]
    async fn execute_routed_search(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        // Route the query
        let decision = self
            .router
            .route(query)
            .map_err(|e| self.routing_error_to_storage_error(e))?;

        debug!(
            primary = %decision.primary_target,
            auxiliary_count = decision.auxiliary_targets.len(),
            merge_strategy = ?decision.merge_strategy,
            "Routing query"
        );

        // If no auxiliary backends, just execute on primary
        if decision.auxiliary_targets.is_empty() {
            return self.execute_primary_search(tenant, query).await;
        }

        // Execute on all backends in parallel
        let (primary_result, auxiliary_results) = self
            .execute_parallel_search(tenant, query, &decision)
            .await?;

        // Merge results
        let merge_options = MergeOptions {
            strategy: decision.merge_strategy,
            preserve_primary_order: true,
            deduplicate: true,
        };

        self.merger
            .merge(primary_result, auxiliary_results, merge_options)
    }

    /// Executes search on the primary backend.
    ///
    /// When a dedicated Search backend (e.g., Elasticsearch) is configured, all
    /// searches are routed there. This is necessary because the primary backend
    /// may have search indexing disabled (`search_offloaded = true`), leaving its
    /// search index empty.
    async fn execute_primary_search(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        // Prefer the Search backend when one is configured, since the primary
        // may have offloaded search indexing to it.
        if let Some(search_backend) = self
            .config
            .backends_with_role(super::config::BackendRole::Search)
            .next()
        {
            if let Some(provider) = self.search_providers.get(&search_backend.id) {
                let result = provider.search(tenant, query).await;
                self.update_health(
                    &search_backend.id,
                    result.is_ok(),
                    result.as_ref().err().map(|e| e.to_string()),
                );
                return result;
            }
        }

        // Fall back to primary
        let primary_id = self.config.primary_id().unwrap_or("primary");

        if let Some(provider) = self.search_providers.get(primary_id) {
            let result = provider.search(tenant, query).await;
            self.update_health(
                primary_id,
                result.is_ok(),
                result.as_ref().err().map(|e| e.to_string()),
            );
            result
        } else {
            Err(StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: primary_id.to_string(),
                capability: "SearchProvider".to_string(),
            }))
        }
    }

    /// Executes search on primary and auxiliary backends in parallel.
    async fn execute_parallel_search(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
        decision: &RoutingDecision,
    ) -> StorageResult<(SearchResult, Vec<(String, SearchResult)>)> {
        use tokio::task::JoinSet;

        let mut tasks: JoinSet<(String, StorageResult<SearchResult>)> = JoinSet::new();

        // Clone what we need for async tasks
        let tenant = tenant.clone();
        let query = query.clone();
        let primary_id = decision.primary_target.clone();

        // Start primary search
        if let Some(provider) = self.search_providers.get(&primary_id).cloned() {
            let t = tenant.clone();
            let q = query.clone();
            let id = primary_id.clone();
            tasks.spawn(async move {
                let result = provider.search(&t, &q).await;
                (id, result)
            });
        }

        // Start auxiliary searches
        for (feature, backend_id) in &decision.auxiliary_targets {
            if let Some(provider) = self.search_providers.get(backend_id).cloned() {
                // Create a modified query with only the relevant parameters
                let part_params = decision
                    .analysis
                    .feature_params
                    .get(feature)
                    .cloned()
                    .unwrap_or_default();

                let mut aux_query = SearchQuery::new(&query.resource_type);
                for param in part_params {
                    aux_query = aux_query.with_parameter(param);
                }
                aux_query.count = query.count;
                aux_query.offset = query.offset;
                aux_query.cursor = query.cursor.clone();

                let t = tenant.clone();
                let id = backend_id.clone();
                tasks.spawn(async move {
                    let result = provider.search(&t, &aux_query).await;
                    (id, result)
                });
            }
        }

        // Collect results
        let mut primary_result: Option<SearchResult> = None;
        let mut primary_unsupported = false;
        let mut auxiliary_results = Vec::new();

        while let Some(result) = tasks.join_next().await {
            match result {
                Ok((id, search_result)) => {
                    self.update_health(
                        &id,
                        search_result.is_ok(),
                        search_result.as_ref().err().map(|e| e.to_string()),
                    );

                    if id == primary_id {
                        match search_result {
                            Ok(r) => primary_result = Some(r),
                            Err(StorageError::Backend(BackendError::UnsupportedCapability {
                                ..
                            })) => {
                                // Primary doesn't support this search feature (e.g. S3 has no
                                // full-text search). An auxiliary backend (e.g. Elasticsearch)
                                // will handle it — promote its result to primary below.
                                primary_unsupported = true;
                            }
                            Err(e) => return Err(e),
                        }
                    } else if let Ok(res) = search_result {
                        auxiliary_results.push((id, res));
                    }
                    // Ignore other auxiliary failures - graceful degradation
                }
                Err(e) => {
                    warn!(error = %e, "Task join error during parallel search");
                }
            }
        }

        // When primary lacks search capability and auxiliary has results, promote the
        // first auxiliary result to primary so the merger can return it directly.
        if primary_unsupported && primary_result.is_none() {
            if !auxiliary_results.is_empty() {
                let (_, promoted) = auxiliary_results.remove(0);
                return Ok((promoted, auxiliary_results));
            }
            return Err(StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: primary_id,
                capability: "search".to_string(),
            }));
        }

        let primary = primary_result.ok_or_else(|| {
            StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: primary_id,
                message: "Primary search task failed".to_string(),
            })
        })?;

        Ok((primary, auxiliary_results))
    }

    /// Syncs bundle results to secondaries by extracting resource info from responses.
    async fn sync_bundle_results(&self, tenant: &TenantContext, result: &BundleResult) {
        for entry_result in &result.entries {
            // Only sync successful mutating operations that have a resource body
            if let Some(ref resource_json) = entry_result.resource {
                let resource_type = resource_json
                    .get("resourceType")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let resource_id = resource_json
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                if resource_type.is_empty() || resource_id.is_empty() {
                    continue;
                }

                let fhir_version = resource_json
                    .get("meta")
                    .and_then(|m| m.get("profile"))
                    .map(|_| FhirVersion::default())
                    .unwrap_or_default();

                if let Err(e) = self
                    .sync_to_secondaries(SyncEvent::Create {
                        resource_type: resource_type.to_string(),
                        resource_id: resource_id.to_string(),
                        content: resource_json.clone(),
                        tenant_id: tenant.tenant_id().clone(),
                        fhir_version,
                    })
                    .await
                {
                    warn!(
                        error = %e,
                        resource_type = resource_type,
                        resource_id = resource_id,
                        "Failed to sync bundle entry to secondaries"
                    );
                }
            }
        }
    }

    /// Converts a routing error to a storage error.
    fn routing_error_to_storage_error(&self, err: RoutingError) -> StorageError {
        match err {
            RoutingError::NoPrimaryBackend => StorageError::Backend(BackendError::Unavailable {
                backend_name: "primary".to_string(),
                message: "No primary backend configured".to_string(),
            }),
            RoutingError::NoCapableBackend { feature } => {
                StorageError::Backend(BackendError::UnsupportedCapability {
                    backend_name: "composite".to_string(),
                    capability: format!("{:?}", feature),
                })
            }
            RoutingError::BackendUnavailable { backend_id } => {
                StorageError::Backend(BackendError::ConnectionFailed {
                    backend_name: backend_id,
                    message: "Backend unavailable".to_string(),
                })
            }
        }
    }
}

#[async_trait]
impl ResourceStorage for CompositeStorage {
    fn backend_name(&self) -> &'static str {
        "composite"
    }

    #[instrument(skip(self, tenant, resource), fields(resource_type = %resource_type))]
    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        // All writes go to primary
        let result = self
            .primary
            .create(tenant, resource_type, resource.clone(), fhir_version)
            .await;

        let primary_id = self.config.primary_id().unwrap_or("primary");
        self.update_health(
            primary_id,
            result.is_ok(),
            result.as_ref().err().map(|e| e.to_string()),
        );

        let stored = result?;

        // Sync to secondaries
        if let Err(e) = self
            .sync_to_secondaries(SyncEvent::Create {
                resource_type: resource_type.to_string(),
                resource_id: stored.id().to_string(),
                content: stored.content().clone(),
                tenant_id: tenant.tenant_id().clone(),
                fhir_version,
            })
            .await
        {
            warn!(error = %e, "Failed to sync create to secondaries");
            // Don't fail the operation - primary succeeded
        }

        Ok(stored)
    }

    #[instrument(skip(self, tenant, resource), fields(resource_type = %resource_type, id = %id))]
    async fn create_or_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        let result = self
            .primary
            .create_or_update(tenant, resource_type, id, resource.clone(), fhir_version)
            .await;

        let primary_id = self.config.primary_id().unwrap_or("primary");
        self.update_health(
            primary_id,
            result.is_ok(),
            result.as_ref().err().map(|e| e.to_string()),
        );

        let (stored, created) = result?;

        // Sync to secondaries
        let event = if created {
            SyncEvent::Create {
                resource_type: resource_type.to_string(),
                resource_id: id.to_string(),
                content: stored.content().clone(),
                tenant_id: tenant.tenant_id().clone(),
                fhir_version,
            }
        } else {
            SyncEvent::Update {
                resource_type: resource_type.to_string(),
                resource_id: id.to_string(),
                content: stored.content().clone(),
                tenant_id: tenant.tenant_id().clone(),
                version: stored.version_id().to_string(),
                fhir_version,
            }
        };

        if let Err(e) = self.sync_to_secondaries(event).await {
            warn!(error = %e, "Failed to sync create_or_update to secondaries");
        }

        Ok((stored, created))
    }

    #[instrument(skip(self, tenant), fields(resource_type = %resource_type, id = %id))]
    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        // Reads always go to primary (source of truth)
        let result = self.primary.read(tenant, resource_type, id).await;

        let primary_id = self.config.primary_id().unwrap_or("primary");
        self.update_health(
            primary_id,
            result.is_ok(),
            result.as_ref().err().map(|e| e.to_string()),
        );

        result
    }

    #[instrument(skip(self, tenant, resource), fields(resource_type = %current.resource_type(), id = %current.id()))]
    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        let result = self.primary.update(tenant, current, resource.clone()).await;

        let primary_id = self.config.primary_id().unwrap_or("primary");
        self.update_health(
            primary_id,
            result.is_ok(),
            result.as_ref().err().map(|e| e.to_string()),
        );

        let stored = result?;

        // Sync to secondaries
        if let Err(e) = self
            .sync_to_secondaries(SyncEvent::Update {
                resource_type: current.resource_type().to_string(),
                resource_id: current.id().to_string(),
                content: stored.content().clone(),
                tenant_id: tenant.tenant_id().clone(),
                version: stored.version_id().to_string(),
                fhir_version: stored.fhir_version(),
            })
            .await
        {
            warn!(error = %e, "Failed to sync update to secondaries");
        }

        Ok(stored)
    }

    #[instrument(skip(self, tenant), fields(resource_type = %resource_type, id = %id))]
    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        let result = self.primary.delete(tenant, resource_type, id).await;

        let primary_id = self.config.primary_id().unwrap_or("primary");
        self.update_health(
            primary_id,
            result.is_ok(),
            result.as_ref().err().map(|e| e.to_string()),
        );

        result?;

        // Sync to secondaries
        if let Err(e) = self
            .sync_to_secondaries(SyncEvent::Delete {
                resource_type: resource_type.to_string(),
                resource_id: id.to_string(),
                tenant_id: tenant.tenant_id().clone(),
            })
            .await
        {
            warn!(error = %e, "Failed to sync delete to secondaries");
        }

        Ok(())
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        self.primary.count(tenant, resource_type).await
    }
}

#[async_trait]
impl SearchProvider for CompositeStorage {
    #[instrument(skip(self, tenant, query), fields(resource_type = %query.resource_type))]
    async fn search(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        self.execute_routed_search(tenant, query).await
    }

    async fn search_count(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<u64> {
        // Prefer dedicated Search backend when configured, matching `search` routing.
        if let Some(search_backend) = self
            .config
            .backends_with_role(super::config::BackendRole::Search)
            .next()
        {
            if let Some(provider) = self.search_providers.get(&search_backend.id) {
                return provider.search_count(tenant, query).await;
            }
        }

        // Fall back to primary provider.
        if let Some(provider) = self
            .search_providers
            .get(self.config.primary_id().unwrap_or("primary"))
        {
            provider.search_count(tenant, query).await
        } else {
            Err(StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: "composite".to_string(),
                capability: "search_count".to_string(),
            }))
        }
    }
}

#[async_trait]
impl ConditionalStorage for CompositeStorage {
    async fn conditional_create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        search_params: &str,
        fhir_version: FhirVersion,
    ) -> StorageResult<ConditionalCreateResult> {
        if self.has_dedicated_search_backend() {
            let matches = self
                .find_conditional_matches(tenant, resource_type, search_params)
                .await?;

            return match matches.len() {
                0 => {
                    let created = self
                        .primary
                        .create(tenant, resource_type, resource, fhir_version)
                        .await?;

                    if let Err(e) = self
                        .sync_to_secondaries(SyncEvent::Create {
                            resource_type: resource_type.to_string(),
                            resource_id: created.id().to_string(),
                            content: created.content().clone(),
                            tenant_id: tenant.tenant_id().clone(),
                            fhir_version,
                        })
                        .await
                    {
                        warn!(error = %e, "Failed to sync conditional_create to secondaries");
                    }

                    Ok(ConditionalCreateResult::Created(created))
                }
                1 => Ok(ConditionalCreateResult::Exists(
                    matches.into_iter().next().expect("single match must exist"),
                )),
                n => Ok(ConditionalCreateResult::MultipleMatches(n)),
            };
        }

        let storage = self.conditional_storage.as_ref().ok_or_else(|| {
            StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: "composite".to_string(),
                capability: "ConditionalStorage".to_string(),
            })
        })?;

        let result = storage
            .conditional_create(tenant, resource_type, resource, search_params, fhir_version)
            .await?;

        // Sync created resource to secondaries
        if let ConditionalCreateResult::Created(ref stored) = result {
            if let Err(e) = self
                .sync_to_secondaries(SyncEvent::Create {
                    resource_type: resource_type.to_string(),
                    resource_id: stored.id().to_string(),
                    content: stored.content().clone(),
                    tenant_id: tenant.tenant_id().clone(),
                    fhir_version,
                })
                .await
            {
                warn!(error = %e, "Failed to sync conditional_create to secondaries");
            }
        }

        Ok(result)
    }

    async fn conditional_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        search_params: &str,
        upsert: bool,
        fhir_version: FhirVersion,
    ) -> StorageResult<ConditionalUpdateResult> {
        if self.has_dedicated_search_backend() {
            let matches = self
                .find_conditional_matches(tenant, resource_type, search_params)
                .await?;

            return match matches.len() {
                0 => {
                    if upsert {
                        let created = self
                            .primary
                            .create(tenant, resource_type, resource, fhir_version)
                            .await?;

                        if let Err(e) = self
                            .sync_to_secondaries(SyncEvent::Create {
                                resource_type: resource_type.to_string(),
                                resource_id: created.id().to_string(),
                                content: created.content().clone(),
                                tenant_id: tenant.tenant_id().clone(),
                                fhir_version,
                            })
                            .await
                        {
                            warn!(
                                error = %e,
                                "Failed to sync conditional_update create to secondaries"
                            );
                        }

                        Ok(ConditionalUpdateResult::Created(created))
                    } else {
                        Ok(ConditionalUpdateResult::NoMatch)
                    }
                }
                1 => {
                    let current = matches.into_iter().next().expect("single match must exist");
                    let updated = self.primary.update(tenant, &current, resource).await?;

                    if let Err(e) = self
                        .sync_to_secondaries(SyncEvent::Update {
                            resource_type: resource_type.to_string(),
                            resource_id: updated.id().to_string(),
                            content: updated.content().clone(),
                            tenant_id: tenant.tenant_id().clone(),
                            version: updated.version_id().to_string(),
                            fhir_version: updated.fhir_version(),
                        })
                        .await
                    {
                        warn!(error = %e, "Failed to sync conditional_update to secondaries");
                    }

                    Ok(ConditionalUpdateResult::Updated(updated))
                }
                n => Ok(ConditionalUpdateResult::MultipleMatches(n)),
            };
        }

        let storage = self.conditional_storage.as_ref().ok_or_else(|| {
            StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: "composite".to_string(),
                capability: "ConditionalStorage".to_string(),
            })
        })?;

        let result = storage
            .conditional_update(
                tenant,
                resource_type,
                resource,
                search_params,
                upsert,
                fhir_version,
            )
            .await?;

        // Sync to secondaries
        match &result {
            ConditionalUpdateResult::Created(stored) => {
                if let Err(e) = self
                    .sync_to_secondaries(SyncEvent::Create {
                        resource_type: resource_type.to_string(),
                        resource_id: stored.id().to_string(),
                        content: stored.content().clone(),
                        tenant_id: tenant.tenant_id().clone(),
                        fhir_version,
                    })
                    .await
                {
                    warn!(error = %e, "Failed to sync conditional_update create to secondaries");
                }
            }
            ConditionalUpdateResult::Updated(stored) => {
                if let Err(e) = self
                    .sync_to_secondaries(SyncEvent::Update {
                        resource_type: resource_type.to_string(),
                        resource_id: stored.id().to_string(),
                        content: stored.content().clone(),
                        tenant_id: tenant.tenant_id().clone(),
                        version: stored.version_id().to_string(),
                        fhir_version: stored.fhir_version(),
                    })
                    .await
                {
                    warn!(error = %e, "Failed to sync conditional_update to secondaries");
                }
            }
            _ => {}
        }

        Ok(result)
    }

    async fn conditional_delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
    ) -> StorageResult<ConditionalDeleteResult> {
        if self.has_dedicated_search_backend() {
            let matches = self
                .find_conditional_matches(tenant, resource_type, search_params)
                .await?;

            return match matches.len() {
                0 => Ok(ConditionalDeleteResult::NoMatch),
                1 => {
                    let current = matches.into_iter().next().expect("single match must exist");
                    self.primary
                        .delete(tenant, resource_type, current.id())
                        .await?;

                    if let Err(e) = self
                        .sync_to_secondaries(SyncEvent::Delete {
                            resource_type: resource_type.to_string(),
                            resource_id: current.id().to_string(),
                            tenant_id: tenant.tenant_id().clone(),
                        })
                        .await
                    {
                        warn!(error = %e, "Failed to sync conditional_delete to secondaries");
                    }

                    Ok(ConditionalDeleteResult::Deleted)
                }
                n => Ok(ConditionalDeleteResult::MultipleMatches(n)),
            };
        }

        let storage = self.conditional_storage.as_ref().ok_or_else(|| {
            StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: "composite".to_string(),
                capability: "ConditionalStorage".to_string(),
            })
        })?;

        let result = storage
            .conditional_delete(tenant, resource_type, search_params)
            .await?;

        // Note: We don't have the resource ID for sync here — the primary already
        // performed the delete. The sync_manager will handle it if configured.

        Ok(result)
    }

    async fn conditional_patch(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
        patch: &PatchFormat,
    ) -> StorageResult<ConditionalPatchResult> {
        let storage = self.conditional_storage.as_ref().ok_or_else(|| {
            StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: "composite".to_string(),
                capability: "ConditionalStorage".to_string(),
            })
        })?;

        let result = storage
            .conditional_patch(tenant, resource_type, search_params, patch)
            .await?;

        // Sync patched resource to secondaries
        if let ConditionalPatchResult::Patched(ref stored) = result {
            if let Err(e) = self
                .sync_to_secondaries(SyncEvent::Update {
                    resource_type: resource_type.to_string(),
                    resource_id: stored.id().to_string(),
                    content: stored.content().clone(),
                    tenant_id: tenant.tenant_id().clone(),
                    version: stored.version_id().to_string(),
                    fhir_version: stored.fhir_version(),
                })
                .await
            {
                warn!(error = %e, "Failed to sync conditional_patch to secondaries");
            }
        }

        Ok(result)
    }
}

#[async_trait]
impl VersionedStorage for CompositeStorage {
    async fn vread(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        version_id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let storage = self.versioned_storage.as_ref().ok_or_else(|| {
            StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: "composite".to_string(),
                capability: "VersionedStorage".to_string(),
            })
        })?;

        storage.vread(tenant, resource_type, id, version_id).await
    }

    async fn update_with_match(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        let storage = self.versioned_storage.as_ref().ok_or_else(|| {
            StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: "composite".to_string(),
                capability: "VersionedStorage".to_string(),
            })
        })?;

        let stored = storage
            .update_with_match(tenant, resource_type, id, expected_version, resource)
            .await?;

        // Sync to secondaries
        if let Err(e) = self
            .sync_to_secondaries(SyncEvent::Update {
                resource_type: resource_type.to_string(),
                resource_id: id.to_string(),
                content: stored.content().clone(),
                tenant_id: tenant.tenant_id().clone(),
                version: stored.version_id().to_string(),
                fhir_version: stored.fhir_version(),
            })
            .await
        {
            warn!(error = %e, "Failed to sync update_with_match to secondaries");
        }

        Ok(stored)
    }

    async fn delete_with_match(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
    ) -> StorageResult<()> {
        let storage = self.versioned_storage.as_ref().ok_or_else(|| {
            StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: "composite".to_string(),
                capability: "VersionedStorage".to_string(),
            })
        })?;

        storage
            .delete_with_match(tenant, resource_type, id, expected_version)
            .await?;

        // Sync to secondaries
        if let Err(e) = self
            .sync_to_secondaries(SyncEvent::Delete {
                resource_type: resource_type.to_string(),
                resource_id: id.to_string(),
                tenant_id: tenant.tenant_id().clone(),
            })
            .await
        {
            warn!(error = %e, "Failed to sync delete_with_match to secondaries");
        }

        Ok(())
    }

    async fn list_versions(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Vec<String>> {
        let storage = self.versioned_storage.as_ref().ok_or_else(|| {
            StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: "composite".to_string(),
                capability: "VersionedStorage".to_string(),
            })
        })?;

        storage.list_versions(tenant, resource_type, id).await
    }
}

#[async_trait]
impl InstanceHistoryProvider for CompositeStorage {
    async fn history_instance(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        params: &HistoryParams,
    ) -> StorageResult<crate::core::HistoryPage> {
        let provider = self.history_provider.as_ref().ok_or_else(|| {
            StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: "composite".to_string(),
                capability: "InstanceHistoryProvider".to_string(),
            })
        })?;

        provider
            .history_instance(tenant, resource_type, id, params)
            .await
    }

    async fn history_instance_count(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<u64> {
        let provider = self.history_provider.as_ref().ok_or_else(|| {
            StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: "composite".to_string(),
                capability: "InstanceHistoryProvider".to_string(),
            })
        })?;

        provider
            .history_instance_count(tenant, resource_type, id)
            .await
    }
}

#[async_trait]
impl BundleProvider for CompositeStorage {
    async fn process_transaction(
        &self,
        tenant: &TenantContext,
        entries: Vec<BundleEntry>,
    ) -> Result<BundleResult, TransactionError> {
        let provider =
            self.bundle_provider
                .as_ref()
                .ok_or_else(|| TransactionError::BundleError {
                    index: 0,
                    message: "BundleProvider not available on composite primary".to_string(),
                })?;

        let result = provider.process_transaction(tenant, entries).await?;

        // Sync successful entries to secondaries by reading resources from primary
        self.sync_bundle_results(tenant, &result).await;

        Ok(result)
    }

    async fn process_batch(
        &self,
        tenant: &TenantContext,
        entries: Vec<BundleEntry>,
    ) -> StorageResult<BundleResult> {
        let provider = self.bundle_provider.as_ref().ok_or_else(|| {
            StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: "composite".to_string(),
                capability: "BundleProvider".to_string(),
            })
        })?;

        let result = provider.process_batch(tenant, entries).await?;

        // Sync successful entries to secondaries
        self.sync_bundle_results(tenant, &result).await;

        Ok(result)
    }
}

#[async_trait]
impl IncludeProvider for CompositeStorage {
    async fn resolve_includes(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        includes: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>> {
        // Include resolution always uses primary (has all resources)
        let primary_id = self.config.primary_id().unwrap_or("primary");

        if let Some(_provider) = self.search_providers.get(primary_id) {
            // Try to downcast to IncludeProvider
            // This is a limitation - we need trait objects
            // For now, fall back to a basic implementation
            self.resolve_includes_basic(tenant, resources, includes)
                .await
        } else {
            self.resolve_includes_basic(tenant, resources, includes)
                .await
        }
    }
}

impl CompositeStorage {
    /// Basic include resolution by reading referenced resources.
    async fn resolve_includes_basic(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        includes: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>> {
        use std::collections::HashSet;

        let mut included = Vec::new();
        let mut seen_ids = HashSet::new();

        for resource in resources {
            for include in includes {
                // Extract references from resource based on search param
                let refs = self.extract_references(resource, &include.search_param);

                for reference in refs {
                    // Parse reference: "ResourceType/id"
                    if let Some((ref_type, ref_id)) = reference.split_once('/') {
                        // Check target type filter
                        if let Some(ref target) = include.target_type {
                            if target != ref_type {
                                continue;
                            }
                        }

                        let key = format!("{}/{}", ref_type, ref_id);
                        if seen_ids.insert(key) {
                            if let Ok(Some(included_resource)) =
                                self.primary.read(tenant, ref_type, ref_id).await
                            {
                                included.push(included_resource);
                            }
                        }
                    }
                }
            }
        }

        Ok(included)
    }

    /// Extracts references from a resource for a given search parameter.
    fn extract_references(&self, resource: &StoredResource, search_param: &str) -> Vec<String> {
        let content = resource.content();
        let mut refs = Vec::new();

        // Simple extraction - looks for the search param as a field
        // A real implementation would use FHIRPath or search parameter definitions
        if let Some(value) = content.get(search_param) {
            Self::extract_reference_values(value, &mut refs);
        }

        // Also check common reference field names
        let field_name = match search_param {
            "patient" | "subject" => Some("subject"),
            "encounter" => Some("encounter"),
            "performer" => Some("performer"),
            _ => None,
        };

        if let Some(field) = field_name {
            if let Some(value) = content.get(field) {
                Self::extract_reference_values(value, &mut refs);
            }
        }

        refs
    }

    /// Recursively extracts reference values.
    fn extract_reference_values(value: &Value, refs: &mut Vec<String>) {
        match value {
            Value::Object(obj) => {
                if let Some(Value::String(reference)) = obj.get("reference") {
                    refs.push(reference.clone());
                }
            }
            Value::Array(arr) => {
                for item in arr {
                    Self::extract_reference_values(item, refs);
                }
            }
            _ => {}
        }
    }
}

#[async_trait]
impl RevincludeProvider for CompositeStorage {
    async fn resolve_revincludes(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        revincludes: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>> {
        // Revinclude resolution - find resources that reference the primary results
        // This typically requires search capability
        let mut revincluded = Vec::new();

        for revinclude in revincludes {
            for resource in resources {
                let reference = format!("{}/{}", resource.resource_type(), resource.id());

                // Search for resources that reference this one
                let query = SearchQuery::new(&revinclude.source_type).with_parameter(
                    crate::types::SearchParameter {
                        name: revinclude.search_param.clone(),
                        param_type: crate::types::SearchParamType::Reference,
                        modifier: None,
                        values: vec![crate::types::SearchValue::eq(&reference)],
                        chain: vec![],
                        components: vec![],
                    },
                );

                if let Ok(result) = self.search(tenant, &query).await {
                    for item in result.resources.items {
                        revincluded.push(item);
                    }
                }
            }
        }

        // Deduplicate
        let mut seen = std::collections::HashSet::new();
        revincluded.retain(|r| seen.insert(format!("{}/{}", r.resource_type(), r.id())));

        Ok(revincluded)
    }
}

#[async_trait]
impl ChainedSearchProvider for CompositeStorage {
    async fn resolve_chain(
        &self,
        tenant: &TenantContext,
        base_type: &str,
        chain: &str,
        value: &str,
    ) -> StorageResult<Vec<String>> {
        // Chain resolution - delegate to graph backend if available
        let graph_backend = self
            .config
            .backends_with_role(super::config::BackendRole::Graph)
            .next();

        if let Some(backend) = graph_backend {
            if let Some(_provider) = self.search_providers.get(&backend.id) {
                // Would need to downcast to ChainedSearchProvider
                // For now, fall back to iterative resolution
            }
        }

        // Fallback: iterative chain resolution
        self.resolve_chain_iterative(tenant, base_type, chain, value)
            .await
    }

    async fn resolve_reverse_chain(
        &self,
        tenant: &TenantContext,
        base_type: &str,
        reverse_chain: &ReverseChainedParameter,
    ) -> StorageResult<Vec<String>> {
        // Find resources of source_type that match the parameter,
        // then return IDs of base_type resources they reference
        let values = match &reverse_chain.value {
            Some(v) => vec![v.clone()],
            None => vec![],
        };
        let query = SearchQuery::new(&reverse_chain.source_type).with_parameter(
            crate::types::SearchParameter {
                name: reverse_chain.search_param.clone(),
                param_type: crate::types::SearchParamType::Token,
                modifier: None,
                values,
                chain: vec![],
                components: vec![],
            },
        );

        let result = self.search(tenant, &query).await?;

        // Extract references to base_type
        let mut ids = Vec::new();
        for resource in result.resources.items {
            let refs = self.extract_references(&resource, &reverse_chain.reference_param);
            for reference in refs {
                if let Some((ref_type, ref_id)) = reference.split_once('/') {
                    if ref_type == base_type {
                        ids.push(ref_id.to_string());
                    }
                }
            }
        }

        Ok(ids)
    }
}

impl CompositeStorage {
    /// Resolves a chain iteratively.
    async fn resolve_chain_iterative(
        &self,
        _tenant: &TenantContext,
        _base_type: &str,
        chain: &str,
        _value: &str,
    ) -> StorageResult<Vec<String>> {
        // Parse chain: "patient.organization.name" -> ["patient", "organization", "name"]
        let parts: Vec<&str> = chain.split('.').collect();

        if parts.is_empty() {
            return Ok(Vec::new());
        }

        // This is a simplified implementation
        // A full implementation would handle multiple chain segments
        // and different parameter types

        // For now, just return empty - this would need FHIRPath evaluation
        Ok(Vec::new())
    }
}

#[async_trait]
impl TerminologySearchProvider for CompositeStorage {
    async fn expand_value_set(&self, _value_set_url: &str) -> StorageResult<Vec<(String, String)>> {
        // Delegate to terminology backend if available
        let term_backend = self
            .config
            .backends_with_role(super::config::BackendRole::Terminology)
            .next();

        if let Some(_backend) = term_backend {
            // Would need to downcast to TerminologySearchProvider
        }

        // Fallback: not supported without terminology service
        Err(StorageError::Backend(BackendError::UnsupportedCapability {
            backend_name: "composite".to_string(),
            capability: "expand_value_set".to_string(),
        }))
    }

    async fn codes_above(&self, _system: &str, _code: &str) -> StorageResult<Vec<String>> {
        Err(StorageError::Backend(BackendError::UnsupportedCapability {
            backend_name: "composite".to_string(),
            capability: "codes_above".to_string(),
        }))
    }

    async fn codes_below(&self, _system: &str, _code: &str) -> StorageResult<Vec<String>> {
        Err(StorageError::Backend(BackendError::UnsupportedCapability {
            backend_name: "composite".to_string(),
            capability: "codes_below".to_string(),
        }))
    }
}

#[async_trait]
impl TextSearchProvider for CompositeStorage {
    async fn search_text(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        text: &str,
        pagination: &Pagination,
    ) -> StorageResult<SearchResult> {
        // Delegate to search backend if available
        let search_backend = self
            .config
            .backends_with_role(super::config::BackendRole::Search)
            .next();

        if let Some(backend) = search_backend {
            if let Some(provider) = self.search_providers.get(&backend.id) {
                // Build a text search query
                let query = SearchQuery::new(resource_type)
                    .with_parameter(crate::types::SearchParameter {
                        name: "_text".to_string(),
                        param_type: crate::types::SearchParamType::String,
                        modifier: None,
                        values: vec![crate::types::SearchValue::string(text)],
                        chain: vec![],
                        components: vec![],
                    })
                    .with_count(pagination.count);

                return provider.search(tenant, &query).await;
            }
        }

        // Fallback to primary (may be less efficient)
        self.execute_primary_search(
            tenant,
            &SearchQuery::new(resource_type)
                .with_parameter(crate::types::SearchParameter {
                    name: "_text".to_string(),
                    param_type: crate::types::SearchParamType::String,
                    modifier: None,
                    values: vec![crate::types::SearchValue::string(text)],
                    chain: vec![],
                    components: vec![],
                })
                .with_count(pagination.count),
        )
        .await
    }

    async fn search_content(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        content: &str,
        pagination: &Pagination,
    ) -> StorageResult<SearchResult> {
        // Similar to search_text but uses _content parameter
        let search_backend = self
            .config
            .backends_with_role(super::config::BackendRole::Search)
            .next();

        if let Some(backend) = search_backend {
            if let Some(provider) = self.search_providers.get(&backend.id) {
                let query = SearchQuery::new(resource_type)
                    .with_parameter(crate::types::SearchParameter {
                        name: "_content".to_string(),
                        param_type: crate::types::SearchParamType::String,
                        modifier: None,
                        values: vec![crate::types::SearchValue::string(content)],
                        chain: vec![],
                        components: vec![],
                    })
                    .with_count(pagination.count);

                return provider.search(tenant, &query).await;
            }
        }

        self.execute_primary_search(
            tenant,
            &SearchQuery::new(resource_type)
                .with_parameter(crate::types::SearchParameter {
                    name: "_content".to_string(),
                    param_type: crate::types::SearchParamType::String,
                    modifier: None,
                    values: vec![crate::types::SearchValue::string(content)],
                    chain: vec![],
                    components: vec![],
                })
                .with_count(pagination.count),
        )
        .await
    }
}

impl CapabilityProvider for CompositeStorage {
    fn capabilities(&self) -> StorageCapabilities {
        use std::collections::HashSet;

        // Merge capabilities from all backends
        let resource_caps = HashMap::new();

        let mut system_interactions = HashSet::new();
        system_interactions.insert(crate::core::SystemInteraction::Transaction);
        system_interactions.insert(crate::core::SystemInteraction::Batch);
        system_interactions.insert(crate::core::SystemInteraction::SearchSystem);
        system_interactions.insert(crate::core::SystemInteraction::HistorySystem);

        StorageCapabilities {
            backend_name: "composite".to_string(),
            backend_version: None,
            resources: resource_caps,
            system_interactions,
            supports_system_history: true,
            supports_system_search: true,
            supported_sorts: vec!["_lastUpdated".to_string(), "_id".to_string()],
            supports_total: true,
            max_page_size: Some(1000),
            default_page_size: 20,
        }
    }

    // resource_capabilities uses the default implementation that returns Option<ResourceCapabilities>
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{BackendKind, CapabilityProvider};
    use crate::error::{BackendError, StorageError, StorageResult};
    use crate::tenant::{TenantContext, TenantId, TenantPermissions};
    use crate::types::{
        SearchParamType, SearchParameter, SearchQuery, SearchValue, StoredResource,
    };
    use async_trait::async_trait;
    use helios_fhir::FhirVersion;
    use serde_json::{Value, json};

    #[derive(Debug)]
    struct FailingSearchBackend {
        backend_name: &'static str,
        error_message: &'static str,
    }

    #[async_trait]
    impl ResourceStorage for FailingSearchBackend {
        fn backend_name(&self) -> &'static str {
            self.backend_name
        }

        async fn create(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<StoredResource> {
            Err(StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: self.backend_name.to_string(),
                capability: "create".to_string(),
            }))
        }

        async fn create_or_update(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<(StoredResource, bool)> {
            Err(StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: self.backend_name.to_string(),
                capability: "create_or_update".to_string(),
            }))
        }

        async fn read(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<Option<StoredResource>> {
            Ok(None)
        }

        async fn update(
            &self,
            _tenant: &TenantContext,
            _current: &StoredResource,
            _resource: Value,
        ) -> StorageResult<StoredResource> {
            Err(StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: self.backend_name.to_string(),
                capability: "update".to_string(),
            }))
        }

        async fn delete(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<()> {
            Err(StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: self.backend_name.to_string(),
                capability: "delete".to_string(),
            }))
        }

        async fn count(
            &self,
            _tenant: &TenantContext,
            _resource_type: Option<&str>,
        ) -> StorageResult<u64> {
            Ok(0)
        }
    }

    #[async_trait]
    impl SearchProvider for FailingSearchBackend {
        async fn search(
            &self,
            _tenant: &TenantContext,
            _query: &SearchQuery,
        ) -> StorageResult<SearchResult> {
            Err(StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: self.backend_name.to_string(),
                message: self.error_message.to_string(),
            }))
        }

        async fn search_count(
            &self,
            _tenant: &TenantContext,
            _query: &SearchQuery,
        ) -> StorageResult<u64> {
            Err(StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: self.backend_name.to_string(),
                message: self.error_message.to_string(),
            }))
        }
    }

    /// Minimal mock storage for unit testing CompositeStorage.
    struct MockStorage;

    #[async_trait]
    impl ResourceStorage for MockStorage {
        fn backend_name(&self) -> &'static str {
            "mock"
        }

        async fn create(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            resource: Value,
            fhir_version: FhirVersion,
        ) -> StorageResult<StoredResource> {
            let id = uuid::Uuid::new_v4().to_string();
            Ok(StoredResource::new(
                resource_type,
                &id,
                tenant.tenant_id().clone(),
                resource,
                fhir_version,
            ))
        }

        async fn create_or_update(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            id: &str,
            resource: Value,
            fhir_version: FhirVersion,
        ) -> StorageResult<(StoredResource, bool)> {
            Ok((
                StoredResource::new(
                    resource_type,
                    id,
                    tenant.tenant_id().clone(),
                    resource,
                    fhir_version,
                ),
                true,
            ))
        }

        async fn read(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<Option<StoredResource>> {
            Ok(None)
        }

        async fn update(
            &self,
            tenant: &TenantContext,
            current: &StoredResource,
            resource: Value,
        ) -> StorageResult<StoredResource> {
            Ok(StoredResource::new(
                current.resource_type(),
                current.id(),
                tenant.tenant_id().clone(),
                resource,
                current.fhir_version(),
            ))
        }

        async fn delete(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<()> {
            Ok(())
        }

        async fn count(
            &self,
            _tenant: &TenantContext,
            _resource_type: Option<&str>,
        ) -> StorageResult<u64> {
            Ok(0)
        }
    }

    fn make_tenant() -> TenantContext {
        TenantContext::new(TenantId::new("test"), TenantPermissions::full_access())
    }

    fn make_composite_no_secondary() -> CompositeStorage {
        let config = CompositeConfig::builder()
            .primary("primary", BackendKind::Sqlite)
            .build()
            .unwrap();
        let mut backends = HashMap::new();
        backends.insert("primary".to_string(), Arc::new(MockStorage) as DynStorage);
        CompositeStorage::new(config, backends).unwrap()
    }

    fn make_composite_with_secondary() -> CompositeStorage {
        let config = CompositeConfig::builder()
            .primary("primary", BackendKind::Sqlite)
            .search_backend("es", BackendKind::Elasticsearch)
            .build()
            .unwrap();
        let mut backends = HashMap::new();
        backends.insert("primary".to_string(), Arc::new(MockStorage) as DynStorage);
        backends.insert("es".to_string(), Arc::new(MockStorage) as DynStorage);
        CompositeStorage::new(config, backends).unwrap()
    }

    fn test_config() -> CompositeConfig {
        CompositeConfig::builder()
            .primary("sqlite", BackendKind::Sqlite)
            .search_backend("es", BackendKind::Elasticsearch)
            .build()
            .unwrap()
    }

    // ── BackendHealth ──────────────────────────────────────────────

    #[test]
    fn test_backend_health_default() {
        let health = BackendHealth::default();
        assert!(health.healthy);
        assert_eq!(health.failure_count, 0);
        assert!(health.last_error.is_none());
        assert!(health.last_success.is_none());
    }

    #[test]
    fn test_backend_health_clone() {
        let health = BackendHealth {
            healthy: false,
            last_success: None,
            failure_count: 5,
            last_error: Some("timeout".to_string()),
        };
        let cloned = health.clone();
        assert!(!cloned.healthy);
        assert_eq!(cloned.failure_count, 5);
        assert_eq!(cloned.last_error.as_deref(), Some("timeout"));
    }

    // ── CompositeConfig ────────────────────────────────────────────

    #[test]
    fn test_composite_config() {
        let config = test_config();
        assert_eq!(config.primary_id(), Some("sqlite"));
        assert_eq!(config.secondaries().count(), 1);
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_search_prefers_configured_search_backend() {
        use std::collections::HashMap;
        use std::sync::Arc;

        use crate::backends::sqlite::SqliteBackend;
        use crate::core::{ResourceStorage, SearchProvider};
        use crate::tenant::{TenantContext, TenantId, TenantPermissions};
        use crate::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

        let primary = Arc::new(SqliteBackend::in_memory().expect("create primary sqlite backend"));
        primary.init_schema().expect("init primary sqlite schema");

        let search = Arc::new(SqliteBackend::in_memory().expect("create search sqlite backend"));
        search.init_schema().expect("init search sqlite schema");

        let tenant = TenantContext::new(
            TenantId::new("composite-test"),
            TenantPermissions::full_access(),
        );

        // Seed distinct data so we can tell which provider answered the query.
        primary
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "primary-only-patient",
                }),
                FhirVersion::default(),
            )
            .await
            .expect("seed primary patient");

        search
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "search-only-patient",
                }),
                FhirVersion::default(),
            )
            .await
            .expect("seed search patient");

        let composite_config = CompositeConfig::builder()
            .primary("primary", BackendKind::Sqlite)
            .search_backend("search", BackendKind::Sqlite)
            .build()
            .expect("build composite config");

        let mut backends = HashMap::new();
        backends.insert("primary".to_string(), primary.clone() as DynStorage);
        backends.insert("search".to_string(), search.clone() as DynStorage);

        let mut search_providers = HashMap::new();
        search_providers.insert("primary".to_string(), primary.clone() as DynSearchProvider);
        search_providers.insert("search".to_string(), search.clone() as DynSearchProvider);

        let composite = CompositeStorage::new(composite_config, backends)
            .expect("create composite storage")
            .with_search_providers(search_providers)
            .with_full_primary(primary.clone());

        let read_result = composite
            .read(&tenant, "Patient", "primary-only-patient")
            .await
            .expect("composite read should succeed");
        assert!(
            read_result.is_some(),
            "Read path should use primary backend data"
        );

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("search-only-patient")],
            chain: vec![],
            components: vec![],
        });

        let result = composite
            .search(&tenant, &query)
            .await
            .expect("composite search should succeed");

        assert_eq!(result.resources.len(), 1);
        assert_eq!(result.resources.items[0].id(), "search-only-patient");

        let count = composite
            .search_count(&tenant, &query)
            .await
            .expect("composite search_count should succeed");
        assert_eq!(count, 1);
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_search_backend_preserves_tenant_isolation() {
        use std::collections::HashMap;
        use std::sync::Arc;

        use crate::backends::sqlite::SqliteBackend;
        use crate::core::{ResourceStorage, SearchProvider};
        use crate::tenant::{TenantContext, TenantId, TenantPermissions};

        let primary = Arc::new(SqliteBackend::in_memory().expect("create primary sqlite backend"));
        primary.init_schema().expect("init primary sqlite schema");

        let search = Arc::new(SqliteBackend::in_memory().expect("create search sqlite backend"));
        search.init_schema().expect("init search sqlite schema");

        let tenant_a =
            TenantContext::new(TenantId::new("tenant-a"), TenantPermissions::full_access());
        let tenant_b =
            TenantContext::new(TenantId::new("tenant-b"), TenantPermissions::full_access());

        search
            .create(
                &tenant_a,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "tenant-a-patient",
                }),
                FhirVersion::default(),
            )
            .await
            .expect("seed tenant A search patient");

        search
            .create(
                &tenant_b,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "tenant-b-patient",
                }),
                FhirVersion::default(),
            )
            .await
            .expect("seed tenant B search patient");

        let composite_config = CompositeConfig::builder()
            .primary("primary", BackendKind::Sqlite)
            .search_backend("search", BackendKind::Sqlite)
            .build()
            .expect("build composite config");

        let mut backends = HashMap::new();
        backends.insert("primary".to_string(), primary.clone() as DynStorage);
        backends.insert("search".to_string(), search.clone() as DynStorage);

        let mut search_providers = HashMap::new();
        search_providers.insert("primary".to_string(), primary.clone() as DynSearchProvider);
        search_providers.insert("search".to_string(), search.clone() as DynSearchProvider);

        let composite = CompositeStorage::new(composite_config, backends)
            .expect("create composite storage")
            .with_search_providers(search_providers)
            .with_full_primary(primary.clone());

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("tenant-a-patient")],
            chain: vec![],
            components: vec![],
        });

        let tenant_a_result = composite
            .search(&tenant_a, &query)
            .await
            .expect("tenant A composite search should succeed");
        assert_eq!(tenant_a_result.resources.len(), 1);
        assert_eq!(tenant_a_result.resources.items[0].id(), "tenant-a-patient");

        let tenant_b_result = composite
            .search(&tenant_b, &query)
            .await
            .expect("tenant B composite search should succeed");
        assert!(
            tenant_b_result.resources.is_empty(),
            "delegated search must not leak tenant A data to tenant B"
        );
    }

    #[test]
    fn test_search_backend_failure_marks_backend_unhealthy() {
        use std::collections::HashMap;
        use std::sync::Arc;

        use crate::composite::config::HealthConfig;

        let primary = Arc::new(FailingSearchBackend {
            backend_name: "primary",
            error_message: "primary should not be used",
        });
        let search = Arc::new(FailingSearchBackend {
            backend_name: "search",
            error_message: "simulated search outage",
        });

        let composite_config = CompositeConfig::builder()
            .primary("primary", BackendKind::MongoDB)
            .search_backend("search", BackendKind::Elasticsearch)
            .with_health_config(HealthConfig {
                failure_threshold: 1,
                ..HealthConfig::default()
            })
            .build()
            .expect("build composite config");

        let mut backends = HashMap::new();
        backends.insert("primary".to_string(), primary.clone() as DynStorage);
        backends.insert("search".to_string(), search.clone() as DynStorage);

        let mut search_providers = HashMap::new();
        search_providers.insert("primary".to_string(), primary.clone() as DynSearchProvider);
        search_providers.insert("search".to_string(), search.clone() as DynSearchProvider);

        let composite = CompositeStorage::new(composite_config, backends)
            .expect("create composite storage")
            .with_search_providers(search_providers);

        let tenant = TenantContext::new(
            TenantId::new("tenant-failure"),
            TenantPermissions::full_access(),
        );
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("failure-patient")],
            chain: vec![],
            components: vec![],
        });

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let err = runtime
            .block_on(composite.search(&tenant, &query))
            .expect_err("delegated search should fail when search backend is down");

        assert!(matches!(
            err,
            StorageError::Backend(BackendError::ConnectionFailed {
                backend_name,
                message,
            }) if backend_name == "search" && message.contains("simulated search outage")
        ));

        let health = composite
            .backend_health("search")
            .expect("search backend health should exist");
        assert!(
            !health.healthy,
            "search backend should be marked unhealthy after failure"
        );
        assert_eq!(health.failure_count, 1);
        assert_eq!(
            health.last_error.as_deref(),
            Some("connection failed to search: simulated search outage")
        );
    }

    // ── CompositeStorage::new() ────────────────────────────────────

    #[test]
    fn test_new_success() {
        let composite = make_composite_no_secondary();
        assert_eq!(composite.backend_name(), "composite");
    }

    #[test]
    fn test_new_missing_primary_backend_in_map() {
        let config = CompositeConfig::builder()
            .primary("primary", BackendKind::Sqlite)
            .build()
            .unwrap();
        // Deliberately omit the primary from the backends map
        let backends: HashMap<String, DynStorage> = HashMap::new();
        let result = CompositeStorage::new(config, backends);
        assert!(result.is_err());
    }

    #[test]
    fn test_new_with_secondary() {
        let composite = make_composite_with_secondary();
        assert!(composite.secondary("es").is_some());
        assert!(composite.secondary("nonexistent").is_none());
    }

    // ── Accessors ─────────────────────────────────────────────────

    #[test]
    fn test_config_accessor() {
        let composite = make_composite_no_secondary();
        assert_eq!(composite.config().primary_id(), Some("primary"));
    }

    #[test]
    fn test_primary_accessor() {
        let composite = make_composite_no_secondary();
        assert_eq!(composite.primary().backend_name(), "mock");
    }

    #[test]
    fn test_secondaries_accessor() {
        let composite = make_composite_with_secondary();
        assert_eq!(composite.secondaries().len(), 1);
        assert!(composite.secondaries().contains_key("es"));
    }

    #[test]
    fn test_secondaries_empty_when_no_secondary() {
        let composite = make_composite_no_secondary();
        assert!(composite.secondaries().is_empty());
    }

    #[test]
    fn test_secondary_accessor_present() {
        let composite = make_composite_with_secondary();
        assert!(composite.secondary("es").is_some());
    }

    #[test]
    fn test_secondary_accessor_absent() {
        let composite = make_composite_no_secondary();
        assert!(composite.secondary("missing").is_none());
    }

    // ── Health tracking ───────────────────────────────────────────

    #[test]
    fn test_backend_health_initially_healthy() {
        let composite = make_composite_no_secondary();
        let health = composite.backend_health("primary").unwrap();
        assert!(health.healthy);
    }

    #[test]
    fn test_backend_health_missing_id_returns_none() {
        let composite = make_composite_no_secondary();
        assert!(composite.backend_health("nonexistent").is_none());
    }

    #[test]
    fn test_is_backend_healthy_true() {
        let composite = make_composite_no_secondary();
        assert!(composite.is_backend_healthy("primary"));
    }

    #[test]
    fn test_is_backend_healthy_unknown_returns_false() {
        let composite = make_composite_no_secondary();
        assert!(!composite.is_backend_healthy("nonexistent"));
    }

    #[test]
    fn test_update_health_success_resets_failures() {
        let composite = make_composite_no_secondary();
        // First, record a few failures
        composite.update_health("primary", false, Some("err1".to_string()));
        composite.update_health("primary", false, Some("err2".to_string()));
        let health = composite.backend_health("primary").unwrap();
        assert_eq!(health.failure_count, 2);

        // Now record a success — should reset
        composite.update_health("primary", true, None);
        let health = composite.backend_health("primary").unwrap();
        assert!(health.healthy);
        assert_eq!(health.failure_count, 0);
        assert!(health.last_error.is_none());
        assert!(health.last_success.is_some());
    }

    #[test]
    fn test_update_health_failure_increments_count() {
        let composite = make_composite_no_secondary();
        composite.update_health("primary", false, Some("timeout".to_string()));
        let health = composite.backend_health("primary").unwrap();
        assert_eq!(health.failure_count, 1);
        assert_eq!(health.last_error.as_deref(), Some("timeout"));
    }

    #[test]
    fn test_update_health_marks_unhealthy_after_threshold() {
        let composite = make_composite_no_secondary();
        // Default failure_threshold is 3
        let threshold = composite.config.health_config.failure_threshold;
        for i in 0..threshold {
            composite.update_health("primary", false, Some(format!("error {}", i)));
        }
        let health = composite.backend_health("primary").unwrap();
        assert!(!health.healthy);
        assert_eq!(health.failure_count, threshold);
    }

    #[test]
    fn test_update_health_ignores_unknown_backend() {
        let composite = make_composite_no_secondary();
        // Should not panic on unknown backend ID
        composite.update_health("nonexistent", false, Some("err".to_string()));
    }

    // ── sync_to_secondaries when sync_manager is None ─────────────

    #[tokio::test]
    async fn test_sync_to_secondaries_no_sync_manager_returns_ok() {
        let composite = make_composite_no_secondary();
        // No sync manager (no secondaries) → should return Ok immediately
        let result = composite
            .sync_to_secondaries(super::super::sync::SyncEvent::Delete {
                resource_type: "Patient".to_string(),
                resource_id: "1".to_string(),
                tenant_id: TenantId::new("test"),
            })
            .await;
        assert!(result.is_ok());
    }

    // ── routing_error_to_storage_error ────────────────────────────

    #[test]
    fn test_routing_error_no_primary_backend() {
        let composite = make_composite_no_secondary();
        let err = composite.routing_error_to_storage_error(RoutingError::NoPrimaryBackend);
        match err {
            StorageError::Backend(BackendError::Unavailable { backend_name, .. }) => {
                assert_eq!(backend_name, "primary");
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn test_routing_error_no_capable_backend() {
        use super::super::analyzer::QueryFeature;
        let composite = make_composite_no_secondary();
        let err = composite.routing_error_to_storage_error(RoutingError::NoCapableBackend {
            feature: QueryFeature::FullTextSearch,
        });
        match err {
            StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name,
                capability,
            }) => {
                assert_eq!(backend_name, "composite");
                assert!(!capability.is_empty());
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn test_routing_error_backend_unavailable() {
        let composite = make_composite_no_secondary();
        let err = composite.routing_error_to_storage_error(RoutingError::BackendUnavailable {
            backend_id: "my-backend".to_string(),
        });
        match err {
            StorageError::Backend(BackendError::ConnectionFailed { backend_name, .. }) => {
                assert_eq!(backend_name, "my-backend");
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    // ── CapabilityProvider ────────────────────────────────────────

    #[test]
    fn test_capabilities_backend_name() {
        let composite = make_composite_no_secondary();
        let caps = composite.capabilities();
        assert_eq!(caps.backend_name, "composite");
    }

    #[test]
    fn test_backend_name_is_composite() {
        let composite = make_composite_no_secondary();
        assert_eq!(composite.backend_name(), "composite");
    }

    // ── "No capability" error paths ───────────────────────────────

    #[tokio::test]
    async fn test_conditional_create_no_capability() {
        use crate::core::ConditionalStorage;
        let composite = make_composite_no_secondary();
        let tenant = make_tenant();
        let result = composite
            .conditional_create(
                &tenant,
                "Patient",
                serde_json::json!({}),
                "identifier=foo",
                FhirVersion::default(),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_versioned_storage_vread_no_capability() {
        use crate::core::VersionedStorage;
        let composite = make_composite_no_secondary();
        let tenant = make_tenant();
        let result = composite.vread(&tenant, "Patient", "1", "1").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            StorageError::Backend(BackendError::UnsupportedCapability { capability, .. }) => {
                assert!(capability.contains("VersionedStorage"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_instance_history_no_capability() {
        use crate::core::InstanceHistoryProvider;
        use crate::core::history::HistoryParams;
        let composite = make_composite_no_secondary();
        let tenant = make_tenant();
        let result = composite
            .history_instance(&tenant, "Patient", "1", &HistoryParams::default())
            .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            StorageError::Backend(BackendError::UnsupportedCapability { capability, .. }) => {
                assert!(capability.contains("InstanceHistoryProvider"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_bundle_provider_process_batch_no_capability() {
        use crate::core::BundleProvider;
        let composite = make_composite_no_secondary();
        let tenant = make_tenant();
        let result = composite.process_batch(&tenant, vec![]).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            StorageError::Backend(BackendError::UnsupportedCapability { capability, .. }) => {
                assert!(capability.contains("BundleProvider"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_bundle_provider_process_transaction_no_capability() {
        use crate::core::BundleProvider;
        let composite = make_composite_no_secondary();
        let tenant = make_tenant();
        let result = composite.process_transaction(&tenant, vec![]).await;
        assert!(result.is_err());
    }

    // ── search_count when no search provider ──────────────────────

    #[tokio::test]
    async fn test_search_count_no_search_provider_returns_error() {
        use crate::core::SearchProvider;
        use crate::types::SearchQuery;
        let composite = make_composite_no_secondary();
        let tenant = make_tenant();
        let query = SearchQuery::new("Patient");
        let result = composite.search_count(&tenant, &query).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            StorageError::Backend(BackendError::UnsupportedCapability { capability, .. }) => {
                assert!(capability.contains("search_count"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    // ── with_search_providers ─────────────────────────────────────

    #[test]
    fn test_with_search_providers() {
        let composite = make_composite_no_secondary();
        let mut providers = HashMap::new();
        providers.insert(
            "primary".to_string(),
            Arc::new(MockStorage) as DynSearchProvider,
        );
        let composite = composite.with_search_providers(providers);
        // Should have one search provider now
        assert!(composite.search_providers.contains_key("primary"));
    }

    // ── extract_reference_values (static method) ──────────────────

    #[test]
    fn test_extract_reference_values_object_with_reference() {
        let obj = serde_json::json!({"reference": "Patient/123"});
        let mut refs = Vec::new();
        CompositeStorage::extract_reference_values(&obj, &mut refs);
        assert_eq!(refs, vec!["Patient/123"]);
    }

    #[test]
    fn test_extract_reference_values_object_without_reference() {
        let obj = serde_json::json!({"display": "John Smith"});
        let mut refs = Vec::new();
        CompositeStorage::extract_reference_values(&obj, &mut refs);
        assert!(refs.is_empty());
    }

    #[test]
    fn test_extract_reference_values_array() {
        let arr = serde_json::json!([
            {"reference": "Patient/1"},
            {"reference": "Patient/2"},
            {"display": "no ref here"}
        ]);
        let mut refs = Vec::new();
        CompositeStorage::extract_reference_values(&arr, &mut refs);
        assert_eq!(refs.len(), 2);
        assert!(refs.contains(&"Patient/1".to_string()));
        assert!(refs.contains(&"Patient/2".to_string()));
    }

    #[test]
    fn test_extract_reference_values_primitive_ignored() {
        let val = serde_json::json!("just a string");
        let mut refs = Vec::new();
        CompositeStorage::extract_reference_values(&val, &mut refs);
        assert!(refs.is_empty());
    }

    #[test]
    fn test_extract_reference_values_null_ignored() {
        let val = serde_json::Value::Null;
        let mut refs = Vec::new();
        CompositeStorage::extract_reference_values(&val, &mut refs);
        assert!(refs.is_empty());
    }

    // ── CRUD delegation to primary ────────────────────────────────

    #[tokio::test]
    async fn test_create_delegates_to_primary() {
        use crate::core::ResourceStorage;
        let composite = make_composite_no_secondary();
        let tenant = make_tenant();
        let result = composite
            .create(
                &tenant,
                "Patient",
                serde_json::json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await;
        assert!(result.is_ok());
        let stored = result.unwrap();
        assert_eq!(stored.resource_type(), "Patient");
    }

    #[tokio::test]
    async fn test_read_delegates_to_primary() {
        use crate::core::ResourceStorage;
        let composite = make_composite_no_secondary();
        let tenant = make_tenant();
        let result = composite.read(&tenant, "Patient", "1").await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none()); // MockStorage always returns None
    }

    #[tokio::test]
    async fn test_count_delegates_to_primary() {
        use crate::core::ResourceStorage;
        let composite = make_composite_no_secondary();
        let tenant = make_tenant();
        let result = composite.count(&tenant, Some("Patient")).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_delete_delegates_to_primary() {
        use crate::core::ResourceStorage;
        let composite = make_composite_no_secondary();
        let tenant = make_tenant();
        let result = composite.delete(&tenant, "Patient", "1").await;
        assert!(result.is_ok());
    }

    // SearchProvider impl for MockStorage (must live inside test module).
    #[async_trait::async_trait]
    impl SearchProvider for MockStorage {
        async fn search(
            &self,
            _tenant: &TenantContext,
            _query: &crate::types::SearchQuery,
        ) -> StorageResult<SearchResult> {
            use crate::types::Page;
            Ok(SearchResult::new(Page::empty()))
        }

        async fn search_count(
            &self,
            _tenant: &TenantContext,
            _query: &crate::types::SearchQuery,
        ) -> StorageResult<u64> {
            Ok(0)
        }
    }
}
