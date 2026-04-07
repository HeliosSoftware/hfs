//! Storage backend implementations for the Helios Terminology Service.
//!
//! Each sub-module provides a concrete implementation of [`TerminologyBackend`]
//! gated by the corresponding feature flag.
//!
//! | Module     | Feature    | Type                          |
//! |------------|------------|-------------------------------|
//! | `sqlite`   | `sqlite`   | `SqliteTerminologyBackend`    |
//! | `postgres` | `postgres` | `PostgresTerminologyBackend`  |
//!
//! [`TerminologyBackend`]: crate::traits::TerminologyBackend

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "sqlite")]
pub use sqlite::SqliteTerminologyBackend;

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(feature = "postgres")]
pub use postgres::PostgresTerminologyBackend;
