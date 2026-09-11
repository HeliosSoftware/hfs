//! Worker-facing traits for bulk export job execution.
//!
//! These traits are *not* part of the REST-facing [`BulkExportStorage`] surface
//! — they are what the export worker uses to claim jobs and persist progress
//! under a heartbeated, fencing-token-guarded lease.
//!
//! [`BulkExportStorage`]: crate::core::bulk_export::BulkExportStorage

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::core::bulk_export::{
    BulkExportStorage, ExportDataProvider, ExportJobId, ExportLevel, ExportRequest, ExportStatus,
    GroupExportProvider, PatientExportProvider, TypeExportProgress,
};
use crate::core::bulk_export_output::{ExportOutputStore, ExportPartKey, FinalizedPart};
use crate::core::search::SearchProvider;
use crate::error::{BulkExportError, StorageError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

/// Identifier for an export worker instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkerId(String);

impl WorkerId {
    /// Creates a worker ID from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Generates a fresh random worker ID.
    pub fn random() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// Returns the ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for WorkerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A lease over a single export job, held by exactly one worker at a time.
///
/// Leases expire; if the holding worker does not heartbeat before
/// `lease_expiry`, the lease is reclaimable. The `fencing_token` is bumped on
/// every claim so a zombie worker cannot mutate a job another worker now owns.
#[derive(Debug, Clone)]
pub struct ExportJobLease {
    /// The leased job.
    pub job_id: ExportJobId,
    /// The tenant the job belongs to.
    pub tenant: TenantContext,
    /// The worker holding the lease.
    pub worker_id: WorkerId,
    /// When the lease expires if not renewed.
    pub lease_expiry: DateTime<Utc>,
    /// Monotonically increasing token, bumped on every claim.
    pub fencing_token: u64,
}

/// Error returned by fenced worker-storage operations.
#[derive(Debug)]
pub enum LeaseError {
    /// The lease was lost — another worker reclaimed the job. The caller MUST
    /// stop writing immediately.
    LeaseLost {
        /// The job whose lease was lost.
        job_id: ExportJobId,
    },
    /// An underlying storage error.
    Storage(StorageError),
}

impl std::fmt::Display for LeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LeaseLost { job_id } => {
                write!(
                    f,
                    "export job {job_id} lease lost (reclaimed by another worker)"
                )
            }
            Self::Storage(e) => write!(f, "storage error: {e}"),
        }
    }
}

impl std::error::Error for LeaseError {}

impl From<StorageError> for LeaseError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

/// The worker's view of a claimed job: everything needed to (re)run it.
#[derive(Debug, Clone)]
pub struct WorkerJobView {
    /// The original export request.
    pub request: ExportRequest,
    /// The export level.
    pub level: ExportLevel,
    /// Server wall-clock frozen at kickoff.
    pub transaction_time: DateTime<Utc>,
    /// The FHIR version the export runs against.
    pub fhir_version: helios_fhir::FhirVersion,
    /// Already-persisted per-type progress, for resuming after a crash.
    pub type_progress: Vec<TypeExportProgress>,
    /// The authenticated principal that kicked the export off, if any.
    ///
    /// Read from the job row rather than passed in, so the terminal audit event
    /// still names the requester when the job is picked up by a different worker
    /// (or a different process) than the one that accepted it.
    pub owner_subject: Option<String>,
}

/// Strategy for atomically claiming the next available export job.
///
/// Each backend reaches for its native primitive — `SELECT … FOR UPDATE SKIP
/// LOCKED` on Postgres, a process-local mutex on SQLite.
#[async_trait]
pub trait ExportClaimStrategy: Send + Sync {
    /// Atomically transitions one eligible job (`accepted`, or `in_progress`
    /// with an expired lease) to held-by-this-worker, bumping the fencing
    /// token. Returns `Ok(None)` when no job is available.
    async fn claim_next(
        &self,
        worker_id: &WorkerId,
        lease_duration: Duration,
    ) -> StorageResult<Option<ExportJobLease>>;

    /// Renews a lease the worker still holds; returns the new expiry, or
    /// `LeaseError::LeaseLost` if the job was reclaimed.
    async fn heartbeat(&self, lease: &ExportJobLease) -> Result<DateTime<Utc>, LeaseError>;

    /// Releases a lease early (graceful shutdown). Best-effort.
    async fn release(&self, lease: ExportJobLease) -> StorageResult<()>;
}

/// Worker-owned mutations of job state.
///
/// **Every method is fenced** by `worker_id` + `fencing_token`: a guarded
/// mutation affecting zero rows returns `LeaseError::LeaseLost`, so a zombie
/// worker cannot corrupt progress, file rows, or terminal status after its
/// job has been reclaimed.
#[async_trait]
pub trait ExportWorkerStorage: Send + Sync {
    /// Loads the claimed job's request, level, frozen metadata and persisted
    /// per-type progress (for resume). Fenced.
    async fn get_export_job_for_worker(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
    ) -> Result<WorkerJobView, LeaseError>;

    /// Marks the job `in_progress` (sets `started_at` if unset). Fenced.
    async fn mark_export_in_progress(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
    ) -> Result<(), LeaseError>;

    /// Idempotent upsert of per-type progress (cursor + counts). Fenced.
    async fn update_export_type_progress(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
        progress: &TypeExportProgress,
    ) -> Result<(), LeaseError>;

    /// Records which resource type the worker is about to write and how far
    /// through the type list it is. `current_type = None` clears the marker
    /// (the worker calls it that way once every type is done). Fenced.
    #[allow(clippy::too_many_arguments)]
    async fn set_export_current_type(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
        current_type: Option<&str>,
        types_done: u32,
        types_total: u32,
    ) -> Result<(), LeaseError>;

    /// Idempotent upsert of a finalized output/error file row. Fenced.
    async fn record_export_file(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
        part: &FinalizedPart,
        file_type: &str,
    ) -> Result<(), LeaseError>;

    /// Marks the job `complete` (sets `completed_at`). Fenced.
    async fn finish_export_job(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
    ) -> Result<(), LeaseError>;

    /// Marks the job `error` with a message (sets `completed_at`). Fenced.
    async fn fail_export_job(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
        error_message: &str,
    ) -> Result<(), LeaseError>;
}

/// Marker trait composing the three job-state surfaces a worker needs.
///
/// Only the SQLite and Postgres backends implement this; it is held as an
/// `Arc<dyn BulkExportJobStore>` and selected at bootstrap by
/// `HFS_BULK_EXPORT_BACKEND`.
pub trait BulkExportJobStore:
    BulkExportStorage + ExportWorkerStorage + ExportClaimStrategy
{
}

impl<T> BulkExportJobStore for T where
    T: BulkExportStorage + ExportWorkerStorage + ExportClaimStrategy
{
}

/// Marker trait for a resource-store that can feed every export level.
///
/// The `SearchProvider` bound lets the worker intersect each fetched batch
/// with a compiled `_typeFilter` through the same search path a normal FHIR
/// search uses (see [`DefaultExportWorker::filter_batch_lines`]), so
/// composite deployments filter through whichever backend actually indexes
/// search (e.g. Elasticsearch) rather than the primary store alone.
pub trait ExportResourceProvider:
    ExportDataProvider + PatientExportProvider + GroupExportProvider + SearchProvider
{
}

impl<T> ExportResourceProvider for T where
    T: ExportDataProvider + PatientExportProvider + GroupExportProvider + SearchProvider
{
}

/// The default in-process export worker.
///
/// Binds a [`BulkExportJobStore`] (job state + claim + worker storage), an
/// [`ExportResourceProvider`] (the resource store), and an
/// [`ExportOutputStore`] (where NDJSON bytes go), and drives a claimed job to
/// completion under its lease.
pub struct DefaultExportWorker<Js: ?Sized, Dp: ?Sized, Os: ?Sized> {
    /// Job-state store (claim, worker storage, lifecycle).
    pub jobs: Arc<Js>,
    /// Resource data provider.
    pub data: Arc<Dp>,
    /// Output store for NDJSON parts.
    pub output: Arc<Os>,
    /// This worker's identifier.
    pub worker_id: WorkerId,
    /// Group-export `_since` toggle: when `true`, exclude resources from
    /// before `_since` for patients added to the Group after `_since`
    /// (using `Group.member.period.start`).
    pub exclude_since_newly_added: bool,
    /// Optional audit sink for the job's terminal lifecycle events.
    audit: Option<WorkerAudit>,
}

/// Where the worker's `AuditEvent`s are sent.
#[derive(Clone)]
struct WorkerAudit {
    sink: Arc<dyn helios_audit::AuditSink>,
    source_observer: String,
}

impl<Js, Dp, Os> DefaultExportWorker<Js, Dp, Os>
where
    Js: BulkExportJobStore + ?Sized,
    Dp: ExportResourceProvider + ?Sized,
    Os: ExportOutputStore + ?Sized,
{
    /// Creates a new worker (defaults to `since_newly_added=include`).
    pub fn new(jobs: Arc<Js>, data: Arc<Dp>, output: Arc<Os>, worker_id: WorkerId) -> Self {
        Self {
            jobs,
            data,
            output,
            worker_id,
            exclude_since_newly_added: false,
            audit: None,
        }
    }

    /// Sets the `since_newly_added=exclude` toggle for Group exports.
    pub fn with_exclude_since_newly_added(mut self, exclude: bool) -> Self {
        self.exclude_since_newly_added = exclude;
        self
    }

    /// Emits a BALP `AuditEvent` when a job reaches a terminal state.
    ///
    /// The REST layer audits the kick-off, but only the worker knows how the
    /// job actually ended — so without this, an export that ran for an hour and
    /// then failed is indistinguishable in the audit log from one that
    /// completed.
    pub fn with_audit(
        mut self,
        sink: Arc<dyn helios_audit::AuditSink>,
        source_observer: impl Into<String>,
    ) -> Self {
        self.audit = Some(WorkerAudit {
            sink,
            source_observer: source_observer.into(),
        });
        self
    }

    /// Records a terminal lifecycle event for a job.
    async fn emit_audit(
        &self,
        job_id: &ExportJobId,
        view: Option<&WorkerJobView>,
        phase: &str,
        outcome: &str,
        outcome_desc: Option<&str>,
    ) {
        let Some(audit) = &self.audit else {
            return;
        };
        let level = view.map(|v| v.level.clone()).unwrap_or(ExportLevel::System);
        let types = view
            .map(|v| v.request.resource_types.clone())
            .unwrap_or_default();

        crate::core::bulk_export::audit::record_export_event(
            audit.sink.as_ref(),
            &audit.source_observer,
            view.and_then(|v| v.owner_subject.as_deref()),
            job_id.as_str(),
            phase,
            &level,
            &types,
            outcome,
            outcome_desc,
        )
        .await;
    }

    /// Runs the export job described by `lease` to completion.
    ///
    /// Every job-state mutation is fenced by `lease.worker_id` +
    /// `lease.fencing_token`; any `LeaseError::LeaseLost` aborts the run
    /// silently (the worker that reclaimed the job now owns it).
    pub async fn run_job(&self, lease: ExportJobLease) -> StorageResult<()> {
        // Captured by `run_job_inner` as soon as it loads the job, so a failure
        // partway through can still attribute its audit event to the principal
        // that requested the export.
        let mut view: Option<WorkerJobView> = None;

        match self.run_job_inner(&lease, &mut view).await {
            Ok(outcome) => {
                let (phase, code) = match outcome {
                    JobOutcome::Completed => ("complete", "0"),
                    JobOutcome::Cancelled => ("cancelled", "4"),
                };
                self.emit_audit(&lease.job_id, view.as_ref(), phase, code, None)
                    .await;
                Ok(())
            }
            Err(LeaseError::LeaseLost { .. }) => {
                // Another worker owns the job now — stop silently, and emit
                // nothing: the worker that reclaimed the job will record its
                // terminal event, and a second one here would double-count.
                Ok(())
            }
            Err(LeaseError::Storage(e)) => {
                // Best-effort: mark the job failed (also fenced).
                tracing::error!(job_id = %lease.job_id, error = %e, "export job failed");
                let public = public_failure_message(&e);
                let _ = self
                    .jobs
                    .fail_export_job(
                        &lease.tenant,
                        &lease.job_id,
                        &lease.worker_id,
                        lease.fencing_token,
                        &public,
                    )
                    .await;
                self.emit_audit(
                    &lease.job_id,
                    view.as_ref(),
                    "failed",
                    "8",
                    Some(&e.to_string()),
                )
                .await;
                Err(e)
            }
        }
    }

    async fn run_job_inner(
        &self,
        lease: &ExportJobLease,
        captured_view: &mut Option<WorkerJobView>,
    ) -> Result<JobOutcome, LeaseError> {
        let tenant = &lease.tenant;
        let job_id = &lease.job_id;
        let wid = &lease.worker_id;
        let token = lease.fencing_token;

        let view = self
            .jobs
            .get_export_job_for_worker(tenant, job_id, wid, token)
            .await?;
        // Handed back to `run_job` immediately, before any work that could
        // fail, so a failure event still carries the requester and the export
        // level rather than degrading to an anonymous "something failed".
        *captured_view = Some(view.clone());

        self.jobs
            .mark_export_in_progress(tenant, job_id, wid, token)
            .await?;

        let request = &view.request;

        // Resolve the resource types to export.
        let types = self
            .data
            .list_export_types(tenant, request)
            .await
            .map_err(LeaseError::Storage)?;

        // Every `_typeFilter` must carry the query compiled against the search
        // parameter registry at kick-off (T4). A filter persisted before that
        // compilation existed has no query to run, so the job fails here
        // rather than silently exporting an unfiltered set. When two filters
        // target the same resource type, the first one wins — the kick-off
        // path does not combine multiple filters for one type today.
        let mut filters: HashMap<&str, &SearchQuery> = HashMap::new();
        for tf in &request.type_filters {
            let Some(compiled) = tf.compiled.as_ref() else {
                return Err(LeaseError::Storage(StorageError::BulkExport(
                    BulkExportError::InvalidTypeFilter {
                        resource_type: tf.resource_type.clone(),
                        message: "the filter was stored without a compiled query; \
                                  re-submit the export"
                            .to_string(),
                    },
                )));
            };
            filters.entry(tf.resource_type.as_str()).or_insert(compiled);
        }

        // For Group exports, resolve the member patient IDs once.
        // When `exclude_since_newly_added` is set AND `_since` is provided,
        // filter out patients whose `Group.member.period.start` is *after*
        // `_since` (i.e., they joined the cohort after the client's last
        // sync) — the IG-recommended behavior under the `exclude` toggle.
        let group_patient_ids: Option<Vec<String>> = match &view.level {
            ExportLevel::Group { group_id } => {
                let ids = match (self.exclude_since_newly_added, view.request.since.as_ref()) {
                    (true, Some(since)) => {
                        let members = self
                            .data
                            .get_group_members_with_periods(tenant, group_id)
                            .await
                            .map_err(LeaseError::Storage)?;
                        members
                            .into_iter()
                            .filter_map(|(reference, period_start)| {
                                let pid = reference.strip_prefix("Patient/")?;
                                // Keep members whose period.start is unknown OR
                                // <= since (i.e., were already members at since).
                                match period_start {
                                    Some(start) if start > *since => None,
                                    _ => Some(pid.to_string()),
                                }
                            })
                            .collect()
                    }
                    _ => self
                        .data
                        .resolve_group_patient_ids(tenant, group_id)
                        .await
                        .map_err(LeaseError::Storage)?,
                };
                Some(ids)
            }
            _ => None,
        };

        let batch_size = request.batch_size.max(1);
        let types_total = types.len() as u32;

        for (type_index, resource_type) in types.iter().enumerate() {
            self.jobs
                .set_export_current_type(
                    tenant,
                    job_id,
                    wid,
                    token,
                    Some(resource_type.as_str()),
                    type_index as u32,
                    types_total,
                )
                .await?;

            // Resume from any persisted cursor for this type.
            let mut cursor: Option<String> = view
                .type_progress
                .iter()
                .find(|p| &p.resource_type == resource_type)
                .and_then(|p| p.cursor_state.clone());
            let mut exported: u64 = view
                .type_progress
                .iter()
                .find(|p| &p.resource_type == resource_type)
                .map(|p| p.exported_count)
                .unwrap_or(0);
            let mut part_index: u32 = 0;

            loop {
                // Cooperative cancellation check.
                if let Ok(progress) = self.jobs.get_export_status(tenant, job_id).await {
                    if progress.status == ExportStatus::Cancelled {
                        return Ok(JobOutcome::Cancelled);
                    }
                }

                let batch = match &group_patient_ids {
                    Some(pids) => self
                        .data
                        .fetch_patient_compartment_batch(
                            tenant,
                            request,
                            resource_type,
                            pids,
                            cursor.as_deref(),
                            batch_size,
                        )
                        .await
                        .map_err(LeaseError::Storage)?,
                    None if matches!(view.level, ExportLevel::Patient)
                        && !request.patient_refs.is_empty() =>
                    {
                        // Patient-level with specific patient filter: scope to
                        // exactly the requested patients' compartments.
                        let patient_ids: Vec<String> = request
                            .patient_refs
                            .iter()
                            .map(|r| r.strip_prefix("Patient/").unwrap_or(r).to_string())
                            .collect();
                        self.data
                            .fetch_patient_compartment_batch(
                                tenant,
                                request,
                                resource_type,
                                &patient_ids,
                                cursor.as_deref(),
                                batch_size,
                            )
                            .await
                            .map_err(LeaseError::Storage)?
                    }
                    None if matches!(view.level, ExportLevel::Patient) => {
                        // Patient-level without a patient filter: export all
                        // resources of this type across the patient compartment.
                        self.data
                            .fetch_export_batch(
                                tenant,
                                request,
                                resource_type,
                                cursor.as_deref(),
                                batch_size,
                            )
                            .await
                            .map_err(LeaseError::Storage)?
                    }
                    None => self
                        .data
                        .fetch_export_batch(
                            tenant,
                            request,
                            resource_type,
                            cursor.as_deref(),
                            batch_size,
                        )
                        .await
                        .map_err(LeaseError::Storage)?,
                };

                // Intersect the batch with the compiled `_typeFilter` for this
                // type, if any. `cursor`, `is_last`, progress and the
                // heartbeat below all keep coming from `batch` — the traversal
                // itself is unaffected by filtering; only what gets written is.
                let lines = match filters.get(resource_type.as_str()) {
                    Some(filter) => self
                        .filter_batch_lines(tenant, resource_type, filter, batch.lines.clone())
                        .await
                        .map_err(LeaseError::Storage)?,
                    None => batch.lines.clone(),
                };

                if !lines.is_empty() {
                    let key = ExportPartKey::output(
                        tenant.tenant_id().as_str(),
                        job_id.clone(),
                        resource_type.clone(),
                        part_index,
                        token,
                    );
                    let mut writer = self
                        .output
                        .open_writer(&key)
                        .await
                        .map_err(LeaseError::Storage)?;
                    for line in &lines {
                        let out_line = apply_elements(line, &request.elements);
                        writer.write_line(&out_line).await.map_err(|e| {
                            LeaseError::Storage(StorageError::Backend(
                                crate::error::BackendError::Internal {
                                    backend_name: "export-worker".to_string(),
                                    message: format!("write_line: {e}"),
                                    source: None,
                                },
                            ))
                        })?;
                    }
                    let finalized = self
                        .output
                        .finalize_part(&key, writer)
                        .await
                        .map_err(LeaseError::Storage)?;
                    exported += finalized.line_count;
                    self.jobs
                        .record_export_file(tenant, job_id, wid, token, &finalized, "output")
                        .await?;
                    part_index += 1;
                }

                cursor = batch.next_cursor.clone();

                // Persist progress + heartbeat after each batch.
                let mut progress = TypeExportProgress::new(resource_type.clone());
                progress.exported_count = exported;
                progress.cursor_state = cursor.clone();
                self.jobs
                    .update_export_type_progress(tenant, job_id, wid, token, &progress)
                    .await?;
                self.jobs.heartbeat(lease).await?;

                if batch.is_last {
                    break;
                }
            }
        }

        self.jobs
            .set_export_current_type(tenant, job_id, wid, token, None, types_total, types_total)
            .await?;
        self.jobs
            .finish_export_job(tenant, job_id, wid, token)
            .await?;
        Ok(JobOutcome::Completed)
    }

    /// Keeps only the lines of `batch` whose resource matches `filter`.
    ///
    /// Runs `filter` with an `_id` OR-list of the batch's ids through the
    /// search provider, paging with the result cursor until every page is
    /// read, and drops the lines the search did not return. A line whose JSON
    /// has no `id` field cannot be matched at all and is dropped with a
    /// warning rather than kept unconditionally.
    async fn filter_batch_lines(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        filter: &SearchQuery,
        lines: Vec<String>,
    ) -> StorageResult<Vec<String>> {
        let ids_by_line: Vec<Option<String>> = lines
            .iter()
            .map(|line| {
                let id = serde_json::from_str::<serde_json::Value>(line)
                    .ok()
                    .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string));
                if id.is_none() {
                    tracing::warn!(
                        resource_type,
                        "export batch line has no id; dropping it from the type-filter check"
                    );
                }
                id
            })
            .collect();
        let ids: Vec<String> = ids_by_line.iter().flatten().cloned().collect();

        let mut query = filter.clone();
        query.resource_type = resource_type.to_string();
        query.parameters.push(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: ids.iter().map(|id| SearchValue::eq(id.clone())).collect(),
            chain: Vec::new(),
            components: Vec::new(),
        });
        query.count = Some(lines.len() as u32);
        query.cursor = None;
        query.total = None;
        query.summary = None;
        query.elements.clear();
        query.sort.clear();
        query.includes.clear();

        let mut matched: HashSet<String> = HashSet::new();
        loop {
            let result = self.data.search(tenant, &query).await?;
            matched.extend(result.resources.items.iter().map(|r| r.id().to_string()));
            match result.resources.page_info.next_cursor {
                Some(next) => query.cursor = Some(next),
                None => break,
            }
        }

        Ok(lines
            .into_iter()
            .zip(ids_by_line)
            .filter(|(_, id)| id.as_deref().is_some_and(|id| matched.contains(id)))
            .map(|(line, _)| line)
            .collect())
    }
}

/// How a worker's run of a job ended.
///
/// `run_job_inner` returns `Ok(())` for both a completed export and one that
/// was cancelled mid-flight, which made the two indistinguishable to the
/// caller — and so unauditable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobOutcome {
    /// Every requested type was exported and the job was finished.
    Completed,
    /// A cooperative cancellation check saw the job cancelled and stopped.
    Cancelled,
}

/// The failure text stored on the job and shown to the export's owner.
///
/// Domain errors (`BulkExportError`) describe the export itself and are
/// stored verbatim; anything else may carry backend detail (SQL, table
/// names, connection strings) and is replaced by a generic message, with
/// the full error kept in the server log by the caller.
fn public_failure_message(e: &StorageError) -> String {
    match e {
        StorageError::BulkExport(inner) => inner.to_string(),
        _ => "export failed: internal storage error".to_string(),
    }
}

/// Applies `_elements` projection to an NDJSON line.
///
/// When `elements` is non-empty, keeps `resourceType`, `id`, `meta` and the
/// listed top-level element names, and adds a `SUBSETTED` `meta.tag`. On any
/// parse failure the original line is returned unchanged.
fn apply_elements(line: &str, elements: &[String]) -> String {
    if elements.is_empty() {
        return line.to_string();
    }
    let Ok(serde_json::Value::Object(obj)) = serde_json::from_str::<serde_json::Value>(line) else {
        return line.to_string();
    };
    let mut out = serde_json::Map::new();
    // Always-included mandatory elements.
    for key in ["resourceType", "id"] {
        if let Some(v) = obj.get(key) {
            out.insert(key.to_string(), v.clone());
        }
    }
    // Requested top-level elements (strip a leading `ResourceType.` prefix).
    for el in elements {
        let name = el.rsplit('.').next().unwrap_or(el.as_str());
        if let Some(v) = obj.get(name) {
            out.insert(name.to_string(), v.clone());
        }
    }
    // meta + SUBSETTED tag.
    let mut meta = obj
        .get("meta")
        .and_then(|m| m.as_object().cloned())
        .unwrap_or_default();
    let tag = serde_json::json!({
        "system": "http://terminology.hl7.org/CodeSystem/v3-ObservationValue",
        "code": "SUBSETTED",
    });
    let tags = meta
        .entry("tag".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    if let serde_json::Value::Array(arr) = tags {
        arr.push(tag);
    }
    out.insert("meta".to_string(), serde_json::Value::Object(meta));
    serde_json::to_string(&serde_json::Value::Object(out)).unwrap_or_else(|_| line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_elements_noop_when_empty() {
        let line = r#"{"resourceType":"Patient","id":"1","name":[]}"#;
        assert_eq!(apply_elements(line, &[]), line);
    }

    #[test]
    fn test_apply_elements_subsets_and_tags() {
        let line = r#"{"resourceType":"Patient","id":"1","name":[{"family":"X"}],"gender":"male"}"#;
        let out = apply_elements(line, &["name".to_string()]);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["resourceType"], "Patient");
        assert_eq!(v["id"], "1");
        assert!(v.get("name").is_some());
        assert!(v.get("gender").is_none());
        assert_eq!(v["meta"]["tag"][0]["code"], "SUBSETTED");
    }

    #[test]
    fn test_public_failure_message_keeps_domain_errors() {
        use crate::error::BulkExportError;

        let err = StorageError::BulkExport(BulkExportError::GroupNotFound {
            group_id: "g1".to_string(),
        });
        let message = public_failure_message(&err);
        assert!(
            message.contains("g1"),
            "domain errors should be stored verbatim, got: {message}"
        );
    }

    #[test]
    fn test_public_failure_message_masks_backend_errors() {
        use crate::error::BackendError;

        let err = StorageError::Backend(BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: "SELECT * FROM secret".to_string(),
            source: None,
        });
        let message = public_failure_message(&err);
        assert_eq!(message, "export failed: internal storage error");
        assert!(!message.contains("SELECT"));
    }

    #[cfg(feature = "sqlite")]
    mod worker_integration {
        use super::*;
        use crate::backends::local_fs::LocalFsOutputStore;
        use crate::backends::sqlite::SqliteBackend;
        use crate::core::ResourceStorage;
        use crate::core::bulk_export::{ExportRequest, StartExportInput, TypeFilter};
        use crate::tenant::{TenantContext, TenantId, TenantPermissions};
        use chrono::Utc;
        use std::sync::Arc;

        fn tenant() -> TenantContext {
            TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
        }

        #[tokio::test]
        async fn test_run_job_system_export_end_to_end() {
            let backend = Arc::new(SqliteBackend::in_memory().unwrap());
            backend.init_schema().unwrap();
            let tenant = tenant();

            for i in 0..3 {
                backend
                    .create(
                        &tenant,
                        "Patient",
                        serde_json::json!({"resourceType": "Patient", "id": format!("p{i}")}),
                        helios_fhir::FhirVersion::default(),
                    )
                    .await
                    .unwrap();
            }

            let tmp = tempfile::tempdir().unwrap();
            let output = Arc::new(LocalFsOutputStore::new(tmp.path(), "http://localhost:8080"));

            let job_id = backend
                .start_export(
                    &tenant,
                    StartExportInput {
                        request: ExportRequest::system()
                            .with_types(vec!["Patient".to_string()])
                            .with_batch_size(2),
                        transaction_time: Utc::now(),
                        request_url: "http://localhost/$export".to_string(),
                        owner_subject: Some("sub".to_string()),
                        fhir_version: helios_fhir::FhirVersion::default(),
                    },
                )
                .await
                .unwrap();

            let worker_id = WorkerId::new("w1");
            let worker = DefaultExportWorker::new(
                Arc::clone(&backend),
                Arc::clone(&backend),
                Arc::clone(&output),
                worker_id.clone(),
            );

            let lease = backend
                .claim_next(&worker_id, Duration::from_secs(60))
                .await
                .unwrap()
                .expect("job claimable");
            assert_eq!(lease.job_id, job_id);

            worker.run_job(lease).await.unwrap();

            let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
            assert_eq!(progress.status, ExportStatus::Complete);
            // The worker clears the in-flight marker on completion and leaves
            // the counters showing every type as done (#961).
            assert_eq!(progress.current_type, None);
            assert_eq!(progress.types_total, 1);
            assert_eq!(progress.types_done, progress.types_total);

            let manifest = backend.get_export_manifest(&tenant, &job_id).await.unwrap();
            let total: u64 = manifest.output.iter().map(|e| e.count).sum();
            assert_eq!(total, 3);
        }

        /// Wraps [`LocalFsOutputStore`], recording the export status the worker
        /// sees when it opens the writer for the export's second resource type.
        ///
        /// (#961) `BulkExportJobStore` composes three traits
        /// (`BulkExportStorage + ExportWorkerStorage + ExportClaimStrategy`);
        /// wrapping it just to spy on `set_export_current_type` calls would mean
        /// delegating every method of all three to the inner SQLite backend.
        /// Wrapping `ExportOutputStore` instead (five methods) and reading
        /// `get_export_status` from inside `open_writer` is far cheaper and
        /// still proves the worker marks a type as current *before* it starts
        /// writing it.
        struct SecondTypeObserver {
            inner: Arc<LocalFsOutputStore>,
            backend: Arc<SqliteBackend>,
            tenant: TenantContext,
            second_type: String,
            observed: std::sync::Mutex<Option<(Option<String>, u32)>>,
        }

        #[async_trait::async_trait]
        impl ExportOutputStore for SecondTypeObserver {
            async fn open_writer(
                &self,
                key: &ExportPartKey,
            ) -> StorageResult<crate::core::bulk_export_output::ExportPartWriter> {
                if key.resource_type == self.second_type {
                    let already_observed = self.observed.lock().unwrap().is_some();
                    if !already_observed {
                        let status = self
                            .backend
                            .get_export_status(&self.tenant, &key.job_id)
                            .await?;
                        *self.observed.lock().unwrap() =
                            Some((status.current_type, status.types_done));
                    }
                }
                self.inner.open_writer(key).await
            }

            async fn finalize_part(
                &self,
                key: &ExportPartKey,
                writer: crate::core::bulk_export_output::ExportPartWriter,
            ) -> StorageResult<FinalizedPart> {
                self.inner.finalize_part(key, writer).await
            }

            async fn download_url(
                &self,
                key: &ExportPartKey,
                ttl: Duration,
            ) -> StorageResult<crate::core::bulk_export_output::DownloadUrl> {
                self.inner.download_url(key, ttl).await
            }

            async fn open_reader(
                &self,
                key: &ExportPartKey,
            ) -> StorageResult<std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>>> {
                self.inner.open_reader(key).await
            }

            async fn delete_job_outputs(
                &self,
                tenant: &TenantContext,
                job_id: &ExportJobId,
            ) -> StorageResult<()> {
                self.inner.delete_job_outputs(tenant, job_id).await
            }
        }

        /// The worker marks a type as current, with `types_done` reflecting how
        /// many types are already finished, before it writes the first line of
        /// that type — not after. See [`SecondTypeObserver`] for why this test
        /// observes the output store rather than the job store.
        #[tokio::test]
        async fn test_run_job_marks_each_type_in_order() {
            let backend = Arc::new(SqliteBackend::in_memory().unwrap());
            backend.init_schema().unwrap();
            let tenant = tenant();

            backend
                .create(
                    &tenant,
                    "Patient",
                    serde_json::json!({"resourceType": "Patient", "id": "p1"}),
                    helios_fhir::FhirVersion::default(),
                )
                .await
                .unwrap();
            backend
                .create(
                    &tenant,
                    "Observation",
                    serde_json::json!({"resourceType": "Observation", "id": "o1"}),
                    helios_fhir::FhirVersion::default(),
                )
                .await
                .unwrap();

            let tmp = tempfile::tempdir().unwrap();
            let inner = Arc::new(LocalFsOutputStore::new(tmp.path(), "http://localhost:8080"));
            let output = Arc::new(SecondTypeObserver {
                inner,
                backend: Arc::clone(&backend),
                tenant: tenant.clone(),
                second_type: "Observation".to_string(),
                observed: std::sync::Mutex::new(None),
            });

            let job_id = backend
                .start_export(
                    &tenant,
                    StartExportInput {
                        request: ExportRequest::system()
                            .with_types(vec!["Patient".to_string(), "Observation".to_string()]),
                        transaction_time: Utc::now(),
                        request_url: "http://localhost/$export".to_string(),
                        owner_subject: Some("sub".to_string()),
                        fhir_version: helios_fhir::FhirVersion::default(),
                    },
                )
                .await
                .unwrap();

            let worker_id = WorkerId::new("w-order");
            let worker = DefaultExportWorker::new(
                Arc::clone(&backend),
                Arc::clone(&backend),
                Arc::clone(&output),
                worker_id.clone(),
            );

            let lease = backend
                .claim_next(&worker_id, Duration::from_secs(60))
                .await
                .unwrap()
                .expect("job claimable");

            worker.run_job(lease).await.unwrap();

            let observed = output.observed.lock().unwrap().clone();
            assert_eq!(
                observed,
                Some((Some("Observation".to_string()), 1)),
                "the job must already show Observation as current, with 1 type \
                 done, by the time its writer opens"
            );

            let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
            assert_eq!(progress.current_type, None);
            assert_eq!(progress.types_done, 2);
            assert_eq!(progress.types_total, 2);
        }

        /// The REST layer audits the kick-off, but only the worker knows how a
        /// job actually ended. Without a terminal event, an export that ran and
        /// then failed is indistinguishable in the audit log from one that
        /// completed — which is precisely the gap issue #168 is about.
        #[tokio::test]
        async fn test_worker_emits_terminal_audit_event_on_completion() {
            use crate::test_audit::{CollectorSink, detail_map};

            let backend = Arc::new(SqliteBackend::in_memory().unwrap());
            backend.init_schema().unwrap();
            let tenant = tenant();

            backend
                .create(
                    &tenant,
                    "Patient",
                    serde_json::json!({"resourceType": "Patient", "id": "p1"}),
                    helios_fhir::FhirVersion::default(),
                )
                .await
                .unwrap();

            let tmp = tempfile::tempdir().unwrap();
            let output = Arc::new(LocalFsOutputStore::new(tmp.path(), "http://localhost:8080"));

            let job_id = backend
                .start_export(
                    &tenant,
                    StartExportInput {
                        request: ExportRequest::system().with_types(vec!["Patient".to_string()]),
                        transaction_time: Utc::now(),
                        request_url: "http://localhost/$export".to_string(),
                        owner_subject: Some("Practitioner/dr-1".to_string()),
                        fhir_version: helios_fhir::FhirVersion::default(),
                    },
                )
                .await
                .unwrap();

            let sink = Arc::new(CollectorSink::new());
            let worker_id = WorkerId::new("w-audit");
            let worker = DefaultExportWorker::new(
                Arc::clone(&backend),
                Arc::clone(&backend),
                Arc::clone(&output),
                worker_id.clone(),
            )
            .with_audit(sink.clone(), "Device/hfs");

            let lease = backend
                .claim_next(&worker_id, Duration::from_secs(60))
                .await
                .unwrap()
                .expect("job claimable");
            worker.run_job(lease).await.unwrap();

            let events = sink.events();
            assert_eq!(events.len(), 1, "exactly one terminal event per job run");

            let details = detail_map(&events[0]);
            assert_eq!(
                details.get("bulk-export-operation").map(String::as_str),
                Some("complete")
            );
            assert_eq!(
                details.get("job-id").map(String::as_str),
                Some(job_id.as_str())
            );
            assert_eq!(
                events[0].outcome.as_ref().and_then(|o| o.value.as_deref()),
                Some("0")
            );

            // The agent comes off the job row, not the request, so it survives
            // the job being picked up by a worker in another process.
            let agent = events[0].agent.as_ref().expect("event must have an agent");
            assert_eq!(
                agent[0]
                    .who
                    .as_ref()
                    .and_then(|w| w.reference.as_ref())
                    .and_then(|r| r.value.as_deref()),
                Some("Practitioner/dr-1")
            );
        }

        #[tokio::test]
        async fn test_run_job_stores_public_message_on_failure() {
            let backend = Arc::new(SqliteBackend::in_memory().unwrap());
            backend.init_schema().unwrap();
            let tenant = tenant();

            let tmp = tempfile::tempdir().unwrap();
            let output = Arc::new(LocalFsOutputStore::new(tmp.path(), "http://localhost:8080"));

            // No `Group/g-missing` was ever created, so resolving its members
            // during the run fails with a domain `GroupNotFound` error.
            let job_id = backend
                .start_export(
                    &tenant,
                    StartExportInput {
                        request: ExportRequest::group("g-missing")
                            .with_types(vec!["Patient".to_string()]),
                        transaction_time: Utc::now(),
                        request_url: "http://localhost/Group/g-missing/$export".to_string(),
                        owner_subject: Some("sub".to_string()),
                        fhir_version: helios_fhir::FhirVersion::default(),
                    },
                )
                .await
                .unwrap();

            let worker_id = WorkerId::new("w-fail");
            let worker = DefaultExportWorker::new(
                Arc::clone(&backend),
                Arc::clone(&backend),
                Arc::clone(&output),
                worker_id.clone(),
            );

            let lease = backend
                .claim_next(&worker_id, Duration::from_secs(60))
                .await
                .unwrap()
                .expect("job claimable");

            let result = worker.run_job(lease).await;
            assert!(result.is_err(), "run_job should surface the failure");

            let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
            assert_eq!(progress.status, ExportStatus::Error);
            let error_message = progress
                .error_message
                .expect("failed job should carry an error message");
            assert!(
                error_message.contains("g-missing"),
                "error message should name the missing group, got: {error_message}"
            );
        }

        /// A `_typeFilter` persisted before compiled queries existed (T4) has
        /// no query for the worker to run. `run_job` must refuse it outright
        /// rather than fall back to exporting the type unfiltered.
        #[tokio::test]
        async fn test_run_job_fails_before_writing_when_type_filter_is_not_compiled() {
            let backend = Arc::new(SqliteBackend::in_memory().unwrap());
            backend.init_schema().unwrap();
            let tenant = tenant();

            backend
                .create(
                    &tenant,
                    "Patient",
                    serde_json::json!({"resourceType": "Patient", "id": "p1", "active": true}),
                    helios_fhir::FhirVersion::default(),
                )
                .await
                .unwrap();

            let tmp = tempfile::tempdir().unwrap();
            let output = Arc::new(LocalFsOutputStore::new(tmp.path(), "http://localhost:8080"));

            // `TypeFilter::new` alone (without `.with_compiled`) mimics a job
            // that was persisted before the compiled-query field existed.
            let job_id = backend
                .start_export(
                    &tenant,
                    StartExportInput {
                        request: ExportRequest::system()
                            .with_types(vec!["Patient".to_string()])
                            .with_type_filter(TypeFilter::new("Patient", "active=true")),
                        transaction_time: Utc::now(),
                        request_url: "http://localhost/$export".to_string(),
                        owner_subject: Some("sub".to_string()),
                        fhir_version: helios_fhir::FhirVersion::default(),
                    },
                )
                .await
                .unwrap();

            let worker_id = WorkerId::new("w-uncompiled-filter");
            let worker = DefaultExportWorker::new(
                Arc::clone(&backend),
                Arc::clone(&backend),
                Arc::clone(&output),
                worker_id.clone(),
            );

            let lease = backend
                .claim_next(&worker_id, Duration::from_secs(60))
                .await
                .unwrap()
                .expect("job claimable");

            let result = worker.run_job(lease).await;
            assert!(
                result.is_err(),
                "run_job should refuse a filter without a compiled query"
            );

            let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
            assert_eq!(progress.status, ExportStatus::Error);
            let error_message = progress
                .error_message
                .expect("failed job should carry an error message");
            assert!(
                error_message.contains("re-submit"),
                "error message should tell the client to re-submit, got: {error_message}"
            );

            let manifest = backend.get_export_manifest(&tenant, &job_id).await.unwrap();
            assert!(
                manifest.output.is_empty(),
                "no output files should be recorded when the filter check fails before writing"
            );
        }

        /// `filter_batch_lines` should keep exactly the lines the search
        /// provider confirms match, in the batch's original order, paging
        /// through the search result until it is exhausted.
        #[tokio::test]
        async fn test_filter_batch_lines_pages_through_the_search_result() {
            // Point at the workspace's data directory so the search-parameter
            // registry loads the full FHIR spec — the minimal embedded set
            // used by `SqliteBackend::in_memory()` does not include `active`.
            let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("data");
            let config = crate::backends::sqlite::SqliteBackendConfig {
                data_dir: Some(data_dir),
                ..Default::default()
            };
            let backend = Arc::new(SqliteBackend::with_config(":memory:", config).unwrap());
            backend.init_schema().unwrap();
            let tenant = tenant();

            let mut lines = Vec::new();
            for i in 0..3 {
                let id = format!("p-active-{i}");
                let resource = serde_json::json!({
                    "resourceType": "Patient",
                    "id": id,
                    "active": true,
                });
                backend
                    .create(
                        &tenant,
                        "Patient",
                        resource.clone(),
                        helios_fhir::FhirVersion::default(),
                    )
                    .await
                    .unwrap();
                lines.push(resource.to_string());
            }
            let inactive = serde_json::json!({
                "resourceType": "Patient",
                "id": "p-inactive",
                "active": false,
            });
            backend
                .create(
                    &tenant,
                    "Patient",
                    inactive.clone(),
                    helios_fhir::FhirVersion::default(),
                )
                .await
                .unwrap();
            lines.push(inactive.to_string());

            let tmp = tempfile::tempdir().unwrap();
            let output = Arc::new(LocalFsOutputStore::new(tmp.path(), "http://localhost:8080"));
            let worker = DefaultExportWorker::new(
                Arc::clone(&backend),
                Arc::clone(&backend),
                Arc::clone(&output),
                WorkerId::new("w-filter-pages"),
            );

            let mut filter = SearchQuery::new("Patient");
            filter.parameters.push(SearchParameter {
                name: "active".to_string(),
                param_type: SearchParamType::Token,
                modifier: None,
                values: vec![SearchValue::boolean(true)],
                chain: Vec::new(),
                components: Vec::new(),
            });

            let filtered = worker
                .filter_batch_lines(&tenant, "Patient", &filter, lines.clone())
                .await
                .expect("filter_batch_lines");

            assert_eq!(
                filtered,
                lines[..3],
                "the 3 active lines, original order kept"
            );
        }
    }
}
