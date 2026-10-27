//! Content-type negotiation and XML serialization for FHIR responses.
//!
//! FHIR servers SHOULD support both JSON (`application/fhir+json`) and XML
//! (`application/fhir+xml`) response formats.  The format is selected by:
//!
//! 1. The `_format` query parameter (takes precedence over the Accept header).
//! 2. The `Accept` request header.
//! 3. Default: JSON.
//!
//! # XML conversion
//!
//! All HTS handlers work with `serde_json::Value` internally.  The
//! [`json_to_fhir_xml`] function converts a FHIR JSON object to a valid FHIR
//! XML string by applying the standard JSON↔XML mapping rules:
//!
//! | JSON | XML |
//! |------|-----|
//! | `{"resourceType": "Foo", …}` | `<Foo xmlns="http://hl7.org/fhir">…</Foo>` |
//! | `{"field": "value"}` | `<field value="value"/>` |
//! | `{"field": true}` | `<field value="true"/>` |
//! | `{"field": {…}}` | `<field>…</field>` |
//! | `{"field": [{…}, {…}]}` | `<field>…</field><field>…</field>` |
//! | `{"field": {"resourceType": "X", …}}` | `<field><X>…</X></field>` |

use axum::Json;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::Value;

// ── Format negotiation ─────────────────────────────────────────────────────────

/// The response serialization format requested by the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseFormat {
    Json,
    Xml,
}

/// Determine the response format from the `_format` query parameter and/or the
/// `Accept` header.
///
/// Priority:
/// 1. `_format` query parameter (`xml`, `json`, or MIME type containing `xml`/`json`)
/// 2. `Accept` header (any value containing `xml`)
/// 3. Default: JSON
pub fn negotiate_format(raw_query: Option<&str>, accept: Option<&str>) -> ResponseFormat {
    // 1. Check _format query param.
    if let Some(raw) = raw_query {
        for (k, v) in form_urlencoded::parse(raw.as_bytes()) {
            if k == "_format" {
                let v = v.as_ref();
                if v.contains("xml") {
                    return ResponseFormat::Xml;
                }
                if v.contains("json") {
                    return ResponseFormat::Json;
                }
            }
        }
    }

    // 2. Check Accept header.
    if let Some(accept) = accept {
        if accept.contains("xml") {
            return ResponseFormat::Xml;
        }
    }

    ResponseFormat::Json
}

// ── Response builder ───────────────────────────────────────────────────────────

/// Serialize `body` using the negotiated format and return an Axum response.
pub fn fhir_respond(body: Value, format: ResponseFormat) -> Response {
    match format {
        ResponseFormat::Json => Json(body).into_response(),
        ResponseFormat::Xml => {
            let xml = json_to_fhir_xml(&body);
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/fhir+xml; charset=utf-8")],
                xml,
            )
                .into_response()
        }
    }
}

// ── FHIR JSON → XML conversion ─────────────────────────────────────────────────

/// Convert a FHIR JSON value (a `serde_json::Value::Object` with a
/// `resourceType` key) to a FHIR XML string.
///
/// The output conforms to the FHIR XML format specification:
/// - The root element tag is the `resourceType` value.
/// - The FHIR namespace `xmlns="http://hl7.org/fhir"` is added to the root.
/// - Primitive fields (string, bool, number) serialize as `<field value="…"/>`.
/// - Complex fields (objects, arrays) serialize as nested elements.
/// - `null` fields are silently skipped.
/// - Nested FHIR resources (objects with a `resourceType` key) are wrapped in
///   a container element: `<field><ResourceType>…</ResourceType></field>`.
pub fn json_to_fhir_xml(value: &Value) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    if let Some(obj) = value.as_object() {
        let rt = obj
            .get("resourceType")
            .and_then(|v| v.as_str())
            .unwrap_or("Resource");
        out.push('<');
        out.push_str(rt);
        out.push_str(" xmlns=\"http://hl7.org/fhir\">");
        write_object_children(&mut out, obj);
        out.push_str("</");
        out.push_str(rt);
        out.push('>');
    }
    out
}

/// Append all child elements of a JSON object, skipping `resourceType`.
fn write_object_children(out: &mut String, obj: &serde_json::Map<String, Value>) {
    for (key, val) in obj {
        if key == "resourceType" {
            continue;
        }
        write_value(out, key, val);
    }
}

/// Append the XML representation of a single `(name, value)` pair.
fn write_value(out: &mut String, name: &str, value: &Value) {
    match value {
        Value::Null => {} // omit null fields
        Value::Bool(b) => {
            out.push('<');
            out.push_str(name);
            out.push_str(" value=\"");
            out.push_str(if *b { "true" } else { "false" });
            out.push_str("\"/>");
        }
        Value::Number(n) => {
            out.push('<');
            out.push_str(name);
            out.push_str(" value=\"");
            out.push_str(&n.to_string());
            out.push_str("\"/>");
        }
        Value::String(s) => {
            out.push('<');
            out.push_str(name);
            out.push_str(" value=\"");
            out.push_str(&xml_escape(s));
            out.push_str("\"/>");
        }
        Value::Array(arr) => {
            // Each array element repeats the parent element name.
            for item in arr {
                write_value(out, name, item);
            }
        }
        Value::Object(obj) => {
            // Nested FHIR resource container: <name><ResourceType>…</ResourceType></name>
            if let Some(rt) = obj.get("resourceType").and_then(|v| v.as_str()) {
                out.push('<');
                out.push_str(name);
                out.push('>');
                out.push('<');
                out.push_str(rt);
                out.push('>');
                write_object_children(out, obj);
                out.push_str("</");
                out.push_str(rt);
                out.push_str("></");
                out.push_str(name);
                out.push('>');
            } else {
                // Plain complex element.
                out.push('<');
                out.push_str(name);
                out.push('>');
                write_object_children(out, obj);
                out.push_str("</");
                out.push_str(name);
                out.push('>');
            }
        }
    }
}

/// Escape characters that are not valid in XML attribute values.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── negotiate_format ───────────────────────────────────────────────────────

    #[test]
    fn format_default_is_json() {
        assert_eq!(negotiate_format(None, None), ResponseFormat::Json);
    }

    #[test]
    fn format_format_param_xml() {
        assert_eq!(
            negotiate_format(Some("_format=xml"), None),
            ResponseFormat::Xml
        );
    }

    #[test]
    fn format_format_param_fhir_xml() {
        assert_eq!(
            negotiate_format(Some("_format=application%2Ffhir%2Bxml"), None),
            ResponseFormat::Xml
        );
    }

    #[test]
    fn format_format_param_json_beats_xml_accept() {
        assert_eq!(
            negotiate_format(Some("_format=json"), Some("application/fhir+xml")),
            ResponseFormat::Json
        );
    }

    #[test]
    fn format_accept_header_xml() {
        assert_eq!(
            negotiate_format(None, Some("application/fhir+xml")),
            ResponseFormat::Xml
        );
    }

    #[test]
    fn format_accept_header_json() {
        assert_eq!(
            negotiate_format(None, Some("application/fhir+json")),
            ResponseFormat::Json
        );
    }

    // ── json_to_fhir_xml ───────────────────────────────────────────────────────

    #[test]
    fn xml_root_element_uses_resource_type() {
        let v = json!({"resourceType": "Parameters", "parameter": []});
        let xml = json_to_fhir_xml(&v);
        assert!(xml.contains("<Parameters xmlns=\"http://hl7.org/fhir\">"));
        assert!(xml.contains("</Parameters>"));
    }

    #[test]
    fn xml_primitive_string_becomes_value_attr() {
        let v = json!({"resourceType": "Parameters", "id": "abc"});
        let xml = json_to_fhir_xml(&v);
        assert!(xml.contains("<id value=\"abc\"/>"));
    }

    #[test]
    fn xml_primitive_bool_becomes_value_attr() {
        let v = json!({"resourceType": "Parameters", "active": true});
        let xml = json_to_fhir_xml(&v);
        assert!(xml.contains("<active value=\"true\"/>"));
    }

    #[test]
    fn xml_array_elements_repeated() {
        let v = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "result", "valueBoolean": true}
            ]
        });
        let xml = json_to_fhir_xml(&v);
        assert!(xml.contains("<parameter>"));
        assert!(xml.contains("</parameter>"));
    }

    #[test]
    fn xml_null_field_omitted() {
        let v = json!({"resourceType": "Parameters", "id": null, "status": "active"});
        let xml = json_to_fhir_xml(&v);
        assert!(!xml.contains("<id"));
        assert!(xml.contains("<status value=\"active\"/>"));
    }

    #[test]
    fn xml_special_chars_escaped_in_attr() {
        let v = json!({"resourceType": "Parameters", "url": "http://example.org/cs&test\"<>"});
        let xml = json_to_fhir_xml(&v);
        assert!(xml.contains("&amp;"));
        assert!(xml.contains("&quot;"));
        assert!(xml.contains("&lt;"));
        assert!(xml.contains("&gt;"));
    }

    #[test]
    fn xml_nested_resource_container() {
        let v = json!({
            "resourceType": "Bundle",
            "entry": [
                {
                    "resource": {
                        "resourceType": "Patient",
                        "id": "p1"
                    }
                }
            ]
        });
        let xml = json_to_fhir_xml(&v);
        assert!(xml.contains("<resource><Patient>"));
        assert!(xml.contains("</Patient></resource>"));
    }
}
