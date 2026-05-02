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
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use helios_persistence::tenant::TenantContext;
use serde_json::{Value, json};

use crate::import::BundleImportBackend;
use crate::state::AppState;
use crate::traits::{
    CodeSystemOperations, ConceptMapOperations, TerminologyBackend, ValueSetOperations,
};
use crate::types::ResourceSearchQuery;

fn ctx() -> TenantContext {
    TenantContext::system()
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
    Query(query): Query<ResourceSearchQuery>,
) -> impl IntoResponse
where
    B: TerminologyBackend + BundleImportBackend,
{
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
    Query(query): Query<ResourceSearchQuery>,
) -> impl IntoResponse
where
    B: TerminologyBackend + BundleImportBackend,
{
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
    Query(query): Query<ResourceSearchQuery>,
) -> impl IntoResponse
where
    B: TerminologyBackend + BundleImportBackend,
{
    match ConceptMapOperations::search(state.backend(), &ctx(), query).await {
        Ok(resources) => (StatusCode::OK, Json(build_searchset_bundle(resources))).into_response(),
        Err(e) => e.into_response(),
    }
}
