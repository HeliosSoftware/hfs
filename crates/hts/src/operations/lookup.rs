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
    extract::{Path, RawQuery, State},
    http::{HeaderMap, header},
    response::Response,
};
use helios_persistence::tenant::TenantContext;
use serde_json::{Value, json};

use crate::error::HtsError;
use crate::state::AppState;
use crate::traits::TerminologyBackend;
use crate::types::LookupRequest;

use super::format::{fhir_respond, negotiate_format};
use super::params::{
    collect_str_params, extract_parameter_array, find_str_param, parse_query_string,
    property_value_part, query_params_to_fhir_params,
};

async fn process_lookup<B: TerminologyBackend>(
    state: &AppState<B>,
    params: Vec<Value>,
) -> Result<Value, HtsError> {
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
        date: find_str_param(&params, "date"),
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

    Ok(json!({
        "resourceType": "Parameters",
        "parameter": parameter
    }))
}

/// POST /CodeSystem/$lookup
pub async fn lookup_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let params = extract_parameter_array(&body)?;
    Ok(fhir_respond(process_lookup(&state, params).await?, format))
}

/// GET /CodeSystem/$lookup?system=...&code=...
pub async fn get_lookup_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let pairs = parse_query_string(raw.as_deref().unwrap_or(""));
    let params = query_params_to_fhir_params(pairs);
    Ok(fhir_respond(process_lookup(&state, params).await?, format))
}

/// Inject (or replace) the `system` parameter in a params list.
///
/// Removes any existing `system` entry from the caller's params so that the
/// resource-id-resolved system URL always wins, then prepends it.
fn inject_system(mut params: Vec<Value>, system: String) -> Vec<Value> {
    params.retain(|p| p.get("name").and_then(|v| v.as_str()) != Some("system"));
    let mut with_system = vec![json!({"name": "system", "valueUri": system})];
    with_system.append(&mut params);
    with_system
}

/// POST /CodeSystem/{id}/$lookup
///
/// Resolves the CodeSystem canonical URL from its FHIR `id`, then delegates to
/// the same lookup logic used by the system-level endpoint.
pub async fn lookup_by_id_post<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let system = state
        .backend()
        .resource_url_by_id("CodeSystem", &id)
        .ok_or_else(|| HtsError::NotFound(format!("CodeSystem/{id}")))?;

    let raw_params = body
        .and_then(|Json(v)| extract_parameter_array(&v).ok())
        .unwrap_or_default();
    Ok(fhir_respond(
        process_lookup(&state, inject_system(raw_params, system)).await?,
        format,
    ))
}

/// GET /CodeSystem/{id}/$lookup?code=...
pub async fn get_lookup_by_id<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let system = state
        .backend()
        .resource_url_by_id("CodeSystem", &id)
        .ok_or_else(|| HtsError::NotFound(format!("CodeSystem/{id}")))?;

    let pairs = parse_query_string(raw.as_deref().unwrap_or(""));
    let params = query_params_to_fhir_params(pairs);
    Ok(fhir_respond(
        process_lookup(&state, inject_system(params, system)).await?,
        format,
    ))
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
