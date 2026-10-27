//! Handler for `POST /ConceptMap/$closure`.
//!
//! Accepts a FHIR Parameters resource containing a `name` (required) and zero
//! or more `concept` parameters (each a `valueCoding` with `system` and `code`).
//! Returns a FHIR Parameters resource with a `return` parameter holding a
//! ConceptMap resource that represents the transitive closure of the input
//! concepts — i.e., all ancestor-descendant pairs found in the hierarchy for
//! the given code systems.
//!
//! # FHIR specification
//! <https://hl7.org/fhir/conceptmap-operation-closure.html>

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
use crate::traits::{ConceptMapOperations, TerminologyBackend};
use crate::types::{ClosureRequest, CodingConcept};

use super::format::{fhir_respond, negotiate_format};
use super::params::extract_parameter_array;

/// POST /ConceptMap/$closure
pub async fn closure_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, HtsError> {
    let params = extract_parameter_array(&body)?;

    // `name` is required.
    let name = params
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("name"))
        .and_then(|p| {
            p.get("valueString")
                .or_else(|| p.get("valueId"))
                .and_then(|v| v.as_str())
        })
        .map(|s| s.to_string())
        .ok_or_else(|| HtsError::InvalidRequest("Missing required parameter: name".into()))?;

    // Collect all `concept` parameters (each is a valueCoding with system + code).
    let concepts: Vec<CodingConcept> = params
        .iter()
        .filter(|p| p.get("name").and_then(|v| v.as_str()) == Some("concept"))
        .filter_map(|p| {
            let coding = p.get("valueCoding")?;
            let system = coding.get("system")?.as_str()?.to_string();
            let code = coding.get("code")?.as_str()?.to_string();
            let display = coding
                .get("display")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Some(CodingConcept {
                system,
                code,
                display,
            })
        })
        .collect();

    // Optional `version` parameter.
    let version = params
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("version"))
        .and_then(|p| p.get("valueString").and_then(|v| v.as_str()))
        .map(|s| s.to_string());

    let req = ClosureRequest {
        name,
        concept: concepts,
        version,
    };

    let ctx = TenantContext::system();
    let resp = ConceptMapOperations::closure(state.backend(), &ctx, req).await?;

    // ── Build FHIR Parameters response ─────────────────────────────────────────
    let mut parameter: Vec<Value> = Vec::new();

    if let Some(concept_map) = resp.concept_map {
        parameter.push(json!({
            "name": "return",
            "resource": concept_map
        }));
    }

    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    Ok(fhir_respond(
        json!({
            "resourceType": "Parameters",
            "parameter": parameter
        }),
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
                 VALUES ('cs1', 'http://example.org/cs', '1.0', 'Test CS',
                         'active', 'complete', '2024-01-01', '2024-01-01');

                 INSERT INTO concepts (id, system_id, code, display)
                 VALUES (1, 'cs1', 'A', 'Alpha'),
                        (2, 'cs1', 'B', 'Beta'),
                        (3, 'cs1', 'C', 'Gamma'),
                        (4, 'cs1', 'D', 'Delta');

                 -- Hierarchy: A → B → C (D is unrelated)
                 INSERT INTO concept_hierarchy (system_id, parent_code, child_code)
                 VALUES ('cs1', 'A', 'B'),
                        ('cs1', 'B', 'C');",
            )
            .unwrap();
        }
        let state = AppState::new(backend);
        Router::new()
            .route(
                "/ConceptMap/$closure",
                post(closure_handler::<SqliteTerminologyBackend>),
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

    async fn body_json(response: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn concept_param(system: &str, code: &str) -> Value {
        json!({
            "name": "concept",
            "valueCoding": {
                "system": system,
                "code": code
            }
        })
    }

    // ── Happy path ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn closure_returns_parameters_resource() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "name", "valueString": "my-closure"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$closure", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        assert_eq!(json["resourceType"], "Parameters");
    }

    #[tokio::test]
    async fn closure_empty_concepts_returns_empty_concept_map() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "name", "valueString": "my-closure"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$closure", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        let ret = params.iter().find(|p| p["name"] == "return").unwrap();
        assert_eq!(ret["resource"]["resourceType"], "ConceptMap");
        assert_eq!(ret["resource"]["group"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn closure_three_codes_returns_subsumption_pairs() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "name", "valueString": "test"},
                concept_param("http://example.org/cs", "A"),
                concept_param("http://example.org/cs", "B"),
                concept_param("http://example.org/cs", "C")
            ]
        });

        let resp = post_json(app, "/ConceptMap/$closure", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let ret = params.iter().find(|p| p["name"] == "return").unwrap();
        let cm = &ret["resource"];
        let groups = cm["group"].as_array().unwrap();
        assert!(!groups.is_empty(), "Expected at least one group");

        // Collect all ancestor→descendant pairs.
        let mut pairs: Vec<(String, String)> = Vec::new();
        for group in groups {
            for elem in group["element"].as_array().unwrap() {
                let ancestor = elem["code"].as_str().unwrap().to_string();
                for target in elem["target"].as_array().unwrap() {
                    pairs.push((
                        ancestor.clone(),
                        target["code"].as_str().unwrap().to_string(),
                    ));
                }
            }
        }

        assert!(pairs.contains(&("A".into(), "B".into())), "A subsumes B");
        assert!(
            pairs.contains(&("A".into(), "C".into())),
            "A subsumes C (transitive)"
        );
        assert!(pairs.contains(&("B".into(), "C".into())), "B subsumes C");
    }

    #[tokio::test]
    async fn closure_unrelated_codes_produce_empty_groups() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "name", "valueString": "test"},
                concept_param("http://example.org/cs", "A"),
                concept_param("http://example.org/cs", "D")
            ]
        });

        let resp = post_json(app, "/ConceptMap/$closure", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        let ret = params.iter().find(|p| p["name"] == "return").unwrap();
        let groups = ret["resource"]["group"].as_array().unwrap();
        assert_eq!(
            groups.len(),
            0,
            "A and D are unrelated — no subsumption pairs"
        );
    }

    #[tokio::test]
    async fn closure_concept_map_has_correct_name() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "name", "valueString": "my-named-closure"},
                concept_param("http://example.org/cs", "A"),
                concept_param("http://example.org/cs", "B")
            ]
        });

        let resp = post_json(app, "/ConceptMap/$closure", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        let ret = params.iter().find(|p| p["name"] == "return").unwrap();
        assert_eq!(ret["resource"]["name"], "my-named-closure");
    }

    // ── Error cases ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn closure_missing_name_returns_400() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                concept_param("http://example.org/cs", "A")
            ]
        });

        let resp = post_json(app, "/ConceptMap/$closure", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn closure_wrong_resource_type_returns_400() {
        let app = make_app();
        let body = json!({"resourceType": "ConceptMap", "parameter": []});

        let resp = post_json(app, "/ConceptMap/$closure", body).await;
        assert_eq!(resp.status(), 400);
    }
}
