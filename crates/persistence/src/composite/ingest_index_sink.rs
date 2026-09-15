//! Index-during-ingest sink for `$bulk-submit` (#1127).
//!
//! With deferred indexing the Elasticsearch secondary receives nothing while a
//! manifest ingests and is rebuilt afterwards — a rebuild measured at many
//! times the ingest itself. [`IngestIndexSink`] is the opt-in alternative: a
//! [`BatchCommitObserver`] the ingest engine awaits right after each batch
//! commits (never before, so the secondary only ever sees what the primary
//! durably holds), which hands the batch's resources to a small pool of
//! writer tasks that write them into every [`ReindexTarget`] it was given.
//!
//! The design constraints, each learned by measurement:
//!
//! - **Bounded, never blocking.** Each writer sits behind a bounded queue, and
//!   the observer waits at most [`IngestIndexSinkConfig::max_wait`] to enqueue.
//!   A secondary that stops accepting writes degrades the batch to
//!   *unindexed* instead of stalling the ingest writer — an unbounded
//!   secondary flush is exactly what starved the lease heartbeat and forced a
//!   full re-walk of the corpus. Once a queue has timed out, further batches
//!   for that writer are rejected immediately while its queue stays full, so a
//!   dead secondary costs one `max_wait`, not one per batch.
//! - **FIFO per resource.** A resource is routed to a writer by a hash of
//!   `(tenant, type, id)`, so every version of it goes through one queue in
//!   commit order and a late duplicate cannot overwrite a newer version.
//!   Within a coalesced page, only the newest copy of a resource is written.
//! - **Small pages.** A writer coalesces at most
//!   [`IngestIndexSinkConfig::coalesce`] queued batches into one page; larger
//!   pages and deeper queues measurably slowed the ingest.
//!
//! What a target rejects (after a short retry of transient failures), what
//! could not be enqueued, and what was lost with a writer that stopped is
//! remembered per manifest. A write that may still be applied late (a timeout,
//! a transport failure) stays rejected even when a newer version is accepted
//! afterwards. The job store drains the sink before the manifest's receipt is
//! written ([`IngestIndexSink::drain`]), marks those entries unindexed, and
//! names their types so only they need a deferred rebuild.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::{Notify, mpsc};
use tokio::time::Instant;
use tracing::warn;

use crate::core::bulk_submit::{BatchCommitObserver, BatchCommitted, SubmissionId};
use crate::core::storage::ResourceStorage;
use crate::error::{BackendError, StorageError, StorageResult};
use crate::search::ReindexTarget;
use crate::tenant::TenantContext;
use crate::types::StoredResource;

/// Transient target failures are retried this many times, with these waits.
const TRANSIENT_RETRY_BACKOFF: [Duration; 2] =
    [Duration::from_millis(250), Duration::from_millis(1000)];

/// Ceiling on [`IngestIndexSink::settle_window`], whatever the configuration.
const MAX_SETTLE_WINDOW: Duration = Duration::from_secs(24 * 60 * 60);

/// Shape of an [`IngestIndexSink`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestIndexSinkConfig {
    /// Batches each writer's queue holds before the ingest has to wait.
    pub queue: usize,
    /// Writer tasks, each with its own queue.
    pub concurrency: usize,
    /// Queued batches a writer folds into one page write.
    pub coalesce: usize,
    /// The longest the ingest waits to enqueue a batch, the longest one job's
    /// read-back from the primary may take, and the longest a single write
    /// attempt into a target may take. [`IngestIndexSink::drain`] derives its
    /// no-progress window from it ([`IngestIndexSink::settle_window`]).
    pub max_wait: Duration,
}

impl Default for IngestIndexSinkConfig {
    fn default() -> Self {
        Self {
            queue: 16,
            concurrency: 4,
            coalesce: 4,
            max_wait: Duration::from_secs(30),
        }
    }
}

/// A resource the sink could not index for a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedResource {
    /// The resource's FHIR type.
    pub resource_type: String,
    /// The resource's id.
    pub resource_id: String,
    /// Why it was not indexed.
    pub reason: String,
}

/// What [`IngestIndexSink::drain`] found for one manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SinkDrain {
    /// Resource writes every target accepted.
    pub written: u64,
    /// Resources left unindexed, sorted by type then id.
    pub rejected: Vec<RejectedResource>,
    /// Whether the drain gave up on writes still in flight (they are
    /// included in `rejected`).
    pub timed_out: bool,
}

impl SinkDrain {
    /// The distinct resource types in [`Self::rejected`], sorted.
    pub fn rejected_types(&self) -> Vec<String> {
        let mut types: Vec<String> = self
            .rejected
            .iter()
            .map(|r| r.resource_type.clone())
            .collect();
        types.sort();
        types.dedup();
        types
    }
}

type ResourceKey = (String, String);
type ManifestKey = (String, String, String);

/// Per-manifest bookkeeping shared by the observer, the writers and the drain.
#[derive(Default)]
struct ManifestState {
    tally: Mutex<Tally>,
    /// Signalled whenever a job leaves `in_flight`.
    changed: Notify,
    /// Jobs numbered below this were cancelled
    /// ([`IngestIndexSink::cancel_pending`]) and are not written.
    cancelled_below: AtomicU64,
}

/// Why a resource is unindexed.
struct Rejection {
    /// The job that last failed the resource.
    job: u64,
    reason: String,
    /// The write reached the target, or may have, and could still be applied
    /// after a newer version: a later success must not clear it, so the type
    /// still gets its deferred rebuild.
    may_land: bool,
}

#[derive(Default)]
struct Tally {
    /// Enqueued (or being enqueued) jobs and the resources each carries.
    in_flight: HashMap<u64, Vec<ResourceKey>>,
    /// Unindexed resources.
    rejected: HashMap<ResourceKey, Rejection>,
    written: u64,
}

impl Tally {
    /// Records a failure from `job`, unless a newer job already decided or
    /// an earlier failure may still land (which keeps its own reason).
    fn reject(&mut self, key: ResourceKey, job: u64, reason: String, may_land: bool) {
        match self.rejected.get_mut(&key) {
            Some(existing) if existing.job > job || (existing.may_land && !may_land) => {
                existing.may_land |= may_land;
            }
            _ => {
                self.rejected.insert(
                    key,
                    Rejection {
                        job,
                        reason,
                        may_land,
                    },
                );
            }
        }
    }

    /// Records a successful write from `job`; it clears only an older failure
    /// that cannot land any more.
    fn accept(&mut self, key: &ResourceKey, job: u64) {
        if matches!(self.rejected.get(key), Some(r) if r.job < job && !r.may_land) {
            self.rejected.remove(key);
        }
        self.written += 1;
    }
}

/// Removes its job from `in_flight` when dropped and wakes any drain. A job
/// dropped before [`InFlight::finish`] — lost with a writer that panicked or
/// stopped, queued or mid-write — has its resources recorded as rejected,
/// never silently as indexed.
struct InFlight {
    job: u64,
    state: Arc<ManifestState>,
    finished: bool,
}

impl InFlight {
    /// Lets the job go once its outcome is recorded.
    fn finish(mut self) {
        self.finished = true;
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        {
            let mut tally = self.state.tally.lock();
            if let Some(keys) = tally.in_flight.remove(&self.job)
                && !self.finished
            {
                for key in keys {
                    tally.reject(
                        key,
                        self.job,
                        "the search index writer stopped before the batch was indexed".to_string(),
                        true,
                    );
                }
            }
        }
        self.state.changed.notify_waiters();
    }
}

/// One writer's share of a committed batch.
#[derive(Default)]
struct Payload {
    /// Committed resources, as the engine handed them over.
    resources: Vec<StoredResource>,
    /// `(type, id)` pairs the writer reads back from the primary: every
    /// successful entry when the engine does not hold the committed
    /// resources, and entries left unchanged, which no engine hands over but
    /// whose earlier indexing may never have landed (a crash, a reclaim).
    ids: Vec<ResourceKey>,
}

impl Payload {
    fn keys(&self) -> Vec<ResourceKey> {
        self.resources
            .iter()
            .map(|r| (r.resource_type().to_string(), r.id().to_string()))
            .chain(self.ids.iter().cloned())
            .collect()
    }
}

struct Job {
    job: u64,
    tenant: TenantContext,
    state: Arc<ManifestState>,
    payload: Payload,
    in_flight: InFlight,
}

struct Writer {
    /// Replaced, with a new writer task behind it, once the task stopped.
    sender: Mutex<mpsc::Sender<Job>>,
    /// Set once an enqueue timed out; while set, a full queue rejects at once.
    saturated: Arc<AtomicBool>,
}

/// Everything a writer task needs, shared by all of them.
struct WriterContext {
    source: Arc<dyn ResourceStorage>,
    targets: Vec<Arc<dyn ReindexTarget>>,
    config: IngestIndexSinkConfig,
}

impl WriterContext {
    /// The longest one coalesced page may legitimately take: every job read
    /// back from the primary, then every target's attempts and backoffs —
    /// plus one `max_wait` of slack.
    fn settle_window(&self) -> Duration {
        let max_wait = self.config.max_wait;
        let attempts = TRANSIENT_RETRY_BACKOFF.len() as u32 + 1;
        let backoff: Duration = TRANSIENT_RETRY_BACKOFF.iter().sum();
        let per_target = max_wait.saturating_mul(attempts).saturating_add(backoff);
        let read_back =
            max_wait.saturating_mul(u32::try_from(self.config.coalesce).unwrap_or(u32::MAX));
        let targets = u32::try_from(self.targets.len()).unwrap_or(u32::MAX);
        read_back
            .saturating_add(per_target.saturating_mul(targets))
            .saturating_add(max_wait)
            .min(MAX_SETTLE_WINDOW)
    }
}

fn spawn_writer(context: &Arc<WriterContext>, saturated: &Arc<AtomicBool>) -> mpsc::Sender<Job> {
    let (sender, receiver) = mpsc::channel(context.config.queue);
    tokio::spawn(run_writer(
        receiver,
        Arc::clone(context),
        Arc::clone(saturated),
    ));
    sender
}

/// Indexes committed bulk-submit batches into search targets while the
/// manifest ingests. See the [module docs](self).
pub struct IngestIndexSink {
    context: Arc<WriterContext>,
    writers: OnceLock<Vec<Writer>>,
    states: Mutex<HashMap<ManifestKey, Arc<ManifestState>>>,
    next_job: AtomicU64,
}

impl IngestIndexSink {
    /// Creates a sink writing into `targets` — the search indexes that serve
    /// search (the Elasticsearch secondary), not the primary's own index —
    /// reading resources back from `source` (the primary) for engines that do
    /// not hand committed resources to their observers and for entries left
    /// unchanged.
    ///
    /// Writer tasks start with the first batch, so construction needs no
    /// Tokio runtime.
    pub fn new(
        source: Arc<dyn ResourceStorage>,
        targets: Vec<Arc<dyn ReindexTarget>>,
        config: IngestIndexSinkConfig,
    ) -> Self {
        let config = IngestIndexSinkConfig {
            queue: config.queue.max(1),
            concurrency: config.concurrency.max(1),
            coalesce: config.coalesce.max(1),
            max_wait: config.max_wait,
        };
        Self {
            context: Arc::new(WriterContext {
                source,
                targets,
                config,
            }),
            writers: OnceLock::new(),
            states: Mutex::new(HashMap::new()),
            next_job: AtomicU64::new(1),
        }
    }

    /// The effective configuration (zero sizes raised to one).
    pub fn config(&self) -> IngestIndexSinkConfig {
        self.context.config
    }

    /// How long [`Self::drain`] and [`Self::cancel_pending`] wait without any
    /// job of the manifest finishing before they give up: the worst case of
    /// one coalesced page (read-backs, every target's retries and backoffs),
    /// plus one [`IngestIndexSinkConfig::max_wait`].
    pub fn settle_window(&self) -> Duration {
        self.context.settle_window()
    }

    fn writers(&self) -> &[Writer] {
        self.writers.get_or_init(|| {
            (0..self.context.config.concurrency)
                .map(|_| {
                    let saturated = Arc::new(AtomicBool::new(false));
                    let sender = spawn_writer(&self.context, &saturated);
                    Writer {
                        sender: Mutex::new(sender),
                        saturated,
                    }
                })
                .collect()
        })
    }

    fn state_for(&self, key: ManifestKey) -> Arc<ManifestState> {
        Arc::clone(self.states.lock().entry(key).or_default())
    }

    fn route(&self, tenant: &str, resource_type: &str, id: &str) -> usize {
        let mut hasher = DefaultHasher::new();
        (tenant, resource_type, id).hash(&mut hasher);
        (hasher.finish() % self.context.config.concurrency as u64) as usize
    }

    /// Forgets whatever an earlier run of the manifest recorded, so a new
    /// claim starts clean: a run that ended early (lease lost, abort, error)
    /// never drained. Writes still in flight for the earlier run report to
    /// the forgotten state.
    pub fn start_run(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
    ) {
        self.states
            .lock()
            .remove(&manifest_key(tenant, submission_id, manifest_id));
    }

    /// Cancels the submission's batches still queued for the search index and
    /// waits — bounded by [`Self::settle_window`] without progress, per
    /// manifest — for the page writes already under way, so nothing lands in
    /// search after an abort or a rollback removed it. Cancelled resources
    /// are recorded as rejected; batches committed afterwards are indexed as
    /// usual. Returns whether everything settled.
    pub async fn cancel_pending(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
    ) -> bool {
        let tenant_id = tenant.tenant_id().as_str();
        let submission = submission_id.to_string();
        let barrier = self.next_job.load(Ordering::Relaxed);
        let states: Vec<Arc<ManifestState>> = self
            .states
            .lock()
            .iter()
            .filter(|((t, s, _), _)| t == tenant_id && *s == submission)
            .map(|(_, state)| Arc::clone(state))
            .collect();
        for state in &states {
            state.cancelled_below.fetch_max(barrier, Ordering::Relaxed);
        }
        let window = self.settle_window();
        let mut settled = true;
        for state in &states {
            settled &= wait_idle(state, window).await;
        }
        settled
    }

    /// Waits until every write already handed over for the manifest finished
    /// — giving up once no job of it finished for [`Self::settle_window`], in
    /// which case whatever is still in flight counts as unindexed — and
    /// returns what the sink recorded for it, forgetting the manifest.
    pub async fn drain(
        &self,
        tenant: &TenantContext,
        submission_id: &SubmissionId,
        manifest_id: &str,
    ) -> SinkDrain {
        let key = manifest_key(tenant, submission_id, manifest_id);
        let Some(state) = self.states.lock().remove(&key) else {
            return SinkDrain::default();
        };
        let window = self.settle_window();
        let timed_out = !wait_idle(&state, window).await;

        let mut tally = state.tally.lock();
        if timed_out {
            let stuck: Vec<(u64, Vec<ResourceKey>)> = tally.in_flight.drain().collect();
            for (job, keys) in stuck {
                for key in keys {
                    tally.reject(
                        key,
                        job,
                        format!("still waiting to be indexed after {window:?} without progress"),
                        true,
                    );
                }
            }
        }
        let mut rejected: Vec<RejectedResource> = tally
            .rejected
            .drain()
            .map(
                |((resource_type, resource_id), rejection)| RejectedResource {
                    resource_type,
                    resource_id,
                    reason: rejection.reason,
                },
            )
            .collect();
        rejected.sort_by(|a, b| {
            (&a.resource_type, &a.resource_id).cmp(&(&b.resource_type, &b.resource_id))
        });
        SinkDrain {
            written: tally.written,
            rejected,
            timed_out,
        }
    }

    /// Hands one writer's share of a batch over, within `deadline`. A job
    /// that could not be handed over comes back with the reason.
    async fn enqueue(
        &self,
        writer: usize,
        job: Job,
        deadline: Instant,
    ) -> Result<(), (Box<Job>, String)> {
        let writer = &self.writers()[writer];
        let sender = {
            let mut sender = writer.sender.lock();
            if sender.is_closed() {
                warn!("bulk-submit: a search index writer stopped; starting a new one");
                writer.saturated.store(false, Ordering::Relaxed);
                *sender = spawn_writer(&self.context, &writer.saturated);
            }
            sender.clone()
        };
        let stopped = || "the search index writer stopped".to_string();
        let job = match sender.try_send(job) {
            Ok(()) => return Ok(()),
            Err(mpsc::error::TrySendError::Closed(job)) => {
                return Err((Box::new(job), stopped()));
            }
            Err(mpsc::error::TrySendError::Full(job)) => job,
        };
        if writer.saturated.load(Ordering::Relaxed) {
            return Err((
                Box::new(job),
                format!(
                    "the search index queue is full and did not drain within {:?}",
                    self.context.config.max_wait
                ),
            ));
        }
        match tokio::time::timeout_at(deadline, sender.reserve()).await {
            Ok(Ok(permit)) => {
                permit.send(job);
                Ok(())
            }
            Ok(Err(_)) => Err((Box::new(job), stopped())),
            Err(_) => {
                writer.saturated.store(true, Ordering::Relaxed);
                Err((
                    Box::new(job),
                    format!(
                        "the search index queue did not accept the batch within {:?}",
                        self.context.config.max_wait
                    ),
                ))
            }
        }
    }
}

fn manifest_key(
    tenant: &TenantContext,
    submission_id: &SubmissionId,
    manifest_id: &str,
) -> ManifestKey {
    (
        tenant.tenant_id().as_str().to_string(),
        submission_id.to_string(),
        manifest_id.to_string(),
    )
}

/// Waits until `state` has nothing in flight, giving up once none of its
/// jobs finished for `window`. Returns whether it settled.
async fn wait_idle(state: &ManifestState, window: Duration) -> bool {
    let mut remaining = usize::MAX;
    let mut deadline = Instant::now() + window;
    loop {
        let notified = state.changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let now_in_flight = state.tally.lock().in_flight.len();
        if now_in_flight == 0 {
            return true;
        }
        if now_in_flight < remaining {
            remaining = now_in_flight;
            deadline = Instant::now() + window;
        }
        if tokio::time::timeout_at(deadline, notified).await.is_err()
            && state.tally.lock().in_flight.len() >= remaining
        {
            return false;
        }
    }
}

#[async_trait]
impl BatchCommitObserver for IngestIndexSink {
    async fn batch_committed(&self, batch: &BatchCommitted<'_>) {
        let tenant_id = batch.tenant.tenant_id().as_str();
        let writers = self.context.config.concurrency;
        let mut shares: Vec<Option<Payload>> = (0..writers).map(|_| None).collect();
        for resource in batch.resources {
            let share = self.route(tenant_id, resource.resource_type(), resource.id());
            shares[share]
                .get_or_insert_default()
                .resources
                .push(resource.clone());
        }
        // Without committed resources every successful entry is read back;
        // with them, only the unchanged ones the engine does not hand over.
        let read_back_all = batch.resources.is_empty();
        for result in batch.results {
            if !result.is_success() || !(read_back_all || result.unchanged) {
                continue;
            }
            let Some(id) = result.resource_id.as_deref() else {
                continue;
            };
            let share = self.route(tenant_id, &result.resource_type, id);
            shares[share]
                .get_or_insert_default()
                .ids
                .push((result.resource_type.clone(), id.to_string()));
        }
        if shares.iter().all(Option::is_none) {
            return;
        }

        let state = self.state_for(manifest_key(
            batch.tenant,
            batch.submission_id,
            batch.manifest_id,
        ));
        let deadline = Instant::now() + self.context.config.max_wait;
        for (writer, payload) in shares.into_iter().enumerate() {
            let Some(payload) = payload else {
                continue;
            };
            let keys = payload.keys();
            let job = self.next_job.fetch_add(1, Ordering::Relaxed);
            state.tally.lock().in_flight.insert(job, keys.clone());
            let queued = Job {
                job,
                tenant: batch.tenant.clone(),
                state: Arc::clone(&state),
                payload,
                in_flight: InFlight {
                    job,
                    state: Arc::clone(&state),
                    finished: false,
                },
            };
            if let Err((queued, reason)) = self.enqueue(writer, queued, deadline).await {
                warn!(
                    submission = %batch.submission_id,
                    manifest = batch.manifest_id,
                    resources = keys.len(),
                    reason,
                    "bulk-submit: a committed batch could not be handed to the search index; \
                     its resources will be reported unindexed"
                );
                {
                    let mut tally = state.tally.lock();
                    for key in keys {
                        tally.reject(key, job, reason.clone(), false);
                    }
                }
                let Job { in_flight, .. } = *queued;
                in_flight.finish();
            }
        }
    }

    fn wants_resources(&self) -> bool {
        true
    }
}

/// One resource of a coalesced page, with the manifest and job it came from.
struct PageEntry {
    state: Arc<ManifestState>,
    job: u64,
    resource: StoredResource,
}

impl PageEntry {
    fn key(&self) -> ResourceKey {
        (
            self.resource.resource_type().to_string(),
            self.resource.id().to_string(),
        )
    }
}

async fn run_writer(
    mut receiver: mpsc::Receiver<Job>,
    context: Arc<WriterContext>,
    saturated: Arc<AtomicBool>,
) {
    while let Some(first) = receiver.recv().await {
        let mut jobs = vec![first];
        while jobs.len() < context.config.coalesce {
            match receiver.try_recv() {
                Ok(job) => jobs.push(job),
                Err(_) => break,
            }
        }
        if write_jobs(&context, jobs).await {
            saturated.store(false, Ordering::Relaxed);
        }
    }
}

/// Writes a coalesced set of jobs; returns whether any resource was accepted.
/// Every outcome is recorded before the jobs' in-flight guards are finished,
/// so a drain never sees a job finished without its result.
async fn write_jobs(context: &WriterContext, jobs: Vec<Job>) -> bool {
    let max_wait = context.config.max_wait;
    let mut guards = Vec::with_capacity(jobs.len());
    // tenant id -> (context, entries in job order)
    let mut pages: BTreeMap<String, (TenantContext, Vec<PageEntry>)> = BTreeMap::new();
    for Job {
        job,
        tenant,
        state,
        payload,
        in_flight,
    } in jobs
    {
        guards.push(in_flight);
        if job < state.cancelled_below.load(Ordering::Relaxed) {
            let mut tally = state.tally.lock();
            let keys = tally.in_flight.get(&job).cloned().unwrap_or_default();
            for key in keys {
                tally.reject(
                    key,
                    job,
                    "cancelled before it was indexed: the submission was aborted or rolled back"
                        .to_string(),
                    false,
                );
            }
            continue;
        }
        let Payload { mut resources, ids } = payload;
        if !ids.is_empty() {
            resources.extend(read_back(context, &tenant, &state, job, ids).await);
        }
        let page = pages
            .entry(tenant.tenant_id().as_str().to_string())
            .or_insert_with(|| (tenant.clone(), Vec::new()));
        page.1
            .extend(resources.into_iter().map(|resource| PageEntry {
                state: Arc::clone(&state),
                job,
                resource,
            }));
    }

    let mut any_accepted = false;
    for (_, (tenant, entries)) in pages {
        // Every copy of a resource in the page, in job order; only the newest
        // copy is written, and the outcome goes to every manifest holding one.
        let mut copies: HashMap<ResourceKey, Vec<usize>> = HashMap::new();
        for (index, entry) in entries.iter().enumerate() {
            copies.entry(entry.key()).or_default().push(index);
        }
        let mut groups: Vec<Vec<usize>> = copies.into_values().collect();
        groups.sort_unstable_by_key(|group| group[group.len() - 1]);
        let resources: Vec<StoredResource> = groups
            .iter()
            .map(|group| entries[group[group.len() - 1]].resource.clone())
            .collect();

        let mut failures: Vec<Option<Failure>> = vec![None; resources.len()];
        for target in &context.targets {
            let outcome = write_page(target.as_ref(), &tenant, &resources, max_wait).await;
            for (slot, failure) in failures.iter_mut().zip(outcome) {
                let Some(failure) = failure else {
                    continue;
                };
                if let Some(first) = slot.as_mut() {
                    first.may_land |= failure.may_land;
                } else {
                    *slot = Some(failure);
                }
            }
        }

        let mut rejected = 0usize;
        let mut first_reason = None;
        for (group, failure) in groups.iter().zip(failures) {
            if let Some(failure) = &failure {
                rejected += 1;
                first_reason.get_or_insert_with(|| failure.reason.clone());
            }
            // Once per manifest, for its newest copy.
            let mut decided: Vec<&Arc<ManifestState>> = Vec::new();
            for &index in group.iter().rev() {
                let entry = &entries[index];
                if decided.iter().any(|state| Arc::ptr_eq(state, &entry.state)) {
                    continue;
                }
                decided.push(&entry.state);
                let key = entry.key();
                let mut tally = entry.state.tally.lock();
                match &failure {
                    None => {
                        tally.accept(&key, entry.job);
                        any_accepted = true;
                    }
                    Some(failure) => {
                        tally.reject(key, entry.job, failure.reason.clone(), failure.may_land)
                    }
                }
            }
        }
        if rejected > 0 {
            warn!(
                tenant = %tenant.tenant_id(),
                rejected,
                page = resources.len(),
                reason = first_reason.as_deref().unwrap_or_default(),
                "bulk-submit: the search index rejected ingested resources; they will be \
                 reported unindexed"
            );
        }
    }
    for guard in guards {
        guard.finish();
    }
    any_accepted
}

/// Reads a job's resources back from the primary within one `max_wait`,
/// recording the ones that could not be read as rejected. Resources gone
/// since they committed are skipped: there is nothing left to index.
async fn read_back(
    context: &WriterContext,
    tenant: &TenantContext,
    state: &ManifestState,
    job: u64,
    ids: Vec<ResourceKey>,
) -> Vec<StoredResource> {
    let deadline = Instant::now() + context.config.max_wait;
    let mut by_type: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (resource_type, id) in ids {
        by_type.entry(resource_type).or_default().push(id);
    }
    let mut resources = Vec::new();
    for (resource_type, ids) in by_type {
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let read = tokio::time::timeout_at(
            deadline,
            context.source.read_batch(tenant, &resource_type, &refs),
        )
        .await;
        let reason = match read {
            Ok(Ok(found)) => {
                resources.extend(found);
                continue;
            }
            Ok(Err(e)) => format!("could not read the resource back from the primary: {e}"),
            Err(_) => format!(
                "reading the resource back from the primary timed out after {:?}",
                context.config.max_wait
            ),
        };
        let mut tally = state.tally.lock();
        for id in ids {
            tally.reject((resource_type.clone(), id), job, reason.clone(), false);
        }
    }
    resources
}

fn is_transient(error: &StorageError) -> bool {
    matches!(
        error,
        StorageError::Backend(BackendError::Unavailable { .. })
    )
}

/// A resource a target did not accept.
#[derive(Debug, Clone)]
struct Failure {
    reason: String,
    /// The write reached the target or may have — a timeout, a transport
    /// failure, no per-resource answer — so it could still be applied later.
    may_land: bool,
}

/// Writes one page into one target, retrying transient per-resource failures
/// briefly. Returns a failure per resource, in input order.
async fn write_page(
    target: &dyn ReindexTarget,
    tenant: &TenantContext,
    resources: &[StoredResource],
    max_wait: Duration,
) -> Vec<Option<Failure>> {
    let mut failures: Vec<Option<Failure>> = vec![None; resources.len()];
    let mut pending: Vec<usize> = (0..resources.len()).collect();
    for attempt in 0..=TRANSIENT_RETRY_BACKOFF.len() {
        let retry_page: Vec<StoredResource>;
        let page: &[StoredResource] = if attempt == 0 {
            resources
        } else {
            retry_page = pending.iter().map(|&i| resources[i].clone()).collect();
            &retry_page
        };
        let results: Vec<StorageResult<usize>> =
            match tokio::time::timeout(max_wait, target.write_search_entries_page(tenant, page))
                .await
            {
                Ok(results) => results,
                Err(_) => {
                    let reason =
                        format!("writing to the search index timed out after {max_wait:?}");
                    for &i in &pending {
                        failures[i] = Some(Failure {
                            reason: reason.clone(),
                            may_land: true,
                        });
                    }
                    return failures;
                }
            };
        let last_attempt = attempt == TRANSIENT_RETRY_BACKOFF.len();
        let mut retry = Vec::new();
        for (position, &i) in pending.iter().enumerate() {
            match results.get(position) {
                Some(Ok(_)) => failures[i] = None,
                Some(Err(e)) => {
                    let earlier_may_land = failures[i].as_ref().is_some_and(|f| f.may_land);
                    failures[i] = Some(Failure {
                        reason: e.to_string(),
                        may_land: is_transient(e) || earlier_may_land,
                    });
                    if is_transient(e) && !last_attempt {
                        retry.push(i);
                    }
                }
                None => {
                    failures[i] = Some(Failure {
                        reason: "the search index returned no result for the resource".to_string(),
                        may_land: true,
                    });
                }
            }
        }
        if retry.is_empty() {
            break;
        }
        if let Some(backoff) = TRANSIENT_RETRY_BACKOFF.get(attempt) {
            tokio::time::sleep(*backoff).await;
        }
        pending = retry;
    }
    failures
}

#[cfg(test)]
pub(crate) mod test_support {
    //! A scriptable [`ReindexTarget`] shared by the sink's and the indexing
    //! job store's tests.

    use std::collections::HashSet;

    use super::*;

    /// Records every accepted write in order; rejects ids in `reject`
    /// permanently, fails ids in `transient_once` once with a retryable
    /// error, panics on a page holding an id in `panic_on`, takes `delay` per
    /// page, never answers while `hang` is set, and waits for `release` while
    /// `hold` is set. Every page write signals `entered` as it starts.
    #[derive(Default)]
    pub(crate) struct SpyTarget {
        pub(crate) writes: Mutex<Vec<(String, serde_json::Value)>>,
        pub(crate) reject: HashSet<String>,
        pub(crate) transient_once: Mutex<HashSet<String>>,
        pub(crate) panic_on: HashSet<String>,
        pub(crate) delay: Duration,
        pub(crate) hang: AtomicBool,
        pub(crate) hold: AtomicBool,
        pub(crate) entered: Notify,
        pub(crate) release: Notify,
        pub(crate) pages: AtomicU64,
    }

    impl SpyTarget {
        pub(crate) fn ids(&self) -> Vec<String> {
            self.writes
                .lock()
                .iter()
                .map(|(id, _)| id.clone())
                .collect()
        }
    }

    fn backend_error(message: String, transient: bool) -> StorageError {
        StorageError::Backend(if transient {
            BackendError::Unavailable {
                backend_name: "spy-target".to_string(),
                message,
            }
        } else {
            BackendError::Internal {
                backend_name: "spy-target".to_string(),
                message,
                source: None,
            }
        })
    }

    #[async_trait]
    impl ReindexTarget for SpyTarget {
        async fn delete_search_entries(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _resource_id: &str,
        ) -> StorageResult<u64> {
            Ok(0)
        }

        async fn write_search_entries(
            &self,
            tenant: &TenantContext,
            resource: &StoredResource,
        ) -> StorageResult<usize> {
            self.write_search_entries_page(tenant, std::slice::from_ref(resource))
                .await
                .pop()
                .expect("one result per resource")
        }

        async fn clear_search_index(&self, _tenant: &TenantContext) -> StorageResult<u64> {
            Ok(0)
        }

        async fn write_search_entries_page(
            &self,
            _tenant: &TenantContext,
            resources: &[StoredResource],
        ) -> Vec<StorageResult<usize>> {
            self.entered.notify_one();
            if self.hang.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            if self.hold.load(Ordering::SeqCst) {
                self.release.notified().await;
            }
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            self.pages.fetch_add(1, Ordering::SeqCst);
            if let Some(id) = resources
                .iter()
                .map(StoredResource::id)
                .find(|id| self.panic_on.contains(*id))
            {
                panic!("spy panics on {id}");
            }
            resources
                .iter()
                .map(|resource| {
                    let id = resource.id().to_string();
                    if self.reject.contains(&id) {
                        return Err(backend_error(format!("spy rejected {id}"), false));
                    }
                    if self.transient_once.lock().remove(&id) {
                        return Err(backend_error(format!("spy busy for {id}"), true));
                    }
                    self.writes.lock().push((id, resource.content().clone()));
                    Ok(1)
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use helios_fhir::FhirVersion;
    use serde_json::json;

    use super::test_support::SpyTarget;
    use super::*;
    use crate::backends::sqlite::SqliteBackend;
    use crate::core::bulk_submit::BulkEntryResult;
    use crate::tenant::{TenantId, TenantPermissions};

    fn tenant() -> TenantContext {
        TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
    }

    fn patient(id: &str, version: u64) -> StoredResource {
        StoredResource::new(
            "Patient",
            id,
            TenantId::new("t1"),
            json!({"resourceType": "Patient", "id": id, "v": version}),
            FhirVersion::default(),
        )
    }

    fn success(line: u64, id: &str) -> BulkEntryResult {
        BulkEntryResult::success(line, "Patient", id, true)
    }

    fn unchanged(line: u64, id: &str) -> BulkEntryResult {
        let mut result = success(line, id);
        result.unchanged = true;
        result
    }

    fn sink_with(
        target: Arc<SpyTarget>,
        source: Arc<dyn ResourceStorage>,
        config: IngestIndexSinkConfig,
    ) -> IngestIndexSink {
        IngestIndexSink::new(source, vec![target as Arc<dyn ReindexTarget>], config)
    }

    fn empty_source() -> Arc<dyn ResourceStorage> {
        let sqlite = SqliteBackend::in_memory().unwrap();
        sqlite.init_schema().unwrap();
        Arc::new(sqlite)
    }

    async fn source_with(ids: &[&str]) -> Arc<dyn ResourceStorage> {
        let sqlite = SqliteBackend::in_memory().unwrap();
        sqlite.init_schema().unwrap();
        for id in ids {
            ResourceStorage::create(
                &sqlite,
                &tenant(),
                "Patient",
                json!({"resourceType": "Patient", "id": id}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        }
        Arc::new(sqlite)
    }

    async fn commit_to(
        sink: &IngestIndexSink,
        submission: &SubmissionId,
        manifest: &str,
        results: &[BulkEntryResult],
        resources: &[StoredResource],
    ) {
        let tenant = tenant();
        sink.batch_committed(&BatchCommitted {
            tenant: &tenant,
            submission_id: submission,
            manifest_id: manifest,
            results,
            resources,
        })
        .await;
    }

    async fn commit(
        sink: &IngestIndexSink,
        submission: &SubmissionId,
        results: &[BulkEntryResult],
        resources: &[StoredResource],
    ) {
        commit_to(sink, submission, "m1", results, resources).await;
    }

    fn versions_of(target: &SpyTarget, id: &str) -> Vec<u64> {
        target
            .writes
            .lock()
            .iter()
            .filter(|(w, _)| w == id)
            .map(|(_, content)| content["v"].as_u64().unwrap())
            .collect()
    }

    #[tokio::test]
    async fn every_committed_resource_reaches_the_target_and_the_drain_counts_it() {
        let target = Arc::new(SpyTarget::default());
        let sink = sink_with(
            target.clone(),
            empty_source(),
            IngestIndexSinkConfig::default(),
        );
        let sub = SubmissionId::generate("sink");
        for batch in 0..5u64 {
            let ids: Vec<String> = (0..10).map(|i| format!("p-{batch}-{i}")).collect();
            let results: Vec<_> = ids.iter().map(|id| success(1, id)).collect();
            let resources: Vec<_> = ids.iter().map(|id| patient(id, 1)).collect();
            commit(&sink, &sub, &results, &resources).await;
        }

        let drain = sink.drain(&tenant(), &sub, "m1").await;
        assert_eq!(drain.written, 50);
        assert!(drain.rejected.is_empty());
        assert!(!drain.timed_out);
        let mut seen = target.ids();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 50);
        // A drained manifest is forgotten.
        assert_eq!(
            sink.drain(&tenant(), &sub, "m1").await,
            SinkDrain::default()
        );
    }

    #[tokio::test]
    async fn writes_for_one_resource_stay_in_commit_order() {
        let target = Arc::new(SpyTarget::default());
        let sink = sink_with(
            target.clone(),
            empty_source(),
            IngestIndexSinkConfig {
                concurrency: 4,
                coalesce: 1,
                ..Default::default()
            },
        );
        let sub = SubmissionId::generate("fifo");
        let ids = ["a", "b", "c", "d", "e", "f"];
        for version in 1..=40u64 {
            let results: Vec<_> = ids.iter().map(|id| success(version, id)).collect();
            let resources: Vec<_> = ids.iter().map(|id| patient(id, version)).collect();
            commit(&sink, &sub, &results, &resources).await;
        }
        let drain = sink.drain(&tenant(), &sub, "m1").await;
        assert!(drain.rejected.is_empty());

        for id in ids {
            let versions = versions_of(&target, id);
            assert_eq!(versions.len(), 40, "every version of {id} is written");
            assert!(
                versions.windows(2).all(|w| w[0] < w[1]),
                "{id} written out of order: {versions:?}"
            );
        }
    }

    #[tokio::test]
    async fn coalesced_duplicates_write_only_the_newest_version() {
        let target = Arc::new(SpyTarget::default());
        target.hold.store(true, Ordering::SeqCst);
        let sink = sink_with(
            target.clone(),
            empty_source(),
            IngestIndexSinkConfig {
                concurrency: 1,
                coalesce: 8,
                queue: 8,
                ..Default::default()
            },
        );
        let sub = SubmissionId::generate("coalesce");
        // The first batch holds the writer until released; the next three
        // queue up behind it and are coalesced into one page.
        commit(&sink, &sub, &[success(1, "x")], &[patient("x", 1)]).await;
        target.entered.notified().await;
        for version in 2..=4u64 {
            commit(
                &sink,
                &sub,
                &[success(version, "x")],
                &[patient("x", version)],
            )
            .await;
        }
        target.hold.store(false, Ordering::SeqCst);
        target.release.notify_one();
        let drain = sink.drain(&tenant(), &sub, "m1").await;
        assert_eq!(
            versions_of(&target, "x"),
            vec![1, 4],
            "only the newest queued copy is written"
        );
        assert!(drain.rejected.is_empty(), "{:?}", drain.rejected);
        assert_eq!(drain.written, 2);
    }

    #[tokio::test]
    async fn every_manifest_with_a_copy_in_a_coalesced_page_gets_the_outcome() {
        let target = Arc::new(SpyTarget::default());
        target.hold.store(true, Ordering::SeqCst);
        let sink = sink_with(
            target.clone(),
            empty_source(),
            IngestIndexSinkConfig {
                concurrency: 1,
                coalesce: 8,
                queue: 8,
                ..Default::default()
            },
        );
        let sub = SubmissionId::generate("two-manifests");
        commit_to(
            &sink,
            &sub,
            "m1",
            &[success(1, "blocker")],
            &[patient("blocker", 1)],
        )
        .await;
        target.entered.notified().await;
        commit_to(&sink, &sub, "m1", &[success(2, "x")], &[patient("x", 1)]).await;
        commit_to(&sink, &sub, "m2", &[success(1, "x")], &[patient("x", 2)]).await;
        target.hold.store(false, Ordering::SeqCst);
        target.release.notify_one();

        let m1 = sink.drain(&tenant(), &sub, "m1").await;
        let m2 = sink.drain(&tenant(), &sub, "m2").await;
        assert_eq!(versions_of(&target, "x"), vec![2]);
        assert!(m1.rejected.is_empty() && m2.rejected.is_empty());
        assert_eq!(m1.written, 2, "m1's superseded copy of x is indexed too");
        assert_eq!(m2.written, 1);
    }

    #[tokio::test]
    async fn a_rejected_resource_is_reported_with_its_type() {
        let target = Arc::new(SpyTarget {
            reject: ["bad".to_string()].into_iter().collect(),
            ..Default::default()
        });
        let sink = sink_with(
            target.clone(),
            empty_source(),
            IngestIndexSinkConfig::default(),
        );
        let sub = SubmissionId::generate("reject");
        commit(
            &sink,
            &sub,
            &[success(1, "good"), success(2, "bad")],
            &[patient("good", 1), patient("bad", 1)],
        )
        .await;
        let drain = sink.drain(&tenant(), &sub, "m1").await;
        assert_eq!(drain.written, 1);
        assert_eq!(drain.rejected.len(), 1);
        assert_eq!(drain.rejected[0].resource_id, "bad");
        assert!(drain.rejected[0].reason.contains("spy rejected bad"));
        assert_eq!(drain.rejected_types(), vec!["Patient".to_string()]);
    }

    #[tokio::test]
    async fn a_transient_failure_is_retried_before_it_counts_as_rejected() {
        let target = Arc::new(SpyTarget::default());
        target.transient_once.lock().insert("flaky".to_string());
        let sink = sink_with(
            target.clone(),
            empty_source(),
            IngestIndexSinkConfig::default(),
        );
        let sub = SubmissionId::generate("transient");
        commit(&sink, &sub, &[success(1, "flaky")], &[patient("flaky", 1)]).await;
        let drain = sink.drain(&tenant(), &sub, "m1").await;
        assert!(drain.rejected.is_empty(), "{:?}", drain.rejected);
        assert_eq!(target.ids(), vec!["flaky".to_string()]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_timed_out_write_stays_rejected_after_a_newer_version_is_accepted() {
        let target = Arc::new(SpyTarget::default());
        target.hang.store(true, Ordering::SeqCst);
        let sink = sink_with(
            target.clone(),
            empty_source(),
            IngestIndexSinkConfig {
                concurrency: 1,
                coalesce: 1,
                max_wait: Duration::from_millis(100),
                ..Default::default()
            },
        );
        let sub = SubmissionId::generate("late-landing");
        commit(&sink, &sub, &[success(1, "x")], &[patient("x", 1)]).await;
        target.entered.notified().await;
        commit(&sink, &sub, &[success(2, "x")], &[patient("x", 2)]).await;
        target.hang.store(false, Ordering::SeqCst);

        let drain = sink.drain(&tenant(), &sub, "m1").await;
        assert_eq!(versions_of(&target, "x"), vec![2]);
        assert_eq!(drain.written, 1);
        // The timed-out v1 may still be applied after v2, so x stays
        // unindexed and its type still gets the deferred rebuild.
        assert_eq!(drain.rejected.len(), 1, "{:?}", drain.rejected);
        assert!(
            drain.rejected[0].reason.contains("timed out"),
            "{:?}",
            drain.rejected
        );
        assert_eq!(drain.rejected_types(), vec!["Patient".to_string()]);
    }

    #[tokio::test]
    async fn a_writer_that_panics_reports_its_batches_unindexed_and_is_replaced() {
        let target = Arc::new(SpyTarget {
            panic_on: ["boom".to_string()].into_iter().collect(),
            ..Default::default()
        });
        let sink = sink_with(
            target.clone(),
            empty_source(),
            IngestIndexSinkConfig {
                concurrency: 1,
                ..Default::default()
            },
        );
        let sub = SubmissionId::generate("panic");
        commit(&sink, &sub, &[success(1, "boom")], &[patient("boom", 1)]).await;
        commit(
            &sink,
            &sub,
            &[success(2, "behind")],
            &[patient("behind", 1)],
        )
        .await;
        let drain = sink.drain(&tenant(), &sub, "m1").await;
        let rejected: Vec<&str> = drain
            .rejected
            .iter()
            .map(|r| r.resource_id.as_str())
            .collect();
        assert!(rejected.contains(&"boom"), "{:?}", drain.rejected);
        assert_eq!(
            drain.written + rejected.len() as u64,
            2,
            "no batch vanishes with the writer: {drain:?}"
        );

        tokio::time::timeout(Duration::from_secs(5), async {
            while !sink.writers()[0].sender.lock().is_closed() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the panicked writer's queue closes");
        let after = SubmissionId::generate("after-panic");
        commit(&sink, &after, &[success(1, "fine")], &[patient("fine", 1)]).await;
        let drain = sink.drain(&tenant(), &after, "m1").await;
        assert!(drain.rejected.is_empty(), "{:?}", drain.rejected);
        assert_eq!(drain.written, 1, "a new writer took over");
        assert!(target.ids().contains(&"fine".to_string()));
    }

    #[tokio::test]
    async fn cancel_pending_skips_queued_batches_and_waits_for_the_write_under_way() {
        let target = Arc::new(SpyTarget::default());
        target.hold.store(true, Ordering::SeqCst);
        let sink = sink_with(
            target.clone(),
            empty_source(),
            IngestIndexSinkConfig {
                concurrency: 1,
                coalesce: 1,
                ..Default::default()
            },
        );
        let sub = SubmissionId::generate("cancel");
        let t = tenant();
        commit(
            &sink,
            &sub,
            &[success(1, "under-way")],
            &[patient("under-way", 1)],
        )
        .await;
        target.entered.notified().await;
        commit(
            &sink,
            &sub,
            &[success(2, "queued")],
            &[patient("queued", 1)],
        )
        .await;
        let (settled, ()) = tokio::join!(sink.cancel_pending(&t, &sub), async {
            target.hold.store(false, Ordering::SeqCst);
            target.release.notify_one();
        });
        assert!(settled);
        assert_eq!(target.ids(), vec!["under-way".to_string()]);

        // A batch committed after the cancel is indexed as usual.
        commit(&sink, &sub, &[success(3, "later")], &[patient("later", 1)]).await;
        let drain = sink.drain(&t, &sub, "m1").await;
        assert_eq!(
            target.ids(),
            vec!["under-way".to_string(), "later".to_string()]
        );
        assert_eq!(drain.rejected.len(), 1, "{:?}", drain.rejected);
        assert_eq!(drain.rejected[0].resource_id, "queued");
    }

    #[tokio::test(start_paused = true)]
    async fn the_drain_waits_out_a_page_that_legitimately_outlasts_max_wait() {
        let max_wait = Duration::from_millis(200);
        let target = Arc::new(SpyTarget {
            delay: Duration::from_millis(150),
            ..Default::default()
        });
        target.transient_once.lock().insert("slow".to_string());
        let sink = sink_with(
            target.clone(),
            empty_source(),
            IngestIndexSinkConfig {
                max_wait,
                ..Default::default()
            },
        );
        assert!(sink.settle_window() > max_wait * 3 + Duration::from_millis(1250));
        let sub = SubmissionId::generate("slow-page");
        // 150 ms, a transient failure, 250 ms of backoff, 150 ms again: the
        // page takes well over one `max_wait` without finishing any job.
        commit(&sink, &sub, &[success(1, "slow")], &[patient("slow", 1)]).await;
        let drain = sink.drain(&tenant(), &sub, "m1").await;
        assert!(!drain.timed_out);
        assert!(drain.rejected.is_empty(), "{:?}", drain.rejected);
        assert_eq!(drain.written, 1);
    }

    #[tokio::test]
    async fn a_stalled_target_degrades_batches_to_unindexed_within_a_bounded_time() {
        let target = Arc::new(SpyTarget::default());
        target.hang.store(true, Ordering::SeqCst);
        let max_wait = Duration::from_millis(100);
        let sink = sink_with(
            target.clone(),
            empty_source(),
            IngestIndexSinkConfig {
                queue: 1,
                concurrency: 1,
                coalesce: 1,
                max_wait,
            },
        );
        let sub = SubmissionId::generate("stalled");
        let started = std::time::Instant::now();
        for batch in 0..20u64 {
            let id = format!("s-{batch}");
            commit(&sink, &sub, &[success(batch, &id)], &[patient(&id, 1)]).await;
        }
        let ingest = started.elapsed();
        // One enqueue timeout, then the saturated queue rejects at once: the
        // ingest pays roughly one `max_wait`, never one per batch.
        assert!(
            ingest < max_wait * 6,
            "a stalled secondary must not stall the ingest: {ingest:?}"
        );

        let drain = tokio::time::timeout(Duration::from_secs(5), sink.drain(&tenant(), &sub, "m1"))
            .await
            .expect("the drain is bounded");
        assert_eq!(drain.written, 0);
        assert_eq!(drain.rejected.len(), 20, "{:?}", drain.rejected);
        assert_eq!(drain.rejected_types(), vec!["Patient".to_string()]);
    }

    #[tokio::test]
    async fn ids_are_read_back_from_the_source_when_the_engine_sends_no_resources() {
        let source = source_with(&["r1", "r2", "same"]).await;
        let target = Arc::new(SpyTarget::default());
        let sink = sink_with(target.clone(), source, IngestIndexSinkConfig::default());
        let sub = SubmissionId::generate("readback");
        commit(
            &sink,
            &sub,
            &[
                success(1, "r1"),
                success(2, "r2"),
                unchanged(3, "same"),
                BulkEntryResult::processing_error(
                    4,
                    "Patient",
                    json!({"resourceType": "OperationOutcome"}),
                ),
            ],
            &[],
        )
        .await;
        let drain = sink.drain(&tenant(), &sub, "m1").await;
        assert!(drain.rejected.is_empty(), "{:?}", drain.rejected);
        let mut ids = target.ids();
        ids.sort();
        assert_eq!(ids, vec!["r1", "r2", "same"]);
    }

    #[tokio::test]
    async fn unchanged_entries_are_read_back_even_when_the_engine_sends_resources() {
        // A replay or a reclaimed run leaves a resource unchanged in the
        // primary although its earlier indexing may never have landed.
        let source = source_with(&["same"]).await;
        let target = Arc::new(SpyTarget::default());
        let sink = sink_with(target.clone(), source, IngestIndexSinkConfig::default());
        let sub = SubmissionId::generate("unchanged");
        commit(
            &sink,
            &sub,
            &[success(1, "fresh"), unchanged(2, "same")],
            &[patient("fresh", 1)],
        )
        .await;
        let drain = sink.drain(&tenant(), &sub, "m1").await;
        assert!(drain.rejected.is_empty(), "{:?}", drain.rejected);
        assert_eq!(drain.written, 2);
        let mut ids = target.ids();
        ids.sort();
        assert_eq!(ids, vec!["fresh", "same"]);
    }
}
