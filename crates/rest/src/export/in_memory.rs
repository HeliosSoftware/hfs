//! In-memory `ExportJobController` implementation.
//!
//! Each job runs inside a `tokio::spawn` task, bounded by a `Semaphore`.
//! Results are stored in a `DashMap<JobId, JobStatus>`.

use std::sync::Arc;

use chrono::Utc;
use dashmap::DashMap;
use futures::StreamExt;
use helios_persistence::core::sof_runner::SofRunner;
use tokio::sync::Semaphore;
use tracing::{debug, warn};
use uuid::Uuid;

use super::controller::{
    CompletedFile, ExportError, ExportJobController, ExportTask, JobId, JobStatus,
};
use super::planner;
use super::sink::ExportSink;

/// Default maximum number of concurrent export jobs.
pub const DEFAULT_MAX_CONCURRENCY: usize = 4;

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

    /// Like [`new`](Self::new) but with an explicit shard row limit.
    pub fn with_shard_rows(
        runner: Arc<dyn SofRunner>,
        sink: Sink,
        max_concurrency: Option<usize>,
        shard_rows: Option<usize>,
    ) -> Self {
        let concurrency = max_concurrency.unwrap_or(DEFAULT_MAX_CONCURRENCY);
        Self {
            jobs: Arc::new(DashMap::new()),
            job_tenants: Arc::new(DashMap::new()),
            runner,
            sink,
            semaphore: Arc::new(Semaphore::new(concurrency)),
            shard_rows: shard_rows.unwrap_or(planner::DEFAULT_SHARD_ROWS),
        }
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

            let view_count = task.views.len().max(1) as u32;

            let format = task.format.to_lowercase();
            let ext = match format.as_str() {
                "csv" => "csv",
                "parquet" => "parquet",
                "json" => "json",
                _ => "ndjson",
            };

            let mut completed_files: Vec<CompletedFile> = Vec::new();
            let mut total_rows: usize = 0;

            // Spec: `view` is 1..* — run each ViewDefinition and produce its
            // own set of output shards. `output.name` in the manifest carries
            // the per-view name. Progress advances by `1/view_count` per view
            // finished so the X-Progress percentage tracks real work.
            for (view_idx, named) in task.views.iter().enumerate() {
                let stream = match runner
                    .run_view(&task.tenant, named.view.clone(), task.filters.clone())
                    .await
                {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(job_id = %jid, view = %named.name, error = %e, "export job failed: run_view error");
                        jobs.insert(
                            jid.clone(),
                            JobStatus::Failed {
                                message: e.to_string(),
                                submitted_at,
                            },
                        );
                        return;
                    }
                };

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

                // Spec: `output` is 0..*. Views with zero rows simply
                // contribute no `output` entries rather than emitting an
                // empty shard with a download URL pointing at zero bytes.
                let ranges = planner::plan(rows.len(), shard_rows);

                for (shard_idx, range) in ranges.into_iter().enumerate() {
                    let shard_rows_slice = &rows[range.clone()];
                    let row_count = shard_rows_slice.len();

                    let data = match format_rows(shard_rows_slice, &format, task.header) {
                        Ok(d) => d,
                        Err(e) => {
                            warn!(job_id = %jid, view = %named.name, shard = shard_idx, error = %e, "export shard serialization failed");
                            jobs.insert(
                                jid.clone(),
                                JobStatus::Failed {
                                    message: e.to_string(),
                                    submitted_at,
                                },
                            );
                            return;
                        }
                    };

                    // Use the view name as the shard's logical name when there
                    // is more than one view; for the single-view case keep the
                    // existing shard naming (`shard-{N}.{ext}`) for back-compat
                    // with sinks that derive filenames from this index.
                    let shard_key = if task.views.len() == 1 {
                        shard_idx
                    } else {
                        // Encode `view_name + shard_idx` into the index space the
                        // sink uses by using a stable hash-ish scheme. Most sinks
                        // serialize the shard index into the filename so we use
                        // a composite key. Concretely we prefix the per-view
                        // filename via the sink's standard `shard-{N}` scheme,
                        // counting offsets across views.
                        completed_files.len() + shard_idx
                    };

                    let url = match sink.write_shard(&jid, shard_key, data, ext) {
                        Ok(u) => u,
                        Err(e) => {
                            warn!(job_id = %jid, view = %named.name, shard = shard_idx, error = %e, "export shard write failed");
                            jobs.insert(
                                jid.clone(),
                                JobStatus::Failed {
                                    message: e.to_string(),
                                    submitted_at,
                                },
                            );
                            return;
                        }
                    };

                    debug!(job_id = %jid, view = %named.name, shard = shard_idx, rows = row_count, url = %url, "shard written");
                    completed_files.push(CompletedFile {
                        view_name: named.name.clone(),
                        url,
                        row_count,
                    });
                }

                // After this view's shards are written, bump the percentage.
                // Capped at 99 while running so callers don't see "100%" until
                // the manifest is actually available at the result URL.
                let views_done = (view_idx as u32) + 1;
                let percent = ((views_done * 100) / view_count).min(99) as u8;
                jobs.insert(
                    jid.clone(),
                    JobStatus::Running {
                        percent,
                        submitted_at,
                    },
                );
            }

            debug!(
                job_id = %jid,
                total_rows,
                shards = completed_files.len(),
                views = task.views.len(),
                "export job completed"
            );

            jobs.insert(
                jid,
                JobStatus::Completed {
                    files: completed_files,
                    submitted_at,
                    completed_at: Utc::now(),
                    format: task.format.clone(),
                    client_tracking_id: task.client_tracking_id.clone(),
                },
            );
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
        if let Some(mut entry) = self.jobs.get_mut(job_id) {
            match &*entry {
                JobStatus::Running { .. } => {
                    *entry = JobStatus::Cancelled;
                    true
                }
                // Already done/failed/cancelled — return true (found it)
                _ => true,
            }
        } else {
            false
        }
    }

    fn read_shard(&self, tenant_id: &str, job_id: &str, filename: &str) -> Option<Vec<u8>> {
        if !self.tenant_matches(tenant_id, job_id) {
            return None;
        }
        self.sink.read_shard(job_id, filename)
    }
}

// ============================================================================
// Row serialization helpers
// ============================================================================

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
