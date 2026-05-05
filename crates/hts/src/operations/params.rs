//! Helpers for parsing and building FHIR Parameters resources.
//!
//! All FHIR operation endpoints accept and return `application/fhir+json`
//! bodies whose `resourceType` is `"Parameters"`. These helpers encapsulate
//! the common patterns so each handler stays focused on business logic.

use serde_json::{Value, json};

use crate::error::HtsError;

// ── Parsing helpers ────────────────────────────────────────────────────────────

/// Validate that `body` is a FHIR Parameters resource and return its
/// `parameter` array (empty vec if the field is absent).
pub fn extract_parameter_array(body: &Value) -> Result<Vec<Value>, HtsError> {
    match body.get("resourceType").and_then(|v| v.as_str()) {
        Some("Parameters") => {}
        _ => {
            return Err(HtsError::InvalidRequest(
                "Request body must be a FHIR Parameters resource \
                 (\"resourceType\": \"Parameters\")"
                    .into(),
            ));
        }
    }

    let arr = body
        .get("parameter")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(arr)
}

/// Return the string value of the first parameter named `name`, or `None`.
pub fn find_str_param(params: &[Value], name: &str) -> Option<String> {
    params
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(name))
        .and_then(extract_any_string_value)
}

/// Collect the string values of **all** parameters named `name` (handles
/// repeated parameters such as `property` in `$lookup`).
pub fn collect_str_params(params: &[Value], name: &str) -> Vec<String> {
    params
        .iter()
        .filter(|p| p.get("name").and_then(|v| v.as_str()) == Some(name))
        .filter_map(extract_any_string_value)
        .collect()
}

/// Extract a string-typed value from a FHIR parameter object, checking the
/// most common `valueXxx` fields.
fn extract_any_string_value(param: &Value) -> Option<String> {
    const STRING_KEYS: &[&str] = &[
        "valueUri",
        "valueCode",
        "valueString",
        "valueUrl",
        "valueOid",
        "valueId",
        "valueUuid",
        "valueMarkdown",
        "valueCanonical",
        "valueDateTime",
        "valueDate",
    ];

    for key in STRING_KEYS {
        if let Some(s) = param.get(key).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }

    // For non-string primitives (boolean, integer, decimal) convert to string.
    for key in &["valueBoolean", "valueInteger", "valueDecimal"] {
        if let Some(v) = param.get(key) {
            return Some(v.to_string());
        }
    }

    None
}

/// Extract a `valueCoding` parameter, returning `(system, code, display)`.
///
/// Looks for the first parameter named `name` that carries a `valueCoding`
/// object and returns the `system`, `code`, and optional `display` from it.
/// Returns `None` if the parameter is absent or incomplete.
pub fn extract_coding(params: &[Value], name: &str) -> Option<(String, String, Option<String>)> {
    let coding = params
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(name))?
        .get("valueCoding")?;
    // FHIR ValueSet/$validate-code allows a Coding without `system` (validate
    // by code alone, scoped by VS membership). Fall back to an empty string
    // so downstream paths can detect "no system" without rejecting the
    // request as malformed.
    let system = coding
        .get("system")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let code = coding.get("code").and_then(|v| v.as_str())?.to_string();
    let display = coding
        .get("display")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some((system, code, display))
}

/// Extract a `resource`-typed parameter by name, returning the resource JSON.
///
/// FHIR Parameters can carry a full resource as `{"name": "...", "resource": {...}}`.
/// This is used by operations like `$expand` which accept an inline `ValueSet`.
pub fn find_resource_param(params: &[Value], name: &str) -> Option<Value> {
    params
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(name))?
        .get("resource")
        .cloned()
}

/// Extract a `valueCodeableConcept` parameter, returning all `(system, code)` pairs.
///
/// Looks for the first parameter named `name` that carries a
/// `valueCodeableConcept` object and collects every `coding` entry that
/// contains both `system` and `code` fields.  Returns `None` if the parameter
/// is absent; returns an empty `Vec` if the `coding` array is missing or all
/// entries are incomplete.
pub fn extract_codeable_concept(params: &[Value], name: &str) -> Option<Vec<(String, String)>> {
    let cc = params
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(name))?
        .get("valueCodeableConcept")?;
    let codings = cc.get("coding")?.as_array()?;
    let pairs = codings
        .iter()
        .filter_map(|c| {
            let system = c.get("system").and_then(|v| v.as_str())?.to_string();
            let code = c.get("code").and_then(|v| v.as_str())?.to_string();
            Some((system, code))
        })
        .collect();
    Some(pairs)
}

// ── GET / query-string helpers ─────────────────────────────────────────────────

/// Parse a raw URL query string (e.g. `"system=http%3A...&code=ABC"`) into a
/// list of `(name, value)` pairs, with percent-decoding applied.
///
/// Repeated parameters (`?property=parent&property=child`) each produce a
/// separate entry, so `collect_str_params` works correctly for GET requests.
pub fn parse_query_string(raw: &str) -> Vec<(String, String)> {
    form_urlencoded::parse(raw.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

/// Convert `(name, value)` pairs (from a GET query string) into a FHIR
/// `parameter` array compatible with `find_str_param` / `collect_str_params`.
///
/// Each pair becomes `{"name": name, "valueString": value}`.
pub fn query_params_to_fhir_params(pairs: Vec<(String, String)>) -> Vec<Value> {
    pairs
        .into_iter()
        .map(|(name, value)| json!({"name": name, "valueString": value}))
        .collect()
}

// ── Building helpers ───────────────────────────────────────────────────────────

/// Map a FHIR property `value_type` string to the correct `valueXxx` key
/// used in a FHIR Parameters `part` entry.
pub fn property_value_part(value_type: &str, raw_value: &str) -> Value {
    match value_type {
        "code" => serde_json::json!({"name": "value", "valueCode": raw_value}),
        "boolean" => {
            let b = raw_value.parse::<bool>().unwrap_or(false);
            serde_json::json!({"name": "value", "valueBoolean": b})
        }
        "integer" => {
            let n = raw_value.parse::<i64>().unwrap_or(0);
            serde_json::json!({"name": "value", "valueInteger": n})
        }
        "decimal" => {
            let f = raw_value.parse::<f64>().unwrap_or(0.0);
            serde_json::json!({"name": "value", "valueDecimal": f})
        }
        "dateTime" => serde_json::json!({"name": "value", "valueDateTime": raw_value}),
        _ => serde_json::json!({"name": "value", "valueString": raw_value}),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_parameter_array_ok() {
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "code", "valueCode": "ABC"}]
        });
        let params = extract_parameter_array(&body).unwrap();
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn extract_parameter_array_empty_when_no_parameter_key() {
        let body = json!({"resourceType": "Parameters"});
        let params = extract_parameter_array(&body).unwrap();
        assert!(params.is_empty());
    }

    #[test]
    fn extract_parameter_array_rejects_non_parameters() {
        let body = json!({"resourceType": "Patient"});
        let err = extract_parameter_array(&body).unwrap_err();
        assert!(matches!(err, crate::error::HtsError::InvalidRequest(_)));
    }

    #[test]
    fn find_str_param_value_uri() {
        let params = vec![json!({"name": "system", "valueUri": "http://example.org"})];
        assert_eq!(
            find_str_param(&params, "system"),
            Some("http://example.org".into())
        );
    }

    #[test]
    fn find_str_param_value_code() {
        let params = vec![json!({"name": "code", "valueCode": "ABC"})];
        assert_eq!(find_str_param(&params, "code"), Some("ABC".into()));
    }

    #[test]
    fn find_str_param_missing_returns_none() {
        let params: Vec<Value> = vec![];
        assert!(find_str_param(&params, "system").is_none());
    }

    #[test]
    fn collect_str_params_repeated() {
        let params = vec![
            json!({"name": "property", "valueCode": "parent"}),
            json!({"name": "property", "valueCode": "inactive"}),
        ];
        let vals = collect_str_params(&params, "property");
        assert_eq!(vals, vec!["parent", "inactive"]);
    }

    #[test]
    fn property_value_part_code() {
        let part = property_value_part("code", "ROOT");
        assert_eq!(part["valueCode"], "ROOT");
    }

    #[test]
    fn property_value_part_boolean() {
        let part = property_value_part("boolean", "true");
        assert_eq!(part["valueBoolean"], true);
    }

    #[test]
    fn property_value_part_integer() {
        let part = property_value_part("integer", "42");
        assert_eq!(part["valueInteger"], 42);
    }

    #[test]
    fn property_value_part_decimal() {
        let part = property_value_part("decimal", "2.5");
        let v = part["valueDecimal"].as_f64().unwrap();
        assert!((v - 2.5_f64).abs() < 1e-9);
    }

    #[test]
    fn property_value_part_unknown_type_falls_back_to_string() {
        let part = property_value_part("custom", "hello");
        assert_eq!(part["valueString"], "hello");
    }

    // ── extract_coding ─────────────────────────────────────────────────────────

    #[test]
    fn extract_coding_full() {
        let params = vec![json!({
            "name": "coding",
            "valueCoding": {"system": "http://cs.org", "code": "A", "display": "Alpha"}
        })];
        let result = extract_coding(&params, "coding").unwrap();
        assert_eq!(result.0, "http://cs.org");
        assert_eq!(result.1, "A");
        assert_eq!(result.2, Some("Alpha".into()));
    }

    #[test]
    fn extract_coding_without_display() {
        let params = vec![json!({
            "name": "coding",
            "valueCoding": {"system": "http://cs.org", "code": "B"}
        })];
        let result = extract_coding(&params, "coding").unwrap();
        assert_eq!(result.1, "B");
        assert!(result.2.is_none());
    }

    #[test]
    fn extract_coding_missing_param_returns_none() {
        let params: Vec<Value> = vec![];
        assert!(extract_coding(&params, "coding").is_none());
    }

    #[test]
    fn extract_coding_wrong_name_returns_none() {
        let params = vec![json!({
            "name": "other",
            "valueCoding": {"system": "http://cs.org", "code": "A"}
        })];
        assert!(extract_coding(&params, "coding").is_none());
    }

    // ── extract_codeable_concept ───────────────────────────────────────────────

    #[test]
    fn extract_codeable_concept_multiple_codings() {
        let params = vec![json!({
            "name": "codeableConcept",
            "valueCodeableConcept": {
                "coding": [
                    {"system": "http://cs1.org", "code": "A"},
                    {"system": "http://cs2.org", "code": "B"}
                ]
            }
        })];
        let codings = extract_codeable_concept(&params, "codeableConcept").unwrap();
        assert_eq!(codings.len(), 2);
        assert_eq!(codings[0], ("http://cs1.org".into(), "A".into()));
        assert_eq!(codings[1], ("http://cs2.org".into(), "B".into()));
    }

    #[test]
    fn extract_codeable_concept_skips_incomplete_entries() {
        let params = vec![json!({
            "name": "codeableConcept",
            "valueCodeableConcept": {
                "coding": [
                    {"system": "http://cs.org"},         // missing code
                    {"code": "B"},                       // missing system
                    {"system": "http://cs.org", "code": "C"}
                ]
            }
        })];
        let codings = extract_codeable_concept(&params, "codeableConcept").unwrap();
        assert_eq!(codings.len(), 1);
        assert_eq!(codings[0].1, "C");
    }

    #[test]
    fn extract_codeable_concept_missing_param_returns_none() {
        let params: Vec<Value> = vec![];
        assert!(extract_codeable_concept(&params, "codeableConcept").is_none());
    }
}
