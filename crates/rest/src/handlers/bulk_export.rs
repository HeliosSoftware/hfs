//! FHIR Bulk Data Export (`$export`) handlers.
//!
//! Implements the asynchronous kick-off → poll → manifest → download → delete
//! flow from the [Bulk Data Access IG](https://build.fhir.org/ig/HL7/bulk-data/).

use std::time::Duration;

use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::{HeaderMap, Method, StatusCode},
    response::Response,
};
use chrono::Utc;
use helios_auth::Principal;
use helios_fhir::FhirVersion;
use helios_persistence::core::ExportDataProvider;
use helios_persistence::core::{
    ExportJobId, ExportLevel, ExportManifest, ExportOutputFile, ExportRequest, ExportStatus,
    GroupExportProvider, PatientExportProvider, ResourceStorage, StartExportInput, TypeFilter,
};
use helios_persistence::error::{BulkExportError, StorageError};

use crate::error::{RestError, RestResult};
use crate::extractors::{FhirVersionExtractor, TenantExtractor};
use crate::state::AppState;

/// Trait bound shared by all bulk-export handlers (the resource-store side).
pub trait ExportResourceStore:
    ResourceStorage + ExportDataProvider + PatientExportProvider + GroupExportProvider
{
}
impl<S> ExportResourceStore for S where
    S: ResourceStorage + ExportDataProvider + PatientExportProvider + GroupExportProvider
{
}

/// Search-result-control params that are NOT valid inside `_typeFilter`.
const FORBIDDEN_FILTER_PARAMS: &[&str] =
    &["_include", "_revinclude", "_sort", "_count", "_elements"];

fn not_implemented() -> RestError {
    RestError::NotImplemented {
        feature: "Bulk Data Export is disabled (HFS_BULK_EXPORT_ENABLED=false)".to_string(),
    }
}

fn bad_request(msg: impl Into<String>) -> RestError {
    RestError::BadRequest {
        message: msg.into(),
    }
}

use crate::extractors::query_pairs::{collect_multi, first_value, parse_query_pairs};

/// Parses a FHIR `instant` into a UTC datetime.
fn parse_instant(s: &str) -> Result<chrono::DateTime<Utc>, RestError> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| bad_request(format!("invalid instant '{s}': {e}")))
}

/// Reads the `Prefer: handling=` directive (`strict` / `lenient`).
fn prefer_handling(headers: &HeaderMap) -> Option<String> {
    headers
        .get("prefer")
        .and_then(|v| v.to_str().ok())
        .and_then(|p| {
            p.split(',')
                .map(|s| s.trim())
                .find_map(|s| s.strip_prefix("handling="))
                .map(|s| s.to_ascii_lowercase())
        })
}

/// Returns true if `Prefer: respond-async` is present.
fn has_respond_async(headers: &HeaderMap) -> bool {
    headers
        .get("prefer")
        .and_then(|v| v.to_str().ok())
        .map(|p| {
            p.split(',')
                .any(|s| s.trim().eq_ignore_ascii_case("respond-async"))
        })
        .unwrap_or(false)
}

/// Builds the parameter pairs from a POST `Parameters` resource body.
fn pairs_from_parameters(body: &serde_json::Value) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    if let Some(arr) = body.get("parameter").and_then(|p| p.as_array()) {
        for p in arr {
            let Some(name) = p.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            // Accept valueString / valueUri / valueInstant / valueCode etc.
            let value = p
                .get("valueString")
                .or_else(|| p.get("valueUri"))
                .or_else(|| p.get("valueInstant"))
                .or_else(|| p.get("valueCode"))
                .or_else(|| p.get("valueDateTime"))
                .and_then(|v| v.as_str())
                .or_else(|| {
                    // patient reference: { name: "patient", valueReference: { reference } }
                    p.get("valueReference")
                        .and_then(|r| r.get("reference"))
                        .and_then(|r| r.as_str())
                });
            if let Some(v) = value {
                pairs.push((name.to_string(), v.to_string()));
            }
        }
    }
    pairs
}

/// Shared kick-off logic for all three export levels.
#[allow(clippy::too_many_arguments)]
async fn kickoff_export<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    principal: Option<&Principal>,
    level: ExportLevel,
    fhir_version: FhirVersion,
    method: &Method,
    headers: &HeaderMap,
    raw_query: Option<&str>,
    full_url: &str,
    body: Option<&serde_json::Value>,
) -> RestResult<Response>
where
    S: ExportResourceStore + Send + Sync,
{
    let cfg = state.bulk_export_config();
    if !cfg.enabled {
        return Err(not_implemented());
    }
    let jobs = state.bulk_export_jobs().ok_or_else(not_implemented)?;

    if !has_respond_async(headers) {
        return Err(bad_request(
            "the `Prefer: respond-async` header is required for $export",
        ));
    }

    let is_post = method == Method::POST;
    let mut pairs = parse_query_pairs(raw_query);
    if is_post {
        if let Some(b) = body {
            pairs.extend(pairs_from_parameters(b));
        }
    }

    // _outputFormat
    let output_format = first_value(&pairs, "_outputFormat")
        .unwrap_or_else(|| "application/fhir+ndjson".to_string());
    if !matches!(
        output_format.as_str(),
        "application/fhir+ndjson" | "application/ndjson" | "ndjson"
    ) {
        return Err(bad_request(format!(
            "unsupported _outputFormat '{output_format}'"
        )));
    }

    // _type
    let resource_types = collect_multi(&pairs, "_type");

    // _since / _until
    let since = match first_value(&pairs, "_since") {
        Some(s) => Some(parse_instant(&s)?),
        None => None,
    };
    let until = match first_value(&pairs, "_until") {
        Some(s) => Some(parse_instant(&s)?),
        None => None,
    };

    // _elements
    let elements = collect_multi(&pairs, "_elements");

    // _typeFilter
    let mut type_filters = Vec::new();
    for tf in pairs.iter().filter(|(k, _)| k == "_typeFilter") {
        let raw = &tf.1;
        let (rt, query) = raw
            .split_once('?')
            .ok_or_else(|| bad_request(format!("malformed _typeFilter '{raw}'")))?;
        if !resource_types.is_empty() && !resource_types.iter().any(|t| t == rt) {
            return Err(bad_request(format!(
                "_typeFilter resource type '{rt}' is not in _type"
            )));
        }
        for (pk, _) in url::form_urlencoded::parse(query.as_bytes()) {
            if FORBIDDEN_FILTER_PARAMS.contains(&pk.as_ref()) {
                return Err(bad_request(format!(
                    "_typeFilter may not contain result-control param '{pk}'"
                )));
            }
        }
        type_filters.push(TypeFilter::new(rt, query));
    }

    // patient (POST only)
    let patient_refs = collect_multi(&pairs, "patient");
    if !patient_refs.is_empty() {
        if matches!(level, ExportLevel::System) {
            return Err(bad_request(
                "the `patient` parameter is not valid for system-level export",
            ));
        }
        // Validate each patient reference resolves.
        for pref in &patient_refs {
            let id = pref.strip_prefix("Patient/").unwrap_or(pref);
            let exists = state
                .storage()
                .read(tenant.context(), "Patient", id)
                .await
                .map_err(map_storage_err)?
                .is_some();
            if !exists {
                return Err(bad_request(format!("unknown patient reference '{pref}'")));
            }
        }
        // For group-level, each must be a member of the group.
        if let ExportLevel::Group { group_id } = &level {
            let members = state
                .storage()
                .resolve_group_patient_ids(tenant.context(), group_id)
                .await
                .map_err(map_storage_err)?;
            for pref in &patient_refs {
                let id = pref.strip_prefix("Patient/").unwrap_or(pref);
                if !members.iter().any(|m| m == id) {
                    return Err(bad_request(format!(
                        "patient '{pref}' is not a member of Group/{group_id}"
                    )));
                }
            }
        }
    }

    // Unsupported parameters — strict vs lenient.
    let handling = prefer_handling(headers);
    let unsupported: Vec<&str> = [
        "includeAssociatedData",
        "organizeOutputBy",
        "allowPartialManifests",
    ]
    .into_iter()
    .filter(|p| pairs.iter().any(|(k, _)| k == p))
    .collect();
    if !unsupported.is_empty() {
        if handling.as_deref() == Some("strict") {
            return Err(bad_request(format!(
                "unsupported parameters: {}",
                unsupported.join(", ")
            )));
        } else {
            tracing::warn!(
                "ignoring unsupported bulk-export parameters: {}",
                unsupported.join(", ")
            );
        }
    }

    // Authorization — every requested type needs read scope; Group also needs Group read.
    if let Some(p) = principal {
        let types_to_check = if resource_types.is_empty() {
            // Whole-scope export — require a wildcard read or accept (best effort).
            vec![]
        } else {
            resource_types.clone()
        };
        for t in &types_to_check {
            helios_auth::SmartScopePolicy::check(p, t, helios_auth::FhirOperation::Read).map_err(
                |e| RestError::Forbidden {
                    message: e.to_string(),
                },
            )?;
        }
        if matches!(level, ExportLevel::Group { .. }) {
            helios_auth::SmartScopePolicy::check(p, "Group", helios_auth::FhirOperation::Read)
                .map_err(|e| RestError::Forbidden {
                    message: e.to_string(),
                })?;
        }
    }

    // Per-tenant concurrency cap.
    let active = jobs
        .count_active_exports(tenant.context())
        .await
        .map_err(map_storage_err)?;
    if active >= cfg.max_concurrent_per_tenant as u64 {
        return Err(RestError::BadRequest {
            message: format!(
                "too many concurrent exports for this tenant (max {})",
                cfg.max_concurrent_per_tenant
            ),
        });
    }

    let request = ExportRequest {
        level: level.clone(),
        resource_types,
        since,
        until,
        type_filters,
        elements,
        include_associated_data: Vec::new(),
        patient_refs,
        batch_size: cfg.batch_size,
        output_format,
    };

    let input = StartExportInput {
        request,
        transaction_time: Utc::now(),
        request_url: full_url.to_string(),
        owner_subject: principal.map(|p| p.subject.clone()),
        fhir_version,
    };

    let request_clone = input.request.clone();
    let job_id = jobs
        .start_export(tenant.context(), input)
        .await
        .map_err(map_storage_err)?;

    emit_export_audit(
        state,
        principal,
        "kickoff",
        job_id.as_str(),
        &request_clone.level,
        &request_clone.resource_types,
        "0",
    )
    .await;

    let status_url = format!(
        "{}/export-status/{}",
        state.base_url().trim_end_matches('/'),
        job_id
    );

    Response::builder()
        .status(StatusCode::ACCEPTED)
        .header("Content-Location", status_url)
        .body(Body::empty())
        .map_err(|e| RestError::InternalError {
            message: e.to_string(),
        })
}

/// Maps a persistence error to a REST error.
fn map_storage_err(e: StorageError) -> RestError {
    match e {
        StorageError::BulkExport(BulkExportError::JobNotFound { job_id }) => RestError::NotFound {
            resource_type: "export-job".to_string(),
            id: job_id,
        },
        StorageError::Backend(helios_persistence::error::BackendError::UnsupportedCapability {
            ..
        }) => RestError::NotImplemented {
            feature: "bulk export not supported by this backend".to_string(),
        },
        other => RestError::InternalError {
            message: other.to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

/// `GET|POST /$export` — system-level kick-off.
pub async fn system_export_kickoff_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ExportResourceStore + Send + Sync + 'static,
{
    run_kickoff(state, tenant, version, ExportLevel::System, request).await
}

/// `GET|POST /Patient/$export` — patient-level kick-off.
pub async fn patient_export_kickoff_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ExportResourceStore + Send + Sync + 'static,
{
    run_kickoff(state, tenant, version, ExportLevel::Patient, request).await
}

/// `GET|POST /Group/{id}/$export` — group-level kick-off.
pub async fn group_export_kickoff_handler<S>(
    State(state): State<AppState<S>>,
    Path(group_id): Path<String>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ExportResourceStore + Send + Sync + 'static,
{
    run_kickoff(
        state,
        tenant,
        version,
        ExportLevel::Group { group_id },
        request,
    )
    .await
}

/// Shared body of the three kick-off wrappers.
async fn run_kickoff<S>(
    state: AppState<S>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    level: ExportLevel,
    request: Request,
) -> RestResult<Response>
where
    S: ExportResourceStore + Send + Sync + 'static,
{
    let method = request.method().clone();
    let headers = request.headers().clone();
    let uri = request.uri().clone();
    let raw_query = uri.query().map(|q| q.to_string());
    let full_url = format!(
        "{}{}",
        state.base_url().trim_end_matches('/'),
        uri.path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or(uri.path())
    );
    let principal = request.extensions().get::<Principal>().cloned();

    let body_json: Option<serde_json::Value> = if method == Method::POST {
        let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
            .await
            .map_err(|e| bad_request(format!("failed to read request body: {e}")))?;
        if bytes.is_empty() {
            None
        } else {
            Some(
                serde_json::from_slice(&bytes)
                    .map_err(|e| bad_request(format!("invalid Parameters JSON: {e}")))?,
            )
        }
    } else {
        None
    };

    kickoff_export(
        &state,
        &tenant,
        principal.as_ref(),
        level,
        version.storage_version(),
        &method,
        &headers,
        raw_query.as_deref(),
        &full_url,
        body_json.as_ref(),
    )
    .await
}

/// `GET /export-status/{job_id}` — poll status / fetch manifest.
pub async fn export_status_handler<S>(
    State(state): State<AppState<S>>,
    Path(job_id): Path<String>,
    tenant: TenantExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ExportResourceStore + Send + Sync + 'static,
{
    let cfg = state.bulk_export_config();
    if !cfg.enabled {
        return Err(not_implemented());
    }
    let jobs = state.bulk_export_jobs().ok_or_else(not_implemented)?;
    let output = state.bulk_export_output().ok_or_else(not_implemented)?;
    let principal = request.extensions().get::<Principal>().cloned();
    let job_id = ExportJobId::from_string(job_id);

    // Ownership check first (do not leak existence).
    let meta = match jobs
        .get_export_job_metadata(tenant.context(), &job_id)
        .await
    {
        Ok(m) => m,
        Err(_) => {
            return Err(RestError::NotFound {
                resource_type: "export-job".to_string(),
                id: job_id.to_string(),
            });
        }
    };
    if !owns_job(principal.as_ref(), meta.owner_subject.as_deref()) {
        return Err(RestError::NotFound {
            resource_type: "export-job".to_string(),
            id: job_id.to_string(),
        });
    }

    match meta.status {
        ExportStatus::Accepted | ExportStatus::InProgress => {
            let progress = jobs
                .get_export_status(tenant.context(), &job_id)
                .await
                .map_err(map_storage_err)?;
            let x_progress = progress
                .current_type
                .clone()
                .unwrap_or_else(|| format!("{:.0}%", progress.overall_progress() * 100.0));
            Response::builder()
                .status(StatusCode::ACCEPTED)
                .header("X-Progress", x_progress)
                .header("Retry-After", "120")
                .body(Body::empty())
                .map_err(|e| RestError::InternalError {
                    message: e.to_string(),
                })
        }
        ExportStatus::Complete => {
            let raw = jobs
                .get_export_manifest(tenant.context(), &job_id)
                .await
                .map_err(map_storage_err)?;
            let ttl = Duration::from_secs(cfg.file_url_ttl_secs);
            let mut output_files = Vec::new();
            let mut error_files = Vec::new();
            let mut requires_token = true;
            for entry in &raw.output {
                let url = output
                    .download_url(&entry.key, ttl)
                    .await
                    .map_err(map_storage_err)?;
                requires_token = url.requires_access_token;
                output_files.push(ExportOutputFile {
                    resource_type: entry.resource_type.clone(),
                    url: url.url,
                    count: Some(entry.count),
                });
            }
            for entry in &raw.errors {
                let url = output
                    .download_url(&entry.key, ttl)
                    .await
                    .map_err(map_storage_err)?;
                error_files.push(ExportOutputFile {
                    resource_type: entry.resource_type.clone(),
                    url: url.url,
                    count: Some(entry.count),
                });
            }
            let manifest = ExportManifest {
                transaction_time: raw.transaction_time,
                request: raw.request_url,
                requires_access_token: requires_token,
                output: output_files,
                error: error_files,
                deleted: Vec::new(),
                link: Vec::new(),
                message: None,
                extension: None,
            };
            let body = serde_json::to_vec(&manifest).map_err(|e| RestError::InternalError {
                message: e.to_string(),
            })?;
            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Body::from(body))
                .map_err(|e| RestError::InternalError {
                    message: e.to_string(),
                })
        }
        ExportStatus::Error => Err(RestError::InternalError {
            message: "export job failed".to_string(),
        }),
        ExportStatus::Cancelled => Err(RestError::NotFound {
            resource_type: "export-job".to_string(),
            id: job_id.to_string(),
        }),
    }
}

/// `DELETE /export-status/{job_id}` — cancel + delete a job.
pub async fn export_cancel_handler<S>(
    State(state): State<AppState<S>>,
    Path(job_id): Path<String>,
    tenant: TenantExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ExportResourceStore + Send + Sync + 'static,
{
    let cfg = state.bulk_export_config();
    if !cfg.enabled {
        return Err(not_implemented());
    }
    let jobs = state.bulk_export_jobs().ok_or_else(not_implemented)?;
    let output = state.bulk_export_output().ok_or_else(not_implemented)?;
    let principal = request.extensions().get::<Principal>().cloned();
    let job_id = ExportJobId::from_string(job_id);

    let meta = match jobs
        .get_export_job_metadata(tenant.context(), &job_id)
        .await
    {
        Ok(m) => m,
        Err(_) => {
            return Err(RestError::NotFound {
                resource_type: "export-job".to_string(),
                id: job_id.to_string(),
            });
        }
    };
    if !owns_job(principal.as_ref(), meta.owner_subject.as_deref()) {
        return Err(RestError::NotFound {
            resource_type: "export-job".to_string(),
            id: job_id.to_string(),
        });
    }

    // Cancel if still active (cooperative — worker observes it).
    if meta.status.is_active() {
        let _ = jobs.cancel_export(tenant.context(), &job_id).await;
    }
    // REST owns the two-step teardown: outputs first, then job rows.
    output
        .delete_job_outputs(tenant.context(), &job_id)
        .await
        .map_err(map_storage_err)?;
    jobs.delete_export(tenant.context(), &job_id)
        .await
        .map_err(map_storage_err)?;

    emit_export_audit(
        &state,
        principal.as_ref(),
        "delete",
        job_id.as_str(),
        &meta.level,
        &[],
        "0",
    )
    .await;

    Response::builder()
        .status(StatusCode::ACCEPTED)
        .body(Body::empty())
        .map_err(|e| RestError::InternalError {
            message: e.to_string(),
        })
}

/// `GET /export-file/{job_id}/{part}` — HFS-served NDJSON download.
pub async fn export_download_handler<S>(
    State(state): State<AppState<S>>,
    Path((job_id, part)): Path<(String, String)>,
    tenant: TenantExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ExportResourceStore + Send + Sync + 'static,
{
    let cfg = state.bulk_export_config();
    if !cfg.enabled {
        return Err(not_implemented());
    }
    let jobs = state.bulk_export_jobs().ok_or_else(not_implemented)?;
    let output = state.bulk_export_output().ok_or_else(not_implemented)?;
    let file_auth = state.bulk_export_file_auth().ok_or_else(not_implemented)?;
    let principal = request.extensions().get::<Principal>().cloned();
    let job_id = ExportJobId::from_string(job_id);

    let file_meta = jobs
        .get_export_file_metadata(tenant.context(), &job_id, &part)
        .await
        .map_err(|_| RestError::NotFound {
            resource_type: "export-file".to_string(),
            id: format!("{job_id}/{part}"),
        })?;

    file_auth
        .authorize_download(
            principal.as_ref(),
            tenant.context(),
            file_meta.job_owner_subject.as_deref(),
            &file_meta,
        )
        .await
        .map_err(|e| RestError::Forbidden {
            message: e.to_string(),
        })?;

    emit_export_audit(
        &state,
        principal.as_ref(),
        "download",
        job_id.as_str(),
        &ExportLevel::System,
        std::slice::from_ref(&file_meta.resource_type),
        "0",
    )
    .await;

    let mut reader = output
        .open_reader(&file_meta.key)
        .await
        .map_err(map_storage_err)?;
    let mut bytes = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut bytes)
        .await
        .map_err(|e| RestError::InternalError {
            message: format!("failed to read export file: {e}"),
        })?;

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/fhir+ndjson")
        .body(Body::from(bytes))
        .map_err(|e| RestError::InternalError {
            message: e.to_string(),
        })
}

/// Emits a bulk-export lifecycle `AuditEvent` when an audit sink is configured.
async fn emit_export_audit<S>(
    state: &AppState<S>,
    principal: Option<&Principal>,
    operation: &str,
    job_id: &str,
    level: &ExportLevel,
    resource_types: &[String],
    outcome: &str,
) where
    S: ResourceStorage,
{
    let Some(sink) = state.audit_sink() else {
        return;
    };
    let mut builder = helios_audit::AuditEventBuilder::new(state.audit_source_observer())
        .event_type(
            "http://terminology.hl7.org/CodeSystem/audit-event-type",
            "object",
        )
        .action(helios_audit::AuditAction::Execute)
        .outcome(outcome)
        .detail("audit-operation", "bulk-export")
        .detail("bulk-export-operation", operation)
        .detail("job-id", job_id)
        .detail("export-level", level.to_string());
    if !resource_types.is_empty() {
        builder = builder.detail("resource-types", resource_types.join(","));
    }
    if let Some(p) = principal {
        builder = builder.agent(&p.subject, None, true);
    }
    sink.record(builder.build()).await;
}

/// Ownership check: the principal owns the job, holds a `system/*` scope, or
/// auth is disabled (no principal).
fn owns_job(principal: Option<&Principal>, owner_subject: Option<&str>) -> bool {
    match principal {
        None => true, // auth disabled — no ownership enforcement
        Some(p) => {
            owner_subject == Some(p.subject.as_str())
                || p.scopes
                    .scopes()
                    .iter()
                    .any(|s| s.resource_type == helios_auth::scope::ResourceTypeSpec::Wildcard)
        }
    }
}
