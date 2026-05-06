use crate::profile::profile_registry::ProfileRegistry;
use crate::profile::validate::{validate_declared_profiles, validate_declared_profiles_async};
use crate::service::{TerminologyService, TerminologyServiceSync};
use crate::{
    AsyncValidationContext, FhirPathEvaluator, ValidationContext, ValidationIssue, ValidationState,
    Validator,
};
use helios_fhir::FhirVersion;

/// Validate an R4B resource by applying generated bindings first, then invariants.
#[cfg(feature = "R4B")]
pub fn validate_r4b_resource(
    validator: &Validator,
    resource: &helios_fhir::r4b::Resource,
    terminology: Option<&dyn TerminologyServiceSync>,
    evaluator: &dyn FhirPathEvaluator,
) -> Vec<ValidationIssue> {
    let mut issues = validator.validate_r4b_resource_bindings(resource, terminology);
    issues.extend(validator.validate_r4b_resource_invariants(resource, evaluator));
    issues
}
#[cfg(feature = "R4B")]
pub async fn validate_r4b_resource_async(
    validator: &Validator,
    resource: &helios_fhir::r4b::Resource,
    terminology: Option<&dyn TerminologyService>,
    evaluator: &dyn FhirPathEvaluator,
) -> Vec<ValidationIssue> {
    let mut issues = validator
        .validate_r4b_resource_bindings_async(resource, terminology)
        .await;
    issues.extend(validator.validate_r4b_resource_invariants(resource, evaluator));
    issues
}

#[cfg(feature = "R4B")]
pub fn validate_r4b_resource_with_profiles(
    validator: &Validator,
    resource: &helios_fhir::r4b::Resource,
    terminology: Option<&dyn TerminologyServiceSync>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let mut issues = validate_r4b_resource(validator, resource, terminology, evaluator);
    issues.extend(validate_r4b_declared_profiles(
        validator,
        resource,
        terminology,
        evaluator,
        profile_registry,
    ));
    issues
}

#[cfg(feature = "R4B")]
pub async fn validate_r4b_resource_async_with_profiles(
    validator: &Validator,
    resource: &helios_fhir::r4b::Resource,
    terminology: Option<&dyn TerminologyService>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let mut issues = validate_r4b_resource_async(validator, resource, terminology, evaluator).await;
    issues.extend(
        validate_r4b_declared_profiles_async(
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
#[cfg(feature = "R4B")]
fn validate_r4b_declared_profiles(
    validator: &Validator,
    resource: &helios_fhir::r4b::Resource,
    terminology: Option<&dyn TerminologyServiceSync>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let ctx = ValidationContext {
        fhir_version: FhirVersion::R4B,
        validator,
        terminology,
        evaluator,
        runtime_profile_registry: Some(profile_registry),
        extracted_profile_map: profile_registry.as_map(),
    };
    let mut state = ValidationState::default();
    validate_declared_profiles(&ctx, &mut state, resource, resource.resource_name())
}

#[cfg(feature = "R4B")]
async fn validate_r4b_declared_profiles_async(
    validator: &Validator,
    resource: &helios_fhir::r4b::Resource,
    terminology: Option<&dyn TerminologyService>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let ctx = AsyncValidationContext {
        fhir_version: FhirVersion::R4B,
        validator,
        terminology,
        evaluator,
        runtime_profile_registry: Some(profile_registry),
        extracted_profile_map: profile_registry.as_map(),
    };
    let mut state = ValidationState::default();
    validate_declared_profiles_async(&ctx, &mut state, resource, resource.resource_name()).await
}

#[cfg(feature = "R4B")]
pub fn validate_r4b_resource_with_validation_addons(
    validator: &Validator,
    resource: &helios_fhir::r4b::Resource,
    terminology: Option<&dyn TerminologyServiceSync>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let mut issues = validate_r4b_resource(validator, resource, terminology, evaluator);
    issues.extend(validator.apply_validation_addons(
        resource,
        resource.resource_name(),
        profile_registry,
    ));
    issues
}

#[cfg(feature = "R4B")]
pub fn validate_r4b_resource_with_profiles_and_validation_addons(
    validator: &Validator,
    resource: &helios_fhir::r4b::Resource,
    terminology: Option<&dyn TerminologyServiceSync>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let mut issues = validate_r4b_resource_with_profiles(
        validator,
        resource,
        terminology,
        evaluator,
        profile_registry,
    );
    issues.extend(validator.apply_validation_addons(
        resource,
        resource.resource_name(),
        profile_registry,
    ));
    issues
}

#[cfg(feature = "R4B")]
pub async fn validate_r4b_resource_async_with_validation_addons(
    validator: &Validator,
    resource: &helios_fhir::r4b::Resource,
    terminology: Option<&dyn TerminologyService>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let mut issues = validate_r4b_resource_async(validator, resource, terminology, evaluator).await;
    issues.extend(validator.apply_validation_addons(
        resource,
        resource.resource_name(),
        profile_registry,
    ));
    issues
}

#[cfg(feature = "R4B")]
pub async fn validate_r4b_resource_async_with_profiles_and_validation_addons(
    validator: &Validator,
    resource: &helios_fhir::r4b::Resource,
    terminology: Option<&dyn TerminologyService>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let mut issues = validate_r4b_resource_async_with_profiles(
        validator,
        resource,
        terminology,
        evaluator,
        profile_registry,
    )
    .await;
    issues.extend(validator.apply_validation_addons(
        resource,
        resource.resource_name(),
        profile_registry,
    ));
    issues
}
