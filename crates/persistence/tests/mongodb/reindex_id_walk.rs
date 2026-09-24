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
use helios_persistence::search::{ReindexOperation, ReindexRequest, ReindexSource};

// `StorageResult` is used bare (not fully qualified) throughout this file's
// test doubles below. `ReindexSource` must be in scope for its methods to be
// callable as `x.fetch_resources_page(..)` — Rust only resolves a trait
// method by dot-call syntax when the trait itself is imported, even though
// the concrete type (`MongoBackend`) already implements it; without this
// import, every such call is E0599 ("no method named ... found — the
// following trait is implemented but not in scope"). `ReindexStatus` and
// `ResourcePage` are deliberately NOT imported here: every use of them in
// this file is already fully qualified
// (`helios_persistence::search::ReindexStatus::Completed` etc.), so
// importing the bare name would be an `unused_imports` error under
// `-D warnings` — they are plain types, not traits, so (unlike
// `ReindexSource`) a fully-qualified reference elsewhere does not count as
// "using" a bare import of them.

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
