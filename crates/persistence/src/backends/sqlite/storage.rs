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
    PatchCandidateValidator, patch_update_result, prepare_bundle_patch,
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
use crate::search::reindex::{ReindexSource, ReindexTarget, ResourcePage, SkippedResource};
use crate::tenant::{Operation, TenantContext};
use crate::types::Pagination;
use crate::types::SearchQuery;
use crate::types::{CursorValue, Page, PageCursor, PageInfo, StoredResource};

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

/// The database-free product of indexing one resource: every `search_index`
/// row's bound parameters (or the extraction error that sends the write to
/// the minimal fallback), the contained resources' rows, and the full-text
/// content. Built by [`SqliteBackend::prepare_index`], written by
/// [`SqliteBackend::write_prepared_index`].
pub(crate) struct PreparedIndex {
    rows: Result<Vec<Vec<SqlValue>>, String>,
    contained_rows: Vec<Vec<SqlValue>>,
    fts: Option<super::search::fts::SearchableContent>,
}

/// Batches below this size are prepared on the calling thread.
const PARALLEL_PREPARE_MIN_BATCH: usize = 16;

/// Runs currently inside a bulk index rebuild, process-wide: the indexes are
/// per database, not per run, so the first run in drops them and the last
/// one out rebuilds them. Held across the DROP / CREATE so two runs cannot
/// race each other's transition.
static BULK_INDEX_REBUILDS: parking_lot::Mutex<usize> = parking_lot::Mutex::new(0);

/// The pool [`SqliteBackend::prepare_index_batch`] runs on. Its own pool
/// rather than rayon's global one so its width can be set independently of
/// anything else in the process that uses rayon: `HFS_INDEX_THREADS`,
/// defaulting to the machine's parallelism.
fn index_prepare_pool() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let threads = std::env::var("HFS_INDEX_THREADS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|&n| n > 0)
            .unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4)
            });
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("hfs-index-{i}"))
            .build()
            .expect("build index thread pool")
    })
}

/// Whether the optional `resource_fts` FTS5 virtual table exists on this
/// database.
///
/// FTS5 is an optional SQLite compile-time feature. `create_fts_table`
/// (`schema.rs`) succeeds silently when it is absent, so `resource_fts` may
/// legitimately not exist — and a database created by an FTS5-less build keeps
/// no table even once reopened by an FTS5-capable one, because the `v2 -> v3`
/// migration is already recorded as done.
///
/// Callers therefore probe before touching the table, and treat "table absent"
/// (a determinate fact: there is nothing indexed, so nothing to erase) very
/// differently from "the statement failed" (an unknown, which must never be
/// swallowed on a purge path — see the `DELETE FROM resource_fts` call sites).
///
/// Cheap: `sqlite_master` is answered from the connection's in-memory schema
/// cache and does not touch disk.
pub(crate) fn fts_table_exists(conn: &rusqlite::Connection) -> StorageResult<bool> {
    use rusqlite::OptionalExtension;

    conn.prepare_cached("SELECT 1 FROM sqlite_master WHERE type='table' AND name='resource_fts'")
        .and_then(|mut stmt| stmt.query_row([], |_| Ok(())).optional())
        .map(|found| found.is_some())
        .map_err(|e| internal_error(format!("Failed to probe for resource_fts: {e}")))
}

/// Cap on [`WriteMarker::recent_writes`](crate::core::WriteMarker): the count is
/// a change detector, not a figure, so it stops at this many rows (#1078).
const WRITE_MARKER_RECENT_CAP: i64 = 10_000;

/// The tenant's newest history timestamp: one descending probe of the
/// `(tenant_id, last_updated)` index (`idx_history_updated`), no sort.
const LATEST_WRITE_SQL: &str = "SELECT last_updated FROM resource_history \
     WHERE tenant_id = ?1 ORDER BY last_updated DESC LIMIT 1";

/// History rows at or after `?2`, counted over the same index range and
/// stopped after `?3` rows by the inner `LIMIT`.
const RECENT_WRITES_SQL: &str = "SELECT COUNT(*) FROM (SELECT 1 FROM resource_history \
     WHERE tenant_id = ?1 AND last_updated >= ?2 LIMIT ?3)";

impl SqliteBackend {
    /// [`ResourceStorage::latest_write_marker`] with the `recent_writes` cap as
    /// a parameter, so tests can exercise the cap without 10,000 writes.
    async fn latest_write_marker_capped(
        &self,
        tenant: &TenantContext,
        recent_since: Option<chrono::DateTime<Utc>>,
        cap: i64,
    ) -> StorageResult<Option<crate::core::WriteMarker>> {
        use rusqlite::OptionalExtension;

        // Owned copies: the probes run in a blocking task (#959).
        let tenant_id = tenant.tenant_id().as_str().to_string();
        // Formatted the same RFC3339 way rows are written, so the raw-column
        // `last_updated >= ?2` range compares like-for-like and stays sargable
        // (see `count_deltas_by_bucket`).
        let since_bound = recent_since.map(|since| since.to_rfc3339());

        self.run_blocking(move |conn| {
            let latest: Option<String> = conn
                .prepare_cached(LATEST_WRITE_SQL)
                .and_then(|mut stmt| {
                    stmt.query_row(params![tenant_id], |row| row.get(0))
                        .optional()
                })
                .or_query_error("Failed to query latest write marker")?;
            let latest = latest
                .map(|s| {
                    chrono::DateTime::parse_from_rfc3339(&s)
                        .map(|dt| dt.with_timezone(&Utc))
                        .map_err(|e| internal_error(format!("Failed to parse last_updated: {}", e)))
                })
                .transpose()?;

            let recent_writes = match since_bound {
                Some(bound) => {
                    let n: i64 = conn
                        .prepare_cached(RECENT_WRITES_SQL)
                        .and_then(|mut stmt| {
                            stmt.query_row(params![tenant_id, bound, cap], |row| row.get(0))
                        })
                        .or_query_error("Failed to count recent writes")?;
                    Some(n.max(0) as u64)
                }
                None => None,
            };

            Ok(Some(crate::core::WriteMarker {
                latest,
                recent_writes,
            }))
        })
        .await
    }
}

/// Runs a `DELETE FROM resource_fts …` on a purge path, skipping it when FTS5
/// is unavailable and propagating any other failure.
///
/// The error handling is the point. Elsewhere in this backend an FTS delete is
/// best-effort (`let _ = …`), which is tolerable on an index-maintenance path
/// where the worst case is a stale entry. On a *purge* path it is not: a
/// swallowed failure means the API answers `200` — "the record is gone" — while
/// the resource's full text is still on disk. That is a false erasure
/// attestation, and it is the same class of defect issue #386 reports, just
/// reached by a different route. So: probe, then be strict.
fn purge_fts_rows(
    conn: &rusqlite::Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> StorageResult<()> {
    if !fts_table_exists(conn)? {
        return Ok(());
    }
    conn.execute(sql, params)
        .map_err(|e| internal_error(format!("purge fts delete: {e}")))?;
    Ok(())
}

#[async_trait]
impl ResourceStorage for SqliteBackend {
    fn backend_name(&self) -> &'static str {
        "sqlite"
    }

    async fn readiness_check(&self) -> Result<(), BackendError> {
        <Self as crate::core::Backend>::health_check(self).await
    }

    fn is_cluster_shared(&self) -> bool {
        false
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
        tenant.check_permission(Operation::Create, resource_type)?;

        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Extract or generate ID
        let id = resource
            .get("id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(crate::types::new_resource_id);

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

        // An overlay-affecting SearchParameter write changes this tenant's
        // registry — drop the cached copy so the next access rebuilds from
        // storage. Seeded spec copies never affect the overlay (see
        // `create_affects_overlay`), which keeps bulk seeding from triggering
        // an O(n²) rebuild storm.
        if resource_type == "SearchParameter"
            && self.tenant_registries().create_affects_overlay(&resource)
        {
            self.tenant_registries().invalidate(tenant_id);
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
                let restored = self.restore_deleted(tenant, resource_type, id, resource)?;
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
        let resource_type = current.resource_type();
        tenant.check_permission(Operation::Update, resource_type)?;

        let mut conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();
        let id = current.id();

        // The expected version is `current`'s, and the UPDATE below only matches
        // a row that still carries it — so the new version follows from what the
        // caller already read.
        let expected_version = current.version_id();
        let new_version: u64 = expected_version.parse().unwrap_or(0) + 1;
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

        // Extract the search values before taking the write lock: it is pure
        // CPU, and SQLite has one writer.
        let prepared = (!self.is_search_offloaded())
            .then(|| self.prepare_index(tenant_id, resource_type, id, &resource));

        let now = Utc::now();
        let last_updated = now.to_rfc3339();
        let fhir_version_str = current.fhir_version().as_mime_param();

        // Compare-and-swap, history row and search index in ONE transaction.
        //
        // This used to be a `SELECT version_id`, a comparison in Rust, and then
        // an `UPDATE` with no version in its `WHERE`, each statement
        // auto-committed on a pooled connection. Two writers holding the same
        // version on two connections both passed the comparison; the second
        // `UPDATE` then overwrote the first and committed, and only its history
        // `INSERT` failed — on `PRIMARY KEY (…, version_id)` — so that writer got
        // a 500 while its content was already the current row, under a version
        // whose history entry holds the *winner's* content (#1404).
        //
        // The version now rides in the `UPDATE`'s predicate, so the comparison
        // and the write are one statement; and everything that follows shares
        // its transaction, so a writer that loses leaves nothing behind.
        // IMMEDIATE takes the write lock up front, where the busy handler
        // applies (see `purge_tenant_data`).
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| internal_error(format!("Failed to begin update: {}", e)))?;

        let updated = tx
            .execute(
                "UPDATE resources SET version_id = ?1, data = ?2, last_updated = ?3
                 WHERE tenant_id = ?4 AND resource_type = ?5 AND id = ?6
                   AND version_id = ?7 AND is_deleted = 0",
                params![
                    new_version_str,
                    data,
                    last_updated,
                    tenant_id,
                    resource_type,
                    id,
                    expected_version
                ],
            )
            .map_err(|e| internal_error(format!("Failed to update resource: {}", e)))?;

        if updated == 0 {
            // Matched nothing; which of the two reasons it was costs a query,
            // but only on the path that is already failing.
            let actual: Result<String, _> = tx.query_row(
                "SELECT version_id FROM resources
                 WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3 AND is_deleted = 0",
                params![tenant_id, resource_type, id],
                |row| row.get(0),
            );
            return match actual {
                Ok(actual_version) => Err(StorageError::Concurrency(
                    ConcurrencyError::VersionConflict {
                        resource_type: resource_type.to_string(),
                        id: id.to_string(),
                        expected_version: expected_version.to_string(),
                        actual_version,
                    },
                )),
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    Err(StorageError::Resource(ResourceError::NotFound {
                        resource_type: resource_type.to_string(),
                        id: id.to_string(),
                    }))
                }
                Err(e) => Err(internal_error(format!(
                    "Failed to get current version: {}",
                    e
                ))),
            };
        }

        // Insert into history (preserve the original FHIR version)
        tx.execute(
            "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
            params![tenant_id, resource_type, id, new_version_str, data, last_updated, fhir_version_str],
        )
        .map_err(|e| internal_error(format!("Failed to insert history: {}", e)))?;

        // Re-index the resource (delete old entries, add new)
        if let Some(prepared) = prepared {
            self.delete_search_index(&tx, tenant_id, resource_type, id)?;
            self.write_prepared_index(&tx, tenant_id, resource_type, id, &resource, prepared)?;
        }

        tx.commit()
            .map_err(|e| internal_error(format!("Failed to commit update: {}", e)))?;

        // A SearchParameter write invalidates this tenant's cached registry.
        if resource_type == "SearchParameter" {
            self.tenant_registries().invalidate(tenant_id);
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
        self.soft_delete(tenant, resource_type, id, None)
    }

    async fn delete_versioned(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
    ) -> StorageResult<()> {
        self.soft_delete(tenant, resource_type, id, Some(expected_version))
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        // An unfiltered `COUNT(*)` is a full index scan — seconds on a
        // multi-million-row database — so it runs on a blocking thread rather
        // than on a tokio worker (#959). The closure must be `'static`, hence
        // the owned copies of both borrowed inputs.
        let tenant_id = tenant.tenant_id().as_str().to_string();
        let resource_type = resource_type.map(str::to_string);

        self.run_blocking(move |conn| {
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
            .or_query_error("Failed to count resources")?;

            Ok(count as u64)
        })
        .await
    }

    async fn count_by_day(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        since: chrono::DateTime<chrono::Utc>,
    ) -> StorageResult<Vec<crate::core::DailyResourceCount>> {
        // Owned copies of the borrowed inputs: the aggregate below runs in a
        // blocking task (#959), whose closure must be `'static`.
        let tenant_id = tenant.tenant_id().as_str().to_string();
        let resource_type = resource_type.to_string();
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

        self.run_blocking(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT substr(last_updated, 1, 10) AS day, COUNT(*) AS n \
                     FROM resources \
                     WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0 \
                       AND last_updated >= ?3 \
                     GROUP BY day ORDER BY day",
                )
                .or_query_error("Failed to prepare count_by_day")?;

            let rows = stmt
                .query_map(params![tenant_id, resource_type, since_bound], |row| {
                    let day: String = row.get(0)?;
                    let n: i64 = row.get(1)?;
                    Ok((day, n))
                })
                .or_query_error("Failed to query count_by_day")?;

            let mut out = Vec::new();
            for row in rows {
                let (day_str, n) = row.or_query_error("Failed to read count_by_day row")?;
                if let Ok(day) = chrono::NaiveDate::parse_from_str(&day_str, "%Y-%m-%d") {
                    out.push(crate::core::DailyResourceCount {
                        day,
                        count: n.max(0) as u64,
                    });
                }
            }
            Ok(out)
        })
        .await
    }

    async fn count_deltas_by_bucket(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        since: chrono::DateTime<chrono::Utc>,
        bucket_seconds: i64,
    ) -> StorageResult<Vec<crate::core::ResourceCountDelta>> {
        if bucket_seconds <= 0 {
            return Err(internal_error(
                "count_deltas_by_bucket: bucket_seconds must be positive".to_string(),
            ));
        }
        // Owned copies of the borrowed inputs: the aggregate below runs in a
        // blocking task (#959), whose closure must be `'static`.
        let tenant_id = tenant.tenant_id().as_str().to_string();
        let resource_type = resource_type.to_string();

        // Bound the scan by the raw `last_updated` column so the
        // `(tenant_id, last_updated)` history index prunes the range (wrapping the
        // column in `strftime(...)` would force a full scan). The bound is floored
        // to a bucket boundary, and formatted the same RFC3339 way the rows are
        // written; because it lands exactly on a whole second it carries no
        // fractional part, and any stored value in that same second sorts after it
        // (`.` = 0x2E > `+` = 0x2B), so no row in the first bucket is missed.
        let since_bound = crate::core::bucket_floor(since, bucket_seconds).to_rfc3339();

        // `strftime('%s', ...)` parses the stored RFC3339 UTC string to epoch
        // seconds; integer-dividing by the bucket width and multiplying back floors
        // each version to its epoch-aligned bucket start. The delta rule mirrors the
        // trait doc: creation `+1`, delete `-1`, plain update `0`.
        self.run_blocking(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT (CAST(strftime('%s', last_updated) AS INTEGER) / ?4) * ?4 AS bucket, \
                            SUM(CASE WHEN is_deleted = 1 THEN -1 \
                                     WHEN version_id = '1' THEN 1 \
                                     ELSE 0 END) AS delta \
                     FROM resource_history \
                     WHERE tenant_id = ?1 AND resource_type = ?2 AND last_updated >= ?3 \
                     GROUP BY bucket HAVING delta != 0 ORDER BY bucket",
                )
                .or_query_error("Failed to prepare count_deltas_by_bucket")?;

            let rows = stmt
                .query_map(
                    params![tenant_id, resource_type, since_bound, bucket_seconds],
                    |row| {
                        let bucket: i64 = row.get(0)?;
                        let delta: i64 = row.get(1)?;
                        Ok((bucket, delta))
                    },
                )
                .or_query_error("Failed to query count_deltas_by_bucket")?;

            let mut out = Vec::new();
            for row in rows {
                let (bucket, delta) =
                    row.or_query_error("Failed to read count_deltas_by_bucket row")?;
                if let Some(bucket_start) = chrono::DateTime::from_timestamp(bucket, 0) {
                    out.push(crate::core::ResourceCountDelta {
                        bucket_start,
                        delta,
                    });
                }
            }
            Ok(out)
        })
        .await
    }

    async fn count_deltas_by_type_and_bucket(
        &self,
        tenant: &TenantContext,
        resource_types: &[&str],
        since: chrono::DateTime<chrono::Utc>,
        bucket_seconds: i64,
    ) -> StorageResult<Vec<(String, crate::core::ResourceCountDelta)>> {
        if resource_types.is_empty() {
            return Ok(Vec::new());
        }
        if bucket_seconds <= 0 {
            return Err(internal_error(
                "count_deltas_by_type_and_bucket: bucket_seconds must be positive".to_string(),
            ));
        }
        // Owned copies of the borrowed inputs: the aggregate below runs in a
        // blocking task (#959), whose closure must be `'static`.
        let tenant_id = tenant.tenant_id().as_str().to_string();
        let mut resource_types: Vec<String> =
            resource_types.iter().map(|rt| (*rt).to_string()).collect();
        resource_types.sort();
        resource_types.dedup();

        // One scan for every requested type (#1078), bounded exactly like
        // `count_deltas_by_bucket`: the raw `last_updated` column against a
        // bucket-floored RFC3339 bound, so the `(tenant_id, last_updated)`
        // history index prunes the range, and the same strftime bucketing and
        // delta rule. The type list only filters that range and splits the
        // grouping, so each type's rows equal its per-type call's.
        let since_bound = crate::core::bucket_floor(since, bucket_seconds).to_rfc3339();

        self.run_blocking(move |conn| {
            let placeholders = (0..resource_types.len())
                .map(|i| format!("?{}", i + 4))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT resource_type, \
                        (CAST(strftime('%s', last_updated) AS INTEGER) / ?3) * ?3 AS bucket, \
                        SUM(CASE WHEN is_deleted = 1 THEN -1 \
                                 WHEN version_id = '1' THEN 1 \
                                 ELSE 0 END) AS delta \
                 FROM resource_history \
                 WHERE tenant_id = ?1 AND last_updated >= ?2 \
                   AND resource_type IN ({placeholders}) \
                 GROUP BY resource_type, bucket HAVING delta != 0 \
                 ORDER BY resource_type, bucket"
            );
            let mut stmt = conn
                .prepare(&sql)
                .or_query_error("Failed to prepare count_deltas_by_type_and_bucket")?;

            let mut bound: Vec<&dyn ToSql> = vec![&tenant_id, &since_bound, &bucket_seconds];
            bound.extend(resource_types.iter().map(|rt| rt as &dyn ToSql));
            let rows = stmt
                .query_map(bound.as_slice(), |row| {
                    let resource_type: String = row.get(0)?;
                    let bucket: i64 = row.get(1)?;
                    let delta: i64 = row.get(2)?;
                    Ok((resource_type, bucket, delta))
                })
                .or_query_error("Failed to query count_deltas_by_type_and_bucket")?;

            let mut out = Vec::new();
            for row in rows {
                let (resource_type, bucket, delta) =
                    row.or_query_error("Failed to read count_deltas_by_type_and_bucket row")?;
                if let Some(bucket_start) = chrono::DateTime::from_timestamp(bucket, 0) {
                    out.push((
                        resource_type,
                        crate::core::ResourceCountDelta {
                            bucket_start,
                            delta,
                        },
                    ));
                }
            }
            Ok(out)
        })
        .await
    }

    async fn activity_histogram(
        &self,
        tenant: &TenantContext,
        since: chrono::DateTime<chrono::Utc>,
    ) -> StorageResult<Vec<crate::core::ActivityCell>> {
        // Owned copy of the borrowed tenant id: the aggregate below runs in a
        // blocking task (#959), whose closure must be `'static`.
        let tenant_id = tenant.tenant_id().as_str().to_string();
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
        self.run_blocking(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT CAST(strftime('%w', last_updated) AS INTEGER) AS wd, \
                            CAST(strftime('%H', last_updated) AS INTEGER) AS hr, \
                            COUNT(*) AS n \
                     FROM resource_history \
                     WHERE tenant_id = ?1 AND last_updated >= ?2 \
                     GROUP BY wd, hr",
                )
                .or_query_error("Failed to prepare activity_histogram")?;

            let rows = stmt
                .query_map(params![tenant_id, since_bound], |row| {
                    let wd: i64 = row.get(0)?;
                    let hr: i64 = row.get(1)?;
                    let n: i64 = row.get(2)?;
                    Ok((wd, hr, n))
                })
                .or_query_error("Failed to query activity_histogram")?;

            let mut out = Vec::new();
            for row in rows {
                let (wd, hr, n) = row.or_query_error("Failed to read activity row")?;
                out.push(crate::core::ActivityCell {
                    weekday: wd.clamp(0, 6) as u8,
                    hour: hr.clamp(0, 23) as u8,
                    count: n.max(0) as u64,
                });
            }
            Ok(out)
        })
        .await
    }

    async fn count_all_types(&self, tenant: &TenantContext) -> StorageResult<Vec<(String, u64)>> {
        // This is the dashboard's single most expensive query — a grouped
        // aggregate over every live row of `resources` — so it runs on a
        // blocking thread instead of stalling a tokio worker for its full
        // duration (#959). The owned `tenant_id` is what makes the closure
        // `'static`.
        let tenant_id = tenant.tenant_id().as_str().to_string();
        self.run_blocking(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT resource_type, COUNT(*) FROM resources \
                     WHERE tenant_id = ?1 AND is_deleted = 0 \
                     GROUP BY resource_type",
                )
                .or_query_error("Failed to prepare count_all_types")?;
            let rows = stmt
                .query_map(params![tenant_id], |row| {
                    let rt: String = row.get(0)?;
                    let n: i64 = row.get(1)?;
                    Ok((rt, n.max(0) as u64))
                })
                .or_query_error("Failed to query count_all_types")?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.or_query_error("count_all_types row")?);
            }
            Ok(out)
        })
        .await
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
        // Bind tenant_id as ?1 and each requested type as ?2, ?3, ...; the type
        // names are bound as parameters, never interpolated into the SQL text.
        // Building the statement text needs no connection, so it stays out here
        // and only the query itself moves onto the blocking thread (#959).
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
        // then the requested types. Owned `String`s rather than borrowed `&str`,
        // because the blocking closure has to be `'static` and cannot hold onto
        // `tenant` or the caller's `resource_types` slice.
        let mut binds: Vec<String> = Vec::with_capacity(resource_types.len() + 1);
        binds.push(tenant.tenant_id().as_str().to_string());
        binds.extend(resource_types.iter().map(|rt| rt.to_string()));

        self.run_blocking(move |conn| {
            let mut stmt = conn
                .prepare(&sql)
                .or_query_error("Failed to prepare count_by_types")?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(binds), |row| {
                    let rt: String = row.get(0)?;
                    let n: i64 = row.get(1)?;
                    Ok((rt, n.max(0) as u64))
                })
                .or_query_error("Failed to query count_by_types")?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.or_query_error("count_by_types row")?);
            }
            Ok(out)
        })
        .await
    }

    async fn count_by_tenant(&self) -> StorageResult<Vec<(String, u64)>> {
        // Cross-tenant admin aggregate (see trait docs): no tenant filter.
        let conn = self.get_connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT tenant_id, COUNT(*) FROM resources \
                 WHERE is_deleted = 0 GROUP BY tenant_id",
            )
            .or_query_error("Failed to prepare count_by_tenant")?;
        let rows = stmt
            .query_map([], |row| {
                let tid: String = row.get(0)?;
                let n: i64 = row.get(1)?;
                Ok((tid, n.max(0) as u64))
            })
            .or_query_error("Failed to query count_by_tenant")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.or_query_error("count_by_tenant row")?);
        }
        Ok(out)
    }

    fn supports_type_counts(&self) -> bool {
        true
    }

    async fn latest_write_marker(
        &self,
        tenant: &TenantContext,
        recent_since: Option<chrono::DateTime<Utc>>,
    ) -> StorageResult<Option<crate::core::WriteMarker>> {
        self.latest_write_marker_capped(tenant, recent_since, WRITE_MARKER_RECENT_CAP)
            .await
    }

    fn supports_tenant_registry(&self) -> bool {
        true
    }

    async fn list_tenants(&self) -> StorageResult<Vec<crate::core::TenantRecord>> {
        let conn = self.get_connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, display_name, created_at FROM tenants \
                 ORDER BY created_at ASC, id ASC",
            )
            .map_err(|e| internal_error(format!("prepare list_tenants: {e}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(crate::core::TenantRecord {
                    id: row.get(0)?,
                    display_name: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })
            .map_err(|e| internal_error(format!("query list_tenants: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| internal_error(format!("list_tenants row: {e}")))?);
        }
        Ok(out)
    }

    async fn get_tenant(&self, id: &str) -> StorageResult<Option<crate::core::TenantRecord>> {
        let conn = self.get_connection()?;
        let mut stmt = conn
            .prepare("SELECT id, display_name, created_at FROM tenants WHERE id = ?1")
            .map_err(|e| internal_error(format!("prepare get_tenant: {e}")))?;
        let mut rows = stmt
            .query_map(params![id], |row| {
                Ok(crate::core::TenantRecord {
                    id: row.get(0)?,
                    display_name: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })
            .map_err(|e| internal_error(format!("query get_tenant: {e}")))?;
        match rows.next() {
            Some(row) => {
                Ok(Some(row.map_err(|e| {
                    internal_error(format!("get_tenant row: {e}"))
                })?))
            }
            None => Ok(None),
        }
    }

    async fn register_tenant(
        &self,
        id: &str,
        display_name: Option<&str>,
    ) -> StorageResult<crate::core::TenantRecord> {
        // Backstop for the canonical tenant-id contract (issue #385). SQLite
        // keys tenants by an exact-match `tenant_id` column, so it has no
        // collision of its own to defend against — this guards the *registry*
        // from minting an id the other backends could not keep distinct, and
        // keeps the precondition uniform across every implementation.
        self.ensure_canonical_tenant_id(id)?;
        let conn = self.get_connection()?;
        // Plain INSERT so a duplicate id surfaces as a constraint error; the
        // admin handler pre-checks existence and returns 409, so reaching here
        // with a duplicate is a race and a 500 is acceptable.
        conn.execute(
            "INSERT INTO tenants (id, display_name, created_at) \
             VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%SZ','now'))",
            params![id, display_name],
        )
        .map_err(|e| internal_error(format!("register_tenant: {e}")))?;
        let mut stmt = conn
            .prepare("SELECT id, display_name, created_at FROM tenants WHERE id = ?1")
            .map_err(|e| internal_error(format!("prepare register read-back: {e}")))?;
        stmt.query_row(params![id], |row| {
            Ok(crate::core::TenantRecord {
                id: row.get(0)?,
                display_name: row.get(1)?,
                created_at: row.get(2)?,
            })
        })
        .map_err(|e| internal_error(format!("register read-back: {e}")))
    }

    async fn deregister_tenant(&self, id: &str) -> StorageResult<bool> {
        crate::tenant::ensure_mutable_tenant(id)?;
        let conn = self.get_connection()?;
        let changed = conn
            .execute("DELETE FROM tenants WHERE id = ?1", params![id])
            .map_err(|e| internal_error(format!("deregister_tenant: {e}")))?;
        Ok(changed > 0)
    }

    async fn purge_tenant_data(&self, id: &str) -> StorageResult<u64> {
        crate::tenant::ensure_mutable_tenant(id)?;
        let mut conn = self.get_connection()?;
        // IMMEDIATE, not the DEFERRED default: this transaction reads (the count
        // below) before it writes. Under WAL a deferred transaction takes a read
        // snapshot on that first read and then fails the read-to-write upgrade
        // with SQLITE_BUSY_SNAPSHOT if another connection committed in between —
        // and the busy handler is *not* invoked for that code, so the configured
        // busy_timeout does not cover it. Taking the write lock up front does.
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .or_query_error("purge begin")?;
        // Count current-version rows first so we can report what was removed.
        let removed: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM resources WHERE tenant_id = ?1",
                params![id],
                |row| row.get(0),
            )
            .or_query_error("purge count")?;
        // search_index has ON DELETE CASCADE from resources, but delete it
        // explicitly too in case foreign keys are not enforced on this handle.
        // (That explicit delete is also what fires the `search_index_fts`
        // triggers, though a cascade fires them too.)
        for sql in [
            "DELETE FROM search_index WHERE tenant_id = ?1",
            "DELETE FROM resource_history WHERE tenant_id = ?1",
            "DELETE FROM resources WHERE tenant_id = ?1",
        ] {
            tx.execute(sql, params![id])
                .or_query_error("purge delete")?;
        }
        // `resource_fts` is an FTS5 *virtual* table: it can carry no foreign key,
        // so the cascade above never reaches it and it must be deleted explicitly
        // (issue #386). Left behind, the purged resource's narrative and its
        // entire serialized body stay in the database, and are resurrected as a
        // match oracle the moment a resource reuses the same logical id.
        //
        // Deliberately NOT gated on `is_search_offloaded()`: that flag is a
        // write-path optimisation ("don't maintain an index nobody reads"),
        // whereas this is an erasure guarantee. A deployment that indexed
        // locally and later moved search to Elasticsearch still has rows here,
        // and a purge is precisely when they must go.
        purge_fts_rows(
            &tx,
            "DELETE FROM resource_fts WHERE tenant_id = ?1",
            params![id],
        )?;
        // The rowid mapping (#967) is a plain table, so it neither cascades
        // nor is reached by the FTS delete above.
        tx.execute(
            "DELETE FROM resource_fts_map WHERE tenant_id = ?1",
            params![id],
        )
        .or_query_error("purge FTS mapping")?;
        // Per-user settings are keyed by user, not tenant, so they are not swept
        // by the deletes above — but a client stores PHI-derived query strings in
        // them, which belong to this tenant (issue #313). Same transaction: this
        // connection already holds the write lock, and a second one would
        // deadlock against it.
        // Provider-side Bulk Submit submissions are tenant-keyed rows (#772).
        tx.execute(
            "DELETE FROM bulk_provider_submissions WHERE tenant_id = ?1",
            params![id],
        )
        .or_query_error("purge provider submissions")?;
        let settings = SqliteBackend::purge_tenant_settings_in_txn(&tx, id)?;
        tx.commit().or_query_error("purge commit")?;
        if settings > 0 {
            tracing::info!(
                tenant = %id,
                documents = settings,
                "purged tenant-scoped content from user settings documents"
            );
        }
        Ok(removed.max(0) as u64)
    }
}

// Search Index Helpers
impl SqliteBackend {
    /// Soft-deletes a resource, optionally only at `expected_version`
    /// ([`ResourceStorage::delete`] / [`ResourceStorage::delete_versioned`]).
    ///
    /// The read of the current row, the tombstone `UPDATE`, the deletion history
    /// row and the search-index cleanup share one `IMMEDIATE` transaction. They
    /// used to be auto-committed statements: a failure after the `UPDATE` left a
    /// tombstone with no history entry, and a writer landing between the read
    /// and the `UPDATE` turned a plain delete into a spurious `NotFound`. Inside
    /// the write lock neither can happen, and `expected_version` is compared
    /// against the very row the `UPDATE` then tombstones — the comparison and
    /// the delete cannot be separated (#1404).
    fn soft_delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: Option<&str>,
    ) -> StorageResult<()> {
        tenant.check_permission(Operation::Delete, resource_type)?;

        let mut conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| internal_error(format!("Failed to begin delete: {}", e)))?;

        // Check if resource exists and get its fhir_version
        let result: Result<(String, Vec<u8>, String), _> = tx.query_row(
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

        if let Some(expected) = expected_version
            && expected != current_version
        {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: expected.to_string(),
                    actual_version: current_version,
                },
            ));
        }

        let now = Utc::now();
        let deleted_at = now.to_rfc3339();

        // Calculate new version for the deletion record
        let new_version: u64 = current_version.parse().unwrap_or(0) + 1;
        let new_version_str = new_version.to_string();

        // Soft delete the resource. The `version_id`/`is_deleted` predicates
        // keep the statement a compare-and-swap in its own right: the write
        // lock already guarantees the row is the one read above, and the
        // predicate is what would say so if that ever stopped being true.
        let updated = tx
            .execute(
                "UPDATE resources SET is_deleted = 1, deleted_at = ?1, version_id = ?2, last_updated = ?1
                 WHERE tenant_id = ?3 AND resource_type = ?4 AND id = ?5
                   AND version_id = ?6 AND is_deleted = 0",
                params![
                    deleted_at,
                    new_version_str,
                    tenant_id,
                    resource_type,
                    id,
                    current_version
                ],
            )
            .map_err(|e| internal_error(format!("Failed to delete resource: {}", e)))?;

        if updated == 0 {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        }

        // Insert deletion record into history (preserve fhir_version)
        tx.execute(
            "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
            params![tenant_id, resource_type, id, new_version_str, data, deleted_at, fhir_version_str],
        )
        .map_err(|e| internal_error(format!("Failed to insert deletion history: {}", e)))?;

        // Delete search index entries (skip when search is offloaded). Keyed on
        // resource_key. The tenant_id/resource_type prefix is required for the
        // delete to seek idx_search_composite instead of full-scanning
        // search_index (see delete_search_index, #1197); the soft-delete keeps
        // the resources row, so the subquery resolves.
        if !self.is_search_offloaded() {
            tx.execute(
                "DELETE FROM search_index
                  WHERE tenant_id = ?1 AND resource_type = ?2
                    AND resource_key = (
                     SELECT rowid FROM resources
                      WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3
                 )",
                params![tenant_id, resource_type, id],
            )
            .map_err(|e| internal_error(format!("Failed to delete search index: {}", e)))?;
        }

        tx.commit()
            .map_err(|e| internal_error(format!("Failed to commit delete: {}", e)))?;

        // A SearchParameter delete invalidates this tenant's cached registry.
        if resource_type == "SearchParameter" {
            self.tenant_registries().invalidate(tenant_id);
        }

        Ok(())
    }

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
    fn restore_deleted(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        tenant.check_permission(Operation::Update, resource_type)?;

        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let row: Result<(String, String), _> = conn.query_row(
            "SELECT version_id, fhir_version FROM resources
             WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3 AND is_deleted = 1",
            params![tenant_id, resource_type, id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );

        let (deleted_version, fhir_version_str) = match row {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
            }
            Err(e) => {
                return Err(internal_error(format!(
                    "Failed to read deleted resource: {}",
                    e
                )));
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

        let data = serde_json::to_vec(&resource)
            .map_err(|e| serialization_error(format!("Failed to serialize resource: {}", e)))?;

        let now = Utc::now();
        let last_updated = now.to_rfc3339();

        conn.execute(
            "UPDATE resources
             SET version_id = ?1, data = ?2, last_updated = ?3, is_deleted = 0, deleted_at = NULL
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
        .map_err(|e| internal_error(format!("Failed to restore resource: {}", e)))?;

        conn.execute(
            "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
            params![tenant_id, resource_type, id, new_version_str, data, last_updated, fhir_version_str],
        )
        .map_err(|e| internal_error(format!("Failed to insert restore history: {}", e)))?;

        // The delete dropped the search index entries; rebuild them for the
        // resource that is live again.
        self.delete_search_index(&conn, tenant_id, resource_type, id)?;
        self.index_resource(&conn, tenant_id, resource_type, id, &resource)?;

        // A restored overlay-affecting SearchParameter re-enters this tenant's
        // overlay.
        if resource_type == "SearchParameter"
            && self.tenant_registries().create_affects_overlay(&resource)
        {
            self.tenant_registries().invalidate(tenant_id);
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
    /// Extracts the resource's search values with the tenant's registry-driven
    /// extractor and writes them, falling back to the hardcoded `_id` /
    /// `_lastUpdated` pair if extraction fails. The two halves are
    /// [`Self::prepare_index`] (pure CPU, no connection) and
    /// [`Self::write_prepared_index`] (the statements); bulk paths prepare a
    /// whole batch in parallel and call the second half alone.
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
        let prepared = self.prepare_index(tenant_id, resource_type, resource_id, resource);
        self.write_prepared_index(
            conn,
            tenant_id,
            resource_type,
            resource_id,
            resource,
            prepared,
        )
        .map(|_| ())
    }

    /// The database-free half of indexing one resource: FHIRPath extraction,
    /// value normalisation, the bound parameters of every `search_index` row,
    /// and the full-text content. Holds no connection and takes no lock other
    /// than the registry's read lock, so a batch of these can be built on a
    /// thread pool while the single writer connection is busy with the
    /// previous batch (see [`Self::prepare_index_batch`]).
    pub(crate) fn prepare_index(
        &self,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> PreparedIndex {
        use super::search::fts::extract_searchable_content;
        use crate::search::converters::IndexValue;

        let _span = crate::perf::span(crate::perf::Phase::Extract);
        let extractor = self.tenant_extractor(tenant_id);
        let rows = match extractor.extract(resource, resource_type) {
            Ok(values) => {
                let marshal_span = crate::perf::span(crate::perf::Phase::IndexMarshal);
                // Composite groups missing a component can never match a
                // composite search (`GROUP BY … HAVING` needs every axis) and
                // were 6% of all rows on a Synthea load; PostgreSQL already
                // skips them.
                let values = crate::search::extractor::drop_incomplete_composites(values);
                let rows = values
                    .into_iter()
                    .map(|v| {
                        let normalized = match &v.value {
                            IndexValue::Date {
                                value: d,
                                precision,
                                end,
                            } => {
                                let mut n = v.clone();
                                n.value = IndexValue::Date {
                                    value: Self::normalize_date_for_sqlite(d),
                                    precision: *precision,
                                    end: end.clone(),
                                };
                                n
                            }
                            _ => v,
                        };
                        // `resource_key` is patched in by `write_prepared_index`
                        // (this half is connection-free); pass a placeholder.
                        SqliteSearchIndexWriter::to_sql_params(
                            tenant_id,
                            resource_type,
                            resource_id,
                            0,
                            &normalized,
                        )
                    })
                    .collect();
                drop(marshal_span);
                Ok(rows)
            }
            Err(e) => Err(e.to_string()),
        };

        // Contained resources, for `_contained` search: each value row is
        // flagged `is_contained = 1` and carries the contained resource's
        // type and local id, with `resource_type` / `resource_id` naming the
        // container.
        let mut contained_rows = Vec::new();
        for contained in extractor.extract_contained(resource) {
            for value in &contained.values {
                let normalized = match &value.value {
                    IndexValue::Date {
                        value: d,
                        precision,
                        end,
                    } => {
                        let mut n = value.clone();
                        n.value = IndexValue::Date {
                            value: Self::normalize_date_for_sqlite(d),
                            precision: *precision,
                            end: end.clone(),
                        };
                        Some(n)
                    }
                    _ => None,
                };
                let mut params = SqliteSearchIndexWriter::to_sql_params(
                    tenant_id,
                    resource_type,
                    resource_id,
                    0, // resource_key patched in by write_prepared_index
                    normalized.as_ref().unwrap_or(value),
                );
                params.push(SqlValue::Int(1));
                params.push(SqlValue::String(contained.contained_type.clone()));
                params.push(SqlValue::String(contained.local_id.clone()));
                contained_rows.push(params);
            }
        }

        let fts = {
            let content = extract_searchable_content(resource);
            (!content.is_empty()).then_some(content)
        };

        PreparedIndex {
            rows,
            contained_rows,
            fts,
        }
    }

    /// [`Self::prepare_index`] for a whole batch, spread across a thread
    /// pool. Extraction is the largest CPU cost of indexing and is
    /// independent per resource; SQLite's single writer cannot be
    /// parallelised, but this can, and it moves the extraction off the
    /// writer's critical path entirely.
    ///
    /// Items are `(resource_type, resource_id, resource)`; the result is in
    /// input order. Small batches are prepared inline — the pool's
    /// scheduling overhead is not worth paying for a handful of resources.
    pub(crate) fn prepare_index_batch(
        &self,
        tenant_id: &str,
        items: &[(&str, &str, &Value)],
    ) -> Vec<PreparedIndex> {
        use rayon::prelude::*;

        let _span = crate::perf::span(crate::perf::Phase::PrepareBatch);
        if items.len() < PARALLEL_PREPARE_MIN_BATCH {
            return items
                .iter()
                .map(|(rt, id, res)| self.prepare_index(tenant_id, rt, id, res))
                .collect();
        }
        index_prepare_pool().install(|| {
            items
                .par_iter()
                .map(|(rt, id, res)| self.prepare_index(tenant_id, rt, id, res))
                .collect()
        })
    }

    /// The statement half of indexing one resource: `search_index` rows
    /// eight per INSERT, the contained rows, then the full-text row. Returns
    /// the number of `search_index` rows written. Runs the minimal `_id` /
    /// `_lastUpdated` fallback when extraction failed, as
    /// [`Self::index_resource`] always has.
    pub(crate) fn write_prepared_index(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
        prepared: PreparedIndex,
    ) -> StorageResult<usize> {
        let mut count = 0;
        // The connection-free prepare step left `resource_key` as a placeholder
        // (see RESOURCE_KEY_PARAM_IX); resolve the owning resource's rowid here,
        // where we hold the connection, and patch every row before binding. The
        // resource has already been written by the time indexing runs.
        let resource_key: i64 = conn
            .query_row(
                "SELECT rowid FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3",
                rusqlite::params![tenant_id, resource_type, resource_id],
                |r| r.get(0),
            )
            .map_err(|e| {
                internal_error(format!(
                    "resolve resource_key for {resource_type}/{resource_id}: {e}"
                ))
            })?;
        let mut prepared = prepared;
        if let Ok(rows) = prepared.rows.as_mut() {
            for row in rows.iter_mut() {
                row[SqliteSearchIndexWriter::RESOURCE_KEY_PARAM_IX] = SqlValue::Int(resource_key);
            }
        }
        for row in prepared.contained_rows.iter_mut() {
            row[SqliteSearchIndexWriter::RESOURCE_KEY_PARAM_IX] = SqlValue::Int(resource_key);
        }
        match prepared.rows {
            Ok(rows) => {
                // Rows are written eight at a time: a single-row INSERT stepped
                // once per row spends a measurable share of its time entering
                // and leaving the statement, and every resource writes ~10-30
                // rows. The B-tree work is unchanged; only the per-statement
                // overhead is amortized (measured +9% bulk-ingest throughput
                // on the real 31 GB manifest).
                let _span = crate::perf::span(crate::perf::Phase::IndexInsert);
                let mut i = 0;
                while i + 8 <= rows.len() {
                    let refs: Vec<&dyn ToSql> = rows[i..i + 8]
                        .iter()
                        .flatten()
                        .map(|p| self.sql_value_to_ref(p))
                        .collect();
                    conn.prepare_cached(SqliteSearchIndexWriter::insert_sql_rows8())
                        .and_then(|mut s| s.execute(refs.as_slice()))
                        .map_err(|e| internal_error(format!("multi-row index insert: {e}")))?;
                    i += 8;
                    count += 8;
                }
                for row in &rows[i..] {
                    let refs: Vec<&dyn ToSql> =
                        row.iter().map(|p| self.sql_value_to_ref(p)).collect();
                    conn.prepare_cached(SqliteSearchIndexWriter::insert_sql())
                        .and_then(|mut s| s.execute(refs.as_slice()))
                        .map_err(|e| internal_error(format!("index insert: {e}")))?;
                    count += 1;
                }
                for row in &prepared.contained_rows {
                    let refs: Vec<&dyn ToSql> =
                        row.iter().map(|p| self.sql_value_to_ref(p)).collect();
                    conn.prepare_cached(SqliteSearchIndexWriter::insert_contained_sql())
                        .and_then(|mut s| s.execute(refs.as_slice()))
                        .map_err(|e| {
                            internal_error(format!(
                                "Failed to insert contained search index entry: {}",
                                e
                            ))
                        })?;
                    count += 1;
                }
                crate::perf::add_rows(crate::perf::Phase::IndexInsert, count as u64);
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
        if let Some(content) = prepared.fts {
            let _span = crate::perf::span(crate::perf::Phase::Fts);
            self.index_fts_content(conn, tenant_id, resource_type, resource_id, content)?;
        }

        Ok(count)
    }

    /// Writes the full-text row for `_text` / `_content` searches, if FTS5 is
    /// available, and records the rowid FTS5 assigned.
    fn index_fts_content(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        content: super::search::fts::SearchableContent,
    ) -> StorageResult<()> {
        if !fts_table_exists(conn)? {
            // FTS5 not available - skip silently
            return Ok(());
        }

        // Insert into FTS table
        conn.prepare_cached(
            "INSERT INTO resource_fts (resource_id, resource_type, tenant_id, narrative_text, full_content)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .map_err(|e| internal_error(format!("Failed to prepare FTS insert: {}", e)))?
        .execute(params![
            resource_id,
            resource_type,
            tenant_id,
            content.narrative,
            content.full_content
        ])
        .map_err(|e| internal_error(format!("Failed to insert FTS content: {}", e)))?;

        // Remember which rowid FTS5 assigned. `resource_fts`'s plain columns
        // are UNINDEXED, so deleting by (tenant, type, id) can only scan the
        // whole table; deleting by rowid is a b-tree lookup (#967).
        conn.prepare_cached(
            "INSERT OR REPLACE INTO resource_fts_map
                (tenant_id, resource_type, resource_id, fts_rowid)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(|e| internal_error(format!("Failed to prepare FTS map insert: {}", e)))?
        .execute(params![
            tenant_id,
            resource_type,
            resource_id,
            conn.last_insert_rowid()
        ])
        .map_err(|e| internal_error(format!("Failed to record FTS rowid: {}", e)))?;

        Ok(())
    }

    /// Removes one resource's `resource_fts` row(s) through the rowid mapping,
    /// and the mapping itself.
    ///
    /// Falls back to the scanning delete when the resource has no mapping. That
    /// is the self-heal for a row written before schema v23 by a build that
    /// never recorded its rowid and that the migration's backfill somehow
    /// missed: correct, slow, and it maps itself on the next write. After a
    /// v23 migration it should never fire.
    fn delete_fts_rows_for(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<()> {
        if !fts_table_exists(conn)? {
            return Ok(());
        }

        let rowids: Vec<i64> = {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT fts_rowid FROM resource_fts_map
                     WHERE tenant_id = ?1 AND resource_type = ?2 AND resource_id = ?3",
                )
                .map_err(|e| internal_error(format!("prepare FTS map lookup: {e}")))?;
            let rows = stmt
                .query_map(params![tenant_id, resource_type, resource_id], |row| {
                    row.get(0)
                })
                .map_err(|e| internal_error(format!("query FTS map: {e}")))?;
            rows.filter_map(|r| r.ok()).collect()
        };

        if rowids.is_empty() {
            tracing::debug!(
                resource_type,
                resource_id,
                "no FTS rowid mapping; falling back to the scanning delete"
            );
            let _ = conn.execute(
                "DELETE FROM resource_fts WHERE tenant_id = ?1 AND resource_type = ?2 AND resource_id = ?3",
                params![tenant_id, resource_type, resource_id],
            );
            return Ok(());
        }

        for rowid in rowids {
            conn.prepare_cached("DELETE FROM resource_fts WHERE rowid = ?1")
                .map_err(|e| internal_error(format!("prepare FTS delete: {e}")))?
                .execute(params![rowid])
                .map_err(|e| internal_error(format!("delete FTS row: {e}")))?;
        }
        conn.prepare_cached(
            "DELETE FROM resource_fts_map
             WHERE tenant_id = ?1 AND resource_type = ?2 AND resource_id = ?3",
        )
        .map_err(|e| internal_error(format!("prepare FTS map delete: {e}")))?
        .execute(params![tenant_id, resource_type, resource_id])
        .map_err(|e| internal_error(format!("delete FTS mapping: {e}")))?;

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
    /// Removes a resource's search entries, returning how many `search_index`
    /// rows were deleted.
    pub(crate) fn delete_search_index(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<u64> {
        // When search is offloaded to a secondary backend, skip local index cleanup
        if self.is_search_offloaded() {
            return Ok(0);
        }

        // Delete from main search index by resource_key. The `tenant_id` and
        // `resource_type` equality prefix is load-bearing, not redundant with
        // the subquery: `idx_search_composite` leads with
        // `(tenant_id, resource_type, resource_key, …)`, so a predicate on
        // `resource_key` alone cannot use it and SQLite falls back to a full
        // scan of `search_index` — O(rows) per delete, which is O(rows) per
        // resource UPDATE and per re-indexed resource (#1197). With the prefix
        // the delete is a covering seek. Every caller runs while the `resources`
        // row still exists (update, re-index, soft-delete), so the subquery
        // resolves; the purge path removes `resources` first and deletes by
        // `resource_id` inline instead.
        let deleted = conn
            .prepare_cached(
                "DELETE FROM search_index
                  WHERE tenant_id = ?1 AND resource_type = ?2
                    AND resource_key = (
                     SELECT rowid FROM resources
                      WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3
                 )",
            )
            .map_err(|e| internal_error(format!("Failed to prepare search index delete: {}", e)))?
            .execute(params![tenant_id, resource_type, resource_id])
            .map_err(|e| internal_error(format!("Failed to delete search index: {}", e)))?;

        // Delete from FTS by rowid. `resource_fts` is an FTS5 virtual table
        // whose plain columns are UNINDEXED, so a WHERE on them scans the
        // entire table — once per resource, which made every re-index
        // quadratic in corpus size (#967: 0.2 entries/s against 6M FTS
        // documents). FTS5 does index rowid, and `resource_fts_map` records
        // the one it assigned. #949's `deleted > 0` guard still stands in
        // front: a resource with no search_index rows was never FTS-indexed,
        // so there is nothing to look up.
        if deleted > 0 {
            self.delete_fts_rows_for(conn, tenant_id, resource_type, resource_id)?;
        }

        Ok(deleted as u64)
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
            "INSERT INTO search_index (tenant_id, resource_type, resource_id, resource_key, param_name, value_token_system, value_token_code)
             VALUES (?1, ?2, ?3, (SELECT rowid FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3), ?4, ?5, ?6)",
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

        // The end of the value's range (#1391), read from the text as written.
        let end = super::search::writer::stored_date_end(
            &crate::search::converters::IndexValue::date(value),
        );

        conn.execute(
            "INSERT INTO search_index (tenant_id, resource_type, resource_id, resource_key, param_name, value_date, value_date_end)
             VALUES (?1, ?2, ?3, (SELECT rowid FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3), ?4, ?5, ?6)",
            params![tenant_id, resource_type, resource_id, param_name, normalized, end],
        )
        .map_err(|e| internal_error(format!("Failed to insert date index: {}", e)))?;
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

        // Check version match. `expected_version` is the client's `If-Match`
        // field value, which is a LIST and is satisfied when any listed tag
        // matches (issue #311); it may also arrive in any ETag spelling
        // (`W/"1"`, `"1"`, `1`). All four backends share this comparison.
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

        // List-aware `If-Match` comparison, shared with every other backend.
        // This previously compared the raw current version against a normalized
        // expected value, which happened to work only because stored values are
        // always bare.
        if !if_match_field_satisfied(expected_version, &current_version) {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: normalize_etag(expected_version).to_string(),
                    actual_version: current_version,
                },
            ));
        }
        drop(conn);

        // Delete exactly the version the precondition was evaluated against.
        // A plain `delete` here was check-then-act: a writer landing after the
        // read above was deleted along with the version the client named
        // (#1404).
        self.delete_versioned(tenant, resource_type, id, &current_version)
            .await
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
            .or_query_error("Failed to prepare history query")?;

        let rows = stmt
            .query_map(params![tenant_id, resource_type, id], |row| {
                let version_id: String = row.get(0)?;
                let data: Vec<u8> = row.get(1)?;
                let last_updated: String = row.get(2)?;
                let is_deleted: i32 = row.get(3)?;
                let fhir_version: String = row.get(4)?;
                Ok((version_id, data, last_updated, is_deleted, fhir_version))
            })
            .or_query_error("Failed to query history")?;

        let mut entries = Vec::new();
        let mut last_version: Option<String> = None;

        for row in rows {
            let (version_id, data, last_updated_str, is_deleted, fhir_version_str) =
                row.or_query_error("Failed to read history row")?;

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
            .or_query_error("Failed to count history")?;

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
            .or_query_error("Failed to delete history")?;

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
            .or_query_error("Failed to prepare type history query")?;

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
            .or_query_error("Failed to query type history")?;

        let mut entries = Vec::new();
        let mut last_entry: Option<(String, String)> = None; // (last_updated, id)

        for row in rows {
            let (id, version_id, data, last_updated_str, is_deleted, fhir_version_str) =
                row.or_query_error("Failed to read type history row")?;

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

        let mut stmt = conn
            .prepare(&sql)
            .or_query_error("Failed to prepare system history query")?;

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
            .or_query_error("Failed to query system history")?;

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
            ) = row.or_query_error("Failed to read system history row")?;

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
            .or_query_error("Failed to count system history")?;

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
        let mut conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // One transaction for the whole purge. Previously these were four
        // independent autocommit statements, so a failure (or a crash) partway
        // through left the resource deleted but its full text still in
        // `resource_fts` — the exact orphan state this method now exists to
        // prevent — and the caller could not tell a partial purge from a failed
        // one, because a retry hits the not-found guard below and reports
        // `NotFound` while the residue remains. IMMEDIATE for the same
        // read-then-write / WAL reason as `purge_tenant_data`.
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| internal_error(format!("purge begin: {e}")))?;

        // Check if resource exists (in any state)
        let exists: bool = tx
            .query_row(
                "SELECT 1 FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3",
                params![tenant_id, resource_type, id],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if !exists {
            // Also check history in case it was already purged from main table
            let history_exists: bool = tx
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
        tx.execute(
            "DELETE FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3",
            params![tenant_id, resource_type, id],
        )
        .or_query_error("Failed to purge resource")?;

        // Delete from history table
        tx.execute(
            "DELETE FROM resource_history WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3",
            params![tenant_id, resource_type, id],
        )
        .or_query_error("Failed to purge resource history")?;

        // Delete from search index
        tx.execute(
            "DELETE FROM search_index WHERE tenant_id = ?1 AND resource_type = ?2 AND resource_id = ?3",
            params![tenant_id, resource_type, id],
        )
        .or_query_error("Failed to purge search index")?;

        // Delete the full-text rows: no cascade reaches an FTS5 virtual table.
        // See `purge_tenant_data` for why this is strict and ungated.
        purge_fts_rows(
            &tx,
            "DELETE FROM resource_fts WHERE tenant_id = ?1 AND resource_type = ?2 AND resource_id = ?3",
            params![tenant_id, resource_type, id],
        )?;
        tx.execute(
            "DELETE FROM resource_fts_map
             WHERE tenant_id = ?1 AND resource_type = ?2 AND resource_id = ?3",
            params![tenant_id, resource_type, id],
        )
        .or_query_error("purge FTS mapping")?;

        tx.commit()
            .map_err(|e| internal_error(format!("purge commit: {e}")))?;

        Ok(())
    }

    async fn purge_all(&self, tenant: &TenantContext, resource_type: &str) -> StorageResult<u64> {
        let mut conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Single transaction — see `purge` for the rationale.
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| internal_error(format!("purge_all begin: {e}")))?;

        // Count how many we're about to delete
        let count: i64 = tx
            .query_row(
                "SELECT COUNT(DISTINCT id) FROM resources WHERE tenant_id = ?1 AND resource_type = ?2",
                params![tenant_id, resource_type],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Delete from resources table
        tx.execute(
            "DELETE FROM resources WHERE tenant_id = ?1 AND resource_type = ?2",
            params![tenant_id, resource_type],
        )
        .or_query_error("Failed to purge resources")?;

        // Delete from history table
        tx.execute(
            "DELETE FROM resource_history WHERE tenant_id = ?1 AND resource_type = ?2",
            params![tenant_id, resource_type],
        )
        .or_query_error("Failed to purge resource history")?;

        // Delete from search index
        tx.execute(
            "DELETE FROM search_index WHERE tenant_id = ?1 AND resource_type = ?2",
            params![tenant_id, resource_type],
        )
        .or_query_error("Failed to purge search index")?;

        // Delete the full-text rows: no cascade reaches an FTS5 virtual table.
        // See `purge_tenant_data` for why this is strict and ungated.
        purge_fts_rows(
            &tx,
            "DELETE FROM resource_fts WHERE tenant_id = ?1 AND resource_type = ?2",
            params![tenant_id, resource_type],
        )?;
        tx.execute(
            "DELETE FROM resource_fts_map WHERE tenant_id = ?1 AND resource_type = ?2",
            params![tenant_id, resource_type],
        )
        .or_query_error("purge FTS mapping")?;

        tx.commit()
            .map_err(|e| internal_error(format!("purge_all commit: {e}")))?;

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

#[async_trait]
impl ConditionalStorage for SqliteBackend {
    fn supports_conditional(&self, interaction: crate::core::ConditionalInteraction) -> bool {
        // One declaration: the capability list the contract test pins (#1384).
        crate::core::Backend::supports(self, interaction.capability())
    }

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

    #[allow(clippy::too_many_arguments)]
    async fn conditional_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        search_params: &str,
        upsert: bool,
        fhir_version: FhirVersion,
        if_match: &crate::core::EntityTagPrecondition,
    ) -> StorageResult<ConditionalUpdateResult> {
        // Find matching resources based on search parameters
        let matches = self
            .find_matching_resources(tenant, resource_type, search_params)
            .await?;

        match matches.len() {
            0 => {
                // `If-Match` names a version; nothing matched, so nothing
                // can carry it and the create below must not run (#1381).
                crate::core::conditional_if_match_gate(if_match, resource_type, None)?;
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
                // Exactly one match - update it (preserves existing FHIR version).
                // `update` compares-and-swaps on `existing`'s version, the one
                // `If-Match` is evaluated against here.
                let existing = matches.into_iter().next().unwrap();
                crate::core::conditional_if_match_gate(if_match, resource_type, Some(&existing))?;
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
        if_match: &crate::core::EntityTagPrecondition,
    ) -> StorageResult<ConditionalDeleteResult> {
        // Find matching resources based on search parameters
        let matches = self
            .find_matching_resources(tenant, resource_type, search_params)
            .await?;

        match matches.len() {
            0 => {
                // No match. A supplied `If-Match` fails against it, as it
                // does on `DELETE [type]/[id]` for a missing resource.
                crate::core::conditional_if_match_gate(if_match, resource_type, None)?;
                Ok(ConditionalDeleteResult::NoMatch)
            }
            1 => {
                // Exactly one match - delete it
                let existing = matches.into_iter().next().unwrap();
                crate::core::conditional_if_match_gate(if_match, resource_type, Some(&existing))?;
                crate::core::delete_under_precondition(self, tenant, if_match, &existing).await?;
                Ok(ConditionalDeleteResult::Deleted(existing))
            }
            n => {
                // Multiple matches - error condition
                Ok(ConditionalDeleteResult::MultipleMatches(n))
            }
        }
    }

    /// The criteria resolver the provided
    /// [`ConditionalStorage::conditional_patch`] is written in terms of: this
    /// backend has no patch code of its own (#1406).
    async fn resolve_conditional_matches(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
    ) -> StorageResult<Vec<StoredResource>> {
        self.find_matching_resources(tenant, resource_type, search_params)
            .await
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
        let Some(query) = self.conditional_query(tenant, resource_type, search_params_str)? else {
            return Ok(Vec::new());
        };

        // Use the SearchProvider implementation which uses the search index
        let result = <Self as SearchProvider>::search(self, tenant, &query).await?;

        Ok(result.resources.items)
    }

    /// Resolves conditional criteria on the transaction's own connection, so
    /// the match set includes what earlier entries of the same bundle wrote
    /// (#511). The pooled-connection twin above cannot see those rows under
    /// `BEGIN IMMEDIATE`.
    fn find_matching_resources_in_tx(
        &self,
        tenant: &TenantContext,
        tx: &crate::backends::sqlite::transaction::SqliteTransaction,
        resource_type: &str,
        search_params_str: &str,
    ) -> StorageResult<Vec<StoredResource>> {
        let Some(query) = self.conditional_query(tenant, resource_type, search_params_str)? else {
            return Ok(Vec::new());
        };

        tx.with_connection(|conn| self.search_with_connection(conn, tenant, &query, None))
            .map(|result| result.resources.items)
    }

    /// Builds the search a conditional interaction's criteria describe, or
    /// `None` when the criteria are empty — matching everything would be the
    /// literal reading, but no conditional interaction means that.
    ///
    /// The parsing is [`crate::search::build_conditional_query`], shared by
    /// every backend so criteria mean what they mean as a direct search
    /// (#1312).
    fn conditional_query(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params_str: &str,
    ) -> StorageResult<Option<SearchQuery>> {
        let registry_arc = self.tenant_registry(tenant.tenant_id().as_str());
        let registry = registry_arc.read();
        crate::search::build_conditional_query(
            &registry,
            resource_type,
            search_params_str,
            crate::search::ResourceTypeScope::version(self.config().fhir_version),
        )
    }
}

#[async_trait]
impl BundleProvider for SqliteBackend {
    /// SQLite provides real ACID transactions; `SqliteBackend` also implements
    /// [`TransactionProvider`](crate::core::TransactionProvider), and
    /// `process_transaction` runs inside one, so a failure unwinds completely.
    fn supports_atomic_transactions(&self) -> bool {
        true
    }

    async fn process_transaction_with_patch_validator(
        &self,
        tenant: &TenantContext,
        entries: Vec<BundleEntry>,
        fhir_version: helios_fhir::FhirVersion,
        validator: Option<&dyn PatchCandidateValidator>,
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
        let mut patch_error: Option<TransactionError> = None;

        // Build a map of fullUrl -> assigned reference for reference resolution
        // This maps urn:uuid:xxx to ResourceType/assigned-id after creates
        let mut reference_map: HashMap<String, String> = HashMap::new();

        // Whether any entry in this transaction writes a SearchParameter that
        // affects this tenant's cached overlay (#787: transaction-bundle writes
        // never invalidated the registry, so a SearchParameter POSTed inside a
        // Bundle — e.g. by Inferno's US Core setup — never took effect until
        // the TTL cache refresh). Mirrors the non-transactional create/update/
        // delete checks below (create is conditional via `create_affects_overlay`;
        // update/delete are unconditional).
        let mut search_param_overlay_changed = false;

        // Make entries mutable for reference resolution
        let mut entries = entries;

        // Process each entry within the transaction
        for (idx, entry) in entries.iter_mut().enumerate() {
            // Resolve references in this entry's resource before processing
            if let Some(ref mut resource) = entry.resource {
                resolve_bundle_references(resource, &reference_map);
            }

            let result = self
                .process_bundle_entry_tx(tenant, &mut tx, entry, fhir_version, validator)
                .await;

            match result {
                Ok(entry_result) => {
                    // Check for error status codes
                    if entry_result.status >= 400 {
                        if entry.method == BundleMethod::Patch {
                            patch_error = Some(TransactionError::PatchEntry {
                                index: idx,
                                status: entry_result.status,
                                outcome: entry_result.outcome.clone().unwrap_or_default(),
                            });
                        }
                        error_info = Some((
                            idx,
                            format!("Entry failed with status {}", entry_result.status),
                        ));
                        break;
                    }

                    if !search_param_overlay_changed {
                        search_param_overlay_changed =
                            match entry_result.status {
                                // Created (POST, or PUT-as-create): only overlay-affecting
                                // creates need to invalidate (see `create_affects_overlay`).
                                201 => entry_result
                                    .resource
                                    .as_ref()
                                    .filter(|r| {
                                        r.get("resourceType").and_then(|v| v.as_str())
                                            == Some("SearchParameter")
                                    })
                                    .is_some_and(|r| {
                                        self.tenant_registries().create_affects_overlay(r)
                                    }),
                                // Updated (PUT/PATCH): unconditional, like the
                                // non-transactional update path.
                                200 => {
                                    entry_result.resource.as_ref().and_then(|r| {
                                        r.get("resourceType").and_then(|v| v.as_str())
                                    }) == Some("SearchParameter")
                                }
                                // Deleted: the emptied result carries no resource, so
                                // parse the type from the entry's URL instead.
                                204 => self
                                    .parse_url(&entry.url)
                                    .map(|(resource_type, _)| resource_type == "SearchParameter")
                                    .unwrap_or(false),
                                _ => false,
                            };
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
            return Err(patch_error.unwrap_or(TransactionError::BundleError { index, message }));
        }

        // Commit the transaction
        Box::new(tx)
            .commit()
            .await
            .map_err(|e| TransactionError::RolledBack {
                reason: format!("Commit failed: {}", e),
            })?;

        // A committed SearchParameter write in this transaction changes this
        // tenant's cached overlay — drop it so the next access rebuilds from
        // storage (#787).
        if search_param_overlay_changed {
            self.tenant_registries()
                .invalidate(tenant.tenant_id().as_str());
        }

        Ok(BundleResult {
            bundle_type: BundleType::Transaction,
            entries: results,
        })
    }
}

impl SqliteBackend {
    /// Process a single bundle entry within a transaction.
    async fn process_bundle_entry_tx(
        &self,
        tenant: &TenantContext,
        tx: &mut crate::backends::sqlite::transaction::SqliteTransaction,
        entry: &BundleEntry,
        bundle_version: helios_fhir::FhirVersion,
        validator: Option<&dyn PatchCandidateValidator>,
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

                if let Some(criteria) = entry.if_none_exist.as_deref() {
                    // With search offloaded to a secondary backend the local
                    // index is empty for every row, so an in-transaction
                    // search would always find nothing and this arm would
                    // create the duplicate `ifNoneExist` exists to prevent.
                    // Refuse the entry instead; the bundle rolls back (#511).
                    if self.is_search_offloaded() {
                        return Ok(crate::core::not_supported_entry(
                            "ifNoneExist cannot be resolved inside a transaction when search \
                             is offloaded to a secondary backend; submit the entry in a batch \
                             Bundle instead",
                        ));
                    }
                    let matches =
                        self.find_matching_resources_in_tx(tenant, tx, &resource_type, criteria)?;
                    if let Some(gated) = crate::core::bundle_if_none_exist_gate(matches) {
                        return Ok(gated);
                    }
                }

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
                        Ok(BundleEntryResult::updated(updated))
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

                // Honor `ifMatch` on DELETE — it was previously ignored here, so
                // a client asking to delete only the version it had reviewed
                // could destroy a concurrent amendment with no 412. The read is
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
                let (resource_type, id) = self.parse_url(&entry.url)?;
                if resource_type == "AuditEvent" {
                    return Ok(BundleEntryResult::error(
                        405,
                        serde_json::json!({
                            "resourceType": "OperationOutcome",
                            "issue": [{"severity": "error", "code": "not-supported", "details": {"text": "AuditEvent resources are immutable"}}]
                        }),
                    ));
                }
                let existing = tx.read(&resource_type, &id).await?;
                if let Some(failure) = bundle_if_match_gate(
                    entry.if_match.as_deref(),
                    existing.as_ref().map(|r| r.version_id()),
                ) {
                    return Ok(failure);
                }
                let Some(existing) = existing else {
                    return Ok(BundleEntryResult::error(
                        404,
                        serde_json::json!({
                            "resourceType": "OperationOutcome",
                            "issue": [{"severity": "error", "code": "not-found", "details": {"text": format!("{resource_type}/{id} not found")}}]
                        }),
                    ));
                };
                let candidate = match prepare_bundle_patch(
                    tenant,
                    &resource_type,
                    &existing,
                    entry.resource.as_ref(),
                    bundle_version,
                    validator,
                )
                .await
                {
                    Ok(candidate) => candidate,
                    Err(failure) => return Ok(*failure),
                };
                patch_update_result(tx.update(&existing, candidate).await)
            }
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

// ReindexSource: SQLite is a primary, so it is where resources are read from.
#[async_trait]
impl ReindexSource for SqliteBackend {
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
        self.fetch_resources_page_capped(tenant, resource_type, cursor, limit, 0)
            .await
    }

    /// Pages by resource count and, when `max_bytes` is set, by the bytes of
    /// stored JSON the page carries: a page of ~108 KB `Provenance` resources
    /// ends at the first one that crosses the cap instead of holding ~108 MB
    /// before a single document is written (#1125). At least one resource is
    /// always returned, so the page loop advances even on a resource larger
    /// than the cap.
    async fn fetch_resources_page_capped(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
        max_bytes: u64,
    ) -> StorageResult<ResourcePage> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str().to_string();

        // Parse cursor if provided (format: "last_updated|id")
        // Split on the last `|`: an id cannot contain one, but the stored
        // `last_updated` text of a corrupted row can.
        let (cursor_ts, cursor_id) = match cursor.and_then(|c| c.rsplit_once('|')) {
            Some((ts, id)) => (Some(ts.to_string()), Some(id.to_string())),
            None => (None, None),
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

        let rows = stmt
            .query_map(param_refs.as_slice(), RawReindexRow::read)
            .map_err(|e| internal_error(format!("Failed to query resources: {}", e)))?;

        let mut resources = Vec::with_capacity(limit as usize);
        let mut skipped = Vec::new();
        let mut scanned = 0usize;
        let mut last_scanned: Option<String> = None;
        let mut bytes = 0u64;
        let mut capped = false;
        for row in rows {
            // A step error is the database failing, not one row being bad:
            // surface it rather than guess where the next page starts.
            let row =
                row.map_err(|e| internal_error(format!("Failed to read resource row: {}", e)))?;
            scanned += 1;
            // The cursor follows every row the query returned, decodable or
            // not, and uses the stored text so the keyset comparison in the
            // SQL above sees exactly what it compares against (#1125).
            if let (Some(last_updated), Some(id)) = (&row.last_updated, &row.id) {
                last_scanned = Some(format!("{last_updated}|{id}"));
            }
            let row_bytes = row.data.as_ref().map_or(0, |data| data.len() as u64);
            match row.decode(tenant, resource_type) {
                Ok(resource) => resources.push(resource),
                Err(skip) => {
                    tracing::warn!(
                        tenant = %tenant.tenant_id(),
                        resource_type,
                        resource_id = %skip.resource_id,
                        reason = %skip.reason,
                        "reindex source: stored resource row cannot be decoded; skipping it"
                    );
                    skipped.push(skip);
                }
            }
            // Counted after the row is taken, so a page always carries at
            // least one resource however large it is.
            bytes = bytes.saturating_add(row_bytes);
            if max_bytes > 0 && bytes >= max_bytes && scanned < limit as usize {
                capped = true;
                break;
            }
        }

        // A full page, or one the byte cap ended early, means there may be
        // more rows, however many of them decoded: deciding on
        // `resources.len()` let one unreadable row end the pagination of its
        // whole type silently.
        let next_cursor = if limit > 0 && (capped || scanned == limit as usize) {
            Some(last_scanned.ok_or_else(|| {
                internal_error(format!(
                    "Cannot page {resource_type}: no row in a full page has a readable id and lastUpdated"
                ))
            })?)
        } else {
            None
        };

        Ok(ResourcePage {
            resources,
            next_cursor,
            skipped,
        })
    }

    async fn fetch_resources_by_ids(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        ids: &[String],
    ) -> StorageResult<Vec<StoredResource>> {
        let mut unique: Vec<&str> = ids.iter().map(String::as_str).collect();
        unique.sort_unstable();
        unique.dedup();
        if unique.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();
        let mut found = Vec::with_capacity(unique.len());
        for batch in unique.chunks(FETCH_BY_IDS_BATCH) {
            let placeholders = (0..batch.len())
                .map(|k| format!("?{}", k + 3))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT id, version_id, data, last_updated, fhir_version FROM resources \
                 WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0 \
                 AND id IN ({placeholders})"
            );
            let mut params: Vec<&dyn ToSql> = Vec::with_capacity(batch.len() + 2);
            params.push(&tenant_id);
            params.push(&resource_type);
            for id in batch {
                params.push(id);
            }
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| internal_error(format!("Failed to prepare statement: {}", e)))?;
            let rows = stmt
                .query_map(params.as_slice(), RawReindexRow::read)
                .map_err(|e| internal_error(format!("Failed to query resources: {}", e)))?;
            for row in rows {
                let row =
                    row.map_err(|e| internal_error(format!("Failed to read resource row: {}", e)))?;
                match row.decode(tenant, resource_type) {
                    Ok(resource) => found.push(resource),
                    // The contract has no channel for these: the scan that
                    // first met the row already reported it as skipped.
                    Err(skip) => tracing::warn!(
                        tenant = %tenant.tenant_id(),
                        resource_type,
                        resource_id = %skip.resource_id,
                        reason = %skip.reason,
                        "reindex source: stored resource row cannot be decoded; skipping it"
                    ),
                }
            }
        }
        Ok(found)
    }
}

/// Ids bound per `IN (...)` query in [`SqliteBackend::fetch_resources_by_ids`],
/// far below SQLite's bound-parameter limit.
const FETCH_BY_IDS_BATCH: usize = 500;

/// One `resources` row as the reindex source reads it, before decoding.
///
/// Every column is read leniently so a bad value becomes a
/// [`SkippedResource`] for that row instead of an error that ends the query
/// (or, as before #1125, a row that silently vanished).
struct RawReindexRow {
    id: Option<String>,
    version_id: Option<String>,
    data: Option<Vec<u8>>,
    last_updated: Option<String>,
    fhir_version: Option<String>,
}

impl RawReindexRow {
    /// Column order: `id, version_id, data, last_updated, fhir_version`.
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: column_text(row, 0),
            version_id: column_text(row, 1),
            data: match row.get_ref(2)? {
                rusqlite::types::ValueRef::Blob(bytes) | rusqlite::types::ValueRef::Text(bytes) => {
                    Some(bytes.to_vec())
                }
                _ => None,
            },
            last_updated: column_text(row, 3),
            fhir_version: column_text(row, 4),
        })
    }

    fn decode(
        self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> Result<StoredResource, SkippedResource> {
        let resource_id = self.id.unwrap_or_default();
        let skip = |reason: String| SkippedResource {
            resource_id: resource_id.clone(),
            reason,
        };
        if resource_id.is_empty() {
            return Err(skip("row has no readable id".to_string()));
        }
        let version_id = self
            .version_id
            .ok_or_else(|| skip("row has no readable versionId".to_string()))?;
        let data = self
            .data
            .ok_or_else(|| skip("row has no resource content".to_string()))?;
        let content: Value = serde_json::from_slice(&data)
            .map_err(|e| skip(format!("resource content is not valid JSON: {e}")))?;
        let last_updated = self
            .last_updated
            .ok_or_else(|| skip("row has no readable lastUpdated".to_string()))?;
        let last_modified = chrono::DateTime::parse_from_rfc3339(&last_updated)
            .map_err(|e| {
                skip(format!(
                    "lastUpdated {last_updated:?} is not a timestamp: {e}"
                ))
            })?
            .with_timezone(&Utc);
        let fhir_version = self
            .fhir_version
            .as_deref()
            .and_then(FhirVersion::from_storage)
            .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);
        Ok(StoredResource::from_storage(
            resource_type.to_string(),
            resource_id.clone(),
            version_id,
            tenant.tenant_id().clone(),
            content,
            last_modified, // created_at (use last_modified as approximation)
            last_modified,
            None, // not deleted
            fhir_version,
        ))
    }
}

/// A column as text whatever its storage class, or `None` for NULL or an
/// out-of-range index.
fn column_text(row: &rusqlite::Row<'_>, index: usize) -> Option<String> {
    use rusqlite::types::ValueRef;
    match row.get_ref(index).ok()? {
        ValueRef::Null => None,
        ValueRef::Integer(v) => Some(v.to_string()),
        ValueRef::Real(v) => Some(v.to_string()),
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => {
            Some(String::from_utf8_lossy(bytes).into_owned())
        }
    }
}

// ReindexTarget: SQLite keeps search entries in its own `search_index`
// table, so it is also a writer and can reindex itself standalone.
impl SqliteBackend {
    /// Writes one resource's search entries — dynamic extraction, contained
    /// resources, and the FTS row — on the caller's connection, so a batched
    /// reindex can put a whole page inside one transaction.
    fn write_search_entries_on(
        &self,
        conn: &rusqlite::Connection,
        tenant: &TenantContext,
        resource: &StoredResource,
    ) -> StorageResult<usize> {
        // Nothing reads this index when search is offloaded, and the matching
        // delete (`delete_search_index`) is already a no-op there, so writing
        // would only accumulate dead rows on every rebuild (#1125).
        if self.is_search_offloaded() {
            return Ok(0);
        }
        let resource_type = resource.resource_type();
        let resource_id = resource.id();
        let content = resource.content();

        // The same two halves the CRUD path uses. Every caller deletes the
        // resource's search entries (FTS row included) immediately before
        // this — without that, `$reindex` was a *destructive* operation for
        // `_text`/`_content`: the delete dropped the FTS row and nothing put
        // it back. `index_fts_content` is a bare INSERT with no
        // delete-first, so if that ordering ever changes this must become
        // delete-then-insert.
        let tenant_id = tenant.tenant_id().as_str();
        let prepared = self.prepare_index(tenant_id, resource_type, resource_id, content);
        self.write_prepared_index(
            conn,
            tenant_id,
            resource_type,
            resource_id,
            content,
            prepared,
        )
    }

    /// [`Self::write_search_entries_on`] with the extraction already done —
    /// the reindex page loop prepares a page in parallel and then writes it
    /// through here on the one connection.
    fn write_prepared_search_entries_on(
        &self,
        conn: &rusqlite::Connection,
        tenant: &TenantContext,
        resource: &StoredResource,
        prepared: PreparedIndex,
    ) -> StorageResult<usize> {
        self.write_prepared_index(
            conn,
            tenant.tenant_id().as_str(),
            resource.resource_type(),
            resource.id(),
            resource.content(),
            prepared,
        )
    }
}

#[async_trait]
impl ReindexTarget for SqliteBackend {
    async fn delete_search_entries(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<u64> {
        let conn = self.get_connection()?;
        // Reuses the delete path the CRUD layer uses, so the FTS table is
        // cleaned up too and the `is_search_offloaded` guard is honored.
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
        resource: &StoredResource,
    ) -> StorageResult<usize> {
        let conn = self.get_connection()?;
        self.write_search_entries_on(&conn, tenant, resource)
    }

    /// Rebuilds a page of resources inside one IMMEDIATE transaction — the
    /// reindex-side counterpart of the #815 batch ingest. Per-resource
    /// autocommit made the fast-load rebuild (#903) run at ~50-100
    /// resources/s, giving back most of what the deferred ingest won.
    async fn write_search_entries_page(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
    ) -> Vec<StorageResult<usize>> {
        // Same guard as `write_search_entries_on` and
        // `begin_bulk_index_rebuild`: when a secondary owns search, a
        // composite still wires this backend as a reindex writer, and the
        // page must neither extract, take the write lock, nor insert (#1125).
        if self.is_search_offloaded() {
            return resources.iter().map(|_| Ok(0)).collect();
        }
        let conn = match self.get_connection() {
            Ok(conn) => conn,
            Err(e) => {
                let msg = e.to_string();
                return resources
                    .iter()
                    .map(|_| Err(internal_error(msg.clone())))
                    .collect();
            }
        };
        if let Err(e) = conn.execute("BEGIN IMMEDIATE", []) {
            let msg = format!("Failed to begin reindex batch: {e}");
            return resources
                .iter()
                .map(|_| Err(internal_error(msg.clone())))
                .collect();
        }
        let page_span = crate::perf::span(crate::perf::Phase::ReindexPage);
        let tenant_id = tenant.tenant_id().as_str();
        // Extraction for the whole page first, across the thread pool, while
        // this connection holds the write lock for nothing else; the loop
        // below is then statements only.
        let items: Vec<(&str, &str, &Value)> = resources
            .iter()
            .map(|r| (r.resource_type(), r.id(), r.content()))
            .collect();
        let prepared = self.prepare_index_batch(tenant_id, &items);
        let mut results: Vec<StorageResult<usize>> = Vec::with_capacity(resources.len());
        for (resource, prepared) in resources.iter().zip(prepared) {
            let outcome = {
                let _span = crate::perf::span(crate::perf::Phase::IndexDelete);
                self.delete_search_index(&conn, tenant_id, resource.resource_type(), resource.id())
            }
            .and_then(|_| self.write_prepared_search_entries_on(&conn, tenant, resource, prepared));
            results.push(outcome);
        }
        let commit_span = crate::perf::span(crate::perf::Phase::Commit);
        let committed = conn.execute("COMMIT", []);
        drop(commit_span);
        drop(page_span);
        if let Err(e) = committed {
            let _ = conn.execute("ROLLBACK", []);
            let msg = format!("Failed to commit reindex batch: {e}");
            return resources
                .iter()
                .map(|_| Err(internal_error(msg.clone())))
                .collect();
        }
        results
    }

    /// Drops the `search_index` value indexes for the duration of the run.
    /// Measured on a 72k-resource rebuild that already prepared its pages in
    /// parallel: 18.5s with the indexes maintained row by row, 10.7s without
    /// them plus 2.75s to build all thirteen sorted at the end — and the
    /// sorted build is sequential I/O, so the gap widens once the b-trees
    /// no longer fit the page cache. Reference-counted across concurrent
    /// runs: the first one in drops, the last one out rebuilds. A process
    /// that dies inside the window is healed at the next startup by
    /// `schema::ensure_search_value_indexes`.
    async fn begin_bulk_index_rebuild(&self) -> StorageResult<()> {
        if self.is_search_offloaded() {
            return Ok(());
        }
        let mut active = BULK_INDEX_REBUILDS.lock();
        if *active == 0 {
            let conn = self.get_connection()?;
            let started = std::time::Instant::now();
            super::schema::drop_search_value_indexes(&conn)?;
            tracing::info!(
                elapsed_ms = started.elapsed().as_millis() as u64,
                "bulk index rebuild: search_index value indexes dropped for the run"
            );
        }
        *active += 1;
        Ok(())
    }

    async fn end_bulk_index_rebuild(&self) -> StorageResult<()> {
        if self.is_search_offloaded() {
            return Ok(());
        }
        let mut active = BULK_INDEX_REBUILDS.lock();
        *active = active.saturating_sub(1);
        if *active == 0 {
            let conn = self.get_connection()?;
            let started = std::time::Instant::now();
            let created = super::schema::ensure_search_value_indexes(&conn)?;
            tracing::info!(
                created,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "bulk index rebuild: search_index value indexes rebuilt"
            );
        }
        Ok(())
    }

    async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
        // Offloaded: the writes above are no-ops, so clearing must be one too
        // or `$reindex` with `clearExisting` would be the only operation that
        // still touches this index.
        if self.is_search_offloaded() {
            return Ok(0);
        }
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let deleted = conn
            .execute(
                "DELETE FROM search_index WHERE tenant_id = ?1",
                params![tenant_id],
            )
            .or_query_error("Failed to clear search index")?;

        // Clear the full-text rows too, matching PostgreSQL. This is only sound
        // because `write_search_entries` above now repopulates them; adding this
        // delete on its own would have made `$reindex --clear-existing` wipe
        // `_text`/`_content` permanently.
        //
        // The returned count deliberately stays the `search_index` total —
        // callers and tests treat it as the number of index entries cleared.
        purge_fts_rows(
            &conn,
            "DELETE FROM resource_fts WHERE tenant_id = ?1",
            params![tenant_id],
        )?;
        // …and the rowid mapping that pointed at them (#967), or the reindex
        // would leave every resource mapped to a row that no longer exists.
        conn.execute(
            "DELETE FROM resource_fts_map WHERE tenant_id = ?1",
            params![tenant_id],
        )
        .or_query_error("clear FTS mapping")?;

        Ok(deleted as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::history::HistoryParams;
    use crate::tenant::{TenantId, TenantPermissions};
    use crate::types::{SearchParamType, SearchParameter, SearchValue};
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

    /// #1078: SQLite's `count_all_types` / `count_deltas_by_bucket` are real
    /// aggregates, so it opts in to `supports_type_counts`.
    #[test]
    fn test_supports_type_counts() {
        let backend = SqliteBackend::in_memory().unwrap();
        assert!(backend.supports_type_counts());
    }

    fn other_tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("other-tenant"),
            TenantPermissions::full_access(),
        )
    }

    /// #1078: an empty tenant has no newest write, and zero recent writes only
    /// when a bound was asked for.
    #[tokio::test]
    async fn test_latest_write_marker_empty_tenant() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let marker = backend
            .latest_write_marker(&tenant, None)
            .await
            .unwrap()
            .expect("SQLite provides a marker");
        assert_eq!(
            marker,
            crate::core::WriteMarker {
                latest: None,
                recent_writes: None
            }
        );

        let since = Utc::now() - chrono::Duration::hours(1);
        let marker = backend
            .latest_write_marker(&tenant, Some(since))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(marker.latest, None);
        assert_eq!(marker.recent_writes, Some(0));
    }

    /// #1078: every committed write of the tenant — create, update, delete —
    /// changes its marker; another tenant's writes do not.
    #[tokio::test]
    async fn test_latest_write_marker_changes_on_each_write_and_is_tenant_scoped() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let since = Some(Utc::now() - chrono::Duration::hours(1));
        let marker = || async { backend.latest_write_marker(&tenant, since).await.unwrap() };

        let empty = marker().await;

        let created = backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        let after_create = marker().await;
        assert_ne!(after_create, empty, "a create changes the marker");
        let after_create_marker = after_create.unwrap();
        assert_eq!(after_create_marker.latest, Some(created.last_modified()));
        assert_eq!(after_create_marker.recent_writes, Some(1));

        let updated = backend
            .update(&tenant, &created, json!({"active": true}))
            .await
            .unwrap();
        let after_update = marker().await;
        assert_ne!(after_update, after_create, "an update changes the marker");
        assert_eq!(after_update.unwrap().latest, Some(updated.last_modified()));

        backend
            .delete(&tenant, "Patient", created.id())
            .await
            .unwrap();
        let after_delete = marker().await;
        assert_ne!(after_delete, after_update, "a delete changes the marker");
        assert_eq!(after_delete.unwrap().recent_writes, Some(3));

        // Another tenant's writes leave this tenant's marker alone.
        let other = other_tenant();
        backend
            .create(&other, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        assert_eq!(marker().await, after_delete);
        let other_marker = backend
            .latest_write_marker(&other, since)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(other_marker.recent_writes, Some(1));
    }

    /// #1078: `recent_writes` counts only the tenant's history rows at or after
    /// the bound, and stops at the cap.
    #[tokio::test]
    async fn test_latest_write_marker_recent_writes_bound_and_cap() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let start = Utc::now() - chrono::Duration::seconds(1);

        for _ in 0..3 {
            backend
                .create(&tenant, "Patient", json!({}), FhirVersion::default())
                .await
                .unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let bound = Utc::now();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        for _ in 0..2 {
            backend
                .create(&tenant, "Observation", json!({}), FhirVersion::default())
                .await
                .unwrap();
        }

        let recent = |since, cap| {
            let backend = &backend;
            let tenant = &tenant;
            async move {
                backend
                    .latest_write_marker_capped(tenant, Some(since), cap)
                    .await
                    .unwrap()
                    .unwrap()
                    .recent_writes
            }
        };
        assert_eq!(recent(bound, WRITE_MARKER_RECENT_CAP).await, Some(2));
        assert_eq!(recent(start, WRITE_MARKER_RECENT_CAP).await, Some(5));
        assert_eq!(recent(start, 3).await, Some(3), "capped");
        assert_eq!(
            recent(Utc::now() + chrono::Duration::hours(1), 3).await,
            Some(0)
        );

        // The trait method uses the production cap.
        let marker = backend
            .latest_write_marker(&tenant, Some(start))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(marker.recent_writes, Some(5));
    }

    /// #1078: both marker probes are index searches on `idx_history_updated` —
    /// never a table or full-index scan, and the newest-row probe never sorts.
    #[test]
    fn test_latest_write_marker_query_plans_use_the_history_index() {
        let backend = create_test_backend();
        let conn = backend.get_connection().unwrap();
        let plan = |sql: &str, binds: &[&dyn ToSql]| -> Vec<String> {
            let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            stmt.query_map(binds, |row| row.get::<_, String>(3))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        let bound = Utc::now().to_rfc3339();

        for (name, details) in [
            ("latest", plan(LATEST_WRITE_SQL, &[&"t"])),
            (
                "recent",
                plan(RECENT_WRITES_SQL, &[&"t", &bound, &WRITE_MARKER_RECENT_CAP]),
            ),
        ] {
            assert!(
                details
                    .iter()
                    .any(|d| d.starts_with("SEARCH resource_history")
                        && d.contains("idx_history_updated")),
                "{name}: expected an idx_history_updated search, got {details:?}"
            );
            assert!(
                !details
                    .iter()
                    .any(|d| d.starts_with("SCAN resource_history")),
                "{name}: must not scan resource_history, got {details:?}"
            );
            assert!(
                !details.iter().any(|d| d.contains("TEMP B-TREE")),
                "{name}: must not sort, got {details:?}"
            );
        }
    }

    fn count_rows(backend: &SqliteBackend, table: &str, tenant: &TenantContext) -> i64 {
        let conn = backend.get_connection().unwrap();
        conn.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE tenant_id = ?1"),
            params![tenant.tenant_id().as_str()],
            |row| row.get(0),
        )
        .unwrap()
    }

    /// #1125: on `sqlite-es` the offloaded primary is still handed to the
    /// reindex as a writer. Its delete was already a no-op there, so an
    /// unguarded write added a full set of dead `search_index` and FTS rows on
    /// every rebuild. Every write-side entry point must now leave the index
    /// exactly as it found it — rows from before the offload included.
    #[tokio::test]
    async fn reindex_writes_are_no_ops_when_search_is_offloaded() {
        let mut backend = create_test_backend();
        let tenant = create_test_tenant();
        for i in 1..=3 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": format!("p{i}"),
                        "name": [{"family": "Offload", "given": ["Ann"]}]
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        let index_rows = count_rows(&backend, "search_index", &tenant);
        let fts_rows = count_rows(&backend, "resource_fts", &tenant);
        assert!(index_rows > 0, "precondition: the resources were indexed");
        assert!(fts_rows > 0, "precondition: the FTS rows exist");

        backend.set_search_offloaded(true);
        let page = backend
            .fetch_resources_page(&tenant, "Patient", None, 100)
            .await
            .unwrap();
        assert_eq!(page.resources.len(), 3);

        for _ in 0..2 {
            let results = backend
                .write_search_entries_page(&tenant, &page.resources)
                .await;
            assert_eq!(results.len(), 3, "one result per resource");
            assert!(results.iter().all(|r| matches!(r, Ok(0))), "{results:?}");
        }
        for resource in &page.resources {
            assert_eq!(
                backend
                    .write_search_entries(&tenant, resource)
                    .await
                    .unwrap(),
                0
            );
        }
        assert_eq!(backend.clear_search_index(&tenant).await.unwrap(), 0);

        assert_eq!(
            count_rows(&backend, "search_index", &tenant),
            index_rows,
            "offloaded reindex writes must neither add nor clear search_index rows"
        );
        assert_eq!(
            count_rows(&backend, "resource_fts", &tenant),
            fts_rows,
            "offloaded reindex writes must neither add nor clear FTS rows"
        );
    }

    /// #1125: a row whose content or timestamp does not parse used to vanish
    /// from its page, and because the cursor was only emitted for a page that
    /// *decoded* to `limit` resources, it also ended the pagination of its
    /// type. It is now reported and the scan carries on past it.
    #[tokio::test]
    async fn fetch_resources_page_reports_unparseable_rows_and_keeps_paginating() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        for i in 1..=5 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({"resourceType": "Patient", "id": format!("p{i}")}),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        {
            let conn = backend.get_connection().unwrap();
            conn.execute(
                "UPDATE resources SET data = CAST('{not json' AS BLOB) \
                 WHERE tenant_id = ?1 AND resource_type = 'Patient' AND id = 'p1'",
                params![tenant.tenant_id().as_str()],
            )
            .unwrap();
            conn.execute(
                "UPDATE resources SET last_updated = 'not-a-date' \
                 WHERE tenant_id = ?1 AND resource_type = 'Patient' AND id = 'p2'",
                params![tenant.tenant_id().as_str()],
            )
            .unwrap();
        }

        let mut decoded = Vec::new();
        let mut skipped = Vec::new();
        let mut cursor: Option<String> = None;
        let mut pages = 0;
        loop {
            let page = backend
                .fetch_resources_page(&tenant, "Patient", cursor.as_deref(), 2)
                .await
                .unwrap();
            pages += 1;
            assert!(pages <= 5, "pagination must terminate");
            decoded.extend(page.resources.iter().map(|r| r.id().to_string()));
            skipped.extend(page.skipped);
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        decoded.sort();
        assert_eq!(
            decoded,
            ["p3", "p4", "p5"],
            "every decodable resource after the bad rows must still be read"
        );
        skipped.sort_by(|a, b| a.resource_id.cmp(&b.resource_id));
        let skipped_ids: Vec<&str> = skipped.iter().map(|s| s.resource_id.as_str()).collect();
        assert_eq!(skipped_ids, ["p1", "p2"]);
        assert!(skipped[0].reason.contains("JSON"), "{}", skipped[0].reason);
        assert!(
            skipped[1].reason.contains("lastUpdated"),
            "{}",
            skipped[1].reason
        );
    }

    /// #1125: `HFS_REINDEX_BATCH_BYTES` bounds a reindex page by the bytes of
    /// stored JSON it carries as well as by its resource count, so a page of
    /// large resources is not one oversized read. Under a cap only one
    /// resource fits beneath, the scan must still walk the whole type — every
    /// resource once, no resource twice — and then stop.
    #[tokio::test]
    async fn fetch_resources_page_capped_pages_one_resource_at_a_time_under_a_byte_cap() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        for i in 1..=5 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": format!("p{i}"),
                        "name": [{"family": "Capped"}]
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        // A cap the smallest stored row already reaches on its own: whichever
        // resource a page starts with ends it.
        let cap: u64 = {
            let conn = backend.get_connection().unwrap();
            conn.query_row(
                "SELECT MIN(LENGTH(data)) FROM resources \
                 WHERE tenant_id = ?1 AND resource_type = 'Patient' AND is_deleted = 0",
                params![tenant.tenant_id().as_str()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap() as u64
        };
        assert!(cap > 0, "precondition: the rows have stored bytes");

        let mut seen = Vec::new();
        let mut cursor: Option<String> = None;
        let mut pages = 0;
        loop {
            let page = backend
                .fetch_resources_page_capped(&tenant, "Patient", cursor.as_deref(), 100, cap)
                .await
                .unwrap();
            pages += 1;
            assert!(pages <= 10, "pagination must terminate");
            assert!(
                page.resources.len() <= 1,
                "the cap admits one resource per page, got {}",
                page.resources.len()
            );
            seen.extend(page.resources.iter().map(|r| r.id().to_string()));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        seen.sort();
        assert_eq!(
            seen,
            ["p1", "p2", "p3", "p4", "p5"],
            "every resource is returned exactly once across the capped pages"
        );

        // `0` is the cap switched off: the same rows come back as one page.
        let whole = backend
            .fetch_resources_page_capped(&tenant, "Patient", None, 100, 0)
            .await
            .unwrap();
        assert_eq!(whole.resources.len(), 5);
        assert!(whole.next_cursor.is_none());
    }

    /// #1125: the bytes are counted *after* the row is taken, so a resource
    /// larger than the cap is still returned. A page that refused it would be
    /// empty, and the reindex loop would either stop early or ask for the same
    /// page forever.
    #[tokio::test]
    async fn fetch_resources_page_capped_returns_a_resource_larger_than_the_cap() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "big",
                    "name": [{"family": "X".repeat(20_000)}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "small"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Two bytes: below every row, the oversized one included.
        let mut seen = Vec::new();
        let mut cursor: Option<String> = None;
        let mut pages = 0;
        loop {
            let page = backend
                .fetch_resources_page_capped(&tenant, "Patient", cursor.as_deref(), 100, 2)
                .await
                .unwrap();
            pages += 1;
            assert!(pages <= 4, "pagination must terminate");
            if seen.len() < 2 {
                assert_eq!(
                    page.resources.len(),
                    1,
                    "page {pages} is empty: a resource over the cap was refused"
                );
            }
            seen.extend(page.resources.iter().map(|r| r.id().to_string()));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        seen.sort();
        assert_eq!(seen, ["big", "small"]);
    }

    /// The SQLite writer drops its value indexes on `begin` and has every one
    /// of them back on `end`, with the rows written meanwhile indexed — the
    /// rebuild wrote them into the table only, and the sorted build picked
    /// them up.
    #[tokio::test]
    async fn bulk_index_rebuild_restores_every_value_index_and_search_works() {
        use crate::search::{ReindexOperation, ReindexRequest};

        let backend = std::sync::Arc::new(create_test_backend());
        let tenant = create_test_tenant();
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p1", "name": [{"family": "Rebuild"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let index_names = |backend: &SqliteBackend| -> Vec<String> {
            let conn = backend.get_connection().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'search_index' ORDER BY name",
                )
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        let before = index_names(&backend);
        assert!(before.len() > 2);

        backend.begin_bulk_index_rebuild().await.unwrap();
        assert_eq!(
            index_names(&backend),
            vec!["idx_search_composite".to_string()]
        );
        // Nested run: still dropped, and the outer end is the one that rebuilds.
        backend.begin_bulk_index_rebuild().await.unwrap();
        backend.end_bulk_index_rebuild().await.unwrap();
        assert_eq!(
            index_names(&backend),
            vec!["idx_search_composite".to_string()]
        );
        backend.end_bulk_index_rebuild().await.unwrap();
        assert_eq!(index_names(&backend), before);

        // The whole run, through the driver, on a database whose rows were
        // written without the indexes.
        let op = ReindexOperation::new(backend.clone(), backend.tenant_registries().clone());
        let id = op
            .start(
                tenant.clone(),
                ReindexRequest::for_types(["Patient"]).with_bulk_index_rebuild(true),
                None,
            )
            .await
            .unwrap();
        // The run does its SQLite work on blocking threads, so yielding the
        // async runtime is not enough to let it finish: wait on the clock.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            if op.get_progress(&id).await.unwrap().status.is_finished() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "reindex did not finish within 60s"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let progress = op.get_progress(&id).await.unwrap();
        assert!(progress.errors.is_empty(), "{:?}", progress.errors);
        assert_eq!(index_names(&backend), before);

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "family".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Rebuild")],
            chain: vec![],
            components: vec![],
        });
        let results = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(results.resources.items.len(), 1);
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

    // ========================================================================
    // Console dashboard count_* tests
    // ========================================================================

    #[tokio::test]
    async fn test_count_by_types() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Seed a small deterministic dataset: 2 Patients, 1 Observation.
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

        let counts = backend
            .count_by_types(&tenant, &["Patient", "Observation", "Encounter"])
            .await
            .unwrap();
        let map: std::collections::HashMap<String, u64> = counts.into_iter().collect();
        assert_eq!(map.get("Patient"), Some(&2));
        assert_eq!(map.get("Observation"), Some(&1));
        // A type with zero rows is ABSENT from the result, not a 0 row.
        assert!(!map.contains_key("Encounter"));
    }

    #[tokio::test]
    async fn test_count_all_types() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

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

        let counts = backend.count_all_types(&tenant).await.unwrap();
        let map: std::collections::HashMap<String, u64> = counts.into_iter().collect();
        assert_eq!(map.get("Patient"), Some(&2));
        assert_eq!(map.get("Observation"), Some(&1));
    }

    #[tokio::test]
    async fn test_count_by_day() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // `since` = start of today (UTC midnight), built the same way the handler
        // does; `today` is derived from the same clock so this stays date-robust.
        let since = Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let today = Utc::now().date_naive();

        let rows = backend
            .count_by_day(&tenant, "Patient", since)
            .await
            .unwrap();
        let today_row = rows
            .iter()
            .find(|r| r.day == today)
            .expect("today bucket should be present");
        assert_eq!(today_row.count, 2);
    }

    /// The delta rule, straight from SQL: a create is `+1`, an update is `0` (it
    /// must not move the resource into a newer bucket — the whole point of reading
    /// history rather than `resources.last_updated`), and a delete is `-1`.
    #[tokio::test]
    async fn test_count_deltas_by_bucket() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let first = backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        // An update writes a v2 history row, which must contribute nothing.
        backend
            .update(&tenant, &first, json!({"active": true}))
            .await
            .unwrap();

        // A one-minute bucket, so the writes above all land in the same one.
        let since = Utc::now() - chrono::Duration::minutes(5);
        let rows = backend
            .count_deltas_by_bucket(&tenant, "Patient", since, 60)
            .await
            .unwrap();

        assert_eq!(
            rows.iter().map(|r| r.delta).sum::<i64>(),
            2,
            "two creates and one update net to +2"
        );
        assert!(
            rows.iter().all(|r| r.bucket_start.timestamp() % 60 == 0),
            "buckets are epoch-aligned to their width"
        );

        // Deleting one nets it back out.
        backend
            .delete(&tenant, "Patient", first.id())
            .await
            .unwrap();
        let rows = backend
            .count_deltas_by_bucket(&tenant, "Patient", since, 60)
            .await
            .unwrap();
        assert_eq!(rows.iter().map(|r| r.delta).sum::<i64>(), 1);

        // Buckets narrower than the window still cover it, and a bogus width is
        // rejected rather than silently dividing by zero.
        assert!(
            backend
                .count_deltas_by_bucket(&tenant, "Patient", since, 0)
                .await
                .is_err()
        );
    }

    /// #1078: the grouped `count_deltas_by_type_and_bucket` returns exactly the
    /// `(type, bucket, delta)` rows the per-type `count_deltas_by_bucket` calls
    /// return — over creates, updates and deletes in two buckets, per tenant,
    /// with a type netting to zero in a bucket and a requested type with no
    /// rows at all.
    #[tokio::test]
    async fn test_count_deltas_by_type_and_bucket_matches_per_type_calls() {
        use std::collections::BTreeSet;

        let backend = create_test_backend();
        let tenant_a = create_test_tenant();
        let tenant_b = TenantContext::new(
            TenantId::new("other-tenant"),
            TenantPermissions::full_access(),
        );
        let v = FhirVersion::default();

        // Earlier bucket, backdated below: tenant A creates two Patients
        // (updating one) and an Observation, and creates then deletes an
        // Encounter (a net zero); tenant B creates three Patients.
        let p1 = backend
            .create(&tenant_a, "Patient", json!({}), v)
            .await
            .unwrap();
        let p2 = backend
            .create(&tenant_a, "Patient", json!({}), v)
            .await
            .unwrap();
        let p1 = backend
            .update(&tenant_a, &p1, json!({"active": true}))
            .await
            .unwrap();
        let o1 = backend
            .create(&tenant_a, "Observation", json!({}), v)
            .await
            .unwrap();
        let e1 = backend
            .create(&tenant_a, "Encounter", json!({}), v)
            .await
            .unwrap();
        backend
            .delete(&tenant_a, "Encounter", e1.id())
            .await
            .unwrap();
        let mut b_patients = Vec::new();
        for _ in 0..3 {
            b_patients.push(
                backend
                    .create(&tenant_b, "Patient", json!({}), v)
                    .await
                    .unwrap(),
            );
        }
        let earlier = Utc::now() - chrono::Duration::hours(2);
        backend
            .get_connection()
            .unwrap()
            .execute(
                "UPDATE resource_history SET last_updated = ?1",
                params![earlier.to_rfc3339()],
            )
            .unwrap();

        // Current bucket: tenant A deletes a Patient, updates p1 and the
        // Observation, and creates an Observation and an Encounter; tenant B
        // deletes a Patient and creates an Observation.
        backend.delete(&tenant_a, "Patient", p2.id()).await.unwrap();
        backend
            .update(&tenant_a, &p1, json!({"active": false}))
            .await
            .unwrap();
        backend
            .update(&tenant_a, &o1, json!({"status": "final"}))
            .await
            .unwrap();
        backend
            .create(&tenant_a, "Observation", json!({}), v)
            .await
            .unwrap();
        backend
            .create(&tenant_a, "Encounter", json!({}), v)
            .await
            .unwrap();
        backend
            .delete(&tenant_b, "Patient", b_patients[0].id())
            .await
            .unwrap();
        backend
            .create(&tenant_b, "Observation", json!({}), v)
            .await
            .unwrap();

        let since = Utc::now() - chrono::Duration::hours(3);
        let bucket = 3600;
        let earlier_bucket = crate::core::bucket_floor(earlier, bucket).timestamp();
        let types = ["Patient", "Observation", "Encounter", "Condition"];

        type Row = (String, i64, i64);
        let mut grouped_by_tenant = Vec::new();
        for tenant in [&tenant_a, &tenant_b] {
            let grouped: Vec<Row> = backend
                .count_deltas_by_type_and_bucket(tenant, &types, since, bucket)
                .await
                .unwrap()
                .into_iter()
                .map(|(rt, d)| (rt, d.bucket_start.timestamp(), d.delta))
                .collect();
            let mut per_type: BTreeSet<Row> = BTreeSet::new();
            for rt in types {
                for d in backend
                    .count_deltas_by_bucket(tenant, rt, since, bucket)
                    .await
                    .unwrap()
                {
                    per_type.insert((rt.to_string(), d.bucket_start.timestamp(), d.delta));
                }
            }
            let grouped_set: BTreeSet<Row> = grouped.iter().cloned().collect();
            assert_eq!(grouped.len(), grouped_set.len(), "no duplicate rows");
            assert_eq!(grouped_set, per_type, "{}", tenant.tenant_id().as_str());
            assert!(grouped.iter().all(|(_, b, d)| b % bucket == 0 && *d != 0));
            assert!(
                grouped
                    .windows(2)
                    .all(|w| w[0].0 != w[1].0 || w[0].1 < w[1].1),
                "buckets ascend within a type"
            );
            assert!(
                !grouped.iter().any(|(rt, _, _)| rt == "Condition"),
                "a type with no rows contributes none"
            );
            grouped_by_tenant.push(grouped_set);
        }

        let net = |rows: &BTreeSet<Row>, rt: &str| -> i64 {
            rows.iter()
                .filter(|(t, _, _)| t == rt)
                .map(|(_, _, d)| d)
                .sum()
        };
        let at = |rows: &BTreeSet<Row>, rt: &str, b: i64| -> Option<i64> {
            rows.iter()
                .find(|(t, rb, _)| t == rt && *rb == b)
                .map(|(_, _, d)| *d)
        };
        let a = &grouped_by_tenant[0];
        let buckets: BTreeSet<i64> = a.iter().map(|(_, b, _)| *b).collect();
        assert!(buckets.len() >= 2, "the fixture spans two buckets: {a:?}");
        assert_eq!(at(a, "Patient", earlier_bucket), Some(2));
        assert_eq!(at(a, "Observation", earlier_bucket), Some(1));
        assert_eq!(
            at(a, "Encounter", earlier_bucket),
            None,
            "a create and delete in one bucket net to zero and are dropped"
        );
        assert_eq!(net(a, "Patient"), 1);
        assert_eq!(net(a, "Observation"), 2);
        assert_eq!(net(a, "Encounter"), 1);

        let b = &grouped_by_tenant[1];
        assert_eq!(at(b, "Patient", earlier_bucket), Some(3), "tenant-isolated");
        assert_eq!(net(b, "Patient"), 2);
        assert_eq!(net(b, "Observation"), 1);
        assert_eq!(net(b, "Encounter"), 0);

        // A duplicated type is counted once; no types means no rows; a bogus
        // width is rejected like the per-type method's.
        let twice = backend
            .count_deltas_by_type_and_bucket(&tenant_a, &["Patient", "Patient"], since, bucket)
            .await
            .unwrap();
        assert_eq!(twice.len(), a.iter().filter(|r| r.0 == "Patient").count());
        assert!(
            backend
                .count_deltas_by_type_and_bucket(&tenant_a, &[], since, bucket)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            backend
                .count_deltas_by_type_and_bucket(&tenant_a, &["Patient"], since, 0)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_activity_histogram() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // 3 writes for this tenant -> 3 resource_history rows.
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

        let since = Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();

        let cells = backend.activity_histogram(&tenant, since).await.unwrap();
        assert!(!cells.is_empty());
        // Total across returned cells equals the number of writes seeded.
        let total: u64 = cells.iter().map(|c| c.count).sum();
        assert_eq!(total, 3);
    }

    #[tokio::test]
    async fn test_count_by_tenant() {
        let backend = create_test_backend();
        let tenant_a =
            TenantContext::new(TenantId::new("tenant-a"), TenantPermissions::full_access());
        let tenant_b =
            TenantContext::new(TenantId::new("tenant-b"), TenantPermissions::full_access());

        // tenant-a: 3 resources, tenant-b: 2 resources.
        backend
            .create(&tenant_a, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant_a, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant_a, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant_b, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant_b, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // Cross-tenant admin aggregate: takes NO TenantContext.
        let counts = backend.count_by_tenant().await.unwrap();
        let map: std::collections::HashMap<String, u64> = counts.into_iter().collect();
        assert_eq!(map.get("tenant-a"), Some(&3));
        assert_eq!(map.get("tenant-b"), Some(&2));
    }

    #[test]
    fn test_is_cluster_shared() {
        let backend = create_test_backend();
        assert!(!backend.is_cluster_shared());
    }

    #[tokio::test]
    async fn test_tenant_registry_crud() {
        let backend = create_test_backend();
        assert!(backend.supports_tenant_registry());

        // Empty to start.
        assert!(backend.list_tenants().await.unwrap().is_empty());
        assert!(backend.get_tenant("acme").await.unwrap().is_none());

        // Register two tenants, one with a display name.
        let acme = backend
            .register_tenant("acme", Some("Acme Health"))
            .await
            .unwrap();
        assert_eq!(acme.id, "acme");
        assert_eq!(acme.display_name.as_deref(), Some("Acme Health"));
        assert!(!acme.created_at.is_empty());
        backend.register_tenant("beta", None).await.unwrap();

        let all = backend.list_tenants().await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(backend.get_tenant("acme").await.unwrap(), Some(acme));

        // Duplicate registration is an error (handler pre-checks for 409).
        assert!(backend.register_tenant("acme", None).await.is_err());

        // Deregister removes the row; second call reports nothing removed.
        assert!(backend.deregister_tenant("beta").await.unwrap());
        assert!(!backend.deregister_tenant("beta").await.unwrap());
        assert_eq!(backend.list_tenants().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_purge_tenant_data() {
        let backend = create_test_backend();
        let acme = TenantContext::new(TenantId::new("acme"), TenantPermissions::full_access());
        let other = TenantContext::new(TenantId::new("other"), TenantPermissions::full_access());

        for _ in 0..3 {
            backend
                .create(&acme, "Patient", json!({}), FhirVersion::default())
                .await
                .unwrap();
        }
        backend
            .create(&other, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // Purge removes acme's data only, reporting the row count.
        let removed = backend.purge_tenant_data("acme").await.unwrap();
        assert_eq!(removed, 3);

        let counts: std::collections::HashMap<String, u64> = backend
            .count_by_tenant()
            .await
            .unwrap()
            .into_iter()
            .collect();
        assert_eq!(counts.get("acme"), None);
        assert_eq!(counts.get("other"), Some(&1));
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
                &crate::core::EntityTagPrecondition::Absent,
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
                &crate::core::EntityTagPrecondition::Absent,
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
                &crate::core::EntityTagPrecondition::Absent,
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
            .conditional_delete(
                &tenant,
                "Patient",
                "_id=p1",
                &crate::core::EntityTagPrecondition::Absent,
            )
            .await
            .unwrap();

        match result {
            ConditionalDeleteResult::Deleted(_) => {
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
            .conditional_delete(
                &tenant,
                "Patient",
                "_id=nonexistent",
                &crate::core::EntityTagPrecondition::Absent,
            )
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
            .conditional_patch(
                &tenant,
                "Patient",
                "_id=p1",
                &patch,
                &crate::core::EntityTagPrecondition::Absent,
            )
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
            .conditional_patch(
                &tenant,
                "Patient",
                "_id=p1",
                &patch,
                &crate::core::EntityTagPrecondition::Absent,
            )
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
            .conditional_patch(
                &tenant,
                "Patient",
                "_id=nonexistent",
                &patch,
                &crate::core::EntityTagPrecondition::Absent,
            )
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

    /// #350: bundle-created resources must be stamped with the bundle's
    /// negotiated version, not the compile-time default.
    ///
    /// This covers the transaction arm only. The batch arm is executed by the
    /// REST layer, and its half of #350 is pinned by
    /// `batch_entries_stamp_the_configured_default` in
    /// `helios-rest/tests/default_version_fallback.rs`.
    #[cfg(feature = "R5")]
    #[tokio::test]
    async fn test_bundle_writes_stamp_the_negotiated_version() {
        use crate::core::transaction::BundleProvider;

        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let entry = |id: &str| BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({"resourceType": "Patient", "id": id})),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: None,
        };

        let tx_result = backend
            .process_transaction(&tenant, vec![entry("tx-r5")], helios_fhir::FhirVersion::R5)
            .await
            .unwrap();
        assert_eq!(tx_result.entries[0].status, 201);

        let stored = backend
            .read(&tenant, "Patient", "tx-r5")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.fhir_version(),
            helios_fhir::FhirVersion::R5,
            "tx-r5 should be stamped R5"
        );
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

        let result = backend
            .process_transaction(&tenant, entries, helios_fhir::FhirVersion::default())
            .await;

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

        let result = backend
            .process_transaction(&tenant, entries, helios_fhir::FhirVersion::default())
            .await
            .unwrap();

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

    /// #1379: a `code` element's row carries the implicit-system marker, and
    /// `system|code` accepts it. A row written before the marker existed has
    /// no system at all, exactly like a system-less Coding, so it cannot be
    /// told apart and must keep its old behaviour — never over-match — until
    /// the resource is reindexed.
    #[tokio::test]
    async fn test_unmarked_code_rows_keep_their_old_behaviour() {
        use crate::search::IMPLICIT_TOKEN_SYSTEM;

        let backend = create_test_backend();
        let tenant = create_test_tenant();
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "pt-f", "gender": "female"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let ids = |modifier: Option<crate::types::SearchModifier>, value: &str| {
            let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
                name: "gender".to_string(),
                param_type: SearchParamType::Token,
                modifier,
                values: vec![SearchValue::eq(value)],
                chain: vec![],
                components: vec![],
            });
            let backend = &backend;
            let tenant = &tenant;
            async move {
                let found = backend.search(tenant, &query).await.unwrap();
                found
                    .resources
                    .items
                    .iter()
                    .map(|r| r.id().to_string())
                    .collect::<Vec<_>>()
            }
        };
        let qualified = "http://hl7.org/fhir/administrative-gender|female";
        let not = Some(crate::types::SearchModifier::Not);

        // As indexed today.
        let stored: Option<String> = backend
            .get_connection()
            .unwrap()
            .query_row(
                "SELECT value_token_system FROM search_index
                 WHERE resource_id = 'pt-f' AND param_name = 'gender'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored.as_deref(), Some(IMPLICIT_TOKEN_SYSTEM));
        assert_eq!(ids(None, qualified).await, vec!["pt-f"]);
        assert!(ids(not.clone(), qualified).await.is_empty());

        // As indexed before #1379.
        let updated = backend
            .get_connection()
            .unwrap()
            .execute(
                "UPDATE search_index SET value_token_system = NULL
                 WHERE resource_id = 'pt-f' AND param_name = 'gender'",
                [],
            )
            .unwrap();
        assert_eq!(updated, 1);
        assert_eq!(ids(None, "female").await, vec!["pt-f"], "positive control");
        assert_eq!(ids(None, "|female").await, vec!["pt-f"]);
        assert!(ids(None, qualified).await.is_empty());
        assert_eq!(ids(not, qualified).await, vec!["pt-f"]);
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

    /// #967: the FTS delete goes through `resource_fts_map`, so the mapping has
    /// to stay exactly in step with the rows it points at — one mapping per
    /// FTS row, pointing at that row, and gone once the row is.
    #[tokio::test]
    async fn fts_rowid_mapping_tracks_the_rows_it_points_at() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let doc = |text: &str| {
            json!({
                "resourceType": "Patient",
                "id": "map-1",
                "text": {"status": "generated", "div": format!("<div>{text}</div>")}
            })
        };

        let created = backend
            .create(&tenant, "Patient", doc("first"), FhirVersion::default())
            .await
            .unwrap();

        let mapping = || -> Vec<i64> {
            let conn = backend.get_connection().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT fts_rowid FROM resource_fts_map
                     WHERE tenant_id = ?1 AND resource_type = 'Patient' AND resource_id = 'map-1'",
                )
                .unwrap();
            let v: Vec<i64> = stmt
                .query_map([tenant.tenant_id().as_str()], |r| r.get(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect();
            v
        };
        let fts_rows_for = |rowids: &[i64]| -> usize {
            let conn = backend.get_connection().unwrap();
            rowids
                .iter()
                .filter(|rowid| {
                    conn.query_row(
                        "SELECT 1 FROM resource_fts WHERE rowid = ?1",
                        [**rowid],
                        |_| Ok(()),
                    )
                    .is_ok()
                })
                .count()
        };

        let after_create = mapping();
        assert_eq!(after_create.len(), 1, "the create must map exactly one row");
        assert_eq!(fts_rows_for(&after_create), 1, "the mapping must resolve");

        backend
            .update(&tenant, &created, doc("second"))
            .await
            .unwrap();

        let after_update = mapping();
        assert_eq!(
            after_update.len(),
            1,
            "an update must leave exactly one mapping, not accumulate them"
        );
        assert_eq!(fts_rows_for(&after_update), 1);
        // The rowid itself may well be the same integer: FTS5 hands back the
        // one the delete just freed. What has to be true is that the row it
        // names now holds the *new* content — i.e. the delete really removed
        // the old row rather than leaving two, and the mapping points at the
        // survivor.
        let content: String = {
            let conn = backend.get_connection().unwrap();
            conn.query_row(
                "SELECT full_content FROM resource_fts WHERE rowid = ?1",
                [after_update[0]],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert!(
            content.contains("second") && !content.contains("first"),
            "the mapped row must hold the updated content, got: {content}"
        );
        let total_rows: i64 = {
            let conn = backend.get_connection().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM resource_fts WHERE resource_id = 'map-1'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(total_rows, 1, "the update must not leave a second FTS row");

        backend.purge(&tenant, "Patient", "map-1").await.unwrap();
        assert!(
            mapping().is_empty(),
            "a purge must clear the mapping with the rows"
        );
        assert_eq!(fts_rows_for(&after_update), 0);
    }
}
