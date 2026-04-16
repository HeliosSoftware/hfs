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
            runner,
            sink,
            semaphore: Arc::new(Semaphore::new(concurrency)),
            shard_rows: shard_rows.unwrap_or(planner::DEFAULT_SHARD_ROWS),
        }
    }
}

impl<Sink: ExportSink + 'static> ExportJobController for InMemoryController<Sink> {
    fn submit(&self, task: ExportTask) -> JobId {
        let job_id = Uuid::new_v4().to_string();
        let submitted_at = Utc::now();

        self.jobs.insert(
            job_id.clone(),
            JobStatus::Running {
                progress: "starting".to_string(),
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

            // Update progress
            jobs.insert(
                jid.clone(),
                JobStatus::Running {
                    progress: "running view".to_string(),
                    submitted_at,
                },
            );

            // Run the view
            let stream = match runner
                .run_view(
                    &task.tenant,
                    task.view_definition.clone(),
                    task.filters.clone(),
                )
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    warn!(job_id = %jid, error = %e, "export job failed: run_view error");
                    jobs.insert(
                        jid,
                        JobStatus::Failed {
                            message: e.to_string(),
                            submitted_at,
                        },
                    );
                    return;
                }
            };

            // Collect all rows
            let format = task.format.to_lowercase();
            let ext = if format == "csv" {
                "csv"
            } else if format == "parquet" {
                "parquet"
            } else {
                "ndjson"
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

            let total_rows = rows.len();

            // Split into shards using the planner
            let ranges = planner::plan(total_rows, shard_rows);
            // At least one shard (even for empty result sets)
            let ranges = if ranges.is_empty() {
                // Empty result set: produce one empty shard so the manifest
                // always contains at least one output entry.
                let empty: std::ops::Range<usize> = 0..0;
                vec![empty]
            } else {
                ranges
            };

            let mut completed_files: Vec<CompletedFile> = Vec::with_capacity(ranges.len());

            for (shard_idx, range) in ranges.into_iter().enumerate() {
                let shard_rows_slice = &rows[range.clone()];
                let row_count = shard_rows_slice.len();

                let data = match format_rows(shard_rows_slice, &format) {
                    Ok(d) => d,
                    Err(e) => {
                        warn!(job_id = %jid, shard = shard_idx, error = %e, "export shard serialization failed");
                        jobs.insert(
                            jid,
                            JobStatus::Failed {
                                message: e.to_string(),
                                submitted_at,
                            },
                        );
                        return;
                    }
                };

                let url = match sink.write_shard(&jid, shard_idx, data, ext) {
                    Ok(u) => u,
                    Err(e) => {
                        warn!(job_id = %jid, shard = shard_idx, error = %e, "export shard write failed");
                        jobs.insert(
                            jid,
                            JobStatus::Failed {
                                message: e.to_string(),
                                submitted_at,
                            },
                        );
                        return;
                    }
                };

                debug!(job_id = %jid, shard = shard_idx, rows = row_count, url = %url, "shard written");
                completed_files.push(CompletedFile { url, row_count });
            }

            debug!(job_id = %jid, total_rows, shards = completed_files.len(), "export job completed");

            jobs.insert(
                jid,
                JobStatus::Completed {
                    files: completed_files,
                    submitted_at,
                    completed_at: Utc::now(),
                },
            );
        });

        job_id
    }

    fn get_status(&self, job_id: &str) -> Option<JobStatus> {
        self.jobs.get(job_id).map(|v| v.clone())
    }

    fn cancel(&self, job_id: &str) -> bool {
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

    fn read_shard(&self, job_id: &str, filename: &str) -> Option<Vec<u8>> {
        self.sink.read_shard(job_id, filename)
    }
}

// ============================================================================
// Row serialization helpers
// ============================================================================

fn format_rows(rows: &[serde_json::Value], format: &str) -> Result<Vec<u8>, ExportError> {
    match format {
        "csv" => format_csv(rows),
        "parquet" => format_parquet(rows),
        _ => format_ndjson(rows),
    }
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

fn format_csv(rows: &[serde_json::Value]) -> Result<Vec<u8>, ExportError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    // Collect column names from the first row
    let cols: Vec<String> = rows[0]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();

    let mut out = Vec::new();

    // Header
    out.extend_from_slice(cols.join(",").as_bytes());
    out.push(b'\n');

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
