//! ViewDefinition JSON → [`PlanNode`] compiler.
//!
//! Walks the SoF `select` tree producing a plan tree rooted in a
//! [`PlanNode::Scan`] over `resources`. Per-clause logic:
//!
//! - Plain `select.column[]` → [`PlanNode::Project`] over the parent scan.
//! - `forEach`/`forEachOrNull` → [`PlanNode::LateralUnnest`] over the parent.
//! - Nested `select` → recursive descent extending the focus row source.
//! - `unionAll[]` → [`PlanNode::Union`].
//! - SoF `repeat:` directive → [`PlanNode::Recurse`].
//! - Top-level `where[].path` → [`PlanNode::Filter`] applied to the root scan.

#![allow(dead_code)] // Stage 1 scaffold; consumers land in stages 2–5.

use serde_json::Value;

use crate::core::sof_runner::SofError;

use super::dialect::Dialect;
use super::ir::PlanNode;

/// Build a plan tree for the given ViewDefinition JSON.
///
/// Stage 1 stub — Stage 2 begins populating the implementation by handling the
/// already-supported subset (flat columns, single forEach, unionAll, simple
/// where[]).
pub fn build_plan(_view_json: &Value, _dialect: &dyn Dialect) -> Result<PlanNode, SofError> {
    Err(SofError::Uncompilable {
        reason: "IR-based plan builder is not yet wired (stage 1 scaffold)".to_string(),
    })
}
