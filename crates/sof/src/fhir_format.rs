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
    let schema = TableSchema::try_from_view_definition(view_json)?;
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
            if let Some(value) = cell {
                push_cell_parts(&mut parts, &result.columns[i], value, &column_types[i])?;
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

/// Appends the parts of one cell: nothing for a NULL, one part for a scalar,
/// and one part per non-NULL element for a collection.
fn push_cell_parts(
    parts: &mut Vec<Value>,
    name: &str,
    value: &Value,
    ty: &ColumnFhirType,
) -> Result<(), SofError> {
    match value {
        Value::Null => {}
        Value::Array(items) => {
            for item in items.iter().filter(|item| !item.is_null()) {
                parts.push(part_or_sof_error(name, item, ty)?);
            }
        }
        other => parts.push(part_or_sof_error(name, other, ty)?),
    }
    Ok(())
}

/// Row-at-a-time form of [`format_view_fhir_parameters`], driven by
/// [`crate::RowStreamEncoder::fhir`]: the same compact bytes, produced without
/// holding the rows or the `Parameters` tree in memory.
///
/// As in [`crate::rows_to_processed_result`], the columns are the keys of the
/// first row: a later row's missing key gives no part, keys the first row did
/// not carry are dropped, and a non-object row gives no parts.
pub(crate) struct FhirRowEncoder {
    schema: TableSchema,
    columns: Option<Vec<(String, ColumnFhirType)>>,
    out: Vec<u8>,
    rows: usize,
}

impl FhirRowEncoder {
    /// Resolves the declared column types once; a bad declaration fails here,
    /// as it does in the buffered formatter, even for an empty result.
    pub(crate) fn new(view_json: &Value) -> Result<Self, SofError> {
        Ok(Self {
            schema: TableSchema::try_from_view_definition(view_json)?,
            columns: None,
            out: Vec::new(),
            rows: 0,
        })
    }

    pub(crate) fn push_row(&mut self, row: &Value) -> Result<(), SofError> {
        if self.columns.is_none() {
            let names = row.as_object().into_iter().flat_map(|map| map.keys());
            self.columns = Some(
                names
                    .map(|name| (name.clone(), self.column_type(name)))
                    .collect(),
            );
        }
        let columns = self.columns.as_deref().unwrap_or_default();
        let mut parts: Vec<Value> = Vec::with_capacity(columns.len());
        if let Value::Object(map) = row {
            for (name, ty) in columns {
                if let Some(value) = map.get(name) {
                    push_cell_parts(&mut parts, name, value, ty)?;
                }
            }
        }
        self.out.extend_from_slice(if self.rows == 0 {
            br#"{"resourceType":"Parameters","parameter":["#
        } else {
            b","
        });
        self.out.extend_from_slice(br#"{"name":"row","part":"#);
        serde_json::to_writer(&mut self.out, &parts).map_err(SofError::SerializationError)?;
        self.out.push(b'}');
        self.rows += 1;
        Ok(())
    }

    pub(crate) fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    /// Closes the resource; no rows is the bare `Parameters` object.
    pub(crate) fn finish(&mut self) -> Vec<u8> {
        self.out.extend_from_slice(if self.rows == 0 {
            br#"{"resourceType":"Parameters"}"#
        } else {
            b"]}"
        });
        self.take()
    }

    /// A column the ViewDefinition does not declare is a string.
    fn column_type(&self, name: &str) -> ColumnFhirType {
        self.schema
            .columns
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.fhir_type.clone())
            .unwrap_or_else(|| ColumnFhirType::String("string".into()))
    }
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
    fn invalid_ansi_type_tag_is_rejected() {
        let result = ProcessedResult {
            columns: vec!["c".into()],
            rows: vec![ProcessedRow {
                values: vec![Some(json!("1.5"))],
            }],
        };
        let v = view(json!([{
            "name": "c", "path": "x", "type": "decimal",
            "tag": [{"name": "ansi/type", "value": "GEOMETRY"}]
        }]));
        let err = format_view_fhir_parameters(&result, &v).unwrap_err();
        assert!(matches!(err, SofError::InvalidViewDefinition(m) if m.contains("'c'")));
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

    /// Rows covering every shape the encoder meets: nulls, collections, a
    /// later row missing a key, an extra key and a non-object row.
    fn mixed_rows() -> Vec<Value> {
        vec![
            json!({"id": "a", "n": 1, "tags": ["x", null, "y"], "flag": true}),
            json!({"n": 2, "id": "b", "tags": [], "flag": null}),
            json!({"id": "c", "extra": "dropped", "tags": ["z"], "undeclared": 7}),
            json!("not an object"),
            json!({"id": "d", "n": 5_000_000_000_i64, "tags": null, "flag": false}),
        ]
    }

    fn mixed_view() -> Value {
        view(json!([
            {"name": "id", "path": "id", "type": "id"},
            {"name": "n", "path": "n", "type": "integer"},
            {"name": "tags", "path": "tags", "type": "string", "collection": true},
            {"name": "flag", "path": "flag", "type": "boolean"}
        ]))
    }

    fn encode_rows(rows: &[Value], view: &Value, chunk: usize) -> Vec<u8> {
        let mut encoder = crate::RowStreamEncoder::fhir(view).unwrap();
        let mut out = Vec::new();
        for (i, row) in rows.iter().enumerate() {
            encoder.push_row(row.clone()).unwrap();
            if chunk != 0 && (i + 1) % chunk == 0 {
                out.extend(encoder.take().unwrap());
            }
        }
        out.extend(encoder.finish().unwrap());
        out
    }

    #[test]
    fn row_encoder_matches_the_buffered_parameters() {
        let all = mixed_rows();
        for n in 0..=all.len() {
            let buffered = format_view_fhir_parameters(
                &crate::rows_to_processed_result(all[..n].to_vec()),
                &mixed_view(),
            )
            .unwrap();
            for chunk in [0, 1, 2] {
                assert_eq!(
                    String::from_utf8_lossy(&encode_rows(&all[..n], &mixed_view(), chunk)),
                    String::from_utf8_lossy(&buffered),
                    "{n} rows, chunk {chunk}"
                );
            }
        }
    }

    #[test]
    fn row_encoder_with_a_non_object_first_row_has_no_columns() {
        let rows = [json!(1), json!({"id": "a"})];
        let buffered = format_view_fhir_parameters(
            &crate::rows_to_processed_result(rows.to_vec()),
            &mixed_view(),
        )
        .unwrap();
        assert_eq!(encode_rows(&rows, &mixed_view(), 0), buffered);
    }

    #[test]
    fn row_encoder_rejects_a_complex_value_and_a_bad_declaration() {
        let mut encoder = crate::RowStreamEncoder::fhir(&mixed_view()).unwrap();
        let err = encoder
            .push_row(json!({"id": {"family": "Smith"}}))
            .unwrap_err();
        assert!(matches!(err, SofError::InvalidViewDefinition(_)));

        let bad = view(json!([{
            "name": "c", "path": "x", "type": "decimal",
            "tag": [{"name": "ansi/type", "value": "GEOMETRY"}]
        }]));
        assert!(matches!(
            crate::RowStreamEncoder::fhir(&bad),
            Err(SofError::InvalidViewDefinition(_))
        ));
    }
}
