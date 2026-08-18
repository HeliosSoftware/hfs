//! `$sql-run` operation handler.
//!
//! Implements the SQL on FHIR
//! [`$sql-run`](http://hl7.org/fhir/uv/sql-on-fhir/OperationDefinition-SQLRun.html)
//! operation: synchronous evaluation of a single subject, returning the rows in
//! the requested output format.
//!
//! The operation is invoked at the **system level** only
//! (`system=true, type=false, instance=false`) and names what it acts on
//! through a subject parameter rather than through the request path, so one
//! endpoint serves ViewDefinitions, SQLQuery Libraries and SQLView Libraries
//! alike. This handler resolves the subject (see [`super::subject`]) and then
//! dispatches: a ViewDefinition is projected here, while a Library's dependency
//! graph is materialized and its SQL executed by [`super::sqlquery`].
//!
//! ```text
//! POST /$sql-run                                    (subject in the body)
//! GET  /$sql-run?subjectCanonical=http://…&_format=ndjson
//! GET  /$sql-run?subjectReference=ViewDefinition/123
//! ```
//!
//! `GET` is available whenever every supplied parameter is primitive.
//! `subjectResource` and `resource` carry resources, so they require `POST`.
//!
//! ## Request body (POST)
//!
//! Accepts a FHIR `Parameters` resource, or a raw `ViewDefinition` JSON object
//! as shorthand for a `Parameters` body whose only entry is `subjectResource`.
//!
//! | Parameter | Type | Description |
//! |-----------|------|-------------|
//! | `subjectCanonical` | canonical | Canonical URL of the subject, optionally `\|version`-pinned |
//! | `subjectReference` | Reference | Literal location of the subject |
//! | `subjectResource` | CanonicalResource | The subject, supplied inline |
//! | `parameters` | Parameters | Values for the parameters a Library declares (Library subjects only) |
//! | `resource` | Resource | FHIR resources to transform instead of server data (ViewDefinition subjects only) |
//! | `patient` | Reference | Restrict the data feeding the view to these patients' compartments |
//! | `group` | Reference | Restrict to members of these Groups |
//! | `_format` | code | Output format: `ndjson`, `csv`, `json`, `parquet`, `fhir` (optional; defaults to `ndjson`; may also come from `Accept`) |
//! | `_limit` | integer | Maximum number of output rows |
//! | `_since` | instant | Only include resources modified after this time |
//!
//! ## Response
//!
//! - `200 OK` — stream of output rows in the requested format
//! - `400 Bad Request` — unsupported `_format`, no subject or more than one, or a parameter the subject kind does not accept
//! - `404 Not Found` — the subject could not be resolved
//! - `422 Unprocessable Entity` — the subject could not be compiled or executed
//! - `501 Not Implemented` — `source` parameter (storage-backed server)

use axum::{
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures::StreamExt;
use helios_persistence::core::search::SearchProvider;
use helios_persistence::core::sof_runner::{SofError, ViewFilters};
use helios_sof::fhir_format::{
    FHIR_JSON_MIME, accept_has_mime, accept_requires_unsupported_fhir_xml,
    format_view_fhir_parameters, wrap_in_binary_envelope,
};
use helios_sof::{
    ContentType, ExtractedRunParams, RunOptions, create_bundle_from_resources_for_version,
    extract_run_params_from_json, filter_resources_by_patient_and_group, filter_resources_by_since,
    parse_view_definition_for_version, process_view_definition, run_view_definition_with_options,
    split_csv_refs,
};
use serde::Deserialize;
use serde_json::Value;
use tracing::{debug, warn};

use super::sqlquery::{SqlQueryRunQuery, run_library_subject};
use super::subject::{SubjectKind, SubjectRef, resolve_subject};
use crate::error::RestError;
use crate::extractors::TenantExtractor;
use crate::state::AppState;

/// Query-string parameters for `$sql-run`.
///
/// `patient` and `group` accept either a single reference or a comma-separated
/// list (spec is `0..*`). Repeated entries supplied in a `Parameters` body are
/// merged in via [`merge_params`] and take precedence.
#[derive(Debug, Default, Deserialize)]
pub struct RunQueryParams {
    /// Output format: `ndjson`, `csv`, `json`, `parquet`, `fhir`. Optional
    /// (`0..1`); defaults to `ndjson`. May also be supplied via the `Accept`
    /// header, with `_format` taking precedence.
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

    /// Canonical URL of the subject, optionally `|version`-pinned. An
    /// *identity*, resolved through the canonical index.
    #[serde(rename = "subjectCanonical")]
    pub subject_canonical: Option<String>,

    /// Literal location of the subject — a relative URL on this server, or an
    /// absolute one. A *location*, read directly. Not a canonical URL.
    #[serde(rename = "subjectReference")]
    pub subject_reference: Option<String>,

    /// External data source. HFS rejects this with 501 (storage-backed; the
    /// stateless `sof-server` is the right place for source-based ETL).
    pub source: Option<String>,
}

/// `POST` (or `GET`) `[base]/$sql-run`
///
/// Evaluates a single subject and returns the rows. The subject is named by
/// `subjectCanonical`, `subjectReference` or `subjectResource` — exactly one —
/// rather than by the request path, which is why the operation is invoked at
/// the system level and serves all three artifact kinds.
///
/// `GET` is available whenever every supplied parameter is primitive, which is
/// what keeps the operation usable from a browser or a command line. It
/// therefore accepts `subjectCanonical` and `subjectReference` on the query
/// string but not `subjectResource` or `resource`, both of which carry a
/// resource and so require `POST`.
///
/// On `POST` the body is either a FHIR `Parameters` resource or, as a
/// shorthand, a bare `ViewDefinition` — the latter being equivalent to a
/// `Parameters` body whose only entry is `subjectResource`. Body parameters
/// take precedence over the corresponding query-string values.
pub async fn sql_run_handler<S>(
    State(state): State<AppState<S>>,
    Query(query_params): Query<RunQueryParams>,
    tenant: TenantExtractor,
    headers: HeaderMap,
    body: Option<axum::extract::Json<Value>>,
) -> Result<Response, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let body_value = body.map(|j| j.0);
    let body_params = body_value
        .as_ref()
        .map(extract_run_params_from_json)
        .unwrap_or_default();

    let subject_ref = build_subject_ref(&query_params, &body_params, body_value.as_ref());
    let subject = resolve_subject(&state, tenant.context(), &subject_ref, "$sql-run").await?;

    // `resource` carries inline FHIR resources to transform instead of using
    // server data, and requires a ViewDefinition subject: how inline resources
    // would reach each dependency view of a query is not specified.
    if !body_params.inline_resources.is_empty() && subject.kind != SubjectKind::ViewDefinition {
        return Err(RestError::BadRequest {
            message: "the 'resource' parameter requires a ViewDefinition subject; how inline \
                      resources reach each dependency view of a SQLQuery or SQLView is not \
                      specified"
                .to_string(),
        });
    }

    // `parameters` binds values the subject declares. A ViewDefinition declares
    // none, so supplying them for one is a 400 rather than a silent no-op.
    let has_parameters = body_value
        .as_ref()
        .map(|b| body_names_parameter(b, "parameters"))
        .unwrap_or(false);
    if has_parameters && !subject.kind.accepts_parameters() {
        return Err(RestError::BadRequest {
            message: "the 'parameters' parameter requires a SQLQuery or SQLView subject; \
                      a ViewDefinition declares no parameters"
                .to_string(),
        });
    }

    match subject.kind {
        SubjectKind::ViewDefinition => {
            let params = merge_params(query_params, &body_params);
            execute_view(
                state,
                params,
                body_params,
                tenant,
                subject.resource,
                &headers,
            )
            .await
        }
        SubjectKind::SqlQuery | SubjectKind::SqlView => {
            let library_query = SqlQueryRunQuery {
                format: query_params.format,
                header: query_params.header,
                limit: query_params.limit.map(|n| n as u32),
            };
            run_library_subject(
                state,
                tenant,
                body_value.unwrap_or(Value::Null),
                library_query,
                &headers,
                subject.resource,
            )
            .await
        }
    }
}

/// Collects the subject naming parameters from the query string and body.
///
/// A bare `ViewDefinition` body is the shorthand for `subjectResource`. Body
/// values win over the query string, matching how every other parameter is
/// merged.
fn build_subject_ref(
    query: &RunQueryParams,
    body_params: &ExtractedRunParams,
    body: Option<&Value>,
) -> SubjectRef {
    // The bare-resource shorthand: the body *is* the subject.
    if let Some(b) = body {
        if b.get("resourceType").and_then(|v| v.as_str()) == Some("ViewDefinition") {
            return SubjectRef {
                resource: Some(b.clone()),
                ..Default::default()
            };
        }
    }
    SubjectRef {
        canonical: body_params
            .subject_canonical
            .clone()
            .or_else(|| query.subject_canonical.clone()),
        reference: body_params
            .subject_reference
            .clone()
            .or_else(|| query.subject_reference.clone()),
        resource: body_params.subject_resource.clone(),
    }
}

/// Whether a `Parameters` body carries an entry with the given name. Used for
/// presence checks where the value's shape does not matter.
fn body_names_parameter(body: &Value, name: &str) -> bool {
    body.get("parameter")
        .and_then(|p| p.as_array())
        .map(|entries| {
            entries
                .iter()
                .any(|e| e.get("name").and_then(|n| n.as_str()) == Some(name))
        })
        .unwrap_or(false)
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
        subject_canonical: query.subject_canonical,
        subject_reference: query.subject_reference,
        source: body.source.clone().or(query.source),
    }
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

    // Spec Common Operation Behavior axis 2 (representation): the FHIR XML
    // envelope form is not supported → 406, never raw bytes under a FHIR
    // media type.
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    if accept_requires_unsupported_fhir_xml(accept) {
        return Err(RestError::NotAcceptable {
            message: "the application/fhir+xml representation is not supported; \
                      use application/fhir+json"
                .to_string(),
        });
    }
    let is_fhir_format = matches!(format.as_str(), "fhir" | "application/fhir+json");
    // `Accept: application/fhir+json` with an explicit flat `_format` selects
    // the serialized `Binary` envelope representation (`_format=fhir` is
    // already a FHIR resource and is never wrapped).
    let wants_envelope = !is_fhir_format && accept_has_mime(accept, FHIR_JSON_MIME);

    // Validate the format value up front so unknown values fail with 400 on
    // every path (inline + streaming), not only the inline one. The
    // resolved `ContentType` is threaded through downstream so we don't
    // re-parse the format string later (audit item #15). The `fhir` format
    // lives outside the flat-format `ContentType` enum and is rendered by
    // `format_view_fhir_parameters` instead.
    let content_type = if is_fhir_format {
        None
    } else {
        Some(
            parse_content_type(&format, include_header).ok_or_else(|| RestError::BadRequest {
                message: format!(
                    "unsupported _format value '{format}'; supported: ndjson, json, csv, parquet, fhir"
                ),
            })?,
        )
    };

    // Audit item #10: enforce the same `_limit` bound as sof-server so
    // both binaries reject the same out-of-range values consistently.
    // The spec leaves `_limit` unbounded; this is a deployment-policy
    // safety cap.
    validate_limit(params.limit)?;

    if !body_params.inline_resources.is_empty() {
        return execute_view_inline(
            &state,
            &params,
            &body_params,
            view_json,
            content_type,
            wants_envelope,
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

    // `_format=fhir`: buffer the rows and render the typed `Parameters`
    // resource, using the ViewDefinition's declared column types.
    let Some(content_type) = content_type else {
        let rows = drain_stream(stream).await?;
        let result = helios_sof::rows_to_processed_result(rows);
        let body =
            format_view_fhir_parameters(&result, &view_json).map_err(map_sof_lib_error_to_rest)?;
        return Ok(build_response(
            StatusCode::OK,
            FHIR_JSON_MIME,
            body,
            &runner_label,
            "fhir",
        ));
    };

    // Streaming path for ndjson: forward rows incrementally. An envelope
    // request forfeits streaming — the base64 `Binary` wrapper needs the
    // whole payload — so it falls through to the buffered path.
    if matches!(content_type, ContentType::NdJson) && !wants_envelope {
        return Ok(streaming_ndjson_response(stream, &runner_label));
    }

    // Buffered paths (csv, json array, parquet) — collect the stream first.
    let (ct, body) = format_stream(stream, content_type).await?;
    let (ct, body) = if wants_envelope {
        let wrapped = wrap_in_binary_envelope(ct, &body).map_err(map_sof_lib_error_to_rest)?;
        (FHIR_JSON_MIME, wrapped)
    } else {
        (ct, body)
    };
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
///
/// `content_type` is `None` for `_format=fhir` (rendered via
/// [`format_view_fhir_parameters`] rather than the flat-format pipeline);
/// `wants_envelope` wraps a flat payload in a serialized `Binary` resource.
fn execute_view_inline<S>(
    state: &AppState<S>,
    params: &RunQueryParams,
    body_params: &ExtractedRunParams,
    view_json: Value,
    content_type: Option<ContentType>,
    wants_envelope: bool,
) -> Result<Response, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let fhir_version = state.config().default_fhir_version;

    let view_definition = parse_view_definition_for_version(view_json.clone(), fhir_version)
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

    // Per SoF v2 spec: absent `patient` / `group` targets are a hard 400
    // (mapped from `SofError::ReferencedResourceNotFound` by
    // `map_sof_lib_error_to_rest`), not a "200 + Warning: 199" path.
    if !patient_refs.is_empty() || !group_refs.is_empty() {
        resources = filter_resources_by_patient_and_group(
            resources,
            &patient_refs,
            &group_refs,
            fhir_version,
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

    // `_format=fhir`: render the typed `Parameters` resource from the
    // structured rows; `_limit` is applied at the row level to match the
    // flat-format pipeline's `apply_pagination_to_result`.
    let Some(content_type) = content_type else {
        let mut processed =
            process_view_definition(view_definition, bundle).map_err(map_sof_lib_error_to_rest)?;
        if let Some(limit) = params.limit {
            processed.rows.truncate(limit);
        }
        let body = format_view_fhir_parameters(&processed, &view_json)
            .map_err(map_sof_lib_error_to_rest)?;
        return Ok(build_response(
            StatusCode::OK,
            FHIR_JSON_MIME,
            body,
            "in-process",
            "fhir",
        ));
    };

    let body = run_view_definition_with_options(view_definition, bundle, content_type, options)
        .map_err(map_sof_lib_error_to_rest)?;

    let (ct_header, response_format) = content_type_headers(content_type);
    let (ct_header, body) = if wants_envelope {
        let wrapped =
            wrap_in_binary_envelope(ct_header, &body).map_err(map_sof_lib_error_to_rest)?;
        (FHIR_JSON_MIME, wrapped)
    } else {
        (ct_header, body)
    };

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
        ContentType::Parquet => ("application/vnd.apache.parquet", "parquet"),
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
/// `application/octet-stream`/`application/parquet` → `parquet`,
/// `application/fhir+json` → `fhir`. Unknown or wildcard Accept values fall
/// through to the `ndjson` default.
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
                "application/octet-stream"
                | "application/parquet"
                | "application/vnd.apache.parquet" => Some("parquet"),
                "application/fhir+json" => Some("fhir"),
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
        "parquet"
        | "application/parquet"
        | "application/octet-stream"
        | "application/vnd.apache.parquet" => Some(ContentType::Parquet),
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
        // Per SoF v2 spec error table: a `patient` / `group` reference that
        // doesn't resolve against the supplied / queryable resources is a
        // `400 Bad Request`. (No `RestError::NotFound`-with-400 variant
        // exists, so the OperationOutcome's `code = invalid`; the
        // 400/spec-status is what matters.)
        LibErr::ReferencedResourceNotFound(msg) => RestError::BadRequest { message: msg },
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
                        // Abort the body: an unserializable row is a server
                        // fault, and silently dropping it would hand the
                        // client a clean — but lossy — 200.
                        warn!(error = %e, "ndjson row serialization failed");
                        let _ = tx
                            .send(Err(std::io::Error::other(format!(
                                "ndjson row serialization failed: {e}"
                            ))))
                            .await;
                        break;
                    }
                },
                Err(e) => {
                    // Yield an error into the body so hyper aborts the
                    // chunked transfer (no terminating chunk). Without this
                    // the client sees a cleanly-ended, silently-truncated 200.
                    warn!(error = %e, "row error while streaming ndjson");
                    let _ = tx
                        .send(Err(std::io::Error::other(format!(
                            "row error while streaming ndjson: {e}"
                        ))))
                        .await;
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
///
/// A mid-stream row error or a formatter failure propagates as a `RestError`
/// (the response status is not yet committed on the buffered path), so the
/// client gets a real error status instead of a silently truncated `200`.
async fn format_stream(
    stream: helios_persistence::core::sof_runner::RowStream,
    content_type: ContentType,
) -> Result<(&'static str, Vec<u8>), RestError> {
    let rows = drain_stream(stream).await?;
    let result = helios_sof::rows_to_processed_result(rows);
    let body =
        helios_sof::format_output(result, content_type, None).map_err(map_sof_lib_error_to_rest)?;
    Ok((content_type_headers(content_type).0, body))
}

/// Drains a [`RowStream`] into a `Vec<Value>`. A mid-stream error aborts the
/// drain and propagates as a `RestError` so the buffered output paths return a
/// proper error status rather than a silently truncated `200`.
async fn drain_stream(
    mut stream: helios_persistence::core::sof_runner::RowStream,
) -> Result<Vec<Value>, RestError> {
    let mut rows = Vec::new();
    while let Some(result) = stream.next().await {
        match result {
            Ok(row) => rows.push(row),
            Err(e) => {
                warn!(error = %e, "row error while collecting stream");
                return Err(map_sof_error_to_rest(e));
            }
        }
    }
    Ok(rows)
}

/// Builds the final `Response` with `X-HFS-Runner` and an optional
/// `Content-Disposition` attachment header for parquet. Absent
/// `patient` / `group` targets are surfaced as a 400 + OperationOutcome
/// upstream, not as `Warning: 199` headers on this response.
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
    if (format == "parquet"
        || format == "application/octet-stream"
        || format == "application/vnd.apache.parquet")
        && content_type != FHIR_JSON_MIME
    {
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

#[cfg(test)]
mod tests {
    use super::*;
    use helios_persistence::core::sof_runner::RowStream;
    use serde_json::json;

    fn row_stream(rows: Vec<Result<Value, SofError>>) -> RowStream {
        Box::pin(futures::stream::iter(rows))
    }

    #[tokio::test]
    async fn streaming_ndjson_aborts_on_row_error() {
        let stream = row_stream(vec![Ok(json!({ "a": 1 })), Err(SofError::Cancelled)]);
        let response = streaming_ndjson_response(stream, "test-runner");
        assert_eq!(response.status(), StatusCode::OK);
        // A mid-stream error must abort the chunked body, not end it cleanly:
        // collecting an aborted body fails.
        let collected = axum::body::to_bytes(response.into_body(), usize::MAX).await;
        assert!(
            collected.is_err(),
            "expected the aborted chunked body to fail collection"
        );
    }

    #[tokio::test]
    async fn streaming_ndjson_completes_on_clean_stream() {
        let stream = row_stream(vec![Ok(json!({ "a": 1 })), Ok(json!({ "a": 2 }))]);
        let response = streaming_ndjson_response(stream, "test-runner");
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("a clean stream should produce a collectable body");
        let text = String::from_utf8(bytes.to_vec()).expect("utf-8 body");
        assert_eq!(text, "{\"a\":1}\n{\"a\":2}\n");
    }

    #[tokio::test]
    async fn drain_stream_errors_on_row_error() {
        let stream = row_stream(vec![Ok(json!({ "a": 1 })), Err(SofError::Cancelled)]);
        assert!(
            drain_stream(stream).await.is_err(),
            "a mid-stream row error must propagate instead of truncating"
        );
    }

    #[tokio::test]
    async fn drain_stream_collects_clean_stream() {
        let stream = row_stream(vec![Ok(json!({ "a": 1 })), Ok(json!({ "a": 2 }))]);
        let rows = drain_stream(stream)
            .await
            .expect("clean stream should drain");
        assert_eq!(rows.len(), 2);
    }
}
