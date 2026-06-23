//! Export job infrastructure for `$viewdefinition-export` and
//! `$sqlquery-export`.
//!
//! This module defines:
//! - [`ExportJobController`] — trait for managing async export jobs
//! - [`InMemoryController`] — default in-process implementation
//! - [`ExportSink`] — trait for writing output files
//! - [`FilesystemSink`] — writes output to a local directory
//! - [`InMemorySink`] — in-process sink for testing

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
