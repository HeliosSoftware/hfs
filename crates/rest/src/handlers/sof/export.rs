//! `$viewdefinition-export` and `$sqlquery-export` operation handlers.
//!
//! Implements the SQL-on-FHIR async bulk export operations:
//!
//! | Route | Method | Description |
//! |-------|--------|-------------|
//! | `/$viewdefinition-export`, `/ViewDefinition/$viewdefinition-export` | POST | Submit a view export job |
//! | `/ViewDefinition/{id}/$viewdefinition-export` | POST | Submit for stored view |
//! | `/$sqlquery-export`, `/Library/$sqlquery-export` | POST | Submit a SQL query export job |
//! | `/Library/{id}/$sqlquery-export` | POST | Submit for stored Library |
//! | `/export/{job-id}/status` | GET | Poll for job status / fetch manifest |
//! | `/export/{job-id}/status` | DELETE | Cancel job |
//! | `/export/{job-id}/{filename}` | GET | Download output file |
//!
//! Both operations share the status/cancel/download flow and the output
//! format set (`ndjson` default, `csv`, `json`, `parquet`, `fhir` — the
//! latter producing newline-delimited `Parameters` files served as
//! `application/fhir+ndjson`).
//!
//! ## Submit response (202)
//!
//! ```text
//! 202 Accepted
//! Content-Location: /export/{job-id}/status
//! ```
//!
//! Per spec, callers should send `Prefer: respond-async`; the server returns
//! `400 Bad Request` if the header is missing.
//!
//! ## Poll response
//!
//! - `202 Accepted` + `X-Progress: running` while the job is running
//! - `200 OK` with the completion manifest `Parameters` resource in the body
//!   when complete (per the FHIR Asynchronous Interaction Request Pattern —
//!   there is no `303 See Other` redirect and no separate result URL)
//! - `500 Internal Server Error` + `OperationOutcome` if the job failed
//! - `404 Not Found` if the job ID is unknown or was cancelled

use axum::{
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use helios_persistence::core::ResourceStorage;
use helios_persistence::core::search::SearchProvider;
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::{
    SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::references::resolve_resource_canonical_or_relative;
use super::sqlquery::{sqlquery_err_to_rest, validate_select_only};
use crate::error::RestError;
use crate::export::controller::{
    ExportTask, ExportWork, JobStatus, NamedSqlQuery, NamedView, SqlExportLimits, SqlTableSource,
};
use crate::extractors::TenantExtractor;
use crate::state::AppState;

/// Top-level Parameters body parameter names recognised by the operation.
/// Anything outside this list is rejected with 400 per the spec's
/// "reject unsupported parameters" rule.
const ALLOWED_BODY_PARAMS: &[&str] = &[
    "view",
    "viewResource", // back-compat single-view form
    "_format",
    "header",
    "patient",
    "group",
    "_since",
    "clientTrackingId",
    "source",
];

/// Output formats this server can serialize. The spec binds `_format` to
/// the extensible `OutputFormatCodes` value set; we reject anything outside
/// this list with 400 per the spec's "reject unsupported parameters" rule
/// rather than silently downgrading the output to NDJSON. `fhir` exports
/// each output as a file of newline-delimited `Parameters` resources
/// (`application/fhir+ndjson`) per the spec's Common Operation Behavior.
const SUPPORTED_FORMATS: &[&str] = &["ndjson", "csv", "json", "parquet", "fhir"];

/// Query parameters for `$viewdefinition-export`.
///
/// `deny_unknown_fields` enforces the spec's "reject unsupported parameters
/// with 400 Bad Request" rule on the query string. Any parameter outside this
/// struct (whether spec-defined-but-unsupported or simply unknown) surfaces
/// as a serde error, which axum/serde maps to a 400 response.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportQueryParams {
    /// Output format: `ndjson` (default), `csv`, `json`, or `parquet`.
    #[serde(rename = "_format")]
    pub format: Option<String>,

    /// Include a CSV header row (default `true`, CSV format only).
    pub header: Option<bool>,

    /// Include only resources modified at or after this instant (RFC 3339).
    #[serde(rename = "_since")]
    pub since: Option<String>,

    /// Filter to patient references (comma-separated for multiple).
    pub patient: Option<String>,

    /// Filter to group references (comma-separated for multiple).
    pub group: Option<String>,

    /// Client-supplied tracking identifier echoed in the completion manifest.
    #[serde(rename = "clientTrackingId")]
    pub client_tracking_id: Option<String>,

    /// Spec input parameter `source` (external data source — e.g. URI or
    /// bucket name). This server does not support external sources, so its
    /// presence triggers a 400 per the spec's "reject unsupported parameters"
    /// rule. Captured here so the handler can detect it on query strings.
    pub source: Option<String>,
}

// ============================================================================
// Submit: POST /ViewDefinition/$viewdefinition-export
// ============================================================================

/// Submit an export job. Accepts:
/// - A bare `ViewDefinition` resource (single, unnamed view), or
/// - A FHIR `Parameters` resource with one or more `view` parameters whose
///   `part` entries supply `name`, `viewResource`, or `viewReference`, plus
///   optional top-level filter parameters (`_format`, `header`, `patient`,
///   `group`, `_since`, `clientTrackingId`). Query-string values take
///   precedence over body values for the same parameter.
pub async fn export_view_definition_handler<S>(
    tenant: TenantExtractor,
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Option<axum::Json<Value>>,
) -> Result<Response, RestError>
where
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    if let Err(resp) = check_prefer_async(&headers) {
        return Ok(resp);
    }
    let params = match parse_export_query(raw_query.as_deref()) {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let body_value = body.map(|axum::Json(v)| v);

    if let Some(b) = body_value.as_ref() {
        if let Some(resp) =
            validate_unknown_body_params(b, ALLOWED_BODY_PARAMS, "$viewdefinition-export")
        {
            return Ok(resp);
        }
    }
    if let Some(resp) = reject_unsupported_source(&params, body_value.as_ref()) {
        return Ok(resp);
    }

    let Some(body) = body_value else {
        return Ok(missing_view_response());
    };
    let views = extract_views_from_body(&state, &tenant, &body).await?;
    if views.is_empty() {
        return Ok(missing_view_response());
    }

    let inputs = merge_export_inputs(&params, Some(&body));

    submit_export_job(&state, &tenant, views, inputs).await
}

/// Submit an export job for a stored ViewDefinition.
pub async fn export_stored_view_definition_handler<S>(
    tenant: TenantExtractor,
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Option<axum::Json<Value>>,
) -> Result<Response, RestError>
where
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    if let Err(resp) = check_prefer_async(&headers) {
        return Ok(resp);
    }
    // Spec scopes every input parameter (`view`, `_format`, `header`,
    // `patient`, `group`, `_since`, `clientTrackingId`, `source`) to
    // "system, type" — none are defined at instance level, where the
    // ViewDefinition is identified entirely by the URL path. Reject any
    // attempt to supply them with 400 + OperationOutcome.
    if let Some(resp) = reject_instance_level_params(
        raw_query.as_deref(),
        body.as_ref(),
        "$viewdefinition-export",
    ) {
        return Ok(resp);
    }
    // body and query are guaranteed empty of spec params at this point; we
    // still drop the body so subsequent code doesn't peek at it by accident.
    drop(body);

    // Fetch the stored ViewDefinition
    let stored = state
        .storage()
        .read(tenant.context(), "ViewDefinition", &id)
        .await
        .map_err(|e| RestError::InternalError {
            message: format!("failed to read ViewDefinition: {e}"),
        })?
        .ok_or_else(|| RestError::NotFound {
            resource_type: "ViewDefinition".to_string(),
            id: id.clone(),
        })?;

    let view = stored.content().clone();
    let view_name = view
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| id.clone());

    let inputs = merge_export_inputs(&ExportQueryParams::default(), None);

    submit_export_job(
        &state,
        &tenant,
        vec![NamedView {
            name: view_name,
            view,
        }],
        inputs,
    )
    .await
}

// ============================================================================
// Submit: POST /$sqlquery-export and /Library/$sqlquery-export
// ============================================================================

/// Top-level Parameters body parameter names recognised by
/// `$sqlquery-export`. Anything outside this list is rejected with 400 per
/// the spec's "reject unsupported parameters" rule.
const SQLQUERY_ALLOWED_BODY_PARAMS: &[&str] = &[
    "query",
    "view",
    "_format",
    "header",
    "patient",
    "group",
    "_since",
    "clientTrackingId",
    "source",
];

/// Submit a `$sqlquery-export` job (system and type level).
///
/// Accepts a FHIR `Parameters` resource with one or more `query` parameters
/// whose `part` entries supply `name`, `queryReference` or `queryResource`,
/// and optional per-query `parameters` bindings; optional `view` parameters
/// supply ViewDefinition table sources for the Libraries' `depends-on`
/// entries (resolved from storage when not supplied inline). Top-level filter
/// parameters mirror `$viewdefinition-export`.
pub async fn sqlquery_export_handler<S>(
    tenant: TenantExtractor,
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Option<axum::Json<Value>>,
) -> Result<Response, RestError>
where
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    if let Err(resp) = check_prefer_async(&headers) {
        return Ok(resp);
    }
    let params = match parse_export_query(raw_query.as_deref()) {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let body_value = body.map(|axum::Json(v)| v);

    if let Some(b) = body_value.as_ref() {
        if let Some(resp) =
            validate_unknown_body_params(b, SQLQUERY_ALLOWED_BODY_PARAMS, "$sqlquery-export")
        {
            return Ok(resp);
        }
    }
    if let Some(resp) = reject_unsupported_source(&params, body_value.as_ref()) {
        return Ok(resp);
    }

    let Some(body) = body_value else {
        return Ok(missing_query_response());
    };
    if body.get("resourceType").and_then(|v| v.as_str()) != Some("Parameters") {
        return Err(RestError::BadRequest {
            message: format!(
                "Expected Parameters, got '{}'",
                body.get("resourceType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
            ),
        });
    }

    let table_sources = extract_table_source_views(&state, &tenant, &body).await?;
    let queries = extract_sql_queries_from_body(&state, &tenant, &body, &table_sources).await?;
    if queries.is_empty() {
        return Ok(missing_query_response());
    }

    let inputs = merge_export_inputs(&params, Some(&body));

    submit_sqlquery_export_job(&state, &tenant, queries, inputs).await
}

/// Submit a `$sqlquery-export` job for a stored Library (instance level).
///
/// The bound Library identified by the URL path is the query source; per the
/// spec, every input parameter is scoped to system/type level only, so any
/// query or body parameter is rejected. `Library.parameter` declarations must
/// all carry defaults (there is no way to supply bindings at instance level).
pub async fn sqlquery_export_stored_handler<S>(
    tenant: TenantExtractor,
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    body: Option<axum::Json<Value>>,
) -> Result<Response, RestError>
where
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    if let Err(resp) = check_prefer_async(&headers) {
        return Ok(resp);
    }
    if let Some(resp) =
        reject_instance_level_params(raw_query.as_deref(), body.as_ref(), "$sqlquery-export")
    {
        return Ok(resp);
    }
    drop(body);

    let stored = state
        .storage()
        .read(tenant.context(), "Library", &id)
        .await
        .map_err(|e| RestError::InternalError {
            message: format!("failed to read Library: {e}"),
        })?
        .ok_or_else(|| RestError::NotFound {
            resource_type: "Library".to_string(),
            id: id.clone(),
        })?;
    let library_json = stored.content().clone();

    let query = prepare_named_sqlquery(&state, &tenant, None, &library_json, None, &[]).await?;

    let inputs = merge_export_inputs(&ExportQueryParams::default(), None);
    submit_sqlquery_export_job(&state, &tenant, vec![query], inputs).await
}

/// 422 response for bodies that don't supply at least one valid query.
fn missing_query_response() -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        axum::Json(json!({
            "resourceType": "OperationOutcome",
            "issue": [{"severity": "error", "code": "invalid",
                "diagnostics": "at least one SQLQuery Library is required (use `query.queryResource` or `query.queryReference`)"}]
        })),
    )
        .into_response()
}

/// A ViewDefinition table source supplied via a body `view` parameter:
/// optional friendly name plus the resolved ViewDefinition JSON. These are
/// materialized as tables for the SQL to query against — they do not produce
/// their own `output` entries.
struct SuppliedTableView {
    name: Option<String>,
    view: Value,
}

/// Extracts and resolves the body's `view` table-source parameters.
async fn extract_table_source_views<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    body: &Value,
) -> Result<Vec<SuppliedTableView>, RestError>
where
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    let entries = body
        .get("parameter")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out: Vec<SuppliedTableView> = Vec::new();
    for p in &entries {
        if p.get("name").and_then(|n| n.as_str()) != Some("view") {
            continue;
        }
        let parts = p.get("part").and_then(|v| v.as_array());
        let mut name: Option<String> = None;
        let mut inline: Option<Value> = None;
        let mut reference: Option<String> = None;
        if let Some(arr) = parts {
            for part in arr {
                match part.get("name").and_then(|v| v.as_str()) {
                    Some("name") => {
                        name = part
                            .get("valueString")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                    Some("viewResource") => inline = part.get("resource").cloned(),
                    Some("viewReference") => {
                        reference = part
                            .get("valueReference")
                            .and_then(|r| r.get("reference"))
                            .and_then(|v| v.as_str())
                            .or_else(|| part.get("valueString").and_then(|v| v.as_str()))
                            .map(|s| s.to_string());
                    }
                    _ => {}
                }
            }
        }
        let view = match (inline, reference) {
            (Some(v), _) => v,
            (None, Some(r)) => {
                resolve_resource_canonical_or_relative(
                    state,
                    tenant.context(),
                    "ViewDefinition",
                    &r,
                )
                .await?
            }
            (None, None) => {
                return Err(RestError::BadRequest {
                    message: "each `view` parameter must supply `viewResource` or `viewReference`"
                        .to_string(),
                });
            }
        };
        out.push(SuppliedTableView { name, view });
    }
    Ok(out)
}

/// Extracts the body's `query` parameters, resolving Libraries and their
/// `depends-on` ViewDefinitions and binding `Library.parameter` values, so
/// the background job has everything it needs without storage access.
async fn extract_sql_queries_from_body<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    body: &Value,
    table_sources: &[SuppliedTableView],
) -> Result<Vec<NamedSqlQuery>, RestError>
where
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    let entries = body
        .get("parameter")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out: Vec<NamedSqlQuery> = Vec::new();
    for p in &entries {
        if p.get("name").and_then(|n| n.as_str()) != Some("query") {
            continue;
        }
        let parts = p.get("part").and_then(|v| v.as_array());
        let mut name: Option<String> = None;
        let mut inline: Option<Value> = None;
        let mut reference: Option<String> = None;
        let mut bindings_resource: Option<Value> = None;
        if let Some(arr) = parts {
            for part in arr {
                match part.get("name").and_then(|v| v.as_str()) {
                    Some("name") => {
                        name = part
                            .get("valueString")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                    Some("queryResource") => inline = part.get("resource").cloned(),
                    Some("queryReference") => {
                        reference = part
                            .get("valueReference")
                            .and_then(|r| r.get("reference"))
                            .and_then(|v| v.as_str())
                            .or_else(|| part.get("valueString").and_then(|v| v.as_str()))
                            .map(|s| s.to_string());
                    }
                    Some("parameters") => bindings_resource = part.get("resource").cloned(),
                    _ => {}
                }
            }
        }
        let library_json = match (inline, reference) {
            (Some(_), Some(_)) => {
                return Err(RestError::BadRequest {
                    message: "supply at most one of `queryReference` or `queryResource` per query"
                        .to_string(),
                });
            }
            (Some(v), None) => v,
            (None, Some(r)) => {
                resolve_resource_canonical_or_relative(state, tenant.context(), "Library", &r)
                    .await?
            }
            (None, None) => {
                return Err(RestError::BadRequest {
                    message:
                        "each `query` parameter must supply `queryResource` or `queryReference`"
                            .to_string(),
                });
            }
        };
        out.push(
            prepare_named_sqlquery(
                state,
                tenant,
                name,
                &library_json,
                bindings_resource.as_ref(),
                table_sources,
            )
            .await?,
        );
    }
    Ok(out)
}

/// Parses and validates one SQLQuery Library and packages it as a
/// [`NamedSqlQuery`]: validates the SQL is a single SELECT, enforces the
/// depends-on cap, resolves every `depends-on` ViewDefinition (preferring
/// supplied `view` table sources, matched by canonical `url` or by name
/// against the depends-on label, then falling back to storage resolution),
/// and binds `Library.parameter` values.
async fn prepare_named_sqlquery<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    name_hint: Option<String>,
    library_json: &Value,
    supplied_params: Option<&Value>,
    table_sources: &[SuppliedTableView],
) -> Result<NamedSqlQuery, RestError>
where
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    let library =
        helios_sof::sqlquery::parse_sqlquery_library(library_json).map_err(sqlquery_err_to_rest)?;

    let max_vds = state.config().sof_sqlquery_max_vds;
    if library.depends_on.len() > max_vds {
        return Err(RestError::UnprocessableEntity {
            message: format!(
                "Library declares {} depends-on ViewDefinitions; max allowed is {}",
                library.depends_on.len(),
                max_vds
            ),
        });
    }

    validate_select_only(&library.sql)?;

    let mut tables: Vec<SqlTableSource> = Vec::with_capacity(library.depends_on.len());
    for dep in &library.depends_on {
        let supplied = table_sources
            .iter()
            .find(|t| t.view.get("url").and_then(|u| u.as_str()) == Some(dep.url.as_str()))
            .or_else(|| {
                table_sources
                    .iter()
                    .find(|t| t.name.as_deref() == Some(dep.label.as_str()))
            });
        let view = match supplied {
            Some(t) => t.view.clone(),
            None => {
                resolve_resource_canonical_or_relative(
                    state,
                    tenant.context(),
                    "ViewDefinition",
                    &dep.url,
                )
                .await?
            }
        };
        tables.push(SqlTableSource {
            label: dep.label.clone(),
            view,
        });
    }

    let bindings = helios_sof::sqlquery::bind_supplied_params(&library.parameters, supplied_params)
        .map_err(sqlquery_err_to_rest)?;

    let name = name_hint
        .or_else(|| {
            library_json
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "output".to_string());

    Ok(NamedSqlQuery {
        name,
        sql: library.sql,
        tables,
        bindings,
    })
}

/// Returns `Err(Response)` with 400 + OperationOutcome if the spec-required
/// `Prefer: respond-async` header is missing. Returns `Ok(())` if present.
#[allow(clippy::result_large_err)]
fn check_prefer_async(headers: &HeaderMap) -> Result<(), Response> {
    let prefers_async = headers
        .get_all("prefer")
        .iter()
        .filter_map(|h| h.to_str().ok())
        .any(|h| {
            h.split(',')
                .any(|tok| tok.trim().eq_ignore_ascii_case("respond-async"))
        });

    if prefers_async {
        return Ok(());
    }
    Err((
        StatusCode::BAD_REQUEST,
        axum::Json(json!({
            "resourceType": "OperationOutcome",
            "issue": [{"severity": "error", "code": "invariant",
                "diagnostics": "bulk export requires the `Prefer: respond-async` header per the SQL-on-FHIR v2 spec"}]
        })),
    )
        .into_response())
}

/// Returns `Some(400 response)` if the caller supplied any input parameter
/// (in the query string or the body) at instance level. Per the spec, every
/// input parameter — `view`, `_format`, `header`, `patient`, `group`,
/// `_since`, `clientTrackingId`, `source` — is scoped to "system, type"
/// only. The instance-level URL `/ViewDefinition/{id}/$viewdefinition-export`
/// identifies the view entirely from the URL path, so any body or query
/// parameter is unsupported.
fn reject_instance_level_params(
    raw_query: Option<&str>,
    body: Option<&axum::Json<Value>>,
    op: &str,
) -> Option<Response> {
    let raw = raw_query.unwrap_or("");
    if let Some((k, _)) = url::form_urlencoded::parse(raw.as_bytes()).next() {
        return Some(
            (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({
                    "resourceType": "OperationOutcome",
                    "issue": [{
                        "severity": "error",
                        "code": "not-supported",
                        "diagnostics": format!(
                            "parameter '{k}' is not supported at the instance-level \
                             {op} endpoint; spec scopes all input \
                             parameters to system and type level only"
                        )
                    }]
                })),
            )
                .into_response(),
        );
    }

    let body_params = body
        .as_ref()
        .and_then(|axum::Json(v)| v.get("parameter"))
        .and_then(|p| p.as_array());
    if let Some(arr) = body_params {
        if let Some(first) = arr.first() {
            let name = first
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("(unnamed)");
            return Some(
                (
                    StatusCode::BAD_REQUEST,
                    axum::Json(json!({
                        "resourceType": "OperationOutcome",
                        "issue": [{
                            "severity": "error",
                            "code": "not-supported",
                            "diagnostics": format!(
                                "body parameter '{name}' is not supported at the \
                                 instance-level {op} endpoint; spec \
                                 scopes all input parameters to system and type level only"
                            )
                        }]
                    })),
                )
                    .into_response(),
            );
        }
    }
    None
}

/// Returns `Some(400 response)` if the caller supplied the spec-defined
/// `source` input parameter (in the query string or the Parameters body).
/// This server does not support an external data source, so per the spec
/// (*"If server does not support a parameter, request should be rejected
/// with `400 Bad Request`"*) we reject the request rather than silently
/// ignoring the parameter.
fn reject_unsupported_source(params: &ExportQueryParams, body: Option<&Value>) -> Option<Response> {
    let in_query = params.source.is_some();
    let in_body = body
        .and_then(|b| b.get("parameter"))
        .and_then(|p| p.as_array())
        .map(|arr| {
            arr.iter()
                .any(|p| p.get("name").and_then(|n| n.as_str()) == Some("source"))
        })
        .unwrap_or(false);

    if !(in_query || in_body) {
        return None;
    }
    Some(
        (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({
                "resourceType": "OperationOutcome",
                "issue": [{"severity": "error", "code": "not-supported",
                    "diagnostics": "the `source` parameter is not supported by this server"}]
            })),
        )
            .into_response(),
    )
}

/// 422 response for bodies that don't supply at least one valid view.
fn missing_view_response() -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        axum::Json(json!({
            "resourceType": "OperationOutcome",
            "issue": [{"severity": "error", "code": "invalid",
                "diagnostics": "at least one ViewDefinition is required (use `view.viewResource` or `view.viewReference`)"}]
        })),
    )
        .into_response()
}

/// Common submit logic: validate every view, dispatch to controller, return 202.
async fn submit_export_job<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    views: Vec<NamedView>,
    inputs: ExportInputs,
) -> Result<Response, RestError>
where
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    // Reject unsupported `_format` values up-front. The serializer in the
    // controller falls back to NDJSON on unknown formats; without this guard
    // the kick-off would 202-accept the job and the completion manifest would
    // echo the bogus format while the files are actually NDJSON.
    if !SUPPORTED_FORMATS.contains(&inputs.format.as_str()) {
        return Ok((
            StatusCode::BAD_REQUEST,
            axum::Json(json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "error",
                    "code": "not-supported",
                    "diagnostics": format!(
                        "unsupported `_format` value '{}'; supported: {}",
                        inputs.format,
                        SUPPORTED_FORMATS.join(", ")
                    )
                }]
            })),
        )
            .into_response());
    }

    // Spec implies one `output` per submitted view. The completion manifest
    // groups files by view name, so two views with the same name would
    // silently collapse into a single `output` entry. Detect collisions at
    // submit time and reject.
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for nv in &views {
        if !seen.insert(nv.name.as_str()) {
            return Ok((
                StatusCode::BAD_REQUEST,
                axum::Json(json!({
                    "resourceType": "OperationOutcome",
                    "issue": [{
                        "severity": "error",
                        "code": "invalid",
                        "diagnostics": format!(
                            "duplicate view name '{}'; each submitted view must have a unique \
                             name (set `view.part[name=name].valueString` or \
                             `ViewDefinition.name` to disambiguate)",
                            nv.name
                        )
                    }]
                })),
            )
                .into_response());
        }
    }

    // Validate that each view has a `resource` field (basic check).
    for nv in &views {
        if nv.view.get("resource").and_then(|v| v.as_str()).is_none() {
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                axum::Json(json!({
                    "resourceType": "OperationOutcome",
                    "issue": [{"severity": "error", "code": "invalid",
                        "diagnostics": format!("ViewDefinition.resource is required (view '{}')", nv.name)}]
                })),
            )
                .into_response());
        }
    }

    // Spec SHOULD: if patient/group references don't resolve, return an
    // OperationOutcome with details. We check relative `Patient/{id}` and
    // `Group/{id}` references here; absolute external refs pass through.
    if let Some(resp) = validate_patient_group_refs(state, tenant, &inputs).await? {
        return Ok(resp);
    }

    dispatch_export_job(state, tenant, ExportWork::Views(views), inputs).await
}

/// Common submit logic for `$sqlquery-export`: validate, dispatch to the
/// controller, return 202. Mirrors [`submit_export_job`] with query-specific
/// checks (the per-query SQL/Library/binding validation happened during
/// extraction).
async fn submit_sqlquery_export_job<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    queries: Vec<NamedSqlQuery>,
    inputs: ExportInputs,
) -> Result<Response, RestError>
where
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    if !SUPPORTED_FORMATS.contains(&inputs.format.as_str()) {
        return Ok((
            StatusCode::BAD_REQUEST,
            axum::Json(json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "error",
                    "code": "not-supported",
                    "diagnostics": format!(
                        "unsupported `_format` value '{}'; supported: {}",
                        inputs.format,
                        SUPPORTED_FORMATS.join(", ")
                    )
                }]
            })),
        )
            .into_response());
    }

    // The completion manifest groups files by output name; duplicates would
    // silently collapse into one `output` entry (same rule as views).
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for q in &queries {
        if !seen.insert(q.name.as_str()) {
            return Ok((
                StatusCode::BAD_REQUEST,
                axum::Json(json!({
                    "resourceType": "OperationOutcome",
                    "issue": [{
                        "severity": "error",
                        "code": "invalid",
                        "diagnostics": format!(
                            "duplicate query name '{}'; each submitted query must have a unique \
                             name (set `query.part[name=name].valueString` or `Library.name` \
                             to disambiguate)",
                            q.name
                        )
                    }]
                })),
            )
                .into_response());
        }
    }

    if let Some(resp) = validate_patient_group_refs(state, tenant, &inputs).await? {
        return Ok(resp);
    }

    // Execution caps mirror the synchronous `$sqlquery-run` configuration.
    let limits = SqlExportLimits {
        max_source_rows_per_vd: state.config().sof_sqlquery_max_source_rows_per_vd,
        max_rows: state.config().sof_sqlquery_max_rows,
        timeout_secs: state.config().sof_sqlquery_timeout_secs,
    };

    dispatch_export_job(
        state,
        tenant,
        ExportWork::SqlQueries { queries, limits },
        inputs,
    )
    .await
}

/// Shared tail of every export kick-off: require the controller, build the
/// task, submit, and return the spec's `202 Accepted` kick-off response.
async fn dispatch_export_job<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    work: ExportWork,
    inputs: ExportInputs,
) -> Result<Response, RestError>
where
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    // Require export controller to be configured
    let controller = match state.export_controller() {
        Some(c) => c,
        None => {
            return Ok((
                StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(json!({
                    "resourceType": "OperationOutcome",
                    "issue": [{"severity": "error", "code": "not-supported",
                        "diagnostics": "Export controller not configured on this server"}]
                })),
            )
                .into_response());
        }
    };

    // Build filters (G4, G5). patient / group multiple values match resources
    // from any of the referenced compartments. Spec defines no `_limit` for
    // the export operations (unlike the run operations); limit stays unset so
    // exports are bounded only by the underlying data set.
    let filters = helios_persistence::core::sof_runner::ViewFilters {
        limit: None,
        since: inputs.since,
        patient: inputs.patient.clone(),
        group: inputs.group.clone(),
    };

    let task = ExportTask {
        work,
        tenant: tenant.context().clone(),
        filters,
        format: inputs.format.clone(),
        header: inputs.header,
        client_tracking_id: inputs.client_tracking_id.clone(),
    };

    let job_id = controller.submit(task);
    // Spec: `Content-Location` must be the absolute URL of the status endpoint.
    let location = format!(
        "{base}/export/{job_id}/status",
        base = state.base_url().trim_end_matches('/'),
    );

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_LOCATION,
        HeaderValue::from_str(&location)
            .unwrap_or_else(|_| HeaderValue::from_static("/export/unknown/status")),
    );

    // Spec: kick-off body is a `Parameters` resource with `exportId`,
    // `status=accepted`, `location`, and optionally `clientTrackingId`.
    let mut body_params = vec![
        json!({"name": "exportId", "valueString": job_id}),
        json!({"name": "status", "valueCode": "accepted"}),
        json!({"name": "location", "valueUri": location}),
    ];
    if let Some(tid) = inputs.client_tracking_id.as_deref() {
        body_params.push(json!({"name": "clientTrackingId", "valueString": tid}));
    }

    Ok((
        StatusCode::ACCEPTED,
        headers,
        axum::Json(json!({
            "resourceType": "Parameters",
            "parameter": body_params
        })),
    )
        .into_response())
}

// ============================================================================
// Poll: GET /export/{job-id}/status
// ============================================================================

/// Poll the status of an export job.
pub async fn get_export_status_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    Path(job_id): Path<String>,
) -> Result<Response, RestError>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let controller = match state.export_controller() {
        Some(c) => c,
        None => {
            return Ok((StatusCode::SERVICE_UNAVAILABLE, "export not configured").into_response());
        }
    };

    match controller.get_status(tenant.tenant_id(), &job_id) {
        None | Some(JobStatus::Cancelled) => Ok((
            StatusCode::NOT_FOUND,
            axum::Json(json!({
                "resourceType": "OperationOutcome",
                "issue": [{"severity": "error", "code": "not-found",
                    "diagnostics": format!("Export job '{job_id}' not found or was cancelled")}]
            })),
        )
            .into_response()),

        Some(JobStatus::Running {
            percent,
            submitted_at,
        }) => {
            let mut headers = HeaderMap::new();
            // Spec: `X-Progress` carries a completion percentage (e.g. `65%`).
            let progress_value = format!("{percent}%");
            if let Ok(v) = HeaderValue::from_str(&progress_value) {
                headers.insert("x-progress", v);
            }
            // Spec SHOULD: include Retry-After during polling.
            headers.insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
            // Spec: in-progress body is an optional `Parameters` resource
            // carrying spec-defined params only (no custom `progress` part —
            // that channel is the `X-Progress` header).
            let mut params = vec![
                json!({"name": "exportId", "valueString": job_id}),
                json!({"name": "status", "valueCode": "in-progress"}),
            ];
            // Optional `estimatedTimeRemaining` (integer seconds).
            // Only meaningful once the job has reported >0% progress; before
            // then we can't compute a defensible estimate. Derived from
            // elapsed and percent: total ≈ elapsed * 100 / percent.
            if percent > 0 && percent < 100 {
                let elapsed = (chrono::Utc::now() - submitted_at).num_seconds().max(0);
                let estimate = elapsed * (100 - percent as i64) / percent as i64;
                params.push(json!({
                    "name": "estimatedTimeRemaining",
                    "valueInteger": estimate
                }));
            }
            Ok((
                StatusCode::ACCEPTED,
                headers,
                axum::Json(json!({
                    "resourceType": "Parameters",
                    "parameter": params
                })),
            )
                .into_response())
        }

        // Spec (Common Operation Behavior — Asynchronous Delivery): the
        // status poll itself carries the terminal response. Failure returns
        // the relevant error status code with an OperationOutcome body;
        // polling-transport errors and operation failures are distinguished
        // by the status code on the poll response itself.
        Some(JobStatus::Failed { message, .. }) => Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "error",
                    "code": "processing",
                    "diagnostics": format!("Export job '{job_id}' failed: {message}")
                }]
            })),
        )
            .into_response()),

        // Completion is `200 OK` with the manifest in the status-poll body —
        // no `303 See Other` redirect and no separate result resource.
        Some(JobStatus::Completed {
            files,
            submitted_at,
            completed_at,
            format,
            client_tracking_id,
        }) => {
            // Spec: the completed status URL and download URLs SHALL be valid
            // for at least 24 hours and MAY carry an `Expires` header. Format
            // is IMF-fixdate per RFC 7231.
            let expires_at = completed_at + chrono::Duration::hours(24);
            let expires_str = expires_at.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
            let mut headers = HeaderMap::new();
            if let Ok(v) = HeaderValue::from_str(&expires_str) {
                headers.insert(header::EXPIRES, v);
            }
            Ok((
                StatusCode::OK,
                headers,
                axum::Json(build_completion_manifest(
                    state.base_url(),
                    &job_id,
                    &files,
                    submitted_at,
                    completed_at,
                    &format,
                    client_tracking_id.as_deref(),
                )),
            )
                .into_response())
        }
    }
}

/// Constructs the SQL-on-FHIR v2 completion manifest as a FHIR `Parameters` resource.
fn build_completion_manifest(
    base_url: &str,
    job_id: &str,
    files: &[crate::export::controller::CompletedFile],
    submitted_at: chrono::DateTime<chrono::Utc>,
    completed_at: chrono::DateTime<chrono::Utc>,
    format: &str,
    client_tracking_id: Option<&str>,
) -> Value {
    // Spec: one `output` per view, with `location` (1..*) repeating once per
    // shard inside it. Group by `view_name` while preserving first-seen
    // insertion order — this stays correct even if a controller emits files
    // for a view non-contiguously.
    let mut output_order: Vec<String> = Vec::new();
    let mut locations_by_view: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for f in files {
        if !locations_by_view.contains_key(&f.view_name) {
            output_order.push(f.view_name.clone());
        }
        locations_by_view
            .entry(f.view_name.clone())
            .or_default()
            .push(f.url.clone());
    }
    let output: Vec<Value> = output_order
        .into_iter()
        .map(|name| {
            let mut parts = vec![json!({"name": "name", "valueString": &name})];
            for url in locations_by_view.remove(&name).unwrap_or_default() {
                parts.push(json!({"name": "location", "valueUri": url}));
            }
            json!({"name": "output", "part": parts})
        })
        .collect();

    let status_url = format!(
        "{base}/export/{job_id}/status",
        base = base_url.trim_end_matches('/'),
    );
    let duration_secs = (completed_at - submitted_at).num_seconds().max(0);

    let mut params: Vec<Value> = vec![
        json!({"name": "exportId", "valueString": job_id}),
        json!({"name": "status", "valueCode": "completed"}),
        json!({"name": "location", "valueUri": status_url}),
        json!({"name": "cancelUrl", "valueUri": status_url}),
        json!({"name": "_format", "valueCode": format}),
        json!({"name": "exportStartTime", "valueInstant": submitted_at.to_rfc3339()}),
        json!({"name": "exportEndTime", "valueInstant": completed_at.to_rfc3339()}),
        json!({"name": "exportDuration", "valueInteger": duration_secs}),
    ];
    if let Some(tid) = client_tracking_id {
        params.push(json!({"name": "clientTrackingId", "valueString": tid}));
    }
    params.extend(output);

    json!({
        "resourceType": "Parameters",
        "parameter": params
    })
}

// ============================================================================
// Cancel: DELETE /export/{job-id}/status
// ============================================================================

/// Cancel an export job.
pub async fn cancel_export_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    Path(job_id): Path<String>,
) -> Result<Response, RestError>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let controller = match state.export_controller() {
        Some(c) => c,
        None => {
            return Ok((StatusCode::SERVICE_UNAVAILABLE, "export not configured").into_response());
        }
    };

    if controller.cancel(tenant.tenant_id(), &job_id) {
        // Spec: cancellation responds 202 Accepted, not 204 No Content.
        Ok((
            StatusCode::ACCEPTED,
            axum::Json(json!({
                "resourceType": "OperationOutcome",
                "issue": [{"severity": "information", "code": "informational",
                    "diagnostics": format!("Export job '{job_id}' cancellation accepted")}]
            })),
        )
            .into_response())
    } else {
        Ok((
            StatusCode::NOT_FOUND,
            axum::Json(json!({
                "resourceType": "OperationOutcome",
                "issue": [{"severity": "error", "code": "not-found",
                    "diagnostics": format!("Export job '{job_id}' not found")}]
            })),
        )
            .into_response())
    }
}

// ============================================================================
// Download: GET /export/{job-id}/{filename}
// ============================================================================

/// Download a shard file from a completed export job.
pub async fn download_export_file_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    Path((job_id, filename)): Path<(String, String)>,
) -> Result<Response, RestError>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let controller = match state.export_controller() {
        Some(c) => c,
        None => {
            return Ok((StatusCode::SERVICE_UNAVAILABLE, "export not configured").into_response());
        }
    };

    match controller.read_shard(tenant.tenant_id(), &job_id, &filename) {
        None => Ok((
            StatusCode::NOT_FOUND,
            axum::Json(json!({
                "resourceType": "OperationOutcome",
                "issue": [{"severity": "error", "code": "not-found",
                    "diagnostics": format!("File '{filename}' not found for job '{job_id}'")}]
            })),
        )
            .into_response()),
        Some(data) => {
            // Determine Content-Type from extension (G3: include Parquet).
            // `.fhir.ndjson` (newline-delimited Parameters resources from
            // `_format=fhir`) must be checked before the plain ndjson
            // fallthrough.
            let content_type = if filename.ends_with(".csv") {
                "text/csv; charset=utf-8"
            } else if filename.ends_with(".parquet") {
                "application/vnd.apache.parquet"
            } else if filename.ends_with(".fhir.ndjson") {
                "application/fhir+ndjson"
            } else if filename.ends_with(".json") {
                // `_format=json` shards hold a single JSON array of rows.
                "application/json"
            } else {
                "application/x-ndjson"
            };
            Ok((StatusCode::OK, [(header::CONTENT_TYPE, content_type)], data).into_response())
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Merged input parameters for a single export job. Built from the query
/// string and (optionally) a `Parameters` body. Query string wins on conflict.
#[derive(Debug, Clone)]
struct ExportInputs {
    format: String,
    header: bool,
    since: Option<chrono::DateTime<chrono::Utc>>,
    patient: Vec<String>,
    group: Vec<String>,
    client_tracking_id: Option<String>,
}

/// Parses the raw query string into [`ExportQueryParams`], rejecting any
/// keys outside the spec-defined parameter set. Returns a 400 OperationOutcome
/// response on rejection.
#[allow(clippy::result_large_err)]
fn parse_export_query(raw: Option<&str>) -> Result<ExportQueryParams, Response> {
    let raw = raw.unwrap_or("");
    if raw.is_empty() {
        return Ok(ExportQueryParams::default());
    }
    // Validate every key up-front so we can report the offender in the
    // OperationOutcome rather than serde's "unknown field" string.
    const ALLOWED_QUERY: &[&str] = &[
        "_format",
        "header",
        "_since",
        "patient",
        "group",
        "clientTrackingId",
        "source",
    ];
    for (k, _) in url::form_urlencoded::parse(raw.as_bytes()) {
        if !ALLOWED_QUERY.contains(&k.as_ref()) {
            return Err((
                StatusCode::BAD_REQUEST,
                axum::Json(json!({
                    "resourceType": "OperationOutcome",
                    "issue": [{
                        "severity": "error",
                        "code": "not-supported",
                        "diagnostics": format!(
                            "unsupported query parameter '{k}' for $viewdefinition-export"
                        )
                    }]
                })),
            )
                .into_response());
        }
    }
    serde_urlencoded::from_str::<ExportQueryParams>(raw).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "error",
                    "code": "invalid",
                    "diagnostics": format!("invalid query string: {e}")
                }]
            })),
        )
            .into_response()
    })
}

/// Rejects body parameters whose `name` is not in [`ALLOWED_BODY_PARAMS`].
/// Returns `Some(400 response)` on the first offender.
fn validate_unknown_body_params(body: &Value, allowed: &[&str], op: &str) -> Option<Response> {
    let entries = body.get("parameter").and_then(|v| v.as_array())?;
    for entry in entries {
        let name = entry.get("name").and_then(|n| n.as_str())?;
        if !allowed.contains(&name) {
            return Some(
                (
                    StatusCode::BAD_REQUEST,
                    axum::Json(json!({
                        "resourceType": "OperationOutcome",
                        "issue": [{
                            "severity": "error",
                            "code": "not-supported",
                            "diagnostics": format!(
                                "unsupported body parameter '{name}' for {op}"
                            )
                        }]
                    })),
                )
                    .into_response(),
            );
        }
    }
    None
}

/// Merges query parameters and body parameters into a single [`ExportInputs`]
/// view of the request. Query string values take precedence over body values
/// for each scalar field; for the repeating `patient`/`group` lists, a
/// non-empty query value replaces the body list entirely (lists do not merge).
fn merge_export_inputs(query: &ExportQueryParams, body: Option<&Value>) -> ExportInputs {
    let body_params = body
        .and_then(|b| b.get("parameter"))
        .and_then(|p| p.as_array());

    // Body lookups
    let body_format = find_body_value(body_params, "_format", "valueCode")
        .or_else(|| find_body_value(body_params, "_format", "valueString"));
    let body_header = body_params.and_then(|arr| {
        arr.iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("header"))
            .and_then(|p| p.get("valueBoolean"))
            .and_then(|v| v.as_bool())
    });
    let body_since = find_body_value(body_params, "_since", "valueInstant")
        .or_else(|| find_body_value(body_params, "_since", "valueDateTime"));
    let body_tracking = find_body_value(body_params, "clientTrackingId", "valueString")
        .or_else(|| find_body_value(body_params, "clientTrackingId", "valueId"));

    // Repeating refs: collect every `patient`/`group` parameter's
    // `valueReference.reference` (or `valueString` as a permissive fallback).
    let body_patient = collect_body_refs(body_params, "patient");
    let body_group = collect_body_refs(body_params, "group");

    let format = query
        .format
        .clone()
        .or(body_format)
        .unwrap_or_else(|| "ndjson".to_string())
        .to_lowercase();
    let header = query.header.or(body_header).unwrap_or(true);
    let since = query
        .since
        .as_deref()
        .and_then(|s| s.parse().ok())
        .or_else(|| body_since.and_then(|s| s.parse().ok()));
    let client_tracking_id = query.client_tracking_id.clone().or(body_tracking);

    let query_patient = split_refs(query.patient.as_deref());
    let query_group = split_refs(query.group.as_deref());
    let patient = if query_patient.is_empty() {
        body_patient
    } else {
        query_patient
    };
    let group = if query_group.is_empty() {
        body_group
    } else {
        query_group
    };

    ExportInputs {
        format,
        header,
        since,
        patient,
        group,
        client_tracking_id,
    }
}

/// Returns the named body parameter's `value*` string (whatever the value
/// type is — caller picks the field name to read).
fn find_body_value(params: Option<&Vec<Value>>, name: &str, value_field: &str) -> Option<String> {
    params?
        .iter()
        .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))
        .and_then(|p| p.get(value_field))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Collects every occurrence of `name` in the body, reading the FHIR
/// `Reference.reference` string. Falls back to `valueString` for permissive
/// clients that send refs as bare strings.
fn collect_body_refs(params: Option<&Vec<Value>>, name: &str) -> Vec<String> {
    params
        .map(|arr| {
            arr.iter()
                .filter(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))
                .filter_map(|p| {
                    p.get("valueReference")
                        .and_then(|r| r.get("reference"))
                        .and_then(|v| v.as_str())
                        .or_else(|| p.get("valueString").and_then(|v| v.as_str()))
                        .map(|s| s.to_string())
                })
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Validates that every relative `Patient/{id}` and `Group/{id}` reference
/// in the inputs resolves to an existing resource. Returns 404 with an
/// OperationOutcome listing the missing references. Absolute / external
/// references are skipped (we can't reach them).
async fn validate_patient_group_refs<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    inputs: &ExportInputs,
) -> Result<Option<Response>, RestError>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let mut missing: Vec<String> = Vec::new();
    for reference in inputs.patient.iter().chain(inputs.group.iter()) {
        let (resource_type, id) = match parse_relative_compartment_ref(reference) {
            Some(r) => r,
            None => continue, // absolute / unparseable — skip
        };
        let exists = state
            .storage()
            .read(tenant.context(), resource_type, id)
            .await
            .map_err(|e| RestError::InternalError {
                message: format!("failed to check {resource_type}/{id}: {e}"),
            })?
            .is_some();
        if !exists {
            missing.push(reference.clone());
        }
    }
    if missing.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        (
            StatusCode::NOT_FOUND,
            axum::Json(json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "error",
                    "code": "not-found",
                    "diagnostics": format!(
                        "one or more patient/group references could not be resolved: {}",
                        missing.join(", ")
                    )
                }]
            })),
        )
            .into_response(),
    ))
}

/// Returns `(resource_type, id)` for relative refs of the form
/// `Patient/{id}` or `Group/{id}`. Returns `None` for absolute URLs or
/// any other shape.
fn parse_relative_compartment_ref(reference: &str) -> Option<(&'static str, &str)> {
    let trimmed = reference.trim();
    for &t in ["Patient", "Group"].iter() {
        let prefix = format!("{t}/");
        if let Some(rest) = trimmed.strip_prefix(&prefix) {
            let id = rest.split('/').next()?;
            if id.is_empty() {
                return None;
            }
            return Some((t, id));
        }
    }
    None
}

/// Splits a comma-separated query value into trimmed, non-empty refs.
fn split_refs(v: Option<&str>) -> Vec<String> {
    match v {
        Some(s) => s
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect(),
        None => Vec::new(),
    }
}

/// Extracts the list of [`NamedView`] inputs from a submit body.
///
/// Accepts:
/// - A bare `ViewDefinition` resource — produces a single unnamed view.
/// - A `Parameters` resource with a top-level `viewResource` parameter
///   (back-compat single-view shape).
/// - A `Parameters` resource with one or more `view` parameters, each carrying
///   `part` entries `name`, `viewResource`, and/or `viewReference` per the
///   SQL-on-FHIR v2 spec (`view` 1..*).
///
/// References are resolved through storage like `$viewdefinition-run` does.
/// Only relative `ViewDefinition/{id}` references are currently supported.
async fn extract_views_from_body<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    body: &Value,
) -> Result<Vec<NamedView>, RestError>
where
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    let rt = body
        .get("resourceType")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if rt == "ViewDefinition" {
        let name = body
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "output".to_string());
        return Ok(vec![NamedView {
            name,
            view: body.clone(),
        }]);
    }

    if rt != "Parameters" {
        return Err(RestError::BadRequest {
            message: format!("Expected Parameters or ViewDefinition, got '{rt}'"),
        });
    }

    let entries = body
        .get("parameter")
        .and_then(|v| v.as_array())
        .ok_or_else(|| RestError::BadRequest {
            message: "Parameters.parameter must be an array".to_string(),
        })?;

    let mut out: Vec<NamedView> = Vec::new();

    // Back-compat: a top-level `viewResource` is treated as a single view.
    for p in entries {
        if p.get("name").and_then(|n| n.as_str()) == Some("viewResource") {
            if let Some(r) = p.get("resource") {
                let name = r
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "output".to_string());
                out.push(NamedView {
                    name,
                    view: r.clone(),
                });
            }
        }
    }

    // Spec form: every `view` parameter contributes one view, defined by its `part` list.
    for p in entries {
        if p.get("name").and_then(|n| n.as_str()) != Some("view") {
            continue;
        }
        let parts = p.get("part").and_then(|v| v.as_array());
        let mut name: Option<String> = None;
        let mut inline: Option<Value> = None;
        let mut reference: Option<String> = None;

        if let Some(arr) = parts {
            for part in arr {
                match part.get("name").and_then(|v| v.as_str()) {
                    Some("name") => {
                        name = part
                            .get("valueString")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                    Some("viewResource") => {
                        inline = part.get("resource").cloned();
                    }
                    Some("viewReference") => {
                        reference = part
                            .get("valueReference")
                            .and_then(|r| r.get("reference"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                    _ => {}
                }
            }
        }

        let view = match (inline, reference) {
            (Some(_), Some(_)) => {
                // Spec: `view.viewReference` and `view.viewResource` are XOR
                // — exactly one of them must be present.
                return Err(RestError::BadRequest {
                    message: "each `view` parameter must contain exactly one of \
                              `viewResource` or `viewReference` (not both)"
                        .to_string(),
                });
            }
            (Some(r), None) => r,
            (None, Some(reference)) => {
                resolve_view_reference_export(state, tenant, &reference).await?
            }
            (None, None) => {
                return Err(RestError::BadRequest {
                    message:
                        "each `view` parameter must contain a `viewResource` or `viewReference` part"
                            .to_string(),
                });
            }
        };

        let resolved_name = name.unwrap_or_else(|| {
            view.get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("output-{}", out.len()))
        });
        out.push(NamedView {
            name: resolved_name,
            view,
        });
    }

    Ok(out)
}

/// Resolves a FHIR reference to a stored ViewDefinition for use in
/// `$viewdefinition-export`. Accepts:
///
/// - Relative: `ViewDefinition/{id}` — `storage.read(...)`.
/// - Canonical: `http(s)://…` optionally with `…|version` — server registry
///   lookup via `SearchProvider::search` on `url` (+ `version`); newest
///   match by `meta.lastUpdated` wins.
///
/// Absolute external URL *fetch* is not supported; callers must register
/// the artifact on this server first. An absolute URL that matches a
/// registered resource's canonical is resolved successfully — that's why
/// `$sql-on-fhir-capabilities` advertises
/// `supportsAbsoluteReference = true`.
async fn resolve_view_reference_export<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    reference: &str,
) -> Result<Value, RestError>
where
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    let trimmed = reference.trim();
    if let Some(rest) = trimmed.strip_prefix("ViewDefinition/") {
        let id = rest.split('/').next().unwrap_or("").to_string();
        if id.is_empty() {
            return Err(RestError::BadRequest {
                message: format!("viewReference '{reference}' has an empty id"),
            });
        }
        let stored = state
            .storage()
            .read(tenant.context(), "ViewDefinition", &id)
            .await
            .map_err(|e| RestError::InternalError {
                message: format!("failed to read ViewDefinition: {e}"),
            })?
            .ok_or_else(|| RestError::NotFound {
                resource_type: "ViewDefinition".to_string(),
                id: id.clone(),
            })?;
        return Ok(stored.content().clone());
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return resolve_canonical_view_definition(state, tenant.context(), trimmed).await;
    }
    Err(RestError::BadRequest {
        message: format!(
            "viewReference '{reference}' uses an unsupported form; supported: \
             relative `ViewDefinition/{{id}}` or canonical `http(s)://…[|version]`"
        ),
    })
}

/// Resolves a canonical ViewDefinition URL (optionally with `|version`) via
/// `SearchProvider::search` against the local registry. Picks the newest
/// match by `meta.lastUpdated` when multiple resources share the same URL.
async fn resolve_canonical_view_definition<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    url: &str,
) -> Result<Value, RestError>
where
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    let (canonical, version) = match url.split_once('|') {
        Some((u, v)) => (u.to_string(), Some(v.to_string())),
        None => (url.to_string(), None),
    };
    let mut query = SearchQuery::new("ViewDefinition");
    query.parameters.push(SearchParameter {
        name: "url".to_string(),
        param_type: SearchParamType::Uri,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Eq, canonical)],
        chain: Vec::new(),
        components: Vec::new(),
    });
    if let Some(v) = version {
        query.parameters.push(SearchParameter {
            name: "version".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, v)],
            chain: Vec::new(),
            components: Vec::new(),
        });
    }
    let result =
        state
            .storage()
            .search(tenant, &query)
            .await
            .map_err(|e| RestError::InternalError {
                message: format!("canonical lookup failed for ViewDefinition url={url}: {e}"),
            })?;
    let chosen = result
        .resources
        .items
        .into_iter()
        .max_by_key(|r| r.last_modified())
        .ok_or_else(|| RestError::NotFound {
            resource_type: "ViewDefinition".to_string(),
            id: url.to_string(),
        })?;
    Ok(chosen.content().clone())
}
