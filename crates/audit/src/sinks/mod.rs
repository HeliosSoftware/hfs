//! Audit sink backend implementations.

#[cfg(feature = "cloudwatch")]
pub mod cloudwatch;
pub mod database;
pub mod file;
pub mod null;

#[cfg(feature = "cloudwatch")]
pub use cloudwatch::CloudWatchLogsSink;
pub use database::DatabaseSink;
pub use file::FileSink;
pub use null::NullSink;
