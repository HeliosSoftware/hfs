//! Lowers IR ([`PlanNode`]/[`SqlExpr`]) to a concrete SQL string for a given
//! [`Dialect`].
//!
//! Stage 1 defines the public surface so the rest of the pipeline can target
//! it. Stages 2–5 fill in the body, growing coverage one IR variant at a time.

#![allow(dead_code)] // Stage 1 scaffold; consumers land in stages 2–5.

use crate::core::sof_runner::SofError;

use super::dialect::Dialect;
use super::ir::PlanNode;

/// Compiled output for a single ViewDefinition.
#[derive(Debug, Clone)]
pub struct EmittedSql {
    /// Parameterised SQL — a single `SELECT` (with CTEs allowed).
    pub sql: String,
    /// Output column names in projection order. Drives `row_to_json` in the
    /// runners.
    pub columns: Vec<String>,
    /// Index of the next free bound parameter (`$N` / `?N`). The runners use
    /// this to chain runtime filters (`since`, `patient`, `group`).
    pub next_param_index: usize,
}

/// Lowers a plan tree to SQL for the given dialect.
///
/// # Errors
///
/// Returns [`SofError::InvalidViewDefinition`] for structurally invalid plans
/// and [`SofError::Uncompilable`] for IR shapes outside the implemented subset
/// at this stage.
pub fn emit_plan(_plan: &PlanNode, _dialect: &dyn Dialect) -> Result<EmittedSql, SofError> {
    // Stage 2 onward populates this. Until then no caller invokes emit_plan;
    // the existing string-pattern compiler in `compiler.rs` remains active.
    Err(SofError::Uncompilable {
        reason: "IR-based emitter is not yet wired (stage 1 scaffold)".to_string(),
    })
}
