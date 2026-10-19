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
//! | `patient` | Reference | Restrict the data feeding the view to these patients' compartments; for a Library (SQLQuery/SQLView) subject this also narrows every dependency view |
//! | `group` | Reference | Restrict to members of these Groups; also narrows every dependency view of a Library subject, and a Group with no Patient members selects nothing |
//! | `_format` | code | Output format: `ndjson`, `csv`, `json`, `parquet`, `arrow`, `fhir` (optional; defaults to `ndjson`; may also come from `Accept`) |
//! | `_limit` | integer | Maximum number of output rows (for Library subjects, caps only the final rows) |
//! | `_since` | instant | Only include resources modified at or after this time; also narrows every dependency view of a Library subject |
//!
//! ## Response
//!
//! - `200 OK` — stream of output rows in the requested format
//! - `X-HFS-Runner` — the runner that served a ViewDefinition request (`sqlite-indb`,
//!   `postgres-indb`, `in-process`). Only sent when `HFS_SOF_RUNNER_HEADER` is on.
//! - `400 Bad Request` — unsupported `_format`, no subject or more than one, an unparsable `_since`, or a parameter the subject kind does not accept
//! - `404 Not Found` — the subject could not be resolved
//! - `422 Unprocessable Entity` — the subject could not be compiled or executed. An
//!   inline `subjectResource` ViewDefinition is linted structurally first (#821):
//!   any structural error responds with an `OperationOutcome` carrying **one
//!   `issue` per diagnostic** (`severity: "error"`; `code` one of `structure`,
//!   `required`, `invalid`; `details.coding[0]` the diagnostic's own code under
//!   `http://heliossoftware.com/fhir/CodeSystem/view-definition-lint`;
//!   `expression[0]` a FHIRPath-style path to the offending node) — the same
//!   shape `sof-server`'s `$sql-run` returns. A subject the lint accepts can
//!   still fail later (compilation or evaluation) with a single-issue `422`.
//! - `501 Not Implemented` — `source` parameter (storage-backed server)

use axum::{
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures::StreamExt;
use helios_persistence::core::search::SearchProvider;
use helios_persistence::core::sof_runner::{SofError, ViewFilters};
use helios_sof::columnar_stream::{ArrowIpcStreamWriter, ParquetStreamWriter, STREAM_BATCH_ROWS};
use helios_sof::fhir_format::{
    FHIR_JSON_MIME, accept_has_mime, accept_requires_unsupported_fhir_xml,
    format_view_fhir_parameters, wrap_in_binary_envelope,
};
use helios_sof::parquet_schema::{SCHEMA_SAMPLE_ROWS, resolve_arrow_schema};
use helios_sof::sqlquery::engine::{DeclaredColumn, try_declared_columns};
use helios_sof::{
    ContentType, ExtractedRunParams, ParquetOptions, ProcessedRow, RunOptions,
    create_bundle_from_resources_for_version, extract_run_params_checked,
    filter_resources_by_patient_and_group, filter_resources_by_since,
    lint::{DiagnosticCode, Severity, lint_operation_outcome, lint_view_definition},
    parse_view_definition_for_version, process_view_definition, run_view_definition_with_options,
    split_csv_refs,
};
use serde::Deserialize;
use serde_json::Value;
use tracing::{debug, warn};

use super::sqlquery::{SqlQueryRunQuery, run_library_subject};
use super::subject::{
    RENAMED_VIEW_PARAM_MESSAGE, RENAMED_VIEW_PARAM_NAME, SubjectKind, SubjectRef, resolve_subject,
};
use crate::error::RestError;
use crate::extractors::TenantExtractor;
use crate::handlers::bulk_common::parse_instant_param;
use crate::state::AppState;

/// Query-string parameters for `$sql-run`.
///
/// `patient` and `group` accept either a single reference or a comma-separated
/// list (spec is `0..*`). Repeated entries supplied in a `Parameters` body are
/// merged in via [`merge_params`] and take precedence.
#[derive(Debug, Default, Deserialize)]
pub struct RunQueryParams {
    /// Output format: `ndjson`, `csv`, `json`, `parquet`, `arrow`, `fhir`. Optional
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

    // Unknown parameter names are deliberately ignored: this handler does
    // not introduce general strict validation. `view` is the pre-ballot
    // spelling of `context`, so a request naming it gets the same didactic
    // 400 that `$sql-export` gives, instead of silently falling through to a
    // generic "requires a subject" error.
    if let Some(b) = body_value.as_ref() {
        if body_names_parameter(b, RENAMED_VIEW_PARAM_NAME) {
            return Err(RestError::BadRequest {
                message: RENAMED_VIEW_PARAM_MESSAGE.to_string(),
            });
        }
    }

    // Refuse unusable patient/group entries before resolving any subject or
    // applying query fallbacks: dropping an entry would widen the run.
    let body_params = body_value
        .as_ref()
        .map(extract_run_params_checked)
        .transpose()
        .map_err(|error| RestError::InvalidParameter {
            param: error.param,
            message: error.message,
        })?
        .unwrap_or_default();

    let subject_ref = build_subject_ref(&query_params, &body_params, body_value.as_ref());
    let subject = resolve_subject(&state, tenant.context(), &subject_ref, "$sql-run").await?;

    // #821: an inline `subjectResource` ViewDefinition is linted structurally
    // before it reaches either execution path below — the storage-backed
    // runner's SQL compiler (`execute_view` -> `runner.run_view`) reports at
    // most one `processing` issue with no position, and the in-process path
    // (`execute_view_inline`) parses the JSON through a typed struct that
    // silently drops keys it doesn't recognize either way. This mirrors
    // sof-server's own `$sql-run` (`helios_sof::handlers::sql_run_handler`):
    // same lint, same `OperationOutcome` shape. A subject resolved from
    // storage via `subjectCanonical`/`subjectReference` is not re-linted here
    // — only the inline shorthand is in scope.
    if subject.kind == SubjectKind::ViewDefinition && subject_ref.resource.is_some() {
        lint_inline_view_definition(&subject.resource)?;
    }

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
            message: "the 'parameters' parameter requires a SQLQuery subject; a ViewDefinition \
                      declares no parameters, and the SQLView profile constrains \
                      Library.parameter to 0..0"
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
                format: query_params.format.clone(),
                header: query_params.header.clone(),
                limit: query_params.limit.map(|n| n as u32),
            };
            // `patient`, `group` and `_since` narrow every dependency view
            // (#1701); `_limit` caps the subject's final rows, never a
            // dependency, so it stays out of the filters.
            let filters = ViewFilters {
                limit: None,
                ..build_filters(&merge_params(query_params, &body_params), &body_params)?
            };
            run_library_subject(
                state,
                tenant,
                body_value.unwrap_or(Value::Null),
                library_query,
                &headers,
                subject.resource,
                filters,
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
                    "unsupported _format value '{format}'; supported: ndjson, json, csv, parquet, arrow, fhir"
                ),
            })?,
        )
    };

    // Audit item #10: enforce the same `_limit` bound as sof-server so
    // both binaries reject the same out-of-range values consistently.
    // The spec leaves `_limit` unbounded; this is a deployment-policy
    // safety cap.
    validate_limit(params.limit)?;

    // Bound the effective `patient` plus `group` values (same precedence as
    // `build_filters`) before they reach the runner or the inline path.
    let (patient, group) = effective_patient_group(&params, &body_params);
    super::input_limits::check_patient_group_values(patient.len() + group.len())?;

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
    let filters = build_filters(&params, &body_params)?;

    debug!(
        runner = runner.runner_name(),
        tenant = %effective_tenant.tenant_id(),
        format = %format,
        "dispatching $viewdefinition-run"
    );

    let runner_label = state
        .config()
        .sof_runner_header
        .then(|| runner.runner_name());

    let parquet_options = parquet_options_from_params(&body_params)?;

    // Parquet and Arrow IPC: let the runner append database values straight
    // to Arrow builders. A runner without a columnar path answers `None` and
    // the row path below serves the request.
    if let Some(columnar_type @ (ContentType::Parquet | ContentType::ArrowIpc)) = content_type
        && let Some(batches) =
            open_columnar(runner.as_ref(), &effective_tenant, &view_json, &filters).await?
    {
        let options = parquet_options.unwrap_or_default();
        // The envelope needs the whole payload, as on the row path.
        if wants_envelope {
            let body = encode_batches(batches, columnar_type, &options).await?;
            return buffered_response(
                content_type_headers(columnar_type).0,
                body,
                wants_envelope,
                runner_label,
                &format,
            );
        }
        return Ok(streaming_batch_response(
            batches,
            columnar_type,
            options,
            runner_label,
            &format,
        ));
    }

    // Probe the runner — surfaces synchronous Uncompilable errors as 422
    // before we start streaming bytes to the client.
    let stream = runner
        .run_view(&effective_tenant, view_json.clone(), filters.clone())
        .await
        .map_err(map_sof_error_to_rest)?;

    // `_format=fhir`: stream the typed `Parameters` resource row by row, using
    // the ViewDefinition's declared column types.
    let Some(content_type) = content_type else {
        let encoder =
            helios_sof::RowStreamEncoder::fhir(&view_json).map_err(map_sof_lib_error_to_rest)?;
        return streaming_rows_response(stream, encoder, &view_json, FHIR_JSON_MIME, runner_label)
            .await;
    };

    // Streaming path for ndjson: forward rows incrementally. An envelope
    // request forfeits streaming — the base64 `Binary` wrapper needs the
    // whole payload — so it falls through to the buffered path.
    if matches!(content_type, ContentType::NdJson) && !wants_envelope {
        return Ok(streaming_ndjson_response(stream, runner_label));
    }

    // Streaming path for parquet and arrow: the schema is fixed from the
    // ViewDefinition's declared types (plus a bounded row prefix) and rows are
    // encoded as they arrive. The envelope forfeits streaming for the same
    // reason as above.
    if matches!(content_type, ContentType::Parquet | ContentType::ArrowIpc) && !wants_envelope {
        return streaming_columnar_response(
            stream,
            content_type,
            &view_json,
            parquet_options,
            runner_label,
            &format,
        )
        .await;
    }

    // Streaming path for csv and the json array: rows are encoded one at a
    // time into the same bytes the buffered formatters produce. The envelope
    // forfeits streaming for the same reason as above.
    if matches!(
        content_type,
        ContentType::Csv | ContentType::CsvWithHeader | ContentType::Json
    ) && !wants_envelope
    {
        let encoder = helios_sof::RowStreamEncoder::new(content_type).ok_or_else(|| {
            RestError::InternalError {
                message: format!("{content_type:?} cannot be streamed row by row"),
            }
        })?;
        return streaming_rows_response(
            stream,
            encoder,
            &view_json,
            content_type_headers(content_type).0,
            runner_label,
        )
        .await;
    }

    // Buffered paths (enveloped csv/json/parquet/arrow) — collect the stream first.
    let (ct, body) =
        format_stream(stream, content_type, &view_json, parquet_options.as_ref()).await?;
    buffered_response(ct, body, wants_envelope, runner_label, &format)
}

/// Wraps a fully buffered body in the optional base64 `Binary` envelope and
/// builds the `200` response.
fn buffered_response(
    ct: &'static str,
    body: Vec<u8>,
    wants_envelope: bool,
    runner_label: Option<&str>,
    format: &str,
) -> Result<Response, RestError> {
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
        runner_label,
        format,
    ))
}

/// Record batches from a runner's columnar path, with the first one already
/// read so a failure before any output still gets a real error status.
struct ColumnarBatches {
    schema: std::sync::Arc<helios_sof::arrow::datatypes::Schema>,
    first: Option<helios_sof::arrow::record_batch::RecordBatch>,
    rest: helios_persistence::core::sof_runner::BatchStream,
}

/// Opens the runner's columnar path for Parquet / Arrow IPC output.
///
/// Returns `Ok(None)` when the runner has no columnar path for this view, so
/// the caller falls back to the row path. `Uncompilable` and the like, and an
/// error producing the first batch, surface here before any bytes are sent.
/// A stream that ends without a batch means the view returned no rows; the
/// schema is then the one the row path uses for an empty result (the declared
/// columns), without running the query again.
async fn open_columnar(
    runner: &dyn helios_persistence::core::sof_runner::SofRunner,
    tenant: &helios_persistence::tenant::TenantContext,
    view_json: &Value,
    filters: &helios_persistence::core::sof_runner::ViewFilters,
) -> Result<Option<ColumnarBatches>, RestError> {
    let Some(mut rest) = runner
        .run_view_columnar(tenant, view_json.clone(), filters.clone())
        .await
        .map_err(map_sof_error_to_rest)?
    else {
        return Ok(None);
    };
    let first = match rest.next().await {
        Some(Ok(batch)) => Some(batch),
        Some(Err(e)) => {
            warn!(error = %e, "batch error before the columnar response started");
            return Err(map_sof_error_to_rest(e));
        }
        None => None,
    };
    let schema = match &first {
        Some(batch) => batch.schema(),
        None => {
            let declared = try_declared_columns(view_json).map_err(map_sof_lib_error_to_rest)?;
            let columns: Vec<String> = declared.iter().map(|d| d.name.clone()).collect();
            std::sync::Arc::new(
                resolve_arrow_schema(&columns, &declared, &[])
                    .map_err(map_sof_lib_error_to_rest)?,
            )
        }
    };
    Ok(Some(ColumnarBatches {
        schema,
        first,
        rest,
    }))
}

/// Encodes every batch into one buffer (the `Binary` envelope needs the whole
/// payload). A batch error returns an error status, never a truncated body.
async fn encode_batches(
    batches: ColumnarBatches,
    content_type: ContentType,
    options: &ParquetOptions,
) -> Result<Vec<u8>, RestError> {
    let ColumnarBatches {
        schema,
        first,
        mut rest,
    } = batches;
    let mut enc = ColumnarEncoder::new(content_type, Vec::new(), schema, options)
        .map_err(map_sof_lib_error_to_rest)?;
    if let Some(batch) = first {
        enc.write_batch(&batch).map_err(map_sof_lib_error_to_rest)?;
    }
    while let Some(item) = rest.next().await {
        let batch = item.map_err(|e| {
            warn!(error = %e, "batch error while collecting columnar stream");
            map_sof_error_to_rest(e)
        })?;
        enc.write_batch(&batch).map_err(map_sof_lib_error_to_rest)?;
    }
    enc.finish().map_err(map_sof_lib_error_to_rest)
}

/// Streams the runner's record batches as Parquet or Arrow IPC through the
/// same blocking encoder as [`streaming_columnar_response`]: past the first
/// batch the `200` is committed, so a batch error aborts the chunked body.
fn streaming_batch_response(
    batches: ColumnarBatches,
    content_type: ContentType,
    options: ParquetOptions,
    runner_label: Option<&str>,
    format: &str,
) -> Response {
    let ColumnarBatches {
        schema,
        first,
        mut rest,
    } = batches;
    let (msg_tx, body_rx) = spawn_columnar_encoder(content_type, schema, options);

    tokio::spawn(async move {
        if let Some(batch) = first
            && msg_tx.send(EncodeMsg::Batch(batch)).await.is_err()
        {
            return;
        }
        loop {
            let item = tokio::select! {
                _ = msg_tx.closed() => return,
                item = rest.next() => item,
            };
            let Some(item) = item else { break };
            match item {
                Ok(batch) => {
                    if msg_tx.send(EncodeMsg::Batch(batch)).await.is_err() {
                        return;
                    }
                }
                Err(e) => {
                    warn!(error = %e, "batch error while streaming columnar output");
                    let _ = msg_tx.send(EncodeMsg::Fail(e.to_string())).await;
                    return;
                }
            }
        }
        let _ = msg_tx.send(EncodeMsg::End).await;
    });

    columnar_body_response(body_rx, content_type, runner_label, format)
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

    let since = params
        .since
        .as_deref()
        .map(|s| parse_instant_param("_since", s))
        .transpose()?;
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
            state.config().sof_runner_header.then_some("in-process"),
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
        state.config().sof_runner_header.then_some("in-process"),
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
        ContentType::ArrowIpc => ("application/vnd.apache.arrow.stream", "arrow"),
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
/// `application/vnd.apache.arrow.stream` → `arrow`,
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
                "application/vnd.apache.arrow.stream" => Some("arrow"),
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
        "arrow" | "application/vnd.apache.arrow.stream" => Some(ContentType::ArrowIpc),
        _ => None,
    }
}

/// Runs `helios_sof::lint` (#821) against an inline ViewDefinition subject
/// and, if it reports any error-severity diagnostic, rejects the request
/// with `422 Unprocessable Entity` and one `OperationOutcome.issue` per
/// diagnostic — identical in shape to sof-server's own `$sql-run` `422`
/// (`helios_sof::lint::lint_operation_outcome`). Warnings never block the
/// request.
fn lint_inline_view_definition(view_json: &Value) -> Result<(), RestError> {
    let diagnostics = lint_view_definition(view_json);
    let has_errors = diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error);
    if has_errors {
        Err(RestError::ValidationFailed {
            outcome: lint_operation_outcome(&diagnostics),
        })
    } else {
        Ok(())
    }
}

/// Write-path guard for stored ViewDefinitions (#1014): a `resource`
/// that names no FHIR resource type can never run, so a create or update
/// carrying one is rejected with `422` and the linter's own
/// `OperationOutcome` (issue code `code-invalid`, coding
/// `unknown-resource-type`) regardless of `HFS_VALIDATION_MODE`. Every
/// other lint finding is deliberately *not* a write error: the editor's
/// "save it anyway" flow (#821) stores drafts on purpose. A no-op for
/// any other resource type.
pub(crate) fn reject_unknown_view_definition_resource(
    resource_type: &str,
    resource: &Value,
) -> Result<(), RestError> {
    if resource_type != "ViewDefinition" {
        return Ok(());
    }
    let diagnostics: Vec<_> = lint_view_definition(resource)
        .into_iter()
        .filter(|diagnostic| diagnostic.code == DiagnosticCode::UnknownResourceType)
        .collect();
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(RestError::ValidationFailed {
            outcome: lint_operation_outcome(&diagnostics),
        })
    }
}

/// The `column 'x': ...` part of a declared-column misfit error, if `msg`
/// carries one. A value that does not fit a declared (or `ansi/type`-tagged)
/// column is the caller's data/view mismatch, not an internal fault, so it is
/// reported to the client with the column name (a 422). Runners may wrap the
/// lib error in their own text, hence the search rather than a prefix match.
pub(crate) fn declared_misfit_detail(msg: &str) -> Option<&str> {
    let start = msg.find("column '")?;
    let detail = &msg[start..];
    (detail.contains("cannot be represented")).then_some(detail)
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
        LibErr::ParquetConversionError(msg) if declared_misfit_detail(&msg).is_some() => {
            RestError::UnprocessableEntity {
                message: declared_misfit_detail(&msg).unwrap_or_default().to_string(),
            }
        }
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
    runner_label: Option<&str>,
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

    channel_body_response(rx, "application/x-ndjson", runner_label)
}

/// Streams a `RowStream` through `encoder` as csv, a pretty-printed json array
/// or the `fhir` `Parameters` resource, served under `content_type_header`.
///
/// The bytes are those of `format_csv` / `format_json` /
/// `format_view_fhir_parameters` (see [`helios_sof::RowStreamEncoder`]),
/// produced row by row so the result set is never held in memory. The first
/// [`SCHEMA_SAMPLE_ROWS`] rows are encoded before anything is sent, so an error
/// inside that prefix still returns a real error status; past it the `200` is
/// committed and a row error aborts the chunked body (no terminating chunk, so
/// the client sees a failed body, not a truncated array) like the NDJSON path.
async fn streaming_rows_response(
    mut stream: helios_persistence::core::sof_runner::RowStream,
    mut encoder: helios_sof::RowStreamEncoder,
    view_json: &Value,
    content_type_header: &'static str,
    runner_label: Option<&str>,
) -> Result<Response, RestError> {
    let mut head: Vec<u8> = Vec::new();
    let mut ended = false;
    for _ in 0..SCHEMA_SAMPLE_ROWS {
        match stream.next().await {
            Some(Ok(row)) => {
                encoder.push_row(row).map_err(map_sof_lib_error_to_rest)?;
                head.extend(encoder.take().map_err(map_sof_lib_error_to_rest)?);
            }
            Some(Err(e)) => {
                warn!(error = %e, "row error before the response started");
                return Err(map_sof_error_to_rest(e));
            }
            None => {
                ended = true;
                break;
            }
        }
    }
    // The buffered path rejects a bad declared column once the rows are in.
    try_declared_columns(view_json).map_err(map_sof_lib_error_to_rest)?;
    // A stream that ended inside the prefix is already complete: answer like
    // the buffered path (Content-Length, and the compression layer's small-body
    // rule) rather than as a chunked body.
    if ended {
        head.extend(encoder.finish().map_err(map_sof_lib_error_to_rest)?);
        return Ok(build_response(
            StatusCode::OK,
            content_type_header,
            head,
            runner_label,
            "",
        ));
    }
    let encoder = Some(encoder);

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(64);
    tokio::spawn(async move {
        let abort = |message: String| std::io::Error::other(message);
        if !head.is_empty() && tx.send(Ok(axum::body::Bytes::from(head))).await.is_err() {
            return;
        }
        let Some(mut encoder) = encoder else {
            return;
        };
        let mut buf: Vec<u8> = Vec::new();
        loop {
            // Stop as soon as the client is gone, even if the runner is slow.
            let item = tokio::select! {
                _ = tx.closed() => return,
                item = stream.next() => item,
            };
            let Some(item) = item else { break };
            let pushed = match item {
                Ok(row) => encoder.push_row(row).and_then(|()| encoder.take()),
                Err(e) => {
                    warn!(error = %e, "row error while streaming rows");
                    let _ = tx
                        .send(Err(abort(format!("row error while streaming rows: {e}"))))
                        .await;
                    return;
                }
            };
            match pushed {
                Ok(bytes) => buf.extend(bytes),
                Err(e) => {
                    warn!(error = %e, "row encoding failed while streaming rows");
                    let _ = tx
                        .send(Err(abort(format!("row encoding failed: {e}"))))
                        .await;
                    return;
                }
            }
            if buf.len() >= COLUMNAR_CHUNK_BYTES
                && tx
                    .send(Ok(axum::body::Bytes::from(std::mem::take(&mut buf))))
                    .await
                    .is_err()
            {
                return;
            }
        }
        match encoder.finish() {
            Ok(tail) => {
                buf.extend(tail);
                let _ = tx.send(Ok(axum::body::Bytes::from(buf))).await;
            }
            Err(e) => {
                let _ = tx
                    .send(Err(abort(format!("row encoding failed: {e}"))))
                    .await;
            }
        }
    });

    Ok(channel_body_response(rx, content_type_header, runner_label))
}

/// Wraps a channel of body chunks into a `200` response. An `Err` item aborts
/// the chunked transfer (no terminating chunk), so the client sees a failed
/// body instead of a clean but truncated one.
fn channel_body_response(
    rx: tokio::sync::mpsc::Receiver<Result<axum::body::Bytes, std::io::Error>>,
    content_type: &'static str,
    runner_label: Option<&str>,
) -> Response {
    let body_stream = futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|chunk| (chunk, rx))
    });
    let body = axum::body::Body::from_stream(body_stream);

    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    if let Some(v) = runner_label.and_then(|label| HeaderValue::from_str(label).ok()) {
        response.headers_mut().insert("x-hfs-runner", v);
    }
    response
}

/// Size of the chunks the columnar encoder hands to the response body.
const COLUMNAR_CHUNK_BYTES: usize = 64 * 1024;

/// `Write` adapter that forwards encoded bytes into the response body channel
/// in [`COLUMNAR_CHUNK_BYTES`] chunks. Meant for a blocking thread: it uses
/// `blocking_send`, which also gives backpressure from a slow client.
struct ChannelWriter {
    tx: tokio::sync::mpsc::Sender<Result<axum::body::Bytes, std::io::Error>>,
    buf: Vec<u8>,
}

impl ChannelWriter {
    fn send_buf(&mut self) -> std::io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk = axum::body::Bytes::from(std::mem::take(&mut self.buf));
        self.tx
            .blocking_send(Ok(chunk))
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "client disconnected"))
    }
}

impl std::io::Write for ChannelWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= COLUMNAR_CHUNK_BYTES {
            self.send_buf()?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.send_buf()
    }
}

/// Message from the async row reader to the blocking encoder.
enum EncodeMsg {
    Rows(Vec<ProcessedRow>),
    /// A batch already built by the runner's columnar path.
    Batch(helios_sof::arrow::record_batch::RecordBatch),
    /// The stream ended cleanly.
    End,
    /// The stream failed; the encoder must abort without writing a footer.
    Fail(String),
}

/// Maps flat JSON rows onto `columns`; missing keys and non-object rows
/// become nulls.
fn to_processed_rows(columns: &[String], rows: &[Value]) -> Vec<ProcessedRow> {
    rows.iter()
        .map(|row| ProcessedRow {
            values: columns
                .iter()
                .map(|col| row.as_object().and_then(|o| o.get(col)).cloned())
                .collect(),
        })
        .collect()
}

/// Builds `ParquetOptions` from the `Parameters` body (`rowGroupSize`,
/// `pageSize`, `compression`), validated against the same ranges as
/// `sof-server`. `maxFileSize` does not apply to a single streamed response and
/// is ignored. `None` when none of the parameters was supplied.
fn parquet_options_from_params(
    body: &ExtractedRunParams,
) -> Result<Option<ParquetOptions>, RestError> {
    if body.row_group_size.is_none() && body.page_size.is_none() && body.compression.is_none() {
        return Ok(None);
    }
    let bad = |message: String| RestError::BadRequest { message };
    let mut options = ParquetOptions::default();
    if let Some(v) = body.row_group_size {
        if !(64..=1024).contains(&v) {
            return Err(bad("rowGroupSize must be between 64 and 1024 MB".into()));
        }
        options.row_group_size_mb = v as u32;
    }
    if let Some(v) = body.page_size {
        if !(64..=8192).contains(&v) {
            return Err(bad("pageSize must be between 64 and 8192 KB".into()));
        }
        options.page_size_kb = v as u32;
    }
    if let Some(c) = &body.compression {
        let c = c.to_ascii_lowercase();
        if !["none", "snappy", "gzip", "lz4", "brotli", "zstd"].contains(&c.as_str()) {
            return Err(bad(format!(
                "compression must be one of none, snappy, gzip, lz4, brotli, zstd (got '{c}')"
            )));
        }
        options.compression = c;
    }
    Ok(Some(options))
}

/// Streams a `RowStream` as Parquet or Arrow IPC.
///
/// The first [`SCHEMA_SAMPLE_ROWS`] rows are read before anything is sent: they
/// fix the column order and feed schema inference for columns without a
/// declared type, and an error inside that prefix still returns a real error
/// status. Past it the `200` is committed, so a row error aborts the chunked
/// body (no footer, no terminating chunk) like the NDJSON path.
///
/// Encoding runs on a blocking thread fed through a bounded channel, so memory
/// stays near one row group instead of the whole result. A client that
/// disconnects makes the encoder's write fail, which ends the reader and drops
/// the `RowStream` so the runner stops.
async fn streaming_columnar_response(
    mut stream: helios_persistence::core::sof_runner::RowStream,
    content_type: ContentType,
    view_json: &Value,
    parquet_options: Option<ParquetOptions>,
    runner_label: Option<&str>,
    format: &str,
) -> Result<Response, RestError> {
    let mut prefix: Vec<Value> = Vec::new();
    let mut ended = false;
    while prefix.len() < SCHEMA_SAMPLE_ROWS {
        match stream.next().await {
            Some(Ok(row)) => prefix.push(row),
            Some(Err(e)) => {
                warn!(error = %e, "row error before the columnar response started");
                return Err(map_sof_error_to_rest(e));
            }
            None => {
                ended = true;
                break;
            }
        }
    }

    let declared: Vec<DeclaredColumn> =
        try_declared_columns(view_json).map_err(map_sof_lib_error_to_rest)?;
    // Column order follows the first row; a key that only shows up later in the
    // prefix is appended, so it is not dropped once the schema is fixed.
    let columns: Vec<String> = match prefix.first() {
        Some(Value::Object(first)) => {
            let mut columns: Vec<String> = first.keys().cloned().collect();
            for row in &prefix {
                if let Value::Object(map) = row {
                    for key in map.keys() {
                        if !columns.contains(key) {
                            columns.push(key.clone());
                        }
                    }
                }
            }
            columns
        }
        _ => declared.iter().map(|d| d.name.clone()).collect(),
    };
    let prefix_rows = to_processed_rows(&columns, &prefix);
    let schema = std::sync::Arc::new(
        resolve_arrow_schema(&columns, &declared, &prefix_rows)
            .map_err(map_sof_lib_error_to_rest)?,
    );
    drop(prefix);

    let (row_tx, body_rx) =
        spawn_columnar_encoder(content_type, schema, parquet_options.unwrap_or_default());

    // Async reader: forwards the prefix, then the rest of the stream in batches.
    tokio::spawn(async move {
        if !prefix_rows.is_empty() && row_tx.send(EncodeMsg::Rows(prefix_rows)).await.is_err() {
            return;
        }
        if ended {
            let _ = row_tx.send(EncodeMsg::End).await;
            return;
        }
        let mut batch: Vec<ProcessedRow> = Vec::new();
        loop {
            // Stop as soon as the encoder is gone (client disconnected or the
            // encoder failed), even if the runner is slow to produce a row.
            let item = tokio::select! {
                _ = row_tx.closed() => return,
                item = stream.next() => item,
            };
            let Some(item) = item else { break };
            match item {
                Ok(row) => {
                    batch.extend(to_processed_rows(&columns, std::slice::from_ref(&row)));
                    if batch.len() >= STREAM_BATCH_ROWS
                        && row_tx
                            .send(EncodeMsg::Rows(std::mem::take(&mut batch)))
                            .await
                            .is_err()
                    {
                        return;
                    }
                }
                Err(e) => {
                    warn!(error = %e, "row error while streaming columnar output");
                    let _ = row_tx.send(EncodeMsg::Fail(e.to_string())).await;
                    return;
                }
            }
        }
        if !batch.is_empty() && row_tx.send(EncodeMsg::Rows(batch)).await.is_err() {
            return;
        }
        let _ = row_tx.send(EncodeMsg::End).await;
    });

    Ok(columnar_body_response(
        body_rx,
        content_type,
        runner_label,
        format,
    ))
}

/// The Parquet or Arrow IPC encoder behind the columnar outputs.
enum ColumnarEncoder<W: std::io::Write + Send> {
    Parquet(ParquetStreamWriter<W>),
    Arrow(ArrowIpcStreamWriter<W>),
}

impl<W: std::io::Write + Send> ColumnarEncoder<W> {
    /// `content_type` is Parquet or Arrow IPC; `options` only apply to Parquet.
    fn new(
        content_type: ContentType,
        sink: W,
        schema: std::sync::Arc<helios_sof::arrow::datatypes::Schema>,
        options: &ParquetOptions,
    ) -> Result<Self, helios_sof::SofError> {
        Ok(match content_type {
            ContentType::ArrowIpc => Self::Arrow(ArrowIpcStreamWriter::new(sink, schema)?),
            _ => Self::Parquet(ParquetStreamWriter::new(sink, schema, options)?),
        })
    }

    fn write_rows(&mut self, rows: &[ProcessedRow]) -> Result<(), helios_sof::SofError> {
        match self {
            Self::Parquet(w) => w.write_rows(rows),
            Self::Arrow(w) => w.write_rows(rows),
        }
    }

    fn write_batch(
        &mut self,
        batch: &helios_sof::arrow::record_batch::RecordBatch,
    ) -> Result<(), helios_sof::SofError> {
        match self {
            Self::Parquet(w) => w.write_batch(batch),
            Self::Arrow(w) => w.write_batch(batch),
        }
    }

    fn finish(self) -> Result<W, helios_sof::SofError> {
        match self {
            Self::Parquet(w) => w.finish(),
            Self::Arrow(w) => w.finish(),
        }
    }
}

/// Starts the blocking encoder: [`EncodeMsg`]s in, encoded body chunks out.
/// A failure (row error, encoder error, client gone) aborts the body without
/// writing a footer.
fn spawn_columnar_encoder(
    content_type: ContentType,
    schema: std::sync::Arc<helios_sof::arrow::datatypes::Schema>,
    options: ParquetOptions,
) -> (
    tokio::sync::mpsc::Sender<EncodeMsg>,
    tokio::sync::mpsc::Receiver<Result<axum::body::Bytes, std::io::Error>>,
) {
    let (msg_tx, mut msg_rx) = tokio::sync::mpsc::channel::<EncodeMsg>(2);
    let (body_tx, body_rx) =
        tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(8);

    let abort_tx = body_tx.clone();
    tokio::task::spawn_blocking(move || {
        let sink = ChannelWriter {
            tx: body_tx,
            buf: Vec::new(),
        };
        let outcome = (|| -> Result<(), String> {
            let mut enc = ColumnarEncoder::new(content_type, sink, schema, &options)
                .map_err(|e| e.to_string())?;
            loop {
                match msg_rx.blocking_recv() {
                    Some(EncodeMsg::Rows(rows)) => {
                        enc.write_rows(&rows).map_err(|e| e.to_string())?
                    }
                    Some(EncodeMsg::Batch(batch)) => {
                        enc.write_batch(&batch).map_err(|e| e.to_string())?
                    }
                    Some(EncodeMsg::End) => break,
                    Some(EncodeMsg::Fail(m)) => return Err(m),
                    None => return Err("row reader stopped unexpectedly".to_string()),
                }
            }
            let mut sink = enc.finish().map_err(|e| e.to_string())?;
            std::io::Write::flush(&mut sink).map_err(|e| e.to_string())
        })();
        if let Err(message) = outcome {
            warn!(error = %message, "columnar encoding aborted");
            let _ = abort_tx.blocking_send(Err(std::io::Error::other(format!(
                "row error while streaming {}: {message}",
                content_type_headers(content_type).1
            ))));
        }
    });
    (msg_tx, body_rx)
}

/// The streamed `200` for a columnar body, with the parquet attachment header
/// when `format` asks for it.
fn columnar_body_response(
    body_rx: tokio::sync::mpsc::Receiver<Result<axum::body::Bytes, std::io::Error>>,
    content_type: ContentType,
    runner_label: Option<&str>,
    format: &str,
) -> Response {
    let mut response =
        channel_body_response(body_rx, content_type_headers(content_type).0, runner_label);
    if wants_parquet_attachment(format, content_type_headers(content_type).0) {
        response.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=\"output.parquet\""),
        );
    }
    response
}

/// Whether a response in `format` carries the parquet `Content-Disposition`.
fn wants_parquet_attachment(format: &str, content_type: &str) -> bool {
    (format == "parquet"
        || format == "application/octet-stream"
        || format == "application/vnd.apache.parquet")
        && content_type != FHIR_JSON_MIME
}

/// Renders a `RowStream` to `(content_type_header, bytes)` for the requested
/// format. NDJSON has its own dedicated streaming path
/// ([`streaming_ndjson_response`]); buffered formats (csv, json, parquet, arrow) drain
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
    view_json: &Value,
    parquet_options: Option<&ParquetOptions>,
) -> Result<(&'static str, Vec<u8>), RestError> {
    let rows = drain_stream(stream).await?;
    let result = helios_sof::rows_to_processed_result(rows);
    let declared = try_declared_columns(view_json).map_err(map_sof_lib_error_to_rest)?;
    let body =
        helios_sof::format_output_with_schema(result, content_type, parquet_options, &declared)
            .map_err(map_sof_lib_error_to_rest)?;
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

/// Builds the final `Response` with an optional `X-HFS-Runner` header (added
/// only when `runner_label` is `Some`, i.e. `HFS_SOF_RUNNER_HEADER` is on) and
/// an optional `Content-Disposition` attachment header for parquet. Absent
/// `patient` / `group` targets are surfaced as a 400 + OperationOutcome
/// upstream, not as `Warning: 199` headers on this response.
fn build_response(
    status: StatusCode,
    content_type: &'static str,
    body: Vec<u8>,
    runner_label: Option<&str>,
    format: &str,
) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    if let Some(label) = runner_label {
        headers.insert(
            "x-hfs-runner",
            HeaderValue::from_str(label).unwrap_or_else(|_| HeaderValue::from_static("unknown")),
        );
    }
    if wants_parquet_attachment(format, content_type) {
        headers.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=\"output.parquet\""),
        );
    }
    (status, headers, body).into_response()
}

/// The effective `patient` and `group` values: the body's repeated entries win
/// when non-empty, otherwise the comma-split query string.
fn effective_patient_group(
    params: &RunQueryParams,
    body: &ExtractedRunParams,
) -> (Vec<String>, Vec<String>) {
    let patient = if !body.patient.is_empty() {
        body.patient.clone()
    } else {
        split_csv_refs(params.patient.as_deref())
    };
    let group = if !body.group.is_empty() {
        body.group.clone()
    } else {
        split_csv_refs(params.group.as_deref())
    };
    (patient, group)
}

/// Builds `ViewFilters` from query parameters. An unparsable `_since` is a
/// `400`, never a dropped filter.
fn build_filters(
    params: &RunQueryParams,
    body_extra: &ExtractedRunParams,
) -> Result<ViewFilters, RestError> {
    let since = params
        .since
        .as_deref()
        .map(|s| parse_instant_param("_since", s))
        .transpose()?;

    let (patient, group) = effective_patient_group(params, body_extra);

    Ok(ViewFilters {
        patient,
        group,
        since,
        limit: params.limit,
    })
}

/// Maps a `SofError` to a `RestError`, returning 422 for uncompilable views and
/// 503 when the store is unavailable.
pub(crate) fn map_sof_error_to_rest(e: SofError) -> RestError {
    match e {
        SofError::Uncompilable { reason }
        | SofError::InvalidViewDefinition(reason)
        | SofError::ResolutionLimit(reason) => RestError::UnprocessableEntity { message: reason },
        SofError::Cancelled => RestError::InternalError {
            message: "View execution was cancelled".to_string(),
        },
        // A declared-column misfit surfaced by a runner's columnar builder.
        SofError::Backend(ref msg) if declared_misfit_detail(msg).is_some() => {
            RestError::UnprocessableEntity {
                message: declared_misfit_detail(msg).unwrap_or_default().to_string(),
            }
        }
        SofError::Unavailable(message) => {
            warn!(error = %message, "SofRunner storage unavailable");
            RestError::ServiceUnavailable { message }
        }
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
        let response = streaming_ndjson_response(stream, Some("test-runner"));
        assert_eq!(response.status(), StatusCode::OK);
        // A mid-stream error must abort the chunked body, not end it cleanly:
        // collecting an aborted body fails.
        let collected = axum::body::to_bytes(response.into_body(), usize::MAX).await;
        assert!(
            collected.is_err(),
            "expected the aborted chunked body to fail collection"
        );
    }

    fn typed_view() -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{ "column": [
                { "path": "id", "name": "id", "type": "string" },
                { "path": "n", "name": "n", "type": "integer" }
            ]}]
        })
    }

    fn numbered_rows(count: usize) -> Vec<Result<Value, SofError>> {
        (0..count)
            .map(|i| Ok(json!({ "id": format!("p{i}"), "n": i })))
            .collect()
    }

    async fn columnar(
        rows: Vec<Result<Value, SofError>>,
        ct: ContentType,
        options: Option<ParquetOptions>,
    ) -> Result<Response, RestError> {
        streaming_columnar_response(
            row_stream(rows),
            ct,
            &typed_view(),
            options,
            None,
            "parquet",
        )
        .await
    }

    #[tokio::test]
    async fn streaming_parquet_aborts_on_row_error() {
        // The error comes after the schema prefix, so the 200 is committed.
        let mut rows = numbered_rows(150);
        rows.push(Err(SofError::Cancelled));
        let response = columnar(rows, ContentType::Parquet, None).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let collected = axum::body::to_bytes(response.into_body(), usize::MAX).await;
        assert!(collected.is_err(), "expected an aborted parquet body");
    }

    #[tokio::test]
    async fn streaming_arrow_aborts_on_row_error() {
        let mut rows = numbered_rows(150);
        rows.push(Err(SofError::Cancelled));
        let response = columnar(rows, ContentType::ArrowIpc, None).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let collected = axum::body::to_bytes(response.into_body(), usize::MAX).await;
        assert!(collected.is_err(), "expected an aborted arrow body");
    }

    #[tokio::test]
    async fn streaming_columnar_client_disconnect_drops_the_row_stream() {
        // An endless stream: it can only stop if the response body being
        // dropped makes the encoder and the reader give up.
        struct DropFlag(std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = DropFlag(dropped.clone());
        let stream: RowStream = Box::pin(futures::stream::unfold(0usize, move |i| {
            let _keep = &flag;
            async move {
                tokio::task::yield_now().await;
                Some((Ok(json!({ "id": format!("p{i}"), "n": i })), i + 1))
            }
        }));
        let options = ParquetOptions {
            row_group_size_mb: 0,
            ..ParquetOptions::default()
        };
        let response = streaming_columnar_response(
            stream,
            ContentType::Parquet,
            &typed_view(),
            Some(options),
            None,
            "parquet",
        )
        .await
        .unwrap();
        let mut body = response.into_body().into_data_stream();
        assert!(body.next().await.is_some(), "expected a first chunk");
        drop(body);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !dropped.load(std::sync::atomic::Ordering::SeqCst) {
            assert!(
                std::time::Instant::now() < deadline,
                "row stream still alive after the client disconnected"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn streaming_columnar_error_inside_prefix_is_a_real_status() {
        let mut rows = numbered_rows(3);
        rows.push(Err(SofError::Cancelled));
        assert!(columnar(rows, ContentType::Parquet, None).await.is_err());
    }

    #[tokio::test]
    async fn streaming_parquet_round_trips_with_declared_types_and_row_groups() {
        use bytes::Bytes;
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

        // row_group_size_mb = 0 cuts a row group after every 8192-row batch.
        let options = ParquetOptions {
            row_group_size_mb: 0,
            ..ParquetOptions::default()
        };
        let response = columnar(numbered_rows(20_000), ContentType::Parquet, Some(options))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().contains_key(header::CONTENT_DISPOSITION));
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("clean stream");

        let builder = ParquetRecordBatchReaderBuilder::try_new(Bytes::from(bytes.to_vec()))
            .expect("valid parquet");
        let schema = builder.schema().clone();
        assert_eq!(
            schema.field_with_name("n").unwrap().data_type(),
            &arrow::datatypes::DataType::Int32
        );
        assert!(builder.metadata().num_row_groups() >= 3);
        let total: usize = builder
            .build()
            .unwrap()
            .map(|b| b.unwrap().num_rows())
            .sum();
        assert_eq!(total, 20_000);
    }

    #[tokio::test]
    async fn streamed_parquet_matches_the_buffered_formatter() {
        use bytes::Bytes;
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

        // Same rows through the streaming response and through the buffered
        // formatter with the same declared types: schema and rows must agree.
        let raw = numbered_rows(9_000);
        let rows: Vec<Value> = raw.iter().map(|r| r.as_ref().unwrap().clone()).collect();
        let streamed = axum::body::to_bytes(
            columnar(raw, ContentType::Parquet, None)
                .await
                .unwrap()
                .into_body(),
            usize::MAX,
        )
        .await
        .unwrap();

        let columns = vec!["id".to_string(), "n".to_string()];
        let buffered = helios_sof::format_output_with_schema(
            helios_sof::ProcessedResult {
                rows: to_processed_rows(&columns, &rows),
                columns,
            },
            ContentType::Parquet,
            None,
            &try_declared_columns(&typed_view()).unwrap(),
        )
        .unwrap();

        let read = |bytes: Vec<u8>| {
            let builder = ParquetRecordBatchReaderBuilder::try_new(Bytes::from(bytes)).unwrap();
            let schema = builder.schema().clone();
            let batches: Vec<_> = builder.build().unwrap().map(|b| b.unwrap()).collect();
            (
                schema,
                arrow::compute::concat_batches(&batches[0].schema(), &batches).unwrap(),
            )
        };
        let (s_schema, s_rows) = read(streamed.to_vec());
        let (b_schema, b_rows) = read(buffered);
        assert_eq!(s_schema, b_schema);
        assert_eq!(s_rows, b_rows);
    }

    // ----- declared (strict) vs inferred (lenient) misfits ------------------
    //
    // Layer covered: the row-stream -> streaming Parquet/Arrow encoder, with
    // synthetic rows (the SQLite runner is columnar and cannot yield a mixed
    // type in an untyped column, so the REST-level tests cannot reach this).

    fn untyped_view() -> Value {
        json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{ "column": [
                { "path": "id", "name": "id", "type": "string" },
                { "path": "v", "name": "v" }
            ]}]
        })
    }

    async fn columnar_with(
        view: &Value,
        rows: Vec<Result<Value, SofError>>,
        ct: ContentType,
    ) -> Result<Response, RestError> {
        streaming_columnar_response(row_stream(rows), ct, view, None, None, "parquet").await
    }

    /// An inferred column's schema is fixed from the first 100 rows (integers
    /// here); a later text value does not fit it and becomes NULL.
    #[tokio::test]
    async fn streaming_inferred_column_misfit_after_the_prefix_becomes_null() {
        use bytes::Bytes;
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

        let mut rows: Vec<Result<Value, SofError>> = (0..150)
            .map(|i| Ok(json!({ "id": format!("p{i}"), "v": i })))
            .collect();
        rows.push(Ok(json!({ "id": "late", "v": "not a number" })));
        let response = columnar_with(&untyped_view(), rows, ContentType::Parquet)
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("an inferred misfit must not abort the stream");
        let reader = ParquetRecordBatchReaderBuilder::try_new(Bytes::from(bytes.to_vec()))
            .unwrap()
            .build()
            .unwrap();
        let batches: Vec<_> = reader.map(|b| b.unwrap()).collect();
        let total: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total, 151);
        let last = batches.last().unwrap();
        let v = last.column_by_name("v").unwrap();
        assert!(arrow::array::Array::is_null(v, last.num_rows() - 1));
        let id = last.column_by_name("id").unwrap();
        assert!(!arrow::array::Array::is_null(id, last.num_rows() - 1));
    }

    /// A declared column's misfit inside the schema prefix is a real status
    /// (nothing has been sent yet) and names the column.
    #[tokio::test]
    async fn streaming_declared_misfit_inside_the_prefix_is_an_error_status() {
        // 2^40 does not fit the declared `integer` (Int32); the exact error is
        // raised when the prefix is turned into a schema/rows or at encode time.
        let rows = vec![Ok(json!({ "id": "a", "n": 1i64 << 40 }))];
        match columnar(rows, ContentType::Parquet, None).await {
            Err(e) => assert!(e.to_string().contains("'n'"), "{e}"),
            Ok(response) => {
                // The encoder rejects the value after the 200 is committed.
                let collected = axum::body::to_bytes(response.into_body(), usize::MAX).await;
                assert!(collected.is_err(), "a misfit must not yield a clean file");
            }
        }
    }

    /// After the 200 is committed (row 151 onwards) a declared misfit cannot
    /// change the status: the chunked body is aborted so no truncated file
    /// looks complete.
    #[tokio::test]
    async fn streaming_declared_misfit_after_the_prefix_aborts_the_body() {
        let mut rows = numbered_rows(150);
        rows.push(Ok(json!({ "id": "late", "n": 1i64 << 40 })));
        for ct in [ContentType::Parquet, ContentType::ArrowIpc] {
            let response = columnar(
                rows.iter()
                    .map(|r| r.as_ref().cloned().map_err(|_| SofError::Cancelled))
                    .collect(),
                ct,
                None,
            )
            .await
            .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let collected = axum::body::to_bytes(response.into_body(), usize::MAX).await;
            assert!(collected.is_err(), "{ct:?}: expected an aborted body");
        }
    }

    #[test]
    fn declared_misfit_maps_to_unprocessable_entity_naming_the_column() {
        let lib = helios_sof::SofError::ParquetConversionError(
            "column 'n': value 1 cannot be represented as integer".to_string(),
        );
        assert!(matches!(
            map_sof_lib_error_to_rest(lib),
            RestError::UnprocessableEntity { message } if message.contains("'n'")
        ));
        let wrapped = SofError::Backend(
            "failed to build record batch: Parquet conversion error: column 'n': a text value \
             cannot be represented in its declared type Int32"
                .to_string(),
        );
        assert!(matches!(
            map_sof_error_to_rest(wrapped),
            RestError::UnprocessableEntity { message } if message.starts_with("column 'n'")
        ));
        // Other conversion failures stay internal.
        let other = helios_sof::SofError::ParquetConversionError("Failed to flush".to_string());
        assert!(matches!(
            map_sof_lib_error_to_rest(other),
            RestError::InternalError { .. }
        ));
    }

    #[tokio::test]
    async fn streaming_columnar_keeps_a_column_that_first_shows_up_later() {
        use bytes::Bytes;
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

        let rows = vec![
            Ok(json!({ "id": "a", "n": 1 })),
            Ok(json!({ "id": "b", "n": 2, "extra": "x" })),
        ];
        let bytes = axum::body::to_bytes(
            columnar(rows, ContentType::Parquet, None)
                .await
                .unwrap()
                .into_body(),
            usize::MAX,
        )
        .await
        .unwrap();
        let builder =
            ParquetRecordBatchReaderBuilder::try_new(Bytes::from(bytes.to_vec())).unwrap();
        assert!(builder.schema().field_with_name("extra").is_ok());
    }

    #[tokio::test]
    async fn streaming_arrow_round_trips() {
        let response = columnar(numbered_rows(250), ContentType::ArrowIpc, None)
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("clean stream");
        let reader =
            arrow::ipc::reader::StreamReader::try_new(std::io::Cursor::new(bytes.to_vec()), None)
                .unwrap();
        let total: usize = reader.map(|b| b.unwrap().num_rows()).sum();
        assert_eq!(total, 250);
    }

    #[tokio::test]
    async fn streaming_parquet_empty_result_keeps_declared_columns() {
        use bytes::Bytes;
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

        let response = columnar(vec![], ContentType::Parquet, None).await.unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("clean stream");
        let builder =
            ParquetRecordBatchReaderBuilder::try_new(Bytes::from(bytes.to_vec())).unwrap();
        assert_eq!(builder.schema().fields().len(), 2);
    }

    #[test]
    fn parquet_options_from_params_validates_ranges() {
        let none = ExtractedRunParams::default();
        assert!(parquet_options_from_params(&none).unwrap().is_none());
        let ok = ExtractedRunParams {
            row_group_size: Some(64),
            compression: Some("ZSTD".into()),
            ..Default::default()
        };
        let o = parquet_options_from_params(&ok).unwrap().unwrap();
        assert_eq!((o.row_group_size_mb, o.compression.as_str()), (64, "zstd"));
        let bad = ExtractedRunParams {
            row_group_size: Some(1),
            ..Default::default()
        };
        assert!(parquet_options_from_params(&bad).is_err());
    }

    #[tokio::test]
    async fn streaming_ndjson_completes_on_clean_stream() {
        let stream = row_stream(vec![Ok(json!({ "a": 1 })), Ok(json!({ "a": 2 }))]);
        let response = streaming_ndjson_response(stream, Some("test-runner"));
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("a clean stream should produce a collectable body");
        let text = String::from_utf8(bytes.to_vec()).expect("utf-8 body");
        assert_eq!(text, "{\"a\":1}\n{\"a\":2}\n");
    }

    async fn rows_response(
        rows: Vec<Result<Value, SofError>>,
        ct: ContentType,
    ) -> Result<Response, RestError> {
        let encoder = helios_sof::RowStreamEncoder::new(ct).unwrap();
        streaming_rows_response(
            row_stream(rows),
            encoder,
            &typed_view(),
            content_type_headers(ct).0,
            Some("test-runner"),
        )
        .await
    }

    async fn fhir_rows_response(
        rows: Vec<Result<Value, SofError>>,
        view: &Value,
    ) -> Result<Response, RestError> {
        let encoder =
            helios_sof::RowStreamEncoder::fhir(view).map_err(map_sof_lib_error_to_rest)?;
        streaming_rows_response(
            row_stream(rows),
            encoder,
            view,
            FHIR_JSON_MIME,
            Some("test-runner"),
        )
        .await
    }

    #[tokio::test]
    async fn streaming_fhir_matches_the_buffered_parameters() {
        // More rows than the prefix and than one body chunk, so every phase runs.
        for count in [0, 1, 5_000] {
            let rows = numbered_rows(count);
            let values: Vec<Value> = rows.iter().map(|r| r.as_ref().unwrap().clone()).collect();
            let response = fhir_rows_response(rows, &typed_view()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE).unwrap(),
                FHIR_JSON_MIME
            );
            assert_eq!(
                response.headers().get("x-hfs-runner").unwrap(),
                "test-runner"
            );
            let streamed = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let buffered = format_view_fhir_parameters(
                &helios_sof::rows_to_processed_result(values),
                &typed_view(),
            )
            .unwrap();
            assert_eq!(streamed.to_vec(), buffered, "{count} rows");
        }
    }

    #[tokio::test]
    async fn streaming_fhir_complex_value_in_the_prefix_is_a_real_status() {
        let mut rows = numbered_rows(3);
        rows.push(Ok(json!({ "id": { "family": "Smith" }, "n": 1 })));
        assert!(fhir_rows_response(rows, &typed_view()).await.is_err());
    }

    #[tokio::test]
    async fn streaming_fhir_error_after_the_prefix_aborts_the_body() {
        for bad in [
            Err(SofError::Cancelled),
            Ok(json!({ "id": { "family": "Smith" }, "n": 1 })),
        ] {
            let mut rows = numbered_rows(SCHEMA_SAMPLE_ROWS + 10);
            rows.push(bad);
            let response = fhir_rows_response(rows, &typed_view()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let collected = axum::body::to_bytes(response.into_body(), usize::MAX).await;
            assert!(collected.is_err(), "the aborted body must fail");
        }
    }

    #[tokio::test]
    async fn streaming_json_and_csv_match_the_buffered_formatters() {
        // More rows than the prefix and than one body chunk, so every phase runs.
        let raw = numbered_rows(5_000);
        let values: Vec<Value> = raw.iter().map(|r| r.as_ref().unwrap().clone()).collect();
        for ct in [
            ContentType::Json,
            ContentType::Csv,
            ContentType::CsvWithHeader,
        ] {
            let response = rows_response(numbered_rows(5_000), ct).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE).unwrap(),
                content_type_headers(ct).0
            );
            let streamed = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let (_, buffered) =
                format_stream(row_stream(numbered_rows(5_000)), ct, &typed_view(), None)
                    .await
                    .unwrap();
            assert_eq!(streamed.to_vec(), buffered, "{ct:?}");
            assert_eq!(
                buffered,
                helios_sof::format_output(
                    helios_sof::rows_to_processed_result(values.clone()),
                    ct,
                    None
                )
                .unwrap()
            );
        }
    }

    #[tokio::test]
    async fn streaming_small_results_keep_the_buffered_headers() {
        use http_body::Body as _;
        for count in [0, 1] {
            for ct in [ContentType::Json, ContentType::Csv] {
                let response = rows_response(numbered_rows(count), ct).await.unwrap();
                // hyper turns an exact body size into Content-Length; a
                // channel body has no exact size and would go out chunked.
                let exact = response.body().size_hint().exact();
                let streamed = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                assert_eq!(
                    exact,
                    Some(streamed.len() as u64),
                    "exact size, so Content-Length"
                );
                let (_, buffered) =
                    format_stream(row_stream(numbered_rows(count)), ct, &typed_view(), None)
                        .await
                        .unwrap();
                assert_eq!(streamed.to_vec(), buffered, "{count} rows {ct:?}");
            }
        }
    }

    #[tokio::test]
    async fn streaming_json_empty_result_is_an_empty_array() {
        let response = rows_response(vec![], ContentType::Json).await.unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], b"[]");
    }

    #[tokio::test]
    async fn streaming_rows_error_inside_the_prefix_is_a_real_status() {
        let mut rows = numbered_rows(3);
        rows.push(Err(SofError::Cancelled));
        for ct in [ContentType::Json, ContentType::Csv] {
            assert!(
                rows_response(rows_clone(&rows), ct).await.is_err(),
                "{ct:?}"
            );
        }
    }

    fn rows_clone(rows: &[Result<Value, SofError>]) -> Vec<Result<Value, SofError>> {
        rows.iter()
            .map(|r| match r {
                Ok(v) => Ok(v.clone()),
                Err(_) => Err(SofError::Cancelled),
            })
            .collect()
    }

    #[tokio::test]
    async fn streaming_rows_error_after_the_prefix_aborts_the_body() {
        let mut rows = numbered_rows(SCHEMA_SAMPLE_ROWS + 10);
        rows.push(Err(SofError::Cancelled));
        for ct in [ContentType::Json, ContentType::CsvWithHeader] {
            let response = rows_response(rows_clone(&rows), ct).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let collected = axum::body::to_bytes(response.into_body(), usize::MAX).await;
            assert!(collected.is_err(), "{ct:?}: the aborted body must fail");
        }
    }

    #[tokio::test]
    async fn drain_stream_errors_on_row_error() {
        let stream = row_stream(vec![Ok(json!({ "a": 1 })), Err(SofError::Cancelled)]);
        assert!(
            drain_stream(stream).await.is_err(),
            "a mid-stream row error must propagate instead of truncating"
        );
    }

    /// A runner with no columnar path: only the defaulted
    /// `run_view_columnar`, which answers `None`.
    struct RowOnlyRunner;

    #[async_trait::async_trait]
    impl helios_persistence::core::sof_runner::SofRunner for RowOnlyRunner {
        async fn run_view(
            &self,
            _tenant: &helios_persistence::tenant::TenantContext,
            _view_definition: Value,
            _filters: helios_persistence::core::sof_runner::ViewFilters,
        ) -> Result<RowStream, SofError> {
            Ok(row_stream(vec![Ok(json!({ "a": 1 }))]))
        }

        fn runner_name(&self) -> &'static str {
            "row-only-test-runner"
        }
    }

    /// A runner whose columnar path yields `batches` (possibly none), then an
    /// error when `fail` is set.
    struct BatchRunner {
        batches: usize,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl helios_persistence::core::sof_runner::SofRunner for BatchRunner {
        async fn run_view(
            &self,
            _tenant: &helios_persistence::tenant::TenantContext,
            _view_definition: Value,
            _filters: helios_persistence::core::sof_runner::ViewFilters,
        ) -> Result<RowStream, SofError> {
            panic!("the columnar path must not fall back to run_view");
        }

        async fn run_view_columnar(
            &self,
            _tenant: &helios_persistence::tenant::TenantContext,
            _view_definition: Value,
            _filters: helios_persistence::core::sof_runner::ViewFilters,
        ) -> Result<Option<helios_persistence::core::sof_runner::BatchStream>, SofError> {
            use helios_sof::arrow::array::{ArrayRef, Int32Array};
            use helios_sof::arrow::datatypes::{DataType, Field, Schema};
            use helios_sof::arrow::record_batch::RecordBatch;

            let schema =
                std::sync::Arc::new(Schema::new(vec![Field::new("n", DataType::Int32, true)]));
            let column: ArrayRef = std::sync::Arc::new(Int32Array::from(vec![1, 2, 3]));
            let batch = RecordBatch::try_new(schema, vec![column]).expect("batch");
            let mut items: Vec<_> = (0..self.batches).map(|_| Ok(batch.clone())).collect();
            if self.fail {
                items.push(Err(SofError::Backend("boom".to_string())));
            }
            Ok(Some(Box::pin(futures::stream::iter(items))))
        }

        fn runner_name(&self) -> &'static str {
            "batch-test-runner"
        }
    }

    fn test_tenant() -> helios_persistence::tenant::TenantContext {
        use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
        TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
    }

    /// A view declaring one `integer` column `n`.
    fn int_view() -> Value {
        json!({ "select": [{ "column": [{ "name": "n", "path": "x", "type": "integer" }] }] })
    }

    async fn open(
        runner: &dyn helios_persistence::core::sof_runner::SofRunner,
    ) -> Result<Option<ColumnarBatches>, RestError> {
        open_columnar(runner, &test_tenant(), &int_view(), &Default::default()).await
    }

    /// The streamed and the buffered (envelope) bodies of the columnar path.
    async fn columnar_bodies(
        runner: &dyn helios_persistence::core::sof_runner::SofRunner,
        content_type: ContentType,
    ) -> Option<(Vec<u8>, Vec<u8>)> {
        let batches = open(runner).await.expect("open_columnar")?;
        let response = streaming_batch_response(
            batches,
            content_type,
            ParquetOptions::default(),
            None,
            "parquet",
        );
        let streamed = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("streamed body")
            .to_vec();
        let batches = open(runner).await.expect("open_columnar")?;
        let buffered = encode_batches(batches, content_type, &ParquetOptions::default())
            .await
            .expect("buffered body");
        Some((streamed, buffered))
    }

    #[tokio::test]
    async fn columnar_falls_back_when_runner_has_no_columnar_path() {
        assert!(open(&RowOnlyRunner).await.expect("open").is_none());
        // The row path the caller then takes still produces Parquet.
        let (mime, body) = format_stream(
            row_stream(vec![Ok(json!({ "a": 1 }))]),
            ContentType::Parquet,
            &json!({}),
            None,
        )
        .await
        .expect("row path");
        assert_eq!(mime, "application/vnd.apache.parquet");
        assert!(body.starts_with(b"PAR1"));
    }

    #[tokio::test]
    async fn columnar_batches_are_encoded_without_the_row_path() {
        let runner = BatchRunner {
            batches: 2,
            fail: false,
        };

        let (streamed, buffered) = columnar_bodies(&runner, ContentType::Parquet)
            .await
            .expect("columnar output");
        for body in [&streamed, &buffered] {
            assert!(body.starts_with(b"PAR1"));
            let reader =
                helios_sof::parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(
                    bytes::Bytes::from(body.clone()),
                )
                .expect("parquet reader")
                .build()
                .expect("reader");
            let rows: usize = reader.map(|b| b.expect("batch").num_rows()).sum();
            assert_eq!(rows, 6);
        }

        let (streamed, buffered) = columnar_bodies(&runner, ContentType::ArrowIpc)
            .await
            .expect("columnar output");
        assert_eq!(streamed, buffered);
        let reader = helios_sof::arrow::ipc::reader::StreamReader::try_new(
            std::io::Cursor::new(streamed),
            None,
        )
        .expect("ipc reader");
        let rows: usize = reader.map(|b| b.expect("batch").num_rows()).sum();
        assert_eq!(rows, 6);
    }

    #[tokio::test]
    async fn columnar_zero_batches_match_the_empty_row_output() {
        let runner = BatchRunner {
            batches: 0,
            fail: false,
        };
        for ct in [ContentType::Parquet, ContentType::ArrowIpc] {
            let (streamed, buffered) = columnar_bodies(&runner, ct).await.expect("columnar output");
            let response = streaming_columnar_response(
                row_stream(vec![]),
                ct,
                &int_view(),
                None,
                None,
                "parquet",
            )
            .await
            .expect("row path");
            let expected = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("row body");
            assert_eq!(streamed, expected.to_vec(), "{ct:?}");
            assert_eq!(buffered, expected.to_vec(), "{ct:?}");
        }
    }

    #[tokio::test]
    async fn columnar_error_before_the_first_batch_is_an_error_status() {
        let runner = BatchRunner {
            batches: 0,
            fail: true,
        };
        assert!(open(&runner).await.is_err());
    }

    #[tokio::test]
    async fn columnar_error_after_the_first_batch_aborts_the_body() {
        let runner = BatchRunner {
            batches: 1,
            fail: true,
        };
        let batches = open(&runner).await.expect("first batch").expect("columnar");
        let response = streaming_batch_response(
            batches,
            ContentType::Parquet,
            ParquetOptions::default(),
            None,
            "parquet",
        );
        assert!(
            axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .is_err()
        );
        let batches = open(&runner).await.expect("first batch").expect("columnar");
        assert!(
            encode_batches(batches, ContentType::Parquet, &ParquetOptions::default())
                .await
                .is_err()
        );
    }

    /// A store that is down, throttling or unable to read an object after
    /// retries answers 503 with `Retry-After`, not 500 (#2048).
    #[test]
    fn an_unavailable_store_is_a_503() {
        let rest = map_sof_error_to_rest(SofError::Unavailable(
            "backend unavailable: s3: failed to read object body after 3 attempts".to_string(),
        ));
        assert!(
            matches!(rest, RestError::ServiceUnavailable { .. }),
            "got {rest:?}"
        );
        assert_eq!(rest.client_response().0, StatusCode::SERVICE_UNAVAILABLE);

        let response = rest.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            response
                .headers()
                .contains_key(axum::http::header::RETRY_AFTER),
            "a 503 must carry Retry-After"
        );
    }

    /// Only an unavailable store is a 503: other storage and backend failures
    /// are still server faults.
    #[test]
    fn other_storage_errors_stay_500() {
        for error in [
            SofError::Storage("x".to_string()),
            SofError::Backend("x".to_string()),
        ] {
            let rest = map_sof_error_to_rest(error);
            assert!(
                matches!(rest, RestError::InternalError { .. }),
                "got {rest:?}"
            );
        }
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
