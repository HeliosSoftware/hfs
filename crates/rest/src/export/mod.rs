//! Export job infrastructure for `$sql-export`.
//!
//! This module defines:
//! - [`ExportJobController`] — trait for managing async export jobs
//! - [`InMemoryController`] — default in-process implementation, including the
//!   background reaper that reclaims finished jobs (see [`CleanupConfig`])
//! - [`ExportSink`] — trait for writing, serving, and deleting output files
//! - [`FilesystemSink`] — writes output to a local directory
//! - [`InMemorySink`] — in-process sink for testing
//! - [`JobManifest`] — a completed job's durable, on-disk record
//!
//! ## Output lifecycle
//!
//! Output shards are created by the controller's background job and removed in
//! one of three ways:
//! - (a) a `DELETE` on a still-running job (cancellation cleanup);
//! - (b) the job's own task when it ends without completing (failed, cancelled,
//!   or already removed by the reaper), which deletes anything it wrote;
//! - (c) the [`CleanupConfig`]-driven reaper once a finished job ages past its
//!   TTL.
//!
//! A cancelled job's task stops at its next checkpoint. The reaper drops a job's
//! status entry only once its delete succeeds, retrying a failed delete on the
//! next sweep.
//!
//! ## Surviving a restart
//!
//! `jobs`/`job_tenants` are in-memory only and start empty on every boot.
//! [`ExportSink::persist_completion`] writes a [`JobManifest`] for each
//! completed job (the filesystem and S3 sinks), and
//! [`ExportSink::load_completed`] reads them back at controller construction
//! so a job completed by an earlier process keeps serving status, result and
//! downloads after a restart (#1474).

pub mod controller;
pub mod in_memory;
pub mod planner;
pub mod sink;

pub use controller::{
    CompletedFile, ExportError, ExportJobController, ExportTask, ExportWork, JobStatus,
    NamedSqlQuery, SqlExportLimits, SubmitError,
};
pub use in_memory::{CleanupConfig, InMemoryController};
pub use planner::DEFAULT_SHARD_ROWS;
#[cfg(feature = "s3")]
pub use sink::S3Sink;
pub use sink::{ExportSink, FilesystemSink, InMemorySink, JobManifest, ManifestFile};
