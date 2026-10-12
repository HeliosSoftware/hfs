//! Process-level Prometheus gauges for the server's PostgreSQL connection
//! pools (#1907).
//!
//! An exhausted pool shows up to clients as slow requests and, once the wait
//! timeout elapses, as `503 connection pool exhausted`. These gauges let an
//! operator see the saturation itself: the configured ceiling, the
//! connections open, the open ones that are idle, and the callers parked
//! waiting for one.
//!
//! The values are sampled at scrape time. The server registers one source per
//! pool with [`register_postgres_pool`]; [`crate::metrics`] calls [`publish`]
//! just before rendering `/metrics`, the same way it sets `uptime_seconds`,
//! because the pull exporter has no scrape callback. A process that registers
//! no pool (SQLite, MongoDB, S3) exports no `postgres_pool_*` series.
//!
//! ## Tenant privacy and cardinality
//!
//! `/metrics` is public (see [`crate::metrics`]). The only label is `pool`,
//! whose values are a fixed set chosen by the server at startup (`primary`,
//! `audit`). No tenant, resource type or database name.
//!
//! Every function is safe to call without an installed recorder: the
//! [`metrics`] facade then records into its no-op recorder.

use std::sync::{Arc, RwLock};

/// Gauge: the pool's configured maximum number of connections.
pub(crate) const MAX_CONNECTIONS: &str = "postgres_pool_max_connections";
/// Gauge: connections currently open, idle or in use. Grows lazily up to the
/// maximum.
pub(crate) const CONNECTIONS: &str = "postgres_pool_connections";
/// Gauge: open connections that are idle and can be checked out now.
pub(crate) const AVAILABLE_CONNECTIONS: &str = "postgres_pool_available_connections";
/// Gauge: callers waiting for a connection.
pub(crate) const WAITING_CALLERS: &str = "postgres_pool_waiting_callers";

/// One pool's state at the moment it is sampled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolGauges {
    /// The configured maximum number of connections.
    pub max_size: usize,
    /// Connections currently open, idle or in use.
    pub size: usize,
    /// Open connections that are idle.
    pub available: usize,
    /// Callers waiting for a connection.
    pub waiting: usize,
}

type Source = Arc<dyn Fn() -> PoolGauges + Send + Sync>;

/// Named pool sources, sampled on every [`publish`](Self::publish).
///
/// The server uses the process-global instance through
/// [`register_postgres_pool`]; tests build their own so they do not share
/// state.
pub struct PoolRegistry {
    pools: RwLock<Vec<(&'static str, Source)>>,
}

impl PoolRegistry {
    /// An empty registry.
    pub const fn new() -> Self {
        Self {
            pools: RwLock::new(Vec::new()),
        }
    }

    /// Register `source` as the pool called `name`. Registering a name again
    /// replaces the earlier source, so each name has one series per gauge and
    /// a later registration never competes with one left behind.
    pub fn register(
        &self,
        name: &'static str,
        source: impl Fn() -> PoolGauges + Send + Sync + 'static,
    ) {
        let source: Source = Arc::new(source);
        // A writer that panicked cannot leave the list half-updated (each
        // update is a single push or assignment), so a poisoned lock is safe
        // to keep using.
        let mut pools = self.pools.write().unwrap_or_else(|e| e.into_inner());
        match pools.iter_mut().find(|(existing, _)| *existing == name) {
            Some(entry) => entry.1 = source,
            None => pools.push((name, source)),
        }
    }

    /// Sample every registered pool and set its four gauges, labelled
    /// `pool => name`.
    pub fn publish(&self) {
        // Clone the sources out so none is called while the lock is held.
        let pools: Vec<(&'static str, Source)> =
            self.pools.read().unwrap_or_else(|e| e.into_inner()).clone();
        for (name, source) in pools {
            let gauges = source();
            metrics::gauge!(MAX_CONNECTIONS, "pool" => name).set(gauges.max_size as f64);
            metrics::gauge!(CONNECTIONS, "pool" => name).set(gauges.size as f64);
            metrics::gauge!(AVAILABLE_CONNECTIONS, "pool" => name).set(gauges.available as f64);
            metrics::gauge!(WAITING_CALLERS, "pool" => name).set(gauges.waiting as f64);
        }
    }
}

impl Default for PoolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL: PoolRegistry = PoolRegistry::new();

/// Register (or replace) the PostgreSQL pool called `name` in the
/// process-global registry that `/metrics` publishes. `name` becomes the
/// `pool` label and must come from a fixed set (`primary`, `audit`).
pub fn register_postgres_pool(
    name: &'static str,
    source: impl Fn() -> PoolGauges + Send + Sync + 'static,
) {
    GLOBAL.register(name, source);
}

/// Publish the process-global registry. Called by the `/metrics` handler.
pub(crate) fn publish() {
    GLOBAL.publish();
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Publishes `registry` through a recorder built exactly as
    /// [`crate::metrics::init`] builds the global one, installed only for
    /// this thread, and returns the rendered text.
    fn render(registry: &PoolRegistry) -> String {
        let recorder = crate::metrics::builder("test").build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, || registry.publish());
        handle.render()
    }

    /// The sample lines (not comments) of `metric` for `pool`.
    fn series<'a>(text: &'a str, metric: &str, pool: &str) -> Vec<&'a str> {
        let pool = format!("pool=\"{pool}\"");
        text.lines()
            .filter(|line| !line.starts_with('#'))
            .filter(|line| line.split_once('{').map(|(name, _)| name) == Some(metric))
            .filter(|line| line.contains(&pool))
            .collect()
    }

    fn value(text: &str, metric: &str, pool: &str) -> Option<f64> {
        match series(text, metric, pool).as_slice() {
            [line] => line.rsplit_once(' ').and_then(|(_, v)| v.parse().ok()),
            _ => None,
        }
    }

    fn gauges(max_size: usize, size: usize, available: usize, waiting: usize) -> PoolGauges {
        PoolGauges {
            max_size,
            size,
            available,
            waiting,
        }
    }

    fn assert_pool(text: &str, pool: &str, expected: PoolGauges) {
        for (metric, want) in [
            (MAX_CONNECTIONS, expected.max_size),
            (CONNECTIONS, expected.size),
            (AVAILABLE_CONNECTIONS, expected.available),
            (WAITING_CALLERS, expected.waiting),
        ] {
            assert_eq!(
                value(text, metric, pool),
                Some(want as f64),
                "{metric}{{pool=\"{pool}\"}}:\n{text}"
            );
        }
    }

    #[test]
    fn publishing_without_a_recorder_does_not_panic() {
        let registry = PoolRegistry::new();
        registry.register("primary", || gauges(4, 3, 2, 1));
        registry.publish();
    }

    #[test]
    fn an_empty_registry_publishes_no_pool_series() {
        let text = render(&PoolRegistry::new());
        assert!(
            !text.contains("postgres_pool_"),
            "no pool registered, no pool series:\n{text}"
        );
    }

    #[test]
    fn a_source_is_sampled_on_every_publish() {
        let registry = PoolRegistry::new();
        let waiting = Arc::new(AtomicUsize::new(0));
        registry.register("primary", {
            let waiting = Arc::clone(&waiting);
            move || gauges(4, 3, 2, waiting.load(Ordering::SeqCst))
        });

        assert_pool(&render(&registry), "primary", gauges(4, 3, 2, 0));

        waiting.store(5, Ordering::SeqCst);
        assert_pool(&render(&registry), "primary", gauges(4, 3, 2, 5));
    }

    #[test]
    fn registering_a_name_again_replaces_its_source() {
        let registry = PoolRegistry::new();
        let replaced_calls = Arc::new(AtomicUsize::new(0));
        registry.register("primary", {
            let replaced_calls = Arc::clone(&replaced_calls);
            move || {
                replaced_calls.fetch_add(1, Ordering::SeqCst);
                gauges(4, 3, 2, 1)
            }
        });
        registry.register("primary", || gauges(8, 7, 6, 5));

        // The recorder keys gauges by name and labels, so an append-only
        // `register` would still render one series with the last value.
        // Check the replaced source is gone, not just outvoted.
        assert_eq!(
            registry.pools.read().unwrap().len(),
            1,
            "one entry per name"
        );
        let text = render(&registry);
        assert_eq!(
            replaced_calls.load(Ordering::SeqCst),
            0,
            "the replaced source is never sampled"
        );
        assert_pool(&text, "primary", gauges(8, 7, 6, 5));
        assert_eq!(
            series(&text, AVAILABLE_CONNECTIONS, "primary").len(),
            1,
            "one series per name:\n{text}"
        );
    }

    #[test]
    fn primary_and_audit_pools_coexist_with_only_service_and_pool_labels() {
        let registry = PoolRegistry::new();
        registry.register("primary", || gauges(4, 3, 2, 1));
        registry.register("audit", || gauges(9, 8, 7, 6));

        let text = render(&registry);
        assert_pool(&text, "primary", gauges(4, 3, 2, 1));
        assert_pool(&text, "audit", gauges(9, 8, 7, 6));

        // Every label key is `service` or `pool`.
        for line in text.lines().filter(|line| !line.starts_with('#')) {
            let Some((_, labels)) = line.split_once('{') else {
                continue;
            };
            let labels = labels.split_once('}').map(|(l, _)| l).unwrap_or(labels);
            for label in labels.split(',') {
                let key = label.split_once('=').map(|(k, _)| k).unwrap_or(label);
                assert!(
                    ["service", "pool"].contains(&key),
                    "unexpected label {key} in: {line}"
                );
            }
        }
    }

    #[test]
    fn a_poisoned_registry_still_registers_and_publishes() {
        let registry = Arc::new(PoolRegistry::new());
        let poisoner = Arc::clone(&registry);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.pools.write().unwrap();
            panic!("poison the registry lock");
        })
        .join();
        assert!(registry.pools.is_poisoned());

        registry.register("primary", || gauges(4, 3, 2, 1));
        assert_pool(&render(&registry), "primary", gauges(4, 3, 2, 1));
    }
}
