//! `$viewdefinition-run` operation handler.
//!
//! Implements the SQL-on-FHIR
//! [`$viewdefinition-run`](https://build.fhir.org/ig/FHIR/sql-on-fhir-v2/operations-viewdefinition-run.html)
//! operation in two forms:
//!
//! - `POST /ViewDefinition/$viewdefinition-run` — supply the ViewDefinition inline in the body
//! - `POST /ViewDefinition/{id}/$viewdefinition-run` — run a stored ViewDefinition
//!
//! ## Request body
//!
//! Accepts a FHIR `Parameters` resource or a raw `ViewDefinition` JSON object.
//!
//! | Parameter | Type | Description |
//! |-----------|------|-------------|
//! | `viewResource` | Resource | The ViewDefinition to execute (Parameters form) |
//! | `patient` | string | Restrict to this patient reference |
//! | `group` | string | Restrict to this group reference |
//! | `_format` | string | Output format: `ndjson` (default), `csv`, `json` |
//! | `_limit` | integer | Maximum number of output rows |
//! | `_since` | instant | Only include resources modified after this time |
//!
//! ## Response
//!
//! - `200 OK` — stream of output rows in the requested format
//! - `422 Unprocessable Entity` — ViewDefinition could not be compiled or executed

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures::StreamExt;
use helios_persistence::core::search::SearchProvider;
use helios_persistence::core::sof_runner::{SofError, SofRunner, ViewFilters};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::error::RestError;
use crate::extractors::TenantExtractor;
use crate::sof::in_process::InProcessRunner;
use crate::state::AppState;

/// Query parameters for `$viewdefinition-run`.
#[derive(Debug, Deserialize)]
pub struct RunQueryParams {
    /// Output format: `ndjson` (default), `csv`, `json`.
    #[serde(rename = "_format")]
    pub format: Option<String>,

    /// Whether to include a CSV header row.
    pub header: Option<String>,

    /// Limit the number of output rows.
    #[serde(rename = "_limit")]
    pub limit: Option<usize>,

    /// Include only resources modified at or after this instant (RFC 3339).
    #[serde(rename = "_since")]
    pub since: Option<String>,

    /// Override runner: `inprocess` forces the in-process FHIRPath runner.
    pub runner: Option<String>,

    /// Filter by patient reference (e.g. `Patient/123`).
    pub patient: Option<String>,

    /// Filter by group reference.
    pub group: Option<String>,
}

/// `POST /ViewDefinition/$viewdefinition-run`
///
/// The ViewDefinition must be supplied in the request body either as:
/// - A raw `ViewDefinition` JSON object, or
/// - A FHIR `Parameters` resource with a `viewResource` parameter.
pub async fn run_view_definition_handler<S>(
    State(state): State<AppState<S>>,
    Query(params): Query<RunQueryParams>,
    tenant: TenantExtractor,
    _headers: HeaderMap,
    body: axum::extract::Json<Value>,
) -> Result<impl IntoResponse, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let view_json = extract_view_definition(body.0)?;
    execute_view(state, params, tenant, view_json).await
}

/// `POST /ViewDefinition/{id}/$viewdefinition-run`
///
/// Looks up the stored ViewDefinition by ID, then runs it.
/// Additional `viewResource` in the body overrides the stored definition.
pub async fn run_stored_view_definition_handler<S>(
    State(state): State<AppState<S>>,
    Path(_id): Path<String>,
    Query(params): Query<RunQueryParams>,
    tenant: TenantExtractor,
    _headers: HeaderMap,
    body: axum::extract::Json<Value>,
) -> Result<impl IntoResponse, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    // For Phase 1: treat body the same as the anonymous form.
    // Phase 2 will add ViewDefinition lookup by ID from storage.
    let view_json = extract_view_definition(body.0)?;
    execute_view(state, params, tenant, view_json).await
}

/// Extracts a ViewDefinition from a request body.
///
/// Accepts either:
/// - A raw `ViewDefinition` object (`resourceType == "ViewDefinition"`)
/// - A `Parameters` resource with a `viewResource` parameter
fn extract_view_definition(body: Value) -> Result<Value, RestError> {
    match body.get("resourceType").and_then(|v| v.as_str()) {
        Some("ViewDefinition") => Ok(body),
        Some("Parameters") => {
            // Extract the viewResource parameter value
            body.get("parameter")
                .and_then(|p| p.as_array())
                .and_then(|params| {
                    params.iter().find(|p| {
                        p.get("name").and_then(|n| n.as_str()) == Some("viewResource")
                    })
                })
                .and_then(|p| p.get("resource"))
                .cloned()
                .ok_or_else(|| RestError::BadRequest {
                    message: "Parameters body must contain a 'viewResource' parameter with the ViewDefinition resource".to_string(),
                })
        }
        Some(other) => Err(RestError::BadRequest {
            message: format!(
                "Expected a ViewDefinition or Parameters body, got resourceType='{}'",
                other
            ),
        }),
        None => Err(RestError::BadRequest {
            message: "Request body must be a FHIR JSON resource with a 'resourceType' field"
                .to_string(),
        }),
    }
}

/// Resolves the SofRunner and executes the view, returning a streaming response.
///
/// Handles G2 (Parquet output), G6 (auto-fallback on Uncompilable), and
/// adds an `X-HFS-Runner` header identifying which runner produced the result.
async fn execute_view<S>(
    state: AppState<S>,
    params: RunQueryParams,
    tenant: TenantExtractor,
    view_json: Value,
) -> Result<Response, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let runner: Arc<dyn SofRunner> = resolve_runner(&state, &params);
    let filters = build_filters(&params);
    let format = params.format.as_deref().unwrap_or("ndjson").to_lowercase();
    let include_header = params
        .header
        .as_deref()
        .map(|h| h == "true" || h == "1")
        .unwrap_or(true);

    debug!(
        runner = runner.runner_name(),
        tenant = %tenant.tenant_id(),
        format = %format,
        "dispatching $viewdefinition-run"
    );

    // Determine whether auto-fallback is permitted (G6)
    let is_inprocess = runner.runner_name() == "inprocess";
    let forced_inprocess = params
        .runner
        .as_deref()
        .map(|r| r.to_lowercase() == "inprocess")
        .unwrap_or(false);
    let can_fallback = !is_inprocess
        && !forced_inprocess
        && state.config().sof_default_runner.to_lowercase() == "auto";

    // First attempt with the resolved runner
    match runner
        .run_view(tenant.context(), view_json.clone(), filters.clone())
        .await
    {
        Ok(stream) => {
            let runner_label = runner.runner_name().to_string();
            let (ct, body) = format_stream(stream, &format, include_header).await;
            Ok(build_response(
                StatusCode::OK,
                ct,
                body,
                &runner_label,
                &format,
            ))
        }
        Err(SofError::Uncompilable { reason }) if can_fallback => {
            // G6: transparently fall back to in-process runner
            warn!(
                runner = runner.runner_name(),
                reason = %reason,
                "in-DB runner returned Uncompilable; falling back to in-process runner"
            );
            let fallback = Arc::new(InProcessRunner::new(
                state.storage_arc(),
                state.config().default_fhir_version,
            ));
            let stream = fallback
                .run_view(tenant.context(), view_json, filters)
                .await
                .map_err(map_sof_error_to_rest)?;
            let runner_label = format!("inprocess (fallback: {reason})");
            let (ct, body) = format_stream(stream, &format, include_header).await;
            Ok(build_response(
                StatusCode::OK,
                ct,
                body,
                &runner_label,
                &format,
            ))
        }
        Err(e) => Err(map_sof_error_to_rest(e)),
    }
}

/// Renders a `RowStream` to `(content_type, bytes)` for the requested format.
async fn format_stream(
    stream: helios_persistence::core::sof_runner::RowStream<'_>,
    format: &str,
    include_header: bool,
) -> (&'static str, Vec<u8>) {
    match format {
        "csv" | "text/csv" => {
            let body = stream_to_csv(stream, include_header).await;
            ("text/csv; charset=utf-8", body)
        }
        "json" | "application/json" => {
            let body = stream_to_json_array(stream).await;
            ("application/json", body)
        }
        "parquet" | "application/octet-stream" => {
            let body = stream_to_parquet(stream).await;
            ("application/octet-stream", body)
        }
        _ => {
            let body = stream_to_ndjson(stream).await;
            ("application/x-ndjson", body)
        }
    }
}

/// Builds the final `Response` with `X-HFS-Runner` and optional Content-Disposition.
fn build_response(
    status: StatusCode,
    content_type: &'static str,
    body: Vec<u8>,
    runner_label: &str,
    format: &str,
) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        "x-hfs-runner",
        HeaderValue::from_str(runner_label).unwrap_or_else(|_| HeaderValue::from_static("unknown")),
    );
    if format == "parquet" || format == "application/octet-stream" {
        headers.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=\"output.parquet\""),
        );
    }
    (status, headers, body).into_response()
}

/// Selects the SofRunner based on state and query params.
fn resolve_runner<S: SearchProvider + Send + Sync + 'static>(
    state: &AppState<S>,
    params: &RunQueryParams,
) -> Arc<dyn SofRunner> {
    // Allow per-request override via ?runner=inprocess
    if params
        .runner
        .as_deref()
        .map(|r| r.to_lowercase() == "inprocess")
        .unwrap_or(false)
    {
        return Arc::new(InProcessRunner::new(
            state.storage_arc(),
            state.config().default_fhir_version,
        ));
    }

    // Use the pre-wired runner from AppState (set at startup)
    #[cfg(feature = "sof")]
    if let Some(runner) = state.sof_runner() {
        return Arc::clone(runner);
    }

    // Fallback: create a fresh InProcessRunner
    Arc::new(InProcessRunner::new(
        state.storage_arc(),
        state.config().default_fhir_version,
    ))
}

/// Builds `ViewFilters` from query parameters.
fn build_filters(params: &RunQueryParams) -> ViewFilters {
    let since = params.since.as_deref().and_then(|s| s.parse().ok());

    ViewFilters {
        patient: params.patient.clone(),
        group: params.group.clone(),
        since,
        limit: params.limit,
    }
}

/// Maps a `SofError` to a `RestError`, returning 422 for uncompilable views.
fn map_sof_error_to_rest(e: SofError) -> RestError {
    match e {
        SofError::Uncompilable { reason } | SofError::InvalidViewDefinition(reason) => {
            RestError::UnprocessableEntity { message: reason }
        }
        SofError::Cancelled => RestError::InternalError {
            message: "View execution was cancelled".to_string(),
        },
        other => {
            warn!(error = %other, "SofRunner error");
            RestError::InternalError {
                message: other.to_string(),
            }
        }
    }
}

/// Collects the row stream into Parquet bytes (G2).
async fn stream_to_parquet(
    mut stream: helios_persistence::core::sof_runner::RowStream<'_>,
) -> Vec<u8> {
    let mut rows: Vec<Value> = Vec::new();
    while let Some(result) = stream.next().await {
        match result {
            Ok(row) => rows.push(row),
            Err(e) => {
                warn!(error = %e, "row error during Parquet streaming");
                break;
            }
        }
    }

    if rows.is_empty() {
        return Vec::new();
    }

    // Build a ProcessedResult from the flat JSON rows
    let columns: Vec<String> = if let Value::Object(map) = &rows[0] {
        map.keys().cloned().collect()
    } else {
        return Vec::new();
    };

    let processed_rows: Vec<helios_sof::ProcessedRow> = rows
        .iter()
        .map(|row| {
            let values = columns
                .iter()
                .map(|col| {
                    if let Value::Object(map) = row {
                        map.get(col).cloned()
                    } else {
                        None
                    }
                })
                .collect();
            helios_sof::ProcessedRow { values }
        })
        .collect();

    let result = helios_sof::ProcessedResult {
        columns,
        rows: processed_rows,
    };

    // Use a very large max_file_size to produce a single Parquet file
    match helios_sof::format_parquet_multi_file(result, None, usize::MAX) {
        Ok(files) => files.into_iter().next().unwrap_or_default(),
        Err(e) => {
            warn!(error = %e, "Parquet serialisation failed");
            Vec::new()
        }
    }
}

/// Collects the row stream into a NDJSON byte string.
async fn stream_to_ndjson(
    mut stream: helios_persistence::core::sof_runner::RowStream<'_>,
) -> Vec<u8> {
    let mut buf = Vec::new();
    while let Some(result) = stream.next().await {
        match result {
            Ok(row) => {
                if let Ok(line) = serde_json::to_string(&row) {
                    buf.extend_from_slice(line.as_bytes());
                    buf.push(b'\n');
                }
            }
            Err(e) => {
                warn!(error = %e, "row error during NDJSON streaming");
                break;
            }
        }
    }
    buf
}

/// Collects the row stream into a JSON array byte string.
async fn stream_to_json_array(
    mut stream: helios_persistence::core::sof_runner::RowStream<'_>,
) -> Vec<u8> {
    let mut rows = Vec::new();
    while let Some(result) = stream.next().await {
        match result {
            Ok(row) => rows.push(row),
            Err(e) => {
                warn!(error = %e, "row error during JSON array streaming");
                break;
            }
        }
    }
    serde_json::to_vec(&rows).unwrap_or_default()
}

/// Collects the row stream into CSV bytes.
async fn stream_to_csv(
    mut stream: helios_persistence::core::sof_runner::RowStream<'_>,
    include_header: bool,
) -> Vec<u8> {
    let mut rows: Vec<Value> = Vec::new();
    while let Some(result) = stream.next().await {
        match result {
            Ok(row) => rows.push(row),
            Err(e) => {
                warn!(error = %e, "row error during CSV streaming");
                break;
            }
        }
    }

    if rows.is_empty() {
        return Vec::new();
    }

    let mut buf = Vec::new();

    // Collect column names from first row
    let columns: Vec<String> = if let Value::Object(map) = &rows[0] {
        map.keys().cloned().collect()
    } else {
        return Vec::new();
    };

    // Header row
    if include_header {
        let header_line = columns.join(",");
        buf.extend_from_slice(header_line.as_bytes());
        buf.push(b'\n');
    }

    // Data rows
    for row in &rows {
        if let Value::Object(map) = row {
            let values: Vec<String> = columns
                .iter()
                .map(|col| {
                    match map.get(col) {
                        Some(Value::String(s)) => {
                            // Escape strings with quotes if they contain commas or quotes
                            if s.contains(',') || s.contains('"') || s.contains('\n') {
                                format!("\"{}\"", s.replace('"', "\"\""))
                            } else {
                                s.clone()
                            }
                        }
                        Some(Value::Null) | None => String::new(),
                        Some(v) => v.to_string(),
                    }
                })
                .collect();
            let line = values.join(",");
            buf.extend_from_slice(line.as_bytes());
            buf.push(b'\n');
        }
    }

    buf
}
