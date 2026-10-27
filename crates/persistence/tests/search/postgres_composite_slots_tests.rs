//! PostgreSQL representation and strict contained runtime cases for #1407.
use std::sync::Arc;

use helios_fhir::FhirVersion;
use helios_persistence::backends::postgres::PostgresBackend;
use helios_persistence::core::{
    BundleEntry, BundleMethod, BundleProvider, ConditionalStorage, ResourceStorage, SearchProvider,
};
use helios_persistence::search::{ReindexOperation, ReindexRequest, ReindexStatus};
use helios_persistence::types::{
    ContainedMode, SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
};
use serde_json::json;

use super::sql_composite_slots_suite::{
    self as slots, assert_ids, composite, observation, pair_query, tenant,
};

async fn isolated(legacy: bool) -> PostgresBackend {
    super::postgres_integration::hfs1407_backend(legacy).await
}

async fn reindex(backend: &PostgresBackend, tenant: &helios_persistence::tenant::TenantContext) {
    let op = ReindexOperation::new(
        Arc::new(backend.clone()),
        backend.tenant_registries().clone(),
    );
    let request = ReindexRequest::all();
    assert!(!request.clear_existing);
    let id = op.start(tenant.clone(), request, None).await.unwrap();
    for _ in 0..400 {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        let progress = op.get_progress(&id).await.unwrap();
        if progress.status == ReindexStatus::Completed {
            assert!(progress.errors.is_empty(), "{:?}", progress.errors);
            return;
        }
        assert!(
            !matches!(
                progress.status,
                ReindexStatus::Failed | ReindexStatus::Cancelled
            ),
            "{progress:?}"
        );
    }
    panic!("tenant reindex did not complete");
}

#[tokio::test]
async fn postgres_composite_missing_legacy_slots_preserve_presence() {
    let backend = isolated(true).await;
    let scope = tenant("pg-composite-missing-legacy");
    backend
        .create(
            &scope,
            "Observation",
            observation("present", &["A"], &["B"]),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create(
            &scope,
            "Observation",
            json!({"resourceType":"Observation","id":"absent","status":"final"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_eq!(backend.get_client().await.unwrap().execute("UPDATE search_index SET composite_slot=NULL WHERE tenant_id=$1 AND param_name='code-value-concept'", &[&scope.tenant_id().as_str()]).await.unwrap(), 2);
    slots::ordinary_composite_missing_is_presence_only(
        &backend,
        &scope,
        "code-value-concept",
        &[SearchParamType::Token, SearchParamType::Token],
        &["present"],
        &["absent"],
    )
    .await;
    let mut positional = pair_query("no-match$no-match", ContainedMode::Off);
    positional.count = Some(0);
    for result in [
        backend.search(&scope, &positional).await.map(|_| ()),
        backend.search_count(&scope, &positional).await.map(|_| ()),
        backend.search_ids(&scope, &positional).await.map(|_| ()),
    ] {
        assert!(result.unwrap_err().to_string().contains("$reindex"));
    }
}

#[tokio::test]
async fn postgres_composite_missing_denormalized_unsupported_shapes_are_presence_only() {
    let backend = isolated(false).await;
    slots::fresh_unsupported_composite_presence(
        &backend,
        "pg-presence-shapes",
        &[
            (SearchParamType::String, 2),
            (SearchParamType::Date, 2),
            (SearchParamType::Quantity, 2),
            (SearchParamType::Token, 3),
            (SearchParamType::Number, 3),
            (SearchParamType::Reference, 2),
            (SearchParamType::Uri, 2),
            (SearchParamType::Token, 256),
        ],
    )
    .await;
}

#[tokio::test]
async fn postgres_contained_composite_denormalized_slots_and_representable_families() {
    let backend = isolated(false).await;
    slots::repeated_slots_and_entity_pairing(&backend, "denorm-pairing").await;
    slots::custom_unfolded_families(&backend, "denorm-custom", false).await;
    let scope = tenant("denorm-custom");
    let mut top = observation("custom-top", &["Q"], &["R"]);
    top["component"] = json!([{"code":{"coding":[{"code":"A"}]},"valueQuantity":{"value":9},"interpretation":[{"coding":[{"code":"B"}]}]}]);
    top["referenceRange"] = json!([{"low":{"value":1},"high":{"value":9}}]);
    backend
        .create(&scope, "Observation", top, FhirVersion::default())
        .await
        .unwrap();
    for (name, types, positive, negative) in [
        (
            "token-quantity-token",
            vec![
                SearchParamType::Token,
                SearchParamType::Quantity,
                SearchParamType::Token,
            ],
            "A$gt5$B",
            "B$gt5$A",
        ),
        (
            "number-pair",
            vec![SearchParamType::Number, SearchParamType::Number],
            "1$9",
            "9$1",
        ),
    ] {
        for mode in [ContainedMode::Off, ContainedMode::Both] {
            let mut query = SearchQuery::new("Observation");
            query.contained = mode;
            query.parameters.push(composite(name, &types, &[positive]));
            let expected = if mode == ContainedMode::Off {
                vec!["custom-top"]
            } else {
                vec!["custom-container", "custom-top"]
            };
            assert_ids(&backend, &scope, &query, &expected).await;
            query.parameters[0].values = vec![SearchValue::eq(negative)];
            assert_ids(&backend, &scope, &query, &[]).await;
        }
    }
    backend.create(&scope,"SearchParameter",json!({"resourceType":"SearchParameter","id":"triple-unsupported","url":"http://example.org/1407/triple-unsupported","name":"triple-unsupported","status":"active","code":"triple-unsupported","base":["Observation"],"type":"composite","expression":"Observation.component","component":[{"definition":"http://example.org/1407/axis-token","expression":"code"},{"definition":"http://example.org/1407/axis-token","expression":"interpretation"},{"definition":"http://example.org/1407/axis-token","expression":"valueCodeableConcept"}]}),FhirVersion::default()).await.unwrap();
    for (name, family, count) in [
        ("string-pair", SearchParamType::String, 2),
        ("date-pair", SearchParamType::Date, 2),
        ("quantity-pair", SearchParamType::Quantity, 2),
        ("triple-unsupported", SearchParamType::Token, 3),
    ] {
        for mode in [ContainedMode::Off, ContainedMode::Both] {
            let mut query = SearchQuery::new("Observation");
            query.contained = mode;
            query.count = Some(0);
            let raw = match family {
                SearchParamType::Date => "2020-01-01$2021-01-01",
                SearchParamType::Quantity => "1$9",
                SearchParamType::Token => "A$B$C",
                _ => "Alpha$Beta",
            };
            query
                .parameters
                .push(composite(name, &vec![family; count], &[raw]));
            for result in [
                backend.search(&scope, &query).await.map(|_| ()),
                backend.search_count(&scope, &query).await.map(|_| ()),
                backend.search_ids(&scope, &query).await.map(|_| ()),
            ] {
                let message = result.unwrap_err().to_string();
                assert!(
                    message.contains(name) && message.contains("unsupported repeated"),
                    "{message}"
                );
                assert!(!message.contains("$reindex"), "{message}");
            }
        }
    }
    // The IDs path must validate before its builder's per-family u8 counter.
    let value = vec!["A"; 256].join("$");
    let mut oversized = SearchQuery::new("Observation");
    oversized.count = Some(0);
    oversized.parameters.push(composite(
        "triple-unsupported",
        &vec![SearchParamType::Token; 256],
        &[&value],
    ));
    for result in [
        backend.search(&scope, &oversized).await.map(|_| ()),
        backend.search_count(&scope, &oversized).await.map(|_| ()),
        backend.search_ids(&scope, &oversized).await.map(|_| ()),
    ] {
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("triple-unsupported") && message.contains("unsupported repeated"),
            "{message}"
        );
        assert!(!message.contains("$reindex"));
    }
}

#[tokio::test]
async fn postgres_contained_composite_legacy_slotted_families_and_third_token() {
    let backend = isolated(true).await;
    slots::repeated_slots_and_entity_pairing(&backend, "legacy-pairing").await;
    slots::custom_unfolded_families(&backend, "legacy-custom", true).await;
    let scope = tenant("legacy-third");
    for (code, ty, expression) in [("triple-axis", "token", "Observation.code")] {
        backend.create(&scope,"SearchParameter",json!({"resourceType":"SearchParameter","id":code,"url":format!("http://example.org/{code}"),"name":code,"status":"active","code":code,"base":["Observation"],"type":ty,"expression":expression}),FhirVersion::default()).await.unwrap();
    }
    backend.create(&scope,"SearchParameter",json!({"resourceType":"SearchParameter","id":"triple","url":"http://example.org/triple","name":"triple","status":"active","code":"triple","base":["Observation"],"type":"composite","expression":"Observation.component","component":[{"definition":"http://example.org/triple-axis","expression":"code"},{"definition":"http://example.org/triple-axis","expression":"interpretation"},{"definition":"http://example.org/triple-axis","expression":"valueCodeableConcept"}]}),FhirVersion::default()).await.unwrap();
    let mut resource = observation("three", &["Q"], &["R"]);
    resource["component"] = json!([{"code":{"coding":[{"code":"A"}]},"interpretation":[{"coding":[{"code":"B"}]}],"valueCodeableConcept":{"coding":[{"code":"C"}]}}]);
    backend
        .create(
            &scope,
            "Observation",
            resource.clone(),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    resource["id"] = json!("child");
    backend
        .create(
            &scope,
            "Patient",
            json!({"resourceType":"Patient","id":"container","contained":[resource]}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    for mode in [ContainedMode::Off, ContainedMode::On, ContainedMode::Both] {
        let mut query = SearchQuery::new("Observation");
        query.contained = mode;
        query.parameters.push(composite(
            "triple",
            &[SearchParamType::Token; 3],
            &["A$B$C"],
        ));
        let expected = match mode {
            ContainedMode::Off => vec!["three"],
            ContainedMode::On => vec!["container"],
            ContainedMode::Both => vec!["container", "three"],
        };
        assert_ids(&backend, &scope, &query, &expected).await;
        query.parameters[0].values = vec![SearchValue::eq("C$B$A")];
        assert_ids(&backend, &scope, &query, &[]).await;
    }
}

#[tokio::test]
async fn postgres_composite_legacy_folded_number_and_foreign_payloads() {
    let denorm = isolated(false).await;
    slots::custom_unfolded_families(&denorm, "legacy-folded", false).await;
    let scope = tenant("legacy-folded");
    let container = denorm
        .read(&scope, "Patient", "custom-container")
        .await
        .unwrap()
        .unwrap();
    let mut resource = container.content()["contained"][0].clone();
    resource["id"] = json!("folded");
    denorm
        .create(
            &scope,
            "Observation",
            resource.clone(),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let client = denorm.get_client().await.unwrap();
    let stored:bool=client.query_one("SELECT EXISTS(SELECT 1 FROM search_index WHERE tenant_id=$1 AND resource_id='folded' AND param_name='number-pair' AND composite_slot IS NULL AND value_number=1 AND value_number_2=9)",&[&scope.tenant_id().as_str()]).await.unwrap().get(0);
    assert!(
        stored,
        "folded Number_2 must come from the production writer"
    );
    client
        .execute("UPDATE search_index_layout SET layout='legacy'", &[])
        .await
        .unwrap();
    drop(client);
    let config = denorm.config().clone();
    drop(denorm);
    let backend = PostgresBackend::new(config).await.unwrap();
    backend.init_schema().await.unwrap();
    resource["id"] = json!("slotted");
    backend
        .create(&scope, "Observation", resource, FhirVersion::default())
        .await
        .unwrap();
    for (name, types, positive, negative) in [
        (
            "number-pair",
            vec![SearchParamType::Number, SearchParamType::Number],
            "1$9",
            "9$1",
        ),
        (
            "token-quantity-token",
            vec![
                SearchParamType::Token,
                SearchParamType::Quantity,
                SearchParamType::Token,
            ],
            "A$gt5$B",
            "B$gt5$A",
        ),
    ] {
        for mode in [ContainedMode::Off, ContainedMode::On, ContainedMode::Both] {
            let mut query = SearchQuery::new("Observation");
            query.contained = mode;
            query.parameters.push(composite(name, &types, &[positive]));
            let expected = match mode {
                ContainedMode::Off => vec!["folded", "slotted"],
                ContainedMode::On => vec!["custom-container"],
                ContainedMode::Both => vec!["custom-container", "folded", "slotted"],
            };
            assert_ids(&backend, &scope, &query, &expected).await;
            query.parameters[0].values = vec![SearchValue::eq(negative)];
            assert_ids(&backend, &scope, &query, &[]).await;
        }
    }
    backend.get_client().await.unwrap().execute("UPDATE search_index SET composite_slot=NULL,value_token_code_2='foreign',value_number_2=NULL WHERE tenant_id=$1 AND resource_id='slotted' AND param_name='number-pair'",&[&scope.tenant_id().as_str()]).await.unwrap();
    let mut query = SearchQuery::new("Observation");
    query.count = Some(0);
    query.parameters.push(composite(
        "number-pair",
        &[SearchParamType::Number; 2],
        &["100$200"],
    ));
    for result in [
        backend.search(&scope, &query).await.map(|_| ()),
        backend.search_count(&scope, &query).await.map(|_| ()),
        backend.search_ids(&scope, &query).await.map(|_| ()),
    ] {
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("number-pair") && message.contains("$reindex"),
            "{message}"
        );
    }
    reindex(&backend, &scope).await;
    query.count = None;
    query.parameters[0].values = vec![SearchValue::eq("1$9")];
    assert_ids(&backend, &scope, &query, &["folded", "slotted"]).await;
}

#[tokio::test]
async fn postgres_composite_legacy_scopes_reindex_and_conditional_transactions() {
    let backend = isolated(true).await;
    for (label, top_old, contained_old) in [
        ("top", true, false),
        ("contained", false, true),
        ("both", true, true),
    ] {
        let scope = tenant(&format!("legacy-{label}"));
        backend
            .create(
                &scope,
                "Observation",
                observation("original", &["A"], &["B"]),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend.create(&scope,"Patient",json!({"resourceType":"Patient","id":"container","contained":[observation("inside",&["A"],&["B"])]}),FhirVersion::default()).await.unwrap();
        backend.get_client().await.unwrap().execute("UPDATE search_index SET composite_slot=NULL WHERE tenant_id=$1 AND param_name='code-value-concept' AND ((is_contained=FALSE AND $2) OR (is_contained=TRUE AND $3))", &[&scope.tenant_id().as_str(),&top_old,&contained_old]).await.unwrap();
        for mode in [ContainedMode::Off, ContainedMode::On, ContainedMode::Both] {
            let blocked = match mode {
                ContainedMode::Off => top_old,
                ContainedMode::On => contained_old,
                ContainedMode::Both => top_old || contained_old,
            };
            let mut query = pair_query("no-match$no-match", mode);
            query.count = Some(0);
            for result in [
                backend.search(&scope, &query).await.map(|_| ()),
                backend.search_count(&scope, &query).await.map(|_| ()),
                backend.search_ids(&scope, &query).await.map(|_| ()),
            ] {
                if blocked {
                    let message = result.unwrap_err().to_string();
                    assert!(
                        message.contains("code-value-concept") && message.contains("$reindex"),
                        "{message}"
                    );
                } else {
                    result.unwrap();
                }
            }
        }
        let conditional = |value: &str| BundleEntry {
            method: BundleMethod::Post,
            url: "Observation".into(),
            resource: Some(observation("incoming", &["A"], &["B"])),
            if_none_exist: Some(format!("code-value-concept={value}")),
            ..Default::default()
        };
        if top_old {
            let error = backend
                .conditional_create(
                    &scope,
                    "Observation",
                    observation("conditional", &["A"], &["B"]),
                    "code-value-concept=A$B",
                    FhirVersion::default(),
                )
                .await
                .unwrap_err();
            assert!(error.to_string().contains("$reindex"), "{error}");
            let error = backend
                .process_transaction(
                    &scope,
                    vec![
                        BundleEntry {
                            method: BundleMethod::Post,
                            url: "Patient".into(),
                            resource: Some(json!({"resourceType":"Patient","id":"rolled-back"})),
                            ..Default::default()
                        },
                        conditional("A$B"),
                    ],
                    FhirVersion::default(),
                )
                .await
                .unwrap_err();
            assert!(error.to_string().contains("$reindex"), "{error}");
            assert!(
                backend
                    .read(&scope, "Patient", "rolled-back")
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(
                backend
                    .read(&scope, "Observation", "conditional")
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        reindex(&backend, &scope).await;
        let remaining:i64=backend.get_client().await.unwrap().query_one("SELECT count(*) FROM search_index WHERE tenant_id=$1 AND param_name='code-value-concept' AND composite_group IS NOT NULL AND composite_slot IS NULL",&[&scope.tenant_id().as_str()]).await.unwrap().get(0);
        assert_eq!(remaining, 0);
        for mode in [ContainedMode::Off, ContainedMode::On, ContainedMode::Both] {
            let expected = match mode {
                ContainedMode::Off => vec!["original"],
                ContainedMode::On => vec!["container"],
                ContainedMode::Both => vec!["container", "original"],
            };
            assert_ids(&backend, &scope, &pair_query("A$B", mode), &expected).await;
            assert_ids(&backend, &scope, &pair_query("B$A", mode), &[]).await;
        }
        let result = backend
            .process_transaction(&scope, vec![conditional("A$B")], FhirVersion::default())
            .await
            .unwrap();
        assert_eq!(result.entries[0].status, 200);
        assert!(
            result.entries[0]
                .location
                .as_deref()
                .unwrap()
                .contains("Observation/original")
        );
        let result = backend
            .process_transaction(&scope, vec![conditional("B$A")], FhirVersion::default())
            .await
            .unwrap();
        assert_eq!(result.entries[0].status, 201);
        eprintln!(
            "{label}: full-tenant reindex clear_existing=false restored slots; A$B reused, B$A created"
        );
    }
    let clean = tenant("legacy-clean");
    backend
        .create(
            &clean,
            "Observation",
            observation("clean", &["A"], &["B"]),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let client = backend.get_client().await.unwrap();
    client.batch_execute("INSERT INTO search_index (tenant_id,resource_type,resource_id,param_name,composite_group,value_token_code) VALUES ('legacy-clean','Patient','other','code-value-concept',1,'A'),('legacy-clean','Observation','other','another-param',1,'A'),('legacy-clean','Observation','other','code-value-concept',NULL,'A'),('other-tenant','Observation','other','code-value-concept',1,'A'); INSERT INTO search_index (tenant_id,resource_type,resource_id,param_name,composite_group,value_token_code,is_contained,contained_type,contained_local_id) VALUES ('legacy-clean','Patient','other','code-value-concept',1,'A',TRUE,'Patient','other'),('legacy-clean','Patient','other','another-param',1,'A',TRUE,'Observation','other'),('other-tenant','Patient','other','code-value-concept',1,'A',TRUE,'Observation','other')").await.unwrap();
    drop(client);
    for mode in [ContainedMode::Off, ContainedMode::On, ContainedMode::Both] {
        assert_ids(
            &backend,
            &clean,
            &pair_query("A$B", mode),
            if mode == ContainedMode::On {
                &[]
            } else {
                &["clean"]
            },
        )
        .await;
    }
    let mut mixed = observation("mixed", &["A"], &["B"]);
    mixed["valueQuantity"] = json!({"value":9});
    backend
        .create(&clean, "Observation", mixed, FhirVersion::default())
        .await
        .unwrap();
    backend.get_client().await.unwrap().execute("UPDATE search_index SET composite_slot=NULL WHERE tenant_id=$1 AND param_name='code-value-quantity'",&[&clean.tenant_id().as_str()]).await.unwrap();
    let query = SearchQuery::new("Observation").with_parameter(composite(
        "code-value-quantity",
        &[SearchParamType::Token, SearchParamType::Quantity],
        &["A$gt5"],
    ));
    assert_ids(&backend, &clean, &query, &["mixed"]).await;
}

#[tokio::test]
async fn postgres_contained_composite_denormalized_legacy_scope_requires_reindex() {
    let backend = isolated(false).await;
    let scope = tenant("denorm-old-contained");
    backend
        .create(
            &scope,
            "Observation",
            observation("folded", &["A"], &["B"]),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend.create(&scope,"Patient",json!({"resourceType":"Patient","id":"container","contained":[observation("inside",&["A"],&["B"])]}),FhirVersion::default()).await.unwrap();
    backend.get_client().await.unwrap().execute("UPDATE search_index SET composite_slot=NULL WHERE tenant_id=$1 AND is_contained=TRUE AND param_name='code-value-concept'",&[&scope.tenant_id().as_str()]).await.unwrap();
    assert_ids(
        &backend,
        &scope,
        &pair_query("A$B", ContainedMode::Off),
        &["folded"],
    )
    .await;
    for mode in [ContainedMode::On, ContainedMode::Both] {
        let mut query = pair_query("no-match$no-match", mode);
        query.count = Some(0);
        for result in [
            backend.search(&scope, &query).await.map(|_| ()),
            backend.search_count(&scope, &query).await.map(|_| ()),
            backend.search_ids(&scope, &query).await.map(|_| ()),
        ] {
            assert!(result.unwrap_err().to_string().contains("$reindex"));
        }
    }
    reindex(&backend, &scope).await;
    assert_ids(
        &backend,
        &scope,
        &pair_query("A$B", ContainedMode::Both),
        &["container", "folded"],
    )
    .await;
}

#[tokio::test]
async fn postgres_contained_modifiers_reference_and_uri_are_strict() {
    let backend = isolated(false).await;
    let scope = tenant("strict-modifiers");
    backend.create(&scope,"Patient",json!({"resourceType":"Patient","id":"target","identifier":[{"system":"http://example.org/mrn","value":"42"}]}),FhirVersion::default()).await.unwrap();
    let patient = |id: &str, family: &str, kind: &str| json!({"resourceType":"Patient","id":id,"name":[{"family":family}],"identifier":[{"system":"http://example.org/mrn","value":"42","type":{"coding":[{"system":"http://types","code":kind}]}}]});
    let obs = |id: &str, display: &str, reference: &str| json!({"resourceType":"Observation","id":id,"status":"final","code":{"coding":[{"system":"http://codes","code":"X","display":display}],"text":display},"subject":{"reference":reference}});
    for (id, children) in [
        (
            "positive",
            vec![
                patient("patient", "Alpha Beta", "MR"),
                obs("observation", "Alpha Marker", "Patient/target"),
                json!({"resourceType":"ValueSet","id":"uri","status":"active","url":"http://example.org/sets/root/child"}),
            ],
        ),
        (
            "negative",
            vec![
                patient("patient", "Gamma", "OTHER"),
                obs("observation", "Beta", "Practitioner/target"),
                json!({"resourceType":"ValueSet","id":"uri","status":"active","url":"http://other.org/sets"}),
            ],
        ),
        (
            "absolute",
            vec![obs(
                "observation",
                "Absolute",
                "http://example.org/fhir/Patient/target",
            )],
        ),
        (
            "parent-uri",
            vec![
                json!({"resourceType":"ValueSet","id":"uri","status":"active","url":"http://example.org/sets/root"}),
            ],
        ),
    ] {
        backend.create(&scope,"DiagnosticReport",json!({"resourceType":"DiagnosticReport","id":id,"status":"final","code":{"text":"container"},"contained":children}),FhirVersion::default()).await.unwrap();
    }
    let parameter = |name: &str, kind, modifier, value: &str| SearchParameter {
        name: name.into(),
        param_type: kind,
        modifier,
        values: vec![SearchValue::eq(value)],
        ..Default::default()
    };
    let cases = vec![
        (
            "Patient",
            parameter(
                "name",
                SearchParamType::String,
                Some(SearchModifier::Exact),
                "Alpha Beta",
            ),
            vec!["positive"],
        ),
        (
            "Patient",
            parameter(
                "name",
                SearchParamType::String,
                Some(SearchModifier::Contains),
                "ha B",
            ),
            vec!["positive"],
        ),
        (
            "Patient",
            parameter(
                "name",
                SearchParamType::String,
                Some(SearchModifier::Text),
                "alpha",
            ),
            vec!["positive"],
        ),
        (
            "Patient",
            parameter(
                "name",
                SearchParamType::String,
                Some(SearchModifier::Exact),
                "alpha beta",
            ),
            vec![],
        ),
        (
            "Observation",
            parameter(
                "code",
                SearchParamType::Token,
                Some(SearchModifier::Text),
                "Marker",
            ),
            vec!["positive"],
        ),
        (
            "Observation",
            parameter(
                "code",
                SearchParamType::Token,
                Some(SearchModifier::CodeText),
                "Alpha",
            ),
            vec!["positive"],
        ),
        (
            "Observation",
            parameter(
                "code",
                SearchParamType::Token,
                Some(SearchModifier::CodeText),
                "Marker",
            ),
            vec![],
        ),
        (
            "Patient",
            parameter(
                "identifier",
                SearchParamType::Token,
                Some(SearchModifier::OfType),
                "http://types|MR|42",
            ),
            vec!["positive"],
        ),
        (
            "Observation",
            parameter(
                "subject",
                SearchParamType::Reference,
                Some(SearchModifier::Type("Patient".into())),
                "target",
            ),
            vec!["positive"],
        ),
        (
            "Observation",
            parameter(
                "subject",
                SearchParamType::Reference,
                Some(SearchModifier::Identifier),
                "http://example.org/mrn|42",
            ),
            vec!["positive"],
        ),
        (
            "Observation",
            parameter(
                "subject",
                SearchParamType::Reference,
                None,
                "http://example.org/fhir/Patient/target",
            ),
            vec!["absolute"],
        ),
        (
            "ValueSet",
            parameter(
                "url",
                SearchParamType::Uri,
                Some(SearchModifier::Contains),
                "root/child",
            ),
            vec!["positive"],
        ),
        (
            "ValueSet",
            parameter(
                "url",
                SearchParamType::Uri,
                Some(SearchModifier::Above),
                "http://example.org/sets/root/child",
            ),
            vec!["parent-uri", "positive"],
        ),
        (
            "ValueSet",
            parameter(
                "url",
                SearchParamType::Uri,
                Some(SearchModifier::Below),
                "http://example.org/sets/root",
            ),
            vec!["parent-uri", "positive"],
        ),
        (
            "ValueSet",
            parameter(
                "url",
                SearchParamType::Uri,
                Some(SearchModifier::Contains),
                "absent",
            ),
            vec![],
        ),
    ];
    let stored:bool=backend.get_client().await.unwrap().query_one("SELECT EXISTS(SELECT 1 FROM search_index WHERE tenant_id=$1 AND is_contained=TRUE AND param_name='subject' AND value_reference='http://example.org/fhir/Patient/target')",&[&scope.tenant_id().as_str()]).await.unwrap().get(0);
    assert!(
        stored,
        "absolute reference must be stored as an absolute URL"
    );
    for (resource_type, parameter, expected) in cases {
        let mut query = SearchQuery::new(resource_type);
        query.contained = ContainedMode::On;
        query.parameters.push(parameter);
        assert_ids(&backend, &scope, &query, &expected).await;
    }
    let mut missing = SearchQuery::new("Patient");
    missing.contained = ContainedMode::On;
    missing.parameters.push(parameter(
        "name",
        SearchParamType::String,
        Some(SearchModifier::Missing),
        "true",
    ));
    for result in [
        backend.search(&scope, &missing).await.map(|_| ()),
        backend.search_count(&scope, &missing).await.map(|_| ()),
        backend.search_ids(&scope, &missing).await.map(|_| ()),
    ] {
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("name")
                && message.contains(":missing")
                && message.contains("_contained"),
            "{message}"
        );
    }
}
