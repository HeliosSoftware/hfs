//! ResourceStorage and VersionedStorage implementations for PostgreSQL.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_fhir::FhirVersion;
use serde_json::Value;

use crate::core::history::{
    DifferentialHistoryProvider, HistoryEntry, HistoryMethod, HistoryPage, HistoryParams,
    InstanceHistoryProvider, SystemHistoryProvider, TypeHistoryProvider,
};
use crate::core::transaction::{
    BundleEntry, BundleEntryResult, BundleMethod, BundleProvider, BundleResult, BundleType,
};
use crate::core::{
    ConditionalCreateResult, ConditionalDeleteResult, ConditionalStorage, ConditionalUpdateResult,
    PurgableStorage, ResourceStorage, SearchProvider, VersionedStorage, bundle_if_match_gate,
    if_match_field_satisfied, normalize_etag,
};
use crate::error::TransactionError;
use crate::error::{
    BackendError, ConcurrencyError, QueryErrorExt, ResourceError, StorageError, StorageResult,
};
use crate::search::reindex::{ReindexSource, ReindexTarget, ResourcePage};
use crate::tenant::{Operation, TenantContext};
use crate::types::Pagination;
use crate::types::{CursorValue, Page, PageCursor, PageInfo, StoredResource};
use crate::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

use super::PostgresBackend;
use super::search::writer::PostgresSearchIndexWriter;

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "postgres".to_string(),
        message,
        source: None,
    })
}

#[allow(dead_code)]
fn serialization_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::SerializationError { message })
}

/// Extracts the `value[x]` payload from a FHIRPath Patch `Parameters.part`
/// entry whose `name` is `"value"`. Returns the value of the first key
/// matching `value[A-Z]…` (e.g. `valueString`, `valueQuantity`,
/// `valueReference`), so every FHIR polymorphic variant is accepted rather
/// than only the handful the patch handler used to special-case.
fn extract_part_value(part: &Value) -> Option<Value> {
    part.as_object()?.iter().find_map(|(k, v)| {
        let suffix = k.strip_prefix("value")?;
        suffix
            .chars()
            .next()?
            .is_ascii_uppercase()
            .then(|| v.clone())
    })
}

#[async_trait]
impl ResourceStorage for PostgresBackend {
    fn backend_name(&self) -> &'static str {
        "postgres"
    }

    async fn readiness_check(&self) -> Result<(), BackendError> {
        <Self as crate::core::Backend>::health_check(self).await
    }

    fn bulk_write_concurrency(&self) -> usize {
        // Bulk seeding is round-trip bound; the pool absorbs parallel writers.
        8
    }

    fn is_cluster_shared(&self) -> bool {
        true
    }

    fn sof_runner(&self) -> Option<std::sync::Arc<dyn crate::core::sof_runner::SofRunner>> {
        use crate::sof::postgres::PgInDbRunner;
        Some(std::sync::Arc::new(PgInDbRunner::new(self.pool())))
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        tenant.check_permission(Operation::Create, resource_type)?;

        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Extract or generate ID
        let id = resource
            .get("id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        // Check if resource already exists
        let exists = client
            .query_opt(
                "SELECT 1 FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to check existence: {}", e)))?;

        if exists.is_some() {
            return Err(StorageError::Resource(ResourceError::AlreadyExists {
                resource_type: resource_type.to_string(),
                id: id.clone(),
            }));
        }

        // Ensure the resource has correct type and id
        let mut resource = resource;
        if let Some(obj) = resource.as_object_mut() {
            obj.insert(
                "resourceType".to_string(),
                Value::String(resource_type.to_string()),
            );
            obj.insert("id".to_string(), Value::String(id.clone()));
        }

        let now = Utc::now();
        let version_id = "1";
        let fhir_version_str = fhir_version.as_mime_param();
        let is_deleted = false;

        // Insert the resource
        client
            .execute(
                "INSERT INTO resources (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&tenant_id, &resource_type, &id, &version_id, &resource, &now, &is_deleted, &fhir_version_str],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to insert resource: {}", e)))?;

        // Insert into history
        client
            .execute(
                "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&tenant_id, &resource_type, &id, &version_id, &resource, &now, &is_deleted, &fhir_version_str],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to insert history: {}", e)))?;

        // Index the resource for search
        self.index_resource(&client, tenant_id, resource_type, &id, &resource)
            .await?;

        // An *active* SearchParameter write changes a tenant's overlay: reload
        // the stored cache and drop the per-tenant registries so they rebuild.
        // Draft copies (the seeded spec set) never overlay, so skip them and
        // avoid an O(n²) reload storm during bulk seeding.
        if resource_type == "SearchParameter"
            && crate::search::search_parameter_create_affects_overlay(&resource)
        {
            if let Err(e) = self.reload_stored_cache().await {
                tracing::warn!("SearchParameter cache reload failed: {e}");
            }
        }

        // Return the stored resource with updated metadata
        Ok(StoredResource::from_storage(
            resource_type,
            &id,
            version_id,
            tenant.tenant_id().clone(),
            resource,
            now,
            now,
            None,
            fhir_version,
        ))
    }

    async fn create_or_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        // Check if exists
        match self.read(tenant, resource_type, id).await {
            // Update existing (preserves original FHIR version)
            Ok(Some(current)) => {
                let updated = self.update(tenant, &current, resource).await?;
                Ok((updated, false))
            }
            // Create new with specific ID
            Ok(None) => {
                let mut resource = resource;
                if let Some(obj) = resource.as_object_mut() {
                    obj.insert("id".to_string(), Value::String(id.to_string()));
                }
                let created = self
                    .create(tenant, resource_type, resource, fhir_version)
                    .await?;
                Ok((created, true))
            }
            // A deleted resource is brought back to life by a subsequent update
            // (FHIR http.html#delete), continuing the existing version chain
            // rather than being rejected with `Gone`.
            Err(StorageError::Resource(ResourceError::Gone { .. })) => {
                let restored = self
                    .restore_deleted(tenant, resource_type, id, resource)
                    .await?;
                Ok((restored, true))
            }
            Err(e) => Err(e),
        }
    }

    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let row = client
            .query_opt(
                "SELECT version_id, data, last_updated, is_deleted, deleted_at, fhir_version
                 FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to read resource: {}", e)))?;

        match row {
            Some(row) => {
                let version_id: String = row.get(0);
                let data: Value = row.get(1);
                let last_updated: DateTime<Utc> = row.get(2);
                let is_deleted: bool = row.get(3);
                let deleted_at: Option<DateTime<Utc>> = row.get(4);
                let fhir_version_str: String = row.get(5);

                // If deleted, return Gone error
                if is_deleted {
                    return Err(StorageError::Resource(ResourceError::Gone {
                        resource_type: resource_type.to_string(),
                        id: id.to_string(),
                        deleted_at,
                    }));
                }

                let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                    .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

                Ok(Some(StoredResource::from_storage(
                    resource_type,
                    id,
                    version_id,
                    tenant.tenant_id().clone(),
                    data,
                    last_updated,
                    last_updated,
                    None,
                    fhir_version,
                )))
            }
            None => Ok(None),
        }
    }

    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        let resource_type = current.resource_type();
        tenant.check_permission(Operation::Update, resource_type)?;

        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let id = current.id();

        // Check that the resource still exists with the expected version
        let row = client
            .query_opt(
                "SELECT version_id FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND is_deleted = FALSE",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get current version: {}", e)))?;

        let actual_version = match row {
            Some(row) => row.get::<_, String>(0),
            None => {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
            }
        };

        // Check version match
        if actual_version != current.version_id() {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: current.version_id().to_string(),
                    actual_version,
                },
            ));
        }

        // Calculate new version
        let new_version: u64 = actual_version.parse().unwrap_or(0) + 1;
        let new_version_str = new_version.to_string();

        // Ensure the resource has correct type and id
        let mut resource = resource;
        if let Some(obj) = resource.as_object_mut() {
            obj.insert(
                "resourceType".to_string(),
                Value::String(resource_type.to_string()),
            );
            obj.insert("id".to_string(), Value::String(id.to_string()));
        }

        let now = Utc::now();
        let fhir_version_str = current.fhir_version().as_mime_param();
        let is_deleted = false;

        // Update the resource
        client
            .execute(
                "UPDATE resources SET version_id = $1, data = $2, last_updated = $3
                 WHERE tenant_id = $4 AND resource_type = $5 AND id = $6",
                &[
                    &new_version_str,
                    &resource,
                    &now,
                    &tenant_id,
                    &resource_type,
                    &id,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to update resource: {}", e)))?;

        // Insert into history (preserve the original FHIR version)
        client
            .execute(
                "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&tenant_id, &resource_type, &id, &new_version_str, &resource, &now, &is_deleted, &fhir_version_str],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to insert history: {}", e)))?;

        // Re-index the resource (delete old entries, add new)
        self.delete_search_index(&client, tenant_id, resource_type, id)
            .await?;
        self.index_resource(&client, tenant_id, resource_type, id, &resource)
            .await?;

        // A SearchParameter write invalidates the tenant overlays.
        if resource_type == "SearchParameter" {
            if let Err(e) = self.reload_stored_cache().await {
                tracing::warn!("SearchParameter cache reload failed: {e}");
            }
        }

        Ok(StoredResource::from_storage(
            resource_type,
            id,
            new_version_str,
            tenant.tenant_id().clone(),
            resource,
            now,
            now,
            None,
            current.fhir_version(),
        ))
    }

    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        tenant.check_permission(Operation::Delete, resource_type)?;

        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check if resource exists and get its fhir_version
        let row = client
            .query_opt(
                "SELECT version_id, data, fhir_version FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND is_deleted = FALSE",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to check resource: {}", e)))?;

        let (current_version, data, fhir_version_str) = match row {
            Some(row) => {
                let v: String = row.get(0);
                let d: Value = row.get(1);
                let f: String = row.get(2);
                (v, d, f)
            }
            None => {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
            }
        };

        let now = Utc::now();

        // Calculate new version for the deletion record
        let new_version: u64 = current_version.parse().unwrap_or(0) + 1;
        let new_version_str = new_version.to_string();
        let is_deleted = true;

        // Soft delete the resource
        client
            .execute(
                "UPDATE resources SET is_deleted = TRUE, deleted_at = $1, version_id = $2, last_updated = $1
                 WHERE tenant_id = $3 AND resource_type = $4 AND id = $5",
                &[&now, &new_version_str, &tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to delete resource: {}", e)))?;

        // Insert deletion record into history (preserve fhir_version)
        client
            .execute(
                "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&tenant_id, &resource_type, &id, &new_version_str, &data, &now, &is_deleted, &fhir_version_str],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to insert deletion history: {}", e)))?;

        // Delete search index entries (skip when search is offloaded)
        if !self.is_search_offloaded() {
            client
                .execute(
                    "DELETE FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3",
                    &[&tenant_id, &resource_type, &id],
                )
                .await
                .map_err(|e| internal_error(format!("Failed to delete search index: {}", e)))?;
        }

        // A SearchParameter delete invalidates the tenant overlays.
        if resource_type == "SearchParameter" {
            if let Err(e) = self.reload_stored_cache().await {
                tracing::warn!("SearchParameter cache reload failed: {e}");
            }
        }

        Ok(())
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let count: i64 = if let Some(rt) = resource_type {
            let row = client
                .query_one(
                    "SELECT COUNT(*) FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE",
                    &[&tenant_id, &rt],
                )
                .await
                .or_query_error("Failed to count resources")?;
            row.get(0)
        } else {
            let row = client
                .query_one(
                    "SELECT COUNT(*) FROM resources WHERE tenant_id = $1 AND is_deleted = FALSE",
                    &[&tenant_id],
                )
                .await
                .or_query_error("Failed to count resources")?;
            row.get(0)
        };

        Ok(count as u64)
    }

    async fn count_by_day(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        since: DateTime<Utc>,
    ) -> StorageResult<Vec<crate::core::DailyResourceCount>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // `last_updated` is `TIMESTAMPTZ`; normalise to UTC before truncating to a
        // calendar day so buckets are stable regardless of the session time zone.
        // The `(tenant_id, last_updated)` index supports the `>= $3` range scan.
        let rows = client
            .query(
                "SELECT (last_updated AT TIME ZONE 'UTC')::date AS day, COUNT(*)::bigint AS n \
                 FROM resources \
                 WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE \
                   AND last_updated >= $3 \
                 GROUP BY day ORDER BY day",
                &[&tenant_id, &resource_type, &since],
            )
            .await
            .or_query_error("Failed to count resources by day")?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let day: chrono::NaiveDate = row.get(0);
            let n: i64 = row.get(1);
            out.push(crate::core::DailyResourceCount {
                day,
                count: n.max(0) as u64,
            });
        }
        Ok(out)
    }

    async fn count_deltas_by_bucket(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        since: DateTime<Utc>,
        bucket_seconds: i64,
    ) -> StorageResult<Vec<crate::core::ResourceCountDelta>> {
        if bucket_seconds <= 0 {
            return Err(internal_error(
                "count_deltas_by_bucket: bucket_seconds must be positive".to_string(),
            ));
        }
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let since_bound = crate::core::bucket_floor(since, bucket_seconds);

        // Floor each version's `last_updated` to its epoch-aligned bucket:
        // epoch seconds / width, floored, scaled back, then read as a timestamptz.
        // Epoch arithmetic is timezone-independent, so buckets are stable whatever
        // the session TimeZone. The `(tenant_id, last_updated)` history index
        // supports the `>= $3` range scan. Delta rule per the trait doc: creation
        // `+1`, delete `-1`, plain update `0`.
        //
        // `$4::bigint` is cast explicitly: `EXTRACT(EPOCH FROM ...)` is `numeric`, so
        // without it Postgres infers the parameter as `numeric` too and rejects the
        // `i64` we bind.
        let rows = client
            .query(
                "SELECT to_timestamp( \
                          (FLOOR(EXTRACT(EPOCH FROM last_updated) / $4::bigint) * $4::bigint) \
                          ::double precision \
                        ) AS bucket, \
                        SUM(CASE WHEN is_deleted THEN -1 \
                                 WHEN version_id = '1' THEN 1 \
                                 ELSE 0 END)::bigint AS delta \
                 FROM resource_history \
                 WHERE tenant_id = $1 AND resource_type = $2 AND last_updated >= $3 \
                 GROUP BY bucket \
                 HAVING SUM(CASE WHEN is_deleted THEN -1 \
                                 WHEN version_id = '1' THEN 1 \
                                 ELSE 0 END) <> 0 \
                 ORDER BY bucket",
                &[&tenant_id, &resource_type, &since_bound, &bucket_seconds],
            )
            .await
            .or_query_error("Failed to count resource deltas")?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let bucket_start: DateTime<Utc> = row.get(0);
            let delta: i64 = row.get(1);
            out.push(crate::core::ResourceCountDelta {
                bucket_start,
                delta,
            });
        }
        Ok(out)
    }

    async fn activity_histogram(
        &self,
        tenant: &TenantContext,
        since: DateTime<Utc>,
    ) -> StorageResult<Vec<crate::core::ActivityCell>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Normalise to UTC, then EXTRACT weekday (DOW: 0=Sunday..6=Saturday) and
        // hour (0..23) so the grid matches the SQL/Mongo backends and JS's
        // `Date.getDay()`. The `(tenant_id, last_updated)` history index backs
        // the `>= $2` range scan.
        let rows = client
            .query(
                "SELECT EXTRACT(DOW FROM (last_updated AT TIME ZONE 'UTC'))::int AS wd, \
                        EXTRACT(HOUR FROM (last_updated AT TIME ZONE 'UTC'))::int AS hr, \
                        COUNT(*)::bigint AS n \
                 FROM resource_history \
                 WHERE tenant_id = $1 AND last_updated >= $2 \
                 GROUP BY wd, hr",
                &[&tenant_id, &since],
            )
            .await
            .or_query_error("Failed to compute activity histogram")?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let wd: i32 = row.get(0);
            let hr: i32 = row.get(1);
            let n: i64 = row.get(2);
            out.push(crate::core::ActivityCell {
                weekday: wd.clamp(0, 6) as u8,
                hour: hr.clamp(0, 23) as u8,
                count: n.max(0) as u64,
            });
        }
        Ok(out)
    }

    async fn count_all_types(&self, tenant: &TenantContext) -> StorageResult<Vec<(String, u64)>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let rows = client
            .query(
                "SELECT resource_type, COUNT(*)::bigint FROM resources \
                 WHERE tenant_id = $1 AND is_deleted = FALSE \
                 GROUP BY resource_type",
                &[&tenant_id],
            )
            .await
            .or_query_error("Failed to count all types")?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let rt: String = row.get(0);
            let n: i64 = row.get(1);
            out.push((rt, n.max(0) as u64));
        }
        Ok(out)
    }

    async fn count_by_types(
        &self,
        tenant: &TenantContext,
        resource_types: &[&str],
    ) -> StorageResult<Vec<(String, u64)>> {
        // An empty `IN ()` is invalid SQL; nothing to count.
        if resource_types.is_empty() {
            return Ok(Vec::new());
        }
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Bind tenant_id as $1 and each requested type as $2, $3, ...; the type
        // names are bound as parameters, never interpolated into the SQL text.
        let placeholders = (0..resource_types.len())
            .map(|i| format!("${}", i + 2))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT resource_type, COUNT(*)::bigint FROM resources \
             WHERE tenant_id = $1 AND is_deleted = FALSE AND resource_type IN ({}) \
             GROUP BY resource_type",
            placeholders
        );

        // Owned bind values in $1..$n order: tenant first, then the requested types.
        let mut query_params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> =
            Vec::with_capacity(resource_types.len() + 1);
        query_params.push(Box::new(tenant_id.to_string()));
        for rt in resource_types {
            query_params.push(Box::new(rt.to_string()));
        }
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = query_params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client
            .query(&sql, &param_refs)
            .await
            .or_query_error("Failed to count by types")?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let rt: String = row.get(0);
            let n: i64 = row.get(1);
            out.push((rt, n.max(0) as u64));
        }
        Ok(out)
    }

    async fn count_by_tenant(&self) -> StorageResult<Vec<(String, u64)>> {
        // Cross-tenant admin aggregate (see trait docs): no tenant filter.
        let client = self.get_client().await?;
        let rows = client
            .query(
                "SELECT tenant_id, COUNT(*)::bigint FROM resources \
                 WHERE is_deleted = FALSE GROUP BY tenant_id",
                &[],
            )
            .await
            .or_query_error("Failed to count by tenant")?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let tid: String = row.get(0);
            let n: i64 = row.get(1);
            out.push((tid, n.max(0) as u64));
        }
        Ok(out)
    }

    fn supports_tenant_registry(&self) -> bool {
        true
    }

    async fn list_tenants(&self) -> StorageResult<Vec<crate::core::TenantRecord>> {
        let client = self.get_client().await?;
        let rows = client
            .query(
                "SELECT id, display_name, created_at FROM tenants \
                 ORDER BY created_at ASC, id ASC",
                &[],
            )
            .await
            .map_err(|e| internal_error(format!("query list_tenants: {e}")))?;
        Ok(rows
            .iter()
            .map(|row| crate::core::TenantRecord {
                id: row.get(0),
                display_name: row.get(1),
                created_at: row.get(2),
            })
            .collect())
    }

    async fn get_tenant(&self, id: &str) -> StorageResult<Option<crate::core::TenantRecord>> {
        let client = self.get_client().await?;
        let row = client
            .query_opt(
                "SELECT id, display_name, created_at FROM tenants WHERE id = $1",
                &[&id],
            )
            .await
            .map_err(|e| internal_error(format!("query get_tenant: {e}")))?;
        Ok(row.map(|row| crate::core::TenantRecord {
            id: row.get(0),
            display_name: row.get(1),
            created_at: row.get(2),
        }))
    }

    async fn register_tenant(
        &self,
        id: &str,
        display_name: Option<&str>,
    ) -> StorageResult<crate::core::TenantRecord> {
        let client = self.get_client().await?;
        // Plain INSERT so a duplicate id surfaces as a constraint error; the
        // admin handler pre-checks existence and returns 409, so reaching here
        // with a duplicate is a race and a 500 is acceptable.
        let row = client
            .query_one(
                "INSERT INTO tenants (id, display_name) VALUES ($1, $2) \
                 RETURNING id, display_name, created_at",
                &[&id, &display_name],
            )
            .await
            .map_err(|e| internal_error(format!("register_tenant: {e}")))?;
        Ok(crate::core::TenantRecord {
            id: row.get(0),
            display_name: row.get(1),
            created_at: row.get(2),
        })
    }

    async fn deregister_tenant(&self, id: &str) -> StorageResult<bool> {
        let client = self.get_client().await?;
        let changed = client
            .execute("DELETE FROM tenants WHERE id = $1", &[&id])
            .await
            .map_err(|e| internal_error(format!("deregister_tenant: {e}")))?;
        Ok(changed > 0)
    }

    async fn purge_tenant_data(&self, id: &str) -> StorageResult<u64> {
        let mut client = self.get_client().await?;
        let tx = client.transaction().await.or_query_error("purge begin")?;
        // Count current-version rows first (soft-deleted included) so we can
        // report what was removed.
        let removed: i64 = tx
            .query_one(
                "SELECT COUNT(*)::bigint FROM resources WHERE tenant_id = $1",
                &[&id],
            )
            .await
            .or_query_error("purge count")?
            .get(0);
        // search_index and resource_fts cascade from resources, but delete them
        // explicitly too, mirroring the purge/purge_all deletion order.
        for sql in [
            "DELETE FROM search_index WHERE tenant_id = $1",
            "DELETE FROM resource_fts WHERE tenant_id = $1",
            "DELETE FROM resource_history WHERE tenant_id = $1",
            "DELETE FROM resources WHERE tenant_id = $1",
        ] {
            tx.execute(sql, &[&id])
                .await
                .or_query_error("purge delete")?;
        }
        tx.commit().await.or_query_error("purge commit")?;
        Ok(removed.max(0) as u64)
    }
}

// ============================================================================
// Search Index Helpers
// ============================================================================

impl PostgresBackend {
    /// Brings a soft-deleted resource back to life with new content.
    ///
    /// FHIR permits a deleted resource to be restored by a subsequent update
    /// ([http.html#delete](https://hl7.org/fhir/http.html#delete)), so a `PUT`
    /// onto a deleted id must succeed instead of failing with `Gone`. The
    /// restored resource continues the existing version chain (the deletion
    /// record keeps its version, the restore gets the next one) and keeps the
    /// FHIR version the resource was originally stored under.
    ///
    /// Returns `NotFound` if no deleted row is present — the caller has already
    /// established one exists, so that only happens under a concurrent write.
    async fn restore_deleted(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        tenant.check_permission(Operation::Update, resource_type)?;

        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let row = client
            .query_opt(
                "SELECT version_id, fhir_version FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND is_deleted = TRUE",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to read deleted resource: {}", e)))?;

        let (deleted_version, fhir_version_str) = match row {
            Some(row) => (row.get::<_, String>(0), row.get::<_, String>(1)),
            None => {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
            }
        };

        let new_version: u64 = deleted_version.parse().unwrap_or(0) + 1;
        let new_version_str = new_version.to_string();

        // Ensure the resource has correct type and id
        let mut resource = resource;
        if let Some(obj) = resource.as_object_mut() {
            obj.insert(
                "resourceType".to_string(),
                Value::String(resource_type.to_string()),
            );
            obj.insert("id".to_string(), Value::String(id.to_string()));
        }

        let now = Utc::now();
        let is_deleted = false;

        client
            .execute(
                "UPDATE resources
                 SET version_id = $1, data = $2, last_updated = $3, is_deleted = FALSE, deleted_at = NULL
                 WHERE tenant_id = $4 AND resource_type = $5 AND id = $6",
                &[
                    &new_version_str,
                    &resource,
                    &now,
                    &tenant_id,
                    &resource_type,
                    &id,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to restore resource: {}", e)))?;

        client
            .execute(
                "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&tenant_id, &resource_type, &id, &new_version_str, &resource, &now, &is_deleted, &fhir_version_str],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to insert restore history: {}", e)))?;

        // The delete dropped the search index entries; rebuild them for the
        // resource that is live again.
        self.delete_search_index(&client, tenant_id, resource_type, id)
            .await?;
        self.index_resource(&client, tenant_id, resource_type, id, &resource)
            .await?;

        // A restored SearchParameter re-enters the tenant overlays.
        if resource_type == "SearchParameter" {
            if let Err(e) = self.reload_stored_cache().await {
                tracing::warn!("SearchParameter cache reload failed: {e}");
            }
        }

        let fhir_version = FhirVersion::from_storage(&fhir_version_str)
            .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

        Ok(StoredResource::from_storage(
            resource_type,
            id,
            new_version_str,
            tenant.tenant_id().clone(),
            resource,
            now,
            now,
            None,
            fhir_version,
        ))
    }

    /// Index a resource for search.
    ///
    /// This method uses the SearchParameterExtractor to dynamically extract
    /// searchable values based on the configured SearchParameterRegistry.
    pub(crate) async fn index_resource(
        &self,
        client: &deadpool_postgres::Client,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> StorageResult<()> {
        // When search is offloaded to a secondary backend, skip local indexing
        if self.is_search_offloaded() {
            return Ok(());
        }

        // Delete existing index entries
        client
            .execute(
                "DELETE FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3",
                &[&tenant_id, &resource_type, &resource_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to clear search index: {}", e)))?;

        // Extract values using the registry-driven extractor
        match self
            .tenant_extractor(tenant_id)
            .extract(resource, resource_type)
        {
            Ok(values) => {
                let mut count = 0;
                for value in values {
                    PostgresSearchIndexWriter::write_entry(
                        client,
                        tenant_id,
                        resource_type,
                        resource_id,
                        &value,
                    )
                    .await?;
                    count += 1;
                }
                tracing::debug!(
                    "Dynamically indexed {} values for {}/{}",
                    count,
                    resource_type,
                    resource_id
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Dynamic extraction failed for {}/{}: {}. Using minimal fallback (_id, _lastUpdated only).",
                    resource_type,
                    resource_id,
                    e
                );
                // Fall back to minimal extraction (just _id and _lastUpdated)
                self.index_minimal_fallback(
                    client,
                    tenant_id,
                    resource_type,
                    resource_id,
                    resource,
                )
                .await?;
            }
        }

        // Index any contained resources for `_contained` search.
        self.index_contained_resources(client, tenant_id, resource_type, resource_id, resource)
            .await?;

        // Index FTS content for _text and _content searches
        self.index_fts_content(client, tenant_id, resource_type, resource_id, resource)
            .await?;

        Ok(())
    }

    /// Extracts and indexes a container's `contained[]` resources for
    /// `_contained` search. Each contained resource's search values are written
    /// as `is_contained = TRUE` rows whose `resource_type` / `resource_id`
    /// identify the container. Returns the number of entries written.
    async fn index_contained_resources(
        &self,
        client: &deadpool_postgres::Client,
        tenant_id: &str,
        container_type: &str,
        container_id: &str,
        resource: &Value,
    ) -> StorageResult<usize> {
        let mut count = 0;
        let container = (container_type, container_id);
        for contained in self.tenant_extractor(tenant_id).extract_contained(resource) {
            for value in &contained.values {
                PostgresSearchIndexWriter::write_contained_entry(
                    client,
                    tenant_id,
                    container,
                    (&contained.contained_type, &contained.local_id),
                    value,
                )
                .await?;
                count += 1;
            }
        }
        Ok(count)
    }

    /// Index full-text search content for _text and _content searches.
    ///
    /// Populates the resource_fts table using PostgreSQL tsvector/tsquery.
    async fn index_fts_content(
        &self,
        client: &deadpool_postgres::Client,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> StorageResult<()> {
        // Check if FTS table exists
        let fts_exists = client
            .query_opt(
                "SELECT 1 FROM information_schema.tables WHERE table_name = 'resource_fts'",
                &[],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to check FTS table: {}", e)))?;

        if fts_exists.is_none() {
            return Ok(());
        }

        // Extract searchable content
        let content = extract_searchable_content(resource);

        if content.is_empty() {
            return Ok(());
        }

        // Delete existing FTS entry first
        let _ = client
            .execute(
                "DELETE FROM resource_fts WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3",
                &[&tenant_id, &resource_type, &resource_id],
            )
            .await;

        // Insert into FTS table (the trigger will populate tsvector columns)
        client
            .execute(
                "INSERT INTO resource_fts (resource_id, resource_type, tenant_id, narrative_text, full_content)
                 VALUES ($1, $2, $3, $4, $5)",
                &[
                    &resource_id,
                    &resource_type,
                    &tenant_id,
                    &content.narrative,
                    &content.full_content,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to insert FTS content: {}", e)))?;

        Ok(())
    }

    /// Index minimal fallback search parameters.
    ///
    /// Only indexes `_id` and `_lastUpdated` when dynamic extraction fails.
    async fn index_minimal_fallback(
        &self,
        client: &deadpool_postgres::Client,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> StorageResult<()> {
        // _id - always available from resource.id
        if let Some(id) = resource.get("id").and_then(|v| v.as_str()) {
            client
                .execute(
                    "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_token_code)
                     VALUES ($1, $2, $3, '_id', $4)",
                    &[&tenant_id, &resource_type, &resource_id, &id],
                )
                .await
                .map_err(|e| internal_error(format!("Failed to insert _id index: {}", e)))?;
        }

        // _lastUpdated - from resource.meta.lastUpdated
        if let Some(last_updated) = resource
            .get("meta")
            .and_then(|m| m.get("lastUpdated"))
            .and_then(|v| v.as_str())
        {
            let normalized = normalize_date_for_pg(last_updated);
            client
                .execute(
                    "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_date)
                     VALUES ($1, $2, $3, '_lastUpdated', $4::timestamptz)",
                    &[&tenant_id, &resource_type, &resource_id, &normalized],
                )
                .await
                .map_err(|e| {
                    internal_error(format!("Failed to insert _lastUpdated index: {}", e))
                })?;
        }

        Ok(())
    }

    /// Delete search index entries for a resource.
    /// Removes a resource's search entries, returning how many `search_index`
    /// rows were deleted.
    pub(crate) async fn delete_search_index(
        &self,
        client: &deadpool_postgres::Client,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<u64> {
        // When search is offloaded to a secondary backend, skip local index cleanup
        if self.is_search_offloaded() {
            return Ok(0);
        }

        // Delete from main search index
        let deleted = client
            .execute(
                "DELETE FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3",
                &[&tenant_id, &resource_type, &resource_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to delete search index: {}", e)))?;

        // Delete from FTS table if it exists
        let _ = client
            .execute(
                "DELETE FROM resource_fts WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3",
                &[&tenant_id, &resource_type, &resource_id],
            )
            .await;

        Ok(deleted)
    }
}

// ============================================================================
// VersionedStorage Implementation
// ============================================================================

#[async_trait]
impl VersionedStorage for PostgresBackend {
    async fn vread(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        version_id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let row = client
            .query_opt(
                "SELECT data, last_updated, is_deleted, fhir_version
                 FROM resource_history
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND version_id = $4",
                &[&tenant_id, &resource_type, &id, &version_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to read version: {}", e)))?;

        match row {
            Some(row) => {
                let data: Value = row.get(0);
                let last_updated: DateTime<Utc> = row.get(1);
                let is_deleted: bool = row.get(2);
                let fhir_version_str: String = row.get(3);

                // For deleted versions, use last_updated as deleted_at
                let deleted_at = if is_deleted { Some(last_updated) } else { None };

                let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                    .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

                Ok(Some(StoredResource::from_storage(
                    resource_type,
                    id,
                    version_id,
                    tenant.tenant_id().clone(),
                    data,
                    last_updated,
                    last_updated,
                    deleted_at,
                    fhir_version,
                )))
            }
            None => Ok(None),
        }
    }

    async fn update_with_match(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        // Read current resource
        let current = self.read(tenant, resource_type, id).await?.ok_or_else(|| {
            StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            })
        })?;

        // Check version match. `expected_version` may arrive in any ETag
        // spelling (`W/"1"`, `"1"`, `1`), so normalise both sides — same as the
        // MongoDB and S3 backends.
        // `expected_version` is the client's `If-Match` field value, which is a
        // LIST and is satisfied when any listed tag matches (issue #311).
        if !if_match_field_satisfied(expected_version, current.version_id()) {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: normalize_etag(expected_version).to_string(),
                    actual_version: current.version_id().to_string(),
                },
            ));
        }

        // Perform update
        self.update(tenant, &current, resource).await
    }

    async fn delete_with_match(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
    ) -> StorageResult<()> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check version match
        let row = client
            .query_opt(
                "SELECT version_id FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND is_deleted = FALSE",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get current version: {}", e)))?;

        let current_version = match row {
            Some(row) => row.get::<_, String>(0),
            None => {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
            }
        };

        let expected = normalize_etag(expected_version);
        if !if_match_field_satisfied(expected_version, &current_version) {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: expected.to_string(),
                    actual_version: current_version,
                },
            ));
        }

        // Perform delete
        self.delete(tenant, resource_type, id).await
    }

    async fn list_versions(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Vec<String>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let rows = client
            .query(
                "SELECT version_id FROM resource_history
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3
                 ORDER BY CAST(version_id AS INTEGER) ASC",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to list versions: {}", e)))?;

        let versions: Vec<String> = rows.iter().map(|row| row.get(0)).collect();
        Ok(versions)
    }
}

// ============================================================================
// InstanceHistoryProvider Implementation
// ============================================================================

#[async_trait]
impl InstanceHistoryProvider for PostgresBackend {
    async fn history_instance(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        params: &HistoryParams,
    ) -> StorageResult<HistoryPage> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Build the query with filters
        let mut sql = String::from(
            "SELECT version_id, data, last_updated, is_deleted, fhir_version
             FROM resource_history
             WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
        );
        let mut param_index: usize = 4;
        let mut query_params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = vec![
            Box::new(tenant_id.to_string()),
            Box::new(resource_type.to_string()),
            Box::new(id.to_string()),
        ];

        // Apply deleted filter
        if !params.include_deleted {
            sql.push_str(" AND is_deleted = FALSE");
        }

        // Apply since filter
        if let Some(since) = &params.since {
            sql.push_str(&format!(" AND last_updated >= ${}", param_index));
            query_params.push(Box::new(*since));
            param_index += 1;
        }

        // Apply before filter
        if let Some(before) = &params.before {
            sql.push_str(&format!(" AND last_updated < ${}", param_index));
            query_params.push(Box::new(*before));
            param_index += 1;
        }

        // Apply cursor filter if present
        if let Some(cursor) = params.pagination.cursor_value() {
            if let Some(CursorValue::String(version_str)) = cursor.sort_values().first() {
                if let Ok(version_int) = version_str.parse::<i64>() {
                    sql.push_str(&format!(
                        " AND CAST(version_id AS INTEGER) < ${}",
                        param_index
                    ));
                    query_params.push(Box::new(version_int));
                    param_index += 1;
                }
            }
        }

        // Order by version descending (newest first) and limit
        let limit = params.pagination.count as i64 + 1; // +1 to detect if there are more
        sql.push_str(&format!(
            " ORDER BY CAST(version_id AS INTEGER) DESC LIMIT ${}",
            param_index
        ));
        query_params.push(Box::new(limit));

        // Execute the query
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = query_params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client
            .query(&sql, &param_refs)
            .await
            .or_query_error("Failed to query history")?;

        let mut entries = Vec::new();
        let mut last_version: Option<String> = None;

        for row in &rows {
            // Stop if we've collected enough items (we fetched count+1 to detect more)
            if entries.len() >= params.pagination.count as usize {
                break;
            }

            let version_id: String = row.get(0);
            let data: Value = row.get(1);
            let last_updated: DateTime<Utc> = row.get(2);
            let is_deleted: bool = row.get(3);
            let fhir_version_str: String = row.get(4);

            let deleted_at = if is_deleted { Some(last_updated) } else { None };

            let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

            let resource = StoredResource::from_storage(
                resource_type,
                id,
                &version_id,
                tenant.tenant_id().clone(),
                data,
                last_updated,
                last_updated,
                deleted_at,
                fhir_version,
            );

            // Determine the method based on version and deletion status
            let method = if is_deleted {
                HistoryMethod::Delete
            } else if version_id == "1" {
                HistoryMethod::Post
            } else {
                HistoryMethod::Put
            };

            last_version = Some(version_id);

            entries.push(HistoryEntry {
                resource,
                method,
                timestamp: last_updated,
            });
        }

        // Determine if there are more results
        let has_more = rows.len() > params.pagination.count as usize;

        // Build page info
        let page_info = if let (true, Some(version)) = (has_more, last_version) {
            let cursor = PageCursor::new(vec![CursorValue::String(version)], id.to_string());
            PageInfo::with_next(cursor)
        } else {
            PageInfo::end()
        };

        Ok(Page::new(entries, page_info))
    }

    async fn history_instance_count(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let row = client
            .query_one(
                "SELECT COUNT(*) FROM resource_history
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .or_query_error("Failed to count history")?;

        let count: i64 = row.get(0);
        Ok(count as u64)
    }

    async fn delete_instance_history(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // First, verify the resource exists
        let exists = client
            .query_opt(
                "SELECT 1 FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to check resource existence: {}", e)))?;

        if exists.is_none() {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        }

        // Get the current version from resources table (to preserve it)
        let current_row = client
            .query_one(
                "SELECT version_id FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get current version: {}", e)))?;

        let current_version: String = current_row.get(0);

        // Delete all history entries EXCEPT the current version
        let deleted = client
            .execute(
                "DELETE FROM resource_history
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND version_id != $4",
                &[&tenant_id, &resource_type, &id, &current_version],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to delete history: {}", e)))?;

        Ok(deleted)
    }

    async fn delete_version(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        version_id: &str,
    ) -> StorageResult<()> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // First, get the current version to ensure we're not deleting it
        let current_row = client
            .query_opt(
                "SELECT version_id FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get current version: {}", e)))?;

        let current_version = match current_row {
            Some(row) => row.get::<_, String>(0),
            None => {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
            }
        };

        // Prevent deletion of the current version
        if version_id == current_version {
            return Err(StorageError::Validation(
                crate::error::ValidationError::InvalidResource {
                    message: format!(
                        "Cannot delete current version {} of {}/{}. Use DELETE on the resource instead.",
                        version_id, resource_type, id
                    ),
                    details: vec![],
                },
            ));
        }

        // Check if the version exists in history
        let version_exists = client
            .query_opt(
                "SELECT 1 FROM resource_history
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND version_id = $4",
                &[&tenant_id, &resource_type, &id, &version_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to check version existence: {}", e)))?;

        if version_exists.is_none() {
            return Err(StorageError::Resource(ResourceError::VersionNotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
                version_id: version_id.to_string(),
            }));
        }

        // Delete the specific version
        client
            .execute(
                "DELETE FROM resource_history
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND version_id = $4",
                &[&tenant_id, &resource_type, &id, &version_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to delete version: {}", e)))?;

        Ok(())
    }
}

// ============================================================================
// TypeHistoryProvider Implementation
// ============================================================================

#[async_trait]
impl TypeHistoryProvider for PostgresBackend {
    async fn history_type(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        params: &HistoryParams,
    ) -> StorageResult<HistoryPage> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Build the query with filters
        let mut sql = String::from(
            "SELECT id, version_id, data, last_updated, is_deleted, fhir_version
             FROM resource_history
             WHERE tenant_id = $1 AND resource_type = $2",
        );
        let mut param_index: usize = 3;
        let mut query_params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = vec![
            Box::new(tenant_id.to_string()),
            Box::new(resource_type.to_string()),
        ];

        // Apply deleted filter
        if !params.include_deleted {
            sql.push_str(" AND is_deleted = FALSE");
        }

        // Apply since filter
        if let Some(since) = &params.since {
            sql.push_str(&format!(" AND last_updated >= ${}", param_index));
            query_params.push(Box::new(*since));
            param_index += 1;
        }

        // Apply before filter
        if let Some(before) = &params.before {
            sql.push_str(&format!(" AND last_updated < ${}", param_index));
            query_params.push(Box::new(*before));
            param_index += 1;
        }

        // Apply cursor filter if present
        if let Some(cursor) = params.pagination.cursor_value() {
            let sort_values = cursor.sort_values();
            if sort_values.len() >= 2 {
                if let (
                    Some(CursorValue::String(timestamp)),
                    Some(CursorValue::String(resource_id)),
                ) = (sort_values.first(), sort_values.get(1))
                {
                    sql.push_str(&format!(
                        " AND (last_updated < ${}::timestamptz OR (last_updated = ${}::timestamptz AND id < ${}))",
                        param_index, param_index, param_index + 1
                    ));
                    query_params.push(Box::new(timestamp.clone()));
                    query_params.push(Box::new(resource_id.clone()));
                    param_index += 2;
                }
            }
        }

        // Order by last_updated descending (newest first), then by id for consistency
        let limit = params.pagination.count as i64 + 1;
        sql.push_str(&format!(
            " ORDER BY last_updated DESC, id DESC, CAST(version_id AS INTEGER) DESC LIMIT ${}",
            param_index
        ));
        query_params.push(Box::new(limit));

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = query_params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client
            .query(&sql, &param_refs)
            .await
            .or_query_error("Failed to query type history")?;

        let mut entries = Vec::new();
        let mut last_entry: Option<(String, String)> = None; // (last_updated, id)

        for row in &rows {
            if entries.len() >= params.pagination.count as usize {
                break;
            }

            let row_id: String = row.get(0);
            let version_id: String = row.get(1);
            let data: Value = row.get(2);
            let last_updated: DateTime<Utc> = row.get(3);
            let is_deleted: bool = row.get(4);
            let fhir_version_str: String = row.get(5);

            let deleted_at = if is_deleted { Some(last_updated) } else { None };

            let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

            let resource = StoredResource::from_storage(
                resource_type,
                &row_id,
                &version_id,
                tenant.tenant_id().clone(),
                data,
                last_updated,
                last_updated,
                deleted_at,
                fhir_version,
            );

            let method = if is_deleted {
                HistoryMethod::Delete
            } else if version_id == "1" {
                HistoryMethod::Post
            } else {
                HistoryMethod::Put
            };

            last_entry = Some((last_updated.to_rfc3339(), row_id));

            entries.push(HistoryEntry {
                resource,
                method,
                timestamp: last_updated,
            });
        }

        // Determine if there are more results
        let has_more = rows.len() > params.pagination.count as usize;

        // Build page info
        let page_info = if let (true, Some((timestamp, id))) = (has_more, last_entry) {
            let cursor = PageCursor::new(
                vec![CursorValue::String(timestamp), CursorValue::String(id)],
                resource_type.to_string(),
            );
            PageInfo::with_next(cursor)
        } else {
            PageInfo::end()
        };

        Ok(Page::new(entries, page_info))
    }

    async fn history_type_count(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let row = client
            .query_one(
                "SELECT COUNT(*) FROM resource_history
                 WHERE tenant_id = $1 AND resource_type = $2",
                &[&tenant_id, &resource_type],
            )
            .await
            .or_query_error("Failed to count type history")?;

        let count: i64 = row.get(0);
        Ok(count as u64)
    }
}

// ============================================================================
// SystemHistoryProvider Implementation
// ============================================================================

#[async_trait]
impl SystemHistoryProvider for PostgresBackend {
    async fn history_system(
        &self,
        tenant: &TenantContext,
        params: &HistoryParams,
    ) -> StorageResult<HistoryPage> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Build the query with filters
        let mut sql = String::from(
            "SELECT resource_type, id, version_id, data, last_updated, is_deleted, fhir_version
             FROM resource_history
             WHERE tenant_id = $1",
        );
        let mut param_index: usize = 2;
        let mut query_params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> =
            vec![Box::new(tenant_id.to_string())];

        // Apply deleted filter
        if !params.include_deleted {
            sql.push_str(" AND is_deleted = FALSE");
        }

        // Apply since filter
        if let Some(since) = &params.since {
            sql.push_str(&format!(" AND last_updated >= ${}", param_index));
            query_params.push(Box::new(*since));
            param_index += 1;
        }

        // Apply before filter
        if let Some(before) = &params.before {
            sql.push_str(&format!(" AND last_updated < ${}", param_index));
            query_params.push(Box::new(*before));
            param_index += 1;
        }

        // Apply cursor filter if present
        if let Some(cursor) = params.pagination.cursor_value() {
            let sort_values = cursor.sort_values();
            if sort_values.len() >= 3 {
                if let (
                    Some(CursorValue::String(timestamp)),
                    Some(CursorValue::String(res_type)),
                    Some(CursorValue::String(res_id)),
                ) = (sort_values.first(), sort_values.get(1), sort_values.get(2))
                {
                    sql.push_str(&format!(
                        " AND (last_updated < ${}::timestamptz OR (last_updated = ${}::timestamptz AND (resource_type < ${} OR (resource_type = ${} AND id < ${}))))",
                        param_index, param_index, param_index + 1, param_index + 1, param_index + 2
                    ));
                    query_params.push(Box::new(timestamp.clone()));
                    query_params.push(Box::new(res_type.clone()));
                    query_params.push(Box::new(res_id.clone()));
                    param_index += 3;
                }
            }
        }

        // Order by last_updated descending (newest first)
        let limit = params.pagination.count as i64 + 1;
        sql.push_str(&format!(
            " ORDER BY last_updated DESC, resource_type DESC, id DESC, CAST(version_id AS INTEGER) DESC LIMIT ${}",
            param_index
        ));
        query_params.push(Box::new(limit));

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = query_params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client
            .query(&sql, &param_refs)
            .await
            .or_query_error("Failed to query system history")?;

        let mut entries = Vec::new();
        let mut last_entry: Option<(String, String, String)> = None;

        for row in &rows {
            if entries.len() >= params.pagination.count as usize {
                break;
            }

            let row_resource_type: String = row.get(0);
            let row_id: String = row.get(1);
            let version_id: String = row.get(2);
            let data: Value = row.get(3);
            let last_updated: DateTime<Utc> = row.get(4);
            let is_deleted: bool = row.get(5);
            let fhir_version_str: String = row.get(6);

            let deleted_at = if is_deleted { Some(last_updated) } else { None };

            let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

            let resource = StoredResource::from_storage(
                &row_resource_type,
                &row_id,
                &version_id,
                tenant.tenant_id().clone(),
                data,
                last_updated,
                last_updated,
                deleted_at,
                fhir_version,
            );

            let method = if is_deleted {
                HistoryMethod::Delete
            } else if version_id == "1" {
                HistoryMethod::Post
            } else {
                HistoryMethod::Put
            };

            last_entry = Some((last_updated.to_rfc3339(), row_resource_type, row_id));

            entries.push(HistoryEntry {
                resource,
                method,
                timestamp: last_updated,
            });
        }

        let has_more = rows.len() > params.pagination.count as usize;

        let page_info = if let (true, Some((timestamp, resource_type, id))) = (has_more, last_entry)
        {
            let cursor = PageCursor::new(
                vec![
                    CursorValue::String(timestamp),
                    CursorValue::String(resource_type),
                    CursorValue::String(id),
                ],
                "system".to_string(),
            );
            PageInfo::with_next(cursor)
        } else {
            PageInfo::end()
        };

        Ok(Page::new(entries, page_info))
    }

    async fn history_system_count(&self, tenant: &TenantContext) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let row = client
            .query_one(
                "SELECT COUNT(*) FROM resource_history WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await
            .or_query_error("Failed to count system history")?;

        let count: i64 = row.get(0);
        Ok(count as u64)
    }
}

// ============================================================================
// DifferentialHistoryProvider Implementation
// ============================================================================

#[async_trait]
impl DifferentialHistoryProvider for PostgresBackend {
    async fn modified_since(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
        since: DateTime<Utc>,
        pagination: &Pagination,
    ) -> StorageResult<Page<StoredResource>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Build query for current versions of resources modified since timestamp
        let mut sql = String::from(
            "SELECT resource_type, id, version_id, data, last_updated, fhir_version
             FROM resources
             WHERE tenant_id = $1 AND last_updated > $2 AND is_deleted = FALSE",
        );
        let mut param_index: usize = 3;
        let mut query_params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> =
            vec![Box::new(tenant_id.to_string()), Box::new(since)];

        // Filter by resource type if specified
        if let Some(rt) = resource_type {
            sql.push_str(&format!(" AND resource_type = ${}", param_index));
            query_params.push(Box::new(rt.to_string()));
            param_index += 1;
        }

        // Apply cursor filter if present
        if let Some(cursor) = pagination.cursor_value() {
            let sort_values = cursor.sort_values();
            if sort_values.len() >= 2 {
                if let (Some(CursorValue::String(timestamp)), Some(CursorValue::String(res_id))) =
                    (sort_values.first(), sort_values.get(1))
                {
                    sql.push_str(&format!(
                        " AND (last_updated > ${}::timestamptz OR (last_updated = ${}::timestamptz AND id > ${}))",
                        param_index, param_index, param_index + 1
                    ));
                    query_params.push(Box::new(timestamp.clone()));
                    query_params.push(Box::new(res_id.clone()));
                    param_index += 2;
                }
            }
        }

        // Order by last_updated ascending (oldest first for sync)
        let limit = pagination.count as i64 + 1;
        sql.push_str(&format!(
            " ORDER BY last_updated ASC, id ASC LIMIT ${}",
            param_index
        ));
        query_params.push(Box::new(limit));

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = query_params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client
            .query(&sql, &param_refs)
            .await
            .map_err(|e| internal_error(format!("Failed to query modified resources: {}", e)))?;

        let mut resources = Vec::new();
        let mut last_entry: Option<(String, String)> = None;

        for row in &rows {
            if resources.len() >= pagination.count as usize {
                break;
            }

            let row_resource_type: String = row.get(0);
            let row_id: String = row.get(1);
            let version_id: String = row.get(2);
            let data: Value = row.get(3);
            let last_updated: DateTime<Utc> = row.get(4);
            let fhir_version_str: String = row.get(5);

            let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

            let resource = StoredResource::from_storage(
                &row_resource_type,
                &row_id,
                &version_id,
                tenant.tenant_id().clone(),
                data,
                last_updated,
                last_updated,
                None,
                fhir_version,
            );

            last_entry = Some((last_updated.to_rfc3339(), row_id));
            resources.push(resource);
        }

        let has_more = rows.len() > pagination.count as usize;

        let page_info = if let (true, Some((timestamp, id))) = (has_more, last_entry) {
            let cursor = PageCursor::new(
                vec![CursorValue::String(timestamp), CursorValue::String(id)],
                "modified_since".to_string(),
            );
            PageInfo::with_next(cursor)
        } else {
            PageInfo::end()
        };

        Ok(Page::new(resources, page_info))
    }
}

// ============================================================================
// PurgableStorage Implementation
// ============================================================================

#[async_trait]
impl PurgableStorage for PostgresBackend {
    async fn purge(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check if resource exists (in any state)
        let exists = client
            .query_opt(
                "SELECT 1 FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to check resource: {}", e)))?;

        if exists.is_none() {
            // Also check history in case it was already purged from main table
            let history_exists = client
                .query_opt(
                    "SELECT 1 FROM resource_history WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                    &[&tenant_id, &resource_type, &id],
                )
                .await
                .or_query_error("Failed to check history")?;

            if history_exists.is_none() {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
            }
        }

        // Delete from search index first (due to FK constraint)
        client
            .execute(
                "DELETE FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .or_query_error("Failed to purge search index")?;

        // Delete from FTS table
        let _ = client
            .execute(
                "DELETE FROM resource_fts WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await;

        // Delete from history table (before resources due to FK)
        client
            .execute(
                "DELETE FROM resource_history WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .or_query_error("Failed to purge resource history")?;

        // Delete from resources table
        client
            .execute(
                "DELETE FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .or_query_error("Failed to purge resource")?;

        Ok(())
    }

    async fn purge_all(&self, tenant: &TenantContext, resource_type: &str) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Count how many we're about to delete
        let row = client
            .query_one(
                "SELECT COUNT(DISTINCT id) FROM resources WHERE tenant_id = $1 AND resource_type = $2",
                &[&tenant_id, &resource_type],
            )
            .await
            .or_query_error("Failed to count resources")?;
        let count: i64 = row.get(0);

        // Delete from search index first (due to FK constraint)
        client
            .execute(
                "DELETE FROM search_index WHERE tenant_id = $1 AND resource_type = $2",
                &[&tenant_id, &resource_type],
            )
            .await
            .or_query_error("Failed to purge search index")?;

        // Delete from FTS table
        let _ = client
            .execute(
                "DELETE FROM resource_fts WHERE tenant_id = $1 AND resource_type = $2",
                &[&tenant_id, &resource_type],
            )
            .await;

        // Delete from history table
        client
            .execute(
                "DELETE FROM resource_history WHERE tenant_id = $1 AND resource_type = $2",
                &[&tenant_id, &resource_type],
            )
            .await
            .or_query_error("Failed to purge resource history")?;

        // Delete from resources table
        client
            .execute(
                "DELETE FROM resources WHERE tenant_id = $1 AND resource_type = $2",
                &[&tenant_id, &resource_type],
            )
            .await
            .or_query_error("Failed to purge resources")?;

        Ok(count as u64)
    }
}

// ============================================================================
// ConditionalStorage Implementation
// ============================================================================

// Helper function to parse simple search parameters
// Supports basic formats like: identifier=X, _id=Y, name=Z
fn parse_simple_search_params(params: &str) -> Vec<(String, String)> {
    params
        .split('&')
        .filter_map(|pair| {
            let parts: Vec<&str> = pair.splitn(2, '=').collect();
            if parts.len() == 2 {
                Some((parts[0].to_string(), parts[1].to_string()))
            } else {
                None
            }
        })
        .collect()
}

#[async_trait]
impl ConditionalStorage for PostgresBackend {
    async fn conditional_create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        search_params: &str,
        fhir_version: FhirVersion,
    ) -> StorageResult<ConditionalCreateResult> {
        // Find matching resources based on search parameters
        let matches = self
            .find_matching_resources(tenant, resource_type, search_params)
            .await?;

        match matches.len() {
            0 => {
                // No match - create the resource
                let created = self
                    .create(tenant, resource_type, resource, fhir_version)
                    .await?;
                Ok(ConditionalCreateResult::Created(created))
            }
            1 => {
                // Exactly one match - return the existing resource
                Ok(ConditionalCreateResult::Exists(
                    matches.into_iter().next().unwrap(),
                ))
            }
            n => {
                // Multiple matches - error condition
                Ok(ConditionalCreateResult::MultipleMatches(n))
            }
        }
    }

    async fn conditional_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        search_params: &str,
        upsert: bool,
        fhir_version: FhirVersion,
    ) -> StorageResult<ConditionalUpdateResult> {
        // Find matching resources based on search parameters
        let matches = self
            .find_matching_resources(tenant, resource_type, search_params)
            .await?;

        match matches.len() {
            0 => {
                if upsert {
                    // No match, but upsert is true - create new resource
                    let created = self
                        .create(tenant, resource_type, resource, fhir_version)
                        .await?;
                    Ok(ConditionalUpdateResult::Created(created))
                } else {
                    // No match and no upsert
                    Ok(ConditionalUpdateResult::NoMatch)
                }
            }
            1 => {
                // Exactly one match - update it (preserves existing FHIR version)
                let existing = matches.into_iter().next().unwrap();
                let updated = self.update(tenant, &existing, resource).await?;
                Ok(ConditionalUpdateResult::Updated(updated))
            }
            n => {
                // Multiple matches - error condition
                Ok(ConditionalUpdateResult::MultipleMatches(n))
            }
        }
    }

    async fn conditional_delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
    ) -> StorageResult<ConditionalDeleteResult> {
        // Find matching resources based on search parameters
        let matches = self
            .find_matching_resources(tenant, resource_type, search_params)
            .await?;

        match matches.len() {
            0 => {
                // No match
                Ok(ConditionalDeleteResult::NoMatch)
            }
            1 => {
                // Exactly one match - delete it
                let existing = matches.into_iter().next().unwrap();
                self.delete(tenant, resource_type, existing.id()).await?;
                Ok(ConditionalDeleteResult::Deleted)
            }
            n => {
                // Multiple matches - error condition
                Ok(ConditionalDeleteResult::MultipleMatches(n))
            }
        }
    }

    async fn conditional_patch(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
        patch: &crate::core::PatchFormat,
    ) -> StorageResult<crate::core::ConditionalPatchResult> {
        use crate::core::{ConditionalPatchResult, PatchFormat};

        // Find matching resources based on search parameters
        let matches = self
            .find_matching_resources(tenant, resource_type, search_params)
            .await?;

        match matches.len() {
            0 => Ok(ConditionalPatchResult::NoMatch),
            1 => {
                // Exactly one match - apply the patch
                let existing = matches.into_iter().next().unwrap();
                let current_content = existing.content().clone();

                // Apply the patch based on format
                let patched_content = match patch {
                    PatchFormat::JsonPatch(patch_doc) => {
                        self.apply_json_patch(&current_content, patch_doc)?
                    }
                    PatchFormat::FhirPathPatch(patch_params) => {
                        self.apply_fhirpath_patch(&current_content, patch_params)?
                    }
                    PatchFormat::MergePatch(merge_doc) => {
                        self.apply_merge_patch(&current_content, merge_doc)
                    }
                };

                // Update the resource with the patched content
                let updated = self.update(tenant, &existing, patched_content).await?;
                Ok(ConditionalPatchResult::Patched(updated))
            }
            n => Ok(ConditionalPatchResult::MultipleMatches(n)),
        }
    }
}

impl PostgresBackend {
    /// Find resources matching the given search parameters.
    ///
    /// Uses the SearchProvider implementation to leverage the pre-computed search index.
    async fn find_matching_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params_str: &str,
    ) -> StorageResult<Vec<StoredResource>> {
        // Parse search parameters into (name, value) pairs
        let parsed_params = parse_simple_search_params(search_params_str);

        if parsed_params.is_empty() {
            // No search params means match all - but for conditional ops this is unusual
            return Ok(Vec::new());
        }

        // Build SearchParameter objects by looking up types from the registry
        let search_params = self.build_search_parameters(tenant, resource_type, &parsed_params)?;

        // Build a SearchQuery
        let query = SearchQuery {
            resource_type: resource_type.to_string(),
            parameters: search_params,
            count: Some(1000),
            ..Default::default()
        };

        // Use the SearchProvider implementation
        let result = <Self as SearchProvider>::search(self, tenant, &query).await?;

        Ok(result.resources.items)
    }

    /// Builds SearchParameter objects from parsed (name, value) pairs.
    fn build_search_parameters(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        params: &[(String, String)],
    ) -> StorageResult<Vec<SearchParameter>> {
        let registry_arc = self.tenant_registry(tenant.tenant_id().as_str());
        let registry = registry_arc.read();
        let mut search_params = Vec::with_capacity(params.len());

        for (name, value) in params {
            let param_type = self
                .lookup_param_type(&registry, resource_type, name)
                .unwrap_or({
                    match name.as_str() {
                        "_id" => SearchParamType::Token,
                        "_lastUpdated" => SearchParamType::Date,
                        "_tag" | "_profile" | "_security" => SearchParamType::Token,
                        "identifier" => SearchParamType::Token,
                        "patient" | "subject" | "encounter" | "performer" | "author"
                        | "requester" | "recorder" | "asserter" | "practitioner"
                        | "organization" | "location" | "device" => SearchParamType::Reference,
                        _ => SearchParamType::String,
                    }
                });

            search_params.push(SearchParameter {
                name: name.clone(),
                param_type,
                modifier: None,
                values: vec![SearchValue::parse(value)],
                chain: vec![],
                components: vec![],
            });
        }

        Ok(search_params)
    }

    /// Looks up a search parameter type from the registry.
    fn lookup_param_type(
        &self,
        registry: &crate::search::SearchParameterRegistry,
        resource_type: &str,
        param_name: &str,
    ) -> Option<SearchParamType> {
        if let Some(def) = registry.get_param(resource_type, param_name) {
            return Some(def.param_type);
        }
        if let Some(def) = registry.get_param("Resource", param_name) {
            return Some(def.param_type);
        }
        None
    }

    // ========================================================================
    // Patch Helper Methods
    // ========================================================================

    /// Applies a JSON Patch (RFC 6902) to a resource.
    fn apply_json_patch(&self, resource: &Value, patch_doc: &Value) -> StorageResult<Value> {
        use crate::error::ValidationError;

        let patch: json_patch::Patch = serde_json::from_value(patch_doc.clone()).map_err(|e| {
            StorageError::Validation(ValidationError::InvalidResource {
                message: format!("Invalid JSON Patch document: {}", e),
                details: vec![],
            })
        })?;

        let mut patched = resource.clone();
        json_patch::patch(&mut patched, &patch).map_err(|e| {
            StorageError::Validation(ValidationError::InvalidResource {
                message: format!("Failed to apply JSON Patch: {}", e),
                details: vec![],
            })
        })?;

        Ok(patched)
    }

    /// Applies a FHIRPath Patch to a resource.
    fn apply_fhirpath_patch(&self, resource: &Value, patch_params: &Value) -> StorageResult<Value> {
        use crate::error::ValidationError;

        let parameter = patch_params.get("parameter").and_then(|p| p.as_array());
        if parameter.is_none() {
            return Err(StorageError::Validation(ValidationError::InvalidResource {
                message: "FHIRPath Patch must have a 'parameter' array".to_string(),
                details: vec![],
            }));
        }

        let mut patched = resource.clone();

        for operation in parameter.unwrap() {
            let parts = operation.get("part").and_then(|p| p.as_array());
            if parts.is_none() {
                continue;
            }

            let mut op_type = None;
            let mut op_path = None;
            let mut op_name = None;
            let mut op_value = None;

            for part in parts.unwrap() {
                match part.get("name").and_then(|n| n.as_str()) {
                    Some("type") => {
                        op_type = part
                            .get("valueCode")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                    Some("path") => {
                        op_path = part
                            .get("valueString")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                    Some("name") => {
                        op_name = part
                            .get("valueString")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                    Some("value") => {
                        op_value = extract_part_value(part);
                    }
                    _ => {}
                }
            }

            match op_type.as_deref() {
                Some("replace") => {
                    if let (Some(path), Some(value)) = (&op_path, &op_value) {
                        self.fhirpath_replace(&mut patched, path, value)?;
                    }
                }
                Some("add") => {
                    if let (Some(path), Some(name), Some(value)) = (&op_path, &op_name, &op_value) {
                        self.fhirpath_add(&mut patched, path, name, value)?;
                    }
                }
                Some("delete") => {
                    if let Some(path) = &op_path {
                        self.fhirpath_delete(&mut patched, path)?;
                    }
                }
                _ => {
                    // Unsupported operation type - skip
                }
            }
        }

        Ok(patched)
    }

    /// Helper for FHIRPath replace operation.
    fn fhirpath_replace(
        &self,
        resource: &mut Value,
        path: &str,
        value: &Value,
    ) -> StorageResult<()> {
        let parts: Vec<&str> = path.split('.').collect();
        if parts.len() == 2 {
            if let Some(obj) = resource.as_object_mut() {
                obj.insert(parts[1].to_string(), value.clone());
            }
        }
        Ok(())
    }

    /// Helper for FHIRPath add operation.
    fn fhirpath_add(
        &self,
        resource: &mut Value,
        path: &str,
        name: &str,
        value: &Value,
    ) -> StorageResult<()> {
        let parts: Vec<&str> = path.split('.').collect();
        if parts.len() == 1
            && parts[0]
                == resource
                    .get("resourceType")
                    .and_then(|r| r.as_str())
                    .unwrap_or("")
        {
            if let Some(obj) = resource.as_object_mut() {
                obj.insert(name.to_string(), value.clone());
            }
        }
        Ok(())
    }

    /// Helper for FHIRPath delete operation.
    fn fhirpath_delete(&self, resource: &mut Value, path: &str) -> StorageResult<()> {
        let parts: Vec<&str> = path.split('.').collect();
        if parts.len() == 2 {
            if let Some(obj) = resource.as_object_mut() {
                obj.remove(parts[1]);
            }
        }
        Ok(())
    }

    /// Applies a JSON Merge Patch (RFC 7386) to a resource.
    fn apply_merge_patch(&self, resource: &Value, merge_doc: &Value) -> Value {
        let mut patched = resource.clone();
        json_patch::merge(&mut patched, merge_doc);
        patched
    }
}

// ============================================================================
// BundleProvider Implementation
// ============================================================================

#[async_trait]
impl BundleProvider for PostgresBackend {
    async fn process_transaction(
        &self,
        tenant: &TenantContext,
        entries: Vec<BundleEntry>,
        fhir_version: helios_fhir::FhirVersion,
    ) -> Result<BundleResult, TransactionError> {
        use crate::core::transaction::{Transaction, TransactionOptions, TransactionProvider};
        use std::collections::HashMap;

        // Start a transaction
        let mut tx = self
            .begin_transaction(tenant, TransactionOptions::new().fhir_version(fhir_version))
            .await
            .map_err(|e| TransactionError::RolledBack {
                reason: format!("Failed to begin transaction: {}", e),
            })?;

        let mut results = Vec::with_capacity(entries.len());
        let mut error_info: Option<(usize, String)> = None;

        // Build a map of fullUrl -> assigned reference for reference resolution
        let mut reference_map: HashMap<String, String> = HashMap::new();

        // Make entries mutable for reference resolution
        let mut entries = entries;

        // Process each entry within the transaction
        for (idx, entry) in entries.iter_mut().enumerate() {
            // Resolve references in this entry's resource before processing
            if let Some(ref mut resource) = entry.resource {
                resolve_bundle_references(resource, &reference_map);
            }

            let result = self.process_bundle_entry_tx(&mut tx, entry).await;

            match result {
                Ok(entry_result) => {
                    if entry_result.status >= 400 {
                        error_info = Some((
                            idx,
                            format!("Entry failed with status {}", entry_result.status),
                        ));
                        break;
                    }

                    // If this was a create (POST) and we have a fullUrl, record the mapping
                    if entry.method == BundleMethod::Post {
                        if let Some(ref full_url) = entry.full_url {
                            if let Some(ref location) = entry_result.location {
                                let reference = location
                                    .split("/_history")
                                    .next()
                                    .unwrap_or(location)
                                    .to_string();
                                reference_map.insert(full_url.clone(), reference);
                            }
                        }
                    }

                    results.push(entry_result);
                }
                Err(e) => {
                    error_info = Some((idx, format!("Entry processing failed: {}", e)));
                    break;
                }
            }
        }

        // Handle error or commit
        if let Some((index, message)) = error_info {
            let _ = Box::new(tx).rollback().await;
            return Err(TransactionError::BundleError { index, message });
        }

        // Commit the transaction
        Box::new(tx)
            .commit()
            .await
            .map_err(|e| TransactionError::RolledBack {
                reason: format!("Commit failed: {}", e),
            })?;

        Ok(BundleResult {
            bundle_type: BundleType::Transaction,
            entries: results,
        })
    }

    async fn process_batch(
        &self,
        tenant: &TenantContext,
        entries: Vec<BundleEntry>,
        fhir_version: helios_fhir::FhirVersion,
    ) -> StorageResult<BundleResult> {
        let mut results = Vec::with_capacity(entries.len());

        // Process each entry independently
        for entry in &entries {
            let result = self.process_batch_entry(tenant, entry, fhir_version).await;
            results.push(result);
        }

        Ok(BundleResult {
            bundle_type: BundleType::Batch,
            entries: results,
        })
    }
}

impl PostgresBackend {
    /// Process a single bundle entry within a transaction.
    async fn process_bundle_entry_tx(
        &self,
        tx: &mut super::transaction::PostgresTransaction,
        entry: &BundleEntry,
    ) -> StorageResult<BundleEntryResult> {
        use crate::core::transaction::Transaction;

        match entry.method {
            BundleMethod::Get => {
                let (resource_type, id) = self.parse_url(&entry.url)?;
                match tx.read(&resource_type, &id).await? {
                    Some(resource) => Ok(BundleEntryResult::ok(resource)),
                    None => Ok(BundleEntryResult::error(
                        404,
                        serde_json::json!({
                            "resourceType": "OperationOutcome",
                            "issue": [{"severity": "error", "code": "not-found"}]
                        }),
                    )),
                }
            }
            BundleMethod::Post => {
                let resource = entry.resource.clone().ok_or_else(|| {
                    StorageError::Validation(crate::error::ValidationError::MissingRequiredField {
                        field: "resource".to_string(),
                    })
                })?;

                let resource_type = resource
                    .get("resourceType")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .ok_or_else(|| {
                        StorageError::Validation(
                            crate::error::ValidationError::MissingRequiredField {
                                field: "resourceType".to_string(),
                            },
                        )
                    })?;

                let created = tx.create(&resource_type, resource).await?;
                Ok(BundleEntryResult::created(created))
            }
            BundleMethod::Put => {
                let resource = entry.resource.clone().ok_or_else(|| {
                    StorageError::Validation(crate::error::ValidationError::MissingRequiredField {
                        field: "resource".to_string(),
                    })
                })?;

                let (resource_type, id) = self.parse_url(&entry.url)?;

                let existing = tx.read(&resource_type, &id).await?;

                // `ifMatch` is a list and is satisfied when any listed tag
                // matches; this used to compare the whole field value against
                // `existing.etag()` as one raw string, so a multi-valued header
                // could never match and `*` was unsupported (issue #311). An
                // absent resource now also fails a supplied `ifMatch` instead of
                // silently creating.
                if let Some(failure) = bundle_if_match_gate(
                    entry.if_match.as_deref(),
                    existing.as_ref().map(|r| r.version_id()),
                ) {
                    return Ok(failure);
                }

                match existing {
                    Some(existing) => {
                        let updated = tx.update(&existing, resource).await?;
                        Ok(BundleEntryResult::ok(updated))
                    }
                    None => {
                        // Create new resource with specified ID
                        let mut resource_with_id = resource;
                        resource_with_id["id"] = serde_json::json!(id);
                        let created = tx.create(&resource_type, resource_with_id).await?;
                        Ok(BundleEntryResult::created(created))
                    }
                }
            }
            BundleMethod::Delete => {
                let (resource_type, id) = self.parse_url(&entry.url)?;

                // Honor `ifMatch` on DELETE — previously ignored here, so a
                // client asking to delete only the version it had reviewed could
                // destroy a concurrent amendment with no 412. The read is
                // skipped entirely when no precondition was supplied.
                if entry.if_match.is_some() {
                    let existing = tx.read(&resource_type, &id).await?;
                    if let Some(failure) = bundle_if_match_gate(
                        entry.if_match.as_deref(),
                        existing.as_ref().map(|r| r.version_id()),
                    ) {
                        return Ok(failure);
                    }
                }

                tx.delete(&resource_type, &id).await?;
                Ok(BundleEntryResult::deleted())
            }
            BundleMethod::Patch => {
                // PATCH is not fully implemented yet
                Ok(BundleEntryResult::error(
                    501,
                    serde_json::json!({
                        "resourceType": "OperationOutcome",
                        "issue": [{"severity": "error", "code": "not-supported", "diagnostics": "PATCH not implemented in transaction bundles"}]
                    }),
                ))
            }
        }
    }

    /// Process a single batch entry (independent, no transaction).
    async fn process_batch_entry(
        &self,
        tenant: &TenantContext,
        entry: &BundleEntry,
        fhir_version: helios_fhir::FhirVersion,
    ) -> BundleEntryResult {
        match self
            .process_batch_entry_inner(tenant, entry, fhir_version)
            .await
        {
            Ok(result) => result,
            Err(e) => BundleEntryResult::error(
                500,
                serde_json::json!({
                    "resourceType": "OperationOutcome",
                    "issue": [{"severity": "error", "code": "exception", "diagnostics": e.to_string()}]
                }),
            ),
        }
    }

    async fn process_batch_entry_inner(
        &self,
        tenant: &TenantContext,
        entry: &BundleEntry,
        fhir_version: helios_fhir::FhirVersion,
    ) -> StorageResult<BundleEntryResult> {
        match entry.method {
            BundleMethod::Get => {
                let (resource_type, id) = self.parse_url(&entry.url)?;
                match self.read(tenant, &resource_type, &id).await? {
                    Some(resource) => Ok(BundleEntryResult::ok(resource)),
                    None => Ok(BundleEntryResult::error(
                        404,
                        serde_json::json!({
                            "resourceType": "OperationOutcome",
                            "issue": [{"severity": "error", "code": "not-found"}]
                        }),
                    )),
                }
            }
            BundleMethod::Post => {
                let resource = entry.resource.clone().ok_or_else(|| {
                    StorageError::Validation(crate::error::ValidationError::MissingRequiredField {
                        field: "resource".to_string(),
                    })
                })?;

                let resource_type = resource
                    .get("resourceType")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .ok_or_else(|| {
                        StorageError::Validation(
                            crate::error::ValidationError::MissingRequiredField {
                                field: "resourceType".to_string(),
                            },
                        )
                    })?;

                let created = self
                    .create(tenant, &resource_type, resource, fhir_version)
                    .await?;
                Ok(BundleEntryResult::created(created))
            }
            BundleMethod::Put => {
                let resource = entry.resource.clone().ok_or_else(|| {
                    StorageError::Validation(crate::error::ValidationError::MissingRequiredField {
                        field: "resource".to_string(),
                    })
                })?;

                let (resource_type, id) = self.parse_url(&entry.url)?;

                // `ifMatch` was previously dropped on the floor in the BATCH
                // path (the transaction path did check it), so optimistic
                // locking silently disappeared for anyone who wrapped a PUT in a
                // `type: batch` Bundle — a lost update reported as 200 OK.
                // Only pay for the extra read when a precondition was supplied.
                if entry.if_match.is_some() {
                    // A deleted resource has no current representation, so
                    // `Gone` is `None` here; real storage errors must not be
                    // swallowed into a bogus precondition failure.
                    let existing = match self.read(tenant, &resource_type, &id).await {
                        Ok(v) => v,
                        Err(StorageError::Resource(ResourceError::Gone { .. })) => None,
                        Err(e) => return Err(e),
                    };
                    if let Some(failure) = bundle_if_match_gate(
                        entry.if_match.as_deref(),
                        existing.as_ref().map(|r| r.version_id()),
                    ) {
                        return Ok(failure);
                    }
                }

                let (stored, _created) = self
                    .create_or_update(tenant, &resource_type, &id, resource, fhir_version)
                    .await?;
                Ok(BundleEntryResult::ok(stored))
            }
            BundleMethod::Delete => {
                let (resource_type, id) = self.parse_url(&entry.url)?;

                // As above: `ifMatch` on DELETE was ignored here entirely.
                if entry.if_match.is_some() {
                    // A deleted resource has no current representation, so
                    // `Gone` is `None` here; real storage errors must not be
                    // swallowed into a bogus precondition failure.
                    let existing = match self.read(tenant, &resource_type, &id).await {
                        Ok(v) => v,
                        Err(StorageError::Resource(ResourceError::Gone { .. })) => None,
                        Err(e) => return Err(e),
                    };
                    if let Some(failure) = bundle_if_match_gate(
                        entry.if_match.as_deref(),
                        existing.as_ref().map(|r| r.version_id()),
                    ) {
                        return Ok(failure);
                    }
                }

                match self.delete(tenant, &resource_type, &id).await {
                    Ok(()) => Ok(BundleEntryResult::deleted()),
                    Err(StorageError::Resource(ResourceError::NotFound { .. })) => {
                        Ok(BundleEntryResult::deleted()) // Idempotent delete
                    }
                    Err(e) => Err(e),
                }
            }
            BundleMethod::Patch => Ok(BundleEntryResult::error(
                501,
                serde_json::json!({
                    "resourceType": "OperationOutcome",
                    "issue": [{"severity": "error", "code": "not-supported", "diagnostics": "PATCH not implemented"}]
                }),
            )),
        }
    }

    /// Parse a FHIR URL into resource type and ID.
    fn parse_url(&self, url: &str) -> StorageResult<(String, String)> {
        let path = url
            .strip_prefix("http://")
            .or_else(|| url.strip_prefix("https://"))
            .map(|s| s.find('/').map(|i| &s[i..]).unwrap_or(s))
            .unwrap_or(url);

        let path = path.trim_start_matches('/');
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        if parts.len() >= 2 {
            let len = parts.len();
            Ok((parts[len - 2].to_string(), parts[len - 1].to_string()))
        } else {
            Err(StorageError::Validation(
                crate::error::ValidationError::InvalidReference {
                    reference: url.to_string(),
                    message: "URL must be in format ResourceType/id".to_string(),
                },
            ))
        }
    }
}

/// Recursively resolves urn:uuid references in a JSON value using the reference map.
fn resolve_bundle_references(
    value: &mut serde_json::Value,
    reference_map: &std::collections::HashMap<String, String>,
) {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            if let Some(Value::String(ref_str)) = map.get("reference") {
                if ref_str.starts_with("urn:uuid:") {
                    if let Some(resolved) = reference_map.get(ref_str) {
                        map.insert("reference".to_string(), Value::String(resolved.clone()));
                    }
                }
            }
            for v in map.values_mut() {
                resolve_bundle_references(v, reference_map);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                resolve_bundle_references(item, reference_map);
            }
        }
        _ => {}
    }
}

// ============================================================================
// ReindexSource Implementation — PostgreSQL is a primary, so it is where
// resources are read from during a reindex.
// ============================================================================

#[async_trait]
impl ReindexSource for PostgresBackend {
    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let rows = client
            .query(
                "SELECT DISTINCT resource_type FROM resources WHERE tenant_id = $1 AND is_deleted = FALSE",
                &[&tenant_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to query resource types: {}", e)))?;

        let types: Vec<String> = rows.iter().map(|row| row.get(0)).collect();
        Ok(types)
    }

    async fn count_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64> {
        self.count(tenant, Some(resource_type)).await
    }

    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> StorageResult<ResourcePage> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Parse cursor if provided (format: "last_updated|id")
        let (cursor_ts, cursor_id) = if let Some(c) = cursor {
            let parts: Vec<&str> = c.split('|').collect();
            if parts.len() == 2 {
                let ts = DateTime::parse_from_rfc3339(parts[0])
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| internal_error(format!("Invalid cursor timestamp: {}", e)))?;
                (Some(ts), Some(parts[1].to_string()))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        let rows = if let (Some(ts), Some(id)) = (&cursor_ts, &cursor_id) {
            client
                .query(
                    "SELECT id, version_id, data, last_updated, fhir_version FROM resources
                     WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE
                     AND (last_updated > $3 OR (last_updated = $3 AND id > $4))
                     ORDER BY last_updated ASC, id ASC LIMIT $5",
                    &[
                        &tenant_id,
                        &resource_type,
                        ts,
                        &id.as_str(),
                        &(limit as i64),
                    ],
                )
                .await
                .map_err(|e| internal_error(format!("Failed to fetch resources page: {}", e)))?
        } else {
            client
                .query(
                    "SELECT id, version_id, data, last_updated, fhir_version FROM resources
                     WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE
                     ORDER BY last_updated ASC, id ASC LIMIT $3",
                    &[&tenant_id, &resource_type, &(limit as i64)],
                )
                .await
                .map_err(|e| internal_error(format!("Failed to fetch resources page: {}", e)))?
        };

        let resources: Vec<StoredResource> = rows
            .iter()
            .map(|row| {
                let id: String = row.get(0);
                let version_id: String = row.get(1);
                let data: Value = row.get(2);
                let last_updated: DateTime<Utc> = row.get(3);
                let fhir_version_str: String = row.get(4);
                let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                    .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

                StoredResource::from_storage(
                    resource_type,
                    id,
                    version_id,
                    tenant.tenant_id().clone(),
                    data,
                    last_updated,
                    last_updated,
                    None,
                    fhir_version,
                )
            })
            .collect();

        // Determine next cursor
        let next_cursor = if resources.len() == limit as usize {
            resources
                .last()
                .map(|r| format!("{}|{}", r.last_modified().to_rfc3339(), r.id()))
        } else {
            None
        };

        Ok(ResourcePage {
            resources,
            next_cursor,
        })
    }
}

// ============================================================================
// ReindexTarget Implementation — PostgreSQL keeps search entries in its own
// `search_index` table, so it is also a writer and can reindex itself.
// ============================================================================

#[async_trait]
impl ReindexTarget for PostgresBackend {
    async fn delete_search_entries(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<u64> {
        let client = self.get_client().await?;
        self.delete_search_index(
            &client,
            tenant.tenant_id().as_str(),
            resource_type,
            resource_id,
        )
        .await
    }

    async fn write_search_entries(
        &self,
        tenant: &TenantContext,
        resource: &StoredResource,
    ) -> StorageResult<usize> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let resource_type = resource.resource_type();
        let resource_id = resource.id();
        let content = resource.content();

        // Use the dynamic extraction over the tenant's registry
        let values = self
            .tenant_extractor(tenant_id)
            .extract(content, resource_type)
            .map_err(|e| internal_error(format!("Search parameter extraction failed: {}", e)))?;

        let mut count = 0;
        for value in values {
            PostgresSearchIndexWriter::write_entry(
                &client,
                tenant_id,
                resource_type,
                resource_id,
                &value,
            )
            .await?;
            count += 1;
        }

        // Re-index contained resources too, so `$reindex` rebuilds `_contained`
        // search entries.
        count += self
            .index_contained_resources(&client, tenant_id, resource_type, resource_id, content)
            .await?;

        Ok(count)
    }

    async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let deleted = client
            .execute(
                "DELETE FROM search_index WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await
            .or_query_error("Failed to clear search index")?;

        // Also clear FTS entries
        let _ = client
            .execute(
                "DELETE FROM resource_fts WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await;

        Ok(deleted)
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Normalize a date string for PostgreSQL TIMESTAMPTZ.
fn normalize_date_for_pg(value: &str) -> String {
    if value.contains('T') {
        if value.contains('+') || value.contains('Z') || value.ends_with("-00:00") {
            value.to_string()
        } else {
            format!("{}+00:00", value)
        }
    } else if value.len() == 10 {
        format!("{}T00:00:00+00:00", value)
    } else if value.len() == 7 {
        format!("{}-01T00:00:00+00:00", value)
    } else if value.len() == 4 {
        format!("{}-01-01T00:00:00+00:00", value)
    } else {
        value.to_string()
    }
}

// ============================================================================
// FTS Content Extraction (local copy to avoid cross-feature dependency on sqlite)
// ============================================================================

/// Content extracted from a resource for full-text search.
struct SearchableContent {
    narrative: String,
    full_content: String,
}

impl SearchableContent {
    fn is_empty(&self) -> bool {
        self.narrative.is_empty() && self.full_content.is_empty()
    }
}

/// Extracts searchable text content from a FHIR resource.
fn extract_searchable_content(resource: &Value) -> SearchableContent {
    SearchableContent {
        narrative: extract_narrative(resource),
        full_content: extract_all_strings(resource),
    }
}

/// Extracts narrative text from resource.text.div, stripping HTML tags.
fn extract_narrative(resource: &Value) -> String {
    resource
        .get("text")
        .and_then(|t| t.get("div"))
        .and_then(|d| d.as_str())
        .map(strip_html_tags)
        .unwrap_or_default()
}

/// Strips HTML tags from a string, returning plain text.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;

    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                result.push(' ');
            }
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }

    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extracts all string values from a JSON value recursively.
fn extract_all_strings(value: &Value) -> String {
    let mut parts = Vec::new();
    collect_strings(value, &mut parts);
    parts.join(" ")
}

fn collect_strings(value: &Value, parts: &mut Vec<String>) {
    match value {
        Value::String(s) => {
            if !s.is_empty() {
                parts.push(s.clone());
            }
        }
        Value::Object(map) => {
            for (key, val) in map {
                if key == "div" || key == "data" {
                    continue;
                }
                collect_strings(val, parts);
            }
        }
        Value::Array(arr) => {
            for val in arr {
                collect_strings(val, parts);
            }
        }
        _ => {}
    }
}
