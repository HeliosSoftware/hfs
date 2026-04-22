//! Convert internal `ValidationIssue` values into a FHIR OperationOutcome.
//!
//! The validation engine returns rich internal issues that are convenient for
//! generator-driven validation and testing. This module provides a FHIR-facing
//! projection of those issues so callers can expose validation results through
//! standard FHIR APIs such as `$validate`.
//!
//! Design notes:
//! - The validator continues to return `Vec<ValidationIssue>` as its canonical
//!   internal representation.
//! - OperationOutcome is treated as a derived presentation layer.
//! - This module builds a version-agnostic `OperationOutcome` JSON object, then
//!   optionally materializes typed [`helios_fhir`] `OperationOutcome` values for the
//!   FHIR release enabled via crate features (`R4`, `R4B`, `R5`, `R6`).
//!
//! Mapping policy:
//! - `ValidationIssue.severity` -> `OperationOutcome.issue.severity`
//! - internal validator `code` strings -> conservative FHIR `issue.code`
//! - prefer `instance_path` over `fhir_path` for `expression`
//! - `diagnostics` is surfaced as `issue.diagnostics`
//! - optional `ValidationIssue.summary` -> `issue.details.text` when present
//! - `ValidationIssue.expression` (when present) is not a location path; it is emitted as
//!   extensions: a [`crate::ValidationSourceKind`] (`valueCode`) plus `valueUri` for canonical
//!   URLs or `valueString` for FHIRPath / invariant ids / other.
//! - Optional `ValidationIssue.source_invariant_key` is emitted as a separate extension
//!   (`valueString`) so the FHIRPath in `expression` and the constraint key need not share one field.
//!
//! Unknown internal issue codes are mapped to `processing`.

use crate::ValidationIssue;
use crate::validation_issue_detail::{
    VALIDATION_ISSUE_DETAIL_SYSTEM, VALIDATION_ISSUE_DETAIL_VERSION, ValidationSourceKind,
    classify_validation_source,
};
use fhir_validation_types::Severity;
use serde_json::{Value, json};

fn severity_to_fhir(value: Severity) -> &'static str {
    match value {
        Severity::Fatal => "fatal",
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Information => "information",
    }
}

/// Map internal validator codes to FHIR `OperationOutcome.issue.code` strings.
///
/// Aligns with the FHIR *Issue Type* value set codes (see `helios_fhir` generated
/// `IssueType` per release). Unknown or internal-only codes fall back to `processing`.
fn code_to_fhir(code: &str) -> &'static str {
    match code {
        "invalid" => "invalid",
        "structure" => "structure",
        "required" => "required",
        "value" => "value",
        "terminology" => "code-invalid",
        "invariant" => "invariant",
        "security" => "security",
        "login" => "login",
        "unknown" => "unknown",
        "expired" => "expired",
        "forbidden" => "forbidden",
        "suppressed" => "suppressed",
        "processing" => "processing",
        "not-supported" => "not-supported",
        "duplicate" => "duplicate",
        "multiple-matches" => "multiple-matches",
        "not-found" => "not-found",
        "deleted" => "deleted",
        "too-long" => "too-long",
        "code-invalid" => "code-invalid",
        "extension" => "extension",
        "too-costly" => "too-costly",
        "business-rule" => "business-rule",
        "conflict" => "conflict",
        "limited-filter" => "limited-filter",
        "transient" => "transient",
        "lock-error" => "lock-error",
        "no-store" => "no-store",
        "exception" => "exception",
        "timeout" => "timeout",
        "incomplete" => "incomplete",
        "throttled" => "throttled",
        "informational" => "informational",
        "success" => "success",
        _ => "processing",
    }
}

fn strip_numeric_indexes(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'[' {
            let start = i;
            i += 1;
            let digits_start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i > digits_start && i < bytes.len() && bytes[i] == b']' {
                i += 1;
                continue;
            }
            out.push_str(&path[start..i]);
            continue;
        }

        out.push(bytes[i] as char);
        i += 1;
    }

    out
}

fn starts_with_valid_context(expr: &str) -> bool {
    if expr == "$this" || expr.starts_with("$this.") {
        return true;
    }

    let Some(first) = expr.split('.').next() else {
        return false;
    };

    !first.is_empty()
        && first
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false)
        && first.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn segment_is_allowed(segment: &str) -> bool {
    if segment.is_empty() {
        return false;
    }

    if segment == "resolve()" {
        return true;
    }

    if let Some(name) = segment
        .strip_prefix("ofType(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        return !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    }

    if let Some(url) = segment
        .strip_prefix("extension(\"")
        .and_then(|rest| rest.strip_suffix("\")"))
    {
        return !url.is_empty();
    }

    segment
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn split_simple_fhirpath_segments(expr: &str) -> Option<Vec<&str>> {
    let mut segments = Vec::new();
    let mut start = 0usize;
    let mut paren_depth = 0usize;
    let mut in_string = false;
    let mut prev_was_escape = false;

    for (idx, ch) in expr.char_indices() {
        match ch {
            '"' if !prev_was_escape => {
                in_string = !in_string;
            }
            '(' if !in_string => {
                paren_depth += 1;
            }
            ')' if !in_string => {
                if paren_depth == 0 {
                    return None;
                }
                paren_depth -= 1;
            }
            '.' if !in_string && paren_depth == 0 => {
                let segment = &expr[start..idx];
                if segment.is_empty() {
                    return None;
                }
                segments.push(segment);
                start = idx + 1;
            }
            _ => {}
        }

        prev_was_escape = ch == '\\' && !prev_was_escape;
        if ch != '\\' {
            prev_was_escape = false;
        }
    }

    if in_string || paren_depth != 0 {
        return None;
    }

    let tail = &expr[start..];
    if tail.is_empty() {
        return None;
    }
    segments.push(tail);
    Some(segments)
}

fn is_simple_fhirpath_subset_expression(expr: &str) -> bool {
    let expr = expr.trim();
    if expr.is_empty() || !starts_with_valid_context(expr) {
        return false;
    }

    if expr.contains(' ') || expr.contains('\t') || expr.contains('\n') || expr.contains('\r') {
        return false;
    }

    if expr.contains('[') || expr.contains(']') {
        return false;
    }

    split_simple_fhirpath_segments(expr)
        .map(|segments| segments.into_iter().all(segment_is_allowed))
        .unwrap_or(false)
}

fn normalize_operation_outcome_expression(path: &str) -> Option<String> {
    let normalized = strip_numeric_indexes(path.trim());
    (!normalized.is_empty() && is_simple_fhirpath_subset_expression(&normalized))
        .then_some(normalized)
}

fn issue_location_expression(issue: &ValidationIssue) -> Option<String> {
    issue
        .instance_path
        .as_deref()
        .and_then(normalize_operation_outcome_expression)
        .or_else(|| normalize_operation_outcome_expression(&issue.fhir_path))
}

/// URL for `OperationOutcome.issue.extension` carrying the classified source (`valueUri` / `valueString`).
pub const VALIDATION_SOURCE_EXPRESSION_URL: &str =
    "https://atrius.health/fhir/StructureDefinition/validation-source-expression";
/// URL for `OperationOutcome.issue.extension` carrying [`ValidationSourceKind`] as `valueCode`.
pub const VALIDATION_SOURCE_KIND_URL: &str =
    "https://atrius.health/fhir/StructureDefinition/validation-source-kind";
/// URL for `OperationOutcome.issue.extension` carrying the constraint / invariant key (`valueString`).
pub const VALIDATION_SOURCE_INVARIANT_KEY_URL: &str =
    "https://atrius.health/fhir/StructureDefinition/validation-source-invariant-key";

fn details_text(issue: &ValidationIssue) -> String {
    issue
        .summary
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            issue
                .resolved_detail_code()
                .details_text_fallback()
                .to_string()
        })
}

fn source_expression_extension_json(issue: &ValidationIssue) -> Option<Value> {
    let mut extensions = Vec::new();

    if let Some(raw) = issue
        .expression
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let kind = issue
            .expression_kind
            .unwrap_or_else(|| classify_validation_source(raw));
        extensions.push(json!({
            "url": VALIDATION_SOURCE_KIND_URL,
            "valueCode": kind.as_code(),
        }));
        let value_ext = match kind {
            ValidationSourceKind::CanonicalUri => json!({
                "url": VALIDATION_SOURCE_EXPRESSION_URL,
                "valueUri": raw,
            }),
            ValidationSourceKind::FhirPath
            | ValidationSourceKind::InvariantId
            | ValidationSourceKind::Unclassified => json!({
                "url": VALIDATION_SOURCE_EXPRESSION_URL,
                "valueString": raw,
            }),
        };
        extensions.push(value_ext);
    }

    if let Some(key) = issue
        .source_invariant_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        extensions.push(json!({
            "url": VALIDATION_SOURCE_INVARIANT_KEY_URL,
            "valueString": key,
        }));
    }

    if extensions.is_empty() {
        None
    } else {
        Some(json!(extensions))
    }
}

fn details_json(issue: &ValidationIssue) -> Value {
    let detail = issue.resolved_detail_code();
    let codings = vec![json!({
        "system": VALIDATION_ISSUE_DETAIL_SYSTEM,
        "version": VALIDATION_ISSUE_DETAIL_VERSION,
        "code": detail.as_code(),
        "display": detail.display(),
    })];

    json!({
        "coding": codings,
        "text": details_text(issue),
    })
}

/// Convert a single `ValidationIssue` into an `OperationOutcome.issue` JSON object.
pub fn validation_issue_to_operation_outcome_issue(issue: &ValidationIssue) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert(
        "severity".to_string(),
        Value::String(severity_to_fhir(issue.severity).to_string()),
    );
    obj.insert(
        "code".to_string(),
        Value::String(code_to_fhir(&issue.code).to_string()),
    );
    obj.insert("details".to_string(), details_json(issue));
    if !issue.diagnostics.is_empty() {
        obj.insert(
            "diagnostics".to_string(),
            Value::String(issue.diagnostics.clone()),
        );
    }

    if let Some(expression) = issue_location_expression(issue) {
        obj.insert("expression".to_string(), json!([expression]));
    }

    if let Some(extension) = source_expression_extension_json(issue) {
        obj.insert("extension".to_string(), extension);
    }

    Value::Object(obj)
}

/// Convert a slice of validation issues into a FHIR `OperationOutcome` JSON document.
///
/// This projection is shared across FHIR releases; use
/// [`validation_issues_to_r4_operation_outcome`] (and the `r4b` / `r5` / `r6`
/// siblings) when you need a typed [`helios_fhir`] model for a specific version.
pub fn validation_issues_to_operation_outcome(issues: &[ValidationIssue]) -> Value {
    let converted: Vec<Value> = issues
        .iter()
        .map(validation_issue_to_operation_outcome_issue)
        .collect();

    json!({
        "resourceType": "OperationOutcome",
        "issue": converted,
    })
}

/// Deserialize [`validation_issues_to_operation_outcome`] into the R4 `OperationOutcome` type.
#[cfg(feature = "R4")]
pub fn validation_issues_to_r4_operation_outcome(
    issues: &[ValidationIssue],
) -> Result<helios_fhir::r4::OperationOutcome, serde_json::Error> {
    serde_json::from_value(validation_issues_to_operation_outcome(issues))
}

/// Serialize an R4 [`helios_fhir::r4::OperationOutcome`] as a versioned [`helios_fhir::r4::Resource`] JSON value.
#[cfg(feature = "R4")]
pub fn validation_issues_to_r4_operation_outcome_json(
    issues: &[ValidationIssue],
) -> Result<Value, serde_json::Error> {
    let outcome = validation_issues_to_r4_operation_outcome(issues)?;
    serde_json::to_value(helios_fhir::r4::Resource::OperationOutcome(Box::new(
        outcome,
    )))
}

/// Deserialize [`validation_issues_to_operation_outcome`] into the R4B `OperationOutcome` type.
#[cfg(feature = "R4B")]
pub fn validation_issues_to_r4b_operation_outcome(
    issues: &[ValidationIssue],
) -> Result<helios_fhir::r4b::OperationOutcome, serde_json::Error> {
    serde_json::from_value(validation_issues_to_operation_outcome(issues))
}

/// Serialize an R4B [`helios_fhir::r4b::OperationOutcome`] as a versioned [`helios_fhir::r4b::Resource`] JSON value.
#[cfg(feature = "R4B")]
pub fn validation_issues_to_r4b_operation_outcome_json(
    issues: &[ValidationIssue],
) -> Result<Value, serde_json::Error> {
    let outcome = validation_issues_to_r4b_operation_outcome(issues)?;
    serde_json::to_value(helios_fhir::r4b::Resource::OperationOutcome(Box::new(
        outcome,
    )))
}

/// Deserialize [`validation_issues_to_operation_outcome`] into the R5 `OperationOutcome` type.
#[cfg(feature = "R5")]
pub fn validation_issues_to_r5_operation_outcome(
    issues: &[ValidationIssue],
) -> Result<helios_fhir::r5::OperationOutcome, serde_json::Error> {
    serde_json::from_value(validation_issues_to_operation_outcome(issues))
}

/// Serialize an R5 [`helios_fhir::r5::OperationOutcome`] as a versioned [`helios_fhir::r5::Resource`] JSON value.
#[cfg(feature = "R5")]
pub fn validation_issues_to_r5_operation_outcome_json(
    issues: &[ValidationIssue],
) -> Result<Value, serde_json::Error> {
    let outcome = validation_issues_to_r5_operation_outcome(issues)?;
    serde_json::to_value(helios_fhir::r5::Resource::OperationOutcome(Box::new(
        outcome,
    )))
}

/// Deserialize [`validation_issues_to_operation_outcome`] into the R6 `OperationOutcome` type.
#[cfg(feature = "R6")]
pub fn validation_issues_to_r6_operation_outcome(
    issues: &[ValidationIssue],
) -> Result<helios_fhir::r6::OperationOutcome, serde_json::Error> {
    serde_json::from_value(validation_issues_to_operation_outcome(issues))
}

/// Serialize an R6 [`helios_fhir::r6::OperationOutcome`] as a versioned [`helios_fhir::r6::Resource`] JSON value.
#[cfg(feature = "R6")]
pub fn validation_issues_to_r6_operation_outcome_json(
    issues: &[ValidationIssue],
) -> Result<Value, serde_json::Error> {
    let outcome = validation_issues_to_r6_operation_outcome(issues)?;
    serde_json::to_value(helios_fhir::r6::Resource::OperationOutcome(Box::new(
        outcome,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue_code;

    fn mk_issue(
        severity: Severity,
        code: &str,
        fhir_path: &str,
        instance_path: Option<&str>,
        expression: Option<&str>,
        diagnostics: &str,
    ) -> ValidationIssue {
        ValidationIssue {
            severity,
            code: code.to_string(),
            fhir_path: fhir_path.to_string(),
            instance_path: instance_path.map(str::to_string),
            expression: expression.map(str::to_string),
            expression_kind: None,
            source_invariant_key: None,
            summary: None,
            detail_code: None,
            diagnostics: diagnostics.to_string(),
        }
    }

    #[test]
    fn converts_single_issue_to_operation_outcome_issue() {
        let issue = mk_issue(
            Severity::Error,
            "value",
            "Patient.gender",
            Some("Patient.gender"),
            Some("http://hl7.org/fhir/ValueSet/administrative-gender|4.0.1"),
            "Gender code is invalid",
        );

        let json = validation_issue_to_operation_outcome_issue(&issue);
        assert_eq!(json["severity"], "error");
        assert_eq!(json["code"], "value");
        assert_eq!(
            json["details"]["text"],
            "Element value is invalid for the expected constraint"
        );
        assert_eq!(json["diagnostics"], "Gender code is invalid");
        assert_eq!(json["expression"][0], "Patient.gender");
        assert_eq!(json["extension"][0]["valueCode"], "canonical-uri");
        assert_eq!(
            json["extension"][1]["valueUri"],
            "http://hl7.org/fhir/ValueSet/administrative-gender|4.0.1"
        );
    }

    #[test]
    fn prefers_instance_path_over_fhir_path() {
        let issue = mk_issue(
            Severity::Error,
            "invariant",
            "Reference",
            Some("Slot.schedule"),
            None,
            "Constraint failed",
        );

        let json = validation_issue_to_operation_outcome_issue(&issue);
        assert_eq!(json["expression"][0], "Slot.schedule");
    }

    #[test]
    fn falls_back_to_fhir_path_when_instance_path_absent() {
        let issue = mk_issue(
            Severity::Warning,
            "invariant",
            "Observation",
            None,
            None,
            "Constraint failed",
        );

        let json = validation_issue_to_operation_outcome_issue(&issue);
        assert_eq!(json["expression"][0], "Observation");
    }

    #[test]
    fn omits_expression_when_instance_path_and_fhir_path_are_empty() {
        let issue = mk_issue(
            Severity::Warning,
            "invariant",
            "",
            Some(""),
            None,
            "Constraint failed",
        );

        let json = validation_issue_to_operation_outcome_issue(&issue);
        assert!(json.get("expression").is_none());
    }

    #[test]
    fn strips_numeric_indexes_from_instance_path_for_operation_outcome_expression() {
        let issue = mk_issue(
            Severity::Error,
            "value",
            "Patient.identifier",
            Some("Patient.identifier[0].assigner.display"),
            None,
            "Identifier assigner display is invalid",
        );

        let json = validation_issue_to_operation_outcome_issue(&issue);
        assert_eq!(json["expression"][0], "Patient.identifier.assigner.display");
    }

    #[test]
    fn omits_expression_when_path_uses_disallowed_fhirpath_operators() {
        let issue = mk_issue(
            Severity::Error,
            "invariant",
            "Patient.name.where(use='official')",
            None,
            None,
            "Constraint failed",
        );

        let json = validation_issue_to_operation_outcome_issue(&issue);
        assert!(json.get("expression").is_none());
    }

    #[test]
    fn keeps_allowed_simple_subset_functions_in_expression() {
        let issue = mk_issue(
            Severity::Error,
            "structure",
            "Patient.managingOrganization.resolve().ofType(Organization).extension(\"http://example.org/ext\")",
            None,
            None,
            "Reference target is invalid",
        );
        println!("{:?}", &issue);
        let json = validation_issue_to_operation_outcome_issue(&issue);
        assert_eq!(
            json["expression"][0],
            "Patient.managingOrganization.resolve().ofType(Organization).extension(\"http://example.org/ext\")"
        );
    }

    #[test]
    fn keeps_source_expression_outside_expression_location() {
        let issue = mk_issue(
            Severity::Error,
            "invariant",
            "",
            None,
            Some("Patient.name.where(use='official').exists()"),
            "Constraint failed",
        );

        let json = validation_issue_to_operation_outcome_issue(&issue);
        assert!(json.get("expression").is_none());
        assert_eq!(json["extension"][0]["valueCode"], "fhirpath");
        assert_eq!(
            json["extension"][1]["valueString"],
            "Patient.name.where(use='official').exists()"
        );
    }

    #[test]
    fn classifies_invariant_id_expression() {
        let issue = mk_issue(
            Severity::Error,
            "invariant",
            "Patient",
            Some("Patient"),
            Some("ele-1"),
            "Constraint failed",
        );
        let json = validation_issue_to_operation_outcome_issue(&issue);
        assert_eq!(json["extension"][0]["valueCode"], "invariant-id");
        assert_eq!(json["extension"][1]["valueString"], "ele-1");
    }

    #[test]
    fn emits_invariant_key_extension_alongside_fhirpath_expression() {
        use crate::validation_issue_detail::ValidationSourceKind;

        let issue = ValidationIssue {
            severity: Severity::Error,
            code: issue_code::INVARIANT.to_string(),
            fhir_path: "Patient".to_string(),
            instance_path: Some("Patient".to_string()),
            expression: Some("Patient.name.empty()".to_string()),
            expression_kind: Some(ValidationSourceKind::FhirPath),
            source_invariant_key: Some("dom-2".to_string()),
            summary: None,
            detail_code: None,
            diagnostics: "Constraint failed".to_string(),
        };
        let json = validation_issue_to_operation_outcome_issue(&issue);
        assert_eq!(json["extension"][0]["valueCode"], "fhirpath");
        assert_eq!(json["extension"][1]["valueString"], "Patient.name.empty()");
        assert_eq!(
            json["extension"][2]["url"],
            VALIDATION_SOURCE_INVARIANT_KEY_URL
        );
        assert_eq!(json["extension"][2]["valueString"], "dom-2");
    }
    #[test]
    fn maps_invariant_code_to_issue_type_invariant() {
        let issue = mk_issue(
            Severity::Error,
            "invariant",
            "Observation",
            Some("Observation"),
            None,
            "Constraint failed",
        );

        let json = validation_issue_to_operation_outcome_issue(&issue);
        assert_eq!(json["code"], "invariant");
    }

    #[test]
    fn maps_terminology_code_to_code_invalid() {
        let issue = mk_issue(
            Severity::Error,
            issue_code::TERMINOLOGY,
            "Patient",
            Some("Patient"),
            None,
            "ValueSet membership failed",
        );

        let json = validation_issue_to_operation_outcome_issue(&issue);
        assert_eq!(json["code"], "code-invalid");
    }

    #[test]
    fn maps_unknown_code_to_processing() {
        let issue = mk_issue(
            Severity::Error,
            "some-internal-code",
            "Observation",
            Some("Observation"),
            None,
            "Unexpected validator condition",
        );

        let json = validation_issue_to_operation_outcome_issue(&issue);
        assert_eq!(json["code"], "processing");
    }
    #[test]
    fn converts_multiple_issues_to_operation_outcome() {
        let issues = vec![
            mk_issue(
                Severity::Error,
                "value",
                "Patient.gender",
                Some("Patient.gender"),
                None,
                "Gender code is invalid",
            ),
            mk_issue(
                Severity::Warning,
                "invariant",
                "Patient",
                Some("Patient"),
                None,
                "Narrative should be present",
            ),
        ];

        let json = validation_issues_to_operation_outcome(&issues);
        assert_eq!(json["resourceType"], "OperationOutcome");
        assert_eq!(json["issue"].as_array().unwrap().len(), 2);
        assert_eq!(json["issue"][0]["severity"], "error");
        assert_eq!(json["issue"][1]["severity"], "warning");
        assert_eq!(json["issue"][0]["expression"][0], "Patient.gender");
        assert_eq!(json["issue"][1]["expression"][0], "Patient");
        assert_eq!(json["issue"][0]["diagnostics"], "Gender code is invalid");
        assert_eq!(
            json["issue"][0]["details"]["coding"][0]["code"],
            "element-value-invalid"
        );
        assert_eq!(
            json["issue"][0]["details"]["coding"][0]["system"],
            VALIDATION_ISSUE_DETAIL_SYSTEM
        );
    }

    #[test]
    fn summary_overrides_synthesized_details_text() {
        let issue = mk_issue(
            Severity::Error,
            "value",
            "Patient.gender",
            Some("Patient.gender"),
            None,
            "Longer explanation for implementers",
        )
        .with_summary(
            "Does not satisfy the required value set binding (verify system, code, and display)",
        );

        let json = validation_issue_to_operation_outcome_issue(&issue);
        assert_eq!(
            json["details"]["text"],
            "Does not satisfy the required value set binding (verify system, code, and display)"
        );
    }
    #[cfg(feature = "R5")]
    #[test]
    fn converts_multiple_issues_to_typed_r5_operation_outcome() {
        let issues = vec![
            mk_issue(
                Severity::Error,
                "value",
                "Patient.gender",
                Some("Patient.gender"),
                None,
                "Gender code is invalid",
            ),
            mk_issue(
                Severity::Warning,
                "invariant",
                "Patient",
                Some("Patient"),
                None,
                "Narrative should be present",
            ),
        ];

        let outcome = validation_issues_to_r5_operation_outcome(&issues)
            .expect("typed R5 OperationOutcome should deserialize from generated JSON");
        let json = serde_json::to_value(helios_fhir::r5::Resource::OperationOutcome(Box::new(
            outcome,
        )))
        .expect("typed R5 OperationOutcome should serialize back to JSON");

        assert_eq!(json["resourceType"], "OperationOutcome");
        assert_eq!(json["issue"].as_array().unwrap().len(), 2);
        assert_eq!(json["issue"][0]["severity"], "error");
        assert_eq!(json["issue"][1]["severity"], "warning");
        assert_eq!(json["issue"][0]["expression"][0], "Patient.gender");
        assert_eq!(json["issue"][1]["expression"][0], "Patient");
    }

    #[cfg(feature = "R5")]
    #[test]
    fn r5_operation_outcome_json_helper_matches_direct_json_projection() {
        let issues = vec![mk_issue(
            Severity::Error,
            "value",
            "Patient.gender",
            Some("Patient.gender"),
            Some("http://hl7.org/fhir/ValueSet/administrative-gender|4.0.1"),
            "Gender code is invalid",
        )];

        let direct = validation_issues_to_operation_outcome(&issues);
        let typed_json = validation_issues_to_r5_operation_outcome_json(&issues)
            .expect("typed R5 OperationOutcome JSON helper should succeed");

        assert_eq!(typed_json, direct);
        assert_eq!(
            typed_json["issue"][0]["diagnostics"],
            "Gender code is invalid"
        );
        assert_eq!(
            typed_json["issue"][0]["extension"][0]["valueCode"],
            "canonical-uri"
        );
        assert_eq!(
            typed_json["issue"][0]["extension"][1]["valueUri"],
            "http://hl7.org/fhir/ValueSet/administrative-gender|4.0.1"
        );
    }

    // Run with e.g. `cargo test -p fhir-validation --features "R4,R4B"` — `R4B` alone
    // does not compile here because `helios-fhirpath` still matches on `FhirVersion::R4`
    // when the R4 module is omitted from `helios-fhir`.
    #[cfg(feature = "R4B")]
    #[test]
    fn converts_multiple_issues_to_typed_r4b_operation_outcome() {
        let issues = vec![
            mk_issue(
                Severity::Error,
                "value",
                "Patient.gender",
                Some("Patient.gender"),
                None,
                "Gender code is invalid",
            ),
            mk_issue(
                Severity::Warning,
                "invariant",
                "Patient",
                Some("Patient"),
                None,
                "Narrative should be present",
            ),
        ];

        let outcome = validation_issues_to_r4b_operation_outcome(&issues)
            .expect("typed R4B OperationOutcome should deserialize from generated JSON");
        let json = serde_json::to_value(helios_fhir::r4b::Resource::OperationOutcome(Box::new(
            outcome,
        )))
        .expect("typed R4B OperationOutcome should serialize back to JSON");

        assert_eq!(json["resourceType"], "OperationOutcome");
        assert_eq!(json["issue"].as_array().unwrap().len(), 2);
        assert_eq!(json["issue"][0]["severity"], "error");
        assert_eq!(json["issue"][1]["severity"], "warning");
        assert_eq!(json["issue"][0]["expression"][0], "Patient.gender");
        assert_eq!(json["issue"][1]["expression"][0], "Patient");
    }

    #[cfg(feature = "R4B")]
    #[test]
    fn r4b_operation_outcome_json_helper_matches_direct_json_projection() {
        let issues = vec![mk_issue(
            Severity::Error,
            "value",
            "Patient.gender",
            Some("Patient.gender"),
            Some("http://hl7.org/fhir/ValueSet/administrative-gender|4.0.1"),
            "Gender code is invalid",
        )];

        let direct = validation_issues_to_operation_outcome(&issues);
        let typed_json = validation_issues_to_r4b_operation_outcome_json(&issues)
            .expect("typed R4B OperationOutcome JSON helper should succeed");

        assert_eq!(typed_json, direct);
        assert_eq!(
            typed_json["issue"][0]["diagnostics"],
            "Gender code is invalid"
        );
        assert_eq!(
            typed_json["issue"][0]["extension"][0]["valueCode"],
            "canonical-uri"
        );
        assert_eq!(
            typed_json["issue"][0]["extension"][1]["valueUri"],
            "http://hl7.org/fhir/ValueSet/administrative-gender|4.0.1"
        );
    }

    // Same as R4B: use `--features "R4,R6"` (not `R6` alone) for this workspace.
    #[cfg(feature = "R6")]
    #[test]
    fn converts_multiple_issues_to_typed_r6_operation_outcome() {
        let issues = vec![
            mk_issue(
                Severity::Error,
                "value",
                "Patient.gender",
                Some("Patient.gender"),
                None,
                "Gender code is invalid",
            ),
            mk_issue(
                Severity::Warning,
                "invariant",
                "Patient",
                Some("Patient"),
                None,
                "Narrative should be present",
            ),
        ];

        let outcome = validation_issues_to_r6_operation_outcome(&issues)
            .expect("typed R6 OperationOutcome should deserialize from generated JSON");
        let json = serde_json::to_value(helios_fhir::r6::Resource::OperationOutcome(Box::new(
            outcome,
        )))
        .expect("typed R6 OperationOutcome should serialize back to JSON");

        assert_eq!(json["resourceType"], "OperationOutcome");
        assert_eq!(json["issue"].as_array().unwrap().len(), 2);
        assert_eq!(json["issue"][0]["severity"], "error");
        assert_eq!(json["issue"][1]["severity"], "warning");
        assert_eq!(json["issue"][0]["expression"][0], "Patient.gender");
        assert_eq!(json["issue"][1]["expression"][0], "Patient");
    }

    #[cfg(feature = "R6")]
    #[test]
    fn r6_operation_outcome_json_helper_matches_direct_json_projection() {
        let issues = vec![mk_issue(
            Severity::Error,
            "value",
            "Patient.gender",
            Some("Patient.gender"),
            Some("http://hl7.org/fhir/ValueSet/administrative-gender|4.0.1"),
            "Gender code is invalid",
        )];

        let direct = validation_issues_to_operation_outcome(&issues);
        let typed_json = validation_issues_to_r6_operation_outcome_json(&issues)
            .expect("typed R6 OperationOutcome JSON helper should succeed");

        assert_eq!(typed_json, direct);
        assert_eq!(
            typed_json["issue"][0]["diagnostics"],
            "Gender code is invalid"
        );
        assert_eq!(
            typed_json["issue"][0]["extension"][0]["valueCode"],
            "canonical-uri"
        );
        assert_eq!(
            typed_json["issue"][0]["extension"][1]["valueUri"],
            "http://hl7.org/fhir/ValueSet/administrative-gender|4.0.1"
        );
    }
}
