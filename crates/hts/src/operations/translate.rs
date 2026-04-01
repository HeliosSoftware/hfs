//! Handler for `POST /ConceptMap/$translate`.
//!
//! Accepts a FHIR Parameters resource containing `code` (required) and optional
//! `system`, `url` (ConceptMap URL), and `reverse`. Returns a FHIR Parameters
//! resource with a `result` boolean and zero or more `match` parts.
//!
//! # FHIR specification
//! <https://hl7.org/fhir/conceptmap-operation-translate.html>

use axum::{Json, extract::State};
use helios_persistence::tenant::TenantContext;
use serde_json::{Value, json};

use crate::error::HtsError;
use crate::state::AppState;
use crate::traits::{ConceptMapOperations, TerminologyBackend};
use crate::types::TranslateRequest;

use super::params::{extract_parameter_array, find_str_param};

/// POST /ConceptMap/$translate
pub async fn translate_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, HtsError> {
    let params = extract_parameter_array(&body)?;

    let code = find_str_param(&params, "code")
        .ok_or_else(|| HtsError::InvalidRequest("Missing required parameter: code".into()))?;

    // `reverse` arrives as valueBoolean; extract_any_string_value converts it to "true"/"false".
    let reverse = find_str_param(&params, "reverse")
        .map(|s| s == "true")
        .unwrap_or(false);

    let req = TranslateRequest {
        url: find_str_param(&params, "url"),
        system: find_str_param(&params, "system"),
        code,
        source: find_str_param(&params, "source"),
        target: find_str_param(&params, "target"),
        target_system: find_str_param(&params, "targetSystem"),
        reverse,
    };

    let ctx = TenantContext::system();
    let resp = ConceptMapOperations::translate(state.backend(), &ctx, req).await?;

    // ── Build FHIR Parameters response ─────────────────────────────────────────
    let mut parameter: Vec<Value> = vec![json!({
        "name": "result",
        "valueBoolean": resp.result
    })];

    if let Some(msg) = resp.message {
        parameter.push(json!({
            "name": "message",
            "valueString": msg
        }));
    }

    for m in resp.matches {
        let mut parts: Vec<Value> = vec![
            json!({"name": "equivalence", "valueCode": m.equivalence}),
            json!({
                "name": "concept",
                "valueCoding": {
                    "system": m.concept_system,
                    "code": m.concept_code,
                    "display": m.concept_display
                }
            }),
        ];
        if let Some(src) = m.source {
            parts.push(json!({"name": "source", "valueUri": src}));
        }
        parameter.push(json!({"name": "match", "part": parts}));
    }

    Ok(Json(json!({
        "resourceType": "Parameters",
        "parameter": parameter
    })))
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
                 VALUES ('cs-src', 'http://example.org/src', '1.0', 'Source CS',
                         'active', 'complete', '2024-01-01', '2024-01-01'),
                        ('cs-tgt', 'http://example.org/tgt', '1.0', 'Target CS',
                         'active', 'complete', '2024-01-01', '2024-01-01');

                 INSERT INTO concepts (id, system_id, code, display)
                 VALUES (1, 'cs-src', 'A', 'Alpha'),
                        (2, 'cs-src', 'B', 'Beta'),
                        (10, 'cs-tgt', 'X', 'X-Ray'),
                        (11, 'cs-tgt', 'Y', 'Yankee');

                 INSERT INTO concept_maps
                     (id, url, version, source_uri, target_uri, status, created_at)
                 VALUES ('cm1', 'http://example.org/cm', '1.0',
                         'http://example.org/src', 'http://example.org/tgt',
                         'active', '2024-01-01');

                 INSERT INTO concept_map_elements
                     (map_id, source_system, source_code, target_system, target_code, equivalence)
                 VALUES ('cm1', 'http://example.org/src', 'A',
                         'http://example.org/tgt', 'X', 'equivalent'),
                        ('cm1', 'http://example.org/src', 'B',
                         'http://example.org/tgt', 'Y', 'wider');",
            )
            .unwrap();
        }
        let state = AppState::new(backend);
        Router::new()
            .route(
                "/ConceptMap/$translate",
                post(translate_handler::<SqliteTerminologyBackend>),
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

    // ── Happy path ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn translate_returns_parameters_resource() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "code", "valueCode": "A"},
                {"name": "system", "valueUri": "http://example.org/src"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        assert_eq!(json["resourceType"], "Parameters");
    }

    #[tokio::test]
    async fn translate_result_true_on_match() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "code", "valueCode": "A"},
                {"name": "system", "valueUri": "http://example.org/src"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        let result_param = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result_param["valueBoolean"], true);
    }

    #[tokio::test]
    async fn translate_match_contains_concept_coding() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "code", "valueCode": "A"},
                {"name": "system", "valueUri": "http://example.org/src"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        let match_param = params.iter().find(|p| p["name"] == "match").unwrap();
        let parts = match_param["part"].as_array().unwrap();

        let concept = parts.iter().find(|p| p["name"] == "concept").unwrap();
        assert_eq!(concept["valueCoding"]["code"], "X");
        assert_eq!(concept["valueCoding"]["system"], "http://example.org/tgt");
        assert_eq!(concept["valueCoding"]["display"], "X-Ray");
    }

    #[tokio::test]
    async fn translate_match_contains_equivalence() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "code", "valueCode": "A"},
                {"name": "system", "valueUri": "http://example.org/src"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        let match_param = params.iter().find(|p| p["name"] == "match").unwrap();
        let parts = match_param["part"].as_array().unwrap();

        let equiv = parts.iter().find(|p| p["name"] == "equivalence").unwrap();
        assert_eq!(equiv["valueCode"], "equivalent");
    }

    #[tokio::test]
    async fn translate_result_false_on_no_match() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "code", "valueCode": "UNKNOWN"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result_param = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result_param["valueBoolean"], false);
    }

    #[tokio::test]
    async fn translate_reverse_finds_source_code() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "code", "valueCode": "X"},
                {"name": "system", "valueUri": "http://example.org/tgt"},
                {"name": "reverse", "valueBoolean": true}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result_param = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result_param["valueBoolean"], true);

        let match_param = params.iter().find(|p| p["name"] == "match").unwrap();
        let parts = match_param["part"].as_array().unwrap();
        let concept = parts.iter().find(|p| p["name"] == "concept").unwrap();
        assert_eq!(concept["valueCoding"]["code"], "A");
    }

    // ── Error cases ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn translate_missing_code_returns_400() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://example.org/src"}
            ]
        });

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn translate_wrong_resource_type_returns_400() {
        let app = make_app();
        let body = json!({"resourceType": "ConceptMap", "parameter": []});

        let resp = post_json(app, "/ConceptMap/$translate", body).await;
        assert_eq!(resp.status(), 400);
    }
}
