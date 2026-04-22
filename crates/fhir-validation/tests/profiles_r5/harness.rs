//! Shared helpers for `profiles_r5` integration tests.

use fhir_validation::profile::types::ExtractedProfile;
use fhir_validation::profile::validate::validate_profile;
use fhir_validation::validation_context::{ValidationContext, ValidationState};
use fhir_validation::{
    FhirPathEvaluator, LocalTerminologyService, R5FhirPathEvaluator, ValidationIssue, Validator,
};
use helios_fhir::{FhirResource, FhirVersion};
use std::collections::HashMap;

pub fn r5_evaluator_for(resource: &FhirResource) -> R5FhirPathEvaluator {
    let FhirResource::R5(r) = resource else {
        panic!("expected R5 FhirResource");
    };
    R5FhirPathEvaluator::new((**r).clone())
}

pub fn run_validate_profile<T: serde::Serialize>(
    validator: &Validator,
    resource: &T,
    resource_type: &str,
    profile: &ExtractedProfile,
    evaluator: &dyn FhirPathEvaluator,
) -> Vec<ValidationIssue> {
    let term = LocalTerminologyService::new(FhirVersion::R5);
    let extracted_profile_map = HashMap::new();
    let ctx = ValidationContext {
        fhir_version: FhirVersion::R5,
        validator,
        terminology: Some(&term),
        evaluator,
        runtime_profile_registry: None,
        extracted_profile_map: &extracted_profile_map,
    };
    let mut state = ValidationState::default();
    validate_profile(&ctx, &mut state, resource, resource_type, profile)
}
