//! Focused fixtures under `tests/fixtures/r5/` for binding shapes not covered by the small HL7 R5 JSON corpus.

use crate::common::fhir_json_examples::is_binding_like_issue;
use crate::common::fixtures::{
    assert_has_binding_issue, assert_no_errors, load_resource, local_terminology_r5,
};
use crate::harness::r5_evaluator_for;
use fhir_validation::Validator;
use helios_fhir::FhirVersion;

/// `StructureDefinition.language` is a **string**-like primitive (`code` in FHIR) with a required
/// language tag binding; the fixture uses an invalid tag (`lp`) to exercise local ValueSet validation.
#[test]
fn structure_definition_invalid_language_binding_local() {
    let resource = load_resource(FhirVersion::R5, "valid/structuredefinition-language.json");
    let evaluator = r5_evaluator_for(&resource);
    let term = local_terminology_r5();
    let issues = Validator::default().validate_resource(&resource, Some(&term), &evaluator);
    assert!(
        issues.iter().any(|i| {
            i.instance_path.as_deref() == Some("StructureDefinition.language")
                && is_binding_like_issue(i)
        }),
        "expected binding-like issue at StructureDefinition.language, got: {issues:#?}"
    );
}

/// `Slot.serviceType` is `CodeableReference` in R5; valid coded service type should pass locally.
#[test]
fn slot_service_type_codeable_reference_valid_local() {
    let resource = load_resource(FhirVersion::R5, "valid/slot-codeable-reference.json");
    let evaluator = r5_evaluator_for(&resource);
    let term = local_terminology_r5();
    let issues = Validator::default().validate_resource(&resource, Some(&term), &evaluator);
    assert_no_errors(&issues);
}

/// **Coding** inside `CodeableConcept` + invalid **primitive code** on `identifier.use`.
#[test]
fn patient_invalid_identifier_type_and_use_local() {
    let resource = load_resource(FhirVersion::R5, "invalid/patient/patient-bindings.json");
    let evaluator = r5_evaluator_for(&resource);
    let term = local_terminology_r5();
    let issues = Validator::default().validate_resource(&resource, Some(&term), &evaluator);
    assert_has_binding_issue(
        &issues,
        "Patient.identifier[0].type",
        "http://hl7.org/fhir/ValueSet/identifier-type",
    );
    assert_has_binding_issue(
        &issues,
        "Patient.identifier[0].use",
        "http://hl7.org/fhir/ValueSet/identifier-use|5.0.0",
    );
}
