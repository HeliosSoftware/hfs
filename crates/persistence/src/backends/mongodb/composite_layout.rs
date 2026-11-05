//! Completeness marker for the `tuple_code` of token+quantity composite rows.
//!
//! The quantity row of a token+quantity composite group carries the codes of
//! its group's token row as `tuple_code`, and `idx_search_composite_tuple_qty`
//! answers `code=X AND value>N` with one range scan. That is only correct when
//! EVERY quantity row in scope carries `tuple_code`: a row written by an
//! earlier binary has none, and would silently vanish from results. Row
//! presence cannot prove completeness, so the planner requires a marker.
//!
//! A marker is a `search_layout` document written when a `$reindex` that
//! rebuilt every resource of its scope completes with no error
//! ([`crate::search::ReindexCoverage`]):
//! * per tenant (`_id = "<tenant>|composite_tuple_code_v2"`) for a run over
//!   every type, `bulkIndexRebuild` included;
//! * per tenant and type (`_id = "<tenant>|<type>|composite_tuple_code_v2"`)
//!   for a run named by types.
//!
//! There is deliberately no all-tenant marker for a database that was empty at
//! boot: a dump restored after that boot, an older replica of a mixed fleet, or
//! a tool that writes rows directly would all write rows without `tuple_code`
//! behind it, and nothing would ever notice. A new tenant therefore uses the
//! pair-validation path until its first `$reindex`, which is cheap while the
//! tenant is small.
//!
//! A marker is trusted only while the recorded `search_index` generation is at
//! least [`COMPOSITE_TUPLE_GENERATION`]. A binary that predates `tuple_code`
//! records its own, lower generation when it boots, so:
//! * a running instance stops trusting the markers once an older instance
//!   booted (within [`MARKER_TTL`]);
//! * the next boot of this binary on a database recorded below that generation
//!   (a rollback and a return) deletes every marker before its builder records
//!   the current generation again ([`MongoBackend::drop_untrusted_tuple_markers_at_boot`]).
//!
//! Reads are cached per tenant and type for [`MARKER_TTL`]; any error reads as
//! "not complete", the pair-validation path.

use std::time::{Duration, Instant};

use mongodb::bson::{Document, doc};

use super::backend::MongoBackend;
use super::search_index_catalog::COMPOSITE_TUPLE_GENERATION;
use crate::error::StorageResult;
use crate::search::ReindexCoverage;

pub(super) const LAYOUT_COLLECTION: &str = "search_layout";
/// `_v2`: the length-prefixed `tuple_code` forms. A marker of the first,
/// `|`-separated forms is never read.
const LAYOUT_COMPOSITE_TUPLE_CODE: &str = "composite_tuple_code_v2";
const MARKER_TTL: Duration = Duration::from_secs(30);

fn tenant_marker_id(tenant_id: &str) -> String {
    format!("{tenant_id}|{LAYOUT_COMPOSITE_TUPLE_CODE}")
}

fn type_marker_id(tenant_id: &str, resource_type: &str) -> String {
    format!("{tenant_id}|{resource_type}|{LAYOUT_COMPOSITE_TUPLE_CODE}")
}

/// Whether markers can be trusted under the recorded `search_index`
/// generation (`None`: never recorded).
fn markers_trusted(recorded_generation: Option<i32>) -> bool {
    recorded_generation.is_some_and(|generation| generation >= COMPOSITE_TUPLE_GENERATION)
}

/// Per tenant and type TTL cache of "tuple codes complete".
#[derive(Default)]
pub(super) struct MarkerCache(
    parking_lot::Mutex<std::collections::HashMap<(String, String), (Instant, bool)>>,
);

impl MarkerCache {
    fn get(&self, tenant_id: &str, resource_type: &str) -> Option<bool> {
        let map = self.0.lock();
        map.get(&(tenant_id.to_string(), resource_type.to_string()))
            .filter(|(at, _)| at.elapsed() < MARKER_TTL)
            .map(|(_, complete)| *complete)
    }

    fn put(&self, tenant_id: &str, resource_type: &str, complete: bool) {
        self.0.lock().insert(
            (tenant_id.to_string(), resource_type.to_string()),
            (Instant::now(), complete),
        );
    }

    fn forget_tenant(&self, tenant_id: &str) {
        self.0.lock().retain(|(tenant, _), _| tenant != tenant_id);
    }
}

impl MongoBackend {
    /// Whether every token+quantity quantity row of `resource_type` in
    /// `tenant_id` carries `tuple_code`.
    pub(super) async fn composite_tuple_codes_complete(
        &self,
        tenant_id: &str,
        resource_type: &str,
    ) -> bool {
        if let Some(cached) = self.composite_marker_cache.get(tenant_id, resource_type) {
            return cached;
        }
        let complete = match self.get_database().await {
            Ok(db) => {
                let marker = db
                    .collection::<Document>(LAYOUT_COLLECTION)
                    .find_one(doc! { "_id": { "$in": [
                        tenant_marker_id(tenant_id),
                        type_marker_id(tenant_id, resource_type),
                    ] } })
                    .await
                    .map(|found| found.is_some())
                    .unwrap_or(false);
                marker
                    && super::schema::get_search_index_generation(&db)
                        .await
                        .is_ok_and(markers_trusted)
            }
            Err(_) => false,
        };
        self.composite_marker_cache
            .put(tenant_id, resource_type, complete);
        complete
    }

    /// Records that every row of `coverage` in `tenant_id` now carries
    /// `tuple_code`.
    pub(super) async fn record_composite_tuple_codes_complete(
        &self,
        tenant_id: &str,
        coverage: ReindexCoverage<'_>,
    ) -> StorageResult<()> {
        match coverage {
            ReindexCoverage::Tenant => {
                self.upsert_layout_marker(tenant_marker_id(tenant_id), tenant_id, None)
                    .await?
            }
            ReindexCoverage::Types(types) => {
                for resource_type in types {
                    self.upsert_layout_marker(
                        type_marker_id(tenant_id, resource_type),
                        tenant_id,
                        Some(resource_type),
                    )
                    .await?;
                }
            }
        }
        self.composite_marker_cache.forget_tenant(tenant_id);
        Ok(())
    }

    /// Deletes the markers of `tenant_id` (tenant purge).
    pub(super) async fn purge_composite_tuple_markers(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
    ) -> StorageResult<()> {
        db.collection::<Document>(LAYOUT_COLLECTION)
            .delete_many(doc! { "tenant_id": tenant_id })
            .await
            .map_err(|e| {
                super::storage::internal_error(format!("Failed to purge layout markers: {e}"))
            })?;
        self.composite_marker_cache.forget_tenant(tenant_id);
        Ok(())
    }

    /// At boot, before the builder records this generation: a database
    /// recorded below [`COMPOSITE_TUPLE_GENERATION`] (or never recorded) was
    /// last booted by a binary that writes rows without `tuple_code`, or has
    /// not finished a build of this one, so every marker goes. The tenants it
    /// held need a `$reindex` to use `idx_search_composite_tuple_qty` again.
    pub(super) async fn drop_untrusted_tuple_markers_at_boot(&self, db: &mongodb::Database) {
        let recorded = match super::schema::get_search_index_generation(db).await {
            Ok(recorded) => recorded,
            Err(error) => {
                // Read-time checks still refuse the markers until the
                // generation reads as current.
                tracing::warn!(%error, "could not read the search_index generation; tuple_code markers kept");
                return;
            }
        };
        if markers_trusted(recorded) {
            return;
        }
        let deleted = db
            .collection::<Document>(LAYOUT_COLLECTION)
            .delete_many(doc! { "layout": LAYOUT_COMPOSITE_TUPLE_CODE })
            .await;
        match deleted {
            Ok(result) if result.deleted_count > 0 => tracing::warn!(
                recorded_generation = ?recorded,
                markers = result.deleted_count,
                "search_index was last recorded below generation {COMPOSITE_TUPLE_GENERATION}, so \
                 rows without tuple_code may exist: removed the tuple_code markers; token+quantity \
                 composite searches use pair validation until `$reindex` rebuilds each tenant"
            ),
            Ok(_) => {
                if recorded.is_some() {
                    tracing::info!(
                        recorded_generation = ?recorded,
                        "token+quantity composite searches use pair validation until `$reindex` \
                         rebuilds each tenant (rows written before generation \
                         {COMPOSITE_TUPLE_GENERATION} have no tuple_code)"
                    );
                }
            }
            Err(error) => {
                tracing::warn!(%error, "could not remove the tuple_code markers at boot")
            }
        }
    }

    async fn upsert_layout_marker(
        &self,
        id: String,
        tenant_id: &str,
        resource_type: Option<&str>,
    ) -> StorageResult<()> {
        let db = self.get_database().await?;
        db.collection::<Document>(LAYOUT_COLLECTION)
            .update_one(
                doc! { "_id": id },
                doc! { "$set": {
                    "tenant_id": tenant_id,
                    "resource_type": resource_type,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_are_trusted_only_at_or_past_the_tuple_generation() {
        assert!(!markers_trusted(None));
        for generation in 1..COMPOSITE_TUPLE_GENERATION {
            assert!(!markers_trusted(Some(generation)), "{generation}");
        }
        assert!(markers_trusted(Some(COMPOSITE_TUPLE_GENERATION)));
        assert!(markers_trusted(Some(COMPOSITE_TUPLE_GENERATION + 1)));
    }

    #[test]
    fn marker_ids_keep_tenants_and_types_apart() {
        assert_eq!(tenant_marker_id("t"), "t|composite_tuple_code_v2");
        assert_eq!(
            type_marker_id("t", "Observation"),
            "t|Observation|composite_tuple_code_v2"
        );
    }
}
