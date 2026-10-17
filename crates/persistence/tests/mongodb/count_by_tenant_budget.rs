//! #1828: the cross-tenant `count_by_tenant` aggregate carries a `maxTimeMS`
//! budget (`MongoBackendConfig::count_by_tenant_max_time_ms`), and no other
//! grouped count does. When the server stops it, the call fails with
//! `BackendError::Timeout`, and the next call succeeds.
//!
//! Child module of the `mongodb_tests` root — `use super::*;` reaches its
//! private harness (`build_backend`, `build_test_database_name`,
//! `repo_data_dir`, `create_tenant`, `raw_test_client`, `shared_mongo`), the
//! same arrangement as `tests/mongodb/broad_search_admission.rs`.
//!
//! How each timeout is induced:
//! - Synthetic: a `failCommand` failpoint scoped to the backend's `appName`
//!   answers `aggregate` with `errorCode` 50 (`MaxTimeMSExpired`) or 262
//!   (`ExceededTimeLimit`). This proves the classification, not the budget.
//! - Real: a `failCommand` failpoint with `blockConnection` holds the
//!   `aggregate` on the server for [`BLOCK_MS`], longer than a backend budget
//!   of [`TINY_BUDGET_MS`]. The deadline starts before the block, so the
//!   server's own `maxTimeMS` check stops the command with `MaxTimeMSExpired`.
//!   The control test runs the same block under the shipped default budget
//!   (`MongoBackendConfig::default()`, 25 s) and succeeds, so the error comes
//!   from the budget and not from the failpoint.
//!
//! Every driver future is awaited to completion; none is dropped or wrapped in
//! `tokio::time::timeout`.

use super::*;

use helios_persistence::error::StorageResult;

use crate::bulk_submit::FailPoint;

/// Server-side block applied to the `aggregate` in the real-enforcement tests.
const BLOCK_MS: i32 = 1_000;
/// Budget configured for the real-enforcement test; well below [`BLOCK_MS`],
/// and still far above what aggregating a handful of documents takes.
const TINY_BUDGET_MS: u64 = 200;

/// The shipped `count_by_tenant` budget, far above [`BLOCK_MS`]. Read from
/// the config default rather than hard-coded, so the tests that run under the
/// default budget follow it if it changes.
fn default_budget_ms() -> u64 {
    MongoBackendConfig::default().count_by_tenant_max_time_ms
}

/// A backend whose connections carry `app_name` and whose `count_by_tenant`
/// budget is `budget_ms`, seeded with two live resources in `tenant-a` and one
/// in `tenant-b`.
async fn seeded_backend(test_name: &str, app_name: &str, budget_ms: u64) -> Option<MongoBackend> {
    let connection_string = shared_mongo::connection_string().await?;
    let backend = build_backend(MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name(test_name),
        app_name: app_name.to_string(),
        data_dir: Some(repo_data_dir()),
        count_by_tenant_max_time_ms: budget_ms,
        ..Default::default()
    })
    .await?;
    for (tenant, resource_type) in [
        ("tenant-a", "Patient"),
        ("tenant-a", "Observation"),
        ("tenant-b", "Patient"),
    ] {
        backend
            .create(
                &create_tenant(tenant),
                resource_type,
                json!({}),
                FhirVersion::default(),
            )
            .await
            .expect("seed resource");
    }
    Some(backend)
}

/// `count_by_tenant` as a sorted list, so assertions compare whole results.
async fn sorted_counts(backend: &MongoBackend) -> StorageResult<Vec<(String, u64)>> {
    let mut counts = backend.count_by_tenant().await?;
    counts.sort();
    Ok(counts)
}

fn expected_counts() -> Vec<(String, u64)> {
    vec![("tenant-a".to_string(), 2), ("tenant-b".to_string(), 1)]
}

fn assert_timeout(result: StorageResult<Vec<(String, u64)>>, what: &str) {
    match result {
        Err(StorageError::Backend(BackendError::Timeout {
            backend_name,
            message,
        })) => {
            assert_eq!(backend_name, "mongodb", "{what}");
            assert!(
                message.starts_with("Failed to count resources by tenant"),
                "{what}: the error names the operation: {message}"
            );
        }
        other => panic!("{what}: expected BackendError::Timeout, got {other:?}"),
    }
}

#[tokio::test]
async fn count_by_tenant_alone_sends_max_time_ms() {
    let Some(backend) =
        seeded_backend("count_by_tenant_max_time", "hfs-1828-profile", 12_345).await
    else {
        eprintln!(
            "Skipping count_by_tenant_alone_sends_max_time_ms (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let tenant = create_tenant("tenant-a");
    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("raw MongoDB client");
    let db_name = backend.config().database_name.clone();
    let database = client.database(&db_name);
    if let Err(e) = database.run_command(doc! { "profile": 2_i32 }).await {
        eprintln!(
            "Skipping count_by_tenant_alone_sends_max_time_ms: {{profile: 2}} was refused ({e})"
        );
        return;
    }

    let by_tenant = sorted_counts(&backend).await;
    let all_types = backend.count_all_types(&tenant).await;
    let by_types = backend.count_by_types(&tenant, &["Patient"]).await;

    let _ = database.run_command(doc! { "profile": 0_i32 }).await;
    assert_eq!(by_tenant.expect("count_by_tenant"), expected_counts());
    assert_eq!(all_types.expect("count_all_types").len(), 2);
    assert_eq!(
        by_types.expect("count_by_types"),
        vec![("Patient".to_string(), 1)]
    );

    let profile: mongodb::Collection<Document> = database.collection("system.profile");
    let mut cursor = profile
        .find(doc! { "command.aggregate": "resources" })
        .await
        .expect("query system.profile");
    let (mut by_tenant_seen, mut type_counts_seen) = (0, 0);
    while cursor.advance().await.expect("advance profile cursor") {
        let entry: Document = cursor.deserialize_current().expect("profile entry");
        let command = entry.get_document("command").expect("profiled command");
        let pipeline = command.get_array("pipeline").expect("aggregate pipeline");
        let group_key = pipeline
            .iter()
            .filter_map(Bson::as_document)
            .find_map(|stage| stage.get_document("$group").ok())
            .and_then(|group| group.get_str("_id").ok())
            .expect("a $group stage")
            .to_string();
        let max_time_ms = command.get("maxTimeMS").map(|value| match value {
            Bson::Int32(ms) => i64::from(*ms),
            Bson::Int64(ms) => *ms,
            other => panic!("maxTimeMS is not an integer: {other:?}"),
        });
        match group_key.as_str() {
            "$tenant_id" => {
                by_tenant_seen += 1;
                assert_eq!(
                    max_time_ms,
                    Some(12_345),
                    "count_by_tenant must send the configured budget: {command:?}"
                );
            }
            "$resource_type" => {
                type_counts_seen += 1;
                assert_eq!(
                    max_time_ms, None,
                    "count_all_types/count_by_types must stay unbounded: {command:?}"
                );
            }
            other => panic!("unexpected grouped aggregate on resources: {other}"),
        }
    }
    assert_eq!(by_tenant_seen, 1, "count_by_tenant was profiled once");
    assert_eq!(
        type_counts_seen, 2,
        "count_all_types and count_by_types were profiled"
    );
}

#[tokio::test]
async fn count_by_tenant_timeout_codes_classify_as_timeout_and_recover() {
    let app = "hfs-1828-timeout-codes";
    let Some(backend) =
        seeded_backend("count_by_tenant_timeout_codes", app, default_budget_ms()).await
    else {
        eprintln!(
            "Skipping count_by_tenant_timeout_codes_classify_as_timeout_and_recover \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };

    // 50 MaxTimeMSExpired is final. 262 ExceededTimeLimit is a retryable read
    // error for the driver, which retries the aggregate once, so it must fail
    // twice to reach the caller.
    for (code, times, name) in [(50, 1, "MaxTimeMSExpired"), (262, 2, "ExceededTimeLimit")] {
        let Some(failpoint) = FailPoint::enable(
            app,
            doc! { "failCommands": ["aggregate"], "errorCode": code },
            doc! { "times": times },
        )
        .await
        else {
            return;
        };
        let failed = sorted_counts(&backend).await;
        let recovered = sorted_counts(&backend).await;
        let fired = failpoint.off_and_count().await;

        assert_timeout(failed, name);
        assert_eq!(
            recovered.expect("the call after a timeout succeeds"),
            expected_counts(),
            "{name}: recovery"
        );
        assert_eq!(fired, i64::from(times), "{name}: failpoint fired");
    }

    // Control: a non-timeout command error keeps its old classification.
    let Some(failpoint) = FailPoint::enable(
        app,
        doc! { "failCommands": ["aggregate"], "errorCode": 2_i32 },
        doc! { "times": 1_i32 },
    )
    .await
    else {
        return;
    };
    let failed = sorted_counts(&backend).await;
    let recovered = sorted_counts(&backend).await;
    failpoint.off().await;
    assert!(
        matches!(
            failed,
            Err(StorageError::Backend(BackendError::Internal { .. }))
        ),
        "BadValue stays Internal: {failed:?}"
    );
    assert_eq!(recovered.expect("recovery"), expected_counts());
}

#[tokio::test]
async fn count_by_tenant_budget_is_enforced_by_the_server_and_recovers() {
    let app = "hfs-1828-enforced";
    let Some(backend) = seeded_backend("count_by_tenant_enforced", app, TINY_BUDGET_MS).await
    else {
        eprintln!(
            "Skipping count_by_tenant_budget_is_enforced_by_the_server_and_recovers \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let Some(failpoint) = FailPoint::enable(
        app,
        doc! { "failCommands": ["aggregate"], "blockConnection": true, "blockTimeMS": BLOCK_MS },
        doc! { "times": 1_i32 },
    )
    .await
    else {
        return;
    };
    let failed = sorted_counts(&backend).await;
    // The failpoint is spent: the same backend, same budget, now succeeds.
    let recovered = sorted_counts(&backend).await;
    let fired = failpoint.off_and_count().await;

    assert_timeout(failed, "blocked past the budget");
    assert_eq!(fired, 1, "only the first aggregate was blocked");
    assert_eq!(
        recovered.expect("the call after a timeout succeeds"),
        expected_counts()
    );
}

#[tokio::test]
async fn count_by_tenant_blocked_within_the_default_budget_succeeds() {
    let app = "hfs-1828-within-budget";
    let Some(backend) =
        seeded_backend("count_by_tenant_within_budget", app, default_budget_ms()).await
    else {
        eprintln!(
            "Skipping count_by_tenant_blocked_within_the_default_budget_succeeds \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let Some(failpoint) = FailPoint::enable(
        app,
        doc! { "failCommands": ["aggregate"], "blockConnection": true, "blockTimeMS": BLOCK_MS },
        doc! { "times": 1_i32 },
    )
    .await
    else {
        return;
    };
    let counts = sorted_counts(&backend).await;
    let fired = failpoint.off_and_count().await;

    assert_eq!(fired, 1, "the aggregate was blocked");
    assert_eq!(
        counts.expect("a block shorter than the budget is not a timeout"),
        expected_counts()
    );
}

/// Tenants past the aggregate's default first batch of 101 documents, so the
/// `$group` result needs a `getMore`.
const TENANTS_PAST_FIRST_BATCH: usize = 105;

#[tokio::test]
async fn count_by_tenant_timeout_on_get_more_names_the_operation() {
    let app = "hfs-1828-get-more";
    let Some(backend) = seeded_backend("count_by_tenant_get_more", app, default_budget_ms()).await
    else {
        eprintln!(
            "Skipping count_by_tenant_timeout_on_get_more_names_the_operation \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    // `maxTimeMS` also covers the cursor's `getMore` batches, so a budget can
    // expire after the aggregate itself answered. Seed enough tenants that
    // the grouped result spills past the first batch.
    for i in 0..TENANTS_PAST_FIRST_BATCH {
        backend
            .create(
                &create_tenant(&format!("tenant-many-{i:03}")),
                "Patient",
                json!({}),
                FhirVersion::default(),
            )
            .await
            .expect("seed resource");
    }
    let expected_len = TENANTS_PAST_FIRST_BATCH + expected_counts().len();

    let Some(failpoint) = FailPoint::enable(
        app,
        doc! { "failCommands": ["getMore"], "errorCode": 50_i32 },
        doc! { "times": 1_i32 },
    )
    .await
    else {
        return;
    };
    let failed = sorted_counts(&backend).await;
    let recovered = sorted_counts(&backend).await;
    let fired = failpoint.off_and_count().await;

    assert_eq!(fired, 1, "the getMore was failed once");
    assert_timeout(failed, "MaxTimeMSExpired on getMore");
    assert_eq!(
        recovered
            .expect("the call after a getMore timeout succeeds")
            .len(),
        expected_len
    );
}
