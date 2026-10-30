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
//!     resource: stored_patient,
//! }, &secondaries).await?;
//! ```

use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::FutureExt;
use parking_lot::RwLock;
use tokio::sync::{mpsc, oneshot};
use tokio::time::sleep;
use tracing::{debug, warn};

use crate::core::ResourceStorage;
use crate::error::{BackendError, ResourceError, StorageError, StorageResult};
use crate::tenant::{TenantContext, TenantId, TenantPermissions};
use crate::types::StoredResource;

use super::config::{RetryConfig, SyncConfig, SyncMode};
use super::sync_failures::{SyncFailureKey, SyncFailureRecorder, SyncOperation};

/// A synchronization event to propagate to secondary backends.
#[derive(Debug, Clone)]
pub enum SyncEvent {
    /// Resource was created. Carries the primary's [`StoredResource`] so a
    /// search secondary can index `version_id` and `last_modified` without
    /// recovering them from the JSON body.
    Create {
        /// The resource the primary stored.
        resource: StoredResource,
    },

    /// Resource was updated. Same payload as [`Self::Create`]: the search
    /// secondary overwrites its document from this snapshot.
    Update {
        /// The resource the primary stored.
        resource: StoredResource,
    },

    /// Resource was deleted.
    Delete {
        /// Resource type.
        resource_type: String,
        /// Resource ID.
        resource_id: String,
        /// Tenant ID.
        tenant_id: TenantId,
        /// Tombstone version id of the delete, when the primary reported it.
        /// `None` means the delete is applied without an ordering guard.
        version_id: Option<String>,
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
    /// A create of `resource` on every secondary.
    pub fn created(resource: StoredResource) -> Self {
        Self::Create { resource }
    }

    /// An update of `resource` on every secondary.
    pub fn updated(resource: StoredResource) -> Self {
        Self::Update { resource }
    }

    /// Returns the resource type for this event.
    pub fn resource_type(&self) -> &str {
        match self {
            SyncEvent::Create { resource } | SyncEvent::Update { resource } => {
                resource.resource_type()
            }
            SyncEvent::Delete { resource_type, .. } => resource_type,
            SyncEvent::BulkSync { .. } => "bulk",
        }
    }

    /// Returns the resource ID for this event (if applicable).
    pub fn resource_id(&self) -> Option<&str> {
        match self {
            SyncEvent::Create { resource } | SyncEvent::Update { resource } => Some(resource.id()),
            SyncEvent::Delete { resource_id, .. } => Some(resource_id),
            SyncEvent::BulkSync { .. } => None,
        }
    }

    /// Returns the tenant ID for this event.
    pub fn tenant_id(&self) -> &TenantId {
        match self {
            SyncEvent::Create { resource } | SyncEvent::Update { resource } => resource.tenant_id(),
            SyncEvent::Delete { tenant_id, .. } => tenant_id,
            SyncEvent::BulkSync { tenant_id, .. } => tenant_id,
        }
    }

    /// How many resources this event writes: one, or a bulk sync's page.
    fn resource_count(&self) -> usize {
        match self {
            SyncEvent::BulkSync { resources, .. } => resources.len(),
            _ => 1,
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

    /// Resources queued for the async worker and not yet written or
    /// recorded, across every backend. Published as a gauge (#1545).
    queue_depth: Arc<AtomicU64>,

    /// Sync status per backend.
    status: Arc<RwLock<HashMap<String, BackendSyncStatus>>>,

    /// Where every final outcome is reported (#1334): the failure metric,
    /// the structured event, and the durable "needs reindex" record.
    recorder: Arc<SyncFailureRecorder>,
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

/// Events queued together for async processing, in commit order.
///
/// A whole write goes on the queue as one item — a transaction bundle's
/// every entry, a `create_many` page — so handing it over is a single send
/// rather than one per resource. A send either completes or it does not:
/// a write whose future is dropped mid-hand-off can no longer leave part of
/// its committed resources queued and the rest neither queued nor recorded
/// (#1545).
struct QueuedBatch {
    events: Vec<SyncEvent>,
    backend_ids: Arc<[String]>,
}

impl QueuedBatch {
    fn resource_count(&self) -> usize {
        self.events.iter().map(SyncEvent::resource_count).sum()
    }
}

/// An item on the asynchronous sync queue.
enum Queued {
    /// Sync events bound for the named backends.
    Batch(QueuedBatch),
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

/// Writes the primary's snapshot to one secondary.
///
/// Elasticsearch (a search document store) goes through
/// [`ReindexTarget::write_search_entries`], which already indexes
/// `version_id` and `last_modified` from the [`StoredResource`]. A replica
/// that stores the resource itself uses `create` or `create_or_update`.
async fn write_primary_resource(
    backend: &dyn ResourceStorage,
    tenant: &TenantContext,
    resource: &StoredResource,
    as_create: bool,
) -> StorageResult<()> {
    if let Some(index) = backend.as_search_index() {
        index
            .write_search_entries(tenant, resource)
            .await
            .map(|_| ())
    } else if as_create {
        backend
            .create(
                tenant,
                resource.resource_type(),
                resource.content_with_meta(),
                resource.fhir_version(),
            )
            .await
            .map(|_| ())
    } else {
        backend
            .create_or_update(
                tenant,
                resource.resource_type(),
                resource.id(),
                resource.content_with_meta(),
                resource.fhir_version(),
            )
            .await
            .map(|_| ())
    }
}

/// Writes a same-type batch the primary already stored.
async fn write_primary_resources(
    backend: &dyn ResourceStorage,
    tenant: &TenantContext,
    resource_type: &str,
    resources: &[StoredResource],
) -> Vec<StorageResult<()>> {
    if let Some(index) = backend.as_search_index() {
        return index
            .write_synced_entries_page(tenant, resources)
            .await
            .into_iter()
            .map(|result| result.map(|_| ()))
            .collect();
    }
    let fhir_version = resources
        .first()
        .map(StoredResource::fhir_version)
        .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);
    let contents = resources
        .iter()
        .map(StoredResource::content_with_meta)
        .collect();
    backend
        .create_many(tenant, resource_type, contents, fhir_version)
        .await
        .into_iter()
        .map(|result| result.map(|_| ()))
        .collect()
}

/// Consecutive queued creates the worker writes as one batch: same tenant,
/// resource type, FHIR version and backends.
struct CreateRun {
    tenant_id: TenantId,
    resource_type: String,
    fhir_version: helios_fhir::FhirVersion,
    backend_ids: Arc<[String]>,
    resources: Vec<StoredResource>,
}

impl CreateRun {
    fn new(resource: StoredResource, backend_ids: &Arc<[String]>) -> Self {
        Self {
            tenant_id: resource.tenant_id().clone(),
            resource_type: resource.resource_type().to_string(),
            fhir_version: resource.fhir_version(),
            backend_ids: backend_ids.clone(),
            resources: vec![resource],
        }
    }

    fn takes(&self, resource: &StoredResource, backend_ids: &Arc<[String]>) -> bool {
        self.tenant_id == *resource.tenant_id()
            && self.resource_type == resource.resource_type()
            && self.fhir_version == resource.fhir_version()
            && *self.backend_ids == **backend_ids
    }

    /// The run's resources with only the last copy of each id kept, in the
    /// order those last copies were queued. An earlier copy would only be
    /// overwritten by the later one in the same batch.
    fn latest_copies(&self) -> Vec<StoredResource> {
        let mut seen = HashSet::new();
        let mut latest: Vec<StoredResource> = self
            .resources
            .iter()
            .rev()
            .filter(|resource| seen.insert(resource.id()))
            .cloned()
            .collect();
        latest.reverse();
        latest
    }
}

/// Pending and queue-depth counts for a batch being queued; taken back on
/// drop unless [`keep`](Self::keep) is called once the batch is on the queue.
struct PendingCount<'a> {
    manager: &'a SyncManager,
    backend_ids: &'a [String],
    events: usize,
    resources: usize,
}

impl<'a> PendingCount<'a> {
    fn add(
        manager: &'a SyncManager,
        backend_ids: &'a [String],
        events: usize,
        resources: usize,
    ) -> Self {
        let count = Self {
            manager,
            backend_ids,
            events,
            resources,
        };
        count.apply(true);
        count
    }

    fn keep(self) {
        std::mem::forget(self);
    }

    fn apply(&self, add: bool) {
        {
            let mut status_map = self.manager.status.write();
            for id in self.backend_ids {
                let status = status_map.entry(id.clone()).or_default();
                status.pending_events = if add {
                    status.pending_events + self.events
                } else {
                    status.pending_events.saturating_sub(self.events)
                };
            }
        }
        let n = self.resources as u64;
        let depth = if add {
            self.manager.queue_depth.fetch_add(n, Ordering::Relaxed) + n
        } else {
            self.manager.queue_depth.fetch_sub(n, Ordering::Relaxed) - n
        };
        self.manager.recorder.queue_depth(depth);
    }
}

impl Drop for PendingCount<'_> {
    fn drop(&mut self) {
        self.apply(false);
    }
}

/// The error recorded for writes whose sync task panicked.
fn sync_panicked(backend_id: &str) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: backend_id.to_string(),
        message: "secondary sync panicked".to_string(),
        source: None,
    })
}

/// What the async worker shares with the manager.
struct WorkerContext {
    backends: HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>,
    config: SyncConfig,
    status: Arc<RwLock<HashMap<String, BackendSyncStatus>>>,
    recorder: Arc<SyncFailureRecorder>,
    queue_depth: Arc<AtomicU64>,
}

impl WorkerContext {
    /// Records `n` processed events (written or recorded as failed) on
    /// `backend_id`.
    fn settle_pending(&self, backend_id: &str, n: usize) {
        let mut status_map = self.status.write();
        let backend_status = status_map.entry(backend_id.to_string()).or_default();
        backend_status.pending_events = backend_status.pending_events.saturating_sub(n);
    }

    /// `n` queued resources are done with, on every backend.
    fn dequeued(&self, n: usize) {
        let depth = self.queue_depth.fetch_sub(n as u64, Ordering::Relaxed) - n as u64;
        self.recorder.queue_depth(depth);
    }
}

impl SyncManager {
    /// Creates a new sync manager.
    pub fn new(config: SyncConfig) -> Self {
        Self {
            config,
            event_sender: None,
            queue_depth: Arc::new(AtomicU64::new(0)),
            status: Arc::new(RwLock::new(HashMap::new())),
            recorder: Arc::new(SyncFailureRecorder::default()),
        }
    }

    /// The recorder every final sync outcome is reported to (#1334).
    pub fn failure_recorder(&self) -> &Arc<SyncFailureRecorder> {
        &self.recorder
    }

    /// Starts the async sync worker.
    pub fn start_async_worker(
        &mut self,
        backends: HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>,
    ) -> tokio::task::JoinHandle<()> {
        let (sender, receiver) = mpsc::channel::<Queued>(1000);
        self.event_sender = Some(sender);

        let context = WorkerContext {
            backends,
            config: self.config.clone(),
            status: self.status.clone(),
            recorder: self.recorder.clone(),
            queue_depth: self.queue_depth.clone(),
        };

        tokio::spawn(async move {
            Self::async_worker(receiver, context).await;
        })
    }

    /// Async worker that processes queued events.
    ///
    /// It collects a page of up to `batch_size` resources — or whatever
    /// arrived within the 100ms batching window, or up to a barrier — and
    /// writes the page in queue order. Consecutive creates of one type go to
    /// each backend as one batch ([`write_primary_resources`]: Elasticsearch
    /// `_bulk`), with the same per-item fallback the synchronous path uses;
    /// updates, deletes and bulk syncs are written one at a time. Writing one
    /// document per round trip left the worker draining ~124 docs/s, below
    /// what a Synthea import commits, so the queue stayed full (#1545).
    async fn async_worker(mut receiver: mpsc::Receiver<Queued>, context: WorkerContext) {
        let batch_timeout = Duration::from_millis(100);

        loop {
            let Some(first) = receiver.recv().await else {
                return; // Channel closed
            };
            let deadline = tokio::time::Instant::now() + batch_timeout;
            let mut resources = Self::queued_resources(&first);
            let mut page = vec![first];
            let mut closed = false;

            // A barrier means someone is waiting on everything ahead of it:
            // process what is collected now rather than holding it for the
            // rest of the batch window.
            while resources < context.config.batch_size
                && !matches!(page.last(), Some(Queued::Barrier(_)))
            {
                match tokio::time::timeout_at(deadline, receiver.recv()).await {
                    Ok(Some(item)) => {
                        resources += Self::queued_resources(&item);
                        page.push(item);
                    }
                    Ok(None) => {
                        closed = true;
                        break;
                    }
                    Err(_) => break, // Timeout
                }
            }

            Self::process_page(page, &context).await;
            if closed {
                return;
            }
        }
    }

    fn queued_resources(item: &Queued) -> usize {
        match item {
            Queued::Batch(batch) => batch.resource_count(),
            Queued::Barrier(_) => 0,
        }
    }

    /// Writes one page of queued items, in queue order.
    async fn process_page(page: Vec<Queued>, context: &WorkerContext) {
        let mut run: Option<CreateRun> = None;

        for item in page {
            let batch = match item {
                Queued::Batch(batch) => batch,
                Queued::Barrier(done) => {
                    if let Some(run) = run.take() {
                        Self::write_run(run, context).await;
                    }
                    // Everything queued ahead of this point has been handed
                    // to its backends. The waiter may have given up; that is
                    // its business.
                    let _ = done.send(());
                    continue;
                }
            };
            for event in batch.events {
                match event {
                    SyncEvent::Create { resource } => match run.as_mut() {
                        Some(current) if current.takes(&resource, &batch.backend_ids) => {
                            current.resources.push(resource);
                        }
                        _ => {
                            if let Some(done) =
                                run.replace(CreateRun::new(resource, &batch.backend_ids))
                            {
                                Self::write_run(done, context).await;
                            }
                        }
                    },
                    event => {
                        if let Some(run) = run.take() {
                            Self::write_run(run, context).await;
                        }
                        Self::write_event(&event, &batch.backend_ids, context).await;
                    }
                }
            }
        }
        if let Some(run) = run.take() {
            Self::write_run(run, context).await;
        }
    }

    /// Writes a run of creates to each of its backends as a batch.
    async fn write_run(run: CreateRun, context: &WorkerContext) {
        let queued = run.resources.len();
        let mut resources = run.latest_copies();
        if let [_] = resources.as_slice() {
            // A batch of one gains nothing from the batch write and would pay
            // a batch attempt on top of the single write's retries.
            let event = SyncEvent::created(resources.remove(0));
            Self::write_event(&event, &run.backend_ids, context).await;
            // Earlier copies of the same id, superseded and never written.
            let superseded = queued - 1;
            if superseded > 0 {
                for backend_id in run.backend_ids.iter() {
                    if context.backends.contains_key(backend_id) {
                        context.settle_pending(backend_id, superseded);
                    }
                }
                context.dequeued(superseded);
            }
            return;
        }
        let tenant = TenantContext::new(run.tenant_id.clone(), TenantPermissions::full_access());

        for backend_id in run.backend_ids.iter() {
            let Some(backend) = context.backends.get(backend_id) else {
                continue;
            };
            let written = AssertUnwindSafe(Self::write_create_batch(
                backend_id,
                backend.as_ref(),
                &tenant,
                &run.resource_type,
                &resources,
                &context.config.retry,
                &context.recorder,
            ))
            .catch_unwind()
            .await;
            let (synced, errors) = match written {
                Ok((_, synced, errors)) => (synced, errors),
                Err(_) => {
                    // Which writes landed before the panic is unknown: record
                    // them all for the repair task, which is idempotent.
                    let error = sync_panicked(backend_id);
                    for resource in &resources {
                        context
                            .recorder
                            .sync_failed(
                                &SyncEvent::created(resource.clone()),
                                backend_id,
                                &error,
                                1,
                            )
                            .await;
                    }
                    (0, resources.len())
                }
            };

            let mut status_map = context.status.write();
            let backend_status = status_map.entry(backend_id.clone()).or_default();
            if synced > 0 {
                backend_status.last_success = Some(std::time::Instant::now());
                backend_status.total_synced += synced as u64;
                backend_status.healthy = true;
            }
            backend_status.total_errors += errors as u64;
            backend_status.pending_events = backend_status.pending_events.saturating_sub(queued);
        }
        context.dequeued(queued);
    }

    /// Writes a single event to each of its backends.
    async fn write_event(event: &SyncEvent, backend_ids: &[String], context: &WorkerContext) {
        for backend_id in backend_ids {
            let Some(backend) = context.backends.get(backend_id) else {
                continue;
            };
            let result = AssertUnwindSafe(Self::sync_event_to_backend(
                event,
                backend.as_ref(),
                &context.config.retry,
            ))
            .catch_unwind()
            .await
            .unwrap_or_else(|_| Err(sync_panicked(backend_id)));

            // Nobody is waiting on this event any more: the recorder is the
            // only witness of how it ended.
            match &result {
                Ok(()) => context.recorder.sync_succeeded(event, backend_id).await,
                Err(e) => {
                    context
                        .recorder
                        .sync_failed(event, backend_id, e, context.config.retry.max_retries + 1)
                        .await
                }
            }

            {
                let mut status_map = context.status.write();
                let backend_status = status_map.entry(backend_id.clone()).or_default();
                match result {
                    Ok(_) => {
                        backend_status.last_success = Some(std::time::Instant::now());
                        backend_status.total_synced += 1;
                        backend_status.healthy = true;
                    }
                    Err(_) => backend_status.total_errors += 1,
                }
            }
            context.settle_pending(backend_id, 1);
        }
        context.dequeued(event.resource_count());
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

    /// Synchronizes the events of one committed write, in order — a
    /// transaction bundle's deletes and creates.
    ///
    /// Where creates are queued (asynchronous mode, or hybrid without
    /// `sync_for_search`) every event goes on the queue as one item, a single
    /// send (#1545). Otherwise each run of consecutive same-tenant, same-type
    /// creates goes through [`sync_creates`](Self::sync_creates) as a batch,
    /// and every other event through [`sync`](Self::sync).
    pub async fn sync_events(
        &self,
        events: Vec<SyncEvent>,
        backends: &HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>,
    ) -> StorageResult<Vec<SyncStatus>> {
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let queued = match self.config.mode {
            SyncMode::Synchronous => false,
            SyncMode::Asynchronous => true,
            SyncMode::Hybrid { sync_for_search } => !sync_for_search,
        };
        if queued {
            return self.enqueue(events, backends).await;
        }

        let mut statuses = Vec::new();
        let mut events = events.into_iter().peekable();
        while let Some(event) = events.next() {
            let SyncEvent::Create { resource } = event else {
                statuses.extend(self.sync(&event, backends).await?);
                continue;
            };
            let tenant_id = resource.tenant_id().clone();
            let resource_type = resource.resource_type().to_string();
            let mut run = vec![resource];
            while let Some(SyncEvent::Create { resource }) = events.peek() {
                if *resource.tenant_id() != tenant_id || resource.resource_type() != resource_type {
                    break;
                }
                let Some(SyncEvent::Create { resource }) = events.next() else {
                    unreachable!("peeked a create");
                };
                run.push(resource);
            }
            statuses.extend(
                self.sync_creates(&tenant_id, &resource_type, run, backends)
                    .await?,
            );
        }
        Ok(statuses)
    }

    /// Synchronizes a batch of creates — one tenant, one resource type — to
    /// the secondary backends as a batch.
    ///
    /// In the synchronous modes every backend receives the whole batch
    /// through [`write_primary_resources`]: Elasticsearch writes the page
    /// with [`ReindexTarget::write_synced_entries_page`], and a replica
    /// uses [`ResourceStorage::create_many`]. A secondary with a native
    /// batch write (Elasticsearch `_bulk`) pays one round trip and, under
    /// `refresh=wait_for`, one refresh wait rather than one per resource.
    /// Items the batch rejects are retried one at a time through the same
    /// retrying path a single [`sync`](Self::sync) takes, so a partial batch
    /// failure degrades to per-resource sync rather than to lost writes.
    /// Asynchronous mode queues the whole batch as one item, which the
    /// worker writes the same way.
    pub async fn sync_creates(
        &self,
        tenant_id: &TenantId,
        resource_type: &str,
        resources: Vec<StoredResource>,
        backends: &HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>,
    ) -> StorageResult<Vec<SyncStatus>> {
        let synchronous = match self.config.mode {
            SyncMode::Synchronous => true,
            SyncMode::Asynchronous => false,
            SyncMode::Hybrid { sync_for_search } => sync_for_search,
        };
        if !synchronous {
            if resources.is_empty() {
                return Ok(Vec::new());
            }
            return self
                .enqueue(
                    resources.into_iter().map(SyncEvent::created).collect(),
                    backends,
                )
                .await;
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
            let recorder = self.recorder.clone();

            tasks.spawn(async move {
                Self::write_create_batch(
                    &backend_id,
                    backend.as_ref(),
                    &tenant,
                    &resource_type,
                    &resources,
                    &retry_config,
                    &recorder,
                )
                .await
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

    /// Writes a same-type batch the primary already stored to one backend,
    /// retrying rejected items one at a time, and reports every outcome to
    /// `recorder`. Returns the backend's status and how many resources it
    /// synced and failed.
    async fn write_create_batch(
        backend_id: &str,
        backend: &dyn ResourceStorage,
        tenant: &TenantContext,
        resource_type: &str,
        resources: &[StoredResource],
        retry_config: &RetryConfig,
        recorder: &SyncFailureRecorder,
    ) -> (SyncStatus, usize, usize) {
        let start = std::time::Instant::now();
        let key = |resource_id: &str| SyncFailureKey {
            tenant_id: tenant.tenant_id().as_str().to_string(),
            resource_type: resource_type.to_string(),
            resource_id: resource_id.to_string(),
            backend_id: backend_id.to_string(),
        };
        let results = write_primary_resources(backend, tenant, resource_type, resources).await;

        let mut synced = 0;
        let mut errors = 0;
        let mut last_error = None;
        let mut failed_resource_ids = Vec::new();
        // Set once a resource retried on its own failed because the
        // backend is unavailable. A cluster that stopped answering
        // makes every further retry wait out the request timeout
        // `max_retries + 1` times, serially: a batch of N would block
        // for N x (max_retries + 1) timeouts. The rest of the batch is
        // recorded as failed instead, for the repair task to re-sync
        // (#1749).
        let mut backend_unavailable = false;
        for (resource, result) in resources.iter().zip(results) {
            let resource_id = resource.id();
            let Err(batch_error) = result else {
                synced += 1;
                recorder.resource_sync_succeeded(key(resource_id)).await;
                continue;
            };
            if backend_unavailable {
                errors += 1;
                recorder
                    .resource_sync_failed(
                        key(resource_id),
                        SyncOperation::Create,
                        Some(resource.version_id()),
                        &batch_error,
                        // Only the batch attempt: no single retry was made.
                        1,
                    )
                    .await;
                last_error = Some(batch_error.to_string());
                failed_resource_ids.push(resource_id.to_string());
                continue;
            }
            warn!(
                backend_id = %backend_id,
                resource_type = %resource_type,
                resource_id = %resource_id,
                error = %batch_error,
                "Batch sync rejected a resource; retrying it individually"
            );
            let event = SyncEvent::created(resource.clone());
            match Self::sync_event_to_backend(&event, backend, retry_config).await {
                Ok(()) => {
                    synced += 1;
                    recorder.resource_sync_succeeded(key(resource_id)).await;
                }
                Err(e) => {
                    if matches!(e, StorageError::Backend(BackendError::Unavailable { .. })) {
                        warn!(
                            backend_id = %backend_id,
                            resource_type = %resource_type,
                            "Backend unavailable; not retrying the rest of the batch one by one"
                        );
                        backend_unavailable = true;
                    }
                    errors += 1;
                    recorder
                        .resource_sync_failed(
                            key(resource_id),
                            SyncOperation::Create,
                            Some(resource.version_id()),
                            &e,
                            // The batch attempt, then the retried single sync.
                            retry_config.max_retries + 2,
                        )
                        .await;
                    last_error = Some(e.to_string());
                    failed_resource_ids.push(resource_id.to_string());
                }
            }
        }

        (
            SyncStatus {
                backend_id: backend_id.to_string(),
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
            let recorder = self.recorder.clone();

            tasks.spawn(async move {
                let start = std::time::Instant::now();

                let result =
                    Self::sync_event_to_backend(&event, backend.as_ref(), &retry_config).await;
                match &result {
                    Ok(()) => recorder.sync_succeeded(&event, &backend_id).await,
                    Err(e) => {
                        recorder
                            .sync_failed(&event, &backend_id, e, retry_config.max_retries + 1)
                            .await
                    }
                }
                match result {
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
        self.enqueue(vec![event.clone()], backends).await
    }

    /// Queues `events`, in order, as one item for the async worker: a single
    /// send, so the events are either all queued or — if the caller is
    /// dropped while the queue is full — none of them are.
    async fn enqueue(
        &self,
        events: Vec<SyncEvent>,
        backends: &HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>,
    ) -> StorageResult<Vec<SyncStatus>> {
        let Some(ref sender) = self.event_sender else {
            return Err(sync_queue_unavailable(
                "Async sync mode is configured but the async worker was never started. \
                 Call start_async_worker() before processing events.",
            ));
        };
        let backend_ids: Arc<[String]> = backends.keys().cloned().collect();
        let batch = QueuedBatch {
            events,
            backend_ids: backend_ids.clone(),
        };
        // Counted before the send, because the worker may finish the batch
        // before `send` returns; undone if the send never completes — it
        // fails, or the caller is dropped while the queue is full.
        let counted = PendingCount::add(
            self,
            &backend_ids,
            batch.events.len(),
            batch.resource_count(),
        );
        sender
            .send(Queued::Batch(batch))
            .await
            .map_err(|e| sync_queue_unavailable(format!("Failed to queue sync event: {}", e)))?;
        counted.keep();

        // Return pending status
        Ok(backend_ids
            .iter()
            .map(|id| SyncStatus {
                backend_id: id.clone(),
                success: true,
                error: None,
                retry_count: 0,
                duration: Duration::ZERO,
                // Queued, not rejected: nothing is known to have failed yet.
                failed_resource_ids: Vec::new(),
            })
            .collect())
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
                SyncEvent::Create { resource } => {
                    let tenant = TenantContext::new(
                        resource.tenant_id().clone(),
                        TenantPermissions::full_access(),
                    );
                    write_primary_resource(backend, &tenant, resource, true).await
                }
                SyncEvent::Update { resource } => {
                    let tenant = TenantContext::new(
                        resource.tenant_id().clone(),
                        TenantPermissions::full_access(),
                    );
                    write_primary_resource(backend, &tenant, resource, false).await
                }
                SyncEvent::Delete {
                    resource_type,
                    resource_id,
                    tenant_id,
                    version_id,
                } => {
                    let tenant =
                        TenantContext::new(tenant_id.clone(), TenantPermissions::full_access());
                    // A delete that carries the tombstone version goes to a
                    // search index as a versioned tombstone, so a late sync of
                    // an older version cannot bring the resource back (#2288).
                    let deleted = match (version_id, backend.as_search_index()) {
                        (Some(version), Some(index)) => index
                            .tombstone_search_entries(&tenant, resource_type, resource_id, version)
                            .await
                            .map(|_| ()),
                        _ => backend.delete(&tenant, resource_type, resource_id).await,
                    };
                    match deleted {
                        // The secondary does not have it, which is the state
                        // the delete asks for: a create that never reached it,
                        // or a delete whose answer was lost. Retrying cannot
                        // change that, and it is not a failure (#1334).
                        Err(StorageError::Resource(
                            ResourceError::NotFound { .. } | ResourceError::Gone { .. },
                        )) => Ok(()),
                        other => other,
                    }
                }
                SyncEvent::BulkSync {
                    resources,
                    tenant_id,
                } => {
                    let tenant =
                        TenantContext::new(tenant_id.clone(), TenantPermissions::full_access());
                    if let Some(index) = backend.as_search_index() {
                        for result in index.write_synced_entries_page(&tenant, resources).await {
                            result?;
                        }
                    } else {
                        for resource in resources {
                            write_primary_resource(backend, &tenant, resource, false).await?;
                        }
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
    use serde_json::Value;

    #[test]
    fn test_sync_event_accessors() {
        let event = SyncEvent::created(StoredResource::new(
            "Patient",
            "123",
            TenantId::new("test"),
            serde_json::json!({}),
            FhirVersion::default(),
        ));

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

    fn stored_patient(id: &str) -> StoredResource {
        StoredResource::new(
            "Patient",
            id,
            TenantId::new("t"),
            serde_json::json!({"resourceType": "Patient", "id": id}),
            FhirVersion::default(),
        )
    }

    fn create_event(id: &str) -> SyncEvent {
        SyncEvent::created(stored_patient(id))
    }

    fn update_event(id: &str) -> SyncEvent {
        SyncEvent::updated(stored_patient(id))
    }

    fn delete_event(id: &str) -> SyncEvent {
        SyncEvent::Delete {
            resource_type: "Patient".to_string(),
            resource_id: id.to_string(),
            tenant_id: TenantId::new("t"),
            version_id: None,
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

    // ------------------------------------------------------------------
    // The batching worker and the single-send hand-off (#1545)
    // ------------------------------------------------------------------

    /// A secondary that records each write it receives: every `create_many`
    /// batch as its ids, and every single-resource write. A create of
    /// `boom` panics.
    #[derive(Default)]
    struct RecordingBackend {
        writes: Mutex<Vec<String>>,
    }

    impl RecordingBackend {
        fn writes(&self) -> Vec<String> {
            self.writes.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ResourceStorage for RecordingBackend {
        fn backend_name(&self) -> &'static str {
            "recording"
        }

        async fn create(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            resource: Value,
            fhir_version: FhirVersion,
        ) -> StorageResult<StoredResource> {
            let id = resource["id"].as_str().unwrap_or("").to_string();
            self.writes.lock().unwrap().push(format!("create {id}"));
            Ok(StoredResource::new(
                resource_type,
                id,
                tenant.tenant_id().clone(),
                resource,
                fhir_version,
            ))
        }

        async fn create_many(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            resources: Vec<Value>,
            fhir_version: FhirVersion,
        ) -> Vec<StorageResult<StoredResource>> {
            let ids: Vec<String> = resources
                .iter()
                .map(|r| r["id"].as_str().unwrap_or("").to_string())
                .collect();
            assert!(!ids.iter().any(|id| id == "boom"), "boom");
            self.writes
                .lock()
                .unwrap()
                .push(format!("batch {resource_type} {}", ids.join(",")));
            resources
                .into_iter()
                .zip(ids)
                .map(|(resource, id)| {
                    Ok(StoredResource::new(
                        resource_type,
                        id,
                        tenant.tenant_id().clone(),
                        resource,
                        fhir_version,
                    ))
                })
                .collect()
        }

        async fn create_or_update(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            id: &str,
            resource: Value,
            fhir_version: FhirVersion,
        ) -> StorageResult<(StoredResource, bool)> {
            self.writes.lock().unwrap().push(format!("update {id}"));
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
            self.writes.lock().unwrap().push(format!("delete {id}"));
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

    /// Counts final failures and tracks the published queue depth.
    #[derive(Default)]
    struct CountingObserver {
        failures: std::sync::atomic::AtomicUsize,
        depth: AtomicU64,
    }

    impl super::super::sync_failures::SecondarySyncObserver for CountingObserver {
        fn sync_failed(&self, _backend_id: &str, _operation: SyncOperation) {
            self.failures.fetch_add(1, Ordering::Relaxed);
        }

        fn needs_reindex(&self, _outstanding: u64) {}

        fn queue_depth(&self, depth: u64) {
            self.depth.store(depth, Ordering::Relaxed);
        }
    }

    type Backends = HashMap<String, Arc<dyn ResourceStorage + Send + Sync>>;

    fn recording() -> (
        Arc<RecordingBackend>,
        Backends,
        SyncManager,
        Arc<CountingObserver>,
    ) {
        let backend = Arc::new(RecordingBackend::default());
        let mut map = Backends::new();
        map.insert("search".to_string(), backend.clone());
        let mut manager = SyncManager::new(async_config());
        let observer = Arc::new(CountingObserver::default());
        manager.failure_recorder().set_observer(observer.clone());
        drop(manager.start_async_worker(map.clone()));
        (backend, map, manager, observer)
    }

    fn stored_observation(id: &str) -> StoredResource {
        StoredResource::new(
            "Observation",
            id,
            TenantId::new("t"),
            serde_json::json!({"resourceType": "Observation", "id": id}),
            FhirVersion::default(),
        )
    }

    /// A batch of creates is queued as one item and written to the backend
    /// as one batch, not one document per round trip.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn queued_creates_are_written_as_one_batch() {
        let (backend, backends, manager, observer) = recording();
        let resources: Vec<_> = (0..50).map(|i| stored_patient(&format!("p{i}"))).collect();
        let ids: Vec<_> = (0..50).map(|i| format!("p{i}")).collect();

        manager
            .sync_creates(&TenantId::new("t"), "Patient", resources, &backends)
            .await
            .unwrap();
        manager.barrier().await.unwrap();

        assert_eq!(
            backend.writes(),
            vec![format!("batch Patient {}", ids.join(","))]
        );
        assert_eq!(manager.backend_status("search").unwrap().pending_events, 0);
        assert_eq!(manager.backend_status("search").unwrap().total_synced, 50);
        assert_eq!(observer.depth.load(Ordering::Relaxed), 0);
    }

    /// Creates queued by separate writes coalesce into one batch per run of
    /// one type; anything else in between ends the run, so queue order
    /// holds. Within a run only the latest copy of an id is written.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_worker_batches_runs_of_creates_in_queue_order() {
        let (backend, backends, manager, _observer) = recording();

        let events = vec![
            create_event("a"),
            create_event("b"),
            SyncEvent::created(stored_observation("o1")),
            create_event("c"),
            delete_event("a"),
            create_event("d"),
            create_event("d"),
            update_event("d"),
        ];
        manager.sync_events(events, &backends).await.unwrap();
        manager.sync(&create_event("e"), &backends).await.unwrap();
        manager.sync(&create_event("f"), &backends).await.unwrap();
        manager.barrier().await.unwrap();

        assert_eq!(
            backend.writes(),
            vec![
                "batch Patient a,b",
                "create o1",
                "create c",
                "delete a",
                "create d",
                "update d",
                "batch Patient e,f",
            ]
        );
        assert_eq!(manager.backend_status("search").unwrap().pending_events, 0);
    }

    /// A run that panics is recorded as failed, and the worker carries on
    /// with what is queued behind it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_panicking_run_is_recorded_and_the_worker_survives() {
        let (backend, backends, manager, observer) = recording();

        manager
            .sync_events(
                vec![create_event("x"), create_event("boom"), delete_event("y")],
                &backends,
            )
            .await
            .unwrap();
        manager.barrier().await.unwrap();
        manager.sync(&create_event("z"), &backends).await.unwrap();
        manager
            .barrier()
            .await
            .expect("the worker is still running");

        assert_eq!(backend.writes(), vec!["delete y", "create z"]);
        assert_eq!(observer.failures.load(Ordering::Relaxed), 2, "x and boom");
        let status = manager.backend_status("search").unwrap();
        assert_eq!(status.pending_events, 0);
        assert_eq!(status.total_errors, 2);
        assert_eq!(observer.depth.load(Ordering::Relaxed), 0);
    }

    /// A write dropped while its hand-off waits on a full queue queues
    /// none of its events — never some of them — and leaves no pending
    /// count behind.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_dropped_hand_off_queues_all_or_nothing() {
        let (backend, mut starts) = GatedBackend::new();
        let backends = backends(&backend);
        let mut manager = SyncManager::new(async_config());
        let observer = Arc::new(CountingObserver::default());
        manager.failure_recorder().set_observer(observer.clone());
        let _worker = manager.start_async_worker(backends.clone());

        // Hold the worker inside the backend, then fill the queue behind it.
        // `unconstrained`: a send must only ever wait because the queue is
        // full, never to give back tokio's cooperative budget.
        manager
            .sync(&create_event("held"), &backends)
            .await
            .unwrap();
        assert_eq!(starts.recv().await.as_deref(), Some("create held"));
        let mut queued = 1;
        loop {
            let event = update_event(&format!("q{queued}"));
            match tokio::task::unconstrained(manager.sync(&event, &backends)).now_or_never() {
                Some(result) => {
                    result.unwrap();
                    queued += 1;
                }
                None => break,
            }
        }

        assert!(queued > 1000, "the queue filled up: {queued} queued");

        // A committed bundle's hand-off is dropped while it waits.
        let bundle: Vec<_> = (0..20).map(|i| create_event(&format!("lost{i}"))).collect();
        assert!(
            tokio::task::unconstrained(manager.sync_events(bundle, &backends))
                .now_or_never()
                .is_none(),
            "the queue is full, so the hand-off waits"
        );

        backend.release(100_000);
        manager.barrier().await.unwrap();
        let completed = backend.completed();
        assert_eq!(completed.len(), queued, "{completed:?}");
        assert!(!completed.iter().any(|op| op.contains("lost")));
        assert_eq!(manager.backend_status("search").unwrap().pending_events, 0);
        assert_eq!(observer.depth.load(Ordering::Relaxed), 0);
    }
}
