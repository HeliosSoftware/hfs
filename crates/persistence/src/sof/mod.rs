//! SQL-on-FHIR support for storage backends.
//!
//! This module contains:
//! - [`compiler`] — ViewDefinition → SQL compiler (SQLite and PostgreSQL dialects)
//! - [`sqlite`] — [`SqliteInDbRunner`] implementing [`SofRunner`] for SQLite
//! - [`postgres`] — [`PgInDbRunner`] implementing [`SofRunner`] for PostgreSQL

pub mod compiler;

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "postgres")]
pub mod postgres;
