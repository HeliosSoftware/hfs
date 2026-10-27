//! FHIRPath-backed [`ConstraintEvaluator`] (feature `fhirpath`).
//!
//! Per resource: the raw JSON is parsed **once** into the typed
//! `helios_fhir` model (the `crates/sof` pattern) — helios-fhirpath
//! evaluates against typed resources, not raw JSON. Each constraint is then
//! evaluated in two phases: for a constraint attached below the root, the
//! validator path is rendered as a relative FHIRPath and evaluated against
//! the resource to obtain the focused nodes; the invariant is then
//! evaluated once per node (as `$this`) and each result is coerced on its
//! own:
//!
//! ```text
//! path Patient.contact.0, expr "name.exists() or telecom.exists()"
//!   → focus contact[0], then evaluate the invariant on each focused node
//! ```
//!
//! Outcomes aggregate as: any node failed → failed; else any node not
//! evaluable → not evaluable; else passed. An absent node (no focused
//! nodes) passes. Root-attached constraints evaluate as-is, so root and
//! nested constraints share one coercion rule. (Splicing the invariant into
//! `path.all(expr)` does not: `all()` counts a per-node empty result as
//! `false`, which made R4 `ref-1` fail every display-only Reference —
//! #2171.)
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
//! depend on nested-resource `%resource` semantics may misfire; and
//! `%context` resolves to the root rather than the focused node.

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
    /// Parse cache, keyed by expression text (focus paths and invariants).
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
            .map(|c| self.evaluate_one(c.path, c.expression, &context))
            .collect()
    }
}

impl FhirPathConstraintEvaluator {
    fn evaluate_one(
        &self,
        path: &str,
        expression: &str,
        context: &EvaluationContext,
    ) -> ConstraintOutcome {
        let invariant = match self.parse(expression) {
            Ok(ast) => ast,
            Err(outcome) => return outcome,
        };
        let Some(relative) = relative_path(path) else {
            return match helios_fhirpath::evaluator::evaluate(&invariant, context, None) {
                Ok(result) => coerce(result),
                Err(e) => ConstraintOutcome::NotEvaluable(format!("evaluation error: {e}")),
            };
        };

        // Phase 1: the focused nodes. Phase 2: the invariant per node.
        let focus = match self.parse(&relative) {
            Ok(ast) => ast,
            Err(outcome) => return outcome,
        };
        let nodes = match helios_fhirpath::evaluator::evaluate(&focus, context, None) {
            Ok(EvaluationResult::Collection { items, .. }) => items,
            Ok(EvaluationResult::Empty) => Vec::new(),
            Ok(single) => vec![single],
            Err(e) => {
                return ConstraintOutcome::NotEvaluable(format!("evaluation error: {e}"));
            }
        };

        let mut outcome = ConstraintOutcome::Passed;
        for node in &nodes {
            match helios_fhirpath::evaluator::evaluate(&invariant, context, Some(node)) {
                Ok(result) => match coerce(result) {
                    ConstraintOutcome::Failed => return ConstraintOutcome::Failed,
                    ConstraintOutcome::NotEvaluable(detail) => {
                        if outcome == ConstraintOutcome::Passed {
                            outcome = ConstraintOutcome::NotEvaluable(detail);
                        }
                    }
                    ConstraintOutcome::Passed => {}
                },
                Err(e) => {
                    if outcome == ConstraintOutcome::Passed {
                        outcome = ConstraintOutcome::NotEvaluable(format!("evaluation error: {e}"));
                    }
                }
            }
        }
        outcome
    }

    /// Parse (or reuse) the AST for `expression`.
    fn parse(
        &self,
        expression: &str,
    ) -> Result<std::sync::Arc<helios_fhirpath::parser::Expression>, ConstraintOutcome> {
        let mut cache = self.asts.lock().expect("ast cache lock");
        if let Some(ast) = cache.get(expression) {
            return Ok(std::sync::Arc::clone(ast));
        }
        match helios_fhirpath::parse_expression(expression) {
            Ok(ast) => {
                let ast = std::sync::Arc::new(ast);
                cache.insert(expression.to_string(), std::sync::Arc::clone(&ast));
                Ok(ast)
            }
            Err(e) => Err(ConstraintOutcome::NotEvaluable(format!("parse error: {e}"))),
        }
    }
}

/// Render a dotted validator path as a FHIRPath relative to the resource
/// (`Patient.contact.0` → `contact[0]`); `None` for the root.
fn relative_path(path: &str) -> Option<String> {
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
    (!relative.is_empty()).then_some(relative)
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
    use super::relative_path;

    #[test]
    fn renders_relative_focus_paths() {
        assert_eq!(relative_path("Patient"), None);
        assert_eq!(
            relative_path("Patient.contact.0").as_deref(),
            Some("contact[0]")
        );
        assert_eq!(
            relative_path("Patient.name.2.given").as_deref(),
            Some("name[2].given")
        );
    }
}

#[cfg(all(test, feature = "R4"))]
mod evaluation_tests {
    use super::FhirPathConstraintEvaluator;
    use crate::effects::{ConstraintEvaluator, ConstraintOutcome, DeferredConstraint};
    use helios_fhir::FhirVersion;
    use serde_json::json;

    fn outcome(resource: serde_json::Value, path: &str, expression: &str) -> ConstraintOutcome {
        let constraint = DeferredConstraint {
            path,
            id: "test-1",
            expression,
        };
        FhirPathConstraintEvaluator::new()
            .evaluate_all(&resource, FhirVersion::R4, &[constraint])
            .remove(0)
    }

    fn patient() -> serde_json::Value {
        json!({
            "resourceType": "Patient",
            "contact": [{ "gender": "male" }],
            "name": [{ "given": ["a", "b"] }]
        })
    }

    #[test]
    fn nested_empty_result_passes() {
        // `name` is absent on the contact: the invariant is empty, not false.
        let result = outcome(patient(), "Patient.contact.0", "name.family = 'x'");
        assert_eq!(result, ConstraintOutcome::Passed);
    }

    #[test]
    fn nested_false_result_fails() {
        let result = outcome(patient(), "Patient.contact.0", "name.exists()");
        assert_eq!(result, ConstraintOutcome::Failed);
    }

    #[test]
    fn nested_non_boolean_result_is_not_evaluable() {
        let result = outcome(patient(), "Patient.contact.0", "gender");
        assert!(
            matches!(result, ConstraintOutcome::NotEvaluable(_)),
            "got {result:?}"
        );
    }

    #[test]
    fn absent_focus_passes() {
        let result = outcome(patient(), "Patient.contact.3", "name.exists()");
        assert_eq!(result, ConstraintOutcome::Passed);
    }

    #[test]
    fn every_focused_node_must_pass() {
        assert_eq!(
            outcome(patient(), "Patient.name.0.given", "length() < 2"),
            ConstraintOutcome::Passed
        );
        assert_eq!(
            outcome(patient(), "Patient.name.0.given", "$this = 'a'"),
            ConstraintOutcome::Failed
        );
    }
}
