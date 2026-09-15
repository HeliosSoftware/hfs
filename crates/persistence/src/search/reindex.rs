//! $reindex Operation Implementation.
//!
//! Provides the ability to rebuild search indexes for existing resources
//! when new SearchParameters are added or when indexes need to be repaired.

use std::collections::{BTreeSet, HashMap};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::FutureExt;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, Semaphore, mpsc, oneshot};
use uuid::Uuid;

use crate::core::DeferredReindexContext;
use crate::error::StorageResult;
use crate::tenant::TenantContext;
use crate::types::StoredResource;

use super::errors::ReindexError;

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

    /// Enters bulk index rebuild mode for a run that asked for it
    /// (`ReindexRequest::bulk_index_rebuild`): a writer that maintains
    /// secondary indexes row by row may drop them here and build them in
    /// [`Self::end_bulk_index_rebuild`], which the driver calls on every exit
    /// path of the run — completion, failure and cancellation alike. Nested
    /// runs are the writer's business (SQLite reference-counts). The default
    /// does nothing, which is right for a writer whose index *is* the
    /// document (Elasticsearch).
    async fn begin_bulk_index_rebuild(&self) -> StorageResult<()> {
        Ok(())
    }

    /// Leaves bulk index rebuild mode; see [`Self::begin_bulk_index_rebuild`].
    async fn end_bulk_index_rebuild(&self) -> StorageResult<()> {
        Ok(())
    }

    /// Rebuilds a whole page of resources — delete each one's stale entries,
    /// write fresh ones — returning a per-resource result of entries written,
    /// in input order.
    ///
    /// The default loops the per-resource methods and is what every writer
    /// got before pages existed. Backends whose per-resource writes each pay
    /// an autocommit (SQLite: 15–30 index rows *and* an FTS row per resource,
    /// each its own implicit transaction) override this to wrap the page in
    /// one transaction — the reindex-side counterpart of the #815 batch
    /// ingest, and what keeps the fast-load rebuild (#903) from giving back
    /// the throughput the deferred ingest won.
    async fn write_search_entries_page(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
    ) -> Vec<StorageResult<usize>> {
        let mut results = Vec::with_capacity(resources.len());
        for resource in resources {
            let deleted = self
                .delete_search_entries(tenant, resource.resource_type(), resource.id())
                .await;
            match deleted {
                Ok(_) => results.push(self.write_search_entries(tenant, resource).await),
                Err(e) => results.push(Err(e)),
            }
        }
        results
    }
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

    /// Bulk index rebuild: writers that support it drop their value indexes
    /// for the duration of the run and build them once, sorted, at the end
    /// (see [`ReindexTarget::begin_bulk_index_rebuild`]). Much faster for a
    /// large load; search on the affected store is unindexed meanwhile.
    #[serde(default)]
    pub bulk_index_rebuild: bool,
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
            bulk_index_rebuild: false,
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

    /// Sets the bulk index rebuild mode (see the field).
    pub fn with_bulk_index_rebuild(mut self, on: bool) -> Self {
        self.bulk_index_rebuild = on;
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

    /// The tenant the job rebuilds, so a per-tenant view (the dashboard's
    /// rebuild banner, #1065) can pick out its own jobs. `None` only on
    /// progress built without a tenant, such as one deserialized from before
    /// the field existed.
    #[serde(default)]
    pub tenant_id: Option<String>,

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
    /// Whether the failure was transient — the writer was unavailable, timed
    /// out, or asked to back off — so running the same work again may succeed.
    /// A permanent failure (a document the search backend rejects outright,
    /// such as one over Elasticsearch's nested-object limit, #1050) fails the
    /// same way on every rerun.
    #[serde(default = "retryable_by_default")]
    pub retryable: bool,
}

/// An error recorded without a classification keeps the retry it always had.
fn retryable_by_default() -> bool {
    true
}

/// Per-resource errors listed in [`ReindexProgress::to_parameters`]. The
/// status response stays bounded however many resources fail; `errorCount`
/// is always the full total.
const MAX_REPORTED_RESOURCE_ERRORS: usize = 100;

/// Failing `Type/id`s named in the log line of an automatic generation that
/// ended with permanent errors.
const MAX_LOGGED_RESOURCE_ERRORS: usize = 5;

/// Whether a writer's error is transient: the conditions the REST layer answers
/// with `503` or `504`, where the backend never judged the resource itself.
fn is_transient_error(error: &crate::error::StorageError) -> bool {
    use crate::error::{BackendError, StorageError};
    matches!(
        error,
        StorageError::Backend(
            BackendError::Unavailable { .. }
                | BackendError::ConnectionFailed { .. }
                | BackendError::PoolExhausted { .. }
                | BackendError::Timeout { .. }
        )
    )
}

impl ReindexProgress {
    /// Creates a new progress tracker for a job.
    pub fn new(job_id: impl Into<String>) -> Self {
        Self {
            job_id: job_id.into(),
            tenant_id: None,
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

    /// Returns true if the job recorded per-resource errors, every one of them
    /// permanent, and did not fail as a whole.
    pub fn has_only_permanent_errors(&self) -> bool {
        self.error_message.is_none()
            && !self.errors.is_empty()
            && self.errors.iter().all(|error| !error.retryable)
    }

    /// Converts to FHIR Parameters resource.
    ///
    /// Besides the counters, lists the job's failure message and the first
    /// [`MAX_REPORTED_RESOURCE_ERRORS`] failing resources, each with its error
    /// and whether it is retryable, so an operator can find the resources that
    /// are stored but not searchable without reading server logs.
    pub fn to_parameters(&self) -> serde_json::Value {
        let mut parameter = vec![
            serde_json::json!({"name": "jobId", "valueString": self.job_id}),
            serde_json::json!({"name": "status", "valueCode": format!("{:?}", self.status).to_lowercase()}),
            serde_json::json!({"name": "total", "valueInteger": self.total_resources}),
            serde_json::json!({"name": "processed", "valueInteger": self.processed_resources}),
            serde_json::json!({"name": "entriesCreated", "valueInteger": self.entries_created}),
            serde_json::json!({"name": "errorCount", "valueInteger": self.errors.len()}),
            serde_json::json!({"name": "percentage", "valueDecimal": self.percentage()}),
        ];
        if let Some(message) = &self.error_message {
            parameter.push(serde_json::json!({"name": "errorMessage", "valueString": message}));
        }
        for error in self.errors.iter().take(MAX_REPORTED_RESOURCE_ERRORS) {
            parameter.push(serde_json::json!({
                "name": "error",
                "part": [
                    {"name": "resourceType", "valueCode": error.resource_type},
                    {"name": "resourceId", "valueId": error.resource_id},
                    {"name": "message", "valueString": error.error},
                    {"name": "retryable", "valueBoolean": error.retryable}
                ]
            }));
        }
        if self.errors.len() > MAX_REPORTED_RESOURCE_ERRORS {
            parameter.push(serde_json::json!({
                "name": "errorsOmitted",
                "valueInteger": self.errors.len() - MAX_REPORTED_RESOURCE_ERRORS
            }));
        }
        serde_json::json!({"resourceType": "Parameters", "parameter": parameter})
    }
}

/// Where reindex lifecycle `AuditEvent`s are sent.
#[derive(Clone)]
struct ReindexAudit {
    sink: Arc<dyn helios_audit::AuditSink>,
    source_observer: String,
}

// Terminal status is a polling window, not a permanent in-memory job archive.
const REINDEX_STATUS_RETENTION_SECONDS: i64 = 24 * 60 * 60;
const MAX_RETAINED_REINDEX_STATUSES: usize = 1024;
const REINDEX_CLEANUP_INTERVAL: Duration = Duration::from_secs(60);
const DEFAULT_AUTOMATIC_REINDEX_CONCURRENCY: usize = 1;

type ReindexJobs = RwLock<HashMap<String, ReindexProgress>>;
type ReindexChannels = RwLock<HashMap<String, mpsc::Sender<()>>>;

#[derive(Clone)]
struct AutomaticReindexLimits {
    max_concurrency: usize,
    resident_tenants: Arc<Semaphore>,
    running_generations: Arc<Semaphore>,
}

impl AutomaticReindexLimits {
    fn new(max_concurrency: usize) -> Self {
        let max_concurrency = max_concurrency.clamp(1, Semaphore::MAX_PERMITS);
        let resident_capacity = max_concurrency
            .saturating_mul(2)
            .min(Semaphore::MAX_PERMITS);
        Self {
            max_concurrency,
            resident_tenants: Arc::new(Semaphore::new(resident_capacity)),
            running_generations: Arc::new(Semaphore::new(max_concurrency)),
        }
    }
}

/// Shape of the `ReindexRequest` an automatic generation starts with.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AutomaticRunOptions {
    /// Resources per rebuild transaction.
    pub(crate) batch_size: u32,
    /// `ReindexRequest::bulk_index_rebuild` for the runs the hook starts.
    pub(crate) bulk_index_rebuild: bool,
}

impl Default for AutomaticRunOptions {
    fn default() -> Self {
        Self {
            batch_size: DEFERRED_REINDEX_BATCH_SIZE,
            bulk_index_rebuild: false,
        }
    }
}

#[derive(Default)]
struct AutomaticTenantState {
    pending_types: BTreeSet<String>,
    next_generation: u64,
    consecutive_failures: u8,
    context: DeferredReindexContext,
    options: AutomaticRunOptions,
    waiting_for_generation: bool,
}

#[derive(Default)]
struct AutomaticReindexCoordinator {
    limits: OnceLock<AutomaticReindexLimits>,
    tenants: AsyncMutex<HashMap<String, AutomaticTenantState>>,
    tenant_changed: tokio::sync::Notify,
    #[cfg(test)]
    terminal_barrier: AsyncMutex<Option<Arc<AutomaticTerminalBarrier>>>,
}

#[cfg(test)]
struct AutomaticTerminalBarrier {
    removed: tokio::sync::Notify,
    resume: Semaphore,
}

struct ReindexTaskExit(Option<oneshot::Sender<()>>);

impl ReindexTaskExit {
    fn signal(mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for ReindexTaskExit {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

/// Only tasks that have exited are eligible, including after cancellation was
/// already reported to the client. All paths acquire jobs before channels and
/// release their guards before awaiting or calling external code.
fn cleanup_reindex_jobs(jobs: &ReindexJobs, channels: &ReindexChannels, max_age_seconds: i64) {
    let cutoff = chrono::Utc::now() - chrono::Duration::seconds(max_age_seconds);
    let mut jobs = jobs.write();
    let channels = channels.read();
    jobs.retain(|id, progress| {
        channels.contains_key(id)
            || !progress.status.is_finished()
            || progress
                .completed_at
                .as_deref()
                .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                .is_none_or(|at| at >= cutoff)
    });
    let eligible = jobs
        .iter()
        .filter(|(id, p)| p.status.is_finished() && !channels.contains_key(*id))
        .count();
    if eligible > MAX_RETAINED_REINDEX_STATUSES {
        let mut oldest: Vec<_> = jobs
            .iter()
            .filter(|(id, p)| p.status.is_finished() && !channels.contains_key(*id))
            .map(|(id, p)| {
                (
                    p.completed_at
                        .as_deref()
                        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                        .map(|at| at.timestamp_micros())
                        .unwrap_or(i64::MIN),
                    id.clone(),
                )
            })
            .collect();
        oldest.sort_unstable();
        for (_, id) in oldest
            .into_iter()
            .take(eligible - MAX_RETAINED_REINDEX_STATUSES)
        {
            jobs.remove(&id);
        }
    }
    // A large burst of active jobs must not leave empty buckets resident forever.
    if jobs.capacity()
        > jobs
            .len()
            .saturating_mul(2)
            .max(MAX_RETAINED_REINDEX_STATUSES)
    {
        jobs.shrink_to_fit();
    }
}

/// Lives from insertion (before kickoff audit awaits) until the spawned task is
/// dropped. Drop also runs when kickoff is aborted or the worker unwinds.
struct ReindexJobGuard {
    job_id: String,
    jobs: Arc<ReindexJobs>,
    channels: Arc<ReindexChannels>,
}

impl Drop for ReindexJobGuard {
    fn drop(&mut self) {
        {
            let mut jobs = self.jobs.write();
            let mut channels = self.channels.write();
            if let Some(progress) = jobs.get_mut(&self.job_id)
                && progress.status.is_running()
            {
                progress.status = ReindexStatus::Failed;
                progress.error_message = Some("Reindex task ended before completing".to_string());
                progress.completed_at = Some(chrono::Utc::now().to_rfc3339());
            }
            channels.remove(&self.job_id);
            if channels.capacity() > channels.len().saturating_mul(2).max(16) {
                channels.shrink_to_fit();
            }
        }
        cleanup_reindex_jobs(&self.jobs, &self.channels, REINDEX_STATUS_RETENTION_SECONDS);
    }
}

/// Manages reindex operations.
///
/// Deliberately **not** generic over a storage type. A composite deployment
/// reads resources from one backend (the primary) and must rewrite search
/// entries into several (the primary's index table *and* the Elasticsearch
/// index), so the source and the writers are separate, dynamically-dispatched
/// handles. A type parameter could only ever name one writer.
///
/// # Job state is per-process
///
/// Jobs live in an in-memory map, so `$reindex-status` for a job started on one
/// node is not visible from another. In a multi-node deployment, poll the node
/// that accepted the kick-off. Terminal states are retained for up to 24 hours,
/// subject to a limit of the most recent 1024 states whose tasks have exited.
/// Expiration is swept every minute; an evicted status is no longer available.
/// Active tasks (including cancellation still unwinding) are never evicted.
pub struct ReindexOperation {
    /// Where resources are read from — the primary.
    source: Arc<dyn ReindexSource>,
    /// Every search index that must be rebuilt. More than one on a composite.
    writers: Vec<Arc<dyn ReindexTarget>>,
    /// The per-tenant search parameter registries; the extractor is built from
    /// the reindexed tenant's registry so a tenant's stored params are honored.
    registries: Arc<crate::search::TenantSearchRegistries>,
    /// Active and recently finished jobs.
    jobs: Arc<RwLock<HashMap<String, ReindexProgress>>>,
    /// Cancellation channels.
    cancel_channels: Arc<RwLock<HashMap<String, mpsc::Sender<()>>>>,
    /// Coordinates only the automatic reindex requests emitted by bulk submit.
    automatic: Arc<AutomaticReindexCoordinator>,
    /// Lazily started by the first job, so construction needs no Tokio runtime.
    cleanup_started: AtomicBool,
    /// Optional audit sink for reindex lifecycle events.
    audit: Option<ReindexAudit>,
}

impl ReindexOperation {
    /// Creates a reindex manager for a backend that indexes itself — the
    /// standalone case (SQLite, PostgreSQL, MongoDB), where the resources and
    /// the search index live in the same place.
    pub fn new<S: ReindexableStorage + 'static>(
        storage: Arc<S>,
        registries: Arc<crate::search::TenantSearchRegistries>,
    ) -> Self {
        Self::with_parts(storage.clone(), vec![storage], registries)
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
        registries: Arc<crate::search::TenantSearchRegistries>,
    ) -> Self {
        Self {
            source,
            writers,
            registries,
            jobs: Arc::new(RwLock::new(HashMap::new())),
            cancel_channels: Arc::new(RwLock::new(HashMap::new())),
            automatic: Arc::new(AutomaticReindexCoordinator::default()),
            cleanup_started: AtomicBool::new(false),
            audit: None,
        }
    }

    fn ensure_cleanup_task(&self) {
        if self.cleanup_started.swap(true, Ordering::Relaxed) {
            return;
        }
        let jobs = Arc::downgrade(&self.jobs);
        let channels = Arc::downgrade(&self.cancel_channels);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(REINDEX_CLEANUP_INTERVAL).await;
                // Upgrades are dropped at the end of this iteration, before
                // sleeping; an idle cleanup task does not keep the maps alive.
                let (Some(jobs), Some(channels)) = (jobs.upgrade(), channels.upgrade()) else {
                    break;
                };
                cleanup_reindex_jobs(&jobs, &channels, REINDEX_STATUS_RETENTION_SECONDS);
            }
        });
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
        let (job_id, _task_exit) = self.start_tracked(tenant, request, agent).await?;
        Ok(job_id)
    }

    /// Starts a job and returns a signal sent after index work has stopped.
    ///
    /// The automatic bulk-submit coordinator uses the signal instead of the
    /// public terminal status. Cancellation marks that status before the task
    /// has necessarily returned, and terminal audit writes may block after the
    /// index scan itself is done.
    async fn start_tracked(
        &self,
        tenant: TenantContext,
        request: ReindexRequest,
        agent: Option<String>,
    ) -> Result<(String, oneshot::Receiver<()>), ReindexError> {
        self.ensure_cleanup_task();
        self.cleanup_old_jobs(REINDEX_STATUS_RETENTION_SECONDS);
        let job_id = Uuid::new_v4().to_string();
        let mut progress = ReindexProgress::new(&job_id);
        progress.tenant_id = Some(tenant.tenant_id().as_str().to_string());

        // Store the job
        self.jobs.write().insert(job_id.clone(), progress);

        // Create cancellation channel
        let (cancel_tx, cancel_rx) = mpsc::channel::<()>(1);
        self.cancel_channels
            .write()
            .insert(job_id.clone(), cancel_tx);

        let lifecycle = ReindexJobGuard {
            job_id: job_id.clone(),
            jobs: self.jobs.clone(),
            channels: self.cancel_channels.clone(),
        };

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
        let registries = self.registries.clone();
        let jobs = self.jobs.clone();
        let audit = self.audit.clone();
        let job_id_clone = job_id.clone();
        let (task_exit_tx, task_exit_rx) = oneshot::channel();

        // Spawn background task
        tokio::spawn(async move {
            // Declare this before the lifecycle guard. If the task is aborted,
            // the lifecycle guard marks the job failed before this signal wakes
            // the automatic coordinator.
            let task_exit = ReindexTaskExit(Some(task_exit_tx));
            let _lifecycle = lifecycle;
            let outcome = AssertUnwindSafe(run_reindex(
                job_id_clone.clone(),
                tenant,
                request,
                source,
                writers,
                registries,
                jobs.clone(),
                cancel_rx,
            ))
            .catch_unwind()
            .await;
            if outcome.is_err() {
                mark_failed(
                    &jobs,
                    &job_id_clone,
                    "Reindex task panicked before completing".to_string(),
                );
                tracing::error!(job_id = %job_id_clone, "reindex task panicked");
            }
            task_exit.signal();

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

        Ok((job_id, task_exit_rx))
    }

    /// Gets the progress of a reindex job.
    pub async fn get_progress(&self, job_id: &str) -> Option<ReindexProgress> {
        self.jobs.read().get(job_id).cloned()
    }

    /// Cancels a running reindex job.
    pub async fn cancel(&self, job_id: &str) -> Result<(), ReindexError> {
        // Check if job exists and is running
        {
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
            if let Some(progress) = jobs.get_mut(job_id)
                && progress.status.is_running()
            {
                progress.status = ReindexStatus::Cancelled;
                progress.completed_at = Some(chrono::Utc::now().to_rfc3339());
            }
        }

        Ok(())
    }

    /// Lists all jobs (active and recent).
    pub fn list_jobs(&self) -> Vec<ReindexProgress> {
        self.jobs.read().values().cloned().collect()
    }

    /// Removes terminal states older than the supplied age and limits retained
    /// terminal states to 1024. A task still owning its cancellation sender is
    /// protected even if cancellation has already marked its status terminal.
    pub fn cleanup_old_jobs(&self, max_age_seconds: i64) {
        cleanup_reindex_jobs(&self.jobs, &self.cancel_channels, max_age_seconds);
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

#[derive(Debug)]
enum AutomaticGenerationOutcome {
    Clean,
    Cancelled,
    /// The job completed, but every resource error it recorded is permanent.
    /// A rerun would be rejected the same way, so it is reported, not retried.
    PermanentErrors {
        count: usize,
        resources: String,
        first_error: String,
    },
    Failed(String),
}

/// Decides what an automatic generation's finished job means for retry policy.
fn automatic_outcome(progress: Option<ReindexProgress>) -> AutomaticGenerationOutcome {
    match progress {
        Some(progress) if progress.status == ReindexStatus::Completed && !progress.has_errors() => {
            AutomaticGenerationOutcome::Clean
        }
        Some(progress) if progress.status == ReindexStatus::Cancelled => {
            AutomaticGenerationOutcome::Cancelled
        }
        Some(progress)
            if progress.status == ReindexStatus::Completed
                && progress.has_only_permanent_errors() =>
        {
            AutomaticGenerationOutcome::PermanentErrors {
                count: progress.errors.len(),
                resources: progress
                    .errors
                    .iter()
                    .take(MAX_LOGGED_RESOURCE_ERRORS)
                    .map(|error| format!("{}/{}", error.resource_type, error.resource_id))
                    .collect::<Vec<_>>()
                    .join(", "),
                first_error: progress.errors[0].error.clone(),
            }
        }
        Some(progress) => AutomaticGenerationOutcome::Failed(format!(
            "status {:?}, {} resource errors{}",
            progress.status,
            progress.errors.len(),
            progress
                .error_message
                .as_deref()
                .map(|message| format!(", error: {message}"))
                .unwrap_or_default()
        )),
        None => {
            AutomaticGenerationOutcome::Failed("job status disappeared after task exit".to_string())
        }
    }
}

impl AutomaticReindexCoordinator {
    fn limits(&self, requested: usize) -> AutomaticReindexLimits {
        let requested = requested.max(1);
        let limits = self
            .limits
            .get_or_init(|| AutomaticReindexLimits::new(requested));
        if limits.max_concurrency != requested {
            tracing::warn!(
                configured = limits.max_concurrency,
                ignored = requested,
                "automatic reindex concurrency was already fixed for this operation"
            );
        }
        limits.clone()
    }

    async fn enqueue(
        self: Arc<Self>,
        op: Arc<ReindexOperation>,
        tenant: TenantContext,
        resource_types: Vec<String>,
        context: DeferredReindexContext,
        options: AutomaticRunOptions,
        max_concurrency: usize,
    ) {
        let requested_types: BTreeSet<_> = resource_types.into_iter().collect();
        if requested_types.is_empty() {
            return;
        }
        let tenant_id = tenant.tenant_id().to_string();
        let limits = self.limits(max_concurrency);

        // Same-tenant callbacks never wait for admission. They only add to the
        // one pending set already owned by that tenant's driver.
        {
            let mut tenants = self.tenants.lock().await;
            if let Some(state) = tenants.get_mut(&tenant_id) {
                state.pending_types.extend(requested_types);
                state.context = context.clone();
                state.options = options;
                tracing::info!(
                    tenant = %tenant_id,
                    submission = ?context.submission_id,
                    manifest = ?context.manifest_id,
                    types = ?state.pending_types,
                    "merged deferred reindex work into the pending generation"
                );
                return;
            }
        }

        // A waiter must also wake when another callback inserts this tenant.
        // Otherwise two simultaneous first callbacks can both observe an
        // absent entry and the loser can wait for admission until the winner's
        // whole scan has drained instead of merging into its pending set.
        let resident_permit = loop {
            let changed = self.tenant_changed.notified();
            {
                let mut tenants = self.tenants.lock().await;
                if let Some(state) = tenants.get_mut(&tenant_id) {
                    state.pending_types.extend(requested_types.clone());
                    state.context = context.clone();
                    state.options = options;
                    return;
                }
            }
            tokio::select! {
                biased;
                permit = limits.resident_tenants.clone().acquire_owned() => {
                    let Ok(permit) = permit else {
                        tracing::warn!(
                            tenant = %tenant_id,
                            "deferred reindex coordinator closed before admission; run $reindex manually"
                        );
                        return;
                    };
                    break permit;
                }
                _ = changed => {}
            }
        };

        // Another callback for the same tenant may have won admission while we
        // waited. Merge into it and release this redundant permit.
        {
            let mut tenants = self.tenants.lock().await;
            if let Some(state) = tenants.get_mut(&tenant_id) {
                state.pending_types.extend(requested_types);
                state.context = context;
                state.options = options;
                return;
            }
            tenants.insert(
                tenant_id.clone(),
                AutomaticTenantState {
                    pending_types: requested_types,
                    context,
                    options,
                    ..Default::default()
                },
            );
        }
        self.tenant_changed.notify_waiters();

        tokio::spawn(self.run_tenant(op, tenant, tenant_id, limits, resident_permit));
    }

    async fn run_tenant(
        self: Arc<Self>,
        op: Arc<ReindexOperation>,
        tenant: TenantContext,
        tenant_id: String,
        limits: AutomaticReindexLimits,
        _resident_permit: tokio::sync::OwnedSemaphorePermit,
    ) {
        loop {
            let (resource_types, generation, context, options) = {
                let mut tenants = self.tenants.lock().await;
                let Some(state) = tenants.get_mut(&tenant_id) else {
                    return;
                };
                if state.pending_types.is_empty() {
                    tenants.remove(&tenant_id);
                    return;
                }
                let types = std::mem::take(&mut state.pending_types)
                    .into_iter()
                    .collect::<Vec<_>>();
                let generation = state.next_generation;
                state.next_generation += 1;
                state.waiting_for_generation = true;
                (types, generation, state.context.clone(), state.options)
            };

            // Acquire for each generation, then release before a follow-up.
            // Tokio's fair semaphore lets another resident tenant run first.
            let Ok(running_permit) = limits.running_generations.clone().acquire_owned().await
            else {
                tracing::warn!(
                    tenant = %tenant_id,
                    generation,
                    "deferred reindex coordinator closed; run $reindex manually"
                );
                self.tenants.lock().await.remove(&tenant_id);
                return;
            };
            if let Some(state) = self.tenants.lock().await.get_mut(&tenant_id) {
                state.waiting_for_generation = false;
            }

            let started = op
                .start_tracked(
                    tenant.clone(),
                    ReindexRequest::for_types(resource_types.clone())
                        .with_batch_size(options.batch_size)
                        .with_bulk_index_rebuild(options.bulk_index_rebuild),
                    None,
                )
                .await;
            let (job_id, outcome) = match started {
                Ok((job_id, task_exit)) => {
                    tracing::info!(
                        tenant = %tenant_id,
                        generation,
                        job_id = %job_id,
                        submission = ?context.submission_id,
                        manifest = ?context.manifest_id,
                        types = ?resource_types,
                        "deferred reindex generation started"
                    );
                    let _ = task_exit.await;
                    let outcome = automatic_outcome(op.get_progress(&job_id).await);
                    (Some(job_id), outcome)
                }
                Err(error) => (
                    None,
                    AutomaticGenerationOutcome::Failed(format!(
                        "failed to start reindex job: {error}"
                    )),
                ),
            };
            drop(running_permit);

            let mut retry = false;
            let continue_driver = {
                let mut tenants = self.tenants.lock().await;
                let Some(state) = tenants.get_mut(&tenant_id) else {
                    return;
                };
                match &outcome {
                    AutomaticGenerationOutcome::Clean => {
                        state.consecutive_failures = 0;
                    }
                    AutomaticGenerationOutcome::Cancelled => {
                        state.consecutive_failures = 0;
                    }
                    AutomaticGenerationOutcome::PermanentErrors { .. } => {
                        state.consecutive_failures = 0;
                    }
                    AutomaticGenerationOutcome::Failed(_) => {
                        state.consecutive_failures += 1;
                        if state.consecutive_failures == 1 {
                            state.pending_types.extend(resource_types.iter().cloned());
                            retry = true;
                        } else {
                            // This generation exhausted its retry, but callbacks
                            // may have queued independent work while it ran.
                            // Give that later batch its own retry budget instead
                            // of removing the whole tenant entry below.
                            state.consecutive_failures = 0;
                        }
                    }
                }
                if state.pending_types.is_empty() {
                    tenants.remove(&tenant_id);
                    false
                } else {
                    true
                }
            };
            self.tenant_changed.notify_waiters();

            #[cfg(test)]
            if !continue_driver && let Some(barrier) = self.terminal_barrier.lock().await.take() {
                barrier.removed.notify_one();
                let permit = barrier
                    .resume
                    .acquire()
                    .await
                    .expect("terminal test barrier remains open");
                permit.forget();
            }

            match outcome {
                AutomaticGenerationOutcome::Clean => tracing::info!(
                    tenant = %tenant_id,
                    generation,
                    job_id = ?job_id,
                    types = ?resource_types,
                    "deferred reindex generation completed"
                ),
                AutomaticGenerationOutcome::Cancelled => tracing::warn!(
                    tenant = %tenant_id,
                    generation,
                    job_id = ?job_id,
                    types = ?resource_types,
                    "deferred reindex generation was cancelled"
                ),
                AutomaticGenerationOutcome::PermanentErrors {
                    count,
                    resources,
                    first_error,
                } => tracing::error!(
                    tenant = %tenant_id,
                    generation,
                    job_id = ?job_id,
                    types = ?resource_types,
                    errors = count,
                    resources = %resources,
                    first_error = %first_error,
                    "deferred reindex completed, but resources were rejected permanently and are stored but not searchable; not retrying because a rerun fails the same way (every failure is listed by $reindex-status for this job)"
                ),
                AutomaticGenerationOutcome::Failed(error) if retry => tracing::warn!(
                    tenant = %tenant_id,
                    generation,
                    job_id = ?job_id,
                    error = %error,
                    types = ?resource_types,
                    "deferred reindex generation failed; retrying once"
                ),
                AutomaticGenerationOutcome::Failed(error) => tracing::error!(
                    tenant = %tenant_id,
                    generation,
                    job_id = ?job_id,
                    error = %error,
                    types = ?resource_types,
                    "deferred reindex failed twice; run $reindex manually"
                ),
            }

            if !continue_driver {
                return;
            }
            tokio::task::yield_now().await;
        }
    }
}

/// Marks a job as failed.
fn mark_failed(jobs: &Arc<RwLock<HashMap<String, ReindexProgress>>>, job_id: &str, error: String) {
    let mut jobs_guard = jobs.write();
    if let Some(progress) = jobs_guard.get_mut(job_id)
        && progress.status.is_running()
    {
        progress.status = ReindexStatus::Failed;
        progress.error_message = Some(error);
        progress.completed_at = Some(chrono::Utc::now().to_rfc3339());
    }
}

/// How the page loop of a run ended, short of completion.
enum RunExit {
    Cancelled,
    Failed(String),
}

/// Marks a job as cancelled.
fn mark_cancelled(jobs: &Arc<RwLock<HashMap<String, ReindexProgress>>>, job_id: &str) {
    let mut jobs_guard = jobs.write();
    if let Some(progress) = jobs_guard.get_mut(job_id)
        && progress.status.is_running()
    {
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
    retryable: bool,
) {
    let mut jobs_guard = jobs.write();
    if let Some(progress) = jobs_guard.get_mut(job_id) {
        progress.errors.push(ReindexProgressError {
            resource_type: resource_type.to_string(),
            resource_id: resource_id.to_string(),
            error,
            retryable,
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
    registries: Arc<crate::search::TenantSearchRegistries>,
    jobs: Arc<RwLock<HashMap<String, ReindexProgress>>>,
    mut cancel_rx: mpsc::Receiver<()>,
) {
    let perf_run = crate::perf::enabled().then(|| (Instant::now(), crate::perf::snapshot()));
    // The writers extract with their own tenant registries; the driver no
    // longer runs a second extraction just to count entries — the per-page
    // outcomes report what was actually written.
    let _ = registries;
    // Mark as started — unless the job already reached a terminal state before
    // this task was first polled.
    //
    // `ReindexOperation::cancel` writes `Cancelled` synchronously, but this
    // task is spawned by `start` and may not run until after that write.
    // Unconditionally stamping `InProgress` here would then resurrect a job the
    // caller has already been told is finished, and leave it reporting
    // "in progress" all the way to the first cancellation check further down —
    // past `list_resource_types`, every `count_resources`, and the optional
    // index clear. Bailing out instead is both the honest status and a faster
    // cancellation: the work below is pointless for a cancelled job.
    {
        let mut jobs_guard = jobs.write();
        match jobs_guard.get_mut(&job_id) {
            Some(progress) if progress.status.is_finished() => return,
            Some(progress) => {
                progress.status = ReindexStatus::InProgress;
                progress.started_at = Some(chrono::Utc::now().to_rfc3339());
            }
            None => {}
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

    // Bulk index rebuild (opt-in): every writer drops what it can before the
    // pages, and gets its `end` on every exit path below.
    if request.bulk_index_rebuild {
        for writer in &writers {
            if let Err(e) = writer.begin_bulk_index_rebuild().await {
                mark_failed(
                    &jobs,
                    &job_id,
                    format!("Failed to enter bulk index rebuild: {e}"),
                );
                return;
            }
        }
    }

    let outcome: Result<(), RunExit> = async {
        // Process each resource type
        for resource_type in &resource_types {
            // Check for cancellation
            if cancel_rx.try_recv().is_ok() {
                return Err(RunExit::Cancelled);
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
                    return Err(RunExit::Cancelled);
                }

                // Fetch a page of resources
                let fetch_span = crate::perf::span(crate::perf::Phase::ReindexFetch);
                let fetched = source
                    .fetch_resources_page(
                        &tenant,
                        resource_type,
                        cursor.as_deref(),
                        request.batch_size,
                    )
                    .await;
                drop(fetch_span);
                let page = match fetched {
                    Ok(page) => page,
                    Err(e) => {
                        return Err(RunExit::Failed(format!("Failed to fetch resources: {e}")));
                    }
                };

                // Rebuild the page through every writer. Page-at-a-time so a
                // writer can wrap it in one transaction; each writer reports a
                // per-resource outcome for error attribution. The entry count for
                // progress comes from the writers' own extraction — the driver no
                // longer extracts a second time just to count.
                let mut wrote_any: Vec<bool> = vec![false; page.resources.len()];
                let mut entry_counts: Vec<u64> = vec![0; page.resources.len()];
                for writer in &writers {
                    let outcomes = writer
                        .write_search_entries_page(&tenant, &page.resources)
                        .await;
                    for (i, outcome) in outcomes.into_iter().enumerate() {
                        match outcome {
                            Ok(written) => {
                                wrote_any[i] = true;
                                entry_counts[i] = entry_counts[i].max(written as u64);
                            }
                            Err(e) => push_error(
                                &jobs,
                                &job_id,
                                resource_type,
                                page.resources[i].id(),
                                format!("Failed to rebuild index entries: {e}"),
                                is_transient_error(&e),
                            ),
                        }
                    }
                }

                for (i, _resource) in page.resources.iter().enumerate() {
                    let mut jobs_guard = jobs.write();
                    if let Some(progress) = jobs_guard.get_mut(&job_id) {
                        progress.processed_resources += 1;
                        if wrote_any[i] {
                            progress.entries_created += entry_counts[i];
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

        Ok(())
    }
    .await;

    if request.bulk_index_rebuild {
        for writer in &writers {
            if let Err(e) = writer.end_bulk_index_rebuild().await {
                mark_failed(
                    &jobs,
                    &job_id,
                    format!("Failed to leave bulk index rebuild: {e}"),
                );
                return;
            }
        }
    }

    match outcome {
        Err(RunExit::Cancelled) => return mark_cancelled(&jobs, &job_id),
        Err(RunExit::Failed(msg)) => return mark_failed(&jobs, &job_id, msg),
        Ok(()) => {}
    }

    if let Some((started, before)) = perf_run {
        let processed = jobs
            .read()
            .get(&job_id)
            .map(|progress| progress.processed_resources)
            .unwrap_or(0);
        let report =
            crate::perf::report_since(&before, processed, started.elapsed()).replace('\n', " | ");
        tracing::info!(
            target: "hfs_perf",
            job_id = %job_id,
            resources = processed,
            process_global = true,
            single_job_required = true,
            phases = %report,
            "reindex phase summary"
        );
    }

    // Mark as completed
    {
        let mut jobs_guard = jobs.write();
        if let Some(progress) = jobs_guard.get_mut(&job_id)
            && progress.status.is_running()
        {
            progress.status = ReindexStatus::Completed;
            progress.completed_at = Some(chrono::Utc::now().to_rfc3339());
            progress.current_resource_type = None;
        }
    }
}

/// [`crate::core::DeferredReindexHook`] adapter over [`ReindexOperation`],
/// for the bulk fast-load path (#903): the submit worker fires it after a
/// manifest that ingested with deferred indexing, and it starts the same
/// background reindex `POST /$reindex` would, restricted to the manifest's
/// types.
#[derive(Clone)]
pub struct ReindexOnFinish {
    op: std::sync::Arc<ReindexOperation>,
    max_concurrency: usize,
    /// Request shape for the generations this hook enqueues.
    options: AutomaticRunOptions,
}

/// The page the deferred rebuild uses. `ReindexRequest`'s default of 100
/// suits an operator-driven `$reindex` on a live server, where a small
/// transaction keeps the write lock short. The rebuild after a fast-load is
/// a different workload: it is the bulk of the import's wall clock, and
/// each page is one COMMIT, whose cost is fixed per transaction rather than
/// per row. Measured on the SQLite backend, 100 -> 1000 took the rebuild
/// from 1,695 to 1,926 resources/s; 5,000 gained a further 2% and holds
/// five times the resources in memory per page, so 1,000 it is.
pub const DEFERRED_REINDEX_BATCH_SIZE: u32 = 1000;

impl ReindexOnFinish {
    /// Wraps a reindex manager with one automatic generation at a time.
    pub fn new(op: std::sync::Arc<ReindexOperation>) -> Self {
        Self::with_max_concurrency(op, DEFAULT_AUTOMATIC_REINDEX_CONCURRENCY)
    }

    /// Wraps a reindex manager and limits automatic generations to
    /// `max_concurrency`. Resident tenant state is limited to twice that value,
    /// so queued tenants can compete with follow-up generations.
    pub fn with_max_concurrency(
        op: std::sync::Arc<ReindexOperation>,
        max_concurrency: usize,
    ) -> Self {
        Self {
            op,
            max_concurrency: max_concurrency.clamp(1, Semaphore::MAX_PERMITS),
            options: AutomaticRunOptions::default(),
        }
    }

    /// Overrides the resources-per-transaction page of the rebuild.
    pub fn with_batch_size(mut self, batch_size: u32) -> Self {
        self.options.batch_size = batch_size.max(1);
        self
    }

    /// Runs the rebuild in bulk index rebuild mode
    /// (`HFS_BULK_SUBMIT_BULK_INDEX_REBUILD`): the writer's value indexes are
    /// dropped for the duration and built once, sorted, at the end.
    pub fn with_bulk_index_rebuild(mut self, on: bool) -> Self {
        self.options.bulk_index_rebuild = on;
        self
    }

    async fn enqueue(
        &self,
        tenant: &crate::tenant::TenantContext,
        resource_types: Vec<String>,
        context: DeferredReindexContext,
    ) {
        self.op
            .automatic
            .clone()
            .enqueue(
                self.op.clone(),
                tenant.clone(),
                resource_types,
                context,
                self.options,
                self.max_concurrency,
            )
            .await;
    }
}

#[async_trait::async_trait]
impl crate::core::DeferredReindexHook for ReindexOnFinish {
    async fn reindex_types(
        &self,
        tenant: &crate::tenant::TenantContext,
        resource_types: Vec<String>,
    ) {
        self.enqueue(tenant, resource_types, DeferredReindexContext::default())
            .await;
    }

    async fn reindex_types_with_context(
        &self,
        tenant: &crate::tenant::TenantContext,
        resource_types: Vec<String>,
        context: DeferredReindexContext,
    ) {
        self.enqueue(tenant, resource_types, context).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::DeferredReindexHook;

    #[derive(Default)]
    struct LifecycleSource {
        blocked: bool,
        fail: bool,
        panic: bool,
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }

    #[async_trait]
    impl ReindexSource for LifecycleSource {
        async fn list_resource_types(&self, _: &TenantContext) -> StorageResult<Vec<String>> {
            self.entered.notify_one();
            if self.blocked {
                self.release.notified().await;
            }
            assert!(!self.panic, "injected reindex task panic");
            if self.fail {
                return Err(crate::error::BackendError::Unavailable {
                    backend_name: "test".into(),
                    message: "injected failure".into(),
                }
                .into());
            }
            Ok(Vec::new())
        }
        async fn count_resources(&self, _: &TenantContext, _: &str) -> StorageResult<u64> {
            unreachable!("empty source has no types")
        }
        async fn fetch_resources_page(
            &self,
            _: &TenantContext,
            _: &str,
            _: Option<&str>,
            _: u32,
        ) -> StorageResult<ResourcePage> {
            unreachable!("empty source has no pages")
        }
    }

    #[derive(Clone, Copy)]
    enum CountBehavior {
        Clean,
        Fail,
        Panic,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum ControlledEvent {
        Count {
            tenant: String,
            resource_type: String,
        },
        Write {
            tenant: String,
            resource_type: String,
        },
    }

    struct ControlledBackend {
        events: tokio::sync::mpsc::UnboundedSender<ControlledEvent>,
        write_gate: Arc<Semaphore>,
        count_behaviors: parking_lot::Mutex<std::collections::VecDeque<CountBehavior>>,
        count_calls: std::sync::atomic::AtomicUsize,
        write_calls: std::sync::atomic::AtomicUsize,
        active_writes: std::sync::atomic::AtomicUsize,
        max_active_writes: std::sync::atomic::AtomicUsize,
        failing_writes: std::sync::atomic::AtomicUsize,
        permanent_failing_writes: std::sync::atomic::AtomicUsize,
    }

    impl ControlledBackend {
        fn new(
            count_behaviors: Vec<CountBehavior>,
            failing_writes: usize,
        ) -> (
            Arc<Self>,
            tokio::sync::mpsc::UnboundedReceiver<ControlledEvent>,
        ) {
            let (events, receiver) = tokio::sync::mpsc::unbounded_channel();
            (
                Arc::new(Self {
                    events,
                    write_gate: Arc::new(Semaphore::new(0)),
                    count_behaviors: parking_lot::Mutex::new(count_behaviors.into()),
                    count_calls: std::sync::atomic::AtomicUsize::new(0),
                    write_calls: std::sync::atomic::AtomicUsize::new(0),
                    active_writes: std::sync::atomic::AtomicUsize::new(0),
                    max_active_writes: std::sync::atomic::AtomicUsize::new(0),
                    failing_writes: std::sync::atomic::AtomicUsize::new(failing_writes),
                    permanent_failing_writes: std::sync::atomic::AtomicUsize::new(0),
                }),
                receiver,
            )
        }

        fn record_active_write(&self) {
            let active = self.active_writes.fetch_add(1, Ordering::SeqCst) + 1;
            let mut observed = self.max_active_writes.load(Ordering::SeqCst);
            while active > observed {
                match self.max_active_writes.compare_exchange_weak(
                    observed,
                    active,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(current) => observed = current,
                }
            }
        }
    }

    #[async_trait]
    impl ReindexSource for ControlledBackend {
        async fn list_resource_types(&self, _: &TenantContext) -> StorageResult<Vec<String>> {
            Ok(vec!["Patient".to_string()])
        }

        async fn count_resources(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
        ) -> StorageResult<u64> {
            self.count_calls.fetch_add(1, Ordering::SeqCst);
            let _ = self.events.send(ControlledEvent::Count {
                tenant: tenant.tenant_id().to_string(),
                resource_type: resource_type.to_string(),
            });
            match self
                .count_behaviors
                .lock()
                .pop_front()
                .unwrap_or(CountBehavior::Clean)
            {
                CountBehavior::Clean => Ok(1),
                CountBehavior::Fail => Err(crate::error::BackendError::Unavailable {
                    backend_name: "controlled".into(),
                    message: "injected count failure".into(),
                }
                .into()),
                CountBehavior::Panic => panic!("injected count panic"),
            }
        }

        async fn fetch_resources_page(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            _: Option<&str>,
            _: u32,
        ) -> StorageResult<ResourcePage> {
            Ok(ResourcePage {
                resources: vec![StoredResource::new(
                    resource_type,
                    "controlled-1",
                    tenant.tenant_id().clone(),
                    serde_json::json!({"resourceType": resource_type, "id": "controlled-1"}),
                    helios_fhir::FhirVersion::default(),
                )],
                next_cursor: None,
            })
        }
    }

    #[async_trait]
    impl ReindexTarget for ControlledBackend {
        async fn delete_search_entries(
            &self,
            _: &TenantContext,
            _: &str,
            _: &str,
        ) -> StorageResult<u64> {
            Ok(0)
        }

        // `fetch_update` was deprecated in Rust 1.98 in favour of
        // `try_update`, but the new name is still unstable
        // (`atomic_try_update`) on the workspace's 1.90 MSRV, so the old
        // one stays until the MSRV catches up.
        #[allow(deprecated)]
        async fn write_search_entries(
            &self,
            tenant: &TenantContext,
            resource: &StoredResource,
        ) -> StorageResult<usize> {
            self.write_calls.fetch_add(1, Ordering::SeqCst);
            self.record_active_write();
            let _ = self.events.send(ControlledEvent::Write {
                tenant: tenant.tenant_id().to_string(),
                resource_type: resource.resource_type().to_string(),
            });
            let permit = self
                .write_gate
                .acquire()
                .await
                .expect("controlled write gate remains open");
            permit.forget();
            self.active_writes.fetch_sub(1, Ordering::SeqCst);
            if self
                .failing_writes
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(crate::error::BackendError::Unavailable {
                    backend_name: "controlled".into(),
                    message: "injected write failure".into(),
                }
                .into());
            }
            if self
                .permanent_failing_writes
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(crate::error::BackendError::Internal {
                    backend_name: "controlled".into(),
                    message: "injected permanent write failure".into(),
                    source: None,
                }
                .into());
            }
            Ok(1)
        }

        async fn clear_search_index(&self, _: &TenantContext) -> StorageResult<u64> {
            Ok(0)
        }
    }

    fn controlled_operation(backend: Arc<ControlledBackend>) -> Arc<ReindexOperation> {
        Arc::new(ReindexOperation::new(
            backend,
            Arc::new(crate::search::TenantSearchRegistries::base_only()),
        ))
    }

    fn named_tenant(name: &str) -> TenantContext {
        TenantContext::new(
            crate::tenant::TenantId::new(name),
            crate::tenant::TenantPermissions::full_access(),
        )
    }

    async fn next_controlled_event(
        events: &mut tokio::sync::mpsc::UnboundedReceiver<ControlledEvent>,
    ) -> ControlledEvent {
        tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .expect("controlled reindex emitted no event")
            .expect("controlled event channel closed")
    }

    async fn await_automatic_idle(op: &ReindexOperation) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if op.automatic.tenants.lock().await.is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("automatic reindex coordinator did not become idle");
    }

    async fn await_tenant_waiting(op: &ReindexOperation, tenant: &str) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if op
                    .automatic
                    .tenants
                    .lock()
                    .await
                    .get(tenant)
                    .is_some_and(|state| state.waiting_for_generation)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("tenant did not wait for a generation permit");
    }

    async fn await_controlled_write(
        events: &mut tokio::sync::mpsc::UnboundedReceiver<ControlledEvent>,
        tenant: &str,
        resource_type: &str,
    ) {
        assert_eq!(
            next_controlled_event(events).await,
            ControlledEvent::Count {
                tenant: tenant.to_string(),
                resource_type: resource_type.to_string(),
            }
        );
        assert_eq!(
            next_controlled_event(events).await,
            ControlledEvent::Write {
                tenant: tenant.to_string(),
                resource_type: resource_type.to_string(),
            }
        );
    }

    fn lifecycle_operation(source: Arc<LifecycleSource>) -> ReindexOperation {
        ReindexOperation::with_parts(
            source,
            Vec::new(),
            Arc::new(crate::search::TenantSearchRegistries::base_only()),
        )
    }

    async fn start_lifecycle_job(op: &ReindexOperation) -> String {
        op.start(TenantContext::system(), ReindexRequest::default(), None)
            .await
            .unwrap()
    }

    async fn await_terminal(op: &ReindexOperation, id: &str) -> ReindexProgress {
        for _ in 0..100 {
            tokio::task::yield_now().await;
            if let Some(progress) = op
                .jobs
                .read()
                .get(id)
                .filter(|p| p.status.is_finished())
                .cloned()
            {
                return progress;
            }
        }
        panic!("job never reached terminal status");
    }

    /// One type whose page fetch fails, so the run leaves through the
    /// failure path of the page loop.
    struct FailingPageSource;

    #[async_trait]
    impl ReindexSource for FailingPageSource {
        async fn list_resource_types(&self, _: &TenantContext) -> StorageResult<Vec<String>> {
            Ok(vec!["Patient".to_string()])
        }
        async fn count_resources(&self, _: &TenantContext, _: &str) -> StorageResult<u64> {
            Ok(1)
        }
        async fn fetch_resources_page(
            &self,
            _: &TenantContext,
            _: &str,
            _: Option<&str>,
            _: u32,
        ) -> StorageResult<ResourcePage> {
            Err(crate::error::BackendError::Unavailable {
                backend_name: "test".into(),
                message: "injected page failure".into(),
            }
            .into())
        }
    }

    /// Counts the bulk-rebuild transitions it is asked for.
    #[derive(Default)]
    struct CountingTarget {
        begun: std::sync::atomic::AtomicUsize,
        ended: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl ReindexTarget for CountingTarget {
        async fn delete_search_entries(
            &self,
            _: &TenantContext,
            _: &str,
            _: &str,
        ) -> StorageResult<u64> {
            Ok(0)
        }
        async fn write_search_entries(
            &self,
            _: &TenantContext,
            _: &StoredResource,
        ) -> StorageResult<usize> {
            Ok(0)
        }
        async fn clear_search_index(&self, _: &TenantContext) -> StorageResult<u64> {
            Ok(0)
        }
        async fn begin_bulk_index_rebuild(&self) -> StorageResult<()> {
            self.begun.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        async fn end_bulk_index_rebuild(&self) -> StorageResult<()> {
            self.ended.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    /// A run that fails inside its page loop still leaves bulk index rebuild
    /// mode: the writer would otherwise be left without its indexes until the
    /// next process start.
    #[tokio::test]
    async fn bulk_index_rebuild_ends_on_the_failure_path() {
        let target = Arc::new(CountingTarget::default());
        let op = ReindexOperation::with_parts(
            Arc::new(FailingPageSource),
            vec![target.clone()],
            Arc::new(crate::search::TenantSearchRegistries::base_only()),
        );
        let id = op
            .start(
                TenantContext::system(),
                ReindexRequest::default().with_bulk_index_rebuild(true),
                None,
            )
            .await
            .unwrap();
        let progress = await_terminal(&op, &id).await;
        assert_eq!(progress.status, ReindexStatus::Failed);
        assert_eq!(target.begun.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(target.ended.load(std::sync::atomic::Ordering::SeqCst), 1);

        // And a run without the flag never touches the hooks.
        let id = op
            .start(TenantContext::system(), ReindexRequest::default(), None)
            .await
            .unwrap();
        await_terminal(&op, &id).await;
        assert_eq!(target.begun.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(target.ended.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retention_completed_jobs_release_channels_and_keep_status() {
        let op = lifecycle_operation(Arc::new(LifecycleSource::default()));
        for _ in 0..3 {
            let id = start_lifecycle_job(&op).await;
            assert_eq!(
                await_terminal(&op, &id).await.status,
                ReindexStatus::Completed
            );
            assert!(
                !op.cancel_channels.read().contains_key(&id),
                "finished job retained sender"
            );
            assert_eq!(
                op.get_progress(&id).await.unwrap().status,
                ReindexStatus::Completed
            );
            op.cancel(&id).await.unwrap(); // Terminal cancellation remains idempotent.
            assert_eq!(
                op.get_progress(&id).await.unwrap().status,
                ReindexStatus::Completed
            );
        }
        assert_eq!(op.list_jobs().len(), 3);
    }

    #[tokio::test]
    async fn test_retention_failed_job_releases_channel() {
        let op = lifecycle_operation(Arc::new(LifecycleSource {
            fail: true,
            ..Default::default()
        }));
        let id = start_lifecycle_job(&op).await;
        let progress = await_terminal(&op, &id).await;
        assert_eq!(progress.status, ReindexStatus::Failed);
        assert!(
            progress
                .error_message
                .unwrap()
                .contains("Failed to list resource types")
        );
        assert!(op.cancel_channels.read().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn test_retention_cancelled_task_is_protected_until_it_returns() {
        let source = Arc::new(LifecycleSource {
            blocked: true,
            ..Default::default()
        });
        let op = lifecycle_operation(source.clone());
        let id = start_lifecycle_job(&op).await;
        source.entered.notified().await;
        op.cancel(&id).await.unwrap();
        op.jobs.write().get_mut(&id).unwrap().completed_at =
            Some((chrono::Utc::now() - chrono::Duration::hours(25)).to_rfc3339());
        op.cleanup_old_jobs(0);
        assert!(
            op.jobs.read().contains_key(&id),
            "cleanup evicted a task still using its status"
        );
        assert!(op.cancel_channels.read().contains_key(&id));
        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        assert!(
            op.jobs.read().contains_key(&id),
            "periodic cleanup evicted a live cancelled task"
        );
        // Restore a recent timestamp before allowing the task to return, so
        // the next assertion tests cancellation state rather than expiry.
        op.jobs.write().get_mut(&id).unwrap().completed_at = Some(chrono::Utc::now().to_rfc3339());
        source.release.notify_one();
        for _ in 0..100 {
            if op.cancel_channels.read().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(op.cancel_channels.read().is_empty());
        assert_eq!(
            op.get_progress(&id).await.unwrap().status,
            ReindexStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn test_retention_panicked_task_does_not_leave_active_metadata() {
        let op = lifecycle_operation(Arc::new(LifecycleSource {
            panic: true,
            ..Default::default()
        }));
        let id = start_lifecycle_job(&op).await;
        assert_eq!(await_terminal(&op, &id).await.status, ReindexStatus::Failed);
        assert!(op.cancel_channels.read().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn test_retention_idle_sweep_expires_status_without_api_calls() {
        let op = lifecycle_operation(Arc::new(LifecycleSource::default()));
        let id = start_lifecycle_job(&op).await;
        await_terminal(&op, &id).await;
        op.jobs.write().get_mut(&id).unwrap().completed_at =
            Some((chrono::Utc::now() - chrono::Duration::hours(25)).to_rfc3339());
        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        assert!(
            !op.jobs.read().contains_key(&id),
            "idle manager retained expired status"
        );
        assert!(op.cancel_channels.read().is_empty());
    }

    #[tokio::test]
    async fn test_retention_keeps_latest_1024_terminal_statuses() {
        let source = Arc::new(LifecycleSource {
            blocked: true,
            ..Default::default()
        });
        let op = lifecycle_operation(source.clone());
        let active = start_lifecycle_job(&op).await;
        source.entered.notified().await;
        // Empty explicit types let other jobs finish while this one is blocked.
        let first = op
            .start(
                TenantContext::system(),
                ReindexRequest::for_types(Vec::<String>::new()),
                None,
            )
            .await
            .unwrap();
        await_terminal(&op, &first).await;
        op.jobs.write().get_mut(&first).unwrap().completed_at =
            Some((chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339());
        let mut latest = first.clone();
        for _ in 0..1024 {
            latest = op
                .start(
                    TenantContext::system(),
                    ReindexRequest::for_types(Vec::<String>::new()),
                    None,
                )
                .await
                .unwrap();
            await_terminal(&op, &latest).await;
        }
        assert_eq!(op.jobs.read().len(), 1025); // 1024 terminal plus the active task.
        assert!(op.get_progress(&first).await.is_none());
        assert_eq!(
            op.get_progress(&latest).await.unwrap().status,
            ReindexStatus::Completed
        );
        assert_eq!(
            op.get_progress(&active).await.unwrap().status,
            ReindexStatus::InProgress
        );
        assert_eq!(op.cancel_channels.read().len(), 1);
        op.cancel(&active).await.unwrap();
        source.release.notify_one();
        for _ in 0..100 {
            if op.cancel_channels.read().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(op.cancel_channels.read().is_empty());
        assert_eq!(
            op.get_progress(&active).await.unwrap().status,
            ReindexStatus::Cancelled
        );
    }

    #[cfg(feature = "R4")]
    #[derive(Default)]
    struct BlockingStartAudit {
        entered: tokio::sync::Notify,
    }

    #[cfg(feature = "R4")]
    #[async_trait]
    impl helios_audit::AuditSink for BlockingStartAudit {
        async fn record(&self, _: helios_fhir::r4::AuditEvent) {
            self.entered.notify_one();
            std::future::pending::<()>().await;
        }
        async fn flush(&self) {}
        fn name(&self) -> &str {
            "blocking-start-audit"
        }
    }

    #[cfg(feature = "R4")]
    #[tokio::test]
    async fn test_retention_aborted_kickoff_does_not_leave_queued_metadata() {
        let audit = Arc::new(BlockingStartAudit::default());
        let op = Arc::new(
            lifecycle_operation(Arc::new(LifecycleSource::default()))
                .with_audit(audit.clone(), "Device/test"),
        );
        let kickoff = tokio::spawn({
            let op = op.clone();
            async move { start_lifecycle_job(&op).await }
        });
        audit.entered.notified().await;
        assert_eq!(op.jobs.read().len(), 1);
        kickoff.abort();
        assert!(kickoff.await.unwrap_err().is_cancelled());
        assert!(op.cancel_channels.read().is_empty());
        assert!(
            op.jobs
                .read()
                .values()
                .all(|p| p.status == ReindexStatus::Failed && p.completed_at.is_some())
        );
    }

    #[tokio::test]
    async fn test_retention_sweeper_does_not_keep_manager_state_alive() {
        let op = lifecycle_operation(Arc::new(LifecycleSource::default()));
        let id = start_lifecycle_job(&op).await;
        await_terminal(&op, &id).await;
        let jobs = Arc::downgrade(&op.jobs);
        let channels = Arc::downgrade(&op.cancel_channels);
        drop(op);
        tokio::task::yield_now().await;
        assert!(jobs.upgrade().is_none());
        assert!(channels.upgrade().is_none());
    }

    #[tokio::test]
    async fn automatic_reindex_clones_coalesce_same_tenant_after_resource_fetch() {
        let (backend, mut events) = ControlledBackend::new(Vec::new(), 0);
        let op = controlled_operation(backend.clone());
        let hook = ReindexOnFinish::with_max_concurrency(op.clone(), 2);
        let cloned_hook = hook.clone();
        let tenant = named_tenant("coalesced");

        hook.reindex_types(&tenant, vec!["Patient".to_string()])
            .await;
        await_controlled_write(&mut events, "coalesced", "Patient").await;

        // The first scan has fetched this Patient and is blocked in its write.
        // A callback on a cloned wrapper must create one follow-up generation.
        cloned_hook
            .reindex_types(&tenant, vec!["Patient".to_string()])
            .await;
        backend.write_gate.add_permits(1);
        await_controlled_write(&mut events, "coalesced", "Patient").await;
        backend.write_gate.add_permits(1);

        await_automatic_idle(&op).await;
        assert_eq!(backend.count_calls.load(Ordering::SeqCst), 2);
        assert_eq!(backend.write_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn automatic_reindex_keeps_tenants_separate_and_respects_concurrency() {
        let (backend, mut events) = ControlledBackend::new(Vec::new(), 0);
        let op = controlled_operation(backend.clone());
        let hook = ReindexOnFinish::with_max_concurrency(op.clone(), 2);
        let alpha = named_tenant("alpha");
        let beta = named_tenant("beta");

        hook.reindex_types(&alpha, vec!["Patient".to_string()])
            .await;
        hook.reindex_types(&beta, vec!["Observation".to_string()])
            .await;

        let mut writes = Vec::new();
        while writes.len() < 2 {
            if let ControlledEvent::Write {
                tenant,
                resource_type,
            } = next_controlled_event(&mut events).await
            {
                writes.push((tenant, resource_type));
            }
        }
        writes.sort();
        assert_eq!(
            writes,
            vec![
                ("alpha".to_string(), "Patient".to_string()),
                ("beta".to_string(), "Observation".to_string())
            ]
        );
        assert_eq!(backend.max_active_writes.load(Ordering::SeqCst), 2);
        assert_eq!(op.automatic.tenants.lock().await.len(), 2);

        backend.write_gate.add_permits(2);
        await_automatic_idle(&op).await;
    }

    #[tokio::test]
    async fn automatic_reindex_backpressures_before_adding_a_new_tenant() {
        let (backend, mut events) = ControlledBackend::new(Vec::new(), 0);
        let op = controlled_operation(backend.clone());
        let hook = ReindexOnFinish::with_max_concurrency(op.clone(), 1);
        let alpha = named_tenant("bounded-alpha");
        let beta = named_tenant("bounded-beta");
        let gamma = named_tenant("bounded-gamma");

        hook.reindex_types(&alpha, vec!["Patient".to_string()])
            .await;
        await_controlled_write(&mut events, "bounded-alpha", "Patient").await;

        hook.reindex_types(&beta, vec!["Observation".to_string()])
            .await;
        await_tenant_waiting(&op, "bounded-beta").await;
        assert_eq!(op.automatic.tenants.lock().await.len(), 2);

        let waiting = tokio::spawn({
            let hook = hook.clone();
            async move {
                hook.reindex_types(&gamma, vec!["Condition".to_string()])
                    .await;
            }
        });
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert!(!waiting.is_finished());
        assert_eq!(op.automatic.tenants.lock().await.len(), 2);

        backend.write_gate.add_permits(1);
        await_controlled_write(&mut events, "bounded-beta", "Observation").await;
        waiting.await.unwrap();
        assert_eq!(op.automatic.tenants.lock().await.len(), 2);
        backend.write_gate.add_permits(1);
        await_controlled_write(&mut events, "bounded-gamma", "Condition").await;
        backend.write_gate.add_permits(1);
        await_automatic_idle(&op).await;
    }

    #[tokio::test]
    async fn simultaneous_first_callbacks_recheck_and_merge_before_scan_finishes() {
        let (backend, mut events) = ControlledBackend::new(Vec::new(), 0);
        let op = controlled_operation(backend.clone());
        let hook = ReindexOnFinish::with_max_concurrency(op.clone(), 1);
        let tenant = named_tenant("simultaneous");
        let limits = op.automatic.limits(1);
        let held_first = limits
            .resident_tenants
            .clone()
            .acquire_owned()
            .await
            .unwrap();
        let held_second = limits
            .resident_tenants
            .clone()
            .acquire_owned()
            .await
            .unwrap();

        let first = tokio::spawn({
            let hook = hook.clone();
            let tenant = tenant.clone();
            async move {
                hook.reindex_types(&tenant, vec!["Patient".to_string()])
                    .await
            }
        });
        let second = tokio::spawn({
            let hook = hook.clone();
            async move {
                hook.reindex_types(&tenant, vec!["Observation".to_string()])
                    .await
            }
        });
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert!(!first.is_finished());
        assert!(!second.is_finished());

        drop(held_first);
        let first_write = loop {
            let event = next_controlled_event(&mut events).await;
            if matches!(event, ControlledEvent::Write { .. }) {
                break event;
            }
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            first.await.unwrap();
            second.await.unwrap();
        })
        .await
        .expect("the losing callback waited for the active scan to drain");

        drop(held_second);
        backend.write_gate.add_permits(4);
        await_automatic_idle(&op).await;
        let written_types: BTreeSet<_> = std::iter::once(first_write)
            .chain(std::iter::from_fn(|| events.try_recv().ok()))
            .filter_map(|event| match event {
                ControlledEvent::Write { resource_type, .. } => Some(resource_type),
                ControlledEvent::Count { .. } => None,
            })
            .collect();
        assert_eq!(
            written_types,
            BTreeSet::from(["Observation".to_string(), "Patient".to_string()])
        );
        assert_eq!(backend.count_calls.load(Ordering::SeqCst), 2);
        assert_eq!(backend.write_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn queued_tenant_runs_before_an_active_tenants_follow_up() {
        let (backend, mut events) = ControlledBackend::new(Vec::new(), 0);
        let op = controlled_operation(backend.clone());
        let hook = ReindexOnFinish::with_max_concurrency(op.clone(), 1);
        let alpha = named_tenant("fair-alpha");
        let beta = named_tenant("fair-beta");

        hook.reindex_types(&alpha, vec!["Patient".to_string()])
            .await;
        await_controlled_write(&mut events, "fair-alpha", "Patient").await;
        hook.reindex_types(&alpha, vec!["Condition".to_string()])
            .await;
        hook.reindex_types(&beta, vec!["Observation".to_string()])
            .await;
        await_tenant_waiting(&op, "fair-beta").await;

        backend.write_gate.add_permits(1);
        await_controlled_write(&mut events, "fair-beta", "Observation").await;
        backend.write_gate.add_permits(1);
        await_controlled_write(&mut events, "fair-alpha", "Condition").await;
        backend.write_gate.add_permits(1);
        await_automatic_idle(&op).await;
    }

    #[tokio::test]
    async fn terminal_driver_does_not_adopt_a_reinserted_tenant_entry() {
        let (backend, mut events) = ControlledBackend::new(Vec::new(), 0);
        let op = controlled_operation(backend.clone());
        let hook = ReindexOnFinish::with_max_concurrency(op.clone(), 1);
        let tenant = named_tenant("reinserted");
        let barrier = Arc::new(AutomaticTerminalBarrier {
            removed: tokio::sync::Notify::new(),
            resume: Semaphore::new(0),
        });
        *op.automatic.terminal_barrier.lock().await = Some(barrier.clone());

        hook.reindex_types(&tenant, vec!["Patient".to_string()])
            .await;
        await_controlled_write(&mut events, "reinserted", "Patient").await;
        let removed = barrier.removed.notified();
        backend.write_gate.add_permits(1);
        removed.await;

        // Model a fresh callback after the old driver removed its entry but
        // before that driver returned. Keep the fresh entry resident without
        // spawning its new driver so any scan can only come from the stale one.
        let limits = op.automatic.limits(1);
        let fresh_resident = limits
            .resident_tenants
            .clone()
            .acquire_owned()
            .await
            .unwrap();
        op.automatic.tenants.lock().await.insert(
            "reinserted".to_string(),
            AutomaticTenantState {
                pending_types: BTreeSet::from(["Observation".to_string()]),
                ..Default::default()
            },
        );
        op.automatic.tenant_changed.notify_waiters();
        barrier.resume.add_permits(1);

        let released_by_old_driver = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::select! {
                permit = limits.resident_tenants.clone().acquire_owned() => {
                    permit.expect("resident semaphore remains open")
                }
                event = events.recv() => {
                    panic!("stale driver consumed the fresh tenant entry: {event:?}")
                }
            }
        })
        .await
        .expect("old tenant driver did not return after removing its entry");

        assert_eq!(backend.count_calls.load(Ordering::SeqCst), 1);
        op.automatic.tenants.lock().await.remove("reinserted");
        drop(fresh_resident);
        drop(released_by_old_driver);
    }

    #[tokio::test]
    async fn explicit_reindex_is_independent_from_automatic_admission() {
        let (backend, mut events) = ControlledBackend::new(Vec::new(), 0);
        let op = controlled_operation(backend.clone());
        let hook = ReindexOnFinish::with_max_concurrency(op.clone(), 1);
        let tenant = named_tenant("explicit-independent");

        hook.reindex_types(&tenant, vec!["Patient".to_string()])
            .await;
        await_controlled_write(&mut events, "explicit-independent", "Patient").await;
        let explicit_job = op
            .start(tenant, ReindexRequest::for_types(["Observation"]), None)
            .await
            .unwrap();
        await_controlled_write(&mut events, "explicit-independent", "Observation").await;
        assert_eq!(backend.max_active_writes.load(Ordering::SeqCst), 2);

        backend.write_gate.add_permits(2);
        await_automatic_idle(&op).await;
        assert_eq!(
            await_terminal(&op, &explicit_job).await.status,
            ReindexStatus::Completed
        );
    }

    #[tokio::test]
    async fn automatic_reindex_clean_success_does_not_retry() {
        let (backend, mut events) = ControlledBackend::new(Vec::new(), 0);
        let op = controlled_operation(backend.clone());
        let hook = ReindexOnFinish::new(op.clone());

        hook.reindex_types(&named_tenant("clean"), vec!["Patient".to_string()])
            .await;
        await_controlled_write(&mut events, "clean", "Patient").await;
        backend.write_gate.add_permits(1);
        await_automatic_idle(&op).await;

        assert_eq!(backend.count_calls.load(Ordering::SeqCst), 1);
        assert_eq!(backend.write_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn automatic_reindex_retries_failure_error_result_and_panic_once() {
        for behavior in [CountBehavior::Fail, CountBehavior::Panic] {
            let (backend, mut events) = ControlledBackend::new(vec![behavior, behavior], 0);
            let op = controlled_operation(backend.clone());
            let hook = ReindexOnFinish::new(op.clone());
            let tenant = named_tenant("failed-count");

            hook.reindex_types(&tenant, vec!["Patient".to_string()])
                .await;
            for _ in 0..2 {
                assert_eq!(
                    next_controlled_event(&mut events).await,
                    ControlledEvent::Count {
                        tenant: "failed-count".to_string(),
                        resource_type: "Patient".to_string(),
                    }
                );
            }
            await_automatic_idle(&op).await;
            assert_eq!(backend.count_calls.load(Ordering::SeqCst), 2);
        }

        // Per-resource transient write errors leave the physical job Completed
        // but are still failures for automatic retry policy.
        let (backend, mut events) = ControlledBackend::new(Vec::new(), 2);
        let op = controlled_operation(backend.clone());
        let hook = ReindexOnFinish::new(op.clone());
        hook.reindex_types(
            &named_tenant("errorful-completion"),
            vec!["Patient".to_string()],
        )
        .await;
        for _ in 0..2 {
            await_controlled_write(&mut events, "errorful-completion", "Patient").await;
            backend.write_gate.add_permits(1);
        }
        await_automatic_idle(&op).await;
        assert_eq!(backend.count_calls.load(Ordering::SeqCst), 2);
        assert_eq!(backend.write_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn automatic_reindex_does_not_retry_a_completion_with_only_permanent_errors() {
        let (backend, mut events) = ControlledBackend::new(Vec::new(), 0);
        backend.permanent_failing_writes.store(1, Ordering::SeqCst);
        let op = controlled_operation(backend.clone());
        let hook = ReindexOnFinish::new(op.clone());

        hook.reindex_types(
            &named_tenant("permanent-errors"),
            vec!["Patient".to_string()],
        )
        .await;
        await_controlled_write(&mut events, "permanent-errors", "Patient").await;
        // Enough permits for a retry, so a wrongly retried job shows up as a
        // second count and write instead of a hang.
        backend.write_gate.add_permits(2);
        await_automatic_idle(&op).await;

        assert_eq!(backend.count_calls.load(Ordering::SeqCst), 1);
        assert_eq!(backend.write_calls.load(Ordering::SeqCst), 1);
        let jobs = op.list_jobs();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].status, ReindexStatus::Completed);
        assert_eq!(jobs[0].errors.len(), 1);
        assert!(!jobs[0].errors[0].retryable);
        assert_eq!(jobs[0].errors[0].resource_id, "controlled-1");
        assert_eq!(jobs[0].tenant_id.as_deref(), Some("permanent-errors"));
    }

    #[test]
    fn automatic_outcome_retries_unless_every_resource_error_is_permanent() {
        let error = |id: &str, retryable| ReindexProgressError {
            resource_type: "Provenance".to_string(),
            resource_id: id.to_string(),
            error: format!("rejected {id}"),
            retryable,
        };
        let mut progress = ReindexProgress::new("job");
        progress.status = ReindexStatus::Completed;
        assert!(matches!(
            automatic_outcome(Some(progress.clone())),
            AutomaticGenerationOutcome::Clean
        ));

        progress.errors.push(error("big", false));
        match automatic_outcome(Some(progress.clone())) {
            AutomaticGenerationOutcome::PermanentErrors {
                count,
                resources,
                first_error,
            } => {
                assert_eq!(count, 1);
                assert_eq!(resources, "Provenance/big");
                assert_eq!(first_error, "rejected big");
            }
            other => panic!("expected permanent errors, got {other:?}"),
        }

        // One transient error among permanent ones earns the retry.
        progress.errors.push(error("flaky", true));
        assert!(matches!(
            automatic_outcome(Some(progress.clone())),
            AutomaticGenerationOutcome::Failed(_)
        ));

        // A job that failed as a whole is retried whatever its resource errors.
        progress.errors.truncate(1);
        progress.status = ReindexStatus::Failed;
        progress.error_message = Some("Failed to fetch resources".to_string());
        assert!(matches!(
            automatic_outcome(Some(progress)),
            AutomaticGenerationOutcome::Failed(_)
        ));
        assert!(matches!(
            automatic_outcome(None),
            AutomaticGenerationOutcome::Failed(_)
        ));
    }

    #[test]
    fn transient_errors_are_the_ones_rest_answers_with_503_or_504() {
        use crate::error::BackendError;
        let backend_name = || "test".to_string();
        let message = || "detail".to_string();
        for transient in [
            BackendError::Unavailable {
                backend_name: backend_name(),
                message: message(),
            },
            BackendError::ConnectionFailed {
                backend_name: backend_name(),
                message: message(),
            },
            BackendError::PoolExhausted {
                backend_name: backend_name(),
            },
            BackendError::Timeout {
                backend_name: backend_name(),
                message: message(),
            },
        ] {
            assert!(is_transient_error(&transient.into()));
        }
        for permanent in [
            BackendError::Internal {
                backend_name: backend_name(),
                message: message(),
                source: None,
            },
            BackendError::QueryError { message: message() },
            BackendError::SerializationError { message: message() },
        ] {
            assert!(!is_transient_error(&permanent.into()));
        }
    }

    #[tokio::test]
    async fn automatic_reindex_keeps_new_work_after_a_retry_is_exhausted() {
        let (backend, mut events) = ControlledBackend::new(Vec::new(), 2);
        let op = controlled_operation(backend.clone());
        let hook = ReindexOnFinish::new(op.clone());
        let tenant = named_tenant("failed-retry-with-pending");

        hook.reindex_types(&tenant, vec!["Patient".to_string()])
            .await;
        await_controlled_write(&mut events, "failed-retry-with-pending", "Patient").await;
        backend.write_gate.add_permits(1);

        // The first failure schedules one retry. Queue unrelated work while
        // that retry is in flight, then make the retry fail as well.
        await_controlled_write(&mut events, "failed-retry-with-pending", "Patient").await;
        hook.reindex_types(&tenant, vec!["Observation".to_string()])
            .await;
        backend.write_gate.add_permits(1);

        // Exhausting Patient's retry must not discard the later callback.
        await_controlled_write(&mut events, "failed-retry-with-pending", "Observation").await;
        backend.write_gate.add_permits(1);
        await_automatic_idle(&op).await;

        assert_eq!(backend.count_calls.load(Ordering::SeqCst), 3);
        assert_eq!(backend.write_calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn automatic_reindex_cancellation_drops_active_types_but_runs_pending_types() {
        let (backend, mut events) = ControlledBackend::new(Vec::new(), 0);
        let op = controlled_operation(backend.clone());
        let hook = ReindexOnFinish::new(op.clone());
        let tenant = named_tenant("cancelled");

        hook.reindex_types(&tenant, vec!["Patient".to_string()])
            .await;
        await_controlled_write(&mut events, "cancelled", "Patient").await;
        hook.reindex_types(&tenant, vec!["Observation".to_string()])
            .await;
        let active_job = op
            .list_jobs()
            .into_iter()
            .find(|progress| progress.status == ReindexStatus::InProgress)
            .expect("automatic job is running")
            .job_id;
        op.cancel(&active_job).await.unwrap();
        backend.write_gate.add_permits(1);

        await_controlled_write(&mut events, "cancelled", "Observation").await;
        backend.write_gate.add_permits(1);
        await_automatic_idle(&op).await;
        assert_eq!(backend.count_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn automatic_reindex_operation_instances_do_not_share_targets() {
        let (first, mut first_events) = ControlledBackend::new(Vec::new(), 0);
        let (second, mut second_events) = ControlledBackend::new(Vec::new(), 0);
        let first_op = controlled_operation(first.clone());
        let second_op = controlled_operation(second.clone());
        let tenant = named_tenant("same-tenant");

        ReindexOnFinish::new(first_op.clone())
            .reindex_types(&tenant, vec!["Patient".to_string()])
            .await;
        ReindexOnFinish::new(second_op.clone())
            .reindex_types(&tenant, vec!["Observation".to_string()])
            .await;
        await_controlled_write(&mut first_events, "same-tenant", "Patient").await;
        await_controlled_write(&mut second_events, "same-tenant", "Observation").await;

        assert_eq!(first.write_calls.load(Ordering::SeqCst), 1);
        assert_eq!(second.write_calls.load(Ordering::SeqCst), 1);
        first.write_gate.add_permits(1);
        second.write_gate.add_permits(1);
        await_automatic_idle(&first_op).await;
        await_automatic_idle(&second_op).await;
    }

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
            retryable: true,
        });

        assert!(progress.has_errors());
        assert!(!progress.has_only_permanent_errors());

        // Errors recorded before classification existed keep their retry.
        let unclassified: ReindexProgressError = serde_json::from_value(serde_json::json!({
            "resource_type": "Patient",
            "resource_id": "1",
            "error": "test error"
        }))
        .unwrap();
        assert!(unclassified.retryable);
    }

    #[test]
    fn test_progress_to_parameters() {
        let progress = ReindexProgress::new("job-123");
        let params = progress.to_parameters();

        assert_eq!(params["resourceType"], "Parameters");
        assert!(params["parameter"].is_array());
    }

    #[test]
    fn test_progress_to_parameters_lists_bounded_resource_errors() {
        let named = |params: &serde_json::Value, name: &str| -> Vec<serde_json::Value> {
            params["parameter"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|parameter| parameter["name"] == name)
                .cloned()
                .collect()
        };
        let mut progress = ReindexProgress::new("job-123");
        progress.status = ReindexStatus::Completed;
        progress.errors.push(ReindexProgressError {
            resource_type: "Provenance".to_string(),
            resource_id: "oversized".to_string(),
            error: "nested documents exceeded the allowed limit".to_string(),
            retryable: false,
        });
        let params = progress.to_parameters();
        assert!(named(&params, "errorMessage").is_empty());
        assert!(named(&params, "errorsOmitted").is_empty());
        let errors = named(&params, "error");
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0]["part"],
            serde_json::json!([
                {"name": "resourceType", "valueCode": "Provenance"},
                {"name": "resourceId", "valueId": "oversized"},
                {"name": "message", "valueString": "nested documents exceeded the allowed limit"},
                {"name": "retryable", "valueBoolean": false}
            ])
        );

        for n in 0..MAX_REPORTED_RESOURCE_ERRORS + 4 {
            progress.errors.push(ReindexProgressError {
                resource_type: "Patient".to_string(),
                resource_id: format!("p{n}"),
                error: "unavailable".to_string(),
                retryable: true,
            });
        }
        progress.error_message = Some("Failed to fetch resources".to_string());
        let params = progress.to_parameters();
        let total = MAX_REPORTED_RESOURCE_ERRORS + 5;
        assert_eq!(named(&params, "errorCount")[0]["valueInteger"], total);
        assert_eq!(named(&params, "error").len(), MAX_REPORTED_RESOURCE_ERRORS);
        assert_eq!(named(&params, "errorsOmitted")[0]["valueInteger"], 5);
        assert_eq!(
            named(&params, "errorMessage")[0]["valueString"],
            "Failed to fetch resources"
        );
    }
}
