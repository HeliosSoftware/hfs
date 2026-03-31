//! Audit sink backend implementations.

pub mod database;
pub mod file;
pub mod null;

pub use database::DatabaseSink;
pub use file::FileSink;
pub use null::NullSink;
