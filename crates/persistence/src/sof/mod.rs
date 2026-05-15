//! SQL-on-FHIR support for storage backends.
//!
//! This module contains:
//! - [`compiler`] — legacy string-pattern ViewDefinition → SQL compiler
//!   (active until the IR-based pipeline reaches feature parity in stage 2).
//! - [`ir`], [`dialect`], [`emit`], [`compile_path`], [`compile_view`] — the
//!   IR-based pipeline introduced as scaffolding in stage 1; consumers land
//!   in stages 2–5.
//! - [`sqlite`] — [`SqliteInDbRunner`] implementing [`SofRunner`] for SQLite.
//! - [`postgres`] — [`PgInDbRunner`] implementing [`SofRunner`] for PostgreSQL.
//!
//! Inline `resource:` parameters on `$viewdefinition-run` are handled by the
//! REST layer via the in-process `helios-sof` FHIRPath evaluator, so this
//! module no longer needs a per-backend inline runner.

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
