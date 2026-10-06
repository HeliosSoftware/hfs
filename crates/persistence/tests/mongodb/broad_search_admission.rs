//! #1748: potentially broad standard searches take a permit before their first
//! database operation, so with fewer permits than pooled connections a read
//! can still get a connection while they run.
//!
//! Child module of the `mongodb_tests` root — `use super::*;` reaches its
//! private harness (`build_backend`, `build_test_database_name`,
//! `repo_data_dir`, `create_tenant`, `shared_mongo`), the same arrangement as
//! `tests/mongodb/transaction_retry.rs`.
//!
//! Each test uses a pool of two connections and a `failCommand` failpoint that
//! blocks only `getMore`, scoped to the backend's `appName`. The broad search
//! matches 150 `search_index` rows, more than the first batch of its driver
//! cursor, so its id phase sends a `getMore` and holds that connection while
//! the server blocks it. A read by id (`find` with `limit 1`) and the search's
//! own page fetch (`limit` 11) fit in one batch and never send `getMore`, so
//! the failpoint never blocks them.

use super::*;

use std::time::Duration;

use crate::bulk_submit::FailPoint;

const MATCHING_OBSERVATIONS: usize = 150;
const BLOCK_MS: i64 = 5_000;
const READ_BUDGET: Duration = Duration::from_millis(1_000);

/// A backend with a pool of two and `limit` broad-search permits, a tenant
/// holding one Patient (whose id is returned) and [`MATCHING_OBSERVATIONS`]
/// Observations with the same code.
async fn seeded_backend(
    test_name: &str,
    app_name: &str,
    limit: usize,
) -> Option<(Arc<MongoBackend>, TenantContext, String)> {
    let connection_string = shared_mongo::connection_string().await?;
    let backend = build_backend(MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name(test_name),
        app_name: app_name.to_string(),
        data_dir: Some(repo_data_dir()),
        max_connections: 2,
        broad_search_concurrency: Some(limit),
        ..Default::default()
    })
    .await?;
    let tenant = create_tenant("broad-search-admission");
    let patient = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "name": [{"family": "Admission"}]}),
            FhirVersion::default(),
        )
        .await
        .expect("create Patient");
    for _ in 0..MATCHING_OBSERVATIONS {
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "status": "final",
                    "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]}
                }),
                FhirVersion::default(),
            )
            .await
            .expect("create Observation");
    }
    Some((Arc::new(backend), tenant, patient.id().to_string()))
}

/// `Observation?code=http://loinc.org|8867-4&_count=10`: one code-only
/// predicate, so potentially broad.
fn broad_query() -> SearchQuery {
    let mut query = SearchQuery::new("Observation");
    query.parameters = vec![SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("http://loinc.org|8867-4")],
        chain: vec![],
        components: vec![],
    }];
    query.count = Some(10);
    query
}

/// Starts two broad searches on their own tasks.
fn start_two_broad_searches(
    backend: &Arc<MongoBackend>,
    tenant: &TenantContext,
) -> Vec<tokio::task::JoinHandle<Result<usize, StorageError>>> {
    (0..2)
        .map(|_| {
            let backend = Arc::clone(backend);
            let tenant = tenant.clone();
            tokio::spawn(async move {
                backend
                    .search(&tenant, &broad_query())
                    .await
                    .map(|result| result.resources.items.len())
            })
        })
        .collect()
}

/// Turns the failpoint off and checks both searches still return their page.
async fn finish(
    failpoint: FailPoint,
    searches: Vec<tokio::task::JoinHandle<Result<usize, StorageError>>>,
) {
    failpoint.off().await;
    for search in searches {
        let page = search.await.expect("search task").expect("broad search");
        assert_eq!(page, 10, "each broad search returns a full page");
    }
}

/// One permit, two pooled connections: the second broad search waits for the
/// permit instead of taking the second connection, so a read by id completes
/// while the first search is blocked.
#[tokio::test]
async fn mongodb_broad_search_limit_leaves_a_connection_for_reads() {
    let app = "broad-search-admission-limit-1";
    let Some((backend, tenant, patient_id)) =
        seeded_backend("broad_search_admission_limit_1", app, 1).await
    else {
        eprintln!(
            "Skipping mongodb_broad_search_limit_leaves_a_connection_for_reads (requires HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let Some(failpoint) = FailPoint::enable(
        app,
        doc! { "failCommands": ["getMore"], "blockConnection": true, "blockTimeMS": BLOCK_MS },
        doc! { "times": 2 },
    )
    .await
    else {
        return;
    };

    let searches = start_two_broad_searches(&backend, &tenant);
    assert!(
        failpoint.entered_within(1, 10_000).await,
        "the first broad search reaches its blocked getMore"
    );
    assert!(
        !failpoint.entered_within(2, 1_000).await,
        "the second broad search waits for the permit and never reaches getMore"
    );

    let read = tokio::time::timeout(READ_BUDGET, backend.read(&tenant, "Patient", &patient_id))
        .await
        .expect("the read gets the free connection while the broad search is blocked")
        .expect("read Patient");
    assert!(read.is_some(), "the Patient is found");

    finish(failpoint, searches).await;
}

/// Control: two permits, two pooled connections. Both broad searches hold a
/// connection in their blocked getMore, and the read waits until a blocked
/// operation releases one.
#[tokio::test]
async fn mongodb_broad_searches_at_the_pool_size_block_reads() {
    let app = "broad-search-admission-limit-2";
    let Some((backend, tenant, patient_id)) =
        seeded_backend("broad_search_admission_limit_2", app, 2).await
    else {
        eprintln!(
            "Skipping mongodb_broad_searches_at_the_pool_size_block_reads (requires HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let Some(failpoint) = FailPoint::enable(
        app,
        doc! { "failCommands": ["getMore"], "blockConnection": true, "blockTimeMS": BLOCK_MS },
        doc! { "times": 2 },
    )
    .await
    else {
        return;
    };

    let searches = start_two_broad_searches(&backend, &tenant);
    assert!(
        failpoint.entered_within(2, 10_000).await,
        "both broad searches reach their blocked getMore"
    );

    let read =
        tokio::time::timeout(READ_BUDGET, backend.read(&tenant, "Patient", &patient_id)).await;
    assert!(
        read.is_err(),
        "with every pooled connection held by a blocked getMore the read waits"
    );

    finish(failpoint, searches).await;
}

#[tokio::test]
async fn mongodb_broad_search_with_total_uses_one_permit() {
    let Some((backend, tenant, _)) =
        seeded_backend("broad_search_total", "broad-search-total", 1).await
    else {
        return;
    };
    let mut query = broad_query();
    query.total = Some(TotalMode::Accurate);
    let result = tokio::time::timeout(Duration::from_secs(10), backend.search(&tenant, &query))
        .await
        .expect("the total must not reacquire the held permit")
        .expect("search succeeds");
    assert_eq!(result.total, Some(MATCHING_OBSERVATIONS as u64));
    assert_eq!(result.resources.items.len(), 10);
    let again = tokio::time::timeout(
        Duration::from_secs(10),
        backend.search_count(&tenant, &SearchQuery::new("Observation")),
    )
    .await
    .expect("the completed search releases its permit")
    .expect("count succeeds");
    assert_eq!(again, MATCHING_OBSERVATIONS as u64);
}
