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

use chrono::Utc;
use helios_fhir::FhirVersion;
use parking_lot::RwLock;
use serde_json::Value;
use tokio::sync::{Notify, mpsc};
use tokio::time::sleep;
use tracing::{debug, error, warn};

use crate::core::ResourceStorage;
use crate::core::composite_sync_outbox::{
    ClaimedSyncEvent, CompositeSyncOutbox, NewSyncOutboxEntry, SyncLease, SyncLeaseError,
    SyncOperation, WorkerId,
};
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
}

/// Synchronization manager for secondary backends.
pub struct SyncManager {
    /// Configuration.
    config: SyncConfig,

    /// Event queue for async mode.
    event_sender: Option<mpsc::Sender<QueuedEvent>>,

    /// Sync status per backend.
    status: Arc<RwLock<HashMap<String, BackendSyncStatus>>>,

    /// Durable outbox for `Create`/`Update`/`Delete` events (E1), when the
    /// primary backend can back one (a Postgres primary). `None` keeps
    /// those events on the pre-existing in-memory channel above, exactly as
    /// `BulkSync` events always stay regardless of this field — see the
    /// module doc on [`crate::core::composite_sync_outbox`].
    outbox: Option<Arc<dyn CompositeSyncOutbox>>,

    /// Same-process fast-path wake for the outbox worker loop: the writer
    /// that just durably enqueued a row is already awake in the request
    /// path, so it can nudge its own idle worker without a DB round trip.
    /// Cross-instance pickup (including crash recovery) rides the worker's
    /// poll floor, never this hint — see the module doc.
    outbox_wake: Arc<Notify>,
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
            outbox: None,
            outbox_wake: Arc::new(Notify::new()),
        }
    }

    /// Wires the durable outbox (E1). Capability-based, not
    /// `HFS_CLUSTER`-gated — see the module doc on
    /// [`crate::core::composite_sync_outbox`] for why this is unconditional
    /// whenever the primary backend supports it.
    pub fn with_composite_sync_outbox(mut self, outbox: Arc<dyn CompositeSyncOutbox>) -> Self {
        self.outbox = Some(outbox);
        self
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
    ///
    /// `Create`/`Update`/`Delete` events route to the durable outbox (E1)
    /// when one is wired; `BulkSync` always stays on the in-memory channel
    /// below regardless (see the module doc on
    /// `crate::core::composite_sync_outbox` for why).
    async fn sync_asynchronous(
        &self,
        event: &SyncEvent,
        backends: &HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>,
    ) -> StorageResult<Vec<SyncStatus>> {
        if let Some(ref outbox) = self.outbox {
            if !matches!(event, SyncEvent::BulkSync { .. }) {
                return self
                    .sync_asynchronous_durable(event, backends, outbox.as_ref())
                    .await;
            }
        }

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
            Err(StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: "sync".to_string(),
                message: "Async sync mode is configured but the async worker was never started. \
                          Call start_async_worker() before processing events."
                    .to_string(),
            }))
        }
    }

    /// Durably enqueues `event` (`Create`/`Update`/`Delete` only) as one
    /// outbox row per backend, then nudges this instance's own idle
    /// worker(s) — a same-process fast path, not required for correctness
    /// (see the module doc on `crate::core::composite_sync_outbox`).
    /// Returns immediately per-backend status, mirroring the pre-existing
    /// channel path's "accepted" contract — the entry is now durable rather
    /// than merely queued in memory.
    async fn sync_asynchronous_durable(
        &self,
        event: &SyncEvent,
        backends: &HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>,
        outbox: &dyn CompositeSyncOutbox,
    ) -> StorageResult<Vec<SyncStatus>> {
        let tenant =
            TenantContext::new(event.tenant_id().clone(), TenantPermissions::full_access());
        let mut results = Vec::with_capacity(backends.len());

        for backend_id in backends.keys() {
            let entry = Self::to_outbox_entry(event, backend_id);
            match outbox.enqueue(&tenant, entry).await {
                Ok(_id) => {
                    let mut status_map = self.status.write();
                    let status = status_map.entry(backend_id.clone()).or_default();
                    status.pending_events += 1;
                    results.push(SyncStatus {
                        backend_id: backend_id.clone(),
                        success: true,
                        error: None,
                        retry_count: 0,
                        duration: Duration::ZERO,
                    });
                }
                Err(e) => {
                    results.push(SyncStatus {
                        backend_id: backend_id.clone(),
                        success: false,
                        error: Some(e.to_string()),
                        retry_count: 0,
                        duration: Duration::ZERO,
                    });
                }
            }
        }

        self.outbox_wake.notify_waiters();
        Ok(results)
    }

    /// Converts a `Create`/`Update`/`Delete` event into an outbox row
    /// targeting `backend_id`. Panics on `BulkSync` — callers must filter
    /// it out first (see `sync_asynchronous`).
    fn to_outbox_entry(event: &SyncEvent, backend_id: &str) -> NewSyncOutboxEntry {
        match event {
            SyncEvent::Create {
                resource_type,
                resource_id,
                content,
                tenant_id,
                fhir_version,
            } => NewSyncOutboxEntry {
                backend_id: backend_id.to_string(),
                operation: SyncOperation::Create,
                resource_type: resource_type.clone(),
                resource_id: resource_id.clone(),
                content: Some(content.clone()),
                version: None,
                tenant_id: tenant_id.as_str().to_string(),
                fhir_version: Some(fhir_version.as_str().to_string()),
            },
            SyncEvent::Update {
                resource_type,
                resource_id,
                content,
                tenant_id,
                version,
                fhir_version,
            } => NewSyncOutboxEntry {
                backend_id: backend_id.to_string(),
                operation: SyncOperation::Update,
                resource_type: resource_type.clone(),
                resource_id: resource_id.clone(),
                content: Some(content.clone()),
                version: Some(version.clone()),
                tenant_id: tenant_id.as_str().to_string(),
                fhir_version: Some(fhir_version.as_str().to_string()),
            },
            SyncEvent::Delete {
                resource_type,
                resource_id,
                tenant_id,
            } => NewSyncOutboxEntry {
                backend_id: backend_id.to_string(),
                operation: SyncOperation::Delete,
                resource_type: resource_type.clone(),
                resource_id: resource_id.clone(),
                content: None,
                version: None,
                tenant_id: tenant_id.as_str().to_string(),
                fhir_version: None,
            },
            SyncEvent::BulkSync { .. } => {
                unreachable!("BulkSync stays on the in-memory channel path — see sync_asynchronous")
            }
        }
    }

    fn require_content(claimed: &ClaimedSyncEvent) -> StorageResult<Value> {
        claimed.content.clone().ok_or_else(|| {
            StorageError::Backend(BackendError::Internal {
                backend_name: "composite-sync".to_string(),
                message: format!(
                    "composite sync outbox row {} is missing content for {:?}",
                    claimed.lease.id, claimed.operation
                ),
                source: None,
            })
        })
    }

    fn require_fhir_version(claimed: &ClaimedSyncEvent) -> StorageResult<FhirVersion> {
        claimed
            .fhir_version
            .as_deref()
            .and_then(FhirVersion::from_storage)
            .ok_or_else(|| {
                StorageError::Backend(BackendError::Internal {
                    backend_name: "composite-sync".to_string(),
                    message: format!(
                        "composite sync outbox row {} is missing/unparseable fhir_version for {:?}",
                        claimed.lease.id, claimed.operation
                    ),
                    source: None,
                })
            })
    }

    /// Applies one claimed outbox row to `backend`. `Create`/`Update` mirror
    /// `sync_event_to_backend`'s single-attempt body (`Update` is a
    /// `create_or_update` since secondaries don't track versions); `Delete`
    /// needs neither content nor a FHIR version.
    async fn apply_claimed_sync_event(
        claimed: &ClaimedSyncEvent,
        backend: &(dyn ResourceStorage + Send + Sync),
    ) -> StorageResult<()> {
        let tenant = TenantContext::new(
            claimed.lease.tenant.tenant_id().clone(),
            TenantPermissions::full_access(),
        );
        match claimed.operation {
            SyncOperation::Create => {
                let content = Self::require_content(claimed)?;
                let fhir_version = Self::require_fhir_version(claimed)?;
                backend
                    .create(&tenant, &claimed.resource_type, content, fhir_version)
                    .await
                    .map(|_| ())
            }
            SyncOperation::Update => {
                let content = Self::require_content(claimed)?;
                let fhir_version = Self::require_fhir_version(claimed)?;
                backend
                    .create_or_update(
                        &tenant,
                        &claimed.resource_type,
                        &claimed.resource_id,
                        content,
                        fhir_version,
                    )
                    .await
                    .map(|_| ())
            }
            SyncOperation::Delete => {
                backend
                    .delete(&tenant, &claimed.resource_type, &claimed.resource_id)
                    .await
            }
        }
    }

    /// Same backoff schedule as `sync_event_to_backend`'s in-process retry
    /// loop (`initial_delay * backoff_multiplier^(attempts-1)`, capped at
    /// `max_delay`), applied as a persisted `next_attempt_at` instead of a
    /// `sleep`.
    fn retry_delay(retry_config: &RetryConfig, attempts: u32) -> Duration {
        let exponent = attempts.saturating_sub(1) as i32;
        let secs = (retry_config.initial_delay.as_secs_f64()
            * retry_config.backoff_multiplier.powi(exponent))
        .min(retry_config.max_delay.as_secs_f64())
        .max(0.0);
        Duration::from_secs_f64(secs)
    }

    async fn finish_composite_sync_failure(
        outbox: &dyn CompositeSyncOutbox,
        lease: &SyncLease,
        error_message: &str,
        status: &Arc<RwLock<HashMap<String, BackendSyncStatus>>>,
        backend_id: &str,
    ) {
        match outbox.fail(lease, error_message).await {
            Ok(()) => {
                let mut status_map = status.write();
                let backend_status = status_map.entry(backend_id.to_string()).or_default();
                backend_status.total_errors += 1;
                if backend_status.pending_events > 0 {
                    backend_status.pending_events -= 1;
                }
            }
            Err(SyncLeaseError::LeaseLost { .. }) => warn!(
                entry_id = %lease.id,
                "Lease lost while failing a composite sync entry; the new holder decides its fate"
            ),
            Err(e) => warn!(
                entry_id = %lease.id,
                error = %e,
                "Failed to record composite sync failure"
            ),
        }
    }

    /// One deterministic composite-sync outbox worker cycle (E1): claim the
    /// next due entry, apply it exactly once, and record the outcome
    /// (complete / scheduled retry / terminal failure). Returns whether an
    /// entry was claimed — the polling loop drains until `false`; tests
    /// drive single cycles. Mirrors
    /// `SubscriptionEngine::run_next_subscription_delivery`'s posture.
    pub async fn run_next_composite_sync(
        &self,
        worker_id: &WorkerId,
        lease_duration: Duration,
        secondaries: &HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>,
    ) -> bool {
        let Some(outbox) = &self.outbox else {
            return false;
        };
        Self::run_next_composite_sync_inner(
            outbox.as_ref(),
            worker_id,
            lease_duration,
            secondaries,
            &self.config.retry,
            &self.status,
        )
        .await
    }

    async fn run_next_composite_sync_inner(
        outbox: &dyn CompositeSyncOutbox,
        worker_id: &WorkerId,
        lease_duration: Duration,
        secondaries: &HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>,
        retry_config: &RetryConfig,
        status: &Arc<RwLock<HashMap<String, BackendSyncStatus>>>,
    ) -> bool {
        let claimed = match outbox.claim_next(worker_id, lease_duration).await {
            Ok(Some(claimed)) => claimed,
            Ok(None) => return false,
            Err(e) => {
                warn!(error = %e, "Composite sync outbox claim failed");
                return false;
            }
        };

        let Some(backend) = secondaries.get(&claimed.backend_id) else {
            warn!(
                backend_id = %claimed.backend_id,
                entry_id = %claimed.lease.id,
                "Failing orphaned composite sync entry: backend no longer configured on this instance"
            );
            Self::finish_composite_sync_failure(
                outbox,
                &claimed.lease,
                "backend no longer configured on this instance",
                status,
                &claimed.backend_id,
            )
            .await;
            return true;
        };

        match Self::apply_claimed_sync_event(&claimed, backend.as_ref()).await {
            Ok(()) => match outbox.complete(&claimed.lease).await {
                Ok(()) => {
                    let mut status_map = status.write();
                    let backend_status = status_map.entry(claimed.backend_id.clone()).or_default();
                    backend_status.last_success = Some(std::time::Instant::now());
                    backend_status.total_synced += 1;
                    backend_status.healthy = true;
                    if backend_status.pending_events > 0 {
                        backend_status.pending_events -= 1;
                    }
                }
                Err(SyncLeaseError::LeaseLost { .. }) => warn!(
                    backend_id = %claimed.backend_id,
                    entry_id = %claimed.lease.id,
                    "Applied but the lease was lost; another worker may re-apply (at-least-once)"
                ),
                Err(e) => warn!(
                    backend_id = %claimed.backend_id,
                    entry_id = %claimed.lease.id,
                    error = %e,
                    "Failed to record composite sync completion"
                ),
            },
            Err(e) => {
                if claimed.attempts <= retry_config.max_retries {
                    let delay = Self::retry_delay(retry_config, claimed.attempts);
                    let next_attempt_at = Utc::now()
                        + chrono::Duration::from_std(delay)
                            .unwrap_or_else(|_| chrono::Duration::seconds(60));
                    if let Err(release_err) = outbox
                        .release_for_retry(&claimed.lease, next_attempt_at, &e.to_string())
                        .await
                    {
                        warn!(
                            backend_id = %claimed.backend_id,
                            entry_id = %claimed.lease.id,
                            error = %release_err,
                            "Failed to re-queue composite sync entry"
                        );
                    }
                } else {
                    error!(
                        backend_id = %claimed.backend_id,
                        entry_id = %claimed.lease.id,
                        attempts = claimed.attempts,
                        error = %e,
                        "Composite sync max retries exhausted"
                    );
                    Self::finish_composite_sync_failure(
                        outbox,
                        &claimed.lease,
                        &e.to_string(),
                        status,
                        &claimed.backend_id,
                    )
                    .await;
                }
            }
        }
        true
    }

    /// Spawns the per-instance composite-sync outbox worker loop (E1):
    /// drains due entries, then waits on a poll tick or a same-process wake
    /// hint — correctness rides the poll floor, never the hint (see the
    /// module doc on `crate::core::composite_sync_outbox`). Piggybacks
    /// terminal-row reaping. No-op without a wired outbox.
    pub fn spawn_composite_sync_workers(
        &self,
        secondaries: HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>,
        worker_count: usize,
    ) {
        const POLL_INTERVAL: Duration = Duration::from_secs(1);
        const LEASE_DURATION: Duration = Duration::from_secs(60);
        /// Reap terminal rows older than this, every `REAP_EVERY` idle laps.
        const REAP_TTL_SECS: i64 = 24 * 3600;
        const REAP_EVERY: u64 = 300;

        let Some(outbox) = self.outbox.clone() else {
            return;
        };
        let instance_id = uuid::Uuid::new_v4().simple().to_string();
        for worker_index in 0..worker_count.max(1) {
            let outbox = Arc::clone(&outbox);
            let secondaries = secondaries.clone();
            let retry_config = self.config.retry.clone();
            let status = self.status.clone();
            let wake = self.outbox_wake.clone();
            let worker_id = WorkerId::new(format!("{instance_id}-composite-sync-{worker_index}"));
            tokio::spawn(async move {
                let mut laps: u64 = 0;
                loop {
                    while Self::run_next_composite_sync_inner(
                        outbox.as_ref(),
                        &worker_id,
                        LEASE_DURATION,
                        &secondaries,
                        &retry_config,
                        &status,
                    )
                    .await
                    {}
                    laps += 1;
                    if laps.is_multiple_of(REAP_EVERY) {
                        let cutoff = Utc::now() - chrono::Duration::seconds(REAP_TTL_SECS);
                        if let Err(e) = outbox.delete_terminal_before(cutoff).await {
                            warn!(error = %e, "Composite sync outbox reap failed");
                        }
                    }
                    tokio::select! {
                        _ = tokio::time::sleep(POLL_INTERVAL) => {}
                        _ = wake.notified() => {}
                    }
                }
            });
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
}
