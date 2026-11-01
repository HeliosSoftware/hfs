//! Connection-pool statistics for the MongoDB driver (#2284).
//!
//! The driver exposes no pool gauges, only CMAP events. [`MongoPoolStats`]
//! keeps atomic counters fed by a CMAP event handler and is read at scrape
//! time, like the PostgreSQL pool gauges. The handler runs on the hot path of
//! every checkout, so it only does relaxed atomic updates and one histogram
//! record: no locks, no allocation.

use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use mongodb::event::EventHandler;
use mongodb::event::cmap::CmapEvent;

/// A point-in-time read of a pool's counters.
///
/// With a replica set the driver keeps one pool per server: `connections`,
/// `in_use`, `idle` and `waiters` are summed across servers while `max_size`
/// is per server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MongoPoolSnapshot {
    /// The effective `maxPoolSize` (per server).
    pub max_size: usize,
    /// Connections open, idle or in use.
    pub connections: usize,
    /// Connections checked out.
    pub in_use: usize,
    /// Open connections not checked out.
    pub idle: usize,
    /// Callers waiting for a connection.
    pub waiters: usize,
}

/// Atomic pool counters driven by CMAP events.
#[derive(Debug, Default)]
pub struct MongoPoolStats {
    connections: AtomicI64,
    in_use: AtomicI64,
    waiters: AtomicI64,
    max_size: AtomicU32,
    /// The `pool` metric label; the checkout-wait histogram is recorded only
    /// once it is set.
    label: OnceLock<&'static str>,
}

impl MongoPoolStats {
    /// Fresh counters, all zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the effective `maxPoolSize`.
    pub fn set_max_size(&self, max_size: u32) {
        self.max_size.store(max_size, Ordering::Relaxed);
    }

    /// Set the `pool` label (`primary`, `audit`). Only the first call takes
    /// effect.
    pub fn set_label(&self, label: &'static str) {
        let _ = self.label.set(label);
    }

    /// The CMAP handler to put in `ClientOptions::cmap_event_handler`.
    pub fn event_handler(self: &Arc<Self>) -> EventHandler<CmapEvent> {
        let stats = Arc::clone(self);
        EventHandler::callback(move |event: CmapEvent| match event {
            CmapEvent::ConnectionCreated(_) => stats.on_connection_created(),
            CmapEvent::ConnectionClosed(_) => stats.on_connection_closed(),
            CmapEvent::ConnectionCheckoutStarted(_) => stats.on_checkout_started(),
            CmapEvent::ConnectionCheckoutFailed(_) => stats.on_checkout_failed(),
            CmapEvent::ConnectionCheckedOut(e) => stats.on_checked_out(e.duration),
            CmapEvent::ConnectionCheckedIn(_) => stats.on_checked_in(),
            // `CmapEvent` is `#[non_exhaustive]`; the pool-level and
            // connection-ready events do not change the counters.
            _ => {}
        })
    }

    /// Read the counters. Negative values (events lost around a pool clear)
    /// are clamped to 0.
    pub fn snapshot(&self) -> MongoPoolSnapshot {
        let clamp = |v: i64| v.max(0) as usize;
        let connections = clamp(self.connections.load(Ordering::Relaxed));
        let in_use = clamp(self.in_use.load(Ordering::Relaxed));
        MongoPoolSnapshot {
            max_size: self.max_size.load(Ordering::Relaxed) as usize,
            connections,
            in_use,
            idle: connections.saturating_sub(in_use),
            waiters: clamp(self.waiters.load(Ordering::Relaxed)),
        }
    }

    pub(crate) fn on_connection_created(&self) {
        self.connections.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn on_connection_closed(&self) {
        self.connections.fetch_sub(1, Ordering::Relaxed);
    }

    pub(crate) fn on_checkout_started(&self) {
        self.waiters.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn on_checkout_failed(&self) {
        self.waiters.fetch_sub(1, Ordering::Relaxed);
    }

    pub(crate) fn on_checked_out(&self, wait: Duration) {
        self.waiters.fetch_sub(1, Ordering::Relaxed);
        self.in_use.fetch_add(1, Ordering::Relaxed);
        if let Some(label) = self.label.get() {
            helios_observability::pool_metrics::record_mongodb_checkout_wait(
                label,
                wait.as_secs_f64(),
            );
        }
    }

    pub(crate) fn on_checked_in(&self) {
        self.in_use.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(
        max_size: usize,
        connections: usize,
        in_use: usize,
        idle: usize,
        waiters: usize,
    ) -> MongoPoolSnapshot {
        MongoPoolSnapshot {
            max_size,
            connections,
            in_use,
            idle,
            waiters,
        }
    }

    #[test]
    fn starts_empty() {
        assert_eq!(MongoPoolStats::new().snapshot(), snap(0, 0, 0, 0, 0));
    }

    #[test]
    fn checkout_started_then_out_then_in() {
        let s = MongoPoolStats::new();
        s.set_max_size(3);
        s.on_connection_created();
        s.on_checkout_started();
        assert_eq!(s.snapshot(), snap(3, 1, 0, 1, 1));
        s.on_checked_out(Duration::from_millis(5));
        assert_eq!(s.snapshot(), snap(3, 1, 1, 0, 0));
        s.on_checked_in();
        assert_eq!(s.snapshot(), snap(3, 1, 0, 1, 0));
    }

    #[test]
    fn failed_checkout_leaves_the_wait_queue() {
        let s = MongoPoolStats::new();
        s.on_checkout_started();
        s.on_checkout_started();
        s.on_checkout_failed();
        assert_eq!(s.snapshot().waiters, 1);
    }

    #[test]
    fn closing_a_connection_shrinks_the_pool() {
        let s = MongoPoolStats::new();
        s.on_connection_created();
        s.on_connection_created();
        s.on_connection_closed();
        assert_eq!(s.snapshot().connections, 1);
    }

    #[test]
    fn negative_counters_are_clamped_to_zero() {
        let s = MongoPoolStats::new();
        s.on_connection_closed();
        s.on_checked_in();
        s.on_checkout_failed();
        assert_eq!(s.snapshot(), snap(0, 0, 0, 0, 0));
    }

    #[test]
    fn idle_never_underflows_when_in_use_exceeds_connections() {
        let s = MongoPoolStats::new();
        s.on_checked_out(Duration::ZERO);
        assert_eq!(s.snapshot().idle, 0);
    }

    #[test]
    fn the_label_is_set_once() {
        let s = MongoPoolStats::new();
        s.set_label("primary");
        s.set_label("audit");
        assert_eq!(s.label.get(), Some(&"primary"));
    }

    #[test]
    fn event_handler_builds() {
        let s = Arc::new(MongoPoolStats::new());
        let _ = s.event_handler();
    }
}
