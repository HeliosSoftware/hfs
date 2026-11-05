//! Large `_id` filters produced by application-side chain resolution.

use std::collections::BTreeSet;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::search::resolve_chains;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    ChainedParameter, ReverseChainedParameter, SearchModifier, SearchParamType, SearchParameter,
    SearchQuery, SearchValue,
};
use serde_json::json;

fn result_ids(result: &helios_persistence::core::SearchResult) -> BTreeSet<String> {
    result
        .resources
        .items
        .iter()
        .map(|resource| resource.id().to_string())
        .collect()
}

fn large_query(modifier: Option<SearchModifier>) -> SearchQuery {
    // Exceeds both SQLite's 32,766-variable limit and PostgreSQL's
    // 65,535-parameter limit without requiring a large stored fixture.
    let mut values: Vec<SearchValue> = (0..70_000)
        .map(|i| SearchValue::eq(format!("absent-{i}")))
        .collect();
    values.extend(["hit-0", "hit-1", "hit-2"].map(SearchValue::eq));
    SearchQuery::new("Patient")
        .with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier,
            values,
            ..Default::default()
        })
        .with_count(2)
}

pub async fn large_id_set_search_count_cursor_not_and_tenant<S>(backend: &S, tenant_id: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access());
    let other = TenantContext::new(
        TenantId::new(format!("{tenant_id}-other")),
        TenantPermissions::full_access(),
    );
    for id in ["hit-0", "hit-1", "hit-2", "outside"] {
        backend
            .create_or_update(
                &tenant,
                "Patient",
                id,
                json!({"resourceType": "Patient", "id": id}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    backend
        .create_or_update(
            &other,
            "Patient",
            "hit-0",
            json!({"resourceType": "Patient", "id": "hit-0"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let query = large_query(None);
    assert_eq!(backend.search_count(&tenant, &query).await.unwrap(), 3);
    let first = backend.search(&tenant, &query).await.unwrap();
    assert_eq!(first.resources.items.len(), 2);
    let cursor = first.next_cursor().expect("first page has a cursor");
    let second = backend
        .search(&tenant, &query.clone().with_cursor(cursor.clone()))
        .await
        .unwrap();
    let mut ids = result_ids(&first);
    ids.extend(result_ids(&second));
    assert_eq!(
        ids,
        BTreeSet::from(["hit-0", "hit-1", "hit-2"].map(str::to_string))
    );
    assert_eq!(backend.search_count(&other, &query).await.unwrap(), 1);
    assert_eq!(
        result_ids(&backend.search(&other, &query).await.unwrap()),
        BTreeSet::from(["hit-0".to_string()]),
    );

    let excluded = large_query(Some(SearchModifier::Not));
    assert_eq!(backend.search_count(&tenant, &excluded).await.unwrap(), 1);
    assert_eq!(
        result_ids(&backend.search(&tenant, &excluded).await.unwrap()),
        BTreeSet::from(["outside".to_string()]),
    );
}

pub async fn wide_chain_and_nested_has<S>(backend: &S, tenant_id: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access());
    for id in ["wide-chain-p", "wide-chain-decoy"] {
        backend
            .create_or_update(
                &tenant,
                "Patient",
                id,
                json!({"resourceType": "Patient", "id": id}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    // One qualifying Encounter and Observation per id crosses the wide-list
    // threshold at both levels of the nested reverse chain.
    for i in 0..=1_000 {
        let encounter_id = format!("wide-chain-e-{i}");
        backend
            .create_or_update(
                &tenant,
                "Encounter",
                &encounter_id,
                json!({
                    "resourceType": "Encounter",
                    "id": encounter_id,
                    "status": "finished",
                    "subject": {"reference": "Patient/wide-chain-p"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let observation_id = format!("wide-chain-o-{i}");
        backend
            .create_or_update(
                &tenant,
                "Observation",
                &observation_id,
                json!({
                    "resourceType": "Observation",
                    "id": observation_id,
                    "status": "final",
                    "code": {"coding": [{"system": "http://loinc.org", "code": "123"}]},
                    "encounter": {"reference": format!("Encounter/{encounter_id}")}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    let chained = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "encounter".to_string(),
        param_type: SearchParamType::Reference,
        values: vec![SearchValue::eq("finished")],
        chain: vec![ChainedParameter {
            reference_param: "encounter".to_string(),
            target_type: Some("Encounter".to_string()),
            target_param: "status".to_string(),
        }],
        ..Default::default()
    });
    let rewritten = resolve_chains(backend, &tenant, &chained).await.unwrap();
    let ids = rewritten
        .parameters
        .iter()
        .find(|param| param.name == "_id")
        .expect("forward chain produces an id filter");
    assert_eq!(ids.values.len(), 1_001);
    assert_eq!(
        backend.search_count(&tenant, &rewritten).await.unwrap(),
        1_001
    );

    let mut nested = SearchQuery::new("Patient");
    nested.reverse_chains.push(ReverseChainedParameter::nested(
        "Encounter",
        "subject",
        ReverseChainedParameter::terminal(
            "Observation",
            "encounter",
            "code",
            SearchValue::eq("http://loinc.org|123"),
        ),
    ));
    let rewritten = resolve_chains(backend, &tenant, &nested).await.unwrap();
    assert_eq!(
        result_ids(&backend.search(&tenant, &rewritten).await.unwrap()),
        BTreeSet::from(["wide-chain-p".to_string()]),
    );
}
