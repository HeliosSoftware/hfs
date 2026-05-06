use crate::issue_code;
use crate::profile::types::ExtractedElementRule;
use crate::{Severity, ValidationConfig, ValidationIssue, ValidationIssueDetailCode};
use serde::Serialize;
use serde_json::Value;

pub fn validate_min_cardinality<T: Serialize>(
    resource: &T,
    resource_type: &str,
    rules: &[ExtractedElementRule],
) -> Vec<ValidationIssue> {
    let root = match serde_json::to_value(resource) {
        Ok(value) => value,
        Err(err) => {
            return vec![ValidationIssue {
                severity: Severity::Error,
                code: "processing".to_string(),
                summary: Some(
                    "Resource could not be serialized for cardinality validation".to_string(),
                ),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::ValidationException),
                diagnostics: format!(
                    "Failed to serialize resource while validating profile cardinality: {}",
                    err
                ),
                expression: None,
                fhir_path: "".to_string(),
                instance_path: None,
            }];
        }
    };

    validate_min_cardinality_from_json(&root, resource_type, rules)
}

pub fn validate_max_cardinality<T: Serialize>(
    resource: &T,
    resource_type: &str,
    rules: &[ExtractedElementRule],
) -> Vec<ValidationIssue> {
    let root = match serde_json::to_value(resource) {
        Ok(value) => value,
        Err(err) => {
            return vec![ValidationIssue {
                severity: Severity::Error,
                code: "processing".to_string(),
                summary: Some(
                    "Resource could not be serialized for cardinality validation".to_string(),
                ),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::ValidationException),
                diagnostics: format!(
                    "Failed to serialize resource while validating profile cardinality: {}",
                    err
                ),
                expression: None,
                fhir_path: "".to_string(),
                instance_path: None,
            }];
        }
    };

    validate_max_cardinality_from_json(&root, resource_type, rules)
}

/// Warn or error when `ElementDefinition.mustSupport` is true but the instance has
/// no values at the rule path (same path resolution as cardinality).
pub fn validate_must_support<T: Serialize>(
    resource: &T,
    resource_type: &str,
    rules: &[ExtractedElementRule],
    config: &ValidationConfig,
) -> Vec<ValidationIssue> {
    if !config.validate_must_support {
        return Vec::new();
    }

    let root = match serde_json::to_value(resource) {
        Ok(value) => value,
        Err(err) => {
            return vec![ValidationIssue {
                severity: Severity::Error,
                code: "processing".to_string(),
                summary: Some(
                    "Resource could not be serialized for mustSupport validation".to_string(),
                ),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::ValidationException),
                diagnostics: format!(
                    "Failed to serialize resource while validating mustSupport: {}",
                    err
                ),
                expression: None,
                fhir_path: "".to_string(),
                instance_path: None,
            }];
        }
    };

    validate_must_support_from_json(&root, resource_type, rules, config)
}

fn validate_must_support_from_json(
    root: &Value,
    resource_type: &str,
    rules: &[ExtractedElementRule],
    config: &ValidationConfig,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    for rule in rules {
        if rule.must_support != Some(true) {
            continue;
        }

        let Some(relative_path) = relative_profile_path(resource_type, &rule.path) else {
            continue;
        };

        if skip_when_optional_parent_absent(root, relative_path) {
            continue;
        }

        let count = count_relative_path(root, relative_path);
        if count > 0 {
            continue;
        }

        let display_path = profile_element_display_path(rule);
        issues.push(ValidationIssue {
            severity: config.must_support_missing_severity,
            code: issue_code::STRUCTURE.to_string(),
            summary: Some(
                "Element is marked mustSupport in the profile but has no value in this instance"
                    .to_string(),
            ),
            expression_kind: None,
            source_invariant_key: None,
            detail_code: Some(ValidationIssueDetailCode::BusinessRuleViolation),
            diagnostics: format!(
                "Element '{}' is mustSupport but is not populated in the instance.",
                display_path
            ),
            expression: None,
            fhir_path: display_path.clone(),
            instance_path: Some(display_path),
        });
    }

    issues
}

fn validate_min_cardinality_from_json(
    root: &Value,
    resource_type: &str,
    rules: &[ExtractedElementRule],
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    for rule in rules {
        let Some(min) = rule.min else {
            continue;
        };

        if min == 0 {
            continue;
        }

        let Some(relative_path) = relative_profile_path(resource_type, &rule.path) else {
            continue;
        };

        if skip_when_optional_parent_absent(root, relative_path) {
            continue;
        }

        let count = count_relative_path(root, relative_path);
        if count >= min as usize {
            continue;
        }

        let display_path = profile_element_display_path(rule);
        issues.push(ValidationIssue {
            severity: Severity::Error,
            code: "required".to_string(),
            summary: Some("Required element is missing or below minimum cardinality".to_string()),
            expression_kind: None,
            source_invariant_key: None,
            detail_code: Some(ValidationIssueDetailCode::RequiredElementMissing),
            diagnostics: format!(
                "Required element '{}' is missing or does not meet minimum cardinality {}.",
                display_path, min
            ),
            expression: None,
            fhir_path: display_path.clone(),
            instance_path: Some(display_path),
        });
    }

    issues
}

fn validate_max_cardinality_from_json(
    root: &Value,
    resource_type: &str,
    rules: &[ExtractedElementRule],
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    for rule in rules {
        let Some(max) = rule.max.as_deref() else {
            continue;
        };

        if max == "*" {
            continue;
        }

        let Ok(max_value) = max.parse::<usize>() else {
            continue;
        };

        let Some(relative_path) = relative_profile_path(resource_type, &rule.path) else {
            continue;
        };

        if skip_when_optional_parent_absent(root, relative_path) {
            continue;
        }

        let count = count_relative_path(root, relative_path);
        if count <= max_value {
            continue;
        }

        let display_path = profile_element_display_path(rule);
        issues.push(ValidationIssue {
            severity: Severity::Error,
            code: issue_code::STRUCTURE.to_string(),
            summary: Some("Element exceeds maximum cardinality for this profile".to_string()),
            expression_kind: None,
            source_invariant_key: None,
            detail_code: Some(ValidationIssueDetailCode::MaximumCardinalityExceeded),
            diagnostics: format!(
                "Element '{}' exceeds maximum cardinality {}.",
                display_path, max
            ),
            expression: None,
            fhir_path: display_path.clone(),
            instance_path: Some(display_path),
        });
    }

    issues
}

pub fn relative_profile_path<'a>(resource_type: &str, absolute_path: &'a str) -> Option<&'a str> {
    if absolute_path == resource_type {
        return Some("");
    }

    let prefix = format!("{}.", resource_type);
    absolute_path.strip_prefix(&prefix)
}

/// FHIR element path for user-facing messages — includes slice discriminator when present
/// (`Patient.extension:birthPlace`), matching [`crate::profile::slicing`].
fn profile_element_display_path(rule: &ExtractedElementRule) -> String {
    match rule.slice_name.as_deref() {
        Some(slice) => format!("{}:{}", rule.path, slice),
        None => rule.path.clone(),
    }
}

/// `true` when `structure_path` is only the resource root (e.g. `Patient`).
///
/// Constraints on the root are evaluated with the resource as FHIRPath context (same as
/// [`crate::Validator::apply_invariants`]). Nested paths use declared-path focus via
/// [`FhirPathEvaluator::eval_invariant`](crate::FhirPathEvaluator::eval_invariant).
pub fn is_root_profile_element_path(resource_type: &str, structure_path: &str) -> bool {
    relative_profile_path(resource_type, structure_path)
        .map(|rel| rel.is_empty())
        .unwrap_or(false)
}

fn count_relative_path(root: &Value, relative_path: &str) -> usize {
    if relative_path.is_empty() {
        return terminal_count(root);
    }

    let segments: Vec<&str> = relative_path.split('.').collect();
    count_path_segments(root, &segments)
}

fn count_path_segments(current: &Value, segments: &[&str]) -> usize {
    if segments.is_empty() {
        return terminal_count(current);
    }

    match current {
        Value::Object(map) => map
            .get(segments[0])
            .map(|next| count_path_segments(next, &segments[1..]))
            .unwrap_or(0),
        Value::Array(items) => items
            .iter()
            .map(|item| count_path_segments(item, segments))
            .sum(),
        _ => 0,
    }
}

fn terminal_count(value: &Value) -> usize {
    match value {
        Value::Null => 0,
        Value::Array(items) => items.len(),
        _ => 1,
    }
}

/// For nested `relative_path` (`communication.language`), ignore the rule when the parent
/// (`communication`) has no instances: child minimums apply only inside an existing slice.
fn skip_when_optional_parent_absent(root: &Value, relative_path: &str) -> bool {
    let Some((parent_rel, _)) = relative_path.rsplit_once('.') else {
        return false;
    };
    if parent_rel.is_empty() {
        return false;
    }
    count_relative_path(root, parent_rel) == 0
}

#[cfg(test)]
mod tests {
    use super::{validate_max_cardinality, validate_min_cardinality, validate_must_support};
    use crate::ValidationConfig;
    use crate::ValidationIssueDetailCode;
    use crate::profile::types::ExtractedElementRule;
    use fhir_validation_types::BindingDef;
    use fhir_validation_types::Severity;
    use serde_json::json;

    fn rule(path: &str, min: Option<u32>, max: Option<&str>) -> ExtractedElementRule {
        ExtractedElementRule {
            id: path.to_string(),
            path: path.to_string(),
            min,
            max: max.map(str::to_owned),
            binding: None::<BindingDef>,
            constraints: Vec::new(),
            value_constraint: None,
            type_constraints: vec![],
            slicing: None,
            slice_name: None,
            ..Default::default()
        }
    }

    fn must_support_rule(path: &str, ms: bool) -> ExtractedElementRule {
        ExtractedElementRule {
            id: path.to_string(),
            path: path.to_string(),
            must_support: Some(ms),
            ..rule(path, None, None)
        }
    }

    fn must_support_rule_slice(path: &str, slice: &str, ms: bool) -> ExtractedElementRule {
        ExtractedElementRule {
            id: format!("{path}:{slice}"),
            path: path.to_string(),
            slice_name: Some(slice.to_string()),
            must_support: Some(ms),
            ..rule(path, None, None)
        }
    }

    #[test]
    fn must_support_slice_name_in_issue_paths() {
        let patient = json!({ "resourceType": "Patient" });
        let rules = vec![must_support_rule_slice(
            "Patient.extension",
            "birthPlace",
            true,
        )];
        let issues =
            validate_must_support(&patient, "Patient", &rules, &ValidationConfig::default());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].fhir_path, "Patient.extension:birthPlace");
        assert_eq!(
            issues[0].instance_path.as_deref(),
            Some("Patient.extension:birthPlace")
        );
        assert!(
            issues[0]
                .diagnostics
                .contains("Patient.extension:birthPlace"),
            "diagnostics should name the slice: {:?}",
            issues[0].diagnostics
        );
    }

    #[test]
    fn must_support_missing_emits_warning_by_default() {
        let patient = json!({ "resourceType": "Patient" });
        let rules = vec![must_support_rule("Patient.active", true)];
        let issues =
            validate_must_support(&patient, "Patient", &rules, &ValidationConfig::default());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Warning);
        assert_eq!(issues[0].fhir_path, "Patient.active");
    }

    #[test]
    fn must_support_present_produces_no_issue() {
        let patient = json!({
            "resourceType": "Patient",
            "active": true
        });
        let rules = vec![must_support_rule("Patient.active", true)];
        let issues =
            validate_must_support(&patient, "Patient", &rules, &ValidationConfig::default());
        assert!(issues.is_empty());
    }

    #[test]
    fn must_support_disabled_emits_nothing() {
        let patient = json!({ "resourceType": "Patient" });
        let rules = vec![must_support_rule("Patient.active", true)];
        let mut cfg = ValidationConfig::default();
        cfg.validate_must_support = false;
        let issues = validate_must_support(&patient, "Patient", &rules, &cfg);
        assert!(issues.is_empty());
    }

    #[test]
    fn must_support_respects_configured_severity() {
        let patient = json!({ "resourceType": "Patient" });
        let rules = vec![must_support_rule("Patient.active", true)];
        let mut cfg = ValidationConfig::default();
        cfg.must_support_missing_severity = Severity::Error;
        let issues = validate_must_support(&patient, "Patient", &rules, &cfg);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Error);
    }

    #[test]
    fn nested_min_skipped_when_parent_slice_absent() {
        let patient = json!({
            "resourceType": "Patient"
        });
        let rules = vec![rule("Patient.contact.name", Some(1), None)];
        let issues = validate_min_cardinality(&patient, "Patient", &rules);
        assert!(
            issues.is_empty(),
            "min on contact.name should not apply when contact is absent: {issues:?}"
        );
    }

    #[test]
    fn missing_required_element_produces_issue() {
        let patient = json!({
            "resourceType": "Patient"
        });

        let rules = vec![rule("Patient.identifier", Some(1), None)];
        let issues = validate_min_cardinality(&patient, "Patient", &rules);

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].fhir_path, "Patient.identifier");
        assert_eq!(issues[0].code, "required");
        assert!(issues[0]
            .diagnostics
            .contains("Required element 'Patient.identifier' is missing or does not meet minimum cardinality 1."));
        assert_eq!(
            issues[0].summary.as_deref(),
            Some("Required element is missing or below minimum cardinality")
        );
        assert_eq!(
            issues[0].detail_code,
            Some(ValidationIssueDetailCode::RequiredElementMissing)
        );
    }

    #[test]
    fn present_required_element_produces_no_issue() {
        let patient = json!({
            "resourceType": "Patient",
            "identifier": [
                { "value": "123" }
            ]
        });

        let rules = vec![rule("Patient.identifier", Some(1), None)];
        let issues = validate_min_cardinality(&patient, "Patient", &rules);

        assert!(issues.is_empty());
    }

    #[test]
    fn zero_minimum_allows_missing_element() {
        let patient = json!({
            "resourceType": "Patient"
        });

        let rules = vec![rule("Patient.maritalStatus", Some(0), None)];
        let issues = validate_min_cardinality(&patient, "Patient", &rules);

        assert!(issues.is_empty());
    }

    #[test]
    fn nested_element_minimum_cardinality() {
        let patient = json!({
            "resourceType": "Patient",
            "contact": [
                {
                    "name": {
                        "family": "Smith"
                    }
                }
            ]
        });

        let rules = vec![rule("Patient.contact.name", Some(1), None)];
        let issues = validate_min_cardinality(&patient, "Patient", &rules);
        assert!(issues.is_empty());
    }

    #[test]
    fn missing_nested_element_produces_issue() {
        let patient = json!({
            "resourceType": "Patient",
            "contact": [{}]
        });

        let rules = vec![rule("Patient.contact.name", Some(1), None)];
        let issues = validate_min_cardinality(&patient, "Patient", &rules);

        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn exceeding_maximum_cardinality_produces_issue() {
        let patient = json!({
            "resourceType": "Patient",
            "identifier": [
                { "value": "1" },
                { "value": "2" }
            ]
        });

        let rules = vec![rule("Patient.identifier", None, Some("1"))];
        let issues = validate_max_cardinality(&patient, "Patient", &rules);

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].fhir_path, "Patient.identifier");
        assert_eq!(issues[0].code, "structure");
        assert!(
            issues[0]
                .diagnostics
                .contains("Element 'Patient.identifier' exceeds maximum cardinality 1.")
        );
        assert_eq!(
            issues[0].summary.as_deref(),
            Some("Element exceeds maximum cardinality for this profile")
        );
        assert_eq!(
            issues[0].detail_code,
            Some(ValidationIssueDetailCode::MaximumCardinalityExceeded)
        );
    }

    #[test]
    fn satisfying_maximum_cardinality_produces_no_issue() {
        let patient = json!({
            "resourceType": "Patient",
            "identifier": [
                { "value": "1" }
            ]
        });

        let rules = vec![rule("Patient.identifier", None, Some("1"))];
        let issues = validate_max_cardinality(&patient, "Patient", &rules);

        assert!(issues.is_empty());
    }

    #[test]
    fn unbounded_maximum_is_ignored() {
        let patient = json!({
            "resourceType": "Patient",
            "identifier": [
                { "value": "1" },
                { "value": "2" },
                { "value": "3" }
            ]
        });

        let rules = vec![rule("Patient.identifier", None, Some("*"))];
        let issues = validate_max_cardinality(&patient, "Patient", &rules);

        assert!(issues.is_empty());
    }
}
