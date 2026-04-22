pub mod binding;
pub mod generated;
pub mod validate_resource;

use async_trait::async_trait;

pub use binding::*;
pub use validate_resource::*;

#[cfg(feature = "R6")]
pub trait R6Validatable {
    fn validate_bindings(
        &self,
        validator: &crate::Validator,
        terminology: Option<&dyn crate::TerminologyServiceSync>,
    ) -> Vec<crate::ValidationIssue>;

    fn validate_invariants(
        &self,
        validator: &crate::Validator,
        evaluator: &dyn crate::FhirPathEvaluator,
    ) -> Vec<crate::ValidationIssue>;
}

#[async_trait]
pub trait R6ValidatableAsync {
    async fn validate_bindings_async(
        &self,
        validator: &crate::Validator,
        terminology: Option<&dyn crate::TerminologyService>,
    ) -> Vec<crate::ValidationIssue>;
}
#[cfg(feature = "R6")]
impl<T> crate::r6::R6Validatable for helios_fhir::Element<T, helios_fhir::r6::Extension> {
    fn validate_bindings(
        &self,
        validator: &crate::Validator,
        terminology: Option<&dyn crate::TerminologyServiceSync>,
    ) -> Vec<crate::ValidationIssue> {
        let _ = (validator, terminology);
        let mut issues = Vec::new();

        if let Some(values) = &self.extension {
            for value in values {
                issues.extend(value.validate_bindings(validator, terminology));
            }
        }

        issues
    }

    fn validate_invariants(
        &self,
        validator: &crate::Validator,
        evaluator: &dyn crate::FhirPathEvaluator,
    ) -> Vec<crate::ValidationIssue> {
        let mut issues = Vec::new();

        if let Some(values) = &self.extension {
            for value in values {
                issues.extend(value.validate_invariants(validator, evaluator));
            }
        }

        issues
    }
}
