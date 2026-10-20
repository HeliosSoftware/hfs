//! `HFS_MONGODB_BUNDLE_BATCH_WRITES`: a transaction Bundle of plain POST
//! creates written with batched round trips must behave exactly like the
//! per-entry path. Each test runs the same Bundle against two fresh databases,
//! one with `bundle_batch_writes` off and one on, and compares the outcome
//! (success shape or the attributed error) and the stored state.
//!
//! Child module of the `mongodb_tests` root; `use super::*;` reaches its
//! private harness. Setting `HFS_TEST_MONGODB_BUNDLE_BATCH_WRITES=true` also
//! turns the batched path on for every other backend the root builds (see
//! `build_backend`), which runs the whole existing suite as a second
//! configuration.

use super::*;
use serde_json::Value;

const MRN: &str = "http://example.org/batch-mrn";

async fn backend_with(
    test: &str,
    batch: bool,
    configure: impl FnOnce(&mut MongoBackendConfig),
) -> Option<MongoBackend> {
    let connection_string = shared_mongo::connection_string().await?;
    let mut config = MongoBackendConfig {
        connection_string,
        database_name: format!(
            "mbf_{}_{}",
            test.chars().take(30).collect::<String>(),
            uuid::Uuid::new_v4().simple()
        ),
        data_dir: Some(repo_data_dir()),
        bundle_batch_writes: batch,
        ..Default::default()
    };
    configure(&mut config);
    // `build_backend` may force the flag on through the environment; keep the
    // explicit setting authoritative for these paired tests.
    let backend = build_backend(config.clone()).await?;
    assert_eq!(backend.config().bundle_batch_writes, batch || env_batch());
    Some(backend)
}

fn env_batch() -> bool {
    std::env::var("HFS_TEST_MONGODB_BUNDLE_BATCH_WRITES")
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn post(resource: Value, full_url: Option<&str>) -> BundleEntry {
    BundleEntry {
        method: BundleMethod::Post,
        url: resource["resourceType"].as_str().unwrap().to_string(),
        resource: Some(resource),
        full_url: full_url.map(str::to_string),
        ..Default::default()
    }
}

fn patient(id: &str) -> Value {
    json!({"resourceType": "Patient", "id": id,
        "identifier": [{"system": MRN, "value": id}], "name": [{"family": id}]})
}

/// Outcome of a Bundle reduced to what must match across both paths.
#[derive(Debug, PartialEq)]
enum Outcome {
    Ok(Vec<(u16, String)>),
    BundleError(usize, String),
    Other(String),
}

async fn run(backend: &MongoBackend, tenant: &TenantContext, entries: Vec<BundleEntry>) -> Outcome {
    match backend
        .process_transaction(tenant, entries, FhirVersion::default())
        .await
    {
        Ok(result) => Outcome::Ok(
            result
                .entries
                .iter()
                .map(|e| {
                    let location = e.location.clone().unwrap_or_default();
                    // Ids minted by the server differ between runs.
                    let kind = location.split('/').next().unwrap_or("").to_string();
                    (e.status, kind)
                })
                .collect(),
        ),
        Err(TransactionError::BundleError { index, message }) => {
            Outcome::BundleError(index, message)
        }
        Err(other) => Outcome::Other(format!("{other:?}")),
    }
}

/// Counts and the sorted (resource, param) pairs of the search rows.
async fn snapshot(backend: &MongoBackend) -> Vec<String> {
    use futures::TryStreamExt;
    let client = raw_test_client(&backend.config().connection_string)
        .await
        .unwrap();
    let db = client.database(&backend.config().database_name);
    let mut out = Vec::new();
    for name in ["resources", "resource_history", "search_index_contained"] {
        let n = db
            .collection::<Document>(name)
            .count_documents(doc! {})
            .await
            .unwrap();
        out.push(format!("{name}={n}"));
    }
    let rows: Vec<Document> = db
        .collection::<Document>("search_index")
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let mut pairs: Vec<String> = rows
        .iter()
        .map(|r| {
            format!(
                "{}/{}:{}",
                r.get_str("resource_type").unwrap_or(""),
                r.get_str("resource_id").unwrap_or(""),
                r.get_str("param_name").unwrap_or("")
            )
        })
        .collect();
    pairs.sort();
    out.extend(pairs);
    out
}

/// State created before the Bundle runs.
#[derive(Clone, Copy)]
enum Seed {
    Nothing,
    Existing(&'static str),
    Tombstone(&'static str),
}

async fn seed(backend: &MongoBackend, tenant: &TenantContext, seed: Seed) {
    let (id, delete) = match seed {
        Seed::Nothing => return,
        Seed::Existing(id) => (id, false),
        Seed::Tombstone(id) => (id, true),
    };
    backend
        .create(tenant, "Patient", patient(id), FhirVersion::default())
        .await
        .unwrap();
    if delete {
        backend.delete(tenant, "Patient", id).await.unwrap();
    }
}

/// Runs `entries_of` against both configurations (after `seed`) and asserts
/// the outcomes and stored state agree. Returns the shared outcome.
async fn compare(
    test: &str,
    state: Seed,
    entries_of: impl Fn() -> Vec<BundleEntry>,
    configure: impl Fn(&mut MongoBackendConfig),
) -> Option<Outcome> {
    let tenant = create_tenant("batch-writes");
    let off = backend_with(&format!("{test}_off"), false, &configure).await?;
    let on = backend_with(&format!("{test}_on"), true, &configure).await?;
    seed(&off, &tenant, state).await;
    seed(&on, &tenant, state).await;
    let a = run(&off, &tenant, entries_of()).await;
    let b = run(&on, &tenant, entries_of()).await;
    assert_eq!(a, b, "{test}: outcomes differ");
    assert_eq!(
        snapshot(&off).await,
        snapshot(&on).await,
        "{test}: stored state differs"
    );
    assert_eq!(
        content_hashes(&off).await,
        content_hashes(&on).await,
        "{test}: stored documents differ"
    );
    drop_database(&off).await;
    drop_database(&on).await;
    Some(a)
}

#[tokio::test]
async fn mongodb_batch_writes_plain_creates_match_the_per_entry_path() {
    let Some(outcome) = compare(
        "batch_plain",
        Seed::Nothing,
        || {
            (0..5)
                .map(|i| post(patient(&format!("p{i}")), None))
                .collect()
        },
        |_| {},
    )
    .await
    else {
        return;
    };
    assert!(
        matches!(outcome, Outcome::Ok(ref r) if r.len() == 5),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn mongodb_batch_writes_duplicate_in_bundle_fails_at_the_repeat() {
    let Some(outcome) = compare(
        "batch_dup",
        Seed::Nothing,
        || {
            vec![
                post(patient("a"), None),
                post(patient("b"), None),
                post(patient("a"), None),
                post(patient("c"), None),
            ]
        },
        |_| {},
    )
    .await
    else {
        return;
    };
    assert!(matches!(outcome, Outcome::BundleError(2, _)), "{outcome:?}");
}

#[tokio::test]
async fn mongodb_batch_writes_existing_id_fails_at_that_entry_and_writes_nothing() {
    let Some(outcome) = compare(
        "batch_existing",
        Seed::Existing("x"),
        || {
            vec![
                post(patient("a"), None),
                post(patient("b"), None),
                post(patient("x"), None),
                post(patient("c"), None),
            ]
        },
        |_| {},
    )
    .await
    else {
        return;
    };
    assert!(matches!(outcome, Outcome::BundleError(2, _)), "{outcome:?}");
}

#[tokio::test]
async fn mongodb_batch_writes_soft_deleted_id_is_treated_like_the_per_entry_path() {
    let Some(outcome) = compare(
        "batch_tombstone",
        Seed::Tombstone("gone"),
        || vec![post(patient("ok"), None), post(patient("gone"), None)],
        |_| {},
    )
    .await
    else {
        return;
    };
    // The per-entry path refuses the tombstoned id; the batched path must too.
    assert!(matches!(outcome, Outcome::BundleError(1, _)), "{outcome:?}");
}

#[tokio::test]
async fn mongodb_batch_writes_urn_uuid_forward_and_backward_references_resolve() {
    let tenant = create_tenant("batch-writes");
    for batch in [false, true] {
        let Some(backend) = backend_with(&format!("batch_urn_{batch}"), batch, |_| {}).await else {
            return;
        };
        let entries = vec![
            post(
                json!({"resourceType": "Observation", "id": "o1", "status": "final",
                    "code": {"text": "c"},
                    "subject": {"reference": "urn:uuid:pat-1"}}),
                Some("urn:uuid:obs-1"),
            ),
            post(patient("p1"), Some("urn:uuid:pat-1")),
            post(
                json!({"resourceType": "Observation", "id": "o2", "status": "final",
                    "code": {"text": "c"},
                    "subject": {"reference": "urn:uuid:pat-1"}}),
                None,
            ),
        ];
        let result = backend
            .process_transaction(&tenant, entries, FhirVersion::default())
            .await;
        let Ok(_) = result else {
            if matches!(
                result,
                Err(TransactionError::UnsupportedIsolationLevel { .. })
            ) {
                return;
            }
            panic!("batch={batch}: {result:?}");
        };
        for id in ["o1", "o2"] {
            let stored = backend
                .read(&tenant, "Observation", id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                stored.content()["subject"]["reference"],
                "Patient/p1",
                "batch={batch} {id}"
            );
        }
        drop_database(&backend).await;
    }
}

#[tokio::test]
async fn mongodb_batch_writes_search_parameter_entries_match() {
    let Some(outcome) = compare(
        "batch_searchparam",
        Seed::Nothing,
        || {
            vec![
                post(patient("a"), None),
                post(
                    json!({"resourceType": "SearchParameter", "id": "sp-batch",
                        "url": "http://example.org/sp-batch", "name": "batchsp",
                        "status": "active", "description": "d", "code": "batchsp",
                        "base": ["Patient"], "type": "string",
                        "expression": "Patient.name.family"}),
                    None,
                ),
            ]
        },
        |_| {},
    )
    .await
    else {
        return;
    };
    assert!(
        matches!(outcome, Outcome::Ok(ref r) if r.len() == 2),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn mongodb_batch_writes_deferred_index_ledger_matches() {
    let Some(outcome) = compare(
        "batch_deferred",
        Seed::Nothing,
        || {
            (0..8)
                .map(|i| post(patient(&format!("d{i}")), None))
                .collect()
        },
        |config| config.transaction_bundle_deferred_index_entries = Some(5),
    )
    .await
    else {
        return;
    };
    assert!(
        matches!(outcome, Outcome::Ok(ref r) if r.len() == 8),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn mongodb_batch_writes_offloaded_search_writes_no_rows() {
    let Some(outcome) = compare(
        "batch_offloaded",
        Seed::Nothing,
        || {
            (0..3)
                .map(|i| post(patient(&format!("o{i}")), None))
                .collect()
        },
        |config| config.search_offloaded = true,
    )
    .await
    else {
        return;
    };
    assert!(
        matches!(outcome, Outcome::Ok(ref r) if r.len() == 3),
        "{outcome:?}"
    );
}

/// Drops the test database so a run leaves nothing behind.
async fn drop_database(backend: &MongoBackend) {
    let client = raw_test_client(&backend.config().connection_string)
        .await
        .unwrap();
    client
        .database(&backend.config().database_name)
        .drop()
        .await
        .unwrap();
}

/// Sorts object keys recursively so equal documents render identically.
fn canonical(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: std::collections::BTreeMap<String, Value> = map
                .into_iter()
                .map(|(key, value)| (key, canonical(value)))
                .collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(canonical).collect()),
        other => other,
    }
}

/// Per collection: the document count and an order-independent hash of every
/// document, ignoring `_id` and the write timestamps (`created_at`,
/// `last_updated`, `deleted_at`), which differ between any two runs. The
/// deferred-rows marker is reduced to its presence because it carries a
/// per-run ledger id.
async fn content_hashes(backend: &MongoBackend) -> Vec<(String, usize, u64)> {
    use futures::TryStreamExt;
    use std::hash::{Hash, Hasher};
    let client = raw_test_client(&backend.config().connection_string)
        .await
        .unwrap();
    let db = client.database(&backend.config().database_name);
    let mut out = Vec::new();
    for name in [
        "resources",
        "resource_history",
        "search_index",
        "search_index_contained",
    ] {
        let docs: Vec<Document> = db
            .collection::<Document>(name)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let mut rendered: Vec<String> = docs
            .into_iter()
            .map(|mut document| {
                for key in ["_id", "created_at", "last_updated", "deleted_at"] {
                    document.remove(key);
                }
                if document.contains_key("search_rows_pending") {
                    document.insert("search_rows_pending", true);
                }
                let value: Value = Bson::Document(document).into_relaxed_extjson();
                canonical(value).to_string()
            })
            .collect();
        rendered.sort();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        rendered.hash(&mut hasher);
        out.push((name.to_string(), rendered.len(), hasher.finish()));
    }
    out
}

fn observation(id: &str, subject: &str) -> Value {
    json!({"resourceType": "Observation", "id": id, "status": "final",
        "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}], "text": "hr"},
        "subject": {"reference": subject},
        "valueQuantity": {"value": 61.5, "unit": "bpm",
            "system": "http://unitsofmeasure.org", "code": "/min"},
        "effectiveDateTime": "2024-03-01T10:00:00Z"})
}

/// A mixed Bundle: several types, a `contained` resource, urn:uuid forward
/// and backward references, generated ids and a SearchParameter.
fn mixed_bundle() -> Vec<BundleEntry> {
    let mut entries = Vec::new();
    for i in 0..12 {
        let mut p = patient(&format!("pat{i}"));
        p["birthDate"] = json!(format!("19{:02}-01-15", 50 + i));
        p["gender"] = json!(if i % 2 == 0 { "male" } else { "female" });
        entries.push(post(p, Some(&format!("urn:uuid:pat-{i}"))));
    }
    for i in 0..12 {
        entries.push(post(
            observation(&format!("obs{i}"), &format!("urn:uuid:pat-{i}")),
            None,
        ));
    }
    // Forward reference: the Patient comes later in the Bundle.
    entries.push(post(
        observation("obs-fwd", "urn:uuid:pat-late"),
        Some("urn:uuid:obs-fwd"),
    ));
    entries.push(post(patient("late"), Some("urn:uuid:pat-late")));
    // Contained resource.
    let mut with_contained = observation("obs-contained", "#c1");
    with_contained["contained"] = json!([{"resourceType": "Patient", "id": "c1",
        "name": [{"family": "Contained"}]}]);
    entries.push(post(with_contained, None));
    entries.push(post(
        json!({"resourceType": "Encounter", "id": "enc1", "status": "finished",
            "class": {"system": "http://terminology.hl7.org/CodeSystem/v3-ActCode", "code": "AMB"},
            "subject": {"reference": "urn:uuid:pat-0"}}),
        None,
    ));
    entries.push(post(
        json!({"resourceType": "SearchParameter", "id": "sp-equiv",
            "url": "http://example.org/sp-equiv", "name": "equivsp",
            "status": "active", "description": "d", "code": "equivsp",
            "base": ["Patient"], "type": "string",
            "expression": "Patient.name.family"}),
        None,
    ));
    entries
}

#[tokio::test]
async fn mongodb_batch_writes_collections_hash_identically_after_the_same_bundles() {
    let tenant = create_tenant("batch-writes");
    let off = backend_with("equiv_off", false, |_| {}).await;
    let on = backend_with("equiv_on", true, |_| {}).await;
    let (Some(off), Some(on)) = (off, on) else {
        return;
    };
    // Two Bundles in a row: the second proves the existence check sees the
    // first Bundle's rows and that both modes keep accepting new ids.
    for backend in [&off, &on] {
        for round in 0..2 {
            let entries = if round == 0 {
                mixed_bundle()
            } else {
                (0..20)
                    .map(|i| post(patient(&format!("second{i}")), None))
                    .collect()
            };
            let outcome = run(backend, &tenant, entries).await;
            assert!(matches!(outcome, Outcome::Ok(_)), "{outcome:?}");
        }
    }
    let hashes_off = content_hashes(&off).await;
    let hashes_on = content_hashes(&on).await;
    // Positive control: the collections are not empty, so equality means something.
    assert!(hashes_off.iter().all(|(_, n, _)| *n > 0), "{hashes_off:?}");
    assert_eq!(hashes_off, hashes_on);
    drop_database(&off).await;
    drop_database(&on).await;
}

#[tokio::test]
async fn mongodb_batch_writes_deferred_collections_hash_identically() {
    let tenant = create_tenant("batch-writes");
    let configure = |config: &mut MongoBackendConfig| {
        config.transaction_bundle_deferred_index_entries = Some(5)
    };
    let off = backend_with("equiv_def_off", false, configure).await;
    let on = backend_with("equiv_def_on", true, configure).await;
    let (Some(off), Some(on)) = (off, on) else {
        return;
    };
    for backend in [&off, &on] {
        let outcome = run(backend, &tenant, mixed_bundle()).await;
        assert!(matches!(outcome, Outcome::Ok(_)), "{outcome:?}");
    }
    assert_eq!(content_hashes(&off).await, content_hashes(&on).await);
    drop_database(&off).await;
    drop_database(&on).await;
}

#[tokio::test]
async fn mongodb_batch_writes_failed_bundle_rolls_back_every_collection() {
    let tenant = create_tenant("batch-writes");
    for batch in [false, true] {
        let Some(backend) = backend_with(&format!("rollback_{batch}"), batch, |_| {}).await else {
            return;
        };
        seed(&backend, &tenant, Seed::Existing("x")).await;
        let before = content_hashes(&backend).await;
        // Entries 0 and 1 are valid; entry 2 collides with the stored "x".
        let outcome = run(
            &backend,
            &tenant,
            vec![
                post(patient("a"), None),
                post(observation("o", "Patient/a"), None),
                post(patient("x"), None),
                post(patient("c"), None),
            ],
        )
        .await;
        assert!(
            matches!(outcome, Outcome::BundleError(2, _)),
            "batch={batch}: {outcome:?}"
        );
        assert_eq!(
            content_hashes(&backend).await,
            before,
            "batch={batch}: the failed Bundle left writes behind"
        );
        for id in ["a", "c"] {
            assert!(
                backend
                    .read(&tenant, "Patient", id)
                    .await
                    .unwrap()
                    .is_none(),
                "batch={batch}: {id} survived the rollback"
            );
        }
        drop_database(&backend).await;
    }
}

#[tokio::test]
async fn mongodb_batch_writes_duplicate_of_stored_id_is_attributed_to_its_entry() {
    // The failing entry is the first, the middle and the last in turn.
    for failing in [0usize, 2, 4] {
        let Some(outcome) = compare(
            &format!("dupstored_{failing}"),
            Seed::Existing("x"),
            || {
                (0..5)
                    .map(|i| {
                        if i == failing {
                            post(patient("x"), None)
                        } else {
                            post(patient(&format!("n{i}")), None)
                        }
                    })
                    .collect()
            },
            |_| {},
        )
        .await
        else {
            return;
        };
        assert!(
            matches!(outcome, Outcome::BundleError(i, _) if i == failing),
            "failing={failing}: {outcome:?}"
        );
    }
}

#[tokio::test]
async fn mongodb_batch_writes_conditional_create_keeps_the_per_entry_path() {
    // A conditional create makes the Bundle ineligible for batching, so both
    // configurations must give the same outcome and the same documents,
    // whether the condition matches a stored resource or not.
    for (name, value) in [("condcreate_match", "x"), ("condcreate_nomatch", "nobody")] {
        let Some(outcome) = compare(
            name,
            Seed::Existing("x"),
            || {
                let mut conditional = post(patient(&format!("cc-{value}")), None);
                conditional.if_none_exist = Some(format!("identifier={MRN}|{value}"));
                vec![post(patient("plain"), None), conditional]
            },
            |_| {},
        )
        .await
        else {
            return;
        };
        assert!(
            matches!(outcome, Outcome::Ok(ref r) if r.len() == 2),
            "{name}: {outcome:?}"
        );
    }
}
