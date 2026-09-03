//! SQL Export workspace (`/ui/sql/export`, #649/#833) — driving `$sql-export`
//! over the stored ViewDefinitions and SQL Queries/Views, list-first like the
//! Bulk Export workspace it mirrors.
//!
//! # The "client's notebook" model
//!
//! HFS's `$sql-export` implementation (`helios-sof`) has no server-side job
//! list: the one job controller is an in-memory, ownerless map that a reaper
//! empties of terminated jobs after 24 hours and that a server restart clears
//! entirely. There is nothing for this page to *list* by asking the server.
//!
//! So the list is the browser's own notebook instead: every kick-off writes a
//! record to the per-user settings document under
//! `byTenant.<tenant>.sqlExport.jobs`, keyed by a locally-generated UUID (see
//! [`ExportJob`]). A job the server has since reaped or restarted away from
//! reads back as `404`, which this module turns into `cancelled` with an
//! explanatory reason rather than treating it as an error — the reaper is
//! expected behavior, not a fault. Jobs kicked off outside this UI (direct API
//! calls) have no notebook entry and never appear here.
//!
//! Kick-off, polling, and the completion manifest all go through
//! [`ConformanceSource`]'s four `$sql-export` methods with the request's
//! [`Caller`] (#833): the browser's own bearer token when it sent one, the
//! configured outbound credential otherwise. This keeps the async job
//! attributable to the person who started it, matching the identity kick-off
//! and polling assume under SMART on FHIR.
//!
//! # The poll state machine
//!
//! [`poll_job`] is the single place that turns one [`SqlExportStatus`] answer
//! into a job mutation:
//!
//! | status answer | job mutation |
//! |---|---|
//! | `Running(progress)` | stays `in-progress`; `progress` updated (or cleared); `pollError` cleared |
//! | `Done` + manifest `Ok` | `complete`; `outputs` recorded; `finishedAt` stamped |
//! | `Done` + manifest `Err` | `failed`; `error` recorded; `finishedAt` stamped |
//! | `Unknown` (404) | `cancelled`; `error` set to a fixed, translated reason; `finishedAt` stamped |
//! | `Unavailable(message)` | stays `in-progress`; `pollError` set; `progress` untouched |
//!
//! A job polls at most once per request: the Active SQL Exports list polls
//! every `in-progress` job once before rendering (so a plain reload without
//! JavaScript stays current), and a card's own htmx fragment polls its single
//! job once. Every transition is persisted with the same optimistic-locking
//! CAS pattern Bulk Export's job store uses (`crates/ui/src/bulk_export.rs`):
//! a bounded number of read-modify-write attempts against the settings
//! document's version, so a poll that loses a race against a concurrent
//! removal of the same job never resurrects it.
//!
//! # Per-job actions (#833)
//!
//! Cancel, Retry, Run again, and Remove from list ([`cancel`], [`retry`],
//! [`rerun`], [`remove`]) are `POST`-only and always redirect back to the
//! list: an id this user/tenant does not own, or one whose current status
//! does not admit the action, is a silent no-op with the same redirect —
//! never a 404 or an error page, since these routes are only ever reached
//! through the card's own buttons.
//!
//! Retry and Run again never mutate the record they act on. Both go through
//! [`resubmit`], which builds a brand-new record (a fresh local id,
//! `started_at` reset, `job_id`/`progress`/`outputs` cleared) from the
//! original's `name`/`subjects`/`format` and kicks it off exactly like a
//! from-scratch submission — the same [`store_new_job`] tail [`start`] uses,
//! so a retried job's shape can never drift from a freshly submitted one.
//! Remove only ever deletes the local record; the server is never called; a
//! job's outputs are the reaper's to reclaim, not this UI's.

use askama::Template;
use axum::{
    Extension,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::i18n::{I18n, RequestLocale};
use crate::{
    Caller, RequestTenant, RequestVersion, SqlExportStatus, Status, WebState, current_status,
    render, settings_user_key,
};

// ---------------------------------------------------------------------------
// Completion-manifest view models (#649): shaping `$sql-export`'s finished
// `Parameters` for the Files page and for a job's persisted `outputs`.
// ---------------------------------------------------------------------------

/// One `output` entry of the manifest: a subject's name and its download
/// URL(s), one per shard.
pub(crate) struct ManifestOutput {
    pub name: String,
    pub locations: Vec<String>,
}

/// Every `output` parameter of the manifest, in manifest order.
pub(crate) fn manifest_outputs(manifest: &Value) -> Vec<ManifestOutput> {
    manifest
        .get("parameter")
        .and_then(Value::as_array)
        .map(|params| {
            params
                .iter()
                .filter(|p| p.get("name").and_then(Value::as_str) == Some("output"))
                .map(|p| {
                    let parts = p.get("part").and_then(Value::as_array);
                    let name = parts
                        .and_then(|parts| {
                            parts.iter().find_map(|part| {
                                (part.get("name").and_then(Value::as_str) == Some("name"))
                                    .then(|| part.get("valueString").and_then(Value::as_str))
                                    .flatten()
                            })
                        })
                        .unwrap_or_default()
                        .to_string();
                    let locations = parts
                        .map(|parts| {
                            parts
                                .iter()
                                .filter(|part| {
                                    part.get("name").and_then(Value::as_str) == Some("location")
                                })
                                .filter_map(|part| part.get("valueUri").and_then(Value::as_str))
                                .map(same_origin_location)
                                .collect()
                        })
                        .unwrap_or_default();
                    ManifestOutput { name, locations }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Rewrites an absolute `location` to a same-origin path (RFC 3986
/// path + query, dropping scheme/host/port): $sql-export's completion
/// manifest bakes in whatever `HFS_BASE_URL` the server was configured
/// with, which does not necessarily match the host and port the browser is
/// actually talking to (a reverse proxy, or simply a different port in
/// development) — but the UI and the FHIR API are served by the same
/// process, so a path-only link always resolves correctly regardless of
/// what the manifest's advertised base URL says (#833). Left unchanged when
/// `location` does not parse as an absolute URL (it is already a bare path,
/// or something this UI does not understand) — never invented, never
/// dropped.
fn same_origin_location(location: &str) -> String {
    match reqwest::Url::parse(location) {
        Ok(url) => match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_string(),
        },
        Err(_) => location.to_string(),
    }
}

/// A top-level manifest parameter's primitive value, whichever `value[x]` it
/// carries.
pub(crate) fn manifest_value(manifest: &Value, name: &str) -> Option<String> {
    manifest
        .get("parameter")?
        .as_array()?
        .iter()
        .find(|p| p.get("name").and_then(Value::as_str) == Some(name))
        .and_then(|p| {
            ["valueString", "valueCode", "valueUri", "valueInstant"]
                .iter()
                .find_map(|k| p.get(*k).and_then(Value::as_str))
                .map(String::from)
                .or_else(|| p.get("valueInteger").map(|v| v.to_string()))
        })
}

// ---------------------------------------------------------------------------
// Job model & store (#833)
// ---------------------------------------------------------------------------

/// One subject in a job's `subjects` list: what the checkbox on `/new`
/// referred to, resolved once at kick-off time against the same list `/new`
/// offered so the card never has to re-resolve a reference later.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobSubject {
    pub name: String,
    /// `ViewDefinition/{id}` or `Library/{id}`.
    pub reference: String,
    /// `view-definition` | `sql-query` | `sql-view` — a stable code, not the
    /// display label; the card meta line translates it back through i18n.
    pub kind: String,
}

/// One `output` entry of a persisted completion manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobOutput {
    pub name: String,
    pub locations: Vec<String>,
}

fn job_outputs(manifest: &Value) -> Vec<JobOutput> {
    manifest_outputs(manifest)
        .into_iter()
        .map(|output| JobOutput {
            name: output.name,
            locations: output.locations,
        })
        .collect()
}

/// One `$sql-export` job as the browser's notebook records it (#833) — see
/// the module docs for the model this shape assumes.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExportJob {
    /// The server's job id; empty when the kick-off never got one (a failed
    /// kick-off still leaves a `failed` card).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub job_id: String,
    /// Optional user-supplied name; the form does not collect one yet (#834),
    /// so this stays empty until then and the card falls back to the
    /// subjects' names.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<JobSubject>,
    /// `ndjson` | `csv` | `json` | `parquet`.
    #[serde(default)]
    pub format: String,
    /// `in-progress` | `complete` | `failed` | `cancelled`.
    #[serde(default)]
    pub status: String,
    /// The last `X-Progress` seen (e.g. `35%`); empty in terminal states.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub progress: String,
    /// A failure message (`failed`), or the reason a job was declared
    /// `cancelled` (e.g. the 404 reaper explanation).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    /// The most recent *transient* poll problem (`Unavailable`); cleared on
    /// the next successful poll. Distinct from `error`: the job stays
    /// `in-progress` while this is set.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub poll_error: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub finished_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<JobOutput>,
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
        .and_then(|value| value.get("sqlExport"))
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

/// An explicit merge value for every job field. Nulls clear fields left
/// behind by an earlier lifecycle state, so one CAS patch is enough.
fn job_merge_value(job: &ExportJob) -> Value {
    json!({
        "jobId": optional_string(&job.job_id),
        "name": optional_string(&job.name),
        "subjects": if job.subjects.is_empty() {
            Value::Null
        } else {
            serde_json::to_value(&job.subjects).unwrap_or(Value::Null)
        },
        "format": job.format,
        "status": job.status,
        "progress": optional_string(&job.progress),
        "error": optional_string(&job.error),
        "pollError": optional_string(&job.poll_error),
        "startedAt": job.started_at,
        "finishedAt": optional_string(&job.finished_at),
        "outputs": if job.outputs.is_empty() {
            Value::Null
        } else {
            serde_json::to_value(&job.outputs).unwrap_or(Value::Null)
        },
    })
}

/// Writes one job member, returning the settings document's new version on
/// success — callers thread it into the next CAS attempt (a single request
/// can poll several jobs in turn; see [`refresh_in_progress_jobs`]).
async fn store_job(
    state: &WebState,
    user_key: &str,
    tenant: &str,
    id: &str,
    job: &ExportJob,
    expected_version: i64,
) -> Result<i64, String> {
    let Some(store) = &state.settings else {
        return Err("settings store unavailable".to_string());
    };
    let value = job_merge_value(job);
    let patch = json!({ "byTenant": { tenant: { "sqlExport": { "jobs": { id: value } } } } });
    store
        .patch_settings(user_key, patch, Some(expected_version))
        .await
        .map(|stored| stored.version)
        .map_err(|e| e.to_string())
}

/// What the caller expects the member to look like before its write, used to
/// tell "someone else changed it" apart from "someone else removed it" when a
/// CAS attempt has to reload and retry.
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

/// Same bound Bulk Export's job store retries a lost CAS with — this store's
/// design explicitly follows that one (#833).
const SETTINGS_CAS_ATTEMPTS: usize = 3;

/// Stores `job` under `id`, retrying a lost optimistic-locking race up to
/// [`SETTINGS_CAS_ATTEMPTS`] times by reloading the document and checking
/// `expectation` still holds. If the member was concurrently removed (or, for
/// a fresh job, concurrently created), this gives up rather than recreating
/// or clobbering it — the same rule Bulk Export's store applies.
async fn store_job_conditionally(
    state: &WebState,
    user_key: &str,
    tenant: &str,
    id: &str,
    job: &ExportJob,
    mut expected_version: i64,
    expectation: MemberExpectation<'_>,
) -> Result<i64, String> {
    let mut last_error = String::new();
    for attempt in 0..SETTINGS_CAS_ATTEMPTS {
        match store_job(state, user_key, tenant, id, job, expected_version).await {
            Ok(new_version) => return Ok(new_version),
            Err(error) => last_error = error,
        }
        if attempt + 1 == SETTINGS_CAS_ATTEMPTS {
            break;
        }
        let refreshed = load_jobs_checked(state, user_key, tenant).await?;
        if !expectation.matches(refreshed.jobs.get(id)) {
            return Err("SQL export job changed concurrently".to_string());
        }
        expected_version = refreshed.version;
    }
    Err(last_error)
}

/// Deletes one job member via a `null` merge patch — the same removal idiom
/// [`store_job`] uses for writes, applied to [`remove`].
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
    let patch = json!({ "byTenant": { tenant: { "sqlExport": { "jobs": { id: Value::Null } } } } });
    store
        .patch_settings(user_key, patch, Some(expected_version))
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Removes `id`, retrying a lost CAS like [`store_job_conditionally`] up to
/// [`SETTINGS_CAS_ATTEMPTS`] times. Gives up rather than deleting whatever
/// the member concurrently became if it no longer matches
/// `expected_member` — a stale Remove must not erase a job a concurrent poll
/// just updated.
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
            return Err("SQL export job changed concurrently".to_string());
        }
        expected_version = refreshed.version;
    }
    Err(last_error)
}

/// `complete` | `failed` | `cancelled` — every state `$sql-export` treats as
/// finished. Rerun and Remove admit any of these; Retry narrows this further
/// to `failed` only.
fn terminal_status(status: &str) -> bool {
    matches!(status, "complete" | "failed" | "cancelled")
}

fn now_stamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_job(value: &Value) -> ExportJob {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Kick-off & polling (#833)
// ---------------------------------------------------------------------------

/// The `$sql-export` output name for each of `subjects`, in submission
/// order: each subject's display name — the same one the card shows — made
/// unique within the job. Two subjects sharing a display name (distinct
/// ViewDefinitions/Libraries can share a `name`) are disambiguated with a
/// `-2`, `-3`, ... suffix on the second and later occurrences, since the
/// manifest's output name doubles as a downloaded shard's file name and the
/// server does not deduplicate it for us (#833).
fn subject_output_names(subjects: &[JobSubject]) -> Vec<String> {
    let mut seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    subjects
        .iter()
        .map(|subject| {
            let occurrence = seen.entry(subject.name.as_str()).or_insert(0);
            *occurrence += 1;
            if *occurrence == 1 {
                subject.name.clone()
            } else {
                format!("{}-{}", subject.name, occurrence)
            }
        })
        .collect()
}

/// Submits the job's subjects to `$sql-export`. On success, records the
/// server's job id; on failure, the job becomes `failed` on the spot — the
/// caller still stores and redirects to the list, which is the feedback.
async fn kickoff(state: &WebState, job: &mut ExportJob, caller: &Caller) {
    // The output name is the subject's display name (the card already shows
    // the same name) rather than the reference's id segment (#833): the
    // manifest's output name is what Files shows and what a downloaded shard
    // is named after, and a UUID id is meaningless there.
    // subject_output_names() disambiguates two subjects that happen to share
    // a display name.
    let names = subject_output_names(&job.subjects);
    let subjects: Vec<(String, String)> = job
        .subjects
        .iter()
        .zip(names)
        .map(|(subject, name)| (name, subject.reference.clone()))
        .collect();
    match state
        .conformance
        .sql_export_start(&subjects, &job.format, caller)
        .await
    {
        Ok(job_id) => job.job_id = job_id,
        Err(error) => {
            job.status = "failed".to_string();
            job.error = error;
            job.finished_at = now_stamp();
        }
    }
}

/// One status poll, applying exactly one of the transitions the module docs
/// table describes. Never called more than once per job per request.
async fn poll_job(state: &WebState, job: &mut ExportJob, caller: &Caller, i18n: &I18n) {
    match state
        .conformance
        .sql_export_status(&job.job_id, caller)
        .await
    {
        SqlExportStatus::Running(progress) => {
            job.status = "in-progress".to_string();
            job.progress = progress.unwrap_or_default();
            job.poll_error.clear();
        }
        SqlExportStatus::Done => match state
            .conformance
            .sql_export_manifest(&job.job_id, caller)
            .await
        {
            Ok(manifest) => {
                job.status = "complete".to_string();
                job.outputs = job_outputs(&manifest);
                job.progress.clear();
                job.poll_error.clear();
                job.error.clear();
                job.finished_at = now_stamp();
            }
            Err(message) => {
                job.status = "failed".to_string();
                job.error = message;
                job.progress.clear();
                job.poll_error.clear();
                job.finished_at = now_stamp();
            }
        },
        SqlExportStatus::Unknown => {
            job.status = "cancelled".to_string();
            job.error = i18n.t("sql-export-cancelled-reason");
            job.progress.clear();
            job.poll_error.clear();
            job.finished_at = now_stamp();
        }
        SqlExportStatus::Unavailable(message) => {
            job.poll_error = message;
        }
    }
}

/// Polls every `in-progress` job in `snapshot` once and persists each
/// transition, threading the settings document's version forward from write
/// to write so a request that refreshes several jobs does not manufacture a
/// spurious CAS conflict against its own earlier write.
///
/// A job whose write loses the race (or whose store call otherwise fails) is
/// simply rendered with its pre-poll state; the next poll — this page's next
/// reload, or the card's own htmx fragment — tries again.
async fn refresh_in_progress_jobs(
    state: &WebState,
    user_key: &str,
    tenant: &str,
    caller: &Caller,
    i18n: &I18n,
    snapshot: JobsSnapshot,
) -> serde_json::Map<String, Value> {
    let JobsSnapshot {
        mut jobs,
        mut version,
    } = snapshot;
    let in_progress: Vec<String> = jobs
        .iter()
        .filter(|(_, value)| value.get("status").and_then(Value::as_str) == Some("in-progress"))
        .map(|(id, _)| id.clone())
        .collect();
    for id in in_progress {
        let Some(original) = jobs.get(&id).cloned() else {
            continue;
        };
        let mut job = parse_job(&original);
        poll_job(state, &mut job, caller, i18n).await;
        if let Ok(new_version) = store_job_conditionally(
            state,
            user_key,
            tenant,
            &id,
            &job,
            version,
            MemberExpectation::Unchanged(&original),
        )
        .await
        {
            version = new_version;
            jobs.insert(id, serde_json::to_value(&job).unwrap_or(original));
        }
    }
    jobs
}

// ---------------------------------------------------------------------------
// Subjects (#649): the stored ViewDefinitions/Libraries the form offers
// ---------------------------------------------------------------------------

/// One checkbox row of the export form: a runnable subject the store holds.
pub(crate) struct ExportSubject {
    /// `ViewDefinition/{id}` or `Library/{id}` — the `subjectReference`.
    pub(crate) reference: String,
    pub(crate) name: String,
    /// Display label for the form's kind chip ("ViewDefinition", "SQL
    /// Query", "SQL View") — the restyle is #834, unchanged here.
    pub(crate) kind_label: &'static str,
    /// The stable code a job's [`JobSubject::kind`] stores; translated back
    /// through i18n when a card's meta line summarizes it.
    pub(crate) kind_code: &'static str,
}

/// The stored subjects `$sql-export` can run: every ViewDefinition, and every
/// Library carrying a SQL on FHIR kind. Shared by `/new` (offering the
/// checkboxes) and the kick-off handler (resolving what was checked).
async fn export_subjects(
    state: &WebState,
    version: helios_fhir::FhirVersion,
    tenant: &str,
) -> (Vec<ExportSubject>, Option<String>) {
    let mut subjects = Vec::new();
    let mut degraded = None;
    match state
        .conformance
        .fetch("ViewDefinition", version, tenant)
        .await
    {
        Ok(vds) => {
            for e in crate::sql_views::summarize(&vds) {
                subjects.push(ExportSubject {
                    reference: format!("ViewDefinition/{}", e.id),
                    name: e.name,
                    kind_label: "ViewDefinition",
                    kind_code: "view-definition",
                });
            }
        }
        Err(error) => degraded = Some(error),
    }
    match state.conformance.fetch("Library", version, tenant).await {
        Ok(libs) => {
            for (code, kind_label, kind_code) in [
                ("sql-query", "SQL Query", "sql-query"),
                ("sql-view", "SQL View", "sql-view"),
            ] {
                for e in crate::sql_libraries::summarize(&libs, code) {
                    subjects.push(ExportSubject {
                        reference: format!("Library/{}", e.id),
                        name: e.name,
                        kind_label,
                        kind_code,
                    });
                }
            }
        }
        Err(error) => degraded = degraded.or(Some(error)),
    }
    (subjects, degraded)
}

// ---------------------------------------------------------------------------
// View models & templates
// ---------------------------------------------------------------------------

struct JobCard {
    id: String,
    name: String,
    status: String,
    status_label: String,
    /// `0`–`100` for the progress track: terminal states fill the bar,
    /// in-progress parses the percentage out of the last `X-Progress`.
    progress_pct: String,
    /// The single `.job-card__meta` line, already fully localized.
    meta: String,
    /// The server's job id — the "View files" link's `?job=` (empty when the
    /// kick-off never got one, in which case the card never reaches
    /// `complete` and the link never renders).
    job_id: String,
}

fn status_label(i18n: &I18n, status: &str) -> String {
    match status {
        "complete" => i18n.t("sql-export-status-complete"),
        "failed" => i18n.t("sql-export-status-failed"),
        "cancelled" => i18n.t("sql-export-status-cancelled"),
        _ => i18n.t("sql-export-status-in-progress"),
    }
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

/// The job's name if it has one, else its subjects' names joined by ` · `
/// (at most 3, then ` +N`).
fn card_name(job: &ExportJob) -> String {
    if !job.name.is_empty() {
        return job.name.clone();
    }
    let names: Vec<&str> = job.subjects.iter().map(|s| s.name.as_str()).collect();
    match names.len() {
        0 => String::new(),
        1..=3 => names.join(" · "),
        n => format!("{} +{}", names[..3].join(" · "), n - 3),
    }
}

fn total_locations(job: &ExportJob) -> usize {
    job.outputs.iter().map(|o| o.locations.len()).sum()
}

/// `finished - started`, formatted `5m 08s` when both timestamps parse.
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

/// `HH:MM UTC` when `stamp` falls on today (UTC); `YYYY-MM-DD HH:MM UTC`
/// otherwise. Empty when `stamp` does not parse.
fn format_hour(stamp: &str) -> String {
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(stamp) else {
        return String::new();
    };
    let utc = parsed.with_timezone(&Utc);
    if utc.date_naive() == Utc::now().date_naive() {
        utc.format("%H:%M UTC").to_string()
    } else {
        utc.format("%Y-%m-%d %H:%M UTC").to_string()
    }
}

fn format_label(i18n: &I18n, format: &str) -> String {
    match format {
        "ndjson" => i18n.t("sql-export-format-ndjson"),
        "csv" => i18n.t("sql-export-format-csv"),
        "json" => i18n.t("sql-export-format-json"),
        "parquet" => i18n.t("sql-export-format-parquet"),
        other => other.to_string(),
    }
}

/// `"6 subjects (4 ViewDefinitions · 1 SQL Query · 1 SQL View)"`: only the
/// kinds actually present, in a fixed order, every count Fluent-pluralized.
fn subjects_summary(i18n: &I18n, subjects: &[JobSubject]) -> String {
    let (mut view_definitions, mut sql_queries, mut sql_views) = (0i64, 0i64, 0i64);
    for subject in subjects {
        match subject.kind.as_str() {
            "view-definition" => view_definitions += 1,
            "sql-query" => sql_queries += 1,
            "sql-view" => sql_views += 1,
            _ => {}
        }
    }
    let mut kinds = Vec::new();
    if view_definitions > 0 {
        kinds.push(i18n.t_arg("sql-export-kind-view-definition", "count", view_definitions));
    }
    if sql_queries > 0 {
        kinds.push(i18n.t_arg("sql-export-kind-sql-query", "count", sql_queries));
    }
    if sql_views > 0 {
        kinds.push(i18n.t_arg("sql-export-kind-sql-view", "count", sql_views));
    }
    let count = i18n.t_arg("sql-export-subjects-count", "count", subjects.len() as i64);
    if kinds.is_empty() {
        count
    } else {
        format!("{count} ({})", kinds.join(" · "))
    }
}

/// The single `.job-card__meta` line: the subjects/format summary is common
/// to every status, the rest of the line varies.
fn job_meta(i18n: &I18n, job: &ExportJob) -> String {
    let subjects = subjects_summary(i18n, &job.subjects);
    let format = format_label(i18n, &job.format);
    match job.status.as_str() {
        "complete" => {
            let files = i18n.t_arg(
                "sql-export-files-count",
                "count",
                total_locations(job) as i64,
            );
            let mut meta = format!("{subjects} · {format} · {files}");
            let elapsed = elapsed(job);
            if !elapsed.is_empty() {
                meta.push_str(&format!(
                    " · {} {elapsed}",
                    i18n.t("sql-export-finished-in")
                ));
            }
            meta
        }
        "failed" => format!("{subjects} · {format} · {}", job.error),
        "cancelled" => {
            let mut meta = format!(
                "{subjects} · {format} · {} {}",
                i18n.t("sql-export-cancelled-at"),
                format_hour(&job.finished_at)
            );
            if !job.error.is_empty() {
                meta.push_str(&format!(" · {}", job.error));
            }
            meta
        }
        // in-progress, and defensively any other/unknown value.
        _ => {
            let progress = if job.progress.is_empty() {
                i18n.t("sql-export-progress-waiting")
            } else {
                job.progress.clone()
            };
            let mut meta = format!(
                "{progress} · {subjects} · {format} · {} {}",
                i18n.t("sql-export-started"),
                format_hour(&job.started_at)
            );
            if !job.poll_error.is_empty() {
                meta.push_str(&format!(
                    " · {}: {}",
                    i18n.t("sql-export-status-unavailable"),
                    job.poll_error
                ));
            }
            meta
        }
    }
}

fn job_card(i18n: &I18n, id: &str, job: &ExportJob) -> JobCard {
    JobCard {
        id: id.to_string(),
        name: card_name(job),
        status_label: status_label(i18n, &job.status),
        status: job.status.clone(),
        progress_pct: progress_pct(&job.status, &job.progress),
        meta: job_meta(i18n, job),
        job_id: job.job_id.clone(),
    }
}

/// The Active SQL Exports page (#833): the user's `$sql-export` jobs as
/// cards, most recent first. The entry point of the SQL Export workspace —
/// the builder moved to `/new`.
#[derive(Template)]
#[template(path = "pages/sql-export.html")]
struct ExportListPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
    available: bool,
    total: usize,
    running: usize,
    cards: Vec<JobCard>,
    /// The just-kicked-off job's server id, carried through the `start`
    /// redirect when the kick-off succeeded but this UI's own settings-store
    /// write to record it failed (see [`start`]) — otherwise `None`, per the
    /// "no flash on success" rule (#833).
    store_error: Option<String>,
}

/// `application/x-www-form-urlencoded` percent-encoding for a value going
/// into a query string this crate constructs — the same encoding
/// `sql_views::urlencode` uses for the same reason (a raw `&`/`#`/space in
/// the value must not corrupt the redirect's `Location` header).
fn urlencode(value: &str) -> String {
    form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// The SQL Export builder (#833, formerly the whole `/ui/sql/export` page):
/// pick stored subjects, choose a format, submit. Its restyle is #834 — this
/// keeps the existing form markup, just moved off the list route.
#[derive(Template)]
#[template(path = "pages/sql-export-new.html")]
struct ExportNewPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
    subjects: Vec<ExportSubject>,
    degraded: Option<String>,
    start_error: Option<String>,
}

#[derive(Template)]
#[template(path = "partials/sql_export_card.html")]
struct JobCardFragment {
    i18n: I18n,
    card: JobCard,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Query the list route accepts. `job` is the pre-#833 single-job tracking
/// param — deserialized so an old bookmark or link does not 400, then
/// ignored. `store-error` is set only by [`start`]'s redirect.
#[derive(Deserialize, Default)]
pub(crate) struct ExportListQuery {
    #[allow(dead_code)]
    job: Option<String>,
    #[serde(rename = "store-error")]
    store_error: Option<String>,
}

/// `GET /ui/sql/export` — the job list (#833). Polls every `in-progress` job
/// once before rendering, so a plain reload without JavaScript stays
/// current; a legacy `?job=` is accepted by the router but ignored.
pub(crate) async fn list(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<ExportListQuery>,
) -> Response {
    let i18n = I18n::new(locale);
    let status = current_status(&state, rv.0, &rt);
    let available = state.settings.is_some();
    let jobs = if available {
        let user_key = settings_user_key(principal.as_deref());
        let snapshot = load_jobs(&state, &user_key, &rt.id).await;
        let caller = Caller::from_request(&headers, &rt.id);
        refresh_in_progress_jobs(&state, &user_key, &rt.id, &caller, &i18n, snapshot).await
    } else {
        Default::default()
    };
    let mut entries: Vec<(String, ExportJob)> = jobs
        .iter()
        .map(|(id, value)| (id.clone(), parse_job(value)))
        .collect();
    entries.sort_by(|a, b| b.1.started_at.cmp(&a.1.started_at));
    let running = entries
        .iter()
        .filter(|(_, job)| job.status == "in-progress")
        .count();
    let cards = entries
        .iter()
        .map(|(id, job)| job_card(&i18n, id, job))
        .collect();
    render(ExportListPage {
        status,
        i18n,
        active_page: "sql-export",
        available,
        total: entries.len(),
        running,
        cards,
        store_error: query.store_error,
    })
}

/// `GET /ui/sql/export/new` — the export builder.
pub(crate) async fn new_page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
) -> Response {
    render_new_page(&state, locale, rv.0, &rt, None).await
}

async fn render_new_page(
    state: &WebState,
    locale: RequestLocale,
    version: helios_fhir::FhirVersion,
    rt: &RequestTenant,
    start_error: Option<String>,
) -> Response {
    let (subjects, degraded) = export_subjects(state, version, &rt.id).await;
    render(ExportNewPage {
        status: current_status(state, version, rt),
        i18n: I18n::new(locale),
        active_page: "sql-export",
        subjects,
        degraded,
        start_error,
    })
}

/// `POST /ui/sql/export` — resolve the checked subjects against the same
/// list `/new` offered, create the job record, kick it off, and land on the
/// list. The form repeats `subject=` per checkbox, which `Form`-into-a-struct
/// cannot express, so the raw body is parsed by hand.
pub(crate) async fn start(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    headers: HeaderMap,
    axum::extract::RawForm(body): axum::extract::RawForm,
) -> Response {
    let i18n = I18n::new(locale);
    let mut format = "ndjson".to_string();
    let mut refs: Vec<String> = Vec::new();
    for (key, value) in form_urlencoded::parse(&body) {
        match key.as_ref() {
            "subject" => refs.push(value.into_owned()),
            "format" => format = value.into_owned(),
            _ => {}
        }
    }
    if refs.is_empty() {
        return render_new_page(
            &state,
            locale,
            rv.0,
            &rt,
            Some(i18n.t("sql-export-select-subject")),
        )
        .await;
    }

    // Resolve every checked reference against the same subjects `/new`
    // offers: an unknown reference (deleted or tampered with since the form
    // was loaded) re-renders instead of kicking off a half-formed job. A
    // degraded fetch cannot validate anything, so it fails the same way
    // rather than trusting the submitted names/kinds blindly.
    let (available_subjects, degraded) = export_subjects(&state, rv.0, &rt.id).await;
    if let Some(message) = degraded {
        return render_new_page(&state, locale, rv.0, &rt, Some(message)).await;
    }
    let mut subjects = Vec::with_capacity(refs.len());
    for reference in &refs {
        let Some(found) = available_subjects
            .iter()
            .find(|subject| &subject.reference == reference)
        else {
            return render_new_page(
                &state,
                locale,
                rv.0,
                &rt,
                Some(i18n.t("sql-export-unknown-subject")),
            )
            .await;
        };
        subjects.push(JobSubject {
            name: found.name.clone(),
            reference: reference.clone(),
            kind: found.kind_code.to_string(),
        });
    }

    let caller = Caller::from_request(&headers, &rt.id);
    let mut job = ExportJob {
        subjects,
        format,
        status: "in-progress".to_string(),
        started_at: now_stamp(),
        ..Default::default()
    };
    kickoff(&state, &mut job, &caller).await;

    let user_key = settings_user_key(principal.as_deref());
    store_new_job(&state, &user_key, &rt.id, job).await
}

/// Persists `job` under a freshly generated local id and redirects to the
/// list — the tail every job-creating entry point shares ([`start`],
/// [`resubmit`]).
///
/// The job already ran (or failed) server-side by the time this runs; if the
/// write loses its optimistic-locking race, the list would otherwise show no
/// card for it at all — a rare lost settings-document race, but silent data
/// loss from the user's point of view. When the kick-off itself got a job
/// id, the redirect carries it via `?store-error=` so the user can still
/// reach Files once it finishes, instead of a `tracing::error!` no one sees.
async fn store_new_job(state: &WebState, user_key: &str, tenant: &str, job: ExportJob) -> Response {
    let snapshot = load_jobs(state, user_key, tenant).await;
    let id = uuid::Uuid::new_v4().to_string();
    let job_id = job.job_id.clone();
    if let Err(error) = store_job_conditionally(
        state,
        user_key,
        tenant,
        &id,
        &job,
        snapshot.version,
        MemberExpectation::Absent,
    )
    .await
    {
        tracing::error!(
            job_id,
            error,
            "failed to persist a new SQL export job record"
        );
        if !job_id.is_empty() {
            return Redirect::to(&format!(
                "/ui/sql/export?store-error={}",
                urlencode(&job_id)
            ))
            .into_response();
        }
    }
    Redirect::to("/ui/sql/export").into_response()
}

/// Builds a fresh `in-progress` job that copies `source`'s
/// `name`/`subjects`/`format`, kicks it off under `caller`, and stores it as
/// a brand-new record via [`store_new_job`] — the resubmission [`retry`] and
/// [`rerun`] share: `source` itself is never written back to.
async fn resubmit(
    state: &WebState,
    user_key: &str,
    tenant: &str,
    caller: &Caller,
    source: &ExportJob,
) -> Response {
    let mut job = ExportJob {
        name: source.name.clone(),
        subjects: source.subjects.clone(),
        format: source.format.clone(),
        status: "in-progress".to_string(),
        started_at: now_stamp(),
        ..Default::default()
    };
    kickoff(state, &mut job, caller).await;
    store_new_job(state, user_key, tenant, job).await
}

/// Shared tail of [`retry`] and [`rerun`]: loads the record `id` names,
/// checks `eligible` against its current status — a no-op redirect when it
/// is not, or when `id` names nothing this user/tenant owns — and, when
/// eligible, hands it to [`resubmit`]. The original record is read-only
/// throughout; only a brand-new record is ever written.
async fn retry_or_rerun(
    state: &WebState,
    tenant: &str,
    principal: Option<Extension<helios_auth::Principal>>,
    headers: &HeaderMap,
    id: &str,
    eligible: impl Fn(&str) -> bool,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    let snapshot = load_jobs(state, &user_key, tenant).await;
    let Some(original) = snapshot.jobs.get(id).map(parse_job) else {
        return Redirect::to("/ui/sql/export").into_response();
    };
    if !eligible(&original.status) {
        return Redirect::to("/ui/sql/export").into_response();
    }
    let caller = Caller::from_request(headers, tenant);
    resubmit(state, &user_key, tenant, &caller, &original).await
}

/// `POST /ui/sql/export/{id}/cancel` — eligible only from `in-progress`.
/// Asks the server to cancel the job under the request's
/// [`Caller`]; whatever it answers (a `404` included — the job may already
/// be reaped), the record becomes `cancelled` locally with a clean slate
/// (`progress`/`error` cleared, since this is a cancellation the user asked
/// for, not a failure) — there is nothing further this UI can do for it
/// either way.
pub(crate) async fn cancel(
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
        if job.status == "in-progress" {
            let caller = Caller::from_request(&headers, &rt.id);
            let _ = state
                .conformance
                .sql_export_cancel(&job.job_id, &caller)
                .await;
            job.status = "cancelled".to_string();
            job.progress.clear();
            job.error.clear();
            job.poll_error.clear();
            job.finished_at = now_stamp();
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
    }
    Redirect::to("/ui/sql/export").into_response()
}

/// `POST /ui/sql/export/{id}/retry` — eligible only from `failed`. See
/// [`retry_or_rerun`]/[`resubmit`] for the shared resubmission; the original
/// failed record is left untouched.
pub(crate) async fn retry(
    State(state): State<WebState>,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    retry_or_rerun(&state, &rt.id, principal, &headers, &id, |status| {
        status == "failed"
    })
    .await
}

/// `POST /ui/sql/export/{id}/rerun` — eligible from any terminal state
/// ([`terminal_status`]: `complete`, `failed`, `cancelled`). Otherwise
/// identical to [`retry`]; only the eligible source states differ, matching
/// which cards' overflow offers "Run again".
pub(crate) async fn rerun(
    State(state): State<WebState>,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    retry_or_rerun(&state, &rt.id, principal, &headers, &id, terminal_status).await
}

/// `POST /ui/sql/export/{id}/remove` — eligible only from a terminal state
/// ([`terminal_status`]); an `in-progress` job cannot be removed from
/// the list (Cancel it first). Deletes exactly the local record — the
/// server is never called, since a finished job's outputs are the reaper's
/// to reclaim, not this UI's.
pub(crate) async fn remove(
    State(state): State<WebState>,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    Path(id): Path<String>,
) -> Response {
    let user_key = settings_user_key(principal.as_deref());
    let snapshot = load_jobs(&state, &user_key, &rt.id).await;
    if let Some(original) = snapshot.jobs.get(&id)
        && terminal_status(&parse_job(original).status)
    {
        let _ =
            remove_job_conditionally(&state, &user_key, &rt.id, &id, original, snapshot.version)
                .await;
    }
    Redirect::to("/ui/sql/export").into_response()
}

/// `GET /ui/sql/export/{id}/card` — one poll, then the refreshed card. 404
/// for an id this user/tenant does not own.
pub(crate) async fn card(
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
        let caller = Caller::from_request(&headers, &rt.id);
        poll_job(&state, &mut job, &caller, &i18n).await;
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
    render(JobCardFragment {
        card: job_card(&i18n, &id, &job),
        i18n,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outputs_and_values_read_the_manifest_shape() {
        let manifest = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "exportId", "valueString": "job-1"},
                {"name": "_format", "valueCode": "csv"},
                {"name": "exportDuration", "valueInteger": 4},
                {"name": "output", "part": [
                    {"name": "name", "valueString": "patients"},
                    {"name": "location", "valueUri": "http://s/export/job-1/patients-0.csv"},
                    {"name": "location", "valueUri": "http://s/export/job-1/patients-1.csv"},
                ]},
                {"name": "output", "part": [
                    {"name": "name", "valueString": "obs"},
                    {"name": "location", "valueUri": "http://s/export/job-1/obs-0.csv"},
                ]},
            ]
        });
        let outputs = manifest_outputs(&manifest);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].name, "patients");
        // Locations come back rewritten to same-origin paths — exercised on
        // its own terms by same_origin_location's tests below.
        assert_eq!(
            outputs[0].locations,
            [
                "/export/job-1/patients-0.csv",
                "/export/job-1/patients-1.csv"
            ]
        );
        assert_eq!(outputs[1].locations, ["/export/job-1/obs-0.csv"]);
        assert_eq!(manifest_value(&manifest, "_format").as_deref(), Some("csv"));
        assert_eq!(
            manifest_value(&manifest, "exportDuration").as_deref(),
            Some("4")
        );
        assert_eq!(manifest_value(&manifest, "missing"), None);
    }

    #[test]
    fn same_origin_location_strips_scheme_host_and_port_but_keeps_the_query() {
        assert_eq!(
            same_origin_location("http://localhost:8080/export/job-1/patients-0.csv"),
            "/export/job-1/patients-0.csv"
        );
        assert_eq!(
            same_origin_location("https://hfs.example.com/export/job-1/x.csv?sig=abc&exp=1"),
            "/export/job-1/x.csv?sig=abc&exp=1"
        );
        // Already a bare path (or anything else that does not parse as an
        // absolute URL): left exactly as-is.
        assert_eq!(
            same_origin_location("/export/job-1/patients-1.csv"),
            "/export/job-1/patients-1.csv"
        );
    }

    #[test]
    fn subject_output_names_uses_the_display_name_and_disambiguates_duplicates() {
        let subjects = vec![
            subject("patients_flat", "view-definition"),
            subject("patients_flat", "sql-query"),
            subject("encounters", "view-definition"),
            subject("patients_flat", "sql-view"),
        ];
        assert_eq!(
            subject_output_names(&subjects),
            vec![
                "patients_flat",
                "patients_flat-2",
                "encounters",
                "patients_flat-3"
            ]
        );
    }

    fn subject(name: &str, kind: &str) -> JobSubject {
        JobSubject {
            name: name.to_string(),
            reference: format!("ViewDefinition/{name}"),
            kind: kind.to_string(),
        }
    }

    #[test]
    fn card_name_falls_back_to_subject_names_capped_at_three() {
        let mut job = ExportJob {
            name: "Monthly flat files".to_string(),
            ..Default::default()
        };
        assert_eq!(card_name(&job), "Monthly flat files");

        job.name.clear();
        job.subjects = vec![subject("a", "view-definition"), subject("b", "sql-query")];
        assert_eq!(card_name(&job), "a · b");

        job.subjects = vec![
            subject("a", "view-definition"),
            subject("b", "view-definition"),
            subject("c", "view-definition"),
            subject("d", "view-definition"),
        ];
        assert_eq!(card_name(&job), "a · b · c +1");
    }

    #[test]
    fn progress_pct_parses_a_leading_percentage_and_floors_terminal_states() {
        assert_eq!(progress_pct("in-progress", "35%"), "35");
        assert_eq!(progress_pct("in-progress", ""), "0");
        assert_eq!(progress_pct("in-progress", "not a percent"), "0");
        assert_eq!(progress_pct("complete", ""), "100");
        assert_eq!(progress_pct("cancelled", ""), "100");
    }

    #[test]
    fn subjects_summary_lists_only_present_kinds_in_a_fixed_order() {
        let i18n = I18n::from_tag("en").unwrap();
        let subjects = vec![
            subject("a", "view-definition"),
            subject("b", "view-definition"),
            subject("c", "sql-view"),
        ];
        assert_eq!(
            subjects_summary(&i18n, &subjects),
            "3 subjects (2 ViewDefinitions · 1 SQL View)"
        );
        assert_eq!(subjects_summary(&i18n, &[]), "0 subjects");
    }

    #[test]
    fn job_merge_value_nulls_out_every_empty_field() {
        let job = ExportJob {
            status: "in-progress".to_string(),
            started_at: "2026-09-01T09:00:00Z".to_string(),
            format: "csv".to_string(),
            ..Default::default()
        };
        let value = job_merge_value(&job);
        assert_eq!(value["jobId"], Value::Null);
        assert_eq!(value["name"], Value::Null);
        assert_eq!(value["subjects"], Value::Null);
        assert_eq!(value["error"], Value::Null);
        assert_eq!(value["pollError"], Value::Null);
        assert_eq!(value["finishedAt"], Value::Null);
        assert_eq!(value["outputs"], Value::Null);
        assert_eq!(value["status"], "in-progress");
        assert_eq!(value["format"], "csv");
    }
}
