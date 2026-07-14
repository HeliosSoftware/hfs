//! Storage backend implementations for the Helios Terminology Server.
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

/// True when `ver` is the sentinel `current` (case-insensitive).
///
/// IG Publisher, VSAC stubs, and CQL-generated Library `dataRequirement` entries
/// validate SNOMED with `version=current`. HTS resolves that to the best loaded
/// edition (complete row with concepts), not a literal version string match.
/// See `docs/ig-publisher-compatibility.md` and `fetch_versions` ordering in
/// `sqlite/code_system.rs` / `postgres/code_system.rs`.
pub(super) fn code_system_version_is_current(ver: &str) -> bool {
    ver.eq_ignore_ascii_case("current")
}
