//! `tuple_code` of token+quantity composites: the quantity row of a group
//! carries the group's codes, and a `code$value` search with one code is one
//! scan of `idx_search_composite_tuple_qty` instead of a pair validation,
//! once a `$reindex` has marked the tenant (or the type) complete. Child
//! module of the `mongodb_tests` root.

use super::*;
use futures::TryStreamExt;
use helios_persistence::search::{
    ReindexCoverage, ReindexOperation, ReindexProgress, ReindexRequest, ReindexStatus,
    ReindexTarget,
};

use super::composite_plan::{command_of, profiled};
use super::reindex_id_walk::wait_for_terminal;

const LOINC: &str = "http://loinc.org";
const UCUM: &str = "http://unitsofmeasure.org";
const LOCAL: &str = "http://local";
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
    // A row restored after the boot, without `tuple_code`.
    unstamp(&backend, "composite-tuple-fresh", Some("h-30")).await;
    assert_eq!(
        found(&backend, &tenant, "8302-2$gt160").await,
        sorted(tall.clone())
    );
    assert!(!tuple_path_used(&backend, &tenant).await);
    assert_eq!(markers(&backend).await, Vec::<String>::new());
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
/// markers, and the next boot's builder deletes them, so the rollback's rows
/// are found.
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

    // The return to this binary: its builder deletes every marker right
    // before it records the current generation again.
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

/// N5: a completed `$reindex` marks nothing over a quantity row without
/// `tuple_code` whose group has a coded token row (an older instance wrote it
/// while the run went on), per tenant or per type; a type without
/// token+quantity composites is still marked, and so is a tenant whose only
/// unstamped quantity rows belong to groups with no code, which are never
/// stamped.
#[tokio::test]
async fn mongodb_a_quantity_row_without_tuple_code_blocks_the_marker() {
    let Some(backend) = create_backend_with_full_registry("composite_tuple_verify").await else {
        eprintln!("Skipping composite_tuple (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let late = create_tenant("composite-tuple-verify-late");
    seed(&backend, &late).await;
    // Written during the run by an instance that predates `tuple_code`.
    unstamp(&backend, "composite-tuple-verify-late", Some("h-30")).await;

    backend
        .reindex_run_completed(&late, ReindexCoverage::Tenant)
        .await;
    assert_eq!(markers(&backend).await, Vec::<String>::new());
    let types = ["Observation".to_string(), "Patient".to_string()];
    backend
        .reindex_run_completed(&late, ReindexCoverage::Types(&types))
        .await;
    assert_eq!(
        markers(&backend).await,
        vec!["composite-tuple-verify-late|Patient|composite_tuple_code_v2".to_string()]
    );
    assert!(!tuple_path_used(&backend, &late).await);

    // A group whose token has no code: its quantity row is never stamped.
    let codeless = create_tenant("composite-tuple-verify-codeless");
    seed(&backend, &codeless).await;
    backend
        .seed_create(
            &codeless,
            "Observation",
            json!({
                "resourceType": "Observation", "id": "no-code", "status": "final",
                "code": {"coding": [{"system": LOINC, "display": "Body height"}]},
                "valueQuantity": {"value": 170.0, "unit": "cm", "system": UCUM},
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let unstamped = raw_db(&backend)
        .await
        .collection::<Document>("search_index")
        .count_documents(doc! {
            "tenant_id": "composite-tuple-verify-codeless",
            "param_name": "code-value-quantity",
            "value_quantity_value": { "$exists": true },
            "tuple_code": { "$exists": false },
        })
        .await
        .unwrap();
    backend
        .reindex_run_completed(&codeless, ReindexCoverage::Tenant)
        .await;
    assert!(
        markers(&backend)
            .await
            .contains(&"composite-tuple-verify-codeless|composite_tuple_code_v2".to_string()),
        "codeless group ({unstamped} unstamped quantity rows) blocked the marker"
    );
    assert!(tuple_path_used(&backend, &codeless).await);
}

/// Creates, updates and deletes of this binary while a `$reindex` runs write
/// stamped rows, so they neither block the marker nor go missing behind it.
/// The writes start right after the job does; the job pages 5 resources at a
/// time, but nothing forces an overlap, so this checks the end state.
#[tokio::test]
async fn mongodb_writes_during_a_reindex_are_stamped_and_keep_the_marker() {
    let Some(backend) = create_backend_with_full_registry("composite_tuple_concurrent").await
    else {
        eprintln!("Skipping composite_tuple (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let tenant = create_tenant("composite-tuple-concurrent");
    let (tall, _) = seed(&backend, &tenant).await;
    unstamp(&backend, "composite-tuple-concurrent", None).await;

    let op = ReindexOperation::new(backend.clone(), backend.tenant_registries().clone());
    let job = op
        .start(
            tenant.clone(),
            ReindexRequest::for_types(["Observation"]).with_batch_size(5),
            None,
        )
        .await
        .unwrap();
    let mut expected: Vec<String> = tall.into_iter().filter(|id| id != "h-35").collect();
    for n in 0..10 {
        let id = format!("n-{n:02}");
        backend
            .create(
                &tenant,
                "Observation",
                observation(&id, "8302-2", 200.0 + n as f64),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        expected.push(id);
    }
    let current = backend
        .read(&tenant, "Observation", "h-05")
        .await
        .unwrap()
        .unwrap();
    backend
        .update(&tenant, &current, observation("h-05", "8302-2", 300.0))
        .await
        .unwrap();
    expected.push("h-05".to_string());
    backend
        .delete(&tenant, "Observation", "h-35")
        .await
        .unwrap();
    let progress = wait_for_terminal(&op, &job).await;
    assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");

    assert_eq!(
        markers(&backend).await,
        vec!["composite-tuple-concurrent|Observation|composite_tuple_code_v2".to_string()]
    );
    let unstamped = raw_db(&backend)
        .await
        .collection::<Document>("search_index")
        .count_documents(doc! {
            "tenant_id": "composite-tuple-concurrent",
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
        sorted(expected)
    );
}

/// A tenant purge that ran (on any instance) after a `$reindex` rebuilt the
/// tenant but before its hook wrote the markers leaves no marker: the hook
/// finds no resource of the tenant after writing and removes what it wrote.
/// Without that, a tenant id reused later would start out marked. The race
/// with a purge in the middle of the run is `tenant_purge_races::
/// a_tenant_purge_racing_a_reindex_job_leaves_no_rows_and_the_job_completes`.
#[tokio::test]
async fn mongodb_a_marker_written_after_a_tenant_purge_is_removed() {
    let Some(backend) = create_backend_with_full_registry("composite_tuple_purged").await else {
        eprintln!("Skipping composite_tuple (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let purged = create_tenant("composite-tuple-purged");
    let kept = create_tenant("composite-tuple-purge-kept");
    seed(&backend, &purged).await;
    seed(&backend, &kept).await;
    backend
        .purge_tenant_data("composite-tuple-purged")
        .await
        .unwrap();

    backend
        .reindex_run_completed(&purged, ReindexCoverage::Tenant)
        .await;
    let types = ["Observation".to_string(), "Patient".to_string()];
    backend
        .reindex_run_completed(&purged, ReindexCoverage::Types(&types))
        .await;
    // The same hook keeps the markers of a tenant that still has resources.
    backend
        .reindex_run_completed(&kept, ReindexCoverage::Tenant)
        .await;
    assert_eq!(
        markers(&backend).await,
        vec!["composite-tuple-purge-kept|composite_tuple_code_v2".to_string()]
    );
}

/// The planner's tuple read, profiled: it scans `idx_search_composite_tuple_qty`
/// for a bare code and for `system|code` (`$in` over the partial index).
#[tokio::test]
async fn mongodb_the_tuple_path_scans_the_tuple_index() {
    let Some(backend) = create_backend_with_full_registry("composite_tuple_explain").await else {
        eprintln!("Skipping composite_tuple (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let tenant = create_tenant("composite-tuple-explain");
    let (tall, _) = seed(&backend, &tenant).await;
    reindex(&backend, &tenant, ReindexRequest::default()).await;
    let db = raw_db(&backend).await;
    for value in ["8302-2$gt160", "http://loinc.org|8302-2$gt160"] {
        let (ids, operations) = profiled(&db, found(&backend, &tenant, value)).await;
        assert_eq!(ids, sorted(tall.clone()), "{value}");
        let tuple_reads: Vec<&Document> = operations
            .iter()
            .filter(|operation| mentions_tuple_code(&Bson::Document(command_of(operation).clone())))
            .collect();
        assert!(
            !tuple_reads.is_empty(),
            "{value}: no read filtered on tuple_code"
        );
        for read in tuple_reads {
            let plan = read.get_str("planSummary").unwrap_or_default();
            assert!(
                plan.contains("tuple_code"),
                "{value}: the tuple read used {plan}: {read}"
            );
            let docs = read
                .get_i64("docsExamined")
                .or_else(|_| read.get_i32("docsExamined").map(i64::from))
                .unwrap_or(-1);
            let keys = read
                .get_i64("keysExamined")
                .or_else(|_| read.get_i32("keysExamined").map(i64::from))
                .unwrap_or(-1);
            eprintln!("{value}: plan {plan}, keysExamined {keys}, docsExamined {docs}");
        }
    }
}

fn mentions_tuple_code(value: &Bson) -> bool {
    match value {
        Bson::Document(document) => document
            .iter()
            .any(|(key, value)| key == "tuple_code" || mentions_tuple_code(value)),
        Bson::Array(values) => values.iter().any(mentions_tuple_code),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Oracle: the tuple path against pair validation on the same corpus.
// ---------------------------------------------------------------------------

/// 24 Observations over three patients: heights with a second, local coding
/// (and, on `o00`, a coding without system and one whose code holds `|`),
/// weights in kg and lb, and blood-pressure panels whose two components carry
/// values far apart, so a cross-group match would show. Two are deleted.
async fn seed_corpus(backend: &MongoBackend, tenant: &TenantContext) {
    for i in 0..24_u32 {
        let id = format!("o{i:02}");
        let mut resource = json!({
            "resourceType": "Observation", "id": id,
            "status": if i % 4 == 3 { "amended" } else { "final" },
            "subject": {"reference": format!("Patient/p{}", i % 3)},
            "effectiveDateTime": format!("2020-01-{:02}", i + 1),
        });
        match i % 3 {
            0 => {
                let mut codings = vec![
                    json!({"system": LOINC, "code": "8302-2"}),
                    json!({"system": LOCAL, "code": "L-height"}),
                ];
                if i == 0 {
                    codings.push(json!({"code": "nosys"}));
                    codings.push(json!({"code": "a|b"}));
                }
                resource["code"] = json!({ "coding": codings });
                resource["valueQuantity"] = json!({
                    "value": 150 + i * 2, "unit": "cm", "system": UCUM, "code": "cm",
                });
            }
            1 => {
                resource["code"] = json!({"coding": [{"system": LOINC, "code": "29463-7"}]});
                resource["valueQuantity"] = if i % 2 == 1 {
                    json!({"value": 50 + i, "unit": "lb", "system": UCUM, "code": "[lb_av]"})
                } else {
                    json!({"value": 50 + i, "unit": "kg", "system": UCUM, "code": "kg"})
                };
            }
            _ => {
                resource["code"] = json!({"coding": [{"system": LOINC, "code": "85354-9"}]});
                resource["component"] = json!([
                    {
                        "code": {"coding": [{"system": LOINC, "code": "8480-6"}]},
                        "valueQuantity": {"value": 100 + i, "unit": "mmHg", "system": UCUM, "code": "mm[Hg]"},
                    },
                    {
                        "code": {"coding": [{"system": LOINC, "code": "8462-4"}]},
                        "valueQuantity": {"value": 60 + i, "unit": "mmHg", "system": UCUM, "code": "mm[Hg]"},
                    },
                ]);
            }
        }
        backend
            .seed_create(tenant, "Observation", resource, FhirVersion::default())
            .await
            .unwrap();
    }
    for deleted in ["o06", "o17"] {
        backend
            .delete(tenant, "Observation", deleted)
            .await
            .unwrap();
    }
}

fn composite(name: &str, token: &str, quantity: &str, value: &str) -> SearchParameter {
    SearchParameter {
        name: name.to_string(),
        param_type: SearchParamType::Composite,
        modifier: None,
        values: vec![SearchValue::eq(value)],
        chain: vec![],
        components: vec![
            CompositeSearchComponent {
                param_type: SearchParamType::Token,
                param_name: token.to_string(),
            },
            CompositeSearchComponent {
                param_type: SearchParamType::Quantity,
                param_name: quantity.to_string(),
            },
        ],
    }
}

fn cvq(value: &str) -> SearchParameter {
    composite("code-value-quantity", "code", "value-quantity", value)
}

fn component(value: &str) -> SearchParameter {
    composite(
        "component-code-value-quantity",
        "component-code",
        "component-value-quantity",
        value,
    )
}

fn combo(value: &str) -> SearchParameter {
    composite(
        "combo-code-value-quantity",
        "combo-code",
        "combo-value-quantity",
        value,
    )
}

fn plain(name: &str, param_type: SearchParamType, value: &str) -> SearchParameter {
    SearchParameter {
        name: name.to_string(),
        param_type,
        modifier: None,
        values: vec![SearchValue::parse_for_type(value, param_type)],
        chain: vec![],
        components: vec![],
    }
}

/// Every page (3 per page) of `query`, its accurate total, and its count.
async fn answer(
    backend: &MongoBackend,
    tenant: &TenantContext,
    query: &SearchQuery,
) -> (Vec<Vec<String>>, Option<u64>, u64) {
    let mut pages = Vec::new();
    let mut first = query.clone().with_count(3);
    first.total = Some(TotalMode::Accurate);
    let mut next = first;
    let mut total = None;
    loop {
        let result = backend.search(tenant, &next).await.unwrap();
        if pages.is_empty() {
            total = result.total;
        }
        pages.push(
            result
                .resources
                .items
                .iter()
                .map(|r| r.id().to_owned())
                .collect::<Vec<_>>(),
        );
        match result.resources.page_info.next_cursor {
            Some(cursor) if pages.len() < 20 => {
                next = query.clone().with_count(3).with_cursor(cursor);
            }
            _ => break,
        }
    }
    let count = backend.search_count(tenant, query).await.unwrap();
    (pages, total, count)
}

/// The tuple path (a tenant marked by a real `$reindex`) and pair validation
/// (an identical tenant without a marker) give the same pages, totals and
/// counts for every shape the planner sends to `tuple_code`: the three
/// token+quantity composites, several codings and components per resource,
/// every quantity prefix, units, codes without system or holding `|`, deleted
/// resources, `_sort`, and a composite combined with other parameters.
#[tokio::test]
async fn mongodb_tuple_path_answers_like_pair_validation() {
    let Some(backend) = create_backend_with_full_registry("composite_tuple_oracle").await else {
        eprintln!("Skipping composite_tuple (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let fast = create_tenant("composite-tuple-fast");
    let slow = create_tenant("composite-tuple-slow");
    seed_corpus(&backend, &fast).await;
    seed_corpus(&backend, &slow).await;
    reindex(&backend, &fast, ReindexRequest::default()).await;
    assert!(tuple_path_used(&backend, &fast).await);
    assert!(!tuple_path_used(&backend, &slow).await);

    let ucum_cm = format!("8302-2$gt100|{UCUM}|cm");
    let ucum_kg = format!("29463-7$gt55|{UCUM}|kg");
    let ucum_lb = format!("29463-7$gt0|{UCUM}|[lb_av]");
    let mut queries: Vec<(String, SearchQuery)> = [
        "8302-2$gt160",
        "http://loinc.org|8302-2$ge150",
        "http://local|L-height$lt170",
        "8302-2$ap170",
        "8302-2$eq170",
        "8302-2$170",
        "8302-2$le166",
        "8302-2$sa160",
        "8302-2$eb180",
        "8302-2$gt100||cm",
        ucum_cm.as_str(),
        ucum_kg.as_str(),
        ucum_lb.as_str(),
        "nosys$gt0",
        "http://x|nosys$gt0",
        "a|b$gt0",
        "8480-6$gt0",
        "nope$gt0",
    ]
    .into_iter()
    .map(|value| {
        (
            format!("code-value-quantity={value}"),
            SearchQuery::new("Observation").with_parameter(cvq(value)),
        )
    })
    .collect();
    for value in [
        "8462-4$gt80",
        "8480-6$gt110",
        "http://loinc.org|8462-4$lt70",
        "8462-4$gt100",
    ] {
        queries.push((
            format!("component-code-value-quantity={value}"),
            SearchQuery::new("Observation").with_parameter(component(value)),
        ));
    }
    for value in [
        "8480-6$gt110",
        "85354-9$gt0",
        "8302-2$gt160",
        "29463-7$lt60",
    ] {
        queries.push((
            format!("combo-code-value-quantity={value}"),
            SearchQuery::new("Observation").with_parameter(combo(value)),
        ));
    }
    queries.extend([
        (
            "code-value-quantity + status".to_string(),
            SearchQuery::new("Observation")
                .with_parameter(cvq("8302-2$gt150"))
                .with_parameter(plain("status", SearchParamType::Token, "final")),
        ),
        (
            "code-value-quantity + subject".to_string(),
            SearchQuery::new("Observation")
                .with_parameter(cvq("8302-2$gt150"))
                .with_parameter(plain("subject", SearchParamType::Reference, "Patient/p0")),
        ),
        (
            "code-value-quantity + date".to_string(),
            SearchQuery::new("Observation")
                .with_parameter(cvq("http://loinc.org|8302-2$gt100"))
                .with_parameter(plain("date", SearchParamType::Date, "ge2020-01-10")),
        ),
        (
            "component + subject".to_string(),
            SearchQuery::new("Observation")
                .with_parameter(component("8480-6$gt105"))
                .with_parameter(plain("subject", SearchParamType::Reference, "Patient/p2")),
        ),
        (
            "code-value-quantity + combo".to_string(),
            SearchQuery::new("Observation")
                .with_parameter(cvq("8302-2$gt150"))
                .with_parameter(combo("8302-2$lt190")),
        ),
        (
            "code-value-quantity _sort=-_id".to_string(),
            SearchQuery::new("Observation")
                .with_parameter(cvq("8302-2$gt0"))
                .with_sort(SortDirective::parse("-_id")),
        ),
        (
            "combo _sort=_id".to_string(),
            SearchQuery::new("Observation")
                .with_parameter(combo("8480-6$gt0"))
                .with_sort(SortDirective::parse("_id")),
        ),
    ]);

    let mut non_empty = 0;
    for (label, query) in &queries {
        let tuple = answer(&backend, &fast, query).await;
        let pairs = answer(&backend, &slow, query).await;
        assert_eq!(tuple, pairs, "{label}");
        if tuple.2 > 0 {
            non_empty += 1;
        }
    }
    // Not vacuous: most shapes match something, and the cross-group and
    // wrong-system shapes match nothing.
    assert!(non_empty >= 20, "{non_empty} of {} matched", queries.len());
    for empty in [
        "code-value-quantity=http://x|nosys$gt0",
        "code-value-quantity=a|b$gt0",
        "code-value-quantity=8480-6$gt0",
        "component-code-value-quantity=8462-4$gt100",
    ] {
        let query = &queries.iter().find(|(label, _)| label == empty).unwrap().1;
        assert_eq!(answer(&backend, &fast, query).await.2, 0, "{empty}");
    }
}
