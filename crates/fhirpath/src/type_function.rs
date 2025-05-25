use fhirpath_support::{EvaluationError, EvaluationResult};

/// FHIRPath type() function implementation
pub fn type_function(
    _invocation_base: &EvaluationResult,
    _args: &[EvaluationResult],
) -> Result<EvaluationResult, EvaluationError> {
    // Temporarily return Empty until we have an implementation
    Ok(EvaluationResult::Empty)
}
