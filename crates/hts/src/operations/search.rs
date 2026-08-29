//! Handlers for resource-type search endpoints.
//!
//! Implements `GET /CodeSystem`, `GET /ValueSet`, and `GET /ConceptMap` with
//! the five mandatory FHIR search parameters: `url`, `version`, `name`,
//! `title`, and `status`.  Results are returned as a FHIR `Bundle` of type
//! `searchset`.
//!
//! Pagination is controlled by `_count` (page size, default 20) and `_offset`
//! (zero-based start position, default 0).  The `total` field in the Bundle
//! reflects the number of resources returned on this page (not the grand total
//! across all pages), consistent with the lazy-count model used by most FHIR
//! servers when a full count would be expensive.

use axum::{
    Json,
    extract::{RawQuery, State},
    http::StatusCode,
    response::IntoResponse,
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

/// Build a FHIR `Bundle` of type `searchset` from a list of resource values.
fn build_searchset_bundle(resources: Vec<Value>) -> Value {
    let total = resources.len() as u64;
    let entries: Vec<Value> = resources
        .into_iter()
        .map(|resource| json!({ "resource": resource }))
        .collect();

    json!({
        "resourceType": "Bundle",
        "type": "searchset",
        "total": total,
        "entry": entries
    })
}

/// `GET /CodeSystem?url=...&name=...&status=...&version=...&title=...`
///
/// Returns a `searchset` Bundle containing matching CodeSystem resources.
pub async fn search_code_systems<B>(
    State(state): State<AppState<B>>,
    RawQuery(raw): RawQuery,
) -> impl IntoResponse
where
    B: TerminologyBackend + BundleImportBackend,
{
    let query = match parse_search_query(raw.as_deref()) {
        Ok(query) => query,
        Err(error) => return error.into_response(),
    };
    match CodeSystemOperations::search(state.backend(), &ctx(), query).await {
        Ok(resources) => (StatusCode::OK, Json(build_searchset_bundle(resources))).into_response(),
        Err(e) => e.into_response(),
    }
}

/// `GET /ValueSet?url=...&name=...&status=...&version=...&title=...`
///
/// Returns a `searchset` Bundle containing matching ValueSet resources.
pub async fn search_value_sets<B>(
    State(state): State<AppState<B>>,
    RawQuery(raw): RawQuery,
) -> impl IntoResponse
where
    B: TerminologyBackend + BundleImportBackend,
{
    let query = match parse_search_query(raw.as_deref()) {
        Ok(query) => query,
        Err(error) => return error.into_response(),
    };
    match ValueSetOperations::search(state.backend(), &ctx(), query).await {
        Ok(resources) => (StatusCode::OK, Json(build_searchset_bundle(resources))).into_response(),
        Err(e) => e.into_response(),
    }
}

/// `GET /ConceptMap?url=...&name=...&status=...&version=...&title=...`
///
/// Returns a `searchset` Bundle containing matching ConceptMap resources.
pub async fn search_concept_maps<B>(
    State(state): State<AppState<B>>,
    RawQuery(raw): RawQuery,
) -> impl IntoResponse
where
    B: TerminologyBackend + BundleImportBackend,
{
    let query = match parse_search_query(raw.as_deref()) {
        Ok(query) => query,
        Err(error) => return error.into_response(),
    };
    match ConceptMapOperations::search(state.backend(), &ctx(), query).await {
        Ok(resources) => (StatusCode::OK, Json(build_searchset_bundle(resources))).into_response(),
        Err(e) => e.into_response(),
    }
}
