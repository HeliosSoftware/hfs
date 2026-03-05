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
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{debug, error, warn};

use crate::core::ResourceStorage;
use crate::error::{ResourceError, StorageError, StorageResult};
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
        /// Resource version.
        version: String,
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

    /// Returns the resource version for this event (if applicable).
    pub fn version(&self) -> Option<&str> {
        match self {
            SyncEvent::Create { version, .. } => Some(version),
            SyncEvent::Update { version, .. } => Some(version),
            SyncEvent::Delete { .. } | SyncEvent::BulkSync { .. } => None,
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
}

/// Synchronization manager for secondary backends.
pub struct SyncManager {
    /// Configuration.
    config: SyncConfig,

    /// Event queue for async mode.
    event_sender: Option<mpsc::Sender<QueuedEvent>>,

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
        let (sender, receiver) = mpsc::channel::<QueuedEvent>(1000);
        self.event_sender = Some(sender);

        let config = self.config.clone();
        let status = self.status.clone();

        tokio::spawn(async move {
            Self::async_worker(receiver, backends, config, status).await;
        })
    }

    /// Async worker that processes queued events.
    async fn async_worker(
        mut receiver: mpsc::Receiver<QueuedEvent>,
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
                    Ok(Some(event)) => batch.push(event),
                    Ok(None) => return, // Channel closed
                    Err(_) => break,    // Timeout
                }
            }

            if batch.is_empty() {
                continue;
            }

            // Process batch
            let events: Vec<_> = std::mem::take(&mut batch);

            for queued in events {
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
                    },
                    Err(e) => SyncStatus {
                        backend_id,
                        success: false,
                        error: Some(e.to_string()),
                        retry_count: retry_config.max_retries,
                        duration: start.elapsed(),
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
                .send(QueuedEvent {
                    event: event.clone(),
                    backend_ids: backend_ids.clone(),
                    created_at: std::time::Instant::now(),
                })
                .await
                .map_err(|e| {
                    StorageError::Backend(crate::error::BackendError::ConnectionFailed {
                        backend_name: "sync".to_string(),
                        message: format!("Failed to queue sync event: {}", e),
                    })
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
                })
                .collect())
        } else {
            // No async worker, fall back to sync
            warn!("Async sync requested but no worker started, falling back to sync");
            self.sync_synchronous(event, backends).await
        }
    }

    /// Syncs a single event to a backend with retries.
    async fn sync_event_to_backend(
        event: &SyncEvent,
        backend: &dyn ResourceStorage,
        retry_config: &RetryConfig,
    ) -> StorageResult<()> {
        fn inject_version(mut content: Value, version: &str) -> Value {
            let Some(obj) = content.as_object_mut() else {
                return content;
            };

            let meta = obj
                .entry("meta")
                .or_insert_with(|| Value::Object(serde_json::Map::new()));

            if !meta.is_object() {
                *meta = Value::Object(serde_json::Map::new());
            }

            if let Some(meta_obj) = meta.as_object_mut() {
                meta_obj.insert("versionId".to_string(), Value::String(version.to_string()));
            }

            content
        }

        let mut delay = retry_config.initial_delay;
        let mut attempts = 0;

        loop {
            attempts += 1;

            let result = match event {
                SyncEvent::Create {
                    resource_type,
                    resource_id,
                    version,
                    content,
                    tenant_id,
                    fhir_version,
                    ..
                } => {
                    let tenant =
                        TenantContext::new(tenant_id.clone(), TenantPermissions::full_access());
                    let content = inject_version(content.clone(), version);
                    backend
                        .create_or_update(
                            &tenant,
                            resource_type,
                            resource_id,
                            content,
                            *fhir_version,
                        )
                        .await
                        .map(|_| ())
                }
                SyncEvent::Update {
                    resource_type,
                    resource_id,
                    version,
                    content,
                    tenant_id,
                    fhir_version,
                    ..
                } => {
                    let tenant =
                        TenantContext::new(tenant_id.clone(), TenantPermissions::full_access());
                    let content = inject_version(content.clone(), version);
                    backend
                        .create_or_update(
                            &tenant,
                            resource_type,
                            resource_id,
                            content,
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
                    match backend.delete(&tenant, resource_type, resource_id).await {
                        Ok(()) => Ok(()),
                        Err(StorageError::Resource(ResourceError::NotFound { .. }))
                        | Err(StorageError::Resource(ResourceError::Gone { .. })) => Ok(()),
                        Err(err) => Err(err),
                    }
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

        for (_, backend_status) in status.iter() {
            // Consider unhealthy if pending events exceed threshold
            // (rough approximation of lag)
            if backend_status.pending_events > self.config.batch_size * 10 {
                return false;
            }
        }

        true
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
    use std::sync::{Arc, Mutex};

    use super::*;
    use async_trait::async_trait;
    use chrono::Utc;
    use helios_fhir::FhirVersion;
    use serde_json::json;

    use crate::core::ResourceStorage;
    use crate::error::ResourceError;
    use crate::types::StoredResource;

    #[derive(Default)]
    struct MockSyncBackend {
        captured_version: Mutex<Option<String>>,
        delete_calls: Mutex<u64>,
        missing_on_delete: bool,
    }

    impl MockSyncBackend {
        fn missing_delete() -> Self {
            Self {
                missing_on_delete: true,
                ..Default::default()
            }
        }
    }

    #[async_trait]
    impl ResourceStorage for MockSyncBackend {
        fn backend_name(&self) -> &'static str {
            "mock-sync"
        }

        async fn create(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<StoredResource> {
            Err(StorageError::Backend(
                crate::error::BackendError::UnsupportedCapability {
                    backend_name: "mock-sync".to_string(),
                    capability: "create".to_string(),
                },
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
            let version = resource
                .get("meta")
                .and_then(|m| m.get("versionId"))
                .and_then(|v| v.as_str())
                .unwrap_or("1")
                .to_string();
            *self
                .captured_version
                .lock()
                .expect("version mutex poisoned") = Some(version.clone());

            Ok((
                StoredResource::from_storage(
                    resource_type,
                    id,
                    version,
                    tenant.tenant_id().clone(),
                    resource,
                    Utc::now(),
                    Utc::now(),
                    None,
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
            Ok(None)
        }

        async fn update(
            &self,
            _tenant: &TenantContext,
            _current: &StoredResource,
            _resource: Value,
        ) -> StorageResult<StoredResource> {
            Err(StorageError::Backend(
                crate::error::BackendError::UnsupportedCapability {
                    backend_name: "mock-sync".to_string(),
                    capability: "update".to_string(),
                },
            ))
        }

        async fn delete(
            &self,
            _tenant: &TenantContext,
            resource_type: &str,
            id: &str,
        ) -> StorageResult<()> {
            *self
                .delete_calls
                .lock()
                .expect("delete calls mutex poisoned") += 1;
            if self.missing_on_delete {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
            }
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

    #[test]
    fn test_sync_event_accessors() {
        let event = SyncEvent::Create {
            resource_type: "Patient".to_string(),
            resource_id: "123".to_string(),
            version: "1".to_string(),
            content: serde_json::json!({}),
            tenant_id: TenantId::new("test"),
            fhir_version: FhirVersion::default(),
        };

        assert_eq!(event.resource_type(), "Patient");
        assert_eq!(event.resource_id(), Some("123"));
        assert_eq!(event.version(), Some("1"));
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

    #[tokio::test]
    async fn test_sync_create_injects_version_into_resource() {
        let backend = MockSyncBackend::default();
        let event = SyncEvent::Create {
            resource_type: "Patient".to_string(),
            resource_id: "p-1".to_string(),
            version: "7".to_string(),
            content: json!({
                "resourceType": "Patient",
                "id": "p-1"
            }),
            tenant_id: TenantId::new("tenant-a"),
            fhir_version: FhirVersion::default(),
        };

        let retry = RetryConfig::default();
        let result = SyncManager::sync_event_to_backend(&event, &backend, &retry).await;
        assert!(result.is_ok());
        assert_eq!(
            backend
                .captured_version
                .lock()
                .expect("version mutex poisoned")
                .as_deref(),
            Some("7")
        );
    }

    #[tokio::test]
    async fn test_sync_delete_missing_resource_is_idempotent() {
        let backend = Arc::new(MockSyncBackend::missing_delete());
        let event = SyncEvent::Delete {
            resource_type: "Patient".to_string(),
            resource_id: "p-404".to_string(),
            tenant_id: TenantId::new("tenant-a"),
        };

        let mut backends: HashMap<String, Arc<dyn ResourceStorage + Send + Sync>> = HashMap::new();
        backends.insert("es".to_string(), backend);

        let manager = SyncManager::new(SyncConfig {
            mode: SyncMode::Synchronous,
            ..Default::default()
        });

        let statuses = manager
            .sync(&event, &backends)
            .await
            .expect("sync should succeed");
        assert_eq!(statuses.len(), 1);
        assert!(statuses[0].success);
        assert!(statuses[0].error.is_none());
    }
}
