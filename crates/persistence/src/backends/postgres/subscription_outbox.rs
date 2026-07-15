//! [`SubscriptionDeliveryOutbox`] implementation for PostgreSQL.
//!
//! The claim query is the cluster-jobs claim shape (`cluster_jobs.rs`
//! `claim_next`): one transaction around `SELECT … FOR UPDATE SKIP LOCKED` +
//! a fencing-token bump — with two outbox-specific twists: eligibility is
//! schedule-based (`next_attempt_at` reached, or an expired `delivering`
//! lease) and the claim itself bumps the `attempts` counter, so the retry
//! bookkeeping survives any crash.

use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::core::subscription_delivery::{
    ClaimedDelivery, DeliveryId, DeliveryLease, DeliveryLeaseError, DeliveryRecord, DeliveryState,
    NewDelivery, SubscriptionDeliveryOutbox, WorkerId,
};
use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::{TenantContext, TenantId, TenantPermissions};

/// Pool-sharing [`SubscriptionDeliveryOutbox`] handle for PostgreSQL.
///
/// Obtained via `ResourceStorage::subscription_delivery_outbox()` on
/// [`super::PostgresBackend`] (the same seam shape as `cluster_job_store()`).
pub struct PgSubscriptionDeliveryOutbox {
    pool: deadpool_postgres::Pool,
}

impl PgSubscriptionDeliveryOutbox {
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
impl SubscriptionDeliveryOutbox for PgSubscriptionDeliveryOutbox {
    async fn enqueue(
        &self,
        tenant: &TenantContext,
        delivery: NewDelivery,
    ) -> StorageResult<DeliveryId> {
        let client = self.get_client().await?;
        let event_number = delivery.event_number.map(|n| n as i64);
        let row = client
            .query_one(
                "INSERT INTO subscription_delivery_outbox
                     (tenant_id, subscription_id, event_number, notification_type, bundle)
                 VALUES ($1, $2, $3, $4, $5)
                 RETURNING id",
                &[
                    &tenant.tenant_id().as_str(),
                    &delivery.subscription_id,
                    &event_number,
                    &delivery.notification_type,
                    &delivery.bundle,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to enqueue delivery: {}", e)))?;
        Ok(DeliveryId::from_i64(row.get(0)))
    }

    async fn claim_next(
        &self,
        worker_id: &WorkerId,
        lease_duration: StdDuration,
    ) -> StorageResult<Option<ClaimedDelivery>> {
        let mut client = self.get_client().await?;

        let txn = client
            .transaction()
            .await
            .map_err(|e| internal_error(format!("Failed to begin claim txn: {}", e)))?;

        // Schedule and lease math run entirely on the database clock (like
        // the refresh cache): `next_attempt_at` was stamped by the database,
        // so comparing it against an instance clock would make "due" depend
        // on clock skew.
        let rows = txn
            .query(
                "SELECT id, tenant_id, subscription_id, event_number, notification_type,
                        bundle, attempts, fencing_token
                 FROM subscription_delivery_outbox
                 WHERE (status = 'queued' AND next_attempt_at <= NOW())
                    OR (status = 'delivering'
                        AND (lease_expiry IS NULL OR lease_expiry < NOW()))
                 ORDER BY next_attempt_at, id
                 LIMIT 1
                 FOR UPDATE SKIP LOCKED",
                &[],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to select claimable delivery: {}", e)))?;

        let Some(row) = rows.first() else {
            txn.commit()
                .await
                .map_err(|e| internal_error(format!("Failed to commit claim txn: {}", e)))?;
            return Ok(None);
        };
        let id: i64 = row.get(0);
        let tenant_id: String = row.get(1);
        let subscription_id: String = row.get(2);
        let event_number: Option<i64> = row.get(3);
        let notification_type: String = row.get(4);
        let bundle: serde_json::Value = row.get(5);
        let attempts: i32 = row.get(6);
        let fencing_token: i64 = row.get(7);
        let new_token = fencing_token + 1;
        let new_attempts = attempts + 1;

        let lease_secs = lease_duration.as_secs_f64();
        let updated = txn
            .query_one(
                "UPDATE subscription_delivery_outbox
                 SET status = 'delivering', worker_id = $1,
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
            .map_err(|e| internal_error(format!("Failed to claim delivery: {}", e)))?;
        let lease_expiry: DateTime<Utc> = updated.get(0);

        txn.commit()
            .await
            .map_err(|e| internal_error(format!("Failed to commit claim txn: {}", e)))?;

        Ok(Some(ClaimedDelivery {
            lease: DeliveryLease {
                id: DeliveryId::from_i64(id),
                tenant: TenantContext::new(
                    TenantId::new(tenant_id),
                    TenantPermissions::full_access(),
                ),
                worker_id: worker_id.clone(),
                lease_expiry,
                fencing_token: new_token as u64,
            },
            subscription_id,
            event_number: event_number.map(|n| n.max(0) as u64),
            notification_type,
            bundle,
            attempts: new_attempts.max(0) as u32,
        }))
    }

    async fn complete(&self, lease: &DeliveryLease) -> Result<(), DeliveryLeaseError> {
        let client = self
            .get_client()
            .await
            .map_err(DeliveryLeaseError::Storage)?;
        let affected = client
            .execute(
                "UPDATE subscription_delivery_outbox
                 SET status = 'delivered', worker_id = NULL, lease_expiry = NULL,
                     finished_at = NOW()
                 WHERE id = $1 AND worker_id = $2 AND fencing_token = $3
                   AND status = 'delivering'",
                &[
                    &lease.id.as_i64(),
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| {
                DeliveryLeaseError::Storage(internal_error(format!("complete failed: {e}")))
            })?;
        if affected == 0 {
            Err(DeliveryLeaseError::LeaseLost { id: lease.id })
        } else {
            Ok(())
        }
    }

    async fn release_for_retry(
        &self,
        lease: &DeliveryLease,
        next_attempt_at: DateTime<Utc>,
        error: &str,
    ) -> Result<(), DeliveryLeaseError> {
        let client = self
            .get_client()
            .await
            .map_err(DeliveryLeaseError::Storage)?;
        let affected = client
            .execute(
                "UPDATE subscription_delivery_outbox
                 SET status = 'queued', worker_id = NULL, lease_expiry = NULL,
                     next_attempt_at = $1, last_error = $2
                 WHERE id = $3 AND worker_id = $4 AND fencing_token = $5
                   AND status = 'delivering'",
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
                DeliveryLeaseError::Storage(internal_error(format!(
                    "release_for_retry failed: {e}"
                )))
            })?;
        if affected == 0 {
            Err(DeliveryLeaseError::LeaseLost { id: lease.id })
        } else {
            Ok(())
        }
    }

    async fn fail(&self, lease: &DeliveryLease, error: &str) -> Result<(), DeliveryLeaseError> {
        let client = self
            .get_client()
            .await
            .map_err(DeliveryLeaseError::Storage)?;
        let affected = client
            .execute(
                "UPDATE subscription_delivery_outbox
                 SET status = 'failed', worker_id = NULL, lease_expiry = NULL,
                     last_error = $1, finished_at = NOW()
                 WHERE id = $2 AND worker_id = $3 AND fencing_token = $4
                   AND status = 'delivering'",
                &[
                    &error,
                    &lease.id.as_i64(),
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| {
                DeliveryLeaseError::Storage(internal_error(format!("fail failed: {e}")))
            })?;
        if affected == 0 {
            Err(DeliveryLeaseError::LeaseLost { id: lease.id })
        } else {
            Ok(())
        }
    }

    async fn get(
        &self,
        tenant: &TenantContext,
        id: DeliveryId,
    ) -> StorageResult<Option<DeliveryRecord>> {
        let client = self.get_client().await?;
        let row = client
            .query_opt(
                "SELECT subscription_id, status, attempts, next_attempt_at, last_error,
                        event_number, notification_type
                 FROM subscription_delivery_outbox
                 WHERE id = $1 AND tenant_id = $2",
                &[&id.as_i64(), &tenant.tenant_id().as_str()],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to read delivery: {}", e)))?;
        row.map(|row| {
            let state_str: String = row.get(1);
            let state = DeliveryState::parse(&state_str)
                .ok_or_else(|| internal_error(format!("unknown delivery state: {state_str}")))?;
            Ok(DeliveryRecord {
                id,
                subscription_id: row.get(0),
                state,
                attempts: row.get::<_, i32>(2).max(0) as u32,
                next_attempt_at: row.get(3),
                last_error: row.get(4),
                event_number: row.get::<_, Option<i64>>(5).map(|n| n.max(0) as u64),
                notification_type: row.get(6),
            })
        })
        .transpose()
    }

    async fn delete_terminal_before(&self, cutoff: DateTime<Utc>) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let removed = client
            .execute(
                "DELETE FROM subscription_delivery_outbox
                 WHERE status IN ('delivered', 'failed')
                   AND finished_at IS NOT NULL AND finished_at < $1",
                &[&cutoff],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to reap deliveries: {}", e)))?;
        Ok(removed)
    }
}
