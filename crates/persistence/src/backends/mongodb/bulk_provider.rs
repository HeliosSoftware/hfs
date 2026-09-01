//! MongoDB implementation of the provider-side [`BulkProviderStore`].
//!
//! One document per `(tenant_id, id)` in the `bulk_provider_submissions`
//! collection, holding the opaque submission document as a JSON string
//! (BSON forbids `.`/`$` in keys) plus a monotonic `version`. Writes use the
//! same version-conditioned update the per-user settings store uses, so the
//! optimistic `If-Match` semantics match the relational backends without
//! requiring a replica set (#772).

use async_trait::async_trait;
use chrono::Utc;
use mongodb::bson::{Document, doc};
use serde_json::Value;

use crate::core::bulk_provider::{BulkProviderStore, StoredProviderSubmission};
use crate::error::{BackendError, ConcurrencyError, StorageError, StorageResult};
use crate::tenant::TenantContext;

use super::MongoBackend;

/// Name of the collection backing the provider-side submission store.
pub(crate) const BULK_PROVIDER_COLLECTION: &str = "bulk_provider_submissions";

fn backend_err(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "mongodb".to_string(),
        message,
        source: None,
    })
}

fn lock_failure(id: &str, expected: i64, actual: i64) -> StorageError {
    StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure {
        resource_type: "BulkProviderSubmission".to_string(),
        id: id.to_string(),
        expected_etag: format!("W/\"{expected}\""),
        actual_etag: Some(format!("W/\"{actual}\"")),
    })
}

async fn collect_documents(mut cursor: mongodb::Cursor<Document>) -> StorageResult<Vec<Document>> {
    let mut out = Vec::new();
    while cursor
        .advance()
        .await
        .map_err(|e| backend_err(format!("cursor advance: {e}")))?
    {
        out.push(
            cursor
                .deserialize_current()
                .map_err(|e| backend_err(format!("cursor deserialize: {e}")))?,
        );
    }
    Ok(out)
}

fn decode(doc: &Document) -> StorageResult<StoredProviderSubmission> {
    let id = doc
        .get_str("id")
        .map_err(|e| backend_err(format!("provider submission missing id: {e}")))?
        .to_string();
    let raw = doc
        .get_str("data")
        .map_err(|e| backend_err(format!("provider submission missing data: {e}")))?;
    let document: Value = serde_json::from_str(raw)
        .map_err(|e| backend_err(format!("decode stored provider submission: {e}")))?;
    let version = doc.get_i64("version").unwrap_or(1);
    let updated_at = doc
        .get_datetime("updated_at")
        .ok()
        .and_then(|t| chrono::DateTime::from_timestamp_millis(t.timestamp_millis()))
        .unwrap_or_else(Utc::now);
    Ok(StoredProviderSubmission {
        id,
        document,
        version,
        updated_at,
    })
}

#[async_trait]
impl BulkProviderStore for MongoBackend {
    async fn list_provider_submissions(
        &self,
        tenant: &TenantContext,
    ) -> StorageResult<Vec<StoredProviderSubmission>> {
        let db = self.get_database().await?;
        let collection = db.collection::<Document>(BULK_PROVIDER_COLLECTION);
        let cursor = collection
            .find(doc! { "tenant_id": tenant.tenant_id().as_str() })
            .await
            .map_err(|e| backend_err(format!("scan provider submissions: {e}")))?;
        let docs = collect_documents(cursor).await?;
        docs.iter().map(decode).collect()
    }

    async fn get_provider_submission(
        &self,
        tenant: &TenantContext,
        id: &str,
    ) -> StorageResult<Option<StoredProviderSubmission>> {
        let db = self.get_database().await?;
        let collection = db.collection::<Document>(BULK_PROVIDER_COLLECTION);
        let found = collection
            .find_one(doc! { "tenant_id": tenant.tenant_id().as_str(), "id": id })
            .await
            .map_err(|e| backend_err(format!("read provider submission: {e}")))?;
        found.as_ref().map(decode).transpose()
    }

    async fn put_provider_submission(
        &self,
        tenant: &TenantContext,
        id: &str,
        document: Value,
        if_match_version: Option<i64>,
    ) -> StorageResult<StoredProviderSubmission> {
        let db = self.get_database().await?;
        let collection = db.collection::<Document>(BULK_PROVIDER_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let data = serde_json::to_string(&document)
            .map_err(|e| backend_err(format!("encode provider submission: {e}")))?;

        // Version-conditioned write, as the settings store does: read the
        // version, then update pinned to it; a lost race surfaces as the same
        // optimistic-lock failure a stale If-Match would.
        let existing = collection
            .find_one(doc! { "tenant_id": tenant_id, "id": id })
            .await
            .map_err(|e| backend_err(format!("read provider submission version: {e}")))?;
        let current_version = existing
            .as_ref()
            .and_then(|d| d.get_i64("version").ok())
            .unwrap_or(0);
        if let Some(expected) = if_match_version
            && expected != current_version
        {
            return Err(lock_failure(id, expected, current_version));
        }

        let new_version = current_version + 1;
        let now = Utc::now();
        let now_bson = mongodb::bson::DateTime::from_millis(now.timestamp_millis());
        let outcome = if current_version == 0 {
            collection
                .insert_one(doc! {
                    "tenant_id": tenant_id,
                    "id": id,
                    "data": &data,
                    "version": new_version,
                    "updated_at": now_bson,
                })
                .await
                .map(|_| 1u64)
                .map_err(|e| backend_err(format!("insert provider submission: {e}")))?
        } else {
            collection
                .update_one(
                    doc! { "tenant_id": tenant_id, "id": id, "version": current_version },
                    doc! { "$set": { "data": &data, "version": new_version, "updated_at": now_bson } },
                )
                .await
                .map(|r| r.modified_count)
                .map_err(|e| backend_err(format!("write provider submission: {e}")))?
        };
        if outcome == 0 {
            return Err(lock_failure(id, current_version, current_version + 1));
        }

        Ok(StoredProviderSubmission {
            id: id.to_string(),
            document,
            version: new_version,
            updated_at: now,
        })
    }

    async fn delete_provider_submission(
        &self,
        tenant: &TenantContext,
        id: &str,
    ) -> StorageResult<bool> {
        let db = self.get_database().await?;
        let collection = db.collection::<Document>(BULK_PROVIDER_COLLECTION);
        let result = collection
            .delete_one(doc! { "tenant_id": tenant.tenant_id().as_str(), "id": id })
            .await
            .map_err(|e| backend_err(format!("delete provider submission: {e}")))?;
        Ok(result.deleted_count > 0)
    }
}
