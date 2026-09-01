//! Process-global read path for the subscriptions operator page (#580).
//!
//! The web UI's Subscriptions page renders the engine's live state — statuses,
//! event counters, failure streaks, connected WebSocket clients. That state
//! lives inside `helios-subscriptions`, wired up deep inside `helios-rest`'s
//! app builder — a layer the UI crate deliberately does not depend on. As with
//! [`crate::dashboard`], the server registers a provider here at startup and
//! the UI reads snapshots without knowing anything about the engine.
//!
//! Everything is plain data so this crate stays dependency-light, and the
//! provider is synchronous: the engine's inventory is an in-memory map, so a
//! snapshot is a cheap read with no storage round-trip.
//!
//! Whether a provider is registered doubles as the feature signal: the page
//! (and its nav entry) only appears when the subscriptions engine is enabled.

use std::sync::{Arc, RwLock};

/// One subscription, as the operator table renders it.
#[derive(Clone, Debug)]
pub struct SubscriptionRow {
    /// The Subscription resource id.
    pub id: String,
    /// Canonical URL of the SubscriptionTopic it watches.
    pub topic_url: String,
    /// Channel type (`rest-hook`, `websocket`, `email`, `message`, …).
    pub channel_type: String,
    /// Delivery endpoint, when the channel has one.
    pub endpoint: Option<String>,
    /// Engine status code (`requested`, `active`, `error`, `off`,
    /// `entered-in-error`).
    pub status: String,
    /// Notifications dispatched since the subscription was registered.
    pub events_since_start: u64,
    /// Consecutive delivery failures (drives the active → error transition).
    pub consecutive_failures: u32,
    /// Connected WebSocket clients — `Some` only for websocket channels, where
    /// zero listeners means notifications go nowhere.
    pub ws_clients: Option<usize>,
    /// Notifications delivered in the last 24 hours (#586). In-memory rolling
    /// window: resets with the process, like the engine's other state.
    pub delivered_24h: u64,
    /// The subset of `delivered_24h` that landed on the first attempt.
    pub first_try_24h: u64,
    /// Deliveries abandoned in the last 24 hours (permanent errors and
    /// exhausted retries).
    pub failed_24h: u64,
    /// `delivered_24h` as a chronological series — one count per half-hour
    /// bucket, oldest first (#782). Empty when the engine carries no series;
    /// the UI then renders no sparkline rather than a fabricated flat line.
    pub delivered_series: Vec<u64>,
}

/// The engine's current inventory for one tenant.
#[derive(Clone, Debug, Default)]
pub struct SubscriptionsSnapshot {
    pub rows: Vec<SubscriptionRow>,
}

/// Supplies [`SubscriptionsSnapshot`]s on demand. Implemented in `helios-rest`
/// over the live engine and registered via [`set_provider`] at startup.
pub trait SubscriptionsProvider: Send + Sync {
    /// The engine's inventory for `tenant`. Cheap — reads in-memory state.
    fn snapshot(&self, tenant: &str) -> SubscriptionsSnapshot;
}

static PROVIDER: RwLock<Option<Arc<dyn SubscriptionsProvider>>> = RwLock::new(None);

/// Register (or replace) the process-global subscriptions provider. The most
/// recent registration wins, so a later real server never reads a provider
/// left behind by an earlier one.
pub fn set_provider(provider: Arc<dyn SubscriptionsProvider>) {
    if let Ok(mut guard) = PROVIDER.write() {
        *guard = Some(provider);
    }
}

/// Whether a provider is registered — the UI's "subscriptions advertised"
/// signal.
pub fn enabled() -> bool {
    PROVIDER.read().map(|g| g.is_some()).unwrap_or(false)
}

/// The engine's inventory for `tenant`, or `None` when no provider is
/// registered (feature disabled, or a build without the engine).
pub fn snapshot(tenant: &str) -> Option<SubscriptionsSnapshot> {
    let provider = PROVIDER.read().ok()?.clone()?;
    Some(provider.snapshot(tenant))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed;
    impl SubscriptionsProvider for Fixed {
        fn snapshot(&self, tenant: &str) -> SubscriptionsSnapshot {
            SubscriptionsSnapshot {
                rows: vec![SubscriptionRow {
                    id: "sub-1".into(),
                    topic_url: "http://example.org/topics/vitals".into(),
                    channel_type: "websocket".into(),
                    endpoint: None,
                    status: "active".into(),
                    events_since_start: 1402,
                    consecutive_failures: 0,
                    ws_clients: Some(if tenant == "busy" { 3 } else { 0 }),
                    delivered_24h: 12,
                    first_try_24h: 11,
                    failed_24h: 1,
                    delivered_series: vec![0, 4, 8],
                }],
            }
        }
    }

    #[test]
    fn registered_provider_round_trips_and_signals_enabled() {
        assert!(snapshot("default").is_none() || enabled());
        set_provider(Arc::new(Fixed));
        assert!(enabled());
        let snap = snapshot("busy").expect("provider registered");
        assert_eq!(snap.rows.len(), 1);
        assert_eq!(snap.rows[0].ws_clients, Some(3));
    }
}
