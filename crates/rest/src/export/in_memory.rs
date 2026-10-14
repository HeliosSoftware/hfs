//! In-memory `ExportJobController` implementation.
//!
//! Each job runs inside a `tokio::spawn` task, bounded by a `Semaphore`.
//! Results are stored in a `DashMap<JobId, JobStatus>`. A tenant may have at
//! most `max_jobs_per_tenant` jobs queued or running at once; `submit` refuses
//! the rest.
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

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use futures::StreamExt;
use helios_persistence::core::sof_runner::{RunContext, RunKind, RunRequest, SofRunner};
use helios_sof::columnar_stream::{ParquetStreamWriter, STREAM_BATCH_ROWS};
use helios_sof::parquet_schema::{SCHEMA_SAMPLE_ROWS, resolve_arrow_schema};
use helios_sof::sqlquery::engine::declared_columns;
use helios_sof::sqlquery::{InMemorySqlEngine, QueryResult, SqlQueryError};
use tokio::sync::Semaphore;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use uuid::Uuid;

use super::controller::{
    CompletedFile, ExportError, ExportJobController, ExportTask, JobHandle, JobId, JobStatus,
    NamedSqlQuery, NamedView, SqlExportLimits, SubmitError,
};
use super::planner;
use super::sink::{ExportSink, JobManifest, MANIFEST_VERSION, ManifestFile};
use crate::error::RestError;
use crate::handlers::sof::run::map_sof_error_to_rest;
use crate::handlers::sof::sqlquery::sqlquery_err_to_rest;
use helios_persistence::core::sof_runner::SofError;

/// Why a job failed and how its result endpoint reports it. Every worker
/// error funnels through here: a failure that is the request's own keeps the
/// 4xx and issue code `$sql-run` would have answered with; a backend that was
/// unavailable keeps `503` (`transient`) and a run past the job's budget
/// keeps `504` (`timeout`), both with the generic wording `$sql-run` gives
/// them (#1864 D-6); everything else is a `500` whose text (backend or driver
/// detail) goes to the job log only; the stored result is generic (see
/// [`server_fault_message`]).
#[derive(Debug)]
pub(crate) struct JobFailure {
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

    /// The job stopped because it is no longer `Running` (cancelled, finished
    /// or removed). Never stored: the worker only logs it.
    fn stopped() -> Self {
        Self::server("export job is no longer running".to_string())
    }

    /// Whether a failure answered with `status` keeps it as the job's result
    /// status, with its own (client-safe) wording: the request's own fault
    /// (4xx), an unavailable backend (`503`) and a run past its budget
    /// (`504`). Anything else is a server fault: a generic `500`.
    fn keeps_status(status: StatusCode) -> bool {
        status.is_client_error()
            || status == StatusCode::SERVICE_UNAVAILABLE
            || status == StatusCode::GATEWAY_TIMEOUT
    }

    /// The `Failed` status this failure gives the job `job_id`, whose status
    /// was `running`: the request's own failure keeps its wording, a server
    /// fault stores the generic text (its detail stays in the log, #1703), and
    /// the subject in flight is reported even then (#1800).
    pub(crate) fn into_status(self, job_id: &str, running: &JobStatus) -> JobStatus {
        let (subject, submitted_at) = match running {
            JobStatus::Running {
                current_subject,
                submitted_at,
                ..
            } => (current_subject.clone(), *submitted_at),
            _ => (None, Utc::now()),
        };
        let message = if Self::keeps_status(self.status) {
            self.message
        } else {
            server_fault_message(job_id)
        };
        JobStatus::Failed {
            message,
            status: self.status,
            code: self.code,
            subject,
            submitted_at,
            failed_at: Utc::now(),
        }
    }

    /// A client-attributable REST error becomes a client failure with its
    /// own wording, and so do an unavailable backend (`503`) and a timeout
    /// (`504`), whose REST wording is generic ([`Self::keeps_status`]); the
    /// mapping already logged their detail. A server error keeps `detail`
    /// (the REST wording hides backend detail from clients, the job log wants
    /// it).
    fn from_rest(prefix: &str, err: RestError, detail: String) -> Self {
        let (status, code, message) = err.client_response();
        if Self::keeps_status(status) {
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

    /// A SQL Query subject's failure, answered as `$sql-run` answers it.
    fn from_sqlquery(prefix: &str, err: SqlQueryError) -> Self {
        let detail = err.to_string();
        Self::from_rest(prefix, sqlquery_err_to_rest(err), detail)
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

/// Default maximum number of export jobs one tenant may have queued or running.
pub const DEFAULT_MAX_JOBS_PER_TENANT: usize = 8;

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
/// shards based on [`shard_rows`](InMemoryController::new). A tenant may have
/// at most [`with_max_jobs_per_tenant`](InMemoryController::with_max_jobs_per_tenant)
/// jobs queued or running; a job waiting for its permit holds its whole task
/// in memory, so the queue is bounded per tenant.
pub struct InMemoryController<Sink: ExportSink> {
    jobs: Arc<DashMap<String, JobStatus>>,
    /// Tenant ID that submitted each job. Used to gate status / cancel /
    /// download so one tenant cannot access another tenant's exports.
    job_tenants: Arc<DashMap<String, String>>,
    /// Each submitted job's cancellation token (see [`JobHandle`]), fired on
    /// every transition out of `Running` and when the reaper removes the job.
    tokens: Arc<DashMap<String, CancellationToken>>,
    runner: Arc<dyn SofRunner>,
    sink: Sink,
    semaphore: Arc<Semaphore>,
    shard_rows: usize,
    /// Per tenant, the jobs whose worker task has not ended, queued or
    /// running. A tenant's entry is removed when its count reaches zero.
    active_jobs: Arc<DashMap<String, usize>>,
    max_jobs_per_tenant: usize,
    /// Retention of a finished job's output when a reaper runs (from
    /// [`CleanupConfig::output_ttl`]); caps the lifetime of the download URLs
    /// handed out. `None` (no reaper) leaves URLs uncapped because nothing
    /// deletes the output.
    output_ttl: Option<Duration>,
    /// Each job's time budget (`HFS_EXPORT_JOB_TIMEOUT_SECS`), counted from
    /// the moment it gets its concurrency permit; see
    /// [`with_job_budget`](InMemoryController::with_job_budget). `None`: no
    /// deadline.
    job_budget: Option<Duration>,
}

/// One of a tenant's places in [`InMemoryController::active_jobs`], held by a
/// job's worker task from `submit` until the task ends. Dropping it, including
/// on a panic, gives the place back.
struct TenantJobSlot {
    active: Arc<DashMap<String, usize>>,
    tenant: String,
}

impl TenantJobSlot {
    /// Takes a place for `tenant`, or returns `None` when it already holds `max`.
    fn acquire(active: &Arc<DashMap<String, usize>>, tenant: &str, max: usize) -> Option<Self> {
        // Refuse before touching the map, so a tenant that can never hold a
        // place leaves no zero-count entry behind.
        if max == 0 {
            return None;
        }
        {
            let mut n = active.entry(tenant.to_string()).or_insert(0);
            if *n >= max {
                return None;
            }
            *n += 1;
        }
        Some(Self {
            active: Arc::clone(active),
            tenant: tenant.to_string(),
        })
    }
}

impl Drop for TenantJobSlot {
    fn drop(&mut self) {
        self.active.remove_if_mut(&self.tenant, |_, n| {
            *n = n.saturating_sub(1);
            *n == 0
        });
    }
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
            tokens: Arc::new(DashMap::new()),
            runner,
            sink,
            semaphore: Arc::new(Semaphore::new(concurrency)),
            shard_rows: shard_rows.unwrap_or(planner::DEFAULT_SHARD_ROWS),
            active_jobs: Arc::new(DashMap::new()),
            max_jobs_per_tenant: DEFAULT_MAX_JOBS_PER_TENANT,
            output_ttl: cleanup.map(|c| c.output_ttl),
            job_budget: None,
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
                &controller.tokens,
                &controller.sink,
                cfg.output_ttl,
            );
            controller.spawn_cleanup(cfg);
        }
        controller
    }

    /// Sets the most export jobs one tenant may have queued or running at once
    /// (defaults to [`DEFAULT_MAX_JOBS_PER_TENANT`]). `submit` refuses a job
    /// beyond it with [`SubmitError::TenantJobLimit`].
    pub fn with_max_jobs_per_tenant(mut self, max: usize) -> Self {
        self.max_jobs_per_tenant = max;
        self
    }

    /// Sets each job's time budget (`HFS_EXPORT_JOB_TIMEOUT_SECS`; default
    /// `None`, no deadline).
    ///
    /// The budget starts when the job gets its concurrency permit, so time
    /// spent queued behind other jobs does not count, and it is not renewed
    /// per subject or page: every run of the job (view subjects and SQL Query
    /// dependencies alike) gets the same absolute deadline in its
    /// [`RunContext`], and the SQL Query engine's statements are capped by it.
    /// A job still running when it passes fails with `504` / `timeout`.
    pub fn with_job_budget(mut self, budget: Option<Duration>) -> Self {
        self.job_budget = budget;
        self
    }

    /// Spawns the background reaper. Holds only `Arc`/`Clone` handles so it is
    /// independent of the controller's own lifetime.
    fn spawn_cleanup(&self, cfg: CleanupConfig) {
        let jobs = Arc::clone(&self.jobs);
        let job_tenants = Arc::clone(&self.job_tenants);
        let tokens = Arc::clone(&self.tokens);
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
                reap_expired(&jobs, &job_tenants, &tokens, &sink, cfg.output_ttl);
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

    /// The handle of a known job, without a tenant check (callers check the
    /// tenant first). A job this process did not run (one rehydrated from its
    /// manifest) has no token of its own and is not `Running`, so its handle
    /// gets a token that has already fired.
    fn handle(&self, job_id: &str) -> Option<JobHandle> {
        if !self.jobs.contains_key(job_id) {
            return None;
        }
        let token = self
            .tokens
            .get(job_id)
            .map(|t| t.value().clone())
            .unwrap_or_else(|| {
                let token = CancellationToken::new();
                token.cancel();
                token
            });
        Some(JobHandle::new(
            Arc::clone(&self.jobs),
            job_id.to_string(),
            token,
        ))
    }
}

/// The failure of a job whose absolute `deadline` (its `budget`, counted
/// from its start) has passed: `504` / `timeout`. `None` while the job is
/// within its budget, or has none.
fn budget_passed(deadline: Option<Instant>, budget: Option<Duration>) -> Option<JobFailure> {
    let deadline = deadline?;
    (Instant::now() >= deadline).then(|| {
        JobFailure::from_sof(
            "export job",
            SofError::Timeout {
                budget,
                message: "the export job's budget passed before it completed".to_string(),
            },
        )
    })
}

impl<Sink: ExportSink + 'static> ExportJobController for InMemoryController<Sink> {
    fn submit(&self, task: ExportTask) -> Result<JobId, SubmitError> {
        // The place is held by the worker task until it ends, so a job that
        // is queued, running or cancelled-while-queued counts toward the limit.
        let Some(slot) = TenantJobSlot::acquire(
            &self.active_jobs,
            task.tenant.tenant_id().as_str(),
            self.max_jobs_per_tenant,
        ) else {
            return Err(SubmitError::TenantJobLimit {
                limit: self.max_jobs_per_tenant,
            });
        };

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

        // One token per job (see `JobHandle`): every transition out of
        // Running fires it, and every run of the job takes a child of it.
        let token = CancellationToken::new();
        self.tokens.insert(job_id.clone(), token.clone());
        let handle = JobHandle::new(Arc::clone(&self.jobs), job_id.clone(), token);

        // A runner that scans storage per view keeps what it read for this job.
        let runner = self
            .runner
            .for_export_job()
            .unwrap_or_else(|| Arc::clone(&self.runner));
        let sink = self.sink.clone();
        let semaphore = Arc::clone(&self.semaphore);
        let shard_rows = self.shard_rows;
        let job_budget = self.job_budget;

        tokio::spawn(async move {
            // Held until this task ends; `let _ = slot` would drop it at once.
            let _slot = slot;
            // Fires the job's token when this task ends for any reason (an
            // early return or a panic included), so nothing the job started
            // outlives it.
            let _stop_on_exit = handle.token().clone().drop_guard();
            let jid = handle.job_id().to_string();
            // Acquire concurrency permit (blocks if too many jobs running)
            let _permit = semaphore.acquire().await;
            // The job's budget starts now, not at submit: time spent queued
            // does not count. One absolute deadline for every run of the job.
            let deadline = job_budget.and_then(|budget| Instant::now().checked_add(budget));

            // One job, any mixture of subjects. Views run first, then
            // queries, and their outputs are concatenated into a single
            // manifest. Progress counts every subject so the X-Progress
            // percentage tracks real work across both halves.
            let total_subjects = task.work.subject_count().max(1) as u32;
            let outcome = async {
                // A job cancelled (or already reaped) while it waited for its
                // permit never starts (#1704).
                ensure_running(&handle)?;

                let (mut files, mut rows) = run_views_job(
                    &handle,
                    deadline,
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
                    &handle,
                    deadline,
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
            // The budget bounds the whole job, its output phase included: a
            // job whose formatting or last shard write took it past its
            // deadline fails with `504` / `timeout` (and its shards are
            // deleted below) instead of publishing a completion.
            let outcome = outcome.and_then(|done| match budget_passed(deadline, job_budget) {
                Some(failure) => Err(failure),
                None => Ok(done),
            });

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
                    if handle.is_running() {
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

                    // Persisting the manifest (a file write, or an S3
                    // `PutObject`) can itself take the job past its deadline:
                    // checked again right before `Completed` is published. A
                    // job past it fails with `504` / `timeout`, and the
                    // cleanup below deletes its output, the manifest just
                    // written included, so no restart rehydrates it.
                    match budget_passed(deadline, job_budget) {
                        Some(failure) => {
                            warn!(
                                job_id = %jid,
                                error = %failure.message,
                                "export job passed its budget while completing"
                            );
                            handle.fail(failure);
                        }
                        None => {
                            handle.complete(
                                completed_files,
                                completed_at,
                                task.format.clone(),
                                task.client_tracking_id.clone(),
                            );
                        }
                    }
                }
                // Whatever stopped the task (its token, a checkpoint, or an
                // error raised while it was being stopped) is not a failure to
                // report: the status is already Cancelled or gone.
                Err(failure) if !handle.is_running() => {
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
                    // fault's text stays in the log line above (#1703). The
                    // failure belongs to the subject in flight, which the
                    // worker recorded as `current_subject` before running it
                    // (#1800).
                    handle.fail(failure);
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
            if !matches!(handle.status(), Some(JobStatus::Completed { .. })) {
                if let Err(e) = sink.delete_job(&jid) {
                    warn!(job_id = %jid, error = %e, "failed to delete partial export output of unfinished job");
                }
            }
        });

        Ok(job_id)
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
        // The lock is held only for the state change; the token fires and the
        // deletion happens after.
        let Some(handle) = self.handle(job_id) else {
            return false;
        };
        let now_cancelled = handle.cancel();

        // Spec (operations-common, HL7/sql-on-fhir#365): SHOULD clean up partial
        // results on cancel. Drop any shards written so far. The cancel fired
        // the job's token, so its task stops at once, wherever it is (waiting
        // for its permit aside: a queued job only notices once it gets one,
        // #1798), frees its concurrency slot, then deletes whatever it wrote
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

    fn job_handle(&self, tenant_id: &str, job_id: &str) -> Option<JobHandle> {
        if !self.tenant_matches(tenant_id, job_id) {
            return None;
        }
        self.handle(job_id)
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
///
/// Removing a job fires its token (and forgets it). A terminal job's token has
/// already fired when it left `Running`; firing it again is a no-op, so whatever
/// still holds the job's handle sees it stopped either way.
fn reap_expired<Sink: ExportSink>(
    jobs: &DashMap<String, JobStatus>,
    job_tenants: &DashMap<String, String>,
    tokens: &DashMap<String, CancellationToken>,
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
                if let Some((_, token)) = tokens.remove(&jid) {
                    token.cancel();
                }
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

/// The worker's checkpoint: fails once the job is no longer `Running` (it was
/// cancelled, finished, or removed by the reaper), so the task stops at its
/// next opportunity instead of doing work nobody can reach. A cancel through
/// the handle also fires the job's token, which stops the task at once; this
/// check covers a status that changed without it (a reaped job).
fn ensure_running(handle: &JobHandle) -> Result<(), JobFailure> {
    if handle.is_running() {
        Ok(())
    } else {
        Err(JobFailure::stopped())
    }
}

/// The context of one run of a job: a child token of the job's token, so one
/// run's stop never stops the job, the job's deadline (its budget from the
/// permit, the same for every run; see
/// [`InMemoryController::with_job_budget`]), and the run kind.
fn job_run_context(handle: &JobHandle, deadline: Option<Instant>) -> RunContext {
    RunContext {
        cancel: handle.token().child_token(),
        deadline,
        kind: RunKind::ExportJob,
    }
}

/// Writes one shard of a view's rows: the #1704 checkpoint, the formatting and
/// the sink write, then records the file in `completed_files`.
#[allow(clippy::too_many_arguments)]
fn write_view_shard<Sink: ExportSink>(
    handle: &JobHandle,
    sink: &Sink,
    name: &str,
    rows: &[serde_json::Value],
    format: &str,
    header: bool,
    ext: &str,
    completed_files: &mut Vec<CompletedFile>,
) -> Result<(), JobFailure> {
    ensure_running(handle)?;

    let data = format_rows(rows, format, header).map_err(|e| format!("view '{name}': {e}"))?;
    store_view_shard(handle, sink, name, data, rows.len(), ext, completed_files)
}

/// Hands one encoded shard to the sink and records it in `completed_files`.
#[allow(clippy::too_many_arguments)]
fn store_view_shard<Sink: ExportSink>(
    handle: &JobHandle,
    sink: &Sink,
    name: &str,
    data: Vec<u8>,
    row_count: usize,
    ext: &str,
    completed_files: &mut Vec<CompletedFile>,
) -> Result<(), JobFailure> {
    ensure_running(handle)?;
    let jid = handle.job_id();

    // Shard files are numbered by a running index across the whole
    // job so every shard gets a unique filename; for the
    // single-view case this matches the historical `shard-{N}`
    // numbering exactly.
    let shard_key = completed_files.len();
    let filename = sink
        .write_shard(jid, shard_key, data, ext)
        .map_err(|e| format!("view '{name}': {e}"))?;

    debug!(job_id = %jid, view = %name, shard = shard_key, rows = row_count, file = %filename, "shard written");
    completed_files.push(CompletedFile {
        view_name: name.to_string(),
        filename,
        row_count,
    });
    Ok(())
}

/// Encodes one view's Parquet shards incrementally: rows go straight into an
/// open Parquet writer in batches, so a subject holds at most one batch of
/// rows plus the encoded bytes of the shard being built (#1812).
///
/// The Arrow schema is fixed once per view, before the first shard is
/// written, from the view's declared column types and the first
/// [`helios_sof::parquet_schema::SCHEMA_SAMPLE_ROWS`] rows of the whole view
/// (not of each shard), so every shard of a view has the same schema. Columns
/// and their order come from the first row's keys.
struct ParquetShardEncoder {
    view: serde_json::Value,
    shard_rows: usize,
    /// Rows held back until enough have arrived to fix the schema.
    head: Vec<serde_json::Value>,
    state: Option<ParquetShardState>,
}

/// An encoded shard and its row count.
type EncodedShard = (Vec<u8>, usize);

struct ParquetShardState {
    columns: Vec<String>,
    new_writer: Box<dyn Fn() -> Result<ParquetStreamWriter<Vec<u8>>, ExportError> + Send>,
    writer: Option<ParquetStreamWriter<Vec<u8>>>,
    pending: Vec<helios_sof::ProcessedRow>,
    rows_in_shard: usize,
}

impl ParquetShardState {
    fn new(
        view: &serde_json::Value,
        first_rows: &[serde_json::Value],
    ) -> Result<Self, ExportError> {
        let columns: Vec<String> = first_rows
            .first()
            .and_then(|r| r.as_object())
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        let sample: Vec<helios_sof::ProcessedRow> = first_rows
            .iter()
            .map(|r| to_processed_row(&columns, r))
            .collect();
        let declared = declared_columns(view);
        let schema = Arc::new(
            resolve_arrow_schema(&columns, &declared, &sample)
                .map_err(|e| ExportError::Serialization(e.to_string()))?,
        );
        let options = helios_sof::ParquetOptions::default();
        Ok(Self {
            columns,
            new_writer: Box::new(move || {
                ParquetStreamWriter::new(Vec::new(), schema.clone(), &options)
                    .map_err(|e| ExportError::Serialization(e.to_string()))
            }),
            writer: None,
            pending: Vec::new(),
            rows_in_shard: 0,
        })
    }

    fn flush_pending(&mut self) -> Result<(), ExportError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        if self.writer.is_none() {
            self.writer = Some((self.new_writer)()?);
        }
        if let Some(w) = self.writer.as_mut() {
            w.write_rows(&self.pending)
                .map_err(|e| ExportError::Serialization(e.to_string()))?;
        }
        self.pending.clear();
        Ok(())
    }

    fn finish_shard(&mut self) -> Result<Option<EncodedShard>, ExportError> {
        if self.rows_in_shard == 0 {
            return Ok(None);
        }
        self.flush_pending()?;
        let rows = std::mem::take(&mut self.rows_in_shard);
        let Some(w) = self.writer.take() else {
            return Ok(None);
        };
        let data = w
            .finish()
            .map_err(|e| ExportError::Serialization(e.to_string()))?;
        Ok(Some((data, rows)))
    }

    fn push(
        &mut self,
        row: &serde_json::Value,
        shard_rows: usize,
        out: &mut Vec<EncodedShard>,
    ) -> Result<(), ExportError> {
        self.pending.push(to_processed_row(&self.columns, row));
        self.rows_in_shard += 1;
        if self.pending.len() == STREAM_BATCH_ROWS {
            self.flush_pending()?;
        }
        if self.rows_in_shard == shard_rows {
            out.extend(self.finish_shard()?);
        }
        Ok(())
    }
}

fn to_processed_row(columns: &[String], row: &serde_json::Value) -> helios_sof::ProcessedRow {
    let values = columns
        .iter()
        .map(|col| row.as_object().and_then(|o| o.get(col)).cloned())
        .collect();
    helios_sof::ProcessedRow { values }
}

impl ParquetShardEncoder {
    fn new(view: &serde_json::Value, shard_rows: usize) -> Self {
        Self {
            view: view.clone(),
            shard_rows,
            head: Vec::new(),
            state: None,
        }
    }

    /// Adds one row; returns the shards completed by it (usually none).
    fn push(&mut self, row: serde_json::Value) -> Result<Vec<EncodedShard>, ExportError> {
        let mut out = Vec::new();
        if let Some(state) = self.state.as_mut() {
            state.push(&row, self.shard_rows, &mut out)?;
            return Ok(out);
        }
        self.head.push(row);
        if self.head.len() == SCHEMA_SAMPLE_ROWS {
            self.resolve(&mut out)?;
        }
        Ok(out)
    }

    /// Fixes the schema from the rows held so far and replays them.
    fn resolve(&mut self, out: &mut Vec<EncodedShard>) -> Result<(), ExportError> {
        let head = std::mem::take(&mut self.head);
        let mut state = ParquetShardState::new(&self.view, &head)?;
        for row in &head {
            state.push(row, self.shard_rows, out)?;
        }
        self.state = Some(state);
        Ok(())
    }

    /// Ends the view: returns the remaining shard, if any rows are left.
    fn finish(mut self) -> Result<Vec<EncodedShard>, ExportError> {
        let mut out = Vec::new();
        if self.state.is_none() && !self.head.is_empty() {
            self.resolve(&mut out)?;
        }
        if let Some(state) = self.state.as_mut() {
            out.extend(state.finish_shard()?);
        }
        Ok(out)
    }
}

/// The ViewDefinition half of an export job: run each named view through the
/// `SofRunner` and write its rows to output shards as the stream delivers
/// them, holding at most one shard's rows in memory. An error from a view's
/// row stream fails the whole job instead of being skipped, so a shard is
/// never published as a complete file when rows are actually missing. This
/// holds across subjects too: one failing view fails the job, the worker
/// deletes the shards of the subjects that already finished, and the other
/// subjects' outputs are not published (#1752 keeps this on purpose; reporting
/// failures per subject is a separate feature). Rows reach the shards in the
/// order the runner streams them; this function does not sort.
///
/// The start of each run and every row it reads are raced against the job's
/// token: a cancel ends the subject at once, drops the run's stream, and the
/// run's own child token has fired too, so the runner stops its database work
/// even while it has no row to hand over (#1864). At most the row already in
/// hand when the cancel arrives is still processed.
#[allow(clippy::too_many_arguments)]
async fn run_views_job<Sink: ExportSink>(
    handle: &JobHandle,
    deadline: Option<Instant>,
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
        ensure_running(handle)?;
        handle.record_progress(
            progress_offset + view_idx as u32,
            total_subjects,
            Some(&named.name),
        );

        let started = tokio::select! {
            biased;
            () = handle.token().cancelled() => return Err(JobFailure::stopped()),
            started = runner.run_view(
                &task.tenant,
                RunRequest::new(named.view.clone(), task.filters.clone()),
                job_run_context(handle, deadline),
            ) => started,
        };
        let stream = started
            .map(|output| output.rows)
            .map_err(|e| JobFailure::from_sof(&format!("view '{}'", named.name), e))?;

        // Rows are written as soon as `shard_rows` of them have arrived, so a
        // subject holds at most one shard's rows in memory (#1705). Cutting
        // every `shard_rows` rows and once at the end gives exactly the ranges
        // `planner::plan` gives over the whole result (and `shard_rows == 0`
        // gives one shard, as there), so shard boundaries, file names and
        // bytes are unchanged. A stream error after some shards were written
        // still fails the job, and the worker deletes them.
        let mut shard: Vec<serde_json::Value> = Vec::new();
        // Parquet shards are encoded as rows arrive instead of being buffered
        // as JSON values first (#1812).
        let mut parquet =
            (format == "parquet").then(|| ParquetShardEncoder::new(&named.view, shard_rows));
        let mut stream = stream;
        loop {
            let item = tokio::select! {
                biased;
                () = handle.token().cancelled() => return Err(JobFailure::stopped()),
                item = stream.next() => item,
            };
            let Some(item) = item else { break };
            match item {
                Ok(v) => {
                    if let Some(enc) = parquet.as_mut() {
                        let done = enc
                            .push(v)
                            .map_err(|e| format!("view '{}': {e}", named.name))?;
                        for (data, rows) in done {
                            store_view_shard(
                                handle,
                                sink,
                                &named.name,
                                data,
                                rows,
                                ext,
                                &mut completed_files,
                            )?;
                            total_rows += rows;
                        }
                        continue;
                    }
                    shard.push(v);
                    if shard.len() == shard_rows {
                        write_view_shard(
                            handle,
                            sink,
                            &named.name,
                            &shard,
                            &format,
                            task.header,
                            ext,
                            &mut completed_files,
                        )?;
                        total_rows += shard.len();
                        shard.clear();
                    }
                }
                Err(e) => {
                    if matches!(e, SofError::Cancelled) {
                        debug!(view = %named.name, "export row stream stopped: job is no longer running");
                    } else {
                        warn!(view = %named.name, error = %e, "export row stream failed");
                    }
                    // A mid-stream error keeps its kind, as one at the start
                    // of the run does: the same mapping decides the job's
                    // failure status (#1864).
                    return Err(JobFailure::from_sof(&format!("view '{}'", named.name), e));
                }
            }
        }

        if let Some(enc) = parquet {
            let done = enc
                .finish()
                .map_err(|e| format!("view '{}': {e}", named.name))?;
            for (data, rows) in done {
                store_view_shard(
                    handle,
                    sink,
                    &named.name,
                    data,
                    rows,
                    ext,
                    &mut completed_files,
                )?;
                total_rows += rows;
            }
        }

        // Spec: `output` is 0..*. Views with zero rows simply contribute no
        // `output` entries rather than emitting an empty shard with a
        // download URL pointing at zero bytes.
        if !shard.is_empty() {
            write_view_shard(
                handle,
                sink,
                &named.name,
                &shard,
                &format,
                task.header,
                ext,
                &mut completed_files,
            )?;
            total_rows += shard.len();
        }

        handle.record_progress(
            progress_offset + (view_idx as u32) + 1,
            total_subjects,
            None,
        );
    }

    Ok((completed_files, total_rows))
}

/// The SQLQuery / SQLView half of an export job: materialize each subject's
/// table sources via the `SofRunner`, execute the pre-validated SQL, and shard
/// the result rows into output files.
///
/// The plan runs under the job's token ([`execute_plan`]'s `ctx`): a cancel
/// stops the leaf runs, the rows being materialized and the engine's running
/// statement within the stop bound (#1864).
///
/// [`execute_plan`]: crate::handlers::sof::graph::execute_plan
#[allow(clippy::too_many_arguments)]
async fn run_sqlquery_job<Sink: ExportSink>(
    handle: &JobHandle,
    deadline: Option<Instant>,
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

    let jid = handle.job_id();
    for (query_idx, query) in queries.iter().enumerate() {
        ensure_running(handle)?;
        handle.record_progress(
            progress_offset + query_idx as u32,
            total_subjects,
            Some(&query.name),
        );

        // The plan's leaves run with the job's deadline (kind
        // `SqlQueryDependency`), and the engine's statements are capped by it
        // (D-5).
        let ctx = job_run_context(handle, deadline);
        let result = execute_sql_query(runner, task, query, limits, &ctx).await?;

        total_rows += result.rows.len();

        for range in planner::plan(result.rows.len(), shard_rows) {
            ensure_running(handle)?;
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

        handle.record_progress(
            progress_offset + (query_idx as u32) + 1,
            total_subjects,
            None,
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
///
/// A failure is answered as `$sql-run` answers it ([`JobFailure::from_sqlquery`]):
/// the request's own fault keeps its 4xx, an unavailable dependency its
/// `503` and a run past the job's deadline its `504`.
async fn execute_sql_query(
    runner: &Arc<dyn SofRunner>,
    task: &ExportTask,
    query: &NamedSqlQuery,
    limits: SqlExportLimits,
    ctx: &RunContext,
) -> Result<QueryResult, JobFailure> {
    let prefix = format!("query '{}'", query.name);
    let engine = InMemorySqlEngine::open()
        .map_err(|e| JobFailure::from_export(&prefix, ExportError::Runner(e.to_string())))?;
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
        ctx,
        exec_limits,
    )
    .await
    // A limit the request ran into, or a subject the engine refuses, is the
    // client's answer; the REST wording of a server fault hides the backend
    // detail, so that path keeps the engine's own text.
    .map_err(|e| JobFailure::from_sqlquery(&prefix, e))?;

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
        // Parquet shards are encoded incrementally by `ParquetShardEncoder`.
        "parquet" => Err(ExportError::Serialization(
            "parquet shards are encoded incrementally".to_string(),
        )),
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
    // Parquet takes its column types from the query result so every shard of
    // a query has the same schema.
    let declared = helios_sof::sqlquery::engine::declared_from_query_types(
        &result.columns,
        &result.column_types,
    );
    helios_sof::format_output_with_schema(processed, ct, None, &declared)
        .map_err(|e| ExportError::Serialization(e.to_string()))
}

/// Serialises rows as a single JSON array (`_format=json`).
fn format_json_array(rows: &[serde_json::Value]) -> Result<Vec<u8>, ExportError> {
    serde_json::to_vec(rows).map_err(|e| ExportError::Serialization(e.to_string()))
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
    use helios_persistence::core::sof_runner::{RunOutput, SofError, ViewFilters};
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
            _request: RunRequest,
            _ctx: RunContext,
        ) -> Result<RunOutput, SofError> {
            self.release.notified().await;
            Ok(RunOutput::from_rows(Box::pin(futures::stream::empty())))
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
        let job_id = controller
            .submit(ExportTask {
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
            })
            .expect("job accepted");

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
        let job_id = controller
            .submit(ExportTask {
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
            })
            .expect("job accepted");

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
            _request: RunRequest,
            _ctx: RunContext,
        ) -> Result<RunOutput, SofError> {
            let _ = self.called.send(());
            self.release.notified().await;
            Ok(RunOutput::from_rows(Box::pin(futures::stream::empty())))
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

        let job_id = controller
            .submit(ExportTask {
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
            })
            .expect("job accepted");

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
            _request: RunRequest,
            _ctx: RunContext,
        ) -> Result<RunOutput, SofError> {
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
            Ok(RunOutput::from_rows(Box::pin(futures::stream::poll_fn(
                move |cx| rx.poll_recv(cx),
            ))))
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

        let job_id = controller
            .submit(ExportTask {
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
            })
            .expect("job accepted");

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
            _request: RunRequest,
            _ctx: RunContext,
        ) -> Result<RunOutput, SofError> {
            let ok_rows = (0..200).map(|i| Ok(serde_json::json!({"id": format!("p{i}")})));
            let failure = std::iter::once(Err(SofError::backend(
                "canceling statement due to statement timeout".to_string(),
            )));
            Ok(RunOutput::from_rows(Box::pin(futures::stream::iter(
                ok_rows.chain(failure),
            ))))
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
            _request: RunRequest,
            _ctx: RunContext,
        ) -> Result<RunOutput, SofError> {
            Err(SofError::InvalidViewDefinition(
                "column 'city' declares `collection: false` but path 'address.city' may yield multiple values".to_string(),
            ))
        }

        fn runner_name(&self) -> &'static str {
            "refusing-test-runner"
        }
    }

    async fn terminal_status<S: ExportSink>(
        controller: &InMemoryController<S>,
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
        })
        .expect("job accepted");
        match terminal_status(&controller, &job_id).await {
            JobStatus::Failed {
                message,
                status,
                code,
                subject,
                ..
            } => {
                assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
                assert_eq!(code, "processing");
                assert!(
                    message.starts_with("view 'demo': column 'city'"),
                    "{message}"
                );
                assert_eq!(
                    subject.as_deref(),
                    Some("demo"),
                    "a client fault names its subject too"
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
            _request: RunRequest,
            _ctx: RunContext,
        ) -> Result<RunOutput, SofError> {
            Err(SofError::backend("connection reset by peer".to_string()))
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
        })
        .expect("job accepted");
        match terminal_status(&controller, &job_id).await {
            JobStatus::Failed {
                message,
                status,
                code,
                subject,
                ..
            } => {
                assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
                assert_eq!(code, "processing");
                assert_eq!(message, server_fault_message(&job_id));
                assert!(!message.contains("connection reset by peer"), "{message}");
                assert_eq!(
                    subject.as_deref(),
                    Some("demo"),
                    "a server fault still names the subject that failed (#1800)"
                );
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
        let job_id = controller
            .submit(ExportTask {
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
            })
            .expect("job accepted");
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
        let job_id = controller
            .submit(ExportTask {
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
            })
            .expect("job accepted");
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

        let job_id = controller
            .submit(ExportTask {
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
            })
            .expect("job accepted");

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
                message,
                status,
                subject,
                ..
            } => {
                assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
                assert_eq!(message, server_fault_message(&job_id));
                assert_eq!(subject.as_deref(), Some("families"));
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

        let job_id = controller
            .submit(ExportTask {
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
            })
            .expect("job accepted");

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
                message,
                status,
                subject,
                ..
            } => {
                assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
                assert_eq!(message, server_fault_message(&job_id));
                assert_eq!(subject.as_deref(), Some("patients"));
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
        reap_expired(
            &jobs,
            &job_tenants,
            &DashMap::new(),
            &sink,
            Duration::from_secs(3600),
        );

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
            request: RunRequest,
            _ctx: RunContext,
        ) -> Result<RunOutput, SofError> {
            let view_definition = request.view;
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
            Ok(RunOutput::from_rows(Box::pin(futures::stream::unfold(
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
            ))))
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

    /// The bytes a shard of `rows[range]` must have. Parquet shards share one
    /// schema per view, fixed from the whole view's first rows, and are
    /// written in `STREAM_BATCH_ROWS` batches from the start of the shard.
    fn oracle_shard_bytes(
        name: &str,
        rows: &[serde_json::Value],
        range: std::ops::Range<usize>,
        fmt: &str,
    ) -> Vec<u8> {
        if fmt != "parquet" {
            return format_rows(&rows[range], fmt, true).unwrap();
        }
        let state = ParquetShardState::new(
            &named_view_json(name),
            &rows[..rows.len().min(SCHEMA_SAMPLE_ROWS)],
        )
        .unwrap();
        let mut writer = (state.new_writer)().unwrap();
        for chunk in rows[range].chunks(STREAM_BATCH_ROWS) {
            let batch: Vec<_> = chunk
                .iter()
                .map(|r| to_processed_row(&state.columns, r))
                .collect();
            writer.write_rows(&batch).unwrap();
        }
        writer.finish().unwrap()
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

        let _a = controller.submit(view_task(&["a"])).expect("job accepted");
        assert_eq!(gate.reached().await, "a");

        // B parks on the semaphore behind A.
        let b = controller.submit(view_task(&["b"])).expect("job accepted");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(controller.cancel("t1", &b));

        gate.open();
        let c = controller.submit(view_task(&["c"])).expect("job accepted");
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

        let job_id = controller
            .submit(view_task(&["first", "second"]))
            .expect("job accepted");
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

        let job_id = controller
            .submit(view_task(&["patients"]))
            .expect("job accepted");
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

        let job_id = controller.submit(task).expect("job accepted");
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

        let _a = controller.submit(view_task(&["a"])).expect("job accepted");
        assert_eq!(gate.reached().await, "a");

        // B parks on the semaphore behind A.
        let b = controller.submit(view_task(&["b"])).expect("job accepted");
        tokio::time::sleep(Duration::from_millis(50)).await;
        // The reaper drops both entries while B waits.
        controller.jobs.remove(&b);
        controller.job_tenants.remove(&b);

        gate.open();
        let c = controller.submit(view_task(&["c"])).expect("job accepted");
        terminal_status(&controller, &c).await;

        assert_eq!(gate.seen(), vec!["a", "c"], "the reaped job must not run");
        assert!(controller.get_status("t1", &b).is_none());
    }

    /// Rows a cancelled job may still read after the cancel: the one already
    /// in hand when its token fires (#1704's bound, re-expressed in tokens:
    /// the job no longer polls its status every 4096 rows; D-3).
    const ROWS_AFTER_CANCEL: usize = 1;

    /// #1704: a view subject stops draining its row stream as soon as it is
    /// cancelled.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_running_view_stops_reading_rows_once_cancelled() {
        let total = 16_384;
        let mut gate = Gate::new(10, total, false);
        let controller = InMemoryController::new(
            gate.runner.clone(),
            InMemorySink::new("http://localhost"),
            None,
        );

        let job_id = controller
            .submit(view_task(&["patients"]))
            .expect("job accepted");
        gate.reached().await;
        assert!(controller.cancel("t1", &job_id));
        gate.open();
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;

        let produced = gate.produced.load(Ordering::SeqCst);
        assert!(
            produced <= 10 + ROWS_AFTER_CANCEL,
            "a cancelled job must stop reading rows, read {produced} of {total}"
        );
    }

    /// #1704: the same for a SQL subject, whose leaf rows are materialized
    /// into the in-memory engine by `execute_plan`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_running_sql_query_stops_reading_rows_once_cancelled() {
        let total = 16_384;
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

        let job_id = controller.submit(task).expect("job accepted");
        gate.reached().await;
        assert!(controller.cancel("t1", &job_id));
        gate.open();
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;

        let produced = gate.produced.load(Ordering::SeqCst);
        assert!(
            produced <= 10 + ROWS_AFTER_CANCEL,
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

        let job_id = controller
            .submit(view_task(&["patients"]))
            .expect("job accepted");
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

        reap_expired(
            &jobs,
            &job_tenants,
            &DashMap::new(),
            &sink,
            Duration::from_secs(3600),
        );

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
        reap_expired(
            &jobs,
            &job_tenants,
            &DashMap::new(),
            &sink,
            Duration::from_secs(3600),
        );

        assert_eq!(sink.deletes.load(Ordering::SeqCst), 2);
        assert!(!jobs.contains_key(&id), "the retry reclaims the job");
        assert!(sink.read_shard(&id, "shard-0.ndjson").is_none());
    }

    /// A tenant at its job limit is refused until one of its jobs' worker
    /// tasks ends. Queued, running and cancelled-while-queued jobs all count,
    /// and the limit is per tenant.
    #[tokio::test]
    async fn a_tenant_beyond_its_job_limit_is_refused_until_one_of_its_jobs_ends() {
        let mut gate = Gate::new(0, 0, false);
        let controller = InMemoryController::new(
            gate.runner.clone(),
            InMemorySink::new("http://localhost"),
            Some(1),
        )
        .with_max_jobs_per_tenant(2);

        let _a = controller.submit(view_task(&["a"])).expect("job accepted");
        let b = controller.submit(view_task(&["b"])).expect("job accepted");
        // A holds the only permit; B is queued behind it.
        assert_eq!(gate.reached().await, "a");

        assert!(matches!(
            controller.submit(view_task(&["c"])),
            Err(SubmitError::TenantJobLimit { limit: 2 })
        ));

        // The limit is per tenant.
        let mut other = view_task(&["x"]);
        other.tenant = TenantContext::new(TenantId::new("t2"), TenantPermissions::full_access());
        controller
            .submit(other)
            .expect("another tenant is unaffected");

        // A cancelled job keeps its place until its task has ended.
        assert!(controller.cancel("t1", &b));
        assert!(matches!(
            controller.submit(view_task(&["d"])),
            Err(SubmitError::TenantJobLimit { limit: 2 })
        ));

        gate.open();
        tokio::time::timeout(Duration::from_secs(15), async {
            while controller.active_jobs.get("t1").is_some() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("both of the tenant's jobs must end");

        controller
            .submit(view_task(&["e"]))
            .expect("a place is free once the tenant's jobs have ended");
    }

    /// A limit of zero refuses every submit and must not record the tenant.
    #[tokio::test]
    async fn a_zero_job_limit_refuses_without_recording_the_tenant() {
        let controller = InMemoryController::new(
            Arc::new(FailingRunner),
            InMemorySink::new("http://localhost"),
            None,
        )
        .with_max_jobs_per_tenant(0);

        assert!(matches!(
            controller.submit(view_task(&["a"])),
            Err(SubmitError::TenantJobLimit { limit: 0 })
        ));
        assert!(controller.active_jobs.is_empty());
    }

    /// #1705: a view's rows are written as each shard fills, not after the
    /// whole result has been read, so a subject holds at most one shard.
    #[tokio::test]
    async fn a_view_shard_is_written_before_its_row_stream_ends() {
        let mut gate = Gate::new(2, 5, false);
        let sink = ScriptedSink::new();
        let controller =
            InMemoryController::with_shard_rows(gate.runner.clone(), sink.clone(), None, Some(2));

        let job_id = controller
            .submit(view_task(&["patients"]))
            .expect("job accepted");
        // The stream has yielded two rows and is parked.
        assert_eq!(gate.reached().await, "patients");

        // Before #1705 this timed out: nothing was written until the stream
        // ended.
        wait_for_writes(&sink, 1).await;
        assert_eq!(sink.writes(), 1);
        gate.open();

        match terminal_status(&controller, &job_id).await {
            JobStatus::Completed { files, .. } => {
                let got: Vec<(String, usize)> = files
                    .iter()
                    .map(|f| (f.filename.clone(), f.row_count))
                    .collect();
                assert_eq!(
                    got,
                    vec![
                        ("shard-0.ndjson".to_string(), 2),
                        ("shard-1.ndjson".to_string(), 2),
                        ("shard-2.ndjson".to_string(), 1),
                    ]
                );
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    /// A [`SofRunner`] that streams fixed rows, keyed by the view's `"name"`.
    struct FixedRowsRunner(std::collections::HashMap<String, Vec<serde_json::Value>>);

    #[async_trait]
    impl SofRunner for FixedRowsRunner {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            request: RunRequest,
            _ctx: RunContext,
        ) -> Result<RunOutput, SofError> {
            let view_definition = request.view;
            let rows = view_definition["name"]
                .as_str()
                .and_then(|n| self.0.get(n))
                .cloned()
                .unwrap_or_default();
            Ok(RunOutput::from_rows(Box::pin(futures::stream::iter(
                rows.into_iter().map(Ok),
            ))))
        }

        fn runner_name(&self) -> &'static str {
            "fixed-rows-test-runner"
        }
    }

    /// #1705: streaming the rows into shards writes exactly the files, row
    /// counts and bytes that sharding the whole result with `planner::plan`
    /// did, for every format and shard size.
    #[tokio::test]
    async fn streamed_view_shards_match_the_planned_shards_byte_for_byte() {
        let make_rows = |n: usize| -> Vec<serde_json::Value> {
            (0..n)
                .map(|i| {
                    serde_json::json!({
                        "id": format!("p{i}"),
                        "n": i,
                        "flag": i % 2 == 0,
                        "note": format!("note {i}"),
                    })
                })
                .collect()
        };
        let fixture: Vec<(&str, Vec<serde_json::Value>)> = vec![
            ("a", make_rows(5)),
            ("b", make_rows(0)),
            ("c", make_rows(4)),
        ];
        let runner: Arc<dyn SofRunner> = Arc::new(FixedRowsRunner(
            fixture
                .iter()
                .map(|(n, r)| (n.to_string(), r.clone()))
                .collect(),
        ));

        for fmt in ["ndjson", "csv", "json", "parquet"] {
            for shard_rows in [0usize, 1, 2, 3, 5, 7] {
                let sink = InMemorySink::new("http://localhost");
                let controller = InMemoryController::with_shard_rows(
                    Arc::clone(&runner),
                    sink.clone(),
                    None,
                    Some(shard_rows),
                );
                let mut task = view_task(&["a", "b", "c"]);
                task.format = fmt.to_string();
                task.header = true;
                let job_id = controller.submit(task).expect("job accepted");
                let files = match terminal_status(&controller, &job_id).await {
                    JobStatus::Completed { files, .. } => files,
                    other => panic!("{fmt}/{shard_rows}: expected Completed, got {other:?}"),
                };

                // What the job wrote before #1705: plan the whole result.
                let mut expected: Vec<(String, String, usize, Vec<u8>)> = Vec::new();
                for (name, rows) in &fixture {
                    for range in planner::plan(rows.len(), shard_rows) {
                        let k = expected.len();
                        expected.push((
                            name.to_string(),
                            format!("shard-{k}.{}", ext_for(fmt)),
                            range.len(),
                            oracle_shard_bytes(name, rows, range, fmt),
                        ));
                    }
                }

                let got: Vec<(String, String, usize)> = files
                    .iter()
                    .map(|f| (f.view_name.clone(), f.filename.clone(), f.row_count))
                    .collect();
                let want: Vec<(String, String, usize)> = expected
                    .iter()
                    .map(|(v, f, n, _)| (v.clone(), f.clone(), *n))
                    .collect();
                assert_eq!(got, want, "{fmt}/{shard_rows}: files");
                for (_, filename, _, bytes) in &expected {
                    assert_eq!(
                        sink.read_shard(&job_id, filename).as_deref(),
                        Some(bytes.as_slice()),
                        "{fmt}/{shard_rows}: bytes of {filename}"
                    );
                }
            }
        }

        // One literal case. Key order depends on serde_json's `preserve_order`
        // feature (on in this build through unification), so compare with
        // `format_ndjson` of the first two rows rather than a literal string.
        let sink = InMemorySink::new("http://localhost");
        let controller =
            InMemoryController::with_shard_rows(Arc::clone(&runner), sink.clone(), None, Some(2));
        let job_id = controller.submit(view_task(&["a"])).expect("job accepted");
        terminal_status(&controller, &job_id).await;
        let first = sink.read_shard(&job_id, "shard-0.ndjson").unwrap();
        assert_eq!(first, format_ndjson(&fixture[0].1[..2]).unwrap());
        assert!(
            first.starts_with(b"{\"") && first.ends_with(b"}\n"),
            "unexpected shard-0: {}",
            String::from_utf8_lossy(&first)
        );
    }
    /// #1705: a view stream that fails after some shards were written fails the
    /// job and removes those shards.
    #[tokio::test]
    async fn a_view_stream_error_after_written_shards_fails_the_job_and_deletes_them() {
        let sink = ScriptedSink::new();
        let controller = InMemoryController::with_shard_rows(
            Arc::new(FailingRunner),
            sink.clone(),
            None,
            Some(2),
        );

        let job_id = controller
            .submit(view_task(&["patients"]))
            .expect("job accepted");

        match terminal_status(&controller, &job_id).await {
            // The view name and backend cause stay in the server log (#1703).
            JobStatus::Failed { message, .. } => {
                assert_eq!(message, server_fault_message(&job_id));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        // The worker deletes after it sets the status.
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;

        assert_eq!(sink.writes(), 100, "shards were written before the error");
        assert!(sink.deletes.load(Ordering::SeqCst) >= 1);
        assert!(sink.read_shard(&job_id, "shard-0.ndjson").is_none());
        assert!(sink.read_shard(&job_id, "shard-99.ndjson").is_none());
        assert!(matches!(
            controller.get_status("t1", &job_id),
            Some(JobStatus::Failed { .. })
        ));
    }

    /// A runner that streams `rows` rows for every view except the one named
    /// `failing`, whose stream yields `rows` rows and then a backend error.
    struct FailsOneViewRunner {
        failing: &'static str,
        rows: usize,
    }

    #[async_trait]
    impl SofRunner for FailsOneViewRunner {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            request: RunRequest,
            _ctx: RunContext,
        ) -> Result<RunOutput, SofError> {
            let view_definition = request.view;
            let fails = view_definition["name"].as_str() == Some(self.failing);
            let ok_rows = (0..self.rows).map(|i| Ok(serde_json::json!({"id": format!("p{i}")})));
            let tail = fails.then(|| {
                Err(SofError::backend(
                    "canceling statement due to statement timeout".to_string(),
                ))
            });
            Ok(RunOutput::from_rows(Box::pin(futures::stream::iter(
                ok_rows.chain(tail),
            ))))
        }

        fn runner_name(&self) -> &'static str {
            "fails-one-view-test-runner"
        }
    }

    /// #1752: an export fails as a whole when one of several subjects fails.
    /// Subject `a` finishes and writes its shards, subject `b` fails halfway
    /// through its stream, and the job ends `Failed` naming `b` with every
    /// shard of the job, `a`'s included, deleted. A per-subject partial
    /// success would need a new terminal state and a manifest error list, so
    /// this pins the fail-fast behaviour until that is built on purpose.
    #[tokio::test]
    async fn a_failing_subject_fails_the_whole_job_and_deletes_earlier_subjects_shards() {
        let sink = ScriptedSink::new();
        let controller = InMemoryController::with_shard_rows(
            Arc::new(FailsOneViewRunner {
                failing: "b",
                rows: 4,
            }),
            sink.clone(),
            None,
            Some(2),
        );

        let job_id = controller
            .submit(view_task(&["a", "b", "c"]))
            .expect("job accepted");

        match terminal_status(&controller, &job_id).await {
            JobStatus::Failed {
                message,
                status,
                subject,
                ..
            } => {
                assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
                assert_eq!(message, server_fault_message(&job_id));
                assert_eq!(subject.as_deref(), Some("b"));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        // The worker deletes after it sets the status.
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;

        // `a` wrote two shards of two rows, `b` wrote two more before its
        // stream failed, and `c` never started.
        assert_eq!(sink.writes(), 4, "a and b wrote shards, c never started");
        assert!(sink.deletes.load(Ordering::SeqCst) >= 1);
        for k in 0..4 {
            assert!(
                sink.read_shard(&job_id, &format!("shard-{k}.ndjson"))
                    .is_none(),
                "shard-{k} of the failed job must be deleted"
            );
        }
        assert!(matches!(
            controller.get_status("t1", &job_id),
            Some(JobStatus::Failed { .. })
        ));
    }

    /// A runner whose start never completes, like one listing every key of a
    /// large type before its first row.
    struct NeverStarts;

    #[async_trait]
    impl SofRunner for NeverStarts {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            _request: RunRequest,
            _ctx: RunContext,
        ) -> Result<RunOutput, SofError> {
            std::future::pending().await
        }

        fn runner_name(&self) -> &'static str {
            "never-starts"
        }
    }

    /// A runner that starts but never yields a row, like a view that filters
    /// out every resource of a large type. It ignores its context, and keeps
    /// the run's cancel token so a test can see it fire.
    #[derive(Default)]
    struct QuietRows {
        cancel: std::sync::Mutex<Option<CancellationToken>>,
    }

    #[async_trait]
    impl SofRunner for QuietRows {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            _request: RunRequest,
            ctx: RunContext,
        ) -> Result<RunOutput, SofError> {
            *self.cancel.lock().unwrap() = Some(ctx.cancel);
            Ok(RunOutput::from_rows(Box::pin(futures::stream::pending())))
        }

        fn runner_name(&self) -> &'static str {
            "quiet-rows"
        }
    }

    /// #1823, on the handle: a cancel while the runner is still starting
    /// stops the job at once, instead of waiting for a first row that may be
    /// minutes away. The runner here ignores its context, so it is the job's
    /// own race of `run_view` against its token that stops it.
    #[tokio::test]
    async fn a_cancel_while_the_runner_is_starting_stops_it() {
        let controller = InMemoryController::new(
            Arc::new(NeverStarts),
            InMemorySink::new("http://localhost"),
            None,
        );
        let job_id = controller
            .submit(view_task(&["patients"]))
            .expect("job accepted");
        tokio::time::sleep(Duration::from_millis(100)).await;
        let handle = controller.job_handle("t1", &job_id).expect("own job");
        assert!(handle.is_running(), "the runner is still starting");

        let started = tokio::time::Instant::now();
        assert!(handle.cancel());
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;
        let took = started.elapsed();
        assert!(took < Duration::from_millis(500), "stopping took {took:?}");
        assert!(matches!(
            controller.get_status("t1", &job_id),
            Some(JobStatus::Cancelled { .. })
        ));
    }

    /// #1823, on the handle: a row stream that yields nothing still ends soon
    /// after the job is cancelled, and the run's own cancel has fired.
    #[tokio::test]
    async fn a_quiet_row_stream_ends_cancelled_after_a_cancel() {
        let quiet = Arc::new(QuietRows::default());
        let controller =
            InMemoryController::new(quiet.clone(), InMemorySink::new("http://localhost"), None);
        let job_id = controller
            .submit(view_task(&["patients"]))
            .expect("job accepted");
        let run_cancel = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(cancel) = quiet.cancel.lock().unwrap().clone() {
                    return cancel;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the view's run starts");
        assert!(!run_cancel.is_cancelled());

        let started = tokio::time::Instant::now();
        assert!(controller.cancel("t1", &job_id));
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;
        let took = started.elapsed();
        assert!(took < Duration::from_millis(500), "stopping took {took:?}");
        assert!(
            run_cancel.is_cancelled(),
            "the run's own cancel fires with the job's"
        );
        assert!(matches!(
            controller.get_status("t1", &job_id),
            Some(JobStatus::Cancelled { .. })
        ));
    }

    // ------------------------------------------------------------------
    // The job handle (#1864, A13): one token per job, fired on every
    // transition out of Running; tenant-checked access.
    // ------------------------------------------------------------------

    /// A runner whose run yields `rows` rows, then the error `fail` builds (if
    /// any), and otherwise ends.
    struct ScriptedRows {
        rows: usize,
        fail: Option<fn() -> SofError>,
    }

    #[async_trait]
    impl SofRunner for ScriptedRows {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            _request: RunRequest,
            _ctx: RunContext,
        ) -> Result<RunOutput, SofError> {
            let mut rows: Vec<Result<serde_json::Value, SofError>> = (0..self.rows)
                .map(|i| Ok(serde_json::json!({"id": format!("p{i}")})))
                .collect();
            if let Some(fail) = self.fail {
                rows.push(Err(fail()));
            }
            Ok(RunOutput::from_rows(Box::pin(futures::stream::iter(rows))))
        }

        fn runner_name(&self) -> &'static str {
            "scripted-rows"
        }
    }

    /// Every way a job leaves `Running` fires its token: a cancel, a failure
    /// and a completion. A progress update does not.
    #[tokio::test]
    async fn every_transition_out_of_running_fires_the_token() {
        // Cancel: the job is held at its runner's start.
        let controller = InMemoryController::new(
            Arc::new(NeverStarts),
            InMemorySink::new("http://localhost"),
            None,
        );
        let job_id = controller
            .submit(view_task(&["patients"]))
            .expect("job accepted");
        let handle = controller.job_handle("t1", &job_id).expect("own job");
        tokio::time::sleep(Duration::from_millis(50)).await;
        handle.record_progress(0, 1, Some("patients"));
        assert!(
            !handle.token().is_cancelled(),
            "progress keeps the job running"
        );
        assert!(controller.cancel("t1", &job_id));
        assert!(handle.token().is_cancelled(), "cancel fires the token");
        assert!(!handle.cancel(), "a second cancel moves nothing");

        // Failure and completion: the worker's own transitions.
        for (name, runner, expect_failed) in [
            (
                "failure",
                ScriptedRows {
                    rows: 1,
                    fail: Some(|| SofError::backend("broken")),
                },
                true,
            ),
            (
                "completion",
                ScriptedRows {
                    rows: 3,
                    fail: None,
                },
                false,
            ),
        ] {
            let controller = InMemoryController::new(
                Arc::new(runner),
                InMemorySink::new("http://localhost"),
                None,
            );
            let job_id = controller
                .submit(view_task(&["patients"]))
                .expect("job accepted");
            let handle = controller.job_handle("t1", &job_id).expect("own job");
            let status = terminal_status(&controller, &job_id).await;
            assert_eq!(
                matches!(status, JobStatus::Failed { .. }),
                expect_failed,
                "{name}: {status:?}"
            );
            assert!(
                matches!(
                    status,
                    JobStatus::Failed { .. } | JobStatus::Completed { .. }
                ),
                "{name}: {status:?}"
            );
            assert!(handle.token().is_cancelled(), "{name} fires the token");
        }
    }

    /// The reaper only removes terminal jobs (`reap_expired` never touches a
    /// `Running` one), whose token has already fired; removing the job fires
    /// it again, which is idempotent, and forgets it.
    #[tokio::test]
    async fn reaping_a_terminal_job_fires_its_token_idempotently() {
        let sink = InMemorySink::new("http://localhost");
        let controller = InMemoryController::new(Arc::new(NeverStarts), sink.clone(), None);
        let job_id = controller
            .submit(view_task(&["patients"]))
            .expect("job accepted");
        let handle = controller.job_handle("t1", &job_id).expect("own job");
        assert!(handle.cancel());
        assert!(handle.token().is_cancelled());
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;

        // A Running job is never reaped, whatever its age.
        let running_jobs: DashMap<String, JobStatus> = DashMap::new();
        let running_tokens: DashMap<String, CancellationToken> = DashMap::new();
        let running_token = CancellationToken::new();
        running_jobs.insert(
            "running".to_string(),
            JobStatus::Running {
                subjects_done: 0,
                subjects_total: 1,
                current_subject: None,
                submitted_at: Utc::now() - chrono::Duration::hours(2),
            },
        );
        running_tokens.insert("running".to_string(), running_token.clone());
        reap_expired(
            &running_jobs,
            &DashMap::new(),
            &running_tokens,
            &sink,
            Duration::ZERO,
        );
        assert!(running_jobs.contains_key("running"));
        assert!(!running_token.is_cancelled());

        // The cancelled job, aged past a zero TTL, is reaped: its token (fired
        // already) is fired again and forgotten.
        tokio::time::sleep(Duration::from_millis(5)).await;
        reap_expired(
            &controller.jobs,
            &controller.job_tenants,
            &controller.tokens,
            &controller.sink,
            Duration::ZERO,
        );
        assert!(controller.jobs.get(&job_id).is_none(), "the job is reaped");
        assert!(
            controller.tokens.get(&job_id).is_none(),
            "its token is forgotten"
        );
        assert!(handle.token().is_cancelled());
        assert!(!handle.is_running());
        assert!(!handle.cancel(), "a reaped job cannot be cancelled again");
    }

    /// Ported from f91f5164c: the reaper removing a job fires its token.
    #[test]
    fn reap_expired_fires_the_reaped_jobs_token() {
        let sink = InMemorySink::new("http://localhost");
        let jobs: DashMap<String, JobStatus> = DashMap::new();
        let job_tenants: DashMap<String, String> = DashMap::new();
        let tokens: DashMap<String, CancellationToken> = DashMap::new();
        let token = CancellationToken::new();
        let two_hours_ago = Utc::now() - chrono::Duration::hours(2);
        jobs.insert(
            "old".to_string(),
            JobStatus::Cancelled {
                cancelled_at: two_hours_ago,
            },
        );
        tokens.insert("old".to_string(), token.clone());

        reap_expired(
            &jobs,
            &job_tenants,
            &tokens,
            &sink,
            Duration::from_secs(3600),
        );

        assert!(token.is_cancelled());
        assert!(tokens.is_empty());
    }

    /// The handle accessor follows the trait's tenant contract: another
    /// tenant, or an unknown job, gets `None`, exactly as for status, cancel
    /// and download.
    #[tokio::test]
    async fn job_handle_is_tenant_checked() {
        let controller = InMemoryController::new(
            Arc::new(NeverStarts),
            InMemorySink::new("http://localhost"),
            None,
        );
        let job_id = controller
            .submit(view_task(&["patients"]))
            .expect("job accepted");

        assert!(controller.job_handle("other-tenant", &job_id).is_none());
        assert!(controller.job_handle("t1", "no-such-job").is_none());
        let handle = controller.job_handle("t1", &job_id).expect("own job");
        assert_eq!(handle.job_id(), job_id);
        assert!(handle.is_running());
        assert!(controller.get_status("other-tenant", &job_id).is_none());

        assert!(handle.cancel());
        assert!(matches!(
            controller.get_status("t1", &job_id),
            Some(JobStatus::Cancelled { .. })
        ));
    }

    // ------------------------------------------------------------------
    // Prompt cancellation (ported from f91f5164c): a fired token stops a
    // job that is waiting on its runner, without waiting for the next row.
    // ------------------------------------------------------------------

    /// Sets its flag when dropped; moved into a row stream, it tells a test
    /// that the stream itself (and with it a real runner's channel) is gone.
    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    /// A `SofRunner` whose stream yields `rows` rows and then never another
    /// one nor its end — a PostgreSQL statement still sorting, say. With
    /// `hang_in_run_view` it never even returns its stream.
    struct HangingRunner {
        rows: usize,
        hang_in_run_view: bool,
        called: tokio::sync::mpsc::UnboundedSender<()>,
        dropped: Arc<AtomicBool>,
    }

    #[async_trait]
    impl SofRunner for HangingRunner {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            _request: RunRequest,
            _ctx: RunContext,
        ) -> Result<RunOutput, SofError> {
            let _ = self.called.send(());
            if self.hang_in_run_view {
                std::future::pending::<()>().await;
            }
            let guard = DropFlag(Arc::clone(&self.dropped));
            let rows = (0..self.rows).map(|i| Ok(serde_json::json!({"id": format!("p{i}")})));
            Ok(RunOutput::from_rows(Box::pin(
                futures::stream::iter(rows)
                    .chain(futures::stream::pending())
                    .map(move |row| {
                        let _ = &guard;
                        row
                    }),
            )))
        }

        fn runner_name(&self) -> &'static str {
            "hanging-test-runner"
        }
    }

    struct Hanging {
        runner: Arc<HangingRunner>,
        called: tokio::sync::mpsc::UnboundedReceiver<()>,
        dropped: Arc<AtomicBool>,
    }

    impl Hanging {
        fn new(rows: usize, hang_in_run_view: bool) -> Self {
            let (called_tx, called) = tokio::sync::mpsc::unbounded_channel();
            let dropped = Arc::new(AtomicBool::new(false));
            Self {
                runner: Arc::new(HangingRunner {
                    rows,
                    hang_in_run_view,
                    called: called_tx,
                    dropped: Arc::clone(&dropped),
                }),
                called,
                dropped,
            }
        }
    }

    /// Cancels `job_id` and returns how long its task took to stop and free
    /// its slot.
    async fn cancel_and_time<S: ExportSink>(
        controller: &InMemoryController<S>,
        job_id: &str,
    ) -> Duration {
        let started = tokio::time::Instant::now();
        assert!(controller.cancel("t1", job_id));
        wait_until_idle(controller, DEFAULT_MAX_CONCURRENCY).await;
        started.elapsed()
    }

    fn sql_query_task(max_rows: usize) -> ExportTask {
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
                max_source_rows_per_vd: max_rows,
                max_rows,
                timeout_secs: 5,
            },
        };
        task
    }

    /// A view whose runner has written shards and then produces nothing more
    /// stops as soon as it is cancelled: the job ends Cancelled, its output
    /// is deleted, and the runner's stream is dropped.
    #[tokio::test]
    async fn a_view_waiting_on_its_runner_stops_promptly_once_cancelled() {
        let mut hanging = Hanging::new(3, false);
        let sink = ScriptedSink::new();
        let controller = InMemoryController::with_shard_rows(
            hanging.runner.clone(),
            sink.clone(),
            None,
            Some(1),
        );

        let job_id = controller
            .submit(view_task(&["patients"]))
            .expect("job accepted");
        hanging.called.recv().await.expect("the view runs");
        wait_for_writes(&sink, 3).await;
        assert!(sink.read_shard(&job_id, "shard-2.ndjson").is_some());

        let took = cancel_and_time(&controller, &job_id).await;
        assert!(took < Duration::from_millis(500), "stopping took {took:?}");
        assert!(
            hanging.dropped.load(Ordering::SeqCst),
            "the row stream is dropped"
        );
        assert!(matches!(
            controller.get_status("t1", &job_id),
            Some(JobStatus::Cancelled { .. })
        ));
        for shard in 0..3 {
            assert!(
                sink.read_shard(&job_id, &format!("shard-{shard}.ndjson"))
                    .is_none()
            );
        }
    }

    /// The same before the runner has handed out its stream at all.
    #[tokio::test]
    async fn a_view_whose_runner_never_returns_stops_promptly_once_cancelled() {
        let mut hanging = Hanging::new(0, true);
        let controller = InMemoryController::new(
            hanging.runner.clone(),
            InMemorySink::new("http://localhost"),
            None,
        );

        let job_id = controller
            .submit(view_task(&["patients"]))
            .expect("job accepted");
        hanging.called.recv().await.expect("the view runs");

        let took = cancel_and_time(&controller, &job_id).await;
        assert!(took < Duration::from_millis(500), "stopping took {took:?}");
        assert!(matches!(
            controller.get_status("t1", &job_id),
            Some(JobStatus::Cancelled { .. })
        ));
    }

    /// A SQL subject materializing a leaf ViewDefinition whose runner has
    /// gone quiet stops as promptly: the leaf's rows are raced against the
    /// same token inside `execute_plan`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_sql_query_waiting_on_a_leaf_stops_promptly_once_cancelled() {
        let mut hanging = Hanging::new(5, false);
        let controller = InMemoryController::new(
            hanging.runner.clone(),
            InMemorySink::new("http://localhost"),
            None,
        );

        let job_id = controller
            .submit(sql_query_task(1000))
            .expect("job accepted");
        hanging.called.recv().await.expect("the leaf runs");
        tokio::time::sleep(Duration::from_millis(20)).await;

        let took = cancel_and_time(&controller, &job_id).await;
        assert!(took < Duration::from_millis(500), "stopping took {took:?}");
        assert!(
            hanging.dropped.load(Ordering::SeqCst),
            "the leaf's row stream is dropped"
        );
        assert!(matches!(
            controller.get_status("t1", &job_id),
            Some(JobStatus::Cancelled { .. })
        ));
    }

    // ------------------------------------------------------------------
    // A12 (export half): a scan that panics part way fails the job.
    // ------------------------------------------------------------------

    /// A `ResourceScan` whose stream yields `ok_items` Patients and then
    /// panics while the in-process runner reads it.
    struct PanickingScan {
        ok_items: usize,
    }

    #[async_trait]
    impl helios_persistence::sof::in_process::ResourceScan for PanickingScan {
        async fn scan_resources(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
        ) -> Result<helios_persistence::sof::in_process::ResourceStream, SofError> {
            let ok_items = self.ok_items;
            Ok(Box::pin(futures::stream::iter(0..).map(move |i| {
                assert!(
                    i < ok_items,
                    "scan double: storage failed after {ok_items} items"
                );
                Ok(serde_json::json!({"resourceType": "Patient", "id": format!("p{i}")}))
            })))
        }

        async fn read_resources(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _ids: &[String],
        ) -> Result<Vec<serde_json::Value>, SofError> {
            Ok(Vec::new())
        }
    }

    /// #1864 (A12, export half): an export over the in-process runner whose
    /// scan panics after some shards were written fails the job (a server
    /// fault, never a truncated `Completed`) and deletes the shards it wrote.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_export_over_a_panicking_scan_fails_and_deletes_its_shards() {
        // More than one in-process batch (1024 resources), so rows and shards
        // exist before the panic.
        let ok_items = 1500;
        let runner = Arc::new(
            helios_persistence::sof::in_process::InProcessSofRunner::new(
                Arc::new(PanickingScan { ok_items }),
                helios_fhir::FhirVersion::R4,
                "panicking-scan",
            ),
        );
        let sink = ScriptedSink::new();
        let controller = InMemoryController::with_shard_rows(runner, sink.clone(), None, Some(100));

        let job_id = controller
            .submit(view_task(&["patients"]))
            .expect("job accepted");
        let status = tokio::time::timeout(
            Duration::from_secs(30),
            terminal_status(&controller, &job_id),
        )
        .await
        .expect("the job ends");
        match &status {
            JobStatus::Failed { status, .. } => {
                assert_eq!(*status, StatusCode::INTERNAL_SERVER_ERROR, "{status:?}")
            }
            other => panic!("a panicking scan must fail the job, got {other:?}"),
        }
        assert!(
            sink.writes() > 0,
            "the job wrote shards before the scan panicked"
        );
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;
        for shard in 0..sink.writes() {
            assert!(
                sink.read_shard(&job_id, &format!("shard-{shard}.ndjson"))
                    .is_none(),
                "shard-{shard} of the failed job must be deleted"
            );
        }
    }

    /// #1864 (B11, A14): cancelling a SQL Query subject whose own SELECT is
    /// already executing stops that statement within the stop bound (1 s),
    /// not when the `HFS_SOF_SQLQUERY_TIMEOUT_SECS` watchdog fires.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancel_stops_a_running_sql_query_subject_statement_within_the_bound() {
        const WATCHDOG_SECS: u64 = 5;
        // One leaf row, held at the gate until the test lets the leaf finish.
        let mut gate = Gate::new(1, 1, false);
        let controller = InMemoryController::new(
            gate.runner.clone(),
            InMemorySink::new("http://localhost"),
            None,
        );

        let mut task = view_task(&[]);
        task.work = ExportWork {
            views: vec![],
            queries: vec![NamedSqlQuery {
                name: "slow".to_string(),
                // Far more work than the watchdog allows: only a stop ends it.
                sql: "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c \
                      WHERE x < 4000000000) SELECT count(*) AS n FROM c, vd_0"
                    .to_string(),
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
                timeout_secs: WATCHDOG_SECS,
            },
        };

        let job_id = controller.submit(task).expect("job accepted");
        gate.reached().await;
        // The leaf ends; its row is materialized and the subject SELECT starts.
        gate.open();
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(
            controller.semaphore.available_permits() < DEFAULT_MAX_CONCURRENCY,
            "the subject statement must still be running when the cancel arrives"
        );

        assert!(controller.cancel("t1", &job_id));
        let cancelled_at = std::time::Instant::now();
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;
        let latency = cancelled_at.elapsed();
        eprintln!(
            "SOF-CANCEL export sql-query subject: the worker stopped {latency:?} after the \
             cancel (watchdog {WATCHDOG_SECS} s)"
        );
        assert!(
            latency <= Duration::from_secs(1),
            "the subject statement must stop within 1 s of the cancel; the worker ran \
             {latency:?} after it (watchdog {WATCHDOG_SECS} s)"
        );
        assert!(matches!(
            controller.get_status("t1", &job_id),
            Some(JobStatus::Cancelled { .. })
        ));
    }

    /// Records the subject name, kind and deadline of every run, then yields
    /// `rows` rows of `{"id": "p<i>"}`. A view named `hold` first waits for
    /// `release`.
    struct ContextCapturingRunner {
        seen: std::sync::Mutex<Vec<(String, RunKind, Option<Instant>)>>,
        rows: usize,
        release: Arc<Notify>,
    }

    impl ContextCapturingRunner {
        fn new(rows: usize) -> Arc<Self> {
            Arc::new(Self {
                seen: std::sync::Mutex::new(Vec::new()),
                rows,
                release: Arc::new(Notify::new()),
            })
        }

        fn seen(&self) -> Vec<(String, RunKind, Option<Instant>)> {
            self.seen.lock().unwrap().clone()
        }

        async fn wait_for_runs(&self, n: usize) {
            tokio::time::timeout(Duration::from_secs(15), async {
                while self.seen.lock().unwrap().len() < n {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("the runs must start before the timeout");
        }
    }

    #[async_trait]
    impl SofRunner for ContextCapturingRunner {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            request: RunRequest,
            ctx: RunContext,
        ) -> Result<RunOutput, SofError> {
            let name = request.view["name"].as_str().unwrap_or("").to_string();
            self.seen
                .lock()
                .unwrap()
                .push((name.clone(), ctx.kind, ctx.deadline));
            if name == "hold" {
                self.release.notified().await;
            }
            let rows = (0..self.rows).map(|i| Ok(serde_json::json!({"id": format!("p{i}")})));
            Ok(RunOutput::from_rows(Box::pin(futures::stream::iter(rows))))
        }

        fn runner_name(&self) -> &'static str {
            "context-capturing-test-runner"
        }
    }

    /// #1864 (A17, job half; D-a "not renewed"): every run of a job gets the
    /// job's one deadline, its budget counted from the moment it got its
    /// concurrency permit (time spent queued does not count), never renewed
    /// per subject: two view subjects (`ExportJob`) and a SQL Query leaf
    /// (`SqlQueryDependency`) see the same instant.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn each_run_kind_gets_its_deadline() {
        const BUDGET: Duration = Duration::from_secs(60);
        const QUEUED: Duration = Duration::from_millis(300);
        let runner = ContextCapturingRunner::new(1);
        let controller = InMemoryController::new(
            runner.clone(),
            InMemorySink::new("http://localhost"),
            Some(1),
        )
        .with_job_budget(Some(BUDGET));

        // Job `a` holds the only permit until released; job `b` queues.
        let a = controller
            .submit(view_task(&["hold"]))
            .expect("job accepted");
        runner.wait_for_runs(1).await;
        let mut b_task = sql_query_task(10);
        b_task.work.views = ["first", "second"]
            .iter()
            .map(|n| NamedView {
                name: n.to_string(),
                view: named_view_json(n),
            })
            .collect();
        let b_submitted = Instant::now();
        let b = controller.submit(b_task).expect("job accepted");
        tokio::time::sleep(QUEUED).await;
        assert_eq!(runner.seen().len(), 1, "job b waits for the permit");
        runner.release.notify_one();

        for job in [&a, &b] {
            assert!(matches!(
                terminal_status(&controller, job).await,
                JobStatus::Completed { .. }
            ));
        }
        let seen = runner.seen();
        let kinds: Vec<_> = seen.iter().map(|(n, k, _)| (n.as_str(), *k)).collect();
        assert_eq!(
            kinds,
            vec![
                ("hold", RunKind::ExportJob),
                ("first", RunKind::ExportJob),
                ("second", RunKind::ExportJob),
                ("leaf", RunKind::SqlQueryDependency),
            ]
        );
        let deadline = seen[1].2.expect("a job run has a deadline");
        assert!(
            seen[1..].iter().all(|(_, _, d)| *d == Some(deadline)),
            "one deadline for every run of the job: {seen:?}"
        );
        assert!(
            deadline >= b_submitted + QUEUED + BUDGET,
            "the budget starts at the permit, not at submit"
        );
        assert!(deadline <= Instant::now() + BUDGET);
        assert_ne!(seen[0].2, Some(deadline), "each job has its own deadline");

        // Without a budget, a job's runs have no deadline.
        let runner = ContextCapturingRunner::new(1);
        let controller =
            InMemoryController::new(runner.clone(), InMemorySink::new("http://localhost"), None);
        let job = controller.submit(view_task(&["x"])).expect("job accepted");
        terminal_status(&controller, &job).await;
        assert_eq!(runner.seen()[0].2, None);
    }

    /// Yields two rows and then the error `fail` builds, or fails at the
    /// start of the run when `at_start` is set.
    struct KindFailingRunner {
        fail: fn() -> SofError,
        at_start: bool,
    }

    #[async_trait]
    impl SofRunner for KindFailingRunner {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            _request: RunRequest,
            _ctx: RunContext,
        ) -> Result<RunOutput, SofError> {
            if self.at_start {
                return Err((self.fail)());
            }
            let rows = (0..2).map(|i| Ok(serde_json::json!({"id": format!("p{i}")})));
            let failure = std::iter::once(Err((self.fail)()));
            Ok(RunOutput::from_rows(Box::pin(futures::stream::iter(
                rows.chain(failure),
            ))))
        }

        fn runner_name(&self) -> &'static str {
            "kind-failing-test-runner"
        }
    }

    /// #1864 (R-c, D-6): a run past its budget fails the job with `504`
    /// `timeout`, and an unavailable backend with `503` `transient`, at the
    /// start of a run or mid-stream, for a view subject and for a SQL Query
    /// leaf alike, with the generic wording `$sql-run` gives them (the
    /// SQLSTATE and driver text stay in the log) and every shard written
    /// before the failure deleted.
    #[tokio::test]
    async fn a_mid_stream_timeout_fails_the_job_with_504_timeout() {
        fn timeout() -> SofError {
            SofError::Timeout {
                budget: Some(Duration::from_secs(3600)),
                message: "page query: SQLSTATE 57014 canceling statement due to statement \
                          timeout"
                    .to_string(),
            }
        }
        fn unavailable() -> SofError {
            SofError::Unavailable {
                message: "connection pool exhausted on db-7".to_string(),
            }
        }
        /// Which runs fail, how, and the result the job must store.
        struct Case {
            name: &'static str,
            fail: fn() -> SofError,
            at_start: bool,
            sql_query: bool,
            status: StatusCode,
            code: &'static str,
        }
        let cases = [
            Case {
                name: "view, mid-stream timeout",
                fail: timeout,
                at_start: false,
                sql_query: false,
                status: StatusCode::GATEWAY_TIMEOUT,
                code: "timeout",
            },
            Case {
                name: "view, timeout at start",
                fail: timeout,
                at_start: true,
                sql_query: false,
                status: StatusCode::GATEWAY_TIMEOUT,
                code: "timeout",
            },
            Case {
                name: "view, mid-stream unavailable",
                fail: unavailable,
                at_start: false,
                sql_query: false,
                status: StatusCode::SERVICE_UNAVAILABLE,
                code: "transient",
            },
            Case {
                name: "view, unavailable at start",
                fail: unavailable,
                at_start: true,
                sql_query: false,
                status: StatusCode::SERVICE_UNAVAILABLE,
                code: "transient",
            },
            Case {
                name: "sql query leaf, mid-stream timeout",
                fail: timeout,
                at_start: false,
                sql_query: true,
                status: StatusCode::GATEWAY_TIMEOUT,
                code: "timeout",
            },
        ];
        for Case {
            name,
            fail,
            at_start,
            sql_query,
            status: want_status,
            code: want_code,
        } in cases
        {
            let sink = ScriptedSink::new();
            let controller = InMemoryController::with_shard_rows(
                Arc::new(KindFailingRunner { fail, at_start }),
                sink.clone(),
                None,
                Some(1),
            );
            let task = if sql_query {
                sql_query_task(100)
            } else {
                view_task(&["patients"])
            };
            let job_id = controller.submit(task).expect("job accepted");
            match terminal_status(&controller, &job_id).await {
                JobStatus::Failed {
                    message,
                    status,
                    code,
                    ..
                } => {
                    assert_eq!((status, code), (want_status, want_code), "{name}");
                    for detail in ["57014", "SQLSTATE", "db-7", "pool exhausted"] {
                        assert!(!message.contains(detail), "{name}: {message}");
                    }
                    assert_ne!(message, server_fault_message(&job_id), "{name}");
                }
                other => panic!("{name}: expected Failed, got {other:?}"),
            }
            wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;
            if !at_start && !sql_query {
                assert!(sink.writes() >= 1, "{name}: a shard was written first");
                assert!(
                    sink.read_shard(&job_id, "shard-0.ndjson").is_none(),
                    "{name}: the shards are deleted"
                );
            }
        }
    }

    /// #1864 (D-5, A17): a SQL Query subject's own statement is capped by the
    /// job's deadline, not only by `HFS_SOF_SQLQUERY_TIMEOUT_SECS`: with a
    /// 1 s job budget and a 30 s statement limit, the job fails with `504`
    /// `timeout` about 1 s after it started.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_sql_query_statement_is_capped_by_the_job_budget() {
        const BUDGET: Duration = Duration::from_secs(1);
        let controller = InMemoryController::new(
            ContextCapturingRunner::new(1),
            InMemorySink::new("http://localhost"),
            None,
        )
        .with_job_budget(Some(BUDGET));
        let mut task = sql_query_task(100);
        task.work.queries[0].sql = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 \
                                    FROM c WHERE x < 4000000000) SELECT count(*) AS n FROM c, vd_0"
            .to_string();
        task.work.limits.timeout_secs = 30;
        let started = Instant::now();
        let job_id = controller.submit(task).expect("job accepted");
        match terminal_status(&controller, &job_id).await {
            JobStatus::Failed { status, code, .. } => {
                assert_eq!((status, code), (StatusCode::GATEWAY_TIMEOUT, "timeout"));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        let elapsed = started.elapsed();
        eprintln!(
            "SOF-BUDGET export sql-query subject: failed {elapsed:?} after submit (budget {BUDGET:?}, statement limit 30 s)"
        );
        assert!(
            elapsed >= BUDGET && elapsed < BUDGET + Duration::from_secs(2),
            "the job ends at its budget, not at the statement limit: {elapsed:?}"
        );
    }

    /// #1864 N11 (L3-F2; D-6, A17): the job budget bounds the output phase
    /// too. A quick runner finishes well inside a 300 ms budget, but writing
    /// the shard takes the job past its deadline: the job fails with `504`
    /// `timeout`, its shards are deleted, and it never reports `Completed`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_job_whose_output_crosses_its_deadline_fails_with_504_timeout() {
        const BUDGET: Duration = Duration::from_millis(300);
        let sink = ScriptedSink::new();
        sink.on_write(|_| std::thread::sleep(BUDGET + Duration::from_millis(200)));
        let controller =
            InMemoryController::new(ContextCapturingRunner::new(3), sink.clone(), None)
                .with_job_budget(Some(BUDGET));
        let job_id = controller
            .submit(view_task(&["patients"]))
            .expect("job accepted");
        match terminal_status(&controller, &job_id).await {
            JobStatus::Failed { status, code, .. } => {
                assert_eq!((status, code), (StatusCode::GATEWAY_TIMEOUT, "timeout"));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;
        assert!(sink.writes() >= 1, "the shard was written first");
        assert!(
            sink.read_shard(&job_id, "shard-0.ndjson").is_none(),
            "the shards are deleted"
        );
    }

    /// A [`FilesystemSink`](crate::export::sink::FilesystemSink) whose
    /// completion manifest takes `delay` to persist (a slow disk or an S3
    /// `PutObject`); the manifest is on disk when the delay starts.
    #[derive(Clone)]
    struct SlowManifestSink {
        inner: crate::export::sink::FilesystemSink,
        delay: Duration,
        persisted: Arc<AtomicUsize>,
    }

    impl ExportSink for SlowManifestSink {
        fn write_shard(
            &self,
            job_id: &str,
            shard_index: usize,
            data: Vec<u8>,
            ext: &str,
        ) -> Result<String, ExportError> {
            self.inner.write_shard(job_id, shard_index, data, ext)
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
            self.inner.delete_job(job_id)
        }

        fn persist_completion(
            &self,
            job_id: &str,
            manifest: &JobManifest,
        ) -> Result<(), ExportError> {
            self.inner.persist_completion(job_id, manifest)?;
            self.persisted.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(self.delay);
            Ok(())
        }

        fn load_completed(&self) -> Vec<(String, JobManifest)> {
            self.inner.load_completed()
        }
    }

    /// #1864 N11 re-review (L3-F2; D-6, A17): the job budget bounds the
    /// completion too. The runner and the shard write finish well inside a
    /// 300 ms budget, so the gate before the completion passes, but persisting
    /// the completion manifest takes the job past its deadline: the job fails
    /// with `504` `timeout` instead of publishing `Completed`, and its shards
    /// and its manifest are deleted, so no later process rehydrates it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_job_whose_completion_manifest_crosses_its_deadline_fails_with_504_timeout() {
        const BUDGET: Duration = Duration::from_millis(300);
        let dir = tempfile::tempdir().expect("tempdir");
        let sink = SlowManifestSink {
            inner: crate::export::sink::FilesystemSink::new(dir.path(), "http://localhost"),
            delay: BUDGET + Duration::from_millis(200),
            persisted: Arc::new(AtomicUsize::new(0)),
        };
        let controller =
            InMemoryController::new(ContextCapturingRunner::new(3), sink.clone(), None)
                .with_job_budget(Some(BUDGET));
        let job_id = controller
            .submit(view_task(&["patients"]))
            .expect("job accepted");
        match terminal_status(&controller, &job_id).await {
            JobStatus::Failed { status, code, .. } => {
                assert_eq!((status, code), (StatusCode::GATEWAY_TIMEOUT, "timeout"));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;
        assert_eq!(
            sink.persisted.load(Ordering::SeqCst),
            1,
            "the gate before the completion passed and the manifest was written"
        );
        assert!(
            sink.read_shard(&job_id, "shard-0.ndjson").is_none(),
            "the shards are deleted"
        );
        assert!(
            !dir.path().join(&job_id).exists(),
            "the job directory, its completion manifest included, is deleted"
        );
        assert!(
            sink.load_completed().is_empty(),
            "no completion manifest is left to rehydrate"
        );
    }
}
