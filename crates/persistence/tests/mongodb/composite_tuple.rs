//! `tuple_code` of token+quantity composites: the quantity row of a group
//! carries the group's codes, and a `code$value` search with one code is one
//! covered scan of `idx_search_composite_tuple_qty` instead of a pair
//! validation. Child module of the `mongodb_tests` root.

use super::*;
use futures::TryStreamExt;

const LOINC: &str = "http://loinc.org";
const UCUM: &str = "http://unitsofmeasure.org";

fn observation(id: &str, code: &str, value: f64) -> serde_json::Value {
    json!({
        "resourceType": "Observation", "id": id, "status": "final",
        "code": {"coding": [{"system": LOINC, "code": code}]},
        "valueQuantity": {"value": value, "unit": "cm", "system": UCUM},
    })
}

async fn raw_db(backend: &MongoBackend) -> mongodb::Database {
    raw_test_client(&backend.config().connection_string)
        .await
        .unwrap()
        .database(&backend.config().database_name)
}

async fn seed(backend: &MongoBackend, tenant: &TenantContext) -> (Vec<String>, Vec<String>) {
    let mut tall = Vec::new();
    let mut all_height = Vec::new();
    for n in 0..40 {
        let id = format!("h-{n:02}");
        let value = 140.0 + n as f64;
        backend
            .seed_create(tenant, "Observation", observation(&id, "8302-2", value), FhirVersion::default())
            .await
            .unwrap();
        if value > 160.0 {
            tall.push(id.clone());
        }
        all_height.push(id);
    }
    // Same values under another code: the value alone must not match.
    for n in 0..20 {
        backend
            .seed_create(
                tenant,
                "Observation",
                observation(&format!("w-{n:02}"), "29463-7", 150.0 + n as f64),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    (tall, all_height)
}

async fn found(backend: &MongoBackend, tenant: &TenantContext, value: &str) -> Vec<String> {
    let mut query = code_value_quantity_query(value).with_count(100);
    query.total = Some(TotalMode::None);
    let mut ids: Vec<String> = backend
        .search(tenant, &query)
        .await
        .unwrap()
        .resources
        .items
        .iter()
        .map(|r| r.id().to_owned())
        .collect();
    ids.sort();
    ids
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

#[tokio::test]
async fn mongodb_quantity_rows_carry_the_group_codes_and_the_tuple_index_exists() {
    let Some(backend) = create_backend_with_full_registry("composite_tuple_rows").await else {
        eprintln!("Skipping composite_tuple (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("composite-tuple-rows");
    seed(&backend, &tenant).await;
    let db = raw_db(&backend).await;
    let rows = db
        .collection::<Document>("search_index")
        .find(doc! {
            "tenant_id": "composite-tuple-rows", "resource_id": "h-00",
            "param_name": "code-value-quantity", "value_quantity_value": { "$exists": true },
        })
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get_array("tuple_code").unwrap(),
        &vec![Bson::from("8302-2"), Bson::from(format!("{LOINC}|8302-2"))]
    );
    // No extra rows: the token row of the group has no tuple_code.
    let token_rows = db
        .collection::<Document>("search_index")
        .count_documents(doc! {
            "tenant_id": "composite-tuple-rows", "tuple_code": { "$exists": true },
            "value_token_code": { "$exists": true },
        })
        .await
        .unwrap();
    assert_eq!(token_rows, 0);
}

/// The tuple path (a fresh database carries the all-tenant marker) and the
/// pair-validation path (no marker) answer the same pages.
#[tokio::test]
async fn mongodb_tuple_path_matches_pair_validation() {
    let Some(backend) = create_backend_with_full_registry("composite_tuple_equiv").await else {
        eprintln!("Skipping composite_tuple (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let fast = create_tenant("composite-tuple-fast");
    let slow = create_tenant("composite-tuple-slow");
    let (tall, all_height) = seed(&backend, &fast).await;
    seed(&backend, &slow).await;
    let db = raw_db(&backend).await;

    let cases: Vec<(&str, Vec<String>)> = vec![
        ("8302-2$gt160", tall.clone()),
        ("http://loinc.org|8302-2$gt160", tall.clone()),
        ("8302-2$ge140", all_height.clone()),
        ("8302-2$lt141", vec!["h-00".to_string()]),
        ("8302-2$gt999", vec![]),
        ("nope$gt0", vec![]),
        // The same values under another code: only that code's rows match.
        (
            "29463-7$gt160",
            (11..20).map(|n| format!("w-{n:02}")).collect(),
        ),
        ("http://other|8302-2$gt0", vec![]),
    ];
    for (value, expected) in &cases {
        assert_eq!(found(&backend, &fast, value).await, sorted(expected.clone()), "fast {value}");
    }

    // The pair path: this tenant has no marker once the all-tenant one is gone
    // (the cache of `slow` is still empty), and its rows lack `tuple_code`.
    db.collection::<Document>("search_layout").delete_many(doc! {}).await.unwrap();
    db.collection::<Document>("search_index")
        .update_many(
            doc! { "tenant_id": "composite-tuple-slow" },
            doc! { "$unset": { "tuple_code": "" } },
        )
        .await
        .unwrap();
    for (value, expected) in &cases {
        assert_eq!(found(&backend, &slow, value).await, sorted(expected.clone()), "pair {value}");
    }
}

/// A full `$reindex` of a tenant whose rows predate `tuple_code` stamps them
/// and records the marker; until then the pair path answers.
#[tokio::test]
async fn mongodb_full_reindex_stamps_tuple_codes_and_enables_the_marker() {
    let Some(backend) = create_backend_with_full_registry("composite_tuple_reindex").await else {
        eprintln!("Skipping composite_tuple (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("composite-tuple-reindex");
    let (tall, _) = seed(&backend, &tenant).await;
    let db = raw_db(&backend).await;
    db.collection::<Document>("search_layout").delete_many(doc! {}).await.unwrap();
    db.collection::<Document>("search_index")
        .update_many(doc! {}, doc! { "$unset": { "tuple_code": "" } })
        .await
        .unwrap();
    assert_eq!(found(&backend, &tenant, "8302-2$gt160").await, sorted(tall.clone()));

    // What the reindex hook does on a clean, unscoped completion.
    helios_persistence::search::ReindexTarget::reindex_run_completed(&backend, &tenant, false, true).await;
    assert!(db.collection::<Document>("search_layout").count_documents(doc! {}).await.unwrap() == 0,
        "a scoped run records nothing");
    helios_persistence::search::ReindexTarget::reindex_run_completed(&backend, &tenant, true, false).await;
    assert!(db.collection::<Document>("search_layout").count_documents(doc! {}).await.unwrap() == 0,
        "a run with errors records nothing");
    helios_persistence::search::ReindexTarget::reindex_run_completed(&backend, &tenant, true, true).await;
    assert_eq!(
        db.collection::<Document>("search_layout").count_documents(doc! {}).await.unwrap(),
        1
    );
}
