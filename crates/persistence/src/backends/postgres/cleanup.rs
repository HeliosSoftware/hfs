//! Exceptional cleanup of PostgreSQL sessions that may hold an open transaction.

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use deadpool_postgres::{Client, ClientWrapper};
use parking_lot::Mutex;
use tokio::sync::{OwnedSemaphorePermit, watch};

/// Counts sessions removed from the pool until rollback or client discard ends.
pub(super) struct CleanupTracker {
    pending: Mutex<usize>,
    changed: watch::Sender<usize>,
}

/// One reindex admission shared across a group and its serial fallback.
/// An oversized resource can release the ordinary permit before requesting
/// the exclusive lane, even while the fallback still owns this handle.
pub(super) struct ReindexAdmission {
    permit: Mutex<Option<OwnedSemaphorePermit>>,
}

impl ReindexAdmission {
    pub(super) fn new(permit: OwnedSemaphorePermit) -> Arc<Self> {
        Arc::new(Self {
            permit: Mutex::new(Some(permit)),
        })
    }

    pub(super) fn is_active(&self) -> bool {
        self.permit.lock().is_some()
    }

    pub(super) fn install(&self, permit: OwnedSemaphorePermit) {
        let previous = self.permit.lock().replace(permit);
        assert!(previous.is_none(), "reindex admission already held");
    }

    pub(super) fn release(&self) {
        self.permit.lock().take();
    }
}

impl CleanupTracker {
    pub(super) fn new() -> Arc<Self> {
        let (changed, _) = watch::channel(0);
        Arc::new(Self {
            pending: Mutex::new(0),
            changed,
        })
    }

    fn increment(&self) {
        let mut pending = self.pending.lock();
        *pending += 1;
        self.changed.send_replace(*pending);
    }

    fn decrement(&self) {
        let mut pending = self.pending.lock();
        *pending -= 1;
        self.changed.send_replace(*pending);
    }

    /// Removes the session from the pool synchronously. Dropping an aborted
    /// cleanup task aborts its connection instead of recycling a dirty session.
    pub(super) fn settle_on_drop(
        self: &Arc<Self>,
        client: Client,
        permit: Option<Arc<ReindexAdmission>>,
    ) {
        self.increment();
        let client = Client::take(client);
        let cleanup = CleanupTask {
            client: Some(client),
            permit,
            tracker: self.clone(),
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    if let Some(client) = cleanup.client.as_ref() {
                        if let Err(error) = client.batch_execute("ROLLBACK").await {
                            tracing::warn!("PostgreSQL exceptional rollback failed; discarding session: {error}");
                        }
                    }
                });
            }
            Err(_) => {
                tracing::warn!(
                    "No runtime for PostgreSQL exceptional rollback; discarding session"
                );
                drop(cleanup);
            }
        }
    }

    pub(super) async fn wait(&self) {
        let mut changed = self.changed.subscribe();
        while *changed.borrow_and_update() != 0 {
            if changed.changed().await.is_err() {
                break;
            }
        }
    }
}

struct CleanupTask {
    client: Option<ClientWrapper>,
    permit: Option<Arc<ReindexAdmission>>,
    tracker: Arc<CleanupTracker>,
}

impl Drop for CleanupTask {
    fn drop(&mut self) {
        // Close/discard the session before releasing admission or announcing
        // that managed cleanup has completed.
        self.client.take();
        self.permit.take();
        self.tracker.decrement();
    }
}

/// Owns one pooled client from checkout through definite transaction settlement.
/// Cancellation or panic detaches an uncertain session before it can be reused.
pub(super) struct GuardedClient {
    client: Option<Client>,
    tracker: Arc<CleanupTracker>,
    permit: Option<Arc<ReindexAdmission>>,
    settled: bool,
}

impl GuardedClient {
    pub(super) fn new(
        client: Client,
        tracker: Arc<CleanupTracker>,
        permit: Option<Arc<ReindexAdmission>>,
    ) -> Self {
        Self {
            client: Some(client),
            tracker,
            permit,
            settled: false,
        }
    }

    /// Call only after a successful COMMIT or ROLLBACK response.
    pub(super) fn mark_settled(&mut self) {
        self.settled = true;
    }

    /// Transfers an intentionally open transaction to its long-lived owner.
    pub(super) fn hand_off(mut self) -> Client {
        self.client.take().expect("guarded client still owned")
    }
}

impl Deref for GuardedClient {
    type Target = Client;

    fn deref(&self) -> &Self::Target {
        self.client.as_ref().expect("guarded client still owned")
    }
}

impl DerefMut for GuardedClient {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.client.as_mut().expect("guarded client still owned")
    }
}

impl Drop for GuardedClient {
    fn drop(&mut self) {
        let Some(client) = self.client.take() else {
            return;
        };
        if self.settled {
            drop(client);
            self.permit.take();
        } else {
            self.tracker.settle_on_drop(client, self.permit.take());
        }
    }
}
