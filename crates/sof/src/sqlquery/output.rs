//! Output formatting for `$sqlquery-run`.
//!
//! Non-FHIR formats (csv/json/ndjson/parquet) reuse `helios_sof::format_output`
//! via `rows_to_processed_result`. This module owns the `_format=fhir` path:
//! emit a `Parameters` resource whose `value[X]` choices are driven by the
//! query result's per-column FHIR type (recorded during materialization /
//! query execution), not by JSON shape.

use serde_json::{Map, Value, json};

use super::{ColumnFhirType, QueryResult, SqlQueryError};

/// Render a `QueryResult` as a FHIR `Parameters` resource per the SoF v2 spec.
/// One top-level `parameter` per row (name `row`), with one `part` per
/// non-NULL column. NULL columns are omitted entirely.
///
/// Empty result sets emit a bare `{ "resourceType": "Parameters" }` with
/// the `parameter` key omitted, matching FHIR's JSON convention for empty
/// repeating elements (and the upstream sof-js reference test's
/// expectation).
pub fn format_fhir_parameters(result: &QueryResult) -> Result<Vec<u8>, SqlQueryError> {
    let mut row_params: Vec<Value> = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        let mut parts: Vec<Value> = Vec::with_capacity(row.len());
        for (i, cell) in row.iter().enumerate() {
            let Some(value) = cell else {
                continue; // NULL → omit
            };
            let col_name = &result.columns[i];
            let col_type = result
                .column_types
                .get(i)
                .cloned()
                .unwrap_or(ColumnFhirType::String("string".into()));
            let part = value_to_fhir_part(col_name, value, &col_type)?;
            parts.push(part);
        }
        row_params.push(json!({ "name": "row", "part": parts }));
    }
    let body = if row_params.is_empty() {
        json!({ "resourceType": "Parameters" })
    } else {
        json!({ "resourceType": "Parameters", "parameter": row_params })
    };
    serde_json::to_vec(&body).map_err(|e| SqlQueryError::MalformedLibrary(e.to_string()))
}

pub(crate) fn value_to_fhir_part(
    name: &str,
    value: &Value,
    ty: &ColumnFhirType,
) -> Result<Value, SqlQueryError> {
    if matches!(value, Value::Object(_) | Value::Array(_)) {
        return Err(SqlQueryError::UnsupportedFhirValue(name.to_string()));
    }

    let (key, json_value): (&'static str, Value) = match ty {
        ColumnFhirType::Boolean => ("valueBoolean", coerce_bool(value)),
        ColumnFhirType::Integer => {
            // Spec: BIGINT → valueInteger64. The engine infers `Integer` from
            // SQLite INTEGER affinity which covers both 32-bit and 64-bit
            // values; promote to integer64 when the value won't fit in i32.
            match integer_or_promote(value) {
                IntKind::Integer(v) => ("valueInteger", Value::Number(v.into())),
                IntKind::Integer64(s) => ("valueInteger64", Value::String(s)),
                IntKind::Other(v) => ("valueInteger", v),
            }
        }
        // FHIR transports integer64 as JSON string.
        ColumnFhirType::Integer64 => ("valueInteger64", coerce_integer64_string(value)),
        ColumnFhirType::Decimal => ("valueDecimal", coerce_decimal(value)),
        ColumnFhirType::Date => ("valueDate", coerce_string(value)),
        ColumnFhirType::DateTime => ("valueDateTime", coerce_string(value)),
        // Spec SHOULD: round valueInstant to the nearest millisecond.
        ColumnFhirType::Instant => ("valueInstant", coerce_instant(value)),
        ColumnFhirType::Time => ("valueTime", coerce_string(value)),
        ColumnFhirType::Base64Binary => ("valueBase64Binary", coerce_string(value)),
        ColumnFhirType::String(code) => (value_x_key_for(code), coerce_string(value)),
    };

    let mut obj = Map::new();
    obj.insert("name".to_string(), Value::String(name.to_string()));
    obj.insert(key.to_string(), json_value);
    Ok(Value::Object(obj))
}

enum IntKind {
    Integer(i32),
    Integer64(String),
    Other(Value),
}

/// Inspects an inferred integer JSON value. Returns `Integer` if it fits in
/// signed 32-bit, `Integer64` (as a string per FHIR rules) if it doesn't, or
/// `Other` for non-integer JSON values (passes through coerce_integer).
fn integer_or_promote(v: &Value) -> IntKind {
    if let Some(i) = v.as_i64() {
        if (i32::MIN as i64..=i32::MAX as i64).contains(&i) {
            IntKind::Integer(i as i32)
        } else {
            IntKind::Integer64(i.to_string())
        }
    } else if let Some(s) = v.as_str() {
        if let Ok(i) = s.parse::<i64>() {
            if (i32::MIN as i64..=i32::MAX as i64).contains(&i) {
                IntKind::Integer(i as i32)
            } else {
                IntKind::Integer64(i.to_string())
            }
        } else {
            IntKind::Other(Value::String(s.to_string()))
        }
    } else {
        IntKind::Other(coerce_integer(v))
    }
}

/// Parse an RFC-3339 / ISO-8601 timestamp and re-emit it with millisecond
/// precision. Falls through to the raw string when parsing fails (the engine
/// may produce non-timestamp values for `instant` columns when the underlying
/// SQL is unusual).
fn coerce_instant(v: &Value) -> Value {
    let Value::String(s) = v else {
        return coerce_string(v);
    };
    match chrono::DateTime::parse_from_rfc3339(s) {
        Ok(dt) => {
            let rounded = dt
                .with_timezone(&chrono::Utc)
                .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                .to_string();
            Value::String(rounded)
        }
        Err(_) => Value::String(s.clone()),
    }
}

/// Renders query-result rows as newline-delimited FHIR `Parameters`
/// resources — the export-side shape of `_format=fhir` per the SoF v2 spec's
/// Common Operation Behavior (media type `application/fhir+ndjson`).
///
/// One `Parameters` resource per line, one line per result row; each row's
/// non-NULL columns become top-level `parameter` entries carrying the
/// appropriate `value[X]` (the same part shape as a single synchronous `row`
/// parameter's part list). NULL columns are omitted; a row with no non-NULL
/// columns emits a bare `{"resourceType":"Parameters"}` line.
pub fn format_fhir_ndjson_rows(
    columns: &[String],
    column_types: &[ColumnFhirType],
    rows: &[Vec<Option<Value>>],
) -> Result<Vec<u8>, SqlQueryError> {
    let mut out = Vec::new();
    for row in rows {
        let mut parts: Vec<Value> = Vec::with_capacity(row.len());
        for (i, cell) in row.iter().enumerate() {
            let Some(value) = cell else {
                continue; // NULL → omit
            };
            if value.is_null() {
                continue;
            }
            let col_name = &columns[i];
            let col_type = column_types
                .get(i)
                .cloned()
                .unwrap_or(ColumnFhirType::String("string".into()));
            parts.push(value_to_fhir_part(col_name, value, &col_type)?);
        }
        let resource = if parts.is_empty() {
            json!({ "resourceType": "Parameters" })
        } else {
            json!({ "resourceType": "Parameters", "parameter": parts })
        };
        let line = serde_json::to_vec(&resource)
            .map_err(|e| SqlQueryError::MalformedLibrary(e.to_string()))?;
        out.extend_from_slice(&line);
        out.push(b'\n');
    }
    Ok(out)
}

fn value_x_key_for(code: &str) -> &'static str {
    match code {
        "code" => "valueCode",
        "id" => "valueId",
        "uri" => "valueUri",
        "url" => "valueUrl",
        "canonical" => "valueCanonical",
        "markdown" => "valueMarkdown",
        "oid" => "valueOid",
        "uuid" => "valueUuid",
        _ => "valueString",
    }
}

fn coerce_bool(v: &Value) -> Value {
    match v {
        Value::Bool(b) => Value::Bool(*b),
        Value::Number(n) => Value::Bool(n.as_i64().unwrap_or(0) != 0),
        Value::String(s) => Value::Bool(s == "true" || s == "1"),
        _ => Value::Null,
    }
}

fn coerce_integer(v: &Value) -> Value {
    match v {
        Value::Number(n) if n.is_i64() => Value::Number(n.clone()),
        Value::Number(n) => n
            .as_f64()
            .and_then(|f| {
                if f.fract() == 0.0 {
                    serde_json::Number::from_f64(f).map(Value::Number)
                } else {
                    None
                }
            })
            .unwrap_or(Value::Null),
        Value::String(s) => s
            .parse::<i64>()
            .map(|i| Value::Number(i.into()))
            .unwrap_or_else(|_| Value::String(s.clone())),
        _ => Value::Null,
    }
}

fn coerce_integer64_string(v: &Value) -> Value {
    match v {
        Value::Number(n) if n.is_i64() => Value::String(n.to_string()),
        Value::String(s) => Value::String(s.clone()),
        other => Value::String(other.to_string()),
    }
}

fn coerce_decimal(v: &Value) -> Value {
    match v {
        Value::Number(n) => Value::Number(n.clone()),
        Value::String(s) => match s.parse::<f64>() {
            Ok(f) => serde_json::Number::from_f64(f)
                .map(Value::Number)
                .unwrap_or_else(|| Value::String(s.clone())),
            Err(_) => Value::String(s.clone()),
        },
        _ => Value::Null,
    }
}

fn coerce_string(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(s.clone()),
        Value::Number(n) => Value::String(n.to_string()),
        Value::Bool(b) => Value::String(b.to_string()),
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn qr(columns: &[(&str, ColumnFhirType)], rows: Vec<Vec<Option<Value>>>) -> QueryResult {
        QueryResult {
            columns: columns.iter().map(|(n, _)| (*n).to_string()).collect(),
            column_types: columns.iter().map(|(_, t)| t.clone()).collect(),
            rows,
        }
    }

    #[test]
    fn renders_basic_types() {
        let result = qr(
            &[
                ("b", ColumnFhirType::Boolean),
                ("i", ColumnFhirType::Integer),
                ("i64", ColumnFhirType::Integer64),
                ("d", ColumnFhirType::Decimal),
                ("s", ColumnFhirType::String("string".into())),
                ("c", ColumnFhirType::String("code".into())),
                ("ts", ColumnFhirType::Instant),
            ],
            vec![vec![
                Some(json!(true)),
                Some(json!(42)),
                Some(json!(9999999999_i64)),
                Some(json!(2.5)),
                Some(json!("hello")),
                Some(json!("a-code")),
                Some(json!("2025-01-02T03:04:05Z")),
            ]],
        );
        let bytes = format_fhir_parameters(&result).unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        let row = &v["parameter"][0]["part"];
        assert_eq!(row[0]["valueBoolean"], json!(true));
        assert_eq!(row[1]["valueInteger"], json!(42));
        assert_eq!(row[2]["valueInteger64"], json!("9999999999"));
        assert_eq!(row[3]["valueDecimal"], json!(2.5));
        assert_eq!(row[4]["valueString"], json!("hello"));
        assert_eq!(row[5]["valueCode"], json!("a-code"));
        // Spec SHOULD: round valueInstant to the nearest millisecond.
        assert_eq!(row[6]["valueInstant"], json!("2025-01-02T03:04:05.000Z"));
    }

    #[test]
    fn instant_normalises_subsecond_precision_to_millis() {
        let result = qr(
            &[("ts", ColumnFhirType::Instant)],
            vec![vec![Some(json!("2025-01-02T03:04:05.123456789Z"))]],
        );
        let bytes = format_fhir_parameters(&result).unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        // chrono truncates rather than rounds, but ms precision is what the spec asks for.
        assert_eq!(
            v["parameter"][0]["part"][0]["valueInstant"],
            json!("2025-01-02T03:04:05.123Z")
        );
    }

    #[test]
    fn integer_inference_promotes_out_of_i32_range_to_integer64() {
        // Engine-inferred Integer column (no VD-declared type override) holds
        // a value larger than i32. Spec says BIGINT maps to valueInteger64.
        let result = qr(
            &[("n", ColumnFhirType::Integer)],
            vec![vec![Some(json!(9_999_999_999_i64))]],
        );
        let bytes = format_fhir_parameters(&result).unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        let part = &v["parameter"][0]["part"][0];
        assert!(part.get("valueInteger").is_none());
        assert_eq!(part["valueInteger64"], json!("9999999999"));
    }

    #[test]
    fn null_columns_omitted() {
        let result = qr(
            &[
                ("a", ColumnFhirType::Integer),
                ("b", ColumnFhirType::Integer),
            ],
            vec![vec![Some(json!(1)), None]],
        );
        let bytes = format_fhir_parameters(&result).unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        let part = &v["parameter"][0]["part"];
        assert_eq!(part.as_array().unwrap().len(), 1);
        assert_eq!(part[0]["name"], json!("a"));
    }

    #[test]
    fn empty_result_omits_parameter_key() {
        // SoF v2: "Zero-row results return empty Parameters resource".
        // The FHIR JSON convention is to omit empty repeating elements;
        // sof-js asserts `body.parameter === undefined`.
        let result = qr(&[("id", ColumnFhirType::String("id".into()))], Vec::new());
        let bytes = format_fhir_parameters(&result).unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["resourceType"], json!("Parameters"));
        assert!(
            v.get("parameter").is_none(),
            "empty result must omit the 'parameter' key, got {v}"
        );
    }

    #[test]
    fn composite_value_errors() {
        let result = qr(
            &[("a", ColumnFhirType::String("string".into()))],
            vec![vec![Some(json!({"nested": 1}))]],
        );
        let err = format_fhir_parameters(&result).unwrap_err();
        assert!(matches!(err, SqlQueryError::UnsupportedFhirValue(_)));
    }
}
