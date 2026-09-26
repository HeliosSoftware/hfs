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
#[derive(Debug, Default)]
pub struct ResourcePage {
    /// The resources in this page.
    pub resources: Vec<StoredResource>,
    /// Cursor for the next page (None if this is the last page).
    ///
    /// A source must derive it from the rows it *scanned*, not from the
    /// resources it managed to decode: a row reported in [`Self::skipped`]
    /// still advances the cursor, so one unreadable row never ends the
    /// pagination of its type.
    pub next_cursor: Option<String>,
    /// Rows the source read but could not turn into a [`StoredResource`]
    /// (unparseable content or timestamp). The reindex records each one as a
    /// permanent per-resource error instead of dropping it silently (#1125).
    pub skipped: Vec<SkippedResource>,
}

/// A stored row a [`ReindexSource`] read but could not decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedResource {
    /// The logical id of the row that could not be decoded.
    pub resource_id: String,
    /// Why it could not be decoded.
    pub reason: String,
}

/// Identifies one resource of one type, for a reindex scoped to specific
/// resources rather than whole types (see [`ReindexRequest::resource_ids`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ResourceRef {
    /// Resource type, e.g. `Provenance`.
    pub resource_type: String,
    /// Logical id.
    pub resource_id: String,
}

impl ResourceRef {
    /// Creates a reference to `resource_type/resource_id`.
    pub fn new(resource_type: impl Into<String>, resource_id: impl Into<String>) -> Self {
        Self {
            resource_type: resource_type.into(),
            resource_id: resource_id.into(),
        }
    }
}

/// Page size [`ReindexSource::fetch_resources_by_ids`]'s default
/// implementation scans with.
const FETCH_BY_IDS_SCAN_PAGE: u32 = 1000;

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

    /// Fetches a page bounded by resource count **and** by bytes of stored
    /// content, so a page of large resources is not one oversized read
    /// (`max_bytes` of `0` means no byte cap, #1125).
    ///
    /// The default ignores the cap, which is what every source did before it
    /// existed; a source that honours it must still return at least one
    /// resource, or the page loop cannot advance.
    async fn fetch_resources_page_capped(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
        max_bytes: u64,
    ) -> StorageResult<ResourcePage> {
        let _ = max_bytes;
        self.fetch_resources_page(tenant, resource_type, cursor, limit)
            .await
    }

    /// Fetches the current, non-deleted resources of `resource_type` whose
    /// ids are in `ids`.
    ///
    /// Ids that no longer exist (deleted since they were recorded) are simply
    /// absent from the result; order is unspecified. The default scans the
    /// type with [`Self::fetch_resources_page`] and filters, stopping once
    /// every id has been found — correct for every source, but linear in the
    /// size of the type. Sources that can look resources up by id should
    /// override it.
    async fn fetch_resources_by_ids(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        ids: &[String],
    ) -> StorageResult<Vec<StoredResource>> {
        let mut wanted: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
        let mut found = Vec::with_capacity(wanted.len());
        let mut cursor: Option<String> = None;
        while !wanted.is_empty() {
            let page = self
                .fetch_resources_page(
                    tenant,
                    resource_type,
                    cursor.as_deref(),
                    FETCH_BY_IDS_SCAN_PAGE,
                )
                .await?;
            for resource in page.resources {
                if wanted.remove(resource.id()) {
                    found.push(resource);
                }
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        Ok(found)
    }
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
    ///
    /// PostgreSQL treats the supplied resource as an identity only. It locks
    /// that identity, reads the current stored row and current SearchParameter
    /// definitions, then atomically replaces its search and FTS entries. A
    /// deleted or absent identity clears stale entries and returns zero. Code
    /// that needs to append arbitrary extracted rows must use the low-level
    /// writer under its own transaction, locking, and freshness guarantees.
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
    /// PostgreSQL may split this input into bounded concurrent write groups;
    /// its result vector still has exactly one slot per input occurrence in
    /// input order. The next source page is fetched only after this call ends.
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

    /// Specific resources to reindex (None = every resource of the selected
    /// types). When set, the run fetches exactly these resources instead of
    /// paging whole types, so a retry covers only what failed (#1125).
    #[serde(default)]
    pub resource_ids: Option<Vec<ResourceRef>>,

    /// Upper bound, in bytes of stored content, on one page of resources
    /// (`0` = no cap, only [`Self::batch_size`]).
    ///
    /// A page of 1,000 Synthea `Provenance` resources of ~108 KB each is
    /// ~108 MB held in memory before a single document goes on the wire.
    /// A source that honours the cap ends the page at the first resource
    /// that crosses it, and always returns at least one (#1125).
    #[serde(default)]
    pub batch_bytes: u64,
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
            resource_ids: None,
            batch_bytes: 0,
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

    /// Creates a reindex request for specific resources (see
    /// [`ReindexRequest::resource_ids`]). `resource_types` is set to the
    /// distinct types of `resources`, sorted.
    pub fn for_resources<I>(resources: I) -> Self
    where
        I: IntoIterator<Item = ResourceRef>,
    {
        let resources: Vec<ResourceRef> = resources.into_iter().collect();
        let types: BTreeSet<String> = resources.iter().map(|r| r.resource_type.clone()).collect();
        Self {
            resource_types: Some(types.into_iter().collect()),
            resource_ids: Some(resources),
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

    /// Sets the byte cap of one page (see [`Self::batch_bytes`]).
    pub fn with_batch_bytes(mut self, bytes: u64) -> Self {
        self.batch_bytes = bytes;
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

    /// Whether the job rebuilt named resources
    /// ([`ReindexRequest::resource_ids`]) rather than whole types — the retry of
    /// an earlier job's transient failures. A clean resource-scoped job says
    /// nothing about the earlier job's permanent failures, so a per-tenant view
    /// must not let it clear them (#1125).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub resource_scoped: bool,
}

/// An error encountered during reindexing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReindexProgressError {
    /// Resource type.
    pub resource_type: String,
    /// Resource ID.
    pub resource_id: String,
    /// Error message.
    ///
    /// Empty past the first [`MAX_REPORTED_RESOURCE_ERRORS`] errors of a job:
    /// the type, id and classification of every failure are kept, because a
    /// retry needs them, but a job that fails on millions of resources must not
    /// hold millions of messages. `$reindex-status` lists only the first ones,
    /// and the log carries a rate-limited sample per type.
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

/// Failing `Type/id`s named in the closing log line of an automatic generation
/// that ended with resource errors, permanent or transient.
const MAX_LOGGED_RESOURCE_ERRORS: usize = 5;

/// Per-resource failures logged at `warn` for one resource type of one run;
/// the rest are only counted, and summarized when the type ends.
const MAX_WARNED_RESOURCE_FAILURES_PER_TYPE: u64 = 20;

/// Logs a run's per-resource failures: each one by `Type/id` up to
/// [`MAX_WARNED_RESOURCE_FAILURES_PER_TYPE`] per type, then one summary per
/// type. Transient failures are named exactly like permanent ones, so a
/// transport outage is as diagnosable from the log as a rejected document.
struct ResourceFailureLog {
    job_id: String,
    tenant_id: String,
    resource_type: Option<String>,
    failed: u64,
    transient: u64,
}

impl ResourceFailureLog {
    fn new(job_id: &str, tenant: &TenantContext) -> Self {
        Self {
            job_id: job_id.to_string(),
            tenant_id: tenant.tenant_id().to_string(),
            resource_type: None,
            failed: 0,
            transient: 0,
        }
    }

    /// Closes the previous type's summary and starts counting `resource_type`.
    fn start_type(&mut self, resource_type: &str) {
        self.finish_type();
        self.resource_type = Some(resource_type.to_string());
    }

    fn record(&mut self, resource_type: &str, resource_id: &str, error: &str, retryable: bool) {
        self.failed += 1;
        if retryable {
            self.transient += 1;
        }
        if self.failed <= MAX_WARNED_RESOURCE_FAILURES_PER_TYPE {
            tracing::warn!(
                tenant = %self.tenant_id,
                job_id = %self.job_id,
                resource_type,
                resource_id,
                retryable,
                error,
                "reindex could not index a resource"
            );
        }
    }

    /// Emits the summary of the type being counted, if it had failures.
    fn finish_type(&mut self) {
        if let Some(resource_type) = self.resource_type.take()
            && self.failed > 0
        {
            tracing::warn!(
                tenant = %self.tenant_id,
                job_id = %self.job_id,
                resource_type = %resource_type,
                failed = self.failed,
                transient = self.transient,
                permanent = self.failed - self.transient,
                not_logged = self.failed.saturating_sub(MAX_WARNED_RESOURCE_FAILURES_PER_TYPE),
                "reindex left resources of this type unindexed (every failure is listed by $reindex-status for this job)"
            );
        }
        self.failed = 0;
        self.transient = 0;
    }
}

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
            resource_scoped: false,
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
    /// `startedAt` and `completedAt` appear once set; their difference is how
    /// long the rebuild took.
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
        if let Some(started_at) = &self.started_at {
            parameter.push(serde_json::json!({"name": "startedAt", "valueDateTime": started_at}));
        }
        if let Some(completed_at) = &self.completed_at {
            parameter
                .push(serde_json::json!({"name": "completedAt", "valueDateTime": completed_at}));
        }
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

/// Where the record of an outstanding deferred rebuild lives, so a restart
/// mid-rebuild can find it instead of losing it with the in-process job map
/// (#1125). Implemented by the bulk-submit storage; a backend that does not
/// record it simply never resumes, as before.
#[async_trait]
pub trait DeferredReindexLedger: Send + Sync {
    /// The rebuild this manifest owed has run: drop the marker.
    async fn rebuild_finished(&self, tenant: &TenantContext, manifest_id: &str);
}

/// What [`AutomaticReindexCoordinator::enqueue`] needs to start, or merge
/// into, a tenant's pending generation.
struct EnqueueGeneration {
    op: Arc<ReindexOperation>,
    tenant: TenantContext,
    resource_types: Vec<String>,
    context: DeferredReindexContext,
    options: AutomaticRunOptions,
    max_concurrency: usize,
    /// Where the "still owes a rebuild" marker is cleared (#1125).
    ledger: Option<Arc<dyn DeferredReindexLedger>>,
}

/// Shape of the `ReindexRequest` an automatic generation starts with.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AutomaticRunOptions {
    /// Resources per rebuild transaction.
    pub(crate) batch_size: u32,
    /// `ReindexRequest::bulk_index_rebuild` for the runs the hook starts.
    pub(crate) bulk_index_rebuild: bool,
    /// `ReindexRequest::batch_bytes` for the runs the hook starts.
    pub(crate) batch_bytes: u64,
}

impl Default for AutomaticRunOptions {
    fn default() -> Self {
        Self {
            batch_size: DEFERRED_REINDEX_BATCH_SIZE,
            bulk_index_rebuild: false,
            batch_bytes: 0,
        }
    }
}

#[derive(Default)]
struct AutomaticTenantState {
    pending_types: BTreeSet<String>,
    /// Resources a finished generation failed on transiently, queued for a
    /// retry that covers only them (#1125). Always a retry: a resource
    /// generation that fails is not retried again.
    pending_resources: BTreeSet<ResourceRef>,
    next_generation: u64,
    consecutive_failures: u8,
    /// The most recent request merged into the pending generation. Only its
    /// submission and manifest are logged; which markers a generation clears
    /// is [`Self::owed_manifests`], because merged requests each owe one.
    context: DeferredReindexContext,
    /// Every manifest whose "still owes a rebuild" marker the pending
    /// generation clears when it ends (#1213). A merge adds to it, like
    /// `pending_types`, instead of replacing it the way `context` is.
    owed_manifests: BTreeSet<String>,
    options: AutomaticRunOptions,
    waiting_for_generation: bool,
    /// Where to clear the "still owes a rebuild" marker once a generation
    /// finishes (#1125).
    ledger: Option<Arc<dyn DeferredReindexLedger>>,
}

impl AutomaticTenantState {
    /// Records that the pending generation owes `context`'s manifest its
    /// rebuild. A request without a manifest owes no marker.
    fn owe_manifest(&mut self, context: &DeferredReindexContext) {
        if let Some(manifest_id) = &context.manifest_id {
            self.owed_manifests.insert(manifest_id.clone());
        }
    }
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
        progress.resource_scoped = request.resource_ids.is_some();

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
    Failed {
        /// Status, error count and job-level error message.
        summary: String,
        /// Up to [`MAX_LOGGED_RESOURCE_ERRORS`] failing `Type/id`s, permanent
        /// or transient; empty when the job failed before any resource.
        resources: String,
        /// What a retry has to cover.
        retry: RetryScope,
    },
}

/// What the retry of a failed automatic generation covers.
#[derive(Debug, PartialEq, Eq)]
enum RetryScope {
    /// The job completed, and these resources failed transiently: only they
    /// are retried, not every resource of their types (#1125).
    Resources(Vec<ResourceRef>),
    /// Too many resources failed transiently to hold and fetch them one by
    /// one (more than [`MAX_RETRY_RESOURCES`]): every resource of the types
    /// that failed is retried, not the generation's other types.
    Types(Vec<String>),
    /// The job failed as a whole (start, count, fetch, panic), so which
    /// resources are missing is unknown: the generation is retried as it was.
    Generation,
}

/// Most transiently failed resources a retry names one by one. Past it the
/// retry rebuilds the failing types instead: a source without an indexed
/// by-id fetch pages its whole type for every batch of ids, and each named
/// resource is held in memory until the retry runs.
const MAX_RETRY_RESOURCES: usize = 50_000;

/// The work of one automatic generation.
#[derive(Debug, Clone)]
enum GenerationScope {
    /// Every resource of these types.
    Types(Vec<String>),
    /// Exactly these resources — the retry of a generation's transient
    /// resource failures.
    Resources(Vec<ResourceRef>),
}

impl GenerationScope {
    fn request(&self, options: AutomaticRunOptions) -> ReindexRequest {
        match self {
            Self::Types(types) => ReindexRequest::for_types(types.clone())
                .with_bulk_index_rebuild(options.bulk_index_rebuild),
            // Dropping and rebuilding a writer's value indexes costs a pass
            // over its whole index: out of proportion for a handful of ids.
            Self::Resources(resources) => ReindexRequest::for_resources(resources.clone()),
        }
        .with_batch_size(options.batch_size)
        .with_batch_bytes(options.batch_bytes)
    }

    /// The distinct resource types the generation touches, sorted.
    fn types(&self) -> Vec<String> {
        match self {
            Self::Types(types) => types.clone(),
            Self::Resources(resources) => resources
                .iter()
                .map(|r| r.resource_type.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        }
    }

    /// How many resources were named, for a resource generation.
    fn resource_count(&self) -> Option<usize> {
        match self {
            Self::Types(_) => None,
            Self::Resources(resources) => Some(resources.len()),
        }
    }
}

/// `Type/id` of the first [`MAX_LOGGED_RESOURCE_ERRORS`] distinct failing
/// resources, joined for a log field.
fn logged_resources(errors: &[ReindexProgressError]) -> String {
    let mut seen = BTreeSet::new();
    errors
        .iter()
        .map(|error| format!("{}/{}", error.resource_type, error.resource_id))
        .filter(|name| seen.insert(name.clone()))
        .take(MAX_LOGGED_RESOURCE_ERRORS)
        .collect::<Vec<_>>()
        .join(", ")
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
                resources: logged_resources(&progress.errors),
                first_error: progress.errors[0].error.clone(),
            }
        }
        Some(progress) => {
            // A completed job knows exactly which resources failed; the ones
            // that failed transiently are the retry. A job that failed as a
            // whole may have stopped before reaching most of its resources.
            let retryable: BTreeSet<ResourceRef> = progress
                .errors
                .iter()
                .filter(|error| error.retryable)
                .map(|error| ResourceRef::new(&error.resource_type, &error.resource_id))
                .collect();
            let retry = if progress.status == ReindexStatus::Completed
                && progress.error_message.is_none()
                && !retryable.is_empty()
            {
                if retryable.len() > MAX_RETRY_RESOURCES {
                    RetryScope::Types(
                        retryable
                            .iter()
                            .map(|resource| resource.resource_type.clone())
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .collect(),
                    )
                } else {
                    RetryScope::Resources(retryable.into_iter().collect())
                }
            } else {
                RetryScope::Generation
            };
            AutomaticGenerationOutcome::Failed {
                summary: format!(
                    "status {:?}, {} resource errors{}",
                    progress.status,
                    progress.errors.len(),
                    progress
                        .error_message
                        .as_deref()
                        .map(|message| format!(", error: {message}"))
                        .unwrap_or_default()
                ),
                resources: logged_resources(&progress.errors),
                retry,
            }
        }
        None => AutomaticGenerationOutcome::Failed {
            summary: "job status disappeared after task exit".to_string(),
            resources: String::new(),
            retry: RetryScope::Generation,
        },
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

    async fn enqueue(self: Arc<Self>, request: EnqueueGeneration) {
        let EnqueueGeneration {
            op,
            tenant,
            resource_types,
            context,
            options,
            max_concurrency,
            ledger,
        } = request;
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
                state.owe_manifest(&context);
                state.context = context.clone();
                state.options = options;
                state.ledger = ledger.clone();
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
                    state.owe_manifest(&context);
                    state.context = context.clone();
                    state.options = options;
                    state.ledger = ledger.clone();
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
                state.owe_manifest(&context);
                state.context = context;
                state.options = options;
                state.ledger = ledger;
                return;
            }
            tenants.insert(
                tenant_id.clone(),
                AutomaticTenantState {
                    pending_types: requested_types,
                    owed_manifests: context.manifest_id.iter().cloned().collect(),
                    context,
                    options,
                    ledger,
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
            let (scope, generation, context, owed_manifests, options, ledger) = {
                let mut tenants = self.tenants.lock().await;
                let Some(state) = tenants.get_mut(&tenant_id) else {
                    return;
                };
                // Whole types first. A queued resource retry whose type is
                // about to be rebuilt in full is covered by that rebuild.
                let scope = if !state.pending_types.is_empty() {
                    let types = std::mem::take(&mut state.pending_types);
                    state
                        .pending_resources
                        .retain(|resource| !types.contains(&resource.resource_type));
                    GenerationScope::Types(types.into_iter().collect())
                } else if !state.pending_resources.is_empty() {
                    GenerationScope::Resources(
                        std::mem::take(&mut state.pending_resources)
                            .into_iter()
                            .collect(),
                    )
                } else {
                    tenants.remove(&tenant_id);
                    return;
                };
                let generation = state.next_generation;
                state.next_generation += 1;
                state.waiting_for_generation = true;
                (
                    scope,
                    generation,
                    state.context.clone(),
                    // The generation covers every manifest merged into it, so
                    // it takes all of their markers, as it takes their types.
                    std::mem::take(&mut state.owed_manifests),
                    state.options,
                    state.ledger.clone(),
                )
            };
            let resource_types = scope.types();
            let resource_count = scope.resource_count();

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
                .start_tracked(tenant.clone(), scope.request(options), None)
                .await;
            let (job_id, outcome) = match started {
                Ok((job_id, task_exit)) => {
                    tracing::info!(
                        tenant = %tenant_id,
                        generation,
                        job_id = %job_id,
                        submission = ?context.submission_id,
                        manifest = ?context.manifest_id,
                        manifests = ?owed_manifests,
                        types = ?resource_types,
                        resources = ?resource_count,
                        "deferred reindex generation started"
                    );
                    let _ = task_exit.await;
                    let outcome = automatic_outcome(op.get_progress(&job_id).await);
                    (Some(job_id), outcome)
                }
                Err(error) => (
                    None,
                    AutomaticGenerationOutcome::Failed {
                        summary: format!("failed to start reindex job: {error}"),
                        resources: String::new(),
                        retry: RetryScope::Generation,
                    },
                ),
            };
            drop(running_permit);

            // `Some(n)`: this generation's retry was queued, covering `n`
            // resources, or every resource of its types when `None`.
            let mut retry: Option<Option<usize>> = None;
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
                    AutomaticGenerationOutcome::Failed {
                        retry: scope_retry, ..
                    } => {
                        match (&scope, scope_retry) {
                            // A resource generation is already the retry.
                            (GenerationScope::Resources(_), _) => {
                                state.consecutive_failures = 0;
                            }
                            // Only the resources that failed transiently are
                            // retried. That retry is bounded by being a
                            // resource generation, so the types' budget resets
                            // — unless this generation already was the retry.
                            (GenerationScope::Types(_), RetryScope::Resources(resources)) => {
                                if state.consecutive_failures == 0 {
                                    state.pending_resources.extend(resources.iter().cloned());
                                    retry = Some(Some(resources.len()));
                                }
                                state.consecutive_failures = 0;
                            }
                            (GenerationScope::Types(types), scope_retry) => {
                                let retry_types = match scope_retry {
                                    RetryScope::Types(failing) => failing,
                                    _ => types,
                                };
                                state.consecutive_failures += 1;
                                if state.consecutive_failures == 1 {
                                    state.pending_types.extend(retry_types.iter().cloned());
                                    retry = Some(None);
                                } else {
                                    // This generation exhausted its retry, but
                                    // callbacks may have queued independent work
                                    // while it ran. Give that later batch its own
                                    // retry budget instead of removing the whole
                                    // tenant entry below.
                                    state.consecutive_failures = 0;
                                }
                            }
                        }
                    }
                }
                // A queued retry finishes this generation's work, so the markers
                // it owed go with the retry and are cleared when that ends. A
                // failure that is not retried leaves them set for a restart to
                // resume, as does a cancellation.
                if retry.is_some() {
                    state.owed_manifests.extend(owed_manifests.iter().cloned());
                }
                if state.pending_types.is_empty() && state.pending_resources.is_empty() {
                    tenants.remove(&tenant_id);
                    false
                } else {
                    true
                }
            };
            self.tenant_changed.notify_waiters();

            // The rebuild these manifests owed has run — cleanly, or with
            // resources the backend rejects on every attempt. Either way there
            // is nothing left for a restart to resume, so the markers go
            // (#1125) — every one the generation covered, not only the last
            // merged request's (#1213). A failure keeps them: the work is
            // still outstanding.
            if matches!(
                outcome,
                AutomaticGenerationOutcome::Clean
                    | AutomaticGenerationOutcome::PermanentErrors { .. }
            ) && let Some(ledger) = &ledger
            {
                for manifest_id in &owed_manifests {
                    ledger.rebuild_finished(&tenant, manifest_id).await;
                }
            }

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
                AutomaticGenerationOutcome::Failed {
                    summary, resources, ..
                } => match retry {
                    Some(retried) => tracing::warn!(
                        tenant = %tenant_id,
                        generation,
                        job_id = ?job_id,
                        error = %summary,
                        types = ?resource_types,
                        resources = %resources,
                        retry_resources = ?retried,
                        retry_scope = if retried.is_some() {
                            "failed resources only"
                        } else {
                            "whole types"
                        },
                        "deferred reindex generation failed; retrying once"
                    ),
                    None => tracing::error!(
                        tenant = %tenant_id,
                        generation,
                        job_id = ?job_id,
                        error = %summary,
                        types = ?resource_types,
                        generation_resources = ?resource_count,
                        resources = %resources,
                        "deferred reindex failed twice; run $reindex manually (every failure is listed by $reindex-status for this job)"
                    ),
                },
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
///
/// Every error keeps its type, id and classification; only the first
/// [`MAX_REPORTED_RESOURCE_ERRORS`] keep their message (see
/// [`ReindexProgressError::error`]).
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
        let error = if progress.errors.len() < MAX_REPORTED_RESOURCE_ERRORS {
            error
        } else {
            String::new()
        };
        progress.errors.push(ReindexProgressError {
            resource_type: resource_type.to_string(),
            resource_id: resource_id.to_string(),
            error,
            retryable,
        });
    }
}

/// Records a per-resource error against the job and in the log.
#[allow(clippy::too_many_arguments)]
fn record_resource_failure(
    jobs: &Arc<RwLock<HashMap<String, ReindexProgress>>>,
    job_id: &str,
    failures: &mut ResourceFailureLog,
    resource_type: &str,
    resource_id: &str,
    error: String,
    retryable: bool,
) {
    failures.record(resource_type, resource_id, &error, retryable);
    push_error(jobs, job_id, resource_type, resource_id, error, retryable);
}

/// Rewrites one batch of resources through every writer and advances the
/// job's counters by `resources.len() + extra_processed`.
///
/// Page-at-a-time so a writer can wrap it in one transaction; each writer
/// reports a per-resource outcome for error attribution. The entry count for
/// progress comes from the writers' own extraction — the driver no longer
/// extracts a second time just to count. `extra_processed` accounts for rows
/// of the batch that were read but not written (skipped, or deleted since
/// they were named).
#[allow(clippy::too_many_arguments)]
async fn write_resource_batch(
    tenant: &TenantContext,
    writers: &[Arc<dyn ReindexTarget>],
    jobs: &Arc<RwLock<HashMap<String, ReindexProgress>>>,
    job_id: &str,
    failures: &mut ResourceFailureLog,
    resource_type: &str,
    resources: &[StoredResource],
    extra_processed: u64,
) {
    let mut wrote_any: Vec<bool> = vec![false; resources.len()];
    let mut entry_counts: Vec<u64> = vec![0; resources.len()];
    if !resources.is_empty() {
        for writer in writers {
            let outcomes = writer.write_search_entries_page(tenant, resources).await;
            for (i, outcome) in outcomes.into_iter().enumerate() {
                match outcome {
                    Ok(written) => {
                        wrote_any[i] = true;
                        entry_counts[i] = entry_counts[i].max(written as u64);
                    }
                    Err(e) => record_resource_failure(
                        jobs,
                        job_id,
                        failures,
                        resource_type,
                        resources[i].id(),
                        format!("Failed to rebuild index entries: {e}"),
                        is_transient_error(&e),
                    ),
                }
            }
        }
    }

    let mut jobs_guard = jobs.write();
    if let Some(progress) = jobs_guard.get_mut(job_id) {
        progress.processed_resources += resources.len() as u64 + extra_processed;
        progress.entries_created += wrote_any
            .iter()
            .zip(&entry_counts)
            .filter(|(wrote, _)| **wrote)
            .map(|(_, entries)| entries)
            .sum::<u64>();
    }
}

/// How long the driver stands back between two page writes.
///
/// Pages are written back-to-back, and on the SQLite backend each one is an
/// `IMMEDIATE` transaction: without a gap the rebuild re-takes the write lock
/// before any foreground writer that is parked on it gets to run, so a single
/// small insert can wait out the whole `busy_timeout` and fail with "database
/// is locked" (#1185). See [`DEFERRED_REINDEX_BATCH_SIZE`] for the other half
/// of this trade-off — page size sets how long each lock hold lasts, this sets
/// how long the gap between them is.
///
/// Milliseconds, not microseconds: SQLite's busy handler backs off on a
/// millisecond granularity, so a shorter window would close again before a
/// waiting writer is retried. At the deferred page size of 1,000 resources a
/// page takes roughly half a second to write, so this costs about 1% of the
/// rebuild's wall clock.
const REINDEX_PAGE_YIELD: Duration = Duration::from_millis(5);

/// Yields the runtime and the storage write lock between two page writes.
///
/// [`tokio::task::yield_now`] alone only returns the tokio worker to the
/// scheduler; the SQLite write lock is released by the `COMMIT` that already
/// happened, so what a parked foreground writer actually needs is wall-clock
/// time in which this task is not asking for the lock again. Hence the sleep.
async fn yield_between_pages() {
    tokio::task::yield_now().await;
    tokio::time::sleep(REINDEX_PAGE_YIELD).await;
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

    // Named resources, grouped by type: a run scoped to them fetches exactly
    // these ids instead of paging whole types (#1125).
    let named_resources: Option<std::collections::BTreeMap<String, Vec<String>>> =
        request.resource_ids.as_ref().map(|resources| {
            let mut by_type: std::collections::BTreeMap<String, BTreeSet<String>> =
                std::collections::BTreeMap::new();
            for resource in resources {
                by_type
                    .entry(resource.resource_type.clone())
                    .or_default()
                    .insert(resource.resource_id.clone());
            }
            by_type
                .into_iter()
                .map(|(resource_type, ids)| (resource_type, ids.into_iter().collect()))
                .collect()
        });

    // Determine resource types to process
    let resource_types = match (&named_resources, request.resource_types) {
        (Some(named), _) => named.keys().cloned().collect(),
        (None, Some(types)) => types,
        (None, None) => match source.list_resource_types(&tenant).await {
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
    if let Some(named) = &named_resources {
        total_resources = named.values().map(|ids| ids.len() as u64).sum();
    } else {
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

    let mut failures = ResourceFailureLog::new(&job_id, &tenant);
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
            failures.start_type(resource_type);

            // A run scoped to named resources fetches them in batches of
            // `batch_size` ids; an id deleted since it was named is simply
            // absent, and counts as processed with nothing to index.
            if let Some(ids) = named_resources
                .as_ref()
                .and_then(|named| named.get(resource_type))
            {
                for (batch_index, batch) in
                    ids.chunks(request.batch_size.max(1) as usize).enumerate()
                {
                    if cancel_rx.try_recv().is_ok() {
                        return Err(RunExit::Cancelled);
                    }
                    // Between two batches only, never before the first or
                    // after the last: the gap exists to let a foreground
                    // writer take the lock, and there is nothing to yield to
                    // once this run has stopped writing.
                    if batch_index > 0 {
                        yield_between_pages().await;
                    }
                    let fetch_span = crate::perf::span(crate::perf::Phase::ReindexFetch);
                    let fetched = source
                        .fetch_resources_by_ids(&tenant, resource_type, batch)
                        .await;
                    drop(fetch_span);
                    let resources = match fetched {
                        Ok(resources) => resources,
                        Err(e) => {
                            return Err(RunExit::Failed(format!("Failed to fetch resources: {e}")));
                        }
                    };
                    let missing = (batch.len() as u64).saturating_sub(resources.len() as u64);
                    write_resource_batch(
                        &tenant,
                        &writers,
                        &jobs,
                        &job_id,
                        &mut failures,
                        resource_type,
                        &resources,
                        missing,
                    )
                    .await;
                }
                continue;
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
                    .fetch_resources_page_capped(
                        &tenant,
                        resource_type,
                        cursor.as_deref(),
                        request.batch_size,
                        request.batch_bytes,
                    )
                    .await;
                drop(fetch_span);
                let page = match fetched {
                    Ok(page) => page,
                    Err(e) => {
                        return Err(RunExit::Failed(format!("Failed to fetch resources: {e}")));
                    }
                };

                // A row the source read but could not decode is a resource that
                // stays unsearchable until the row is repaired: a permanent
                // failure, recorded rather than silently dropped (#1125).
                for skipped in &page.skipped {
                    record_resource_failure(
                        &jobs,
                        &job_id,
                        &mut failures,
                        resource_type,
                        &skipped.resource_id,
                        format!("Failed to read stored resource: {}", skipped.reason),
                        false,
                    );
                }

                // Rebuild the page through every writer.
                write_resource_batch(
                    &tenant,
                    &writers,
                    &jobs,
                    &job_id,
                    &mut failures,
                    resource_type,
                    &page.resources,
                    page.skipped.len() as u64,
                )
                .await;

                // Check if there are more pages
                match page.next_cursor {
                    Some(next) => {
                        cursor = Some(next);
                        // Stand back before re-taking the write lock for the
                        // next page. Only between pages: the last page has no
                        // successor to hold the lock against.
                        yield_between_pages().await;
                    }
                    None => break,
                }
            }
        }

        Ok(())
    }
    .await;
    failures.finish_type();

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
    /// Clears the persisted "owes a rebuild" marker when a generation ends
    /// (#1125). `None` keeps the pre-#1125 behaviour: nothing is recorded and
    /// nothing is resumed.
    ledger: Option<Arc<dyn DeferredReindexLedger>>,
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
            ledger: None,
        }
    }

    /// Overrides the resources-per-transaction page of the rebuild.
    pub fn with_batch_size(mut self, batch_size: u32) -> Self {
        self.options.batch_size = batch_size.max(1);
        self
    }

    /// Records rebuild completion in `ledger`, so an outstanding rebuild is
    /// discoverable — and resumable — after a restart (#1125).
    pub fn with_ledger(mut self, ledger: Arc<dyn DeferredReindexLedger>) -> Self {
        self.ledger = Some(ledger);
        self
    }

    /// Caps one page of the rebuild by bytes of stored content as well as by
    /// resource count (`0` = count only).
    pub fn with_batch_bytes(mut self, batch_bytes: u64) -> Self {
        self.options.batch_bytes = batch_bytes;
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
            .enqueue(EnqueueGeneration {
                op: self.op.clone(),
                tenant: tenant.clone(),
                resource_types,
                context,
                options: self.options,
                max_concurrency: self.max_concurrency,
                ledger: self.ledger.clone(),
            })
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
                skipped: Vec::new(),
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

    /// The write of a resource retry generation: it names its resources, so
    /// it fetches them by id without counting (paging) the type first.
    async fn await_controlled_retry_write(
        events: &mut tokio::sync::mpsc::UnboundedReceiver<ControlledEvent>,
        tenant: &str,
        resource_type: &str,
    ) {
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

    /// Records every marker the coordinator clears, in order (#1213).
    #[derive(Default)]
    struct RecordingLedger {
        cleared: parking_lot::Mutex<Vec<(String, String)>>,
    }

    impl RecordingLedger {
        /// The manifests cleared so far, sorted.
        fn cleared(&self) -> Vec<String> {
            let mut cleared: Vec<_> = self
                .cleared
                .lock()
                .iter()
                .map(|(_, manifest)| manifest.clone())
                .collect();
            cleared.sort();
            cleared
        }

        /// Waits until `count` markers were cleared. The clear runs after the
        /// tenant entry is gone, so an idle coordinator does not imply it.
        async fn await_cleared(&self, count: usize) -> Vec<String> {
            tokio::time::timeout(Duration::from_secs(2), async {
                while self.cleared.lock().len() < count {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap_or_else(|_| {
                panic!("expected {count} cleared markers, got {:?}", self.cleared())
            });
            self.cleared()
        }
    }

    #[async_trait]
    impl DeferredReindexLedger for RecordingLedger {
        async fn rebuild_finished(&self, tenant: &TenantContext, manifest_id: &str) {
            self.cleared
                .lock()
                .push((tenant.tenant_id().to_string(), manifest_id.to_string()));
        }
    }

    fn manifest_context(manifest_id: &str) -> DeferredReindexContext {
        DeferredReindexContext {
            submission_id: Some("submission-1213".to_string()),
            manifest_id: Some(manifest_id.to_string()),
        }
    }

    async fn await_write_calls(backend: &ControlledBackend, count: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while backend.write_calls.load(Ordering::SeqCst) < count {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("controlled backend did not reach the expected writes");
    }

    /// #1213: manifests merged into one pending generation each owe a
    /// rebuild, and that generation clears every one of their markers — not
    /// only the marker of the last request merged into it.
    #[tokio::test]
    async fn merged_manifests_all_clear_their_markers() {
        let (backend, mut events) = ControlledBackend::new(Vec::new(), 0);
        let op = controlled_operation(backend.clone());
        let ledger = Arc::new(RecordingLedger::default());
        let hook = ReindexOnFinish::new(op.clone()).with_ledger(ledger.clone());
        let tenant = named_tenant("ledger-merge");

        hook.reindex_types_with_context(
            &tenant,
            vec!["Patient".to_string()],
            manifest_context("m-1"),
        )
        .await;
        await_controlled_write(&mut events, "ledger-merge", "Patient").await;
        // Generation 0 is running, so these two merge into generation 1.
        hook.reindex_types_with_context(
            &tenant,
            vec!["Observation".to_string()],
            manifest_context("m-2"),
        )
        .await;
        hook.reindex_types_with_context(
            &tenant,
            vec!["Condition".to_string()],
            manifest_context("m-3"),
        )
        .await;
        assert!(ledger.cleared().is_empty());

        backend.write_gate.add_permits(1);
        assert_eq!(ledger.await_cleared(1).await, vec!["m-1"]);
        backend.write_gate.add_permits(2);
        await_automatic_idle(&op).await;

        assert_eq!(ledger.await_cleared(3).await, vec!["m-1", "m-2", "m-3"]);
        assert_eq!(op.list_jobs().len(), 2, "m-2 and m-3 shared one generation");
        assert!(
            ledger
                .cleared
                .lock()
                .iter()
                .all(|(tenant, _)| tenant == "ledger-merge")
        );
    }

    /// A generation that fails and queues a retry hands its markers to the
    /// retry: none is cleared by the failure, and all are once the retry
    /// ends clean.
    #[tokio::test]
    async fn owed_manifests_survive_a_retried_failure() {
        let (backend, mut events) = ControlledBackend::new(Vec::new(), 0);
        let op = controlled_operation(backend.clone());
        let ledger = Arc::new(RecordingLedger::default());
        let hook = ReindexOnFinish::new(op.clone()).with_ledger(ledger.clone());
        let tenant = named_tenant("ledger-retry");

        hook.reindex_types_with_context(
            &tenant,
            vec!["Patient".to_string()],
            manifest_context("m-0"),
        )
        .await;
        await_controlled_write(&mut events, "ledger-retry", "Patient").await;
        hook.reindex_types_with_context(
            &tenant,
            vec!["Observation".to_string()],
            manifest_context("m-1"),
        )
        .await;
        hook.reindex_types_with_context(
            &tenant,
            vec!["Condition".to_string()],
            manifest_context("m-2"),
        )
        .await;

        backend.write_gate.add_permits(1);
        assert_eq!(ledger.await_cleared(1).await, vec!["m-0"]);

        // The merged generation's first write fails transiently, so it ends
        // Failed and queues a retry of only that resource.
        backend.failing_writes.store(1, Ordering::SeqCst);
        backend.write_gate.add_permits(2);
        await_write_calls(&backend, 4).await;
        tokio::task::yield_now().await;
        assert_eq!(
            ledger.cleared(),
            vec!["m-0"],
            "a failed generation must not clear the markers it owed"
        );

        backend.write_gate.add_permits(1);
        await_automatic_idle(&op).await;
        assert_eq!(ledger.await_cleared(3).await, vec!["m-0", "m-1", "m-2"]);
        assert_eq!(op.list_jobs().len(), 3);
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
        // but are still failures for automatic retry policy. The retry covers
        // only the failed resource: it is written again, but its type is not
        // counted (paged) a second time, and it is not retried a third time.
        let (backend, mut events) = ControlledBackend::new(Vec::new(), 2);
        let op = controlled_operation(backend.clone());
        let hook = ReindexOnFinish::new(op.clone());
        hook.reindex_types(
            &named_tenant("errorful-completion"),
            vec!["Patient".to_string()],
        )
        .await;
        await_controlled_write(&mut events, "errorful-completion", "Patient").await;
        backend.write_gate.add_permits(1);
        await_controlled_retry_write(&mut events, "errorful-completion", "Patient").await;
        backend.write_gate.add_permits(1);
        await_automatic_idle(&op).await;
        assert_eq!(backend.count_calls.load(Ordering::SeqCst), 1);
        assert_eq!(backend.write_calls.load(Ordering::SeqCst), 2);
        let mut jobs = op.list_jobs();
        jobs.sort_by_key(|job| job.total_resources);
        assert_eq!(jobs.len(), 2);
        assert!(
            jobs.iter()
                .all(|job| job.status == ReindexStatus::Completed)
        );
        assert!(
            jobs.iter()
                .all(|job| job.errors.len() == 1 && job.errors[0].resource_id == "controlled-1")
        );
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

        // One transient error among permanent ones earns the retry — of the
        // transient resources only, each once even when several writers failed
        // on it, while the log names every failing resource.
        progress.errors.push(error("flaky", true));
        progress.errors.push(error("flaky", true));
        progress.errors.push(error("blip", true));
        match automatic_outcome(Some(progress.clone())) {
            AutomaticGenerationOutcome::Failed {
                summary,
                resources,
                retry,
            } => {
                assert!(summary.contains("4 resource errors"), "{summary}");
                assert_eq!(
                    resources,
                    "Provenance/big, Provenance/flaky, Provenance/blip"
                );
                assert_eq!(
                    retry,
                    RetryScope::Resources(vec![
                        ResourceRef::new("Provenance", "blip"),
                        ResourceRef::new("Provenance", "flaky"),
                    ])
                );
            }
            other => panic!("expected a failure, got {other:?}"),
        }

        // Past the cap the retry rebuilds the failing types instead of holding
        // and fetching every failed resource by id.
        let mut many = progress.clone();
        many.errors
            .extend((0..=MAX_RETRY_RESOURCES).map(|n| ReindexProgressError {
                resource_type: "Observation".to_string(),
                resource_id: format!("o{n}"),
                error: String::new(),
                retryable: true,
            }));
        match automatic_outcome(Some(many)) {
            AutomaticGenerationOutcome::Failed { retry, .. } => assert_eq!(
                retry,
                RetryScope::Types(vec!["Observation".to_string(), "Provenance".to_string()])
            ),
            other => panic!("expected a failure, got {other:?}"),
        }

        // A job that failed as a whole is retried whatever its resource
        // errors: it may have stopped before most of its resources.
        progress.errors.truncate(2);
        progress.status = ReindexStatus::Failed;
        progress.error_message = Some("Failed to fetch resources".to_string());
        assert!(matches!(
            automatic_outcome(Some(progress)),
            AutomaticGenerationOutcome::Failed {
                retry: RetryScope::Generation,
                ..
            }
        ));
        assert!(matches!(
            automatic_outcome(None),
            AutomaticGenerationOutcome::Failed {
                retry: RetryScope::Generation,
                ..
            }
        ));
    }

    #[test]
    fn logged_resources_names_the_first_distinct_failures() {
        let errors: Vec<ReindexProgressError> = ["a", "a", "b", "c", "d", "e", "f"]
            .iter()
            .map(|id| ReindexProgressError {
                resource_type: "Provenance".to_string(),
                resource_id: id.to_string(),
                error: String::new(),
                retryable: true,
            })
            .collect();
        assert_eq!(
            logged_resources(&errors),
            "Provenance/a, Provenance/b, Provenance/c, Provenance/d, Provenance/e"
        );
    }

    #[test]
    fn push_error_keeps_every_id_but_bounds_the_messages() {
        let jobs: Arc<ReindexJobs> = Arc::default();
        jobs.write()
            .insert("job".to_string(), ReindexProgress::new("job"));
        let total = MAX_REPORTED_RESOURCE_ERRORS + 7;
        for n in 0..total {
            push_error(
                &jobs,
                "job",
                "Provenance",
                &format!("p{n}"),
                format!("failed p{n}"),
                n % 2 == 0,
            );
        }
        let progress = jobs.read().get("job").cloned().unwrap();
        assert_eq!(progress.errors.len(), total);
        for (n, error) in progress.errors.iter().enumerate() {
            assert_eq!(error.resource_id, format!("p{n}"));
            assert_eq!(error.retryable, n % 2 == 0);
            if n < MAX_REPORTED_RESOURCE_ERRORS {
                assert_eq!(error.error, format!("failed p{n}"));
            } else {
                assert!(error.error.is_empty(), "message kept for error {n}");
            }
        }
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

        // The first failure schedules one retry of the failed resource. Queue
        // unrelated work while that retry is in flight, then make the retry
        // fail as well.
        await_controlled_retry_write(&mut events, "failed-retry-with-pending", "Patient").await;
        hook.reindex_types(&tenant, vec!["Observation".to_string()])
            .await;
        backend.write_gate.add_permits(1);

        // Exhausting Patient's retry must not discard the later callback.
        await_controlled_write(&mut events, "failed-retry-with-pending", "Observation").await;
        backend.write_gate.add_permits(1);
        await_automatic_idle(&op).await;

        assert_eq!(backend.count_calls.load(Ordering::SeqCst), 2);
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

    /// A source holding `Patient/p0..p{n}` that pages by position and counts
    /// the pages it serves.
    struct PagedSource {
        ids: Vec<String>,
        pages: std::sync::atomic::AtomicUsize,
    }

    impl PagedSource {
        fn new(n: usize) -> Self {
            Self {
                ids: (0..n).map(|i| format!("p{i}")).collect(),
                pages: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl ReindexSource for PagedSource {
        async fn list_resource_types(&self, _: &TenantContext) -> StorageResult<Vec<String>> {
            Ok(vec!["Patient".to_string()])
        }

        async fn count_resources(&self, _: &TenantContext, _: &str) -> StorageResult<u64> {
            Ok(self.ids.len() as u64)
        }

        async fn fetch_resources_page(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            cursor: Option<&str>,
            limit: u32,
        ) -> StorageResult<ResourcePage> {
            self.pages.fetch_add(1, Ordering::SeqCst);
            let start: usize = cursor.map_or(0, |c| c.parse().unwrap());
            let end = (start + limit as usize).min(self.ids.len());
            Ok(ResourcePage {
                resources: self.ids[start..end]
                    .iter()
                    .map(|id| {
                        StoredResource::new(
                            resource_type,
                            id,
                            tenant.tenant_id().clone(),
                            serde_json::json!({"resourceType": resource_type, "id": id}),
                            helios_fhir::FhirVersion::default(),
                        )
                    })
                    .collect(),
                next_cursor: (end < self.ids.len()).then(|| end.to_string()),
                skipped: Vec::new(),
            })
        }
    }

    /// A cancelled run stops at the *page boundary*: the page that was in
    /// flight when the cancellation arrived is written in full, and no
    /// following page is fetched — so none of its resources reaches a
    /// statement.
    ///
    /// `ReindexOperation::cancel` marks the job `Cancelled` synchronously, so
    /// that status is terminal while the page it interrupted is still running;
    /// the signal that the task itself has stopped is its cancellation channel
    /// being released, which is what this waits for before counting the
    /// source's pages.
    #[tokio::test]
    async fn cancellation_during_a_page_finishes_it_and_stops_before_the_next_fetch() {
        let (backend, mut events) = ControlledBackend::new(Vec::new(), 0);
        let source = Arc::new(PagedSource::new(9));
        let source_port: Arc<dyn ReindexSource> = source.clone();
        let writers: Vec<Arc<dyn ReindexTarget>> = vec![backend.clone()];
        let op = Arc::new(ReindexOperation::with_parts(
            source_port,
            writers,
            Arc::new(crate::search::TenantSearchRegistries::base_only()),
        ));
        let tenant = named_tenant("cancelled-page");
        let tenant_id = tenant.tenant_id().to_string();

        let job_id = op
            .start(
                tenant.clone(),
                ReindexRequest::for_types(vec!["Patient".to_string()]).with_batch_size(2),
                None,
            )
            .await
            .expect("start the reindex");

        // Page one is in flight and blocked inside its first resource's write.
        assert_eq!(
            next_controlled_event(&mut events).await,
            ControlledEvent::Write {
                tenant: tenant_id.clone(),
                resource_type: "Patient".to_string(),
            }
        );
        assert_eq!(source.pages.load(Ordering::SeqCst), 1);

        op.cancel(&job_id).await.expect("cancel the job");
        assert_eq!(
            op.get_progress(&job_id).await.expect("progress").status,
            ReindexStatus::Cancelled
        );
        assert!(
            op.cancel_channels.read().contains_key(&job_id),
            "`cancel` writes terminal status, not task exit: the page is still running"
        );
        assert_eq!(
            backend.write_calls.load(Ordering::SeqCst),
            1,
            "the page in flight is mid-way through its resources"
        );

        // Let the in-flight page finish — two resources, two gated writes.
        backend.write_gate.add_permits(2);

        // The task releases its cancellation channel when it returns, which is
        // the only signal that it has stopped for good.
        tokio::time::timeout(Duration::from_secs(2), async {
            while op.cancel_channels.read().contains_key(&job_id) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the cancelled reindex task did not return");

        assert_eq!(
            backend.write_calls.load(Ordering::SeqCst),
            2,
            "the page that was in flight when the cancellation arrived finished"
        );
        assert_eq!(
            source.pages.load(Ordering::SeqCst),
            1,
            "the driver fetched a page after cancellation"
        );
        assert_eq!(
            op.get_progress(&job_id).await.expect("progress").status,
            ReindexStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn fetch_resources_by_ids_default_scans_pages_and_stops_when_all_found() {
        let tenant = named_tenant("by-ids");
        let source = PagedSource::new(FETCH_BY_IDS_SCAN_PAGE as usize * 3);

        // Ids on the first and second page: the third page is never read.
        let wanted = vec!["p1".to_string(), format!("p{}", FETCH_BY_IDS_SCAN_PAGE + 5)];
        let mut found: Vec<String> = source
            .fetch_resources_by_ids(&tenant, "Patient", &wanted)
            .await
            .unwrap()
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        found.sort();
        let mut expected = wanted.clone();
        expected.sort();
        assert_eq!(found, expected);
        assert_eq!(source.pages.load(Ordering::SeqCst), 2);

        // An id that no longer exists is absent, and the scan ends at the last page.
        let source = PagedSource::new(3);
        let found = source
            .fetch_resources_by_ids(&tenant, "Patient", &["p2".to_string(), "gone".to_string()])
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id(), "p2");

        // No ids, no reads.
        let source = PagedSource::new(3);
        assert!(
            source
                .fetch_resources_by_ids(&tenant, "Patient", &[])
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(source.pages.load(Ordering::SeqCst), 0);
    }

    /// A source holding `Patient/p0..p{n}` that pages by position and records
    /// when each page was served, so a test can measure the gap the driver
    /// leaves between two page writes.
    struct TimedPageSource {
        ids: Vec<String>,
        fetched_at: parking_lot::Mutex<Vec<Instant>>,
    }

    impl TimedPageSource {
        fn new(n: usize) -> Self {
            Self {
                ids: (0..n).map(|i| format!("p{i}")).collect(),
                fetched_at: parking_lot::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl ReindexSource for TimedPageSource {
        async fn list_resource_types(&self, _: &TenantContext) -> StorageResult<Vec<String>> {
            Ok(vec!["Patient".to_string()])
        }

        async fn count_resources(&self, _: &TenantContext, _: &str) -> StorageResult<u64> {
            Ok(self.ids.len() as u64)
        }

        async fn fetch_resources_page(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            cursor: Option<&str>,
            limit: u32,
        ) -> StorageResult<ResourcePage> {
            self.fetched_at.lock().push(Instant::now());
            let start: usize = cursor.map_or(0, |c| c.parse().unwrap());
            let end = (start + limit as usize).min(self.ids.len());
            Ok(ResourcePage {
                resources: self.ids[start..end]
                    .iter()
                    .map(|id| {
                        StoredResource::new(
                            resource_type,
                            id,
                            tenant.tenant_id().clone(),
                            serde_json::json!({"resourceType": resource_type, "id": id}),
                            helios_fhir::FhirVersion::default(),
                        )
                    })
                    .collect(),
                next_cursor: (end < self.ids.len()).then(|| end.to_string()),
                skipped: Vec::new(),
            })
        }
    }

    /// The driver must not re-take the storage write lock the instant it let
    /// go of it. Page writes are back-to-back `IMMEDIATE` transactions on the
    /// SQLite backend, and with no gap between them a foreground writer parked
    /// on the write lock waits out its whole `busy_timeout` and fails with
    /// "database is locked" (#1185).
    ///
    /// The mirror property — no pause after the *last* page — is structural
    /// rather than timed: the `None` arm of the cursor match breaks out of the
    /// loop before the yield is reached.
    #[tokio::test]
    async fn pages_are_written_with_a_gap_a_foreground_writer_can_use() {
        let source = Arc::new(TimedPageSource::new(3));
        let target = Arc::new(RecordingTarget::default());
        let op = recording_operation(source.clone(), target.clone());

        let job = op
            .start(
                named_tenant("page-yield"),
                ReindexRequest::default().with_batch_size(1),
                None,
            )
            .await
            .unwrap();
        let progress = await_finished(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Completed);

        let fetched_at = source.fetched_at.lock().clone();
        assert_eq!(fetched_at.len(), 3, "expected one fetch per page");
        for pair in fetched_at.windows(2) {
            let gap = pair[1].duration_since(pair[0]);
            assert!(
                gap >= REINDEX_PAGE_YIELD,
                "consecutive pages must be separated by at least \
                 {REINDEX_PAGE_YIELD:?} so a foreground writer can take the \
                 lock, got {gap:?}"
            );
        }
    }

    /// Records the ids it is asked to write, failing some of them.
    #[derive(Default)]
    struct RecordingTarget {
        written: parking_lot::Mutex<Vec<String>>,
        /// Ids whose first write fails transiently.
        transient_once: parking_lot::Mutex<BTreeSet<String>>,
        /// Ids whose every write is rejected.
        permanent: BTreeSet<String>,
    }

    #[async_trait]
    impl ReindexTarget for RecordingTarget {
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
            resource: &StoredResource,
        ) -> StorageResult<usize> {
            let id = resource.id().to_string();
            self.written.lock().push(id.clone());
            if self.transient_once.lock().remove(&id) {
                return Err(crate::error::BackendError::Unavailable {
                    backend_name: "recording".into(),
                    message: format!("injected transient failure of {id}"),
                }
                .into());
            }
            if self.permanent.contains(&id) {
                return Err(crate::error::BackendError::Internal {
                    backend_name: "recording".into(),
                    message: format!("injected rejection of {id}"),
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

    fn recording_operation(
        source: Arc<dyn ReindexSource>,
        target: Arc<RecordingTarget>,
    ) -> Arc<ReindexOperation> {
        Arc::new(ReindexOperation::with_parts(
            source,
            vec![target],
            Arc::new(crate::search::TenantSearchRegistries::base_only()),
        ))
    }

    async fn await_finished(op: &ReindexOperation, id: &str) -> ReindexProgress {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(progress) = op.get_progress(id).await
                    && progress.status.is_finished()
                {
                    return progress;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("reindex job did not finish")
    }

    #[tokio::test]
    async fn a_run_scoped_to_resources_writes_only_those_resources() {
        let source = Arc::new(PagedSource::new(10));
        let target = Arc::new(RecordingTarget::default());
        let op = recording_operation(source.clone(), target.clone());

        let job = op
            .start(
                named_tenant("by-ids-run"),
                ReindexRequest::for_resources([
                    ResourceRef::new("Patient", "p7"),
                    ResourceRef::new("Patient", "p2"),
                    ResourceRef::new("Patient", "p2"),
                    ResourceRef::new("Patient", "deleted"),
                ])
                .with_batch_size(1),
                None,
            )
            .await
            .unwrap();
        let progress = await_finished(&op, &job).await;

        assert_eq!(progress.status, ReindexStatus::Completed);
        assert!(progress.errors.is_empty(), "{:?}", progress.errors);
        let mut written = target.written.lock().clone();
        written.sort();
        assert_eq!(written, vec!["p2".to_string(), "p7".to_string()]);
        // Three distinct ids named; the deleted one counts as processed with
        // nothing to index.
        assert_eq!(progress.total_resources, 3);
        assert_eq!(progress.processed_resources, 3);
        assert_eq!(progress.entries_created, 2);
    }

    /// Two pages of `Patient`; the first also reports a row it could not
    /// decode.
    struct SkippingSource;

    #[async_trait]
    impl ReindexSource for SkippingSource {
        async fn list_resource_types(&self, _: &TenantContext) -> StorageResult<Vec<String>> {
            Ok(vec!["Patient".to_string()])
        }

        async fn count_resources(&self, _: &TenantContext, _: &str) -> StorageResult<u64> {
            Ok(3)
        }

        async fn fetch_resources_page(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            cursor: Option<&str>,
            _: u32,
        ) -> StorageResult<ResourcePage> {
            let resource = |id: &str| {
                StoredResource::new(
                    resource_type,
                    id,
                    tenant.tenant_id().clone(),
                    serde_json::json!({"resourceType": resource_type, "id": id}),
                    helios_fhir::FhirVersion::default(),
                )
            };
            Ok(match cursor {
                None => ResourcePage {
                    resources: vec![resource("good-1")],
                    next_cursor: Some("page-2".to_string()),
                    skipped: vec![SkippedResource {
                        resource_id: "corrupt".to_string(),
                        reason: "invalid JSON".to_string(),
                    }],
                },
                Some(_) => ResourcePage {
                    resources: vec![resource("good-2")],
                    ..Default::default()
                },
            })
        }
    }

    #[tokio::test]
    async fn an_unreadable_row_is_a_permanent_error_and_paging_continues() {
        let target = Arc::new(RecordingTarget::default());
        let op = recording_operation(Arc::new(SkippingSource), target.clone());

        let job = op
            .start(named_tenant("skipped-rows"), ReindexRequest::all(), None)
            .await
            .unwrap();
        let progress = await_finished(&op, &job).await;

        assert_eq!(progress.status, ReindexStatus::Completed);
        assert_eq!(
            *target.written.lock(),
            vec!["good-1".to_string(), "good-2".to_string()]
        );
        assert_eq!(progress.processed_resources, 3);
        assert_eq!(progress.errors.len(), 1);
        let error = &progress.errors[0];
        assert_eq!(error.resource_type, "Patient");
        assert_eq!(error.resource_id, "corrupt");
        assert!(!error.retryable);
        assert!(error.error.contains("invalid JSON"), "{}", error.error);
        assert!(progress.has_only_permanent_errors());
    }

    #[tokio::test]
    async fn automatic_retry_covers_only_the_transiently_failed_resources() {
        let source = Arc::new(PagedSource::new(5));
        let target = Arc::new(RecordingTarget {
            transient_once: parking_lot::Mutex::new(BTreeSet::from([
                "p1".to_string(),
                "p3".to_string(),
            ])),
            permanent: BTreeSet::from(["p4".to_string()]),
            ..Default::default()
        });
        let op = recording_operation(source, target.clone());

        ReindexOnFinish::new(op.clone())
            .reindex_types(&named_tenant("retry-by-id"), vec!["Patient".to_string()])
            .await;
        await_automatic_idle(&op).await;

        // Generation 0 writes the whole type; the retry writes the two
        // resources that failed transiently — not the rejected p4, and not the
        // three that indexed.
        let written = target.written.lock().clone();
        assert_eq!(written.len(), 7, "{written:?}");
        assert_eq!(&written[..5], ["p0", "p1", "p2", "p3", "p4"]);
        let mut retried = written[5..].to_vec();
        retried.sort();
        assert_eq!(retried, ["p1", "p3"]);

        let mut jobs = op.list_jobs();
        jobs.sort_by_key(|job| std::cmp::Reverse(job.total_resources));
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].total_resources, 5);
        assert_eq!(jobs[0].errors.len(), 3);
        assert_eq!(jobs[1].total_resources, 2);
        assert!(jobs[1].errors.is_empty(), "{:?}", jobs[1].errors);
        assert!(
            jobs.iter()
                .all(|job| job.status == ReindexStatus::Completed)
        );
    }

    #[tokio::test]
    async fn a_failed_resource_retry_is_not_retried_again() {
        let source = Arc::new(PagedSource::new(3));
        let target = Arc::new(RecordingTarget::default());
        // p1 fails transiently on both attempts.
        target.transient_once.lock().insert("p1".to_string());
        struct TwiceFailing(Arc<RecordingTarget>);
        #[async_trait]
        impl ReindexTarget for TwiceFailing {
            async fn delete_search_entries(
                &self,
                tenant: &TenantContext,
                resource_type: &str,
                id: &str,
            ) -> StorageResult<u64> {
                self.0
                    .delete_search_entries(tenant, resource_type, id)
                    .await
            }
            async fn write_search_entries(
                &self,
                tenant: &TenantContext,
                resource: &StoredResource,
            ) -> StorageResult<usize> {
                let outcome = self.0.write_search_entries(tenant, resource).await;
                if resource.id() == "p1" {
                    self.0.transient_once.lock().insert("p1".to_string());
                }
                outcome
            }
            async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
                self.0.clear_search_index(tenant).await
            }
        }
        let op = Arc::new(ReindexOperation::with_parts(
            source,
            vec![Arc::new(TwiceFailing(target.clone()))],
            Arc::new(crate::search::TenantSearchRegistries::base_only()),
        ));

        ReindexOnFinish::new(op.clone())
            .reindex_types(
                &named_tenant("retry-exhausted"),
                vec!["Patient".to_string()],
            )
            .await;
        await_automatic_idle(&op).await;

        let written = target.written.lock().clone();
        assert_eq!(written, ["p0", "p1", "p2", "p1"]);
        assert_eq!(op.list_jobs().len(), 2);
    }

    #[test]
    fn reindex_request_for_resources_scopes_types_and_ids() {
        let request = ReindexRequest::for_resources([
            ResourceRef::new("Provenance", "b"),
            ResourceRef::new("Patient", "a"),
            ResourceRef::new("Provenance", "c"),
        ]);
        assert_eq!(
            request.resource_types,
            Some(vec!["Patient".to_string(), "Provenance".to_string()])
        );
        assert_eq!(request.resource_ids.as_ref().map(Vec::len), Some(3));
        assert_eq!(request.batch_size, default_batch_size());

        // A request serialized before the field existed still deserializes.
        let legacy: ReindexRequest = serde_json::from_value(
            serde_json::json!({"resource_types": null, "search_param_urls": null}),
        )
        .unwrap();
        assert!(legacy.resource_ids.is_none());
        assert!(ReindexRequest::default().resource_ids.is_none());
        assert!(ResourcePage::default().skipped.is_empty());
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
    fn test_progress_to_parameters_reports_timestamps_once_set() {
        let value = |params: &serde_json::Value, name: &str| {
            params["parameter"]
                .as_array()
                .unwrap()
                .iter()
                .find(|parameter| parameter["name"] == name)
                .map(|parameter| parameter["valueDateTime"].clone())
        };
        let mut progress = ReindexProgress::new("job-123");
        let params = progress.to_parameters();
        assert_eq!(value(&params, "startedAt"), None);
        assert_eq!(value(&params, "completedAt"), None);

        progress.started_at = Some("2026-09-25T10:00:00+00:00".to_string());
        let params = progress.to_parameters();
        assert_eq!(
            value(&params, "startedAt"),
            Some(serde_json::json!("2026-09-25T10:00:00+00:00"))
        );
        assert_eq!(value(&params, "completedAt"), None);

        progress.completed_at = Some("2026-09-25T10:02:25+00:00".to_string());
        let params = progress.to_parameters();
        assert_eq!(
            value(&params, "completedAt"),
            Some(serde_json::json!("2026-09-25T10:02:25+00:00"))
        );
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
