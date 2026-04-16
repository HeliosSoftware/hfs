//! `$viewdefinition-export` operation handler.
//!
//! Implements the SQL-on-FHIR async bulk export operation:
//!
//! | Route | Method | Description |
//! |-------|--------|-------------|
//! | `/ViewDefinition/$viewdefinition-export` | POST | Submit an export job |
//! | `/ViewDefinition/{id}/$viewdefinition-export` | POST | Submit for stored view |
//! | `/_operations/export/{job-id}` | GET | Poll for job status |
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
//! ## Poll response
//!
//! - `202 Accepted` + `X-Progress: running` while the job is running
//! - `200 OK` with a FHIR `Parameters` manifest when completed
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
use crate::export::controller::{ExportTask, JobStatus};
use crate::extractors::TenantExtractor;
use crate::state::AppState;

/// Query parameters for `$viewdefinition-export`.
#[derive(Debug, Deserialize)]
pub struct ExportQueryParams {
    /// Output format: `ndjson` (default), `csv`, or `parquet`.
    #[serde(rename = "_format")]
    pub format: Option<String>,

    /// Maximum number of output rows.
    #[serde(rename = "_limit")]
    pub limit: Option<usize>,

    /// Include only resources modified at or after this instant (RFC 3339).
    #[serde(rename = "_since")]
    pub since: Option<String>,

    /// Filter to resources belonging to this patient reference (e.g. `Patient/123`).
    pub patient: Option<String>,

    /// Filter to resources belonging to this group reference.
    pub group: Option<String>,
}

// ============================================================================
// Submit: POST /ViewDefinition/$viewdefinition-export
// ============================================================================

/// Submit an export job with an inline ViewDefinition.
pub async fn export_view_definition_handler<S>(
    tenant: TenantExtractor,
    State(state): State<AppState<S>>,
    Query(params): Query<ExportQueryParams>,
    axum::Json(body): axum::Json<Value>,
) -> Result<Response, RestError>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let view = extract_view_from_body(&body)?;
    let format = params
        .format
        .clone()
        .unwrap_or_else(|| "ndjson".to_string())
        .to_lowercase();

    submit_export_job(&state, tenant.context().clone(), view, format, &params)
}

/// Submit an export job for a stored ViewDefinition.
pub async fn export_stored_view_definition_handler<S>(
    tenant: TenantExtractor,
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
    Query(params): Query<ExportQueryParams>,
) -> Result<Response, RestError>
where
    S: ResourceStorage + Send + Sync + 'static,
{
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
    let format = params
        .format
        .clone()
        .unwrap_or_else(|| "ndjson".to_string())
        .to_lowercase();

    submit_export_job(&state, tenant.context().clone(), view, format, &params)
}

/// Common submit logic: validate, dispatch to controller, return 202.
fn submit_export_job<S>(
    state: &AppState<S>,
    tenant: TenantContext,
    view: Value,
    format: String,
    params: &ExportQueryParams,
) -> Result<Response, RestError>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    // Validate the view has a resource type field (basic check)
    if view.get("resource").and_then(|v| v.as_str()).is_none() {
        return Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            axum::Json(json!({
                "resourceType": "OperationOutcome",
                "issue": [{"severity": "error", "code": "invalid",
                    "diagnostics": "ViewDefinition.resource is required"}]
            })),
        )
            .into_response());
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

    // Build filters (G4, G5)
    let since = params.since.as_deref().and_then(|s| s.parse().ok());
    let filters = helios_persistence::core::sof_runner::ViewFilters {
        limit: params.limit,
        since,
        patient: params.patient.clone(),
        group: params.group.clone(),
    };

    let task = ExportTask {
        view_definition: view,
        tenant,
        filters,
        format,
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

    match controller.get_status(&job_id) {
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

        Some(JobStatus::Completed {
            files,
            submitted_at,
            completed_at,
        }) => {
            let output: Vec<Value> = files
                .iter()
                .map(|f| {
                    json!({
                        "name": "output",
                        "valueAttachment": {
                            "url": f.url,
                            "extension": [{
                                "url": "http://hl7.org/fhir/uv/sql-on-fhir/StructureDefinition/row-count",
                                "valueInteger": f.row_count
                            }]
                        }
                    })
                })
                .collect();

            let mut params: Vec<Value> = vec![
                json!({"name": "jobId", "valueString": job_id}),
                json!({"name": "submittedAt", "valueInstant": submitted_at.to_rfc3339()}),
                json!({"name": "completedAt", "valueInstant": completed_at.to_rfc3339()}),
                json!({"name": "outputCount", "valueInteger": files.len()}),
            ];
            params.extend(output);

            let manifest = json!({
                "resourceType": "Parameters",
                "parameter": params
            });

            Ok((StatusCode::OK, axum::Json(manifest)).into_response())
        }
    }
}

// ============================================================================
// Cancel: DELETE /_operations/export/{job-id}
// ============================================================================

/// Cancel an export job.
pub async fn cancel_export_handler<S>(
    State(state): State<AppState<S>>,
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

    if controller.cancel(&job_id) {
        Ok(StatusCode::NO_CONTENT.into_response())
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

    match controller.read_shard(&job_id, &filename) {
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

/// Extracts a ViewDefinition from a request body.
///
/// Accepts either:
/// - A raw `ViewDefinition` object
/// - A `Parameters` resource with a `viewResource` parameter
fn extract_view_from_body(body: &Value) -> Result<Value, RestError> {
    let rt = body
        .get("resourceType")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if rt == "Parameters" {
        if let Some(params) = body.get("parameter").and_then(|v| v.as_array()) {
            for p in params {
                if p.get("name").and_then(|v| v.as_str()) == Some("viewResource") {
                    if let Some(r) = p.get("resource") {
                        return Ok(r.clone());
                    }
                }
            }
        }
        Err(RestError::BadRequest {
            message: "Parameters body missing 'viewResource' parameter".to_string(),
        })
    } else if rt == "ViewDefinition" {
        Ok(body.clone())
    } else {
        Err(RestError::BadRequest {
            message: format!("Expected Parameters or ViewDefinition, got '{rt}'"),
        })
    }
}
