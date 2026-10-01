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
pub mod scan;

pub use bind::{BINDABLE_PARAMETER_TYPES, BoundParam, bind_supplied_params};
pub use engine::{ColumnFhirType, InMemorySqlEngine, QueryResult, TableSchema};
pub use library::{DependsOnView, LibraryParameter, SqlQueryLibrary, parse_sqlquery_library};
pub use output::format_fhir_parameters;
pub use params::{SqlQueryRunParams, extract_sqlquery_params_from_json};
pub use scan::{
    Placeholder, ScanError, ScanResult, SourcePosition, TableRef, scan_sql, undeclared_tables,
};

use std::fmt;

use thiserror::Error;

/// The kind of artifact a `depends-on` dependency resolves to. Used to word
/// [`SqlQueryError::DependencyRowCapExceeded`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyKind {
    /// A leaf ViewDefinition, materialized by the wired SQL-on-FHIR runner.
    ViewDefinition,
    /// An interior SQL View Library, materialized by running its SQL.
    SqlView,
}

impl fmt::Display for DependencyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ViewDefinition => f.write_str("ViewDefinition"),
            Self::SqlView => f.write_str("SQL View"),
        }
    }
}

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

    /// A `depends-on` dependency (a ViewDefinition or a SQL View) produced
    /// more rows than the per-dependency cap
    /// (`HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD`). The engine's bare
    /// [`SqlQueryError::RowCapExceeded`] names neither the dependency nor
    /// the setting; the plan executor maps it to this variant, which does.
    /// `label` is the name the consuming SQL selects the dependency by, and
    /// `view` is the ViewDefinition's or SQL View's name (or canonical URL
    /// when it has no name).
    #[error(
        "dependency '{label}' ({kind} {view}) produced more than {max} rows; SQL queries \
         materialize each dependency in full before the query's WHERE runs. Narrow the \
         dependency with a ViewDefinition 'where', or raise \
         HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD."
    )]
    DependencyRowCapExceeded {
        label: String,
        kind: DependencyKind,
        view: String,
        max: usize,
    },

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

    /// The `SofRunner` stream feeding a `depends-on` dependency table
    /// yielded an error — a storage failure, a backend statement timeout,
    /// or a lost connection. The dependency was not materialized. This is
    /// never the client's fault (the Library and its ViewDefinitions may be
    /// perfectly well-formed) and must be surfaced as a server error, not
    /// folded into [`SqlQueryError::MalformedLibrary`].
    #[error("dependency source failed: {0}")]
    SourceStream(String),

    /// A failure in the server's own execution machinery rather than in the
    /// client's request — e.g. the blocking worker that materializes a
    /// depends-on ViewDefinition's row stream panicked. This is never
    /// caused by malformed client input and should be surfaced as a 500,
    /// not as a validation error.
    #[error("internal error: {0}")]
    Internal(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_row_cap_message_for_a_view_definition() {
        let err = SqlQueryError::DependencyRowCapExceeded {
            label: "obs".to_string(),
            kind: DependencyKind::ViewDefinition,
            view: "observation_flat".to_string(),
            max: 1_000_000,
        };
        assert_eq!(
            err.to_string(),
            "dependency 'obs' (ViewDefinition observation_flat) produced more than 1000000 \
             rows; SQL queries materialize each dependency in full before the query's WHERE \
             runs. Narrow the dependency with a ViewDefinition 'where', or raise \
             HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD."
        );
    }

    #[test]
    fn dependency_row_cap_message_for_a_sql_view() {
        let err = SqlQueryError::DependencyRowCapExceeded {
            label: "fp".to_string(),
            kind: DependencyKind::SqlView,
            view: "female_patients".to_string(),
            max: 50,
        };
        let text = err.to_string();
        assert!(
            text.starts_with(
                "dependency 'fp' (SQL View female_patients) produced more than 50 rows;"
            ),
            "{text}"
        );
        assert!(!text.contains("ViewDefinition female_patients"), "{text}");
    }
}
