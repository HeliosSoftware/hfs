//! #1500: MongoDB's `ReindexSource::fetch_resources_by_ids` override.
//!
//! Before the fix, MongoDB has no override and falls back to the trait
//! default (`crates/persistence/src/search/reindex.rs`), which scans the
//! whole type page by page via `fetch_resources_page` looking for the wanted
//! ids. That default is already functionally correct — tenant/type scope,
//! dedup and deleted-id handling all fall out of it "for free" — so a plain
//! before/after value assertion never goes red on its own; only a
//! profiler-based check of *how* Mongo satisfies the call can tell a
//! full-type scan apart from a direct `$in`-bounded point lookup hinted to
//! `idx_resources_identity`. This file is a `#[path]`-included child module
//! of `mongodb_tests.rs`, not a standalone test binary — see
//! `reindex_id_walk.rs`'s header for the arrangement, whose profiler test
//! (`mongodb_reindex_id_walk_pages_plan_without_a_blocking_sort`,
//! `reindex_id_walk.rs:707+`) this file's plan tests are modeled on.

use super::*;
use helios_persistence::error::StorageResult;
use helios_persistence::search::ReindexSource;
use helios_persistence::types::StoredResource;
use mongodb::bson::{Bson, DateTime as BsonDateTime, Document};

// ===========================================================================
// Harness
// ===========================================================================

/// Creates `count` Patients (ids `"{prefix}-0000"`..) through the ordinary
/// write path, so each gets real `created_at`/`last_updated` stamps.
async fn seed_patients(
    backend: &MongoBackend,
    tenant: &TenantContext,
    count: usize,
    prefix: &str,
) -> Vec<String> {
    let mut ids = Vec::with_capacity(count);
    for i in 0..count {
        let id = format!("{prefix}-{i:04}");
        backend
            .create(
                tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": id,
                    "name": [{ "family": id }],
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        ids.push(id);
    }
    ids
}

/// Inserts `count` minimal, already-current Patient rows directly into
/// `resources`, bypassing `backend.create` — used only by the chunking test,
/// where seeding 2000+ resources one write at a time would make the test
/// needlessly slow. The row shape mirrors exactly what `create` writes
/// (`storage.rs`'s `resource_doc`), so `parse_history_row` decodes it the
/// same way.
async fn seed_patients_raw(
    backend: &MongoBackend,
    tenant: &TenantContext,
    count: usize,
    prefix: &str,
) -> Vec<String> {
    let db = backend.get_database().await.unwrap();
    let resources = db.collection::<Document>("resources");
    let tenant_id = tenant.tenant_id().as_str();
    let now = BsonDateTime::from_millis(chrono::Utc::now().timestamp_millis());
    let fhir_version = FhirVersion::default().as_mime_param().to_string();

    let mut ids = Vec::with_capacity(count);
    let mut docs = Vec::with_capacity(count);
    for i in 0..count {
        let id = format!("{prefix}-{i:05}");
        docs.push(doc! {
            "tenant_id": tenant_id,
            "resource_type": "Patient",
            "id": &id,
            "version_id": "1",
            "data": { "resourceType": "Patient", "id": &id },
            "created_at": now,
            "last_updated": now,
            "is_deleted": false,
            "deleted_at": Bson::Null,
            "fhir_version": &fhir_version,
        });
        ids.push(id);
    }
    resources.insert_many(docs).await.unwrap();
    ids
}

/// Turns on `{profile: 2}` and reports whether the server allowed it — some
/// managed/shared `HFS_TEST_MONGODB_URL` targets refuse it, in which case the
/// caller must skip its own plan assertions without failing (mirrors
/// `reindex_id_walk.rs:740-748`).
async fn enable_profiling(db: &mongodb::Database) -> bool {
    db.run_command(doc! { "profile": 2_i32 }).await.is_ok()
}

/// `system.profile` entries for `find` commands against `resources`, read
/// after profiling is turned back off (mirrors `reindex_id_walk.rs:761-774`).
async fn resources_find_profile_entries(db: &mongodb::Database) -> Vec<Document> {
    use futures::stream::TryStreamExt;
    let profile: mongodb::Collection<Document> = db.collection("system.profile");
    profile
        .find(doc! {
            "ns": format!("{}.resources", db.name()),
            "op": "query",
            "command.find": "resources",
        })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

/// `true` when a profiled `resources` find's command filters `id` with
/// `$in` — the shape only [`MongoBackend::fetch_resources_by_ids`]'s override
/// produces; the trait default's scan queries filter on `last_updated`
/// and/or a keyset `id: {"$gt": ...}` instead.
fn filters_id_with_in(command: &Document) -> bool {
    command
        .get_document("filter")
        .ok()
        .and_then(|f| f.get_document("id").ok())
        .is_some_and(|id_filter| id_filter.contains_key("$in"))
}

// ===========================================================================
// Red: the override's query shape (#1500)
// ===========================================================================

/// The override must run a `$in`-bounded, unsorted lookup hinted to
/// `idx_resources_identity` — not the default's `last_updated`-filtered,
/// `id`-sorted scan of the whole type. Every one of those properties (the
/// `id.$in` filter, no `last_updated` filter, no `sort`, the hint, and
/// `docsExamined` bounded by the request size) fails against today's code,
/// which has no override and falls back to the trait default.
#[tokio::test]
async fn mongodb_reindex_fetch_by_ids_uses_in_lookup_hinted_to_identity() {
    let Some(backend) = create_backend("reindex_fetch_ids_plan").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("fetch-ids-plan");
    let ids = seed_patients(&backend, &tenant, 20, "plan").await;
    let wanted: Vec<String> = ids.iter().take(5).cloned().collect();

    let db = backend.get_database().await.unwrap();
    let profiled = enable_profiling(&db).await;
    if !profiled {
        assert!(
            test_mongo_url().is_some(),
            "the harness's own Mongo container must allow {{profile: 2}}"
        );
        eprintln!(
            "mongodb_reindex_fetch_by_ids_uses_in_lookup_hinted_to_identity: server refused \
             {{profile: 2}} (likely a managed/shared HFS_TEST_MONGODB_URL); skipping the plan \
             assertions"
        );
    }

    let found = backend
        .fetch_resources_by_ids(&tenant, "Patient", &wanted)
        .await
        .unwrap();

    let _ = db.run_command(doc! { "profile": 0_i32 }).await;

    assert_eq!(
        found.len(),
        wanted.len(),
        "all requested live ids should be found"
    );

    if !profiled {
        return;
    }

    let entries = resources_find_profile_entries(&db).await;
    assert!(
        !entries.is_empty(),
        "fetch_resources_by_ids should have issued at least one profiled find against resources"
    );

    for entry in &entries {
        let command = entry
            .get_document("command")
            .expect("profiled find has a command");

        assert!(
            filters_id_with_in(command),
            "fetch_resources_by_ids must filter id with $in, got command {command:?}"
        );
        let filter = command.get_document("filter").unwrap();
        assert!(
            !filter.contains_key("last_updated"),
            "fetch_resources_by_ids must not filter on last_updated (that's the scan's job), \
             got filter {filter:?}"
        );
        assert_eq!(
            command.get_str("hint").ok(),
            Some("idx_resources_identity"),
            "fetch_resources_by_ids must hint idx_resources_identity, got command {command:?}"
        );
        assert!(
            command.get_document("sort").is_err(),
            "fetch_resources_by_ids must not sort (order is unspecified per the trait \
             contract), got command {command:?}"
        );

        let docs_examined = entry
            .get_i64("docsExamined")
            .or_else(|_| entry.get_i32("docsExamined").map(i64::from))
            .expect("profiled find reports docsExamined");
        assert!(
            docs_examined <= wanted.len() as i64,
            "fetch_resources_by_ids examined {docs_examined} docs for {} wanted ids \
             (should be index-bounded, not a type scan)",
            wanted.len()
        );
    }
}

/// Requests spanning more than one `REINDEX_IDS_QUERY_SIZE` (1000) chunk must
/// produce that many separate `$in` lookups, all found. Today's code has no
/// chunking at all — it never issues an `id.$in` query in the first place —
/// so this is red for the same reason as the test above, not merely a
/// performance check.
#[tokio::test]
async fn mongodb_reindex_fetch_by_ids_chunks_at_query_size() {
    let Some(backend) = create_backend("reindex_fetch_ids_chunk").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("fetch-ids-chunk");
    let ids = seed_patients_raw(&backend, &tenant, 2001, "chunk").await;

    let db = backend.get_database().await.unwrap();
    let profiled = enable_profiling(&db).await;
    if !profiled {
        assert!(
            test_mongo_url().is_some(),
            "the harness's own Mongo container must allow {{profile: 2}}"
        );
        eprintln!(
            "mongodb_reindex_fetch_by_ids_chunks_at_query_size: server refused {{profile: 2}} \
             (likely a managed/shared HFS_TEST_MONGODB_URL); skipping the plan assertions"
        );
    }

    let found = backend
        .fetch_resources_by_ids(&tenant, "Patient", &ids)
        .await
        .unwrap();

    let _ = db.run_command(doc! { "profile": 0_i32 }).await;

    assert_eq!(found.len(), ids.len(), "every seeded id should be found");

    if !profiled {
        return;
    }

    let entries = resources_find_profile_entries(&db).await;
    let by_ids_entries: Vec<&Document> = entries
        .iter()
        .filter(|entry| entry.get_document("command").is_ok_and(filters_id_with_in))
        .collect();
    assert_eq!(
        by_ids_entries.len(),
        3,
        "2001 ids at REINDEX_IDS_QUERY_SIZE=1000 should chunk into exactly 3 $in finds, got \
         {} matching entries out of {} total: {entries:?}",
        by_ids_entries.len(),
        entries.len()
    );
}

// ===========================================================================
// Pins: functional contract (scope, dedup, deleted ids, empty input) — the
// default already gets these right, so they pass before the fix too; they
// pin the behavior so the override cannot regress it.
// ===========================================================================

/// Scope, currency and deleted-id handling: a same-id different-type
/// resource, a same-id different-tenant resource, a soft-deleted id, and an
/// unknown id are all absent; an updated resource comes back with its
/// current content and version, not a stale copy.
#[tokio::test]
async fn mongodb_reindex_fetch_by_ids_scope_pin() {
    let Some(backend) = create_backend("reindex_fetch_ids_scope").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("fetch-ids-scope");
    let other_tenant = create_tenant("fetch-ids-scope-other");

    let p1 = backend
        .create(
            &tenant,
            "Patient",
            json!({ "resourceType": "Patient", "id": "p1", "name": [{ "family": "Original" }] }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .update(
            &tenant,
            &p1,
            json!({ "resourceType": "Patient", "id": "p1", "name": [{ "family": "Updated" }] }),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Patient",
            json!({ "resourceType": "Patient", "id": "p2", "name": [{ "family": "Deleted" }] }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend.delete(&tenant, "Patient", "p2").await.unwrap();

    backend
        .create(
            &tenant,
            "Patient",
            json!({ "resourceType": "Patient", "id": "p3", "name": [{ "family": "Live" }] }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Live but never requested: must not satisfy the lookup either, so a
    // filter that dropped the `id` clause (returning every live Patient of
    // the type) can't slip past this test's found.len() == 2 check.
    backend
        .create(
            &tenant,
            "Patient",
            json!({ "resourceType": "Patient", "id": "p4", "name": [{ "family": "Unrequested" }] }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Same id, different type: must not satisfy a Patient lookup for "p1".
    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "p1",
                "status": "final",
                "code": { "coding": [{ "system": "http://loinc.org", "code": "8867-4" }] },
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Same id, different tenant: must not satisfy this tenant's lookup.
    backend
        .create(
            &other_tenant,
            "Patient",
            json!({ "resourceType": "Patient", "id": "p1", "name": [{ "family": "OtherTenant" }] }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let found = backend
        .fetch_resources_by_ids(
            &tenant,
            "Patient",
            &[
                "p1".to_string(),
                "p2".to_string(),
                "p3".to_string(),
                "unknown".to_string(),
            ],
        )
        .await
        .unwrap();

    let mut by_id: std::collections::HashMap<&str, &StoredResource> =
        std::collections::HashMap::new();
    for r in &found {
        by_id.insert(r.id(), r);
    }

    assert_eq!(
        found.len(),
        2,
        "only p1 (live) and p3 (live) should be found; p2 is deleted, unknown does not exist, \
         got ids {:?}",
        found.iter().map(StoredResource::id).collect::<Vec<_>>()
    );
    let p1_found = by_id.get("p1").expect("p1 should be found");
    assert_eq!(
        p1_found.version_id(),
        "2",
        "p1 must come back at its current version, not the stale v1 content"
    );
    assert_eq!(
        p1_found.content()["name"][0]["family"],
        "Updated",
        "p1 must come back with its current content"
    );
    assert!(by_id.contains_key("p3"), "p3 (live) should be found");
    assert!(!by_id.contains_key("p2"), "p2 (deleted) must be absent");
    assert!(
        !by_id.contains_key("p4"),
        "p4 (live but never requested) must be absent"
    );
}

/// Empty input returns `Ok(vec![])`; duplicate ids in the input collapse to
/// one result.
#[tokio::test]
async fn mongodb_reindex_fetch_by_ids_empty_and_duplicate_pin() {
    let Some(backend) = create_backend("reindex_fetch_ids_empty_dup").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("fetch-ids-empty-dup");

    let empty: StorageResult<Vec<StoredResource>> = backend
        .fetch_resources_by_ids(&tenant, "Patient", &[])
        .await;
    assert_eq!(
        empty.unwrap().len(),
        0,
        "empty input must return Ok(vec![])"
    );

    backend
        .create(
            &tenant,
            "Patient",
            json!({ "resourceType": "Patient", "id": "dup", "name": [{ "family": "Dup" }] }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let found = backend
        .fetch_resources_by_ids(
            &tenant,
            "Patient",
            &["dup".to_string(), "dup".to_string(), "dup".to_string()],
        )
        .await
        .unwrap();
    assert_eq!(
        found.len(),
        1,
        "duplicate ids in the request must collapse to one result"
    );
}

/// The decode-error policy is the user's explicit decision (#1500): skip an
/// undecodable row with a warning rather than fail the whole batch, mirroring
/// SQLite's `fetch_resources_by_ids` (`sqlite/storage.rs:4001-4011`). Nothing
/// else in this file exercises that branch, so a later change back to the
/// walk's own style (`parse_history_row(..)?`, `storage.rs:5418-5424`) would
/// otherwise leave every other test in this file green.
#[tokio::test]
async fn mongodb_reindex_fetch_by_ids_skips_undecodable_row() {
    let Some(backend) = create_backend("reindex_fetch_ids_skip_bad").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("fetch-ids-skip-bad");

    backend
        .create(
            &tenant,
            "Patient",
            json!({ "resourceType": "Patient", "id": "good", "name": [{ "family": "Good" }] }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // A raw row missing `version_id`, which `parse_history_row` requires
    // (`storage.rs:490-493`) — inserted directly so it bypasses `create`'s
    // normal, always-valid shape.
    let db = backend.get_database().await.unwrap();
    let resources = db.collection::<Document>("resources");
    let tenant_id = tenant.tenant_id().as_str();
    let now = BsonDateTime::from_millis(chrono::Utc::now().timestamp_millis());
    let fhir_version = FhirVersion::default().as_mime_param().to_string();
    resources
        .insert_one(doc! {
            "tenant_id": tenant_id,
            "resource_type": "Patient",
            "id": "bad",
            "data": { "resourceType": "Patient", "id": "bad" },
            "created_at": now,
            "last_updated": now,
            "is_deleted": false,
            "deleted_at": Bson::Null,
            "fhir_version": &fhir_version,
        })
        .await
        .unwrap();

    let found = backend
        .fetch_resources_by_ids(&tenant, "Patient", &["good".to_string(), "bad".to_string()])
        .await
        .expect("an undecodable row must be skipped, not fail the whole batch");

    let ids: Vec<&str> = found.iter().map(StoredResource::id).collect();
    assert_eq!(
        ids,
        vec!["good"],
        "the undecodable \"bad\" row must be skipped; only \"good\" should be returned"
    );
}
