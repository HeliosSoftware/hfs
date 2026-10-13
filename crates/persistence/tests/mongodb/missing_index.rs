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
    let outcomes = backend
        .write_search_entries_page(&tenant, &[resource])
        .await;
    assert_eq!(outcomes.len(), 1);
    assert!(
        outcomes[0]
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("failed-index"),
        "{outcomes:?}"
    );
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
    let since = mongodb::bson::DateTime::now();
    let mut ordered = query.clone();
    ordered.sort.clear();
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
        "ts": {"$gte": since},
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
        let since = mongodb::bson::DateTime::now();
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
            "ts": {"$gte": since},
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
async fn mongodb_missing_conditional_transactions_do_not_invert_absence() {
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
        let error = backend
            .process_transaction(
                &tenant,
                vec![put.clone(), delete(true)],
                FhirVersion::default(),
            )
            .await
            .unwrap_err();
        assert!(format!("{error}").contains("missing=true"), "{error:?}");
        assert!(
            backend
                .read(&tenant, "Observation", "present")
                .await
                .unwrap()
                .is_none(),
            "unsupported absence criteria must roll back the preceding write"
        );
        assert!(
            backend
                .read(&tenant, "Observation", "absent")
                .await
                .unwrap()
                .is_some()
        );
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
                .read(&tenant, "Observation", "absent")
                .await
                .unwrap()
                .is_some()
        );
        assert_results(
            &backend,
            &tenant,
            &missing_query(&PARAMETERS[1], true),
            &["absent"],
        )
        .await;
    }
    db.drop().await.unwrap();
}
