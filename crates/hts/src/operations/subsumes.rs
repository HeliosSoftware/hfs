//! `$subsumes` operation on `CodeSystem`.
//!
//! Accepts a FHIR Parameters resource (POST) or query string (GET) containing
//! either `system` + `codeA` + `codeB` + optional `version`, or two
//! `valueCoding` parameters (`codingA`, `codingB`).  Returns a Parameters
//! resource whose single `outcome` value is one of:
//!
//! - `"equivalent"` — the two codes are identical
//! - `"subsumes"` — codeA is an ancestor of codeB
//! - `"subsumed-by"` — codeA is a descendant of codeB
//! - `"not-subsumed"` — no hierarchical relationship exists
//!
//! Spec: <https://hl7.org/fhir/codesystem-operation-subsumes.html>

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
use crate::types::{SubsumesRequest, SubsumptionOutcome};

use super::format::{fhir_respond, negotiate_format};
use super::params::{
    extract_coding, extract_parameter_array, find_str_param, parse_query_string,
    query_params_to_fhir_params,
};

/// Core subsumption logic shared by the POST and GET handlers.
///
/// Accepts two input forms (detected by inspecting `params`):
///
/// - **Bare code form** — `system` + `codeA` + `codeB` as separate parameters.
/// - **Coding form** — `codingA` and `codingB` as `valueCoding` objects.
///   Both codings must reference the same code system; an HTTP 400 is returned
///   if they differ.
///
/// ## Returns
///
/// A FHIR `Parameters` resource with a single `outcome` parameter whose
/// `valueCode` is one of `"equivalent"`, `"subsumes"`, `"subsumed-by"`, or
/// `"not-subsumed"`.
///
/// ## Errors
///
/// | Condition | Error |
/// |-----------|-------|
/// | Missing `system` (bare form) | [`HtsError::InvalidRequest`] |
/// | Cross-system codings | [`HtsError::InvalidRequest`] |
/// | Neither form detected | [`HtsError::InvalidRequest`] |
/// | Unknown code system | [`HtsError::NotFound`] |
async fn process_subsumes<B: TerminologyBackend>(
    state: &AppState<B>,
    params: Vec<Value>,
) -> Result<Value, HtsError> {
    // ── Resolve (system, codeA, codeB) from one of two input forms ───────────
    //
    // Form 1: bare `codeA` + `codeB` + shared `system` parameter.
    // Form 2: `codingA` (valueCoding) + `codingB` (valueCoding) — system is
    //         embedded in each Coding and they MUST be the same code system.
    let (system, code_a, code_b) = match (
        find_str_param(&params, "codeA"),
        find_str_param(&params, "codeB"),
    ) {
        (Some(code_a), Some(code_b)) => {
            // Bare code form — explicit `system` parameter is required.
            let system = find_str_param(&params, "system").ok_or_else(|| {
                HtsError::InvalidRequest("Missing required parameter: system".into())
            })?;
            (system, code_a, code_b)
        }
        _ => {
            // Coding form — look for codingA and codingB valueCoding objects.
            match (
                extract_coding(&params, "codingA"),
                extract_coding(&params, "codingB"),
            ) {
                (Some((sys_a, code_a, _)), Some((sys_b, code_b, _))) => {
                    // FHIR spec: both codings MUST belong to the same code system.
                    if sys_a != sys_b {
                        return Err(HtsError::InvalidRequest(format!(
                            "codingA.system ({sys_a}) and codingB.system ({sys_b}) \
                             must be the same code system for $subsumes"
                        )));
                    }
                    (sys_a, code_a, code_b)
                }
                _ => {
                    return Err(HtsError::InvalidRequest(
                        "Must provide either (system + codeA + codeB) \
                         or (codingA + codingB) parameters"
                            .into(),
                    ));
                }
            }
        }
    };

    let req = SubsumesRequest {
        system,
        version: find_str_param(&params, "version"),
        code_a,
        code_b,
    };

    let ctx = TenantContext::system();
    let resp = CodeSystemOperations::subsumes(state.backend(), &ctx, req).await?;

    let outcome_str = match resp.outcome {
        SubsumptionOutcome::Equivalent => "equivalent",
        SubsumptionOutcome::Subsumes => "subsumes",
        SubsumptionOutcome::SubsumedBy => "subsumed-by",
        SubsumptionOutcome::NotSubsumed => "not-subsumed",
    };

    Ok(json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "outcome", "valueCode": outcome_str}
        ]
    }))
}

/// POST /CodeSystem/$subsumes
pub async fn subsumes_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let params = extract_parameter_array(&body)?;
    Ok(fhir_respond(
        process_subsumes(&state, params).await?,
        format,
    ))
}

/// GET /CodeSystem/$subsumes?system=...&codeA=...&codeB=...
pub async fn get_subsumes_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let pairs = parse_query_string(raw.as_deref().unwrap_or(""));
    let params = query_params_to_fhir_params(pairs);
    Ok(fhir_respond(
        process_subsumes(&state, params).await?,
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
                 VALUES ('cs1', 'http://example.org/hier', '1.0', 'Hier CS',
                         'active', 'complete', '2024-01-01', '2024-01-01');

                 INSERT INTO concepts (id, system_id, code, display)
                 VALUES (1, 'cs1', 'A', 'Concept A'),
                        (2, 'cs1', 'B', 'Concept B'),
                        (3, 'cs1', 'C', 'Concept C'),
                        (4, 'cs1', 'D', 'Concept D');

                 -- A → B → C  (direct edges only)
                 INSERT INTO concept_hierarchy (system_id, parent_code, child_code)
                 VALUES ('cs1', 'A', 'B'),
                        ('cs1', 'B', 'C');",
            )
            .unwrap();
            crate::backends::sqlite::schema::build_concept_closure(
                &backend.pool().get().unwrap(),
                "cs1",
            )
            .unwrap();
        }
        let state = AppState::new(backend);
        Router::new()
            .route(
                "/CodeSystem/$subsumes",
                post(subsumes_handler::<SqliteTerminologyBackend>),
            )
            .with_state(state)
    }

    async fn post_json(app: Router, uri: &str, body: Value) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn outcome(response: axum::response::Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap();
        json["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "outcome")
            .unwrap()["valueCode"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn params_body(code_a: &str, code_b: &str) -> Value {
        json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://example.org/hier"},
                {"name": "codeA", "valueCode": code_a},
                {"name": "codeB", "valueCode": code_b}
            ]
        })
    }

    #[tokio::test]
    async fn equivalent_same_code() {
        let resp = post_json(make_app(), "/CodeSystem/$subsumes", params_body("A", "A")).await;
        assert_eq!(resp.status(), 200);
        assert_eq!(outcome(resp).await, "equivalent");
    }

    #[tokio::test]
    async fn a_subsumes_b_direct() {
        let resp = post_json(make_app(), "/CodeSystem/$subsumes", params_body("A", "B")).await;
        assert_eq!(resp.status(), 200);
        assert_eq!(outcome(resp).await, "subsumes");
    }

    #[tokio::test]
    async fn a_subsumes_c_transitive() {
        // A → B → C: A is a two-hop ancestor of C.
        let resp = post_json(make_app(), "/CodeSystem/$subsumes", params_body("A", "C")).await;
        assert_eq!(resp.status(), 200);
        assert_eq!(outcome(resp).await, "subsumes");
    }

    #[tokio::test]
    async fn c_subsumed_by_a_transitive() {
        let resp = post_json(make_app(), "/CodeSystem/$subsumes", params_body("C", "A")).await;
        assert_eq!(resp.status(), 200);
        assert_eq!(outcome(resp).await, "subsumed-by");
    }

    #[tokio::test]
    async fn b_subsumed_by_a_direct() {
        let resp = post_json(make_app(), "/CodeSystem/$subsumes", params_body("B", "A")).await;
        assert_eq!(resp.status(), 200);
        assert_eq!(outcome(resp).await, "subsumed-by");
    }

    #[tokio::test]
    async fn not_subsumed_unrelated() {
        // D has no hierarchical relationship to A.
        let resp = post_json(make_app(), "/CodeSystem/$subsumes", params_body("A", "D")).await;
        assert_eq!(resp.status(), 200);
        assert_eq!(outcome(resp).await, "not-subsumed");
    }

    #[tokio::test]
    async fn not_subsumed_siblings() {
        // B and D are unrelated.
        let resp = post_json(make_app(), "/CodeSystem/$subsumes", params_body("B", "D")).await;
        assert_eq!(resp.status(), 200);
        assert_eq!(outcome(resp).await, "not-subsumed");
    }

    #[tokio::test]
    async fn missing_system_returns_400() {
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "codeA", "valueCode": "A"},
                {"name": "codeB", "valueCode": "B"}
            ]
        });
        let resp = post_json(make_app(), "/CodeSystem/$subsumes", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn missing_code_a_returns_400() {
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://example.org/hier"},
                {"name": "codeB", "valueCode": "B"}
            ]
        });
        let resp = post_json(make_app(), "/CodeSystem/$subsumes", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn missing_code_b_returns_400() {
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://example.org/hier"},
                {"name": "codeA", "valueCode": "A"}
            ]
        });
        let resp = post_json(make_app(), "/CodeSystem/$subsumes", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn unknown_system_returns_404() {
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://unknown.org/cs"},
                {"name": "codeA", "valueCode": "A"},
                {"name": "codeB", "valueCode": "B"}
            ]
        });
        let resp = post_json(make_app(), "/CodeSystem/$subsumes", body).await;
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    async fn wrong_resource_type_returns_400() {
        let body = json!({"resourceType": "CodeSystem", "parameter": []});
        let resp = post_json(make_app(), "/CodeSystem/$subsumes", body).await;
        assert_eq!(resp.status(), 400);
    }

    // ── codingA / codingB (valueCoding) input form ────────────────────────────

    #[tokio::test]
    async fn coding_form_subsumes_direct() {
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "codingA",
                    "valueCoding": {"system": "http://example.org/hier", "code": "A"}
                },
                {
                    "name": "codingB",
                    "valueCoding": {"system": "http://example.org/hier", "code": "B"}
                }
            ]
        });
        let resp = post_json(make_app(), "/CodeSystem/$subsumes", body).await;
        assert_eq!(resp.status(), 200);
        assert_eq!(outcome(resp).await, "subsumes");
    }

    #[tokio::test]
    async fn coding_form_equivalent() {
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "codingA",
                    "valueCoding": {"system": "http://example.org/hier", "code": "A"}
                },
                {
                    "name": "codingB",
                    "valueCoding": {"system": "http://example.org/hier", "code": "A"}
                }
            ]
        });
        let resp = post_json(make_app(), "/CodeSystem/$subsumes", body).await;
        assert_eq!(resp.status(), 200);
        assert_eq!(outcome(resp).await, "equivalent");
    }

    #[tokio::test]
    async fn cross_system_codings_returns_400() {
        // codingA and codingB from different systems — must be rejected.
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "codingA",
                    "valueCoding": {"system": "http://example.org/hier", "code": "A"}
                },
                {
                    "name": "codingB",
                    "valueCoding": {"system": "http://other.org/cs", "code": "B"}
                }
            ]
        });
        let resp = post_json(make_app(), "/CodeSystem/$subsumes", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn missing_coding_b_returns_400() {
        // codingA provided but codingB missing — falls through to error.
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "codingA",
                    "valueCoding": {"system": "http://example.org/hier", "code": "A"}
                }
            ]
        });
        let resp = post_json(make_app(), "/CodeSystem/$subsumes", body).await;
        assert_eq!(resp.status(), 400);
    }
}
