//! Shared `$sql-run` parameter extraction.
//!
//! Both the REST handler in `helios-rest` and the standalone sof-server walk a
//! FHIR `Parameters` body for the same set of operation parameters
//! (`_format`, `_limit`, `_since`, `patient`, `group`, `subjectCanonical`,
//! `subjectReference`, `subjectResource`, `resource`, `header`, plus the
//! Parquet options). This module owns the field-name list and the accepted
//! JSON shapes so a new parameter name only needs to be added in one place.
//!
//! ## Subject naming
//!
//! SQL on FHIR 3.0.0-ballot consolidated `$viewdefinition-run` and
//! `$sqlquery-run` into a single system-level `$sql-run`, which names what it
//! acts on through a *subject* rather than through the request path. The three
//! subject parameters are mutually exclusive and one is required:
//!
//! | Parameter           | Names the subject by                                |
//! |---------------------|-----------------------------------------------------|
//! | `subjectCanonical`  | Canonical URL, optionally with a `\|version` suffix  |
//! | `subjectReference`  | Literal location — relative on this server, or absolute |
//! | `subjectResource`   | Inline resource (POST only)                         |
//!
//! `subjectCanonical` and `subjectReference` are deliberately distinct: a
//! canonical URL is an identity, a literal reference is a location, and the
//! same string can be neither or both. The pre-ballot `viewResource` /
//! `viewReference` pair conflated the two and is not accepted.
//!
//! The extractor is **permissive**: missing / wrong-typed `value[X]` fields
//! produce `None`/empty rather than an error. Strict callers (sof-server) run
//! an additional validation pass on the same JSON for bounds checks
//! (e.g. `_limit` upper bound, `compression` allowed values).

use serde_json::Value;

/// SoF v2 `$viewdefinition-run` parameters lifted out of a JSON `Parameters`
/// resource. Scalar fields hold the first occurrence; `patient`, `group`,
/// `inline_resources` collect every entry (spec is `0..*`).
#[derive(Debug, Default, Clone)]
pub struct ExtractedRunParams {
    /// `_format` — `valueCode` or `valueString`.
    pub format: Option<String>,
    /// `header` — `valueBoolean` (preferred) or `valueString` (lenient).
    pub header: Option<bool>,
    /// `_limit` — `valueInteger` or `valuePositiveInt`.
    pub limit: Option<u64>,
    /// `_since` — `valueInstant`, `valueDateTime`, or `valueString`.
    pub since: Option<String>,
    /// `patient` — `valueReference.reference` or `valueString` (any number).
    pub patient: Vec<String>,
    /// `group` — `valueReference.reference` or `valueString` (any number).
    pub group: Vec<String>,
    /// `subjectResource` — the inline `resource` (a ViewDefinition, SQLQuery
    /// Library or SQLView Library).
    pub subject_resource: Option<Value>,
    /// `subjectReference` — literal location, `valueReference.reference` or
    /// `valueString`.
    pub subject_reference: Option<String>,
    /// `subjectCanonical` — canonical URL, `valueCanonical`/`valueUri`/
    /// `valueString`, optionally carrying a `|version` suffix.
    pub subject_canonical: Option<String>,
    /// `resource` — every inline resource encountered (any number).
    pub inline_resources: Vec<Value>,
    /// `source` — `valueString` or `valueUri`.
    pub source: Option<String>,
    /// `maxFileSize` — `valueInteger` or `valuePositiveInt`.
    pub max_file_size: Option<u64>,
    /// `rowGroupSize` — `valueInteger` or `valuePositiveInt`.
    pub row_group_size: Option<u64>,
    /// `pageSize` — `valueInteger` or `valuePositiveInt`.
    pub page_size: Option<u64>,
    /// `compression` — `valueCode` or `valueString`.
    pub compression: Option<String>,
}

/// Splits a comma-separated reference string into trimmed, non-empty
/// entries. Used by both sof-server and HFS REST to lower a single
/// `?group=Group/a,Group/b` query value into the spec's `0..*` shape.
/// Returns an empty `Vec` when the input is `None` or yields no
/// non-empty entries.
pub fn split_csv_refs(value: Option<&str>) -> Vec<String> {
    match value {
        Some(s) => s
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect(),
        None => Vec::new(),
    }
}

/// Returns `true` when `body` names a subject the caller can run — either a
/// bare `ViewDefinition` resource or a `Parameters` body carrying one of the
/// three subject parameters.
pub fn body_has_subject(body: &Value) -> bool {
    match body.get("resourceType").and_then(|v| v.as_str()) {
        Some("ViewDefinition") => true,
        Some("Parameters") => body
            .get("parameter")
            .and_then(|p| p.as_array())
            .map(|params| {
                params.iter().any(|p| {
                    matches!(
                        parameter_name(p).as_deref(),
                        Some("subjectResource")
                            | Some("subjectReference")
                            | Some("subjectCanonical")
                    )
                })
            })
            .unwrap_or(false),
        _ => false,
    }
}

/// Walks a JSON `Parameters` body (or any object with a `parameter` array)
/// and pulls every SoF v2 run-operation field into [`ExtractedRunParams`].
///
/// Returns an empty struct when `body` isn't a `Parameters` resource — call
/// sites that may receive a bare `ViewDefinition` should detect that case
/// separately (e.g. via [`body_has_subject`]). Repeated entries for
/// the same scalar field keep the first value; `patient` / `group` /
/// `inline_resources` accumulate.
pub fn extract_run_params_from_json(body: &Value) -> ExtractedRunParams {
    let mut out = ExtractedRunParams::default();

    if body.get("resourceType").and_then(|v| v.as_str()) != Some("Parameters") {
        return out;
    }
    let Some(entries) = body.get("parameter").and_then(|p| p.as_array()) else {
        return out;
    };

    for p in entries {
        let Some(name) = parameter_name(p) else {
            continue;
        };
        match name.as_str() {
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
            "_limit" => {
                if out.limit.is_none() {
                    out.limit = p
                        .get("valueInteger")
                        .or_else(|| p.get("valuePositiveInt"))
                        .and_then(|v| v.as_u64());
                }
            }
            "_since" => {
                if out.since.is_none() {
                    out.since = read_str(p, &["valueInstant", "valueDateTime", "valueString"]);
                }
            }
            "patient" => {
                if let Some(s) = read_reference_or_string(p) {
                    out.patient.push(s);
                }
            }
            "group" => {
                if let Some(s) = read_reference_or_string(p) {
                    out.group.push(s);
                }
            }
            "subjectResource" => {
                if out.subject_resource.is_none() {
                    if let Some(r) = p.get("resource") {
                        out.subject_resource = Some(r.clone());
                    }
                }
            }
            "subjectReference" => {
                if out.subject_reference.is_none() {
                    out.subject_reference = read_reference_or_string(p);
                }
            }
            "subjectCanonical" => {
                if out.subject_canonical.is_none() {
                    out.subject_canonical = read_str(
                        p,
                        &["valueCanonical", "valueUri", "valueUrl", "valueString"],
                    );
                }
            }
            "resource" => {
                if let Some(r) = p.get("resource") {
                    // Spec (Resource Parameter and Bundle Inputs): a `Bundle`
                    // supplied as a `resource` value is unwrapped one level —
                    // the view runs against each `Bundle.entry[*].resource`,
                    // never against the `Bundle` itself. Mixing discrete
                    // resources and bundles is permitted; the effective input
                    // is their union.
                    if r.get("resourceType").and_then(|v| v.as_str()) == Some("Bundle") {
                        if let Some(entries) = r.get("entry").and_then(|e| e.as_array()) {
                            for entry in entries {
                                if let Some(res) = entry.get("resource") {
                                    out.inline_resources.push(res.clone());
                                }
                            }
                        }
                    } else {
                        out.inline_resources.push(r.clone());
                    }
                }
            }
            "source" => {
                if out.source.is_none() {
                    out.source = read_str(p, &["valueString", "valueUri"]);
                }
            }
            "maxFileSize" => {
                if out.max_file_size.is_none() {
                    out.max_file_size = read_u64(p, &["valueInteger", "valuePositiveInt"]);
                }
            }
            "rowGroupSize" => {
                if out.row_group_size.is_none() {
                    out.row_group_size = read_u64(p, &["valueInteger", "valuePositiveInt"]);
                }
            }
            "pageSize" => {
                if out.page_size.is_none() {
                    out.page_size = read_u64(p, &["valueInteger", "valuePositiveInt"]);
                }
            }
            "compression" => {
                if out.compression.is_none() {
                    out.compression = read_str(p, &["valueCode", "valueString"]);
                }
            }
            _ => {}
        }
    }
    out
}

/// Pulls a parameter's `name`. Accepts both raw-JSON shape (`"name": "..."`)
/// and FHIR-typed serde output (`"name": {"value": "..."}`).
fn parameter_name(p: &Value) -> Option<String> {
    let raw = p.get("name")?;
    if let Some(s) = raw.as_str() {
        return Some(s.to_string());
    }
    if let Some(v) = raw.get("value").and_then(|v| v.as_str()) {
        return Some(v.to_string());
    }
    None
}

fn read_str(p: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(s) = p.get(*key).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

fn read_u64(p: &Value, keys: &[&str]) -> Option<u64> {
    for key in keys {
        if let Some(n) = p.get(*key).and_then(|v| v.as_u64()) {
            return Some(n);
        }
    }
    None
}

/// Reads a `valueReference.reference` (preferred) or `valueString` (fallback).
fn read_reference_or_string(p: &Value) -> Option<String> {
    if let Some(s) = p
        .get("valueReference")
        .and_then(|r| r.get("reference"))
        .and_then(|v| v.as_str())
    {
        return Some(s.to_string());
    }
    p.get("valueString")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn params(parameter: Vec<Value>) -> Value {
        json!({"resourceType": "Parameters", "parameter": parameter})
    }

    #[test]
    fn bare_viewdefinition_detected() {
        assert!(body_has_subject(&json!({"resourceType": "ViewDefinition"})));
    }

    #[test]
    fn parameters_with_subject_resource_detected() {
        assert!(body_has_subject(&params(vec![json!({
            "name": "subjectResource",
            "resource": {"resourceType": "ViewDefinition"}
        })])));
    }

    #[test]
    fn parameters_with_subject_reference_detected() {
        assert!(body_has_subject(&params(vec![json!({
            "name": "subjectReference",
            "valueReference": {"reference": "ViewDefinition/x"}
        })])));
    }

    #[test]
    fn parameters_with_subject_canonical_detected() {
        assert!(body_has_subject(&params(vec![json!({
            "name": "subjectCanonical",
            "valueCanonical": "http://example.org/ViewDefinition/x"
        })])));
    }

    #[test]
    fn parameters_without_subject_returns_false() {
        assert!(!body_has_subject(&params(vec![json!({
            "name": "patient",
            "valueString": "Patient/123"
        })])));
    }

    #[test]
    fn pre_ballot_view_parameter_names_are_not_subjects() {
        // `viewResource` / `viewReference` existed only in the continuous
        // build and were never published. They name nothing in 3.0.0.
        assert!(!body_has_subject(&params(vec![json!({
            "name": "viewResource",
            "resource": {"resourceType": "ViewDefinition"}
        })])));
    }

    #[test]
    fn empty_for_non_parameters_body() {
        let p = extract_run_params_from_json(&json!({"resourceType": "ViewDefinition"}));
        assert!(p.patient.is_empty());
        assert!(p.format.is_none());
    }

    #[test]
    fn format_accepts_code_and_string() {
        let p = extract_run_params_from_json(&params(vec![
            json!({"name": "_format", "valueCode": "csv"}),
        ]));
        assert_eq!(p.format.as_deref(), Some("csv"));
        let p = extract_run_params_from_json(&params(vec![
            json!({"name": "_format", "valueString": "csv"}),
        ]));
        assert_eq!(p.format.as_deref(), Some("csv"));
    }

    #[test]
    fn header_boolean_or_string() {
        let p = extract_run_params_from_json(&params(vec![
            json!({"name": "header", "valueBoolean": true}),
        ]));
        assert_eq!(p.header, Some(true));
        let p = extract_run_params_from_json(&params(vec![
            json!({"name": "header", "valueString": "false"}),
        ]));
        assert_eq!(p.header, Some(false));
    }

    #[test]
    fn limit_integer_or_positive_int() {
        let p = extract_run_params_from_json(&params(vec![
            json!({"name": "_limit", "valueInteger": 100}),
        ]));
        assert_eq!(p.limit, Some(100));
        let p = extract_run_params_from_json(&params(vec![
            json!({"name": "_limit", "valuePositiveInt": 50}),
        ]));
        assert_eq!(p.limit, Some(50));
    }

    #[test]
    fn since_accepts_three_shapes() {
        for key in ["valueInstant", "valueDateTime", "valueString"] {
            let p = extract_run_params_from_json(&params(vec![
                json!({"name": "_since", key: "2024-01-02T03:04:05Z"}),
            ]));
            assert_eq!(p.since.as_deref(), Some("2024-01-02T03:04:05Z"));
        }
    }

    #[test]
    fn patient_repeated_entries_accumulate() {
        let p = extract_run_params_from_json(&params(vec![
            json!({"name": "patient", "valueReference": {"reference": "Patient/1"}}),
            json!({"name": "patient", "valueString": "Patient/2"}),
        ]));
        assert_eq!(
            p.patient,
            vec!["Patient/1".to_string(), "Patient/2".to_string()]
        );
    }

    #[test]
    fn group_repeated_entries_accumulate() {
        let p = extract_run_params_from_json(&params(vec![
            json!({"name": "group", "valueReference": {"reference": "Group/1"}}),
            json!({"name": "group", "valueString": "Group/2"}),
        ]));
        assert_eq!(p.group, vec!["Group/1".to_string(), "Group/2".to_string()]);
    }

    #[test]
    fn inline_resources_accumulate() {
        let p = extract_run_params_from_json(&params(vec![
            json!({"name": "resource", "resource": {"resourceType": "Patient", "id": "1"}}),
            json!({"name": "resource", "resource": {"resourceType": "Patient", "id": "2"}}),
        ]));
        assert_eq!(p.inline_resources.len(), 2);
    }

    #[test]
    fn subject_resource_extracted() {
        let p = extract_run_params_from_json(&params(vec![json!({
            "name": "subjectResource",
            "resource": {"resourceType": "ViewDefinition"}
        })]));
        assert!(p.subject_resource.is_some());
    }

    #[test]
    fn subject_reference_string_or_reference() {
        let p = extract_run_params_from_json(&params(vec![json!({
            "name": "subjectReference",
            "valueReference": {"reference": "ViewDefinition/x"}
        })]));
        assert_eq!(p.subject_reference.as_deref(), Some("ViewDefinition/x"));
        let p = extract_run_params_from_json(&params(vec![json!({
            "name": "subjectReference",
            "valueString": "ViewDefinition/y"
        })]));
        assert_eq!(p.subject_reference.as_deref(), Some("ViewDefinition/y"));
    }

    #[test]
    fn subject_canonical_accepts_canonical_uri_and_string() {
        for key in ["valueCanonical", "valueUri", "valueUrl", "valueString"] {
            let p = extract_run_params_from_json(&params(vec![json!({
                "name": "subjectCanonical",
                key: "http://example.org/ViewDefinition/x|1.0.0"
            })]));
            assert_eq!(
                p.subject_canonical.as_deref(),
                Some("http://example.org/ViewDefinition/x|1.0.0"),
                "key {key}"
            );
        }
    }

    #[test]
    fn subject_canonical_and_reference_are_independent() {
        // A canonical URL is an identity; a literal reference is a location.
        // Supplying both is rejected by the handler, but the extractor keeps
        // them apart so it can tell that both were present.
        let p = extract_run_params_from_json(&params(vec![
            json!({"name": "subjectCanonical", "valueCanonical": "http://example.org/vd"}),
            json!({"name": "subjectReference", "valueString": "ViewDefinition/x"}),
        ]));
        assert_eq!(
            p.subject_canonical.as_deref(),
            Some("http://example.org/vd")
        );
        assert_eq!(p.subject_reference.as_deref(), Some("ViewDefinition/x"));
    }

    #[test]
    fn typed_serde_name_shape_accepted() {
        // Mirrors how the FHIR typed `RunParameters` serialises (`name.value`).
        let p = extract_run_params_from_json(&params(vec![json!({
            "name": {"value": "_format"},
            "valueCode": "json"
        })]));
        assert_eq!(p.format.as_deref(), Some("json"));
    }

    #[test]
    fn parquet_options_extracted() {
        let p = extract_run_params_from_json(&params(vec![
            json!({"name": "maxFileSize", "valueInteger": 500}),
            json!({"name": "rowGroupSize", "valueInteger": 128}),
            json!({"name": "pageSize", "valueInteger": 1024}),
            json!({"name": "compression", "valueCode": "snappy"}),
        ]));
        assert_eq!(p.max_file_size, Some(500));
        assert_eq!(p.row_group_size, Some(128));
        assert_eq!(p.page_size, Some(1024));
        assert_eq!(p.compression.as_deref(), Some("snappy"));
    }

    #[test]
    fn split_csv_refs_trims_and_drops_empty() {
        assert_eq!(split_csv_refs(None), Vec::<String>::new());
        assert_eq!(split_csv_refs(Some("")), Vec::<String>::new());
        assert_eq!(
            split_csv_refs(Some("Group/a, Group/b ,,Group/c")),
            vec![
                "Group/a".to_string(),
                "Group/b".to_string(),
                "Group/c".to_string()
            ]
        );
    }

    #[test]
    fn unknown_param_names_ignored() {
        let p = extract_run_params_from_json(&params(vec![
            json!({"name": "_format", "valueCode": "json"}),
            json!({"name": "unknownParam", "valueString": "ignored"}),
        ]));
        assert_eq!(p.format.as_deref(), Some("json"));
    }
}
