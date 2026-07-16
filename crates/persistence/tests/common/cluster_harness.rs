//! T2 cluster-harness helpers.
//!
//! A cluster bug is a multi-observer bug, and the faithful single-process
//! simulation of two `hfs` instances is two *freshly constructed* backend
//! handles that share nothing but the backing store (same connection
//! parameters) — never a cloned `Arc`, which would share an in-process heap
//! and prove nothing (methodology §7, the cloned-`Arc` anti-pattern).
//!
//! These helpers encode the mechanics every T2 cluster suite reuses: fresh
//! two-handle construction, barrier-synchronized racing, and the
//! definition-of-done assertion rows (visibility / isolation / exclusivity,
//! strategy §4). Calibrated in Phase 0 against the already-cluster-safe
//! bulk-export job store (`postgres_integration_cluster_bulk_export_*` in
//! `postgres_tests.rs`); later phases point the same helpers at each new
//! cluster-capable subsystem.

use std::future::Future;
use std::sync::Arc;

/// Two independently constructed backend handles over one shared store.
///
/// `a` and `b` play the roles of "instance A" and "instance B". They must be
/// separate constructions — the factory in [`two_handles`] is invoked twice —
/// so the only thing they can possibly share is the backing store itself.
pub struct ClusterHandles<B> {
    pub a: B,
    pub b: B,
}

/// Builds two fresh handles by invoking `factory` twice.
///
/// The factory is the backend's normal constructor pointed at the shared
/// store (e.g. `create_backend()` against the shared Postgres container).
pub async fn two_handles<B, F, Fut>(factory: F) -> ClusterHandles<B>
where
    F: Fn() -> Fut,
    Fut: Future<Output = B>,
{
    ClusterHandles {
        a: factory().await,
        b: factory().await,
    }
}

/// Runs two futures on separate tasks released simultaneously by a barrier,
/// so both hit the shared store as close to concurrently as the runtime
/// allows (the same shape as the D3 cold-start race test).
///
/// Each future must own everything it touches (`Send + 'static`); move the
/// handle into an `async move` block and re-observe afterwards through a
/// fresh handle.
pub async fn race2<T, U>(
    fa: impl Future<Output = T> + Send + 'static,
    fb: impl Future<Output = U> + Send + 'static,
) -> (T, U)
where
    T: Send + 'static,
    U: Send + 'static,
{
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let barrier_a = Arc::clone(&barrier);
    let barrier_b = barrier;
    let task_a = tokio::spawn(async move {
        barrier_a.wait().await;
        fa.await
    });
    let task_b = tokio::spawn(async move {
        barrier_b.wait().await;
        fb.await
    });
    (
        task_a.await.expect("racing task A panicked"),
        task_b.await.expect("racing task B panicked"),
    )
}

/// Exclusivity row: of two racing claimants, exactly one may win.
///
/// Both winning is the cluster bug (double execution / double redeem);
/// neither winning means the race setup is wrong — also a failure, so the
/// test can't silently pass without exercising the claim.
pub fn assert_exactly_one<T, U>(a: &Option<T>, b: &Option<U>, what: &str) {
    match (a.is_some(), b.is_some()) {
        (true, false) | (false, true) => {}
        (true, true) => panic!("exclusivity violated: both handles won {what}"),
        (false, false) => {
            panic!("exclusivity check inconclusive: neither handle won {what}")
        }
    }
}

/// Visibility row: state created via handle A must be observable via handle
/// B (same tenant). Returns the observed value for follow-on assertions.
pub fn assert_visible<T>(got: Option<T>, what: &str) -> T {
    match got {
        Some(value) => value,
        None => panic!("visibility violated: {what} is not observable via the second handle"),
    }
}

/// Isolation row (mandatory on every suite): state created under one tenant
/// must not be observable under another — the observer sees nothing, exactly
/// as if the state did not exist.
pub fn assert_wrong_tenant_hidden<T>(got: Option<T>, what: &str) {
    assert!(
        got.is_none(),
        "tenant isolation violated: {what} is observable under the wrong tenant"
    );
}
