//! Cross-instance terminology cache invalidation (cluster-capable-state
//! effort, Class C3).
//!
//! A CodeSystem/ValueSet/ConceptMap write clears this *instance's* response
//! caches (`PostgresTerminologyBackend::clear_response_caches`,
//! `AppState::clear_expand_cache`) but not another instance's — an import on
//! A leaves B serving stale `$expand`/`$validate-code`/`$lookup`/
//! `$translate`/`$subsumes` results indefinitely.
//!
//! [`EpochGuard`] closes that gap with a single-row `terminology_epoch`
//! counter: every write bumps it, and every read choke point checks it
//! (memoized, so most reads pay no DB round trip) before trusting its local
//! caches, clearing them on a detected transition.
//!
//! Opt-in and standalone to the `hts` crate — see `HTS_TERMINOLOGY_CACHE_INVALIDATION`
//! in `crate::config`. Not coupled to the `hfs` binary's `HFS_CLUSTER` switch;
//! HTS can be scaled independently of the FHIR server.

use deadpool_postgres::Pool;
use tokio::sync::Mutex;

use crate::error::HtsError;

/// The epoch value new (never-bumped) databases start at, matching the
/// seed row `INSERT INTO terminology_epoch (id, epoch) VALUES (1, 1)`.
const INITIAL_EPOCH: i64 = 1;

struct GuardState {
    /// Most recently observed value of `terminology_epoch.epoch`, refreshed
    /// at most once per `memo_window`.
    last_fetched_epoch: i64,
    last_fetched_at: Option<std::time::Instant>,
    /// Epoch value the `AppState`-layer handler caches were last cleared for.
    appstate_cleared_epoch: i64,
    /// Epoch value the backend-layer response caches were last cleared for.
    backend_cleared_epoch: i64,
}

/// Cross-instance freshness check for HTS's Postgres-backed response
/// caches, gated by `HTS_TERMINOLOGY_CACHE_INVALIDATION`.
///
/// One `EpochGuard` is shared (via `Arc`) between a `PostgresTerminologyBackend`
/// and every `AppState` built over it, since the two own disjoint cache sets
/// (backend-layer response caches vs. `AppState`-layer handler caches) that
/// must each independently decide whether to clear on a detected epoch
/// transition — a single shared "have we cleared yet" flag would let
/// whichever layer checks first silently suppress the other's clear within
/// the same memo window, so each layer tracks its own `*_cleared_epoch`
/// against one shared, memoized fetch of the current DB epoch.
///
/// `enabled = false` (the `local` mode, and the default) makes every check a
/// cheap branch — zero DB round trips, zero locking beyond the uncontended
/// `Mutex` check.
pub struct EpochGuard {
    pool: Pool,
    enabled: bool,
    memo_window: std::time::Duration,
    state: Mutex<GuardState>,
}

impl EpochGuard {
    /// `memo_window` is a constructor parameter, not a `cfg(test)` switch, so
    /// the same code path runs in production (~1s) and tests (`Duration::ZERO`
    /// for determinism).
    pub fn new(pool: Pool, enabled: bool, memo_window: std::time::Duration) -> Self {
        Self {
            pool,
            enabled,
            memo_window,
            state: Mutex::new(GuardState {
                last_fetched_epoch: INITIAL_EPOCH,
                last_fetched_at: None,
                appstate_cleared_epoch: INITIAL_EPOCH,
                backend_cleared_epoch: INITIAL_EPOCH,
            }),
        }
    }

    /// A disabled guard (`local` mode) — never touches the database.
    pub fn disabled(pool: Pool) -> Self {
        Self::new(pool, false, std::time::Duration::from_secs(1))
    }

    /// Checks the `AppState`-layer handler caches' freshness, running `clear`
    /// if the epoch advanced since this layer's own last clear. No-ops
    /// entirely when disabled. Never fails the caller's request on a DB
    /// error — a transient epoch-check failure logs a warning and keeps
    /// serving from whatever is currently cached (availability over
    /// strict freshness, matching the JWKS coordination fallback posture).
    pub async fn check_appstate(&self, clear: impl FnOnce()) {
        self.check(CacheLayer::AppState, clear).await;
    }

    /// Checks the backend-layer response caches' freshness. See
    /// [`Self::check_appstate`] for the shared contract.
    pub async fn check_backend(&self, clear: impl FnOnce()) {
        self.check(CacheLayer::Backend, clear).await;
    }

    async fn check(&self, layer: CacheLayer, clear: impl FnOnce()) {
        if !self.enabled {
            return;
        }
        let epoch = match self.current_epoch().await {
            Ok(epoch) => epoch,
            Err(e) => {
                tracing::warn!(error = %e, "terminology epoch check failed; serving from local caches");
                return;
            }
        };
        let mut state = self.state.lock().await;
        let cleared_epoch = match layer {
            CacheLayer::AppState => &mut state.appstate_cleared_epoch,
            CacheLayer::Backend => &mut state.backend_cleared_epoch,
        };
        if epoch > *cleared_epoch {
            clear();
            *cleared_epoch = epoch;
        }
    }

    /// Returns the current epoch, refreshing from the database at most once
    /// per `memo_window`.
    async fn current_epoch(&self) -> Result<i64, HtsError> {
        {
            let state = self.state.lock().await;
            if let Some(last_fetched_at) = state.last_fetched_at {
                if last_fetched_at.elapsed() < self.memo_window {
                    return Ok(state.last_fetched_epoch);
                }
            }
        }

        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
        let row = client
            .query_one("SELECT epoch FROM terminology_epoch WHERE id = 1", &[])
            .await
            .map_err(|e| HtsError::StorageError(format!("terminology_epoch read: {e}")))?;
        let epoch: i64 = row.get(0);

        let mut state = self.state.lock().await;
        state.last_fetched_epoch = epoch;
        state.last_fetched_at = Some(std::time::Instant::now());
        Ok(epoch)
    }

    /// Bumps the shared epoch after a successful terminology write. Not
    /// transactional with the write it follows (the caller has already
    /// committed and run its own local cache clears by this point) — a
    /// best-effort statement that bounds cross-instance staleness to
    /// roughly `memo_window`, not the write's own transaction. No-ops when
    /// disabled. Errors are the caller's to log-and-continue on; a failed
    /// bump must never fail the write it followed.
    pub async fn bump(&self) -> Result<(), HtsError> {
        if !self.enabled {
            return Ok(());
        }
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
        let row = client
            .query_one(
                "UPDATE terminology_epoch SET epoch = epoch + 1, bumped_at = now() \
                 WHERE id = 1 RETURNING epoch",
                &[],
            )
            .await
            .map_err(|e| HtsError::StorageError(format!("terminology_epoch bump: {e}")))?;
        let epoch: i64 = row.get(0);

        // Optimistically install the value we just wrote so this instance's
        // own next check (on either layer) doesn't pay a redundant SELECT
        // within the same memo window.
        let mut state = self.state.lock().await;
        state.last_fetched_epoch = epoch;
        state.last_fetched_at = Some(std::time::Instant::now());
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum CacheLayer {
    AppState,
    Backend,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_pool() -> Pool {
        // A guard constructed disabled never touches the pool, so an
        // unconnectable config is fine for the `enabled=false` no-op tests.
        let mut cfg = deadpool_postgres::Config::new();
        cfg.url = Some("postgres://invalid:invalid@127.0.0.1:1/invalid".to_string());
        cfg.create_pool(
            Some(deadpool_postgres::Runtime::Tokio1),
            tokio_postgres::NoTls,
        )
        .expect("pool config should build even for an unreachable host")
    }

    #[tokio::test]
    async fn disabled_guard_never_touches_the_pool() {
        let guard = EpochGuard::disabled(test_pool());
        let mut cleared = false;
        guard.check_appstate(|| cleared = true).await;
        assert!(!cleared, "a disabled guard must never clear");
        guard.check_backend(|| cleared = true).await;
        assert!(!cleared, "a disabled guard must never clear");
        // Would hang/error against the unreachable pool if it attempted a
        // real connection — bump() must also no-op.
        guard.bump().await.expect("disabled bump is a no-op Ok");
    }
}
