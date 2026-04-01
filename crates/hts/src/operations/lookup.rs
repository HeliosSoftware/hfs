/// Handlers for `GET` and `POST /CodeSystem/$lookup`.
///
/// Accepts a FHIR Parameters resource (POST) or URL query string (GET),
/// extracts `system`, `code`, and optional fields, delegates to the
/// terminology backend, and returns the result as a FHIR Parameters resource.
///
/// # FHIR specification
/// <https://hl7.org/fhir/codesystem-operation-lookup.html>
use axum::{
    Json,
    extract::{RawQuery, State},
};
use helios_persistence::tenant::TenantContext;
use serde_json::{Value, json};

use crate::error::HtsError;
use crate::state::AppState;
use crate::traits::TerminologyBackend;
use crate::types::LookupRequest;

use super::params::{
    collect_str_params, extract_parameter_array, find_str_param, parse_query_string,
    property_value_part, query_params_to_fhir_params,
};

async fn process_lookup<B: TerminologyBackend>(
    state: &AppState<B>,
    params: Vec<Value>,
) -> Result<Json<Value>, HtsError> {
    let system = find_str_param(&params, "system")
        .ok_or_else(|| HtsError::InvalidRequest("Missing required parameter: system".into()))?;

    let code = find_str_param(&params, "code")
        .ok_or_else(|| HtsError::InvalidRequest("Missing required parameter: code".into()))?;

    let req = LookupRequest {
        system,
        code,
        version: find_str_param(&params, "version"),
        display_language: find_str_param(&params, "displayLanguage"),
        expression: find_str_param(&params, "expression"),
        properties: collect_str_params(&params, "property"),
    };

    let ctx = TenantContext::system();
    let resp = state.backend().lookup(&ctx, req).await?;

    // ── Build FHIR Parameters response ─────────────────────────────────────────
    let mut parameter: Vec<Value> = vec![json!({"name": "name", "valueString": resp.name})];

    if let Some(ver) = resp.version {
        parameter.push(json!({"name": "version", "valueString": ver}));
    }

    if let Some(display) = resp.display {
        parameter.push(json!({"name": "display", "valueString": display}));
    }

    for prop in resp.properties {
        let value_part = property_value_part(&prop.value_type, &prop.value);
        let mut parts = vec![json!({"name": "code", "valueCode": prop.code}), value_part];
        if let Some(desc) = prop.description {
            parts.push(json!({"name": "description", "valueString": desc}));
        }
        parameter.push(json!({"name": "property", "part": parts}));
    }

    for desig in resp.designations {
        let mut parts: Vec<Value> = vec![];

        if let Some(lang) = desig.language {
            parts.push(json!({"name": "language", "valueCode": lang}));
        }

        if desig.use_system.is_some() || desig.use_code.is_some() {
            parts.push(json!({
                "name": "use",
                "valueCoding": {
                    "system": desig.use_system,
                    "code": desig.use_code
                }
            }));
        }

        parts.push(json!({"name": "value", "valueString": desig.value}));
        parameter.push(json!({"name": "designation", "part": parts}));
    }

    Ok(Json(json!({
        "resourceType": "Parameters",
        "parameter": parameter
    })))
}

/// POST /CodeSystem/$lookup
pub async fn lookup_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, HtsError> {
    let params = extract_parameter_array(&body)?;
    process_lookup(&state, params).await
}

/// GET /CodeSystem/$lookup?system=...&code=...
pub async fn get_lookup_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    RawQuery(raw): RawQuery,
) -> Result<Json<Value>, HtsError> {
    let pairs = parse_query_string(raw.as_deref().unwrap_or(""));
    let params = query_params_to_fhir_params(pairs);
    process_lookup(&state, params).await
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
                 VALUES ('cs1', 'http://example.org/cs', '1.0', 'Example CS',
                         'active', 'complete', '2024-01-01', '2024-01-01');

                 INSERT INTO concepts (id, system_id, code, display)
                 VALUES (1, 'cs1', 'ABC', 'Alpha Beta Charlie');

                 INSERT INTO concept_properties (concept_id, property, value_type, value)
                 VALUES (1, 'inactive', 'boolean', 'false');

                 INSERT INTO concept_designations (concept_id, language, use_system, use_code, value)
                 VALUES (1, 'fr', NULL, NULL, 'Alpha Bêta Charlie');",
            )
            .unwrap();
        }
        let state = AppState::new(backend);
        Router::new()
            .route(
                "/CodeSystem/$lookup",
                post(lookup_handler::<SqliteTerminologyBackend>),
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

    #[tokio::test]
    async fn lookup_valid_code_returns_200_and_display() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://example.org/cs"},
                {"name": "code", "valueCode": "ABC"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$lookup", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        assert_eq!(json["resourceType"], "Parameters");

        let params = json["parameter"].as_array().unwrap();
        let display_param = params.iter().find(|p| p["name"] == "display").unwrap();
        assert_eq!(display_param["valueString"], "Alpha Beta Charlie");
    }

    #[tokio::test]
    async fn lookup_returns_cs_name() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://example.org/cs"},
                {"name": "code", "valueCode": "ABC"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$lookup", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        let name_param = params.iter().find(|p| p["name"] == "name").unwrap();
        assert_eq!(name_param["valueString"], "Example CS");
    }

    #[tokio::test]
    async fn lookup_returns_properties() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://example.org/cs"},
                {"name": "code", "valueCode": "ABC"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$lookup", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        let prop_param = params.iter().find(|p| p["name"] == "property").unwrap();
        let parts = prop_param["part"].as_array().unwrap();
        let code_part = parts.iter().find(|p| p["name"] == "code").unwrap();
        assert_eq!(code_part["valueCode"], "inactive");
    }

    #[tokio::test]
    async fn lookup_returns_designation() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://example.org/cs"},
                {"name": "code", "valueCode": "ABC"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$lookup", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        let desig_param = params.iter().find(|p| p["name"] == "designation").unwrap();
        let parts = desig_param["part"].as_array().unwrap();
        let lang_part = parts.iter().find(|p| p["name"] == "language").unwrap();
        assert_eq!(lang_part["valueCode"], "fr");
    }

    #[tokio::test]
    async fn lookup_unknown_code_returns_404() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://example.org/cs"},
                {"name": "code", "valueCode": "UNKNOWN"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$lookup", body).await;
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    async fn lookup_missing_system_returns_400() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "code", "valueCode": "ABC"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$lookup", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn lookup_missing_code_returns_400() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://example.org/cs"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$lookup", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn lookup_wrong_resource_type_returns_400() {
        let app = make_app();
        let body = json!({
            "resourceType": "Patient",
            "parameter": []
        });

        let resp = post_json(app, "/CodeSystem/$lookup", body).await;
        assert_eq!(resp.status(), 400);
    }
}
