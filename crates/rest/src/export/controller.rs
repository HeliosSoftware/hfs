//! `ExportJobController` trait and associated types.

use std::sync::Arc;

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use helios_persistence::core::sof_runner::ViewFilters;
use helios_persistence::tenant::TenantContext;
use serde_json::Value;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use super::in_memory::JobFailure;

/// Opaque identifier for an export job.
pub type JobId = String;

/// A single named ViewDefinition to be run as part of an export job.
///
/// `$sql-export` accepts `subject` 1..*, each with an optional `name` plus one
/// of `subjectCanonical`, `subjectReference` or `subjectResource`. The kick-off
/// handler resolves the subject and packages each view here.
#[derive(Debug, Clone)]
pub struct NamedView {
    /// `subject.name` from the spec — drives `output.name` in the manifest.
    pub name: String,
    /// The resolved ViewDefinition JSON.
    pub view: Value,
}

/// A single named SQL query to be run as part of a `$sql-export` job.
///
/// The kickoff handler resolves the Library's full dependency graph (Phase 1
/// of the two-phase resolver in `crate::handlers::sof::graph` — every
/// ViewDefinition and SQLView Library it reaches, at any depth), validates
/// the SQL, and binds `Library.parameter` values before submitting, so the
/// background job only materializes the plan (Phase 2) and executes.
#[derive(Debug, Clone)]
pub struct NamedSqlQuery {
    /// `subject.name` from the spec — drives `output.name` in the manifest.
    pub name: String,
    /// The validated (SELECT-only) SQL text from the Library.
    pub sql: String,
    /// The subject's fully-resolved dependency graph, ready for Phase 2
    /// materialization. Not part of the crate's public API — the plan
    /// structure is an internal contract between the kickoff handler and the
    /// background export worker.
    pub(crate) plan: crate::handlers::sof::graph::GraphPlan,
    /// Bound `Library.parameter` values for the SQL's `:name` placeholders.
    pub bindings: Vec<helios_sof::sqlquery::BoundParam>,
}

/// Execution caps for SQL query export work. Mirrors the `$sql-run`
/// server configuration so exports and synchronous runs enforce the same
/// resource limits.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlExportLimits {
    /// Maximum rows materialized per depends-on ViewDefinition.
    pub max_source_rows_per_vd: usize,
    /// Maximum rows returned by the user SQL.
    pub max_rows: usize,
    /// SQL execution timeout in seconds.
    pub timeout_secs: u64,
}

/// The work an export job performs.
///
/// `$sql-export` takes a repeating `subject` parameter carrying "any mixture of
/// ViewDefinitions, SQLQuery Libraries and SQLView Libraries", so one job holds
/// both kinds rather than one kind per job. That mixture is the point of the
/// operation: every subject is computed against a single snapshot of the data,
/// so a view output and a query output can be joined on a shared key without a
/// skew window — which two separate jobs, seeing the data at two different
/// moments, cannot offer.
#[derive(Debug, Clone, Default)]
pub struct ExportWork {
    /// ViewDefinition subjects. Each produces one output entry in the manifest.
    pub views: Vec<NamedView>,
    /// SQLQuery / SQLView Library subjects. Each produces one output entry.
    pub queries: Vec<NamedSqlQuery>,
    /// Execution caps shared by every query in the job. Ignored when
    /// [`queries`](Self::queries) is empty.
    pub limits: SqlExportLimits,
}

impl ExportWork {
    /// Total number of subjects, which is also the number of `output` entries
    /// the manifest will carry.
    pub fn subject_count(&self) -> usize {
        self.views.len() + self.queries.len()
    }

    /// Whether the job names no subjects at all. `subject` is `1..*`, so a
    /// request that produces this is rejected with `400 Bad Request`.
    pub fn is_empty(&self) -> bool {
        self.subject_count() == 0
    }
}

/// Input task for a new export job.
#[derive(Debug, Clone)]
pub struct ExportTask {
    /// The subjects to compute, in any mixture of views and queries.
    pub work: ExportWork,
    /// Tenant that owns this export.
    pub tenant: TenantContext,
    /// Row filters (limit, patient, etc.). For SQL query work these apply to
    /// the materialized table sources.
    pub filters: ViewFilters,
    /// Output format: `"ndjson"`, `"csv"`, `"json"`, `"parquet"`, or `"fhir"`.
    pub format: String,
    /// Whether to include a CSV header row (CSV format only).
    pub header: bool,
    /// Optional client-supplied tracking identifier echoed back in the manifest.
    pub client_tracking_id: Option<String>,
}

/// A single output file produced by an export job.
#[derive(Debug, Clone)]
pub struct CompletedFile {
    /// Logical view name this file belongs to (matches `view.name`).
    pub view_name: String,
    /// The shard's stable filename within the job (e.g. `shard-0.ndjson`).
    ///
    /// The public download URL is *not* stored here: it is resolved on demand
    /// via [`ExportSink::download_url`](super::sink::ExportSink::download_url)
    /// each time the manifest is rendered. This matters for S3-backed exports,
    /// whose pre-signed URLs would otherwise expire relative to a single
    /// write-time signing instead of each poll.
    pub filename: String,
    /// Number of data rows written.
    pub row_count: usize,
}

/// Current status of an export job.
#[derive(Debug, Clone)]
pub enum JobStatus {
    /// Job is still running.
    Running {
        /// Number of subjects that have finished completely (all their
        /// shards written). `0 ≤ subjects_done ≤ subjects_total`.
        subjects_done: u32,
        /// Total number of subjects in the job (`ExportWork::subject_count`).
        subjects_total: u32,
        /// Output name of the subject currently being written, or `None`
        /// when no subject is in flight (before the first one starts, or
        /// after the last one finishes). See [`JobStatus::percent`] for the
        /// single place the completion percentage is derived from these
        /// counts — this variant deliberately does not also store a
        /// percentage, so the two can never disagree (#853).
        current_subject: Option<String>,
        /// Time the job was submitted.
        submitted_at: DateTime<Utc>,
    },
    /// Job finished successfully.
    Completed {
        /// Output files produced by the job.
        files: Vec<CompletedFile>,
        /// Time the job was submitted.
        submitted_at: DateTime<Utc>,
        /// Time the job finished.
        completed_at: DateTime<Utc>,
        /// Output format echoed in the completion manifest (e.g. `"ndjson"`).
        format: String,
        /// Client-supplied tracking id, echoed back to the caller if present.
        client_tracking_id: Option<String>,
    },
    /// Job failed with an error.
    Failed {
        /// The message the result endpoint returns: the specific wording when
        /// `status` is a 4xx (the request's own failure); for a server fault,
        /// a generic message that names the job, with the underlying error
        /// logged instead (#1703).
        message: String,
        /// What the result endpoint answers: the 4xx `$sql-run` gives the
        /// same failure when it is the request's own (a documented limit, a
        /// subject the runner refuses), `500` for a fault of the server.
        status: StatusCode,
        /// The `OperationOutcome.issue.code` that goes with `status`.
        code: &'static str,
        /// Output name of the subject the job was writing when it failed:
        /// the client's own `subject.name` (or the subject's own `name`, or
        /// the generated `output-N`). The result endpoint returns it as
        /// `OperationOutcome.issue[0].expression` even when `message` is the
        /// generic server-fault text (#1800). `None` when no subject was in
        /// flight.
        subject: Option<String>,
        /// Time the job was submitted.
        submitted_at: DateTime<Utc>,
        /// Time the worker recorded the failure. Captured once at the
        /// transition into `Failed` so successive polls of the result URL
        /// report a stable `exportEndTime` / `exportDuration`.
        failed_at: DateTime<Utc>,
    },
    /// Job was cancelled, or deleted via `DELETE` on the status URL. Carries
    /// the time of the transition so the cleanup reaper can age it out like the
    /// other terminal states.
    Cancelled {
        /// Time the job was cancelled / deleted.
        cancelled_at: DateTime<Utc>,
    },
}

impl JobStatus {
    /// Returns the time the job entered a terminal state (completed, failed, or
    /// cancelled), or `None` while it is still `Running`. The cleanup reaper
    /// uses this to decide when a finished job's output may be reclaimed.
    pub fn terminal_at(&self) -> Option<DateTime<Utc>> {
        match self {
            JobStatus::Running { .. } => None,
            JobStatus::Completed { completed_at, .. } => Some(*completed_at),
            JobStatus::Failed { failed_at, .. } => Some(*failed_at),
            JobStatus::Cancelled { cancelled_at } => Some(*cancelled_at),
        }
    }

    /// Returns the job's completion percentage while it is `Running`, or
    /// `None` for any other variant.
    ///
    /// This is the *only* place the percentage is computed
    /// (`subjects_done * 100 / subjects_total`, capped at 99): both the
    /// `X-Progress` header and the `subjectsDone`/`subjectsTotal` body
    /// parameters on a status poll read from this one method, so they can
    /// never disagree (#853). The 99 cap keeps a client from seeing "100%"
    /// before the completion manifest actually exists.
    pub fn percent(&self) -> Option<u8> {
        match self {
            JobStatus::Running {
                subjects_done,
                subjects_total,
                ..
            } => Some((((*subjects_done) * 100) / (*subjects_total).max(1)).min(99) as u8),
            _ => None,
        }
    }
}

/// One export job's status and its cancellation token, as the job's worker and
/// the controller see them (#1864).
///
/// **Contract:** every transition out of [`JobStatus::Running`] (a cancel, a
/// failure, a completion) goes through this handle and fires
/// [`token`](Self::token), so whatever the job is running stops at once
/// instead of noticing the new status at its next checkpoint. The worker also
/// holds a drop guard of the token, so the token fires when the worker ends
/// for any reason (an early return or a panic included), and removing the job
/// fires it again, which is idempotent. Each run the job starts gets a child
/// token of it in its [`RunContext`](helios_persistence::core::sof_runner::RunContext),
/// so one run's stop never stops the job.
///
/// A worker receives its handle from the controller's `submit`; any other
/// caller obtains one through [`ExportJobController::job_handle`], which is
/// tenant-checked.
#[derive(Clone)]
pub struct JobHandle {
    jobs: Arc<DashMap<JobId, JobStatus>>,
    job_id: JobId,
    token: CancellationToken,
}

impl std::fmt::Debug for JobHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobHandle")
            .field("job_id", &self.job_id)
            .field("cancelled", &self.token.is_cancelled())
            .finish()
    }
}

impl JobHandle {
    /// The handle of `job_id`, whose status lives in `jobs`.
    pub(crate) fn new(
        jobs: Arc<DashMap<JobId, JobStatus>>,
        job_id: JobId,
        token: CancellationToken,
    ) -> Self {
        Self {
            jobs,
            job_id,
            token,
        }
    }

    /// The job's id.
    pub fn job_id(&self) -> &str {
        &self.job_id
    }

    /// The job's cancellation token. It fires on every transition out of
    /// `Running`, when the worker ends, and when the job is removed.
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    /// Whether the job is still `Running`. A cancelled, finished or removed
    /// job is not.
    pub fn is_running(&self) -> bool {
        matches!(
            self.jobs.get(&self.job_id).as_deref(),
            Some(JobStatus::Running { .. })
        )
    }

    /// The job's current status, or `None` once it was removed.
    pub fn status(&self) -> Option<JobStatus> {
        self.jobs.get(&self.job_id).map(|entry| entry.clone())
    }

    /// Cancels the job: a `Running` job becomes [`JobStatus::Cancelled`] and
    /// its token fires. Returns whether this call moved the job; a job that
    /// already finished (or was removed) is left untouched.
    pub fn cancel(&self) -> bool {
        self.transition_if_running(|_| JobStatus::Cancelled {
            cancelled_at: Utc::now(),
        })
    }

    /// Records the job's failure: a `Running` job becomes
    /// [`JobStatus::Failed`] with the subject it was writing, and its token
    /// fires. Returns whether this call moved the job.
    pub(crate) fn fail(&self, failure: JobFailure) -> bool {
        self.transition_if_running(|running| failure.into_status(&self.job_id, running))
    }

    /// Records the job's completion with `files`: a `Running` job becomes
    /// [`JobStatus::Completed`] and its token fires. Returns whether this call
    /// moved the job; a job cancelled in the meantime stays cancelled.
    pub(crate) fn complete(
        &self,
        files: Vec<CompletedFile>,
        completed_at: DateTime<Utc>,
        format: String,
        client_tracking_id: Option<String>,
    ) -> bool {
        self.transition_if_running(|running| JobStatus::Completed {
            files,
            submitted_at: running_submitted_at(running),
            completed_at,
            format,
            client_tracking_id,
        })
    }

    /// Records progress of a `Running` job (the job stays `Running`, its token
    /// does not fire). A job that is no longer `Running` is left untouched.
    pub(crate) fn record_progress(
        &self,
        subjects_done: u32,
        subjects_total: u32,
        current_subject: Option<&str>,
    ) {
        if let Some(mut entry) = self.jobs.get_mut(&self.job_id) {
            if let JobStatus::Running { submitted_at, .. } = &*entry {
                let submitted_at = *submitted_at;
                *entry = JobStatus::Running {
                    subjects_done,
                    subjects_total,
                    current_subject: current_subject.map(str::to_string),
                    submitted_at,
                };
            }
        }
    }

    /// The single transition out of `Running`: replaces a `Running` status
    /// with `next(&running)` and fires the token. A job that is no longer
    /// `Running` keeps its status (a cancelled job is never resurrected), and
    /// the call returns `false`.
    fn transition_if_running(&self, next: impl FnOnce(&JobStatus) -> JobStatus) -> bool {
        let moved = match self.jobs.get_mut(&self.job_id) {
            Some(mut entry) if matches!(&*entry, JobStatus::Running { .. }) => {
                let status = next(&entry);
                debug_assert!(
                    !matches!(status, JobStatus::Running { .. }),
                    "a transition leaves Running"
                );
                *entry = status;
                true
            }
            _ => false,
        };
        // Fired after the map entry is released: whatever wakes on the token
        // may read the status.
        if moved {
            self.token.cancel();
        }
        moved
    }
}

/// The submission time a `Running` status carries (`Utc::now()` for any other
/// status, which a transition never sees).
fn running_submitted_at(status: &JobStatus) -> DateTime<Utc> {
    match status {
        JobStatus::Running { submitted_at, .. } => *submitted_at,
        _ => Utc::now(),
    }
}

/// Errors returned by export operations.
#[derive(Debug, Error)]
pub enum ExportError {
    /// The SofRunner returned an error.
    #[error("view runner error: {0}")]
    Runner(String),
    /// The ExportSink failed to write.
    #[error("sink write error: {0}")]
    Sink(String),
    /// Output serialization (NDJSON/CSV) failed.
    #[error("serialization error: {0}")]
    Serialization(String),
    /// The request's own fault — a documented limit or a subject the runner
    /// refuses — with the status and issue code `$sql-run` answers it with.
    #[error("{message}")]
    Client {
        /// The HTTP status the result endpoint answers with.
        status: StatusCode,
        /// The `OperationOutcome.issue.code` that goes with `status`.
        code: &'static str,
        /// The wording `$sql-run` gives the same failure.
        message: String,
    },
}

/// Why a controller refused to accept an export job.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SubmitError {
    /// The task's tenant already has the controller's maximum number of jobs
    /// queued or running (`HFS_EXPORT_MAX_JOBS_PER_TENANT`).
    #[error(
        "this tenant already has {limit} export jobs queued or running, the most HFS_EXPORT_MAX_JOBS_PER_TENANT allows; a job cancelled while it is still queued keeps counting until its turn comes; retry once one of them has finished"
    )]
    TenantJobLimit {
        /// The per-tenant limit that was reached.
        limit: usize,
    },
}

/// Trait for managing async export jobs.
///
/// All methods are synchronous (no `async`) because the controller uses internal
/// locking (DashMap) for shared state.  The actual work is spawned via
/// `tokio::spawn` inside `submit()`.
///
/// Status/cancel/download methods all require the caller's `tenant_id`.
/// Implementations MUST return `None` / `false` when the supplied `tenant_id`
/// does not match the tenant that submitted the job, so that one tenant
/// cannot poll, cancel, or read another tenant's exports by guessing a
/// job ID. The handler maps `None`/`false` to `404 Not Found` rather than
/// `403 Forbidden` to avoid leaking the existence of cross-tenant jobs.
pub trait ExportJobController: Send + Sync + 'static {
    /// Submits a new export job and returns its [`JobId`].
    ///
    /// The job begins running immediately in the background. The tenant
    /// is taken from `task.tenant` and recorded so subsequent accessor
    /// calls can be tenant-checked.
    ///
    /// Refuses the job with [`SubmitError::TenantJobLimit`] when the task's
    /// tenant already has the controller's maximum number of jobs queued or
    /// running.
    fn submit(&self, task: ExportTask) -> Result<JobId, SubmitError>;

    /// Returns the current [`JobStatus`] for the given job, or `None` if
    /// the job ID is unknown OR if `tenant_id` does not match the tenant
    /// that submitted the job.
    fn get_status(&self, tenant_id: &str, job_id: &str) -> Option<JobStatus>;

    /// Requests cancellation of the given job.
    ///
    /// Returns `true` if the job was found (and cancelled / already done),
    /// `false` if the job ID was not found OR if `tenant_id` does not match
    /// the tenant that submitted the job.
    fn cancel(&self, tenant_id: &str, job_id: &str) -> bool;

    /// Reads raw bytes for a shard file produced by a completed job.
    ///
    /// Used by the download handler to serve the file contents.
    /// Returns `None` if the job or shard does not exist OR if `tenant_id`
    /// does not match the tenant that submitted the job.
    fn read_shard(&self, tenant_id: &str, job_id: &str, filename: &str) -> Option<Vec<u8>>;

    /// Resolves a completed shard's [`CompletedFile::filename`] to a public
    /// download URL, freshly each call.
    ///
    /// Called while rendering the completion manifest so every status poll
    /// hands out a URL with a full validity window — for S3-backed exports this
    /// re-signs the GET URL on each poll rather than reusing one signed at write
    /// time. For server-routed sinks (filesystem / in-memory) the URL is stable.
    ///
    /// Returns `None` if `tenant_id` does not match the submitting tenant, or if
    /// the sink fails to produce a URL (e.g. an S3 pre-signing error).
    fn download_url(
        &self,
        tenant_id: &str,
        public_base_url: &str,
        job_id: &str,
        filename: &str,
    ) -> Option<String>;

    /// Returns the [`JobHandle`] of the given job (its status and its
    /// cancellation token), or `None` if the job ID is unknown OR if
    /// `tenant_id` does not match the tenant that submitted the job.
    ///
    /// The default returns `None`: a controller that does not run its jobs
    /// through handles exposes none. A controller that does MUST fire the
    /// handle's token on every transition of the job out of `Running` (see
    /// [`JobHandle`]).
    fn job_handle(&self, tenant_id: &str, job_id: &str) -> Option<JobHandle> {
        let _ = (tenant_id, job_id);
        None
    }
}
