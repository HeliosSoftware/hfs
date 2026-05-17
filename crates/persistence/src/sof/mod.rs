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
//! - [`sqlite`] — [`SqliteInDbRunner`] implementing [`SofRunner`] for SQLite.
//! - [`postgres`] — [`PgInDbRunner`] implementing [`SofRunner`] for PostgreSQL.
//!
//! Inline `resource:` parameters on `$viewdefinition-run` are handled by the
//! REST layer via the in-process `helios-sof` FHIRPath evaluator, so this
//! module does not need a per-backend inline runner.

pub mod compile_path;
pub mod compile_view;
pub mod compiler;
pub mod dialect;
pub mod emit;
pub mod ir;

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "sqlite")]
pub mod sqlite_udfs;

#[cfg(feature = "postgres")]
pub mod postgres;

use helios_fhir::FhirVersion;

/// Dispatches to the per-version field-type table in `helios-fhir`. Returns
/// `(field_type, is_collection)` when the `(parent_type, field_name)` pair
/// is known.
///
/// The `FhirVersion` enum's variants are gated on `helios-fhir`'s own
/// feature flags, which may not align with this crate's feature flags when
/// an upstream consumer enables a version on `helios-fhir` directly. Falls
/// back to `None` rather than failing to compile.
pub(super) fn lookup_field_type(
    version: FhirVersion,
    parent_type: &str,
    field_name: &str,
) -> Option<(&'static str, bool)> {
    match version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => helios_fhir::r4::get_field_type(parent_type, field_name),
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => helios_fhir::r4b::get_field_type(parent_type, field_name),
        #[cfg(feature = "R5")]
        FhirVersion::R5 => helios_fhir::r5::get_field_type(parent_type, field_name),
        #[cfg(feature = "R6")]
        FhirVersion::R6 => helios_fhir::r6::get_field_type(parent_type, field_name),
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

/// Returns true when `field_name` appears in the per-version FIELD_TYPES
/// table for any parent. Used as a parent-context-free fallback for
/// detecting polymorphic typed variants (`valueQuantity`, `deceasedBoolean`,
/// …) when the path's parent FHIR type can't be resolved from the resource
/// root (e.g. inside a `where`/`forEach` sub-scope).
pub(super) fn field_exists_anywhere(version: FhirVersion, field_name: &str) -> bool {
    let table: &[(&str, &str, &str, bool)] = match version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => helios_fhir::r4::FIELD_TYPES,
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => helios_fhir::r4b::FIELD_TYPES,
        #[cfg(feature = "R5")]
        FhirVersion::R5 => helios_fhir::r5::FIELD_TYPES,
        #[cfg(feature = "R6")]
        FhirVersion::R6 => helios_fhir::r6::FIELD_TYPES,
        #[allow(unreachable_patterns)]
        _ => return false,
    };
    table.iter().any(|(_, f, _, _)| *f == field_name)
}
