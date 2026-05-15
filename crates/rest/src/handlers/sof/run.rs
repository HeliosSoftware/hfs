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
use helios_persistence::core::sof_runner::{SofError, ViewFilters};
use helios_sof::{
    ContentType, ExtractedRunParams, RunOptions, body_has_view_definition,
    create_bundle_from_resources_for_version, extract_run_params_from_json,
    filter_resources_by_patient_and_group, filter_resources_by_since,
    parse_view_definition_for_version, run_view_definition_with_options,
};
use serde::Deserialize;
use serde_json::Value;
use tracing::{debug, warn};

use crate::error::RestError;
use crate::extractors::TenantExtractor;
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
    let body_params = extract_run_params_from_json(&body.0);
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
    let body_params = extract_run_params_from_json(&body.0);
    // If the body provides a ViewDefinition (inline or by reference), prefer
    // it. Otherwise, load the stored ViewDefinition by id from the path.
    let view_json = if body_has_view_definition(&body.0) {
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

/// Merges body parameters onto query-string parameters with body precedence
/// for scalar values. Multi-valued fields (`patient`, `group`) and inline
/// resources stay on the [`ExtractedRunParams`] and are consumed in
/// [`build_filters`] / [`execute_view`].
///
/// `header` is normalised back to `Option<String>` so it matches the axum
/// query-string shape — `execute_view` lowers it to bool at the use site.
fn merge_params(query: RunQueryParams, body: &ExtractedRunParams) -> RunQueryParams {
    RunQueryParams {
        format: body.format.clone().or(query.format),
        header: body
            .header
            .map(|b| {
                if b {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            })
            .or(query.header),
        limit: body.limit.map(|n| n as usize).or(query.limit),
        since: body.since.clone().or(query.since),
        patient: query.patient,
        group: query.group,
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
        let extracted = extract_run_params_from_json(body);

        // 1. Inline viewResource takes precedence when both are present.
        if let Some(view) = extracted.view_resource {
            return Ok(view);
        }

        // 2. Otherwise, resolve viewReference.
        if let Some(reference) = extracted.view_reference {
            return resolve_view_reference(state, tenant, &reference).await;
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
/// Inline `resource:` parameters are evaluated through the in-process
/// `helios-sof` FHIRPath pipeline (the same code path `sof-server` uses),
/// so this handler does not require any storage backend when the caller
/// supplies resources inline. Persistent requests are dispatched to the
/// backend's in-DB SOF runner.
async fn execute_view<S>(
    state: AppState<S>,
    params: RunQueryParams,
    body_params: ExtractedRunParams,
    tenant: TenantExtractor,
    view_json: Value,
) -> Result<Response, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let format = params.format.as_deref().unwrap_or("ndjson").to_lowercase();
    let include_header = params
        .header
        .as_deref()
        .map(|h| h == "true" || h == "1")
        .unwrap_or(true);

    if !body_params.inline_resources.is_empty() {
        return execute_view_inline(
            &state,
            &params,
            &body_params,
            view_json,
            &format,
            include_header,
        );
    }

    let runner = state
        .sof_runner()
        .ok_or_else(|| RestError::NotImplemented {
            feature: "$viewdefinition-run is not available: the configured storage backend \
                      does not provide an in-DB SOF runner"
                .to_string(),
        })?
        .clone();
    let effective_tenant = tenant.context().clone();
    let filters = build_filters(&params, &body_params);

    debug!(
        runner = runner.runner_name(),
        tenant = %effective_tenant.tenant_id(),
        format = %format,
        "dispatching $viewdefinition-run"
    );

    // Probe the runner — surfaces synchronous Uncompilable errors as 422
    // before we start streaming bytes to the client.
    let stream = runner
        .run_view(&effective_tenant, view_json.clone(), filters.clone())
        .await
        .map_err(map_sof_error_to_rest)?;
    let runner_label = runner.runner_name().to_string();

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

/// Runs the view against inline `resource:` parameters using the in-process
/// `helios-sof` FHIRPath evaluator. Returns fully buffered output bytes —
/// inline runs do not stream because the evaluator materialises the entire
/// result set before formatting.
fn execute_view_inline<S>(
    state: &AppState<S>,
    params: &RunQueryParams,
    body_params: &ExtractedRunParams,
    view_json: Value,
    format: &str,
    include_header: bool,
) -> Result<Response, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let fhir_version = state.config().default_fhir_version;

    let view_definition = parse_view_definition_for_version(view_json, fhir_version)
        .map_err(map_sof_lib_error_to_rest)?;

    let mut resources = body_params.inline_resources.clone();

    // Patient/group filtering: prefer the multi-valued body entries; fall
    // back to a single comma-split query value. The in-process evaluator
    // takes a single reference, so we only apply the first one for now.
    let patient_ref = body_params
        .patient
        .first()
        .cloned()
        .or_else(|| split_refs(params.patient.as_deref()).into_iter().next());
    let group_ref = body_params
        .group
        .first()
        .cloned()
        .or_else(|| split_refs(params.group.as_deref()).into_iter().next());

    if patient_ref.is_some() || group_ref.is_some() {
        resources = filter_resources_by_patient_and_group(
            resources,
            patient_ref.as_deref(),
            group_ref.as_deref(),
        )
        .map_err(map_sof_lib_error_to_rest)?;
    }

    let since = params.since.as_deref().and_then(|s| s.parse().ok());
    if let Some(since) = since {
        resources =
            filter_resources_by_since(resources, since).map_err(map_sof_lib_error_to_rest)?;
    }

    let bundle = create_bundle_from_resources_for_version(resources, fhir_version)
        .map_err(map_sof_lib_error_to_rest)?;

    let content_type =
        parse_content_type(format, include_header).ok_or_else(|| RestError::BadRequest {
            message: format!("Unsupported _format value: {format}"),
        })?;

    let options = RunOptions {
        since,
        limit: params.limit,
        page: None,
        parquet_options: None,
    };

    debug!(
        runner = "in-process",
        format = %format,
        "dispatching $viewdefinition-run (inline)"
    );

    let body = run_view_definition_with_options(view_definition, bundle, content_type, options)
        .map_err(map_sof_lib_error_to_rest)?;

    let (ct_header, response_format) = content_type_headers(content_type);

    Ok(build_response(
        StatusCode::OK,
        ct_header,
        body,
        "in-process",
        response_format,
    ))
}

/// Maps a [`ContentType`] to its (HTTP `Content-Type` header, `_format`-label)
/// pair. Shared between the inline and streaming response paths so both emit
/// the same content-type strings.
fn content_type_headers(ct: ContentType) -> (&'static str, &'static str) {
    match ct {
        ContentType::Csv | ContentType::CsvWithHeader => ("text/csv; charset=utf-8", "csv"),
        ContentType::Json => ("application/json", "json"),
        ContentType::NdJson => ("application/x-ndjson", "ndjson"),
        ContentType::Parquet => ("application/octet-stream", "parquet"),
    }
}

/// Maps a `_format` string + header flag to a `ContentType` understood by the
/// in-process evaluator. Returns `None` when the format is not recognised.
fn parse_content_type(format: &str, include_header: bool) -> Option<ContentType> {
    match format {
        "ndjson" | "application/x-ndjson" | "application/ndjson" => Some(ContentType::NdJson),
        "json" | "application/json" => Some(ContentType::Json),
        "csv" | "text/csv" => Some(if include_header {
            ContentType::CsvWithHeader
        } else {
            ContentType::Csv
        }),
        "parquet" | "application/parquet" | "application/octet-stream" => {
            Some(ContentType::Parquet)
        }
        _ => None,
    }
}

/// Maps a `helios_sof::SofError` to a `RestError`. Distinct from
/// [`map_sof_error_to_rest`] which handles the `helios_persistence` `SofError`
/// variants emitted by storage-backed runners.
fn map_sof_lib_error_to_rest(e: helios_sof::SofError) -> RestError {
    use helios_sof::SofError as LibErr;
    match e {
        LibErr::InvalidViewDefinition(msg) | LibErr::FhirPathError(msg) => {
            RestError::UnprocessableEntity { message: msg }
        }
        LibErr::UnsupportedContentType(msg) => RestError::BadRequest { message: msg },
        other => {
            warn!(error = %other, "in-process SOF evaluator error");
            RestError::InternalError {
                message: other.to_string(),
            }
        }
    }
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

/// Renders a `RowStream` to `(content_type_header, bytes)` for the requested
/// format. NDJSON has its own dedicated streaming path
/// ([`streaming_ndjson_response`]); buffered formats (csv, json, parquet) drain
/// here and pass through `helios_sof::format_output` so REST output matches
/// `sof-server` / `pysof` byte-for-byte. Unknown formats fall back to NDJSON.
async fn format_stream(
    stream: helios_persistence::core::sof_runner::RowStream,
    format: &str,
    include_header: bool,
) -> (&'static str, Vec<u8>) {
    let rows = drain_stream(stream).await;
    let content_type = parse_content_type(format, include_header).unwrap_or(ContentType::NdJson);
    let result = helios_sof::rows_to_processed_result(rows);
    let body = helios_sof::format_output(result, content_type, None).unwrap_or_else(|e| {
        warn!(error = %e, format, "shared output formatter failed; returning empty body");
        Vec::new()
    });
    (content_type_headers(content_type).0, body)
}

/// Drains a [`RowStream`] into a `Vec<Value>`, stopping at the first stream
/// error after logging it. Used by the buffered output paths.
async fn drain_stream(mut stream: helios_persistence::core::sof_runner::RowStream) -> Vec<Value> {
    let mut rows = Vec::new();
    while let Some(result) = stream.next().await {
        match result {
            Ok(row) => rows.push(row),
            Err(e) => {
                warn!(error = %e, "row error while collecting stream");
                break;
            }
        }
    }
    rows
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

/// Builds `ViewFilters` from query parameters.
fn build_filters(params: &RunQueryParams, body_extra: &ExtractedRunParams) -> ViewFilters {
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
