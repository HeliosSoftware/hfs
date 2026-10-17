//! Strict SQL slot scenarios. Vectors deliberately preserve duplicate local IDs.
#![allow(dead_code)]

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    CompositeSearchComponent, ContainedMode, ContainedReturn, SearchModifier, SearchParamType,
    SearchParameter, SearchQuery, SearchValue, TotalMode,
};
use serde_json::{Value, json};

pub fn tenant(name: &str) -> TenantContext {
    TenantContext::new(TenantId::new(name), TenantPermissions::full_access())
}

pub fn observation(id: &str, codes: &[&str], values: &[&str]) -> Value {
    json!({"resourceType":"Observation", "id":id, "status":"final",
        "code":{"coding":codes.iter().map(|code| json!({"code":code})).collect::<Vec<_>>()},
        "valueCodeableConcept":{"coding":values.iter().map(|code| json!({"code":code})).collect::<Vec<_>>()}})
}

pub fn composite(name: &str, types: &[SearchParamType], values: &[&str]) -> SearchParameter {
    SearchParameter {
        name: name.into(),
        param_type: SearchParamType::Composite,
        values: values.iter().map(|value| SearchValue::eq(*value)).collect(),
        components: types
            .iter()
            .enumerate()
            .map(|(position, param_type)| CompositeSearchComponent {
                param_type: *param_type,
                param_name: format!("axis-{position}"),
            })
            .collect(),
        ..Default::default()
    }
}

pub fn pair_query(value: &str, mode: ContainedMode) -> SearchQuery {
    let mut query = SearchQuery::new("Observation");
    query.contained = mode;
    query.parameters.push(composite(
        "code-value-concept",
        &[SearchParamType::Token, SearchParamType::Token],
        &[value],
    ));
    query
}

/// Presence depends on whether a parameter has an index entry, never on the
/// order or representability of its declared composite components.
pub async fn ordinary_composite_missing_is_presence_only<S: SearchProvider>(
    backend: &S,
    tenant: &TenantContext,
    name: &str,
    families: &[SearchParamType],
    present: &[&str],
    absent: &[&str],
) {
    for (missing, expected) in [(false, present), (true, absent)] {
        let mut query = SearchQuery::new("Observation");
        query.parameters.push(composite(
            name,
            families,
            &[if missing { "true" } else { "false" }],
        ));
        query.parameters[0].modifier = Some(SearchModifier::Missing);
        assert_ids(backend, tenant, &query, expected).await;
        query.count = Some(0);
        query.total = Some(TotalMode::Accurate);
        let empty = backend.search(tenant, &query).await.unwrap();
        assert!(empty.resources.items.is_empty(), "{query:?}");
        assert_eq!(empty.total, Some(expected.len() as u64), "{query:?}");
        assert_eq!(
            backend.search_count(tenant, &query).await.unwrap(),
            expected.len() as u64
        );
        assert!(
            backend
                .search_ids(tenant, &query)
                .await
                .unwrap()
                .items
                .is_empty()
        );
        for mode in [ContainedMode::On, ContainedMode::Both] {
            query.contained = mode;
            for result in [
                backend.search(tenant, &query).await.map(|_| ()),
                backend.search_count(tenant, &query).await.map(|_| ()),
                backend.search_ids(tenant, &query).await.map(|_| ()),
            ] {
                let message = result.unwrap_err().to_string();
                assert!(
                    message.contains(name) && message.contains("modifier"),
                    "{message}"
                );
                assert!(!message.contains("$reindex"), "{message}");
            }
        }
    }
}

/// Real fresh definitions with no matching index entries. Their positional
/// shapes are unsupported by the selected reader, but presence is still known.
pub async fn fresh_unsupported_composite_presence<S: ResourceStorage + SearchProvider>(
    backend: &S,
    name: &str,
    declarations: &[(SearchParamType, usize)],
) {
    let tenant = tenant(name);
    for id in ["first", "second"] {
        backend
            .create(
                &tenant,
                "Observation",
                observation(id, &["A"], &["B"]),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    for (position, (family, count)) in declarations.iter().enumerate() {
        let ty = match family {
            SearchParamType::Token => "token",
            SearchParamType::Number => "number",
            SearchParamType::String => "string",
            SearchParamType::Quantity => "quantity",
            SearchParamType::Date => "date",
            SearchParamType::Reference => "reference",
            SearchParamType::Uri => "uri",
            _ => panic!("unexpected presence fixture family"),
        };
        let axis = format!("presence-axis-{position}");
        let composite_name = format!("presence-composite-{position}");
        let url = format!("http://example.org/1407/{name}/{axis}");
        backend.create(&tenant, "SearchParameter", json!({"resourceType":"SearchParameter","id":axis,"url":url,"name":axis,"status":"active","code":axis,"base":["Observation"],"type":ty,"expression":"Observation.value"}), FhirVersion::default()).await.unwrap();
        backend.create(&tenant, "SearchParameter", json!({"resourceType":"SearchParameter","id":composite_name,"url":format!("http://example.org/1407/{name}/{composite_name}"),"name":composite_name,"status":"active","code":composite_name,"base":["Observation"],"type":"composite","expression":"Observation","component":(0..*count).map(|_|json!({"definition":url,"expression":"value"})).collect::<Vec<_>>()}), FhirVersion::default()).await.unwrap();
        ordinary_composite_missing_is_presence_only(
            backend,
            &tenant,
            &composite_name,
            &vec![*family; *count],
            &[],
            &["first", "second"],
        )
        .await;
    }
}

pub async fn assert_ids<S: SearchProvider>(
    backend: &S,
    tenant: &TenantContext,
    query: &SearchQuery,
    expected: &[&str],
) {
    let mut query = query.clone();
    query.count = Some(100);
    query.offset = None;
    query.total = Some(TotalMode::Accurate);
    let result = backend
        .search(tenant, &query)
        .await
        .expect("strict composite search");
    let mut ids = result
        .resources
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect::<Vec<_>>();
    ids.sort();
    let mut expected = expected.iter().map(|v| v.to_string()).collect::<Vec<_>>();
    expected.sort();
    assert_eq!(ids, expected, "{query:?}");
    assert_eq!(result.resources.items.len(), expected.len());
    assert_eq!(result.total, Some(expected.len() as u64));
    let counted = backend.search_count(tenant, &query).await.unwrap();
    assert_eq!(counted, expected.len() as u64);
    let criteria = query
        .parameters
        .iter()
        .map(|parameter| {
            (
                &parameter.name,
                parameter
                    .values
                    .iter()
                    .map(|value| value.value.as_str())
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    eprintln!(
        "[sql-composite-slots] {} {:?}/{:?} {criteria:?}: ids={ids:?} total={:?} count={counted}",
        tenant.tenant_id().as_str(),
        query.contained,
        query.contained_return,
        result.total
    );
    let mut id_query = query.clone();
    id_query.total = None;
    let mut ids = backend.search_ids(tenant, &id_query).await.unwrap().items;
    ids.sort();
    assert_eq!(ids, expected, "ID route {query:?}");

    if query.contained != ContainedMode::Off {
        let mut pages = Vec::new();
        for offset in 0..=expected.len() + 1 {
            query.count = Some(1);
            query.offset = Some(offset as u32);
            let page = backend.search(tenant, &query).await.unwrap();
            assert_eq!(page.total, Some(expected.len() as u64));
            assert_eq!(
                page.resources.items.len(),
                usize::from(offset < expected.len())
            );
            pages.extend(page.resources.items.into_iter().map(|r| r.id().to_string()));
        }
        pages.sort();
        assert_eq!(pages, expected, "complete page traversal");
        eprintln!(
            "[sql-composite-slots] offset pages={pages:?}; count-zero page empty, total={counted}"
        );
        query.count = Some(0);
        query.offset = None;
        let empty = backend.search(tenant, &query).await.unwrap();
        assert!(empty.resources.items.is_empty());
        assert_eq!(empty.total, Some(expected.len() as u64));
    }
}

pub async fn repeated_slots_and_entity_pairing<S: ResourceStorage + SearchProvider>(
    backend: &S,
    name: &str,
) {
    use ContainedMode::{Both, Off, On};
    let tenant = tenant(name);
    let top = observation("top", &["A", "X"], &["B", "Y"]);
    backend
        .create(&tenant, "Observation", top.clone(), FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(
            &tenant,
            "Observation",
            observation("equal", &["A"], &["A"]),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let mut same = observation("same", &["A"], &["B"]);
    same["contained"] = json!([
        observation("reverse", &["B"], &["A"]),
        observation("same", &["A"], &["B"])
    ]);
    backend
        .create(&tenant, "Observation", same, FhirVersion::default())
        .await
        .unwrap();
    let mut decoy = observation("decoy", &["C"], &["D"]);
    decoy["contained"] = json!([observation("child", &["A"], &["B"])]);
    backend
        .create(&tenant, "Observation", decoy, FhirVersion::default())
        .await
        .unwrap();
    backend.create(&tenant,"Patient",json!({"resourceType":"Patient","id":"container","contained":[top,observation("equal-child",&["A"],&["A"])]}),FhirVersion::default()).await.unwrap();
    backend.create(&tenant,"Patient",json!({"resourceType":"Patient","id":"siblings","contained":[observation("one",&["A"],&["Z"]),observation("two",&["Z"],&["B"]),observation("incomplete",&["A"],&[])]}),FhirVersion::default()).await.unwrap();
    let other = crate_tenant(name);
    backend
        .create(
            &other,
            "Observation",
            observation("outsider", &["A"], &["B"]),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    for (mode, returns, value, ids) in [
        (Off, ContainedReturn::Container, "A$B", vec!["same", "top"]),
        (Off, ContainedReturn::Container, "B$A", vec![]),
        (Off, ContainedReturn::Container, "A$A", vec!["equal"]),
        (
            On,
            ContainedReturn::Container,
            "A$B",
            vec!["container", "decoy", "same"],
        ),
        (
            On,
            ContainedReturn::Contained,
            "A$B",
            vec!["child", "same", "top"],
        ),
        (
            Both,
            ContainedReturn::Container,
            "A$B",
            vec!["container", "decoy", "same", "top"],
        ),
        (
            Both,
            ContainedReturn::Contained,
            "A$B",
            vec!["child", "same", "same", "top", "top"],
        ),
        (On, ContainedReturn::Container, "B$A", vec!["same"]),
        (Both, ContainedReturn::Container, "B$A", vec!["same"]),
        (On, ContainedReturn::Contained, "A$A", vec!["equal-child"]),
        (
            Both,
            ContainedReturn::Container,
            "X$Y",
            vec!["container", "top"],
        ),
        (
            Both,
            ContainedReturn::Container,
            "X$B",
            vec!["container", "top"],
        ),
        (Both, ContainedReturn::Container, "Y$X", vec![]),
    ] {
        let mut query = pair_query(value, mode);
        query.contained_return = returns;
        assert_ids(backend, &tenant, &query, &ids).await;
    }
    let mut grouped = observation("groups", &["Q"], &["R"]);
    grouped["component"] = json!([
        {"code":{"coding":[{"code":"A"}]},"valueCodeableConcept":{"coding":[{"code":"B"}]}},
        {"code":{"coding":[{"code":"C"}]},"valueCodeableConcept":{"coding":[{"code":"D"}]}}
    ]);
    backend
        .create(
            &tenant,
            "Observation",
            grouped.clone(),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    grouped["id"] = json!("group-child");
    backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType":"Patient","id":"group-container","contained":[grouped]}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    for mode in [Off, On, Both] {
        let expected = match mode {
            Off => vec!["groups"],
            On => vec!["group-container"],
            Both => vec!["group-container", "groups"],
        };
        let mut query = SearchQuery::new("Observation");
        query.contained = mode;
        query.parameters.push(composite(
            "component-code-value-concept",
            &[SearchParamType::Token, SearchParamType::Token],
            &["A$B"],
        ));
        query.parameters.push(composite(
            "component-code-value-concept",
            &[SearchParamType::Token, SearchParamType::Token],
            &["C$D"],
        ));
        assert_ids(backend, &tenant, &query, &expected).await;
        query.parameters.truncate(1);
        query.parameters[0].values = vec![SearchValue::eq("A$D")];
        assert_ids(backend, &tenant, &query, &[]).await;
        query.parameters[0].values = vec![SearchValue::eq("A$D"), SearchValue::eq("C$D")];
        assert_ids(backend, &tenant, &query, &expected).await;
    }
}

fn crate_tenant(name: &str) -> TenantContext {
    tenant(&format!("{name}-other"))
}

/// Register real custom definitions so extraction and querying agree on the
/// per-type ordinal, including the third absolute component becoming Token 2.
pub async fn custom_unfolded_families<S: ResourceStorage + SearchProvider>(
    backend: &S,
    name: &str,
    ordinary: bool,
) {
    let tenant = tenant(name);
    for (code, ty, expression) in [
        ("axis-token", "token", "Observation.code"),
        ("axis-number", "number", "Observation.valueInteger"),
        ("axis-string", "string", "Observation.valueString"),
        ("axis-date", "date", "Observation.effective"),
        ("axis-quantity", "quantity", "Observation.valueQuantity"),
    ] {
        backend.create(&tenant,"SearchParameter",json!({"resourceType":"SearchParameter","id":code,"url":format!("http://example.org/1407/{code}"),"name":code,"status":"active","code":code,"base":["Observation"],"type":ty,"expression":expression}),FhirVersion::default()).await.unwrap();
    }
    for (code, base, components) in [
        (
            "token-quantity-token",
            "Observation.component",
            vec![
                ("axis-token", "code"),
                ("axis-quantity", "valueQuantity"),
                ("axis-token", "interpretation"),
            ],
        ),
        (
            "number-pair",
            "Observation.referenceRange",
            vec![("axis-number", "low.value"), ("axis-number", "high.value")],
        ),
        (
            "quantity-pair",
            "Observation.referenceRange",
            vec![("axis-quantity", "low"), ("axis-quantity", "high")],
        ),
        (
            "string-pair",
            "Observation",
            vec![("axis-string", "valueString"), ("axis-string", "note.text")],
        ),
        (
            "date-pair",
            "Observation",
            vec![
                ("axis-date", "effectivePeriod.start"),
                ("axis-date", "effectivePeriod.end"),
            ],
        ),
    ] {
        backend.create(&tenant,"SearchParameter",json!({"resourceType":"SearchParameter","id":code,"url":format!("http://example.org/1407/{code}"),"name":code,"status":"active","code":code,"base":["Observation"],"type":"composite","expression":base,"component":components.iter().map(|(definition,expression)|json!({"definition":format!("http://example.org/1407/{definition}"),"expression":expression})).collect::<Vec<_>>()}),FhirVersion::default()).await.unwrap();
    }
    let mut observed = observation("custom-top", &["Q"], &["R"]);
    observed["component"] = json!([
        {"code":{"coding":[{"code":"A"}]},"valueQuantity":{"value":9},"interpretation":[{"coding":[{"code":"B"}]}]},
        {"code":{"coding":[{"code":"C"}]},"valueQuantity":{"value":1},"interpretation":[{"coding":[{"code":"D"}]}]}
    ]);
    observed["referenceRange"] = json!([{"low":{"value":1},"high":{"value":9}}]);
    observed["valueString"] = json!("Alpha");
    observed["note"] = json!([{"text":"Beta"}]);
    observed["effectivePeriod"] = json!({"start":"2020-01-01","end":"2021-01-01"});
    if ordinary {
        backend
            .create(
                &tenant,
                "Observation",
                observed.clone(),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    observed["id"] = json!("custom-child");
    backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType":"Patient","id":"custom-container","contained":[observed]}),
            FhirVersion::default(),
        )
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
        (
            "quantity-pair",
            vec![SearchParamType::Quantity, SearchParamType::Quantity],
            "1$9",
            "9$1",
        ),
        (
            "string-pair",
            vec![SearchParamType::String, SearchParamType::String],
            "Alpha$Beta",
            "Beta$Alpha",
        ),
        (
            "date-pair",
            vec![SearchParamType::Date, SearchParamType::Date],
            "2020-01-01$2021-01-01",
            "2021-01-01$2020-01-01",
        ),
    ] {
        for mode in [ContainedMode::Off, ContainedMode::On, ContainedMode::Both] {
            if !ordinary && mode != ContainedMode::On {
                continue;
            }
            let expected = match mode {
                ContainedMode::Off => vec!["custom-top"],
                ContainedMode::On => vec!["custom-container"],
                ContainedMode::Both => vec!["custom-container", "custom-top"],
            };
            let mut query = SearchQuery::new("Observation");
            query.contained = mode;
            query.parameters.push(composite(name, &types, &[positive]));
            assert_ids(backend, &tenant, &query, &expected).await;
            query.parameters[0].values = vec![SearchValue::eq(negative)];
            assert_ids(backend, &tenant, &query, &[]).await;
        }
    }
}
