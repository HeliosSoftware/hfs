//! `ElementDefinition.maxLength` and `minValue` / `maxValue` checks against instance JSON.
//!
//! Temporal comparisons use the generated FHIR library’s precision types
//! ([`PrecisionDate`],
//! [`PrecisionDateTime`],
//! [`PrecisionInstant`],
//! [`PrecisionTime`]) and their [`PrecisionDate::compare`] /
//! [`PrecisionDateTime::compare`] logic so ordering follows
//! partial date/datetime/time rules instead of naive string ordering.
//!
//! Numeric bounds align with the same primitives as generated resources:
//! - **decimal** — [`PreciseDecimal`] / [`DecimalElement`](helios_fhir::DecimalElement)
//!   (scientific notation, nested `value`, same ordering as the FHIR model).
//! - **integer**, **positiveInt**, **unsignedInt** (stored as `Element<i32, _>` in
//!   generated R5), **integer64** — `Element<i64, _>`: JSON may be a bare number
//!   or `{"value": …}`; we deserialize accordingly.
//!
//! Rules are evaluated for every JSON value matching the element’s relative path(s), including each
//! array element, with [`crate::ValidationIssue::instance_path`] pointing at the offending leaf.

use crate::issue_code;
use crate::profile::cardinality::relative_profile_path;
use crate::profile::helpers::get_values_with_paths_at_relative_path;
use crate::profile::types::ExtractedElementRule;
use crate::validation_issue_detail::ValidationIssueDetailCode;
use crate::{Severity, ValidationIssue};
use helios_fhir::{
    PreciseDecimal, PrecisionDate, PrecisionDateTime, PrecisionInstant, PrecisionTime,
};
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::cmp::Ordering;
use std::str::FromStr;

/// Validate `maxLength` and numeric / date / time min/max bounds declared on
/// [`ExtractedElementRule`] rows against serialized instance JSON.
///
/// Skips rules with no bound fields; otherwise walks every value at the rule path (same path
/// traversal strategy as other profile validators: repeated array indices, choice `[x]` keys).
pub fn validate_element_bounds<T: Serialize>(
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
                    "Resource could not be serialized for element bound validation".to_string(),
                ),
                expression_kind: None,
                source_invariant_key: None,
                detail_code: Some(ValidationIssueDetailCode::ValidationException),
                diagnostics: format!(
                    "Failed to serialize resource while validating profile element bounds: {}",
                    err
                ),
                expression: None,
                fhir_path: "".to_string(),
                instance_path: None,
            }];
        }
    };

    validate_element_bounds_from_json(&root, resource_type, rules)
}

fn validate_element_bounds_from_json(
    root: &Value,
    resource_type: &str,
    rules: &[ExtractedElementRule],
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    for rule in rules {
        let has_bounds =
            rule.max_length.is_some() || rule.min_value.is_some() || rule.max_value.is_some();
        if !has_bounds {
            continue;
        }

        let Some(rel) = relative_profile_path(resource_type, &rule.path) else {
            continue;
        };

        let values = get_values_with_paths_at_relative_path(root, resource_type, rel);

        for (value, instance_path) in values {
            if let Some(max_len) = rule.max_length {
                issues.extend(check_max_length(&rule.path, &instance_path, value, max_len));
            }
            if let Some(ref min_v) = rule.min_value {
                issues.extend(check_min_value(&rule.path, &instance_path, value, min_v));
            }
            if let Some(ref max_v) = rule.max_value {
                issues.extend(check_max_value(&rule.path, &instance_path, value, max_v));
            }
        }
    }

    issues
}

fn check_max_length(
    path: &str,
    instance_path: &str,
    value: &Value,
    max_len: u32,
) -> Vec<ValidationIssue> {
    let s = match primitive_string_value(value) {
        Some(s) => s,
        None => return Vec::new(),
    };
    if s.chars().count() as u32 <= max_len {
        return Vec::new();
    }
    vec![ValidationIssue {
        severity: Severity::Error,
        code: issue_code::STRUCTURE.to_string(),
        summary: Some("Element value exceeds maxLength".to_string()),
        expression_kind: None,
        source_invariant_key: None,
        detail_code: Some(ValidationIssueDetailCode::ConstraintViolation),
        diagnostics: format!(
            "String '{}' at '{}' has length {} which exceeds maxLength {}",
            path,
            instance_path,
            s.chars().count(),
            max_len
        ),
        expression: None,
        fhir_path: path.to_string(),
        instance_path: Some(instance_path.to_string()),
    }]
}

fn check_min_value(
    path: &str,
    instance_path: &str,
    value: &Value,
    bound: &Value,
) -> Vec<ValidationIssue> {
    let Some((key, bound_val)) = single_bound_key_value(bound, "minValue") else {
        return Vec::new();
    };
    match compare_by_bound_key(key, value, bound_val) {
        CompareOutcome::Less => {
            violation_issue(path, instance_path, key, bound, "below", "minValue")
        }
        CompareOutcome::Indeterminate => indeterminate_issue(
            path,
            instance_path,
            key,
            "minValue",
            "cannot order instance and bound (e.g. incompatible precision, timezone, or units)",
        ),
        CompareOutcome::Unparseable | CompareOutcome::Unsupported => Vec::new(),
        CompareOutcome::Equal | CompareOutcome::Greater => Vec::new(),
    }
}

fn check_max_value(
    path: &str,
    instance_path: &str,
    value: &Value,
    bound: &Value,
) -> Vec<ValidationIssue> {
    let Some((key, bound_val)) = single_bound_key_value(bound, "maxValue") else {
        return Vec::new();
    };
    match compare_by_bound_key(key, value, bound_val) {
        CompareOutcome::Greater => {
            violation_issue(path, instance_path, key, bound, "above", "maxValue")
        }
        CompareOutcome::Indeterminate => indeterminate_issue(
            path,
            instance_path,
            key,
            "maxValue",
            "cannot order instance and bound (e.g. incompatible precision, timezone, or units)",
        ),
        CompareOutcome::Unparseable | CompareOutcome::Unsupported => Vec::new(),
        CompareOutcome::Equal | CompareOutcome::Less => Vec::new(),
    }
}

fn violation_issue(
    path: &str,
    instance_path: &str,
    key: &str,
    bound: &Value,
    direction: &str,
    label: &str,
) -> Vec<ValidationIssue> {
    vec![ValidationIssue {
        severity: Severity::Error,
        code: issue_code::STRUCTURE.to_string(),
        summary: Some(format!("Element value is {direction} {label}")),
        expression_kind: None,
        source_invariant_key: None,
        detail_code: Some(ValidationIssueDetailCode::ConstraintViolation),
        diagnostics: format!(
            "Value at '{}' ({}) is {direction} {} bound {:?}",
            instance_path, path, key, bound
        ),
        expression: None,
        fhir_path: path.to_string(),
        instance_path: Some(instance_path.to_string()),
    }]
}

fn indeterminate_issue(
    path: &str,
    instance_path: &str,
    key: &str,
    label: &str,
    reason: &str,
) -> Vec<ValidationIssue> {
    vec![ValidationIssue {
        severity: Severity::Warning,
        code: issue_code::STRUCTURE.to_string(),
        summary: Some(format!("{label} check was inconclusive ({key})")),
        expression_kind: None,
        source_invariant_key: None,
        detail_code: Some(ValidationIssueDetailCode::BusinessRuleViolation),
        diagnostics: format!("At '{}' ({}): {} — {}", instance_path, path, reason, label),
        expression: None,
        fhir_path: path.to_string(),
        instance_path: Some(instance_path.to_string()),
    }]
}

fn single_bound_key_value<'a>(bound: &'a Value, prefix: &str) -> Option<(&'a str, &'a Value)> {
    let obj = bound.as_object()?;
    let (k, v) = obj.iter().find(|(k, _)| k.starts_with(prefix))?;
    Some((k.as_str(), v))
}

fn primitive_string_value(value: &Value) -> Option<&str> {
    match value {
        Value::String(s) => Some(s.as_str()),
        Value::Object(map) => map
            .get(issue_code::FHIR_JSON_VALUE)
            .and_then(|v| v.as_str()),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompareOutcome {
    Less,
    Equal,
    Greater,
    Indeterminate,
    Unparseable,
    Unsupported,
}

fn outcome_from_ordering(o: Ordering) -> CompareOutcome {
    match o {
        Ordering::Less => CompareOutcome::Less,
        Ordering::Equal => CompareOutcome::Equal,
        Ordering::Greater => CompareOutcome::Greater,
    }
}

/// Compare instance value to the profile bound using the `minValue*` / `maxValue*` key
/// from the profile (e.g. `minValueDate` → key `minValueDate`).
fn compare_by_bound_key(bound_key: &str, actual: &Value, bound: &Value) -> CompareOutcome {
    match bound_key {
        "minValueDate" | "maxValueDate" => compare_dates(actual, bound),
        "minValueDateTime" | "maxValueDateTime" => compare_datetimes(actual, bound),
        "minValueInstant" | "maxValueInstant" => compare_instants(actual, bound),
        "minValueTime" | "maxValueTime" => compare_times(actual, bound),
        "minValueDecimal" | "maxValueDecimal" => compare_decimals(actual, bound),
        "minValueInteger" | "maxValueInteger" | "minValuePositiveInt" | "maxValuePositiveInt" => {
            compare_integer_pair(actual, bound)
        }
        "minValueInteger64" | "maxValueInteger64" => compare_integer64_pair(actual, bound),
        "minValueUnsignedInt" | "maxValueUnsignedInt" => compare_unsigned_int_pair(actual, bound),
        "minValueQuantity" | "maxValueQuantity" => compare_quantities(actual, bound),
        _ => CompareOutcome::Unsupported,
    }
}

fn extract_string_for_primitive(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(m) => match m.get(issue_code::FHIR_JSON_VALUE)? {
            Value::String(s) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn compare_dates(actual: &Value, bound: &Value) -> CompareOutcome {
    let (a_str, b_str) = match (
        extract_string_for_primitive(actual),
        extract_string_for_primitive(bound),
    ) {
        (Some(a), Some(b)) => (a, b),
        _ => return CompareOutcome::Unparseable,
    };
    let (a, b) = match (PrecisionDate::parse(&a_str), PrecisionDate::parse(&b_str)) {
        (Some(a), Some(b)) => (a, b),
        _ => return CompareOutcome::Unparseable,
    };
    match a.compare(&b) {
        Some(o) => outcome_from_ordering(o),
        None => CompareOutcome::Indeterminate,
    }
}

fn compare_datetimes(actual: &Value, bound: &Value) -> CompareOutcome {
    let (a_str, b_str) = match (
        extract_string_for_primitive(actual),
        extract_string_for_primitive(bound),
    ) {
        (Some(a), Some(b)) => (a, b),
        _ => return CompareOutcome::Unparseable,
    };
    let (a, b) = match (
        PrecisionDateTime::parse(&a_str),
        PrecisionDateTime::parse(&b_str),
    ) {
        (Some(a), Some(b)) => (a, b),
        _ => return CompareOutcome::Unparseable,
    };
    match a.compare(&b) {
        Some(o) => outcome_from_ordering(o),
        None => CompareOutcome::Indeterminate,
    }
}

fn compare_instants(actual: &Value, bound: &Value) -> CompareOutcome {
    let (a_str, b_str) = match (
        extract_string_for_primitive(actual),
        extract_string_for_primitive(bound),
    ) {
        (Some(a), Some(b)) => (a, b),
        _ => return CompareOutcome::Unparseable,
    };
    let (a, b) = match (
        PrecisionInstant::parse(&a_str),
        PrecisionInstant::parse(&b_str),
    ) {
        (Some(a), Some(b)) => (a, b),
        _ => return CompareOutcome::Unparseable,
    };
    match a.as_datetime().compare(b.as_datetime()) {
        Some(o) => outcome_from_ordering(o),
        None => CompareOutcome::Indeterminate,
    }
}

fn compare_times(actual: &Value, bound: &Value) -> CompareOutcome {
    let (a_str, b_str) = match (
        extract_string_for_primitive(actual),
        extract_string_for_primitive(bound),
    ) {
        (Some(a), Some(b)) => (a, b),
        _ => return CompareOutcome::Unparseable,
    };
    let (a, b) = match (PrecisionTime::parse(&a_str), PrecisionTime::parse(&b_str)) {
        (Some(a), Some(b)) => (a, b),
        _ => return CompareOutcome::Unparseable,
    };
    match a.compare(&b) {
        Some(o) => outcome_from_ordering(o),
        None => CompareOutcome::Indeterminate,
    }
}

/// Parse a FHIR primitive JSON value the same way as [`Element`] / [`DecimalElement`]: bare
/// number or string, or an object with a `value` field (and optional `id` / `extension`).
fn precise_decimal_from_value(v: &Value) -> Option<PreciseDecimal> {
    serde_json::from_value(v.clone()).ok()
}

fn compare_decimals(actual: &Value, bound: &Value) -> CompareOutcome {
    let a = match precise_decimal_from_value(actual) {
        Some(d) => d,
        None => return CompareOutcome::Unparseable,
    };
    let b = match precise_decimal_from_value(bound) {
        Some(d) => d,
        None => return CompareOutcome::Unparseable,
    };
    outcome_from_ordering(a.cmp(&b))
}

#[derive(Deserialize)]
struct ValueOnly<T> {
    value: Option<T>,
}

/// Integer / positiveInt / unsignedInt / integer64: match generated [`Element`] JSON shapes.
fn parse_fhir_integral<T>(v: &Value) -> Option<T>
where
    T: serde::de::DeserializeOwned + Ord + FromStr,
{
    if let Ok(x) = serde_json::from_value::<T>(v.clone()) {
        return Some(x);
    }
    if let Ok(ValueOnly { value: Some(x) }) = serde_json::from_value::<ValueOnly<T>>(v.clone()) {
        return Some(x);
    }
    if let Value::String(s) = v {
        return s.parse().ok();
    }
    None
}

fn compare_integer_pair(actual: &Value, bound: &Value) -> CompareOutcome {
    let a = match parse_fhir_integral::<i32>(actual) {
        Some(d) => d,
        None => return CompareOutcome::Unparseable,
    };
    let b = match parse_fhir_integral::<i32>(bound) {
        Some(d) => d,
        None => return CompareOutcome::Unparseable,
    };
    outcome_from_ordering(a.cmp(&b))
}

fn compare_integer64_pair(actual: &Value, bound: &Value) -> CompareOutcome {
    let a = match parse_fhir_integral::<i64>(actual) {
        Some(d) => d,
        None => return CompareOutcome::Unparseable,
    };
    let b = match parse_fhir_integral::<i64>(bound) {
        Some(d) => d,
        None => return CompareOutcome::Unparseable,
    };
    outcome_from_ordering(a.cmp(&b))
}

fn compare_unsigned_int_pair(actual: &Value, bound: &Value) -> CompareOutcome {
    // Generated R5 `unsignedInt` is `Element<i32, Extension>`; compare as i32 like other 32-bit ints.
    let a = match parse_fhir_integral::<i32>(actual) {
        Some(d) => d,
        None => return CompareOutcome::Unparseable,
    };
    let b = match parse_fhir_integral::<i32>(bound) {
        Some(d) => d,
        None => return CompareOutcome::Unparseable,
    };
    outcome_from_ordering(a.cmp(&b))
}

fn quantity_value_precise(q: &Value) -> Option<PreciseDecimal> {
    let obj = q.as_object()?;
    precise_decimal_from_value(obj.get(issue_code::FHIR_JSON_VALUE)?)
}

fn quantity_unit_key(q: &Value) -> Option<(Option<String>, Option<String>)> {
    let obj = q.as_object()?;
    let code = obj
        .get("code")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let system = obj
        .get("system")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some((code, system))
}

fn compare_quantities(actual: &Value, bound: &Value) -> CompareOutcome {
    let av = match quantity_value_precise(actual) {
        Some(d) => d,
        None => return CompareOutcome::Unparseable,
    };
    let bv = match quantity_value_precise(bound) {
        Some(d) => d,
        None => return CompareOutcome::Unparseable,
    };
    if let (Some(au), Some(bu)) = (quantity_unit_key(actual), quantity_unit_key(bound)) {
        if au != bu {
            return CompareOutcome::Indeterminate;
        }
    }
    outcome_from_ordering(av.cmp(&bv))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn date_orders_using_precision_not_lexicographic_edge() {
        // Same calendar order; precision types still agree here.
        let a = json!("2010-06-15");
        let b = json!("1990-01-01");
        assert_eq!(compare_dates(&a, &b), CompareOutcome::Greater);
    }

    #[test]
    fn date_incompatible_precision_is_indeterminate() {
        let year_only = json!("1990");
        let year_month = json!("1990-06");
        assert_eq!(
            compare_dates(&year_only, &year_month),
            CompareOutcome::Indeterminate
        );
    }

    #[test]
    fn datetime_compare_uses_precision_datetime() {
        let a = json!("2010-01-01T12:00:00Z");
        let b = json!("2010-01-01T00:00:00Z");
        assert_eq!(compare_datetimes(&a, &b), CompareOutcome::Greater);
    }

    #[test]
    fn quantity_different_units_indeterminate() {
        let actual = json!({"value": 10, "code": "mg", "system": "http://unitsofmeasure.org"});
        let bound = json!({"value": 10, "code": "g", "system": "http://unitsofmeasure.org"});
        assert_eq!(
            compare_quantities(&actual, &bound),
            CompareOutcome::Indeterminate
        );
    }

    #[test]
    fn quantity_same_unit_compares_value() {
        let actual = json!({"value": 5, "code": "mg", "system": "http://unitsofmeasure.org"});
        let bound = json!({"value": 10, "code": "mg", "system": "http://unitsofmeasure.org"});
        assert_eq!(compare_quantities(&actual, &bound), CompareOutcome::Less);
    }

    #[test]
    fn decimal_uses_precise_decimal_parsing_scientific_notation() {
        let a = json!("1.23e2");
        let b = json!("100");
        assert_eq!(compare_decimals(&a, &b), CompareOutcome::Greater);
    }

    #[test]
    fn integer_accepts_element_shaped_json() {
        let a = json!({"value": 5});
        let b = json!(10);
        assert_eq!(compare_integer_pair(&a, &b), CompareOutcome::Less);
    }
}
