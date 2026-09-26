//! MongoDB-backed "needs reindex" ledger for failed secondary syncs (#1334).
//!
//! One document per (tenant, resource type, id, secondary) in the
//! `secondary_sync_failures` collection, keyed by a compound `_id` so the
//! uniqueness a record needs comes from the collection itself: no extra index
//! and no schema-version step. Listing sorts by `last_failed_at` under a
//! `limit`, which MongoDB answers with a bounded top-k sort. See
//! [`crate::composite::sync_failures`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};

use crate::composite::sync_failures::{
    SecondarySyncFailure, SecondarySyncFailureLedger, SyncFailureKey, SyncFailureReport,
    SyncOperation,
};
use crate::error::{BackendError, StorageError, StorageResult};

use super::MongoBackend;
use super::retry::retry_transient;

/// Name of the collection holding the records.
pub(crate) const SYNC_FAILURES_COLLECTION: &str = "secondary_sync_failures";

fn backend_err(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "mongodb".to_string(),
        message,
        source: None,
    })
}

/// The compound `_id`. Field order is part of a document's identity, so it is
/// built in exactly one place.
fn record_id(key: &SyncFailureKey) -> Document {
    doc! {
        "tenant_id": &key.tenant_id,
        "resource_type": &key.resource_type,
        "resource_id": &key.resource_id,
        "backend_id": &key.backend_id,
    }
}

/// Fixed-width RFC 3339 (UTC, microseconds): sorts as text.
fn format_time(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

fn parse_time(value: Option<&str>) -> DateTime<Utc> {
    value
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
}

#[async_trait]
impl SecondarySyncFailureLedger for MongoBackend {
    async fn record_sync_failure(&self, report: &SyncFailureReport) -> StorageResult<bool> {
        let db = self.get_database().await?;
        let collection = db.collection::<Document>(SYNC_FAILURES_COLLECTION);
        let failed_at = format_time(report.failed_at);
        let result = retry_transient(|| async {
            collection
                .update_one(
                    doc! { "_id": record_id(&report.key) },
                    doc! {
                        "$setOnInsert": { "first_failed_at": &failed_at },
                        "$set": {
                            "operation": report.operation.as_str(),
                            "last_failed_at": &failed_at,
                            "last_error": &report.error,
                        },
                        "$inc": { "attempts": i64::from(report.attempts) },
                    },
                )
                .upsert(true)
                .await
        })
        .await
        .map_err(|e| backend_err(format!("record secondary sync failure: {e}")))?;
        Ok(result.upserted_id.is_some())
    }

    async fn clear_sync_failure(&self, key: &SyncFailureKey) -> StorageResult<bool> {
        let db = self.get_database().await?;
        let collection = db.collection::<Document>(SYNC_FAILURES_COLLECTION);
        let result = retry_transient(|| async {
            collection.delete_one(doc! { "_id": record_id(key) }).await
        })
        .await
        .map_err(|e| backend_err(format!("clear secondary sync failure: {e}")))?;
        Ok(result.deleted_count > 0)
    }

    async fn list_sync_failures(&self, limit: usize) -> StorageResult<Vec<SecondarySyncFailure>> {
        let db = self.get_database().await?;
        let collection = db.collection::<Document>(SYNC_FAILURES_COLLECTION);
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let documents: Vec<Document> = retry_transient(|| async {
            collection
                .find(doc! {})
                .sort(doc! { "last_failed_at": 1_i32, "_id": 1_i32 })
                .limit(limit)
                .await?
                .try_collect()
                .await
        })
        .await
        .map_err(|e| backend_err(format!("list secondary sync failures: {e}")))?;

        Ok(documents
            .iter()
            .filter_map(|document| {
                let id = document.get_document("_id").ok()?;
                Some(SecondarySyncFailure {
                    key: SyncFailureKey {
                        tenant_id: id.get_str("tenant_id").ok()?.to_string(),
                        resource_type: id.get_str("resource_type").ok()?.to_string(),
                        resource_id: id.get_str("resource_id").ok()?.to_string(),
                        backend_id: id.get_str("backend_id").ok()?.to_string(),
                    },
                    operation: SyncOperation::from_stored(
                        document.get_str("operation").unwrap_or_default(),
                    ),
                    first_failed_at: parse_time(document.get_str("first_failed_at").ok()),
                    last_failed_at: parse_time(document.get_str("last_failed_at").ok()),
                    last_error: document
                        .get_str("last_error")
                        .unwrap_or_default()
                        .to_string(),
                    attempts: document.get_i64("attempts").unwrap_or_default().max(0) as u64,
                })
            })
            .collect())
    }

    async fn count_sync_failures(&self) -> StorageResult<u64> {
        let db = self.get_database().await?;
        let collection = db.collection::<Document>(SYNC_FAILURES_COLLECTION);
        retry_transient(|| async { collection.count_documents(doc! {}).await })
            .await
            .map_err(|e| backend_err(format!("count secondary sync failures: {e}")))
    }
}
