//! SQL-on-FHIR v2 SQL Query (`$sql-run` with a Library subject) logic.
//!
//! Parses a SQLQuery Library, binds `Library.parameter` values and formats the
//! rows. Where the Library's SQL runs is the storage backend's business:
//!
//! - A backend that speaks SQL (SQLite, PostgreSQL) runs it inside the primary
//!   database, over the compiled views. [`native`] checks and re-serializes
//!   the SQL for that (#2902).
//! - A backend with no SQL engine (MongoDB, S3) has its `depends-on`
//!   ViewDefinitions read out and materialized into the embedded, per-request
//!   SQLite of [`engine`], where the SQL then runs.
//!
//! The REST handler in `helios-rest` wires this to storage (resolving Library
//! / ViewDefinition resources and calling the wired `SofRunner`); this module
//! contains no storage or HTTP concerns.

pub mod bind;
pub mod engine;
pub mod library;
pub mod native;
pub mod output;
pub mod params;
pub mod projection;
pub mod pushdown;
pub mod scan;
pub mod terminology;

pub use bind::{BINDABLE_PARAMETER_TYPES, BoundParam, SqlValue, bind_supplied_params};
pub use engine::{ColumnFhirType, InMemorySqlEngine, QueryResult, TableSchema, read_query_result};
pub use library::{
    DependsOnView, LibraryParameter, SqlQueryLibrary, parse_sqlquery_library,
    parse_sqlquery_library_for,
};
pub use native::{NativeDialect, NativeSql, RESERVED_RELATION_PREFIX, prepare_native_sql};
pub use output::format_fhir_parameters;
pub use params::{SqlQueryRunParams, extract_sqlquery_params_from_json};
pub use projection::{project_view_definition, referenced_identifiers};
pub use pushdown::{Predicate, push_down_view_definition, pushable_predicates};
pub use scan::{
    Placeholder, ScanError, ScanResult, SourcePosition, TableRef, scan_sql, undeclared_tables,
};
pub use terminology::{TerminologyFlattenError, TerminologyKind, TerminologyRelation};

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
    /// A ValueSet, materialized from its expansion.
    ValueSet,
    /// A ConceptMap, materialized from its mappings.
    ConceptMap,
}

impl fmt::Display for DependencyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ViewDefinition => f.write_str("ViewDefinition"),
            Self::SqlView => f.write_str("SQL View"),
            Self::ValueSet => f.write_str("ValueSet"),
            Self::ConceptMap => f.write_str("ConceptMap"),
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

    /// The statement is not a read: the engine's read-only authorizer refused
    /// a write, a schema change, a PRAGMA, an ATTACH/DETACH or a transaction
    /// statement, or the statement is a VACUUM / any statement SQLite reports as
    /// not read-only. The payload is the complete client-facing message.
    #[error("{0}")]
    NotSelect(String),

    #[error("invalid parameter binding: {0}")]
    BindParameter(String),

    /// The SQL uses something a SQL Query may not use when it runs inside the
    /// primary database: a table that is not a declared label, a function
    /// outside the allowed set, an unsupported `FROM` item. A client error.
    #[error("{0}")]
    Unsupported(String),

    /// The primary database refused or failed the query's SQL (a syntax
    /// error, an unknown column, a failed cast). A client error; the text is
    /// the database's own message.
    #[error("{0}")]
    Database(String),

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

    /// The store feeding a `depends-on` dependency was unavailable: it could
    /// not be reached, was throttling, or could not read an object after
    /// retries (#2048). Like [`SqlQueryError::SourceStream`] it is never the
    /// client's fault, but it is transient, so the REST layer answers it with
    /// `503` and a `Retry-After` hint rather than `500`.
    #[error("dependency source unavailable: {0}")]
    SourceUnavailable(String),

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
