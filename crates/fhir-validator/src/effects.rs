//! Deferred effects: validations collected during the pure synchronous walk
//! and executed afterwards.
//!
//! The structural walk ([`crate::engine`]) never performs I/O and never
//! evaluates FHIRPath. When it encounters a `constraints` or `binding`
//! keyword it records a [`Deferred`] entry; the async half of validation
//! (Phase 4) executes them via pluggable handlers.

use crate::schema::Binding;
use serde_json::Value;

/// A validation obligation collected during the sync walk.
#[derive(Debug, Clone, PartialEq)]
pub enum Deferred {
    /// A FHIRPath invariant to evaluate with the node at `path` as focus.
    Constraint {
        /// Dotted conformance path of the node the constraint attaches to.
        path: String,
        /// Constraint id (e.g. `pat-1`).
        id: String,
        /// FHIRPath expression.
        expression: String,
        /// Human-readable description (used in the emitted message).
        human: Option<String>,
        /// `error` | `warning` | `guideline`.
        severity: Option<String>,
    },
    /// A required-strength terminology binding to check.
    Binding {
        /// Dotted conformance path of the coded node.
        path: String,
        /// The binding (ValueSet canonical + strength).
        binding: Binding,
        /// The coded value as found in the data.
        value: Value,
    },
}
