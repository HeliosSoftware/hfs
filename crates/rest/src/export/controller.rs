//! `ExportJobController` trait and associated types.

use chrono::{DateTime, Utc};
use helios_persistence::core::sof_runner::ViewFilters;
use helios_persistence::tenant::TenantContext;
use serde_json::Value;
use thiserror::Error;

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

/// One table source for a SQL query export: a resolved ViewDefinition that is
/// materialized under `label` for the SQL to query against. Table sources do
/// not produce their own `output` entries in the manifest.
#[derive(Debug, Clone)]
pub struct SqlTableSource {
    /// Table alias the SQL references (the `relatedArtifact.label`).
    pub label: String,
    /// The resolved ViewDefinition JSON.
    pub view: Value,
}

/// A single named SQL query to be run as part of a `$sql-export` job.
///
/// The kickoff handler resolves the Library and its `depends-on`
/// ViewDefinitions, validates the SQL, and binds `Library.parameter` values
/// before submitting, so the background job only materializes and executes.
#[derive(Debug, Clone)]
pub struct NamedSqlQuery {
    /// `subject.name` from the spec — drives `output.name` in the manifest.
    pub name: String,
    /// The validated (SELECT-only) SQL text from the Library.
    pub sql: String,
    /// Resolved table sources, in `relatedArtifact` declaration order.
    pub tables: Vec<SqlTableSource>,
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
        /// Completion percentage (0..=100). Surfaced as the spec's
        /// `X-Progress: {n}%` header on polling responses.
        percent: u8,
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
        /// Human-readable error message.
        message: String,
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
    fn submit(&self, task: ExportTask) -> JobId;

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
    fn download_url(&self, tenant_id: &str, job_id: &str, filename: &str) -> Option<String>;
}
