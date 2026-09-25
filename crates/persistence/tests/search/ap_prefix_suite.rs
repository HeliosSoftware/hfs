//! Backend-agnostic suite for the `ap` (approximately) search prefix on
//! number and quantity parameters (issue #1390).
//!
//! Every backend used to pick its own margin: 10% with a floor of 0.0001
//! (SQLite, PostgreSQL), 0.1 (MongoDB) or 0.5 (Elasticsearch). There is now
//! one rule, in `helios_persistence::search::approx_range`: `[v − m, v + m]`
//! with `m = max(10% of |v|, half the implicit precision)`, so `ap` always
//! contains what `eq` matches. Seeds sit away from every window edge, so
//! floating-point rounding cannot flip a case.
//!
//! Date `ap` is not here: #1391 gave dates one shared, clock-free window per
//! precision, and `date_period_suite.rs` covers it on every backend.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `number_exponent_suite.rs`. The backend must be built with the spec search
//! parameters loaded; the positive controls fail loudly if it was not.

#![allow(dead_code)]

use std::collections::BTreeSet;

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

/// (id, `ChargeItem.factorOverride`), either side of the `ap` windows below.
const FACTORS: &[(&str, f64)] = &[
    ("ap-n-neg-111", -111.0),
    ("ap-n-neg-106", -106.0),
    ("ap-n-neg-095", -95.0),
    ("ap-n-neg-089", -89.0),
    ("ap-n-neg-0-4", -0.4),
    ("ap-n-zero", 0.0),
    ("ap-n-0-04", 0.04),
    ("ap-n-0-4", 0.4),
    ("ap-n-0-6", 0.6),
    ("ap-n-060", 60.0),
    ("ap-n-089", 89.0),
    ("ap-n-091", 91.0),
    ("ap-n-100", 100.0),
    ("ap-n-109", 109.0),
    ("ap-n-111", 111.0),
    ("ap-n-140", 140.0),
    ("ap-n-160", 160.0),
];

/// `factor-override=` value → the seeded ChargeItems it must match. The first
/// row is the positive control.
const NUMBER_CASES: &[(&str, &[&str])] = &[
    (
        "ge-1000",
        &[
            "ap-n-neg-111",
            "ap-n-neg-106",
            "ap-n-neg-095",
            "ap-n-neg-089",
            "ap-n-neg-0-4",
            "ap-n-zero",
            "ap-n-0-04",
            "ap-n-0-4",
            "ap-n-0-6",
            "ap-n-060",
            "ap-n-089",
            "ap-n-091",
            "ap-n-100",
            "ap-n-109",
            "ap-n-111",
            "ap-n-140",
            "ap-n-160",
        ],
    ),
    // 10% of the value: [90, 110].
    ("ap100", &["ap-n-091", "ap-n-100", "ap-n-109"]),
    // More written precision does not narrow it: 10% still wins.
    ("ap100.0", &["ap-n-091", "ap-n-100", "ap-n-109"]),
    // One significant figure: `eq1e2` is [50, 150), and `ap` contains it.
    (
        "ap1e2",
        &[
            "ap-n-060", "ap-n-089", "ap-n-091", "ap-n-100", "ap-n-109", "ap-n-111", "ap-n-140",
        ],
    ),
    // A negative value keeps its window in order: [-110, -90].
    ("ap-100", &["ap-n-neg-106", "ap-n-neg-095"]),
    // Zero has no 10%: the floor is half the implicit precision, [-0.5, 0.5].
    (
        "ap0",
        &["ap-n-neg-0-4", "ap-n-zero", "ap-n-0-04", "ap-n-0-4"],
    ),
    // One decimal: [-0.05, 0.05].
    ("ap0.0", &["ap-n-zero", "ap-n-0-04"]),
];

/// (id, `Observation.valueQuantity.value` in mg).
const DOSES: &[(&str, f64)] = &[
    ("ap-q-neg-5-4", -5.4),
    ("ap-q-089", 89.0),
    ("ap-q-091", 91.0),
    ("ap-q-100", 100.0),
    ("ap-q-109", 109.0),
    ("ap-q-111", 111.0),
    ("ap-q-160", 160.0),
];

/// `value-quantity=` value → the seeded Observations it must match. The first
/// row is the positive control.
const QUANTITY_CASES: &[(&str, &[&str])] = &[
    (
        "ge-1000||mg",
        &[
            "ap-q-neg-5-4",
            "ap-q-089",
            "ap-q-091",
            "ap-q-100",
            "ap-q-109",
            "ap-q-111",
            "ap-q-160",
        ],
    ),
    ("ap100||mg", &["ap-q-091", "ap-q-100", "ap-q-109"]),
    ("ap100", &["ap-q-091", "ap-q-100", "ap-q-109"]),
    (
        "ap100|http://unitsofmeasure.org|mg",
        &["ap-q-091", "ap-q-100", "ap-q-109"],
    ),
    (
        "ap1e2||mg",
        &["ap-q-089", "ap-q-091", "ap-q-100", "ap-q-109", "ap-q-111"],
    ),
    // [-5.94, -4.86].
    ("ap-5.4||mg", &["ap-q-neg-5-4"]),
];

/// For backends that also match UCUM-equivalent units: the window is taken
/// on the stated value and both ends are converted.
const CONVERTED_QUANTITY_CASES: &[(&str, &[&str])] = &[
    // 10% wins over the floor: [0.09 g, 0.11 g] = [90 mg, 110 mg].
    ("ap0.100||g", &["ap-q-091", "ap-q-100", "ap-q-109"]),
    // Floor of half the precision: [0.05 g, 0.15 g] = [50 mg, 150 mg].
    (
        "ap1e-1||g",
        &["ap-q-089", "ap-q-091", "ap-q-100", "ap-q-109", "ap-q-111"],
    ),
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

async fn matched<S>(backend: &S, tenant: &TenantContext, query: &SearchQuery) -> BTreeSet<String>
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

fn ids(expected: &[&str]) -> BTreeSet<String> {
    expected.iter().map(|id| id.to_string()).collect()
}

/// Polls `control` until it returns `expected`: Elasticsearch is
/// near-real-time. A failure means the parameter did not index — a backend
/// built without the spec search parameters — not that `ap` is wrong.
async fn positive_control<S>(
    backend: &S,
    tenant: &TenantContext,
    control: &SearchQuery,
    expected: &[&str],
    label: &str,
) where
    S: ResourceStorage + SearchProvider,
{
    let mut got = BTreeSet::new();
    for _ in 0..60 {
        got = matched(backend, tenant, control).await;
        if got == ids(expected) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    assert_eq!(got, ids(expected), "positive control {label}");
}

/// Seeds the resources under a caller-unique tenant and asserts the tables.
/// `unit_conversion` adds the cases that need UCUM-canonical quantity columns.
pub async fn ap_prefix<S>(backend: &S, tenant_base: &str, unit_conversion: bool)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());

    for (id, factor) in FACTORS {
        backend
            .create(
                &tenant,
                "ChargeItem",
                json!({
                    "id": id,
                    "status": "billable",
                    "code": {"text": "x"},
                    "subject": {"reference": "Patient/ap-subject"},
                    "factorOverride": factor,
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap_or_else(|e| panic!("create {id} failed: {e}"));
    }
    for (id, dose) in DOSES {
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "id": id,
                    "status": "final",
                    "code": {"text": "dose"},
                    "valueQuantity": {
                        "value": dose,
                        "unit": "mg",
                        "system": "http://unitsofmeasure.org",
                        "code": "mg",
                    },
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap_or_else(|e| panic!("create {id} failed: {e}"));
    }
    let number = ("ChargeItem", "factor-override", SearchParamType::Number);
    let quantity = ("Observation", "value-quantity", SearchParamType::Quantity);

    for ((resource_type, param, param_type), (value, expected)) in
        [(number, NUMBER_CASES[0]), (quantity, QUANTITY_CASES[0])]
    {
        positive_control(
            backend,
            &tenant,
            &query(resource_type, param, param_type, value),
            expected,
            &format!("{resource_type}?{param}={value}"),
        )
        .await;
    }
    let converted: &[(&str, &[&str])] = if unit_conversion {
        CONVERTED_QUANTITY_CASES
    } else {
        &[]
    };
    let tables = [
        (number, NUMBER_CASES),
        (quantity, QUANTITY_CASES),
        (quantity, converted),
    ];
    let mut failures = Vec::new();
    for ((resource_type, param, param_type), cases) in tables {
        for (value, expected) in cases {
            let got = matched(
                backend,
                &tenant,
                &query(resource_type, param, param_type, value),
            )
            .await;
            if got != ids(expected) {
                failures.push(format!(
                    "{resource_type}?{param}={value}: got {got:?}, expected {expected:?}"
                ));
            }
        }
    }
    for (params, expected) in COMPOSED_NUMBER_CASES {
        let params: Vec<_> = params
            .iter()
            .map(|(n, v)| (*n, SearchParamType::Number, *v))
            .collect();
        let query = multi_query("ChargeItem", &params);
        let got = matched(backend, &tenant, &query).await;
        if got != ids(expected) {
            failures.push(format!(
                "ChargeItem {params:?}: got {got:?}, expected {expected:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// One query with one `SearchParameter` per entry of `params`; each entry's
/// values are the `,`-separated alternatives of that parameter (OR), and the
/// entries are ANDed.
fn multi_query(resource_type: &str, params: &[(&str, SearchParamType, &[&str])]) -> SearchQuery {
    let mut query = SearchQuery::new(resource_type).with_count(100);
    for (name, param_type, values) in params {
        query = query.with_parameter(SearchParameter {
            name: name.to_string(),
            param_type: *param_type,
            values: values.iter().map(|v| SearchValue::parse(v)).collect(),
            ..Default::default()
        });
    }
    query
}

/// (params of a query, the seeded resources it must match).
type ComposedCase = (
    &'static [(&'static str, &'static [&'static str])],
    &'static [&'static str],
);

/// `date=` alternatives (OR) and repeated `date` parameters (AND), over the
/// seeds `DATE_CASES` already pins: `ap2016` is {2015-03, 2016-06, 2017-11},
/// `ap2036` is {2035-03, 2037-10}, `2026` is {2026-06}.
const COMPOSED_NUMBER_CASES: &[ComposedCase] = &[
    // 500's window [450, 550] holds no seed.
    (
        &[("factor-override", &["ap100", "ap500"])],
        &["ap-n-091", "ap-n-100", "ap-n-109"],
    ),
    // [-110, -90] and [90, 110].
    (
        &[("factor-override", &["ap100", "ap-100"])],
        &[
            "ap-n-neg-106",
            "ap-n-neg-095",
            "ap-n-091",
            "ap-n-100",
            "ap-n-109",
        ],
    ),
    // [-0.05, 0.05] and [144, 176].
    (
        &[("factor-override", &["ap0.0", "ap160"])],
        &["ap-n-zero", "ap-n-0-04", "ap-n-160"],
    ),
    (
        &[
            ("factor-override", &["ap100"]),
            ("factor-override", &["ge100"]),
        ],
        &["ap-n-100", "ap-n-109"],
    ),
    (
        &[
            ("factor-override", &["ap100"]),
            ("factor-override", &["ap160"]),
        ],
        &[],
    ),
];
