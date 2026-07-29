//! PostgreSQL backend implementation.
//!
//! This module provides a complete PostgreSQL implementation of all storage traits.
//! It supports connection pooling via deadpool-postgres, JSONB storage for resources,
//! native TIMESTAMPTZ for timestamps, and PostgreSQL full-text search.
//!
//! # Features
//!
//! - Connection pooling with deadpool-postgres
//! - Full CRUD operations with tenant isolation
//! - Version history tracking
//! - Search support (string, token, date, reference, quantity, composite)
//! - Full-text search using tsvector/tsquery
//! - Transaction support with configurable isolation levels
//! - Pessimistic locking with SELECT ... FOR UPDATE
//!
//! # Example
//!
//! ```no_run
//! use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
//! use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
//!
//! # async fn main_example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a PostgreSQL backend
//! let config = PostgresConfig::default();
//! let backend = PostgresBackend::new(config).await?;
//!
//! // Initialize the schema
//! backend.init_schema().await?;
//!
//! // Create a tenant context
//! let tenant = TenantContext::new(
//!     TenantId::new("acme"),
//!     TenantPermissions::full_access(),
//! );
//!
//! // Now you can use the backend for CRUD operations
//! # Ok(())
//! # }
//! ```
//!
//! # Schema
//!
//! The PostgreSQL backend uses the following core schema:
//!
//! ```sql
//! -- Main resource table
//! CREATE TABLE IF NOT EXISTS resources (
//!     tenant_id TEXT NOT NULL,
//!     resource_type TEXT NOT NULL,
//!     id TEXT NOT NULL,
//!     version_id TEXT NOT NULL,
//!     data JSONB NOT NULL,
//!     last_updated TIMESTAMPTZ NOT NULL,
//!     is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
//!     deleted_at TIMESTAMPTZ,
//!     fhir_version TEXT NOT NULL DEFAULT '4.0',
//!     PRIMARY KEY (tenant_id, resource_type, id)
//! );
//!
//! -- Version history table
//! CREATE TABLE IF NOT EXISTS resource_history (
//!     tenant_id TEXT NOT NULL,
//!     resource_type TEXT NOT NULL,
//!     id TEXT NOT NULL,
//!     version_id TEXT NOT NULL,
//!     data JSONB NOT NULL,
//!     last_updated TIMESTAMPTZ NOT NULL,
//!     is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
//!     fhir_version TEXT NOT NULL DEFAULT '4.0',
//!     PRIMARY KEY (tenant_id, resource_type, id, version_id)
//! );
//! ```

mod backend;
mod bulk_export;
mod bulk_submit;
pub(crate) mod schema;
pub mod search;
mod search_impl;
mod storage;
mod transaction;
mod user_settings;

pub use backend::{PostgresBackend, PostgresConfig};

/// Converts a `tokio_postgres` error into a [`StorageError`], classified by
/// SQLSTATE, with `context` describing what the caller was doing.
///
/// This is the driver-facing counterpart to each module's `internal_error`
/// helper. Prefer it at every site that maps a `tokio_postgres::Error`:
/// `internal_error(format!("…: {e}"))` stringifies the error and throws away
/// `err.code()`, which is the only thing that can tell a statement cancellation
/// (`57014`) apart from a genuine defect (issue #353).
///
/// ```ignore
/// // before — SQLSTATE lost, always a 500
/// .map_err(|e| internal_error(format!("Failed to execute search: {e}")))?
/// // after  — 57014 becomes a 504, everything else is unchanged
/// .map_err(|e| query_error("Failed to execute search", e))?
/// ```
///
/// The fallback is deliberately byte-identical to `internal_error`'s output for
/// the same context, so converting a call site cannot change behaviour for any
/// error that is not explicitly classified. Because it takes a
/// `tokio_postgres::Error` by type, a site whose closure yields some other error
/// (serde, chrono, a parse) fails to compile rather than being silently
/// mis-converted.
pub(crate) fn query_error(context: &str, err: tokio_postgres::Error) -> crate::error::StorageError {
    crate::error::StorageError::Backend(crate::error::classify_postgres_error(context, err))
}
