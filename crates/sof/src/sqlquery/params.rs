//! Parse the FHIR `Parameters` body for `$sqlquery-run`.

use serde_json::Value;

/// Parameters lifted out of a FHIR `Parameters` body for `$sqlquery-run`.
#[derive(Debug, Default, Clone)]
pub struct SqlQueryRunParams {
    /// `_format` — `valueCode` (spec) or `valueString` (lenient). **Required**.
    pub format: Option<String>,
    /// `header` — CSV header control (default `true`).
    pub header: Option<bool>,
    /// `queryReference` — extracted strictly from `valueReference.reference`
    /// per the operation's `Reference` typing. May be a relative `Library/{id}`
    /// or an absolute / canonical URL the server can resolve.
    pub query_reference: Option<String>,
    /// `queryResource` — inline `Library` resource carried in `parameter.resource`.
    pub query_resource: Option<Value>,
    /// `parameters` — the nested `Parameters` resource of name-to-value bindings
    /// carried in `parameter.resource`. Left as raw JSON; bound after the
    /// Library's parameter declarations are known.
    pub parameters: Option<Value>,
    /// `source` — external data source URL (out of scope v1).
    pub source: Option<String>,
}

/// Walks a `Parameters` body and pulls every `$sqlquery-run` field.
pub fn extract_sqlquery_params_from_json(body: &Value) -> SqlQueryRunParams {
    let mut out = SqlQueryRunParams::default();
    if body.get("resourceType").and_then(|v| v.as_str()) != Some("Parameters") {
        return out;
    }
    let Some(entries) = body.get("parameter").and_then(|p| p.as_array()) else {
        return out;
    };
    for p in entries {
        let Some(name) = p.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        match name {
            "_format" | "format" => {
                if out.format.is_none() {
                    out.format = read_str(p, &["valueCode", "valueString"]);
                }
            }
            "header" => {
                if out.header.is_none() {
                    if let Some(b) = p.get("valueBoolean").and_then(|v| v.as_bool()) {
                        out.header = Some(b);
                    } else if let Some(s) = p.get("valueString").and_then(|v| v.as_str()) {
                        out.header = Some(s == "true" || s == "1");
                    }
                }
            }
            "queryReference" => {
                if out.query_reference.is_none() {
                    out.query_reference = read_reference(p);
                }
            }
            "queryResource" => {
                if out.query_resource.is_none() {
                    if let Some(r) = p.get("resource") {
                        out.query_resource = Some(r.clone());
                    }
                }
            }
            "parameters" => {
                if out.parameters.is_none() {
                    if let Some(r) = p.get("resource") {
                        out.parameters = Some(r.clone());
                    }
                }
            }
            "source" => {
                if out.source.is_none() {
                    out.source = read_str(p, &["valueString", "valueUri"]);
                }
            }
            _ => {}
        }
    }
    out
}

fn read_str(p: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = p.get(*k).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

/// Spec: `queryReference` is typed as `Reference`, so only
/// `valueReference.reference` is honored. Other shapes (`valueString`,
/// `valueUri`, `valueCanonical`) are ignored.
fn read_reference(p: &Value) -> Option<String> {
    p.get("valueReference")
        .and_then(|v| v.get("reference"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_format_and_header() {
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": "csv"},
                {"name": "header", "valueBoolean": false}
            ]
        });
        let p = extract_sqlquery_params_from_json(&body);
        assert_eq!(p.format.as_deref(), Some("csv"));
        assert_eq!(p.header, Some(false));
    }

    #[test]
    fn extracts_query_reference_and_resource() {
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": "json"},
                {"name": "queryReference", "valueReference": {"reference": "Library/foo"}},
                {"name": "queryResource", "resource": {"resourceType": "Library"}}
            ]
        });
        let p = extract_sqlquery_params_from_json(&body);
        assert_eq!(p.query_reference.as_deref(), Some("Library/foo"));
        assert!(p.query_resource.is_some());
    }

    #[test]
    fn non_parameters_body_returns_default() {
        let p = extract_sqlquery_params_from_json(&json!({"resourceType": "Bundle"}));
        assert!(p.format.is_none());
    }

    #[test]
    fn query_reference_only_reads_value_reference() {
        // valueString / valueUri / valueCanonical are NOT accepted — the spec
        // types queryReference strictly as Reference.
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": "json"},
                {"name": "queryReference", "valueString": "Library/foo"}
            ]
        });
        let p = extract_sqlquery_params_from_json(&body);
        assert!(p.query_reference.is_none());
    }
}
