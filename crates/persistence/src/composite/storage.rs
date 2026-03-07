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
    IncludeDirective, Pagination, ReverseChainedParameter, SearchQuery, StoredResource,
};

use super::config::CompositeConfig;
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
        // For count, we can just use primary
        // A more sophisticated implementation might route based on features
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
    use crate::core::BackendKind;

    fn test_config() -> CompositeConfig {
        CompositeConfig::builder()
            .primary("sqlite", BackendKind::Sqlite)
            .search_backend("es", BackendKind::Elasticsearch)
            .build()
            .unwrap()
    }

    #[test]
    fn test_backend_health_default() {
        let health = BackendHealth::default();
        assert!(health.healthy);
        assert_eq!(health.failure_count, 0);
        assert!(health.last_error.is_none());
    }

    #[test]
    fn test_composite_config() {
        let config = test_config();
        assert_eq!(config.primary_id(), Some("sqlite"));
        assert_eq!(config.secondaries().count(), 1);
    }
}
