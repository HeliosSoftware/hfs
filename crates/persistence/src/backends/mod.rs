//! Database backend implementations.
//!
//! This module contains implementations of the storage traits for various
//! database backends. Each backend is gated behind a feature flag.
//!
//! # Available Backends
//!
//! | Backend | Feature | Description |
//! |---------|---------|-------------|
//! | SQLite | `sqlite` | Lightweight embedded database, great for development |
//! | PostgreSQL | `postgres` | Full-featured RDBMS with JSONB support |
//! | Cassandra | `cassandra` | Wide-column store for high write throughput |
//! | MongoDB | `mongodb` | Document store with native JSON support |
//! | Neo4j | `neo4j` | Graph database for relationship-heavy queries |
//! | Elasticsearch | `elasticsearch` | Full-text search optimized |
//! | S3 | `s3` | Object storage for bulk data |
//!
//! # Example
//!
//! ```no_run
//! # #[cfg(feature = "sqlite")]
//! use helios_persistence::backends::sqlite::SqliteBackend;
//!
//! # #[cfg(feature = "sqlite")]
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create an in-memory SQLite backend
//! let backend = SqliteBackend::in_memory()?;
//!
//! // Or use a file-based database
//! let backend = SqliteBackend::open("./data/fhir.db")?;
//! # Ok(())
//! # }
//! ```

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "postgres")]
pub mod postgres;

/// Local filesystem [`ExportOutputStore`](crate::core::bulk_export_output::ExportOutputStore).
pub mod local_fs;
//
// #[cfg(feature = "cassandra")]
// pub mod cassandra;
//
#[cfg(feature = "mongodb")]
pub mod mongodb;
//
// #[cfg(feature = "neo4j")]
// pub mod neo4j;
//
#[cfg(feature = "elasticsearch")]
pub mod elasticsearch;
//
#[cfg(feature = "s3")]
pub mod s3;
