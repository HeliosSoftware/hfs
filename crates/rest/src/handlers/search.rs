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
use helios_persistence::core::{
    IncludeProvider, MultiTypeSearchProvider, ResourceStorage, RevincludeProvider, SearchProvider,
    is_include_truncation_marker, resolve_includes_iterate_continuation,
    resolve_includes_iterative,
};
use helios_persistence::types::{
    IncludeDirective, SearchBundle, SearchParamType, StoredResource, TotalMode,
};
use tracing::{debug, warn};

use helios_fhir::FhirVersion;

use crate::error::{RestError, RestResult};
use crate::extractors::query_pairs::{last_value, parse_query_pairs};
use crate::extractors::{SearchParams, TenantExtractor, build_search_query, unknown_search_params};
use crate::middleware::content_type::{FhirFormat, negotiate_format};
use crate::middleware::prefer::PreferHeader;
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
    S: ResourceStorage + SearchProvider + IncludeProvider + RevincludeProvider + Send + Sync,
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
    let strict = PreferHeader::from_headers(&req_headers).is_strict();

    execute_search(
        &state,
        tenant,
        &resource_type,
        pairs,
        negotiated.format,
        strict,
    )
    .await
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
    S: ResourceStorage + SearchProvider + IncludeProvider + RevincludeProvider + Send + Sync,
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
    let strict = PreferHeader::from_headers(&req_headers).is_strict();

    execute_search(
        &state,
        tenant,
        &resource_type,
        pairs,
        negotiated.format,
        strict,
    )
    .await
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
    strict: bool,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + IncludeProvider + RevincludeProvider + Send + Sync,
{
    let bundle_json = execute_search_bundle(state, &tenant, resource_type, pairs, strict).await?;
    format_resource_response(StatusCode::OK, HeaderMap::new(), &bundle_json, format).map_err(|_| {
        RestError::InternalError {
            message: "Failed to serialize response".to_string(),
        }
    })
}

/// Executes a type-level search and returns the searchset Bundle as JSON.
///
/// The HTTP search handlers wrap this in content negotiation; bundle
/// processing (`GET [type]?params` entries in batch/transaction Bundles,
/// #478) embeds the returned Bundle as an entry resource.
pub(crate) async fn execute_search_bundle<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    resource_type: &str,
    pairs: Vec<(String, String)>,
    strict: bool,
) -> RestResult<serde_json::Value>
where
    S: ResourceStorage + SearchProvider + IncludeProvider + RevincludeProvider + Send + Sync,
{
    // Reject known-but-unimplemented control parameters instead of silently
    // ignoring them (which returns an unfiltered, misleading `200`). `_query`
    // (named queries) is not implemented by any backend. (`_list` is implemented
    // via list resolution; `_score` as an output/`_sort` concept; `_contained` /
    // `_containedType` are parsed below and gated per backend capability.)
    //
    // `_in` (R5/R6) asks whether a resource is a member of a referenced List or
    // Group; resolving that is not implemented (#638). It cannot be left to fall
    // through, because it *is* a registered parameter of type `reference` on
    // those versions — so `Prefer: handling=lenient` would not drop it, and the
    // backends would answer it with whatever their reference path makes of the
    // spec's placeholder `Resource.id` expression: an identity test on
    // PostgreSQL, an unfiltered result set on SQLite. Use `_list` for List
    // membership.
    const UNSUPPORTED_PARAMS: [&str; 2] = ["_query", "_in"];
    if let Some((key, _)) = pairs
        .iter()
        .find(|(k, _)| UNSUPPORTED_PARAMS.contains(&k.as_str()))
    {
        return Err(RestError::InvalidParameter {
            param: key.clone(),
            message: format!("search parameter '{key}' is not supported by this server"),
        });
    }

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
        // No terminology server configured: token `:in` / `:above` / `:below`
        // cannot be satisfied. Reject them with `501` rather than silently
        // falling through to literal matching (which returns misleading results).
        // `:in` is token-only, so it always needs terminology; `:above`/`:below`
        // also apply to reference/uri, which resolve locally — only reject those
        // when the parameter is a token. See assessment item A2c.
        {
            let reg = state.storage().search_param_registry(tenant.context());
            let registry = reg.read();
            for (key, _) in &pairs {
                let Some((base, modifier)) = key.split_once(':') else {
                    continue;
                };
                let needs_terminology = match modifier {
                    "in" => true,
                    "above" | "below" => registry
                        .get_param(resource_type, base)
                        .or_else(|| registry.get_param("Resource", base))
                        .map(|p| p.param_type == SearchParamType::Token)
                        .unwrap_or(false),
                    _ => false,
                };
                if needs_terminology {
                    return Err(RestError::NotImplemented {
                        feature: format!(
                            "search modifier ':{modifier}' on token parameter '{base}' requires a \
                             configured terminology server (set HFS_TERMINOLOGY_SERVER)"
                        ),
                    });
                }
            }
        }
        pairs
    };

    let mut search_params = SearchParams::from_pairs(pairs);

    // Unknown search parameters. Per FHIR search error handling these may be
    // ignored only under lenient handling and only if that is reported; under
    // `Prefer: handling=strict` they are an error.
    //
    // Scope the registry read guard tightly so it doesn't span any await —
    // parking_lot guards aren't Send by default, which would make this async fn
    // !Send.
    let (ignored_params, sort_warnings) = {
        let reg = state.storage().search_param_registry(tenant.context());
        let registry = reg.read();
        (
            unknown_search_params(resource_type, &search_params, &registry),
            crate::extractors::search_query_builder::unsortable_sort_warnings(
                resource_type,
                &search_params,
                &registry,
            ),
        )
    };
    if !ignored_params.is_empty() {
        if strict {
            return Err(RestError::InvalidParameter {
                param: ignored_params.join(", "),
                message: format!(
                    "unknown search parameter(s) rejected under Prefer: handling=strict: {}",
                    ignored_params.join(", ")
                ),
            });
        }
        // Lenient: ignore them for real. Dropping them here keeps the filter out
        // of the executed query (an unrecognized name would otherwise be matched
        // against the search index and quietly return nothing), keeps them out of
        // the self link built below, and they are reported as an OperationOutcome
        // entry on the bundle.
        debug!(
            resource_type = %resource_type,
            params = %ignored_params.join(", "),
            "Ignoring unknown search parameter(s) under lenient handling"
        );
        let retained: Vec<(String, String)> = search_params
            .iter()
            .filter(|(k, _)| !ignored_params.contains(k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        search_params = SearchParams::from_pairs(retained);
    }

    // Convert REST params to persistence SearchQuery. Scope the registry read
    // guard tightly so it doesn't span any await — parking_lot guards aren't
    // Send by default, which would make this async fn !Send.
    let mut query = {
        let reg = state.storage().search_param_registry(tenant.context());
        let registry = reg.read();
        let built = build_search_query(resource_type, &search_params, &registry)?;
        // Under strict handling, reject a `_sort` on a field the server cannot
        // actually sort by (it would otherwise silently fall back to `id`). Only
        // `_id`, `_lastUpdated`, and registered indexed typed params sort
        // reliably; composite/special/unknown fields do not. See item A4a.
        if strict {
            for s in &built.sort {
                let sortable = s.parameter == "_id"
                    || s.parameter == "_lastUpdated"
                    // `_score` ranks by relevance on full-text backends
                    // (Elasticsearch); other backends fall back to default order.
                    || s.parameter == "_score"
                    || matches!(
                        s.param_type,
                        Some(t) if t != SearchParamType::Composite && t != SearchParamType::Special
                    );
                if !sortable {
                    return Err(RestError::InvalidParameter {
                        param: format!("_sort={}", s.parameter),
                        message: format!(
                            "cannot sort by '{}' (unsupported sort field) under Prefer: handling=strict",
                            s.parameter
                        ),
                    });
                }
            }
        }
        built
    };

    // `_contained=true|both` requires contained-resource indexing. Gate it on the
    // backend's capability so unsupported backends return a clear 501 rather than
    // silently ignoring the parameter and returning an unfiltered result.
    if query.contained != helios_persistence::types::ContainedMode::Off
        && !state.storage().supports_contained_search()
    {
        return Err(RestError::NotImplemented {
            feature: "'_contained' search is not supported by this storage backend".to_string(),
        });
    }

    // Clamp page size to the configured default/maximum.
    let count = query
        .count
        .map(|c| c as usize)
        .unwrap_or(state.default_page_size())
        .min(state.max_page_size());
    query.count = Some(count as u32);

    // Resolve `_list` into an `_id` filter via application-side List lookup, so
    // any backend's `search()` can execute it. Functional list values (the
    // `$current-*` pseudo-lists) require patient-compartment clinical logic that
    // no backend implements; reject them explicitly rather than silently
    // returning an unfiltered result.
    if let Some(functional) = query.list.iter().find(|v| v.starts_with('$')) {
        return Err(RestError::NotImplemented {
            feature: format!(
                "functional list '{functional}' is not supported; \
                 use '_list=[List id]' with a stored List resource"
            ),
        });
    }
    let query = if helios_persistence::search::query_has_list(&query) {
        helios_persistence::search::resolve_list(state.storage(), tenant.context(), &query)
            .await
            .map_err(|e| {
                warn!(error = %e, "List (_list) resolution failed");
                RestError::from(e)
            })?
    } else {
        query
    };

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
    let mut result = state
        .storage()
        .search(tenant.context(), &query)
        .await
        .map_err(|e| {
            warn!(error = %e, "Search failed");
            RestError::from(e)
        })?;

    // Resolve _include/_revinclude for backends whose search() does not
    // populate includes inline (SQLite, Postgres, Elasticsearch): the Full
    // pass runs the whole backend-agnostic resolver. MongoDB is the only
    // backend that resolves inline, and it only ever runs hop 1 itself
    // (#1063) — when a directive also carries `:iterate`, the Continuation
    // pass picks up from whatever the backend already returned, without
    // re-resolving hop 1.
    // `backend_resolved` treats an `included` holding only a truncation
    // marker (#1061) as "nothing real resolved", so a backend reply that
    // truncated to zero real resources still takes the Full path rather than
    // iterating from a frontier containing the marker.
    match iterative_pass(&query.includes, backend_resolved(&result.included)) {
        IterativePass::None => {}
        IterativePass::Full => {
            let included = resolve_includes_iterative(
                state.storage(),
                tenant.context(),
                &result.resources.items,
                &query.includes,
            )
            .await
            .map_err(|e| {
                warn!(error = %e, "Include resolution failed");
                RestError::from(e)
            })?;
            result.included = included;
        }
        IterativePass::Continuation => {
            let continuation = resolve_includes_iterate_continuation(
                state.storage(),
                tenant.context(),
                &result.resources.items,
                &query.includes,
                &result.included,
            )
            .await
            .map_err(|e| {
                warn!(error = %e, "Include iterate continuation failed");
                RestError::from(e)
            })?;
            result.included.extend(continuation);
        }
    }

    // Drain any include-truncation marker(s) (#1061) out of `included` before
    // counting or building the bundle. The marker is control-plane
    // information about the search, not an included resource, and
    // `_elements`/`_summary` subsetting (via `apply_subsetting`, which runs
    // over every entry's resource including this one) could otherwise strip
    // or mangle an `OperationOutcome` embedded as an ordinary `include` entry
    // — so the diagnostics are re-emitted through `append_warning_outcome`
    // below, AFTER subsetting, the same way ignored-parameter and sort
    // warnings already are.
    let truncation_messages = drain_truncation_markers(&mut result.included);

    // Build the self link URL
    let public_base = state.public_base_url_for_request(tenant);
    let self_link = build_search_url(&public_base, resource_type, &search_params);

    // Counts for the debug line below, taken before `into_bundle` consumes the
    // result.
    let match_count = result.resources.len();
    let include_count = result.included.len();

    // Convert result to FHIR Bundle. `into_bundle` moves each resource's JSON
    // into its entry; `to_bundle` would deep-clone every one of them and then
    // drop the originals.
    let mut bundle = result.into_bundle(&public_base, &self_link);
    crate::public_url::rewrite_bundle_full_urls(&mut bundle, |resource_type, id| {
        state.public_url_for_request(tenant, [resource_type, id])
    });

    // Parse subsetting parameters
    let summary_mode = search_params
        .get("_summary")
        .and_then(|v| SummaryMode::parse(v));
    let elements: Option<Vec<&str>> = search_params
        .elements()
        .map(|e| e.iter().map(|s| s.as_str()).collect());

    debug!(
        resource_type = %resource_type,
        results = match_count,
        included = include_count,
        summary = ?summary_mode,
        elements = ?elements,
        "Search completed"
    );

    // Get FHIR version from config for subsetting
    let fhir_version = state.config().default_fhir_version;

    // Whether the resolved query actually requires an accurate total: either
    // requested explicitly via `_total=accurate` or implied by
    // `_summary=count` with no conflicting explicit `_total` (see
    // `build_search_query`). A missing total only fails closed when this
    // holds — an explicit `_total=none`/`_total=estimate` opts out (#254).
    let total_required = query.total == Some(TotalMode::Accurate);

    let mut bundle_json = bundle_to_json_with_subsetting(
        bundle,
        summary_mode,
        total_required,
        elements.as_deref(),
        fhir_version,
    )?;

    // Report the parameters that were ignored under lenient handling. Added
    // after subsetting so `_elements`/`_summary` don't strip the outcome.
    // `_summary=count` returns only `Bundle.total`, so there is no entry list to
    // attach it to.
    if !ignored_params.is_empty() && summary_mode != Some(SummaryMode::Count) {
        append_ignored_params_outcome(&mut bundle_json, &ignored_params);
    }
    // An unsortable `_sort` silently degrades to an id sort in the backends;
    // report the degradation the same way ignored parameters are (#958).
    if summary_mode != Some(SummaryMode::Count) {
        for warning in &sort_warnings {
            append_warning_outcome(&mut bundle_json, warning);
        }
        // #1061: include/revinclude resolution truncated at the configured
        // resource cap. Emitted here (post-subsetting) rather than by leaving
        // the marker in `included`, so `_elements`/`_summary` cannot strip it.
        for message in &truncation_messages {
            append_warning_outcome_with_code(
                &mut bundle_json,
                crate::responses::operation_outcome::IssueType::Incomplete,
                message,
            );
        }
    }

    Ok(bundle_json)
}

/// Decision made once `search()` returns, about whether (and how) to resolve
/// `:iterate` includes transitively (#1063).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IterativePass {
    /// No includes requested, or the backend already resolved everything
    /// there is to resolve (no directive carries `:iterate`).
    None,
    /// The backend does not resolve includes inline (SQLite, Postgres):
    /// run the full backend-agnostic resolver, hop 1 included.
    Full,
    /// The backend resolved hop 1 inline and at least one directive carries
    /// `:iterate`: continue from hop 2 without re-resolving hop 1.
    Continuation,
}

/// Pure decision table behind [`IterativePass`] — see its variants for the
/// rationale of each branch. Kept free of `state`/`result` so it is
/// unit-testable without a server.
fn iterative_pass(includes: &[IncludeDirective], backend_resolved: bool) -> IterativePass {
    if includes.is_empty() {
        IterativePass::None
    } else if !backend_resolved {
        IterativePass::Full
    } else if includes.iter().any(|d| d.iterate) {
        IterativePass::Continuation
    } else {
        IterativePass::None
    }
}

/// True when `included` holds at least one resource that is not the #1061
/// truncation marker — i.e. the backend genuinely resolved something inline,
/// as opposed to returning only a "truncated to zero" warning.
fn backend_resolved(included: &[StoredResource]) -> bool {
    included.iter().any(|r| !is_include_truncation_marker(r))
}

/// Removes every #1061 truncation marker from `included` in place, returning
/// each one's diagnostics message so the caller can re-emit them as bundle
/// warnings after subsetting.
fn drain_truncation_markers(included: &mut Vec<StoredResource>) -> Vec<String> {
    let mut messages = Vec::new();
    included.retain(|resource| {
        if is_include_truncation_marker(resource) {
            if let Some(msg) = resource.content()["issue"][0]["diagnostics"].as_str() {
                messages.push(msg.to_string());
            }
            false
        } else {
            true
        }
    });
    messages
}

/// Appends a `search.mode = outcome` entry to a searchset bundle reporting the
/// search parameters the server ignored under lenient handling.
///
/// FHIR allows an unsupported parameter to be ignored only if the server says
/// so; the self link already omits it, and this outcome names it explicitly.
fn append_ignored_params_outcome(bundle_json: &mut serde_json::Value, ignored: &[String]) {
    append_warning_outcome(
        bundle_json,
        &format!(
            "search parameter(s) not supported by this server and ignored: {}",
            ignored.join(", ")
        ),
    );
}

/// Appends one `search.mode = outcome`, `code = not-supported` warning entry
/// to the searchset. Prefer [`append_warning_outcome_with_code`] for a
/// warning whose FHIR issue type isn't "not supported" (e.g. #1061
/// truncation, which is `incomplete`).
fn append_warning_outcome(bundle_json: &mut serde_json::Value, message: &str) {
    append_warning_outcome_with_code(
        bundle_json,
        crate::responses::operation_outcome::IssueType::NotSupported,
        message,
    );
}

/// Appends one `search.mode = outcome` warning entry to the searchset, with
/// the given FHIR issue type.
fn append_warning_outcome_with_code(
    bundle_json: &mut serde_json::Value,
    code: crate::responses::operation_outcome::IssueType,
    message: &str,
) {
    let outcome = crate::responses::OperationOutcomeBuilder::new()
        .warning(code, message.to_string())
        .build();
    let entry = serde_json::json!({
        "search": { "mode": "outcome" },
        "resource": outcome,
    });
    match bundle_json.get_mut("entry").and_then(|e| e.as_array_mut()) {
        Some(entries) => entries.push(entry),
        None => bundle_json["entry"] = serde_json::json!([entry]),
    }
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
    // Note: strict unknown-parameter validation is intentionally skipped for
    // system search — parameters there are interpreted across many resource
    // types, so a per-"Resource" registry check would false-positive.
    let mut query = {
        let reg = state.storage().search_param_registry(tenant.context());
        let registry = reg.read();
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
    let public_base = state.public_base_url_for_request(&tenant);
    let self_link = build_system_search_url(&public_base, &search_params);

    // Count for the debug line below, taken before `into_bundle` consumes the
    // result.
    let match_count = result.resources.len();

    // Convert result to FHIR Bundle (moving, not cloning, each resource).
    let mut bundle = result.into_bundle(&public_base, &self_link);
    crate::public_url::rewrite_bundle_full_urls(&mut bundle, |resource_type, id| {
        state.public_url_for_request(&tenant, [resource_type, id])
    });

    // Parse subsetting parameters
    let summary_mode = search_params
        .get("_summary")
        .and_then(|v| SummaryMode::parse(v));
    let elements: Option<Vec<&str>> = search_params
        .elements()
        .map(|e| e.iter().map(|s| s.as_str()).collect());

    debug!(
        results = match_count,
        summary = ?summary_mode,
        elements = ?elements,
        "System-level search completed"
    );

    // Get FHIR version from config for subsetting
    let fhir_version = state.config().default_fhir_version;

    // See the type-level search handler above for why this check matters
    // (#254, #1012).
    let total_required = query.total == Some(TotalMode::Accurate);

    let bundle_json = bundle_to_json_with_subsetting(
        bundle,
        summary_mode,
        total_required,
        elements.as_deref(),
        fhir_version,
    )?;

    format_resource_response(StatusCode::OK, HeaderMap::new(), &bundle_json, format).map_err(|_| {
        RestError::InternalError {
            message: "Failed to serialize response".to_string(),
        }
    })
}

/// Builds a type-level search URL from base URL and parameters.
fn build_search_url(base_url: &str, resource_type: &str, params: &SearchParams) -> String {
    let query = encode_query(params);
    crate::public_url::PublicUrl::parse(base_url)
        .expect("request public base was built from validated configuration")
        .with_segments_and_query([resource_type], &query)
}

/// Builds a system-level search URL from base URL and parameters.
fn build_system_search_url(base_url: &str, params: &SearchParams) -> String {
    let query = encode_query(params);
    crate::public_url::PublicUrl::parse(base_url)
        .expect("request public base was built from validated configuration")
        .with_segments_and_query(std::iter::empty::<&str>(), &query)
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
///
/// With `_summary=count`, the response carries only `Bundle.total`. When the
/// query required an accurate total (either implied by `_summary=count` or
/// requested explicitly via `_total=accurate`) and the backend did not
/// compute one, this returns `Err(RestError::InternalError)` instead of
/// silently emitting `"total": null`. If the client explicitly opted out of
/// an accurate total (e.g. `_total=none`), a missing total still serializes
/// as `null`, since that is the behavior the client asked for.
fn bundle_to_json_with_subsetting(
    bundle: SearchBundle,
    summary_mode: Option<SummaryMode>,
    total_required: bool,
    elements: Option<&[&str]>,
    fhir_version: FhirVersion,
) -> Result<serde_json::Value, RestError> {
    // Handle _summary=count specially - only return count, no entries.
    if summary_mode == Some(SummaryMode::Count) {
        if total_required {
            // `_summary=count` implies an accurate total; a missing one is a
            // server fault and must surface as an error, never as
            // `"total": null` (#1012).
            let total = bundle.total.ok_or_else(|| RestError::InternalError {
                message: "search backend returned no total for _summary=count".to_string(),
            })?;
            return Ok(serde_json::json!({
                "resourceType": "Bundle",
                "type": bundle.bundle_type,
                "total": total
            }));
        }
        // An explicit `_total` other than `accurate` (e.g. `_total=none`) wins
        // over the `_summary=count` implication (#254); a missing total here
        // reflects that explicit choice, not a server fault, so it passes
        // through unchanged.
        return Ok(serde_json::json!({
            "resourceType": "Bundle",
            "type": bundle.bundle_type,
            "total": bundle.total
        }));
    }

    Ok(crate::responses::bundle::searchset_to_json(
        bundle,
        |resource| apply_subsetting(resource, summary_mode, elements, fhir_version),
    ))
}

/// Applies subsetting to a resource based on _summary and _elements parameters.
fn apply_subsetting(
    resource: serde_json::Value,
    summary_mode: Option<SummaryMode>,
    elements: Option<&[&str]>,
    fhir_version: FhirVersion,
) -> serde_json::Value {
    // Takes the resource by value: with neither `_summary` nor `_elements` in
    // play — the overwhelmingly common case, and every request the benchmark
    // search suite makes — this function now does nothing at all instead of
    // deep-cloning the resource and returning the copy unchanged.
    let mut result = resource;
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
    use std::path::PathBuf;
    use std::sync::Arc;

    use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};

    use crate::config::ServerConfig;
    use crate::tenant::TenantSource;

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

    fn include_directive(iterate: bool) -> IncludeDirective {
        IncludeDirective {
            include_type: helios_persistence::types::IncludeType::Include,
            source_type: "Observation".to_string(),
            search_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            iterate,
        }
    }

    fn real_resource() -> StoredResource {
        StoredResource::new(
            "Patient",
            "1",
            helios_persistence::tenant::TenantId::new("t1"),
            serde_json::json!({"resourceType": "Patient", "id": "1"}),
            helios_fhir::FhirVersion::default(),
        )
    }

    fn truncation_marker() -> StoredResource {
        let tenant = helios_persistence::tenant::TenantContext::new(
            helios_persistence::tenant::TenantId::new("t1"),
            helios_persistence::tenant::TenantPermissions::full_access(),
        );
        helios_persistence::core::include_truncation_outcome(
            &tenant,
            helios_fhir::FhirVersion::default(),
            5,
            "_include=Observation:subject",
            "increase the limit",
        )
    }

    /// #1063: pins the full truth table behind [`IterativePass`], including
    /// the `Continuation` case — unreachable before this change, since the
    /// old guard only ever produced two outcomes (nothing, or the full pass).
    #[test]
    fn iterative_pass_decision_table() {
        // No includes requested at all: never run anything, regardless of
        // whether the backend resolved something.
        assert_eq!(iterative_pass(&[], false), IterativePass::None);
        assert_eq!(iterative_pass(&[], true), IterativePass::None);

        // A non-iterate directive: run Full only if the backend resolved
        // nothing itself; otherwise the backend's hop 1 is already the whole
        // answer.
        assert_eq!(
            iterative_pass(&[include_directive(false)], false),
            IterativePass::Full
        );
        assert_eq!(
            iterative_pass(&[include_directive(false)], true),
            IterativePass::None
        );

        // A directive carrying `:iterate`: the backend having resolved hop 1
        // means there is a second hop left to chase (Continuation); the
        // backend not having resolved anything still means the Full
        // backend-agnostic pass (which itself walks every hop).
        assert_eq!(
            iterative_pass(&[include_directive(true)], true),
            IterativePass::Continuation
        );
        assert_eq!(
            iterative_pass(&[include_directive(true)], false),
            IterativePass::Full
        );

        // Mixed directives: one iterate directive is enough to trigger
        // Continuation once the backend has resolved something.
        assert_eq!(
            iterative_pass(&[include_directive(false), include_directive(true)], true),
            IterativePass::Continuation
        );
    }

    /// `backend_resolved` must not be fooled by an `included` that holds only
    /// the #1061 truncation marker — that is "truncated to zero real
    /// resources", not "the backend resolved something".
    #[test]
    fn backend_resolved_ignores_truncation_markers() {
        assert!(!backend_resolved(&[]));
        assert!(!backend_resolved(&[truncation_marker()]));
        assert!(backend_resolved(&[real_resource()]));
        assert!(backend_resolved(&[truncation_marker(), real_resource()]));
    }

    /// `drain_truncation_markers` removes the marker(s) and hands back their
    /// diagnostics text, leaving real resources untouched and in order.
    #[test]
    fn drain_truncation_markers_extracts_messages_and_leaves_real_resources() {
        let mut included = vec![real_resource(), truncation_marker()];
        let messages = drain_truncation_markers(&mut included);
        assert_eq!(included.len(), 1);
        assert_eq!(included[0].resource_type(), "Patient");
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("_include=Observation:subject"));
    }

    #[tokio::test]
    async fn system_search_uses_the_tenant_aware_public_base() {
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .map(|path| path.join("data"));
        let backend = Arc::new(
            SqliteBackend::with_config(
                ":memory:",
                SqliteBackendConfig {
                    data_dir,
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        backend.init_schema().unwrap();
        let state = AppState::new(
            backend,
            ServerConfig {
                base_url: "https://public.example/fhir".to_string(),
                ..ServerConfig::for_testing()
            },
        );
        let tenant = TenantExtractor::new("acme", TenantSource::UrlPath);

        let response = execute_system_search(
            &state,
            tenant,
            vec![("_type".to_string(), "Patient".to_string())],
            FhirFormat::Json,
        )
        .await
        .unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let bundle: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(
            bundle["link"][0]["url"],
            "https://public.example/fhir/acme?_type=Patient"
        );
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

    #[test]
    fn test_bundle_json_emits_search_score() {
        use helios_persistence::types::{BundleEntry, SearchBundle};

        let bundle = SearchBundle::new().with_entry(
            BundleEntry::match_entry(
                "http://example.com/fhir/Patient/1",
                serde_json::json!({"resourceType": "Patient", "id": "1"}),
            )
            .with_score(Some(0.42)),
        );

        let json = bundle_to_json_with_subsetting(bundle, None, false, None, FhirVersion::R4)
            .expect("bundle without _summary=count always serializes");
        let search = &json["entry"][0]["search"];
        assert_eq!(search["mode"], "match");
        assert_eq!(search["score"], serde_json::json!(0.42));
    }

    #[test]
    fn test_bundle_json_omits_absent_search_score() {
        use helios_persistence::types::{BundleEntry, SearchBundle};

        let bundle = SearchBundle::new().with_entry(BundleEntry::match_entry(
            "http://example.com/fhir/Patient/1",
            serde_json::json!({"resourceType": "Patient", "id": "1"}),
        ));

        let json = bundle_to_json_with_subsetting(bundle, None, false, None, FhirVersion::R4)
            .expect("bundle without _summary=count always serializes");
        assert!(json["entry"][0]["search"].get("score").is_none());
    }

    /// `_summary=count` with a backend-supplied total returns only the count:
    /// `Bundle.total` as a number and no `entry` key at all.
    #[test]
    fn test_count_summary_emits_total_and_no_entries() {
        use helios_persistence::types::{BundleEntry, SearchBundle};

        let bundle = SearchBundle::new()
            .with_entry(BundleEntry::match_entry(
                "http://example.com/fhir/Patient/1",
                serde_json::json!({"resourceType": "Patient", "id": "1"}),
            ))
            .with_total(83);

        let json = bundle_to_json_with_subsetting(
            bundle,
            Some(SummaryMode::Count),
            true,
            None,
            FhirVersion::R4,
        )
        .expect("total is present, so _summary=count serializes");

        assert_eq!(json["resourceType"], "Bundle");
        assert_eq!(json["total"], serde_json::json!(83));
        assert!(json.get("entry").is_none());
    }

    /// `_summary=count` implies an accurate total (#1012); a backend that
    /// returns no total despite an accurate total being required is a server
    /// fault, not a silent `"total": null`.
    #[test]
    fn test_count_summary_without_total_is_internal_error() {
        use helios_persistence::types::SearchBundle;

        let bundle = SearchBundle::new();

        let result = bundle_to_json_with_subsetting(
            bundle,
            Some(SummaryMode::Count),
            true,
            None,
            FhirVersion::R4,
        );

        match result {
            Err(RestError::InternalError { message }) => {
                assert!(
                    message.contains("_summary=count"),
                    "message should mention _summary=count, got: {message}"
                );
            }
            other => panic!("expected RestError::InternalError, got: {other:?}"),
        }
    }

    /// The fail-closed behavior is exclusive to `_summary=count`: a normal
    /// FHIR search is allowed to omit `total`, so it still serializes today
    /// as `"total": null` via `searchset_to_json`.
    #[test]
    fn test_non_count_summary_without_total_still_serializes() {
        use helios_persistence::types::{BundleEntry, SearchBundle};

        let bundle = SearchBundle::new().with_entry(BundleEntry::match_entry(
            "http://example.com/fhir/Patient/1",
            serde_json::json!({"resourceType": "Patient", "id": "1"}),
        ));

        let json = bundle_to_json_with_subsetting(bundle, None, false, None, FhirVersion::R4)
            .expect("a missing total is not an error outside of _summary=count");

        assert!(json["total"].is_null());
        assert_eq!(json["entry"][0]["resource"]["id"], "1");
    }

    /// An explicit `_total` other than `accurate` (e.g. `_total=none`) wins
    /// over the `_summary=count` implication (#254): the caller signals this
    /// by passing `total_required = false`, and a missing total then
    /// serializes as `null` instead of failing closed.
    #[test]
    fn test_count_summary_with_total_not_required_serializes_null_without_error() {
        use helios_persistence::types::SearchBundle;

        let bundle = SearchBundle::new();

        let json = bundle_to_json_with_subsetting(
            bundle,
            Some(SummaryMode::Count),
            false,
            None,
            FhirVersion::R4,
        )
        .expect("total_required = false opts out of the fail-closed check");

        assert_eq!(json["resourceType"], "Bundle");
        assert!(json["total"].is_null());
        assert!(json.get("entry").is_none());
    }
}
