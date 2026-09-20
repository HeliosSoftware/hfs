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
    ConditionalUpdateResult, ExportDataProvider, ExportRequest, GroupExportProvider,
    IncludeProvider, InstanceHistoryProvider, NdjsonBatch, PatchFormat, PatientExportProvider,
    PurgableStorage, ResourceStorage, RevincludeProvider, SearchProvider, SearchResult, SofRunner,
    StorageCapabilities, SystemHistoryProvider, TerminologySearchProvider, TextSearchProvider,
    TypeHistoryProvider, VersionedStorage,
};
use crate::error::{BackendError, ResourceError, StorageError, StorageResult, TransactionError};
use crate::search::ChainResolveOptions;
use crate::tenant::TenantContext;
use crate::types::{
    IncludeDirective, Pagination, ReverseChainedParameter, SearchParamType, SearchParameter,
    SearchQuery, SearchValue, StoredResource,
};

use super::config::{CompositeConfig, SyncMode};
use super::merger::{MergeOptions, ResultMerger};
use super::router::{QueryRouter, RoutingDecision, RoutingError};
use super::sync::{SyncEvent, SyncManager, SyncStatus};

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

/// A dynamically typed system history provider (also covers Type + Instance).
pub type DynSystemHistoryProvider = Arc<dyn SystemHistoryProvider + Send + Sync>;

/// A dynamically typed bundle provider.
pub type DynBundleProvider = Arc<dyn BundleProvider + Send + Sync>;

/// A dynamically typed group export provider (also covers Patient + System).
pub type DynGroupExportProvider = Arc<dyn GroupExportProvider + Send + Sync>;

/// Type alias for a purgable backend.
pub type DynPurgableStorage = Arc<dyn PurgableStorage + Send + Sync>;

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

    /// Primary as SystemHistoryProvider (if supported) — covers Type + System history.
    system_history_provider: Option<DynSystemHistoryProvider>,

    /// Primary as BundleProvider (if supported).
    bundle_provider: Option<DynBundleProvider>,

    /// Primary as GroupExportProvider (if supported) — covers all export levels.
    export_provider: Option<DynGroupExportProvider>,

    /// Primary as PurgableStorage (if supported).
    ///
    /// `secondaries` are typed only as `ResourceStorage`, which cannot reach
    /// `PurgableStorage`, so the purge fan-out needs its own typed handles.
    /// Set via [`with_purgable_backends`](Self::with_purgable_backends).
    purgable_primary: Option<DynPurgableStorage>,

    /// Every secondary that must also be purged, by backend id. The id is
    /// carried so a failure can name which backend refused.
    purgable_secondaries: Vec<(String, DynPurgableStorage)>,
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

    async fn find_conditional_matches(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
    ) -> StorageResult<Vec<StoredResource>> {
        // The one criteria builder every backend shares, so criteria mean what
        // they mean as a direct search (#1312). The guard is dropped before
        // the first await.
        let query = {
            let registry_arc = self.search_param_registry(tenant);
            let registry = registry_arc.read();
            // A composite has no FHIR version of its own to judge a `:[type]`
            // qualifier against, so any enabled version's type passes here.
            crate::search::build_conditional_query(
                &registry,
                resource_type,
                search_params,
                crate::search::ResourceTypeScope::any_enabled(),
            )?
        };
        let Some(query) = query else {
            return Ok(Vec::new());
        };

        // The criteria resolve against the search index, which on a
        // composite is the secondary: without this, an `If-None-Exist`
        // create issued right after the matching resource was created finds
        // nothing and writes the duplicate it exists to prevent (#1047).
        self.ensure_writes_visible(tenant, &[resource_type]).await?;
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
            system_history_provider: None,
            bundle_provider: None,
            export_provider: None,
            purgable_primary: None,
            purgable_secondaries: Vec::new(),
        })
    }

    /// Wires the typed handles that `$purge` fans out to.
    ///
    /// `secondaries` must name every backend that holds a copy of the resource
    /// — in practice the Elasticsearch search index. Omitting one means `$purge`
    /// silently leaves the purged resource in that backend, where it stays
    /// searchable and continues to hold the resource's content.
    pub fn with_purgable_backends(
        mut self,
        primary: DynPurgableStorage,
        secondaries: Vec<(String, DynPurgableStorage)>,
    ) -> Self {
        self.purgable_primary = Some(primary);
        self.purgable_secondaries = secondaries;
        self
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
            + TypeHistoryProvider
            + SystemHistoryProvider
            + BundleProvider
            + GroupExportProvider
            + Send
            + Sync
            + 'static,
    {
        self.conditional_storage = Some(primary.clone() as DynConditionalStorage);
        self.versioned_storage = Some(primary.clone() as DynVersionedStorage);
        self.history_provider = Some(primary.clone() as DynInstanceHistoryProvider);
        self.system_history_provider = Some(primary.clone() as DynSystemHistoryProvider);
        self.bundle_provider = Some(primary.clone() as DynBundleProvider);
        self.export_provider = Some(primary as DynGroupExportProvider);
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

    /// Reports whether a secondary's search results become consistent with
    /// the primary as soon as a sync call returns, rather than at some later
    /// point after an async queue drains.
    ///
    /// This is `true` for [`SyncMode::Synchronous`], and for
    /// [`SyncMode::Hybrid`] when `sync_for_search` is set — the same
    /// derivation `SyncManager::sync_creates` (see [`SyncManager`]) uses to
    /// decide whether a write waits on the secondary. It is `false` for
    /// [`SyncMode::Asynchronous`] and whenever there is no [`SyncManager`]
    /// at all (no secondaries configured).
    ///
    /// Callers use this to gate any post-sync check that reads a secondary's
    /// state immediately: under asynchronous sync, a `count()` taken right
    /// after a sync call reflects whatever had already drained from the
    /// queue, not what was just synced, so such a check would be comparing
    /// against a moving target rather than a real discrepancy.
    pub fn syncs_search_synchronously(&self) -> bool {
        if self.sync_manager.is_none() {
            return false;
        }
        matches!(
            self.config.sync_config.mode,
            SyncMode::Synchronous
                | SyncMode::Hybrid {
                    sync_for_search: true
                }
        )
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
    pub(crate) async fn sync_to_secondaries(&self, event: SyncEvent) -> StorageResult<()> {
        if let Some(ref sync_manager) = self.sync_manager {
            sync_manager.sync(&event, &self.secondaries).await?;
        }
        Ok(())
    }

    /// Routes and executes a search query.
    ///
    /// With a dedicated Search backend the whole query goes there; otherwise
    /// the query is routed, split by feature across backends, and merged.
    #[instrument(skip(self, tenant, query), fields(resource_type = %query.resource_type))]
    async fn execute_routed_search(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        // When a dedicated Search backend is configured, the primary has its
        // search index offloaded (`search_offloaded = true`) and thus empty.
        // Splitting the query and intersecting with the primary's (empty)
        // results would always return nothing and drop the `total`. The
        // Search backend supports every query feature, including full-text,
        // so send it the whole query instead, exactly like the no-auxiliary
        // path below already does. See #1012.
        if self.has_dedicated_search_backend() {
            return self.execute_primary_search(tenant, query).await;
        }

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
    ///
    /// Entries are grouped by resource type and synced as batches
    /// ([`Self::sync_creates_to_secondaries`]) rather than one event each: a
    /// transaction Bundle's entries each waited on the secondary in turn, and
    /// under Elasticsearch `refresh=wait_for` (one refresh interval per write)
    /// a bundle of a few dozen entries outran the 30s request timeout.
    /// Ordering across types does not matter to a search secondary, and the
    /// primary has already committed the whole bundle.
    async fn sync_bundle_results(
        &self,
        tenant: &TenantContext,
        result: &BundleResult,
        fhir_version: FhirVersion,
    ) {
        let mut by_type: Vec<(String, Vec<(String, Value)>)> = Vec::new();
        for entry_result in &result.entries {
            // Only sync successful mutating operations that have a resource body
            let Some(ref resource_json) = entry_result.resource else {
                continue;
            };
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
            let group = match by_type.iter_mut().find(|(t, _)| t == resource_type) {
                Some(group) => group,
                None => {
                    by_type.push((resource_type.to_string(), Vec::new()));
                    by_type.last_mut().expect("just pushed")
                }
            };
            group
                .1
                .push((resource_id.to_string(), resource_json.clone()));
        }
        for (resource_type, resources) in by_type {
            // The bundle path has no per-entry receipt to correct on a
            // rejection; the per-backend statuses are for callers that do
            // (bulk-submit), so this one discards them.
            let _ = self
                .sync_creates_to_secondaries(tenant, &resource_type, fhir_version, resources)
                .await;
        }
    }

    /// Syncs a batch of resources of one type to the secondaries as a batch
    /// ([`SyncManager::sync_creates`]), logging rather than failing: the
    /// primary already holds them, and a secondary that missed the batch is
    /// repaired by `$reindex`.
    ///
    /// Returns each secondary's [`SyncStatus`] (including which ids, if any,
    /// it rejected after retries), so a caller that must know — e.g. to mark
    /// a bulk-submitted resource's entry result `processing-error` — can
    /// inspect it. Empty when there are no resources, no secondaries
    /// configured, or the batch sync itself errored (already logged here).
    pub(crate) async fn sync_creates_to_secondaries(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        fhir_version: FhirVersion,
        resources: Vec<(String, Value)>,
    ) -> Vec<SyncStatus> {
        if resources.is_empty() {
            return Vec::new();
        }
        let Some(ref sync_manager) = self.sync_manager else {
            return Vec::new();
        };
        match sync_manager
            .sync_creates(
                tenant.tenant_id(),
                resource_type,
                fhir_version,
                resources,
                &self.secondaries,
            )
            .await
        {
            Ok(statuses) => statuses,
            Err(e) => {
                warn!(
                    error = %e,
                    resource_type,
                    "Failed to sync batch of resources to secondaries"
                );
                Vec::new()
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

    /// Readiness gates on the primary (CRUD) backend only. A secondary/search
    /// backend being down is a partial degradation — core CRUD still works, so
    /// the instance should stay in rotation rather than be pulled out entirely
    /// (which would take down the traffic it can still serve). If the primary
    /// system-of-record is unreachable, the instance is not ready.
    async fn readiness_check(&self) -> Result<(), BackendError> {
        self.primary().readiness_check().await
    }

    fn is_cluster_shared(&self) -> bool {
        self.primary.is_cluster_shared()
    }

    fn bulk_write_concurrency(&self) -> usize {
        // Writes route to the primary, so its tolerance is the bound.
        self.primary.bulk_write_concurrency()
    }

    fn sof_runner(&self) -> Option<Arc<dyn SofRunner>> {
        // SQL-on-FHIR runs as in-DB SQL against the primary store (where all
        // writes land), so delegate to the primary backend. Composites whose
        // primary is SOF-capable (e.g. sqlite-elasticsearch) thus expose a
        // runner; others inherit `None`.
        self.primary.sof_runner()
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

    /// The batch goes to the primary as a batch, and then what the primary
    /// accepted goes to every secondary as a batch too
    /// ([`SyncManager::sync_creates`]): a bulk load pays the secondary's batch
    /// write (one Elasticsearch `_bulk` request, one refresh wait) rather than
    /// one synchronous per-resource sync each, which is what turned the
    /// startup conformance seed into a twenty-minute stall under
    /// `refresh=wait_for`.
    async fn create_many(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resources: Vec<Value>,
        fhir_version: FhirVersion,
    ) -> Vec<StorageResult<StoredResource>> {
        let results = self
            .primary
            .create_many(tenant, resource_type, resources, fhir_version)
            .await;

        // `AlreadyExists` and validation errors are per-resource outcomes, not
        // evidence about the backend; only a backend error counts against its
        // health.
        let primary_id = self.config.primary_id().unwrap_or("primary");
        let backend_failure = results.iter().find_map(|result| match result {
            Err(error @ StorageError::Backend(_)) => Some(error.to_string()),
            _ => None,
        });
        self.update_health(primary_id, backend_failure.is_none(), backend_failure);

        let created: Vec<(String, Value)> = results
            .iter()
            .filter_map(|result| result.as_ref().ok())
            .map(|stored| (stored.id().to_string(), stored.content().clone()))
            .collect();
        // `create_many` has no per-entry receipt to correct either — that is
        // `CompositeSubmitJobs::sync_ingested`'s job, which calls
        // `sync_creates_to_secondaries` itself and inspects the statuses.
        let _ = self
            .sync_creates_to_secondaries(tenant, resource_type, fhir_version, created)
            .await;

        results
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

        // Sync to secondaries.
        //
        // A `PUT` onto a deleted id restores the resource, which the primary
        // reports as `created` even though it continues the existing version
        // chain (a genuine create is always version "1"). Secondaries already
        // hold a row for that id, so a restore has to sync as an update —
        // backends whose `create` rejects an existing id would otherwise drop
        // the write and go stale.
        let event = if created && stored.version_id() == "1" {
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

    async fn count_by_day(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        since: chrono::DateTime<chrono::Utc>,
    ) -> StorageResult<Vec<crate::core::DailyResourceCount>> {
        // Counts are an authoritative-store concern; the search secondary may
        // lag or omit soft-deletes. Delegate to the primary backend, which owns
        // the canonical `resources` table. (`count_by_types` uses the default
        // impl, which routes through `count` — also primary-backed above.)
        self.primary
            .count_by_day(tenant, resource_type, since)
            .await
    }

    async fn count_deltas_by_bucket(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        since: chrono::DateTime<chrono::Utc>,
        bucket_seconds: i64,
    ) -> StorageResult<Vec<crate::core::ResourceCountDelta>> {
        // The history log lives with the authoritative primary store, same as the
        // counts above and `activity_histogram` below.
        self.primary
            .count_deltas_by_bucket(tenant, resource_type, since, bucket_seconds)
            .await
    }

    async fn count_deltas_by_type_and_bucket(
        &self,
        tenant: &TenantContext,
        resource_types: &[&str],
        since: chrono::DateTime<chrono::Utc>,
        bucket_seconds: i64,
    ) -> StorageResult<Vec<(String, crate::core::ResourceCountDelta)>> {
        // Same history log as `count_deltas_by_bucket`: the primary's, which
        // answers with its own grouped query rather than the per-type default.
        self.primary
            .count_deltas_by_type_and_bucket(tenant, resource_types, since, bucket_seconds)
            .await
    }

    async fn activity_histogram(
        &self,
        tenant: &TenantContext,
        since: chrono::DateTime<chrono::Utc>,
    ) -> StorageResult<Vec<crate::core::ActivityCell>> {
        // History/activity is owned by the authoritative primary store.
        self.primary.activity_histogram(tenant, since).await
    }

    async fn count_all_types(&self, tenant: &TenantContext) -> StorageResult<Vec<(String, u64)>> {
        self.primary.count_all_types(tenant).await
    }

    fn supports_type_counts(&self) -> bool {
        // Both count aggregates above delegate to the primary, so it decides.
        self.primary.supports_type_counts()
    }

    async fn latest_write_marker(
        &self,
        tenant: &TenantContext,
        recent_since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> StorageResult<Option<crate::core::WriteMarker>> {
        // The history log the marker reads lives with the authoritative primary,
        // same as the count aggregates above.
        self.primary.latest_write_marker(tenant, recent_since).await
    }

    async fn count_by_tenant(&self) -> StorageResult<Vec<(String, u64)>> {
        self.primary.count_by_tenant().await
    }

    // ---- Tenant registry ----------------------------------------------------
    //
    // The registry of record is the authoritative primary store; reads and
    // registration delegate straight to it. Only the data purge fans out: in
    // search-offloaded modes the purged tenant's search documents live solely
    // in the secondary, so a primary-only purge would leave them discoverable.

    fn supports_tenant_registry(&self) -> bool {
        self.primary.supports_tenant_registry()
    }

    async fn list_tenants(&self) -> StorageResult<Vec<crate::core::TenantRecord>> {
        self.primary.list_tenants().await
    }

    async fn get_tenant(&self, id: &str) -> StorageResult<Option<crate::core::TenantRecord>> {
        self.primary.get_tenant(id).await
    }

    async fn register_tenant(
        &self,
        id: &str,
        display_name: Option<&str>,
    ) -> StorageResult<crate::core::TenantRecord> {
        // Checked here as well as in the primary (issue #385). The composite is
        // what a search-backed deployment actually calls, and its secondaries —
        // Elasticsearch above all — derive index names from the tenant id. The
        // primary's own check would still catch this, but only after the
        // composite had already accepted the id as legitimate.
        self.ensure_canonical_tenant_id(id)?;
        self.primary.register_tenant(id, display_name).await
    }

    async fn deregister_tenant(&self, id: &str) -> StorageResult<bool> {
        crate::tenant::ensure_mutable_tenant(id)?;
        self.primary.deregister_tenant(id).await
    }

    async fn purge_tenant_data(&self, id: &str) -> StorageResult<u64> {
        crate::tenant::ensure_mutable_tenant(id)?;
        let removed = self.primary.purge_tenant_data(id).await?;
        for (backend_id, secondary) in &self.secondaries {
            match secondary.purge_tenant_data(id).await {
                Ok(_) => {}
                // A secondary without a purge (the trait default) has no
                // tenant-scoped data of its own to leak; skip it.
                Err(StorageError::Backend(BackendError::UnsupportedCapability { .. })) => {
                    debug!(backend_id = %backend_id, "secondary has no tenant purge; skipping");
                }
                // Unlike create/delete sync, a failed purge is surfaced: the
                // primary data is gone but search documents may still be
                // discoverable, and the admin must know to retry.
                Err(e) => {
                    warn!(backend_id = %backend_id, error = %e, "tenant purge failed on secondary");
                    return Err(e);
                }
            }
        }
        Ok(removed)
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

    fn search_param_registry(
        &self,
        tenant: &crate::tenant::TenantContext,
    ) -> std::sync::Arc<parking_lot::RwLock<crate::search::SearchParameterRegistry>> {
        // Same routing as `search`: prefer the dedicated Search backend's
        // registry, fall back to primary, otherwise an empty registry.
        if let Some(search_backend) = self
            .config
            .backends_with_role(super::config::BackendRole::Search)
            .next()
        {
            if let Some(provider) = self.search_providers.get(&search_backend.id) {
                return provider.search_param_registry(tenant);
            }
        }

        if let Some(provider) = self
            .search_providers
            .get(self.config.primary_id().unwrap_or("primary"))
        {
            return provider.search_param_registry(tenant);
        }

        std::sync::Arc::new(parking_lot::RwLock::new(
            crate::search::SearchParameterRegistry::new(),
        ))
    }

    fn supports_contained_search(&self) -> bool {
        // Same routing as `search`: defer to the dedicated Search backend when
        // configured, otherwise the primary provider.
        if let Some(search_backend) = self
            .config
            .backends_with_role(super::config::BackendRole::Search)
            .next()
        {
            if let Some(provider) = self.search_providers.get(&search_backend.id) {
                return provider.supports_contained_search();
            }
        }
        self.search_providers
            .get(self.config.primary_id().unwrap_or("primary"))
            .map(|p| p.supports_contained_search())
            .unwrap_or(false)
    }

    fn modifiers_for_param_type(
        &self,
        param_type: crate::types::SearchParamType,
    ) -> Vec<&'static str> {
        // Same routing as `search`: defer to the dedicated Search backend when
        // configured, otherwise the primary provider.
        if let Some(search_backend) = self
            .config
            .backends_with_role(super::config::BackendRole::Search)
            .next()
        {
            if let Some(provider) = self.search_providers.get(&search_backend.id) {
                return provider.modifiers_for_param_type(param_type);
            }
        }
        self.search_providers
            .get(self.config.primary_id().unwrap_or("primary"))
            .map(|p| p.modifiers_for_param_type(param_type))
            .unwrap_or_default()
    }

    /// A composite's search lags its primary on two axes, and this closes
    /// both: writes still sitting in the asynchronous sync queue, and writes
    /// the search backend has accepted but not yet made searchable (#1047).
    ///
    /// The queue is drained up to this point first — the write has to reach
    /// the search backend before a refresh there can reveal it. Under
    /// synchronous sync every write already reached it before its own call
    /// returned, so only the second step runs. The refresh is then delegated
    /// along the same route `search` takes, so whichever backend answers the
    /// search is the one made current. This is what makes the four `*-es`
    /// composites behave alike: the primary's own index is offloaded and
    /// empty (or, for S3, nonexistent), so the answer never comes from it.
    async fn ensure_writes_visible(
        &self,
        tenant: &TenantContext,
        resource_types: &[&str],
    ) -> StorageResult<()> {
        if let Some(manager) = self.sync_manager.as_ref()
            && !self.syncs_search_synchronously()
        {
            manager.barrier().await?;
        }

        if let Some(search_backend) = self
            .config
            .backends_with_role(super::config::BackendRole::Search)
            .next()
            && let Some(provider) = self.search_providers.get(&search_backend.id)
        {
            return provider.ensure_writes_visible(tenant, resource_types).await;
        }
        if let Some(provider) = self
            .search_providers
            .get(self.config.primary_id().unwrap_or("primary"))
        {
            return provider.ensure_writes_visible(tenant, resource_types).await;
        }
        Ok(())
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

                    Ok(ConditionalDeleteResult::Deleted(current))
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

        // The primary resolved the criteria and performed the delete; its
        // result names the row it removed, so the secondaries can drop it too.
        if let ConditionalDeleteResult::Deleted(deleted) = &result
            && let Err(e) = self
                .sync_to_secondaries(SyncEvent::Delete {
                    resource_type: resource_type.to_string(),
                    resource_id: deleted.id().to_string(),
                    tenant_id: tenant.tenant_id().clone(),
                })
                .await
        {
            warn!(error = %e, "Failed to sync conditional_delete to secondaries");
        }

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
impl TypeHistoryProvider for CompositeStorage {
    async fn history_type(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        params: &HistoryParams,
    ) -> StorageResult<crate::core::HistoryPage> {
        let provider = self.system_history_provider.as_ref().ok_or_else(|| {
            StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: "composite".to_string(),
                capability: "TypeHistoryProvider".to_string(),
            })
        })?;

        provider.history_type(tenant, resource_type, params).await
    }

    async fn history_type_count(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64> {
        let provider = self.system_history_provider.as_ref().ok_or_else(|| {
            StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: "composite".to_string(),
                capability: "TypeHistoryProvider".to_string(),
            })
        })?;

        provider.history_type_count(tenant, resource_type).await
    }
}

#[async_trait]
impl SystemHistoryProvider for CompositeStorage {
    async fn history_system(
        &self,
        tenant: &TenantContext,
        params: &HistoryParams,
    ) -> StorageResult<crate::core::HistoryPage> {
        let provider = self.system_history_provider.as_ref().ok_or_else(|| {
            StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: "composite".to_string(),
                capability: "SystemHistoryProvider".to_string(),
            })
        })?;

        provider.history_system(tenant, params).await
    }

    async fn history_system_count(&self, tenant: &TenantContext) -> StorageResult<u64> {
        let provider = self.system_history_provider.as_ref().ok_or_else(|| {
            StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: "composite".to_string(),
                capability: "SystemHistoryProvider".to_string(),
            })
        })?;

        provider.history_system_count(tenant).await
    }
}

#[async_trait]
impl BundleProvider for CompositeStorage {
    /// Delegated to the primary, never assumed.
    ///
    /// A composite is exactly as atomic as the backend holding the resources —
    /// `sqlite-elasticsearch` is atomic, `s3-elasticsearch` is not. Answering
    /// `true` unconditionally is what let an `s3-elasticsearch` deployment
    /// advertise transaction support it could not honour (#489).
    ///
    /// Absent a bundle provider there is nothing to delegate to, so the answer
    /// is `false` — `process_transaction` fails on the same condition below.
    fn supports_atomic_transactions(&self) -> bool {
        self.bundle_provider
            .as_ref()
            .is_some_and(|p| p.supports_atomic_transactions())
    }

    async fn process_transaction(
        &self,
        tenant: &TenantContext,
        entries: Vec<BundleEntry>,
        fhir_version: helios_fhir::FhirVersion,
    ) -> Result<BundleResult, TransactionError> {
        let provider =
            self.bundle_provider
                .as_ref()
                .ok_or_else(|| TransactionError::BundleError {
                    index: 0,
                    message: "BundleProvider not available on composite primary".to_string(),
                })?;

        let result = provider
            .process_transaction(tenant, entries, fhir_version)
            .await?;

        // Sync successful entries to secondaries by reading resources from primary
        self.sync_bundle_results(tenant, &result, fhir_version)
            .await;

        Ok(result)
    }
}

#[async_trait]
impl IncludeProvider for CompositeStorage {
    /// Delegates to the shared, registry-driven resolver so `_include` (and
    /// `:iterate`) follows the same search-parameter definitions and the same
    /// backend routing as `search()` (the Search-role backend if one is
    /// configured, otherwise the primary) — matching what the REST search
    /// path already does for this storage.
    async fn resolve_includes(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        includes: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>> {
        crate::core::resolve_includes_iterative(self, tenant, resources, includes).await
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
        // Delegates to the shared application-side resolver — the one REST
        // uses — running its iterative plain searches through composite's own
        // search routing. It types the terminal parameter from the registry
        // and parses the value like a direct search: comparator prefix,
        // comma-separated OR list, every target type of an untyped hop (#1304).
        let Some(param) = parse_chain_path(chain, value) else {
            // A single segment is not a chain.
            return Ok(Vec::new());
        };
        // The trait API carries no terminology server, so a terminology-backed
        // modifier on the terminal is rejected rather than searched literally
        // (the resolver's default, as for `resolve_chains`; #1317).
        crate::search::chain_resolver::resolve_forward_chain(
            self,
            tenant,
            base_type,
            &param,
            ChainResolveOptions::default(),
        )
        .await
    }

    async fn resolve_reverse_chain(
        &self,
        tenant: &TenantContext,
        base_type: &str,
        reverse_chain: &ReverseChainedParameter,
    ) -> StorageResult<Vec<String>> {
        // Same delegation as `resolve_chain`. The terminal used to be searched
        // as a hardcoded `Token` holding the raw value — prefix and commas
        // included — and a nested `_has` was ignored outright (#1304).
        crate::search::chain_resolver::resolve_reverse_chain(
            self,
            tenant,
            base_type,
            reverse_chain,
            ChainResolveOptions::default(),
        )
        .await
    }
}

/// Parses a trait-API chain path (`subject:Patient.organization.name`) and its
/// raw value into the chained [`SearchParameter`] the shared resolver takes:
/// one hop per reference segment, each with its optional `:Type` qualifier,
/// and the value split into its raw OR alternatives. The resolver types and
/// parses those once it knows the terminal parameter.
///
/// Returns `None` when `chain` has no reference hop.
fn parse_chain_path(chain: &str, value: &str) -> Option<SearchParameter> {
    let segments: Vec<(&str, Option<&str>)> = chain
        .split('.')
        .map(|segment| match segment.split_once(':') {
            Some((name, target_type)) => (name, Some(target_type)),
            None => (segment, None),
        })
        .collect();
    if segments.len() < 2 {
        return None;
    }

    let hops = segments
        .windows(2)
        .map(|pair| crate::types::ChainedParameter {
            reference_param: pair[0].0.to_string(),
            target_type: pair[0].1.map(str::to_string),
            target_param: pair[1].0.to_string(),
        })
        .collect();

    Some(SearchParameter {
        name: segments[0].0.to_string(),
        param_type: SearchParamType::Reference,
        modifier: None,
        values: crate::search::split_unescaped_commas(value)
            .into_iter()
            .map(SearchValue::eq)
            .collect(),
        chain: hops,
        components: vec![],
    })
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
        // Advertise `transaction` only when the primary can actually honour it.
        // This was inserted unconditionally, so an `s3-elasticsearch`
        // deployment published transaction support in its CapabilityStatement
        // while the S3 primary had no way to roll back (#489). `batch` is
        // unconditional — every bundle provider can do independent entries.
        if self.supports_atomic_transactions() {
            system_interactions.insert(crate::core::SystemInteraction::Transaction);
        }
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

/// Returns an `UnsupportedCapability` error for export operations when the
/// primary backend does not implement the export provider traits.
fn export_unsupported() -> StorageError {
    StorageError::Backend(BackendError::UnsupportedCapability {
        backend_name: "composite".to_string(),
        capability: "bulk-export".to_string(),
    })
}

#[async_trait]
impl ExportDataProvider for CompositeStorage {
    async fn list_export_types(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
    ) -> StorageResult<Vec<String>> {
        match &self.export_provider {
            Some(p) => p.list_export_types(tenant, request).await,
            None => Err(export_unsupported()),
        }
    }

    async fn count_export_resources(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        resource_type: &str,
    ) -> StorageResult<u64> {
        match &self.export_provider {
            Some(p) => {
                p.count_export_resources(tenant, request, resource_type)
                    .await
            }
            None => Err(export_unsupported()),
        }
    }

    async fn fetch_export_batch(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        resource_type: &str,
        cursor: Option<&str>,
        batch_size: u32,
    ) -> StorageResult<NdjsonBatch> {
        match &self.export_provider {
            Some(p) => {
                p.fetch_export_batch(tenant, request, resource_type, cursor, batch_size)
                    .await
            }
            None => Err(export_unsupported()),
        }
    }
}

#[async_trait]
impl PatientExportProvider for CompositeStorage {
    async fn list_patient_ids(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        cursor: Option<&str>,
        batch_size: u32,
    ) -> StorageResult<(Vec<String>, Option<String>)> {
        match &self.export_provider {
            Some(p) => {
                p.list_patient_ids(tenant, request, cursor, batch_size)
                    .await
            }
            None => Err(export_unsupported()),
        }
    }

    async fn fetch_patient_compartment_batch(
        &self,
        tenant: &TenantContext,
        request: &ExportRequest,
        resource_type: &str,
        patient_ids: &[String],
        cursor: Option<&str>,
        batch_size: u32,
    ) -> StorageResult<NdjsonBatch> {
        match &self.export_provider {
            Some(p) => {
                p.fetch_patient_compartment_batch(
                    tenant,
                    request,
                    resource_type,
                    patient_ids,
                    cursor,
                    batch_size,
                )
                .await
            }
            None => Err(export_unsupported()),
        }
    }
}

/// Purge fans out across the composite, **secondaries first**.
///
/// This deliberately inverts the ordering of [`CompositeStorage::delete`], which
/// writes the primary and then syncs to secondaries best-effort, warning on
/// failure. That is defensible for a soft delete: a stale Elasticsearch document
/// is a searchability bug, recoverable by a reindex.
///
/// It is not defensible for a purge. The Elasticsearch document holds the full
/// resource content, so a purge that destroys the primary copy and then fails to
/// remove the secondary has permanently lost the system of record while leaving
/// the data searchable — the exact opposite of what the caller asked for, and a
/// PHI-retention bug. Purging the secondaries first means the worst case is a
/// resource that is missing from search but still readable, which a `$reindex`
/// restores.
///
/// Any failure aborts and propagates, naming the backend that refused. Purge is
/// never partially successful.
#[async_trait]
impl PurgableStorage for CompositeStorage {
    async fn purge(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        let Some(primary) = &self.purgable_primary else {
            return Err(purge_unsupported());
        };

        for (backend_id, secondary) in &self.purgable_secondaries {
            secondary
                .purge(tenant, resource_type, id)
                .await
                .map_err(|e| purge_failed(backend_id, e))?;
        }

        // A resource already absent from the primary but present in a secondary
        // is the state a previously-failed purge leaves behind. Treating that as
        // NotFound would make the retry fail forever, so the secondaries above
        // run first and this tolerates the missing primary copy.
        match primary.purge(tenant, resource_type, id).await {
            Err(StorageError::Resource(ResourceError::NotFound { .. }))
                if !self.purgable_secondaries.is_empty() =>
            {
                Ok(())
            }
            other => other,
        }
    }

    async fn purge_all(&self, tenant: &TenantContext, resource_type: &str) -> StorageResult<u64> {
        let Some(primary) = &self.purgable_primary else {
            return Err(purge_unsupported());
        };

        for (backend_id, secondary) in &self.purgable_secondaries {
            secondary
                .purge_all(tenant, resource_type)
                .await
                .map_err(|e| purge_failed(backend_id, e))?;
        }

        // The primary is the system of record, so its count is the count.
        primary.purge_all(tenant, resource_type).await
    }
}

/// The composite was built without purge handles — `with_purgable_backends` was
/// never called, so there is nothing to purge through.
fn purge_unsupported() -> StorageError {
    StorageError::Backend(BackendError::UnsupportedCapability {
        backend_name: "composite".to_string(),
        capability: "purge".to_string(),
    })
}

/// Names the backend that refused a purge, so an operator knows which store is
/// now inconsistent with the others.
fn purge_failed(backend_id: &str, source: StorageError) -> StorageError {
    StorageError::Backend(BackendError::QueryError {
        message: format!("purge failed on secondary '{backend_id}': {source}"),
    })
}

#[async_trait]
impl GroupExportProvider for CompositeStorage {
    async fn get_group_members(
        &self,
        tenant: &TenantContext,
        group_id: &str,
    ) -> StorageResult<Vec<String>> {
        match &self.export_provider {
            Some(p) => p.get_group_members(tenant, group_id).await,
            None => Err(export_unsupported()),
        }
    }

    async fn resolve_group_patient_ids(
        &self,
        tenant: &TenantContext,
        group_id: &str,
    ) -> StorageResult<Vec<String>> {
        match &self.export_provider {
            Some(p) => p.resolve_group_patient_ids(tenant, group_id).await,
            None => Err(export_unsupported()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{BackendKind, CapabilityProvider};
    use crate::error::{BackendError, StorageError, StorageResult};
    use crate::tenant::{TenantContext, TenantId, TenantPermissions};
    use crate::types::{
        Page, PageInfo, SearchParamType, SearchParameter, SearchQuery, SearchValue, StoredResource,
        StoredResourceBuilder,
    };
    use async_trait::async_trait;
    use helios_fhir::FhirVersion;
    use serde_json::{Value, json};
    use std::sync::Mutex;

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

        fn search_param_registry(
            &self,
            _tenant: &crate::tenant::TenantContext,
        ) -> std::sync::Arc<parking_lot::RwLock<crate::search::SearchParameterRegistry>> {
            std::sync::Arc::new(parking_lot::RwLock::new(
                crate::search::SearchParameterRegistry::new(),
            ))
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

    /// Mock storage with an in-memory tenant registry and a configurable purge.
    #[derive(Default)]
    struct MockRegistryStorage {
        tenants: std::sync::Mutex<Vec<crate::core::TenantRecord>>,
        purge_count: u64,
        fail_purge: bool,
    }

    #[async_trait]
    impl ResourceStorage for MockRegistryStorage {
        fn backend_name(&self) -> &'static str {
            "registry-mock"
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

        fn supports_tenant_registry(&self) -> bool {
            true
        }

        async fn list_tenants(&self) -> StorageResult<Vec<crate::core::TenantRecord>> {
            Ok(self.tenants.lock().unwrap().clone())
        }

        async fn get_tenant(&self, id: &str) -> StorageResult<Option<crate::core::TenantRecord>> {
            Ok(self
                .tenants
                .lock()
                .unwrap()
                .iter()
                .find(|t| t.id == id)
                .cloned())
        }

        async fn register_tenant(
            &self,
            id: &str,
            display_name: Option<&str>,
        ) -> StorageResult<crate::core::TenantRecord> {
            let record = crate::core::TenantRecord {
                id: id.to_string(),
                display_name: display_name.map(str::to_string),
                created_at: "2026-01-01T00:00:00Z".to_string(),
            };
            self.tenants.lock().unwrap().push(record.clone());
            Ok(record)
        }

        async fn deregister_tenant(&self, id: &str) -> StorageResult<bool> {
            let mut tenants = self.tenants.lock().unwrap();
            let before = tenants.len();
            tenants.retain(|t| t.id != id);
            Ok(tenants.len() < before)
        }

        async fn purge_tenant_data(&self, _id: &str) -> StorageResult<u64> {
            if self.fail_purge {
                return Err(StorageError::Backend(BackendError::ConnectionFailed {
                    backend_name: self.backend_name().to_string(),
                    message: "purge failed".to_string(),
                }));
            }
            Ok(self.purge_count)
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

    // ── PurgableStorage fan-out ────────────────────────────────────

    /// Records purge calls, and optionally refuses them.
    struct SpyPurgable {
        name: &'static str,
        calls: Arc<parking_lot::Mutex<Vec<String>>>,
        fail: bool,
    }

    impl SpyPurgable {
        fn new(name: &'static str, calls: Arc<parking_lot::Mutex<Vec<String>>>) -> Arc<Self> {
            Arc::new(Self {
                name,
                calls,
                fail: false,
            })
        }

        fn failing(name: &'static str, calls: Arc<parking_lot::Mutex<Vec<String>>>) -> Arc<Self> {
            Arc::new(Self {
                name,
                calls,
                fail: true,
            })
        }
    }

    // PurgableStorage: ResourceStorage, so the spy has to be one. Only purge
    // is exercised; the CRUD surface delegates to MockStorage's behavior.
    #[async_trait]
    impl ResourceStorage for SpyPurgable {
        fn backend_name(&self) -> &'static str {
            "spy"
        }

        async fn create(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            resource: Value,
            fhir_version: FhirVersion,
        ) -> StorageResult<StoredResource> {
            MockStorage
                .create(tenant, resource_type, resource, fhir_version)
                .await
        }

        async fn create_or_update(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            id: &str,
            resource: Value,
            fhir_version: FhirVersion,
        ) -> StorageResult<(StoredResource, bool)> {
            MockStorage
                .create_or_update(tenant, resource_type, id, resource, fhir_version)
                .await
        }

        async fn read(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            id: &str,
        ) -> StorageResult<Option<StoredResource>> {
            MockStorage.read(tenant, resource_type, id).await
        }

        async fn update(
            &self,
            tenant: &TenantContext,
            current: &StoredResource,
            resource: Value,
        ) -> StorageResult<StoredResource> {
            MockStorage.update(tenant, current, resource).await
        }

        async fn delete(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            id: &str,
        ) -> StorageResult<()> {
            MockStorage.delete(tenant, resource_type, id).await
        }

        async fn count(
            &self,
            tenant: &TenantContext,
            resource_type: Option<&str>,
        ) -> StorageResult<u64> {
            MockStorage.count(tenant, resource_type).await
        }
    }

    #[async_trait]
    impl PurgableStorage for SpyPurgable {
        async fn purge(&self, _t: &TenantContext, rt: &str, id: &str) -> StorageResult<()> {
            self.calls.lock().push(format!("{}:{rt}/{id}", self.name));
            if self.fail {
                return Err(StorageError::Backend(BackendError::QueryError {
                    message: "simulated purge outage".to_string(),
                }));
            }
            Ok(())
        }

        async fn purge_all(&self, _t: &TenantContext, rt: &str) -> StorageResult<u64> {
            self.calls.lock().push(format!("{}:{rt}/*", self.name));
            if self.fail {
                return Err(StorageError::Backend(BackendError::QueryError {
                    message: "simulated purge outage".to_string(),
                }));
            }
            Ok(1)
        }
    }

    fn purge_tenant() -> TenantContext {
        TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
    }

    fn composite_with_purge(
        primary: Arc<SpyPurgable>,
        secondary: Arc<SpyPurgable>,
    ) -> CompositeStorage {
        let mut backends = HashMap::new();
        backends.insert("primary".to_string(), Arc::new(MockStorage) as DynStorage);
        backends.insert("es".to_string(), Arc::new(MockStorage) as DynStorage);
        let config = CompositeConfig::builder()
            .primary("primary", BackendKind::Sqlite)
            .search_backend("es", BackendKind::Elasticsearch)
            .build()
            .unwrap();

        CompositeStorage::new(config, backends)
            .unwrap()
            .with_purgable_backends(
                primary as DynPurgableStorage,
                vec![("es".to_string(), secondary as DynPurgableStorage)],
            )
    }

    /// The bug this whole fan-out exists to prevent: purging only the primary
    /// leaves the resource in the Elasticsearch index, where it stays
    /// searchable and keeps holding the resource's full content.
    #[tokio::test]
    async fn readiness_check_delegates_to_primary() {
        // Composite readiness gates on the primary system-of-record only. The
        // primary mock leaves the trait's default `readiness_check` (Ok) in
        // place, so a healthy primary makes the composite ready — this exercises
        // both the composite delegation and the default trait impl.
        let composite = make_composite_no_secondary();
        composite
            .readiness_check()
            .await
            .expect("a composite over a healthy primary must be ready");
    }

    #[tokio::test]
    async fn test_purge_reaches_every_backend() {
        let calls = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let composite = composite_with_purge(
            SpyPurgable::new("primary", calls.clone()),
            SpyPurgable::new("es", calls.clone()),
        );

        composite
            .purge(&purge_tenant(), "Patient", "p1")
            .await
            .unwrap();

        assert_eq!(
            *calls.lock(),
            ["es:Patient/p1", "primary:Patient/p1"],
            "secondaries must be purged before the primary, and none may be skipped"
        );
    }

    /// If a secondary refuses, the primary must be left alone. Destroying the
    /// system of record and then failing to remove the searchable copy is the
    /// worst possible outcome: the data is both lost and still exposed.
    #[tokio::test]
    async fn test_purge_aborts_before_primary_when_secondary_fails() {
        let calls = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let composite = composite_with_purge(
            SpyPurgable::new("primary", calls.clone()),
            SpyPurgable::failing("es", calls.clone()),
        );

        let err = composite
            .purge(&purge_tenant(), "Patient", "p1")
            .await
            .expect_err("a failing secondary must fail the whole purge");

        assert!(
            err.to_string().contains("es"),
            "the error must name the backend that refused, got: {err}"
        );
        assert_eq!(
            *calls.lock(),
            ["es:Patient/p1"],
            "the primary must not be touched once a secondary has failed"
        );
    }

    /// A composite built without purge handles must say so rather than
    /// silently purging the primary alone.
    #[tokio::test]
    async fn test_purge_unsupported_without_handles() {
        let composite = make_composite_with_secondary();
        let err = composite
            .purge(&purge_tenant(), "Patient", "p1")
            .await
            .expect_err("purge without wired handles must be unsupported");
        assert!(matches!(
            err,
            StorageError::Backend(BackendError::UnsupportedCapability { .. })
        ));
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

    /// A fake resource storage that also acts as a search provider. It
    /// records every `search` query it receives (so tests can assert which
    /// backend answered, and with which parameters) and returns a fixed set
    /// of results with the total set to the number of results.
    struct RecordingSearchProvider {
        name: &'static str,
        calls: Mutex<Vec<SearchQuery>>,
        results: Vec<StoredResource>,
        registry: Option<Arc<parking_lot::RwLock<crate::search::SearchParameterRegistry>>>,
    }

    impl RecordingSearchProvider {
        fn new(name: &'static str, results: Vec<StoredResource>) -> Self {
            Self {
                name,
                calls: Mutex::new(Vec::new()),
                results,
                registry: None,
            }
        }

        /// Attaches a search-parameter registry so `search_param_registry`
        /// returns it instead of the empty default — used by tests that need
        /// this fake to answer registry-driven lookups (e.g. include
        /// resolution via `resolve_includes_iterative`).
        fn with_registry(
            mut self,
            registry: Arc<parking_lot::RwLock<crate::search::SearchParameterRegistry>>,
        ) -> Self {
            self.registry = Some(registry);
            self
        }

        fn call_count(&self) -> usize {
            self.calls.lock().expect("calls mutex poisoned").len()
        }
    }

    #[async_trait]
    impl ResourceStorage for RecordingSearchProvider {
        fn backend_name(&self) -> &'static str {
            self.name
        }

        async fn create(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<StoredResource> {
            Err(StorageError::Backend(BackendError::UnsupportedCapability {
                backend_name: self.name.to_string(),
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
                backend_name: self.name.to_string(),
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
                backend_name: self.name.to_string(),
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
                backend_name: self.name.to_string(),
                capability: "delete".to_string(),
            }))
        }

        async fn count(
            &self,
            _tenant: &TenantContext,
            _resource_type: Option<&str>,
        ) -> StorageResult<u64> {
            Ok(self.results.len() as u64)
        }
    }

    #[async_trait]
    impl SearchProvider for RecordingSearchProvider {
        async fn search(
            &self,
            _tenant: &TenantContext,
            query: &SearchQuery,
        ) -> StorageResult<SearchResult> {
            self.calls
                .lock()
                .expect("calls mutex poisoned")
                .push(query.clone());
            Ok(
                SearchResult::new(Page::new(self.results.clone(), PageInfo::end()))
                    .with_total(self.results.len() as u64),
            )
        }

        async fn search_count(
            &self,
            _tenant: &TenantContext,
            _query: &SearchQuery,
        ) -> StorageResult<u64> {
            Ok(self.results.len() as u64)
        }

        fn search_param_registry(
            &self,
            _tenant: &TenantContext,
        ) -> std::sync::Arc<parking_lot::RwLock<crate::search::SearchParameterRegistry>> {
            self.registry.clone().unwrap_or_else(|| {
                std::sync::Arc::new(parking_lot::RwLock::new(
                    crate::search::SearchParameterRegistry::new(),
                ))
            })
        }
    }

    /// Builds a fake `Patient` resource with the given id, for use as a
    /// dedicated-search-backend fixture.
    fn fake_patient(id: &str) -> StoredResource {
        StoredResourceBuilder::new()
            .resource_type("Patient")
            .id(id)
            .tenant_id(TenantId::new("composite-test"))
            .content(json!({
                "resourceType": "Patient",
                "id": id,
            }))
            .build()
    }

    /// Builds a composite whose config has a primary and a dedicated
    /// Search-role backend, backed by [`RecordingSearchProvider`] fakes so
    /// tests can assert which backend answered a query.
    fn make_composite_with_dedicated_search(
        primary_results: Vec<StoredResource>,
        search_results: Vec<StoredResource>,
    ) -> (
        CompositeStorage,
        Arc<RecordingSearchProvider>,
        Arc<RecordingSearchProvider>,
    ) {
        let primary = Arc::new(RecordingSearchProvider::new("primary", primary_results));
        let search = Arc::new(RecordingSearchProvider::new("search", search_results));

        let composite_config = CompositeConfig::builder()
            .primary("primary", BackendKind::Sqlite)
            .search_backend("search", BackendKind::Sqlite)
            .build()
            .expect("build composite config");

        let mut backends: HashMap<String, DynStorage> = HashMap::new();
        backends.insert("primary".to_string(), primary.clone() as DynStorage);
        backends.insert("search".to_string(), search.clone() as DynStorage);

        let mut search_providers: HashMap<String, DynSearchProvider> = HashMap::new();
        search_providers.insert("primary".to_string(), primary.clone() as DynSearchProvider);
        search_providers.insert("search".to_string(), search.clone() as DynSearchProvider);

        let composite = CompositeStorage::new(composite_config, backends)
            .expect("create composite storage")
            .with_search_providers(search_providers);

        (composite, primary, search)
    }

    /// Builds a composite whose config has only a primary backend (no
    /// dedicated Search role), backed by a single [`RecordingSearchProvider`]
    /// fake.
    fn make_composite_no_search_backend(
        primary_results: Vec<StoredResource>,
    ) -> (CompositeStorage, Arc<RecordingSearchProvider>) {
        let primary = Arc::new(RecordingSearchProvider::new("primary", primary_results));

        let composite_config = CompositeConfig::builder()
            .primary("primary", BackendKind::Sqlite)
            .build()
            .expect("build composite config");

        let mut backends: HashMap<String, DynStorage> = HashMap::new();
        backends.insert("primary".to_string(), primary.clone() as DynStorage);

        let mut search_providers: HashMap<String, DynSearchProvider> = HashMap::new();
        search_providers.insert("primary".to_string(), primary.clone() as DynSearchProvider);

        let composite = CompositeStorage::new(composite_config, backends)
            .expect("create composite storage")
            .with_search_providers(search_providers);

        (composite, primary)
    }

    #[tokio::test]
    async fn resolve_includes_delegates_to_shared_resolver_via_search_backend() {
        use crate::search::{SearchParameterDefinition, SearchParameterRegistry};
        use crate::types::IncludeType;

        let tenant = TenantContext::new(
            TenantId::new("composite-test"),
            TenantPermissions::full_access(),
        );

        let registry = Arc::new(parking_lot::RwLock::new(SearchParameterRegistry::new()));
        registry
            .write()
            .register(
                SearchParameterDefinition::new(
                    "http://hl7.org/fhir/SearchParameter/Patient-organization",
                    "organization",
                    SearchParamType::Reference,
                    "Patient.managingOrganization",
                )
                .with_base(vec!["Patient"])
                .with_targets(vec!["Organization"]),
            )
            .unwrap();

        let org = StoredResourceBuilder::new()
            .resource_type("Organization")
            .id("org-1")
            .tenant_id(TenantId::new("composite-test"))
            .content(json!({
                "resourceType": "Organization",
                "id": "org-1",
            }))
            .build();

        let primary = Arc::new(RecordingSearchProvider::new("primary", vec![]));
        let search = Arc::new(
            RecordingSearchProvider::new("search", vec![org.clone()]).with_registry(registry),
        );

        let composite_config = CompositeConfig::builder()
            .primary("primary", BackendKind::Sqlite)
            .search_backend("search", BackendKind::Sqlite)
            .build()
            .expect("build composite config");

        let mut backends: HashMap<String, DynStorage> = HashMap::new();
        backends.insert("primary".to_string(), primary.clone() as DynStorage);
        backends.insert("search".to_string(), search.clone() as DynStorage);

        let mut search_providers: HashMap<String, DynSearchProvider> = HashMap::new();
        search_providers.insert("primary".to_string(), primary.clone() as DynSearchProvider);
        search_providers.insert("search".to_string(), search.clone() as DynSearchProvider);

        let composite = CompositeStorage::new(composite_config, backends)
            .expect("create composite storage")
            .with_search_providers(search_providers);

        let patient = StoredResourceBuilder::new()
            .resource_type("Patient")
            .id("p1")
            .tenant_id(TenantId::new("composite-test"))
            .content(json!({
                "resourceType": "Patient",
                "id": "p1",
                "managingOrganization": {"reference": "Organization/org-1"},
            }))
            .build();

        let include = IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Patient".to_string(),
            search_param: "organization".to_string(),
            target_type: None,
            iterate: false,
        };

        let included = composite
            .resolve_includes(
                &tenant,
                std::slice::from_ref(&patient),
                std::slice::from_ref(&include),
            )
            .await
            .expect("resolve_includes should succeed");

        assert_eq!(included.len(), 1);
        assert_eq!(included[0].resource_type(), "Organization");
        assert_eq!(included[0].id(), "org-1");

        {
            let calls = search.calls.lock().expect("calls mutex poisoned");
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].resource_type, "Organization");
            assert_eq!(calls[0].parameters.len(), 1);
            assert_eq!(calls[0].parameters[0].name, "_id");
            assert_eq!(calls[0].parameters[0].values.len(), 1);
            assert_eq!(calls[0].parameters[0].values[0].value, "org-1");
        }
        assert_eq!(primary.call_count(), 0);

        // A target-type filter that doesn't match the reference's resource
        // type yields no included resources and never issues a search — the
        // shared resolver skips fetching when nothing survives the filter.
        let include_wrong_target = IncludeDirective {
            target_type: Some("Practitioner".to_string()),
            ..include
        };
        let included_filtered = composite
            .resolve_includes(
                &tenant,
                std::slice::from_ref(&patient),
                std::slice::from_ref(&include_wrong_target),
            )
            .await
            .expect("resolve_includes should succeed");

        assert_eq!(included_filtered.len(), 0);
        assert_eq!(search.call_count(), 1);
        assert_eq!(primary.call_count(), 0);
    }

    #[tokio::test]
    async fn test_full_text_query_is_answered_whole_by_dedicated_search_backend() {
        let tenant = TenantContext::new(
            TenantId::new("composite-test"),
            TenantPermissions::full_access(),
        );

        let search_results = vec![
            fake_patient("patient-1"),
            fake_patient("patient-2"),
            fake_patient("patient-3"),
        ];
        let (composite, primary, search) =
            make_composite_with_dedicated_search(vec![], search_results);

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_content".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::string("everett")],
            chain: vec![],
            components: vec![],
        });

        let result = composite
            .search(&tenant, &query)
            .await
            .expect("composite search should succeed");

        assert_eq!(result.resources.len(), 3);
        assert_eq!(result.total, Some(3));
        assert_eq!(search.call_count(), 1);
        assert_eq!(primary.call_count(), 0);
    }

    #[tokio::test]
    async fn test_mixed_query_reaches_dedicated_search_backend_intact() {
        let tenant = TenantContext::new(
            TenantId::new("composite-test"),
            TenantPermissions::full_access(),
        );

        let search_results = vec![
            fake_patient("patient-1"),
            fake_patient("patient-2"),
            fake_patient("patient-3"),
        ];
        let (composite, primary, search) =
            make_composite_with_dedicated_search(vec![], search_results);

        let query = SearchQuery::new("Patient")
            .with_parameter(SearchParameter {
                name: "_content".to_string(),
                param_type: SearchParamType::String,
                modifier: None,
                values: vec![SearchValue::string("everett")],
                chain: vec![],
                components: vec![],
            })
            .with_parameter(SearchParameter {
                name: "gender".to_string(),
                param_type: SearchParamType::Token,
                modifier: None,
                values: vec![SearchValue::eq("female")],
                chain: vec![],
                components: vec![],
            });

        let result = composite
            .search(&tenant, &query)
            .await
            .expect("composite search should succeed");

        assert_eq!(result.total, Some(3));
        assert_eq!(primary.call_count(), 0);
        assert_eq!(search.call_count(), 1);

        let recorded = search.calls.lock().expect("calls mutex poisoned");
        let recorded_query = recorded.first().expect("search should have been called");
        let names: Vec<&str> = recorded_query
            .parameters
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert!(
            names.contains(&"_content"),
            "recorded query should keep the _content parameter"
        );
        assert!(
            names.contains(&"gender"),
            "recorded query should keep the gender parameter"
        );
    }

    #[tokio::test]
    async fn test_full_text_query_without_dedicated_search_backend_is_unchanged() {
        // No Search-role backend is configured, so the guard added for #1012
        // never triggers here: this exercises the pre-existing routing path
        // (single backend, no auxiliary targets), unchanged by this fix.
        let tenant = TenantContext::new(
            TenantId::new("composite-test"),
            TenantPermissions::full_access(),
        );

        let primary_results = vec![fake_patient("patient-1"), fake_patient("patient-2")];
        let (composite, primary) = make_composite_no_search_backend(primary_results);

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_text".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::string("everett")],
            chain: vec![],
            components: vec![],
        });

        let result = composite
            .search(&tenant, &query)
            .await
            .expect("composite search should succeed from the primary backend");

        assert_eq!(result.resources.len(), 2);
        assert_eq!(primary.call_count(), 1);
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

    #[test]
    fn test_bulk_write_concurrency_delegates_to_primary() {
        let composite = make_composite_no_secondary();
        assert_eq!(
            composite.bulk_write_concurrency(),
            composite.primary().bulk_write_concurrency()
        );
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

    /// #1078: `supports_type_counts` defaults to `false` (a backend that keeps
    /// the empty count defaults, like `MockStorage`), and composite storage
    /// reports its primary's answer.
    #[test]
    fn test_supports_type_counts_defaults_false_and_follows_the_primary() {
        assert!(!MockStorage.supports_type_counts());
        assert!(!make_composite_no_secondary().supports_type_counts());
        assert!(
            !make_composite_with_secondary().supports_type_counts(),
            "a secondary never answers for the primary"
        );
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_supports_type_counts_is_true_over_a_sqlite_primary() {
        let sqlite = crate::backends::sqlite::SqliteBackend::in_memory().unwrap();
        let config = CompositeConfig::builder()
            .primary("primary", BackendKind::Sqlite)
            .search_backend("es", BackendKind::Elasticsearch)
            .build()
            .unwrap();
        let mut backends = HashMap::new();
        backends.insert("primary".to_string(), Arc::new(sqlite) as DynStorage);
        backends.insert("es".to_string(), Arc::new(MockStorage) as DynStorage);
        let composite = CompositeStorage::new(config, backends).unwrap();
        assert!(composite.supports_type_counts());
    }

    /// #1078: `latest_write_marker` defaults to `None` (a backend that cannot
    /// probe its history cheaply, like `MockStorage`), and composite storage
    /// reports its primary's answer, never a secondary's.
    #[tokio::test]
    async fn test_latest_write_marker_defaults_none_and_follows_the_primary() {
        let tenant = make_tenant();
        let since = Some(chrono::Utc::now());
        assert_eq!(
            MockStorage
                .latest_write_marker(&tenant, since)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            make_composite_no_secondary()
                .latest_write_marker(&tenant, since)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            make_composite_with_secondary()
                .latest_write_marker(&tenant, since)
                .await
                .unwrap(),
            None,
            "a secondary never answers for the primary"
        );
    }

    /// #1078: over a SQLite primary the composite's marker is the primary's.
    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_latest_write_marker_delegates_to_primary() {
        let sqlite = Arc::new(crate::backends::sqlite::SqliteBackend::in_memory().unwrap());
        sqlite.init_schema().unwrap();
        let config = CompositeConfig::builder()
            .primary("primary", BackendKind::Sqlite)
            .search_backend("es", BackendKind::Elasticsearch)
            .build()
            .unwrap();
        let mut backends = HashMap::new();
        backends.insert("primary".to_string(), sqlite.clone() as DynStorage);
        backends.insert("es".to_string(), Arc::new(MockStorage) as DynStorage);
        let composite = CompositeStorage::new(config, backends).unwrap();

        let tenant = make_tenant();
        let since = Some(chrono::Utc::now() - chrono::Duration::hours(1));
        let created = sqlite
            .create(
                &tenant,
                "Patient",
                serde_json::json!({ "resourceType": "Patient" }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let via_composite = composite.latest_write_marker(&tenant, since).await.unwrap();
        let via_primary = sqlite.latest_write_marker(&tenant, since).await.unwrap();
        assert_eq!(via_composite, via_primary);
        assert_eq!(
            via_composite,
            Some(crate::core::WriteMarker {
                latest: Some(created.last_modified()),
                recent_writes: Some(1),
            })
        );
    }

    /// #1078: `count_deltas_by_type_and_bucket` is answered by the primary (its
    /// grouped query), never the search secondary.
    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_count_deltas_by_type_and_bucket_delegates_to_primary() {
        use crate::core::ResourceStorage;
        let sqlite = Arc::new(crate::backends::sqlite::SqliteBackend::in_memory().unwrap());
        sqlite.init_schema().unwrap();
        let config = CompositeConfig::builder()
            .primary("primary", BackendKind::Sqlite)
            .search_backend("es", BackendKind::Elasticsearch)
            .build()
            .unwrap();
        let mut backends = HashMap::new();
        backends.insert("primary".to_string(), sqlite.clone() as DynStorage);
        backends.insert("es".to_string(), Arc::new(MockStorage) as DynStorage);
        let composite = CompositeStorage::new(config, backends).unwrap();

        let tenant = make_tenant();
        for rt in ["Patient", "Patient", "Observation"] {
            sqlite
                .create(
                    &tenant,
                    rt,
                    serde_json::json!({ "resourceType": rt }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        let since = chrono::Utc::now() - chrono::Duration::hours(1);
        let types = ["Patient", "Observation", "Encounter"];
        let via_composite = composite
            .count_deltas_by_type_and_bucket(&tenant, &types, since, 3600)
            .await
            .unwrap();
        let via_primary = sqlite
            .count_deltas_by_type_and_bucket(&tenant, &types, since, 3600)
            .await
            .unwrap();
        assert_eq!(via_composite, via_primary);
        assert_eq!(via_composite.iter().map(|(_, d)| d.delta).sum::<i64>(), 3);

        // A primary without history keeps the trait's empty answer.
        assert!(
            make_composite_no_secondary()
                .count_deltas_by_type_and_bucket(&tenant, &types, since, 3600)
                .await
                .unwrap()
                .is_empty()
        );
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
    async fn test_bundle_provider_process_transaction_no_capability() {
        use crate::core::BundleProvider;
        let composite = make_composite_no_secondary();
        let tenant = make_tenant();
        let result = composite
            .process_transaction(&tenant, vec![], helios_fhir::FhirVersion::default())
            .await;
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

    // ── Tenant registry delegation ────────────────────────────────

    fn make_composite_registry(
        primary: Arc<MockRegistryStorage>,
        secondary: Option<DynStorage>,
    ) -> CompositeStorage {
        let mut builder = CompositeConfig::builder().primary("primary", BackendKind::Sqlite);
        if secondary.is_some() {
            builder = builder.search_backend("es", BackendKind::Elasticsearch);
        }
        let config = builder.build().unwrap();
        let mut backends = HashMap::new();
        backends.insert("primary".to_string(), primary as DynStorage);
        if let Some(secondary) = secondary {
            backends.insert("es".to_string(), secondary);
        }
        CompositeStorage::new(config, backends).unwrap()
    }

    #[tokio::test]
    async fn test_tenant_registry_delegates_to_primary() {
        use crate::core::ResourceStorage;
        let primary = Arc::new(MockRegistryStorage::default());
        let composite = make_composite_registry(primary.clone(), None);

        assert!(composite.supports_tenant_registry());

        let record = composite
            .register_tenant("acme", Some("Acme Health"))
            .await
            .unwrap();
        assert_eq!(record.id, "acme");
        assert_eq!(record.display_name.as_deref(), Some("Acme Health"));
        assert_eq!(primary.tenants.lock().unwrap().len(), 1);

        assert_eq!(
            composite.get_tenant("acme").await.unwrap(),
            Some(record.clone())
        );
        assert_eq!(composite.list_tenants().await.unwrap(), vec![record]);

        assert!(composite.deregister_tenant("acme").await.unwrap());
        assert!(!composite.deregister_tenant("acme").await.unwrap());
        assert!(composite.get_tenant("acme").await.unwrap().is_none());
        assert!(primary.tenants.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_purge_tenant_data_skips_unsupported_secondary() {
        use crate::core::ResourceStorage;
        let primary = Arc::new(MockRegistryStorage {
            purge_count: 7,
            ..Default::default()
        });
        let composite = make_composite_registry(primary, Some(Arc::new(MockStorage) as DynStorage));

        assert_eq!(composite.purge_tenant_data("acme").await.unwrap(), 7);
    }

    #[tokio::test]
    async fn test_purge_tenant_data_propagates_secondary_failure() {
        use crate::core::ResourceStorage;
        let primary = Arc::new(MockRegistryStorage {
            purge_count: 3,
            ..Default::default()
        });
        let failing = Arc::new(MockRegistryStorage {
            fail_purge: true,
            ..Default::default()
        });
        let composite = make_composite_registry(primary, Some(failing as DynStorage));

        let result = composite.purge_tenant_data("acme").await;
        assert!(matches!(
            result,
            Err(StorageError::Backend(BackendError::ConnectionFailed { .. }))
        ));
    }

    fn make_composite_with_search_provider() -> CompositeStorage {
        let composite = make_composite_no_secondary();
        let mut providers = HashMap::new();
        providers.insert(
            "primary".to_string(),
            Arc::new(MockStorage) as DynSearchProvider,
        );
        composite.with_search_providers(providers)
    }

    /// `resolve_chain` should run iterative SearchQueries against composite's
    /// own search routing rather than the previous stub that returned an
    /// empty Vec for any chain longer than 2 segments. This test uses
    /// MockStorage (which returns no results) so the assertion is just that
    /// the implementation runs to completion and yields empty (not that it
    /// finds anything) — full end-to-end coverage comes from inferno against
    /// real backends.
    #[tokio::test]
    async fn test_resolve_chain_three_segments_does_not_error() {
        use crate::core::ChainedSearchProvider;
        let composite = make_composite_with_search_provider();
        let tenant = make_tenant();
        let result = composite
            .resolve_chain(
                &tenant,
                "Observation",
                "subject.organization.name",
                "Hospital",
            )
            .await;
        assert!(result.is_ok(), "resolve_chain failed: {:?}", result.err());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_resolve_chain_short_chain_does_not_error() {
        use crate::core::ChainedSearchProvider;
        let composite = make_composite_with_search_provider();
        let tenant = make_tenant();
        let result = composite
            .resolve_chain(&tenant, "Observation", "patient.name", "Smith")
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_resolve_chain_invalid_chain_returns_empty() {
        use crate::core::ChainedSearchProvider;
        let composite = make_composite_with_search_provider();
        let tenant = make_tenant();
        // Single segment isn't a chain — must return empty, not error.
        let result = composite
            .resolve_chain(&tenant, "Observation", "patient", "x")
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    // ------------------------------------------------------------------
    // #1304: typed terminal values through the `ChainedSearchProvider` trait
    // API. These run against a real SQLite primary built WITH the spec data
    // dir: without it only the 5 embedded params load, date/quantity params
    // index nothing, and every assertion below would be vacuous.
    // ------------------------------------------------------------------

    #[cfg(feature = "sqlite")]
    async fn make_seeded_chain_composite() -> (CompositeStorage, TenantContext) {
        use crate::backends::sqlite::{SqliteBackend, SqliteBackendConfig};

        let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap();
        let sqlite = Arc::new(
            SqliteBackend::with_config(
                ":memory:",
                SqliteBackendConfig {
                    data_dir: Some(data_dir),
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        sqlite.init_schema().unwrap();

        let tenant = make_tenant();
        let resources = [
            json!({ "resourceType": "Practitioner", "id": "pr1" }),
            json!({ "resourceType": "Practitioner", "id": "pr2" }),
            json!({ "resourceType": "Patient", "id": "p1", "birthDate": "1980-05-06",
                    "name": [{ "family": "Smith" }],
                    "generalPractitioner": [{ "reference": "Practitioner/pr1" }] }),
            json!({ "resourceType": "Patient", "id": "p2", "birthDate": "1990-01-01",
                    "name": [{ "family": "Jones" }],
                    "generalPractitioner": [{ "reference": "Practitioner/pr2" }] }),
            json!({ "resourceType": "Patient", "id": "p3", "birthDate": "1970-01-01",
                    "name": [{ "family": "Brown" }] }),
            json!({ "resourceType": "Observation", "id": "o1", "status": "final",
                    "subject": { "reference": "Patient/p1" },
                    "effectiveDateTime": "2021-06-15T10:00:00Z",
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "8867-4" }] },
                    "valueQuantity": { "value": 7.2, "unit": "mg",
                        "system": "http://unitsofmeasure.org", "code": "mg" } }),
            json!({ "resourceType": "Observation", "id": "o2", "status": "final",
                    "subject": { "reference": "Patient/p2" },
                    "effectiveDateTime": "2019-03-01T10:00:00Z",
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "1234-5" }] },
                    "valueQuantity": { "value": 3.0, "unit": "mg",
                        "system": "http://unitsofmeasure.org", "code": "mg" } }),
            json!({ "resourceType": "Provenance", "id": "prov1",
                    "target": [{ "reference": "Observation/o1" }],
                    "recorded": "2021-06-16T00:00:00Z",
                    "agent": [{ "who": { "reference": "Practitioner/pr1" } }] }),
        ];
        for resource in resources {
            let resource_type = resource["resourceType"].as_str().unwrap().to_string();
            sqlite
                .create(&tenant, &resource_type, resource, FhirVersion::default())
                .await
                .unwrap();
        }

        let config = CompositeConfig::builder()
            .primary("primary", BackendKind::Sqlite)
            .build()
            .unwrap();
        let mut backends = HashMap::new();
        backends.insert("primary".to_string(), sqlite.clone() as DynStorage);
        let mut providers = HashMap::new();
        providers.insert("primary".to_string(), sqlite as DynSearchProvider);
        let composite = CompositeStorage::new(config, backends)
            .unwrap()
            .with_search_providers(providers);
        (composite, tenant)
    }

    /// Positive control: a direct, non-chained typed search through the
    /// composite finds exactly `expected`. Proves the param is registered and
    /// indexed, so a chain test over the same param cannot pass vacuously.
    #[cfg(feature = "sqlite")]
    async fn assert_direct_search(
        composite: &CompositeStorage,
        tenant: &TenantContext,
        resource_type: &str,
        param: &str,
        raw: &str,
        expected: &[&str],
    ) {
        let (param_type, values) = {
            let reg = composite.search_param_registry(tenant);
            let registry = reg.read();
            assert!(
                registry.get_param(resource_type, param).is_some(),
                "{resource_type}.{param} must be registered (data dir not loaded?)"
            );
            crate::search::parse_typed_values(
                &registry,
                resource_type,
                param,
                &crate::search::split_unescaped_commas(raw),
            )
        };
        let query = SearchQuery::new(resource_type).with_parameter(SearchParameter {
            name: param.to_string(),
            param_type,
            modifier: None,
            values,
            chain: vec![],
            components: vec![],
        });
        let mut ids: Vec<String> = composite
            .search(tenant, &query)
            .await
            .unwrap()
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        ids.sort();
        assert_eq!(ids, expected, "control {resource_type}?{param}={raw}");
    }

    /// `base_type?_has:source_type:reference_param:search_param=raw` through
    /// the trait API, sorted and de-duplicated.
    #[cfg(feature = "sqlite")]
    async fn has_ids(
        composite: &CompositeStorage,
        tenant: &TenantContext,
        base_type: &str,
        (source_type, reference_param, search_param): (&str, &str, &str),
        raw: &str,
    ) -> StorageResult<Vec<String>> {
        use crate::core::ChainedSearchProvider;
        let rc = ReverseChainedParameter::terminal(
            source_type,
            reference_param,
            search_param,
            SearchValue::eq(raw),
        );
        let mut ids = composite
            .resolve_reverse_chain(tenant, base_type, &rc)
            .await?;
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    #[cfg(feature = "sqlite")]
    async fn chain_ids(
        composite: &CompositeStorage,
        tenant: &TenantContext,
        chain: &str,
        raw: &str,
    ) -> StorageResult<Vec<String>> {
        use crate::core::ChainedSearchProvider;
        let mut ids = composite
            .resolve_chain(tenant, "Observation", chain, raw)
            .await?;
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    /// (a) `_has:Observation:subject:date=ge…/lt…`
    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_reverse_chain_date_terminal_with_prefix() {
        let (c, t) = make_seeded_chain_composite().await;
        assert_direct_search(&c, &t, "Observation", "date", "ge2020-01-01", &["o1"]).await;
        assert_direct_search(&c, &t, "Observation", "date", "lt2020-01-01", &["o2"]).await;

        let has = ("Observation", "subject", "date");
        let ge = has_ids(&c, &t, "Patient", has, "ge2020-01-01").await;
        let lt = has_ids(&c, &t, "Patient", has, "lt2020-01-01").await;
        assert_eq!(ge.unwrap(), vec!["p1"]);
        assert_eq!(lt.unwrap(), vec!["p2"]);
    }

    /// (b) unprefixed day-precision date must cover the whole day.
    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_reverse_chain_date_terminal_day_precision() {
        let (c, t) = make_seeded_chain_composite().await;
        assert_direct_search(&c, &t, "Observation", "date", "2021-06-15", &["o1"]).await;

        let ids = has_ids(
            &c,
            &t,
            "Patient",
            ("Observation", "subject", "date"),
            "2021-06-15",
        )
        .await;
        assert_eq!(ids.unwrap(), vec!["p1"]);
    }

    /// (c) quantity terminal with comparator prefix.
    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_reverse_chain_quantity_terminal_with_prefix() {
        let (c, t) = make_seeded_chain_composite().await;
        let raw = "gt5.4|http://unitsofmeasure.org|mg";
        assert_direct_search(&c, &t, "Observation", "value-quantity", raw, &["o1"]).await;

        let ids = has_ids(
            &c,
            &t,
            "Patient",
            ("Observation", "subject", "value-quantity"),
            raw,
        )
        .await;
        assert_eq!(ids.unwrap(), vec!["p1"]);
    }

    /// (d) a comma-separated value is an OR list.
    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_reverse_chain_comma_separated_or_list() {
        let (c, t) = make_seeded_chain_composite().await;
        assert_direct_search(
            &c,
            &t,
            "Observation",
            "code",
            "8867-4,1234-5",
            &["o1", "o2"],
        )
        .await;

        let tokens = has_ids(
            &c,
            &t,
            "Patient",
            ("Observation", "subject", "code"),
            "8867-4,1234-5",
        )
        .await;
        let dates = has_ids(
            &c,
            &t,
            "Patient",
            ("Observation", "subject", "date"),
            "2021-06-15,2019-03-01",
        )
        .await;
        assert_eq!(tokens.unwrap(), vec!["p1", "p2"]);
        assert_eq!(dates.unwrap(), vec!["p1", "p2"]);
    }

    /// (e) string terminal keeps default starts-with, case-insensitive matching.
    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_reverse_chain_string_terminal_prefix_match() {
        let (c, t) = make_seeded_chain_composite().await;
        assert_direct_search(&c, &t, "Patient", "family", "smi", &["p1"]).await;

        let ids = has_ids(
            &c,
            &t,
            "Practitioner",
            ("Patient", "general-practitioner", "family"),
            "smi",
        )
        .await;
        assert_eq!(ids.unwrap(), vec!["pr1"]);
    }

    /// (f) regression guard: a token terminal still works.
    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_reverse_chain_token_terminal_still_works() {
        let (c, t) = make_seeded_chain_composite().await;
        assert_direct_search(&c, &t, "Observation", "code", "8867-4", &["o1"]).await;

        let has = ("Observation", "subject", "code");
        let bare = has_ids(&c, &t, "Patient", has, "8867-4").await;
        let system = has_ids(&c, &t, "Patient", has, "http://loinc.org|1234-5").await;
        assert_eq!(bare.unwrap(), vec!["p1"]);
        assert_eq!(system.unwrap(), vec!["p2"]);
    }

    /// Nested `_has` is resolved recursively rather than searched as an
    /// empty-named parameter.
    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_reverse_chain_nested_has() {
        use crate::core::ChainedSearchProvider;
        let (c, t) = make_seeded_chain_composite().await;
        assert_direct_search(&c, &t, "Provenance", "recorded", "ge2021-01-01", &["prov1"]).await;
        // Patient?_has:Observation:subject:_has:Provenance:target:recorded=ge2021-01-01
        let rc = ReverseChainedParameter::nested(
            "Observation",
            "subject",
            ReverseChainedParameter::terminal(
                "Provenance",
                "target",
                "recorded",
                SearchValue::eq("ge2021-01-01"),
            ),
        );
        let ids = c.resolve_reverse_chain(&t, "Patient", &rc).await;
        assert_eq!(ids.unwrap(), vec!["p1"]);
    }

    /// An unregistered terminal param behaves exactly as it does in the shared
    /// resolver (typed by the value-shape fallback, never a hardcoded Token).
    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_reverse_chain_unknown_terminal_param_matches_shared_resolver() {
        let (c, t) = make_seeded_chain_composite().await;
        let has = ("Observation", "subject", "no-such-param");
        let ids = has_ids(&c, &t, "Patient", has, "x").await;

        let mut query = SearchQuery::new("Patient");
        query.reverse_chains.push(ReverseChainedParameter::terminal(
            has.0,
            has.1,
            has.2,
            SearchValue::eq("x"),
        ));
        let shared = crate::search::resolve_chains(&c, &t, &query).await;
        assert_eq!(ids.is_ok(), shared.is_ok());
        if let Ok(ids) = ids {
            assert!(ids.is_empty());
        }
    }

    /// Forward chain: prefixed date terminal, comma OR list, `:Type` qualifier.
    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_forward_chain_typed_terminal_values() {
        let (c, t) = make_seeded_chain_composite().await;
        assert_direct_search(&c, &t, "Patient", "birthdate", "ge1985-01-01", &["p2"]).await;

        let ge = chain_ids(&c, &t, "subject.birthdate", "ge1985-01-01").await;
        let lt = chain_ids(&c, &t, "subject:Patient.birthdate", "lt1985-01-01").await;
        let day = chain_ids(&c, &t, "subject.birthdate", "1980-05-06").await;
        let dates = chain_ids(&c, &t, "subject.birthdate", "1980-05-06,1990-01-01").await;
        let strings = chain_ids(&c, &t, "subject.family", "smi,jon").await;
        let literal = chain_ids(&c, &t, "subject.family", "smith").await;

        assert_eq!(ge.unwrap(), vec!["o2"]);
        assert_eq!(lt.unwrap(), vec!["o1"]);
        assert_eq!(day.unwrap(), vec!["o1"]);
        assert_eq!(dates.unwrap(), vec!["o1", "o2"]);
        assert_eq!(strings.unwrap(), vec!["o1", "o2"]);
        assert_eq!(literal.unwrap(), vec!["o1"]);
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

        fn search_param_registry(
            &self,
            _tenant: &crate::tenant::TenantContext,
        ) -> std::sync::Arc<parking_lot::RwLock<crate::search::SearchParameterRegistry>> {
            std::sync::Arc::new(parking_lot::RwLock::new(
                crate::search::SearchParameterRegistry::new(),
            ))
        }
    }
}
