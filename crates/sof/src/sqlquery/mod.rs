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
pub mod terminology;

pub use bind::{BINDABLE_PARAMETER_TYPES, BoundParam, bind_supplied_params};
pub use engine::{ColumnFhirType, InMemorySqlEngine, QueryResult, TableSchema};
pub use library::{DependsOnView, LibraryParameter, SqlQueryLibrary, parse_sqlquery_library};
pub use output::format_fhir_parameters;
pub use params::{SqlQueryRunParams, extract_sqlquery_params_from_json};
pub use scan::{
    Placeholder, ScanError, ScanResult, SourcePosition, TableRef, scan_sql, undeclared_tables,
};
pub use terminology::{
    CONCEPT_MAP_COLUMNS, ConceptMapRow, FlattenLimits, TerminologyContentError, VALUE_SET_COLUMNS,
    ValueSetRow, concept_map_schema, estimate_json_bytes, expand_enumerated_compose,
    flatten_concept_map, flatten_value_set, local_expansion, row_bytes, value_set_schema,
};

use std::fmt;

use thiserror::Error;

/// The kind of artifact a `depends-on` dependency resolves to. Used to word
/// [`SqlQueryError::DependencyRowCapExceeded`] and
/// [`SqlQueryError::TerminologyContent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DependencyKind {
    /// A leaf ViewDefinition, materialized by the wired SQL-on-FHIR runner.
    ViewDefinition,
    /// An interior SQL View Library, materialized by running its SQL.
    SqlView,
    /// A ValueSet exposed as a value set relation from its expansion
    /// ([`terminology::flatten_value_set`]).
    ValueSet,
    /// A ConceptMap exposed as a concept map relation
    /// ([`terminology::flatten_concept_map`]).
    ConceptMap,
}

impl DependencyKind {
    /// Whether this kind is a terminology relation rather than a view.
    pub fn is_terminology(self) -> bool {
        matches!(self, Self::ValueSet | Self::ConceptMap)
    }

    /// What one row of this kind is called, for row-cap messages.
    fn row_noun(self) -> &'static str {
        match self {
            Self::ViewDefinition | Self::SqlView => "rows",
            Self::ValueSet => "members",
            Self::ConceptMap => "mappings",
        }
    }

    /// How to shrink a dependency of this kind, for row-cap messages.
    fn narrowing_advice(self) -> &'static str {
        match self {
            Self::ViewDefinition | Self::SqlView => {
                "Narrow the dependency with a ViewDefinition 'where'"
            }
            Self::ValueSet => "Narrow the value set",
            Self::ConceptMap => "Narrow the concept map",
        }
    }
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

    /// A `depends-on` dependency (a ViewDefinition, a SQL View, a ValueSet
    /// or a ConceptMap) produced more rows than the per-dependency cap
    /// (`HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD`). The engine's bare
    /// [`SqlQueryError::RowCapExceeded`] names neither the dependency nor
    /// the setting; the plan executor maps it to this variant, which does.
    /// `label` is the name the consuming SQL selects the dependency by, and
    /// `view` is the artifact's name (or canonical URL when it has no name).
    /// The wording is kind-aware: a value set has members and is narrowed,
    /// not filtered with a ViewDefinition `where`.
    #[error(
        "dependency '{label}' ({kind} {view}) produced more than {max} {}; SQL queries \
         materialize each dependency in full before the query's WHERE runs. {}, or raise \
         HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD.",
        kind.row_noun(), kind.narrowing_advice()
    )]
    DependencyRowCapExceeded {
        label: String,
        kind: DependencyKind,
        view: String,
        max: usize,
    },

    /// A terminology dependency resolved, but its content cannot be turned
    /// into a relation (term-20: 422 before any SQL runs). `label` is the
    /// name the consuming SQL selects it by and `url` the canonical it was
    /// declared with.
    #[error("dependency '{label}' ({kind} {url}): {error}")]
    TerminologyContent {
        kind: DependencyKind,
        label: String,
        url: String,
        error: TerminologyContentError,
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

    #[test]
    fn dependency_row_cap_message_is_kind_aware_for_terminology() {
        let err = SqlQueryError::DependencyRowCapExceeded {
            label: "cvd_codes".to_string(),
            kind: DependencyKind::ValueSet,
            view: "http://example.org/ValueSet/cvd".to_string(),
            max: 10,
        };
        let text = err.to_string();
        assert!(
            text.starts_with(
                "dependency 'cvd_codes' (ValueSet http://example.org/ValueSet/cvd) produced \
                 more than 10 members;"
            ),
            "{text}"
        );
        assert!(text.contains("Narrow the value set, or raise"), "{text}");
        assert!(!text.contains("ViewDefinition 'where'"), "{text}");
        let map = SqlQueryError::DependencyRowCapExceeded {
            label: "m".to_string(),
            kind: DependencyKind::ConceptMap,
            view: "cm".to_string(),
            max: 1,
        }
        .to_string();
        assert!(
            map.contains("more than 1 mappings") && map.contains("Narrow the concept map"),
            "{map}"
        );
    }

    #[test]
    fn terminology_content_error_names_label_kind_and_url() {
        let err = SqlQueryError::TerminologyContent {
            kind: DependencyKind::ValueSet,
            label: "cvd".to_string(),
            url: "http://example.org/vs|1".to_string(),
            error: TerminologyContentError::NoExpansion,
        };
        let text = err.to_string();
        assert!(
            text.starts_with("dependency 'cvd' (ValueSet http://example.org/vs|1): "),
            "{text}"
        );
        assert!(text.contains("has no expansion"), "{text}");
    }
}
