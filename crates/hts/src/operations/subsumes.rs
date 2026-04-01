/// Handler for `POST /CodeSystem/$subsumes`.
///
/// Accepts a FHIR Parameters resource containing `system`, `codeA`, `codeB`,
/// and an optional `version`. Returns a FHIR Parameters resource with a single
/// `outcome` parameter whose value is one of:
/// - `"equivalent"`   — the two codes are identical
/// - `"subsumes"`     — codeA is an ancestor of codeB
/// - `"subsumed-by"`  — codeA is a descendant of codeB
/// - `"not-subsumed"` — no hierarchical relationship exists
///
/// # FHIR specification
/// <https://hl7.org/fhir/codesystem-operation-subsumes.html>
use axum::{
    Json,
    extract::{RawQuery, State},
};
use helios_persistence::tenant::TenantContext;
use serde_json::{Value, json};

use crate::error::HtsError;
use crate::state::AppState;
use crate::traits::{CodeSystemOperations, TerminologyBackend};
use crate::types::{SubsumesRequest, SubsumptionOutcome};

use super::params::{
    extract_parameter_array, find_str_param, parse_query_string, query_params_to_fhir_params,
};

async fn process_subsumes<B: TerminologyBackend>(
    state: &AppState<B>,
    params: Vec<Value>,
) -> Result<Json<Value>, HtsError> {
    let system = find_str_param(&params, "system")
        .ok_or_else(|| HtsError::InvalidRequest("Missing required parameter: system".into()))?;

    let code_a = find_str_param(&params, "codeA")
        .ok_or_else(|| HtsError::InvalidRequest("Missing required parameter: codeA".into()))?;

    let code_b = find_str_param(&params, "codeB")
        .ok_or_else(|| HtsError::InvalidRequest("Missing required parameter: codeB".into()))?;

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

    Ok(Json(json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "outcome", "valueCode": outcome_str}
        ]
    })))
}

/// POST /CodeSystem/$subsumes
pub async fn subsumes_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, HtsError> {
    let params = extract_parameter_array(&body)?;
    process_subsumes(&state, params).await
}

/// GET /CodeSystem/$subsumes?system=...&codeA=...&codeB=...
pub async fn get_subsumes_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    RawQuery(raw): RawQuery,
) -> Result<Json<Value>, HtsError> {
    let pairs = parse_query_string(raw.as_deref().unwrap_or(""));
    let params = query_params_to_fhir_params(pairs);
    process_subsumes(&state, params).await
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::Request, routing::post};
    use tower::ServiceExt;

    use crate::backend::sqlite::SqliteTerminologyBackend;
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

                 -- A → B → C  (direct edges only; recursive CTE traverses transitively)
                 INSERT INTO concept_hierarchy (system_id, parent_code, child_code)
                 VALUES ('cs1', 'A', 'B'),
                        ('cs1', 'B', 'C');",
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
}
