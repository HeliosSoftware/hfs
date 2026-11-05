//! `tuple_code` of token+quantity composites: the quantity row of a group
//! carries the group's codes, and a `code$value` search with one code is one
//! scan of `idx_search_composite_tuple_qty` instead of a pair validation,
//! once a `$reindex` has marked the tenant (or the type) complete. Child
//! module of the `mongodb_tests` root.

use super::*;
use futures::TryStreamExt;
use helios_persistence::search::{
    ReindexOperation, ReindexProgress, ReindexRequest, ReindexStatus,
};

use super::reindex_id_walk::wait_for_terminal;

const LOINC: &str = "http://loinc.org";
const UCUM: &str = "http://unitsofmeasure.org";
const LAYOUT: &str = "search_layout";

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

/// 40 heights (`h-00`..`h-39`, 140..179) and 20 weights with the same values
/// (`w-00`..`w-19`, 150..169). Returns the heights above 160 and every height.
async fn seed(backend: &MongoBackend, tenant: &TenantContext) -> (Vec<String>, Vec<String>) {
    let mut tall = Vec::new();
    let mut all_height = Vec::new();
    for n in 0..40 {
        let id = format!("h-{n:02}");
        let value = 140.0 + n as f64;
        backend
            .seed_create(
                tenant,
                "Observation",
                observation(&id, "8302-2", value),
                FhirVersion::default(),
            )
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

/// Removes `tuple_code` from the `tenant`'s rows, as a binary that predates it
/// would have written them; `resource_id` narrows it to one resource.
async fn unstamp(backend: &MongoBackend, tenant: &str, resource_id: Option<&str>) {
    let mut filter = doc! { "tenant_id": tenant };
    if let Some(id) = resource_id {
        filter.insert("resource_id", id);
    }
    raw_db(backend)
        .await
        .collection::<Document>("search_index")
        .update_many(filter, doc! { "$unset": { "tuple_code": "" } })
        .await
        .unwrap();
}

async fn markers(backend: &MongoBackend) -> Vec<String> {
    let mut ids: Vec<String> = raw_db(backend)
        .await
        .collection::<Document>(LAYOUT)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
        .iter()
        .map(|marker| marker.get_str("_id").unwrap().to_string())
        .collect();
    ids.sort();
    ids
}

async fn set_recorded_generation(backend: &MongoBackend, generation: i32) {
    raw_db(backend)
        .await
        .collection::<Document>("schema_version")
        .update_one(
            doc! { "_id": "schema_version" },
            doc! { "$set": { "search_indexes.generation": generation } },
        )
        .await
        .unwrap();
}

/// Whether `tenant`'s `code-value-quantity` searches take the tuple path: a
/// copy of one of its quantity rows, stamped with a code no token row of its
/// group has, is found by the tuple path and never by pair validation. The
/// copy is removed again.
async fn tuple_path_used(backend: &MongoBackend, tenant: &TenantContext) -> bool {
    let db = raw_db(backend).await;
    let rows = db.collection::<Document>("search_index");
    let mut probe = rows
        .find_one(doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "param_name": "code-value-quantity",
            "value_quantity_value": { "$exists": true },
        })
        .await
        .unwrap()
        .expect("a code-value-quantity quantity row");
    probe.remove("_id");
    probe.insert("composite_group", 9_999_i32);
    probe.insert("value_quantity_value", 123_456.0);
    probe.insert("tuple_code", vec![Bson::from("c:tuple-probe")]);
    let inserted = rows.insert_one(&probe).await.unwrap().inserted_id;
    let hit = !found(backend, tenant, "tuple-probe$gt123455")
        .await
        .is_empty();
    rows.delete_one(doc! { "_id": inserted }).await.unwrap();
    hit
}

async fn reindex(
    backend: &Arc<MongoBackend>,
    tenant: &TenantContext,
    request: ReindexRequest,
) -> ReindexProgress {
    let op = ReindexOperation::new(backend.clone(), backend.tenant_registries().clone());
    let job = op.start(tenant.clone(), request, None).await.unwrap();
    let progress = wait_for_terminal(&op, &job).await;
    assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
    progress
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
        &vec![
            Bson::from("c:8302-2"),
            Bson::from(format!("s{}:{LOINC}|8302-2", LOINC.len()))
        ]
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
    let indexes = db
        .collection::<Document>("search_index")
        .list_index_names()
        .await
        .unwrap();
    assert!(
        indexes
            .iter()
            .any(|name| name == "idx_search_composite_tuple_qty"),
        "{indexes:?}"
    );
}

/// I2: a database that is empty at boot gets no all-tenant marker, so a row
/// written without `tuple_code` after the boot (a restored dump, an older
/// replica, a direct write) is still found.
#[tokio::test]
async fn mongodb_a_fresh_database_has_no_all_tenant_marker_and_late_rows_stay_visible() {
    let Some(backend) = create_backend_with_full_registry("composite_tuple_fresh").await else {
        eprintln!("Skipping composite_tuple (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("composite-tuple-fresh");
    let (tall, _) = seed(&backend, &tenant).await;
    assert_eq!(markers(&backend).await, Vec::<String>::new());
    // A row restored after the boot, without `tuple_code`.
    unstamp(&backend, "composite-tuple-fresh", Some("h-30")).await;
    assert_eq!(
        found(&backend, &tenant, "8302-2$gt160").await,
        sorted(tall.clone())
    );
    assert!(!tuple_path_used(&backend, &tenant).await);
}

/// T1/I3: a real `$reindex` stamps rows written before `tuple_code` and marks
/// what it rebuilt in full; a range run marks nothing, a run over other types
/// does not mark Observation, and `bulkIndexRebuild` marks the tenant. The
/// marker is in place when the job reads `Completed`.
#[tokio::test]
async fn mongodb_a_real_reindex_stamps_the_rows_and_enables_the_tuple_path() {
    let Some(backend) = create_backend_with_full_registry("composite_tuple_reindex").await else {
        eprintln!("Skipping composite_tuple (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let tenant = create_tenant("composite-tuple-reindex");
    let (tall, _) = seed(&backend, &tenant).await;
    unstamp(&backend, "composite-tuple-reindex", None).await;
    assert_eq!(
        found(&backend, &tenant, "8302-2$gt160").await,
        sorted(tall.clone())
    );
    assert!(!tuple_path_used(&backend, &tenant).await);

    // An id range rebuilds part of the type: no marker.
    reindex(
        &backend,
        &tenant,
        ReindexRequest {
            id_start: Some("h-00".into()),
            id_end: Some("h-20".into()),
            ..ReindexRequest::for_types(["Observation"])
        },
    )
    .await;
    assert_eq!(markers(&backend).await, Vec::<String>::new());

    // Another type: its own marker, not Observation's.
    reindex(&backend, &tenant, ReindexRequest::for_types(["Patient"])).await;
    assert_eq!(
        markers(&backend).await,
        vec!["composite-tuple-reindex|Patient|composite_tuple_code_v2".to_string()]
    );
    assert!(!tuple_path_used(&backend, &tenant).await);

    // Observation in full: stamped, marked, and the tuple path answers.
    reindex(
        &backend,
        &tenant,
        ReindexRequest::for_types(["Observation"]),
    )
    .await;
    assert!(
        markers(&backend)
            .await
            .contains(&"composite-tuple-reindex|Observation|composite_tuple_code_v2".to_string())
    );
    let unstamped = raw_db(&backend)
        .await
        .collection::<Document>("search_index")
        .count_documents(doc! {
            "tenant_id": "composite-tuple-reindex",
            "param_name": "code-value-quantity",
            "value_quantity_value": { "$exists": true },
            "tuple_code": { "$exists": false },
        })
        .await
        .unwrap();
    assert_eq!(unstamped, 0);
    assert!(tuple_path_used(&backend, &tenant).await);
    assert_eq!(
        found(&backend, &tenant, "8302-2$gt160").await,
        sorted(tall.clone())
    );

    // `bulkIndexRebuild` over the whole tenant marks the tenant.
    let bulk = create_tenant("composite-tuple-bulk");
    seed(&backend, &bulk).await;
    unstamp(&backend, "composite-tuple-bulk", None).await;
    reindex(
        &backend,
        &bulk,
        ReindexRequest::default().with_bulk_index_rebuild(true),
    )
    .await;
    assert!(
        markers(&backend)
            .await
            .contains(&"composite-tuple-bulk|composite_tuple_code_v2".to_string())
    );
    assert!(tuple_path_used(&backend, &bulk).await);
    assert_eq!(
        found(&backend, &bulk, "8302-2$gt160").await,
        sorted(tall.clone())
    );
}

/// I1: after a rollback (an older binary records its own, lower generation and
/// writes rows without `tuple_code`) a running instance stops trusting the
/// markers, and the next boot deletes them, so the rollback's rows are found.
#[tokio::test]
async fn mongodb_a_rollback_invalidates_the_markers() {
    let Some(backend) = create_backend_with_full_registry("composite_tuple_rollback").await else {
        eprintln!("Skipping composite_tuple (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let first = create_tenant("composite-tuple-rollback-a");
    let second = create_tenant("composite-tuple-rollback-b");
    let (tall, _) = seed(&backend, &first).await;
    seed(&backend, &second).await;
    reindex(&backend, &first, ReindexRequest::default()).await;
    reindex(&backend, &second, ReindexRequest::default()).await;
    assert!(tuple_path_used(&backend, &first).await);

    // The rollback binary boots (records generation 4) and writes h-30 again.
    set_recorded_generation(&backend, 4).await;
    unstamp(&backend, "composite-tuple-rollback-a", Some("h-30")).await;
    unstamp(&backend, "composite-tuple-rollback-b", Some("h-30")).await;

    // This instance: a read not served from its cache refuses the marker.
    assert!(!tuple_path_used(&backend, &second).await);
    assert_eq!(
        found(&backend, &second, "8302-2$gt160").await,
        sorted(tall.clone())
    );

    // The return to this binary: its boot deletes every marker before its
    // builder records the current generation again.
    let returned = build_backend(backend.config().clone())
        .await
        .expect("second boot");
    assert_eq!(markers(&returned).await, Vec::<String>::new());
    let generation = raw_db(&returned)
        .await
        .collection::<Document>("schema_version")
        .find_one(doc! { "_id": "schema_version" })
        .await
        .unwrap()
        .unwrap()
        .get_document("search_indexes")
        .unwrap()
        .get_i32("generation")
        .unwrap();
    assert_eq!(generation, 5);
    assert!(!tuple_path_used(&returned, &first).await);
    assert_eq!(
        found(&returned, &first, "8302-2$gt160").await,
        sorted(tall.clone())
    );
}
