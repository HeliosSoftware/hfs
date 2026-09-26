//! #1499: MongoDB honours `HFS_REINDEX_BATCH_BYTES` on the automatic
//! `$reindex` rebuild. Child module of the `mongodb_tests` root — `use
//! super::*;` reaches its private harness (`create_tenant`, `raw_test_client`,
//! `build_backend`, `repo_data_dir`, `build_test_database_name`,
//! `shared_mongo`, plus `Bson`/`Document`/`doc`/`json`/`FhirVersion`/`Client`,
//! all imported at the test-crate root), the same arrangement as
//! `tests/mongodb/reindex_id_walk.rs`.

use super::*;

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::bulk_submit::FailPoint;
use async_trait::async_trait;
use futures::TryStreamExt;
use helios_persistence::error::StorageResult;
use helios_persistence::search::{
    ReindexOperation, ReindexPageStats, ReindexRequest, ReindexSource, ReindexStatus,
    ReindexTarget, ResourcePage,
};
use helios_persistence::types::StoredResource;

/// Builds a `MongoBackend` for this file's tests: same shape as
/// `create_backend_with_search_offloaded` (`mongodb_tests.rs`), but
/// lets the caller tweak the config first — used by [`create_id_phase_backend`]
/// below, and directly by other tests in this file that need an unmodified
/// config, so this helper stays a plain pass-through with no default
/// catch-up-margin override of its own.
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
/// [`settle_into_id_phase`] below. Every test that exercises the id phase
/// specifically — as opposed to a resource just created, which the default
/// 120 s margin would fold into catch-up round 1 (`floor = min(newest_live +
/// 1 ms, t0 - margin)`) — must use this instead of [`create_backend_with`]
/// directly: with the normal 120 s margin, `floor = t0 - 120 s` is earlier
/// than every row the test just seeded, so the id phase (`last_updated <
/// floor`) would come back empty; this helper shortens the margin so the
/// seeded rows fall in the id phase instead.
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
/// returning every non-empty page in fetch order. The id-order walk guarantees
/// exactly one trailing empty page per type, so more than 20 fetches means the
/// walk is not terminating.
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

/// `n` Provenance resources shaped like the #1403 corpus's extreme case: one
/// agent and 1,600 `target` references each, ~70-100 KB of BSON per resource.
fn provenance_fixture(n: usize) -> Vec<(String, serde_json::Value)> {
    (0..n)
        .map(|i| {
            let id = format!("prov-{i:02}");
            let targets: Vec<serde_json::Value> = (0..1600)
                .map(|k| json!({ "reference": format!("Observation/{id}-{k:05}") }))
                .collect();
            (
                id.clone(),
                json!({
                    "resourceType": "Provenance",
                    "id": id,
                    "agent": [{ "who": { "reference": "Practitioner/example" } }],
                    "target": targets,
                }),
            )
        })
        .collect()
}

/// Seeds a 24-resource Provenance fixture, settles it into the id phase, and
/// returns the raw database handle, each row's id-sorted stored size, and the
/// byte cap that admits exactly the three smallest resources. Shared by the
/// two Provenance-shaped tests below.
async fn seed_provenance(
    backend: &MongoBackend,
    tenant: &TenantContext,
    tenant_id: &str,
) -> (mongodb::Database, Vec<(String, u64)>, u64) {
    let fixture = provenance_fixture(24);
    for (id, resource) in &fixture {
        backend
            .create_or_update(
                tenant,
                "Provenance",
                id,
                resource.clone(),
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
    let sizes = reindex_resource_row_sizes(&db, tenant_id, "Provenance").await;
    let mut by_size = sizes.clone();
    by_size.sort_by_key(|(_, size)| *size);
    let cap: u64 = by_size.iter().take(3).map(|(_, size)| *size).sum();
    (db, sizes, cap)
}

#[tokio::test]
async fn mongodb_integration_reindex_fetch_capped_provenance_shaped() {
    let Some(backend) = create_id_phase_backend("reindex_provenance_shaped").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant_id = "tenant-provenance-shaped";
    let tenant = create_tenant(tenant_id);
    let (_db, sizes, cap) = seed_provenance(&backend, &tenant, tenant_id).await;

    let pages = walk_capped(&backend, &tenant, "Provenance", 24, cap).await;
    for page in &pages {
        assert!(
            page.resources.len() <= 3,
            "page held {} resources",
            page.resources.len()
        );
        let bytes: u64 = page
            .resources
            .iter()
            .map(|r| sizes.iter().find(|(id, _)| id == r.id()).unwrap().1)
            .sum();
        assert!(bytes <= cap || page.resources.len() == 1);
    }
    let seen: Vec<String> = pages
        .iter()
        .flat_map(|p| p.resources.iter().map(|r| r.id().to_string()))
        .collect();
    assert_eq!(
        seen.len(),
        24,
        "every resource must come back at least once"
    );
    let deduped: std::collections::BTreeSet<&String> = seen.iter().collect();
    assert_eq!(
        deduped.len(),
        24,
        "every resource must come back exactly once"
    );

    let unbounded = walk_capped(&backend, &tenant, "Provenance", 5, u64::MAX).await;
    let last_index = unbounded.len().saturating_sub(1);
    for (i, page) in unbounded.iter().enumerate() {
        assert!(
            page.next_cursor
                .as_deref()
                .is_some_and(|c| c.starts_with("v2|i|"))
                || i == last_index,
            "page {i} of an id-only limit-5 walk must stay in the id phase"
        );
        if i == last_index {
            assert!(page.resources.len() <= 5);
        } else {
            assert_eq!(
                page.resources.len(),
                5,
                "page {i} of an id-only limit-5 walk"
            );
        }
    }
}

/// `fetch_resources_page_ahead` must build the exact same page as
/// `fetch_resources_page_capped` for every id-phase cursor, and must decline
/// (`Ok(None)`) the one cursor whose query is empty, where the walk leaves
/// the id phase for its first catch-up round (#1403).
#[tokio::test]
async fn mongodb_integration_reindex_fetch_ahead_matches_capped_id_phase() {
    let Some(backend) = create_id_phase_backend("reindex_fetch_ahead_matches").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant_id = "tenant-fetch-ahead-matches";
    let tenant = create_tenant(tenant_id);
    let (_db, sizes, cap) = seed_provenance(&backend, &tenant, tenant_id).await;

    let mut cursor = backend
        .fetch_resources_page_capped(&tenant, "Provenance", None, 24, cap)
        .await
        .unwrap()
        .next_cursor
        .expect("the first page of a freshly seeded type must continue");
    assert!(
        cursor.starts_with("v2|i|"),
        "first page must stay in the id phase: {cursor}"
    );

    let mut declined_at_the_end = false;
    for _ in 0..20 {
        let ahead = backend
            .fetch_resources_page_ahead(&tenant, "Provenance", &cursor, 24, cap)
            .await
            .unwrap();
        let Some(ahead_page) = ahead else {
            declined_at_the_end = true;
            break;
        };
        let capped = backend
            .fetch_resources_page_capped(&tenant, "Provenance", Some(&cursor), 24, cap)
            .await
            .unwrap();
        let ahead_ids: Vec<_> = ahead_page
            .resources
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        let capped_ids: Vec<_> = capped
            .resources
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        assert_eq!(ahead_ids, capped_ids, "cursor {cursor}");
        assert_eq!(
            ahead_page.next_cursor, capped.next_cursor,
            "cursor {cursor}"
        );
        let bytes: u64 = ahead_page
            .resources
            .iter()
            .map(|r| sizes.iter().find(|(id, _)| id == r.id()).unwrap().1)
            .sum();
        assert!(
            bytes <= cap || ahead_page.resources.len() == 1,
            "page {ahead_ids:?} totalled {bytes} bytes, over cap {cap}"
        );
        let next = ahead_page
            .next_cursor
            .expect("a non-empty id-phase page must continue");
        assert!(
            next.starts_with("v2|i|"),
            "an ahead-fetched page must stay in the id phase: {next}"
        );
        cursor = next;
    }
    assert!(
        declined_at_the_end,
        "the id phase's final cursor must decline the ahead fetch"
    );
}

struct RecordingSource {
    inner: Arc<MongoBackend>,
    sizes: std::collections::HashMap<String, u64>,
    pages: Mutex<Vec<(usize, u64)>>,
}

#[async_trait]
impl ReindexSource for RecordingSource {
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
        self.inner
            .fetch_resources_page(tenant, resource_type, cursor, limit)
            .await
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
        let bytes: u64 = page
            .resources
            .iter()
            .map(|r| *self.sizes.get(r.id()).expect("fixture size"))
            .sum();
        self.pages
            .lock()
            .unwrap()
            .push((page.resources.len(), bytes));
        Ok(page)
    }
}

struct RecordingWriter {
    inner: Arc<MongoBackend>,
    written: AtomicU64,
}

#[async_trait]
impl ReindexTarget for RecordingWriter {
    async fn delete_search_entries(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<u64> {
        self.inner
            .delete_search_entries(tenant, resource_type, resource_id)
            .await
    }
    async fn write_search_entries(
        &self,
        tenant: &TenantContext,
        resource: &StoredResource,
    ) -> StorageResult<usize> {
        self.inner.write_search_entries(tenant, resource).await
    }
    async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
        self.inner.clear_search_index(tenant).await
    }
    async fn begin_bulk_index_rebuild(&self) -> StorageResult<()> {
        self.inner.begin_bulk_index_rebuild().await
    }
    async fn end_bulk_index_rebuild(&self) -> StorageResult<()> {
        self.inner.end_bulk_index_rebuild().await
    }
    // Delegates to `write_search_entries_page_timed` with a throwaway
    // `ReindexPageStats`, per `ReindexTarget::write_search_entries_page`'s
    // trait contract that an override MUST route the untimed page method
    // through it, so the two paths cannot diverge — mirrors MongoBackend's
    // own `ReindexTarget::write_search_entries_page` impl.
    async fn write_search_entries_page(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
    ) -> Vec<StorageResult<usize>> {
        let mut stats = ReindexPageStats::default();
        self.write_search_entries_page_timed(tenant, resources, &mut stats)
            .await
    }
    async fn write_search_entries_page_timed(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        stats: &mut ReindexPageStats,
    ) -> Vec<StorageResult<usize>> {
        let results = self
            .inner
            .write_search_entries_page_timed(tenant, resources, stats)
            .await;
        let ok: u64 = results
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .map(|n| *n as u64)
            .sum();
        self.written.fetch_add(ok, Ordering::SeqCst);
        results
    }
}

#[tokio::test]
async fn mongodb_integration_reindex_capped_run_bounds_every_page() {
    let Some(backend) = create_id_phase_backend("reindex_capped_run_bounds").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant_id = "tenant-capped-run-bounds";
    let tenant = create_tenant(tenant_id);
    let (db, sizes, cap) = seed_provenance(&backend, &tenant, tenant_id).await;

    let source = Arc::new(RecordingSource {
        inner: backend.clone(),
        sizes: sizes.into_iter().collect(),
        pages: Mutex::new(Vec::new()),
    });
    let writer = Arc::new(RecordingWriter {
        inner: backend.clone(),
        written: AtomicU64::new(0),
    });
    let operation = ReindexOperation::with_parts(
        source.clone(),
        vec![writer.clone()],
        backend.tenant_registries().clone(),
    );
    let request = ReindexRequest::for_types(["Provenance"])
        .with_batch_size(100)
        .with_batch_bytes(cap);
    let job_id = operation.start(tenant, request, None).await.unwrap();
    let progress = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let progress = operation.get_progress(&job_id).await.unwrap();
            if progress.status.is_finished() {
                break progress;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("reindex did not finish within 60s");

    assert_eq!(progress.status, ReindexStatus::Completed);
    assert!(progress.errors.is_empty(), "{:?}", progress.errors);
    assert_eq!(progress.processed_resources, 24);
    let recorded_pages = source.pages.lock().unwrap().clone();
    assert_eq!(
        recorded_pages
            .iter()
            .map(|(resources, _)| *resources)
            .sum::<usize>(),
        24,
        "pages must have been recorded, so the per-page bound below cannot pass vacuously"
    );
    for (resources, bytes) in &recorded_pages {
        assert!(*resources <= 3, "page held {resources} resources");
        assert!(
            *bytes <= cap || *resources == 1,
            "page held {resources} resources totalling {bytes} bytes, over cap {cap}"
        );
    }
    let own = db
        .collection::<Document>("search_index")
        .count_documents(doc! { "tenant_id": tenant_id, "resource_type": "Provenance" })
        .await
        .unwrap();
    let contained = db
        .collection::<Document>("search_index_contained")
        .count_documents(doc! { "tenant_id": tenant_id, "resource_type": "Provenance" })
        .await
        .unwrap();
    assert_eq!(progress.entries_created, own + contained);
    assert_eq!(
        writer.written.load(Ordering::SeqCst),
        progress.entries_created
    );
}

#[tokio::test]
async fn mongodb_integration_reindex_fetch_capped_page_never_spans_the_catch_up_boundary() {
    let Some(backend) = create_id_phase_backend("reindex_capped_catch_up_boundary").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant_id = "tenant-capped-catch-up-boundary";
    let tenant = create_tenant(tenant_id);
    for (id, k) in [
        ("p01", 1usize),
        ("p02", 2),
        ("p03", 3),
        ("p04", 4),
        ("p05", 5),
        ("p06", 6),
    ] {
        backend
            .create_or_update(
                &tenant,
                "Patient",
                id,
                json!({"resourceType": "Patient", "id": id, "name": [{"family": "X".repeat(100 * k)}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    // The id-phase fixture rule: sleep past the margin so every seeded row is
    // walked by the id phase, not folded into round 1.
    settle_into_id_phase().await;

    let db = raw_test_client(&backend.config().connection_string)
        .await
        .unwrap()
        .database(&backend.config().database_name);
    let sizes = reindex_resource_row_sizes(&db, tenant_id, "Patient").await;
    let cap: u64 = sizes
        .iter()
        .filter(|(id, _)| id != "p06")
        .map(|(_, size)| *size)
        .sum();

    let page1 = backend
        .fetch_resources_page_capped(&tenant, "Patient", None, 6, cap)
        .await
        .unwrap();
    let page1_ids: Vec<String> = page1.resources.iter().map(|r| r.id().to_string()).collect();
    assert_eq!(page1_ids, ["p01", "p02", "p03", "p04", "p05"]);
    assert!(
        page1
            .next_cursor
            .as_deref()
            .is_some_and(|c| c.starts_with("v2|i|"))
    );

    // A racing update to a row already on page 1, after it was written.
    backend
        .create_or_update(
            &tenant,
            "Patient",
            "p02",
            json!({"resourceType": "Patient", "id": "p02", "name": [{"family": "Updated"}]}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let mut cursor = page1.next_cursor;
    let mut saw_id_phase_p06 = false;
    let mut p02_round_hits = 0u32;
    let mut p02_round_family: Option<String> = None;
    let mut seen_ids: Vec<String> = page1_ids.clone();
    {
        let unique: std::collections::HashSet<&String> = page1_ids.iter().collect();
        assert_eq!(unique.len(), page1_ids.len(), "page 1 holds an id twice");
    }
    for _ in 0..20 {
        let page = backend
            .fetch_resources_page_capped(&tenant, "Patient", cursor.as_deref(), 6, cap)
            .await
            .unwrap();
        let ids: Vec<String> = page.resources.iter().map(|r| r.id().to_string()).collect();
        {
            let unique: std::collections::HashSet<&String> = ids.iter().collect();
            assert_eq!(unique.len(), ids.len(), "a page holds an id twice: {ids:?}");
        }
        if !ids.is_empty() {
            if let Some(next) = &page.next_cursor {
                if next.starts_with("v2|i|") {
                    assert_eq!(ids, ["p06"], "the id phase's last page must be exactly p06");
                    saw_id_phase_p06 = true;
                } else if next.starts_with("v2|c|") {
                    if let Some(p02) = page.resources.iter().find(|r| r.id() == "p02") {
                        p02_round_hits += 1;
                        p02_round_family = p02.content()["name"][0]["family"]
                            .as_str()
                            .map(str::to_string);
                    }
                }
            }
        }
        seen_ids.extend(ids);
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert!(
        saw_id_phase_p06,
        "p06 must be walked by the id phase, not folded into a capped page"
    );
    assert_eq!(
        p02_round_hits, 1,
        "p02 must be re-walked by exactly one catch-up round page"
    );
    assert_eq!(
        p02_round_family.as_deref(),
        Some("Updated"),
        "p02 must come back with its updated content"
    );

    let mut counts = std::collections::HashMap::new();
    for id in &seen_ids {
        *counts.entry(id.clone()).or_insert(0) += 1;
    }
    for id in ["p01", "p03", "p04", "p05", "p06"] {
        assert_eq!(counts.get(id), Some(&1), "{id} must appear exactly once");
    }
}

/// Covers the round-page gap left by every other test here: they all build
/// their backend through [`create_id_phase_backend`], so every capped page
/// they check is an id-phase page (`v2|i|`). This test uses [`create_backend_with`] with the
/// default 120 s margin instead: rows seeded moments ago are all newer than
/// the walk's floor, so the id phase's first query comes back empty and the
/// walker falls straight into catch-up round 1 within the same call
/// (`WalkStep::IdPhase` -> `WalkStep::RoundStart` -> `WalkStep::Round`), and
/// every page this test sees is a round page (`v2|c|1|`) built from more than
/// one row — exercising the round arm's own continuation (`scanned.last()`
/// before dedupe), its `walked` accounting, and the cap rule when a round
/// page holds multiple rows, none of which the single-row round page in
/// `..._page_never_spans_the_catch_up_boundary` reaches.
#[tokio::test]
async fn mongodb_integration_reindex_fetch_capped_round_page_bounds_multiple_rows() {
    let Some(backend) = create_backend_with("reindex_capped_round_multi_row", |_| {}).await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant_id = "tenant-capped-round-multi-row";
    let tenant = create_tenant(tenant_id);
    // No `settle_into_id_phase` here: this test deliberately keeps the
    // default margin so these rows land in round 1 instead.
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

    let db = raw_test_client(&backend.config().connection_string)
        .await
        .unwrap()
        .database(&backend.config().database_name);
    let sizes = reindex_resource_row_sizes(&db, tenant_id, "Patient").await;
    let mut by_size = sizes.clone();
    by_size.sort_by_key(|(_, size)| *size);
    let cap: u64 = by_size.iter().take(2).map(|(_, size)| *size).sum();

    let pages = walk_capped(&backend, &tenant, "Patient", 4, cap).await;
    assert!(!pages.is_empty(), "must return at least one page");

    let mut seen: Vec<String> = Vec::new();
    for page in &pages {
        let page_ids: Vec<String> = page.resources.iter().map(|r| r.id().to_string()).collect();
        let page_bytes: u64 = page_ids
            .iter()
            .map(|id| sizes.iter().find(|(key, _)| key == id).unwrap().1)
            .sum();
        assert!(
            page_bytes <= cap || page_ids.len() == 1,
            "page {page_ids:?} totalled {page_bytes} bytes, over cap {cap}"
        );
        if let Some(next) = &page.next_cursor {
            assert!(
                next.starts_with("v2|c|1|"),
                "a capped round-1 page's continuation cursor must stay in round 1, got {next}"
            );
        }
        seen.extend(page_ids);
    }
    assert!(
        pages.iter().any(|page| page.resources.len() > 1),
        "at least one page must hold more than one resource, or this test cannot tell the \
         round arm's multi-row cap rule apart from a page-per-row walk"
    );

    let mut counts = std::collections::HashMap::new();
    for id in &seen {
        *counts.entry(id.clone()).or_insert(0u32) += 1;
    }
    for id in ["p01", "p02", "p03", "p04"] {
        assert_eq!(counts.get(id), Some(&1), "{id} must appear exactly once");
    }
}

fn build_test_patients(tenant: &TenantContext, prefix: &str, n: usize) -> Vec<StoredResource> {
    (0..n)
        .map(|i| {
            StoredResource::from_storage(
                "Patient",
                format!("{prefix}-{i}"),
                "1",
                tenant.tenant_id().clone(),
                json!({
                    "resourceType": "Patient",
                    "id": format!("{prefix}-{i}"),
                    "name": [{"family": format!("F{i}")}]
                }),
                chrono::Utc::now(),
                chrono::Utc::now(),
                None,
                FhirVersion::default(),
            )
        })
        .collect()
}

/// The set of `resource_id`s that currently have at least one `search_index`
/// row for `tenant`/`resource_type`, via one client and one `distinct` query.
/// Replaces a per-resource `search_index_entry_count` loop — a fresh MongoDB
/// client per call — with a single round trip; a test compares the result
/// against the ids it expects to find indexed (#1403).
async fn indexed_resource_ids(
    backend: &MongoBackend,
    tenant: &TenantContext,
    resource_type: &str,
) -> std::collections::BTreeSet<String> {
    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("failed to connect MongoDB client for search_index assertions");
    let database = client.database(&backend.config().database_name);
    let search_index = database.collection::<Document>("search_index");
    let ids: Vec<Bson> = search_index
        .distinct(
            "resource_id",
            doc! {
                "tenant_id": tenant.tenant_id().as_str(),
                "resource_type": resource_type,
            },
        )
        .await
        .expect("failed to list distinct resource_id values");
    ids.into_iter()
        .filter_map(|b| b.as_str().map(str::to_string))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reindex_page_serial_pipeline_writes_a_large_page() {
    let Some(backend) = create_backend_with("reindex_serial_pipeline_large_page", |c| {
        c.reindex_prepare_threads = 1;
        c.reindex_overlap = false;
    })
    .await
    else {
        eprintln!(
            "Skipping mongodb_integration_reindex_page_serial_pipeline_writes_a_large_page (requires Docker)"
        );
        return;
    };

    let tenant = create_tenant("reindex-serial-pipeline-tenant");
    let page = build_test_patients(&tenant, "serialbig", 300);

    let target: &dyn ReindexTarget = &*backend;
    let mut stats = ReindexPageStats::default();
    let outcomes = target
        .write_search_entries_page_timed(&tenant, &page, &mut stats)
        .await;
    assert!(outcomes.iter().all(|o| o.is_ok()), "{outcomes:?}");
    assert_eq!(
        stats.sub_batches, 1,
        "the serial path always runs exactly one extraction pass, whatever the page size"
    );
    assert!(stats.inserted_entries as usize >= page.len());

    let indexed = indexed_resource_ids(&backend, &tenant, "Patient").await;
    let expected: std::collections::BTreeSet<String> =
        page.iter().map(|r| r.id().to_string()).collect();
    assert_eq!(indexed, expected, "every resource must have a row");
}

/// A scoped (not global) tracing subscriber for one test, so it does not
/// race `capture_walk_logs`'s own one-shot global subscriber for exclusive
/// use of `tracing::dispatcher::set_global_default` (only the first caller
/// in the whole `mongodb_tests` binary can win that race). `set_default`
/// installs a thread-local override instead, which is safe to set up per
/// test as long as the awaited work never leaves the calling OS thread — true
/// here since the test driving it uses the default (current-thread) `#[tokio::test]`
/// runtime (#1403).
fn capture_writer_config_log() -> (&'static Mutex<Vec<u8>>, tracing::dispatcher::DefaultGuard) {
    static BUF: std::sync::OnceLock<Mutex<Vec<u8>>> = std::sync::OnceLock::new();
    let buf = BUF.get_or_init(|| Mutex::new(Vec::new()));
    let writer = tracing_test::internal::MockWriter::new(buf);
    let dispatch = tracing_test::internal::get_subscriber(
        writer,
        "helios_persistence::backends::mongodb::reindex_pipeline=info",
    );
    let guard = tracing::dispatcher::set_default(&dispatch);
    (buf, guard)
}

/// The value substring `line` gives for `field=`, up to the next space:
/// panics if the field is missing.
pub(super) fn log_field_value<'a>(line: &'a str, field: &str) -> &'a str {
    let needle = format!(" {field}=");
    let start = line
        .find(&needle)
        .unwrap_or_else(|| panic!("missing field {field}: {line}"))
        + needle.len();
    line[start..].split(' ').next().unwrap_or(&line[start..])
}

#[tokio::test]
async fn mongodb_integration_reindex_writer_configuration_log_line() {
    let Some(backend) = create_backend_with("reindex_writer_configuration_log", |c| {
        c.reindex_overlap = true;
        c.reindex_prefetch = true;
        c.reindex_prepare_threads = 2;
    })
    .await
    else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };

    let (buf, _guard) = capture_writer_config_log();
    let tenant = create_tenant("reindex-writer-config-log-tenant");
    let target: &dyn ReindexTarget = &*backend;

    // A type's first page over `REINDEX_SUBBATCH_FIRST` (32) resources must
    // log the line; a second such page on the same backend must not log it
    // again (#1403).
    let page1 = build_test_patients(&tenant, "writerconfig1", 40);
    let mut stats1 = ReindexPageStats::default();
    let outcomes1 = target
        .write_search_entries_page_timed(&tenant, &page1, &mut stats1)
        .await;
    assert!(outcomes1.iter().all(|o| o.is_ok()), "{outcomes1:?}");

    let page2 = build_test_patients(&tenant, "writerconfig2", 40);
    let mut stats2 = ReindexPageStats::default();
    let outcomes2 = target
        .write_search_entries_page_timed(&tenant, &page2, &mut stats2)
        .await;
    assert!(outcomes2.iter().all(|o| o.is_ok()), "{outcomes2:?}");

    let lines: Vec<String> = {
        let buf = buf.lock().unwrap();
        String::from_utf8_lossy(&buf)
            .lines()
            .filter(|l| l.contains("mongodb reindex writer configuration"))
            .map(str::to_string)
            .collect()
    };
    assert_eq!(
        lines.len(),
        1,
        "the writer-configuration line must log once per backend instance: {lines:?}"
    );
    let line = &lines[0];
    assert!(
        line.contains("helios_persistence::backends::mongodb::reindex_pipeline"),
        "target must be reindex_pipeline: {line}"
    );

    // The 7 fields, in the analyze_arm.py contract's order.
    let fields = [
        "overlap",
        "prefetch",
        "prepare_threads_configured",
        "prepare_threads",
        "pool",
        "multi_thread_runtime",
        "path",
    ];
    let positions: Vec<usize> = fields
        .iter()
        .map(|f| {
            line.find(&format!(" {f}="))
                .unwrap_or_else(|| panic!("missing field {f}: {line}"))
        })
        .collect();
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "fields out of order: {line}"
    );

    // `pool` and `path` use Display, so they print unquoted.
    assert!(
        !log_field_value(line, "pool").starts_with('"'),
        "pool must print unquoted: {line}"
    );
    assert!(
        !log_field_value(line, "path").starts_with('"'),
        "path must print unquoted: {line}"
    );
    // This test's `#[tokio::test]` runtime is current-thread, so the driver
    // always takes the serial writer path regardless of `reindex_overlap`.
    assert_eq!(
        log_field_value(line, "multi_thread_runtime"),
        "false",
        "{line}"
    );
    assert_eq!(log_field_value(line, "path"), "serial", "{line}");
    assert_eq!(log_field_value(line, "overlap"), "true", "{line}");
    assert_eq!(log_field_value(line, "prefetch"), "true", "{line}");
    assert_eq!(
        log_field_value(line, "prepare_threads_configured"),
        "2",
        "{line}"
    );
}

/// Reads every row of `collection` matching `filter`, drops `_id` and
/// `tenant_id`, and sorts a deterministic per-row string — so two backends'
/// collections, or two tenants within the same collection, can be compared
/// regardless of insert order or `_id` values (#1403).
async fn rows_without_id_and_tenant(
    backend: &MongoBackend,
    collection: &str,
    filter: Document,
) -> Vec<String> {
    use futures::stream::TryStreamExt;
    let db = backend.get_database().await.expect("get_database");
    let mut rows: Vec<String> = db
        .collection::<Document>(collection)
        .find(filter)
        .await
        .expect("find")
        .try_collect::<Vec<Document>>()
        .await
        .expect("collect")
        .into_iter()
        .map(|mut d| {
            d.remove("_id");
            d.remove("tenant_id");
            let mut keys: Vec<&String> = d.keys().collect();
            keys.sort();
            keys.into_iter()
                .map(|k| format!("{k}={:?}", d.get(k)))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect();
    rows.sort();
    rows
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reindex_page_overlapped_matches_serial() {
    let Some(serial) = create_backend_with("reindex_overlap_matches_serial", |c| {
        c.reindex_overlap = false;
        c.reindex_prepare_threads = 1;
    })
    .await
    else {
        eprintln!(
            "Skipping mongodb_integration_reindex_page_overlapped_matches_serial (requires Docker)"
        );
        return;
    };
    let overlapped = create_backend_with("reindex_overlap_matches_serial", |c| {
        c.reindex_overlap = true;
        c.reindex_prepare_threads = 0;
    })
    .await
    .expect(
        "Docker was available for the serial backend, so it must be for the overlapped one too",
    );

    let ts = create_tenant("reindex-overlap-serial-tenant-s");
    let to = create_tenant("reindex-overlap-serial-tenant-o");

    let build_page = |tenant: &TenantContext| -> Vec<StoredResource> {
        (0..300)
            .map(|n| {
                let mut resource = json!({
                    "resourceType": "Observation",
                    "id": format!("obs-{n}"),
                    "status": "final",
                    "code": {"coding": [{"system": "http://loinc.org", "code": "1234-5"}]},
                });
                if n % 10 == 0 {
                    resource["contained"] = json!([{
                        "resourceType": "Patient",
                        "id": "p1",
                        "name": [{"family": format!("Contained{n}")}]
                    }]);
                }
                StoredResource::from_storage(
                    "Observation",
                    format!("obs-{n}"),
                    "1",
                    tenant.tenant_id().clone(),
                    resource,
                    chrono::Utc::now(),
                    chrono::Utc::now(),
                    None,
                    FhirVersion::default(),
                )
            })
            .collect()
    };

    let page_s = build_page(&ts);
    let page_o = build_page(&to);
    let target_s: &dyn ReindexTarget = &*serial;
    let target_o: &dyn ReindexTarget = &*overlapped;
    let mut stats_s = ReindexPageStats::default();
    let mut stats_o = ReindexPageStats::default();
    let outcomes_s = target_s
        .write_search_entries_page_timed(&ts, &page_s, &mut stats_s)
        .await;
    let outcomes_o = target_o
        .write_search_entries_page_timed(&to, &page_o, &mut stats_o)
        .await;

    assert_eq!(outcomes_s.len(), outcomes_o.len());
    assert!(
        outcomes_s.iter().all(|o| o.is_ok()),
        "the reference write must succeed, or matching failures below proves nothing: {outcomes_s:?}"
    );
    for (a, b) in outcomes_s.iter().zip(&outcomes_o) {
        assert_eq!(a.as_ref().ok(), b.as_ref().ok());
    }
    assert_eq!(
        stats_s.sub_batches, 1,
        "300 resources is one sub-batch serially"
    );
    assert!(
        stats_o.sub_batches >= 2,
        "300 resources must split on the overlapped path"
    );
    assert_eq!(stats_s.inserted_entries, stats_o.inserted_entries);

    assert_eq!(
        rows_without_id_and_tenant(&serial, "search_index", doc! {}).await,
        rows_without_id_and_tenant(&overlapped, "search_index", doc! {}).await
    );
    assert_eq!(
        rows_without_id_and_tenant(&serial, "search_index_contained", doc! {}).await,
        rows_without_id_and_tenant(&overlapped, "search_index_contained", doc! {}).await
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reindex_page_overlapped_command_shape() {
    let Some(backend) = create_backend_with("reindex_overlap_command_shape", |c| {
        c.reindex_overlap = true;
        c.reindex_prepare_threads = 1; // keep the assertions independent of pool scheduling
    })
    .await
    else {
        eprintln!(
            "Skipping mongodb_integration_reindex_page_overlapped_command_shape (requires Docker)"
        );
        return;
    };

    let tenant = create_tenant("reindex-overlap-shape-tenant");
    let page = build_test_patients(&tenant, "shape", 40);

    let db = backend.get_database().await.unwrap();
    let profiling_enabled = db.run_command(doc! { "profile": 2_i32 }).await.is_ok();
    if !profiling_enabled {
        eprintln!(
            "mongodb_integration_reindex_page_overlapped_command_shape: server refused \
             {{profile: 2}}; skipping"
        );
        return;
    }

    let target: &dyn ReindexTarget = &*backend;
    let mut stats = ReindexPageStats::default();
    let outcomes = target
        .write_search_entries_page_timed(&tenant, &page, &mut stats)
        .await;
    assert!(outcomes.iter().all(|o| o.is_ok()), "{outcomes:?}");
    assert!(
        stats.sub_batches >= 2,
        "a type's first page caps its first sub-batch at REINDEX_SUBBATCH_FIRST=32, so 40 must split"
    );

    db.run_command(doc! { "profile": 0_i32 })
        .await
        .expect("disable profiling");
    let ns = format!("{}.search_index", db.name());
    let entries: Vec<Document> = db
        .collection::<Document>("system.profile")
        .find(doc! { "ns": ns.as_str(), "op": { "$in": ["remove", "insert"] } })
        .sort(doc! { "ts": 1 })
        .await
        .expect("read system.profile")
        .try_collect()
        .await
        .expect("collect system.profile");

    let removes: Vec<&Document> = entries
        .iter()
        .filter(|e| e.get_str("op").ok() == Some("remove"))
        .collect();
    let inserts: Vec<&Document> = entries
        .iter()
        .filter(|e| e.get_str("op").ok() == Some("insert"))
        .collect();
    assert_eq!(
        removes.len(),
        1,
        "exactly one delete_many for the page: {entries:?}"
    );
    assert_eq!(inserts.len() as u64, stats.insert_commands, "{entries:?}");
    assert!(inserts.len() >= 2);
    let remove_ts = removes[0].get_datetime("ts").expect("remove ts");
    for insert in &inserts {
        let insert_ts = insert.get_datetime("ts").expect("insert ts");
        assert!(
            remove_ts <= insert_ts,
            "the delete must happen at or before every insert"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reindex_page_overlapped_issues_no_insert_after_a_failed_one() {
    let app_name = "reindex-overlap-fail-first-insert";
    let Some(backend) = create_backend_with("reindex_overlap_fails_first_insert", |c| {
        c.reindex_overlap = true;
        c.reindex_prepare_threads = 1;
        c.app_name = app_name.to_string();
        c.connection_string = append_query_param(&c.connection_string, "retryWrites=false");
    })
    .await
    else {
        eprintln!(
            "Skipping mongodb_integration_reindex_page_overlapped_issues_no_insert_after_a_failed_one (requires Docker)"
        );
        return;
    };

    let tenant = create_tenant("reindex-overlap-fail-first-tenant");
    let page = build_test_patients(&tenant, "failfirst", 80);

    // Seed the rows with the failpoint off, so the zero-row assertion below
    // actually shows the page's delete removed something (#1403).
    let target: &dyn ReindexTarget = &*backend;
    for resource in &page {
        let seeded = target.write_search_entries(&tenant, resource).await;
        assert!(seeded.is_ok(), "{seeded:?}");
    }
    assert!(
        search_index_entry_count(&backend, &tenant, "Patient", page[0].id()).await > 0,
        "the seed write must have created rows, or the later zero-row assertion proves nothing"
    );

    let Some(failpoint) = FailPoint::enable(
        app_name,
        doc! { "failCommands": ["insert"], "errorCode": 2 },
        doc! { "times": 1 },
    )
    .await
    else {
        eprintln!("Skipping: enableTestCommands unavailable");
        return;
    };

    let mut stats = ReindexPageStats::default();
    let outcomes = target
        .write_search_entries_page_timed(&tenant, &page, &mut stats)
        .await;
    failpoint.off().await;

    assert!(stats.sub_batches >= 2);
    assert!(
        outcomes
            .iter()
            .all(|o| matches!(o, Err(e) if e.to_string().contains("Failed to insert search index entries"))),
        "{outcomes:?}"
    );
    let indexed = indexed_resource_ids(&backend, &tenant, "Patient").await;
    assert!(
        indexed.is_empty(),
        "every resource must have zero rows: the delete removed them and sub-batch 1's insert \
         failed before any row could be re-inserted: {indexed:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reindex_page_overlapped_keeps_rows_of_completed_sub_batches() {
    let app_name = "reindex-overlap-fail-later-insert";
    let Some(backend) = create_backend_with("reindex_overlap_keeps_completed_rows", |c| {
        c.reindex_overlap = true;
        c.reindex_prepare_threads = 1;
        c.app_name = app_name.to_string();
        c.connection_string = append_query_param(&c.connection_string, "retryWrites=false");
    })
    .await
    else {
        eprintln!(
            "Skipping mongodb_integration_reindex_page_overlapped_keeps_rows_of_completed_sub_batches (requires Docker)"
        );
        return;
    };

    let tenant = create_tenant("reindex-overlap-fail-later-tenant");
    let page = build_test_patients(&tenant, "faillater", 80);

    // Seed the rows with the failpoint off, so the "non-empty prefix" claim
    // below shows real rows being kept, not an already-empty collection.
    let target: &dyn ReindexTarget = &*backend;
    for resource in &page {
        let seeded = target.write_search_entries(&tenant, resource).await;
        assert!(seeded.is_ok(), "{seeded:?}");
    }
    assert!(
        search_index_entry_count(&backend, &tenant, "Patient", page[0].id()).await > 0,
        "the seed write must have created rows, or the later prefix assertion proves nothing"
    );

    let Some(failpoint) = FailPoint::enable(
        app_name,
        doc! { "failCommands": ["insert"], "errorCode": 2 },
        doc! { "skip": 1 },
    )
    .await
    else {
        eprintln!("Skipping: enableTestCommands unavailable");
        return;
    };

    let mut stats = ReindexPageStats::default();
    let outcomes = target
        .write_search_entries_page_timed(&tenant, &page, &mut stats)
        .await;
    failpoint.off().await;
    assert!(stats.sub_batches >= 2, "{stats:?}");

    assert!(outcomes.iter().all(|o| o.is_err()), "{outcomes:?}");

    let indexed = indexed_resource_ids(&backend, &tenant, "Patient").await;
    let has_rows: Vec<bool> = page.iter().map(|r| indexed.contains(r.id())).collect();
    let with_rows = has_rows.iter().filter(|b| **b).count();
    assert!(
        with_rows > 0 && with_rows < page.len(),
        "the rows must form a non-empty, proper prefix: {has_rows:?}"
    );
    assert!(has_rows[..with_rows].iter().all(|b| *b), "{has_rows:?}");
    assert!(has_rows[with_rows..].iter().all(|b| !*b), "{has_rows:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reindex_page_overlapped_fans_out_a_delete_failure() {
    let app_name = "reindex-overlap-fail-delete";
    let Some(backend) = create_backend_with("reindex_overlap_fans_out_delete_failure", |c| {
        c.reindex_overlap = true;
        c.reindex_prepare_threads = 1;
        c.app_name = app_name.to_string();
        c.connection_string = append_query_param(&c.connection_string, "retryWrites=false");
    })
    .await
    else {
        eprintln!(
            "Skipping mongodb_integration_reindex_page_overlapped_fans_out_a_delete_failure (requires Docker)"
        );
        return;
    };

    let tenant = create_tenant("reindex-overlap-fail-delete-tenant");
    let page = build_test_patients(&tenant, "faildelete", 40);

    // Seed with a per-resource write, like the two failpoint tests above,
    // rather than one page write: seeding one resource at a time records no
    // per-type size hint (a hint only changes how a later page is split into
    // sub-batches), so it cannot affect which writer runs. The page below is
    // routed to `write_page_overlapped` by configuration alone (overlap on,
    // a multi-thread runtime, and more than `REINDEX_SUBBATCH_FIRST`
    // resources); a delete failure is always joined right after the first
    // sub-batch is extracted, so `sub_batches` cannot prove which writer ran
    // in this test (#1403).
    let target: &dyn ReindexTarget = &*backend;
    for resource in &page {
        let seeded = target.write_search_entries(&tenant, resource).await;
        assert!(seeded.is_ok(), "{seeded:?}");
    }
    assert!(
        search_index_entry_count(&backend, &tenant, "Patient", page[0].id()).await > 0,
        "the seed write must have created rows, or the later unchanged-rows assertion proves nothing"
    );

    let Some(failpoint) = FailPoint::enable(
        app_name,
        doc! { "failCommands": ["delete"], "errorCode": 2 },
        doc! { "times": 1 },
    )
    .await
    else {
        eprintln!("Skipping: enableTestCommands unavailable");
        return;
    };

    let mut stats = ReindexPageStats::default();
    let outcomes = target
        .write_search_entries_page_timed(&tenant, &page, &mut stats)
        .await;
    failpoint.off().await;

    // Unlike the two insert-failure tests below, `sub_batches` cannot prove
    // the overlapped writer ran: both writers join the page's delete right
    // after extracting only their first sub-batch, so a delete failure is
    // always caught with `sub_batches == 1`, whichever writer is running.
    // This test instead relies on its config (`reindex_overlap = true`, a
    // multi-thread runtime, and a 40-resource page) to route it through
    // `write_page_overlapped` (#1403).
    assert!(
        outcomes.iter().all(
            |o| matches!(o, Err(e) if e.to_string().contains("Failed to delete search entries"))
        ),
        "{outcomes:?}"
    );
    assert_eq!(
        stats.insert_commands, 0,
        "no insert may be spawned once the delete has failed"
    );
    let indexed = indexed_resource_ids(&backend, &tenant, "Patient").await;
    let expected: std::collections::BTreeSet<String> =
        page.iter().map(|r| r.id().to_string()).collect();
    assert_eq!(
        indexed, expected,
        "the failed delete must leave every resource's rows exactly as seeded"
    );
}

/// An in-memory, scripted `ReindexSource` that always allows prefetch
/// (#1403) — used to prove that a page written from a resource seen on one
/// fetch, followed by an update to the same resource seen on the next fetch,
/// still ends with the later version's rows and no duplicates. The two
/// pages' own writes must never overlap each other; what does overlap is the
/// next page's fetch with the current page's write.
struct ScriptedResourceSource {
    resource_type: &'static str,
    pages: Vec<Vec<StoredResource>>,
}

#[async_trait]
impl ReindexSource for ScriptedResourceSource {
    async fn list_resource_types(&self, _: &TenantContext) -> StorageResult<Vec<String>> {
        Ok(vec![self.resource_type.to_string()])
    }

    async fn count_resources(&self, _: &TenantContext, _: &str) -> StorageResult<u64> {
        Ok(self.pages.iter().map(|p| p.len() as u64).sum())
    }

    async fn fetch_resources_page(
        &self,
        _: &TenantContext,
        _: &str,
        cursor: Option<&str>,
        _: u32,
    ) -> StorageResult<ResourcePage> {
        let page = cursor.and_then(|c| c.parse::<usize>().ok()).unwrap_or(0);
        let resources = self.pages.get(page).cloned().unwrap_or_default();
        let next_cursor = (page + 1 < self.pages.len()).then(|| (page + 1).to_string());
        Ok(ResourcePage {
            resources,
            next_cursor,
            skipped: Vec::new(),
        })
    }

    fn may_prefetch_page(&self, _: &str) -> bool {
        true
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reindex_resource_in_consecutive_pages_ends_with_the_later_version() {
    let Some(backend) = create_backend_with("reindex_prefetch_consecutive_versions", |c| {
        c.reindex_overlap = true;
        c.reindex_prepare_threads = 1;
    })
    .await
    else {
        eprintln!(
            "Skipping mongodb_integration_reindex_resource_in_consecutive_pages_ends_with_the_later_version (requires Docker)"
        );
        return;
    };

    let tenant = create_tenant("reindex-prefetch-consecutive-tenant");
    let r_v2 = StoredResource::from_storage(
        "Patient",
        "r",
        "2",
        tenant.tenant_id().clone(),
        json!({"resourceType": "Patient", "id": "r", "name": [{"family": "V2"}]}),
        chrono::Utc::now(),
        chrono::Utc::now(),
        None,
        FhirVersion::default(),
    );
    let r_v1 = StoredResource::from_storage(
        "Patient",
        "r",
        "1",
        tenant.tenant_id().clone(),
        json!({"resourceType": "Patient", "id": "r", "name": [{"family": "V1"}]}),
        chrono::Utc::now(),
        chrono::Utc::now(),
        None,
        FhirVersion::default(),
    );
    let mut page1 = vec![r_v1];
    page1.extend(build_test_patients(&tenant, "others", 40));
    let source = Arc::new(ScriptedResourceSource {
        resource_type: "Patient",
        pages: vec![page1, vec![r_v2.clone()]],
    });

    let op = ReindexOperation::with_parts(
        source,
        vec![backend.clone() as Arc<dyn ReindexTarget>],
        backend.tenant_registries().clone(),
    );
    let job = op
        .start(
            tenant.clone(),
            ReindexRequest::for_types(vec!["Patient".to_string()]).with_batch_size(50),
            None,
        )
        .await
        .expect("start");
    let progress = super::reindex_id_walk::wait_for_terminal(&op, &job).await;
    assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");

    // A direct rewrite from v2 alone, into a fresh tenant, is the reference:
    // R's rows in `tenant` must equal it exactly, including no duplicates.
    let fresh_tenant = create_tenant("reindex-prefetch-consecutive-fresh");
    let target: &dyn ReindexTarget = backend.as_ref();
    let mut stats = ReindexPageStats::default();
    let outcome = target
        .write_search_entries_page_timed(&fresh_tenant, std::slice::from_ref(&r_v2), &mut stats)
        .await;
    assert!(outcome[0].is_ok());

    let reference_rows = rows_without_id_and_tenant(
        &backend,
        "search_index",
        doc! {
            "tenant_id": fresh_tenant.tenant_id().as_str(),
            "resource_type": "Patient",
            "resource_id": "r",
        },
    )
    .await;
    assert!(
        !reference_rows.is_empty(),
        "the reference write must have produced rows, or the equality below proves nothing"
    );
    assert_eq!(
        rows_without_id_and_tenant(
            &backend,
            "search_index",
            doc! {
                "tenant_id": tenant.tenant_id().as_str(),
                "resource_type": "Patient",
                "resource_id": "r",
            },
        )
        .await,
        reference_rows,
        "R must end with v2's rows and no duplicates"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reindex_run_parity_across_knobs() {
    let configs: [(bool, bool, usize); 3] = [(false, false, 1), (true, false, 1), (true, true, 0)];
    let mut backends: Vec<Arc<MongoBackend>> = Vec::new();
    for (overlap, prefetch, threads) in configs {
        let Some(backend) = create_backend_with("reindex_run_parity_across_knobs", move |c| {
            c.reindex_overlap = overlap;
            c.reindex_prefetch = prefetch;
            c.reindex_prepare_threads = threads;
            c.reindex_catch_up_margin_ms = 1_000;
        })
        .await
        else {
            eprintln!(
                "Skipping mongodb_integration_reindex_run_parity_across_knobs (requires Docker)"
            );
            return;
        };
        backends.push(backend);
    }

    let tenants: Vec<TenantContext> = ["k0", "k1", "k2"]
        .iter()
        .map(|n| create_tenant(n))
        .collect();
    for (backend, tenant) in backends.iter().zip(&tenants) {
        for n in 0..300 {
            backend
                .create_or_update(
                    tenant,
                    "Observation",
                    &format!("obs-{n}"),
                    json!({"resourceType": "Observation", "id": format!("obs-{n}"), "status": "final"}),
                    FhirVersion::default(),
                )
                .await
                .expect("seed");
        }
    }
    settle_into_id_phase().await;

    let mut entries_created = Vec::new();
    for (backend, tenant) in backends.iter().zip(&tenants) {
        let op = ReindexOperation::new(backend.clone(), backend.tenant_registries().clone());
        let job = op
            .start(
                tenant.clone(),
                ReindexRequest::for_types(vec!["Observation".to_string()]).with_batch_size(50),
                None,
            )
            .await
            .expect("start");
        let progress = super::reindex_id_walk::wait_for_terminal(&op, &job).await;
        assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
        assert_eq!(progress.error_message, None);
        assert!(progress.errors.is_empty(), "{:?}", progress.errors);
        entries_created.push(progress.entries_created);
    }
    assert!(
        entries_created[0] > 0,
        "the reindex must have created some entries, or the parity checks below prove nothing"
    );
    assert_eq!(
        entries_created[0], entries_created[1],
        "entries_created must match across knobs"
    );
    assert_eq!(
        entries_created[1], entries_created[2],
        "entries_created must match across knobs"
    );

    let rows: Vec<Vec<String>> = {
        let mut v = Vec::new();
        for backend in &backends {
            v.push(rows_without_id_and_tenant(backend, "search_index", doc! {}).await);
        }
        v
    };
    assert!(
        !rows[0].is_empty(),
        "the reindex must have written some rows, or the parity checks below prove nothing"
    );
    assert_eq!(rows[0], rows[1]);
    assert_eq!(rows[1], rows[2]);
}

/// Wraps a real `MongoBackend` writer and, around each page write, checks
/// whether `mongodb reindex id phase finished` is present in the capture
/// buffer before the write starts and again after it returns — flagging only
/// a transition that was absent before and present after, i.e. one this
/// write's own execution window could have raced. Checking only "present
/// after" would also flag a page that runs after an earlier page already
/// logged the transition, which is not a race at all (#1403).
struct PhaseLogProbeTarget {
    backend: Arc<MongoBackend>,
    saw_transition_early: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl ReindexTarget for PhaseLogProbeTarget {
    async fn delete_search_entries(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<u64> {
        self.backend
            .delete_search_entries(tenant, resource_type, id)
            .await
    }

    async fn write_search_entries(
        &self,
        tenant: &TenantContext,
        resource: &StoredResource,
    ) -> StorageResult<usize> {
        self.backend.write_search_entries(tenant, resource).await
    }

    // Delegates to `write_search_entries_page_timed` with a throwaway
    // `ReindexPageStats`, per `ReindexTarget::write_search_entries_page`'s
    // trait contract that an override MUST route the untimed page method
    // through it, so the two paths cannot diverge.
    async fn write_search_entries_page(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
    ) -> Vec<StorageResult<usize>> {
        let mut stats = ReindexPageStats::default();
        self.write_search_entries_page_timed(tenant, resources, &mut stats)
            .await
    }

    async fn write_search_entries_page_timed(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        stats: &mut ReindexPageStats,
    ) -> Vec<StorageResult<usize>> {
        // `capture_walk_logs`/`walk_log_lines` install ONE global subscriber
        // and ONE global buffer for the whole `mongodb_tests` binary, so
        // filtering on the message alone would also match every other walk's
        // lines from other tests in this binary. A tenant needle isolates
        // this run's lines (walk lines log `tenant = %tenant_id`, which the
        // test log capture prints unquoted as `tenant=<id>`) (#1403).
        let tenant_needle = format!("tenant={}", tenant.tenant_id().as_str());
        let transition_lines = || {
            super::reindex_id_walk::walk_log_lines(&[
                "mongodb reindex id phase finished",
                &tenant_needle,
            ])
        };
        let seen_before = !transition_lines().is_empty();
        let result = self
            .backend
            .write_search_entries_page_timed(tenant, resources, stats)
            .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let seen_after = !transition_lines().is_empty();
        if seen_after && !seen_before {
            self.saw_transition_early
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        result
    }

    async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
        self.backend.clear_search_index(tenant).await
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_integration_reindex_prefetch_logs_id_phase_finished_after_the_last_id_page_is_written()
 {
    // Sets `reindex_prefetch` explicitly rather than relying on
    // `create_id_phase_backend`'s default: prefetch is on by default today,
    // but this test specifically exercises the prefetch path, so it must not
    // start passing on the serial path if that default ever changes (#1403).
    let Some(backend) = create_backend_with("reindex_prefetch_logs_id_phase_finished", |c| {
        c.reindex_catch_up_margin_ms = 1_000;
        c.reindex_prefetch = true;
    })
    .await
    else {
        eprintln!(
            "Skipping mongodb_integration_reindex_prefetch_logs_id_phase_finished_after_the_last_id_page_is_written (requires Docker)"
        );
        return;
    };
    let tenant = create_tenant("reindex-prefetch-log-order-tenant");
    for n in 0..120 {
        backend
            .create_or_update(
                &tenant,
                "Observation",
                &format!("obs-{n}"),
                json!({"resourceType": "Observation", "id": format!("obs-{n}"), "status": "final"}),
                FhirVersion::default(),
            )
            .await
            .expect("seed");
    }
    settle_into_id_phase().await;

    super::reindex_id_walk::capture_walk_logs();
    let saw_transition_early = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let probe = Arc::new(PhaseLogProbeTarget {
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
            ReindexRequest::for_types(vec!["Observation".to_string()]).with_batch_size(50),
            None,
        )
        .await
        .expect("start");
    let progress = super::reindex_id_walk::wait_for_terminal(&op, &job).await;
    assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
    assert!(
        !saw_transition_early.load(std::sync::atomic::Ordering::SeqCst),
        "id phase finished must never be logged concurrently with an id-page write"
    );

    let tenant_needle = format!("tenant={}", tenant.tenant_id().as_str());
    assert_eq!(
        super::reindex_id_walk::walk_log_lines(&[
            "mongodb reindex id phase finished",
            &tenant_needle
        ])
        .len(),
        1
    );
    assert_eq!(
        super::reindex_id_walk::walk_log_lines(&[
            "mongodb reindex catch-up round started",
            &tenant_needle
        ])
        .len(),
        1
    );
}
