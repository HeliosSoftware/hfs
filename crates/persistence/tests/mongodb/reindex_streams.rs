//! #1403: concurrent write streams over disjoint id ranges for the MongoDB
//! `$reindex` rebuild — the range and catch-up cursors, the stream plan, and
//! whole rebuilds split across streams. A `#[path]`-included child module of
//! `mongodb_tests.rs`, like `reindex_id_walk.rs`: `use super::*` reaches the
//! parent's harness (`create_backend`, `create_tenant`, `shared_mongo`, …).

use super::*;

use std::collections::BTreeSet;

use helios_persistence::search::{ReindexSource, ResourcePage, TypeWalkPlan, TypeWalkRequest};

use super::reindex_id_walk::{
    backdate_fixture, capture_walk_logs, seed_walk_fixture, walk_log_lines,
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
    // continues that range, and each range ends on one empty page.
    let mut seen = BTreeSet::new();
    for (i, range) in ranges.iter().enumerate() {
        let mut cursor = range.clone();
        let mut in_range: Vec<String> = Vec::new();
        let mut ended = false;
        for _ in 0..1_000 {
            let page = backend
                .fetch_resources_page_capped(&tenant, "Observation", Some(&cursor), 100, 1)
                .await
                .unwrap();
            let page_ids = ids_of(&page);
            match page.next_cursor {
                Some(next) => {
                    assert_eq!(page_ids.len(), 1, "range {i}: {page_ids:?}");
                    in_range.extend(page_ids);
                    cursor = next;
                }
                None => {
                    assert!(page_ids.is_empty(), "range {i} ends on one empty page");
                    ended = true;
                    break;
                }
            }
        }
        assert!(ended, "range {i} did not end within 1,000 pages");
        assert!(!in_range.is_empty(), "range {i} is empty");
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
        let tokens: Vec<&str> = line.split_whitespace().collect();
        for expected in ["requested=4", "allowed=1", "resources=0", "streams=1"] {
            assert!(tokens.contains(&expected), "{line}");
        }
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
    let first: Vec<&str> = planned[0].split_whitespace().collect();
    for expected in ["allowed=2", "resources=200", "streams=2"] {
        assert!(first.contains(&expected), "{}", planned[0]);
    }
    let second: Vec<&str> = planned[1].split_whitespace().collect();
    for expected in ["allowed=4", "resources=200", "streams=1"] {
        assert!(second.contains(&expected), "{}", planned[1]);
    }
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
