#[cfg(feature = "R4")]
use fhir_validation::profile::extract::extract_r4_structure_definition_profile;
#[cfg(feature = "R4B")]
use fhir_validation::profile::extract::extract_r4b_structure_definition_profile;
#[cfg(feature = "R5")]
use fhir_validation::profile::extract::extract_r5_structure_definition_profile;
#[cfg(feature = "R6")]
use fhir_validation::profile::extract::extract_r6_structure_definition_profile;
use fhir_validation::profile::types::ExtractedProfile;
use fhir_validation::{Severity, ValidationIssue};
use helios_fhir::{FhirResource, FhirVersion};
use std::fs;
use std::path::PathBuf;

pub fn fixture_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("r4")
        .join(rel)
}
#[allow(dead_code)]
pub fn fixture_path_r5(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("r5")
        .join(rel)
}

pub fn load_fixture(version: FhirVersion, rel: &str) -> String {
    let path = match version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => fixture_path(rel),
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => fixture_path(rel),
        #[cfg(feature = "R5")]
        FhirVersion::R5 => fixture_path_r5(rel),
        #[cfg(feature = "R6")]
        FhirVersion::R6 => fixture_path_r5(rel),
    };

    fs::read_to_string(path).unwrap_or_else(|e| panic!("failed to read fixture {rel}: {e}"))
}

#[allow(dead_code)]
pub fn load_resource(version: FhirVersion, rel: &str) -> FhirResource {
    let json = load_fixture(version, rel);

    match version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => {
            let r: helios_fhir::r4::Resource = serde_json::from_str(&json).unwrap();
            FhirResource::R4(Box::new(r))
        }

        #[cfg(feature = "R4B")]
        FhirVersion::R4B => {
            let r: helios_fhir::r4b::Resource = serde_json::from_str(&json).unwrap();
            FhirResource::R4B(Box::new(r))
        }

        #[cfg(feature = "R5")]
        FhirVersion::R5 => {
            let r: helios_fhir::r5::Resource = serde_json::from_str(&json).unwrap();
            FhirResource::R5(Box::new(r))
        }

        #[cfg(feature = "R6")]
        FhirVersion::R6 => {
            let r: helios_fhir::r6::Resource = serde_json::from_str(&json).unwrap();
            FhirResource::R6(Box::new(r))
        }
    }
}
#[allow(dead_code)]
pub fn load_profile(version: FhirVersion, rel: &str) -> ExtractedProfile {
    let json = load_fixture(version, rel);

    match version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => {
            let sd: helios_fhir::r4::StructureDefinition = serde_json::from_str(&json).unwrap();
            extract_r4_structure_definition_profile(&sd).unwrap()
        }

        #[cfg(feature = "R4B")]
        FhirVersion::R4B => {
            let sd: helios_fhir::r4b::StructureDefinition = serde_json::from_str(&json).unwrap();
            extract_r4b_structure_definition_profile(&sd).unwrap()
        }

        #[cfg(feature = "R5")]
        FhirVersion::R5 => {
            let sd: helios_fhir::r5::StructureDefinition = serde_json::from_str(&json).unwrap();
            extract_r5_structure_definition_profile(&sd).unwrap()
        }

        #[cfg(feature = "R6")]
        FhirVersion::R6 => {
            let sd: helios_fhir::r6::StructureDefinition = serde_json::from_str(&json).unwrap();
            extract_r6_structure_definition_profile(&sd).unwrap()
        }
    }
}
#[cfg(feature = "R5")]
#[allow(dead_code)]
pub fn load_r5_patient(rel: &str) -> helios_fhir::r5::Patient {
    let json = load_fixture(FhirVersion::R5, rel);
    serde_json::from_str(&json).unwrap()
}

/// Local terminology for synchronous validation tests: delegates to generated
/// `helios_fhir` ValueSet / code validation (same local-first path as production).
#[cfg(feature = "R4")]
#[allow(dead_code)] // Not every integration test crate that includes this module calls it.
pub fn local_terminology_r4() -> fhir_validation::LocalTerminologyService {
    fhir_validation::LocalTerminologyService::new(FhirVersion::R4)
}

#[cfg(feature = "R5")]
#[allow(dead_code)] // Not every integration test crate that includes this module calls it.
pub fn local_terminology_r5() -> fhir_validation::LocalTerminologyService {
    fhir_validation::LocalTerminologyService::new(FhirVersion::R5)
}
#[allow(dead_code)]
fn issue_matches_requested_severity(issue: &ValidationIssue, severity: Severity) -> bool {
    match severity {
        Severity::Warning => {
            matches!(issue.severity, Severity::Warning | Severity::Information)
        }
        Severity::Information => issue.severity == Severity::Information,
        Severity::Error => issue.severity == Severity::Error,
        Severity::Fatal => issue.severity == Severity::Fatal,
    }
}
#[allow(dead_code)]
fn assert_has_severity_like(issues: &[ValidationIssue], severity: Severity, label: &str) {
    assert!(
        issues
            .iter()
            .any(|issue| issue_matches_requested_severity(issue, severity)),
        "expected {label}, got issues: {issues:#?}"
    );
}
#[allow(dead_code)]
pub fn assert_has_invariant_issue(
    issues: &[ValidationIssue],
    path: &str,
    expression: &str,
    severity: Severity,
) {
    assert!(
        issues.iter().any(|issue| {
            issue.instance_path.as_deref() == Some(path)
                && issue.expression.as_deref() == Some(expression)
                && issue_matches_requested_severity(issue, severity)
        }),
        "expected issue at path {path:?} with expression {expression:?} and severity {severity:?}, got issues: {issues:#?}"
    );
}
#[allow(dead_code)]
pub fn assert_has_binding_issue(issues: &[ValidationIssue], path: &str, expression: &str) {
    assert!(
        issues.iter().any(|issue| {
            issue.instance_path.as_deref() == Some(path)
                && issue.expression.as_deref() == Some(expression)
        }),
        "expected issue at path {path:?}, for value set {expression:?}, got issues: {issues:#?}"
    );
}
#[allow(dead_code)]
pub fn assert_has_binding_issue_with_diagnostics(
    issues: &[ValidationIssue],
    path: &str,
    expression: &str,
    diag: &str,
) {
    assert!(
        issues.iter().any(|issue| {
            issue.instance_path.as_deref() == Some(path)
                && issue.expression.as_deref() == Some(expression)
                && issue.diagnostics.contains(diag)
        }),
        "expected issue at path {path:?}, for value set {expression:?}, got issues: {issues:#?}, expected diagnostics: {diag:?}"
    );
}
#[allow(dead_code)]
pub fn assert_has_error(issues: &[ValidationIssue]) {
    assert_has_severity_like(issues, Severity::Error, "error/fatal issue");
}
#[allow(dead_code)]
pub fn assert_has_warning(issues: &[ValidationIssue]) {
    assert_has_severity_like(issues, Severity::Warning, "warning/info issue");
}
#[allow(dead_code)]
pub fn assert_has_info(issues: &[ValidationIssue]) {
    assert_has_severity_like(issues, Severity::Information, "info issue");
}
#[allow(dead_code)]
pub fn assert_no_errors(issues: &[ValidationIssue]) {
    let errors: Vec<_> = issues
        .iter()
        .filter(|i| matches!(i.severity, Severity::Error | Severity::Fatal ))
        .collect();

    assert!(
        errors.is_empty(),
        "expected no errors, got issues: {:#?}",
        issues
    );
}
#[allow(dead_code)]
pub fn assert_no_errors_or_warnings(issues: &[ValidationIssue]) {
    let bad: Vec<_> = issues
        .iter()
        .filter(|i| {
            matches!(
                i.severity,
                Severity::Error | Severity::Fatal | Severity::Warning
            )
        })
        .collect();

    assert!(
        bad.is_empty(),
        "expected no errors/warnings, got issues: {:#?}",
        issues
    );
}

#[allow(dead_code)]
pub fn assert_issue_count(issues: &[ValidationIssue], expected: usize) {
    assert_eq!(
        issues.len(),
        expected,
        "expected {expected} issues, got issues: {issues:#?}"
    );
}
#[allow(dead_code)]
pub fn assert_has_issue_code(issues: &[ValidationIssue], code: &str) {
    assert!(
        issues.iter().any(|issue| issue.code == code),
        "expected issue code {code:?}, got issues: {issues:#?}"
    );
}
#[allow(dead_code)]
pub fn assert_has_issue_at_path(issues: &[ValidationIssue], code: &str, instance_path: &str) {
    assert!(
        issues.iter().any(|issue| {
            issue.code == code && issue.instance_path.as_deref() == Some(instance_path)
        }),
        "expected issue code {code:?} at path {instance_path:?}, got issues: {issues:#?}"
    );
}
#[allow(dead_code)]
pub fn assert_has_invariant(
    issues: &[ValidationIssue],
    instance_path: &str,
    diagnostics_contains: &str,
) {
    assert!(
        issues.iter().any(|issue| {
            issue.code == "invariant"
                && issue.instance_path.as_deref() == Some(instance_path)
                && issue.diagnostics.contains(diagnostics_contains)
        }),
        "expected invariant at path {instance_path:?} containing {diagnostics_contains:?}, got issues: {issues:#?}"
    );
}
#[allow(dead_code)]
pub fn assert_has_severity(issues: &[ValidationIssue], severity: Severity) {
    assert!(
        issues.iter().any(|issue| issue.severity == severity),
        "expected severity {severity:?}, got issues: {issues:#?}"
    );
}
#[allow(dead_code)]
pub fn assert_has_invariant_expression(
    issues: &[ValidationIssue],
    instance_path: &str,
    expression: &str,
) {
    assert!(
        issues.iter().any(|issue| {
            issue.code == "invariant"
                && issue.instance_path.as_deref() == Some(instance_path)
                && issue.expression.as_deref() == Some(expression)
        }),
        "expected invariant at path {instance_path:?} with expression {expression:?}, got issues: {issues:#?}"
    );
}
#[allow(dead_code)]
pub fn eval_r4_patient_expr(
    patient: &helios_fhir::r4::Patient,
    expr: &str,
) -> Result<Vec<helios_fhirpath_support::EvaluationResult>, String> {
    let root = helios_fhir::r4::Resource::Patient(Box::new(patient.clone()));
    let evaluator = fhir_validation::R4FhirPathEvaluator::new(root);

    evaluator
        .eval_expression(expr)
        .map_err(|e| format!("{e:?}"))
}
#[cfg(feature = "R5")]
#[allow(dead_code)]
pub fn eval_r5_patient_expr(
    patient: &helios_fhir::r5::Patient,
    expr: &str,
) -> Result<Vec<helios_fhirpath_support::EvaluationResult>, String> {
    let root = helios_fhir::r5::Resource::Patient(Box::new(patient.clone()));
    let evaluator = fhir_validation::R5FhirPathEvaluator::new(root);

    evaluator
        .eval_expression(expr)
        .map_err(|e| format!("{e:?}"))
}
