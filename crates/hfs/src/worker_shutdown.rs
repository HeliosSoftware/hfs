//! Graceful shutdown of the in-process bulk export and submit worker pools
//! (#1531).
//!
//! Without this, a stopping server dropped its workers along with the runtime,
//! and every job they held stayed leased until the lease expired
//! (`HFS_BULK_{EXPORT,SUBMIT}_LEASE_DURATION`, 60 s by default). During a
//! rolling restart no other instance could pick those jobs up in the
//! meantime. Now the shutdown path cancels a shared token: each worker stops
//! claiming, stops its job at a safe boundary, releases the lease, and exits.
//! [`WorkerPools::drain`] waits for that, up to a deadline.
//!
//! The pools are process-global because the worker pools are spawned deep in
//! each backend's bootstrap and the HTTP server is started from a different
//! place. Threading one handle through every combination would add a
//! parameter to a dozen functions for no gain.

use std::future::Future;
use std::sync::LazyLock;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

/// How long shutdown waits for the workers to release their leases when
/// `HFS_WORKER_SHUTDOWN_TIMEOUT` is unset. Kept below the 30 s grace period
/// that Kubernetes and most process supervisors allow before `SIGKILL`.
const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(20);

static POOLS: LazyLock<WorkerPools> = LazyLock::new(WorkerPools::default);

/// The process's bulk worker pools.
pub(crate) fn pools() -> &'static WorkerPools {
    &POOLS
}

/// The bulk export and submit worker loops, and the token that stops them.
#[derive(Default)]
pub(crate) struct WorkerPools {
    shutdown: CancellationToken,
    tracker: TaskTracker,
}

impl WorkerPools {
    /// The token a worker watches: cancelled once shutdown begins.
    pub(crate) fn token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    /// Spawns a worker loop that shutdown waits for.
    pub(crate) fn spawn<F>(&self, worker: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.tracker.spawn(worker);
    }

    /// Tells every worker to stop claiming and to hand its lease back.
    pub(crate) fn begin_shutdown(&self) {
        self.shutdown.cancel();
    }

    /// Stops the workers and waits, up to `timeout`, for each to release its
    /// lease and exit. Returns whether all of them did.
    ///
    /// A worker still running at the deadline is dropped with the runtime,
    /// and its lease lapses on its own, as it did before #1531.
    pub(crate) async fn drain(&self, timeout: Duration) -> bool {
        self.begin_shutdown();
        self.tracker.close();
        tokio::time::timeout(timeout, self.tracker.wait())
            .await
            .is_ok()
    }

    /// How many worker loops are still running.
    pub(crate) fn running(&self) -> usize {
        self.tracker.len()
    }
}

/// `HFS_WORKER_SHUTDOWN_TIMEOUT`, in seconds: how long shutdown waits for the
/// bulk workers to release their leases.
pub(crate) fn drain_timeout() -> Duration {
    parse_drain_timeout(std::env::var("HFS_WORKER_SHUTDOWN_TIMEOUT").ok().as_deref())
}

fn parse_drain_timeout(value: Option<&str>) -> Duration {
    match value.map(str::trim) {
        None | Some("") => DEFAULT_DRAIN_TIMEOUT,
        Some(raw) => match raw.parse::<u64>() {
            Ok(secs) => Duration::from_secs(secs),
            Err(_) => {
                tracing::warn!(
                    value = raw,
                    default_secs = DEFAULT_DRAIN_TIMEOUT.as_secs(),
                    "HFS_WORKER_SHUTDOWN_TIMEOUT is not a whole number of seconds; using the default"
                );
                DEFAULT_DRAIN_TIMEOUT
            }
        },
    }
}

/// Sleeps for `period` unless shutdown begins first. Returns `true` when
/// shutdown began, so a worker loop can stop without waiting out its idle poll.
pub(crate) async fn idle(shutdown: &CancellationToken, period: Duration) -> bool {
    tokio::select! {
        biased;
        _ = shutdown.cancelled() => true,
        _ = tokio::time::sleep(period) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_timeout_defaults_and_parses_seconds() {
        assert_eq!(parse_drain_timeout(None), DEFAULT_DRAIN_TIMEOUT);
        assert_eq!(parse_drain_timeout(Some(" ")), DEFAULT_DRAIN_TIMEOUT);
        assert_eq!(parse_drain_timeout(Some("5")), Duration::from_secs(5));
        assert_eq!(parse_drain_timeout(Some("0")), Duration::ZERO);
        assert_eq!(parse_drain_timeout(Some("soon")), DEFAULT_DRAIN_TIMEOUT);
    }

    #[tokio::test]
    async fn drain_stops_idle_workers_and_waits_for_them() {
        let pools = WorkerPools::default();
        for _ in 0..3 {
            let shutdown = pools.token();
            pools.spawn(async move { while !idle(&shutdown, Duration::from_secs(3600)).await {} });
        }
        assert_eq!(pools.running(), 3);
        assert!(pools.drain(Duration::from_secs(5)).await);
        assert_eq!(pools.running(), 0);
    }

    #[tokio::test]
    async fn drain_gives_up_on_a_worker_that_outlives_the_deadline() {
        let pools = WorkerPools::default();
        pools.spawn(std::future::pending());
        assert!(!pools.drain(Duration::from_millis(50)).await);
        assert_eq!(pools.running(), 1);
    }
}
