//! Transaction support for SQLite backend.

use std::ops::Deref;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use helios_fhir::FhirVersion;
use parking_lot::Mutex;
use r2d2::PooledConnection;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use serde_json::Value;

use crate::core::{
    ConditionalTransaction, Transaction, TransactionOptions, TransactionProvider, conditional_query,
};
use crate::error::{
    BackendError, ConcurrencyError, ResourceError, StorageError, StorageResult, TransactionError,
    is_sqlite_busy_or_locked,
};
use crate::perf::{self, Phase};
use crate::tenant::{Operation, TenantContext};
use crate::types::{SearchParameter, StoredResource};

use super::SqliteBackend;
use super::backend::load_tenant_stored_params_with_conn;
use super::storage::PreparedIndex;

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

/// The error for a failed `BEGIN IMMEDIATE` (#1636).
///
/// `SQLITE_BUSY` / `SQLITE_LOCKED` means another connection held the write
/// lock for longer than `busy_timeout`. The database is healthy and merely
/// contended, nothing was written, and an unchanged retry usually succeeds, so
/// it is [`TransactionError::Transient`] (a retryable 503) — the same
/// busy/locked verdict as [`crate::error::classify_sqlite_error`]. `attempts`
/// is 1: BEGIN is not retried here, SQLite's busy handler has already waited.
///
/// Every other BEGIN failure keeps its [`TransactionError::RolledBack`].
fn begin_error(err: &rusqlite::Error) -> StorageError {
    let reason = format!("Failed to begin transaction: {err}");
    StorageError::Transaction(if is_sqlite_busy_or_locked(err) {
        TransactionError::Transient {
            attempts: 1,
            reason,
        }
    } else {
        TransactionError::RolledBack { reason }
    })
}

/// The error for a pool that cannot hand out a connection (#1636).
///
/// r2d2 fails `get()` only after the pool's connection timeout, so the
/// database is out of connections — typically because every one is parked
/// behind the write lock — rather than broken. The same condition surfaces as
/// a 503 on the non-transactional paths (`BackendError::ConnectionFailed`); for
/// a transaction Bundle it is [`TransactionError::Transient`].
fn pool_timeout_error(err: &r2d2::Error) -> StorageError {
    StorageError::Transaction(TransactionError::Transient {
        attempts: 1,
        reason: format!("Failed to begin transaction: {err}"),
    })
}

/// Maps a failed [`TransactionProvider::begin_transaction`] to the error a
/// transaction Bundle ends with (#1636).
///
/// A [`TransactionError::Transient`] from the begin path passes through
/// untouched, so the SQLite error text appears once. It used to be wrapped
/// into `RolledBack` here — and, since the begin path had already wrapped it
/// once, twice: "Failed to begin transaction: transaction rolled back: Failed
/// to begin transaction: database is locked". Every other failure keeps the
/// wrapping it has always had.
pub(super) fn bundle_begin_error(err: StorageError) -> TransactionError {
    match err {
        StorageError::Transaction(transient @ TransactionError::Transient { .. }) => transient,
        other => TransactionError::RolledBack {
            reason: format!("Failed to begin transaction: {other}"),
        },
    }
}

/// Keep rollback responsibility with the pooled connection, including when a
/// cancelled caller leaves extension work running on a blocking thread.
struct TransactionConnection(PooledConnection<SqliteConnectionManager>);

impl Deref for TransactionConnection {
    type Target = rusqlite::Connection;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for TransactionConnection {
    fn drop(&mut self) {
        if !self.is_autocommit() {
            let _ = self.execute("ROLLBACK", []);
        }
    }
}

/// A SQLite transaction.
pub struct SqliteTransaction {
    /// The connection used for this transaction.
    conn: Arc<Mutex<TransactionConnection>>,
    /// Whether the transaction is still active.
    active: bool,
    /// The tenant context for this transaction.
    tenant: TenantContext,
    /// The parent backend: writes in a transaction index through the same
    /// path as direct writes (dynamic extraction with fallback, FTS,
    /// contained resources), so the two can never drift (#815 review).
    backend: SqliteBackend,
    /// Whether a SearchParameter write in this transaction must invalidate
    /// the tenant's cached registry once the commit lands.
    invalidate_registry: bool,
    /// The FHIR version writes in this transaction are stamped with.
    fhir_version: FhirVersion,
    /// Bulk fast-load (#903): skip search-index and FTS writes; stale rows of
    /// updated resources are still deleted so a deferred index never lies.
    defer_search_indexing: bool,
}

impl std::fmt::Debug for SqliteTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteTransaction")
            .field("active", &self.active)
            .field("tenant", &self.tenant)
            .finish()
    }
}

impl SqliteTransaction {
    /// Read the current revision, including its deletion timestamp when it is
    /// a tombstone. Ordinary transaction reads continue to hide deleted rows.
    pub async fn read_including_deleted(
        &mut self,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        self.read_resource(resource_type, id, true).await
    }

    /// Restore a tombstone as its next revision in this transaction. The
    /// supplied tombstone must belong to this tenant and still match the
    /// stored deletion state and version. History and search indexes are
    /// updated with the resource; callers must roll back on any write error.
    pub async fn restore(
        &mut self,
        deleted: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        if !deleted.is_deleted() || deleted.tenant_id() != self.tenant.tenant_id() {
            return Err(StorageError::Validation(
                crate::error::ValidationError::InvalidResource {
                    message: "Restore requires a deleted resource from the transaction's tenant"
                        .into(),
                    details: Vec::new(),
                },
            ));
        }
        self.write_revision(deleted, resource, true).await
    }

    async fn read_resource(
        &mut self,
        resource_type: &str,
        id: &str,
        include_deleted: bool,
    ) -> StorageResult<Option<StoredResource>> {
        if !self.active {
            return Err(StorageError::Transaction(
                TransactionError::InvalidTransaction,
            ));
        }

        let conn = self.conn.lock();
        let tenant_id = self.tenant.tenant_id().as_str();

        let result = conn.query_row(
            "SELECT version_id, data, last_updated, is_deleted, fhir_version, deleted_at
             FROM resources
             WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3",
            params![tenant_id, resource_type, id],
            |row| {
                let version_id: String = row.get(0)?;
                let data: Vec<u8> = row.get(1)?;
                let last_updated: String = row.get(2)?;
                let is_deleted: i32 = row.get(3)?;
                let fhir_version: String = row.get(4)?;
                let deleted_at: Option<String> = row.get(5)?;
                Ok((
                    version_id,
                    data,
                    last_updated,
                    is_deleted,
                    fhir_version,
                    deleted_at,
                ))
            },
        );

        match result {
            Ok((version_id, data, last_updated, is_deleted, fhir_version_str, deleted_at)) => {
                if is_deleted != 0 && !include_deleted {
                    return Ok(None);
                }

                let json_data: serde_json::Value = serde_json::from_slice(&data).map_err(|e| {
                    serialization_error(format!("Failed to deserialize resource: {}", e))
                })?;

                let last_updated = chrono::DateTime::parse_from_rfc3339(&last_updated)
                    .map_err(|e| internal_error(format!("Failed to parse last_updated: {}", e)))?
                    .with_timezone(&Utc);

                // Older tombstones may lack deleted_at. Preserve their deleted
                // state using the timestamp of the deletion revision.
                let deleted_at = if is_deleted != 0 {
                    Some(
                        deleted_at
                            .map(|value| {
                                chrono::DateTime::parse_from_rfc3339(&value)
                                    .map(|value| value.with_timezone(&Utc))
                                    .map_err(|error| {
                                        internal_error(format!(
                                            "Failed to parse deleted_at: {error}"
                                        ))
                                    })
                            })
                            .transpose()?
                            .unwrap_or(last_updated),
                    )
                } else {
                    None
                };

                let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                    .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

                Ok(Some(StoredResource::from_storage(
                    resource_type,
                    id,
                    version_id,
                    self.tenant.tenant_id().clone(),
                    json_data,
                    last_updated,
                    last_updated,
                    deleted_at,
                    fhir_version,
                )))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(internal_error(format!("Failed to read resource: {}", e))),
        }
    }

    async fn write_revision(
        &mut self,
        current: &StoredResource,
        resource: Value,
        expected_deleted: bool,
    ) -> StorageResult<StoredResource> {
        let _update_span = perf::span(Phase::Update);
        self.tenant
            .check_permission(Operation::Update, current.resource_type())?;

        if !self.active {
            return Err(StorageError::Transaction(
                TransactionError::InvalidTransaction,
            ));
        }

        let conn = self.conn.lock();
        let tenant_id = self.tenant.tenant_id().as_str();
        let resource_type = current.resource_type();
        let id = current.id();

        // Verify current version still matches (optimistic locking)
        let db_version: Result<String, _> = conn.query_row(
            "SELECT version_id FROM resources
             WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3 AND is_deleted = ?4",
            params![tenant_id, resource_type, id, expected_deleted],
            |row| row.get(0),
        );

        let db_version = match db_version {
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

        if db_version != current.version_id() {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: current.version_id().to_string(),
                    actual_version: db_version,
                },
            ));
        }

        // Calculate new version
        let new_version: u64 = db_version.parse().unwrap_or(0) + 1;
        let new_version_str = new_version.to_string();

        // Build the resource with id and resourceType
        let mut data = resource;
        if let Some(obj) = data.as_object_mut() {
            obj.insert("id".to_string(), Value::String(id.to_string()));
            obj.insert(
                "resourceType".to_string(),
                Value::String(resource_type.to_string()),
            );
        }

        // Serialize the resource data
        let data_bytes = {
            let _span = perf::span(Phase::Serialize);
            serde_json::to_vec(&data)
                .map_err(|e| serialization_error(format!("Failed to serialize resource: {}", e)))?
        };

        let now = Utc::now();
        let last_updated = now.to_rfc3339();

        // Preserve FHIR version from the current resource
        let fhir_version = current.fhir_version();
        let fhir_version_str = fhir_version.as_mime_param();

        // Update the resource
        let _update_row_span = perf::span(Phase::ResourceUpdate);
        let updated = conn
            .execute(
                "UPDATE resources SET version_id = ?1, data = ?2, last_updated = ?3,
                 is_deleted = 0, deleted_at = NULL
             WHERE tenant_id = ?4 AND resource_type = ?5 AND id = ?6
               AND version_id = ?7 AND is_deleted = ?8",
                params![
                    new_version_str,
                    data_bytes,
                    last_updated,
                    tenant_id,
                    resource_type,
                    id,
                    current.version_id(),
                    expected_deleted
                ],
            )
            .map_err(|e| internal_error(format!("Failed to update resource: {}", e)))?;
        if updated != 1 {
            return Err(internal_error(
                "Resource changed while writing its revision".into(),
            ));
        }
        drop(_update_row_span);

        // Insert into history
        {
            let _span = perf::span(Phase::HistoryInsert);
            conn.execute(
                "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, \
                 data, last_updated, is_deleted, fhir_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
                params![
                    tenant_id,
                    resource_type,
                    id,
                    new_version_str,
                    data_bytes,
                    last_updated,
                    fhir_version_str,
                ],
            )
            .map_err(|e| internal_error(format!("Failed to insert history: {}", e)))?;
        }

        // Re-index the resource for search, clearing the previous version's rows.
        self.index_resource(&conn, tenant_id, resource_type, id, &data, true, None)?;
        drop(conn);

        // A SearchParameter write invalidates this tenant's cached registry
        // (after commit), mirroring the storage path.
        if resource_type == "SearchParameter" {
            self.invalidate_registry = true;
        }

        Ok(StoredResource::from_storage(
            resource_type,
            id,
            new_version_str,
            self.tenant.tenant_id().clone(),
            data,
            now,
            now,
            None,
            fhir_version,
        ))
    }

    /// Create a new transaction.
    fn new(
        conn: PooledConnection<SqliteConnectionManager>,
        tenant: TenantContext,
        backend: SqliteBackend,
        fhir_version: FhirVersion,
        defer_search_indexing: bool,
    ) -> StorageResult<Self> {
        // Start the transaction
        conn.execute("BEGIN IMMEDIATE", [])
            .map_err(|e| begin_error(&e))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(TransactionConnection(conn))),
            active: true,
            tenant,
            backend,
            invalidate_registry: false,
            fhir_version,
            defer_search_indexing,
        })
    }

    /// Runs statements on this transaction's connection, so callers can make
    /// their own rows atomic with the resource writes (e.g. bulk-submit's
    /// rollback records and per-line receipts, #815 review).
    pub(crate) fn with_connection<T>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> StorageResult<T>,
    ) -> StorageResult<T> {
        if !self.active {
            return Err(StorageError::Transaction(
                TransactionError::InvalidTransaction,
            ));
        }
        let conn = self.conn.lock();
        f(&conn)
    }

    /// Run extension-table work on the active resource transaction, off the
    /// async runtime. The callback must not begin, commit or roll back a
    /// transaction. Its connection cannot escape the callback.
    ///
    /// Callers must roll back when extension work fails. If the awaiting task
    /// is cancelled, connection ownership keeps queued work isolated from the
    /// pool; work that starts after rollback is refused.
    pub async fn run_on_connection<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&rusqlite::Connection) -> StorageResult<T> + Send + 'static,
    ) -> StorageResult<T> {
        if !self.active {
            return Err(StorageError::Transaction(
                TransactionError::InvalidTransaction,
            ));
        }
        let connection = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let connection = connection.lock();
            if connection.is_autocommit() {
                return Err(StorageError::Transaction(
                    TransactionError::InvalidTransaction,
                ));
            }
            operation(&connection)
        })
        .await
        .map_err(|error| {
            StorageError::Transaction(TransactionError::RolledBack {
                reason: format!("Transaction extension task failed: {error}"),
            })
        })?
    }

    /// Generate a new ID for a resource.
    fn generate_id() -> String {
        crate::types::new_resource_id()
    }

    /// Index a resource for search — through the backend's own path (dynamic
    /// extraction with minimal fallback, FTS content, contained resources),
    /// so transactional writes are searchable exactly like direct ones
    /// (#815 review).
    #[allow(clippy::too_many_arguments)]
    fn index_resource(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
        clear_stale_rows: bool,
        prepared: Option<PreparedIndex>,
    ) -> StorageResult<()> {
        let _index_span = perf::span(Phase::Index);
        // `clear_stale_rows` is false only on the create path, where the
        // caller has just proved no `resources` row exists for this id. No
        // resource row means no index rows: every path that removes a
        // `resources` row — soft delete keeps the row, `purge`, `purge_all`
        // and `purge_tenant_data` delete `search_index` in the same
        // transaction, and the table's foreign key cascades besides — so the
        // DELETE could only ever match nothing. It is not free: it is an
        // index seek per ingested resource, plus (before #949) a full scan of
        // the FTS table.
        if clear_stale_rows {
            let _span = perf::span(Phase::IndexDelete);
            self.backend
                .delete_search_index(conn, tenant_id, resource_type, resource_id)?;
        }
        // Fast-load (#903): the delete above still runs so an updated
        // resource's stale rows are gone, but the rebuild is deferred to the
        // post-ingest reindex — extraction plus FTS is the dominant per-byte
        // cost of bulk ingestion.
        if self.defer_search_indexing {
            return Ok(());
        }
        // A batch caller may have run the extraction already, off this
        // thread (`SqliteBackend::prepare_index_batch`); then only the
        // statements are left.
        match prepared {
            Some(prepared) if !self.backend.is_search_offloaded() => self
                .backend
                .write_prepared_index(
                    conn,
                    tenant_id,
                    resource_type,
                    resource_id,
                    resource,
                    prepared,
                )
                .map(|_| ()),
            _ => self
                .backend
                .index_resource(conn, tenant_id, resource_type, resource_id, resource),
        }
    }

    /// [`Transaction::create`] with the search-index extraction already done
    /// by the caller. Bulk ingest prepares a whole batch in parallel before
    /// opening the entry loop and hands each entry's result in here; the
    /// caller must have extracted from a resource carrying the same `id` and
    /// `resourceType` this create will store, or the index rows would
    /// describe a different resource. Unlike the trait method this is not
    /// reached through `dyn Transaction`, so it is inherent.
    pub(crate) async fn create_prepared(
        &mut self,
        resource_type: &str,
        resource: Value,
        prepared: Option<PreparedIndex>,
    ) -> StorageResult<StoredResource> {
        self.create_inner(resource_type, resource, prepared)
    }
}

impl SqliteTransaction {
    fn create_inner(
        &mut self,
        resource_type: &str,
        resource: Value,
        prepared: Option<PreparedIndex>,
    ) -> StorageResult<StoredResource> {
        let _create_span = perf::span(Phase::Create);
        self.tenant
            .check_permission(Operation::Create, resource_type)?;

        if !self.active {
            return Err(StorageError::Transaction(
                TransactionError::InvalidTransaction,
            ));
        }

        let conn = self.conn.lock();
        let tenant_id = self.tenant.tenant_id().as_str();

        // Get or generate ID
        let id = resource
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(Self::generate_id);

        // Check if resource already exists
        let exists: bool = {
            let _span = perf::span(Phase::CreateExists);
            conn.prepare_cached(
                "SELECT 1 FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3",
            )
            .and_then(|mut stmt| {
                stmt.query_row(params![tenant_id, resource_type, id], |_| Ok(true))
            })
            .unwrap_or(false)
        };

        if exists {
            return Err(StorageError::Resource(ResourceError::AlreadyExists {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        }

        // Build the resource with id and resourceType
        let mut data = {
            let _span = perf::span(Phase::EntryClone);
            resource.clone()
        };
        if let Some(obj) = data.as_object_mut() {
            obj.insert("id".to_string(), Value::String(id.clone()));
            obj.insert(
                "resourceType".to_string(),
                Value::String(resource_type.to_string()),
            );
        }

        // Serialize the resource data
        let data_bytes = {
            let _span = perf::span(Phase::Serialize);
            serde_json::to_vec(&data)
                .map_err(|e| serialization_error(format!("Failed to serialize resource: {}", e)))?
        };

        let now = Utc::now();
        let last_updated = now.to_rfc3339();
        let version_id = "1";

        let fhir_version_str = self.fhir_version.as_mime_param();

        // Insert the resource
        {
            let _span = perf::span(Phase::ResourceInsert);
            conn.prepare_cached(
                "INSERT INTO resources (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
            )
            .map_err(|e| internal_error(format!("Failed to prepare resource insert: {}", e)))?
            .execute(params![tenant_id, resource_type, id, version_id, data_bytes, last_updated, fhir_version_str])
            .map_err(|e| internal_error(format!("Failed to insert resource: {}", e)))?;
        }

        // Insert into history
        {
            let _span = perf::span(Phase::HistoryInsert);
            conn.prepare_cached(
                "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
            )
            .map_err(|e| internal_error(format!("Failed to prepare history insert: {}", e)))?
            .execute(params![tenant_id, resource_type, id, version_id, data_bytes, last_updated, fhir_version_str])
            .map_err(|e| internal_error(format!("Failed to insert history: {}", e)))?;
        }

        // Index the resource for search. Nothing to clear: the existence
        // probe above established there is no row for this id.
        self.index_resource(&conn, tenant_id, resource_type, &id, &data, false, prepared)?;
        drop(conn);

        // Mirrors the storage path: an overlay-affecting SearchParameter
        // write invalidates the tenant's cached registry — once the commit
        // makes it readable.
        if resource_type == "SearchParameter"
            && self
                .backend
                .tenant_registries()
                .create_affects_overlay(&data)
        {
            self.invalidate_registry = true;
        }

        Ok(StoredResource::from_storage(
            resource_type,
            &id,
            version_id,
            self.tenant.tenant_id().clone(),
            data,
            now,
            now,
            None,
            self.fhir_version,
        ))
    }
}

#[async_trait]
impl Transaction for SqliteTransaction {
    async fn create(
        &mut self,
        resource_type: &str,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        self.create_inner(resource_type, resource, None)
    }

    async fn read(
        &mut self,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        self.read_resource(resource_type, id, false).await
    }

    async fn update(
        &mut self,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        self.write_revision(current, resource, false).await
    }

    async fn delete(&mut self, resource_type: &str, id: &str) -> StorageResult<()> {
        self.tenant
            .check_permission(Operation::Delete, resource_type)?;

        if !self.active {
            return Err(StorageError::Transaction(
                TransactionError::InvalidTransaction,
            ));
        }

        let conn = self.conn.lock();
        let tenant_id = self.tenant.tenant_id().as_str();

        // Check if resource exists
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
        let new_version: u64 = current_version.parse().unwrap_or(0) + 1;
        let new_version_str = new_version.to_string();

        // Soft delete the resource
        conn.execute(
            "UPDATE resources SET is_deleted = 1, deleted_at = ?1, version_id = ?2, last_updated = ?1
             WHERE tenant_id = ?3 AND resource_type = ?4 AND id = ?5",
            params![deleted_at, new_version_str, tenant_id, resource_type, id],
        )
        .map_err(|e| internal_error(format!("Failed to delete resource: {}", e)))?;

        // Insert deletion record into history
        conn.execute(
            "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
            params![tenant_id, resource_type, id, new_version_str, data, deleted_at, fhir_version_str],
        )
        .map_err(|e| internal_error(format!("Failed to insert deletion history: {}", e)))?;
        drop(conn);

        // A SearchParameter delete invalidates this tenant's cached registry
        // (after commit), mirroring the storage path.
        if resource_type == "SearchParameter" {
            self.invalidate_registry = true;
        }

        Ok(())
    }

    async fn commit(mut self: Box<Self>) -> StorageResult<()> {
        if !self.active {
            return Err(StorageError::Transaction(
                TransactionError::InvalidTransaction,
            ));
        }

        let conn = self.conn.lock();
        {
            let _span = perf::span(Phase::Commit);
            conn.execute("COMMIT", []).map_err(|e| {
                StorageError::Transaction(TransactionError::RolledBack {
                    reason: format!("Commit failed: {}", e),
                })
            })?;
        }
        drop(conn);

        self.active = false;

        // Only a landed commit makes the SearchParameter rows readable, so
        // the registry invalidation waits until here.
        if self.invalidate_registry {
            self.backend
                .tenant_registries()
                .invalidate(self.tenant.tenant_id().as_str());
        }
        Ok(())
    }

    async fn rollback(mut self: Box<Self>) -> StorageResult<()> {
        if !self.active {
            return Err(StorageError::Transaction(
                TransactionError::InvalidTransaction,
            ));
        }

        let conn = self.conn.lock();
        conn.execute("ROLLBACK", []).map_err(|e| {
            StorageError::Transaction(TransactionError::RolledBack {
                reason: format!("Rollback failed: {}", e),
            })
        })?;

        self.active = false;
        Ok(())
    }

    fn tenant(&self) -> &TenantContext {
        &self.tenant
    }

    fn is_active(&self) -> bool {
        self.active
    }
}

impl Drop for SqliteTransaction {
    fn drop(&mut self) {
        if self.active {
            // Never wait on a blocking callback from the async worker. If it
            // owns the mutex, its Arc retains the connection and the final
            // TransactionConnection drop rolls back before returning it to
            // the pool. No runtime is needed to schedule that cleanup.
            if let Some(conn) = self.conn.try_lock() {
                let _ = conn.execute("ROLLBACK", []);
            }
        }
    }
}

/// The transaction-scoped search surface (#859).
///
/// Runs the backend's own search body on this transaction's connection, so
/// the match set includes what earlier entries of the same bundle wrote; the
/// pooled-connection twin on the backend cannot see those rows under
/// `BEGIN IMMEDIATE` (#511).
#[async_trait]
impl ConditionalTransaction for SqliteTransaction {
    async fn find_matching(
        &mut self,
        resource_type: &str,
        criteria: &[SearchParameter],
    ) -> StorageResult<Vec<StoredResource>> {
        if criteria.is_empty() {
            return Ok(Vec::new());
        }
        let query = conditional_query(resource_type, criteria);
        let backend = self.backend.clone();
        let tenant = self.tenant.clone();
        self.with_connection(|conn| backend.search_with_connection(conn, &tenant, &query, None))
            .map(|result| result.resources.items)
    }
}

#[async_trait]
impl TransactionProvider for SqliteBackend {
    type Transaction = SqliteTransaction;

    async fn begin_transaction(
        &self,
        tenant: &TenantContext,
        options: TransactionOptions,
    ) -> StorageResult<Self::Transaction> {
        // Not `get_connection()`: that maps the pool timeout to
        // `ConnectionFailed`, and a transaction must report it as `Transient`.
        let conn = self.pool().get().map_err(|e| pool_timeout_error(&e))?;
        let tenant_id = tenant.tenant_id().as_str();
        let fhir_version = options.fhir_version.unwrap_or(self.config().fhir_version);

        // Warm this tenant's search-parameter registry from THIS connection
        // on a cache miss, rather than letting the backend's indexing path
        // ask the pool for a second, simultaneous one (#787) — see
        // `load_tenant_stored_params_with_conn`. Writes in this transaction
        // index through `SqliteBackend::index_resource`, whose extractor
        // reads the registry cache populated here; a cached registry skips
        // the query entirely, same as the non-transactional path.
        if self.tenant_registries().cached(tenant_id).is_none() {
            let stored = load_tenant_stored_params_with_conn(&conn, fhir_version, tenant_id)
                .map_err(|e| {
                    StorageError::Backend(BackendError::Internal {
                        backend_name: "sqlite".to_string(),
                        message: format!(
                            "Failed to load this tenant's SearchParameter overlay; \
                             refusing to start a transaction that would index \
                             resources against an incomplete registry: {e}"
                        ),
                        source: None,
                    })
                })?;
            self.tenant_registries().build_and_cache(tenant_id, stored);
        }

        SqliteTransaction::new(
            conn,
            tenant.clone(),
            self.clone(),
            fhir_version,
            options.defer_search_indexing,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ResourceStorage;
    use crate::tenant::{TenantId, TenantPermissions};
    use serde_json::json;

    fn create_test_backend() -> SqliteBackend {
        let backend = SqliteBackend::in_memory().unwrap();
        backend.init_schema().unwrap();
        backend
    }

    fn create_test_tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        )
    }

    fn assert_restore_rows(tx: &SqliteTransaction, versions: &[(&str, bool)], tag: Option<&str>) {
        tx.with_connection(|conn| {
            let mut statement = conn
                .prepare(
                    "SELECT version_id, is_deleted FROM resource_history
                 WHERE tenant_id = ?1 AND resource_type = 'Patient' AND id = 'restored'
                 ORDER BY CAST(version_id AS INTEGER)",
                )
                .unwrap();
            let actual: Vec<(String, bool)> = statement
                .query_map([tx.tenant.tenant_id().as_str()], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(
                actual,
                versions
                    .iter()
                    .map(|(v, deleted)| (v.to_string(), *deleted))
                    .collect::<Vec<_>>()
            );
            let mut statement = conn
                .prepare(
                    "SELECT value_token_code FROM search_index
                 WHERE tenant_id = ?1 AND resource_type = 'Patient' AND resource_id = 'restored'
                   AND param_name = '_tag'",
                )
                .unwrap();
            let tags: Vec<String> = statement
                .query_map([tx.tenant.tenant_id().as_str()], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(
                tags,
                tag.into_iter().map(str::to_string).collect::<Vec<_>>()
            );
            Ok(())
        })
        .unwrap();
    }

    #[tokio::test]
    async fn restore_handles_a_legacy_tombstone_without_deleted_at() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let mut tx = backend
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .unwrap();
        let body = json!({"resourceType":"Patient", "id":"legacy-tombstone"});
        tx.create("Patient", body.clone()).await.unwrap();
        tx.delete("Patient", "legacy-tombstone").await.unwrap();
        tx.with_connection(|conn| {
            conn.execute(
                "UPDATE resources SET deleted_at = NULL WHERE tenant_id = ?1 \
                 AND id = 'legacy-tombstone'",
                [tenant.tenant_id().as_str()],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
        assert!(
            tx.read("Patient", "legacy-tombstone")
                .await
                .unwrap()
                .is_none()
        );
        let deleted = tx
            .read_including_deleted("Patient", "legacy-tombstone")
            .await
            .unwrap()
            .unwrap();
        assert!(deleted.is_deleted());
        assert_eq!(deleted.deleted_at(), Some(deleted.last_modified()));
        let restored = tx.restore(&deleted, body).await.unwrap();
        assert_eq!(restored.version_id(), "3");
        assert!(!restored.is_deleted());
        Box::new(tx).rollback().await.unwrap();
    }

    #[tokio::test]
    async fn restore_preserves_history_and_indexes_across_commit_and_rollback() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let body = |tag| {
            json!({"resourceType":"Patient", "id":"restored",
            "meta":{"tag":[{"system":"http://example.com/tags", "code":tag}]}})
        };
        let mut tx = backend
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .unwrap();
        let created = tx.create("Patient", body("original")).await.unwrap();
        assert_eq!(
            tx.read_including_deleted("Patient", "restored")
                .await
                .unwrap()
                .unwrap()
                .version_id(),
            "1"
        );
        assert!(matches!(
            tx.restore(&created, body("invalid")).await,
            Err(StorageError::Validation(_))
        ));
        tx.delete("Patient", "restored").await.unwrap();
        assert!(tx.read("Patient", "restored").await.unwrap().is_none());
        let deleted = tx
            .read_including_deleted("Patient", "restored")
            .await
            .unwrap()
            .unwrap();
        assert!(deleted.is_deleted());
        assert!(deleted.deleted_at().is_some());
        assert_eq!(deleted.version_id(), "2");
        // Transactional delete retains index rows; searches exclude them using
        // the resource's deleted state. Restore must replace those old rows.
        assert_restore_rows(&tx, &[("1", false), ("2", true)], Some("original"));
        Box::new(tx).commit().await.unwrap();

        let mut tx = backend
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .unwrap();
        let restored = tx.restore(&deleted, body("rolled-back")).await.unwrap();
        assert_eq!(restored.version_id(), "3");
        assert!(!restored.is_deleted());
        assert_eq!(restored.fhir_version(), created.fhir_version());
        assert_eq!(
            tx.read("Patient", "restored")
                .await
                .unwrap()
                .unwrap()
                .version_id(),
            "3"
        );
        assert_restore_rows(
            &tx,
            &[("1", false), ("2", true), ("3", false)],
            Some("rolled-back"),
        );
        Box::new(tx).rollback().await.unwrap();

        let mut tx = backend
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .unwrap();
        assert!(tx.read("Patient", "restored").await.unwrap().is_none());
        assert_eq!(
            tx.read_including_deleted("Patient", "restored")
                .await
                .unwrap()
                .unwrap()
                .version_id(),
            "2"
        );
        assert_restore_rows(&tx, &[("1", false), ("2", true)], Some("original"));
        tx.restore(&deleted, body("committed")).await.unwrap();
        Box::new(tx).commit().await.unwrap();

        let mut tx = backend
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .unwrap();
        let read = tx.read("Patient", "restored").await.unwrap().unwrap();
        assert_eq!(read.version_id(), "3");
        assert_eq!(read.fhir_version(), created.fhir_version());
        assert_eq!(read.content(), &body("committed"));
        assert_restore_rows(
            &tx,
            &[("1", false), ("2", true), ("3", false)],
            Some("committed"),
        );
        assert!(matches!(
            tx.restore(&deleted, body("must-not-overwrite-live")).await,
            Err(StorageError::Resource(ResourceError::NotFound { .. }))
        ));
        Box::new(tx).rollback().await.unwrap();
        let mut tx = backend
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .unwrap();
        assert_eq!(
            tx.read("Patient", "restored")
                .await
                .unwrap()
                .unwrap()
                .version_id(),
            "3"
        );
        tx.delete("Patient", "restored").await.unwrap();
        Box::new(tx).commit().await.unwrap();

        // The same old tombstone must not overwrite a later delete revision.
        let mut tx = backend
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .unwrap();
        assert!(matches!(
            tx.restore(&deleted, body("stale")).await,
            Err(StorageError::Concurrency(ConcurrencyError::VersionConflict {
                expected_version,
                actual_version,
                ..
            })) if expected_version == "2" && actual_version == "4"
        ));
        Box::new(tx).rollback().await.unwrap();
        let mut tx = backend
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .unwrap();
        assert_eq!(
            tx.read_including_deleted("Patient", "restored")
                .await
                .unwrap()
                .unwrap()
                .version_id(),
            "4"
        );
        assert_restore_rows(
            &tx,
            &[("1", false), ("2", true), ("3", false), ("4", true)],
            Some("committed"),
        );
        Box::new(tx).rollback().await.unwrap();
    }

    #[tokio::test]
    async fn restore_rejects_a_tombstone_from_another_tenant() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let mut tx = backend
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .unwrap();
        tx.create("Patient", json!({"id":"restored"}))
            .await
            .unwrap();
        tx.delete("Patient", "restored").await.unwrap();
        let deleted = tx
            .read_including_deleted("Patient", "restored")
            .await
            .unwrap()
            .unwrap();
        Box::new(tx).commit().await.unwrap();
        let other = TenantContext::new(
            TenantId::new("restore-other-tenant"),
            TenantPermissions::full_access(),
        );
        let mut tx = backend
            .begin_transaction(&other, TransactionOptions::default())
            .await
            .unwrap();
        assert!(matches!(
            tx.restore(&deleted, json!({"resourceType":"Patient"}))
                .await,
            Err(StorageError::Validation(_))
        ));
        assert!(
            tx.read_including_deleted("Patient", "restored")
                .await
                .unwrap()
                .is_none()
        );
        Box::new(tx).rollback().await.unwrap();
    }

    #[tokio::test]
    async fn test_transaction_commit() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Start transaction
        let mut tx = backend
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .unwrap();

        // Create resource in transaction
        let resource = json!({
            "resourceType": "Patient",
            "id": "patient-1",
            "name": [{"family": "Test"}]
        });
        tx.create("Patient", resource).await.unwrap();

        // Commit
        Box::new(tx).commit().await.unwrap();

        // Verify resource exists
        let result = backend.read(&tenant, "Patient", "patient-1").await.unwrap();
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn test_transaction_rollback() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Start transaction
        let mut tx = backend
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .unwrap();

        // Create resource in transaction
        let resource = json!({
            "resourceType": "Patient",
            "id": "patient-1"
        });
        tx.create("Patient", resource).await.unwrap();

        // Rollback
        Box::new(tx).rollback().await.unwrap();

        // Verify resource does NOT exist
        let result = backend.read(&tenant, "Patient", "patient-1").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_transaction_read_own_writes() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Start transaction
        let mut tx = backend
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .unwrap();

        // Create resource
        let resource = json!({
            "resourceType": "Patient",
            "id": "patient-1"
        });
        tx.create("Patient", resource).await.unwrap();

        // Read within same transaction
        let read = tx.read("Patient", "patient-1").await.unwrap();
        assert!(read.is_some());

        Box::new(tx).rollback().await.unwrap();
    }

    #[tokio::test]
    async fn test_transaction_update() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create initial resource
        let resource = json!({
            "resourceType": "Patient",
            "name": [{"family": "Original"}]
        });
        let created = backend
            .create(&tenant, "Patient", resource, FhirVersion::default())
            .await
            .unwrap();

        // Start transaction and update
        let mut tx = backend
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .unwrap();

        let updated_data = json!({
            "resourceType": "Patient",
            "name": [{"family": "Updated"}]
        });
        let result = tx.update(&created, updated_data).await.unwrap();
        assert_eq!(result.version_id(), "2");

        Box::new(tx).commit().await.unwrap();

        // Verify update persisted
        let read = backend
            .read(&tenant, "Patient", created.id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.version_id(), "2");
    }

    /// #947: a transactional `create` no longer runs the "clear stale index
    /// rows" DELETE, because the existence probe it just ran proves there is
    /// nothing to clear. The two properties that has to preserve: a create
    /// writes exactly its own index rows, and an `update` — which does still
    /// clear — leaves none of the previous version's behind.
    ///
    /// The tag is the observed value because an in-memory backend carries only
    /// the embedded search parameters (`_id`, `_lastUpdated`, `_tag`,
    /// `_profile`, `_security`).
    #[tokio::test]
    async fn create_writes_only_its_own_index_rows_and_update_still_clears_stale_ones() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let tagged = |tag: &str| {
            json!({
                "resourceType": "Patient",
                "id": "p1",
                "meta": {"tag": [{"system": "http://example.com/tags", "code": tag}]}
            })
        };
        let rows_with_tag = |tag: &str| -> i64 {
            let conn = backend.get_connection().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM search_index
                 WHERE resource_type = 'Patient' AND resource_id = 'p1'
                   AND param_name = '_tag' AND value_token_code = ?1",
                params![tag],
                |row| row.get(0),
            )
            .unwrap()
        };
        let total_rows = || -> i64 {
            let conn = backend.get_connection().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM search_index
                 WHERE resource_type = 'Patient' AND resource_id = 'p1'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };

        let mut tx = backend
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .unwrap();
        tx.create("Patient", tagged("before")).await.unwrap();
        Box::new(tx).commit().await.unwrap();

        let after_create = total_rows();
        assert!(after_create > 0, "the create must index the resource");
        assert_eq!(rows_with_tag("before"), 1, "the tag must be indexed");

        let current = backend
            .read(&tenant, "Patient", "p1")
            .await
            .unwrap()
            .unwrap();
        let mut tx = backend
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .unwrap();
        tx.update(&current, tagged("after")).await.unwrap();
        Box::new(tx).commit().await.unwrap();

        assert_eq!(
            rows_with_tag("before"),
            0,
            "the update must clear the previous version's index rows"
        );
        assert_eq!(rows_with_tag("after"), 1, "the new tag must be indexed");
        assert_eq!(
            total_rows(),
            after_create,
            "an update must not accumulate rows on top of the create's"
        );
    }

    #[tokio::test]
    async fn test_transaction_delete() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create initial resource
        let resource = json!({
            "resourceType": "Patient",
            "id": "patient-1"
        });
        backend
            .create(&tenant, "Patient", resource, FhirVersion::default())
            .await
            .unwrap();

        // Start transaction and delete
        let mut tx = backend
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .unwrap();

        tx.delete("Patient", "patient-1").await.unwrap();
        Box::new(tx).commit().await.unwrap();

        // Verify deleted (returns Gone error)
        let result = backend.read(&tenant, "Patient", "patient-1").await;
        assert!(matches!(
            result,
            Err(StorageError::Resource(ResourceError::Gone { .. }))
        ));
    }

    #[tokio::test]
    async fn test_transaction_auto_rollback_on_drop() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        {
            // Start transaction
            let mut tx = backend
                .begin_transaction(&tenant, TransactionOptions::default())
                .await
                .unwrap();

            // Create resource
            let resource = json!({
                "resourceType": "Patient",
                "id": "patient-1"
            });
            tx.create("Patient", resource).await.unwrap();

            // Drop without commit or rollback
        }

        // Verify resource does NOT exist (auto-rollback)
        let result = backend.read(&tenant, "Patient", "patient-1").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelled_extension_work_does_not_block_the_runtime_and_rolls_back() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let mut transaction = backend
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .unwrap();
        transaction
            .create(
                "Patient",
                json!({"resourceType":"Patient","id":"extension-cancel"}),
            )
            .await
            .unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let task = tokio::spawn(async move {
            transaction
                .run_on_connection(move |connection| {
                    connection
                        .execute_batch("CREATE TABLE extension_cancel_proof(value INTEGER)")
                        .unwrap();
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    connection
                        .execute("INSERT INTO extension_cancel_proof VALUES (1)", [])
                        .unwrap();
                    Ok(())
                })
                .await
                .unwrap();
            Box::new(transaction).commit().await.unwrap();
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), started_rx)
            .await
            .unwrap()
            .unwrap();

        // The external watchdog releases the callback even if the old blocking
        // Drop stalls this single-thread runtime. A passing test must signal
        // runtime progress while the callback is still held, before release.
        let (progress_tx, progress_rx) = std::sync::mpsc::channel();
        let watchdog = std::thread::spawn(move || {
            let progressed = progress_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .is_ok();
            release_tx.send(()).unwrap();
            progressed
        });
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            let _ = progress_tx.send(());
        })
        .await
        .unwrap();
        assert!(
            watchdog.join().unwrap(),
            "cancellation blocked unrelated runtime work"
        );

        // Acquiring a new write transaction also proves that deferred cleanup
        // released the old write lock, not merely that another reader cannot
        // see the uncommitted changes. The released callback still has to
        // finish and roll back on its blocking thread, so retry until it has.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut proof = loop {
            match backend
                .begin_transaction(&tenant, TransactionOptions::default())
                .await
            {
                Ok(proof) => break proof,
                Err(StorageError::Transaction(TransactionError::Transient { .. }))
                    if tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                Err(error) => panic!("could not start verification transaction: {error}"),
            }
        };
        assert!(
            proof
                .read("Patient", "extension-cancel")
                .await
                .unwrap()
                .is_none()
        );
        let tables: i64 = proof
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_master \
                         WHERE name = 'extension_cancel_proof'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(|error| internal_error(error.to_string()))
            })
            .unwrap();
        assert_eq!(
            tables, 0,
            "extension DDL and resource must roll back together"
        );
        Box::new(proof).rollback().await.unwrap();
    }

    #[test]
    fn dropped_transaction_rolls_back_without_a_runtime() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let transaction = runtime.block_on(async {
            let mut transaction = backend
                .begin_transaction(&tenant, TransactionOptions::default())
                .await
                .unwrap();
            transaction
                .create(
                    "Patient",
                    json!({"resourceType":"Patient","id":"no-runtime"}),
                )
                .await
                .unwrap();
            transaction
        });
        drop(runtime);
        drop(transaction);
        let connection = backend.get_connection().unwrap();
        assert!(connection.is_autocommit());
        let resources: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM resources WHERE id = 'no-runtime'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(resources, 0);
    }

    #[tokio::test]
    async fn test_transaction_is_active() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let tx = backend
            .begin_transaction(&tenant, TransactionOptions::default())
            .await
            .unwrap();

        assert!(tx.is_active());

        Box::new(tx).commit().await.unwrap();
        // After commit, we can't check is_active since tx is consumed
    }

    fn sqlite_failure(code: std::os::raw::c_int, message: &str) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), Some(message.to_string()))
    }

    /// #1636: only `SQLITE_BUSY` / `SQLITE_LOCKED` (extended codes included)
    /// make a failed BEGIN transient; the reason keeps the driver text once.
    #[test]
    fn begin_error_is_transient_only_for_busy_and_locked() {
        use rusqlite::ffi;

        for code in [
            ffi::SQLITE_BUSY,
            ffi::SQLITE_LOCKED,
            ffi::SQLITE_BUSY_SNAPSHOT,
            ffi::SQLITE_LOCKED_SHAREDCACHE,
        ] {
            match begin_error(&sqlite_failure(code, "database is locked")) {
                StorageError::Transaction(TransactionError::Transient { attempts, reason }) => {
                    assert_eq!(attempts, 1, "code {code}");
                    assert_eq!(
                        reason, "Failed to begin transaction: database is locked",
                        "code {code}"
                    );
                }
                other => panic!("code {code} must be transient, got {other:?}"),
            }
        }

        for code in [
            ffi::SQLITE_ERROR,
            ffi::SQLITE_IOERR,
            ffi::SQLITE_FULL,
            ffi::SQLITE_CORRUPT,
            ffi::SQLITE_INTERRUPT,
            ffi::SQLITE_READONLY,
        ] {
            match begin_error(&sqlite_failure(code, "boom")) {
                StorageError::Transaction(TransactionError::RolledBack { reason }) => {
                    assert_eq!(reason, "Failed to begin transaction: boom", "code {code}");
                }
                other => panic!("code {code} must stay RolledBack, got {other:?}"),
            }
        }

        // A non-`SqliteFailure` error has no result code at all.
        assert!(matches!(
            begin_error(&rusqlite::Error::QueryReturnedNoRows),
            StorageError::Transaction(TransactionError::RolledBack { .. })
        ));
    }

    /// The pool's timeout is the other way a BEGIN can fail for contention.
    #[test]
    fn pool_timeout_is_transient() {
        let pool = r2d2::Pool::builder()
            .max_size(1)
            .connection_timeout(std::time::Duration::from_millis(20))
            .build_unchecked(SqliteConnectionManager::memory());
        // `build` would wait for the first connection under the same 20 ms
        // timeout, which a loaded test run can exceed: wait for it here
        // instead, so only the second `get` is timing-sensitive.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while pool.state().connections == 0 {
            assert!(std::time::Instant::now() < deadline, "pool never filled");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let _held = pool.get().unwrap();
        let err = pool.get().expect_err("the only connection is held");

        match pool_timeout_error(&err) {
            StorageError::Transaction(TransactionError::Transient { attempts, reason }) => {
                assert_eq!(attempts, 1);
                assert!(
                    reason.starts_with("Failed to begin transaction: ")
                        && reason.contains("timed out waiting for connection"),
                    "{reason:?}"
                );
            }
            other => panic!("a pool timeout must be transient, got {other:?}"),
        }
    }

    /// The bundle's begin mapping passes a transient error through untouched
    /// (so the driver text is not wrapped a second time) and keeps the
    /// `RolledBack` wrapping for everything else.
    #[test]
    fn bundle_begin_error_passes_transient_through_and_wraps_the_rest() {
        match bundle_begin_error(StorageError::Transaction(TransactionError::Transient {
            attempts: 1,
            reason: "Failed to begin transaction: database is locked".to_string(),
        })) {
            TransactionError::Transient { attempts, reason } => {
                assert_eq!(attempts, 1);
                assert_eq!(reason, "Failed to begin transaction: database is locked");
            }
            other => panic!("expected Transient, got {other:?}"),
        }

        match bundle_begin_error(begin_error(&sqlite_failure(
            rusqlite::ffi::SQLITE_ERROR,
            "boom",
        ))) {
            TransactionError::RolledBack { reason } => assert_eq!(
                reason,
                "Failed to begin transaction: transaction rolled back: \
                 Failed to begin transaction: boom",
                "other begin failures keep the wrapping they have always had"
            ),
            other => panic!("expected RolledBack, got {other:?}"),
        }

        match bundle_begin_error(internal_error("registry load failed".to_string())) {
            TransactionError::RolledBack { reason } => assert_eq!(
                reason,
                "Failed to begin transaction: internal error in sqlite: registry load failed"
            ),
            other => panic!("expected RolledBack, got {other:?}"),
        }
    }
}
