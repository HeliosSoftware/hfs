pub mod binding;
pub mod generated;
pub mod validate_resource;

use async_trait::async_trait;

pub use binding::*;
pub use validate_resource::*;

#[cfg(feature = "R5")]
pub trait R5Validatable {
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
pub trait R5ValidatableAsync {
    async fn validate_bindings_async(
        &self,
        validator: &crate::Validator,
        terminology: Option<&dyn crate::TerminologyService>,
    ) -> Vec<crate::ValidationIssue>;
}
#[cfg(feature = "R5")]
impl<T> crate::r5::R5Validatable for helios_fhir::Element<T, helios_fhir::r5::Extension> {
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
