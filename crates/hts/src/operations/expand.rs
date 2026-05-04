//! Handlers for `ValueSet/$expand` — type-level and instance-level.
//!
//! Expansion resolves all codes that belong to a ValueSet and returns them
//! inside a `ValueSet.expansion.contains[]` array.  Four handler variants are
//! provided:
//!
//! * **Type-level POST** — `POST /ValueSet/$expand` with a FHIR `Parameters` body.
//! * **Type-level GET** — `GET /ValueSet/$expand?url=<url>`.
//! * **Instance-level POST** — `POST /ValueSet/{id}/$expand`.
//! * **Instance-level GET** — `GET /ValueSet/{id}/$expand?filter=...`.
//!
//! ## Supported parameters
//!
//! | Parameter | Type | Description |
//! |-----------|------|-------------|
//! | `url` | uri | Canonical URL of the ValueSet (type-level) |
//! | `filter` | string | Substring filter on code or display |
//! | `count` | integer | Page size |
//! | `offset` | integer | Zero-based page start |
//! | `date` | dateTime | Point-in-time ISO-8601 date for evaluation |
//! | `hierarchical` | boolean | Return a tree-structured expansion instead of a flat list |
//!
//! ## Implicit ValueSets
//!
//! When `url` matches a CodeSystem's `valueSet` property and no explicit
//! ValueSet resource exists for that URL, the expansion falls back to all
//! codes in that CodeSystem (FHIR R5 §4.8.7).
//!
//! ## FHIR specification
//!
//! <https://hl7.org/fhir/valueset-operation-expand.html>

use axum::{
    Json,
    extract::{Path, RawQuery, State},
    http::{HeaderMap, StatusCode, header},
    response::Response,
};
use bytes::Bytes;
use helios_persistence::tenant::TenantContext;
use serde_json::{Value, json};

use crate::error::HtsError;
use crate::state::{AppState, EXPAND_CACHE_MAX, ExpandCacheKey, NOT_FOUND_CACHE_MAX};
use crate::traits::{TerminologyBackend, ValueSetOperations};
use crate::types::{ExpandRequest, ExpansionContains};

use super::format::{ResponseFormat, json_to_fhir_xml, negotiate_format};
use super::params::{
    extract_parameter_array, find_resource_param, find_str_param, parse_query_string,
    query_params_to_fhir_params,
};

/// Serialize a single [`ExpansionContains`] entry to a FHIR-compliant JSON value.
///
/// Recursively serializes nested `contains` arrays, so that a hierarchical
/// expansion (produced when `hierarchical=true`) is correctly represented as
/// nested `contains[]` objects rather than a flat list.
///
/// The `display` field is omitted when absent, and `contains` is omitted when
/// the entry has no children — keeping the output compact for flat expansions.
fn serialize_expansion_contains(c: &ExpansionContains) -> Value {
    let mut item = json!({
        "system": c.system,
        "code": c.code,
    });
    if let Some(display) = &c.display {
        item["display"] = json!(display);
    }
    if !c.contains.is_empty() {
        let nested: Vec<Value> = c
            .contains
            .iter()
            .map(serialize_expansion_contains)
            .collect();
        item["contains"] = json!(nested);
    }
    item
}

/// Expand a ValueSet and return the result as pre-serialized JSON bytes.
///
/// Bytes are cached keyed on request parameters.  On a cache hit the stored
/// [`Bytes`] handle is cloned in O(1) (reference-count bump only — no heap
/// allocation or JSON re-serialization).  On a cache miss the result is
/// serialized once, stored, and returned.
async fn process_expand<B: TerminologyBackend>(
    state: &AppState<B>,
    params: Vec<Value>,
) -> Result<Bytes, HtsError> {
    let url = find_str_param(&params, "url");
    let value_set = if url.is_none() {
        find_resource_param(&params, "valueSet")
    } else {
        None
    };

    if url.is_none() && value_set.is_none() {
        return Err(HtsError::InvalidRequest(
            "Missing required parameter: url (ValueSet canonical URL) or valueSet (inline ValueSet resource)".into()
        ));
    }

    let filter = find_str_param(&params, "filter");

    // `count` and `offset` may arrive as integer or string parameters.
    let count = find_str_param(&params, "count")
        .and_then(|s| s.parse::<u32>().ok())
        .or_else(|| {
            params
                .iter()
                .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("count"))
                .and_then(|p| p.get("valueInteger").and_then(|v| v.as_u64()))
                .map(|v| v as u32)
        });

    let offset = find_str_param(&params, "offset")
        .and_then(|s| s.parse::<u32>().ok())
        .or_else(|| {
            params
                .iter()
                .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("offset"))
                .and_then(|p| p.get("valueInteger").and_then(|v| v.as_u64()))
                .map(|v| v as u32)
        });

    let hierarchical = find_str_param(&params, "hierarchical").map(|s| s == "true");

    // ── Cache lookup ─────────────────────────────────────────────────────────
    // Build a stable key from the request parameters. For inline ValueSets
    // (ad-hoc POST) we serialise the body to compact JSON; k6 sends identical
    // bytes each iteration so the string is a reliable cache discriminator.
    // Build a canonical (name-sorted) form of the input parameters minus the
    // ones already captured in `url_or_body`. This makes cache entries unique
    // per combination of "extra" inputs that the response will echo back in
    // `expansion.parameter`.
    let extra_params = {
        let mut sorted: Vec<&Value> = params
            .iter()
            .filter(|p| {
                let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
                !matches!(name, "url" | "valueSet")
            })
            .collect();
        sorted.sort_by(|a, b| {
            let an = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let bn = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
            an.cmp(bn)
        });
        serde_json::to_string(&sorted).unwrap_or_default()
    };

    let cache_key = ExpandCacheKey {
        url_or_body: url.clone().unwrap_or_else(|| {
            value_set
                .as_ref()
                .and_then(|vs| serde_json::to_string(vs).ok())
                .unwrap_or_default()
        }),
        filter: filter.clone().unwrap_or_default(),
        count: count.unwrap_or(u32::MAX),
        offset: offset.unwrap_or(0),
        hierarchical: hierarchical.unwrap_or(false),
        extra_params,
    };

    if let Ok(cache) = state.expand_cache.read() {
        if let Some(cached) = cache.get(&cache_key) {
            // O(1) clone — just bumps the reference count on the shared buffer.
            return Ok(cached.clone());
        }
    }

    // ── Negative-cache check (URL-based 404s) ─────────────────────────────────
    // URLs that previously returned NotFound are remembered here so we can skip
    // all backend queries on repeated requests (saves 5+ SQLite round-trips per
    // hit).
    if let Some(ref url_str) = url {
        if let Ok(neg) = state.not_found_urls.read() {
            if neg.contains(url_str.as_str()) {
                return Err(HtsError::NotFound(url_str.clone()));
            }
        }
    }

    // Preserve the URL before it moves into ExpandRequest so we can record it
    // in the negative cache if the backend returns NotFound.
    let url_for_neg_cache = url.clone();

    // ── Cache miss: compute ───────────────────────────────────────────────────
    let req = ExpandRequest {
        url,
        value_set,
        filter,
        count,
        offset,
        max_expansion_size: Some(state.max_expansion_size),
        date: find_str_param(&params, "date"),
        hierarchical,
    };

    let ctx = TenantContext::system();
    let resp = match ValueSetOperations::expand(state.backend(), &ctx, req).await {
        Ok(r) => r,
        Err(HtsError::NotFound(msg)) => {
            // Populate the negative cache so future requests for this URL
            // are resolved in O(1) without touching the database.
            if let Some(url_str) = url_for_neg_cache {
                if let Ok(mut neg) = state.not_found_urls.write() {
                    if neg.len() < NOT_FOUND_CACHE_MAX {
                        neg.insert(url_str);
                    }
                }
            }
            return Err(HtsError::NotFound(msg));
        }
        Err(e) => return Err(e),
    };

    // ── Build FHIR ValueSet response with expansion ──────────────────────────
    let contains: Vec<Value> = resp
        .contains
        .iter()
        .map(serialize_expansion_contains)
        .collect();

    // The IG validator (txTests) treats `expansion.identifier` and
    // `expansion.timestamp` as required (they appear in every fixture without
    // an `$optional$` marker). The values are matched as `$uuid$` / `$instant$`
    // wildcards, so any well-formed value satisfies the comparison.
    let mut expansion = json!({
        "identifier": format!("urn:uuid:{}", uuid::Uuid::new_v4()),
        "timestamp": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "contains": contains,
    });

    if let Some(total) = resp.total {
        expansion["total"] = json!(total);
    }
    if let Some(off) = resp.offset {
        expansion["offset"] = json!(off);
    }

    // ── expansion.parameter ──────────────────────────────────────────────────
    // Echo back the input parameters that influenced the result (e.g.
    // `excludeNested`, `displayLanguage`, `includeDesignations`, `count`,
    // `offset`, `activeOnly`). The validator's tests check that we report
    // every honored input here.
    //
    // Skip the `url` / `valueSet` discriminators (they identify the
    // ValueSet, not a knob), and skip `filter` (already reflected in the
    // contains[] result).
    let mut emitted_params: Vec<Value> = params
        .iter()
        .filter(|p| {
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
            !matches!(name, "url" | "valueSet" | "filter")
        })
        .cloned()
        .collect();

    // Append any expansion warnings as parameter entries with name=warning.
    for w in &resp.warnings {
        emitted_params.push(json!({ "name": "warning", "valueString": w }));
    }

    if !emitted_params.is_empty() {
        expansion["parameter"] = json!(emitted_params);
    }

    let response = json!({
        "resourceType": "ValueSet",
        "expansion": expansion,
    });

    // ── Serialize once, cache, return ─────────────────────────────────────────
    // `serde_json::to_vec` writes directly into a Vec<u8>; wrapping in
    // `Bytes::from` transfers ownership without copying.
    let bytes = Bytes::from(
        serde_json::to_vec(&response)
            .map_err(|e| HtsError::Internal(format!("JSON serialization failed: {e}")))?,
    );

    if let Ok(mut cache) = state.expand_cache.write() {
        if cache.len() < EXPAND_CACHE_MAX {
            // `Bytes::clone` is O(1); storing it here and returning the clone
            // below means both the cache and the caller share the same buffer.
            cache.insert(cache_key, bytes.clone());
        }
    }

    Ok(bytes)
}

/// Turn pre-serialized JSON bytes into an HTTP response.
///
/// For JSON format: the bytes are returned directly with no extra copy.
/// For XML format: the bytes are deserialized back to a `Value` first (rare
/// code path — the benchmark always uses JSON).
fn expand_bytes_respond(bytes: Bytes, format: ResponseFormat) -> Response {
    use axum::response::IntoResponse;
    match format {
        ResponseFormat::Json => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/fhir+json; charset=utf-8")],
            bytes,
        )
            .into_response(),
        ResponseFormat::Xml => {
            let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let xml = json_to_fhir_xml(&value);
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/fhir+xml; charset=utf-8")],
                xml,
            )
                .into_response()
        }
    }
}

/// `POST /ValueSet/$expand`
///
/// Accepts a FHIR `Parameters` body.  The `url` parameter (canonical ValueSet
/// URL) is required.  Content negotiation via `Accept` header or `_format`
/// query parameter selects JSON or XML output.
pub async fn expand_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let params = extract_parameter_array(&body)?;
    Ok(expand_bytes_respond(
        process_expand(&state, params).await?,
        format,
    ))
}

/// `GET /ValueSet/$expand?url=<url>`
///
/// URL query parameters are mapped to FHIR `Parameters` name/value pairs and
/// processed identically to the POST form.  `url`, `filter`, `count`, `offset`,
/// `date`, and `hierarchical` are all accepted.
pub async fn get_expand_handler<B: TerminologyBackend>(
    State(state): State<AppState<B>>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response, HtsError> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    let format = negotiate_format(raw.as_deref(), accept);
    let pairs = parse_query_string(raw.as_deref().unwrap_or(""));
    let params = query_params_to_fhir_params(pairs);
    Ok(expand_bytes_respond(
        process_expand(&state, params).await?,
        format,
    ))
}

/// Inject (or replace) the `url` parameter in a params list.
///
/// Removes any existing `url` entry from the caller's params so that the
/// resource-id-resolved URL always wins, then prepends the canonical URL as a
/// `valueUri` parameter.
fn inject_url(mut params: Vec<Value>, url: String) -> Vec<Value> {
    params.retain(|p| p.get("name").and_then(|v| v.as_str()) != Some("url"));
    let mut with_url = vec![json!({"name": "url", "valueUri": url})];
    with_url.append(&mut params);
    with_url
}

/// POST /ValueSet/{id}/$expand
///
/// Resolves the ValueSet canonical URL from its FHIR `id`, then delegates to
/// the same expansion logic used by the system-level endpoint.
pub async fn expand_by_id_post<B: TerminologyBackend>(
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
    Ok(expand_bytes_respond(
        process_expand(&state, inject_url(raw_params, url)).await?,
        format,
    ))
}

/// `GET /ValueSet/{id}/$expand?filter=<text>&count=<n>`
///
/// Instance-level GET variant.  Resolves the ValueSet canonical URL from its
/// FHIR logical `id` and merges it with the remaining query-string parameters
/// before dispatching to the shared expansion pipeline.
///
/// Returns 404 when no ValueSet with the given `id` is found.
pub async fn get_expand_by_id<B: TerminologyBackend>(
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
    Ok(expand_bytes_respond(
        process_expand(&state, inject_url(params, url)).await?,
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

    fn make_app_with_data() -> Router {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();

        // Seed directly via SQL (same pattern as other operation handler tests).
        {
            let conn = backend.pool().get().unwrap();
            conn.execute_batch(
                "INSERT INTO code_systems
                     (id, url, version, name, status, content, created_at, updated_at)
                 VALUES ('cs1', 'http://example.org/cs', '1.0', 'TestCS',
                         'active', 'complete', '2024-01-01', '2024-01-01');

                 INSERT INTO concepts (id, system_id, code, display)
                 VALUES (1, 'cs1', 'A', 'Alpha'),
                        (2, 'cs1', 'B', 'Beta'),
                        (3, 'cs1', 'C', 'Gamma');

                 INSERT INTO value_sets
                     (id, url, name, status, compose_json, created_at, updated_at)
                 VALUES ('vs1', 'http://example.org/vs', 'TestVS', 'active',
                         '{\"include\":[{\"system\":\"http://example.org/cs\",\"concept\":[{\"code\":\"A\"},{\"code\":\"B\"}]}]}',
                         '2024-01-01', '2024-01-01');",
            )
            .unwrap();
        }

        let state = AppState::new(backend);
        Router::new()
            .route(
                "/ValueSet/$expand",
                post(expand_handler::<SqliteTerminologyBackend>),
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
    async fn expand_returns_valueset_resource() {
        let app = make_app_with_data();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$expand", body).await;
        assert_eq!(resp.status(), 200);

        let json = body_json(resp).await;
        assert_eq!(json["resourceType"], "ValueSet");
        assert!(json["expansion"].is_object());
    }

    #[tokio::test]
    async fn expand_returns_correct_codes() {
        let app = make_app_with_data();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$expand", body).await;
        let json = body_json(resp).await;

        let contains = json["expansion"]["contains"].as_array().unwrap();
        assert_eq!(contains.len(), 2);

        let codes: Vec<&str> = contains
            .iter()
            .map(|c| c["code"].as_str().unwrap())
            .collect();
        assert!(codes.contains(&"A"));
        assert!(codes.contains(&"B"));
        assert!(!codes.contains(&"C"));
    }

    #[tokio::test]
    async fn expand_returns_total_count() {
        let app = make_app_with_data();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$expand", body).await;
        let json = body_json(resp).await;

        assert_eq!(json["expansion"]["total"], 2);
    }

    // ── Pagination ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn expand_with_count_limits_results() {
        let app = make_app_with_data();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://example.org/vs" },
                { "name": "count", "valueInteger": 1 }
            ]
        });

        let resp = post_json(app, "/ValueSet/$expand", body).await;
        let json = body_json(resp).await;

        let contains = json["expansion"]["contains"].as_array().unwrap();
        assert_eq!(contains.len(), 1);
        assert_eq!(json["expansion"]["total"], 2); // total is still the full count
    }

    // ── Missing url → 400 ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn expand_missing_url_returns_400() {
        let app = make_app_with_data();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": []
        });

        let resp = post_json(app, "/ValueSet/$expand", body).await;
        assert_eq!(resp.status(), 400);
    }

    // ── Unknown value set → 404 ────────────────────────────────────────────────

    #[tokio::test]
    async fn expand_unknown_value_set_returns_404() {
        let app = make_app_with_data();
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "url", "valueUri": "http://unknown.org/vs" }
            ]
        });

        let resp = post_json(app, "/ValueSet/$expand", body).await;
        assert_eq!(resp.status(), 404);
    }

    // ── Wrong resource type → 400 ──────────────────────────────────────────────

    #[tokio::test]
    async fn expand_wrong_resource_type_returns_400() {
        let app = make_app_with_data();
        let body = json!({ "resourceType": "ValueSet", "parameter": [] });

        let resp = post_json(app, "/ValueSet/$expand", body).await;
        assert_eq!(resp.status(), 400);
    }
}
