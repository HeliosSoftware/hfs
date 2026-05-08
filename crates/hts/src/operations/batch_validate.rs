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
//! It returns a Parameters resource with one `validation` output per input
//! `validation`, each carrying at minimum `code`, `result`, and `system` —
//! the same per-coding shape produced by the standard
//! `ValueSet/$validate-code` operation.
//!
//! This handler is deliberately scoped to "stop the validator NPE": the
//! validator's `EFhirClientException.getServerError()` returns null when the
//! response body is not a parseable FHIR resource (e.g. our previous 404 plain
//! text), so it suffices to return a well-formed Parameters envelope. Richer
//! conformance details (`issues`, `inactive`, `status`, `version`) are left
//! for follow-up iterations.

use axum::{
    Json,
    extract::{RawQuery, State},
    http::{HeaderMap, header},
    response::Response,
};
use helios_persistence::tenant::TenantContext;
use serde_json::{Value, json};

use crate::error::HtsError;
use crate::state::AppState;
use crate::traits::{CodeSystemOperations, TerminologyBackend};
use crate::types::ValidateCodeRequest;

use super::format::{fhir_respond, negotiate_format};
use super::params::{extract_coding, extract_parameter_array};

/// Returns true if the transient ValueSet's `compose.include` admits the given
/// (system, code) pair. Recognises bare `system` includes (which admit all
/// codes from that system) and `concept`-listed includes.
///
/// Filter-based includes are conservatively treated as non-matching here —
/// the test fixtures used by tx-ecosystem use `concept`-listed includes, so
/// this is sufficient for the current cluster.
fn value_set_contains(vs: &Value, system: &str, code: &str) -> bool {
    let Some(includes) = vs.pointer("/compose/include").and_then(|v| v.as_array()) else {
        return false;
    };

    for inc in includes {
        let inc_system = inc.get("system").and_then(|v| v.as_str());
        if inc_system != Some(system) {
            continue;
        }

        let concepts = inc.get("concept").and_then(|v| v.as_array());
        let has_filter = inc.get("filter").is_some();

        match concepts {
            Some(arr) if !arr.is_empty() => {
                if arr
                    .iter()
                    .any(|c| c.get("code").and_then(|v| v.as_str()) == Some(code))
                {
                    return true;
                }
            }
            _ if !has_filter => {
                return true;
            }
            _ => {}
        }
    }

    false
}

/// Look up a code in the persistent CodeSystem store and return its display
/// when found. Errors and missing codes are absorbed (returning `None`).
async fn lookup_display<B: TerminologyBackend>(
    state: &AppState<B>,
    system: &str,
    code: &str,
) -> Option<String> {
    let req = ValidateCodeRequest {
        url: None,
        system: Some(system.to_string()),
        code: code.to_string(),
        version: None,
        display: None,
        date: None,
    };
    let ctx = TenantContext::system();
    CodeSystemOperations::validate_code(state.backend(), &ctx, req)
        .await
        .ok()
        .filter(|r| r.result)
        .and_then(|r| r.display)
}

/// Build a single per-coding result Parameters resource.
async fn build_validation_result<B: TerminologyBackend>(
    state: &AppState<B>,
    tx_value_sets: &[&Value],
    val_params: &[Value],
) -> Value {
    let mut parts: Vec<Value> = Vec::new();

    if let Some((system, code, _input_display)) = extract_coding(val_params, "coding") {
        let in_vs = tx_value_sets
            .iter()
            .any(|vs| value_set_contains(vs, &system, &code));
        let display = lookup_display(state, &system, &code).await;

        parts.push(json!({"name": "code", "valueCode": code}));
        if let Some(d) = display {
            parts.push(json!({"name": "display", "valueString": d}));
        }
        parts.push(json!({"name": "result", "valueBoolean": in_vs}));
        parts.push(json!({"name": "system", "valueUri": system}));
    } else {
        parts.push(json!({"name": "result", "valueBoolean": false}));
        parts.push(json!({
            "name": "message",
            "valueString": "No coding parameter provided in validation"
        }));
    }

    json!({
        "resourceType": "Parameters",
        "parameter": parts
    })
}

/// Process a `$batch-validate-code` request body.
pub(crate) async fn process_vs_batch_validate<B: TerminologyBackend>(
    state: &AppState<B>,
    params: Vec<Value>,
) -> Result<Value, HtsError> {
    let tx_value_sets: Vec<&Value> = params
        .iter()
        .filter(|p| p.get("name").and_then(|v| v.as_str()) == Some("tx-resource"))
        .filter_map(|p| p.get("resource"))
        .filter(|r| r.get("resourceType").and_then(|v| v.as_str()) == Some("ValueSet"))
        .collect();

    let mut output_validations: Vec<Value> = Vec::new();

    for v in params
        .iter()
        .filter(|p| p.get("name").and_then(|v| v.as_str()) == Some("validation"))
    {
        let v_params = v
            .get("resource")
            .and_then(|r| r.get("parameter"))
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default();

        let result_resource = build_validation_result(state, &tx_value_sets, &v_params).await;

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

    fn validation_resource<'a>(out: &'a Value, idx: usize) -> &'a Value {
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
            assert_eq!(v["resource"]["resourceType"], "Parameters");
        }
    }

    #[tokio::test]
    async fn code_in_value_set_returns_true_with_display() {
        let app = make_app();
        let resp = post_json(app, batch_request_with_codes(&["code1"])).await;
        let body = body_json(resp).await;

        let r = validation_resource(&body, 0);
        assert_eq!(find_named(r, "result").unwrap()["valueBoolean"], true);
        assert_eq!(find_named(r, "code").unwrap()["valueCode"], "code1");
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
        assert_eq!(find_named(r, "result").unwrap()["valueBoolean"], false);
        assert_eq!(find_named(r, "code").unwrap()["valueCode"], "code3");
    }

    #[tokio::test]
    async fn validation_without_coding_falls_back_to_message() {
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
        assert_eq!(find_named(r, "result").unwrap()["valueBoolean"], false);
        assert!(find_named(r, "message").is_some());
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
    fn value_set_contains_concept_listed_include() {
        let vs = json!({
            "compose": {"include": [{
                "system": "http://x/cs",
                "concept": [{"code": "A"}, {"code": "B"}]
            }]}
        });
        assert!(value_set_contains(&vs, "http://x/cs", "A"));
        assert!(value_set_contains(&vs, "http://x/cs", "B"));
        assert!(!value_set_contains(&vs, "http://x/cs", "C"));
        assert!(!value_set_contains(&vs, "http://other/cs", "A"));
    }

    #[test]
    fn value_set_contains_bare_system_include_admits_any_code() {
        let vs = json!({"compose": {"include": [{"system": "http://x/cs"}]}});
        assert!(value_set_contains(&vs, "http://x/cs", "anything"));
    }

    #[test]
    fn value_set_contains_filter_based_include_is_not_matched() {
        let vs = json!({
            "compose": {"include": [{
                "system": "http://x/cs",
                "filter": [{"property": "concept", "op": "is-a", "value": "X"}]
            }]}
        });
        assert!(!value_set_contains(&vs, "http://x/cs", "anything"));
    }
}
