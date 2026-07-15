//! Unified cluster job store (design doc §4, strategy §8 Phase 1).
//!
//! One shared `cluster_jobs` table, discriminated by [`JobKind`], backs every
//! async job surface that is process-local today (SoF `$export` #169, search
//! reindex A2). Any instance can enqueue; any instance's worker can claim,
//! heartbeat, and finish — under the same lease + fencing-token discipline as
//! the bulk-export job store ([`crate::core::bulk_export_worker`]), whose
//! Postgres claim shape (`FOR UPDATE SKIP LOCKED` + token bump) this trait's
//! backend implementations clone.
//!
//! Deliberately NOT a generalization of the bulk-export traits: those are
//! hard-wired to `ExportJobId`/`ExportRequest`/`TypeExportProgress` and stay
//! as-is (resolved decision F3). Payload, progress, and result here are
//! opaque JSON so new [`JobKind`]s do not need schema changes.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::error::{StorageError, StorageResult};
use crate::tenant::TenantContext;

pub use crate::core::bulk_export_worker::WorkerId;

/// Identifier for a cluster job (a UUID string).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClusterJobId(String);

impl ClusterJobId {
    /// Wraps an existing id string.
    pub fn from_string(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Generates a fresh random job id.
    pub fn random() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// Returns the id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ClusterJobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The kind of work a cluster job carries; workers claim by kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobKind {
    /// SQL-on-FHIR async `$export` (#169).
    SofExport,
    /// Search parameter reindex (A2).
    Reindex,
}

impl JobKind {
    /// Stable storage discriminator.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SofExport => "sof-export",
            Self::Reindex => "reindex",
        }
    }

    /// Parses the storage discriminator.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "sof-export" => Some(Self::SofExport),
            "reindex" => Some(Self::Reindex),
            _ => None,
        }
    }
}

impl std::fmt::Display for JobKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Lifecycle state of a cluster job.
///
/// `queued → running → completed | failed | cancelled`; an expired-lease
/// `running` job is reclaimable (it goes back to `running` under the new
/// worker's bumped fencing token rather than through `queued`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterJobState {
    /// Enqueued, waiting for a worker.
    Queued,
    /// Claimed by a worker under a live (or expired-but-unreclaimed) lease.
    Running,
    /// Finished successfully; `result` is populated.
    Completed,
    /// Finished with an error; `error` is populated.
    Failed,
    /// Cancelled by a caller; workers observe `cancel_requested` and stop.
    Cancelled,
}

impl ClusterJobState {
    /// Stable storage discriminator.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Parses the storage discriminator.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    /// True for states no worker will touch again.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

impl std::fmt::Display for ClusterJobState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A caller-facing snapshot of one cluster job.
#[derive(Debug, Clone)]
pub struct ClusterJobRecord {
    /// The job id.
    pub id: ClusterJobId,
    /// What kind of work this is.
    pub kind: JobKind,
    /// Current lifecycle state.
    pub state: ClusterJobState,
    /// The enqueue-time work description (opaque to the store).
    pub payload: Value,
    /// Latest worker-reported progress snapshot, if any.
    pub progress: Option<Value>,
    /// Terminal result for `Completed` jobs.
    pub result: Option<Value>,
    /// Terminal error message for `Failed` jobs.
    pub error: Option<String>,
    /// True once a caller asked for cancellation.
    pub cancel_requested: bool,
    /// When the job was enqueued.
    pub created_at: DateTime<Utc>,
    /// When a worker first claimed it.
    pub started_at: Option<DateTime<Utc>>,
    /// When it reached a terminal state.
    pub finished_at: Option<DateTime<Utc>>,
}

/// A lease over one cluster job, held by exactly one worker at a time.
///
/// Same discipline as [`crate::core::bulk_export_worker::ExportJobLease`]:
/// leases expire unless heartbeated, and the `fencing_token` is bumped on
/// every claim so a zombie worker cannot mutate a job another worker now owns.
#[derive(Debug, Clone)]
pub struct ClusterJobLease {
    /// The leased job.
    pub job_id: ClusterJobId,
    /// The tenant the job belongs to.
    pub tenant: TenantContext,
    /// The worker holding the lease.
    pub worker_id: WorkerId,
    /// When the lease expires if not renewed.
    pub lease_expiry: DateTime<Utc>,
    /// Monotonically increasing token, bumped on every claim.
    pub fencing_token: u64,
}

/// Error returned by fenced cluster-job operations.
///
/// Not a reuse of [`crate::core::bulk_export_worker::LeaseError`] because that
/// variant carries a bulk-export `ExportJobId`; the discipline is identical.
#[derive(Debug)]
pub enum ClusterLeaseError {
    /// The lease was lost — another worker reclaimed the job, or it was
    /// cancelled. The caller MUST stop writing immediately.
    LeaseLost {
        /// The job whose lease was lost.
        job_id: ClusterJobId,
    },
    /// An underlying storage error.
    Storage(StorageError),
}

impl std::fmt::Display for ClusterLeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LeaseLost { job_id } => {
                write!(f, "cluster job {job_id} lease lost")
            }
            Self::Storage(e) => write!(f, "storage error: {e}"),
        }
    }
}

impl std::error::Error for ClusterLeaseError {}

impl From<StorageError> for ClusterLeaseError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

/// The unified cluster job store.
///
/// Tenancy contract: `get_status`/`list_jobs`/`cancel` are tenant-checked — a
/// job belonging to another tenant is indistinguishable from a missing one.
/// `claim_next` is deliberately cross-tenant (workers drain one shared queue);
/// the claimed lease carries the job's tenant for the worker to operate under.
///
/// Fencing contract: every worker-side mutation (`heartbeat`,
/// `update_progress`, `complete`, `fail`) is guarded by `worker_id` +
/// `fencing_token`, and the state-changing ones additionally require the job
/// to still be `running` — a guarded write affecting zero rows returns
/// [`ClusterLeaseError::LeaseLost`], so a zombie worker cannot resurrect a
/// cancelled job or clobber a reclaimed one.
#[async_trait]
pub trait ClusterJobStore: Send + Sync {
    /// Durably enqueues a job (state `queued`) and returns its id.
    async fn enqueue(
        &self,
        tenant: &TenantContext,
        kind: JobKind,
        payload: Value,
    ) -> StorageResult<ClusterJobId>;

    /// Atomically claims one eligible job of `kind` (`queued`, or `running`
    /// with an expired lease), bumping the fencing token. Returns the lease
    /// and the job payload, or `Ok(None)` when nothing is claimable.
    async fn claim_next(
        &self,
        kind: JobKind,
        worker_id: &WorkerId,
        lease_duration: Duration,
    ) -> StorageResult<Option<(ClusterJobLease, Value)>>;

    /// Renews a lease the worker still holds; returns the new expiry.
    async fn heartbeat(&self, lease: &ClusterJobLease) -> Result<DateTime<Utc>, ClusterLeaseError>;

    /// Releases a lease early (graceful shutdown), re-queueing the job.
    /// Best-effort: releasing an already-lost lease is not an error.
    async fn release(&self, lease: ClusterJobLease) -> StorageResult<()>;

    /// Tenant-checked read of one job; `None` when the job does not exist
    /// *for this tenant*.
    async fn get_status(
        &self,
        tenant: &TenantContext,
        job_id: &ClusterJobId,
    ) -> StorageResult<Option<ClusterJobRecord>>;

    /// Tenant-checked list of this tenant's jobs of `kind`, newest first.
    async fn list_jobs(
        &self,
        tenant: &TenantContext,
        kind: JobKind,
    ) -> StorageResult<Vec<ClusterJobRecord>>;

    /// Requests cancellation. A `queued` or `running` job transitions to
    /// `cancelled` immediately (pollers see it at once) and
    /// `cancel_requested` is set for the owning worker to observe; a job
    /// already terminal is left untouched. Returns whether the job exists
    /// for this tenant.
    async fn cancel(&self, tenant: &TenantContext, job_id: &ClusterJobId) -> StorageResult<bool>;

    /// Whether cancellation has been requested for the leased job. Fenced by
    /// `worker_id` + `fencing_token` (a lost lease reads as `LeaseLost`, on
    /// which the worker must stop anyway).
    async fn cancel_requested(&self, lease: &ClusterJobLease) -> Result<bool, ClusterLeaseError>;

    /// Replaces the job's progress snapshot. Fenced; requires `running`.
    async fn update_progress(
        &self,
        lease: &ClusterJobLease,
        progress: Value,
    ) -> Result<(), ClusterLeaseError>;

    /// Marks the job `completed` with its result. Fenced; requires `running`.
    async fn complete(
        &self,
        lease: &ClusterJobLease,
        result: Value,
    ) -> Result<(), ClusterLeaseError>;

    /// Marks the job `failed` with an error message. Fenced; requires
    /// `running`.
    async fn fail(
        &self,
        lease: &ClusterJobLease,
        error_message: &str,
    ) -> Result<(), ClusterLeaseError>;

    /// Deletes terminal jobs of `kind` that finished before `cutoff`,
    /// returning the removed ids so the caller can reap associated artifacts
    /// (e.g. export output files). Idempotent — safe for every instance to
    /// run on a timer.
    async fn delete_terminal_before(
        &self,
        kind: JobKind,
        cutoff: DateTime<Utc>,
    ) -> StorageResult<Vec<ClusterJobId>>;
}

/// Test support: an in-memory [`ClusterJobStore`] implementing the same
/// state-machine and fencing contract as the database backends.
///
/// This is NOT a cluster-safe production backend — two instances would each
/// have their own map (the "unsafe contract" `HFS_CLUSTER=true` refuses). It
/// exists so trait consumers (workers, controllers) can be unit-tested
/// without a database, and so the contract itself has a T1 reference model.
pub mod testing {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    struct FakeJob {
        tenant_id: String,
        kind: JobKind,
        state: ClusterJobState,
        payload: Value,
        progress: Option<Value>,
        result: Option<Value>,
        error: Option<String>,
        cancel_requested: bool,
        worker_id: Option<String>,
        lease_expiry: Option<DateTime<Utc>>,
        fencing_token: u64,
        created_at: DateTime<Utc>,
        started_at: Option<DateTime<Utc>>,
        finished_at: Option<DateTime<Utc>>,
        /// Monotonic enqueue sequence — deterministic claim order even when
        /// two jobs share a `created_at` timestamp.
        seq: u64,
    }

    impl FakeJob {
        fn record(&self, id: &str) -> ClusterJobRecord {
            ClusterJobRecord {
                id: ClusterJobId::from_string(id),
                kind: self.kind,
                state: self.state,
                payload: self.payload.clone(),
                progress: self.progress.clone(),
                result: self.result.clone(),
                error: self.error.clone(),
                cancel_requested: self.cancel_requested,
                created_at: self.created_at,
                started_at: self.started_at,
                finished_at: self.finished_at,
            }
        }

        fn holds_lease(&self, lease: &ClusterJobLease) -> bool {
            self.worker_id.as_deref() == Some(lease.worker_id.as_str())
                && self.fencing_token == lease.fencing_token
        }
    }

    /// See [module docs](self).
    #[derive(Default)]
    pub struct InMemoryClusterJobStore {
        jobs: Mutex<HashMap<String, FakeJob>>,
        next_seq: Mutex<u64>,
    }

    impl InMemoryClusterJobStore {
        /// Creates an empty store.
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl ClusterJobStore for InMemoryClusterJobStore {
        async fn enqueue(
            &self,
            tenant: &TenantContext,
            kind: JobKind,
            payload: Value,
        ) -> StorageResult<ClusterJobId> {
            let job_id = ClusterJobId::random();
            let seq = {
                let mut next = self.next_seq.lock().unwrap();
                *next += 1;
                *next
            };
            self.jobs.lock().unwrap().insert(
                job_id.as_str().to_string(),
                FakeJob {
                    tenant_id: tenant.tenant_id().as_str().to_string(),
                    kind,
                    state: ClusterJobState::Queued,
                    payload,
                    progress: None,
                    result: None,
                    error: None,
                    cancel_requested: false,
                    worker_id: None,
                    lease_expiry: None,
                    fencing_token: 0,
                    created_at: Utc::now(),
                    started_at: None,
                    finished_at: None,
                    seq,
                },
            );
            Ok(job_id)
        }

        async fn claim_next(
            &self,
            kind: JobKind,
            worker_id: &WorkerId,
            lease_duration: Duration,
        ) -> StorageResult<Option<(ClusterJobLease, Value)>> {
            let now = Utc::now();
            let lease_expiry = now
                + chrono::Duration::from_std(lease_duration)
                    .unwrap_or_else(|_| chrono::Duration::seconds(60));
            let mut jobs = self.jobs.lock().unwrap();
            let eligible = jobs
                .iter()
                .filter(|(_, j)| {
                    j.kind == kind
                        && (j.state == ClusterJobState::Queued
                            || (j.state == ClusterJobState::Running
                                && j.lease_expiry.is_none_or(|e| e < now)))
                })
                .min_by_key(|(_, j)| j.seq)
                .map(|(id, _)| id.clone());
            let Some(id) = eligible else {
                return Ok(None);
            };
            let job = jobs.get_mut(&id).expect("job present under lock");
            job.state = ClusterJobState::Running;
            job.worker_id = Some(worker_id.as_str().to_string());
            job.lease_expiry = Some(lease_expiry);
            job.fencing_token += 1;
            job.started_at.get_or_insert(now);
            Ok(Some((
                ClusterJobLease {
                    job_id: ClusterJobId::from_string(id),
                    tenant: TenantContext::new(
                        crate::tenant::TenantId::new(&job.tenant_id),
                        crate::tenant::TenantPermissions::full_access(),
                    ),
                    worker_id: worker_id.clone(),
                    lease_expiry,
                    fencing_token: job.fencing_token,
                },
                job.payload.clone(),
            )))
        }

        async fn heartbeat(
            &self,
            lease: &ClusterJobLease,
        ) -> Result<DateTime<Utc>, ClusterLeaseError> {
            let new_expiry = Utc::now() + chrono::Duration::seconds(60);
            let mut jobs = self.jobs.lock().unwrap();
            match jobs.get_mut(lease.job_id.as_str()) {
                Some(job) if job.holds_lease(lease) => {
                    job.lease_expiry = Some(new_expiry);
                    Ok(new_expiry)
                }
                _ => Err(ClusterLeaseError::LeaseLost {
                    job_id: lease.job_id.clone(),
                }),
            }
        }

        async fn release(&self, lease: ClusterJobLease) -> StorageResult<()> {
            let mut jobs = self.jobs.lock().unwrap();
            if let Some(job) = jobs.get_mut(lease.job_id.as_str()) {
                if job.holds_lease(&lease) && job.state == ClusterJobState::Running {
                    job.state = ClusterJobState::Queued;
                    job.worker_id = None;
                    job.lease_expiry = None;
                }
            }
            Ok(())
        }

        async fn get_status(
            &self,
            tenant: &TenantContext,
            job_id: &ClusterJobId,
        ) -> StorageResult<Option<ClusterJobRecord>> {
            let jobs = self.jobs.lock().unwrap();
            Ok(jobs
                .get(job_id.as_str())
                .filter(|j| j.tenant_id == tenant.tenant_id().as_str())
                .map(|j| j.record(job_id.as_str())))
        }

        async fn list_jobs(
            &self,
            tenant: &TenantContext,
            kind: JobKind,
        ) -> StorageResult<Vec<ClusterJobRecord>> {
            let jobs = self.jobs.lock().unwrap();
            let mut records: Vec<(u64, ClusterJobRecord)> = jobs
                .iter()
                .filter(|(_, j)| j.tenant_id == tenant.tenant_id().as_str() && j.kind == kind)
                .map(|(id, j)| (j.seq, j.record(id)))
                .collect();
            records.sort_by_key(|r| std::cmp::Reverse(r.0));
            Ok(records.into_iter().map(|(_, r)| r).collect())
        }

        async fn cancel(
            &self,
            tenant: &TenantContext,
            job_id: &ClusterJobId,
        ) -> StorageResult<bool> {
            let mut jobs = self.jobs.lock().unwrap();
            match jobs.get_mut(job_id.as_str()) {
                Some(job) if job.tenant_id == tenant.tenant_id().as_str() => {
                    if !job.state.is_terminal() {
                        job.state = ClusterJobState::Cancelled;
                        job.cancel_requested = true;
                        job.finished_at = Some(Utc::now());
                    }
                    Ok(true)
                }
                _ => Ok(false),
            }
        }

        async fn cancel_requested(
            &self,
            lease: &ClusterJobLease,
        ) -> Result<bool, ClusterLeaseError> {
            let jobs = self.jobs.lock().unwrap();
            match jobs.get(lease.job_id.as_str()) {
                Some(job) if job.holds_lease(lease) => Ok(job.cancel_requested),
                _ => Err(ClusterLeaseError::LeaseLost {
                    job_id: lease.job_id.clone(),
                }),
            }
        }

        async fn update_progress(
            &self,
            lease: &ClusterJobLease,
            progress: Value,
        ) -> Result<(), ClusterLeaseError> {
            let mut jobs = self.jobs.lock().unwrap();
            match jobs.get_mut(lease.job_id.as_str()) {
                Some(job) if job.holds_lease(lease) && job.state == ClusterJobState::Running => {
                    job.progress = Some(progress);
                    Ok(())
                }
                _ => Err(ClusterLeaseError::LeaseLost {
                    job_id: lease.job_id.clone(),
                }),
            }
        }

        async fn complete(
            &self,
            lease: &ClusterJobLease,
            result: Value,
        ) -> Result<(), ClusterLeaseError> {
            let mut jobs = self.jobs.lock().unwrap();
            match jobs.get_mut(lease.job_id.as_str()) {
                Some(job) if job.holds_lease(lease) && job.state == ClusterJobState::Running => {
                    job.state = ClusterJobState::Completed;
                    job.result = Some(result);
                    job.finished_at = Some(Utc::now());
                    Ok(())
                }
                _ => Err(ClusterLeaseError::LeaseLost {
                    job_id: lease.job_id.clone(),
                }),
            }
        }

        async fn fail(
            &self,
            lease: &ClusterJobLease,
            error_message: &str,
        ) -> Result<(), ClusterLeaseError> {
            let mut jobs = self.jobs.lock().unwrap();
            match jobs.get_mut(lease.job_id.as_str()) {
                Some(job) if job.holds_lease(lease) && job.state == ClusterJobState::Running => {
                    job.state = ClusterJobState::Failed;
                    job.error = Some(error_message.to_string());
                    job.finished_at = Some(Utc::now());
                    Ok(())
                }
                _ => Err(ClusterLeaseError::LeaseLost {
                    job_id: lease.job_id.clone(),
                }),
            }
        }

        async fn delete_terminal_before(
            &self,
            kind: JobKind,
            cutoff: DateTime<Utc>,
        ) -> StorageResult<Vec<ClusterJobId>> {
            let mut jobs = self.jobs.lock().unwrap();
            let doomed: Vec<String> = jobs
                .iter()
                .filter(|(_, j)| {
                    j.kind == kind
                        && j.state.is_terminal()
                        && j.finished_at.is_some_and(|f| f < cutoff)
                })
                .map(|(id, _)| id.clone())
                .collect();
            for id in &doomed {
                jobs.remove(id);
            }
            Ok(doomed.into_iter().map(ClusterJobId::from_string).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::testing::InMemoryClusterJobStore;
    use super::*;
    use crate::tenant::{TenantId, TenantPermissions};

    fn tenant(id: &str) -> TenantContext {
        TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
    }

    fn lease_of(claim: Option<(ClusterJobLease, Value)>) -> ClusterJobLease {
        claim.expect("a job should be claimable").0
    }

    const LEASE: Duration = Duration::from_secs(60);

    /// enqueue → claim → progress → complete, with the record reflecting
    /// every transition; tenant check hides the job from others.
    #[tokio::test]
    async fn t1_state_machine_happy_path() {
        let store = InMemoryClusterJobStore::new();
        let t = tenant("t1");
        let job_id = store
            .enqueue(&t, JobKind::SofExport, json!({"work": 1}))
            .await
            .unwrap();

        let rec = store.get_status(&t, &job_id).await.unwrap().unwrap();
        assert_eq!(rec.state, ClusterJobState::Queued);
        assert!(rec.started_at.is_none());

        // Tenant isolation: another tenant sees nothing.
        assert!(
            store
                .get_status(&tenant("other"), &job_id)
                .await
                .unwrap()
                .is_none()
        );

        let worker = WorkerId::new("w1");
        let (lease, payload) = store
            .claim_next(JobKind::SofExport, &worker, LEASE)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(lease.job_id, job_id);
        assert_eq!(lease.tenant.tenant_id().as_str(), "t1");
        assert_eq!(payload, json!({"work": 1}));
        assert_eq!(lease.fencing_token, 1);

        store
            .update_progress(&lease, json!({"pct": 50}))
            .await
            .unwrap();
        store.complete(&lease, json!({"files": []})).await.unwrap();

        let rec = store.get_status(&t, &job_id).await.unwrap().unwrap();
        assert_eq!(rec.state, ClusterJobState::Completed);
        assert_eq!(rec.progress, Some(json!({"pct": 50})));
        assert_eq!(rec.result, Some(json!({"files": []})));
        assert!(rec.started_at.is_some() && rec.finished_at.is_some());
    }

    /// Kinds are separate queues: a SofExport worker never claims Reindex.
    #[tokio::test]
    async fn t1_claim_respects_kind() {
        let store = InMemoryClusterJobStore::new();
        let t = tenant("t1");
        store
            .enqueue(&t, JobKind::Reindex, json!({}))
            .await
            .unwrap();
        assert!(
            store
                .claim_next(JobKind::SofExport, &WorkerId::new("w1"), LEASE)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .claim_next(JobKind::Reindex, &WorkerId::new("w1"), LEASE)
                .await
                .unwrap()
                .is_some()
        );
    }

    /// release → reclaim bumps the fencing token; every stale-lease write is
    /// refused with LeaseLost while the new holder is unaffected.
    #[tokio::test]
    async fn t1_fencing_after_release_and_reclaim() {
        let store = InMemoryClusterJobStore::new();
        let t = tenant("t1");
        store
            .enqueue(&t, JobKind::SofExport, json!({}))
            .await
            .unwrap();

        let worker_a = WorkerId::new("a");
        let lease_a = lease_of(
            store
                .claim_next(JobKind::SofExport, &worker_a, LEASE)
                .await
                .unwrap(),
        );
        store.release(lease_a.clone()).await.unwrap();

        let worker_b = WorkerId::new("b");
        let lease_b = lease_of(
            store
                .claim_next(JobKind::SofExport, &worker_b, LEASE)
                .await
                .unwrap(),
        );
        assert!(lease_b.fencing_token > lease_a.fencing_token);

        assert!(matches!(
            store.heartbeat(&lease_a).await,
            Err(ClusterLeaseError::LeaseLost { .. })
        ));
        assert!(matches!(
            store.update_progress(&lease_a, json!({})).await,
            Err(ClusterLeaseError::LeaseLost { .. })
        ));
        assert!(matches!(
            store.complete(&lease_a, json!({})).await,
            Err(ClusterLeaseError::LeaseLost { .. })
        ));
        assert!(matches!(
            store.cancel_requested(&lease_a).await,
            Err(ClusterLeaseError::LeaseLost { .. })
        ));

        store.heartbeat(&lease_b).await.unwrap();
        store.complete(&lease_b, json!({})).await.unwrap();
    }

    /// cancel flips a running job to Cancelled immediately; the owning worker
    /// observes cancel_requested and cannot resurrect the job with a terminal
    /// write. Cancelling a terminal job reports found but changes nothing.
    #[tokio::test]
    async fn t1_cancel_running_job_is_visible_and_final() {
        let store = InMemoryClusterJobStore::new();
        let t = tenant("t1");
        let job_id = store
            .enqueue(&t, JobKind::SofExport, json!({}))
            .await
            .unwrap();
        let lease = lease_of(
            store
                .claim_next(JobKind::SofExport, &WorkerId::new("w"), LEASE)
                .await
                .unwrap(),
        );

        assert!(!store.cancel_requested(&lease).await.unwrap());
        assert!(store.cancel(&t, &job_id).await.unwrap());

        // Pollers see it at once; the worker sees the request.
        let rec = store.get_status(&t, &job_id).await.unwrap().unwrap();
        assert_eq!(rec.state, ClusterJobState::Cancelled);
        assert!(store.cancel_requested(&lease).await.unwrap());

        // The worker cannot resurrect it.
        assert!(matches!(
            store.complete(&lease, json!({})).await,
            Err(ClusterLeaseError::LeaseLost { .. })
        ));
        assert!(matches!(
            store.fail(&lease, "late failure").await,
            Err(ClusterLeaseError::LeaseLost { .. })
        ));

        // Cancel again: found, still cancelled (no double transition).
        assert!(store.cancel(&t, &job_id).await.unwrap());
        // Wrong tenant: indistinguishable from missing.
        assert!(!store.cancel(&tenant("other"), &job_id).await.unwrap());
        // A cancelled job is not claimable.
        assert!(
            store
                .claim_next(JobKind::SofExport, &WorkerId::new("w2"), LEASE)
                .await
                .unwrap()
                .is_none()
        );
    }

    /// The reaper removes only terminal jobs older than the cutoff.
    #[tokio::test]
    async fn t1_reaper_removes_only_old_terminal_jobs() {
        let store = InMemoryClusterJobStore::new();
        let t = tenant("t1");

        let done = store
            .enqueue(&t, JobKind::SofExport, json!({}))
            .await
            .unwrap();
        let lease = lease_of(
            store
                .claim_next(JobKind::SofExport, &WorkerId::new("w"), LEASE)
                .await
                .unwrap(),
        );
        store.complete(&lease, json!({})).await.unwrap();

        let live = store
            .enqueue(&t, JobKind::SofExport, json!({}))
            .await
            .unwrap();

        let removed = store
            .delete_terminal_before(
                JobKind::SofExport,
                Utc::now() + chrono::Duration::seconds(1),
            )
            .await
            .unwrap();
        assert_eq!(removed, vec![done.clone()]);
        assert!(store.get_status(&t, &done).await.unwrap().is_none());
        assert!(store.get_status(&t, &live).await.unwrap().is_some());

        // Idempotent; kind-scoped (the live job is SofExport but queued, and
        // a terminal job of another kind would be out of scope entirely).
        assert!(
            store
                .delete_terminal_before(
                    JobKind::SofExport,
                    Utc::now() + chrono::Duration::seconds(1)
                )
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// list_jobs is tenant-scoped and newest-first.
    #[tokio::test]
    async fn t1_list_jobs_is_tenant_scoped_and_ordered() {
        let store = InMemoryClusterJobStore::new();
        let t = tenant("t1");
        let first = store
            .enqueue(&t, JobKind::Reindex, json!({"n": 1}))
            .await
            .unwrap();
        let second = store
            .enqueue(&t, JobKind::Reindex, json!({"n": 2}))
            .await
            .unwrap();
        store
            .enqueue(&tenant("other"), JobKind::Reindex, json!({}))
            .await
            .unwrap();
        store
            .enqueue(&t, JobKind::SofExport, json!({}))
            .await
            .unwrap();

        let listed = store.list_jobs(&t, JobKind::Reindex).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, second);
        assert_eq!(listed[1].id, first);
    }
}
