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
//! [`extract_run_params_from_json`] is **permissive**: missing / wrong-typed
//! `value[X]` fields produce `None`/empty rather than an error. HFS uses
//! [`extract_run_params_checked`] to reject unusable patient/group filters.
//! The standalone sof-server shares that reference rule and also checks
//! bounds and other value shapes (e.g. `_limit`, `compression`).

use serde_json::Value;

/// An unusable `patient` or `group` operation parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceParameterError {
    /// The operation parameter whose value cannot be used.
    pub param: String,
    /// The client-facing explanation of the invalid value.
    pub message: String,
}

impl std::fmt::Display for ReferenceParameterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ReferenceParameterError {}

/// Reads a usable `patient`/`group` reference from one parameter entry.
///
/// A string `valueReference.reference` takes precedence over `valueString`,
/// even when it is blank. The selected string is trimmed and must be non-empty.
/// Identifier-only and display-only references cannot restrict a run and are
/// rejected instead of silently dropping the filter. Reference shape and
/// target existence validation are outside this helper.
pub fn read_patient_group_reference(
    entry: &Value,
    name: &str,
) -> Result<String, ReferenceParameterError> {
    entry
        .get("valueReference")
        .and_then(|r| r.get("reference"))
        .and_then(Value::as_str)
        .or_else(|| entry.get("valueString").and_then(Value::as_str))
        .map(str::trim)
        .filter(|reference| !reference.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            let ty = if name == "group" { "Group" } else { "Patient" };
            ReferenceParameterError {
                param: name.to_string(),
                message: format!(
                    "a `{name}` entry carries no usable reference; send \
                     valueReference.reference (or valueString) as a relative `{ty}/{{id}}` reference"
                ),
            }
        })
}

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

/// Extracts run parameters, refusing any unusable `patient`/`group` entry.
///
/// All other fields keep the permissive behavior of
/// [`extract_run_params_from_json`]. The reference lists contain every entry
/// in request order, normalized by [`read_patient_group_reference`]. Validate
/// the body before applying query fallbacks so an invalid body filter cannot
/// disappear behind another valid reference.
pub fn extract_run_params_checked(
    body: &Value,
) -> Result<ExtractedRunParams, ReferenceParameterError> {
    let mut out = extract_run_params_from_json(body);
    if body.get("resourceType").and_then(Value::as_str) != Some("Parameters") {
        return Ok(out);
    }
    let Some(entries) = body.get("parameter").and_then(Value::as_array) else {
        return Ok(out);
    };

    out.patient.clear();
    out.group.clear();
    for entry in entries {
        match parameter_name(entry).as_deref() {
            Some("patient") => out
                .patient
                .push(read_patient_group_reference(entry, "patient")?),
            Some("group") => out
                .group
                .push(read_patient_group_reference(entry, "group")?),
            _ => {}
        }
    }
    Ok(out)
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
    fn checked_reference_parameters_reject_unusable_values() {
        let invalid_values = [
            json!({"valueReference": {"identifier": {"system": "urn:s", "value": "v"}}}),
            json!({"valueReference": {"display": "a patient"}}),
            json!({"valueReference": {}}),
            json!({"valueReference": {"reference": ""}}),
            json!({"valueReference": {"reference": " \t "}}),
            json!({"valueUri": "Patient/p1"}),
            json!({"valueIdentifier": {"system": "urn:s", "value": "v"}}),
            json!({"valueInteger": 42}),
            json!({"valueString": 42}),
            json!({"valueReference": {"reference": 42}}),
            json!({"valueString": ""}),
            json!({"valueString": " \t "}),
            json!({}),
            // A preferred string remains authoritative even when it is blank.
            json!({"valueReference": {"reference": " "}, "valueString": "Patient/p1"}),
        ];
        for name in ["patient", "group"] {
            for invalid in &invalid_values {
                for typed_name in [false, true] {
                    let mut entry = invalid.clone();
                    entry["name"] = if typed_name {
                        json!({"value": name})
                    } else {
                        json!(name)
                    };
                    let body = params(vec![
                        json!({"name": name, "valueString": "valid-id"}),
                        entry.clone(),
                    ]);
                    let error = extract_run_params_checked(&body).unwrap_err();
                    assert_eq!(error.param, name, "{entry}");
                    assert!(error.message.contains(name), "{error}");
                    assert!(error.message.contains("no usable reference"), "{error}");
                }
            }
        }
    }

    #[test]
    fn checked_reference_parameters_normalize_all_entries_and_keep_fallback() {
        let body = params(vec![
            json!({"name": "patient", "valueReference": {"reference": " Patient/p1 "}, "valueString": "Patient/ignored"}),
            json!({"name": {"value": "patient"}, "valueString": " p2 "}),
            json!({"name": "patient", "valueReference": {"display": "p3"}, "valueString": " Patient/p3 "}),
            json!({"name": "group", "valueReference": {"reference": " Group/g1 "}}),
            json!({"name": {"value": "group"}, "valueString": " Group/g2 "}),
            // Unrelated fields retain the permissive extraction contract.
            json!({"name": "_format", "valueCode": "json"}),
            json!({"name": "subjectReference", "valueReference": {"identifier": {"value": "v"}}}),
            json!({"name": "unknown", "valueInteger": 1}),
        ]);
        let extracted = extract_run_params_checked(&body).unwrap();
        assert_eq!(extracted.patient, ["Patient/p1", "p2", "Patient/p3"]);
        assert_eq!(extracted.group, ["Group/g1", "Group/g2"]);
        assert_eq!(extracted.format.as_deref(), Some("json"));
        assert!(extracted.subject_reference.is_none());
        assert!(
            extract_run_params_checked(&json!({"resourceType": "ViewDefinition"}))
                .unwrap()
                .patient
                .is_empty()
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
