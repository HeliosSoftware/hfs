//! Worker-facing traits for bulk export job execution.
//!
//! These traits are *not* part of the REST-facing [`BulkExportStorage`] surface
//! — they are what the export worker uses to claim jobs and persist progress
//! under a heartbeated, fencing-token-guarded lease.
//!
//! [`BulkExportStorage`]: crate::core::bulk_export::BulkExportStorage

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::core::bulk_export::{
    BulkExportStorage, ExportDataProvider, ExportJobId, ExportLevel, ExportRequest, ExportStatus,
    GroupExportProvider, PatientExportProvider, TypeExportProgress,
};
use crate::core::bulk_export_output::{ExportOutputStore, ExportPartKey, FinalizedPart};
use crate::core::search::SearchProvider;
use crate::error::{BulkExportError, StorageError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

/// Identifier for an export worker instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkerId(String);

impl WorkerId {
    /// Creates a worker ID from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Generates a fresh random worker ID.
    pub fn random() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// Returns the ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for WorkerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A lease over a single export job, held by exactly one worker at a time.
///
/// Leases expire; if the holding worker does not heartbeat before
/// `lease_expiry`, the lease is reclaimable. The `fencing_token` is bumped on
/// every claim so a zombie worker cannot mutate a job another worker now owns.
#[derive(Debug, Clone)]
pub struct ExportJobLease {
    /// The leased job.
    pub job_id: ExportJobId,
    /// The tenant the job belongs to.
    pub tenant: TenantContext,
    /// The worker holding the lease.
    pub worker_id: WorkerId,
    /// When the lease expires if not renewed.
    pub lease_expiry: DateTime<Utc>,
    /// Monotonically increasing token, bumped on every claim.
    pub fencing_token: u64,
    /// How long a renewal extends the lease for.
    ///
    /// Carried on the lease so a heartbeat extends it by the duration the job
    /// was claimed under, rather than by a constant the backend picked: a
    /// deployment that raises `HFS_BULK_EXPORT_LEASE_DURATION` for slow
    /// batches would otherwise see every renewal silently shrink the lease
    /// back to the hardcoded value (#1152).
    pub lease_duration: Duration,
}

/// Calculates an export lease expiry from one operation timestamp.
///
/// Claims and heartbeats share this conversion so the stored expiry and
/// heartbeat timestamp describe exactly the duration the job was claimed for.
/// The fallback preserves the existing behavior for durations that cannot be
/// represented by `chrono`.
pub(crate) fn export_lease_expiry(now: DateTime<Utc>, lease_duration: Duration) -> DateTime<Utc> {
    now + chrono::Duration::from_std(lease_duration)
        .unwrap_or_else(|_| chrono::Duration::seconds(60))
}

impl ExportJobLease {
    /// The expiry a renewal issued now should set.
    pub fn renewed_expiry(&self) -> DateTime<Utc> {
        export_lease_expiry(Utc::now(), self.lease_duration)
    }
}

/// Error returned by fenced worker-storage operations.
#[derive(Debug)]
pub enum LeaseError {
    /// The lease was lost — another worker reclaimed the job. The caller MUST
    /// stop writing immediately.
    LeaseLost {
        /// The job whose lease was lost.
        job_id: ExportJobId,
    },
    /// An underlying storage error.
    Storage(StorageError),
}

impl std::fmt::Display for LeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LeaseLost { job_id } => {
                write!(
                    f,
                    "export job {job_id} lease lost (reclaimed by another worker)"
                )
            }
            Self::Storage(e) => write!(f, "storage error: {e}"),
        }
    }
}

impl std::error::Error for LeaseError {}

impl From<StorageError> for LeaseError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

/// The worker's view of a claimed job: everything needed to (re)run it.
#[derive(Debug, Clone)]
pub struct WorkerJobView {
    /// The original export request.
    pub request: ExportRequest,
    /// The export level.
    pub level: ExportLevel,
    /// Server wall-clock frozen at kickoff.
    pub transaction_time: DateTime<Utc>,
    /// The FHIR version the export runs against.
    pub fhir_version: helios_fhir::FhirVersion,
    /// Already-persisted per-type progress, for resuming after a crash.
    pub type_progress: Vec<TypeExportProgress>,
    /// The authenticated principal that kicked the export off, if any.
    ///
    /// Read from the job row rather than passed in, so the terminal audit event
    /// still names the requester when the job is picked up by a different worker
    /// (or a different process) than the one that accepted it.
    pub owner_subject: Option<String>,
}

/// Strategy for atomically claiming the next available export job.
///
/// Each backend reaches for its native primitive — `SELECT … FOR UPDATE SKIP
/// LOCKED` on Postgres, a process-local mutex on SQLite.
#[async_trait]
pub trait ExportClaimStrategy: Send + Sync {
    /// Atomically transitions one eligible job (`accepted`, or `in_progress`
    /// with an expired lease) to held-by-this-worker, bumping the fencing
    /// token and the job's claim count. Returns `Ok(None)` when no job is
    /// available.
    ///
    /// `max_attempts` caps how many times one job may be claimed. A job that
    /// would exceed it is not handed out again: it is retired with
    /// [`abandoned_export_message`] as its error, and the scan moves on to the
    /// next eligible job (#1041).
    ///
    /// Re-claiming an `in_progress` job **discards the previous attempt's
    /// progress and file rows**, atomically with the token bump: the worker
    /// restarts `part_index` at 0 on every attempt, so a resumed run would
    /// otherwise overwrite the parts the dead worker wrote and lose their
    /// resources without failing the job (#1041). Output artifacts are left on
    /// disk/S3 for the TTL sweep — see the backend implementations for why
    /// unlinking them under a zombie worker is worse than orphaning them.
    async fn claim_next(
        &self,
        worker_id: &WorkerId,
        lease_duration: Duration,
        max_attempts: u32,
    ) -> StorageResult<Option<ExportJobLease>>;

    /// Renews a lease the worker still holds; returns the new expiry, or
    /// `LeaseError::LeaseLost` if the job was reclaimed.
    async fn heartbeat(&self, lease: &ExportJobLease) -> Result<DateTime<Utc>, LeaseError>;

    /// Releases a lease early (graceful shutdown). Best-effort: the job goes
    /// back to `accepted` for the next worker to pick up.
    ///
    /// **Currently unused** — nothing in the workspace calls it; the worker
    /// loop lets a shutdown lease lapse instead. Before wiring up a caller,
    /// note that the implementations do *not* give back the attempt they
    /// consumed (see `max_attempts` on [`Self::claim_next`]): a job released
    /// and re-claimed still burns one of its attempts, so a few rolling
    /// restarts would retire a perfectly healthy export. A caller must first
    /// make `release` decrement `attempts` — fenced on `worker_id` +
    /// `fencing_token`, so a zombie cannot spend another worker's attempt.
    ///
    /// The same caller also has to deal with the released job's half-written
    /// rows: it goes back as `accepted`, which is the one status
    /// [`Self::claim_next`] does *not* wipe, so the next attempt would resume
    /// from the cursor and overwrite the parts already recorded — the very
    /// loss the wipe exists to prevent (#1041). `release` must therefore clear
    /// the job's progress and file rows itself, or leave the job
    /// `in_progress` with a lapsed lease.
    async fn release(&self, lease: ExportJobLease) -> StorageResult<()>;
}

/// Worker-owned mutations of job state.
///
/// **Every method is fenced** by `worker_id` + `fencing_token`: a guarded
/// mutation affecting zero rows returns `LeaseError::LeaseLost`, so a zombie
/// worker cannot corrupt progress, file rows, or terminal status after its
/// job has been reclaimed.
#[async_trait]
pub trait ExportWorkerStorage: Send + Sync {
    /// Loads the claimed job's request, level, frozen metadata and persisted
    /// per-type progress (for resume). Fenced.
    async fn get_export_job_for_worker(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
    ) -> Result<WorkerJobView, LeaseError>;

    /// Marks the job `in_progress` (sets `started_at` if unset). Fenced.
    async fn mark_export_in_progress(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
    ) -> Result<(), LeaseError>;

    /// Idempotent upsert of per-type progress (cursor + counts). Fenced.
    async fn update_export_type_progress(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
        progress: &TypeExportProgress,
    ) -> Result<(), LeaseError>;

    /// Records which resource type the worker is about to write and how far
    /// through the type list it is. `current_type = None` clears the marker
    /// (the worker calls it that way once every type is done). Fenced.
    #[allow(clippy::too_many_arguments)]
    async fn set_export_current_type(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
        current_type: Option<&str>,
        types_done: u32,
        types_total: u32,
    ) -> Result<(), LeaseError>;

    /// Idempotent upsert of a finalized output/error file row. Fenced.
    async fn record_export_file(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
        part: &FinalizedPart,
        file_type: &str,
    ) -> Result<(), LeaseError>;

    /// Marks the job `complete` (sets `completed_at`). Fenced.
    async fn finish_export_job(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
    ) -> Result<(), LeaseError>;

    /// Marks the job `error` with a message (sets `completed_at`). Fenced.
    async fn fail_export_job(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
        worker_id: &WorkerId,
        fencing_token: u64,
        error_message: &str,
    ) -> Result<(), LeaseError>;
}

/// Marker trait composing the three job-state surfaces a worker needs.
///
/// Only the SQLite and Postgres backends implement this; it is held as an
/// `Arc<dyn BulkExportJobStore>` and selected at bootstrap by
/// `HFS_BULK_EXPORT_BACKEND`.
pub trait BulkExportJobStore:
    BulkExportStorage + ExportWorkerStorage + ExportClaimStrategy
{
}

impl<T> BulkExportJobStore for T where
    T: BulkExportStorage + ExportWorkerStorage + ExportClaimStrategy
{
}

/// Marker trait for a resource-store that can feed every export level.
///
/// The `SearchProvider` bound lets the worker intersect each fetched batch
/// with a compiled `_typeFilter` through the same search path a normal FHIR
/// search uses (see [`DefaultExportWorker::filter_batch_lines`]), so
/// composite deployments filter through whichever backend actually indexes
/// search (e.g. Elasticsearch) rather than the primary store alone.
pub trait ExportResourceProvider:
    ExportDataProvider + PatientExportProvider + GroupExportProvider + SearchProvider
{
}

impl<T> ExportResourceProvider for T where
    T: ExportDataProvider + PatientExportProvider + GroupExportProvider + SearchProvider
{
}

/// The default in-process export worker.
///
/// Binds a [`BulkExportJobStore`] (job state + claim + worker storage), an
/// [`ExportResourceProvider`] (the resource store), and an
/// [`ExportOutputStore`] (where NDJSON bytes go), and drives a claimed job to
/// completion under its lease.
pub struct DefaultExportWorker<Js: ?Sized, Dp: ?Sized, Os: ?Sized> {
    /// Job-state store (claim, worker storage, lifecycle).
    pub jobs: Arc<Js>,
    /// Resource data provider.
    pub data: Arc<Dp>,
    /// Output store for NDJSON parts.
    pub output: Arc<Os>,
    /// This worker's identifier.
    pub worker_id: WorkerId,
    /// Group-export `_since` toggle: when `true`, exclude resources from
    /// before `_since` for patients added to the Group after `_since`
    /// (using `Group.member.period.start`).
    pub exclude_since_newly_added: bool,
    /// Optional audit sink for the job's terminal lifecycle events.
    audit: Option<WorkerAudit>,
    /// How often the lease keeper renews the lease while a job runs.
    heartbeat_interval: Duration,
}

/// Fallback renewal cadence when the caller does not configure one.
const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);

/// Where the worker's `AuditEvent`s are sent.
#[derive(Clone)]
struct WorkerAudit {
    sink: Arc<dyn helios_audit::AuditSink>,
    source_observer: String,
}

/// Renews an [`ExportJobLease`] from a dedicated task for as long as it is alive.
///
/// The renewal must not share a future with the export itself. Until #1041 the
/// only heartbeat was inline, at the *end* of each batch: nothing renewed the
/// lease while a batch was being fetched, type-filtered and written. On a slow
/// type (millions of Observations, a `_typeFilter` that pages through the
/// search provider, an output store on S3) one batch outlives the lease, a
/// second worker reclaims the job, and the export restarts from zero — over and
/// over, because every attempt is just as slow as the last one. A separately
/// spawned task renews on schedule no matter how long a batch takes.
///
/// A lease that cannot be renewed before it expires is fatal to the run holding
/// it: once `lease_expiry` passes unrenewed the job is claimable, so continuing
/// to write would mean two workers producing parts for the same job. Each
/// renewal attempt is therefore bounded by what is left of the lease, and when
/// the window closes the keeper declares the lease lost, which the run notices
/// at its next batch boundary or mid-read through [`LeaseKeeper::lost`].
///
/// Dropping the keeper stops the renewal task.
struct LeaseKeeper {
    /// Held rather than only subscribed to, so the run can declare the lease
    /// lost too when a fenced write answers `LeaseLost`.
    lost: tokio::sync::watch::Sender<bool>,
    /// Newest expiry renewed to outside the keeper's task, in Unix
    /// milliseconds (see [`LeaseKeeper::note_renewed`]).
    renewed_until: Arc<AtomicI64>,
    handle: tokio::task::JoinHandle<()>,
}

/// The slice of a job store the [`LeaseKeeper`] uses.
///
/// Narrow on purpose: a keeper that must be handed a whole
/// [`BulkExportJobStore`] can only be exercised against a real backend, and the
/// failure this guards — a heartbeat that never lands — is exactly the one a
/// real backend will not reproduce on demand.
#[async_trait]
trait ExportLeaseRenewal: Send + Sync {
    /// Renews the lease, returning its new expiry.
    async fn heartbeat(&self, lease: &ExportJobLease) -> Result<DateTime<Utc>, LeaseError>;
}

/// Adapts a job store to the keeper's narrow surface.
struct JobStoreRenewal<Js: ?Sized>(Arc<Js>);

#[async_trait]
impl<Js> ExportLeaseRenewal for JobStoreRenewal<Js>
where
    Js: BulkExportJobStore + ?Sized + 'static,
{
    async fn heartbeat(&self, lease: &ExportJobLease) -> Result<DateTime<Utc>, LeaseError> {
        self.0.heartbeat(lease).await
    }
}

impl LeaseKeeper {
    /// Spawns the renewal task for `lease`.
    ///
    /// `heartbeat_interval` is the configured cadence
    /// (`HFS_BULK_EXPORT_HEARTBEAT_INTERVAL`); the keeper never waits longer
    /// than a third of the lease's remaining life regardless, so a deployment
    /// that raises the interval above the lease duration cannot starve itself.
    fn spawn<R>(jobs: Arc<R>, lease: ExportJobLease, heartbeat_interval: Duration) -> Self
    where
        R: ExportLeaseRenewal + ?Sized + 'static,
    {
        /// Pause between renewal attempts after a storage error. Short, because
        /// the whole retry window is capped by the lease's remaining life.
        const RETRY_AFTER: Duration = Duration::from_millis(500);
        let (lost, _) = tokio::sync::watch::channel(false);
        let flag = lost.clone();
        let renewed_until = Arc::new(AtomicI64::new(lease.lease_expiry.timestamp_millis()));
        let renewed_elsewhere = Arc::clone(&renewed_until);
        let handle = tokio::spawn(async move {
            let mut expiry = lease.lease_expiry;
            loop {
                expiry = latest_expiry(expiry, &renewed_elsewhere);
                let remaining = (expiry - Utc::now())
                    .to_std()
                    .unwrap_or(Duration::from_secs(1));
                // Honour the configured cadence, but never sleep past a third
                // of what is left: a 20 s interval under a 10 s lease would
                // hand the job away while this worker is still writing.
                let wait = heartbeat_interval
                    .min((remaining / 3).clamp(Duration::from_secs(1), Duration::from_secs(60)));
                tokio::time::sleep(wait).await;

                // Renew, retrying only for as long as the lease still covers
                // the writes the export loop is making in parallel. A renewal
                // that has not landed by `expiry` is indistinguishable from a
                // lost one: the job is claimable either way.
                let mut renewed = None;
                loop {
                    expiry = latest_expiry(expiry, &renewed_elsewhere);
                    let left = (expiry - Utc::now()).to_std().unwrap_or(Duration::ZERO);
                    if left.is_zero() {
                        break;
                    }
                    match tokio::time::timeout(left, jobs.heartbeat(&lease)).await {
                        Ok(Ok(new_expiry)) => {
                            renewed = Some(new_expiry);
                            break;
                        }
                        // Already reclaimed by another worker.
                        Ok(Err(LeaseError::LeaseLost { .. })) => {
                            warn_lease_lost(&lease, "heartbeat");
                            flag.send_replace(true);
                            return;
                        }
                        Ok(Err(LeaseError::Storage(e))) => {
                            tracing::debug!(
                                job_id = %lease.job_id,
                                error = %e,
                                "bulk-export lease heartbeat failed; retrying"
                            );
                            tokio::time::sleep(RETRY_AFTER.min(left)).await;
                        }
                        // Starved behind whatever the job store is doing for
                        // the rest of the lease.
                        Err(_elapsed) => {
                            tracing::warn!(
                                job_id = %lease.job_id,
                                worker = %lease.worker_id,
                                waited_ms = u64::try_from(left.as_millis()).unwrap_or(u64::MAX),
                                "bulk-export lease heartbeat timed out: no answer before the \
                                 lease expired"
                            );
                            break;
                        }
                    }
                }
                // The run may have renewed the lease itself meanwhile; a
                // renewal that landed there counts just as much as ours.
                let renewed = renewed.or_else(|| {
                    let latest = latest_expiry(expiry, &renewed_elsewhere);
                    (latest > Utc::now()).then_some(latest)
                });
                let Some(new_expiry) = renewed else {
                    tracing::warn!(
                        job_id = %lease.job_id,
                        worker = %lease.worker_id,
                        fencing_token = lease.fencing_token,
                        "bulk-export lease could not be renewed before it expired; \
                         abandoning the job so it can be reclaimed"
                    );
                    flag.send_replace(true);
                    return;
                };
                expiry = new_expiry;
            }
        });
        Self {
            lost,
            renewed_until,
            handle,
        }
    }

    /// Records a renewal made outside the keeper's own task, so the keeper
    /// measures its deadline from the newest expiry and does not declare a
    /// freshly renewed lease lost.
    ///
    /// Nothing on the export path renews out of band today — the inline
    /// per-batch heartbeat this replaced is gone — but the rescue it feeds is
    /// what keeps a starved keeper from tearing down a lease somebody else
    /// just extended, so it stays wired up and tested.
    #[allow(dead_code)]
    fn note_renewed(&self, expiry: DateTime<Utc>) {
        self.renewed_until
            .fetch_max(expiry.timestamp_millis(), Ordering::Relaxed);
    }

    /// Whether the run in flight should wind down because the lease is gone.
    fn should_stop(&self) -> bool {
        *self.lost.borrow()
    }

    /// Marks the lease lost from outside the renewal task — used when a fenced
    /// write answers `LeaseLost`, which is as conclusive as a failed heartbeat.
    ///
    /// `send_replace`, not `send`: `send` refuses to store the value when no
    /// receiver happens to be subscribed, and the run only subscribes for the
    /// duration of a read (see [`Self::lost`]). A loss declared while the run
    /// was between reads — writing a part, say — would be dropped on the floor
    /// and [`Self::should_stop`] would keep answering `false` forever.
    fn declare_lost(&self) {
        self.lost.send_replace(true);
    }

    /// Resolves once the lease is lost, and never otherwise. Raced against the
    /// batch reads so a loss abandons the run almost immediately rather than
    /// only after the current (possibly very long) fetch returns.
    async fn lost(&self) {
        let mut rx = self.lost.subscribe();
        while !*rx.borrow_and_update() {
            if rx.changed().await.is_err() {
                // Only reachable if the sender is dropped, which cannot happen
                // while `self` is alive.
                return;
            }
        }
    }
}

impl Drop for LeaseKeeper {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// The later of the keeper's own expiry and any renewal recorded through
/// [`LeaseKeeper::note_renewed`].
fn latest_expiry(expiry: DateTime<Utc>, renewed_until: &AtomicI64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(renewed_until.load(Ordering::Relaxed))
        .map_or(expiry, |renewed| renewed.max(expiry))
}

/// Logs a run abandoned because its lease is gone.
///
/// A lost lease is never silent: whoever reclaims the job starts it again from
/// scratch — `claim_next` wipes the previous attempt's progress and file rows —
/// so everything this worker exported is thrown away, and on a large corpus
/// that is hours of work (#1041).
fn warn_lease_lost(lease: &ExportJobLease, step: &str) {
    tracing::warn!(
        job_id = %lease.job_id,
        worker = %lease.worker_id,
        fencing_token = lease.fencing_token,
        step,
        "bulk-export run abandoned: its lease is no longer held; the job will be \
         re-claimed and this attempt's work discarded"
    );
}

/// Resolves what a failed fenced write means for the run.
///
/// A storage error fails the job, as it always has. A lost lease is not an
/// error at all: the job belongs to someone else now, so the keeper is told —
/// which stops the reads racing against it — and this run ends quietly.
fn fenced_outcome(keeper: &LeaseKeeper, e: LeaseError) -> Result<JobOutcome, LeaseError> {
    match e {
        LeaseError::Storage(e) => Err(LeaseError::Storage(e)),
        LeaseError::LeaseLost { .. } => {
            keeper.declare_lost();
            Ok(JobOutcome::Abandoned)
        }
    }
}

impl<Js, Dp, Os> DefaultExportWorker<Js, Dp, Os>
where
    Js: BulkExportJobStore + ?Sized + 'static,
    Dp: ExportResourceProvider + ?Sized,
    Os: ExportOutputStore + ?Sized,
{
    /// Creates a new worker (defaults to `since_newly_added=include`).
    pub fn new(jobs: Arc<Js>, data: Arc<Dp>, output: Arc<Os>, worker_id: WorkerId) -> Self {
        Self {
            jobs,
            data,
            output,
            worker_id,
            exclude_since_newly_added: false,
            audit: None,
            heartbeat_interval: DEFAULT_HEARTBEAT_INTERVAL,
        }
    }

    /// Sets how often the lease keeper renews the lease while a job runs.
    pub fn with_heartbeat_interval(mut self, interval: Duration) -> Self {
        self.heartbeat_interval = interval;
        self
    }

    /// Sets the `since_newly_added=exclude` toggle for Group exports.
    pub fn with_exclude_since_newly_added(mut self, exclude: bool) -> Self {
        self.exclude_since_newly_added = exclude;
        self
    }

    /// Emits a BALP `AuditEvent` when a job reaches a terminal state.
    ///
    /// The REST layer audits the kick-off, but only the worker knows how the
    /// job actually ended — so without this, an export that ran for an hour and
    /// then failed is indistinguishable in the audit log from one that
    /// completed.
    pub fn with_audit(
        mut self,
        sink: Arc<dyn helios_audit::AuditSink>,
        source_observer: impl Into<String>,
    ) -> Self {
        self.audit = Some(WorkerAudit {
            sink,
            source_observer: source_observer.into(),
        });
        self
    }

    /// Records a terminal lifecycle event for a job.
    async fn emit_audit(
        &self,
        job_id: &ExportJobId,
        view: Option<&WorkerJobView>,
        phase: &str,
        outcome: &str,
        outcome_desc: Option<&str>,
    ) {
        let Some(audit) = &self.audit else {
            return;
        };
        let level = view.map(|v| v.level.clone()).unwrap_or(ExportLevel::System);
        let types = view
            .map(|v| v.request.resource_types.clone())
            .unwrap_or_default();

        crate::core::bulk_export::audit::record_export_event(
            audit.sink.as_ref(),
            &audit.source_observer,
            view.and_then(|v| v.owner_subject.as_deref()),
            job_id.as_str(),
            phase,
            &level,
            &types,
            outcome,
            outcome_desc,
        )
        .await;
    }

    /// Runs the export job described by `lease` to completion.
    ///
    /// Every job-state mutation is fenced by `lease.worker_id` +
    /// `lease.fencing_token`; any `LeaseError::LeaseLost` aborts the run
    /// silently (the worker that reclaimed the job now owns it).
    pub async fn run_job(&self, lease: ExportJobLease) -> StorageResult<()> {
        let keeper = LeaseKeeper::spawn(
            Arc::new(JobStoreRenewal(Arc::clone(&self.jobs))),
            lease.clone(),
            self.heartbeat_interval,
        );
        self.run_job_with_keeper(lease, keeper).await
    }

    /// The body of [`Self::run_job`], with the lease keeper handed in.
    ///
    /// Split out so the tests can drive a whole run under a keeper whose
    /// renewals are stubbed — a lease genuinely expiring mid-run is, by
    /// construction, what the keeper exists to prevent, so it cannot be
    /// provoked through a real job store.
    async fn run_job_with_keeper(
        &self,
        lease: ExportJobLease,
        keeper: LeaseKeeper,
    ) -> StorageResult<()> {
        // Captured by `run_job_inner` as soon as it loads the job, so a failure
        // partway through can still attribute its audit event to the principal
        // that requested the export.
        let mut view: Option<WorkerJobView> = None;

        match self.run_job_inner(&lease, &keeper, &mut view).await {
            // Another worker owns the job now — stop, and emit nothing: the
            // worker that reclaimed the job will record its terminal event, and
            // a second one here would double-count. Not silently, though: this
            // attempt's output is about to be thrown away.
            Ok(JobOutcome::Abandoned) => {
                warn_lease_lost(&lease, "running the export");
                Ok(())
            }
            Ok(outcome) => {
                let (phase, code) = match outcome {
                    JobOutcome::Completed => ("complete", "0"),
                    JobOutcome::Cancelled => ("cancelled", "4"),
                    JobOutcome::Abandoned => unreachable!("handled above"),
                };
                self.emit_audit(&lease.job_id, view.as_ref(), phase, code, None)
                    .await;
                Ok(())
            }
            Err(LeaseError::LeaseLost { .. }) => {
                warn_lease_lost(&lease, "fenced job-state write");
                Ok(())
            }
            Err(LeaseError::Storage(e)) => {
                tracing::error!(job_id = %lease.job_id, error = %e, "export job failed");
                let public = public_failure_message(&e);
                let marked = self
                    .jobs
                    .fail_export_job(
                        &lease.tenant,
                        &lease.job_id,
                        &lease.worker_id,
                        lease.fencing_token,
                        &public,
                    )
                    .await;
                if let Err(LeaseError::LeaseLost { .. }) = marked {
                    // The job moved on while this run was failing — reclaimed
                    // by another worker, or retired by the attempt cap. Whoever
                    // owns it now records its outcome; auditing a failure here
                    // would put two terminal events on one job, which is the
                    // double-count every other lost-lease path avoids.
                    warn_lease_lost(&lease, "recording a failed run");
                    return Ok(());
                }
                self.emit_audit(
                    &lease.job_id,
                    view.as_ref(),
                    "failed",
                    "8",
                    Some(&e.to_string()),
                )
                .await;
                Err(e)
            }
        }
    }

    async fn run_job_inner(
        &self,
        lease: &ExportJobLease,
        keeper: &LeaseKeeper,
        captured_view: &mut Option<WorkerJobView>,
    ) -> Result<JobOutcome, LeaseError> {
        let tenant = &lease.tenant;
        let job_id = &lease.job_id;
        let wid = &lease.worker_id;
        let token = lease.fencing_token;

        let view = self
            .jobs
            .get_export_job_for_worker(tenant, job_id, wid, token)
            .await?;
        // Handed back to `run_job` immediately, before any work that could
        // fail, so a failure event still carries the requester and the export
        // level rather than degrading to an anonymous "something failed".
        *captured_view = Some(view.clone());

        self.jobs
            .mark_export_in_progress(tenant, job_id, wid, token)
            .await?;

        let request = &view.request;

        // Resolve the resource types to export.
        let types = self
            .data
            .list_export_types(tenant, request)
            .await
            .map_err(LeaseError::Storage)?;

        // Every `_typeFilter` must carry the query compiled against the search
        // parameter registry at kick-off (T4). A filter persisted before that
        // compilation existed has no query to run, so the job fails here
        // rather than silently exporting an unfiltered set. When two filters
        // target the same resource type, the first one wins — the kick-off
        // path does not combine multiple filters for one type today.
        let mut filters: HashMap<&str, &SearchQuery> = HashMap::new();
        for tf in &request.type_filters {
            let Some(compiled) = tf.compiled.as_ref() else {
                return Err(LeaseError::Storage(StorageError::BulkExport(
                    BulkExportError::InvalidTypeFilter {
                        resource_type: tf.resource_type.clone(),
                        message: "the filter was stored without a compiled query; \
                                  re-submit the export"
                            .to_string(),
                    },
                )));
            };
            filters.entry(tf.resource_type.as_str()).or_insert(compiled);
        }

        // For Group exports, resolve the member patient IDs once.
        // When `exclude_since_newly_added` is set AND `_since` is provided,
        // filter out patients whose `Group.member.period.start` is *after*
        // `_since` (i.e., they joined the cohort after the client's last
        // sync) — the IG-recommended behavior under the `exclude` toggle.
        let group_patient_ids: Option<Vec<String>> = match &view.level {
            ExportLevel::Group { group_id } => {
                let ids = match (self.exclude_since_newly_added, view.request.since.as_ref()) {
                    (true, Some(since)) => {
                        let members = self
                            .data
                            .get_group_members_with_periods(tenant, group_id)
                            .await
                            .map_err(LeaseError::Storage)?;
                        members
                            .into_iter()
                            .filter_map(|(reference, period_start)| {
                                let pid = reference.strip_prefix("Patient/")?;
                                // Keep members whose period.start is unknown OR
                                // <= since (i.e., were already members at since).
                                match period_start {
                                    Some(start) if start > *since => None,
                                    _ => Some(pid.to_string()),
                                }
                            })
                            .collect()
                    }
                    _ => self
                        .data
                        .resolve_group_patient_ids(tenant, group_id)
                        .await
                        .map_err(LeaseError::Storage)?,
                };
                Some(ids)
            }
            _ => None,
        };

        let batch_size = request.batch_size.max(1);
        let types_total = types.len() as u32;

        for (type_index, resource_type) in types.iter().enumerate() {
            if let Err(e) = self
                .jobs
                .set_export_current_type(
                    tenant,
                    job_id,
                    wid,
                    token,
                    Some(resource_type.as_str()),
                    type_index as u32,
                    types_total,
                )
                .await
            {
                return fenced_outcome(keeper, e);
            }

            // Resume from any persisted cursor for this type.
            let mut cursor: Option<String> = view
                .type_progress
                .iter()
                .find(|p| &p.resource_type == resource_type)
                .and_then(|p| p.cursor_state.clone());
            let mut exported: u64 = view
                .type_progress
                .iter()
                .find(|p| &p.resource_type == resource_type)
                .map(|p| p.exported_count)
                .unwrap_or(0);
            let mut part_index: u32 = 0;

            loop {
                // The clean abandonment point: the lease keeper has given up on
                // renewing, so the job is claimable and another worker may
                // already be writing parts for it. Stop before opening a writer.
                if keeper.should_stop() {
                    return Ok(JobOutcome::Abandoned);
                }

                // Cooperative cancellation check.
                if let Ok(progress) = self.jobs.get_export_status(tenant, job_id).await {
                    if progress.status == ExportStatus::Cancelled {
                        return Ok(JobOutcome::Cancelled);
                    }
                }

                let fetch = async {
                    match &group_patient_ids {
                        Some(pids) => {
                            self.data
                                .fetch_patient_compartment_batch(
                                    tenant,
                                    request,
                                    resource_type,
                                    pids,
                                    cursor.as_deref(),
                                    batch_size,
                                )
                                .await
                        }
                        None if matches!(view.level, ExportLevel::Patient)
                            && !request.patient_refs.is_empty() =>
                        {
                            // Patient-level with specific patient filter: scope
                            // to exactly the requested patients' compartments.
                            let patient_ids: Vec<String> = request
                                .patient_refs
                                .iter()
                                .map(|r| r.strip_prefix("Patient/").unwrap_or(r).to_string())
                                .collect();
                            self.data
                                .fetch_patient_compartment_batch(
                                    tenant,
                                    request,
                                    resource_type,
                                    &patient_ids,
                                    cursor.as_deref(),
                                    batch_size,
                                )
                                .await
                        }
                        // Patient-level without a patient filter: export all
                        // resources of this type across the patient compartment.
                        None => {
                            self.data
                                .fetch_export_batch(
                                    tenant,
                                    request,
                                    resource_type,
                                    cursor.as_deref(),
                                    batch_size,
                                )
                                .await
                        }
                    }
                };

                // Race the read against the lease. The reads are where a big
                // type spends nearly all of its time, and dropping one costs
                // nothing — no part is open, no row has been written — so this
                // is both the cheapest and the most effective place to notice
                // that the job is no longer ours (#1041). The write block below
                // is deliberately *outside* the race: cancelling between
                // `finalize_part` and `record_export_file` would publish an
                // artifact with no manifest row behind it.
                let batch = tokio::select! {
                    biased;
                    _ = keeper.lost() => return Ok(JobOutcome::Abandoned),
                    fetched = fetch => fetched.map_err(LeaseError::Storage)?,
                };

                // Intersect the batch with the compiled `_typeFilter` for this
                // type, if any. `cursor`, `is_last` and progress all keep coming
                // from `batch` — the traversal itself is unaffected by
                // filtering; only what gets written is.
                let lines = match filters.get(resource_type.as_str()) {
                    Some(filter) => {
                        let filtering = self.filter_batch_lines(
                            tenant,
                            resource_type,
                            filter,
                            batch.lines.clone(),
                        );
                        tokio::select! {
                            biased;
                            _ = keeper.lost() => return Ok(JobOutcome::Abandoned),
                            filtered = filtering => filtered.map_err(LeaseError::Storage)?,
                        }
                    }
                    None => batch.lines.clone(),
                };

                if !lines.is_empty() {
                    let key = ExportPartKey::output(
                        tenant.tenant_id().as_str(),
                        job_id.clone(),
                        resource_type.clone(),
                        part_index,
                        token,
                    );
                    let mut writer = self
                        .output
                        .open_writer(&key)
                        .await
                        .map_err(LeaseError::Storage)?;
                    for line in &lines {
                        let out_line = apply_elements(line, &request.elements);
                        writer.write_line(&out_line).await.map_err(|e| {
                            LeaseError::Storage(StorageError::Backend(
                                crate::error::BackendError::Internal {
                                    backend_name: "export-worker".to_string(),
                                    message: format!("write_line: {e}"),
                                    source: None,
                                },
                            ))
                        })?;
                    }
                    let finalized = self
                        .output
                        .finalize_part(&key, writer)
                        .await
                        .map_err(LeaseError::Storage)?;
                    exported += finalized.line_count;
                    if let Err(e) = self
                        .jobs
                        .record_export_file(tenant, job_id, wid, token, &finalized, "output")
                        .await
                    {
                        return fenced_outcome(keeper, e);
                    }
                    part_index += 1;
                }

                cursor = batch.next_cursor.clone();

                // Persist progress after each batch. The lease is renewed by the
                // keeper's own task, not here: an inline heartbeat only ever
                // fires *between* batches, which is exactly the gap a slow type
                // falls through (#1041).
                let mut progress = TypeExportProgress::new(resource_type.clone());
                progress.exported_count = exported;
                progress.cursor_state = cursor.clone();
                if let Err(e) = self
                    .jobs
                    .update_export_type_progress(tenant, job_id, wid, token, &progress)
                    .await
                {
                    return fenced_outcome(keeper, e);
                }

                if batch.is_last {
                    break;
                }
            }
        }

        // Publishing the job as complete is the one write that must not happen
        // under a lost lease: the reclaiming worker's own run would then finish
        // a job already advertised as complete with a different set of parts.
        if keeper.should_stop() {
            return Ok(JobOutcome::Abandoned);
        }
        if let Err(e) = self
            .jobs
            .set_export_current_type(tenant, job_id, wid, token, None, types_total, types_total)
            .await
        {
            return fenced_outcome(keeper, e);
        }
        if let Err(e) = self
            .jobs
            .finish_export_job(tenant, job_id, wid, token)
            .await
        {
            return fenced_outcome(keeper, e);
        }
        Ok(JobOutcome::Completed)
    }

    /// Keeps only the lines of `batch` whose resource matches `filter`.
    ///
    /// Runs `filter` with an `_id` OR-list of the batch's ids through the
    /// search provider, paging with the result cursor until every page is
    /// read, and drops the lines the search did not return. A line whose JSON
    /// has no `id` field cannot be matched at all and is dropped with a
    /// warning rather than kept unconditionally.
    async fn filter_batch_lines(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        filter: &SearchQuery,
        lines: Vec<String>,
    ) -> StorageResult<Vec<String>> {
        let ids_by_line: Vec<Option<String>> = lines
            .iter()
            .map(|line| {
                let id = serde_json::from_str::<serde_json::Value>(line)
                    .ok()
                    .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string));
                if id.is_none() {
                    tracing::warn!(
                        resource_type,
                        "export batch line has no id; dropping it from the type-filter check"
                    );
                }
                id
            })
            .collect();
        let ids: Vec<String> = ids_by_line.iter().flatten().cloned().collect();

        let mut query = filter.clone();
        query.resource_type = resource_type.to_string();
        query.parameters.push(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: ids.iter().map(|id| SearchValue::eq(id.clone())).collect(),
            chain: Vec::new(),
            components: Vec::new(),
        });
        query.count = Some(lines.len() as u32);
        query.cursor = None;
        query.total = None;
        query.summary = None;
        query.elements.clear();
        query.sort.clear();
        query.includes.clear();

        let mut matched: HashSet<String> = HashSet::new();
        loop {
            let result = self.data.search(tenant, &query).await?;
            matched.extend(result.resources.items.iter().map(|r| r.id().to_string()));
            match result.resources.page_info.next_cursor {
                Some(next) => query.cursor = Some(next),
                None => break,
            }
        }

        Ok(lines
            .into_iter()
            .zip(ids_by_line)
            .filter(|(_, id)| id.as_deref().is_some_and(|id| matched.contains(id)))
            .map(|(line, _)| line)
            .collect())
    }
}

/// How a worker's run of a job ended.
///
/// `run_job_inner` returns `Ok(())` for both a completed export and one that
/// was cancelled mid-flight, which made the two indistinguishable to the
/// caller — and so unauditable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobOutcome {
    /// Every requested type was exported and the job was finished.
    Completed,
    /// A cooperative cancellation check saw the job cancelled and stopped.
    Cancelled,
    /// The lease was lost mid-run, so the worker stopped without touching the
    /// job's state at all: the worker that reclaimed it owns its outcome now.
    Abandoned,
}

/// The failure text stored on the job and shown to the export's owner.
///
/// Domain errors (`BulkExportError`) describe the export itself and are
/// stored verbatim; anything else may carry backend detail (SQL, table
/// names, connection strings) and is replaced by a generic message, with
/// the full error kept in the server log by the caller.
fn public_failure_message(e: &StorageError) -> String {
    match e {
        StorageError::BulkExport(inner) => inner.to_string(),
        _ => "export failed: internal storage error".to_string(),
    }
}

/// The failure text stored on a job the claim cap retired (#1041).
///
/// A job whose lease expires mid-run is reclaimable, so a job that keeps dying
/// the same way — a worker crash, an OOM, a pod eviction — would otherwise be
/// passed from worker to worker forever: never terminal, never poll-able as a
/// failure, and holding one of its tenant's export slots the whole time.
/// `claim_next` stops handing it out once its claims would exceed the
/// configured cap and stores this instead. It is shown to the export's owner,
/// so it says what happened without naming a worker, a host or a backend.
pub fn abandoned_export_message(attempts: u32) -> String {
    let unit = if attempts == 1 { "attempt" } else { "attempts" };
    let cause = "each worker that claimed it lost its lease before finishing";
    format!("export abandoned after {attempts} {unit}: {cause}")
}

/// Applies `_elements` projection to an NDJSON line.
///
/// When `elements` is non-empty, keeps `resourceType`, `id`, `meta` and the
/// listed top-level element names, and adds a `SUBSETTED` `meta.tag`. On any
/// parse failure the original line is returned unchanged.
fn apply_elements(line: &str, elements: &[String]) -> String {
    if elements.is_empty() {
        return line.to_string();
    }
    let Ok(serde_json::Value::Object(obj)) = serde_json::from_str::<serde_json::Value>(line) else {
        return line.to_string();
    };
    let mut out = serde_json::Map::new();
    // Always-included mandatory elements.
    for key in ["resourceType", "id"] {
        if let Some(v) = obj.get(key) {
            out.insert(key.to_string(), v.clone());
        }
    }
    // Requested top-level elements (strip a leading `ResourceType.` prefix).
    for el in elements {
        let name = el.rsplit('.').next().unwrap_or(el.as_str());
        if let Some(v) = obj.get(name) {
            out.insert(name.to_string(), v.clone());
        }
    }
    // meta + SUBSETTED tag.
    let mut meta = obj
        .get("meta")
        .and_then(|m| m.as_object().cloned())
        .unwrap_or_default();
    let tag = serde_json::json!({
        "system": "http://terminology.hl7.org/CodeSystem/v3-ObservationValue",
        "code": "SUBSETTED",
    });
    let tags = meta
        .entry("tag".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    if let serde_json::Value::Array(arr) = tags {
        arr.push(tag);
    }
    out.insert("meta".to_string(), serde_json::Value::Object(meta));
    serde_json::to_string(&serde_json::Value::Object(out)).unwrap_or_else(|_| line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_lease_expiry_uses_explicit_time_duration_and_fallback() {
        let now = DateTime::parse_from_rfc3339("2026-09-17T12:00:00.123456789Z")
            .unwrap()
            .with_timezone(&Utc);

        for seconds in [30, 60, 180] {
            let duration = Duration::from_secs(seconds);
            assert_eq!(
                export_lease_expiry(now, duration),
                now + chrono::Duration::seconds(seconds as i64)
            );
        }

        let later = now + chrono::Duration::seconds(17);
        assert_eq!(
            export_lease_expiry(later, Duration::from_secs(180)),
            later + chrono::Duration::seconds(180),
            "a later renewal starts from its own operation time"
        );
        assert_eq!(
            export_lease_expiry(now, Duration::MAX),
            now + chrono::Duration::seconds(60),
            "an unrepresentable duration keeps the existing 60s fallback"
        );
    }

    #[test]
    fn test_apply_elements_noop_when_empty() {
        let line = r#"{"resourceType":"Patient","id":"1","name":[]}"#;
        assert_eq!(apply_elements(line, &[]), line);
    }

    #[test]
    fn test_apply_elements_subsets_and_tags() {
        let line = r#"{"resourceType":"Patient","id":"1","name":[{"family":"X"}],"gender":"male"}"#;
        let out = apply_elements(line, &["name".to_string()]);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["resourceType"], "Patient");
        assert_eq!(v["id"], "1");
        assert!(v.get("name").is_some());
        assert!(v.get("gender").is_none());
        assert_eq!(v["meta"]["tag"][0]["code"], "SUBSETTED");
    }

    #[test]
    fn test_public_failure_message_keeps_domain_errors() {
        use crate::error::BulkExportError;

        let err = StorageError::BulkExport(BulkExportError::GroupNotFound {
            group_id: "g1".to_string(),
        });
        let message = public_failure_message(&err);
        assert!(
            message.contains("g1"),
            "domain errors should be stored verbatim, got: {message}"
        );
    }

    #[test]
    fn test_public_failure_message_masks_backend_errors() {
        use crate::error::BackendError;

        let err = StorageError::Backend(BackendError::Internal {
            backend_name: "sqlite".to_string(),
            message: "SELECT * FROM secret".to_string(),
            source: None,
        });
        let message = public_failure_message(&err);
        assert_eq!(message, "export failed: internal storage error");
        assert!(!message.contains("SELECT"));
    }

    /// How a stubbed heartbeat behaves, for the [`LeaseKeeper`] tests.
    enum Renewal {
        /// Never answers — a heartbeat starved behind a long batch, which is
        /// what let the lease expire mid-type in #1041.
        Hangs,
        /// Answers, but always with a storage error.
        Fails,
        /// Renews normally.
        Lands,
        /// Answers that another worker holds the lease now.
        Lost,
    }

    struct StubRenewal {
        renewal: Renewal,
        /// How many times the keeper asked for a renewal.
        calls: Arc<std::sync::atomic::AtomicU32>,
    }

    impl StubRenewal {
        fn new(renewal: Renewal) -> Arc<Self> {
            Arc::new(Self {
                renewal,
                calls: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            })
        }
    }

    #[async_trait]
    impl ExportLeaseRenewal for StubRenewal {
        async fn heartbeat(&self, lease: &ExportJobLease) -> Result<DateTime<Utc>, LeaseError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            match self.renewal {
                Renewal::Hangs => std::future::pending().await,
                Renewal::Fails => Err(LeaseError::Storage(StorageError::Backend(
                    crate::error::BackendError::Internal {
                        backend_name: "stub".to_string(),
                        message: "job store busy".to_string(),
                        source: None,
                    },
                ))),
                Renewal::Lands => Ok(lease.renewed_expiry()),
                Renewal::Lost => Err(LeaseError::LeaseLost {
                    job_id: lease.job_id.clone(),
                }),
            }
        }
    }

    fn keeper_tenant() -> TenantContext {
        use crate::tenant::{TenantId, TenantPermissions};
        TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
    }

    /// A two-second lease, short enough that a keeper's timings play out inside
    /// a test.
    fn keeper_lease() -> ExportJobLease {
        ExportJobLease {
            job_id: ExportJobId::from_string("job-keeper"),
            tenant: keeper_tenant(),
            worker_id: WorkerId::new("w"),
            lease_expiry: Utc::now() + chrono::Duration::seconds(2),
            fencing_token: 1,
            lease_duration: Duration::from_secs(2),
        }
    }

    /// Spawns a keeper over a two-second lease and reports whether it declared
    /// that lease lost within `wait`.
    async fn keeper_loses_lease(renewal: Renewal, wait: Duration) -> bool {
        let keeper = LeaseKeeper::spawn(
            StubRenewal::new(renewal),
            keeper_lease(),
            DEFAULT_HEARTBEAT_INTERVAL,
        );
        tokio::time::timeout(wait, keeper.lost()).await.is_ok()
    }

    /// A heartbeat that cannot land before the lease expires is fatal (#1041).
    ///
    /// This is the shape of the bug: the only heartbeat used to run inline at
    /// the end of each batch, so a type whose batch takes longer than the lease
    /// never renewed at all — the job was reclaimed and restarted from zero,
    /// forever. A keeper that hangs must declare the lease lost rather than let
    /// the run keep writing under it.
    #[tokio::test]
    async fn test_lease_keeper_gives_up_on_a_starved_heartbeat() {
        assert!(keeper_loses_lease(Renewal::Hangs, Duration::from_secs(15)).await);
    }

    #[tokio::test]
    async fn test_lease_keeper_gives_up_when_heartbeats_keep_failing() {
        assert!(keeper_loses_lease(Renewal::Fails, Duration::from_secs(15)).await);
    }

    /// The converse: a lease that is being renewed is never declared lost, so
    /// the keeper cannot abort a healthy run.
    #[tokio::test]
    async fn test_lease_keeper_holds_a_renewable_lease() {
        assert!(!keeper_loses_lease(Renewal::Lands, Duration::from_secs(5)).await);
    }

    /// A renewal recorded from outside the keeper's task keeps the lease alive
    /// even while the keeper's own heartbeat is starved.
    #[tokio::test]
    async fn test_lease_keeper_honours_a_renewal_made_elsewhere() {
        let keeper = LeaseKeeper::spawn(
            StubRenewal::new(Renewal::Hangs),
            keeper_lease(),
            DEFAULT_HEARTBEAT_INTERVAL,
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
        while tokio::time::Instant::now() < deadline {
            keeper.note_renewed(Utc::now() + chrono::Duration::seconds(2));
            let lost = tokio::time::timeout(Duration::from_millis(500), keeper.lost()).await;
            assert!(
                lost.is_err(),
                "a lease renewed outside the keeper was declared lost"
            );
        }
    }

    /// A lost lease is declared lost *and* logged at `warn`: whoever reclaims
    /// the job restarts it from zero, so an operator has to be able to see it
    /// happen (#1041).
    #[tokio::test]
    async fn test_lease_keeper_warns_when_the_lease_was_reclaimed() {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let _guard = tracing::subscriber::set_default(CaptureWarnings(Arc::clone(&events)));
        assert!(keeper_loses_lease(Renewal::Lost, Duration::from_secs(15)).await);
        let events = events.lock().unwrap();
        assert!(
            events
                .iter()
                .any(|e: &String| e.contains("lease is no longer held")),
            "no warn for a reclaimed lease: {events:?}"
        );
    }

    /// The renewal cadence follows the lease's own life, not a constant.
    ///
    /// A deployment that raises `HFS_BULK_EXPORT_HEARTBEAT_INTERVAL` above the
    /// lease duration would otherwise starve itself: here a two-second lease is
    /// still heartbeated inside its first second even though the configured
    /// interval is a minute.
    #[tokio::test]
    async fn test_lease_keeper_renews_within_the_lease_not_the_configured_interval() {
        let stub = StubRenewal::new(Renewal::Lands);
        let calls = Arc::clone(&stub.calls);
        let keeper = LeaseKeeper::spawn(stub, keeper_lease(), Duration::from_secs(60));
        tokio::time::sleep(Duration::from_millis(1_500)).await;
        assert!(
            calls.load(Ordering::Relaxed) >= 1,
            "a 2 s lease must be renewed inside its first third, not after 60 s"
        );
        assert!(!keeper.should_stop(), "the renewed lease is still held");
    }

    /// And the configured interval is honoured when it is the tighter of the
    /// two — it was dead configuration before #1041.
    #[tokio::test]
    async fn test_lease_keeper_honours_a_shorter_configured_interval() {
        let stub = StubRenewal::new(Renewal::Lands);
        let calls = Arc::clone(&stub.calls);
        let lease = ExportJobLease {
            lease_expiry: Utc::now() + chrono::Duration::seconds(300),
            lease_duration: Duration::from_secs(300),
            ..keeper_lease()
        };
        let _keeper = LeaseKeeper::spawn(stub, lease, Duration::from_millis(200));
        tokio::time::sleep(Duration::from_millis(1_200)).await;
        let seen = calls.load(Ordering::Relaxed);
        assert!(
            seen >= 3,
            "a 200 ms interval under a 300 s lease should renew repeatedly, saw {seen}"
        );
    }

    /// A fenced write that answers `LeaseLost` ends the run quietly and tells
    /// the keeper, so the reads racing against it stop too. A storage error
    /// still fails the job.
    #[tokio::test]
    async fn test_fenced_outcome_declares_lost_but_propagates_storage_errors() {
        let keeper = LeaseKeeper::spawn(
            StubRenewal::new(Renewal::Lands),
            keeper_lease(),
            DEFAULT_HEARTBEAT_INTERVAL,
        );
        let outcome = fenced_outcome(
            &keeper,
            LeaseError::LeaseLost {
                job_id: ExportJobId::from_string("job-keeper"),
            },
        );
        assert!(matches!(outcome, Ok(JobOutcome::Abandoned)));
        assert!(keeper.should_stop(), "the keeper must be told");

        let storage = fenced_outcome(
            &keeper,
            LeaseError::Storage(StorageError::Backend(
                crate::error::BackendError::Internal {
                    backend_name: "stub".to_string(),
                    message: "disk full".to_string(),
                    source: None,
                },
            )),
        );
        assert!(matches!(storage, Err(LeaseError::Storage(_))));
    }

    /// Records the text of every `warn` or `error` event on the current thread.
    struct CaptureWarnings(Arc<std::sync::Mutex<Vec<String>>>);

    impl tracing::Subscriber for CaptureWarnings {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            struct Text(String);
            impl tracing::field::Visit for Text {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    self.0.push_str(&format!("{}={:?} ", field.name(), value));
                }
            }
            if *event.metadata().level() <= tracing::Level::WARN {
                let mut text = Text(String::new());
                event.record(&mut text);
                self.0.lock().unwrap().push(text.0);
            }
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    #[cfg(feature = "sqlite")]
    mod worker_integration {
        use super::*;
        use crate::backends::local_fs::LocalFsOutputStore;
        use crate::backends::sqlite::SqliteBackend;
        use crate::core::ResourceStorage;
        use crate::core::bulk_export::{ExportRequest, StartExportInput, TypeFilter};
        use crate::tenant::{TenantContext, TenantId, TenantPermissions};
        use chrono::Utc;
        use std::sync::Arc;

        /// Claim cap for tests that are not exercising the cap itself.
        const TEST_MAX_ATTEMPTS: u32 = 3;

        fn tenant() -> TenantContext {
            TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
        }

        #[tokio::test]
        async fn test_run_job_system_export_end_to_end() {
            let backend = Arc::new(SqliteBackend::in_memory().unwrap());
            backend.init_schema().unwrap();
            let tenant = tenant();

            for i in 0..3 {
                backend
                    .create(
                        &tenant,
                        "Patient",
                        serde_json::json!({"resourceType": "Patient", "id": format!("p{i}")}),
                        helios_fhir::FhirVersion::default(),
                    )
                    .await
                    .unwrap();
            }

            let tmp = tempfile::tempdir().unwrap();
            let output = Arc::new(LocalFsOutputStore::new(tmp.path(), "http://localhost:8080"));

            let job_id = backend
                .start_export(
                    &tenant,
                    StartExportInput {
                        request: ExportRequest::system()
                            .with_types(vec!["Patient".to_string()])
                            .with_batch_size(2),
                        transaction_time: Utc::now(),
                        request_url: "http://localhost/$export".to_string(),
                        owner_subject: Some("sub".to_string()),
                        fhir_version: helios_fhir::FhirVersion::default(),
                    },
                )
                .await
                .unwrap();

            let worker_id = WorkerId::new("w1");
            let worker = DefaultExportWorker::new(
                Arc::clone(&backend),
                Arc::clone(&backend),
                Arc::clone(&output),
                worker_id.clone(),
            );

            let lease = backend
                .claim_next(&worker_id, Duration::from_secs(60), TEST_MAX_ATTEMPTS)
                .await
                .unwrap()
                .expect("job claimable");
            assert_eq!(lease.job_id, job_id);

            worker.run_job(lease).await.unwrap();

            let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
            assert_eq!(progress.status, ExportStatus::Complete);
            // The worker clears the in-flight marker on completion and leaves
            // the counters showing every type as done (#961).
            assert_eq!(progress.current_type, None);
            assert_eq!(progress.types_total, 1);
            assert_eq!(progress.types_done, progress.types_total);

            let manifest = backend.get_export_manifest(&tenant, &job_id).await.unwrap();
            let total: u64 = manifest.output.iter().map(|e| e.count).sum();
            assert_eq!(total, 3);
        }

        /// Wraps [`LocalFsOutputStore`], recording the export status the worker
        /// sees when it opens the writer for the export's second resource type.
        ///
        /// (#961) `BulkExportJobStore` composes three traits
        /// (`BulkExportStorage + ExportWorkerStorage + ExportClaimStrategy`);
        /// wrapping it just to spy on `set_export_current_type` calls would mean
        /// delegating every method of all three to the inner SQLite backend.
        /// Wrapping `ExportOutputStore` instead (five methods) and reading
        /// `get_export_status` from inside `open_writer` is far cheaper and
        /// still proves the worker marks a type as current *before* it starts
        /// writing it.
        struct SecondTypeObserver {
            inner: Arc<LocalFsOutputStore>,
            backend: Arc<SqliteBackend>,
            tenant: TenantContext,
            second_type: String,
            observed: std::sync::Mutex<Option<(Option<String>, u32)>>,
        }

        #[async_trait::async_trait]
        impl ExportOutputStore for SecondTypeObserver {
            async fn open_writer(
                &self,
                key: &ExportPartKey,
            ) -> StorageResult<crate::core::bulk_export_output::ExportPartWriter> {
                if key.resource_type == self.second_type {
                    let already_observed = self.observed.lock().unwrap().is_some();
                    if !already_observed {
                        let status = self
                            .backend
                            .get_export_status(&self.tenant, &key.job_id)
                            .await?;
                        *self.observed.lock().unwrap() =
                            Some((status.current_type, status.types_done));
                    }
                }
                self.inner.open_writer(key).await
            }

            async fn finalize_part(
                &self,
                key: &ExportPartKey,
                writer: crate::core::bulk_export_output::ExportPartWriter,
            ) -> StorageResult<FinalizedPart> {
                self.inner.finalize_part(key, writer).await
            }

            async fn download_url(
                &self,
                key: &ExportPartKey,
                ttl: Duration,
            ) -> StorageResult<crate::core::bulk_export_output::DownloadUrl> {
                self.inner.download_url(key, ttl).await
            }

            async fn open_reader(
                &self,
                key: &ExportPartKey,
            ) -> StorageResult<std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>>> {
                self.inner.open_reader(key).await
            }

            async fn delete_job_outputs(
                &self,
                tenant: &TenantContext,
                job_id: &ExportJobId,
            ) -> StorageResult<()> {
                self.inner.delete_job_outputs(tenant, job_id).await
            }
        }

        /// The worker marks a type as current, with `types_done` reflecting how
        /// many types are already finished, before it writes the first line of
        /// that type — not after. See [`SecondTypeObserver`] for why this test
        /// observes the output store rather than the job store.
        #[tokio::test]
        async fn test_run_job_marks_each_type_in_order() {
            let backend = Arc::new(SqliteBackend::in_memory().unwrap());
            backend.init_schema().unwrap();
            let tenant = tenant();

            backend
                .create(
                    &tenant,
                    "Patient",
                    serde_json::json!({"resourceType": "Patient", "id": "p1"}),
                    helios_fhir::FhirVersion::default(),
                )
                .await
                .unwrap();
            backend
                .create(
                    &tenant,
                    "Observation",
                    serde_json::json!({"resourceType": "Observation", "id": "o1"}),
                    helios_fhir::FhirVersion::default(),
                )
                .await
                .unwrap();

            let tmp = tempfile::tempdir().unwrap();
            let inner = Arc::new(LocalFsOutputStore::new(tmp.path(), "http://localhost:8080"));
            let output = Arc::new(SecondTypeObserver {
                inner,
                backend: Arc::clone(&backend),
                tenant: tenant.clone(),
                second_type: "Observation".to_string(),
                observed: std::sync::Mutex::new(None),
            });

            let job_id = backend
                .start_export(
                    &tenant,
                    StartExportInput {
                        request: ExportRequest::system()
                            .with_types(vec!["Patient".to_string(), "Observation".to_string()]),
                        transaction_time: Utc::now(),
                        request_url: "http://localhost/$export".to_string(),
                        owner_subject: Some("sub".to_string()),
                        fhir_version: helios_fhir::FhirVersion::default(),
                    },
                )
                .await
                .unwrap();

            let worker_id = WorkerId::new("w-order");
            let worker = DefaultExportWorker::new(
                Arc::clone(&backend),
                Arc::clone(&backend),
                Arc::clone(&output),
                worker_id.clone(),
            );

            let lease = backend
                .claim_next(&worker_id, Duration::from_secs(60), TEST_MAX_ATTEMPTS)
                .await
                .unwrap()
                .expect("job claimable");

            worker.run_job(lease).await.unwrap();

            let observed = output.observed.lock().unwrap().clone();
            assert_eq!(
                observed,
                Some((Some("Observation".to_string()), 1)),
                "the job must already show Observation as current, with 1 type \
                 done, by the time its writer opens"
            );

            let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
            assert_eq!(progress.current_type, None);
            assert_eq!(progress.types_done, 2);
            assert_eq!(progress.types_total, 2);
        }

        /// The REST layer audits the kick-off, but only the worker knows how a
        /// job actually ended. Without a terminal event, an export that ran and
        /// then failed is indistinguishable in the audit log from one that
        /// completed — which is precisely the gap issue #168 is about.
        #[tokio::test]
        async fn test_worker_emits_terminal_audit_event_on_completion() {
            use crate::test_audit::{CollectorSink, detail_map};

            let backend = Arc::new(SqliteBackend::in_memory().unwrap());
            backend.init_schema().unwrap();
            let tenant = tenant();

            backend
                .create(
                    &tenant,
                    "Patient",
                    serde_json::json!({"resourceType": "Patient", "id": "p1"}),
                    helios_fhir::FhirVersion::default(),
                )
                .await
                .unwrap();

            let tmp = tempfile::tempdir().unwrap();
            let output = Arc::new(LocalFsOutputStore::new(tmp.path(), "http://localhost:8080"));

            let job_id = backend
                .start_export(
                    &tenant,
                    StartExportInput {
                        request: ExportRequest::system().with_types(vec!["Patient".to_string()]),
                        transaction_time: Utc::now(),
                        request_url: "http://localhost/$export".to_string(),
                        owner_subject: Some("Practitioner/dr-1".to_string()),
                        fhir_version: helios_fhir::FhirVersion::default(),
                    },
                )
                .await
                .unwrap();

            let sink = Arc::new(CollectorSink::new());
            let worker_id = WorkerId::new("w-audit");
            let worker = DefaultExportWorker::new(
                Arc::clone(&backend),
                Arc::clone(&backend),
                Arc::clone(&output),
                worker_id.clone(),
            )
            .with_audit(sink.clone(), "Device/hfs");

            let lease = backend
                .claim_next(&worker_id, Duration::from_secs(60), TEST_MAX_ATTEMPTS)
                .await
                .unwrap()
                .expect("job claimable");
            worker.run_job(lease).await.unwrap();

            let events = sink.events();
            assert_eq!(events.len(), 1, "exactly one terminal event per job run");

            let details = detail_map(&events[0]);
            assert_eq!(
                details.get("bulk-export-operation").map(String::as_str),
                Some("complete")
            );
            assert_eq!(
                details.get("job-id").map(String::as_str),
                Some(job_id.as_str())
            );
            assert_eq!(
                events[0].outcome.as_ref().and_then(|o| o.value.as_deref()),
                Some("0")
            );

            // The agent comes off the job row, not the request, so it survives
            // the job being picked up by a worker in another process.
            let agent = events[0].agent.as_ref().expect("event must have an agent");
            assert_eq!(
                agent[0]
                    .who
                    .as_ref()
                    .and_then(|w| w.reference.as_ref())
                    .and_then(|r| r.value.as_deref()),
                Some("Practitioner/dr-1")
            );
        }

        #[tokio::test]
        async fn test_run_job_stores_public_message_on_failure() {
            let backend = Arc::new(SqliteBackend::in_memory().unwrap());
            backend.init_schema().unwrap();
            let tenant = tenant();

            let tmp = tempfile::tempdir().unwrap();
            let output = Arc::new(LocalFsOutputStore::new(tmp.path(), "http://localhost:8080"));

            // No `Group/g-missing` was ever created, so resolving its members
            // during the run fails with a domain `GroupNotFound` error.
            let job_id = backend
                .start_export(
                    &tenant,
                    StartExportInput {
                        request: ExportRequest::group("g-missing")
                            .with_types(vec!["Patient".to_string()]),
                        transaction_time: Utc::now(),
                        request_url: "http://localhost/Group/g-missing/$export".to_string(),
                        owner_subject: Some("sub".to_string()),
                        fhir_version: helios_fhir::FhirVersion::default(),
                    },
                )
                .await
                .unwrap();

            let worker_id = WorkerId::new("w-fail");
            let worker = DefaultExportWorker::new(
                Arc::clone(&backend),
                Arc::clone(&backend),
                Arc::clone(&output),
                worker_id.clone(),
            );

            let lease = backend
                .claim_next(&worker_id, Duration::from_secs(60), TEST_MAX_ATTEMPTS)
                .await
                .unwrap()
                .expect("job claimable");

            let result = worker.run_job(lease).await;
            assert!(result.is_err(), "run_job should surface the failure");

            let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
            assert_eq!(progress.status, ExportStatus::Error);
            let error_message = progress
                .error_message
                .expect("failed job should carry an error message");
            assert!(
                error_message.contains("g-missing"),
                "error message should name the missing group, got: {error_message}"
            );
        }

        /// A `_typeFilter` persisted before compiled queries existed (T4) has
        /// no query for the worker to run. `run_job` must refuse it outright
        /// rather than fall back to exporting the type unfiltered.
        #[tokio::test]
        async fn test_run_job_fails_before_writing_when_type_filter_is_not_compiled() {
            let backend = Arc::new(SqliteBackend::in_memory().unwrap());
            backend.init_schema().unwrap();
            let tenant = tenant();

            backend
                .create(
                    &tenant,
                    "Patient",
                    serde_json::json!({"resourceType": "Patient", "id": "p1", "active": true}),
                    helios_fhir::FhirVersion::default(),
                )
                .await
                .unwrap();

            let tmp = tempfile::tempdir().unwrap();
            let output = Arc::new(LocalFsOutputStore::new(tmp.path(), "http://localhost:8080"));

            // `TypeFilter::new` alone (without `.with_compiled`) mimics a job
            // that was persisted before the compiled-query field existed.
            let job_id = backend
                .start_export(
                    &tenant,
                    StartExportInput {
                        request: ExportRequest::system()
                            .with_types(vec!["Patient".to_string()])
                            .with_type_filter(TypeFilter::new("Patient", "active=true")),
                        transaction_time: Utc::now(),
                        request_url: "http://localhost/$export".to_string(),
                        owner_subject: Some("sub".to_string()),
                        fhir_version: helios_fhir::FhirVersion::default(),
                    },
                )
                .await
                .unwrap();

            let worker_id = WorkerId::new("w-uncompiled-filter");
            let worker = DefaultExportWorker::new(
                Arc::clone(&backend),
                Arc::clone(&backend),
                Arc::clone(&output),
                worker_id.clone(),
            );

            let lease = backend
                .claim_next(&worker_id, Duration::from_secs(60), TEST_MAX_ATTEMPTS)
                .await
                .unwrap()
                .expect("job claimable");

            let result = worker.run_job(lease).await;
            assert!(
                result.is_err(),
                "run_job should refuse a filter without a compiled query"
            );

            let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
            assert_eq!(progress.status, ExportStatus::Error);
            let error_message = progress
                .error_message
                .expect("failed job should carry an error message");
            assert!(
                error_message.contains("re-submit"),
                "error message should tell the client to re-submit, got: {error_message}"
            );

            let manifest = backend.get_export_manifest(&tenant, &job_id).await.unwrap();
            assert!(
                manifest.output.is_empty(),
                "no output files should be recorded when the filter check fails before writing"
            );
        }

        /// Wraps [`LocalFsOutputStore`], making every part take `delay` to
        /// finalize so a run spans several lease lifetimes.
        struct SlowOutput {
            inner: Arc<LocalFsOutputStore>,
            delay: Duration,
        }

        #[async_trait::async_trait]
        impl ExportOutputStore for SlowOutput {
            async fn open_writer(
                &self,
                key: &ExportPartKey,
            ) -> StorageResult<crate::core::bulk_export_output::ExportPartWriter> {
                self.inner.open_writer(key).await
            }

            async fn finalize_part(
                &self,
                key: &ExportPartKey,
                writer: crate::core::bulk_export_output::ExportPartWriter,
            ) -> StorageResult<FinalizedPart> {
                tokio::time::sleep(self.delay).await;
                self.inner.finalize_part(key, writer).await
            }

            async fn download_url(
                &self,
                key: &ExportPartKey,
                ttl: Duration,
            ) -> StorageResult<crate::core::bulk_export_output::DownloadUrl> {
                self.inner.download_url(key, ttl).await
            }

            async fn open_reader(
                &self,
                key: &ExportPartKey,
            ) -> StorageResult<std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>>> {
                self.inner.open_reader(key).await
            }

            async fn delete_job_outputs(
                &self,
                tenant: &TenantContext,
                job_id: &ExportJobId,
            ) -> StorageResult<()> {
                self.inner.delete_job_outputs(tenant, job_id).await
            }
        }

        /// A run whose lease is lost mid-export stops without recording a
        /// terminal state for the job and without emitting a terminal audit
        /// event: the worker that reclaimed the job owns its outcome now, and
        /// two terminal events for one job would double-count (#1041).
        ///
        /// The keeper is stubbed rather than driven through the real job store
        /// on purpose — a lease genuinely lapsing under a live keeper is exactly
        /// what the keeper prevents, so it cannot be provoked from outside.
        #[tokio::test]
        async fn test_run_job_abandons_mid_export_when_the_lease_is_lost() {
            use crate::test_audit::CollectorSink;

            let backend = Arc::new(SqliteBackend::in_memory().unwrap());
            backend.init_schema().unwrap();
            let tenant = tenant();

            for i in 0..10 {
                backend
                    .create(
                        &tenant,
                        "Patient",
                        serde_json::json!({"resourceType": "Patient", "id": format!("p{i}")}),
                        helios_fhir::FhirVersion::default(),
                    )
                    .await
                    .unwrap();
            }

            let tmp = tempfile::tempdir().unwrap();
            let output = Arc::new(SlowOutput {
                inner: Arc::new(LocalFsOutputStore::new(tmp.path(), "http://localhost:8080")),
                // One part per batch, so ten batches take ~4 s — twice the
                // stubbed lease's life.
                delay: Duration::from_millis(400),
            });

            let job_id = backend
                .start_export(
                    &tenant,
                    StartExportInput {
                        request: ExportRequest::system()
                            .with_types(vec!["Patient".to_string()])
                            .with_batch_size(1),
                        transaction_time: Utc::now(),
                        request_url: "http://localhost/$export".to_string(),
                        owner_subject: Some("sub".to_string()),
                        fhir_version: helios_fhir::FhirVersion::default(),
                    },
                )
                .await
                .unwrap();

            let sink = Arc::new(CollectorSink::new());
            let worker_id = WorkerId::new("w-lost");
            let worker = DefaultExportWorker::new(
                Arc::clone(&backend),
                Arc::clone(&backend),
                Arc::clone(&output),
                worker_id.clone(),
            )
            .with_audit(sink.clone(), "Device/hfs");

            let lease = backend
                .claim_next(&worker_id, Duration::from_secs(60), TEST_MAX_ATTEMPTS)
                .await
                .unwrap()
                .expect("job claimable");

            // A keeper whose heartbeat never answers: it gives up two seconds
            // in, part-way through the ten batches.
            let keeper = LeaseKeeper::spawn(
                StubRenewal::new(Renewal::Hangs),
                ExportJobLease {
                    lease_expiry: Utc::now() + chrono::Duration::seconds(2),
                    lease_duration: Duration::from_secs(2),
                    ..lease.clone()
                },
                DEFAULT_HEARTBEAT_INTERVAL,
            );

            let warnings = Arc::new(std::sync::Mutex::new(Vec::new()));
            let guard = tracing::subscriber::set_default(CaptureWarnings(Arc::clone(&warnings)));
            worker
                .run_job_with_keeper(lease, keeper)
                .await
                .expect("a lost lease is not an error for this worker");
            drop(guard);

            let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
            assert_eq!(
                progress.status,
                ExportStatus::InProgress,
                "an abandoned run must not mark the job terminal"
            );
            assert!(
                sink.events().is_empty(),
                "an abandoned run must emit no terminal audit event; the worker \
                 that reclaims the job records one"
            );

            let manifest = backend.get_export_manifest(&tenant, &job_id).await.unwrap();
            assert!(
                manifest.output.len() < 10,
                "the run should have stopped before exporting every batch, got {} parts",
                manifest.output.len()
            );

            let warnings = warnings.lock().unwrap();
            assert!(
                warnings
                    .iter()
                    .any(|e: &String| e.contains("lease is no longer held")),
                "an abandoned run must say so in the log: {warnings:?}"
            );
        }

        /// A run that takes several lease lifetimes still finishes — under the
        /// real [`LeaseKeeper`] that [`DefaultExportWorker::run_job`] spawns for
        /// itself, with a second worker polling `claim_next` throughout.
        ///
        /// This is the test #1041 is about. Every other keeper test drives a
        /// stub: they prove the keeper renews, and that a run whose lease is
        /// already gone abandons cleanly. Neither shows the two halves working
        /// together, which is the only thing the reporter cared about. Before
        /// the fix the sole heartbeat was inline at the end of each batch, so
        /// nothing renewed the lease while a batch was being fetched, filtered
        /// and written: the contender below would have reclaimed the job
        /// part-way through, wiped this attempt's parts and restarted the
        /// export from its first resource type — forever, because the next
        /// attempt is just as slow as the last.
        ///
        /// The work here outlasts the lease more than twice over: ten batches
        /// at 500 ms each under a two-second lease.
        #[tokio::test]
        async fn test_run_job_outlives_a_lease_shorter_than_the_work() {
            /// Short enough that the run spans several of them.
            const LEASE: Duration = Duration::from_secs(2);
            /// Renews roughly six times per lease, so no single heartbeat
            /// landing late can decide the outcome.
            const HEARTBEAT: Duration = Duration::from_millis(300);
            /// One batch — and so one part — per seeded resource.
            const PATIENTS: usize = 10;

            let backend = Arc::new(SqliteBackend::in_memory().unwrap());
            backend.init_schema().unwrap();
            let tenant = tenant();

            for i in 0..PATIENTS {
                backend
                    .create(
                        &tenant,
                        "Patient",
                        serde_json::json!({"resourceType": "Patient", "id": format!("p{i}")}),
                        helios_fhir::FhirVersion::default(),
                    )
                    .await
                    .unwrap();
            }

            let tmp = tempfile::tempdir().unwrap();
            let output = Arc::new(SlowOutput {
                inner: Arc::new(LocalFsOutputStore::new(tmp.path(), "http://localhost:8080")),
                // Ten batches spend ~5 s in the output store alone, well past
                // the lease taken out below.
                delay: Duration::from_millis(500),
            });

            let job_id = backend
                .start_export(
                    &tenant,
                    StartExportInput {
                        request: ExportRequest::system()
                            .with_types(vec!["Patient".to_string()])
                            .with_batch_size(1),
                        transaction_time: Utc::now(),
                        request_url: "http://localhost/$export".to_string(),
                        owner_subject: Some("sub".to_string()),
                        fhir_version: helios_fhir::FhirVersion::default(),
                    },
                )
                .await
                .unwrap();

            let worker_id = WorkerId::new("w-slow");
            let worker = DefaultExportWorker::new(
                Arc::clone(&backend),
                Arc::clone(&backend),
                Arc::clone(&output),
                worker_id.clone(),
            )
            .with_heartbeat_interval(HEARTBEAT);

            let lease = backend
                .claim_next(&worker_id, LEASE, TEST_MAX_ATTEMPTS)
                .await
                .unwrap()
                .expect("job claimable");
            assert_eq!(lease.job_id, job_id);
            let claimed_token = lease.fencing_token;
            let claimed_expiry = lease.lease_expiry;

            // A second worker doing exactly what the worker loop does: asking
            // the job store for anything claimable. A lease allowed to lapse
            // makes the job eligible again, and this is what would take it.
            let contender = {
                let backend = Arc::clone(&backend);
                tokio::spawn(async move {
                    let thief = WorkerId::new("w-contender");
                    loop {
                        if let Some(stolen) = backend
                            .claim_next(&thief, Duration::from_secs(60), TEST_MAX_ATTEMPTS)
                            .await
                            .expect("claim_next")
                        {
                            return stolen;
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                })
            };

            // The real `run_job`, which builds its own `LeaseKeeper` — that is
            // the whole point of this test.
            worker.run_job(lease).await.unwrap();
            contender.abort();

            match contender.await {
                Err(e) if e.is_cancelled() => {}
                Ok(stolen) => panic!(
                    "a second worker reclaimed the job while it was still \
                     running ({stolen:?}): its lease was not renewed during the \
                     export"
                ),
                Err(e) => panic!("the contending claimer panicked: {e}"),
            }

            let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
            assert_eq!(
                progress.status,
                ExportStatus::Complete,
                "a run longer than its lease must still finish the job"
            );

            // Read the lease columns straight from the job row: nothing on the
            // public status surface exposes them, and they are what tell a
            // renewed lease apart from a merely lucky run.
            let (token, expiry): (i64, String) = backend
                .get_connection()
                .unwrap()
                .query_row(
                    "SELECT fencing_token, lease_expiry FROM bulk_export_jobs WHERE id = ?1",
                    rusqlite::params![job_id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(
                token as u64, claimed_token,
                "the job must still carry the original claim's fencing token; a \
                 higher one means it changed hands mid-run"
            );

            let expiry = chrono::DateTime::parse_from_rfc3339(&expiry)
                .expect("lease_expiry is stored as RFC 3339")
                .with_timezone(&Utc);
            assert!(
                expiry > claimed_expiry,
                "the keeper must have pushed the lease out during the run: \
                 persisted expiry {expiry} is no later than the claim's \
                 {claimed_expiry}"
            );

            let manifest = backend.get_export_manifest(&tenant, &job_id).await.unwrap();
            assert_eq!(
                manifest.output.len(),
                PATIENTS,
                "every batch should have produced its own part"
            );
            let total: u64 = manifest.output.iter().map(|e| e.count).sum();
            assert_eq!(
                total, PATIENTS as u64,
                "every seeded resource should be exported exactly once"
            );
        }

        /// An [`ExportOutputStore`] that fails as soon as the worker opens a
        /// part writer, optionally letting a second worker steal the job first.
        ///
        /// `open_writer` is the shortest route to the `LeaseError::Storage`
        /// that `run_job` answers by failing the job: the run only reaches it
        /// once the job has been loaded and marked in-progress under the lease,
        /// which is exactly where a real export dies when its output store goes
        /// away mid-run.
        struct FailingOutput {
            /// When set, this worker re-claims the job before the failure is
            /// returned, so the running attempt's fencing token is already
            /// stale by the time it tries to record that failure.
            steal: Option<(Arc<SqliteBackend>, WorkerId)>,
        }

        #[async_trait::async_trait]
        impl ExportOutputStore for FailingOutput {
            async fn open_writer(
                &self,
                _key: &ExportPartKey,
            ) -> StorageResult<crate::core::bulk_export_output::ExportPartWriter> {
                if let Some((backend, thief)) = &self.steal {
                    // The run's lease was claimed for a millisecond, so it has
                    // lapsed and the job is eligible again; the claim bumps the
                    // fencing token past the one this run is fenced on.
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    let stolen = backend
                        .claim_next(thief, Duration::from_secs(60), TEST_MAX_ATTEMPTS)
                        .await
                        .expect("claim_next")
                        .expect("the lapsed lease must be re-claimable");
                    assert_eq!(&stolen.worker_id, thief, "the job must change hands");
                }
                Err(StorageError::Backend(
                    crate::error::BackendError::Internal {
                        backend_name: "failing-output".to_string(),
                        message: "output store unavailable".to_string(),
                        source: None,
                    },
                ))
            }

            async fn finalize_part(
                &self,
                _key: &ExportPartKey,
                _writer: crate::core::bulk_export_output::ExportPartWriter,
            ) -> StorageResult<FinalizedPart> {
                unreachable!("no writer is ever opened")
            }

            async fn download_url(
                &self,
                _key: &ExportPartKey,
                _ttl: Duration,
            ) -> StorageResult<crate::core::bulk_export_output::DownloadUrl> {
                unreachable!("no part is ever written")
            }

            async fn open_reader(
                &self,
                _key: &ExportPartKey,
            ) -> StorageResult<std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>>> {
                unreachable!("no part is ever written")
            }

            async fn delete_job_outputs(
                &self,
                _tenant: &TenantContext,
                _job_id: &ExportJobId,
            ) -> StorageResult<()> {
                unreachable!("no part is ever written")
            }
        }

        /// What a run whose output store failed left behind.
        struct FailedRun {
            /// What `run_job_with_keeper` returned.
            result: StorageResult<()>,
            /// The terminal audit events the run emitted, if any.
            events: Vec<helios_fhir::r4::AuditEvent>,
            /// The job's status once the run was over.
            status: ExportStatus,
            /// The failure text stored on the job, if any.
            error_message: Option<String>,
            /// Every `warn`/`error` the run logged.
            warnings: Vec<String>,
        }

        /// Drives a one-patient system export whose output store fails on the
        /// first `open_writer`, and reports what the run left behind.
        ///
        /// `steal_with`, when set, is the worker that re-claims the job from
        /// inside that failing `open_writer` — the only window in which the
        /// lease can change hands *after* the run has started failing and
        /// *before* it records the failure.
        async fn run_export_with_failing_output(steal_with: Option<WorkerId>) -> FailedRun {
            use crate::test_audit::CollectorSink;

            let backend = Arc::new(SqliteBackend::in_memory().unwrap());
            backend.init_schema().unwrap();
            let tenant = tenant();

            backend
                .create(
                    &tenant,
                    "Patient",
                    serde_json::json!({"resourceType": "Patient", "id": "p1"}),
                    helios_fhir::FhirVersion::default(),
                )
                .await
                .unwrap();

            let job_id = backend
                .start_export(
                    &tenant,
                    StartExportInput {
                        request: ExportRequest::system().with_types(vec!["Patient".to_string()]),
                        transaction_time: Utc::now(),
                        request_url: "http://localhost/$export".to_string(),
                        owner_subject: Some("Practitioner/dr-1".to_string()),
                        fhir_version: helios_fhir::FhirVersion::default(),
                    },
                )
                .await
                .unwrap();

            let output = Arc::new(FailingOutput {
                steal: steal_with.map(|thief| (Arc::clone(&backend), thief)),
            });

            let sink = Arc::new(CollectorSink::new());
            let worker_id = WorkerId::new("w-output-down");
            let worker = DefaultExportWorker::new(
                Arc::clone(&backend),
                Arc::clone(&backend),
                Arc::clone(&output),
                worker_id.clone(),
            )
            .with_audit(sink.clone(), "Device/hfs");

            // A one-millisecond lease: the run keeps its fencing token — every
            // fenced write still matches on it — but the job itself is
            // re-claimable by the time the output store fails.
            let lease = backend
                .claim_next(&worker_id, Duration::from_millis(1), TEST_MAX_ATTEMPTS)
                .await
                .unwrap()
                .expect("job claimable");

            // The keeper renews a stub, so the job store never sees a heartbeat
            // that would resurrect the real lease, and the keeper never
            // declares the run abandoned — which would end it before it could
            // reach the failure path under test.
            let keeper = LeaseKeeper::spawn(
                StubRenewal::new(Renewal::Lands),
                ExportJobLease {
                    lease_expiry: Utc::now() + chrono::Duration::seconds(300),
                    lease_duration: Duration::from_secs(300),
                    ..lease.clone()
                },
                DEFAULT_HEARTBEAT_INTERVAL,
            );

            let warnings = Arc::new(std::sync::Mutex::new(Vec::new()));
            let guard = tracing::subscriber::set_default(CaptureWarnings(Arc::clone(&warnings)));
            let result = worker.run_job_with_keeper(lease, keeper).await;
            drop(guard);

            let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
            let warnings = warnings.lock().unwrap().clone();
            FailedRun {
                result,
                events: sink.events(),
                status: progress.status,
                error_message: progress.error_message,
                warnings,
            }
        }

        /// A run that fails while its lease is gone records nothing at all: the
        /// worker that reclaimed the job owns its outcome now, so auditing the
        /// failure here would put two terminal events on one job (#1041).
        ///
        /// This is the one lost-lease path that is only reachable from a
        /// *failing* run — `fail_export_job` itself answering `LeaseLost` —
        /// which is why the output store both steals the lease and fails.
        #[tokio::test]
        async fn test_run_job_does_not_audit_a_failure_it_no_longer_owns() {
            let run = run_export_with_failing_output(Some(WorkerId::new("w-thief"))).await;

            assert!(
                run.result.is_ok(),
                "a failure under a lost lease belongs to the new owner, not to \
                 this run: {:?}",
                run.result.err()
            );
            assert!(
                run.events.is_empty(),
                "a run that lost its lease must emit no terminal audit event, \
                 got {:?}",
                run.events.len()
            );
            assert_eq!(
                run.status,
                ExportStatus::InProgress,
                "the job must be left as the worker that reclaimed it set it up, \
                 not failed out from under that worker"
            );
            assert_eq!(run.error_message, None);
            assert!(
                run.warnings
                    .iter()
                    .any(|e: &String| e.contains("recording a failed run")),
                "an abandoned failure must say so in the log: {:?}",
                run.warnings
            );
        }

        /// The contrast that keeps the test above honest: the very same storage
        /// failure, with the lease still held, does fail the job, audit it, and
        /// surface the error to the worker loop.
        #[tokio::test]
        async fn test_run_job_audits_a_failure_it_still_owns() {
            use crate::test_audit::detail_map;

            let run = run_export_with_failing_output(None).await;

            assert!(
                run.result.is_err(),
                "a storage failure under a held lease must surface"
            );
            assert_eq!(run.events.len(), 1, "exactly one terminal event per run");
            let details = detail_map(&run.events[0]);
            assert_eq!(
                details.get("bulk-export-operation").map(String::as_str),
                Some("failed")
            );
            assert_eq!(
                run.events[0]
                    .outcome
                    .as_ref()
                    .and_then(|o| o.value.as_deref()),
                Some("8")
            );
            assert_eq!(run.status, ExportStatus::Error);
            assert_eq!(
                run.error_message.as_deref(),
                Some("export failed: internal storage error"),
                "the stored message is the masked, publishable one"
            );
        }

        /// `filter_batch_lines` should keep exactly the lines the search
        /// provider confirms match, in the batch's original order, paging
        /// through the search result until it is exhausted.
        #[tokio::test]
        async fn test_filter_batch_lines_pages_through_the_search_result() {
            // Point at the workspace's data directory so the search-parameter
            // registry loads the full FHIR spec — the minimal embedded set
            // used by `SqliteBackend::in_memory()` does not include `active`.
            let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("data");
            let config = crate::backends::sqlite::SqliteBackendConfig {
                data_dir: Some(data_dir),
                ..Default::default()
            };
            let backend = Arc::new(SqliteBackend::with_config(":memory:", config).unwrap());
            backend.init_schema().unwrap();
            let tenant = tenant();

            let mut lines = Vec::new();
            for i in 0..3 {
                let id = format!("p-active-{i}");
                let resource = serde_json::json!({
                    "resourceType": "Patient",
                    "id": id,
                    "active": true,
                });
                backend
                    .create(
                        &tenant,
                        "Patient",
                        resource.clone(),
                        helios_fhir::FhirVersion::default(),
                    )
                    .await
                    .unwrap();
                lines.push(resource.to_string());
            }
            let inactive = serde_json::json!({
                "resourceType": "Patient",
                "id": "p-inactive",
                "active": false,
            });
            backend
                .create(
                    &tenant,
                    "Patient",
                    inactive.clone(),
                    helios_fhir::FhirVersion::default(),
                )
                .await
                .unwrap();
            lines.push(inactive.to_string());

            let tmp = tempfile::tempdir().unwrap();
            let output = Arc::new(LocalFsOutputStore::new(tmp.path(), "http://localhost:8080"));
            let worker = DefaultExportWorker::new(
                Arc::clone(&backend),
                Arc::clone(&backend),
                Arc::clone(&output),
                WorkerId::new("w-filter-pages"),
            );

            let mut filter = SearchQuery::new("Patient");
            filter.parameters.push(SearchParameter {
                name: "active".to_string(),
                param_type: SearchParamType::Token,
                modifier: None,
                values: vec![SearchValue::boolean(true)],
                chain: Vec::new(),
                components: Vec::new(),
            });

            let filtered = worker
                .filter_batch_lines(&tenant, "Patient", &filter, lines.clone())
                .await
                .expect("filter_batch_lines");

            assert_eq!(
                filtered,
                lines[..3],
                "the 3 active lines, original order kept"
            );
        }
    }
}
