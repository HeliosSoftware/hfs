//! #1499: MongoDB honours `HFS_REINDEX_BATCH_BYTES` on the automatic
//! `$reindex` rebuild. Child module of the `mongodb_tests` root — `use
//! super::*;` reaches its private harness (`create_tenant`, `raw_test_client`,
//! `build_backend`, `repo_data_dir`, `build_test_database_name`,
//! `shared_mongo`, plus `Bson`/`Document`/`doc`/`json`/`FhirVersion`/`Client`,
//! all imported at the test-crate root), the same arrangement as
//! `tests/mongodb/reindex_id_walk.rs`.

use super::*;

use futures::TryStreamExt;
use helios_persistence::search::{ReindexSource, ResourcePage};

/// Builds a `MongoBackend` for this file's tests: same shape as
/// `create_backend_with_search_offloaded` (`mongodb_tests.rs`), but
/// lets the caller tweak the config first — used by [`create_id_phase_backend`]
/// below, and directly by PR2b's own tests (S3 §5.13), so this helper stays a
/// plain pass-through with no default catch-up-margin override of its own.
async fn create_backend_with(
    test_name: &str,
    configure: impl FnOnce(&mut MongoBackendConfig),
) -> Option<Arc<MongoBackend>> {
    let connection_string = shared_mongo::connection_string().await?;
    let mut config = MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name(test_name),
        data_dir: Some(repo_data_dir()),
        ..Default::default()
    };
    configure(&mut config);
    build_backend(config).await.map(Arc::new)
}

/// Builds a `MongoBackend` with the catch-up margin shortened to 1 s, so a
/// walk over already-seeded fixture rows can be moved into its id phase by
/// [`settle_into_id_phase`] (S3 §4.5's "id-phase fixture rule"). Every test
/// that exercises the id phase specifically — as opposed to a resource just
/// created, which the default 120 s margin would fold into catch-up round 1
/// (S2 §3.2: `floor = min(newest_live + 1 ms, t0 - margin)`, which with a
/// 120 s margin and resources seeded moments ago sits in the past relative to
/// nothing, so every seeded row has `last_updated >= floor` and the id phase's
/// first query comes back empty) — must use this instead of
/// [`create_backend_with`] directly.
async fn create_id_phase_backend(test_name: &str) -> Option<Arc<MongoBackend>> {
    create_backend_with(test_name, |c| c.reindex_catch_up_margin_ms = 1_000).await
}

/// Sleeps past [`create_id_phase_backend`]'s shortened margin, so every row
/// seeded before this call has a `last_updated` older than any walk's floor
/// and is walked by the id phase rather than folded into catch-up round 1.
/// Call it after seeding, before the first `fetch_resources_page_capped` /
/// `fetch_resources_page` call of the test.
async fn settle_into_id_phase() {
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
}

/// Reads each row's exact stored size the same way PostgreSQL's boundary test
/// does, via `$bsonSize` (needs MongoDB 4.4+; the floor is 5.0.6), sorted by
/// `id` to match the id phase's scan order. `$bsonSize` returns a 32-bit
/// `NumberInt` (MongoDB computes it as `Value(doc.toBson().objsize())`, and
/// `objsize()` is a 32-bit `int`), so `get_i64` — which bson 2.15.0 accepts
/// only for an actual `Bson::Int64` — is not safe to call on it; accept
/// either width explicitly.
async fn reindex_resource_row_sizes(
    db: &mongodb::Database,
    tenant_id: &str,
    resource_type: &str,
) -> Vec<(String, u64)> {
    let mut cursor = db
        .collection::<Document>("resources")
        .aggregate(vec![
            doc! { "$match": { "tenant_id": tenant_id, "resource_type": resource_type, "is_deleted": false } },
            doc! { "$sort": { "id": 1 } },
            doc! { "$project": { "id": 1, "size": { "$bsonSize": "$$ROOT" } } },
        ])
        .await
        .unwrap();
    let mut rows = Vec::new();
    while let Some(doc) = cursor.try_next().await.unwrap() {
        let id = doc.get_str("id").unwrap().to_string();
        let size = match doc.get("size") {
            Some(Bson::Int32(n)) => *n as u64,
            Some(Bson::Int64(n)) => *n as u64,
            other => panic!("$bsonSize returned {other:?}"),
        };
        rows.push((id, size));
    }
    rows
}

/// Walks `resource_type` to completion through `fetch_resources_page_capped`,
/// returning every non-empty page in fetch order. PR1 guarantees exactly one
/// trailing empty page per type (S2 D12), so more than 20 fetches means the
/// walk is not terminating (S3 §4.5's guard).
async fn walk_capped(
    backend: &MongoBackend,
    tenant: &TenantContext,
    resource_type: &str,
    limit: u32,
    max_bytes: u64,
) -> Vec<ResourcePage> {
    let mut cursor: Option<String> = None;
    let mut pages = Vec::new();
    for _ in 0..20 {
        let page = backend
            .fetch_resources_page_capped(tenant, resource_type, cursor.as_deref(), limit, max_bytes)
            .await
            .unwrap();
        let done = page.next_cursor.is_none();
        cursor = page.next_cursor.clone();
        if !page.resources.is_empty() {
            pages.push(page);
        }
        if done {
            return pages;
        }
    }
    panic!("walk of {resource_type} did not reach its trailing empty page within 20 fetches");
}

#[tokio::test]
async fn mongodb_integration_reindex_fetch_capped_boundaries() {
    let Some(backend) = create_id_phase_backend("reindex_capped_boundaries").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-capped-boundaries");
    for (id, k) in [("p01", 1usize), ("p02", 2), ("p03", 3), ("p04", 4)] {
        backend
            .create_or_update(
                &tenant,
                "Patient",
                id,
                json!({"resourceType": "Patient", "id": id, "name": [{"family": "X".repeat(200 * k)}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    settle_into_id_phase().await;
    let db = raw_test_client(&backend.config().connection_string)
        .await
        .unwrap()
        .database(&backend.config().database_name);
    let sizes = reindex_resource_row_sizes(&db, "tenant-capped-boundaries", "Patient").await;
    let ids: Vec<_> = sizes.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["p01", "p02", "p03", "p04"]);
    let first = sizes[0].1;
    let two = sizes[0].1 + sizes[1].1;

    for (cap, expected_first) in [
        (two, vec!["p01", "p02"]),
        (two - 1, vec!["p01"]),
        (first - 1, vec!["p01"]),
        (two + 1, vec!["p01", "p02"]),
    ] {
        let mut cursor: Option<String> = None;
        let mut seen = Vec::new();
        let mut first_page = true;
        let mut finished = false;
        for _ in 0..20 {
            let page = backend
                .fetch_resources_page_capped(&tenant, "Patient", cursor.as_deref(), 4, cap)
                .await
                .unwrap();
            let page_ids: Vec<String> = page.resources.iter().map(|r| r.id().to_string()).collect();
            let page_bytes: u64 = page_ids
                .iter()
                .map(|id| sizes.iter().find(|(key, _)| key == id).unwrap().1)
                .sum();
            assert!(
                page_bytes <= cap || page_ids.len() == 1,
                "cap {cap}: page {page_ids:?} totalled {page_bytes} bytes"
            );
            if first_page && !page_ids.is_empty() {
                assert_eq!(page_ids, expected_first, "cap {cap}: first page");
                assert!(
                    page.next_cursor
                        .as_deref()
                        .is_some_and(|c| c.starts_with("v2|i|")),
                    "cap {cap}: a capped page must stay in the id phase"
                );
                first_page = false;
            }
            seen.extend(page_ids);
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => {
                    finished = true;
                    break;
                }
            }
        }
        assert!(
            finished,
            "cap {cap}: walk did not terminate within 20 fetches"
        );
        assert_eq!(seen, ["p01", "p02", "p03", "p04"], "cap {cap}");
    }
}

#[tokio::test]
async fn mongodb_integration_reindex_fetch_capped_zero_cap_matches_uncapped() {
    let Some(backend) = create_id_phase_backend("reindex_zero_cap_matches").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-zero-cap-matches");
    for id in ["p01", "p02", "p03", "p04"] {
        backend
            .create_or_update(
                &tenant,
                "Patient",
                id,
                json!({"resourceType": "Patient", "id": id}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    settle_into_id_phase().await;
    for limit in [2u32, 4] {
        let capped = walk_capped(&backend, &tenant, "Patient", limit, 0).await;

        let mut cursor: Option<String> = None;
        let mut uncapped = Vec::new();
        for _ in 0..20 {
            let page = backend
                .fetch_resources_page(&tenant, "Patient", cursor.as_deref(), limit)
                .await
                .unwrap();
            let done = page.next_cursor.is_none();
            cursor = page.next_cursor.clone();
            if !page.resources.is_empty() {
                uncapped.push(page);
            }
            if done {
                break;
            }
        }

        assert_eq!(capped.len(), uncapped.len(), "limit {limit}: page count");
        for (c, u) in capped.iter().zip(uncapped.iter()) {
            let c_ids: Vec<_> = c.resources.iter().map(|r| r.id().to_string()).collect();
            let u_ids: Vec<_> = u.resources.iter().map(|r| r.id().to_string()).collect();
            assert_eq!(c_ids, u_ids, "limit {limit}");
            if c.next_cursor
                .as_deref()
                .is_some_and(|s| s.starts_with("v2|i|"))
            {
                assert_eq!(
                    c.next_cursor, u.next_cursor,
                    "limit {limit}: id-phase cursors must match exactly"
                );
            }
        }
    }
}

#[tokio::test]
async fn mongodb_integration_reindex_fetch_capped_limit_zero_reads_one_per_page() {
    let Some(backend) = create_id_phase_backend("reindex_limit_zero").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-limit-zero");
    for id in ["p01", "p02"] {
        backend
            .create_or_update(
                &tenant,
                "Patient",
                id,
                json!({"resourceType": "Patient", "id": id}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    settle_into_id_phase().await;
    for max_bytes in [0u64, u64::MAX] {
        let page = backend
            .fetch_resources_page_capped(&tenant, "Patient", None, 0, max_bytes)
            .await
            .unwrap();
        assert_eq!(page.resources.len(), 1, "max_bytes {max_bytes}");
        assert!(
            page.next_cursor.is_some(),
            "max_bytes {max_bytes}: must continue"
        );
    }
}

#[tokio::test]
async fn mongodb_integration_reindex_fetch_capped_returns_a_resource_larger_than_the_cap() {
    let Some(backend) = create_id_phase_backend("reindex_larger_than_cap").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-larger-than-cap");
    for id in ["p01", "p02"] {
        backend
            .create_or_update(
                &tenant,
                "Patient",
                id,
                json!({"resourceType": "Patient", "id": id, "name": [{"family": "X".repeat(500)}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    settle_into_id_phase().await;
    let pages = walk_capped(&backend, &tenant, "Patient", 10, 2).await;
    for page in &pages {
        assert_eq!(
            page.resources.len(),
            1,
            "every page must hold exactly one over-cap resource"
        );
    }
    let seen: Vec<String> = pages
        .iter()
        .flat_map(|p| p.resources.iter().map(|r| r.id().to_string()))
        .collect();
    assert_eq!(seen, ["p01", "p02"]);
}
