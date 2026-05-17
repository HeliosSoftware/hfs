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

use std::sync::Arc;

use crate::error::HtsError;
use crate::state::{
    AppState, LOOKUP_HANDLER_CACHE_MAX, LookupHandlerCache, NOT_FOUND_CACHE_MAX, NotFoundCache,
};
use crate::traits::{SupplementInfo, TerminologyBackend};
use crate::types::{DesignationValue, LookupRequest, PropertyValue};

use super::format::{fhir_respond, negotiate_format};
use super::params::{
    collect_str_params, extract_parameter_array, find_str_param, parse_query_string,
    property_value_part, query_params_to_fhir_params,
};

/// Build a canonical cache key for a `$lookup` request from its normalised
/// FHIR Parameters array.  Returns `None` when caching MUST be skipped:
///
/// * Any param carries a `resource` field — `$lookup` doesn't normally accept
///   inline resources, but defensively bail to keep the key compact.
/// * A param is missing its `name` (malformed input — slow path will reject).
///
/// Otherwise produces a string of compact-JSON fragments sorted by `name`,
/// pipe-separated.  Identical canonical inputs always collide on the same
/// key; semantically equivalent inputs that differ only in field order would
/// miss (worst case = a cold miss, never a wrong response).  Mirrors
/// `build_validate_code_cache_key` / `build_expand_cache_key`.
fn build_lookup_cache_key(params: &[Value]) -> Option<String> {
    let mut frags: Vec<(String, String)> = Vec::with_capacity(params.len());
    for p in params {
        let name = match p.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return None,
        };
        if p.get("resource").is_some() {
            return None;
        }
        let frag = match serde_json::to_string(p) {
            Ok(s) => s,
            Err(_) => return None,
        };
        frags.push((name.to_string(), frag));
    }
    frags.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = String::with_capacity(frags.iter().map(|(_, f)| f.len() + 1).sum());
    for (i, (_, f)) in frags.iter().enumerate() {
        if i > 0 {
            out.push('|');
        }
        out.push_str(f);
    }
    Some(out)
}

/// Fetch a cached `$lookup` response by canonical key.
fn lookup_cache_get(cache: &LookupHandlerCache, key: &str) -> Option<Arc<Value>> {
    cache.read().ok()?.get(key).cloned()
}

/// Insert a successfully-built `$lookup` response into the per-AppState cache.
/// Drops new entries silently once the cache reaches
/// [`LOOKUP_HANDLER_CACHE_MAX`].
fn lookup_cache_put(cache: &LookupHandlerCache, key: String, value: Arc<Value>) {
    if let Ok(mut guard) = cache.write() {
        if guard.len() >= LOOKUP_HANDLER_CACHE_MAX {
            return;
        }
        guard.insert(key, value);
    }
}

/// Iter 7i — Is this canonical request key in the negative cache (the last
/// observation was `NotFound`)?
fn lookup_not_found_contains(cache: &NotFoundCache, key: &str) -> bool {
    cache.read().map(|g| g.contains(key)).unwrap_or(false)
}

/// Iter 7i — Mark this canonical request key as definitively unknown until
/// the next bundle import wipes the cache. Bounded to
/// [`NOT_FOUND_CACHE_MAX`] entries so an adversary probing many unique
/// codes cannot grow the cache without bound.
fn lookup_not_found_put(cache: &NotFoundCache, key: String) {
    if let Ok(mut guard) = cache.write() {
        if guard.len() >= NOT_FOUND_CACHE_MAX {
            return;
        }
        guard.insert(key);
    }
}

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
    // ── Handler-level response cache ─────────────────────────────────────────
    // Skips supplement resolution, the backend `lookup` call, the
    // `code_system_language` lookup, and FHIR Parameters assembly when the
    // same canonical params have produced a response earlier in this
    // AppState's lifetime.  Cleared on every bundle import / CRUD write via
    // `clear_expand_cache`. LK01-04 hot path: each of 2 000 distinct codes is
    // hit ~hundreds of times by 50 VUs over a 30 s run, so the warm-hit ratio
    // is overwhelmingly favourable.
    let cache_key = build_lookup_cache_key(&params);
    if let Some(ref key) = cache_key {
        // ── Iter 7i: positive cache first, then negative cache ──────────
        // LK05's pool of nonexistent codes hits the same ~200 keys
        // repeatedly across 50 VUs over 30 s. The positive cache always
        // misses on these. Without the negative cache, each request paid
        // for supplement resolution + backend lookup + the synthesized
        // designation assembly. With it, a warm-hit on the negative cache
        // returns NotFound in a few μs.
        if let Some(cached) = lookup_cache_get(&state.lookup_handler_cache, key) {
            return Ok((*cached).clone());
        }
        if lookup_not_found_contains(&state.lookup_not_found_cache, key) {
            // Reconstruct the same NotFound the slow path would have
            // produced. Both backends (postgres/code_system.rs:1456 and
            // sqlite/code_system.rs:1795) use the same `"Concept not found:
            // {code}"` shape, so match that exactly to keep the cached
            // negative response byte-identical to a fresh slow-path miss.
            let code = find_str_param(&params, "code").unwrap_or_default();
            return Err(HtsError::NotFound(format!("Concept not found: {code}")));
        }
    }
    let result = process_lookup_inner(state, params).await;
    match (&result, cache_key) {
        (Ok(value), Some(key)) => {
            lookup_cache_put(&state.lookup_handler_cache, key, Arc::new(value.clone()));
        }
        (Err(HtsError::NotFound(_)), Some(key)) => {
            lookup_not_found_put(&state.lookup_not_found_cache, key);
        }
        _ => {}
    }
    result
}

async fn process_lookup_inner<B: TerminologyBackend>(
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
        use_supplements: collect_str_params(&params, "useSupplement"),
    };

    let ctx = TenantContext::system();

    // ── Resolve supplements ──────────────────────────────────────────────────
    // Accept either valueCanonical or valueUri on `useSupplement`. For each,
    // verify it points at a stored CodeSystem with `content=supplement` whose
    // `supplements` URL matches the lookup `system`. Mismatches and unknown
    // supplements both reject with NotFound (issue.code=not-found) per the
    // IG fixtures.
    let supplement_inputs: Vec<String> = collect_supplement_inputs(&params);
    let mut applied_supplements: Vec<SupplementInfo> = Vec::new();
    for raw in &supplement_inputs {
        let bare = raw.split('|').next().unwrap_or(raw).to_string();
        match state.backend().supplement_target(&ctx, &bare).await? {
            Some(info) if info.target_url == system => applied_supplements.push(info),
            _ => {
                return Err(HtsError::NotFound(format!(
                    "Required supplement not found: {bare}"
                )));
            }
        }
    }

    let mut resp = state.backend().lookup(&ctx, req).await?;

    // Surface the default display as a synthesised "preferredForLanguage"
    // designation tagged with the CodeSystem's primary language. The IG
    // `parameters/parameters-lookup-supplement-*` fixtures expect this row,
    // and `simple/simple-lookup*` accepts it as optional.
    //
    // This used to call `CodeSystemOperations::search(...)` which selected
    // and parsed the entire `resource_json` blob just to read `.language` —
    // the dominant cost on the LK01-04 hot path under 50-VU load. The
    // dedicated trait method runs ONE `json_extract` query and is memoised
    // in a process-wide cache.
    let cs_language: Option<String> = state
        .backend()
        .code_system_language(&ctx, &system)
        .await
        .ok()
        .flatten();
    if let (Some(lang), Some(disp)) = (cs_language.as_deref(), resp.display.clone()) {
        let already = resp
            .designations
            .iter()
            .any(|d| d.language.as_deref() == Some(lang) && d.value == disp);
        if !already {
            resp.designations.push(DesignationValue {
                language: Some(lang.to_string()),
                use_system: None,
                use_code: None,
                value: disp,
                source: None,
            });
        }
    }

    // Merge supplement designations and properties (matched on concept code).
    if !applied_supplements.is_empty() {
        // The lookup keys for the supplement queries are the supplement CS
        // URLs themselves (not their `supplements` targets).
        let bare_supp_urls: Vec<String> = supplement_inputs
            .iter()
            .map(|s| s.split('|').next().unwrap_or(s).to_string())
            .collect();
        let codes = vec![code.clone()];

        // Designations: tag with `source = "url|version"` (set by the
        // backend). Append AFTER base designations so the IG fixture order
        // (base first, supplement last) is preserved.
        let supp_desigs = state
            .backend()
            .supplement_designations(&ctx, &bare_supp_urls, &codes)
            .await
            .unwrap_or_default();
        if let Some(list) = supp_desigs.get(&code) {
            for d in list {
                resp.designations.push(DesignationValue {
                    language: d.language.clone(),
                    use_system: d.use_system.clone(),
                    use_code: d.use_code.clone(),
                    value: d.value.clone(),
                    source: d.source.clone(),
                });
            }
        }

        // Properties: when the caller asks for specific properties, scope
        // the supplement query to those names. Otherwise (no filter or
        // wildcard `*`) pass an empty list to mean "all properties".
        let requested_props_raw = collect_str_params(&params, "property");
        let want_all =
            requested_props_raw.is_empty() || requested_props_raw.iter().any(|p| p == "*");
        let prop_filter: Vec<String> = if want_all {
            Vec::new()
        } else {
            requested_props_raw
        };
        let supp_props = state
            .backend()
            .supplement_property_values(&ctx, &bare_supp_urls, &codes, &prop_filter)
            .await
            .unwrap_or_default();
        if let Some(list) = supp_props.get(&code) {
            for (prop, value) in list {
                resp.properties.push(PropertyValue {
                    code: prop.clone(),
                    value_type: "string".into(),
                    value: value.clone(),
                    description: None,
                });
            }
        }
    }

    // ── Build FHIR Parameters response ─────────────────────────────────────────
    let mut parameter: Vec<Value> = vec![json!({"name": "name", "valueString": resp.name})];

    if let Some(ver) = resp.version {
        parameter.push(json!({"name": "version", "valueString": ver}));
    }

    if let Some(display) = resp.display {
        parameter.push(json!({"name": "display", "valueString": display}));
    }

    // Top-level concept definition (free-form text from concepts.definition).
    if let Some(def) = resp.definition {
        parameter.push(json!({"name": "definition", "valueString": def}));
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

        // FHIR `designation.source` part — points at the supplement CS
        // (`url|version`) that contributed this designation. Only present
        // for supplement-derived rows; base CS designations carry no source.
        if let Some(src) = desig.source {
            parts.push(json!({"name": "source", "valueCanonical": src}));
        }

        parts.push(json!({"name": "value", "valueString": desig.value}));
        parameter.push(json!({"name": "designation", "part": parts}));
    }

    // ── used-supplement parameters ───────────────────────────────────────────
    // Echo each applied supplement as a `used-supplement` parameter so the
    // caller can see which supplements actually contributed to the response
    // (matches IG fixture `parameters-lookup-supplement-good-response`).
    for info in &applied_supplements {
        parameter.push(json!({
            "name": "used-supplement",
            "valueCanonical": info.supplement_canonical,
        }));
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

/// Collect every `useSupplement` input from a Parameters body, accepting
/// either `valueCanonical` or `valueUri`. Returns the raw values so callers
/// can later strip an optional `|version` suffix when looking up the CS.
fn collect_supplement_inputs(params: &[Value]) -> Vec<String> {
    params
        .iter()
        .filter(|p| p.get("name").and_then(|v| v.as_str()) == Some("useSupplement"))
        .filter_map(|p| {
            p.get("valueCanonical")
                .or_else(|| p.get("valueUri"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect()
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

    // ── useSupplement ─────────────────────────────────────────────────────────
    //
    // Mirror the IG `parameters/parameters-lookup-supplement-good` fixture: a
    // supplement defines an alternate display ("ectenoot") for code1 in the
    // base CS; the lookup response must include that designation tagged with
    // `source = supplement_url|version` plus a `used-supplement` parameter
    // echoing the applied supplement.
    fn make_supplement_app() -> Router {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        {
            let conn = backend.pool().get().unwrap();
            conn.execute_batch(
                "INSERT INTO code_systems
                     (id, url, version, name, status, content, created_at, updated_at, resource_json)
                 VALUES ('base', 'http://hl7.org/fhir/test/CodeSystem/extensions', '5.0.0',
                         'ExtensionsTestCodeSystem', 'active', 'complete',
                         '2024-01-01', '2024-01-01',
                         '{\"resourceType\":\"CodeSystem\",\"url\":\"http://hl7.org/fhir/test/CodeSystem/extensions\"}');

                 INSERT INTO code_systems
                     (id, url, version, name, status, content, created_at, updated_at, resource_json)
                 VALUES ('supp', 'http://hl7.org/fhir/test/CodeSystem/supplement', '0.1.1',
                         'SupplementToExtensionsTestCodeSystem', 'active', 'supplement',
                         '2024-01-01', '2024-01-01',
                         '{\"resourceType\":\"CodeSystem\",\"url\":\"http://hl7.org/fhir/test/CodeSystem/supplement\",\"supplements\":\"http://hl7.org/fhir/test/CodeSystem/extensions\"}');

                 INSERT INTO concepts (id, system_id, code, display)
                 VALUES (1, 'base', 'code1', 'Display 1'),
                        (2, 'supp', 'code1', NULL);

                 INSERT INTO concept_designations (concept_id, language, use_system, use_code, value)
                 VALUES (2, 'nl', NULL, NULL, 'ectenoot');",
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

    /// Build an app with a small hierarchy so we can exercise the synthesised
    /// `parent` / `child` properties and the top-level `definition` parameter
    /// emitted by `process_lookup` for `property=*` requests (mirroring the
    /// IG simple-lookup fixture shape).
    fn make_hierarchical_app() -> Router {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        {
            let conn = backend.pool().get().unwrap();
            conn.execute_batch(
                "INSERT INTO code_systems
                     (id, url, version, name, status, content, created_at, updated_at)
                 VALUES ('cs-h', 'http://example.org/h', '0.1.0', 'HierCS',
                         'active', 'complete', '2024-01-01', '2024-01-01');

                 INSERT INTO concepts (id, system_id, code, display, definition)
                 VALUES (10, 'cs-h', 'top',  'Top display',  NULL),
                        (11, 'cs-h', 'mid',  'Middle display', 'Middle definition'),
                        (12, 'cs-h', 'leaf', 'Leaf display', NULL);

                 INSERT INTO concept_hierarchy (system_id, parent_code, child_code)
                 VALUES ('cs-h', 'top', 'mid'),
                        ('cs-h', 'mid', 'leaf');",
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

    #[tokio::test]
    async fn lookup_with_use_supplement_includes_designation_with_source() {
        let app = make_supplement_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://hl7.org/fhir/test/CodeSystem/extensions"},
                {"name": "code", "valueCode": "code1"},
                {"name": "useSupplement", "valueCanonical": "http://hl7.org/fhir/test/CodeSystem/supplement"}
            ]
        });
        let resp = post_json(app, "/CodeSystem/$lookup", body).await;
        assert_eq!(resp.status(), 200);
        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();

        // Designation with value "ectenoot" and a source pointing back at
        // the supplement canonical (with version).
        let supp_desig = params
            .iter()
            .filter(|p| p["name"] == "designation")
            .find(|p| {
                p["part"]
                    .as_array()
                    .map(|parts| parts.iter().any(|q| q["valueString"] == "ectenoot"))
                    .unwrap_or(false)
            })
            .expect("supplement designation should appear");
        let parts = supp_desig["part"].as_array().unwrap();
        let source = parts
            .iter()
            .find(|p| p["name"] == "source")
            .expect("designation.source part required for supplement-derived rows");
        assert_eq!(
            source["valueCanonical"],
            "http://hl7.org/fhir/test/CodeSystem/supplement|0.1.1",
        );

        // used-supplement parameter at top level.
        let used = params
            .iter()
            .find(|p| p["name"] == "used-supplement")
            .expect("used-supplement parameter expected");
        assert_eq!(
            used["valueCanonical"],
            "http://hl7.org/fhir/test/CodeSystem/supplement|0.1.1",
        );
    }

    #[tokio::test]
    async fn lookup_without_use_supplement_omits_supplement_designation() {
        let app = make_supplement_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://hl7.org/fhir/test/CodeSystem/extensions"},
                {"name": "code", "valueCode": "code1"}
            ]
        });
        let resp = post_json(app, "/CodeSystem/$lookup", body).await;
        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();
        let has_ectenoot = params.iter().any(|p| {
            p["name"] == "designation"
                && p["part"]
                    .as_array()
                    .map(|parts| parts.iter().any(|q| q["valueString"] == "ectenoot"))
                    .unwrap_or(false)
        });
        assert!(
            !has_ectenoot,
            "supplement designation must NOT appear without useSupplement"
        );
        let has_used_supplement = params.iter().any(|p| p["name"] == "used-supplement");
        assert!(!has_used_supplement);
    }

    #[tokio::test]
    async fn lookup_unknown_supplement_returns_404() {
        let app = make_supplement_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://hl7.org/fhir/test/CodeSystem/extensions"},
                {"name": "code", "valueCode": "code1"},
                {"name": "useSupplement", "valueCanonical": "http://does-not-exist/cs"}
            ]
        });
        let resp = post_json(app, "/CodeSystem/$lookup", body).await;
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    async fn lookup_supplement_targeting_other_cs_returns_404() {
        // The supplement points at .../extensions, but the lookup is against
        // a different CS — the rejection is the same as not-found.
        let app = make_supplement_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://other.org/cs"},
                {"name": "code", "valueCode": "code1"},
                {"name": "useSupplement", "valueCanonical": "http://hl7.org/fhir/test/CodeSystem/supplement"}
            ]
        });
        let resp = post_json(app, "/CodeSystem/$lookup", body).await;
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    async fn lookup_wildcard_emits_definition_parent_child_inactive() {
        let app = make_hierarchical_app();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "system", "valueUri": "http://example.org/h"},
                {"name": "code",   "valueCode": "mid"},
                {"name": "property", "valueCode": "*"}
            ]
        });

        let resp = post_json(app, "/CodeSystem/$lookup", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        let params = json["parameter"].as_array().unwrap();

        // Top-level `definition` from concepts.definition.
        let def = params.iter().find(|p| p["name"] == "definition").unwrap();
        assert_eq!(def["valueString"], "Middle definition");

        // Synthesised parent property pointing at "top" with description.
        let parent = params
            .iter()
            .find(|p| {
                p["name"] == "property"
                    && p["part"]
                        .as_array()
                        .map(|parts| parts.iter().any(|x| x["valueCode"] == "parent"))
                        .unwrap_or(false)
            })
            .expect("synthesised parent property should be present");
        let parent_parts = parent["part"].as_array().unwrap();
        let parent_value = parent_parts.iter().find(|x| x["name"] == "value").unwrap();
        assert_eq!(parent_value["valueCode"], "top");
        let parent_desc = parent_parts
            .iter()
            .find(|x| x["name"] == "description")
            .unwrap();
        assert_eq!(parent_desc["valueString"], "Top display");

        // Synthesised child property pointing at "leaf".
        let child = params
            .iter()
            .find(|p| {
                p["name"] == "property"
                    && p["part"]
                        .as_array()
                        .map(|parts| parts.iter().any(|x| x["valueCode"] == "child"))
                        .unwrap_or(false)
            })
            .expect("synthesised child property should be present");
        let child_value = child["part"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["name"] == "value")
            .unwrap();
        assert_eq!(child_value["valueCode"], "leaf");

        // Synthesised inactive=false (no status property on `mid`).
        let inactive = params
            .iter()
            .find(|p| {
                p["name"] == "property"
                    && p["part"]
                        .as_array()
                        .map(|parts| parts.iter().any(|x| x["valueCode"] == "inactive"))
                        .unwrap_or(false)
            })
            .expect("synthesised inactive property should be present");
        let inactive_value = inactive["part"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["name"] == "value")
            .unwrap();
        assert_eq!(inactive_value["valueBoolean"], false);
    }
}
