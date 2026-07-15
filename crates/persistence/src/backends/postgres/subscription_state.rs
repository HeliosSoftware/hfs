//! [`SubscriptionStateStore`] + [`SubscriptionHydrationSource`] for
//! PostgreSQL.
//!
//! Counters are single-statement upsert-increments
//! (`INSERT … ON CONFLICT … DO UPDATE … RETURNING`), so concurrent callers
//! across instances observe distinct consecutive values with no explicit
//! locking, and a row is created lazily by its first increment — a
//! Subscription resource update can never reset shared counters.
//!
//! Hydration reads the `resources` table directly and cross-tenant: the
//! tenant registry is opt-in metadata that may not cover every tenant with
//! resources, so boot reconciliation must not iterate it. `Basic` rows are
//! pre-filtered in SQL to the R4 backport topic marker (the
//! `http://hl7.org/fhir/fhir-types` / `SubscriptionTopic` coding checked by
//! `parse_r4_backport_basic_topic_resource`) so hydration never hauls a
//! deployment's unrelated `Basic` resources into memory.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::core::subscription_state::{
    HydratedResource, SubscriptionHydrationSource, SubscriptionStateRecord, SubscriptionStateStore,
};
use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::TenantContext;

/// Pool-sharing [`SubscriptionStateStore`] / [`SubscriptionHydrationSource`]
/// handle for PostgreSQL.
///
/// Obtained via `ResourceStorage::subscription_state_store()` /
/// `subscription_hydration_source()` on [`super::PostgresBackend`] (the same
/// seam shape as `cluster_job_store()`).
pub struct PgSubscriptionStateStore {
    pool: deadpool_postgres::Pool,
}

impl PgSubscriptionStateStore {
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

#[async_trait]
impl SubscriptionStateStore for PgSubscriptionStateStore {
    async fn next_event_number(
        &self,
        tenant: &TenantContext,
        subscription_id: &str,
    ) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let row = client
            .query_one(
                "INSERT INTO subscription_state (tenant_id, subscription_id, event_number)
                 VALUES ($1, $2, 1)
                 ON CONFLICT (tenant_id, subscription_id)
                 DO UPDATE SET event_number = subscription_state.event_number + 1,
                               updated_at = NOW()
                 RETURNING event_number",
                &[&tenant.tenant_id().as_str(), &subscription_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to increment event number: {}", e)))?;
        Ok(row.get::<_, i64>(0).max(0) as u64)
    }

    async fn record_failure(
        &self,
        tenant: &TenantContext,
        subscription_id: &str,
    ) -> StorageResult<u32> {
        let client = self.get_client().await?;
        let row = client
            .query_one(
                "INSERT INTO subscription_state (tenant_id, subscription_id, consecutive_failures)
                 VALUES ($1, $2, 1)
                 ON CONFLICT (tenant_id, subscription_id)
                 DO UPDATE SET consecutive_failures = subscription_state.consecutive_failures + 1,
                               updated_at = NOW()
                 RETURNING consecutive_failures",
                &[&tenant.tenant_id().as_str(), &subscription_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to record failure: {}", e)))?;
        Ok(row.get::<_, i32>(0).max(0) as u32)
    }

    async fn reset_failures(
        &self,
        tenant: &TenantContext,
        subscription_id: &str,
    ) -> StorageResult<()> {
        let client = self.get_client().await?;
        client
            .execute(
                "UPDATE subscription_state
                 SET consecutive_failures = 0, updated_at = NOW()
                 WHERE tenant_id = $1 AND subscription_id = $2",
                &[&tenant.tenant_id().as_str(), &subscription_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to reset failures: {}", e)))?;
        Ok(())
    }

    async fn set_status(
        &self,
        tenant: &TenantContext,
        subscription_id: &str,
        status: &str,
    ) -> StorageResult<()> {
        let client = self.get_client().await?;
        client
            .execute(
                "INSERT INTO subscription_state (tenant_id, subscription_id, status)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (tenant_id, subscription_id)
                 DO UPDATE SET status = EXCLUDED.status, updated_at = NOW()",
                &[&tenant.tenant_id().as_str(), &subscription_id, &status],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to set status: {}", e)))?;
        Ok(())
    }

    async fn get(
        &self,
        tenant: &TenantContext,
        subscription_id: &str,
    ) -> StorageResult<Option<SubscriptionStateRecord>> {
        let client = self.get_client().await?;
        let row = client
            .query_opt(
                "SELECT event_number, consecutive_failures, status, updated_at
                 FROM subscription_state
                 WHERE tenant_id = $1 AND subscription_id = $2",
                &[&tenant.tenant_id().as_str(), &subscription_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to read subscription state: {}", e)))?;
        Ok(row.map(|row| SubscriptionStateRecord {
            event_number: row.get::<_, i64>(0).max(0) as u64,
            consecutive_failures: row.get::<_, i32>(1).max(0) as u32,
            status: row.get(2),
            updated_at: row.get(3),
        }))
    }

    async fn delete(&self, tenant: &TenantContext, subscription_id: &str) -> StorageResult<()> {
        let client = self.get_client().await?;
        client
            .execute(
                "DELETE FROM subscription_state
                 WHERE tenant_id = $1 AND subscription_id = $2",
                &[&tenant.tenant_id().as_str(), &subscription_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to delete subscription state: {}", e)))?;
        client
            .execute(
                "DELETE FROM subscription_notification_events
                 WHERE tenant_id = $1 AND subscription_id = $2",
                &[&tenant.tenant_id().as_str(), &subscription_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to delete notification events: {}", e)))?;
        Ok(())
    }

    async fn put_notification_event(
        &self,
        tenant: &TenantContext,
        subscription_id: &str,
        event_number: u64,
        bundle: &Value,
    ) -> StorageResult<()> {
        let client = self.get_client().await?;
        client
            .execute(
                "INSERT INTO subscription_notification_events
                     (tenant_id, subscription_id, event_number, bundle)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (tenant_id, subscription_id, event_number)
                 DO UPDATE SET bundle = EXCLUDED.bundle",
                &[
                    &tenant.tenant_id().as_str(),
                    &subscription_id,
                    &(event_number as i64),
                    bundle,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to store notification event: {}", e)))?;
        Ok(())
    }

    async fn get_notification_event(
        &self,
        tenant: &TenantContext,
        subscription_id: &str,
        event_number: u64,
    ) -> StorageResult<Option<Value>> {
        let client = self.get_client().await?;
        let row = client
            .query_opt(
                "SELECT bundle FROM subscription_notification_events
                 WHERE tenant_id = $1 AND subscription_id = $2 AND event_number = $3",
                &[
                    &tenant.tenant_id().as_str(),
                    &subscription_id,
                    &(event_number as i64),
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to read notification event: {}", e)))?;
        Ok(row.map(|row| row.get(0)))
    }

    async fn prune_notification_events_before(&self, cutoff: DateTime<Utc>) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let removed = client
            .execute(
                "DELETE FROM subscription_notification_events WHERE created_at < $1",
                &[&cutoff],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to prune notification events: {}", e)))?;
        Ok(removed)
    }
}

/// The R4 backport topic marker `parse_r4_backport_basic_topic_resource`
/// checks: a `Basic` resource whose `code.coding` contains this entry is a
/// backport SubscriptionTopic. Used as a JSONB containment pre-filter so
/// hydration never loads unrelated `Basic` rows.
const R4_BASIC_TOPIC_MARKER: &str =
    r#"{"coding":[{"system":"http://hl7.org/fhir/fhir-types","code":"SubscriptionTopic"}]}"#;

fn hydrated_from_row(row: &tokio_postgres::Row) -> HydratedResource {
    HydratedResource {
        tenant_id: row.get(0),
        resource_type: row.get(1),
        resource_id: row.get(2),
        fhir_version: row.get(3),
        content: row.get(4),
    }
}

#[async_trait]
impl SubscriptionHydrationSource for PgSubscriptionStateStore {
    async fn list_current(&self, resource_types: &[&str]) -> StorageResult<Vec<HydratedResource>> {
        let client = self.get_client().await?;
        let types: Vec<String> = resource_types.iter().map(|t| t.to_string()).collect();
        let marker: Value =
            serde_json::from_str(R4_BASIC_TOPIC_MARKER).expect("static marker JSON is well-formed");
        let rows = client
            .query(
                "SELECT tenant_id, resource_type, id, fhir_version, data
                 FROM resources
                 WHERE resource_type = ANY($1)
                   AND is_deleted = FALSE
                   AND (resource_type <> 'Basic' OR data->'code' @> $2)
                 ORDER BY tenant_id, resource_type, id",
                &[&types, &marker],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to list hydration resources: {}", e)))?;
        Ok(rows.iter().map(hydrated_from_row).collect())
    }

    async fn get_current(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<Option<HydratedResource>> {
        let client = self.get_client().await?;
        let row = client
            .query_opt(
                "SELECT tenant_id, resource_type, id, fhir_version, data
                 FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3
                   AND is_deleted = FALSE",
                &[&tenant.tenant_id().as_str(), &resource_type, &resource_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to read hydration resource: {}", e)))?;
        Ok(row.as_ref().map(hydrated_from_row))
    }
}
