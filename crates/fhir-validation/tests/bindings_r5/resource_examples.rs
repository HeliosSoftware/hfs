//! Full-resource validation using HL7 examples from `crates/fhir/tests/data/json/R5/`.
//!
//! Some `#[tokio::test]` cases use [`crate::harness::remote_terminology_for_tests`] and require a
//! live terminology server (see crate root docs).

use crate::common::fhir_json_examples::load_r5_fhir_resource;
use crate::common::fixtures::{
    assert_has_binding_issue, assert_has_binding_issue_with_diagnostics, assert_no_errors,
    load_resource, local_terminology_r5,
};
use crate::harness::{r5_evaluator_for, remote_terminology_for_tests};
use fhir_validation::Validator;
use helios_fhir::FhirVersion;

// --- Patient.genomicPatient: primitive `code`, CodeableConcept on identifiers, `meta.tag` --------

/// Local validation: no **error**/**fatal** issues. `meta.tag` uses a code from `v3-ActReason` while
/// the element binding targets `common-tags` (example strength); locally we only defer with a
/// terminology message, not an error.
#[test]
fn patient_genomic_local_no_errors_meta_tag_deferred_to_remote() {
    let resource = load_r5_fhir_resource("Patient-genomicPatient.json");
    let evaluator = r5_evaluator_for(&resource);
    let term = local_terminology_r5();
    let issues = Validator::default().validate_resource(&resource, Some(&term), &evaluator);
    assert_no_errors(&issues);
    assert_has_binding_issue_with_diagnostics(
        &issues,
        "Patient.meta.tag[0]",
        "http://hl7.org/fhir/ValueSet/common-tags",
        "Remote terminology validation required for http://hl7.org/fhir/ValueSet/common-tags",
    );
}

/// Same instance with remote terminology: still no severity **errors**; server reports `v3-ActReason`
/// is not in `common-tags`.
#[tokio::test]
async fn patient_genomic_remote_no_errors_meta_tag_not_in_valueset() {
    let resource = load_r5_fhir_resource("Patient-genomicPatient.json");
    let evaluator = r5_evaluator_for(&resource);
    let term = remote_terminology_for_tests();
    let issues = Validator::default()
        .validate_resource_async(&resource, Some(&term), &evaluator)
        .await;
    assert_no_errors(&issues);
    assert_has_binding_issue_with_diagnostics(
        &issues,
        "Patient.meta.tag[0]",
        "http://hl7.org/fhir/ValueSet/common-tags",
        "Code 'HTEST' is not in value set 'http://terminology.hl7.org/ValueSet/common-tags'",
    );
}

// --- CodeableConcept / Coding (meta.tag, priority) -----------------------------------------

#[test]
fn coverage_eligibility_request_wrong_valueset_in_meta_tag_local() {
    let resource = load_r5_fhir_resource("coverageeligibilityrequest-example.json");
    let evaluator = r5_evaluator_for(&resource);
    let term = local_terminology_r5();
    let issues = Validator::default().validate_resource(&resource, Some(&term), &evaluator);
    assert_has_binding_issue(
        &issues,
        "CoverageEligibilityRequest.meta.tag[0]",
        "http://hl7.org/fhir/ValueSet/common-tags",
    );
}

#[test]
fn coverage_eligibility_request_priority_code_without_system_local() {
    let resource = load_r5_fhir_resource("coverageeligibilityrequest-example.json");
    let evaluator = r5_evaluator_for(&resource);
    let term = local_terminology_r5();
    let issues = Validator::default().validate_resource(&resource, Some(&term), &evaluator);
    assert_has_binding_issue_with_diagnostics(
        &issues,
        "CoverageEligibilityRequest.priority",
        "http://hl7.org/fhir/ValueSet/process-priority",
        "Coding.system is required",
    );
}

// --- CodeableReference (`ServiceRequest.code`) -----------------------------------------------

/// Minimal `ServiceRequest` with only `code` as `CodeableReference`. Expect **no** error/fatal
/// issues; `procedure-code` may still produce a **non-error** issue while local terminology defers
/// to remote.
#[test]
fn service_request_minimal_codeableref_local_no_errors_procedure_code_deferred() {
    let resource = load_resource(
        FhirVersion::R5,
        "valid/servicerequest-codeableref-minimal.json",
    );
    let evaluator = r5_evaluator_for(&resource);
    let term = local_terminology_r5();
    let issues = Validator::default().validate_resource(&resource, Some(&term), &evaluator);
    assert_no_errors(&issues);
    assert_has_binding_issue_with_diagnostics(
        &issues,
        "ServiceRequest.code",
        "http://hl7.org/fhir/ValueSet/procedure-code",
        "Remote terminology validation required for http://hl7.org/fhir/ValueSet/procedure-code",
    );
}

#[tokio::test]
async fn service_request_minimal_codeableref_remote_no_errors() {
    let resource = load_resource(
        FhirVersion::R5,
        "valid/servicerequest-codeableref-minimal.json",
    );
    let evaluator = r5_evaluator_for(&resource);
    let term = remote_terminology_for_tests();
    let issues = Validator::default()
        .validate_resource_async(&resource, Some(&term), &evaluator)
        .await;
    assert_no_errors(&issues);
}

// Diagnostics below are asserted as returned by a typical FHIR `$validate-code` implementation
// (e.g. HAPI/Snowstorm). If your server wording differs, adjust the substring or gate the test.

#[tokio::test]
async fn service_request_codeableref_remote_unknown_snomed_code() {
    let resource = load_resource(
        FhirVersion::R5,
        "valid/servicerequest-wrong-codeable-reference.json",
    );
    let evaluator = r5_evaluator_for(&resource);
    let term = remote_terminology_for_tests();
    let issues = Validator::default()
        .validate_resource_async(&resource, Some(&term), &evaluator)
        .await;
    assert_has_binding_issue_with_diagnostics(
        &issues,
        "ServiceRequest.code",
        "http://hl7.org/fhir/ValueSet/procedure-code",
        "Code '398100567' is not in value set 'http://hl7.org/fhir/ValueSet/procedure-code'",
    );
}

#[tokio::test]
async fn service_request_codeableref_remote_display_mismatch() {
    let resource = load_resource(
        FhirVersion::R5,
        "valid/servicerequest-wrong-display-codeable-reference.json",
    );
    let evaluator = r5_evaluator_for(&resource);
    let term = remote_terminology_for_tests();
    let issues = Validator::default()
        .validate_resource_async(&resource, Some(&term), &evaluator)
        .await;
    assert_has_binding_issue_with_diagnostics(
        &issues,
        "ServiceRequest.code",
        "http://hl7.org/fhir/ValueSet/procedure-code",
        "Code '3981005' is not in value set 'http://hl7.org/fhir/ValueSet/procedure-code'",
    );
}
