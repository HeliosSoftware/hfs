//! `$viewdefinition-export` operation handler.
//!
//! Implements the SQL-on-FHIR async bulk export operation:
//!
//! | Route | Method | Description |
//! |-------|--------|-------------|
//! | `/ViewDefinition/$viewdefinition-export` | POST | Submit an export job |
//! | `/ViewDefinition/{id}/$viewdefinition-export` | POST | Submit for stored view |
//! | `/_operations/export/{job-id}` | GET | Poll for job status |
//! | `/_operations/export/{job-id}/$result` | GET | Fetch completion manifest |
//! | `/_operations/export/{job-id}` | DELETE | Cancel job |
//! | `/_operations/export/{job-id}/{filename}` | GET | Download output file |
//!
//! ## Submit response (202)
//!
//! ```text
//! 202 Accepted
//! Content-Location: /_operations/export/{job-id}
//! ```
//!
//! Per spec, callers should send `Prefer: respond-async`; the server returns
//! `400 Bad Request` if the header is missing.
//!
//! ## Poll response
//!
//! - `202 Accepted` + `X-Progress: running` while the job is running
//! - `303 See Other` (Location: `…/$result`) when complete — clients fetch
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
    let location = format!("/_operations/export/{job_id}");

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_LOCATION,
        HeaderValue::from_str(&location)
            .unwrap_or_else(|_| HeaderValue::from_static("/_operations/export/unknown")),
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
// Poll: GET /_operations/export/{job-id}
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

        Some(JobStatus::Running { progress, .. }) => {
            let mut headers = HeaderMap::new();
            if let Ok(v) = HeaderValue::from_str(&progress) {
                headers.insert("x-progress", v);
            }
            // Spec SHOULD: include Retry-After during polling.
            headers.insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
            Ok((
                StatusCode::ACCEPTED,
                headers,
                axum::Json(json!({
                    "resourceType": "OperationOutcome",
                    "issue": [{"severity": "information", "code": "informational",
                        "diagnostics": format!("Export job '{job_id}' is running: {progress}")}]
                })),
            )
                .into_response())
        }

        Some(JobStatus::Failed { message, .. }) => Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({
                "resourceType": "OperationOutcome",
                "issue": [{"severity": "error", "code": "processing",
                    "diagnostics": format!("Export job '{job_id}' failed: {message}")}]
            })),
        )
            .into_response()),

        Some(JobStatus::Completed { .. }) => {
            // Spec: completion sends a 303 See Other pointing to a separate
            // result URL. The manifest itself is served by the result handler.
            let result_url = format!("/_operations/export/{job_id}/$result");
            let mut headers = HeaderMap::new();
            headers.insert(
                header::LOCATION,
                HeaderValue::from_str(&result_url)
                    .unwrap_or_else(|_| HeaderValue::from_static("/_operations/export/")),
            );
            Ok((StatusCode::SEE_OTHER, headers).into_response())
        }
    }
}

/// `GET /_operations/export/{job_id}/$result` — completion manifest.
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
                    "diagnostics": format!("Export job '{job_id}' has not yet completed; poll /_operations/export/{job_id} first")}]
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
        }) => Ok((
            StatusCode::OK,
            axum::Json(build_completion_manifest(
                &job_id,
                &files,
                submitted_at,
                completed_at,
                &format,
                client_tracking_id.as_deref(),
            )),
        )
            .into_response()),
    }
}

/// Constructs the SQL-on-FHIR v2 completion manifest as a FHIR `Parameters` resource.
fn build_completion_manifest(
    job_id: &str,
    files: &[crate::export::controller::CompletedFile],
    submitted_at: chrono::DateTime<chrono::Utc>,
    completed_at: chrono::DateTime<chrono::Utc>,
    format: &str,
    client_tracking_id: Option<&str>,
) -> Value {
    // One `output` parameter per shard, carrying the view name + location.
    let output: Vec<Value> = files
        .iter()
        .map(|f| {
            json!({
                "name": "output",
                "part": [
                    {"name": "name", "valueString": f.view_name},
                    {"name": "location", "valueUri": f.url},
                    {"name": "rowCount", "valueInteger": f.row_count}
                ]
            })
        })
        .collect();

    let status_url = format!("/_operations/export/{job_id}");
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
// Cancel: DELETE /_operations/export/{job-id}
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
// Download: GET /_operations/export/{job-id}/{filename}
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
