//! FHIR Bulk Data Export (`$export`) handlers.
//!
//! Implements the asynchronous kick-off → poll → manifest → download → delete
//! flow from the [Bulk Data Access IG](https://build.fhir.org/ig/HL7/bulk-data/).

use std::{pin::Pin, time::Duration};

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
    DownloadUrl, ExportJobId, ExportLevel, ExportManifest, ExportOutputFile, ExportOutputStore,
    ExportRequest, ExportStatus, GroupExportProvider, PatientExportProvider, RawManifestEntry,
    ResourceStorage, StartExportInput, TypeFilter,
};
use helios_persistence::error::{BulkExportError, StorageError};
use tokio::io::AsyncRead;
use tokio_util::io::ReaderStream;

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

fn external_download_url(download: &DownloadUrl) -> Option<String> {
    (!download.requires_access_token).then(|| download.url.clone())
}

fn advertised_download_url(download: &DownloadUrl, fallback: impl FnOnce() -> String) -> String {
    external_download_url(download).unwrap_or_else(fallback)
}

async fn resolve_manifest_files<S>(
    entries: &[RawManifestEntry],
    output: &dyn ExportOutputStore,
    ttl: Duration,
    state: &AppState<S>,
    tenant: &TenantExtractor,
    job_id: &ExportJobId,
    requires_token: &mut Option<bool>,
) -> RestResult<Vec<ExportOutputFile>>
where
    S: ResourceStorage,
{
    let mut files = Vec::with_capacity(entries.len());
    for entry in entries {
        let download = output
            .download_url(&entry.key, ttl)
            .await
            .map_err(map_storage_err)?;
        *requires_token = Some(requires_token.unwrap_or(false) || download.requires_access_token);
        let url = advertised_download_url(&download, || {
            let part = entry.key.part_segment();
            state.public_url_for_request(tenant, ["export-file", job_id.as_str(), part.as_str()])
        });
        files.push(ExportOutputFile {
            resource_type: entry.resource_type.clone(),
            url,
            count: Some(entry.count),
        });
    }
    Ok(files)
}

// Shared `Prefer` / `Parameters` parsing helpers live in `bulk_common`.
use super::bulk_common::{
    collect_multi, first_value, has_respond_async, pairs_from_parameters, parse_instant,
    parse_query_pairs, prefer_handling,
};

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

    let status_url = state.public_url_for_request(tenant, ["export-status", job_id.as_str()]);

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
    let endpoint_segments: Vec<&str> = match &level {
        ExportLevel::System => vec!["$export"],
        ExportLevel::Patient => vec!["Patient", "$export"],
        ExportLevel::Group { group_id } => vec!["Group", group_id, "$export"],
    };
    let full_url = state.public_url_for_request_with_query(&tenant, endpoint_segments, uri.query());
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
        version.storage_version_or(state.config().default_fhir_version),
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
            let mut requires_token = None;
            let output_files = resolve_manifest_files(
                &raw.output,
                output.as_ref(),
                ttl,
                &state,
                &tenant,
                &job_id,
                &mut requires_token,
            )
            .await?;
            let error_files = resolve_manifest_files(
                &raw.errors,
                output.as_ref(),
                ttl,
                &state,
                &tenant,
                &job_id,
                &mut requires_token,
            )
            .await?;
            let manifest = ExportManifest {
                transaction_time: raw.transaction_time,
                request: raw.request_url,
                requires_access_token: requires_token.unwrap_or(true),
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

    let reader = output
        .open_reader(&file_meta.key)
        .await
        .map_err(map_storage_err)?;

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/fhir+ndjson")
        // The application request timeout covers the handler future, not a
        // returned response body. Reader errors therefore propagate through
        // the body stream without forcing a whole-file buffer here.
        .body(streamed_reader_body(reader))
        .map_err(|e| RestError::InternalError {
            message: e.to_string(),
        })
}

fn streamed_reader_body(reader: Pin<Box<dyn AsyncRead + Send>>) -> Body {
    Body::from_stream(ReaderStream::new(reader))
}

/// Emits a bulk-export lifecycle `AuditEvent` when an audit sink is configured.
///
/// Delegates to the shared emit helper in `helios-persistence`, which the export
/// *worker* also calls for the terminal complete/cancelled/failed events. This
/// used to be a hand-rolled duplicate of that helper — REST could not reach it,
/// because `helios-persistence` gated its audit code behind a cargo feature that
/// only the `hfs` binary enabled. Two implementations of one event drift; now
/// there is one.
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
    helios_persistence::core::bulk_export::audit::record_export_event(
        sink.as_ref(),
        state.audit_source_observer(),
        principal.map(|p| p.subject.as_str()),
        job_id,
        operation,
        level,
        resource_types,
        outcome,
        None,
    )
    .await;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn streamed_export_body_propagates_reader_errors() {
        let reader = tokio_test::io::Builder::new()
            .read(b"first chunk\n")
            .read_error(std::io::Error::other("forced reader failure"))
            .build();
        let result = axum::body::to_bytes(streamed_reader_body(Box::pin(reader)), usize::MAX).await;
        assert!(result.is_err(), "reader failure must remain a body error");
    }

    #[test]
    fn presigned_download_url_is_preserved_byte_for_byte() {
        let url = "https://s3.example/object?X-Amz-Signature=a%2Fb&x=1";
        let download = DownloadUrl {
            url: url.to_string(),
            requires_access_token: false,
        };
        assert_eq!(external_download_url(&download).as_deref(), Some(url));
        assert_eq!(
            advertised_download_url(&download, || "https://fallback.example".to_string()),
            url
        );

        let protected = DownloadUrl {
            url: "internal://artifact".to_string(),
            requires_access_token: true,
        };
        assert_eq!(
            advertised_download_url(&protected, || "https://public.example/artifact".to_string()),
            "https://public.example/artifact"
        );
    }
}
