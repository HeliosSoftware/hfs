//! `ProfileRegistry` resolution and `validate_resource_with_profiles` on `tests/fixtures/r5/profile/*`.

use crate::common::fixtures::{load_profile, load_resource, local_terminology_r5};
use crate::harness::r5_evaluator_for;
use fhir_validation::Validator;
use fhir_validation::profile::profile_registry::ProfileRegistry;
use helios_fhir::FhirVersion;

fn validator() -> Validator {
    Validator::default()
}

fn atrius_registry() -> ProfileRegistry {
    let mut registry = ProfileRegistry::new();
    registry.insert(load_profile(FhirVersion::R5, "profile/atrius-profile.json"));
    registry
}

#[test]
fn declared_meta_reports_missing_required_fields_and_root_invariant() {
    let resource = load_resource(FhirVersion::R5, "profile/declared-meta.json");
    let registry = atrius_registry();
    let evaluator = r5_evaluator_for(&resource);
    let term = local_terminology_r5();
    let issues =
        validator().validate_resource_with_profiles(&resource, Some(&term), &evaluator, &registry);
    assert_eq!(
        issues.len(),
        5,
        "identifier + gender + birthDate + root invariant + identifier value rule: {issues:#?}"
    );
    assert!(issues.iter().any(|i| i.fhir_path == "Patient.identifier"));
    assert!(issues.iter().any(|i| i.fhir_path == "Patient.gender"));
    assert!(issues.iter().any(|i| i.fhir_path == "Patient.birthDate"));
    assert!(issues.iter().any(|i| i.fhir_path == "Patient"));
    assert!(!issues.iter().any(|i| i.code == "not-found"));
    assert!(
        issues
            .iter()
            .any(|i| i.expression.as_deref() == Some("active = true implies name.exists()"))
    );
}

#[test]
fn only_invariants_fixture_reports_expected_profile_issue_count() {
    let resource = load_resource(FhirVersion::R5, "profile/only-invariants.json");
    let registry = atrius_registry();
    let evaluator = r5_evaluator_for(&resource);
    let term = local_terminology_r5();
    let issues =
        validator().validate_resource_with_profiles(&resource, Some(&term), &evaluator, &registry);
    assert_eq!(issues.len(), 4);
    assert!(
        issues.iter().any(|i| i.fhir_path == "Patient"),
        "expected at least one issue at Patient: {issues:#?}"
    );
    assert!(
        issues
            .iter()
            .any(|i| { i.expression.as_deref() == Some("active = true implies name.exists()") }),
        "expected active/name invariant: {issues:#?}"
    );
    assert!(issues.iter().any(|i| i.fhir_path == "Patient.identifier"));
    assert!(
        issues
            .iter()
            .any(|i| i.expression.as_deref() == Some("value.exists()"))
    );
}

#[test]
fn missing_declared_profile_url_produces_not_found_issue() {
    let resource = load_resource(FhirVersion::R5, "profile/missing-profile.json");
    let registry = atrius_registry();
    let evaluator = r5_evaluator_for(&resource);
    let term = local_terminology_r5();
    let issues =
        validator().validate_resource_with_profiles(&resource, Some(&term), &evaluator, &registry);

    assert!(!issues.is_empty());
    assert!(issues.iter().any(|i| i.code == "not-found"));
    assert!(issues.iter().any(|i| i.fhir_path == "Patient.meta.profile"));
    assert!(issues.iter().any(|i| {
        i.expression.as_deref()
            == Some("http://atrius.health/fhir/StructureDefinition/missing-profile")
    }));
}
