//! FHIR Bulk Data Submit (`$bulk-submit`) handlers.
//!
//! HFS is the **Data Consumer**: `POST /$bulk-submit` records a submission and
//! enqueues its manifest for asynchronous fetch + ingest; `POST /$bulk-submit-status`
//! returns a `Content-Location` the provider polls; `GET`/`DELETE` on that location
//! poll / cancel; and `GET /bulk-submit-file/{token}/{part}` serves HFS-hosted
//! status-manifest artifacts.
//!
//! See `submit.html` (Argo25 branch of the Bulk Data IG).

use std::time::Duration;

use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::{Method, StatusCode},
    response::Response,
};
use helios_auth::Principal;
use helios_persistence::core::{
    ExportPartKey, ManifestStatus, ResourceStorage, SubmissionId, SubmissionStatus,
    submission_output_job_id,
};
use serde_json::{Value, json};

use crate::error::{RestError, RestResult};
use crate::extractors::TenantExtractor;
use crate::state::AppState;

/// The SMART operation scope authorizing every `$bulk-submit` surface.
const SUBMIT_SCOPE: &str = "bulk-submit";

/// The code system for the `submissionStatus` Coding (per the IG).
const SUBMISSION_STATUS_SYSTEM: &str = "http://hl7.org/fhir/event-status";

fn not_implemented() -> RestError {
    RestError::NotImplemented {
        feature: "Bulk Data Submit is disabled or unsupported by this backend".to_string(),
    }
}

fn bad_request(msg: impl Into<String>) -> RestError {
    RestError::BadRequest {
        message: msg.into(),
    }
}

/// Enforces the `system/bulk-submit` operation scope when auth is enabled.
fn check_submit_scope(principal: Option<&Principal>) -> RestResult<()> {
    if let Some(p) = principal {
        if !p.scopes.grants_operation(SUBMIT_SCOPE) {
            return Err(RestError::Forbidden {
                message: "the `system/bulk-submit` scope is required".to_string(),
            });
        }
    }
    Ok(())
}

/// Ownership check for status / cancel / file surfaces: the principal must own the
/// submission (subject match) or hold a system wildcard scope. Auth-disabled → allowed.
fn owns_submission(principal: Option<&Principal>, owner_subject: Option<&str>) -> bool {
    match principal {
        None => true,
        Some(p) => owner_subject == Some(p.subject.as_str()) || p.scopes.has_system_wildcard(),
    }
}

// ---------------------------------------------------------------------------
// Parameters parsing
// ---------------------------------------------------------------------------

/// The parsed `$bulk-submit` kickoff request.
#[derive(Debug, Default)]
struct SubmitRequest {
    submitter_system: Option<String>,
    submitter_value: String,
    submission_id: String,
    submission_status: String,
    manifest_url: Option<String>,
    replaces_manifest_url: Option<String>,
    output_format: Option<String>,
    fhir_base_url: Option<String>,
    file_request_headers: Vec<(String, String)>,
    oauth_metadata_urls: Vec<String>,
    file_encryption_key: Option<Value>,
    import_directives: Vec<(String, String)>,
    metadata: Vec<(String, String)>,
}

/// Reads a scalar `value[x]` string from a `Parameters.parameter` object.
fn scalar(param: &Value) -> Option<String> {
    param
        .get("valueString")
        .or_else(|| param.get("valueUrl"))
        .or_else(|| param.get("valueUri"))
        .or_else(|| param.get("valueCode"))
        .and_then(|v| v.as_str())
        .map(String::from)
}

/// Reads a named child `part` scalar value from a parameter's `part` array.
fn part_scalar(param: &Value, child: &str) -> Option<String> {
    param
        .get("part")
        .and_then(|p| p.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|c| c.get("name").and_then(|n| n.as_str()) == Some(child))
        })
        .and_then(scalar)
}

/// Parses a `$bulk-submit` `Parameters` body, enforcing the spec's validation SHALLs.
fn parse_submit_request(body: &Value) -> RestResult<SubmitRequest> {
    let params = body
        .get("parameter")
        .and_then(|p| p.as_array())
        .ok_or_else(|| bad_request("request body must be a Parameters resource"))?;

    let mut req = SubmitRequest {
        submission_status: "in-progress".to_string(),
        ..Default::default()
    };
    let mut saw_submitter = false;
    let mut saw_submission_id = false;

    for p in params {
        let Some(name) = p.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        match name {
            "submitter" => {
                let id = p
                    .get("valueIdentifier")
                    .ok_or_else(|| bad_request("submitter must be a valueIdentifier"))?;
                req.submitter_system = id.get("system").and_then(|v| v.as_str()).map(String::from);
                req.submitter_value = id
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| bad_request("submitter.value is required"))?
                    .to_string();
                saw_submitter = true;
            }
            "submissionId" => {
                req.submission_id =
                    scalar(p).ok_or_else(|| bad_request("submissionId must be a string"))?;
                saw_submission_id = true;
            }
            "submissionStatus" => {
                // When a Coding is supplied, its system (if present) SHALL be the
                // Bulk Data submission-status system.
                if let Some(coding) = p.get("valueCoding") {
                    if let Some(system) = coding.get("system").and_then(|v| v.as_str()) {
                        if system != SUBMISSION_STATUS_SYSTEM {
                            return Err(bad_request(format!(
                                "submissionStatus.system must be '{SUBMISSION_STATUS_SYSTEM}', got '{system}'"
                            )));
                        }
                    }
                }
                let code = p
                    .get("valueCoding")
                    .and_then(|c| c.get("code"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .or_else(|| scalar(p));
                if let Some(code) = code {
                    req.submission_status = code;
                }
            }
            "manifestUrl" => req.manifest_url = scalar(p),
            "replacesManifestUrl" => req.replaces_manifest_url = scalar(p),
            "outputFormat" => req.output_format = scalar(p),
            "fhirBaseUrl" => req.fhir_base_url = scalar(p),
            "oauthMetadataUrl" => {
                if let Some(u) = scalar(p) {
                    req.oauth_metadata_urls.push(u);
                }
            }
            "fileRequestHeader" => {
                if let (Some(h), Some(v)) =
                    (part_scalar(p, "headerName"), part_scalar(p, "headerValue"))
                {
                    req.file_request_headers.push((h, v));
                }
            }
            "fileEncryptionKey" => {
                req.file_encryption_key = Some(p.clone());
            }
            "import" => {
                if let (Some(u), Some(v)) = (
                    part_scalar(p, "parameterUrl"),
                    part_scalar(p, "parameterValue"),
                ) {
                    req.import_directives.push((u, v));
                }
            }
            "metadata" => {
                if let (Some(u), Some(v)) = (
                    part_scalar(p, "parameterUrl"),
                    part_scalar(p, "parameterValue"),
                ) {
                    req.metadata.push((u, v));
                }
            }
            _ => {}
        }
    }

    // Validation SHALLs (submit.html).
    if !saw_submitter || req.submitter_value.is_empty() {
        return Err(bad_request("the `submitter` parameter is required"));
    }
    if !saw_submission_id || req.submission_id.is_empty() {
        return Err(bad_request("the `submissionId` parameter is required"));
    }
    let has_status_param = body
        .get("parameter")
        .and_then(|p| p.as_array())
        .map(|a| {
            a.iter()
                .any(|p| p.get("name").and_then(|n| n.as_str()) == Some("submissionStatus"))
        })
        .unwrap_or(false);
    if !has_status_param && req.manifest_url.is_none() {
        return Err(bad_request(
            "at least one of `submissionStatus` and `manifestUrl` must be populated",
        ));
    }
    if req.manifest_url.is_some() && req.fhir_base_url.is_none() {
        return Err(bad_request(
            "`fhirBaseUrl` is required when `manifestUrl` is populated",
        ));
    }
    if !matches!(
        req.submission_status.as_str(),
        "in-progress" | "completed" | "stopped"
    ) {
        return Err(bad_request(format!(
            "invalid submissionStatus '{}' (expected in-progress|completed|stopped)",
            req.submission_status
        )));
    }
    Ok(req)
}

impl SubmitRequest {
    /// The internal submission key: `system|value`.
    fn submitter_key(&self) -> String {
        format!(
            "{}|{}",
            self.submitter_system.as_deref().unwrap_or(""),
            self.submitter_value
        )
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /$bulk-submit` — synchronous accept (200), enqueues async ingestion.
pub async fn bulk_submit_kickoff_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let cfg = state.bulk_submit_config();
    if !cfg.enabled {
        return Err(not_implemented());
    }
    let jobs = state
        .bulk_submit_jobs()
        .ok_or_else(not_implemented)?
        .clone();
    let principal = request.extensions().get::<Principal>().cloned();
    check_submit_scope(principal.as_ref())?;

    if request.method() != Method::POST {
        return Err(RestError::MethodNotAllowed {
            method: request.method().to_string(),
            resource_type: "$bulk-submit".to_string(),
        });
    }

    let request_url = format!("{}/$bulk-submit", state.base_url().trim_end_matches('/'));
    // Capture `Prefer: handling=` before consuming the body.
    let strict = request
        .headers()
        .get("prefer")
        .and_then(|v| v.to_str().ok())
        .map(|p| {
            p.split(',')
                .any(|s| s.trim().eq_ignore_ascii_case("handling=strict"))
        })
        .unwrap_or(false);
    let bytes = axum::body::to_bytes(request.into_body(), 4 * 1024 * 1024)
        .await
        .map_err(|e| bad_request(format!("failed to read request body: {e}")))?;
    let body: Value = serde_json::from_slice(&bytes)
        .map_err(|e| bad_request(format!("invalid Parameters JSON: {e}")))?;
    let req = parse_submit_request(&body)?;

    // Under `Prefer: handling=strict`, reject pre-coordinated directives we do not
    // recognize (we recognize none specifically); lenient/absent → ignore them.
    if strict && (!req.import_directives.is_empty() || !req.metadata.is_empty()) {
        return Err(bad_request(
            "unrecognized `import`/`metadata` directives rejected under Prefer: handling=strict",
        ));
    }

    let sub_id = SubmissionId::new(req.submitter_key(), req.submission_id.clone());
    let ctx = tenant.context();

    // Reject further submissions for a terminal submitter+submissionId.
    if let Some(existing) = jobs
        .get_submission(ctx, &sub_id)
        .await
        .map_err(RestError::from)?
    {
        if existing.status.is_terminal() {
            return Err(RestError::Conflict {
                message: format!(
                    "submission {} is already {} — no further submissions allowed",
                    sub_id, existing.status
                ),
            });
        }
    } else {
        // Per-tenant concurrency cap.
        let active = jobs
            .count_active_submissions(ctx)
            .await
            .map_err(RestError::from)?;
        if active >= cfg.max_concurrent_per_tenant as u64 {
            return Err(RestError::TooManyRequests {
                message: format!(
                    "too many concurrent submissions for this tenant (max {})",
                    cfg.max_concurrent_per_tenant
                ),
            });
        }
        let requires_token = cfg.requires_access_token != "false";
        jobs.create_submission(ctx, &sub_id, None)
            .await
            .map_err(RestError::from)?;
        jobs.set_submission_kickoff_meta(
            ctx,
            &sub_id,
            principal.as_ref().map(|p| p.subject.as_str()),
            &request_url,
            requires_token,
        )
        .await
        .map_err(RestError::from)?;
    }

    // replacesManifestUrl: supersede prior manifests with this URL (+ roll back).
    if let Some(replaces) = &req.replaces_manifest_url {
        let replaced = jobs
            .replace_manifest_by_url(ctx, &sub_id, replaces)
            .await
            .map_err(RestError::from)?;
        if !replaced.is_empty() {
            tracing::info!(
                submission = %sub_id,
                count = replaced.len(),
                "marked manifests replaced via replacesManifestUrl"
            );
        }
    }

    // Add the new manifest (if any) and persist its fetch parameters.
    if let Some(manifest_url) = &req.manifest_url {
        // manifestUrl SHALL be unique within submitter+submissionId.
        let dup = jobs
            .list_manifests(ctx, &sub_id)
            .await
            .map_err(RestError::from)?
            .into_iter()
            .any(|m| {
                m.manifest_url.as_deref() == Some(manifest_url.as_str())
                    && m.status != ManifestStatus::Replaced
            });
        if dup {
            return Err(RestError::Conflict {
                message: format!("manifestUrl '{manifest_url}' already submitted"),
            });
        }
        let manifest = jobs
            .add_manifest(
                ctx,
                &sub_id,
                Some(manifest_url),
                req.replaces_manifest_url.as_deref(),
            )
            .await
            .map_err(RestError::from)?;
        jobs.set_manifest_fetch_params(
            ctx,
            &sub_id,
            &manifest.manifest_id,
            req.fhir_base_url.as_deref(),
            req.output_format.as_deref(),
            &req.file_request_headers,
            &req.oauth_metadata_urls,
            req.file_encryption_key.as_ref(),
        )
        .await
        .map_err(RestError::from)?;
    }

    // submissionStatus=stopped → abort (rolls back recorded changes).
    if req.submission_status == "stopped" {
        jobs.abort_submission(ctx, &sub_id, "submissionStatus=stopped")
            .await
            .map_err(RestError::from)?;
    }

    let oo = json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": "information",
            "code": "informational",
            "details": { "text": format!("submission {sub_id} accepted") }
        }]
    });
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/fhir+json")
        .body(Body::from(serde_json::to_vec(&oo).unwrap_or_default()))
        .map_err(|e| RestError::InternalError {
            message: e.to_string(),
        })
}

/// `POST /$bulk-submit-status` — returns a `Content-Location` poll URL (202).
pub async fn bulk_submit_status_kickoff_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let cfg = state.bulk_submit_config();
    if !cfg.enabled {
        return Err(not_implemented());
    }
    let jobs = state
        .bulk_submit_jobs()
        .ok_or_else(not_implemented)?
        .clone();
    let principal = request.extensions().get::<Principal>().cloned();
    check_submit_scope(principal.as_ref())?;

    let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
        .await
        .map_err(|e| bad_request(format!("failed to read request body: {e}")))?;
    let body: Value = serde_json::from_slice(&bytes)
        .map_err(|e| bad_request(format!("invalid Parameters JSON: {e}")))?;
    let req = parse_status_request(&body)?;
    let sub_id = SubmissionId::new(req.0, req.1);
    let ctx = tenant.context();

    let summary = jobs
        .get_submission(ctx, &sub_id)
        .await
        .map_err(RestError::from)?
        .ok_or_else(|| RestError::NotFound {
            resource_type: "Submission".to_string(),
            id: sub_id.to_string(),
        })?;
    let _ = summary;

    let token = jobs
        .ensure_poll_token(ctx, &sub_id)
        .await
        .map_err(RestError::from)?;
    let status_url = format!(
        "{}/bulk-submit-status/{}",
        state.base_url().trim_end_matches('/'),
        token
    );
    Response::builder()
        .status(StatusCode::ACCEPTED)
        .header("Content-Location", status_url)
        .body(Body::empty())
        .map_err(|e| RestError::InternalError {
            message: e.to_string(),
        })
}

/// Parses a `$bulk-submit-status` body into `(submitter_key, submissionId)`.
fn parse_status_request(body: &Value) -> RestResult<(String, String)> {
    let params = body
        .get("parameter")
        .and_then(|p| p.as_array())
        .ok_or_else(|| bad_request("request body must be a Parameters resource"))?;
    let mut system = None;
    let mut value = None;
    let mut submission_id = None;
    let mut output_format: Option<String> = None;
    for p in params {
        match p.get("name").and_then(|n| n.as_str()) {
            Some("submitter") => {
                if let Some(id) = p.get("valueIdentifier") {
                    system = id.get("system").and_then(|v| v.as_str()).map(String::from);
                    value = id.get("value").and_then(|v| v.as_str()).map(String::from);
                }
            }
            Some("submissionId") => submission_id = scalar(p),
            Some("_outputFormat") => output_format = scalar(p),
            _ => {}
        }
    }
    // Only NDJSON status files are produced; reject any other requested format.
    if let Some(fmt) = output_format.as_deref() {
        if !matches!(
            fmt,
            "application/fhir+ndjson" | "application/ndjson" | "ndjson"
        ) {
            return Err(bad_request(format!(
                "unsupported _outputFormat '{fmt}' (only NDJSON is produced)"
            )));
        }
    }
    let value = value.ok_or_else(|| bad_request("submitter.value is required"))?;
    let submission_id = submission_id.ok_or_else(|| bad_request("submissionId is required"))?;
    Ok((
        format!("{}|{}", system.as_deref().unwrap_or(""), value),
        submission_id,
    ))
}

/// `GET /bulk-submit-status/{poll_token}` — poll / fetch the status manifest.
pub async fn bulk_submit_poll_handler<S>(
    State(state): State<AppState<S>>,
    Path(token): Path<String>,
    _tenant: TenantExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let cfg = state.bulk_submit_config();
    if !cfg.enabled {
        return Err(not_implemented());
    }
    let jobs = state
        .bulk_submit_jobs()
        .ok_or_else(not_implemented)?
        .clone();
    let output = state
        .bulk_submit_output()
        .ok_or_else(not_implemented)?
        .clone();
    let principal = request.extensions().get::<Principal>().cloned();
    check_submit_scope(principal.as_ref())?;

    let target = jobs
        .resolve_poll_token(&token)
        .await
        .map_err(RestError::from)?
        .ok_or_else(|| RestError::NotFound {
            resource_type: "bulk-submit-status".to_string(),
            id: token.clone(),
        })?;
    if !owns_submission(principal.as_ref(), target.owner_subject.as_deref()) {
        return Err(RestError::NotFound {
            resource_type: "bulk-submit-status".to_string(),
            id: token.clone(),
        });
    }
    let ctx = &target.tenant;
    let sub_id = &target.submission_id;

    let summary = jobs
        .get_submission(ctx, sub_id)
        .await
        .map_err(RestError::from)?
        .ok_or_else(|| RestError::NotFound {
            resource_type: "bulk-submit-status".to_string(),
            id: token.clone(),
        })?;

    let manifests = jobs
        .list_manifests(ctx, sub_id)
        .await
        .map_err(RestError::from)?;
    let all_terminal = manifests.iter().all(|m| m.status.is_terminal());
    let stopped = summary.status == SubmissionStatus::Aborted;

    if !all_terminal && !stopped {
        let pct = if manifests.is_empty() {
            0
        } else {
            (manifests.iter().filter(|m| m.status.is_terminal()).count() * 100) / manifests.len()
        };
        return Response::builder()
            .status(StatusCode::ACCEPTED)
            .header("X-Progress", format!("processing {pct}% complete"))
            .header("Retry-After", "120")
            .body(Body::empty())
            .map_err(|e| RestError::InternalError {
                message: e.to_string(),
            });
    }

    // Build the status manifest from recorded artifacts.
    let transaction_time = jobs
        .ensure_transaction_time(ctx, sub_id)
        .await
        .map_err(RestError::from)?;
    let files = jobs
        .list_submit_files(ctx, sub_id)
        .await
        .map_err(RestError::from)?;
    let job_id = submission_output_job_id(sub_id);
    let ttl = Duration::from_secs(cfg.file_url_ttl_secs);
    let base = state.base_url().trim_end_matches('/');

    let mut output_arr = Vec::new();
    let mut error_arr = Vec::new();
    let mut deleted_arr = Vec::new();
    let mut requires_token = cfg.requires_access_token != "false";

    for f in &files {
        let resource_type = f
            .resource_type
            .clone()
            .unwrap_or_else(|| "OperationOutcome".into());
        let key = ExportPartKey {
            tenant_id: ctx.tenant_id().as_str().to_string(),
            job_id: job_id.clone(),
            resource_type: resource_type.clone(),
            file_type: f.file_type.clone(),
            part_index: f.part_index,
            fencing_token: f.fencing_token,
        };
        let dl = output
            .download_url(&key, ttl)
            .await
            .map_err(RestError::from)?;
        // HFS-served artifacts (local-fs) MUST be advertised on the
        // `/bulk-submit-file/{poll_token}/{part}` surface so downloads are gated by
        // submit ownership + `system/bulk-submit` — never the export-file surface.
        // Pre-signed URLs (S3) are capability URLs and are used as-is.
        let url = if dl.requires_access_token {
            requires_token = true;
            format!(
                "{base}/bulk-submit-file/{token}/{resource_type}-{}",
                f.part_index
            )
        } else {
            requires_token = false;
            dl.url
        };
        let url = serde_json::Value::String(url);
        match f.file_type.as_str() {
            "output" => {
                let mut entry = json!({ "url": url, "count": f.line_count });
                if let Some(rt) = &f.resource_type {
                    entry["type"] = json!(rt);
                }
                if let Some(mu) = &f.manifest_url {
                    entry["manifestUrl"] = json!(mu);
                }
                output_arr.push(entry);
            }
            "error" => {
                let mut entry = json!({
                    "url": url,
                    "count": f.line_count,
                    "manifestUrl": f.manifest_url.clone().unwrap_or_default(),
                });
                if let Some(cs) = &f.count_severity {
                    entry["countSeverity"] = severity_array(cs);
                }
                error_arr.push(entry);
            }
            "deleted" => {
                deleted_arr.push(json!({ "url": url, "count": f.line_count }));
            }
            _ => {}
        }
    }

    let manifest = json!({
        "submissionId": sub_id.submission_id,
        "transactionTime": transaction_time.to_rfc3339(),
        "requiresAccessToken": requires_token,
        "outputFormat": "application/fhir+ndjson",
        "output": output_arr,
        "error": error_arr,
        "deleted": deleted_arr,
        // `link` is part of the status-manifest schema; HFS returns a single,
        // non-paginated manifest, so it is always empty.
        "link": [],
    });
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

/// Converts a stored `count_severity` object into the manifest `countSeverity` array.
fn severity_array(cs: &Value) -> Value {
    let mut arr = Vec::new();
    if let Some(obj) = cs.as_object() {
        for (code, count) in obj {
            arr.push(json!({ "code": code, "count": count }));
        }
    }
    Value::Array(arr)
}

/// `DELETE /bulk-submit-status/{poll_token}` — cancel + delete a submission.
pub async fn bulk_submit_cancel_handler<S>(
    State(state): State<AppState<S>>,
    Path(token): Path<String>,
    _tenant: TenantExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let cfg = state.bulk_submit_config();
    if !cfg.enabled {
        return Err(not_implemented());
    }
    let jobs = state
        .bulk_submit_jobs()
        .ok_or_else(not_implemented)?
        .clone();
    let output = state
        .bulk_submit_output()
        .ok_or_else(not_implemented)?
        .clone();
    let principal = request.extensions().get::<Principal>().cloned();
    check_submit_scope(principal.as_ref())?;

    let target = jobs
        .resolve_poll_token(&token)
        .await
        .map_err(RestError::from)?
        .ok_or_else(|| RestError::NotFound {
            resource_type: "bulk-submit-status".to_string(),
            id: token.clone(),
        })?;
    if !owns_submission(principal.as_ref(), target.owner_subject.as_deref()) {
        return Err(RestError::NotFound {
            resource_type: "bulk-submit-status".to_string(),
            id: token.clone(),
        });
    }
    let ctx = &target.tenant;
    let sub_id = &target.submission_id;

    // Cooperative cancel (rolls back recorded changes).
    let _ = jobs
        .abort_submission(ctx, sub_id, "cancelled via DELETE")
        .await;
    // Remove output-store artifacts, then their rows; then clear the poll token.
    let _ = output
        .delete_job_outputs(ctx, &submission_output_job_id(sub_id))
        .await;
    let _ = jobs.delete_submission_artifacts(ctx, sub_id).await;
    jobs.clear_poll_token(ctx, sub_id)
        .await
        .map_err(RestError::from)?;

    Response::builder()
        .status(StatusCode::ACCEPTED)
        .body(Body::empty())
        .map_err(|e| RestError::InternalError {
            message: e.to_string(),
        })
}

/// `GET /bulk-submit-file/{poll_token}/{part}` — serve an HFS-hosted artifact.
pub async fn bulk_submit_file_handler<S>(
    State(state): State<AppState<S>>,
    Path((token, part)): Path<(String, String)>,
    _tenant: TenantExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync + 'static,
{
    let cfg = state.bulk_submit_config();
    if !cfg.enabled {
        return Err(not_implemented());
    }
    let jobs = state
        .bulk_submit_jobs()
        .ok_or_else(not_implemented)?
        .clone();
    let output = state
        .bulk_submit_output()
        .ok_or_else(not_implemented)?
        .clone();
    let principal = request.extensions().get::<Principal>().cloned();
    check_submit_scope(principal.as_ref())?;

    let target = jobs
        .resolve_poll_token(&token)
        .await
        .map_err(RestError::from)?
        .ok_or_else(|| RestError::NotFound {
            resource_type: "bulk-submit-file".to_string(),
            id: format!("{token}/{part}"),
        })?;
    if !owns_submission(principal.as_ref(), target.owner_subject.as_deref()) {
        return Err(RestError::NotFound {
            resource_type: "bulk-submit-file".to_string(),
            id: format!("{token}/{part}"),
        });
    }
    let ctx = &target.tenant;
    let sub_id = &target.submission_id;

    // Resolve the part to a recorded artifact (part = "{resource_type}-{part_index}").
    let files = jobs
        .list_submit_files(ctx, sub_id)
        .await
        .map_err(RestError::from)?;
    let job_id = submission_output_job_id(sub_id);
    let matched = files.into_iter().find(|f| {
        let rt = f
            .resource_type
            .clone()
            .unwrap_or_else(|| "OperationOutcome".into());
        format!("{}-{}", rt, f.part_index) == part
    });
    let f = matched.ok_or_else(|| RestError::NotFound {
        resource_type: "bulk-submit-file".to_string(),
        id: format!("{token}/{part}"),
    })?;
    let key = ExportPartKey {
        tenant_id: ctx.tenant_id().as_str().to_string(),
        job_id,
        resource_type: f
            .resource_type
            .clone()
            .unwrap_or_else(|| "OperationOutcome".into()),
        file_type: f.file_type.clone(),
        part_index: f.part_index,
        fencing_token: f.fencing_token,
    };

    let mut reader = output.open_reader(&key).await.map_err(RestError::from)?;
    let mut bytes = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut bytes)
        .await
        .map_err(|e| RestError::InternalError {
            message: format!("failed to read submit file: {e}"),
        })?;
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/fhir+ndjson")
        .body(Body::from(bytes))
        .map_err(|e| RestError::InternalError {
            message: e.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn param(name: &str, key: &str, val: Value) -> Value {
        json!({ "name": name, key: val })
    }

    #[test]
    fn test_parse_minimal_valid() {
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                param("submitter", "valueIdentifier", json!({"system": "http://ehr", "value": "ehr-1"})),
                param("submissionId", "valueString", json!("sub-1")),
                param("manifestUrl", "valueUrl", json!("https://p/manifest.json")),
                param("fhirBaseUrl", "valueUrl", json!("https://p/fhir")),
            ]
        });
        let req = parse_submit_request(&body).expect("valid");
        assert_eq!(req.submitter_value, "ehr-1");
        assert_eq!(req.submission_id, "sub-1");
        assert_eq!(req.submitter_key(), "http://ehr|ehr-1");
        assert_eq!(req.submission_status, "in-progress");
    }

    #[test]
    fn test_requires_submitter_and_submission_id() {
        let body = json!({"resourceType": "Parameters", "parameter": [
            param("submissionId", "valueString", json!("s")),
            param("submissionStatus", "valueCoding", json!({"code": "completed"})),
        ]});
        assert!(parse_submit_request(&body).is_err()); // missing submitter

        let body = json!({"resourceType": "Parameters", "parameter": [
            param("submitter", "valueIdentifier", json!({"value": "e"})),
            param("submissionStatus", "valueCoding", json!({"code": "completed"})),
        ]});
        assert!(parse_submit_request(&body).is_err()); // missing submissionId
    }

    #[test]
    fn test_at_least_one_of_status_or_manifest() {
        // Neither submissionStatus nor manifestUrl → 400.
        let body = json!({"resourceType": "Parameters", "parameter": [
            param("submitter", "valueIdentifier", json!({"value": "e"})),
            param("submissionId", "valueString", json!("s")),
        ]});
        assert!(parse_submit_request(&body).is_err());
    }

    #[test]
    fn test_fhir_base_url_required_with_manifest() {
        let body = json!({"resourceType": "Parameters", "parameter": [
            param("submitter", "valueIdentifier", json!({"value": "e"})),
            param("submissionId", "valueString", json!("s")),
            param("manifestUrl", "valueUrl", json!("https://p/m.json")),
        ]});
        assert!(parse_submit_request(&body).is_err());
    }

    #[test]
    fn test_submission_status_codes() {
        for code in ["in-progress", "completed", "stopped"] {
            let body = json!({"resourceType": "Parameters", "parameter": [
                param("submitter", "valueIdentifier", json!({"value": "e"})),
                param("submissionId", "valueString", json!("s")),
                param("submissionStatus", "valueCoding", json!({
                    "system": "http://hl7.org/fhir/event-status", "code": code
                })),
            ]});
            let req = parse_submit_request(&body).expect("valid status");
            assert_eq!(req.submission_status, code);
        }
        // Invalid code rejected.
        let body = json!({"resourceType": "Parameters", "parameter": [
            param("submitter", "valueIdentifier", json!({"value": "e"})),
            param("submissionId", "valueString", json!("s")),
            param("submissionStatus", "valueCoding", json!({"code": "bogus"})),
        ]});
        assert!(parse_submit_request(&body).is_err());
    }

    #[test]
    fn test_parses_structured_parts() {
        let body = json!({"resourceType": "Parameters", "parameter": [
            param("submitter", "valueIdentifier", json!({"value": "e"})),
            param("submissionId", "valueString", json!("s")),
            param("manifestUrl", "valueUrl", json!("https://p/m.json")),
            param("fhirBaseUrl", "valueUrl", json!("https://p/fhir")),
            param("oauthMetadataUrl", "valueUrl", json!("https://p/.well-known/smart-configuration")),
            json!({"name": "fileRequestHeader", "part": [
                {"name": "headerName", "valueString": "X-Custom"},
                {"name": "headerValue", "valueString": "abc"}
            ]}),
            json!({"name": "import", "part": [
                {"name": "parameterUrl", "valueUri": "https://helios.software/import-mode"},
                {"name": "parameterValue", "valueString": "replace"}
            ]}),
            json!({"name": "metadata", "part": [
                {"name": "parameterUrl", "valueUri": "https://ex/ctx"},
                {"name": "parameterValue", "valueString": "v"}
            ]}),
        ]});
        let req = parse_submit_request(&body).expect("valid");
        assert_eq!(
            req.file_request_headers,
            vec![("X-Custom".to_string(), "abc".to_string())]
        );
        assert_eq!(
            req.oauth_metadata_urls,
            vec!["https://p/.well-known/smart-configuration".to_string()]
        );
        assert_eq!(req.import_directives.len(), 1);
        assert_eq!(req.import_directives[0].1, "replace");
        assert_eq!(req.metadata.len(), 1);
    }

    #[test]
    fn test_submission_status_wrong_system_rejected() {
        let body = json!({"resourceType": "Parameters", "parameter": [
            {"name": "submitter", "valueIdentifier": {"value": "e"}},
            {"name": "submissionId", "valueString": "s"},
            {"name": "submissionStatus", "valueCoding": {
                "system": "http://example.org/wrong", "code": "in-progress"
            }}
        ]});
        assert!(parse_submit_request(&body).is_err());

        // Correct system accepted.
        let body = json!({"resourceType": "Parameters", "parameter": [
            {"name": "submitter", "valueIdentifier": {"value": "e"}},
            {"name": "submissionId", "valueString": "s"},
            {"name": "submissionStatus", "valueCoding": {
                "system": "http://hl7.org/fhir/event-status", "code": "completed"
            }}
        ]});
        assert_eq!(
            parse_submit_request(&body).unwrap().submission_status,
            "completed"
        );
    }

    #[test]
    fn test_neither_status_nor_manifest_rejected() {
        let body = json!({"resourceType": "Parameters", "parameter": [
            {"name": "submitter", "valueIdentifier": {"value": "e"}},
            {"name": "submissionId", "valueString": "s"}
        ]});
        assert!(parse_submit_request(&body).is_err());
    }

    fn principal_with(scope: &str, subject: &str) -> Principal {
        Principal {
            subject: subject.to_string(),
            issuer: "https://issuer".to_string(),
            tenant_id: None,
            scopes: helios_auth::scope::ScopeSet::parse(scope),
            jti: None,
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            custom_claims: serde_json::Map::new(),
        }
    }

    #[test]
    fn test_submit_scope_gate() {
        // Auth disabled → allowed.
        assert!(check_submit_scope(None).is_ok());
        // Missing scope → forbidden.
        let p = principal_with("system/Patient.rs", "c1");
        assert!(matches!(
            check_submit_scope(Some(&p)),
            Err(RestError::Forbidden { .. })
        ));
        // Explicit operation scope → allowed.
        let p = principal_with("system/bulk-submit", "c1");
        assert!(check_submit_scope(Some(&p)).is_ok());
        // System wildcard → allowed.
        let p = principal_with("system/*.cruds", "c1");
        assert!(check_submit_scope(Some(&p)).is_ok());
    }

    #[test]
    fn test_ownership_check() {
        // Auth disabled → owned.
        assert!(owns_submission(None, Some("owner")));
        // Matching subject → owned.
        let p = principal_with("system/bulk-submit", "owner");
        assert!(owns_submission(Some(&p), Some("owner")));
        // Wildcard scope bypasses ownership.
        let p = principal_with("system/*.rs", "someone-else");
        assert!(owns_submission(Some(&p), Some("owner")));
        // Mismatched subject without wildcard → not owned.
        let p = principal_with("system/bulk-submit", "someone-else");
        assert!(!owns_submission(Some(&p), Some("owner")));
    }
}
