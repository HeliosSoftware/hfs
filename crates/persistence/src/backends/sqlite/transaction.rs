//! Transaction support for SQLite backend.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use helios_fhir::FhirVersion;
use parking_lot::Mutex;
use r2d2::PooledConnection;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use serde_json::Value;

use crate::core::{Transaction, TransactionOptions, TransactionProvider};
use crate::error::{
    BackendError, ConcurrencyError, ResourceError, StorageError, StorageResult, TransactionError,
};
use crate::tenant::{Operation, TenantContext};
use crate::types::StoredResource;

use super::SqliteBackend;

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

/// A SQLite transaction.
pub struct SqliteTransaction {
    /// The connection used for this transaction.
    conn: Arc<Mutex<PooledConnection<SqliteConnectionManager>>>,
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
    /// Create a new transaction.
    fn new(
        conn: PooledConnection<SqliteConnectionManager>,
        tenant: TenantContext,
        backend: SqliteBackend,
        fhir_version: FhirVersion,
    ) -> StorageResult<Self> {
        // Start the transaction
        conn.execute("BEGIN IMMEDIATE", []).map_err(|e| {
            StorageError::Transaction(TransactionError::RolledBack {
                reason: format!("Failed to begin transaction: {}", e),
            })
        })?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            active: true,
            tenant,
            backend,
            invalidate_registry: false,
            fhir_version,
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

    /// Generate a new ID for a resource.
    fn generate_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// Index a resource for search — through the backend's own path (dynamic
    /// extraction with minimal fallback, FTS content, contained resources),
    /// so transactional writes are searchable exactly like direct ones
    /// (#815 review).
    fn index_resource(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> StorageResult<()> {
        self.backend
            .delete_search_index(conn, tenant_id, resource_type, resource_id)?;
        self.backend
            .index_resource(conn, tenant_id, resource_type, resource_id, resource)
    }
}

#[async_trait]
impl Transaction for SqliteTransaction {
    async fn create(
        &mut self,
        resource_type: &str,
        resource: Value,
    ) -> StorageResult<StoredResource> {
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
                id: id.to_string(),
            }));
        }

        // Build the resource with id and resourceType
        let mut data = resource.clone();
        if let Some(obj) = data.as_object_mut() {
            obj.insert("id".to_string(), Value::String(id.clone()));
            obj.insert(
                "resourceType".to_string(),
                Value::String(resource_type.to_string()),
            );
        }

        // Serialize the resource data
        let data_bytes = serde_json::to_vec(&data)
            .map_err(|e| serialization_error(format!("Failed to serialize resource: {}", e)))?;

        let now = Utc::now();
        let last_updated = now.to_rfc3339();
        let version_id = "1";

        let fhir_version_str = self.fhir_version.as_mime_param();

        // Insert the resource
        conn.execute(
            "INSERT INTO resources (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
            params![tenant_id, resource_type, id, version_id, data_bytes, last_updated, fhir_version_str],
        )
        .map_err(|e| internal_error(format!("Failed to insert resource: {}", e)))?;

        // Insert into history
        conn.execute(
            "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
            params![tenant_id, resource_type, id, version_id, data_bytes, last_updated, fhir_version_str],
        )
        .map_err(|e| internal_error(format!("Failed to insert history: {}", e)))?;

        // Index the resource for search
        self.index_resource(&conn, tenant_id, resource_type, &id, &data)?;
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

    async fn read(
        &mut self,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        if !self.active {
            return Err(StorageError::Transaction(
                TransactionError::InvalidTransaction,
            ));
        }

        let conn = self.conn.lock();
        let tenant_id = self.tenant.tenant_id().as_str();

        let result = conn.query_row(
            "SELECT version_id, data, last_updated, is_deleted, fhir_version
             FROM resources
             WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3",
            params![tenant_id, resource_type, id],
            |row| {
                let version_id: String = row.get(0)?;
                let data: Vec<u8> = row.get(1)?;
                let last_updated: String = row.get(2)?;
                let is_deleted: i32 = row.get(3)?;
                let fhir_version: String = row.get(4)?;
                Ok((version_id, data, last_updated, is_deleted, fhir_version))
            },
        );

        match result {
            Ok((version_id, data, last_updated, is_deleted, fhir_version_str)) => {
                if is_deleted != 0 {
                    return Ok(None);
                }

                let json_data: serde_json::Value = serde_json::from_slice(&data).map_err(|e| {
                    serialization_error(format!("Failed to deserialize resource: {}", e))
                })?;

                let last_updated = chrono::DateTime::parse_from_rfc3339(&last_updated)
                    .map_err(|e| internal_error(format!("Failed to parse last_updated: {}", e)))?
                    .with_timezone(&Utc);

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
                    None,
                    fhir_version,
                )))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(internal_error(format!("Failed to read resource: {}", e))),
        }
    }

    async fn update(
        &mut self,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
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
             WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3 AND is_deleted = 0",
            params![tenant_id, resource_type, id],
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
        let mut data = resource.clone();
        if let Some(obj) = data.as_object_mut() {
            obj.insert("id".to_string(), Value::String(id.to_string()));
            obj.insert(
                "resourceType".to_string(),
                Value::String(resource_type.to_string()),
            );
        }

        // Serialize the resource data
        let data_bytes = serde_json::to_vec(&data)
            .map_err(|e| serialization_error(format!("Failed to serialize resource: {}", e)))?;

        let now = Utc::now();
        let last_updated = now.to_rfc3339();

        // Preserve FHIR version from the current resource
        let fhir_version = current.fhir_version();
        let fhir_version_str = fhir_version.as_mime_param();

        // Update the resource
        conn.execute(
            "UPDATE resources SET version_id = ?1, data = ?2, last_updated = ?3
             WHERE tenant_id = ?4 AND resource_type = ?5 AND id = ?6",
            params![
                new_version_str,
                data_bytes,
                last_updated,
                tenant_id,
                resource_type,
                id
            ],
        )
        .map_err(|e| internal_error(format!("Failed to update resource: {}", e)))?;

        // Insert into history
        conn.execute(
            "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
            params![tenant_id, resource_type, id, new_version_str, data_bytes, last_updated, fhir_version_str],
        )
        .map_err(|e| internal_error(format!("Failed to insert history: {}", e)))?;

        // Re-index the resource for search
        self.index_resource(&conn, tenant_id, resource_type, id, &data)?;
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
        conn.execute("COMMIT", []).map_err(|e| {
            StorageError::Transaction(TransactionError::RolledBack {
                reason: format!("Commit failed: {}", e),
            })
        })?;
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
        // If transaction wasn't explicitly committed or rolled back, roll it back
        if self.active {
            let conn = self.conn.lock();
            let _ = conn.execute("ROLLBACK", []);
        }
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
        let conn = self.get_connection()?;
        SqliteTransaction::new(
            conn,
            tenant.clone(),
            self.clone(),
            options.fhir_version.unwrap_or(self.config().fhir_version),
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
}
