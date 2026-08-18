//! Database-backed [`ExportJobController`] for cluster deployments (#169).
//!
//! Jobs live in the shared [`ClusterJobStore`] (`cluster_jobs` table) instead
//! of a process-local map, so any instance behind the load balancer can serve
//! poll / cancel / download for a job submitted on another instance, and any
//! instance's worker can claim and run queued work under the store's lease +
//! fencing discipline.
//!
//! Split of responsibilities:
//! - [`DatabaseExportJobController`] — the handler-facing surface: durable
//!   `submit`, tenant-checked reads mapped onto [`JobStatus`], cancel.
//! - [`run_next_sof_export_job`] — claim one job and drive it to a terminal
//!   state (also the deterministic entry point for tests).
//! - [`spawn_sof_export_workers`] — the per-instance polling worker pool,
//!   modeled on the bulk-export pool in the `hfs` binary.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{debug, info, warn};

use helios_persistence::core::cluster_job_store::{
    ClusterJobId, ClusterJobLease, ClusterJobRecord, ClusterJobState, ClusterJobStore,
    ClusterLeaseError, JobKind, WorkerId,
};
use helios_persistence::core::sof_runner::{SofRunner, ViewFilters};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

use super::controller::{
    CompletedFile, ExportJobController, ExportTask, ExportWork, JobId, JobStatus, NamedSqlQuery,
    NamedView, SqlExportLimits, SqlTableSource,
};
use super::in_memory::{JobProgress, run_sqlquery_job, run_views_job};
use super::sink::ExportSink;

// ---------------------------------------------------------------------------
// Job payload — the serialized form of an ExportTask (minus the tenant, which
// the store records first-class and returns inside the claim lease).
// ---------------------------------------------------------------------------

/// `rusqlite`'s value enum is not serde-serializable, so SQL bindings travel
/// through this mirror. Blobs ride as JSON number arrays — SQL-query
/// parameters are small scalars in practice.
#[derive(Debug, Clone, Serialize, Deserialize)]
enum SqlValuePayload {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl From<&helios_sof::sqlquery::SqlValue> for SqlValuePayload {
    fn from(v: &helios_sof::sqlquery::SqlValue) -> Self {
        use helios_sof::sqlquery::SqlValue as V;
        match v {
            V::Null => Self::Null,
            V::Integer(i) => Self::Integer(*i),
            V::Real(r) => Self::Real(*r),
            V::Text(t) => Self::Text(t.clone()),
            V::Blob(b) => Self::Blob(b.clone()),
        }
    }
}

impl From<SqlValuePayload> for helios_sof::sqlquery::SqlValue {
    fn from(v: SqlValuePayload) -> Self {
        use helios_sof::sqlquery::SqlValue as V;
        match v {
            SqlValuePayload::Null => V::Null,
            SqlValuePayload::Integer(i) => V::Integer(i),
            SqlValuePayload::Real(r) => V::Real(r),
            SqlValuePayload::Text(t) => V::Text(t),
            SqlValuePayload::Blob(b) => V::Blob(b),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BoundParamPayload {
    name: String,
    value: SqlValuePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SqlQueryPayload {
    name: String,
    sql: String,
    tables: Vec<SqlTableSource>,
    bindings: Vec<BoundParamPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum WorkPayload {
    Views(Vec<NamedView>),
    SqlQueries {
        queries: Vec<SqlQueryPayload>,
        limits: SqlExportLimits,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExportPayload {
    work: WorkPayload,
    filters: ViewFilters,
    format: String,
    header: bool,
    client_tracking_id: Option<String>,
}

impl ExportPayload {
    fn from_task(task: &ExportTask) -> Self {
        let work = match &task.work {
            ExportWork::Views(views) => WorkPayload::Views(views.clone()),
            ExportWork::SqlQueries { queries, limits } => WorkPayload::SqlQueries {
                queries: queries
                    .iter()
                    .map(|q| SqlQueryPayload {
                        name: q.name.clone(),
                        sql: q.sql.clone(),
                        tables: q.tables.clone(),
                        bindings: q
                            .bindings
                            .iter()
                            .map(|b| BoundParamPayload {
                                name: b.name.clone(),
                                value: (&b.value).into(),
                            })
                            .collect(),
                    })
                    .collect(),
                limits: *limits,
            },
        };
        Self {
            work,
            filters: task.filters.clone(),
            format: task.format.clone(),
            header: task.header,
            client_tracking_id: task.client_tracking_id.clone(),
        }
    }

    fn into_task(self, tenant: TenantContext) -> ExportTask {
        let work = match self.work {
            WorkPayload::Views(views) => ExportWork::Views(views),
            WorkPayload::SqlQueries { queries, limits } => ExportWork::SqlQueries {
                queries: queries
                    .into_iter()
                    .map(|q| NamedSqlQuery {
                        name: q.name,
                        sql: q.sql,
                        tables: q.tables,
                        bindings: q
                            .bindings
                            .into_iter()
                            .map(|b| helios_sof::sqlquery::BoundParam {
                                name: b.name,
                                value: b.value.into(),
                            })
                            .collect(),
                    })
                    .collect(),
                limits,
            },
        };
        ExportTask {
            work,
            tenant,
            filters: self.filters,
            format: self.format,
            header: self.header,
            client_tracking_id: self.client_tracking_id,
        }
    }
}

/// Terminal result stored in `cluster_jobs.result` on completion.
#[derive(Debug, Serialize, Deserialize)]
struct ExportResult {
    files: Vec<CompletedFile>,
    total_rows: usize,
}

/// Store-facing tenant context for a handler-supplied tenant id. The store
/// only uses the id (tenant scoping); permissions were enforced upstream by
/// the extractors before the handler ran.
fn store_tenant(tenant_id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access())
}

fn progress_percent(progress: Option<&Value>) -> u8 {
    progress
        .and_then(|p| p.get("percent"))
        .and_then(Value::as_u64)
        .map(|p| p.min(99) as u8)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Controller
// ---------------------------------------------------------------------------

/// See [module docs](self).
pub struct DatabaseExportJobController<Sink: ExportSink> {
    store: Arc<dyn ClusterJobStore>,
    sink: Sink,
}

impl<Sink: ExportSink> DatabaseExportJobController<Sink> {
    /// Creates a controller over the shared job store and output sink.
    ///
    /// The sink must resolve to storage shared by every instance (S3, or a
    /// shared filesystem) — the cluster validator refuses per-instance-local
    /// sinks under `HFS_CLUSTER=true`.
    pub fn new(store: Arc<dyn ClusterJobStore>, sink: Sink) -> Self {
        Self { store, sink }
    }

    fn status_of(&self, record: &ClusterJobRecord) -> JobStatus {
        let submitted_at = record.created_at;
        match record.state {
            ClusterJobState::Queued | ClusterJobState::Running => JobStatus::Running {
                percent: progress_percent(record.progress.as_ref()),
                submitted_at,
            },
            ClusterJobState::Completed => {
                let result: ExportResult = record
                    .result
                    .clone()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or(ExportResult {
                        files: Vec::new(),
                        total_rows: 0,
                    });
                let payload: Option<ExportPayload> =
                    serde_json::from_value(record.payload.clone()).ok();
                JobStatus::Completed {
                    files: result.files,
                    submitted_at,
                    completed_at: record.finished_at.unwrap_or(submitted_at),
                    format: payload
                        .as_ref()
                        .map(|p| p.format.clone())
                        .unwrap_or_default(),
                    client_tracking_id: payload.and_then(|p| p.client_tracking_id),
                }
            }
            ClusterJobState::Failed => JobStatus::Failed {
                message: record
                    .error
                    .clone()
                    .unwrap_or_else(|| "export failed".into()),
                submitted_at,
                failed_at: record.finished_at.unwrap_or(submitted_at),
            },
            ClusterJobState::Cancelled => JobStatus::Cancelled {
                cancelled_at: record.finished_at.unwrap_or(submitted_at),
            },
        }
    }

    async fn record_for(&self, tenant_id: &str, job_id: &str) -> Option<ClusterJobRecord> {
        let tenant = store_tenant(tenant_id);
        match self
            .store
            .get_status(&tenant, &ClusterJobId::from_string(job_id))
            .await
        {
            Ok(rec) => rec,
            Err(e) => {
                warn!(job_id, error = %e, "cluster job store read failed");
                None
            }
        }
    }
}

#[async_trait::async_trait]
impl<Sink: ExportSink> ExportJobController for DatabaseExportJobController<Sink> {
    async fn submit(&self, task: ExportTask) -> JobId {
        let payload = serde_json::to_value(ExportPayload::from_task(&task))
            .expect("export payload serializes");
        match self
            .store
            .enqueue(&task.tenant, JobKind::SofExport, payload)
            .await
        {
            Ok(job_id) => job_id.to_string(),
            Err(e) => {
                // The trait's submit is infallible by contract (the handler
                // has already validated the request); a store outage here is
                // an operational failure. Return a job id that will poll as
                // unknown rather than panicking the handler.
                warn!(error = %e, "failed to enqueue export job");
                ClusterJobId::random().to_string()
            }
        }
    }

    async fn get_status(&self, tenant_id: &str, job_id: &str) -> Option<JobStatus> {
        let record = self.record_for(tenant_id, job_id).await?;
        Some(self.status_of(&record))
    }

    async fn cancel(&self, tenant_id: &str, job_id: &str) -> bool {
        let tenant = store_tenant(tenant_id);
        let found = match self
            .store
            .cancel(&tenant, &ClusterJobId::from_string(job_id))
            .await
        {
            Ok(found) => found,
            Err(e) => {
                warn!(job_id, error = %e, "cluster job cancel failed");
                return false;
            }
        };
        if found {
            // Reclaim partial output if the cancel actually landed (a job
            // that was already terminal keeps its output for the reaper).
            if let Some(rec) = self.record_for(tenant_id, job_id).await {
                if rec.state == ClusterJobState::Cancelled {
                    if let Err(e) = self.sink.delete_job(job_id) {
                        warn!(job_id, error = %e, "failed to delete cancelled export output");
                    }
                }
            }
        }
        found
    }

    async fn read_shard(&self, tenant_id: &str, job_id: &str, filename: &str) -> Option<Vec<u8>> {
        let record = self.record_for(tenant_id, job_id).await?;
        // Mirror the in-memory controller: cancelled/failed jobs serve
        // nothing even while output deletion is still draining.
        if matches!(
            record.state,
            ClusterJobState::Cancelled | ClusterJobState::Failed
        ) {
            return None;
        }
        self.sink.read_shard(job_id, filename)
    }

    async fn download_url(&self, tenant_id: &str, job_id: &str, filename: &str) -> Option<String> {
        // Tenant check first; then resolve freshly per call (S3 re-signs).
        self.record_for(tenant_id, job_id).await?;
        match self.sink.download_url(job_id, filename) {
            Ok(url) => Some(url),
            Err(e) => {
                warn!(job_id, filename, error = %e, "failed to resolve download URL");
                None
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

/// [`JobProgress`] over the shared store: persists a percent snapshot and
/// observes cross-instance cancellation between work units.
struct StoreProgress<'a> {
    store: &'a Arc<dyn ClusterJobStore>,
    lease: &'a ClusterJobLease,
}

#[async_trait::async_trait]
impl JobProgress for StoreProgress<'_> {
    async fn advance(&self, done: u32, total: u32) -> bool {
        let percent = ((done * 100) / total.max(1)).min(99);
        match self
            .store
            .update_progress(self.lease, json!({ "percent": percent }))
            .await
        {
            Ok(()) => {}
            Err(ClusterLeaseError::LeaseLost { .. }) => return false,
            Err(ClusterLeaseError::Storage(e)) => {
                // Transient store trouble must not kill a running export;
                // the heartbeat/fenced terminal write is the real guard.
                warn!(job_id = %self.lease.job_id, error = %e, "progress update failed");
            }
        }
        match self.store.cancel_requested(self.lease).await {
            Ok(requested) => !requested,
            Err(ClusterLeaseError::LeaseLost { .. }) => false,
            Err(ClusterLeaseError::Storage(e)) => {
                warn!(job_id = %self.lease.job_id, error = %e, "cancel_requested check failed");
                true
            }
        }
    }
}

/// Claims at most one queued SoF export job and drives it to a terminal
/// state. Returns whether a job was claimed.
///
/// This is the worker pool's unit of work, exposed so tests can execute a
/// claim/run cycle deterministically (no polling loops or sleeps).
pub async fn run_next_sof_export_job<Sink: ExportSink>(
    store: &Arc<dyn ClusterJobStore>,
    runner: &Arc<dyn SofRunner>,
    sink: &Sink,
    shard_rows: usize,
    worker_id: &WorkerId,
    lease_duration: Duration,
) -> Result<bool, String> {
    let claimed = store
        .claim_next(JobKind::SofExport, worker_id, lease_duration)
        .await
        .map_err(|e| format!("claim failed: {e}"))?;
    let Some((lease, payload)) = claimed else {
        return Ok(false);
    };
    let jid = lease.job_id.to_string();
    debug!(job_id = %jid, worker = %worker_id, "claimed SoF export job");

    // Renew the lease in the background while the job runs; `run_*_job`
    // checkpoints (progress + cancel) only between work units, which can be
    // far apart for large views.
    let heartbeat = {
        let store = Arc::clone(store);
        let lease = lease.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(15));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                if let Err(ClusterLeaseError::LeaseLost { .. }) = store.heartbeat(&lease).await {
                    debug!(job_id = %lease.job_id, "heartbeat: lease lost");
                    break;
                }
            }
        })
    };

    let task = match serde_json::from_value::<ExportPayload>(payload)
        .map(|p| p.into_task(lease.tenant.clone()))
    {
        Ok(task) => task,
        Err(e) => {
            heartbeat.abort();
            let msg = format!("invalid export payload: {e}");
            if let Err(err) = store.fail(&lease, &msg).await {
                warn!(job_id = %jid, error = %err, "failed to mark bad-payload job failed");
            }
            return Ok(true);
        }
    };

    let progress = StoreProgress {
        store,
        lease: &lease,
    };
    let outcome = match &task.work {
        ExportWork::Views(views) => {
            run_views_job(&jid, runner, sink, shard_rows, &task, views, &progress).await
        }
        ExportWork::SqlQueries { queries, limits } => {
            run_sqlquery_job(
                &jid, runner, sink, shard_rows, &task, queries, *limits, &progress,
            )
            .await
        }
    };
    heartbeat.abort();

    let terminal = match outcome {
        Ok((files, total_rows)) => {
            let result = serde_json::to_value(ExportResult { files, total_rows })
                .expect("export result serializes");
            store.complete(&lease, result).await
        }
        Err(message) => {
            warn!(job_id = %jid, error = %message, "SoF export job failed");
            store.fail(&lease, &message).await
        }
    };

    match terminal {
        Ok(()) => {}
        Err(ClusterLeaseError::LeaseLost { .. }) => {
            // Cancelled (or reclaimed) while we were running: our shards are
            // orphaned — reclaim them, mirroring the in-memory controller's
            // post-run cleanup.
            debug!(job_id = %jid, "terminal write fenced out (cancelled or reclaimed)");
            if let Err(e) = sink.delete_job(&jid) {
                warn!(job_id = %jid, error = %e, "failed to delete orphaned export output");
            }
        }
        Err(ClusterLeaseError::Storage(e)) => {
            warn!(job_id = %jid, error = %e, "failed to record export job outcome");
        }
    }

    Ok(true)
}

/// Reaps terminal SoF export jobs older than `ttl`, deleting their sink
/// output first. Idempotent — safe on every instance.
pub async fn reap_sof_export_jobs<Sink: ExportSink>(
    store: &Arc<dyn ClusterJobStore>,
    sink: &Sink,
    ttl: Duration,
) {
    let cutoff: DateTime<Utc> =
        Utc::now() - chrono::Duration::from_std(ttl).unwrap_or_else(|_| chrono::Duration::hours(1));
    match store
        .delete_terminal_before(JobKind::SofExport, cutoff)
        .await
    {
        Ok(deleted) => {
            for job_id in &deleted {
                if let Err(e) = sink.delete_job(job_id.as_str()) {
                    warn!(job_id = %job_id, error = %e, "failed to delete reaped export output");
                }
            }
            if !deleted.is_empty() {
                debug!(count = deleted.len(), "reaped terminal SoF export jobs");
            }
        }
        Err(e) => warn!(error = %e, "SoF export reaper failed"),
    }
}

/// Spawns the per-instance SoF export worker pool (plus one reaper task),
/// modeled on the bulk-export pool in the `hfs` binary: claim → run →
/// idle-sleep when the queue is empty.
pub fn spawn_sof_export_workers<Sink: ExportSink>(
    store: Arc<dyn ClusterJobStore>,
    runner: Arc<dyn SofRunner>,
    sink: Sink,
    worker_count: usize,
    shard_rows: usize,
    output_ttl: Duration,
    cleanup_interval: Duration,
) {
    const LEASE: Duration = Duration::from_secs(60);

    for n in 0..worker_count.max(1) {
        let store = Arc::clone(&store);
        let runner = Arc::clone(&runner);
        let sink = sink.clone();
        let worker_id = WorkerId::new(format!("sof-export-{n}-{}", uuid::Uuid::new_v4()));
        tokio::spawn(async move {
            info!(worker = %worker_id, "SoF export worker started");
            loop {
                match run_next_sof_export_job(&store, &runner, &sink, shard_rows, &worker_id, LEASE)
                    .await
                {
                    Ok(true) => {} // claim again immediately
                    Ok(false) => tokio::time::sleep(Duration::from_secs(2)).await,
                    Err(e) => {
                        warn!(worker = %worker_id, error = %e, "SoF export worker error");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        });
    }

    let cleanup_interval = cleanup_interval.max(Duration::from_secs(1));
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(cleanup_interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            reap_sof_export_jobs(&store, &sink, output_ttl).await;
        }
    });
}
