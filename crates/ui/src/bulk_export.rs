//! Bulk Export workspace (`/ui/bulk-export`) — driving HFS's own `$export`
//! operation (#537).
//!
//! The pull-based companion to the Bulk Import workspace: the user picks a
//! scope (everything / patients / group), the resource types, and the
//! narrowing filters; the workspace kicks off the server's async `$export`,
//! then tracks the job on the Active Exports page — one server-side status
//! poll per htmx fetch, exactly like the import workspace's recipient
//! polling.
//!
//! Kick-offs and polls are self-calls: the target is this same server,
//! addressed via the request's `Host` header, with the caller's own
//! `Authorization` and tenant forwarded — so the export runs with the
//! user's credentials, not a service account.
//!
//! Job state lives in the per-user settings document under
//! `byTenant.<tenant>.bulkExport.jobs`, object-keyed by id.

use askama::Template;
use axum::{
    Extension,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use chrono::{Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
    /// `in-progress` | `complete` | `failed` | `cancelled`.
    #[serde(default)]
    pub status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub poll_url: String,
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

async fn load_jobs(
    state: &WebState,
    user_key: &str,
    tenant: &str,
) -> serde_json::Map<String, Value> {
    let Some(store) = &state.settings else {
        return serde_json::Map::new();
    };
    store
        .get_settings(user_key)
        .await
        .ok()
        .flatten()
        .and_then(|s| {
            s.document
                .get("byTenant")?
                .get(tenant)?
                .get("bulkExport")?
                .get("jobs")?
                .as_object()
                .cloned()
        })
        .unwrap_or_default()
}

async fn store_job(
    state: &WebState,
    user_key: &str,
    tenant: &str,
    id: &str,
    job: &ExportJob,
) -> Result<(), String> {
    let Some(store) = &state.settings else {
        return Err("settings store unavailable".to_string());
    };
    // Null first so dropped fields (files, error) don't survive the merge.
    let clear =
        json!({ "byTenant": { tenant: { "bulkExport": { "jobs": { id: Value::Null } } } } });
    store
        .patch_settings(user_key, clear, None)
        .await
        .map_err(|e| e.to_string())?;
    let value = serde_json::to_value(job).map_err(|e| e.to_string())?;
    let patch = json!({ "byTenant": { tenant: { "bulkExport": { "jobs": { id: value } } } } });
    store
        .patch_settings(user_key, patch, None)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn now_stamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_job(value: &Value) -> ExportJob {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

/// The FHIR API base of this same server, from the request's Host header.
fn self_base(headers: &HeaderMap) -> String {
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost:8080");
    format!("http://{host}")
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

fn job_card(i18n: &I18n, id: &str, job: &ExportJob) -> JobCard {
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
    active_count: usize,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/bulk-export-active.html")]
struct ActiveExportsPage {
    status: crate::Status,
    i18n: I18n,
    active_page: &'static str,
    total: usize,
    running: usize,
    cards: Vec<JobCard>,
}

#[derive(Template)]
#[template(path = "partials/bulk_export_card.html")]
struct JobCardFragment {
    i18n: I18n,
    card: JobCard,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /ui/bulk-export` — the export builder.
pub async fn page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
) -> Response {
    let i18n = I18n::new(locale);
    let status = current_status(&state, rv.0, &rt);
    let user_key = settings_user_key(principal.as_deref());
    let resource_types = state.compartments.resource_type_names(&rt.id, rv.0).await;
    let active_count = load_jobs(&state, &user_key, &rt.id)
        .await
        .values()
        .filter(|j| j["status"] == "in-progress")
        .count();
    render(BulkExportPage {
        status,
        i18n,
        active_page: "bulk-export",
        available: state.settings.is_some(),
        resource_types,
        active_count,
        error: None,
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
            _ => {}
        }
    }
    form
}

/// Maps the Since preset (or the custom stamp) onto an `_since` instant.
fn since_instant(preset: &str, custom: &str) -> String {
    let ago = |d: Duration| (Utc::now() - d).to_rfc3339_opts(SecondsFormat::Secs, true);
    match preset {
        "day" => ago(Duration::days(1)),
        "week" => ago(Duration::days(7)),
        "month" => ago(Duration::weeks(4)),
        "custom" => custom.trim().to_string(),
        _ => String::new(),
    }
}

/// `POST /ui/bulk-export` — kick off the export, then land on Active Exports.
pub async fn start(
    State(state): State<WebState>,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    headers: HeaderMap,
    axum::extract::RawForm(body): axum::extract::RawForm,
) -> Response {
    let form = parse_start_form(&String::from_utf8_lossy(&body));
    let user_key = settings_user_key(principal.as_deref());
    let mut job = ExportJob {
        name: form.name.trim().to_string(),
        scope: match form.scope.as_str() {
            "patient" | "group" => form.scope.clone(),
            _ => "system".to_string(),
        },
        group_id: form.group_id.trim().to_string(),
        types: if form.all_types {
            String::new()
        } else {
            form.types.join(",")
        },
        elements: form.elements.trim().to_string(),
        type_filter: form.type_filter.trim().to_string(),
        since: since_instant(&form.since_preset, &form.since_custom),
        status: "in-progress".to_string(),
        started_at: now_stamp(),
        ..Default::default()
    };
    let id = uuid::Uuid::new_v4().to_string();
    kickoff(&mut job, &headers, &rt.id).await;
    let _ = store_job(&state, &user_key, &rt.id, &id, &job).await;
    Redirect::to("/ui/bulk-export/active").into_response()
}

/// Performs the `$export` kick-off self-call, recording the poll URL or the
/// failure on the job.
async fn kickoff(job: &mut ExportJob, headers: &HeaderMap, tenant: &str) {
    let base = self_base(headers);
    let path = match job.scope.as_str() {
        "patient" => format!("{base}/Patient/$export"),
        "group" => format!("{base}/Group/{}/$export", job.group_id),
        _ => format!("{base}/$export"),
    };
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
    let request = forward_identity(
        reqwest::Client::new()
            .get(&path)
            .query(&query)
            .header("Accept", "application/fhir+json")
            .header("Prefer", "respond-async")
            .timeout(std::time::Duration::from_secs(15)),
        headers,
        tenant,
    );
    match request.send().await {
        Ok(response) if response.status().as_u16() == 202 => {
            match response
                .headers()
                .get("content-location")
                .and_then(|v| v.to_str().ok())
            {
                // A relative Content-Location is resolved against this server.
                Some(poll) if poll.starts_with('/') => {
                    job.poll_url = format!("{base}{poll}");
                }
                Some(poll) => job.poll_url = poll.to_string(),
                None => {
                    job.status = "failed".to_string();
                    job.error = "kick-off accepted without a Content-Location".to_string();
                }
            }
        }
        Ok(response) => {
            let code = response.status().as_u16();
            let mut body = response.text().await.unwrap_or_default();
            body.truncate(300);
            job.status = "failed".to_string();
            job.error = format!("kick-off answered {code}: {}", body.replace('\n', " "));
        }
        Err(e) => {
            job.status = "failed".to_string();
            job.error = e.to_string();
        }
    }
}

/// `GET /ui/bulk-export/active` — the job list.
pub async fn active(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
) -> Response {
    let i18n = I18n::new(locale);
    let status = current_status(&state, rv.0, &rt);
    let user_key = settings_user_key(principal.as_deref());
    let jobs = load_jobs(&state, &user_key, &rt.id).await;
    let mut entries: Vec<(String, ExportJob)> = jobs
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
        .map(|(id, j)| job_card(&i18n, id, j))
        .collect();
    render(ActiveExportsPage {
        status,
        i18n: I18n::new(locale),
        active_page: "bulk-export",
        total: entries.len(),
        running,
        cards,
    })
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
    let jobs = load_jobs(&state, &user_key, &rt.id).await;
    let Some(mut job) = jobs.get(&id).map(parse_job) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if job.status == "in-progress" && !job.poll_url.is_empty() {
        poll_job(&mut job, &headers, &rt.id).await;
        let _ = store_job(&state, &user_key, &rt.id, &id, &job).await;
    }
    let card = job_card(&i18n, &id, &job);
    render(JobCardFragment { i18n, card })
}

/// One poll of the export status endpoint.
async fn poll_job(job: &mut ExportJob, headers: &HeaderMap, tenant: &str) {
    let request = forward_identity(
        reqwest::Client::new()
            .get(&job.poll_url)
            .header("Accept", "application/fhir+json")
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
    let jobs = load_jobs(&state, &user_key, &rt.id).await;
    if let Some(mut job) = jobs.get(&id).map(parse_job) {
        if !job.poll_url.is_empty() {
            let request = forward_identity(
                reqwest::Client::new()
                    .delete(&job.poll_url)
                    .timeout(std::time::Duration::from_secs(10)),
                &headers,
                &rt.id,
            );
            let _ = request.send().await;
        }
        job.status = "cancelled".to_string();
        job.finished_at = now_stamp();
        job.progress = String::new();
        let _ = store_job(&state, &user_key, &rt.id, &id, &job).await;
    }
    Redirect::to("/ui/bulk-export/active").into_response()
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
    let jobs = load_jobs(&state, &user_key, &rt.id).await;
    if let Some(mut job) = jobs.get(&id).map(parse_job) {
        job.status = "in-progress".to_string();
        job.error = String::new();
        job.progress = String::new();
        job.files = Vec::new();
        job.poll_url = String::new();
        job.finished_at = String::new();
        job.started_at = now_stamp();
        kickoff(&mut job, &headers, &rt.id).await;
        let _ = store_job(&state, &user_key, &rt.id, &id, &job).await;
    }
    Redirect::to("/ui/bulk-export/active").into_response()
}
