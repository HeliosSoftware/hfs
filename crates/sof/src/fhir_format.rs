//! Shared `fhir`-format output and FHIR-representation content negotiation
//! for the SQL-on-FHIR run operations (SoF v2 Common Operation Behavior).
//!
//! Two independent negotiation axes are implemented here so `sof-server` and
//! the HFS REST handlers apply identical rules:
//!
//! - **Axis 1 — format selection**: `_format=fhir` (or
//!   `Accept: application/fhir+json` when `_format` is absent) selects the
//!   `fhir` output format: a `Parameters` resource with one repeating `row`
//!   parameter per result row ([`format_view_fhir_parameters`]).
//! - **Axis 2 — representation**: when a *flat* format is selected and the
//!   client sends `Accept: application/fhir+json`, the raw payload is wrapped
//!   in a serialized `Binary` resource envelope with base64 `data`
//!   ([`wrap_in_binary_envelope`]). The XML envelope form
//!   (`application/fhir+xml`) is not supported and callers should reject it
//!   with `406 Not Acceptable` (see [`accept_requires_unsupported_fhir_xml`]).

use base64::Engine as _;
use serde_json::{Value, json};

use crate::sqlquery::engine::TableSchema;
use crate::sqlquery::output::value_to_fhir_part;
use crate::sqlquery::{ColumnFhirType, SqlQueryError};
use crate::{ProcessedResult, SofError};

/// Native media type of the `fhir` output format.
pub const FHIR_JSON_MIME: &str = "application/fhir+json";

/// Media type of `_format=fhir` export files: newline-delimited FHIR
/// `Parameters` resources, one per result row.
pub const FHIR_NDJSON_MIME: &str = "application/fhir+ndjson";

/// The FHIR XML media type — recognised only to reject it explicitly.
pub const FHIR_XML_MIME: &str = "application/fhir+xml";

/// True when the given `Accept` header value lists `mime` (parameters such as
/// `;q=` are ignored; matching is case-insensitive).
pub fn accept_has_mime(accept: Option<&str>, mime: &str) -> bool {
    let Some(accept) = accept else {
        return false;
    };
    accept
        .split(',')
        .map(|s| s.split(';').next().unwrap_or("").trim())
        .any(|m| m.eq_ignore_ascii_case(mime))
}

/// True when the client asked for a FHIR XML representation without also
/// accepting FHIR JSON. Per the spec, a server that does not support the
/// envelope form requested via `Accept` SHALL respond `406 Not Acceptable`
/// rather than silently returning raw bytes under a FHIR media type; this
/// server does not produce XML.
pub fn accept_requires_unsupported_fhir_xml(accept: Option<&str>) -> bool {
    accept_has_mime(accept, FHIR_XML_MIME) && !accept_has_mime(accept, FHIR_JSON_MIME)
}

/// Wraps raw payload bytes in a serialized FHIR `Binary` resource envelope
/// (`contentType` = the payload's native media type, `data` = base64). The
/// caller serves the result under `application/fhir+json`.
pub fn wrap_in_binary_envelope(content_type: &str, payload: &[u8]) -> Result<Vec<u8>, SofError> {
    // `Binary.contentType` carries the bare media type — strip any
    // parameters (e.g. `; charset=utf-8`) from HTTP header values.
    let media_type = content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim();
    let binary = json!({
        "resourceType": "Binary",
        "contentType": media_type,
        "data": base64::engine::general_purpose::STANDARD.encode(payload),
    });
    serde_json::to_vec(&binary).map_err(SofError::SerializationError)
}

/// Renders a `ProcessedResult` as a FHIR `Parameters` resource per the SoF v2
/// spec's `fhir` output format: one top-level `row` parameter per result row,
/// with one `part` per non-NULL column carrying the appropriate `value[x]`.
///
/// The `value[x]` choice is driven by the ViewDefinition's declared
/// `column.type` (collected from `view_json`, including nested `select` and
/// `unionAll` branches); columns without a declared type default to `string`.
/// NULL cells are omitted. Collection (array) cells repeat the part once per
/// element, matching FHIR's repeating-element semantics. An empty result emits
/// a bare `{"resourceType": "Parameters"}` with the `parameter` key omitted.
pub fn format_view_fhir_parameters(
    result: &ProcessedResult,
    view_json: &Value,
) -> Result<Vec<u8>, SofError> {
    let schema = TableSchema::from_view_definition(view_json);
    let column_types: Vec<ColumnFhirType> = result
        .columns
        .iter()
        .map(|name| {
            schema
                .columns
                .iter()
                .find(|c| &c.name == name)
                .map(|c| c.fhir_type.clone())
                .unwrap_or_else(|| ColumnFhirType::String("string".into()))
        })
        .collect();

    let mut row_params: Vec<Value> = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        let mut parts: Vec<Value> = Vec::with_capacity(row.values.len());
        for (i, cell) in row.values.iter().enumerate() {
            let Some(value) = cell else {
                continue; // NULL → omit the part
            };
            if value.is_null() {
                continue;
            }
            let name = &result.columns[i];
            let ty = &column_types[i];
            match value {
                Value::Array(items) => {
                    for item in items {
                        if item.is_null() {
                            continue;
                        }
                        parts.push(part_or_sof_error(name, item, ty)?);
                    }
                }
                other => parts.push(part_or_sof_error(name, other, ty)?),
            }
        }
        row_params.push(json!({ "name": "row", "part": parts }));
    }

    let body = if row_params.is_empty() {
        json!({ "resourceType": "Parameters" })
    } else {
        json!({ "resourceType": "Parameters", "parameter": row_params })
    };
    serde_json::to_vec(&body).map_err(SofError::SerializationError)
}

/// Renders view-run output rows (column → value JSON objects, as produced by
/// a `SofRunner` row stream) as newline-delimited FHIR `Parameters`
/// resources — the export-side shape of `_format=fhir` per the SoF v2 spec's
/// Common Operation Behavior (media type [`FHIR_NDJSON_MIME`]).
///
/// One `Parameters` resource per line, one line per result row; each row's
/// non-NULL columns become top-level `parameter` entries carrying the
/// appropriate `value[x]`, driven by the ViewDefinition's declared
/// `column.type` (default `string`). Collection (array) cells repeat the
/// parameter once per element. A row with no non-NULL columns emits a bare
/// `{"resourceType":"Parameters"}` line.
pub fn format_view_fhir_ndjson(rows: &[Value], view_json: &Value) -> Result<Vec<u8>, SofError> {
    let schema = TableSchema::from_view_definition(view_json);
    let type_for = |name: &str| -> ColumnFhirType {
        schema
            .columns
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.fhir_type.clone())
            .unwrap_or_else(|| ColumnFhirType::String("string".into()))
    };

    let mut out = Vec::new();
    for row in rows {
        let Some(obj) = row.as_object() else {
            return Err(SofError::InvalidViewDefinition(
                "view output row is not a JSON object; cannot render as FHIR Parameters"
                    .to_string(),
            ));
        };
        let mut parts: Vec<Value> = Vec::with_capacity(obj.len());
        for (name, value) in obj {
            if value.is_null() {
                continue; // NULL → omit
            }
            let ty = type_for(name);
            match value {
                Value::Array(items) => {
                    for item in items {
                        if item.is_null() {
                            continue;
                        }
                        parts.push(part_or_sof_error(name, item, &ty)?);
                    }
                }
                other => parts.push(part_or_sof_error(name, other, &ty)?),
            }
        }
        let resource = if parts.is_empty() {
            json!({ "resourceType": "Parameters" })
        } else {
            json!({ "resourceType": "Parameters", "parameter": parts })
        };
        let line = serde_json::to_vec(&resource).map_err(SofError::SerializationError)?;
        out.extend_from_slice(&line);
        out.push(b'\n');
    }
    Ok(out)
}

fn part_or_sof_error(name: &str, value: &Value, ty: &ColumnFhirType) -> Result<Value, SofError> {
    value_to_fhir_part(name, value, ty).map_err(|e| match e {
        SqlQueryError::UnsupportedFhirValue(col) => SofError::InvalidViewDefinition(format!(
            "column '{col}' holds a complex value that cannot be represented as a \
             FHIR value[x] part; the 'fhir' output format supports scalar columns only"
        )),
        other => SofError::InvalidViewDefinition(other.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProcessedRow;
    use serde_json::json;

    fn view(columns: Value) -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "select": [{ "column": columns }]
        })
    }

    #[test]
    fn typed_columns_map_to_value_x() {
        let result = ProcessedResult {
            columns: vec!["id".into(), "active".into(), "age".into()],
            rows: vec![ProcessedRow {
                values: vec![Some(json!("p1")), Some(json!(true)), Some(json!(42))],
            }],
        };
        let v = view(json!([
            {"name": "id", "path": "id", "type": "id"},
            {"name": "active", "path": "active", "type": "boolean"},
            {"name": "age", "path": "age", "type": "integer"}
        ]));
        let bytes = format_view_fhir_parameters(&result, &v).unwrap();
        let out: Value = serde_json::from_slice(&bytes).unwrap();
        let part = &out["parameter"][0]["part"];
        assert_eq!(part[0]["valueId"], json!("p1"));
        assert_eq!(part[1]["valueBoolean"], json!(true));
        assert_eq!(part[2]["valueInteger"], json!(42));
    }

    #[test]
    fn untyped_column_defaults_to_string_and_null_is_omitted() {
        let result = ProcessedResult {
            columns: vec!["name".into(), "missing".into()],
            rows: vec![ProcessedRow {
                values: vec![Some(json!("Smith")), None],
            }],
        };
        let v = view(json!([{"name": "name", "path": "name.family"}]));
        let bytes = format_view_fhir_parameters(&result, &v).unwrap();
        let out: Value = serde_json::from_slice(&bytes).unwrap();
        let parts = out["parameter"][0]["part"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["valueString"], json!("Smith"));
    }

    #[test]
    fn collection_column_repeats_part_per_element() {
        let result = ProcessedResult {
            columns: vec!["given".into()],
            rows: vec![ProcessedRow {
                values: vec![Some(json!(["John", "Quincy"]))],
            }],
        };
        let v = view(json!([
            {"name": "given", "path": "name.given", "type": "string", "collection": true}
        ]));
        let bytes = format_view_fhir_parameters(&result, &v).unwrap();
        let out: Value = serde_json::from_slice(&bytes).unwrap();
        let parts = out["parameter"][0]["part"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["name"], json!("given"));
        assert_eq!(parts[0]["valueString"], json!("John"));
        assert_eq!(parts[1]["valueString"], json!("Quincy"));
    }

    #[test]
    fn empty_result_omits_parameter_key() {
        let result = ProcessedResult {
            columns: vec!["id".into()],
            rows: vec![],
        };
        let v = view(json!([{"name": "id", "path": "id", "type": "id"}]));
        let bytes = format_view_fhir_parameters(&result, &v).unwrap();
        let out: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(out["resourceType"], json!("Parameters"));
        assert!(out.get("parameter").is_none());
    }

    #[test]
    fn complex_value_errors() {
        let result = ProcessedResult {
            columns: vec!["name".into()],
            rows: vec![ProcessedRow {
                values: vec![Some(json!({"family": "Smith"}))],
            }],
        };
        let v = view(json!([{"name": "name", "path": "name"}]));
        let err = format_view_fhir_parameters(&result, &v).unwrap_err();
        assert!(matches!(err, SofError::InvalidViewDefinition(_)));
    }

    #[test]
    fn accept_matching() {
        assert!(accept_has_mime(
            Some("text/csv, application/fhir+json;q=0.9"),
            FHIR_JSON_MIME
        ));
        assert!(accept_has_mime(
            Some("Application/FHIR+JSON"),
            FHIR_JSON_MIME
        ));
        assert!(!accept_has_mime(Some("application/json"), FHIR_JSON_MIME));
        assert!(!accept_has_mime(None, FHIR_JSON_MIME));

        assert!(accept_requires_unsupported_fhir_xml(Some(
            "application/fhir+xml"
        )));
        assert!(!accept_requires_unsupported_fhir_xml(Some(
            "application/fhir+xml, application/fhir+json"
        )));
        assert!(!accept_requires_unsupported_fhir_xml(Some("text/csv")));
    }

    #[test]
    fn view_fhir_ndjson_one_parameters_per_row() {
        let rows = vec![
            json!({"id": "p1", "active": true, "given": ["John", "Quincy"]}),
            json!({"id": "p2", "active": null}),
        ];
        let v = view(json!([
            {"name": "id", "path": "id", "type": "id"},
            {"name": "active", "path": "active", "type": "boolean"},
            {"name": "given", "path": "name.given", "type": "string", "collection": true}
        ]));
        let bytes = format_view_fhir_ndjson(&rows, &v).unwrap();
        let lines: Vec<Value> = String::from_utf8(bytes)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        for line in &lines {
            assert_eq!(line["resourceType"], json!("Parameters"));
        }
        let p1 = lines[0]["parameter"].as_array().unwrap();
        assert!(
            p1.iter()
                .any(|p| p["name"] == json!("id") && p["valueId"] == json!("p1"))
        );
        assert!(
            p1.iter()
                .any(|p| p["name"] == json!("active") && p["valueBoolean"] == json!(true))
        );
        // collection column repeats the parameter per element
        assert_eq!(p1.iter().filter(|p| p["name"] == json!("given")).count(), 2);
        // NULL column omitted on row 2
        let p2 = lines[1]["parameter"].as_array().unwrap();
        assert_eq!(p2.len(), 1);
        assert_eq!(p2[0]["name"], json!("id"));
    }

    #[test]
    fn view_fhir_ndjson_empty_rows_yield_empty_output() {
        let v = view(json!([{"name": "id", "path": "id", "type": "id"}]));
        let bytes = format_view_fhir_ndjson(&[], &v).unwrap();
        assert!(bytes.is_empty());
    }

    #[test]
    fn binary_envelope_round_trips() {
        let bytes = wrap_in_binary_envelope("text/csv", b"a,b\n1,2\n").unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["resourceType"], json!("Binary"));
        assert_eq!(v["contentType"], json!("text/csv"));
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(v["data"].as_str().unwrap())
            .unwrap();
        assert_eq!(decoded, b"a,b\n1,2\n");
    }
}
