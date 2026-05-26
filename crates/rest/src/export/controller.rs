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
/// Per the SQL-on-FHIR v2 spec, `$viewdefinition-export` accepts `view` 1..*,
/// each with an optional `name` plus either `viewResource` or `viewReference`.
/// The kickoff handler resolves references and packages each view here.
#[derive(Debug, Clone)]
pub struct NamedView {
    /// `view.name` from the spec — drives `output.name` in the manifest.
    pub name: String,
    /// The resolved ViewDefinition JSON.
    pub view: Value,
}

/// Input task for a new export job.
#[derive(Debug, Clone)]
pub struct ExportTask {
    /// The set of (named) ViewDefinitions to run. Spec is `1..*`; running
    /// produces one or more output entries per view in the manifest.
    pub views: Vec<NamedView>,
    /// Tenant that owns this export.
    pub tenant: TenantContext,
    /// Row filters (limit, patient, etc.).
    pub filters: ViewFilters,
    /// Output format: `"ndjson"`, `"csv"`, `"json"`, or `"parquet"`.
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
    /// Public URL that can be fetched via the export download route.
    pub url: String,
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
    /// Job was cancelled by the caller.
    Cancelled,
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
}
