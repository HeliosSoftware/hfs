//! In-memory, per-`(tenant, resource type)` live resource counters for the web
//! UI's "FHIR Resources over Time" chart (#1078).
//!
//! # Why this exists
//!
//! The chart used to be computed only from storage aggregates — a `GROUP BY`
//! count of the live rows plus a per-type bucketed scan of the history log.
//! Those are exactly the queries that stop finishing while a large import is
//! writing, so the one moment an operator most wants to watch the chart move
//! (a bulk load) was the moment it sat on "Waiting…" forever.
//!
//! The write paths (REST create/update/delete, Bundles, `$bulk-submit`) report
//! each committed write to the server's post-commit write observer, which feeds
//! [`record`](DashboardCounters::record) the net live-count change, and the
//! dashboard reads the result in O(1). There is no process-global instance: the
//! server creates one set of counters and injects it into both the observer and
//! the dashboard provider. Storage is
//! still the source of truth: a background reconcile periodically re-reads the
//! totals ([`begin_reconcile`](DashboardCounters::begin_reconcile) /
//! [`finish_reconcile`](DashboardCounters::finish_reconcile)) and each window's
//! history ring ([`begin_ring_seed`](DashboardCounters::begin_ring_seed) /
//! [`finish_ring_seed`](DashboardCounters::finish_ring_seed)), correcting any
//! drift the counters accumulated.
//!
//! # Counts are process-local
//!
//! Every figure here is what *this process* has observed. A multi-instance
//! deployment sharing one PostgreSQL or MongoDB database only sees the writes
//! that landed on the local instance; writes made through a sibling instance,
//! direct database edits, and write paths that do not record (see the callers)
//! are only picked up by the next background reconcile. Between reconciles the
//! figures are therefore approximate, and the views say so through their
//! `exact` flags so the UI can label them.
//!
//! # Storage markers: noticing writes this process did not see
//!
//! "No local write since the last reconcile" alone cannot vouch for a figure:
//! a sibling instance may have written in the meantime. So the caller also
//! reads a cheap storage *write marker* ([`StorageMarker`]: the newest write
//! time and a recent-write count, as far as the backend can tell) and hands it
//! over at three points:
//!
//! - [`begin_reconcile`](DashboardCounters::begin_reconcile) takes the marker
//!   read right *before* the storage count, and
//!   [`finish_reconcile`](DashboardCounters::finish_reconcile) the one read
//!   right *after* it. If they differ, storage changed while it was being
//!   counted: the figures are applied but the tenant keeps
//!   [`external_change`](TotalsView::external_change) set, so it stays
//!   approximate and the next pass reconciles it again. If they match, the
//!   after-marker becomes the tenant's *reconciled marker* and the flag clears.
//! - [`note_marker`](DashboardCounters::note_marker) is a probe between
//!   reconciles: a marker different from the reconciled one that no local
//!   write explains sets `external_change` and asks the caller to reconcile
//!   now. The before-marker given to `begin_reconcile` is checked the same way,
//!   so a change is noticed even when no probe ran.
//!
//! A marker cannot tell a local write from a foreign one, so once this process
//! wrote, any marker change is treated as possibly external. Every detection
//! also bumps the tenant's *external epoch*; a history ring is only exact when
//! it was seeded under the current epoch, so a clean totals reconcile never
//! vouches for a ring loaded before the change. Without markers (a backend
//! that cannot provide them passes `None`), a reconcile that corrects drift the
//! local writes cannot explain bumps the epoch the same way.
//!
//! # The begin/finish subtraction
//!
//! A reconcile cannot atomically read storage and the counters together, so it
//! brackets the storage read with a token:
//!
//! 1. [`begin_reconcile`](DashboardCounters::begin_reconcile) snapshots every
//!    type's cumulative live delta.
//! 2. The caller reads storage (the read starts *after* begin).
//! 3. [`finish_reconcile`](DashboardCounters::finish_reconcile) sets each
//!    type's base to the storage figure and keeps only the live delta recorded
//!    *since begin* (`live_now - live_at_begin`).
//!
//! A write recorded before begin is therefore dropped — storage already holds
//! it. A write recorded after begin is kept, which is right when storage did
//! not see it yet, but a write that committed after begin and *before* the
//! storage read finished is in both, and is counted twice until the next quiet
//! reconcile. That is why [`TotalsView::exact`] is only `true` when no write
//! was recorded for the tenant since the last successful reconcile began, no
//! external change is flagged, and the tenant was not invalidated since; and
//! [`CountersSeries::exact`] likewise per type since its ring seed began. The
//! history rings use the same scheme bucket-wise. Tokens are ordered by their
//! begin: a token begun before one that already finished is
//! [superseded](ReconcileOutcome::Superseded).
//!
//! # Periodic ring re-seeds
//!
//! A type written continuously never becomes exact again: every seed overlaps
//! a write. [`rings_to_reseed`](DashboardCounters::rings_to_reseed) tells the
//! reconcile loop which viewed rings to reload: a ring that is not exact is
//! re-seeded at once when the tenant recorded no write since its seed (it is
//! inexact because of an invalidation or an external change, so a reload fixes
//! it), and otherwise once its seed is older than a maximum age, so even under
//! a sustained import the history is reloaded from storage periodically.
//!
//! # Measured, never invented (#956)
//!
//! These are measured figures. A tenant whose totals have never been reconciled
//! from storage has no trustworthy base — the counters only know the deltas of
//! the writes they saw — so [`totals_view`](DashboardCounters::totals_view) and
//! [`series_view`](DashboardCounters::series_view) return `None` for it rather
//! than zeros, and the dashboard keeps showing its "waiting" state instead of
//! an invented empty chart.
//!
//! # Invalidation keeps the last figures
//!
//! A purge erases storage behind the counters' back.
//! [`invalidate_tenant`](DashboardCounters::invalidate_tenant) does not forget
//! the tenant — a dashboard that went blank after every purge would wait on a
//! full storage read to show anything again. It keeps the last totals and
//! rings, starts a new generation (so every reconcile or ring seed begun
//! before the purge is rejected when it finishes: those reads may describe
//! erased data), and marks the tenant as
//! [needing a reseed](TotalsView::needs_reseed): its totals and every series
//! are no longer `exact`, and every ring reports `history_seeded == false`
//! until it is loaded again. The next successful reconcile replaces the base
//! with storage's figures, and each ring seed replaces that ring's history.
//!
//! # Views, eviction and removal
//!
//! State is kept per tenant for as long as it is useful, not forever.
//! [`note_viewed`](DashboardCounters::note_viewed) stamps a dashboard view;
//! [`tenants_by_priority`](DashboardCounters::tenants_by_priority) orders the
//! reconcile work (recently viewed first) so a pass with a bounded budget
//! spends it where someone is looking; and
//! [`evict_idle`](DashboardCounters::evict_idle) drops tenants nobody viewed
//! for a while (their next view seeds them again from storage).
//! [`remove_tenant`](DashboardCounters::remove_tenant) drops a deregistered
//! tenant's state at once. After an eviction or a removal every outstanding
//! token for the tenant is [invalidated](ReconcileOutcome::Invalidated) — a
//! later incarnation always gets a new generation — and finishing one never
//! recreates the tenant.
//!
//! # Buckets
//!
//! Each `(type, window)` keeps a fixed ring of [`DashboardWindow::points`]
//! buckets, [`DashboardWindow::bucket_seconds`] wide and epoch-aligned
//! (`secs.div_euclid(bucket) * bucket`, the same flooring the persistence
//! layer's history bucketing uses), so a ring bucket and a storage bucket with
//! the same start describe the same interval. Each ring has two layers: a
//! static `seeded` layer loaded from storage history and a `live` layer of
//! recorded writes; a displayed bucket is their sum.
//!
//! Everything is synchronous and in-memory; the locks are held only for a few
//! arithmetic operations and never across an `.await`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use crate::dashboard::DashboardWindow;

/// Number of [`DashboardWindow`] variants, i.e. rings kept per type.
const WINDOWS: usize = DashboardWindow::ALL.len();

/// Ring slot of a window.
fn window_index(window: DashboardWindow) -> usize {
    match window {
        DashboardWindow::LastHour => 0,
        DashboardWindow::LastDay => 1,
        DashboardWindow::LastMonth => 2,
    }
}

/// Epoch-aligned start (in seconds) of the bucket containing `secs`.
fn bucket_floor(secs: i64, bucket: i64) -> i64 {
    secs.div_euclid(bucket) * bucket
}

/// `u64` → `i64`, saturating instead of wrapping.
fn to_i64(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// Clamps a possibly negative count at zero.
fn clamp_count(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

/// Locks a mutex, recovering the data if a panicking holder poisoned it — the
/// counters are plain integers, so a torn update is at worst an approximate
/// figure that the next reconcile corrects.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A cheap reading of "has storage changed" for one tenant, as far as the
/// backend can tell (see the module docs). Two markers compare equal when
/// storage saw no write between the two reads; a backend that cannot tell a
/// field leaves it `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageMarker {
    /// Epoch milliseconds of the newest write storage holds for the tenant.
    pub latest_millis: Option<i64>,
    /// Number of recent writes storage holds for the tenant.
    pub recent_writes: Option<u64>,
}

/// Result of [`DashboardCounters::finish_reconcile`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconcileOutcome {
    /// Storage's figures replaced the tenant's base.
    Applied {
        /// Sum over types of `|total after − total before|`: how far the
        /// counters had drifted from storage. `0` for a first seed and for the
        /// reseed after an [invalidation](DashboardCounters::invalidate_tenant),
        /// whose previous figures were not a claim about current storage.
        correction: u64,
    },
    /// A reconcile begun after this one already finished; nothing changed,
    /// and the tenant holds figures at least as fresh.
    Superseded,
    /// The tenant was invalidated, evicted or removed since this reconcile
    /// began; nothing changed (and a removed tenant was not recreated).
    Invalidated,
}

/// One fixed-size bucket ring for a `(type, window)`.
#[derive(Debug)]
struct Ring {
    /// Bucket width in seconds.
    bucket: i64,
    /// Epoch seconds of the newest bucket's start; `None` until first touched.
    newest: Option<i64>,
    /// Recorded writes, oldest first (index `len - 1` is `newest`).
    live: Vec<i64>,
    /// Storage history loaded by the last ring seed, aligned with `live`.
    seeded: Vec<i64>,
    /// Tenant write sequence at the begin of the last successful ring seed;
    /// `None` until the ring has been seeded from storage (or since an
    /// invalidation).
    seed_seq: Option<u64>,
    /// Begin order of the last successful ring seed, to supersede older tokens.
    seed_begin: Option<u64>,
    /// When the last successful ring seed was applied; `None` with `seed_seq`.
    seeded_at: Option<DateTime<Utc>>,
    /// Tenant external epoch at the begin of the last successful ring seed.
    seed_epoch: u64,
}

impl Ring {
    fn new(window: DashboardWindow) -> Self {
        let points = window.points();
        Self {
            bucket: window.bucket_seconds(),
            newest: None,
            live: vec![0; points],
            seeded: vec![0; points],
            seed_seq: None,
            seed_begin: None,
            seeded_at: None,
            seed_epoch: 0,
        }
    }

    fn points(&self) -> usize {
        self.live.len()
    }

    /// Moves the ring forward so its newest bucket starts at `target` (already
    /// floored), zeroing the buckets skipped over. Never moves backwards.
    fn advance(&mut self, target: i64) {
        match self.newest {
            None => self.newest = Some(target),
            Some(newest) if target > newest => {
                let points = self.points();
                let steps = (target - newest) / self.bucket;
                let steps = usize::try_from(steps).unwrap_or(usize::MAX);
                if steps >= points {
                    self.live.fill(0);
                    self.seeded.fill(0);
                } else {
                    for layer in [&mut self.live, &mut self.seeded] {
                        layer.copy_within(steps.., 0);
                        layer[points - steps..].fill(0);
                    }
                }
                self.newest = Some(target);
            }
            Some(_) => {}
        }
    }

    /// Slot of the bucket starting at `start` (already floored), if it lies
    /// inside the ring's current span.
    fn slot(&self, start: i64) -> Option<usize> {
        let newest = self.newest?;
        if start > newest {
            return None;
        }
        let back = usize::try_from((newest - start) / self.bucket).ok()?;
        (back < self.points()).then(|| self.points() - 1 - back)
    }

    /// Adds a recorded write at `secs`, advancing to its bucket first. Writes
    /// older than the ring's oldest bucket are ignored.
    fn record(&mut self, secs: i64, delta: i64) {
        let start = bucket_floor(secs, self.bucket);
        self.advance(start);
        if let Some(slot) = self.slot(start) {
            self.live[slot] = self.live[slot].saturating_add(delta);
        }
    }

    /// Displayed value (seeded + live) of the bucket starting at `start`, or 0
    /// when the ring does not cover it.
    fn displayed_at(&self, start: i64) -> i64 {
        self.slot(start)
            .map(|slot| self.seeded[slot].saturating_add(self.live[slot]))
            .unwrap_or(0)
    }

    /// Live layer as `(newest, buckets)`, for a ring-seed token.
    fn live_snapshot(&self) -> Option<(i64, Vec<i64>)> {
        self.newest.map(|newest| (newest, self.live.clone()))
    }
}

/// Counter state of one resource type within a tenant.
#[derive(Debug)]
struct TypeCounters {
    /// Live count read from storage by the last reconcile (0 before any).
    base: i64,
    /// Cumulative sum of every delta recorded for this type.
    live: i64,
    /// Value of `live` that `base` already accounts for.
    live_at_base: i64,
    /// Tenant write sequence of the last write recorded for this type (0 =
    /// none).
    last_write_seq: u64,
    /// One ring per window, indexed by [`window_index`].
    rings: [Ring; WINDOWS],
}

impl TypeCounters {
    fn new() -> Self {
        Self {
            base: 0,
            live: 0,
            live_at_base: 0,
            last_write_seq: 0,
            rings: DashboardWindow::ALL.map(Ring::new),
        }
    }

    fn total(&self) -> u64 {
        clamp_count(
            self.base
                .saturating_add(self.live.saturating_sub(self.live_at_base)),
        )
    }
}

/// When the tenant's totals were last reconciled.
#[derive(Debug, Clone, Copy)]
struct Reconciled {
    at: DateTime<Utc>,
    /// Tenant write sequence at that reconcile's begin.
    seq: u64,
    /// Begin order of that reconcile, to supersede older tokens.
    begin: u64,
}

/// All counter state of one tenant.
#[derive(Debug)]
struct TenantCounters {
    /// Identity of this incarnation of the tenant's state; a token whose
    /// generation differs was begun before an invalidation, an eviction or a
    /// removal.
    generation: u64,
    /// Incremented for every recorded (non-zero) write.
    write_seq: u64,
    /// Incremented by every reconcile or ring-seed begin; orders tokens.
    begins: u64,
    reconciled: Option<Reconciled>,
    /// Storage changed behind the counters (a purge): the figures are kept
    /// but not exact until the next successful reconcile.
    needs_reseed: bool,
    /// A storage marker changed in a way no local write is known to explain;
    /// cleared by a reconcile whose before and after markers match.
    external_change: bool,
    /// Incremented on every detected (possibly) external storage change; a
    /// ring seeded under an older epoch is not exact.
    external_epoch: u64,
    /// Storage marker read right after the last applied reconcile's count.
    reconciled_marker: Option<StorageMarker>,
    /// When this incarnation was created (idle time of a never-viewed tenant).
    created_at: Instant,
    /// Most recent [`DashboardCounters::note_viewed`].
    last_viewed: Option<Instant>,
    types: HashMap<String, TypeCounters>,
}

impl TenantCounters {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            write_seq: 0,
            begins: 0,
            reconciled: None,
            needs_reseed: false,
            external_change: false,
            external_epoch: 0,
            reconciled_marker: None,
            created_at: Instant::now(),
            last_viewed: None,
            types: HashMap::new(),
        }
    }

    fn has_state(&self) -> bool {
        self.reconciled.is_some() || !self.types.is_empty()
    }

    fn type_mut(&mut self, resource_type: &str) -> &mut TypeCounters {
        if !self.types.contains_key(resource_type) {
            self.types
                .insert(resource_type.to_string(), TypeCounters::new());
        }
        self.types
            .get_mut(resource_type)
            .expect("type counters inserted above")
    }

    fn next_begin(&mut self) -> u64 {
        self.begins += 1;
        self.begins
    }

    /// No write was recorded since the last applied reconcile began.
    fn quiet_since_reconcile(&self) -> bool {
        self.reconciled.is_some_and(|r| self.write_seq <= r.seq)
    }

    /// Flags a (possibly) external storage change.
    fn flag_external_change(&mut self) {
        self.external_change = true;
        self.external_epoch += 1;
    }

    /// Compares `marker` with the reconciled marker; see
    /// [`DashboardCounters::note_marker`].
    fn check_marker(&mut self, marker: StorageMarker) -> bool {
        match self.reconciled_marker {
            Some(reconciled) if reconciled != marker => {
                self.flag_external_change();
                self.quiet_since_reconcile()
            }
            _ => false,
        }
    }

    fn ring_exact(&self, tc: &TypeCounters, ring: &Ring) -> bool {
        ring.seed_seq.is_some_and(|seq| tc.last_write_seq <= seq)
            && !self.external_change
            && ring.seed_epoch == self.external_epoch
    }

    /// Idle time as of `now`: since the last view, or since creation.
    fn idle(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.last_viewed.unwrap_or(self.created_at))
    }
}

type TenantMap = HashMap<String, Arc<Mutex<TenantCounters>>>;

/// Process-local live resource counters, keyed by tenant and resource type.
///
/// See the [module documentation](self) for the consistency model.
#[derive(Debug)]
pub struct DashboardCounters {
    tenants: RwLock<TenantMap>,
    /// Source of tenant generations; never reused, so a token from a removed
    /// tenant incarnation can never match a later one.
    next_generation: AtomicU64,
}

impl Default for DashboardCounters {
    fn default() -> Self {
        Self::new()
    }
}

/// Proof that a totals reconcile began, carried to
/// [`DashboardCounters::finish_reconcile`].
#[derive(Debug, Clone)]
pub struct ReconcileToken {
    tenant: String,
    generation: u64,
    seq: u64,
    begin: u64,
    /// Storage marker read right before the count.
    marker: Option<StorageMarker>,
    live_at_begin: HashMap<String, i64>,
}

/// Proof that a ring seed began, carried to
/// [`DashboardCounters::finish_ring_seed`].
#[derive(Debug, Clone)]
pub struct RingSeedToken {
    tenant: String,
    generation: u64,
    resource_type: String,
    window: DashboardWindow,
    seq: u64,
    begin: u64,
    external_epoch: u64,
    /// The type's live ring layer at begin, as `(newest, buckets)`.
    live_at_begin: Option<(i64, Vec<i64>)>,
}

/// A tenant's per-type live totals.
#[derive(Clone, Debug)]
pub struct TotalsView {
    /// Every type with counter state (`base + live`, clamped at 0), largest
    /// first, ties by name.
    pub totals: Vec<(String, u64)>,
    /// When the totals were last reconciled from storage.
    pub reconciled_at: DateTime<Utc>,
    /// No write was recorded for the tenant since the last successful
    /// reconcile began, no external storage change is flagged, and it was not
    /// invalidated since, so the figures equal what storage reported.
    pub exact: bool,
    /// The tenant was [invalidated](DashboardCounters::invalidate_tenant)
    /// since the last successful reconcile: these are the last known figures,
    /// kept on show until a reconcile replaces them.
    pub needs_reseed: bool,
    /// A storage marker showed a change this process may not have recorded
    /// (see the module docs); cleared by the next reconcile whose storage did
    /// not change while it was counted.
    pub external_change: bool,
}

/// One resource type's total and bucketed deltas over a window.
#[derive(Clone, Debug)]
pub struct CountersSeries {
    /// The resource type.
    pub resource_type: String,
    /// Current live total (clamped at 0).
    pub total: u64,
    /// Dense `(bucket_start, delta)` entries — [`DashboardWindow::points`] of
    /// them, oldest first; the last is the bucket containing `now`.
    pub buckets: Vec<(DateTime<Utc>, i64)>,
    /// The ring's seeded layer was loaded from storage history for this
    /// window; without it the buckets only hold writes this process saw.
    pub history_seeded: bool,
    /// When that history was loaded (`None` exactly when `history_seeded` is
    /// `false`).
    pub history_seeded_at: Option<DateTime<Utc>>,
    /// `history_seeded`, no write was recorded for this type since that seed
    /// began, no external storage change is flagged, and none was detected
    /// since the seed began.
    pub exact: bool,
}

impl DashboardCounters {
    /// Creates an empty counter set.
    pub fn new() -> Self {
        Self {
            tenants: RwLock::new(HashMap::new()),
            next_generation: AtomicU64::new(1),
        }
    }

    fn read_map(&self) -> std::sync::RwLockReadGuard<'_, TenantMap> {
        self.tenants.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write_map(&self) -> std::sync::RwLockWriteGuard<'_, TenantMap> {
        self.tenants.write().unwrap_or_else(|e| e.into_inner())
    }

    /// Runs `f` on the tenant's state, creating it if absent. The map's read
    /// guard is held for the duration so an eviction or removal cannot drop
    /// the entry mid-update (the write lands before or after it, never into a
    /// detached incarnation).
    fn with_tenant_or_create<R>(
        &self,
        tenant: &str,
        f: impl FnOnce(&mut TenantCounters) -> R,
    ) -> R {
        {
            let map = self.read_map();
            if let Some(entry) = map.get(tenant) {
                return f(&mut lock(entry));
            }
        }
        let mut map = self.write_map();
        let entry = map.entry(tenant.to_string()).or_insert_with(|| {
            Arc::new(Mutex::new(TenantCounters::new(
                self.next_generation.fetch_add(1, Ordering::Relaxed),
            )))
        });
        f(&mut lock(entry))
    }

    /// Runs `f` on the tenant's state if it exists.
    fn with_tenant<R>(&self, tenant: &str, f: impl FnOnce(&mut TenantCounters) -> R) -> Option<R> {
        let map = self.read_map();
        map.get(tenant).map(|entry| f(&mut lock(entry)))
    }

    /// Net live-count change of a committed write for `(tenant, type)` at
    /// `at`: `+n` for creates, `-n` for deletes. `delta == 0` is a no-op.
    pub fn record(&self, tenant: &str, resource_type: &str, delta: i64, at: DateTime<Utc>) {
        if delta == 0 {
            return;
        }
        let secs = at.timestamp();
        self.with_tenant_or_create(tenant, |t| {
            t.write_seq += 1;
            let seq = t.write_seq;
            let tc = t.type_mut(resource_type);
            tc.live = tc.live.saturating_add(delta);
            tc.last_write_seq = seq;
            for ring in &mut tc.rings {
                ring.record(secs, delta);
            }
        });
    }

    /// Marks a tenant's figures stale after storage changed behind the
    /// counters (purge / tenant data wipe), keeping the last figures on show.
    ///
    /// The totals and rings are kept, but a new generation starts: every
    /// reconcile or ring seed begun before this is rejected by its `finish_*`
    /// call. The totals and every series stop being `exact`
    /// ([`TotalsView::needs_reseed`]), and each ring reports
    /// `history_seeded == false`, until a reconcile and ring seeds begun after
    /// this replace them with storage's figures. The reconciled storage marker
    /// is forgotten (it describes the erased data). A tenant with no counter
    /// state is left alone.
    pub fn invalidate_tenant(&self, tenant: &str) {
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        self.with_tenant(tenant, |t| {
            t.generation = generation;
            t.needs_reseed = true;
            t.reconciled_marker = None;
            for tc in t.types.values_mut() {
                for ring in &mut tc.rings {
                    ring.seed_seq = None;
                    ring.seeded_at = None;
                }
            }
        });
    }

    /// Totals have been reconciled from storage at least once. Stays `true`
    /// after an [invalidation](Self::invalidate_tenant), whose figures are kept
    /// (see [`needs_reseed`](Self::needs_reseed)).
    pub fn is_seeded(&self, tenant: &str) -> bool {
        self.with_tenant(tenant, |t| t.reconciled.is_some())
            .unwrap_or(false)
    }

    /// The tenant was [invalidated](Self::invalidate_tenant) and no reconcile
    /// begun since has finished: its figures are the last known ones, not
    /// storage's.
    pub fn needs_reseed(&self, tenant: &str) -> bool {
        self.with_tenant(tenant, |t| t.needs_reseed)
            .unwrap_or(false)
    }

    /// Tenants with any counter state (recorded writes or seeded), sorted.
    pub fn tenants(&self) -> Vec<String> {
        let map = self.read_map();
        let mut tenants: Vec<String> = map
            .iter()
            .filter(|(_, entry)| lock(entry).has_state())
            .map(|(tenant, _)| tenant.clone())
            .collect();
        tenants.sort();
        tenants
    }

    /// Records a probe of the tenant's storage write marker between
    /// reconciles, returning `true` when the caller should reconcile the
    /// tenant now.
    ///
    /// When the tenant has a reconciled marker and `marker` differs from it,
    /// storage changed since that reconcile: `external_change` is set (the
    /// totals and series stop being `exact`, and rings seeded before now need
    /// a reload). If no local write was recorded since the reconcile began, no
    /// local write can explain the change, and `true` is returned. If local
    /// writes were recorded, the change cannot be told apart from them: the
    /// flag is still set, but the figures were already approximate, so `false`
    /// is returned and the regular reconcile picks it up.
    ///
    /// An unknown tenant, a tenant without a reconciled marker (never
    /// reconciled with markers, or invalidated since) and an unchanged marker
    /// all return `false` and change nothing.
    pub fn note_marker(&self, tenant: &str, marker: StorageMarker) -> bool {
        self.with_tenant(tenant, |t| t.check_marker(marker))
            .unwrap_or(false)
    }

    /// Begins a totals reconcile. `marker` is the tenant's storage write
    /// marker read right *before* the count (`None` when the backend has
    /// none); it is checked against the reconciled marker like
    /// [`note_marker`](Self::note_marker). Read storage's per-type live counts
    /// *after* this returns, then the marker again, and pass both to
    /// [`finish_reconcile`](Self::finish_reconcile).
    pub fn begin_reconcile(&self, tenant: &str, marker: Option<StorageMarker>) -> ReconcileToken {
        self.with_tenant_or_create(tenant, |t| {
            if let Some(marker) = marker {
                t.check_marker(marker);
            }
            ReconcileToken {
                tenant: tenant.to_string(),
                generation: t.generation,
                seq: t.write_seq,
                begin: t.next_begin(),
                marker,
                live_at_begin: t
                    .types
                    .iter()
                    .map(|(name, tc)| (name.clone(), tc.live))
                    .collect(),
            }
        })
    }

    /// Completes a totals reconcile.
    ///
    /// `totals` are storage's per-type live counts read after
    /// [`begin_reconcile`](Self::begin_reconcile), and `marker` the storage
    /// write marker read right after them. For each type the base becomes the
    /// storage figure (types absent from `totals` → 0) and only the live delta
    /// recorded since begin is kept. Stamps `reconciled_at = at` and stores
    /// `marker` as the reconciled marker. If it differs from the token's
    /// before-marker, storage changed while it was counted: `external_change`
    /// stays set so the tenant remains approximate and is reconciled again;
    /// otherwise the flag clears. A successful finish also clears
    /// [`needs_reseed`](Self::needs_reseed).
    ///
    /// Returns [`ReconcileOutcome::Invalidated`] (and changes nothing, never
    /// recreating the tenant) if the tenant was invalidated, evicted or
    /// removed since begin, and [`ReconcileOutcome::Superseded`] (changing
    /// nothing) if a reconcile begun later already finished.
    pub fn finish_reconcile(
        &self,
        token: ReconcileToken,
        totals: &[(String, u64)],
        marker: Option<StorageMarker>,
        at: DateTime<Utc>,
    ) -> ReconcileOutcome {
        self.with_tenant(&token.tenant, |t| {
            if t.generation != token.generation {
                return ReconcileOutcome::Invalidated;
            }
            if t.reconciled.is_some_and(|r| r.begin > token.begin) {
                return ReconcileOutcome::Superseded;
            }
            let fresh_base = t.reconciled.is_none() || t.needs_reseed;
            // No write at all since the previous reconcile began, nor during
            // this count: any correction is drift no local write explains (a
            // write during the count is legitimately counted twice).
            let quiet =
                t.reconciled.is_some_and(|r| token.seq <= r.seq) && t.write_seq <= token.seq;
            let before: HashMap<String, u64> = t
                .types
                .iter()
                .map(|(name, tc)| (name.clone(), tc.total()))
                .collect();

            let mut storage: HashMap<&str, i64> = HashMap::with_capacity(totals.len());
            for (name, count) in totals {
                let slot = storage.entry(name.as_str()).or_insert(0);
                *slot = slot.saturating_add(to_i64(*count));
            }
            for name in storage.keys() {
                t.type_mut(name);
            }
            let mut correction = 0u64;
            for (name, tc) in &mut t.types {
                tc.base = storage.get(name.as_str()).copied().unwrap_or(0);
                tc.live_at_base = token.live_at_begin.get(name).copied().unwrap_or(0);
                let was = before.get(name).copied().unwrap_or(0);
                correction = correction.saturating_add(tc.total().abs_diff(was));
            }
            if fresh_base {
                correction = 0;
            }

            t.reconciled = Some(Reconciled {
                at,
                seq: token.seq,
                begin: token.begin,
            });
            t.needs_reseed = false;
            t.reconciled_marker = marker;
            if token.marker != marker {
                t.flag_external_change();
            } else {
                t.external_change = false;
                if correction > 0 && quiet {
                    // Marker-less backends: drift nothing local explains
                    // means storage changed, so rings loaded before are stale.
                    t.external_epoch += 1;
                }
            }
            ReconcileOutcome::Applied { correction }
        })
        .unwrap_or(ReconcileOutcome::Invalidated)
    }

    /// Begins seeding one `(type, window)` ring from storage history. Read the
    /// history buckets *after* this returns, then pass them to
    /// [`finish_ring_seed`](Self::finish_ring_seed).
    pub fn begin_ring_seed(
        &self,
        tenant: &str,
        resource_type: &str,
        window: DashboardWindow,
    ) -> RingSeedToken {
        self.with_tenant_or_create(tenant, |t| RingSeedToken {
            tenant: tenant.to_string(),
            generation: t.generation,
            resource_type: resource_type.to_string(),
            window,
            seq: t.write_seq,
            begin: t.next_begin(),
            external_epoch: t.external_epoch,
            live_at_begin: t
                .types
                .get(resource_type)
                .and_then(|tc| tc.rings[window_index(window)].live_snapshot()),
        })
    }

    /// Completes a ring seed.
    ///
    /// `deltas` are storage's `(bucket_start, delta)` history buckets for the
    /// token's window, read after [`begin_ring_seed`](Self::begin_ring_seed).
    /// The ring's seeded layer is replaced by them (buckets outside the window
    /// as of `at` are dropped), its live layer keeps only what was recorded
    /// since begin, bucket by bucket, and `at` becomes its
    /// [`history_seeded_at`](CountersSeries::history_seeded_at).
    ///
    /// Returns `false` (and changes nothing, never recreating the tenant) if
    /// the tenant was invalidated, evicted or removed since begin. A token
    /// begun before a seed of the same ring that already finished is
    /// superseded by it: nothing changes and `true` is returned.
    pub fn finish_ring_seed(
        &self,
        token: RingSeedToken,
        deltas: &[(DateTime<Utc>, i64)],
        at: DateTime<Utc>,
    ) -> bool {
        self.with_tenant(&token.tenant, |t| {
            if t.generation != token.generation {
                return false;
            }
            let ring = &mut t.type_mut(&token.resource_type).rings[window_index(token.window)];
            if ring.seed_begin.is_some_and(|begin| begin > token.begin) {
                return true;
            }
            ring.advance(bucket_floor(at.timestamp(), ring.bucket));

            if let Some((snap_newest, snap)) = &token.live_at_begin {
                let points = snap.len() as i64;
                for (i, value) in snap.iter().enumerate() {
                    let start = snap_newest - (points - 1 - i as i64) * ring.bucket;
                    if let Some(slot) = ring.slot(start) {
                        ring.live[slot] = ring.live[slot].saturating_sub(*value);
                    }
                }
            }

            ring.seeded.fill(0);
            for (bucket_start, delta) in deltas {
                let start = bucket_floor(bucket_start.timestamp(), ring.bucket);
                if let Some(slot) = ring.slot(start) {
                    ring.seeded[slot] = ring.seeded[slot].saturating_add(*delta);
                }
            }
            ring.seed_seq = Some(token.seq);
            ring.seed_begin = Some(token.begin);
            ring.seeded_at = Some(at);
            ring.seed_epoch = token.external_epoch;
            true
        })
        .unwrap_or(false)
    }

    /// Which of `candidates` (the `(type, window)` rings the dashboard is
    /// showing) the reconcile loop should re-seed now.
    ///
    /// A candidate is returned when its ring is not
    /// [`exact`](CountersSeries::exact) and either it was never seeded (or
    /// was invalidated since), or the tenant recorded no write since its seed
    /// began (it is inexact because of an external change, so a reload fixes
    /// it), or its seed was applied at least `max_age` before `now` — the
    /// periodic re-seed that keeps a ring under continuous writes tied to
    /// storage. "Quiet" is tenant-wide because a storage marker change cannot
    /// be attributed to one type. Candidates keep their order. An unknown
    /// tenant yields nothing: reconcile its totals first.
    pub fn rings_to_reseed(
        &self,
        tenant: &str,
        candidates: &[(String, DashboardWindow)],
        max_age: chrono::Duration,
        now: DateTime<Utc>,
    ) -> Vec<(String, DashboardWindow)> {
        self.with_tenant(tenant, |t| {
            candidates
                .iter()
                .filter(|(name, window)| {
                    let Some(tc) = t.types.get(name) else {
                        return true;
                    };
                    let ring = &tc.rings[window_index(*window)];
                    if t.ring_exact(tc, ring) {
                        return false;
                    }
                    let Some(seed_seq) = ring.seed_seq else {
                        return true;
                    };
                    t.write_seq <= seed_seq
                        || ring
                            .seeded_at
                            .is_none_or(|seeded| now.signed_duration_since(seeded) >= max_age)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
    }

    /// Stamps a dashboard view of `tenant` at `now`, creating its (empty)
    /// state if needed so it is prioritised and kept from eviction. A view
    /// older than one already noted is ignored.
    pub fn note_viewed(&self, tenant: &str, now: Instant) {
        self.with_tenant_or_create(tenant, |t| {
            t.last_viewed = Some(t.last_viewed.map_or(now, |seen| seen.max(now)));
        });
    }

    /// Every tenant held in memory, in the order a bounded reconcile pass
    /// should serve them: viewed tenants first, most recently viewed first;
    /// then tenants needing a reseed or with an external change flagged; then
    /// the rest. Within the last two groups, tenants reconciled longest ago
    /// come first and never-reconciled tenants last. Ties break by name.
    pub fn tenants_by_priority(&self) -> Vec<String> {
        let map = self.read_map();
        let mut entries: Vec<_> = map
            .iter()
            .map(|(name, entry)| {
                let t = lock(entry);
                let group = if t.last_viewed.is_some() {
                    0u8
                } else if t.needs_reseed || t.external_change {
                    1
                } else {
                    2
                };
                (
                    group,
                    t.last_viewed,
                    t.reconciled.map(|r| r.at),
                    name.clone(),
                )
            })
            .collect();
        drop(map);
        entries.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| match (a.2, b.2) {
                    (Some(x), Some(y)) => x.cmp(&y),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                })
                .then_with(|| a.3.cmp(&b.3))
        });
        entries.into_iter().map(|e| e.3).collect()
    }

    /// Drops every tenant not viewed for more than `max_idle` as of `now` (a
    /// never-viewed tenant counts from when its state was created), except
    /// those in `keep`. Returns the evicted tenants, sorted. Outstanding
    /// tokens for them are invalidated; their next write, view or reconcile
    /// starts a fresh, unseeded incarnation.
    pub fn evict_idle(&self, max_idle: Duration, now: Instant, keep: &[&str]) -> Vec<String> {
        let mut map = self.write_map();
        let mut evicted: Vec<String> = map
            .iter()
            .filter(|(name, entry)| {
                !keep.contains(&name.as_str()) && lock(entry).idle(now) > max_idle
            })
            .map(|(name, _)| name.clone())
            .collect();
        for name in &evicted {
            map.remove(name);
        }
        evicted.sort();
        evicted
    }

    /// Drops all of a tenant's state (the tenant was deregistered). Returns
    /// whether it had any. Outstanding tokens for it are invalidated, and
    /// finishing them does not recreate it.
    pub fn remove_tenant(&self, tenant: &str) -> bool {
        self.write_map().remove(tenant).is_some()
    }

    /// The tenant's per-type totals, or `None` when the tenant has never been
    /// reconciled from storage (see the #956 note in the module docs).
    pub fn totals_view(&self, tenant: &str) -> Option<TotalsView> {
        self.with_tenant(tenant, |t| {
            let reconciled = t.reconciled?;
            let mut totals: Vec<(String, u64)> = t
                .types
                .iter()
                .map(|(name, tc)| (name.clone(), tc.total()))
                .collect();
            totals.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            Some(TotalsView {
                totals,
                reconciled_at: reconciled.at,
                exact: !t.needs_reseed && !t.external_change && t.write_seq <= reconciled.seq,
                needs_reseed: t.needs_reseed,
                external_change: t.external_change,
            })
        })
        .flatten()
    }

    /// Totals and bucketed deltas over `window` for each requested type, in
    /// request order, as of `now`. An unknown type yields total 0 and zero
    /// buckets. `None` when the tenant has never been reconciled.
    pub fn series_view(
        &self,
        tenant: &str,
        window: DashboardWindow,
        types: &[&str],
        now: DateTime<Utc>,
    ) -> Option<Vec<CountersSeries>> {
        let bucket = window.bucket_seconds();
        let points = window.points() as i64;
        let newest = bucket_floor(now.timestamp(), bucket);
        let starts: Vec<i64> = (0..points)
            .map(|i| newest - (points - 1 - i) * bucket)
            .collect();
        let stamp = |secs: i64| DateTime::<Utc>::from_timestamp(secs, 0).unwrap_or_default();

        self.with_tenant(tenant, |t| {
            t.reconciled?;
            Some(
                types
                    .iter()
                    .map(|name| match t.types.get(*name) {
                        Some(tc) => {
                            let ring = &tc.rings[window_index(window)];
                            CountersSeries {
                                resource_type: (*name).to_string(),
                                total: tc.total(),
                                buckets: starts
                                    .iter()
                                    .map(|s| (stamp(*s), ring.displayed_at(*s)))
                                    .collect(),
                                history_seeded: ring.seed_seq.is_some(),
                                history_seeded_at: ring.seeded_at,
                                exact: t.ring_exact(tc, ring),
                            }
                        }
                        None => CountersSeries {
                            resource_type: (*name).to_string(),
                            total: 0,
                            buckets: starts.iter().map(|s| (stamp(*s), 0)).collect(),
                            history_seeded: false,
                            history_seeded_at: None,
                            exact: false,
                        },
                    })
                    .collect(),
            )
        })
        .flatten()
    }

    /// Cumulative sum of every delta recorded for `(tenant, type)`,
    /// independent of reconciles and invalidations. Test helper.
    #[doc(hidden)]
    pub fn live_delta(&self, tenant: &str, resource_type: &str) -> i64 {
        self.with_tenant(tenant, |t| {
            t.types.get(resource_type).map(|tc| tc.live).unwrap_or(0)
        })
        .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const HOUR: DashboardWindow = DashboardWindow::LastHour;
    const DAY: DashboardWindow = DashboardWindow::LastDay;
    const MONTH: DashboardWindow = DashboardWindow::LastMonth;

    /// A fixed, bucket-aligned instant for every window (midnight UTC).
    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap()
    }

    fn secs(n: i64) -> chrono::Duration {
        chrono::Duration::seconds(n)
    }

    /// Reconciles `tenant` with `totals` (so views are available).
    fn seed(c: &DashboardCounters, tenant: &str, totals: &[(&str, u64)], at: DateTime<Utc>) {
        let token = c.begin_reconcile(tenant, None);
        let totals: Vec<(String, u64)> = totals.iter().map(|(n, v)| (n.to_string(), *v)).collect();
        assert!(matches!(
            c.finish_reconcile(token, &totals, None, at),
            ReconcileOutcome::Applied { .. }
        ));
    }

    fn applied(outcome: ReconcileOutcome) -> bool {
        matches!(outcome, ReconcileOutcome::Applied { .. })
    }

    fn total_of(view: &TotalsView, name: &str) -> Option<u64> {
        view.totals.iter().find(|(n, _)| n == name).map(|(_, v)| *v)
    }

    fn series(
        c: &DashboardCounters,
        tenant: &str,
        window: DashboardWindow,
        name: &str,
        now: DateTime<Utc>,
    ) -> CountersSeries {
        c.series_view(tenant, window, &[name], now)
            .expect("seeded")
            .remove(0)
    }

    fn values(s: &CountersSeries) -> Vec<i64> {
        s.buckets.iter().map(|(_, v)| *v).collect()
    }

    #[test]
    fn bucket_floor_is_epoch_aligned_for_negative_and_positive() {
        assert_eq!(bucket_floor(119, 60), 60);
        assert_eq!(bucket_floor(120, 60), 120);
        assert_eq!(bucket_floor(-1, 60), -60);
        assert_eq!(bucket_floor(-60, 60), -60);
    }

    #[test]
    fn unseeded_tenant_returns_none_never_zeros() {
        let c = DashboardCounters::new();
        assert!(c.totals_view("t").is_none());
        assert!(c.series_view("t", HOUR, &["Patient"], t0()).is_none());

        c.record("t", "Patient", 3, t0());
        assert!(!c.is_seeded("t"));
        assert!(c.totals_view("t").is_none());
        assert!(c.series_view("t", HOUR, &["Patient"], t0()).is_none());
        assert_eq!(c.live_delta("t", "Patient"), 3);
        assert_eq!(c.tenants(), vec!["t".to_string()]);
    }

    #[test]
    fn zero_delta_is_a_no_op() {
        let c = DashboardCounters::new();
        c.record("t", "Patient", 0, t0());
        assert!(c.tenants().is_empty());
        seed(&c, "t", &[], t0());
        assert!(c.totals_view("t").unwrap().exact);
        c.record("t", "Patient", 0, t0());
        let view = c.totals_view("t").unwrap();
        assert!(view.exact, "a zero delta is not a write");
        assert!(view.totals.is_empty());
    }

    #[test]
    fn tenants_lists_only_tenants_with_state() {
        let c = DashboardCounters::new();
        let _token = c.begin_reconcile("pending", None);
        c.record("b", "Patient", 1, t0());
        seed(&c, "a", &[], t0());
        assert_eq!(c.tenants(), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn hour_ring_rotates_bucket_by_bucket() {
        let c = DashboardCounters::new();
        seed(&c, "t", &[], t0());
        c.record("t", "Patient", 1, t0());
        c.record("t", "Patient", 2, t0() + secs(59));
        c.record("t", "Patient", 4, t0() + secs(60));
        c.record("t", "Patient", 8, t0() + secs(185));

        let now = t0() + secs(185);
        let s = series(&c, "t", HOUR, "Patient", now);
        assert_eq!(s.buckets.len(), 60);
        assert_eq!(
            s.buckets.last().unwrap().0,
            t0() + secs(180),
            "last bucket contains now"
        );
        assert_eq!(s.buckets[0].0, t0() + secs(180) - secs(59 * 60));
        let v = values(&s);
        assert_eq!(&v[56..], &[3, 4, 0, 8]);
        assert_eq!(v[..56].iter().sum::<i64>(), 0);
        assert_eq!(s.total, 15);
        // Buckets are strictly increasing, one bucket apart.
        for pair in s.buckets.windows(2) {
            assert_eq!(pair[1].0 - pair[0].0, secs(60));
        }
    }

    #[test]
    fn reading_later_virtually_advances_without_losing_data() {
        let c = DashboardCounters::new();
        seed(&c, "t", &[], t0());
        c.record("t", "Patient", 5, t0());
        // 30 minutes later the write sits 30 buckets back.
        let s = series(&c, "t", HOUR, "Patient", t0() + secs(30 * 60));
        let v = values(&s);
        assert_eq!(v[59 - 30], 5);
        assert_eq!(v.iter().sum::<i64>(), 5);
        // An hour later it has rotated out of the 1h window…
        let s = series(&c, "t", HOUR, "Patient", t0() + secs(60 * 60));
        assert_eq!(values(&s).iter().sum::<i64>(), 0);
        // …but is still in the 24h and 30d windows, and in the total.
        let s = series(&c, "t", DAY, "Patient", t0() + secs(60 * 60));
        assert_eq!(values(&s)[47 - 2], 5);
        assert_eq!(s.total, 5);
        // Reading never mutated the ring: an earlier read still sees it.
        let s = series(&c, "t", HOUR, "Patient", t0());
        assert_eq!(values(&s)[59], 5);
    }

    #[test]
    fn day_and_month_rings_rotate_on_their_own_bucket_widths() {
        let c = DashboardCounters::new();
        seed(&c, "t", &[], t0());
        c.record("t", "Obs", 1, t0() + secs(10));
        c.record("t", "Obs", 1, t0() + secs(1_799));
        c.record("t", "Obs", 1, t0() + secs(1_800));
        c.record("t", "Obs", 1, t0() + secs(86_400 + 5));

        let now = t0() + secs(86_400 + 5);
        let day = series(&c, "t", DAY, "Obs", now);
        assert_eq!(day.buckets.len(), 48);
        assert_eq!(day.buckets.last().unwrap().0, t0() + secs(86_400));
        let v = values(&day);
        // t0 bucket is 48 buckets before `now`'s → rotated out; t0+1800 is 47
        // back → the oldest slot.
        assert_eq!(v[0], 1);
        assert_eq!(v[47], 1);
        assert_eq!(v.iter().sum::<i64>(), 2);

        let month = series(&c, "t", MONTH, "Obs", now);
        assert_eq!(month.buckets.len(), 30);
        assert_eq!(month.buckets.last().unwrap().0, t0() + secs(86_400));
        let v = values(&month);
        assert_eq!(&v[28..], &[3, 1]);
        assert_eq!(month.total, 4);
    }

    #[test]
    fn gaps_longer_than_the_ring_clear_every_bucket() {
        for window in DashboardWindow::ALL {
            let c = DashboardCounters::new();
            seed(&c, "t", &[], t0());
            let bucket = window.bucket_seconds();
            let points = window.points() as i64;
            c.record("t", "Patient", 7, t0());
            c.record("t", "Patient", 1, t0() + secs(bucket));
            // Write far beyond the ring's span: everything older is gone.
            let later = t0() + secs(bucket * (points * 3 + 1));
            c.record("t", "Patient", 2, later);
            let s = series(&c, "t", window, "Patient", later);
            let v = values(&s);
            assert_eq!(v[v.len() - 1], 2, "{window:?}");
            assert_eq!(v.iter().sum::<i64>(), 2, "{window:?}");
            assert_eq!(s.total, 10, "{window:?}");

            // Exactly `points` buckets later also clears everything.
            let c = DashboardCounters::new();
            seed(&c, "t", &[], t0());
            c.record("t", "Patient", 7, t0());
            let edge = t0() + secs(bucket * points);
            c.record("t", "Patient", 1, edge);
            let v = values(&series(&c, "t", window, "Patient", edge));
            assert_eq!(v.iter().sum::<i64>(), 1, "{window:?}");
        }
    }

    #[test]
    fn writes_older_than_the_oldest_bucket_are_ignored_by_the_ring_only() {
        let c = DashboardCounters::new();
        seed(&c, "t", &[], t0());
        let now = t0() + secs(2 * 60 * 60);
        c.record("t", "Patient", 1, now);
        // 61 minutes before `now`: outside the 1h ring, inside 24h.
        c.record("t", "Patient", 5, now - secs(61 * 60));
        // Inside the 1h ring's oldest bucket.
        c.record("t", "Patient", 3, now - secs(59 * 60));

        let hour = series(&c, "t", HOUR, "Patient", now);
        let v = values(&hour);
        assert_eq!(v[0], 3);
        assert_eq!(v[59], 1);
        assert_eq!(v.iter().sum::<i64>(), 4);
        assert_eq!(hour.total, 9, "old writes still count toward the total");
        let day = series(&c, "t", DAY, "Patient", now);
        assert_eq!(values(&day).iter().sum::<i64>(), 9);
    }

    #[test]
    fn writes_ahead_of_now_are_not_shown_until_now_reaches_them() {
        let c = DashboardCounters::new();
        seed(&c, "t", &[], t0());
        c.record("t", "Patient", 2, t0());
        c.record("t", "Patient", 4, t0() + secs(120));
        let s = series(&c, "t", HOUR, "Patient", t0() + secs(60));
        let v = values(&s);
        assert_eq!(&v[58..], &[2, 0]);
        let s = series(&c, "t", HOUR, "Patient", t0() + secs(120));
        assert_eq!(&values(&s)[57..], &[2, 0, 4]);
    }

    #[test]
    fn negative_deltas_and_clamping() {
        let c = DashboardCounters::new();
        seed(&c, "t", &[("Patient", 2)], t0());
        c.record("t", "Patient", -5, t0());
        let view = c.totals_view("t").unwrap();
        assert_eq!(total_of(&view, "Patient"), Some(0), "clamped at zero");
        let s = series(&c, "t", HOUR, "Patient", t0());
        assert_eq!(s.total, 0);
        assert_eq!(values(&s)[59], -5, "bucket deltas are not clamped");
        c.record("t", "Patient", 4, t0());
        // The clamp is on read only: the underlying sum is 2 - 5 + 4 = 1.
        assert_eq!(total_of(&c.totals_view("t").unwrap(), "Patient"), Some(1));
        assert_eq!(c.live_delta("t", "Patient"), -1);

        // Saturation on huge counts.
        assert_eq!(to_i64(u64::MAX), i64::MAX);
        assert_eq!(clamp_count(i64::MIN), 0);
    }

    #[test]
    fn totals_are_sorted_largest_first_then_by_name() {
        let c = DashboardCounters::new();
        seed(
            &c,
            "t",
            &[("Observation", 5), ("Patient", 9), ("Encounter", 5)],
            t0(),
        );
        c.record("t", "Condition", 1, t0());
        let view = c.totals_view("t").unwrap();
        assert_eq!(
            view.totals,
            vec![
                ("Patient".to_string(), 9),
                ("Encounter".to_string(), 5),
                ("Observation".to_string(), 5),
                ("Condition".to_string(), 1),
            ]
        );
        assert_eq!(view.reconciled_at, t0());
    }

    #[test]
    fn reconcile_keeps_writes_after_begin_and_drops_those_before() {
        let c = DashboardCounters::new();
        c.record("t", "Patient", 10, t0()); // before begin: storage has these
        c.record("t", "Observation", 3, t0());
        let token = c.begin_reconcile("t", None);
        c.record("t", "Patient", 2, t0()); // after begin: kept
        c.record("t", "Encounter", 1, t0()); // new type after begin: kept
        let later = t0() + secs(30);
        assert!(applied(c.finish_reconcile(
            token,
            &[("Patient".to_string(), 10), ("Condition".to_string(), 4)],
            None,
            later,
        )));
        let view = c.totals_view("t").unwrap();
        assert_eq!(total_of(&view, "Patient"), Some(12));
        assert_eq!(total_of(&view, "Encounter"), Some(1));
        assert_eq!(total_of(&view, "Condition"), Some(4), "storage-only type");
        assert_eq!(
            total_of(&view, "Observation"),
            Some(0),
            "a type storage no longer reports rebases to 0"
        );
        assert_eq!(view.reconciled_at, later);
        assert!(!view.exact, "writes landed during the reconcile");

        // A quiet reconcile makes it exact again, and rebases correctly.
        let token = c.begin_reconcile("t", None);
        assert!(applied(c.finish_reconcile(
            token,
            &[
                ("Patient".to_string(), 12),
                ("Encounter".to_string(), 1),
                ("Condition".to_string(), 4),
            ],
            None,
            later,
        )));
        let view = c.totals_view("t").unwrap();
        assert!(view.exact);
        assert_eq!(total_of(&view, "Patient"), Some(12));
        c.record("t", "Patient", -1, later);
        let view = c.totals_view("t").unwrap();
        assert!(!view.exact);
        assert_eq!(total_of(&view, "Patient"), Some(11));
    }

    #[test]
    fn duplicate_storage_rows_for_a_type_are_summed() {
        let c = DashboardCounters::new();
        seed(&c, "t", &[("Patient", 2), ("Patient", 3)], t0());
        assert_eq!(total_of(&c.totals_view("t").unwrap(), "Patient"), Some(5));
    }

    #[test]
    fn an_older_reconcile_token_is_superseded_by_a_newer_finished_one() {
        let c = DashboardCounters::new();
        let old = c.begin_reconcile("t", None);
        c.record("t", "Patient", 1, t0());
        let new = c.begin_reconcile("t", None);
        assert!(applied(c.finish_reconcile(
            new,
            &[("Patient".to_string(), 1)],
            None,
            t0() + secs(2)
        )));
        assert_eq!(
            c.finish_reconcile(old, &[("Patient".to_string(), 0)], None, t0() + secs(1)),
            ReconcileOutcome::Superseded
        );
        let view = c.totals_view("t").unwrap();
        assert_eq!(total_of(&view, "Patient"), Some(1));
        assert_eq!(view.reconciled_at, t0() + secs(2));
        assert!(view.exact);
    }

    #[test]
    fn stale_tokens_after_invalidate_are_rejected() {
        let c = DashboardCounters::new();
        c.record("t", "Patient", 3, t0());
        let reconcile = c.begin_reconcile("t", None);
        let ring = c.begin_ring_seed("t", "Patient", HOUR);
        c.invalidate_tenant("t");
        assert_eq!(
            c.finish_reconcile(reconcile, &[("Patient".to_string(), 3)], None, t0()),
            ReconcileOutcome::Invalidated
        );
        assert!(!c.finish_ring_seed(ring, &[(t0(), 3)], t0()));
        assert!(!c.is_seeded("t"));
        assert!(c.needs_reseed("t"));
        assert_eq!(c.live_delta("t", "Patient"), 3, "recorded writes are kept");

        // A token begun after the invalidation is accepted and clears the flag.
        let reconcile = c.begin_reconcile("t", None);
        assert!(applied(c.finish_reconcile(
            reconcile,
            &[("Patient".to_string(), 3)],
            None,
            t0()
        )));
        assert!(!c.needs_reseed("t"));
        let view = c.totals_view("t").unwrap();
        assert!(view.exact && !view.needs_reseed);
        assert_eq!(total_of(&view, "Patient"), Some(3));

        // Invalidating an unknown tenant creates no state.
        c.invalidate_tenant("nobody");
        assert!(!c.needs_reseed("nobody"));
        assert_eq!(c.tenants(), vec!["t".to_string()]);
    }

    #[test]
    fn invalidate_keeps_the_last_figures_approximate_until_reseeded() {
        let c = DashboardCounters::new();
        let now = t0() + secs(10 * 60);
        seed(&c, "t", &[("Patient", 3)], now);
        let token = c.begin_ring_seed("t", "Patient", HOUR);
        assert!(c.finish_ring_seed(token, &[(now, 3)], now));
        seed(&c, "other", &[("Patient", 1)], now);

        let in_flight = c.begin_reconcile("t", None);
        let in_flight_ring = c.begin_ring_seed("t", "Patient", HOUR);
        c.invalidate_tenant("t");

        // The last figures stay on show, labelled not exact.
        assert!(c.is_seeded("t"));
        assert!(c.needs_reseed("t"));
        let view = c.totals_view("t").expect("figures are kept");
        assert_eq!(total_of(&view, "Patient"), Some(3));
        assert!(!view.exact && view.needs_reseed);
        let s = series(&c, "t", HOUR, "Patient", now);
        assert_eq!(s.total, 3);
        assert_eq!(values(&s)[59], 3, "the ring's history is kept too");
        assert!(!s.history_seeded && !s.exact, "the ring needs a reseed");

        // Reads begun before the purge may describe erased data: rejected,
        // and the kept figures are untouched.
        assert_eq!(
            c.finish_reconcile(in_flight, &[], None, now),
            ReconcileOutcome::Invalidated
        );
        assert!(!c.finish_ring_seed(in_flight_ring, &[], now));
        assert_eq!(total_of(&c.totals_view("t").unwrap(), "Patient"), Some(3));
        assert!(c.needs_reseed("t"));
        assert_eq!(values(&series(&c, "t", HOUR, "Patient", now))[59], 3);

        // Other tenants are untouched.
        assert!(!c.needs_reseed("other"));
        assert!(c.totals_view("other").unwrap().exact);

        // Writes keep moving the stale figures.
        c.record("t", "Patient", 1, now);
        assert_eq!(total_of(&c.totals_view("t").unwrap(), "Patient"), Some(4));

        // The reseed replaces the base with storage's figures…
        let token = c.begin_reconcile("t", None);
        assert_eq!(
            c.finish_reconcile(token, &[("Patient".to_string(), 1)], None, now),
            ReconcileOutcome::Applied { correction: 0 },
            "a reseed after an invalidation is not a correction"
        );
        let view = c.totals_view("t").unwrap();
        assert!(view.exact && !view.needs_reseed);
        assert_eq!(total_of(&view, "Patient"), Some(1));
        let s = series(&c, "t", HOUR, "Patient", now);
        assert!(!s.exact, "totals alone do not vouch for the ring");

        // …and a ring seed replaces the ring's history.
        let token = c.begin_ring_seed("t", "Patient", HOUR);
        assert!(c.finish_ring_seed(token, &[(now, 1)], now));
        let s = series(&c, "t", HOUR, "Patient", now);
        assert!(s.history_seeded && s.exact);
        assert_eq!(values(&s).iter().sum::<i64>(), 1);
    }

    #[test]
    fn ring_seed_layers_storage_history_under_live_writes() {
        let c = DashboardCounters::new();
        let now = t0() + secs(10 * 60);
        // Writes storage will report (recorded before the seed began).
        c.record("t", "Patient", 2, now - secs(120));
        c.record("t", "Patient", 1, now);
        seed(&c, "t", &[("Patient", 50)], now);

        let token = c.begin_ring_seed("t", "Patient", HOUR);
        // A write during the seed: kept in the live layer.
        c.record("t", "Patient", 4, now);
        let deltas = vec![
            (now - secs(120), 2),
            (now - secs(300) + secs(7), 6), // unaligned start is floored
            (now, 1),
            (now - secs(60 * 60), 99), // outside the 1h window as of `now`
            (now + secs(60), 42),      // beyond the newest bucket
        ];
        assert!(c.finish_ring_seed(token, &deltas, now));

        let s = series(&c, "t", HOUR, "Patient", now);
        assert!(s.history_seeded);
        assert!(!s.exact, "a write landed during the seed");
        let v = values(&s);
        assert_eq!(v[59], 1 + 4);
        assert_eq!(v[57], 2);
        assert_eq!(v[54], 6);
        assert_eq!(v.iter().sum::<i64>(), 13);

        // Other windows are not seeded by the hour seed.
        let day = series(&c, "t", DAY, "Patient", now);
        assert!(!day.history_seeded);
        assert!(!day.exact);
        assert_eq!(values(&day).iter().sum::<i64>(), 7, "live layer only");

        // Re-seeding replaces the seeded layer and rebases live again.
        let token = c.begin_ring_seed("t", "Patient", HOUR);
        assert!(c.finish_ring_seed(token, &[(now, 5), (now - secs(120), 2)], now));
        let s = series(&c, "t", HOUR, "Patient", now);
        assert!(s.exact);
        let v = values(&s);
        assert_eq!(v[59], 5);
        assert_eq!(v[57], 2);
        assert_eq!(v.iter().sum::<i64>(), 7);

        // The seeded layer rotates together with the live layer.
        let later = now + secs(60);
        c.record("t", "Patient", 1, later);
        let s = series(&c, "t", HOUR, "Patient", later);
        assert!(!s.exact);
        let v = values(&s);
        assert_eq!(&v[56..], &[2, 0, 5, 1]);
    }

    #[test]
    fn ring_seed_subtraction_aligns_by_bucket_start_across_rotation() {
        let c = DashboardCounters::new();
        seed(&c, "t", &[], t0());
        c.record("t", "Patient", 3, t0());
        let token = c.begin_ring_seed("t", "Patient", HOUR);
        // The ring rotates two buckets between begin and finish.
        let later = t0() + secs(120);
        c.record("t", "Patient", 1, later);
        assert!(c.finish_ring_seed(token, &[(t0(), 3)], later));
        let v = values(&series(&c, "t", HOUR, "Patient", later));
        assert_eq!(&v[57..], &[3, 0, 1]);
        assert_eq!(v.iter().sum::<i64>(), 4);
    }

    #[test]
    fn ring_seed_of_an_unwritten_type_creates_it() {
        let c = DashboardCounters::new();
        seed(&c, "t", &[("Patient", 3)], t0());
        let token = c.begin_ring_seed("t", "Patient", MONTH);
        assert!(c.finish_ring_seed(token, &[(t0() - secs(86_400), 3)], t0()));
        let s = series(&c, "t", MONTH, "Patient", t0());
        assert!(s.history_seeded && s.exact);
        assert_eq!(&values(&s)[28..], &[3, 0]);
        assert_eq!(s.total, 3);
    }

    #[test]
    fn an_older_ring_seed_token_is_superseded() {
        let c = DashboardCounters::new();
        seed(&c, "t", &[], t0());
        let old = c.begin_ring_seed("t", "Patient", HOUR);
        c.record("t", "Patient", 1, t0());
        let new = c.begin_ring_seed("t", "Patient", HOUR);
        assert!(c.finish_ring_seed(new, &[(t0(), 1)], t0()));
        assert!(c.finish_ring_seed(old, &[], t0()));
        let s = series(&c, "t", HOUR, "Patient", t0());
        assert_eq!(values(&s)[59], 1);
        assert!(s.exact);
    }

    #[test]
    fn exact_flags_track_writes_since_the_seed_began() {
        let c = DashboardCounters::new();
        seed(&c, "t", &[], t0());
        assert!(c.totals_view("t").unwrap().exact);

        let token = c.begin_ring_seed("t", "Patient", HOUR);
        assert!(c.finish_ring_seed(token, &[], t0()));
        let s = series(&c, "t", HOUR, "Patient", t0());
        assert!(s.history_seeded && s.exact);

        // A write to another type makes tenant totals inexact but leaves this
        // type's series exact.
        c.record("t", "Observation", 1, t0());
        assert!(!c.totals_view("t").unwrap().exact);
        let both = c
            .series_view("t", HOUR, &["Patient", "Observation"], t0())
            .unwrap();
        assert!(both[0].exact);
        assert!(!both[1].exact && !both[1].history_seeded);

        c.record("t", "Patient", 1, t0());
        assert!(!series(&c, "t", HOUR, "Patient", t0()).exact);
    }

    #[test]
    fn series_preserve_request_order_and_fill_unknown_types() {
        let c = DashboardCounters::new();
        seed(&c, "t", &[("Patient", 2), ("Observation", 7)], t0());
        let out = c
            .series_view("t", DAY, &["Observation", "Nope", "Patient"], t0())
            .unwrap();
        let names: Vec<&str> = out.iter().map(|s| s.resource_type.as_str()).collect();
        assert_eq!(names, ["Observation", "Nope", "Patient"]);
        assert_eq!(out[0].total, 7);
        assert_eq!(out[1].total, 0);
        assert_eq!(out[1].buckets.len(), 48);
        assert!(out[1].buckets.iter().all(|(_, v)| *v == 0));
        assert!(!out[1].history_seeded && !out[1].exact);
        assert_eq!(
            out[1].buckets,
            out[0]
                .buckets
                .iter()
                .map(|(t, _)| (*t, 0))
                .collect::<Vec<_>>()
        );
        assert_eq!(out[2].total, 2);
        assert!(c.series_view("t", DAY, &[], t0()).unwrap().is_empty());
    }

    #[test]
    fn concurrent_records_do_not_lose_counts() {
        let c = Arc::new(DashboardCounters::new());
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let c = Arc::clone(&c);
                std::thread::spawn(move || {
                    let tenant = if i % 2 == 0 { "even" } else { "odd" };
                    for n in 0..5_000 {
                        c.record(tenant, "Patient", 1, t0() + secs(n % 120));
                        if n % 10 == 0 {
                            c.record(tenant, "Observation", -1, t0());
                        }
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        for tenant in ["even", "odd"] {
            assert_eq!(c.live_delta(tenant, "Patient"), 20_000);
            assert_eq!(c.live_delta(tenant, "Observation"), -2_000);
        }
        seed(&c, "even", &[], t0());
        // Rebased against an empty storage read: live deltas before begin drop.
        let view = c.totals_view("even").unwrap();
        assert_eq!(total_of(&view, "Patient"), Some(0));
        // The 1h ring saw every Patient write (all within two minutes).
        let s = series(&c, "even", HOUR, "Patient", t0() + secs(119));
        assert_eq!(values(&s).iter().sum::<i64>(), 20_000);
    }
    fn marker(n: i64) -> StorageMarker {
        StorageMarker {
            latest_millis: Some(n),
            recent_writes: Some(n as u64),
        }
    }

    /// One reconcile with explicit before/after markers.
    fn reconcile(
        c: &DashboardCounters,
        tenant: &str,
        totals: &[(&str, u64)],
        before: Option<StorageMarker>,
        after: Option<StorageMarker>,
        at: DateTime<Utc>,
    ) -> ReconcileOutcome {
        let token = c.begin_reconcile(tenant, before);
        let totals: Vec<(String, u64)> = totals.iter().map(|(n, v)| (n.to_string(), *v)).collect();
        c.finish_reconcile(token, &totals, after, at)
    }

    fn seed_ring(
        c: &DashboardCounters,
        tenant: &str,
        name: &str,
        window: DashboardWindow,
        at: DateTime<Utc>,
    ) {
        let token = c.begin_ring_seed(tenant, name, window);
        assert!(c.finish_ring_seed(token, &[], at));
    }

    fn cands(list: &[(&str, DashboardWindow)]) -> Vec<(String, DashboardWindow)> {
        list.iter().map(|(n, w)| (n.to_string(), *w)).collect()
    }

    #[test]
    fn unchanged_marker_during_the_count_keeps_totals_exact() {
        let c = DashboardCounters::new();
        let m = Some(marker(1));
        assert_eq!(
            reconcile(&c, "t", &[("Patient", 3)], m, m, t0()),
            ReconcileOutcome::Applied { correction: 0 }
        );
        let view = c.totals_view("t").unwrap();
        assert!(view.exact && !view.external_change && !view.needs_reseed);
        assert!(!c.note_marker("t", marker(1)), "same marker: nothing to do");
        assert!(c.totals_view("t").unwrap().exact);
        seed_ring(&c, "t", "Patient", HOUR, t0());
        assert!(series(&c, "t", HOUR, "Patient", t0()).exact);
    }

    #[test]
    fn marker_changed_during_the_count_applies_but_stays_approximate() {
        let c = DashboardCounters::new();
        assert_eq!(
            reconcile(
                &c,
                "t",
                &[("Patient", 5)],
                Some(marker(1)),
                Some(marker(2)),
                t0()
            ),
            ReconcileOutcome::Applied { correction: 0 }
        );
        let view = c.totals_view("t").unwrap();
        assert_eq!(total_of(&view, "Patient"), Some(5), "figures are applied");
        assert!(!view.exact && view.external_change);
        seed_ring(&c, "t", "Patient", HOUR, t0());
        let s = series(&c, "t", HOUR, "Patient", t0());
        assert!(
            s.history_seeded && !s.exact,
            "flag keeps series approximate"
        );

        // Tokens with a Some/None mismatch count as a change too.
        let c2 = DashboardCounters::new();
        reconcile(&c2, "t", &[], Some(marker(1)), None, t0());
        assert!(c2.totals_view("t").unwrap().external_change);

        // The next quiet reconcile clears it; the ring seeded after the
        // detection is exact again.
        let m = Some(marker(2));
        assert_eq!(
            reconcile(&c, "t", &[("Patient", 5)], m, m, t0() + secs(5)),
            ReconcileOutcome::Applied { correction: 0 }
        );
        let view = c.totals_view("t").unwrap();
        assert!(view.exact && !view.external_change);
        assert!(series(&c, "t", HOUR, "Patient", t0()).exact);
    }

    #[test]
    fn note_marker_flags_an_external_change_until_a_clean_reconcile() {
        let c = DashboardCounters::new();
        let now = t0();
        let day = chrono::Duration::days(1);
        let hour_patient = cands(&[("Patient", HOUR)]);
        assert!(!c.note_marker("nobody", marker(1)), "unknown tenant");
        assert!(
            c.tenants_by_priority().is_empty(),
            "the probe creates nothing"
        );

        let m1 = Some(marker(1));
        reconcile(&c, "t", &[("Patient", 3)], m1, m1, now);
        seed_ring(&c, "t", "Patient", HOUR, now);
        assert!(c.rings_to_reseed("t", &hour_patient, day, now).is_empty());

        assert!(!c.note_marker("t", marker(1)));
        assert!(c.note_marker("t", marker(2)), "no local write explains it");
        let view = c.totals_view("t").unwrap();
        assert!(!view.exact && view.external_change);
        let s = series(&c, "t", HOUR, "Patient", now);
        assert!(s.history_seeded && !s.exact);
        assert_eq!(
            c.rings_to_reseed("t", &hour_patient, day, now),
            hour_patient,
            "quiet but inexact: reloaded at once"
        );
        assert!(c.note_marker("t", marker(2)), "still unexplained");

        // A clean reconcile corrects the totals and clears the flag…
        let m2 = Some(marker(2));
        assert_eq!(
            reconcile(&c, "t", &[("Patient", 4)], m2, m2, now + secs(1)),
            ReconcileOutcome::Applied { correction: 1 }
        );
        let view = c.totals_view("t").unwrap();
        assert!(view.exact && !view.external_change);
        assert_eq!(total_of(&view, "Patient"), Some(4));
        assert!(!c.note_marker("t", marker(2)));
        // …but does not vouch for the ring loaded before the change.
        assert!(!series(&c, "t", HOUR, "Patient", now).exact);
        assert_eq!(
            c.rings_to_reseed("t", &hour_patient, day, now),
            hour_patient
        );
        seed_ring(&c, "t", "Patient", HOUR, now + secs(2));
        assert!(series(&c, "t", HOUR, "Patient", now).exact);
        assert!(c.rings_to_reseed("t", &hour_patient, day, now).is_empty());

        // A tenant reconciled without markers cannot be probed.
        reconcile(&c, "plain", &[], None, None, now);
        assert!(!c.note_marker("plain", marker(9)));
        assert!(c.totals_view("plain").unwrap().exact);
    }

    #[test]
    fn note_marker_after_local_writes_flags_without_forcing_a_reconcile() {
        let c = DashboardCounters::new();
        let m1 = Some(marker(1));
        reconcile(&c, "t", &[("Patient", 0)], m1, m1, t0());
        c.record("t", "Patient", 1, t0());
        assert!(
            !c.note_marker("t", marker(2)),
            "a local write may explain it"
        );
        let view = c.totals_view("t").unwrap();
        assert!(view.external_change && !view.exact);

        let m2 = Some(marker(2));
        assert_eq!(
            reconcile(&c, "t", &[("Patient", 1)], m2, m2, t0()),
            ReconcileOutcome::Applied { correction: 0 },
            "the recorded write matched storage"
        );
        let view = c.totals_view("t").unwrap();
        assert!(view.exact && !view.external_change);
    }

    #[test]
    fn begin_reconcile_notices_a_marker_change_without_a_probe() {
        let c = DashboardCounters::new();
        let m1 = Some(marker(1));
        reconcile(&c, "t", &[("Patient", 2)], m1, m1, t0());
        seed_ring(&c, "t", "Patient", HOUR, t0());
        seed_ring(&c, "t", "Observation", HOUR, t0());

        // A foreign update: the count does not change, the marker does.
        let m2 = Some(marker(2));
        assert_eq!(
            reconcile(&c, "t", &[("Patient", 2)], m2, m2, t0() + secs(1)),
            ReconcileOutcome::Applied { correction: 0 }
        );
        assert!(c.totals_view("t").unwrap().exact);
        let both = c
            .series_view("t", HOUR, &["Patient", "Observation"], t0())
            .unwrap();
        assert!(
            both.iter().all(|s| s.history_seeded && !s.exact),
            "rings loaded before the change need a reload"
        );
    }

    #[test]
    fn correction_measures_drift_only() {
        let c = DashboardCounters::new();
        assert_eq!(
            reconcile(
                &c,
                "t",
                &[("Patient", 10), ("Observation", 5)],
                None,
                None,
                t0()
            ),
            ReconcileOutcome::Applied { correction: 0 },
            "first seed"
        );
        // Writes this process recorded and storage holds: no correction.
        c.record("t", "Patient", 2, t0());
        assert_eq!(
            reconcile(
                &c,
                "t",
                &[("Patient", 12), ("Observation", 5)],
                None,
                None,
                t0()
            ),
            ReconcileOutcome::Applied { correction: 0 }
        );
        // Drift: +1 Patient, -2 Observation, +4 of a storage-only type.
        assert_eq!(
            reconcile(
                &c,
                "t",
                &[("Patient", 13), ("Observation", 3), ("Condition", 4)],
                None,
                None,
                t0()
            ),
            ReconcileOutcome::Applied { correction: 7 }
        );
        // A type storage no longer reports drops to 0: counted too.
        assert_eq!(
            reconcile(
                &c,
                "t",
                &[("Patient", 13), ("Observation", 3)],
                None,
                None,
                t0()
            ),
            ReconcileOutcome::Applied { correction: 4 }
        );
        // The reseed after an invalidation replaces figures, not drift.
        c.invalidate_tenant("t");
        assert_eq!(
            reconcile(&c, "t", &[("Patient", 1)], None, None, t0()),
            ReconcileOutcome::Applied { correction: 0 }
        );
        assert_eq!(
            reconcile(&c, "t", &[("Patient", 2)], None, None, t0()),
            ReconcileOutcome::Applied { correction: 1 },
            "Patient +1; the other types were already rebased to 0"
        );
    }

    #[test]
    fn markerless_unexplained_drift_marks_rings_for_reload() {
        let c = DashboardCounters::new();
        reconcile(&c, "t", &[("Patient", 3)], None, None, t0());
        seed_ring(&c, "t", "Patient", HOUR, t0());
        assert!(series(&c, "t", HOUR, "Patient", t0()).exact);
        assert_eq!(
            reconcile(&c, "t", &[("Patient", 5)], None, None, t0()),
            ReconcileOutcome::Applied { correction: 2 }
        );
        assert!(c.totals_view("t").unwrap().exact);
        assert!(!series(&c, "t", HOUR, "Patient", t0()).exact);
        let hour_patient = cands(&[("Patient", HOUR)]);
        assert_eq!(
            c.rings_to_reseed("t", &hour_patient, chrono::Duration::days(1), t0()),
            hour_patient
        );

        // A write during the count is double counted, not foreign: other
        // types' rings stay exact.
        let c = DashboardCounters::new();
        reconcile(
            &c,
            "u",
            &[("Patient", 0), ("Observation", 0)],
            None,
            None,
            t0(),
        );
        seed_ring(&c, "u", "Observation", HOUR, t0());
        let token = c.begin_reconcile("u", None);
        c.record("u", "Patient", 1, t0());
        assert_eq!(
            c.finish_reconcile(
                token,
                &[("Patient".to_string(), 1), ("Observation".to_string(), 0)],
                None,
                t0()
            ),
            ReconcileOutcome::Applied { correction: 1 }
        );
        assert!(series(&c, "u", HOUR, "Observation", t0()).exact);
    }

    #[test]
    fn an_older_token_with_the_same_write_seq_is_superseded() {
        let c = DashboardCounters::new();
        let old = c.begin_reconcile("t", None);
        let new = c.begin_reconcile("t", None);
        assert!(applied(c.finish_reconcile(
            new,
            &[("Patient".to_string(), 5)],
            None,
            t0()
        )));
        assert_eq!(
            c.finish_reconcile(old, &[("Patient".to_string(), 3)], None, t0()),
            ReconcileOutcome::Superseded
        );
        assert_eq!(total_of(&c.totals_view("t").unwrap(), "Patient"), Some(5));

        let old = c.begin_ring_seed("t", "Patient", HOUR);
        let new = c.begin_ring_seed("t", "Patient", HOUR);
        assert!(c.finish_ring_seed(new, &[(t0(), 5)], t0()));
        assert!(c.finish_ring_seed(old, &[(t0(), 3)], t0()));
        assert_eq!(values(&series(&c, "t", HOUR, "Patient", t0()))[59], 5);
    }

    #[test]
    fn rings_to_reseed_reloads_rings_under_continuous_writes_after_max_age() {
        let c = DashboardCounters::new();
        let now = t0();
        let max_age = chrono::Duration::minutes(10);
        let all = cands(&[("Patient", HOUR), ("Patient", DAY), ("Observation", HOUR)]);
        assert!(
            c.rings_to_reseed("nobody", &all, max_age, now).is_empty(),
            "unknown tenant"
        );

        reconcile(&c, "t", &[("Patient", 0)], None, None, now);
        seed_ring(&c, "t", "Patient", HOUR, now);
        let s = series(&c, "t", HOUR, "Patient", now);
        assert!(s.exact);
        assert_eq!(s.history_seeded_at, Some(now));
        assert_eq!(series(&c, "t", DAY, "Patient", now).history_seeded_at, None);
        assert_eq!(
            c.rings_to_reseed("t", &all, max_age, now),
            cands(&[("Patient", DAY), ("Observation", HOUR)]),
            "never-seeded rings, in candidate order"
        );
        seed_ring(&c, "t", "Patient", DAY, now);
        seed_ring(&c, "t", "Observation", HOUR, now);
        assert!(c.rings_to_reseed("t", &all, max_age, now).is_empty());

        // Continuous writes: inexact, but not reloaded before max_age…
        c.record("t", "Patient", 1, now + secs(60));
        assert!(
            c.rings_to_reseed("t", &all, max_age, now + secs(5 * 60))
                .is_empty()
        );
        // …and reloaded once the seed is that old.
        let aged = now + max_age;
        assert_eq!(
            c.rings_to_reseed("t", &all, max_age, aged),
            cands(&[("Patient", HOUR), ("Patient", DAY)])
        );

        // A re-seed overlapping another write stays inexact but restarts the
        // clock for that ring only.
        let token = c.begin_ring_seed("t", "Patient", HOUR);
        c.record("t", "Patient", 1, aged);
        assert!(c.finish_ring_seed(token, &[], aged));
        let s = series(&c, "t", HOUR, "Patient", aged);
        assert!(!s.exact && s.history_seeded_at == Some(aged));
        assert_eq!(
            c.rings_to_reseed("t", &all, max_age, aged + secs(5 * 60)),
            cands(&[("Patient", DAY)])
        );
        assert_eq!(
            c.rings_to_reseed("t", &all, max_age, aged + max_age),
            cands(&[("Patient", HOUR), ("Patient", DAY)])
        );

        // An external change while the tenant keeps writing: the quiet type's
        // ring is inexact, and reloaded on the periodic schedule.
        let m = Some(marker(1));
        reconcile(&c, "w", &[], m, m, now);
        seed_ring(&c, "w", "Observation", HOUR, now);
        c.record("w", "Patient", 1, now);
        assert!(!c.note_marker("w", marker(2)));
        let obs = cands(&[("Observation", HOUR)]);
        assert!(!series(&c, "w", HOUR, "Observation", now).exact);
        assert!(c.rings_to_reseed("w", &obs, max_age, now).is_empty());
        assert_eq!(c.rings_to_reseed("w", &obs, max_age, now + max_age), obs);

        // Invalidation: every ring is never-seeded again.
        c.invalidate_tenant("t");
        assert_eq!(
            series(&c, "t", HOUR, "Patient", aged).history_seeded_at,
            None
        );
        assert_eq!(c.rings_to_reseed("t", &all, max_age, aged), all);
    }

    #[test]
    fn tenants_by_priority_orders_views_then_stale_then_oldest_reconcile() {
        let c = DashboardCounters::new();
        let base = Instant::now();
        reconcile(&c, "old", &[], None, None, t0());
        reconcile(&c, "new", &[], None, None, t0() + secs(60));
        c.record("unseeded", "Patient", 1, t0());
        let _pending = c.begin_reconcile("pending", None);
        reconcile(&c, "stale", &[], None, None, t0() + secs(120));
        c.invalidate_tenant("stale");
        let m = Some(marker(1));
        reconcile(&c, "external", &[], m, m, t0() + secs(30));
        assert!(c.note_marker("external", marker(2)));
        c.note_viewed("viewed-early", base);
        reconcile(&c, "viewed-stale", &[], None, None, t0());
        c.invalidate_tenant("viewed-stale");
        c.note_viewed("viewed-stale", base + Duration::from_secs(1));
        c.note_viewed("viewed-late", base + Duration::from_secs(5));
        // An older view does not move a tenant back.
        c.note_viewed("viewed-late", base);

        assert_eq!(
            c.tenants_by_priority(),
            [
                "viewed-late",
                "viewed-stale",
                "viewed-early",
                "external",
                "stale",
                "old",
                "new",
                "pending",
                "unseeded",
            ]
        );
        assert!(
            !c.tenants().contains(&"viewed-early".to_string()),
            "a view alone is not counter state"
        );
    }

    #[test]
    fn evict_idle_keeps_recent_views_and_the_keep_list() {
        let c = DashboardCounters::new();
        let base = Instant::now();
        c.record("never", "Patient", 1, t0());
        c.record("kept", "Patient", 1, t0());
        c.note_viewed("viewed-old", base);
        c.note_viewed("viewed-recent", base + Duration::from_secs(50));
        c.note_viewed("edge", base + Duration::from_secs(30));
        reconcile(&c, "never", &[("Patient", 1)], None, None, t0());
        let outstanding = c.begin_reconcile("never", None);
        let ring = c.begin_ring_seed("never", "Patient", HOUR);

        let now = base + Duration::from_secs(60);
        let max_idle = Duration::from_secs(30);
        assert_eq!(
            c.evict_idle(max_idle, now, &["kept"]),
            ["never", "viewed-old"]
        );
        let mut left = c.tenants_by_priority();
        left.sort();
        assert_eq!(left, ["edge", "kept", "viewed-recent"]);
        assert!(c.evict_idle(max_idle, now, &["kept"]).is_empty());

        // Outstanding tokens are rejected and do not bring the tenant back.
        assert_eq!(
            c.finish_reconcile(outstanding, &[("Patient".to_string(), 1)], None, t0()),
            ReconcileOutcome::Invalidated
        );
        assert!(!c.finish_ring_seed(ring, &[(t0(), 1)], t0()));
        assert!(!c.tenants_by_priority().contains(&"never".to_string()));

        // A later write starts a fresh, unseeded incarnation.
        c.record("never", "Patient", 1, t0());
        assert!(!c.is_seeded("never"));
        assert!(c.totals_view("never").is_none());

        // Nothing is kept once idle long enough, unless listed.
        let later = now + Duration::from_secs(3_600);
        assert_eq!(
            c.evict_idle(max_idle, later, &[]),
            ["edge", "kept", "never", "viewed-recent"]
        );
    }

    #[test]
    fn remove_tenant_rejects_outstanding_tokens_and_does_not_resurrect() {
        let c = DashboardCounters::new();
        let m = Some(marker(1));
        reconcile(&c, "t", &[("Patient", 3)], m, m, t0());
        seed(&c, "other", &[("Patient", 1)], t0());
        let token = c.begin_reconcile("t", m);
        let stale = token.clone();
        let ring = c.begin_ring_seed("t", "Patient", HOUR);
        let stale_ring = ring.clone();

        assert!(c.remove_tenant("t"));
        assert!(!c.remove_tenant("t"));
        assert!(!c.remove_tenant("nobody"));
        assert_eq!(
            c.finish_reconcile(token, &[("Patient".to_string(), 3)], m, t0()),
            ReconcileOutcome::Invalidated
        );
        assert!(!c.finish_ring_seed(ring, &[(t0(), 3)], t0()));
        assert!(!c.note_marker("t", marker(2)));
        assert!(
            c.rings_to_reseed(
                "t",
                &cands(&[("Patient", HOUR)]),
                chrono::Duration::zero(),
                t0()
            )
            .is_empty()
        );
        assert_eq!(c.tenants(), ["other"]);
        assert_eq!(c.tenants_by_priority(), ["other"]);
        assert!(!c.is_seeded("t") && c.totals_view("t").is_none());

        // A new incarnation never accepts the removed one's tokens.
        c.record("t", "Patient", 1, t0());
        assert_eq!(
            c.finish_reconcile(stale, &[("Patient".to_string(), 3)], m, t0()),
            ReconcileOutcome::Invalidated
        );
        assert!(!c.finish_ring_seed(stale_ring, &[(t0(), 3)], t0()));
        assert!(!c.is_seeded("t"));
        assert_eq!(c.live_delta("t", "Patient"), 1);
        assert!(c.totals_view("other").unwrap().exact, "others untouched");
    }

    #[test]
    fn concurrent_reconciles_probes_and_removals_do_not_lose_other_tenants() {
        let c = Arc::new(DashboardCounters::new());
        let writers: Vec<_> = (0..4)
            .map(|_| {
                let c = Arc::clone(&c);
                std::thread::spawn(move || {
                    for n in 0..2_000 {
                        c.record("steady", "Patient", 1, t0() + secs(n % 60));
                        c.record("churn", "Patient", 1, t0());
                    }
                })
            })
            .collect();
        let maintenance = {
            let c = Arc::clone(&c);
            std::thread::spawn(move || {
                for n in 0..500u64 {
                    let m = Some(marker(n as i64));
                    let token = c.begin_reconcile("churn", m);
                    let _ = c.note_marker("churn", marker(n as i64 + 1));
                    let _ = c.finish_reconcile(token, &[], m, t0());
                    c.note_viewed("steady", Instant::now());
                    let _ = c.tenants_by_priority();
                    if n % 50 == 0 {
                        c.remove_tenant("churn");
                    }
                    let _ = c.evict_idle(Duration::from_secs(3_600), Instant::now(), &[]);
                }
            })
        };
        for t in writers {
            t.join().unwrap();
        }
        maintenance.join().unwrap();
        assert_eq!(c.live_delta("steady", "Patient"), 8_000);
        assert!(c.live_delta("churn", "Patient") <= 8_000);
        seed(&c, "steady", &[("Patient", 8_000)], t0());
        assert!(c.totals_view("steady").unwrap().exact);
    }
}
