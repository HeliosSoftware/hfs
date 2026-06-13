//! SQL-on-FHIR v2 `$sqlquery-run` engine.
//!
//! Pure execution logic: parse a SQLQuery Library, materialize its `depends-on`
//! ViewDefinitions into an in-memory SQLite database, bind `Library.parameter`
//! values to the SQL, run the user query, and format the rows.
//!
//! The REST handler in `helios-rest` wires this to storage (resolving Library
//! / ViewDefinition resources and supplying `RowStream`s from the wired
//! `SofRunner`); this module contains no storage or HTTP concerns.

pub mod bind;
pub mod engine;
pub mod library;
pub mod output;
pub mod params;

pub use bind::{BoundParam, bind_supplied_params};
pub use engine::{ColumnFhirType, InMemorySqlEngine, QueryResult, TableSchema};
pub use library::{DependsOnView, LibraryParameter, SqlQueryLibrary, parse_sqlquery_library};
pub use output::format_fhir_parameters;
pub use params::{SqlQueryRunParams, extract_sqlquery_params_from_json};

use thiserror::Error;

/// Errors produced by the `$sqlquery-run` pipeline.
#[derive(Debug, Error)]
pub enum SqlQueryError {
    #[error("malformed Library: {0}")]
    MalformedLibrary(String),

    #[error("SQLQuery Library has no SQL content")]
    MissingSql,

    #[error("depends-on entry missing label")]
    MissingDependsOnLabel,

    #[error("could not resolve canonical URL: {0}")]
    UnknownCanonical(String),

    #[error("too many depends-on ViewDefinitions: {count} (max {max})")]
    TooManyDependsOn { count: usize, max: usize },

    #[error("row limit exceeded ({max} rows)")]
    RowCapExceeded { max: usize },

    #[error("query exceeded {secs}s timeout")]
    Timeout { secs: u64 },

    #[error("SQL parse error: {0}")]
    NotSelect(String),

    #[error("invalid parameter binding: {0}")]
    BindParameter(String),

    #[error("invalid identifier '{0}': must not contain a double-quote")]
    InvalidIdentifier(String),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("composite SQL value for column '{0}' cannot be represented as a FHIR scalar")]
    UnsupportedFhirValue(String),
}
