//! In-memory `ExportJobController` implementation.
//!
//! Each job runs inside a `tokio::spawn` task, bounded by a `Semaphore`.
//! Results are stored in a `DashMap<JobId, JobStatus>`.
//!
//! Two kinds of work are executed (see [`ExportWork`]):
//! - **Views** (`$viewdefinition-export`) — each named ViewDefinition is run
//!   through the `SofRunner` and its rows are sharded into output files.
//! - **SQL queries** (`$sqlquery-export`) — each named query's table sources
//!   are materialized into an in-memory SQLite engine via the `SofRunner`,
//!   the (pre-validated) SQL is executed, and the result rows are sharded
//!   into output files.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use futures::StreamExt;
use helios_persistence::core::sof_runner::SofRunner;
use helios_sof::sqlquery::{InMemorySqlEngine, QueryResult, TableSchema};
use tokio::sync::Semaphore;
use tracing::{debug, warn};
use uuid::Uuid;

use super::controller::{
    CompletedFile, ExportError, ExportJobController, ExportTask, ExportWork, JobId, JobStatus,
    NamedSqlQuery, NamedView, SqlExportLimits,
};
use super::planner;
use super::sink::ExportSink;

/// Default maximum number of concurrent export jobs.
pub const DEFAULT_MAX_CONCURRENCY: usize = 4;

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
    /// reaper is started; finished jobs are retained until the process exits.
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
        };
        if let Some(cfg) = cleanup {
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
            // The immediate first tick is a no-op (no jobs yet); skip waiting on
            // it so the cadence starts at `interval`.
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
                percent: 0,
                submitted_at,
            },
        );

        // Clone everything needed by the spawned task
        let jobs = Arc::clone(&self.jobs);
        let runner = Arc::clone(&self.runner);
        let sink = self.sink.clone();
        let semaphore = Arc::clone(&self.semaphore);
        let jid = job_id.clone();
        let shard_rows = self.shard_rows;

        tokio::spawn(async move {
            // Acquire concurrency permit (blocks if too many jobs running)
            let _permit = semaphore.acquire().await;

            let outcome = match &task.work {
                ExportWork::Views(views) => {
                    run_views_job(
                        &jobs,
                        &jid,
                        submitted_at,
                        &runner,
                        &sink,
                        shard_rows,
                        &task,
                        views,
                    )
                    .await
                }
                ExportWork::SqlQueries { queries, limits } => {
                    run_sqlquery_job(
                        &jobs,
                        &jid,
                        submitted_at,
                        &runner,
                        &sink,
                        shard_rows,
                        &task,
                        queries,
                        *limits,
                    )
                    .await
                }
            };

            match outcome {
                Ok((completed_files, total_rows)) => {
                    debug!(
                        job_id = %jid,
                        total_rows,
                        shards = completed_files.len(),
                        "export job completed"
                    );
                    set_status_if_running(
                        &jobs,
                        &jid,
                        JobStatus::Completed {
                            files: completed_files,
                            submitted_at,
                            completed_at: Utc::now(),
                            format: task.format.clone(),
                            client_tracking_id: task.client_tracking_id.clone(),
                        },
                    );
                }
                Err(message) => {
                    warn!(job_id = %jid, error = %message, "export job failed");
                    set_status_if_running(
                        &jobs,
                        &jid,
                        JobStatus::Failed {
                            message,
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
            // - Failed: the result URL returns 500 with no manifest, so the
            //   partial shards are unreachable and just waste storage.
            if matches!(
                jobs.get(&jid).as_deref(),
                Some(JobStatus::Cancelled { .. }) | Some(JobStatus::Failed { .. })
            ) {
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
        // results on cancel. Drop any shards written so far. A background task
        // still draining cleans up whatever it writes after this point when it
        // observes the Cancelled state (see `submit`).
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
        // A cancelled or failed job's shards must 404 even if deletion is still
        // draining, so a poll racing the cleanup never serves stale output.
        if matches!(
            self.jobs.get(job_id).as_deref(),
            Some(JobStatus::Cancelled { .. }) | Some(JobStatus::Failed { .. })
        ) {
            return None;
        }
        self.sink.read_shard(job_id, filename)
    }
}

// ============================================================================
// Cleanup reaper
// ============================================================================

/// Removes terminal jobs whose age exceeds `output_ttl`: deletes their output
/// via the sink and drops their status / tenant bookkeeping. `Running` jobs are
/// never touched ([`JobStatus::terminal_at`] returns `None` for them).
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
        if let Err(e) = sink.delete_job(&jid) {
            warn!(job_id = %jid, error = %e, "cleanup: failed to delete expired export output");
        }
        jobs.remove(&jid);
        job_tenants.remove(&jid);
        debug!(job_id = %jid, "cleanup: reclaimed expired export job");
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

/// Records job progress, capped at 99 while running so callers don't see
/// "100%" until the manifest is actually available from the status poll.
/// A job that is no longer `Running` (e.g. cancelled mid-run) is left
/// untouched — see [`set_status_if_running`].
fn record_progress(
    jobs: &DashMap<String, JobStatus>,
    jid: &str,
    submitted_at: DateTime<Utc>,
    done: u32,
    total: u32,
) {
    let percent = ((done * 100) / total.max(1)).min(99) as u8;
    set_status_if_running(
        jobs,
        jid,
        JobStatus::Running {
            percent,
            submitted_at,
        },
    );
}

/// `$viewdefinition-export` work: run each named ViewDefinition through the
/// `SofRunner` and shard its rows into output files.
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
) -> Result<(Vec<CompletedFile>, usize), String> {
    let view_count = views.len().max(1) as u32;
    let format = task.format.to_lowercase();
    let ext = ext_for(&format);

    let mut completed_files: Vec<CompletedFile> = Vec::new();
    let mut total_rows: usize = 0;

    // Spec: `view` is 1..* — run each ViewDefinition and produce its own set
    // of output shards. `output.name` in the manifest carries the per-view
    // name. Progress advances by `1/view_count` per view finished so the
    // X-Progress percentage tracks real work.
    for (view_idx, named) in views.iter().enumerate() {
        let stream = runner
            .run_view(&task.tenant, named.view.clone(), task.filters.clone())
            .await
            .map_err(|e| format!("view '{}': {e}", named.name))?;

        let rows: Vec<serde_json::Value> = stream
            .filter_map(|r| async move {
                match r {
                    Ok(v) => Some(v),
                    Err(e) => {
                        warn!("export row error (skipped): {e}");
                        None
                    }
                }
            })
            .collect()
            .await;

        total_rows += rows.len();

        // Spec: `output` is 0..*. Views with zero rows simply contribute no
        // `output` entries rather than emitting an empty shard with a
        // download URL pointing at zero bytes.
        for range in planner::plan(rows.len(), shard_rows) {
            let shard_slice = &rows[range];
            let row_count = shard_slice.len();

            let data = format_rows(shard_slice, &format, task.header)
                .map_err(|e| format!("view '{}': {e}", named.name))?;

            // Shard files are numbered by a running index across the whole
            // job so every shard gets a unique filename; for the
            // single-view case this matches the historical `shard-{N}`
            // numbering exactly.
            let shard_key = completed_files.len();
            let url = sink
                .write_shard(jid, shard_key, data, ext)
                .map_err(|e| format!("view '{}': {e}", named.name))?;

            debug!(job_id = %jid, view = %named.name, shard = shard_key, rows = row_count, url = %url, "shard written");
            completed_files.push(CompletedFile {
                view_name: named.name.clone(),
                url,
                row_count,
            });
        }

        record_progress(jobs, jid, submitted_at, (view_idx as u32) + 1, view_count);
    }

    Ok((completed_files, total_rows))
}

/// `$sqlquery-export` work: materialize each query's table sources via the
/// `SofRunner`, execute the pre-validated SQL, and shard the result rows into
/// output files.
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
) -> Result<(Vec<CompletedFile>, usize), String> {
    let query_count = queries.len().max(1) as u32;
    let format = task.format.to_lowercase();
    let ext = ext_for(&format);

    let mut completed_files: Vec<CompletedFile> = Vec::new();
    let mut total_rows: usize = 0;

    for (query_idx, query) in queries.iter().enumerate() {
        let result = execute_sql_query(runner, task, query, limits)
            .await
            .map_err(|e| format!("query '{}': {e}", query.name))?;

        total_rows += result.rows.len();

        for range in planner::plan(result.rows.len(), shard_rows) {
            let row_count = range.len();
            let data = format_query_rows(&result, range, &format, task.header)
                .map_err(|e| format!("query '{}': {e}", query.name))?;

            let shard_key = completed_files.len();
            let url = sink
                .write_shard(jid, shard_key, data, ext)
                .map_err(|e| format!("query '{}': {e}", query.name))?;

            debug!(job_id = %jid, query = %query.name, shard = shard_key, rows = row_count, url = %url, "shard written");
            completed_files.push(CompletedFile {
                view_name: query.name.clone(),
                url,
                row_count,
            });
        }

        record_progress(jobs, jid, submitted_at, (query_idx as u32) + 1, query_count);
    }

    Ok((completed_files, total_rows))
}

/// Materializes a query's table sources and executes its SQL, enforcing the
/// same row caps and timeout as the synchronous `$sqlquery-run` operation.
/// The export operations emit flat formats only (csv/ndjson/parquet/json), so
/// the result's JSON cell values feed `format_output` directly.
async fn execute_sql_query(
    runner: &Arc<dyn SofRunner>,
    task: &ExportTask,
    query: &NamedSqlQuery,
    limits: SqlExportLimits,
) -> Result<QueryResult, ExportError> {
    let mut engine = InMemorySqlEngine::open().map_err(|e| ExportError::Runner(e.to_string()))?;

    let mut schemas: Vec<TableSchema> = Vec::with_capacity(query.tables.len());
    for table in &query.tables {
        let schema = TableSchema::from_view_definition(&table.view);
        engine
            .create_table(&table.label, &schema)
            .map_err(|e| ExportError::Runner(e.to_string()))?;

        let stream = runner
            .run_view(&task.tenant, table.view.clone(), task.filters.clone())
            .await
            .map_err(|e| {
                ExportError::Runner(format!(
                    "table '{}' failed to materialize: {e}",
                    table.label
                ))
            })?;
        let stream = stream.map(|r| r.map_err(|e| e.to_string()));
        engine
            .insert_rows(
                &table.label,
                &schema,
                Box::pin(stream),
                limits.max_source_rows_per_vd,
            )
            .await
            .map_err(|e| ExportError::Runner(e.to_string()))?;
        schemas.push(schema);
    }

    // Run the user SQL on a blocking thread with a watchdog timeout, exactly
    // like the synchronous `$sqlquery-run` path.
    let interrupt = engine.interrupt_handle();
    let timeout_secs = limits.timeout_secs;
    let watchdog = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(timeout_secs)).await;
        interrupt.interrupt();
    });
    let sql = query.sql.clone();
    let bindings = query.bindings.clone();
    let max_rows = limits.max_rows;
    let exec_result =
        tokio::task::spawn_blocking(move || engine.execute_select(&sql, &bindings, max_rows)).await;
    watchdog.abort();

    let result = match exec_result {
        Ok(Ok(r)) => r,
        Ok(Err(e)) if e.to_string().contains("interrupted") => {
            return Err(ExportError::Runner(format!(
                "query exceeded {timeout_secs}s timeout"
            )));
        }
        Ok(Err(e)) => return Err(ExportError::Runner(e.to_string())),
        Err(join_err) => {
            return Err(ExportError::Runner(format!(
                "sqlquery worker panicked: {join_err}"
            )));
        }
    };

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
/// `helios_sof::format_output` (matching the `$sqlquery-run` bytes). The
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
    // (mirrors the `$sqlquery-run` handler).
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::sink::InMemorySink;
    use async_trait::async_trait;
    use helios_persistence::core::sof_runner::{RowStream, SofError, ViewFilters};
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
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
            work: ExportWork::Views(vec![NamedView {
                name: "patients".to_string(),
                view: serde_json::json!({
                    "resourceType": "ViewDefinition",
                    "resource": "Patient",
                    "status": "active",
                    "select": [{"column": [{"name": "id", "path": "id"}]}]
                }),
            }]),
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
            work: ExportWork::Views(vec![NamedView {
                name: "patients".to_string(),
                view: serde_json::json!({
                    "resourceType": "ViewDefinition",
                    "resource": "Patient",
                    "status": "active",
                    "select": [{"column": [{"name": "id", "path": "id"}]}]
                }),
            }]),
            tenant,
            filters: ViewFilters::default(),
            format: "ndjson".to_string(),
            header: true,
            client_tracking_id: None,
        });

        // Simulate a shard the running job had already streamed out.
        sink.write_shard(&job_id, 0, b"{\"id\":\"a\"}\n".to_vec(), "ndjson")
            .unwrap();
        assert!(
            controller
                .read_shard("t1", &job_id, "shard-0.ndjson")
                .is_some(),
            "shard should be downloadable while the job is running"
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
                percent: 10,
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
}
