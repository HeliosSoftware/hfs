//! MongoDB implementation of the [`SofTerminologyStore`] (#2185).
//!
//! A standalone MongoDB has no multi-document transactions, so this is the
//! generation-and-registry algorithm of
//! [`sof_terminology_cas`](crate::backends::sof_terminology_cas) over two
//! collections:
//!
//! - `sof_terminology_registry`: one document per tenant holding its
//!   registry (as a JSON string, like `user_settings`) and a `version` the
//!   compare-and-swap is pinned to. A unique index on `tenant_id` turns a lost
//!   first-insert race into a duplicate-key error.
//! - `sof_terminology_rows`: a generation's rows, as JSON-array strings of at
//!   most [`CHUNK_ROWS`] rows per document (MongoDB caps a document at
//!   16 MiB), ordered by `seq`.
//!
//! [`SofTerminologyStore`]: crate::core::sof_terminology::SofTerminologyStore

use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};
use serde_json::Value;

use std::sync::Arc;

use mongodb::{Client, Database};
use tokio::sync::OnceCell;

use super::retry::retry_transient;
use super::user_settings::is_duplicate_key_error;
use super::{MongoBackend, MongoBackendConfig};
use crate::backends::sof_terminology_cas::{
    GenerationStore, TerminologyRegistry, impl_sof_terminology_store,
};
use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::TenantContext;

/// Rows per `sof_terminology_rows` document: MongoDB caps a document at
/// 16 MiB, and a relation may hold up to
/// `HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD` rows.
const CHUNK_ROWS: usize = 1000;

/// Collection holding one registry document per tenant.
pub(crate) const SOF_TERMINOLOGY_REGISTRY_COLLECTION: &str = "sof_terminology_registry";

/// Collection holding the generations' row chunks.
pub(crate) const SOF_TERMINOLOGY_ROWS_COLLECTION: &str = "sof_terminology_rows";

fn backend_err(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "mongodb".to_string(),
        message,
        source: None,
    })
}

/// The store handed out by `MongoBackend::sof_terminology_store`: the
/// backend's shared client cell and its config, like the in-DB SOF runner.
pub(crate) struct MongoTerminologyStore {
    client: Arc<OnceCell<Client>>,
    config: MongoBackendConfig,
}

impl MongoTerminologyStore {
    pub(crate) fn new(backend: &MongoBackend) -> Self {
        Self {
            client: backend.client_cell(),
            config: backend.config().clone(),
        }
    }

    async fn get_database(&self) -> StorageResult<Database> {
        let client = self
            .client
            .get_or_try_init(|| super::backend::connect_client(&self.config))
            .await?;
        Ok(client.database(&self.config.database_name))
    }
}

impl MongoBackend {
    /// Removes the tenant's registry and every generation it wrote (purge).
    pub(super) async fn purge_tenant_sof_terminology(&self, tenant_id: &str) -> StorageResult<()> {
        let db = self.get_database().await?;
        // Registry first: once it is gone no reader resolves a generation, so
        // a failure between the two deletes leaves only unreachable rows.
        for collection in [
            SOF_TERMINOLOGY_REGISTRY_COLLECTION,
            SOF_TERMINOLOGY_ROWS_COLLECTION,
        ] {
            db.collection::<Document>(collection)
                .delete_many(doc! { "tenant_id": tenant_id })
                .await
                .map_err(|e| backend_err(format!("purge delete ({collection}): {e}")))?;
        }
        Ok(())
    }
}

#[async_trait]
impl GenerationStore for MongoTerminologyStore {
    /// The registry document's `version`; 0 when there is none yet.
    type Token = i64;

    async fn read_registry(
        &self,
        tenant: &TenantContext,
    ) -> StorageResult<(TerminologyRegistry, i64)> {
        let db = self.get_database().await?;
        let collection = db.collection::<Document>(SOF_TERMINOLOGY_REGISTRY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let found = retry_transient(|| async {
            collection.find_one(doc! { "tenant_id": tenant_id }).await
        })
        .await
        .map_err(|e| backend_err(format!("read terminology registry: {e}")))?;
        let Some(found) = found else {
            return Ok((TerminologyRegistry::default(), 0));
        };
        let version = found
            .get_i64("version")
            .map_err(|e| backend_err(format!("terminology registry version: {e}")))?;
        let registry = found
            .get_str("registry")
            .map_err(|e| backend_err(format!("terminology registry body: {e}")))
            .and_then(|json| {
                serde_json::from_str(json)
                    .map_err(|e| backend_err(format!("decode terminology registry: {e}")))
            })?;
        Ok((registry, version))
    }

    async fn swap_registry(
        &self,
        tenant: &TenantContext,
        expected: &i64,
        registry: &TerminologyRegistry,
    ) -> StorageResult<bool> {
        let db = self.get_database().await?;
        let collection = db.collection::<Document>(SOF_TERMINOLOGY_REGISTRY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let body = serde_json::to_string(registry)
            .map_err(|e| backend_err(format!("encode terminology registry: {e}")))?;
        let version = expected + 1;
        if *expected == 0 {
            return match collection
                .insert_one(doc! { "tenant_id": tenant_id, "version": version, "registry": &body })
                .await
            {
                Ok(_) => Ok(true),
                Err(e) if is_duplicate_key_error(&e) => Ok(false),
                Err(e) => Err(backend_err(format!("insert terminology registry: {e}"))),
            };
        }
        let result = collection
            .update_one(
                doc! { "tenant_id": tenant_id, "version": expected },
                doc! { "$set": { "version": version, "registry": &body } },
            )
            .await
            .map_err(|e| backend_err(format!("swap terminology registry: {e}")))?;
        Ok(result.matched_count == 1)
    }

    async fn write_generation(
        &self,
        tenant: &TenantContext,
        generation: &str,
        rows: &[Value],
    ) -> StorageResult<()> {
        let db = self.get_database().await?;
        let collection = db.collection::<Document>(SOF_TERMINOLOGY_ROWS_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let mut chunks = Vec::new();
        for (seq, chunk) in rows.chunks(CHUNK_ROWS).enumerate() {
            let body = serde_json::to_string(chunk)
                .map_err(|e| backend_err(format!("encode terminology rows: {e}")))?;
            chunks.push(doc! {
                "tenant_id": tenant_id,
                "generation": generation,
                "seq": seq as i64,
                "rows": body,
            });
        }
        if chunks.is_empty() {
            return Ok(());
        }
        collection
            .insert_many(chunks)
            .await
            .map_err(|e| backend_err(format!("write terminology rows: {e}")))?;
        Ok(())
    }

    async fn read_generation(
        &self,
        tenant: &TenantContext,
        generation: &str,
    ) -> StorageResult<Option<Vec<Value>>> {
        let db = self.get_database().await?;
        let collection = db.collection::<Document>(SOF_TERMINOLOGY_ROWS_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let chunks: Vec<Document> = retry_transient(|| async {
            collection
                .find(doc! { "tenant_id": tenant_id, "generation": generation })
                .sort(doc! { "seq": 1 })
                .await?
                .try_collect()
                .await
        })
        .await
        .map_err(|e| backend_err(format!("read terminology rows: {e}")))?;
        // An empty relation has no chunks; the row count check in the caller
        // tells it apart from a deleted one.
        let mut rows = Vec::new();
        for chunk in chunks {
            let body = chunk
                .get_str("rows")
                .map_err(|e| backend_err(format!("terminology rows body: {e}")))?;
            let decoded: Vec<Value> = serde_json::from_str(body)
                .map_err(|e| backend_err(format!("decode terminology rows: {e}")))?;
            rows.extend(decoded);
        }
        Ok(Some(rows))
    }

    async fn delete_generations(
        &self,
        tenant: &TenantContext,
        generations: &[String],
    ) -> StorageResult<()> {
        if generations.is_empty() {
            return Ok(());
        }
        let db = self.get_database().await?;
        db.collection::<Document>(SOF_TERMINOLOGY_ROWS_COLLECTION)
            .delete_many(doc! {
                "tenant_id": tenant.tenant_id().as_str(),
                "generation": { "$in": generations },
            })
            .await
            .map_err(|e| backend_err(format!("delete terminology rows: {e}")))?;
        Ok(())
    }
}

impl_sof_terminology_store!(MongoTerminologyStore);
