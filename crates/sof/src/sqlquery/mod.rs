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
pub use engine::{ColumnFhirType, EngineStop, InMemorySqlEngine, QueryResult, TableSchema};
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

/// What kind of failure a `depends-on` source reported. The engine carries
/// it unchanged from the source's row stream to [`SqlQueryError::SourceStream`],
/// so the caller can answer the request with the source's own status (for
/// example a dependency's 422 stays a 422) instead of a generic server error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFailure {
    /// The request is unusable as given; the value is the HTTP status the
    /// caller should answer with (for example `422` or `400`).
    ClientInput(u16),
    /// A referenced resource does not exist.
    NotFound,
    /// The source's backend is temporarily unavailable; a retry may succeed.
    Unavailable,
    /// The source exceeded a time limit and was stopped.
    Timeout,
    /// The source was cancelled.
    Cancelled,
    /// A server-side fault in the source.
    Backend,
}

/// A failure reported by a `depends-on` source's row stream: its kind and a
/// description. This is the error type of the rows
/// [`InMemorySqlEngine::insert_rows`] reads.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct SourceError {
    /// What kind of failure it was.
    pub kind: SourceFailure,
    /// Human-readable description. For a server-side kind it is
    /// operator-facing detail; for a client kind it is the client's message.
    pub message: String,
}

impl SourceError {
    /// A source failure of `kind` described by `message`.
    pub fn new(kind: SourceFailure, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// A server-side fault in the source ([`SourceFailure::Backend`]).
    pub fn backend(message: impl Into<String>) -> Self {
        Self::new(SourceFailure::Backend, message)
    }
}

/// Why the engine stopped its work before it finished. The caller sets it on
/// the engine's [`EngineStop`] before the engine is interrupted, so the error a
/// stopped statement ends with is read from here, never from SQLite's own
/// "interrupted" text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopCause {
    /// The caller cancelled the query: an export job was cancelled, or the
    /// request that ran it was abandoned.
    Cancelled,
    /// The deadline of the run the query belongs to passed.
    DeadlineExceeded,
    /// One statement ran longer than the per-statement limit
    /// (`HFS_SOF_SQLQUERY_TIMEOUT_SECS`), `secs` seconds.
    StatementTimeout {
        /// The per-statement limit, in seconds.
        secs: u64,
    },
}

impl fmt::Display for StopCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("query cancelled"),
            Self::DeadlineExceeded => f.write_str("query stopped: its deadline passed"),
            Self::StatementTimeout { secs } => write!(f, "query exceeded {secs}s timeout"),
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

    /// The engine stopped before the work finished, for the [`StopCause`] the
    /// caller set: a cancel, the run's deadline, or the per-statement limit.
    #[error("{0}")]
    Stopped(StopCause),

    /// The statement is not a read: the engine's read-only authorizer refused
    /// a write, a schema change, a PRAGMA, an ATTACH/DETACH or a transaction
    /// statement, or the statement is a VACUUM / any statement SQLite reports as
    /// not read-only. The payload is the complete client-facing message.
    #[error("{0}")]
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
    /// yielded an error, before or after some of its rows. The dependency
    /// was not materialized. The [`SourceError`] keeps the source's kind:
    /// a dependency the runner refuses (for example one it cannot compile)
    /// is the client's to fix, while a storage failure, a statement timeout
    /// or a lost connection is not. It is never folded into
    /// [`SqlQueryError::MalformedLibrary`].
    #[error("dependency source failed: {0}")]
    SourceStream(SourceError),

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
