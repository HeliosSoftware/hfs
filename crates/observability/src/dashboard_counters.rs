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
//! The write paths (REST create/update/delete, Bundles, `$bulk-submit`) now
//! call [`record`](DashboardCounters::record) with the net live-count change of
//! each committed write, and the dashboard reads the result in O(1). Storage is
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
//! was recorded for the tenant since the last successful reconcile began, and
//! [`CountersSeries::exact`] likewise per type since its ring seed began. The
//! history rings use the same scheme bucket-wise.
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
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, RwLock};

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
    /// `None` until the ring has been seeded from storage.
    seed_seq: Option<u64>,
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
}

/// All counter state of one tenant.
#[derive(Debug)]
struct TenantCounters {
    /// Identity of this incarnation of the tenant's state; a token whose
    /// generation differs was begun before an invalidation.
    generation: u64,
    /// Incremented for every recorded (non-zero) write.
    write_seq: u64,
    reconciled: Option<Reconciled>,
    types: HashMap<String, TypeCounters>,
}

impl TenantCounters {
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
    /// reconcile began, so the figures equal what storage reported.
    pub exact: bool,
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
    /// `history_seeded` and no write was recorded for this type since that
    /// seed began.
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
    /// guard is held for the duration so an invalidation cannot remove the
    /// entry mid-update (the write lands before or after it, never into a
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
            Arc::new(Mutex::new(TenantCounters {
                generation: self.next_generation.fetch_add(1, Ordering::Relaxed),
                write_seq: 0,
                reconciled: None,
                types: HashMap::new(),
            }))
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

    /// Forgets everything for a tenant (purge / tenant data wipe). Tokens
    /// begun before this are rejected by the `finish_*` calls.
    pub fn invalidate_tenant(&self, tenant: &str) {
        self.write_map().remove(tenant);
    }

    /// Totals have been reconciled from storage at least once since the last
    /// invalidation.
    pub fn is_seeded(&self, tenant: &str) -> bool {
        self.with_tenant(tenant, |t| t.reconciled.is_some())
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

    /// Begins a totals reconcile. Read storage's per-type live counts *after*
    /// this returns, then pass them to
    /// [`finish_reconcile`](Self::finish_reconcile).
    pub fn begin_reconcile(&self, tenant: &str) -> ReconcileToken {
        self.with_tenant_or_create(tenant, |t| ReconcileToken {
            tenant: tenant.to_string(),
            generation: t.generation,
            seq: t.write_seq,
            live_at_begin: t
                .types
                .iter()
                .map(|(name, tc)| (name.clone(), tc.live))
                .collect(),
        })
    }

    /// Completes a totals reconcile.
    ///
    /// `totals` are storage's per-type live counts read after
    /// [`begin_reconcile`](Self::begin_reconcile). For each type the base
    /// becomes the storage figure (types absent from `totals` → 0) and only the
    /// live delta recorded since begin is kept. Stamps `reconciled_at = at`.
    ///
    /// Returns `false` (and changes nothing) if the tenant was invalidated
    /// since begin. A token older than a reconcile that already finished is
    /// superseded by it: nothing changes and `true` is returned, since the
    /// tenant is seeded with figures at least as fresh.
    pub fn finish_reconcile(
        &self,
        token: ReconcileToken,
        totals: &[(String, u64)],
        at: DateTime<Utc>,
    ) -> bool {
        self.with_tenant(&token.tenant, |t| {
            if t.generation != token.generation {
                return false;
            }
            if t.reconciled.is_some_and(|r| r.seq > token.seq) {
                return true;
            }
            let mut storage: HashMap<&str, i64> = HashMap::with_capacity(totals.len());
            for (name, count) in totals {
                let slot = storage.entry(name.as_str()).or_insert(0);
                *slot = slot.saturating_add(to_i64(*count));
            }
            for name in storage.keys() {
                t.type_mut(name);
            }
            for (name, tc) in &mut t.types {
                tc.base = storage.get(name.as_str()).copied().unwrap_or(0);
                tc.live_at_base = token.live_at_begin.get(name).copied().unwrap_or(0);
            }
            t.reconciled = Some(Reconciled { at, seq: token.seq });
            true
        })
        .unwrap_or(false)
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
    /// as of `at` are dropped) and its live layer keeps only what was recorded
    /// since begin, bucket by bucket.
    ///
    /// Returns `false` (and changes nothing) if the tenant was invalidated
    /// since begin. A token older than a seed of the same ring that already
    /// finished is superseded by it: nothing changes and `true` is returned.
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
            if ring.seed_seq.is_some_and(|seq| seq > token.seq) {
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
            true
        })
        .unwrap_or(false)
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
                exact: t.write_seq <= reconciled.seq,
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
                            let history_seeded = ring.seed_seq.is_some();
                            CountersSeries {
                                resource_type: (*name).to_string(),
                                total: tc.total(),
                                buckets: starts
                                    .iter()
                                    .map(|s| (stamp(*s), ring.displayed_at(*s)))
                                    .collect(),
                                history_seeded,
                                exact: ring.seed_seq.is_some_and(|seq| tc.last_write_seq <= seq),
                            }
                        }
                        None => CountersSeries {
                            resource_type: (*name).to_string(),
                            total: 0,
                            buckets: starts.iter().map(|s| (stamp(*s), 0)).collect(),
                            history_seeded: false,
                            exact: false,
                        },
                    })
                    .collect(),
            )
        })
        .flatten()
    }

    /// Cumulative sum of every delta recorded for `(tenant, type)` since the
    /// tenant's last invalidation, independent of reconciles. Test helper.
    #[doc(hidden)]
    pub fn live_delta(&self, tenant: &str, resource_type: &str) -> i64 {
        self.with_tenant(tenant, |t| {
            t.types.get(resource_type).map(|tc| tc.live).unwrap_or(0)
        })
        .unwrap_or(0)
    }
}

static GLOBAL: LazyLock<DashboardCounters> = LazyLock::new(DashboardCounters::new);

/// The process-global counters the server's write paths record into and the
/// dashboard provider reads.
pub fn global() -> &'static DashboardCounters {
    &GLOBAL
}

/// Records `n` committed creates of `resource_type` for `tenant` now.
pub fn record_created(tenant: &str, resource_type: &str, n: u64) {
    global().record(tenant, resource_type, to_i64(n), Utc::now());
}

/// Records `n` committed deletes of `resource_type` for `tenant` now.
pub fn record_deleted(tenant: &str, resource_type: &str, n: u64) {
    global().record(tenant, resource_type, -to_i64(n), Utc::now());
}

/// Forgets the global counter state of `tenant` (purge / data wipe).
pub fn invalidate_tenant(tenant: &str) {
    global().invalidate_tenant(tenant);
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
        let token = c.begin_reconcile(tenant);
        let totals: Vec<(String, u64)> = totals.iter().map(|(n, v)| (n.to_string(), *v)).collect();
        assert!(c.finish_reconcile(token, &totals, at));
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
        let _token = c.begin_reconcile("pending");
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

        // `record_created`/`record_deleted`-style saturation on huge counts.
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
        let token = c.begin_reconcile("t");
        c.record("t", "Patient", 2, t0()); // after begin: kept
        c.record("t", "Encounter", 1, t0()); // new type after begin: kept
        let later = t0() + secs(30);
        assert!(c.finish_reconcile(
            token,
            &[("Patient".to_string(), 10), ("Condition".to_string(), 4)],
            later,
        ));
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
        let token = c.begin_reconcile("t");
        assert!(c.finish_reconcile(
            token,
            &[
                ("Patient".to_string(), 12),
                ("Encounter".to_string(), 1),
                ("Condition".to_string(), 4),
            ],
            later,
        ));
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
        let old = c.begin_reconcile("t");
        c.record("t", "Patient", 1, t0());
        let new = c.begin_reconcile("t");
        assert!(c.finish_reconcile(new, &[("Patient".to_string(), 1)], t0() + secs(2)));
        assert!(c.finish_reconcile(old, &[("Patient".to_string(), 0)], t0() + secs(1)));
        let view = c.totals_view("t").unwrap();
        assert_eq!(total_of(&view, "Patient"), Some(1));
        assert_eq!(view.reconciled_at, t0() + secs(2));
        assert!(view.exact);
    }

    #[test]
    fn stale_tokens_after_invalidate_are_rejected() {
        let c = DashboardCounters::new();
        c.record("t", "Patient", 3, t0());
        let reconcile = c.begin_reconcile("t");
        let ring = c.begin_ring_seed("t", "Patient", HOUR);
        c.invalidate_tenant("t");
        assert!(!c.finish_reconcile(reconcile, &[("Patient".to_string(), 3)], t0()));
        assert!(!c.finish_ring_seed(ring, &[(t0(), 3)], t0()));
        assert!(!c.is_seeded("t"));
        assert!(c.tenants().is_empty());

        // Also rejected when the tenant has been recreated by a later write.
        let reconcile = c.begin_reconcile("t");
        let ring = c.begin_ring_seed("t", "Patient", HOUR);
        c.invalidate_tenant("t");
        c.record("t", "Patient", 1, t0());
        assert!(!c.finish_reconcile(reconcile, &[], t0()));
        assert!(!c.finish_ring_seed(ring, &[], t0()));
        assert!(!c.is_seeded("t"));
        assert_eq!(c.live_delta("t", "Patient"), 1);
    }

    #[test]
    fn invalidate_forgets_a_seeded_tenant() {
        let c = DashboardCounters::new();
        seed(&c, "t", &[("Patient", 3)], t0());
        seed(&c, "other", &[("Patient", 1)], t0());
        assert!(c.is_seeded("t"));
        c.invalidate_tenant("t");
        assert!(!c.is_seeded("t"));
        assert!(c.totals_view("t").is_none());
        assert_eq!(c.live_delta("t", "Patient"), 0);
        assert!(c.is_seeded("other"), "other tenants are untouched");
        // Invalidating an unknown tenant is harmless.
        c.invalidate_tenant("nobody");
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

    #[test]
    fn global_helpers_record_into_the_global_counters() {
        let tenant = "dashboard-counters-global-helper-test";
        record_created(tenant, "Patient", 3);
        record_deleted(tenant, "Patient", 1);
        record_created(tenant, "Patient", 0);
        assert_eq!(global().live_delta(tenant, "Patient"), 2);
        invalidate_tenant(tenant);
        assert_eq!(global().live_delta(tenant, "Patient"), 0);
        assert!(!global().tenants().contains(&tenant.to_string()));
    }
}
