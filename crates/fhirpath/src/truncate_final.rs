// This file serves as a central point for truncate-related functionality
// It can be expanded with additional truncate variants or utilities

pub use crate::truncate_function::truncate_function;
pub use crate::truncate_impl::truncate;

// This module could contain additional truncate-related utilities
// such as specialized truncation for different FHIR data types,
// additional truncation modes, precision control, etc.

// Re-export the primary API
pub fn truncate_expression(
    base: &fhirpath_support::EvaluationResult,
    args: &[fhirpath_support::EvaluationResult],
) -> Result<fhirpath_support::EvaluationResult, fhirpath_support::EvaluationError> {
    truncate(base, args)
}