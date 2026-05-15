//! SNOMED CT Expression Constraint Language (ECL) support.
//!
//! This module provides parsing and SQLite-backed evaluation of ECL expressions
//! as defined by SNOMED International:
//! <https://confluence.ihtsdotools.org/display/DOCECL>
//!
//! ## Usage
//!
//! ```rust,ignore
//! use helios_hts::ecl;
//!
//! let results = ecl::parse_and_evaluate(&conn, &system_id, "<< 404684003")?;
//! for concept in results {
//!     println!("{}: {}", concept.code, concept.display.unwrap_or_default());
//! }
//! ```
//!
//! ## Integration with `$expand`
//!
//! ECL is evaluated during `ValueSet/$expand` when a ValueSet compose include
//! contains a filter of the form:
//!
//! ```json
//! {
//!   "filter": [{
//!     "property": "constraint",
//!     "op": "=",
//!     "value": "<< 404684003 |Clinical finding|"
//!   }]
//! }
//! ```
//!
//! The simpler `concept is-a` filter is also supported:
//!
//! ```json
//! {
//!   "filter": [{
//!     "property": "concept",
//!     "op": "is-a",
//!     "value": "404684003"
//!   }]
//! }
//! ```

pub mod evaluator;
pub mod parser;

pub use evaluator::ResolvedConcept;
pub use parser::{ConceptOperator, EclExpr, FocusConcept};

use rusqlite::Connection;

use crate::error::HtsError;

/// Parse an ECL string and evaluate it against the given code system.
///
/// Returns a sorted `Vec<ResolvedConcept>` (sorted by code).
///
/// # Errors
///
/// - Returns `HtsError::InvalidRequest` if the ECL expression cannot be parsed.
/// - Returns `HtsError::StorageError` if a database query fails.
pub fn parse_and_evaluate(
    conn: &Connection,
    system_id: &str,
    ecl: &str,
) -> Result<Vec<ResolvedConcept>, HtsError> {
    let expr = parser::parse(ecl)
        .map_err(|e| HtsError::InvalidRequest(format!("Invalid ECL expression: {e}")))?;
    evaluator::evaluate(conn, system_id, &expr)
}
