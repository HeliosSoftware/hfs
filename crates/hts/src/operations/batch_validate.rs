//! Handler for `POST /ValueSet/$batch-validate-code`.
//!
//! The `$batch-validate-code` operation, used by the HL7 FHIR Tx Ecosystem
//! conformance suite, accepts an input Parameters resource that bundles:
//!
//! * one or more `tx-resource` parameters carrying transient ValueSet
//!   resources (or other terminology resources) to validate against,
//! * a `url` parameter naming the principal ValueSet,
//! * one or more `validation` parameters, each whose value is a Parameters
//!   resource describing a single coding to validate.
//!
//! The response is a Parameters resource with one output entry per input
//! `validation` parameter, in the same order. Each output entry is either:
//!
//! * a Parameters resource carrying the per-coding `$validate-code` result —
//!   identical in shape to what `POST /ValueSet/$validate-code` produces (so
//!   `code`, `display`, `inactive`, `issues`, `message`, `result`, `status`,
//!   `system`, `version` are all surfaced when applicable, in alphabetical
//!   order); or
//! * an OperationOutcome carrying an `invalid` issue when the input
//!   validation Parameters did not name a coding/code/codeableConcept.
//!
//! Implementation strategy: this handler does NOT reimplement validation. It
//! merges batch-level inheritable parameters (`lenient-display-validation`,
//! `displayLanguage`, `valueSetVersion`, `default-valueset-version`,
//! `useSupplement`) with each per-validation Parameters and delegates to
//! `process_vs_validate_code` (the same code path used by
//! `POST /ValueSet/$validate-code`). The principal `tx-resource` ValueSet is
//! injected as `valueSet` so the inline-validation path is taken — that way
//! the principal VS does not need to be persistently stored.

use axum::{
    Json,
    extract::{RawQuery, State},
    http::{HeaderMap, header},
    response::Response,
};
use serde_json::{Value, json};

use crate::error::HtsError;
use crate::state::AppState;
use crate::traits::TerminologyBackend;

use super::format::{fhir_respond, negotiate_format};
use super::params::{
    extract_codeable_concept, extract_coding_full, extract_parameter_array, find_str_param,
};
use super::validate_code::process_vs_validate_code;

/// Per-validation Parameters keys that, when present at the batch level, are
/// forwarded into each delegated `$validate-code` call. Per-validation params
/// take precedence (the per-validation entry is forwarded first; `find_str_param`
/// returns the first match).
///
/// `useSupplement` is a repeated parameter — handled specially below so all
/// occurrences propagate, not just the first.
const BATCH_INHERITED_KEYS: &[&str] = &[
    "lenient-display-validation",
    "displayLanguage",
    "valueSetVersion",
    "default-valueset-version",
    "abstract",
    "system-version",
    "force-system-version",
    "check-system-version",
    "date",
];

/// IG-pinned text for the "no coding/code/codeableConcept supplied" error.
/// Matches the reference tx.fhir.org behaviour and the IG
/// `batch/batch-validate-bad-response-bundle.json` fixture (which deliberately
/// includes a validation entry with the parameter name `codingX` to exercise
/// this path).
const NO_CODE_TO_VALIDATE_TEXT: &str = "Unable to find code to validate \
     (looked for coding | codeableConcept | code+system | code+inferSystem in parameters";

/// Pick the principal `tx-resource` ValueSet to validate against.
///
/// Strategy:
///  1. If the batch supplies a top-level `url` parameter, return the
///     tx-resource VS whose `url` matches it.
///  2. Otherwise, return the first ValueSet tx-resource.
fn principal_value_set(params: &[Value]) -> Option<Value> {
    let target_url = find_str_param(params, "url");
    let mut first_vs: Option<Value> = None;

    for p in params
        .iter()
        .filter(|p| p.get("name").and_then(|v| v.as_str()) == Some("tx-resource"))
    {
        let Some(res) = p.get("resource") else {
            continue;
        };
        if res.get("resourceType").and_then(|v| v.as_str()) != Some("ValueSet") {
            continue;
        }
        if first_vs.is_none() {
            first_vs = Some(res.clone());
        }
        if let Some(ref u) = target_url {
            if res.get("url").and_then(|v| v.as_str()) == Some(u.as_str()) {
                return Some(res.clone());
            }
        }
    }

    first_vs
}

/// Returns true if the per-validation Parameters carries a usable code input —
/// `coding`, `codeableConcept`, or bare `code` (with or without `system`).
/// Mirrors the discovery rules in `process_inline_vs_validate_code`.
fn has_validatable_input(v_params: &[Value]) -> bool {
    if extract_coding_full(v_params, "coding").is_some() {
        return true;
    }
    if extract_codeable_concept(v_params, "codeableConcept").is_some() {
        return true;
    }
    if find_str_param(v_params, "code").is_some() {
        return true;
    }
    false
}

/// Build the IG-pinned OperationOutcome for the "no validatable input" path.
fn no_code_to_validate_outcome() -> Value {
    json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": "error",
            "code": "invalid",
            "details": { "text": NO_CODE_TO_VALIDATE_TEXT }
        }]
    })
}

/// Convert an `HtsError` into an OperationOutcome resource for embedding in a
/// per-validation slot. Keeps the diagnostic text in the `details.text` field
/// (where the IG fixtures compare it).
fn err_to_outcome(err: &HtsError) -> Value {
    let (severity, code, text) = match err {
        HtsError::InvalidRequest(m) => ("error", "invalid", m.clone()),
        HtsError::NotFound(m) => ("error", "not-found", m.clone()),
        HtsError::VsInvalid(m) => ("error", "invalid", m.clone()),
        other => ("error", "exception", other.to_string()),
    };
    json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": severity,
            "code": code,
            "details": { "text": text }
        }]
    })
}

/// Forward batch-level inheritable parameters into a per-validation params
/// vector, then append the per-validation params themselves and finally the
/// `valueSet` (inline) reference. Per-validation params come FIRST so they
/// override batch-level defaults via `find_str_param`'s first-match semantics.
fn build_delegated_params(
    batch_params: &[Value],
    v_params: &[Value],
    principal_vs: Option<&Value>,
) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::with_capacity(v_params.len() + BATCH_INHERITED_KEYS.len() + 1);

    // Per-validation first — overrides batch defaults.
    for p in v_params {
        out.push(p.clone());
    }

    // Batch-level inheritable single-valued params.
    for key in BATCH_INHERITED_KEYS {
        for p in batch_params {
            if p.get("name").and_then(|v| v.as_str()) == Some(*key) {
                out.push(p.clone());
            }
        }
    }

    // Repeated param `useSupplement` — propagate every occurrence.
    for p in batch_params {
        if p.get("name").and_then(|v| v.as_str()) == Some("useSupplement") {
            out.push(p.clone());
        }
    }

    // Principal VS as inline `valueSet` so the inline path is taken in
    // process_vs_validate_code (avoids needing the principal VS to be
    // persistently stored).
    if let Some(vs) = principal_vs {
        out.push(json!({ "name": "valueSet", "resource": vs }));
    }

    out
}

/// Process a `$batch-validate-code` request body.
pub(crate) async fn process_vs_batch_validate<B: TerminologyBackend>(
    state: &AppState<B>,
    params: Vec<Value>,
) -> Result<Value, HtsError> {
    let principal_vs = principal_value_set(&params);

    let mut output_validations: Vec<Value> = Vec::new();

    for v in params
        .iter()
        .filter(|p| p.get("name").and_then(|v| v.as_str()) == Some("validation"))
    {
        let v_params: Vec<Value> = v
            .get("resource")
            .and_then(|r| r.get("parameter"))
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default();

        // Pre-detect "no usable code input" — emit the IG-pinned
        // OperationOutcome rather than delegating (the delegated path's
        // error text differs from the IG fixture).
        if !has_validatable_input(&v_params) {
            output_validations.push(json!({
                "name": "validation",
                "resource": no_code_to_validate_outcome()
            }));
            continue;
        }

        let delegated_params = build_delegated_params(&params, &v_params, principal_vs.as_ref());

        let result_resource = match process_vs_validate_code(state, delegated_params).await {
            Ok(value) => value,
            Err(err) => err_to_outcome(&err),
        };

        output_validations.push(json!({
            "name": "validation",
            "resource": result_resource
        }));
    }

    Ok(json!({
        "resourceType": "Parameters",
        "parameter": output_validations
    }))
}

/// `POST /ValueSet/$batch-validate-code`
pub async fn vs_batch_validate_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let params = extract_parameter_array(&body)?;
    Ok(fhir_respond(
        process_vs_batch_validate(&state, params).await?,
        format,
    ))
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::Request, routing::post};
    use tower::ServiceExt;

    use crate::backends::sqlite::SqliteTerminologyBackend;
    use crate::state::AppState;

    fn make_app() -> Router {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        {
            let conn = backend.pool().get().unwrap();
            conn.execute_batch(
                "INSERT INTO code_systems
                     (id, url, version, name, status, content, created_at, updated_at)
                 VALUES ('cs-simple', 'http://hl7.org/fhir/test/CodeSystem/simple',
                         '0.1.0', 'SimpleCS', 'active', 'complete',
                         '2024-01-01', '2024-01-01');

                 INSERT INTO concepts (id, system_id, code, display)
                 VALUES (1, 'cs-simple', 'code1', 'Display 1'),
                        (2, 'cs-simple', 'code2', 'Display 2');",
            )
            .unwrap();
        }
        let state = AppState::new(backend);
        Router::new()
            .route(
                "/ValueSet/$batch-validate-code",
                post(vs_batch_validate_handler::<SqliteTerminologyBackend>),
            )
            .with_state(state)
    }

    async fn post_json(app: Router, body: Value) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/ValueSet/$batch-validate-code")
                .header("content-type", "application/fhir+json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn body_json(response: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn batch_request_with_codes(codes: &[&str]) -> Value {
        let mut params = vec![
            json!({
                "name": "tx-resource",
                "resource": {
                    "resourceType": "ValueSet",
                    "url": "urn:uuid:test-vs",
                    "status": "active",
                    "compose": {
                        "include": [{
                            "system": "http://hl7.org/fhir/test/CodeSystem/simple",
                            "concept": [
                                {"code": "code1"},
                                {"code": "code2"}
                            ]
                        }]
                    }
                }
            }),
            json!({"name": "url", "valueUri": "urn:uuid:test-vs"}),
        ];
        for c in codes {
            params.push(json!({
                "name": "validation",
                "resource": {
                    "resourceType": "Parameters",
                    "parameter": [{
                        "name": "coding",
                        "valueCoding": {
                            "system": "http://hl7.org/fhir/test/CodeSystem/simple",
                            "code": c
                        }
                    }]
                }
            }));
        }
        json!({"resourceType": "Parameters", "parameter": params})
    }

    fn validation_resource(out: &Value, idx: usize) -> &Value {
        &out["parameter"][idx]["resource"]
    }

    fn find_named<'a>(parts: &'a Value, name: &str) -> Option<&'a Value> {
        parts["parameter"]
            .as_array()?
            .iter()
            .find(|p| p["name"] == name)
    }

    #[tokio::test]
    async fn returns_parameters_envelope_with_validation_per_input() {
        let app = make_app();
        let resp = post_json(app, batch_request_with_codes(&["code1", "code2", "code3"])).await;
        assert_eq!(resp.status(), 200);

        let body = body_json(resp).await;
        assert_eq!(body["resourceType"], "Parameters");

        let outer = body["parameter"].as_array().unwrap();
        assert_eq!(outer.len(), 3, "one validation output per input");
        for v in outer {
            assert_eq!(v["name"], "validation");
        }
    }

    #[tokio::test]
    async fn code_in_value_set_returns_true_with_display() {
        let app = make_app();
        let resp = post_json(app, batch_request_with_codes(&["code1"])).await;
        let body = body_json(resp).await;

        let r = validation_resource(&body, 0);
        // Delegated path returns a Parameters resource.
        assert_eq!(r["resourceType"], "Parameters");
        assert_eq!(find_named(r, "result").unwrap()["valueBoolean"], true);
        assert_eq!(
            find_named(r, "display").unwrap()["valueString"],
            "Display 1"
        );
    }

    #[tokio::test]
    async fn code_not_in_value_set_returns_false() {
        let app = make_app();
        let resp = post_json(app, batch_request_with_codes(&["code3"])).await;
        let body = body_json(resp).await;

        let r = validation_resource(&body, 0);
        assert_eq!(r["resourceType"], "Parameters");
        assert_eq!(find_named(r, "result").unwrap()["valueBoolean"], false);
    }

    #[tokio::test]
    async fn validation_without_coding_returns_operation_outcome() {
        // Mirrors the IG `batch-validate-bad` fixture: a validation entry that
        // names `codingX` (typo) instead of `coding` must come back as an
        // OperationOutcome carrying the IG-pinned "Unable to find code …" text.
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "tx-resource",
                    "resource": {
                        "resourceType": "ValueSet",
                        "url": "urn:uuid:test-vs",
                        "status": "active",
                        "compose": {"include": [{
                            "system": "http://hl7.org/fhir/test/CodeSystem/simple",
                            "concept": [{"code": "code1"}]
                        }]}
                    }
                },
                {"name": "url", "valueUri": "urn:uuid:test-vs"},
                {
                    "name": "validation",
                    "resource": {
                        "resourceType": "Parameters",
                        "parameter": [{
                            "name": "codingX",
                            "valueCoding": {
                                "system": "http://hl7.org/fhir/test/CodeSystem/simple",
                                "code": "code2"
                            }
                        }]
                    }
                }
            ]
        });
        let resp = post_json(app, body).await;
        assert_eq!(resp.status(), 200);

        let body = body_json(resp).await;
        let r = validation_resource(&body, 0);
        assert_eq!(r["resourceType"], "OperationOutcome");
        let issue = &r["issue"][0];
        assert_eq!(issue["severity"], "error");
        assert_eq!(issue["code"], "invalid");
        let text = issue["details"]["text"].as_str().unwrap_or("");
        assert!(
            text.contains("Unable to find code to validate"),
            "expected IG 'Unable to find code…' text, got: {text}"
        );
    }

    #[tokio::test]
    async fn empty_request_returns_empty_parameters() {
        let app = make_app();
        let body = json!({"resourceType": "Parameters", "parameter": []});
        let resp = post_json(app, body).await;
        assert_eq!(resp.status(), 200);

        let body = body_json(resp).await;
        assert_eq!(body["resourceType"], "Parameters");
        assert_eq!(body["parameter"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn non_parameters_body_returns_400() {
        let app = make_app();
        let body = json!({"resourceType": "ValueSet"});
        let resp = post_json(app, body).await;
        assert_eq!(resp.status(), 400);
    }

    #[test]
    fn principal_value_set_matches_url_when_supplied() {
        let params = vec![
            json!({"name": "tx-resource", "resource": {
                "resourceType": "ValueSet", "url": "urn:other"
            }}),
            json!({"name": "tx-resource", "resource": {
                "resourceType": "ValueSet", "url": "urn:wanted"
            }}),
            json!({"name": "url", "valueUri": "urn:wanted"}),
        ];
        let chosen = principal_value_set(&params).unwrap();
        assert_eq!(chosen["url"], "urn:wanted");
    }

    #[test]
    fn principal_value_set_falls_back_to_first_vs_when_no_url_match() {
        let params = vec![json!({"name": "tx-resource", "resource": {
            "resourceType": "ValueSet", "url": "urn:only"
        }})];
        let chosen = principal_value_set(&params).unwrap();
        assert_eq!(chosen["url"], "urn:only");
    }

    #[test]
    fn principal_value_set_skips_non_value_set_tx_resources() {
        let params = vec![
            json!({"name": "tx-resource", "resource": {
                "resourceType": "CodeSystem", "url": "urn:cs"
            }}),
            json!({"name": "tx-resource", "resource": {
                "resourceType": "ValueSet", "url": "urn:vs"
            }}),
        ];
        let chosen = principal_value_set(&params).unwrap();
        assert_eq!(chosen["url"], "urn:vs");
    }

    #[test]
    fn has_validatable_input_finds_coding() {
        let params = vec![json!({
            "name": "coding",
            "valueCoding": {"system": "http://x", "code": "A"}
        })];
        assert!(has_validatable_input(&params));
    }

    #[test]
    fn has_validatable_input_finds_bare_code() {
        let params = vec![json!({"name": "code", "valueCode": "A"})];
        assert!(has_validatable_input(&params));
    }

    #[test]
    fn has_validatable_input_finds_codeable_concept() {
        let params = vec![json!({
            "name": "codeableConcept",
            "valueCodeableConcept": {"coding": [{"system": "http://x", "code": "A"}]}
        })];
        assert!(has_validatable_input(&params));
    }

    #[test]
    fn has_validatable_input_rejects_typo_param_name() {
        let params = vec![json!({
            "name": "codingX",
            "valueCoding": {"system": "http://x", "code": "A"}
        })];
        assert!(!has_validatable_input(&params));
    }

    #[test]
    fn build_delegated_params_per_validation_overrides_batch() {
        // Batch lenient='true', per-validation lenient='false' — per-validation
        // appears first in the merged list so find_str_param picks it up.
        let batch = vec![
            json!({"name": "lenient-display-validation", "valueBoolean": "true"}),
            json!({"name": "useSupplement", "valueCanonical": "http://s|1"}),
        ];
        let v = vec![
            json!({"name": "coding", "valueCoding": {"system": "http://x", "code": "A"}}),
            json!({"name": "lenient-display-validation", "valueBoolean": "false"}),
        ];
        let merged = build_delegated_params(&batch, &v, None);
        // Per-validation params copied first.
        assert_eq!(merged[0]["name"], "coding");
        assert_eq!(merged[1]["name"], "lenient-display-validation");
        assert_eq!(merged[1]["valueBoolean"], "false");
        // Batch lenient appended after per-validation.
        let lenient_indices: Vec<usize> = merged
            .iter()
            .enumerate()
            .filter(|(_, p)| p["name"] == "lenient-display-validation")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(lenient_indices.len(), 2, "both copies present");
        assert!(lenient_indices[0] < lenient_indices[1], "per-val first");
        // useSupplement propagated.
        assert!(merged.iter().any(|p| p["name"] == "useSupplement"));
    }

    #[test]
    fn build_delegated_params_includes_principal_value_set() {
        let vs = json!({"resourceType": "ValueSet", "url": "urn:vs"});
        let merged = build_delegated_params(&[], &[], Some(&vs));
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0]["name"], "valueSet");
        assert_eq!(merged[0]["resource"]["url"], "urn:vs");
    }
}
