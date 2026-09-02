//! Bulk Export workspace (`/ui/bulk-export`) — driving HFS's own `$export`
//! operation (#537).
//!
//! The pull-based companion to the Bulk Import workspace: the user picks a
//! scope (everything / patients / group), the resource types, and the
//! narrowing filters; the workspace kicks off the server's async `$export`,
//! then tracks the job on the Exports page — one server-side status
//! poll per htmx fetch, exactly like the import workspace's recipient
//! polling.
//!
//! Kick-offs, Patient lookup, and polls are self-calls against the configured loopback HFS base,
//! with the caller's own `Authorization` and tenant forwarded — so the export
//! runs with the user's credentials, not a service account. Advertised public
//! paths are validated, but stored URLs are never used as outbound authorities.
//!
//! Job state lives in the per-user settings document under
//! `byTenant.<tenant>.bulkExport.jobs`, object-keyed by id.

use std::collections::{HashMap, HashSet};
use std::io;

use askama::Template;
use async_zip::{Compression, ZipEntryBuilder, base::write::ZipFileWriter};
use axum::{
    Extension,
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header::CACHE_CONTROL},
    response::{IntoResponse, Redirect, Response},
};
use axum_htmx::HxRequest;
use chrono::{Duration, SecondsFormat, Utc};
use futures_lite::future::zip;
use futures_lite::io::AsyncWriteExt as FuturesAsyncWriteExt;
use helios_fhir::FhirVersion;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
use tokio::io::AsyncReadExt;

use crate::i18n::{I18n, RequestLocale};
use crate::{RequestTenant, RequestVersion, WebState, current_status, render, settings_user_key};

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExportJob {
    #[serde(default)]
    pub name: String,
    /// `system` | `patient` | `group`.
    #[serde(default)]
    pub scope: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group_id: String,
    /// Comma-separated `_type` list; empty exports every type.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub types: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub elements: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub type_filter: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub since: String,
    /// Canonical `Patient/{logical-id}` references selected for this export.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patient_refs: Vec<String>,
    /// The request version used for the original kick-off. Optional so jobs
    /// persisted by older HFS versions continue to deserialize and retry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fhir_version: Option<FhirVersion>,
    /// `in-progress` | `complete` | `failed` | `cancelled`.
    #[serde(default)]
    pub status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub poll_url: String,
    /// Trust decision captured at kick-off. Legacy records have `unknown`.
    #[serde(default, skip_serializing_if = "RemoteJobProvenance::is_unknown")]
    remote_job: RemoteJobProvenance,
    /// REST job UUID, populated only when `remote_job` is `known`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    remote_job_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub progress: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub finished_at: String,
    /// Completion-manifest `output` entries (`{type, url, count?}`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<Value>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum RemoteJobProvenance {
    /// The request may have reached HFS, but no valid job identity was returned.
    #[default]
    Unknown,
    /// A valid REST job UUID was returned by the trusted kick-off response.
    Known,
    /// HFS rejected the kick-off before creating a remotely addressable job.
    NoRemoteJob,
}

impl RemoteJobProvenance {
    fn is_unknown(&self) -> bool {
        *self == Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteJobIdentity {
    Known(uuid::Uuid),
    NoRemoteJob,
    Unknown,
}

#[derive(Default)]
struct JobsSnapshot {
    jobs: serde_json::Map<String, Value>,
    version: i64,
}

async fn load_jobs_checked(
    state: &WebState,
    user_key: &str,
    tenant: &str,
) -> Result<JobsSnapshot, String> {
    let Some(store) = &state.settings else {
        return Ok(JobsSnapshot::default());
    };
    let Some(stored) = store
        .get_settings(user_key)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(JobsSnapshot::default());
    };
    let jobs = stored
        .document
        .get("byTenant")
        .and_then(|value| value.get(tenant))
        .and_then(|value| value.get("bulkExport"))
        .and_then(|value| value.get("jobs"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    Ok(JobsSnapshot {
        jobs,
        version: stored.version,
    })
}

async fn load_jobs(state: &WebState, user_key: &str, tenant: &str) -> JobsSnapshot {
    load_jobs_checked(state, user_key, tenant)
        .await
        .unwrap_or_default()
}

fn optional_string(value: &str) -> Value {
    if value.is_empty() {
        Value::Null
    } else {
        Value::String(value.to_string())
    }
}

/// An explicit merge value for every job field. Nulls clear fields left behind
/// by an earlier lifecycle state, so one CAS patch is enough.
fn job_merge_value(job: &ExportJob) -> Value {
    json!({
        "name": job.name,
        "scope": job.scope,
        "groupId": optional_string(&job.group_id),
        "types": optional_string(&job.types),
        "elements": optional_string(&job.elements),
        "typeFilter": optional_string(&job.type_filter),
        "since": optional_string(&job.since),
        "patientRefs": if job.patient_refs.is_empty() {
            Value::Null
        } else {
            json!(&job.patient_refs)
        },
        "fhirVersion": serde_json::to_value(job.fhir_version).unwrap_or(Value::Null),
        "status": job.status,
        "pollUrl": optional_string(&job.poll_url),
        "remoteJob": match job.remote_job {
            RemoteJobProvenance::Unknown => Value::Null,
            RemoteJobProvenance::Known => Value::String("known".to_string()),
            RemoteJobProvenance::NoRemoteJob => Value::String("no-remote-job".to_string()),
        },
        "remoteJobId": optional_string(&job.remote_job_id),
        "progress": optional_string(&job.progress),
        "error": optional_string(&job.error),
        "startedAt": job.started_at,
        "finishedAt": optional_string(&job.finished_at),
        "files": if job.files.is_empty() { Value::Null } else { Value::Array(job.files.clone()) },
    })
}

async fn store_job(
    state: &WebState,
    user_key: &str,
    tenant: &str,
    id: &str,
    job: &ExportJob,
    expected_version: i64,
) -> Result<(), String> {
    let Some(store) = &state.settings else {
        return Err("settings store unavailable".to_string());
    };
    let value = job_merge_value(job);
    let patch = json!({ "byTenant": { tenant: { "bulkExport": { "jobs": { id: value } } } } });
    store
        .patch_settings(user_key, patch, Some(expected_version))
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

async fn remove_job(
    state: &WebState,
    user_key: &str,
    tenant: &str,
    id: &str,
    expected_version: i64,
) -> Result<(), String> {
    let Some(store) = &state.settings else {
        return Err("settings store unavailable".to_string());
    };
    let patch =
        json!({ "byTenant": { tenant: { "bulkExport": { "jobs": { id: Value::Null } } } } });
    store
        .patch_settings(user_key, patch, Some(expected_version))
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

enum MemberExpectation<'a> {
    Absent,
    Unchanged(&'a Value),
}

impl MemberExpectation<'_> {
    fn matches(&self, current: Option<&Value>) -> bool {
        match self {
            Self::Absent => current.is_none(),
            Self::Unchanged(expected) => current == Some(*expected),
        }
    }
}

const SETTINGS_CAS_ATTEMPTS: usize = 3;

async fn store_job_conditionally(
    state: &WebState,
    user_key: &str,
    tenant: &str,
    id: &str,
    job: &ExportJob,
    mut expected_version: i64,
    expectation: MemberExpectation<'_>,
) -> Result<(), String> {
    let mut last_error = String::new();
    for attempt in 0..SETTINGS_CAS_ATTEMPTS {
        match store_job(state, user_key, tenant, id, job, expected_version).await {
            Ok(()) => return Ok(()),
            Err(error) => last_error = error,
        }
        if attempt + 1 == SETTINGS_CAS_ATTEMPTS {
            break;
        }
        let refreshed = load_jobs_checked(state, user_key, tenant).await?;
        if !expectation.matches(refreshed.jobs.get(id)) {
            return Err("export job changed concurrently".to_string());
        }
        expected_version = refreshed.version;
    }
    Err(last_error)
}

async fn remove_job_conditionally(
    state: &WebState,
    user_key: &str,
    tenant: &str,
    id: &str,
    expected_member: &Value,
    mut expected_version: i64,
) -> Result<(), String> {
    let mut last_error = String::new();
    for attempt in 0..SETTINGS_CAS_ATTEMPTS {
        match remove_job(state, user_key, tenant, id, expected_version).await {
            Ok(()) => return Ok(()),
            Err(error) => last_error = error,
        }
        if attempt + 1 == SETTINGS_CAS_ATTEMPTS {
            break;
        }
        let refreshed = load_jobs_checked(state, user_key, tenant).await?;
        if refreshed.jobs.get(id) != Some(expected_member) {
            return Err("export job changed concurrently".to_string());
        }
        expected_version = refreshed.version;
    }
    Err(last_error)
}

fn now_stamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_job(value: &Value) -> ExportJob {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

fn base_url_with_segments<'a>(
    base: &str,
    tenant: &str,
    tenant_path_routing: bool,
    segments: impl IntoIterator<Item = &'a str>,
) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(base).map_err(|e| e.to_string())?;
    url.set_query(None);
    url.set_fragment(None);
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| "HFS base URL cannot carry path segments".to_string())?;
        path.pop_if_empty();
        if tenant_path_routing {
            path.push(tenant);
        }
        for segment in segments {
            path.push(segment);
        }
    }
    Ok(url)
}

/// Internal self-call URL. This is always based on the trusted loopback base,
/// never a Host header or an advertised/public authority.
fn internal_api_url<'a>(
    state: &WebState,
    tenant: &str,
    segments: impl IntoIterator<Item = &'a str>,
) -> Result<reqwest::Url, String> {
    base_url_with_segments(
        &state.self_base_url,
        tenant,
        state.tenant_path_routing,
        segments,
    )
}

fn public_api_url<'a>(
    state: &WebState,
    tenant: &str,
    tenant_path_routing: bool,
    segments: impl IntoIterator<Item = &'a str>,
) -> Result<reqwest::Url, String> {
    base_url_with_segments(
        &state.public_base_url,
        tenant,
        tenant_path_routing,
        segments,
    )
}

fn no_redirect_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())
}

fn parse_url_without_credentials(raw: &str) -> Option<reqwest::Url> {
    let parsed = reqwest::Url::parse(raw)
        .or_else(|_| reqwest::Url::parse("http://hfs.invalid").and_then(|base| base.join(raw)));
    let url = parsed.ok()?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    Some(url)
}

/// Accepts only the two legacy path shapes that HFS emitted before the UI
/// stored an explicit REST job UUID. The stored authority is discarded.
fn legacy_remote_job_id(state: &WebState, poll_url: &str, tenant: &str) -> Option<uuid::Uuid> {
    let url = parse_url_without_credentials(poll_url)?;
    let id = url
        .path_segments()?
        .next_back()
        .and_then(|raw| uuid::Uuid::parse_str(raw).ok())?;
    let id_text = id.as_hyphenated().to_string();
    let without_tenant =
        public_api_url(state, tenant, false, ["export-status", id_text.as_str()]).ok()?;
    let with_tenant =
        public_api_url(state, tenant, true, ["export-status", id_text.as_str()]).ok()?;
    matches!(url.path(), path if path == without_tenant.path() || path == with_tenant.path())
        .then_some(id)
}

fn remote_job_identity(job: &ExportJob, state: &WebState, tenant: &str) -> RemoteJobIdentity {
    match job.remote_job {
        RemoteJobProvenance::Known => uuid::Uuid::parse_str(&job.remote_job_id)
            .map(RemoteJobIdentity::Known)
            .unwrap_or(RemoteJobIdentity::Unknown),
        RemoteJobProvenance::NoRemoteJob => RemoteJobIdentity::NoRemoteJob,
        RemoteJobProvenance::Unknown => {
            if job.poll_url.is_empty() && job.error.starts_with("kick-off answered ") {
                RemoteJobIdentity::NoRemoteJob
            } else {
                legacy_remote_job_id(state, &job.poll_url, tenant)
                    .map(RemoteJobIdentity::Known)
                    .unwrap_or(RemoteJobIdentity::Unknown)
            }
        }
    }
}

fn trusted_kickoff_job_id(
    state: &WebState,
    tenant: &str,
    content_location: &str,
) -> Option<uuid::Uuid> {
    let url = parse_url_without_credentials(content_location)?;
    let id = url
        .path_segments()?
        .next_back()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())?;
    let expected = public_api_url(
        state,
        tenant,
        state.tenant_path_routing,
        ["export-status", id.as_hyphenated().to_string().as_str()],
    )
    .ok()?;
    (url.path() == expected.path()).then_some(id)
}

fn status_url(state: &WebState, tenant: &str, id: &uuid::Uuid) -> Result<reqwest::Url, String> {
    let id = id.as_hyphenated().to_string();
    internal_api_url(state, tenant, ["export-status", id.as_str()])
}

fn public_status_url(
    state: &WebState,
    tenant: &str,
    id: &uuid::Uuid,
) -> Result<reqwest::Url, String> {
    let id = id.as_hyphenated().to_string();
    public_api_url(
        state,
        tenant,
        state.tenant_path_routing,
        ["export-status", id.as_str()],
    )
}

/// Forwards the caller's credentials and tenant onto a self-call, so the
/// export runs as the user who asked for it.
fn forward_identity(
    mut request: reqwest::RequestBuilder,
    headers: &HeaderMap,
    tenant: &str,
) -> reqwest::RequestBuilder {
    if let Some(auth) = headers.get("authorization").and_then(|v| v.to_str().ok()) {
        request = request.header("Authorization", auth);
    }
    request = request.header("X-Tenant-ID", tenant);
    request
}

// ---------------------------------------------------------------------------
// View models & templates
// ---------------------------------------------------------------------------

struct JobCard {
    id: String,
    name: String,
    status: String,
    status_label: String,
    progress: String,
    /// `0`–`100` for the progress track (#735): terminal states fill the bar,
    /// in-progress parses the percentage out of the recipient's X-Progress.
    progress_pct: String,
    error: String,
    file_count: usize,
    files: Vec<(String, String)>,
    elapsed: String,
    can_delete: bool,
}

fn progress_pct(status: &str, progress: &str) -> String {
    if matches!(status, "complete" | "failed" | "cancelled") {
        return "100".to_string();
    }
    let Some(idx) = progress.find('%') else {
        return "0".to_string();
    };
    let digits: String = progress[..idx]
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let pct: String = digits.chars().rev().collect();
    if pct.is_empty() { "0".to_string() } else { pct }
}

fn status_label(i18n: &I18n, status: &str) -> String {
    match status {
        "complete" => i18n.t("bulk-export-status-complete"),
        "failed" => i18n.t("bulk-export-status-failed"),
        "cancelled" => i18n.t("bulk-export-status-cancelled"),
        _ => i18n.t("bulk-export-status-in-progress"),
    }
}

/// `finished - started` as `5m 08s`, when both stamps parse.
fn elapsed(job: &ExportJob) -> String {
    let (Ok(start), Ok(end)) = (
        chrono::DateTime::parse_from_rfc3339(&job.started_at),
        chrono::DateTime::parse_from_rfc3339(&job.finished_at),
    ) else {
        return String::new();
    };
    let secs = (end - start).num_seconds().max(0);
    format!("{}m {:02}s", secs / 60, secs % 60)
}

fn job_card(i18n: &I18n, id: &str, job: &ExportJob, state: &WebState, tenant: &str) -> JobCard {
    JobCard {
        id: id.to_string(),
        name: if job.name.is_empty() {
            job.scope.clone()
        } else {
            job.name.clone()
        },
        status_label: status_label(i18n, &job.status),
        status: job.status.clone(),
        progress_pct: progress_pct(&job.status, &job.progress),
        progress: job.progress.clone(),
        error: job.error.clone(),
        file_count: job.files.len(),
        files: job
            .files
            .iter()
            .filter_map(|f| {
                Some((
                    f.get("type")?.as_str()?.to_string(),
                    f.get("url")?.as_str()?.to_string(),
                ))
            })
            .collect(),
        elapsed: elapsed(job),
        can_delete: terminal_status(&job.status)
            && remote_job_identity(job, state, tenant) != RemoteJobIdentity::Unknown,
    }
}

#[derive(Template)]
#[template(path = "pages/bulk-export.html")]
struct BulkExportPage {
    status: crate::Status,
    i18n: I18n,
    active_page: &'static str,
    available: bool,
    resource_types: Vec<String>,
    error: Option<String>,
    name_error: Option<String>,
    since_custom_error: Option<String>,
    form: StartForm,
    rejected: bool,
    patient_value: String,
    patient_hint: String,
}

#[derive(Template)]
#[template(path = "pages/bulk-export-active.html")]
struct ActiveExportsPage {
    status: crate::Status,
    i18n: I18n,
    active_page: &'static str,
    available: bool,
    total: usize,
    running: usize,
    cards: Vec<JobCard>,
    delete_error: bool,
}

#[derive(Template)]
#[template(path = "partials/bulk_export_card.html")]
struct JobCardFragment {
    i18n: I18n,
    card: JobCard,
}

#[derive(Default, Deserialize)]
pub(crate) struct ActiveQuery {
    #[serde(rename = "delete-error")]
    delete_error: Option<String>,
}

struct PatientOption {
    value: String,
    label: String,
}

#[derive(Template)]
#[template(path = "partials/bulk_export_patient_options.html")]
struct PatientOptionsFragment {
    options: Vec<PatientOption>,
    message: String,
    error: bool,
    id_only: bool,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /ui/bulk-export/new` — the export builder.
pub async fn page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
) -> Response {
    bulk_export_page(
        &state,
        locale,
        rv.0,
        &rt,
        StartForm::initial(),
        StartErrors::default(),
        None,
    )
    .await
}

async fn bulk_export_page(
    state: &WebState,
    locale: RequestLocale,
    version: FhirVersion,
    rt: &RequestTenant,
    mut form: StartForm,
    errors: StartErrors,
    error: Option<String>,
) -> Response {
    let i18n = I18n::new(locale);
    let status = current_status(state, version, rt);
    let resource_types = state
        .compartments
        .resource_type_names(&rt.id, version)
        .await;
    form.normalize_for_view(&resource_types);
    let patient_value = form.patients.join("\n");
    let patient_hint = if state.patient_name_search.load(Ordering::Relaxed) {
        i18n.t("bulk-export-field-patients-hint")
    } else {
        i18n.t("bulk-export-field-patients-id-only-hint")
    };
    let rejected = errors.rejected || error.is_some();
    render(BulkExportPage {
        status,
        i18n,
        active_page: "bulk-export",
        available: state.settings.is_some(),
        resource_types,
        error,
        name_error: errors.name,
        since_custom_error: errors.since_custom,
        form,
        rejected,
        patient_value,
        patient_hint,
    })
}

/// The export form. Parsed by hand: the resource-type checkboxes arrive as
/// repeated `types` fields, which `axum::Form`'s serde_urlencoded rejects.
#[derive(Default)]
pub struct StartForm {
    pub name: String,
    pub scope: String,
    pub group_id: String,
    pub all_types: bool,
    pub types: Vec<String>,
    pub elements: String,
    pub type_filter: String,
    pub since_preset: String,
    pub since_custom: String,
    pub patients: Vec<String>,
}

impl StartForm {
    fn initial() -> Self {
        Self {
            scope: "system".to_string(),
            all_types: true,
            ..Default::default()
        }
    }

    fn normalize_for_view(&mut self, resource_types: &[String]) {
        if !matches!(self.scope.as_str(), "system" | "patient" | "group") {
            self.scope = "system".to_string();
        }
        if !matches!(
            self.since_preset.as_str(),
            "" | "day" | "week" | "month" | "custom"
        ) {
            self.since_preset.clear();
        }
        let mut seen = HashSet::new();
        self.types.retain(|resource_type| {
            resource_types.contains(resource_type) && seen.insert(resource_type.clone())
        });
    }

    fn has_type(&self, resource_type: &str) -> bool {
        self.types.iter().any(|selected| selected == resource_type)
    }
}

#[derive(Default)]
struct StartErrors {
    name: Option<String>,
    since_custom: Option<String>,
    rejected: bool,
}

fn parse_start_form(body: &str) -> StartForm {
    let mut form = StartForm::default();
    for (key, value) in form_urlencoded::parse(body.as_bytes()) {
        let value = value.into_owned();
        match key.as_ref() {
            "name" => form.name = value,
            "scope" => form.scope = value,
            "group_id" => form.group_id = value,
            // Presence is the HTML checkbox contract. Its value is deliberately
            // ignored so field order and manipulated payloads cannot make
            // individual types override "All Resources".
            "all_types" => form.all_types = true,
            "types" => form.types.push(value),
            "elements" => form.elements = value,
            "type_filter" => form.type_filter = value,
            "since_preset" => form.since_preset = value,
            "since_custom" => form.since_custom = value,
            "patient" => form.patients.push(value),
            _ => {}
        }
    }
    form
}

/// Maps the Since preset (or a valid, non-empty custom stamp) onto `_since`.
fn has_fhir_r4_instant_lexical_form(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return false;
    }

    let number = |start: usize, end: usize| {
        bytes
            .get(start..end)?
            .iter()
            .try_fold(0_u32, |value, byte| {
                byte.is_ascii_digit()
                    .then_some(value * 10 + u32::from(*byte - b'0'))
            })
    };
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        number(0, 4),
        number(5, 7),
        number(8, 10),
        number(11, 13),
        number(14, 16),
        number(17, 19),
    ) else {
        return false;
    };
    if year == 0
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return false;
    }

    let mut zone_start = 19;
    if bytes.get(zone_start) == Some(&b'.') {
        zone_start += 1;
        let fraction_start = zone_start;
        while bytes.get(zone_start).is_some_and(u8::is_ascii_digit) {
            zone_start += 1;
        }
        if zone_start == fraction_start {
            return false;
        }
    }

    match bytes.get(zone_start) {
        Some(b'Z') => zone_start + 1 == bytes.len(),
        Some(b'+') | Some(b'-') => {
            if bytes.len() != zone_start + 6 || bytes.get(zone_start + 3) != Some(&b':') {
                return false;
            }
            let (Some(offset_hour), Some(offset_minute)) = (
                number(zone_start + 1, zone_start + 3),
                number(zone_start + 4, zone_start + 6),
            ) else {
                return false;
            };
            offset_minute <= 59 && (offset_hour <= 13 || (offset_hour == 14 && offset_minute == 0))
        }
        _ => false,
    }
}

fn since_instant(preset: &str, custom: &str) -> Result<String, ()> {
    let ago = |d: Duration| (Utc::now() - d).to_rfc3339_opts(SecondsFormat::Secs, true);
    match preset {
        "day" => Ok(ago(Duration::days(1))),
        "week" => Ok(ago(Duration::days(7))),
        "month" => Ok(ago(Duration::weeks(4))),
        "custom" => {
            let custom = custom.trim();
            if custom.is_empty() {
                Ok(String::new())
            } else {
                has_fhir_r4_instant_lexical_form(custom)
                    .then_some(())
                    .ok_or(())
                    .and_then(|_| chrono::DateTime::parse_from_rfc3339(custom).map_err(|_| ()))
                    .map(|_| custom.to_string())
            }
        }
        _ => Ok(String::new()),
    }
}

fn canonical_patient_ref(value: &str) -> Option<String> {
    let value = value.trim();
    let id = value.strip_prefix("Patient/").unwrap_or(value);
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return None;
    }
    Some(format!("Patient/{id}"))
}

fn parse_patient_refs(values: &[String]) -> Result<Vec<String>, ()> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    for raw in values {
        for candidate in raw.split([',', '\n', '\r']) {
            let candidate = candidate.trim();
            if candidate.is_empty() {
                continue;
            }
            let reference = canonical_patient_ref(candidate).ok_or(())?;
            if seen.insert(reference.clone()) {
                refs.push(reference);
            }
        }
    }
    Ok(refs)
}

fn fhir_json(version: FhirVersion) -> String {
    format!(
        "application/fhir+json; fhirVersion={}",
        version.as_mime_param()
    )
}

fn patient_option(resource: &Value) -> Option<PatientOption> {
    if resource.get("resourceType").and_then(Value::as_str) != Some("Patient") {
        return None;
    }
    let id = resource.get("id")?.as_str()?;
    let value = canonical_patient_ref(id)?;
    let name = resource
        .get("name")
        .and_then(Value::as_array)
        .and_then(|names| names.iter().find_map(human_name_label));
    let label = match name {
        Some(name) if !name.is_empty() => format!("{name} — {value}"),
        _ => value.clone(),
    };
    Some(PatientOption { value, label })
}

fn human_name_label(name: &Value) -> Option<String> {
    if let Some(text) = name.get("text").and_then(Value::as_str) {
        let text = text.trim();
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    let given = name
        .get("given")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|part| !part.is_empty());
    let family = name
        .get("family")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|part| !part.is_empty());
    let parts: Vec<&str> = given.chain(family).collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn options_response(fragment: PatientOptionsFragment) -> Response {
    let mut response = render(fragment);
    response.headers_mut().insert(
        CACHE_CONTROL,
        "private, no-store"
            .parse()
            .expect("static cache-control value"),
    );
    response
}

fn patient_lookup_error(i18n: &I18n, id_only: bool) -> Response {
    options_response(PatientOptionsFragment {
        options: Vec::new(),
        message: i18n.t("ui-combobox-error"),
        error: true,
        id_only,
    })
}

fn patient_search_options(bundle: &Value) -> Option<Vec<PatientOption>> {
    let bundle = bundle.as_object()?;
    if bundle.get("resourceType").and_then(Value::as_str) != Some("Bundle")
        || bundle.get("type").and_then(Value::as_str) != Some("searchset")
    {
        return None;
    }
    let entries = match bundle.get("entry") {
        None => return Some(Vec::new()),
        Some(Value::Array(entries)) => entries,
        Some(_) => return None,
    };
    entries
        .iter()
        .map(|entry| patient_option(entry.as_object()?.get("resource")?))
        .collect()
}

fn append_patient_options(
    source: Vec<PatientOption>,
    options: &mut Vec<PatientOption>,
    seen: &mut HashSet<String>,
) {
    for option in source {
        if options.len() >= 8 {
            break;
        }
        if seen.insert(option.value.clone()) {
            options.push(option);
        }
    }
}

/// `POST /ui/bulk-export/patient-options` — a small HTML result fragment for
/// the progressively-enhanced Patient selector.
pub async fn patient_options(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    HxRequest(is_htmx): HxRequest,
    headers: HeaderMap,
    axum::extract::RawForm(body): axum::extract::RawForm,
) -> Response {
    if !is_htmx {
        return Redirect::to("/ui/bulk-export/new").into_response();
    }
    let i18n = I18n::new(locale);
    let q = form_urlencoded::parse(&body)
        .find(|(key, _)| key == "q")
        .map(|(_, value)| value.trim().to_string())
        .unwrap_or_default();
    let id_only = !state.patient_name_search.load(Ordering::Relaxed);
    if q.is_empty() {
        return options_response(PatientOptionsFragment {
            options: Vec::new(),
            message: String::new(),
            error: false,
            id_only,
        });
    }
    if q.chars().count() > 64 {
        return patient_lookup_error(&i18n, id_only);
    }

    let Ok(client) = no_redirect_client() else {
        return patient_lookup_error(&i18n, id_only);
    };
    let media = fhir_json(rv.0);
    let exact_ref = canonical_patient_ref(&q);
    let mut options = Vec::new();
    let mut seen = HashSet::new();

    if let Some(reference) = &exact_ref {
        let id = reference.trim_start_matches("Patient/");
        let Ok(url) = internal_api_url(&state, &rt.id, ["Patient", id]) else {
            return patient_lookup_error(&i18n, id_only);
        };
        let request = forward_identity(
            client
                .get(url)
                .header("Accept", &media)
                .timeout(std::time::Duration::from_secs(10)),
            &headers,
            &rt.id,
        );
        match request.send().await {
            Ok(response)
                if matches!(response.status(), StatusCode::NOT_FOUND | StatusCode::GONE) => {}
            Ok(response) if response.status().is_success() => {
                let Ok(resource) = response.json::<Value>().await else {
                    return patient_lookup_error(&i18n, id_only);
                };
                let Some(option) = patient_option(&resource) else {
                    return patient_lookup_error(&i18n, id_only);
                };
                if option.value != *reference {
                    return patient_lookup_error(&i18n, id_only);
                }
                seen.insert(option.value.clone());
                options.push(option);
            }
            Ok(_) | Err(_) => return patient_lookup_error(&i18n, id_only),
        }
    }

    let search_patients = q.chars().count() >= 2 && !q.starts_with("Patient/") && !id_only;
    let mut downgraded = id_only;
    if search_patients {
        let Ok(url) = internal_api_url(&state, &rt.id, ["Patient", "_search"]) else {
            return patient_lookup_error(&i18n, false);
        };
        let identifier_request = forward_identity(
            client
                .post(url.clone())
                .header("Accept", &media)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .form(&[
                    ("identifier", q.as_str()),
                    ("_count", "9"),
                    ("_elements", "id,name"),
                ])
                .timeout(std::time::Duration::from_secs(10)),
            &headers,
            &rt.id,
        );
        let name_request = forward_identity(
            client
                .post(url)
                .header("Accept", &media)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .form(&[
                    ("name", q.as_str()),
                    ("_count", "9"),
                    ("_elements", "id,name"),
                ])
                .timeout(std::time::Duration::from_secs(10)),
            &headers,
            &rt.id,
        );
        let (identifier_result, name_result) =
            zip(identifier_request.send(), name_request.send()).await;

        let not_implemented = matches!(
            &identifier_result,
            Ok(response) if response.status() == StatusCode::NOT_IMPLEMENTED
        ) || matches!(
            &name_result,
            Ok(response) if response.status() == StatusCode::NOT_IMPLEMENTED
        );
        if not_implemented {
            state.patient_name_search.store(false, Ordering::Relaxed);
            downgraded = true;
        } else {
            let (Ok(identifier_response), Ok(name_response)) = (identifier_result, name_result)
            else {
                return patient_lookup_error(&i18n, false);
            };
            if !identifier_response.status().is_success() || !name_response.status().is_success() {
                return patient_lookup_error(&i18n, false);
            }
            let Ok(identifier_bundle) = identifier_response.json::<Value>().await else {
                return patient_lookup_error(&i18n, false);
            };
            let Ok(name_bundle) = name_response.json::<Value>().await else {
                return patient_lookup_error(&i18n, false);
            };
            let Some(identifier_options) = patient_search_options(&identifier_bundle) else {
                return patient_lookup_error(&i18n, false);
            };
            let Some(name_options) = patient_search_options(&name_bundle) else {
                return patient_lookup_error(&i18n, false);
            };
            append_patient_options(identifier_options, &mut options, &mut seen);
            append_patient_options(name_options, &mut options, &mut seen);
        }
    }

    let message = if options.is_empty() {
        i18n.t("bulk-export-patient-options-empty")
    } else {
        String::new()
    };
    options_response(PatientOptionsFragment {
        options,
        message,
        error: false,
        id_only: downgraded,
    })
}

/// `POST /ui/bulk-export` — kick off the export, then land on Exports.
pub async fn start(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    headers: HeaderMap,
    axum::extract::RawForm(body): axum::extract::RawForm,
) -> Response {
    let form = parse_start_form(&String::from_utf8_lossy(&body));
    let scope = match form.scope.as_str() {
        "patient" | "group" => form.scope.clone(),
        _ => "system".to_string(),
    };
    let patient_refs = if scope == "patient" {
        parse_patient_refs(&form.patients)
    } else {
        Ok(Vec::new())
    };
    let since = since_instant(&form.since_preset, &form.since_custom);
    let i18n = I18n::new(locale);
    let errors = StartErrors {
        name: form
            .name
            .trim()
            .is_empty()
            .then(|| i18n.t("bulk-export-name-required")),
        since_custom: since.is_err().then(|| i18n.t("bulk-export-since-invalid")),
        rejected: true,
    };
    let patient_error = patient_refs
        .is_err()
        .then(|| i18n.t("bulk-export-patient-invalid"));
    if errors.name.is_some() || errors.since_custom.is_some() || patient_error.is_some() {
        let mut response =
            bulk_export_page(&state, locale, rv.0, &rt, form, errors, patient_error).await;
        *response.status_mut() = StatusCode::BAD_REQUEST;
        return response;
    }

    let patient_refs = patient_refs.expect("patient references were validated");
    let since = since.expect("custom instant was validated");
    let user_key = settings_user_key(principal.as_deref());
    let snapshot = load_jobs(&state, &user_key, &rt.id).await;
    let mut job = ExportJob {
        name: form.name.trim().to_string(),
        scope,
        group_id: form.group_id.trim().to_string(),
        types: if form.all_types {
            String::new()
        } else {
            form.types.join(",")
        },
        elements: form.elements.trim().to_string(),
        type_filter: form.type_filter.trim().to_string(),
        since,
        patient_refs,
        fhir_version: Some(rv.0),
        status: "in-progress".to_string(),
        started_at: now_stamp(),
        ..Default::default()
    };
    let id = uuid::Uuid::new_v4().to_string();
    kickoff(&state, &mut job, &headers, &rt.id).await;
    if store_job_conditionally(
        &state,
        &user_key,
        &rt.id,
        &id,
        &job,
        snapshot.version,
        MemberExpectation::Absent,
    )
    .await
    .is_err()
    {
        cleanup_or_record_recovery(&state, &user_key, &rt.id, &job, &headers).await;
    }
    Redirect::to("/ui/bulk-export").into_response()
}

/// Performs the `$export` kick-off self-call, recording the poll URL or the
/// failure on the job.
async fn kickoff(state: &WebState, job: &mut ExportJob, headers: &HeaderMap, tenant: &str) {
    job.remote_job = RemoteJobProvenance::Unknown;
    job.remote_job_id.clear();
    job.poll_url.clear();
    let path = match job.scope.as_str() {
        "patient" => internal_api_url(state, tenant, ["Patient", "$export"]),
        "group" => internal_api_url(state, tenant, ["Group", job.group_id.as_str(), "$export"]),
        _ => internal_api_url(state, tenant, ["$export"]),
    };
    let path = match path {
        Ok(path) => path,
        Err(e) => {
            job.status = "failed".to_string();
            job.error = e;
            return;
        }
    };
    let version = job.fhir_version.unwrap_or(state.fhir_version);
    let media = fhir_json(version);
    let mut query: Vec<(&str, &str)> = Vec::new();
    if !job.types.is_empty() {
        query.push(("_type", &job.types));
    }
    if !job.elements.is_empty() {
        query.push(("_elements", &job.elements));
    }
    if !job.type_filter.is_empty() {
        query.push(("_typeFilter", &job.type_filter));
    }
    if !job.since.is_empty() {
        query.push(("_since", &job.since));
    }
    let client = match no_redirect_client() {
        Ok(client) => client,
        Err(e) => {
            job.status = "failed".to_string();
            job.error = e;
            return;
        }
    };
    let builder = if job.scope == "patient" && !job.patient_refs.is_empty() {
        let mut parameters = Vec::new();
        for (name, value) in &query {
            if *name == "_since" {
                parameters.push(json!({ "name": name, "valueInstant": value }));
            } else {
                parameters.push(json!({ "name": name, "valueString": value }));
            }
        }
        for reference in &job.patient_refs {
            parameters.push(json!({
                "name": "patient",
                "valueReference": { "reference": reference }
            }));
        }
        client
            .post(path.clone())
            .header("Content-Type", &media)
            .json(&json!({
                "resourceType": "Parameters",
                "parameter": parameters
            }))
    } else {
        client.get(path).query(&query)
    };
    let request = forward_identity(
        builder
            .header("Accept", &media)
            .header("Prefer", "respond-async")
            .timeout(std::time::Duration::from_secs(15)),
        headers,
        tenant,
    );
    match request.send().await {
        Ok(response) if response.status() == StatusCode::ACCEPTED => {
            match response
                .headers()
                .get("content-location")
                .and_then(|v| v.to_str().ok())
            {
                Some(poll) => match trusted_kickoff_job_id(state, tenant, poll) {
                    Some(id) => {
                        job.remote_job = RemoteJobProvenance::Known;
                        job.remote_job_id = id.as_hyphenated().to_string();
                        job.poll_url = public_status_url(state, tenant, &id)
                            .map(|url| url.to_string())
                            .unwrap_or_default();
                    }
                    None => {
                        job.status = "failed".to_string();
                        job.error = "kick-off returned an invalid Content-Location".to_string();
                    }
                },
                None => {
                    job.status = "failed".to_string();
                    job.error = "kick-off accepted without a Content-Location".to_string();
                }
            }
        }
        Ok(response) => {
            job.remote_job = RemoteJobProvenance::NoRemoteJob;
            let code = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            job.status = "failed".to_string();
            job.error = format!("kick-off answered {code}: {}", response_diagnostics(&body));
        }
        Err(e) => {
            job.remote_job = RemoteJobProvenance::Unknown;
            job.status = "failed".to_string();
            job.error = e.to_string();
        }
    }
}

async fn cleanup_kickoff_job(
    state: &WebState,
    job: &ExportJob,
    headers: &HeaderMap,
    tenant: &str,
) -> Result<(), String> {
    let RemoteJobIdentity::Known(remote_id) = remote_job_identity(job, state, tenant) else {
        return Ok(());
    };
    let client = no_redirect_client()?;
    let url = status_url(state, tenant, &remote_id)?;
    let mut last_error = String::new();
    for attempt in 0..SETTINGS_CAS_ATTEMPTS {
        let response = forward_identity(
            client
                .delete(url.clone())
                .timeout(std::time::Duration::from_secs(10)),
            headers,
            tenant,
        )
        .send()
        .await;
        match response {
            Ok(response)
                if response.status().is_success() || response.status() == StatusCode::NOT_FOUND =>
            {
                return Ok(());
            }
            Ok(response)
                if response.status().is_server_error()
                    || matches!(
                        response.status(),
                        StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
                    ) =>
            {
                last_error = format!("cleanup answered {}", response.status());
            }
            Ok(response) => return Err(format!("cleanup answered {}", response.status())),
            Err(error) => last_error = format!("cleanup request failed: {error}"),
        }
        if attempt + 1 < SETTINGS_CAS_ATTEMPTS {
            tokio::time::sleep(std::time::Duration::from_millis(25 * (attempt + 1) as u64)).await;
        }
    }
    Err(last_error)
}

async fn cleanup_or_record_recovery(
    state: &WebState,
    user_key: &str,
    tenant: &str,
    job: &ExportJob,
    headers: &HeaderMap,
) {
    let Err(cleanup_error) = cleanup_kickoff_job(state, job, headers, tenant).await else {
        return;
    };
    let mut recovery = job.clone();
    recovery.name = if job.name.is_empty() {
        "Export cleanup required".to_string()
    } else {
        format!("{} (cleanup required)", job.name)
    };
    recovery.status = "failed".to_string();
    recovery.error = format!(
        "Automatic cleanup failed ({cleanup_error}). Delete this card to retry remote cleanup."
    );
    recovery.progress.clear();
    recovery.files.clear();
    recovery.finished_at = now_stamp();
    let recovery_id = uuid::Uuid::new_v4().to_string();
    let snapshot = match load_jobs_checked(state, user_key, tenant).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::error!(
                remote_job_id = %job.remote_job_id,
                "failed to load settings for export cleanup recovery: {error}"
            );
            return;
        }
    };
    if let Err(error) = store_job_conditionally(
        state,
        user_key,
        tenant,
        &recovery_id,
        &recovery,
        snapshot.version,
        MemberExpectation::Absent,
    )
    .await
    {
        tracing::error!(
            remote_job_id = %job.remote_job_id,
            "failed to persist export cleanup recovery card: {error}"
        );
    }
}

fn response_diagnostics(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(body)
        && value.get("resourceType").and_then(Value::as_str) == Some("OperationOutcome")
    {
        let diagnostics: Vec<&str> = value
            .get("issue")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|issue| {
                issue
                    .get("diagnostics")
                    .and_then(Value::as_str)
                    .or_else(|| issue.pointer("/details/text").and_then(Value::as_str))
            })
            .collect();
        if !diagnostics.is_empty() {
            return diagnostics.join("; ").chars().take(300).collect();
        }
    }
    body.replace('\n', " ").chars().take(300).collect()
}

/// `GET /ui/bulk-export` — the job list.
pub async fn active(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    Query(query): Query<ActiveQuery>,
) -> Response {
    let i18n = I18n::new(locale);
    let status = current_status(&state, rv.0, &rt);
    let available = state.settings.is_some();
    let user_key = settings_user_key(principal.as_deref());
    let jobs = if available {
        load_jobs(&state, &user_key, &rt.id).await
    } else {
        JobsSnapshot::default()
    };
    let mut entries: Vec<(String, ExportJob)> = jobs
        .jobs
        .iter()
        .map(|(id, v)| (id.clone(), parse_job(v)))
        .collect();
    entries.sort_by(|a, b| b.1.started_at.cmp(&a.1.started_at));
    let running = entries
        .iter()
        .filter(|(_, j)| j.status == "in-progress")
        .count();
    let cards = entries
        .iter()
        .map(|(id, j)| job_card(&i18n, id, j, &state, &rt.id))
        .collect();
    render(ActiveExportsPage {
        status,
        i18n: I18n::new(locale),
        active_page: "bulk-export",
        available,
        total: entries.len(),
        running,
        cards,
        delete_error: matches!(
            query.delete_error.as_deref(),
            Some("remote") | Some("local")
        ),
    })
}

/// `GET /ui/bulk-export/active` — permanent compatibility redirect.
pub async fn active_redirect() -> Redirect {
    Redirect::permanent("/ui/bulk-export")
}

/// `GET /ui/bulk-export/active/{id}/card` — one poll, then the refreshed card.
pub async fn card(
    State(state): State<WebState>,
    locale: RequestLocale,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let i18n = I18n::new(locale);
    let user_key = settings_user_key(principal.as_deref());
    let snapshot = load_jobs(&state, &user_key, &rt.id).await;
    let Some(original) = snapshot.jobs.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut job = parse_job(original);
    if job.status == "in-progress" {
        poll_job(&state, &mut job, &headers, &rt.id).await;
        let _ = store_job_conditionally(
            &state,
            &user_key,
            &rt.id,
            &id,
            &job,
            snapshot.version,
            MemberExpectation::Unchanged(original),
        )
        .await;
    }
    let card = job_card(&i18n, &id, &job, &state, &rt.id);
    render(JobCardFragment { i18n, card })
}

/// One poll of the export status endpoint.
async fn poll_job(state: &WebState, job: &mut ExportJob, headers: &HeaderMap, tenant: &str) {
    let RemoteJobIdentity::Known(remote_id) = remote_job_identity(job, state, tenant) else {
        job.status = "failed".to_string();
        job.error = "status poll unavailable: remote job identity is unknown".to_string();
        return;
    };
    let Ok(url) = status_url(state, tenant, &remote_id) else {
        job.status = "failed".to_string();
        job.error = "status poll unavailable: invalid HFS base URL".to_string();
        return;
    };
    let Ok(client) = no_redirect_client() else {
        job.status = "failed".to_string();
        job.error = "status poll unavailable: HTTP client setup failed".to_string();
        return;
    };
    let media = fhir_json(job.fhir_version.unwrap_or(state.fhir_version));
    let request = forward_identity(
        client
            .get(url)
            .header("Accept", media)
            .timeout(std::time::Duration::from_secs(10)),
        headers,
        tenant,
    );
    let response = match request.send().await {
        Ok(r) => r,
        Err(e) => {
            job.status = "failed".to_string();
            job.error = format!("status poll failed: {e}");
            return;
        }
    };
    match response.status().as_u16() {
        202 => {
            job.progress = response
                .headers()
                .get("x-progress")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("in progress")
                .to_string();
        }
        200 => {
            let manifest: Value = response.json().await.unwrap_or(Value::Null);
            job.files = manifest["output"].as_array().cloned().unwrap_or_default();
            job.status = "complete".to_string();
            job.finished_at = now_stamp();
            job.progress = String::new();
        }
        code => {
            let mut body = response.text().await.unwrap_or_default();
            body.truncate(300);
            job.status = "failed".to_string();
            job.error = format!("{code}: {}", body.replace('\n', " "));
        }
    }
}

/// `POST /ui/bulk-export/active/{id}/cancel` — DELETE against the poll URL.
pub async fn cancel(
    State(state): State<WebState>,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    let snapshot = load_jobs(&state, &user_key, &rt.id).await;
    if let Some(original) = snapshot.jobs.get(&id) {
        let mut job = parse_job(original);
        if let RemoteJobIdentity::Known(remote_id) = remote_job_identity(&job, &state, &rt.id)
            && let (Ok(client), Ok(url)) =
                (no_redirect_client(), status_url(&state, &rt.id, &remote_id))
        {
            let request = forward_identity(
                client
                    .delete(url)
                    .timeout(std::time::Duration::from_secs(10)),
                &headers,
                &rt.id,
            );
            let _ = request.send().await;
        }
        job.status = "cancelled".to_string();
        job.finished_at = now_stamp();
        job.progress = String::new();
        let _ = store_job_conditionally(
            &state,
            &user_key,
            &rt.id,
            &id,
            &job,
            snapshot.version,
            MemberExpectation::Unchanged(original),
        )
        .await;
    }
    Redirect::to("/ui/bulk-export").into_response()
}

/// `POST /ui/bulk-export/active/{id}/retry` — same parameters, fresh kick-off.
pub async fn retry(
    State(state): State<WebState>,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    let snapshot = load_jobs(&state, &user_key, &rt.id).await;
    if let Some(original) = snapshot.jobs.get(&id) {
        let mut job = parse_job(original);
        job.status = "in-progress".to_string();
        job.error = String::new();
        job.progress = String::new();
        job.files = Vec::new();
        job.poll_url = String::new();
        job.finished_at = String::new();
        job.started_at = now_stamp();
        kickoff(&state, &mut job, &headers, &rt.id).await;
        if store_job_conditionally(
            &state,
            &user_key,
            &rt.id,
            &id,
            &job,
            snapshot.version,
            MemberExpectation::Unchanged(original),
        )
        .await
        .is_err()
        {
            cleanup_or_record_recovery(&state, &user_key, &rt.id, &job, &headers).await;
        }
    }
    Redirect::to("/ui/bulk-export").into_response()
}

fn terminal_status(status: &str) -> bool {
    matches!(status, "complete" | "failed" | "cancelled")
}

fn delete_error_redirect(kind: &str) -> Response {
    Redirect::to(&format!("/ui/bulk-export?delete-error={kind}")).into_response()
}

/// `POST /ui/bulk-export/active/{id}/delete` — reclaim server output, then
/// remove exactly one terminal UI record.
pub async fn delete(
    State(state): State<WebState>,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    let snapshot = load_jobs(&state, &user_key, &rt.id).await;
    let Some(original) = snapshot.jobs.get(&id) else {
        return Redirect::to("/ui/bulk-export").into_response();
    };
    let job = parse_job(original);
    if !terminal_status(&job.status) {
        return Redirect::to("/ui/bulk-export").into_response();
    }

    match remote_job_identity(&job, &state, &rt.id) {
        RemoteJobIdentity::Known(remote_id) => {
            let (Ok(client), Ok(url)) =
                (no_redirect_client(), status_url(&state, &rt.id, &remote_id))
            else {
                return delete_error_redirect("remote");
            };
            let response = forward_identity(
                client
                    .delete(url)
                    .timeout(std::time::Duration::from_secs(10)),
                &headers,
                &rt.id,
            )
            .send()
            .await;
            match response {
                Ok(response)
                    if response.status().is_success()
                        || response.status() == StatusCode::NOT_FOUND => {}
                _ => return delete_error_redirect("remote"),
            }
        }
        RemoteJobIdentity::NoRemoteJob => {}
        RemoteJobIdentity::Unknown => return delete_error_redirect("remote"),
    }

    match remove_job_conditionally(&state, &user_key, &rt.id, &id, original, snapshot.version).await
    {
        Ok(()) => Redirect::to("/ui/bulk-export").into_response(),
        Err(_) => delete_error_redirect("local"),
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FreshManifest {
    requires_access_token: bool,
    output: Vec<FreshOutput>,
}

#[derive(Debug, Clone, Deserialize)]
struct FreshOutput {
    #[serde(rename = "type")]
    resource_type: String,
    url: String,
}

#[derive(Debug, Clone)]
struct ZipPlanEntry {
    resource_type: String,
    ordinal: usize,
    filename: String,
    authenticated_url: Option<reqwest::Url>,
}

async fn fetch_fresh_manifest(
    state: &WebState,
    headers: &HeaderMap,
    tenant: &str,
    remote_id: &uuid::Uuid,
) -> Result<FreshManifest, String> {
    let client = no_redirect_client()?;
    let url = status_url(state, tenant, remote_id)?;
    let response = forward_identity(
        client
            .get(url)
            .header("Accept", "application/fhir+json")
            .timeout(std::time::Duration::from_secs(15)),
        headers,
        tenant,
    )
    .send()
    .await
    .map_err(|e| e.to_string())?;
    if response.status() != StatusCode::OK {
        return Err(format!("manifest answered {}", response.status()));
    }
    let manifest = response
        .json::<FreshManifest>()
        .await
        .map_err(|e| format!("invalid export manifest: {e}"))?;
    if manifest.output.is_empty() {
        return Err("export manifest has no output".to_string());
    }
    Ok(manifest)
}

fn trusted_output_url(
    state: &WebState,
    tenant: &str,
    remote_id: &uuid::Uuid,
    advertised: &str,
) -> Result<reqwest::Url, String> {
    let advertised = parse_url_without_credentials(advertised)
        .ok_or_else(|| "invalid authenticated output URL".to_string())?;
    let segments: Vec<&str> = advertised
        .path_segments()
        .ok_or_else(|| "authenticated output URL has no path".to_string())?
        .collect();
    let remote_id_text = remote_id.as_hyphenated().to_string();
    let part = segments
        .last()
        .copied()
        .filter(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
        .ok_or_else(|| "invalid authenticated output part".to_string())?;
    let advertised_expected = public_api_url(
        state,
        tenant,
        state.tenant_path_routing,
        ["export-file", remote_id_text.as_str(), part],
    )?;
    if advertised.path() != advertised_expected.path() {
        return Err("authenticated output URL is not this HFS job".to_string());
    }
    internal_api_url(
        state,
        tenant,
        ["export-file", remote_id_text.as_str(), part],
    )
}

fn external_output_url(advertised: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(advertised).map_err(|e| e.to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("invalid pre-signed output URL".to_string());
    }
    Ok(url)
}

fn safe_resource_name(resource_type: &str) -> String {
    let name: String = resource_type
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    if name.is_empty() {
        "Resource".to_string()
    } else {
        name
    }
}

fn unique_zip_filename(base: &str, ordinal: usize, used: &mut HashSet<String>) -> String {
    let stem = format!("{base}-{ordinal:04}");
    let mut collision = 1;
    loop {
        let suffix = if collision == 1 {
            String::new()
        } else {
            format!("-{collision:02}")
        };
        let candidate = format!("{stem}{suffix}.ndjson");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        collision += 1;
    }
}

fn zip_plan(
    state: &WebState,
    tenant: &str,
    remote_id: &uuid::Uuid,
    manifest: &FreshManifest,
) -> Result<Vec<ZipPlanEntry>, String> {
    let mut ordinals: HashMap<String, usize> = HashMap::new();
    let mut filenames = HashSet::new();
    manifest
        .output
        .iter()
        .map(|output| {
            let ordinal = ordinals.entry(output.resource_type.clone()).or_default();
            *ordinal += 1;
            let current = *ordinal;
            let authenticated_url = manifest
                .requires_access_token
                .then(|| trusted_output_url(state, tenant, remote_id, &output.url))
                .transpose()?;
            Ok(ZipPlanEntry {
                resource_type: output.resource_type.clone(),
                ordinal: current,
                filename: unique_zip_filename(
                    &safe_resource_name(&output.resource_type),
                    current,
                    &mut filenames,
                ),
                authenticated_url,
            })
        })
        .collect()
}

fn output_by_type_ordinal<'a>(
    manifest: &'a FreshManifest,
    resource_type: &str,
    ordinal: usize,
) -> Option<&'a FreshOutput> {
    manifest
        .output
        .iter()
        .filter(|output| output.resource_type == resource_type)
        .nth(ordinal.saturating_sub(1))
}

async fn stream_zip(
    state: WebState,
    headers: HeaderMap,
    tenant: String,
    remote_id: uuid::Uuid,
    plan: Vec<ZipPlanEntry>,
    writer: tokio::io::DuplexStream,
) -> Result<(), String> {
    let client = no_redirect_client()?;
    let mut archive = ZipFileWriter::with_tokio(writer);
    for item in plan {
        let (url, send_identity) = match item.authenticated_url {
            Some(url) => (url, true),
            None => {
                // Pre-signed URLs expire. Re-poll immediately before every
                // sequential fetch and select the same type ordinal.
                let refreshed = fetch_fresh_manifest(&state, &headers, &tenant, &remote_id).await?;
                if refreshed.requires_access_token {
                    return Err("export manifest access posture changed".to_string());
                }
                let output = output_by_type_ordinal(&refreshed, &item.resource_type, item.ordinal)
                    .ok_or_else(|| "export manifest output changed".to_string())?;
                (external_output_url(&output.url)?, false)
            }
        };
        let request = client.get(url).header("Accept", "application/fhir+ndjson");
        let request = if send_identity {
            forward_identity(request, &headers, &tenant)
        } else {
            request
        };
        let mut response = request.send().await.map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("output answered {}", response.status()));
        }

        let entry = ZipEntryBuilder::new(item.filename.into(), Compression::Stored);
        let mut entry_writer = archive
            .write_entry_stream(entry)
            .await
            .map_err(|e| e.to_string())?;
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            entry_writer
                .write_all(&chunk)
                .await
                .map_err(|e| e.to_string())?;
        }
        entry_writer.close().await.map_err(|e| e.to_string())?;
    }
    archive.close().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// `GET /ui/bulk-export/active/{id}/download` — stream a fresh manifest's
/// complete output set as one ZIP. A late source failure terminates the body;
/// a concurrent Delete can therefore leave the client with a truncated ZIP.
pub async fn download_all(
    State(state): State<WebState>,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    let snapshot = load_jobs(&state, &user_key, &rt.id).await;
    let Some(job) = snapshot.jobs.get(&id).map(parse_job) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if job.status != "complete" || job.files.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let RemoteJobIdentity::Known(remote_id) = remote_job_identity(&job, &state, &rt.id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let manifest = match fetch_fresh_manifest(&state, &headers, &rt.id, &remote_id).await {
        Ok(manifest) => manifest,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };
    let plan = match zip_plan(&state, &rt.id, &remote_id, &manifest) {
        Ok(plan) if !plan.is_empty() => plan,
        _ => return StatusCode::BAD_GATEWAY.into_response(),
    };

    let (writer, mut reader) = tokio::io::duplex(64 * 1024);
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let stream_state = state.clone();
    let stream_headers = headers.clone();
    let tenant = rt.id.clone();
    tokio::spawn(async move {
        let result = stream_zip(
            stream_state,
            stream_headers,
            tenant,
            remote_id,
            plan,
            writer,
        )
        .await;
        let _ = done_tx.send(result);
    });

    let body_stream = async_stream::stream! {
        let mut buffer = vec![0_u8; 16 * 1024];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) => break,
                Ok(read) => yield Ok::<Bytes, io::Error>(Bytes::copy_from_slice(&buffer[..read])),
                Err(e) => {
                    yield Err(e);
                    return;
                }
            }
        }
        match done_rx.await {
            Ok(Ok(())) => {}
            Ok(Err(message)) => yield Err(io::Error::other(message)),
            Err(_) => yield Err(io::Error::other("ZIP stream task ended unexpectedly")),
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/zip")
        .header(
            "Content-Disposition",
            "attachment; filename=\"bulk-export.zip\"",
        )
        .header("Cache-Control", "no-store")
        .body(Body::from_stream(body_stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patient_references_accept_bare_and_canonical_ids_and_deduplicate() {
        let values = vec![" p-1,Patient/p-2\np-1 ".to_string()];
        assert_eq!(
            parse_patient_refs(&values),
            Ok(vec!["Patient/p-1".to_string(), "Patient/p-2".to_string()])
        );
        assert!(parse_patient_refs(&["Patient/not/valid".to_string()]).is_err());
    }

    #[test]
    fn legacy_jobs_deserialize_without_patient_or_version_fields() {
        let job: ExportJob = serde_json::from_value(json!({
            "name": "legacy",
            "scope": "patient",
            "status": "failed",
            "startedAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        assert!(job.patient_refs.is_empty());
        assert_eq!(job.fhir_version, None);
    }

    #[test]
    fn operation_outcome_diagnostics_surface_unknown_patient_details() {
        let body = json!({
            "resourceType": "OperationOutcome",
            "issue": [{"diagnostics": "Patient/missing was not found"}]
        })
        .to_string();
        assert_eq!(response_diagnostics(&body), "Patient/missing was not found");
    }
}
