//! Parse the FHIR `Parameters` body for a `$sql-run` whose subject is a
//! SQLQuery or SQLView Library.
//!
//! The subject itself is named by the shared `subjectCanonical` /
//! `subjectReference` / `subjectResource` trio and resolved before this runs
//! (see `helios_sof::params` and the REST `sof::subject` module), so this
//! struct covers only the parameters specific to executing a Library.

use serde_json::Value;

/// Library-specific `$sql-run` parameters lifted out of a FHIR `Parameters`
/// body.
#[derive(Debug, Default, Clone)]
pub struct SqlQueryRunParams {
    /// `_format` — `valueCode` (spec) or `valueString` (lenient). Optional;
    /// defaults to `ndjson`.
    pub format: Option<String>,
    /// `header` — CSV header control (default `true`).
    pub header: Option<bool>,
    /// `parameters` — the nested `Parameters` resource of name-to-value bindings
    /// carried in `parameter.resource`. Left as raw JSON; bound after the
    /// Library's parameter declarations are known.
    ///
    /// Permitted only when the subject is a SQLQuery or SQLView. Supplying it
    /// with a ViewDefinition subject is a `400`, because a ViewDefinition
    /// declares no parameters.
    pub parameters: Option<Value>,
    /// `source` — external data source URL (out of scope v1).
    pub source: Option<String>,
    /// `_limit` — soft cap on the final result-set size, applied AFTER SQL
    /// evaluation (including any in-query `LIMIT`). The server MAY return
    /// fewer rows than requested without erroring; returning fewer rows than
    /// the supplied `_limit` is not an error.
    pub limit: Option<u32>,
}

/// Walks a `Parameters` body and pulls every Library-specific `$sql-run` field.
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
            "_limit" => {
                if out.limit.is_none() {
                    if let Some(n) = p.get("valueInteger").and_then(|v| v.as_u64()) {
                        out.limit = Some(n as u32);
                    } else if let Some(n) = p
                        .get("valuePositiveInt")
                        .or_else(|| p.get("valueUnsignedInt"))
                        .and_then(|v| v.as_u64())
                    {
                        out.limit = Some(n as u32);
                    }
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
    fn extracts_parameter_bindings() {
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": "json"},
                {"name": "parameters", "resource": {
                    "resourceType": "Parameters",
                    "parameter": [{"name": "min_age", "valueInteger": 18}]
                }}
            ]
        });
        let p = extract_sqlquery_params_from_json(&body);
        assert_eq!(p.format.as_deref(), Some("json"));
        assert!(p.parameters.is_some());
    }

    #[test]
    fn non_parameters_body_returns_default() {
        let p = extract_sqlquery_params_from_json(&json!({"resourceType": "Bundle"}));
        assert!(p.format.is_none());
    }

    #[test]
    fn extracts_limit() {
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_limit", "valueInteger": 50}
            ]
        });
        let p = extract_sqlquery_params_from_json(&body);
        assert_eq!(p.limit, Some(50));
    }

    #[test]
    fn pre_ballot_query_parameters_are_ignored() {
        // `queryReference` / `queryResource` belonged to `$sqlquery-run`, which
        // was consolidated into `$sql-run`. The subject now arrives through the
        // shared `subject*` trio, so these names carry no meaning here.
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": "json"},
                {"name": "queryReference", "valueReference": {"reference": "Library/foo"}},
                {"name": "queryResource", "resource": {"resourceType": "Library"}}
            ]
        });
        let p = extract_sqlquery_params_from_json(&body);
        assert_eq!(p.format.as_deref(), Some("json"));
        assert!(p.parameters.is_none());
    }
}
