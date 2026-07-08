//! Export job infrastructure for `$viewdefinition-export` and
//! `$sqlquery-export`.
//!
//! This module defines:
//! - [`ExportJobController`] — trait for managing async export jobs
//! - [`InMemoryController`] — default in-process implementation, including the
//!   background reaper that reclaims finished jobs (see [`CleanupConfig`])
//! - [`ExportSink`] — trait for writing, serving, and deleting output files
//! - [`FilesystemSink`] — writes output to a local directory
//! - [`InMemorySink`] — in-process sink for testing
//!
//! ## Output lifecycle
//!
//! Output shards are created by the controller's background job and removed in
//! one of three ways: a `DELETE` on a still-running job (cancellation cleanup),
//! the job's own failure path (orphaned partial shards), or the
//! [`CleanupConfig`]-driven reaper once a finished job ages past its TTL.

pub mod controller;
pub mod in_memory;
pub mod planner;
pub mod sink;

pub use controller::{
    CompletedFile, ExportError, ExportJobController, ExportTask, ExportWork, JobStatus,
    NamedSqlQuery, SqlExportLimits, SqlTableSource,
};
pub use in_memory::{CleanupConfig, InMemoryController};
pub use planner::DEFAULT_SHARD_ROWS;
#[cfg(feature = "s3")]
pub use sink::S3Sink;
pub use sink::{ExportSink, FilesystemSink, InMemorySink};
