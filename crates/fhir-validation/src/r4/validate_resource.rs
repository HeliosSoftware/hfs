use crate::profile::profile_registry::ProfileRegistry;
use crate::profile::validate::{validate_declared_profiles, validate_declared_profiles_async};
use crate::service::{TerminologyService, TerminologyServiceSync};
use crate::{
    AsyncValidationContext, FhirPathEvaluator, ValidationContext, ValidationIssue, ValidationState,
    Validator,
};
use helios_fhir::FhirVersion;
use std::collections::HashSet;

fn dedupe_issues(issues: Vec<ValidationIssue>) -> Vec<ValidationIssue> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(issues.len());
    for i in issues {
        let k = format!(
            "{:?}|{}|{}|{:?}|{:?}|{:?}|{}",
            i.severity,
            i.code,
            i.fhir_path,
            i.instance_path,
            i.expression,
            i.source_invariant_key,
            i.diagnostics
        );
        if seen.insert(k) {
            out.push(i);
        }
    }
    out
}

/// Validate an R4 resource by applying generated bindings first, then invariants.
#[cfg(feature = "R4")]
pub fn validate_r4_resource(
    validator: &Validator,
    resource: &helios_fhir::r4::Resource,
    terminology: Option<&dyn TerminologyServiceSync>,
    evaluator: &dyn FhirPathEvaluator,
) -> Vec<ValidationIssue> {
    let mut issues = validator.validate_r4_resource_bindings(resource, terminology);
    issues.extend(validator.validate_r4_resource_invariants(resource, evaluator));
    issues
}
#[cfg(feature = "R4")]
pub async fn validate_r4_resource_async(
    validator: &Validator,
    resource: &helios_fhir::r4::Resource,
    terminology: Option<&dyn TerminologyService>,
    evaluator: &dyn FhirPathEvaluator,
) -> Vec<ValidationIssue> {
    let mut issues = validator
        .validate_r4_resource_bindings_async(resource, terminology)
        .await;
    issues.extend(validator.validate_r4_resource_invariants(resource, evaluator));
    issues
}

#[cfg(feature = "R4")]
pub fn validate_r4_resource_with_profiles(
    validator: &Validator,
    resource: &helios_fhir::r4::Resource,
    terminology: Option<&dyn TerminologyServiceSync>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let mut issues = validate_r4_resource(validator, resource, terminology, evaluator);
    issues.extend(validate_r4_declared_profiles(
        validator,
        resource,
        terminology,
        evaluator,
        profile_registry,
    ));
    dedupe_issues(issues)
}

#[cfg(feature = "R4")]
pub async fn validate_r4_resource_async_with_profiles(
    validator: &Validator,
    resource: &helios_fhir::r4::Resource,
    terminology: Option<&dyn TerminologyService>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let mut issues = validate_r4_resource_async(validator, resource, terminology, evaluator).await;
    issues.extend(
        validate_r4_declared_profiles_async(
            validator,
            resource,
            terminology,
            evaluator,
            profile_registry,
        )
        .await,
    );
    dedupe_issues(issues)
}
#[cfg(feature = "R4")]
fn validate_r4_declared_profiles(
    validator: &Validator,
    resource: &helios_fhir::r4::Resource,
    terminology: Option<&dyn TerminologyServiceSync>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let ctx = ValidationContext {
        fhir_version: FhirVersion::R4,
        validator,
        terminology,
        evaluator,
        runtime_profile_registry: Some(profile_registry),
        extracted_profile_map: profile_registry.as_map(),
    };
    let mut state = ValidationState::default();
    validate_declared_profiles(&ctx, &mut state, resource, resource.resource_name())
}

#[cfg(feature = "R4")]
async fn validate_r4_declared_profiles_async(
    validator: &Validator,
    resource: &helios_fhir::r4::Resource,
    terminology: Option<&dyn TerminologyService>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let ctx = AsyncValidationContext {
        fhir_version: FhirVersion::R4,
        validator,
        terminology,
        evaluator,
        runtime_profile_registry: Some(profile_registry),
        extracted_profile_map: profile_registry.as_map(),
    };
    let mut state = ValidationState::default();
    validate_declared_profiles_async(&ctx, &mut state, resource, resource.resource_name()).await
}

#[cfg(feature = "R4")]
pub fn validate_r4_resource_with_validation_addons(
    validator: &Validator,
    resource: &helios_fhir::r4::Resource,
    terminology: Option<&dyn TerminologyServiceSync>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let mut issues = validate_r4_resource(validator, resource, terminology, evaluator);
    issues.extend(validator.apply_validation_addons(
        resource,
        resource.resource_name(),
        profile_registry,
    ));
    dedupe_issues(issues)
}

#[cfg(feature = "R4")]
pub fn validate_r4_resource_with_profiles_and_validation_addons(
    validator: &Validator,
    resource: &helios_fhir::r4::Resource,
    terminology: Option<&dyn TerminologyServiceSync>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let mut issues = validate_r4_resource_with_profiles(
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
    dedupe_issues(issues)
}

#[cfg(feature = "R4")]
pub async fn validate_r4_resource_async_with_validation_addons(
    validator: &Validator,
    resource: &helios_fhir::r4::Resource,
    terminology: Option<&dyn TerminologyService>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let mut issues = validate_r4_resource_async(validator, resource, terminology, evaluator).await;
    issues.extend(validator.apply_validation_addons(
        resource,
        resource.resource_name(),
        profile_registry,
    ));
    dedupe_issues(issues)
}

#[cfg(feature = "R4")]
pub async fn validate_r4_resource_async_with_profiles_and_validation_addons(
    validator: &Validator,
    resource: &helios_fhir::r4::Resource,
    terminology: Option<&dyn TerminologyService>,
    evaluator: &dyn FhirPathEvaluator,
    profile_registry: &ProfileRegistry,
) -> Vec<ValidationIssue> {
    let mut issues = validate_r4_resource_async_with_profiles(
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
    dedupe_issues(issues)
}
