//! `ExportJobController` trait and associated types.

use chrono::{DateTime, Utc};
use helios_persistence::core::sof_runner::ViewFilters;
use helios_persistence::tenant::TenantContext;
use serde_json::Value;
use thiserror::Error;

/// Opaque identifier for an export job.
pub type JobId = String;

/// Input task for a new export job.
#[derive(Debug, Clone)]
pub struct ExportTask {
    /// The ViewDefinition to run.
    pub view_definition: Value,
    /// Tenant that owns this export.
    pub tenant: TenantContext,
    /// Row filters (limit, patient, etc.).
    pub filters: ViewFilters,
    /// Output format: `"ndjson"` or `"csv"`.
    pub format: String,
}

/// A single output file produced by an export job.
#[derive(Debug, Clone)]
pub struct CompletedFile {
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
        /// Human-readable progress description, e.g. `"running view"`.
        progress: String,
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
    },
    /// Job failed with an error.
    Failed {
        /// Human-readable error message.
        message: String,
        /// Time the job was submitted.
        submitted_at: DateTime<Utc>,
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
pub trait ExportJobController: Send + Sync + 'static {
    /// Submits a new export job and returns its [`JobId`].
    ///
    /// The job begins running immediately in the background.
    fn submit(&self, task: ExportTask) -> JobId;

    /// Returns the current [`JobStatus`] for the given job, or `None` if
    /// the job ID is unknown.
    fn get_status(&self, job_id: &str) -> Option<JobStatus>;

    /// Requests cancellation of the given job.
    ///
    /// Returns `true` if the job was found (and cancelled / already done),
    /// `false` if the job ID was not found.
    fn cancel(&self, job_id: &str) -> bool;

    /// Reads raw bytes for a shard file produced by a completed job.
    ///
    /// Used by the download handler to serve the file contents.
    /// Returns `None` if the job or shard does not exist.
    fn read_shard(&self, job_id: &str, filename: &str) -> Option<Vec<u8>>;
}
