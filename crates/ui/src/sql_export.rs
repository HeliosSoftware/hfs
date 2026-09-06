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

use std::sync::atomic::Ordering;

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
    Caller, RequestTenant, RequestVersion, SqlExportParameter, SqlExportRequest, SqlExportStatus,
    SqlExportSubject, Status, WebState, current_status, render, render_not_found,
    settings_user_key,
};

// ---------------------------------------------------------------------------
// Completion-manifest view models (#649): shaping `$sql-export`'s finished
// `Parameters` into a job's persisted `outputs`.
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
    /// The values submitted for this subject's `Library.parameter`
    /// declarations (#837) — empty for a ViewDefinition, a SQL View, or an
    /// unparameterized SQL Query. Carried verbatim from the builder form into
    /// [`kickoff`]'s `SqlExportRequest`, and copied unchanged by
    /// [`resubmit`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<JobParameter>,
}

/// One subject parameter value as the job record stores it (#837): the
/// declared name (without a leading `:`), the `Library.parameter.type` code,
/// and the value text exactly as the user entered it — already validated by
/// the time it is stored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobParameter {
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub value: String,
}

/// The job-wide filters `$sql-export` applies across every subject (#836):
/// which patients/groups to scope to, how far back `_since` reaches, whether
/// the CSV output carries a header row, and a client tracking id. Verified
/// against `crates/rest/src/handlers/sof/export.rs`: `patient`/`group`/
/// `_since` fold into a single `ViewFilters` for the whole `ExportTask`, so
/// this is one record per job, never one per subject.
///
/// Serialization is deliberately all-or-nothing and field-stable: when at
/// least one filter is set, every one of the five keys below is written —
/// an empty `Vec` as `[]`, `header: None` as `null` — never individually
/// omitted. Omitting an empty `patients`/`groups`/etc. per-field would make
/// a round trip through [`job_merge_value`] lossy: [`resubmit`] deserializes
/// a stored record back into a `JobFilters` and re-serializes it into the
/// new record, and a field that silently vanished when empty would make the
/// new record's `filters` unequal to the original's, even though nothing
/// about that filter changed. Only the whole object collapses — to
/// `filters: null` — and only when [`JobFilters::is_empty`] is `true`; see
/// [`ExportJob::filters`] and [`job_merge_value`].
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobFilters {
    /// `Patient/{id}` references, in submission order.
    #[serde(default)]
    pub patients: Vec<String>,
    /// `Group/{id}` references, in submission order.
    #[serde(default)]
    pub groups: Vec<String>,
    /// An RFC 3339 instant already resolved from whatever preset/custom input
    /// the form offered; empty means no filter — never a raw preset name.
    #[serde(default)]
    pub since: String,
    /// The CSV header switch; only meaningful for `format: "csv"`.
    #[serde(default)]
    pub header: Option<bool>,
    #[serde(default)]
    pub client_tracking_id: String,
}

impl JobFilters {
    /// `true` when every field is at its empty default — the signal
    /// [`job_merge_value`] uses to write `filters: null` instead of an empty
    /// object, and [`ExportJob`]'s own `filters` field uses to skip itself on
    /// serialization.
    fn is_empty(&self) -> bool {
        self.patients.is_empty()
            && self.groups.is_empty()
            && self.since.is_empty()
            && self.header.is_none()
            && self.client_tracking_id.is_empty()
    }
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
    /// Optional user-supplied name (#834): trimmed by the builder before
    /// storage, so an empty submission leaves this empty and the card falls
    /// back to the subjects' names — see [`card_name`]. Never sent to
    /// `$sql-export` itself; it is purely this notebook's own label.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<JobSubject>,
    /// `ndjson` | `csv` | `json` | `parquet`.
    #[serde(default)]
    pub format: String,
    /// The job-wide patient/group/since/header/tracking-id filters this job
    /// was submitted with (#836); its empty default deserializes cleanly from
    /// a record persisted before this field existed.
    #[serde(default, skip_serializing_if = "JobFilters::is_empty")]
    pub filters: JobFilters,
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

/// Every stored `$sql-export` job, most recently started first — the same
/// shape and sort [`list`] itself shows, reused by the SQL Query/SQL View
/// *Used by* card (`sql_libraries::used_by_exports`, #842) to find which
/// jobs' `subjects[].reference` name a given Library. Unlike [`list`], this
/// never polls a job's live status: the card only needs each job's own id
/// and display name to link to its detail page, not its current progress,
/// so a *Used by* render never pays for a poll it does not show.
pub(crate) async fn jobs_for_used_by(
    state: &WebState,
    user_key: &str,
    tenant: &str,
) -> Vec<(String, ExportJob)> {
    let snapshot = load_jobs(state, user_key, tenant).await;
    let mut entries: Vec<(String, ExportJob)> = snapshot
        .jobs
        .iter()
        .map(|(id, value)| (id.clone(), parse_job(value)))
        .collect();
    entries.sort_by(|a, b| b.1.started_at.cmp(&a.1.started_at));
    entries
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
        "filters": if job.filters.is_empty() {
            Value::Null
        } else {
            serde_json::to_value(&job.filters).unwrap_or(Value::Null)
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

/// Builds the `$sql-export` request `kickoff` submits from `job`: subjects
/// under their disambiguated output names carrying their own parameter
/// values, plus `job.filters` mapped field by field — an empty
/// `since`/`client_tracking_id` becomes `None` rather than an empty `Some`,
/// matching [`sql_export_parameters_body`](crate::sql_export_parameters_body)'s
/// own "set and not empty" rule for `clientTrackingId`.
fn export_request(job: &ExportJob) -> SqlExportRequest {
    // The output name is the subject's display name (the card already shows
    // the same name) rather than the reference's id segment (#833): the
    // manifest's output name is what Files shows and what a downloaded shard
    // is named after, and a UUID id is meaningless there.
    // subject_output_names() disambiguates two subjects that happen to share
    // a display name.
    let names = subject_output_names(&job.subjects);
    let subjects: Vec<SqlExportSubject> = job
        .subjects
        .iter()
        .zip(names)
        .map(|(subject, name)| SqlExportSubject {
            name,
            reference: subject.reference.clone(),
            parameters: subject
                .parameters
                .iter()
                .map(|p| SqlExportParameter {
                    name: p.name.clone(),
                    type_code: p.r#type.clone(),
                    value: p.value.clone(),
                })
                .collect(),
        })
        .collect();
    SqlExportRequest {
        subjects,
        format: job.format.clone(),
        header: job.filters.header,
        since: (!job.filters.since.is_empty()).then(|| job.filters.since.clone()),
        patients: job.filters.patients.clone(),
        groups: job.filters.groups.clone(),
        client_tracking_id: (!job.filters.client_tracking_id.is_empty())
            .then(|| job.filters.client_tracking_id.clone()),
    }
}

/// Submits the job's subjects and filters to `$sql-export`. On success,
/// records the server's job id; on failure, the job becomes `failed` on the
/// spot — the caller still stores and redirects to the list, which is the
/// feedback.
async fn kickoff(state: &WebState, job: &mut ExportJob, caller: &Caller) {
    let request = export_request(job);
    match state.conformance.sql_export_start(&request, caller).await {
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

/// One row of the export form's subjects table: a runnable subject the store
/// holds.
pub(crate) struct ExportSubject {
    /// `ViewDefinition/{id}` or `Library/{id}` — the `subjectReference`.
    pub(crate) reference: String,
    pub(crate) name: String,
    /// Display label for the table's kind tag ("ViewDefinition", "SQL
    /// Query", "SQL View").
    pub(crate) kind_label: &'static str,
    /// The stable code a job's [`JobSubject::kind`] stores, and the table
    /// row's `data-kind` (#834); translated back through i18n when a card's
    /// meta line summarizes it.
    pub(crate) kind_code: &'static str,
    /// `ViewDefinition.status` / `Library.status` — `draft`, `active`,
    /// `retired`, `unknown`, or empty when the resource carries none (#834).
    pub(crate) status: String,
    /// The Library's declared `Library.parameter[use=in]` entries (#837),
    /// read by [`crate::sql_libraries::parameters`] — always empty for a
    /// ViewDefinition or a SQL View, even when the underlying JSON carries a
    /// `parameter` array (the SQLView profile forbids it; a stray one there
    /// is simply not a query parameter this UI can bind).
    pub(crate) parameters: Vec<crate::sql_libraries::DeclaredParameter>,
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
                    status: e.status,
                    parameters: Vec::new(),
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
                    // A SQL Query's parameter values (#837) come from the
                    // full resource `summarize` already read `id`/`name`/
                    // `status` out of — re-found here by id in the same
                    // fetched list rather than fetched a second time. SQL
                    // Views never carry parameters (the profile forbids
                    // them), so this lookup is skipped entirely for them.
                    let parameters = if kind_code == "sql-query" {
                        libs.iter()
                            .find(|l| l.get("id").and_then(Value::as_str) == Some(e.id.as_str()))
                            .map(crate::sql_libraries::parameters)
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    subjects.push(ExportSubject {
                        reference: format!("Library/{}", e.id),
                        name: e.name,
                        kind_label,
                        kind_code,
                        status: e.status,
                        parameters,
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

/// One row of the builder's subjects table: a subject plus whether this
/// render should show it pre-checked — a `?subject=` prefill, or the
/// selection a rejected submission sent back for the user to correct —
/// computed once in [`subject_rows`] so the template only ever reads a plain
/// `bool`.
struct SubjectRow {
    reference: String,
    name: String,
    kind_label: &'static str,
    kind_code: &'static str,
    /// A missing status renders as an em dash, decided here rather than in
    /// the template so the fallback lives in exactly one place.
    status: String,
    checked: bool,
    /// `param:{reference}` — the shared `partials/sql_parameter_fields.html`
    /// partial's `prefix` argument (#837). Empty (and unused by the
    /// template) when `parameters` is empty.
    param_prefix: String,
    /// This subject's declared parameters, paired with the value/error this
    /// render should show for each (#837) — empty for every subject but a
    /// parameterized SQL Query, which is what both the values row and the
    /// `n parameter(s)` chip key off of.
    parameters: Vec<ParamFieldView>,
    /// The already-localized `n parameter(s)` chip text (#837), `None` when
    /// `parameters` is empty. Shown next to the subject's name whether or
    /// not the row is checked, so a parameterized query's requirement is
    /// visible before it is ever selected.
    param_chip: Option<String>,
    /// `sql-export-params-{reference, "/" replaced with "-"}` (#837) — the
    /// values row's own element `id`, and the row-toggle chevron's
    /// `aria-controls`. Empty (and unused by the template) when `parameters`
    /// is empty. A slug rather than the raw reference: an HTML `id` must not
    /// contain whitespace, and this keeps it free of `/` too so it can also
    /// serve, unescaped, as a CSS/`querySelector` target if ever needed.
    param_row_id: String,
    /// Whether this render's values row should carry `data-open` (#837): the
    /// browser's JavaScript treats this as an explicit, server-authoritative
    /// signal to keep the row expanded and focus its first invalid field,
    /// independent of whatever fold state the row would otherwise start
    /// in — set exactly when a resubmission left at least one of this
    /// subject's parameter fields in error (always a checked subject, since
    /// [`validate_subject_parameters`] only ever runs over checked ones).
    open: bool,
}

/// `status`, or an em dash when the resource carries none.
fn status_display(status: &str) -> String {
    if status.is_empty() {
        "—".to_string()
    } else {
        status.to_string()
    }
}

/// One `param:{reference}:{name}` value as `start` reads it off the raw form
/// body (#837): which subject it belongs to (by reference, not table
/// position — the form can submit these in any order), the declared
/// parameter name, and the raw text exactly as submitted (not yet trimmed —
/// [`validate_subject_parameters`] does that once, at the point it decides
/// whether the field is empty).
#[derive(Debug, Clone)]
struct ParamValue {
    reference: String,
    name: String,
    value: String,
}

/// A field-level validation failure for one `param:{reference}:{name}`
/// input (#837) — [`start`]'s equivalent, for parameter fields, of
/// [`NewFormErrors::since_custom`]/`client_tracking_id`. Unlike those two,
/// a single submission can fail several parameter fields across several
/// subjects at once, so [`NewFormErrors::param_errors`] collects every one
/// instead of stopping at the first.
#[derive(Debug, Clone)]
struct ParamFieldError {
    reference: String,
    name: String,
    message: String,
}

/// One parameter field as `partials/sql_parameter_fields.html` renders it:
/// the declaration ([`name`](Self::name)/[`type_code`](Self::type_code)/
/// [`default`](Self::default)) plus this render's own
/// [`value`](Self::value), [`error`](Self::error), and
/// [`required`](Self::required) — [`subject_rows`] is the only place that
/// builds these, from a [`crate::sql_libraries::DeclaredParameter`] plus
/// whatever this render's [`ParamValue`]s/[`ParamFieldError`]s say about it.
struct ParamFieldView {
    name: String,
    type_code: String,
    default: Option<String>,
    /// The value submitted for this field on a re-render, or the declared
    /// default when nothing was submitted at all — never both at once, so a
    /// submission that deliberately blanked a defaulted field renders back
    /// blank rather than snapping to the default (#837).
    value: String,
    error: Option<String>,
    /// Whether this render marks the input `required`: only when the
    /// parameter has no default **and** its subject is checked in this
    /// render. An unmarked query's fields never carry `required` — the
    /// no-JavaScript requirement this page must keep working under — so the
    /// browser's native validation can never block the whole form on a
    /// query nobody selected; `start` is still the one place that actually
    /// enforces this.
    required: bool,
}

/// Pairs `subjects` with `checked`, resolves each parameterized SQL Query's
/// fields against `param_values`/`param_errors`, and counts how many
/// subjects ended up checked — the "n of m selected" hint's `n`. A
/// `selected` reference that matches no current subject (an unknown
/// `?subject=`, or a resubmitted reference the store no longer has) simply
/// checks nothing rather than erroring — unknown references are ignored in
/// silence, same as before #837.
fn subject_rows(
    subjects: Vec<ExportSubject>,
    selected: &[String],
    param_values: &[ParamValue],
    param_errors: &[ParamFieldError],
    i18n: &I18n,
) -> (Vec<SubjectRow>, usize) {
    let rows: Vec<SubjectRow> = subjects
        .into_iter()
        .map(|s| {
            let checked = selected.contains(&s.reference);
            let parameters: Vec<ParamFieldView> = s
                .parameters
                .iter()
                .map(|p| {
                    let submitted = param_values
                        .iter()
                        .find(|v| v.reference == s.reference && v.name == p.name)
                        .map(|v| v.value.as_str());
                    let value = submitted
                        .unwrap_or_else(|| p.default.as_deref().unwrap_or(""))
                        .to_string();
                    let error = param_errors
                        .iter()
                        .find(|e| e.reference == s.reference && e.name == p.name)
                        .map(|e| e.message.clone());
                    ParamFieldView {
                        name: p.name.clone(),
                        type_code: p.type_code.clone(),
                        default: p.default.clone(),
                        value,
                        error,
                        required: checked && p.default.is_none(),
                    }
                })
                .collect();
            let param_chip = (!parameters.is_empty())
                .then(|| i18n.t_arg("sql-export-param-count", "count", parameters.len() as i64));
            let open = parameters.iter().any(|p| p.error.is_some());
            SubjectRow {
                param_prefix: format!("param:{}", s.reference),
                param_row_id: format!("sql-export-params-{}", s.reference.replace('/', "-")),
                reference: s.reference,
                name: s.name,
                kind_label: s.kind_label,
                kind_code: s.kind_code,
                status: status_display(&s.status),
                checked,
                parameters,
                param_chip,
                open,
            }
        })
        .collect();
    let selected_count = rows.iter().filter(|row| row.checked).count();
    (rows, selected_count)
}

/// The builder's conserved form state across a re-render: a `?subject=`
/// prefill on a fresh `GET`, or — after a rejected `POST` — the
/// `name`/`format`/selection/filters the submission itself carried, so the
/// user never has to redo the parts that were fine. Defaults to a bare `GET
/// /new` with no prefill: no name, NDJSON (the default output format),
/// nothing checked, every "Narrow it down" filter empty, and the header
/// checkbox on (#836).
struct NewFormState {
    name: String,
    format: String,
    selected: Vec<String>,
    /// Raw submitted `patient` values, one per repeated field — each may
    /// itself carry several comma/newline-separated references, the
    /// combobox fallback textarea's own shape (#836). Never canonicalized:
    /// an invalid entry must echo back exactly as typed.
    patients: Vec<String>,
    /// Same shape as `patients`, for `group` (#836).
    groups: Vec<String>,
    since_preset: String,
    since_custom: String,
    /// Trimmed once by `start` before it is ever stored here, so every
    /// re-render echoes the same value the length check itself validated
    /// (#836).
    client_tracking_id: String,
    /// Whether the "Include a header row" checkbox renders checked (#836):
    /// always `true`, except a submission that paired `format: csv`
    /// with an unchecked box — the box's meaning outside `csv` is purely
    /// cosmetic (the server never receives it), so every other format keeps
    /// showing it checked regardless of what was actually submitted.
    header_checked: bool,
    /// Every submitted `param:{reference}:{name}` field (#837), verbatim —
    /// echoed back on a re-render exactly like `patients`/`groups`, whether
    /// or not its subject ended up checked (an unmarked query's typed
    /// values are never lost, only ignored at submission time).
    param_values: Vec<ParamValue>,
}

impl Default for NewFormState {
    fn default() -> Self {
        Self {
            name: String::new(),
            format: "ndjson".to_string(),
            selected: Vec::new(),
            patients: Vec::new(),
            groups: Vec::new(),
            since_preset: String::new(),
            since_custom: String::new(),
            client_tracking_id: String::new(),
            header_checked: true,
            param_values: Vec::new(),
        }
    }
}

impl NewFormState {
    /// A `since_preset` outside the known list renders as "All time" rather
    /// than echoing a value the `<select>` has no matching `<option>` for
    /// (#836) — the same normalization Bulk Export's own `StartForm`
    /// applies to its Since field.
    fn normalize_since_preset(&mut self) {
        if !matches!(
            self.since_preset.as_str(),
            "" | "day" | "week" | "month" | "custom"
        ) {
            self.since_preset.clear();
        }
    }
}

/// Field-level validation errors [`start`] can produce beyond the single
/// `start_error` banner shared with the subjects-selection checks (#836):
/// the "Narrow it down"/"Advanced" fields each own an inline error slot,
/// the same idiom Bulk Export's Since field uses.
#[derive(Default)]
struct NewFormErrors {
    since_custom: Option<String>,
    client_tracking_id: Option<String>,
    /// One entry per invalid `param:{reference}:{name}` field (#837) — see
    /// [`ParamFieldError`] for why this collects every failing field
    /// instead of stopping at the first.
    param_errors: Vec<ParamFieldError>,
}

/// Validates and types every value submitted for one checked SQL Query
/// subject's declared parameters (#837): for each of `declared`, the
/// trimmed value found in `submitted` under `reference`/its name —
///
/// 1. empty with no default → a "required" [`ParamFieldError`];
/// 2. empty with a default → skipped: the server applies its own default,
///    so nothing is stored or sent for it;
/// 3. non-empty → checked against [`value_matches_type`]; a mismatch is a
///    "expected a {type}" [`ParamFieldError`], a match becomes a
///    [`JobParameter`].
///
/// Every declared parameter is checked, not just up to the first failure —
/// [`start`] accumulates every subject's errors together so a submission
/// with several bad fields is corrected in one round trip.
fn validate_subject_parameters(
    reference: &str,
    declared: &[crate::sql_libraries::DeclaredParameter],
    submitted: &[ParamValue],
    i18n: &I18n,
) -> (Vec<JobParameter>, Vec<ParamFieldError>) {
    let mut params = Vec::with_capacity(declared.len());
    let mut errors = Vec::new();
    for p in declared {
        let raw = submitted
            .iter()
            .find(|v| v.reference == reference && v.name == p.name)
            .map(|v| v.value.trim().to_string())
            .unwrap_or_default();
        if raw.is_empty() {
            if p.default.is_none() {
                errors.push(ParamFieldError {
                    reference: reference.to_string(),
                    name: p.name.clone(),
                    message: i18n.t("sql-export-param-required"),
                });
            }
            continue;
        }
        if value_matches_type(&p.type_code, &raw) {
            params.push(JobParameter {
                name: p.name.clone(),
                r#type: p.type_code.clone(),
                value: raw,
            });
        } else {
            errors.push(ParamFieldError {
                reference: reference.to_string(),
                name: p.name.clone(),
                message: i18n.t_arg(
                    "sql-export-param-type-mismatch",
                    "type",
                    p.type_code.clone(),
                ),
            });
        }
    }
    (params, errors)
}

/// Whether `value` (already trimmed and known non-empty) is well-formed for
/// `type_code` — the same FHIR primitive shapes
/// [`crate::conformance::typed_parameter_value`] eventually encodes a valid
/// value of that type as (#837), so a value this accepts is a value that
/// function will never have to fall back to `valueString` for. A type this
/// function does not specifically recognize (`string`, `code`, `uri`, an
/// unrecognized code, …) accepts anything: the profile itself does not
/// further constrain those shapes, and the server is the authority on
/// whether the value it eventually binds actually parses as its declared
/// type.
fn value_matches_type(type_code: &str, value: &str) -> bool {
    match type_code {
        "integer" | "integer64" => value.parse::<i64>().is_ok(),
        "positiveInt" => value.parse::<i64>().is_ok_and(|n| n >= 1),
        "unsignedInt" => value.parse::<i64>().is_ok_and(|n| n >= 0),
        "decimal" => value.parse::<f64>().is_ok_and(f64::is_finite),
        "boolean" => matches!(value, "true" | "false"),
        "date" => is_valid_date(value),
        "dateTime" | "instant" => chrono::DateTime::parse_from_rfc3339(value).is_ok(),
        "time" => is_valid_time(value),
        _ => true,
    }
}

/// A strict `YYYY-MM-DD` calendar date: fixed-width digit groups (so
/// `2026-1-1` is rejected, unlike a lenient `chrono` parse) that also name a
/// real day (so `2026-02-30` is rejected).
fn is_valid_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    if !value[0..4].bytes().all(|b| b.is_ascii_digit())
        || !value[5..7].bytes().all(|b| b.is_ascii_digit())
        || !value[8..10].bytes().all(|b| b.is_ascii_digit())
    {
        return false;
    }
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
}

/// A strict `HH:MM:SS` or `HH:MM:SS.fff` time of day, fixed-width the same
/// way [`is_valid_date`] is.
fn is_valid_time(value: &str) -> bool {
    let (main, fraction) = match value.split_once('.') {
        Some((main, fraction)) => (main, Some(fraction)),
        None => (value, None),
    };
    if main.len() != 8 || main.as_bytes()[2] != b':' || main.as_bytes()[5] != b':' {
        return false;
    }
    if let Some(fraction) = fraction
        && (fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    chrono::NaiveTime::parse_from_str(main, "%H:%M:%S").is_ok()
}

/// The SQL Export builder (#833, #834): pick stored subjects from a single
/// filterable table, choose an output format, submit — the create-form half
/// of the workspace's two doors, in Bulk Export's own visual language.
#[derive(Template)]
#[template(path = "pages/sql-export-new.html")]
struct ExportNewPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
    /// Empty renders the "nothing to export yet" card instead of the form —
    /// unless `degraded` is set too, in which case neither the form nor that
    /// card renders, only the notice (an empty result here could just be the
    /// fetch failure, not an actually-empty store).
    subjects: Vec<SubjectRow>,
    total: usize,
    selected_count: usize,
    name: String,
    format: String,
    degraded: Option<String>,
    start_error: Option<String>,
    /// "Narrow it down" (#836): the combobox fallback textareas' own value —
    /// every submitted reference, one per line — plus the version-aware
    /// search hint each field currently has.
    patients_value: String,
    patients_hint: String,
    groups_value: String,
    groups_hint: String,
    since_preset: String,
    since_custom: String,
    since_custom_error: Option<String>,
    /// "Advanced" (#836): the tracking id field's value/error, the header
    /// checkbox's rendered state, and whether the disclosure starts open —
    /// `true` the moment either field carries something worth showing.
    client_tracking_id: String,
    client_tracking_id_error: Option<String>,
    header_checked: bool,
    advanced_open: bool,
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

/// `GET /ui/sql/export/new` — the export builder. `subject` may repeat
/// (`?subject=ViewDefinition/x&subject=Library/y`, prefilling the table's
/// checkboxes), which `Query<T>` cannot express, so the raw query string is
/// parsed by hand the same way [`start`] parses its body.
pub(crate) async fn new_page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
) -> Response {
    let selected = query
        .as_deref()
        .map(|raw| {
            form_urlencoded::parse(raw.as_bytes())
                .filter(|(key, _)| key == "subject")
                .map(|(_, value)| value.into_owned())
                .collect()
        })
        .unwrap_or_default();
    render_new_page(
        &state,
        locale,
        rv.0,
        &rt,
        None,
        NewFormState {
            selected,
            ..Default::default()
        },
        NewFormErrors::default(),
    )
    .await
}

/// Shared render tail of [`new_page`] and every re-render [`start`] falls
/// back to: fetches the current subjects, pairs them with `form`'s selection,
/// and renders the builder — or its empty/degraded states, both decided in
/// the template from `subjects`/`degraded`.
async fn render_new_page(
    state: &WebState,
    locale: RequestLocale,
    version: helios_fhir::FhirVersion,
    rt: &RequestTenant,
    start_error: Option<String>,
    mut form: NewFormState,
    errors: NewFormErrors,
) -> Response {
    form.normalize_since_preset();
    let (subjects, degraded) = export_subjects(state, version, &rt.id).await;
    let total = subjects.len();
    let i18n = I18n::new(locale);
    let (rows, selected_count) = subject_rows(
        subjects,
        &form.selected,
        &form.param_values,
        &errors.param_errors,
        &i18n,
    );
    // Patients' search hint degrades the same way Bulk Export's own field
    // does — a shared, process-wide capability flag, not a per-page one
    // (#836).
    let patients_hint = if state.patient_name_search.load(Ordering::Relaxed) {
        i18n.t("sql-export-field-patients-hint")
    } else {
        i18n.t("sql-export-field-patients-id-only-hint")
    };
    let groups_hint = if crate::lookup::supports_group_name_search(version) {
        i18n.t("sql-export-field-groups-hint-r5")
    } else {
        i18n.t("sql-export-field-groups-hint-r4")
    };
    // Open the moment either "Advanced" field carries something worth
    // showing: a tracking id, its error, or a header box the submission
    // actually unchecked (#836) — never merely because the checkbox sits at
    // its own default `true`.
    let advanced_open = !form.client_tracking_id.is_empty()
        || errors.client_tracking_id.is_some()
        || !form.header_checked;
    render(ExportNewPage {
        status: current_status(state, version, rt),
        i18n,
        active_page: "sql-export",
        subjects: rows,
        total,
        selected_count,
        name: form.name,
        format: form.format,
        degraded,
        start_error,
        patients_value: form.patients.join("\n"),
        patients_hint,
        groups_value: form.groups.join("\n"),
        groups_hint,
        since_preset: form.since_preset,
        since_custom: form.since_custom,
        since_custom_error: errors.since_custom,
        client_tracking_id: form.client_tracking_id,
        client_tracking_id_error: errors.client_tracking_id,
        header_checked: form.header_checked,
        advanced_open,
    })
}

/// `POST /ui/sql/export` — resolve the checked subjects against the same
/// list `/new` offered, validate the job-wide filters (#836), create the job
/// record, kick it off, and land on the list. The form repeats several
/// fields (`subject=` per checkbox, `patient=`/`group=` per combobox
/// selection), which `Form`-into-a-struct cannot express, so the raw body is
/// parsed by hand.
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
    let mut name = String::new();
    let mut format = "ndjson".to_string();
    let mut refs: Vec<String> = Vec::new();
    let mut patients: Vec<String> = Vec::new();
    let mut groups: Vec<String> = Vec::new();
    let mut since_preset = String::new();
    let mut since_custom = String::new();
    let mut client_tracking_id = String::new();
    let mut header_present = false;
    let mut param_values: Vec<ParamValue> = Vec::new();
    for (key, value) in form_urlencoded::parse(&body) {
        // `param:{reference}:{name}` (#837) — the reference itself never
        // contains `:` (neither a FHIR reference nor a parameter name does,
        // per the `sql-name` invariant `^[A-Za-z][A-Za-z0-9_]*$`), so
        // splitting on the first `:` after the fixed `param:` prefix always
        // separates the two cleanly. A key with no second `:` (malformed,
        // never produced by this page's own markup) is simply dropped.
        if let Some(rest) = key.strip_prefix("param:") {
            if let Some((reference, name)) = rest.split_once(':') {
                param_values.push(ParamValue {
                    reference: reference.to_string(),
                    name: name.to_string(),
                    value: value.into_owned(),
                });
            }
            continue;
        }
        match key.as_ref() {
            "name" => name = value.into_owned(),
            "subject" => refs.push(value.into_owned()),
            "format" => format = value.into_owned(),
            "patient" => patients.push(value.into_owned()),
            "group" => groups.push(value.into_owned()),
            "since_preset" => since_preset = value.into_owned(),
            "since_custom" => since_custom = value.into_owned(),
            "client_tracking_id" => client_tracking_id = value.into_owned(),
            // The checkbox contract is presence, like Bulk Export's own
            // `all_types` — its submitted value is never inspected.
            "header" => header_present = true,
            _ => {}
        }
    }
    // Trimmed once, up front — an empty result is never stored (the job's
    // `name`/`filters.clientTrackingId` serialization already skips an empty
    // string), and every re-render below echoes back the exact same trimmed
    // value.
    let name = name.trim().to_string();
    let client_tracking_id = client_tracking_id.trim().to_string();
    // The checkbox's re-rendered state (#836): checked unless this exact
    // submission paired `csv` with an unchecked box. Computed once so every
    // re-render below — whichever field actually failed — shows it exactly
    // as [`start`] would send it.
    let header_checked = if format == "csv" {
        header_present
    } else {
        true
    };
    // A closure rather than a value: every early-return below needs its own
    // independent `NewFormState`, and inlining the same nine-field struct
    // literal at each of seven call sites would otherwise dwarf the actual
    // validation logic.
    let conserved_form = || NewFormState {
        name: name.clone(),
        format: format.clone(),
        selected: refs.clone(),
        patients: patients.clone(),
        groups: groups.clone(),
        since_preset: since_preset.clone(),
        since_custom: since_custom.clone(),
        client_tracking_id: client_tracking_id.clone(),
        header_checked,
        param_values: param_values.clone(),
    };

    if refs.is_empty() {
        return render_new_page(
            &state,
            locale,
            rv.0,
            &rt,
            Some(i18n.t("sql-export-select-subject")),
            conserved_form(),
            NewFormErrors::default(),
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
        return render_new_page(
            &state,
            locale,
            rv.0,
            &rt,
            Some(message),
            conserved_form(),
            NewFormErrors::default(),
        )
        .await;
    }
    let mut subjects = Vec::with_capacity(refs.len());
    // #837: every parameter field error across every checked subject,
    // collected rather than stopped-at-first — see [`ParamFieldError`].
    let mut param_errors: Vec<ParamFieldError> = Vec::new();
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
                conserved_form(),
                NewFormErrors::default(),
            )
            .await;
        };
        let (parameters, mut errors) =
            validate_subject_parameters(reference, &found.parameters, &param_values, &i18n);
        param_errors.append(&mut errors);
        subjects.push(JobSubject {
            name: found.name.clone(),
            reference: reference.clone(),
            kind: found.kind_code.to_string(),
            parameters,
        });
    }
    if !param_errors.is_empty() {
        return render_new_page(
            &state,
            locale,
            rv.0,
            &rt,
            None,
            conserved_form(),
            NewFormErrors {
                param_errors,
                ..Default::default()
            },
        )
        .await;
    }

    // #836: the job-wide filters, validated patients, then groups, then
    // since, then the tracking id — the first failing check re-renders with
    // its own message/field error, and every step that already passed keeps
    // its now-known-good value instead of being re-parsed from the raw
    // submission a second time.
    let patient_refs = match crate::lookup::parse_reference_list("Patient", &patients) {
        Ok(refs) => refs,
        Err(()) => {
            return render_new_page(
                &state,
                locale,
                rv.0,
                &rt,
                Some(i18n.t("sql-export-patient-invalid")),
                conserved_form(),
                NewFormErrors::default(),
            )
            .await;
        }
    };
    let group_refs = match crate::lookup::parse_reference_list("Group", &groups) {
        Ok(refs) => refs,
        Err(()) => {
            return render_new_page(
                &state,
                locale,
                rv.0,
                &rt,
                Some(i18n.t("sql-export-group-invalid")),
                conserved_form(),
                NewFormErrors::default(),
            )
            .await;
        }
    };
    let since = match crate::lookup::since_instant(&since_preset, &since_custom) {
        Ok(since) => since,
        Err(()) => {
            return render_new_page(
                &state,
                locale,
                rv.0,
                &rt,
                None,
                conserved_form(),
                NewFormErrors {
                    since_custom: Some(i18n.t("sql-export-since-invalid")),
                    ..Default::default()
                },
            )
            .await;
        }
    };
    if client_tracking_id.chars().count() > 200 {
        return render_new_page(
            &state,
            locale,
            rv.0,
            &rt,
            None,
            conserved_form(),
            NewFormErrors {
                client_tracking_id: Some(i18n.t("sql-export-tracking-id-too-long")),
                ..Default::default()
            },
        )
        .await;
    }
    // `header` is only ever meaningful for `csv` — every other format sends
    // no filter at all, matching `sql_export_parameters_body`'s own "only
    // when `Some`" rule.
    let header = (format == "csv").then_some(header_present);

    let caller = Caller::from_request(&headers, &rt.id);
    let mut job = ExportJob {
        name,
        subjects,
        format,
        filters: JobFilters {
            patients: patient_refs,
            groups: group_refs,
            since,
            header,
            client_tracking_id,
        },
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
/// `name`/`subjects`/`format`/`filters` — each subject's own `parameters`
/// travel along inside its cloned `JobSubject` — kicks it off under
/// `caller`, and stores it as a brand-new record via [`store_new_job`] — the
/// resubmission [`retry`] and [`rerun`] share: `source` itself is never
/// written back to.
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
        filters: source.filters.clone(),
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

// ---------------------------------------------------------------------------
// Job detail (#835): GET /ui/sql/export/{id} and its htmx fragment
// ---------------------------------------------------------------------------

/// A job's `kind` code's display label ("ViewDefinition", "SQL Query", "SQL
/// View") — the same literal English labels [`export_subjects`] assigns a
/// live [`ExportSubject`], applied here to a stored [`JobSubject::kind`]
/// instead. Empty for a code this build no longer recognizes, rather than a
/// guess.
fn kind_label(kind: &str) -> &'static str {
    match kind {
        "view-definition" => "ViewDefinition",
        "sql-query" => "SQL Query",
        "sql-view" => "SQL View",
        _ => "",
    }
}

/// One `.tag.tag--type` pill: a subject's kind and display name. Shared by
/// the Job card's Subjects field (every subject in the record) and an Output
/// row's resolved Subject column (at most one, by name match).
struct SubjectTag {
    kind_label: &'static str,
    name: String,
    /// This subject's stored parameter values (#837), `(name, value)` pairs
    /// in submission order — rendered as `:name = value` chips right after
    /// the subject's name in the Job card's Subjects field. Always empty for
    /// a ViewDefinition, a SQL View, or an unparameterized SQL Query, and
    /// always empty on an Output row's own Subject column (only the Job
    /// card's Subjects field shows chips; see [`resolve_output_subject`]).
    parameters: Vec<(String, String)>,
}

/// One download link in an Output row's Files column: the label shown next
/// to the download icon, and the `href` — already same-origin (#833).
struct FileLink {
    label: String,
    href: String,
}

/// One row of the Output files table: the manifest's own output `name`, the
/// subject it resolved back to (or `None` — an em dash, when no subject's
/// disambiguated output name matches), and one [`FileLink`] per shard.
struct OutputRow {
    name: String,
    subject: Option<SubjectTag>,
    files: Vec<FileLink>,
}

/// `YYYY-MM-DD HH:MM UTC`, unconditionally — the header lede's own format.
/// Unlike the list card's [`format_hour`], the date is never dropped even
/// when `stamp` falls on today: a job's permalink is read long after "today"
/// has passed. Empty when `stamp` does not parse.
fn format_timestamp_minutes(stamp: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(stamp)
        .map(|parsed| {
            parsed
                .with_timezone(&Utc)
                .format("%Y-%m-%d %H:%M UTC")
                .to_string()
        })
        .unwrap_or_default()
}

/// `YYYY-MM-DD HH:MM:SS UTC` — the Job card's Started field, the one place
/// seconds are shown. Empty when `stamp` does not parse.
fn format_timestamp_seconds(stamp: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(stamp)
        .map(|parsed| {
            parsed
                .with_timezone(&Utc)
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string()
        })
        .unwrap_or_default()
}

/// The subject `output_name` came from, found by running
/// [`subject_output_names`]'s own disambiguation backwards: the manifest
/// carries no subject association at all, and the server runs views before
/// queries, so a job's `outputs` do not land in submission order — matching
/// by output name is the only association available.
fn resolve_output_subject(job: &ExportJob, output_name: &str) -> Option<SubjectTag> {
    subject_output_names(&job.subjects)
        .iter()
        .zip(&job.subjects)
        .find(|(name, _)| name.as_str() == output_name)
        .map(|(_, subject)| SubjectTag {
            kind_label: kind_label(&subject.kind),
            name: subject.name.clone(),
            // Deliberately empty (#837): the Output files table's Subject
            // column is a resolved-by-name pointer back to the Job card's
            // own Subjects field, which is the only place chips render.
            parameters: Vec::new(),
        })
}

/// `location`'s last path segment, query dropped — the label shown next to
/// each download link. Falls back to the translated "File n" (`n` 1-based
/// within the output's own shards) when nothing usable can be derived (an
/// empty path, or a location ending in `/`).
fn file_label(i18n: &I18n, location: &str, n: usize) -> String {
    let without_query = location.split('?').next().unwrap_or(location);
    match without_query.rsplit('/').next() {
        Some(segment) if !segment.is_empty() => segment.to_string(),
        _ => i18n.t_arg("sql-export-file-fallback", "n", n.to_string()),
    }
}

/// Every row of the Output files table, in the record's own `outputs`
/// order — the manifest order [`poll_job`] persisted, not submission order.
fn build_output_rows(i18n: &I18n, job: &ExportJob) -> Vec<OutputRow> {
    job.outputs
        .iter()
        .map(|output| OutputRow {
            name: output.name.clone(),
            subject: resolve_output_subject(job, &output.name),
            files: output
                .locations
                .iter()
                .enumerate()
                .map(|(index, location)| FileLink {
                    label: file_label(i18n, location, index + 1),
                    href: location.clone(),
                })
                .collect(),
        })
        .collect()
}

/// A `failed` job's notice: the server's own error message either names a
/// subject this UI can point to, or does not — a generic failure never
/// invents a cause.
enum FailureNotice {
    /// `error` contained `view '<name>'`/`query '<name>'` and `<name>`
    /// matched one of the job's disambiguated output names: the subject's
    /// name, and the remainder of the message after `'<name>': `.
    Subject(String, String),
    /// No match — a kick-off failure, or a message this UI does not
    /// recognize. The raw error, untouched.
    Generic(String),
}

/// Parses `error` for the server's own `view '<name>'` / `query '<name>'`
/// pattern (whichever occurs first) and checks `<name>` against `subjects`'
/// disambiguated output names ([`subject_output_names`]) — a match returns
/// the name and everything after `'<name>': `; anything else (no pattern, or
/// a name matching no known subject) returns `None` so the caller falls back
/// to the raw message.
fn failed_subject(error: &str, subjects: &[JobSubject]) -> Option<(String, String)> {
    let (marker_at, marker) = ["view '", "query '"]
        .iter()
        .filter_map(|marker| error.find(marker).map(|at| (at, *marker)))
        .min_by_key(|(at, _)| *at)?;
    let after_marker = &error[marker_at + marker.len()..];
    let name_end = after_marker.find('\'')?;
    let name = &after_marker[..name_end];
    let after_name = &after_marker[name_end + 1..];
    let rest = after_name.strip_prefix(": ").unwrap_or(after_name);
    subject_output_names(subjects)
        .iter()
        .any(|output_name| output_name == name)
        .then(|| (name.to_string(), rest.to_string()))
}

/// [`FailureNotice`] for `job`, `None` outside `failed`.
fn failure_notice(job: &ExportJob) -> Option<FailureNotice> {
    if job.status != "failed" {
        return None;
    }
    Some(match failed_subject(&job.error, &job.subjects) {
        Some((name, rest)) => FailureNotice::Subject(name, rest),
        None => FailureNotice::Generic(job.error.clone()),
    })
}

/// The Job card's Duration field: the elapsed time in a terminal state, or
/// the same progress-or-waiting text the header lede shows while
/// `in-progress`.
fn duration_label(i18n: &I18n, job: &ExportJob) -> String {
    if job.status == "in-progress" {
        if job.progress.is_empty() {
            i18n.t("sql-export-progress-waiting")
        } else {
            job.progress.clone()
        }
    } else {
        elapsed(job)
    }
}

/// The header's `.page-head__lede`: worded differently per status, unlike
/// the list card's single shared [`job_meta`] line.
fn detail_lede(i18n: &I18n, job: &ExportJob) -> String {
    let format = format_label(i18n, &job.format);
    match job.status.as_str() {
        "complete" => {
            let mut lede = format!(
                "{} {}",
                i18n.t("sql-export-detail-finished"),
                format_timestamp_minutes(&job.finished_at)
            );
            let elapsed = elapsed(job);
            if !elapsed.is_empty() {
                lede.push_str(&format!(" · {elapsed}"));
            }
            lede.push_str(&format!(" · {format}"));
            lede
        }
        "failed" => {
            let mut lede = format!(
                "{} {}",
                i18n.t("sql-export-detail-failed"),
                format_timestamp_minutes(&job.finished_at)
            );
            let elapsed = elapsed(job);
            if !elapsed.is_empty() {
                lede.push_str(&format!(
                    " · {} {elapsed}",
                    i18n.t("sql-export-detail-after")
                ));
            }
            lede.push_str(&format!(" · {format}"));
            lede
        }
        "cancelled" => {
            let mut lede = format!(
                "{} {}",
                i18n.t("sql-export-detail-cancelled"),
                format_timestamp_minutes(&job.finished_at)
            );
            lede.push_str(&format!(" · {format}"));
            if !job.error.is_empty() {
                lede.push_str(&format!(" · {}", job.error));
            }
            lede
        }
        // in-progress, and defensively any other/unknown value.
        _ => {
            let progress = if job.progress.is_empty() {
                i18n.t("sql-export-progress-waiting")
            } else {
                job.progress.clone()
            };
            let mut lede = format!(
                "{} {} · {progress} · {format}",
                i18n.t("sql-export-detail-started"),
                format_timestamp_minutes(&job.started_at)
            );
            if !job.poll_error.is_empty() {
                lede.push_str(&format!(
                    " · {}: {}",
                    i18n.t("sql-export-status-unavailable"),
                    job.poll_error
                ));
            }
            lede
        }
    }
}

/// The Job card's Format field (#836): [`format_label`] unchanged for every
/// job except a `csv` one whose `filters.header` is a known choice, which
/// appends " · with header row" / " · no header row" — the same suffix the
/// Playwright AC (`sql-export.spec.ts`) checks for after unchecking the
/// header switch and submitting. `filters.header` is `None` for any
/// non-`csv` job and for a record persisted before #836, in which case this
/// falls back to the plain label exactly as the list card's own
/// [`job_meta`] still renders it — that line is deliberately left
/// untouched by #836.
fn detail_format_label(i18n: &I18n, job: &ExportJob) -> String {
    let base = format_label(i18n, &job.format);
    if job.format != "csv" {
        return base;
    }
    match job.filters.header {
        Some(true) => format!("{base} · {}", i18n.t("sql-export-detail-header-included")),
        Some(false) => format!("{base} · {}", i18n.t("sql-export-detail-header-omitted")),
        None => base,
    }
}

/// The `/ui/sql/export/{id}` job detail's view model (#835): everything that
/// varies with the job's status, built once by [`build_job_detail`] over
/// [`ExportJob`] + i18n so neither template branches on raw job fields
/// beyond `status` itself (which action/overflow/polling markup applies).
struct JobDetail {
    id: String,
    name: String,
    status: String,
    status_label: String,
    lede: String,
    warning: Option<FailureNotice>,
    /// Only meaningful — and only rendered — while `status` is
    /// `in-progress`.
    progress_pct: String,
    job_id: String,
    /// [`detail_format_label`]'s own header-aware label — not the plain
    /// [`format_label`] the list card uses.
    format_label: String,
    started_label: String,
    duration_label: String,
    /// The Job card's Tracking id field (#836): `filters.client_tracking_id`
    /// verbatim, empty when the job carries none — the template skips the
    /// whole `detail__field` in that case.
    tracking_id: String,
    /// The Job card's Since field (#836): `filters.since`, the RFC 3339
    /// instant already resolved at submission time — never a raw preset
    /// name — empty (and unrendered) when the job carries no Since filter.
    since_label: String,
    /// The Job card's Patients field (#836): `filters.patients` verbatim, one
    /// `.tag` chip per reference; empty (and unrendered) when the job
    /// carries no Patients filter.
    patients: Vec<String>,
    /// The Job card's Groups field (#836), the same shape as `patients`.
    groups: Vec<String>,
    subjects: Vec<SubjectTag>,
    outputs: Vec<OutputRow>,
    /// The toolbar's file count: the sum of every output's `locations`, not
    /// the number of output rows.
    output_count: usize,
}

fn build_job_detail(i18n: &I18n, id: &str, job: &ExportJob) -> JobDetail {
    JobDetail {
        id: id.to_string(),
        name: card_name(job),
        status: job.status.clone(),
        status_label: status_label(i18n, &job.status),
        lede: detail_lede(i18n, job),
        warning: failure_notice(job),
        progress_pct: progress_pct(&job.status, &job.progress),
        job_id: job.job_id.clone(),
        format_label: detail_format_label(i18n, job),
        started_label: format_timestamp_seconds(&job.started_at),
        duration_label: duration_label(i18n, job),
        tracking_id: job.filters.client_tracking_id.clone(),
        since_label: job.filters.since.clone(),
        patients: job.filters.patients.clone(),
        groups: job.filters.groups.clone(),
        subjects: job
            .subjects
            .iter()
            .map(|subject| SubjectTag {
                kind_label: kind_label(&subject.kind),
                name: subject.name.clone(),
                parameters: subject
                    .parameters
                    .iter()
                    .map(|p| (p.name.clone(), p.value.clone()))
                    .collect(),
            })
            .collect(),
        output_count: job.outputs.iter().map(|o| o.locations.len()).sum(),
        outputs: build_output_rows(i18n, job),
    }
}

/// The job detail's status-dependent region (#835): the header (back link,
/// name, lede, contextual action, status chip, overflow), the failure
/// notice, the progress bar, and the Job/Output files cards — written once
/// here and rendered by two callers, the same way
/// `partials/sql_run_results.html`/`RunResultsPartial` already share their
/// own markup between a page and its own htmx endpoint:
///
/// - Nested as a template field of [`DetailPage`] for `GET
///   /ui/sql/export/{id}`'s own render.
/// - Directly as the whole response of [`detail_fragment`] (`GET
///   /ui/sql/export/{id}/detail`), which this same element's own
///   `hx-get`/`hx-trigger="every 5s"` calls back into while the job stays
///   `in-progress` — a terminal job's `#job-detail` carries neither
///   attribute, so polling stops itself the moment the swap that lands the
///   terminal state runs.
#[derive(Template)]
#[template(path = "partials/sql_export_detail.html")]
struct JobDetailFragment {
    i18n: I18n,
    detail: JobDetail,
}

/// `GET /ui/sql/export/{id}` (#835): the job's own permalink, in the full
/// shell.
#[derive(Template)]
#[template(path = "pages/sql-export-detail.html")]
struct DetailPage {
    status: Status,
    i18n: I18n,
    active_page: &'static str,
    fragment: JobDetailFragment,
}

/// Loads `id`'s record for the request's user/tenant, polling it once and
/// persisting the transition with the same CAS [`card`] uses if it is still
/// `in-progress` — the shared body of [`detail_page`] and
/// [`detail_fragment`]. `None` when `id` names nothing this user/tenant
/// owns — never distinguished from "does not exist", so a foreign id reads
/// exactly like an unknown one.
async fn load_detail(
    state: &WebState,
    i18n: &I18n,
    tenant: &str,
    principal: Option<&helios_auth::Principal>,
    headers: &HeaderMap,
    id: &str,
) -> Option<JobDetail> {
    let user_key = settings_user_key(principal);
    let snapshot = load_jobs(state, &user_key, tenant).await;
    let original = snapshot.jobs.get(id)?;
    let mut job = parse_job(original);
    if job.status == "in-progress" {
        let caller = Caller::from_request(headers, tenant);
        poll_job(state, &mut job, &caller, i18n).await;
        let _ = store_job_conditionally(
            state,
            &user_key,
            tenant,
            id,
            &job,
            snapshot.version,
            MemberExpectation::Unchanged(original),
        )
        .await;
    }
    Some(build_job_detail(i18n, id, &job))
}

/// `GET /ui/sql/export/{id}` — the job's own permalink inside the full
/// shell. Renders a `404` inside the shell when `id` names nothing this
/// user/tenant owns, or there is no settings store to look it up in —
/// indistinguishable from a foreign id or an unknown one.
pub(crate) async fn detail_page(
    State(state): State<WebState>,
    locale: RequestLocale,
    rv: RequestVersion,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let i18n = I18n::new(locale);
    let status = current_status(&state, rv.0, &rt);
    match load_detail(&state, &i18n, &rt.id, principal.as_deref(), &headers, &id).await {
        Some(detail) => render(DetailPage {
            status,
            i18n,
            active_page: "sql-export",
            fragment: JobDetailFragment { i18n, detail },
        }),
        None => render_not_found(
            status,
            i18n,
            "sql-export",
            "/ui/sql/export",
            i18n.t("sql-export-active-title"),
        ),
    }
}

/// `GET /ui/sql/export/{id}/detail` — the same `#job-detail` content
/// [`detail_page`] renders, without the shell: htmx's own 5s refresh while
/// the job stays `in-progress`. `404` with no body for an id this
/// user/tenant does not own, matching [`card`].
pub(crate) async fn detail_fragment(
    State(state): State<WebState>,
    locale: RequestLocale,
    rt: RequestTenant,
    principal: Option<Extension<helios_auth::Principal>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let i18n = I18n::new(locale);
    match load_detail(&state, &i18n, &rt.id, principal.as_deref(), &headers, &id).await {
        Some(detail) => render(JobDetailFragment { i18n, detail }),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_outputs_reads_the_manifest_shape() {
        let manifest = json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "exportId", "valueString": "job-1"},
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
            parameters: vec![],
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
        assert_eq!(value["filters"], Value::Null);
        assert_eq!(value["status"], "in-progress");
        assert_eq!(value["format"], "csv");
    }

    /// #836: a job with every filter set serializes `filters`, and
    /// `job_merge_value` carries it through instead of nulling it out.
    #[test]
    fn job_merge_value_writes_filters_when_present() {
        let job = ExportJob {
            status: "in-progress".to_string(),
            started_at: "2026-09-01T09:00:00Z".to_string(),
            format: "csv".to_string(),
            filters: JobFilters {
                patients: vec!["Patient/p1".to_string()],
                groups: vec![],
                since: "2026-01-01T00:00:00Z".to_string(),
                header: Some(true),
                client_tracking_id: "trk-1".to_string(),
            },
            ..Default::default()
        };
        let value = job_merge_value(&job);
        assert_eq!(value["filters"]["patients"], json!(["Patient/p1"]));
        assert_eq!(value["filters"]["since"], "2026-01-01T00:00:00Z");
        assert_eq!(value["filters"]["header"], true);
        assert_eq!(value["filters"]["clientTrackingId"], "trk-1");
        // An empty `groups` is still an explicit key with `[]`, not omitted —
        // serialization is all-or-nothing per JobFilters' own docs, so a
        // round trip through job_merge_value never silently drops a field.
        assert_eq!(value["filters"]["groups"], json!([]));
    }

    /// A `JobFilters` with only one field set (e.g. just `header`) still
    /// serializes every key — the "all-or-nothing" rule is about the whole
    /// object collapsing to `null`, never about individual fields dropping
    /// out while siblings are set.
    #[test]
    fn job_merge_value_writes_every_filter_key_even_when_only_one_filter_is_set() {
        let job = ExportJob {
            status: "in-progress".to_string(),
            started_at: "2026-09-01T09:00:00Z".to_string(),
            filters: JobFilters {
                header: Some(false),
                ..Default::default()
            },
            ..Default::default()
        };
        let value = job_merge_value(&job);
        assert_eq!(value["filters"]["patients"], json!([]));
        assert_eq!(value["filters"]["groups"], json!([]));
        assert_eq!(value["filters"]["since"], "");
        assert_eq!(value["filters"]["header"], false);
        assert_eq!(value["filters"]["clientTrackingId"], "");
    }

    /// #836/#837: a job carrying `filters` and a parameterized subject
    /// round-trips through JSON unchanged.
    #[test]
    fn export_job_round_trips_filters_and_subject_parameters() {
        let job = ExportJob {
            subjects: vec![JobSubject {
                name: "ward_counts".to_string(),
                reference: "Library/q1".to_string(),
                kind: "sql-query".to_string(),
                parameters: vec![JobParameter {
                    name: "ward".to_string(),
                    r#type: "string".to_string(),
                    value: "west".to_string(),
                }],
            }],
            format: "csv".to_string(),
            filters: JobFilters {
                patients: vec!["Patient/p1".to_string()],
                groups: vec!["Group/g1".to_string()],
                since: "2026-01-01T00:00:00Z".to_string(),
                header: Some(true),
                client_tracking_id: "trk-1".to_string(),
            },
            ..Default::default()
        };
        let value = serde_json::to_value(&job).expect("job serializes");
        let round_tripped: ExportJob =
            serde_json::from_value(value).expect("job deserializes back");
        assert_eq!(round_tripped.filters, job.filters);
        assert_eq!(round_tripped.subjects, job.subjects);
    }

    /// #836/#837: a record persisted before `filters`/`parameters` existed
    /// still deserializes, with both defaulting to empty.
    #[test]
    fn export_job_deserializes_a_pre_filters_record_with_empty_defaults() {
        let legacy = json!({
            "jobId": "job-1",
            "subjects": [{
                "name": "patients",
                "reference": "ViewDefinition/vd1",
                "kind": "view-definition",
            }],
            "format": "csv",
            "status": "complete",
            "startedAt": "2026-01-01T09:00:00Z",
        });
        let job: ExportJob = serde_json::from_value(legacy).expect("legacy record deserializes");
        assert_eq!(job.filters, JobFilters::default());
        assert!(job.subjects[0].parameters.is_empty());
    }

    // -----------------------------------------------------------------------
    // Job detail (#835)
    // -----------------------------------------------------------------------

    #[test]
    fn failed_subject_names_the_matching_subject_and_returns_the_remainder() {
        let subjects = vec![subject("v03_counts", "sql-query")];
        let (name, rest) = failed_subject(
            "Export job 'x' failed: query 'v03_counts': column \"ward\" does not exist",
            &subjects,
        )
        .expect("the query pattern names a known subject");
        assert_eq!(name, "v03_counts");
        assert_eq!(rest, "column \"ward\" does not exist");
    }

    #[test]
    fn failed_subject_also_recognizes_the_view_marker_and_picks_the_earliest_match() {
        let subjects = vec![subject("patients_flat", "view-definition")];
        let (name, rest) = failed_subject(
            "Export job 'x' failed: view 'patients_flat': invalid FHIRPath expression",
            &subjects,
        )
        .expect("the view pattern names a known subject");
        assert_eq!(name, "patients_flat");
        assert_eq!(rest, "invalid FHIRPath expression");
    }

    #[test]
    fn failed_subject_is_none_when_the_name_matches_no_subject_or_the_pattern_is_absent() {
        let subjects = vec![subject("patients_flat", "view-definition")];
        // The pattern is present, but names something this job never ran.
        assert!(
            failed_subject(
                "Export job 'x' failed: query 'unrelated_view': boom",
                &subjects
            )
            .is_none()
        );
        // No kick-off ever reaches a view/query — a plain transport failure.
        assert!(failed_subject("connection refused", &subjects).is_none());
    }

    #[test]
    fn resolve_output_subject_matches_by_disambiguated_output_name() {
        let job = ExportJob {
            subjects: vec![
                subject("patients_flat", "view-definition"),
                subject("patients_flat", "sql-query"),
            ],
            ..Default::default()
        };
        // `patients_flat-2` is the second occurrence's disambiguated name
        // (subject_output_names), not a subject's own stored name.
        let resolved = resolve_output_subject(&job, "patients_flat-2").expect("a match");
        assert_eq!(resolved.kind_label, "SQL Query");
        assert_eq!(resolved.name, "patients_flat");
        assert!(resolve_output_subject(&job, "no_such_output").is_none());
    }

    #[test]
    fn file_label_uses_the_last_path_segment_without_the_query_and_falls_back_to_file_n() {
        let i18n = I18n::from_tag("en").unwrap();
        assert_eq!(
            file_label(&i18n, "/export/job-1/patients-0.csv?sig=abc", 1),
            "patients-0.csv"
        );
        assert_eq!(file_label(&i18n, "/export/job-1/", 3), "File 3");
        assert_eq!(file_label(&i18n, "", 1), "File 1");
    }

    #[test]
    fn build_output_rows_resolves_subjects_regardless_of_manifest_order() {
        let job = ExportJob {
            subjects: vec![
                subject("patients", "view-definition"),
                subject("encounters", "sql-query"),
            ],
            outputs: vec![
                // The manifest lists `encounters` before `patients` — the
                // server runs queries after views but that is not what
                // determined this order; the resolution must not assume
                // submission order either.
                JobOutput {
                    name: "encounters".to_string(),
                    locations: vec!["/export/job-1/encounters-0.csv".to_string()],
                },
                JobOutput {
                    name: "patients".to_string(),
                    locations: vec![
                        "/export/job-1/patients-0.csv".to_string(),
                        "/export/job-1/patients-1.csv".to_string(),
                    ],
                },
            ],
            ..Default::default()
        };
        let i18n = I18n::from_tag("en").unwrap();
        let rows = build_output_rows(&i18n, &job);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "encounters");
        assert_eq!(rows[0].subject.as_ref().unwrap().kind_label, "SQL Query");
        assert_eq!(rows[0].files.len(), 1);
        assert_eq!(rows[1].name, "patients");
        assert_eq!(
            rows[1].subject.as_ref().unwrap().kind_label,
            "ViewDefinition"
        );
        assert_eq!(rows[1].files.len(), 2);
        assert_eq!(rows[1].files[0].label, "patients-0.csv");
        assert_eq!(rows[1].files[1].label, "patients-1.csv");
    }

    #[test]
    fn detail_lede_reads_differently_per_status() {
        let i18n = I18n::from_tag("en").unwrap();
        let mut job = ExportJob {
            format: "parquet".to_string(),
            started_at: "2026-09-01T09:00:00Z".to_string(),
            finished_at: "2026-09-01T09:05:08Z".to_string(),
            ..Default::default()
        };

        job.status = "complete".to_string();
        assert_eq!(
            detail_lede(&i18n, &job),
            "Finished 2026-09-01 09:05 UTC · 5m 08s · Parquet"
        );

        job.status = "failed".to_string();
        assert_eq!(
            detail_lede(&i18n, &job),
            "Failed 2026-09-01 09:05 UTC · after 5m 08s · Parquet"
        );

        job.status = "cancelled".to_string();
        job.error = "the server no longer knows this job".to_string();
        assert_eq!(
            detail_lede(&i18n, &job),
            "Cancelled 2026-09-01 09:05 UTC · Parquet · the server no longer knows this job"
        );

        job.status = "in-progress".to_string();
        job.error.clear();
        job.progress = "40%".to_string();
        assert_eq!(
            detail_lede(&i18n, &job),
            "Started 2026-09-01 09:00 UTC · 40% · Parquet"
        );

        job.progress.clear();
        job.poll_error = "status poll answered 401".to_string();
        assert_eq!(
            detail_lede(&i18n, &job),
            "Started 2026-09-01 09:00 UTC · Waiting for the first status report… · Parquet · status unavailable: status poll answered 401"
        );
    }

    #[test]
    fn failure_notice_is_none_outside_failed() {
        let mut job = ExportJob {
            status: "in-progress".to_string(),
            error: "query 'v03_counts': boom".to_string(),
            subjects: vec![subject("v03_counts", "sql-query")],
            ..Default::default()
        };
        assert!(failure_notice(&job).is_none());
        job.status = "failed".to_string();
        assert!(matches!(
            failure_notice(&job),
            Some(FailureNotice::Subject(name, _)) if name == "v03_counts"
        ));
    }

    /// #836: the header suffix only ever appears for `csv` with a known
    /// `filters.header` — every other combination falls back to the plain
    /// [`format_label`].
    #[test]
    fn detail_format_label_adds_the_header_choice_only_for_csv_with_a_known_header() {
        let i18n = I18n::from_tag("en").unwrap();
        let mut job = ExportJob {
            format: "csv".to_string(),
            filters: JobFilters {
                header: Some(true),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(detail_format_label(&i18n, &job), "CSV · with header row");

        job.filters.header = Some(false);
        assert_eq!(detail_format_label(&i18n, &job), "CSV · no header row");

        // No header choice recorded (a pre-#836 record, or a csv job that
        // never went through the "Advanced" disclosure at all): the plain
        // label, matching the list card's own format_label.
        job.filters.header = None;
        assert_eq!(detail_format_label(&i18n, &job), "CSV");

        // A header choice is only ever meaningful for csv — a non-csv job
        // ignores it even if a stray `filters.header` were somehow set.
        job.format = "parquet".to_string();
        job.filters.header = Some(true);
        assert_eq!(detail_format_label(&i18n, &job), "Parquet");
    }
}
