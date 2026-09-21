//! Bookkeeping for secondary syncs that failed for good (#1334).
//!
//! A composite write succeeds once the primary has committed it, in every
//! [`SyncMode`](super::config::SyncMode): the primary is the system of record.
//! A secondary that then rejects the change, after [`SyncManager`]'s retries,
//! leaves the resource missing from (or stale in) every search that secondary
//! serves. This module is what makes that visible and repairable instead of a
//! free-text log line:
//!
//! 1. a **counter** and a **gauge**, through [`SecondarySyncObserver`] (the
//!    persistence crate has no metrics dependency; the server plugs its
//!    exporter in),
//! 2. one **structured event** per final failure, carrying the tenant,
//!    resource type, id, version, backend and operation — never resource
//!    content,
//! 3. a **durable "needs reindex" record** in a [`SecondarySyncFailureLedger`]
//!    kept by the primary backend, so the affected resources are still known
//!    after a restart and
//!    [`CompositeStorage::repair_secondary_sync_failures`] can re-sync them.
//!
//! A record is a *hint*, not a queued write: the repair always pushes the
//! primary's **current** state, so a stale or duplicate record costs one
//! redundant idempotent write and nothing else.
//!
//! [`SyncManager`]: super::sync::SyncManager
//! [`CompositeStorage::repair_secondary_sync_failures`]:
//!     super::storage::CompositeStorage::repair_secondary_sync_failures

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::{Mutex, RwLock};
use tracing::{error, warn};

use crate::error::{BackendError, ResourceError, StorageError, StorageResult};
use crate::tenant::{TenantContext, TenantId, TenantPermissions};
use crate::types::StoredResource;

use super::storage::{CompositeStorage, DynStorage};
use super::sync::SyncEvent;

/// Longest error text kept on a record. Backend errors can be long (an
/// Elasticsearch rejection nests its causes); the record needs enough to
/// triage, not the whole response.
pub const MAX_RECORDED_ERROR_CHARS: usize = 1024;

/// Most outstanding keys a process keeps in memory to decide whether a
/// successful sync has a record to clear. Past it, every successful sync
/// issues the (indexed, usually no-op) delete instead.
const MAX_TRACKED_KEYS: usize = 50_000;

/// The kind of change a secondary failed to take. The fixed values of the
/// `operation` metric label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyncOperation {
    /// A new resource.
    Create,
    /// A new version of an existing resource.
    Update,
    /// A delete.
    Delete,
}

impl SyncOperation {
    /// The label / stored value.
    pub fn as_str(self) -> &'static str {
        match self {
            SyncOperation::Create => "create",
            SyncOperation::Update => "update",
            SyncOperation::Delete => "delete",
        }
    }

    /// Reads a stored value back. Anything unrecognised is an `Update`: the
    /// repair does not branch on the operation, so the safe reading is the
    /// generic one rather than an error that would wedge the ledger.
    pub fn from_stored(value: &str) -> Self {
        match value {
            "create" => SyncOperation::Create,
            "delete" => SyncOperation::Delete,
            _ => SyncOperation::Update,
        }
    }
}

/// What a record is unique on: one resource on one secondary.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SyncFailureKey {
    /// Tenant the resource belongs to.
    pub tenant_id: String,
    /// Resource type.
    pub resource_type: String,
    /// Resource id.
    pub resource_id: String,
    /// The secondary that missed the change.
    pub backend_id: String,
}

/// One final sync failure, as handed to the ledger.
#[derive(Debug, Clone)]
pub struct SyncFailureReport {
    /// Which resource on which secondary.
    pub key: SyncFailureKey,
    /// The change that failed.
    pub operation: SyncOperation,
    /// Attempts this failure took (the first try plus its retries).
    pub attempts: u32,
    /// The last error, truncated to [`MAX_RECORDED_ERROR_CHARS`].
    pub error: String,
    /// When it failed.
    pub failed_at: DateTime<Utc>,
}

/// A durable "needs reindex" record.
#[derive(Debug, Clone)]
pub struct SecondarySyncFailure {
    /// Which resource on which secondary.
    pub key: SyncFailureKey,
    /// The most recent change that failed.
    pub operation: SyncOperation,
    /// When the resource first fell out of sync.
    pub first_failed_at: DateTime<Utc>,
    /// When it most recently failed (a write, or a repair attempt).
    pub last_failed_at: DateTime<Utc>,
    /// The most recent error.
    pub last_error: String,
    /// Attempts made in total, across every failure folded into this record.
    pub attempts: u64,
}

/// Where "needs reindex" records live so they survive a restart.
///
/// Implemented by the primary backends that can keep a small table of their
/// own (SQLite, PostgreSQL, MongoDB). A composite whose primary has no ledger
/// (S3) still counts and logs every failure; it just cannot list them later.
#[async_trait]
pub trait SecondarySyncFailureLedger: Send + Sync {
    /// Records a failure. One record per [`SyncFailureKey`]: a repeat keeps
    /// `first_failed_at`, adds to `attempts`, and replaces the operation, the
    /// error and `last_failed_at`. Returns whether a **new** record was made.
    async fn record_sync_failure(&self, report: &SyncFailureReport) -> StorageResult<bool>;

    /// Drops the record, if any. Returns whether there was one.
    async fn clear_sync_failure(&self, key: &SyncFailureKey) -> StorageResult<bool>;

    /// Up to `limit` records across all tenants, least recently failed first,
    /// so a record that keeps failing moves to the back instead of starving
    /// the rest of a bounded batch.
    async fn list_sync_failures(&self, limit: usize) -> StorageResult<Vec<SecondarySyncFailure>>;

    /// How many records are outstanding.
    async fn count_sync_failures(&self) -> StorageResult<u64>;
}

/// Receives the countable side of a failure; the server forwards it to its
/// metrics exporter. No tenant, resource type or id reaches this trait: the
/// metrics endpoint is public and those would be unbounded label values.
pub trait SecondarySyncObserver: Send + Sync {
    /// One final failure (after retries) of `operation` on `backend_id`.
    fn sync_failed(&self, backend_id: &str, operation: SyncOperation);

    /// The number of outstanding "needs reindex" records changed.
    fn needs_reindex(&self, outstanding: u64);
}

/// The error as text, including the detail `BackendError::Unavailable`'s
/// display leaves out — which is exactly what an exhausted Elasticsearch write
/// reports (#1382), so without this every such record would read
/// "backend unavailable: elasticsearch" and nothing else.
pub(crate) fn error_detail(error: &StorageError) -> String {
    match error {
        StorageError::Backend(BackendError::Unavailable { message, .. }) => {
            format!("{error}: {message}")
        }
        other => other.to_string(),
    }
}

/// Truncates an error for the ledger, on a character boundary.
pub(crate) fn truncate_error(error: &str) -> String {
    match error.char_indices().nth(MAX_RECORDED_ERROR_CHARS) {
        Some((cut, _)) => format!("{}…", &error[..cut]),
        None => error.to_string(),
    }
}

/// The resources an event names, with the operation and (when the event
/// knows it) the version each one was at.
fn event_subjects(event: &SyncEvent) -> Vec<(String, String, SyncOperation, Option<String>)> {
    match event {
        SyncEvent::Create {
            resource_type,
            resource_id,
            content,
            ..
        } => vec![(
            resource_type.clone(),
            resource_id.clone(),
            SyncOperation::Create,
            // A stored body need not carry `meta`; a create is version 1
            // unless the content says otherwise.
            Some(
                content
                    .pointer("/meta/versionId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("1")
                    .to_string(),
            ),
        )],
        SyncEvent::Update {
            resource_type,
            resource_id,
            version,
            ..
        } => vec![(
            resource_type.clone(),
            resource_id.clone(),
            SyncOperation::Update,
            Some(version.clone()),
        )],
        SyncEvent::Delete {
            resource_type,
            resource_id,
            ..
        } => vec![(
            resource_type.clone(),
            resource_id.clone(),
            SyncOperation::Delete,
            None,
        )],
        SyncEvent::BulkSync { resources, .. } => resources
            .iter()
            .map(|resource| {
                (
                    resource.resource_type().to_string(),
                    resource.id().to_string(),
                    SyncOperation::Update,
                    Some(resource.version_id().to_string()),
                )
            })
            .collect(),
    }
}

/// Turns sync outcomes into the metric, the event and the ledger record.
///
/// Owned by the [`SyncManager`](super::sync::SyncManager), because that is
/// the only place every outcome passes through: the asynchronous worker
/// finishes an event long after the write that queued it has returned.
#[derive(Default)]
pub struct SyncFailureRecorder {
    ledger: RwLock<Option<Arc<dyn SecondarySyncFailureLedger>>>,
    observer: RwLock<Option<Arc<dyn SecondarySyncObserver>>>,
    /// Outstanding records, as far as this process knows.
    outstanding: AtomicU64,
    /// Keys this process knows to be outstanding; see [`MAX_TRACKED_KEYS`].
    tracked: Mutex<HashSet<SyncFailureKey>>,
    /// `tracked` does not cover the ledger (too many records, or the ledger
    /// could not be read): clear on every success rather than trust it.
    untracked: AtomicBool,
    hydrated: tokio::sync::OnceCell<()>,
}

impl SyncFailureRecorder {
    /// Sets the durable ledger.
    pub(crate) fn set_ledger(&self, ledger: Arc<dyn SecondarySyncFailureLedger>) {
        *self.ledger.write() = Some(ledger);
    }

    /// Sets the metrics observer.
    pub(crate) fn set_observer(&self, observer: Arc<dyn SecondarySyncObserver>) {
        *self.observer.write() = Some(observer);
    }

    pub(crate) fn ledger(&self) -> Option<Arc<dyn SecondarySyncFailureLedger>> {
        self.ledger.read().clone()
    }

    /// The record for `key` is gone from the ledger; stop tracking it.
    pub(crate) fn forget(&self, key: &SyncFailureKey) {
        self.tracked.lock().remove(key);
    }

    /// Outstanding records, as far as this process knows.
    pub fn outstanding(&self) -> u64 {
        self.outstanding.load(Ordering::Relaxed)
    }

    fn publish_outstanding(&self) {
        if let Some(observer) = self.observer.read().clone() {
            observer.needs_reindex(self.outstanding());
        }
    }

    /// Replaces the outstanding count with the ledger's own (another process
    /// may have added or repaired records).
    pub(crate) fn set_outstanding(&self, outstanding: u64) {
        self.outstanding.store(outstanding, Ordering::Relaxed);
        self.publish_outstanding();
    }

    /// Loads what an earlier run left behind, once. Runs on the first outcome
    /// rather than at construction so building a composite stays synchronous.
    async fn hydrate(&self) {
        self.hydrated
            .get_or_init(|| async {
                let Some(ledger) = self.ledger() else {
                    return;
                };
                match ledger.list_sync_failures(MAX_TRACKED_KEYS + 1).await {
                    Ok(records) => {
                        if records.len() > MAX_TRACKED_KEYS {
                            self.untracked.store(true, Ordering::Relaxed);
                        }
                        let mut tracked = self.tracked.lock();
                        tracked.extend(
                            records
                                .into_iter()
                                .take(MAX_TRACKED_KEYS)
                                .map(|record| record.key),
                        );
                    }
                    Err(e) => {
                        self.untracked.store(true, Ordering::Relaxed);
                        warn!(
                            error = %e,
                            "Could not read the secondary sync failure ledger; \
                             every successful sync will try to clear its record"
                        );
                    }
                }
                match ledger.count_sync_failures().await {
                    Ok(outstanding) => self.set_outstanding(outstanding),
                    Err(e) => warn!(
                        error = %e,
                        "Could not count the secondary sync failure ledger"
                    ),
                }
            })
            .await;
    }

    /// A sync of `event` to `backend_id` failed for good.
    pub(crate) async fn sync_failed(
        &self,
        event: &SyncEvent,
        backend_id: &str,
        error: &StorageError,
        attempts: u32,
    ) {
        let tenant_id = event.tenant_id().as_str().to_string();
        for (resource_type, resource_id, operation, version) in event_subjects(event) {
            self.resource_sync_failed(
                SyncFailureKey {
                    tenant_id: tenant_id.clone(),
                    resource_type,
                    resource_id,
                    backend_id: backend_id.to_string(),
                },
                operation,
                version.as_deref(),
                error,
                attempts,
            )
            .await;
        }
    }

    /// One resource's sync failed for good: count it, say so, record it.
    pub(crate) async fn resource_sync_failed(
        &self,
        key: SyncFailureKey,
        operation: SyncOperation,
        version: Option<&str>,
        error: &StorageError,
        attempts: u32,
    ) {
        self.hydrate().await;
        let error = error_detail(error);
        let ledger = self.ledger();
        if let Some(observer) = self.observer.read().clone() {
            observer.sync_failed(&key.backend_id, operation);
        }
        error!(
            tenant = %key.tenant_id,
            resource_type = %key.resource_type,
            id = %key.resource_id,
            version = version.unwrap_or(""),
            backend_id = %key.backend_id,
            operation = operation.as_str(),
            attempts,
            recorded = ledger.is_some(),
            error = %error,
            "Secondary sync failed; the primary holds the write and the secondary needs a reindex of this resource"
        );

        let Some(ledger) = ledger else {
            return;
        };
        let report = SyncFailureReport {
            key: key.clone(),
            operation,
            attempts,
            error: truncate_error(&error),
            failed_at: Utc::now(),
        };
        match ledger.record_sync_failure(&report).await {
            Ok(inserted) => {
                {
                    let mut tracked = self.tracked.lock();
                    if tracked.len() < MAX_TRACKED_KEYS {
                        tracked.insert(key);
                    } else if !tracked.contains(&key) {
                        self.untracked.store(true, Ordering::Relaxed);
                    }
                }
                if inserted {
                    self.outstanding.fetch_add(1, Ordering::Relaxed);
                    self.publish_outstanding();
                }
            }
            Err(e) => error!(
                tenant = %key.tenant_id,
                resource_type = %key.resource_type,
                id = %key.resource_id,
                backend_id = %key.backend_id,
                error = %e,
                "Could not record the secondary sync failure; only this log line and the metric say the resource needs a reindex"
            ),
        }
    }

    /// A sync of `event` to `backend_id` succeeded: whatever was owed for
    /// these resources on that backend no longer is.
    pub(crate) async fn sync_succeeded(&self, event: &SyncEvent, backend_id: &str) {
        let tenant_id = event.tenant_id().as_str();
        for (resource_type, resource_id, _, _) in event_subjects(event) {
            self.resource_sync_succeeded(SyncFailureKey {
                tenant_id: tenant_id.to_string(),
                resource_type,
                resource_id,
                backend_id: backend_id.to_string(),
            })
            .await;
        }
    }

    /// One resource's sync succeeded. Free unless something is outstanding:
    /// the healthy path never touches the ledger.
    pub(crate) async fn resource_sync_succeeded(&self, key: SyncFailureKey) {
        self.hydrate().await;
        let Some(ledger) = self.ledger() else {
            return;
        };
        let known = self.tracked.lock().remove(&key);
        if !known && !self.untracked.load(Ordering::Relaxed) {
            return;
        }
        match ledger.clear_sync_failure(&key).await {
            Ok(true) => {
                // Saturating: another process may have counted this record.
                let _ = self
                    .outstanding
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                        Some(n.saturating_sub(1))
                    });
                self.publish_outstanding();
            }
            Ok(false) => {}
            Err(e) => {
                // Still owed as far as the ledger says; the repair pass will
                // find it in sync and clear it.
                self.tracked.lock().insert(key.clone());
                warn!(
                    tenant = %key.tenant_id,
                    resource_type = %key.resource_type,
                    id = %key.resource_id,
                    backend_id = %key.backend_id,
                    error = %e,
                    "Resource synced, but its needs-reindex record could not be cleared"
                );
            }
        }
    }
}

/// What one [`repair_secondary_sync_failures`] pass did.
///
/// [`repair_secondary_sync_failures`]:
///     super::storage::CompositeStorage::repair_secondary_sync_failures
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncRepairReport {
    /// Records looked at (at most the requested batch).
    pub examined: usize,
    /// Records whose resource was re-synced and whose record was cleared.
    pub repaired: usize,
    /// Records whose secondary still refused; they stay for the next pass.
    pub still_failing: usize,
    /// Records dropped because they name a backend this composite does not
    /// have (a secondary removed from the configuration).
    pub dropped: usize,
    /// Records outstanding after the pass.
    pub remaining: u64,
}

/// How often one repair re-reads the primary after pushing, when a write
/// slipped in between. Past it the record stays for the next pass.
const REPAIR_MAX_ROUNDS: usize = 3;

impl CompositeStorage {
    /// Works through up to `limit` "needs reindex" records: pushes the
    /// primary's **current** state of each recorded resource to the secondary
    /// that missed it — a delete when the primary no longer has it — and
    /// clears the record (#1334).
    ///
    /// * **Idempotent.** The push is `create_or_update` / `delete`, and a
    ///   secondary that already lacks a deleted resource counts as done.
    /// * **Bounded.** One attempt per record per pass, no retry loop: a
    ///   secondary that is still down costs `limit` failed calls, and the
    ///   records, re-stamped, go to the back of the queue.
    /// * **Safe alongside writes.** After pushing, the primary is read again;
    ///   if a write changed the resource meanwhile, the newer state is pushed
    ///   too, so the repair cannot leave behind an older version than the one
    ///   the write's own sync delivered.
    ///
    /// Without a ledger (no secondaries, or a primary that keeps none) there
    /// is nothing to drain and the report is empty.
    pub async fn repair_secondary_sync_failures(
        &self,
        limit: usize,
    ) -> StorageResult<SyncRepairReport> {
        let mut report = SyncRepairReport::default();
        let Some(recorder) = self.sync_failure_recorder() else {
            return Ok(report);
        };
        let Some(ledger) = recorder.ledger() else {
            return Ok(report);
        };

        for record in ledger.list_sync_failures(limit).await? {
            report.examined += 1;
            let key = &record.key;
            let Some(backend) = self.secondary(&key.backend_id) else {
                warn!(
                    tenant = %key.tenant_id,
                    resource_type = %key.resource_type,
                    id = %key.resource_id,
                    backend_id = %key.backend_id,
                    "Dropping a needs-reindex record for a backend this composite no longer has"
                );
                ledger.clear_sync_failure(key).await?;
                recorder.forget(key);
                report.dropped += 1;
                continue;
            };

            let tenant = TenantContext::new(
                TenantId::new(&key.tenant_id),
                TenantPermissions::full_access(),
            );
            match self.resync_current_state(&tenant, key, backend).await {
                Ok(()) => {
                    ledger.clear_sync_failure(key).await?;
                    recorder.forget(key);
                    report.repaired += 1;
                }
                Err(e) => {
                    report.still_failing += 1;
                    let error = error_detail(&e);
                    warn!(
                        tenant = %key.tenant_id,
                        resource_type = %key.resource_type,
                        id = %key.resource_id,
                        backend_id = %key.backend_id,
                        operation = record.operation.as_str(),
                        error = %error,
                        "Repair of a failed secondary sync failed again; the record stays"
                    );
                    ledger
                        .record_sync_failure(&SyncFailureReport {
                            key: key.clone(),
                            operation: record.operation,
                            attempts: 1,
                            error: truncate_error(&error),
                            failed_at: Utc::now(),
                        })
                        .await?;
                }
            }
        }

        report.remaining = ledger.count_sync_failures().await?;
        recorder.set_outstanding(report.remaining);
        Ok(report)
    }

    /// Makes `backend` hold what the primary holds for `key`, right now.
    async fn resync_current_state(
        &self,
        tenant: &TenantContext,
        key: &SyncFailureKey,
        backend: &DynStorage,
    ) -> StorageResult<()> {
        let read = || async {
            match self
                .primary()
                .read(tenant, &key.resource_type, &key.resource_id)
                .await
            {
                // Deleted on the primary, however the backend words it.
                Err(StorageError::Resource(
                    ResourceError::Gone { .. } | ResourceError::NotFound { .. },
                )) => Ok(None),
                other => other,
            }
        };

        let mut current = read().await?;
        for _ in 0..REPAIR_MAX_ROUNDS {
            match &current {
                Some(resource) => {
                    backend
                        .create_or_update(
                            tenant,
                            &key.resource_type,
                            &key.resource_id,
                            resource.content().clone(),
                            resource.fhir_version(),
                        )
                        .await?;
                }
                None => match backend
                    .delete(tenant, &key.resource_type, &key.resource_id)
                    .await
                {
                    Ok(())
                    | Err(StorageError::Resource(
                        ResourceError::NotFound { .. } | ResourceError::Gone { .. },
                    )) => {}
                    Err(e) => return Err(e),
                },
            }

            let after = read().await?;
            let version = |resource: &Option<StoredResource>| {
                resource.as_ref().map(|r| r.version_id().to_string())
            };
            if version(&after) == version(&current) {
                return Ok(());
            }
            current = after;
        }
        Err(StorageError::Backend(BackendError::Unavailable {
            backend_name: key.backend_id.clone(),
            message: format!(
                "{}/{} kept changing on the primary during repair; left for the next pass",
                key.resource_type, key.resource_id
            ),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_round_trips_and_unknown_is_update() {
        for op in [
            SyncOperation::Create,
            SyncOperation::Update,
            SyncOperation::Delete,
        ] {
            assert_eq!(SyncOperation::from_stored(op.as_str()), op);
        }
        assert_eq!(SyncOperation::from_stored("?"), SyncOperation::Update);
    }

    #[test]
    fn errors_are_truncated_on_a_character_boundary() {
        assert_eq!(truncate_error("short"), "short");
        let long = "é".repeat(MAX_RECORDED_ERROR_CHARS + 10);
        let cut = truncate_error(&long);
        assert_eq!(cut.chars().count(), MAX_RECORDED_ERROR_CHARS + 1);
        assert!(cut.ends_with('…'));
    }
}
