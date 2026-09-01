//! FHIR Bulk Data Submit (`$bulk-submit`) handlers.
//!
//! HFS is the **Data Consumer**: `POST /$bulk-submit` records a submission and
//! enqueues its manifest for asynchronous fetch + ingest; `POST /$bulk-submit-status`
//! returns a `Content-Location` the provider polls; `GET`/`DELETE` on that location
//! poll / cancel; and `GET /bulk-submit-file/{token}/{part}` serves HFS-hosted
//! status-manifest artifacts.
//!
//! Implements the FHIR Bulk Data Access **Submit** operation:
//! <https://build.fhir.org/ig/HL7/bulk-data/en/submit.html>.

use std::net::IpAddr;
use std::time::Duration;

use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::{Method, StatusCode},
    response::Response,
};
use helios_auth::Principal;
use helios_persistence::core::{
    DownloadUrl, ExportPartKey, IMPORT_MODE_PARAMETER_URL, ImportMode, ManifestFetchParams,
    ManifestStatus, ResourceStorage, SubmissionId, SubmissionStatus, submission_output_job_id,
};
use serde_json::{Value, json};

use super::bulk_common::{first_value, parse_query_pairs};
use crate::config::BulkSubmitConfig;
use crate::error::{RestError, RestResult};
use crate::extractors::{PeerIp, TenantExtractor};
use crate::rate_limit::RateLimiter;
use crate::state::AppState;

/// The SMART operation scope authorizing every `$bulk-submit` surface.
const SUBMIT_SCOPE: &str = "bulk-submit";

/// Query parameter selecting a status-manifest page (1-based).
const PAGE_PARAM: &str = "page";

fn external_download_url(download: &DownloadUrl) -> Option<String> {
    (!download.requires_access_token).then(|| download.url.clone())
}

fn advertised_download_url(download: &DownloadUrl, fallback: impl FnOnce() -> String) -> String {
    external_download_url(download).unwrap_or_else(fallback)
}

/// The code system for the `submissionStatus` Coding (per the IG).
const SUBMISSION_STATUS_SYSTEM: &str = "http://hl7.org/fhir/event-status";
/// The pre-STU4 draft's status system, still sent by providers tracking the
/// older spec (the SMART reference provider among them). Accepted leniently;
/// its `complete`/`aborted` codes map onto `completed`/`stopped`.
const LEGACY_SUBMISSION_STATUS_SYSTEM: &str =
    "http://hl7.org/fhir/uv/bulkdata/ValueSet/submission-status";

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
// Status-poll rate limiting
// ---------------------------------------------------------------------------

/// Buckets for the status-poll surface. Private to this module, so a hammering
/// poller cannot spend another surface's budget.
static POLL_LIMITER: RateLimiter = RateLimiter::new();

/// Who a poll is billed to: the poll token scopes the bucket to one submission,
/// and the principal (else the peer address) separates clients sharing it. A
/// deployment with neither auth nor a recorded peer shares one bucket per
/// token — the restrictive reading, since polling is what we are limiting.
fn poll_rate_limit_key(token: &str, principal: Option<&Principal>, peer: Option<IpAddr>) -> String {
    match principal {
        Some(p) => format!("{token}|sub:{}", p.subject),
        None => match peer {
            Some(ip) => format!("{token}|ip:{ip}"),
            None => format!("{token}|anon"),
        },
    }
}

/// Enforces the per-client status-poll rate limit.
///
/// The Submit spec has Data Consumers rate-limit the status endpoint and tell a
/// throttled client when to come back, so the `429` carries the delta-seconds
/// until the window rolls forward (issue #399). `poll_rate_limit = 0` disables
/// the limiter outright.
fn check_poll_rate_limit(
    cfg: &BulkSubmitConfig,
    token: &str,
    principal: Option<&Principal>,
    peer: Option<IpAddr>,
) -> RestResult<()> {
    if cfg.poll_rate_limit == 0 {
        return Ok(());
    }
    let key = poll_rate_limit_key(token, principal, peer);
    POLL_LIMITER
        .check_window(
            &key,
            cfg.poll_rate_limit,
            Duration::from_secs(cfg.poll_rate_window_secs),
        )
        .map_err(|limited| RestError::TooManyRequests {
            message: format!(
                "too many status polls (max {} per {}s); honour the Retry-After header",
                cfg.poll_rate_limit, cfg.poll_rate_window_secs
            ),
            retry_after_secs: Some(limited.retry_after_secs),
        })
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

/// Reads an `import` / `metadata` parameter's `(parameterUrl, parameterValue)` pair.
///
/// Both child parts are 1..1 in the IG and `parameterUrl` SHALL be an absolute URL,
/// so a malformed directive is a client error rather than something to drop silently.
fn pre_coordinated_part(param: &Value, kind: &str) -> RestResult<(String, String)> {
    let url = part_scalar(param, "parameterUrl")
        .ok_or_else(|| bad_request(format!("`{kind}.parameterUrl` is required")))?;
    if !url.contains("://") {
        return Err(bad_request(format!(
            "`{kind}.parameterUrl` must be an absolute URL, got '{url}'"
        )));
    }
    let value = part_scalar(param, "parameterValue")
        .ok_or_else(|| bad_request(format!("`{kind}.parameterValue` is required")))?;
    Ok((url, value))
}

/// Validates the `import` directives HFS recognizes and reports the rest.
///
/// A recognized directive with an unusable value is always a `400`. Directives HFS
/// does not recognize are rejected under `Prefer: handling=strict` and ignored
/// (with a warning) otherwise. `metadata` parts carry no processing semantics — HFS
/// retains every one of them verbatim, so none are rejected.
fn validate_import_directives(directives: &[(String, String)], strict: bool) -> RestResult<()> {
    for (url, value) in directives {
        if url == IMPORT_MODE_PARAMETER_URL {
            if ImportMode::parse(value).is_none() {
                return Err(bad_request(format!(
                    "unsupported import mode '{value}' for '{IMPORT_MODE_PARAMETER_URL}' \
                     (expected replace|merge)"
                )));
            }
        } else if strict {
            return Err(bad_request(format!(
                "unrecognized `import` directive '{url}' rejected under Prefer: handling=strict"
            )));
        } else {
            tracing::warn!(
                directive = %url,
                "ignoring unrecognized `import` directive on $bulk-submit kickoff"
            );
        }
    }
    Ok(())
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
                        if system != SUBMISSION_STATUS_SYSTEM
                            && system != LEGACY_SUBMISSION_STATUS_SYSTEM
                        {
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
                    // Providers tracking the pre-STU4 draft (the SMART
                    // reference among them) code these two differently.
                    req.submission_status = match code.as_str() {
                        "complete" => "completed".to_string(),
                        "aborted" => "stopped".to_string(),
                        _ => code,
                    };
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
            "import" => req
                .import_directives
                .push(pre_coordinated_part(p, "import")?),
            "metadata" => req.metadata.push(pre_coordinated_part(p, "metadata")?),
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

    let request_url = state.public_url_for_request(&tenant, ["$bulk-submit"]);
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

    validate_import_directives(&req.import_directives, strict)?;

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
                // A slot frees when an in-flight submission finishes, which no
                // one can time; the advertised poll cadence is the best hint
                // available and keeps retries off a tight loop.
                retry_after_secs: Some(cfg.retry_after_secs),
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
            ManifestFetchParams {
                fhir_base_url: req.fhir_base_url.as_deref(),
                output_format: req.output_format.as_deref(),
                file_request_headers: &req.file_request_headers,
                oauth_metadata_urls: &req.oauth_metadata_urls,
                file_encryption_key: req.file_encryption_key.as_ref(),
                import_directives: &req.import_directives,
                metadata: &req.metadata,
            },
        )
        .await
        .map_err(RestError::from)?;
    } else if !req.import_directives.is_empty() || !req.metadata.is_empty() {
        // `import`/`metadata` are pre-coordinated with the data they describe; a
        // status-only kickoff carries no manifest for them to attach to.
        tracing::warn!(
            submission = %sub_id,
            "ignoring `import`/`metadata` directives on a kickoff without a manifestUrl"
        );
    }

    // submissionStatus=stopped → abort (rolls back recorded changes).
    if req.submission_status == "stopped" {
        jobs.abort_submission(ctx, &sub_id, "submissionStatus=stopped")
            .await
            .map_err(RestError::from)?;
    }

    // submissionStatus=completed → mark the submission terminal. Per the spec this
    // means "no additional requests are expected for this submitter+submissionId",
    // not "stop processing" — manifests registered by this or an earlier request
    // still drain, and workers keep claiming them (see `claim_next_manifest`, which
    // admits `complete` submissions for exactly this reason). Subsequent kick-offs
    // are rejected by the terminal-status check above.
    if req.submission_status == "completed" {
        jobs.complete_submission(ctx, &sub_id)
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
    let status_url = state.public_url_for_request(&tenant, ["bulk-submit-status", token.as_str()]);
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

/// Parses the 1-based `?page=N` status-manifest page selector; absent → page 1.
fn parse_page_param(query: Option<&str>) -> RestResult<usize> {
    let pairs = parse_query_pairs(query);
    match first_value(&pairs, PAGE_PARAM) {
        None => Ok(1),
        Some(raw) => raw
            .parse::<usize>()
            .ok()
            .filter(|n| *n >= 1)
            .ok_or_else(|| bad_request(format!("invalid page '{raw}' (expected an integer >= 1)"))),
    }
}

/// `GET /bulk-submit-status/{poll_token}` — poll / fetch the status manifest.
///
/// The manifest's `output` / `outcome` / `deleted` entries are paginated at
/// `manifest_page_size` entries per page; when more remain, the response carries a
/// `link[]` entry with `relation: next` pointing at `?page=N+1` on this same URL
/// (per <https://build.fhir.org/ig/HL7/bulk-data/en/submit.html>). Every other
/// manifest field repeats identically on each page, as the spec requires.
pub async fn bulk_submit_poll_handler<S>(
    State(state): State<AppState<S>>,
    Path(token): Path<String>,
    tenant: TenantExtractor,
    PeerIp(peer): PeerIp,
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

    // Throttle before any job-store work: a client ignoring the advertised
    // Retry-After should cost a cheap 429, not a round trip per poll.
    check_poll_rate_limit(cfg, &token, principal.as_ref(), peer)?;

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
    if target.tenant.tenant_id().as_str() != tenant.tenant_id() {
        return Err(RestError::NotFound {
            resource_type: "bulk-submit-status".to_string(),
            id: token.clone(),
        });
    }
    let ctx = &target.tenant;
    let sub_id = &target.submission_id;
    let page = parse_page_param(request.uri().query())?;

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
        // A `processing` manifest whose lease expired several lease-durations
        // ago has a worker that stopped heartbeating and no healthy worker
        // reclaiming it (#646). Say so: the old answer was a quiet
        // "processing" forever, indistinguishable from progress. State is not
        // mutated — a remote worker deployment may still legitimately pick
        // the manifest back up, and the reclaim path stays the authority.
        let stall_after = chrono::Duration::seconds((cfg.lease_duration_secs as i64) * 3);
        let now = chrono::Utc::now();
        let stalled = manifests.iter().any(|m| {
            m.status == helios_persistence::core::ManifestStatus::Processing
                && m.lease_expiry.is_some_and(|e| now - e > stall_after)
        });
        // The percentage is manifest-granular, so a single-manifest
        // submission reads 0% until the very end; the entry counter is what
        // actually moves while a file streams in (#790).
        let entries: u64 = manifests.iter().map(|m| m.processed_entries).sum();
        let progress = if stalled {
            tracing::warn!(
                submission = %sub_id,
                "bulk-submit ingestion appears stalled: a processing manifest's \
                 worker lease expired without renewal or reclaim"
            );
            format!("stalled at {pct}% - a worker stopped without handoff; see server logs")
        } else if entries > 0 {
            format!("processing {pct}% complete ({entries} entries ingested)")
        } else {
            format!("processing {pct}% complete")
        };
        return Response::builder()
            .status(StatusCode::ACCEPTED)
            .header("X-Progress", progress)
            .header("Retry-After", cfg.retry_after_secs.to_string())
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

    // Slice the artifact rows into the requested page. Backends return them in a
    // stable order (`ORDER BY id`) and rows are append-only until the whole
    // submission is deleted, so a given page is the same set across polls.
    let page_size = cfg.manifest_page_size as usize;
    let total_pages = if page_size == 0 {
        1
    } else {
        files.len().div_ceil(page_size).max(1)
    };
    if page > total_pages {
        return Err(RestError::NotFound {
            resource_type: "bulk-submit-status".to_string(),
            id: format!("{token}?{PAGE_PARAM}={page}"),
        });
    }
    let page_files = if page_size == 0 {
        &files[..]
    } else {
        let start = (page - 1) * page_size;
        &files[start..(start + page_size).min(files.len())]
    };

    let mut output_arr = Vec::new();
    let mut outcome_arr = Vec::new();
    let mut deleted_arr = Vec::new();
    // `requiresAccessToken` describes whether retrieving *any* file requires a
    // token. Seed from the configured posture, then OR in each file — never let a
    // single open file clear a token requirement set by another file or the config.
    // The posture an output store returns is store-wide, not per-artifact, so this
    // stays identical on every page even though each page sees a different slice.
    let mut requires_token = cfg.requires_access_token == "true";

    for f in page_files {
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
        requires_token |= dl.requires_access_token;
        let url = advertised_download_url(&dl, || {
            let part = format!("{resource_type}-{}", f.part_index);
            state.public_url_for_request(
                &tenant,
                ["bulk-submit-file", token.as_str(), part.as_str()],
            )
        });
        let url = serde_json::Value::String(url);
        match f.file_type.as_str() {
            "output" => {
                let mut entry = json!({
                    "url": url,
                    "count": f.line_count,
                    "fileSize": f.byte_count,
                });
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
                    "fileSize": f.byte_count,
                    "manifestUrl": f.manifest_url.clone().unwrap_or_default(),
                });
                if let Some(cs) = &f.count_severity {
                    entry["countSeverity"] = severity_array(cs);
                }
                outcome_arr.push(entry);
            }
            "deleted" => {
                deleted_arr.push(json!({
                    "url": url,
                    "count": f.line_count,
                    "fileSize": f.byte_count,
                }));
            }
            _ => {}
        }
    }

    // A single `next` link chains to the following page; the last page has none.
    let mut link_arr = Vec::new();
    if page < total_pages {
        let query = format!("{PAGE_PARAM}={}", page + 1);
        link_arr.push(json!({
            "relation": "next",
            "url": state.public_url_for_request_with_query(
                &tenant,
                ["bulk-submit-status", token.as_str()],
                Some(&query),
            ),
        }));
    }

    let manifest = json!({
        "submissionId": sub_id.submission_id,
        "transactionTime": transaction_time.to_rfc3339(),
        "requiresAccessToken": requires_token,
        "outputFormat": "application/fhir+ndjson",
        "output": output_arr,
        "outcome": outcome_arr,
        "deleted": deleted_arr,
        "link": link_arr,
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
    if target.tenant.tenant_id().as_str() != tenant.tenant_id() {
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
    if target.tenant.tenant_id().as_str() != tenant.tenant_id() {
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

    /// Providers on the pre-STU4 draft (the SMART reference among them) code
    /// the status against the old ValueSet URL with `complete`/`aborted`.
    /// Both are accepted and normalized onto the spec vocabulary.
    #[test]
    fn test_parse_legacy_status_vocabulary() {
        for (legacy, normalized) in [("complete", "completed"), ("aborted", "stopped")] {
            let body = json!({
                "resourceType": "Parameters",
                "parameter": [
                    param("submitter", "valueIdentifier", json!({"system": "http://ehr", "value": "ehr-1"})),
                    param("submissionId", "valueString", json!("sub-1")),
                    param("submissionStatus", "valueCoding", json!({
                        "system": LEGACY_SUBMISSION_STATUS_SYSTEM,
                        "code": legacy
                    })),
                ]
            });
            let req = parse_submit_request(&body).expect("legacy accepted");
            assert_eq!(req.submission_status, normalized);
        }
        // An unknown system is still rejected.
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                param("submitter", "valueIdentifier", json!({"system": "http://ehr", "value": "ehr-1"})),
                param("submissionId", "valueString", json!("sub-1")),
                param("submissionStatus", "valueCoding", json!({
                    "system": "http://example.org/other", "code": "completed"
                })),
            ]
        });
        assert!(parse_submit_request(&body).is_err());
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
                {"name": "parameterUrl", "valueUri": IMPORT_MODE_PARAMETER_URL},
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

    /// Builds a kickoff body carrying a single `import` or `metadata` directive.
    fn body_with_directive(kind: &str, parts: Value) -> Value {
        json!({"resourceType": "Parameters", "parameter": [
            param("submitter", "valueIdentifier", json!({"value": "e"})),
            param("submissionId", "valueString", json!("s")),
            param("manifestUrl", "valueUrl", json!("https://p/m.json")),
            param("fhirBaseUrl", "valueUrl", json!("https://p/fhir")),
            json!({"name": kind, "part": parts}),
        ]})
    }

    #[test]
    fn test_directive_parts_are_required_and_absolute() {
        for kind in ["import", "metadata"] {
            // parameterValue missing (1..1 in the IG).
            let body = body_with_directive(
                kind,
                json!([{"name": "parameterUrl", "valueUri": "https://ex/a"}]),
            );
            assert!(parse_submit_request(&body).is_err(), "{kind} without value");

            // parameterUrl missing.
            let body = body_with_directive(
                kind,
                json!([{"name": "parameterValue", "valueString": "v"}]),
            );
            assert!(parse_submit_request(&body).is_err(), "{kind} without url");

            // parameterUrl SHALL be an absolute URL.
            let body = body_with_directive(
                kind,
                json!([
                    {"name": "parameterUrl", "valueUri": "not-absolute"},
                    {"name": "parameterValue", "valueString": "v"}
                ]),
            );
            assert!(parse_submit_request(&body).is_err(), "{kind} relative url");
        }
    }

    #[test]
    fn test_import_mode_directive_values() {
        for mode in ["replace", "merge"] {
            let body = body_with_directive(
                "import",
                json!([
                    {"name": "parameterUrl", "valueUri": IMPORT_MODE_PARAMETER_URL},
                    {"name": "parameterValue", "valueString": mode}
                ]),
            );
            let req = parse_submit_request(&body).expect("recognized mode");
            // A recognized directive is accepted under both handling postures.
            assert!(validate_import_directives(&req.import_directives, true).is_ok());
            assert!(validate_import_directives(&req.import_directives, false).is_ok());
            assert_eq!(
                ImportMode::from_directives(&req.import_directives),
                ImportMode::parse(mode).unwrap()
            );
        }

        // An unusable value for a directive we DO recognize is always a 400.
        let body = body_with_directive(
            "import",
            json!([
                {"name": "parameterUrl", "valueUri": IMPORT_MODE_PARAMETER_URL},
                {"name": "parameterValue", "valueString": "upsert"}
            ]),
        );
        let req = parse_submit_request(&body).expect("parses");
        assert!(validate_import_directives(&req.import_directives, false).is_err());
        assert!(validate_import_directives(&req.import_directives, true).is_err());
    }

    #[test]
    fn test_unrecognized_import_directive_handling() {
        let body = body_with_directive(
            "import",
            json!([
                {"name": "parameterUrl", "valueUri": "https://ex/unknown-option"},
                {"name": "parameterValue", "valueString": "on"}
            ]),
        );
        let req = parse_submit_request(&body).expect("parses");
        // Lenient (default): ignored, and the mode falls back to replace.
        assert!(validate_import_directives(&req.import_directives, false).is_ok());
        assert_eq!(
            ImportMode::from_directives(&req.import_directives),
            ImportMode::Replace
        );
        // Strict: rejected.
        assert!(validate_import_directives(&req.import_directives, true).is_err());
    }

    #[test]
    fn test_metadata_is_never_rejected_under_strict() {
        // HFS retains every `metadata` part verbatim, so unknown URLs are not a
        // reason to fail the kickoff even under `Prefer: handling=strict`.
        let body = body_with_directive(
            "metadata",
            json!([
                {"name": "parameterUrl", "valueUri": "https://ex/whatever"},
                {"name": "parameterValue", "valueString": "v"}
            ]),
        );
        let req = parse_submit_request(&body).expect("parses");
        assert_eq!(req.metadata.len(), 1);
        assert!(validate_import_directives(&req.import_directives, true).is_ok());
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

    #[test]
    fn test_parse_page_param() {
        // Absent / empty query → first page.
        assert_eq!(parse_page_param(None).unwrap(), 1);
        assert_eq!(parse_page_param(Some("")).unwrap(), 1);
        // Explicit page, alone or alongside other params.
        assert_eq!(parse_page_param(Some("page=3")).unwrap(), 3);
        assert_eq!(parse_page_param(Some("foo=bar&page=2")).unwrap(), 2);
        // Non-numeric, zero, and negative pages are rejected.
        for q in ["page=abc", "page=0", "page=-1", "page="] {
            assert!(
                matches!(parse_page_param(Some(q)), Err(RestError::BadRequest { .. })),
                "expected 400 for query '{q}'"
            );
        }
    }
}
