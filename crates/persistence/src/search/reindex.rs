//! $reindex Operation Implementation.
//!
//! Provides the ability to rebuild search indexes for existing resources
//! when new SearchParameters are added or when indexes need to be repaired.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::core::cluster_job_store::{
    ClusterJobId, ClusterJobRecord, ClusterJobState, ClusterJobStore, ClusterLeaseError, JobKind,
    WorkerId,
};
use crate::error::StorageResult;
use crate::tenant::TenantContext;
use crate::types::StoredResource;

use super::errors::ReindexError;
use super::extractor::SearchParameterExtractor;

/// Audit event helpers for reindex operations.
pub mod audit {
    use helios_audit::{AuditAction, AuditEventBuilder, AuditSink};

    /// Record an audit event for a reindex lifecycle event.
    ///
    /// Call this at reindex start, completion, or cancellation.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_reindex_event(
        sink: &dyn AuditSink,
        source_observer: &str,
        agent: Option<&str>,
        job_id: &str,
        phase: &str,
        resource_types: &[String],
        resources_processed: u64,
        outcome: &str,
    ) {
        let mut builder = AuditEventBuilder::new(source_observer)
            .event_type(
                "http://terminology.hl7.org/CodeSystem/audit-event-type",
                "object",
            )
            .action(AuditAction::Execute)
            .outcome(outcome)
            .detail("audit-operation", "reindex")
            .detail("job-id", job_id)
            .detail("phase", phase)
            .detail("resources-processed", resources_processed.to_string());
        if !resource_types.is_empty() {
            builder = builder.detail("resource-types", resource_types.join(","));
        }
        if let Some(a) = agent {
            builder = builder.agent(a, None, true);
        }
        sink.record(builder.build()).await;
    }

    #[cfg(test)]
    mod tests {
        use helios_audit::{AuditAction, AuditEventBuilder};

        #[test]
        fn test_reindex_event_includes_resources_processed() {
            let event = AuditEventBuilder::new("Device/hfs")
                .event_type(
                    "http://terminology.hl7.org/CodeSystem/audit-event-type",
                    "object",
                )
                .action(AuditAction::Execute)
                .outcome("0")
                .detail("audit-operation", "reindex")
                .detail("job-id", "reindex-1")
                .detail("phase", "complete")
                .detail("resources-processed", "500")
                .build();
            let details = event.entity.as_ref().unwrap()[0].detail.as_ref().unwrap();
            assert_eq!(details.len(), 4);
            assert_eq!(
                details[3].r#type.value.as_deref(),
                Some("resources-processed")
            );
        }
    }
}

/// A page of resources for reindexing.
#[derive(Debug)]
pub struct ResourcePage {
    /// The resources in this page.
    pub resources: Vec<StoredResource>,
    /// Cursor for the next page (None if this is the last page).
    pub next_cursor: Option<String>,
}

/// A backend that can enumerate stored resources so they can be reindexed.
///
/// This is the *read* half of reindexing: where the resources come from. It is
/// split from [`ReindexTarget`] because the two capabilities do not line up
/// across our backends. S3 holds resources but has no search index of its own
/// (it is always paired with Elasticsearch), so it can be a source but never a
/// writer. Welding the two together is what previously forced `$reindex` to be
/// a SQL-only operation.
#[async_trait]
pub trait ReindexSource: Send + Sync {
    /// Lists all resource types that have resources in the tenant.
    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>>;

    /// Counts resources of a specific type.
    async fn count_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64>;

    /// Fetches a page of resources for reindexing.
    ///
    /// `cursor` is an opaque, backend-defined continuation token from the
    /// previous page; `None` starts from the beginning. The returned
    /// [`ResourcePage::next_cursor`] is `None` on the last page.
    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> StorageResult<ResourcePage>;
}

/// A backend that maintains a search index which can be rebuilt.
///
/// This is the *write* half of reindexing: where the extracted search-parameter
/// values go. A composite deployment has more than one — the SQL primary's
/// `search_index` table *and* the Elasticsearch index — which is why
/// [`ReindexOperation`] holds a `Vec` of these rather than a single storage
/// handle. Rebuilding only the primary's index on a deployment where
/// Elasticsearch serves search would leave search untouched by `$reindex`.
#[async_trait]
pub trait ReindexTarget: Send + Sync {
    /// Deletes this writer's search index entries for a single resource.
    ///
    /// Backends whose [`write_search_entries`](Self::write_search_entries) is a
    /// full replace (Elasticsearch, where the indexed document *is* the search
    /// entry) have nothing to do here and return `Ok(0)`.
    async fn delete_search_entries(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<u64>;

    /// Writes this writer's search index entries for a single resource, and
    /// returns how many entries were written.
    ///
    /// Takes the whole [`StoredResource`] rather than just the JSON body
    /// because Elasticsearch's indexed document carries `version_id` and
    /// `fhir_version`, neither of which is reliably recoverable from the raw
    /// resource JSON.
    async fn write_search_entries(
        &self,
        tenant: &TenantContext,
        resource: &StoredResource,
    ) -> StorageResult<usize>;

    /// Clears every search index entry this writer holds for a tenant, and
    /// returns how many were removed.
    ///
    /// Implementations MUST scope the deletion to `tenant` and nothing else.
    async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64>;
}

/// A backend that is both a [`ReindexSource`] and a [`ReindexTarget`] —
/// i.e. one that can reindex itself standalone (SQLite, PostgreSQL, MongoDB).
///
/// Blanket-implemented, so backends implement the two halves and get this for
/// free. It exists so `ReindexOperation::new` can take a single handle for the
/// common standalone case.
pub trait ReindexableStorage: ReindexSource + ReindexTarget {}

impl<T: ReindexSource + ReindexTarget> ReindexableStorage for T {}

/// Request to start a reindex operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReindexRequest {
    /// Target resource types (None = all types).
    pub resource_types: Option<Vec<String>>,

    /// Specific SearchParameter URLs to reindex (None = all active).
    pub search_param_urls: Option<Vec<String>>,

    /// Batch size for processing resources.
    #[serde(default = "default_batch_size")]
    pub batch_size: u32,

    /// Whether to clear existing indexes before reindexing.
    #[serde(default)]
    pub clear_existing: bool,
}

fn default_batch_size() -> u32 {
    100
}

impl Default for ReindexRequest {
    fn default() -> Self {
        Self {
            resource_types: None,
            search_param_urls: None,
            batch_size: default_batch_size(),
            clear_existing: false,
        }
    }
}

impl ReindexRequest {
    /// Creates a new reindex request for all resources.
    pub fn all() -> Self {
        Self::default()
    }

    /// Creates a reindex request for specific resource types.
    pub fn for_types<I, S>(types: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            resource_types: Some(types.into_iter().map(Into::into).collect()),
            ..Self::default()
        }
    }

    /// Creates a reindex request for specific parameters.
    pub fn for_params<I, S>(urls: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            search_param_urls: Some(urls.into_iter().map(Into::into).collect()),
            ..Self::default()
        }
    }

    /// Sets the batch size.
    pub fn with_batch_size(mut self, size: u32) -> Self {
        self.batch_size = size;
        self
    }

    /// Enables clearing existing indexes.
    pub fn clear_existing(mut self) -> Self {
        self.clear_existing = true;
        self
    }
}

/// Status of a reindex operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReindexStatus {
    /// Reindex is queued but not started.
    Queued,
    /// Reindex is currently running.
    InProgress,
    /// Reindex completed successfully.
    Completed,
    /// Reindex failed with an error.
    Failed,
    /// Reindex was cancelled.
    Cancelled,
}

impl ReindexStatus {
    /// Returns true if the job is still running.
    pub fn is_running(&self) -> bool {
        matches!(self, ReindexStatus::Queued | ReindexStatus::InProgress)
    }

    /// Returns true if the job has finished (success, failure, or cancelled).
    pub fn is_finished(&self) -> bool {
        matches!(
            self,
            ReindexStatus::Completed | ReindexStatus::Failed | ReindexStatus::Cancelled
        )
    }
}

/// Progress information for a reindex job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReindexProgress {
    /// Unique job identifier.
    pub job_id: String,

    /// Current status.
    pub status: ReindexStatus,

    /// Total number of resources to process.
    pub total_resources: u64,

    /// Number of resources processed so far.
    pub processed_resources: u64,

    /// Number of index entries created.
    pub entries_created: u64,

    /// Errors encountered during processing.
    pub errors: Vec<ReindexProgressError>,

    /// When the job was started.
    pub started_at: Option<String>,

    /// When the job completed.
    pub completed_at: Option<String>,

    /// Error message if status is Failed.
    pub error_message: Option<String>,

    /// Current resource type being processed.
    pub current_resource_type: Option<String>,
}

/// An error encountered during reindexing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReindexProgressError {
    /// Resource type.
    pub resource_type: String,
    /// Resource ID.
    pub resource_id: String,
    /// Error message.
    pub error: String,
}

impl ReindexProgress {
    /// Creates a new progress tracker for a job.
    pub fn new(job_id: impl Into<String>) -> Self {
        Self {
            job_id: job_id.into(),
            status: ReindexStatus::Queued,
            total_resources: 0,
            processed_resources: 0,
            entries_created: 0,
            errors: Vec::new(),
            started_at: None,
            completed_at: None,
            error_message: None,
            current_resource_type: None,
        }
    }

    /// Returns the progress percentage (0-100).
    pub fn percentage(&self) -> f64 {
        if self.total_resources == 0 {
            0.0
        } else {
            (self.processed_resources as f64 / self.total_resources as f64) * 100.0
        }
    }

    /// Returns true if any errors occurred.
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty() || self.error_message.is_some()
    }

    /// Converts to FHIR Parameters resource.
    pub fn to_parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "jobId", "valueString": self.job_id},
                {"name": "status", "valueCode": format!("{:?}", self.status).to_lowercase()},
                {"name": "total", "valueInteger": self.total_resources},
                {"name": "processed", "valueInteger": self.processed_resources},
                {"name": "entriesCreated", "valueInteger": self.entries_created},
                {"name": "errorCount", "valueInteger": self.errors.len()},
                {"name": "percentage", "valueDecimal": self.percentage()}
            ]
        })
    }
}

/// Where reindex lifecycle `AuditEvent`s are sent.
#[derive(Clone)]
struct ReindexAudit {
    sink: Arc<dyn helios_audit::AuditSink>,
    source_observer: String,
}

/// Manages reindex operations.
///
/// Deliberately **not** generic over a storage type. A composite deployment
/// reads resources from one backend (the primary) and must rewrite search
/// entries into several (the primary's index table *and* the Elasticsearch
/// index), so the source and the writers are separate, dynamically-dispatched
/// handles. A type parameter could only ever name one writer.
///
/// # Job state: per-process by default, shared with a cluster store
///
/// By default jobs live in an in-memory map, so `$reindex-status` for a job
/// started on one node is not visible from another. With
/// [`with_cluster_store`](Self::with_cluster_store) jobs live on the shared
/// cluster job store instead: any node answers status/cancel and any node's
/// [`run_next_cluster_job`](Self::run_next_cluster_job) worker may run the
/// rebuild.
pub struct ReindexOperation {
    /// Where resources are read from — the primary.
    source: Arc<dyn ReindexSource>,
    /// Every search index that must be rebuilt. More than one on a composite.
    writers: Vec<Arc<dyn ReindexTarget>>,
    /// The search parameter extractor.
    extractor: Arc<SearchParameterExtractor>,
    /// Active jobs.
    jobs: Arc<RwLock<HashMap<String, ReindexProgress>>>,
    /// Owning tenant of each in-memory job, for tenant-checked reads.
    job_tenants: Arc<RwLock<HashMap<String, String>>>,
    /// Cancellation channels.
    cancel_channels: Arc<RwLock<HashMap<String, mpsc::Sender<()>>>>,
    /// Optional audit sink for reindex lifecycle events.
    audit: Option<ReindexAudit>,
    /// When set, jobs live on the shared cluster job store instead of the
    /// in-memory map: any instance can poll/cancel, and any instance's
    /// [`run_next_cluster_job`](Self::run_next_cluster_job) worker can claim
    /// and run them (Phase 1, A2).
    store: Option<Arc<dyn ClusterJobStore>>,
}

impl ReindexOperation {
    /// Creates a reindex manager for a backend that indexes itself — the
    /// standalone case (SQLite, PostgreSQL, MongoDB), where the resources and
    /// the search index live in the same place.
    pub fn new<S: ReindexableStorage + 'static>(
        storage: Arc<S>,
        extractor: Arc<SearchParameterExtractor>,
    ) -> Self {
        Self::with_parts(storage.clone(), vec![storage], extractor)
    }

    /// Creates a reindex manager that reads from `source` and rebuilds every
    /// index in `writers`.
    ///
    /// This is the composite case: `source` is the primary (SQLite, PostgreSQL,
    /// MongoDB, or S3) and `writers` holds both the primary's own index — when
    /// it has one — and the Elasticsearch secondary that actually serves search.
    /// Passing only the primary here is the bug this split exists to prevent:
    /// `$reindex` would rebuild an index nothing queries.
    pub fn with_parts(
        source: Arc<dyn ReindexSource>,
        writers: Vec<Arc<dyn ReindexTarget>>,
        extractor: Arc<SearchParameterExtractor>,
    ) -> Self {
        Self {
            source,
            writers,
            extractor,
            jobs: Arc::new(RwLock::new(HashMap::new())),
            job_tenants: Arc::new(RwLock::new(HashMap::new())),
            cancel_channels: Arc::new(RwLock::new(HashMap::new())),
            audit: None,
            store: None,
        }
    }

    /// Moves job state onto the shared cluster job store (`HFS_JOB_STORE_BACKEND=database`):
    /// `start` enqueues instead of spawning, reads/cancel go to the store, and
    /// the work itself is claimed by per-instance workers
    /// ([`run_next_cluster_job`](Self::run_next_cluster_job)).
    pub fn with_cluster_store(mut self, store: Arc<dyn ClusterJobStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Whether jobs live on the shared cluster job store.
    pub fn is_cluster_backed(&self) -> bool {
        self.store.is_some()
    }

    /// Emits a BALP `AuditEvent` at each reindex lifecycle transition.
    pub fn with_audit(
        mut self,
        sink: Arc<dyn helios_audit::AuditSink>,
        source_observer: impl Into<String>,
    ) -> Self {
        self.audit = Some(ReindexAudit {
            sink,
            source_observer: source_observer.into(),
        });
        self
    }

    /// Returns true when this manager has at least one search index to rebuild.
    ///
    /// False for an S3-standalone deployment, which stores resources but has no
    /// search index of any kind; `$reindex` there has nothing to do and the
    /// REST layer reports that rather than pretending to have done work.
    pub fn has_writers(&self) -> bool {
        !self.writers.is_empty()
    }

    /// Starts a reindex operation.
    ///
    /// Returns immediately with a job ID; the reindex runs in the background.
    /// `agent` is the authenticated principal that requested it and is carried
    /// into the audit events — a destructive index rebuild that cannot be
    /// attributed to anyone is not much of an audit trail.
    pub async fn start(
        &self,
        tenant: TenantContext,
        request: ReindexRequest,
        agent: Option<String>,
    ) -> Result<String, ReindexError> {
        // Cluster mode: durably enqueue for any instance's worker; no local
        // state, no local spawn. The start audit event is still emitted here
        // (the terminal one comes from whichever worker runs the job).
        if let Some(store) = &self.store {
            let payload = serde_json::json!({
                "request": request,
                "agent": agent,
            });
            let job_id = store
                .enqueue(&tenant, JobKind::Reindex, payload)
                .await
                .map_err(|e| ReindexError::StorageError {
                    message: format!("failed to enqueue reindex job: {e}"),
                })?
                .to_string();

            if let Some(audit) = &self.audit {
                audit::record_reindex_event(
                    audit.sink.as_ref(),
                    &audit.source_observer,
                    agent.as_deref(),
                    &job_id,
                    "start",
                    &request.resource_types.clone().unwrap_or_default(),
                    0,
                    "0",
                )
                .await;
            }
            return Ok(job_id);
        }

        let job_id = Uuid::new_v4().to_string();
        let progress = ReindexProgress::new(&job_id);

        // Store the job (and its owning tenant, for tenant-checked reads)
        self.jobs.write().insert(job_id.clone(), progress);
        self.job_tenants
            .write()
            .insert(job_id.clone(), tenant.tenant_id().as_str().to_string());

        // Create cancellation channel
        let (cancel_tx, cancel_rx) = mpsc::channel::<()>(1);
        self.cancel_channels
            .write()
            .insert(job_id.clone(), cancel_tx);

        let requested_types = request.resource_types.clone().unwrap_or_default();

        // Emitted before the spawn so the start event can never be ordered
        // after a terminal event from the background task.
        if let Some(audit) = &self.audit {
            audit::record_reindex_event(
                audit.sink.as_ref(),
                &audit.source_observer,
                agent.as_deref(),
                &job_id,
                "start",
                &requested_types,
                0,
                "0",
            )
            .await;
        }

        // Clone references for the background task
        let source = self.source.clone();
        let writers = self.writers.clone();
        let extractor = self.extractor.clone();
        let jobs = self.jobs.clone();
        let audit = self.audit.clone();
        let job_id_clone = job_id.clone();

        // Spawn background task
        tokio::spawn(async move {
            run_reindex(
                job_id_clone.clone(),
                tenant,
                request,
                source,
                writers,
                extractor,
                jobs.clone(),
                cancel_rx,
            )
            .await;

            // Terminal audit event, read back from whatever state the run left
            // the job in.
            if let Some(audit) = audit {
                let (status, processed) = {
                    let guard = jobs.read();
                    let progress = guard.get(&job_id_clone);
                    (
                        progress.map(|p| p.status).unwrap_or(ReindexStatus::Failed),
                        progress.map(|p| p.processed_resources).unwrap_or(0),
                    )
                };
                let (phase, outcome) = match status {
                    ReindexStatus::Completed => ("complete", "0"),
                    ReindexStatus::Cancelled => ("cancel", "4"),
                    // Queued/InProgress are not reachable once the driver has
                    // returned; record them as failures rather than dropping
                    // the event entirely.
                    ReindexStatus::Failed | ReindexStatus::Queued | ReindexStatus::InProgress => {
                        ("fail", "8")
                    }
                };
                audit::record_reindex_event(
                    audit.sink.as_ref(),
                    &audit.source_observer,
                    agent.as_deref(),
                    &job_id_clone,
                    phase,
                    &requested_types,
                    processed,
                    outcome,
                )
                .await;
            }
        });

        Ok(job_id)
    }

    /// Gets the progress of a reindex job. Tenant-checked: a job belonging
    /// to another tenant is indistinguishable from a missing one.
    pub async fn get_progress(
        &self,
        tenant: &TenantContext,
        job_id: &str,
    ) -> Option<ReindexProgress> {
        if let Some(store) = &self.store {
            let record = store
                .get_status(tenant, &ClusterJobId::from_string(job_id))
                .await
                .ok()??;
            return Some(progress_from_record(&record));
        }
        if !self.owned_by(tenant, job_id) {
            return None;
        }
        self.jobs.read().get(job_id).cloned()
    }

    /// Cancels a running reindex job. Tenant-checked like
    /// [`get_progress`](Self::get_progress).
    pub async fn cancel(&self, tenant: &TenantContext, job_id: &str) -> Result<(), ReindexError> {
        if let Some(store) = &self.store {
            // Flips a queued/running job to cancelled at once (pollers on any
            // instance see it); the owning worker observes cancel_requested
            // and stops between batches.
            let found = store
                .cancel(tenant, &ClusterJobId::from_string(job_id))
                .await
                .map_err(|e| ReindexError::StorageError {
                    message: format!("failed to cancel reindex job: {e}"),
                })?;
            return if found {
                Ok(())
            } else {
                Err(ReindexError::JobNotFound {
                    job_id: job_id.to_string(),
                })
            };
        }

        // Check if job exists (for this tenant) and is running
        {
            if !self.owned_by(tenant, job_id) {
                return Err(ReindexError::JobNotFound {
                    job_id: job_id.to_string(),
                });
            }
            let jobs = self.jobs.read();
            let progress = jobs.get(job_id).ok_or_else(|| ReindexError::JobNotFound {
                job_id: job_id.to_string(),
            })?;

            if !progress.status.is_running() {
                return Ok(()); // Already finished
            }
        }

        // Send cancellation signal
        let tx = self.cancel_channels.read().get(job_id).cloned();
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }

        // Update status
        {
            let mut jobs = self.jobs.write();
            if let Some(progress) = jobs.get_mut(job_id) {
                progress.status = ReindexStatus::Cancelled;
                progress.completed_at = Some(chrono::Utc::now().to_rfc3339());
            }
        }

        Ok(())
    }

    /// Lists this tenant's jobs (active and recent).
    pub async fn list_jobs(&self, tenant: &TenantContext) -> Vec<ReindexProgress> {
        if let Some(store) = &self.store {
            return store
                .list_jobs(tenant, JobKind::Reindex)
                .await
                .map(|records| records.iter().map(progress_from_record).collect())
                .unwrap_or_default();
        }
        let tenants = self.job_tenants.read();
        self.jobs
            .read()
            .iter()
            .filter(|(id, _)| {
                tenants.get(*id).map(String::as_str) == Some(tenant.tenant_id().as_str())
            })
            .map(|(_, p)| p.clone())
            .collect()
    }

    /// True when the in-memory job exists and belongs to `tenant`.
    fn owned_by(&self, tenant: &TenantContext, job_id: &str) -> bool {
        self.job_tenants.read().get(job_id).map(String::as_str) == Some(tenant.tenant_id().as_str())
    }

    /// Claims at most one queued reindex job from the cluster store and
    /// drives it to a terminal state; returns whether a job was claimed.
    ///
    /// The worker pool's unit of work
    /// ([`spawn_reindex_cluster_workers`]), exposed so tests can execute a
    /// claim/run cycle deterministically. Errors only on claim-time storage
    /// trouble or when the operation is not cluster-backed.
    pub async fn run_next_cluster_job(
        &self,
        worker_id: &WorkerId,
        lease_duration: std::time::Duration,
    ) -> Result<bool, ReindexError> {
        let Some(store) = &self.store else {
            return Err(ReindexError::StorageError {
                message: "reindex is not cluster-backed (no job store configured)".to_string(),
            });
        };
        let claimed = store
            .claim_next(JobKind::Reindex, worker_id, lease_duration)
            .await
            .map_err(|e| ReindexError::StorageError {
                message: format!("failed to claim reindex job: {e}"),
            })?;
        let Some((lease, payload)) = claimed else {
            return Ok(false);
        };
        let jid = lease.job_id.to_string();

        // Renew the lease in the background for long-running rebuilds.
        let heartbeat = {
            let store = Arc::clone(store);
            let lease = lease.clone();
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(15));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    tick.tick().await;
                    if let Err(ClusterLeaseError::LeaseLost { .. }) = store.heartbeat(&lease).await
                    {
                        break;
                    }
                }
            })
        };

        let request: ReindexRequest = match payload
            .get("request")
            .cloned()
            .ok_or_else(|| "payload has no request".to_string())
            .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
        {
            Ok(request) => request,
            Err(e) => {
                heartbeat.abort();
                let msg = format!("invalid reindex payload: {e}");
                let _ = store.fail(&lease, &msg).await;
                return Ok(true);
            }
        };
        let agent: Option<String> = payload
            .get("agent")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        // `run_reindex` is shared with the in-memory mode and reports into a
        // jobs map + a local cancel channel; the bridge task below mirrors
        // that map into the store as progress snapshots and feeds
        // cross-instance cancellation (and lease loss) back into the channel.
        let local_jobs: Arc<RwLock<HashMap<String, ReindexProgress>>> =
            Arc::new(RwLock::new(HashMap::new()));
        local_jobs
            .write()
            .insert(jid.clone(), ReindexProgress::new(&jid));
        let (cancel_tx, cancel_rx) = mpsc::channel::<()>(1);

        let bridge = {
            let store = Arc::clone(store);
            let lease = lease.clone();
            let local_jobs = Arc::clone(&local_jobs);
            let jid = jid.clone();
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(std::time::Duration::from_millis(500));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    tick.tick().await;
                    let snapshot = local_jobs.read().get(&jid).cloned();
                    if let Some(progress) = snapshot
                        && let Ok(value) = serde_json::to_value(&progress)
                        && let Err(ClusterLeaseError::LeaseLost { .. }) =
                            store.update_progress(&lease, value).await
                    {
                        let _ = cancel_tx.send(()).await;
                        break;
                    }
                    match store.cancel_requested(&lease).await {
                        Ok(true) | Err(ClusterLeaseError::LeaseLost { .. }) => {
                            let _ = cancel_tx.send(()).await;
                            break;
                        }
                        _ => {}
                    }
                }
            })
        };

        run_reindex(
            jid.clone(),
            lease.tenant.clone(),
            request.clone(),
            self.source.clone(),
            self.writers.clone(),
            self.extractor.clone(),
            Arc::clone(&local_jobs),
            cancel_rx,
        )
        .await;
        bridge.abort();
        heartbeat.abort();

        let final_progress = local_jobs
            .read()
            .get(&jid)
            .cloned()
            .unwrap_or_else(|| ReindexProgress::new(&jid));
        let final_json = serde_json::to_value(&final_progress).unwrap_or(serde_json::Value::Null);
        let terminal = match final_progress.status {
            ReindexStatus::Completed => store.complete(&lease, final_json).await,
            // A local cancel is only ever bridge-induced: the store row is
            // already cancelled (or reclaimed), so there is nothing to write.
            ReindexStatus::Cancelled => Ok(()),
            _ => {
                let message = final_progress
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "reindex failed".to_string());
                // Best-effort final snapshot before the terminal transition.
                let _ = store.update_progress(&lease, final_json).await;
                store.fail(&lease, &message).await
            }
        };
        if let Err(ClusterLeaseError::Storage(e)) = terminal {
            tracing::warn!(job_id = %jid, error = %e, "failed to record reindex outcome");
        }

        // Terminal audit event, mirroring the in-memory background task.
        if let Some(audit) = &self.audit {
            let (phase, outcome) = match final_progress.status {
                ReindexStatus::Completed => ("complete", "0"),
                ReindexStatus::Cancelled => ("cancel", "4"),
                ReindexStatus::Failed | ReindexStatus::Queued | ReindexStatus::InProgress => {
                    ("fail", "8")
                }
            };
            audit::record_reindex_event(
                audit.sink.as_ref(),
                &audit.source_observer,
                agent.as_deref(),
                &jid,
                phase,
                &request.resource_types.unwrap_or_default(),
                final_progress.processed_resources,
                outcome,
            )
            .await;
        }

        Ok(true)
    }

    /// Removes completed jobs older than the specified duration.
    pub fn cleanup_old_jobs(&self, max_age_seconds: i64) {
        let cutoff = chrono::Utc::now() - chrono::Duration::seconds(max_age_seconds);

        let mut jobs = self.jobs.write();
        let mut channels = self.cancel_channels.write();

        jobs.retain(|job_id, progress| {
            if progress.status.is_finished()
                && let Some(ref completed_at) = progress.completed_at
                && let Ok(completed) = chrono::DateTime::parse_from_rfc3339(completed_at)
                && completed.with_timezone(&chrono::Utc) < cutoff
            {
                channels.remove(job_id);
                return false;
            }
            true
        });
    }
}

impl std::fmt::Debug for ReindexOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReindexOperation")
            .field("active_jobs", &self.jobs.read().len())
            .field("writers", &self.writers.len())
            .finish()
    }
}

/// Maps a cluster-job row onto the caller-facing [`ReindexProgress`].
///
/// The worker's snapshots (progress/result) carry the full progress document;
/// the row's state is authoritative for terminal transitions the worker never
/// wrote (cancel-before-claim, hard crash before the terminal write).
fn progress_from_record(record: &ClusterJobRecord) -> ReindexProgress {
    let mut progress: ReindexProgress = record
        .result
        .clone()
        .or_else(|| record.progress.clone())
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_else(|| ReindexProgress::new(record.id.as_str()));
    match record.state {
        ClusterJobState::Cancelled => {
            progress.status = ReindexStatus::Cancelled;
            if progress.completed_at.is_none() {
                progress.completed_at = record.finished_at.map(|t| t.to_rfc3339());
            }
        }
        ClusterJobState::Failed => {
            progress.status = ReindexStatus::Failed;
            if progress.error_message.is_none() {
                progress.error_message = record.error.clone();
            }
            if progress.completed_at.is_none() {
                progress.completed_at = record.finished_at.map(|t| t.to_rfc3339());
            }
        }
        ClusterJobState::Queued | ClusterJobState::Running | ClusterJobState::Completed => {}
    }
    progress
}

/// Spawns the per-instance reindex worker pool over the cluster store, plus a
/// reaper that ages out terminal job rows (the cluster twin of
/// [`ReindexOperation::cleanup_old_jobs`]).
///
/// # Panics
///
/// Panics when `op` is not cluster-backed — wiring error, same posture as the
/// other boot-time misconfigurations.
pub fn spawn_reindex_cluster_workers(op: Arc<ReindexOperation>, worker_count: usize) {
    const LEASE: std::time::Duration = std::time::Duration::from_secs(60);
    const TERMINAL_TTL_SECONDS: i64 = 24 * 60 * 60;

    let store = op
        .store
        .clone()
        .expect("spawn_reindex_cluster_workers requires a cluster-backed ReindexOperation");

    for n in 0..worker_count.max(1) {
        let op = Arc::clone(&op);
        let worker_id = WorkerId::new(format!("reindex-{n}-{}", Uuid::new_v4()));
        tokio::spawn(async move {
            tracing::info!(worker = %worker_id, "reindex worker started");
            loop {
                match op.run_next_cluster_job(&worker_id, LEASE).await {
                    Ok(true) => {}
                    Ok(false) => {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                    Err(e) => {
                        tracing::warn!(worker = %worker_id, error = %e, "reindex worker error");
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }
            }
        });
    }

    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let cutoff = chrono::Utc::now() - chrono::Duration::seconds(TERMINAL_TTL_SECONDS);
            if let Err(e) = store.delete_terminal_before(JobKind::Reindex, cutoff).await {
                tracing::warn!(error = %e, "reindex job reaper failed");
            }
        }
    });
}

/// Marks a job as failed.
fn mark_failed(jobs: &Arc<RwLock<HashMap<String, ReindexProgress>>>, job_id: &str, error: String) {
    let mut jobs_guard = jobs.write();
    if let Some(progress) = jobs_guard.get_mut(job_id) {
        progress.status = ReindexStatus::Failed;
        progress.error_message = Some(error);
        progress.completed_at = Some(chrono::Utc::now().to_rfc3339());
    }
}

/// Marks a job as cancelled.
fn mark_cancelled(jobs: &Arc<RwLock<HashMap<String, ReindexProgress>>>, job_id: &str) {
    let mut jobs_guard = jobs.write();
    if let Some(progress) = jobs_guard.get_mut(job_id) {
        progress.status = ReindexStatus::Cancelled;
        progress.completed_at = Some(chrono::Utc::now().to_rfc3339());
    }
}

/// Records a per-resource error against the job without aborting the run.
fn push_error(
    jobs: &Arc<RwLock<HashMap<String, ReindexProgress>>>,
    job_id: &str,
    resource_type: &str,
    resource_id: &str,
    error: String,
) {
    let mut jobs_guard = jobs.write();
    if let Some(progress) = jobs_guard.get_mut(job_id) {
        progress.errors.push(ReindexProgressError {
            resource_type: resource_type.to_string(),
            resource_id: resource_id.to_string(),
            error,
        });
    }
}

/// Drives a reindex job to completion in the background.
///
/// Reads resources from `source` and rewrites the search entries for each one
/// into **every** writer. On a composite deployment that means both the SQL
/// primary's index table and the Elasticsearch index — rebuilding only one of
/// them leaves search inconsistent with the resources.
#[allow(clippy::too_many_arguments)]
async fn run_reindex(
    job_id: String,
    tenant: TenantContext,
    request: ReindexRequest,
    source: Arc<dyn ReindexSource>,
    writers: Vec<Arc<dyn ReindexTarget>>,
    extractor: Arc<SearchParameterExtractor>,
    jobs: Arc<RwLock<HashMap<String, ReindexProgress>>>,
    mut cancel_rx: mpsc::Receiver<()>,
) {
    // Mark as started
    {
        let mut jobs_guard = jobs.write();
        if let Some(progress) = jobs_guard.get_mut(&job_id) {
            progress.status = ReindexStatus::InProgress;
            progress.started_at = Some(chrono::Utc::now().to_rfc3339());
        }
    }

    // Determine resource types to process
    let resource_types = match request.resource_types {
        Some(types) => types,
        None => match source.list_resource_types(&tenant).await {
            Ok(types) => types,
            Err(e) => {
                mark_failed(
                    &jobs,
                    &job_id,
                    format!("Failed to list resource types: {e}"),
                );
                return;
            }
        },
    };

    // Count total resources
    let mut total_resources: u64 = 0;
    for resource_type in &resource_types {
        match source.count_resources(&tenant, resource_type).await {
            Ok(count) => total_resources += count,
            Err(e) => {
                mark_failed(
                    &jobs,
                    &job_id,
                    format!("Failed to count {resource_type}: {e}"),
                );
                return;
            }
        }
    }

    // Update total
    {
        let mut jobs_guard = jobs.write();
        if let Some(progress) = jobs_guard.get_mut(&job_id) {
            progress.total_resources = total_resources;
        }
    }

    // Clear existing indexes if requested — in every writer, not just the first.
    if request.clear_existing {
        for writer in &writers {
            if let Err(e) = writer.clear_search_index(&tenant).await {
                mark_failed(&jobs, &job_id, format!("Failed to clear search index: {e}"));
                return;
            }
        }
    }

    // Process each resource type
    for resource_type in &resource_types {
        // Check for cancellation
        if cancel_rx.try_recv().is_ok() {
            mark_cancelled(&jobs, &job_id);
            return;
        }

        // Update current resource type
        {
            let mut jobs_guard = jobs.write();
            if let Some(progress) = jobs_guard.get_mut(&job_id) {
                progress.current_resource_type = Some(resource_type.clone());
            }
        }

        // Process resources in batches
        let mut cursor: Option<String> = None;
        loop {
            // Check for cancellation
            if cancel_rx.try_recv().is_ok() {
                mark_cancelled(&jobs, &job_id);
                return;
            }

            // Fetch a page of resources
            let page = match source
                .fetch_resources_page(
                    &tenant,
                    resource_type,
                    cursor.as_deref(),
                    request.batch_size,
                )
                .await
            {
                Ok(page) => page,
                Err(e) => {
                    mark_failed(&jobs, &job_id, format!("Failed to fetch resources: {e}"));
                    return;
                }
            };

            // Process each resource
            for resource in &page.resources {
                // How many entries this resource yields, for the progress
                // counter. A failure here means the resource cannot be indexed
                // at all, so skip it rather than half-writing it.
                let entry_count = match extractor.extract(resource.content(), resource_type) {
                    Ok(values) => values.len() as u64,
                    Err(e) => {
                        let mut jobs_guard = jobs.write();
                        if let Some(progress) = jobs_guard.get_mut(&job_id) {
                            progress.processed_resources += 1;
                        }
                        drop(jobs_guard);
                        push_error(
                            &jobs,
                            &job_id,
                            resource_type,
                            resource.id(),
                            format!("Extraction failed: {e}"),
                        );
                        continue;
                    }
                };

                let mut wrote_any = false;
                for writer in &writers {
                    if let Err(e) = writer
                        .delete_search_entries(&tenant, resource_type, resource.id())
                        .await
                    {
                        push_error(
                            &jobs,
                            &job_id,
                            resource_type,
                            resource.id(),
                            format!("Failed to delete index entries: {e}"),
                        );
                        continue;
                    }

                    match writer.write_search_entries(&tenant, resource).await {
                        Ok(_written) => wrote_any = true,
                        Err(e) => push_error(
                            &jobs,
                            &job_id,
                            resource_type,
                            resource.id(),
                            format!("Failed to write index entries: {e}"),
                        ),
                    }
                }

                let mut jobs_guard = jobs.write();
                if let Some(progress) = jobs_guard.get_mut(&job_id) {
                    progress.processed_resources += 1;
                    if wrote_any {
                        progress.entries_created += entry_count;
                    }
                }
            }

            // Check if there are more pages
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
    }

    // Mark as completed
    {
        let mut jobs_guard = jobs.write();
        if let Some(progress) = jobs_guard.get_mut(&job_id) {
            progress.status = ReindexStatus::Completed;
            progress.completed_at = Some(chrono::Utc::now().to_rfc3339());
            progress.current_resource_type = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reindex_request() {
        let req = ReindexRequest::for_types(vec!["Patient", "Observation"])
            .with_batch_size(50)
            .clear_existing();

        assert_eq!(req.resource_types.as_ref().unwrap().len(), 2);
        assert_eq!(req.batch_size, 50);
        assert!(req.clear_existing);
    }

    #[test]
    fn test_reindex_status() {
        assert!(ReindexStatus::InProgress.is_running());
        assert!(!ReindexStatus::Completed.is_running());
        assert!(ReindexStatus::Completed.is_finished());
        assert!(ReindexStatus::Failed.is_finished());
    }

    #[test]
    fn test_reindex_progress() {
        let mut progress = ReindexProgress::new("job-123");
        progress.total_resources = 100;
        progress.processed_resources = 50;

        assert_eq!(progress.percentage(), 50.0);
        assert!(!progress.has_errors());

        progress.errors.push(ReindexProgressError {
            resource_type: "Patient".to_string(),
            resource_id: "1".to_string(),
            error: "test error".to_string(),
        });

        assert!(progress.has_errors());
    }

    #[test]
    fn test_progress_to_parameters() {
        let progress = ReindexProgress::new("job-123");
        let params = progress.to_parameters();

        assert_eq!(params["resourceType"], "Parameters");
        assert!(params["parameter"].is_array());
    }
}
