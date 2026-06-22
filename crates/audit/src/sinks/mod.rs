//! Audit sink backend implementations.

#[cfg(feature = "cloudwatch")]
pub mod cloudwatch;
pub mod database;
pub mod file;
pub mod memory;
pub mod null;

#[cfg(feature = "cloudwatch")]
pub use cloudwatch::CloudWatchLogsSink;
pub use database::DatabaseSink;
pub use file::FileSink;
pub use memory::InMemoryAuditSink;
pub use null::NullSink;
