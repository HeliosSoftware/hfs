//! Handlers for resource-type search endpoints.
//!
//! Implements `GET /CodeSystem`, `GET /ValueSet`, and `GET /ConceptMap` with
//! the five mandatory FHIR search parameters: `url`, `version`, `name`,
//! `title`, and `status`.  Results are returned as a FHIR `Bundle` of type
//! `searchset`.
//!
//! Pagination is controlled by `_count` (page size, default 20) and `_offset`
//! (zero-based start position, default 0).  `Bundle.total` is the number of
//! resources matching the search across all pages, and the Bundle carries
//! `self`, `previous`, and `next` links for navigating the pages.
//! `_summary=count` returns only `total`, with no entries.

use axum::{
    Json,
    extract::{OriginalUri, RawQuery, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use helios_persistence::tenant::TenantContext;
use serde_json::{Value, json};

use crate::error::HtsError;
use crate::import::BundleImportBackend;
use crate::state::AppState;
use crate::string_search::FhirStringSearchMode;
use crate::traits::{
    CodeSystemOperations, ConceptMapOperations, TerminologyBackend, ValueSetOperations,
};
use crate::types::ResourceSearchQuery;

fn ctx() -> TenantContext {
    TenantContext::system()
}

fn parse_search_query(raw: Option<&str>) -> Result<ResourceSearchQuery, HtsError> {
    let mut query = ResourceSearchQuery::default();

    for (key, value) in form_urlencoded::parse(raw.unwrap_or_default().as_bytes()) {
        // FHIR requires empty search parameters to be ignored. This check must
        // precede modifier and control validation (for example `url:not=` and
        // `_count=` are both no-ops).
        if value.is_empty() {
            continue;
        }

        let value = value.into_owned();
        let (parameter, modifier) = key
            .split_once(':')
            .map_or((key.as_ref(), None), |(base, modifier)| {
                (base, Some(modifier))
            });

        match parameter {
            "url" => {
                reject_modifier("url", modifier)?;
                set_once(&mut query.url, value, "url")?;
            }
            "version" => {
                reject_modifier("version", modifier)?;
                set_once(&mut query.version, value, "version")?;
            }
            "name" => {
                let mode = string_mode("name", modifier)?;
                set_string_filter(&mut query.name, &mut query.name_mode, value, mode, "name")?;
            }
            "title" => {
                let mode = string_mode("title", modifier)?;
                set_string_filter(
                    &mut query.title,
                    &mut query.title_mode,
                    value,
                    mode,
                    "title",
                )?;
            }
            "status" => {
                reject_modifier("status", modifier)?;
                set_once(&mut query.status, value, "status")?;
            }
            "_count" if modifier.is_none() => {
                let count = parse_u32("_count", &value)?;
                set_once(&mut query.count, count, "_count")?;
            }
            "_offset" if modifier.is_none() => {
                let offset = parse_u32("_offset", &value)?;
                set_once(&mut query.offset, offset, "_offset")?;
            }
            "_summary" if modifier.is_none() => {
                set_once(&mut query.summary, value, "_summary")?;
            }
            // Unknown parameters remain lenient, matching the existing HTS
            // behavior. Only modifiers on the five announced parameters above
            // are rejected.
            _ => {}
        }
    }

    Ok(query)
}

fn set_once<T>(slot: &mut Option<T>, value: T, parameter: &str) -> Result<(), HtsError> {
    if slot.is_some() {
        return Err(HtsError::InvalidRequest(format!(
            "Search parameter `{parameter}` was supplied more than once"
        )));
    }
    *slot = Some(value);
    Ok(())
}

fn set_string_filter(
    slot: &mut Option<String>,
    mode_slot: &mut FhirStringSearchMode,
    value: String,
    mode: FhirStringSearchMode,
    parameter: &str,
) -> Result<(), HtsError> {
    set_once(slot, value, parameter)?;
    *mode_slot = mode;
    Ok(())
}

fn string_mode(parameter: &str, modifier: Option<&str>) -> Result<FhirStringSearchMode, HtsError> {
    match modifier {
        None => Ok(FhirStringSearchMode::Prefix),
        Some("contains") => Ok(FhirStringSearchMode::Contains),
        Some("exact") => Ok(FhirStringSearchMode::Exact),
        Some(modifier) => Err(unsupported_modifier(parameter, modifier)),
    }
}

fn reject_modifier(parameter: &str, modifier: Option<&str>) -> Result<(), HtsError> {
    match modifier {
        Some(modifier) => Err(unsupported_modifier(parameter, modifier)),
        None => Ok(()),
    }
}

fn unsupported_modifier(parameter: &str, modifier: &str) -> HtsError {
    HtsError::InvalidRequest(format!(
        "Unsupported modifier `:{modifier}` for search parameter `{parameter}`"
    ))
}

fn parse_u32(parameter: &str, value: &str) -> Result<u32, HtsError> {
    value.parse().map_err(|_| {
        HtsError::InvalidRequest(format!(
            "Search parameter `{parameter}` must be an unsigned integer"
        ))
    })
}

/// Default page size when `_count` is absent, matching the backends.
const DEFAULT_PAGE_SIZE: u32 = 20;

/// Where the search was addressed, used to build `Bundle.link` URLs.
struct SearchLinkBase {
    /// `scheme://host/path` when a `Host` header is present, else the path.
    url: String,
    /// The request's query parameters minus `_count` and `_offset`, which
    /// each link sets for its own page.
    params: Vec<(String, String)>,
}

impl SearchLinkBase {
    fn new(uri: &axum::http::Uri, headers: &HeaderMap, raw_query: Option<&str>) -> Self {
        let path = uri.path();
        let host = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .filter(|host| !host.is_empty());
        let url = match host {
            Some(host) => {
                let scheme = headers
                    .get("x-forwarded-proto")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.split(',').next())
                    .map(str::trim)
                    .filter(|scheme| matches!(*scheme, "http" | "https"))
                    .unwrap_or("http");
                format!("{scheme}://{host}{path}")
            }
            None => path.to_string(),
        };
        let params = form_urlencoded::parse(raw_query.unwrap_or_default().as_bytes())
            .filter(|(key, _)| key != "_count" && key != "_offset")
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        Self { url, params }
    }

    fn page_url(&self, count: u32, offset: u64) -> String {
        let mut query = form_urlencoded::Serializer::new(String::new());
        query.extend_pairs(&self.params);
        query.append_pair("_count", &count.to_string());
        query.append_pair("_offset", &offset.to_string());
        format!("{}?{}", self.url, query.finish())
    }
}

/// Build a FHIR `Bundle` of type `searchset`.
///
/// `total` is the number of matches across all pages. `resources` is the
/// current page, or `None` for `_summary=count`, which omits `entry` and
/// paging links.
fn build_searchset_bundle(
    query: &ResourceSearchQuery,
    links: &SearchLinkBase,
    total: u64,
    resources: Option<Vec<Value>>,
) -> Value {
    let count = query.count.unwrap_or(DEFAULT_PAGE_SIZE);
    let offset = u64::from(query.offset.unwrap_or(0));

    let mut link = vec![json!({ "relation": "self", "url": links.page_url(count, offset) })];
    let mut bundle = json!({
        "resourceType": "Bundle",
        "type": "searchset",
        "total": total,
    });

    if let Some(resources) = resources {
        if count > 0 {
            let step = u64::from(count);
            if offset > 0 {
                let previous = offset.saturating_sub(step);
                link.push(
                    json!({ "relation": "previous", "url": links.page_url(count, previous) }),
                );
            }
            if offset + step < total {
                link.push(
                    json!({ "relation": "next", "url": links.page_url(count, offset + step) }),
                );
            }
        }
        let entries: Vec<Value> = resources
            .into_iter()
            .map(|resource| json!({ "resource": resource }))
            .collect();
        bundle["entry"] = Value::Array(entries);
    }

    bundle["link"] = Value::Array(link);
    bundle
}

/// Run a parsed search: count the matches, then fetch the page unless the
/// client asked for `_summary=count`.
async fn searchset_response<CountFut, SearchFut>(
    query: ResourceSearchQuery,
    links: SearchLinkBase,
    count: impl FnOnce(ResourceSearchQuery) -> CountFut,
    search: impl FnOnce(ResourceSearchQuery) -> SearchFut,
) -> Response
where
    CountFut: std::future::Future<Output = Result<u64, HtsError>>,
    SearchFut: std::future::Future<Output = Result<Vec<Value>, HtsError>>,
{
    let total = match count(query.clone()).await {
        Ok(total) => total,
        Err(error) => return error.into_response(),
    };
    let resources = if query.summary.as_deref() == Some("count") {
        None
    } else {
        match search(query.clone()).await {
            Ok(resources) => Some(resources),
            Err(error) => return error.into_response(),
        }
    };
    let bundle = build_searchset_bundle(&query, &links, total, resources);
    (StatusCode::OK, Json(bundle)).into_response()
}

/// `GET /CodeSystem?url=...&name=...&status=...&version=...&title=...`
///
/// Returns a `searchset` Bundle containing matching CodeSystem resources.
pub async fn search_code_systems<B>(
    State(state): State<AppState<B>>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> impl IntoResponse
where
    B: TerminologyBackend + BundleImportBackend,
{
    let query = match parse_search_query(raw.as_deref()) {
        Ok(query) => query,
        Err(error) => return error.into_response(),
    };
    let links = SearchLinkBase::new(&uri, &headers, raw.as_deref());
    let backend = state.backend();
    let ctx = ctx();
    searchset_response(
        query,
        links,
        |query| CodeSystemOperations::count(backend, &ctx, query),
        |query| CodeSystemOperations::search(backend, &ctx, query),
    )
    .await
}

/// `GET /ValueSet?url=...&name=...&status=...&version=...&title=...`
///
/// Returns a `searchset` Bundle containing matching ValueSet resources.
pub async fn search_value_sets<B>(
    State(state): State<AppState<B>>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> impl IntoResponse
where
    B: TerminologyBackend + BundleImportBackend,
{
    let query = match parse_search_query(raw.as_deref()) {
        Ok(query) => query,
        Err(error) => return error.into_response(),
    };
    let links = SearchLinkBase::new(&uri, &headers, raw.as_deref());
    let backend = state.backend();
    let ctx = ctx();
    searchset_response(
        query,
        links,
        |query| ValueSetOperations::count(backend, &ctx, query),
        |query| ValueSetOperations::search(backend, &ctx, query),
    )
    .await
}

/// `GET /ConceptMap?url=...&name=...&status=...&version=...&title=...`
///
/// Returns a `searchset` Bundle containing matching ConceptMap resources.
pub async fn search_concept_maps<B>(
    State(state): State<AppState<B>>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> impl IntoResponse
where
    B: TerminologyBackend + BundleImportBackend,
{
    let query = match parse_search_query(raw.as_deref()) {
        Ok(query) => query,
        Err(error) => return error.into_response(),
    };
    let links = SearchLinkBase::new(&uri, &headers, raw.as_deref());
    let backend = state.backend();
    let ctx = ctx();
    searchset_response(
        query,
        links,
        |query| ConceptMapOperations::count(backend, &ctx, query),
        |query| ConceptMapOperations::search(backend, &ctx, query),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_links_are_absolute_when_host_is_known_and_keep_other_params() {
        let uri: axum::http::Uri = "/ValueSet?name=cafe&_count=5&_offset=5".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "hts.example.org".parse().unwrap());
        headers.insert("x-forwarded-proto", "https, http".parse().unwrap());

        let links = SearchLinkBase::new(&uri, &headers, uri.query());
        assert_eq!(
            links.page_url(5, 10),
            "https://hts.example.org/ValueSet?name=cafe&_count=5&_offset=10"
        );

        let relative = SearchLinkBase::new(&uri, &HeaderMap::new(), uri.query());
        assert_eq!(
            relative.page_url(5, 0),
            "/ValueSet?name=cafe&_count=5&_offset=0"
        );
    }

    #[test]
    fn bundle_total_is_independent_of_the_page() {
        let query = ResourceSearchQuery {
            count: Some(2),
            offset: Some(2),
            ..ResourceSearchQuery::default()
        };
        let links = SearchLinkBase::new(&"/CodeSystem".parse().unwrap(), &HeaderMap::new(), None);
        let bundle = build_searchset_bundle(&query, &links, 7, Some(vec![json!({}), json!({})]));
        assert_eq!(bundle["total"], 7);
        assert_eq!(bundle["entry"].as_array().unwrap().len(), 2);
        let relations: Vec<_> = bundle["link"]
            .as_array()
            .unwrap()
            .iter()
            .map(|link| link["relation"].as_str().unwrap())
            .collect();
        assert_eq!(relations, ["self", "previous", "next"]);

        let count_only = build_searchset_bundle(&query, &links, 7, None);
        assert_eq!(count_only["total"], 7);
        assert!(count_only.get("entry").is_none());
        assert_eq!(count_only["link"].as_array().unwrap().len(), 1);
    }
}
