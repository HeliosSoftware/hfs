//! SQLite backend implementation.
//!
//! This module provides a complete SQLite implementation of all storage traits.
//! It supports both in-memory databases (great for testing) and file-based
//! databases (for development and small deployments).
//!
//! # Features
//!
//! - In-memory and file-based modes
//! - Full CRUD operations with tenant isolation
//! - Version history tracking
//! - Basic search support (string, token, date, reference)
//! - Transaction support with ACID guarantees
//!
//! # Example
//!
//! ```no_run
//! use helios_persistence::backends::sqlite::SqliteBackend;
//! use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create an in-memory database
//! let backend = SqliteBackend::in_memory()?;
//!
//! // Initialize the schema
//! backend.init_schema()?;
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
//! The SQLite backend uses the following schema:
//!
//! ```sql
//! -- Main resource table
//! CREATE TABLE resources (
//!     tenant_id TEXT NOT NULL,
//!     resource_type TEXT NOT NULL,
//!     id TEXT NOT NULL,
//!     version_id TEXT NOT NULL,
//!     data BLOB NOT NULL,  -- JSON data
//!     last_updated TEXT NOT NULL,
//!     is_deleted INTEGER NOT NULL DEFAULT 0,
//!     deleted_at TEXT,
//!     PRIMARY KEY (tenant_id, resource_type, id)
//! );
//!
//! -- Version history table
//! CREATE TABLE resource_history (
//!     tenant_id TEXT NOT NULL,
//!     resource_type TEXT NOT NULL,
//!     id TEXT NOT NULL,
//!     version_id TEXT NOT NULL,
//!     data BLOB NOT NULL,
//!     last_updated TEXT NOT NULL,
//!     is_deleted INTEGER NOT NULL DEFAULT 0,
//!     PRIMARY KEY (tenant_id, resource_type, id, version_id)
//! );
//! ```

mod backend;
mod bulk_export;
mod bulk_submit;
mod schema;
pub mod search;
mod search_impl;
mod storage;
mod transaction;
mod user_settings;

pub use backend::{SqliteBackend, SqliteBackendConfig};

/// Converts a `rusqlite` error into a [`StorageError`], classified by
/// `ErrorCode`, with `context` describing what the caller was doing.
///
/// The SQLite counterpart to `postgres::query_error`. See
/// [`crate::error::classify_sqlite_error`] for the mapping: an interrupted
/// statement becomes a `Timeout` (504), a `busy_timeout` expiry becomes
/// `Unavailable` (503 + `Retry-After`), and everything else stays `Internal`
/// (500) with byte-identical text to the `internal_error(format!(…))` it
/// replaces (issue #353).
///
/// Taking `rusqlite::Error` by type means a site whose closure yields some
/// other error fails to compile rather than being silently mis-converted.
///
/// [`StorageError`]: crate::error::StorageError
pub(crate) fn query_error(context: &str, err: rusqlite::Error) -> crate::error::StorageError {
    crate::error::StorageError::Backend(crate::error::classify_sqlite_error(context, err))
}
