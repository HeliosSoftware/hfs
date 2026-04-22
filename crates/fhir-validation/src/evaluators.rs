//! # FHIRPath Evaluators (Atrius Validation Engine v1)
//!
//! ## Overview
//!
//! This module implements the FHIRPath evaluation layer used by the Atrius
//! validation engine. It is responsible for:
//!
//! - Evaluating invariant expressions (FHIRPath)
//! - Resolving declared paths into focus values
//! - Supporting both single and bulk invariant execution
//!
//! ## Why this refactor?
//!
//! The original design evaluated invariants one-by-one:
//!
//! - Each invariant created its own evaluation context
//! - Each invariant resolved its own focus
//! - Each invariant executed independently
//!
//! This resulted in:
//! - repeated context cloning
//! - repeated FHIRPath evaluation
//! - poor scaling for arrays and nested structures
//!
//! ## New Design (v1)
//!
//! We now use a **batch evaluation model**:
//!
//! - Resolve focus once
//! - Evaluate multiple invariant expressions on that focus
//!
//! ```text
//! Validator
//!   → resolve focus values
//!   → evaluator.eval_invariants_on(...)
//!       → evaluate N expressions
//!       → return results
//! ```
//!
//! ## Key Decisions
//!
//! ### 1. Owned `EvaluationResult`
//!
//! Reasons:
//! - avoids lifetime complexity
//! - works cleanly with `EvaluationContext::set_this`
//! - aligns with Helios FHIRPath internals
//!
//! Trade-off: cloning may occur, but correctness and simplicity are prioritized.
//!
//! ### 2. `InvariantExprRef`
//!
//! Instead of passing full `InvariantDef`, we use a lightweight struct:
//!
//! ```rust
//! pub struct InvariantExprRef<'a> {
//!     pub declared_path: &'a str,
//!     pub expression: &'a str,
//! }
//! ```
//!
//! This:
//! - decouples evaluator from generator
//! - enables dynamic/runtime invariants
//! - supports future StructureDefinition validation
//!
//! ### 3. Bulk Evaluation API
//!
//! This:
//! - sets context once
//! - evaluates multiple expressions
//! - returns one result per invariant
//!
//! ## Summary
//!
//! This module provides a:
//! - performant
//! - decoupled
//! - extensible
//!
//! foundation for FHIR invariant evaluation in Atrius.
use crate::ValidationError;
use helios_fhir::FhirResource;
#[cfg(feature = "R4")]
use helios_fhir::r4::Resource;
#[cfg(feature = "R4B")]
use helios_fhir::r4b::Resource as R4BResource;
#[cfg(feature = "R5")]
use helios_fhir::r5::Resource as R5Resource;
#[cfg(feature = "R6")]
use helios_fhir::r6::Resource as R6Resource;
use helios_fhirpath::evaluator::convert_resource_to_result;
use helios_fhirpath::{EvaluationContext, evaluate_expression};
use helios_fhirpath_support::EvaluationResult;

/// Internal evaluator used by the version-specific public evaluators.
///
/// `GenericFhirPathEvaluator` owns the shared `helios_fhirpath::EvaluationContext`
/// setup and the common logic for evaluating expressions, resolving declared
/// paths, and coercing invariant results to booleans.
///
/// It is intentionally not exposed publicly because callers should work through
/// the version-specific wrappers (`R4FhirPathEvaluator`, `R5FhirPathEvaluator`),
/// which know how to convert concrete FHIR resources into the cross-version
/// `FhirResource` representation used by `helios_fhirpath`.
///
/// In short:
/// - `GenericFhirPathEvaluator` implements the shared mechanics
/// - `FhirPathEvaluator` is the public abstraction consumed by validation code
/// - `R4FhirPathEvaluator` / `R5FhirPathEvaluator` bridge concrete model types
///   to the generic implementation
struct GenericFhirPathEvaluator {
    context: EvaluationContext,
}
/// Shared FHIRPath evaluation operations used by the public version-specific
/// evaluators.
impl GenericFhirPathEvaluator {
    /// Create a generic evaluator rooted at the given FHIR resource.
    ///
    /// The evaluation context initializes `%rootResource` and `%resource` so
    /// invariant expressions can use the standard FHIRPath variables.
    fn from_fhir_resource(resource: FhirResource) -> Self {
        let mut context = EvaluationContext::new(vec![resource]);

        if let Some(root) = context.this.clone() {
            context.set_variable_result("%rootResource", root.clone());
            context.set_variable_result("%resource", root);
        }

        Self { context }
    }
    /// Evaluate an arbitrary FHIRPath expression against the current context.
    ///
    /// The raw `helios_fhirpath` result is normalized into a flat collection of
    /// `EvaluationResult` items for easier consumption by validation code.
    fn eval_expression(&self, expr: &str) -> Result<Vec<EvaluationResult>, ValidationError> {
        let result = evaluate_expression(expr, &self.context).map_err(|e| {
            ValidationError::FhirPath(helios_fhirpath_support::EvaluationError::SemanticError(
                format!("failed to evaluate expression '{expr}': {e}"),
            ))
        })?;

        Ok(collect_focus_items(result))
    }
    /// Evaluate an invariant relative to its declared FHIR path.
    ///
    /// The declared path is first normalized relative to the current resource
    /// root. If the path resolves to multiple focus items, the invariant must
    /// hold for each item.
    fn eval_invariant(
        &self,
        declared_path: &str,
        expression: &str,
    ) -> Result<bool, ValidationError> {
        let normalized_focus_path = normalize_declared_path(declared_path);

        if normalized_focus_path.is_empty() {
            let result = evaluate_expression(expression, &self.context).map_err(|e| {
                ValidationError::FhirPath(helios_fhirpath_support::EvaluationError::SemanticError(
                    e.to_string(),
                ))
            })?;

            return coerce_result_to_bool(result);
        }

        let focus_result =
            evaluate_expression(&normalized_focus_path, &self.context).map_err(|e| {
                ValidationError::FhirPath(helios_fhirpath_support::EvaluationError::SemanticError(
                    format!("failed to resolve declared path '{declared_path}': {e}"),
                ))
            })?;

        let focus_items = collect_focus_items(focus_result);
        if focus_items.is_empty() {
            return Ok(true);
        }

        for focus_item in focus_items {
            let mut focus_context = self.context.clone();

            if let Some(root) = self.context.this.clone() {
                focus_context.set_variable_result("%rootResource", root.clone());
                focus_context.set_variable_result("%resource", root);
            }

            focus_context.set_this(focus_item);

            let result = evaluate_expression(expression, &focus_context).map_err(|e| {
                ValidationError::FhirPath(helios_fhirpath_support::EvaluationError::SemanticError(
                    format!(
                        "failed to evaluate invariant '{expression}' at '{declared_path}': {e}"
                    ),
                ))
            })?;

            if !coerce_result_to_bool(result)? {
                return Ok(false);
            }
        }

        Ok(true)
    }
    /// Evaluate multiple invariants on a single focus value.
    ///
    /// This is the core optimization in the Atrius validation engine:
    ///
    /// - The evaluation context is created once
    /// - `%resource` and `%rootResource` are reused
    /// - Multiple expressions are evaluated against the same focus
    ///
    /// Compared to per-invariant execution, this:
    /// - reduces repeated context cloning
    /// - improves performance on arrays and nested structures
    ///
    /// Returns one result per invariant expression.
    fn eval_invariants_on(
        &self,
        focus: EvaluationResult,
        invariants: &[InvariantExprRef<'_>],
    ) -> Vec<Result<bool, ValidationError>> {
        let mut focus_context = self.context.clone();

        if let Some(root) = self.context.this.clone() {
            focus_context.set_variable_result("%rootResource", root.clone());
            focus_context.set_variable_result("%resource", root);
        }

        focus_context.set_this(focus);

        invariants
            .iter()
            .map(|invariant| {
                let result =
                    evaluate_expression(invariant.expression, &focus_context).map_err(|e| {
                        ValidationError::FhirPath(
                            helios_fhirpath_support::EvaluationError::SemanticError(format!(
                                "failed to evaluate invariant '{}' at '{}' on focused value: {e}",
                                invariant.expression, invariant.declared_path
                            )),
                        )
                    })?;

                coerce_result_to_bool(result)
            })
            .collect()
    }

    /// Evaluate a single invariant on a focus value.
    ///
    /// This is a thin wrapper over `eval_invariants_on` and exists for API
    /// compatibility.
    ///
    /// All actual logic is delegated to the bulk evaluation path to ensure
    /// consistent behavior.
    fn eval_invariant_on(
        &self,
        focus: EvaluationResult,
        declared_path: &str,
        expression: &str,
    ) -> Result<bool, ValidationError> {
        self.eval_invariants_on(
            focus,
            &[InvariantExprRef {
                declared_path,
                expression,
            }],
        )
        .into_iter()
        .next()
        .expect("single-expression bulk evaluation should return exactly one result")
    }
    /// Evaluate a declared FHIR path and return the matching focus items.
    ///
    /// An empty normalized path resolves to the root resource itself.
    fn eval_path(&self, path: &str) -> Result<Vec<EvaluationResult>, ValidationError> {
        let normalized_path = normalize_declared_path(path);

        if normalized_path.is_empty() {
            let root = self
                .context
                .this
                .clone()
                .or_else(|| {
                    self.context
                        .resources
                        .first()
                        .map(convert_resource_to_result)
                })
                .ok_or_else(|| {
                    ValidationError::FhirPath(
                        helios_fhirpath_support::EvaluationError::SemanticError(
                            "evaluation context has no root resource".to_string(),
                        ),
                    )
                })?;

            return Ok(vec![root]);
        }

        let result = evaluate_expression(&normalized_path, &self.context).map_err(|e| {
            ValidationError::FhirPath(helios_fhirpath_support::EvaluationError::SemanticError(
                format!("failed to evaluate path '{path}': {e}"),
            ))
        })?;

        Ok(collect_focus_items(result))
    }
    /// Create a generic evaluator rooted at the given resource but with an
    /// explicit current focus value.
    ///
    /// This is useful for tests and for validation scenarios where evaluation
    /// should begin from a nested focus item rather than the resource root.
    fn from_fhir_resource_with_focus(resource: FhirResource, focus: EvaluationResult) -> Self {
        let mut context = EvaluationContext::new(vec![resource]);

        if let Some(root) = context.this.clone() {
            context.set_variable_result("%rootResource", root.clone());
            context.set_variable_result("%resource", root);
        }

        context.set_this(focus);
        Self { context }
    }
}
/// Public abstraction over FHIRPath evaluation used by validation code.
///
/// Validation logic depends on this trait rather than the generic evaluator so
/// the rest of the crate can stay agnostic to the concrete FHIR version being
/// validated.
///
/// The separation between this trait and `GenericFhirPathEvaluator` exists so:
/// - shared evaluation mechanics live in one place
/// - version-specific wrappers can convert concrete resources into
///   `FhirResource`
/// - validation code can depend on a small, stable interface
///
/// Implementations provide helpers for:
/// - evaluating invariants relative to a declared FHIR path
/// - evaluating invariants on an explicit focus value
/// - resolving FHIRPath expressions to collections
///
/// # Caller contract
///
/// - Use [`FhirPathEvaluator::eval_invariant`] when the FHIRPath must run in the context of the
///   **element** identified by `declared_path` (possibly repeated); the implementation resolves
///   that path from `%resource` and evaluates `expression` per focus.
/// - Use [`FhirPathEvaluator::eval_invariants_on`] when every expression should run on the **same**
///   pre-built `focus` value (same `$this`). The `InvariantExprRef::declared_path` field is used
///   for error messages only, **not** to re-target `$this` per invariant.
pub trait FhirPathEvaluator {
    /// Evaluate `expression` with evaluation context set from `declared_path` (FHIR: constraint
    /// context is the value at that path; all repeats must satisfy the expression when applicable).
    fn eval_invariant(
        &self,
        declared_path: &str,
        expression: &str,
    ) -> Result<bool, ValidationError>;

    /// Evaluate `expression` with `$this` = `focus`; `declared_path` is for diagnostics.
    fn eval_invariant_on(
        &self,
        focus: EvaluationResult,
        declared_path: &str,
        expression: &str,
    ) -> Result<bool, ValidationError>;

    /// Evaluate each invariant’s `expression` with the **same** `$this` = `focus`.
    ///
    /// `InvariantExprRef::declared_path` does **not** change the focus; it is only referenced in
    /// error text. Prefer [`FhirPathEvaluator::eval_invariant`] when expressions assume the element
    /// at each path, not the serialized root passed here.
    fn eval_invariants_on(
        &self,
        focus: EvaluationResult,
        invariants: &[InvariantExprRef<'_>],
    ) -> Vec<Result<bool, ValidationError>>;

    /// Evaluate a FHIR path and return the resulting collection.
    fn eval_path(
        &self,
        path: &str,
    ) -> Result<Vec<helios_fhirpath_support::EvaluationResult>, ValidationError>;
}

/// Lightweight reference to an invariant expression.
///
/// This struct borrows data instead of owning it, allowing the evaluator
/// to operate independently of generator-specific types (`InvariantDef`).
///
/// Benefits:
/// - enables bulk evaluation
/// - reduces allocation overhead
/// - supports dynamic validation rules in the future
///
/// In [`FhirPathEvaluator::eval_invariants_on`], `declared_path` is **not** used to resolve
/// focus; only `expression` is evaluated against the provided `focus`. For path-based focus, use
/// [`FhirPathEvaluator::eval_invariant`].
pub struct InvariantExprRef<'a> {
    pub declared_path: &'a str,
    pub expression: &'a str,
}
/// FHIRPath evaluator for R4 resources.
///
/// This is a thin wrapper over `GenericFhirPathEvaluator` that converts
/// R4 resources into `FhirResource` before evaluation.
#[cfg(feature = "R4")]
pub struct R4FhirPathEvaluator {
    inner: GenericFhirPathEvaluator,
}
#[cfg(feature = "R4")]
impl R4FhirPathEvaluator {
    /// Create an R4 evaluator rooted at the given resource.
    pub fn new(resource: Resource) -> Self {
        Self {
            inner: GenericFhirPathEvaluator::from_fhir_resource(FhirResource::R4(Box::new(
                resource,
            ))),
        }
    }

    /// Evaluate an arbitrary FHIRPath expression against the R4 resource.
    pub fn eval_expression(&self, expr: &str) -> Result<Vec<EvaluationResult>, ValidationError> {
        self.inner.eval_expression(expr)
    }
    /// Create an R4 evaluator with an explicit current focus value.
    pub fn new_with_focus(resource: Resource, focus: EvaluationResult) -> Self {
        Self {
            inner: GenericFhirPathEvaluator::from_fhir_resource_with_focus(
                FhirResource::R4(Box::new(resource)),
                focus,
            ),
        }
    }
}
#[cfg(feature = "R4")]
impl FhirPathEvaluator for R4FhirPathEvaluator {
    fn eval_invariant(
        &self,
        declared_path: &str,
        expression: &str,
    ) -> Result<bool, ValidationError> {
        self.inner.eval_invariant(declared_path, expression)
    }

    fn eval_invariant_on(
        &self,
        focus: EvaluationResult,
        declared_path: &str,
        expression: &str,
    ) -> Result<bool, ValidationError> {
        self.inner
            .eval_invariant_on(focus, declared_path, expression)
    }

    fn eval_invariants_on(
        &self,
        focus: EvaluationResult,
        invariants: &[InvariantExprRef<'_>],
    ) -> Vec<Result<bool, ValidationError>> {
        self.inner.eval_invariants_on(focus, invariants)
    }

    fn eval_path(&self, path: &str) -> Result<Vec<EvaluationResult>, ValidationError> {
        self.inner.eval_path(path)
    }
}
/// FHIRPath evaluator for R4B resources.
///
/// This is a thin wrapper over `GenericFhirPathEvaluator` that converts
/// R4B resources into `FhirResource` before evaluation.
#[cfg(feature = "R4B")]
pub struct R4BFhirPathEvaluator {
    inner: GenericFhirPathEvaluator,
}
#[cfg(feature = "R4B")]
impl R4BFhirPathEvaluator {
    /// Create an R4B evaluator rooted at the given resource.
    pub fn new(resource: R4BResource) -> Self {
        Self {
            inner: GenericFhirPathEvaluator::from_fhir_resource(FhirResource::R4B(Box::new(
                resource,
            ))),
        }
    }

    /// Evaluate an arbitrary FHIRPath expression against the R4B resource.
    pub fn eval_expression(&self, expr: &str) -> Result<Vec<EvaluationResult>, ValidationError> {
        self.inner.eval_expression(expr)
    }
    /// Create an R4B evaluator with an explicit current focus value.
    pub fn new_with_focus(resource: Resource, focus: EvaluationResult) -> Self {
        Self {
            inner: GenericFhirPathEvaluator::from_fhir_resource_with_focus(
                FhirResource::R4(Box::new(resource)),
                focus,
            ),
        }
    }
}
#[cfg(feature = "R4B")]
impl FhirPathEvaluator for R4BFhirPathEvaluator {
    fn eval_invariant(
        &self,
        declared_path: &str,
        expression: &str,
    ) -> Result<bool, ValidationError> {
        self.inner.eval_invariant(declared_path, expression)
    }

    fn eval_invariant_on(
        &self,
        focus: EvaluationResult,
        declared_path: &str,
        expression: &str,
    ) -> Result<bool, ValidationError> {
        self.inner
            .eval_invariant_on(focus, declared_path, expression)
    }

    fn eval_invariants_on(
        &self,
        focus: EvaluationResult,
        invariants: &[InvariantExprRef<'_>],
    ) -> Vec<Result<bool, ValidationError>> {
        self.inner.eval_invariants_on(focus, invariants)
    }

    fn eval_path(&self, path: &str) -> Result<Vec<EvaluationResult>, ValidationError> {
        self.inner.eval_path(path)
    }
}
/// FHIRPath evaluator for R5 resources.
///
/// This is a thin wrapper over `GenericFhirPathEvaluator` that converts
/// R5 resources into `FhirResource` before evaluation.
#[cfg(feature = "R5")]
pub struct R5FhirPathEvaluator {
    inner: GenericFhirPathEvaluator,
}
#[cfg(feature = "R5")]
impl R5FhirPathEvaluator {
    /// Create an R5 evaluator rooted at the given resource.
    pub fn new(resource: R5Resource) -> Self {
        Self {
            inner: GenericFhirPathEvaluator::from_fhir_resource(FhirResource::R5(Box::new(
                resource,
            ))),
        }
    }

    /// Evaluate an arbitrary FHIRPath expression against the R5 resource.
    pub fn eval_expression(&self, expr: &str) -> Result<Vec<EvaluationResult>, ValidationError> {
        self.inner.eval_expression(expr)
    }
    /// Create an R5 evaluator with an explicit current focus value.
    pub fn new_with_focus(resource: R5Resource, focus: EvaluationResult) -> Self {
        Self {
            inner: GenericFhirPathEvaluator::from_fhir_resource_with_focus(
                FhirResource::R5(Box::new(resource)),
                focus,
            ),
        }
    }
}
#[cfg(feature = "R5")]
impl FhirPathEvaluator for R5FhirPathEvaluator {
    fn eval_invariant(
        &self,
        declared_path: &str,
        expression: &str,
    ) -> Result<bool, ValidationError> {
        self.inner.eval_invariant(declared_path, expression)
    }

    fn eval_invariant_on(
        &self,
        focus: EvaluationResult,
        declared_path: &str,
        expression: &str,
    ) -> Result<bool, ValidationError> {
        self.inner
            .eval_invariant_on(focus, declared_path, expression)
    }

    fn eval_invariants_on(
        &self,
        focus: EvaluationResult,
        invariants: &[InvariantExprRef<'_>],
    ) -> Vec<Result<bool, ValidationError>> {
        self.inner.eval_invariants_on(focus, invariants)
    }

    fn eval_path(&self, path: &str) -> Result<Vec<EvaluationResult>, ValidationError> {
        self.inner.eval_path(path)
    }
}
/// FHIRPath evaluator for R6 resources.
///
/// This is a thin wrapper over `GenericFhirPathEvaluator` that converts
/// R6 resources into `FhirResource` before evaluation.
#[cfg(feature = "R6")]
pub struct R6FhirPathEvaluator {
    inner: GenericFhirPathEvaluator,
}
#[cfg(feature = "R6")]
impl R6FhirPathEvaluator {
    /// Create an R6 evaluator rooted at the given resource.
    pub fn new(resource: R6Resource) -> Self {
        Self {
            inner: GenericFhirPathEvaluator::from_fhir_resource(FhirResource::R6(Box::new(
                resource,
            ))),
        }
    }

    /// Evaluate an arbitrary FHIRPath expression against the R6 resource.
    pub fn eval_expression(&self, expr: &str) -> Result<Vec<EvaluationResult>, ValidationError> {
        self.inner.eval_expression(expr)
    }
    /// Create an R6 evaluator with an explicit current focus value.
    pub fn new_with_focus(resource: R6Resource, focus: EvaluationResult) -> Self {
        Self {
            inner: GenericFhirPathEvaluator::from_fhir_resource_with_focus(
                FhirResource::R6(Box::new(resource)),
                focus,
            ),
        }
    }
}
#[cfg(feature = "R6")]
impl FhirPathEvaluator for R6FhirPathEvaluator {
    fn eval_invariant(
        &self,
        declared_path: &str,
        expression: &str,
    ) -> Result<bool, ValidationError> {
        self.inner.eval_invariant(declared_path, expression)
    }

    fn eval_invariant_on(
        &self,
        focus: EvaluationResult,
        declared_path: &str,
        expression: &str,
    ) -> Result<bool, ValidationError> {
        self.inner
            .eval_invariant_on(focus, declared_path, expression)
    }

    fn eval_invariants_on(
        &self,
        focus: EvaluationResult,
        invariants: &[InvariantExprRef<'_>],
    ) -> Vec<Result<bool, ValidationError>> {
        self.inner.eval_invariants_on(focus, invariants)
    }

    fn eval_path(&self, path: &str) -> Result<Vec<EvaluationResult>, ValidationError> {
        self.inner.eval_path(path)
    }
}
/// Normalize a declared invariant path to a relative FHIRPath expression.
///
/// In StructureDefinition invariants, the declared path is usually of the form
/// `Resource.field.subfield`. During evaluation we already have the resource as
/// the root context, so the leading resource name is stripped.
fn normalize_declared_path(declared_path: &str) -> String {
    let path = declared_path.trim();
    if path.is_empty() {
        return String::new();
    }

    match path.split_once('.') {
        Some((_, rest)) => rest.to_string(),
        None => String::new(),
    }
}

/// Convert an EvaluationResult into a flat list of focus items.
///
/// FHIRPath may return a collection, a single value, or empty. This helper
/// normalizes the result into a vector for easier invariant evaluation.
fn collect_focus_items(result: EvaluationResult) -> Vec<EvaluationResult> {
    match result {
        EvaluationResult::Collection { items, .. } => items,
        EvaluationResult::Empty => Vec::new(),
        other => vec![other],
    }
}

/// Convert a FHIRPath evaluation result into a boolean invariant outcome.
///
/// According to FHIRPath rules:
/// - empty result → true
/// - single boolean → that value
/// - single empty → true
/// - multiple values → error
fn coerce_result_to_bool(result: EvaluationResult) -> Result<bool, ValidationError> {
    match result {
        EvaluationResult::Boolean(b, _, _) => Ok(b),

        EvaluationResult::Collection { items, .. } => match items.len() {
            0 => Ok(true),
            1 => match &items[0] {
                EvaluationResult::Boolean(b, _, _) => Ok(*b),
                EvaluationResult::Empty => Ok(true),
                other => Err(ValidationError::FhirPath(
                    helios_fhirpath_support::EvaluationError::SemanticError(format!(
                        "Invariant did not evaluate to boolean, got {}",
                        other.type_name()
                    )),
                )),
            },
            _ => Err(ValidationError::FhirPath(
                helios_fhirpath_support::EvaluationError::SemanticError(
                    "Invariant returned multiple values".to_string(),
                ),
            )),
        },

        EvaluationResult::Empty => Ok(true),

        other => Err(ValidationError::FhirPath(
            helios_fhirpath_support::EvaluationError::SemanticError(format!(
                "Invariant did not evaluate to boolean, got {}",
                other.type_name()
            )),
        )),
    }
}
