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
//! * the builder of this binary, on a database recorded below that generation
//!   (a rollback and a return), deletes every marker right before it records
//!   the current generation again, from the same read of the recorded
//!   generation ([`drop_untrusted_tuple_markers`]), and records nothing when
//!   it cannot delete them.
//!
//! Reads are cached per tenant and type for [`MARKER_TTL`]; any error reads as
//! "not complete", the pair-validation path.

use std::time::{Duration, Instant};

use mongodb::bson::{Document, doc};

use super::backend::MongoBackend;
use super::composite_search::TUPLE_CODE_FIELD;
use super::search_index_catalog::{COMPOSITE_TUPLE_GENERATION, SEARCH_INDEX_COLLECTION};
use crate::error::StorageResult;
use crate::search::ReindexCoverage;
use crate::types::SearchParamType;

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

/// Called by the `search_index` builder right before it records
/// [`COMPOSITE_TUPLE_GENERATION`] or later over `recorded`, the generation it
/// read for that record: a database recorded below it (or never recorded) was
/// last booted by a binary that writes rows without `tuple_code`, or has not
/// finished a build of this one, so every marker goes. Deciding from the same
/// read the record replaces leaves no window in which an older binary records
/// its generation and writes rows the markers would then cover. An error is
/// returned, not logged: the builder must not record a generation that trusts
/// markers it could not delete. The tenants they held need a `$reindex` to
/// use `idx_search_composite_tuple_qty` again.
pub(super) async fn drop_untrusted_tuple_markers(
    db: &mongodb::Database,
    recorded: Option<i32>,
) -> StorageResult<()> {
    if markers_trusted(recorded) {
        return Ok(());
    }
    let deleted = db
        .collection::<Document>(LAYOUT_COLLECTION)
        .delete_many(doc! { "layout": LAYOUT_COMPOSITE_TUPLE_CODE })
        .await
        .map_err(|e| {
            super::storage::internal_error(format!(
                "Failed to remove the tuple_code markers before recording generation \
                 {COMPOSITE_TUPLE_GENERATION}: {e}"
            ))
        })?;
    if deleted.deleted_count > 0 {
        tracing::warn!(
            recorded_generation = ?recorded,
            markers = deleted.deleted_count,
            "search_index was last recorded below generation {COMPOSITE_TUPLE_GENERATION}, so \
             rows without tuple_code may exist: removed the tuple_code markers; token+quantity \
             composite searches use pair validation until `$reindex` rebuilds each tenant"
        );
    } else if recorded.is_some() {
        tracing::info!(
            recorded_generation = ?recorded,
            "token+quantity composite searches use pair validation until `$reindex` \
             rebuilds each tenant (rows written before generation \
             {COMPOSITE_TUPLE_GENERATION} have no tuple_code)"
        );
    }
    Ok(())
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
    /// `tuple_code`, after checking it: a scope that still holds a quantity row
    /// the planner would miss (written during the `$reindex` by an older
    /// instance, or by a path that does not stamp) gets no marker, with a
    /// warning. For a run over every type that means no tenant marker at all.
    ///
    /// A tenant purge can run, on any instance, before or while the run's
    /// hook writes, and would leave the marker behind for a tenant id that
    /// may be reused. [`remove_markers_of_a_purged_tenant`] settles that once
    /// the markers are written.
    pub(super) async fn record_composite_tuple_codes_complete(
        &self,
        tenant_id: &str,
        coverage: ReindexCoverage<'_>,
    ) -> StorageResult<()> {
        let db = self.get_database().await?;
        let mut written = Vec::new();
        let wrote = self
            .write_verified_markers(&db, tenant_id, coverage, &mut written)
            .await;
        // Also after a failed write: the markers written before it count.
        let settled = remove_markers_of_a_purged_tenant(&db, tenant_id, &written).await;
        self.composite_marker_cache.forget_tenant(tenant_id);
        wrote.and(settled)
    }

    /// The marker writes of [`Self::record_composite_tuple_codes_complete`],
    /// each after its scope was checked; `written` collects their ids.
    async fn write_verified_markers(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        coverage: ReindexCoverage<'_>,
        written: &mut Vec<String>,
    ) -> StorageResult<()> {
        match coverage {
            ReindexCoverage::Tenant => {
                let mut unstamped = Vec::new();
                for (resource_type, params) in self.token_quantity_params(tenant_id, None) {
                    if has_unstamped_quantity_rows(db, tenant_id, &resource_type, &params).await? {
                        unstamped.push(resource_type);
                    }
                }
                if unstamped.is_empty() {
                    // Listed before the write: a failed write may still land.
                    written.push(tenant_marker_id(tenant_id));
                    self.upsert_layout_marker(db, tenant_marker_id(tenant_id), tenant_id, None)
                        .await?;
                } else {
                    warn_unstamped(tenant_id, &unstamped);
                }
            }
            ReindexCoverage::Types(types) => {
                let checked = self.token_quantity_params(tenant_id, Some(types));
                let mut unstamped = Vec::new();
                for resource_type in types {
                    let params = checked
                        .iter()
                        .find(|(checked_type, _)| checked_type == resource_type)
                        .map_or(&[][..], |(_, params)| params.as_slice());
                    if has_unstamped_quantity_rows(db, tenant_id, resource_type, params).await? {
                        unstamped.push(resource_type.clone());
                        continue;
                    }
                    let id = type_marker_id(tenant_id, resource_type);
                    written.push(id.clone());
                    self.upsert_layout_marker(db, id, tenant_id, Some(resource_type))
                        .await?;
                }
                if !unstamped.is_empty() {
                    warn_unstamped(tenant_id, &unstamped);
                }
            }
        }
        Ok(())
    }

    /// The codes of the two-component token+quantity composites of each
    /// resource type (of `types`, or of every registered type), types without
    /// one left out: the parameters the planner reads through `tuple_code`.
    fn token_quantity_params(
        &self,
        tenant_id: &str,
        types: Option<&[String]>,
    ) -> Vec<(String, Vec<String>)> {
        let registry = self.tenant_registry(tenant_id);
        let registry = registry.read();
        let types = types.map_or_else(|| registry.resource_types(), <[String]>::to_vec);
        types
            .into_iter()
            .filter_map(|resource_type| {
                let params: Vec<String> = registry
                    .get_all_params(&resource_type)
                    .iter()
                    .filter(|param| {
                        param.param_type == SearchParamType::Composite
                            && matches!(
                                param.component.as_deref(),
                                Some([token, quantity])
                                    if registry.get_by_url(&token.definition)
                                        .is_some_and(|d| d.param_type == SearchParamType::Token)
                                    && registry.get_by_url(&quantity.definition)
                                        .is_some_and(|d| d.param_type == SearchParamType::Quantity)
                            )
                    })
                    .map(|param| param.code.clone())
                    .collect();
                (!params.is_empty()).then_some((resource_type, params))
            })
            .collect()
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

    async fn upsert_layout_marker(
        &self,
        db: &mongodb::Database,
        id: String,
        tenant_id: &str,
        resource_type: Option<&str>,
    ) -> StorageResult<()> {
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

/// Whether `resource_type` of `tenant_id` holds a quantity row of one of the
/// token+quantity composites `params` without `tuple_code` whose group has a
/// token row with a code: a row the tuple path would miss. A group whose token
/// rows carry no code (a `CodeableConcept` with only `text`) is legitimately
/// left unstamped and is not counted.
///
/// Cost: the first stage reads `idx_search_quantity_v2` over
/// `(tenant_id, resource_type, param_name)` and fetches each quantity row of
/// those parameters to test `tuple_code` (no index can find an absent field
/// under the partial `idx_search_composite_tuple_qty`); each unstamped row then
/// costs one point lookup on `idx_search_composite`. So one document read per
/// composite quantity value of the scope, once per completed `$reindex` that
/// just rewrote every one of those rows: a fraction of the reindex itself.
async fn has_unstamped_quantity_rows(
    db: &mongodb::Database,
    tenant_id: &str,
    resource_type: &str,
    params: &[String],
) -> StorageResult<bool> {
    if params.is_empty() {
        return Ok(false);
    }
    let pipeline = vec![
        doc! { "$match": {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "param_name": { "$in": params },
            "value_quantity_value": { "$exists": true },
            "composite_group": { "$exists": true },
            TUPLE_CODE_FIELD: { "$exists": false },
        } },
        doc! { "$lookup": {
            "from": SEARCH_INDEX_COLLECTION,
            "let": {
                "id": "$resource_id",
                "param": "$param_name",
                "group": "$composite_group",
            },
            "pipeline": [
                { "$match": {
                    "tenant_id": tenant_id,
                    "resource_type": resource_type,
                    "value_token_code": { "$exists": true },
                    "$expr": { "$and": [
                        { "$eq": ["$resource_id", "$$id"] },
                        { "$eq": ["$param_name", "$$param"] },
                        { "$eq": ["$composite_group", "$$group"] },
                    ] },
                } },
                { "$limit": 1 },
                { "$project": { "_id": 1 } },
            ],
            "as": "token",
        } },
        doc! { "$match": { "token.0": { "$exists": true } } },
        doc! { "$limit": 1 },
        doc! { "$project": { "_id": 1 } },
    ];
    let mut cursor = db
        .collection::<Document>(SEARCH_INDEX_COLLECTION)
        .aggregate(pipeline)
        .await
        .map_err(|e| {
            super::storage::internal_error(format!("Failed to check tuple_code coverage: {e}"))
        })?;
    cursor.advance().await.map_err(|e| {
        super::storage::internal_error(format!("Failed to check tuple_code coverage: {e}"))
    })
}

/// Removes the markers just `written` for `tenant_id` when the tenant holds no
/// document in `resources` any more: a tenant purge ran, and nothing else will
/// delete them.
///
/// The read comes after the marker writes, the same proof a `$reindex` page
/// and a `$bulk-submit` batch use against a purge (#2310).
/// `purge_tenant_data` deletes the tenant's `resources` before its markers. A
/// read that finds any document of the tenant ran before that delete was done,
/// so the marker writes before it ran before the delete of the markers, which
/// removes them. A read that finds nothing removes them here. Read on the
/// primary with read concern `local` (see
/// [`super::bulk_submit::primary_selection`]).
///
/// A tenant that is simply empty loses its marker too. That only costs it the
/// tuple path until its next `$reindex`; pair validation is correct.
async fn remove_markers_of_a_purged_tenant(
    db: &mongodb::Database,
    tenant_id: &str,
    written: &[String],
) -> StorageResult<()> {
    if written.is_empty() {
        return Ok(());
    }
    let present = db
        .collection::<Document>(MongoBackend::RESOURCES_COLLECTION)
        .find_one(doc! { "tenant_id": tenant_id })
        .projection(doc! { "_id": 1 })
        .selection_criteria(super::bulk_submit::primary_selection())
        .read_concern(mongodb::options::ReadConcern::local())
        .await
        .map_err(|e| {
            super::storage::internal_error(format!(
                "Failed to check the tenant before keeping its tuple_code markers: {e}"
            ))
        });
    // An unanswered read is treated as a purge: dropping a marker is safe.
    if matches!(present, Ok(Some(_))) {
        return Ok(());
    }
    db.collection::<Document>(LAYOUT_COLLECTION)
        .delete_many(doc! { "_id": { "$in": written } })
        .await
        .map_err(|e| {
            super::storage::internal_error(format!(
                "Failed to remove the tuple_code markers of a purged tenant: {e}"
            ))
        })?;
    tracing::info!(
        tenant_id,
        markers = ?written,
        "found no resource of the tenant after its `$reindex` (a tenant purge ran) or could \
         not check: removed the tuple_code markers the run wrote"
    );
    present.map(|_| ())
}

fn warn_unstamped(tenant_id: &str, resource_types: &[String]) {
    tracing::warn!(
        tenant_id,
        resource_types = ?resource_types,
        "token+quantity composite quantity rows without tuple_code remain after `$reindex` \
         (an older instance or a raw writer wrote them while it ran): no tuple_code marker \
         recorded for these types, so their composite searches keep pair validation; stop \
         the older instances and run `$reindex` again"
    );
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
        // #2394's generation 5 predates `tuple_code`.
        assert!(!markers_trusted(Some(5)));
    }

    /// The hand-written 6 -> 5 rollback script names what this code uses.
    #[test]
    fn the_rollback_script_matches_the_names_in_the_code() {
        let script = include_str!("../../../mongodb/composite-tuple-rollback.mongosh.js");
        assert!(script.contains(&format!(
            "db.{LAYOUT_COLLECTION}.deleteMany({{ layout: \"{LAYOUT_COMPOSITE_TUPLE_CODE}\" }})"
        )));
        let index = super::super::search_index_catalog::COMPOSITE_TUPLE_INDEX;
        assert!(script.contains(&format!(
            "db.{}.dropIndex(\"{index}\")",
            SEARCH_INDEX_COLLECTION
        )));
        assert!(script.contains(&format!(
            "\"search_indexes.generation\": {COMPOSITE_TUPLE_GENERATION} }}"
        )));
        assert!(script.contains(&format!("NumberInt({})", COMPOSITE_TUPLE_GENERATION - 1)));
        assert!(script.contains("process.env.HFS_MONGODB_DATABASE"));
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
