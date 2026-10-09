//! #1910: the cross-tenant `count_by_tenant` aggregate is served by
//! `idx_resources_live_tenant` (`{is_deleted: 1, tenant_id: 1}`) as a covered
//! index scan, not by reading every resource document.
//!
//! Child module of the `mongodb_tests` root — `use super::*;` reaches its
//! private harness (`build_backend`, `build_test_database_name`,
//! `repo_data_dir`, `create_tenant`, `raw_test_client`, `shared_mongo`), the
//! same arrangement as `tests/mongodb/count_by_tenant_budget.rs`.
//!
//! The plan is read from the profiler entry of the backend's own `aggregate`,
//! so these tests check the command `count_by_tenant` really sends, not a copy
//! of its pipeline. The budget and `Timeout` contract (#1828) stays covered by
//! `count_by_tenant_budget.rs`; the live-resource semantics by
//! `mongodb_integration_count_by_tenant`.

use super::*;

/// Name and key of the index this issue adds.
const LIVE_TENANT_INDEX: &str = "idx_resources_live_tenant";

/// A backend on a fresh database, seeded with live and soft-deleted resources
/// in three tenants: `tenant-a` 3 live + 1 deleted, `tenant-b` 1 live,
/// `tenant-gone` 1 deleted (so no live resources).
async fn seeded_backend(test_name: &str) -> Option<MongoBackend> {
    let connection_string = shared_mongo::connection_string().await?;
    let backend = build_backend(MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name(test_name),
        data_dir: Some(repo_data_dir()),
        ..Default::default()
    })
    .await?;
    for (tenant, resource_type, delete) in [
        ("tenant-a", "Patient", false),
        ("tenant-a", "Patient", false),
        ("tenant-a", "Observation", false),
        ("tenant-a", "Observation", true),
        ("tenant-b", "Patient", false),
        ("tenant-gone", "Patient", true),
    ] {
        let tenant = create_tenant(tenant);
        let created = backend
            .create(&tenant, resource_type, json!({}), FhirVersion::default())
            .await
            .expect("seed resource");
        if delete {
            backend
                .delete(&tenant, resource_type, created.id())
                .await
                .expect("soft delete seed resource");
        }
    }
    Some(backend)
}

fn expected_counts() -> Vec<(String, u64)> {
    vec![("tenant-a".to_string(), 3), ("tenant-b".to_string(), 1)]
}

async fn sorted_counts(backend: &MongoBackend) -> Vec<(String, u64)> {
    let mut counts = backend.count_by_tenant().await.expect("count_by_tenant");
    counts.sort();
    counts
}

/// The database the backend writes to, through a separate small-pool client.
async fn raw_database(backend: &MongoBackend) -> mongodb::Database {
    raw_test_client(&backend.config().connection_string)
        .await
        .expect("raw MongoDB client")
        .database(&backend.config().database_name)
}

/// What the profiler recorded for the one `count_by_tenant` aggregate run
/// while it was on.
#[derive(Debug)]
struct ProfiledCount {
    plan_summary: String,
    keys_examined: i64,
    docs_examined: i64,
}

fn as_i64(entry: &Document, field: &str) -> i64 {
    match entry.get(field) {
        Some(Bson::Int32(n)) => i64::from(*n),
        Some(Bson::Int64(n)) => *n,
        other => panic!("profiler field {field} is not an integer: {other:?}"),
    }
}

/// Runs `count_by_tenant` with the profiler on and returns its result and
/// the profiled plan of its aggregate; `None` when the server refuses
/// `{profile: 2}` (an external server without the privilege).
async fn profiled_count_by_tenant(
    backend: &MongoBackend,
) -> Option<(Vec<(String, u64)>, ProfiledCount)> {
    let database = raw_database(backend).await;
    if let Err(e) = database.run_command(doc! { "profile": 2_i32 }).await {
        eprintln!("Skipping: {{profile: 2}} was refused ({e})");
        return None;
    }
    let counts = sorted_counts(backend).await;
    let _ = database.run_command(doc! { "profile": 0_i32 }).await;

    let profile: mongodb::Collection<Document> = database.collection("system.profile");
    let mut cursor = profile
        .find(doc! { "command.aggregate": "resources" })
        .await
        .expect("query system.profile");
    let mut seen = Vec::new();
    while cursor.advance().await.expect("advance profile cursor") {
        let entry: Document = cursor.deserialize_current().expect("profile entry");
        seen.push(ProfiledCount {
            plan_summary: entry.get_str("planSummary").unwrap_or_default().to_string(),
            keys_examined: as_i64(&entry, "keysExamined"),
            docs_examined: as_i64(&entry, "docsExamined"),
        });
    }
    assert_eq!(seen.len(), 1, "one aggregate profiled: {seen:?}");
    let _ = database
        .collection::<Document>("system.profile")
        .drop()
        .await;
    Some((counts, seen.pop().expect("one entry")))
}

#[tokio::test]
async fn schema_init_creates_the_live_tenant_index() {
    let Some(backend) = seeded_backend("count_index_exists").await else {
        eprintln!(
            "Skipping schema_init_creates_the_live_tenant_index (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let database = raw_database(&backend).await;
    let reply = database
        .run_command(doc! { "listIndexes": "resources" })
        .await
        .expect("listIndexes resources");
    let indexes = reply
        .get_document("cursor")
        .and_then(|cursor| cursor.get_array("firstBatch"))
        .expect("listIndexes firstBatch");
    let index = indexes
        .iter()
        .filter_map(Bson::as_document)
        .find(|index| index.get_str("name") == Ok(LIVE_TENANT_INDEX))
        .unwrap_or_else(|| panic!("{LIVE_TENANT_INDEX} is missing: {indexes:?}"));

    // Key order matters: `is_deleted` first so `{is_deleted: false}` is an
    // index bound, `tenant_id` second so `$group` reads it from the key.
    let key = index.get_document("key").expect("index key");
    let fields: Vec<(&str, i64)> = key
        .iter()
        .map(|(field, value)| {
            (
                field.as_str(),
                value
                    .as_i64()
                    .or(value.as_i32().map(i64::from))
                    .expect("integer key"),
            )
        })
        .collect();
    assert_eq!(fields, vec![("is_deleted", 1), ("tenant_id", 1)]);
    assert!(
        !index.contains_key("partialFilterExpression"),
        "a partial index would need a FETCH to re-check is_deleted: {index:?}"
    );
    assert!(
        !index.get_bool("unique").unwrap_or(false),
        "not unique: {index:?}"
    );
}

#[tokio::test]
async fn count_by_tenant_is_a_covered_index_scan() {
    let Some(backend) = seeded_backend("count_index_covered").await else {
        eprintln!(
            "Skipping count_by_tenant_is_a_covered_index_scan (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let Some((counts, profiled)) = profiled_count_by_tenant(&backend).await else {
        return;
    };

    assert_eq!(counts, expected_counts(), "live resources only, never zero");
    assert_eq!(
        profiled.plan_summary, "IXSCAN { is_deleted: 1, tenant_id: 1 }",
        "the planner uses {LIVE_TENANT_INDEX}: {profiled:?}"
    );
    assert_eq!(
        profiled.docs_examined, 0,
        "covered: no document is fetched: {profiled:?}"
    );
    assert_eq!(
        profiled.keys_examined, 4,
        "only the live keys are scanned, not the 2 deleted ones: {profiled:?}"
    );
}

/// An existing store gets the index from schema init on its next boot, and
/// until then (index missing, or still building) the count still answers,
/// from a collection scan, because the aggregate does not hint the index.
#[tokio::test]
async fn an_existing_store_without_the_index_still_counts_and_gets_it_at_boot() {
    let Some(backend) = seeded_backend("count_index_rollout").await else {
        eprintln!(
            "Skipping an_existing_store_without_the_index_still_counts_and_gets_it_at_boot \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let database = raw_database(&backend).await;
    database
        .run_command(doc! { "dropIndexes": "resources", "index": LIVE_TENANT_INDEX })
        .await
        .expect("drop the index, as on a store from before #1910");
    database
        .collection::<Document>("schema_version")
        .update_one(
            doc! { "_id": "schema_version" },
            doc! { "$set": { "version": 11_i32 } },
        )
        .await
        .expect("record schema v11");

    let Some((counts, before)) = profiled_count_by_tenant(&backend).await else {
        return;
    };
    assert_eq!(
        counts,
        expected_counts(),
        "unchanged result without the index"
    );
    assert_eq!(before.plan_summary, "COLLSCAN", "{before:?}");

    // The next boot on the same database.
    let rebooted = build_backend(backend.config().clone())
        .await
        .expect("reboot on the same database");
    let Some((counts, after)) = profiled_count_by_tenant(&rebooted).await else {
        return;
    };
    assert_eq!(counts, expected_counts());
    assert_eq!(
        after.plan_summary, "IXSCAN { is_deleted: 1, tenant_id: 1 }",
        "{after:?}"
    );
    assert_eq!(after.docs_examined, 0, "{after:?}");
    let version = database
        .collection::<Document>("schema_version")
        .find_one(doc! { "_id": "schema_version" })
        .await
        .expect("read schema_version")
        .and_then(|doc| doc.get_i32("version").ok());
    assert_eq!(
        version,
        Some(helios_persistence::backends::mongodb::SCHEMA_VERSION),
        "boot records the current schema version"
    );
}
