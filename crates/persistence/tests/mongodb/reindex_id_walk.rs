//! #1403: the id-order `$reindex` walk and its catch-up rounds.
//!
//! Uses `super::*` for the parent test crate's imports and private harness
//! helpers (`create_backend`, `create_tenant`, `build_test_database_name`,
//! etc.) — this file is a `#[path]`-included child module of
//! `mongodb_tests.rs`, not a standalone test binary.

use super::*;
use std::collections::{BTreeMap, BTreeSet};
use mongodb::bson::{DateTime as BsonDateTime, Document};
use helios_persistence::error::StorageResult;
use helios_persistence::search::{ReindexOperation, ReindexRequest, ReindexSource, ReindexTarget};

// `ReindexSource` and `ReindexTarget` must be in scope for dot-call syntax
// (`x.fetch_resources_page(..)`, `x.write_search_entries_page(..)`, ...);
// every other type here (`ReindexStatus`, `ResourcePage`, ...) is written
// fully qualified instead, so importing it bare would be `unused_imports`.

// ===========================================================================
// Harness
// ===========================================================================

/// Live and tombstoned ids per type, as seeded by [`seed_walk_fixture`].
struct WalkFixture {
    live: BTreeMap<String, BTreeSet<String>>,
    tombstones: BTreeMap<String, BTreeSet<String>>,
}

/// Seeds one tenant with the fixture #1403's tests share: a fixed set of
/// Patients exercising FHIR id ordering (`-` < `.` < digits < upper < lower),
/// `observations` Observations, and tombstones on two Patients and every
/// tenth Observation. Ids are NOT yet backdated — call [`backdate_fixture`]
/// separately so a test can inspect CRUD-time snapshots first.
async fn seed_walk_fixture(
    backend: &MongoBackend,
    tenant: &TenantContext,
    observations: usize,
    extra_patient: &str,
) -> WalkFixture {
    let mut live: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut tombstones: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    let mut patient_ids: Vec<String> = [
        "-lead", "0", "9.9", "A-1", "A.1", "Z", "a-1", "a.1", "aa", "z", "zz-9",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    patient_ids.push(extra_patient.to_string());

    for id in &patient_ids {
        backend
            .create(
                tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": id,
                    "name": [{ "family": id }],
                    "identifier": [{ "system": "urn:walk", "value": format!("p-{id}") }],
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    for i in 0..observations {
        let id = format!("obs-{i:03}");
        let mut body = json!({
            "resourceType": "Observation",
            "id": id,
            "status": "final",
            "code": { "coding": [{ "system": "http://loinc.org", "code": "8867-4" }] },
            "subject": { "reference": "Patient/A-1" },
            "effectiveDateTime": "2020-01-01",
            "valueQuantity": { "value": i, "unit": "/min" },
            "identifier": [{ "system": "urn:walk", "value": format!("o-{i}") }],
        });
        if i % 10 == 3 {
            body["contained"] = json!([{
                "resourceType": "Patient",
                "id": "p1",
                "name": [{ "family": format!("Contained{i}") }],
            }]);
            body["performer"] = json!([{ "reference": "#p1" }]);
        }
        backend
            .create(tenant, "Observation", body, FhirVersion::default())
            .await
            .unwrap();
    }

    for id in ["Z", "a.1"] {
        backend.delete(tenant, "Patient", id).await.unwrap();
        tombstones.entry("Patient".to_string()).or_default().insert(id.to_string());
    }
    for i in 0..observations {
        if i % 100 == 5 {
            let id = format!("obs-{i:03}");
            backend.delete(tenant, "Observation", &id).await.unwrap();
            tombstones
                .entry("Observation".to_string())
                .or_default()
                .insert(id);
        }
    }

    let patient_tombstones = tombstones.get("Patient").cloned().unwrap_or_default();
    live.insert(
        "Patient".to_string(),
        patient_ids
            .iter()
            .filter(|id| !patient_tombstones.contains(*id))
            .cloned()
            .collect(),
    );
    let obs_tombstones = tombstones.get("Observation").cloned().unwrap_or_default();
    live.insert(
        "Observation".to_string(),
        (0..observations)
            .map(|i| format!("obs-{i:03}"))
            .filter(|id| !obs_tombstones.contains(id))
            .collect(),
    );

    WalkFixture { live, tombstones }
}

/// Raw `update_many` on `resources` that backdates every id in `fixture`
/// (live and tombstoned) so the fast-load shape holds: three groups of equal
/// `last_updated`, interleaved with id order. Observation `i` goes to second
/// `i % 3`; every Patient goes to second 3.
async fn backdate_fixture(backend: &MongoBackend, tenant: &TenantContext, fixture: &WalkFixture) {
    let db = backend.get_database().await.unwrap();
    let resources = db.collection::<Document>("resources");
    let tenant_id = tenant.tenant_id().as_str();

    let mut patient_ids: Vec<String> = fixture
        .live
        .get("Patient")
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect();
    patient_ids.extend(fixture.tombstones.get("Patient").cloned().unwrap_or_default());
    if !patient_ids.is_empty() {
        resources
            .update_many(
                doc! {
                    "tenant_id": tenant_id,
                    "resource_type": "Patient",
                    "id": { "$in": &patient_ids },
                },
                doc! {
                    "$set": {
                        "last_updated": BsonDateTime::from_millis(
                            ts("2020-01-01T00:00:03.000Z").timestamp_millis(),
                        ),
                    },
                },
            )
            .await
            .unwrap();
    }

    let mut obs_by_group: [Vec<String>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut all_obs: BTreeSet<String> = fixture.live.get("Observation").cloned().unwrap_or_default();
    all_obs.extend(fixture.tombstones.get("Observation").cloned().unwrap_or_default());
    for id in &all_obs {
        let i: usize = id.trim_start_matches("obs-").parse().unwrap();
        obs_by_group[i % 3].push(id.clone());
    }
    for (group, ids) in obs_by_group.iter().enumerate() {
        if ids.is_empty() {
            continue;
        }
        resources
            .update_many(
                doc! {
                    "tenant_id": tenant_id,
                    "resource_type": "Observation",
                    "id": { "$in": ids },
                },
                doc! {
                    "$set": {
                        "last_updated": BsonDateTime::from_millis(
                            ts(&format!("2020-01-01T00:00:0{group}.000Z")).timestamp_millis(),
                        ),
                    },
                },
            )
            .await
            .unwrap();
    }
}

fn ts(s: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&chrono::Utc)
}

// ===========================================================================
// T2: order and isolation
// ===========================================================================

#[tokio::test]
async fn mongodb_reindex_id_walk_returns_each_live_resource_once_in_byte_order() {
    let Some(backend) = create_backend("reindex_id_walk_order").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant_a = create_tenant("walk-ord-a");
    let tenant_b = create_tenant("walk-ord-b");
    let fixture_a = seed_walk_fixture(&backend, &tenant_a, 20, "only-in-a").await;
    let fixture_b = seed_walk_fixture(&backend, &tenant_b, 20, "only-in-b").await;
    backdate_fixture(&backend, &tenant_a, &fixture_a).await;
    backdate_fixture(&backend, &tenant_b, &fixture_b).await;

    async fn walk_all(backend: &MongoBackend, tenant: &TenantContext, resource_type: &str, limit: u32) -> Vec<String> {
        let mut ids = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..40 {
            let page = backend
                .fetch_resources_page(tenant, resource_type, cursor.as_deref(), limit)
                .await
                .unwrap();
            let empty = page.resources.is_empty();
            ids.extend(page.resources.iter().map(|r| r.id().to_string()));
            match page.next_cursor {
                Some(next) => {
                    assert!(!empty, "every page but the trailing one must return at least one resource");
                    cursor = Some(next);
                }
                None => {
                    assert!(empty, "the trailing page must be empty");
                    return ids;
                }
            }
        }
        panic!("walk did not terminate within 40 pages");
    }

    let patient_ids = walk_all(&backend, &tenant_a, "Patient", 3).await;
    let expected_patients: Vec<String> = fixture_a.live.get("Patient").unwrap().iter().cloned().collect();
    assert_eq!(patient_ids, expected_patients);
    assert!(patient_ids.contains(&"only-in-a".to_string()));
    assert!(!patient_ids.contains(&"only-in-b".to_string()));
    assert!(!patient_ids.contains(&"Z".to_string()));
    assert!(!patient_ids.contains(&"a.1".to_string()));

    let obs_ids = walk_all(&backend, &tenant_a, "Observation", 7).await;
    let expected_obs: Vec<String> = fixture_a.live.get("Observation").unwrap().iter().cloned().collect();
    assert_eq!(obs_ids, expected_obs);
    assert_eq!(obs_ids.len(), 19);
    assert!(!obs_ids.contains(&"obs-005".to_string()));
    let _ = fixture_b; // seeded only to prove isolation via the assertions above
}

// ===========================================================================
// T12: a foreign or corrupt cursor is rejected
// ===========================================================================

#[tokio::test]
async fn mongodb_reindex_id_walk_rejects_a_foreign_cursor() {
    let Some(backend) = create_backend("reindex_id_walk_bad_cursor").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("walk-bad-cursor");
    backend
        .create(
            &tenant,
            "Patient",
            json!({ "resourceType": "Patient", "id": "p1", "name": [{ "family": "X" }] }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let head_style = Some("2026-09-19T04:43:29.668+00:00|e357ce58-f379-216d-a369-99da40ff76ae");
    let err = backend
        .fetch_resources_page(&tenant, "Patient", head_style, 10)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        StorageError::Search(SearchError::InvalidCursor { .. })
    ));

    let err = backend
        .fetch_resources_page(&tenant, "Patient", Some("garbage"), 10)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        StorageError::Search(SearchError::InvalidCursor { .. })
    ));
}

/// All rows of `search_index` and `search_index_contained` for `tenant_id`,
/// as canonical sorted JSON strings (order-independent, `_id`-independent).
async fn index_rows(
    db: &mongodb::Database,
    collection: &str,
    tenant_id: &str,
    strip_tenant: bool,
) -> Vec<String> {
    use futures::stream::TryStreamExt;
    let coll = db.collection::<Document>(collection);
    let mut rows: Vec<Document> = coll
        .find(doc! { "tenant_id": tenant_id })
        .projection(doc! { "_id": 0 })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    if strip_tenant {
        for row in &mut rows {
            row.remove("tenant_id");
        }
    }
    let mut lines: Vec<String> = rows
        .into_iter()
        .map(|d| canonical(mongodb::bson::Bson::Document(d).into_relaxed_extjson()).to_string())
        .collect();
    lines.sort();
    lines
}

/// Rebuilds `v` with every object's keys sorted, recursively, so two BSON
/// documents with the same content but different field insertion order
/// snapshot identically (#1403). `serde_json`'s `preserve_order` feature is
/// enabled workspace-wide (`crates/sof/Cargo.toml`, and `helios-persistence`
/// depends on `helios-sof`), so without this a `Value::Object`'s iteration
/// order otherwise follows BSON insertion order rather than being sorted.
fn canonical(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<(String, serde_json::Value)> =
                map.into_iter().map(|(k, v)| (k, canonical(v))).collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            serde_json::Value::Object(entries.into_iter().collect())
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(canonical).collect())
        }
        other => other,
    }
}

async fn snapshot(
    db: &mongodb::Database,
    tenant_id: &str,
    strip_tenant: bool,
) -> (Vec<String>, Vec<String>) {
    (
        index_rows(db, "search_index", tenant_id, strip_tenant).await,
        index_rows(db, "search_index_contained", tenant_id, strip_tenant).await,
    )
}

/// Routes `helios_persistence::backends::mongodb::storage` events at `debug`
/// and above into `tracing-test`'s global buffer, once per test binary. Every
/// walk test that asserts on log lines calls this before it starts its walk.
fn capture_walk_logs() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let writer =
            tracing_test::internal::MockWriter::new(tracing_test::internal::global_buf());
        let dispatch = tracing_test::internal::get_subscriber(
            writer,
            "helios_persistence::backends::mongodb::storage=debug",
        );
        tracing::dispatcher::set_global_default(dispatch)
            .expect("no other global tracing subscriber in this test binary");
    });
}

/// Captured log lines containing every one of `needles`.
fn walk_log_lines(needles: &[&str]) -> Vec<String> {
    let buf = tracing_test::internal::global_buf().lock().unwrap();
    String::from_utf8_lossy(&buf)
        .lines()
        .filter(|line| needles.iter().all(|needle| line.contains(needle)))
        .map(str::to_string)
        .collect()
}

/// HEAD's walk, verbatim and test-only: `(last_updated, id)` keyset, no hint,
/// `"{rfc3339}|{id}"` cursor. Copied rather than reused because PR1 replaces
/// the production implementation.
struct LegacyWalkSource {
    backend: std::sync::Arc<MongoBackend>,
}

#[async_trait::async_trait]
impl helios_persistence::search::ReindexSource for LegacyWalkSource {
    async fn list_resource_types(
        &self,
        tenant: &TenantContext,
    ) -> StorageResult<Vec<String>> {
        self.backend.list_resource_types(tenant).await
    }

    async fn count_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64> {
        self.backend.count_resources(tenant, resource_type).await
    }

    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> StorageResult<helios_persistence::search::ResourcePage> {
        let db = self.backend.get_database().await?;
        let resources = db.collection::<Document>("resources");

        let mut stream = resources
            .find(legacy_filter(tenant, resource_type, cursor))
            .sort(doc! { "last_updated": 1, "id": 1 })
            .limit(limit as i64)
            .await
            .map_err(|e| StorageError::Backend(BackendError::Internal {
                backend_name: "mongodb".to_string(),
                message: format!("legacy walk find: {e}"),
                source: None,
            }))?;
        let mut docs: Vec<Document> = Vec::new();
        while stream.advance().await.map_err(|e| {
            StorageError::Backend(BackendError::Internal {
                backend_name: "mongodb".to_string(),
                message: format!("legacy walk advance: {e}"),
                source: None,
            })
        })? {
            docs.push(stream.deserialize_current().map_err(|e| {
                StorageError::Backend(BackendError::Internal {
                    backend_name: "mongodb".to_string(),
                    message: format!("legacy walk deserialize: {e}"),
                    source: None,
                })
            })?);
        }

        let full_page = docs.len() as u32 == limit;
        let next_cursor = match (full_page, docs.last()) {
            (true, Some(last)) => {
                let dt = last.get_datetime("last_updated").unwrap();
                let id = last.get_str("id").unwrap();
                let lu = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(dt.timestamp_millis()).unwrap();
                Some(format!("{}|{}", lu.to_rfc3339(), id))
            }
            _ => None,
        };

        let resources_out: StorageResult<Vec<_>> = docs
            .iter()
            .map(|d| {
                let dt = d.get_datetime("last_updated").unwrap();
                let lu = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(dt.timestamp_millis()).unwrap();
                let data = d.get_document("data").unwrap();
                let content: serde_json::Value =
                    mongodb::bson::from_document(data.clone()).unwrap();
                Ok(helios_persistence::types::StoredResource::from_storage(
                    resource_type,
                    d.get_str("id").unwrap(),
                    d.get_str("version_id").unwrap(),
                    tenant.tenant_id().clone(),
                    content,
                    lu,
                    lu,
                    None,
                    FhirVersion::default(),
                ))
            })
            .collect();

        Ok(helios_persistence::search::ResourcePage {
            resources: resources_out?,
            next_cursor,
            skipped: Vec::new(),
        })
    }
}

/// HEAD's exact `fetch_resources_page` filter (`storage.rs:4790-4809` at
/// c86d0f08b, before PR1 replaces it): `(last_updated, id)` keyset, no hint.
fn legacy_filter(tenant: &TenantContext, resource_type: &str, cursor: Option<&str>) -> Document {
    let mut filter = doc! {
        "tenant_id": tenant.tenant_id().as_str(),
        "resource_type": resource_type,
        "is_deleted": false,
    };
    if let Some(cursor) = cursor {
        if let Some((ts_str, id)) = cursor.split_once('|') {
            if let Ok(cur_dt) = chrono::DateTime::parse_from_rfc3339(ts_str) {
                let cur_dt = cur_dt.with_timezone(&chrono::Utc);
                filter.insert(
                    "$or",
                    vec![
                        doc! { "last_updated": { "$gt": BsonDateTime::from_millis(cur_dt.timestamp_millis()) } },
                        doc! {
                            "last_updated": BsonDateTime::from_millis(cur_dt.timestamp_millis()),
                            "id": { "$gt": id },
                        },
                    ],
                );
            }
        }
    }
    filter
}

// ===========================================================================
// T1: parity with HEAD's walk
// ===========================================================================

#[tokio::test]
async fn mongodb_reindex_id_walk_matches_the_legacy_walk_row_for_row() {
    use std::sync::Arc;

    let Some(backend) = create_backend("reindex_id_walk_parity").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    capture_walk_logs();

    let tenant_a = create_tenant("walk-a");
    let tenant_b = create_tenant("walk-b");
    let fixture_a = seed_walk_fixture(&backend, &tenant_a, 300, "only-in-a").await;
    let fixture_b = seed_walk_fixture(&backend, &tenant_b, 300, "only-in-b").await;

    let db = backend.get_database().await.unwrap();
    let s_crud_a = snapshot(&db, "walk-a", false).await;
    let s_crud_b = snapshot(&db, "walk-b", false).await;

    backdate_fixture(&backend, &tenant_a, &fixture_a).await;
    backdate_fixture(&backend, &tenant_b, &fixture_b).await;

    let regs = backend.tenant_registries().clone();
    let request = || {
        ReindexRequest::for_types(["Observation", "Patient"])
            .clear_existing()
            .with_batch_size(40)
    };

    let legacy_source = Arc::new(LegacyWalkSource { backend: backend.clone() });
    let legacy_op = ReindexOperation::with_parts(legacy_source, vec![backend.clone()], regs.clone());
    let legacy_job = legacy_op.start(tenant_a.clone(), request(), None).await.unwrap();
    let legacy_progress = wait_for_terminal(&legacy_op, &legacy_job).await;
    let s_old = snapshot(&db, "walk-a", false).await;

    let new_op = ReindexOperation::new(backend.clone(), regs.clone());
    let new_job = new_op.start(tenant_a.clone(), request(), None).await.unwrap();
    let new_progress = wait_for_terminal(&new_op, &new_job).await;
    let s_new = snapshot(&db, "walk-a", false).await;

    assert_eq!(s_new, s_old, "new walk must match HEAD's walk row for row");
    assert_eq!(
        s_new, s_crud_a,
        "reindex must match CRUD indexing exactly (#1064) — if only this \
         assertion fails, stop and report; do not change the writer"
    );
    assert_eq!(snapshot(&db, "walk-b", false).await, s_crud_b);

    for progress in [&legacy_progress, &new_progress] {
        assert_eq!(progress.status, helios_persistence::search::ReindexStatus::Completed);
        assert!(progress.errors.is_empty());
        assert_eq!(progress.processed_resources, progress.total_resources);
        assert_eq!(progress.processed_resources, 297 + 10);
    }
    assert_eq!(legacy_progress.entries_created, new_progress.entries_created);

    let obs_started =
        walk_log_lines(&["tenant=walk-a", "resource_type=Observation", "mongodb reindex walk started"]);
    assert!(
        obs_started.iter().any(|l| l.contains("newest_live=2020-01-01T00:00:02.000Z")
            && l.contains("floor=2020-01-01T00:00:02.001Z")),
        "{obs_started:?}"
    );
    let patient_started =
        walk_log_lines(&["tenant=walk-a", "resource_type=Patient", "mongodb reindex walk started"]);
    assert!(
        patient_started.iter().any(|l| l.contains("newest_live=2020-01-01T00:00:03.000Z")
            && l.contains("floor=2020-01-01T00:00:03.001Z")),
        "{patient_started:?}"
    );
    for rt in ["Observation", "Patient"] {
        let finished = walk_log_lines(&[
            "tenant=walk-a",
            &format!("resource_type={rt}"),
            "mongodb reindex catch-up round finished",
            "round=1",
        ]);
        assert!(finished.iter().any(|l| l.contains("walked=0")), "{rt}: {finished:?}");
    }

    // Rerun without clear_existing: no duplicates should appear.
    let rerun_job = new_op.start(tenant_a.clone(), ReindexRequest::for_types(["Observation", "Patient"]).with_batch_size(40), None).await.unwrap();
    wait_for_terminal(&new_op, &rerun_job).await;
    assert_eq!(snapshot(&db, "walk-a", false).await, s_crud_a);
}

async fn wait_for_terminal(
    op: &helios_persistence::search::ReindexOperation,
    job_id: &str,
) -> helios_persistence::search::ReindexProgress {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let progress = op.get_progress(job_id).await.unwrap();
            if progress.status.is_finished() {
                return progress;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("reindex status should become terminal")
}

// ===========================================================================
// T9: plans — hint honoured, no blocking sort, no from-floor re-scan (#1021)
// ===========================================================================

#[tokio::test]
async fn mongodb_reindex_id_walk_pages_plan_without_a_blocking_sort() {
    use futures::stream::TryStreamExt;

    let Some(backend) = create_backend("reindex_id_walk_plan").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("walk-plan");
    let fixture = seed_walk_fixture(&backend, &tenant, 300, "walk-plan-extra").await;
    backdate_fixture(&backend, &tenant, &fixture).await;

    let db = backend.get_database().await.unwrap();
    let resources = db.collection::<Document>("resources");
    let tail_ids: Vec<String> = (250..300).map(|i| format!("obs-{i:03}")).collect();
    resources
        .update_many(
            doc! {
                "tenant_id": "walk-plan",
                "resource_type": "Observation",
                "id": { "$in": &tail_ids },
            },
            doc! {
                "$set": {
                    "last_updated": BsonDateTime::from_millis(
                        (chrono::Utc::now() - chrono::Duration::seconds(10)).timestamp_millis(),
                    ),
                },
            },
        )
        .await
        .unwrap();

    let profiling_enabled = db.run_command(doc! { "profile": 2_i32 }).await.is_ok();
    if !profiling_enabled {
        eprintln!(
            "mongodb_reindex_id_walk_pages_plan_without_a_blocking_sort: server refused \
             {{profile: 2}} (likely a managed/shared HFS_TEST_MONGODB_URL); skipping the \
             plan assertions"
        );
        return;
    }

    let mut cursor: Option<String> = None;
    loop {
        let page = backend
            .fetch_resources_page(&tenant, "Observation", cursor.as_deref(), 20)
            .await
            .unwrap();
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    let _ = db.run_command(doc! { "profile": 0_i32 }).await;

    let profile: mongodb::Collection<Document> = db.collection("system.profile");
    let entries: Vec<Document> = profile
        .find(doc! {
            "ns": format!("{}.resources", db.name()),
            "op": "query",
            "command.find": "resources",
        })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();

    let mut id_queries = 0u32;
    let mut probes = 0u32;
    let mut round_queries = 0u32;
    let mut round_queries_with_or = 0u32;

    for entry in &entries {
        let command = match entry.get_document("command") {
            Ok(c) => c,
            Err(_) => continue,
        };
        let hint = command.get_str("hint").ok();
        let sort = command.get_document("sort").ok();
        let has_sort_stage = entry.get_bool("hasSortStage").unwrap_or(false);
        let keys_examined = entry
            .get_i64("keysExamined")
            .or_else(|_| entry.get_i32("keysExamined").map(i64::from))
            .unwrap_or(0);
        let docs_examined = entry
            .get_i64("docsExamined")
            .or_else(|_| entry.get_i32("docsExamined").map(i64::from))
            .unwrap_or(0);

        let mut inner = Document::new();
        for key in ["find", "filter", "sort", "limit", "projection", "hint"] {
            if let Some(v) = command.get(key) {
                inner.insert(key, v.clone());
            }
        }
        let explain = db
            .run_command(doc! { "explain": inner, "verbosity": "executionStats" })
            .await
            .unwrap();
        let winning_plan = match explain
            .get_document("queryPlanner")
            .and_then(|qp| qp.get_document("winningPlan"))
        {
            Ok(p) => p.clone(),
            Err(_) => continue,
        };
        let mut names = Vec::new();
        collect_index_names(&winning_plan, &mut names);
        let has_sort = contains_stage_named(&winning_plan, "SORT");

        let is_probe = sort == Some(&doc! { "last_updated": -1_i32, "id": -1_i32 });
        match hint {
            Some(h) if h == "idx_resources_identity" => {
                id_queries += 1;
                assert!(!has_sort_stage, "id query used a blocking sort");
                assert!(!has_sort, "id query plan has a SORT stage");
                assert!(
                    !names.is_empty() && names.iter().all(|n| n == "idx_resources_identity"),
                    "{names:?}"
                );
                assert!(keys_examined <= 20 + 3 + 50 + 1, "id query examined {keys_examined} keys");
            }
            Some(h) if h == "idx_resources_type_scan" && is_probe => {
                probes += 1;
                assert!(
                    !names.is_empty() && names.iter().all(|n| n == "idx_resources_type_scan"),
                    "{names:?}"
                );
                assert!(!has_sort);
                assert!(keys_examined <= 2, "probe examined {keys_examined} keys");
                assert_eq!(docs_examined, 0, "the probe must be covered");
            }
            Some(h) if h == "idx_resources_type_scan" => {
                round_queries += 1;
                // A round's continuation query plans as SORT_MERGE of two
                // IXSCANs of the same index (run 17's measured plan, S2 §4.3),
                // so `names` holds two equal entries here, not one —
                // `collect_index_names` does not de-duplicate.
                assert!(
                    !names.is_empty() && names.iter().all(|n| n == "idx_resources_type_scan"),
                    "{names:?}"
                );
                assert!(!has_sort, "round query plan has a SORT stage (SORT_MERGE is fine)");
                assert!(!has_sort_stage);
                assert!(
                    keys_examined <= 20 + 2,
                    "round query examined {keys_examined} keys (>= 40 would mean a from-floor rescan)"
                );
                let filter_has_or = command
                    .get_document("filter")
                    .map(|f| f.contains_key("$or"))
                    .unwrap_or(false);
                if filter_has_or {
                    round_queries_with_or += 1;
                }
            }
            _ => {}
        }
    }

    assert!(id_queries >= 13, "expected at least 13 id queries, got {id_queries}");
    assert!(probes >= 2, "expected at least 2 probes, got {probes}");
    assert!(round_queries >= 4, "expected at least 4 round queries, got {round_queries}");
    assert!(
        round_queries_with_or >= 3,
        "expected at least 3 round queries carrying $or, got {round_queries_with_or}"
    );
}

// ===========================================================================
// Shared search-assertion helper (T3, T4; also T5, T6 in a later task)
// ===========================================================================

/// A search for `Observation?identifier=urn:walk|{value}`, as every
/// self-healing test below uses to confirm the walk indexed the resource's
/// post-mutation content rather than a stale snapshot.
fn walk_identifier_query(value: &str) -> SearchQuery {
    SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token(Some("urn:walk"), value)],
        chain: vec![],
        components: vec![],
    })
}

// ===========================================================================
// Test doubles for T3, T4, T7, T8
// ===========================================================================

/// Delegates to `inner`, and awaits `mutation()` once — after the inner call
/// returns a page containing `trigger_id`, before this call returns it — so
/// the mutation lands after the page's fetch and before its write,
/// deterministically, in either walk.
struct MutatingSource {
    inner: std::sync::Arc<dyn helios_persistence::search::ReindexSource>,
    trigger_id: String,
    fired: std::sync::atomic::AtomicBool,
    mutation: Box<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>,
}

#[async_trait::async_trait]
impl helios_persistence::search::ReindexSource for MutatingSource {
    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>> {
        self.inner.list_resource_types(tenant).await
    }

    async fn count_resources(&self, tenant: &TenantContext, resource_type: &str) -> StorageResult<u64> {
        self.inner.count_resources(tenant, resource_type).await
    }

    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> StorageResult<helios_persistence::search::ResourcePage> {
        let page = self.inner.fetch_resources_page(tenant, resource_type, cursor, limit).await?;
        if !self.fired.load(std::sync::atomic::Ordering::SeqCst)
            && page.resources.iter().any(|r| r.id() == self.trigger_id)
        {
            self.fired.store(true, std::sync::atomic::Ordering::SeqCst);
            (self.mutation)().await;
        }
        Ok(page)
    }
}

/// Records each written resource's `(id, version_id)` and delegates every
/// write to `inner`; sleeps `delay_after_page` after each page (if nonzero)
/// and notifies `page_written`.
struct RecordingTarget {
    inner: std::sync::Arc<MongoBackend>,
    writes: std::sync::Mutex<Vec<(String, String)>>,
    pages_written: std::sync::atomic::AtomicUsize,
    page_written: tokio::sync::Notify,
    delay_after_page: std::time::Duration,
}

impl RecordingTarget {
    fn new(inner: std::sync::Arc<MongoBackend>) -> Self {
        Self {
            inner,
            writes: std::sync::Mutex::new(Vec::new()),
            pages_written: std::sync::atomic::AtomicUsize::new(0),
            page_written: tokio::sync::Notify::new(),
            delay_after_page: std::time::Duration::ZERO,
        }
    }

    fn with_delay(mut self, delay: std::time::Duration) -> Self {
        self.delay_after_page = delay;
        self
    }
}

#[async_trait::async_trait]
impl helios_persistence::search::ReindexTarget for RecordingTarget {
    async fn delete_search_entries(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<u64> {
        self.inner.delete_search_entries(tenant, resource_type, resource_id).await
    }

    async fn write_search_entries(
        &self,
        tenant: &TenantContext,
        resource: &helios_persistence::types::StoredResource,
    ) -> StorageResult<usize> {
        self.inner.write_search_entries(tenant, resource).await
    }

    async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
        self.inner.clear_search_index(tenant).await
    }

    async fn write_search_entries_page(
        &self,
        tenant: &TenantContext,
        resources: &[helios_persistence::types::StoredResource],
    ) -> Vec<StorageResult<usize>> {
        for r in resources {
            self.writes
                .lock()
                .unwrap()
                .push((r.id().to_string(), r.version_id().to_string()));
        }
        let results = self.inner.write_search_entries_page(tenant, resources).await;
        if !self.delay_after_page.is_zero() {
            tokio::time::sleep(self.delay_after_page).await;
        }
        self.pages_written.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.page_written.notify_one();
        results
    }
}

// ===========================================================================
// T3: an update between fetch and write of an id-phase page heals
// ===========================================================================

#[tokio::test]
async fn mongodb_reindex_id_walk_heals_an_update_between_fetch_and_write() {
    use std::sync::Arc;

    async fn run_for(
        backend: Arc<MongoBackend>,
        source: Arc<dyn helios_persistence::search::ReindexSource>,
        tenant: &TenantContext,
        regs: Arc<helios_persistence::search::TenantSearchRegistries>,
    ) -> (Arc<RecordingTarget>, helios_persistence::search::ReindexProgress) {
        let target = Arc::new(RecordingTarget::new(backend));
        let op = ReindexOperation::with_parts(source, vec![target.clone()], regs);
        let job = op
            .start(tenant.clone(), ReindexRequest::for_types(["Observation"]).with_batch_size(10), None)
            .await
            .unwrap();
        let progress = wait_for_terminal(&op, &job).await;
        (target, progress)
    }

    let Some(backend) = create_backend("reindex_id_walk_heal_id").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let tenant_legacy = create_tenant("walk-upd-legacy");
    let tenant_new = create_tenant("walk-upd-new");
    for tenant in [&tenant_legacy, &tenant_new] {
        let fixture = seed_walk_fixture(&backend, tenant, 60, "extra").await;
        backdate_fixture(&backend, tenant, &fixture).await;
    }

    let mutation_for = |backend: Arc<MongoBackend>, tenant: TenantContext| {
        Box::new(move || {
            let backend = backend.clone();
            let tenant = tenant.clone();
            Box::pin(async move {
                let current = backend.read(&tenant, "Observation", "obs-017").await.unwrap().unwrap();
                let mut content = current.content().clone();
                content["valueQuantity"]["value"] = json!(9999);
                content["identifier"][0]["value"] = json!("o-17-updated");
                backend.update(&tenant, &current, content).await.unwrap();
            }) as futures::future::BoxFuture<'static, ()>
        }) as Box<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>
    };

    let regs = backend.tenant_registries().clone();

    let legacy_source: Arc<dyn helios_persistence::search::ReindexSource> = Arc::new(MutatingSource {
        inner: Arc::new(LegacyWalkSource { backend: backend.clone() }),
        trigger_id: "obs-017".to_string(),
        fired: std::sync::atomic::AtomicBool::new(false),
        mutation: mutation_for(backend.clone(), tenant_legacy.clone()),
    });
    run_for(backend.clone(), legacy_source, &tenant_legacy, regs.clone()).await;

    let new_source: Arc<dyn helios_persistence::search::ReindexSource> = Arc::new(MutatingSource {
        inner: backend.clone(),
        trigger_id: "obs-017".to_string(),
        fired: std::sync::atomic::AtomicBool::new(false),
        mutation: mutation_for(backend.clone(), tenant_new.clone()),
    });
    let (target, progress) = run_for(backend.clone(), new_source, &tenant_new, regs.clone()).await;
    assert_eq!(progress.status, helios_persistence::search::ReindexStatus::Completed);
    assert!(progress.errors.is_empty());

    let writes = target.writes.lock().unwrap().clone();
    let v1_idx = writes.iter().position(|(id, v)| id == "obs-017" && v == "1");
    let v2_idx = writes.iter().position(|(id, v)| id == "obs-017" && v == "2");
    assert!(v1_idx.is_some() && v2_idx.is_some() && v1_idx < v2_idx, "{writes:?}");

    let db = backend.get_database().await.unwrap();
    let s_legacy = snapshot(&db, "walk-upd-legacy", true).await;
    let s_new = snapshot(&db, "walk-upd-new", true).await;
    assert_eq!(s_new, s_legacy);

    let query = walk_identifier_query("o-17-updated");
    let found = backend.search(&tenant_new, &query).await.unwrap();
    assert_eq!(found.resources.items.len(), 1);
    let stale_query = walk_identifier_query("o-17");
    assert!(backend.search(&tenant_new, &stale_query).await.unwrap().resources.items.is_empty());

    let s_final = snapshot(&db, "walk-upd-new", false).await;
    let resource = backend.read(&tenant_new, "Observation", "obs-017").await.unwrap().unwrap();
    backend.write_search_entries(&tenant_new, &resource).await.unwrap();
    assert_eq!(snapshot(&db, "walk-upd-new", false).await, s_final);
}

// ===========================================================================
// T4: an update racing a catch-up page heals
// ===========================================================================

#[tokio::test]
async fn mongodb_reindex_id_walk_heals_an_update_racing_a_catch_up_page() {
    use std::sync::Arc;

    let Some(backend) = create_backend("reindex_id_walk_heal_round").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let tenant = create_tenant("walk-rnd");
    let fixture = seed_walk_fixture(&backend, &tenant, 60, "extra").await;
    backdate_fixture(&backend, &tenant, &fixture).await;

    for i in [10, 20, 30] {
        let id = format!("obs-{i:03}");
        let current = backend.read(&tenant, "Observation", &id).await.unwrap().unwrap();
        let mut content = current.content().clone();
        content["identifier"][0]["value"] = json!(format!("o-{i}-v2"));
        backend.update(&tenant, &current, content).await.unwrap();
    }

    let trigger_tenant = tenant.clone();
    let backend_for_mutation = backend.clone();
    let mutation: Box<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync> =
        Box::new(move || {
            let backend = backend_for_mutation.clone();
            let tenant = trigger_tenant.clone();
            Box::pin(async move {
                let current = backend.read(&tenant, "Observation", "obs-020").await.unwrap().unwrap();
                let mut content = current.content().clone();
                content["identifier"][0]["value"] = json!("o-20-v3");
                backend.update(&tenant, &current, content).await.unwrap();
            }) as futures::future::BoxFuture<'static, ()>
        });

    let source: Arc<dyn helios_persistence::search::ReindexSource> = Arc::new(MutatingSource {
        inner: backend.clone(),
        trigger_id: "obs-020".to_string(),
        fired: std::sync::atomic::AtomicBool::new(false),
        mutation,
    });
    let target = Arc::new(RecordingTarget::new(backend.clone()));
    let op = ReindexOperation::with_parts(source, vec![target.clone()], backend.tenant_registries().clone());
    let job = op
        .start(tenant.clone(), ReindexRequest::for_types(["Observation"]).with_batch_size(10), None)
        .await
        .unwrap();
    let progress = wait_for_terminal(&op, &job).await;
    assert_eq!(progress.status, helios_persistence::search::ReindexStatus::Completed);
    assert!(progress.errors.is_empty());

    let writes = target.writes.lock().unwrap().clone();
    let v2_idx = writes.iter().position(|(id, v)| id == "obs-020" && v == "2");
    let v3_idx = writes.iter().position(|(id, v)| id == "obs-020" && v == "3");
    assert!(
        v2_idx.is_some() && v3_idx.is_some() && v2_idx < v3_idx,
        "under a design that ended the round on the short page, version 3 would never be written: {writes:?}"
    );

    let query = walk_identifier_query("o-20-v3");
    assert_eq!(backend.search(&tenant, &query).await.unwrap().resources.items.len(), 1);
    let stale_query = walk_identifier_query("o-20-v2");
    assert!(backend.search(&tenant, &stale_query).await.unwrap().resources.items.is_empty());

    let db = backend.get_database().await.unwrap();
    let s_final = snapshot(&db, "walk-rnd", false).await;
    let resource = backend.read(&tenant, "Observation", "obs-020").await.unwrap().unwrap();
    backend.write_search_entries(&tenant, &resource).await.unwrap();
    assert_eq!(snapshot(&db, "walk-rnd", false).await, s_final);
}
