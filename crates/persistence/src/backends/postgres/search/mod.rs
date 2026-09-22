//! PostgreSQL search implementation.
//!
//! This module contains the search query builder and parameter handlers
//! for the PostgreSQL backend, using $N parameter placeholders,
//! ILIKE for case-insensitive matching, and native TIMESTAMPTZ comparisons.

pub mod chain_builder;
pub(crate) mod composite_rows;
pub mod query_builder;
pub mod writer;

/// SQL expression extracting the target logical id from a stored reference.
///
/// `value_reference` holds the version-agnostic base (`Patient/<id>`, an
/// absolute URL, or a bare `<id>`). The last `/`-delimited segment is the
/// target id in every form; for a bare stored id the whole value is returned.
/// Shared with `schema.rs` so the expression index and the bare-id predicate
/// stay byte-identical — the planner only uses an expression index when the
/// query expression matches it exactly (#1414).
pub(crate) const REFERENCE_TARGET_ID_EXPR: &str = "split_part(value_reference, '/', -1)";
