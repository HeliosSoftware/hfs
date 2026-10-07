//! In-memory `ExportJobController` implementation.
//!
//! Each job runs inside a `tokio::spawn` task, bounded by a `Semaphore`.
//! Results are stored in a `DashMap<JobId, JobStatus>`.
//!
//! A job carries a mixture of subjects (see [`ExportWork`]), computed against
//! one snapshot of the data and written into one manifest:
//! - **Views** — each named ViewDefinition is run through the `SofRunner` and
//!   its rows are sharded into output files.
//! - **SQL queries** — each named SQLQuery/SQLView Library's fully-resolved
//!   dependency graph ([`crate::handlers::sof::graph`]'s two-phase resolver)
//!   is materialized into an in-memory SQLite engine — leaf ViewDefinitions
//!   via the `SofRunner`, interior SQLView nodes by running their own
//!   (already-validated) SQL — then the subject's own SQL is executed and
//!   the result rows are sharded into output files.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use futures::StreamExt;
use helios_persistence::core::sof_runner::{RowStream, SofRunner, ViewFilters};
use helios_sof::sqlquery::{InMemorySqlEngine, QueryResult};
use tokio::sync::Semaphore;
use tracing::{debug, warn};
use uuid::Uuid;

use super::controller::{
    CompletedFile, ExportError, ExportJobController, ExportTask, JobId, JobStatus, NamedSqlQuery,
    NamedView, SqlExportLimits,
};
use super::planner;
use super::sink::{ExportSink, JobManifest, MANIFEST_VERSION, ManifestFile};
use crate::error::RestError;
use crate::handlers::sof::run::map_sof_error_to_rest;
use crate::handlers::sof::sqlquery::sqlquery_err_to_rest;
use helios_persistence::core::sof_runner::SofError;
use helios_persistence::tenant::TenantContext;

/// Why a job failed and how its result endpoint reports it. Every worker
/// error funnels through here: a failure that is the request's own keeps the
/// 4xx and issue code `$sql-run` would have answered with, everything else
/// is a `500` whose text (backend or driver detail) goes to the job log only;
/// the stored result is generic (see [`server_fault_message`]).
#[derive(Debug)]
struct JobFailure {
    message: String,
    status: StatusCode,
    code: &'static str,
}

impl JobFailure {
    fn server(message: String) -> Self {
        Self {
            message,
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "processing",
        }
    }

    /// A client-attributable REST error becomes a client failure with its
    /// own wording; a server error keeps `detail` (the REST wording hides
    /// backend detail from clients, the job log wants it).
    fn from_rest(prefix: &str, err: RestError, detail: String) -> Self {
        let (status, code, message) = err.client_response();
        if status.is_client_error() {
            Self {
                message: format!("{prefix}: {message}"),
                status,
                code,
            }
        } else {
            Self::server(format!("{prefix}: {detail}"))
        }
    }

    fn from_sof(prefix: &str, err: SofError) -> Self {
        let detail = err.to_string();
        Self::from_rest(prefix, map_sof_error_to_rest(err), detail)
    }

    fn from_export(prefix: &str, err: ExportError) -> Self {
        match err {
            ExportError::Client {
                status,
                code,
                message,
            } => Self {
                message: format!("{prefix}: {message}"),
                status,
                code,
            },
            other => Self::server(format!("{prefix}: {other}")),
        }
    }
}

impl From<String> for JobFailure {
    fn from(message: String) -> Self {
        Self::server(message)
    }
}

/// What a server-fault job stores and the result endpoint returns. The
/// underlying (backend, driver, sink) text stays in the server log, where the
/// `export job failed` line records it under the same job id; this is the
/// split `RestError::InternalError` makes for synchronous requests (#1703).
fn server_fault_message(job_id: &str) -> String {
    format!("The export failed because of a server error; see the server log for job {job_id}.")
}

/// Default maximum number of concurrent export jobs.
pub const DEFAULT_MAX_CONCURRENCY: usize = 4;

/// Rows a running export job reads from one row stream between two checks that
/// it is still `Running` (#1704). `run_view` also checks once per call, so a
/// cancelled job stops before its next stream and within this many rows of the
/// current one.
const CANCEL_CHECK_ROWS: usize = 4096;

/// Configuration for the background task that reclaims finished export jobs.
///
/// Without this, terminal jobs (and their output shards) live for the lifetime
/// of the process: the `jobs` map grows unbounded and completed output never
/// frees, even though the completion manifest advertises a 24h `Expires`.
#[derive(Debug, Clone, Copy)]
pub struct CleanupConfig {
    /// How long after a job reaches a terminal state its output and bookkeeping
    /// are retained before the reaper removes them.
    pub output_ttl: Duration,
    /// How often the reaper scans for expired jobs.
    pub interval: Duration,
}

/// In-memory export job controller.
///
/// Jobs are tracked in a `DashMap` and execute in background `tokio` tasks,
/// bounded by a `Semaphore`.  Large result sets are split into multiple output
/// shards based on [`shard_rows`](InMemoryController::new).
pub struct InMemoryController<Sink: ExportSink> {
    jobs: Arc<DashMap<String, JobStatus>>,
    /// Tenant ID that submitted each job. Used to gate status / cancel /
    /// download so one tenant cannot access another tenant's exports.
    job_tenants: Arc<DashMap<String, String>>,
    runner: Arc<dyn SofRunner>,
    sink: Sink,
    semaphore: Arc<Semaphore>,
    shard_rows: usize,
    /// Retention of a finished job's output when a reaper runs (from
    /// [`CleanupConfig::output_ttl`]); caps the lifetime of the download URLs
    /// handed out. `None` (no reaper) leaves URLs uncapped because nothing
    /// deletes the output.
    output_ttl: Option<Duration>,
}

impl<Sink: ExportSink> InMemoryController<Sink> {
    /// Creates a new `InMemoryController`.
    ///
    /// - `runner` — the `SofRunner` used to evaluate ViewDefinitions
    /// - `sink` — where output files are written
    /// - `max_concurrency` — maximum concurrent jobs (defaults to [`DEFAULT_MAX_CONCURRENCY`])
    /// - `shard_rows` — target rows per output file (defaults to
    ///   [`planner::DEFAULT_SHARD_ROWS`])
    pub fn new(runner: Arc<dyn SofRunner>, sink: Sink, max_concurrency: Option<usize>) -> Self {
        Self::with_shard_rows(runner, sink, max_concurrency, None)
    }

    /// Like [`new`](Self::new) but with an explicit shard row limit. No cleanup
    /// reaper is started; finished jobs are never reaped (on a filesystem sink
    /// they also survive restarts via their persisted manifest).
    pub fn with_shard_rows(
        runner: Arc<dyn SofRunner>,
        sink: Sink,
        max_concurrency: Option<usize>,
        shard_rows: Option<usize>,
    ) -> Self {
        Self::with_options(runner, sink, max_concurrency, shard_rows, None)
    }

    /// Full constructor. When `cleanup` is `Some`, a background task is spawned
    /// that periodically reclaims terminal jobs older than the configured TTL —
    /// deleting their output via the sink and dropping their bookkeeping. Must
    /// be called from within a Tokio runtime when `cleanup` is `Some`.
    pub fn with_options(
        runner: Arc<dyn SofRunner>,
        sink: Sink,
        max_concurrency: Option<usize>,
        shard_rows: Option<usize>,
        cleanup: Option<CleanupConfig>,
    ) -> Self {
        let concurrency = max_concurrency.unwrap_or(DEFAULT_MAX_CONCURRENCY);
        let controller = Self {
            jobs: Arc::new(DashMap::new()),
            job_tenants: Arc::new(DashMap::new()),
            runner,
            sink,
            semaphore: Arc::new(Semaphore::new(concurrency)),
            shard_rows: shard_rows.unwrap_or(planner::DEFAULT_SHARD_ROWS),
            output_ttl: cleanup.map(|c| c.output_ttl),
        };

        // Rehydrate jobs a previous process completed and persisted (the
        // filesystem sink; other sinks' `load_completed` returns nothing, so
        // this is a no-op for them). Without this, a restart's fresh, empty
        // `jobs`/`job_tenants` maps make every already-completed job 404 on
        // status/result/download even though its output is still on disk
        // (#1474).
        rehydrate_completed_jobs(&controller.jobs, &controller.job_tenants, &controller.sink);

        if let Some(cfg) = cleanup {
            // A rehydrated job can already be older than `cfg.output_ttl` —
            // e.g. the process was down past it — so reap once now rather
            // than waiting for the reaper's own first tick (which is skipped;
            // see `spawn_cleanup`). Otherwise it would stay servable for up
            // to `cfg.interval` longer than an in-process-only job ever
            // could.
            reap_expired(
                &controller.jobs,
                &controller.job_tenants,
                &controller.sink,
                cfg.output_ttl,
            );
            controller.spawn_cleanup(cfg);
        }
        controller
    }

    /// Spawns the background reaper. Holds only `Arc`/`Clone` handles so it is
    /// independent of the controller's own lifetime.
    fn spawn_cleanup(&self, cfg: CleanupConfig) {
        let jobs = Arc::clone(&self.jobs);
        let job_tenants = Arc::clone(&self.job_tenants);
        let sink = self.sink.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(cfg.interval);
            // `with_options` already ran one reap pass covering whatever this
            // constructor call starts with — rehydrated jobs included — so
            // the immediate first tick would be a no-op; skip waiting on it
            // so the cadence starts at `interval`.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                reap_expired(&jobs, &job_tenants, &sink, cfg.output_ttl);
            }
        });
    }

    /// Returns `true` if `tenant_id` matches the tenant that submitted
    /// `job_id`. Returns `false` if the job is unknown or owned by a
    /// different tenant.
    fn tenant_matches(&self, tenant_id: &str, job_id: &str) -> bool {
        self.job_tenants
            .get(job_id)
            .map(|v| v.value() == tenant_id)
            .unwrap_or(false)
    }
}

impl<Sink: ExportSink + 'static> ExportJobController for InMemoryController<Sink> {
    fn submit(&self, task: ExportTask) -> JobId {
        let job_id = Uuid::new_v4().to_string();
        let submitted_at = Utc::now();

        self.job_tenants
            .insert(job_id.clone(), task.tenant.tenant_id().as_str().to_string());

        self.jobs.insert(
            job_id.clone(),
            JobStatus::Running {
                subjects_done: 0,
                subjects_total: task.work.subject_count() as u32,
                current_subject: None,
                submitted_at,
            },
        );

        // Clone everything needed by the spawned task
        let jobs = Arc::clone(&self.jobs);
        // Every row stream the task reads goes through this wrapper, so a job
        // that is no longer Running stops reading rows (#1704).
        let runner: Arc<dyn SofRunner> = Arc::new(StopWhenNotRunning {
            inner: Arc::clone(&self.runner),
            jobs: Arc::clone(&self.jobs),
            jid: job_id.clone(),
        });
        let sink = self.sink.clone();
        let semaphore = Arc::clone(&self.semaphore);
        let jid = job_id.clone();
        let shard_rows = self.shard_rows;

        tokio::spawn(async move {
            // Acquire concurrency permit (blocks if too many jobs running)
            let _permit = semaphore.acquire().await;

            // One job, one snapshot, any mixture of subjects. Views run first,
            // then queries, and their outputs are concatenated into a single
            // manifest. Progress counts every subject so the X-Progress
            // percentage tracks real work across both halves.
            let total_subjects = task.work.subject_count().max(1) as u32;
            let outcome = async {
                // A job cancelled (or already reaped) while it waited for its
                // permit never starts (#1704).
                ensure_running(&jobs, &jid)?;

                let (mut files, mut rows) = run_views_job(
                    &jobs,
                    &jid,
                    submitted_at,
                    &runner,
                    &sink,
                    shard_rows,
                    &task,
                    &task.work.views,
                    0,
                    total_subjects,
                )
                .await?;

                let (query_files, query_rows) = run_sqlquery_job(
                    &jobs,
                    &jid,
                    submitted_at,
                    &runner,
                    &sink,
                    shard_rows,
                    &task,
                    &task.work.queries,
                    task.work.limits,
                    task.work.views.len() as u32,
                    total_subjects,
                    files.len(),
                )
                .await?;

                files.extend(query_files);
                rows += query_rows;
                Ok::<_, JobFailure>((files, rows))
            }
            .await;

            match outcome {
                Ok((completed_files, total_rows)) => {
                    debug!(
                        job_id = %jid,
                        total_rows,
                        shards = completed_files.len(),
                        "export job completed"
                    );
                    let completed_at = Utc::now();

                    // Persist a durable record of this completion *before*
                    // flipping the in-memory status below (#1474): the status
                    // handler only reports `Completed` — the 303 a polling
                    // client sees — once this write has landed, so by the
                    // time any client has observed the job as `Completed`,
                    // its manifest is already on disk and a restart after
                    // that point still serves the job. Only do this while
                    // the job is still `Running`: if a cancel already won
                    // the race, its cleanup (`cancel`/`submit`'s post-task
                    // block) deletes the whole job directory — best-effort,
                    // so on a failed delete a manifest written here would
                    // outlive it and resurrect the cancelled job as
                    // `Completed` on the next rehydration.
                    if is_running(&jobs, &jid) {
                        let manifest = JobManifest {
                            version: MANIFEST_VERSION,
                            job_id: jid.clone(),
                            tenant_id: task.tenant.tenant_id().as_str().to_string(),
                            format: task.format.clone(),
                            files: completed_files
                                .iter()
                                .map(|f| ManifestFile {
                                    view_name: f.view_name.clone(),
                                    filename: f.filename.clone(),
                                    row_count: f.row_count,
                                })
                                .collect(),
                            submitted_at,
                            completed_at,
                            client_tracking_id: task.client_tracking_id.clone(),
                        };
                        if let Err(e) = sink.persist_completion(&jid, &manifest) {
                            // Degrade to pre-fix behaviour: the job still
                            // completes and stays servable for the rest of
                            // this process's life, it just won't survive a
                            // restart.
                            warn!(job_id = %jid, error = %e, "failed to persist export completion manifest; job will not survive a restart");
                        }
                    }

                    set_status_if_running(
                        &jobs,
                        &jid,
                        JobStatus::Completed {
                            files: completed_files,
                            submitted_at,
                            completed_at,
                            format: task.format.clone(),
                            client_tracking_id: task.client_tracking_id.clone(),
                        },
                    );
                }
                // Whatever stopped the task (a checkpoint, or an error raised
                // while it was being stopped) is not a failure to report: the
                // status is already Cancelled or gone.
                Err(failure) if !is_running(&jobs, &jid) => {
                    debug!(job_id = %jid, reason = %failure.message, "export job stopped: it is no longer running");
                }
                Err(failure) => {
                    warn!(
                        job_id = %jid,
                        error = %failure.message,
                        status = %failure.status,
                        "export job failed"
                    );
                    // The request's own failure keeps its wording; a server
                    // fault's text stays in the log line above (#1703).
                    let message = if failure.status.is_client_error() {
                        failure.message
                    } else {
                        server_fault_message(&jid)
                    };
                    set_status_if_running(
                        &jobs,
                        &jid,
                        JobStatus::Failed {
                            message,
                            status: failure.status,
                            code: failure.code,
                            submitted_at,
                            failed_at: Utc::now(),
                        },
                    );
                }
            }

            // Clean up output for any job that won't serve it:
            // - Cancelled: a concurrent DELETE set this state; shards this task
            //   wrote before observing it are orphaned (the cancel handler
            //   cleaned up whatever existed at DELETE time — this covers the race).
            // - Failed: the result URL returns the failure's status with no manifest, so the
            //   partial shards are unreachable and just waste storage.
            // - Gone: the reaper already removed the entry (the job was cancelled or
            //   failed and aged past `HFS_EXPORT_OUTPUT_TTL` while this task still
            //   waited or ran), so nothing else will ever delete what this task wrote.
            //   When the entry is already gone, a failed delete here is not
            //   retried: with no status entry left there is no retry handle for
            //   the reaper. This is an accepted limit (#1704).
            // A job can't be Running here: every outcome arm leaves it Completed,
            // Failed, Cancelled or absent.
            if !matches!(jobs.get(&jid).as_deref(), Some(JobStatus::Completed { .. })) {
                if let Err(e) = sink.delete_job(&jid) {
                    warn!(job_id = %jid, error = %e, "failed to delete partial export output of unfinished job");
                }
            }
        });

        job_id
    }

    fn get_status(&self, tenant_id: &str, job_id: &str) -> Option<JobStatus> {
        if !self.tenant_matches(tenant_id, job_id) {
            return None;
        }
        self.jobs.get(job_id).map(|v| v.clone())
    }

    fn cancel(&self, tenant_id: &str, job_id: &str) -> bool {
        if !self.tenant_matches(tenant_id, job_id) {
            return false;
        }
        // Only an in-progress job is cancellable. A DELETE on an already-finished
        // job is a no-op that still reports "found" (the handler 202s), but it
        // must NOT overwrite the terminal state: a completed job's status URL
        // keeps redirecting to its result manifest. Completed output is reclaimed
        // later by the cleanup reaper, not here.
        //
        // The lock is held only for the state change; deletion happens after.
        let now_cancelled = if let Some(mut entry) = self.jobs.get_mut(job_id) {
            match &*entry {
                JobStatus::Running { .. } => {
                    *entry = JobStatus::Cancelled {
                        cancelled_at: Utc::now(),
                    };
                    true
                }
                // Already done/failed/cancelled — found, but left untouched.
                _ => false,
            }
        } else {
            return false;
        };

        // Spec (operations-common, HL7/sql-on-fhir#365): SHOULD clean up partial
        // results on cancel. Drop any shards written so far. The job's task
        // notices the Cancelled state at its next checkpoint (before it starts,
        // before each subject and shard, and every `CANCEL_CHECK_ROWS` rows),
        // stops and frees its concurrency slot, then deletes whatever it wrote
        // after this point (see `submit`).
        if now_cancelled {
            if let Err(e) = self.sink.delete_job(job_id) {
                warn!(%job_id, error = %e, "failed to delete partial export output on cancel");
            }
        }
        true
    }

    fn read_shard(&self, tenant_id: &str, job_id: &str, filename: &str) -> Option<Vec<u8>> {
        if !self.tenant_matches(tenant_id, job_id) {
            return None;
        }
        // Serve only a filename the job's own completion record actually
        // lists — this is what keeps a rehydrated job's directory name and
        // manifest as the sole source of truth for what's servable, rather
        // than whatever happens to exist on the sink under that job id
        // (#1474). A job that hasn't completed yet (`Running`, or the map
        // lookup racing a remove and finding nothing) has no completion
        // record to check a filename against, so it has nothing to serve
        // either — same as `Cancelled`/`Failed`.
        match self.jobs.get(job_id).as_deref() {
            Some(JobStatus::Completed { files, .. }) => {
                if !files.iter().any(|f| f.filename == filename) {
                    return None;
                }
            }
            None
            | Some(JobStatus::Running { .. })
            | Some(JobStatus::Cancelled { .. })
            | Some(JobStatus::Failed { .. }) => return None,
        }
        self.sink.read_shard(job_id, filename)
    }

    fn download_url(
        &self,
        tenant_id: &str,
        public_base_url: &str,
        job_id: &str,
        filename: &str,
    ) -> Option<String> {
        if !self.tenant_matches(tenant_id, job_id) {
            return None;
        }
        // Cap the URL at what is left of the job's retention so it does not
        // outlive the object the reaper deletes (#1706).
        let cap = self
            .output_ttl
            .zip(self.jobs.get(job_id).and_then(|s| s.terminal_at()))
            .map(|(ttl, at)| download_url_lifetime_cap(ttl, at, Utc::now()));
        let url = match cap {
            Some(max_lifetime) => {
                self.sink
                    .download_url_capped(public_base_url, job_id, filename, max_lifetime)
            }
            None => self.sink.download_url(public_base_url, job_id, filename),
        };
        match url {
            Ok(url) => Some(url),
            Err(e) => {
                warn!(%job_id, %filename, error = %e, "failed to resolve export download URL");
                None
            }
        }
    }
}

// ============================================================================
// Download URL lifetime
// ============================================================================

/// Floor for a capped download URL, so a URL handed out moments before the
/// reaper's sweep is still usable.
const MIN_DOWNLOAD_URL_LIFETIME: Duration = Duration::from_secs(60);

/// The longest a download URL may stay valid: what is left of the job's
/// retention (`output_ttl` counted from `terminal_at`, the clock the reaper
/// uses), never below [`MIN_DOWNLOAD_URL_LIFETIME`]. A `terminal_at` in the
/// future (clock skew) counts as age zero.
fn download_url_lifetime_cap(
    output_ttl: Duration,
    terminal_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Duration {
    let age = (now - terminal_at).to_std().unwrap_or(Duration::ZERO);
    output_ttl
        .saturating_sub(age)
        .max(MIN_DOWNLOAD_URL_LIFETIME)
}

// ============================================================================
// Boot-time rehydration
// ============================================================================

/// Rehydrates `jobs`/`job_tenants` from every manifest the sink reports via
/// [`ExportSink::load_completed`], so a controller built in a fresh process —
/// e.g. after a restart — can serve status/result/download for jobs an
/// earlier process already completed (#1474). A sink that doesn't persist
/// completions (in-memory, S3) reports nothing, making this a no-op.
///
/// A manifest whose own `job_id` doesn't match the storage key it was loaded
/// under (or whose key isn't a UUID at all) is skipped: serving a job's files
/// by a path that disagrees with the job's own record would let that path
/// alone — rather than the manifest — decide which job's files get served.
/// Skipped and unparsable manifests are logged and otherwise ignored, never
/// fatal: a stray or corrupt directory under the export dir must never stop
/// the server from starting.
fn rehydrate_completed_jobs<Sink: ExportSink>(
    jobs: &DashMap<String, JobStatus>,
    job_tenants: &DashMap<String, String>,
    sink: &Sink,
) {
    for (key, manifest) in sink.load_completed() {
        if !Uuid::parse_str(&key).is_ok_and(|u| u.hyphenated().to_string() == key)
            || key != manifest.job_id
        {
            warn!(
                key = %key,
                manifest_job_id = %manifest.job_id,
                "skipping export manifest whose job id doesn't match its own storage key"
            );
            continue;
        }

        let files = manifest
            .files
            .iter()
            .map(|f| CompletedFile {
                view_name: f.view_name.clone(),
                filename: f.filename.clone(),
                row_count: f.row_count,
            })
            .collect();

        job_tenants.insert(manifest.job_id.clone(), manifest.tenant_id.clone());
        jobs.insert(
            manifest.job_id.clone(),
            JobStatus::Completed {
                files,
                submitted_at: manifest.submitted_at,
                completed_at: manifest.completed_at,
                format: manifest.format.clone(),
                client_tracking_id: manifest.client_tracking_id.clone(),
            },
        );
        debug!(job_id = %manifest.job_id, "rehydrated completed export job from its manifest");
    }
}

// ============================================================================
// Cleanup reaper
// ============================================================================

/// Removes terminal jobs whose age exceeds `output_ttl`: deletes their output
/// via the sink and drops their status / tenant bookkeeping. A job is dropped
/// once its delete succeeds; if the delete fails its status entry is kept so the
/// next sweep retries it. A sink which keeps failing keeps the entry and is
/// retried, with a warn, on every sweep until the delete succeeds; the job stays
/// unreachable to clients throughout, because its tenant entry is dropped on the
/// first sweep. `Running` jobs are never touched
/// ([`JobStatus::terminal_at`] returns `None` for them).
fn reap_expired<Sink: ExportSink>(
    jobs: &DashMap<String, JobStatus>,
    job_tenants: &DashMap<String, String>,
    sink: &Sink,
    output_ttl: Duration,
) {
    let ttl = match chrono::Duration::from_std(output_ttl) {
        Ok(d) => d,
        // A TTL too large to represent as a chrono::Duration means "effectively
        // never expire" — nothing to reap this pass.
        Err(_) => return,
    };
    let now = Utc::now();

    // Collect keys first: holding DashMap iterator guards while calling
    // `remove` on the same map would deadlock.
    let expired: Vec<String> = jobs
        .iter()
        .filter(|e| e.value().terminal_at().is_some_and(|t| now - t > ttl))
        .map(|e| e.key().clone())
        .collect();

    for jid in expired {
        // The tenant entry is dropped either way, so an expired job stops being
        // served exactly as before (every client route is tenant-gated and 404s).
        job_tenants.remove(&jid);
        match sink.delete_job(&jid) {
            Ok(()) => {
                jobs.remove(&jid);
                debug!(job_id = %jid, "cleanup: reclaimed expired export job");
            }
            // The status entry is kept on a failed delete because it is what the
            // next sweep finds the job by (its `terminal_at` is unchanged), so
            // the delete is retried instead of the output being orphaned.
            Err(e) => warn!(
                job_id = %jid,
                error = %e,
                "cleanup: failed to delete expired export output; retrying on the next sweep"
            ),
        }
    }
}

// ============================================================================
// Job execution
// ============================================================================

/// File extension (without leading dot) for an output format.
fn ext_for(format: &str) -> &'static str {
    match format {
        "csv" => "csv",
        "parquet" => "parquet",
        "json" => "json",
        _ => "ndjson",
    }
}

/// Transitions `jid` to `status` only if the job is still `Running`.
///
/// A job cancelled mid-run keeps its `Cancelled` state: the spec requires
/// status polls after a DELETE to return 404, so a background task that
/// finishes anyway must not resurrect the job to Completed/Failed.
fn set_status_if_running(jobs: &DashMap<String, JobStatus>, jid: &str, status: JobStatus) {
    if let Some(mut entry) = jobs.get_mut(jid) {
        if matches!(&*entry, JobStatus::Running { .. }) {
            *entry = status;
        }
    }
}

/// Whether `jid` is still `Running`. A cancelled, finished, or
/// reaper-removed job is not.
fn is_running(jobs: &DashMap<String, JobStatus>, jid: &str) -> bool {
    matches!(jobs.get(jid).as_deref(), Some(JobStatus::Running { .. }))
}

/// The worker's checkpoint: fails once `jid` is no longer `Running` (it was
/// cancelled, finished, or removed by the reaper), so the task stops at its
/// next opportunity instead of doing work nobody can reach.
fn ensure_running(jobs: &DashMap<String, JobStatus>, jid: &str) -> Result<(), JobFailure> {
    if is_running(jobs, jid) {
        Ok(())
    } else {
        Err(JobFailure::server(
            "export job is no longer running".to_string(),
        ))
    }
}

/// A [`SofRunner`] that stops feeding a job once it is no longer `Running`.
///
/// It wraps every row stream the job reads: the view subjects' streams and
/// the leaf ViewDefinitions `execute_plan` materializes for a SQL subject.
/// `run_view` fails with [`SofError::Cancelled`] for a job that is not
/// `Running`, and the returned stream re-checks every [`CANCEL_CHECK_ROWS`]
/// rows and yields `Err(SofError::Cancelled)` instead of the next row. Both
/// consumers stop at the first `Err`, and dropping the stream drops the
/// inner runner's channel receiver, which stops its producer.
///
/// A SQLite statement that is already executing is not interrupted; it stays
/// bounded by its own timeout.
struct StopWhenNotRunning {
    inner: Arc<dyn SofRunner>,
    jobs: Arc<DashMap<String, JobStatus>>,
    jid: String,
}

/// How often a running export re-checks its job while the runner is still
/// starting and while its row stream is quiet (#1823). The per-row check only
/// runs when rows arrive; a runner that lists a whole resource type before
/// its first row, or a view that filters out almost everything, can go
/// minutes without one.
const CANCEL_POLL: Duration = Duration::from_millis(500);

/// Resolves once `jid` is no longer `Running`, checking every
/// [`CANCEL_POLL`].
async fn until_not_running(jobs: Arc<DashMap<String, JobStatus>>, jid: String) {
    let mut tick = tokio::time::interval(CANCEL_POLL);
    loop {
        tick.tick().await;
        if !is_running(&jobs, &jid) {
            return;
        }
    }
}

#[async_trait]
impl SofRunner for StopWhenNotRunning {
    async fn run_view(
        &self,
        tenant: &TenantContext,
        view_definition: serde_json::Value,
        filters: ViewFilters,
    ) -> Result<RowStream, SofError> {
        if !is_running(&self.jobs, &self.jid) {
            return Err(SofError::Cancelled);
        }
        // A cancel while the runner is still starting drops its future, which
        // stops whatever it was doing, e.g. listing every key of a type (#1823).
        let stream = tokio::select! {
            started = self.inner.run_view(tenant, view_definition, filters) => started?,
            () = until_not_running(Arc::clone(&self.jobs), self.jid.clone()) => {
                return Err(SofError::Cancelled);
            }
        };
        let jobs = Arc::clone(&self.jobs);
        let jid = self.jid.clone();
        let mut rows: usize = 0;
        let checked = stream.map(move |row| {
            rows += 1;
            if rows.is_multiple_of(CANCEL_CHECK_ROWS) && !is_running(&jobs, &jid) {
                Err(SofError::Cancelled)
            } else {
                row
            }
        });
        // A quiet stream is cut by the poll instead, and then ends with the
        // same `Cancelled` a per-row check would have yielded.
        let jobs_end = Arc::clone(&self.jobs);
        let jid_end = self.jid.clone();
        let cancelled_tail = futures::stream::once(async move {
            (!is_running(&jobs_end, &jid_end)).then_some(Err(SofError::Cancelled))
        })
        .filter_map(futures::future::ready);
        Ok(Box::pin(
            checked
                .take_until(until_not_running(Arc::clone(&self.jobs), self.jid.clone()))
                .chain(cancelled_tail),
        ))
    }

    fn runner_name(&self) -> &'static str {
        self.inner.runner_name()
    }
}

/// Records that a subject has started: `subjects_done` is left unchanged and
/// `current_subject` is set to its output name. A job that is no longer
/// `Running` (e.g. cancelled mid-run) is left untouched — see
/// [`set_status_if_running`].
fn record_subject_started(
    jobs: &DashMap<String, JobStatus>,
    jid: &str,
    submitted_at: DateTime<Utc>,
    subjects_done: u32,
    subjects_total: u32,
    name: &str,
) {
    set_status_if_running(
        jobs,
        jid,
        JobStatus::Running {
            subjects_done,
            subjects_total,
            current_subject: Some(name.to_string()),
            submitted_at,
        },
    );
}

/// Records that a subject has finished: `subjects_done` advances to the given
/// count and `current_subject` is cleared, since nothing is in flight between
/// one subject finishing and the next one starting. A job that is no longer
/// `Running` is left untouched — see [`set_status_if_running`].
fn record_subject_finished(
    jobs: &DashMap<String, JobStatus>,
    jid: &str,
    submitted_at: DateTime<Utc>,
    subjects_done: u32,
    subjects_total: u32,
) {
    set_status_if_running(
        jobs,
        jid,
        JobStatus::Running {
            subjects_done,
            subjects_total,
            current_subject: None,
            submitted_at,
        },
    );
}

/// The ViewDefinition half of an export job: run each named view through the
/// `SofRunner` and shard its rows into output files. An error from a view's
/// row stream fails the whole job instead of being skipped, so a shard is
/// never published as a complete file when rows are actually missing.
#[allow(clippy::too_many_arguments)]
async fn run_views_job<Sink: ExportSink>(
    jobs: &DashMap<String, JobStatus>,
    jid: &str,
    submitted_at: DateTime<Utc>,
    runner: &Arc<dyn SofRunner>,
    sink: &Sink,
    shard_rows: usize,
    task: &ExportTask,
    views: &[NamedView],
    // `progress_offset`: subjects already finished before this half started.
    // `total_subjects`: subjects in the whole job, views and queries together.
    progress_offset: u32,
    total_subjects: u32,
) -> Result<(Vec<CompletedFile>, usize), JobFailure> {
    let format = task.format.to_lowercase();
    let ext = ext_for(&format);

    let mut completed_files: Vec<CompletedFile> = Vec::new();
    let mut total_rows: usize = 0;

    // Each ViewDefinition subject produces its own set of output shards, and
    // `output.name` in the manifest carries its name. Progress advances by one
    // subject per view finished.
    for (view_idx, named) in views.iter().enumerate() {
        ensure_running(jobs, jid)?;
        record_subject_started(
            jobs,
            jid,
            submitted_at,
            progress_offset + view_idx as u32,
            total_subjects,
            &named.name,
        );

        let stream = runner
            .run_view(&task.tenant, named.view.clone(), task.filters.clone())
            .await
            .map_err(|e| JobFailure::from_sof(&format!("view '{}'", named.name), e))?;

        let mut rows: Vec<serde_json::Value> = Vec::new();
        let mut stream = stream;
        while let Some(item) = stream.next().await {
            match item {
                Ok(v) => rows.push(v),
                Err(e) => {
                    if matches!(e, SofError::Cancelled) {
                        debug!(view = %named.name, "export row stream stopped: job is no longer running");
                    } else {
                        warn!(view = %named.name, error = %e, "export row stream failed");
                    }
                    return Err(format!("view '{}': {e}", named.name).into());
                }
            }
        }

        total_rows += rows.len();

        // Spec: `output` is 0..*. Views with zero rows simply contribute no
        // `output` entries rather than emitting an empty shard with a
        // download URL pointing at zero bytes.
        for range in planner::plan(rows.len(), shard_rows) {
            ensure_running(jobs, jid)?;
            let shard_slice = &rows[range];
            let row_count = shard_slice.len();

            let data = format_rows(shard_slice, &format, task.header)
                .map_err(|e| format!("view '{}': {e}", named.name))?;

            // Shard files are numbered by a running index across the whole
            // job so every shard gets a unique filename; for the
            // single-view case this matches the historical `shard-{N}`
            // numbering exactly.
            let shard_key = completed_files.len();
            let filename = sink
                .write_shard(jid, shard_key, data, ext)
                .map_err(|e| format!("view '{}': {e}", named.name))?;

            debug!(job_id = %jid, view = %named.name, shard = shard_key, rows = row_count, file = %filename, "shard written");
            completed_files.push(CompletedFile {
                view_name: named.name.clone(),
                filename,
                row_count,
            });
        }

        record_subject_finished(
            jobs,
            jid,
            submitted_at,
            progress_offset + (view_idx as u32) + 1,
            total_subjects,
        );
    }

    Ok((completed_files, total_rows))
}

/// The SQLQuery / SQLView half of an export job: materialize each subject's
/// table sources via the `SofRunner`, execute the pre-validated SQL, and shard
/// the result rows into output files.
#[allow(clippy::too_many_arguments)]
async fn run_sqlquery_job<Sink: ExportSink>(
    jobs: &DashMap<String, JobStatus>,
    jid: &str,
    submitted_at: DateTime<Utc>,
    runner: &Arc<dyn SofRunner>,
    sink: &Sink,
    shard_rows: usize,
    task: &ExportTask,
    queries: &[NamedSqlQuery],
    limits: SqlExportLimits,
    // `progress_offset`: subjects already finished before this half started.
    // `total_subjects`: subjects in the whole job, views and queries together.
    // `shard_offset`: shards already written by the views half, so shard
    // filenames stay unique across the whole job.
    progress_offset: u32,
    total_subjects: u32,
    shard_offset: usize,
) -> Result<(Vec<CompletedFile>, usize), JobFailure> {
    let format = task.format.to_lowercase();
    let ext = ext_for(&format);

    let mut completed_files: Vec<CompletedFile> = Vec::new();
    let mut total_rows: usize = 0;

    for (query_idx, query) in queries.iter().enumerate() {
        ensure_running(jobs, jid)?;
        record_subject_started(
            jobs,
            jid,
            submitted_at,
            progress_offset + query_idx as u32,
            total_subjects,
            &query.name,
        );

        let result = execute_sql_query(runner, task, query, limits)
            .await
            .map_err(|e| JobFailure::from_export(&format!("query '{}'", query.name), e))?;

        total_rows += result.rows.len();

        for range in planner::plan(result.rows.len(), shard_rows) {
            ensure_running(jobs, jid)?;
            let row_count = range.len();
            let data = format_query_rows(&result, range, &format, task.header)
                .map_err(|e| format!("query '{}': {e}", query.name))?;

            let shard_key = shard_offset + completed_files.len();
            let filename = sink
                .write_shard(jid, shard_key, data, ext)
                .map_err(|e| format!("query '{}': {e}", query.name))?;

            debug!(job_id = %jid, query = %query.name, shard = shard_key, rows = row_count, file = %filename, "shard written");
            completed_files.push(CompletedFile {
                view_name: query.name.clone(),
                filename,
                row_count,
            });
        }

        record_subject_finished(
            jobs,
            jid,
            submitted_at,
            progress_offset + (query_idx as u32) + 1,
            total_subjects,
        );
    }

    Ok((completed_files, total_rows))
}

/// Materializes a query's fully-resolved dependency graph (Phase 2 of the
/// two-phase resolver — [`crate::handlers::sof::graph::execute_plan`]) and
/// executes its SQL, enforcing the same row caps and timeout as the
/// synchronous `$sql-run` operation. The export operations emit flat formats
/// only (csv/ndjson/parquet/json), so the result's JSON cell values feed
/// `format_output` directly.
async fn execute_sql_query(
    runner: &Arc<dyn SofRunner>,
    task: &ExportTask,
    query: &NamedSqlQuery,
    limits: SqlExportLimits,
) -> Result<QueryResult, ExportError> {
    let engine = InMemorySqlEngine::open().map_err(|e| ExportError::Runner(e.to_string()))?;
    let exec_limits = crate::handlers::sof::graph::ExecLimits {
        max_source_rows_per_vd: limits.max_source_rows_per_vd,
        max_rows: limits.max_rows,
        timeout_secs: limits.timeout_secs,
    };

    let (result, _leaf_schemas) = crate::handlers::sof::graph::execute_plan(
        engine,
        runner,
        &task.tenant,
        &task.filters,
        &query.plan,
        &query.sql,
        &query.bindings,
        exec_limits,
    )
    .await
    .map_err(|e| {
        // A limit the request ran into, or a subject the engine refuses, is
        // the client's answer; the REST wording of a server fault hides the
        // backend detail, so that path keeps the engine's own text.
        let detail = e.to_string();
        let (status, code, message) = sqlquery_err_to_rest(e).client_response();
        if status.is_client_error() {
            ExportError::Client {
                status,
                code,
                message,
            }
        } else {
            ExportError::Runner(detail)
        }
    })?;

    Ok(result)
}

// ============================================================================
// Row serialization helpers
// ============================================================================

/// Serializes a shard of view-output rows (column → value JSON objects).
fn format_rows(
    rows: &[serde_json::Value],
    format: &str,
    include_csv_header: bool,
) -> Result<Vec<u8>, ExportError> {
    match format {
        "csv" => format_csv(rows, include_csv_header),
        "parquet" => format_parquet(rows),
        "json" => format_json_array(rows),
        _ => format_ndjson(rows),
    }
}

/// Serializes a shard of SQL query result rows through
/// `helios_sof::format_output` (matching the `$sql-run` bytes). The
/// export operations support flat formats only; `fhir` is a run-operation
/// format and is rejected at kick-off.
fn format_query_rows(
    result: &QueryResult,
    range: std::ops::Range<usize>,
    format: &str,
    include_csv_header: bool,
) -> Result<Vec<u8>, ExportError> {
    let rows = &result.rows[range];
    let ct = match format {
        "csv" => {
            if include_csv_header {
                helios_sof::ContentType::CsvWithHeader
            } else {
                helios_sof::ContentType::Csv
            }
        }
        "json" => helios_sof::ContentType::Json,
        "parquet" => helios_sof::ContentType::Parquet,
        _ => helios_sof::ContentType::NdJson,
    };
    // Build a ProcessedResult directly so columns keep their SQL order
    // (mirrors the `$sql-run` handler).
    let processed = helios_sof::ProcessedResult {
        columns: result.columns.clone(),
        rows: rows
            .iter()
            .map(|r| helios_sof::ProcessedRow { values: r.clone() })
            .collect(),
    };
    helios_sof::format_output(processed, ct, None)
        .map_err(|e| ExportError::Serialization(e.to_string()))
}

/// Serialises rows as a single JSON array (`_format=json`).
fn format_json_array(rows: &[serde_json::Value]) -> Result<Vec<u8>, ExportError> {
    serde_json::to_vec(rows).map_err(|e| ExportError::Serialization(e.to_string()))
}

fn format_parquet(rows: &[serde_json::Value]) -> Result<Vec<u8>, ExportError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let columns: Vec<String> = rows[0]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();

    let processed_rows: Vec<helios_sof::ProcessedRow> = rows
        .iter()
        .map(|row| {
            let values = columns
                .iter()
                .map(|col| row.as_object().and_then(|o| o.get(col)).cloned())
                .collect();
            helios_sof::ProcessedRow { values }
        })
        .collect();

    let result = helios_sof::ProcessedResult {
        columns,
        rows: processed_rows,
    };

    helios_sof::format_parquet_multi_file(result, None, usize::MAX)
        .map_err(|e| ExportError::Serialization(e.to_string()))
        .map(|files| files.into_iter().next().unwrap_or_default())
}

fn format_ndjson(rows: &[serde_json::Value]) -> Result<Vec<u8>, ExportError> {
    let mut out = Vec::new();
    for row in rows {
        let line =
            serde_json::to_vec(row).map_err(|e| ExportError::Serialization(e.to_string()))?;
        out.extend_from_slice(&line);
        out.push(b'\n');
    }
    Ok(out)
}

fn format_csv(rows: &[serde_json::Value], include_header: bool) -> Result<Vec<u8>, ExportError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    // Collect column names from the first row
    let cols: Vec<String> = rows[0]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();

    let mut out = Vec::new();

    // Header (only when caller opts in, per the SoF `header` parameter).
    if include_header {
        out.extend_from_slice(cols.join(",").as_bytes());
        out.push(b'\n');
    }

    // Data rows
    for row in rows {
        let obj = match row.as_object() {
            Some(o) => o,
            None => continue,
        };
        let values: Vec<String> = cols
            .iter()
            .map(|c| {
                let v = obj.get(c).unwrap_or(&serde_json::Value::Null);
                csv_cell(v)
            })
            .collect();
        out.extend_from_slice(values.join(",").as_bytes());
        out.push(b'\n');
    }

    Ok(out)
}

/// Applies the same formula guard as `helios_sof`'s CSV writer to string cells.
fn csv_cell(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => {
            let s = helios_sof::neutralize_csv_formula(s);
            if s.contains(',') || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.into_owned()
            }
        }
        other => {
            let s = other.to_string();
            format!("\"{}\"", s.replace('"', "\"\""))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::controller::ExportWork;
    use crate::export::sink::InMemorySink;
    use async_trait::async_trait;
    use helios_persistence::core::sof_runner::{RowStream, SofError, ViewFilters};
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::sync::Notify;

    /// A `SofRunner` that blocks until `release` is notified, then yields an
    /// empty row stream. Lets a test hold a job in the Running state for as
    /// long as it needs.
    struct BlockingRunner {
        release: Arc<Notify>,
    }

    #[async_trait]
    impl SofRunner for BlockingRunner {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            _view_definition: serde_json::Value,
            _filters: ViewFilters,
        ) -> Result<RowStream, SofError> {
            self.release.notified().await;
            Ok(Box::pin(futures::stream::empty()))
        }

        fn runner_name(&self) -> &'static str {
            "blocking-test-runner"
        }
    }

    /// Spec (#363): status polls after a DELETE return 404, so a job
    /// cancelled while running must stay Cancelled — the background task
    /// finishing later must not overwrite the state with Completed.
    #[tokio::test]
    async fn cancelled_job_is_not_resurrected_by_late_completion() {
        let release = Arc::new(Notify::new());
        let runner = Arc::new(BlockingRunner {
            release: Arc::clone(&release),
        });
        let controller =
            InMemoryController::new(runner, InMemorySink::new("http://localhost"), None);

        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        let job_id = controller.submit(ExportTask {
            work: ExportWork {
                views: vec![NamedView {
                    name: "patients".to_string(),
                    view: serde_json::json!({
                        "resourceType": "ViewDefinition",
                        "resource": "Patient",
                        "status": "active",
                        "select": [{"column": [{"name": "id", "path": "id"}]}]
                    }),
                }],
                ..Default::default()
            },
            tenant,
            filters: ViewFilters::default(),
            format: "ndjson".to_string(),
            header: true,
            client_tracking_id: None,
        });

        // The runner is blocked, so the job is still Running — cancel it.
        assert!(controller.cancel("t1", &job_id));
        assert!(matches!(
            controller.get_status("t1", &job_id),
            Some(JobStatus::Cancelled { .. })
        ));

        // Unblock the background task and give it time to run to completion.
        // The Cancelled state must survive.
        release.notify_one();
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            match controller.get_status("t1", &job_id) {
                Some(JobStatus::Cancelled { .. }) => {}
                other => panic!("cancelled job must stay Cancelled, got {other:?}"),
            }
        }
    }

    /// Spec (operations-common, HL7/sql-on-fhir#365): cancelling a job SHOULD
    /// clean up partial results. The shards written before the DELETE must be
    /// removed from the sink, and the download route must 404 afterwards.
    #[tokio::test]
    async fn cancel_deletes_partial_output_and_download_404s() {
        let release = Arc::new(Notify::new());
        let runner = Arc::new(BlockingRunner {
            release: Arc::clone(&release),
        });
        // Keep a handle on the sink (shares the inner Arc<DashMap>) so the test
        // can both seed a partial shard and assert it was deleted.
        let sink = InMemorySink::new("http://localhost");
        let controller = InMemoryController::new(runner, sink.clone(), None);

        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        let job_id = controller.submit(ExportTask {
            work: ExportWork {
                views: vec![NamedView {
                    name: "patients".to_string(),
                    view: serde_json::json!({
                        "resourceType": "ViewDefinition",
                        "resource": "Patient",
                        "status": "active",
                        "select": [{"column": [{"name": "id", "path": "id"}]}]
                    }),
                }],
                ..Default::default()
            },
            tenant,
            filters: ViewFilters::default(),
            format: "ndjson".to_string(),
            header: true,
            client_tracking_id: None,
        });

        // Simulate a shard the running job had already streamed out. It's on
        // the sink, but the job has no completion record yet to check a
        // filename against, so the download route must not serve it while
        // still running (#1474: a job's manifest is the sole source of truth
        // for what's servable, not whatever the sink happens to hold).
        sink.write_shard(&job_id, 0, b"{\"id\":\"a\"}\n".to_vec(), "ndjson")
            .unwrap();
        assert!(
            sink.read_shard(&job_id, "shard-0.ndjson").is_some(),
            "sanity: the sink itself does hold the shard"
        );
        assert!(
            controller
                .read_shard("t1", &job_id, "shard-0.ndjson")
                .is_none(),
            "a running job serves no files until it completes"
        );

        // Cancel: partial output is dropped and the download route 404s.
        assert!(controller.cancel("t1", &job_id));
        assert!(
            sink.read_shard(&job_id, "shard-0.ndjson").is_none(),
            "cancel must delete partial shards from the sink"
        );
        assert!(
            controller
                .read_shard("t1", &job_id, "shard-0.ndjson")
                .is_none(),
            "download route must 404 for a cancelled job"
        );

        release.notify_one();
    }

    /// A `SofRunner` that reports each `run_view` call, in order, over an
    /// unbounded channel and then blocks on a shared `Notify` until the test
    /// releases it. Lets a test observe the export worker's `Running` state
    /// exactly at the boundary between two subjects, deterministically.
    struct SteppingRunner {
        called: tokio::sync::mpsc::UnboundedSender<()>,
        release: Arc<Notify>,
    }

    #[async_trait]
    impl SofRunner for SteppingRunner {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            _view_definition: serde_json::Value,
            _filters: ViewFilters,
        ) -> Result<RowStream, SofError> {
            let _ = self.called.send(());
            self.release.notified().await;
            Ok(Box::pin(futures::stream::empty()))
        }

        fn runner_name(&self) -> &'static str {
            "stepping-test-runner"
        }
    }

    /// Spec (#853): the worker records a subject's *start* (`current_subject`
    /// set, `subjects_done` unchanged) as well as its *finish*
    /// (`subjects_done + 1`, `current_subject` cleared), in kick-off order —
    /// views first, then queries. The query subject here depends on a `Leaf`
    /// ViewDefinition node, so its execution also calls `run_view`, letting
    /// the same `SteppingRunner` observe both subjects.
    #[tokio::test]
    async fn worker_records_subject_start_and_finish_in_kickoff_order() {
        let (called_tx, mut called_rx) = tokio::sync::mpsc::unbounded_channel();
        let release = Arc::new(Notify::new());
        let runner = Arc::new(SteppingRunner {
            called: called_tx,
            release: Arc::clone(&release),
        });
        let controller =
            InMemoryController::new(runner, InMemorySink::new("http://localhost"), None);

        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        let leaf_view = serde_json::json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"name": "id", "path": "id"}]}]
        });
        let query_plan = crate::handlers::sof::graph::GraphPlan {
            nodes: vec![crate::handlers::sof::graph::PlanNode::Leaf {
                internal_name: "vd_0".to_string(),
                view: leaf_view.clone(),
            }],
            subject_edges: Vec::new(),
        };

        let job_id = controller.submit(ExportTask {
            work: ExportWork {
                views: vec![NamedView {
                    name: "demographics".to_string(),
                    view: leaf_view,
                }],
                queries: vec![NamedSqlQuery {
                    name: "families".to_string(),
                    sql: "SELECT * FROM vd_0".to_string(),
                    plan: query_plan,
                    bindings: Vec::new(),
                }],
                limits: SqlExportLimits {
                    max_source_rows_per_vd: 1000,
                    max_rows: 1000,
                    timeout_secs: 5,
                },
            },
            tenant,
            filters: ViewFilters::default(),
            format: "ndjson".to_string(),
            header: true,
            client_tracking_id: None,
        });

        // The view subject ("demographics") starts first, per kick-off order.
        called_rx
            .recv()
            .await
            .expect("view subject should call run_view");
        match controller.get_status("t1", &job_id) {
            Some(JobStatus::Running {
                subjects_done,
                subjects_total,
                current_subject,
                ..
            }) => {
                assert_eq!(subjects_done, 0);
                assert_eq!(subjects_total, 2);
                assert_eq!(current_subject.as_deref(), Some("demographics"));
            }
            other => panic!("expected Running with demographics in progress, got {other:?}"),
        }
        release.notify_one();

        // The query subject ("families") starts only once the view subject
        // has finished — subjects_done must already read 1.
        called_rx
            .recv()
            .await
            .expect("query subject should call run_view");
        match controller.get_status("t1", &job_id) {
            Some(JobStatus::Running {
                subjects_done,
                subjects_total,
                current_subject,
                ..
            }) => {
                assert_eq!(
                    subjects_done, 1,
                    "the view subject must be marked done before the query starts"
                );
                assert_eq!(subjects_total, 2);
                assert_eq!(current_subject.as_deref(), Some("families"));
            }
            other => panic!("expected Running with families in progress, got {other:?}"),
        }
        release.notify_one();

        // Both subjects done: the job completes with no subject in flight.
        for _ in 0..40 {
            if matches!(
                controller.get_status("t1", &job_id),
                Some(JobStatus::Completed { .. })
            ) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("job did not reach Completed after both subjects finished");
    }

    /// A `SofRunner` whose `run_view` streams 1,000 rows through a bounded
    /// `tokio::sync::mpsc` channel fed from `spawn_blocking`, mirroring how
    /// the real per-backend runners (e.g.
    /// `crates/persistence/src/sof/sqlite.rs`) produce their row streams.
    /// `helios-rest` does not depend on `tokio-stream`, so the receiver is
    /// adapted into a [`RowStream`] with `futures::stream::poll_fn` instead
    /// of `ReceiverStream`.
    struct ChannelRunner;

    #[async_trait]
    impl SofRunner for ChannelRunner {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            _view_definition: serde_json::Value,
            _filters: ViewFilters,
        ) -> Result<RowStream, SofError> {
            let (tx, mut rx) =
                tokio::sync::mpsc::channel::<Result<serde_json::Value, SofError>>(256);
            tokio::task::spawn_blocking(move || {
                for i in 0..1000i64 {
                    let row = serde_json::json!({"id": format!("p{i}")});
                    if tx.blocking_send(Ok(row)).is_err() {
                        break;
                    }
                }
            });
            Ok(Box::pin(futures::stream::poll_fn(move |cx| {
                rx.poll_recv(cx)
            })))
        }

        fn runner_name(&self) -> &'static str {
            "channel-test-runner"
        }
    }

    /// Regression test for the export-side hang this ticket fixes: the
    /// SQLQuery subject's only dependency streams past tokio's 128-item
    /// cooperative-poll budget via an `mpsc` channel fed from
    /// `spawn_blocking`, exactly like a real backend runner. Before the fix,
    /// `execute_plan`'s call into `insert_rows` never returned once the
    /// stream crossed that budget, so the export job stayed `Running`
    /// forever; with the fix it reaches `Completed` with every row written.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sqlquery_export_completes_when_dependency_streams_past_coop_budget() {
        let runner = Arc::new(ChannelRunner);
        let controller =
            InMemoryController::new(runner, InMemorySink::new("http://localhost"), None);

        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        let leaf_view = serde_json::json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"name": "id", "path": "id"}]}]
        });
        let query_plan = crate::handlers::sof::graph::GraphPlan {
            nodes: vec![crate::handlers::sof::graph::PlanNode::Leaf {
                internal_name: "vd_0".to_string(),
                view: leaf_view,
            }],
            subject_edges: Vec::new(),
        };

        let job_id = controller.submit(ExportTask {
            work: ExportWork {
                views: vec![],
                queries: vec![NamedSqlQuery {
                    name: "families".to_string(),
                    sql: "SELECT * FROM vd_0".to_string(),
                    plan: query_plan,
                    bindings: Vec::new(),
                }],
                limits: SqlExportLimits {
                    max_source_rows_per_vd: 10_000,
                    max_rows: 10_000,
                    timeout_secs: 5,
                },
            },
            tenant,
            filters: ViewFilters::default(),
            format: "ndjson".to_string(),
            header: true,
            client_tracking_id: None,
        });

        let status = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                match controller.get_status("t1", &job_id) {
                    Some(status @ JobStatus::Completed { .. })
                    | Some(status @ JobStatus::Failed { .. }) => return status,
                    _ => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
        })
        .await
        .expect("export job must reach a terminal state before the timeout");

        match status {
            JobStatus::Completed { files, .. } => {
                assert_eq!(
                    files.len(),
                    1,
                    "expected exactly one output file, got {files:?}"
                );
                assert_eq!(files[0].row_count, 1000);
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    /// A `SofRunner` whose `run_view` streams 200 rows and then fails with a
    /// backend error, simulating a Postgres `statement_timeout` or a lost
    /// connection partway through materializing a dependency.
    struct FailingRunner;

    #[async_trait]
    impl SofRunner for FailingRunner {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            _view_definition: serde_json::Value,
            _filters: ViewFilters,
        ) -> Result<RowStream, SofError> {
            let ok_rows = (0..200).map(|i| Ok(serde_json::json!({"id": format!("p{i}")})));
            let failure = std::iter::once(Err(SofError::Backend(
                "canceling statement due to statement timeout".to_string(),
            )));
            Ok(Box::pin(futures::stream::iter(ok_rows.chain(failure))))
        }

        fn runner_name(&self) -> &'static str {
            "failing-test-runner"
        }
    }

    /// A runner that refuses every ViewDefinition it is handed, the way the
    /// compilers refuse a malformed one.
    struct RefusingRunner;

    #[async_trait]
    impl SofRunner for RefusingRunner {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            _view_definition: serde_json::Value,
            _filters: ViewFilters,
        ) -> Result<RowStream, SofError> {
            Err(SofError::InvalidViewDefinition(
                "column 'city' declares `collection: false` but path 'address.city' may yield multiple values".to_string(),
            ))
        }

        fn runner_name(&self) -> &'static str {
            "refusing-test-runner"
        }
    }

    async fn terminal_status(
        controller: &InMemoryController<InMemorySink>,
        job_id: &str,
    ) -> JobStatus {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                match controller.get_status("t1", job_id) {
                    Some(status @ JobStatus::Completed { .. })
                    | Some(status @ JobStatus::Failed { .. }) => return status,
                    _ => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
        })
        .await
        .expect("export job must reach a terminal state before the timeout")
    }

    /// #1570: a ViewDefinition the runner refuses is the request's fault —
    /// the job fails with the 422 `$sql-run` answers, not a 500.
    #[tokio::test]
    async fn a_refused_view_fails_the_job_as_the_clients_fault() {
        let controller = InMemoryController::new(
            Arc::new(RefusingRunner),
            InMemorySink::new("http://localhost"),
            None,
        );
        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        let job_id = controller.submit(ExportTask {
            work: ExportWork {
                views: vec![NamedView {
                    name: "demo".to_string(),
                    view: serde_json::json!({"resourceType": "ViewDefinition", "resource": "Patient"}),
                }],
                queries: vec![],
                limits: SqlExportLimits::default(),
            },
            tenant,
            filters: ViewFilters::default(),
            format: "ndjson".to_string(),
            header: true,
            client_tracking_id: None,
        });
        match terminal_status(&controller, &job_id).await {
            JobStatus::Failed {
                message,
                status,
                code,
                ..
            } => {
                assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
                assert_eq!(code, "processing");
                assert!(
                    message.starts_with("view 'demo': column 'city'"),
                    "{message}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// A runner whose backend is gone before the first row.
    struct BackendDownRunner;

    #[async_trait]
    impl SofRunner for BackendDownRunner {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            _view_definition: serde_json::Value,
            _filters: ViewFilters,
        ) -> Result<RowStream, SofError> {
            Err(SofError::Backend("connection reset by peer".to_string()))
        }

        fn runner_name(&self) -> &'static str {
            "backend-down-test-runner"
        }
    }

    /// #1570/#1703: a backend failure at kick-off stays a server fault (500),
    /// and its text stays in the job log; the stored message is generic and
    /// names the job.
    #[tokio::test]
    async fn a_backend_failure_at_kickoff_stays_a_server_fault() {
        let controller = InMemoryController::new(
            Arc::new(BackendDownRunner),
            InMemorySink::new("http://localhost"),
            None,
        );
        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        let job_id = controller.submit(ExportTask {
            work: ExportWork {
                views: vec![NamedView {
                    name: "demo".to_string(),
                    view: serde_json::json!({"resourceType": "ViewDefinition", "resource": "Patient"}),
                }],
                queries: vec![],
                limits: SqlExportLimits::default(),
            },
            tenant,
            filters: ViewFilters::default(),
            format: "ndjson".to_string(),
            header: true,
            client_tracking_id: None,
        });
        match terminal_status(&controller, &job_id).await {
            JobStatus::Failed {
                message,
                status,
                code,
                ..
            } => {
                assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
                assert_eq!(code, "processing");
                assert_eq!(message, server_fault_message(&job_id));
                assert!(!message.contains("connection reset by peer"), "{message}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// #1570: a SQL Query subject that runs into the source-row limit fails
    /// the job with the 422 and wording `$sql-run` gives the same limit.
    #[tokio::test]
    async fn a_row_limit_fails_the_job_as_the_clients_fault() {
        let controller = InMemoryController::new(
            Arc::new(FailingRunner),
            InMemorySink::new("http://localhost"),
            None,
        );
        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        let query_plan = crate::handlers::sof::graph::GraphPlan {
            nodes: vec![crate::handlers::sof::graph::PlanNode::Leaf {
                internal_name: "vd_0".to_string(),
                view: serde_json::json!({
                    "resourceType": "ViewDefinition",
                    "resource": "Patient",
                    "status": "active",
                    "select": [{"column": [{"name": "id", "path": "id"}]}]
                }),
            }],
            subject_edges: Vec::new(),
        };
        let job_id = controller.submit(ExportTask {
            work: ExportWork {
                views: vec![],
                queries: vec![NamedSqlQuery {
                    name: "tall_female_patients".to_string(),
                    sql: "SELECT * FROM vd_0".to_string(),
                    plan: query_plan,
                    bindings: Vec::new(),
                }],
                // The runner yields 200 rows before its own failure: the cap
                // is what the job runs into.
                limits: SqlExportLimits {
                    max_source_rows_per_vd: 10,
                    max_rows: 10_000,
                    timeout_secs: 5,
                },
            },
            tenant,
            filters: ViewFilters::default(),
            format: "csv".to_string(),
            header: true,
            client_tracking_id: None,
        });
        match terminal_status(&controller, &job_id).await {
            JobStatus::Failed {
                message,
                status,
                code,
                ..
            } => {
                assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{message}");
                assert_eq!(code, "processing");
                assert!(message.contains("exceeds 10-row limit"), "{message}");
                assert!(
                    message.starts_with("query 'tall_female_patients'"),
                    "{message}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// #1473: a SQL Query dependency over the per-dependency cap fails the
    /// export naming the dependency, the cap and the setting; a LIMIT in the
    /// query cannot bound it, so the WHERE/LIMIT advice must not appear.
    #[tokio::test]
    async fn a_dependency_over_the_row_cap_fails_the_job_naming_it_and_the_setting() {
        let controller = InMemoryController::new(
            Arc::new(FailingRunner),
            InMemorySink::new("http://localhost"),
            None,
        );
        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        let query_plan = crate::handlers::sof::graph::GraphPlan {
            nodes: vec![crate::handlers::sof::graph::PlanNode::Leaf {
                internal_name: "__sof_node_0".to_string(),
                view: serde_json::json!({
                    "resourceType": "ViewDefinition",
                    "name": "observation_flat",
                    "resource": "Observation",
                    "status": "active",
                    "select": [{"column": [{"name": "id", "path": "id"}]}]
                }),
            }],
            subject_edges: vec![crate::handlers::sof::graph::Edge {
                label: "obs".to_string(),
                target_internal_name: "__sof_node_0".to_string(),
            }],
        };
        let job_id = controller.submit(ExportTask {
            work: ExportWork {
                views: vec![],
                queries: vec![NamedSqlQuery {
                    name: "tall_female_patients".to_string(),
                    sql: "SELECT * FROM obs LIMIT 5".to_string(),
                    plan: query_plan,
                    bindings: Vec::new(),
                }],
                // The runner yields 200 rows before its own failure: the cap
                // is what the job runs into.
                limits: SqlExportLimits {
                    max_source_rows_per_vd: 10,
                    max_rows: 10_000,
                    timeout_secs: 5,
                },
            },
            tenant,
            filters: ViewFilters::default(),
            format: "csv".to_string(),
            header: true,
            client_tracking_id: None,
        });
        match terminal_status(&controller, &job_id).await {
            JobStatus::Failed {
                message,
                status,
                code,
                ..
            } => {
                assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{message}");
                assert_eq!(code, "processing");
                assert_eq!(
                    message,
                    "query 'tall_female_patients': dependency 'obs' (ViewDefinition \
                     observation_flat) exceeds 10-row limit: SQL queries materialize each \
                     dependency in full before the query's WHERE runs. Narrow the dependency \
                     with a ViewDefinition 'where', or raise \
                     HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD."
                );
                assert!(!message.contains("WHERE/LIMIT"), "{message}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// A storage failure mid-materialization of a SQLQuery dependency must
    /// fail the export job as a server fault (500), not blame the client's
    /// Library as malformed. The real cause (the backend statement timeout)
    /// is kept in the server log, not returned (#1703).
    #[tokio::test]
    async fn sqlquery_export_fails_with_diagnostic_when_dependency_stream_errors() {
        let runner = Arc::new(FailingRunner);
        let controller =
            InMemoryController::new(runner, InMemorySink::new("http://localhost"), None);

        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        let leaf_view = serde_json::json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"name": "id", "path": "id"}]}]
        });
        let query_plan = crate::handlers::sof::graph::GraphPlan {
            nodes: vec![crate::handlers::sof::graph::PlanNode::Leaf {
                internal_name: "vd_0".to_string(),
                view: leaf_view,
            }],
            subject_edges: Vec::new(),
        };

        let job_id = controller.submit(ExportTask {
            work: ExportWork {
                views: vec![],
                queries: vec![NamedSqlQuery {
                    name: "families".to_string(),
                    sql: "SELECT * FROM vd_0".to_string(),
                    plan: query_plan,
                    bindings: Vec::new(),
                }],
                limits: SqlExportLimits {
                    max_source_rows_per_vd: 10_000,
                    max_rows: 10_000,
                    timeout_secs: 5,
                },
            },
            tenant,
            filters: ViewFilters::default(),
            format: "ndjson".to_string(),
            header: true,
            client_tracking_id: None,
        });

        let status = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                match controller.get_status("t1", &job_id) {
                    Some(status @ JobStatus::Completed { .. })
                    | Some(status @ JobStatus::Failed { .. }) => return status,
                    _ => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
        })
        .await
        .expect("export job must reach a terminal state before the timeout");

        match status {
            JobStatus::Failed {
                message, status, ..
            } => {
                assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
                assert_eq!(message, server_fault_message(&job_id));
                assert!(
                    !message.contains("statement timeout"),
                    "unexpected message: {message}"
                );
                assert!(
                    !message.contains("malformed"),
                    "a source failure must not be blamed on a malformed Library: {message}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// A storage failure mid-materialization of a ViewDefinition subject must
    /// fail the export job (a server fault, with the cause kept in the server
    /// log rather than the result, #1703), instead of silently dropping the
    /// failed rows and reporting the job as complete with a truncated file.
    #[tokio::test]
    async fn view_export_fails_with_diagnostic_when_row_stream_errors() {
        let runner = Arc::new(FailingRunner);
        let controller =
            InMemoryController::new(runner, InMemorySink::new("http://localhost"), None);

        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        let view = serde_json::json!({
            "resourceType": "ViewDefinition",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"name": "id", "path": "id"}]}]
        });

        let job_id = controller.submit(ExportTask {
            work: ExportWork {
                views: vec![NamedView {
                    name: "patients".to_string(),
                    view,
                }],
                queries: vec![],
                limits: SqlExportLimits::default(),
            },
            tenant,
            filters: ViewFilters::default(),
            format: "ndjson".to_string(),
            header: true,
            client_tracking_id: None,
        });

        let status = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                match controller.get_status("t1", &job_id) {
                    Some(status @ JobStatus::Completed { .. })
                    | Some(status @ JobStatus::Failed { .. }) => return status,
                    _ => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
        })
        .await
        .expect("export job must reach a terminal state before the timeout");

        match status {
            JobStatus::Failed {
                message, status, ..
            } => {
                assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
                assert_eq!(message, server_fault_message(&job_id));
                assert!(
                    !message.contains("statement timeout"),
                    "unexpected message: {message}"
                );
            }
            other => panic!(
                "expected Failed (a mid-stream error must not be reported as a completed job \
                 with a truncated file), got {other:?}"
            ),
        }
    }

    /// The cleanup reaper deletes terminal jobs older than the TTL (output +
    /// bookkeeping) while leaving running jobs untouched.
    #[test]
    fn reap_expired_reclaims_terminal_jobs_only() {
        let sink = InMemorySink::new("http://localhost");
        let jobs: DashMap<String, JobStatus> = DashMap::new();
        let job_tenants: DashMap<String, String> = DashMap::new();

        // An old completed job with written output — should be reclaimed.
        let old = "old-completed".to_string();
        let two_hours_ago = Utc::now() - chrono::Duration::hours(2);
        jobs.insert(
            old.clone(),
            JobStatus::Completed {
                files: Vec::new(),
                submitted_at: two_hours_ago,
                completed_at: two_hours_ago,
                format: "ndjson".to_string(),
                client_tracking_id: None,
            },
        );
        job_tenants.insert(old.clone(), "t1".to_string());
        sink.write_shard(&old, 0, b"old\n".to_vec(), "ndjson")
            .unwrap();

        // A freshly completed job — newer than the TTL, should survive.
        let fresh = "fresh-completed".to_string();
        jobs.insert(
            fresh.clone(),
            JobStatus::Completed {
                files: Vec::new(),
                submitted_at: Utc::now(),
                completed_at: Utc::now(),
                format: "ndjson".to_string(),
                client_tracking_id: None,
            },
        );
        job_tenants.insert(fresh.clone(), "t1".to_string());

        // A running job — never reclaimed regardless of age.
        let running = "still-running".to_string();
        jobs.insert(
            running.clone(),
            JobStatus::Running {
                subjects_done: 1,
                subjects_total: 10,
                current_subject: Some("still-writing".to_string()),
                submitted_at: two_hours_ago,
            },
        );
        job_tenants.insert(running.clone(), "t1".to_string());

        // Reap anything terminal for longer than one hour.
        reap_expired(&jobs, &job_tenants, &sink, Duration::from_secs(3600));

        // Old completed job and its output are gone.
        assert!(jobs.get(&old).is_none(), "expired job should be removed");
        assert!(
            job_tenants.get(&old).is_none(),
            "tenant entry should be removed"
        );
        assert!(
            sink.read_shard(&old, "shard-0.ndjson").is_none(),
            "expired job's output should be deleted"
        );

        // Fresh completed job and the running job survive.
        assert!(jobs.get(&fresh).is_some(), "fresh job must survive");
        assert!(
            jobs.get(&running).is_some(),
            "running job must never be reaped"
        );
    }

    /// An `ExportSink` whose `load_completed` returns a fixed, test-chosen
    /// list of `(key, manifest)` pairs, standing in for whatever
    /// `FilesystemSink::load_completed` would have scanned off disk. Lets a
    /// test hand `rehydrate_completed_jobs` a manifest/key mismatch directly,
    /// without touching the filesystem.
    #[derive(Clone)]
    struct StubRehydrationSink {
        manifests: Vec<(String, JobManifest)>,
    }

    impl ExportSink for StubRehydrationSink {
        fn write_shard(
            &self,
            _job_id: &str,
            _shard_index: usize,
            _data: Vec<u8>,
            _ext: &str,
        ) -> Result<String, ExportError> {
            unimplemented!("not exercised by rehydration tests")
        }
        fn read_shard(&self, _job_id: &str, _filename: &str) -> Option<Vec<u8>> {
            None
        }
        fn download_url(
            &self,
            _public_base_url: &str,
            _job_id: &str,
            _filename: &str,
        ) -> Result<String, ExportError> {
            unimplemented!("not exercised by rehydration tests")
        }
        fn delete_job(&self, _job_id: &str) -> Result<(), ExportError> {
            Ok(())
        }
        fn load_completed(&self) -> Vec<(String, JobManifest)> {
            self.manifests.clone()
        }
    }

    /// A minimal, otherwise-valid manifest for the given job id/tenant, for
    /// tests that only care about the key/`job_id` relationship.
    fn stub_manifest(job_id: &str, tenant_id: &str) -> JobManifest {
        JobManifest {
            version: MANIFEST_VERSION,
            job_id: job_id.to_string(),
            tenant_id: tenant_id.to_string(),
            format: "ndjson".to_string(),
            files: Vec::new(),
            submitted_at: Utc::now(),
            completed_at: Utc::now(),
            client_tracking_id: None,
        }
    }

    /// A manifest is only rehydrated when its own `job_id` agrees with the
    /// storage key it was loaded under (the containing directory name) *and*
    /// that key is itself a UUID — otherwise a job's files could be served
    /// under a path that disagrees with the job's own record (#1474). A
    /// manifest that does agree is the working path: it must actually land
    /// in both `jobs` and `job_tenants`, so this test would still pass if
    /// rehydration were a no-op without the positive case below.
    #[test]
    fn rehydration_skips_manifests_whose_job_id_disagrees_with_their_key() {
        let uuid_a = Uuid::new_v4().to_string();
        let uuid_b = Uuid::new_v4().to_string();
        let uuid_c = Uuid::new_v4().to_string();
        let sink = StubRehydrationSink {
            manifests: vec![
                // Key isn't a UUID at all.
                ("not-a-uuid".to_string(), stub_manifest("not-a-uuid", "t1")),
                // Key is a UUID, but disagrees with the manifest's own job_id.
                (uuid_a.clone(), stub_manifest(&uuid_b, "t1")),
                // Key agrees with the manifest's own job_id — must rehydrate.
                (uuid_c.clone(), stub_manifest(&uuid_c, "t1")),
            ],
        };
        let jobs: DashMap<String, JobStatus> = DashMap::new();
        let job_tenants: DashMap<String, String> = DashMap::new();

        rehydrate_completed_jobs(&jobs, &job_tenants, &sink);

        assert!(
            jobs.get("not-a-uuid").is_none(),
            "a non-UUID storage key must never be rehydrated"
        );
        assert!(
            jobs.get(&uuid_a).is_none(),
            "a key/job_id mismatch must not be rehydrated under the key"
        );
        assert!(
            jobs.get(&uuid_b).is_none(),
            "a key/job_id mismatch must not be rehydrated under the manifest's job_id either"
        );
        assert!(
            jobs.get(&uuid_c).is_some(),
            "a matching key/job_id manifest must be rehydrated"
        );
        assert_eq!(
            job_tenants.get(&uuid_c).as_deref().map(String::as_str),
            Some("t1"),
            "a rehydrated job's tenant must be recorded in job_tenants"
        );
    }

    /// A job rehydrated from a manifest that's already older than the
    /// configured TTL must not linger until the reaper's first scheduled
    /// tick — `with_options` runs one reap pass immediately after
    /// rehydrating, so it's gone (status and on-disk directory both) as soon
    /// as the controller is constructed.
    #[tokio::test]
    async fn startup_reap_removes_an_already_expired_rehydrated_job() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sink = crate::export::sink::FilesystemSink::new(dir.path(), "http://localhost");

        // Complete a job directly against the sink and its manifest — this
        // test only cares about what `with_options` does with an on-disk
        // manifest at startup, not about running an export.
        let job_id = Uuid::new_v4().to_string();
        sink.write_shard(&job_id, 0, b"{}\n".to_vec(), "ndjson")
            .unwrap();
        sink.persist_completion(&job_id, &stub_manifest(&job_id, "t1"))
            .unwrap();

        // Let the manifest's `completed_at` age past a 1ms TTL.
        tokio::time::sleep(Duration::from_millis(20)).await;

        let runner = Arc::new(BlockingRunner {
            release: Arc::new(Notify::new()),
        });
        let controller = InMemoryController::with_options(
            runner,
            sink,
            None,
            None,
            Some(CleanupConfig {
                output_ttl: Duration::from_millis(1),
                interval: Duration::from_secs(3600),
            }),
        );

        assert!(
            controller.get_status("t1", &job_id).is_none(),
            "an already-expired rehydrated job must be reaped at startup"
        );
        assert!(
            !dir.path().join(&job_id).exists(),
            "the startup reap must delete the expired job's directory too"
        );
    }

    #[test]
    fn csv_cell_neutralizes_formula_text_but_not_numbers() {
        use serde_json::json;
        for (input, expected) in [
            (json!("=1+2"), "'=1+2"),
            (json!("+cmd"), "'+cmd"),
            (json!("@x"), "'@x"),
            (json!("-2+3"), "'-2+3"),
            (json!("-5"), "-5"),
            (json!("+3.5"), "+3.5"),
            (json!(-5), "-5"),
            (json!(3.5), "3.5"),
            (json!("plain"), "plain"),
            (json!("=a,b"), "\"'=a,b\""),
        ] {
            assert_eq!(csv_cell(&input), expected, "input {input}");
        }
    }

    #[test]
    fn view_and_query_csv_writers_agree_on_formula_cells() {
        use serde_json::json;
        let rows = vec![
            json!({"a": "=1+2", "b": -5}),
            json!({"a": "-5", "b": "@SUM(A1)"}),
            json!({"a": "=x,y", "b": "+3.5"}),
        ];
        let view = format_csv(&rows, true).unwrap();
        assert_eq!(
            String::from_utf8(view.clone()).unwrap(),
            "a,b\n'=1+2,-5\n-5,'@SUM(A1)\n\"'=x,y\",+3.5\n"
        );
        let query =
            helios_sof::format_csv(helios_sof::rows_to_processed_result(rows.clone()), true)
                .unwrap();
        assert_eq!(view, query);
    }

    #[test]
    fn download_url_lifetime_cap_is_the_remaining_retention_with_a_floor() {
        let ttl = Duration::from_secs(24 * 3600);
        let now = Utc::now();
        let cap = |ttl, at| download_url_lifetime_cap(ttl, at, now);
        let secs = chrono::Duration::seconds;

        assert_eq!(cap(ttl, now), ttl);
        assert_eq!(
            cap(ttl, now - chrono::Duration::hours(1)),
            Duration::from_secs(23 * 3600)
        );
        assert_eq!(
            cap(ttl, now - secs(24 * 3600 - 10)),
            MIN_DOWNLOAD_URL_LIFETIME
        );
        assert_eq!(
            cap(ttl, now - chrono::Duration::hours(25)),
            MIN_DOWNLOAD_URL_LIFETIME
        );
        // A terminal time in the future (clock skew) counts as age zero.
        assert_eq!(cap(ttl, now + chrono::Duration::minutes(5)), ttl);
        // The floor applies even when the whole retention is shorter; the S3
        // sink still takes the minimum with its configured presign TTL.
        assert_eq!(cap(Duration::from_secs(30), now), MIN_DOWNLOAD_URL_LIFETIME);
    }

    /// Records the cap each `download_url*` call received (`None` = the
    /// uncapped `download_url`).
    #[derive(Clone)]
    struct CapRecordingSink {
        caps: Arc<std::sync::Mutex<Vec<Option<Duration>>>>,
    }

    impl ExportSink for CapRecordingSink {
        fn write_shard(
            &self,
            _job_id: &str,
            shard_index: usize,
            _data: Vec<u8>,
            ext: &str,
        ) -> Result<String, ExportError> {
            Ok(format!("shard-{shard_index}.{ext}"))
        }

        fn read_shard(&self, _job_id: &str, _filename: &str) -> Option<Vec<u8>> {
            None
        }

        fn download_url(
            &self,
            _public_base_url: &str,
            _job_id: &str,
            _filename: &str,
        ) -> Result<String, ExportError> {
            self.caps.lock().unwrap().push(None);
            Ok("https://signed.example/uncapped".to_string())
        }

        fn download_url_capped(
            &self,
            _public_base_url: &str,
            _job_id: &str,
            _filename: &str,
            max_lifetime: Duration,
        ) -> Result<String, ExportError> {
            self.caps.lock().unwrap().push(Some(max_lifetime));
            Ok("https://signed.example/capped".to_string())
        }

        fn delete_job(&self, _job_id: &str) -> Result<(), ExportError> {
            Ok(())
        }
    }

    /// With a reaper configured the controller caps the URL at the job's
    /// remaining retention; without one nothing deletes the output, so the URL
    /// is left uncapped (#1706).
    #[tokio::test]
    async fn controller_caps_download_url_lifetime_at_remaining_retention() {
        let caps = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = CapRecordingSink { caps: caps.clone() };
        let runner = Arc::new(BlockingRunner {
            release: Arc::new(Notify::new()),
        });
        let completed = || {
            let at = Utc::now() - chrono::Duration::hours(1);
            JobStatus::Completed {
                files: vec![],
                submitted_at: at,
                completed_at: at,
                format: "ndjson".to_string(),
                client_tracking_id: None,
            }
        };

        let controller = InMemoryController::with_options(
            runner.clone(),
            sink.clone(),
            None,
            None,
            Some(CleanupConfig {
                output_ttl: Duration::from_secs(7200),
                interval: Duration::from_secs(3600),
            }),
        );
        controller
            .job_tenants
            .insert("job-1".to_string(), "t1".to_string());
        controller.jobs.insert("job-1".to_string(), completed());
        assert!(
            controller
                .download_url("t1", "https://public.example", "job-1", "shard-0.ndjson")
                .is_some()
        );
        {
            let recorded = caps.lock().unwrap();
            assert_eq!(recorded.len(), 1);
            let d = recorded[0].expect("a reaper is configured, so the URL is capped");
            assert!(
                d >= Duration::from_secs(3590) && d <= Duration::from_secs(3600),
                "unexpected cap {d:?}"
            );
        }

        caps.lock().unwrap().clear();
        let no_reaper = InMemoryController::new(runner, sink, None);
        no_reaper
            .job_tenants
            .insert("job-1".to_string(), "t1".to_string());
        no_reaper.jobs.insert("job-1".to_string(), completed());
        assert!(
            no_reaper
                .download_url("t1", "https://public.example", "job-1", "shard-0.ndjson")
                .is_some()
        );
        assert_eq!(*caps.lock().unwrap(), vec![None]);
    }

    /// `write_shard` returns the shard's filename (not a URL), and the sink
    /// resolves that filename to a stable server-routed URL on demand.
    #[test]
    fn in_memory_sink_writes_filename_and_resolves_url() {
        let sink = InMemorySink::new("http://localhost/");
        let filename = sink
            .write_shard("job-1", 0, b"{}\n".to_vec(), "ndjson")
            .unwrap();
        assert_eq!(filename, "shard-0.ndjson");
        assert_eq!(
            sink.download_url("http://localhost", "job-1", &filename)
                .unwrap(),
            "http://localhost/export/job-1/shard-0.ndjson"
        );
    }

    /// An `ExportSink` whose `download_url` returns a different URL on every
    /// call, standing in for S3's per-poll re-signing.
    #[derive(Clone)]
    struct ResigningSink {
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl ExportSink for ResigningSink {
        fn write_shard(
            &self,
            _job_id: &str,
            shard_index: usize,
            _data: Vec<u8>,
            ext: &str,
        ) -> Result<String, ExportError> {
            Ok(format!("shard-{shard_index}.{ext}"))
        }
        fn read_shard(&self, _job_id: &str, _filename: &str) -> Option<Vec<u8>> {
            None
        }
        fn download_url(
            &self,
            _public_base_url: &str,
            job_id: &str,
            filename: &str,
        ) -> Result<String, ExportError> {
            // Each call advances the nonce, mimicking a fresh pre-signature.
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(format!(
                "https://signed.example/{job_id}/{filename}?sig={n}"
            ))
        }
        fn delete_job(&self, _job_id: &str) -> Result<(), ExportError> {
            Ok(())
        }
    }

    /// The controller re-resolves a shard's URL on every call (so each manifest
    /// poll hands out a freshly signed URL), and gates resolution by tenant.
    #[tokio::test]
    async fn controller_download_url_is_fresh_and_tenant_gated() {
        let sink = ResigningSink {
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };
        let runner = Arc::new(BlockingRunner {
            release: Arc::new(Notify::new()),
        });
        let controller = InMemoryController::new(runner, sink, None);

        // Register a job for tenant t1 (download_url only checks ownership).
        let job_id = "job-1".to_string();
        controller
            .job_tenants
            .insert(job_id.clone(), "t1".to_string());

        // Two polls yield two distinct URLs — proof the URL is resolved fresh
        // rather than reused from write time.
        let first = controller
            .download_url(
                "t1",
                "https://public.example/fhir/acme",
                &job_id,
                "shard-0.ndjson",
            )
            .expect("owner should resolve a URL");
        let second = controller
            .download_url(
                "t1",
                "https://public.example/fhir/acme",
                &job_id,
                "shard-0.ndjson",
            )
            .expect("owner should resolve a URL");
        assert!(first.starts_with("https://signed.example/"));
        assert_ne!(first, second, "each poll must re-resolve the download URL");

        // A different tenant cannot resolve URLs for this job.
        assert!(
            controller
                .download_url(
                    "other",
                    "https://public.example/fhir/other",
                    &job_id,
                    "shard-0.ndjson",
                )
                .is_none(),
            "cross-tenant resolution must be denied"
        );
    }

    // ------------------------------------------------------------------
    // #1704: cancel semantics. A job cancelled while queued must never
    // start, a running job must notice a cancel at its next checkpoint, a
    // job whose entry was reaped must not orphan its output, and a failed
    // reaper delete must be retried.
    // ------------------------------------------------------------------

    /// A `SofRunner` that records the `"name"` of every view it is asked to
    /// run and streams `total` rows per call, parking once at a gate (a
    /// `watch` flag the test opens) after `gate_after` rows. Rows yielded
    /// across all calls are counted in `produced`, so a test can tell how far
    /// a job read before it stopped.
    struct GateRunner {
        seen: Arc<std::sync::Mutex<Vec<String>>>,
        produced: Arc<AtomicUsize>,
        reached: tokio::sync::mpsc::UnboundedSender<String>,
        open: tokio::sync::watch::Receiver<bool>,
        gate_after: usize,
        total: usize,
    }

    /// Per-stream state of a [`GateRunner`] row stream.
    struct GateState {
        name: String,
        next: usize,
        gated: bool,
        produced: Arc<AtomicUsize>,
        reached: tokio::sync::mpsc::UnboundedSender<String>,
        open: tokio::sync::watch::Receiver<bool>,
        gate_after: usize,
        total: usize,
    }

    #[async_trait]
    impl SofRunner for GateRunner {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            view_definition: serde_json::Value,
            _filters: ViewFilters,
        ) -> Result<RowStream, SofError> {
            let name = view_definition["name"].as_str().unwrap_or("").to_string();
            self.seen.lock().unwrap().push(name.clone());
            let state = GateState {
                name,
                next: 0,
                gated: false,
                produced: Arc::clone(&self.produced),
                reached: self.reached.clone(),
                open: self.open.clone(),
                gate_after: self.gate_after,
                total: self.total,
            };
            Ok(Box::pin(futures::stream::unfold(
                state,
                |mut s| async move {
                    if !s.gated && s.next == s.gate_after {
                        s.gated = true;
                        let _ = s.reached.send(s.name.clone());
                        let _ = s.open.wait_for(|o| *o).await;
                    }
                    if s.next >= s.total {
                        return None;
                    }
                    let row = serde_json::json!({"id": format!("p{}", s.next)});
                    s.next += 1;
                    s.produced.fetch_add(1, Ordering::SeqCst);
                    Some((Ok::<_, SofError>(row), s))
                },
            )))
        }

        fn runner_name(&self) -> &'static str {
            "gate-test-runner"
        }
    }

    /// A [`GateRunner`] plus the test-side ends of its channels.
    struct Gate {
        runner: Arc<GateRunner>,
        seen: Arc<std::sync::Mutex<Vec<String>>>,
        produced: Arc<AtomicUsize>,
        reached: tokio::sync::mpsc::UnboundedReceiver<String>,
        open: tokio::sync::watch::Sender<bool>,
    }

    impl Gate {
        /// `open == true` builds a runner whose gate never holds anything up.
        fn new(gate_after: usize, total: usize, open: bool) -> Self {
            let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
            let produced = Arc::new(AtomicUsize::new(0));
            let (reached_tx, reached) = tokio::sync::mpsc::unbounded_channel();
            let (open_tx, open_rx) = tokio::sync::watch::channel(open);
            Self {
                runner: Arc::new(GateRunner {
                    seen: Arc::clone(&seen),
                    produced: Arc::clone(&produced),
                    reached: reached_tx,
                    open: open_rx,
                    gate_after,
                    total,
                }),
                seen,
                produced,
                reached,
                open: open_tx,
            }
        }

        /// Waits for a stream to park at the gate; returns the view's name.
        async fn reached(&mut self) -> String {
            tokio::time::timeout(Duration::from_secs(15), self.reached.recv())
                .await
                .expect("a view must reach the gate before the timeout")
                .expect("the runner outlives the test")
        }

        fn open(&self) {
            self.open.send(true).expect("the runner holds a receiver");
        }

        fn seen(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }
    }

    /// An [`InMemorySink`] with failure injection and write hooks.
    #[derive(Clone)]
    struct ScriptedSink {
        inner: InMemorySink,
        writes: Arc<AtomicUsize>,
        #[allow(clippy::type_complexity)]
        after_write: Arc<std::sync::Mutex<Option<Box<dyn Fn(&str) + Send + Sync>>>>,
        fail_deletes: Arc<AtomicBool>,
        deletes: Arc<AtomicUsize>,
    }

    impl ScriptedSink {
        fn new() -> Self {
            Self {
                inner: InMemorySink::new("http://localhost"),
                writes: Arc::new(AtomicUsize::new(0)),
                after_write: Arc::new(std::sync::Mutex::new(None)),
                fail_deletes: Arc::new(AtomicBool::new(false)),
                deletes: Arc::new(AtomicUsize::new(0)),
            }
        }

        /// Runs `hook(job_id)` after every successful `write_shard`.
        fn on_write(&self, hook: impl Fn(&str) + Send + Sync + 'static) {
            *self.after_write.lock().unwrap() = Some(Box::new(hook));
        }

        fn writes(&self) -> usize {
            self.writes.load(Ordering::SeqCst)
        }
    }

    impl ExportSink for ScriptedSink {
        fn write_shard(
            &self,
            job_id: &str,
            shard_index: usize,
            data: Vec<u8>,
            ext: &str,
        ) -> Result<String, ExportError> {
            let filename = self.inner.write_shard(job_id, shard_index, data, ext)?;
            if let Some(hook) = self.after_write.lock().unwrap().as_ref() {
                hook(job_id);
            }
            // Counted after the hook, so a test that sees the write also sees
            // the hook's effect.
            self.writes.fetch_add(1, Ordering::SeqCst);
            Ok(filename)
        }

        fn read_shard(&self, job_id: &str, filename: &str) -> Option<Vec<u8>> {
            self.inner.read_shard(job_id, filename)
        }

        fn download_url(
            &self,
            public_base_url: &str,
            job_id: &str,
            filename: &str,
        ) -> Result<String, ExportError> {
            self.inner.download_url(public_base_url, job_id, filename)
        }

        fn delete_job(&self, job_id: &str) -> Result<(), ExportError> {
            self.deletes.fetch_add(1, Ordering::SeqCst);
            if self.fail_deletes.load(Ordering::SeqCst) {
                return Err(ExportError::Sink("injected delete failure".into()));
            }
            self.inner.delete_job(job_id)
        }
    }

    /// Waits until every permit is back, i.e. no job task is still running.
    /// Only call it once the job's task is known to hold its permit.
    async fn wait_until_idle<S: ExportSink>(controller: &InMemoryController<S>, max: usize) {
        tokio::time::timeout(Duration::from_secs(15), async {
            while controller.semaphore.available_permits() != max {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the job task must release its concurrency permit before the timeout");
    }

    /// Waits until the sink has seen at least `n` shard writes.
    async fn wait_for_writes(sink: &ScriptedSink, n: usize) {
        tokio::time::timeout(Duration::from_secs(15), async {
            while sink.writes() < n {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the job must write a shard before the timeout");
    }

    fn named_view_json(name: &str) -> serde_json::Value {
        serde_json::json!({
            "resourceType": "ViewDefinition",
            "name": name,
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [{"name": "id", "path": "id"}]}]
        })
    }

    /// An ndjson export for tenant `t1` with one view subject per name.
    fn view_task(names: &[&str]) -> ExportTask {
        ExportTask {
            work: ExportWork {
                views: names
                    .iter()
                    .map(|n| NamedView {
                        name: n.to_string(),
                        view: named_view_json(n),
                    })
                    .collect(),
                ..Default::default()
            },
            tenant: TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access()),
            filters: ViewFilters::default(),
            format: "ndjson".to_string(),
            header: true,
            client_tracking_id: None,
        }
    }

    /// #1704: a job cancelled while it waited for a concurrency slot must
    /// not run once the slot frees up.
    #[tokio::test]
    async fn a_job_cancelled_while_queued_never_starts() {
        let mut gate = Gate::new(0, 0, false);
        let controller = InMemoryController::new(
            gate.runner.clone(),
            InMemorySink::new("http://localhost"),
            Some(1),
        );

        let _a = controller.submit(view_task(&["a"]));
        assert_eq!(gate.reached().await, "a");

        // B parks on the semaphore behind A.
        let b = controller.submit(view_task(&["b"]));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(controller.cancel("t1", &b));

        gate.open();
        let c = controller.submit(view_task(&["c"]));
        terminal_status(&controller, &c).await;

        // Permits are handed out in request order, so B was decided before C.
        assert_eq!(
            gate.seen(),
            vec!["a", "c"],
            "the cancelled job must not run"
        );
        assert!(matches!(
            controller.get_status("t1", &b),
            Some(JobStatus::Cancelled { .. })
        ));
    }

    /// #1704: a running job stops at the next subject boundary once cancelled.
    #[tokio::test]
    async fn a_running_job_stops_between_subjects_once_cancelled() {
        let mut gate = Gate::new(0, 0, false);
        let controller = InMemoryController::new(
            gate.runner.clone(),
            InMemorySink::new("http://localhost"),
            None,
        );

        let job_id = controller.submit(view_task(&["first", "second"]));
        assert_eq!(gate.reached().await, "first");
        assert!(controller.cancel("t1", &job_id));
        gate.open();
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;

        assert_eq!(
            gate.seen(),
            vec!["first"],
            "no subject may start after a cancel"
        );
    }

    /// #1704: a running job stops before writing its next shard once cancelled.
    #[tokio::test]
    async fn a_running_job_stops_before_its_next_shard_once_cancelled() {
        let gate = Gate::new(0, 3, true);
        let sink = ScriptedSink::new();
        let controller =
            InMemoryController::with_shard_rows(gate.runner.clone(), sink.clone(), None, Some(1));

        // A DELETE lands right after the first shard is written.
        let jobs = Arc::clone(&controller.jobs);
        sink.on_write(move |jid| {
            if let Some(mut entry) = jobs.get_mut(jid) {
                *entry = JobStatus::Cancelled {
                    cancelled_at: Utc::now(),
                };
            }
        });

        let job_id = controller.submit(view_task(&["patients"]));
        wait_for_writes(&sink, 1).await;
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;

        assert_eq!(sink.writes(), 1, "no shard may be written after a cancel");
        assert!(matches!(
            controller.get_status("t1", &job_id),
            Some(JobStatus::Cancelled { .. })
        ));
        assert!(sink.read_shard(&job_id, "shard-0.ndjson").is_none());
    }

    /// #1704: a running SQL subject stops before writing its next shard once
    /// cancelled.
    #[tokio::test]
    async fn a_running_sql_query_stops_before_its_next_shard_once_cancelled() {
        let gate = Gate::new(0, 3, true);
        let sink = ScriptedSink::new();
        let controller =
            InMemoryController::with_shard_rows(gate.runner.clone(), sink.clone(), None, Some(1));

        // A DELETE lands right after the first shard is written.
        let jobs = Arc::clone(&controller.jobs);
        sink.on_write(move |jid| {
            if let Some(mut entry) = jobs.get_mut(jid) {
                *entry = JobStatus::Cancelled {
                    cancelled_at: Utc::now(),
                };
            }
        });

        let mut task = view_task(&[]);
        task.work = ExportWork {
            views: vec![],
            queries: vec![NamedSqlQuery {
                name: "families".to_string(),
                sql: "SELECT * FROM vd_0".to_string(),
                plan: crate::handlers::sof::graph::GraphPlan {
                    nodes: vec![crate::handlers::sof::graph::PlanNode::Leaf {
                        internal_name: "vd_0".to_string(),
                        view: named_view_json("leaf"),
                    }],
                    subject_edges: Vec::new(),
                },
                bindings: Vec::new(),
            }],
            limits: SqlExportLimits {
                max_source_rows_per_vd: 100,
                max_rows: 100,
                timeout_secs: 5,
            },
        };

        let job_id = controller.submit(task);
        wait_for_writes(&sink, 1).await;
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;

        assert_eq!(sink.writes(), 1, "no shard may be written after a cancel");
        assert!(matches!(
            controller.get_status("t1", &job_id),
            Some(JobStatus::Cancelled { .. })
        ));
        assert!(sink.read_shard(&job_id, "shard-0.ndjson").is_none());
    }

    /// #1704: a job the reaper removed while it was queued must not run
    /// either.
    #[tokio::test]
    async fn a_job_reaped_while_queued_never_starts() {
        let mut gate = Gate::new(0, 0, false);
        let controller = InMemoryController::new(
            gate.runner.clone(),
            InMemorySink::new("http://localhost"),
            Some(1),
        );

        let _a = controller.submit(view_task(&["a"]));
        assert_eq!(gate.reached().await, "a");

        // B parks on the semaphore behind A.
        let b = controller.submit(view_task(&["b"]));
        tokio::time::sleep(Duration::from_millis(50)).await;
        // The reaper drops both entries while B waits.
        controller.jobs.remove(&b);
        controller.job_tenants.remove(&b);

        gate.open();
        let c = controller.submit(view_task(&["c"]));
        terminal_status(&controller, &c).await;

        assert_eq!(gate.seen(), vec!["a", "c"], "the reaped job must not run");
        assert!(controller.get_status("t1", &b).is_none());
    }

    /// #1704: a view subject stops draining its row stream soon after a cancel.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_running_view_stops_reading_rows_once_cancelled() {
        let total = 4 * CANCEL_CHECK_ROWS;
        let mut gate = Gate::new(10, total, false);
        let controller = InMemoryController::new(
            gate.runner.clone(),
            InMemorySink::new("http://localhost"),
            None,
        );

        let job_id = controller.submit(view_task(&["patients"]));
        gate.reached().await;
        assert!(controller.cancel("t1", &job_id));
        gate.open();
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;

        let produced = gate.produced.load(Ordering::SeqCst);
        assert!(
            produced <= 10 + CANCEL_CHECK_ROWS && produced < total,
            "a cancelled job must stop reading rows, read {produced} of {total}"
        );
    }

    /// #1704: the same for a SQL subject, whose leaf rows are materialized
    /// into the in-memory engine by `execute_plan`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_running_sql_query_stops_reading_rows_once_cancelled() {
        let total = 4 * CANCEL_CHECK_ROWS;
        let mut gate = Gate::new(10, total, false);
        let controller = InMemoryController::new(
            gate.runner.clone(),
            InMemorySink::new("http://localhost"),
            None,
        );

        let mut task = view_task(&[]);
        task.work = ExportWork {
            views: vec![],
            queries: vec![NamedSqlQuery {
                name: "families".to_string(),
                sql: "SELECT * FROM vd_0".to_string(),
                plan: crate::handlers::sof::graph::GraphPlan {
                    nodes: vec![crate::handlers::sof::graph::PlanNode::Leaf {
                        internal_name: "vd_0".to_string(),
                        view: named_view_json("leaf"),
                    }],
                    subject_edges: Vec::new(),
                },
                bindings: Vec::new(),
            }],
            limits: SqlExportLimits {
                max_source_rows_per_vd: 10 * total,
                max_rows: 10 * total,
                timeout_secs: 5,
            },
        };

        let job_id = controller.submit(task);
        gate.reached().await;
        assert!(controller.cancel("t1", &job_id));
        gate.open();
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;

        let produced = gate.produced.load(Ordering::SeqCst);
        assert!(
            produced <= 10 + CANCEL_CHECK_ROWS && produced < total,
            "a cancelled job must stop reading rows, read {produced} of {total}"
        );
    }

    /// #1704: when the reaper removed the job's entry while the task was still
    /// running, the task still deletes what it wrote.
    #[tokio::test]
    async fn the_worker_deletes_what_it_wrote_for_a_job_the_reaper_already_removed() {
        let gate = Gate::new(0, 1, true);
        let sink = ScriptedSink::new();
        let controller = InMemoryController::new(gate.runner.clone(), sink.clone(), None);

        // The reaper drops both entries right after the shard lands.
        let jobs = Arc::clone(&controller.jobs);
        let job_tenants = Arc::clone(&controller.job_tenants);
        sink.on_write(move |jid| {
            jobs.remove(jid);
            job_tenants.remove(jid);
        });

        let job_id = controller.submit(view_task(&["patients"]));
        wait_for_writes(&sink, 1).await;
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;

        assert!(
            sink.read_shard(&job_id, "shard-0.ndjson").is_none(),
            "output of a job nobody can reach any more must not be left behind"
        );
    }

    /// #1704: a delete that failed is retried by the next sweep, so the
    /// status entry stays as the retry handle. The tenant entry is dropped
    /// regardless, which keeps the expired job unreachable for clients.
    #[test]
    fn reap_expired_keeps_a_job_whose_delete_failed_and_retries_it() {
        let sink = ScriptedSink::new();
        sink.fail_deletes.store(true, Ordering::SeqCst);
        let jobs: DashMap<String, JobStatus> = DashMap::new();
        let job_tenants: DashMap<String, String> = DashMap::new();

        let id = "old-completed".to_string();
        let two_hours_ago = Utc::now() - chrono::Duration::hours(2);
        jobs.insert(
            id.clone(),
            JobStatus::Completed {
                files: Vec::new(),
                submitted_at: two_hours_ago,
                completed_at: two_hours_ago,
                format: "ndjson".to_string(),
                client_tracking_id: None,
            },
        );
        job_tenants.insert(id.clone(), "t1".to_string());
        sink.write_shard(&id, 0, b"old\n".to_vec(), "ndjson")
            .unwrap();

        reap_expired(&jobs, &job_tenants, &sink, Duration::from_secs(3600));

        assert_eq!(sink.deletes.load(Ordering::SeqCst), 1);
        assert!(
            jobs.contains_key(&id),
            "a job whose delete failed must stay for the next sweep"
        );
        assert!(
            !job_tenants.contains_key(&id),
            "an expired job stops being served even when its delete failed"
        );
        assert!(sink.read_shard(&id, "shard-0.ndjson").is_some());

        sink.fail_deletes.store(false, Ordering::SeqCst);
        reap_expired(&jobs, &job_tenants, &sink, Duration::from_secs(3600));

        assert_eq!(sink.deletes.load(Ordering::SeqCst), 2);
        assert!(!jobs.contains_key(&id), "the retry reclaims the job");
        assert!(sink.read_shard(&id, "shard-0.ndjson").is_none());
    }

    /// A runner whose start never completes, like one listing every key of a
    /// large type before its first row.
    struct NeverStarts;

    #[async_trait]
    impl SofRunner for NeverStarts {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            _view_definition: serde_json::Value,
            _filters: ViewFilters,
        ) -> Result<RowStream, SofError> {
            std::future::pending().await
        }

        fn runner_name(&self) -> &'static str {
            "never-starts"
        }
    }

    /// A runner that starts but never yields a row, like a view that filters
    /// out every resource of a large type.
    struct QuietRows;

    #[async_trait]
    impl SofRunner for QuietRows {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            _view_definition: serde_json::Value,
            _filters: ViewFilters,
        ) -> Result<RowStream, SofError> {
            Ok(Box::pin(futures::stream::pending()))
        }

        fn runner_name(&self) -> &'static str {
            "quiet-rows"
        }
    }

    fn running(jid: &str) -> Arc<DashMap<String, JobStatus>> {
        let jobs = Arc::new(DashMap::new());
        jobs.insert(
            jid.to_string(),
            JobStatus::Running {
                subjects_done: 0,
                subjects_total: 1,
                current_subject: None,
                submitted_at: Utc::now(),
            },
        );
        jobs
    }

    /// #1823: a cancel while the runner is still starting stops it within a
    /// poll, instead of waiting for a first row that may be minutes away.
    #[tokio::test]
    async fn a_cancel_while_the_runner_is_starting_stops_it() {
        let jobs = running("j");
        let runner = StopWhenNotRunning {
            inner: Arc::new(NeverStarts),
            jobs: Arc::clone(&jobs),
            jid: "j".to_string(),
        };
        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        let cancel = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            jobs.remove("j");
        };
        let (started, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(
                runner.run_view(&tenant, serde_json::json!({}), ViewFilters::default()),
                cancel
            )
        })
        .await
        .expect("the start is dropped soon after the cancel");
        assert!(matches!(started, Err(SofError::Cancelled)));
    }

    /// #1823: a row stream that yields nothing still ends, with `Cancelled`,
    /// soon after the job is cancelled.
    #[tokio::test]
    async fn a_quiet_row_stream_ends_cancelled_after_a_cancel() {
        let jobs = running("j");
        let runner = StopWhenNotRunning {
            inner: Arc::new(QuietRows),
            jobs: Arc::clone(&jobs),
            jid: "j".to_string(),
        };
        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        let mut rows = runner
            .run_view(&tenant, serde_json::json!({}), ViewFilters::default())
            .await
            .expect("started");
        jobs.remove("j");
        let first = tokio::time::timeout(Duration::from_secs(5), rows.next())
            .await
            .expect("the stream ends soon after the cancel");
        assert!(matches!(first, Some(Err(SofError::Cancelled))));
        assert!(rows.next().await.is_none());
    }
}
