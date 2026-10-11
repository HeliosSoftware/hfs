//! The Tenants page's shared, background tenant inventory (#1850, #1848).
//!
//! Which tenants hold data, and how many resources each holds, is a
//! cross-tenant scan: a grouped count on the SQL and document backends,
//! seconds at scale, and a paginated presence walk on S3. [`TenantInventory`]
//! runs that scan **once per mounted app/storage instance**, in the
//! background, and serves its last result to every request without awaiting
//! storage. One inventory lives in each mounted app's `WebState`, so two
//! mounted apps never share results, and nothing about a request (search
//! term, locale, FHIR version, user) selects a different refresh.
//!
//! ## Refresh ownership
//!
//! [`TenantInventory::view`] never awaits. When the snapshot is missing,
//! expired or marked stale, and no refresh is running or cooling down after
//! a failure, it starts one owner task with `tokio::spawn`. That task is the
//! only thing that ends a refresh:
//!
//! - It is never aborted. A request that waits for it (through
//!   [`TenantInventory::subscribe`]) and gives up, times out or disconnects
//!   leaves it running; the slot stays taken until the scan itself completes
//!   or fails, so a slow count never runs twice concurrently.
//! - It holds the inventory only through a `Weak`. Dropping the inventory
//!   (the app is torn down) lets the scan in flight finish, publishes
//!   nothing, starts nothing more, and releases the storage handle.
//! - A guard releases the slot and records a failure if the task ends any
//!   other way (a panic in the backend, runtime shutdown), so a crashed
//!   owner cannot pin the slot. The guard is armed before the task is
//!   spawned and moves into it, so a task dropped before its first poll
//!   releases the slot too.
//!
//! Refreshes are demand-driven only (#1848 D13): nothing scans while nobody
//! opens Tenants.
//!
//! ## Freshness and failure
//!
//! A completed snapshot is fresh for `max(min_ttl, ttl_per_refresh × the
//! refresh's own duration)` (#1848 D12): sixty seconds for a fast SQLite
//! count, proportionally longer for a scan that takes minutes, so a watched
//! page spends at most about a tenth of its time scanning. A failed refresh
//! keeps the previous snapshot on show as stale and blocks retries for a
//! cooldown that doubles per consecutive failure, from `cooldown_base` up to
//! `cooldown_max`; the first success clears it. Pending (never measured),
//! fresh, stale and unavailable (failed with nothing to show) are distinct
//! [`InventoryPhase`]s, and each cell distinguishes a measured zero from an
//! unknown, unsupported or presence-only answer ([`ResourceCell`]).
//!
//! ## Partial discovery
//!
//! Every discovery call carries a bounded request budget,
//! [`DEFAULT_MAX_REQUESTS_PER_SLICE`] (100) by default (#1848 D3). The SQL and
//! document backends answer with one grouped query and ignore it; S3 spends
//! it on LIST requests, delimiter pages and per-group probes alike, and
//! answers [`DiscoveryCoverage::Partial`] with a resume cursor when it runs
//! out. A partial answer is driven to the end inside the same owner task
//! (#1848 D14), one bounded slice per call. Each slice is published as
//! progress only when there is no complete snapshot to keep showing; absent
//! ids stay unknown until coverage is complete.
//!
//! Presence-only evidence (S3) cannot see hierarchical ids: a tenant such as
//! `acme/research` is never discovered under its own id (#1672). So an id
//! containing `/` that a complete listing without counts does not name reads
//! [`ResourceCell::Unknown`], never a measured zero (#1848 D23).
//!
//! ## Mutations
//!
//! - [`TenantInventory::invalidate_purged`]: a tenant's data was purged. It
//!   leaves the snapshot at once, and per-tenant generations reject any
//!   result for it from a refresh that began before the purge (#1848 D11);
//!   the rest of that result is still published. Exactly one follow-up
//!   refresh runs after the refresh in flight finishes, however many purges
//!   landed during it (with none in flight, the next view refreshes), so
//!   data written after the purge brings the tenant back.
//! - [`TenantInventory::mark_stale`]: provisioning finished, a tenant was
//!   deregistered, or a resource type of a tenant was purged. The snapshot
//!   stays on show, so data-only membership survives deregistration, and the
//!   next view refreshes it. Ordinary writes and single-resource purges are
//!   left to the TTL.
//!
//! The internal system tenant is never part of the inventory.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_persistence::core::{
    CountBasis, DiscoveryCoverage, DiscoveryRequest, ErasedScope, PresenceBasis, ResourceStorage,
    TenantDataEvidence, TenantDiscovery, WriteEvent, WriteObserver,
};
use helios_persistence::error::StorageResult;
use helios_persistence::tenant::SYSTEM_TENANT;
use tokio::sync::watch;

/// Where the inventory's data comes from: one cross-tenant discovery call.
///
/// The production source is the mounted storage handle
/// ([`ResourceStorage::discover_tenants`]); tests substitute a gated,
/// counting fake.
#[async_trait]
pub trait DiscoverySource: Send + Sync {
    /// One discovery call. See [`ResourceStorage::discover_tenants`] for the
    /// contract.
    async fn discover(&self, req: &DiscoveryRequest) -> StorageResult<TenantDiscovery>;
}

#[async_trait]
impl DiscoverySource for Arc<dyn ResourceStorage> {
    async fn discover(&self, req: &DiscoveryRequest) -> StorageResult<TenantDiscovery> {
        self.discover_tenants(req).await
    }
}

/// The inventory's notion of "now", injected so tests move time by hand.
pub trait Clock: Send + Sync {
    /// The current instant.
    fn now(&self) -> Instant;
}

/// The real monotonic clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Freshness, failure and work-bound policy (see the module docs).
#[derive(Clone)]
pub struct InventoryPolicy {
    /// The shortest time a completed snapshot is served without refreshing.
    pub min_ttl: Duration,
    /// A snapshot stays fresh for this many times the duration of the
    /// refresh that produced it, when that is longer than `min_ttl`.
    pub ttl_per_refresh: u32,
    /// Retry cooldown after the first consecutive failure; doubles per
    /// further failure.
    pub cooldown_base: Duration,
    /// The longest retry cooldown.
    pub cooldown_max: Duration,
    /// Request budget handed to each discovery call; `None` uses the
    /// backend's default, which for S3 is an unbounded walk.
    pub max_requests_per_slice: Option<NonZeroU32>,
    /// Stop a partial discovery after this many slices; `None` follows the
    /// resume cursor until coverage is complete or it stops advancing.
    pub max_slices: Option<NonZeroU32>,
    /// The clock every freshness decision reads.
    pub clock: Arc<dyn Clock>,
}

/// One minute: the floor of the adaptive TTL.
const MIN_TTL: Duration = Duration::from_secs(60);
/// A snapshot lives ten times as long as it took to produce.
const TTL_PER_REFRESH: u32 = 10;
/// First retry after fifteen seconds.
const COOLDOWN_BASE: Duration = Duration::from_secs(15);
/// Retries never wait more than five minutes.
const COOLDOWN_MAX: Duration = Duration::from_secs(300);
/// The default request budget of one discovery call (#1848 D3, D14). On S3
/// that is about a hundred tenant groups per slice (one delimiter page plus
/// one probe per group), so progress is published every hundred groups and a
/// slice that stops inside a page re-lists that page once. Backends that
/// answer with one grouped query ignore it.
pub const DEFAULT_MAX_REQUESTS_PER_SLICE: NonZeroU32 = NonZeroU32::new(100).unwrap();

impl Default for InventoryPolicy {
    fn default() -> Self {
        Self {
            min_ttl: MIN_TTL,
            ttl_per_refresh: TTL_PER_REFRESH,
            cooldown_base: COOLDOWN_BASE,
            cooldown_max: COOLDOWN_MAX,
            max_requests_per_slice: Some(DEFAULT_MAX_REQUESTS_PER_SLICE),
            max_slices: None,
            clock: Arc::new(SystemClock),
        }
    }
}

impl InventoryPolicy {
    /// How long a snapshot produced by a refresh that took `took` is fresh.
    fn ttl(&self, took: Duration) -> Duration {
        self.min_ttl.max(took.saturating_mul(self.ttl_per_refresh))
    }

    /// The retry cooldown after `failures` consecutive failures (at least 1).
    fn cooldown(&self, failures: u32) -> Duration {
        let doublings = failures.saturating_sub(1).min(16);
        self.cooldown_base
            .saturating_mul(1u32 << doublings)
            .min(self.cooldown_max)
    }
}

/// How fresh the inventory is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InventoryPhase {
    /// Nothing measured yet and no failure: the first refresh is running or
    /// about to.
    Pending,
    /// The snapshot is within its TTL and nothing invalidated it.
    Fresh,
    /// The snapshot is on show but out of date: expired, marked stale, or the
    /// last refresh failed (`last_error`).
    Stale {
        /// Why the latest refresh failed, if it did.
        last_error: Option<String>,
    },
    /// The latest refresh failed and there is no snapshot to show.
    Unavailable {
        /// Why it failed.
        error: String,
        /// No refresh starts before this instant.
        retry_after: Instant,
    },
}

/// What the inventory knows about one tenant's resources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceCell {
    /// An exact count of `basis` items, never zero.
    Number {
        /// How many.
        value: u64,
        /// What was counted; only [`CountBasis::LiveResources`] is an
        /// authoritative live-resource total.
        basis: CountBasis,
        /// The snapshot is stale.
        stale: bool,
    },
    /// Discovery covered every tenant and found no data for this one. Under
    /// presence-only evidence (S3) it means "no objects found" for a flat
    /// id; a hierarchical id (containing `/`) such evidence cannot see reads
    /// [`Unknown`](Self::Unknown) instead.
    MeasuredZero {
        /// The snapshot is stale.
        stale: bool,
    },
    /// The tenant holds data, but nothing was counted. Never a number.
    HasData {
        /// What the presence proves.
        basis: PresenceBasis,
    },
    /// A refresh that will answer for this tenant is running.
    Pending,
    /// Nothing is known: partial coverage without this tenant, a purge
    /// invalidated it and no refresh is running yet, or a hierarchical id
    /// that a presence-only listing cannot see.
    Unknown,
    /// The backend cannot discover tenant data.
    Unsupported,
    /// The latest refresh failed and nothing was ever measured.
    Unavailable,
}

/// The sum of every tenant's resources, when it can honestly be claimed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceTotal {
    /// Every tenant was counted, as live resources, by one complete
    /// discovery, and no purge invalidated part of it since.
    Exact {
        /// The total.
        value: u64,
        /// The snapshot is stale.
        stale: bool,
    },
    /// Partial, presence-only, unsupported, failed, pending or invalidated:
    /// no total.
    Unknown,
}

/// One read of the inventory. Cheap to take: the tenant map is shared.
#[derive(Clone, Debug)]
pub struct InventoryView {
    /// How fresh the snapshot is.
    pub phase: InventoryPhase,
    /// Tenants found with data, without the internal system tenant and
    /// without tenants purged since the snapshot's refresh began.
    pub discovered: Arc<BTreeMap<String, TenantDataEvidence>>,
    /// The snapshot's coverage; `None` before anything was measured.
    pub coverage: Option<DiscoveryCoverage>,
    /// When the snapshot (or its latest progress slice) completed.
    pub completed_at: Option<DateTime<Utc>>,
    /// A refresh is running.
    pub refreshing: bool,
    /// Tenants purged since the snapshot's refresh began: the snapshot cannot
    /// speak for them.
    pub invalidated: BTreeSet<String>,
    /// The snapshot's discovery counted tenants (a grouped count), so its
    /// absences speak for every id. Without counts (presence-only evidence,
    /// or no tenant found at all) an absent hierarchical id is unknown: S3
    /// discovery never names `acme/research` (#1672, #1848 D23).
    pub counted: bool,
}

impl InventoryView {
    /// What the inventory knows about `tenant`'s resources.
    pub fn cell(&self, tenant: &str) -> ResourceCell {
        if tenant == SYSTEM_TENANT {
            return ResourceCell::Unknown;
        }
        if self.invalidated.contains(tenant) {
            return self.pending_or_unknown();
        }
        let stale = matches!(self.phase, InventoryPhase::Stale { .. });
        let Some(coverage) = &self.coverage else {
            return match self.phase {
                InventoryPhase::Unavailable { .. } => ResourceCell::Unavailable,
                _ => ResourceCell::Pending,
            };
        };
        match self.discovered.get(tenant) {
            Some(TenantDataEvidence::Counted { resources, basis }) => ResourceCell::Number {
                value: *resources,
                basis: *basis,
                stale,
            },
            Some(TenantDataEvidence::Present { basis }) => ResourceCell::HasData { basis: *basis },
            None => match coverage {
                DiscoveryCoverage::Complete if !self.counted && tenant.contains('/') => {
                    ResourceCell::Unknown
                }
                DiscoveryCoverage::Complete => ResourceCell::MeasuredZero { stale },
                DiscoveryCoverage::Unsupported { .. } => ResourceCell::Unsupported,
                DiscoveryCoverage::Partial { .. } => self.pending_or_unknown(),
                _ => ResourceCell::Unknown,
            },
        }
    }

    /// The resource total across every tenant, only when one complete
    /// discovery counted every tenant's live resources (#1848 contract 6).
    ///
    /// A listing without counts never yields a total, even an empty one: a
    /// presence-only discovery (S3) that names no tenant may still hide
    /// hierarchical tenants holding data, so its "nothing found" is not a
    /// measured zero. Discovery does not report its evidence kind when it
    /// finds no tenant, so an empty grouped count reads `Unknown` too: the
    /// safe side of the unknown-versus-zero contract.
    pub fn resources_total(&self) -> ResourceTotal {
        if self.coverage != Some(DiscoveryCoverage::Complete)
            || !self.invalidated.is_empty()
            || !self.counted
        {
            return ResourceTotal::Unknown;
        }
        let mut value = 0u64;
        for evidence in self.discovered.values() {
            match evidence {
                TenantDataEvidence::Counted {
                    resources,
                    basis: CountBasis::LiveResources,
                } => value = value.saturating_add(*resources),
                _ => return ResourceTotal::Unknown,
            }
        }
        ResourceTotal::Exact {
            value,
            stale: matches!(self.phase, InventoryPhase::Stale { .. }),
        }
    }

    fn pending_or_unknown(&self) -> ResourceCell {
        if self.refreshing {
            ResourceCell::Pending
        } else {
            ResourceCell::Unknown
        }
    }
}

/// Work counters, for evidence and tests (#1848 D5). Never exported per
/// tenant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InventoryCounters {
    /// Refreshes started, follow-ups included.
    pub refreshes_started: u64,
    /// [`DiscoverySource::discover`] calls (one per slice).
    pub discover_calls: u64,
    /// Refreshes that published a result.
    pub refreshes_completed: u64,
    /// Refreshes that failed (or whose owner ended unexpectedly).
    pub refreshes_failed: u64,
    /// Follow-up refreshes run because a purge landed mid-refresh.
    pub follow_ups: u64,
    /// Per-tenant results dropped because the tenant was purged after the
    /// refresh began.
    pub rejected_entries: u64,
    /// [`TenantInventory::invalidate_purged`] calls.
    pub purges: u64,
    /// [`TenantInventory::mark_stale`] calls.
    pub marked_stale: u64,
}

/// A published result.
struct Snapshot {
    discovered: Arc<BTreeMap<String, TenantDataEvidence>>,
    coverage: DiscoveryCoverage,
    completed_at: Instant,
    completed_wall: DateTime<Utc>,
    ttl: Duration,
    /// The mutation sequence when the refresh that produced it began.
    as_of: u64,
    /// The discovery counted at least one tenant (see
    /// [`InventoryView::counted`]).
    counted: bool,
}

/// Whether `results` holds counts, not just presence.
fn any_counted(results: &BTreeMap<String, TenantDataEvidence>) -> bool {
    results
        .values()
        .any(|evidence| matches!(evidence, TenantDataEvidence::Counted { .. }))
}

/// The latest refresh failed.
struct Failure {
    error: String,
    consecutive: u32,
    retry_after: Instant,
}

#[derive(Default)]
struct State {
    snapshot: Option<Snapshot>,
    failure: Option<Failure>,
    /// The single-flight slot: an owner task is running.
    in_flight: bool,
    /// Bumped by every purge and stale mark, ordering them against
    /// refreshes.
    mutation_seq: u64,
    /// The mutation sequence of the latest stale mark: a snapshot whose
    /// refresh began before it is stale as a whole. (A purge only
    /// invalidates its own tenant; see `purged`.)
    stale_mark: u64,
    /// Purged tenant -> the mutation sequence of its latest purge. Entries
    /// at or below the snapshot's `as_of` are pruned at publication.
    purged: HashMap<String, u64>,
    counters: InventoryCounters,
}

impl State {
    fn expired(&self, snapshot: &Snapshot, now: Instant) -> bool {
        self.stale_mark > snapshot.as_of
            || now.saturating_duration_since(snapshot.completed_at) >= snapshot.ttl
    }

    /// A tenant was purged after the snapshot's refresh began, so the
    /// snapshot cannot answer for it.
    fn invalidates(&self, snapshot: &Snapshot) -> bool {
        self.purged.values().any(|seq| *seq > snapshot.as_of)
    }

    fn cooling_down(&self, now: Instant) -> bool {
        self.failure.as_ref().is_some_and(|f| now < f.retry_after)
    }

    fn wants_refresh(&self, now: Instant) -> bool {
        if self.in_flight || self.cooling_down(now) {
            return false;
        }
        match &self.snapshot {
            None => true,
            Some(snapshot) => {
                self.failure.is_some() || self.expired(snapshot, now) || self.invalidates(snapshot)
            }
        }
    }

    /// `results` minus tenants purged after `started` (the refresh's begin
    /// sequence), and how many were dropped.
    fn admit(
        &self,
        results: &BTreeMap<String, TenantDataEvidence>,
        started: u64,
    ) -> (BTreeMap<String, TenantDataEvidence>, u64) {
        let mut rejected = 0;
        let admitted = results
            .iter()
            .filter(|(id, _)| {
                let purged_since = self.purged.get(*id).is_some_and(|seq| *seq > started);
                rejected += u64::from(purged_since);
                !purged_since
            })
            .map(|(id, evidence)| (id.clone(), evidence.clone()))
            .collect();
        (admitted, rejected)
    }
}

/// The per-app tenant inventory. See the [module documentation](self).
pub struct TenantInventory {
    source: Arc<dyn DiscoverySource>,
    policy: InventoryPolicy,
    state: Mutex<State>,
    /// Bumped on every publication and mutation, for waiters.
    published: watch::Sender<u64>,
}

impl TenantInventory {
    /// An empty inventory over `source`. Starts nothing until the first
    /// [`view`](Self::view).
    pub fn new(source: Arc<dyn DiscoverySource>, policy: InventoryPolicy) -> Arc<Self> {
        Arc::new(Self {
            source,
            policy,
            state: Mutex::new(State::default()),
            published: watch::Sender::new(0),
        })
    }

    /// An inventory over the mounted storage handle, with the default policy.
    pub fn over_storage(storage: Arc<dyn ResourceStorage>) -> Arc<Self> {
        Self::new(Arc::new(storage), InventoryPolicy::default())
    }

    /// The current snapshot and its state. Never awaits storage: when a
    /// refresh is due and allowed, it starts the owner task and returns at
    /// once. Outside a Tokio runtime it starts nothing.
    pub fn view(self: &Arc<Self>) -> InventoryView {
        let now = self.policy.clock.now();
        let runtime = tokio::runtime::Handle::try_current().ok();
        let mut state = self.lock();
        let owner = match runtime {
            Some(runtime) if state.wants_refresh(now) => {
                state.in_flight = true;
                state.counters.refreshes_started += 1;
                Some(runtime)
            }
            _ => None,
        };
        let view = self.view_of(&state, now);
        drop(state);
        // Spawned with the lock released: a runtime that is shutting down
        // drops the task at once, and its guard then takes the lock.
        if let Some(runtime) = owner {
            self.spawn_owner(&runtime);
        }
        view
    }

    /// The tenant's data was purged: drop it from the snapshot now, reject it
    /// from any refresh that began earlier, and run one follow-up after the
    /// refresh in flight.
    pub fn invalidate_purged(&self, tenant: &str) {
        let mut state = self.lock();
        state.mutation_seq += 1;
        let seq = state.mutation_seq;
        state.purged.insert(tenant.to_string(), seq);
        state.counters.purges += 1;
        if let Some(snapshot) = state.snapshot.as_mut()
            && snapshot.discovered.contains_key(tenant)
        {
            Arc::make_mut(&mut snapshot.discovered).remove(tenant);
        }
        drop(state);
        self.notify();
    }

    /// Something changed that the snapshot may not reflect (provisioning
    /// finished, a tenant was deregistered, a resource type of a tenant was
    /// purged). The snapshot stays on show; the next view refreshes it.
    pub fn mark_stale(&self) {
        let mut state = self.lock();
        state.mutation_seq += 1;
        state.stale_mark = state.mutation_seq;
        state.counters.marked_stale += 1;
        drop(state);
        self.notify();
    }

    /// The work counters so far.
    pub fn metrics(&self) -> InventoryCounters {
        self.lock().counters
    }

    /// A receiver whose value changes on every publication and mutation, for
    /// a bounded wait. Dropping it, or giving up on it, never affects the
    /// refresh.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.published.subscribe()
    }

    /// A [`WriteObserver`] that feeds server-wide purges and deregistrations
    /// (the `/admin/tenants` API as well as this UI) into this inventory. It
    /// holds the inventory weakly and does nothing once it is gone.
    pub fn observer(self: &Arc<Self>) -> Arc<dyn WriteObserver> {
        Arc::new(InventoryObserver(Arc::downgrade(self)))
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn notify(&self) {
        self.published.send_modify(|version| *version += 1);
    }

    /// Spawns the owner task for the slot the caller just took. The armed
    /// guard moves into the task, so the slot is released however the task
    /// ends, including dropped unpolled by a runtime shutting down.
    fn spawn_owner(self: &Arc<Self>, runtime: &tokio::runtime::Handle) {
        let owner = Owner {
            inventory: Arc::downgrade(self),
            source: Arc::clone(&self.source),
            policy: self.policy.clone(),
        };
        let guard = SlotGuard {
            inventory: Arc::downgrade(self),
            armed: true,
        };
        runtime.spawn(owner.run(guard));
    }

    fn view_of(&self, state: &State, now: Instant) -> InventoryView {
        let phase = match (&state.snapshot, &state.failure) {
            (None, None) => InventoryPhase::Pending,
            (None, Some(failure)) => InventoryPhase::Unavailable {
                error: failure.error.clone(),
                retry_after: failure.retry_after,
            },
            (Some(_), Some(failure)) => InventoryPhase::Stale {
                last_error: Some(failure.error.clone()),
            },
            (Some(snapshot), None) if state.expired(snapshot, now) => {
                InventoryPhase::Stale { last_error: None }
            }
            (Some(_), None) => InventoryPhase::Fresh,
        };
        let as_of = state.snapshot.as_ref().map_or(0, |s| s.as_of);
        InventoryView {
            phase,
            discovered: state
                .snapshot
                .as_ref()
                .map(|s| Arc::clone(&s.discovered))
                .unwrap_or_default(),
            coverage: state.snapshot.as_ref().map(|s| s.coverage.clone()),
            completed_at: state.snapshot.as_ref().map(|s| s.completed_wall),
            refreshing: state.in_flight,
            invalidated: state
                .purged
                .iter()
                .filter(|(_, seq)| **seq > as_of)
                .map(|(id, _)| id.clone())
                .collect(),
            counted: state.snapshot.as_ref().is_some_and(|s| s.counted),
        }
    }

    /// Publishes one slice of a partial discovery, unless a complete
    /// snapshot is on show (it stays until the run completes).
    fn publish_progress(
        &self,
        results: &BTreeMap<String, TenantDataEvidence>,
        coverage: DiscoveryCoverage,
        started: u64,
    ) {
        let mut state = self.lock();
        let keeps_complete = state
            .snapshot
            .as_ref()
            .is_some_and(|s| !matches!(s.coverage, DiscoveryCoverage::Partial { .. }));
        if keeps_complete {
            return;
        }
        let (discovered, _) = state.admit(results, started);
        state.snapshot = Some(Snapshot {
            discovered: Arc::new(discovered),
            coverage,
            completed_at: self.policy.clock.now(),
            completed_wall: Utc::now(),
            ttl: self.policy.min_ttl,
            as_of: started,
            counted: any_counted(results),
        });
        drop(state);
        self.notify();
    }

    /// Ends a refresh that began at mutation sequence `started` and instant
    /// `began`. Returns whether a follow-up refresh must run now (the slot
    /// then stays taken); otherwise the slot is released.
    fn finish(
        &self,
        outcome: Result<(BTreeMap<String, TenantDataEvidence>, DiscoveryCoverage), String>,
        started: u64,
        began: Instant,
    ) -> bool {
        let now = self.policy.clock.now();
        let mut state = self.lock();
        let succeeded = outcome.is_ok();
        match outcome {
            Ok((results, coverage)) => {
                let counted = any_counted(&results);
                let (discovered, rejected) = state.admit(&results, started);
                state.counters.rejected_entries += rejected;
                state.counters.refreshes_completed += 1;
                state.snapshot = Some(Snapshot {
                    discovered: Arc::new(discovered),
                    coverage,
                    completed_at: now,
                    completed_wall: Utc::now(),
                    ttl: self.policy.ttl(now.saturating_duration_since(began)),
                    as_of: started,
                    counted,
                });
                state.failure = None;
            }
            Err(error) => {
                let consecutive = state.failure.as_ref().map_or(1, |f| f.consecutive + 1);
                state.counters.refreshes_failed += 1;
                state.failure = Some(Failure {
                    error,
                    consecutive,
                    retry_after: now + self.policy.cooldown(consecutive),
                });
            }
        }
        // A purge during this refresh owes exactly one follow-up, run after
        // it (#1848 contract 5). A failure leaves the retry to the cooldown.
        let purged_during = state.purged.values().any(|seq| *seq > started);
        let follow_up = succeeded && purged_during;
        if succeeded {
            state.purged.retain(|_, seq| *seq > started);
        }
        if follow_up {
            state.counters.follow_ups += 1;
            state.counters.refreshes_started += 1;
        } else {
            state.in_flight = false;
        }
        drop(state);
        self.notify();
        follow_up
    }
}

/// The owner task's handles: the inventory only weakly.
struct Owner {
    inventory: Weak<TenantInventory>,
    source: Arc<dyn DiscoverySource>,
    policy: InventoryPolicy,
}

/// Releases the slot if the owner task ends without finishing (a panic in
/// the source, or the runtime dropping the task, polled or not).
struct SlotGuard {
    inventory: Weak<TenantInventory>,
    armed: bool,
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Some(inventory) = self.inventory.upgrade() {
            let mut state = inventory.lock();
            let now = inventory.policy.clock.now();
            let consecutive = state.failure.as_ref().map_or(1, |f| f.consecutive + 1);
            state.counters.refreshes_failed += 1;
            state.failure = Some(Failure {
                error: "the tenant inventory refresh ended unexpectedly".to_string(),
                consecutive,
                retry_after: now + inventory.policy.cooldown(consecutive),
            });
            state.in_flight = false;
            drop(state);
            inventory.notify();
        }
    }
}

impl Owner {
    async fn run(self, mut guard: SlotGuard) {
        loop {
            let Some(started) = self.begin() else {
                guard.armed = false;
                return;
            };
            let began = self.policy.clock.now();
            let Some(outcome) = self.discover_all(started).await else {
                // The inventory is gone: nothing to publish or release.
                guard.armed = false;
                return;
            };
            let Some(inventory) = self.inventory.upgrade() else {
                guard.armed = false;
                return;
            };
            if !inventory.finish(outcome, started, began) {
                guard.armed = false;
                return;
            }
        }
    }

    /// The mutation sequence this refresh's results will speak for.
    fn begin(&self) -> Option<u64> {
        Some(self.inventory.upgrade()?.lock().mutation_seq)
    }

    /// Runs discovery to the end: one call, or a slice per call following the
    /// resume cursor. `None` when the inventory was dropped meanwhile.
    async fn discover_all(
        &self,
        started: u64,
    ) -> Option<Result<(BTreeMap<String, TenantDataEvidence>, DiscoveryCoverage), String>> {
        let mut results = BTreeMap::new();
        let mut request = DiscoveryRequest {
            max_requests: self.policy.max_requests_per_slice,
            resume: None,
        };
        let mut slices = 0u32;
        loop {
            self.inventory.upgrade()?.lock().counters.discover_calls += 1;
            let answer = self.source.discover(&request).await;
            let inventory = self.inventory.upgrade()?;
            slices += 1;
            let discovery = match answer {
                Ok(discovery) => discovery,
                Err(error) => return Some(Err(error.to_string())),
            };
            for tenant in discovery.tenants {
                if tenant.id != SYSTEM_TENANT {
                    // A tenant seen twice across slices keeps its latest answer.
                    results.insert(tenant.id, tenant.evidence);
                }
            }
            let resume = match &discovery.coverage {
                DiscoveryCoverage::Partial {
                    resume: Some(cursor),
                } if request.resume.as_ref() != Some(cursor)
                    && self.policy.max_slices.is_none_or(|max| slices < max.get()) =>
                {
                    cursor.clone()
                }
                _ => return Some(Ok((results, discovery.coverage))),
            };
            inventory.publish_progress(&results, discovery.coverage, started);
            request.resume = Some(resume);
        }
    }
}

/// Feeds write events into an inventory (see [`TenantInventory::observer`]).
struct InventoryObserver(Weak<TenantInventory>);

impl WriteObserver for InventoryObserver {
    /// The inventory is gone (its app was torn down): the server's fan-out
    /// drops this observer on its next subscription.
    fn retired(&self) -> bool {
        self.0.strong_count() == 0
    }

    fn on_write(&self, event: &WriteEvent) {
        let Some(inventory) = self.0.upgrade() else {
            return;
        };
        match event {
            WriteEvent::Erased {
                tenant,
                scope: ErasedScope::Tenant,
            } => inventory.invalidate_purged(tenant.as_str()),
            WriteEvent::Erased {
                scope: ErasedScope::Type(_),
                ..
            }
            | WriteEvent::TenantRemoved { .. } => inventory.mark_stale(),
            // Ordinary writes, and instance purges (which change a count no
            // more than a delete does), are left to the TTL: marking stale on
            // each would rescan continuously under an import or a cleanup
            // script, however long the last scan took.
            WriteEvent::Resource(_)
            | WriteEvent::Counts { .. }
            | WriteEvent::Erased {
                scope: ErasedScope::Instance { .. },
                ..
            } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helios_persistence::core::{DiscoveredTenant, DiscoveryCursor};
    use helios_persistence::error::{BackendError, StorageError};
    use helios_persistence::tenant::TenantId;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Semaphore;

    /// Bounds every wait so a broken test fails instead of hanging. Never an
    /// assertion about elapsed time.
    const HANG_GUARD: Duration = Duration::from_secs(10);

    /// A clock moved by hand.
    struct ManualClock {
        base: Instant,
        offset: Mutex<Duration>,
    }

    impl ManualClock {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                base: Instant::now(),
                offset: Mutex::new(Duration::ZERO),
            })
        }

        fn advance(&self, by: Duration) {
            *self.offset.lock().unwrap() += by;
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> Instant {
            self.base + *self.offset.lock().unwrap()
        }
    }

    /// One scripted answer.
    enum Step {
        Answer(TenantDiscovery),
        Fail(&'static str),
        Panic,
    }

    /// A discovery source that counts calls, records requests, answers from a
    /// script (an empty complete store once it runs out), and, when gated,
    /// holds each call until the test releases one permit.
    struct FakeSource {
        calls: AtomicUsize,
        entered: watch::Sender<usize>,
        requests: Mutex<Vec<DiscoveryRequest>>,
        script: Mutex<VecDeque<Step>>,
        gate: Option<Semaphore>,
    }

    impl FakeSource {
        fn new(gated: bool, script: Vec<Step>) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                entered: watch::Sender::new(0),
                requests: Mutex::new(Vec::new()),
                script: Mutex::new(script.into()),
                gate: gated.then(|| Semaphore::new(0)),
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        /// Lets one held call (or the next one) finish.
        fn release(&self) {
            self.gate.as_ref().expect("gated source").add_permits(1);
        }

        /// Waits until `n` calls have entered the source.
        async fn entered(&self, n: usize) {
            let mut rx = self.entered.subscribe();
            tokio::time::timeout(HANG_GUARD, rx.wait_for(|calls| *calls >= n))
                .await
                .expect("the source was called")
                .expect("sender alive");
        }
    }

    #[async_trait]
    impl DiscoverySource for FakeSource {
        async fn discover(&self, req: &DiscoveryRequest) -> StorageResult<TenantDiscovery> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.requests.lock().unwrap().push(req.clone());
            self.entered.send_modify(|n| *n += 1);
            if let Some(gate) = &self.gate {
                gate.acquire().await.expect("gate open").forget();
            }
            let step = self.script.lock().unwrap().pop_front();
            match step {
                Some(Step::Answer(discovery)) => Ok(discovery),
                Some(Step::Fail(message)) => {
                    Err(StorageError::Backend(BackendError::Unavailable {
                        backend_name: "fake".to_string(),
                        message: message.to_string(),
                    }))
                }
                Some(Step::Panic) => panic!("scripted discovery panic"),
                None => Ok(TenantDiscovery::from_grouped_counts(
                    Vec::new(),
                    CountBasis::LiveResources,
                )),
            }
        }
    }

    fn counted(rows: &[(&str, u64)]) -> Step {
        Step::Answer(TenantDiscovery::from_grouped_counts(
            rows.iter().map(|(id, n)| (id.to_string(), *n)).collect(),
            CountBasis::LiveResources,
        ))
    }

    fn present(ids: &[&str], coverage: DiscoveryCoverage) -> Step {
        Step::Answer(TenantDiscovery {
            tenants: ids
                .iter()
                .map(|id| DiscoveredTenant {
                    id: id.to_string(),
                    evidence: TenantDataEvidence::Present {
                        basis: PresenceBasis::ResourceObjects,
                    },
                })
                .collect(),
            coverage,
        })
    }

    fn policy(clock: &Arc<ManualClock>) -> InventoryPolicy {
        InventoryPolicy {
            clock: Arc::clone(clock) as Arc<dyn Clock>,
            ..InventoryPolicy::default()
        }
    }

    fn inventory(source: &Arc<FakeSource>, clock: &Arc<ManualClock>) -> Arc<TenantInventory> {
        TenantInventory::new(
            Arc::clone(source) as Arc<dyn DiscoverySource>,
            policy(clock),
        )
    }

    /// Waits until the inventory has published at least `n` times since the
    /// receiver was taken.
    async fn published(rx: &mut watch::Receiver<u64>, at_least: u64) {
        tokio::time::timeout(HANG_GUARD, rx.wait_for(|v| *v >= at_least))
            .await
            .expect("published in time")
            .expect("sender alive");
    }

    /// Waits until no refresh is running.
    async fn idle(inventory: &TenantInventory) {
        let mut rx = inventory.subscribe();
        tokio::time::timeout(HANG_GUARD, rx.wait_for(|_| !inventory.lock().in_flight))
            .await
            .expect("the refresh ended in time")
            .expect("sender alive");
    }

    fn number(value: u64, stale: bool) -> ResourceCell {
        ResourceCell::Number {
            value,
            basis: CountBasis::LiveResources,
            stale,
        }
    }

    /// Concurrent reads of a cold inventory start one discovery; reads of the
    /// fresh result start none. No request input selects another refresh:
    /// `view` takes none.
    #[tokio::test]
    async fn concurrent_views_share_one_discovery_and_fresh_reads_start_none() {
        let clock = ManualClock::new();
        let source = FakeSource::new(true, vec![counted(&[("acme", 3)])]);
        let inventory = inventory(&source, &clock);

        let views: Vec<InventoryView> = (0..50).map(|_| inventory.view()).collect();
        source.entered(1).await;
        assert!(views.iter().all(|v| v.phase == InventoryPhase::Pending));
        assert!(views.iter().all(|v| v.refreshing));
        assert_eq!(views[0].cell("acme"), ResourceCell::Pending);
        assert_eq!(source.calls(), 1, "one discovery for fifty readers");

        source.release();
        idle(&inventory).await;
        for _ in 0..50 {
            let view = inventory.view();
            assert_eq!(view.phase, InventoryPhase::Fresh);
            assert_eq!(view.cell("acme"), number(3, false));
            assert!(view.completed_at.is_some());
        }
        assert_eq!(source.calls(), 1, "fresh reads start nothing");
        let metrics = inventory.metrics();
        assert_eq!(metrics.refreshes_started, 1);
        assert_eq!(metrics.discover_calls, 1);
        assert_eq!(metrics.refreshes_completed, 1);
    }

    /// The waiter that triggered a refresh can be dropped, or give up on a
    /// timeout, without freeing the slot: the owner finishes, warms the
    /// cache, and later readers reuse it.
    #[tokio::test]
    async fn abandoned_waiters_and_wait_timeouts_never_free_the_slot() {
        let clock = ManualClock::new();
        let source = FakeSource::new(true, vec![counted(&[("acme", 2)])]);
        let inventory = inventory(&source, &clock);

        // A request that starts the refresh and waits for it, then is dropped
        // (the browser navigated away).
        let waiter = tokio::spawn({
            let inventory = Arc::clone(&inventory);
            async move {
                let mut rx = inventory.subscribe();
                inventory.view();
                let _ = rx.changed().await;
            }
        });
        source.entered(1).await;
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());

        // Another request's bounded wait elapses while the work is held.
        let mut rx = inventory.subscribe();
        let waited = tokio::time::timeout(Duration::from_millis(20), rx.changed()).await;
        assert!(waited.is_err(), "the gate is closed, so the wait times out");

        let view = inventory.view();
        assert!(view.refreshing, "the slot is still taken");
        assert_eq!(view.phase, InventoryPhase::Pending);
        assert_eq!(
            source.calls(),
            1,
            "no second discovery while the first runs"
        );

        source.release();
        idle(&inventory).await;
        let view = inventory.view();
        assert_eq!(view.phase, InventoryPhase::Fresh);
        assert_eq!(view.cell("acme"), number(2, false));
        assert_eq!(source.calls(), 1, "the abandoned refresh warmed the cache");
    }

    /// A failure with nothing measured is "unavailable", not zero; retries
    /// wait out the cooldown; a later success recovers.
    #[tokio::test]
    async fn a_cold_failure_is_unavailable_until_a_retry_after_the_cooldown_succeeds() {
        let clock = ManualClock::new();
        let source = FakeSource::new(
            false,
            vec![Step::Fail("pool timeout"), counted(&[("acme", 1)])],
        );
        let inventory = inventory(&source, &clock);

        inventory.view();
        idle(&inventory).await;
        let view = inventory.view();
        assert!(matches!(
            &view.phase,
            InventoryPhase::Unavailable { error, .. } if error.contains("pool timeout")
        ));
        assert_eq!(view.cell("acme"), ResourceCell::Unavailable);
        assert_eq!(view.cell("other"), ResourceCell::Unavailable);
        assert_eq!(view.resources_total(), ResourceTotal::Unknown);
        assert_eq!(source.calls(), 1);

        // Inside the cooldown, reads start nothing.
        clock.advance(COOLDOWN_BASE - Duration::from_secs(1));
        assert!(!inventory.view().refreshing);
        assert_eq!(source.calls(), 1);

        // The first read after it starts exactly one retry, which recovers.
        clock.advance(Duration::from_secs(1));
        assert!(inventory.view().refreshing);
        idle(&inventory).await;
        let view = inventory.view();
        assert_eq!(view.phase, InventoryPhase::Fresh);
        assert_eq!(view.cell("acme"), number(1, false));
        assert_eq!(source.calls(), 2);
        let metrics = inventory.metrics();
        assert_eq!(
            (metrics.refreshes_failed, metrics.refreshes_completed),
            (1, 1)
        );
    }

    /// A count-only failure after a success keeps the useful result on show
    /// as stale, backs off with a doubling cooldown, and recovers.
    #[tokio::test]
    async fn a_failure_after_success_keeps_the_stale_result_and_backs_off() {
        let clock = ManualClock::new();
        let source = FakeSource::new(
            false,
            vec![
                counted(&[("acme", 4)]),
                Step::Fail("statement timeout"),
                Step::Fail("statement timeout"),
                counted(&[("acme", 5)]),
            ],
        );
        let inventory = inventory(&source, &clock);
        inventory.view();
        idle(&inventory).await;
        let first = inventory.view();
        let completed_at = first.completed_at;

        clock.advance(MIN_TTL);
        assert!(inventory.view().refreshing, "an expired snapshot refreshes");
        idle(&inventory).await;
        let view = inventory.view();
        assert!(matches!(
            &view.phase,
            InventoryPhase::Stale { last_error: Some(error) } if error.contains("statement timeout")
        ));
        assert_eq!(
            view.cell("acme"),
            number(4, true),
            "the stale count is kept"
        );
        assert_eq!(
            view.cell("none"),
            ResourceCell::MeasuredZero { stale: true }
        );
        assert_eq!(view.completed_at, completed_at, "the stale result's time");
        assert_eq!(
            view.resources_total(),
            ResourceTotal::Exact {
                value: 4,
                stale: true
            }
        );

        // First cooldown: the base. Second: doubled.
        clock.advance(COOLDOWN_BASE);
        assert!(inventory.view().refreshing);
        idle(&inventory).await;
        assert_eq!(source.calls(), 3);
        clock.advance(COOLDOWN_BASE);
        assert!(
            !inventory.view().refreshing,
            "the second cooldown is longer"
        );
        assert_eq!(source.calls(), 3);
        clock.advance(COOLDOWN_BASE);
        assert!(inventory.view().refreshing);
        idle(&inventory).await;
        let view = inventory.view();
        assert_eq!(view.phase, InventoryPhase::Fresh);
        assert_eq!(view.cell("acme"), number(5, false));
        assert_eq!(source.calls(), 4);
    }

    /// The TTL adapts to the refresh's own duration: a slow scan is reused
    /// for ten times as long as it took, never less than a minute.
    #[tokio::test]
    async fn the_ttl_adapts_to_how_long_the_refresh_took() {
        let clock = ManualClock::new();
        let source = FakeSource::new(true, vec![counted(&[("acme", 1)]), counted(&[("acme", 2)])]);
        let inventory = inventory(&source, &clock);
        inventory.view();
        source.entered(1).await;
        clock.advance(Duration::from_secs(20)); // the scan takes 20 s
        source.release();
        idle(&inventory).await;

        clock.advance(Duration::from_secs(199));
        let view = inventory.view();
        assert_eq!(view.phase, InventoryPhase::Fresh, "200 s TTL, not 60 s");
        assert!(!view.refreshing);
        clock.advance(Duration::from_secs(1));
        let view = inventory.view();
        assert_eq!(view.phase, InventoryPhase::Stale { last_error: None });
        assert!(view.refreshing, "stale-while-revalidate");
        assert_eq!(view.cell("acme"), number(1, true));
        source.release();
        idle(&inventory).await;
        assert_eq!(inventory.view().cell("acme"), number(2, false));
        assert_eq!(source.calls(), 2);
    }

    #[test]
    fn policy_ttl_and_cooldown_values() {
        let policy = InventoryPolicy::default();
        assert_eq!(policy.ttl(Duration::from_millis(30)), MIN_TTL);
        assert_eq!(policy.ttl(Duration::from_secs(7)), Duration::from_secs(70));
        assert_eq!(policy.cooldown(1), Duration::from_secs(15));
        assert_eq!(policy.cooldown(2), Duration::from_secs(30));
        assert_eq!(policy.cooldown(5), Duration::from_secs(240));
        assert_eq!(policy.cooldown(6), COOLDOWN_MAX);
        assert_eq!(policy.cooldown(1_000), COOLDOWN_MAX);
    }

    /// A purge during a gated refresh: the pre-purge result for the purged
    /// tenant is rejected before publication, the rest is published, exactly
    /// one follow-up runs after the in-flight job (however many purges
    /// landed), and data written after the purge brings the tenant back.
    #[tokio::test]
    async fn a_purge_mid_refresh_rejects_the_tenant_and_coalesces_one_follow_up() {
        let clock = ManualClock::new();
        let source = FakeSource::new(
            true,
            vec![
                counted(&[("acme", 9), ("beta", 2)]),
                // The follow-up sees a post-purge write to acme.
                counted(&[("acme", 1), ("beta", 2)]),
            ],
        );
        let inventory = inventory(&source, &clock);
        let mut rx = inventory.subscribe();
        inventory.view();
        source.entered(1).await;

        inventory.invalidate_purged("acme");
        inventory.invalidate_purged("acme");
        inventory.invalidate_purged("beta-never-seen");
        assert_eq!(inventory.view().cell("acme"), ResourceCell::Pending);

        let before = *rx.borrow_and_update();
        source.release();
        published(&mut rx, before + 1).await;
        source.entered(2).await;
        let view = inventory.view();
        assert!(
            !view.discovered.contains_key("acme"),
            "the pre-purge count is rejected"
        );
        assert_eq!(
            view.cell("acme"),
            ResourceCell::Pending,
            "the follow-up will answer"
        );
        assert_eq!(view.cell("beta"), number(2, false), "the rest is published");
        assert_eq!(view.resources_total(), ResourceTotal::Unknown);
        assert!(view.refreshing, "the follow-up holds the same slot");
        assert_eq!(source.calls(), 2);

        source.release();
        idle(&inventory).await;
        let view = inventory.view();
        assert_eq!(
            view.cell("acme"),
            number(1, false),
            "a post-purge write is real"
        );
        assert!(view.invalidated.is_empty());
        assert_eq!(
            view.resources_total(),
            ResourceTotal::Exact {
                value: 3,
                stale: false
            }
        );
        assert_eq!(source.calls(), 2, "one follow-up, not one per purge");
        let metrics = inventory.metrics();
        assert_eq!(metrics.follow_ups, 1);
        assert_eq!(metrics.rejected_entries, 1);
        assert_eq!(metrics.purges, 3);
        assert_eq!(metrics.refreshes_started, 2);
    }

    /// A purge while idle drops the tenant at once and invalidates its
    /// count; the next read refreshes.
    #[tokio::test]
    async fn a_purge_while_idle_drops_the_tenant_and_the_next_read_refreshes() {
        let clock = ManualClock::new();
        let source = FakeSource::new(false, vec![counted(&[("acme", 3), ("beta", 1)])]);
        let inventory = inventory(&source, &clock);
        inventory.view();
        idle(&inventory).await;

        inventory.invalidate_purged("acme");
        let view = inventory.view();
        assert!(!view.discovered.contains_key("acme"));
        assert!(view.refreshing, "the purge made the snapshot stale");
        assert_eq!(view.cell("acme"), ResourceCell::Pending);
        idle(&inventory).await;
        let view = inventory.view();
        assert_eq!(
            view.cell("acme"),
            ResourceCell::MeasuredZero { stale: false }
        );
        assert_eq!(
            view.cell("beta"),
            ResourceCell::MeasuredZero { stale: false }
        );
        assert_eq!(inventory.metrics().follow_ups, 0);
    }

    /// Deregistration (a stale mark) keeps data-only membership; the internal
    /// system tenant never appears; a stale mark mid-refresh waits for the
    /// next read instead of forcing a follow-up.
    #[tokio::test]
    async fn deregistration_keeps_data_only_tenants_and_the_system_tenant_is_excluded() {
        let clock = ManualClock::new();
        let source = FakeSource::new(
            true,
            vec![
                counted(&[("orphan", 7), (SYSTEM_TENANT, 40)]),
                counted(&[("orphan", 7), (SYSTEM_TENANT, 41)]),
            ],
        );
        let inventory = inventory(&source, &clock);
        inventory.view();
        source.entered(1).await;
        inventory.mark_stale(); // a deregistration while the scan runs
        source.release();
        idle(&inventory).await;
        assert_eq!(source.calls(), 1, "a stale mark forces no follow-up");

        let view = inventory.view();
        assert_eq!(view.phase, InventoryPhase::Stale { last_error: None });
        assert!(view.refreshing, "the next read refreshes");
        assert_eq!(
            view.cell("orphan"),
            number(7, true),
            "data-only membership is kept"
        );
        assert!(!view.discovered.contains_key(SYSTEM_TENANT));
        assert_eq!(view.cell(SYSTEM_TENANT), ResourceCell::Unknown);
        source.release();
        idle(&inventory).await;
        let view = inventory.view();
        assert_eq!(view.discovered.keys().collect::<Vec<_>>(), vec!["orphan"]);
        assert_eq!(
            view.resources_total(),
            ResourceTotal::Exact {
                value: 7,
                stale: false
            },
            "the system tenant is not in the total"
        );
    }

    /// Unsupported discovery is never an empty measured store, and presence
    /// never becomes a number or a total.
    #[tokio::test]
    async fn unsupported_and_presence_only_never_read_as_numbers() {
        let clock = ManualClock::new();
        let source = FakeSource::new(
            false,
            vec![Step::Answer(TenantDiscovery::unsupported(
                "s3-bucket-per-tenant-discovery",
            ))],
        );
        let inventory = inventory(&source, &clock);
        inventory.view();
        idle(&inventory).await;
        let view = inventory.view();
        assert_eq!(view.phase, InventoryPhase::Fresh);
        assert_eq!(view.cell("acme"), ResourceCell::Unsupported);
        assert_eq!(view.resources_total(), ResourceTotal::Unknown);

        let presence =
            FakeSource::new(false, vec![present(&["acme"], DiscoveryCoverage::Complete)]);
        let presence = TenantInventory::new(presence as Arc<dyn DiscoverySource>, policy(&clock));
        presence.view();
        idle(&presence).await;
        let view = presence.view();
        assert_eq!(
            view.cell("acme"),
            ResourceCell::HasData {
                basis: PresenceBasis::ResourceObjects
            }
        );
        assert_eq!(
            view.cell("empty"),
            ResourceCell::MeasuredZero { stale: false }
        );
        assert_eq!(view.resources_total(), ResourceTotal::Unknown);
    }

    /// A hierarchical id is invisible to presence-only discovery (S3 never
    /// names `acme/research`), so a complete listing without counts cannot
    /// call it a measured zero; a grouped count can.
    #[tokio::test]
    async fn presence_only_listings_never_zero_a_hierarchical_id() {
        let clock = ManualClock::new();
        let source = FakeSource::new(false, vec![present(&["acme"], DiscoveryCoverage::Complete)]);
        let inventory = inventory(&source, &clock);
        inventory.view();
        idle(&inventory).await;
        let view = inventory.view();
        assert!(!view.counted);
        assert_eq!(view.cell("acme/research"), ResourceCell::Unknown);
        assert_eq!(
            view.cell("beta"),
            ResourceCell::MeasuredZero { stale: false }
        );

        // An empty listing proves nothing about evidence kind either.
        let source = FakeSource::new(false, vec![present(&[], DiscoveryCoverage::Complete)]);
        let empty = TenantInventory::new(
            Arc::clone(&source) as Arc<dyn DiscoverySource>,
            policy(&clock),
        );
        empty.view();
        idle(&empty).await;
        assert_eq!(empty.view().cell("acme/research"), ResourceCell::Unknown);
        assert_eq!(
            empty.view().resources_total(),
            ResourceTotal::Unknown,
            "an empty presence-only listing is not an exact zero total"
        );

        let source = FakeSource::new(false, vec![counted(&[("acme", 2)])]);
        let counting = TenantInventory::new(
            Arc::clone(&source) as Arc<dyn DiscoverySource>,
            policy(&clock),
        );
        counting.view();
        idle(&counting).await;
        let view = counting.view();
        assert!(view.counted);
        assert_eq!(
            view.cell("acme/research"),
            ResourceCell::MeasuredZero { stale: false },
            "a grouped count answers for every id"
        );
        // Purging the only counted tenant does not turn the count into
        // presence.
        counting.invalidate_purged("acme");
        let view = counting.view();
        assert!(view.counted);
        assert_eq!(view.cell("acme"), ResourceCell::Pending);
        assert_eq!(
            view.cell("acme/research"),
            ResourceCell::MeasuredZero { stale: false }
        );
        idle(&counting).await;
    }

    /// A complete presence-only listing that leaves no user tenant (S3 with
    /// data only under `acme/research`, or only under the system tenant)
    /// has measured nothing, so it has no total, never an exact zero
    /// (#1848 contract 6).
    #[tokio::test]
    async fn an_empty_presence_only_listing_has_no_total() {
        let clock = ManualClock::new();
        for step in [
            present(&[], DiscoveryCoverage::Complete),
            present(&[SYSTEM_TENANT], DiscoveryCoverage::Complete),
        ] {
            let source = FakeSource::new(false, vec![step]);
            let inventory = inventory(&source, &clock);
            inventory.view();
            idle(&inventory).await;
            let view = inventory.view();
            assert!(view.discovered.is_empty());
            assert_eq!(view.coverage, Some(DiscoveryCoverage::Complete));
            assert!(view.invalidated.is_empty());
            assert!(!view.counted);
            assert_eq!(view.cell("acme/research"), ResourceCell::Unknown);
            assert_eq!(
                view.resources_total(),
                ResourceTotal::Unknown,
                "presence that found nothing is not a measured zero"
            );
        }
    }

    /// The default policy bounds every discovery call (#1848 D3, D14), so S3
    /// answers in slices instead of walking every tenant group in one call.
    #[tokio::test]
    async fn the_default_policy_sends_a_bounded_request_budget() {
        assert_eq!(
            InventoryPolicy::default().max_requests_per_slice,
            Some(DEFAULT_MAX_REQUESTS_PER_SLICE)
        );
        assert_eq!(DEFAULT_MAX_REQUESTS_PER_SLICE.get(), 100);
        let clock = ManualClock::new();
        let source = FakeSource::new(false, Vec::new());
        let inventory = inventory(&source, &clock);
        inventory.view();
        idle(&inventory).await;
        let requests = source.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].max_requests,
            Some(DEFAULT_MAX_REQUESTS_PER_SLICE)
        );
    }

    /// A partial discovery is followed to the end inside the one owner task,
    /// resuming from each cursor and publishing progress on a cold inventory;
    /// a purge between slices is rejected before each publication.
    #[tokio::test]
    async fn partial_discovery_is_driven_to_completion_by_the_owner() {
        let clock = ManualClock::new();
        let partial = |cursor: &str| DiscoveryCoverage::Partial {
            resume: Some(DiscoveryCursor::new(cursor)),
        };
        let source = FakeSource::new(
            true,
            vec![
                present(&["a", "b"], partial("b")),
                present(&["c"], partial("c")),
                present(&["b", "d"], DiscoveryCoverage::Complete),
                // The purge's follow-up.
                present(&["b", "c", "d"], DiscoveryCoverage::Complete),
            ],
        );
        let mut policy = policy(&clock);
        policy.max_requests_per_slice = NonZeroU32::new(8);
        let inventory =
            TenantInventory::new(Arc::clone(&source) as Arc<dyn DiscoverySource>, policy);
        let mut rx = inventory.subscribe();
        inventory.view();

        source.release();
        published(&mut rx, 1).await;
        let view = inventory.view();
        assert!(matches!(
            view.coverage,
            Some(DiscoveryCoverage::Partial { .. })
        ));
        assert!(view.refreshing);
        assert!(matches!(view.cell("a"), ResourceCell::HasData { .. }));
        assert_eq!(
            view.cell("z"),
            ResourceCell::Pending,
            "absent is unknown, not zero"
        );

        inventory.invalidate_purged("a");
        source.release();
        source.release();
        // The run completed and its follow-up started: look before it ends.
        source.entered(4).await;
        let view = inventory.view();
        assert_eq!(view.coverage, Some(DiscoveryCoverage::Complete));
        assert_eq!(
            view.discovered.keys().collect::<Vec<_>>(),
            vec!["b", "c", "d"],
            "slices merge; the purged tenant is rejected"
        );
        source.release();
        idle(&inventory).await;
        let requests = source.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 4, "three slices plus the purge's follow-up");
        assert_eq!(requests[0].resume, None);
        assert_eq!(requests[1].resume, Some(DiscoveryCursor::new("b")));
        assert_eq!(requests[2].resume, Some(DiscoveryCursor::new("c")));
        assert!(
            requests
                .iter()
                .all(|r| r.max_requests == NonZeroU32::new(8))
        );
        assert_eq!(inventory.metrics().refreshes_started, 2);
    }

    /// A cursor that does not advance, or the slice cap, ends the run with
    /// partial coverage instead of looping.
    #[tokio::test]
    async fn a_stuck_cursor_or_the_slice_cap_ends_a_partial_run() {
        let clock = ManualClock::new();
        let stuck = || {
            present(
                &["a"],
                DiscoveryCoverage::Partial {
                    resume: Some(DiscoveryCursor::new("a")),
                },
            )
        };
        let source = FakeSource::new(false, vec![stuck(), stuck(), stuck()]);
        let inventory = inventory(&source, &clock);
        inventory.view();
        idle(&inventory).await;
        assert_eq!(source.calls(), 2, "the repeated cursor stops the run");
        let view = inventory.view();
        assert!(matches!(
            view.coverage,
            Some(DiscoveryCoverage::Partial { .. })
        ));
        assert_eq!(view.cell("zzz"), ResourceCell::Unknown);

        let partial = |cursor: &str| {
            present(
                &[cursor],
                DiscoveryCoverage::Partial {
                    resume: Some(DiscoveryCursor::new(cursor)),
                },
            )
        };
        let source = FakeSource::new(false, vec![partial("a"), partial("b"), partial("c")]);
        let mut capped = policy(&clock);
        capped.max_slices = NonZeroU32::new(2);
        let inventory =
            TenantInventory::new(Arc::clone(&source) as Arc<dyn DiscoverySource>, capped);
        inventory.view();
        idle(&inventory).await;
        assert_eq!(source.calls(), 2, "the slice cap stops the run");
    }

    /// Two inventories (two mounted apps) never read each other's results.
    #[tokio::test]
    async fn two_inventories_are_isolated() {
        let clock = ManualClock::new();
        let first_source = FakeSource::new(false, vec![counted(&[("one", 1)])]);
        let second_source = FakeSource::new(false, vec![counted(&[("two", 2)])]);
        let first = inventory(&first_source, &clock);
        let second = inventory(&second_source, &clock);
        first.view();
        second.view();
        idle(&first).await;
        idle(&second).await;
        assert_eq!(first.view().cell("one"), number(1, false));
        assert_eq!(
            first.view().cell("two"),
            ResourceCell::MeasuredZero { stale: false }
        );
        assert_eq!(second.view().cell("two"), number(2, false));
        assert_eq!(
            second.view().cell("one"),
            ResourceCell::MeasuredZero { stale: false }
        );
        first.invalidate_purged("one");
        assert_eq!(second.metrics().purges, 0);
        assert_eq!((first_source.calls(), second_source.calls()), (1, 1));
    }

    /// Dropping the inventory mid-refresh lets the scan finish, publishes
    /// nothing, starts nothing more, and releases the source (the storage).
    #[tokio::test]
    async fn dropping_the_inventory_mid_refresh_releases_the_source() {
        let clock = ManualClock::new();
        let source = FakeSource::new(true, vec![counted(&[("acme", 1)])]);
        let inventory = inventory(&source, &clock);
        inventory.view();
        source.entered(1).await;
        inventory.invalidate_purged("acme"); // would owe a follow-up

        let observer = inventory.observer();
        let weak_source = Arc::downgrade(&source);
        drop(inventory);
        source.release();
        let source_calls = source.calls();
        drop(source);
        tokio::time::timeout(HANG_GUARD, async {
            while weak_source.upgrade().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the owner task let go of the source");
        assert_eq!(source_calls, 1, "no follow-up for a dropped inventory");
        // Events for a dropped inventory are ignored, and the fan-out may
        // drop the observer.
        observer.on_write(&WriteEvent::TenantRemoved {
            tenant: TenantId::new("acme"),
        });
        assert!(observer.retired());
    }

    /// A source that panics cannot pin the slot: the owner's guard records a
    /// failure, and a read after the cooldown retries once.
    #[tokio::test]
    async fn a_panicking_source_releases_the_slot_as_a_failure() {
        let clock = ManualClock::new();
        let source = FakeSource::new(false, vec![Step::Panic, counted(&[("acme", 1)])]);
        let inventory = inventory(&source, &clock);
        inventory.view();
        idle(&inventory).await;
        let view = inventory.view();
        assert!(matches!(view.phase, InventoryPhase::Unavailable { .. }));
        assert!(!view.refreshing);
        clock.advance(COOLDOWN_BASE);
        assert!(inventory.view().refreshing);
        idle(&inventory).await;
        assert_eq!(inventory.view().cell("acme"), number(1, false));
        assert_eq!(source.calls(), 2);
    }

    /// An owner task the runtime drops before its first poll (the runtime
    /// shut down right after the view) still releases the slot, as a
    /// failure, so a later runtime can refresh after the cooldown.
    #[test]
    fn an_owner_dropped_before_it_ran_releases_the_slot() {
        let clock = ManualClock::new();
        let source = FakeSource::new(false, vec![counted(&[("acme", 1)])]);
        let inventory = inventory(&source, &clock);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let view = {
            let _entered = runtime.enter();
            inventory.view()
        };
        assert!(view.refreshing, "the slot was taken");
        drop(runtime);

        assert_eq!(source.calls(), 0, "the owner never ran");
        let view = inventory.view();
        assert!(!view.refreshing, "the slot was released");
        assert!(matches!(view.phase, InventoryPhase::Unavailable { .. }));
        assert_eq!(inventory.metrics().refreshes_failed, 1);

        clock.advance(COOLDOWN_BASE);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            assert!(inventory.view().refreshing);
            idle(&inventory).await;
        });
        assert_eq!(inventory.view().cell("acme"), number(1, false));
        assert_eq!(source.calls(), 1);
    }

    /// Outside a Tokio runtime nothing starts and nothing is claimed.
    #[test]
    fn outside_a_runtime_a_view_starts_nothing() {
        let clock = ManualClock::new();
        let source = FakeSource::new(false, Vec::new());
        let inventory = inventory(&source, &clock);
        let view = inventory.view();
        assert_eq!(view.phase, InventoryPhase::Pending);
        assert!(!view.refreshing);
        assert_eq!(source.calls(), 0);
        assert_eq!(inventory.metrics(), InventoryCounters::default());
    }

    /// Server-wide events reach the inventory through its observer: a tenant
    /// purge invalidates, a deregistration or type purge marks stale, and
    /// ordinary writes and instance purges are left to the TTL.
    #[tokio::test]
    async fn write_events_invalidate_or_mark_stale() {
        let clock = ManualClock::new();
        let source = FakeSource::new(false, vec![counted(&[("acme", 2), ("beta", 1)])]);
        let inventory = inventory(&source, &clock);
        inventory.view();
        idle(&inventory).await;
        let observer = inventory.observer();

        observer.on_write(&WriteEvent::Counts {
            tenant: TenantId::new("acme"),
            resource_type: "Patient".to_string(),
            created: 1,
            updated: 0,
            deleted: 0,
            origin: helios_persistence::core::WriteOrigin::BulkSubmit,
            at: Utc::now(),
        });
        observer.on_write(&WriteEvent::Erased {
            tenant: TenantId::new("acme"),
            scope: ErasedScope::Instance {
                resource_type: "Patient".to_string(),
                id: "p1".to_string(),
            },
        });
        assert_eq!(inventory.metrics().marked_stale, 0);
        assert_eq!(inventory.metrics().purges, 0);
        assert_eq!(inventory.view().phase, InventoryPhase::Fresh);

        observer.on_write(&WriteEvent::TenantRemoved {
            tenant: TenantId::new("beta"),
        });
        observer.on_write(&WriteEvent::Erased {
            tenant: TenantId::new("beta"),
            scope: ErasedScope::Type("Patient".to_string()),
        });
        assert_eq!(inventory.metrics().marked_stale, 2);

        observer.on_write(&WriteEvent::Erased {
            tenant: TenantId::new("acme"),
            scope: ErasedScope::Tenant,
        });
        assert_eq!(inventory.metrics().purges, 1);
        assert!(!inventory.view().discovered.contains_key("acme"));
    }
}
