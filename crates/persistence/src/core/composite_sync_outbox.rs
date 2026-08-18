//! Durable composite secondary-backend sync outbox (design doc §Class E, E1;
//! roadmap Phase 4).
//!
//! `composite::sync::SyncManager`'s async mode buffers secondary-backend
//! (e.g. Elasticsearch) propagation in an in-process `mpsc` channel: once
//! retries exhaust, or on a crash/redeploy with events still queued, the
//! propagation is silently dropped and the secondary index diverges from the
//! primary, recoverable only by a full reindex. In durable mode, `Create`/
//! `Update`/`Delete` events are instead enqueued here — one row per
//! `(event, backend_id)` pair — and drained by per-instance workers under
//! the same lease + fencing discipline as the cluster job store; a crash
//! leaves rows `applying` under an expired lease, reclaimed by any surviving
//! worker (including a fresh boot of the same instance).
//!
//! Deliberately a dedicated table, not a new [`JobKind`] on `cluster_jobs`,
//! for the same reasons [`crate::core::subscription_delivery`] gives for not
//! reusing it for B5: these rows are high-volume and small, need none of the
//! job store's progress/result/cancel surface, and would pollute the
//! SoF/reindex claim index. Nor is it reused as another
//! [`SubscriptionDeliveryOutbox`]-shaped row: that table's `notification_type`
//! + `bundle` columns are push-channel-shaped, not resource-sync-shaped.
//! What IS shared: [`WorkerId`] and the claim shape (transaction + `FOR
//! UPDATE SKIP LOCKED` + fencing bump), cloned from
//! `backends/postgres/subscription_outbox.rs`.
//!
//! `SyncEvent::BulkSync` is deliberately out of scope — a multi-resource
//! batch doesn't denormalize cleanly to the one-row-per-resource shape below
//! without changing its current all-or-nothing semantics, and bulk resync
//! is already driven through the (already cluster-safe) reindex job store.
//! `SyncManager` keeps `BulkSync` on the pre-existing in-memory channel path
//! unconditionally.
//!
//! Wiring is capability-based, not `HFS_CLUSTER`-gated: whenever the primary
//! backend can back this seam (`composite_sync_outbox()` returns `Some`,
//! i.e. a Postgres primary), `CompositeStorage` wires it unconditionally —
//! durable delivery is strictly better than the in-memory channel even for a
//! single instance (crash-durability, not just cluster-correctness), the
//! same reasoning F5's unconditional CAS fix used. Non-Postgres primaries
//! keep today's in-memory-channel behavior, cluster or not.
//!
//! Semantics are **at-least-once**: a lease that expires after a
//! successful-but-unrecorded apply is reclaimed and re-applied.
//! `create_or_update`-shaped applies are already idempotent (see
//! `sync_event_to_backend`), so a duplicate apply is harmless.
//!
//! [`JobKind`]: crate::core::cluster_job_store::JobKind
//! [`SubscriptionDeliveryOutbox`]: crate::core::subscription_delivery::SubscriptionDeliveryOutbox

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::error::{StorageError, StorageResult};
use crate::tenant::TenantContext;

pub use crate::core::bulk_export_worker::WorkerId;

/// Identifier for one outbox row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SyncOutboxId(i64);

impl SyncOutboxId {
    /// Wraps a raw row id.
    pub fn from_i64(id: i64) -> Self {
        Self(id)
    }

    /// Returns the raw row id.
    pub fn as_i64(&self) -> i64 {
        self.0
    }
}

impl std::fmt::Display for SyncOutboxId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The write operation a row carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOperation {
    /// Resource was created.
    Create,
    /// Resource was updated.
    Update,
    /// Resource was deleted.
    Delete,
}

impl SyncOperation {
    /// Stable storage discriminator.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }

    /// Parses the storage discriminator.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "create" => Some(Self::Create),
            "update" => Some(Self::Update),
            "delete" => Some(Self::Delete),
            _ => None,
        }
    }
}

/// Lifecycle state of an outbox row.
///
/// `queued → applying → applied | failed`; an expired-lease `applying` row
/// is reclaimable (back to `applying` under the new worker's bumped fencing
/// token), and a retryable failure releases it back to `queued` with a
/// future `next_attempt_at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOutboxState {
    /// Waiting for a worker (possibly scheduled in the future).
    Queued,
    /// Claimed by a worker under a lease.
    Applying,
    /// Applied to the target backend successfully.
    Applied,
    /// Terminally failed (attempts exhausted or permanent error).
    Failed,
}

impl SyncOutboxState {
    /// Stable storage discriminator.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Applying => "applying",
            Self::Applied => "applied",
            Self::Failed => "failed",
        }
    }

    /// Parses the storage discriminator.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(Self::Queued),
            "applying" => Some(Self::Applying),
            "applied" => Some(Self::Applied),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// A new sync entry to enqueue, targeting one secondary backend.
#[derive(Debug, Clone)]
pub struct NewSyncOutboxEntry {
    /// Which secondary backend this row targets.
    pub backend_id: String,
    /// The write operation to replay.
    pub operation: SyncOperation,
    /// The FHIR resource type.
    pub resource_type: String,
    /// The FHIR resource id.
    pub resource_id: String,
    /// The resource content (`None` for `Delete`).
    pub content: Option<Value>,
    /// The resource version, when the originating event carried one
    /// (`Update`).
    pub version: Option<String>,
    /// The tenant that owns this resource.
    pub tenant_id: String,
    /// FHIR version, serialized (e.g. `"R4"`). `None` for `Delete`, which
    /// carries no version — `ResourceStorage::delete` needs none.
    pub fhir_version: Option<String>,
}

/// A caller-facing snapshot of one outbox row (tests, observability).
#[derive(Debug, Clone)]
pub struct SyncOutboxRecord {
    /// The row id.
    pub id: SyncOutboxId,
    /// Which secondary backend this row targets.
    pub backend_id: String,
    /// Current lifecycle state.
    pub state: SyncOutboxState,
    /// Attempts made so far (claiming bumps this).
    pub attempts: u32,
    /// When the next attempt is due (for `queued` rows).
    pub next_attempt_at: DateTime<Utc>,
    /// The most recent apply error, if any.
    pub last_error: Option<String>,
}

/// A lease over one claimed row, held by exactly one worker at a time.
#[derive(Debug, Clone)]
pub struct SyncLease {
    /// The claimed row.
    pub id: SyncOutboxId,
    /// The tenant the entry belongs to.
    pub tenant: TenantContext,
    /// The worker holding the lease.
    pub worker_id: WorkerId,
    /// When the lease expires if the attempt has not concluded.
    pub lease_expiry: DateTime<Utc>,
    /// Monotonically increasing token, bumped on every claim.
    pub fencing_token: u64,
}

/// A claimed row: the lease plus everything needed to apply it.
#[derive(Debug, Clone)]
pub struct ClaimedSyncEvent {
    /// The lease guarding this attempt.
    pub lease: SyncLease,
    /// Which secondary backend this row targets.
    pub backend_id: String,
    /// The write operation to replay.
    pub operation: SyncOperation,
    /// The FHIR resource type.
    pub resource_type: String,
    /// The FHIR resource id.
    pub resource_id: String,
    /// The resource content (`None` for `Delete`).
    pub content: Option<Value>,
    /// The resource version, when carried.
    pub version: Option<String>,
    /// FHIR version, serialized. `None` for `Delete`.
    pub fhir_version: Option<String>,
    /// Attempts made INCLUDING this one (the claim bumps the counter, so the
    /// first attempt observes 1).
    pub attempts: u32,
}

/// Error returned by fenced outbox operations.
#[derive(Debug)]
pub enum SyncLeaseError {
    /// The lease was lost — another worker reclaimed the row. The caller
    /// MUST stop acting on it immediately.
    LeaseLost {
        /// The row whose lease was lost.
        id: SyncOutboxId,
    },
    /// An underlying storage error.
    Storage(StorageError),
}

impl std::fmt::Display for SyncLeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LeaseLost { id } => write!(f, "composite sync entry {id} lease lost"),
            Self::Storage(e) => write!(f, "storage error: {e}"),
        }
    }
}

impl std::error::Error for SyncLeaseError {}

impl From<StorageError> for SyncLeaseError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

/// The durable composite sync outbox.
///
/// Tenancy contract: `enqueue` and `get` are tenant-scoped. `claim_next` is
/// deliberately cross-tenant (workers drain one shared queue, like the
/// cluster job store's and the subscription delivery outbox's); the claimed
/// lease carries the row's tenant.
///
/// Fencing contract: `complete`, `release_for_retry`, and `fail` are guarded
/// by `worker_id` + `fencing_token` and require the row to still be
/// `applying`; a guarded write affecting zero rows returns
/// [`SyncLeaseError::LeaseLost`].
#[async_trait]
pub trait CompositeSyncOutbox: Send + Sync {
    /// Durably enqueues one entry (state `queued`, due immediately,
    /// `attempts` 0) and returns its id.
    async fn enqueue(
        &self,
        tenant: &TenantContext,
        entry: NewSyncOutboxEntry,
    ) -> StorageResult<SyncOutboxId>;

    /// Atomically claims one due entry (`queued` with `next_attempt_at`
    /// reached, or `applying` with an expired lease), bumping the fencing
    /// token AND the attempt counter. Returns `Ok(None)` when nothing is
    /// claimable.
    async fn claim_next(
        &self,
        worker_id: &WorkerId,
        lease_duration: Duration,
    ) -> StorageResult<Option<ClaimedSyncEvent>>;

    /// Marks the entry `applied`. Fenced; requires `applying`.
    async fn complete(&self, lease: &SyncLease) -> Result<(), SyncLeaseError>;

    /// Releases the row back to `queued` for a future attempt at
    /// `next_attempt_at`, recording the error. Fenced; requires `applying`.
    async fn release_for_retry(
        &self,
        lease: &SyncLease,
        next_attempt_at: DateTime<Utc>,
        error: &str,
    ) -> Result<(), SyncLeaseError>;

    /// Marks the entry terminally `failed`, recording the error. Fenced;
    /// requires `applying`.
    async fn fail(&self, lease: &SyncLease, error: &str) -> Result<(), SyncLeaseError>;

    /// Tenant-checked read of one row; `None` when it does not exist *for
    /// this tenant*.
    async fn get(
        &self,
        tenant: &TenantContext,
        id: SyncOutboxId,
    ) -> StorageResult<Option<SyncOutboxRecord>>;

    /// Deletes terminal rows (`applied`/`failed`) that finished before
    /// `cutoff`, returning the number removed. Deliberately cross-tenant
    /// (reaper duty); idempotent — safe for every instance to run on a
    /// timer.
    async fn delete_terminal_before(&self, cutoff: DateTime<Utc>) -> StorageResult<u64>;
}

/// Test support: an in-memory [`CompositeSyncOutbox`] implementing the same
/// state-machine and fencing contract as the database backends.
///
/// NOT a cluster-safe production backend — two instances would each have
/// their own queue (exactly the E1 failure the outbox exists to fix). It is
/// the T1 reference model.
pub mod testing {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;
    use crate::tenant::{TenantId, TenantPermissions};

    struct Row {
        tenant_id: String,
        backend_id: String,
        operation: SyncOperation,
        resource_type: String,
        resource_id: String,
        content: Option<Value>,
        version: Option<String>,
        fhir_version: Option<String>,
        state: SyncOutboxState,
        attempts: u32,
        next_attempt_at: DateTime<Utc>,
        last_error: Option<String>,
        worker_id: Option<String>,
        lease_expiry: Option<DateTime<Utc>>,
        fencing_token: u64,
        finished_at: Option<DateTime<Utc>>,
    }

    impl Row {
        fn holds_lease(&self, lease: &SyncLease) -> bool {
            self.state == SyncOutboxState::Applying
                && self.worker_id.as_deref() == Some(lease.worker_id.as_str())
                && self.fencing_token == lease.fencing_token
        }
    }

    /// See [module docs](self::super).
    #[derive(Default)]
    pub struct InMemorySyncOutbox {
        rows: Mutex<HashMap<i64, Row>>,
        next_id: Mutex<i64>,
    }

    impl InMemorySyncOutbox {
        /// Creates an empty outbox.
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl CompositeSyncOutbox for InMemorySyncOutbox {
        async fn enqueue(
            &self,
            tenant: &TenantContext,
            entry: NewSyncOutboxEntry,
        ) -> StorageResult<SyncOutboxId> {
            let mut next_id = self.next_id.lock().unwrap();
            *next_id += 1;
            let id = *next_id;
            self.rows.lock().unwrap().insert(
                id,
                Row {
                    tenant_id: tenant.tenant_id().as_str().to_string(),
                    backend_id: entry.backend_id,
                    operation: entry.operation,
                    resource_type: entry.resource_type,
                    resource_id: entry.resource_id,
                    content: entry.content,
                    version: entry.version,
                    fhir_version: entry.fhir_version,
                    state: SyncOutboxState::Queued,
                    attempts: 0,
                    next_attempt_at: Utc::now(),
                    last_error: None,
                    worker_id: None,
                    lease_expiry: None,
                    fencing_token: 0,
                    finished_at: None,
                },
            );
            Ok(SyncOutboxId::from_i64(id))
        }

        async fn claim_next(
            &self,
            worker_id: &WorkerId,
            lease_duration: Duration,
        ) -> StorageResult<Option<ClaimedSyncEvent>> {
            let now = Utc::now();
            let mut rows = self.rows.lock().unwrap();
            let claimable = rows
                .iter()
                .filter(|(_, row)| match row.state {
                    SyncOutboxState::Queued => row.next_attempt_at <= now,
                    SyncOutboxState::Applying => row.lease_expiry.is_none_or(|expiry| expiry < now),
                    _ => false,
                })
                .map(|(id, row)| (*id, row.next_attempt_at))
                .min_by_key(|(id, due)| (*due, *id));

            let Some((id, _)) = claimable else {
                return Ok(None);
            };
            let row = rows.get_mut(&id).expect("row exists");
            let lease_expiry = now + chrono::Duration::from_std(lease_duration).unwrap_or_default();
            row.state = SyncOutboxState::Applying;
            row.worker_id = Some(worker_id.as_str().to_string());
            row.lease_expiry = Some(lease_expiry);
            row.fencing_token += 1;
            row.attempts += 1;

            Ok(Some(ClaimedSyncEvent {
                lease: SyncLease {
                    id: SyncOutboxId::from_i64(id),
                    tenant: TenantContext::new(
                        TenantId::new(&row.tenant_id),
                        TenantPermissions::full_access(),
                    ),
                    worker_id: worker_id.clone(),
                    lease_expiry,
                    fencing_token: row.fencing_token,
                },
                backend_id: row.backend_id.clone(),
                operation: row.operation,
                resource_type: row.resource_type.clone(),
                resource_id: row.resource_id.clone(),
                content: row.content.clone(),
                version: row.version.clone(),
                fhir_version: row.fhir_version.clone(),
                attempts: row.attempts,
            }))
        }

        async fn complete(&self, lease: &SyncLease) -> Result<(), SyncLeaseError> {
            let mut rows = self.rows.lock().unwrap();
            match rows.get_mut(&lease.id.as_i64()) {
                Some(row) if row.holds_lease(lease) => {
                    row.state = SyncOutboxState::Applied;
                    row.worker_id = None;
                    row.lease_expiry = None;
                    row.finished_at = Some(Utc::now());
                    Ok(())
                }
                _ => Err(SyncLeaseError::LeaseLost { id: lease.id }),
            }
        }

        async fn release_for_retry(
            &self,
            lease: &SyncLease,
            next_attempt_at: DateTime<Utc>,
            error: &str,
        ) -> Result<(), SyncLeaseError> {
            let mut rows = self.rows.lock().unwrap();
            match rows.get_mut(&lease.id.as_i64()) {
                Some(row) if row.holds_lease(lease) => {
                    row.state = SyncOutboxState::Queued;
                    row.worker_id = None;
                    row.lease_expiry = None;
                    row.next_attempt_at = next_attempt_at;
                    row.last_error = Some(error.to_string());
                    Ok(())
                }
                _ => Err(SyncLeaseError::LeaseLost { id: lease.id }),
            }
        }

        async fn fail(&self, lease: &SyncLease, error: &str) -> Result<(), SyncLeaseError> {
            let mut rows = self.rows.lock().unwrap();
            match rows.get_mut(&lease.id.as_i64()) {
                Some(row) if row.holds_lease(lease) => {
                    row.state = SyncOutboxState::Failed;
                    row.worker_id = None;
                    row.lease_expiry = None;
                    row.last_error = Some(error.to_string());
                    row.finished_at = Some(Utc::now());
                    Ok(())
                }
                _ => Err(SyncLeaseError::LeaseLost { id: lease.id }),
            }
        }

        async fn get(
            &self,
            tenant: &TenantContext,
            id: SyncOutboxId,
        ) -> StorageResult<Option<SyncOutboxRecord>> {
            let rows = self.rows.lock().unwrap();
            Ok(rows
                .get(&id.as_i64())
                .filter(|row| row.tenant_id == tenant.tenant_id().as_str())
                .map(|row| SyncOutboxRecord {
                    id,
                    backend_id: row.backend_id.clone(),
                    state: row.state,
                    attempts: row.attempts,
                    next_attempt_at: row.next_attempt_at,
                    last_error: row.last_error.clone(),
                }))
        }

        async fn delete_terminal_before(&self, cutoff: DateTime<Utc>) -> StorageResult<u64> {
            let mut rows = self.rows.lock().unwrap();
            let before = rows.len();
            rows.retain(|_, row| {
                !(matches!(
                    row.state,
                    SyncOutboxState::Applied | SyncOutboxState::Failed
                ) && row.finished_at.is_some_and(|at| at < cutoff))
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

    fn entry(backend_id: &str) -> NewSyncOutboxEntry {
        NewSyncOutboxEntry {
            backend_id: backend_id.to_string(),
            operation: SyncOperation::Create,
            resource_type: "Patient".to_string(),
            resource_id: "p1".to_string(),
            content: Some(serde_json::json!({"resourceType": "Patient", "id": "p1"})),
            version: None,
            tenant_id: "t1".to_string(),
            fhir_version: Some("R4".to_string()),
        }
    }

    fn worker(name: &str) -> WorkerId {
        WorkerId::new(name)
    }

    #[tokio::test]
    async fn claim_bumps_attempts_and_fencing_and_orders_by_due_time() {
        let outbox = InMemorySyncOutbox::new();
        let t = tenant("t1");
        let first = outbox.enqueue(&t, entry("es")).await.unwrap();
        outbox.enqueue(&t, entry("es")).await.unwrap();

        let claimed = outbox
            .claim_next(&worker("w1"), Duration::from_secs(30))
            .await
            .unwrap()
            .expect("a due row must be claimable");
        assert_eq!(claimed.lease.id, first, "oldest due row claims first");
        assert_eq!(claimed.attempts, 1, "the claim IS the first attempt");
        assert_eq!(claimed.lease.fencing_token, 1);
        assert_eq!(claimed.lease.tenant.tenant_id().as_str(), "t1");
        assert_eq!(claimed.backend_id, "es");
        assert!(matches!(claimed.operation, SyncOperation::Create));
    }

    #[tokio::test]
    async fn complete_and_stale_fencing_token() {
        let outbox = InMemorySyncOutbox::new();
        let t = tenant("t1");
        let id = outbox.enqueue(&t, entry("es")).await.unwrap();

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
            Err(SyncLeaseError::LeaseLost { .. })
        ));
        // ...while the current holder's succeeds.
        outbox.complete(&fresh.lease).await.unwrap();
        let record = outbox.get(&t, id).await.unwrap().unwrap();
        assert_eq!(record.state, SyncOutboxState::Applied);
    }

    #[tokio::test]
    async fn release_for_retry_schedules_and_records_the_error() {
        let outbox = InMemorySyncOutbox::new();
        let t = tenant("t1");
        let id = outbox.enqueue(&t, entry("es")).await.unwrap();
        let claimed = outbox
            .claim_next(&worker("w1"), Duration::from_secs(30))
            .await
            .unwrap()
            .unwrap();

        let future = Utc::now() + chrono::Duration::seconds(60);
        outbox
            .release_for_retry(&claimed.lease, future, "connection refused")
            .await
            .unwrap();

        let record = outbox.get(&t, id).await.unwrap().unwrap();
        assert_eq!(record.state, SyncOutboxState::Queued);
        assert_eq!(record.attempts, 1);
        assert_eq!(record.last_error.as_deref(), Some("connection refused"));
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
        let outbox = InMemorySyncOutbox::new();
        let t = tenant("t1");
        let id = outbox.enqueue(&t, entry("es")).await.unwrap();
        let claimed = outbox
            .claim_next(&worker("w1"), Duration::from_secs(30))
            .await
            .unwrap()
            .unwrap();
        outbox.fail(&claimed.lease, "index rejected").await.unwrap();

        let record = outbox.get(&t, id).await.unwrap().unwrap();
        assert_eq!(record.state, SyncOutboxState::Failed);
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
        let outbox = InMemorySyncOutbox::new();
        let id = outbox
            .enqueue(&tenant("tenant-a"), entry("es"))
            .await
            .unwrap();
        assert!(
            outbox.get(&tenant("tenant-b"), id).await.unwrap().is_none(),
            "another tenant's row must be invisible"
        );
    }
}
