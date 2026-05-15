//! AWS S3 backend implementation.
//!
//! This backend is optimized for object-storage persistence workloads:
//! CRUD, versioning/history, and bulk operations. It is intentionally not a
//! general-purpose FHIR search/query engine.

mod backend;
mod bulk_export;
mod bulk_submit;
mod bundle;
mod client;
mod config;
mod keyspace;
mod models;
mod output_store;
mod storage;

pub use backend::S3Backend;
pub use client::{AwsS3Client, AwsS3ClientOptions, S3Api};
pub use config::{S3BackendConfig, S3TenancyMode};
pub use output_store::{AccessTokenMode, S3OutputStore};

#[cfg(test)]
mod tests;
