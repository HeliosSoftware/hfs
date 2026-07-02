//! ResourceStorage and VersionedStorage implementations for SQLite.

use async_trait::async_trait;
use chrono::Utc;
use helios_fhir::FhirVersion;
use rusqlite::{ToSql, params};
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
    PurgableStorage, ResourceStorage, SearchProvider, VersionedStorage,
};
use crate::error::TransactionError;
use crate::error::{BackendError, ConcurrencyError, ResourceError, StorageError, StorageResult};
use crate::search::extractor::ExtractedValue;
use crate::search::loader::SearchParameterLoader;
use crate::search::registry::SearchParameterStatus;
use crate::search::reindex::{ReindexableStorage, ResourcePage};
use crate::tenant::TenantContext;
use crate::types::Pagination;
use crate::types::{CursorValue, Page, PageCursor, PageInfo, StoredResource};
use crate::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

use super::SqliteBackend;
use super::search::writer::{SqlValue, SqliteSearchIndexWriter};

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "sqlite".to_string(),
        message,
        source: None,
    })
}

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
impl ResourceStorage for SqliteBackend {
    fn backend_name(&self) -> &'static str {
        "sqlite"
    }

    fn sof_runner(&self) -> Option<std::sync::Arc<dyn crate::core::sof_runner::SofRunner>> {
        use crate::sof::sqlite::SqliteInDbRunner;
        Some(std::sync::Arc::new(SqliteInDbRunner::new(self.pool())))
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Extract or generate ID
        let id = resource
            .get("id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        // Check if resource already exists
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3",
                params![tenant_id, resource_type, id],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if exists {
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

        // Serialize the resource data
        let data = serde_json::to_vec(&resource)
            .map_err(|e| serialization_error(format!("Failed to serialize resource: {}", e)))?;

        let now = Utc::now();
        let last_updated = now.to_rfc3339();
        let version_id = "1";
        let fhir_version_str = fhir_version.as_mime_param();

        // Insert the resource
        conn.execute(
            "INSERT INTO resources (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
            params![tenant_id, resource_type, id, version_id, data, last_updated, fhir_version_str],
        )
        .map_err(|e| internal_error(format!("Failed to insert resource: {}", e)))?;

        // Insert into history
        conn.execute(
            "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
            params![tenant_id, resource_type, id, version_id, data, last_updated, fhir_version_str],
        )
        .map_err(|e| internal_error(format!("Failed to insert history: {}", e)))?;

        // Index the resource for search
        self.index_resource(&conn, tenant_id, resource_type, &id, &resource)?;

        // Handle SearchParameter resources specially - update registry
        if resource_type == "SearchParameter" {
            self.handle_search_parameter_create(&resource)?;
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
        let existing = self.read(tenant, resource_type, id).await?;

        if let Some(current) = existing {
            // Update existing (preserves original FHIR version)
            let updated = self.update(tenant, &current, resource).await?;
            Ok((updated, false))
        } else {
            // Create new with specific ID
            let mut resource = resource;
            if let Some(obj) = resource.as_object_mut() {
                obj.insert("id".to_string(), Value::String(id.to_string()));
            }
            let created = self
                .create(tenant, resource_type, resource, fhir_version)
                .await?;
            Ok((created, true))
        }
    }

    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let result = conn.query_row(
            "SELECT version_id, data, last_updated, is_deleted, deleted_at, fhir_version
             FROM resources
             WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3",
            params![tenant_id, resource_type, id],
            |row| {
                let version_id: String = row.get(0)?;
                let data: Vec<u8> = row.get(1)?;
                let last_updated: String = row.get(2)?;
                let is_deleted: i32 = row.get(3)?;
                let deleted_at: Option<String> = row.get(4)?;
                let fhir_version: String = row.get(5)?;
                Ok((
                    version_id,
                    data,
                    last_updated,
                    is_deleted,
                    deleted_at,
                    fhir_version,
                ))
            },
        );

        match result {
            Ok((version_id, data, last_updated, is_deleted, deleted_at, fhir_version_str)) => {
                // If deleted, return Gone error
                if is_deleted != 0 {
                    let deleted_at = deleted_at.and_then(|s| {
                        chrono::DateTime::parse_from_rfc3339(&s)
                            .ok()
                            .map(|dt| dt.with_timezone(&Utc))
                    });
                    return Err(StorageError::Resource(ResourceError::Gone {
                        resource_type: resource_type.to_string(),
                        id: id.to_string(),
                        deleted_at,
                    }));
                }

                let json_data: serde_json::Value = serde_json::from_slice(&data).map_err(|e| {
                    serialization_error(format!("Failed to deserialize resource: {}", e))
                })?;

                let last_updated = chrono::DateTime::parse_from_rfc3339(&last_updated)
                    .map_err(|e| internal_error(format!("Failed to parse last_updated: {}", e)))?
                    .with_timezone(&Utc);

                // Parse the FHIR version from storage
                let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                    .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

                Ok(Some(StoredResource::from_storage(
                    resource_type,
                    id,
                    version_id,
                    tenant.tenant_id().clone(),
                    json_data,
                    last_updated,
                    last_updated,
                    None,
                    fhir_version,
                )))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(internal_error(format!("Failed to read resource: {}", e))),
        }
    }

    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();
        let resource_type = current.resource_type();
        let id = current.id();

        // Check that the resource still exists with the expected version
        let actual_version: Result<String, _> = conn.query_row(
            "SELECT version_id FROM resources
             WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3 AND is_deleted = 0",
            params![tenant_id, resource_type, id],
            |row| row.get(0),
        );

        let actual_version = match actual_version {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
            }
            Err(e) => {
                return Err(internal_error(format!(
                    "Failed to get current version: {}",
                    e
                )));
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

        // Serialize the resource data
        let data = serde_json::to_vec(&resource)
            .map_err(|e| serialization_error(format!("Failed to serialize resource: {}", e)))?;

        let now = Utc::now();
        let last_updated = now.to_rfc3339();

        // Update the resource
        conn.execute(
            "UPDATE resources SET version_id = ?1, data = ?2, last_updated = ?3
             WHERE tenant_id = ?4 AND resource_type = ?5 AND id = ?6",
            params![
                new_version_str,
                data,
                last_updated,
                tenant_id,
                resource_type,
                id
            ],
        )
        .map_err(|e| internal_error(format!("Failed to update resource: {}", e)))?;

        // Insert into history (preserve the original FHIR version)
        let fhir_version_str = current.fhir_version().as_mime_param();
        conn.execute(
            "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
            params![tenant_id, resource_type, id, new_version_str, data, last_updated, fhir_version_str],
        )
        .map_err(|e| internal_error(format!("Failed to insert history: {}", e)))?;

        // Re-index the resource (delete old entries, add new)
        self.delete_search_index(&conn, tenant_id, resource_type, id)?;
        self.index_resource(&conn, tenant_id, resource_type, id, &resource)?;

        // Handle SearchParameter resources specially - update registry
        if resource_type == "SearchParameter" {
            self.handle_search_parameter_update(current.content(), &resource)?;
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
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check if resource exists and get its fhir_version
        let result: Result<(String, Vec<u8>, String), _> = conn.query_row(
            "SELECT version_id, data, fhir_version FROM resources
             WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3 AND is_deleted = 0",
            params![tenant_id, resource_type, id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        );

        let (current_version, data, fhir_version_str) = match result {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
            }
            Err(e) => {
                return Err(internal_error(format!("Failed to check resource: {}", e)));
            }
        };

        let now = Utc::now();
        let deleted_at = now.to_rfc3339();

        // Calculate new version for the deletion record
        let new_version: u64 = current_version.parse().unwrap_or(0) + 1;
        let new_version_str = new_version.to_string();

        // Soft delete the resource
        conn.execute(
            "UPDATE resources SET is_deleted = 1, deleted_at = ?1, version_id = ?2, last_updated = ?1
             WHERE tenant_id = ?3 AND resource_type = ?4 AND id = ?5",
            params![deleted_at, new_version_str, tenant_id, resource_type, id],
        )
        .map_err(|e| internal_error(format!("Failed to delete resource: {}", e)))?;

        // Insert deletion record into history (preserve fhir_version)
        conn.execute(
            "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
            params![tenant_id, resource_type, id, new_version_str, data, deleted_at, fhir_version_str],
        )
        .map_err(|e| internal_error(format!("Failed to insert deletion history: {}", e)))?;

        // Delete search index entries (skip when search is offloaded)
        if !self.is_search_offloaded() {
            conn.execute(
                "DELETE FROM search_index WHERE tenant_id = ?1 AND resource_type = ?2 AND resource_id = ?3",
                params![tenant_id, resource_type, id],
            )
            .map_err(|e| internal_error(format!("Failed to delete search index: {}", e)))?;
        }

        // Handle SearchParameter resources specially - update registry
        if resource_type == "SearchParameter" {
            if let Ok(resource_json) = serde_json::from_slice::<Value>(&data) {
                self.handle_search_parameter_delete(&resource_json)?;
            }
        }

        Ok(())
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let count: i64 = if let Some(rt) = resource_type {
            conn.query_row(
                "SELECT COUNT(*) FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0",
                params![tenant_id, rt],
                |row| row.get(0),
            )
        } else {
            conn.query_row(
                "SELECT COUNT(*) FROM resources WHERE tenant_id = ?1 AND is_deleted = 0",
                params![tenant_id],
                |row| row.get(0),
            )
        }
        .map_err(|e| internal_error(format!("Failed to count resources: {}", e)))?;

        Ok(count as u64)
    }

    async fn count_by_day(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        since: chrono::DateTime<chrono::Utc>,
    ) -> StorageResult<Vec<crate::core::DailyResourceCount>> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();
        // `last_updated` is stored as an RFC3339 UTC string (Utc::now().to_rfc3339()),
        // e.g. `2026-06-01T14:30:00.123+00:00` — always `+00:00`, with a fixed-width
        // `YYYY-MM-DDTHH:MM:SS` prefix. Its first 10 characters are the UTC calendar
        // day, so we bucket on that prefix in SELECT/GROUP BY. The WHERE filter, by
        // contrast, ranges over the *raw* column so the `(tenant_id, last_updated)`
        // index can prune by date (wrapping the column in `substr(...)` would force a
        // full scan). `since_bound` is the start-of-day timestamp for `since` in the
        // stored format; since every stored value shares that fixed prefix and any
        // sub-day suffix (`.` = 0x2E or `+` = 0x2B) sorts at/after `...T00:00:00+00:00`,
        // the lexicographic `>=` selects exactly the rows the day-prefix filter did.
        let since_bound = since
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .expect("00:00:00 is always a valid time")
            .and_utc()
            .to_rfc3339();

        let mut stmt = conn
            .prepare(
                "SELECT substr(last_updated, 1, 10) AS day, COUNT(*) AS n \
                 FROM resources \
                 WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0 \
                   AND last_updated >= ?3 \
                 GROUP BY day ORDER BY day",
            )
            .map_err(|e| internal_error(format!("Failed to prepare count_by_day: {}", e)))?;

        let rows = stmt
            .query_map(params![tenant_id, resource_type, since_bound], |row| {
                let day: String = row.get(0)?;
                let n: i64 = row.get(1)?;
                Ok((day, n))
            })
            .map_err(|e| internal_error(format!("Failed to query count_by_day: {}", e)))?;

        let mut out = Vec::new();
        for row in rows {
            let (day_str, n) = row
                .map_err(|e| internal_error(format!("Failed to read count_by_day row: {}", e)))?;
            if let Ok(day) = chrono::NaiveDate::parse_from_str(&day_str, "%Y-%m-%d") {
                out.push(crate::core::DailyResourceCount {
                    day,
                    count: n.max(0) as u64,
                });
            }
        }
        Ok(out)
    }

    async fn activity_histogram(
        &self,
        tenant: &TenantContext,
        since: chrono::DateTime<chrono::Utc>,
    ) -> StorageResult<Vec<crate::core::ActivityCell>> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();
        // Start-of-day bound for `since` in the stored RFC3339 UTC format; see
        // `count_by_day` for why a raw-column `last_updated >= ?` range is both
        // sargable (uses the `(tenant_id, last_updated)` history index) and selects
        // exactly the same rows the old `substr(last_updated, 1, 10) >= day` filter did.
        let since_bound = since
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .expect("00:00:00 is always a valid time")
            .and_utc()
            .to_rfc3339();

        // `strftime` parses the stored RFC3339 UTC string and yields UTC weekday
        // (`%w`: 0=Sunday..6=Saturday) and hour (`%H`: 00..23).
        let mut stmt = conn
            .prepare(
                "SELECT CAST(strftime('%w', last_updated) AS INTEGER) AS wd, \
                        CAST(strftime('%H', last_updated) AS INTEGER) AS hr, \
                        COUNT(*) AS n \
                 FROM resource_history \
                 WHERE tenant_id = ?1 AND last_updated >= ?2 \
                 GROUP BY wd, hr",
            )
            .map_err(|e| internal_error(format!("Failed to prepare activity_histogram: {}", e)))?;

        let rows = stmt
            .query_map(params![tenant_id, since_bound], |row| {
                let wd: i64 = row.get(0)?;
                let hr: i64 = row.get(1)?;
                let n: i64 = row.get(2)?;
                Ok((wd, hr, n))
            })
            .map_err(|e| internal_error(format!("Failed to query activity_histogram: {}", e)))?;

        let mut out = Vec::new();
        for row in rows {
            let (wd, hr, n) = row
                .map_err(|e| internal_error(format!("Failed to read activity row: {}", e)))?;
            out.push(crate::core::ActivityCell {
                weekday: wd.clamp(0, 6) as u8,
                hour: hr.clamp(0, 23) as u8,
                count: n.max(0) as u64,
            });
        }
        Ok(out)
    }

    async fn count_all_types(
        &self,
        tenant: &TenantContext,
    ) -> StorageResult<Vec<(String, u64)>> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();
        let mut stmt = conn
            .prepare(
                "SELECT resource_type, COUNT(*) FROM resources \
                 WHERE tenant_id = ?1 AND is_deleted = 0 \
                 GROUP BY resource_type",
            )
            .map_err(|e| internal_error(format!("Failed to prepare count_all_types: {}", e)))?;
        let rows = stmt
            .query_map(params![tenant_id], |row| {
                let rt: String = row.get(0)?;
                let n: i64 = row.get(1)?;
                Ok((rt, n.max(0) as u64))
            })
            .map_err(|e| internal_error(format!("Failed to query count_all_types: {}", e)))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| internal_error(format!("count_all_types row: {}", e)))?);
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
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Bind tenant_id as ?1 and each requested type as ?2, ?3, ...; the type
        // names are bound as parameters, never interpolated into the SQL text.
        let placeholders = (0..resource_types.len())
            .map(|i| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT resource_type, COUNT(*) FROM resources \
             WHERE tenant_id = ?1 AND is_deleted = 0 AND resource_type IN ({}) \
             GROUP BY resource_type",
            placeholders
        );

        // Positional bind values matching the `?1..?n` order above: tenant first,
        // then the requested types.
        let mut binds: Vec<&str> = Vec::with_capacity(resource_types.len() + 1);
        binds.push(tenant_id);
        binds.extend_from_slice(resource_types);

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| internal_error(format!("Failed to prepare count_by_types: {}", e)))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(binds), |row| {
                let rt: String = row.get(0)?;
                let n: i64 = row.get(1)?;
                Ok((rt, n.max(0) as u64))
            })
            .map_err(|e| internal_error(format!("Failed to query count_by_types: {}", e)))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| internal_error(format!("count_by_types row: {}", e)))?);
        }
        Ok(out)
    }

    async fn count_by_tenant(&self) -> StorageResult<Vec<(String, u64)>> {
        // Cross-tenant admin aggregate (see trait docs): no tenant filter.
        let conn = self.get_connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT tenant_id, COUNT(*) FROM resources \
                 WHERE is_deleted = 0 GROUP BY tenant_id",
            )
            .map_err(|e| internal_error(format!("Failed to prepare count_by_tenant: {}", e)))?;
        let rows = stmt
            .query_map([], |row| {
                let tid: String = row.get(0)?;
                let n: i64 = row.get(1)?;
                Ok((tid, n.max(0) as u64))
            })
            .map_err(|e| internal_error(format!("Failed to query count_by_tenant: {}", e)))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| internal_error(format!("count_by_tenant row: {}", e)))?);
        }
        Ok(out)
    }
}

// Search Index Helpers
impl SqliteBackend {
    /// Index a resource for search.
    ///
    /// This method uses the SearchParameterExtractor to dynamically extract
    /// searchable values based on the configured SearchParameterRegistry.
    /// Falls back to hardcoded common parameter extraction if the registry
    /// extraction fails.
    pub(crate) fn index_resource(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> StorageResult<()> {
        // When search is offloaded to a secondary backend, skip local indexing
        if self.is_search_offloaded() {
            return Ok(());
        }

        // Try dynamic extraction using the registry-driven extractor
        match self.index_resource_dynamic(conn, tenant_id, resource_type, resource_id, resource) {
            Ok(count) => {
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
                self.index_minimal_fallback(conn, tenant_id, resource_type, resource_id, resource)?;
            }
        }

        // Index FTS content for _text and _content searches
        self.index_fts_content(conn, tenant_id, resource_type, resource_id, resource)?;

        Ok(())
    }

    /// Index full-text search content for _text and _content searches.
    ///
    /// This populates the resource_fts table if FTS5 is available.
    fn index_fts_content(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> StorageResult<()> {
        use super::search::fts::extract_searchable_content;

        // Check if FTS table exists (created in schema v3)
        let fts_exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='resource_fts'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if !fts_exists {
            // FTS5 not available - skip silently
            return Ok(());
        }

        // Extract searchable content
        let content = extract_searchable_content(resource);

        if content.is_empty() {
            return Ok(());
        }

        // Insert into FTS table
        conn.execute(
            "INSERT INTO resource_fts (resource_id, resource_type, tenant_id, narrative_text, full_content)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                resource_id,
                resource_type,
                tenant_id,
                content.narrative,
                content.full_content
            ],
        )
        .map_err(|e| internal_error(format!("Failed to insert FTS content: {}", e)))?;

        Ok(())
    }

    /// Index a resource using dynamic extraction from the SearchParameterRegistry.
    ///
    /// Returns the number of index entries created.
    fn index_resource_dynamic(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> StorageResult<usize> {
        // Extract values using the registry-driven extractor
        let values = self
            .search_extractor()
            .extract(resource, resource_type)
            .map_err(|e| internal_error(format!("Search parameter extraction failed: {}", e)))?;

        let mut count = 0;
        for value in values {
            self.write_index_entry(conn, tenant_id, resource_type, resource_id, &value)?;
            count += 1;
        }

        // Also index any contained resources for `_contained` search.
        count +=
            self.index_contained_resources(conn, tenant_id, resource_type, resource_id, resource)?;

        Ok(count)
    }

    /// Writes a single ExtractedValue to the search_index table.
    fn write_index_entry(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        value: &ExtractedValue,
    ) -> StorageResult<()> {
        use crate::search::converters::IndexValue;

        // For date values, normalize the date format for consistent SQLite comparisons
        let normalized_value = match &value.value {
            IndexValue::Date {
                value: date_str,
                precision,
            } => {
                let normalized_date = Self::normalize_date_for_sqlite(date_str);
                let mut normalized = value.clone();
                normalized.value = IndexValue::Date {
                    value: normalized_date,
                    precision: *precision,
                };
                Some(normalized)
            }
            _ => None,
        };

        let value_to_use = normalized_value.as_ref().unwrap_or(value);
        let sql_params = SqliteSearchIndexWriter::to_sql_params(
            tenant_id,
            resource_type,
            resource_id,
            value_to_use,
        );

        // Build parameter refs for rusqlite
        let param_refs: Vec<&dyn ToSql> = sql_params
            .iter()
            .map(|p| self.sql_value_to_ref(p))
            .collect();

        conn.execute(SqliteSearchIndexWriter::insert_sql(), param_refs.as_slice())
            .map_err(|e| internal_error(format!("Failed to insert search index entry: {}", e)))?;

        Ok(())
    }

    /// Extracts and indexes a container's `contained[]` resources for
    /// `_contained` search. Each contained resource's search values are written
    /// as `is_contained = 1` rows whose `resource_type` / `resource_id` identify
    /// the container. Returns the number of entries written.
    fn index_contained_resources(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        container_type: &str,
        container_id: &str,
        resource: &Value,
    ) -> StorageResult<usize> {
        let mut count = 0;
        let container = (container_type, container_id);
        for contained in self.search_extractor().extract_contained(resource) {
            for value in &contained.values {
                self.write_contained_index_entry(
                    conn,
                    tenant_id,
                    container,
                    (&contained.contained_type, &contained.local_id),
                    value,
                )?;
                count += 1;
            }
        }
        Ok(count)
    }

    /// Writes a single contained `ExtractedValue` to the search_index table,
    /// flagged `is_contained = 1` and carrying the contained resource's type and
    /// local id (mirrors [`Self::write_index_entry`]). `container` is the
    /// `(type, id)` of the holding resource; `contained` is the
    /// `(type, local id)` of the nested resource.
    fn write_contained_index_entry(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        container: (&str, &str),
        contained: (&str, &str),
        value: &ExtractedValue,
    ) -> StorageResult<()> {
        use crate::search::converters::IndexValue;

        let (container_type, container_id) = container;
        let (contained_type, contained_local_id) = contained;

        let normalized_value = match &value.value {
            IndexValue::Date {
                value: date_str,
                precision,
            } => {
                let mut normalized = value.clone();
                normalized.value = IndexValue::Date {
                    value: Self::normalize_date_for_sqlite(date_str),
                    precision: *precision,
                };
                Some(normalized)
            }
            _ => None,
        };
        let value_to_use = normalized_value.as_ref().unwrap_or(value);

        let mut sql_params = SqliteSearchIndexWriter::to_sql_params(
            tenant_id,
            container_type,
            container_id,
            value_to_use,
        );
        // Trailing contained columns (?25..?27).
        sql_params.push(SqlValue::Int(1));
        sql_params.push(SqlValue::String(contained_type.to_string()));
        sql_params.push(SqlValue::String(contained_local_id.to_string()));

        let param_refs: Vec<&dyn ToSql> = sql_params
            .iter()
            .map(|p| self.sql_value_to_ref(p))
            .collect();

        conn.execute(
            SqliteSearchIndexWriter::insert_contained_sql(),
            param_refs.as_slice(),
        )
        .map_err(|e| {
            internal_error(format!(
                "Failed to insert contained search index entry: {}",
                e
            ))
        })?;

        Ok(())
    }

    /// Normalizes a date string for SQLite comparisons.
    ///
    /// Ensures dates have a time component for consistent range comparisons.
    fn normalize_date_for_sqlite(value: &str) -> String {
        if value.contains('T') {
            value.to_string()
        } else if value.len() == 10 {
            // YYYY-MM-DD -> YYYY-MM-DDTHH:MM:SS
            format!("{}T00:00:00", value)
        } else if value.len() == 7 {
            // YYYY-MM -> YYYY-MM-01T00:00:00
            format!("{}-01T00:00:00", value)
        } else if value.len() == 4 {
            // YYYY -> YYYY-01-01T00:00:00
            format!("{}-01-01T00:00:00", value)
        } else {
            value.to_string()
        }
    }

    /// Converts a SqlValue to a rusqlite-compatible reference.
    fn sql_value_to_ref<'a>(&'a self, value: &'a super::search::writer::SqlValue) -> &'a dyn ToSql {
        use super::search::writer::SqlValue;
        match value {
            SqlValue::String(s) => s,
            SqlValue::OptString(opt) => opt,
            SqlValue::Int(i) => i,
            SqlValue::OptInt(opt) => opt,
            SqlValue::Float(f) => f,
            SqlValue::Null => &rusqlite::types::Null,
        }
    }

    /// Delete search index entries for a resource.
    pub(crate) fn delete_search_index(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<()> {
        // When search is offloaded to a secondary backend, skip local index cleanup
        if self.is_search_offloaded() {
            return Ok(());
        }

        // Delete from main search index
        conn.execute(
            "DELETE FROM search_index WHERE tenant_id = ?1 AND resource_type = ?2 AND resource_id = ?3",
            params![tenant_id, resource_type, resource_id],
        )
        .map_err(|e| internal_error(format!("Failed to delete search index: {}", e)))?;

        // Delete from FTS table if it exists
        let _ = conn.execute(
            "DELETE FROM resource_fts WHERE tenant_id = ?1 AND resource_type = ?2 AND resource_id = ?3",
            params![tenant_id, resource_type, resource_id],
        );

        Ok(())
    }

    /// Index minimal fallback search parameters.
    ///
    /// This only indexes `_id` and `_lastUpdated` - the essential Resource-level
    /// parameters that should always work. Used when dynamic extraction fails
    /// and spec files are not available.
    fn index_minimal_fallback(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> StorageResult<()> {
        // _id - always available from resource.id
        if let Some(id) = resource.get("id").and_then(|v| v.as_str()) {
            self.insert_token_index(conn, tenant_id, resource_type, resource_id, "_id", None, id)?;
        }

        // _lastUpdated - from resource.meta.lastUpdated
        if let Some(last_updated) = resource
            .get("meta")
            .and_then(|m| m.get("lastUpdated"))
            .and_then(|v| v.as_str())
        {
            self.insert_date_index(
                conn,
                tenant_id,
                resource_type,
                resource_id,
                "_lastUpdated",
                last_updated,
            )?;
        }

        Ok(())
    }

    /// Insert a token index entry.
    #[allow(clippy::too_many_arguments)]
    fn insert_token_index(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        param_name: &str,
        system: Option<&str>,
        code: &str,
    ) -> StorageResult<()> {
        conn.execute(
            "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_token_system, value_token_code)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![tenant_id, resource_type, resource_id, param_name, system, code],
        )
        .map_err(|e| internal_error(format!("Failed to insert token index: {}", e)))?;
        Ok(())
    }

    /// Insert a date index entry.
    fn insert_date_index(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        param_name: &str,
        value: &str,
    ) -> StorageResult<()> {
        // Normalize date format: ensure we have at least YYYY-MM-DDTHH:MM:SS
        // This enables proper range comparisons in SQLite
        let normalized = if value.contains('T') {
            value.to_string()
        } else if value.len() == 10 {
            // YYYY-MM-DD -> YYYY-MM-DDTHH:MM:SS
            format!("{}T00:00:00", value)
        } else if value.len() == 7 {
            // YYYY-MM -> YYYY-MM-01T00:00:00
            format!("{}-01T00:00:00", value)
        } else if value.len() == 4 {
            // YYYY -> YYYY-01-01T00:00:00
            format!("{}-01-01T00:00:00", value)
        } else {
            value.to_string()
        };

        conn.execute(
            "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_date)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![tenant_id, resource_type, resource_id, param_name, normalized],
        )
        .map_err(|e| internal_error(format!("Failed to insert date index: {}", e)))?;
        Ok(())
    }
}

// SearchParameter Resource Handling
impl SqliteBackend {
    /// Handle creation of a SearchParameter resource.
    ///
    /// If the SearchParameter has status=active, it will be registered in the
    /// search parameter registry, making it available for searches on new resources.
    /// Existing resources will NOT be indexed for this parameter until $reindex is run.
    fn handle_search_parameter_create(&self, resource: &Value) -> StorageResult<()> {
        let loader = SearchParameterLoader::new(FhirVersion::default_enabled());

        match loader.parse_resource(resource) {
            Ok(def) => {
                // Only register if status is active
                if def.status == SearchParameterStatus::Active {
                    let mut registry = self.search_registry().write();
                    // Ignore duplicate URL errors - the param may already be embedded
                    if let Err(e) = registry.register(def) {
                        tracing::debug!("SearchParameter registration skipped: {}", e);
                    }
                }
            }
            Err(e) => {
                // Log but don't fail - the resource is still stored
                tracing::warn!("Failed to parse SearchParameter for registry: {}", e);
            }
        }

        Ok(())
    }

    /// Handle update of a SearchParameter resource.
    ///
    /// Updates the registry based on status changes:
    /// - active -> retired: Parameter disabled for searches
    /// - retired -> active: Parameter re-enabled for searches
    /// - Any other change: Updates the registry entry
    fn handle_search_parameter_update(
        &self,
        old_resource: &Value,
        new_resource: &Value,
    ) -> StorageResult<()> {
        let loader = SearchParameterLoader::new(FhirVersion::default_enabled());

        let old_def = loader.parse_resource(old_resource).ok();
        let new_def = loader.parse_resource(new_resource).ok();

        match (old_def, new_def) {
            (Some(old), Some(new)) => {
                let mut registry = self.search_registry().write();

                // If URL changed, unregister old and register new
                if old.url != new.url {
                    let _ = registry.unregister(&old.url);
                    if new.status == SearchParameterStatus::Active {
                        let _ = registry.register(new);
                    }
                } else if old.status != new.status {
                    // Status change - update in registry
                    if let Err(e) = registry.update_status(&new.url, new.status) {
                        tracing::debug!("SearchParameter status update skipped: {}", e);
                    }
                } else {
                    // Other changes - re-register (unregister then register)
                    let _ = registry.unregister(&old.url);
                    if new.status == SearchParameterStatus::Active {
                        let _ = registry.register(new);
                    }
                }
            }
            (None, Some(new)) => {
                // Old wasn't valid, try to register new
                if new.status == SearchParameterStatus::Active {
                    let mut registry = self.search_registry().write();
                    let _ = registry.register(new);
                }
            }
            (Some(old), None) => {
                // New isn't valid, unregister old
                let mut registry = self.search_registry().write();
                let _ = registry.unregister(&old.url);
            }
            (None, None) => {
                // Neither valid - nothing to do
            }
        }

        Ok(())
    }

    /// Handle deletion of a SearchParameter resource.
    ///
    /// Removes the parameter from the registry. Search index entries for this
    /// parameter are NOT automatically cleaned up (use $reindex for that).
    fn handle_search_parameter_delete(&self, resource: &Value) -> StorageResult<()> {
        if let Some(url) = resource.get("url").and_then(|v| v.as_str()) {
            let mut registry = self.search_registry().write();
            if let Err(e) = registry.unregister(url) {
                tracing::debug!("SearchParameter unregistration skipped: {}", e);
            }
        }

        Ok(())
    }
}

#[async_trait]
impl VersionedStorage for SqliteBackend {
    async fn vread(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        version_id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let result = conn.query_row(
            "SELECT data, last_updated, is_deleted, fhir_version
             FROM resource_history
             WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3 AND version_id = ?4",
            params![tenant_id, resource_type, id, version_id],
            |row| {
                let data: Vec<u8> = row.get(0)?;
                let last_updated: String = row.get(1)?;
                let is_deleted: i32 = row.get(2)?;
                let fhir_version: String = row.get(3)?;
                Ok((data, last_updated, is_deleted, fhir_version))
            },
        );

        match result {
            Ok((data, last_updated, is_deleted, fhir_version_str)) => {
                let json_data: serde_json::Value = serde_json::from_slice(&data).map_err(|e| {
                    serialization_error(format!("Failed to deserialize resource: {}", e))
                })?;

                let last_updated = chrono::DateTime::parse_from_rfc3339(&last_updated)
                    .map_err(|e| internal_error(format!("Failed to parse last_updated: {}", e)))?
                    .with_timezone(&Utc);

                // For deleted versions, use last_updated as deleted_at
                let deleted_at = if is_deleted != 0 {
                    Some(last_updated)
                } else {
                    None
                };

                let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                    .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

                Ok(Some(StoredResource::from_storage(
                    resource_type,
                    id,
                    version_id,
                    tenant.tenant_id().clone(),
                    json_data,
                    last_updated,
                    last_updated,
                    deleted_at,
                    fhir_version,
                )))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(internal_error(format!("Failed to read version: {}", e))),
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

        // Check version match
        if current.version_id() != expected_version {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: expected_version.to_string(),
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
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check version match
        let current_version: Result<String, _> = conn.query_row(
            "SELECT version_id FROM resources
             WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3 AND is_deleted = 0",
            params![tenant_id, resource_type, id],
            |row| row.get(0),
        );

        let current_version = match current_version {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
            }
            Err(e) => {
                return Err(internal_error(format!(
                    "Failed to get current version: {}",
                    e
                )));
            }
        };

        if current_version != expected_version {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: expected_version.to_string(),
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
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let mut stmt = conn
            .prepare(
                "SELECT version_id FROM resource_history
                 WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3
                 ORDER BY CAST(version_id AS INTEGER) ASC",
            )
            .map_err(|e| internal_error(format!("Failed to prepare query: {}", e)))?;

        let versions = stmt
            .query_map(params![tenant_id, resource_type, id], |row| row.get(0))
            .map_err(|e| internal_error(format!("Failed to list versions: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(versions)
    }
}

#[async_trait]
impl InstanceHistoryProvider for SqliteBackend {
    async fn history_instance(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        params: &HistoryParams,
    ) -> StorageResult<HistoryPage> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Build the query with filters
        let mut sql = String::from(
            "SELECT version_id, data, last_updated, is_deleted, fhir_version
             FROM resource_history
             WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3",
        );

        // Apply deleted filter
        if !params.include_deleted {
            sql.push_str(" AND is_deleted = 0");
        }

        // Apply since filter
        if let Some(since) = &params.since {
            sql.push_str(&format!(" AND last_updated >= '{}'", since.to_rfc3339()));
        }

        // Apply before filter
        if let Some(before) = &params.before {
            sql.push_str(&format!(" AND last_updated < '{}'", before.to_rfc3339()));
        }

        // Apply cursor filter if present
        if let Some(cursor) = params.pagination.cursor_value() {
            // Cursor contains version_id for history pagination
            if let Some(CursorValue::String(version_str)) = cursor.sort_values().first() {
                // For reverse chronological order, get versions less than cursor
                sql.push_str(&format!(
                    " AND CAST(version_id AS INTEGER) < {}",
                    version_str.parse::<i64>().unwrap_or(i64::MAX)
                ));
            }
        }

        // Order by version descending (newest first) and limit
        sql.push_str(" ORDER BY CAST(version_id AS INTEGER) DESC");
        sql.push_str(&format!(" LIMIT {}", params.pagination.count + 1)); // +1 to detect if there are more

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| internal_error(format!("Failed to prepare history query: {}", e)))?;

        let rows = stmt
            .query_map(params![tenant_id, resource_type, id], |row| {
                let version_id: String = row.get(0)?;
                let data: Vec<u8> = row.get(1)?;
                let last_updated: String = row.get(2)?;
                let is_deleted: i32 = row.get(3)?;
                let fhir_version: String = row.get(4)?;
                Ok((version_id, data, last_updated, is_deleted, fhir_version))
            })
            .map_err(|e| internal_error(format!("Failed to query history: {}", e)))?;

        let mut entries = Vec::new();
        let mut last_version: Option<String> = None;

        for row in rows {
            let (version_id, data, last_updated_str, is_deleted, fhir_version_str) =
                row.map_err(|e| internal_error(format!("Failed to read history row: {}", e)))?;

            // Stop if we've collected enough items (we fetched count+1 to detect more)
            if entries.len() >= params.pagination.count as usize {
                break;
            }

            let json_data: serde_json::Value = serde_json::from_slice(&data).map_err(|e| {
                serialization_error(format!("Failed to deserialize resource: {}", e))
            })?;

            let last_updated = chrono::DateTime::parse_from_rfc3339(&last_updated_str)
                .map_err(|e| internal_error(format!("Failed to parse last_updated: {}", e)))?
                .with_timezone(&Utc);

            let deleted_at = if is_deleted != 0 {
                Some(last_updated)
            } else {
                None
            };

            let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

            let resource = StoredResource::from_storage(
                resource_type,
                id,
                &version_id,
                tenant.tenant_id().clone(),
                json_data,
                last_updated,
                last_updated,
                deleted_at,
                fhir_version,
            );

            // Determine the method based on version and deletion status
            let method = if is_deleted != 0 {
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
        let has_more = stmt
            .query_map(params![tenant_id, resource_type, id], |_| Ok(()))
            .map_err(|e| internal_error(format!("Failed to check for more results: {}", e)))?
            .count()
            > params.pagination.count as usize;

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
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM resource_history
                 WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3",
                params![tenant_id, resource_type, id],
                |row| row.get(0),
            )
            .map_err(|e| internal_error(format!("Failed to count history: {}", e)))?;

        Ok(count as u64)
    }

    /// Deletes all history for a specific resource instance.
    ///
    /// This is a FHIR v6.0.0 Trial Use feature. After this operation:
    /// - All historical versions are removed from resource_history
    /// - The current version in the resources table is preserved
    /// - The resource continues to be accessible via normal read operations
    ///
    /// # Returns
    ///
    /// The number of history entries deleted.
    async fn delete_instance_history(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<u64> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // First, verify the resource exists
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3",
                params![tenant_id, resource_type, id],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if !exists {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        }

        // Get the current version from resources table (to preserve it)
        let current_version: String = conn
            .query_row(
                "SELECT version_id FROM resources
                 WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3",
                params![tenant_id, resource_type, id],
                |row| row.get(0),
            )
            .map_err(|e| internal_error(format!("Failed to get current version: {}", e)))?;

        // Delete all history entries EXCEPT the current version
        // This preserves the current version in history as well
        let deleted = conn
            .execute(
                "DELETE FROM resource_history
                 WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3 AND version_id != ?4",
                params![tenant_id, resource_type, id, current_version],
            )
            .map_err(|e| internal_error(format!("Failed to delete history: {}", e)))?;

        Ok(deleted as u64)
    }

    /// Deletes a specific version from a resource's history.
    ///
    /// This is a FHIR v6.0.0 Trial Use feature. Restrictions:
    /// - Cannot delete the current version (use regular delete instead)
    /// - The version must exist in the history
    async fn delete_version(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        version_id: &str,
    ) -> StorageResult<()> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // First, get the current version to ensure we're not deleting it
        let current_version: Result<String, _> = conn.query_row(
            "SELECT version_id FROM resources
             WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3",
            params![tenant_id, resource_type, id],
            |row| row.get(0),
        );

        let current_version = match current_version {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
            }
            Err(e) => {
                return Err(internal_error(format!(
                    "Failed to get current version: {}",
                    e
                )));
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
        let version_exists: bool = conn
            .query_row(
                "SELECT 1 FROM resource_history
                 WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3 AND version_id = ?4",
                params![tenant_id, resource_type, id, version_id],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if !version_exists {
            return Err(StorageError::Resource(ResourceError::VersionNotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
                version_id: version_id.to_string(),
            }));
        }

        // Delete the specific version
        conn.execute(
            "DELETE FROM resource_history
             WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3 AND version_id = ?4",
            params![tenant_id, resource_type, id, version_id],
        )
        .map_err(|e| internal_error(format!("Failed to delete version: {}", e)))?;

        Ok(())
    }
}

#[async_trait]
impl TypeHistoryProvider for SqliteBackend {
    async fn history_type(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        params: &HistoryParams,
    ) -> StorageResult<HistoryPage> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Build the query with filters
        let mut sql = String::from(
            "SELECT id, version_id, data, last_updated, is_deleted, fhir_version
             FROM resource_history
             WHERE tenant_id = ?1 AND resource_type = ?2",
        );

        // Apply deleted filter
        if !params.include_deleted {
            sql.push_str(" AND is_deleted = 0");
        }

        // Apply since filter
        if let Some(since) = &params.since {
            sql.push_str(&format!(" AND last_updated >= '{}'", since.to_rfc3339()));
        }

        // Apply before filter
        if let Some(before) = &params.before {
            sql.push_str(&format!(" AND last_updated < '{}'", before.to_rfc3339()));
        }

        // Apply cursor filter if present
        // For type history, cursor contains (last_updated, id, version_id) for proper ordering
        if let Some(cursor) = params.pagination.cursor_value() {
            let sort_values = cursor.sort_values();
            if sort_values.len() >= 2 {
                if let (
                    Some(CursorValue::String(timestamp)),
                    Some(CursorValue::String(resource_id)),
                ) = (sort_values.first(), sort_values.get(1))
                {
                    // For reverse chronological order, get entries older than cursor
                    sql.push_str(&format!(
                        " AND (last_updated < '{}' OR (last_updated = '{}' AND id < '{}'))",
                        timestamp, timestamp, resource_id
                    ));
                }
            }
        }

        // Order by last_updated descending (newest first), then by id for consistency
        sql.push_str(" ORDER BY last_updated DESC, id DESC, CAST(version_id AS INTEGER) DESC");
        sql.push_str(&format!(" LIMIT {}", params.pagination.count + 1)); // +1 to detect if there are more

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| internal_error(format!("Failed to prepare type history query: {}", e)))?;

        let rows = stmt
            .query_map(params![tenant_id, resource_type], |row| {
                let id: String = row.get(0)?;
                let version_id: String = row.get(1)?;
                let data: Vec<u8> = row.get(2)?;
                let last_updated: String = row.get(3)?;
                let is_deleted: i32 = row.get(4)?;
                let fhir_version: String = row.get(5)?;
                Ok((id, version_id, data, last_updated, is_deleted, fhir_version))
            })
            .map_err(|e| internal_error(format!("Failed to query type history: {}", e)))?;

        let mut entries = Vec::new();
        let mut last_entry: Option<(String, String)> = None; // (last_updated, id)

        for row in rows {
            let (id, version_id, data, last_updated_str, is_deleted, fhir_version_str) =
                row.map_err(|e| internal_error(format!("Failed to read type history row: {}", e)))?;

            // Stop if we've collected enough items (we fetched count+1 to detect more)
            if entries.len() >= params.pagination.count as usize {
                break;
            }

            let json_data: serde_json::Value = serde_json::from_slice(&data).map_err(|e| {
                serialization_error(format!("Failed to deserialize resource: {}", e))
            })?;

            let last_updated = chrono::DateTime::parse_from_rfc3339(&last_updated_str)
                .map_err(|e| internal_error(format!("Failed to parse last_updated: {}", e)))?
                .with_timezone(&Utc);

            let deleted_at = if is_deleted != 0 {
                Some(last_updated)
            } else {
                None
            };

            let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

            let resource = StoredResource::from_storage(
                resource_type,
                &id,
                &version_id,
                tenant.tenant_id().clone(),
                json_data,
                last_updated,
                last_updated,
                deleted_at,
                fhir_version,
            );

            // Determine the method based on version and deletion status
            let method = if is_deleted != 0 {
                HistoryMethod::Delete
            } else if version_id == "1" {
                HistoryMethod::Post
            } else {
                HistoryMethod::Put
            };

            last_entry = Some((last_updated_str.clone(), id));

            entries.push(HistoryEntry {
                resource,
                method,
                timestamp: last_updated,
            });
        }

        // Check if there are more results by seeing if we got more than count
        let _total_fetched = entries.len();
        let has_more = {
            // Re-run query to check if there are more
            let check_sql = sql.replace(
                &format!(" LIMIT {}", params.pagination.count + 1),
                &format!(" LIMIT {}", params.pagination.count + 2),
            );
            let mut check_stmt = conn
                .prepare(&check_sql)
                .map_err(|e| internal_error(format!("Failed to prepare check query: {}", e)))?;
            let check_count = check_stmt
                .query_map(params![tenant_id, resource_type], |_| Ok(()))
                .map_err(|e| internal_error(format!("Failed to check for more results: {}", e)))?
                .count();
            check_count > params.pagination.count as usize
        };

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
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM resource_history
                 WHERE tenant_id = ?1 AND resource_type = ?2",
                params![tenant_id, resource_type],
                |row| row.get(0),
            )
            .map_err(|e| internal_error(format!("Failed to count type history: {}", e)))?;

        Ok(count as u64)
    }
}

#[async_trait]
impl SystemHistoryProvider for SqliteBackend {
    async fn history_system(
        &self,
        tenant: &TenantContext,
        params: &HistoryParams,
    ) -> StorageResult<HistoryPage> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Build the query with filters
        let mut sql = String::from(
            "SELECT resource_type, id, version_id, data, last_updated, is_deleted, fhir_version
             FROM resource_history
             WHERE tenant_id = ?1",
        );

        // Apply deleted filter
        if !params.include_deleted {
            sql.push_str(" AND is_deleted = 0");
        }

        // Apply since filter
        if let Some(since) = &params.since {
            sql.push_str(&format!(" AND last_updated >= '{}'", since.to_rfc3339()));
        }

        // Apply before filter
        if let Some(before) = &params.before {
            sql.push_str(&format!(" AND last_updated < '{}'", before.to_rfc3339()));
        }

        // Apply cursor filter if present
        // For system history, cursor contains (last_updated, resource_type, id) for proper ordering
        if let Some(cursor) = params.pagination.cursor_value() {
            let sort_values = cursor.sort_values();
            if sort_values.len() >= 3 {
                if let (
                    Some(CursorValue::String(timestamp)),
                    Some(CursorValue::String(res_type)),
                    Some(CursorValue::String(res_id)),
                ) = (sort_values.first(), sort_values.get(1), sort_values.get(2))
                {
                    // For reverse chronological order, get entries older than cursor
                    sql.push_str(&format!(
                        " AND (last_updated < '{}' OR (last_updated = '{}' AND (resource_type < '{}' OR (resource_type = '{}' AND id < '{}'))))",
                        timestamp, timestamp, res_type, res_type, res_id
                    ));
                }
            }
        }

        // Order by last_updated descending (newest first), then by resource_type and id for consistency
        sql.push_str(" ORDER BY last_updated DESC, resource_type DESC, id DESC, CAST(version_id AS INTEGER) DESC");
        sql.push_str(&format!(" LIMIT {}", params.pagination.count + 1)); // +1 to detect if there are more

        let mut stmt = conn.prepare(&sql).map_err(|e| {
            internal_error(format!("Failed to prepare system history query: {}", e))
        })?;

        let rows = stmt
            .query_map(params![tenant_id], |row| {
                let resource_type: String = row.get(0)?;
                let id: String = row.get(1)?;
                let version_id: String = row.get(2)?;
                let data: Vec<u8> = row.get(3)?;
                let last_updated: String = row.get(4)?;
                let is_deleted: i32 = row.get(5)?;
                let fhir_version: String = row.get(6)?;
                Ok((
                    resource_type,
                    id,
                    version_id,
                    data,
                    last_updated,
                    is_deleted,
                    fhir_version,
                ))
            })
            .map_err(|e| internal_error(format!("Failed to query system history: {}", e)))?;

        let mut entries = Vec::new();
        let mut last_entry: Option<(String, String, String)> = None; // (last_updated, resource_type, id)

        for row in rows {
            let (
                resource_type,
                id,
                version_id,
                data,
                last_updated_str,
                is_deleted,
                fhir_version_str,
            ) = row
                .map_err(|e| internal_error(format!("Failed to read system history row: {}", e)))?;

            // Stop if we've collected enough items (we fetched count+1 to detect more)
            if entries.len() >= params.pagination.count as usize {
                break;
            }

            let json_data: serde_json::Value = serde_json::from_slice(&data).map_err(|e| {
                serialization_error(format!("Failed to deserialize resource: {}", e))
            })?;

            let last_updated = chrono::DateTime::parse_from_rfc3339(&last_updated_str)
                .map_err(|e| internal_error(format!("Failed to parse last_updated: {}", e)))?
                .with_timezone(&Utc);

            let deleted_at = if is_deleted != 0 {
                Some(last_updated)
            } else {
                None
            };

            let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

            let resource = StoredResource::from_storage(
                &resource_type,
                &id,
                &version_id,
                tenant.tenant_id().clone(),
                json_data,
                last_updated,
                last_updated,
                deleted_at,
                fhir_version,
            );

            // Determine the method based on version and deletion status
            let method = if is_deleted != 0 {
                HistoryMethod::Delete
            } else if version_id == "1" {
                HistoryMethod::Post
            } else {
                HistoryMethod::Put
            };

            last_entry = Some((last_updated_str.clone(), resource_type, id));

            entries.push(HistoryEntry {
                resource,
                method,
                timestamp: last_updated,
            });
        }

        // Check if there are more results
        let has_more = {
            let check_sql = sql.replace(
                &format!(" LIMIT {}", params.pagination.count + 1),
                &format!(" LIMIT {}", params.pagination.count + 2),
            );
            let mut check_stmt = conn
                .prepare(&check_sql)
                .map_err(|e| internal_error(format!("Failed to prepare check query: {}", e)))?;
            let check_count = check_stmt
                .query_map(params![tenant_id], |_| Ok(()))
                .map_err(|e| internal_error(format!("Failed to check for more results: {}", e)))?
                .count();
            check_count > params.pagination.count as usize
        };

        // Build page info
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
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM resource_history WHERE tenant_id = ?1",
                params![tenant_id],
                |row| row.get(0),
            )
            .map_err(|e| internal_error(format!("Failed to count system history: {}", e)))?;

        Ok(count as u64)
    }
}

#[async_trait]
impl PurgableStorage for SqliteBackend {
    async fn purge(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check if resource exists (in any state)
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3",
                params![tenant_id, resource_type, id],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if !exists {
            // Also check history in case it was already purged from main table
            let history_exists: bool = conn
                .query_row(
                    "SELECT 1 FROM resource_history WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3",
                    params![tenant_id, resource_type, id],
                    |_| Ok(true),
                )
                .unwrap_or(false);

            if !history_exists {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
            }
        }

        // Delete from resources table
        conn.execute(
            "DELETE FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3",
            params![tenant_id, resource_type, id],
        )
        .map_err(|e| internal_error(format!("Failed to purge resource: {}", e)))?;

        // Delete from history table
        conn.execute(
            "DELETE FROM resource_history WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3",
            params![tenant_id, resource_type, id],
        )
        .map_err(|e| internal_error(format!("Failed to purge resource history: {}", e)))?;

        // Delete from search index
        conn.execute(
            "DELETE FROM search_index WHERE tenant_id = ?1 AND resource_type = ?2 AND resource_id = ?3",
            params![tenant_id, resource_type, id],
        )
        .map_err(|e| internal_error(format!("Failed to purge search index: {}", e)))?;

        Ok(())
    }

    async fn purge_all(&self, tenant: &TenantContext, resource_type: &str) -> StorageResult<u64> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Count how many we're about to delete
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT id) FROM resources WHERE tenant_id = ?1 AND resource_type = ?2",
                params![tenant_id, resource_type],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Delete from resources table
        conn.execute(
            "DELETE FROM resources WHERE tenant_id = ?1 AND resource_type = ?2",
            params![tenant_id, resource_type],
        )
        .map_err(|e| internal_error(format!("Failed to purge resources: {}", e)))?;

        // Delete from history table
        conn.execute(
            "DELETE FROM resource_history WHERE tenant_id = ?1 AND resource_type = ?2",
            params![tenant_id, resource_type],
        )
        .map_err(|e| internal_error(format!("Failed to purge resource history: {}", e)))?;

        // Delete from search index
        conn.execute(
            "DELETE FROM search_index WHERE tenant_id = ?1 AND resource_type = ?2",
            params![tenant_id, resource_type],
        )
        .map_err(|e| internal_error(format!("Failed to purge search index: {}", e)))?;

        Ok(count as u64)
    }
}

#[async_trait]
impl DifferentialHistoryProvider for SqliteBackend {
    async fn modified_since(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
        since: chrono::DateTime<Utc>,
        pagination: &Pagination,
    ) -> StorageResult<Page<StoredResource>> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();
        let since_str = since.to_rfc3339();

        // Build query for current versions of resources modified since timestamp
        let mut sql = String::from(
            "SELECT resource_type, id, version_id, data, last_updated, fhir_version
             FROM resources
             WHERE tenant_id = ?1 AND last_updated > ?2 AND is_deleted = 0",
        );

        // Filter by resource type if specified
        if let Some(rt) = resource_type {
            sql.push_str(&format!(" AND resource_type = '{}'", rt));
        }

        // Apply cursor filter if present
        if let Some(cursor) = pagination.cursor_value() {
            let sort_values = cursor.sort_values();
            if sort_values.len() >= 2 {
                if let (Some(CursorValue::String(timestamp)), Some(CursorValue::String(res_id))) =
                    (sort_values.first(), sort_values.get(1))
                {
                    sql.push_str(&format!(
                        " AND (last_updated > '{}' OR (last_updated = '{}' AND id > '{}'))",
                        timestamp, timestamp, res_id
                    ));
                }
            }
        }

        // Order by last_updated ascending (oldest first for sync)
        sql.push_str(" ORDER BY last_updated ASC, id ASC");
        sql.push_str(&format!(" LIMIT {}", pagination.count + 1));

        let mut stmt = conn.prepare(&sql).map_err(|e| {
            internal_error(format!("Failed to prepare modified_since query: {}", e))
        })?;

        let rows = stmt
            .query_map(params![tenant_id, since_str], |row| {
                let resource_type: String = row.get(0)?;
                let id: String = row.get(1)?;
                let version_id: String = row.get(2)?;
                let data: Vec<u8> = row.get(3)?;
                let last_updated: String = row.get(4)?;
                let fhir_version: String = row.get(5)?;
                Ok((
                    resource_type,
                    id,
                    version_id,
                    data,
                    last_updated,
                    fhir_version,
                ))
            })
            .map_err(|e| internal_error(format!("Failed to query modified resources: {}", e)))?;

        let mut resources = Vec::new();
        let mut last_entry: Option<(String, String)> = None; // (last_updated, id)

        for row in rows {
            let (resource_type, id, version_id, data, last_updated_str, fhir_version_str) =
                row.map_err(|e| internal_error(format!("Failed to read row: {}", e)))?;

            // Stop if we've collected enough items
            if resources.len() >= pagination.count as usize {
                break;
            }

            let json_data: serde_json::Value = serde_json::from_slice(&data).map_err(|e| {
                serialization_error(format!("Failed to deserialize resource: {}", e))
            })?;

            let last_updated = chrono::DateTime::parse_from_rfc3339(&last_updated_str)
                .map_err(|e| internal_error(format!("Failed to parse last_updated: {}", e)))?
                .with_timezone(&Utc);

            let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

            let resource = StoredResource::from_storage(
                &resource_type,
                &id,
                &version_id,
                tenant.tenant_id().clone(),
                json_data,
                last_updated,
                last_updated,
                None,
                fhir_version,
            );

            last_entry = Some((last_updated_str, id));
            resources.push(resource);
        }

        // Check if there are more results
        let has_more = {
            let check_sql = sql.replace(
                &format!(" LIMIT {}", pagination.count + 1),
                &format!(" LIMIT {}", pagination.count + 2),
            );
            let mut check_stmt = conn
                .prepare(&check_sql)
                .map_err(|e| internal_error(format!("Failed to prepare check query: {}", e)))?;
            let check_count = check_stmt
                .query_map(params![tenant_id, since_str], |_| Ok(()))
                .map_err(|e| internal_error(format!("Failed to check for more results: {}", e)))?
                .count();
            check_count > pagination.count as usize
        };

        // Build page info
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
impl ConditionalStorage for SqliteBackend {
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

    /// Patches a resource based on search criteria.
    ///
    /// This implements conditional patch as defined in FHIR:
    /// `PATCH [base]/[type]?[search-params]`
    ///
    /// Supports three patch formats:
    /// - JSON Patch (RFC 6902)
    /// - FHIRPath Patch (FHIR-specific)
    /// - JSON Merge Patch (RFC 7386)
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

impl SqliteBackend {
    /// Find resources matching the given search parameters.
    ///
    /// Uses the SearchProvider implementation to leverage the pre-computed search index,
    /// ensuring consistent search behavior with the main search API.
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
            // Return empty to avoid unintended matches
            return Ok(Vec::new());
        }

        // Build SearchParameter objects by looking up types from the registry
        let search_params = self.build_search_parameters(resource_type, &parsed_params)?;

        // Build a SearchQuery
        let query = SearchQuery {
            resource_type: resource_type.to_string(),
            parameters: search_params,
            // No pagination limit for conditional operations - we need all matches
            count: Some(1000), // Reasonable upper limit for conditional matching
            ..Default::default()
        };

        // Use the SearchProvider implementation which uses the search index
        let result = <Self as SearchProvider>::search(self, tenant, &query).await?;

        Ok(result.resources.items)
    }

    /// Builds SearchParameter objects from parsed (name, value) pairs.
    ///
    /// Looks up the parameter type from the registry, falling back to sensible defaults
    /// for common parameters when not found.
    fn build_search_parameters(
        &self,
        resource_type: &str,
        params: &[(String, String)],
    ) -> StorageResult<Vec<SearchParameter>> {
        let registry = self.search_registry().read();
        let mut search_params = Vec::with_capacity(params.len());

        for (name, value) in params {
            // Look up the parameter definition to get its type
            let param_type = self
                .lookup_param_type(&registry, resource_type, name)
                .unwrap_or({
                    // Fallback for common parameters when not in registry
                    match name.as_str() {
                        "_id" => SearchParamType::Token,
                        "_lastUpdated" => SearchParamType::Date,
                        "_tag" | "_profile" | "_security" => SearchParamType::Token,
                        "identifier" => SearchParamType::Token,
                        // Common reference parameters across many resource types
                        "patient" | "subject" | "encounter" | "performer" | "author"
                        | "requester" | "recorder" | "asserter" | "practitioner"
                        | "organization" | "location" | "device" => SearchParamType::Reference,
                        _ => SearchParamType::String, // Default fallback
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
    ///
    /// Checks both the specific resource type and "Resource" base type for common params.
    fn lookup_param_type(
        &self,
        registry: &crate::search::SearchParameterRegistry,
        resource_type: &str,
        param_name: &str,
    ) -> Option<SearchParamType> {
        // First try the specific resource type
        if let Some(def) = registry.get_param(resource_type, param_name) {
            return Some(def.param_type);
        }

        // Then try "Resource" for common parameters like _id, _lastUpdated
        if let Some(def) = registry.get_param("Resource", param_name) {
            return Some(def.param_type);
        }

        None
    }

    // ========================================================================
    // Patch Helper Methods
    // ========================================================================

    /// Applies a JSON Patch (RFC 6902) to a resource.
    ///
    /// JSON Patch operations:
    /// - `add`: Add a value at the specified path
    /// - `remove`: Remove the value at the specified path
    /// - `replace`: Replace the value at the specified path
    /// - `move`: Move a value from one path to another
    /// - `copy`: Copy a value from one path to another
    /// - `test`: Test that a value equals the expected value
    fn apply_json_patch(&self, resource: &Value, patch_doc: &Value) -> StorageResult<Value> {
        use crate::error::ValidationError;

        // Parse the patch document as an array of operations
        let patch: json_patch::Patch = serde_json::from_value(patch_doc.clone()).map_err(|e| {
            StorageError::Validation(ValidationError::InvalidResource {
                message: format!("Invalid JSON Patch document: {}", e),
                details: vec![],
            })
        })?;

        // Apply the patch to a mutable copy
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
    ///
    /// FHIRPath Patch uses a Parameters resource with operation parts:
    /// - `type`: add, insert, delete, replace, move
    /// - `path`: FHIRPath expression
    /// - `name`: element name (for add)
    /// - `value`: new value
    ///
    /// Note: Full FHIRPath Patch support requires the helios-fhirpath evaluator.
    /// This implementation handles common cases.
    fn apply_fhirpath_patch(&self, resource: &Value, patch_params: &Value) -> StorageResult<Value> {
        use crate::error::ValidationError;

        // The patch_params should be a Parameters resource with operation parts
        let parameter = patch_params.get("parameter").and_then(|p| p.as_array());
        if parameter.is_none() {
            return Err(StorageError::Validation(ValidationError::InvalidResource {
                message: "FHIRPath Patch must have a 'parameter' array".to_string(),
                details: vec![],
            }));
        }

        let mut patched = resource.clone();

        for operation in parameter.unwrap() {
            // Each operation has parts with name "type", "path", "name", "value"
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

            // Apply the operation based on type
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
        // Simple implementation for common paths like "Resource.field"
        // Full implementation would use helios-fhirpath for path evaluation
        let parts: Vec<&str> = path.split('.').collect();
        if parts.len() == 2 {
            // Simple path like "Patient.active"
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
        // Simple implementation for adding to root or nested object
        let parts: Vec<&str> = path.split('.').collect();
        if parts.len() == 1
            && parts[0]
                == resource
                    .get("resourceType")
                    .and_then(|r| r.as_str())
                    .unwrap_or("")
        {
            // Adding to root level
            if let Some(obj) = resource.as_object_mut() {
                obj.insert(name.to_string(), value.clone());
            }
        }
        Ok(())
    }

    /// Helper for FHIRPath delete operation.
    fn fhirpath_delete(&self, resource: &mut Value, path: &str) -> StorageResult<()> {
        // Simple implementation for deleting fields
        let parts: Vec<&str> = path.split('.').collect();
        if parts.len() == 2 {
            if let Some(obj) = resource.as_object_mut() {
                obj.remove(parts[1]);
            }
        }
        Ok(())
    }

    /// Applies a JSON Merge Patch (RFC 7386) to a resource.
    ///
    /// Merge Patch is simpler than JSON Patch:
    /// - Fields in the patch replace those in the target
    /// - null values remove fields from the target
    /// - Nested objects are merged recursively
    fn apply_merge_patch(&self, resource: &Value, merge_doc: &Value) -> Value {
        let mut patched = resource.clone();
        json_patch::merge(&mut patched, merge_doc);
        patched
    }
}

#[async_trait]
impl BundleProvider for SqliteBackend {
    async fn process_transaction(
        &self,
        tenant: &TenantContext,
        entries: Vec<BundleEntry>,
    ) -> Result<BundleResult, TransactionError> {
        use crate::core::transaction::{Transaction, TransactionOptions, TransactionProvider};
        use std::collections::HashMap;

        // Start a transaction
        let mut tx = self
            .begin_transaction(tenant, TransactionOptions::new())
            .await
            .map_err(|e| TransactionError::RolledBack {
                reason: format!("Failed to begin transaction: {}", e),
            })?;

        let mut results = Vec::with_capacity(entries.len());
        let mut error_info: Option<(usize, String)> = None;

        // Build a map of fullUrl -> assigned reference for reference resolution
        // This maps urn:uuid:xxx to ResourceType/assigned-id after creates
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
                    // Check for error status codes
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
                                // location is in format "ResourceType/id/_history/version"
                                // Extract "ResourceType/id"
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
    ) -> StorageResult<BundleResult> {
        let mut results = Vec::with_capacity(entries.len());

        // Process each entry independently
        for entry in &entries {
            let result = self.process_batch_entry(tenant, entry).await;
            results.push(result);
        }

        Ok(BundleResult {
            bundle_type: BundleType::Batch,
            entries: results,
        })
    }
}

impl SqliteBackend {
    /// Process a single bundle entry within a transaction.
    async fn process_bundle_entry_tx(
        &self,
        tx: &mut crate::backends::sqlite::transaction::SqliteTransaction,
        entry: &BundleEntry,
    ) -> StorageResult<BundleEntryResult> {
        use crate::core::transaction::Transaction;

        match entry.method {
            BundleMethod::Get => {
                // Parse resource type and ID from URL
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
                // Create new resource
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
                // Update or create resource
                let resource = entry.resource.clone().ok_or_else(|| {
                    StorageError::Validation(crate::error::ValidationError::MissingRequiredField {
                        field: "resource".to_string(),
                    })
                })?;

                let (resource_type, id) = self.parse_url(&entry.url)?;

                // Check if resource exists
                match tx.read(&resource_type, &id).await? {
                    Some(existing) => {
                        // Check If-Match if provided
                        if let Some(ref if_match) = entry.if_match {
                            let current_etag = existing.etag();
                            if current_etag != if_match.as_str() {
                                return Ok(BundleEntryResult::error(
                                    412,
                                    serde_json::json!({
                                        "resourceType": "OperationOutcome",
                                        "issue": [{"severity": "error", "code": "conflict", "diagnostics": "ETag mismatch"}]
                                    }),
                                ));
                            }
                        }
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
                tx.delete(&resource_type, &id).await?;
                Ok(BundleEntryResult::deleted())
            }
            BundleMethod::Patch => {
                // PATCH is not fully implemented yet
                Ok(BundleEntryResult::error(
                    501,
                    serde_json::json!({
                        "resourceType": "OperationOutcome",
                        "issue": [{"severity": "error", "code": "not-supported", "diagnostics": "PATCH not implemented"}]
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
    ) -> BundleEntryResult {
        match self.process_batch_entry_inner(tenant, entry).await {
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

                // Use default FHIR version for bundle operations
                let created = self
                    .create(
                        tenant,
                        &resource_type,
                        resource,
                        FhirVersion::default_enabled(),
                    )
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
                // Use default FHIR version for bundle operations
                let (stored, _created) = self
                    .create_or_update(
                        tenant,
                        &resource_type,
                        &id,
                        resource,
                        FhirVersion::default_enabled(),
                    )
                    .await?;
                Ok(BundleEntryResult::ok(stored))
            }
            BundleMethod::Delete => {
                let (resource_type, id) = self.parse_url(&entry.url)?;
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
        // Handle formats like:
        // - Patient/123
        // - /Patient/123
        // - http://example.com/fhir/Patient/123
        let path = url
            .strip_prefix("http://")
            .or_else(|| url.strip_prefix("https://"))
            .map(|s| {
                // Find the path part after the host
                s.find('/').map(|i| &s[i..]).unwrap_or(s)
            })
            .unwrap_or(url);

        let path = path.trim_start_matches('/');
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        // Take the last two parts (resource type and ID)
        // This handles URLs like /fhir/Patient/123 where we want Patient/123
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
///
/// This function walks through the JSON structure and replaces any `reference` fields
/// that contain urn:uuid: values with the corresponding resource references from the map.
fn resolve_bundle_references(
    value: &mut serde_json::Value,
    reference_map: &std::collections::HashMap<String, String>,
) {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            // Check if this is a Reference with a urn:uuid reference
            if let Some(Value::String(ref_str)) = map.get("reference") {
                if ref_str.starts_with("urn:uuid:") {
                    if let Some(resolved) = reference_map.get(ref_str) {
                        map.insert("reference".to_string(), Value::String(resolved.clone()));
                    }
                }
            }
            // Recurse into all values
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

// ReindexableStorage implementation for SQLite backend.
#[async_trait]
impl ReindexableStorage for SqliteBackend {
    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str().to_string();

        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT resource_type FROM resources WHERE tenant_id = ?1 AND is_deleted = 0",
            )
            .map_err(|e| internal_error(format!("Failed to prepare statement: {}", e)))?;

        let types: Vec<String> = stmt
            .query_map([&tenant_id], |row| row.get(0))
            .map_err(|e| internal_error(format!("Failed to query resource types: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

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
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str().to_string();

        // Parse cursor if provided (format: "last_updated|id")
        let (cursor_ts, cursor_id) = if let Some(c) = cursor {
            let parts: Vec<&str> = c.split('|').collect();
            if parts.len() == 2 {
                (Some(parts[0].to_string()), Some(parts[1].to_string()))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        // Build query based on whether we have a cursor
        let (sql, params): (String, Vec<Box<dyn ToSql>>) =
            if let (Some(ts), Some(id)) = (&cursor_ts, &cursor_id) {
                (
                    "SELECT id, version_id, data, last_updated, fhir_version FROM resources \
                 WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0 \
                 AND (last_updated > ?3 OR (last_updated = ?3 AND id > ?4)) \
                 ORDER BY last_updated ASC, id ASC LIMIT ?5"
                        .to_string(),
                    vec![
                        Box::new(tenant_id.clone()) as Box<dyn ToSql>,
                        Box::new(resource_type.to_string()),
                        Box::new(ts.clone()),
                        Box::new(id.clone()),
                        Box::new(limit as i64),
                    ],
                )
            } else {
                (
                    "SELECT id, version_id, data, last_updated, fhir_version FROM resources \
                 WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0 \
                 ORDER BY last_updated ASC, id ASC LIMIT ?3"
                        .to_string(),
                    vec![
                        Box::new(tenant_id.clone()) as Box<dyn ToSql>,
                        Box::new(resource_type.to_string()),
                        Box::new(limit as i64),
                    ],
                )
            };

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| internal_error(format!("Failed to prepare statement: {}", e)))?;

        let param_refs: Vec<&dyn ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let resources: Vec<StoredResource> = stmt
            .query_map(param_refs.as_slice(), |row| {
                let id: String = row.get(0)?;
                let version_id: String = row.get(1)?;
                let data: Vec<u8> = row.get(2)?;
                let last_updated: String = row.get(3)?;
                let fhir_version: String = row.get(4)?;

                Ok((id, version_id, data, last_updated, fhir_version))
            })
            .map_err(|e| internal_error(format!("Failed to query resources: {}", e)))?
            .filter_map(|r| r.ok())
            .filter_map(|(id, version_id, data, last_updated, fhir_version_str)| {
                let content: Value = serde_json::from_slice(&data).ok()?;
                let last_modified = chrono::DateTime::parse_from_rfc3339(&last_updated)
                    .ok()?
                    .with_timezone(&Utc);
                let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                    .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);
                Some(StoredResource::from_storage(
                    resource_type.to_string(),
                    id,
                    version_id,
                    tenant.tenant_id().clone(),
                    content,
                    last_modified, // created_at (use last_modified as approximation)
                    last_modified,
                    None, // not deleted
                    fhir_version,
                ))
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

    async fn delete_search_entries(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<()> {
        let conn = self.get_connection()?;
        self.delete_search_index(
            &conn,
            tenant.tenant_id().as_str(),
            resource_type,
            resource_id,
        )
    }

    async fn write_search_entries(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> StorageResult<usize> {
        let conn = self.get_connection()?;

        // Use the dynamic extraction
        let values = self
            .search_extractor()
            .extract(resource, resource_type)
            .map_err(|e| internal_error(format!("Search parameter extraction failed: {}", e)))?;

        let mut count = 0;
        for value in values {
            self.write_index_entry(
                &conn,
                tenant.tenant_id().as_str(),
                resource_type,
                resource_id,
                &value,
            )?;
            count += 1;
        }

        // Re-index contained resources too, so `$reindex` rebuilds `_contained`
        // search entries.
        count += self.index_contained_resources(
            &conn,
            tenant.tenant_id().as_str(),
            resource_type,
            resource_id,
            resource,
        )?;

        Ok(count)
    }

    async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let deleted = conn
            .execute(
                "DELETE FROM search_index WHERE tenant_id = ?1",
                params![tenant_id],
            )
            .map_err(|e| internal_error(format!("Failed to clear search index: {}", e)))?;

        Ok(deleted as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::history::HistoryParams;
    use crate::tenant::{TenantId, TenantPermissions};
    use serde_json::json;
    use std::path::PathBuf;

    use crate::backends::sqlite::SqliteBackendConfig;

    fn create_test_backend() -> SqliteBackend {
        // Configure with data directory to load spec SearchParameters
        // Use the workspace root data directory
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));

        let config = SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        };
        let backend = SqliteBackend::with_config(":memory:", config).unwrap();
        backend.init_schema().unwrap();
        backend
    }

    fn create_test_tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        )
    }

    #[tokio::test]
    async fn test_create_and_read() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let resource = json!({
            "resourceType": "Patient",
            "name": [{"family": "Test", "given": ["User"]}]
        });

        // Create
        let created = backend
            .create(&tenant, "Patient", resource, FhirVersion::default())
            .await
            .unwrap();
        assert_eq!(created.resource_type(), "Patient");
        assert_eq!(created.version_id(), "1");

        // Read
        let read = backend
            .read(&tenant, "Patient", created.id())
            .await
            .unwrap();
        assert!(read.is_some());
        let read = read.unwrap();
        assert_eq!(read.version_id(), "1");
    }

    #[tokio::test]
    async fn test_create_with_id() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let resource = json!({
            "resourceType": "Patient",
            "id": "patient-123",
            "name": [{"family": "Test"}]
        });

        let created = backend
            .create(&tenant, "Patient", resource, FhirVersion::default())
            .await
            .unwrap();
        assert_eq!(created.id(), "patient-123");
    }

    #[tokio::test]
    async fn test_create_duplicate_fails() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let resource = json!({"id": "patient-1"});
        backend
            .create(&tenant, "Patient", resource.clone(), FhirVersion::default())
            .await
            .unwrap();

        let result = backend
            .create(&tenant, "Patient", resource, FhirVersion::default())
            .await;
        assert!(matches!(
            result,
            Err(StorageError::Resource(ResourceError::AlreadyExists { .. }))
        ));
    }

    #[tokio::test]
    async fn test_read_nonexistent() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let result = backend
            .read(&tenant, "Patient", "nonexistent")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_update() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create
        let resource = json!({"name": [{"family": "Original"}]});
        let created = backend
            .create(&tenant, "Patient", resource, FhirVersion::default())
            .await
            .unwrap();

        // Update
        let updated_content = json!({"name": [{"family": "Updated"}]});
        let updated = backend
            .update(&tenant, &created, updated_content)
            .await
            .unwrap();
        assert_eq!(updated.version_id(), "2");

        // Verify
        let read = backend
            .read(&tenant, "Patient", created.id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.content()["name"][0]["family"], "Updated");
    }

    #[tokio::test]
    async fn test_update_version_conflict() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create
        let resource = json!({});
        let created = backend
            .create(&tenant, "Patient", resource, FhirVersion::default())
            .await
            .unwrap();

        // Update once
        let _ = backend.update(&tenant, &created, json!({})).await.unwrap();

        // Try to update with stale version
        let result = backend.update(&tenant, &created, json!({})).await;
        assert!(matches!(
            result,
            Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict { .. }
            ))
        ));
    }

    #[tokio::test]
    async fn test_delete() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create
        let resource = json!({});
        let created = backend
            .create(&tenant, "Patient", resource, FhirVersion::default())
            .await
            .unwrap();

        // Delete
        backend
            .delete(&tenant, "Patient", created.id())
            .await
            .unwrap();

        // Read should return Gone
        let result = backend.read(&tenant, "Patient", created.id()).await;
        assert!(matches!(
            result,
            Err(StorageError::Resource(ResourceError::Gone { .. }))
        ));
    }

    #[tokio::test]
    async fn test_create_or_update_new() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let (resource, created) = backend
            .create_or_update(
                &tenant,
                "Patient",
                "new-id",
                json!({}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        assert!(created);
        assert_eq!(resource.id(), "new-id");
        assert_eq!(resource.version_id(), "1");
    }

    #[tokio::test]
    async fn test_create_or_update_existing() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create first
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "existing-id"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Update via create_or_update
        let (resource, created) = backend
            .create_or_update(
                &tenant,
                "Patient",
                "existing-id",
                json!({}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        assert!(!created);
        assert_eq!(resource.version_id(), "2");
    }

    #[tokio::test]
    async fn test_count() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Initially empty
        assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 0);

        // Create some resources
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();

        assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 2);
        assert_eq!(
            backend.count(&tenant, Some("Observation")).await.unwrap(),
            1
        );
        assert_eq!(backend.count(&tenant, None).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn test_tenant_isolation() {
        let backend = create_test_backend();

        let tenant1 =
            TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
        let tenant2 =
            TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

        // Create in tenant 1
        let resource = json!({"id": "patient-1"});
        backend
            .create(&tenant1, "Patient", resource, FhirVersion::default())
            .await
            .unwrap();

        // Tenant 1 can read
        assert!(
            backend
                .read(&tenant1, "Patient", "patient-1")
                .await
                .unwrap()
                .is_some()
        );

        // Tenant 2 cannot read
        assert!(
            backend
                .read(&tenant2, "Patient", "patient-1")
                .await
                .unwrap()
                .is_none()
        );
    }

    // ========================================================================
    // History Tests
    // ========================================================================

    #[tokio::test]
    async fn test_history_instance_basic() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create a resource
        let resource = json!({"name": [{"family": "Smith"}]});
        let created = backend
            .create(&tenant, "Patient", resource, FhirVersion::default())
            .await
            .unwrap();

        // Update it twice
        let v2 = backend
            .update(&tenant, &created, json!({"name": [{"family": "Jones"}]}))
            .await
            .unwrap();
        let _v3 = backend
            .update(&tenant, &v2, json!({"name": [{"family": "Brown"}]}))
            .await
            .unwrap();

        // Get history
        let params = HistoryParams::new();
        let history = backend
            .history_instance(&tenant, "Patient", created.id(), &params)
            .await
            .unwrap();

        // Should have 3 versions, newest first
        assert_eq!(history.items.len(), 3);
        assert_eq!(history.items[0].resource.version_id(), "3");
        assert_eq!(history.items[1].resource.version_id(), "2");
        assert_eq!(history.items[2].resource.version_id(), "1");

        // Check methods
        assert_eq!(history.items[0].method, HistoryMethod::Put);
        assert_eq!(history.items[1].method, HistoryMethod::Put);
        assert_eq!(history.items[2].method, HistoryMethod::Post);
    }

    #[tokio::test]
    async fn test_history_instance_count() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create and update
        let resource = json!({});
        let created = backend
            .create(&tenant, "Patient", resource, FhirVersion::default())
            .await
            .unwrap();
        let v2 = backend.update(&tenant, &created, json!({})).await.unwrap();
        let _v3 = backend.update(&tenant, &v2, json!({})).await.unwrap();

        let count = backend
            .history_instance_count(&tenant, "Patient", created.id())
            .await
            .unwrap();
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn test_history_instance_with_delete() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create, update, then delete
        let resource = json!({"id": "p1"});
        let created = backend
            .create(&tenant, "Patient", resource, FhirVersion::default())
            .await
            .unwrap();
        let _v2 = backend
            .update(&tenant, &created, json!({"id": "p1"}))
            .await
            .unwrap();
        backend.delete(&tenant, "Patient", "p1").await.unwrap();

        // Get history including deleted
        let params = HistoryParams::new().include_deleted(true);
        let history = backend
            .history_instance(&tenant, "Patient", "p1", &params)
            .await
            .unwrap();

        assert_eq!(history.items.len(), 3);
        assert_eq!(history.items[0].method, HistoryMethod::Delete);
        assert_eq!(history.items[0].resource.version_id(), "3");
    }

    #[tokio::test]
    async fn test_history_instance_exclude_deleted() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create, update, then delete
        let resource = json!({"id": "p2"});
        let created = backend
            .create(&tenant, "Patient", resource, FhirVersion::default())
            .await
            .unwrap();
        let _v2 = backend
            .update(&tenant, &created, json!({"id": "p2"}))
            .await
            .unwrap();
        backend.delete(&tenant, "Patient", "p2").await.unwrap();

        // Get history excluding deleted
        let params = HistoryParams::new().include_deleted(false);
        let history = backend
            .history_instance(&tenant, "Patient", "p2", &params)
            .await
            .unwrap();

        // Should not include the delete version
        assert_eq!(history.items.len(), 2);
        assert_eq!(history.items[0].resource.version_id(), "2");
        assert_eq!(history.items[1].resource.version_id(), "1");
    }

    #[tokio::test]
    async fn test_history_instance_pagination() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create with multiple versions
        let resource = json!({});
        let mut current = backend
            .create(&tenant, "Patient", resource, FhirVersion::default())
            .await
            .unwrap();
        for _ in 0..4 {
            current = backend.update(&tenant, &current, json!({})).await.unwrap();
        }
        // Now have 5 versions

        // Get first page (2 items)
        let params = HistoryParams::new().count(2);
        let page1 = backend
            .history_instance(&tenant, "Patient", current.id(), &params)
            .await
            .unwrap();

        assert_eq!(page1.items.len(), 2);
        assert_eq!(page1.items[0].resource.version_id(), "5");
        assert_eq!(page1.items[1].resource.version_id(), "4");
        assert!(page1.page_info.has_next);
    }

    #[tokio::test]
    async fn test_history_instance_nonexistent() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let params = HistoryParams::new();
        let history = backend
            .history_instance(&tenant, "Patient", "nonexistent", &params)
            .await
            .unwrap();

        assert!(history.items.is_empty());
    }

    #[tokio::test]
    async fn test_history_instance_tenant_isolation() {
        let backend = create_test_backend();
        let tenant1 =
            TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
        let tenant2 =
            TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

        // Create in tenant 1
        let resource = json!({"id": "shared-id"});
        let created = backend
            .create(&tenant1, "Patient", resource, FhirVersion::default())
            .await
            .unwrap();
        let _v2 = backend
            .update(&tenant1, &created, json!({"id": "shared-id"}))
            .await
            .unwrap();

        // Tenant 1 sees history
        let history1 = backend
            .history_instance(&tenant1, "Patient", "shared-id", &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history1.items.len(), 2);

        // Tenant 2 sees nothing
        let history2 = backend
            .history_instance(&tenant2, "Patient", "shared-id", &HistoryParams::new())
            .await
            .unwrap();
        assert!(history2.items.is_empty());
    }

    // ========================================================================
    // Type History Tests
    // ========================================================================

    #[tokio::test]
    async fn test_history_type_basic() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create multiple patients
        let p1 = backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let _p2 = backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p2"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Update p1
        let _p1_v2 = backend
            .update(&tenant, &p1, json!({"id": "p1"}))
            .await
            .unwrap();

        // Get type history
        let params = HistoryParams::new();
        let history = backend
            .history_type(&tenant, "Patient", &params)
            .await
            .unwrap();

        // Should have 3 entries total (p1 v1, p1 v2, p2 v1)
        assert_eq!(history.items.len(), 3);

        // All should be Patient type
        for entry in &history.items {
            assert_eq!(entry.resource.resource_type(), "Patient");
        }
    }

    #[tokio::test]
    async fn test_history_type_count() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create multiple patients with updates
        let p1 = backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        let _p1_v2 = backend.update(&tenant, &p1, json!({})).await.unwrap();
        let _p2 = backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // Create an observation (different type)
        backend
            .create(&tenant, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // Count patient history
        let count = backend
            .history_type_count(&tenant, "Patient")
            .await
            .unwrap();
        assert_eq!(count, 3); // p1 v1, p1 v2, p2 v1

        // Count observation history
        let obs_count = backend
            .history_type_count(&tenant, "Observation")
            .await
            .unwrap();
        assert_eq!(obs_count, 1);
    }

    #[tokio::test]
    async fn test_history_type_filters_by_type() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create different resource types
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Encounter", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // Get only Patient history
        let history = backend
            .history_type(&tenant, "Patient", &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history.items.len(), 1);
        assert_eq!(history.items[0].resource.resource_type(), "Patient");

        // Get only Observation history
        let obs_history = backend
            .history_type(&tenant, "Observation", &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(obs_history.items.len(), 1);
        assert_eq!(obs_history.items[0].resource.resource_type(), "Observation");
    }

    #[tokio::test]
    async fn test_history_type_includes_deleted() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create and delete a patient
        let _p1 = backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "del-p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend.delete(&tenant, "Patient", "del-p1").await.unwrap();

        // Create another patient
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p2"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Without including deleted
        let history = backend
            .history_type(&tenant, "Patient", &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history.items.len(), 2); // p1 v1, p2 v1 (excludes delete)

        // Including deleted
        let history_with_deleted = backend
            .history_type(
                &tenant,
                "Patient",
                &HistoryParams::new().include_deleted(true),
            )
            .await
            .unwrap();
        assert_eq!(history_with_deleted.items.len(), 3); // p1 v1, p1 delete, p2 v1
    }

    #[tokio::test]
    async fn test_history_type_tenant_isolation() {
        let backend = create_test_backend();
        let tenant1 =
            TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
        let tenant2 =
            TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

        // Create patients in tenant 1
        backend
            .create(&tenant1, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant1, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // Create patient in tenant 2
        backend
            .create(&tenant2, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // Tenant 1 sees only its history
        let history1 = backend
            .history_type(&tenant1, "Patient", &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history1.items.len(), 2);

        // Tenant 2 sees only its history
        let history2 = backend
            .history_type(&tenant2, "Patient", &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history2.items.len(), 1);
    }

    #[tokio::test]
    async fn test_history_type_pagination() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create several patients
        for i in 0..5 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({"id": format!("p{}", i)}),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        // Get first page (2 items)
        let params = HistoryParams::new().count(2);
        let page1 = backend
            .history_type(&tenant, "Patient", &params)
            .await
            .unwrap();

        assert_eq!(page1.items.len(), 2);
        assert!(page1.page_info.has_next);
    }

    #[tokio::test]
    async fn test_history_type_empty() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // No resources created
        let history = backend
            .history_type(&tenant, "Patient", &HistoryParams::new())
            .await
            .unwrap();
        assert!(history.items.is_empty());
        assert!(!history.page_info.has_next);
    }

    // ========================================================================
    // System History Tests
    // ========================================================================

    #[tokio::test]
    async fn test_history_system_basic() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create different resource types
        let p1 = backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Observation",
                json!({"id": "o1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Encounter",
                json!({"id": "e1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Update patient
        let _p1_v2 = backend
            .update(&tenant, &p1, json!({"id": "p1"}))
            .await
            .unwrap();

        // Get system history
        let history = backend
            .history_system(&tenant, &HistoryParams::new())
            .await
            .unwrap();

        // Should have 4 entries total
        assert_eq!(history.items.len(), 4);

        // Should include all resource types
        let types: std::collections::HashSet<_> = history
            .items
            .iter()
            .map(|e| e.resource.resource_type())
            .collect();
        assert!(types.contains("Patient"));
        assert!(types.contains("Observation"));
        assert!(types.contains("Encounter"));
    }

    #[tokio::test]
    async fn test_history_system_count() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create different resource types
        let p1 = backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        let _p1_v2 = backend.update(&tenant, &p1, json!({})).await.unwrap();
        backend
            .create(&tenant, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Encounter", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // Count all history
        let count = backend.history_system_count(&tenant).await.unwrap();
        assert_eq!(count, 4); // p1 v1, p1 v2, o1, e1
    }

    #[tokio::test]
    async fn test_history_system_includes_deleted() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create and delete a patient
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "del-p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend.delete(&tenant, "Patient", "del-p1").await.unwrap();

        // Create another resource
        backend
            .create(&tenant, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // Without including deleted
        let history = backend
            .history_system(&tenant, &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history.items.len(), 2); // p1 v1, obs (excludes delete)

        // Including deleted
        let history_with_deleted = backend
            .history_system(&tenant, &HistoryParams::new().include_deleted(true))
            .await
            .unwrap();
        assert_eq!(history_with_deleted.items.len(), 3); // p1 v1, p1 delete, obs
    }

    #[tokio::test]
    async fn test_history_system_tenant_isolation() {
        let backend = create_test_backend();
        let tenant1 =
            TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
        let tenant2 =
            TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

        // Create resources in tenant 1
        backend
            .create(&tenant1, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant1, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // Create resource in tenant 2
        backend
            .create(&tenant2, "Encounter", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // Tenant 1 sees only its history
        let history1 = backend
            .history_system(&tenant1, &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history1.items.len(), 2);

        // Tenant 2 sees only its history
        let history2 = backend
            .history_system(&tenant2, &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history2.items.len(), 1);

        // Counts should also be isolated
        assert_eq!(backend.history_system_count(&tenant1).await.unwrap(), 2);
        assert_eq!(backend.history_system_count(&tenant2).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_history_system_pagination() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create several resources of different types
        for i in 0..3 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({"id": format!("p{}", i)}),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        for i in 0..2 {
            backend
                .create(
                    &tenant,
                    "Observation",
                    json!({"id": format!("o{}", i)}),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        // Total: 5 entries

        // Get first page (2 items)
        let params = HistoryParams::new().count(2);
        let page1 = backend.history_system(&tenant, &params).await.unwrap();

        assert_eq!(page1.items.len(), 2);
        assert!(page1.page_info.has_next);
    }

    #[tokio::test]
    async fn test_history_system_empty() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // No resources created
        let history = backend
            .history_system(&tenant, &HistoryParams::new())
            .await
            .unwrap();
        assert!(history.items.is_empty());
        assert!(!history.page_info.has_next);

        assert_eq!(backend.history_system_count(&tenant).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_history_system_ordered_by_time() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create resources - they should be ordered by last_updated DESC
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "first"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Observation",
                json!({"id": "second"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Encounter",
                json!({"id": "third"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let history = backend
            .history_system(&tenant, &HistoryParams::new())
            .await
            .unwrap();

        // Should be in reverse chronological order (newest first)
        assert_eq!(history.items.len(), 3);
        // The last created should be first in the list
        assert_eq!(history.items[0].resource.id(), "third");
        assert_eq!(history.items[1].resource.id(), "second");
        assert_eq!(history.items[2].resource.id(), "first");
    }

    // ========================================================================
    // Delete History Tests (FHIR v6.0.0)
    // ========================================================================

    #[tokio::test]
    async fn test_delete_instance_history() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create a resource and update it twice
        let p1 = backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1", "name": [{"family": "Smith"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let p1_v2 = backend
            .update(
                &tenant,
                &p1,
                json!({"id": "p1", "name": [{"family": "Jones"}]}),
            )
            .await
            .unwrap();
        let _p1_v3 = backend
            .update(
                &tenant,
                &p1_v2,
                json!({"id": "p1", "name": [{"family": "Brown"}]}),
            )
            .await
            .unwrap();

        // Verify we have 3 versions in history
        let history = backend
            .history_instance(&tenant, "Patient", "p1", &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history.items.len(), 3);

        // Delete the instance history (preserves current version)
        let deleted_count = backend
            .delete_instance_history(&tenant, "Patient", "p1")
            .await
            .unwrap();
        assert_eq!(deleted_count, 2); // Only v1 and v2 deleted, v3 preserved

        // History should now only contain the current version
        let history = backend
            .history_instance(&tenant, "Patient", "p1", &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history.items.len(), 1);
        assert_eq!(history.items[0].resource.version_id(), "3");

        // Resource should still be readable
        let resource = backend.read(&tenant, "Patient", "p1").await.unwrap();
        assert!(resource.is_some());
        assert_eq!(resource.unwrap().version_id(), "3");
    }

    #[tokio::test]
    async fn test_delete_instance_history_nonexistent() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Try to delete history for a resource that doesn't exist
        let result = backend
            .delete_instance_history(&tenant, "Patient", "nonexistent")
            .await;

        assert!(matches!(
            result,
            Err(StorageError::Resource(ResourceError::NotFound { .. }))
        ));
    }

    #[tokio::test]
    async fn test_delete_version() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create a resource and update it twice
        let p1 = backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1", "name": [{"family": "Smith"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let p1_v2 = backend
            .update(
                &tenant,
                &p1,
                json!({"id": "p1", "name": [{"family": "Jones"}]}),
            )
            .await
            .unwrap();
        let _p1_v3 = backend
            .update(
                &tenant,
                &p1_v2,
                json!({"id": "p1", "name": [{"family": "Brown"}]}),
            )
            .await
            .unwrap();

        // Delete version 2
        backend
            .delete_version(&tenant, "Patient", "p1", "2")
            .await
            .unwrap();

        // History should now only have versions 1 and 3
        let history = backend
            .history_instance(&tenant, "Patient", "p1", &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history.items.len(), 2);
        let versions: Vec<&str> = history
            .items
            .iter()
            .map(|e| e.resource.version_id())
            .collect();
        assert!(versions.contains(&"1"));
        assert!(versions.contains(&"3"));
        assert!(!versions.contains(&"2"));
    }

    #[tokio::test]
    async fn test_delete_version_current_fails() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create a resource
        let p1 = backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let _p1_v2 = backend
            .update(&tenant, &p1, json!({"id": "p1"}))
            .await
            .unwrap();

        // Try to delete the current version (2)
        let result = backend.delete_version(&tenant, "Patient", "p1", "2").await;

        // Should fail with validation error
        assert!(matches!(result, Err(StorageError::Validation(_))));
    }

    #[tokio::test]
    async fn test_delete_version_nonexistent() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create a resource
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Try to delete a version that doesn't exist
        let result = backend
            .delete_version(&tenant, "Patient", "p1", "999")
            .await;

        assert!(matches!(
            result,
            Err(StorageError::Resource(
                ResourceError::VersionNotFound { .. }
            ))
        ));
    }

    #[tokio::test]
    async fn test_delete_version_resource_not_found() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Try to delete a version for a resource that doesn't exist
        let result = backend
            .delete_version(&tenant, "Patient", "nonexistent", "1")
            .await;

        assert!(matches!(
            result,
            Err(StorageError::Resource(ResourceError::NotFound { .. }))
        ));
    }

    // ========================================================================
    // PurgableStorage Tests
    // ========================================================================

    #[tokio::test]
    async fn test_purge_single_resource() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create a resource with multiple versions
        let p1 = backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let _p1_v2 = backend
            .update(&tenant, &p1, json!({"id": "p1"}))
            .await
            .unwrap();

        // Purge the resource
        backend.purge(&tenant, "Patient", "p1").await.unwrap();

        // Resource should be gone
        let read_result = backend.read(&tenant, "Patient", "p1").await.unwrap();
        assert!(read_result.is_none());

        // History should also be gone
        let history = backend
            .history_instance(&tenant, "Patient", "p1", &HistoryParams::new())
            .await
            .unwrap();
        assert!(history.items.is_empty());
    }

    #[tokio::test]
    async fn test_purge_deleted_resource() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create and delete a resource
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "del-p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend.delete(&tenant, "Patient", "del-p1").await.unwrap();

        // Purge the deleted resource
        backend.purge(&tenant, "Patient", "del-p1").await.unwrap();

        // History should be completely gone
        let history = backend
            .history_instance(
                &tenant,
                "Patient",
                "del-p1",
                &HistoryParams::new().include_deleted(true),
            )
            .await
            .unwrap();
        assert!(history.items.is_empty());
    }

    #[tokio::test]
    async fn test_purge_nonexistent_resource() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Purging a nonexistent resource should fail
        let result = backend.purge(&tenant, "Patient", "nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_purge_tenant_isolation() {
        let backend = create_test_backend();
        let tenant1 =
            TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
        let tenant2 =
            TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

        // Create resource in tenant 1
        backend
            .create(
                &tenant1,
                "Patient",
                json!({"id": "shared-id"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Create resource with same ID in tenant 2
        backend
            .create(
                &tenant2,
                "Patient",
                json!({"id": "shared-id"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Purge from tenant 1
        backend
            .purge(&tenant1, "Patient", "shared-id")
            .await
            .unwrap();

        // Tenant 2's resource should still exist
        let t2_read = backend
            .read(&tenant2, "Patient", "shared-id")
            .await
            .unwrap();
        assert!(t2_read.is_some());

        // Tenant 1's resource should be gone
        let t1_read = backend
            .read(&tenant1, "Patient", "shared-id")
            .await
            .unwrap();
        assert!(t1_read.is_none());
    }

    #[tokio::test]
    async fn test_purge_all_single_type() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create multiple patients
        for i in 0..5 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({"id": format!("p{}", i)}),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        // Create some observations too
        backend
            .create(&tenant, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // Purge all patients
        let count = backend.purge_all(&tenant, "Patient").await.unwrap();
        assert_eq!(count, 5);

        // Patients should be gone
        let patient_history = backend
            .history_type(&tenant, "Patient", &HistoryParams::new())
            .await
            .unwrap();
        assert!(patient_history.items.is_empty());

        // Observations should still exist
        let obs_history = backend
            .history_type(&tenant, "Observation", &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(obs_history.items.len(), 1);
    }

    #[tokio::test]
    async fn test_purge_all_empty_type() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Purging empty type should return 0
        let count = backend.purge_all(&tenant, "Patient").await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_purge_all_tenant_isolation() {
        let backend = create_test_backend();
        let tenant1 =
            TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
        let tenant2 =
            TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

        // Create patients in both tenants
        for i in 0..3 {
            backend
                .create(
                    &tenant1,
                    "Patient",
                    json!({"id": format!("t1-p{}", i)}),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        for i in 0..2 {
            backend
                .create(
                    &tenant2,
                    "Patient",
                    json!({"id": format!("t2-p{}", i)}),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        // Purge all patients from tenant 1
        let count = backend.purge_all(&tenant1, "Patient").await.unwrap();
        assert_eq!(count, 3);

        // Tenant 2's patients should still exist
        let t2_history = backend
            .history_type(&tenant2, "Patient", &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(t2_history.items.len(), 2);
    }

    // ========================================================================
    // DifferentialHistoryProvider Tests
    // ========================================================================

    #[tokio::test]
    async fn test_modified_since_basic() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Capture time before creating resources
        let before_create = Utc::now();

        // Create some resources
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p2"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Observation",
                json!({"id": "o1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Query for all resources modified since before_create
        let pagination = Pagination::default();
        let result = backend
            .modified_since(&tenant, None, before_create, &pagination)
            .await
            .unwrap();

        // Should find all 3 resources
        assert_eq!(result.items.len(), 3);
    }

    #[tokio::test]
    async fn test_modified_since_with_type_filter() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let before_create = Utc::now();

        // Create different resource types
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p2"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Observation",
                json!({"id": "o1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Query for only Patient resources
        let pagination = Pagination::default();
        let result = backend
            .modified_since(&tenant, Some("Patient"), before_create, &pagination)
            .await
            .unwrap();

        // Should find only 2 patients
        assert_eq!(result.items.len(), 2);
        for resource in &result.items {
            assert_eq!(resource.resource_type(), "Patient");
        }
    }

    #[tokio::test]
    async fn test_modified_since_excludes_older() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create a resource
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "old"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Wait a tiny bit and capture time
        let after_first = Utc::now();

        // Create another resource
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "new"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Query for resources modified after the first creation
        let pagination = Pagination::default();
        let result = backend
            .modified_since(&tenant, None, after_first, &pagination)
            .await
            .unwrap();

        // Should find only the newer resource
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].id(), "new");
    }

    #[tokio::test]
    async fn test_modified_since_tenant_isolation() {
        let backend = create_test_backend();
        let tenant1 =
            TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
        let tenant2 =
            TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

        let before_create = Utc::now();

        // Create resources in both tenants
        backend
            .create(
                &tenant1,
                "Patient",
                json!({"id": "t1-p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant2,
                "Patient",
                json!({"id": "t2-p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Query tenant 1
        let pagination = Pagination::default();
        let result1 = backend
            .modified_since(&tenant1, None, before_create, &pagination)
            .await
            .unwrap();
        assert_eq!(result1.items.len(), 1);
        assert_eq!(result1.items[0].id(), "t1-p1");

        // Query tenant 2
        let result2 = backend
            .modified_since(&tenant2, None, before_create, &pagination)
            .await
            .unwrap();
        assert_eq!(result2.items.len(), 1);
        assert_eq!(result2.items[0].id(), "t2-p1");
    }

    #[tokio::test]
    async fn test_modified_since_excludes_deleted() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let before_create = Utc::now();

        // Create and then delete a resource
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "del-p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend.delete(&tenant, "Patient", "del-p1").await.unwrap();

        // Create another resource
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "live-p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Query - deleted resources should be excluded
        let pagination = Pagination::default();
        let result = backend
            .modified_since(&tenant, None, before_create, &pagination)
            .await
            .unwrap();

        // Should only find the live resource
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].id(), "live-p1");
    }

    #[tokio::test]
    async fn test_modified_since_pagination() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let before_create = Utc::now();

        // Create multiple resources
        for i in 0..5 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({"id": format!("p{}", i)}),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        // Get first page (2 items)
        let pagination = Pagination::cursor().with_count(2);
        let page1 = backend
            .modified_since(&tenant, None, before_create, &pagination)
            .await
            .unwrap();

        assert_eq!(page1.items.len(), 2);
        assert!(page1.page_info.has_next);
    }

    #[tokio::test]
    async fn test_modified_since_empty() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Query with no resources
        let pagination = Pagination::default();
        let result = backend
            .modified_since(&tenant, None, Utc::now(), &pagination)
            .await
            .unwrap();

        assert!(result.items.is_empty());
        assert!(!result.page_info.has_next);
    }

    #[tokio::test]
    async fn test_modified_since_returns_current_version() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let before_create = Utc::now();

        // Create a resource and update it multiple times
        let p1 = backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1", "name": "v1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let p1_v2 = backend
            .update(&tenant, &p1, json!({"id": "p1", "name": "v2"}))
            .await
            .unwrap();
        let _p1_v3 = backend
            .update(&tenant, &p1_v2, json!({"id": "p1", "name": "v3"}))
            .await
            .unwrap();

        // Query - should return only the current (latest) version
        let pagination = Pagination::default();
        let result = backend
            .modified_since(&tenant, None, before_create, &pagination)
            .await
            .unwrap();

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].version_id(), "3");
    }

    // ========================================================================
    // ConditionalStorage Tests
    // ========================================================================

    #[tokio::test]
    async fn test_conditional_create_no_match() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create with no matching resources
        let result = backend
            .conditional_create(
                &tenant,
                "Patient",
                json!({"identifier": [{"value": "12345"}]}),
                "identifier=99999", // No match
                FhirVersion::default(),
            )
            .await
            .unwrap();

        match result {
            ConditionalCreateResult::Created(resource) => {
                assert_eq!(resource.resource_type(), "Patient");
            }
            _ => panic!("Expected Created result"),
        }
    }

    #[tokio::test]
    async fn test_conditional_create_single_match() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create an existing resource
        let existing = backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1", "identifier": [{"value": "12345"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Conditional create with matching identifier
        let result = backend
            .conditional_create(
                &tenant,
                "Patient",
                json!({"identifier": [{"value": "12345"}]}),
                "identifier=12345",
                FhirVersion::default(),
            )
            .await
            .unwrap();

        match result {
            ConditionalCreateResult::Exists(resource) => {
                assert_eq!(resource.id(), existing.id());
            }
            _ => panic!("Expected Exists result"),
        }
    }

    #[tokio::test]
    async fn test_conditional_create_by_id() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create an existing resource
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Conditional create with _id parameter
        let result = backend
            .conditional_create(
                &tenant,
                "Patient",
                json!({}),
                "_id=p1",
                FhirVersion::default(),
            )
            .await
            .unwrap();

        match result {
            ConditionalCreateResult::Exists(resource) => {
                assert_eq!(resource.id(), "p1");
            }
            _ => panic!("Expected Exists result"),
        }
    }

    #[tokio::test]
    async fn test_conditional_update_single_match() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create an existing resource
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1", "identifier": [{"value": "12345"}], "active": false}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Conditional update
        let result = backend
            .conditional_update(
                &tenant,
                "Patient",
                json!({"id": "p1", "identifier": [{"value": "12345"}], "active": true}),
                "identifier=12345",
                false,
                FhirVersion::default(),
            )
            .await
            .unwrap();

        match result {
            ConditionalUpdateResult::Updated(resource) => {
                assert_eq!(resource.version_id(), "2");
            }
            _ => panic!("Expected Updated result"),
        }
    }

    #[tokio::test]
    async fn test_conditional_update_no_match_no_upsert() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Conditional update with no match and upsert=false
        let result = backend
            .conditional_update(
                &tenant,
                "Patient",
                json!({"identifier": [{"value": "99999"}]}),
                "identifier=99999",
                false,
                FhirVersion::default(),
            )
            .await
            .unwrap();

        match result {
            ConditionalUpdateResult::NoMatch => {}
            _ => panic!("Expected NoMatch result"),
        }
    }

    #[tokio::test]
    async fn test_conditional_update_no_match_with_upsert() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Conditional update with no match and upsert=true
        let result = backend
            .conditional_update(
                &tenant,
                "Patient",
                json!({"identifier": [{"value": "new-id"}]}),
                "identifier=new-id",
                true,
                FhirVersion::default(),
            )
            .await
            .unwrap();

        match result {
            ConditionalUpdateResult::Created(resource) => {
                assert_eq!(resource.resource_type(), "Patient");
            }
            _ => panic!("Expected Created result"),
        }
    }

    #[tokio::test]
    async fn test_conditional_delete_single_match() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create a resource
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Conditional delete
        let result = backend
            .conditional_delete(&tenant, "Patient", "_id=p1")
            .await
            .unwrap();

        match result {
            ConditionalDeleteResult::Deleted => {
                // Verify resource is deleted (read returns Gone error or None)
                let read_result = backend.read(&tenant, "Patient", "p1").await;
                match read_result {
                    Ok(None) => {}                                                // Resource not found
                    Err(StorageError::Resource(ResourceError::Gone { .. })) => {} // Soft deleted
                    other => panic!("Expected None or Gone, got {:?}", other),
                }
            }
            _ => panic!("Expected Deleted result"),
        }
    }

    #[tokio::test]
    async fn test_conditional_delete_no_match() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Conditional delete with no match
        let result = backend
            .conditional_delete(&tenant, "Patient", "_id=nonexistent")
            .await
            .unwrap();

        match result {
            ConditionalDeleteResult::NoMatch => {}
            _ => panic!("Expected NoMatch result"),
        }
    }

    #[tokio::test]
    async fn test_conditional_operations_tenant_isolation() {
        let backend = create_test_backend();
        let tenant1 =
            TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
        let tenant2 =
            TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

        // Create resource in tenant 1
        backend
            .create(
                &tenant1,
                "Patient",
                json!({"id": "shared-id"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Conditional create in tenant 2 should not find tenant 1's resource
        let result = backend
            .conditional_create(
                &tenant2,
                "Patient",
                json!({}),
                "_id=shared-id",
                FhirVersion::default(),
            )
            .await
            .unwrap();

        match result {
            ConditionalCreateResult::Created(_) => {}
            _ => panic!("Expected Created result (tenant isolation)"),
        }
    }

    // ========================================================================
    // Conditional Patch Tests
    // ========================================================================

    #[tokio::test]
    async fn test_conditional_patch_json_patch() {
        use crate::core::PatchFormat;

        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create a resource
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1", "active": false, "name": [{"family": "Smith"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Apply a JSON Patch
        let patch = PatchFormat::JsonPatch(json!([
            {"op": "replace", "path": "/active", "value": true}
        ]));

        let result = backend
            .conditional_patch(&tenant, "Patient", "_id=p1", &patch)
            .await
            .unwrap();

        match result {
            crate::core::ConditionalPatchResult::Patched(resource) => {
                assert_eq!(resource.content()["active"], json!(true));
            }
            _ => panic!("Expected Patched result"),
        }
    }

    #[tokio::test]
    async fn test_conditional_patch_merge_patch() {
        use crate::core::PatchFormat;

        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create a resource
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1", "active": false, "gender": "unknown"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Apply a merge patch
        let patch = PatchFormat::MergePatch(json!({
            "active": true,
            "gender": null  // null removes the field
        }));

        let result = backend
            .conditional_patch(&tenant, "Patient", "_id=p1", &patch)
            .await
            .unwrap();

        match result {
            crate::core::ConditionalPatchResult::Patched(resource) => {
                assert_eq!(resource.content()["active"], json!(true));
                assert!(resource.content().get("gender").is_none());
            }
            _ => panic!("Expected Patched result"),
        }
    }

    #[tokio::test]
    async fn test_conditional_patch_no_match() {
        use crate::core::PatchFormat;

        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let patch = PatchFormat::JsonPatch(json!([
            {"op": "replace", "path": "/active", "value": true}
        ]));

        let result = backend
            .conditional_patch(&tenant, "Patient", "_id=nonexistent", &patch)
            .await
            .unwrap();

        match result {
            crate::core::ConditionalPatchResult::NoMatch => {}
            _ => panic!("Expected NoMatch result"),
        }
    }

    // ========================================================================
    // BundleProvider Tests
    // ========================================================================

    #[tokio::test]
    async fn test_batch_create_multiple() {
        use crate::core::transaction::BundleProvider;

        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let entries = vec![
            BundleEntry {
                method: BundleMethod::Post,
                url: "Patient".to_string(),
                resource: Some(json!({"resourceType": "Patient", "id": "batch-p1"})),
                if_match: None,
                if_none_match: None,
                if_none_exist: None,
                full_url: None,
            },
            BundleEntry {
                method: BundleMethod::Post,
                url: "Patient".to_string(),
                resource: Some(json!({"resourceType": "Patient", "id": "batch-p2"})),
                if_match: None,
                if_none_match: None,
                if_none_exist: None,
                full_url: None,
            },
        ];

        let result = backend.process_batch(&tenant, entries).await.unwrap();

        assert_eq!(result.entries.len(), 2);
        assert_eq!(result.entries[0].status, 201);
        assert_eq!(result.entries[1].status, 201);
    }

    #[tokio::test]
    async fn test_batch_mixed_operations() {
        use crate::core::transaction::BundleProvider;

        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create a resource first
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "existing"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let entries = vec![
            // Read existing
            BundleEntry {
                method: BundleMethod::Get,
                url: "Patient/existing".to_string(),
                resource: None,
                if_match: None,
                if_none_match: None,
                if_none_exist: None,
                full_url: None,
            },
            // Create new
            BundleEntry {
                method: BundleMethod::Post,
                url: "Patient".to_string(),
                resource: Some(json!({"resourceType": "Patient", "id": "new"})),
                if_match: None,
                if_none_match: None,
                if_none_exist: None,
                full_url: None,
            },
            // Read nonexistent
            BundleEntry {
                method: BundleMethod::Get,
                url: "Patient/nonexistent".to_string(),
                resource: None,
                if_match: None,
                if_none_match: None,
                if_none_exist: None,
                full_url: None,
            },
        ];

        let result = backend.process_batch(&tenant, entries).await.unwrap();

        assert_eq!(result.entries.len(), 3);
        assert_eq!(result.entries[0].status, 200); // Read existing
        assert_eq!(result.entries[1].status, 201); // Create new
        assert_eq!(result.entries[2].status, 404); // Read nonexistent
    }

    #[tokio::test]
    async fn test_batch_delete() {
        use crate::core::transaction::BundleProvider;

        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create a resource
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "to-delete"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let entries = vec![BundleEntry {
            method: BundleMethod::Delete,
            url: "Patient/to-delete".to_string(),
            resource: None,
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: None,
        }];

        let result = backend.process_batch(&tenant, entries).await.unwrap();

        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].status, 204);

        // Verify deletion (read returns Gone error or None)
        let read_result = backend.read(&tenant, "Patient", "to-delete").await;
        match read_result {
            Ok(None) => {}                                                // Resource not found
            Err(StorageError::Resource(ResourceError::Gone { .. })) => {} // Soft deleted
            other => panic!("Expected None or Gone, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_transaction_all_or_nothing() {
        use crate::core::transaction::BundleProvider;

        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create a resource first
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "existing"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let entries = vec![
            // This should succeed
            BundleEntry {
                method: BundleMethod::Post,
                url: "Patient".to_string(),
                resource: Some(json!({"resourceType": "Patient", "id": "tx-p1"})),
                if_match: None,
                if_none_match: None,
                if_none_exist: None,
                full_url: None,
            },
            // This should fail (duplicate ID)
            BundleEntry {
                method: BundleMethod::Post,
                url: "Patient".to_string(),
                resource: Some(json!({"resourceType": "Patient", "id": "existing"})),
                if_match: None,
                if_none_match: None,
                if_none_exist: None,
                full_url: None,
            },
        ];

        let result = backend.process_transaction(&tenant, entries).await;

        // Should fail
        assert!(result.is_err());

        // First resource should NOT have been created (rollback)
        let read = backend.read(&tenant, "Patient", "tx-p1").await.unwrap();
        assert!(read.is_none());
    }

    #[tokio::test]
    async fn test_transaction_success() {
        use crate::core::transaction::BundleProvider;

        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let entries = vec![
            BundleEntry {
                method: BundleMethod::Post,
                url: "Patient".to_string(),
                resource: Some(json!({"resourceType": "Patient", "id": "tx-success-1"})),
                if_match: None,
                if_none_match: None,
                if_none_exist: None,
                full_url: None,
            },
            BundleEntry {
                method: BundleMethod::Post,
                url: "Observation".to_string(),
                resource: Some(json!({"resourceType": "Observation", "id": "tx-success-2"})),
                if_match: None,
                if_none_match: None,
                if_none_exist: None,
                full_url: None,
            },
        ];

        let result = backend.process_transaction(&tenant, entries).await.unwrap();

        assert_eq!(result.entries.len(), 2);
        assert_eq!(result.entries[0].status, 201);
        assert_eq!(result.entries[1].status, 201);

        // Both resources should exist
        assert!(
            backend
                .read(&tenant, "Patient", "tx-success-1")
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            backend
                .read(&tenant, "Observation", "tx-success-2")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn test_parse_url_formats() {
        let backend = create_test_backend();

        // Simple format
        let (rt, id) = backend.parse_url("Patient/123").unwrap();
        assert_eq!(rt, "Patient");
        assert_eq!(id, "123");

        // With leading slash
        let (rt, id) = backend.parse_url("/Patient/456").unwrap();
        assert_eq!(rt, "Patient");
        assert_eq!(id, "456");

        // Full URL
        let (rt, id) = backend
            .parse_url("http://example.com/fhir/Patient/789")
            .unwrap();
        assert_eq!(rt, "Patient");
        assert_eq!(id, "789");

        // HTTPS URL
        let (rt, id) = backend
            .parse_url("https://example.com/fhir/Observation/obs-1")
            .unwrap();
        assert_eq!(rt, "Observation");
        assert_eq!(id, "obs-1");
    }

    // ========================================================================
    // Search Index Display Text Tests
    // ========================================================================

    #[tokio::test]
    async fn test_search_index_display_text_populated() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create an observation with display text
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs-display-test",
                    "code": {
                        "coding": [
                            {
                                "system": "http://loinc.org",
                                "code": "8867-4",
                                "display": "Heart rate"
                            }
                        ]
                    },
                    "status": "final"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Query the search_index directly
        let conn = backend.get_connection().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT param_name, value_token_system, value_token_code, value_token_display
             FROM search_index
             WHERE tenant_id = 'test-tenant'
               AND resource_id = 'obs-display-test'
               AND param_name = 'code'",
            )
            .unwrap();

        #[allow(clippy::type_complexity)]
        let rows: Vec<(String, Option<String>, Option<String>, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        // Should have at least one entry
        assert!(
            !rows.is_empty(),
            "Should have indexed 'code' parameter for Observation"
        );

        // Find the entry with code 8867-4
        let entry = rows
            .iter()
            .find(|(_, _, code, _)| code.as_deref() == Some("8867-4"));
        assert!(entry.is_some(), "Should have entry with code 8867-4");

        // Verify display text is populated
        let (_, _, _, display) = entry.unwrap();
        assert_eq!(
            display.as_deref(),
            Some("Heart rate"),
            "Display text should be 'Heart rate'"
        );
    }

    #[tokio::test]
    async fn test_search_index_identifier_type_populated() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create a patient with typed identifier
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "patient-type-test",
                    "identifier": [
                        {
                            "type": {
                                "coding": [
                                    {
                                        "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
                                        "code": "MR"
                                    }
                                ]
                            },
                            "system": "http://hospital.org/mrn",
                            "value": "MRN12345"
                        }
                    ]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Query the search_index directly
        let conn = backend.get_connection().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT param_name, value_token_code, value_identifier_type_system, value_identifier_type_code
             FROM search_index
             WHERE tenant_id = 'test-tenant'
               AND resource_id = 'patient-type-test'
               AND param_name = 'identifier'",
            )
            .unwrap();

        #[allow(clippy::type_complexity)]
        let rows: Vec<(String, Option<String>, Option<String>, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        // Should have at least one entry
        assert!(
            !rows.is_empty(),
            "Should have indexed 'identifier' parameter for Patient"
        );

        // Find the entry with value MRN12345
        let entry = rows
            .iter()
            .find(|(_, code, _, _)| code.as_deref() == Some("MRN12345"));
        assert!(entry.is_some(), "Should have entry with value MRN12345");

        // Verify identifier type is populated
        let (_, _, type_system, type_code) = entry.unwrap();
        assert_eq!(
            type_system.as_deref(),
            Some("http://terminology.hl7.org/CodeSystem/v2-0203"),
            "Identifier type system should be populated"
        );
        assert_eq!(
            type_code.as_deref(),
            Some("MR"),
            "Identifier type code should be populated"
        );
    }
}
