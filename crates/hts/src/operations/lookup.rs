//! Handlers for `CodeSystem/$lookup` — both type-level and instance-level.
//!
//! The lookup operation returns display name, designations, and arbitrary
//! concept properties for a single code in a given code system.  Two
//! addressing styles are supported:
//!
//! * **Type-level** — `GET /CodeSystem/$lookup?system=<url>&code=<code>` /
//!   `POST /CodeSystem/$lookup` with a FHIR `Parameters` body.
//! * **Instance-level** — `GET /CodeSystem/{id}/$lookup?code=<code>` /
//!   `POST /CodeSystem/{id}/$lookup` — resolves the canonical URL from the
//!   resource `id`, then delegates to the same logic as the type-level endpoint.
//!
//! ## Response shape
//!
//! All variants return a FHIR `Parameters` resource.  The mandatory `name`
//! parameter carries the CodeSystem's human name; optional parameters include
//! `version`, `display`, `property` (multi-valued), and `designation`
//! (multi-valued with language/use/value parts).
//!
//! ## FHIR specification
//!
//! <https://hl7.org/fhir/codesystem-operation-lookup.html>

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

/// Core lookup logic shared by all four public handlers.
///
/// Extracts `system`, `code`, and optional parameters (`version`,
/// `displayLanguage`, `expression`, `property`, `date`) from the normalised
/// `params` list, delegates to [`TerminologyBackend::lookup`], and assembles
/// the FHIR `Parameters` response.
///
/// ## Parameters
/// - `state` — application state holding the terminology backend.
/// - `params` — normalised FHIR parameter list (name/value pairs as JSON
///   objects).  Both POST body and GET query string params flow through this
///   function after normalisation by their respective callers.
///
/// ## Returns
/// A `serde_json::Value` representing a FHIR `Parameters` resource, or an
/// [`HtsError`] (400 when `system`/`code` are missing, 404 when the code is
/// not found, 501 when `expression` is supplied).
async fn process_lookup<B: TerminologyBackend>(
    state: &AppState<B>,
    params: Vec<Value>,
) -> Result<Value, HtsError> {
    let system = find_str_param(&params, "system")
        .ok_or_else(|| HtsError::InvalidRequest("Missing required parameter: system".into()))?;

    let code = find_str_param(&params, "code")
        .ok_or_else(|| HtsError::InvalidRequest("Missing required parameter: code".into()))?;

    let req = LookupRequest {
        system: system.clone(),
        code: code.clone(),
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

    // Echo back the system + code so the IG fixtures can confirm what we
    // looked up; also surface `abstract` from the notSelectable property
    // when set.
    parameter.push(json!({"name": "system", "valueUri": system}));
    parameter.push(json!({"name": "code", "valueCode": code}));
    if resp
        .properties
        .iter()
        .any(|p| p.code == "notSelectable" && p.value == "true")
    {
        parameter.push(json!({"name": "abstract", "valueBoolean": true}));
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

/// `POST /CodeSystem/$lookup`
///
/// Accepts a FHIR `Parameters` body.  The `system` (CodeSystem URL) and `code`
/// parameters are required; `version`, `displayLanguage`, `property`, and
/// `date` are optional.  Content negotiation via `Accept` header or `_format`
/// query parameter selects JSON or XML output.
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

/// `GET /CodeSystem/$lookup?system=<url>&code=<code>`
///
/// URL query parameters are mapped to FHIR `Parameters` name/value pairs and
/// then processed identically to the POST form.  `system`, `version`,
/// `displayLanguage`, `property`, and `date` are all accepted.
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

/// Inject (or replace) the `system` parameter in a FHIR params list.
///
/// Used by the instance-level handlers (`/CodeSystem/{id}/$lookup`) to ensure
/// the canonical URL resolved from the resource `id` always takes precedence
/// over any `system` value the caller may have supplied.
///
/// ## Parameters
/// - `params` — existing parameter list (may contain a `system` entry).
/// - `system` — canonical URL resolved from the CodeSystem's logical `id`.
///
/// ## Returns
/// A new params list with the resolved `system` prepended and any prior
/// `system` entry removed.
fn inject_system(mut params: Vec<Value>, system: String) -> Vec<Value> {
    params.retain(|p| p.get("name").and_then(|v| v.as_str()) != Some("system"));
    let mut with_system = vec![json!({"name": "system", "valueUri": system})];
    with_system.append(&mut params);
    with_system
}

/// `POST /CodeSystem/{id}/$lookup`
///
/// Instance-level variant of `$lookup`.  Resolves the CodeSystem canonical URL
/// from its FHIR logical `id`, injects it as the `system` parameter (overriding
/// any caller-supplied value), and delegates to the same processing pipeline as
/// the type-level endpoint.
///
/// Returns 404 when no CodeSystem with the given `id` is found.
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

/// `GET /CodeSystem/{id}/$lookup?code=<code>`
///
/// Instance-level GET variant.  Resolves the CodeSystem canonical URL from its
/// FHIR logical `id` and merges it with the query-string parameters before
/// dispatching to the shared lookup pipeline.
///
/// Returns 404 when no CodeSystem with the given `id` is found.
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

    use crate::backends::sqlite::SqliteTerminologyBackend;
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
