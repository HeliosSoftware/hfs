//! Inject `meta.profile` into HL7 R5 examples and validate with [`Validator::validate_resource_with_profiles`].

use crate::common::fhir_json_examples::read_fhir_example_json;
use crate::common::fixtures::{load_profile, local_terminology_r5};
use crate::harness::r5_evaluator_for;
use fhir_validation::Validator;
use fhir_validation::profile::profile_registry::ProfileRegistry;
use helios_fhir::FhirResource;
use helios_fhir::r5::{Patient, Resource};
use serde_json::json;

const ATRIUS_PATIENT_PROFILE: &str = "http://atrius.health/fhir/StructureDefinition/atrius-patient";

fn patient_with_profile_meta(example_json: &str, profile_url: &str) -> FhirResource {
    let mut v: serde_json::Value = serde_json::from_str(example_json).expect("json");
    v.as_object_mut()
        .expect("object")
        .insert("meta".to_string(), json!({ "profile": [ profile_url ] }));
    let patient: Patient = serde_json::from_value(v).expect("Patient");
    FhirResource::R5(Box::new(Resource::Patient(Box::new(patient))))
}

#[test]
fn genomic_patient_with_atrius_profile_has_no_errors() {
    let json = read_fhir_example_json("R5", "Patient-genomicPatient.json");
    let resource = patient_with_profile_meta(&json, ATRIUS_PATIENT_PROFILE);
    let mut registry = ProfileRegistry::new();
    registry.insert(load_profile(
        helios_fhir::FhirVersion::R5,
        "profile/atrius-profile.json",
    ));
    let evaluator = r5_evaluator_for(&resource);
    let term = local_terminology_r5();
    let issues = Validator::default().validate_resource_with_profiles(
        &resource,
        Some(&term),
        &evaluator,
        &registry,
    );
    let errors: Vec<_> = issues
        .iter()
        .filter(|i| {
            matches!(
                i.severity,
                fhir_validation::Severity::Error | fhir_validation::Severity::Fatal
            )
        })
        .collect();
    assert!(
        errors.is_empty(),
        "expected no errors for conforming example, got: {errors:#?}"
    );
}

#[test]
fn patient_without_identifier_fails_atrius_profile() {
    let mut v: serde_json::Value =
        serde_json::from_str(&read_fhir_example_json("R5", "Patient-genomicPatient.json")).unwrap();
    let obj = v.as_object_mut().unwrap();
    obj.remove("identifier");
    obj.insert(
        "meta".to_string(),
        json!({ "profile": [ ATRIUS_PATIENT_PROFILE ] }),
    );
    let patient: Patient = serde_json::from_value(v).unwrap();
    let resource = FhirResource::R5(Box::new(Resource::Patient(Box::new(patient))));
    let mut registry = ProfileRegistry::new();
    registry.insert(load_profile(
        helios_fhir::FhirVersion::R5,
        "profile/atrius-profile.json",
    ));

    let issues = Validator::default().validate_resource_with_profiles(
        &resource,
        Some(&local_terminology_r5()),
        &r5_evaluator_for(&resource),
        &registry,
    );

    assert!(
        issues.iter().any(|i| i.fhir_path == "Patient.identifier"),
        "expected a profile issue on Patient.identifier, got: {issues:#?}"
    );
}

#[test]
fn resource_without_declared_profile_still_validates_via_base_structure() {
    let resource =
        crate::common::fhir_json_examples::load_r5_fhir_resource("Patient-genomicPatient.json");
    let evaluator = r5_evaluator_for(&resource);
    let term = local_terminology_r5();
    let issues = Validator::default().validate_resource(&resource, Some(&term), &evaluator);
    assert!(
        issues.len() < 1000,
        "structural validation should stay bounded: {} issues",
        issues.len()
    );
}
