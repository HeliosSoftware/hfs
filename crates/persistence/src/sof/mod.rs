//! SQL-on-FHIR support for storage backends.
//!
//! The ViewDefinition → SQL pipeline:
//!
//! 1. [`ir`] — `PlanNode` tree and value types (`LitValue`, `Expr`, …).
//! 2. [`compile_path`] — FHIRPath expression → `Expr` lowering.
//! 3. [`compile_view`] — ViewDefinition JSON → `PlanNode` (`build_plan`).
//! 4. [`dialect`] — `SqliteDialect` / `PgDialect` implementations of the
//!    `Dialect` trait (JSON accessors, parameter syntax, etc.).
//! 5. [`emit`] — `PlanNode` → parameterised SQL (`emit_plan`).
//! 6. [`compiler`] — public façade combining `build_plan` + `emit_plan` into
//!    `compile_view_definition_dialect`, the entry point used by the runners.
//!
//! Backend runners:
//! - [`sqlite`] — `SqliteInDbRunner` (SQL) for SQLite.
//! - [`postgres`] — `PgInDbRunner` (SQL) for PostgreSQL.
//! - [`mongodb`] — `MongoInDbRunner` (aggregation pipeline) for MongoDB.
//! - [`in_process`] — `InProcessSofRunner`, a backend-agnostic runner that
//!   evaluates the view with the `helios-sof` engine over scanned resources
//!   (used by S3 / S3-primary composites, which have no query engine).
//!
//! Inline `resource:` parameters on `$viewdefinition-run` are handled by the
//! REST layer via the in-process `helios-sof` FHIRPath evaluator, so this
//! module does not need a per-backend inline runner.

pub mod compile_path;
pub mod compile_view;
pub mod compiler;
pub mod dialect;
pub mod emit;
pub mod in_process;
pub mod ir;

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "sqlite")]
pub mod sqlite_udfs;

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(feature = "mongodb")]
pub mod emit_mongo;

#[cfg(feature = "mongodb")]
pub mod mongodb;

use helios_fhir::FhirVersion;

/// Thin alias for [`helios_fhir::get_field_type`] — keeps existing
/// `super::lookup_field_type(...)` call sites inside this crate working
/// without sprinkling the `helios_fhir::` prefix everywhere.
pub(super) fn lookup_field_type(
    version: FhirVersion,
    parent_type: &str,
    field_name: &str,
) -> Option<(&'static str, bool)> {
    helios_fhir::get_field_type(version, parent_type, field_name)
}

/// Thin alias for [`helios_fhir::field_exists_anywhere`] — see the canonical
/// definition there.
pub(super) fn field_exists_anywhere(version: FhirVersion, field_name: &str) -> bool {
    helios_fhir::field_exists_anywhere(version, field_name)
}
