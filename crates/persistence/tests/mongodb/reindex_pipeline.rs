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
/// two Provenance-shaped tests below, which otherwise duplicated this setup
/// verbatim (ledger Ruling R3).
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
    let seen: std::collections::BTreeSet<String> = pages
        .iter()
        .flat_map(|p| p.resources.iter().map(|r| r.id().to_string()))
        .collect();
    assert_eq!(seen.len(), 24, "every resource must come back exactly once");

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
    // `ReindexPageStats`, per that method's trait contract (reindex.rs:379-386)
    // that an override MUST route the untimed page method through it, so the
    // two paths cannot diverge (ledger Ruling R5) — mirrors MongoBackend's own
    // `ReindexTarget::write_search_entries_page` impl.
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
    for (resources, bytes) in source.pages.lock().unwrap().iter() {
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

/// S3 §4.5's round-page gap: every other test here builds its backend
/// through [`create_id_phase_backend`], so every capped page it checks is an
/// id-phase page (`v2|i|`). This test uses [`create_backend_with`] with the
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

    let mut counts = std::collections::HashMap::new();
    for id in &seen {
        *counts.entry(id.clone()).or_insert(0u32) += 1;
    }
    for id in ["p01", "p02", "p03", "p04"] {
        assert_eq!(counts.get(id), Some(&1), "{id} must appear exactly once");
    }
}
