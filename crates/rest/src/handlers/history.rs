//! History interaction handlers.
//!
//! Implements the FHIR [history interaction](https://hl7.org/fhir/http.html#history):
//! - Instance history: `GET [base]/[type]/[id]/_history`
//! - Type history: `GET [base]/[type]/_history`
//! - System history: `GET [base]/_history`
//!
//! Also implements FHIR v6.0.0 Trial Use delete history operations:
//! - Delete instance history: `DELETE [base]/[type]/[id]/_history`
//! - Delete specific version: `DELETE [base]/[type]/[id]/_history/[vid]`
//!
//! History reads are backed by the `InstanceHistoryProvider`,
//! `TypeHistoryProvider`, and `SystemHistoryProvider` traits, which every
//! first-class storage backend (SQLite, PostgreSQL, MongoDB, S3) implements
//! and which `CompositeStorage` delegates to its primary.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use helios_persistence::core::{
    HistoryEntry, HistoryMethod, HistoryParams, InstanceHistoryProvider, ResourceStorage,
    SystemHistoryProvider, TypeHistoryProvider,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::error::{RestError, RestResult};
use crate::extractors::TenantExtractor;
use crate::middleware::content_type::negotiate_format;
use crate::responses::format_resource_response;
use crate::state::AppState;

/// Query parameters for history requests.
#[derive(Debug, Deserialize, Default)]
pub struct HistoryQuery {
    /// The page size.
    #[serde(rename = "_count")]
    pub count: Option<usize>,

    /// Only include versions created since this time.
    #[serde(rename = "_since")]
    pub since: Option<String>,

    /// Only include versions created before this time.
    #[serde(rename = "_at")]
    #[allow(dead_code)]
    pub at: Option<String>,

    /// Response format override (`json` / `xml` / full media types).
    #[serde(rename = "_format")]
    pub format: Option<String>,
}

/// Builds [`HistoryParams`] from the request query, applying the configured
/// page-size limits and parsing the optional `_since` instant.
fn build_history_params<S>(params: &HistoryQuery, state: &AppState<S>) -> RestResult<HistoryParams>
where
    S: ResourceStorage + Send + Sync,
{
    let count = params
        .count
        .unwrap_or(state.default_page_size())
        .min(state.max_page_size()) as u32;

    let mut history_params = HistoryParams::new().count(count).include_deleted(true);

    if let Some(since_str) = params.since.as_deref() {
        let since = DateTime::parse_from_rfc3339(since_str)
            .map_err(|_| RestError::BadRequest {
                message: format!("Invalid _since value (expected an RFC3339 instant): {since_str}"),
            })?
            .with_timezone(&Utc);
        history_params = history_params.since(since);
    }

    Ok(history_params)
}

/// Converts a single [`HistoryEntry`] into a FHIR history Bundle entry.
fn history_entry_to_json(entry: &HistoryEntry, base_url: &str) -> Value {
    let resource = &entry.resource;
    let resource_type = resource.resource_type();
    let id = resource.id();
    let version_id = resource.version_id();

    let (method, request_url, status) = match entry.method {
        HistoryMethod::Post => ("POST", resource_type.to_string(), "201"),
        HistoryMethod::Put => ("PUT", format!("{resource_type}/{id}"), "200"),
        HistoryMethod::Patch => ("PATCH", format!("{resource_type}/{id}"), "200"),
        HistoryMethod::Delete => ("DELETE", format!("{resource_type}/{id}"), "204"),
    };

    let mut bundle_entry = json!({
        "fullUrl": format!("{base_url}/{resource_type}/{id}"),
        "request": {
            "method": method,
            "url": request_url,
        },
        "response": {
            "status": status,
            "etag": format!("W/\"{version_id}\""),
            "lastModified": entry.timestamp.to_rfc3339(),
        },
    });

    // Deleted versions carry no resource body.
    if entry.method != HistoryMethod::Delete {
        bundle_entry["resource"] = resource.content().clone();
    }

    bundle_entry
}

/// Builds a FHIR `history` Bundle from a page of history entries.
fn build_history_bundle(entries: &[HistoryEntry], base_url: &str, total: u64) -> Value {
    let bundle_entries: Vec<Value> = entries
        .iter()
        .map(|entry| history_entry_to_json(entry, base_url))
        .collect();

    json!({
        "resourceType": "Bundle",
        "type": "history",
        "total": total,
        "entry": bundle_entries,
    })
}

/// Serializes a history Bundle in the negotiated response format.
fn respond_with_bundle(
    bundle: &Value,
    req_headers: &HeaderMap,
    format_param: Option<&str>,
) -> Response {
    let negotiated = negotiate_format(req_headers, format_param);
    match format_resource_response(StatusCode::OK, HeaderMap::new(), bundle, negotiated.format) {
        Ok(response) => response,
        Err(response) => response,
    }
}

/// Handler for instance history.
///
/// Returns the version history for a specific resource instance.
///
/// # HTTP Request
///
/// `GET [base]/[type]/[id]/_history`
///
/// # Query Parameters
///
/// - `_count` - Page size
/// - `_since` - Only versions since this time
///
/// # Response
///
/// - `200 OK` - Returns a Bundle of type "history"
/// - `404 Not Found` - The resource has no history (never existed)
pub async fn history_instance_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
    req_headers: HeaderMap,
    Query(params): Query<HistoryQuery>,
) -> RestResult<Response>
where
    S: ResourceStorage + InstanceHistoryProvider + Send + Sync,
{
    debug!(
        resource_type = %resource_type,
        id = %id,
        tenant = %tenant.tenant_id(),
        "Processing instance history request"
    );

    let history_params = build_history_params(&params, &state)?;

    let page = state
        .storage()
        .history_instance(tenant.context(), &resource_type, &id, &history_params)
        .await
        .map_err(|e| {
            warn!(error = %e, "Instance history failed");
            RestError::from(e)
        })?;

    // An existing resource always has at least its current version in history,
    // so an empty *unfiltered* page means the resource never existed. When a
    // `_since` filter is in play, an empty page just means nothing matched, so
    // we return an empty history Bundle rather than a 404.
    if page.items.is_empty() && params.since.is_none() {
        return Err(RestError::NotFound { resource_type, id });
    }

    let total = page.page_info.total.unwrap_or(page.items.len() as u64);
    let bundle = build_history_bundle(&page.items, state.base_url(), total);

    Ok(respond_with_bundle(
        &bundle,
        &req_headers,
        params.format.as_deref(),
    ))
}

/// Handler for type history.
///
/// Returns the version history for all resources of a type.
///
/// # HTTP Request
///
/// `GET [base]/[type]/_history`
pub async fn history_type_handler<S>(
    State(state): State<AppState<S>>,
    Path(resource_type): Path<String>,
    tenant: TenantExtractor,
    req_headers: HeaderMap,
    Query(params): Query<HistoryQuery>,
) -> RestResult<Response>
where
    S: ResourceStorage + TypeHistoryProvider + Send + Sync,
{
    debug!(
        resource_type = %resource_type,
        tenant = %tenant.tenant_id(),
        "Processing type history request"
    );

    let history_params = build_history_params(&params, &state)?;

    let page = state
        .storage()
        .history_type(tenant.context(), &resource_type, &history_params)
        .await
        .map_err(|e| {
            warn!(error = %e, "Type history failed");
            RestError::from(e)
        })?;

    let total = page.page_info.total.unwrap_or(page.items.len() as u64);
    let bundle = build_history_bundle(&page.items, state.base_url(), total);

    Ok(respond_with_bundle(
        &bundle,
        &req_headers,
        params.format.as_deref(),
    ))
}

/// Handler for system history.
///
/// Returns the version history for all resources.
///
/// # HTTP Request
///
/// `GET [base]/_history`
pub async fn history_system_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    req_headers: HeaderMap,
    Query(params): Query<HistoryQuery>,
) -> RestResult<Response>
where
    S: ResourceStorage + SystemHistoryProvider + Send + Sync,
{
    debug!(
        tenant = %tenant.tenant_id(),
        "Processing system history request"
    );

    let history_params = build_history_params(&params, &state)?;

    let page = state
        .storage()
        .history_system(tenant.context(), &history_params)
        .await
        .map_err(|e| {
            warn!(error = %e, "System history failed");
            RestError::from(e)
        })?;

    let total = page.page_info.total.unwrap_or(page.items.len() as u64);
    let bundle = build_history_bundle(&page.items, state.base_url(), total);

    Ok(respond_with_bundle(
        &bundle,
        &req_headers,
        params.format.as_deref(),
    ))
}

/// Handler for deleting instance history.
///
/// Deletes all historical versions of a resource, preserving only the current version.
/// This is a FHIR v6.0.0 Trial Use feature.
///
/// # HTTP Request
///
/// `DELETE [base]/[type]/[id]/_history`
///
/// # Response
///
/// - `200 OK` with OperationOutcome indicating number of versions deleted
/// - `404 Not Found` if the resource doesn't exist
/// - `501 Not Implemented` if the backend doesn't support this operation
pub async fn delete_instance_history_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
) -> RestResult<Response>
where
    S: ResourceStorage + InstanceHistoryProvider + Send + Sync,
{
    debug!(
        resource_type = %resource_type,
        id = %id,
        tenant = %tenant.tenant_id(),
        "Processing delete instance history request"
    );

    let deleted_count = state
        .storage()
        .delete_instance_history(tenant.context(), &resource_type, &id)
        .await
        .map_err(|e| {
            warn!(error = %e, "Delete instance history failed");
            RestError::from(e)
        })?;

    debug!(
        resource_type = %resource_type,
        id = %id,
        deleted_count = deleted_count,
        "Instance history deleted successfully"
    );

    // Return OperationOutcome with success message
    let outcome = serde_json::json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": "information",
            "code": "informational",
            "diagnostics": format!(
                "Deleted {} historical version(s) of {}/{}. Current version preserved.",
                deleted_count, resource_type, id
            )
        }]
    });

    Ok((StatusCode::OK, Json(outcome)).into_response())
}

/// Handler for deleting a specific version from history.
///
/// Deletes a specific historical version of a resource. Cannot delete the current version.
/// This is a FHIR v6.0.0 Trial Use feature.
///
/// # HTTP Request
///
/// `DELETE [base]/[type]/[id]/_history/[vid]`
///
/// # Response
///
/// - `204 No Content` on successful deletion
/// - `404 Not Found` if the resource or version doesn't exist
/// - `400 Bad Request` if attempting to delete the current version
/// - `501 Not Implemented` if the backend doesn't support this operation
pub async fn delete_version_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id, version_id)): Path<(String, String, String)>,
    tenant: TenantExtractor,
) -> RestResult<Response>
where
    S: ResourceStorage + InstanceHistoryProvider + Send + Sync,
{
    debug!(
        resource_type = %resource_type,
        id = %id,
        version_id = %version_id,
        tenant = %tenant.tenant_id(),
        "Processing delete version request"
    );

    state
        .storage()
        .delete_version(tenant.context(), &resource_type, &id, &version_id)
        .await
        .map_err(|e| {
            warn!(error = %e, "Delete version failed");
            RestError::from(e)
        })?;

    debug!(
        resource_type = %resource_type,
        id = %id,
        version_id = %version_id,
        "Version deleted successfully"
    );

    Ok(StatusCode::NO_CONTENT.into_response())
}
