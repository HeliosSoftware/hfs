//! ResourceStorage and VersionedStorage implementations for PostgreSQL.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_fhir::FhirVersion;
use serde_json::Value;

use crate::core::history::{
    DifferentialHistoryProvider, HistoryEntry, HistoryMethod, HistoryPage, HistoryParams,
    InstanceHistoryProvider, SystemHistoryProvider, TypeHistoryProvider,
};
use crate::core::transaction::{
    BundleEntry, BundleEntryResult, BundleMethod, BundleProvider, BundleResult, BundleType,
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
use crate::search::reindex::{ReindexSource, ReindexTarget, ResourcePage};
use crate::tenant::{Operation, TenantContext};
use crate::types::Pagination;
use crate::types::{CursorValue, Page, PageCursor, PageInfo, StoredResource};
use crate::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

use super::PostgresBackend;
use super::cached::{execute_cached, query_opt_cached};
use super::search::writer::{IndexRow, PostgresSearchIndexWriter};

/// Whether a resource being indexed can already have `search_index` rows.
///
/// `Fresh` asserts that nothing is indexed under this id, so the clearing
/// `DELETE` can be skipped — one round trip on paths a bulk import takes for
/// every resource it writes. Two situations establish it:
///
/// - A create that has already inserted the `resources` row — the insert
///   succeeded, so no resource existed under this id, and an index row cannot
///   outlive its resource. Since schema v23 that last part is upheld by the
///   code rather than by a constraint: `purge`, `purge_all` and
///   `purge_tenant_data` each delete the `search_index` rows explicitly before
///   deleting from `resources`. A new deletion path that skipped that would
///   make this assertion false and leave stale rows for a later create to
///   inherit, which is why those call sites carry the obligation in a comment.
/// - A caller that has just run [`PostgresBackend::delete_search_index`], which
///   clears the `search_index` rows and the `resource_fts` row with them.
///
/// Claiming `Fresh` wrongly leaves stale index rows behind — a resource that
/// still matches searches for values it no longer has — so it is an explicit
/// enum rather than a bool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IndexWrite {
    /// Nothing is indexed under this id.
    Fresh,
    /// The resource may already be indexed; clear it first.
    Replace,
}

/// The full-text upsert, shared by the first attempt and the truncating retry.
///
/// `ON CONFLICT` names the columns of the UNIQUE `idx_fts_lookup` created in
/// schema v27. Column order in the target list is the table's, not the index's.
///
/// # The `WHERE` on the `DO UPDATE` is the point
///
/// Without it, every rewrite of a resource replaces this row unconditionally:
/// a new heap tuple, a dead one left behind for autovacuum, and a fresh entry
/// in **both** GIN indexes — whether or not a single lexeme changed. The
/// decomposition on the docstring of `index_fts_content` puts that at 24% (GIN)
/// plus 12% (heap and unique index) of the statement, i.e. **36% of every
/// update's full-text cost is spent writing the row it already had**.
///
/// The `WHERE` makes the write conditional on the vectors actually differing.
/// What it cannot avoid is `to_tsvector` itself — the other 63% — because the
/// comparison needs the new vector to compare against. So the guard converts an
/// unchanged-text update from `tokenise + heap + 2 GIN` into `tokenise +
/// compare`, and leaves a changed-text update paying one extra `tsvector`
/// comparison (an O(lexemes) memcmp over ~5 KB, against the two GIN inserts it
/// is deciding about).
///
/// This is not a benchmark-shaped optimisation, but the benchmark is its best
/// case and that is worth stating plainly: the crud suite's `PUT` sends back a
/// byte-identical body, so **every** one of its 261,459 updates takes the skip.
/// A deployment whose updates change the narrative takes none of it and pays
/// the comparison. The reason to do it anyway is that rewriting an index entry
/// to the same value is never right: it is dead tuples, WAL and autovacuum work
/// with no reader consequence, and `search_index` in the same run left 8.7M dead
/// tuples and four autovacuum passes over a 22M-row table competing for the
/// same four cores.
///
/// `IS DISTINCT FROM`, not `<>`: `narrative_tsvector` is NULL on any row
/// written before the vectors were bound directly (v25), and `NULL <> x` is
/// NULL, which would suppress the update and strand that row unfixed forever.
///
/// Correctness of the skip rests on these two columns being the only thing the
/// statement writes. They are: `narrative_text` and `full_content` are
/// write-only leftovers that this statement has not bound since v25, and the
/// three key columns are what the conflict matched on.
const FTS_UPSERT_SQL: &str = "\
INSERT INTO resource_fts (resource_id, resource_type, tenant_id, narrative_tsvector, content_tsvector) \
VALUES ($1, $2, $3, to_tsvector('english', $4), to_tsvector('english', $5)) \
ON CONFLICT (tenant_id, resource_type, resource_id) DO UPDATE \
SET narrative_tsvector = EXCLUDED.narrative_tsvector, content_tsvector = EXCLUDED.content_tsvector \
WHERE resource_fts.content_tsvector IS DISTINCT FROM EXCLUDED.content_tsvector \
   OR resource_fts.narrative_tsvector IS DISTINCT FROM EXCLUDED.narrative_tsvector";

/// How much text a single retry hands `to_tsvector` after it has refused the
/// whole thing.
///
/// Postgres caps a `tsvector` at 1 MB. The worst *observed* expansion on real
/// data is 1.74x (a Synthea `Provenance`: 751,802 input bytes, a 1,308,960-byte
/// vector). The worst the default parser can produce is bounded at 5x: a
/// hyphenated run yields the compound plus each of its parts, so at most 2x in
/// lexeme bytes, and a lexeme costs a further 4-byte entry plus 2 bytes per
/// position over an input that must spend at least two bytes per lexeme, so at
/// most 3x again. 128 KiB therefore cannot reach the limit even adversarially,
/// and it is far more text than a resource whose `_content` someone searches.
///
/// This is a retry bound, not a cap on indexing: a resource is truncated only
/// after Postgres has already refused the whole thing.
const FTS_MAX_INPUT_BYTES: usize = 128 * 1024;

/// `s` cut to at most `max` bytes, never mid-character.
fn truncate_on_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

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

/// Extracts the `value[x]` payload from a FHIRPath Patch `Parameters.part`
/// entry whose `name` is `"value"`. Returns the value of the first key
/// matching `value[A-Z]…` (e.g. `valueString`, `valueQuantity`,
/// `valueReference`), so every FHIR polymorphic variant is accepted rather
/// than only the handful the patch handler used to special-case.
fn extract_part_value(part: &Value) -> Option<Value> {
    part.as_object()?.iter().find_map(|(k, v)| {
        let suffix = k.strip_prefix("value")?;
        suffix
            .chars()
            .next()?
            .is_ascii_uppercase()
            .then(|| v.clone())
    })
}

#[async_trait]

impl ResourceStorage for PostgresBackend {
    fn backend_name(&self) -> &'static str {
        "postgres"
    }

    async fn readiness_check(&self) -> Result<(), BackendError> {
        <Self as crate::core::Backend>::health_check(self).await
    }

    fn bulk_write_concurrency(&self) -> usize {
        // Bulk seeding is round-trip bound; the pool absorbs parallel writers.
        8
    }

    fn is_cluster_shared(&self) -> bool {
        true
    }

    fn sof_runner(&self) -> Option<std::sync::Arc<dyn crate::core::sof_runner::SofRunner>> {
        use crate::sof::postgres::PgInDbRunner;
        Some(std::sync::Arc::new(PgInDbRunner::new(self.pool())))
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        tenant.check_permission(Operation::Create, resource_type)?;

        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Extract or generate ID
        let id = resource
            .get("id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        // Ensure the resource has correct type and id
        let mut resource = resource;
        if let Some(obj) = resource.as_object_mut() {
            obj.insert(
                "resourceType".to_string(),
                Value::String(resource_type.to_string()),
            );
            obj.insert("id".to_string(), Value::String(id.clone()));
        }

        let now = Utc::now();
        let version_id = "1";
        let fhir_version_str = fhir_version.as_mime_param();
        let is_deleted = false;

        // Resource row, history row and the existence check in one statement.
        //
        // The check used to be its own `SELECT` — a round trip per create, on the
        // path a bulk import takes for every resource — and it was racy besides:
        // two concurrent creates of the same id could both pass it and one would
        // then fail on the primary key with an internal error instead of
        // `AlreadyExists`. `ON CONFLICT DO NOTHING` decides it atomically.
        //
        // On conflict the CTE yields no row, so the history insert selects
        // nothing and the statement reports zero rows affected — one signal for
        // both writes. A soft-deleted resource still occupies its primary key, so
        // it conflicts too, exactly as the old check treated it.
        let inserted = execute_cached(
                &client,
                "WITH ins AS (
                     INSERT INTO resources (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                     ON CONFLICT (tenant_id, resource_type, id) DO NOTHING
                     RETURNING tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version
                 )
                 INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
                 SELECT tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version FROM ins",
                &[&tenant_id, &resource_type, &id, &version_id, &resource, &now, &is_deleted, &fhir_version_str],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to insert resource: {}", e)))?;

        if inserted == 0 {
            return Err(StorageError::Resource(ResourceError::AlreadyExists {
                resource_type: resource_type.to_string(),
                id: id.clone(),
            }));
        }

        // Index the resource for search
        self.index_resource(
            &client,
            tenant_id,
            resource_type,
            &id,
            now,
            IndexWrite::Fresh,
            &resource,
        )
        .await?;

        // An *active* SearchParameter write changes a tenant's overlay: reload
        // the stored cache and drop the per-tenant registries so they rebuild.
        // Draft copies (the seeded spec set) never overlay, so skip them and
        // avoid an O(n²) reload storm during bulk seeding.
        if resource_type == "SearchParameter"
            && crate::search::search_parameter_create_affects_overlay(&resource)
        {
            if let Err(e) = self.reload_stored_cache().await {
                tracing::warn!("SearchParameter cache reload failed: {e}");
            }
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
                let restored = self
                    .restore_deleted(tenant, resource_type, id, resource)
                    .await?;
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
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let row = query_opt_cached(
            &client,
            "SELECT version_id, data, last_updated, is_deleted, deleted_at, fhir_version
                 FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
            &[&tenant_id, &resource_type, &id],
        )
        .await
        .map_err(|e| internal_error(format!("Failed to read resource: {}", e)))?;

        match row {
            Some(row) => {
                let version_id: String = row.get(0);
                let data: Value = row.get(1);
                let last_updated: DateTime<Utc> = row.get(2);
                let is_deleted: bool = row.get(3);
                let deleted_at: Option<DateTime<Utc>> = row.get(4);
                let fhir_version_str: String = row.get(5);

                // If deleted, return Gone error
                if is_deleted {
                    return Err(StorageError::Resource(ResourceError::Gone {
                        resource_type: resource_type.to_string(),
                        id: id.to_string(),
                        deleted_at,
                    }));
                }

                let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                    .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

                Ok(Some(StoredResource::from_storage(
                    resource_type,
                    id,
                    version_id,
                    tenant.tenant_id().clone(),
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
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        let resource_type = current.resource_type();
        tenant.check_permission(Operation::Update, resource_type)?;

        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let id = current.id();

        // The expected version is `current`'s, and the UPDATE below only matches a
        // row that still carries it — so the new version follows from what the
        // caller already read, with no round trip to ask what is stored.
        let expected_version = current.version_id().to_string();
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

        let now = Utc::now();
        let fhir_version_str = current.fhir_version().as_mime_param();

        // Version check, update and history row in one statement. The check used
        // to be a separate `SELECT` — a round trip on every update, and a
        // check-then-act besides: a concurrent writer could bump the version in
        // between and this update would overwrite it while reporting success.
        // Folding the expected version into the `WHERE` makes the two atomic.
        //
        // Zero rows means the update matched nothing; which of the two reasons it
        // was costs a query, but only on the path that is already failing.
        let updated = execute_cached(
                &client,
                "WITH upd AS (
                     UPDATE resources SET version_id = $1, data = $2, last_updated = $3
                     WHERE tenant_id = $4 AND resource_type = $5 AND id = $6
                       AND is_deleted = FALSE AND version_id = $7
                     RETURNING tenant_id, resource_type, id, version_id, data, last_updated, is_deleted
                 )
                 INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
                 SELECT tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, $8 FROM upd",
                &[
                    &new_version_str,
                    &resource,
                    &now,
                    &tenant_id,
                    &resource_type,
                    &id,
                    &expected_version,
                    &fhir_version_str,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to update resource: {}", e)))?;

        if updated == 0 {
            let actual = client
                .query_opt(
                    "SELECT version_id FROM resources
                     WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND is_deleted = FALSE",
                    &[&tenant_id, &resource_type, &id],
                )
                .await
                .map_err(|e| internal_error(format!("Failed to get current version: {}", e)))?;

            return match actual {
                Some(row) => Err(StorageError::Concurrency(
                    ConcurrencyError::VersionConflict {
                        resource_type: resource_type.to_string(),
                        id: id.to_string(),
                        expected_version,
                        actual_version: row.get::<_, String>(0),
                    },
                )),
                None => Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                })),
            };
        }

        // Re-index the resource. `Replace` clears the old `search_index` rows
        // inside the same statement that writes the new ones — the clearing
        // `DELETE` used to be a `delete_search_index` call of its own here, i.e.
        // a second statement and a second round trip binding the same three
        // parameters. The `resource_fts` row stays either way:
        // `index_fts_content` upserts over it, which is one more statement and
        // round trip saved per update.
        self.index_resource(
            &client,
            tenant_id,
            resource_type,
            id,
            now,
            IndexWrite::Replace,
            &resource,
        )
        .await?;

        // A SearchParameter write invalidates the tenant overlays.
        if resource_type == "SearchParameter" {
            if let Err(e) = self.reload_stored_cache().await {
                tracing::warn!("SearchParameter cache reload failed: {e}");
            }
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
        tenant.check_permission(Operation::Delete, resource_type)?;

        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let now = Utc::now();

        // Soft delete the resource and write its deletion history row, in one
        // statement, deriving the tombstone's version from the row itself.
        //
        // Three things used to be separate here: a `SELECT version_id`, an
        // `UPDATE` compare-and-swapping against it, and an `INSERT` of the
        // history row. The `INSERT` was folded into the `UPDATE` first; this
        // folds in the `SELECT` too, so a delete is one round trip where it was
        // three. On the crud suite that is 275,382 statements and 275,382
        // occupied-connection round trips removed from a workload that already
        // demands ~36 cores' worth of PostgreSQL execution on a 4-core host —
        // the round trip, not the 0.06 ms of execution behind it, is what is
        // being bought back.
        //
        // ## Why this is *more* atomic, not less
        //
        // The read-then-CAS it replaces was correct but pessimistic. Under READ
        // COMMITTED, a writer landing between the `SELECT` and the `UPDATE`
        // meant the `version_id = <stale>` predicate matched nothing, and this
        // returned `NotFound` — a 404 for a resource that plainly existed and
        // was live. Computing `version_id + 1` inside the `UPDATE`'s target list
        // removes the window rather than detecting it: PostgreSQL takes the row
        // lock, and if the row was concurrently updated it re-evaluates both the
        // qualifier and the target list against the *committed new* version of
        // the tuple (EvalPlanQual). So the tombstone's version is always exactly
        // one more than whatever version is current at the instant the row is
        // locked, never one more than a version that has since moved on.
        //
        // That is what preserves the primary-key fix the CAS was introduced for.
        // `resource_history` is keyed `PRIMARY KEY (tenant_id, resource_type,
        // id, version_id)` (schema.rs). The failure the CAS prevented was a
        // history row computed from a stale read colliding with one a concurrent
        // writer had already inserted. A version derived from the locked row
        // cannot be stale, so it cannot collide — the invariant is enforced by
        // construction instead of by a guard that has to lose a race to notice.
        //
        // A concurrent *delete* is still resolved correctly and still costs
        // nothing extra: the loser re-evaluates `is_deleted = FALSE` against the
        // committed tombstone, matches no row, and reports `NotFound`, which is
        // exactly what it reported before.
        //
        // ## What changes, stated plainly
        //
        // An unconditional `DELETE` that races a concurrent `UPDATE` now
        // succeeds — deleting the version that writer just committed — where it
        // used to fail with `NotFound`. That is a deliberate correction: FHIR's
        // delete interaction (https://hl7.org/fhir/http.html#delete) carries no
        // precondition of its own, so "delete the current state" is the right
        // reading and the 404 was spurious. Callers that *do* want a
        // precondition use `If-Match`, which is evaluated above this layer.
        //
        // What this does NOT change: that `If-Match` on `DELETE` is evaluated by
        // the REST handler (and by `delete_with_match`) against its own earlier
        // read and is therefore still check-then-act. It was check-then-act
        // before this change too — the CAS removed here guarded the version
        // *this function* had read a microsecond earlier, never the version the
        // caller's precondition was evaluated against — so no precondition
        // guarantee moves in either direction. Making `If-Match` on `DELETE`
        // atomic needs the expected version threaded into this statement, which
        // is a signature change and a separate piece of work.
        //
        // ## The version arithmetic
        //
        // `version_id` is `TEXT`, so the increment is guarded rather than a bare
        // cast: a non-numeric value would make `::bigint` raise 22P02 and turn a
        // delete into a 500. The `CASE` reproduces the Rust it replaces —
        // `current_version.parse::<u64>().unwrap_or(0) + 1` — for every value
        // this server can have written (`'7'` -> 8, `'007'` -> 8, `''` and
        // `'abc'` -> 1, matching `unwrap_or(0)`). `CASE` does not evaluate the
        // branch it did not select, so the cast never runs on a value the regex
        // rejected. Version ids are server-issued decimal integers on every
        // write path in this backend, so the fallback is unreachable in
        // practice and is here only so that it degrades the same way the Rust
        // did rather than differently.
        //
        // `RETURNING` feeds the history row from the tuple just written, so the
        // deletion entry carries the resource's own `data` and `fhir_version`
        // without either making a round trip through the client. As in `create`
        // and `update`, no matching row means the CTE yields nothing, the insert
        // selects nothing, and the statement reports zero rows affected — one
        // signal for both writes. One statement is also one implicit
        // transaction: the tombstone lands with the delete or neither does.
        let updated = execute_cached(
                &client,
                "WITH del AS (
                     UPDATE resources
                     SET is_deleted = TRUE,
                         deleted_at = $1,
                         last_updated = $1,
                         version_id = ((CASE WHEN version_id ~ '^[0-9]+$' THEN version_id::bigint ELSE 0 END) + 1)::text
                     WHERE tenant_id = $2 AND resource_type = $3 AND id = $4
                       AND is_deleted = FALSE
                     RETURNING tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version
                 )
                 INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
                 SELECT tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version FROM del",
                &[&now, &tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to delete resource: {}", e)))?;

        if updated == 0 {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        }

        // Delete search index entries (skip when search is offloaded)
        if !self.is_search_offloaded() {
            execute_cached(
                    &client,
                    "DELETE FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3",
                    &[&tenant_id, &resource_type, &id],
                )
                .await
                .map_err(|e| internal_error(format!("Failed to delete search index: {}", e)))?;
        }

        // A SearchParameter delete invalidates the tenant overlays.
        if resource_type == "SearchParameter" {
            if let Err(e) = self.reload_stored_cache().await {
                tracing::warn!("SearchParameter cache reload failed: {e}");
            }
        }

        Ok(())
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let count: i64 = if let Some(rt) = resource_type {
            let row = client
                .query_one(
                    "SELECT COUNT(*) FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE",
                    &[&tenant_id, &rt],
                )
                .await
                .or_query_error("Failed to count resources")?;
            row.get(0)
        } else {
            let row = client
                .query_one(
                    "SELECT COUNT(*) FROM resources WHERE tenant_id = $1 AND is_deleted = FALSE",
                    &[&tenant_id],
                )
                .await
                .or_query_error("Failed to count resources")?;
            row.get(0)
        };

        Ok(count as u64)
    }

    async fn count_by_day(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        since: DateTime<Utc>,
    ) -> StorageResult<Vec<crate::core::DailyResourceCount>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // `last_updated` is `TIMESTAMPTZ`; normalise to UTC before truncating to a
        // calendar day so buckets are stable regardless of the session time zone.
        // The `(tenant_id, last_updated)` index supports the `>= $3` range scan.
        let rows = client
            .query(
                "SELECT (last_updated AT TIME ZONE 'UTC')::date AS day, COUNT(*)::bigint AS n \
                 FROM resources \
                 WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE \
                   AND last_updated >= $3 \
                 GROUP BY day ORDER BY day",
                &[&tenant_id, &resource_type, &since],
            )
            .await
            .or_query_error("Failed to count resources by day")?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let day: chrono::NaiveDate = row.get(0);
            let n: i64 = row.get(1);
            out.push(crate::core::DailyResourceCount {
                day,
                count: n.max(0) as u64,
            });
        }
        Ok(out)
    }

    async fn count_deltas_by_bucket(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        since: DateTime<Utc>,
        bucket_seconds: i64,
    ) -> StorageResult<Vec<crate::core::ResourceCountDelta>> {
        if bucket_seconds <= 0 {
            return Err(internal_error(
                "count_deltas_by_bucket: bucket_seconds must be positive".to_string(),
            ));
        }
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let since_bound = crate::core::bucket_floor(since, bucket_seconds);

        // Floor each version's `last_updated` to its epoch-aligned bucket:
        // epoch seconds / width, floored, scaled back, then read as a timestamptz.
        // Epoch arithmetic is timezone-independent, so buckets are stable whatever
        // the session TimeZone. The `(tenant_id, last_updated)` history index
        // supports the `>= $3` range scan. Delta rule per the trait doc: creation
        // `+1`, delete `-1`, plain update `0`.
        //
        // `$4::bigint` is cast explicitly: `EXTRACT(EPOCH FROM ...)` is `numeric`, so
        // without it Postgres infers the parameter as `numeric` too and rejects the
        // `i64` we bind.
        let rows = client
            .query(
                "SELECT to_timestamp( \
                          (FLOOR(EXTRACT(EPOCH FROM last_updated) / $4::bigint) * $4::bigint) \
                          ::double precision \
                        ) AS bucket, \
                        SUM(CASE WHEN is_deleted THEN -1 \
                                 WHEN version_id = '1' THEN 1 \
                                 ELSE 0 END)::bigint AS delta \
                 FROM resource_history \
                 WHERE tenant_id = $1 AND resource_type = $2 AND last_updated >= $3 \
                 GROUP BY bucket \
                 HAVING SUM(CASE WHEN is_deleted THEN -1 \
                                 WHEN version_id = '1' THEN 1 \
                                 ELSE 0 END) <> 0 \
                 ORDER BY bucket",
                &[&tenant_id, &resource_type, &since_bound, &bucket_seconds],
            )
            .await
            .or_query_error("Failed to count resource deltas")?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let bucket_start: DateTime<Utc> = row.get(0);
            let delta: i64 = row.get(1);
            out.push(crate::core::ResourceCountDelta {
                bucket_start,
                delta,
            });
        }
        Ok(out)
    }

    async fn activity_histogram(
        &self,
        tenant: &TenantContext,
        since: DateTime<Utc>,
    ) -> StorageResult<Vec<crate::core::ActivityCell>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Normalise to UTC, then EXTRACT weekday (DOW: 0=Sunday..6=Saturday) and
        // hour (0..23) so the grid matches the SQL/Mongo backends and JS's
        // `Date.getDay()`. The `(tenant_id, last_updated)` history index backs
        // the `>= $2` range scan.
        let rows = client
            .query(
                "SELECT EXTRACT(DOW FROM (last_updated AT TIME ZONE 'UTC'))::int AS wd, \
                        EXTRACT(HOUR FROM (last_updated AT TIME ZONE 'UTC'))::int AS hr, \
                        COUNT(*)::bigint AS n \
                 FROM resource_history \
                 WHERE tenant_id = $1 AND last_updated >= $2 \
                 GROUP BY wd, hr",
                &[&tenant_id, &since],
            )
            .await
            .or_query_error("Failed to compute activity histogram")?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let wd: i32 = row.get(0);
            let hr: i32 = row.get(1);
            let n: i64 = row.get(2);
            out.push(crate::core::ActivityCell {
                weekday: wd.clamp(0, 6) as u8,
                hour: hr.clamp(0, 23) as u8,
                count: n.max(0) as u64,
            });
        }
        Ok(out)
    }

    async fn count_all_types(&self, tenant: &TenantContext) -> StorageResult<Vec<(String, u64)>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let rows = client
            .query(
                "SELECT resource_type, COUNT(*)::bigint FROM resources \
                 WHERE tenant_id = $1 AND is_deleted = FALSE \
                 GROUP BY resource_type",
                &[&tenant_id],
            )
            .await
            .or_query_error("Failed to count all types")?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let rt: String = row.get(0);
            let n: i64 = row.get(1);
            out.push((rt, n.max(0) as u64));
        }
        Ok(out)
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
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Bind tenant_id as $1 and each requested type as $2, $3, ...; the type
        // names are bound as parameters, never interpolated into the SQL text.
        let placeholders = (0..resource_types.len())
            .map(|i| format!("${}", i + 2))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT resource_type, COUNT(*)::bigint FROM resources \
             WHERE tenant_id = $1 AND is_deleted = FALSE AND resource_type IN ({}) \
             GROUP BY resource_type",
            placeholders
        );

        // Owned bind values in $1..$n order: tenant first, then the requested types.
        let mut query_params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> =
            Vec::with_capacity(resource_types.len() + 1);
        query_params.push(Box::new(tenant_id.to_string()));
        for rt in resource_types {
            query_params.push(Box::new(rt.to_string()));
        }
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = query_params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client
            .query(&sql, &param_refs)
            .await
            .or_query_error("Failed to count by types")?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let rt: String = row.get(0);
            let n: i64 = row.get(1);
            out.push((rt, n.max(0) as u64));
        }
        Ok(out)
    }

    async fn count_by_tenant(&self) -> StorageResult<Vec<(String, u64)>> {
        // Cross-tenant admin aggregate (see trait docs): no tenant filter.
        let client = self.get_client().await?;
        let rows = client
            .query(
                "SELECT tenant_id, COUNT(*)::bigint FROM resources \
                 WHERE is_deleted = FALSE GROUP BY tenant_id",
                &[],
            )
            .await
            .or_query_error("Failed to count by tenant")?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let tid: String = row.get(0);
            let n: i64 = row.get(1);
            out.push((tid, n.max(0) as u64));
        }
        Ok(out)
    }

    fn supports_tenant_registry(&self) -> bool {
        true
    }

    async fn list_tenants(&self) -> StorageResult<Vec<crate::core::TenantRecord>> {
        let client = self.get_client().await?;
        let rows = client
            .query(
                "SELECT id, display_name, created_at FROM tenants \
                 ORDER BY created_at ASC, id ASC",
                &[],
            )
            .await
            .map_err(|e| internal_error(format!("query list_tenants: {e}")))?;
        Ok(rows
            .iter()
            .map(|row| crate::core::TenantRecord {
                id: row.get(0),
                display_name: row.get(1),
                created_at: row.get(2),
            })
            .collect())
    }

    async fn get_tenant(&self, id: &str) -> StorageResult<Option<crate::core::TenantRecord>> {
        let client = self.get_client().await?;
        let row = client
            .query_opt(
                "SELECT id, display_name, created_at FROM tenants WHERE id = $1",
                &[&id],
            )
            .await
            .map_err(|e| internal_error(format!("query get_tenant: {e}")))?;
        Ok(row.map(|row| crate::core::TenantRecord {
            id: row.get(0),
            display_name: row.get(1),
            created_at: row.get(2),
        }))
    }

    async fn register_tenant(
        &self,
        id: &str,
        display_name: Option<&str>,
    ) -> StorageResult<crate::core::TenantRecord> {
        // Backstop for the canonical tenant-id contract (issue #385). As with
        // SQLite, PostgreSQL keys tenants by an exact-match, case-sensitive
        // `tenant_id` column and has no derivation to protect — this keeps the
        // precondition uniform across every implementation.
        self.ensure_canonical_tenant_id(id)?;
        let client = self.get_client().await?;
        // Plain INSERT so a duplicate id surfaces as a constraint error; the
        // admin handler pre-checks existence and returns 409, so reaching here
        // with a duplicate is a race and a 500 is acceptable.
        let row = client
            .query_one(
                "INSERT INTO tenants (id, display_name) VALUES ($1, $2) \
                 RETURNING id, display_name, created_at",
                &[&id, &display_name],
            )
            .await
            .map_err(|e| internal_error(format!("register_tenant: {e}")))?;
        Ok(crate::core::TenantRecord {
            id: row.get(0),
            display_name: row.get(1),
            created_at: row.get(2),
        })
    }

    async fn deregister_tenant(&self, id: &str) -> StorageResult<bool> {
        crate::tenant::ensure_mutable_tenant(id)?;
        let client = self.get_client().await?;
        let changed = client
            .execute("DELETE FROM tenants WHERE id = $1", &[&id])
            .await
            .map_err(|e| internal_error(format!("deregister_tenant: {e}")))?;
        Ok(changed > 0)
    }

    async fn purge_tenant_data(&self, id: &str) -> StorageResult<u64> {
        crate::tenant::ensure_mutable_tenant(id)?;
        let mut client = self.get_client().await?;
        let tx = client.transaction().await.or_query_error("purge begin")?;
        // Count current-version rows first (soft-deleted included) so we can
        // report what was removed.
        let removed: i64 = tx
            .query_one(
                "SELECT COUNT(*)::bigint FROM resources WHERE tenant_id = $1",
                &[&id],
            )
            .await
            .or_query_error("purge count")?
            .get(0);
        // These deletes are the only thing that removes the dependent rows:
        // `search_index` has no foreign key to `resources` (schema v23) and
        // nothing cascades. Same order as purge/purge_all.
        for sql in [
            "DELETE FROM search_index WHERE tenant_id = $1",
            "DELETE FROM resource_fts WHERE tenant_id = $1",
            "DELETE FROM resource_history WHERE tenant_id = $1",
            "DELETE FROM resources WHERE tenant_id = $1",
        ] {
            tx.execute(sql, &[&id])
                .await
                .or_query_error("purge delete")?;
        }
        // Per-user settings are keyed by user, not tenant, so they are not swept
        // by the deletes above — but a client stores PHI-derived query strings in
        // them, which belong to this tenant (issue #313). Same transaction, so an
        // offboarding cannot half-apply.
        let settings = PostgresBackend::purge_tenant_settings_in_txn(&tx, id).await?;
        tx.commit().await.or_query_error("purge commit")?;
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

// ============================================================================
// Search Index Helpers
// ============================================================================

impl PostgresBackend {
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
    async fn restore_deleted(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        tenant.check_permission(Operation::Update, resource_type)?;

        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let row = client
            .query_opt(
                "SELECT version_id, fhir_version FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND is_deleted = TRUE",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to read deleted resource: {}", e)))?;

        let (deleted_version, fhir_version_str) = match row {
            Some(row) => (row.get::<_, String>(0), row.get::<_, String>(1)),
            None => {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
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

        let now = Utc::now();
        let is_deleted = false;

        execute_cached(
                &client,
                "UPDATE resources
                 SET version_id = $1, data = $2, last_updated = $3, is_deleted = FALSE, deleted_at = NULL
                 WHERE tenant_id = $4 AND resource_type = $5 AND id = $6",
                &[
                    &new_version_str,
                    &resource,
                    &now,
                    &tenant_id,
                    &resource_type,
                    &id,
                ],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to restore resource: {}", e)))?;

        execute_cached(
                &client,
                "INSERT INTO resource_history (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted, fhir_version)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&tenant_id, &resource_type, &id, &new_version_str, &resource, &now, &is_deleted, &fhir_version_str],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to insert restore history: {}", e)))?;

        // The delete dropped the search index entries; rebuild them for the
        // resource that is live again. As in `update`, `Replace` folds the
        // clearing `DELETE` into the insert rather than sending it separately —
        // and it must stay a `Replace`, not a `Fresh`: a resource can be
        // soft-deleted by a path that leaves its rows in place, so this cannot
        // assert that nothing is indexed under the id. The full-text row is
        // upserted over.
        self.index_resource(
            &client,
            tenant_id,
            resource_type,
            id,
            now,
            IndexWrite::Replace,
            &resource,
        )
        .await?;

        // A restored SearchParameter re-enters the tenant overlays.
        if resource_type == "SearchParameter" {
            if let Err(e) = self.reload_stored_cache().await {
                tracing::warn!("SearchParameter cache reload failed: {e}");
            }
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
    /// This method uses the SearchParameterExtractor to dynamically extract
    /// searchable values based on the configured SearchParameterRegistry.
    pub(crate) async fn index_resource(
        &self,
        client: &deadpool_postgres::Client,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        last_updated: DateTime<Utc>,
        mode: IndexWrite,
        resource: &Value,
    ) -> StorageResult<()> {
        // When search is offloaded to a secondary backend, skip local indexing
        if self.is_search_offloaded() {
            return Ok(());
        }

        // `Replace` no longer sends a `DELETE` of its own: the clearing delete
        // rides inside the first insert statement (`INSERT_SQL_REPLACE`), which
        // is one statement and one round trip instead of two, binding the same
        // three parameters once instead of twice. See `replace_rows`.

        // Extract values using the registry-driven extractor
        let mut rows: Vec<IndexRow> = match self
            .tenant_extractor(tenant_id)
            .extract(resource, resource_type)
        {
            Ok(values) => PostgresSearchIndexWriter::build_rows(
                resource_type,
                resource_id,
                last_updated,
                self.index_layout(),
                values,
            ),
            Err(e) => {
                // There used to be a fallback here that indexed `_id` and
                // `_lastUpdated`. Both are now answered from the `resources`
                // columns they restate (see `PARAMS_ANSWERED_FROM_RESOURCES`),
                // so they keep working with no index rows at all and the
                // fallback had nothing left to write. Every other parameter of
                // this resource is unindexed either way — that is what the
                // extraction failure means — so the warning is the whole
                // remaining behaviour.
                tracing::warn!(
                    "Dynamic extraction failed for {}/{}: {}. Resource is stored but not indexed; \
                     `_id` and `_lastUpdated` still resolve from the resources table.",
                    resource_type,
                    resource_id,
                    e
                );
                // No minimal fallback any more: it only ever wrote `_id` and
                // `_lastUpdated`, and both are now answered from the
                // `resources` columns they restate, so there is nothing left
                // for it to write.
                Vec::new()
            }
        };

        // Rows for any `contained[]` entries ride along in the same statement.
        // They used to be one single-row `INSERT` per extracted value — 311,630
        // statements and 311,630 round trips in one 5-minute crud run, 6% of
        // that run's Postgres execution time — even though they are written
        // under the same tenant, type and id as the rows above.
        rows.extend(self.contained_index_rows(tenant_id, resource_type, resource_id, resource));

        let count = rows.len();
        match mode {
            IndexWrite::Fresh => {
                PostgresSearchIndexWriter::insert_rows(
                    client,
                    tenant_id,
                    resource_type,
                    resource_id,
                    &rows,
                )
                .await?
            }
            IndexWrite::Replace => {
                PostgresSearchIndexWriter::replace_rows(
                    client,
                    tenant_id,
                    resource_type,
                    resource_id,
                    &rows,
                )
                .await?
            }
        }
        tracing::debug!(
            "Dynamically indexed {} values for {}/{}",
            count,
            resource_type,
            resource_id
        );

        // Index FTS content for _text and _content searches
        self.index_fts_content(client, tenant_id, resource_type, resource_id, resource)
            .await?;

        Ok(())
    }

    /// Flattens a container's `contained[]` resources into `is_contained = TRUE`
    /// index rows, whose `resource_type` / `resource_id` identify the container.
    ///
    /// Builds rows rather than writing them, so the caller can send them in the
    /// same statement as the container's own rows. A value whose date does not
    /// parse yields no row, exactly as the single-row insert refused to store
    /// one (#494) — so the caller's `$reindex` count now reports rows written
    /// rather than values visited, which is what the container's own rows have
    /// always reported.
    fn contained_index_rows(
        &self,
        tenant_id: &str,
        container_type: &str,
        container_id: &str,
        resource: &Value,
    ) -> Vec<IndexRow> {
        let container = (container_type, container_id);
        let mut rows = Vec::new();
        for contained in self.tenant_extractor(tenant_id).extract_contained(resource) {
            rows.extend(PostgresSearchIndexWriter::build_contained_rows(
                container,
                (&contained.contained_type, &contained.local_id),
                &contained.values,
            ));
        }
        rows
    }

    /// Whether `resource_fts` exists, asked of the catalog at most once.
    ///
    /// This used to be an `information_schema.tables` lookup on every resource
    /// write. `information_schema` views are joins over the system catalogs, so
    /// that is not a free question, and the answer cannot change while the
    /// instance is serving: the table is created by `initialize_schema` and
    /// only migrations touch it, both of which run at startup under an advisory
    /// lock before any request is accepted.
    async fn fts_table_exists(&self, client: &deadpool_postgres::Client) -> StorageResult<bool> {
        if let Some(known) = self.fts_table_exists.get() {
            return Ok(*known);
        }
        let exists = client
            .query_opt(
                "SELECT 1 FROM information_schema.tables WHERE table_name = 'resource_fts'",
                &[],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to check FTS table: {}", e)))?
            .is_some();
        let _ = self.fts_table_exists.set(exists);
        Ok(exists)
    }

    /// Index full-text search content for _text and _content searches.
    ///
    /// Populates the resource_fts table using PostgreSQL tsvector/tsquery.
    ///
    /// The write is an upsert keyed on `idx_fts_lookup`, which schema v27 makes
    /// UNIQUE. Two things follow from that. A rewrite no longer needs its own
    /// `DELETE` — `update` and `restore` leave the row alone and this statement
    /// replaces it in place, which removes one statement and one round trip from
    /// every update. And a
    /// duplicate `resource_fts` row is now impossible: `search_text` /
    /// `search_content` join `resources` to `resource_fts`, so a duplicate used
    /// to return the same resource twice in a `_text` / `_content` page.
    ///
    /// # Where the remaining cost is, measured
    ///
    /// This statement is still the single most expensive one in the crud suite
    /// after `search_index`: 2,597.9 s over 550,764 calls on run 33128380492,
    /// **4.717 ms to write one row**, 20% of the suite. The obvious suspect is
    /// GIN maintenance on `content_tsvector`, which is built from every string
    /// in the resource. It is not the answer. Decomposed on local Postgres 18.6
    /// over a 60,000-resource corpus (mean content 688 bytes, 61 lexemes), four
    /// clients, same statement, arms differing only in what the table carries:
    ///
    /// ```text
    /// arm                                        ms/call    share
    /// full upsert, both GIN indexes               0.1610     100%
    /// same, GIN indexes dropped                   0.1218      76%
    /// `to_tsvector` alone, nothing written        0.1020      63%
    /// ```
    ///
    /// So roughly **63% of this statement is `to_tsvector` running in the
    /// Postgres backend**, 24% is GIN, and 12% is the heap and unique-index
    /// write. And the tokeniser is linear in input bytes, not per call — 120,
    /// 688 and 2,752-byte inputs cost 0.021, 0.076 and 0.319 ms of tokenisation,
    /// a flat ~0.11 us/byte (~9 MB/s) across a 23x range.
    ///
    /// That redirects the obvious optimisations. GIN pending-list tuning has a
    /// 24% ceiling and measured +4.5% at `gin_pending_list_limit = 32MB`, inside
    /// noise; `fastupdate = off` was measured by an earlier seat at **2.24x
    /// slower** and must stay on. Deduplicating words before tokenising was
    /// tried and did nothing (545-byte deduplicated input, 0.0868 ms, against
    /// 688 bytes at 0.0841 ms) — it also changes `ts_rank` ordering for
    /// `_content` while leaving `@@` membership identical, so it costs
    /// semantics for no gain.
    ///
    /// The only lever that moves this is **fewer bytes reaching `to_tsvector`**,
    /// and `_content` is defined as the whole resource, so the bytes are the
    /// feature. What is left is therefore a policy decision rather than an
    /// optimisation: making the `content_tsvector` half optional would return
    /// most of 2,597.9 s to a deployment that does not use `_content`. It is
    /// deliberately not done here — the benchmark issues no `_text` or
    /// `_content` query at all, so switching it off would buy 20% of crud
    /// without buying any user anything, and a silently unindexed `_content`
    /// that answers with an empty page is worse than a slow one. The numbers are
    /// recorded so the choice can be made with them rather than guessed at.
    ///
    /// # A defect this does not fix
    ///
    /// `PostgresTransaction` (`transaction.rs`) writes `resources`,
    /// `resource_history` and `search_index`, and never `resource_fts`. A
    /// resource created through a bundle therefore has no full-text row and is
    /// invisible to `_text` and `_content` on this backend until something
    /// re-indexes it. That is pre-existing and unfiled. It is left alone here
    /// deliberately: the fix adds a `to_tsvector` per bundle entry, which is
    /// cost on the import path, and it belongs with a decision about the
    /// paragraph above rather than inside a performance change.
    async fn index_fts_content(
        &self,
        client: &deadpool_postgres::Client,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> StorageResult<()> {
        if !self.fts_table_exists(client).await? {
            return Ok(());
        }

        // Extract searchable content
        let content = extract_searchable_content(resource);

        if content.is_empty() {
            // Nothing to index. A stored resource always carries at least its
            // `resourceType`, so this is unreachable in practice, but if a
            // rewrite ever did empty a resource out, the previous row has to go
            // rather than survive as a stale match.
            let _ = execute_cached(
                    client,
                    "DELETE FROM resource_fts WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3",
                    &[&tenant_id, &resource_type, &resource_id],
                )
                .await;
            return Ok(());
        }

        // Store the vectors, not their input. `narrative_text` and
        // `full_content` are write-only columns — `_text` and `_content` query
        // `narrative_tsvector` / `content_tsvector` and nothing reads the raw
        // text — so binding them stored the resource's text a second time, with
        // its TOAST compression and WAL, for no reader. Tokenising happens here
        // instead of in a `BEFORE INSERT` trigger; the trigger is dropped in
        // schema v25 because it would otherwise overwrite these vectors with
        // the tsvector of an empty string.
        let result = execute_cached(
            client,
            FTS_UPSERT_SQL,
            &[
                &resource_id,
                &resource_type,
                &tenant_id,
                &content.narrative,
                &content.full_content,
            ],
        )
        .await;

        let err = match result {
            Ok(_) => return Ok(()),
            Err(e) => e,
        };

        // `to_tsvector` refuses to build a vector larger than 1 MB and raises
        // `program_limit_exceeded` when the input demands one. That is not
        // hypothetical: a Synthea `Provenance` lists every resource in the
        // patient's bundle, and two of the 177,612 resources in 150 of the
        // benchmark corpus's patients exceed the limit — 751,802 bytes of
        // content becoming a 1,308,960-byte vector. Before this branch the
        // error surfaced as a 500 and the whole `POST` failed, so an entirely
        // valid resource could not be created at all.
        //
        // Retry once against a truncated input instead. Nothing here runs
        // inside an explicit transaction — the bundle path
        // (`PostgresTransaction`) does not write `resource_fts` at all — so the
        // failed statement leaves the session usable.
        if err.code() != Some(&tokio_postgres::error::SqlState::PROGRAM_LIMIT_EXCEEDED) {
            return Err(internal_error(format!(
                "Failed to insert FTS content: {}",
                err
            )));
        }

        let narrative = truncate_on_char_boundary(&content.narrative, FTS_MAX_INPUT_BYTES);
        let full_content = truncate_on_char_boundary(&content.full_content, FTS_MAX_INPUT_BYTES);
        tracing::warn!(
            "FTS content for {}/{} exceeds PostgreSQL's 1 MB tsvector limit; \
             indexing the first {} bytes of narrative and content only",
            resource_type,
            resource_id,
            FTS_MAX_INPUT_BYTES,
        );
        execute_cached(
            client,
            FTS_UPSERT_SQL,
            &[
                &resource_id,
                &resource_type,
                &tenant_id,
                &narrative,
                &full_content,
            ],
        )
        .await
        .map_err(|e| internal_error(format!("Failed to insert FTS content: {}", e)))?;

        Ok(())
    }

    /// Removes a resource from search entirely: its `search_index` rows and its
    /// `resource_fts` row. Returns how many `search_index` rows went.
    ///
    /// This used to take an `FtsRow` telling it whether to drop the full-text
    /// row, because a *re-indexing* caller — `update`, `restore` — wanted only
    /// the `search_index` half cleared: the full-text write is an upsert against
    /// the UNIQUE `idx_fts_lookup` (schema v27) and replaces the row in place, so
    /// deleting it first was one statement and one round trip of pure waste
    /// (227.3 s over a 5-minute crud run's 192,825 updates, measured on run
    /// 33086933938).
    ///
    /// Those callers no longer come here at all. Clearing the `search_index`
    /// rows is now folded into the statement that writes the new ones
    /// (`IndexWrite::Replace` -> `PostgresSearchIndexWriter::replace_rows`),
    /// which removes a second statement and round trip on top of the one the
    /// `FtsRow` split removed. What is left is the one caller that means
    /// "unindex this resource" — `delete_search_entries`, the `$reindex` clear —
    /// and it always wanted both halves gone, so the parameter had one possible
    /// value and is now implicit.
    ///
    /// The obligation the enum carried is still real and now lives here: a
    /// caller that removes `search_index` rows and does *not* rewrite them must
    /// take the `resource_fts` row too, or the resource stays matchable by
    /// `_text` / `_content` after it has stopped matching everything else.
    pub(crate) async fn delete_search_index(
        &self,
        client: &deadpool_postgres::Client,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<u64> {
        // When search is offloaded to a secondary backend, skip local index cleanup
        if self.is_search_offloaded() {
            return Ok(0);
        }

        // Delete from main search index
        let deleted = execute_cached(
                client,
                "DELETE FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3",
                &[&tenant_id, &resource_type, &resource_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to delete search index: {}", e)))?;

        // The full-text row goes with them.
        let _ = execute_cached(
                client,
                "DELETE FROM resource_fts WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3",
                &[&tenant_id, &resource_type, &resource_id],
            )
            .await;

        Ok(deleted)
    }
}

// ============================================================================
// VersionedStorage Implementation
// ============================================================================

#[async_trait]
impl VersionedStorage for PostgresBackend {
    async fn vread(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        version_id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let row = client
            .query_opt(
                "SELECT data, last_updated, is_deleted, fhir_version
                 FROM resource_history
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND version_id = $4",
                &[&tenant_id, &resource_type, &id, &version_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to read version: {}", e)))?;

        match row {
            Some(row) => {
                let data: Value = row.get(0);
                let last_updated: DateTime<Utc> = row.get(1);
                let is_deleted: bool = row.get(2);
                let fhir_version_str: String = row.get(3);

                // For deleted versions, use last_updated as deleted_at
                let deleted_at = if is_deleted { Some(last_updated) } else { None };

                let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                    .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

                Ok(Some(StoredResource::from_storage(
                    resource_type,
                    id,
                    version_id,
                    tenant.tenant_id().clone(),
                    data,
                    last_updated,
                    last_updated,
                    deleted_at,
                    fhir_version,
                )))
            }
            None => Ok(None),
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

        // Check version match. `expected_version` may arrive in any ETag
        // spelling (`W/"1"`, `"1"`, `1`), so normalise both sides — same as the
        // MongoDB and S3 backends.
        // `expected_version` is the client's `If-Match` field value, which is a
        // LIST and is satisfied when any listed tag matches (issue #311).
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
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check version match
        let row = client
            .query_opt(
                "SELECT version_id FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND is_deleted = FALSE",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get current version: {}", e)))?;

        let current_version = match row {
            Some(row) => row.get::<_, String>(0),
            None => {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
            }
        };

        let expected = normalize_etag(expected_version);
        if !if_match_field_satisfied(expected_version, &current_version) {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: expected.to_string(),
                    actual_version: current_version,
                },
            ));
        }

        // Perform delete
        self.delete(tenant, resource_type, id).await
    }

    async fn list_versions(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Vec<String>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let rows = client
            .query(
                "SELECT version_id FROM resource_history
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3
                 ORDER BY CAST(version_id AS INTEGER) ASC",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to list versions: {}", e)))?;

        let versions: Vec<String> = rows.iter().map(|row| row.get(0)).collect();
        Ok(versions)
    }
}

// ============================================================================
// InstanceHistoryProvider Implementation
// ============================================================================

#[async_trait]
impl InstanceHistoryProvider for PostgresBackend {
    async fn history_instance(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        params: &HistoryParams,
    ) -> StorageResult<HistoryPage> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Build the query with filters
        let mut sql = String::from(
            "SELECT version_id, data, last_updated, is_deleted, fhir_version
             FROM resource_history
             WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
        );
        let mut param_index: usize = 4;
        let mut query_params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = vec![
            Box::new(tenant_id.to_string()),
            Box::new(resource_type.to_string()),
            Box::new(id.to_string()),
        ];

        // Apply deleted filter
        if !params.include_deleted {
            sql.push_str(" AND is_deleted = FALSE");
        }

        // Apply since filter
        if let Some(since) = &params.since {
            sql.push_str(&format!(" AND last_updated >= ${}", param_index));
            query_params.push(Box::new(*since));
            param_index += 1;
        }

        // Apply before filter
        if let Some(before) = &params.before {
            sql.push_str(&format!(" AND last_updated < ${}", param_index));
            query_params.push(Box::new(*before));
            param_index += 1;
        }

        // Apply cursor filter if present
        if let Some(cursor) = params.pagination.cursor_value() {
            if let Some(CursorValue::String(version_str)) = cursor.sort_values().first() {
                if let Ok(version_int) = version_str.parse::<i64>() {
                    sql.push_str(&format!(
                        " AND CAST(version_id AS INTEGER) < ${}",
                        param_index
                    ));
                    query_params.push(Box::new(version_int));
                    param_index += 1;
                }
            }
        }

        // Order by version descending (newest first) and limit
        let limit = params.pagination.count as i64 + 1; // +1 to detect if there are more
        sql.push_str(&format!(
            " ORDER BY CAST(version_id AS INTEGER) DESC LIMIT ${}",
            param_index
        ));
        query_params.push(Box::new(limit));

        // Execute the query
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = query_params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client
            .query(&sql, &param_refs)
            .await
            .or_query_error("Failed to query history")?;

        let mut entries = Vec::new();
        let mut last_version: Option<String> = None;

        for row in &rows {
            // Stop if we've collected enough items (we fetched count+1 to detect more)
            if entries.len() >= params.pagination.count as usize {
                break;
            }

            let version_id: String = row.get(0);
            let data: Value = row.get(1);
            let last_updated: DateTime<Utc> = row.get(2);
            let is_deleted: bool = row.get(3);
            let fhir_version_str: String = row.get(4);

            let deleted_at = if is_deleted { Some(last_updated) } else { None };

            let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

            let resource = StoredResource::from_storage(
                resource_type,
                id,
                &version_id,
                tenant.tenant_id().clone(),
                data,
                last_updated,
                last_updated,
                deleted_at,
                fhir_version,
            );

            // Determine the method based on version and deletion status
            let method = if is_deleted {
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
        let has_more = rows.len() > params.pagination.count as usize;

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
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let row = client
            .query_one(
                "SELECT COUNT(*) FROM resource_history
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .or_query_error("Failed to count history")?;

        let count: i64 = row.get(0);
        Ok(count as u64)
    }

    async fn delete_instance_history(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // First, verify the resource exists
        let exists = client
            .query_opt(
                "SELECT 1 FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to check resource existence: {}", e)))?;

        if exists.is_none() {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        }

        // Get the current version from resources table (to preserve it)
        let current_row = client
            .query_one(
                "SELECT version_id FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get current version: {}", e)))?;

        let current_version: String = current_row.get(0);

        // Delete all history entries EXCEPT the current version
        let deleted = client
            .execute(
                "DELETE FROM resource_history
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND version_id != $4",
                &[&tenant_id, &resource_type, &id, &current_version],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to delete history: {}", e)))?;

        Ok(deleted)
    }

    async fn delete_version(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        version_id: &str,
    ) -> StorageResult<()> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // First, get the current version to ensure we're not deleting it
        let current_row = client
            .query_opt(
                "SELECT version_id FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to get current version: {}", e)))?;

        let current_version = match current_row {
            Some(row) => row.get::<_, String>(0),
            None => {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
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
        let version_exists = client
            .query_opt(
                "SELECT 1 FROM resource_history
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND version_id = $4",
                &[&tenant_id, &resource_type, &id, &version_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to check version existence: {}", e)))?;

        if version_exists.is_none() {
            return Err(StorageError::Resource(ResourceError::VersionNotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
                version_id: version_id.to_string(),
            }));
        }

        // Delete the specific version
        client
            .execute(
                "DELETE FROM resource_history
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND version_id = $4",
                &[&tenant_id, &resource_type, &id, &version_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to delete version: {}", e)))?;

        Ok(())
    }
}

// ============================================================================
// TypeHistoryProvider Implementation
// ============================================================================

#[async_trait]
impl TypeHistoryProvider for PostgresBackend {
    async fn history_type(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        params: &HistoryParams,
    ) -> StorageResult<HistoryPage> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Build the query with filters
        let mut sql = String::from(
            "SELECT id, version_id, data, last_updated, is_deleted, fhir_version
             FROM resource_history
             WHERE tenant_id = $1 AND resource_type = $2",
        );
        let mut param_index: usize = 3;
        let mut query_params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = vec![
            Box::new(tenant_id.to_string()),
            Box::new(resource_type.to_string()),
        ];

        // Apply deleted filter
        if !params.include_deleted {
            sql.push_str(" AND is_deleted = FALSE");
        }

        // Apply since filter
        if let Some(since) = &params.since {
            sql.push_str(&format!(" AND last_updated >= ${}", param_index));
            query_params.push(Box::new(*since));
            param_index += 1;
        }

        // Apply before filter
        if let Some(before) = &params.before {
            sql.push_str(&format!(" AND last_updated < ${}", param_index));
            query_params.push(Box::new(*before));
            param_index += 1;
        }

        // Apply cursor filter if present
        if let Some(cursor) = params.pagination.cursor_value() {
            let sort_values = cursor.sort_values();
            if sort_values.len() >= 2 {
                if let (
                    Some(CursorValue::String(timestamp)),
                    Some(CursorValue::String(resource_id)),
                ) = (sort_values.first(), sort_values.get(1))
                {
                    sql.push_str(&format!(
                        " AND (last_updated < ${}::timestamptz OR (last_updated = ${}::timestamptz AND id < ${}))",
                        param_index, param_index, param_index + 1
                    ));
                    query_params.push(Box::new(timestamp.clone()));
                    query_params.push(Box::new(resource_id.clone()));
                    param_index += 2;
                }
            }
        }

        // Order by last_updated descending (newest first), then by id for consistency
        let limit = params.pagination.count as i64 + 1;
        sql.push_str(&format!(
            " ORDER BY last_updated DESC, id DESC, CAST(version_id AS INTEGER) DESC LIMIT ${}",
            param_index
        ));
        query_params.push(Box::new(limit));

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = query_params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client
            .query(&sql, &param_refs)
            .await
            .or_query_error("Failed to query type history")?;

        let mut entries = Vec::new();
        let mut last_entry: Option<(String, String)> = None; // (last_updated, id)

        for row in &rows {
            if entries.len() >= params.pagination.count as usize {
                break;
            }

            let row_id: String = row.get(0);
            let version_id: String = row.get(1);
            let data: Value = row.get(2);
            let last_updated: DateTime<Utc> = row.get(3);
            let is_deleted: bool = row.get(4);
            let fhir_version_str: String = row.get(5);

            let deleted_at = if is_deleted { Some(last_updated) } else { None };

            let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

            let resource = StoredResource::from_storage(
                resource_type,
                &row_id,
                &version_id,
                tenant.tenant_id().clone(),
                data,
                last_updated,
                last_updated,
                deleted_at,
                fhir_version,
            );

            let method = if is_deleted {
                HistoryMethod::Delete
            } else if version_id == "1" {
                HistoryMethod::Post
            } else {
                HistoryMethod::Put
            };

            last_entry = Some((last_updated.to_rfc3339(), row_id));

            entries.push(HistoryEntry {
                resource,
                method,
                timestamp: last_updated,
            });
        }

        // Determine if there are more results
        let has_more = rows.len() > params.pagination.count as usize;

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
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let row = client
            .query_one(
                "SELECT COUNT(*) FROM resource_history
                 WHERE tenant_id = $1 AND resource_type = $2",
                &[&tenant_id, &resource_type],
            )
            .await
            .or_query_error("Failed to count type history")?;

        let count: i64 = row.get(0);
        Ok(count as u64)
    }
}

// ============================================================================
// SystemHistoryProvider Implementation
// ============================================================================

#[async_trait]
impl SystemHistoryProvider for PostgresBackend {
    async fn history_system(
        &self,
        tenant: &TenantContext,
        params: &HistoryParams,
    ) -> StorageResult<HistoryPage> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Build the query with filters
        let mut sql = String::from(
            "SELECT resource_type, id, version_id, data, last_updated, is_deleted, fhir_version
             FROM resource_history
             WHERE tenant_id = $1",
        );
        let mut param_index: usize = 2;
        let mut query_params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> =
            vec![Box::new(tenant_id.to_string())];

        // Apply deleted filter
        if !params.include_deleted {
            sql.push_str(" AND is_deleted = FALSE");
        }

        // Apply since filter
        if let Some(since) = &params.since {
            sql.push_str(&format!(" AND last_updated >= ${}", param_index));
            query_params.push(Box::new(*since));
            param_index += 1;
        }

        // Apply before filter
        if let Some(before) = &params.before {
            sql.push_str(&format!(" AND last_updated < ${}", param_index));
            query_params.push(Box::new(*before));
            param_index += 1;
        }

        // Apply cursor filter if present
        if let Some(cursor) = params.pagination.cursor_value() {
            let sort_values = cursor.sort_values();
            if sort_values.len() >= 3 {
                if let (
                    Some(CursorValue::String(timestamp)),
                    Some(CursorValue::String(res_type)),
                    Some(CursorValue::String(res_id)),
                ) = (sort_values.first(), sort_values.get(1), sort_values.get(2))
                {
                    sql.push_str(&format!(
                        " AND (last_updated < ${}::timestamptz OR (last_updated = ${}::timestamptz AND (resource_type < ${} OR (resource_type = ${} AND id < ${}))))",
                        param_index, param_index, param_index + 1, param_index + 1, param_index + 2
                    ));
                    query_params.push(Box::new(timestamp.clone()));
                    query_params.push(Box::new(res_type.clone()));
                    query_params.push(Box::new(res_id.clone()));
                    param_index += 3;
                }
            }
        }

        // Order by last_updated descending (newest first)
        let limit = params.pagination.count as i64 + 1;
        sql.push_str(&format!(
            " ORDER BY last_updated DESC, resource_type DESC, id DESC, CAST(version_id AS INTEGER) DESC LIMIT ${}",
            param_index
        ));
        query_params.push(Box::new(limit));

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = query_params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client
            .query(&sql, &param_refs)
            .await
            .or_query_error("Failed to query system history")?;

        let mut entries = Vec::new();
        let mut last_entry: Option<(String, String, String)> = None;

        for row in &rows {
            if entries.len() >= params.pagination.count as usize {
                break;
            }

            let row_resource_type: String = row.get(0);
            let row_id: String = row.get(1);
            let version_id: String = row.get(2);
            let data: Value = row.get(3);
            let last_updated: DateTime<Utc> = row.get(4);
            let is_deleted: bool = row.get(5);
            let fhir_version_str: String = row.get(6);

            let deleted_at = if is_deleted { Some(last_updated) } else { None };

            let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

            let resource = StoredResource::from_storage(
                &row_resource_type,
                &row_id,
                &version_id,
                tenant.tenant_id().clone(),
                data,
                last_updated,
                last_updated,
                deleted_at,
                fhir_version,
            );

            let method = if is_deleted {
                HistoryMethod::Delete
            } else if version_id == "1" {
                HistoryMethod::Post
            } else {
                HistoryMethod::Put
            };

            last_entry = Some((last_updated.to_rfc3339(), row_resource_type, row_id));

            entries.push(HistoryEntry {
                resource,
                method,
                timestamp: last_updated,
            });
        }

        let has_more = rows.len() > params.pagination.count as usize;

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
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let row = client
            .query_one(
                "SELECT COUNT(*) FROM resource_history WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await
            .or_query_error("Failed to count system history")?;

        let count: i64 = row.get(0);
        Ok(count as u64)
    }
}

// ============================================================================
// DifferentialHistoryProvider Implementation
// ============================================================================

#[async_trait]
impl DifferentialHistoryProvider for PostgresBackend {
    async fn modified_since(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
        since: DateTime<Utc>,
        pagination: &Pagination,
    ) -> StorageResult<Page<StoredResource>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Build query for current versions of resources modified since timestamp
        let mut sql = String::from(
            "SELECT resource_type, id, version_id, data, last_updated, fhir_version
             FROM resources
             WHERE tenant_id = $1 AND last_updated > $2 AND is_deleted = FALSE",
        );
        let mut param_index: usize = 3;
        let mut query_params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> =
            vec![Box::new(tenant_id.to_string()), Box::new(since)];

        // Filter by resource type if specified
        if let Some(rt) = resource_type {
            sql.push_str(&format!(" AND resource_type = ${}", param_index));
            query_params.push(Box::new(rt.to_string()));
            param_index += 1;
        }

        // Apply cursor filter if present
        if let Some(cursor) = pagination.cursor_value() {
            let sort_values = cursor.sort_values();
            if sort_values.len() >= 2 {
                if let (Some(CursorValue::String(timestamp)), Some(CursorValue::String(res_id))) =
                    (sort_values.first(), sort_values.get(1))
                {
                    sql.push_str(&format!(
                        " AND (last_updated > ${}::timestamptz OR (last_updated = ${}::timestamptz AND id > ${}))",
                        param_index, param_index, param_index + 1
                    ));
                    query_params.push(Box::new(timestamp.clone()));
                    query_params.push(Box::new(res_id.clone()));
                    param_index += 2;
                }
            }
        }

        // Order by last_updated ascending (oldest first for sync)
        let limit = pagination.count as i64 + 1;
        sql.push_str(&format!(
            " ORDER BY last_updated ASC, id ASC LIMIT ${}",
            param_index
        ));
        query_params.push(Box::new(limit));

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = query_params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client
            .query(&sql, &param_refs)
            .await
            .map_err(|e| internal_error(format!("Failed to query modified resources: {}", e)))?;

        let mut resources = Vec::new();
        let mut last_entry: Option<(String, String)> = None;

        for row in &rows {
            if resources.len() >= pagination.count as usize {
                break;
            }

            let row_resource_type: String = row.get(0);
            let row_id: String = row.get(1);
            let version_id: String = row.get(2);
            let data: Value = row.get(3);
            let last_updated: DateTime<Utc> = row.get(4);
            let fhir_version_str: String = row.get(5);

            let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

            let resource = StoredResource::from_storage(
                &row_resource_type,
                &row_id,
                &version_id,
                tenant.tenant_id().clone(),
                data,
                last_updated,
                last_updated,
                None,
                fhir_version,
            );

            last_entry = Some((last_updated.to_rfc3339(), row_id));
            resources.push(resource);
        }

        let has_more = rows.len() > pagination.count as usize;

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

// ============================================================================
// PurgableStorage Implementation
// ============================================================================

#[async_trait]
impl PurgableStorage for PostgresBackend {
    async fn purge(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Check if resource exists (in any state)
        let exists = client
            .query_opt(
                "SELECT 1 FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to check resource: {}", e)))?;

        if exists.is_none() {
            // Also check history in case it was already purged from main table
            let history_exists = client
                .query_opt(
                    "SELECT 1 FROM resource_history WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                    &[&tenant_id, &resource_type, &id],
                )
                .await
                .or_query_error("Failed to check history")?;

            if history_exists.is_none() {
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
            }
        }

        // Removing the index rows is REQUIRED, not just ordering: `search_index`
        // has no foreign key to `resources` (schema v23), so nothing cascades.
        client
            .execute(
                "DELETE FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .or_query_error("Failed to purge search index")?;

        // Delete from FTS table
        let _ = client
            .execute(
                "DELETE FROM resource_fts WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await;

        // Delete from history table (before resources due to FK)
        client
            .execute(
                "DELETE FROM resource_history WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .or_query_error("Failed to purge resource history")?;

        // Delete from resources table
        client
            .execute(
                "DELETE FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND id = $3",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .or_query_error("Failed to purge resource")?;

        Ok(())
    }

    async fn purge_all(&self, tenant: &TenantContext, resource_type: &str) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Count how many we're about to delete
        let row = client
            .query_one(
                "SELECT COUNT(DISTINCT id) FROM resources WHERE tenant_id = $1 AND resource_type = $2",
                &[&tenant_id, &resource_type],
            )
            .await
            .or_query_error("Failed to count resources")?;
        let count: i64 = row.get(0);

        // Removing the index rows is REQUIRED, not just ordering: `search_index`
        // has no foreign key to `resources` (schema v23), so nothing cascades.
        client
            .execute(
                "DELETE FROM search_index WHERE tenant_id = $1 AND resource_type = $2",
                &[&tenant_id, &resource_type],
            )
            .await
            .or_query_error("Failed to purge search index")?;

        // Delete from FTS table
        let _ = client
            .execute(
                "DELETE FROM resource_fts WHERE tenant_id = $1 AND resource_type = $2",
                &[&tenant_id, &resource_type],
            )
            .await;

        // Delete from history table
        client
            .execute(
                "DELETE FROM resource_history WHERE tenant_id = $1 AND resource_type = $2",
                &[&tenant_id, &resource_type],
            )
            .await
            .or_query_error("Failed to purge resource history")?;

        // Delete from resources table
        client
            .execute(
                "DELETE FROM resources WHERE tenant_id = $1 AND resource_type = $2",
                &[&tenant_id, &resource_type],
            )
            .await
            .or_query_error("Failed to purge resources")?;

        Ok(count as u64)
    }
}

// ============================================================================
// ConditionalStorage Implementation
// ============================================================================

// Helper function to parse simple search parameters
// Supports basic formats like: identifier=X, _id=Y, name=Z
fn parse_simple_search_params(params: &str) -> Vec<(String, String)> {
    params
        .split('&')
        .filter_map(|pair| {
            let parts: Vec<&str> = pair.splitn(2, '=').collect();
            if parts.len() == 2 {
                Some((parts[0].to_string(), parts[1].to_string()))
            } else {
                None
            }
        })
        .collect()
}

#[async_trait]
impl ConditionalStorage for PostgresBackend {
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

    async fn conditional_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        search_params: &str,
        upsert: bool,
        fhir_version: FhirVersion,
    ) -> StorageResult<ConditionalUpdateResult> {
        // Find matching resources based on search parameters
        let matches = self
            .find_matching_resources(tenant, resource_type, search_params)
            .await?;

        match matches.len() {
            0 => {
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
                // Exactly one match - update it (preserves existing FHIR version)
                let existing = matches.into_iter().next().unwrap();
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
    ) -> StorageResult<ConditionalDeleteResult> {
        // Find matching resources based on search parameters
        let matches = self
            .find_matching_resources(tenant, resource_type, search_params)
            .await?;

        match matches.len() {
            0 => {
                // No match
                Ok(ConditionalDeleteResult::NoMatch)
            }
            1 => {
                // Exactly one match - delete it
                let existing = matches.into_iter().next().unwrap();
                self.delete(tenant, resource_type, existing.id()).await?;
                Ok(ConditionalDeleteResult::Deleted)
            }
            n => {
                // Multiple matches - error condition
                Ok(ConditionalDeleteResult::MultipleMatches(n))
            }
        }
    }

    async fn conditional_patch(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
        patch: &crate::core::PatchFormat,
    ) -> StorageResult<crate::core::ConditionalPatchResult> {
        use crate::core::{ConditionalPatchResult, PatchFormat};

        // Find matching resources based on search parameters
        let matches = self
            .find_matching_resources(tenant, resource_type, search_params)
            .await?;

        match matches.len() {
            0 => Ok(ConditionalPatchResult::NoMatch),
            1 => {
                // Exactly one match - apply the patch
                let existing = matches.into_iter().next().unwrap();
                let current_content = existing.content().clone();

                // Apply the patch based on format
                let patched_content = match patch {
                    PatchFormat::JsonPatch(patch_doc) => {
                        self.apply_json_patch(&current_content, patch_doc)?
                    }
                    PatchFormat::FhirPathPatch(patch_params) => {
                        self.apply_fhirpath_patch(&current_content, patch_params)?
                    }
                    PatchFormat::MergePatch(merge_doc) => {
                        self.apply_merge_patch(&current_content, merge_doc)
                    }
                };

                // Update the resource with the patched content
                let updated = self.update(tenant, &existing, patched_content).await?;
                Ok(ConditionalPatchResult::Patched(updated))
            }
            n => Ok(ConditionalPatchResult::MultipleMatches(n)),
        }
    }
}

impl PostgresBackend {
    /// Find resources matching the given search parameters.
    ///
    /// Uses the SearchProvider implementation to leverage the pre-computed search index.
    async fn find_matching_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params_str: &str,
    ) -> StorageResult<Vec<StoredResource>> {
        // Parse search parameters into (name, value) pairs
        let parsed_params = parse_simple_search_params(search_params_str);

        if parsed_params.is_empty() {
            // No search params means match all - but for conditional ops this is unusual
            return Ok(Vec::new());
        }

        // Build SearchParameter objects by looking up types from the registry
        let search_params = self.build_search_parameters(tenant, resource_type, &parsed_params)?;

        // Build a SearchQuery
        let query = SearchQuery {
            resource_type: resource_type.to_string(),
            parameters: search_params,
            count: Some(1000),
            ..Default::default()
        };

        // Use the SearchProvider implementation
        let result = <Self as SearchProvider>::search(self, tenant, &query).await?;

        Ok(result.resources.items)
    }

    /// Builds SearchParameter objects from parsed (name, value) pairs.
    fn build_search_parameters(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        params: &[(String, String)],
    ) -> StorageResult<Vec<SearchParameter>> {
        let registry_arc = self.tenant_registry(tenant.tenant_id().as_str());
        let registry = registry_arc.read();
        let mut search_params = Vec::with_capacity(params.len());

        for (name, value) in params {
            let param_type = self
                .lookup_param_type(&registry, resource_type, name)
                .unwrap_or_else(|| crate::search::fallback_param_type(name));

            search_params.push(SearchParameter {
                name: name.clone(),
                param_type,
                modifier: None,
                values: vec![SearchValue::parse(value)],
                chain: vec![],
                components: vec![],
            });
        }

        Ok(search_params)
    }

    /// Looks up a search parameter type from the registry.
    fn lookup_param_type(
        &self,
        registry: &crate::search::SearchParameterRegistry,
        resource_type: &str,
        param_name: &str,
    ) -> Option<SearchParamType> {
        if let Some(def) = registry.get_param(resource_type, param_name) {
            return Some(def.param_type);
        }
        if let Some(def) = registry.get_param("Resource", param_name) {
            return Some(def.param_type);
        }
        None
    }

    // ========================================================================
    // Patch Helper Methods
    // ========================================================================

    /// Applies a JSON Patch (RFC 6902) to a resource.
    fn apply_json_patch(&self, resource: &Value, patch_doc: &Value) -> StorageResult<Value> {
        use crate::error::ValidationError;

        let patch: json_patch::Patch = serde_json::from_value(patch_doc.clone()).map_err(|e| {
            StorageError::Validation(ValidationError::InvalidResource {
                message: format!("Invalid JSON Patch document: {}", e),
                details: vec![],
            })
        })?;

        let mut patched = resource.clone();
        json_patch::patch(&mut patched, &patch).map_err(|e| {
            StorageError::Validation(ValidationError::InvalidResource {
                message: format!("Failed to apply JSON Patch: {}", e),
                details: vec![],
            })
        })?;

        Ok(patched)
    }

    /// Applies a FHIRPath Patch to a resource.
    fn apply_fhirpath_patch(&self, resource: &Value, patch_params: &Value) -> StorageResult<Value> {
        use crate::error::ValidationError;

        let parameter = patch_params.get("parameter").and_then(|p| p.as_array());
        if parameter.is_none() {
            return Err(StorageError::Validation(ValidationError::InvalidResource {
                message: "FHIRPath Patch must have a 'parameter' array".to_string(),
                details: vec![],
            }));
        }

        let mut patched = resource.clone();

        for operation in parameter.unwrap() {
            let parts = operation.get("part").and_then(|p| p.as_array());
            if parts.is_none() {
                continue;
            }

            let mut op_type = None;
            let mut op_path = None;
            let mut op_name = None;
            let mut op_value = None;

            for part in parts.unwrap() {
                match part.get("name").and_then(|n| n.as_str()) {
                    Some("type") => {
                        op_type = part
                            .get("valueCode")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                    Some("path") => {
                        op_path = part
                            .get("valueString")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                    Some("name") => {
                        op_name = part
                            .get("valueString")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                    Some("value") => {
                        op_value = extract_part_value(part);
                    }
                    _ => {}
                }
            }

            match op_type.as_deref() {
                Some("replace") => {
                    if let (Some(path), Some(value)) = (&op_path, &op_value) {
                        self.fhirpath_replace(&mut patched, path, value)?;
                    }
                }
                Some("add") => {
                    if let (Some(path), Some(name), Some(value)) = (&op_path, &op_name, &op_value) {
                        self.fhirpath_add(&mut patched, path, name, value)?;
                    }
                }
                Some("delete") => {
                    if let Some(path) = &op_path {
                        self.fhirpath_delete(&mut patched, path)?;
                    }
                }
                _ => {
                    // Unsupported operation type - skip
                }
            }
        }

        Ok(patched)
    }

    /// Helper for FHIRPath replace operation.
    fn fhirpath_replace(
        &self,
        resource: &mut Value,
        path: &str,
        value: &Value,
    ) -> StorageResult<()> {
        let parts: Vec<&str> = path.split('.').collect();
        if parts.len() == 2 {
            if let Some(obj) = resource.as_object_mut() {
                obj.insert(parts[1].to_string(), value.clone());
            }
        }
        Ok(())
    }

    /// Helper for FHIRPath add operation.
    fn fhirpath_add(
        &self,
        resource: &mut Value,
        path: &str,
        name: &str,
        value: &Value,
    ) -> StorageResult<()> {
        let parts: Vec<&str> = path.split('.').collect();
        if parts.len() == 1
            && parts[0]
                == resource
                    .get("resourceType")
                    .and_then(|r| r.as_str())
                    .unwrap_or("")
        {
            if let Some(obj) = resource.as_object_mut() {
                obj.insert(name.to_string(), value.clone());
            }
        }
        Ok(())
    }

    /// Helper for FHIRPath delete operation.
    fn fhirpath_delete(&self, resource: &mut Value, path: &str) -> StorageResult<()> {
        let parts: Vec<&str> = path.split('.').collect();
        if parts.len() == 2 {
            if let Some(obj) = resource.as_object_mut() {
                obj.remove(parts[1]);
            }
        }
        Ok(())
    }

    /// Applies a JSON Merge Patch (RFC 7386) to a resource.
    fn apply_merge_patch(&self, resource: &Value, merge_doc: &Value) -> Value {
        let mut patched = resource.clone();
        json_patch::merge(&mut patched, merge_doc);
        patched
    }
}

// ============================================================================
// BundleProvider Implementation
// ============================================================================

#[async_trait]
impl BundleProvider for PostgresBackend {
    /// PostgreSQL provides real ACID transactions; `PostgresBackend` also
    /// implements [`TransactionProvider`](crate::core::TransactionProvider),
    /// and `process_transaction` runs inside one, so a failure unwinds
    /// completely.
    fn supports_atomic_transactions(&self) -> bool {
        true
    }

    async fn process_transaction(
        &self,
        tenant: &TenantContext,
        entries: Vec<BundleEntry>,
        fhir_version: helios_fhir::FhirVersion,
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

        // `create` no longer sends its insert on the spot — the transaction
        // batches consecutive creates and flushes them together, which is what
        // takes a 1,632-entry import bundle from 3,264 statements to 26. A
        // conflict is therefore discovered at flush time, after later entries
        // have already been processed, so the entry index has to be recovered
        // rather than read off the loop counter. This maps the transaction's
        // n-th `create` call back to the entry that made it.
        let mut create_entry_index: Vec<usize> = Vec::with_capacity(entries.len());

        // Build a map of fullUrl -> assigned reference for reference resolution
        let mut reference_map: HashMap<String, String> = HashMap::new();

        // Make entries mutable for reference resolution
        let mut entries = entries;

        // Process each entry within the transaction
        for (idx, entry) in entries.iter_mut().enumerate() {
            // Resolve references in this entry's resource before processing
            if let Some(ref mut resource) = entry.resource {
                resolve_bundle_references(resource, &reference_map);
            }

            let creates_before = tx.creates_seen();
            let result = self.process_bundle_entry_tx(&mut tx, entry).await;
            for _ in creates_before..tx.creates_seen() {
                create_entry_index.push(idx);
            }

            match result {
                Ok(entry_result) => {
                    if entry_result.status >= 400 {
                        error_info = Some((
                            idx,
                            format!("Entry failed with status {}", entry_result.status),
                        ));
                        break;
                    }

                    // If this was a create (POST) and we have a fullUrl, record the mapping
                    if entry.method == BundleMethod::Post {
                        if let Some(ref full_url) = entry.full_url {
                            if let Some(ref location) = entry_result.location {
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
                    error_info = Some(attribute_entry_error(&tx, &create_entry_index, idx, &e));
                    break;
                }
            }
        }

        // Send whatever is still buffered before committing, so a conflict in
        // the last batch is reported as the bundle error it is — with the
        // offending entry's index — rather than as a bare commit failure.
        if error_info.is_none() {
            if let Err(e) = tx.flush().await {
                let last = entries.len().saturating_sub(1);
                error_info = Some(attribute_entry_error(&tx, &create_entry_index, last, &e));
            }
        }

        // Handle error or commit
        if let Some((index, message)) = error_info {
            let _ = Box::new(tx).rollback().await;
            return Err(TransactionError::BundleError { index, message });
        }

        // Commit the transaction
        Box::new(tx)
            .commit()
            .await
            .map_err(|e| TransactionError::RolledBack {
                reason: format!("Commit failed: {}", e),
            })?;

        Ok(BundleResult {
            bundle_type: BundleType::Transaction,
            entries: results,
        })
    }
}

/// Names the bundle entry responsible for a transaction error.
///
/// A conflict found while flushing buffered creates belongs to the entry whose
/// `create` produced the row, not to whichever entry happened to be in flight
/// when the flush ran. Everything else belongs to the entry that raised it.
fn attribute_entry_error(
    tx: &super::transaction::PostgresTransaction,
    create_entry_index: &[usize],
    fallback: usize,
    error: &StorageError,
) -> (usize, String) {
    match tx.deferred_conflict() {
        Some(conflict) => (
            create_entry_index
                .get(conflict.ordinal)
                .copied()
                .unwrap_or(fallback),
            format!(
                "Entry processing failed: {}/{} already exists",
                conflict.resource_type, conflict.id
            ),
        ),
        None => (fallback, format!("Entry processing failed: {}", error)),
    }
}

impl PostgresBackend {
    /// Process a single bundle entry within a transaction.
    async fn process_bundle_entry_tx(
        &self,
        tx: &mut super::transaction::PostgresTransaction,
        entry: &BundleEntry,
    ) -> StorageResult<BundleEntryResult> {
        use crate::core::transaction::Transaction;

        match entry.method {
            BundleMethod::Get => {
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

                let created = tx.create(&resource_type, resource).await?;
                Ok(BundleEntryResult::created(created))
            }
            BundleMethod::Put => {
                let resource = entry.resource.clone().ok_or_else(|| {
                    StorageError::Validation(crate::error::ValidationError::MissingRequiredField {
                        field: "resource".to_string(),
                    })
                })?;

                let (resource_type, id) = self.parse_url(&entry.url)?;

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
                        Ok(BundleEntryResult::ok(updated))
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

                // Honor `ifMatch` on DELETE — previously ignored here, so a
                // client asking to delete only the version it had reviewed could
                // destroy a concurrent amendment with no 412. The read is
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
                // PATCH is not fully implemented yet
                Ok(BundleEntryResult::error(
                    501,
                    serde_json::json!({
                        "resourceType": "OperationOutcome",
                        "issue": [{"severity": "error", "code": "not-supported", "diagnostics": "PATCH not implemented in transaction bundles"}]
                    }),
                ))
            }
        }
    }

    /// Parse a FHIR URL into resource type and ID.
    fn parse_url(&self, url: &str) -> StorageResult<(String, String)> {
        let path = url
            .strip_prefix("http://")
            .or_else(|| url.strip_prefix("https://"))
            .map(|s| s.find('/').map(|i| &s[i..]).unwrap_or(s))
            .unwrap_or(url);

        let path = path.trim_start_matches('/');
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

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
fn resolve_bundle_references(
    value: &mut serde_json::Value,
    reference_map: &std::collections::HashMap<String, String>,
) {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            if let Some(Value::String(ref_str)) = map.get("reference") {
                if ref_str.starts_with("urn:uuid:") {
                    if let Some(resolved) = reference_map.get(ref_str) {
                        map.insert("reference".to_string(), Value::String(resolved.clone()));
                    }
                }
            }
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

// ============================================================================
// ReindexSource Implementation — PostgreSQL is a primary, so it is where
// resources are read from during a reindex.
// ============================================================================

#[async_trait]
impl ReindexSource for PostgresBackend {
    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let rows = client
            .query(
                "SELECT DISTINCT resource_type FROM resources WHERE tenant_id = $1 AND is_deleted = FALSE",
                &[&tenant_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to query resource types: {}", e)))?;

        let types: Vec<String> = rows.iter().map(|row| row.get(0)).collect();
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
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Parse cursor if provided (format: "last_updated|id")
        let (cursor_ts, cursor_id) = if let Some(c) = cursor {
            let parts: Vec<&str> = c.split('|').collect();
            if parts.len() == 2 {
                let ts = DateTime::parse_from_rfc3339(parts[0])
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| internal_error(format!("Invalid cursor timestamp: {}", e)))?;
                (Some(ts), Some(parts[1].to_string()))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        let rows = if let (Some(ts), Some(id)) = (&cursor_ts, &cursor_id) {
            client
                .query(
                    "SELECT id, version_id, data, last_updated, fhir_version FROM resources
                     WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE
                     AND (last_updated > $3 OR (last_updated = $3 AND id > $4))
                     ORDER BY last_updated ASC, id ASC LIMIT $5",
                    &[
                        &tenant_id,
                        &resource_type,
                        ts,
                        &id.as_str(),
                        &(limit as i64),
                    ],
                )
                .await
                .map_err(|e| internal_error(format!("Failed to fetch resources page: {}", e)))?
        } else {
            client
                .query(
                    "SELECT id, version_id, data, last_updated, fhir_version FROM resources
                     WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE
                     ORDER BY last_updated ASC, id ASC LIMIT $3",
                    &[&tenant_id, &resource_type, &(limit as i64)],
                )
                .await
                .map_err(|e| internal_error(format!("Failed to fetch resources page: {}", e)))?
        };

        let resources: Vec<StoredResource> = rows
            .iter()
            .map(|row| {
                let id: String = row.get(0);
                let version_id: String = row.get(1);
                let data: Value = row.get(2);
                let last_updated: DateTime<Utc> = row.get(3);
                let fhir_version_str: String = row.get(4);
                let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                    .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

                StoredResource::from_storage(
                    resource_type,
                    id,
                    version_id,
                    tenant.tenant_id().clone(),
                    data,
                    last_updated,
                    last_updated,
                    None,
                    fhir_version,
                )
            })
            .collect();

        // Determine next cursor
        let next_cursor = if resources.len() == limit as usize {
            resources
                .last()
                .map(|r| format!("{}|{}", r.last_modified().to_rfc3339(), r.id()))
        } else {
            None
        };

        Ok(ResourcePage {
            resources,
            next_cursor,
        })
    }
}

// ============================================================================
// ReindexTarget Implementation — PostgreSQL keeps search entries in its own
// `search_index` table, so it is also a writer and can reindex itself.
// ============================================================================

#[async_trait]
impl ReindexTarget for PostgresBackend {
    async fn delete_search_entries(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<u64> {
        let client = self.get_client().await?;
        // `$reindex` may clear without rewriting, so the full-text row goes
        // too; `write_search_entries` puts it back when the rewrite follows.
        self.delete_search_index(
            &client,
            tenant.tenant_id().as_str(),
            resource_type,
            resource_id,
        )
        .await
    }

    async fn write_search_entries(
        &self,
        tenant: &TenantContext,
        resource: &StoredResource,
    ) -> StorageResult<usize> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let resource_type = resource.resource_type();
        let resource_id = resource.id();
        let content = resource.content();

        // Use the dynamic extraction over the tenant's registry
        let values = self
            .tenant_extractor(tenant_id)
            .extract(content, resource_type)
            .map_err(|e| internal_error(format!("Search parameter extraction failed: {}", e)))?;

        let mut rows = PostgresSearchIndexWriter::build_rows(
            resource_type,
            resource_id,
            resource.last_modified(),
            self.index_layout(),
            values,
        );

        // Re-index contained resources too, so `$reindex` rebuilds `_contained`
        // search entries — in the same statement as the resource's own rows.
        rows.extend(self.contained_index_rows(tenant_id, resource_type, resource_id, content));

        let count = rows.len();
        PostgresSearchIndexWriter::insert_rows(
            &client,
            tenant_id,
            resource_type,
            resource_id,
            &rows,
        )
        .await?;

        // Rebuild the full-text row as well. `run_reindex` deletes each
        // resource's search entries first (`delete_search_entries` ->
        // `delete_search_index`), and that drops the `resource_fts` row; without
        // this call nothing put it back, so `$reindex` silently disabled
        // `_text`/`_content` on every reindex, with or without `clear_existing`.
        // Same defect as the SQLite side — this is not PostgreSQL-specific.
        //
        // Not counted in `count`, which reports `search_index` entries only.
        // The write is an upsert, so it is idempotent regardless of what ran
        // before it — a reindex can find a row present.
        self.index_fts_content(&client, tenant_id, resource_type, resource_id, content)
            .await?;

        Ok(count)
    }

    async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let deleted = client
            .execute(
                "DELETE FROM search_index WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await
            .or_query_error("Failed to clear search index")?;

        // Also clear FTS entries
        let _ = client
            .execute(
                "DELETE FROM resource_fts WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await;

        Ok(deleted)
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

// ============================================================================
// FTS Content Extraction (local copy to avoid cross-feature dependency on sqlite)
// ============================================================================

/// Content extracted from a resource for full-text search.
struct SearchableContent {
    narrative: String,
    full_content: String,
}

impl SearchableContent {
    fn is_empty(&self) -> bool {
        self.narrative.is_empty() && self.full_content.is_empty()
    }
}

/// Extracts searchable text content from a FHIR resource.
///
/// `full_content` backs `_content`, which FHIR defines as a search over *the
/// entire content of the resource* — so it must be a superset of `_text`, the
/// narrative-only search. `collect_strings` skips the `div` key deliberately, to
/// keep raw XHTML markup (`div`, `p`, `xmlns`, attribute values) out of the
/// index; that left the narrative missing from `_content` altogether, so a term
/// that appeared only in `text.div` was findable through `_text` and invisible
/// to `_content`. Appending the already-stripped narrative restores the
/// superset relationship without indexing the markup, and matches SQLite, which
/// has always had the narrative in `_content`. `data` stays excluded — base64
/// attachment blobs are not text.
fn extract_searchable_content(resource: &Value) -> SearchableContent {
    let narrative = extract_narrative(resource);
    let mut full_content = extract_all_strings(resource);
    if !narrative.is_empty() {
        if !full_content.is_empty() {
            full_content.push(' ');
        }
        full_content.push_str(&narrative);
    }
    SearchableContent {
        narrative,
        full_content,
    }
}

/// Extracts narrative text from resource.text.div, stripping HTML tags.
fn extract_narrative(resource: &Value) -> String {
    resource
        .get("text")
        .and_then(|t| t.get("div"))
        .and_then(|d| d.as_str())
        .map(strip_html_tags)
        .unwrap_or_default()
}

/// Strips HTML tags from a string, returning plain text.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;

    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                result.push(' ');
            }
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }

    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extracts all string values from a JSON value recursively.
fn extract_all_strings(value: &Value) -> String {
    let mut parts = Vec::new();
    collect_strings(value, None, &mut parts);
    parts.join(" ")
}

/// The keys whose value is a machine identifier or a link rather than text.
///
/// `id` is the resource's (or an element's) identity, `reference` a link to
/// another resource, `fullUrl` the same link in a Bundle entry, and
/// `versionId` the row's version. FHIR answers all four with dedicated,
/// exactly-matching search machinery — `_id`, reference-typed parameters,
/// `_include` / `_revinclude`, chaining — none of which goes anywhere near
/// `resource_fts`.
const OPAQUE_ID_KEYS: [&str; 4] = ["id", "reference", "fullUrl", "versionId"];

/// Collects the strings that make up `_content`.
///
/// `key` is the JSON object key the value hangs off (inherited through arrays,
/// so `Provenance.target[].reference` is still seen as a `reference`).
///
/// A value at one of [`OPAQUE_ID_KEYS`] that contains a canonical UUID is left
/// out. That is a deliberate narrowing of `_content`, and it is what makes the
/// full-text write path cheap: a GIN index costs roughly what its *entry tree*
/// costs, and a UUID never seen before is a brand-new key inserted at a random
/// position in that tree — page split, WAL, cache miss — whereas an ordinary
/// word only appends to a posting list that already exists. Measured over
/// 45,000 crud-shaped resources (the benchmark's own nine seed resources with
/// server-assigned ids and cross-references), the vectors held **224,438**
/// distinct lexemes before this filter and **333** after: 99.85% of the entry
/// tree was ids and links. Over 177,603 resources of the Synthea corpus the
/// content GIN index falls from 88 MB to 30 MB, index buffer touches per
/// insert from 27.8 to 9.1, and WAL records per insert from 11.1 to 5.2.
///
/// What can no longer be found: `_content=<a uuid>` and
/// `_content=Patient/<a uuid>` — searching the free-text index for a resource
/// id or a literal reference. `_id`, `subject=Patient/<id>`, `_include` and
/// reverse chaining all still answer those, exactly rather than through a
/// stemming text query. What is still found: everything a person wrote or a
/// terminology defines — names, addresses, narrative, codes, display strings,
/// `text` elements, and `Identifier.value`, which is where an MRN, an NPI or an
/// accession number lives. Only the four keys above are touched, and only when
/// the value is UUID-shaped, so a server that assigns readable ids
/// (`Patient/patient-smith`) keeps indexing them.
///
/// The SQLite backend has always excluded `id`, `reference`, `meta`,
/// `extension`, `url` and every `http(s)://` string from `_content`
/// (`sqlite/search/fts.rs`). This narrows the *existing* divergence rather than
/// creating one: PostgreSQL's `_content` remains a strict superset of
/// SQLite's.
///
/// # An absolute `http(s)` URL is not text either
///
/// The UUID rule above removed the *entry tree* cost — the one-off keys that a
/// GIN index pays a page split for. It did not remove the **tokeniser** cost,
/// and that is where what is left of this write path lives: an earlier seat
/// decomposed `INSERT INTO resource_fts` on this corpus and found **63% of the
/// statement is `to_tsvector` running in the Postgres backend**, 24% GIN, 12%
/// heap and unique index. The tokeniser is linear in input *bytes*, so the only
/// lever on that 63% is fewer bytes.
///
/// A FHIR resource's bytes are not evenly split between prose and machinery. In
/// the benchmark's nine crud seed resources, `_content` is 6,114 bytes, of which
/// **2,899 (47%) are absolute `http(s)` URLs** — `system` values, profile
/// canonicals, `CodeSystem` URLs. They are terminology addresses, not words, and
/// Postgres's default parser is unusually expensive on them: it recognises
/// `protocol`, `url`, `host` and `url_path` tokens, so one 54-byte system URL
/// costs several tokens and several stemmer calls to produce lexemes nobody
/// types into a `_content` query.
///
/// Measured on PostgreSQL 18.6 over exactly those nine resources, 2,000
/// repetitions × 3 interleaved rounds, arms differing only in whether
/// `http(s)`-prefixed strings are collected:
///
/// ```text
/// arm                     us/resource   lexemes   tsvector bytes
/// with URLs (before)         209.45         416          9,594
/// without URLs (this)        119.07         307          4,972
///                            -43.1%       -26.2%         -48.2%
/// ```
///
/// All three columns matter and they hit different thirds of the statement:
/// −43% of the tokeniser's 63%, −26% of the GIN's 24% (GIN cost is per entry),
/// and −48% of the vector that the heap and the unique index have to carry.
///
/// The two right-hand columns are counts and are load-independent. The
/// microseconds are not: the box was carrying another seat's release build at
/// the time and every absolute figure on it is inflated by roughly 2-3x against
/// the ~0.11 us/byte an earlier seat measured on a quiet one. What transfers is
/// the **ratio**, which is what the arms were interleaved to isolate.
///
/// What can no longer be found: `_content=http://loinc.org`, i.e. asking the
/// free-text index for a code system's *address*. `system` is still indexed
/// exactly by every token parameter (`code=http://loinc.org|8302-2`,
/// `code:in=…`), which is the machinery FHIR provides for that question and
/// which answers it without stemming. The `Identifier.value`, the display, the
/// `text` and the narrative that sit next to the URL are all untouched.
///
/// This is the *same* rule SQLite has always applied, so it removes a
/// divergence: `_content` on the two backends now agrees about URLs. PostgreSQL
/// still indexes `meta`, `extension` and `url`-keyed non-URL strings that SQLite
/// drops, so it remains the superset.
///
/// # It needs no migration, and deliberately does not get one
///
/// `resource_fts` stores the vectors, not their input (`narrative_text` and
/// `full_content` have been write-only since v25 and are left unbound), so no
/// SQL migration can recompute a row: the extraction is Rust-side. Rows written
/// before this change keep their URL lexemes. That direction is safe — a wider
/// vector *over*-matches, so an old row answers `_content=http://loinc.org` and
/// a new one does not, and no query that used to return a resource stops
/// returning it for any other term. `$reindex` regenerates the rows for a
/// deployment that wants them uniform. Bumping the schema version would not
/// help, because there is nothing the migration could execute.
fn collect_strings(value: &Value, key: Option<&str>, parts: &mut Vec<String>) {
    match value {
        Value::String(s) => {
            if s.is_empty() {
                return;
            }
            if key.is_some_and(|k| OPAQUE_ID_KEYS.contains(&k)) && contains_uuid(s) {
                return;
            }
            if is_absolute_http_url(s) {
                return;
            }
            parts.push(s.clone());
        }
        Value::Object(map) => {
            for (key, val) in map {
                if key == "div" || key == "data" {
                    continue;
                }
                collect_strings(val, Some(key.as_str()), parts);
            }
        }
        Value::Array(arr) => {
            for val in arr {
                collect_strings(val, key, parts);
            }
        }
        _ => {}
    }
}

/// Whether `s` *is* an absolute `http` or `https` URL.
///
/// Prefix-anchored, not "contains": a sentence that mentions a link — a
/// narrative, a `text` element, an `Identifier.value` that happens to embed one
/// — is prose and stays in `_content`. Only a value whose entire content is the
/// URL is dropped, which is the shape a `system`, a `profile` canonical or a
/// `CodeSystem.url` takes.
///
/// `urn:` is deliberately *not* included even though `urn:oid:` and `urn:uuid:`
/// systems are just as machine-shaped. It is only 80 bytes of the crud corpus's
/// 6,114 (1.3%), SQLite does not drop it, and adding it would re-open the
/// divergence this closes for no measurable gain.
fn is_absolute_http_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

/// Whether `s` contains a canonical UUID: 8-4-4-4-12 hexadecimal digits.
///
/// The match has to be delimited on both sides — the byte before and the byte
/// after may not be a hex digit or a hyphen — so a longer hexadecimal run is
/// never mistaken for a UUID it happens to contain.
fn contains_uuid(s: &str) -> bool {
    const UUID_LEN: usize = 36;
    let b = s.as_bytes();
    if b.len() < UUID_LEN {
        return false;
    }
    for i in 0..=(b.len() - UUID_LEN) {
        if i > 0 && (b[i - 1].is_ascii_hexdigit() || b[i - 1] == b'-') {
            continue;
        }
        let end = i + UUID_LEN;
        if end < b.len() && (b[end].is_ascii_hexdigit() || b[end] == b'-') {
            continue;
        }
        if is_uuid(&b[i..end]) {
            return true;
        }
    }
    false
}

/// Whether exactly these 36 bytes are `8-4-4-4-12` hexadecimal digits.
fn is_uuid(b: &[u8]) -> bool {
    debug_assert_eq!(b.len(), 36);
    for (i, byte) in b.iter().enumerate() {
        let expect_hyphen = matches!(i, 8 | 13 | 18 | 23);
        if expect_hyphen {
            if *byte != b'-' {
                return false;
            }
        } else if !byte.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod fts_extraction_tests {
    use super::*;
    use serde_json::json;

    fn patient_with_narrative() -> Value {
        json!({
            "resourceType": "Patient",
            "name": [{"family": "Purgetest"}],
            "text": {
                "status": "generated",
                "div": "<div xmlns=\"http://www.w3.org/1999/xhtml\"><p>Assessment: \
                        Zebracrossingdiagnosis.</p></div>"
            },
            "photo": [{"contentType": "image/png", "data": "iVBORw0KGgoAAAANSUhEUg=="}]
        })
    }

    #[test]
    fn content_includes_the_narrative() {
        // Regression: `_content` is "the entire content of the resource", so a
        // term that only appears in the narrative must be reachable through it.
        // `collect_strings` skips the `div` key, which used to drop the
        // narrative from `full_content` entirely — `_text` found the term,
        // `_content` did not.
        let content = extract_searchable_content(&patient_with_narrative());

        assert!(
            content.narrative.contains("Zebracrossingdiagnosis"),
            "narrative: {}",
            content.narrative
        );
        assert!(
            content.full_content.contains("Zebracrossingdiagnosis"),
            "_content must be a superset of _text: {}",
            content.full_content
        );
        assert!(
            content.full_content.contains("Purgetest"),
            "the non-narrative fields must still be there: {}",
            content.full_content
        );
    }

    #[test]
    fn content_excludes_markup_and_binary_payloads() {
        // The narrative goes in stripped, not raw: tag and attribute noise would
        // make every resource match `xmlns` or `div`. Base64 attachment data
        // stays out for the same reason plus index size.
        let content = extract_searchable_content(&patient_with_narrative());

        assert!(
            !content.full_content.contains("xmlns"),
            "{}",
            content.full_content
        );
        assert!(
            !content.full_content.contains("<p>"),
            "{}",
            content.full_content
        );
        assert!(
            !content.full_content.contains("iVBORw0KGgo"),
            "{}",
            content.full_content
        );
    }

    #[test]
    fn narrative_only_resource_is_not_empty() {
        // `index_fts_content` returns early on `is_empty()`; a resource whose
        // only text is its narrative must still be indexed.
        let content = extract_searchable_content(&json!({
            "resourceType": "Binary",
            "text": {"div": "<div><p>Solitary</p></div>"}
        }));

        assert!(!content.is_empty());
        assert!(content.full_content.contains("Solitary"));
    }

    #[test]
    fn uuid_shapes_are_recognised_only_when_delimited() {
        let u = "a424cdce-e753-faae-a2c4-9fe945223809";
        assert!(contains_uuid(u));
        assert!(contains_uuid(&format!("Patient/{u}")));
        assert!(contains_uuid(&format!("urn:uuid:{u}")));
        assert!(contains_uuid(&format!("Patient/{u}/_history/3")));
        assert!(contains_uuid(&format!(
            "https://example.org/fhir/Patient/{u}"
        )));

        // Not a UUID: wrong group lengths, a non-hex digit, and a longer
        // hexadecimal run that merely contains 36 matching bytes.
        assert!(!contains_uuid("a424cdce-e753-faae-a2c4-9fe94522380"));
        assert!(!contains_uuid("a424cdce-e753-faae-a2c4-9fe9452238zz"));
        assert!(!contains_uuid(&format!("ff{u}")));
        assert!(!contains_uuid(&format!("{u}ff")));
        assert!(!contains_uuid(&format!("{u}-0000")));
        assert!(!contains_uuid("patient-smith-1"));
        assert!(!contains_uuid(""));
    }

    #[test]
    fn content_drops_ids_and_references_but_keeps_identifier_values() {
        // The narrowing this makes to `_content`: the resource's own id and the
        // links it holds are not text, and indexing them is what made the
        // full-text write path expensive. Everything a person wrote — including
        // an `Identifier.value`, which is where an MRN lives even when the MRN
        // happens to be a UUID — stays.
        let id = "a424cdce-e753-faae-a2c4-9fe945223809";
        let mrn = "7ec34f99-4ae7-bb4c-cc1c-4ab0bc19784f";
        let content = extract_searchable_content(&json!({
            "resourceType": "Observation",
            "id": id,
            "meta": {"versionId": "550e8400-e29b-41d4-a716-446655440000"},
            "subject": {"reference": format!("Patient/{id}"), "display": "Pok428 Metz686"},
            "identifier": [{"system": "http://hospital.example.org", "value": mrn}],
            "code": {"text": "Pain severity"}
        }));

        assert!(
            !content.full_content.contains(id),
            "{}",
            content.full_content
        );
        assert!(
            !content.full_content.contains("550e8400"),
            "{}",
            content.full_content
        );
        assert!(
            content.full_content.contains(mrn),
            "{}",
            content.full_content
        );
        assert!(
            content.full_content.contains("Pok428 Metz686"),
            "{}",
            content.full_content
        );
        assert!(
            content.full_content.contains("Pain severity"),
            "{}",
            content.full_content
        );
        // The `system` URL next to the MRN is now dropped as well — see
        // `is_absolute_http_url`. `identifier=http://hospital.example.org|<mrn>`
        // still answers exactly for it; the MRN itself, above, stays in
        // `_content`.
        assert!(
            !content.full_content.contains("http://hospital.example.org"),
            "{}",
            content.full_content
        );
    }

    #[test]
    fn content_drops_whole_urls_but_keeps_prose_that_mentions_one() {
        // The rule is prefix-anchored on purpose: a `system` or a profile
        // canonical is a machine address and goes, but a narrative or a `text`
        // that happens to quote a link is prose and stays. Dropping on
        // "contains" would silently take the sentence with it.
        let content = extract_searchable_content(&json!({
            "resourceType": "Observation",
            "meta": {"profile": ["http://hl7.org/fhir/StructureDefinition/vitalsigns"]},
            "code": {
                "coding": [{"system": "http://loinc.org", "code": "8302-2",
                            "display": "Body Height"}],
                "text": "Body Height"
            },
            "note": [{"text": "Method described at https://example.org/protocols/height"}]
        }));

        assert!(
            !content.full_content.contains("http://loinc.org"),
            "{}",
            content.full_content
        );
        assert!(
            !content.full_content.contains("StructureDefinition/vitalsigns"),
            "{}",
            content.full_content
        );
        assert!(
            content
                .full_content
                .contains("Method described at https://example.org/protocols/height"),
            "{}",
            content.full_content
        );
        assert!(content.full_content.contains("Body Height"));
        assert!(content.full_content.contains("8302-2"));
    }

    #[test]
    fn urn_systems_are_still_indexed() {
        // `urn:` is deliberately outside the rule; see `is_absolute_http_url`.
        let content = extract_searchable_content(&json!({
            "resourceType": "Patient",
            "identifier": [{"system": "urn:oid:2.16.840.1.113883.4.1", "value": "999-11-2222"}]
        }));

        assert!(content.full_content.contains("urn:oid:2.16.840.1.113883.4.1"));
        assert!(content.full_content.contains("999-11-2222"));
    }

    #[test]
    fn readable_ids_and_references_are_still_indexed() {
        // Only UUID-shaped values are dropped, so a server that assigns
        // human-readable ids keeps `_content` finding them.
        let content = extract_searchable_content(&json!({
            "resourceType": "Observation",
            "id": "obs-smith-2024",
            "subject": {"reference": "Patient/patient-smith"}
        }));

        assert!(content.full_content.contains("obs-smith-2024"));
        assert!(content.full_content.contains("Patient/patient-smith"));
    }

    #[test]
    fn references_inside_arrays_are_dropped_too() {
        // `collect_strings` inherits the key through arrays, which is what makes
        // `Provenance.target[].reference` — the single largest producer of
        // one-off GIN keys in the corpus — take the same path.
        let content = extract_searchable_content(&json!({
            "resourceType": "Provenance",
            "target": [
                {"reference": "Encounter/834823d5-da27-4685-ba3b-5bd316e92682"},
                {"reference": "Claim/9fe94522-e753-faae-a2c4-3809a424cdce"}
            ],
            "activity": {"text": "Record authoring"}
        }));

        assert!(
            !content.full_content.contains("834823d5"),
            "{}",
            content.full_content
        );
        assert!(
            !content.full_content.contains("9fe94522"),
            "{}",
            content.full_content
        );
        assert!(content.full_content.contains("Record authoring"));
    }

    #[test]
    fn truncation_never_splits_a_character() {
        let s = "ä".repeat(100);
        for max in 0..s.len() + 2 {
            let cut = truncate_on_char_boundary(&s, max);
            assert!(cut.len() <= max.min(s.len()));
            assert!(s.starts_with(cut));
        }
        assert_eq!(truncate_on_char_boundary("abc", 10), "abc");
    }
}
