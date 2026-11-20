//! Backend-agnostic scenarios for `Range` values of quantity search
//! parameters (#2602), shared by the SQLite, PostgreSQL, MongoDB and
//! Elasticsearch test binaries via `#[path]`.
//!
//! `Condition.onset`, `Condition.abatement` and `<canonical>.useContext.value`
//! select a `Range` as well as an `Age`/`Quantity`. A `Range` is one closed
//! interval `[low, high]`; a missing bound leaves that side open, and a
//! `Quantity` or `Age` is the point `[x, x]`. For a search value `v` with
//! implicit-precision range `[s, e)` and `ap` margin `m`:
//!
//! | prefix | matches when                  |
//! |--------|-------------------------------|
//! | `eq`   | `s <= low` and `high < e`     |
//! | `ne`   | `low < s` or `high >= e`      |
//! | `gt`   | `high > v`                    |
//! | `lt`   | `low < v`                     |
//! | `ge`   | `high >= v`                   |
//! | `le`   | `low <= v`                    |
//! | `sa`   | `low > v`                     |
//! | `eb`   | `high < v`                    |
//! | `ap`   | `low <= v+m` and `high >= v-m`|
//!
//! The suite needs the backend built with the spec search parameters loaded;
//! its positive controls fail loudly if it was not.

#![allow(dead_code)]

use std::collections::BTreeSet;

use serde_json::{Value, json};

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchParamType, SearchParameter, SearchQuery, SearchValue, SortDirection, SortDirective,
};

use crate::seeding::SeedingStorage;

const UCUM: &str = "http://unitsofmeasure.org";

/// A bound of a `Range`, in years.
fn bound(value: f64) -> Value {
    json!({"value": value, "unit": "a", "system": UCUM, "code": "a"})
}

fn range(low: Option<f64>, high: Option<f64>) -> Value {
    let mut range = serde_json::Map::new();
    if let Some(low) = low {
        range.insert("low".into(), bound(low));
    }
    if let Some(high) = high {
        range.insert("high".into(), bound(high));
    }
    Value::Object(range)
}

/// Seeded Conditions, in years. Their intervals:
///
/// - `c-range`: `onsetRange` `[40, 50]`
/// - `c-low`: `onsetRange` with only `low` 60, so `[60, +inf)`
/// - `c-high`: `onsetRange` with only `high` 10, so `(-inf, 10]`
/// - `c-age`: `onsetAge` 45, so `[45, 45]`
/// - `c-abate`: `abatementRange` `[20, 30]`, no onset
fn conditions() -> Vec<Value> {
    let base = |id: &str| {
        json!({
            "id": id,
            "clinicalStatus": {"coding": [{
                "system": "http://terminology.hl7.org/CodeSystem/condition-clinical",
                "code": "active"
            }]},
            "subject": {"reference": "Patient/qr-pt"},
        })
    };
    let with = |id: &str, field: &str, value: Value| {
        let mut condition = base(id);
        condition[field] = value;
        condition
    };
    vec![
        with("c-range", "onsetRange", range(Some(40.0), Some(50.0))),
        with("c-low", "onsetRange", range(Some(60.0), None)),
        with("c-high", "onsetRange", range(None, Some(10.0))),
        with(
            "c-age",
            "onsetAge",
            json!({"value": 45, "unit": "a", "system": UCUM, "code": "a"}),
        ),
        with("c-abate", "abatementRange", range(Some(20.0), Some(30.0))),
    ]
}

const USE_CONTEXT_TYPE: &str = "http://terminology.hl7.org/CodeSystem/usage-context-type";

fn value_set(id: &str, value_field: &str, value: Value) -> Value {
    json!({
        "id": id,
        "status": "active",
        "useContext": [{
            "code": {"system": USE_CONTEXT_TYPE, "code": "age"},
            value_field: value,
        }],
    })
}

/// `Condition?onset-age=` value -> the Conditions it must match.
const ONSET_CASES: &[(&str, &[&str])] = &[
    // eq: the search range must contain the whole interval.
    ("45", &["c-age"]),
    ("eq45", &["c-age"]),
    ("40", &[]),
    // ne is the complement of eq, open intervals included.
    ("ne45", &["c-high", "c-low", "c-range"]),
    // gt: the interval extends above the value.
    ("gt45", &["c-low", "c-range"]),
    ("gt10", &["c-age", "c-low", "c-range"]),
    ("gt1000", &["c-low"]),
    // lt: the interval extends below the value.
    ("lt45", &["c-high", "c-range"]),
    ("lt60", &["c-age", "c-high", "c-range"]),
    ("lt0", &["c-high"]),
    ("ge45", &["c-age", "c-low", "c-range"]),
    ("ge10", &["c-age", "c-high", "c-low", "c-range"]),
    ("le45", &["c-age", "c-high", "c-range"]),
    ("le60", &["c-age", "c-high", "c-low", "c-range"]),
    // sa: the interval starts after the value.
    ("sa45", &["c-low"]),
    ("sa50", &["c-low"]),
    // eb: the interval ends before the value.
    ("eb45", &["c-high"]),
    ("eb40", &["c-high"]),
    // ap: the interval overlaps [v - m, v + m], m = 4.5 for 45.
    ("ap45", &["c-age", "c-range"]),
    ("ap5", &["c-high"]),
    ("ap100", &["c-low"]),
];

/// `Condition?abatement-age=` value -> the Conditions it must match.
const ABATEMENT_CASES: &[(&str, &[&str])] = &[
    ("ne1800", &["c-abate"]),
    ("25", &[]),
    ("ne25", &["c-abate"]),
    ("gt25", &["c-abate"]),
    ("gt30", &[]),
    ("ge30", &["c-abate"]),
    ("lt25", &["c-abate"]),
    ("lt20", &[]),
    ("le20", &["c-abate"]),
    ("sa10", &["c-abate"]),
    ("sa25", &[]),
    ("eb40", &["c-abate"]),
    ("eb30", &[]),
    ("ap25", &["c-abate"]),
];

/// `ValueSet?context-quantity=` value -> the ValueSets it must match.
/// `vs-range` carries `useContext.valueRange` `[1, 5]`, `vs-qty` carries
/// `useContext.valueQuantity` 3.
const CONTEXT_CASES: &[(&str, &[&str])] = &[
    ("ne0", &["vs-qty", "vs-range"]),
    ("3", &["vs-qty"]),
    ("gt3", &["vs-range"]),
    ("ge3", &["vs-qty", "vs-range"]),
    ("lt3", &["vs-range"]),
    ("le3", &["vs-qty", "vs-range"]),
    ("sa3", &[]),
    ("eb3", &[]),
    ("sa0", &["vs-qty", "vs-range"]),
    ("eb6", &["vs-qty", "vs-range"]),
];

fn query(
    resource_type: &str,
    param: &str,
    param_type: SearchParamType,
    value: &str,
) -> SearchQuery {
    SearchQuery::new(resource_type)
        .with_parameter(SearchParameter {
            name: param.to_string(),
            param_type,
            values: vec![SearchValue::parse(value)],
            ..Default::default()
        })
        .with_count(100)
}

fn quantity_query(resource_type: &str, param: &str, value: &str) -> SearchQuery {
    query(resource_type, param, SearchParamType::Quantity, value)
}

/// Every onset Condition (all of them, whatever their interval: no value
/// lies within 1800), ordered by `onset-age`.
fn sorted_by_onset_age(direction: SortDirection) -> SearchQuery {
    quantity_query("Condition", "onset-age", "ne1800").with_sort(SortDirective {
        parameter: "onset-age".to_string(),
        direction,
        param_type: Some(SearchParamType::Quantity),
    })
}

async fn found<S>(backend: &S, tenant: &TenantContext, query: &SearchQuery) -> Vec<String>
where
    S: ResourceStorage + SearchProvider,
{
    backend
        .search(tenant, query)
        .await
        .unwrap_or_else(|e| panic!("search {query:?} failed: {e}"))
        .resources
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect()
}

async fn matched<S>(backend: &S, tenant: &TenantContext, query: &SearchQuery) -> BTreeSet<String>
where
    S: ResourceStorage + SearchProvider,
{
    found(backend, tenant, query).await.into_iter().collect()
}

fn ids(expected: &[&str]) -> BTreeSet<String> {
    expected.iter().map(|id| id.to_string()).collect()
}

async fn create<S>(backend: &S, tenant: &TenantContext, resource_type: &str, resource: Value)
where
    S: ResourceStorage + SearchProvider,
{
    let id = resource["id"].as_str().unwrap_or_default().to_string();
    backend
        .seed_create(tenant, resource_type, resource, FhirVersion::default())
        .await
        .unwrap_or_else(|e| panic!("create {resource_type}/{id} failed: {e}"));
}

/// Waits until the control search finds `expected` resources: the positive
/// control that the index was written, and the wait for eventually
/// consistent backends.
async fn wait_for<S>(backend: &S, tenant: &TenantContext, control: &SearchQuery, expected: usize)
where
    S: ResourceStorage + SearchProvider,
{
    for attempt in 0..60 {
        let visible = matched(backend, tenant, control).await;
        if visible.len() == expected {
            return;
        }
        assert!(
            attempt < 59,
            "the seeded resources never became searchable ({}/{expected}) by {control:?}: \
             is the backend built with the spec search parameters?",
            visible.len(),
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

/// Seeds the resources under the caller's tenant and waits until their index
/// rows are visible.
pub async fn seed<S>(backend: &S, tenant: &TenantContext)
where
    S: ResourceStorage + SearchProvider,
{
    create(
        backend,
        tenant,
        "Patient",
        json!({"id": "qr-pt", "active": true}),
    )
    .await;
    for condition in conditions() {
        create(backend, tenant, "Condition", condition).await;
    }
    create(
        backend,
        tenant,
        "ValueSet",
        value_set("vs-range", "valueRange", range(Some(1.0), Some(5.0))),
    )
    .await;
    create(
        backend,
        tenant,
        "ValueSet",
        value_set(
            "vs-qty",
            "valueQuantity",
            json!({"value": 3, "unit": "a", "system": UCUM, "code": "a"}),
        ),
    )
    .await;

    wait_for(
        backend,
        tenant,
        &query(
            "Condition",
            "clinical-status",
            SearchParamType::Token,
            "active",
        ),
        conditions().len(),
    )
    .await;
    wait_for(
        backend,
        tenant,
        &query("ValueSet", "status", SearchParamType::Token, "active"),
        2,
    )
    .await;
}

/// Asserts every prefix table against `Condition.onset`,
/// `Condition.abatement` and `ValueSet.useContext`, and `_sort` both ways.
/// Seeds under a caller-unique tenant.
pub async fn quantity_ranges_are_intervals<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    seed(backend, &tenant).await;
    check_searches(backend, &tenant).await;
}

/// The searches and orderings alone, for a caller that seeded (and perhaps
/// reindexed) the data itself.
pub async fn check_searches<S>(backend: &S, tenant: &TenantContext)
where
    S: ResourceStorage + SearchProvider,
{
    // Positive control: every onset interval, open sides and the point
    // included, has an index entry. Nothing lies within 1800, so `ne` finds
    // them all.
    assert_eq!(
        matched(
            backend,
            tenant,
            &quantity_query("Condition", "onset-age", "ne1800")
        )
        .await,
        ids(&["c-age", "c-high", "c-low", "c-range"]),
        "Condition?onset-age=ne1800"
    );

    for (value, expected) in ONSET_CASES {
        assert_eq!(
            matched(
                backend,
                tenant,
                &quantity_query("Condition", "onset-age", value)
            )
            .await,
            ids(expected),
            "Condition?onset-age={value}"
        );
    }
    for (value, expected) in ABATEMENT_CASES {
        assert_eq!(
            matched(
                backend,
                tenant,
                &quantity_query("Condition", "abatement-age", value)
            )
            .await,
            ids(expected),
            "Condition?abatement-age={value}"
        );
    }
    for (value, expected) in CONTEXT_CASES {
        assert_eq!(
            matched(
                backend,
                tenant,
                &quantity_query("ValueSet", "context-quantity", value)
            )
            .await,
            ids(expected),
            "ValueSet?context-quantity={value}"
        );
    }

    // _sort=onset-age orders by where each interval starts, an open start
    // first; _sort=-onset-age by where it ends, an open end first.
    assert_eq!(
        found(
            backend,
            tenant,
            &sorted_by_onset_age(SortDirection::Ascending)
        )
        .await,
        ["c-high", "c-range", "c-age", "c-low"],
        "_sort=onset-age"
    );
    assert_eq!(
        found(
            backend,
            tenant,
            &sorted_by_onset_age(SortDirection::Descending)
        )
        .await,
        ["c-low", "c-range", "c-age", "c-high"],
        "_sort=-onset-age"
    );
}
