//! `Patient` + `contained` / local `Reference` invariants (`dom-3`, `dom-2`, etc.).

use crate::common::fixtures::{
    assert_has_invariant, assert_issue_count, assert_no_errors, load_resource, local_terminology_r5,
};
use crate::harness::r5_evaluator_for;
use fhir_validation::Validator;
use helios_fhir::FhirVersion;

#[test]
fn patient_example_valid_no_errors() {
    let r = load_resource(FhirVersion::R5, "valid/patient/patient-example.json");
    let validator = Validator::default();
    let evaluator = r5_evaluator_for(&r);
    let term = local_terminology_r5();
    let issues = validator.validate_resource(&r, Some(&term), &evaluator);
    assert_no_errors(&issues);
}

#[test]
fn local_reference_without_contained_emits_dom3_style_invariant() {
    let r = load_resource(
        FhirVersion::R5,
        "invalid/patient/patient-local_reference_but_no_contained.json",
    );
    let validator = Validator::default();
    let evaluator = r5_evaluator_for(&r);
    let term = local_terminology_r5();
    let issues = validator.validate_resource(&r, Some(&term), &evaluator);
    assert_has_invariant(
        &issues,
        "Patient.managingOrganization",
        "contained resource",
    );
}

#[test]
fn contained_without_id_emits_expected_invariants() {
    let r = load_resource(
        FhirVersion::R5,
        "invalid/patient/patient_no_id_in_contained.json",
    );
    let validator = Validator::default();
    let evaluator = r5_evaluator_for(&r);
    let term = local_terminology_r5();
    let issues = validator.validate_resource(&r, Some(&term), &evaluator);
    assert_has_invariant(
        &issues,
        "Patient.managingOrganization",
        "contained resource",
    );
    assert_has_invariant(
        &issues,
        "Patient.contained[0]",
        "The organization SHALL at least have a name or an identifier, and possibly more than one",
    );
}

#[test]
fn malformed_local_reference_emits_invariants() {
    let r = load_resource(
        FhirVersion::R5,
        "invalid/patient/patient_malformed_local_reference.json",
    );
    let validator = Validator::default();
    let evaluator = r5_evaluator_for(&r);
    let term = local_terminology_r5();
    let issues = validator.validate_resource(&r, Some(&term), &evaluator);
    assert_issue_count(&issues, 3);
    assert_has_invariant(
        &issues,
        "Patient.contained[0]",
        "The organization SHALL at least have a name or an identifier, and possibly more than one",
    );
    assert_has_invariant(
        &issues,
        "Patient",
        "If the resource is contained in another resource, it SHALL be referred to from elsewhere in the resource or SHALL refer to the containing resource",
    );
}
