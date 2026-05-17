//! `$viewdefinition-export` operation handler.
//!
//! Implements the SQL-on-FHIR async bulk export operation:
//!
//! | Route | Method | Description |
//! |-------|--------|-------------|
//! | `/ViewDefinition/$viewdefinition-export` | POST | Submit an export job |
//! | `/ViewDefinition/{id}/$viewdefinition-export` | POST | Submit for stored view |
//! | `/export/{job-id}/status` | GET | Poll for job status |
//! | `/export/{job-id}/result` | GET | Fetch completion manifest |
//! | `/export/{job-id}/status` | DELETE | Cancel job |
//! | `/export/{job-id}/{filename}` | GET | Download output file |
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
//! - `303 See Other` (Location: `…/result`) when complete — clients fetch
//!   the final manifest from the separate result URL
//! - `404 Not Found` if the job ID is unknown or was cancelled

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use helios_persistence::core::ResourceStorage;
use serde::Deserialize;
use serde_json::{Value, json};

use helios_persistence::tenant::TenantContext;

use crate::error::RestError;
use crate::export::controller::{ExportTask, JobStatus, NamedView};
use crate::extractors::TenantExtractor;
use crate::state::AppState;

/// Query parameters for `$viewdefinition-export`.
#[derive(Debug, Default, Deserialize)]
pub struct ExportQueryParams {
    /// Output format: `ndjson` (default), `csv`, `json`, or `parquet`.
    #[serde(rename = "_format")]
    pub format: Option<String>,

    /// Include a CSV header row (default `true`, CSV format only).
    pub header: Option<bool>,

    /// Maximum number of output rows.
    #[serde(rename = "_limit")]
    pub limit: Option<usize>,

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
///   `part` entries supply `name`, `viewResource`, or `viewReference`.
pub async fn export_view_definition_handler<S>(
    tenant: TenantExtractor,
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    Query(params): Query<ExportQueryParams>,
    axum::Json(body): axum::Json<Value>,
) -> Result<Response, RestError>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    if let Err(resp) = check_prefer_async(&headers) {
        return Ok(resp);
    }
    if let Some(resp) = reject_unsupported_source(&params, Some(&body)) {
        return Ok(resp);
    }
    let views = extract_views_from_body(&state, &tenant, &body).await?;
    if views.is_empty() {
        return Ok(missing_view_response());
    }

    let format = params
        .format
        .clone()
        .unwrap_or_else(|| "ndjson".to_string())
        .to_lowercase();

    submit_export_job(&state, tenant.context().clone(), views, format, &params)
}

/// Submit an export job for a stored ViewDefinition.
pub async fn export_stored_view_definition_handler<S>(
    tenant: TenantExtractor,
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(params): Query<ExportQueryParams>,
) -> Result<Response, RestError>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    if let Err(resp) = check_prefer_async(&headers) {
        return Ok(resp);
    }
    if let Some(resp) = reject_unsupported_source(&params, None) {
        return Ok(resp);
    }

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
    let format = params
        .format
        .clone()
        .unwrap_or_else(|| "ndjson".to_string())
        .to_lowercase();

    submit_export_job(
        &state,
        tenant.context().clone(),
        vec![NamedView {
            name: view_name,
            view,
        }],
        format,
        &params,
    )
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
fn submit_export_job<S>(
    state: &AppState<S>,
    tenant: TenantContext,
    views: Vec<NamedView>,
    format: String,
    params: &ExportQueryParams,
) -> Result<Response, RestError>
where
    S: ResourceStorage + Send + Sync + 'static,
{
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

    // Build filters (G4, G5). patient / group are comma-split per the spec's
    // 0..* cardinality; multiple values match resources from any of the
    // referenced compartments.
    let since = params.since.as_deref().and_then(|s| s.parse().ok());
    let filters = helios_persistence::core::sof_runner::ViewFilters {
        limit: params.limit,
        since,
        patient: split_refs(params.patient.as_deref()),
        group: split_refs(params.group.as_deref()),
    };

    let task = ExportTask {
        views,
        tenant,
        filters,
        format,
        header: params.header.unwrap_or(true),
        client_tracking_id: params.client_tracking_id.clone(),
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

    Ok((
        StatusCode::ACCEPTED,
        headers,
        axum::Json(json!({
            "resourceType": "OperationOutcome",
            "issue": [{"severity": "information", "code": "informational",
                "diagnostics": format!("Export job submitted: {job_id}")
            }]
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

        Some(JobStatus::Running { percent, .. }) => {
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
            Ok((
                StatusCode::ACCEPTED,
                headers,
                axum::Json(json!({
                    "resourceType": "Parameters",
                    "parameter": [
                        {"name": "exportId", "valueString": job_id},
                        {"name": "status", "valueCode": "in-progress"}
                    ]
                })),
            )
                .into_response())
        }

        // Spec: terminal states (success OR failure) both 303 to the result
        // URL. The result handler serves the success manifest with 200, or
        // a 500 + OperationOutcome on failure.
        Some(JobStatus::Failed { .. }) | Some(JobStatus::Completed { .. }) => {
            let result_url = format!(
                "{base}/export/{job_id}/result",
                base = state.base_url().trim_end_matches('/'),
            );
            let mut headers = HeaderMap::new();
            headers.insert(
                header::LOCATION,
                HeaderValue::from_str(&result_url)
                    .unwrap_or_else(|_| HeaderValue::from_static("/export/")),
            );
            Ok((StatusCode::SEE_OTHER, headers).into_response())
        }
    }
}

/// `GET /export/{job_id}/result` — completion manifest.
///
/// Per spec, the result URL is distinct from the status URL: clients reach
/// here after following the `303 See Other` redirect on a completed poll.
pub async fn get_export_result_handler<S>(
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

        Some(JobStatus::Running { .. }) => Ok((
            StatusCode::PRECONDITION_FAILED,
            axum::Json(json!({
                "resourceType": "OperationOutcome",
                "issue": [{"severity": "error", "code": "exception",
                    "diagnostics": format!("Export job '{job_id}' has not yet completed; poll /export/{job_id}/status first")}]
            })),
        )
            .into_response()),

        Some(JobStatus::Failed { message, .. }) => Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({
                "resourceType": "OperationOutcome",
                "issue": [{"severity": "error", "code": "processing",
                    "diagnostics": format!("Export job '{job_id}' failed: {message}")}]
            })),
        )
            .into_response()),

        Some(JobStatus::Completed {
            files,
            submitted_at,
            completed_at,
            format,
            client_tracking_id,
        }) => {
            // Spec: result URLs SHALL be valid for at least 24 hours and MAY
            // carry an `Expires` header. Format is IMF-fixdate per RFC 7231.
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
    // shard inside it. `files` is already in view-then-shard order, so we
    // collapse runs of equal `view_name` into a single output entry.
    let mut output: Vec<Value> = Vec::new();
    for f in files {
        let last_matches = output
            .last()
            .and_then(|o| o.get("part"))
            .and_then(|p| p.as_array())
            .and_then(|arr| arr.iter().find(|p| p["name"] == "name"))
            .and_then(|p| p["valueString"].as_str())
            == Some(f.view_name.as_str());
        if last_matches {
            // Append another `location` part to the in-progress output entry.
            if let Some(parts) = output
                .last_mut()
                .and_then(|o| o.get_mut("part"))
                .and_then(|p| p.as_array_mut())
            {
                parts.push(json!({"name": "location", "valueUri": f.url}));
            }
        } else {
            output.push(json!({
                "name": "output",
                "part": [
                    {"name": "name", "valueString": f.view_name},
                    {"name": "location", "valueUri": f.url}
                ]
            }));
        }
    }

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
            // Determine Content-Type from extension (G3: include Parquet)
            let content_type = if filename.ends_with(".csv") {
                "text/csv; charset=utf-8"
            } else if filename.ends_with(".parquet") {
                "application/octet-stream"
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
    S: ResourceStorage + Send + Sync + 'static,
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

        let view = if let Some(r) = inline {
            r
        } else if let Some(reference) = reference {
            resolve_view_reference_export(state, tenant, &reference).await?
        } else {
            return Err(RestError::BadRequest {
                message:
                    "each `view` parameter must contain a `viewResource` or `viewReference` part"
                        .to_string(),
            });
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
/// `$viewdefinition-export`. Mirrors the relative-only behavior of the
/// `$viewdefinition-run` handler.
async fn resolve_view_reference_export<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    reference: &str,
) -> Result<Value, RestError>
where
    S: ResourceStorage + Send + Sync + 'static,
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
    Err(RestError::BadRequest {
        message: format!(
            "viewReference '{reference}' uses an unsupported form; supported: 'ViewDefinition/{{id}}'"
        ),
    })
}
