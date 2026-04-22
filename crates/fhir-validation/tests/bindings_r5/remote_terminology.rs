//! Async validation with [`RemoteTerminologyService`] against a **live** FHIR terminology server.
//!
//! These tests are **not** ignored: they expect a reachable server (e.g. Snowstorm on
//! `http://localhost:8080/fhir`, or set `FHIR_TERMINOLOGY_BASE_URL`).
//!
//! ```text
//! cargo test -p fhir-validation --features R5 --test bindings_r5
//! ```

use crate::common::fhir_json_examples::load_r5_fhir_resource;
use crate::common::fixtures::{
    assert_has_binding_issue, assert_has_binding_issue_with_diagnostics, load_resource,
};
use crate::harness::{r5_evaluator_for, remote_terminology_for_tests};
use fhir_validation::Validator;
use helios_fhir::FhirVersion;

#[tokio::test]
async fn patient_invalid_identifier_bindings_remote() {
    let resource = load_resource(FhirVersion::R5, "invalid/patient/patient-bindings.json");
    let evaluator = r5_evaluator_for(&resource);
    let term = remote_terminology_for_tests();
    let issues = Validator::default()
        .validate_resource_async(&resource, Some(&term), &evaluator)
        .await;
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

/// Same `meta.tag` / `common-tags` scenario as HL7 examples; remote server returns a definitive miss message.
#[tokio::test]
async fn coverage_eligibility_meta_tag_remote_validate_code() {
    let resource = load_r5_fhir_resource("coverageeligibilityrequest-example.json");
    let evaluator = r5_evaluator_for(&resource);
    let term = remote_terminology_for_tests();
    let issues = Validator::default()
        .validate_resource_async(&resource, Some(&term), &evaluator)
        .await;
    assert_has_binding_issue_with_diagnostics(
        &issues,
        "CoverageEligibilityRequest.meta.tag[0]",
        "http://hl7.org/fhir/ValueSet/common-tags",
        "The system 'http://terminology.hl7.org/CodeSystem/v3-ActReason' is not included in this ValueSet.",
    );
}
