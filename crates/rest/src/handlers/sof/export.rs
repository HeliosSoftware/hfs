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
//! ## Poll response
//!
//! Per the FHIR Asynchronous Interaction Request Pattern, the status URL only
//! reports polling machinery; the job's outcome is served from a separate
//! result URL (`GET /export/{job-id}/result`):
//!
//! - `202 Accepted` + `X-Progress: running` while the job is running
//! - `303 See Other` with a `Location` header carrying the result URL and an
//!   empty body once the job has finished — whether it succeeded or failed
//! - `404 Not Found` if the job ID is unknown or was cancelled
//!
//! ## Result response (`GET /export/{job-id}/result`)
//!
//! - `200 OK` with the completion manifest `Parameters` resource on success
//! - `500 Internal Server Error` + `OperationOutcome` if the job failed
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

use super::references::resolve_resource_canonical_or_relative;
use super::sqlquery::{sqlquery_err_to_rest, validate_select_only};
use super::subject::{SubjectKind, SubjectRef, resolve_subject};
use super::view_sources::extract_table_source_views;
use crate::error::RestError;
use crate::export::controller::{
    ExportTask, ExportWork, JobStatus, NamedSqlQuery, NamedView, SqlExportLimits, SqlTableSource,
};
use crate::extractors::TenantExtractor;
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

    let work = extract_subjects_from_body(&state, &tenant, &body).await?;
    if work.is_empty() {
        return Ok(missing_subject_response());
    }

    let inputs = merge_export_inputs(&params, Some(&body));
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
                    "subject '{}' is a ViewDefinition, which declares no parameters; \
                     the `parameters` part requires a SQLQuery or SQLView subject",
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
/// Parses and validates one SQLQuery Library and packages it as a
/// [`NamedSqlQuery`]: validates the SQL is a single SELECT, enforces the
/// depends-on cap, resolves every `depends-on` ViewDefinition (preferring
/// supplied `view` table sources, matched by canonical `url` or by name
/// against the depends-on label, then falling back to storage resolution),
/// and binds `Library.parameter` values.
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

    let max_vds = state.config().sof_sqlquery_max_vds;
    if library.depends_on.len() > max_vds {
        return Err(RestError::UnprocessableEntity {
            message: format!(
                "Library declares {} depends-on ViewDefinitions; max allowed is {}",
                library.depends_on.len(),
                max_vds
            ),
        });
    }

    validate_select_only(&library.sql)?;

    let mut tables: Vec<SqlTableSource> = Vec::with_capacity(library.depends_on.len());
    for dep in &library.depends_on {
        let view = match table_sources
            .iter()
            .find(|vd| vd.get("url").and_then(|u| u.as_str()) == Some(dep.url.as_str()))
        {
            Some(vd) => vd.clone(),
            None => {
                resolve_resource_canonical_or_relative(
                    state,
                    tenant.context(),
                    "ViewDefinition",
                    &dep.url,
                )
                .await?
            }
        };
        tables.push(SqlTableSource {
            label: dep.label.clone(),
            view,
        });
    }

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
        tables,
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

    // A `patient` or `group` that names a resource the server cannot find is
    // rejected with 400, not 404: it scopes the data rather than being the
    // thing the operation is about (operations-common.html#filter-resolution-errors).
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

        Some(JobStatus::Running {
            percent,
            submitted_at,
        }) => {
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
            let mut params = vec![
                json!({"name": "exportId", "valueString": job_id}),
                json!({"name": "status", "valueCode": "in-progress"}),
            ];
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
            let result_url = format!(
                "{base}/export/{job_id}/result",
                base = state.base_url().trim_end_matches('/'),
            );
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
    base_url: &str,
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
// Result: GET /export/{job-id}/result
// ============================================================================

/// Serve the result of a finished export job.
///
/// Per the FHIR Asynchronous Interaction Request Pattern, the completing status
/// poll redirects here with `303 See Other`. A successful export returns
/// `200 OK` with the manifest `Parameters` resource; a failed export returns the
/// relevant error status code (e.g. `500 Internal Server Error`) with an
/// `OperationOutcome`. The result and its download URLs remain valid for at
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

        // Failed export → the relevant error status code with an
        // OperationOutcome body explaining the failure.
        Some(JobStatus::Failed { message, .. }) => Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({
                "resourceType": "OperationOutcome",
                "issue": [{
                    "severity": "error",
                    "code": "processing",
                    "diagnostics": format!("Export job '{job_id}' failed: {message}")
                }]
            })),
        )
            .into_response()),

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
            for f in &files {
                match controller.download_url(tenant.tenant_id(), &job_id, &f.filename) {
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
            Ok((
                StatusCode::OK,
                headers,
                axum::Json(build_completion_manifest(
                    state.base_url(),
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
            Ok((StatusCode::OK, [(header::CONTENT_TYPE, content_type)], data).into_response())
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

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
                            "unsupported query parameter '{k}' for $viewdefinition-export"
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
/// Returns `Some(400 response)` on the first offender.
fn validate_unknown_body_params(body: &Value, allowed: &[&str], op: &str) -> Option<Response> {
    let entries = body.get("parameter").and_then(|v| v.as_array())?;
    for entry in entries {
        let name = entry.get("name").and_then(|n| n.as_str())?;
        if !allowed.contains(&name) {
            return Some(
                (
                    StatusCode::BAD_REQUEST,
                    axum::Json(json!({
                        "resourceType": "OperationOutcome",
                        "issue": [{
                            "severity": "error",
                            "code": "not-supported",
                            "diagnostics": format!(
                                "unsupported body parameter '{name}' for {op}"
                            )
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
fn merge_export_inputs(query: &ExportQueryParams, body: Option<&Value>) -> ExportInputs {
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
    let body_patient = collect_body_refs(body_params, "patient");
    let body_group = collect_body_refs(body_params, "group");

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
        .and_then(|s| s.parse().ok())
        .or_else(|| body_since.and_then(|s| s.parse().ok()));
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

    ExportInputs {
        format,
        header,
        since,
        patient,
        group,
        client_tracking_id,
    }
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
/// `Reference.reference` string. Falls back to `valueString` for permissive
/// clients that send refs as bare strings.
fn collect_body_refs(params: Option<&Vec<Value>>, name: &str) -> Vec<String> {
    params
        .map(|arr| {
            arr.iter()
                .filter(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))
                .filter_map(|p| {
                    p.get("valueReference")
                        .and_then(|r| r.get("reference"))
                        .and_then(|v| v.as_str())
                        .or_else(|| p.get("valueString").and_then(|v| v.as_str()))
                        .map(|s| s.to_string())
                })
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Validates that every relative `Patient/{id}` and `Group/{id}` reference
/// in the inputs resolves to an existing resource. Returns 404 with an
/// OperationOutcome listing the missing references. Absolute / external
/// references are skipped (we can't reach them).
async fn validate_patient_group_refs<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    inputs: &ExportInputs,
) -> Result<Option<Response>, RestError>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let mut missing: Vec<String> = Vec::new();
    for reference in inputs.patient.iter().chain(inputs.group.iter()) {
        let (resource_type, id) = match parse_relative_compartment_ref(reference) {
            Some(r) => r,
            None => continue, // absolute / unparseable — skip
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

/// Returns `(resource_type, id)` for relative refs of the form
/// `Patient/{id}` or `Group/{id}`. Returns `None` for absolute URLs or
/// any other shape.
fn parse_relative_compartment_ref(reference: &str) -> Option<(&'static str, &str)> {
    let trimmed = reference.trim();
    for &t in ["Patient", "Group"].iter() {
        let prefix = format!("{t}/");
        if let Some(rest) = trimmed.strip_prefix(&prefix) {
            let id = rest.split('/').next()?;
            if id.is_empty() {
                return None;
            }
            return Some((t, id));
        }
    }
    None
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
