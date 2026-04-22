//! Integration tests: custom validation detail codes project correctly into OperationOutcome JSON.

use fhir_validation::issue_to_op_outcome::validation_issue_to_operation_outcome_issue;
use fhir_validation::{
    Severity, VALIDATION_ISSUE_DETAIL_SYSTEM, VALIDATION_ISSUE_DETAIL_VERSION, ValidationIssue,
    ValidationIssueDetailCode,
};
use serde_json::json;

fn minimal_issue(detail: ValidationIssueDetailCode, category_code: &str) -> ValidationIssue {
    ValidationIssue {
        severity: Severity::Error,
        code: category_code.to_string(),
        fhir_path: "Patient".to_string(),
        instance_path: Some("Patient".to_string()),
        expression: None,
        expression_kind: None,
        source_invariant_key: None,
        summary: None,
        detail_code: Some(detail),
        diagnostics: "diagnostics body".to_string(),
    }
}

#[test]
fn detail_code_sets_operation_outcome_details_coding() {
    let issue = minimal_issue(ValidationIssueDetailCode::RequiredBindingMiss, "value");
    let v = validation_issue_to_operation_outcome_issue(&issue);
    assert_eq!(
        v["details"]["coding"][0]["system"],
        json!(VALIDATION_ISSUE_DETAIL_SYSTEM)
    );
    assert_eq!(
        v["details"]["coding"][0]["version"],
        json!(VALIDATION_ISSUE_DETAIL_VERSION)
    );
    assert_eq!(v["details"]["coding"][0]["code"], "required-binding-miss");
    assert_eq!(
        v["details"]["coding"][0]["display"],
        "Required binding miss"
    );
}

#[test]
fn coarse_category_used_when_detail_code_absent() {
    let issue = ValidationIssue {
        detail_code: None,
        ..minimal_issue(ValidationIssueDetailCode::ValidationFailure, "required")
    };
    let v = validation_issue_to_operation_outcome_issue(&issue);
    assert_eq!(
        v["details"]["coding"][0]["code"],
        "required-element-missing"
    );
    assert_eq!(v["details"]["text"], "Required element is missing");
}

#[test]
fn detail_code_overrides_mismatched_category_for_coding() {
    // Category says "structure" but explicit detail is binding-related — coding follows detail_code.
    let issue = minimal_issue(
        ValidationIssueDetailCode::ExtensibleBindingMiss,
        "structure",
    );
    let v = validation_issue_to_operation_outcome_issue(&issue);
    assert_eq!(v["details"]["coding"][0]["code"], "extensible-binding-miss");
    assert_eq!(v["code"], "structure"); // FHIR issue type still from internal category mapping
}

#[test]
fn summary_overrides_details_text_even_with_detail_code() {
    let issue = ValidationIssue {
        summary: Some("Custom headline".to_string()),
        ..minimal_issue(ValidationIssueDetailCode::SliceOrderViolation, "structure")
    };
    let v = validation_issue_to_operation_outcome_issue(&issue);
    assert_eq!(v["details"]["text"], "Custom headline");
}

#[test]
fn pattern_and_slice_detail_codes_in_json() {
    let p = minimal_issue(
        ValidationIssueDetailCode::PatternConstraintMismatch,
        "value",
    );
    let pj = validation_issue_to_operation_outcome_issue(&p);
    assert_eq!(
        pj["details"]["coding"][0]["code"],
        "pattern-constraint-mismatch"
    );

    let s = minimal_issue(
        ValidationIssueDetailCode::SlicingNoDiscriminators,
        "business-rule",
    );
    let sj = validation_issue_to_operation_outcome_issue(&s);
    assert_eq!(
        sj["details"]["coding"][0]["code"],
        "slicing-no-discriminators"
    );
}
