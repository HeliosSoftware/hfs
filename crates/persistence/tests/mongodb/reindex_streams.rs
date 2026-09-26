//! #1403: concurrent write streams over disjoint id ranges for the MongoDB
//! `$reindex` rebuild — the range and catch-up cursors, the stream plan, and
//! whole rebuilds split across streams. A `#[path]`-included child module of
//! `mongodb_tests.rs`, like `reindex_id_walk.rs`: `use super::*` reaches the
//! parent's harness (`create_backend`, `create_tenant`, `shared_mongo`, …).

use super::*;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use helios_persistence::error::StorageResult;
use helios_persistence::search::{
    ReindexOperation, ReindexRequest, ReindexSource, ReindexStatus, ReindexTarget, ResourcePage,
    TypeWalkPlan, TypeWalkRequest,
};
use helios_persistence::types::StoredResource;

use super::reindex_id_walk::{
    backdate_fixture, capture_walk_logs, seed_walk_fixture, snapshot, wait_for_terminal,
    walk_log_lines,
};
use super::reindex_pipeline::{create_backend_with_pool, log_field_value};

/// Floor of the hand-built range cursors below: later than every backdated
/// fixture row, so the whole fixture falls in the id phase.
const RANGE_FLOOR: &str = "2021-01-01T00:00:00.000Z";

/// A range cursor spelled out, pinning the backend's grammar
/// `v2|r|<floor>|<lo>|<hi>|<after_id>`: an empty bound is open, and an empty
/// `after_id` starts the range.
fn range_cursor(lo: &str, hi: &str) -> String {
    format!("v2|r|{RANGE_FLOOR}|{lo}|{hi}|")
}

fn ids_of(page: &ResourcePage) -> Vec<String> {
    page.resources.iter().map(|r| r.id().to_string()).collect()
}

/// Pages one Observation walk from `cursor` to its end, checking the walk's
/// page contract on the way: every page with a next cursor holds at least
/// one resource, and the walk ends on one empty page with none. Returns the
/// ids of each page that had a next cursor, in fetch order — the empty page
/// that ends the walk contributes no entry.
async fn walk_ids(
    backend: &MongoBackend,
    tenant: &TenantContext,
    cursor: &str,
    limit: u32,
    max_bytes: u64,
) -> Vec<Vec<String>> {
    let mut pages = Vec::new();
    let mut cursor = cursor.to_string();
    for _ in 0..1000 {
        let page = backend
            .fetch_resources_page_capped(tenant, "Observation", Some(&cursor), limit, max_bytes)
            .await
            .unwrap();
        let page_ids = ids_of(&page);
        match page.next_cursor {
            Some(next) => {
                assert!(
                    !page_ids.is_empty(),
                    "a page with a next cursor must hold a resource"
                );
                pages.push(page_ids);
                cursor = next;
            }
            None => {
                assert!(page_ids.is_empty(), "a walk ends on one empty page");
                return pages;
            }
        }
    }
    panic!("the walk from {cursor} did not end within 1000 pages");
}

/// Pool of the write-stream tests' backends: `(10 - 2) / 2` admits four
/// streams for one rebuild, where the suite-wide cap of 4 admits one.
const STREAMS_TEST_POOL: u32 = 10;

async fn create_streams_backend(test_name: &str) -> Option<Arc<MongoBackend>> {
    create_backend_with_pool(test_name, STREAMS_TEST_POOL, |_| {}).await
}

/// The field names that follow `message` on a captured log line, in order.
fn field_names_after(line: &str, message: &str) -> Vec<String> {
    let (_, rest) = line
        .split_once(message)
        .unwrap_or_else(|| panic!("{line:?} does not carry {message:?}"));
    rest.split_whitespace()
        .filter_map(|token| token.split_once('=').map(|(name, _)| name.to_string()))
        .collect()
}

async fn server_major_version(db: &mongodb::Database) -> u32 {
    let info = db.run_command(doc! { "buildInfo": 1_i32 }).await.unwrap();
    info.get_str("version")
        .ok()
        .and_then(|v| v.split('.').next())
        .and_then(|major| major.parse().ok())
        .unwrap_or(0)
}

#[tokio::test]
async fn mongodb_plan_type_walk_ranges_cover_the_id_phase() {
    let Some(backend) = create_streams_backend("reindex_streams_plan_cover").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("streams-plan-cover");
    let fixture = seed_walk_fixture(&backend, &tenant, 400, "extra").await;
    backdate_fixture(&backend, &tenant, &fixture).await;

    let plan = backend
        .plan_type_walk(
            &tenant,
            "Observation",
            TypeWalkRequest {
                streams: 4,
                min_resources_per_stream: 50,
                concurrent_runs: 1,
            },
        )
        .await
        .unwrap();
    let (ranges, catch_up) = match plan {
        TypeWalkPlan::Ranges { ranges, catch_up } => (ranges, catch_up),
        TypeWalkPlan::Single => panic!("400 Observations at 50 per stream must plan four ranges"),
    };
    assert_eq!(ranges.len(), 4, "{ranges:?}");
    assert!(ranges.iter().all(|c| c.starts_with("v2|r|")), "{ranges:?}");
    assert!(catch_up.starts_with("v2|d|"), "{catch_up}");

    // A 1-byte cap takes exactly one resource per page; every page of a range
    // continues that range, and each range ends on one empty page (checked by
    // `walk_ids` itself).
    let mut seen = BTreeSet::new();
    for (i, range) in ranges.iter().enumerate() {
        let pages = walk_ids(&backend, &tenant, range, 100, 1).await;
        assert!(!pages.is_empty(), "range {i} is empty");
        for page in &pages {
            assert_eq!(page.len(), 1, "range {i}: {page:?}");
        }
        let in_range = pages.concat();
        assert!(
            in_range.windows(2).all(|w| w[0] < w[1]),
            "range {i} is not in id order"
        );
        for id in in_range {
            assert!(seen.insert(id.clone()), "{id} is in two ranges");
        }
    }
    assert_eq!(&seen, &fixture.live["Observation"]);
}

#[tokio::test]
async fn mongodb_plan_type_walk_fits_the_pool_and_the_type_size() {
    capture_walk_logs();
    let request = TypeWalkRequest {
        streams: 4,
        min_resources_per_stream: 50,
        concurrent_runs: 1,
    };

    // The suite-wide pool of 4 admits (4 - 2) / 2 = 1 stream.
    let Some(small_pool) = create_backend("reindex_streams_small_pool").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("streams-small-pool");
    seed_walk_fixture(&small_pool, &tenant, 200, "extra").await;
    for _ in 0..2 {
        assert_eq!(
            small_pool
                .plan_type_walk(&tenant, "Observation", request)
                .await
                .unwrap(),
            TypeWalkPlan::Single
        );
    }
    let needle = format!("tenant={}", tenant.tenant_id().as_str());
    let warned = walk_log_lines(&[
        "HFS_REINDEX_WRITE_STREAMS=4 needs HFS_MONGODB_MAX_CONNECTIONS ≥ 10; using 1",
        &needle,
    ]);
    assert_eq!(
        warned.len(),
        1,
        "the clamp warns once per backend: {warned:?}"
    );
    assert_eq!(
        warned[0].split_whitespace().nth(1),
        Some("WARN"),
        "{}",
        warned[0]
    );
    let planned = walk_log_lines(&["mongodb reindex streams planned", &needle]);
    assert_eq!(planned.len(), 2, "{planned:?}");
    for line in &planned {
        assert_eq!(
            field_names_after(line, "mongodb reindex streams planned"),
            [
                "tenant",
                "resource_type",
                "requested",
                "allowed",
                "resources",
                "streams",
                "plan_ms"
            ]
        );
        assert_eq!(log_field_value(line, "requested"), "4", "{line}");
        assert_eq!(log_field_value(line, "allowed"), "1", "{line}");
        assert_eq!(log_field_value(line, "resources"), "0", "{line}");
        assert_eq!(log_field_value(line, "streams"), "1", "{line}");
    }

    // A pool of 10 shared by two rebuilds admits (10 - 2) / (2 * 2) = 2 streams.
    let Some(pool_ten) = create_streams_backend("reindex_streams_pool_ten").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("streams-pool-ten");
    seed_walk_fixture(&pool_ten, &tenant, 200, "extra").await;
    let plan = pool_ten
        .plan_type_walk(
            &tenant,
            "Observation",
            TypeWalkRequest {
                concurrent_runs: 2,
                ..request
            },
        )
        .await
        .unwrap();
    assert!(
        matches!(&plan, TypeWalkPlan::Ranges { ranges, .. } if ranges.len() == 2),
        "{plan:?}"
    );
    // A type too small for two streams of 1,000 keeps its single walk.
    let small_type = pool_ten
        .plan_type_walk(
            &tenant,
            "Observation",
            TypeWalkRequest {
                min_resources_per_stream: 1_000,
                ..request
            },
        )
        .await
        .unwrap();
    assert_eq!(small_type, TypeWalkPlan::Single);
    let needle = format!("tenant={}", tenant.tenant_id().as_str());
    let planned = walk_log_lines(&["mongodb reindex streams planned", &needle]);
    assert_eq!(planned.len(), 2, "{planned:?}");
    assert_eq!(
        log_field_value(&planned[0], "allowed"),
        "2",
        "{}",
        planned[0]
    );
    assert_eq!(
        log_field_value(&planned[0], "resources"),
        "200",
        "{}",
        planned[0]
    );
    assert_eq!(
        log_field_value(&planned[0], "streams"),
        "2",
        "{}",
        planned[0]
    );
    assert_eq!(
        log_field_value(&planned[1], "allowed"),
        "4",
        "{}",
        planned[1]
    );
    assert_eq!(
        log_field_value(&planned[1], "resources"),
        "200",
        "{}",
        planned[1]
    );
    assert_eq!(
        log_field_value(&planned[1], "streams"),
        "1",
        "{}",
        planned[1]
    );
}

#[tokio::test]
async fn mongodb_plan_type_walk_boundary_probes_are_covered() {
    use futures::stream::TryStreamExt;

    let Some(backend) = create_streams_backend("reindex_streams_probe_plan").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("streams-probe-plan");
    seed_walk_fixture(&backend, &tenant, 300, "extra").await;

    let db = backend.get_database().await.unwrap();
    // The suite's testcontainers mongod is a standalone that allows
    // profiling; a refusal is a failure, not a skip.
    db.run_command(doc! { "profile": 2_i32 })
        .await
        .expect("the test mongod must accept {profile: 2}");
    let plan = backend
        .plan_type_walk(
            &tenant,
            "Observation",
            TypeWalkRequest {
                streams: 4,
                min_resources_per_stream: 50,
                concurrent_runs: 1,
            },
        )
        .await
        .unwrap();
    let _ = db.run_command(doc! { "profile": 0_i32 }).await;
    assert!(
        matches!(&plan, TypeWalkPlan::Ranges { ranges, .. } if ranges.len() == 4),
        "{plan:?}"
    );

    let probes: Vec<Document> = db
        .collection::<Document>("system.profile")
        .find(doc! {
            "ns": format!("{}.resources", db.name()),
            "command.find": "resources",
            "command.skip": { "$exists": true },
        })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(probes.len(), 3, "one probe per boundary: {probes:?}");

    let major = server_major_version(&db).await;
    for entry in &probes {
        let command = entry.get_document("command").unwrap();
        assert_eq!(command.get_str("hint").ok(), Some("idx_resources_identity"));
        let mut inner = Document::new();
        for key in [
            "find",
            "filter",
            "sort",
            "skip",
            "limit",
            "projection",
            "hint",
        ] {
            if let Some(value) = command.get(key) {
                inner.insert(key, value.clone());
            }
        }
        let explain = db
            .run_command(doc! { "explain": inner, "verbosity": "executionStats" })
            .await
            .unwrap();
        let winning = explain
            .get_document("queryPlanner")
            .and_then(|qp| qp.get_document("winningPlan"))
            .unwrap()
            .clone();
        let mut names = Vec::new();
        collect_index_names(&winning, &mut names);
        assert!(
            !names.is_empty() && names.iter().all(|n| n == "idx_resources_identity"),
            "{names:?}"
        );
        assert!(
            !contains_stage_named(&winning, "SORT"),
            "a probe must not sort: {winning:?}"
        );
        if major >= 7 {
            let docs_examined = explain
                .get_document("executionStats")
                .and_then(|s| {
                    s.get_i64("totalDocsExamined")
                        .or_else(|_| s.get_i32("totalDocsExamined").map(i64::from))
                })
                .unwrap();
            assert_eq!(docs_examined, 0, "a probe must be covered: {explain:?}");
        } else {
            eprintln!("MongoDB {major}: probe plan, not asserted covered below 7.0: {winning:?}");
        }
    }
}

#[tokio::test]
async fn mongodb_id_range_cursors_partition_the_id_phase() {
    let Some(backend) = create_backend("reindex_streams_range_cursors").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("streams-range-cursors");
    let fixture = seed_walk_fixture(&backend, &tenant, 60, "extra").await;
    backdate_fixture(&backend, &tenant, &fixture).await;
    capture_walk_logs();

    let bounds = [("", "obs-020"), ("obs-020", "obs-040"), ("obs-040", "")];
    let mut seen = BTreeSet::new();
    for (lo, hi) in bounds {
        let ids = walk_ids(&backend, &tenant, &range_cursor(lo, hi), 7, 0)
            .await
            .concat();
        assert!(!ids.is_empty(), "[{lo}, {hi}) is empty");
        assert!(
            ids.windows(2).all(|w| w[0] < w[1]),
            "[{lo}, {hi}) is not in id order: {ids:?}"
        );
        for id in ids {
            assert!(lo.is_empty() || id.as_str() >= lo, "{id} is below {lo}");
            assert!(
                hi.is_empty() || id.as_str() < hi,
                "{id} is at or above {hi}"
            );
            assert!(seen.insert(id.clone()), "{id} came back from two ranges");
        }
    }
    assert_eq!(&seen, &fixture.live["Observation"]);

    let needle = format!("tenant={}", tenant.tenant_id().as_str());
    let finished = walk_log_lines(&["mongodb reindex id range finished", &needle]);
    assert_eq!(finished.len(), 3, "{finished:?}");
    for (line, (lo, hi)) in finished.iter().zip(bounds) {
        assert_eq!(
            log_field_value(line, "lo"),
            if lo.is_empty() { "*" } else { lo },
            "{line}"
        );
        assert_eq!(
            log_field_value(line, "hi"),
            if hi.is_empty() { "*" } else { hi },
            "{line}"
        );
        assert_eq!(log_field_value(line, "floor"), RANGE_FLOOR, "{line}");
    }
    assert!(
        walk_log_lines(&["mongodb reindex id phase finished", &needle]).is_empty(),
        "a range never ends the id phase itself"
    );
}

#[tokio::test]
async fn mongodb_id_range_ahead_fetch_matches_the_serial_fetch() {
    let Some(backend) = create_backend("reindex_streams_range_ahead").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("streams-range-ahead");
    let fixture = seed_walk_fixture(&backend, &tenant, 30, "extra").await;
    backdate_fixture(&backend, &tenant, &fixture).await;

    let mut cursor = range_cursor("obs-010", "");
    for _ in 0..50 {
        let serial = backend
            .fetch_resources_page_capped(&tenant, "Observation", Some(&cursor), 4, 0)
            .await
            .unwrap();
        let ahead = backend
            .fetch_resources_page_ahead(&tenant, "Observation", &cursor, 4, 0)
            .await
            .unwrap()
            .expect("a range page is always fetched ahead, the empty last one included");
        assert_eq!(ids_of(&ahead), ids_of(&serial));
        assert_eq!(ahead.next_cursor, serial.next_cursor);
        match serial.next_cursor {
            Some(next) => cursor = next,
            None => return,
        }
    }
    panic!("the range did not end within 50 pages");
}

#[tokio::test]
async fn mongodb_id_phase_done_cursor_runs_the_catch_up_from_its_floor() {
    let Some(backend) = create_backend("reindex_streams_phase_done").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("streams-phase-done");
    let fixture = seed_walk_fixture(&backend, &tenant, 30, "extra").await;
    backdate_fixture(&backend, &tenant, &fixture).await;
    capture_walk_logs();

    // The backdated fixture stamps Observation `i` at second `i % 3`, so a
    // catch-up from second 1 walks exactly the Observations of seconds 1 and 2.
    let walked: BTreeSet<String> =
        walk_ids(&backend, &tenant, "v2|d|2020-01-01T00:00:01.000Z", 50, 0)
            .await
            .concat()
            .into_iter()
            .collect();
    let expected: BTreeSet<String> = fixture.live["Observation"]
        .iter()
        .filter(|id| {
            let i: usize = id.trim_start_matches("obs-").parse().unwrap();
            !i.is_multiple_of(3)
        })
        .cloned()
        .collect();
    assert_eq!(walked, expected);

    let needle = format!("tenant={}", tenant.tenant_id().as_str());
    let finished = walk_log_lines(&["mongodb reindex id phase finished", &needle]);
    assert_eq!(finished.len(), 1, "{finished:?}");
    assert_eq!(
        log_field_value(&finished[0], "floor"),
        "2020-01-01T00:00:01.000Z",
        "{}",
        finished[0]
    );
    assert_eq!(
        walk_log_lines(&["mongodb reindex catch-up round started", &needle, "round=1"]).len(),
        1
    );
    assert!(
        walk_log_lines(&["mongodb reindex walk started", &needle]).is_empty(),
        "a catch-up cursor never restarts the walk"
    );
}

/// A mutation run from inside a source fetch.
type Mutation = Box<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>;

/// Delegates every `ReindexSource` method to a real backend, and counts what
/// the id ranges return: fetches whose cursor carries the backend's range tag
/// `v2|r|`, ahead fetches included when they return a page. Keeps every plan
/// it hands the driver, and can run a mutation once, right after the first
/// fetch of one range returns and before the driver gets that page (#1403).
struct RangeCountingSource {
    inner: Arc<MongoBackend>,
    range_fetches: AtomicU64,
    range_resources: AtomicU64,
    plans: std::sync::Mutex<Vec<TypeWalkPlan>>,
    trigger: Option<(usize, Mutation)>,
    fired: AtomicBool,
}

impl RangeCountingSource {
    fn new(inner: Arc<MongoBackend>) -> Self {
        Self {
            inner,
            range_fetches: AtomicU64::new(0),
            range_resources: AtomicU64::new(0),
            plans: std::sync::Mutex::new(Vec::new()),
            trigger: None,
            fired: AtomicBool::new(false),
        }
    }

    /// Runs `mutation` once, after the first fetch of range `range` (the
    /// cursor the plan handed out for it) returns.
    fn with_trigger(mut self, range: usize, mutation: Mutation) -> Self {
        self.trigger = Some((range, mutation));
        self
    }

    fn count(&self, cursor: Option<&str>, page: &ResourcePage) {
        if cursor.is_some_and(|c| c.starts_with("v2|r|")) {
            self.range_fetches.fetch_add(1, Ordering::SeqCst);
            self.range_resources
                .fetch_add(page.resources.len() as u64, Ordering::SeqCst);
        }
    }

    fn range_start(&self, range: usize) -> Option<String> {
        match self.plans.lock().unwrap().last() {
            Some(TypeWalkPlan::Ranges { ranges, .. }) => ranges.get(range).cloned(),
            _ => None,
        }
    }
}

#[async_trait]
impl ReindexSource for RangeCountingSource {
    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>> {
        self.inner.list_resource_types(tenant).await
    }

    async fn count_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64> {
        self.inner.count_resources(tenant, resource_type).await
    }

    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> StorageResult<ResourcePage> {
        let page = self
            .inner
            .fetch_resources_page(tenant, resource_type, cursor, limit)
            .await?;
        self.count(cursor, &page);
        Ok(page)
    }

    async fn fetch_resources_page_capped(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
        max_bytes: u64,
    ) -> StorageResult<ResourcePage> {
        let page = self
            .inner
            .fetch_resources_page_capped(tenant, resource_type, cursor, limit, max_bytes)
            .await?;
        self.count(cursor, &page);
        if let Some((range, mutation)) = &self.trigger
            && cursor.is_some()
            && cursor.map(str::to_string) == self.range_start(*range)
            && !self.fired.swap(true, Ordering::SeqCst)
        {
            mutation().await;
        }
        Ok(page)
    }

    async fn fetch_resources_by_ids(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        ids: &[String],
    ) -> StorageResult<Vec<StoredResource>> {
        self.inner
            .fetch_resources_by_ids(tenant, resource_type, ids)
            .await
    }

    fn may_prefetch_page(&self, cursor: &str) -> bool {
        self.inner.may_prefetch_page(cursor)
    }

    async fn fetch_resources_page_ahead(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: &str,
        limit: u32,
        max_bytes: u64,
    ) -> StorageResult<Option<ResourcePage>> {
        let page = self
            .inner
            .fetch_resources_page_ahead(tenant, resource_type, cursor, limit, max_bytes)
            .await?;
        if let Some(page) = &page {
            self.count(Some(cursor), page);
        }
        Ok(page)
    }

    async fn plan_type_walk(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        request: TypeWalkRequest,
    ) -> StorageResult<TypeWalkPlan> {
        let plan = self
            .inner
            .plan_type_walk(tenant, resource_type, request)
            .await?;
        self.plans.lock().unwrap().push(plan.clone());
        Ok(plan)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_reindex_write_streams_match_single_stream() {
    let Some(backend) = create_streams_backend("reindex_streams_match_single").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant_a = create_tenant("streams-match-a");
    let tenant_b = create_tenant("streams-match-b");
    let fixture_a = seed_walk_fixture(&backend, &tenant_a, 2_000, "extra").await;
    backdate_fixture(&backend, &tenant_a, &fixture_a).await;
    let fixture_b = seed_walk_fixture(&backend, &tenant_b, 200, "extra").await;
    backdate_fixture(&backend, &tenant_b, &fixture_b).await;
    let live_observations = fixture_a.live["Observation"].len() as u64;

    let plan = backend
        .plan_type_walk(
            &tenant_a,
            "Observation",
            TypeWalkRequest {
                streams: 4,
                min_resources_per_stream: 500,
                concurrent_runs: 1,
            },
        )
        .await
        .unwrap();
    match &plan {
        TypeWalkPlan::Ranges { ranges, catch_up } => {
            assert_eq!(ranges.len(), 4, "{plan:?}");
            assert!(ranges.iter().all(|c| c.starts_with("v2|r|")), "{ranges:?}");
            assert!(catch_up.starts_with("v2|d|"), "{catch_up}");
        }
        TypeWalkPlan::Single => {
            panic!("2,000 Observations at 500 per stream must plan four ranges")
        }
    }

    let db = backend.get_database().await.unwrap();
    let tenant_b_before = snapshot(&db, "streams-match-b", false).await;

    let mut snapshots = Vec::new();
    for (label, streams, batch_bytes) in [
        ("one stream", 1, 0),
        ("four streams", 4, 0),
        ("four streams, 4 KiB pages", 4, 4096),
    ] {
        let source = Arc::new(RangeCountingSource::new(backend.clone()));
        let op = ReindexOperation::with_parts(
            source.clone(),
            vec![backend.clone() as Arc<dyn ReindexTarget>],
            backend.tenant_registries().clone(),
        );
        let job = op
            .start(
                tenant_a.clone(),
                ReindexRequest::for_types(["Observation"])
                    .with_batch_size(100)
                    .with_batch_bytes(batch_bytes)
                    .with_write_streams(streams)
                    .with_min_resources_per_stream(500)
                    .clear_existing(),
                None,
            )
            .await
            .unwrap();
        let progress = wait_for_terminal(&op, &job).await;
        assert_eq!(
            progress.status,
            ReindexStatus::Completed,
            "{label}: {progress:?}"
        );
        assert!(progress.errors.is_empty(), "{label}: {:?}", progress.errors);
        if streams > 1 {
            assert_eq!(
                source.range_resources.load(Ordering::SeqCst),
                live_observations,
                "{label}: the ranges must return every live Observation exactly once"
            );
            let plans = source.plans.lock().unwrap().clone();
            assert!(
                matches!(plans.as_slice(), [TypeWalkPlan::Ranges { ranges, .. }] if ranges.len() == 4),
                "{label}: {plans:?}"
            );
        } else {
            assert_eq!(
                source.range_fetches.load(Ordering::SeqCst),
                0,
                "{label}: a single walk never uses a range cursor"
            );
        }
        snapshots.push((label, snapshot(&db, "streams-match-a", false).await));
    }

    let (_, baseline) = &snapshots[0];
    assert!(
        !baseline.0.is_empty(),
        "the single-stream rebuild must have written search_index rows"
    );
    for (label, rows) in &snapshots[1..] {
        assert_eq!(
            rows, baseline,
            "{label}: rows differ from the single-stream rebuild"
        );
    }
    assert_eq!(
        snapshot(&db, "streams-match-b", false).await,
        tenant_b_before,
        "tenant B's rows must be untouched"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_streams_catch_up_indexes_a_resource_created_during_the_ranges() {
    use helios_persistence::core::{BulkProcessingOptions, BulkSubmitProvider, NdjsonEntry};

    let Some(backend) = create_streams_backend("reindex_streams_catch_up").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("streams-catch-up");
    let fixture = seed_walk_fixture(&backend, &tenant, 800, "extra").await;
    backdate_fixture(&backend, &tenant, &fixture).await;
    let (submission, manifest) = super::bulk_submit::seed(&backend, &tenant).await;
    let live_observations = fixture.live["Observation"].len() as u64;

    let mutation_backend = backend.clone();
    let mutation_tenant = tenant.clone();
    let mutation: Mutation = Box::new(move || {
        let backend = mutation_backend.clone();
        let tenant = mutation_tenant.clone();
        let submission = submission.clone();
        let manifest = manifest.clone();
        Box::pin(async move {
            backend
                .process_entries(
                    &tenant,
                    &submission,
                    &manifest,
                    vec![NdjsonEntry::new(
                        1,
                        "Observation",
                        json!({
                            "resourceType": "Observation",
                            "id": "--created-mid-walk",
                            "status": "final",
                            "code": { "coding": [{ "system": "http://loinc.org", "code": "8867-4" }] },
                        }),
                    )],
                    &BulkProcessingOptions::new().with_defer_indexing(true),
                )
                .await
                .unwrap();
            // A deferred create writes no search rows of its own, so rows
            // found at the end can only come from the rebuild.
            assert_eq!(
                search_index_entry_count(&backend, &tenant, "Observation", "--created-mid-walk")
                    .await,
                0
            );
        }) as futures::future::BoxFuture<'static, ()>
    });
    let source = Arc::new(RangeCountingSource::new(backend.clone()).with_trigger(1, mutation));
    let op = ReindexOperation::with_parts(
        source.clone(),
        vec![backend.clone() as Arc<dyn ReindexTarget>],
        backend.tenant_registries().clone(),
    );
    let job = op
        .start(
            tenant.clone(),
            ReindexRequest::for_types(["Observation"])
                .with_batch_size(50)
                .with_write_streams(4)
                .with_min_resources_per_stream(100),
            None,
        )
        .await
        .unwrap();
    let progress = wait_for_terminal(&op, &job).await;
    assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
    assert!(
        source.fired.load(Ordering::SeqCst),
        "the create ran during range 1"
    );
    assert!(source.range_fetches.load(Ordering::SeqCst) > 0);
    assert_eq!(
        source.range_resources.load(Ordering::SeqCst),
        live_observations,
        "the ranges walk exactly the Observations stamped before the floor"
    );
    assert!(
        search_index_entry_count(&backend, &tenant, "Observation", "--created-mid-walk").await > 0,
        "the catch-up must index a resource created while the ranges ran"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_streams_log_id_phase_finished_after_every_range_page_is_written() {
    let Some(backend) = create_streams_backend("reindex_streams_log_order").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("streams-log-order");
    let fixture = seed_walk_fixture(&backend, &tenant, 400, "extra").await;
    backdate_fixture(&backend, &tenant, &fixture).await;

    capture_walk_logs();
    let saw_transition_early = Arc::new(AtomicBool::new(false));
    let probe = Arc::new(super::reindex_pipeline::PhaseLogProbeTarget {
        backend: backend.clone(),
        saw_transition_early: saw_transition_early.clone(),
    });
    let op = ReindexOperation::with_parts(
        backend.clone(),
        vec![probe as Arc<dyn ReindexTarget>],
        backend.tenant_registries().clone(),
    );
    let job = op
        .start(
            tenant.clone(),
            ReindexRequest::for_types(["Observation"])
                .with_batch_size(50)
                .with_write_streams(4)
                .with_min_resources_per_stream(50),
            None,
        )
        .await
        .unwrap();
    let progress = wait_for_terminal(&op, &job).await;
    assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
    assert!(
        !saw_transition_early.load(Ordering::SeqCst),
        "id phase finished must never be logged while a range page is being written"
    );

    let needle = format!("tenant={}", tenant.tenant_id().as_str());
    assert_eq!(
        walk_log_lines(&["mongodb reindex walk started", &needle]).len(),
        1,
        "the plan fixes the floor once for the whole type"
    );
    assert_eq!(
        walk_log_lines(&["mongodb reindex id phase finished", &needle]).len(),
        1
    );
    let planned = walk_log_lines(&["mongodb reindex streams planned", &needle]);
    assert_eq!(planned.len(), 1, "{planned:?}");
    assert_eq!(
        log_field_value(&planned[0], "requested"),
        "4",
        "{}",
        planned[0]
    );
    assert_eq!(
        log_field_value(&planned[0], "allowed"),
        "4",
        "{}",
        planned[0]
    );
    assert_eq!(
        log_field_value(&planned[0], "streams"),
        "4",
        "{}",
        planned[0]
    );
    let ranges = walk_log_lines(&["mongodb reindex id range finished", &needle]);
    assert_eq!(ranges.len(), 4, "{ranges:?}");
    for line in &ranges {
        assert_eq!(
            field_names_after(line, "mongodb reindex id range finished"),
            ["tenant", "resource_type", "floor", "lo", "hi"]
        );
    }
    let open_lo = ranges
        .iter()
        .filter(|l| log_field_value(l, "lo") == "*")
        .count();
    let open_hi = ranges
        .iter()
        .filter(|l| log_field_value(l, "hi") == "*")
        .count();
    assert_eq!((open_lo, open_hi), (1, 1), "{ranges:?}");
}
