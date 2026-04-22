//! Tests for [`fhir_validation::validation_error_to_issues`].

use fhir_validation::issue_code;
use fhir_validation::{
    MalformedValidateCodeParameters, RemoteTerminologyError, TerminologyIssueContext,
    TerminologyRequestInvalid, ValidationError, Validator, validation_error_to_issues,
};
use fhir_validation_types::BindingStrength;
use helios_fhir::TerminologyValidationError;

const FHIR_PATH: &str = "Patient.gender";
const VS: &str = "http://hl7.org/fhir/ValueSet/administrative-gender";

#[test]
fn local_terminology_maps_through_local_error_to_issues() {
    let validator = Validator::default();
    let ctx = TerminologyIssueContext::new(&validator, FHIR_PATH, VS, BindingStrength::Required);
    let err = ValidationError::LocalTerminology(TerminologyValidationError::WrongDisplay {
        system: "http://hl7.org/fhir/administrative-gender".to_string(),
        code: "male".to_string(),
        expected: "Male".to_string(),
        provided: "wrong".to_string(),
    });
    let issues = validation_error_to_issues(&ctx, &err);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, issue_code::VALUE);
    assert!(issues[0].diagnostics.contains("Wrong display"));
}

#[test]
fn remote_terminology_produces_terminology_issue() {
    let validator = Validator::default();
    let ctx = TerminologyIssueContext::new(&validator, FHIR_PATH, VS, BindingStrength::Required);
    let err = ValidationError::RemoteTerminology(RemoteTerminologyError::MalformedResponse(
        MalformedValidateCodeParameters::MissingParameterArray,
    ));
    let issues = validation_error_to_issues(&ctx, &err);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, issue_code::TERMINOLOGY);
    assert!(issues[0].diagnostics.contains("parameter"));
}

#[test]
fn invalid_request_produces_terminology_issue() {
    let validator = Validator::default();
    let ctx = TerminologyIssueContext::new(&validator, FHIR_PATH, VS, BindingStrength::Required);
    let err = ValidationError::InvalidRequest(TerminologyRequestInvalid {
        message: "missing ValueSet URL".to_string(),
    });
    let issues = err.to_binding_issues(&ctx);
    let issues_fn = validation_error_to_issues(&ctx, &err);
    assert_eq!(issues.len(), issues_fn.len());
    assert_eq!(issues[0].code, issues_fn[0].code);
    assert_eq!(issues[0].diagnostics, issues_fn[0].diagnostics);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, issue_code::TERMINOLOGY);
    assert_eq!(issues[0].diagnostics, "missing ValueSet URL");
}

#[test]
fn fhir_path_produces_exception_issue() {
    let validator = Validator::default();
    let ctx = TerminologyIssueContext::new(&validator, FHIR_PATH, VS, BindingStrength::Required);
    let inner = helios_fhirpath_support::EvaluationError::TypeError("x".to_string());
    let err = ValidationError::FhirPath(inner);
    let issues = validation_error_to_issues(&ctx, &err);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, issue_code::EXCEPTION);
}
