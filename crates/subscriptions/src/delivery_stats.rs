//! In-memory rolling delivery counters (#586).
//!
//! The operator page's "delivered in 24 h" card and its per-subscription
//! last-24-hours column need delivery outcomes counted over time — something
//! the engine never recorded. This is the cheapest honest source: a ring of
//! 48 half-hour, epoch-aligned buckets per subscription (the same shape as
//! the dashboard's `DashboardWindow::LastDay`), bumped once per delivery
//! outcome at the single dispatch funnel.
//!
//! **Volatile by design.** The window resets with the process, like the rest
//! of the engine's in-memory state — after a restart the figures rebuild as
//! deliveries happen. A durable delivery log (queryable across restarts) is a
//! separate, storage-backed piece of work; this deliberately is not it.

use std::sync::Mutex;

use dashmap::DashMap;

/// Width of one bucket, in seconds. Half-hour buckets over 24 hours mirror
/// the dashboard's day window, so the two surfaces bucket time identically.
pub const BUCKET_SECONDS: i64 = 1_800;
/// Buckets kept per subscription: 48 × 30 min = 24 h.
pub const BUCKET_COUNT: usize = 48;

/// One bucket's counters. `start` identifies the half-hour it covers; a slot
/// is reused for a new window by resetting when `start` moves on.
#[derive(Clone, Copy, Default)]
struct Bucket {
    start: i64,
    delivered: u64,
    first_try: u64,
    failed: u64,
}

/// Delivery outcomes for one subscription over the last 24 hours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeliveryWindow {
    /// Notifications that reached the endpoint (any attempt).
    pub delivered: u64,
    /// The subset delivered on the first attempt.
    pub first_try: u64,
    /// Notifications abandoned: permanent errors and exhausted retries.
    pub failed: u64,
}

/// Rolling per-subscription delivery counters, keyed by (tenant, id).
#[derive(Default)]
pub struct DeliveryStats {
    rings: DashMap<(String, String), Mutex<[Bucket; BUCKET_COUNT]>>,
}

impl DeliveryStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a successful delivery at `now_epoch` seconds.
    pub fn record_success(&self, tenant: &str, id: &str, first_try: bool, now_epoch: i64) {
        self.bump(tenant, id, now_epoch, |b| {
            b.delivered += 1;
            if first_try {
                b.first_try += 1;
            }
        });
    }

    /// Records an abandoned delivery (permanent error or retries exhausted).
    pub fn record_failure(&self, tenant: &str, id: &str, now_epoch: i64) {
        self.bump(tenant, id, now_epoch, |b| b.failed += 1);
    }

    /// The last 24 hours of outcomes for one subscription, as of `now_epoch`.
    pub fn window(&self, tenant: &str, id: &str, now_epoch: i64) -> DeliveryWindow {
        let mut out = DeliveryWindow::default();
        let Some(ring) = self.rings.get(&(tenant.to_string(), id.to_string())) else {
            return out;
        };
        let horizon = bucket_start(now_epoch) - BUCKET_SECONDS * (BUCKET_COUNT as i64 - 1);
        let ring = ring.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for bucket in ring.iter() {
            if bucket.start >= horizon && bucket.start <= now_epoch {
                out.delivered += bucket.delivered;
                out.first_try += bucket.first_try;
                out.failed += bucket.failed;
            }
        }
        out
    }

    /// Drops a subscription's counters (deregistration).
    pub fn remove(&self, tenant: &str, id: &str) {
        self.rings.remove(&(tenant.to_string(), id.to_string()));
    }

    fn bump(&self, tenant: &str, id: &str, now_epoch: i64, apply: impl FnOnce(&mut Bucket)) {
        let start = bucket_start(now_epoch);
        let index = ((start / BUCKET_SECONDS) as usize) % BUCKET_COUNT;
        let entry = self
            .rings
            .entry((tenant.to_string(), id.to_string()))
            .or_insert_with(|| Mutex::new([Bucket::default(); BUCKET_COUNT]));
        let mut ring = entry.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let bucket = &mut ring[index];
        if bucket.start != start {
            // The slot's old half-hour aged out of the window; reuse it.
            *bucket = Bucket {
                start,
                ..Bucket::default()
            };
        }
        apply(bucket);
    }
}

/// Floors an epoch second to its bucket's start.
fn bucket_start(epoch: i64) -> i64 {
    epoch.div_euclid(BUCKET_SECONDS) * BUCKET_SECONDS
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_755_600_000; // an exact bucket boundary

    #[test]
    fn outcomes_accumulate_within_the_window() {
        let stats = DeliveryStats::new();
        stats.record_success("t", "s", true, T0);
        stats.record_success("t", "s", false, T0 + 10);
        stats.record_failure("t", "s", T0 + 20);

        let w = stats.window("t", "s", T0 + 30);
        assert_eq!(
            w,
            DeliveryWindow {
                delivered: 2,
                first_try: 1,
                failed: 1
            }
        );
        // Another subscription is untouched.
        assert_eq!(stats.window("t", "other", T0 + 30), DeliveryWindow::default());
    }

    #[test]
    fn buckets_age_out_after_twenty_four_hours() {
        let stats = DeliveryStats::new();
        stats.record_success("t", "s", true, T0);
        let inside = T0 + BUCKET_SECONDS * (BUCKET_COUNT as i64 - 1);
        assert_eq!(stats.window("t", "s", inside).delivered, 1, "still inside");
        let outside = T0 + BUCKET_SECONDS * (BUCKET_COUNT as i64);
        assert_eq!(stats.window("t", "s", outside).delivered, 0, "aged out");
    }

    #[test]
    fn a_slot_is_reset_when_its_half_hour_comes_around_again() {
        let stats = DeliveryStats::new();
        stats.record_success("t", "s", true, T0);
        // Exactly one full ring later the same slot is reused for a new window.
        let next_cycle = T0 + BUCKET_SECONDS * (BUCKET_COUNT as i64);
        stats.record_failure("t", "s", next_cycle);
        let w = stats.window("t", "s", next_cycle);
        assert_eq!(
            w,
            DeliveryWindow {
                delivered: 0,
                first_try: 0,
                failed: 1
            }
        );
    }

    #[test]
    fn remove_drops_the_ring() {
        let stats = DeliveryStats::new();
        stats.record_success("t", "s", true, T0);
        stats.remove("t", "s");
        assert_eq!(stats.window("t", "s", T0).delivered, 0);
    }
}
