//! Missing-parameter metadata through the actual MongoDB write and search paths.

use super::*;
use helios_persistence::core::{BulkProcessingOptions, BulkSubmitProvider, NdjsonEntry};
use helios_persistence::search::ReindexTarget;
use serde_json::Value;

struct TypedParameter {
    name: &'static str,
    kind: SearchParamType,
    fhir_type: &'static str,
    extension_field: &'static str,
    index_field: &'static str,
}

const PARAMETERS: [TypedParameter; 7] = [
    TypedParameter {
        name: "missing-string",
        kind: SearchParamType::String,
        fhir_type: "string",
        extension_field: "valueString",
        index_field: "value_string",
    },
    TypedParameter {
        name: "missing-token",
        kind: SearchParamType::Token,
        fhir_type: "token",
        extension_field: "valueCode",
        index_field: "value_token_code",
    },
    TypedParameter {
        name: "missing-date",
        kind: SearchParamType::Date,
        fhir_type: "date",
        extension_field: "valueDateTime",
        index_field: "value_date",
    },
    TypedParameter {
        name: "missing-number",
        kind: SearchParamType::Number,
        fhir_type: "number",
        extension_field: "valueDecimal",
        index_field: "value_number",
    },
    TypedParameter {
        name: "missing-quantity",
        kind: SearchParamType::Quantity,
        fhir_type: "quantity",
        extension_field: "valueQuantity",
        index_field: "value_quantity_value",
    },
    TypedParameter {
        name: "missing-reference",
        kind: SearchParamType::Reference,
        fhir_type: "reference",
        extension_field: "valueReference",
        index_field: "value_reference",
    },
    TypedParameter {
        name: "missing-uri",
        kind: SearchParamType::Uri,
        fhir_type: "uri",
        extension_field: "valueUri",
        index_field: "value_uri",
    },
];

async fn register_parameters(backend: &MongoBackend, tenant: &TenantContext) {
    for parameter in &PARAMETERS {
        backend.create(tenant, "SearchParameter", json!({
            "resourceType": "SearchParameter", "id": parameter.name,
            "url": format!("urn:hfs:test:{}", parameter.name),
            "name": parameter.name, "status": "active", "code": parameter.name,
            "base": ["Observation"], "type": parameter.fhir_type,
            "expression": format!("Observation.extension.where(url = 'urn:hfs:test:{}').value", parameter.name)
        }), FhirVersion::default()).await.unwrap();
    }
}

fn extensions(null: bool) -> Vec<Value> {
    PARAMETERS
        .iter()
        .flat_map(|parameter| {
            let value = if null {
                Value::Null
            } else {
                match parameter.kind {
                    SearchParamType::String => json!("present"),
                    SearchParamType::Token => json!("blocked"),
                    SearchParamType::Date => json!("2026-01-01T12:00:00Z"),
                    SearchParamType::Number => json!(5),
                    SearchParamType::Quantity => {
                        json!({"value": 5, "system": "http://unitsofmeasure.org", "code": "mg"})
                    }
                    SearchParamType::Reference => json!({"reference": "Patient/target"}),
                    SearchParamType::Uri => json!("urn:hfs:test:present"),
                    _ => unreachable!("fixture contains only scalar search parameters"),
                }
            };
            let extension = json!({
                "url": format!("urn:hfs:test:{}", parameter.name),
                (parameter.extension_field): value,
            });
            if null {
                vec![extension]
            } else {
                vec![extension.clone(), extension]
            }
        })
        .collect()
}

fn observation(id: &str, extensions: Vec<Value>) -> Value {
    json!({
        "resourceType": "Observation", "id": id, "status": "final",
        "code": {"coding": [{"code": "fixture"}]}, "extension": extensions,
    })
}

fn condition(parameter: &TypedParameter, value: &str) -> SearchParameter {
    SearchParameter {
        name: parameter.name.to_owned(),
        param_type: parameter.kind,
        modifier: None,
        values: vec![SearchValue::eq(value)],
        chain: vec![],
        components: vec![],
    }
}

fn missing_query(parameter: &TypedParameter, missing: bool) -> SearchQuery {
    let mut parameter = condition(parameter, if missing { "true" } else { "false" });
    parameter.modifier = Some(SearchModifier::Missing);
    SearchQuery::new("Observation")
        .with_count(20)
        .with_parameter(parameter)
        .with_sort(SortDirective::parse("_id"))
}

async fn assert_results(
    backend: &MongoBackend,
    tenant: &TenantContext,
    query: &SearchQuery,
    expected: &[&str],
) {
    let result = backend.search(tenant, query).await.unwrap();
    for row in &result.resources.items {
        assert!(
            row.content().get("missing_params").is_none(),
            "metadata leaked into FHIR content"
        );
    }
    let mut actual: Vec<_> = result
        .resources
        .items
        .iter()
        .map(|row| row.id().to_owned())
        .collect();
    actual.sort();
    let mut expected = expected.to_vec();
    expected.sort();
    assert_eq!(actual, expected, "query={query:?}");
    if matches!(query.total, Some(TotalMode::Accurate | TotalMode::Estimate)) {
        assert_eq!(result.total, Some(expected.len() as u64), "query={query:?}");
    }
}

async fn missing_parameters(
    db: &mongodb::Database,
    tenant: &TenantContext,
    resource_type: &str,
    id: &str,
) -> Vec<String> {
    let document = db.collection::<Document>("resources")
        .find_one(doc! { "tenant_id": tenant.tenant_id().as_str(), "resource_type": resource_type, "id": id })
        .await.unwrap().unwrap();
    assert!(
        !document
            .get_document("data")
            .unwrap()
            .contains_key("missing_params")
    );
    let names: Vec<_> = document
        .get_array("missing_params")
        .expect("resource metadata must be written")
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        names, sorted,
        "missing parameters must be sorted and unique"
    );
    names
}

async fn assert_missing_metadata(
    db: &mongodb::Database,
    tenant: &TenantContext,
    id: &str,
    parameter: &TypedParameter,
    expected: bool,
) {
    let names = missing_parameters(db, tenant, "Observation", id).await;
    assert_eq!(
        names.iter().any(|name| name == parameter.name),
        expected,
        "id={id}, parameter={}",
        parameter.name
    );
    let null_rows = db
        .collection::<Document>("search_index")
        .count_documents(doc! {
            "tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation",
            "resource_id": id, "param_name": parameter.name,
            parameter.index_field: { "$type": 10 },
        })
        .await
        .unwrap();
    assert_eq!(null_rows, 0, "absence must not create search_index rows");
}

#[tokio::test]
async fn mongodb_missing_metadata_typed_parameters_preserve_search_and_live_scope() {
    let Some(backend) = create_backend("missing_typed").await else {
        return;
    };
    let tenant = create_tenant("missing-typed");
    register_parameters(&backend, &tenant).await;
    for (id, values) in [
        ("absent", vec![]),
        ("null", extensions(true)),
        ("present", extensions(false)),
        ("deleted", vec![]),
    ] {
        backend
            .create(
                &tenant,
                "Observation",
                observation(id, values),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    backend
        .delete(&tenant, "Observation", "deleted")
        .await
        .unwrap();
    let db = backend.get_database().await.unwrap();
    for parameter in &PARAMETERS {
        assert_missing_metadata(&db, &tenant, "absent", parameter, true).await;
        assert_missing_metadata(&db, &tenant, "null", parameter, true).await;
        assert_missing_metadata(&db, &tenant, "present", parameter, false).await;
        assert_missing_metadata(&db, &tenant, "deleted", parameter, false).await;
    }
    let foreign = create_tenant("missing-foreign");
    backend
        .create(
            &foreign,
            "Observation",
            observation("foreign-only", vec![]),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "id": "wrong-type"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let names: Vec<_> = PARAMETERS.iter().map(|parameter| parameter.name).collect();
    // Stale metadata must still respect deletion, tenant and resource-type scope.
    for (owner, resource_type, id) in [
        (&tenant, "Observation", "deleted"),
        (&foreign, "Observation", "foreign-only"),
        (&tenant, "Patient", "wrong-type"),
    ] {
        db.collection::<Document>("resources").update_one(
            doc! {"tenant_id": owner.tenant_id().as_str(), "resource_type": resource_type, "id": id},
            doc! {"$set": {"missing_params": &names}},
        ).await.unwrap();
    }
    for parameter in &PARAMETERS {
        for missing in [true, false] {
            let expected: &[&str] = if missing {
                &["absent", "null"]
            } else {
                &["present"]
            };
            let mut query = missing_query(parameter, missing);
            for total in [TotalMode::Accurate, TotalMode::Estimate] {
                query.total = Some(total);
                assert_results(&backend, &tenant, &query, expected).await;
            }
            assert_eq!(
                backend.search_count(&tenant, &query).await.unwrap(),
                expected.len() as u64
            );
        }
    }
    for parameter in PARAMETERS
        .iter()
        .filter(|p| matches!(p.kind, SearchParamType::Number | SearchParamType::Quantity))
    {
        for (value, expected) in [("5", vec![]), ("6", vec!["present"])] {
            let mut ne = condition(parameter, value);
            ne.values[0].prefix = SearchPrefix::Ne;
            let mut query = SearchQuery::new("Observation").with_parameter(ne);
            query.total = Some(TotalMode::Accurate);
            assert_results(&backend, &tenant, &query, &expected).await;
        }
    }
    let mut not = condition(&PARAMETERS[1], "blocked");
    not.modifier = Some(SearchModifier::Not);
    let mut query = SearchQuery::new("Observation").with_parameter(not);
    query.total = Some(TotalMode::Accurate);
    assert_results(&backend, &tenant, &query, &["absent", "null"]).await;

    // Parameter sorting routes this selective query through the general matching fallback.
    for missing in [true, false] {
        let mut query = missing_query(&PARAMETERS[0], missing);
        query.parameters.push(SearchParameter {
            name: "code".into(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("fixture")],
            chain: vec![],
            components: vec![],
        });
        query.sort = vec![
            SortDirective::parse("missing-string").with_param_type(Some(SearchParamType::String)),
        ];
        query.total = Some(TotalMode::Accurate);
        let expected: &[&str] = if missing {
            &["absent", "null"]
        } else {
            &["present"]
        };
        assert_results(&backend, &tenant, &query, expected).await;
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn mongodb_missing_metadata_updates_restore_and_reindex_replace_absence() {
    let Some(backend) = create_backend("missing_lifecycle").await else {
        return;
    };
    let tenant = create_tenant("missing-lifecycle");
    register_parameters(&backend, &tenant).await;
    let db = backend.get_database().await.unwrap();
    for (values, missing) in [
        (vec![], true),
        (extensions(false), false),
        (extensions(true), true),
    ] {
        backend
            .create_or_update(
                &tenant,
                "Observation",
                "changing",
                observation("changing", values),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        for parameter in &PARAMETERS {
            assert_missing_metadata(&db, &tenant, "changing", parameter, missing).await;
            assert_results(
                &backend,
                &tenant,
                &missing_query(parameter, missing),
                &["changing"],
            )
            .await;
            assert_results(&backend, &tenant, &missing_query(parameter, !missing), &[]).await;
        }
    }
    backend
        .delete(&tenant, "Observation", "changing")
        .await
        .unwrap();
    assert_eq!(
        search_index_entry_count(&backend, &tenant, "Observation", "changing").await,
        0
    );
    assert!(
        missing_parameters(&db, &tenant, "Observation", "changing")
            .await
            .is_empty()
    );
    let (restored, _) = backend
        .create_or_update(
            &tenant,
            "Observation",
            "changing",
            observation("changing", vec![]),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    for parameter in &PARAMETERS {
        assert_missing_metadata(&db, &tenant, "changing", parameter, true).await;
    }
    db.collection::<Document>("search_index").delete_many(doc! {
        "tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation", "resource_id": "changing",
    }).await.unwrap();
    db.collection::<Document>("resources").update_one(
        doc! {"tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation", "id": "changing"},
        doc! {"$unset": {"missing_params": ""}},
    ).await.unwrap();
    for parameter in &PARAMETERS {
        db.collection::<Document>("search_index")
            .insert_one(doc! {
                "tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation",
                "resource_id": "changing", "param_name": parameter.name,
                parameter.index_field: mongodb::bson::Bson::Null,
            })
            .await
            .unwrap();
    }
    for _ in 0..2 {
        let results = backend
            .write_search_entries_page(&tenant, std::slice::from_ref(&restored))
            .await;
        assert!(results.iter().all(Result::is_ok), "{results:?}");
        for parameter in &PARAMETERS {
            assert_missing_metadata(&db, &tenant, "changing", parameter, true).await;
            assert_results(
                &backend,
                &tenant,
                &missing_query(parameter, true),
                &["changing"],
            )
            .await;
        }
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn mongodb_missing_metadata_transaction_and_bulk_writes_share_absence() {
    let Some(backend) = create_backend("missing_write_paths").await else {
        return;
    };
    let tenant = create_tenant("missing-write-paths");
    register_parameters(&backend, &tenant).await;
    let db = backend.get_database().await.unwrap();
    let entries = vec![BundleEntry {
        method: BundleMethod::Put,
        url: "Observation/transaction".into(),
        resource: Some(observation("transaction", vec![])),
        if_match: None,
        if_none_match: None,
        if_none_exist: None,
        full_url: None,
        criteria: None,
    }];
    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
        .expect("missing-index lifecycle coverage requires a replica set");
    assert_eq!(result.entries.len(), 1);
    let (submission, manifest) = bulk_submit::seed(&backend, &tenant).await;
    for (line, values, missing) in [
        (1, vec![], true),
        (2, extensions(false), false),
        (3, vec![], true),
    ] {
        let results = backend
            .process_entries(
                &tenant,
                &submission,
                &manifest,
                vec![NdjsonEntry::new(
                    line,
                    "Observation",
                    observation("bulk", values),
                )],
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].is_success(), "{results:?}");
        for parameter in &PARAMETERS {
            assert_missing_metadata(&db, &tenant, "transaction", parameter, true).await;
            assert_missing_metadata(&db, &tenant, "bulk", parameter, missing).await;
            let expected: &[&str] = if missing {
                &["bulk", "transaction"]
            } else {
                &["transaction"]
            };
            assert_results(&backend, &tenant, &missing_query(parameter, true), expected).await;
        }
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn mongodb_missing_metadata_failed_extraction_is_reported_by_reindex() {
    let Some(backend) = create_backend("missing_failed").await else {
        return;
    };
    let tenant = create_tenant("missing-failed");
    register_parameters(&backend, &tenant).await;
    backend
        .create(
            &tenant,
            "SearchParameter",
            json!({
                "resourceType": "SearchParameter", "id": "failed-index",
                "url": "urn:hfs:test:failed-index", "name": "failed-index",
                "status": "active", "code": "failed-index", "base": ["Observation"],
                "type": "token", "expression": "Observation.identifier.single()",
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let resource = backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation", "id": "failed", "status": "final",
                "code": {"text": "example"}, "identifier": [{"value": "one"}, {"value": "two"}],
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let db = backend.get_database().await.unwrap();
    let filter = doc! { "tenant_id": tenant.tenant_id().as_str(),
    "resource_type": "Observation", "resource_id": "failed", "param_name": "failed-index" };
    assert_eq!(
        db.collection::<Document>("search_index")
            .count_documents(filter)
            .await
            .unwrap(),
        0,
        "failed extraction must not create an index value"
    );
    let names = missing_parameters(&db, &tenant, "Observation", "failed").await;
    assert!(!names.iter().any(|name| name == "failed-index"));
    for parameter in &PARAMETERS {
        assert_missing_metadata(&db, &tenant, "failed", parameter, true).await;
    }
    for missing in [true, false] {
        let unknown = TypedParameter {
            name: "failed-index",
            kind: SearchParamType::Token,
            fhir_type: "token",
            extension_field: "valueCode",
            index_field: "value_token_code",
        };
        let mut query = missing_query(&unknown, missing);
        query.total = Some(TotalMode::Accurate);
        assert_results(&backend, &tenant, &query, &[]).await;
    }
    let mut stats = helios_persistence::search::ReindexPageStats::default();
    let outcomes = backend
        .write_search_entries_page_timed(&tenant, &[resource], &mut stats)
        .await;
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].is_ok(), "{outcomes:?}");
    let tally = stats
        .parameter_failures
        .get("failed-index")
        .expect("the failing parameter is tallied");
    assert_eq!(tally.resources, 1);
    assert!(
        tally.sample_error.contains("single"),
        "{:?}",
        tally.sample_error
    );
    assert_eq!(stats.parameter_failures.len(), 1, "{stats:?}");
    db.drop().await.unwrap();
}

#[tokio::test]
async fn mongodb_missing_metadata_expressionless_parameter_retains_fallback() {
    let Some(backend) = create_backend("missing_no_expression").await else {
        return;
    };
    let tenant = create_tenant("missing-no-expression");
    backend.create(&tenant, "SearchParameter", json!({
        "resourceType": "SearchParameter", "id": "expressionless",
        "url": "urn:hfs:test:expressionless", "name": "Expressionless",
        "status": "active", "code": "expressionless", "base": ["Observation"], "type": "string",
    }), FhirVersion::default()).await.unwrap();
    backend
        .create(
            &tenant,
            "Observation",
            observation("live", vec![]),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let parameter = TypedParameter {
        name: "expressionless",
        kind: SearchParamType::String,
        fhir_type: "string",
        extension_field: "valueString",
        index_field: "value_string",
    };
    let db = backend.get_database().await.unwrap();
    assert_missing_metadata(&db, &tenant, "live", &parameter, false).await;
    for missing in [true, false] {
        let mut query = missing_query(&parameter, missing);
        query.total = Some(TotalMode::Accurate);
        assert_results(
            &backend,
            &tenant,
            &query,
            if missing { &["live"] } else { &[] },
        )
        .await;
    }
    db.drop().await.unwrap();
}

fn patch_extensions(id: &str, values: Vec<Value>) -> BundleEntry {
    let operations: Vec<Value> = if values.is_empty() {
        extensions(false).iter().map(|extension| json!({"name": "operation", "part": [
            {"name": "type", "valueCode": "delete"},
            {"name": "path", "valueString": format!("Observation.extension.where(url = '{}').first()", extension["url"].as_str().unwrap())},
        ]})).collect()
    } else {
        values
            .into_iter()
            .map(|value| {
                json!({"name": "operation", "part": [
                    {"name": "type", "valueCode": "add"},
                    {"name": "path", "valueString": "Observation"},
                    {"name": "name", "valueString": "extension"},
                    {"name": "value", "valueExtension": value},
                ]})
            })
            .collect()
    };
    BundleEntry {
        method: BundleMethod::Patch,
        url: format!("Observation/{id}"),
        resource: Some(json!({"resourceType": "Parameters", "parameter": operations})),
        if_match: None,
        if_none_match: None,
        if_none_exist: None,
        full_url: None,
        criteria: None,
    }
}

#[tokio::test]
async fn mongodb_missing_metadata_patch_rollback_and_stale_reindex_preserve_current_version() {
    let Some(backend) = create_backend("missing_patch_stale").await else {
        return;
    };
    let tenant = create_tenant("missing-patch-stale");
    register_parameters(&backend, &tenant).await;
    let original = backend
        .create(
            &tenant,
            "Observation",
            observation("changing", vec![]),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let db = backend.get_database().await.unwrap();
    let result = backend
        .process_transaction(
            &tenant,
            vec![patch_extensions("changing", extensions(false))],
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.entries[0].status, 200);
    for parameter in &PARAMETERS {
        assert_missing_metadata(&db, &tenant, "changing", parameter, false).await;
        assert_results(
            &backend,
            &tenant,
            &missing_query(parameter, false),
            &["changing"],
        )
        .await;
    }
    let rollback = backend
        .process_transaction(
            &tenant,
            vec![
                patch_extensions("changing", vec![]),
                patch_extensions("does-not-exist", vec![]),
            ],
            FhirVersion::default(),
        )
        .await;
    assert!(rollback.is_err());
    for parameter in &PARAMETERS {
        assert_missing_metadata(&db, &tenant, "changing", parameter, false).await;
        assert_results(
            &backend,
            &tenant,
            &missing_query(parameter, false),
            &["changing"],
        )
        .await;
    }
    let current = backend
        .read(&tenant, "Observation", "changing")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.content()["extension"], json!(extensions(false)));
    assert!(current.content().get("missing_params").is_none());
    let historical = backend
        .vread(&tenant, "Observation", "changing", "1")
        .await
        .unwrap()
        .unwrap();
    assert!(historical.content().get("missing_params").is_none());
    let stale = backend
        .write_search_entries_page(&tenant, &[original])
        .await;
    assert!(
        stale[0].is_ok(),
        "a superseded snapshot leaves current metadata alone without failing the catch-up walk"
    );
    for parameter in &PARAMETERS {
        assert_missing_metadata(&db, &tenant, "changing", parameter, false).await;
    }
    backend
        .process_transaction(
            &tenant,
            vec![patch_extensions("changing", vec![])],
            FhirVersion::default(),
        )
        .await
        .unwrap();
    for parameter in &PARAMETERS {
        assert_missing_metadata(&db, &tenant, "changing", parameter, true).await;
    }
    backend
        .process_transaction(
            &tenant,
            vec![BundleEntry {
                method: BundleMethod::Delete,
                url: "Observation/changing".into(),
                resource: None,
                if_match: None,
                if_none_match: None,
                if_none_exist: None,
                full_url: None,
                criteria: None,
            }],
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert!(
        missing_parameters(&db, &tenant, "Observation", "changing")
            .await
            .is_empty()
    );
    db.drop().await.unwrap();
}

fn missing_condition(parameter: &TypedParameter, missing: bool) -> SearchParameter {
    let mut result = condition(parameter, if missing { "true" } else { "false" });
    result.modifier = Some(SearchModifier::Missing);
    result
}

#[tokio::test]
async fn mongodb_missing_metadata_repeated_and_mixed_filters_page_and_count() {
    let Some(backend) = create_backend("missing_combinations").await else {
        return;
    };
    let tenant = create_tenant("missing-combinations");
    register_parameters(&backend, &tenant).await;
    let mut fixture = Vec::new();
    for number in 0..48 {
        let values = extensions(false)
            .into_iter()
            .filter(|value| {
                let name = value["url"].as_str().unwrap();
                !(name.ends_with("missing-token") && number % 2 == 0
                    || name.ends_with("missing-string") && number % 3 == 0)
            })
            .collect();
        let id = format!("row-{number:03}");
        let stored = backend
            .create(
                &tenant,
                "Observation",
                observation(&id, values),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        fixture.push((id, stored.last_modified().timestamp_millis(), number));
    }
    let db = backend.get_database().await.unwrap();
    // Mix pre-metadata resources with both forms of current metadata in every query.
    for (id, _, number) in &fixture {
        if number % 5 == 0 {
            db.collection::<Document>("resources").update_one(
                doc! {"tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation", "id": id},
                doc! {"$unset": {"missing_params": ""}},
            ).await.unwrap();
        }
    }
    db.collection::<Document>("resources").update_one(
        doc! {"tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation", "id": "row-001"},
        doc! {"$set": {"missing_params": []}},
    ).await.unwrap();
    let foreign = create_tenant("missing-combinations-foreign");
    backend
        .create(
            &foreign,
            "Observation",
            observation("row-001", vec![]),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "id": "row-001"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    for (owner, resource_type) in [(&foreign, "Observation"), (&tenant, "Patient")] {
        db.collection::<Document>("resources").update_one(
            doc! {"tenant_id": owner.tenant_id().as_str(), "resource_type": resource_type, "id": "row-001"},
            doc! {"$set": {"missing_params": ["missing-string", "missing-token"]}},
        ).await.unwrap();
    }
    let shapes = [
        vec![(1, true), (0, true)],
        vec![(1, true), (0, false)],
        vec![(1, true), (1, true)],
        vec![(1, true), (1, false)],
    ];
    for shape in shapes {
        for id_sort in [true, false] {
            let mut query = SearchQuery::new("Observation").with_count(3);
            query.total = Some(TotalMode::None);
            if id_sort {
                query = query.with_sort(SortDirective::parse("_id"));
            }
            for &(parameter, missing) in &shape {
                query
                    .parameters
                    .push(missing_condition(&PARAMETERS[parameter], missing));
            }
            let mut expected: Vec<_> = fixture
                .iter()
                .filter(|(_, _, number)| {
                    shape.iter().all(|&(parameter, missing)| {
                        let absent = if parameter == 1 {
                            number % 2 == 0
                        } else {
                            number % 3 == 0
                        };
                        absent == missing
                    })
                })
                .collect();
            if id_sort {
                expected.sort_by(|a, b| a.0.cmp(&b.0));
            } else {
                expected.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
            }
            let expected_ids: Vec<_> = expected.iter().map(|row| row.0.as_str()).collect();
            assert_eq!(
                backend.search_count(&tenant, &query).await.unwrap(),
                expected_ids.len() as u64
            );
            let mut summary = query.clone();
            summary.summary = Some(helios_persistence::types::SummaryMode::Count);
            for total in [TotalMode::Accurate, TotalMode::Estimate] {
                summary.total = Some(total);
                let count_result = backend.search(&tenant, &summary).await.unwrap();
                assert_eq!(count_result.total, Some(expected_ids.len() as u64));
                assert!(
                    count_result
                        .resources
                        .items
                        .iter()
                        .all(|row| expected_ids.contains(&row.id()))
                );
            }
            let first = backend.search(&tenant, &query).await.unwrap();
            let ids: Vec<_> = first.resources.items.iter().map(|row| row.id()).collect();
            assert_eq!(
                ids,
                expected_ids[..expected_ids.len().min(3)],
                "shape={shape:?}, id_sort={id_sort}"
            );
            if expected_ids.len() > 3 {
                let next = first.resources.page_info.next_cursor.unwrap();
                let second = backend
                    .search(&tenant, &query.clone().with_cursor(next))
                    .await
                    .unwrap();
                let ids: Vec<_> = second.resources.items.iter().map(|row| row.id()).collect();
                assert_eq!(ids, expected_ids[3..expected_ids.len().min(6)]);
                let previous = second.resources.page_info.previous_cursor.unwrap();
                let back = backend
                    .search(&tenant, &query.clone().with_cursor(previous))
                    .await
                    .unwrap();
                let ids: Vec<_> = back.resources.items.iter().map(|row| row.id()).collect();
                assert_eq!(ids, expected_ids[..3]);
            } else {
                assert!(first.resources.page_info.next_cursor.is_none());
            }
        }
    }
    backend.get_database().await.unwrap().drop().await.unwrap();
}

#[tokio::test]
async fn mongodb_missing_metadata_composite_and_derived_definitions_preserve_presence() {
    let Some(backend) = create_backend_with_full_registry("missing_composite").await else {
        return;
    };
    let tenant = create_tenant("missing-composite");
    backend.create(&tenant, "SearchParameter", json!({
        "resourceType": "SearchParameter", "id": "derived-method", "status": "active",
        "url": "urn:hfs:test:derived-method", "name": "DerivedMethod", "code": "derived-method",
        "base": ["Observation"], "type": "token", "expression": "Observation.method",
        "derivedFrom": "http://hl7.org/fhir/SearchParameter/Observation-method"
    }), FhirVersion::default()).await.unwrap();
    for (id, components) in [
        ("empty", json!([])),
        (
            "complete",
            json!([{"code": {"coding": [{"code": "x"}]}, "valueQuantity": {"value": 5}}]),
        ),
        ("partial", json!([{"code": {"coding": [{"code": "x"}]}}])),
    ] {
        let mut resource = observation(id, vec![]);
        resource["component"] = components;
        if id == "complete" {
            resource["method"] = json!({"coding": [{"code": "manual"}]});
        }
        backend
            .create(&tenant, "Observation", resource, FhirVersion::default())
            .await
            .unwrap();
    }
    let db = backend.get_database().await.unwrap();
    for (name, kind, missing_ids, present_ids) in [
        (
            "component-code-value-quantity",
            SearchParamType::Composite,
            vec!["empty"],
            vec!["complete", "partial"],
        ),
        (
            "derived-method",
            SearchParamType::Token,
            vec!["empty", "partial"],
            vec!["complete"],
        ),
    ] {
        for id in ["empty", "complete", "partial"] {
            let names = missing_parameters(&db, &tenant, "Observation", id).await;
            assert_eq!(
                names.iter().any(|entry| entry == name),
                missing_ids.contains(&id),
                "{name}/{id}"
            );
        }
        for (missing, expected) in [(true, &missing_ids), (false, &present_ids)] {
            let mut query = SearchQuery::new("Observation")
                .with_parameter(SearchParameter {
                    name: name.into(),
                    param_type: kind,
                    modifier: Some(SearchModifier::Missing),
                    values: vec![SearchValue::eq(if missing { "true" } else { "false" })],
                    chain: vec![],
                    components: vec![],
                })
                .with_sort(SortDirective::parse("_id"));
            query.total = Some(TotalMode::Accurate);
            assert_results(&backend, &tenant, &query, expected).await;
        }
    }
    db.drop().await.unwrap();
}

/// Empties the profiler so the next capture holds only the next search's
/// operations. Profiling must be off. A `ts` window against the client clock
/// does not isolate searches: mongod's clock and timestamp semantics differ
/// from the runner's by milliseconds, which let the previous search's work in
/// or dropped the next search's first reads.
async fn clear_profile(db: &mongodb::Database) {
    db.collection::<Document>("system.profile")
        .drop()
        .await
        .unwrap();
}

#[tokio::test]
async fn mongodb_missing_metadata_shared_index_handles_empty_and_dense_id_pages() {
    use futures::TryStreamExt;
    let Some(backend) = create_backend("missing_index_work").await else {
        return;
    };
    let tenant = create_tenant("missing-index-work");
    register_parameters(&backend, &tenant).await;
    // Populate the presence index so an unbounded compatibility lookup cannot pass cheaply.
    for number in 0..1200 {
        // The string ID "false" must not collide with the lookup's Boolean no-match key.
        let id = if number == 0 {
            "false".to_owned()
        } else {
            format!("z-present-{number:04}")
        };
        let values = vec![json!({
            "url": "urn:hfs:test:missing-token", "valueCode": "present",
        })];
        backend
            .create(
                &tenant,
                "Observation",
                observation(&id, values),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    for number in 0..1200 {
        let id = format!("row-{number:04}");
        let mut resource = observation(&id, vec![]);
        if number == 1199 {
            resource["code"] = json!({"coding": [{"code": "rare"}]});
        }
        backend
            .create(&tenant, "Observation", resource, FhirVersion::default())
            .await
            .unwrap();
    }
    let db = backend.get_database().await.unwrap();
    let explained = db
        .run_command(doc! {
            "explain": {"find": "resources", "filter": {
                "tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation",
                "is_deleted": false, "missing_params": "status",
            }, "projection": {"_id": 0, "id": 1}, "hint": "idx_resources_missing_params"},
            "verbosity": "executionStats",
        })
        .await
        .unwrap();
    let stats = explained.get_document("executionStats").unwrap();
    assert_eq!(stats.get_i32("nReturned").unwrap(), 0);
    assert_eq!(stats.get_i32("totalDocsExamined").unwrap(), 0);
    assert!(
        format!("{:?}", explained.get_document("queryPlanner").unwrap())
            .contains("idx_resources_missing_params")
    );
    db.run_command(doc! {"profile": 2}).await.unwrap();
    let mut query = missing_query(&PARAMETERS[1], true).with_count(2);
    query.total = Some(TotalMode::None);
    let result = backend.search(&tenant, &query).await;
    db.run_command(doc! {"profile": 0}).await.unwrap();
    let result = result.unwrap();
    assert_eq!(
        result
            .resources
            .items
            .iter()
            .map(|row| row.id())
            .collect::<Vec<_>>(),
        ["row-0000", "row-0001"]
    );
    let replies: Vec<Document> = db.collection("system.profile").find(doc! {
        "ns": {"$in": [format!("{}.resources", db.name()), format!("{}.search_index", db.name())]},
        "op": {"$in": ["query", "command", "getmore"]}, "nreturned": {"$exists": true},
    }).await.unwrap().try_collect().await.unwrap();
    let returned: i64 = replies
        .iter()
        .map(|reply| {
            reply
                .get_i64("nreturned")
                .or_else(|_| reply.get_i32("nreturned").map(i64::from))
                .unwrap()
        })
        .sum();
    assert!(
        (2..600).contains(&returned),
        "dense page must not transfer all 1200 matches: {returned}; {replies:?}"
    );
    for reply in &replies {
        assert_ne!(
            reply.get_bool("hasSortStage"),
            Ok(true),
            "dense _id page must not sort all missing matches: {reply:?}"
        );
    }
    let command = replies
        .iter()
        .filter_map(|reply| reply.get_document("command").ok())
        .find(|command| command.get_str("aggregate") == Ok("resources"))
        .expect("capture the actual missing-page aggregate");
    let explained = db
        .run_command(doc! {
            "explain": {
                "aggregate": "resources",
                "pipeline": command.get_array("pipeline").unwrap().clone(),
                "hint": command.get("hint").unwrap().clone(),
                "allowDiskUse": true,
                "cursor": {},
            },
            "verbosity": "executionStats",
            "maxTimeMS": 5000,
        })
        .await
        .unwrap();
    let lookups: Vec<_> = explained
        .get_array("stages")
        .unwrap()
        .iter()
        .filter_map(Bson::as_document)
        .filter(|stage| stage.contains_key("$lookup"))
        .collect();
    assert!(!lookups.is_empty(), "must measure the compatibility lookup");
    // Cursor read-ahead may consume the small fixture; known metadata must do no foreign reads.
    for lookup in lookups {
        for field in ["totalKeysExamined", "totalDocsExamined"] {
            let examined = lookup
                .get_i64(field)
                .or_else(|_| lookup.get_i32(field).map(i64::from))
                .unwrap();
            assert_eq!(
                examined, 0,
                "known missing metadata must skip presence-index reads: {field}; {lookup:?}"
            );
            eprintln!("Dense ID page compatibility work: {field}={examined}");
        }
    }
    let mut ordered = query.clone();
    ordered.sort.clear();
    clear_profile(&db).await;
    db.run_command(doc! {"profile": 2}).await.unwrap();
    let result = backend.search(&tenant, &ordered).await;
    db.run_command(doc! {"profile": 0}).await.unwrap();
    let result = result.unwrap();
    assert_eq!(
        result
            .resources
            .items
            .iter()
            .map(|row| row.id())
            .collect::<Vec<_>>(),
        ["row-1199", "row-1198"]
    );
    let ordered_operations: Vec<Document> = db.collection("system.profile").find(doc! {
        "ns": {"$in": [format!("{}.resources", db.name()), format!("{}.search_index", db.name())]},
        "op": {"$in": ["query", "command", "getmore"]}, "nreturned": {"$exists": true},
    }).await.unwrap().try_collect().await.unwrap();
    assert!(!ordered_operations.is_empty());
    let candidate_operations: Vec<_> = ordered_operations
        .iter()
        .filter(|operation| {
            operation
                .get_document("command")
                .ok()
                .is_some_and(|command| {
                    command.get_str("hint") == Ok("idx_resources_missing_params")
                        && command.contains_key("sort")
                })
        })
        .collect();
    assert!(
        !candidate_operations.is_empty(),
        "missing candidate stream must use the shared index"
    );
    for operation in candidate_operations {
        assert_ne!(
            operation.get_bool("hasSortStage"),
            Ok(true),
            "candidate metadata must retain index order: {operation:?}"
        );
        assert_eq!(
            operation.get_i32("docsExamined"),
            Ok(0),
            "current candidate metadata must be covered: {operation:?}"
        );
    }
    // One 256-row stream, bounded metadata reads, and the final three-row fetch.
    for (field, bound) in [("keysExamined", 780), ("docsExamined", 260)] {
        let examined: i64 = ordered_operations
            .iter()
            .map(|operation| {
                operation
                    .get_i64(field)
                    .or_else(|_| operation.get_i32(field).map(i64::from))
                    .unwrap_or(0)
            })
            .sum();
        assert!(
            examined <= bound,
            "current metadata ordered page must seek, not scan all 1200 rows: {field}={examined}; {ordered_operations:?}"
        );
    }
    for (code, expected) in [("rare", vec!["row-1199"]), ("does-not-exist", vec![])] {
        let mut selective = query.clone();
        selective.parameters.push(SearchParameter {
            name: "code".into(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq(code)],
            chain: vec![],
            components: vec![],
        });
        clear_profile(&db).await;
        db.run_command(doc! {"profile": 2}).await.unwrap();
        let result = backend.search(&tenant, &selective).await;
        db.run_command(doc! {"profile": 0}).await.unwrap();
        let result = result.unwrap();
        assert_eq!(
            result
                .resources
                .items
                .iter()
                .map(|row| row.id())
                .collect::<Vec<_>>(),
            expected
        );
        let operations: Vec<Document> = db.collection("system.profile").find(doc! {
            "ns": {"$in": [format!("{}.resources", db.name()), format!("{}.search_index", db.name())]},
            "op": {"$in": ["query", "command", "getmore"]},
            "nreturned": {"$exists": true},
        }).await.unwrap().try_collect().await.unwrap();
        assert!(!operations.is_empty(), "must capture actual search work");
        for field in ["keysExamined", "docsExamined", "nreturned"] {
            let examined: i64 = operations
                .iter()
                .map(|operation| {
                    operation
                        .get_i64(field)
                        .or_else(|_| operation.get_i32(field).map(i64::from))
                        .unwrap_or(0)
                })
                .sum();
            assert!(
                examined < 600,
                "code={code}: selective search must not enumerate 1200 missing resources; {field}={examined}, {operations:?}"
            );
        }
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn mongodb_missing_metadata_deferred_and_offloaded_writes_leave_presence_unknown() {
    for offloaded in [true, false] {
        let Some(backend) =
            create_backend_with_search_offloaded("missing_unknown_write", offloaded).await
        else {
            return;
        };
        let tenant = create_tenant("missing-unknown-write");
        let db = backend.get_database().await.unwrap();
        let filter = doc! {"tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation", "id": "changing"};
        if offloaded {
            backend
                .create(
                    &tenant,
                    "Observation",
                    observation("changing", vec![]),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
            let row = db
                .collection::<Document>("resources")
                .find_one(filter.clone())
                .await
                .unwrap()
                .unwrap();
            assert!(!row.contains_key("missing_params"));
        } else {
            backend
                .create(
                    &tenant,
                    "Observation",
                    observation("changing", vec![]),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        // A later unindexed write must remove previously authoritative metadata.
        db.collection::<Document>("resources")
            .update_one(filter.clone(), doc! {"$set": {"missing_params": []}})
            .await
            .unwrap();
        if offloaded {
            backend
                .create_or_update(
                    &tenant,
                    "Observation",
                    "changing",
                    observation("changing", extensions(false)),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        } else {
            let (submission, manifest) = bulk_submit::seed(&backend, &tenant).await;
            let results = backend
                .process_entries(
                    &tenant,
                    &submission,
                    &manifest,
                    vec![
                        NdjsonEntry::new(
                            1,
                            "Observation",
                            observation("changing", extensions(false)),
                        ),
                        NdjsonEntry::new(2, "Observation", observation("new", vec![])),
                    ],
                    &BulkProcessingOptions::new().with_defer_indexing(true),
                )
                .await
                .unwrap();
            assert_eq!(results.len(), 2);
            assert!(results.iter().all(|result| result.is_success()));
            let row = db.collection::<Document>("resources").find_one(doc! {"tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation", "id": "new"}).await.unwrap().unwrap();
            assert!(!row.contains_key("missing_params"));
        }
        let row = db
            .collection::<Document>("resources")
            .find_one(filter)
            .await
            .unwrap()
            .unwrap();
        assert!(
            !row.contains_key("missing_params"),
            "unindexed data must not claim complete metadata"
        );
        assert!(
            !row.get_document("data")
                .unwrap()
                .contains_key("missing_params")
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn mongodb_missing_metadata_definition_maintenance_restores_fallback_and_reindexes() {
    let Some(backend) = create_backend("missing_definition_freshness").await else {
        return;
    };
    let tenant = create_tenant("missing-definition-freshness");
    let original = backend
        .create(
            &tenant,
            "Observation",
            observation("before-definition", vec![]),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let db = backend.get_database().await.unwrap();
    let filter = doc! {"tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation", "id": "before-definition"};
    let initial = db
        .collection::<Document>("resources")
        .find_one(filter.clone())
        .await
        .unwrap()
        .unwrap();
    assert!(initial.contains_key("missing_params"));
    let mut definition = json!({
        "resourceType": "SearchParameter", "id": "newly-defined", "status": "active",
        "url": "urn:hfs:test:newly-defined", "name": "NewlyDefined", "code": "newly-defined",
        "base": ["Observation"], "type": "token",
        "expression": "Observation.extension.where(url = 'urn:hfs:test:newly-defined').value"
    });
    backend
        .create(
            &tenant,
            "SearchParameter",
            definition.clone(),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    // Definition maintenance drains writers, refreshes every registry, then
    // removes the complete metadata field before requests resume.
    backend.refresh_stored_search_parameters().await.unwrap();
    db.collection::<Document>("resources")
        .update_many(
            doc! {"tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation"},
            doc! {"$unset": {"missing_params": ""}},
        )
        .await
        .unwrap();
    let parameter = TypedParameter {
        name: "newly-defined",
        kind: SearchParamType::Token,
        fhir_type: "token",
        extension_field: "valueCode",
        index_field: "value_token_code",
    };
    let mut missing = missing_query(&parameter, true);
    missing.total = Some(TotalMode::Accurate);
    assert_results(&backend, &tenant, &missing, &["before-definition"]).await;
    let reindexed = backend
        .write_search_entries_page(&tenant, std::slice::from_ref(&original))
        .await;
    assert!(reindexed.iter().all(Result::is_ok), "{reindexed:?}");
    assert_missing_metadata(&db, &tenant, "before-definition", &parameter, true).await;
    let current = db
        .collection::<Document>("resources")
        .find_one(filter.clone())
        .await
        .unwrap()
        .unwrap();
    let registered_missing = current.get("missing_params").unwrap().clone();
    backend.refresh_stored_search_parameters().await.unwrap();
    let reindexed = backend
        .write_search_entries_page(&tenant, std::slice::from_ref(&original))
        .await;
    assert!(reindexed.iter().all(Result::is_ok));
    let unchanged = db
        .collection::<Document>("resources")
        .find_one(filter.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        unchanged.get("missing_params"),
        Some(&registered_missing),
        "unchanged definitions preserve absence metadata"
    );
    let (present, _) = backend
        .create_or_update(
            &tenant,
            "Observation",
            "before-definition",
            observation(
                "before-definition",
                vec![json!({"url":"urn:hfs:test:newly-defined", "valueCode":"present"})],
            ),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_results(
        &backend,
        &tenant,
        &missing_query(&parameter, false),
        &["before-definition"],
    )
    .await;
    definition["expression"] = json!("Observation.method");
    backend
        .create_or_update(
            &tenant,
            "SearchParameter",
            "newly-defined",
            definition,
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let outcomes = backend.write_search_entries_page(&tenant, &[present]).await;
    assert!(outcomes.iter().all(Result::is_ok), "{outcomes:?}");
    assert_missing_metadata(&db, &tenant, "before-definition", &parameter, true).await;
    assert_results(&backend, &tenant, &missing, &["before-definition"]).await;
    assert_results(&backend, &tenant, &missing_query(&parameter, false), &[]).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn mongodb_missing_conditional_transactions_match_absence() {
    let Some(backend) = create_backend("missing_conditional").await else {
        return;
    };
    let db = backend.get_database().await.unwrap();
    for old_metadata in [false, true] {
        let tenant = create_tenant(&format!("missing-conditional-{old_metadata}"));
        register_parameters(&backend, &tenant).await;
        backend
            .create(
                &tenant,
                "Observation",
                observation("absent", vec![]),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        if old_metadata {
            db.collection::<Document>("resources")
                .update_many(
                    doc! {"tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation"},
                    doc! {"$unset": {"missing_params": ""}},
                )
                .await
                .unwrap();
        }
        let put = BundleEntry {
            method: BundleMethod::Put,
            url: "Observation/present".into(),
            resource: Some(observation("present", extensions(false))),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: None,
            criteria: None,
        };
        let delete = |missing: bool| {
            conditional_url_suite::with_typed_criteria(
                &backend,
                &tenant,
                BundleEntry {
                    method: BundleMethod::Delete,
                    url: format!("Observation?missing-token:missing={missing}"),
                    resource: None,
                    if_match: None,
                    if_none_match: None,
                    if_none_exist: None,
                    full_url: None,
                    criteria: None,
                },
            )
        };
        assert_results(
            &backend,
            &tenant,
            &missing_query(&PARAMETERS[1], true),
            &["absent"],
        )
        .await;
        let result = backend
            .process_transaction(
                &tenant,
                vec![put.clone(), delete(true)],
                FhirVersion::default(),
            )
            .await
            .unwrap();
        assert_eq!(result.entries[0].status, 201);
        assert_eq!(result.entries[1].status, 204);
        assert!(
            matches!(
                backend.read(&tenant, "Observation", "absent").await,
                Err(StorageError::Resource(ResourceError::Gone { .. }))
            ),
            "absence criteria must delete the resource GET would have matched"
        );
        assert!(
            backend
                .read(&tenant, "Observation", "present")
                .await
                .unwrap()
                .is_some()
        );

        let bare_put = BundleEntry {
            method: BundleMethod::Put,
            url: "Observation/bare".into(),
            resource: Some(observation("bare", vec![])),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: None,
            criteria: None,
        };
        let absent_conditional_create = BundleEntry {
            method: BundleMethod::Post,
            url: "Observation".into(),
            resource: Some(observation("never-created", extensions(false))),
            if_match: None,
            if_none_match: None,
            if_none_exist: Some("missing-token:missing=true".into()),
            full_url: None,
            criteria: None,
        };
        let result = backend
            .process_transaction(
                &tenant,
                vec![bare_put, absent_conditional_create],
                FhirVersion::default(),
            )
            .await
            .unwrap();
        assert_eq!(result.entries[0].status, 201);
        assert_eq!(
            result.entries[1].status, 200,
            "ifNoneExist absence must match the resource written earlier in the same bundle"
        );
        assert_eq!(
            result.entries[1].reference().as_deref(),
            Some("Observation/bare")
        );

        backend
            .create(
                &tenant,
                "Observation",
                observation("bare2", vec![]),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .process_transaction(&tenant, vec![delete(true)], FhirVersion::default())
            .await
            .unwrap_err();
        for id in ["bare", "bare2"] {
            assert!(
                backend
                    .read(&tenant, "Observation", id)
                    .await
                    .unwrap()
                    .is_some(),
                "a multi-match conditional delete must change nothing"
            );
        }
        backend
            .delete(&tenant, "Observation", "bare2")
            .await
            .unwrap();
        // Presence reads the value rows written earlier in the same transaction.
        let conditional_create = BundleEntry {
            method: BundleMethod::Post,
            url: "Observation".into(),
            resource: Some(observation("would-create", extensions(false))),
            if_match: None,
            if_none_match: None,
            if_none_exist: Some("missing-token:missing=false".into()),
            full_url: None,
            criteria: None,
        };
        let result = backend
            .process_transaction(
                &tenant,
                vec![put, conditional_create],
                FhirVersion::default(),
            )
            .await
            .unwrap();
        assert_eq!(result.entries[1].status, 200);
        assert_eq!(
            result.entries[1].reference().as_deref(),
            Some("Observation/present")
        );
        let result = backend
            .process_transaction(&tenant, vec![delete(false)], FhirVersion::default())
            .await
            .unwrap();
        assert_eq!(result.entries[0].status, 204);
        assert!(
            matches!(
                backend.read(&tenant, "Observation", "present").await,
                Err(StorageError::Resource(ResourceError::Gone { .. }))
            ),
            "conditional presence must delete the newly written resource"
        );
        assert!(
            backend
                .read(&tenant, "Observation", "bare")
                .await
                .unwrap()
                .is_some()
        );
        assert_results(
            &backend,
            &tenant,
            &missing_query(&PARAMETERS[1], true),
            &["bare"],
        )
        .await;
    }
    db.drop().await.unwrap();
}

fn date_extensions(values: Vec<Value>) -> Vec<Value> {
    values
        .into_iter()
        .map(|value| {
            let field = if value.is_object() {
                "valuePeriod"
            } else {
                "valueDateTime"
            };
            json!({"url": "urn:hfs:test:missing-date", (field): value})
        })
        .collect()
}

/// #2000: a value the extractor yields but the MongoDB writer cannot build a
/// row for leaves the parameter as absent as one that extracted nothing.
#[tokio::test]
async fn mongodb_missing_metadata_unbuildable_dates_are_missing() {
    let Some(backend) = create_backend("missing_unbuildable").await else {
        return;
    };
    let tenant = create_tenant("missing-unbuildable");
    register_parameters(&backend, &tenant).await;
    for (id, values) in [
        ("invalid-date", vec![json!("not-a-date")]),
        (
            "invalid-period-end",
            vec![json!({"start": "2026-01-01", "end": "not-a-date"})],
        ),
        ("mixed", vec![json!("not-a-date"), json!("2026-01-01")]),
        ("valid", vec![json!("2026-01-01")]),
    ] {
        backend
            .create(
                &tenant,
                "Observation",
                observation(id, date_extensions(values)),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    let db = backend.get_database().await.unwrap();
    let date = &PARAMETERS[2];
    assert_eq!(date.name, "missing-date");
    let rows = db
        .collection::<Document>("search_index")
        .count_documents(doc! {
            "tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation",
            "resource_id": "invalid-date", "param_name": date.name,
        })
        .await
        .unwrap();
    assert_eq!(rows, 0, "an invalid date must not build an index row");
    for (id, missing) in [
        ("invalid-date", true),
        ("invalid-period-end", true),
        ("mixed", false),
        ("valid", false),
    ] {
        assert_missing_metadata(&db, &tenant, id, date, missing).await;
    }
    for (missing, expected) in [
        (true, vec!["invalid-date", "invalid-period-end"]),
        (false, vec!["mixed", "valid"]),
    ] {
        let mut query = missing_query(date, missing);
        query.total = Some(TotalMode::Accurate);
        assert_results(&backend, &tenant, &query, &expected).await;
    }
    db.drop().await.unwrap();
}

/// For every resource and every parameter family, exactly one of
/// `:missing=true` and `:missing=false` matches (#2000).
#[tokio::test]
async fn mongodb_missing_metadata_true_and_false_are_complements() {
    let Some(backend) = create_backend_with_full_registry("missing_complement").await else {
        return;
    };
    let tenant = create_tenant("missing-complement");
    register_parameters(&backend, &tenant).await;
    let mut resources = vec![
        ("absent", observation("absent", vec![])),
        ("null", observation("null", extensions(true))),
        ("present", observation("present", extensions(false))),
        (
            "invalid-date",
            observation("invalid-date", date_extensions(vec![json!("not-a-date")])),
        ),
        (
            "invalid-period-end",
            observation(
                "invalid-period-end",
                date_extensions(vec![json!({"start": "2026-01-01", "end": "not-a-date"})]),
            ),
        ),
    ];
    resources[2].1["effectiveDateTime"] = json!("2026-01-01");
    resources[2].1["component"] =
        json!([{"code": {"coding": [{"code": "x"}]}, "valueQuantity": {"value": 5}}]);
    resources[3].1["effectiveDateTime"] = json!("not-a-date");
    resources[3].1["component"] = json!([{"code": {"coding": [{"code": "x"}]}}]);
    resources[4].1["effectivePeriod"] = json!({"start": "2026-01-01", "end": "not-a-date"});
    let ids: Vec<_> = resources.iter().map(|(id, _)| *id).collect();
    for (_, resource) in resources {
        backend
            .create(&tenant, "Observation", resource, FhirVersion::default())
            .await
            .unwrap();
    }
    let parameters = PARAMETERS
        .iter()
        .map(|parameter| (parameter.name, parameter.kind))
        .chain([
            ("date", SearchParamType::Date),
            ("component-code-value-quantity", SearchParamType::Composite),
        ]);
    for (name, kind) in parameters {
        let mut matched = Vec::new();
        for missing in [true, false] {
            let mut query = SearchQuery::new("Observation")
                .with_count(20)
                .with_parameter(SearchParameter {
                    name: name.into(),
                    param_type: kind,
                    modifier: Some(SearchModifier::Missing),
                    values: vec![SearchValue::eq(if missing { "true" } else { "false" })],
                    chain: vec![],
                    components: vec![],
                })
                .with_sort(SortDirective::parse("_id"));
            query.total = Some(TotalMode::Accurate);
            let result = backend.search(&tenant, &query).await.unwrap();
            assert_eq!(
                result.total,
                Some(result.resources.items.len() as u64),
                "{name}:missing={missing}"
            );
            matched.extend(result.resources.items.iter().map(|row| row.id().to_owned()));
        }
        matched.sort();
        let mut expected: Vec<_> = ids.iter().map(|id| id.to_string()).collect();
        expected.sort();
        assert_eq!(
            matched, expected,
            "{name}: each resource must match exactly once"
        );
    }
    let db = backend.get_database().await.unwrap();
    db.drop().await.unwrap();
}

async fn resources_index_names(db: &mongodb::Database) -> Vec<String> {
    db.collection::<Document>("resources")
        .list_index_names()
        .await
        .unwrap()
}

/// #2001: `idx_resources_missing_params` follows `HFS_MONGODB_INDEX_BUILD`
/// instead of blocking startup. With `off` the node starts and is ready
/// without it, `:missing=true` takes the value-index route (a hint naming the
/// absent index would fail the search), and results are identical once a
/// later boot has built it.
#[tokio::test]
async fn mongodb_missing_index_follows_index_build_mode() {
    let Some(cs) = shared_mongo::connection_string().await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let db_name = build_test_database_name("missing_index_mode");
    let index = "idx_resources_missing_params".to_string();
    let fp_index = "idx_resources_missing_params_fp".to_string();

    let off = boot_with_mode(&cs, &db_name, IndexBuildMode::Off).await;
    assert_eq!(
        off.wait_for_missing_index_build().await,
        Some(BuildOutcome::Skipped {
            missing: vec![index.clone(), fp_index.clone()]
        })
    );
    let db = off.get_database().await.unwrap();
    let names = resources_index_names(&db).await;
    assert!(
        !names.contains(&index) && !names.contains(&fp_index),
        "{names:?}"
    );
    off.readiness_check()
        .await
        .expect("ready while the missing_params index is absent");

    let tenant = create_tenant("missing-index-mode");
    register_parameters(&off, &tenant).await;
    for (id, values) in [
        ("absent", vec![]),
        ("null", extensions(true)),
        ("present", extensions(false)),
    ] {
        off.create(
            &tenant,
            "Observation",
            observation(id, values),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    }
    for parameter in &PARAMETERS {
        assert_results(
            &off,
            &tenant,
            &missing_query(parameter, true),
            &["absent", "null"],
        )
        .await;
        assert_results(
            &off,
            &tenant,
            &missing_query(parameter, false),
            &["present"],
        )
        .await;
    }

    let inline = boot_with_mode(&cs, &db_name, IndexBuildMode::Inline).await;
    assert_eq!(
        inline.wait_for_missing_index_build().await,
        Some(BuildOutcome::Built {
            created: vec![index.clone(), fp_index.clone()],
            dropped: vec![],
        })
    );
    let names = resources_index_names(&db).await;
    assert!(
        names.contains(&index) && names.contains(&fp_index),
        "{names:?}"
    );
    inline.readiness_check().await.expect("ready once built");
    for parameter in &PARAMETERS {
        assert_results(
            &inline,
            &tenant,
            &missing_query(parameter, true),
            &["absent", "null"],
        )
        .await;
        assert_results(
            &inline,
            &tenant,
            &missing_query(parameter, false),
            &["present"],
        )
        .await;
    }

    // Built and visible: readiness now guards the index searches hint.
    set_test_index_hidden(&inline, "resources", &index, true).await;
    let error = inline
        .readiness_check()
        .await
        .expect_err("a hidden hinted index fails readiness");
    assert!(error.to_string().contains(&index), "{error}");
    set_test_index_hidden(&inline, "resources", &index, false).await;
    inline.readiness_check().await.expect("ready once unhidden");
    db.collection::<Document>("resources")
        .drop_index(&index)
        .await
        .unwrap();
    let error = inline
        .readiness_check()
        .await
        .expect_err("a dropped hinted index fails readiness");
    assert!(error.to_string().contains(&index), "{error}");

    let background = boot_with_mode(&cs, &db_name, IndexBuildMode::Background).await;
    assert!(matches!(
        background.wait_for_missing_index_build().await,
        Some(BuildOutcome::Built { .. })
    ));
    background
        .readiness_check()
        .await
        .expect("rebuilt in the background");
    for parameter in &PARAMETERS {
        assert_results(
            &background,
            &tenant,
            &missing_query(parameter, true),
            &["absent", "null"],
        )
        .await;
    }
}

#[tokio::test]
async fn mongodb_missing_results_survive_searchparameter_create_update_delete() {
    let Some(backend) = create_backend("missing_fp_lifecycle").await else {
        return;
    };
    let tenant = create_tenant("missing-fp-lifecycle");
    register_parameters(&backend, &tenant).await;
    let full = backend
        .create(
            &tenant,
            "Observation",
            observation("lifecycle-full", extensions(false)),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let empty = backend
        .create(
            &tenant,
            "Observation",
            observation("lifecycle-empty", vec![]),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let db = backend.get_database().await.unwrap();
    let stamped = db
        .collection::<Document>("resources")
        .find_one(doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": "Observation",
            "id": "lifecycle-full",
        })
        .await
        .unwrap()
        .unwrap();
    let stamp = stamped.get_str("missing_params_fp").unwrap();
    assert_eq!(stamp.len(), 16, "{stamp}");
    assert!(stamp.bytes().all(|b| b.is_ascii_hexdigit()), "{stamp}");

    let token = &PARAMETERS[1];
    assert_results(
        &backend,
        &tenant,
        &missing_query(token, true),
        &["lifecycle-empty"],
    )
    .await;

    let added = TypedParameter {
        name: "fp-added",
        kind: SearchParamType::Token,
        fhir_type: "token",
        extension_field: "valueCode",
        index_field: "value_token_code",
    };
    backend
        .create(
            &tenant,
            "SearchParameter",
            json!({
                "resourceType": "SearchParameter", "id": added.name,
                "url": format!("urn:hfs:test:{}", added.name),
                "name": added.name, "status": "active", "code": added.name,
                "base": ["Observation"], "type": added.fhir_type,
                "expression": format!(
                    "Observation.extension.where(url = 'urn:hfs:test:{}').value",
                    added.name
                )
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_results(
        &backend,
        &tenant,
        &missing_query(&added, true),
        &["lifecycle-empty", "lifecycle-full"],
    )
    .await;
    assert_results(&backend, &tenant, &missing_query(&added, false), &[]).await;

    let rewritten = backend
        .write_search_entries_page(&tenant, &[full.clone(), empty.clone()])
        .await;
    assert!(rewritten.iter().all(Result::is_ok), "{rewritten:?}");
    assert_missing_metadata(&db, &tenant, "lifecycle-empty", &added, true).await;
    assert_results(
        &backend,
        &tenant,
        &missing_query(&added, true),
        &["lifecycle-empty", "lifecycle-full"],
    )
    .await;

    let current = backend
        .read(&tenant, "SearchParameter", "missing-string")
        .await
        .unwrap()
        .unwrap();
    backend
        .update(
            &tenant,
            &current,
            json!({
                "resourceType": "SearchParameter", "id": "missing-string",
                "url": "urn:hfs:test:missing-string",
                "name": "missing-string", "status": "active", "code": "missing-string",
                "base": ["Observation"], "type": "string",
                "expression":
                    "Observation.extension.where(url = 'urn:hfs:test:missing-string-v2').value"
            }),
        )
        .await
        .unwrap();
    let string = &PARAMETERS[0];
    assert_results(
        &backend,
        &tenant,
        &missing_query(string, true),
        &["lifecycle-empty"],
    )
    .await;
    let rewritten = backend
        .write_search_entries_page(&tenant, &[full.clone(), empty.clone()])
        .await;
    assert!(rewritten.iter().all(Result::is_ok), "{rewritten:?}");
    assert_results(
        &backend,
        &tenant,
        &missing_query(string, true),
        &["lifecycle-empty", "lifecycle-full"],
    )
    .await;

    backend
        .delete(&tenant, "SearchParameter", "fp-added")
        .await
        .unwrap();
    assert_results(
        &backend,
        &tenant,
        &missing_query(&added, true),
        &["lifecycle-empty", "lifecycle-full"],
    )
    .await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn mongodb_missing_metadata_stamped_by_another_registry_is_distrusted() {
    let Some(backend) = create_backend("missing_fp_foreign_stamp").await else {
        return;
    };
    let tenant = create_tenant("missing-fp-foreign");
    register_parameters(&backend, &tenant).await;
    let original = backend
        .create(
            &tenant,
            "Observation",
            observation("node-a", vec![]),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let token = &PARAMETERS[1];
    assert_results(&backend, &tenant, &missing_query(token, true), &["node-a"]).await;

    let db = backend.get_database().await.unwrap();
    db.collection::<Document>("resources")
        .update_one(
            doc! {
                "tenant_id": tenant.tenant_id().as_str(),
                "resource_type": "Observation",
                "id": "node-a",
            },
            doc! { "$set": {
                "missing_params": Vec::<String>::new(),
                "missing_params_fp": "0000000000000000",
            } },
        )
        .await
        .unwrap();
    assert_results(&backend, &tenant, &missing_query(token, true), &["node-a"]).await;
    assert_results(&backend, &tenant, &missing_query(token, false), &[]).await;

    let rewritten = backend
        .write_search_entries_page(&tenant, std::slice::from_ref(&original))
        .await;
    assert!(rewritten.iter().all(Result::is_ok), "{rewritten:?}");
    assert_missing_metadata(&db, &tenant, "node-a", token, true).await;
    assert_results(&backend, &tenant, &missing_query(token, true), &["node-a"]).await;
    db.drop().await.unwrap();
}

const UNSTAMPED_ADDED: TypedParameter = TypedParameter {
    name: "fp-unstamped-added",
    kind: SearchParamType::Token,
    fhir_type: "token",
    extension_field: "valueCode",
    index_field: "value_token_code",
};

async fn add_token_parameter(backend: &MongoBackend, tenant: &TenantContext, name: &str) {
    backend
        .create(
            tenant,
            "SearchParameter",
            json!({
                "resourceType": "SearchParameter", "id": name,
                "url": format!("urn:hfs:test:{name}"),
                "name": name, "status": "active", "code": name,
                "base": ["Observation"], "type": "token",
                "expression": format!("Observation.extension.where(url = 'urn:hfs:test:{name}').value")
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
}

/// Removes the stamps, as arrays written before #1999 (or by a node that
/// predates it) have none.
async fn unstamp(db: &mongodb::Database, tenant: &TenantContext) {
    db.collection::<Document>("resources")
        .update_many(
            doc! { "tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation" },
            doc! { "$unset": { "missing_params_fp": "" } },
        )
        .await
        .unwrap();
}

/// Runs `query` under the profiler and returns the result ids and the
/// profiled operations on `resources` and `search_index`.
async fn profiled_resource_operations(
    backend: &MongoBackend,
    db: &mongodb::Database,
    tenant: &TenantContext,
    query: &SearchQuery,
) -> (Vec<String>, Vec<Document>) {
    use futures::TryStreamExt;
    clear_profile(db).await;
    db.run_command(doc! {"profile": 2}).await.unwrap();
    let result = backend.search(tenant, query).await;
    db.run_command(doc! {"profile": 0}).await.unwrap();
    let ids = result
        .unwrap()
        .resources
        .items
        .iter()
        .map(|row| row.id().to_string())
        .collect();
    let operations = db
        .collection::<Document>("system.profile")
        .find(doc! { "ns": {
            "$in": [format!("{}.resources", db.name()), format!("{}.search_index", db.name())]
        } })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    (ids, operations)
}

/// Whether the search read the value index: the metadata route answers from
/// `resources` alone.
fn reads_search_index(operations: &[Document]) -> bool {
    operations.iter().any(|operation| {
        operation
            .get_str("ns")
            .is_ok_and(|ns| ns.ends_with(".search_index"))
    })
}

/// #1999 on an upgraded tenant: arrays without a stamp predate the
/// fingerprint, so a parameter defined after them is not known absent from
/// them; they answer from the value index until rewritten.
#[tokio::test]
async fn mongodb_missing_metadata_without_a_stamp_is_distrusted() {
    let Some(backend) = create_backend("missing_fp_unstamped").await else {
        return;
    };
    let tenant = create_tenant("missing-fp-unstamped");
    register_parameters(&backend, &tenant).await;
    let mut written = Vec::new();
    for (id, values) in [
        ("unstamped-empty", vec![]),
        ("unstamped-full", extensions(false)),
    ] {
        written.push(
            backend
                .create(
                    &tenant,
                    "Observation",
                    observation(id, values),
                    FhirVersion::default(),
                )
                .await
                .unwrap(),
        );
    }
    let db = backend.get_database().await.unwrap();
    unstamp(&db, &tenant).await;
    add_token_parameter(&backend, &tenant, UNSTAMPED_ADDED.name).await;

    let token = &PARAMETERS[1];
    let expected = ["unstamped-empty", "unstamped-full"];
    let (ids, operations) = profiled_resource_operations(
        &backend,
        &db,
        &tenant,
        &missing_query(&UNSTAMPED_ADDED, true),
    )
    .await;
    assert_eq!(ids, expected);
    assert!(
        reads_search_index(&operations),
        "value-index route: {operations:?}"
    );
    assert_eq!(
        backend
            .search_count(&tenant, &missing_query(&UNSTAMPED_ADDED, true))
            .await
            .unwrap(),
        2
    );
    assert_results(
        &backend,
        &tenant,
        &missing_query(token, true),
        &["unstamped-empty"],
    )
    .await;

    // Rewriting stamps them under the current definitions: the metadata
    // route is back, with the same answers.
    let rewritten = backend.write_search_entries_page(&tenant, &written).await;
    assert!(rewritten.iter().all(Result::is_ok), "{rewritten:?}");
    let (ids, operations) = profiled_resource_operations(
        &backend,
        &db,
        &tenant,
        &missing_query(&UNSTAMPED_ADDED, true),
    )
    .await;
    assert_eq!(ids, expected);
    assert!(
        !reads_search_index(&operations),
        "metadata route: {operations:?}"
    );
    db.drop().await.unwrap();
}

/// After a SearchParameter is added, the arrays stamped under the previous
/// definitions still describe every parameter whose definition did not
/// change, so those keep the metadata route; the new one does not.
#[tokio::test]
async fn mongodb_missing_metadata_of_unchanged_parameters_survives_a_definition_change() {
    let Some(backend) = create_backend("missing_fp_unchanged").await else {
        return;
    };
    let tenant = create_tenant("missing-fp-unchanged");
    register_parameters(&backend, &tenant).await;
    for (id, values) in [
        ("unchanged-empty", vec![]),
        ("unchanged-full", extensions(false)),
    ] {
        backend
            .create(
                &tenant,
                "Observation",
                observation(id, values),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    let db = backend.get_database().await.unwrap();
    let token = &PARAMETERS[1];
    let (ids, operations) =
        profiled_resource_operations(&backend, &db, &tenant, &missing_query(token, true)).await;
    assert_eq!(ids, ["unchanged-empty"]);
    assert!(
        !reads_search_index(&operations),
        "metadata route: {operations:?}"
    );

    add_token_parameter(&backend, &tenant, "fp-unchanged-added").await;
    let added = TypedParameter {
        name: "fp-unchanged-added",
        ..UNSTAMPED_ADDED
    };
    let (ids, operations) =
        profiled_resource_operations(&backend, &db, &tenant, &missing_query(token, true)).await;
    assert_eq!(ids, ["unchanged-empty"]);
    assert!(
        !reads_search_index(&operations),
        "an unchanged parameter keeps the metadata route: {operations:?}"
    );
    let (ids, operations) =
        profiled_resource_operations(&backend, &db, &tenant, &missing_query(&added, true)).await;
    assert_eq!(ids, ["unchanged-empty", "unchanged-full"]);
    assert!(
        reads_search_index(&operations),
        "value-index route: {operations:?}"
    );
    db.drop().await.unwrap();
}

/// The staleness probe is bounded by the distinct stamps it finds, not by
/// the size of the type: an unstamped array is found in one key, each stamp
/// costs one covered seek, and a fully stamped type reads no document.
#[tokio::test]
async fn mongodb_missing_staleness_probe_reads_one_key_per_stamp() {
    let Some(backend) = create_backend("missing_fp_probe_cost").await else {
        return;
    };
    let tenant = create_tenant("missing-fp-probe-cost");
    register_parameters(&backend, &tenant).await;
    for i in 0..50 {
        backend
            .create(
                &tenant,
                "Observation",
                observation(&format!("probe-{i:02}"), vec![]),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    let db = backend.get_database().await.unwrap();
    let token = &PARAMETERS[1];
    let probes = |operations: &[Document]| -> Vec<(i64, i64)> {
        operations
            .iter()
            .filter(|operation| {
                operation.get_document("command").is_ok_and(|command| {
                    command.get_str("hint") == Ok("idx_resources_missing_params_fp")
                })
            })
            .map(|operation| {
                let read = |field| {
                    operation
                        .get_i64(field)
                        .or_else(|_| operation.get_i32(field).map(i64::from))
                        .unwrap_or(0)
                };
                (read("keysExamined"), read("docsExamined"))
            })
            .collect()
    };

    let (_, operations) =
        profiled_resource_operations(&backend, &db, &tenant, &missing_query(token, true)).await;
    // The unstamped seek, then the stamp seeks: the current one, then none.
    let stamped = probes(&operations);
    assert_eq!(stamped.len(), 3, "{operations:?}");
    assert!(
        stamped.iter().all(|&(keys, docs)| keys <= 1 && docs == 0),
        "{stamped:?}"
    );

    unstamp(&db, &tenant).await;
    let (_, operations) =
        profiled_resource_operations(&backend, &db, &tenant, &missing_query(token, true)).await;
    let unstamped = probes(&operations);
    assert_eq!(
        unstamped.len(),
        1,
        "an unstamped array ends the probe: {operations:?}"
    );
    assert!(unstamped[0].0 <= 1 && unstamped[0].1 <= 1, "{unstamped:?}");
    db.drop().await.unwrap();
}

/// #2004: a count that combines a broad positive predicate with metadata
/// `:missing=true` streams the positive index rows in batches. It must not
/// read the larger missing population, and resources without metadata still
/// consult their value index.
#[tokio::test]
async fn mongodb_missing_metadata_count_drives_from_selective_positive_filter() {
    use futures::TryStreamExt;
    let Some(backend) = create_backend("missing_positive_count").await else {
        return;
    };
    let tenant = create_tenant("missing-positive-count");
    register_parameters(&backend, &tenant).await;
    let token = &PARAMETERS[1];
    let present = || {
        vec![json!({
            "url": format!("urn:hfs:test:{}", token.name), "valueCode": "present",
        })]
    };
    // 300 positive matches above the selective-candidate limit; every third
    // lacks the token. 2000 resources of another code all lack it.
    for number in 0..300 {
        let values = if number % 3 == 0 { vec![] } else { present() };
        let mut resource = observation(&format!("common-{number:04}"), values);
        resource["code"] = json!({"coding": [{"code": "common"}]});
        backend
            .create(&tenant, "Observation", resource, FhirVersion::default())
            .await
            .unwrap();
    }
    for number in 0..2000 {
        let mut resource = observation(&format!("other-{number:04}"), vec![]);
        resource["code"] = json!({"coding": [{"code": "other"}]});
        backend
            .create(&tenant, "Observation", resource, FhirVersion::default())
            .await
            .unwrap();
    }
    let db = backend.get_database().await.unwrap();
    // Every fifth positive match predates the metadata.
    for number in (0..300).step_by(5) {
        db.collection::<Document>("resources")
            .update_one(
                doc! {"tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation",
                "id": format!("common-{number:04}")},
                doc! {"$unset": {"missing_params": ""}},
            )
            .await
            .unwrap();
    }
    let code = SearchParameter {
        name: "code".to_owned(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("common")],
        chain: vec![],
        components: vec![],
    };
    for missing in [true, false] {
        let query = SearchQuery::new("Observation")
            .with_parameter(code.clone())
            .with_parameter(missing_condition(token, missing));
        let expected = if missing { 100 } else { 200 };
        clear_profile(&db).await;
        db.run_command(doc! {"profile": 2}).await.unwrap();
        let count = backend.search_count(&tenant, &query).await;
        db.run_command(doc! {"profile": 0}).await.unwrap();
        assert_eq!(count.unwrap(), expected, "missing={missing}");
        if missing {
            let operations: Vec<Document> = db
                .collection("system.profile")
                .find(doc! { "ns": format!("{}.resources", db.name()) })
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            let examined: i64 = operations
                .iter()
                .filter_map(|operation| {
                    operation
                        .get_i64("docsExamined")
                        .or_else(|_| operation.get_i32("docsExamined").map(i64::from))
                        .ok()
                })
                .sum();
            assert!(
                examined < 1000,
                "count must not read the 2100-resource missing population: {examined}"
            );
        }
        let mut summary = query.clone();
        summary.summary = Some(helios_persistence::types::SummaryMode::Count);
        for total in [TotalMode::Accurate, TotalMode::Estimate] {
            summary.total = Some(total);
            let result = backend.search(&tenant, &summary).await.unwrap();
            assert_eq!(result.total, Some(expected), "missing={missing}, {total:?}");
        }
    }
    db.drop().await.unwrap();
}

/// `_id` and `_lastUpdated` are columns of every resource document and
/// `:missing` on them never reads `missing_params`, so they are not recorded
/// even though Synthea-style resources carry no `meta.lastUpdated` (#2006).
#[tokio::test]
async fn mongodb_missing_metadata_omits_resource_columns() {
    let Some(backend) = create_backend("missing_columns").await else {
        return;
    };
    let tenant = create_tenant("missing-columns");
    backend
        .create(
            &tenant,
            "Observation",
            observation("no-meta", vec![]),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let db = backend.get_database().await.unwrap();
    let names = missing_parameters(&db, &tenant, "Observation", "no-meta").await;
    assert!(
        names.iter().any(|name| name == "_tag"),
        "other absent meta parameters are still recorded: {names:?}"
    );
    for column in ["_id", "_lastUpdated"] {
        assert!(
            !names.iter().any(|name| name == column),
            "{column}: {names:?}"
        );
    }
}

/// `missing_metadata: false` (#2006): writes leave no `missing_params` and
/// clear what an enabled writer left, the index is not built, and
/// `:missing` answers exactly as with metadata.
#[tokio::test]
async fn mongodb_missing_metadata_disabled_keeps_missing_results() {
    let Some(connection_string) = shared_mongo::connection_string().await else {
        return;
    };
    let database_name = build_test_database_name("missing_disabled");
    let config = MongoBackendConfig {
        connection_string,
        database_name: database_name.clone(),
        data_dir: Some(repo_data_dir()),
        ..Default::default()
    };
    let tenant = create_tenant("missing-disabled");

    // An enabled writer first, as on a deployment that later opts out.
    let Some(enabled) = build_backend(config.clone()).await else {
        return;
    };
    register_parameters(&enabled, &tenant).await;
    enabled
        .create(
            &tenant,
            "Observation",
            observation("written-enabled", vec![]),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let db = enabled.get_database().await.unwrap();
    assert!(
        !missing_parameters(&db, &tenant, "Observation", "written-enabled")
            .await
            .is_empty()
    );
    db.collection::<Document>("resources")
        .drop_index("idx_resources_missing_params")
        .await
        .unwrap();
    drop(enabled);

    let Some(disabled) = build_backend(MongoBackendConfig {
        missing_metadata: false,
        ..config
    })
    .await
    else {
        return;
    };
    assert!(matches!(
        disabled.wait_for_missing_index_build().await,
        Some(BuildOutcome::Skipped { .. })
    ));
    let listed = db
        .collection::<Document>("resources")
        .list_index_names()
        .await
        .unwrap();
    assert!(
        !listed
            .iter()
            .any(|name| name == "idx_resources_missing_params"),
        "the disabled backend must not build the index: {listed:?}"
    );

    for (id, values) in [
        ("absent", vec![]),
        ("null", extensions(true)),
        ("present", extensions(false)),
    ] {
        disabled
            .create(
                &tenant,
                "Observation",
                observation(id, values),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    let current = disabled
        .read(&tenant, "Observation", "written-enabled")
        .await
        .unwrap()
        .unwrap();
    disabled
        .update(
            &tenant,
            &current,
            observation("written-enabled", extensions(false)),
        )
        .await
        .unwrap();
    let without_metadata = db
        .collection::<Document>("resources")
        .count_documents(doc! {
            "tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation",
            "missing_params": { "$exists": false },
        })
        .await
        .unwrap();
    assert_eq!(
        without_metadata, 4,
        "no Observation may keep missing_params"
    );

    for parameter in &PARAMETERS {
        for missing in [true, false] {
            let expected: &[&str] = if missing {
                &["absent", "null"]
            } else {
                &["present", "written-enabled"]
            };
            let mut query = missing_query(parameter, missing);
            for total in [TotalMode::Accurate, TotalMode::Estimate] {
                query.total = Some(total);
                assert_results(&disabled, &tenant, &query, expected).await;
            }
        }
    }
}

#[tokio::test]
async fn mongodb_missing_conditional_matches_same_bundle_write_on_empty_tenant() {
    let Some(backend) = create_backend("missing_conditional_empty").await else {
        return;
    };
    let tenant = create_tenant("missing-conditional-empty");
    register_parameters(&backend, &tenant).await;
    let bare_put = BundleEntry {
        method: BundleMethod::Put,
        url: "Observation/only".into(),
        resource: Some(observation("only", vec![])),
        if_match: None,
        if_none_match: None,
        if_none_exist: None,
        full_url: None,
        criteria: None,
    };
    let conditional = BundleEntry {
        method: BundleMethod::Post,
        url: "Observation".into(),
        resource: Some(observation("never", extensions(false))),
        if_match: None,
        if_none_match: None,
        if_none_exist: Some("missing-token:missing=true".into()),
        full_url: None,
        criteria: None,
    };
    let result = backend
        .process_transaction(&tenant, vec![bare_put, conditional], FhirVersion::default())
        .await
        .unwrap();
    assert_eq!(result.entries[0].status, 201);
    assert_eq!(
        result.entries[1].status, 200,
        "empty tenant: ifNoneExist absence must match the same-bundle write"
    );

    let spec_tenant = create_tenant("missing-conditional-spec");
    let put = BundleEntry {
        method: BundleMethod::Put,
        url: "Patient/m2".into(),
        resource: Some(json!({"resourceType": "Patient", "id": "m2", "active": true})),
        if_match: None,
        if_none_match: None,
        if_none_exist: None,
        full_url: None,
        criteria: None,
    };
    let conditional = BundleEntry {
        method: BundleMethod::Post,
        url: "Patient".into(),
        resource: Some(json!({"resourceType": "Patient", "gender": "other"})),
        if_match: None,
        if_none_match: None,
        if_none_exist: Some("gender:missing=true".into()),
        full_url: None,
        criteria: None,
    };
    let result = backend
        .process_transaction(&spec_tenant, vec![put, conditional], FhirVersion::default())
        .await
        .unwrap();
    assert_eq!(result.entries[0].status, 201);
    assert_eq!(
        result.entries[1].status, 200,
        "spec param: ifNoneExist absence must match the same-bundle write"
    );
    let db = backend.get_database().await.unwrap();
    db.drop().await.unwrap();
}

/// #2003: `:missing=true` in conditional criteria is applied per candidate
/// batch, after the other criteria narrow it, on both routes: metadata and,
/// with metadata disabled, the value index. The absent resource is written
/// after 600 present ones, so it lies beyond the first batch of the
/// index-driven pager (128) and of the resource scan (512).
#[tokio::test]
async fn mongodb_missing_conditional_criteria_cross_candidate_batches() {
    let Some(connection_string) = shared_mongo::connection_string().await else {
        return;
    };
    for metadata in [true, false] {
        let database_name = build_test_database_name(if metadata {
            "missing_cond_batches_md"
        } else {
            "missing_cond_batches_vi"
        });
        let Some(backend) = build_backend(MongoBackendConfig {
            connection_string: connection_string.clone(),
            database_name,
            data_dir: Some(repo_data_dir()),
            missing_metadata: metadata,
            ..Default::default()
        })
        .await
        else {
            return;
        };
        let tenant = create_tenant("missing-cond-batches");
        register_parameters(&backend, &tenant).await;
        let put = |id: String, values: Vec<Value>| BundleEntry {
            method: BundleMethod::Put,
            url: format!("Observation/{id}"),
            resource: Some(observation(&id, values)),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: None,
            criteria: None,
        };
        for chunk in 0..6 {
            let entries = (0..100)
                .map(|i| put(format!("present-{}", chunk * 100 + i), extensions(false)))
                .collect();
            backend
                .process_transaction(&tenant, entries, FhirVersion::default())
                .await
                .unwrap();
        }
        backend
            .process_transaction(
                &tenant,
                vec![put("absent-a".into(), vec![])],
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let conditional = |criteria: &str| BundleEntry {
            method: BundleMethod::Post,
            url: "Observation".into(),
            resource: Some(observation("conditional", extensions(false))),
            if_match: None,
            if_none_match: None,
            if_none_exist: Some(criteria.to_owned()),
            full_url: None,
            criteria: None,
        };
        let run = |criteria: &'static str| {
            let backend = &backend;
            let tenant = &tenant;
            async move {
                backend
                    .process_transaction(
                        tenant,
                        vec![conditional(criteria)],
                        FhirVersion::default(),
                    )
                    .await
            }
        };
        for criteria in [
            "missing-token:missing=true",
            "code=fixture&missing-token:missing=true",
            "_id=absent-a&missing-token:missing=true",
            "missing-token:missing=true&missing-string:missing=true",
        ] {
            let result = run(criteria).await.unwrap();
            assert_eq!(
                (result.entries[0].status, result.entries[0].reference()),
                (200, Some("Observation/absent-a".to_owned())),
                "metadata={metadata} criteria={criteria}"
            );
        }

        backend
            .process_transaction(
                &tenant,
                vec![put("absent-b".into(), vec![])],
                FhirVersion::default(),
            )
            .await
            .unwrap();
        assert_results(
            &backend,
            &tenant,
            &missing_query(&PARAMETERS[1], true),
            &["absent-a", "absent-b"],
        )
        .await;
        for criteria in [
            "missing-token:missing=true",
            "code=fixture&missing-token:missing=true",
        ] {
            let error = run(criteria).await.unwrap_err();
            assert!(
                format!("{error}").contains("412"),
                "metadata={metadata} criteria={criteria}: {error}"
            );
        }
        // A present resource does not match, so the entry creates.
        let result = run("_id=present-3&missing-token:missing=true")
            .await
            .unwrap();
        assert_eq!(result.entries[0].status, 201, "metadata={metadata}");

        let db = backend.get_database().await.unwrap();
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn mongodb_reindex_reports_parameter_extraction_failures_as_warnings() {
    let Some(backend) = create_backend("missing_param_warnings").await else {
        return;
    };
    let backend = Arc::new(backend);
    let tenant = create_tenant("missing-param-warnings");
    register_parameters(&backend, &tenant).await;
    backend
        .create(
            &tenant,
            "SearchParameter",
            json!({
                "resourceType": "SearchParameter", "id": "failed-index",
                "url": "urn:hfs:test:failed-index", "name": "failed-index",
                "status": "active", "code": "failed-index", "base": ["Observation"],
                "type": "token", "expression": "Observation.identifier.single()",
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    for number in 0..3 {
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation", "id": format!("failing-{number}"),
                    "status": "final", "code": {"text": "example"},
                    "identifier": [{"value": "one"}, {"value": "two"}],
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    backend
        .create(
            &tenant,
            "Observation",
            observation("clean", vec![]),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let op = helios_persistence::search::ReindexOperation::new(
        backend.clone(),
        backend.tenant_registries().clone(),
    );
    let job = op
        .start(
            tenant.clone(),
            helios_persistence::search::ReindexRequest::for_types(["Observation"]),
            None,
        )
        .await
        .unwrap();
    let progress = crate::reindex_id_walk::wait_for_terminal(&op, &job).await;

    assert_eq!(
        progress.status,
        helios_persistence::search::ReindexStatus::Completed,
        "{progress:?}"
    );
    assert!(progress.errors.is_empty(), "{progress:?}");
    assert_eq!(progress.processed_resources, 4, "{progress:?}");
    assert_eq!(progress.parameter_warnings.len(), 1, "{progress:?}");
    let warning = progress.parameter_warnings.get("failed-index").unwrap();
    assert_eq!(warning.resources, 3, "{progress:?}");
    assert!(
        warning.sample_error.contains("single"),
        "{:?}",
        warning.sample_error
    );

    let parameters = progress.to_parameters();
    let warning_part = parameters["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "parameterWarning")
        .expect("status lists the parameter warning");
    let parts = warning_part["part"].as_array().unwrap();
    assert!(
        parts
            .iter()
            .any(|p| p["name"] == "parameter" && p["valueCode"] == "failed-index")
    );
    assert!(
        parts
            .iter()
            .any(|p| p["name"] == "resources" && p["valueInteger"] == 3)
    );

    let db = backend.get_database().await.unwrap();
    for number in 0..3 {
        let names =
            missing_parameters(&db, &tenant, "Observation", &format!("failing-{number}")).await;
        assert!(
            !names.iter().any(|name| name == "failed-index"),
            "{names:?}"
        );
    }
    let rows = db
        .collection::<Document>("search_index")
        .count_documents(doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": "Observation",
            "param_name": "missing-token",
        })
        .await
        .unwrap();
    assert_eq!(rows, 0);
    let status_rows = db
        .collection::<Document>("search_index")
        .count_documents(doc! {
            "tenant_id": tenant.tenant_id().as_str(),
            "resource_type": "Observation",
            "param_name": "status",
        })
        .await
        .unwrap();
    assert_eq!(status_rows, 4, "other parameters' rows are written");
    db.drop().await.unwrap();
}
