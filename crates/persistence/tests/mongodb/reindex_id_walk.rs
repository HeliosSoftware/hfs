//! #1403: the id-order `$reindex` walk and its catch-up rounds.
//!
//! Uses `super::*` for the parent test crate's imports and private harness
//! helpers (`create_backend`, `create_tenant`, `build_test_database_name`,
//! etc.) — this file is a `#[path]`-included child module of
//! `mongodb_tests.rs`, not a standalone test binary.

use super::*;
use std::collections::{BTreeMap, BTreeSet};
use mongodb::bson::{DateTime as BsonDateTime, Document};
use helios_persistence::search::ReindexSource;

// `ReindexSource` must be in scope for its methods to be callable as
// `x.fetch_resources_page(..)` — Rust only resolves a trait method by
// dot-call syntax when the trait itself is imported, even though the
// concrete type (`MongoBackend`) already implements it; without this
// import, every such call is E0599 ("no method named ... found — the
// following trait is implemented but not in scope").

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
