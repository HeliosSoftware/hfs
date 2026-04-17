//! Filter matching logic.
//!
//! Evaluates subscription filter criteria against resource JSON content.
//! Uses simple JSON path traversal for Phase 1 (not FHIRPath).

use crate::manager::SubscriptionFilter;

/// Checks whether a resource matches all of a subscription's filters.
///
/// Returns `true` if:
/// - The filter list is empty (match all).
/// - Every filter matches the resource content.
pub fn matches_filters(
    resource: &serde_json::Value,
    resource_type: &str,
    filters: &[SubscriptionFilter],
) -> bool {
    filters
        .iter()
        .all(|filter| matches_single_filter(resource, resource_type, filter))
}

/// Evaluates a single filter against a resource.
fn matches_single_filter(
    resource: &serde_json::Value,
    resource_type: &str,
    filter: &SubscriptionFilter,
) -> bool {
    // If the filter specifies a resource type, it must match.
    if let Some(ref filter_rt) = filter.resource_type {
        if filter_rt != resource_type {
            return false;
        }
    }

    // Look up the field in the resource.
    let field_value = resolve_field(resource, &filter.filter_parameter);

    match field_value {
        None => {
            // Field not present — only matches "ne" comparator.
            filter.comparator == "ne"
        }
        Some(value) => match filter.comparator.as_str() {
            "eq" => value_matches_eq(&value, &filter.value),
            "ne" => !value_matches_eq(&value, &filter.value),
            "in" => value_matches_in(&value, &filter.value),
            "not-in" => !value_matches_in(&value, &filter.value),
            // For unsupported comparators, default to not matching.
            _ => false,
        },
    }
}

/// Resolves a field path in a resource JSON.
///
/// Supports simple dot-free FHIR search parameter names by mapping common
/// search parameters to their JSON paths. For example, "patient" maps to
/// "subject.reference" for some resource types.
fn resolve_field<'a>(resource: &'a serde_json::Value, param_name: &str) -> Option<FieldValue<'a>> {
    // Try known search parameter mappings first (these extract structured
    // types like CodeableConcept, Reference, and Identifier into token strings).
    match param_name {
        "patient" => {
            if let Some(v) = try_resolve_reference(resource, &["subject", "patient"]) {
                return Some(v);
            }
        }
        "code" => {
            if let Some(v) = resolve_codeable_concept(resource, "code") {
                return Some(v);
            }
        }
        "category" => {
            if let Some(v) = resolve_codeable_concept(resource, "category") {
                return Some(v);
            }
        }
        "identifier" => {
            if let Some(v) = resolve_identifier(resource, "identifier") {
                return Some(v);
            }
        }
        _ => {}
    }

    // Fall back to direct field lookup for simple fields (e.g., "status").
    if let Some(val) = resource.get(param_name) {
        return Some(FieldValue::from_json(val));
    }

    None
}

/// Try to resolve a reference field from multiple possible paths.
fn try_resolve_reference<'a>(
    resource: &'a serde_json::Value,
    field_names: &[&str],
) -> Option<FieldValue<'a>> {
    for name in field_names {
        if let Some(obj) = resource.get(name) {
            if let Some(reference) = obj.get("reference").and_then(|v| v.as_str()) {
                return Some(FieldValue::String(reference));
            }
        }
    }
    None
}

/// Resolve a CodeableConcept field to a set of system|code tokens.
fn resolve_codeable_concept<'a>(
    resource: &'a serde_json::Value,
    field_name: &str,
) -> Option<FieldValue<'a>> {
    let cc = resource.get(field_name)?;

    // Could be a single CodeableConcept or an array of them.
    let concepts = if cc.is_array() {
        cc.as_array().unwrap().iter().collect::<Vec<_>>()
    } else {
        vec![cc]
    };

    let mut tokens = Vec::new();
    for concept in concepts {
        if let Some(codings) = concept.get("coding").and_then(|v| v.as_array()) {
            for coding in codings {
                let system = coding.get("system").and_then(|v| v.as_str()).unwrap_or("");
                let code = coding.get("code").and_then(|v| v.as_str()).unwrap_or("");
                tokens.push(format!("{system}|{code}"));
                // Also match just the code.
                if !code.is_empty() {
                    tokens.push(code.to_string());
                }
            }
        }
    }

    if tokens.is_empty() {
        None
    } else {
        Some(FieldValue::Tokens(tokens))
    }
}

/// Resolve an Identifier field.
fn resolve_identifier<'a>(
    resource: &'a serde_json::Value,
    field_name: &str,
) -> Option<FieldValue<'a>> {
    let identifiers = resource.get(field_name)?.as_array()?;
    let mut tokens = Vec::new();
    for id in identifiers {
        let system = id.get("system").and_then(|v| v.as_str()).unwrap_or("");
        let value = id.get("value").and_then(|v| v.as_str()).unwrap_or("");
        tokens.push(format!("{system}|{value}"));
        if !value.is_empty() {
            tokens.push(value.to_string());
        }
    }

    if tokens.is_empty() {
        None
    } else {
        Some(FieldValue::Tokens(tokens))
    }
}

/// A resolved field value from a resource.
#[derive(Debug)]
enum FieldValue<'a> {
    /// A simple string value.
    String(&'a str),
    /// A set of token values (from CodeableConcept, Identifier, etc.).
    Tokens(Vec<String>),
    /// A raw JSON value.
    Json(&'a serde_json::Value),
}

impl<'a> FieldValue<'a> {
    fn from_json(val: &'a serde_json::Value) -> Self {
        if let Some(s) = val.as_str() {
            FieldValue::String(s)
        } else {
            FieldValue::Json(val)
        }
    }
}

/// Check equality between a field value and a filter value string.
fn value_matches_eq(field: &FieldValue<'_>, filter_value: &str) -> bool {
    match field {
        FieldValue::String(s) => *s == filter_value,
        FieldValue::Tokens(tokens) => tokens.iter().any(|t| t == filter_value),
        FieldValue::Json(val) => {
            // Try numeric, boolean, or string comparison.
            if let Some(s) = val.as_str() {
                s == filter_value
            } else if let Some(b) = val.as_bool() {
                filter_value == if b { "true" } else { "false" }
            } else if let Some(n) = val.as_f64() {
                filter_value
                    .parse::<f64>()
                    .is_ok_and(|fv| (n - fv).abs() < f64::EPSILON)
            } else {
                false
            }
        }
    }
}

/// Check if a field value matches any value in a comma-separated "in" list.
fn value_matches_in(field: &FieldValue<'_>, filter_value: &str) -> bool {
    let in_values: Vec<&str> = filter_value.split(',').collect();
    in_values.iter().any(|v| value_matches_eq(field, v.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_empty_filters_match_all() {
        let resource = json!({"resourceType": "Patient", "id": "123"});
        assert!(matches_filters(&resource, "Patient", &[]));
    }

    #[test]
    fn test_direct_field_eq_match() {
        let resource = json!({"resourceType": "Encounter", "status": "in-progress"});
        let filters = vec![SubscriptionFilter {
            resource_type: None,
            filter_parameter: "status".to_string(),
            comparator: "eq".to_string(),
            value: "in-progress".to_string(),
        }];
        assert!(matches_filters(&resource, "Encounter", &filters));
    }

    #[test]
    fn test_direct_field_eq_no_match() {
        let resource = json!({"resourceType": "Encounter", "status": "finished"});
        let filters = vec![SubscriptionFilter {
            resource_type: None,
            filter_parameter: "status".to_string(),
            comparator: "eq".to_string(),
            value: "in-progress".to_string(),
        }];
        assert!(!matches_filters(&resource, "Encounter", &filters));
    }

    #[test]
    fn test_resource_type_filter_mismatch() {
        let resource = json!({"resourceType": "Patient", "status": "active"});
        let filters = vec![SubscriptionFilter {
            resource_type: Some("Encounter".to_string()),
            filter_parameter: "status".to_string(),
            comparator: "eq".to_string(),
            value: "active".to_string(),
        }];
        // Filter resource type is Encounter, but resource is Patient.
        assert!(!matches_filters(&resource, "Patient", &filters));
    }

    #[test]
    fn test_code_filter_with_codeable_concept() {
        let resource = json!({
            "resourceType": "Observation",
            "code": {
                "coding": [{
                    "system": "http://loinc.org",
                    "code": "1234-5"
                }]
            }
        });

        // Match by system|code.
        let filters = vec![SubscriptionFilter {
            resource_type: None,
            filter_parameter: "code".to_string(),
            comparator: "eq".to_string(),
            value: "http://loinc.org|1234-5".to_string(),
        }];
        assert!(matches_filters(&resource, "Observation", &filters));

        // Match by code only.
        let filters = vec![SubscriptionFilter {
            resource_type: None,
            filter_parameter: "code".to_string(),
            comparator: "eq".to_string(),
            value: "1234-5".to_string(),
        }];
        assert!(matches_filters(&resource, "Observation", &filters));
    }

    #[test]
    fn test_code_filter_no_match() {
        let resource = json!({
            "resourceType": "Observation",
            "code": {
                "coding": [{
                    "system": "http://loinc.org",
                    "code": "1234-5"
                }]
            }
        });

        let filters = vec![SubscriptionFilter {
            resource_type: None,
            filter_parameter: "code".to_string(),
            comparator: "eq".to_string(),
            value: "http://loinc.org|9999-9".to_string(),
        }];
        assert!(!matches_filters(&resource, "Observation", &filters));
    }

    #[test]
    fn test_patient_reference_filter() {
        let resource = json!({
            "resourceType": "Encounter",
            "subject": {
                "reference": "Patient/123"
            }
        });

        let filters = vec![SubscriptionFilter {
            resource_type: None,
            filter_parameter: "patient".to_string(),
            comparator: "eq".to_string(),
            value: "Patient/123".to_string(),
        }];
        assert!(matches_filters(&resource, "Encounter", &filters));
    }

    #[test]
    fn test_identifier_filter() {
        let resource = json!({
            "resourceType": "Patient",
            "identifier": [{
                "system": "http://example.org/mrn",
                "value": "MRN-001"
            }]
        });

        let filters = vec![SubscriptionFilter {
            resource_type: None,
            filter_parameter: "identifier".to_string(),
            comparator: "eq".to_string(),
            value: "http://example.org/mrn|MRN-001".to_string(),
        }];
        assert!(matches_filters(&resource, "Patient", &filters));
    }

    #[test]
    fn test_ne_comparator() {
        let resource = json!({"resourceType": "Encounter", "status": "finished"});
        let filters = vec![SubscriptionFilter {
            resource_type: None,
            filter_parameter: "status".to_string(),
            comparator: "ne".to_string(),
            value: "in-progress".to_string(),
        }];
        assert!(matches_filters(&resource, "Encounter", &filters));
    }

    #[test]
    fn test_in_comparator() {
        let resource = json!({"resourceType": "Encounter", "status": "in-progress"});
        let filters = vec![SubscriptionFilter {
            resource_type: None,
            filter_parameter: "status".to_string(),
            comparator: "in".to_string(),
            value: "planned,in-progress,completed".to_string(),
        }];
        assert!(matches_filters(&resource, "Encounter", &filters));
    }

    #[test]
    fn test_multiple_filters_all_must_match() {
        let resource = json!({
            "resourceType": "Observation",
            "status": "final",
            "code": {
                "coding": [{"system": "http://loinc.org", "code": "1234-5"}]
            }
        });

        let filters = vec![
            SubscriptionFilter {
                resource_type: None,
                filter_parameter: "status".to_string(),
                comparator: "eq".to_string(),
                value: "final".to_string(),
            },
            SubscriptionFilter {
                resource_type: None,
                filter_parameter: "code".to_string(),
                comparator: "eq".to_string(),
                value: "1234-5".to_string(),
            },
        ];
        assert!(matches_filters(&resource, "Observation", &filters));

        // Change one filter to not match.
        let filters = vec![
            SubscriptionFilter {
                resource_type: None,
                filter_parameter: "status".to_string(),
                comparator: "eq".to_string(),
                value: "preliminary".to_string(),
            },
            SubscriptionFilter {
                resource_type: None,
                filter_parameter: "code".to_string(),
                comparator: "eq".to_string(),
                value: "1234-5".to_string(),
            },
        ];
        assert!(!matches_filters(&resource, "Observation", &filters));
    }

    #[test]
    fn test_missing_field_ne_matches() {
        let resource = json!({"resourceType": "Encounter"});
        let filters = vec![SubscriptionFilter {
            resource_type: None,
            filter_parameter: "status".to_string(),
            comparator: "ne".to_string(),
            value: "finished".to_string(),
        }];
        // Missing field with "ne" comparator should match.
        assert!(matches_filters(&resource, "Encounter", &filters));
    }
}
