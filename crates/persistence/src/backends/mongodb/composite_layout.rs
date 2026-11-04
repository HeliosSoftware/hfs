//! Completeness marker for the `tuple_code` of token+quantity composite rows.
//!
//! The quantity row of a token+quantity composite group carries the codes of
//! its group's token row as `tuple_code`, and `idx_search_composite_tuple_qty`
//! answers `code=X AND value>N` with one covered scan. That is only correct
//! when EVERY quantity row of the tenant carries `tuple_code`: a row written
//! by an earlier binary has none, and would silently vanish from results. Row
//! presence cannot prove completeness, so the planner requires a marker.
//!
//! The marker is, in `search_layout`:
//! * a per-tenant document (`_id = "<tenant>|composite_tuple_code"`), written
//!   when an unscoped `$reindex` of the tenant completes with no error; or
//! * a global document (`_id = "*|composite_tuple_code"`), written at boot when
//!   `search_index` is empty, because every later row is written by a binary
//!   that stamps `tuple_code`.
//!
//! Reads are cached per tenant for [`MARKER_TTL`]; any error reads as "not
//! complete", the legacy pair-validation path. A binary that predates
//! `tuple_code` and writes to the database again leaves unstamped rows behind
//! a valid marker: run `$reindex` after such a rollback.

use std::time::{Duration, Instant};

use mongodb::bson::{Document, doc};

use super::backend::MongoBackend;
use crate::error::StorageResult;

pub(super) const LAYOUT_COLLECTION: &str = "search_layout";
const LAYOUT_COMPOSITE_TUPLE_CODE: &str = "composite_tuple_code";
const ALL_TENANTS: &str = "*";
const MARKER_TTL: Duration = Duration::from_secs(30);

fn marker_id(tenant_id: &str) -> String {
    format!("{tenant_id}|{LAYOUT_COMPOSITE_TUPLE_CODE}")
}

/// Per-tenant TTL cache of "tuple codes complete".
#[derive(Default)]
pub(super) struct MarkerCache(
    parking_lot::Mutex<std::collections::HashMap<String, (Instant, bool)>>,
);

impl MarkerCache {
    fn get(&self, tenant_id: &str) -> Option<bool> {
        let map = self.0.lock();
        map.get(tenant_id)
            .filter(|(at, _)| at.elapsed() < MARKER_TTL)
            .map(|(_, complete)| *complete)
    }

    fn put(&self, tenant_id: &str, complete: bool) {
        self.0
            .lock()
            .insert(tenant_id.to_string(), (Instant::now(), complete));
    }

    fn forget(&self, tenant_id: &str) {
        self.0.lock().remove(tenant_id);
    }
}

impl MongoBackend {
    /// Whether every token+quantity quantity row of `tenant_id` carries
    /// `tuple_code`.
    pub(super) async fn composite_tuple_codes_complete(&self, tenant_id: &str) -> bool {
        if let Some(cached) = self.composite_marker_cache.get(tenant_id) {
            return cached;
        }
        let complete = match self.get_database().await {
            Ok(db) => db
                .collection::<Document>(LAYOUT_COLLECTION)
                .find_one(doc! { "_id": { "$in": [marker_id(tenant_id), marker_id(ALL_TENANTS)] } })
                .await
                .map(|found| found.is_some())
                .unwrap_or(false),
            Err(_) => false,
        };
        self.composite_marker_cache.put(tenant_id, complete);
        complete
    }

    /// Records that every row of `tenant_id` now carries `tuple_code`.
    pub(super) async fn record_composite_tuple_codes_complete(
        &self,
        tenant_id: &str,
    ) -> StorageResult<()> {
        self.upsert_layout_marker(tenant_id).await?;
        self.composite_marker_cache.forget(tenant_id);
        Ok(())
    }

    /// At boot: an empty `search_index` means every row to come is written by a
    /// binary that stamps `tuple_code`, for every tenant.
    pub(super) async fn record_empty_search_index_at_boot(&self, db: &mongodb::Database) {
        let layout = db.collection::<Document>(LAYOUT_COLLECTION);
        let already = layout
            .find_one(doc! { "_id": marker_id(ALL_TENANTS) })
            .await
            .map(|found| found.is_some());
        if !matches!(already, Ok(false)) {
            return;
        }
        let rows = db
            .collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION)
            .estimated_document_count()
            .await;
        if !matches!(rows, Ok(0)) {
            return;
        }
        if let Err(error) = self.upsert_layout_marker(ALL_TENANTS).await {
            tracing::warn!(%error, "could not record the tuple_code marker of an empty search_index");
        }
    }

    async fn upsert_layout_marker(&self, tenant_id: &str) -> StorageResult<()> {
        let db = self.get_database().await?;
        db.collection::<Document>(LAYOUT_COLLECTION)
            .update_one(
                doc! { "_id": marker_id(tenant_id) },
                doc! { "$set": {
                    "tenant_id": tenant_id,
                    "layout": LAYOUT_COMPOSITE_TUPLE_CODE,
                    "complete_at": mongodb::bson::DateTime::now(),
                } },
            )
            .upsert(true)
            .await
            .map_err(|e| {
                super::storage::internal_error(format!("Failed to write layout marker: {e}"))
            })?;
        Ok(())
    }
}
