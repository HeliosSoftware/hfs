//! FHIRPath expression → [`SqlExpr`] compiler.
//!
//! Stage 1 defines the entry signature and the compile environment. Stages 2–5
//! implement the AST traversal:
//!
//! - Stage 2: dot navigation, indexing, comparison/logical operators, literals,
//!   `exists`, `empty`, `count`, `first`, `last`, `iif`, `not`, `ofType(primitive)`.
//! - Stage 3: nested `where`/`select` chains, focus-as-collection threading.
//! - Stage 4: `%name` constants (bound as parameters), `extension(url)`,
//!   `getResourceKey` / `getReferenceKey`, `ofType(complex)`, `join`.
//! - Stage 5: `lowBoundary` / `highBoundary`.

#![allow(dead_code)] // Stage 1 scaffold; consumers land in stages 2–5.
#![allow(missing_docs)] // Per-field docs land alongside their consumers in stages 2–5.

use std::collections::HashMap;

use crate::core::sof_runner::SofError;

use super::ir::{LitValue, SqlExpr};

/// Compile-time environment threaded through expression lowering.
///
/// Tracks the current row-source alias (for `JsonPath { root, .. }` rooting),
/// the next free parameter slot (constants and lifted string literals allocate
/// from here), and any user-supplied `ViewDefinition.constant[]` values so
/// `%name` lookups resolve to a stable parameter index.
#[derive(Debug)]
pub struct CompileEnv {
    /// SQL alias of the current focus row (typically `r`, `fe`, `it1`, …).
    pub root_alias: String,
    /// Next parameter index to allocate (1-based). Initialised to 3 (after
    /// `tenant_id` = $1 and `resource_type` = $2).
    pub next_param: usize,
    /// `ViewDefinition.constant[]` lookup. Each entry is the typed value plus
    /// the parameter slot it has been bound to (or `None` if unallocated).
    pub constants: HashMap<String, Constant>,
}

/// A `ViewDefinition.constant[]` entry resolved to a typed value.
#[derive(Debug, Clone)]
pub struct Constant {
    pub value: LitValue,
    /// Set on first reference; subsequent `%name` references reuse the same
    /// parameter slot.
    pub bound_to: Option<usize>,
}

impl CompileEnv {
    pub fn new(root_alias: impl Into<String>) -> Self {
        Self {
            root_alias: root_alias.into(),
            next_param: 3,
            constants: HashMap::new(),
        }
    }
}

/// Compile a FHIRPath expression source string into a value-level [`SqlExpr`].
///
/// Stage 1 returns [`SofError::Uncompilable`] for every input — Stage 2 wires
/// this up against `helios_fhirpath::parser`.
pub fn compile_fhirpath_expr(_src: &str, _env: &mut CompileEnv) -> Result<SqlExpr, SofError> {
    Err(SofError::Uncompilable {
        reason: "FHIRPath → SQL compiler is not yet wired (stage 1 scaffold)".to_string(),
    })
}
