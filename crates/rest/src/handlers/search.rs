//! Search interaction handler.
//!
//! Implements the FHIR [search interaction](https://hl7.org/fhir/http.html#search):
//! - `GET [base]/[type]?params` - Type-level search
//! - `POST [base]/[type]/_search` - Type-level search (POST)
//! - `GET [base]?params` - System-level search (all types)
//!
//! The search handler connects to the persistence layer's SearchProvider trait
//! to execute searches against the storage backend.

use axum::{
    extract::{Path, RawForm, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use helios_persistence::core::{MultiTypeSearchProvider, ResourceStorage, SearchProvider};
use helios_persistence::types::SearchBundle;
use tracing::{debug, warn};

use helios_fhir::FhirVersion;

use crate::error::{RestError, RestResult};
use crate::extractors::query_pairs::{last_value, parse_query_pairs};
use crate::extractors::{SearchParams, TenantExtractor, build_search_query};
use crate::middleware::content_type::{FhirFormat, negotiate_format};
use crate::responses::format_resource_response;
use crate::responses::subsetting::{SummaryMode, apply_elements, apply_summary};
use crate::state::AppState;
use crate::terminology::TerminologyServiceClient;

/// Handler for GET search.
///
/// Searches for resources of a specific type.
///
/// # HTTP Request
///
/// `GET [base]/[type]?params`
///
/// # Response
///
/// Returns a Bundle of type "searchset".
pub async fn search_get_handler<S>(
    State(state): State<AppState<S>>,
    Path(resource_type): Path<String>,
    tenant: TenantExtractor,
    req_headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    let pairs = parse_query_pairs(raw_query.as_deref());
    debug!(
        resource_type = %resource_type,
        tenant = %tenant.tenant_id(),
        params = ?pairs,
        "Processing search GET request"
    );

    let format_param = last_value(&pairs, "_format");
    let negotiated = negotiate_format(&req_headers, format_param.as_deref());

    execute_search(&state, tenant, &resource_type, pairs, negotiated.format).await
}

/// Handler for POST search.
///
/// Searches for resources using form-encoded parameters.
///
/// # HTTP Request
///
/// `POST [base]/[type]/_search`
///
/// This is useful when search parameters are too long for a GET URL.
pub async fn search_post_handler<S>(
    State(state): State<AppState<S>>,
    Path(resource_type): Path<String>,
    tenant: TenantExtractor,
    req_headers: HeaderMap,
    RawForm(form): RawForm,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    let body = String::from_utf8_lossy(form.as_ref());
    let pairs = parse_query_pairs(Some(&body));
    debug!(
        resource_type = %resource_type,
        tenant = %tenant.tenant_id(),
        params = ?pairs,
        "Processing search POST request"
    );

    let negotiated = negotiate_format(&req_headers, None);

    execute_search(&state, tenant, &resource_type, pairs, negotiated.format).await
}

/// Handler for system-level search.
///
/// Searches across all resource types.
///
/// # HTTP Request
///
/// `GET [base]?params`
pub async fn search_system_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    req_headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
) -> RestResult<Response>
where
    S: ResourceStorage + MultiTypeSearchProvider + Send + Sync,
{
    let pairs = parse_query_pairs(raw_query.as_deref());
    debug!(
        tenant = %tenant.tenant_id(),
        params = ?pairs,
        "Processing system-level search request"
    );

    let format_param = last_value(&pairs, "_format");
    let negotiated = negotiate_format(&req_headers, format_param.as_deref());

    execute_system_search(&state, tenant, pairs, negotiated.format).await
}

/// Executes a type-level search and returns a Bundle response.
async fn execute_search<S>(
    state: &AppState<S>,
    tenant: TenantExtractor,
    resource_type: &str,
    pairs: Vec<(String, String)>,
    format: FhirFormat,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    // `:not-in` requires negated value-set filtering, which no backend
    // implements. Reject it explicitly (501) regardless of whether a terminology
    // server is configured, rather than silently ignoring it (which would return
    // a superset of the intended results).
    if let Some((key, _)) = pairs.iter().find(|(k, _)| k.ends_with(":not-in")) {
        return Err(RestError::NotImplemented {
            feature: format!(
                "search modifier ':not-in' is not supported ({key}); \
                 use ':in' or remove this modifier"
            ),
        });
    }

    // Pre-process :in / :above / :below modifiers via the terminology server.
    let pairs = if let Some(ts_url) = state.terminology_server_url() {
        expand_terminology_params(pairs, ts_url).await?
    } else {
        pairs
    };

    let search_params = SearchParams::from_pairs(pairs);

    // Convert REST params to persistence SearchQuery. Scope the registry read
    // guard tightly so it doesn't span any await — parking_lot guards aren't
    // Send by default, which would make this async fn !Send.
    let mut query = {
        let registry = state.storage().search_param_registry().read();
        build_search_query(resource_type, &search_params, &registry)?
    };

    // Clamp page size to the configured default/maximum.
    let count = query
        .count
        .map(|c| c as usize)
        .unwrap_or(state.default_page_size())
        .min(state.max_page_size());
    query.count = Some(count as u32);

    // Resolve chained / reverse-chained (`_has`) parameters into an `_id` filter
    // via application-side joins, so any backend's `search()` can execute them.
    let query = if helios_persistence::search::query_has_chains(&query) {
        helios_persistence::search::resolve_chains(state.storage(), tenant.context(), &query)
            .await
            .map_err(|e| {
                warn!(error = %e, "Chained search resolution failed");
                RestError::from(e)
            })?
    } else {
        query
    };

    // Execute the search
    // Note: The search provider is responsible for resolving _include/_revinclude
    // directives that are part of the query. The result already contains included resources.
    let result = state
        .storage()
        .search(tenant.context(), &query)
        .await
        .map_err(|e| {
            warn!(error = %e, "Search failed");
            RestError::from(e)
        })?;

    // Build the self link URL
    let self_link = build_search_url(state.base_url(), resource_type, &search_params);

    // Convert result to FHIR Bundle
    let bundle = result.to_bundle(state.base_url(), &self_link);

    // Parse subsetting parameters
    let summary_mode = search_params
        .get("_summary")
        .and_then(|v| SummaryMode::parse(v));
    let elements: Option<Vec<&str>> = search_params
        .elements()
        .map(|e| e.iter().map(|s| s.as_str()).collect());

    debug!(
        resource_type = %resource_type,
        results = result.resources.len(),
        included = result.included.len(),
        summary = ?summary_mode,
        elements = ?elements,
        "Search completed"
    );

    // Get FHIR version from config for subsetting
    let fhir_version = state.config().default_fhir_version;

    let bundle_json =
        bundle_to_json_with_subsetting(bundle, summary_mode, elements.as_deref(), fhir_version);

    format_resource_response(StatusCode::OK, HeaderMap::new(), &bundle_json, format).map_err(|_| {
        RestError::InternalError {
            message: "Failed to serialize response".to_string(),
        }
    })
}

/// Executes a system-level search across all resource types.
#[allow(dead_code)]
async fn execute_system_search<S>(
    state: &AppState<S>,
    tenant: TenantExtractor,
    pairs: Vec<(String, String)>,
    format: FhirFormat,
) -> RestResult<Response>
where
    S: ResourceStorage + MultiTypeSearchProvider + Send + Sync,
{
    let search_params = SearchParams::from_pairs(pairs);

    // Get resource types from _type parameter (if specified)
    let type_param = search_params.get("_type").cloned();
    let resource_types: Vec<&str> = type_param
        .as_deref()
        .map(|t| t.split(',').map(|s| s.trim()).collect())
        .unwrap_or_default();

    // Build a search query (resource type doesn't matter much for system search).
    // Scope the registry read guard tightly so it doesn't span any await.
    let mut query = {
        let registry = state.storage().search_param_registry().read();
        build_search_query("Resource", &search_params, &registry)?
    };

    // Clamp page size to the configured default/maximum.
    let count = query
        .count
        .map(|c| c as usize)
        .unwrap_or(state.default_page_size())
        .min(state.max_page_size());
    query.count = Some(count as u32);

    // Execute the multi-type search
    let result = state
        .storage()
        .search_multi(tenant.context(), &resource_types, &query)
        .await
        .map_err(|e| {
            warn!(error = %e, "System-level search failed");
            RestError::from(e)
        })?;

    // Build the self link URL
    let self_link = build_system_search_url(state.base_url(), &search_params);

    // Convert result to FHIR Bundle
    let bundle = result.to_bundle(state.base_url(), &self_link);

    // Parse subsetting parameters
    let summary_mode = search_params
        .get("_summary")
        .and_then(|v| SummaryMode::parse(v));
    let elements: Option<Vec<&str>> = search_params
        .elements()
        .map(|e| e.iter().map(|s| s.as_str()).collect());

    debug!(
        results = result.resources.len(),
        summary = ?summary_mode,
        elements = ?elements,
        "System-level search completed"
    );

    // Get FHIR version from config for subsetting
    let fhir_version = state.config().default_fhir_version;

    let bundle_json =
        bundle_to_json_with_subsetting(bundle, summary_mode, elements.as_deref(), fhir_version);

    format_resource_response(StatusCode::OK, HeaderMap::new(), &bundle_json, format).map_err(|_| {
        RestError::InternalError {
            message: "Failed to serialize response".to_string(),
        }
    })
}

/// Builds a type-level search URL from base URL and parameters.
fn build_search_url(base_url: &str, resource_type: &str, params: &SearchParams) -> String {
    let query = encode_query(params);
    if query.is_empty() {
        format!("{}/{}", base_url, resource_type)
    } else {
        format!("{}/{}?{}", base_url, resource_type, query)
    }
}

/// Builds a system-level search URL from base URL and parameters.
fn build_system_search_url(base_url: &str, params: &SearchParams) -> String {
    let query = encode_query(params);
    if query.is_empty() {
        base_url.to_string()
    } else {
        format!("{}?{}", base_url, query)
    }
}

/// Encodes search params back into a query string, preserving repeated keys.
fn encode_query(params: &SearchParams) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Converts a SearchBundle to a serde_json::Value for response with optional subsetting.
fn bundle_to_json_with_subsetting(
    bundle: SearchBundle,
    summary_mode: Option<SummaryMode>,
    elements: Option<&[&str]>,
    fhir_version: FhirVersion,
) -> serde_json::Value {
    // Handle _summary=count specially - only return count, no entries
    if summary_mode == Some(SummaryMode::Count) {
        return serde_json::json!({
            "resourceType": "Bundle",
            "type": bundle.bundle_type,
            "total": bundle.total
        });
    }

    serde_json::json!({
        "resourceType": "Bundle",
        "type": bundle.bundle_type,
        "total": bundle.total,
        "link": bundle.link.iter().map(|l| {
            serde_json::json!({
                "relation": l.relation,
                "url": l.url
            })
        }).collect::<Vec<_>>(),
        "entry": bundle.entry.iter().map(|e| {
            let mut entry = serde_json::json!({});
            if let Some(ref full_url) = e.full_url {
                entry["fullUrl"] = serde_json::Value::String(full_url.clone());
            }
            if let Some(ref resource) = e.resource {
                // Apply subsetting to the resource
                let subsetted = apply_subsetting(resource, summary_mode, elements, fhir_version);
                entry["resource"] = subsetted;
            }
            if let Some(ref search) = e.search {
                entry["search"] = serde_json::json!({
                    "mode": match search.mode {
                        helios_persistence::types::SearchEntryMode::Match => "match",
                        helios_persistence::types::SearchEntryMode::Include => "include",
                        helios_persistence::types::SearchEntryMode::Outcome => "outcome",
                    }
                });
            }
            entry
        }).collect::<Vec<_>>()
    })
}

/// Applies subsetting to a resource based on _summary and _elements parameters.
fn apply_subsetting(
    resource: &serde_json::Value,
    summary_mode: Option<SummaryMode>,
    elements: Option<&[&str]>,
    fhir_version: FhirVersion,
) -> serde_json::Value {
    let mut result = resource.clone();
    let mut subsetted = false;

    // Apply _summary if specified
    if let Some(mode) = summary_mode {
        result = apply_summary(&result, mode, fhir_version);
        if mode != SummaryMode::False {
            subsetted = true;
        }
    }

    // Apply _elements if specified (takes precedence over _summary for element selection)
    if let Some(elem_list) = elements {
        if !elem_list.is_empty() {
            result = apply_elements(&result, elem_list);
            subsetted = true;
        }
    }

    // Flag incomplete representations with the SUBSETTED tag (FHIR spec).
    if subsetted {
        crate::responses::subsetting::add_subsetted_tag(&mut result);
    }

    result
}

// URL encoding helper
mod urlencoding {
    pub fn encode(s: &str) -> String {
        url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
    }
}

/// Pre-processes search params that contain `:in` or `:not-in` modifiers by
/// expanding the referenced ValueSet via the terminology server.
///
/// **`:in` modifier** — The ValueSet at the given URL is expanded.  The
/// parameter is replaced with a plain token parameter whose value is the
/// expanded codes joined by commas (FHIR OR semantics).
/// Example: `code:in=http://example.org/vs` → `code=http://cs|A,http://cs|B`
///
/// **`:not-in` modifier** — Returns `Err(RestError::NotImplemented)` so the
/// caller can surface an explicit 501 to the client.  Silently dropping a
/// negation filter would return incorrect results (all resources instead of
/// the expected subset), which is worse than an honest error.
///
/// All other parameters pass through unchanged. On individual expansion
/// failures the problematic parameter is skipped with a warning so a single
/// bad ValueSet URL does not abort the entire search.
async fn expand_terminology_params(
    pairs: Vec<(String, String)>,
    ts_url: &str,
) -> Result<Vec<(String, String)>, RestError> {
    let client = TerminologyServiceClient::new(ts_url.to_string());
    let mut result: Vec<(String, String)> = Vec::with_capacity(pairs.len());

    for (key, value) in pairs {
        if let Some(param_name) = key.strip_suffix(":in") {
            // Expand the ValueSet and join codes with commas for OR token search.
            match client.expand_value_set(&value).await {
                Ok(codes) if !codes.is_empty() => {
                    let token_list = codes
                        .iter()
                        .map(|c| c.as_token())
                        .collect::<Vec<_>>()
                        .join(",");
                    debug!(
                        param = %param_name,
                        vs_url = %value,
                        codes = %codes.len(),
                        "Expanded ValueSet for :in modifier"
                    );
                    result.push((param_name.to_string(), token_list));
                }
                Ok(_) => {
                    // Empty expansion — no code can match; use a sentinel value
                    // that will match nothing in the token index.
                    warn!(
                        param = %key,
                        vs_url = %value,
                        "ValueSet $expand returned empty expansion for :in modifier; \
                         injecting sentinel value so no resources match"
                    );
                    result.push((param_name.to_string(), HTS_EMPTY_EXPANSION.to_string()));
                }
                Err(e) => {
                    warn!(
                        param = %key,
                        vs_url = %value,
                        error = %e,
                        "ValueSet $expand failed for :in modifier; skipping parameter (fail-open)"
                    );
                    // Omit the param — searches continue without this filter.
                }
            }
        } else if let Some(param_name) = key.strip_suffix(":not-in") {
            // :not-in requires negated value-set filtering which the current
            // SQLite search backend does not support.  Return an explicit error
            // rather than silently dropping the parameter — a silent drop would
            // return all resources when the client expects a filtered subset.
            return Err(RestError::NotImplemented {
                feature: format!(
                    "search modifier ':not-in' is not supported (param: {param_name}, \
                     ValueSet: {value}); use ':in' or remove this modifier"
                ),
            });
        } else if let Some((param_name, op)) = key
            .strip_suffix(":below")
            .map(|p| (p, "is-a"))
            .or_else(|| key.strip_suffix(":above").map(|p| (p, "generalizes")))
        {
            // Token hierarchy: `code:below=system|code` expands to the code and
            // its descendants (is-a) / ancestors (generalizes) via the
            // terminology server. A bare value with no `system|code` is left
            // untouched — that is the URI form, resolved natively by the backend.
            if let Some((system, code)) = value.split_once('|') {
                let modifier = if op == "is-a" { ":below" } else { ":above" };
                expand_subsumption_into(
                    &mut result,
                    &client,
                    param_name,
                    &key,
                    system,
                    code,
                    op,
                    modifier,
                )
                .await;
            } else {
                result.push((key, value));
            }
        } else {
            result.push((key, value));
        }
    }

    Ok(result)
}

/// Sentinel token that cannot match any indexed code, used to force an empty
/// result when a terminology expansion legitimately yields no codes.
const HTS_EMPTY_EXPANSION: &str = "__hts_empty_expansion__";

/// Expands a token hierarchy modifier (`:below`/`:above`) and inserts the
/// resulting comma-joined token list under `param_name`. Mirrors the `:in`
/// policy: empty expansion → sentinel (match nothing); request error →
/// fail-open (drop the filter).
#[allow(clippy::too_many_arguments)]
async fn expand_subsumption_into(
    result: &mut Vec<(String, String)>,
    client: &TerminologyServiceClient,
    param_name: &str,
    key: &str,
    system: &str,
    code: &str,
    op: &str,
    modifier: &str,
) {
    match client.expand_subsumption(system, code, op).await {
        Ok(codes) if !codes.is_empty() => {
            let token_list = codes
                .iter()
                .map(|c| c.as_token())
                .collect::<Vec<_>>()
                .join(",");
            debug!(
                param = %param_name,
                system = %system,
                code = %code,
                modifier = %modifier,
                codes = codes.len(),
                "Expanded token hierarchy modifier"
            );
            result.push((param_name.to_string(), token_list));
        }
        Ok(_) => {
            warn!(
                param = %key,
                modifier = %modifier,
                "Token hierarchy expansion returned no codes; injecting sentinel so no resources match"
            );
            result.push((param_name.to_string(), HTS_EMPTY_EXPANSION.to_string()));
        }
        Err(e) => {
            warn!(
                param = %key,
                modifier = %modifier,
                error = %e,
                "Token hierarchy expansion failed; skipping parameter (fail-open)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Collapses result pairs into a last-wins map for assertions.
    fn as_map(pairs: &[(String, String)]) -> HashMap<String, String> {
        pairs.iter().cloned().collect()
    }

    fn sp(pairs: &[(&str, &str)]) -> SearchParams {
        SearchParams::from_pairs(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    #[test]
    fn test_build_search_url_no_params() {
        let url = build_search_url("http://example.com/fhir", "Patient", &sp(&[]));
        assert_eq!(url, "http://example.com/fhir/Patient");
    }

    #[test]
    fn test_build_search_url_with_params() {
        let url = build_search_url(
            "http://example.com/fhir",
            "Patient",
            &sp(&[("name", "Smith"), ("_count", "10")]),
        );
        assert!(url.starts_with("http://example.com/fhir/Patient?"));
        assert!(url.contains("name=Smith"));
        assert!(url.contains("_count=10"));
    }

    #[test]
    fn test_build_search_url_preserves_repeated_keys() {
        let url = build_search_url(
            "http://example.com/fhir",
            "Observation",
            &sp(&[
                ("_include", "Observation:subject"),
                ("_include", "Observation:encounter"),
            ]),
        );
        assert!(url.contains("_include=Observation%3Asubject"));
        assert!(url.contains("_include=Observation%3Aencounter"));
    }

    #[test]
    fn test_build_system_search_url() {
        let url = build_system_search_url(
            "http://example.com/fhir",
            &sp(&[("_type", "Patient,Observation")]),
        );
        assert!(url.starts_with("http://example.com/fhir?"));
        assert!(url.contains("_type="));
    }

    // ─── expand_terminology_params ───────────────────────────────────────────

    /// Verifies that non-terminology params pass through unchanged, including
    /// repeated keys.
    #[tokio::test]
    async fn test_expand_terminology_params_passthrough() {
        let params = vec![
            ("name".to_string(), "Smith".to_string()),
            ("name".to_string(), "Jones".to_string()),
            ("_count".to_string(), "10".to_string()),
        ];

        // Use a URL that won't be reachable — but only :in/:above/:below keys trigger calls.
        let result = expand_terminology_params(params, "http://localhost:9999")
            .await
            .unwrap();

        // Both repeated `name` values survive (FHIR AND semantics).
        let names: Vec<&str> = result
            .iter()
            .filter(|(k, _)| k == "name")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(names, vec!["Smith", "Jones"]);
        assert_eq!(as_map(&result).get("_count"), Some(&"10".to_string()));
    }

    /// Verifies that a `:not-in` parameter returns `NotImplemented` rather than
    /// silently dropping the filter (which would return incorrect results).
    #[tokio::test]
    async fn test_expand_terminology_params_not_in_returns_not_implemented() {
        use crate::error::RestError;
        let params = vec![
            (
                "code:not-in".to_string(),
                "http://example.org/vs".to_string(),
            ),
            ("name".to_string(), "Smith".to_string()),
        ];

        let result = expand_terminology_params(params, "http://127.0.0.1:19999").await;

        // :not-in must return an explicit error, not silently drop the filter.
        assert!(matches!(result, Err(RestError::NotImplemented { .. })));
    }

    /// Verifies that a :in param is dropped on network error (fail-open).
    #[tokio::test]
    async fn test_expand_terminology_params_in_dropped_on_network_error() {
        let params = vec![("code:in".to_string(), "http://example.org/vs".to_string())];

        let result = expand_terminology_params(params, "http://127.0.0.1:19999")
            .await
            .unwrap();

        // The :in key is gone; no code key was injected either (fail-open)
        let map = as_map(&result);
        assert!(!map.contains_key("code:in"));
        assert!(!map.contains_key("code"));
    }

    /// A token `:below` (`system|code`) is fail-open dropped on network error.
    #[tokio::test]
    async fn test_expand_terminology_params_below_token_dropped_on_network_error() {
        let params = vec![(
            "code:below".to_string(),
            "http://snomed.info/sct|73211009".to_string(),
        )];

        let result = expand_terminology_params(params, "http://127.0.0.1:19999")
            .await
            .unwrap();

        let map = as_map(&result);
        assert!(!map.contains_key("code:below"));
        assert!(!map.contains_key("code"));
    }

    /// Happy path: a token `:below` expands to the code and its descendants via
    /// a mock terminology server, rewriting to a comma-joined token list.
    #[tokio::test]
    async fn test_expand_terminology_params_below_success() {
        use axum::{Json, Router, routing::post};
        use serde_json::json;

        let app = Router::new().route(
            "/ValueSet/$expand",
            post(|| async {
                Json(json!({
                    "resourceType": "ValueSet",
                    "expansion": { "contains": [
                        { "system": "http://snomed.info/sct", "code": "73211009" },
                        { "system": "http://snomed.info/sct", "code": "44054006" },
                        { "system": "http://snomed.info/sct", "code": "46635009" }
                    ]}
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let params = vec![(
            "code:below".to_string(),
            "http://snomed.info/sct|73211009".to_string(),
        )];
        let result = expand_terminology_params(params, &format!("http://{addr}"))
            .await
            .unwrap();

        // Modifier consumed; rewritten to a plain token OR list of descendants.
        let map = as_map(&result);
        assert!(!map.contains_key("code:below"));
        let codes = map.get("code").expect("code param injected");
        assert!(codes.contains("http://snomed.info/sct|73211009"));
        assert!(codes.contains("http://snomed.info/sct|44054006"));
        assert!(codes.contains("http://snomed.info/sct|46635009"));
    }

    /// A `:below` value without a `system|code` (the URI form) is left untouched
    /// so the backend can resolve URI hierarchy natively — no terminology call.
    #[tokio::test]
    async fn test_expand_terminology_params_below_uri_passthrough() {
        let params = vec![(
            "url:below".to_string(),
            "http://example.org/fhir/ValueSet/x".to_string(),
        )];

        let result = expand_terminology_params(params, "http://127.0.0.1:19999")
            .await
            .unwrap();

        // Unchanged — no `|`, so not treated as a token subsumption.
        assert_eq!(
            as_map(&result).get("url:below"),
            Some(&"http://example.org/fhir/ValueSet/x".to_string())
        );
    }
}
