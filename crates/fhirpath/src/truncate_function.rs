use fhirpath_support::{EvaluationError, EvaluationResult};

/// Public function that exposes the truncate functionality
/// This acts as an interface for the evaluator
pub fn truncate_function(
    invocation_base: &EvaluationResult,
    args: &[EvaluationResult],
) -> Result<EvaluationResult, EvaluationError> {
    // Forward to the implementation in truncate_impl
    crate::truncate_impl::truncate(invocation_base, args)
}