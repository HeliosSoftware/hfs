//! Bind values from a supplied `Parameters` resource to `Library.parameter`
//! declarations, using FHIR type codes to choose the right rusqlite value.

use rusqlite::types::Value as SqlValue;
use serde_json::Value;

use super::{LibraryParameter, SqlQueryError};

/// A named, type-checked binding. The handler passes a `Vec<BoundParam>` to
/// the engine; `name` is the `Library.parameter.name` without a leading colon.
#[derive(Debug, Clone)]
pub struct BoundParam {
    pub name: String,
    pub value: SqlValue,
}

/// Walks the supplied `parameters` Parameters resource and produces bindings
/// for every `Library.parameter[use="in"]`. Missing values fall back to the
/// declared default; if no default, returns `BindParameter`.
pub fn bind_supplied_params(
    declared: &[LibraryParameter],
    supplied: Option<&Value>,
) -> Result<Vec<BoundParam>, SqlQueryError> {
    let supplied_entries: Vec<&Value> = supplied
        .and_then(|v| v.get("parameter"))
        .and_then(|p| p.as_array())
        .map(|arr| arr.iter().collect())
        .unwrap_or_default();

    // Reject unknown supplied names so callers learn about typos.
    let declared_names: std::collections::HashSet<&str> =
        declared.iter().map(|d| d.name.as_str()).collect();
    for entry in &supplied_entries {
        if let Some(name) = entry.get("name").and_then(|n| n.as_str()) {
            if !declared_names.contains(name) {
                return Err(SqlQueryError::BindParameter(format!(
                    "supplied parameter '{name}' is not declared in Library.parameter"
                )));
            }
        }
    }

    let mut out = Vec::with_capacity(declared.len());
    for p in declared {
        let supplied_entry = supplied_entries
            .iter()
            .find(|e| e.get("name").and_then(|n| n.as_str()) == Some(p.name.as_str()));
        let value = if let Some(entry) = supplied_entry {
            value_for_param(p, entry)?
        } else if let Some(default) = &p.default_value {
            // Default values are FHIR `value[X]` shapes; wrap them in a fake
            // parameter entry to reuse the binder.
            let fake = serde_json::json!({ "name": p.name, "value": default });
            // Default extension shape uses `defaultX`; here we already have
            // the raw value, so synthesize the right key from the type code.
            let key = format!(
                "value{}",
                first_letter_upper(value_x_suffix_for(&p.type_code))
            );
            let mut obj = serde_json::Map::new();
            obj.insert("name".to_string(), Value::String(p.name.clone()));
            obj.insert(key, default.clone());
            let _ = fake; // suppress unused warning when default-value mode
            value_for_param(p, &Value::Object(obj))?
        } else {
            return Err(SqlQueryError::BindParameter(format!(
                "parameter '{}' has no supplied value and no default",
                p.name
            )));
        };
        out.push(BoundParam {
            name: p.name.clone(),
            value,
        });
    }
    Ok(out)
}

fn value_for_param(p: &LibraryParameter, entry: &Value) -> Result<SqlValue, SqlQueryError> {
    let obj = entry
        .as_object()
        .ok_or_else(|| SqlQueryError::BindParameter("parameter entry must be an object".into()))?;

    let suffix = value_x_suffix_for(&p.type_code);
    let expected_keys = expected_value_keys_for(&p.type_code);
    let value = obj
        .iter()
        .find(|(k, _)| {
            k.starts_with("value")
                && (expected_keys.contains(&k.as_str()) || k == &&format!("value{suffix}"))
        })
        .map(|(_, v)| v);

    let value = match value {
        Some(v) => v,
        None => {
            return Err(SqlQueryError::BindParameter(format!(
                "parameter '{}' (type {}) is missing a value{suffix} entry",
                p.name, p.type_code
            )));
        }
    };

    bind_value(&p.name, &p.type_code, value)
}

fn first_letter_upper(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn value_x_suffix_for(type_code: &str) -> &'static str {
    match type_code {
        "boolean" => "Boolean",
        "integer" | "positiveInt" | "unsignedInt" => "Integer",
        "integer64" => "Integer64",
        "decimal" => "Decimal",
        "date" => "Date",
        "dateTime" => "DateTime",
        "instant" => "Instant",
        "time" => "Time",
        "string" => "String",
        "code" => "Code",
        "id" => "Id",
        "uri" => "Uri",
        "url" => "Url",
        "canonical" => "Canonical",
        "markdown" => "Markdown",
        "oid" => "Oid",
        "uuid" => "Uuid",
        "base64Binary" => "Base64Binary",
        _ => "String",
    }
}

fn expected_value_keys_for(type_code: &str) -> &'static [&'static str] {
    match type_code {
        "boolean" => &["valueBoolean"],
        "integer" | "positiveInt" | "unsignedInt" => {
            &["valueInteger", "valuePositiveInt", "valueUnsignedInt"]
        }
        "integer64" => &["valueInteger64"],
        "decimal" => &["valueDecimal"],
        "date" => &["valueDate"],
        "dateTime" => &["valueDateTime"],
        "instant" => &["valueInstant"],
        "time" => &["valueTime"],
        "string" | "code" | "id" | "uri" | "url" | "canonical" | "markdown" | "oid" | "uuid" => &[
            "valueString",
            "valueCode",
            "valueId",
            "valueUri",
            "valueUrl",
            "valueCanonical",
            "valueMarkdown",
            "valueOid",
            "valueUuid",
        ],
        "base64Binary" => &["valueBase64Binary"],
        _ => &["valueString"],
    }
}

fn bind_value(name: &str, type_code: &str, v: &Value) -> Result<SqlValue, SqlQueryError> {
    let invalid = |reason: String| SqlQueryError::BindParameter(format!("'{name}': {reason}"));
    match type_code {
        "boolean" => v
            .as_bool()
            .map(|b| SqlValue::Integer(if b { 1 } else { 0 }))
            .ok_or_else(|| invalid("expected JSON boolean".into())),
        "integer" | "positiveInt" | "unsignedInt" => v
            .as_i64()
            .map(SqlValue::Integer)
            .ok_or_else(|| invalid("expected JSON integer".into())),
        "integer64" => {
            // FHIR transports integer64 as a JSON string (per the FHIR spec) but
            // many clients send a number. Accept either.
            if let Some(i) = v.as_i64() {
                return Ok(SqlValue::Integer(i));
            }
            if let Some(s) = v.as_str() {
                return s
                    .parse::<i64>()
                    .map(SqlValue::Integer)
                    .map_err(|e| invalid(format!("integer64 parse: {e}")));
            }
            Err(invalid("expected JSON integer or numeric string".into()))
        }
        "decimal" => v
            .as_f64()
            .map(SqlValue::Real)
            .or_else(|| v.as_i64().map(|i| SqlValue::Real(i as f64)))
            .ok_or_else(|| invalid("expected JSON number".into())),
        "date" => {
            let s = v
                .as_str()
                .ok_or_else(|| invalid("expected JSON string".into()))?;
            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map_err(|e| invalid(format!("invalid date '{s}': {e}")))?;
            Ok(SqlValue::Text(s.to_string()))
        }
        "dateTime" | "instant" => {
            let s = v
                .as_str()
                .ok_or_else(|| invalid("expected JSON string".into()))?;
            chrono::DateTime::parse_from_rfc3339(s)
                .map_err(|e| invalid(format!("invalid {type_code} '{s}': {e}")))?;
            Ok(SqlValue::Text(s.to_string()))
        }
        "time" => {
            let s = v
                .as_str()
                .ok_or_else(|| invalid("expected JSON string".into()))?;
            chrono::NaiveTime::parse_from_str(s, "%H:%M:%S")
                .or_else(|_| chrono::NaiveTime::parse_from_str(s, "%H:%M:%S%.f"))
                .map_err(|e| invalid(format!("invalid time '{s}': {e}")))?;
            Ok(SqlValue::Text(s.to_string()))
        }
        // String-ish FHIR types.
        _ => v
            .as_str()
            .map(|s| SqlValue::Text(s.to_string()))
            .ok_or_else(|| invalid("expected JSON string".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn decl(name: &str, type_code: &str) -> LibraryParameter {
        LibraryParameter {
            name: name.into(),
            type_code: type_code.into(),
            has_default: false,
            default_value: None,
        }
    }

    #[test]
    fn binds_integer_and_string() {
        let declared = vec![decl("min", "integer"), decl("city", "string")];
        let supplied = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "min", "valueInteger": 18},
                {"name": "city", "valueString": "NYC"}
            ]
        });
        let out = bind_supplied_params(&declared, Some(&supplied)).unwrap();
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0].value, SqlValue::Integer(18)));
        assert!(matches!(&out[1].value, SqlValue::Text(s) if s == "NYC"));
    }

    #[test]
    fn missing_required_param_errors() {
        let declared = vec![decl("min", "integer")];
        let supplied = json!({"resourceType": "Parameters", "parameter": []});
        let err = bind_supplied_params(&declared, Some(&supplied)).unwrap_err();
        assert!(matches!(err, SqlQueryError::BindParameter(_)));
    }

    #[test]
    fn unknown_supplied_param_errors() {
        let declared = vec![decl("min", "integer")];
        let supplied = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "min", "valueInteger": 1},
                {"name": "unknown", "valueString": "x"}
            ]
        });
        let err = bind_supplied_params(&declared, Some(&supplied)).unwrap_err();
        assert!(matches!(err, SqlQueryError::BindParameter(_)));
    }

    #[test]
    fn type_mismatch_errors() {
        let declared = vec![decl("min", "integer")];
        let supplied = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "min", "valueString": "oops"}]
        });
        let err = bind_supplied_params(&declared, Some(&supplied)).unwrap_err();
        assert!(matches!(err, SqlQueryError::BindParameter(_)));
    }

    #[test]
    fn datetime_validates() {
        let declared = vec![decl("ts", "dateTime")];
        let supplied = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "ts", "valueDateTime": "not-a-date"}]
        });
        assert!(bind_supplied_params(&declared, Some(&supplied)).is_err());

        let ok = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "ts", "valueDateTime": "2025-01-02T03:04:05Z"}]
        });
        assert!(bind_supplied_params(&declared, Some(&ok)).is_ok());
    }

    #[test]
    fn injection_payload_bound_as_text() {
        let declared = vec![decl("name", "string")];
        let supplied = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "name", "valueString": "Robert');--"}]
        });
        let out = bind_supplied_params(&declared, Some(&supplied)).unwrap();
        match &out[0].value {
            SqlValue::Text(s) => assert_eq!(s, "Robert');--"),
            _ => panic!("expected Text"),
        }
    }
}
