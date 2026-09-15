//! Synchronization for secondary backends.
//!
//! This module provides synchronization mechanisms to keep secondary backends
//! in sync with the primary backend.
//!
//! # Sync Modes
//!
//! | Mode | Description | Latency | Consistency |
//! |------|-------------|---------|-------------|
//! | Synchronous | Update secondaries in same operation | Higher | Strong |
//! | Asynchronous | Update via event queue | Lower | Eventual |
//! | Hybrid | Sync for some, async for others | Medium | Configurable |
//!
//! # Example
//!
//! ```ignore
//! use helios_persistence::composite::sync::{SyncManager, SyncEvent};
//!
//! let manager = SyncManager::new(SyncConfig::default());
//!
//! // Sync a create event to secondaries
//! manager.sync(&SyncEvent::Create {
//!     resource_type: "Patient".to_string(),
//!     resource_id: "123".to_string(),
//!     content: patient_json,
//!     tenant_id: tenant.tenant_id().clone(),
//! }, &secondaries).await?;
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use helios_fhir::FhirVersion;
use parking_lot::RwLock;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio::time::sleep;
use tracing::{debug, error, warn};

use crate::core::ResourceStorage;
use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::{TenantContext, TenantId, TenantPermissions};
use crate::types::StoredResource;

use super::config::{RetryConfig, SyncConfig, SyncMode};

/// A synchronization event to propagate to secondary backends.
#[derive(Debug, Clone)]
pub enum SyncEvent {
    /// Resource was created.
    Create {
        /// Resource type.
        resource_type: String,
        /// Resource ID.
        resource_id: String,
        /// Resource content.
        content: Value,
        /// Tenant ID.
        tenant_id: TenantId,
        /// FHIR version.
        fhir_version: FhirVersion,
    },

    /// Resource was updated.
    Update {
        /// Resource type.
        resource_type: String,
        /// Resource ID.
        resource_id: String,
        /// New resource content.
        content: Value,
        /// Tenant ID.
        tenant_id: TenantId,
        /// New version.
        version: String,
        /// FHIR version.
        fhir_version: FhirVersion,
    },

    /// Resource was deleted.
    Delete {
        /// Resource type.
        resource_type: String,
        /// Resource ID.
        resource_id: String,
        /// Tenant ID.
        tenant_id: TenantId,
    },

    /// Bulk sync request.
    BulkSync {
        /// Resources to sync.
        resources: Vec<StoredResource>,
        /// Tenant ID.
        tenant_id: TenantId,
    },
}

impl SyncEvent {
    /// Returns the resource type for this event.
    pub fn resource_type(&self) -> &str {
        match self {
            SyncEvent::Create { resource_type, .. } => resource_type,
            SyncEvent::Update { resource_type, .. } => resource_type,
            SyncEvent::Delete { resource_type, .. } => resource_type,
            SyncEvent::BulkSync { .. } => "bulk",
        }
    }

    /// Returns the resource ID for this event (if applicable).
    pub fn resource_id(&self) -> Option<&str> {
        match self {
            SyncEvent::Create { resource_id, .. } => Some(resource_id),
            SyncEvent::Update { resource_id, .. } => Some(resource_id),
            SyncEvent::Delete { resource_id, .. } => Some(resource_id),
            SyncEvent::BulkSync { .. } => None,
        }
    }

    /// Returns the tenant ID for this event.
    pub fn tenant_id(&self) -> &TenantId {
        match self {
            SyncEvent::Create { tenant_id, .. } => tenant_id,
            SyncEvent::Update { tenant_id, .. } => tenant_id,
            SyncEvent::Delete { tenant_id, .. } => tenant_id,
            SyncEvent::BulkSync { tenant_id, .. } => tenant_id,
        }
    }
}

/// Status of a sync operation.
#[derive(Debug, Clone)]
pub struct SyncStatus {
    /// Backend ID.
    pub backend_id: String,

    /// Whether the sync succeeded.
    pub success: bool,

    /// Error message if failed.
    pub error: Option<String>,

    /// Retry count.
    pub retry_count: u32,

    /// Duration of the operation.
    pub duration: Duration,

    /// Ids this backend rejected after retries; empty on success.
    ///
    /// Only [`SyncManager::sync_creates`] populates this — a batch sync knows
    /// which of its many resources failed. A single-event [`SyncManager::sync`]
    /// concerns one resource already named by the event, and in asynchronous
    /// mode the event is merely enqueued, so both leave this empty.
    pub failed_resource_ids: Vec<String>,
}

/// Synchronization manager for secondary backends.
pub struct SyncManager {
    /// Configuration.
    config: SyncConfig,

    /// Event queue for async mode.
    event_sender: Option<mpsc::Sender<Queued>>,

    /// Sync status per backend.
    status: Arc<RwLock<HashMap<String, BackendSyncStatus>>>,
}

/// Status tracking for a backend.
#[derive(Debug, Clone, Default)]
pub struct BackendSyncStatus {
    /// Last successful sync timestamp.
    pub last_success: Option<std::time::Instant>,

    /// Current sync lag (events pending).
    pub pending_events: usize,

    /// Total events synced.
    pub total_synced: u64,

    /// Total errors.
    pub total_errors: u64,

    /// Whether sync is healthy.
    pub healthy: bool,
}

/// Event queued for async processing.
struct QueuedEvent {
    event: SyncEvent,
    backend_ids: Vec<String>,
    #[allow(dead_code)]
    created_at: std::time::Instant,
}

/// An item on the asynchronous sync queue.
enum Queued {
    /// A sync event bound for the named backends.
    Event(QueuedEvent),
    /// A barrier, resolved once every item queued ahead of it has been
    /// processed. See [`SyncManager::barrier`].
    Barrier(oneshot::Sender<()>),
}

/// The asynchronous sync queue cannot take or complete a request: the worker
/// was never started, has stopped, or was aborted mid-batch.
fn sync_queue_unavailable(message: impl Into<String>) -> StorageError {
    StorageError::Backend(BackendError::ConnectionFailed {
        backend_name: "sync".to_string(),
        message: message.into(),
    })
}

impl SyncManager {
    /// Creates a new sync manager.
    pub fn new(config: SyncConfig) -> Self {
        Self {
            config,
            event_sender: None,
            status: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Starts the async sync worker.
    pub fn start_async_worker(
        &mut self,
        backends: HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>,
    ) -> tokio::task::JoinHandle<()> {
        let (sender, receiver) = mpsc::channel::<Queued>(1000);
        self.event_sender = Some(sender);

        let config = self.config.clone();
        let status = self.status.clone();

        tokio::spawn(async move {
            Self::async_worker(receiver, backends, config, status).await;
        })
    }

    /// Async worker that processes queued events.
    async fn async_worker(
        mut receiver: mpsc::Receiver<Queued>,
        backends: HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>,
        config: SyncConfig,
        status: Arc<RwLock<HashMap<String, BackendSyncStatus>>>,
    ) {
        let mut batch = Vec::new();
        let batch_timeout = Duration::from_millis(100);

        loop {
            // Collect events into batches
            let deadline = tokio::time::Instant::now() + batch_timeout;

            loop {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() || batch.len() >= config.batch_size {
                    break;
                }

                match tokio::time::timeout(remaining, receiver.recv()).await {
                    Ok(Some(item)) => {
                        // A barrier means someone is waiting on everything
                        // ahead of it: process what is collected now rather
                        // than holding it for the rest of the batch window.
                        let flush = matches!(item, Queued::Barrier(_));
                        batch.push(item);
                        if flush {
                            break;
                        }
                    }
                    Ok(None) => return, // Channel closed
                    Err(_) => break,    // Timeout
                }
            }

            if batch.is_empty() {
                continue;
            }

            // Process batch
            let items: Vec<_> = std::mem::take(&mut batch);

            for item in items {
                let queued = match item {
                    Queued::Event(queued) => queued,
                    Queued::Barrier(done) => {
                        // Everything queued ahead of this point has been
                        // handed to its backends. The waiter may have given
                        // up; that is its business.
                        let _ = done.send(());
                        continue;
                    }
                };
                for backend_id in &queued.backend_ids {
                    if let Some(backend) = backends.get(backend_id) {
                        let result = Self::sync_event_to_backend(
                            &queued.event,
                            backend.as_ref(),
                            &config.retry,
                        )
                        .await;

                        // Update status
                        let mut status_map = status.write();
                        let backend_status = status_map.entry(backend_id.clone()).or_default();

                        match result {
                            Ok(_) => {
                                backend_status.last_success = Some(std::time::Instant::now());
                                backend_status.total_synced += 1;
                                backend_status.healthy = true;
                            }
                            Err(e) => {
                                backend_status.total_errors += 1;
                                error!(
                                    backend = %backend_id,
                                    error = %e,
                                    "Async sync failed"
                                );
                            }
                        }

                        if backend_status.pending_events > 0 {
                            backend_status.pending_events -= 1;
                        }
                    }
                }
            }
        }
    }

    /// Synchronizes an event to secondary backends.
    pub async fn sync(
        &self,
        event: &SyncEvent,
        backends: &HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>,
    ) -> StorageResult<Vec<SyncStatus>> {
        match self.config.mode {
            SyncMode::Synchronous => self.sync_synchronous(event, backends).await,
            SyncMode::Asynchronous => self.sync_asynchronous(event, backends).await,
            SyncMode::Hybrid { sync_for_search } => {
                // In hybrid mode, sync search-related events synchronously
                let is_search_related = matches!(
                    event,
                    SyncEvent::Create { .. } | SyncEvent::Update { .. } | SyncEvent::Delete { .. }
                );

                if sync_for_search && is_search_related {
                    self.sync_synchronous(event, backends).await
                } else {
                    self.sync_asynchronous(event, backends).await
                }
            }
        }
    }

    /// Synchronizes a batch of creates — one tenant, one resource type — to
    /// the secondary backends as a batch.
    ///
    /// In the synchronous modes every backend receives the whole batch
    /// through [`ResourceStorage::create_many`], so a secondary with a native
    /// batch write (Elasticsearch `_bulk`) pays one round trip and, under
    /// `refresh=wait_for`, one refresh wait rather than one per resource.
    /// Items the batch rejects are retried one at a time through the same
    /// retrying path a single [`sync`](Self::sync) takes, so a partial batch
    /// failure degrades to per-resource sync rather than to lost writes.
    /// Asynchronous mode queues one event per resource, exactly as `sync`
    /// would for each.
    pub async fn sync_creates(
        &self,
        tenant_id: &TenantId,
        resource_type: &str,
        fhir_version: FhirVersion,
        resources: Vec<(String, Value)>,
        backends: &HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>,
    ) -> StorageResult<Vec<SyncStatus>> {
        let create_event = |(id, content): &(String, Value)| SyncEvent::Create {
            resource_type: resource_type.to_string(),
            resource_id: id.clone(),
            content: content.clone(),
            tenant_id: tenant_id.clone(),
            fhir_version,
        };
        let synchronous = match self.config.mode {
            SyncMode::Synchronous => true,
            SyncMode::Asynchronous => false,
            SyncMode::Hybrid { sync_for_search } => sync_for_search,
        };
        if !synchronous {
            let mut statuses = Vec::new();
            for resource in &resources {
                statuses.extend(
                    self.sync_asynchronous(&create_event(resource), backends)
                        .await?,
                );
            }
            return Ok(statuses);
        }
        if resources.is_empty() {
            return Ok(Vec::new());
        }

        use tokio::task::JoinSet;

        let resources = Arc::new(resources);
        let tenant = TenantContext::new(tenant_id.clone(), TenantPermissions::full_access());
        let resource_type = resource_type.to_string();
        let mut tasks: JoinSet<(SyncStatus, usize, usize)> = JoinSet::new();

        for (backend_id, backend) in backends {
            let resources = resources.clone();
            let tenant = tenant.clone();
            let resource_type = resource_type.clone();
            let backend = backend.clone();
            let backend_id = backend_id.clone();
            let retry_config = self.config.retry.clone();

            tasks.spawn(async move {
                let start = std::time::Instant::now();
                let contents = resources
                    .iter()
                    .map(|(_, content)| content.clone())
                    .collect();
                let results = backend
                    .create_many(&tenant, &resource_type, contents, fhir_version)
                    .await;

                let mut synced = 0;
                let mut errors = 0;
                let mut last_error = None;
                let mut failed_resource_ids = Vec::new();
                for ((resource_id, content), result) in resources.iter().zip(results) {
                    let Err(batch_error) = result else {
                        synced += 1;
                        continue;
                    };
                    warn!(
                        backend_id = %backend_id,
                        resource_type = %resource_type,
                        resource_id = %resource_id,
                        error = %batch_error,
                        "Batch sync rejected a resource; retrying it individually"
                    );
                    let event = SyncEvent::Create {
                        resource_type: resource_type.clone(),
                        resource_id: resource_id.clone(),
                        content: content.clone(),
                        tenant_id: tenant.tenant_id().clone(),
                        fhir_version,
                    };
                    match Self::sync_event_to_backend(&event, backend.as_ref(), &retry_config).await
                    {
                        Ok(()) => synced += 1,
                        Err(e) => {
                            errors += 1;
                            last_error = Some(e.to_string());
                            failed_resource_ids.push(resource_id.clone());
                        }
                    }
                }

                (
                    SyncStatus {
                        backend_id,
                        success: errors == 0,
                        error: last_error,
                        retry_count: if errors == 0 {
                            0
                        } else {
                            retry_config.max_retries
                        },
                        duration: start.elapsed(),
                        failed_resource_ids,
                    },
                    synced,
                    errors,
                )
            });
        }

        let mut results = Vec::new();
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok((status, synced, errors)) => {
                    let mut status_map = self.status.write();
                    let backend_status = status_map.entry(status.backend_id.clone()).or_default();
                    if synced > 0 {
                        backend_status.last_success = Some(std::time::Instant::now());
                        backend_status.total_synced += synced as u64;
                    }
                    backend_status.total_errors += errors as u64;
                    if status.success {
                        backend_status.healthy = true;
                    }
                    drop(status_map);
                    results.push(status);
                }
                Err(e) => {
                    warn!(error = %e, "Batch sync task failed");
                }
            }
        }

        Ok(results)
    }

    /// Synchronous sync - waits for all backends.
    async fn sync_synchronous(
        &self,
        event: &SyncEvent,
        backends: &HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>,
    ) -> StorageResult<Vec<SyncStatus>> {
        use tokio::task::JoinSet;

        let mut tasks: JoinSet<SyncStatus> = JoinSet::new();
        let event = event.clone();

        for (backend_id, backend) in backends {
            let event = event.clone();
            let backend = backend.clone();
            let backend_id = backend_id.clone();
            let retry_config = self.config.retry.clone();

            tasks.spawn(async move {
                let start = std::time::Instant::now();

                match Self::sync_event_to_backend(&event, backend.as_ref(), &retry_config).await {
                    Ok(_) => SyncStatus {
                        backend_id,
                        success: true,
                        error: None,
                        retry_count: 0,
                        duration: start.elapsed(),
                        failed_resource_ids: Vec::new(),
                    },
                    Err(e) => SyncStatus {
                        backend_id,
                        success: false,
                        error: Some(e.to_string()),
                        retry_count: retry_config.max_retries,
                        duration: start.elapsed(),
                        failed_resource_ids: Vec::new(),
                    },
                }
            });
        }

        let mut results = Vec::new();
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(status) => {
                    // Update internal status
                    let mut status_map = self.status.write();
                    let backend_status = status_map.entry(status.backend_id.clone()).or_default();

                    if status.success {
                        backend_status.last_success = Some(std::time::Instant::now());
                        backend_status.total_synced += 1;
                        backend_status.healthy = true;
                    } else {
                        backend_status.total_errors += 1;
                    }

                    results.push(status);
                }
                Err(e) => {
                    warn!(error = %e, "Sync task failed");
                }
            }
        }

        Ok(results)
    }

    /// Asynchronous sync - queues events for background processing.
    async fn sync_asynchronous(
        &self,
        event: &SyncEvent,
        backends: &HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>,
    ) -> StorageResult<Vec<SyncStatus>> {
        if let Some(ref sender) = self.event_sender {
            let backend_ids: Vec<_> = backends.keys().cloned().collect();

            // Update pending counts
            {
                let mut status_map = self.status.write();
                for id in &backend_ids {
                    let status = status_map.entry(id.clone()).or_default();
                    status.pending_events += 1;
                }
            }

            sender
                .send(Queued::Event(QueuedEvent {
                    event: event.clone(),
                    backend_ids: backend_ids.clone(),
                    created_at: std::time::Instant::now(),
                }))
                .await
                .map_err(|e| {
                    sync_queue_unavailable(format!("Failed to queue sync event: {}", e))
                })?;

            // Return pending status
            Ok(backend_ids
                .into_iter()
                .map(|id| SyncStatus {
                    backend_id: id,
                    success: true,
                    error: None,
                    retry_count: 0,
                    duration: Duration::ZERO,
                    // Queued, not rejected: nothing is known to have failed yet.
                    failed_resource_ids: Vec::new(),
                })
                .collect())
        } else {
            Err(sync_queue_unavailable(
                "Async sync mode is configured but the async worker was never started. \
                 Call start_async_worker() before processing events.",
            ))
        }
    }

    /// Syncs a single event to a backend with retries.
    async fn sync_event_to_backend(
        event: &SyncEvent,
        backend: &dyn ResourceStorage,
        retry_config: &RetryConfig,
    ) -> StorageResult<()> {
        let mut delay = retry_config.initial_delay;
        let mut attempts = 0;

        loop {
            attempts += 1;

            let result = match event {
                SyncEvent::Create {
                    resource_type,
                    content,
                    tenant_id,
                    fhir_version,
                    ..
                } => {
                    let tenant =
                        TenantContext::new(tenant_id.clone(), TenantPermissions::full_access());
                    backend
                        .create(&tenant, resource_type, content.clone(), *fhir_version)
                        .await
                        .map(|_| ())
                }
                SyncEvent::Update {
                    resource_type,
                    resource_id,
                    content,
                    tenant_id,
                    fhir_version,
                    ..
                } => {
                    let tenant =
                        TenantContext::new(tenant_id.clone(), TenantPermissions::full_access());

                    // For secondary backends, we do a create_or_update
                    // since we don't track versions in secondaries
                    backend
                        .create_or_update(
                            &tenant,
                            resource_type,
                            resource_id,
                            content.clone(),
                            *fhir_version,
                        )
                        .await
                        .map(|_| ())
                }
                SyncEvent::Delete {
                    resource_type,
                    resource_id,
                    tenant_id,
                } => {
                    let tenant =
                        TenantContext::new(tenant_id.clone(), TenantPermissions::full_access());
                    backend.delete(&tenant, resource_type, resource_id).await
                }
                SyncEvent::BulkSync {
                    resources,
                    tenant_id,
                } => {
                    let tenant =
                        TenantContext::new(tenant_id.clone(), TenantPermissions::full_access());

                    for resource in resources {
                        backend
                            .create_or_update(
                                &tenant,
                                resource.resource_type(),
                                resource.id(),
                                resource.content().clone(),
                                resource.fhir_version(),
                            )
                            .await?;
                    }
                    Ok(())
                }
            };

            match result {
                Ok(()) => {
                    if attempts > 1 {
                        debug!(attempts = attempts, "Sync succeeded after retries");
                    }
                    return Ok(());
                }
                Err(e) => {
                    if attempts > retry_config.max_retries {
                        return Err(e);
                    }

                    warn!(
                        attempt = attempts,
                        max_retries = retry_config.max_retries,
                        delay_ms = delay.as_millis(),
                        error = %e,
                        "Sync attempt failed, retrying"
                    );

                    sleep(delay).await;
                    delay = std::cmp::min(
                        Duration::from_secs_f64(
                            delay.as_secs_f64() * retry_config.backoff_multiplier,
                        ),
                        retry_config.max_delay,
                    );
                }
            }
        }
    }

    /// Returns the sync status for a backend.
    pub fn backend_status(&self, backend_id: &str) -> Option<BackendSyncStatus> {
        self.status.read().get(backend_id).cloned()
    }

    /// Returns all backend statuses.
    pub fn all_statuses(&self) -> HashMap<String, BackendSyncStatus> {
        self.status.read().clone()
    }

    /// Checks if all backends are healthy (no excessive lag).
    pub fn is_healthy(&self) -> bool {
        let _max_lag = self.config.max_read_lag_ms;
        let status = self.status.read();

        for backend_status in status.values() {
            // Consider unhealthy if pending events exceed threshold
            // (rough approximation of lag)
            if backend_status.pending_events > self.config.batch_size * 10 {
                return false;
            }
        }

        true
    }

    /// Waits until every event queued for asynchronous sync before this
    /// call has been handed to its backends.
    ///
    /// Unlike [`wait_for_sync`](Self::wait_for_sync), which waits for the
    /// queue to be *empty* and so never returns under a steady write stream,
    /// this rides the queue as an item of its own: it resolves as soon as the
    /// worker reaches it, whatever arrives behind it. A search issued after
    /// it returns sees every write acknowledged before it was called — the
    /// read-your-writes a transaction's conditional-reference resolution
    /// needs (#1047).
    ///
    /// Without an asynchronous worker (synchronous mode, or no worker
    /// started) nothing is ever queued, so there is nothing to wait for.
    pub async fn barrier(&self) -> StorageResult<()> {
        let Some(sender) = self.event_sender.as_ref() else {
            return Ok(());
        };
        let (done, reached) = oneshot::channel();
        sender
            .send(Queued::Barrier(done))
            .await
            .map_err(|e| sync_queue_unavailable(format!("Failed to queue sync barrier: {}", e)))?;
        // The worker drops the barrier unsent only if it stops (or is
        // aborted) while holding it.
        reached.await.map_err(|_| {
            sync_queue_unavailable("Async sync worker stopped before reaching the barrier")
        })
    }

    /// Waits for sync lag to be below threshold.
    pub async fn wait_for_sync(&self, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;

        while tokio::time::Instant::now() < deadline {
            if self.is_healthy() {
                let status = self.status.read();
                let all_synced = status.values().all(|s| s.pending_events == 0);
                if all_synced {
                    return true;
                }
            }
            sleep(Duration::from_millis(10)).await;
        }

        false
    }
}

/// Sync reconciliation for detecting and fixing inconsistencies.
pub struct SyncReconciler {
    /// Maximum resources to check per batch.
    #[allow(dead_code)]
    batch_size: usize,
}

impl SyncReconciler {
    /// Creates a new reconciler.
    pub fn new() -> Self {
        Self { batch_size: 100 }
    }

    /// Reconciles a secondary backend with the primary.
    pub async fn reconcile(
        &self,
        tenant: &TenantContext,
        primary: &dyn ResourceStorage,
        secondary: &dyn ResourceStorage,
        resource_type: &str,
    ) -> StorageResult<ReconciliationResult> {
        let mut result = ReconciliationResult::default();

        // Get count from both
        let primary_count = primary.count(tenant, Some(resource_type)).await?;
        result.primary_count = primary_count;

        let secondary_count = secondary.count(tenant, Some(resource_type)).await?;
        result.secondary_count = secondary_count;

        // TODO: Implement full reconciliation by:
        // 1. Iterating through primary resources
        // 2. Checking if they exist in secondary
        // 3. Checking if content matches
        // 4. Syncing any differences

        // For now, just report counts
        if primary_count != secondary_count {
            result.differences = (primary_count as i64 - secondary_count as i64).unsigned_abs();
        }

        Ok(result)
    }
}

impl Default for SyncReconciler {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of a reconciliation operation.
#[derive(Debug, Default)]
pub struct ReconciliationResult {
    /// Resource count in primary.
    pub primary_count: u64,

    /// Resource count in secondary.
    pub secondary_count: u64,

    /// Number of differences found.
    pub differences: u64,

    /// Resources missing from secondary.
    pub missing_in_secondary: Vec<String>,

    /// Resources extra in secondary (should be deleted).
    pub extra_in_secondary: Vec<String>,

    /// Resources with content mismatch.
    pub content_mismatches: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use helios_fhir::FhirVersion;

    #[test]
    fn test_sync_event_accessors() {
        let event = SyncEvent::Create {
            resource_type: "Patient".to_string(),
            resource_id: "123".to_string(),
            content: serde_json::json!({}),
            tenant_id: TenantId::new("test"),
            fhir_version: FhirVersion::default(),
        };

        assert_eq!(event.resource_type(), "Patient");
        assert_eq!(event.resource_id(), Some("123"));
        assert_eq!(event.tenant_id().as_str(), "test");
    }

    #[test]
    fn test_sync_status_default() {
        let status = BackendSyncStatus::default();
        assert!(status.last_success.is_none());
        assert_eq!(status.pending_events, 0);
        assert_eq!(status.total_synced, 0);
        assert!(!status.healthy);
    }

    #[test]
    fn test_reconciliation_result() {
        let result = ReconciliationResult {
            primary_count: 100,
            secondary_count: 95,
            differences: 5,
            ..Default::default()
        };

        assert_eq!(result.differences, 5);
    }

    #[test]
    fn test_sync_manager_creation() {
        let config = SyncConfig::default();
        let manager = SyncManager::new(config);
        assert!(manager.is_healthy());
    }

    // ------------------------------------------------------------------
    // `barrier()` (#1047)
    //
    // Exercised here, in-process, with a backend whose writes block until
    // the test releases them, so what the worker holds when `barrier()` is
    // called is a fact of the test rather than a race against scheduling.
    // ------------------------------------------------------------------

    use std::sync::Mutex;
    use std::time::Instant;

    use async_trait::async_trait;
    use tokio::sync::Semaphore;

    /// A secondary that announces each write as it starts, then holds it
    /// until the test grants a permit, and records the writes it completed
    /// in order.
    struct GatedBackend {
        completed: Mutex<Vec<String>>,
        gate: Semaphore,
        started: mpsc::UnboundedSender<String>,
    }

    impl GatedBackend {
        fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<String>) {
            let (started, starts) = mpsc::unbounded_channel();
            let backend = Arc::new(Self {
                completed: Mutex::new(Vec::new()),
                gate: Semaphore::new(0),
                started,
            });
            (backend, starts)
        }

        /// Lets `n` more writes through.
        fn release(&self, n: usize) {
            self.gate.add_permits(n);
        }

        fn completed(&self) -> Vec<String> {
            self.completed.lock().unwrap().clone()
        }

        async fn write(&self, op: String) {
            let _ = self.started.send(op.clone());
            self.gate
                .acquire()
                .await
                .expect("gate is never closed")
                .forget();
            self.completed.lock().unwrap().push(op);
        }
    }

    #[async_trait]
    impl ResourceStorage for GatedBackend {
        fn backend_name(&self) -> &'static str {
            "gated"
        }

        async fn create(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            resource: Value,
            fhir_version: FhirVersion,
        ) -> StorageResult<StoredResource> {
            let id = resource["id"].as_str().unwrap_or("").to_string();
            self.write(format!("create {id}")).await;
            Ok(StoredResource::new(
                resource_type,
                id,
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
            self.write(format!("update {id}")).await;
            Ok((
                StoredResource::new(
                    resource_type,
                    id,
                    tenant.tenant_id().clone(),
                    resource,
                    fhir_version,
                ),
                false,
            ))
        }

        async fn read(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<Option<StoredResource>> {
            unimplemented!("the sync worker never reads")
        }

        async fn update(
            &self,
            _tenant: &TenantContext,
            _current: &StoredResource,
            _resource: Value,
        ) -> StorageResult<StoredResource> {
            unimplemented!("the sync worker updates through create_or_update")
        }

        async fn delete(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            id: &str,
        ) -> StorageResult<()> {
            self.write(format!("delete {id}")).await;
            Ok(())
        }

        async fn count(
            &self,
            _tenant: &TenantContext,
            _resource_type: Option<&str>,
        ) -> StorageResult<u64> {
            unimplemented!("the sync worker never counts")
        }
    }

    fn async_config() -> SyncConfig {
        SyncConfig {
            mode: SyncMode::Asynchronous,
            ..SyncConfig::default()
        }
    }

    fn backends(
        backend: &Arc<GatedBackend>,
    ) -> HashMap<String, Arc<dyn ResourceStorage + Send + Sync>> {
        let mut map: HashMap<String, Arc<dyn ResourceStorage + Send + Sync>> = HashMap::new();
        map.insert("search".to_string(), backend.clone());
        map
    }

    fn create_event(id: &str) -> SyncEvent {
        SyncEvent::Create {
            resource_type: "Patient".to_string(),
            resource_id: id.to_string(),
            content: serde_json::json!({"resourceType": "Patient", "id": id}),
            tenant_id: TenantId::new("t"),
            fhir_version: FhirVersion::default(),
        }
    }

    fn update_event(id: &str) -> SyncEvent {
        SyncEvent::Update {
            resource_type: "Patient".to_string(),
            resource_id: id.to_string(),
            content: serde_json::json!({"resourceType": "Patient", "id": id}),
            tenant_id: TenantId::new("t"),
            version: "2".to_string(),
            fhir_version: FhirVersion::default(),
        }
    }

    fn delete_event(id: &str) -> SyncEvent {
        SyncEvent::Delete {
            resource_type: "Patient".to_string(),
            resource_id: id.to_string(),
            tenant_id: TenantId::new("t"),
        }
    }

    /// The barrier resolves only once every event queued ahead of it has
    /// been handed to the backend, and not for anything queued behind it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn barrier_resolves_after_the_events_queued_ahead_of_it() {
        let (backend, mut starts) = GatedBackend::new();
        let backends = backends(&backend);
        let mut manager = Arc::new(SyncManager::new(async_config()));
        let _worker = Arc::get_mut(&mut manager)
            .unwrap()
            .start_async_worker(backends.clone());

        manager.sync(&create_event("a"), &backends).await.unwrap();
        manager.sync(&create_event("b"), &backends).await.unwrap();
        assert!(
            backend.completed().is_empty(),
            "async sync returns before the backend is written"
        );

        let barrier = {
            let manager = manager.clone();
            tokio::spawn(async move { manager.barrier().await })
        };

        // The worker is now holding `a`: it reached the backend, which is
        // waiting for the test. The barrier must not have resolved.
        assert_eq!(starts.recv().await.as_deref(), Some("create a"));
        assert!(
            !barrier.is_finished(),
            "barrier resolved before `a` was written"
        );
        backend.release(1);
        assert_eq!(starts.recv().await.as_deref(), Some("create b"));
        assert!(
            !barrier.is_finished(),
            "barrier resolved before `b` was written"
        );
        backend.release(1);

        barrier.await.unwrap().expect("barrier");
        assert_eq!(backend.completed(), vec!["create a", "create b"]);

        // Queued behind the barrier: the backend is holding it, and the
        // barrier that already resolved did not wait for it.
        manager.sync(&create_event("c"), &backends).await.unwrap();
        assert_eq!(starts.recv().await.as_deref(), Some("create c"));
        assert_eq!(backend.completed().len(), 2);
        backend.release(1);
        manager.barrier().await.expect("second barrier");
        assert_eq!(
            backend.completed(),
            vec!["create a", "create b", "create c"]
        );
    }

    /// Every event kind is ordered behind the barrier the same way: a
    /// create, then the update and delete of the same resource, land in
    /// the order they were queued before the barrier resolves.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn barrier_orders_every_event_kind() {
        let (backend, _starts) = GatedBackend::new();
        backend.release(3);
        let backends = backends(&backend);
        let mut manager = SyncManager::new(async_config());
        let _worker = manager.start_async_worker(backends.clone());

        manager.sync(&create_event("a"), &backends).await.unwrap();
        manager.sync(&update_event("a"), &backends).await.unwrap();
        manager.sync(&delete_event("a"), &backends).await.unwrap();
        manager.barrier().await.expect("barrier");

        assert_eq!(
            backend.completed(),
            vec!["create a", "update a", "delete a"]
        );
    }

    /// The worker holds events for up to a 100ms batching window; a barrier
    /// flushes what is collected immediately instead of waiting it out.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn barrier_flushes_the_batch_window_early() {
        let (backend, _starts) = GatedBackend::new();
        backend.release(1);
        let backends = backends(&backend);
        let mut manager = SyncManager::new(async_config());
        let _worker = manager.start_async_worker(backends.clone());

        let started = Instant::now();
        manager.sync(&create_event("a"), &backends).await.unwrap();
        manager.barrier().await.expect("barrier");
        let elapsed = started.elapsed();

        assert_eq!(backend.completed(), vec!["create a"]);
        assert!(
            elapsed < Duration::from_millis(100),
            "barrier waited out the batch window: {elapsed:?}"
        );
    }

    /// Without an asynchronous worker nothing is ever queued: synchronous
    /// mode, or an asynchronous manager whose worker was never started.
    #[tokio::test]
    async fn barrier_without_a_worker_is_a_no_op() {
        let synchronous = SyncManager::new(SyncConfig {
            mode: SyncMode::Synchronous,
            ..SyncConfig::default()
        });
        synchronous
            .barrier()
            .await
            .expect("synchronous: nothing queued");

        let never_started = SyncManager::new(async_config());
        never_started
            .barrier()
            .await
            .expect("no worker: nothing queued");
    }

    fn assert_sync_unavailable(err: &StorageError, expected_message: &str) {
        match err {
            StorageError::Backend(BackendError::ConnectionFailed {
                backend_name,
                message,
            }) => {
                assert_eq!(backend_name, "sync");
                assert!(message.contains(expected_message), "{message}");
            }
            other => panic!("expected the sync queue to report itself unavailable: {other}"),
        }
    }

    /// Asynchronous mode with no worker started cannot queue anything: the
    /// write is refused up front rather than silently never synced.
    #[tokio::test]
    async fn async_sync_without_a_started_worker_is_refused() {
        let (backend, _starts) = GatedBackend::new();
        let backends = backends(&backend);
        let manager = SyncManager::new(async_config());

        let err = manager
            .sync(&create_event("a"), &backends)
            .await
            .expect_err("nothing can take the event");
        assert_sync_unavailable(&err, "async worker was never started");
    }

    /// Once the worker is gone the queue is closed, and a write that would
    /// have been queued is refused rather than dropped.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn async_sync_after_the_worker_stopped_is_refused() {
        let (backend, _starts) = GatedBackend::new();
        let backends = backends(&backend);
        let mut manager = SyncManager::new(async_config());
        let worker = manager.start_async_worker(backends.clone());
        worker.abort();
        let _ = worker.await;

        let err = manager
            .sync(&create_event("a"), &backends)
            .await
            .expect_err("the receiver is gone; the event cannot be queued");
        assert_sync_unavailable(&err, "Failed to queue sync event");
    }

    /// A worker that is already gone cannot even take the barrier; the
    /// caller learns that rather than waiting forever.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn barrier_reports_a_stopped_worker() {
        let (backend, _starts) = GatedBackend::new();
        let backends = backends(&backend);
        let mut manager = SyncManager::new(async_config());
        let worker = manager.start_async_worker(backends);
        worker.abort();
        let _ = worker.await;

        let err = manager
            .barrier()
            .await
            .expect_err("the receiver is gone; the barrier cannot be queued");
        assert_sync_unavailable(&err, "Failed to queue sync barrier");
    }

    /// A worker that dies while holding the barrier — aborted mid-batch,
    /// with the barrier's event still being written — drops it unsent, and
    /// the waiter is told so instead of hanging.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn barrier_reports_a_worker_that_died_holding_it() {
        let (backend, mut starts) = GatedBackend::new();
        let backends = backends(&backend);
        let mut manager = Arc::new(SyncManager::new(async_config()));
        let worker = Arc::get_mut(&mut manager)
            .unwrap()
            .start_async_worker(backends.clone());

        manager.sync(&create_event("a"), &backends).await.unwrap();
        let barrier = {
            let manager = manager.clone();
            tokio::spawn(async move { manager.barrier().await })
        };

        // The worker took the barrier along with `a` and is blocked inside
        // the backend on `a`; kill it there.
        assert_eq!(starts.recv().await.as_deref(), Some("create a"));
        worker.abort();
        let _ = worker.await;

        let err = barrier
            .await
            .unwrap()
            .expect_err("the barrier was dropped unsent");
        assert_sync_unavailable(&err, "stopped before reaching the barrier");
        assert!(backend.completed().is_empty(), "nothing was written");
    }
}
