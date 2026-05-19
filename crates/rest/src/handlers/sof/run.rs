//! `$viewdefinition-run` operation handler.
//!
//! Implements the SQL-on-FHIR
//! [`$viewdefinition-run`](https://build.fhir.org/ig/FHIR/sql-on-fhir-v2/operations-viewdefinition-run.html)
//! operation. Both `POST` and `GET` are routed:
//!
//! - `POST /ViewDefinition/$viewdefinition-run` — supply the ViewDefinition inline in the body
//! - `POST /ViewDefinition/{id}/$viewdefinition-run` — run a stored ViewDefinition (body may override)
//! - `GET /ViewDefinition/$viewdefinition-run?viewReference=ViewDefinition/{id}&_format=ndjson`
//! - `GET /ViewDefinition/{id}/$viewdefinition-run?_format=ndjson`
//!
//! ## Request body (POST)
//!
//! Accepts a FHIR `Parameters` resource or a raw `ViewDefinition` JSON object.
//!
//! | Parameter | Type | Description |
//! |-----------|------|-------------|
//! | `viewResource` | Resource | The ViewDefinition to execute (Parameters form) |
//! | `patient` | string | Restrict to this patient reference |
//! | `group` | string | Restrict to this group reference |
//! | `_format` | string | Output format: `ndjson`, `csv`, `json`, `parquet` (optional; defaults to `ndjson`; may also be supplied via `Accept`) |
//! | `_limit` | integer | Maximum number of output rows |
//! | `_since` | instant | Only include resources modified after this time |
//!
//! ## Response
//!
//! - `200 OK` — stream of output rows in the requested format
//! - `400 Bad Request` — unsupported `_format` value or invalid parameters
//! - `422 Unprocessable Entity` — ViewDefinition could not be compiled or executed
//! - `501 Not Implemented` — `source` parameter (storage-backed server)

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
    parse_view_definition_for_version, run_view_definition_with_options, split_csv_refs,
};
use serde::Deserialize;
use serde_json::Value;
use tracing::{debug, warn};

use super::references::resolve_resource_canonical_or_relative;
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
    /// Output format: `ndjson`, `csv`, `json`, `parquet`. Optional per SoF
    /// v2 PR #353 (`0..1`); defaults to `ndjson`. May also be supplied via
    /// the `Accept` header (with `_format` taking precedence).
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

    /// Reference to a stored ViewDefinition. Only meaningful on GET requests
    /// (POST callers supply `viewResource`/`viewReference` in the body).
    #[serde(rename = "viewReference")]
    pub view_reference: Option<String>,

    /// External data source. HFS rejects this with 501 (storage-backed; the
    /// stateless `sof-server` is the right place for source-based ETL).
    pub source: Option<String>,
}

/// `POST` (or `GET`) `/ViewDefinition/$viewdefinition-run`
///
/// On `POST`, the ViewDefinition must be supplied in the request body either as:
/// - A raw `ViewDefinition` JSON object, or
/// - A FHIR `Parameters` resource with a `viewResource` parameter.
///
/// On `GET`, no body is permitted (per spec: `viewResource` and `resource` are
/// POST-only). The ViewDefinition must come from the `viewReference` query
/// parameter.
///
/// When the body is a `Parameters` resource, additional parameter entries
/// (`_format`, `_limit`, `_since`, `patient`, `group`, `header`) override
/// the corresponding query-string values per the SQL-on-FHIR spec.
pub async fn run_view_definition_handler<S>(
    State(state): State<AppState<S>>,
    Query(query_params): Query<RunQueryParams>,
    tenant: TenantExtractor,
    headers: HeaderMap,
    body: Option<axum::extract::Json<Value>>,
) -> Result<impl IntoResponse, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let body_value = body.map(|j| j.0);
    let body_params = body_value
        .as_ref()
        .map(extract_run_params_from_json)
        .unwrap_or_default();
    let view_json = match body_value.as_ref() {
        Some(b) => resolve_view_from_body(&state, &tenant, b).await?,
        None => match query_params.view_reference.as_deref() {
            Some(reference) => resolve_view_reference(&state, &tenant, reference).await?,
            None => {
                return Err(RestError::BadRequest {
                    message: "GET $viewdefinition-run requires a 'viewReference' query parameter; \
                              use POST to supply 'viewResource' or 'resource' in the body"
                        .to_string(),
                });
            }
        },
    };
    let params = merge_params(query_params, &body_params);
    execute_view(state, params, body_params, tenant, view_json, &headers).await
}

/// `POST` (or `GET`) `/ViewDefinition/{id}/$viewdefinition-run`
///
/// Looks up the stored ViewDefinition by ID and runs it. On POST, if the body
/// contains a `viewResource` (or is itself a `ViewDefinition` resource), the
/// body overrides the stored definition. GET infers the ViewDefinition from
/// the path id and ignores any body.
pub async fn run_stored_view_definition_handler<S>(
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
    Query(query_params): Query<RunQueryParams>,
    tenant: TenantExtractor,
    headers: HeaderMap,
    body: Option<axum::extract::Json<Value>>,
) -> Result<impl IntoResponse, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let body_value = body.map(|j| j.0);
    let body_params = body_value
        .as_ref()
        .map(extract_run_params_from_json)
        .unwrap_or_default();
    // Spec: at instance level the server infers `viewReference` from the URL
    // path. A body that supplies a different `viewResource`/`viewReference`
    // would silently change which view runs — reject that with 400 + invalid.
    // A body that supplies the *same* view as the path is allowed (no-op).
    if let Some(b) = body_value.as_ref() {
        if body_has_view_definition(b) {
            ensure_instance_body_matches_path(b, &id, &body_params)?;
        }
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
    let view_json = stored.content().clone();
    let params = merge_params(query_params, &body_params);
    execute_view(state, params, body_params, tenant, view_json, &headers).await
}

/// Verifies that a body-supplied `viewResource`/`viewReference` on an
/// instance-level URL refers to the same ViewDefinition as the path id.
/// Returns 400 + `invalid` when it doesn't.
fn ensure_instance_body_matches_path(
    body: &Value,
    path_id: &str,
    body_params: &ExtractedRunParams,
) -> Result<(), RestError> {
    // Bare ViewDefinition body: its `id` (if present) must match the path.
    if body.get("resourceType").and_then(|v| v.as_str()) == Some("ViewDefinition") {
        let body_id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if body_id.is_empty() || body_id == path_id {
            return Ok(());
        }
        return Err(RestError::BadRequest {
            message: format!(
                "instance-level URL is bound to ViewDefinition/{path_id}; \
                 body must not supply a different ViewDefinition (got id='{body_id}'). \
                 POST to /ViewDefinition/$viewdefinition-run for ad-hoc runs."
            ),
        });
    }

    // Parameters body: inline viewResource or viewReference must agree.
    if let Some(view) = &body_params.view_resource {
        let body_id = view.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if !body_id.is_empty() && body_id != path_id {
            return Err(RestError::BadRequest {
                message: format!(
                    "instance-level URL is bound to ViewDefinition/{path_id}; \
                     body viewResource has a different id='{body_id}'. \
                     POST to /ViewDefinition/$viewdefinition-run for ad-hoc runs."
                ),
            });
        }
    }
    if let Some(reference) = &body_params.view_reference {
        let trimmed = reference.trim();
        let expected_relative = format!("ViewDefinition/{path_id}");
        // Accept the relative form, or any canonical/absolute URL that ends
        // with `/ViewDefinition/{path_id}` (with optional `|version` /
        // `@version` suffix).
        let matches_relative = trimmed == expected_relative;
        let matches_canonical = {
            let without_suffix = trimmed
                .split_once('|')
                .map(|(u, _)| u)
                .unwrap_or_else(|| trimmed.rsplit_once('@').map(|(u, _)| u).unwrap_or(trimmed));
            without_suffix.ends_with(&format!("/{expected_relative}"))
        };
        if !matches_relative && !matches_canonical {
            return Err(RestError::BadRequest {
                message: format!(
                    "instance-level URL is bound to ViewDefinition/{path_id}; \
                     body viewReference '{reference}' refers to a different ViewDefinition. \
                     POST to /ViewDefinition/$viewdefinition-run for ad-hoc runs."
                ),
            });
        }
    }

    Ok(())
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
        view_reference: query.view_reference,
        source: body.source.clone().or(query.source),
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
/// Supports all three spec-listed forms via the shared
/// [`resolve_resource_canonical_or_relative`] helper:
/// - Relative: `ViewDefinition/{id}`
/// - Canonical URL with `|version` (FHIR convention) or `@version` (spec
///   narrative form)
/// - Absolute URL
///
/// Advertised by `/$sql-on-fhir-capabilities` as
/// `supportsRelativeReference`, `supportsCanonicalReference`, and
/// `supportsAbsoluteReference`.
async fn resolve_view_reference<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    reference: &str,
) -> Result<Value, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    resolve_resource_canonical_or_relative(state, tenant.context(), "ViewDefinition", reference)
        .await
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
    headers: &HeaderMap,
) -> Result<Response, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    // Per spec: `source` is an alternate data origin for stateless ETL. HFS
    // is storage-backed; the stateless `sof-server` is the right home for
    // this. Return 400 + `not-supported` so the OperationOutcome matches the
    // spec's error-code examples for refused parameters.
    if body_params.source.is_some() || params.source.is_some() {
        return Err(RestError::NotSupported {
            feature: "the 'source' parameter is not supported by this storage-backed server; \
                      use the stateless 'sof-server' for external-data-source runs"
                .to_string(),
        });
    }

    // Resolve `_format`: SoF v2 PR #353 makes this `0..1`. Precedence:
    // `_format` (query or body, already merged) > `Accept` header > `ndjson`.
    let format = resolve_format(params.format.as_deref(), headers);
    let include_header = params
        .header
        .as_deref()
        .map(|h| h == "true" || h == "1")
        .unwrap_or(true);

    // Validate the format value up front so unknown values fail with 400 on
    // every path (inline + streaming), not only the inline one. The
    // resolved `ContentType` is threaded through downstream so we don't
    // re-parse the format string later (audit item #15).
    let content_type =
        parse_content_type(&format, include_header).ok_or_else(|| RestError::BadRequest {
            message: format!(
                "unsupported _format value '{format}'; supported: ndjson, json, csv, parquet"
            ),
        })?;

    // Audit item #10: enforce the same `_limit` bound as sof-server so
    // both binaries reject the same out-of-range values consistently.
    // The spec leaves `_limit` unbounded; this is a deployment-policy
    // safety cap.
    validate_limit(params.limit)?;

    if !body_params.inline_resources.is_empty() {
        return execute_view_inline(&state, &params, &body_params, view_json, content_type);
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
    if matches!(content_type, ContentType::NdJson) {
        return Ok(streaming_ndjson_response(stream, &runner_label));
    }

    // Buffered paths (csv, json array, parquet) — collect the stream first.
    let (ct, body) = format_stream(stream, content_type).await;
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
    content_type: ContentType,
) -> Result<Response, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let fhir_version = state.config().default_fhir_version;

    let view_definition = parse_view_definition_for_version(view_json, fhir_version)
        .map_err(map_sof_lib_error_to_rest)?;

    let mut resources = body_params.inline_resources.clone();

    // Patient/group filtering: prefer the multi-valued body entries; fall
    // back to comma-split query values. Spec is `patient` 0..1, `group`
    // 0..* — pass all references through so the shared filter can union
    // multiple group memberships once that path is implemented (today the
    // filter still errors when group_refs is non-empty).
    let patient_refs = if !body_params.patient.is_empty() {
        body_params.patient.clone()
    } else {
        split_csv_refs(params.patient.as_deref())
    };
    let group_refs = if !body_params.group.is_empty() {
        body_params.group.clone()
    } else {
        split_csv_refs(params.group.as_deref())
    };

    // Track absent-target warnings (audit item #5) to surface as `Warning:`
    // HTTP headers on the response.
    let mut filter_warnings: Vec<String> = Vec::new();
    if !patient_refs.is_empty() || !group_refs.is_empty() {
        let outcome = filter_resources_by_patient_and_group(
            resources,
            &patient_refs,
            &group_refs,
            fhir_version,
        )
        .map_err(map_sof_lib_error_to_rest)?;
        resources = outcome.resources;
        filter_warnings.extend(outcome.warnings);
    }

    let since = params.since.as_deref().and_then(|s| s.parse().ok());
    if let Some(since) = since {
        resources =
            filter_resources_by_since(resources, since).map_err(map_sof_lib_error_to_rest)?;
    }

    let bundle = create_bundle_from_resources_for_version(resources, fhir_version)
        .map_err(map_sof_lib_error_to_rest)?;

    let options = RunOptions {
        since,
        limit: params.limit,
        page: None,
        parquet_options: None,
    };

    debug!(
        runner = "in-process",
        content_type = ?content_type,
        "dispatching $viewdefinition-run (inline)"
    );

    let body = run_view_definition_with_options(view_definition, bundle, content_type, options)
        .map_err(map_sof_lib_error_to_rest)?;

    let (ct_header, response_format) = content_type_headers(content_type);

    Ok(build_response_with_warnings(
        StatusCode::OK,
        ct_header,
        body,
        "in-process",
        response_format,
        &filter_warnings,
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

/// Audit item #10: enforces the `1..=10000` `_limit` cap (matches
/// sof-server). The spec leaves `_limit` unbounded; both binaries adopt
/// the same deployment-policy safety cap so a client gets the same
/// behavior regardless of which server is in front.
fn validate_limit(limit: Option<usize>) -> Result<(), RestError> {
    if let Some(n) = limit {
        if n == 0 {
            return Err(RestError::BadRequest {
                message: "_limit parameter must be greater than 0".to_string(),
            });
        }
        if n > 10000 {
            return Err(RestError::BadRequest {
                message: "_limit parameter cannot exceed 10000".to_string(),
            });
        }
    }
    Ok(())
}

/// Resolves the output format for a run. Spec precedence (SoF v2 PR #353):
/// `_format` parameter (already merged from query and body upstream) >
/// `Accept` header > `ndjson` default. `_format` is `0..1` in the operation
/// definition; absence is not an error.
///
/// Accept-header values map: `application/json` → `json`,
/// `application/x-ndjson`/`application/ndjson` → `ndjson`, `text/csv` → `csv`,
/// `application/octet-stream`/`application/parquet` → `parquet`. Unknown or
/// wildcard Accept values fall through to the `ndjson` default.
fn resolve_format(format_param: Option<&str>, headers: &HeaderMap) -> String {
    if let Some(f) = format_param {
        return f.to_lowercase();
    }
    if let Some(accept) = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_lowercase)
    {
        let mapped = accept
            .split(',')
            .map(|s| s.split(';').next().unwrap_or("").trim())
            .find_map(|mime| match mime {
                "application/json" => Some("json"),
                "application/x-ndjson" | "application/ndjson" => Some("ndjson"),
                "text/csv" => Some("csv"),
                "application/octet-stream" | "application/parquet" => Some("parquet"),
                _ => None,
            });
        if let Some(f) = mapped {
            return f.to_string();
        }
    }
    "ndjson".to_string()
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
/// `sof-server` / `pysof` byte-for-byte. Takes the already-validated
/// `ContentType` so there's no re-parse-with-`expect` here (audit item #15).
async fn format_stream(
    stream: helios_persistence::core::sof_runner::RowStream,
    content_type: ContentType,
) -> (&'static str, Vec<u8>) {
    let rows = drain_stream(stream).await;
    let result = helios_sof::rows_to_processed_result(rows);
    let body = helios_sof::format_output(result, content_type, None).unwrap_or_else(|e| {
        warn!(error = %e, ?content_type, "shared output formatter failed; returning empty body");
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
    build_response_with_warnings(status, content_type, body, runner_label, format, &[])
}

/// Like [`build_response`] but appends one `Warning:` header per
/// absent-target message (RFC 7234 §5.5, warn-code 199). Audit item #5
/// — surfaces `patient` / `group` absence to clients regardless of body
/// format.
fn build_response_with_warnings(
    status: StatusCode,
    content_type: &'static str,
    body: Vec<u8>,
    runner_label: &str,
    format: &str,
    warnings: &[String],
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
    for msg in warnings {
        let safe = msg.replace('"', "'");
        if let Ok(v) = HeaderValue::from_str(&format!("199 - \"{}\"", safe)) {
            headers.append("warning", v);
        }
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
        split_csv_refs(params.patient.as_deref())
    };
    let group = if !body_extra.group.is_empty() {
        body_extra.group.clone()
    } else {
        split_csv_refs(params.group.as_deref())
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
