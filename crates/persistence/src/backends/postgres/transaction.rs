//! Transaction support for PostgreSQL backend.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use deadpool_postgres::Client;
use helios_fhir::FhirVersion;
use serde_json::Value;

use crate::core::{Transaction, TransactionOptions, TransactionProvider};
use crate::error::{
    BackendError, ConcurrencyError, ResourceError, StorageError, StorageResult, TransactionError,
};
use crate::search::SearchParameterExtractor;
use crate::tenant::TenantContext;
use crate::types::StoredResource;

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

/// A PostgreSQL transaction.
///
/// Wraps a deadpool_postgres Client that has an active transaction.
/// The transaction is automatically rolled back on drop if not committed.
pub struct PostgresTransaction {
    /// The client with active transaction.
    /// Option so we can take it during commit/rollback.
    client: Option<Client>,
    /// Whether the transaction is still active.
    active: bool,
    /// The tenant context for this transaction.
    tenant: TenantContext,
    /// Search parameter extractor for indexing resources.
    search_extractor: Arc<SearchParameterExtractor>,
    /// When true, search indexing is offloaded to a secondary backend.
    search_offloaded: bool,
}

impl std::fmt::Debug for PostgresTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresTransaction")
            .field("active", &self.active)
            .field("tenant", &self.tenant)
            .finish()
    }
}

impl PostgresTransaction {
    /// Create a new transaction.
    async fn new(
        client: Client,
        tenant: TenantContext,
        search_extractor: Arc<SearchParameterExtractor>,
        search_offloaded: bool,
    ) -> StorageResult<Self> {
        // Start the transaction
        client.execute("BEGIN", &[]).await.map_err(|e| {
            StorageError::Transaction(TransactionError::RolledBack {
                reason: format!("Failed to begin transaction: {}", e),
            })
        })?;

        Ok(Self {
            client: Some(client),
            active: true,
            tenant,
            search_extractor,
            search_offloaded,
        })
    }

    fn client(&self) -> StorageResult<&Client> {
        self.client
            .as_ref()
            .ok_or_else(|| StorageError::Transaction(TransactionError::InvalidTransaction))
    }

    /// Index a resource for search within the transaction.
    async fn index_resource(
        &self,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> StorageResult<()> {
        if self.search_offloaded {
            return Ok(());
        }

        let client = self.client()?;

        // Delete existing index entries
        client
            .execute(
                "DELETE FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3",
                &[&tenant_id, &resource_type, &resource_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to clear search index: {}", e)))?;

        // Extract values using the registry-driven extractor
        let values = self
            .search_extractor
            .extract(resource, resource_type)
            .map_err(|e| internal_error(format!("Search parameter extraction failed: {}", e)))?;

        // Write each extracted value to the index
        for value in values {
            PostgresSearchIndexWriter::write_entry(
                client,
                tenant_id,
                resource_type,
                resource_id,
                &value,
            )
            .await?;
        }

        tracing::debug!(
            "Indexed resource {}/{} within transaction",
            resource_type,
            resource_id
        );

        Ok(())
    }
}

#[async_trait]
impl Transaction for PostgresTransaction {
    async fn create(
        &mut self,
        resource_type: &str,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        if !self.active {
            return Err(StorageError::Transaction(
                TransactionError::InvalidTransaction,
            ));
        }

        let client = self.client()?;
        let tenant_id = self.tenant.tenant_id().as_str();

        // Get or generate ID
        let id = resource
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
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

        let now = Utc::now();
        let version_id = "1";
        let fhir_version = FhirVersion::default_enabled();
        let fhir_version_str = fhir_version.as_mime_param();
        let is_deleted = false;

        // Insert the resource
        client
            .execute(
                "INSERT INTO resources (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&tenant_id, &resource_type, &id, &version_id, &data, &now, &is_deleted, &fhir_version_str],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to insert resource: {}", e)))?;

        // Insert into history
        client
            .execute(
                "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&tenant_id, &resource_type, &id, &version_id, &data, &now, &is_deleted, &fhir_version_str],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to insert history: {}", e)))?;

        // Index the resource for search
        self.index_resource(tenant_id, resource_type, &id, &data)
            .await?;

        Ok(StoredResource::from_storage(
            resource_type,
            &id,
            version_id,
            self.tenant.tenant_id().clone(),
            data,
            now,
            now,
            None,
            fhir_version,
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

        let client = self.client()?;
        let tenant_id = self.tenant.tenant_id().as_str();

        let row = client
            .query_opt(
                "SELECT version_id, data, last_updated, is_deleted, fhir_version
                 FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to read resource: {}", e)))?;

        match row {
            Some(row) => {
                let version_id: String = row.get(0);
                let data: serde_json::Value = row.get(1);
                let last_updated: chrono::DateTime<Utc> = row.get(2);
                let is_deleted: bool = row.get(3);
                let fhir_version_str: String = row.get(4);

                if is_deleted {
                    return Ok(None);
                }

                let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                    .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

                Ok(Some(StoredResource::from_storage(
                    resource_type,
                    id,
                    version_id,
                    self.tenant.tenant_id().clone(),
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
        &mut self,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        if !self.active {
            return Err(StorageError::Transaction(
                TransactionError::InvalidTransaction,
            ));
        }

        let client = self.client()?;
        let tenant_id = self.tenant.tenant_id().as_str();
        let resource_type = current.resource_type();
        let id = current.id();

        // Verify current version still matches (optimistic locking)
        let row = client
            .query_opt(
                "SELECT version_id FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND is_deleted = FALSE",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get current version: {}", e)))?;

        let db_version = match row {
            Some(row) => row.get::<_, String>(0),
            None => {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
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

        let now = Utc::now();
        let fhir_version = current.fhir_version();
        let fhir_version_str = fhir_version.as_mime_param();
        let is_deleted = false;

        // Update the resource
        client
            .execute(
                "UPDATE resources SET version_id = $1, data = $2, last_updated = $3
                 WHERE tenant_id = $4 AND resource_type = $5 AND id = $6",
                &[
                    &new_version_str,
                    &data,
                    &now,
                    &tenant_id,
                    &resource_type,
                    &id,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to update resource: {}", e)))?;

        // Insert into history
        client
            .execute(
                "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&tenant_id, &resource_type, &id, &new_version_str, &data, &now, &is_deleted, &fhir_version_str],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to insert history: {}", e)))?;

        // Re-index the resource for search
        self.index_resource(tenant_id, resource_type, id, &data)
            .await?;

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
        if !self.active {
            return Err(StorageError::Transaction(
                TransactionError::InvalidTransaction,
            ));
        }

        let client = self.client()?;
        let tenant_id = self.tenant.tenant_id().as_str();

        // Check if resource exists
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
                let d: serde_json::Value = row.get(1);
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

        // Insert deletion record into history
        client
            .execute(
                "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&tenant_id, &resource_type, &id, &new_version_str, &data, &now, &is_deleted, &fhir_version_str],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to insert deletion history: {}", e)))?;

        Ok(())
    }

    async fn commit(mut self: Box<Self>) -> StorageResult<()> {
        if !self.active {
            return Err(StorageError::Transaction(
                TransactionError::InvalidTransaction,
            ));
        }

        if let Some(client) = self.client.as_ref() {
            client.execute("COMMIT", &[]).await.map_err(|e| {
                StorageError::Transaction(TransactionError::RolledBack {
                    reason: format!("Commit failed: {}", e),
                })
            })?;
        }

        self.active = false;
        Ok(())
    }

    async fn rollback(mut self: Box<Self>) -> StorageResult<()> {
        if !self.active {
            return Err(StorageError::Transaction(
                TransactionError::InvalidTransaction,
            ));
        }

        if let Some(client) = self.client.as_ref() {
            client.execute("ROLLBACK", &[]).await.map_err(|e| {
                StorageError::Transaction(TransactionError::RolledBack {
                    reason: format!("Rollback failed: {}", e),
                })
            })?;
        }

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

impl Drop for PostgresTransaction {
    fn drop(&mut self) {
        // If the transaction wasn't explicitly committed or rolled back, we must
        // still ROLLBACK before the connection is returned to the pool. deadpool's
        // default recycling does NOT reset session state, so a connection handed
        // back with an open transaction poisons the pool: the next checkout fails
        // with "there is already a transaction in progress", cascading across every
        // subsequent request that reuses it. (The previous code assumed recycling
        // auto-rolls-back, which is false — this was the cause of the import-suite
        // failure cascade under concurrent load.)
        //
        // Drop can't be async, so move the client into a spawned task that issues
        // the ROLLBACK and only then drops it — the connection returns to the pool
        // clean, and only after the rollback completes.
        if !self.active {
            return;
        }
        self.active = false;
        let Some(client) = self.client.take() else {
            return;
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                tracing::warn!(
                    "PostgreSQL transaction dropped without explicit commit/rollback; rolling back before pool return"
                );
                handle.spawn(async move {
                    // Ignore the result: the connection drops (returns to the pool)
                    // when this task ends regardless, and a failed rollback here
                    // just means a broken connection deadpool will discard anyway.
                    let _ = client.execute("ROLLBACK", &[]).await;
                });
            }
            Err(_) => {
                // No async runtime (e.g. a synchronous test drop): can't roll back.
                tracing::warn!(
                    "PostgreSQL transaction dropped without explicit commit/rollback and no runtime available to roll back; connection may re-enter the pool with an open transaction"
                );
            }
        }
    }
}

#[async_trait]
impl TransactionProvider for PostgresBackend {
    type Transaction = PostgresTransaction;

    async fn begin_transaction(
        &self,
        tenant: &TenantContext,
        _options: TransactionOptions,
    ) -> StorageResult<Self::Transaction> {
        let client = self.get_client().await?;
        PostgresTransaction::new(
            client,
            tenant.clone(),
            self.search_extractor().clone(),
            self.is_search_offloaded(),
        )
        .await
    }
}
