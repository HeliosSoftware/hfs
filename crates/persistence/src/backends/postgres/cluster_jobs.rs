//! [`ClusterJobStore`] implementation for PostgreSQL.
//!
//! The claim query is a clone of the bulk-export claim shape
//! (`bulk_export.rs`, `impl ExportClaimStrategy for PostgresBackend`): one
//! transaction around `SELECT … FOR UPDATE SKIP LOCKED` + a fencing-token
//! bump. Keep the two in sync when touching either.

use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::core::cluster_job_store::{
    ClusterJobId, ClusterJobLease, ClusterJobRecord, ClusterJobState, ClusterJobStore,
    ClusterLeaseError, JobKind, WorkerId,
};
use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::{TenantContext, TenantId, TenantPermissions};

/// Pool-sharing [`ClusterJobStore`] handle for PostgreSQL.
///
/// Obtained via `ResourceStorage::cluster_job_store()` on
/// [`super::PostgresBackend`] (the same seam shape as `sof_runner()`): the
/// backend is not `Clone`, so the store is a cheap handle over the shared
/// connection pool rather than an impl on the backend itself.
pub struct PgClusterJobStore {
    pool: deadpool_postgres::Pool,
}

impl PgClusterJobStore {
    /// Creates a store handle over the backend's connection pool.
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

fn record_from_row(row: &tokio_postgres::Row) -> StorageResult<ClusterJobRecord> {
    let kind_str: String = row.get(1);
    let state_str: String = row.get(2);
    Ok(ClusterJobRecord {
        id: ClusterJobId::from_string(row.get::<_, String>(0)),
        kind: JobKind::parse(&kind_str)
            .ok_or_else(|| internal_error(format!("unknown cluster job kind: {kind_str}")))?,
        state: ClusterJobState::parse(&state_str)
            .ok_or_else(|| internal_error(format!("unknown cluster job state: {state_str}")))?,
        payload: row.get(3),
        progress: row.get(4),
        result: row.get(5),
        error: row.get(6),
        cancel_requested: row.get(7),
        created_at: row.get(8),
        started_at: row.get(9),
        finished_at: row.get(10),
    })
}

const RECORD_COLUMNS: &str = "id, kind, status, payload, progress, result, error_message, \
     cancel_requested, created_at, started_at, finished_at";

#[async_trait]
impl ClusterJobStore for PgClusterJobStore {
    async fn enqueue(
        &self,
        tenant: &TenantContext,
        kind: JobKind,
        payload: Value,
    ) -> StorageResult<ClusterJobId> {
        let client = self.get_client().await?;
        let job_id = ClusterJobId::random();
        client
            .execute(
                "INSERT INTO cluster_jobs (id, tenant_id, kind, status, payload)
                 VALUES ($1, $2, $3, 'queued', $4)",
                &[
                    &job_id.as_str(),
                    &tenant.tenant_id().as_str(),
                    &kind.as_str(),
                    &payload,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to enqueue cluster job: {}", e)))?;
        Ok(job_id)
    }

    async fn claim_next(
        &self,
        kind: JobKind,
        worker_id: &WorkerId,
        lease_duration: StdDuration,
    ) -> StorageResult<Option<(ClusterJobLease, Value)>> {
        let mut client = self.get_client().await?;
        let now = Utc::now();
        let lease_expiry = now
            + chrono::Duration::from_std(lease_duration)
                .unwrap_or_else(|_| chrono::Duration::seconds(60));

        // Cloned from the bulk-export claim (bulk_export.rs claim_next):
        // txn + FOR UPDATE SKIP LOCKED + fencing-token bump.
        let txn = client
            .transaction()
            .await
            .map_err(|e| internal_error(format!("Failed to begin claim txn: {}", e)))?;

        let rows = txn
            .query(
                "SELECT id, tenant_id, fencing_token, payload FROM cluster_jobs
                 WHERE kind = $1
                   AND (status = 'queued'
                        OR (status = 'running' AND (lease_expiry IS NULL OR lease_expiry < $2)))
                 ORDER BY created_at
                 LIMIT 1
                 FOR UPDATE SKIP LOCKED",
                &[&kind.as_str(), &now],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to select claimable job: {}", e)))?;

        let Some(row) = rows.first() else {
            txn.commit()
                .await
                .map_err(|e| internal_error(format!("Failed to commit claim txn: {}", e)))?;
            return Ok(None);
        };
        let job_id: String = row.get(0);
        let tenant_id: String = row.get(1);
        let fencing_token: i64 = row.get(2);
        let payload: Value = row.get(3);
        let new_token = fencing_token + 1;

        txn.execute(
            "UPDATE cluster_jobs
             SET status = 'running', worker_id = $1, lease_expiry = $2,
                 heartbeat_at = $3, fencing_token = $4,
                 started_at = COALESCE(started_at, $3)
             WHERE id = $5",
            &[
                &worker_id.as_str(),
                &lease_expiry,
                &now,
                &new_token,
                &job_id.as_str(),
            ],
        )
        .await
        .map_err(|e| internal_error(format!("Failed to claim cluster job: {}", e)))?;

        txn.commit()
            .await
            .map_err(|e| internal_error(format!("Failed to commit claim txn: {}", e)))?;

        Ok(Some((
            ClusterJobLease {
                job_id: ClusterJobId::from_string(job_id),
                tenant: TenantContext::new(
                    TenantId::new(tenant_id),
                    TenantPermissions::full_access(),
                ),
                worker_id: worker_id.clone(),
                lease_expiry,
                fencing_token: new_token as u64,
            },
            payload,
        )))
    }

    async fn heartbeat(&self, lease: &ClusterJobLease) -> Result<DateTime<Utc>, ClusterLeaseError> {
        let client = self
            .get_client()
            .await
            .map_err(ClusterLeaseError::Storage)?;
        let now = Utc::now();
        let new_expiry = now + chrono::Duration::seconds(60);
        let affected = client
            .execute(
                "UPDATE cluster_jobs
                 SET lease_expiry = $1, heartbeat_at = $2
                 WHERE id = $3 AND worker_id = $4 AND fencing_token = $5",
                &[
                    &new_expiry,
                    &now,
                    &lease.job_id.as_str(),
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| {
                ClusterLeaseError::Storage(internal_error(format!("heartbeat failed: {e}")))
            })?;
        if affected == 0 {
            Err(ClusterLeaseError::LeaseLost {
                job_id: lease.job_id.clone(),
            })
        } else {
            Ok(new_expiry)
        }
    }

    async fn release(&self, lease: ClusterJobLease) -> StorageResult<()> {
        let client = self.get_client().await?;
        client
            .execute(
                "UPDATE cluster_jobs
                 SET status = 'queued', worker_id = NULL, lease_expiry = NULL
                 WHERE id = $1 AND worker_id = $2 AND fencing_token = $3
                   AND status = 'running'",
                &[
                    &lease.job_id.as_str(),
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to release lease: {}", e)))?;
        Ok(())
    }

    async fn get_status(
        &self,
        tenant: &TenantContext,
        job_id: &ClusterJobId,
    ) -> StorageResult<Option<ClusterJobRecord>> {
        let client = self.get_client().await?;
        let row = client
            .query_opt(
                &format!(
                    "SELECT {RECORD_COLUMNS} FROM cluster_jobs
                     WHERE id = $1 AND tenant_id = $2"
                ),
                &[&job_id.as_str(), &tenant.tenant_id().as_str()],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to read cluster job: {}", e)))?;
        row.as_ref().map(record_from_row).transpose()
    }

    async fn list_jobs(
        &self,
        tenant: &TenantContext,
        kind: JobKind,
    ) -> StorageResult<Vec<ClusterJobRecord>> {
        let client = self.get_client().await?;
        let rows = client
            .query(
                &format!(
                    "SELECT {RECORD_COLUMNS} FROM cluster_jobs
                     WHERE tenant_id = $1 AND kind = $2
                     ORDER BY created_at DESC"
                ),
                &[&tenant.tenant_id().as_str(), &kind.as_str()],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to list cluster jobs: {}", e)))?;
        rows.iter().map(record_from_row).collect()
    }

    async fn cancel(&self, tenant: &TenantContext, job_id: &ClusterJobId) -> StorageResult<bool> {
        let client = self.get_client().await?;
        // Terminal jobs are left untouched; the tenant check keeps another
        // tenant's job indistinguishable from a missing one.
        let row = client
            .query_opt(
                "WITH flipped AS (
                     UPDATE cluster_jobs
                     SET status = 'cancelled', cancel_requested = TRUE, finished_at = NOW()
                     WHERE id = $1 AND tenant_id = $2 AND status IN ('queued', 'running')
                     RETURNING id
                 )
                 SELECT EXISTS(SELECT 1 FROM cluster_jobs WHERE id = $1 AND tenant_id = $2)",
                &[&job_id.as_str(), &tenant.tenant_id().as_str()],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to cancel cluster job: {}", e)))?;
        Ok(row.map(|r| r.get::<_, bool>(0)).unwrap_or(false))
    }

    async fn cancel_requested(&self, lease: &ClusterJobLease) -> Result<bool, ClusterLeaseError> {
        let client = self
            .get_client()
            .await
            .map_err(ClusterLeaseError::Storage)?;
        let row = client
            .query_opt(
                "SELECT cancel_requested FROM cluster_jobs
                 WHERE id = $1 AND worker_id = $2 AND fencing_token = $3",
                &[
                    &lease.job_id.as_str(),
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| {
                ClusterLeaseError::Storage(internal_error(format!(
                    "cancel_requested check failed: {e}"
                )))
            })?;
        match row {
            Some(row) => Ok(row.get(0)),
            None => Err(ClusterLeaseError::LeaseLost {
                job_id: lease.job_id.clone(),
            }),
        }
    }

    async fn update_progress(
        &self,
        lease: &ClusterJobLease,
        progress: Value,
    ) -> Result<(), ClusterLeaseError> {
        let client = self
            .get_client()
            .await
            .map_err(ClusterLeaseError::Storage)?;
        let affected = client
            .execute(
                "UPDATE cluster_jobs SET progress = $1
                 WHERE id = $2 AND worker_id = $3 AND fencing_token = $4
                   AND status = 'running'",
                &[
                    &progress,
                    &lease.job_id.as_str(),
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| {
                ClusterLeaseError::Storage(internal_error(format!("progress update failed: {e}")))
            })?;
        if affected == 0 {
            Err(ClusterLeaseError::LeaseLost {
                job_id: lease.job_id.clone(),
            })
        } else {
            Ok(())
        }
    }

    async fn complete(
        &self,
        lease: &ClusterJobLease,
        result: Value,
    ) -> Result<(), ClusterLeaseError> {
        let client = self
            .get_client()
            .await
            .map_err(ClusterLeaseError::Storage)?;
        let affected = client
            .execute(
                "UPDATE cluster_jobs
                 SET status = 'completed', result = $1, finished_at = NOW()
                 WHERE id = $2 AND worker_id = $3 AND fencing_token = $4
                   AND status = 'running'",
                &[
                    &result,
                    &lease.job_id.as_str(),
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| {
                ClusterLeaseError::Storage(internal_error(format!("complete failed: {e}")))
            })?;
        if affected == 0 {
            Err(ClusterLeaseError::LeaseLost {
                job_id: lease.job_id.clone(),
            })
        } else {
            Ok(())
        }
    }

    async fn fail(
        &self,
        lease: &ClusterJobLease,
        error_message: &str,
    ) -> Result<(), ClusterLeaseError> {
        let client = self
            .get_client()
            .await
            .map_err(ClusterLeaseError::Storage)?;
        let affected = client
            .execute(
                "UPDATE cluster_jobs
                 SET status = 'failed', error_message = $1, finished_at = NOW()
                 WHERE id = $2 AND worker_id = $3 AND fencing_token = $4
                   AND status = 'running'",
                &[
                    &error_message,
                    &lease.job_id.as_str(),
                    &lease.worker_id.as_str(),
                    &(lease.fencing_token as i64),
                ],
            )
            .await
            .map_err(|e| ClusterLeaseError::Storage(internal_error(format!("fail failed: {e}"))))?;
        if affected == 0 {
            Err(ClusterLeaseError::LeaseLost {
                job_id: lease.job_id.clone(),
            })
        } else {
            Ok(())
        }
    }

    async fn delete_terminal_before(
        &self,
        kind: JobKind,
        cutoff: DateTime<Utc>,
    ) -> StorageResult<Vec<ClusterJobId>> {
        let client = self.get_client().await?;
        let rows = client
            .query(
                "DELETE FROM cluster_jobs
                 WHERE kind = $1
                   AND status IN ('completed', 'failed', 'cancelled')
                   AND finished_at IS NOT NULL AND finished_at < $2
                 RETURNING id",
                &[&kind.as_str(), &cutoff],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to reap cluster jobs: {}", e)))?;
        Ok(rows
            .iter()
            .map(|r| ClusterJobId::from_string(r.get::<_, String>(0)))
            .collect())
    }
}
