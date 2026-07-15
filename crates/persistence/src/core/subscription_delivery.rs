//! Durable subscription delivery outbox (design doc §Class B, B5; strategy
//! §8 Phase 3).
//!
//! Today's delivery retry loop keeps its attempt counter on a
//! fire-and-forget task's stack — a redeploy drops every pending retry with
//! no record. In cluster mode, event notifications for the push channels
//! (rest-hook, email, messaging) are instead enqueued here at attempt zero
//! and claimed by per-instance workers under the same lease + fencing-token
//! discipline as the cluster job store; the retry backoff becomes a
//! persisted `next_attempt_at` schedule instead of an in-process sleep.
//!
//! Deliberately a dedicated table, not a new [`JobKind`] on `cluster_jobs`:
//! deliveries are high-volume small rows with a *schedule* (`attempts`,
//! `next_attempt_at`) that jobs lack, they need none of the job store's
//! progress/result/cancel surface, and mixing thousands of delivery rows
//! into the claim index the SoF/reindex workers scan would degrade Phase 1
//! surfaces. What IS shared: [`WorkerId`] and the claim shape (transaction +
//! `FOR UPDATE SKIP LOCKED` + fencing bump).
//!
//! Delivery semantics are **at-least-once**: a lease that expires after a
//! successful-but-unrecorded attempt is reclaimed and redelivered. Push
//! endpoints must tolerate duplicates (the FHIR rest-hook contract already
//! requires idempotent handling by `eventNumber`).
//!
//! [`JobKind`]: crate::core::cluster_job_store::JobKind

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::error::{StorageError, StorageResult};
use crate::tenant::TenantContext;

pub use crate::core::bulk_export_worker::WorkerId;

/// Identifier for one outbox row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeliveryId(i64);

impl DeliveryId {
    /// Wraps a raw row id.
    pub fn from_i64(id: i64) -> Self {
        Self(id)
    }

    /// Returns the raw row id.
    pub fn as_i64(&self) -> i64 {
        self.0
    }
}

impl std::fmt::Display for DeliveryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Lifecycle state of an outbox row.
///
/// `queued → delivering → delivered | failed`; an expired-lease `delivering`
/// row is reclaimable (back to `delivering` under the new worker's bumped
/// fencing token), and a retryable failure releases it back to `queued`
/// with a future `next_attempt_at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryState {
    /// Waiting for a worker (possibly scheduled in the future).
    Queued,
    /// Claimed by a worker under a lease.
    Delivering,
    /// Delivered successfully.
    Delivered,
    /// Terminally failed (attempts exhausted or permanent error).
    Failed,
}

impl DeliveryState {
    /// Stable storage discriminator.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Delivering => "delivering",
            Self::Delivered => "delivered",
            Self::Failed => "failed",
        }
    }

    /// Parses the storage discriminator.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(Self::Queued),
            "delivering" => Some(Self::Delivering),
            "delivered" => Some(Self::Delivered),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// A new delivery to enqueue.
#[derive(Debug, Clone)]
pub struct NewDelivery {
    /// The subscription this notification belongs to.
    pub subscription_id: String,
    /// The cluster-wide event number minted for this notification, when the
    /// notification carries one (`event-notification`).
    pub event_number: Option<u64>,
    /// The notification type (e.g. `event-notification`); handshakes stay
    /// inline at the write path and never enter the outbox.
    pub notification_type: String,
    /// The fully built notification bundle to push.
    pub bundle: Value,
}

/// A caller-facing snapshot of one outbox row (tests, observability).
#[derive(Debug, Clone)]
pub struct DeliveryRecord {
    /// The row id.
    pub id: DeliveryId,
    /// The subscription the notification belongs to.
    pub subscription_id: String,
    /// Current lifecycle state.
    pub state: DeliveryState,
    /// Attempts made so far (claiming bumps this).
    pub attempts: u32,
    /// When the next attempt is due (for `queued` rows).
    pub next_attempt_at: DateTime<Utc>,
    /// The most recent delivery error, if any.
    pub last_error: Option<String>,
    /// The event number carried by the notification, if any.
    pub event_number: Option<u64>,
    /// The notification type.
    pub notification_type: String,
}

/// A lease over one claimed delivery, held by exactly one worker at a time.
#[derive(Debug, Clone)]
pub struct DeliveryLease {
    /// The claimed row.
    pub id: DeliveryId,
    /// The tenant the delivery belongs to.
    pub tenant: TenantContext,
    /// The worker holding the lease.
    pub worker_id: WorkerId,
    /// When the lease expires if the attempt has not concluded.
    pub lease_expiry: DateTime<Utc>,
    /// Monotonically increasing token, bumped on every claim.
    pub fencing_token: u64,
}

/// A claimed delivery: the lease plus everything needed to attempt it.
#[derive(Debug, Clone)]
pub struct ClaimedDelivery {
    /// The lease guarding this attempt.
    pub lease: DeliveryLease,
    /// The subscription the notification belongs to.
    pub subscription_id: String,
    /// The event number carried by the notification, if any.
    pub event_number: Option<u64>,
    /// The notification type.
    pub notification_type: String,
    /// The notification bundle to push.
    pub bundle: Value,
    /// Attempts made INCLUDING this one (the claim bumps the counter, so the
    /// first attempt observes 1).
    pub attempts: u32,
}

/// Error returned by fenced outbox operations.
#[derive(Debug)]
pub enum DeliveryLeaseError {
    /// The lease was lost — another worker reclaimed the row. The caller
    /// MUST stop acting on it immediately.
    LeaseLost {
        /// The row whose lease was lost.
        id: DeliveryId,
    },
    /// An underlying storage error.
    Storage(StorageError),
}

impl std::fmt::Display for DeliveryLeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LeaseLost { id } => write!(f, "delivery {id} lease lost"),
            Self::Storage(e) => write!(f, "storage error: {e}"),
        }
    }
}

impl std::error::Error for DeliveryLeaseError {}

impl From<StorageError> for DeliveryLeaseError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

/// The durable delivery outbox.
///
/// Tenancy contract: `enqueue` and `get` are tenant-scoped. `claim_next` is
/// deliberately cross-tenant (workers drain one shared queue, like the
/// cluster job store's); the claimed lease carries the row's tenant.
///
/// Fencing contract: `complete`, `release_for_retry`, and `fail` are guarded
/// by `worker_id` + `fencing_token` and require the row to still be
/// `delivering`; a guarded write affecting zero rows returns
/// [`DeliveryLeaseError::LeaseLost`].
#[async_trait]
pub trait SubscriptionDeliveryOutbox: Send + Sync {
    /// Durably enqueues a delivery (state `queued`, due immediately,
    /// `attempts` 0) and returns its id.
    async fn enqueue(
        &self,
        tenant: &TenantContext,
        delivery: NewDelivery,
    ) -> StorageResult<DeliveryId>;

    /// Atomically claims one due delivery (`queued` with `next_attempt_at`
    /// reached, or `delivering` with an expired lease), bumping the fencing
    /// token AND the attempt counter. Returns `Ok(None)` when nothing is
    /// claimable.
    async fn claim_next(
        &self,
        worker_id: &WorkerId,
        lease_duration: Duration,
    ) -> StorageResult<Option<ClaimedDelivery>>;

    /// Marks the delivery `delivered`. Fenced; requires `delivering`.
    async fn complete(&self, lease: &DeliveryLease) -> Result<(), DeliveryLeaseError>;

    /// Releases the row back to `queued` for a future attempt at
    /// `next_attempt_at`, recording the error. Fenced; requires `delivering`.
    async fn release_for_retry(
        &self,
        lease: &DeliveryLease,
        next_attempt_at: DateTime<Utc>,
        error: &str,
    ) -> Result<(), DeliveryLeaseError>;

    /// Marks the delivery terminally `failed`, recording the error. Fenced;
    /// requires `delivering`.
    async fn fail(&self, lease: &DeliveryLease, error: &str) -> Result<(), DeliveryLeaseError>;

    /// Tenant-checked read of one row; `None` when it does not exist *for
    /// this tenant*.
    async fn get(
        &self,
        tenant: &TenantContext,
        id: DeliveryId,
    ) -> StorageResult<Option<DeliveryRecord>>;

    /// Deletes terminal rows (`delivered`/`failed`) that finished before
    /// `cutoff`, returning the number removed. Deliberately cross-tenant
    /// (reaper duty); idempotent — safe for every instance to run on a
    /// timer.
    async fn delete_terminal_before(&self, cutoff: DateTime<Utc>) -> StorageResult<u64>;
}

/// Test support: an in-memory [`SubscriptionDeliveryOutbox`] implementing
/// the same state-machine and fencing contract as the database backends.
///
/// NOT a cluster-safe production backend — two instances would each have
/// their own queue (exactly the B5 failure the outbox exists to fix). It is
/// the T1 reference model.
pub mod testing {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;
    use crate::tenant::{TenantId, TenantPermissions};

    struct Row {
        tenant_id: String,
        subscription_id: String,
        event_number: Option<u64>,
        notification_type: String,
        bundle: Value,
        state: DeliveryState,
        attempts: u32,
        next_attempt_at: DateTime<Utc>,
        last_error: Option<String>,
        worker_id: Option<String>,
        lease_expiry: Option<DateTime<Utc>>,
        fencing_token: u64,
        finished_at: Option<DateTime<Utc>>,
    }

    impl Row {
        fn holds_lease(&self, lease: &DeliveryLease) -> bool {
            self.state == DeliveryState::Delivering
                && self.worker_id.as_deref() == Some(lease.worker_id.as_str())
                && self.fencing_token == lease.fencing_token
        }
    }

    /// See [module docs](self::super).
    #[derive(Default)]
    pub struct InMemoryDeliveryOutbox {
        rows: Mutex<HashMap<i64, Row>>,
        next_id: Mutex<i64>,
    }

    impl InMemoryDeliveryOutbox {
        /// Creates an empty outbox.
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl SubscriptionDeliveryOutbox for InMemoryDeliveryOutbox {
        async fn enqueue(
            &self,
            tenant: &TenantContext,
            delivery: NewDelivery,
        ) -> StorageResult<DeliveryId> {
            let mut next_id = self.next_id.lock().unwrap();
            *next_id += 1;
            let id = *next_id;
            self.rows.lock().unwrap().insert(
                id,
                Row {
                    tenant_id: tenant.tenant_id().as_str().to_string(),
                    subscription_id: delivery.subscription_id,
                    event_number: delivery.event_number,
                    notification_type: delivery.notification_type,
                    bundle: delivery.bundle,
                    state: DeliveryState::Queued,
                    attempts: 0,
                    next_attempt_at: Utc::now(),
                    last_error: None,
                    worker_id: None,
                    lease_expiry: None,
                    fencing_token: 0,
                    finished_at: None,
                },
            );
            Ok(DeliveryId::from_i64(id))
        }

        async fn claim_next(
            &self,
            worker_id: &WorkerId,
            lease_duration: Duration,
        ) -> StorageResult<Option<ClaimedDelivery>> {
            let now = Utc::now();
            let mut rows = self.rows.lock().unwrap();
            let claimable = rows
                .iter()
                .filter(|(_, row)| match row.state {
                    DeliveryState::Queued => row.next_attempt_at <= now,
                    DeliveryState::Delivering => row.lease_expiry.is_none_or(|expiry| expiry < now),
                    _ => false,
                })
                .map(|(id, row)| (*id, row.next_attempt_at))
                .min_by_key(|(id, due)| (*due, *id));

            let Some((id, _)) = claimable else {
                return Ok(None);
            };
            let row = rows.get_mut(&id).expect("row exists");
            let lease_expiry = now + chrono::Duration::from_std(lease_duration).unwrap_or_default();
            row.state = DeliveryState::Delivering;
            row.worker_id = Some(worker_id.as_str().to_string());
            row.lease_expiry = Some(lease_expiry);
            row.fencing_token += 1;
            row.attempts += 1;

            Ok(Some(ClaimedDelivery {
                lease: DeliveryLease {
                    id: DeliveryId::from_i64(id),
                    tenant: TenantContext::new(
                        TenantId::new(&row.tenant_id),
                        TenantPermissions::full_access(),
                    ),
                    worker_id: worker_id.clone(),
                    lease_expiry,
                    fencing_token: row.fencing_token,
                },
                subscription_id: row.subscription_id.clone(),
                event_number: row.event_number,
                notification_type: row.notification_type.clone(),
                bundle: row.bundle.clone(),
                attempts: row.attempts,
            }))
        }

        async fn complete(&self, lease: &DeliveryLease) -> Result<(), DeliveryLeaseError> {
            let mut rows = self.rows.lock().unwrap();
            match rows.get_mut(&lease.id.as_i64()) {
                Some(row) if row.holds_lease(lease) => {
                    row.state = DeliveryState::Delivered;
                    row.worker_id = None;
                    row.lease_expiry = None;
                    row.finished_at = Some(Utc::now());
                    Ok(())
                }
                _ => Err(DeliveryLeaseError::LeaseLost { id: lease.id }),
            }
        }

        async fn release_for_retry(
            &self,
            lease: &DeliveryLease,
            next_attempt_at: DateTime<Utc>,
            error: &str,
        ) -> Result<(), DeliveryLeaseError> {
            let mut rows = self.rows.lock().unwrap();
            match rows.get_mut(&lease.id.as_i64()) {
                Some(row) if row.holds_lease(lease) => {
                    row.state = DeliveryState::Queued;
                    row.worker_id = None;
                    row.lease_expiry = None;
                    row.next_attempt_at = next_attempt_at;
                    row.last_error = Some(error.to_string());
                    Ok(())
                }
                _ => Err(DeliveryLeaseError::LeaseLost { id: lease.id }),
            }
        }

        async fn fail(&self, lease: &DeliveryLease, error: &str) -> Result<(), DeliveryLeaseError> {
            let mut rows = self.rows.lock().unwrap();
            match rows.get_mut(&lease.id.as_i64()) {
                Some(row) if row.holds_lease(lease) => {
                    row.state = DeliveryState::Failed;
                    row.worker_id = None;
                    row.lease_expiry = None;
                    row.last_error = Some(error.to_string());
                    row.finished_at = Some(Utc::now());
                    Ok(())
                }
                _ => Err(DeliveryLeaseError::LeaseLost { id: lease.id }),
            }
        }

        async fn get(
            &self,
            tenant: &TenantContext,
            id: DeliveryId,
        ) -> StorageResult<Option<DeliveryRecord>> {
            let rows = self.rows.lock().unwrap();
            Ok(rows
                .get(&id.as_i64())
                .filter(|row| row.tenant_id == tenant.tenant_id().as_str())
                .map(|row| DeliveryRecord {
                    id,
                    subscription_id: row.subscription_id.clone(),
                    state: row.state,
                    attempts: row.attempts,
                    next_attempt_at: row.next_attempt_at,
                    last_error: row.last_error.clone(),
                    event_number: row.event_number,
                    notification_type: row.notification_type.clone(),
                }))
        }

        async fn delete_terminal_before(&self, cutoff: DateTime<Utc>) -> StorageResult<u64> {
            let mut rows = self.rows.lock().unwrap();
            let before = rows.len();
            rows.retain(|_, row| {
                !(matches!(row.state, DeliveryState::Delivered | DeliveryState::Failed)
                    && row.finished_at.is_some_and(|at| at < cutoff))
            });
            Ok((before - rows.len()) as u64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use crate::tenant::{TenantId, TenantPermissions};

    fn tenant(id: &str) -> TenantContext {
        TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
    }

    fn delivery(sub: &str) -> NewDelivery {
        NewDelivery {
            subscription_id: sub.to_string(),
            event_number: Some(1),
            notification_type: "event-notification".to_string(),
            bundle: serde_json::json!({"resourceType": "Bundle"}),
        }
    }

    fn worker(name: &str) -> WorkerId {
        WorkerId::new(name)
    }

    #[tokio::test]
    async fn claim_bumps_attempts_and_fencing_and_orders_by_due_time() {
        let outbox = InMemoryDeliveryOutbox::new();
        let t = tenant("t1");
        let first = outbox.enqueue(&t, delivery("s1")).await.unwrap();
        outbox.enqueue(&t, delivery("s2")).await.unwrap();

        let claimed = outbox
            .claim_next(&worker("w1"), Duration::from_secs(30))
            .await
            .unwrap()
            .expect("a due row must be claimable");
        assert_eq!(claimed.lease.id, first, "oldest due row claims first");
        assert_eq!(claimed.attempts, 1, "the claim IS the first attempt");
        assert_eq!(claimed.lease.fencing_token, 1);
        assert_eq!(claimed.lease.tenant.tenant_id().as_str(), "t1");
    }

    #[tokio::test]
    async fn complete_and_stale_fencing_token() {
        let outbox = InMemoryDeliveryOutbox::new();
        let t = tenant("t1");
        let id = outbox.enqueue(&t, delivery("s1")).await.unwrap();

        // Claim with a zero lease so a second worker can reclaim immediately
        // (deterministic, no sleeps).
        let stale = outbox
            .claim_next(&worker("w1"), Duration::ZERO)
            .await
            .unwrap()
            .unwrap();
        let fresh = outbox
            .claim_next(&worker("w2"), Duration::from_secs(30))
            .await
            .unwrap()
            .expect("expired lease must be reclaimable");
        assert_eq!(fresh.lease.id, id);
        assert_eq!(fresh.attempts, 2, "reclaim is a new attempt");
        assert!(fresh.lease.fencing_token > stale.lease.fencing_token);

        // The zombie's terminal write must be fenced out...
        assert!(matches!(
            outbox.complete(&stale.lease).await,
            Err(DeliveryLeaseError::LeaseLost { .. })
        ));
        // ...while the current holder's succeeds.
        outbox.complete(&fresh.lease).await.unwrap();
        let record = outbox.get(&t, id).await.unwrap().unwrap();
        assert_eq!(record.state, DeliveryState::Delivered);
    }

    #[tokio::test]
    async fn release_for_retry_schedules_and_records_the_error() {
        let outbox = InMemoryDeliveryOutbox::new();
        let t = tenant("t1");
        let id = outbox.enqueue(&t, delivery("s1")).await.unwrap();
        let claimed = outbox
            .claim_next(&worker("w1"), Duration::from_secs(30))
            .await
            .unwrap()
            .unwrap();

        let future = Utc::now() + chrono::Duration::seconds(60);
        outbox
            .release_for_retry(&claimed.lease, future, "503 from endpoint")
            .await
            .unwrap();

        let record = outbox.get(&t, id).await.unwrap().unwrap();
        assert_eq!(record.state, DeliveryState::Queued);
        assert_eq!(record.attempts, 1);
        assert_eq!(record.last_error.as_deref(), Some("503 from endpoint"));
        // Not due yet — nothing claimable.
        assert!(
            outbox
                .claim_next(&worker("w2"), Duration::from_secs(30))
                .await
                .unwrap()
                .is_none(),
            "a future next_attempt_at must not be claimable"
        );
    }

    #[tokio::test]
    async fn fail_is_terminal_and_reapable() {
        let outbox = InMemoryDeliveryOutbox::new();
        let t = tenant("t1");
        let id = outbox.enqueue(&t, delivery("s1")).await.unwrap();
        let claimed = outbox
            .claim_next(&worker("w1"), Duration::from_secs(30))
            .await
            .unwrap()
            .unwrap();
        outbox.fail(&claimed.lease, "410 Gone").await.unwrap();

        let record = outbox.get(&t, id).await.unwrap().unwrap();
        assert_eq!(record.state, DeliveryState::Failed);
        assert!(
            outbox
                .claim_next(&worker("w2"), Duration::from_secs(30))
                .await
                .unwrap()
                .is_none(),
            "terminal rows are never claimable"
        );
        let reaped = outbox
            .delete_terminal_before(Utc::now() + chrono::Duration::seconds(1))
            .await
            .unwrap();
        assert_eq!(reaped, 1);
        assert!(outbox.get(&t, id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn get_is_tenant_checked() {
        let outbox = InMemoryDeliveryOutbox::new();
        let id = outbox
            .enqueue(&tenant("tenant-a"), delivery("s1"))
            .await
            .unwrap();
        assert!(
            outbox.get(&tenant("tenant-b"), id).await.unwrap().is_none(),
            "another tenant's row must be invisible"
        );
    }
}
