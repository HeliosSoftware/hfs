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
    CompositeSearchComponent, ContainedMode, ContainedReturn, ReverseChainedParameter,
    SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue, SortDirection,
    SortDirective,
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
    check_extended(backend, tenant_base).await;
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

// ---------------------------------------------------------------------------
// Extended scenarios: unit-qualified and cross-unit searches, `:missing`,
// multi-valued parameters, tenant isolation, composites, `_has`, `_contained`.
//
// They seed their own tenants (`<base>-x` and `<base>-y`) so the intervals of
// the tables above, and the `ne1800` control that counts them, are untouched.
// ---------------------------------------------------------------------------

const X_ONLY: &[&str] = &["x-yr", "x-low", "x-high", "x-mo", "x-narrow"];

/// `onsetRange` low/high in years, or any other element.
fn x_condition(id: &str, field: Option<(&str, Value)>) -> Value {
    let mut condition = json!({
        "id": id,
        "clinicalStatus": {"coding": [{
            "system": "http://terminology.hl7.org/CodeSystem/condition-clinical",
            "code": "active"
        }]},
        "subject": {"reference": format!("Patient/pt-{}", id.trim_start_matches("x-"))},
    });
    if let Some((field, value)) = field {
        condition[field] = value;
    }
    condition
}

/// The extended Conditions, in weeks (1 wk = 7 d, exact, so cross-unit
/// comparisons have no float ties):
///
/// - `x-yr`: Range `[4, 6]` wk = `[28, 42]` d
/// - `x-low`: Range low 10 wk only = `[70, +inf)` d
/// - `x-high`: Range high 2 wk only = `(-inf, 14]` d
/// - `x-mo`: Age 35 d (stored in days) = `[5, 5]` wk
/// - `x-narrow`: Range `[4.98, 5.02]` wk = `[34.86, 35.14]` d
/// - `x-bad`: Range low 9 > high 1: not an interval, skipped at indexing
/// - `x-mix`: Range low in UCUM, high in another system: skipped
/// - `x-empty`: Range with neither bound: nothing to index
/// - `x-none`: no onset at all
/// A `Range` in weeks.
fn wk_range(low: Option<f64>, high: Option<f64>) -> Value {
    let mut range = serde_json::Map::new();
    for (key, value) in [("low", low), ("high", high)] {
        if let Some(value) = value {
            range.insert(
                key.into(),
                json!({"value": value, "unit": "wk", "system": UCUM, "code": "wk"}),
            );
        }
    }
    Value::Object(range)
}

fn x_conditions() -> Vec<Value> {
    vec![
        x_condition("x-yr", Some(("onsetRange", wk_range(Some(4.0), Some(6.0))))),
        x_condition("x-low", Some(("onsetRange", wk_range(Some(10.0), None)))),
        x_condition("x-high", Some(("onsetRange", wk_range(None, Some(2.0))))),
        x_condition(
            "x-mo",
            Some((
                "onsetAge",
                json!({"value": 35, "unit": "d", "system": UCUM, "code": "d"}),
            )),
        ),
        x_condition(
            "x-narrow",
            Some(("onsetRange", wk_range(Some(4.98), Some(5.02)))),
        ),
        x_condition(
            "x-bad",
            Some(("onsetRange", wk_range(Some(9.0), Some(1.0)))),
        ),
        x_condition(
            "x-mix",
            Some((
                "onsetRange",
                json!({
                    "low": {"value": 1, "unit": "wk", "system": UCUM, "code": "wk"},
                    "high": {"value": 3, "unit": "wk",
                             "system": "http://example.org/units", "code": "wk"},
                }),
            )),
        ),
        x_condition("x-empty", Some(("onsetRange", json!({})))),
        x_condition("x-none", None),
    ]
}

fn x_value_set(id: &str, contexts: Vec<(&str, &str, Value)>) -> Value {
    json!({
        "id": id,
        "status": "active",
        "useContext": contexts.into_iter().map(|(code, field, value)| json!({
            "code": {"system": USE_CONTEXT_TYPE, "code": code},
            field: value,
        })).collect::<Vec<_>>(),
    })
}

/// `<value>|<ucum>|<unit>`.
fn in_unit(value: &str, unit: &str) -> String {
    format!("{value}|{UCUM}|{unit}")
}

/// A `Condition?onset-age` search for every unit-qualified case, in days
/// and in weeks: `(days, weeks, matches)`.
const UNIT_CASES: &[(&str, &str, &[&str])] = &[
    ("eq35", "eq5", &["x-mo", "x-narrow"]),
    ("eq28", "eq4", &[]),
    ("ne35", "ne5", &["x-high", "x-low", "x-yr"]),
    ("gt35", "gt5", &["x-low", "x-narrow", "x-yr"]),
    ("lt35", "lt5", &["x-high", "x-narrow", "x-yr"]),
    ("ge35", "ge5", &["x-low", "x-mo", "x-narrow", "x-yr"]),
    ("le35", "le5", &["x-high", "x-mo", "x-narrow", "x-yr"]),
    ("ap35", "ap5", &["x-mo", "x-narrow", "x-yr"]),
    ("sa35", "sa5", &["x-low"]),
    ("eb35", "eb5", &["x-high"]),
    // Open bounds are sentinels: they must not scale between units.
    ("ge70", "ge10", &["x-low"]),
    ("le70", "le10", X_ONLY),
    ("lt70", "lt10", &["x-high", "x-mo", "x-narrow", "x-yr"]),
    ("ne70", "ne10", X_ONLY),
    ("gt7000", "gt1000", &["x-low"]),
    ("lt-7000", "lt-1000", &["x-high"]),
];

static FAILURES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Records a mismatch instead of stopping at the first one: the whole matrix
/// is the useful output when a backend disagrees. `check_extended` fails at
/// the end if any was recorded.
async fn assert_matches<S>(
    backend: &S,
    tenant: &TenantContext,
    query: &SearchQuery,
    expected: &[&str],
    label: &str,
) where
    S: ResourceStorage + SearchProvider,
{
    let got = matched(backend, tenant, query).await;
    if got != ids(expected) {
        FAILURES
            .lock()
            .unwrap()
            .push(format!("{label}: got {got:?}, expected {expected:?}"));
    }
}

fn missing(
    resource_type: &str,
    param: &str,
    param_type: SearchParamType,
    flag: &str,
) -> SearchQuery {
    let mut q = query(resource_type, param, param_type, flag);
    q.parameters[0].modifier = Some(SearchModifier::Missing);
    q.parameters[0].values = vec![SearchValue::eq(flag)];
    q
}

fn x_tenant(tenant_base: &str, suffix: &str) -> TenantContext {
    TenantContext::new(
        TenantId::new(format!("{tenant_base}-{suffix}")),
        TenantPermissions::full_access(),
    )
}

/// Seeds tenants `x` and `y`, then asserts the extended scenarios.
pub async fn check_extended<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let x = x_tenant(tenant_base, "x");
    let y = x_tenant(tenant_base, "y");

    // ---- tenant x ----
    for condition in x_conditions() {
        let patient = format!(
            "pt-{}",
            condition["id"].as_str().unwrap().trim_start_matches("x-")
        );
        create(
            backend,
            &x,
            "Patient",
            json!({"id": patient, "active": true}),
        )
        .await;
        create(backend, &x, "Condition", condition).await;
    }
    // vs-multi has two useContext entries: one [1,5], one 20.
    create(
        backend,
        &x,
        "ValueSet",
        x_value_set(
            "vs-multi",
            vec![
                ("age", "valueRange", range(Some(1.0), Some(5.0))),
                (
                    "gender",
                    "valueQuantity",
                    json!({"value": 20, "unit": "a", "system": UCUM, "code": "a"}),
                ),
            ],
        ),
    )
    .await;
    create(
        backend,
        &x,
        "ValueSet",
        x_value_set(
            "vs-solo",
            vec![(
                "age",
                "valueQuantity",
                json!({"value": 3, "unit": "a", "system": UCUM, "code": "a"}),
            )],
        ),
    )
    .await;
    create(
        backend,
        &x,
        "ValueSet",
        x_value_set(
            "vs-gender",
            vec![(
                "gender",
                "valueQuantity",
                json!({"value": 30, "unit": "a", "system": UCUM, "code": "a"}),
            )],
        ),
    )
    .await;
    create(
        backend,
        &x,
        "ValueSet",
        json!({"id": "vs-none", "status": "active"}),
    )
    .await;
    // vs-pt: a Range that is the point 3 (low == high), age context.
    create(
        backend,
        &x,
        "ValueSet",
        x_value_set(
            "vs-pt",
            vec![("age", "valueRange", range(Some(3.0), Some(3.0)))],
        ),
    )
    .await;
    // Containers holding a contained Condition with an onset Range in weeks.
    for (id, low, high) in [("pc-in", 4.0, 6.0), ("pc-out", 20.0, 30.0)] {
        create(
            backend,
            &x,
            "Patient",
            json!({
                "id": id,
                "active": true,
                "contained": [{
                    "resourceType": "Condition",
                    "id": "c1",
                    "clinicalStatus": {"coding": [{
                        "system": "http://terminology.hl7.org/CodeSystem/condition-clinical",
                        "code": "active"
                    }]},
                    "subject": {"reference": "Patient/pt-x"},
                    "onsetRange": {
                        "low": {"value": low, "unit": "wk", "system": UCUM, "code": "wk"},
                        "high": {"value": high, "unit": "wk", "system": UCUM, "code": "wk"},
                    },
                }],
            }),
        )
        .await;
    }

    // ---- tenant y: same ids with other intervals, plus an id of its own ----
    create(
        backend,
        &y,
        "Patient",
        json!({"id": "pt-yr", "active": true}),
    )
    .await;
    create(
        backend,
        &y,
        "Patient",
        json!({"id": "pt-iso", "active": true}),
    )
    .await;
    create(
        backend,
        &y,
        "Condition",
        x_condition(
            "x-yr",
            Some(("onsetRange", wk_range(Some(100.0), Some(110.0)))),
        ),
    )
    .await;
    let mut iso = x_condition(
        "y-iso",
        Some(("onsetRange", wk_range(Some(2.0), Some(3.0)))),
    );
    iso["subject"] = json!({"reference": "Patient/pt-iso"});
    create(backend, &y, "Condition", iso).await;

    wait_for(
        backend,
        &x,
        &query(
            "Condition",
            "clinical-status",
            SearchParamType::Token,
            "active",
        ),
        x_conditions().len(),
    )
    .await;
    wait_for(
        backend,
        &x,
        &query("ValueSet", "status", SearchParamType::Token, "active"),
        5,
    )
    .await;
    wait_for(
        backend,
        &y,
        &query(
            "Condition",
            "clinical-status",
            SearchParamType::Token,
            "active",
        ),
        2,
    )
    .await;

    // (a) unit-qualified and cross-unit searches against Range rows.
    //
    // Probe whether the backend converts between UCUM units at all: the Age
    // `x-mo` is stored in days and `eq5|wk` finds it only if it does. MongoDB
    // matches the unit code verbatim (no UCUM canonicalization), so there the
    // days searches are skipped and `x-mo` is left out of the weeks ones;
    // every other backend must convert, and the Ranges with open bounds must
    // survive the conversion.
    let converts = matched(
        backend,
        &x,
        &quantity_query("Condition", "onset-age", &in_unit("eq5", "wk")),
    )
    .await
    .contains("x-mo");
    if !converts {
        eprintln!(
            "[quantity_range_suite] backend does not convert UCUM units: \
             skipping the cross-unit searches"
        );
    }
    for (days, weeks, expected) in UNIT_CASES {
        for (value, unit) in [(*days, "d"), (*weeks, "wk")] {
            if !converts && unit == "d" {
                continue;
            }
            let expected: Vec<&str> = expected
                .iter()
                .copied()
                .filter(|id| converts || *id != "x-mo")
                .collect();
            let value = match value.find(|c: char| c.is_ascii_digit() || c == '-') {
                Some(at) => format!("{}{}", &value[..at], in_unit(&value[at..], unit)),
                None => unreachable!(),
            };
            assert_matches(
                backend,
                &x,
                &quantity_query("Condition", "onset-age", &value),
                &expected,
                &format!("Condition?onset-age={value}"),
            )
            .await;
        }
    }

    // (b) :missing on onset-age. A Range skipped at indexing (low > high,
    // mismatched systems, no bound) has no row, like an absent element.
    assert_matches(
        backend,
        &x,
        &missing("Condition", "onset-age", SearchParamType::Quantity, "true"),
        &["x-bad", "x-empty", "x-mix", "x-none"],
        "Condition?onset-age:missing=true",
    )
    .await;
    assert_matches(
        backend,
        &x,
        &missing("Condition", "onset-age", SearchParamType::Quantity, "false"),
        X_ONLY,
        "Condition?onset-age:missing=false",
    )
    .await;
    assert_matches(
        backend,
        &x,
        &missing(
            "ValueSet",
            "context-quantity",
            SearchParamType::Quantity,
            "true",
        ),
        &["vs-none"],
        "ValueSet?context-quantity:missing=true",
    )
    .await;
    assert_matches(
        backend,
        &x,
        &missing(
            "ValueSet",
            "context-quantity",
            SearchParamType::Quantity,
            "false",
        ),
        &["vs-gender", "vs-multi", "vs-pt", "vs-solo"],
        "ValueSet?context-quantity:missing=false",
    )
    .await;

    // (c) multi-valued parameter: vs-multi carries [1, 5] and 20. A prefix
    // matches the resource when any entry satisfies it.
    const MULTI: &[(&str, &[&str])] = &[
        ("20", &["vs-multi"]),
        ("ne3", &["vs-gender", "vs-multi"]),
        // vs-multi's entry [1, 5] is outside 20, so ne still holds.
        ("ne20", &["vs-gender", "vs-multi", "vs-pt", "vs-solo"]),
        ("ne0", &["vs-gender", "vs-multi", "vs-pt", "vs-solo"]),
        ("gt25", &["vs-gender"]),
        ("ge20", &["vs-gender", "vs-multi"]),
        ("lt25", &["vs-multi", "vs-pt", "vs-solo"]),
        ("le3", &["vs-multi", "vs-pt", "vs-solo"]),
        ("ap20", &["vs-multi"]),
        ("sa5", &["vs-gender", "vs-multi"]),
        ("eb2", &[]),
        ("eb4", &["vs-pt", "vs-solo"]),
    ];
    for (value, expected) in MULTI {
        assert_matches(
            backend,
            &x,
            &quantity_query("ValueSet", "context-quantity", value),
            expected,
            &format!("ValueSet?context-quantity={value} (multi-valued)"),
        )
        .await;
    }
    // :not is token-only; on the multi-valued token it must exclude a
    // resource with any entry of the code, not just one without it.
    let mut not_age = query("ValueSet", "context-type", SearchParamType::Token, "age");
    not_age.parameters[0].modifier = Some(SearchModifier::Not);
    not_age.parameters[0].values = vec![SearchValue::eq(&format!("{USE_CONTEXT_TYPE}|age"))];
    let not_age = matched(backend, &x, &not_age).await;
    assert!(
        !not_age.contains("vs-multi")
            && !not_age.contains("vs-solo")
            && not_age.contains("vs-gender"),
        "ValueSet?context-type:not=age: {not_age:?}"
    );

    // (d) tenant isolation.
    assert_matches(
        backend,
        &x,
        &quantity_query("Condition", "onset-age", &in_unit("ge100", "wk")),
        &["x-low"],
        "x: Condition?onset-age=ge100|a",
    )
    .await;
    assert_matches(
        backend,
        &y,
        &quantity_query("Condition", "onset-age", &in_unit("ge100", "wk")),
        &["x-yr"],
        "y: Condition?onset-age=ge100|a",
    )
    .await;
    assert_matches(
        backend,
        &y,
        &quantity_query("Condition", "onset-age", &in_unit("le3", "wk")),
        &["y-iso"],
        "y: Condition?onset-age=le3|a",
    )
    .await;
    let x_le3 = matched(
        backend,
        &x,
        &quantity_query("Condition", "onset-age", &in_unit("le3", "wk")),
    )
    .await;
    assert!(
        !x_le3.contains("y-iso"),
        "tenant x saw y's Condition: {x_le3:?}"
    );
    assert_eq!(x_le3, ids(&["x-high"]));

    // (e) composite token+quantity: ValueSet context-type-quantity pairs a
    // useContext code with its value. The positive control is the plain
    // Quantity: vs-solo is `age$3`.
    let composite = |value: &str| {
        SearchQuery::new("ValueSet")
            .with_parameter(SearchParameter {
                name: "context-type-quantity".to_string(),
                param_type: SearchParamType::Composite,
                values: vec![SearchValue::eq(value)],
                components: vec![
                    CompositeSearchComponent {
                        param_type: SearchParamType::Token,
                        param_name: "context-type".to_string(),
                    },
                    CompositeSearchComponent {
                        param_type: SearchParamType::Quantity,
                        param_name: "context-quantity".to_string(),
                    },
                ],
                ..Default::default()
            })
            .with_count(100)
    };
    // A Range with low != high is not matched through the composite
    // (vs-multi's [1, 5] under `age`, whatever the prefix); a Range with
    // low == high IS, as the point it is (vs-pt).
    // NOTE: the point case depends on the extractor change that indexes a
    // low == high Range as a point for composites; expected to match.
    assert_matches(
        backend,
        &x,
        &composite("age$3"),
        &["vs-pt", "vs-solo"],
        "ValueSet?context-type-quantity=age$3 (vs-multi [1,5] skipped; vs-pt = point 3)",
    )
    .await;
    assert_matches(
        backend,
        &x,
        &composite("age$ge3"),
        &["vs-pt", "vs-solo"],
        "ValueSet?context-type-quantity=age$ge3 (vs-multi [1,5] skipped)",
    )
    .await;
    assert_matches(
        backend,
        &x,
        &composite("gender$ge20"),
        &["vs-gender", "vs-multi"],
        "ValueSet?context-type-quantity=gender$ge20",
    )
    .await;

    // (f) `_has`: Patients whose Conditions' onset matches.
    let mut has = SearchQuery::new("Patient").with_count(100);
    has.reverse_chains.push(ReverseChainedParameter::terminal(
        "Condition",
        "subject",
        "onset-age",
        SearchValue::parse(&in_unit("gt5", "wk")),
    ));
    let has = helios_persistence::search::chain_resolver::resolve_chains(backend, &x, &has)
        .await
        .unwrap_or_else(|e| panic!("resolving _has failed: {e}"));
    assert_matches(
        backend,
        &x,
        &has,
        &["pt-low", "pt-narrow", "pt-yr"],
        "Patient?_has:Condition:subject:onset-age=gt5|wk",
    )
    .await;

    // `_contained`: the Patients containing a Condition whose onset Range
    // matches (the container is returned).
    for (value, expected) in [("gt10|wk", "pc-out"), ("lt5|wk", "pc-in")] {
        let mut contained = quantity_query(
            "Condition",
            "onset-age",
            &value.replacen('|', &format!("|{UCUM}|"), 1),
        );
        contained.contained = ContainedMode::On;
        contained.contained_return = ContainedReturn::Container;
        assert_matches(
            backend,
            &x,
            &contained,
            &[expected],
            &format!("Condition?_contained=true&onset-age={value}"),
        )
        .await;
    }

    let failures = std::mem::take(&mut *FAILURES.lock().unwrap());
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
