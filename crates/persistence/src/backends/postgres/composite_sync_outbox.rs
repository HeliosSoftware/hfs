//! [`CompositeSyncOutbox`] implementation for PostgreSQL.
//!
//! The claim query is the same shape as `subscription_outbox.rs`'s
//! `claim_next` (itself cloned from `cluster_jobs.rs`): one transaction
//! around `SELECT … FOR UPDATE SKIP LOCKED` + a fencing-token bump, with
//! schedule-based eligibility and an attempt-counter bump on claim so retry
//! bookkeeping survives any crash.

use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::core::composite_sync_outbox::{
    ClaimedSyncEvent, CompositeSyncOutbox, NewSyncOutboxEntry, SyncLease, SyncLeaseError,
    SyncOperation, SyncOutboxId, SyncOutboxRecord, SyncOutboxState, WorkerId,
};
use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::{TenantContext, TenantId, TenantPermissions};

/// Pool-sharing [`CompositeSyncOutbox`] handle for PostgreSQL.
///
/// Obtained via `ResourceStorage::composite_sync_outbox()` on
/// [`super::PostgresBackend`] (the same seam shape as `cluster_job_store()`).
pub struct PgCompositeSyncOutbox {
    pool: deadpool_postgres::Pool,
}

impl PgCompositeSyncOutbox {
    /// Creates an outbox handle over the backend's connection pool.
    pub(crate) fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }

    async fn get_client(&self) -> StorageResult<deadpool_postgres::Client> {
        self.pool.get().await.map_err(|e| {
            StorageError::Backend(BackendError::ConnectionFailed {
                backend_name: "postgres".to_string(),
                message: e.to_string(),
            })
        })
    }
}

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "postgres".to_string(),
        message,
        source: None,
    })
}

#[async_trait]
impl CompositeSyncOutbox for PgCompositeSyncOutbox {
    async fn enqueue(
        &self,
        tenant: &TenantContext,
        entry: NewSyncOutboxEntry,
    ) -> StorageResult<SyncOutboxId> {
        let client = self.get_client().await?;
        let row = client
            .query_one(
                "INSERT INTO composite_sync_outbox
                     (tenant_id, backend_id, operation, resource_type, resource_id,
                      content, version, fhir_version)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 RETURNING id",
                &[
                    &tenant.tenant_id().as_str(),
                    &entry.backend_id,
                    &entry.operation.as_str(),
                    &entry.resource_type,
                    &entry.resource_id,
                    &entry.content,
                    &entry.version,
                    &entry.fhir_version,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to enqueue sync entry: {}", e)))?;
        Ok(SyncOutboxId::from_i64(row.get(0)))
    }

    async fn claim_next(
        &self,
        worker_id: &WorkerId,
        lease_duration: StdDuration,
    ) -> StorageResult<Option<ClaimedSyncEvent>> {
        let mut client = self.get_client().await?;

        let txn = client
            .transaction()
            .await
            .map_err(|e| internal_error(format!("Failed to begin claim txn: {}", e)))?;

        // Schedule and lease math run entirely on the database clock (like
        // the subscription delivery outbox and refresh cache): next_attempt_at
        // was stamped by the database, so comparing it against an instance
        // clock would make "due" depend on clock skew.
        let rows = txn
            .query(
                "SELECT id, tenant_id, backend_id, operation, resource_type, resource_id,
                        content, version, fhir_version, attempts, fencing_token
                 FROM composite_sync_outbox
                 WHERE (status = 'queued' AND next_attempt_at <= NOW())
                    OR (status = 'applying'
                        AND (lease_expiry IS NULL OR lease_expiry < NOW()))
                 ORDER BY next_attempt_at, id
                 LIMIT 1
                 FOR UPDATE SKIP LOCKED",
                &[],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to select claimable entry: {}", e)))?;

        let Some(row) = rows.first() else {
            txn.commit()
                .await
                .map_err(|e| internal_error(format!("Failed to commit claim txn: {}", e)))?;
            return Ok(None);
        };
        let id: i64 = row.get(0);
        let tenant_id: String = row.get(1);
        let backend_id: String = row.get(2);
        let operation_str: String = row.get(3);
        let resource_type: String = row.get(4);
        let resource_id: String = row.get(5);
        let content: Option<serde_json::Value> = row.get(6);
        let version: Option<String> = row.get(7);
        let fhir_version: Option<String> = row.get(8);
        let attempts: i32 = row.get(9);
        let fencing_token: i64 = row.get(10);
        let operation = SyncOperation::parse(&operation_str)
            .ok_or_else(|| internal_error(format!("unknown sync operation: {operation_str}")))?;
        let new_token = fencing_token + 1;
        let new_attempts = attempts + 1;

        let lease_secs = lease_duration.as_secs_f64();
        let updated = txn
            .query_one(
                "UPDATE composite_sync_outbox
                 SET status = 'applying', worker_id = $1,
                     lease_expiry = NOW() + $2 * INTERVAL '1 second',
                     fencing_token = $3, attempts = $4
                 WHERE id = $5
                 RETURNING lease_expiry",
                &[
                    &worker_id.as_str(),
                    &lease_secs,
                    &new_token,
                    &new_attempts,
                    &id,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to claim entry: {}", e)))?;
        let lease_expiry: DateTime<Utc> = updated.get(0);

        txn.commit()
            .await
            .map_err(|e| internal_error(format!("Failed to commit claim txn: {}", e)))?;

        Ok(Some(ClaimedSyncEvent {
            lease: SyncLease {
                id: SyncOutboxId::from_i64(id),
                tenant: TenantContext::new(
                    TenantId::new(tenant_id),
                    TenantPermissions::full_access(),
                ),
                worker_id: worker_id.clone(),
                lease_expiry,
                fencing_token: new_token as u64,
            },
            backend_id,
            operation,
            resource_type,
            resource_id,
            content,
            version,
            fhir_version,
            attempts: new_attempts.max(0) as u32,
        }))
    }

    async fn complete(&self, lease: &SyncLease) -> Result<(), SyncLeaseError> {
        let client = self.get_client().await.map_err(SyncLeaseError::Storage)?;
        let affected = client
            .execute(
                "UPDATE composite_sync_outbox
                 SET status = 'applied', worker_id = NULL, lease_expiry = NULL,
                     finished_at = NOW()
                 WHERE id = $1 AND worker_id = $2 AND fencing_token = $3
                   AND status = 'applying'",
                &[
                    &lease.id.as_i64(),
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| {
                SyncLeaseError::Storage(internal_error(format!("complete failed: {e}")))
            })?;
        if affected == 0 {
            Err(SyncLeaseError::LeaseLost { id: lease.id })
        } else {
            Ok(())
        }
    }

    async fn release_for_retry(
        &self,
        lease: &SyncLease,
        next_attempt_at: DateTime<Utc>,
        error: &str,
    ) -> Result<(), SyncLeaseError> {
        let client = self.get_client().await.map_err(SyncLeaseError::Storage)?;
        let affected = client
            .execute(
                "UPDATE composite_sync_outbox
                 SET status = 'queued', worker_id = NULL, lease_expiry = NULL,
                     next_attempt_at = $1, last_error = $2
                 WHERE id = $3 AND worker_id = $4 AND fencing_token = $5
                   AND status = 'applying'",
                &[
                    &next_attempt_at,
                    &error,
                    &lease.id.as_i64(),
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| {
                SyncLeaseError::Storage(internal_error(format!("release_for_retry failed: {e}")))
            })?;
        if affected == 0 {
            Err(SyncLeaseError::LeaseLost { id: lease.id })
        } else {
            Ok(())
        }
    }

    async fn fail(&self, lease: &SyncLease, error: &str) -> Result<(), SyncLeaseError> {
        let client = self.get_client().await.map_err(SyncLeaseError::Storage)?;
        let affected = client
            .execute(
                "UPDATE composite_sync_outbox
                 SET status = 'failed', worker_id = NULL, lease_expiry = NULL,
                     last_error = $1, finished_at = NOW()
                 WHERE id = $2 AND worker_id = $3 AND fencing_token = $4
                   AND status = 'applying'",
                &[
                    &error,
                    &lease.id.as_i64(),
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| SyncLeaseError::Storage(internal_error(format!("fail failed: {e}"))))?;
        if affected == 0 {
            Err(SyncLeaseError::LeaseLost { id: lease.id })
        } else {
            Ok(())
        }
    }

    async fn get(
        &self,
        tenant: &TenantContext,
        id: SyncOutboxId,
    ) -> StorageResult<Option<SyncOutboxRecord>> {
        let client = self.get_client().await?;
        let row = client
            .query_opt(
                "SELECT backend_id, status, attempts, next_attempt_at, last_error
                 FROM composite_sync_outbox
                 WHERE id = $1 AND tenant_id = $2",
                &[&id.as_i64(), &tenant.tenant_id().as_str()],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to read sync entry: {}", e)))?;
        row.map(|row| {
            let state_str: String = row.get(1);
            let state = SyncOutboxState::parse(&state_str)
                .ok_or_else(|| internal_error(format!("unknown sync outbox state: {state_str}")))?;
            Ok(SyncOutboxRecord {
                id,
                backend_id: row.get(0),
                state,
                attempts: row.get::<_, i32>(2).max(0) as u32,
                next_attempt_at: row.get(3),
                last_error: row.get(4),
            })
        })
        .transpose()
    }

    async fn delete_terminal_before(&self, cutoff: DateTime<Utc>) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let removed = client
            .execute(
                "DELETE FROM composite_sync_outbox
                 WHERE status IN ('applied', 'failed')
                   AND finished_at IS NOT NULL AND finished_at < $1",
                &[&cutoff],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to reap sync entries: {}", e)))?;
        Ok(removed)
    }
}
