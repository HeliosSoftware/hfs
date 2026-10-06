//! In-memory `ExportJobController` implementation.
//!
//! Each job runs inside a `tokio::spawn` task, bounded by a `Semaphore`.
//! Results are stored in a `DashMap<JobId, JobStatus>`.
//!
//! A job carries a mixture of subjects (see [`ExportWork`]), written into one
//! manifest. The subjects run one after another, each as its own
//! statement(s), so they are not guaranteed a common snapshot of the data:
//! - **Views** — each named ViewDefinition is run through the `SofRunner` and
//!   its rows are streamed into output shards: every row is serialized into
//!   the current shard's buffer as it arrives (see `ShardEncoder`), and the
//!   shard is written as soon as it holds `shard_rows` rows.
//! - **SQL queries** — each named SQLQuery/SQLView Library's fully-resolved
//!   dependency graph ([`crate::handlers::sof::graph`]'s two-phase resolver)
//!   is materialized into an in-memory SQLite engine — leaf ViewDefinitions
//!   via the `SofRunner`, interior SQLView nodes by running their own
//!   (already-validated) SQL — then the subject's own SQL is executed and
//!   the result rows are sharded into output files.
//!
//! Shards are written to the sink on the blocking pool by a `ShardWriter`,
//! one at a time, while the next shard fills; a view subject therefore holds
//! about two shards in memory, not its whole result.
//!
//! A job is stopped through a per-job [`CancellationToken`], fired when the
//! job leaves `Running` (a `DELETE`, the reaper, or the job's own end): every
//! row stream the job reads stops at once, even while the runner has not
//! produced a row yet (see `StopWhenNotRunning`).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use futures::{Stream, StreamExt};
use helios_persistence::core::sof_runner::{RowStream, SofRunner, ViewFilters};
use helios_sof::sqlquery::{InMemorySqlEngine, QueryResult};
use tokio::sync::Semaphore;
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};
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
/// current one. This is the fallback for a status change that did not fire the
/// job's cancellation token; a fired token stops the stream at once.
const CANCEL_CHECK_ROWS: usize = 4096;

/// Configuration for the background task that reclaims finished export jobs.
///
/// Without this, terminal jobs (and their output shards) live for the lifetime
/// of the process: the `jobs` map grows unbounded and completed output never
/// frees, even though the completion manifest advertises a 24h `Expires`.
#[derive(Debug, Clone, Copy)]
pub struct CleanupConfig {
    /// How long after a job reaches a terminal state its output and bookkeeping
    /// are retained before the reaper removes them. Also the age past which
    /// the reaper deletes an orphaned job directory (see
    /// [`ExportSink::sweep_orphans`]).
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
    /// Cancellation token of each job whose task has not finished yet; see
    /// [`fire_signal`].
    signals: Arc<JobSignals>,
    runner: Arc<dyn SofRunner>,
    sink: Sink,
    semaphore: Arc<Semaphore>,
    shard_rows: usize,
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
    /// deleting their output via the sink and dropping their bookkeeping — and
    /// that deletes orphaned job directories older than the same TTL (see
    /// [`ExportSink::sweep_orphans`]). Must be called from within a Tokio
    /// runtime when `cleanup` is `Some`.
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
            signals: Arc::new(DashMap::new()),
            runner,
            sink,
            semaphore: Arc::new(Semaphore::new(concurrency)),
            shard_rows: shard_rows.unwrap_or(planner::DEFAULT_SHARD_ROWS),
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
                &controller.signals,
                &controller.sink,
                cfg.output_ttl,
            );
            // A process that died mid-job left a directory that no manifest
            // and no status entry account for; nothing else deletes it.
            sweep_orphans(&controller.jobs, &controller.sink, cfg.output_ttl);
            controller.spawn_cleanup(cfg);
        }
        controller
    }

    /// Spawns the background reaper. Holds only `Arc`/`Clone` handles so it is
    /// independent of the controller's own lifetime.
    fn spawn_cleanup(&self, cfg: CleanupConfig) {
        let jobs = Arc::clone(&self.jobs);
        let job_tenants = Arc::clone(&self.job_tenants);
        let signals = Arc::clone(&self.signals);
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
                reap_expired(&jobs, &job_tenants, &signals, &sink, cfg.output_ttl);
                sweep_orphans(&jobs, &sink, cfg.output_ttl);
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

        let cancel = CancellationToken::new();
        self.signals.insert(job_id.clone(), cancel.clone());

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
        // that is no longer Running stops reading rows (#1704) — at once when
        // its token fires, even before the runner's first row.
        let runner: Arc<dyn SofRunner> = Arc::new(StopWhenNotRunning {
            inner: Arc::clone(&self.runner),
            jobs: Arc::clone(&self.jobs),
            jid: job_id.clone(),
            cancel,
        });
        let sink = self.sink.clone();
        let signals = Arc::clone(&self.signals);
        let semaphore = Arc::clone(&self.semaphore);
        let jid = job_id.clone();
        let shard_rows = self.shard_rows;

        tokio::spawn(async move {
            // Acquire concurrency permit (blocks if too many jobs running)
            let _permit = semaphore.acquire().await;

            // One job, any mixture of subjects. Views run first, then
            // queries, one subject after another, each as its own
            // statement(s) — the subjects are not guaranteed a common
            // snapshot. Their outputs are concatenated into a single
            // manifest. Progress counts every subject so the X-Progress
            // percentage tracks real work across both halves.
            let total_subjects = task.work.subject_count().max(1) as u32;
            let mut writer = ShardWriter::new(sink.clone(), jid.clone(), &task.format);
            let outcome = async {
                // A job cancelled (or already reaped) while it waited for its
                // permit never starts (#1704).
                ensure_running(&jobs, &jid)?;

                let rows = run_views_job(
                    &jobs,
                    &jid,
                    submitted_at,
                    &runner,
                    &mut writer,
                    shard_rows,
                    &task,
                    &task.work.views,
                    0,
                    total_subjects,
                )
                .await?;

                let query_rows = run_sqlquery_job(
                    &jobs,
                    &jid,
                    submitted_at,
                    &runner,
                    &mut writer,
                    shard_rows,
                    &task,
                    &task.work.queries,
                    task.work.limits,
                    task.work.views.len() as u32,
                    total_subjects,
                )
                .await?;

                Ok::<_, JobFailure>(rows + query_rows)
            }
            .await;
            // Wait out a shard write still in flight whatever the outcome: the
            // cleanup below deletes this job's output, and a write landing
            // after it would leave a shard nothing deletes.
            let settled = writer.settle().await;
            let outcome = outcome.and_then(|rows| settled.map(|()| (writer.into_files(), rows)));

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
            //   When the entry is already gone, this task does not retry a
            //   failed delete and the reaper has no status entry to retry it
            //   from (#1704). On the filesystem sink the reaper's orphan sweep
            //   removes the leftover directory once it is older than
            //   `HFS_EXPORT_OUTPUT_TTL`; on S3 it stays (use a bucket
            //   lifecycle rule).
            // A job can't be Running here: every outcome arm leaves it Completed,
            // Failed, Cancelled or absent.
            if !matches!(jobs.get(&jid).as_deref(), Some(JobStatus::Completed { .. })) {
                if let Err(e) = sink.delete_job(&jid) {
                    warn!(job_id = %jid, error = %e, "failed to delete partial export output of unfinished job");
                }
            }

            // The job has left `Running` and reads nothing more; drop its
            // token so `signals` only ever holds unfinished jobs.
            fire_signal(&signals, &jid);
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
        // results on cancel. Drop any shards written so far. Firing the job's
        // token stops the row stream it is reading at once, even while the
        // runner is still computing its first row; the task also checks the
        // Cancelled state before it starts and before each subject and shard.
        // It then frees its concurrency slot and deletes whatever it wrote
        // after this point (see `submit`).
        if now_cancelled {
            fire_signal(&self.signals, job_id);
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
        match self.sink.download_url(public_base_url, job_id, filename) {
            Ok(url) => Some(url),
            Err(e) => {
                warn!(%job_id, %filename, error = %e, "failed to resolve export download URL");
                None
            }
        }
    }
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
/// A reaped job's token is fired too, in case its task is somehow still
/// reading rows.
fn reap_expired<Sink: ExportSink>(
    jobs: &DashMap<String, JobStatus>,
    job_tenants: &DashMap<String, String>,
    signals: &JobSignals,
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
        fire_signal(signals, &jid);
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

/// Deletes the output of jobs no status entry accounts for — left behind by a
/// process that stopped mid-job, before it wrote a manifest — once it has
/// been untouched for longer than `output_ttl`. A job this controller knows
/// about (running, finished, or awaiting a retried delete) is never touched;
/// see [`ExportSink::sweep_orphans`] for what the sink itself guarantees.
fn sweep_orphans<Sink: ExportSink>(
    jobs: &DashMap<String, JobStatus>,
    sink: &Sink,
    output_ttl: Duration,
) {
    for jid in sink.sweep_orphans(&|jid| jobs.contains_key(jid), output_ttl) {
        debug!(job_id = %jid, "cleanup: deleted orphaned export output");
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

/// Per-job cancellation tokens, keyed by job id. A job's entry lives from
/// `submit` until it is fired.
type JobSignals = DashMap<String, CancellationToken>;

/// Fires `jid`'s cancellation token and forgets it, so every row stream the
/// job is reading stops at once (see [`StopWhenNotRunning`]). Called when the
/// job leaves `Running` — on a cancel, when the reaper removes it, and when
/// its own task ends. A job with no token left is a no-op.
fn fire_signal(signals: &JobSignals, jid: &str) {
    if let Some((_, token)) = signals.remove(jid) {
        token.cancel();
    }
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
/// `Running`, or whose token fires while the inner runner is still preparing
/// its stream. The returned [`StopOnSignal`] stream yields
/// `Err(SofError::Cancelled)` as soon as the token fires — also while the
/// runner has no row ready yet (e.g. PostgreSQL still sorting) — and, as a
/// fallback for a status change that fired no token, re-checks the status
/// every [`CANCEL_CHECK_ROWS`] rows. Both consumers stop at the first `Err`.
/// The inner stream is dropped right there, which drops the inner runner's
/// channel receiver and stops its producer; the SQL runners stop their
/// running statement when that channel closes.
struct StopWhenNotRunning {
    inner: Arc<dyn SofRunner>,
    jobs: Arc<DashMap<String, JobStatus>>,
    jid: String,
    /// The job's token, fired by [`fire_signal`].
    cancel: CancellationToken,
}

#[async_trait]
impl SofRunner for StopWhenNotRunning {
    async fn run_view(
        &self,
        tenant: &TenantContext,
        view_definition: serde_json::Value,
        filters: ViewFilters,
    ) -> Result<RowStream, SofError> {
        if self.cancel.is_cancelled() || !is_running(&self.jobs, &self.jid) {
            return Err(SofError::Cancelled);
        }
        let stream = tokio::select! {
            biased;
            () = self.cancel.cancelled() => return Err(SofError::Cancelled),
            stream = self.inner.run_view(tenant, view_definition, filters) => stream?,
        };
        Ok(Box::pin(StopOnSignal {
            inner: Some(stream),
            fired: Box::pin(self.cancel.clone().cancelled_owned()),
            cancel: self.cancel.clone(),
            jobs: Arc::clone(&self.jobs),
            jid: self.jid.clone(),
            rows: 0,
        }))
    }

    fn runner_name(&self) -> &'static str {
        self.inner.runner_name()
    }
}

/// The row stream [`StopWhenNotRunning`] hands out: the inner runner's rows
/// until the job's token fires or the every-[`CANCEL_CHECK_ROWS`] status check
/// finds the job no longer `Running`, then one `Err(SofError::Cancelled)` and
/// the end of the stream.
struct StopOnSignal {
    /// `None` once the stream has stopped or ended: the inner stream is
    /// dropped at that moment, not when the consumer drops this one.
    inner: Option<RowStream>,
    /// Polled only while the inner stream is pending, so that a fired token
    /// wakes a consumer waiting for the runner's next row.
    fired: Pin<Box<WaitForCancellationFutureOwned>>,
    cancel: CancellationToken,
    jobs: Arc<DashMap<String, JobStatus>>,
    jid: String,
    rows: usize,
}

impl StopOnSignal {
    fn stop(&mut self) -> Poll<Option<Result<serde_json::Value, SofError>>> {
        self.inner = None;
        Poll::Ready(Some(Err(SofError::Cancelled)))
    }
}

impl Stream for StopOnSignal {
    type Item = Result<serde_json::Value, SofError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        let Some(inner) = this.inner.as_mut() else {
            return Poll::Ready(None);
        };
        if this.cancel.is_cancelled() {
            return this.stop();
        }
        match inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(row)) => {
                this.rows += 1;
                if this.rows.is_multiple_of(CANCEL_CHECK_ROWS) && !is_running(&this.jobs, &this.jid)
                {
                    return this.stop();
                }
                Poll::Ready(Some(row))
            }
            Poll::Ready(None) => {
                this.inner = None;
                Poll::Ready(None)
            }
            Poll::Pending => {
                if this.fired.as_mut().poll(cx).is_ready() {
                    this.stop()
                } else {
                    Poll::Pending
                }
            }
        }
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
/// `SofRunner` and stream its rows into output shards. An error from a view's
/// row stream fails the whole job instead of being skipped, so a shard is
/// never published as a complete file when rows are actually missing (the
/// shards already written are deleted with the failed job).
///
/// Rows are never collected: each is serialized into the current shard by a
/// [`ShardEncoder`] and dropped, and the shard goes to `writer` once it holds
/// `shard_rows` rows, the remainder at the end. The boundaries are the ones
/// [`planner::plan`] gives for the view's row count, so shard files and their
/// row counts are the same as slicing the whole result. Returns the number of
/// rows read.
#[allow(clippy::too_many_arguments)]
async fn run_views_job<Sink: ExportSink>(
    jobs: &DashMap<String, JobStatus>,
    jid: &str,
    submitted_at: DateTime<Utc>,
    runner: &Arc<dyn SofRunner>,
    writer: &mut ShardWriter<Sink>,
    shard_rows: usize,
    task: &ExportTask,
    views: &[NamedView],
    // `progress_offset`: subjects already finished before this half started.
    // `total_subjects`: subjects in the whole job, views and queries together.
    progress_offset: u32,
    total_subjects: u32,
) -> Result<usize, JobFailure> {
    let format = ShardFormat::parse(&task.format.to_lowercase());
    // `planner::plan` reads a zero shard size as "one shard for everything".
    let shard_limit = if shard_rows == 0 {
        usize::MAX
    } else {
        shard_rows
    };

    let mut total_rows: usize = 0;

    // Each ViewDefinition subject produces its own set of output shards, and
    // `output.name` in the manifest carries its name. Progress advances by one
    // subject per view finished, once all its shards are written.
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
        let label = format!("view '{}'", named.name);

        let mut stream = runner
            .run_view(&task.tenant, named.view.clone(), task.filters.clone())
            .await
            .map_err(|e| JobFailure::from_sof(&label, e))?;

        // An in-DB SQL runner's view has one declared layout, shared by every
        // shard, so a shard whose first row has a NULL keeps that column.
        let sql_columns =
            crate::handlers::sof::run::sql_output_columns(runner.runner_name(), &named.view);
        let mut shard = ShardEncoder::new(format, task.header, sql_columns.as_deref());

        while let Some(item) = stream.next().await {
            match item {
                Ok(row) => {
                    total_rows += 1;
                    shard.push(row).map_err(|e| format!("{label}: {e}"))?;
                    // Notice a failed shard write without waiting for the
                    // next shard to fill.
                    if total_rows.is_multiple_of(CANCEL_CHECK_ROWS) {
                        writer.settle_if_done().await?;
                    }
                    if shard.rows() == shard_limit {
                        let (data, row_count) = shard.finish();
                        writer
                            .write(jobs, &named.name, &label, data, row_count)
                            .await?;
                    }
                }
                Err(e) => {
                    if matches!(e, SofError::Cancelled) {
                        debug!(view = %named.name, "export row stream stopped: job is no longer running");
                    } else {
                        warn!(view = %named.name, error = %e, "export row stream failed");
                    }
                    return Err(format!("{label}: {e}").into());
                }
            }
        }
        drop(stream);

        // Spec: `output` is 0..*. Views with zero rows simply contribute no
        // `output` entries rather than emitting an empty shard with a
        // download URL pointing at zero bytes.
        if shard.rows() > 0 {
            let (data, row_count) = shard.finish();
            writer
                .write(jobs, &named.name, &label, data, row_count)
                .await?;
        }
        writer.settle().await?;

        record_subject_finished(
            jobs,
            jid,
            submitted_at,
            progress_offset + (view_idx as u32) + 1,
            total_subjects,
        );
    }

    Ok(total_rows)
}

/// The SQLQuery / SQLView half of an export job: materialize each subject's
/// table sources via the `SofRunner`, execute the pre-validated SQL, and shard
/// the result rows into output files. Returns the number of result rows.
#[allow(clippy::too_many_arguments)]
async fn run_sqlquery_job<Sink: ExportSink>(
    jobs: &DashMap<String, JobStatus>,
    jid: &str,
    submitted_at: DateTime<Utc>,
    runner: &Arc<dyn SofRunner>,
    writer: &mut ShardWriter<Sink>,
    shard_rows: usize,
    task: &ExportTask,
    queries: &[NamedSqlQuery],
    limits: SqlExportLimits,
    // `progress_offset`: subjects already finished before this half started.
    // `total_subjects`: subjects in the whole job, views and queries together.
    progress_offset: u32,
    total_subjects: u32,
) -> Result<usize, JobFailure> {
    let format = task.format.to_lowercase();
    let content_type = query_content_type(&format, task.header);

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
        let label = format!("query '{}'", query.name);

        for range in planner::plan(result.rows.len(), shard_rows) {
            let row_count = range.len();
            let data = ShardData::Output(query_shard(&result, range), content_type);
            writer
                .write(jobs, &query.name, &label, data, row_count)
                .await?;
        }
        writer.settle().await?;

        record_subject_finished(
            jobs,
            jid,
            submitted_at,
            progress_offset + (query_idx as u32) + 1,
            total_subjects,
        );
    }

    Ok(total_rows)
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
// Shard writing
// ============================================================================

/// Writes a job's shards to the sink on the blocking pool, one at a time.
///
/// [`write`](Self::write) first waits for the previous shard's write, so at
/// most one write is in flight while the caller fills the next shard: a job
/// holds about two shards at once. Shard keys run across the whole job, so
/// every shard gets a unique `shard-{N}` filename (for a single-subject job
/// the historical numbering exactly), and the written files are recorded in
/// write order. [`settle`](Self::settle) must run before the job's outcome is
/// acted on, so that no write outlives the job's own cleanup.
struct ShardWriter<Sink: ExportSink> {
    sink: Sink,
    jid: String,
    ext: &'static str,
    next_key: usize,
    in_flight: Option<InFlightShard>,
    files: Vec<CompletedFile>,
}

/// A shard [`ShardWriter`] has handed to the blocking pool.
struct InFlightShard {
    handle: tokio::task::JoinHandle<Result<String, ExportError>>,
    /// Output name of the subject the shard belongs to.
    subject: String,
    /// `view '…'` / `query '…'`, prefixed to a failure's message.
    label: String,
    key: usize,
    row_count: usize,
}

impl<Sink: ExportSink> ShardWriter<Sink> {
    fn new(sink: Sink, jid: String, format: &str) -> Self {
        Self {
            sink,
            jid,
            ext: ext_for(&format.to_lowercase()),
            next_key: 0,
            in_flight: None,
            files: Vec::new(),
        }
    }

    /// Hands `data` to the blocking pool, which encodes it if needed and
    /// writes it as the job's next shard. Waits for the previous write first
    /// and fails, instead of writing, once the job is no longer `Running`.
    async fn write(
        &mut self,
        jobs: &DashMap<String, JobStatus>,
        subject: &str,
        label: &str,
        data: ShardData,
        row_count: usize,
    ) -> Result<(), JobFailure> {
        self.settle().await?;
        ensure_running(jobs, &self.jid)?;

        let key = self.next_key;
        self.next_key += 1;
        let sink = self.sink.clone();
        let jid = self.jid.clone();
        let ext = self.ext;
        let handle = tokio::task::spawn_blocking(move || {
            sink.write_shard(&jid, key, data.into_bytes()?, ext)
        });
        self.in_flight = Some(InFlightShard {
            handle,
            subject: subject.to_string(),
            label: label.to_string(),
            key,
            row_count,
        });
        Ok(())
    }

    /// Waits for the write in flight, if any, and records its file. A failed
    /// write fails the job.
    async fn settle(&mut self) -> Result<(), JobFailure> {
        let Some(shard) = self.in_flight.take() else {
            return Ok(());
        };
        let filename = shard
            .handle
            .await
            .map_err(|e| format!("{}: shard write task failed: {e}", shard.label))?
            .map_err(|e| format!("{}: {e}", shard.label))?;
        debug!(job_id = %self.jid, subject = %shard.subject, shard = shard.key, rows = shard.row_count, file = %filename, "shard written");
        self.files.push(CompletedFile {
            view_name: shard.subject,
            filename,
            row_count: shard.row_count,
        });
        Ok(())
    }

    /// [`settle`](Self::settle)s the write in flight if it has already
    /// finished, so a failed write fails the job without waiting for the
    /// next shard. Never waits.
    async fn settle_if_done(&mut self) -> Result<(), JobFailure> {
        if self
            .in_flight
            .as_ref()
            .is_some_and(|shard| shard.handle.is_finished())
        {
            self.settle().await?;
        }
        Ok(())
    }

    /// The files written so far, in shard order. Call after
    /// [`settle`](Self::settle).
    fn into_files(self) -> Vec<CompletedFile> {
        self.files
    }
}

/// A finished shard, as [`ShardWriter::write`] takes it. Whatever encoding is
/// left happens on the blocking pool, together with the write.
enum ShardData {
    /// Serialized bytes, ready to write.
    Bytes(Vec<u8>),
    /// A Parquet view shard: the cell values, encoded when written.
    Parquet(helios_sof::ProcessedResult),
    /// A SQL query shard, formatted through `helios_sof::format_output`
    /// (matching the `$sql-run` bytes) when written.
    Output(helios_sof::ProcessedResult, helios_sof::ContentType),
}

impl ShardData {
    fn into_bytes(self) -> Result<Vec<u8>, ExportError> {
        match self {
            ShardData::Bytes(bytes) => Ok(bytes),
            ShardData::Parquet(result) => encode_parquet(result),
            ShardData::Output(processed, content_type) => {
                helios_sof::format_output(processed, content_type, None)
                    .map_err(|e| ExportError::Serialization(e.to_string()))
            }
        }
    }
}

// ============================================================================
// Row serialization helpers
// ============================================================================

/// Output format of a view shard (`_format`, lower-cased).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShardFormat {
    Ndjson,
    Json,
    Csv,
    Parquet,
}

impl ShardFormat {
    fn parse(format: &str) -> Self {
        match format {
            "csv" => ShardFormat::Csv,
            "parquet" => ShardFormat::Parquet,
            "json" => ShardFormat::Json,
            _ => ShardFormat::Ndjson,
        }
    }
}

/// Serializes the rows of one view shard as they arrive, so each row's
/// `Value` is dropped as soon as it is encoded instead of the whole result
/// being held until the stream ends.
///
/// The output is byte-identical to formatting the shard's rows in one go (the
/// `format_*` functions the tests keep as oracles): NDJSON writes one
/// `serde_json` line per row, JSON one `serde_json` array of the rows; CSV and
/// Parquet take their columns from `columns` when given (see
/// [`crate::handlers::sof::run::sql_output_columns`]), else from the shard's
/// first row, and CSV skips a row that is not an object. The header is
/// written only when requested.
///
/// Parquet is not encoded row by row: the encoder keeps the shard's cell
/// values (projected onto the columns, without each row's key map) and the
/// Parquet writer runs when the shard is written, so a Parquet export still
/// holds one shard of values.
struct ShardEncoder<'a> {
    format: ShardFormat,
    include_csv_header: bool,
    declared: Option<&'a [String]>,
    /// Rows pushed into the current shard.
    rows: usize,
    /// CSV / Parquet columns of the current shard, fixed by its first row.
    columns: Vec<String>,
    /// NDJSON / JSON / CSV bytes of the current shard.
    bytes: Vec<u8>,
    /// Parquet cell values of the current shard.
    values: Vec<helios_sof::ProcessedRow>,
}

impl<'a> ShardEncoder<'a> {
    fn new(format: ShardFormat, include_csv_header: bool, columns: Option<&'a [String]>) -> Self {
        Self {
            format,
            include_csv_header,
            declared: columns,
            rows: 0,
            columns: Vec::new(),
            bytes: Vec::new(),
            values: Vec::new(),
        }
    }

    /// Rows in the current shard.
    fn rows(&self) -> usize {
        self.rows
    }

    /// Appends `row` to the current shard.
    fn push(&mut self, row: serde_json::Value) -> Result<(), ExportError> {
        if self.rows == 0 {
            self.start_shard(&row);
        }
        match self.format {
            ShardFormat::Ndjson => {
                serde_json::to_writer(&mut self.bytes, &row)
                    .map_err(|e| ExportError::Serialization(e.to_string()))?;
                self.bytes.push(b'\n');
            }
            ShardFormat::Json => {
                if self.rows > 0 {
                    self.bytes.push(b',');
                }
                serde_json::to_writer(&mut self.bytes, &row)
                    .map_err(|e| ExportError::Serialization(e.to_string()))?;
            }
            ShardFormat::Csv => {
                if let Some(obj) = row.as_object() {
                    for (i, column) in self.columns.iter().enumerate() {
                        if i > 0 {
                            self.bytes.push(b',');
                        }
                        write_csv_cell(
                            &mut self.bytes,
                            obj.get(column).unwrap_or(&serde_json::Value::Null),
                        );
                    }
                    self.bytes.push(b'\n');
                }
            }
            ShardFormat::Parquet => {
                let values = self
                    .columns
                    .iter()
                    .map(|column| row.as_object().and_then(|o| o.get(column)).cloned())
                    .collect();
                self.values.push(helios_sof::ProcessedRow { values });
            }
        }
        self.rows += 1;
        Ok(())
    }

    /// Fixes the shard's columns and writes what precedes its first row.
    fn start_shard(&mut self, first: &serde_json::Value) {
        match self.format {
            ShardFormat::Ndjson => {}
            ShardFormat::Json => self.bytes.push(b'['),
            ShardFormat::Csv | ShardFormat::Parquet => {
                self.columns = shard_columns(std::slice::from_ref(first), self.declared);
                if self.format == ShardFormat::Csv && self.include_csv_header {
                    self.bytes
                        .extend_from_slice(self.columns.join(",").as_bytes());
                    self.bytes.push(b'\n');
                }
            }
        }
    }

    /// Closes the current shard and returns it with its row count; the next
    /// push starts a new shard. Only call it on a shard with rows.
    fn finish(&mut self) -> (ShardData, usize) {
        let rows = std::mem::take(&mut self.rows);
        let columns = std::mem::take(&mut self.columns);
        let data = match self.format {
            ShardFormat::Parquet => ShardData::Parquet(helios_sof::ProcessedResult {
                columns,
                rows: std::mem::take(&mut self.values),
            }),
            ShardFormat::Json => {
                self.bytes.push(b']');
                ShardData::Bytes(self.take_bytes())
            }
            ShardFormat::Ndjson | ShardFormat::Csv => ShardData::Bytes(self.take_bytes()),
        };
        (data, rows)
    }

    /// Takes the current shard's bytes, sizing the next shard's buffer like
    /// this one so it does not regrow (and copy itself) from empty.
    fn take_bytes(&mut self) -> Vec<u8> {
        let next = Vec::with_capacity(self.bytes.len());
        std::mem::replace(&mut self.bytes, next)
    }
}

/// Encodes a view shard's cell values as one Parquet file.
fn encode_parquet(result: helios_sof::ProcessedResult) -> Result<Vec<u8>, ExportError> {
    helios_sof::format_parquet_multi_file(result, None, usize::MAX)
        .map_err(|e| ExportError::Serialization(e.to_string()))
        .map(|files| files.into_iter().next().unwrap_or_default())
}

/// Serializes a shard of view-output rows (column → value JSON objects) in one
/// go. The export streams its shards through [`ShardEncoder`] instead; the
/// tests keep this as the oracle its bytes must match.
///
/// `columns` fixes the CSV / Parquet columns (see
/// [`crate::handlers::sof::run::sql_output_columns`]); `None` infers them
/// from the shard's first row. JSON and NDJSON write the row objects as-is.
#[cfg(test)]
fn format_rows(
    rows: &[serde_json::Value],
    format: &str,
    include_csv_header: bool,
    columns: Option<&[String]>,
) -> Result<Vec<u8>, ExportError> {
    match format {
        "csv" => format_csv(rows, include_csv_header, columns),
        "parquet" => format_parquet(rows, columns),
        "json" => format_json_array(rows),
        _ => format_ndjson(rows),
    }
}

/// The `helios_sof::format_output` content type a SQL query shard is written
/// with (matching the `$sql-run` bytes). The export operations support flat
/// formats only; `fhir` is a run-operation format and is rejected at kick-off.
fn query_content_type(format: &str, include_csv_header: bool) -> helios_sof::ContentType {
    match format {
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
    }
}

/// One shard of SQL query result rows, for [`ShardData::Output`]. Built as a
/// `ProcessedResult` directly so columns keep their SQL order (mirrors the
/// `$sql-run` handler).
fn query_shard(result: &QueryResult, range: std::ops::Range<usize>) -> helios_sof::ProcessedResult {
    helios_sof::ProcessedResult {
        columns: result.columns.clone(),
        rows: result.rows[range]
            .iter()
            .map(|r| helios_sof::ProcessedRow { values: r.clone() })
            .collect(),
    }
}

/// Serialises rows as a single JSON array (`_format=json`). Test oracle; see
/// [`format_rows`].
#[cfg(test)]
fn format_json_array(rows: &[serde_json::Value]) -> Result<Vec<u8>, ExportError> {
    serde_json::to_vec(rows).map_err(|e| ExportError::Serialization(e.to_string()))
}

#[cfg(test)]
fn format_parquet(
    rows: &[serde_json::Value],
    columns: Option<&[String]>,
) -> Result<Vec<u8>, ExportError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let columns = shard_columns(rows, columns);

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

#[cfg(test)]
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

/// A shard's columns: `columns` when given, else the first row's keys.
fn shard_columns(rows: &[serde_json::Value], columns: Option<&[String]>) -> Vec<String> {
    match columns {
        Some(columns) => columns.to_vec(),
        None => rows
            .first()
            .and_then(|row| row.as_object())
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default(),
    }
}

#[cfg(test)]
fn format_csv(
    rows: &[serde_json::Value],
    include_header: bool,
    columns: Option<&[String]>,
) -> Result<Vec<u8>, ExportError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let cols = shard_columns(rows, columns);

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

#[cfg(test)]
fn csv_cell(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => {
            if s.contains(',') || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.clone()
            }
        }
        other => {
            let s = other.to_string();
            format!("\"{}\"", s.replace('"', "\"\""))
        }
    }
}

/// Appends one CSV cell for `v` to `out`: empty for null, the plain text of a
/// boolean, number or string, and a string holding a comma, quote or newline —
/// or any array/object, written as compact JSON — in double quotes with its
/// quotes doubled.
fn write_csv_cell(out: &mut Vec<u8>, v: &serde_json::Value) {
    use std::io::Write as _;
    match v {
        serde_json::Value::Null => {}
        serde_json::Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        // Writing into a `Vec` cannot fail.
        serde_json::Value::Number(n) => {
            let _ = write!(out, "{n}");
        }
        serde_json::Value::String(s) => {
            if s.contains(',') || s.contains('"') || s.contains('\n') {
                write_quoted_csv(out, s);
            } else {
                out.extend_from_slice(s.as_bytes());
            }
        }
        other => write_quoted_csv(out, &other.to_string()),
    }
}

fn write_quoted_csv(out: &mut Vec<u8>, s: &str) {
    out.push(b'"');
    out.extend_from_slice(s.replace('"', "\"\"").as_bytes());
    out.push(b'"');
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

    fn shard_rows_with_bare_first() -> Vec<serde_json::Value> {
        vec![
            serde_json::json!({"id": "a3", "family": "Three"}),
            serde_json::json!({"id": "a4", "gender": "other", "family": "Four"}),
        ]
    }

    /// Without declared columns (any non-SQL runner) a CSV shard keeps its
    /// first row's columns, as before #1623.
    #[test]
    fn csv_shard_without_columns_keeps_first_row_inference() {
        let data = format_rows(&shard_rows_with_bare_first(), "csv", true, None).expect("csv");
        assert_eq!(
            String::from_utf8(data).unwrap(),
            "id,family\na3,Three\na4,Four\n"
        );
    }

    #[test]
    fn csv_shard_with_columns_keeps_every_declared_column() {
        let columns = ["id", "gender", "family"].map(String::from);
        let data =
            format_rows(&shard_rows_with_bare_first(), "csv", true, Some(&columns)).expect("csv");
        assert_eq!(
            String::from_utf8(data).unwrap(),
            "id,gender,family\na3,,Three\na4,other,Four\n"
        );
    }

    /// JSON and NDJSON shards write the row objects untouched either way.
    #[test]
    fn json_shards_ignore_declared_columns() {
        let rows = shard_rows_with_bare_first();
        let columns = ["id", "gender", "family"].map(String::from);
        for format in ["json", "ndjson"] {
            assert_eq!(
                format_rows(&rows, format, true, Some(&columns)).expect("format"),
                format_rows(&rows, format, true, None).expect("format"),
                "{format}"
            );
        }
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
        /// Shard keys from this one on fail to write.
        fail_writes_from: Arc<AtomicUsize>,
    }

    impl ScriptedSink {
        fn new() -> Self {
            Self {
                inner: InMemorySink::new("http://localhost"),
                writes: Arc::new(AtomicUsize::new(0)),
                after_write: Arc::new(std::sync::Mutex::new(None)),
                fail_deletes: Arc::new(AtomicBool::new(false)),
                deletes: Arc::new(AtomicUsize::new(0)),
                fail_writes_from: Arc::new(AtomicUsize::new(usize::MAX)),
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
            if shard_index >= self.fail_writes_from.load(Ordering::SeqCst) {
                return Err(ExportError::Sink("injected write failure".into()));
            }
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

    // ------------------------------------------------------------------
    // Streamed shards: the bytes, boundaries and row counts must be the
    // ones the collect-then-slice export produced (`planner::plan` plus the
    // `format_*` oracles).
    // ------------------------------------------------------------------

    /// Every format the export writes, with and without a CSV header.
    const FORMATS: [(&str, bool); 6] = [
        ("ndjson", false),
        ("json", false),
        ("csv", true),
        ("csv", false),
        ("parquet", false),
        ("NDJSON", false),
    ];

    /// Rows covering what a cell can hold: nulls, missing keys, nested
    /// objects and arrays, strings that need CSV quoting, numbers, booleans
    /// and a row that is not an object.
    fn mixed_rows() -> Vec<serde_json::Value> {
        vec![
            serde_json::json!({"id": "a", "n": 1, "b": true, "s": "plain"}),
            serde_json::json!({"id": "b", "n": null, "b": false, "s": "has,comma"}),
            serde_json::json!({"id": "c", "s": "has \"quotes\"", "nested": {"k": [1, 2, {"x": "y,z"}]}}),
            serde_json::json!({"id": "d", "n": 2.5, "s": "line\nbreak", "arr": ["a", "b,c", null]}),
            serde_json::json!({"n": -7, "s": "", "b": null}),
            serde_json::json!("not an object"),
            serde_json::json!({"id": "e", "n": 12_345_678_901_234_567_890u64, "s": "ünïcödé"}),
            serde_json::json!({"id": "f", "n": 1e300, "nested": {}}),
        ]
    }

    /// Rows a Parquet writer accepts: one type per column, nulls and missing
    /// keys included.
    fn typed_rows() -> Vec<serde_json::Value> {
        vec![
            serde_json::json!({"id": "a", "gender": null, "age": 30, "active": true}),
            serde_json::json!({"id": "b", "gender": "female", "age": 41, "active": false}),
            serde_json::json!({"id": "c, \"q\"", "age": 7}),
            serde_json::json!({"id": "d", "gender": "male", "age": null, "active": true}),
            serde_json::json!({"id": "e", "gender": "other", "age": 52, "active": null}),
        ]
    }

    /// Rows whose keys change between shards, so a shard without declared
    /// columns must infer them from its own first row.
    fn shifting_rows() -> Vec<serde_json::Value> {
        vec![
            serde_json::json!({"a": "1"}),
            serde_json::json!({"a": "2", "b": "3"}),
            serde_json::json!({"b": "4", "c": "5"}),
            serde_json::json!({"a": "6"}),
            serde_json::json!({"c": "7", "a": "8"}),
        ]
    }

    type Shards = Vec<(Result<Vec<u8>, String>, usize)>;

    /// The collect-then-slice export: `planner::plan` over the whole result,
    /// each slice formatted in one go.
    fn sliced_shards(
        rows: &[serde_json::Value],
        format: &str,
        header: bool,
        columns: Option<&[String]>,
        shard_rows: usize,
    ) -> Shards {
        let format = format.to_lowercase();
        planner::plan(rows.len(), shard_rows)
            .into_iter()
            .map(|range| {
                let bytes = format_rows(&rows[range.clone()], &format, header, columns)
                    .map_err(|e| e.to_string());
                (bytes, range.len())
            })
            .collect()
    }

    /// The streamed export: rows pushed one by one into a [`ShardEncoder`]
    /// that is finished every `shard_rows` rows, as `run_views_job` does.
    fn streamed_shards(
        rows: &[serde_json::Value],
        format: &str,
        header: bool,
        columns: Option<&[String]>,
        shard_rows: usize,
    ) -> Shards {
        let limit = if shard_rows == 0 {
            usize::MAX
        } else {
            shard_rows
        };
        let mut encoder =
            ShardEncoder::new(ShardFormat::parse(&format.to_lowercase()), header, columns);
        let mut shards = Vec::new();
        let mut finish = |encoder: &mut ShardEncoder<'_>| {
            let (data, row_count) = encoder.finish();
            shards.push((data.into_bytes().map_err(|e| e.to_string()), row_count));
        };
        for row in rows {
            encoder.push(row.clone()).expect("encode");
            if encoder.rows() == limit {
                finish(&mut encoder);
            }
        }
        if encoder.rows() > 0 {
            finish(&mut encoder);
        }
        shards
    }

    fn assert_streamed_matches_sliced(
        rows: &[serde_json::Value],
        columns: Option<&[String]>,
        formats: &[(&str, bool)],
    ) {
        for &(format, header) in formats {
            for shard_rows in [0, 1, 2, 3, rows.len(), rows.len() + 1] {
                let sliced = sliced_shards(rows, format, header, columns, shard_rows);
                let streamed = streamed_shards(rows, format, header, columns, shard_rows);
                assert_eq!(
                    streamed, sliced,
                    "format {format}, header {header}, columns {columns:?}, shard_rows {shard_rows}"
                );
            }
        }
    }

    #[test]
    fn streamed_shards_are_byte_identical_to_sliced_shards() {
        let declared = ["id", "n", "b", "s", "nested", "arr", "missing"].map(String::from);
        for columns in [None, Some(&declared[..])] {
            // Parquet needs one type per column; `mixed_rows` has several.
            assert_streamed_matches_sliced(&mixed_rows(), columns, &FORMATS[..4]);
        }

        let declared = ["id", "gender", "age", "active"].map(String::from);
        for columns in [None, Some(&declared[..])] {
            assert_streamed_matches_sliced(&typed_rows(), columns, &FORMATS);
            assert_streamed_matches_sliced(&shifting_rows(), columns, &FORMATS);
        }
    }

    /// The oracles agree on the cases the comparison leans on: a declared
    /// column survives a first row holding NULL, and without declared columns
    /// each shard takes its own first row's keys.
    #[test]
    fn streamed_csv_keeps_declared_columns_and_infers_per_shard() {
        let declared = ["id", "gender", "age", "active"].map(String::from);
        let with_declared = streamed_shards(&typed_rows(), "csv", true, Some(&declared), 2);
        assert_eq!(
            String::from_utf8(with_declared[0].0.clone().unwrap()).unwrap(),
            "id,gender,age,active\na,,30,true\nb,female,41,false\n"
        );

        let inferred = streamed_shards(&shifting_rows(), "csv", true, None, 2);
        let texts: Vec<String> = inferred
            .into_iter()
            .map(|(bytes, _)| String::from_utf8(bytes.unwrap()).unwrap())
            .collect();
        assert_eq!(texts, vec!["a\n1\n2\n", "b,c\n4,5\n,\n", "c,a\n7,8\n"]);
    }

    /// A `SofRunner` that streams a fixed set of rows under a chosen runner
    /// name (an in-DB SQL runner's name makes the export use the view's
    /// declared columns).
    struct VecRunner {
        rows: Vec<serde_json::Value>,
        name: &'static str,
    }

    #[async_trait]
    impl SofRunner for VecRunner {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            _view_definition: serde_json::Value,
            _filters: ViewFilters,
        ) -> Result<RowStream, SofError> {
            Ok(Box::pin(futures::stream::iter(
                self.rows.clone().into_iter().map(Ok),
            )))
        }

        fn runner_name(&self) -> &'static str {
            self.name
        }
    }

    fn demographics_view() -> serde_json::Value {
        serde_json::json!({
            "resourceType": "ViewDefinition",
            "name": "demographics",
            "resource": "Patient",
            "status": "active",
            "select": [{"column": [
                {"name": "id", "path": "id"},
                {"name": "gender", "path": "gender"},
                {"name": "age", "path": "extension.value.ofType(integer)"},
                {"name": "active", "path": "active"}
            ]}]
        })
    }

    async fn completed_files<S: ExportSink>(
        controller: &InMemoryController<S>,
        job_id: &str,
    ) -> Vec<CompletedFile> {
        let status = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                match controller.get_status("t1", job_id) {
                    Some(JobStatus::Running { .. }) => {
                        tokio::time::sleep(Duration::from_millis(10)).await
                    }
                    other => return other,
                }
            }
        })
        .await
        .expect("export job must finish before the timeout");
        match status {
            Some(JobStatus::Completed { files, .. }) => files,
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    /// End to end through the controller: every format, with declared
    /// columns (an in-DB SQL runner) and without, across several shards and
    /// two subjects, writes the shards — names, row counts and bytes — that
    /// slicing the collected result wrote.
    #[tokio::test]
    async fn streamed_export_writes_the_sliced_shards() {
        for runner_name in ["postgres-indb", "gate-test-runner"] {
            let columns =
                crate::handlers::sof::run::sql_output_columns(runner_name, &demographics_view());
            if runner_name == "postgres-indb" {
                assert_eq!(
                    columns.as_deref(),
                    Some(&["id", "gender", "age", "active"].map(String::from)[..]),
                    "sanity: an in-DB runner declares the view's columns"
                );
            }
            for (format, header) in FORMATS {
                let sink = InMemorySink::new("http://localhost");
                let controller = InMemoryController::with_shard_rows(
                    Arc::new(VecRunner {
                        rows: typed_rows(),
                        name: runner_name,
                    }),
                    sink.clone(),
                    None,
                    Some(2),
                );
                let mut task = view_task(&["first", "second"]);
                for view in &mut task.work.views {
                    view.view = demographics_view();
                }
                task.format = format.to_string();
                task.header = header;
                let job_id = controller.submit(task);
                let files = completed_files(&controller, &job_id).await;

                let expected = sliced_shards(&typed_rows(), format, header, columns.as_deref(), 2);
                let ext = ext_for(&format.to_lowercase());
                let context = format!("{runner_name}, {format}, header {header}");
                assert_eq!(files.len(), 2 * expected.len(), "{context}");
                for (i, file) in files.iter().enumerate() {
                    let (bytes, row_count) = &expected[i % expected.len()];
                    let subject = if i < expected.len() {
                        "first"
                    } else {
                        "second"
                    };
                    assert_eq!(file.view_name, subject, "{context}");
                    assert_eq!(file.filename, format!("shard-{i}.{ext}"), "{context}");
                    assert_eq!(file.row_count, *row_count, "{context}");
                    assert_eq!(
                        sink.read_shard(&job_id, &file.filename).as_ref(),
                        Some(bytes.as_ref().expect("oracle formats")),
                        "{context}, {}",
                        file.filename
                    );
                }
            }
        }
    }

    /// A view with no rows writes no shard and contributes no output entry,
    /// for CSV with a header too (no header-only file).
    #[tokio::test]
    async fn a_view_without_rows_writes_no_shard() {
        let sink = InMemorySink::new("http://localhost");
        let controller = InMemoryController::new(
            Arc::new(VecRunner {
                rows: Vec::new(),
                name: "postgres-indb",
            }),
            sink.clone(),
            None,
        );
        let mut task = view_task(&["empty"]);
        task.work.views[0].view = demographics_view();
        task.format = "csv".to_string();
        task.header = true;
        let job_id = controller.submit(task);

        assert!(completed_files(&controller, &job_id).await.is_empty());
        assert!(sink.read_shard(&job_id, "shard-0.csv").is_none());
    }

    /// A shard write that fails fails the job as a server fault, and the
    /// shards written before it are deleted with the failed job.
    #[tokio::test]
    async fn a_failed_shard_write_fails_the_job_and_deletes_its_output() {
        let sink = ScriptedSink::new();
        sink.fail_writes_from.store(1, Ordering::SeqCst);
        let controller = InMemoryController::with_shard_rows(
            Arc::new(VecRunner {
                rows: typed_rows(),
                name: "gate-test-runner",
            }),
            sink.clone(),
            None,
            Some(2),
        );
        let job_id = controller.submit(view_task(&["patients"]));
        wait_for_writes(&sink, 1).await;
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;

        match controller.get_status("t1", &job_id) {
            Some(JobStatus::Failed {
                message, status, ..
            }) => {
                assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
                assert_eq!(message, server_fault_message(&job_id));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert!(
            sink.read_shard(&job_id, "shard-0.ndjson").is_none(),
            "the shard written before the failure must be deleted"
        );
    }

    /// A `SofRunner` whose stream never ends: it pauses once after
    /// `pause_after` rows, then keeps producing, counting every row.
    struct EndlessRunner {
        pause_after: usize,
        produced: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl SofRunner for EndlessRunner {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            _view_definition: serde_json::Value,
            _filters: ViewFilters,
        ) -> Result<RowStream, SofError> {
            let pause_after = self.pause_after;
            let produced = Arc::clone(&self.produced);
            Ok(Box::pin(futures::stream::unfold(0usize, move |i| {
                let produced = Arc::clone(&produced);
                async move {
                    if i == pause_after {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    produced.fetch_add(1, Ordering::SeqCst);
                    Some((Ok(serde_json::json!({"id": format!("p{i}")})), i + 1))
                }
            })))
        }

        fn runner_name(&self) -> &'static str {
            "endless-test-runner"
        }
    }

    /// A failed shard write fails the job within [`CANCEL_CHECK_ROWS`] rows,
    /// not only once the next shard has filled.
    #[tokio::test]
    async fn a_failed_shard_write_is_noticed_before_the_next_shard_fills() {
        let shard_rows = 10 * CANCEL_CHECK_ROWS;
        let produced = Arc::new(AtomicUsize::new(0));
        let sink = ScriptedSink::new();
        sink.fail_writes_from.store(0, Ordering::SeqCst);
        let controller = InMemoryController::with_shard_rows(
            Arc::new(EndlessRunner {
                // The pause lets the first shard's write fail meanwhile.
                pause_after: shard_rows,
                produced: Arc::clone(&produced),
            }),
            sink.clone(),
            None,
            Some(shard_rows),
        );
        let job_id = controller.submit(view_task(&["patients"]));
        tokio::time::timeout(Duration::from_secs(15), async {
            while !matches!(
                controller.get_status("t1", &job_id),
                Some(JobStatus::Failed { .. })
            ) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the job fails");
        wait_until_idle(&controller, DEFAULT_MAX_CONCURRENCY).await;

        let produced = produced.load(Ordering::SeqCst);
        assert!(
            produced <= shard_rows + 2 * CANCEL_CHECK_ROWS,
            "{produced} rows read after a failed write"
        );
    }

    // ------------------------------------------------------------------
    // Prompt cancellation: a fired token stops a job that is waiting on its
    // runner, without waiting for the next row.
    // ------------------------------------------------------------------

    /// Sets its flag when dropped; moved into a row stream, it tells a test
    /// that the stream itself (and with it a real runner's channel) is gone.
    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    /// How long a [`HangingRunner`] hangs before giving up. Far longer than
    /// a prompt stop, yet bounded: the SQL Query leaf's stream is drained on
    /// a blocking thread the test runtime waits for at shutdown, so a stream
    /// that hung forever would turn a regression into a stalled test run
    /// instead of a failure.
    const HANG_BOUND: Duration = Duration::from_secs(5);

    /// A `SofRunner` whose stream yields `rows` rows and then no other one
    /// nor its end for [`HANG_BOUND`] — a PostgreSQL statement still sorting,
    /// say. With `hang_in_run_view` it does not even return its stream for
    /// that long.
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
            _view_definition: serde_json::Value,
            _filters: ViewFilters,
        ) -> Result<RowStream, SofError> {
            let _ = self.called.send(());
            if self.hang_in_run_view {
                tokio::time::sleep(HANG_BOUND).await;
            }
            let guard = DropFlag(Arc::clone(&self.dropped));
            let rows = (0..self.rows).map(|i| Ok(serde_json::json!({"id": format!("p{i}")})));
            let hang = futures::stream::once(tokio::time::sleep(HANG_BOUND))
                .filter_map(|()| std::future::ready(None));
            Ok(Box::pin(futures::stream::iter(rows).chain(hang).map(
                move |row| {
                    let _ = &guard;
                    row
                },
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

        let job_id = controller.submit(view_task(&["patients"]));
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

        let job_id = controller.submit(view_task(&["patients"]));
        hanging.called.recv().await.expect("the view runs");

        let took = cancel_and_time(&controller, &job_id).await;
        assert!(took < Duration::from_millis(500), "stopping took {took:?}");
        assert!(matches!(
            controller.get_status("t1", &job_id),
            Some(JobStatus::Cancelled { .. })
        ));
    }

    /// A SQL subject materializing a leaf ViewDefinition whose runner has
    /// gone quiet stops as promptly: the leaf runs through the same wrapped
    /// runner.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_sql_query_waiting_on_a_leaf_stops_promptly_once_cancelled() {
        let mut hanging = Hanging::new(5, false);
        let controller = InMemoryController::new(
            hanging.runner.clone(),
            InMemorySink::new("http://localhost"),
            None,
        );

        let job_id = controller.submit(sql_query_task(1000));
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

    /// The reaper removing a job fires its token too.
    #[test]
    fn reap_expired_fires_the_reaped_jobs_token() {
        let sink = InMemorySink::new("http://localhost");
        let jobs: DashMap<String, JobStatus> = DashMap::new();
        let job_tenants: DashMap<String, String> = DashMap::new();
        let signals: JobSignals = DashMap::new();
        let token = CancellationToken::new();
        let two_hours_ago = Utc::now() - chrono::Duration::hours(2);
        jobs.insert(
            "old".to_string(),
            JobStatus::Cancelled {
                cancelled_at: two_hours_ago,
            },
        );
        signals.insert("old".to_string(), token.clone());

        reap_expired(
            &jobs,
            &job_tenants,
            &signals,
            &sink,
            Duration::from_secs(3600),
        );

        assert!(token.is_cancelled());
        assert!(signals.is_empty());
    }

    // ------------------------------------------------------------------
    // Orphan sweep: a filesystem job directory with no manifest that no
    // job accounts for is deleted once older than the output TTL.
    // ------------------------------------------------------------------

    /// Creates `{root}/{name}` holding the given files.
    fn job_dir(root: &std::path::Path, name: &str, files: &[&str]) -> std::path::PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        for file in files {
            std::fs::write(dir.join(file), b"x").unwrap();
        }
        dir
    }

    #[test]
    fn filesystem_sweep_deletes_only_unknown_old_dirs_without_a_manifest() {
        let root = tempfile::tempdir().unwrap();
        let sink = crate::export::FilesystemSink::new(root.path(), "http://localhost");
        let orphan = Uuid::new_v4().to_string();
        let half_persisted = Uuid::new_v4().to_string();
        let known = Uuid::new_v4().to_string();
        let completed = Uuid::new_v4().to_string();
        job_dir(root.path(), &orphan, &["shard-0.ndjson"]);
        job_dir(
            root.path(),
            &half_persisted,
            &["shard-0.ndjson", "job.json.tmp"],
        );
        job_dir(root.path(), &known, &["shard-0.ndjson"]);
        job_dir(root.path(), &completed, &["shard-0.ndjson", "job.json"]);
        job_dir(root.path(), "operator-notes", &["readme.txt"]);
        std::fs::write(root.path().join(Uuid::new_v4().to_string()), b"x").unwrap();
        // UUID-named directories that are not this sink's job output — a
        // bulk-export tenant tree (`{tenant}/{job}/…`), a foreign file — are
        // never swept, however old.
        let nested = Uuid::new_v4().to_string();
        let nested_dir = job_dir(root.path(), &nested, &[]);
        job_dir(&nested_dir, "some-job", &["Patient.ndjson"]);
        let foreign = Uuid::new_v4().to_string();
        job_dir(root.path(), &foreign, &["shard-0.ndjson", "Patient.ndjson"]);
        let is_known = |jid: &str| jid == known;

        // Nothing is older than an hour yet.
        assert!(
            sink.sweep_orphans(&is_known, Duration::from_secs(3600))
                .is_empty()
        );

        std::thread::sleep(Duration::from_millis(50));
        let mut removed = sink.sweep_orphans(&is_known, Duration::from_millis(10));
        removed.sort();
        let mut expected = vec![orphan.clone(), half_persisted.clone()];
        expected.sort();
        assert_eq!(removed, expected);
        assert!(!root.path().join(&orphan).exists());
        assert!(!root.path().join(&half_persisted).exists());
        assert!(root.path().join(&known).exists(), "a known job is kept");
        assert!(root.path().join(&completed).exists(), "a manifest is kept");
        assert!(root.path().join("operator-notes").exists());
        assert!(
            nested_dir.join("some-job").join("Patient.ndjson").exists(),
            "a directory holding a subdirectory is kept"
        );
        assert!(
            root.path().join(&foreign).join("Patient.ndjson").exists(),
            "a directory holding a file that is not job output is kept"
        );
    }

    /// Controller construction sweeps orphans, and the reaper's sweep never
    /// deletes the directory of a job that is still running.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn controller_sweeps_orphans_but_never_a_running_jobs_dir() {
        let root = tempfile::tempdir().unwrap();
        let orphan = Uuid::new_v4().to_string();
        job_dir(root.path(), &orphan, &["shard-0.ndjson"]);
        std::thread::sleep(Duration::from_millis(50));

        let sink = crate::export::FilesystemSink::new(root.path(), "http://localhost");
        let mut gate = Gate::new(1, 2, false);
        let controller = InMemoryController::with_options(
            gate.runner.clone(),
            sink.clone(),
            None,
            Some(1),
            Some(CleanupConfig {
                output_ttl: Duration::from_millis(10),
                interval: Duration::from_secs(3600),
            }),
        );
        assert!(
            !root.path().join(&orphan).exists(),
            "construction deletes an old orphan"
        );

        // The running job writes its first shard and then parks at the gate.
        let job_id = controller.submit(view_task(&["patients"]));
        gate.reached().await;
        let shard = root.path().join(&job_id).join("shard-0.ndjson");
        tokio::time::timeout(Duration::from_secs(15), async {
            while !shard.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the first shard is written");
        tokio::time::sleep(Duration::from_millis(50)).await;

        sweep_orphans(&controller.jobs, &sink, Duration::from_millis(10));
        assert!(shard.exists(), "a running job's directory is never swept");

        gate.open();
        let files = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                match controller.get_status("t1", &job_id) {
                    Some(JobStatus::Completed { files, .. }) => return files,
                    _ => tokio::time::sleep(Duration::from_millis(10)).await,
                }
            }
        })
        .await
        .expect("the job completes");
        assert_eq!(files.len(), 2);
    }
}
