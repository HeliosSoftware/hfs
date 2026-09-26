//! #1403: concurrent write streams over disjoint id ranges for the MongoDB
//! `$reindex` rebuild — the range and catch-up cursors, the stream plan, and
//! whole rebuilds split across streams. A `#[path]`-included child module of
//! `mongodb_tests.rs`, like `reindex_id_walk.rs`: `use super::*` reaches the
//! parent's harness (`create_backend`, `create_tenant`, `shared_mongo`, …).

use super::*;

use std::collections::BTreeSet;

use helios_persistence::search::{ReindexSource, ResourcePage};

use super::reindex_id_walk::{
    backdate_fixture, capture_walk_logs, seed_walk_fixture, walk_log_lines,
};
use super::reindex_pipeline::log_field_value;

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
/// one resource, and the walk ends on one empty page with none. Returns every
/// id, in fetch order.
async fn walk_ids(
    backend: &MongoBackend,
    tenant: &TenantContext,
    cursor: &str,
    limit: u32,
) -> Vec<String> {
    let mut ids = Vec::new();
    let mut cursor = cursor.to_string();
    for _ in 0..500 {
        let page = backend
            .fetch_resources_page_capped(tenant, "Observation", Some(&cursor), limit, 0)
            .await
            .unwrap();
        let page_ids = ids_of(&page);
        match page.next_cursor {
            Some(next) => {
                assert!(
                    !page_ids.is_empty(),
                    "a page with a next cursor must hold a resource"
                );
                ids.extend(page_ids);
                cursor = next;
            }
            None => {
                assert!(page_ids.is_empty(), "a walk ends on one empty page");
                return ids;
            }
        }
    }
    panic!("the walk from {cursor} did not end within 500 pages");
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
        let ids = walk_ids(&backend, &tenant, &range_cursor(lo, hi), 7).await;
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
    let walked: BTreeSet<String> = walk_ids(&backend, &tenant, "v2|d|2020-01-01T00:00:01.000Z", 50)
        .await
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
