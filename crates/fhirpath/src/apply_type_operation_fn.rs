use crate::parser::TypeSpecifier;
use fhirpath_support::{EvaluationError, EvaluationResult};

pub fn apply_type_operation(
    _value: &EvaluationResult,
    _op: &str,
    _type_spec: &TypeSpecifier,
) -> Result<EvaluationResult, EvaluationError> {
    // Temporarily return Empty until we have an implementation for "is" and "as"
    Ok(EvaluationResult::Empty)
}
