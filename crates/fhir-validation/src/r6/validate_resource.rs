use crate::profile::profile_registry::ProfileRegistry;
use crate::profile::validate::{validate_declared_profiles, validate_declared_profiles_async};
use crate::service::{TerminologyService, TerminologyServiceSync};
use crate::{
    AsyncValidationContext, FhirPathEvaluator, ValidationContext, ValidationIssue, ValidationState,
    Validator,
};
use helios_fhir::FhirVersion;

/// Validate an R6 resource by applying generated bindings first, then invariants.
#[cfg(feature = "R6")]
pub fn validate_r6_resource(
    validator: &Validator,
    resource: &helios_fhir::r6::Resource,
    terminology: Option<&dyn TerminologyServiceSync>,
    evaluator: &dyn FhirPathEvaluator,
) -> Vec<ValidationIssue> {
    let mut issues = validator.validate_r6_resource_bindings(resource, terminology);
    issues.extend(validator.validate_r6_resource_invariants(resource, evaluator));
    issues
}

#[cfg(feature = "R6")]
pub async fn validate_r6_resource_async(
    validator: &Validator,
    resource: &helios_fhir::r6::Resource,
    terminology: Option<&dyn TerminologyService>,
    evaluator: &dyn FhirPathEvaluator,
) -> Vec<ValidationIssue> {
    let mut issues = validator
        .validate_r6_resource_bindings_async(resource, terminology)
        .await;
    issues.extend(validator.validate_r6_resource_invariants(resource, evaluator));
    issues
}

#[cfg(feature = "R6")]
pub fn validate_r6_resource_with_profiles(
    validator: &Validator,
    resource: &helios_fhir::r6::Resource,
    terminology: Option<&dyn TerminologyServiceSync>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let mut issues = validate_r6_resource(validator, resource, terminology, evaluator);
    issues.extend(validate_r6_declared_profiles(
        validator,
        resource,
        terminology,
        evaluator,
        profile_registry,
    ));
    issues
}

#[cfg(feature = "R6")]
pub async fn validate_r6_resource_async_with_profiles(
    validator: &Validator,
    resource: &helios_fhir::r6::Resource,
    terminology: Option<&dyn TerminologyService>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let mut issues = validate_r6_resource_async(validator, resource, terminology, evaluator).await;
    issues.extend(
        validate_r6_declared_profiles_async(
            validator,
            resource,
            terminology,
            evaluator,
            profile_registry,
        )
        .await,
    );
    issues
}
#[cfg(feature = "R6")]
fn validate_r6_declared_profiles(
    validator: &Validator,
    resource: &helios_fhir::r6::Resource,
    terminology: Option<&dyn TerminologyServiceSync>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let ctx = ValidationContext {
        fhir_version: FhirVersion::R6,
        validator,
        terminology,
        evaluator,
        runtime_profile_registry: Some(profile_registry),
        extracted_profile_map: profile_registry.as_map(),
    };
    let mut state = ValidationState::default();
    validate_declared_profiles(&ctx, &mut state, resource, resource.resource_name())
}

#[cfg(feature = "R6")]
async fn validate_r6_declared_profiles_async(
    validator: &Validator,
    resource: &helios_fhir::r6::Resource,
    terminology: Option<&dyn TerminologyService>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let ctx = AsyncValidationContext {
        fhir_version: FhirVersion::R6,
        validator,
        terminology,
        evaluator,
        runtime_profile_registry: Some(profile_registry),
        extracted_profile_map: profile_registry.as_map(),
    };
    let mut state = ValidationState::default();
    validate_declared_profiles_async(&ctx, &mut state, resource, resource.resource_name()).await
}
