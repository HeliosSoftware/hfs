//! Element rules such as **`ele-1`** (empty JSON objects, empty primitive codings).

use crate::common::fixtures::{
    assert_has_invariant_expression, load_resource, local_terminology_r5,
};
use crate::harness::r5_evaluator_for;
use fhir_validation::Validator;
use helios_fhir::FhirVersion;

const ELE1: &str = "hasValue() or (children().count() > id.count())";

#[test]
fn patient_empty_human_name_violates_ele1() {
    let r = load_resource(
        FhirVersion::R5,
        "invalid/patient/patient-empty_name_ele1.json",
    );
    let validator = Validator::default();
    let evaluator = r5_evaluator_for(&r);
    let term = local_terminology_r5();
    let issues = validator.validate_resource(&r, Some(&term), &evaluator);
    assert_has_invariant_expression(&issues, "Patient.name[0]", ELE1);
}

#[test]
fn patient_meta_security_empty_coding_violates_ele1() {
    let r = load_resource(
        FhirVersion::R5,
        "invalid/patient/patient-empty-meta-security-code.json",
    );
    let validator = Validator::default();
    let evaluator = r5_evaluator_for(&r);
    let term = local_terminology_r5();
    let issues = validator.validate_resource(&r, Some(&term), &evaluator);
    assert_has_invariant_expression(&issues, "Patient.meta.security[0]", ELE1);
}
