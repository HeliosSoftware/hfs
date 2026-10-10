use super::*;
use crate::{
    backends::mongodb::{IndexBuildMode, MongoBackendConfig},
    core::SearchProvider,
    tenant::{TenantContext, TenantId, TenantPermissions},
    types::{SearchValue, SortDirective, TotalMode},
};
use mongodb::bson::DateTime;
use std::path::PathBuf;

/// Process-exit removal of the shared Mongo testcontainer below. Declared at
/// file level because a `#[path]` inside an inline module resolves through a
/// virtual directory that does not exist.
#[path = "../../../../../tests/common/container_cleanup.rs"]
mod container_cleanup;

/// One standalone `mongod` per test binary; every fixture owns a database on it.
///
/// Prefers `HFS_TEST_MONGODB_URL`, otherwise starts a container with test
/// commands enabled so the timeout tests can arm `failCommand`.
mod shared_mongo {
    use testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner};
    use testcontainers_modules::mongo::Mongo;
    use tokio::sync::OnceCell;

    struct SharedMongo {
        connection_string: String,
        /// Never dropped; the `container_cleanup` exit hook removes it.
        /// `None` when `HFS_TEST_MONGODB_URL` is used.
        _container: Option<ContainerAsync<Mongo>>,
    }

    static SHARED: OnceCell<Result<SharedMongo, String>> = OnceCell::const_new();

    /// `Err` carries the reason no server is available.
    pub(super) async fn connection_string() -> Result<String, &'static str> {
        SHARED
            .get_or_init(|| async {
                match std::env::var("HFS_TEST_MONGODB_URL") {
                    Ok(url) => Ok(SharedMongo {
                        connection_string: without_read_retries(url),
                        _container: None,
                    }),
                    Err(_) => start_container().await,
                }
            })
            .await
            .as_ref()
            .map(|shared| shared.connection_string.clone())
            .map_err(String::as_str)
    }

    /// The driver retries a read once on codes 89 and 262 and on I/O errors.
    /// A retry would hide the injected failures the timeout tests count.
    fn without_read_retries(url: String) -> String {
        let (base, query) = url.split_once('?').unwrap_or((&url, ""));
        let mut options: Vec<_> = query
            .split('&')
            .filter(|option| {
                let name = option.split_once('=').map_or(*option, |(name, _)| name);
                !option.is_empty() && !name.eq_ignore_ascii_case("retryReads")
            })
            .collect();
        options.push("retryReads=false");
        let authority = base.find("://").map_or(0, |at| at + 3);
        let slash = if base[authority..].contains('/') {
            ""
        } else {
            "/"
        };
        format!("{base}{slash}?{}", options.join("&"))
    }

    #[tokio::test]
    async fn test_connections_disable_read_retries_and_preserve_other_options() {
        for url in [
            "mongodb://localhost:27017",
            "mongodb://localhost:27017/",
            "mongodb://localhost:27017/?retryReads=true",
            "mongodb://localhost:27017/?retryReads=false",
            "mongodb://localhost:27017/?ReTrYrEaDs=true",
            "mongodb://localhost:27017/?retryReads=true&retryReads=false",
        ] {
            let options = mongodb::options::ClientOptions::parse(without_read_retries(url.into()))
                .await
                .unwrap();
            assert_eq!(options.retry_reads, Some(false), "{url}");
        }
        let url = "mongodb://reader:retryReads=true@localhost:27017/fixture?directConnection=true&retryReads=true&appName=timeout-tests";
        let options = mongodb::options::ClientOptions::parse(without_read_retries(url.into()))
            .await
            .unwrap();
        assert_eq!(options.retry_reads, Some(false));
        assert_eq!(options.direct_connection, Some(true));
        assert_eq!(options.default_database.as_deref(), Some("fixture"));
        assert_eq!(options.app_name.as_deref(), Some("timeout-tests"));
        let credential = options.credential.unwrap();
        assert_eq!(credential.username.as_deref(), Some("reader"));
        assert_eq!(credential.password.as_deref(), Some("retryReads=true"));
    }

    async fn start_container() -> Result<SharedMongo, String> {
        let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
        let container = super::container_cleanup::with_cleanup_label(
            Mongo::default()
                .with_label("github.run_id", &run_id)
                .with_cmd([
                    "mongod",
                    "--bind_ip_all",
                    "--setParameter",
                    "enableTestCommands=1",
                    "--wiredTigerCacheSizeGB",
                    "0.25",
                ]),
        )
        .start()
        .await
        .map_err(|error| {
            format!("HFS_TEST_MONGODB_URL unset and the Mongo container did not start ({error})")
        })?;
        let host = container
            .get_host()
            .await
            .map_err(|error| format!("Mongo container host unavailable ({error})"))?;
        let port = container
            .get_host_port_ipv4(27017)
            .await
            .map_err(|error| format!("Mongo container port unavailable ({error})"))?;
        Ok(SharedMongo {
            connection_string: without_read_retries(format!("mongodb://{host}:{port}")),
            _container: Some(container),
        })
    }
}

/// Reports a skip, or fails when `HFS_TEST_REQUIRE_MONGODB` is set to any value.
pub(super) fn skip_or_fail(label: &str, reason: &str) {
    if std::env::var("HFS_TEST_REQUIRE_MONGODB").is_ok_and(|value| !value.is_empty()) {
        panic!("{label}: {reason}; HFS_TEST_REQUIRE_MONGODB forbids skipping");
    }
    eprintln!("Skipping {label}: {reason}");
}

const PLAN_TENANT: &str = "plan-tests";

// failCommand configuration is server-wide, even when its data names one client.
static FAILPOINT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct FailpointSession {
    admin: mongodb::Database,
    _guard: tokio::sync::MutexGuard<'static, ()>,
}

struct Row {
    id: String,
    status: &'static str,
    deleted: bool,
}

struct Fixture {
    backend: MongoBackend,
    db: mongodb::Database,
    tenant: TenantContext,
    rows: Vec<Row>,
}

impl Fixture {
    async fn new(label: &str, count: usize) -> Option<Self> {
        let uri = match shared_mongo::connection_string().await {
            Ok(uri) => uri,
            Err(reason) => {
                skip_or_fail(label, reason);
                return None;
            }
        };
        let backend = MongoBackend::new(MongoBackendConfig {
            connection_string: uri,
            database_name: format!("hfs_plan_{}", uuid::Uuid::new_v4().simple()),
            app_name: format!("hfs-plan-{label}-{}", uuid::Uuid::new_v4().simple()),
            data_dir: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
            index_build: IndexBuildMode::Inline,
            ..Default::default()
        })
        .unwrap();
        backend.init_schema().await.unwrap();
        assert!(
            backend.search_indexes_ready(),
            "fixture needs completed search indexes"
        );
        let db = backend.get_database().await.unwrap();
        let tenant =
            TenantContext::new(TenantId::new(PLAN_TENANT), TenantPermissions::full_access());
        let rows: Vec<_> = (0..count)
            .map(|number| Row {
                id: format!("r{number:05}"),
                status: if number % 4 == 0 {
                    "final"
                } else {
                    "preliminary"
                },
                deleted: number == 0,
            })
            .collect();
        let fixture = Self {
            backend,
            db,
            tenant,
            rows,
        };
        let now = DateTime::now();
        for chunk in fixture.rows.chunks(500) {
            let resources: Vec<_> = chunk
                .iter()
                .map(|row| {
                    doc! {
                        "tenant_id":PLAN_TENANT, "resource_type":"Observation", "id":&row.id,
                        "version_id":"1", "created_at":now, "last_updated":now,
                        "is_deleted":row.deleted, "fhir_version":"4.0",
                        "data": { "resourceType":"Observation", "id":&row.id,
                            "status":row.status, "code":{"coding":[{"code":"base"}]} },
                    }
                })
                .collect();
            let index: Vec<_> = chunk
                .iter()
                .flat_map(|row| {
                    [
                        index_row(&row.id, "code", "base"),
                        index_row(&row.id, "status", row.status),
                    ]
                })
                .collect();
            fixture
                .db
                .collection::<Document>("resources")
                .insert_many(resources)
                .await
                .unwrap();
            fixture
                .db
                .collection::<Document>("search_index")
                .insert_many(index)
                .await
                .unwrap();
        }
        Some(fixture)
    }

    fn expected(
        &self,
        final_only: bool,
        direction: i32,
        offset: usize,
        limit: usize,
    ) -> Vec<String> {
        let mut ids: Vec<_> = self
            .rows
            .iter()
            .filter(|row| !row.deleted && (!final_only || row.status == "final"))
            .map(|row| row.id.clone())
            .collect();
        ids.sort();
        if direction == -1 {
            ids.reverse();
        }
        ids.into_iter().skip(offset).take(limit).collect()
    }

    async fn mark_deleted(&mut self, start: usize, end: usize) {
        let ids: Vec<_> = self.rows[start..end]
            .iter()
            .map(|row| row.id.clone())
            .collect();
        self.db
            .collection::<Document>("resources")
            .update_many(
                doc! { "tenant_id":PLAN_TENANT, "resource_type":"Observation", "id":{"$in":ids} },
                doc! { "$set":{"is_deleted":true} },
            )
            .await
            .unwrap();
        for row in &mut self.rows[start..end] {
            row.deleted = true;
        }
    }

    /// Hold exclusive access from the capability check through final disable.
    async fn failpoints(&self, label: &str) -> Option<FailpointSession> {
        let guard = FAILPOINT_LOCK.lock().await;
        let admin = self.backend.get_client().await.unwrap().database("admin");
        match admin
            .run_command(doc! {"configureFailPoint":"failCommand","mode":"off"})
            .await
        {
            Ok(_) => Some(FailpointSession {
                admin,
                _guard: guard,
            }),
            Err(error) => {
                skip_or_fail(
                    label,
                    &format!("test server needs enableTestCommands ({error})"),
                );
                None
            }
        }
    }

    /// Fail `command` on this fixture's `search_index` with a timeout error.
    async fn arm_failpoint(
        &self,
        session: &FailpointSession,
        command: &str,
        mode: Document,
    ) -> Document {
        self.arm_error(session, command, mode, 50).await
    }

    async fn arm_error(
        &self,
        session: &FailpointSession,
        command: &str,
        mode: Document,
        code: i32,
    ) -> Document {
        session
            .admin
            .run_command(doc! {"configureFailPoint":"failCommand", "mode":mode,
            "data":{"failCommands":[command], "appName":&self.backend.config().app_name,
                "namespace":format!("{}.search_index", self.db.name()), "errorCode":code,
                "errorLabels":[]}})
            .await
            .unwrap()
    }

    async fn close(self) {
        self.db.drop().await.unwrap();
    }
}

fn index_row(id: &str, parameter: &str, value: &str) -> Document {
    doc! { "tenant_id":PLAN_TENANT, "resource_type":"Observation", "resource_id":id,
    "param_name":parameter, "value_token_code":value }
}

fn resource_filter() -> Document {
    doc! { "tenant_id":PLAN_TENANT, "resource_type":"Observation", "is_deleted":false }
}

fn predicate(name: &str, value: &str) -> IndexPredicate {
    IndexPredicate::from(
        doc! { "tenant_id":PLAN_TENANT, "resource_type":"Observation",
        "param_name":name, "value_token_code":value },
    )
}

fn batched_plan(remaining: Vec<IndexPredicate>) -> BatchedIndexPlan {
    let mut driver = predicate("code", "base");
    driver.ordered_page_ready = true;
    BatchedIndexPlan {
        driver,
        remaining,
        page_policy: NoTotalPagePolicy::Streaming,
        page_cache: None,
        probe_timeout: Duration::from_millis(DEFAULT_PROBE_TIMEOUT_MS),
    }
}

fn parameter(name: &str, value: &str) -> SearchParameter {
    SearchParameter {
        name: name.into(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::parse(value)],
        chain: vec![],
        components: vec![],
    }
}

fn failpoint_hits(reply: &Document) -> i64 {
    reply
        .get_i64("count")
        .or_else(|_| reply.get_i32("count").map(i64::from))
        .expect("configureFailPoint must return its hit count")
}

async fn disable_failpoint(session: &FailpointSession) -> Document {
    session
        .admin
        .run_command(doc! {"configureFailPoint":"failCommand","mode":"off"})
        .await
        .unwrap()
}

#[tokio::test]
async fn estimated_executors_count_distinct_index_ids_and_validate_pages() {
    let Some(fixture) = Fixture::new("estimate-contract", 1024).await else {
        return;
    };
    let index = fixture.db.collection::<Document>("search_index");
    for id in ["foreign", "missing", "wrongtype"] {
        index
            .insert_many([
                index_row(id, "code", "base"),
                index_row(id, "status", "final"),
            ])
            .await
            .unwrap();
    }
    fixture.db.collection::<Document>("resources").insert_many([
        doc! { "tenant_id":"other", "resource_type":"Observation", "id":"foreign", "is_deleted":false },
        doc! { "tenant_id":PLAN_TENANT, "resource_type":"Patient", "id":"wrongtype", "is_deleted":false },
    ]).await.unwrap();
    index
        .insert_many(vec![index_row("r00004", "status", "final"); 300])
        .await
        .unwrap();
    let indexed_all = fixture.rows.len() as u64 + 3;
    let indexed_final = fixture
        .rows
        .iter()
        .filter(|row| row.status == "final")
        .count() as u64
        + 3;
    for (label, remaining, expected_total) in [
        ("scalar", vec![], indexed_all),
        ("batched", vec![predicate("status", "final")], indexed_final),
    ] {
        let plan = batched_plan(remaining);
        let result = plan
            .execute(
                &fixture.db,
                resource_filter(),
                None,
                BatchSelection::EstimatedCount,
            )
            .await
            .unwrap();
        assert_eq!(result.count, expected_total, "{label}");
        assert!(
            result.ids.is_empty(),
            "count-only execution must not produce a page"
        );
    }
    // A single predicate leaves no remaining predicate; its page must still work.
    for (label, remaining, expected_total, final_only) in [
        ("single", vec![], indexed_all, false),
        (
            "batched",
            vec![predicate("status", "final")],
            indexed_final,
            true,
        ),
    ] {
        let plan = batched_plan(remaining);
        for direction in [1, -1] {
            for offset in [0, 20, 200, 300] {
                let result = plan
                    .execute(
                        &fixture.db,
                        resource_filter(),
                        None,
                        BatchSelection::EstimatedPageAndCount {
                            direction,
                            offset,
                            limit: 21,
                            boundary: None,
                        },
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    result.count, expected_total,
                    "{label}, direction={direction}, offset={offset}"
                );
                assert_eq!(
                    result.ids,
                    fixture.expected(final_only, direction, offset as usize, 21),
                    "{label}, direction={direction}, offset={offset}"
                );
            }
        }
    }
    fixture.close().await;
}

#[tokio::test]
async fn batch_modes_preserve_repeated_filters_global_totals_and_live_cursor_pages() {
    let Some(fixture) = Fixture::new("batch-mode-contract", 1024).await else {
        return;
    };
    let index = fixture.db.collection::<Document>("search_index");
    let qualified: HashSet<_> = fixture
        .rows
        .iter()
        .step_by(3)
        .map(|row| row.id.clone())
        .collect();
    index
        .insert_many(
            qualified
                .iter()
                .map(|id| index_row(id, "code", "qualified")),
        )
        .await
        .unwrap();
    let stale_ids = ["foreign", "missing", "wrongtype"];
    index
        .insert_many(stale_ids.iter().flat_map(|id| {
            [
                index_row(id, "code", "base"),
                index_row(id, "code", "qualified"),
                index_row(id, "status", "final"),
            ]
        }))
        .await
        .unwrap();
    fixture
        .db
        .collection::<Document>("resources")
        .insert_many([
            doc! { "tenant_id":"other", "resource_type":"Observation", "id":"foreign", "is_deleted":false },
            doc! { "tenant_id":PLAN_TENANT, "resource_type":"Patient", "id":"wrongtype", "is_deleted":false },
        ])
        .await
        .unwrap();
    index
        .insert_many([
            index_row("r00132", "code", "base"),
            index_row("r00132", "code", "qualified"),
            index_row("r00132", "status", "final"),
        ])
        .await
        .unwrap();

    // Separate occurrences of code must intersect, even though they share a name.
    let plan = batched_plan(vec![
        predicate("status", "final"),
        predicate("code", "qualified"),
    ]);
    let indexed_rows: Vec<_> = fixture
        .rows
        .iter()
        .filter(|row| row.status == "final" && qualified.contains(&row.id))
        .collect();
    let live_ids: Vec<_> = indexed_rows
        .iter()
        .filter(|row| !row.deleted)
        .map(|row| row.id.clone())
        .collect();
    let exact_total = live_ids.len() as u64;
    let estimated_total = (indexed_rows.len() + stale_ids.len()) as u64;
    assert!(estimated_total > exact_total);
    for (label, selection, expected_total) in [
        ("exact", BatchSelection::Count, exact_total),
        ("estimate", BatchSelection::EstimatedCount, estimated_total),
    ] {
        let result = plan
            .execute(&fixture.db, resource_filter(), None, selection)
            .await
            .unwrap();
        assert_eq!(result.count, expected_total, "{label}");
        assert!(result.ids.is_empty(), "{label}");
    }

    for direction in [1, -1] {
        for (label, lower, upper) in [
            ("bounded-range", "r00120", "r00900"),
            ("empty-page", "r02000", "r03000"),
        ] {
            let mut expected: Vec<_> = live_ids
                .iter()
                .filter(|id| id.as_str() > lower && id.as_str() < upper)
                .cloned()
                .collect();
            expected.sort();
            if direction == -1 {
                expected.reverse();
            }
            let expected: Vec<_> = expected.into_iter().skip(2).take(3).collect();
            let boundary = Some(doc! { "id": { "$gt": lower, "$lt": upper } });
            for (mode, selection, cursor, expected_total) in [
                (
                    "bounded",
                    BatchSelection::BoundedPage {
                        direction,
                        offset: 2,
                        limit: 3,
                        boundary: boundary.clone(),
                    },
                    None,
                    Some(0),
                ),
                (
                    "exact-page",
                    BatchSelection::PageAndCount {
                        direction,
                        offset: 2,
                        limit: 3,
                        boundary: boundary.clone(),
                    },
                    None,
                    Some(exact_total),
                ),
                (
                    "estimated-page",
                    BatchSelection::EstimatedPageAndCount {
                        direction,
                        offset: 2,
                        limit: 3,
                        boundary,
                    },
                    None,
                    Some(estimated_total),
                ),
                (
                    "streaming",
                    BatchSelection::Page {
                        direction,
                        offset: 2,
                        limit: 3,
                    },
                    Some(doc! { "resource_id": { "$gt": lower, "$lt": upper } }),
                    None,
                ),
            ] {
                let result = plan
                    .execute(&fixture.db, resource_filter(), cursor, selection)
                    .await
                    .unwrap();
                assert_eq!(
                    result.ids, expected,
                    "{mode}, {label}, direction={direction}"
                );
                if let Some(total) = expected_total {
                    assert_eq!(
                        result.count, total,
                        "{mode}, {label}, direction={direction}: page bounds must not change totals"
                    );
                }
            }
        }
    }
    fixture.close().await;
}

#[tokio::test]
async fn ordered_short_and_duplicate_windows_replay_complete_live_pages() {
    for (label, count, duplicates) in [("short", 80, 0), ("duplicates", 600, 300)] {
        let Some(fixture) = Fixture::new(label, count).await else {
            return;
        };
        if duplicates > 0 {
            fixture
                .db
                .collection::<Document>("search_index")
                .insert_many(vec![index_row("r00000", "code", "base"); duplicates])
                .await
                .unwrap();
        }
        let plan = batched_plan(vec![]);
        let selection = BatchSelection::Page {
            direction: 1,
            offset: 0,
            limit: 21,
        };
        assert!(
            plan.try_ordered_page(&fixture.db, &resource_filter(), None, &selection)
                .await
                .unwrap()
                .is_none(),
            "{label} must take the fallback"
        );
        let result = plan
            .execute(&fixture.db, resource_filter(), None, selection)
            .await
            .unwrap();
        assert_eq!(result.ids, fixture.expected(false, 1, 0, 21), "{label}");
        fixture.close().await;
    }
}

#[tokio::test]
async fn ordered_sparse_windows_discard_partial_work_before_replay() {
    for (label, start, end, offset) in [
        ("initial-sparse", 0, 256, 0),
        ("later-sparse", 256, 512, 257),
    ] {
        let Some(mut fixture) = Fixture::new(label, 1024).await else {
            return;
        };
        fixture.mark_deleted(start, end).await;
        let plan = batched_plan(vec![]);
        let selection = BatchSelection::Page {
            direction: 1,
            offset,
            limit: 21,
        };
        assert!(
            plan.try_ordered_page(&fixture.db, &resource_filter(), None, &selection)
                .await
                .unwrap()
                .is_none(),
            "{label} must discard its partial result"
        );
        let result = plan
            .execute(&fixture.db, resource_filter(), None, selection)
            .await
            .unwrap();
        assert_eq!(
            result.ids,
            fixture.expected(false, 1, offset as usize, 21),
            "{label}"
        );
        fixture.close().await;
    }
}

#[tokio::test]
async fn stale_probe_changes_only_driver_order_and_refresh_clears_it() {
    let Some(fixture) = Fixture::new("cache-contract", 1024).await else {
        return;
    };
    let mut query = SearchQuery::new("Observation")
        .with_count(20)
        .with_sort(SortDirective::parse("_id"))
        .with_parameter(parameter("code", "base"))
        .with_parameter(parameter("status", "final"));
    query.total = Some(TotalMode::Accurate);
    let code = &query.parameters[0];
    let predicate = fixture.backend.planning_predicate(
        &fixture.db,
        PLAN_TENANT,
        "Observation",
        code,
        fixture
            .backend
            .build_search_index_filter(PLAN_TENANT, "Observation", code)
            .unwrap(),
        true,
    );
    let key = predicate.probe_key.unwrap();
    fixture
        .backend
        .probe_cache
        .insert(key.clone(), ProbeEstimate::Completed(1));
    let Some(SearchFilterPlan::BatchedIndex(stale)) = fixture
        .backend
        .resource_filter_plan(
            &fixture.db,
            PLAN_TENANT,
            &query,
            SearchFilterPurpose::Count,
            NoTotalPagePolicy::Streaming,
            &mut ProbeFacts::default(),
        )
        .await
        .unwrap()
    else {
        panic!("fixture must execute a batched plan");
    };
    assert_eq!(stale.driver.filter.get_str("param_name").unwrap(), "code");
    let result = fixture
        .backend
        .search(&fixture.tenant, &query)
        .await
        .unwrap();
    let expected = fixture.expected(true, 1, 0, 20);
    assert_eq!(
        result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        result.total,
        Some(fixture.expected(true, 1, 0, usize::MAX).len() as u64)
    );
    assert_eq!(
        fixture.backend.probe_cache.get(&key),
        Some(ProbeEstimate::Completed(1)),
        "actual searches must reuse the observation rather than overwrite it"
    );
    fixture
        .backend
        .refresh_stored_search_parameters()
        .await
        .unwrap();
    assert_eq!(fixture.backend.probe_cache.get(&key), None);
    let Some(SearchFilterPlan::BatchedIndex(fresh)) = fixture
        .backend
        .resource_filter_plan(
            &fixture.db,
            PLAN_TENANT,
            &query,
            SearchFilterPurpose::Count,
            NoTotalPagePolicy::Streaming,
            &mut ProbeFacts::default(),
        )
        .await
        .unwrap()
    else {
        panic!("fixture must still execute a batched plan");
    };
    let status = &query.parameters[1];
    let status_key = fixture
        .backend
        .planning_predicate(
            &fixture.db,
            PLAN_TENANT,
            "Observation",
            status,
            fixture
                .backend
                .build_search_index_filter(PLAN_TENANT, "Observation", status)
                .unwrap(),
            true,
        )
        .probe_key
        .unwrap();
    if matches!(
        fixture.backend.probe_cache.get(&status_key),
        Some(ProbeEstimate::Completed(_))
    ) {
        assert_eq!(fresh.driver.filter.get_str("param_name").unwrap(), "status");
    } else {
        eprintln!(
            "status probe did not complete within its time budget; driver choice not asserted"
        );
    }
    let result = fixture
        .backend
        .search(&fixture.tenant, &query)
        .await
        .unwrap();
    assert_eq!(
        result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        result.total,
        Some(fixture.expected(true, 1, 0, usize::MAX).len() as u64)
    );
    fixture.close().await;
}

#[tokio::test]
async fn ordered_probe_timeouts_fall_back_without_losing_matches() {
    let Some(fixture) = Fixture::new("ordered-timeouts", 1024).await else {
        return;
    };
    let Some(admin) = fixture.failpoints("ordered-timeouts").await else {
        fixture.close().await;
        return;
    };
    let plan = batched_plan(vec![]);
    for code in [50, 89, 262] {
        for (label, mode, offset) in [
            ("initial", doc! {"times":1}, 0),
            // The failpoint matches only finds on search_index, so with skip:1 the hit can
            // only come from the continuation window.
            ("continuation", doc! {"skip":1}, 257),
        ] {
            let armed = fixture.arm_error(&admin, "find", mode.clone(), code).await;
            let selection = BatchSelection::Page {
                direction: 1,
                offset,
                limit: 21,
            };
            let probed = plan
                .try_ordered_page(&fixture.db, &resource_filter(), None, &selection)
                .await;
            let disabled = disable_failpoint(&admin).await;
            assert_eq!(
                failpoint_hits(&disabled),
                failpoint_hits(&armed) + 1,
                "code={code}, {label}: the ordered probe must reach the injected timeout"
            );
            assert!(
                matches!(probed, Ok(None)),
                "code={code}, {label}: the timed-out ordered read must fall back"
            );
            let armed = fixture.arm_error(&admin, "find", mode, code).await;
            let outcome = plan
                .execute(&fixture.db, resource_filter(), None, selection)
                .await;
            let disabled = disable_failpoint(&admin).await;
            assert_eq!(
                failpoint_hits(&disabled),
                failpoint_hits(&armed) + 1,
                "code={code}, {label}: full page execution must reach the injected timeout"
            );
            let result = outcome.unwrap_or_else(|error| panic!("code={code}, {label}: {error}"));
            assert_eq!(
                result.ids,
                fixture.expected(false, 1, offset as usize, 21),
                "{label}"
            );
        }
    }
    fixture.close().await;
}

/// Probe with fresh facts until the time budget is met; `None` after five expiries.
async fn complete_zero_probe(
    fixture: &Fixture,
    predicates: &[IndexPredicate],
) -> Option<ProbeFacts> {
    for _ in 0..5 {
        let mut facts = ProbeFacts::default();
        let counts = probe_candidates(
            &fixture.db,
            predicates,
            &fixture.backend.probe_cache,
            &mut facts,
            fixture.backend.probe_timeout(),
        )
        .await
        .unwrap();
        match counts.as_slice() {
            [Some(0)] => return Some(facts),
            [None] => continue,
            other => panic!("a zero-row probe counts zero or expires, not {other:?}"),
        }
    }
    None
}

#[tokio::test]
async fn request_zero_probe_is_reused_but_candidates_are_reread() {
    let Some(fixture) = Fixture::new("request-zero", 16).await else {
        return;
    };
    let predicates = vec![predicate("code", "new")];
    let Some(mut facts) = complete_zero_probe(&fixture, &predicates).await else {
        fixture.close().await;
        skip_or_fail(
            "request-zero",
            "the zero-row probe expired on every attempt",
        );
        return;
    };
    fixture
        .db
        .collection::<Document>("search_index")
        .insert_one(index_row("r00004", "code", "new"))
        .await
        .unwrap();
    let ids = bounded_candidates(
        &fixture.db,
        &predicates,
        &fixture.backend.probe_cache,
        &mut facts,
        fixture.backend.probe_timeout(),
    )
    .await
    .unwrap()
    .expect("a zero observation still selects its predicate as the candidate driver");
    assert_eq!(ids, vec!["r00004"]);
    assert_eq!((facts.requests, facts.reuses), (1, 1));
    let timed = Some(fixture.backend.probe_timeout());
    let key = ProbeFacts::key(
        &fixture.db,
        &predicates[0].filter,
        None,
        MAX_CANDIDATE_ROWS + 1,
        timed,
    )
    .unwrap();
    for other in [
        ProbeFacts::key(
            &fixture.db,
            &predicates[0].filter,
            None,
            SELECTIVE_SEARCH_ROW_LIMIT + 1,
            timed,
        )
        .unwrap(),
        ProbeFacts::key(
            &fixture.db,
            &predicates[0].filter,
            None,
            MAX_CANDIDATE_ROWS + 1,
            None,
        )
        .unwrap(),
        ProbeFacts::key(
            &fixture.db,
            &predicate("code", "other").filter,
            None,
            MAX_CANDIDATE_ROWS + 1,
            timed,
        )
        .unwrap(),
    ] {
        assert_ne!(key, other);
        assert_eq!(facts.peek(&other), None);
    }
    fixture.close().await;
}

#[tokio::test]
async fn uncacheable_unknown_probes_are_reused_only_within_the_request() {
    let Some(fixture) = Fixture::new("request-timeout", 16).await else {
        return;
    };
    let Some(admin) = fixture.failpoints("request-timeout").await else {
        fixture.close().await;
        return;
    };
    let filters = vec![predicate("code", "base"), predicate("status", "final")];
    for _ in 0..2 {
        let armed = fixture
            .arm_failpoint(&admin, "aggregate", doc! {"times":2})
            .await;
        let mut facts = ProbeFacts::default();
        let first = probe_candidates(
            &fixture.db,
            &filters,
            &fixture.backend.probe_cache,
            &mut facts,
            fixture.backend.probe_timeout(),
        )
        .await;
        let second = probe_candidates(
            &fixture.db,
            &filters,
            &fixture.backend.probe_cache,
            &mut facts,
            fixture.backend.probe_timeout(),
        )
        .await;
        let disabled = disable_failpoint(&admin).await;
        assert_eq!(failpoint_hits(&disabled), failpoint_hits(&armed) + 2);
        assert_eq!(first.unwrap(), vec![None, None]);
        assert_eq!(second.unwrap(), vec![None, None]);
        assert_eq!((facts.requests, facts.reuses), (2, 2));
        let plan = BatchedIndexPlan::new(
            &fixture.db,
            filters.clone(),
            &fixture.backend.probe_cache,
            NoTotalPagePolicy::Streaming,
            &mut facts,
            fixture.backend.probe_timeout(),
        )
        .await
        .unwrap();
        let matches = plan
            .execute(&fixture.db, resource_filter(), None, BatchSelection::Count)
            .await
            .unwrap();
        assert_eq!(
            matches.count,
            fixture.expected(true, 1, 0, usize::MAX).len() as u64
        );
        assert_eq!((facts.requests, facts.reuses), (2, 4));
    }
    fixture.close().await;
}

#[tokio::test]
async fn configured_probe_budget_reaches_the_plan_that_probes() {
    let Some(fixture) = Fixture::new("probe-budget", 1).await else {
        return;
    };
    let mut config = fixture.backend.config().clone();
    config.probe_timeout_ms = 27;
    let configured = MongoBackend::new(config).unwrap();
    assert_eq!(
        fixture.backend.probe_timeout(),
        Duration::from_millis(DEFAULT_PROBE_TIMEOUT_MS)
    );
    assert_eq!(configured.probe_timeout(), Duration::from_millis(27));

    // The plan that runs the probes carries the budget of the backend that built it.
    let query = SearchQuery::new("Observation").with_parameter(parameter("code", "base"));
    let Some(SearchFilterPlan::BatchedIndex(plan)) = configured
        .resource_filter_plan(
            &fixture.db,
            PLAN_TENANT,
            &query,
            SearchFilterPurpose::Count,
            NoTotalPagePolicy::Streaming,
            &mut ProbeFacts::default(),
        )
        .await
        .unwrap()
    else {
        panic!("a positive count must plan a batched index read");
    };
    assert_eq!(plan.probe_timeout, Duration::from_millis(27));
    fixture.close().await;
}

#[tokio::test]
async fn cached_unknown_probes_are_reused_across_requests_and_cleared_on_refresh() {
    let Some(fixture) = Fixture::new("cached-unknown", 16).await else {
        return;
    };
    let Some(session) = fixture.failpoints("cached-unknown").await else {
        fixture.close().await;
        return;
    };
    let parameters = [parameter("code", "base"), parameter("status", "final")];
    let filters: Vec<_> = parameters
        .iter()
        .map(|parameter| {
            fixture.backend.planning_predicate(
                &fixture.db,
                PLAN_TENANT,
                "Observation",
                parameter,
                fixture
                    .backend
                    .build_search_index_filter(PLAN_TENANT, "Observation", parameter)
                    .unwrap(),
                true,
            )
        })
        .collect();
    let keys: Vec<_> = filters
        .iter()
        .map(|filter| filter.probe_key.clone().expect("fixture must be cacheable"))
        .collect();
    for code in [50, 89, 262] {
        fixture.backend.probe_cache.clear();
        let armed = fixture
            .arm_error(&session, "aggregate", doc! {"times":2}, code)
            .await;
        let mut first = ProbeFacts::default();
        let observed = probe_candidates(
            &fixture.db,
            &filters,
            &fixture.backend.probe_cache,
            &mut first,
            fixture.backend.probe_timeout(),
        )
        .await;
        let disabled = disable_failpoint(&session).await;
        assert_eq!(
            failpoint_hits(&disabled),
            failpoint_hits(&armed) + 2,
            "code={code}"
        );
        assert_eq!(observed.unwrap(), vec![None, None]);
        assert_eq!(first.requests, 2);
        for key in &keys {
            assert_eq!(
                fixture.backend.probe_cache.get(key),
                Some(ProbeEstimate::Unknown)
            );
        }

        let mut next = ProbeFacts::default();
        let observed = probe_candidates(
            &fixture.db,
            &filters,
            &fixture.backend.probe_cache,
            &mut next,
            fixture.backend.probe_timeout(),
        )
        .await
        .unwrap();
        assert_eq!(observed, vec![None, None]);
        assert_eq!((next.requests, next.reuses), (0, 0));
        let plan = BatchedIndexPlan::new(
            &fixture.db,
            filters.clone(),
            &fixture.backend.probe_cache,
            NoTotalPagePolicy::Streaming,
            &mut next,
            fixture.backend.probe_timeout(),
        )
        .await
        .unwrap();
        assert_eq!((next.requests, next.reuses), (0, 2));
        let result = plan
            .execute(
                &fixture.db,
                resource_filter(),
                None,
                BatchSelection::PageAndCount {
                    direction: 1,
                    offset: 0,
                    limit: 20,
                    boundary: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(result.ids, fixture.expected(true, 1, 0, 20));
        assert_eq!(
            result.count,
            fixture.expected(true, 1, 0, usize::MAX).len() as u64
        );
    }
    fixture
        .backend
        .refresh_stored_search_parameters()
        .await
        .unwrap();
    for key in &keys {
        assert_eq!(fixture.backend.probe_cache.get(key), None);
    }
    let mut refreshed = ProbeFacts::default();
    probe_candidates(
        &fixture.db,
        &filters,
        &fixture.backend.probe_cache,
        &mut refreshed,
        fixture.backend.probe_timeout(),
    )
    .await
    .unwrap();
    assert_eq!(refreshed.requests, 2, "refresh must allow new probes");
    fixture.close().await;
}

#[tokio::test]
async fn optional_probes_propagate_non_timeout_errors() {
    let Some(fixture) = Fixture::new("probe-errors", 1024).await else {
        return;
    };
    let Some(session) = fixture.failpoints("probe-errors").await else {
        fixture.close().await;
        return;
    };
    let plan = batched_plan(vec![]);
    for code in [13, 2] {
        for command in ["aggregate", "find"] {
            let armed = fixture
                .arm_error(&session, command, doc! {"times":1}, code)
                .await;
            let outcome = if command == "aggregate" {
                probe_candidates(
                    &fixture.db,
                    &[predicate("code", "base")],
                    &fixture.backend.probe_cache,
                    &mut ProbeFacts::default(),
                    fixture.backend.probe_timeout(),
                )
                .await
                .map(|_| ())
            } else {
                plan.try_ordered_page(
                    &fixture.db,
                    &resource_filter(),
                    None,
                    &BatchSelection::Page {
                        direction: 1,
                        offset: 0,
                        limit: 21,
                    },
                )
                .await
                .map(|_| ())
            };
            let disabled = disable_failpoint(&session).await;
            assert_eq!(
                failpoint_hits(&disabled),
                failpoint_hits(&armed) + 1,
                "code={code}, command={command}"
            );
            assert!(
                outcome.is_err(),
                "code={code}, command={command}: non-timeout error must propagate"
            );
        }
    }
    fixture.close().await;
}

#[tokio::test]
async fn required_matching_does_not_suppress_timeout_errors() {
    let Some(fixture) = Fixture::new("matching-timeouts", 16).await else {
        return;
    };
    let Some(session) = fixture.failpoints("matching-timeouts").await else {
        fixture.close().await;
        return;
    };
    let plan = batched_plan(vec![predicate("status", "final")]);
    for code in [50, 89, 262] {
        for (label, selection) in [
            ("count", BatchSelection::Count),
            ("estimate", BatchSelection::EstimatedCount),
            (
                "page",
                BatchSelection::Page {
                    direction: 1,
                    offset: 0,
                    limit: 21,
                },
            ),
            (
                "page-count",
                BatchSelection::PageAndCount {
                    direction: 1,
                    offset: 0,
                    limit: 21,
                    boundary: None,
                },
            ),
            (
                "page-estimate",
                BatchSelection::EstimatedPageAndCount {
                    direction: 1,
                    offset: 0,
                    limit: 21,
                    boundary: None,
                },
            ),
        ] {
            let armed = fixture
                .arm_error(&session, "aggregate", doc! {"times":1}, code)
                .await;
            let outcome = plan
                .count_batch(
                    &fixture.db,
                    &resource_filter(),
                    selection.clone(),
                    vec!["r00004".into()],
                )
                .await;
            let disabled = disable_failpoint(&session).await;
            assert_eq!(
                failpoint_hits(&disabled),
                failpoint_hits(&armed) + 1,
                "code={code}, selection={label}"
            );
            assert!(
                outcome.is_err(),
                "code={code}, selection={label}: actual matching must fail"
            );
        }
    }
    fixture.close().await;
}

#[tokio::test]
async fn shared_cache_observations_do_not_count_as_probe_requests() {
    let Some(fixture) = Fixture::new("cached-probe-accounting", 16).await else {
        return;
    };
    let parameters = [parameter("code", "base"), parameter("status", "final")];
    let filters: Vec<_> = parameters
        .iter()
        .map(|parameter| {
            fixture.backend.planning_predicate(
                &fixture.db,
                PLAN_TENANT,
                "Observation",
                parameter,
                fixture
                    .backend
                    .build_search_index_filter(PLAN_TENANT, "Observation", parameter)
                    .unwrap(),
                true,
            )
        })
        .collect();
    for (filter, count) in filters.iter().zip([16, 4]) {
        fixture.backend.probe_cache.insert(
            filter.probe_key.clone().expect("fixture must be cacheable"),
            ProbeEstimate::Completed(count),
        );
    }
    let mut facts = ProbeFacts::default();
    for expected_reuses in [0, 2] {
        assert_eq!(
            probe_candidates(
                &fixture.db,
                &filters,
                &fixture.backend.probe_cache,
                &mut facts,
                fixture.backend.probe_timeout(),
            )
            .await
            .unwrap(),
            vec![Some(16), Some(4)]
        );
        assert_eq!((facts.requests, facts.reuses), (0, expected_reuses));
    }
    fixture.backend.probe_cache.clear();
    let mut fresh = ProbeFacts::default();
    probe_candidates(
        &fixture.db,
        &filters,
        &fixture.backend.probe_cache,
        &mut fresh,
        fixture.backend.probe_timeout(),
    )
    .await
    .unwrap();
    assert_eq!((fresh.requests, fresh.reuses), (2, 0));
    fixture.close().await;
}

#[tokio::test]
async fn exhausted_zero_probes_remain_inconclusive() {
    let Some(fixture) = Fixture::new("zero-probe-expiries", 16).await else {
        return;
    };
    let Some(session) = fixture.failpoints("zero-probe-expiries").await else {
        fixture.close().await;
        return;
    };
    let armed = fixture
        .arm_failpoint(&session, "aggregate", doc! {"times":5})
        .await;
    let facts = complete_zero_probe(&fixture, &[predicate("code", "new")]).await;
    let disabled = disable_failpoint(&session).await;
    assert_eq!(failpoint_hits(&disabled), failpoint_hits(&armed) + 5);
    assert!(
        facts.is_none(),
        "timeouts cannot establish a zero observation"
    );
    fixture.close().await;
}

/// Complement searches probe each positive predicate without a time budget.
/// Within one request the second planning call reuses that observation.
#[tokio::test]
async fn request_reuses_untimed_selective_probe_across_planning_calls() {
    let Some(fixture) = Fixture::new("selective-reuse", 1024).await else {
        return;
    };
    let mut not_preliminary = parameter("status", "preliminary");
    not_preliminary.modifier = Some(SearchModifier::Not);
    let mut query = SearchQuery::new("Observation")
        .with_count(20)
        .with_sort(SortDirective::parse("_id"))
        .with_parameter(parameter("code", "base"))
        .with_parameter(not_preliminary);
    query.total = Some(TotalMode::Accurate);
    let untimed_key = ProbeFacts::key(
        &fixture.db,
        &fixture
            .backend
            .build_search_index_filter(PLAN_TENANT, "Observation", &query.parameters[0])
            .unwrap(),
        None,
        SELECTIVE_SEARCH_ROW_LIMIT + 1,
        None,
    )
    .unwrap();
    let mut facts = ProbeFacts::default();
    let mut plans = Vec::new();
    for _ in 0..2 {
        let Some(SearchFilterPlan::ResourceLookups {
            index_filter,
            stages,
        }) = fixture
            .backend
            .resource_filter_plan(
                &fixture.db,
                PLAN_TENANT,
                &query,
                SearchFilterPurpose::Count,
                NoTotalPagePolicy::Streaming,
                &mut facts,
            )
            .await
            .unwrap()
        else {
            panic!("a complement search must plan resource lookups");
        };
        plans.push((index_filter.map(|driver| driver.filter), stages));
    }
    assert_eq!((facts.requests, facts.reuses), (1, 1));
    assert_eq!(
        facts.peek(&untimed_key),
        Some(Some(SELECTIVE_SEARCH_ROW_LIMIT + 1)),
        "the bounded count stops one past the selective limit"
    );
    assert_eq!(
        plans[0], plans[1],
        "the reused observation must not change the plan"
    );
    assert_eq!(
        plans[0]
            .0
            .as_ref()
            .and_then(|driver| driver.get_str("param_name").ok()),
        Some("code")
    );
    let result = fixture
        .backend
        .search(&fixture.tenant, &query)
        .await
        .unwrap();
    assert_eq!(
        result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect::<Vec<_>>(),
        fixture.expected(true, 1, 0, 20)
    );
    assert_eq!(
        result.total,
        Some(fixture.expected(true, 1, 0, usize::MAX).len() as u64)
    );
    fixture.close().await;
}

#[tokio::test]
async fn id_collectors_drain_all_cursor_batches_and_deduplicate() {
    let Some(fixture) = Fixture::new("id-collector-getmore", 130).await else {
        return;
    };
    let expected_live: HashSet<_> = fixture
        .rows
        .iter()
        .filter(|row| !row.deleted)
        .map(|row| row.id.clone())
        .collect();
    let expected_indexed: HashSet<_> = fixture.rows.iter().map(|row| row.id.clone()).collect();
    assert_eq!(expected_live.len(), 129);
    assert_eq!(expected_indexed.len(), 130);

    let scope = doc! { "tenant_id": PLAN_TENANT, "resource_type": "Observation" };
    let index = fixture.db.collection::<Document>("search_index");
    assert_eq!(index.count_documents(scope.clone()).await.unwrap(), 260);
    fixture.db.run_command(doc! { "profile": 2 }).await.unwrap();

    let live = fixture
        .backend
        .all_resource_ids(&fixture.db, PLAN_TENANT, "Observation")
        .await
        .unwrap();
    let indexed = fixture
        .backend
        .distinct_resource_ids(&index, scope)
        .await
        .unwrap();
    fixture.db.run_command(doc! { "profile": 0 }).await.unwrap();
    assert_eq!(live, expected_live);
    assert_eq!(indexed, expected_indexed);
    let profile = fixture.db.collection::<Document>("system.profile");
    for (command, collection) in [("find", "resources"), ("aggregate", "search_index")] {
        let mut filter = doc! { "command.getMore": { "$exists": true } };
        filter.insert(format!("originatingCommand.{command}"), collection);
        assert!(
            profile.count_documents(filter).await.unwrap() > 0,
            "{command} on {collection} must consume a later cursor batch"
        );
    }
    fixture.close().await;
}
