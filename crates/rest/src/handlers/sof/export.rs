//! `$sql-export` operation handler.
//!
//! Implements the SQL on FHIR
//! [`$sql-export`](http://hl7.org/fhir/uv/sql-on-fhir/OperationDefinition-SQLExport.html)
//! operation: asynchronous export of one or more subjects as a single job,
//! following the FHIR Asynchronous Interaction Request Pattern.
//!
//! | Route | Method | Description |
//! |-------|--------|-------------|
//! | `/$sql-export` | POST | Submit an export job naming one or more subjects |
//! | `/export/{job-id}/status` | GET | Poll for job status |
//! | `/export/{job-id}/status` | DELETE | Cancel job |
//! | `/export/{job-id}/result` | GET | Fetch the completion manifest |
//! | `/export/{job-id}/{filename}` | GET | Download output file |
//!
//! The operation is invoked at the **system level** and names what it acts on
//! through a repeating `subject` parameter, so one job may mix ViewDefinitions,
//! SQLQuery Libraries and SQLView Libraries. That mixture is the point: every
//! subject is computed against a single snapshot of the data, under one set of
//! filters, so a view output and a query output can be joined on a shared key
//! without a skew window.
//!
//! Output formats are bound to `ExportOutputFormatCodes` (`ndjson` default,
//! `csv`, `json`, `parquet`). `fhir` is deliberately absent: it applies to
//! `$sql-run` only, since an export produces flat files.
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
//! A tenant may have at most `HFS_EXPORT_MAX_JOBS_PER_TENANT` jobs queued or
//! running; one more is `429 Too Many Requests` with `Retry-After`.
//!
//! A request over any of these fixed limits is a `400 Bad Request` that names
//! the limit, returned before the work it bounds: 64 `subject` entries, 1000
//! `patient` plus `group` values, 256 `context` entries, and
//! 4 × `HFS_SOF_SQLQUERY_MAX_VDS` `depends-on` entries per Library.
//!
//! Submit errors for the `patient`/`group` filters (#1701):
//!
//! - `400 Bad Request` with an `OperationOutcome` naming the parameter for an
//!   unusable `patient`/`group` value: an absolute URL, `urn:uuid:`, the other
//!   type, a versioned or contained reference, an empty id, or a body entry
//!   with no reference string.
//! - `404 Not Found` for a usable `patient`/`group` reference to a missing
//!   Patient/Group.
//! - Shapes are checked before existence.
//! - An empty query value is treated as not supplied.
//!
//! ## Poll response
//!
//! Per the FHIR Asynchronous Interaction Request Pattern, the status URL only
//! reports polling machinery; the job's outcome is served from a separate
//! result URL (`GET /export/{job-id}/result`):
//!
//! - `202 Accepted` while the job is running, with `X-Progress: {n}%`
//!   (`n = floor(subjectsDone × 100 / subjectsTotal)`, capped at 99 until the
//!   result is available) and `Retry-After: 5`. The body is a `Parameters`
//!   resource:
//!
//!   | Parameter | Type | Present when |
//!   |-----------|------|--------------|
//!   | `exportId` | `valueString` | always |
//!   | `status` | `valueCode` (`in-progress`) | always |
//!   | `subjectsTotal` | `valueInteger` | always — total subjects in the job |
//!   | `subjectsDone` | `valueInteger` | always — subjects fully finished |
//!   | `currentSubject` | `valueString` | a subject is being written; absent before the first one starts or after the last one finishes. Equal to the `output.name` that subject will carry in the completion manifest |
//!   | `estimatedTimeRemaining` | `valueInteger` (seconds) | progress is between 0 and 100% |
//!
//!   `subjectsTotal`/`subjectsDone`/`currentSubject` are this server's own
//!   extension of the spec's allowance for implementation-defined interim
//!   content; a client that only reads `X-Progress` notices nothing (#853).
//! - `303 See Other` with a `Location` header carrying the result URL and an
//!   empty body once the job has finished — whether it succeeded or failed
//! - `404 Not Found` if the job ID is unknown or was cancelled
//!
//! ## Result response (`GET /export/{job-id}/result`)
//!
//! - `200 OK` with the completion manifest `Parameters` resource on success
//! - the failure's own status (a `4xx` for the request's own fault, `500` for a
//!   server fault) + `OperationOutcome` if the job failed, whose
//!   `issue[0].expression` carries the failed subject's output name (#1800)
//! - `404 Not Found` if the job is unknown, cancelled, or not yet finished

use axum::{
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use helios_persistence::core::ResourceStorage;
use helios_persistence::core::search::SearchProvider;
use helios_sof::fhir_format::accept_requires_unsupported_fhir_xml;
use serde::Deserialize;
use serde_json::{Value, json};

use super::sqlquery::{sqlquery_err_to_rest, validate_select_only};
use super::subject::{
    RENAMED_VIEW_PARAM_MESSAGE, RENAMED_VIEW_PARAM_NAME, SubjectKind, SubjectRef, resolve_subject,
};
use super::view_sources::extract_table_source_views;
use crate::error::RestError;
use crate::export::controller::{
    ExportTask, ExportWork, JobStatus, NamedSqlQuery, NamedView, SqlExportLimits, SubmitError,
};
use crate::extractors::TenantExtractor;
use crate::handlers::bulk_common::parse_instant_param;
use crate::state::AppState;

/// Top-level `Parameters` body parameter names recognised by `$sql-export`.
/// Anything outside this list is rejected with 400 per the spec's
/// "reject unsupported parameters" rule.
///
/// `_limit` is deliberately absent: it caps the rows returned to the client in
/// an operation response, and an export delivers files rather than rows, so
/// there is nothing for it to cap. Supplying it here is a 400.
const ALLOWED_BODY_PARAMS: &[&str] = &[
    "subject",
    "context",
    "_format",
    "header",
    "patient",
    "group",
    "_since",
    "clientTrackingId",
    "source",
];

/// `Retry-After` for a job refused by the per-tenant job limit. A place frees
/// when one of the tenant's jobs ends, which no one can time, so this is the
/// status poll's cadence.
const JOB_LIMIT_RETRY_AFTER_SECS: u64 = 5;

/// Output formats this server can serialize. The spec binds the export
/// `_format` to the extensible `ExportOutputFormatCodes` value set
/// (`csv`, `ndjson`, `parquet`, `json`); we reject anything outside this list
/// with 400 per the spec's "reject unsupported parameters" rule rather than
/// silently downgrading the output to NDJSON. `fhir` is deliberately absent:
/// per the spec's Common Operation Behavior it applies to `$sql-run` only,
/// since an export produces flat files.
const SUPPORTED_FORMATS: &[&str] = &["ndjson", "csv", "json", "parquet"];

/// Query parameters for `$sql-export`.
///
/// `deny_unknown_fields` enforces the spec's "reject unsupported parameters
/// with 400 Bad Request" rule on the query string. Any parameter outside this
/// struct (whether spec-defined-but-unsupported or simply unknown) surfaces
/// as a serde error, which axum/serde maps to a 400 response — including
/// `_limit`, which the export operation does not offer.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportQueryParams {
    /// Output format: `ndjson` (default), `csv`, `json`, or `parquet`
    /// (`ExportOutputFormatCodes`; `fhir` is a run-operation-only format).
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
// Submit: POST [base]/$sql-export
// ============================================================================

/// Submit an export job.
///
/// `$sql-export` is invoked at the **system level** with `POST`, since it
/// creates a job. Each repetition of the `subject` parameter names one artifact
/// — a ViewDefinition, a SQLQuery Library or a SQLView Library — and produces
/// exactly one `output` entry in the manifest. Any mixture may be named in one
/// request, and every subject is computed against a single snapshot of the
/// data, under one set of filters.
///
/// Accepts:
/// - A FHIR `Parameters` resource with one or more `subject` parameters whose
///   `part` entries supply `name`, one of `subjectCanonical` /
///   `subjectReference` / `subjectResource`, and optional `parameters`
///   bindings; plus job-wide `_format`, `header`, `patient`, `group`,
///   `_since` and `clientTrackingId`.
/// - A bare `ViewDefinition` resource, as shorthand for a single unnamed
///   subject.
///
/// Query-string values take precedence over body values for the same parameter.
pub async fn sql_export_handler<S>(
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
        if let Some(resp) = validate_unknown_body_params(b, ALLOWED_BODY_PARAMS, "$sql-export") {
            return Ok(resp);
        }
    }
    if let Some(resp) = reject_unsupported_source(&params, body_value.as_ref()) {
        return Ok(resp);
    }

    let Some(body) = body_value else {
        return Ok(missing_subject_response());
    };

    // Pure, and it bounds the `patient`/`group` values, so an over-limit
    // request gets its 400 before any subject is resolved.
    let inputs = merge_export_inputs(&params, Some(&body))?;

    let work = extract_subjects_from_body(&state, &tenant, &body).await?;
    if work.is_empty() {
        return Ok(missing_subject_response());
    }

    submit_export_job(&state, &tenant, work, inputs).await
}

/// 400 response for a request that names no subject. `subject` is `1..*`, and
/// the spec is explicit that a request supplying none is rejected.
fn missing_subject_response() -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(json!({
            "resourceType": "OperationOutcome",
            "issue": [{"severity": "error", "code": "required",
                "diagnostics": "$sql-export requires at least one `subject`; each repetition \
                                supplies one of `subjectCanonical`, `subjectReference` or \
                                `subjectResource`",
                "expression": ["subject"]}]
        })),
    )
        .into_response()
}

/// Walks the body's repeating `subject` parameter, resolving each named
/// artifact and sorting it into the views or queries half of the job.
///
/// A Library subject is fully prepared here — its dependency graph resolved and
/// its parameter bindings applied — so the background job needs no storage
/// access once it starts.
async fn extract_subjects_from_body<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    body: &Value,
) -> Result<ExportWork, RestError>
where
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    let mut work = ExportWork {
        limits: SqlExportLimits {
            max_source_rows_per_vd: state.config().sof_sqlquery_max_source_rows_per_vd,
            max_rows: state.config().sof_sqlquery_max_rows,
            timeout_secs: state.config().sof_sqlquery_timeout_secs,
        },
        ..Default::default()
    };

    let rt = body
        .get("resourceType")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Shorthand: a bare ViewDefinition body is a single unnamed subject.
    if rt == "ViewDefinition" {
        work.views.push(NamedView {
            name: subject_output_name(None, body, 0),
            view: body.clone(),
        });
        return Ok(work);
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

    // Bound the subject count before any subject is resolved or prepared.
    let subject_count = entries
        .iter()
        .filter(|p| p.get("name").and_then(|n| n.as_str()) == Some("subject"))
        .count();
    super::input_limits::check_export_subjects(subject_count)?;

    // Supporting artifacts supplied once for the whole job, matched to
    // dependencies by canonical URL.
    let table_sources = extract_table_source_views(body)?;

    let mut index = 0usize;
    for p in entries {
        if p.get("name").and_then(|n| n.as_str()) != Some("subject") {
            continue;
        }
        let parts = p.get("part").and_then(|v| v.as_array());
        let mut name: Option<String> = None;
        let mut subject_ref = SubjectRef::default();
        let mut bindings: Option<Value> = None;

        if let Some(arr) = parts {
            for part in arr {
                match part.get("name").and_then(|v| v.as_str()) {
                    Some("name") => {
                        name = part
                            .get("valueString")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                    Some("subjectResource") => subject_ref.resource = part.get("resource").cloned(),
                    Some("subjectCanonical") => {
                        subject_ref.canonical = ["valueCanonical", "valueUri", "valueString"]
                            .iter()
                            .find_map(|k| part.get(*k).and_then(|v| v.as_str()))
                            .map(|s| s.to_string());
                    }
                    Some("subjectReference") => {
                        subject_ref.reference = part
                            .get("valueReference")
                            .and_then(|r| r.get("reference"))
                            .and_then(|v| v.as_str())
                            .or_else(|| part.get("valueString").and_then(|v| v.as_str()))
                            .map(|s| s.to_string());
                    }
                    Some("parameters") => bindings = part.get("resource").cloned(),
                    _ => {}
                }
            }
        }

        let subject = resolve_subject(state, tenant.context(), &subject_ref, "$sql-export").await?;

        // `parameters` binds values the subject declares. A ViewDefinition
        // declares none, so supplying them for one is a 400.
        if bindings.is_some() && !subject.kind.accepts_parameters() {
            return Err(RestError::BadRequest {
                message: format!(
                    "subject '{}' does not accept the `parameters` part: a ViewDefinition \
                     declares no parameters, and the SQLView profile constrains \
                     Library.parameter to 0..0; only a SQLQuery subject accepts `parameters`",
                    subject_output_name(name.as_deref(), &subject.resource, index)
                ),
            });
        }

        let output_name = subject_output_name(name.as_deref(), &subject.resource, index);
        match subject.kind {
            SubjectKind::ViewDefinition => work.views.push(NamedView {
                name: output_name,
                view: subject.resource,
            }),
            SubjectKind::SqlQuery | SubjectKind::SqlView => {
                work.queries.push(
                    prepare_named_sqlquery(
                        state,
                        tenant,
                        Some(output_name),
                        &subject.resource,
                        bindings.as_ref(),
                        &table_sources,
                    )
                    .await?,
                );
            }
        }
        index += 1;
    }

    Ok(work)
}

/// Resolves a subject's output name: the `name` part, else the subject's own
/// `name` element, else a server-generated identifier. Output names are unique
/// across the job, which [`submit_export_job`] enforces.
fn subject_output_name(explicit: Option<&str>, resource: &Value, index: usize) -> String {
    explicit
        .map(|s| s.to_string())
        .or_else(|| {
            resource
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| format!("output-{index}"))
}
/// Parses and validates one SQLQuery or SQLView Library and packages it as a
/// [`NamedSqlQuery`]: validates the SQL is a single SELECT, resolves the full
/// dependency graph (Phase 1 of [`super::graph`] — every ViewDefinition and
/// SQLView Library it reaches, preferring supplied `context` table sources
/// matched by canonical `url`, then falling back to storage resolution),
/// enforces the graph-size cap, and binds `Library.parameter` values.
///
/// The plan carried on the returned [`NamedSqlQuery`] is fully resolved —
/// every artifact fetched, every structural check passed — so the background
/// export job (Phase 2) needs no storage access to materialize it.
async fn prepare_named_sqlquery<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    name_hint: Option<String>,
    library_json: &Value,
    supplied_params: Option<&Value>,
    table_sources: &[Value],
) -> Result<NamedSqlQuery, RestError>
where
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    let library =
        helios_sof::sqlquery::parse_sqlquery_library(library_json).map_err(sqlquery_err_to_rest)?;

    validate_select_only(&library.sql)?;

    let is_sql_view = matches!(
        super::subject::classify_subject(library_json)?,
        super::subject::SubjectKind::SqlView
    );
    let subject_url = library_json.get("url").and_then(|v| v.as_str());
    let fetcher = super::graph::StorageArtifactFetcher::new(state, tenant.context());
    let max_vds = state.config().sof_sqlquery_max_vds;
    let subject_node = super::graph::SubjectNode {
        identity: subject_url,
        is_sql_view,
        parameters_empty: library.parameters.is_empty(),
        depends_on: &library.depends_on,
        max_depends_on: super::input_limits::max_depends_on(max_vds),
        max_nodes: max_vds,
    };
    let plan = super::graph::build_plan(&fetcher, table_sources, subject_node)
        .await
        .map_err(super::graph::errors_to_rest_error)?;

    super::graph::check_max_nodes(&plan, max_vds)?;

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
        plan,
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
                "diagnostics": "$sql-export requires the `Prefer: respond-async` header per the FHIR Asynchronous Interaction Request Pattern"}]
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

/// Validates every subject in the job, then dispatches it and returns 202.
async fn submit_export_job<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    work: ExportWork,
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
                        "unsupported `_format` value '{}'; supported: {}. `fhir` applies to \
                         $sql-run only, since an export produces flat files",
                        inputs.format,
                        SUPPORTED_FORMATS.join(", ")
                    ),
                    "expression": ["_format"]
                }]
            })),
        )
            .into_response());
    }

    // Spec: "a request in which two repetitions would produce the same output
    // name" is rejected. Clients correlate manifest entries by name, and the
    // manifest groups files by name, so a collision would silently collapse two
    // subjects into one `output` entry. Views and queries share one namespace
    // because they share one manifest.
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let names = work
        .views
        .iter()
        .map(|v| v.name.as_str())
        .chain(work.queries.iter().map(|q| q.name.as_str()));
    for name in names {
        if !seen.insert(name) {
            return Ok((
                StatusCode::BAD_REQUEST,
                axum::Json(json!({
                    "resourceType": "OperationOutcome",
                    "issue": [{
                        "severity": "error",
                        "code": "invalid",
                        "diagnostics": format!(
                            "duplicate output name '{name}'; output names are unique across the \
                             job (set `subject.part[name=name].valueString`, or the subject's own \
                             `name` element, to disambiguate)"
                        ),
                        "expression": ["subject.name"]
                    }]
                })),
            )
                .into_response());
        }
    }

    // A ViewDefinition without `resource` names nothing to project.
    for nv in &work.views {
        if nv.view.get("resource").and_then(|v| v.as_str()).is_none() {
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                axum::Json(json!({
                    "resourceType": "OperationOutcome",
                    "issue": [{"severity": "error", "code": "invalid",
                        "diagnostics": format!("ViewDefinition.resource is required (subject '{}')", nv.name)}]
                })),
            )
                .into_response());
        }
    }

    // A `patient` or `group` the server cannot act on is a 400 naming the
    // parameter (#1701); a usable reference to a resource the server cannot
    // find is a 404, kept deliberately (#1701).
    if let Some(resp) = validate_patient_group_refs(state, tenant, &inputs).await? {
        return Ok(resp);
    }

    dispatch_export_job(state, tenant, work, inputs).await
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

    let job_id = match controller.submit(task) {
        Ok(job_id) => job_id,
        Err(e @ SubmitError::TenantJobLimit { .. }) => {
            return Err(RestError::TooManyRequests {
                message: e.to_string(),
                retry_after_secs: Some(JOB_LIMIT_RETRY_AFTER_SECS),
            });
        }
    };
    // Spec: `Content-Location` must be the absolute URL of the status endpoint.
    let location = state.public_url_for_request(tenant, ["export", job_id.as_str(), "status"]);

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
    headers: HeaderMap,
) -> Result<Response, RestError>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    // Spec Common Operation Behavior — Asynchronous Delivery: the `Accept`
    // header on each poll governs that poll's response (interim `202 Accepted`
    // bodies and error responses). The FHIR XML representation is not supported
    // → 406, same as the run operations. The completing poll itself is a
    // `303 See Other` with an empty body, so its representation is unaffected.
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    if accept_requires_unsupported_fhir_xml(accept) {
        return Err(RestError::NotAcceptable {
            message: "the application/fhir+xml representation is not supported; \
                      use application/fhir+json"
                .to_string(),
        });
    }

    let controller = match state.export_controller() {
        Some(c) => c,
        None => {
            return Ok((StatusCode::SERVICE_UNAVAILABLE, "export not configured").into_response());
        }
    };

    match controller.get_status(tenant.tenant_id(), &job_id) {
        None | Some(JobStatus::Cancelled { .. }) => Ok((
            StatusCode::NOT_FOUND,
            axum::Json(json!({
                "resourceType": "OperationOutcome",
                "issue": [{"severity": "error", "code": "not-found",
                    "diagnostics": format!("Export job '{job_id}' not found or was cancelled")}]
            })),
        )
            .into_response()),

        Some(status @ JobStatus::Running { .. }) => {
            // `percent()` is the single place the completion percentage is
            // derived from `subjects_done`/`subjects_total` (#853), so
            // `X-Progress` and the body's `subjectsDone`/`subjectsTotal`
            // parameters below always agree.
            let percent = status.percent().unwrap_or(0);
            let JobStatus::Running {
                subjects_done,
                subjects_total,
                current_subject,
                submitted_at,
            } = status
            else {
                unreachable!("guarded by the match arm pattern")
            };

            let mut headers = HeaderMap::new();
            // Spec: `X-Progress` carries a completion percentage (e.g. `65%`).
            let progress_value = format!("{percent}%");
            if let Ok(v) = HeaderValue::from_str(&progress_value) {
                headers.insert("x-progress", v);
            }
            // Spec SHOULD: include Retry-After during polling.
            headers.insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
            // Spec: in-progress body is an optional `Parameters` resource.
            // `subjectsTotal`/`subjectsDone`/`currentSubject` are this
            // server's own extension of that allowance (#853) — always
            // present while the job runs, except `currentSubject`, which is
            // absent when no subject is currently being written.
            let mut params = vec![
                json!({"name": "exportId", "valueString": job_id}),
                json!({"name": "status", "valueCode": "in-progress"}),
                json!({"name": "subjectsTotal", "valueInteger": subjects_total}),
                json!({"name": "subjectsDone", "valueInteger": subjects_done}),
            ];
            if let Some(name) = current_subject {
                params.push(json!({"name": "currentSubject", "valueString": name}));
            }
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

        // Spec (Common Operation Behavior — Asynchronous Delivery, FHIR
        // Asynchronous Interaction Request Pattern): once the job has finished
        // — whether it succeeded or failed — the status poll returns
        // `303 See Other` with a `Location` header carrying the result URL and
        // an empty body. The status endpoint reflects polling machinery only;
        // it never communicates the job's outcome. The outcome (manifest on
        // success, OperationOutcome on failure) is served from the result URL.
        Some(JobStatus::Failed { .. }) | Some(JobStatus::Completed { .. }) => {
            let result_url =
                state.public_url_for_request(&tenant, ["export", job_id.as_str(), "result"]);
            let mut headers = HeaderMap::new();
            if let Ok(v) = HeaderValue::from_str(&result_url) {
                headers.insert(header::LOCATION, v);
            }
            Ok((StatusCode::SEE_OTHER, headers).into_response())
        }
    }
}

/// Constructs the SQL-on-FHIR v2 completion manifest as a FHIR `Parameters` resource.
fn build_completion_manifest(
    status_url: &str,
    job_id: &str,
    outputs: &[(String, String)],
    submitted_at: chrono::DateTime<chrono::Utc>,
    completed_at: chrono::DateTime<chrono::Utc>,
    format: &str,
    client_tracking_id: Option<&str>,
) -> Value {
    // `outputs` is a list of (view_name, download URL) pairs, one per shard,
    // resolved fresh by the caller (see the completed-status branch) so S3
    // pre-signed URLs carry a full TTL window on every poll.
    //
    // Spec: one `output` per view, with `location` (1..*) repeating once per
    // shard inside it. Group by `view_name` while preserving first-seen
    // insertion order — this stays correct even if a controller emits files
    // for a view non-contiguously.
    let mut output_order: Vec<String> = Vec::new();
    let mut locations_by_view: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for (view_name, location) in outputs {
        if !locations_by_view.contains_key(view_name) {
            output_order.push(view_name.clone());
        }
        locations_by_view
            .entry(view_name.clone())
            .or_default()
            .push(location.clone());
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
// Result: GET /export/{job-id}/result
// ============================================================================

/// Serve the result of a finished export job.
///
/// Per the FHIR Asynchronous Interaction Request Pattern, the completing status
/// poll redirects here with `303 See Other`. A successful export returns
/// `200 OK` with the manifest `Parameters` resource; a failed export returns the
/// relevant error status code (e.g. `500 Internal Server Error`) with an
/// `OperationOutcome` whose `issue[0].expression` carries the output name of the
/// subject that failed (#1800). The result and its download URLs remain valid for at
/// least 24 hours, so repeated fetches return the same outcome within that
/// window. A job that is unknown, cancelled, or still in progress has no result
/// to serve and returns `404 Not Found`.
pub async fn get_export_result_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    Path(job_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, RestError>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    // The result `GET`'s `Accept` header governs the manifest representation.
    // FHIR XML is not supported → 406, same as the run operations.
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    if accept_requires_unsupported_fhir_xml(accept) {
        return Err(RestError::NotAcceptable {
            message: "the application/fhir+xml representation is not supported; \
                      use application/fhir+json"
                .to_string(),
        });
    }

    let controller = match state.export_controller() {
        Some(c) => c,
        None => {
            return Ok((StatusCode::SERVICE_UNAVAILABLE, "export not configured").into_response());
        }
    };

    match controller.get_status(tenant.tenant_id(), &job_id) {
        // Unknown, cancelled, or still running → no result available. The
        // result URL is only handed out (via the status poll's 303) once the
        // job has finished, so reaching here otherwise means there is nothing
        // to serve.
        None | Some(JobStatus::Cancelled { .. }) | Some(JobStatus::Running { .. }) => Ok((
            StatusCode::NOT_FOUND,
            axum::Json(json!({
                "resourceType": "OperationOutcome",
                "issue": [{"severity": "error", "code": "not-found",
                    "diagnostics": format!("No result available for export job '{job_id}'")}]
            })),
        )
            .into_response()),

        // Failed export → the failure's own status (the 4xx `$sql-run` gives
        // a request's fault such as a row limit, 500 for a server fault) with
        // an OperationOutcome body explaining it. `issue[0].expression` names
        // the subject that failed by its output name (#1800); that is the
        // client's own input, so it is there even when a server fault's
        // `diagnostics` are generic.
        Some(JobStatus::Failed {
            message,
            status,
            code,
            subject,
            ..
        }) => {
            let mut issue = json!({
                "severity": "error",
                "code": code,
                "diagnostics": format!("Export job '{job_id}' failed: {message}")
            });
            if let Some(name) = subject {
                issue["expression"] = json!([name]);
            }
            Ok((
                status,
                axum::Json(json!({"resourceType": "OperationOutcome", "issue": [issue]})),
            )
                .into_response())
        }

        // Successful export → `200 OK` with the manifest `Parameters` resource.
        Some(JobStatus::Completed {
            files,
            submitted_at,
            completed_at,
            format,
            client_tracking_id,
        }) => {
            // Resolve every shard's download URL fresh on this poll. For S3 this
            // re-signs the GET URL so it carries a full TTL window from now; for
            // server-routed sinks it is a stable route. A resolution failure
            // (e.g. an S3 pre-signing error) must not yield a manifest with a
            // missing/empty `location`, so surface it as a 500 instead.
            let mut outputs: Vec<(String, String)> = Vec::with_capacity(files.len());
            let public_base_url = state.public_base_url_for_request(&tenant);
            for f in &files {
                match controller.download_url(
                    tenant.tenant_id(),
                    &public_base_url,
                    &job_id,
                    &f.filename,
                ) {
                    Some(url) => outputs.push((f.view_name.clone(), url)),
                    None => {
                        return Ok((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            axum::Json(json!({
                                "resourceType": "OperationOutcome",
                                "issue": [{"severity": "error", "code": "processing",
                                    "diagnostics": format!(
                                        "Failed to resolve a download URL for export job '{job_id}'")}]
                            })),
                        )
                            .into_response());
                    }
                }
            }

            // Spec: the result URL and download URLs SHALL be valid for at least
            // 24 hours and MAY carry an `Expires` header (IMF-fixdate, RFC 7231).
            //
            // `Expires` tracks the result-*retention* window, not the lifetime of
            // any single pre-signed URL: download URLs are re-resolved on every
            // poll (above), so a client re-polling within the window always gets
            // a fresh URL even if an individual S3 pre-signature has a shorter
            // TTL. 24h matches the default output-retention reaper, after which
            // the job and its output are reclaimed.
            let expires_at = completed_at + chrono::Duration::hours(24);
            let expires_str = expires_at.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
            let mut headers = HeaderMap::new();
            if let Ok(v) = HeaderValue::from_str(&expires_str) {
                headers.insert(header::EXPIRES, v);
            }
            // `Expires` still advertises the retention window; `no-store` keeps any
            // cache from holding a copy of the (possibly pre-signed) URLs.
            set_export_output_headers(&mut headers);
            let status_url =
                state.public_url_for_request(&tenant, ["export", job_id.as_str(), "status"]);
            Ok((
                StatusCode::OK,
                headers,
                axum::Json(build_completion_manifest(
                    &status_url,
                    &job_id,
                    &outputs,
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
            let content_type = if filename.ends_with(".csv") {
                "text/csv; charset=utf-8"
            } else if filename.ends_with(".parquet") {
                "application/vnd.apache.parquet"
            } else if filename.ends_with(".json") {
                // `_format=json` shards hold a single JSON array of rows.
                "application/json"
            } else {
                "application/x-ndjson"
            };
            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
            set_export_output_headers(&mut headers);
            Ok((StatusCode::OK, headers, data).into_response())
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Responses that carry export output (a shard's rows, or the manifest whose
/// `location`s can be pre-signed URLs) must not be MIME-sniffed or kept by any
/// cache (#1703).
fn set_export_output_headers(headers: &mut HeaderMap) {
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
}

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
                            "unsupported query parameter '{k}' for $sql-export"
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
/// Returns `Some(400 response)` on the first offender. A parameter named
/// exactly `view` — the pre-ballot spelling of `context` — gets a didactic
/// diagnostic naming the rename; any other unrecognised name gets the
/// generic "unsupported body parameter" message.
fn validate_unknown_body_params(body: &Value, allowed: &[&str], op: &str) -> Option<Response> {
    let entries = body.get("parameter").and_then(|v| v.as_array())?;
    for entry in entries {
        let name = entry.get("name").and_then(|n| n.as_str())?;
        if !allowed.contains(&name) {
            let diagnostics = if name == RENAMED_VIEW_PARAM_NAME {
                RENAMED_VIEW_PARAM_MESSAGE.to_string()
            } else {
                format!("unsupported body parameter '{name}' for {op}")
            };
            return Some(
                (
                    StatusCode::BAD_REQUEST,
                    axum::Json(json!({
                        "resourceType": "OperationOutcome",
                        "issue": [{
                            "severity": "error",
                            "code": "not-supported",
                            "diagnostics": diagnostics
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
fn merge_export_inputs(
    query: &ExportQueryParams,
    body: Option<&Value>,
) -> Result<ExportInputs, RestError> {
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
    let body_patient = collect_body_refs(body_params, "patient")?;
    let body_group = collect_body_refs(body_params, "group")?;

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
        .or(body_since.as_deref())
        .map(|s| parse_instant_param("_since", s))
        .transpose()?;
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
    // Bound the values before `validate_patient_group_refs` reads each one.
    super::input_limits::check_patient_group_values(patient.len() + group.len())?;

    Ok(ExportInputs {
        format,
        header,
        since,
        patient,
        group,
        client_tracking_id,
    })
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
/// `Reference.reference` string and falling back to `valueString` for
/// permissive clients that send refs as bare strings. Each value is trimmed.
///
/// An entry that carries no usable reference string (an identifier-only or
/// display-only `valueReference`, a `valueUri`/`valueIdentifier`, an empty or
/// blank string) is a 400 naming the parameter (#1701) rather than being
/// dropped, since dropping it would silently widen the export to everything.
fn collect_body_refs(params: Option<&Vec<Value>>, name: &str) -> Result<Vec<String>, RestError> {
    let mut refs = Vec::new();
    for p in params
        .into_iter()
        .flatten()
        .filter(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))
    {
        let reference =
            helios_sof::params::read_patient_group_reference(p, name).map_err(|error| {
                RestError::InvalidParameter {
                    param: error.param,
                    message: error.message,
                }
            })?;
        refs.push(reference);
    }
    Ok(refs)
}

/// Validates the `patient` and `group` inputs. A value the runner cannot act
/// on (an absolute URL, `urn:uuid:`, a reference to the other type, a
/// versioned or contained reference, an empty id) is a 400 OperationOutcome
/// naming the parameter (#1701). Shapes are checked for every value first;
/// only then is existence checked, and a usable reference to a resource the
/// server cannot find is a 404 listing the missing references.
async fn validate_patient_group_refs<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    inputs: &ExportInputs,
) -> Result<Option<Response>, RestError>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let params: [(&str, &str, &Vec<String>); 2] = [
        ("patient", "Patient", &inputs.patient),
        ("group", "Group", &inputs.group),
    ];

    for (param, resource_type, refs) in params {
        for reference in refs {
            if compartment_ref_id(reference, resource_type).is_none() {
                return Ok(Some(
                    (
                        StatusCode::BAD_REQUEST,
                        axum::Json(json!({
                            "resourceType": "OperationOutcome",
                            "issue": [{
                                "severity": "error",
                                "code": "invalid",
                                "diagnostics": format!(
                                    "`{param}` value '{reference}' is not a reference this \
                                     server can resolve; use a relative `{resource_type}/{{id}}` reference"
                                ),
                                "expression": [param]
                            }]
                        })),
                    )
                        .into_response(),
                ));
            }
        }
    }

    let mut missing: Vec<String> = Vec::new();
    for (_, resource_type, refs) in params {
        for reference in refs {
            let Some(id) = compartment_ref_id(reference, resource_type) else {
                continue;
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

/// The id a `patient`/`group` value names when the runner can act on it, i.e.
/// a relative `{resource_type}/{id}` reference or a bare `{id}`. `None` for
/// anything else: an absolute URL, a `urn:uuid:`, a reference to another
/// type, a versioned or contained reference, an empty id.
///
/// The check is structural, not the FHIR id grammar: the server does not
/// enforce that grammar on create/PUT, so an id such as `p_1` can exist and
/// the runner can act on it. The id must be non-empty and free of `/`, `:`,
/// `#`, `?`, whitespace and control characters.
fn compartment_ref_id<'a>(reference: &'a str, resource_type: &str) -> Option<&'a str> {
    let id = reference
        .strip_prefix(resource_type)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(reference);
    let valid = !id.is_empty()
        && !id.chars().any(|c| {
            matches!(c, '/' | ':' | '#' | '?') || c.is_ascii_whitespace() || c.is_ascii_control()
        });
    valid.then_some(id)
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

#[cfg(test)]
mod tests {
    use super::{collect_body_refs, compartment_ref_id};
    use serde_json::json;

    #[test]
    fn compartment_ref_id_accepts_relative_and_bare_ids() {
        assert_eq!(compartment_ref_id("Patient/p1", "Patient"), Some("p1"));
        assert_eq!(compartment_ref_id("p1", "Patient"), Some("p1"));
        assert_eq!(compartment_ref_id("Group/g-1.a", "Group"), Some("g-1.a"));
        assert_eq!(compartment_ref_id("Patient/p_1", "Patient"), Some("p_1"));
        let long = "a".repeat(65);
        assert_eq!(
            compartment_ref_id(long.as_str(), "Patient"),
            Some(long.as_str())
        );
    }

    #[test]
    fn collect_body_refs_requires_a_usable_reference_string() {
        let only_identifier = vec![json!({
            "name": "patient",
            "valueReference": {"identifier": {"system": "urn:s", "value": "v"}}
        })];
        assert!(collect_body_refs(Some(&only_identifier), "patient").is_err());

        let blank = vec![json!({"name": "group", "valueString": "  "})];
        assert!(collect_body_refs(Some(&blank), "group").is_err());

        let params = vec![
            json!({"name": "patient", "valueString": " Patient/p1 "}),
            json!({"name": "group", "valueString": "  "}),
            json!({"name": "_format", "valueCode": "csv"}),
        ];
        assert_eq!(
            collect_body_refs(Some(&params), "patient").unwrap(),
            vec!["Patient/p1".to_string()]
        );
        assert!(collect_body_refs(None, "patient").unwrap().is_empty());
    }

    #[test]
    fn compartment_ref_id_rejects_unusable_references() {
        for (reference, resource_type) in [
            ("http://example.org/fhir/Patient/p1", "Patient"),
            ("urn:uuid:0b0e8c86-5a8f-4d2e-9e4a-1d2c3b4a5f60", "Patient"),
            ("Group/g1", "Patient"),
            ("Patient/p1", "Group"),
            ("Patient/p1/_history/2", "Patient"),
            ("Patient/", "Patient"),
            ("", "Patient"),
            ("#p1", "Patient"),
            (" Patient/p1", "Patient"),
        ] {
            assert_eq!(
                compartment_ref_id(reference, resource_type),
                None,
                "{reference:?} must be rejected for {resource_type}"
            );
        }
    }
}
