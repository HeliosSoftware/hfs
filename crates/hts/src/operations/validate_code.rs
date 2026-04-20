//! Handlers for `POST /CodeSystem/$validate-code` and
//! `POST /ValueSet/$validate-code`.
//!
//! Both operations accept a FHIR Parameters resource and return a FHIR
//! Parameters resource with a boolean `result`, optional `message`, and
//! optional `display`.
//!
//! **CodeSystem/$validate-code** requires the `url` parameter (CodeSystem
//! canonical URL). Sending `system` instead returns HTTP 400.
//!
//! **ValueSet/$validate-code** requires the `url` parameter (ValueSet
//! canonical URL) and optionally accepts `system` (to scope the lookup to a
//! specific code system within the expanded value set).
//!
//! # FHIR specifications
//! - CodeSystem: <https://hl7.org/fhir/codesystem-operation-validate-code.html>
//! - ValueSet:   <https://hl7.org/fhir/valueset-operation-validate-code.html>
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
use crate::traits::{CodeSystemOperations, TerminologyBackend, ValueSetOperations};
use crate::types::{ValidateCodeRequest, ValidateCodeResponse};

use super::format::{fhir_respond, negotiate_format};
use super::params::{
    extract_codeable_concept, extract_coding, extract_parameter_array, find_str_param,
    parse_query_string, query_params_to_fhir_params,
};

/// Serialize a [`ValidateCodeResponse`] into a FHIR Parameters JSON value.
///
/// Always includes `result` (boolean).  Includes `message` when set (e.g.,
/// when a display mismatch is detected).  Includes `display` on success.
fn build_validate_response(resp: ValidateCodeResponse) -> Value {
    let mut parameter: Vec<Value> = vec![json!({"name": "result", "valueBoolean": resp.result})];
    if let Some(msg) = resp.message {
        parameter.push(json!({"name": "message", "valueString": msg}));
    }
    if let Some(display) = resp.display {
        parameter.push(json!({"name": "display", "valueString": display}));
    }
    json!({
        "resourceType": "Parameters",
        "parameter": parameter
    })
}

/// Core validate-code logic for `CodeSystem/$validate-code`.
///
/// Accepts three input forms (checked in priority order):
///
/// 1. **`code`** parameter — requires `url` (CodeSystem canonical URL); `system`
///    is intentionally not accepted here (FHIR spec distinction).
/// 2. **`coding`** (`valueCoding`) — system and code bundled in a single object.
/// 3. **`codeableConcept`** (`valueCodeableConcept`) — returns `true` if *any*
///    coding in the concept is valid.
///
/// ## Returns
///
/// A FHIR `Parameters` resource with `result` (boolean), optional `display`
/// (on success), and optional `message` (on display mismatch or failure).
///
/// ## Errors
///
/// Returns [`HtsError::InvalidRequest`] when none of the three input forms are
/// present, or when `url` is absent for the bare-code form.
pub(crate) async fn process_validate_code<B: TerminologyBackend>(
    state: &AppState<B>,
    params: Vec<Value>,
) -> Result<Value, HtsError> {
    let ctx = TenantContext::system();

    // ── Path 1: bare `code` parameter (requires `url` = CodeSystem canonical URL) ──
    if let Some(code) = find_str_param(&params, "code") {
        let system = find_str_param(&params, "url").ok_or_else(|| {
            HtsError::InvalidRequest(
                "Missing required parameter: url (CodeSystem canonical URL). \
                 Use `url`, not `system`, for CodeSystem/$validate-code."
                    .into(),
            )
        })?;
        let req = ValidateCodeRequest {
            url: None,
            system: Some(system),
            code,
            version: find_str_param(&params, "version"),
            display: find_str_param(&params, "display"),
            date: find_str_param(&params, "date"),
        };
        let resp = CodeSystemOperations::validate_code(state.backend(), &ctx, req).await?;
        return Ok(build_validate_response(resp));
    }

    // ── Path 2: `coding` parameter (valueCoding — system+code bundled together) ──
    if let Some((system, code, _display)) = extract_coding(&params, "coding") {
        let req = ValidateCodeRequest {
            url: None,
            system: Some(system),
            code,
            version: find_str_param(&params, "version"),
            display: find_str_param(&params, "display"),
            date: find_str_param(&params, "date"),
        };
        let resp = CodeSystemOperations::validate_code(state.backend(), &ctx, req).await?;
        return Ok(build_validate_response(resp));
    }

    // ── Path 3: `codeableConcept` parameter (multiple codings — true if any matches) ──
    if let Some(codings) = extract_codeable_concept(&params, "codeableConcept") {
        if codings.is_empty() {
            return Err(HtsError::InvalidRequest(
                "codeableConcept parameter has no valid coding entries".into(),
            ));
        }
        for (system, code) in codings {
            let req = ValidateCodeRequest {
                url: None,
                system: Some(system),
                code,
                version: find_str_param(&params, "version"),
                display: None,
                date: find_str_param(&params, "date"),
            };
            let resp = CodeSystemOperations::validate_code(state.backend(), &ctx, req).await?;
            if resp.result {
                return Ok(build_validate_response(resp));
            }
        }
        // No coding matched
        return Ok(build_validate_response(ValidateCodeResponse {
            result: false,
            message: Some("None of the provided codings were found in any CodeSystem".into()),
            display: None,
        }));
    }

    Err(HtsError::InvalidRequest(
        "Must provide one of: code, coding (valueCoding), or \
         codeableConcept (valueCodeableConcept)"
            .into(),
    ))
}

/// POST /CodeSystem/$validate-code
pub async fn validate_code_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let params = extract_parameter_array(&body)?;
    Ok(fhir_respond(
        process_validate_code(&state, params).await?,
        format,
    ))
}

/// GET /CodeSystem/$validate-code?url=...&code=...
pub async fn get_validate_code_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let pairs = parse_query_string(raw.as_deref().unwrap_or(""));
    let params = query_params_to_fhir_params(pairs);
    Ok(fhir_respond(
        process_validate_code(&state, params).await?,
        format,
    ))
}

// ── ValueSet/$validate-code ────────────────────────────────────────────────────

/// Core validate-code logic for `ValueSet/$validate-code`.
///
/// Always requires the `url` parameter (ValueSet canonical URL).  The optional
/// `system` parameter can further scope the check to a specific code system
/// within the expanded value set.
///
/// Supports the same three input forms as [`process_validate_code`] (bare
/// `code`, `coding`, and `codeableConcept`), with the same priority order.
///
/// Unlike `CodeSystem/$validate-code`, a missing or unknown ValueSet URL
/// returns `result = false` (not an error), consistent with the FHIR spec's
/// intent to treat absence of a value set as a negative match.
pub(crate) async fn process_vs_validate_code<B: TerminologyBackend>(
    state: &AppState<B>,
    params: Vec<Value>,
) -> Result<Value, HtsError> {
    // ValueSet/$validate-code always requires `url` (the ValueSet canonical URL).
    let url = find_str_param(&params, "url").ok_or_else(|| {
        HtsError::InvalidRequest("Missing required parameter: url (ValueSet canonical URL)".into())
    })?;

    let ctx = TenantContext::system();

    // ── Path 1: bare `code` parameter ────────────────────────────────────────────
    if let Some(code) = find_str_param(&params, "code") {
        let req = ValidateCodeRequest {
            url: Some(url),
            system: find_str_param(&params, "system"),
            code,
            version: find_str_param(&params, "version"),
            display: find_str_param(&params, "display"),
            date: find_str_param(&params, "date"),
        };
        let resp = ValueSetOperations::validate_code(state.backend(), &ctx, req).await?;
        return Ok(build_validate_response(resp));
    }

    // ── Path 2: `coding` parameter (valueCoding) ──────────────────────────────
    if let Some((system, code, _display)) = extract_coding(&params, "coding") {
        let req = ValidateCodeRequest {
            url: Some(url),
            system: Some(system),
            code,
            version: find_str_param(&params, "version"),
            display: find_str_param(&params, "display"),
            date: find_str_param(&params, "date"),
        };
        let resp = ValueSetOperations::validate_code(state.backend(), &ctx, req).await?;
        return Ok(build_validate_response(resp));
    }

    // ── Path 3: `codeableConcept` parameter (true if any coding is in the ValueSet) ──
    if let Some(codings) = extract_codeable_concept(&params, "codeableConcept") {
        if codings.is_empty() {
            return Err(HtsError::InvalidRequest(
                "codeableConcept parameter has no valid coding entries".into(),
            ));
        }
        for (system, code) in codings {
            let req = ValidateCodeRequest {
                url: Some(url.clone()),
                system: Some(system),
                code,
                version: find_str_param(&params, "version"),
                display: None,
                date: find_str_param(&params, "date"),
            };
            let resp = ValueSetOperations::validate_code(state.backend(), &ctx, req).await?;
            if resp.result {
                return Ok(build_validate_response(resp));
            }
        }
        return Ok(build_validate_response(ValidateCodeResponse {
            result: false,
            message: Some("None of the provided codings were found in the ValueSet".into()),
            display: None,
        }));
    }

    Err(HtsError::InvalidRequest(
        "Must provide one of: code, coding (valueCoding), or \
         codeableConcept (valueCodeableConcept)"
            .into(),
    ))
}

/// POST /ValueSet/$validate-code
pub async fn vs_validate_code_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let params = extract_parameter_array(&body)?;
    Ok(fhir_respond(
        process_vs_validate_code(&state, params).await?,
        format,
    ))
}

/// GET /ValueSet/$validate-code?url=...&code=...
pub async fn get_vs_validate_code_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let pairs = parse_query_string(raw.as_deref().unwrap_or(""));
    let params = query_params_to_fhir_params(pairs);
    Ok(fhir_respond(
        process_vs_validate_code(&state, params).await?,
        format,
    ))
}

// ── Instance-level: /ValueSet/{id}/$validate-code ─────────────────────────────

/// Inject (or replace) the `url` parameter in a params list.
fn inject_url(mut params: Vec<Value>, url: String) -> Vec<Value> {
    params.retain(|p| p.get("name").and_then(|v| v.as_str()) != Some("url"));
    let mut with_url = vec![json!({"name": "url", "valueUri": url})];
    with_url.append(&mut params);
    with_url
}

/// POST /ValueSet/{id}/$validate-code
///
/// Resolves the ValueSet canonical URL from its FHIR `id`, then delegates to
/// the same validate-code logic used by the system-level endpoint.
pub async fn vs_validate_by_id_post<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let url = state
        .backend()
        .resource_url_by_id("ValueSet", &id)
        .ok_or_else(|| HtsError::NotFound(format!("ValueSet/{id}")))?;

    let raw_params = body
        .and_then(|Json(v)| extract_parameter_array(&v).ok())
        .unwrap_or_default();
    Ok(fhir_respond(
        process_vs_validate_code(&state, inject_url(raw_params, url)).await?,
        format,
    ))
}

/// GET /ValueSet/{id}/$validate-code?code=...
pub async fn get_vs_validate_by_id<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let url = state
        .backend()
        .resource_url_by_id("ValueSet", &id)
        .ok_or_else(|| HtsError::NotFound(format!("ValueSet/{id}")))?;

    let pairs = parse_query_string(raw.as_deref().unwrap_or(""));
    let params = query_params_to_fhir_params(pairs);
    Ok(fhir_respond(
        process_vs_validate_code(&state, inject_url(params, url)).await?,
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
                 VALUES (1, 'cs1', 'ABC', 'Alpha Beta Charlie');",
            )
            .unwrap();
        }
        let state = AppState::new(backend);
        Router::new()
            .route(
                "/CodeSystem/$validate-code",
                post(validate_code_handler::<SqliteTerminologyBackend>),
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
    async fn valid_code_returns_true() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/cs"},
                {"name": "code", "valueCode": "ABC"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result_param = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result_param["valueBoolean"], true);
    }

    #[tokio::test]
    async fn valid_code_returns_display() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/cs"},
                {"name": "code", "valueCode": "ABC"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        let display_param = params.iter().find(|p| p["name"] == "display").unwrap();
        assert_eq!(display_param["valueString"], "Alpha Beta Charlie");
    }

    #[tokio::test]
    async fn system_param_rejected_with_400() {
        // FHIR spec requires `url`; sending `system` is not accepted.
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://example.org/cs"},
                {"name": "code", "valueCode": "ABC"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn unknown_code_returns_false() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/cs"},
                {"name": "code", "valueCode": "NOPE"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result_param = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result_param["valueBoolean"], false);
    }

    #[tokio::test]
    async fn unknown_system_returns_false() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://unknown.org/cs"},
                {"name": "code", "valueCode": "ABC"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result_param = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result_param["valueBoolean"], false);
    }

    #[tokio::test]
    async fn display_match_has_no_message() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/cs"},
                {"name": "code", "valueCode": "ABC"},
                {"name": "display", "valueString": "Alpha Beta Charlie"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        assert!(
            params.iter().all(|p| p["name"] != "message"),
            "no message expected when display matches"
        );
    }

    #[tokio::test]
    async fn display_mismatch_returns_false_with_message() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/cs"},
                {"name": "code", "valueCode": "ABC"},
                {"name": "display", "valueString": "Wrong Display"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        let result_param = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(
            result_param["valueBoolean"], false,
            "display mismatch makes result=false per FHIR spec"
        );

        let has_message = params.iter().any(|p| p["name"] == "message");
        assert!(has_message, "message expected for display mismatch");
    }

    #[tokio::test]
    async fn missing_url_returns_400() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "code", "valueCode": "ABC"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn missing_code_returns_400() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/cs"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn wrong_resource_type_returns_400() {
        let app = make_app();
        let body = json!({
            "resourceType": "CodeSystem",
            "parameter": []
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 400);
    }

    // ── ValueSet/$validate-code tests ──────────────────────────────────────────

    fn make_vs_app() -> Router {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();

        // Seed directly via SQL (same pattern as other operation handler tests).
        {
            let conn = backend.pool().get().unwrap();
            conn.execute_batch(
                "INSERT INTO code_systems
                     (id, url, version, name, status, content, created_at, updated_at)
                 VALUES ('cs-vs', 'http://example.org/cs', '1.0', 'TestCS',
                         'active', 'complete', '2024-01-01', '2024-01-01');

                 INSERT INTO concepts (id, system_id, code, display)
                 VALUES (1, 'cs-vs', 'A', 'Alpha'),
                        (2, 'cs-vs', 'B', 'Beta'),
                        (3, 'cs-vs', 'C', 'Gamma');

                 INSERT INTO value_sets
                     (id, url, name, status, compose_json, created_at, updated_at)
                 VALUES ('vs-main', 'http://example.org/vs', 'TestVS', 'active',
                         '{\"include\":[{\"system\":\"http://example.org/cs\",\"concept\":[{\"code\":\"A\"},{\"code\":\"B\"}]}]}',
                         '2024-01-01', '2024-01-01');",
            )
            .unwrap();
        }

        let state = AppState::new(backend);
        Router::new()
            .route(
                "/ValueSet/$validate-code",
                post(vs_validate_code_handler::<SqliteTerminologyBackend>),
            )
            .with_state(state)
    }

    #[tokio::test]
    async fn vs_code_in_set_returns_true() {
        let app = make_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs" },
                { "name": "code", "valueCode": "A" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], true);
    }

    #[tokio::test]
    async fn vs_code_not_in_set_returns_false() {
        let app = make_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs" },
                { "name": "code", "valueCode": "C" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], false);
    }

    #[tokio::test]
    async fn vs_missing_url_returns_400() {
        let app = make_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "code", "valueCode": "A" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn vs_missing_code_returns_400() {
        let app = make_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn vs_unknown_value_set_returns_false() {
        let app = make_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://unknown.org/vs" },
                { "name": "code", "valueCode": "A" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], false);
    }

    #[tokio::test]
    async fn vs_returns_display_for_valid_code() {
        let app = make_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs" },
                { "name": "code", "valueCode": "A" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        let json = body_json(resp).await;

        let params = json["parameter"].as_array().unwrap();
        let display = params.iter().find(|p| p["name"] == "display").unwrap();
        assert_eq!(display["valueString"], "Alpha");
    }

    // ── valueCoding input ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn cs_validate_coding_valid_returns_true() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "coding",
                    "valueCoding": {
                        "system": "http://example.org/cs",
                        "code": "ABC",
                        "display": "Alpha Beta Charlie"
                    }
                }
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], true);
    }

    #[tokio::test]
    async fn cs_validate_coding_unknown_code_returns_false() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "coding",
                "valueCoding": {"system": "http://example.org/cs", "code": "UNKNOWN"}
            }]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], false);
    }

    #[tokio::test]
    async fn vs_validate_coding_in_set_returns_true() {
        let app = make_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/vs"},
                {
                    "name": "coding",
                    "valueCoding": {
                        "system": "http://example.org/cs",
                        "code": "A"
                    }
                }
            ]
        });

        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], true);
    }

    // ── valueCodeableConcept input ────────────────────────────────────────────

    #[tokio::test]
    async fn cs_validate_codeable_concept_one_match_returns_true() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "codeableConcept",
                "valueCodeableConcept": {
                    "coding": [
                        {"system": "http://other.org/cs", "code": "NOPE"},
                        {"system": "http://example.org/cs", "code": "ABC"}
                    ]
                }
            }]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], true);
    }

    #[tokio::test]
    async fn cs_validate_codeable_concept_no_match_returns_false() {
        let app = make_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "codeableConcept",
                "valueCodeableConcept": {
                    "coding": [
                        {"system": "http://example.org/cs", "code": "X"},
                        {"system": "http://example.org/cs", "code": "Y"}
                    ]
                }
            }]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], false);
    }

    #[tokio::test]
    async fn vs_validate_codeable_concept_one_match_returns_true() {
        let app = make_vs_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/vs"},
                {
                    "name": "codeableConcept",
                    "valueCodeableConcept": {
                        "coding": [
                            {"system": "http://example.org/cs", "code": "C"}, // not in VS
                            {"system": "http://example.org/cs", "code": "A"}  // in VS
                        ]
                    }
                }
            ]
        });

        let resp = post_json(app, "/ValueSet/$validate-code", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let result = params.iter().find(|p| p["name"] == "result").unwrap();
        assert_eq!(result["valueBoolean"], true);
    }

    #[tokio::test]
    async fn no_input_param_returns_400() {
        let app = make_app();
        // No code, coding, or codeableConcept — should be rejected
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "url", "valueUri": "http://example.org/cs"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$validate-code", body).await;
        assert_eq!(resp.status(), 400);
    }
}
