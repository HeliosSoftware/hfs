//! FHIRPath-backed [`ConstraintEvaluator`] (feature `fhirpath`).
//!
//! Per resource: the raw JSON is parsed **once** into the typed
//! `helios_fhir` model (the `crates/sof` pattern) — helios-fhirpath
//! evaluates against typed resources, not raw JSON. Each constraint is then
//! evaluated as one combined expression: for a constraint attached below
//! the root, the validator path is rendered as a relative FHIRPath and the
//! invariant is wrapped in `.all(...)`:
//!
//! ```text
//! path Patient.contact.0, expr "name.exists() or telecom.exists()"
//!   → contact[0].all(name.exists() or telecom.exists())
//! ```
//!
//! `all()` gives the correct per-node focus *and* the FHIR invariant
//! semantics for absent nodes (empty collection → pass). Root-attached
//! constraints evaluate as-is.
//!
//! Result coercion (FHIRPath 5.1.1 / FHIR invariant semantics): empty →
//! pass; a single boolean → itself; anything else → not evaluable
//! (surfaced as a warning issue by the effects layer). Note the JS
//! reference validator fails on empty — a known bug there, upstream
//! fixtures don't pin it.
//!
//! Parsed expression ASTs are memoized per evaluator instance — core
//! invariants repeat across every resource of a type.
//!
//! Known limitation (documented in the plan): helios-fhirpath resolves
//! `%resource`/`%rootResource` to the root resource, so constraints that
//! depend on nested-resource `%resource` semantics may misfire; and the
//! `.all()` wrapping makes `%context` the root rather than the node.

use crate::effects::{ConstraintEvaluator, ConstraintOutcome, DeferredConstraint};
use helios_fhir::{FhirResource, FhirVersion};
use helios_fhirpath::{EvaluationContext, EvaluationResult};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

/// Evaluates invariants with helios-fhirpath. Cheap to construct; hold one
/// per service for the lifetime of the process to benefit from the AST
/// cache.
#[derive(Default)]
pub struct FhirPathConstraintEvaluator {
    /// Combined-expression parse cache.
    asts: Mutex<HashMap<String, std::sync::Arc<helios_fhirpath::parser::Expression>>>,
}

impl FhirPathConstraintEvaluator {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ConstraintEvaluator for FhirPathConstraintEvaluator {
    fn evaluate_all(
        &self,
        resource: &Value,
        version: FhirVersion,
        constraints: &[DeferredConstraint<'_>],
    ) -> Vec<ConstraintOutcome> {
        let typed = match parse_typed(resource, version) {
            Ok(t) => t,
            Err(e) => {
                // Structural errors are already on the report when the
                // typed parse fails; constraints simply cannot run.
                let detail = format!("resource does not parse into the typed model: {e}");
                return constraints
                    .iter()
                    .map(|_| ConstraintOutcome::NotEvaluable(detail.clone()))
                    .collect();
            }
        };
        let context = EvaluationContext::new(vec![typed]);

        constraints
            .iter()
            .map(|c| {
                let combined = combine(c.path, c.expression);
                self.evaluate_one(&combined, &context)
            })
            .collect()
    }
}

impl FhirPathConstraintEvaluator {
    fn evaluate_one(&self, expression: &str, context: &EvaluationContext) -> ConstraintOutcome {
        // Parse (or reuse) the AST, then evaluate.
        let parsed = {
            let mut cache = self.asts.lock().expect("ast cache lock");
            match cache.get(expression) {
                Some(ast) => std::sync::Arc::clone(ast),
                None => match helios_fhirpath::parse_expression(expression) {
                    Ok(ast) => {
                        let ast = std::sync::Arc::new(ast);
                        cache.insert(expression.to_string(), std::sync::Arc::clone(&ast));
                        ast
                    }
                    Err(e) => return ConstraintOutcome::NotEvaluable(format!("parse error: {e}")),
                },
            }
        };

        match helios_fhirpath::evaluator::evaluate(&parsed, context, None) {
            Ok(result) => coerce(result),
            Err(e) => ConstraintOutcome::NotEvaluable(format!("evaluation error: {e}")),
        }
    }
}

/// Build the combined focus+invariant expression from a dotted validator
/// path (`Patient.contact.0` → `contact[0].all(<expr>)`).
fn combine(path: &str, expression: &str) -> String {
    let mut relative = String::new();
    for segment in path.split('.').skip(1) {
        if segment.chars().all(|c| c.is_ascii_digit()) && !segment.is_empty() {
            relative.push('[');
            relative.push_str(segment);
            relative.push(']');
        } else {
            if !relative.is_empty() {
                relative.push('.');
            }
            relative.push_str(segment);
        }
    }
    if relative.is_empty() {
        expression.to_string()
    } else {
        format!("{relative}.all({expression})")
    }
}

/// FHIRPath 5.1.1 boolean coercion with FHIR invariant semantics.
fn coerce(result: EvaluationResult) -> ConstraintOutcome {
    match result {
        EvaluationResult::Empty => ConstraintOutcome::Passed,
        EvaluationResult::Boolean(b, ..) => {
            if b {
                ConstraintOutcome::Passed
            } else {
                ConstraintOutcome::Failed
            }
        }
        EvaluationResult::Collection { items, .. } => match items.as_slice() {
            [] => ConstraintOutcome::Passed,
            [EvaluationResult::Boolean(b, ..)] => {
                if *b {
                    ConstraintOutcome::Passed
                } else {
                    ConstraintOutcome::Failed
                }
            }
            [_] => ConstraintOutcome::NotEvaluable("single non-boolean result".to_string()),
            _ => ConstraintOutcome::NotEvaluable("multiple results".to_string()),
        },
        _ => ConstraintOutcome::NotEvaluable("single non-boolean result".to_string()),
    }
}

/// Parse raw JSON into the version's typed model (`crates/sof` pattern).
fn parse_typed(resource: &Value, version: FhirVersion) -> Result<FhirResource, String> {
    match version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => serde_json::from_value::<helios_fhir::r4::Resource>(resource.clone())
            .map(|r| FhirResource::R4(Box::new(r)))
            .map_err(|e| e.to_string()),
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => serde_json::from_value::<helios_fhir::r4b::Resource>(resource.clone())
            .map(|r| FhirResource::R4B(Box::new(r)))
            .map_err(|e| e.to_string()),
        #[cfg(feature = "R5")]
        FhirVersion::R5 => serde_json::from_value::<helios_fhir::r5::Resource>(resource.clone())
            .map(|r| FhirResource::R5(Box::new(r)))
            .map_err(|e| e.to_string()),
        #[cfg(feature = "R6")]
        FhirVersion::R6 => serde_json::from_value::<helios_fhir::r6::Resource>(resource.clone())
            .map(|r| FhirResource::R6(Box::new(r)))
            .map_err(|e| e.to_string()),
        #[allow(unreachable_patterns)]
        other => Err(format!(
            "FHIR version {other:?} not enabled in helios-fhir-validator"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::combine;

    #[test]
    fn combines_paths_into_focused_expressions() {
        assert_eq!(combine("Patient", "name.exists()"), "name.exists()");
        assert_eq!(
            combine("Patient.contact.0", "name.exists() or telecom.exists()"),
            "contact[0].all(name.exists() or telecom.exists())"
        );
        assert_eq!(
            combine("Patient.name.2.given", "length() < 10"),
            "name[2].given.all(length() < 10)"
        );
    }
}
