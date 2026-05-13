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
///
/// `patient` and `group` accept either a single reference or a comma-separated
/// list (spec is `0..*`). Repeated entries supplied in a `Parameters` body are
/// merged in via [`merge_params`] and take precedence.
#[derive(Debug, Default, Deserialize)]
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

    /// Filter by patient references (comma-separated for multiple).
    pub patient: Option<String>,

    /// Filter by group references (comma-separated for multiple).
    pub group: Option<String>,
}

/// Splits a comma-separated query value into trimmed, non-empty references.
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

/// `POST /ViewDefinition/$viewdefinition-run`
///
/// The ViewDefinition must be supplied in the request body either as:
/// - A raw `ViewDefinition` JSON object, or
/// - A FHIR `Parameters` resource with a `viewResource` parameter.
///
/// When the body is a `Parameters` resource, additional parameter entries
/// (`_format`, `_limit`, `_since`, `patient`, `group`, `header`) override
/// the corresponding query-string values per the SQL-on-FHIR spec.
pub async fn run_view_definition_handler<S>(
    State(state): State<AppState<S>>,
    Query(query_params): Query<RunQueryParams>,
    tenant: TenantExtractor,
    _headers: HeaderMap,
    body: axum::extract::Json<Value>,
) -> Result<impl IntoResponse, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let body_params = extract_body_params(&body.0);
    let view_json = resolve_view_from_body(&state, &tenant, &body.0).await?;
    let params = merge_params(query_params, &body_params);
    execute_view(state, params, body_params, tenant, view_json).await
}

/// `POST /ViewDefinition/{id}/$viewdefinition-run`
///
/// Looks up the stored ViewDefinition by ID and runs it. If the body contains
/// a `viewResource` (or is itself a `ViewDefinition` resource), the body
/// overrides the stored definition.
pub async fn run_stored_view_definition_handler<S>(
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
    Query(query_params): Query<RunQueryParams>,
    tenant: TenantExtractor,
    _headers: HeaderMap,
    body: axum::extract::Json<Value>,
) -> Result<impl IntoResponse, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let body_params = extract_body_params(&body.0);
    // If the body provides a ViewDefinition (inline or by reference), prefer
    // it. Otherwise, load the stored ViewDefinition by id from the path.
    let view_json = if body_has_view(&body.0) {
        resolve_view_from_body(&state, &tenant, &body.0).await?
    } else {
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
        stored.content().clone()
    };
    let params = merge_params(query_params, &body_params);
    execute_view(state, params, body_params, tenant, view_json).await
}

/// Parameters extracted from a FHIR `Parameters` body. Anything not present
/// in the body stays `None`/empty so the merge step preserves the query-string
/// value. `patient` and `group` collect every repeated entry (spec is 0..*).
#[derive(Debug, Default)]
struct BodyParams {
    format: Option<String>,
    header: Option<String>,
    limit: Option<usize>,
    since: Option<String>,
    patient: Vec<String>,
    group: Vec<String>,
    /// Inline `resource` parameter values (any number; spec 0..*). Drives the
    /// in-process runner when present so the view runs against these resources
    /// instead of the tenant's stored data.
    inline_resources: Vec<Value>,
}

/// Reads SoF-spec parameters out of a FHIR `Parameters` body. Returns an empty
/// `BodyParams` for any non-Parameters body (e.g. a bare ViewDefinition).
fn extract_body_params(body: &Value) -> BodyParams {
    if body.get("resourceType").and_then(|v| v.as_str()) != Some("Parameters") {
        return BodyParams::default();
    }
    let Some(entries) = body.get("parameter").and_then(|p| p.as_array()) else {
        return BodyParams::default();
    };

    let mut out = BodyParams::default();
    for p in entries {
        let name = match p.get("name").and_then(|n| n.as_str()) {
            Some(n) => n,
            None => continue,
        };
        match name {
            "_format" => {
                out.format = p
                    .get("valueCode")
                    .or_else(|| p.get("valueString"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
            }
            "header" => {
                if let Some(b) = p.get("valueBoolean").and_then(|v| v.as_bool()) {
                    out.header = Some(if b { "true" } else { "false" }.to_string());
                } else if let Some(s) = p.get("valueString").and_then(|v| v.as_str()) {
                    out.header = Some(s.to_string());
                }
            }
            "_limit" => {
                out.limit = p
                    .get("valueInteger")
                    .or_else(|| p.get("valuePositiveInt"))
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize);
            }
            "_since" => {
                out.since = p
                    .get("valueInstant")
                    .or_else(|| p.get("valueDateTime"))
                    .or_else(|| p.get("valueString"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
            }
            "patient" => {
                if let Some(s) = p
                    .get("valueReference")
                    .and_then(|r| r.get("reference"))
                    .or_else(|| p.get("valueString"))
                    .and_then(|v| v.as_str())
                {
                    out.patient.push(s.to_string());
                }
            }
            "group" => {
                if let Some(s) = p
                    .get("valueReference")
                    .and_then(|r| r.get("reference"))
                    .or_else(|| p.get("valueString"))
                    .and_then(|v| v.as_str())
                {
                    out.group.push(s.to_string());
                }
            }
            "resource" => {
                if let Some(r) = p.get("resource") {
                    out.inline_resources.push(r.clone());
                }
            }
            _ => {}
        }
    }
    out
}

/// Merges body parameters onto query-string parameters with body precedence
/// for scalar values. Multi-valued fields (`patient`, `group`) and inline
/// resources stay on the [`BodyParams`] and are consumed in [`build_filters`]
/// / [`execute_view`].
fn merge_params(query: RunQueryParams, body: &BodyParams) -> RunQueryParams {
    RunQueryParams {
        format: body.format.clone().or(query.format),
        header: body.header.clone().or(query.header),
        limit: body.limit.or(query.limit),
        since: body.since.clone().or(query.since),
        runner: query.runner,
        patient: query.patient,
        group: query.group,
    }
}

/// Returns `true` when the body carries a ViewDefinition the handler should use
/// instead of loading from storage. Accepts either a bare `ViewDefinition`
/// resource or a `Parameters` body containing a `viewResource` *or*
/// `viewReference` parameter.
fn body_has_view(body: &Value) -> bool {
    match body.get("resourceType").and_then(|v| v.as_str()) {
        Some("ViewDefinition") => true,
        Some("Parameters") => body
            .get("parameter")
            .and_then(|p| p.as_array())
            .map(|params| {
                params.iter().any(|p| {
                    matches!(
                        p.get("name").and_then(|n| n.as_str()),
                        Some("viewResource") | Some("viewReference")
                    )
                })
            })
            .unwrap_or(false),
        _ => false,
    }
}

/// Resolves a ViewDefinition from a request body, fetching from storage when
/// the caller supplies a `viewReference` instead of an inline `viewResource`.
/// Supports relative references of the form `ViewDefinition/{id}`; canonical
/// and absolute URL forms are rejected with a 400 until they are wired up.
async fn resolve_view_from_body<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    body: &Value,
) -> Result<Value, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    // Bare ViewDefinition body is used as-is.
    if body.get("resourceType").and_then(|v| v.as_str()) == Some("ViewDefinition") {
        return Ok(body.clone());
    }

    // Parameters body: look for viewResource first, fall back to viewReference.
    if body.get("resourceType").and_then(|v| v.as_str()) == Some("Parameters") {
        let entries = body.get("parameter").and_then(|p| p.as_array());

        // 1. Inline viewResource takes precedence when both are present.
        if let Some(arr) = entries {
            if let Some(view) = arr
                .iter()
                .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("viewResource"))
                .and_then(|p| p.get("resource"))
            {
                return Ok(view.clone());
            }
        }

        // 2. Otherwise, resolve viewReference.
        if let Some(arr) = entries {
            if let Some(reference) = arr
                .iter()
                .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("viewReference"))
                .and_then(|p| p.get("valueReference"))
                .and_then(|r| r.get("reference"))
                .and_then(|v| v.as_str())
            {
                return resolve_view_reference(state, tenant, reference).await;
            }
        }

        return Err(RestError::BadRequest {
            message: "Parameters body must contain a 'viewResource' or 'viewReference' parameter"
                .to_string(),
        });
    }

    // Anything else is an error.
    let rt = body
        .get("resourceType")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    Err(RestError::BadRequest {
        message: format!("Expected a ViewDefinition or Parameters body, got resourceType='{rt}'"),
    })
}

/// Resolves a FHIR reference string into a stored ViewDefinition.
///
/// Supports:
/// - Relative references: `ViewDefinition/{id}` → `storage.read(...)`
///
/// Canonical (`http://example.org/...`) and absolute references are not yet
/// implemented; they return a 400 with a descriptive OperationOutcome. The
/// `$sql-on-fhir-capabilities` response advertises this via
/// `supportsCanonicalReference` / `supportsAbsoluteReference = false`.
async fn resolve_view_reference<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    reference: &str,
) -> Result<Value, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let trimmed = reference.trim();
    // Relative form: "ViewDefinition/{id}" (optionally /_history/{vid} suffix is ignored).
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
            "viewReference '{reference}' uses an unsupported form; \
             this server currently supports only relative references like \
             'ViewDefinition/{{id}}'. See `/$sql-on-fhir-capabilities` for details."
        ),
    })
}

/// Resolves the SofRunner and executes the view, returning a streaming response.
///
/// Handles G2 (Parquet output), G6 (auto-fallback on Uncompilable), and
/// adds an `X-HFS-Runner` header identifying which runner produced the result.
async fn execute_view<S>(
    state: AppState<S>,
    params: RunQueryParams,
    body_params: BodyParams,
    tenant: TenantExtractor,
    view_json: Value,
) -> Result<Response, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    // Inline resources force the in-process runner — in-DB runners can only
    // see stored data. We feed the inline resources directly into the runner
    // and skip backend reads entirely.
    let has_inline = !body_params.inline_resources.is_empty();

    let runner: Arc<dyn SofRunner> = if has_inline {
        Arc::new(InProcessRunner::with_inline_resources(
            state.storage_arc(),
            state.config().default_fhir_version,
            body_params.inline_resources.clone(),
        ))
    } else {
        resolve_runner(&state, &params)
    };
    let filters = build_filters(&params, &body_params);
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

    // Determine whether auto-fallback is permitted (G6). Auto-fallback is
    // disabled when we're already on the in-process runner (including the
    // inline-resources path).
    let is_inprocess = runner.runner_name() == "inprocess";
    let forced_inprocess = params
        .runner
        .as_deref()
        .map(|r| r.to_lowercase() == "inprocess")
        .unwrap_or(false);
    let can_fallback = !is_inprocess
        && !forced_inprocess
        && !has_inline
        && state.config().sof_default_runner.to_lowercase() == "auto";

    // For the `ndjson` format we stream rows directly into the response body
    // (T5.3) so large views don't have to be fully buffered server-side. We
    // need a probe call to surface synchronous Uncompilable errors with the
    // existing fallback semantics; once we have a stream we hand it off to a
    // background task that pumps serialized bytes into the response body.
    let probe = runner
        .run_view(tenant.context(), view_json.clone(), filters.clone())
        .await;

    let (stream, runner_label) = match probe {
        Ok(s) => (s, runner.runner_name().to_string()),
        Err(SofError::Uncompilable { reason }) if can_fallback => {
            warn!(
                runner = runner.runner_name(),
                reason = %reason,
                "in-DB runner returned Uncompilable; falling back to in-process runner"
            );
            let fallback: Arc<dyn SofRunner> = Arc::new(InProcessRunner::new(
                state.storage_arc(),
                state.config().default_fhir_version,
            ));
            let s = fallback
                .run_view(tenant.context(), view_json.clone(), filters.clone())
                .await
                .map_err(map_sof_error_to_rest)?;
            (s, format!("inprocess (fallback: {reason})"))
        }
        Err(e) => return Err(map_sof_error_to_rest(e)),
    };

    // Streaming path for ndjson: forward rows incrementally.
    if format == "ndjson" || format == "application/x-ndjson" {
        return Ok(streaming_ndjson_response(stream, &runner_label));
    }

    // Buffered paths (csv, json array, parquet) — collect the stream first.
    let (ct, body) = format_stream(stream, &format, include_header).await;
    Ok(build_response(
        StatusCode::OK,
        ct,
        body,
        &runner_label,
        &format,
    ))
}

/// Builds a chunked-transfer-encoding response that streams NDJSON rows as
/// they arrive from the runner. Each row is serialised once and pushed
/// through an mpsc channel into the response body, so the full result set
/// never has to be buffered server-side.
fn streaming_ndjson_response(
    mut stream: helios_persistence::core::sof_runner::RowStream,
    runner_label: &str,
) -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(64);

    tokio::spawn(async move {
        while let Some(row) = futures::StreamExt::next(&mut stream).await {
            let mut buf = match row {
                Ok(r) => match serde_json::to_vec(&r) {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(error = %e, "ndjson row serialization failed");
                        continue;
                    }
                },
                Err(e) => {
                    warn!(error = %e, "row error while streaming ndjson");
                    break;
                }
            };
            buf.push(b'\n');
            if tx.send(Ok(axum::body::Bytes::from(buf))).await.is_err() {
                break;
            }
        }
    });

    let body_stream = futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|chunk| (chunk, rx))
    });
    let body = axum::body::Body::from_stream(body_stream);

    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-ndjson"),
    );
    if let Ok(v) = HeaderValue::from_str(runner_label) {
        response.headers_mut().insert("x-hfs-runner", v);
    }
    response
}

/// Renders a `RowStream` to `(content_type, bytes)` for the requested format.
async fn format_stream(
    stream: helios_persistence::core::sof_runner::RowStream,
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
fn build_filters(params: &RunQueryParams, body_extra: &BodyParams) -> ViewFilters {
    let since = params.since.as_deref().and_then(|s| s.parse().ok());

    // Effective patient/group: body's repeated entries override query when present;
    // otherwise fall back to the comma-split query string.
    let patient = if !body_extra.patient.is_empty() {
        body_extra.patient.clone()
    } else {
        split_refs(params.patient.as_deref())
    };
    let group = if !body_extra.group.is_empty() {
        body_extra.group.clone()
    } else {
        split_refs(params.group.as_deref())
    };

    ViewFilters {
        patient,
        group,
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
async fn stream_to_parquet(mut stream: helios_persistence::core::sof_runner::RowStream) -> Vec<u8> {
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
async fn stream_to_ndjson(mut stream: helios_persistence::core::sof_runner::RowStream) -> Vec<u8> {
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
    mut stream: helios_persistence::core::sof_runner::RowStream,
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
    mut stream: helios_persistence::core::sof_runner::RowStream,
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
