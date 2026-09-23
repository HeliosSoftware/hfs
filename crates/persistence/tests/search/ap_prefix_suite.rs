//! Backend-agnostic suite for the `ap` (approximately) search prefix on
//! number, quantity and date parameters (issue #1390).
//!
//! Every backend used to pick its own margin: 10% with a floor of 0.0001
//! (SQLite, PostgreSQL), 0.1 (MongoDB) or 0.5 (Elasticsearch) for numbers;
//! for dates a precision-scaled window (SQLite), scalar equality with the
//! range start (PostgreSQL), ±12 h (MongoDB) or plain `eq` (Elasticsearch).
//! There is now one rule, in `helios_persistence::search`:
//!
//! - number / quantity: `[v − m, v + m]` with
//!   `m = max(10% of |v|, half the implicit precision)`, so `ap` always
//!   contains what `eq` matches;
//! - date: the value's precision range widened on both sides by a tenth of
//!   the gap between `now` and that range (zero when `now` falls inside it).
//!
//! The date cases pin `now` with [`SearchQuery::with_now`], so they do not
//! depend on the wall clock. Seeds sit days (or tens of minutes) away from
//! every window edge, so millisecond/microsecond storage and floating-point
//! rounding cannot flip a case.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `number_exponent_suite.rs`. The backend must be built with the spec search
//! parameters loaded; the positive controls fail loudly if it was not.

#![allow(dead_code)]

use std::collections::BTreeSet;

use chrono::{DateTime, TimeZone, Utc};
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

/// (id, `Procedure.performedDateTime`), in UTC.
const PERFORMED: &[(&str, &str)] = &[
    ("ap-d-2015-01", "2015-01-15T12:00:00Z"),
    ("ap-d-2015-03", "2015-03-01T12:00:00Z"),
    ("ap-d-2016-06", "2016-06-15T12:00:00Z"),
    ("ap-d-2017-11", "2017-11-01T12:00:00Z"),
    ("ap-d-2017-12", "2017-12-20T12:00:00Z"),
    ("ap-d-2025-12-20", "2025-12-20T12:00:00Z"),
    ("ap-d-1230", "2025-12-31T12:30:00Z"),
    ("ap-d-1330", "2025-12-31T13:30:00Z"),
    ("ap-d-1440", "2025-12-31T14:40:00Z"),
    ("ap-d-1530", "2025-12-31T15:30:00Z"),
    ("ap-d-2026-06", "2026-06-15T12:00:00Z"),
    ("ap-d-2027-01", "2027-01-10T12:00:00Z"),
    ("ap-d-2034-10", "2034-10-01T12:00:00Z"),
    ("ap-d-2035-03", "2035-03-01T12:00:00Z"),
    ("ap-d-2037-10", "2037-10-01T12:00:00Z"),
    ("ap-d-2038-03", "2038-03-01T12:00:00Z"),
];

/// The `now` the date cases are measured from, unless a case names its own.
fn default_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

/// (`now` as (year, month, day), `date=` value, the seeded Procedures it must
/// match).
type DateCase = ((i32, u32, u32), &'static str, &'static [&'static str]);

/// The first row is the positive control.
const DATE_CASES: &[DateCase] = &[
    (
        (2026, 1, 1),
        "ge1900-01-01",
        &[
            "ap-d-2015-01",
            "ap-d-2015-03",
            "ap-d-2016-06",
            "ap-d-2017-11",
            "ap-d-2017-12",
            "ap-d-2025-12-20",
            "ap-d-1230",
            "ap-d-1330",
            "ap-d-1440",
            "ap-d-1530",
            "ap-d-2026-06",
            "ap-d-2027-01",
            "ap-d-2034-10",
            "ap-d-2035-03",
            "ap-d-2037-10",
            "ap-d-2038-03",
        ],
    ),
    ((2026, 1, 1), "2016", &["ap-d-2016-06"]),
    // Past, year precision: [2016, 2017) is 3287 days before `now`, so the
    // margin is 328.7 days: [~2015-02-06, ~2017-11-26).
    (
        (2026, 1, 1),
        "ap2016",
        &["ap-d-2015-03", "ap-d-2016-06", "ap-d-2017-11"],
    ),
    // The same value seen from further away widens: from 2036-06-01 the gap
    // is 7091 days, the margin 709.1 days: [~2014-01-22, ~2018-12-11).
    (
        (2036, 6, 1),
        "ap2016",
        &[
            "ap-d-2015-01",
            "ap-d-2015-03",
            "ap-d-2016-06",
            "ap-d-2017-11",
            "ap-d-2017-12",
        ],
    ),
    // `now` inside the range: no margin, `ap` is `eq`.
    ((2016, 6, 1), "ap2016", &["ap-d-2016-06"]),
    ((2026, 1, 1), "ap2026", &["ap-d-2026-06"]),
    ((2026, 1, 1), "2026", &["ap-d-2026-06"]),
    // `now` on the (exclusive) end of the range: still no margin.
    (
        (2026, 1, 1),
        "ap2025-12",
        &[
            "ap-d-2025-12-20",
            "ap-d-1230",
            "ap-d-1330",
            "ap-d-1440",
            "ap-d-1530",
        ],
    ),
    // Future, year precision: [2036, 2037) is 3652 days after `now`, the
    // margin 365.2 days: [~2034-12-31T19:12, ~2038-01-01T04:48).
    ((2026, 1, 1), "ap2036", &["ap-d-2035-03", "ap-d-2037-10"]),
    // Day precision: 11 days before `now`, a 1.1-day margin.
    ((2026, 1, 1), "ap2025-12-20", &["ap-d-2025-12-20"]),
    // Minute precision: [14:00, 14:01) ends 599 minutes before `now`, the
    // margin is 59.9 minutes: [~13:00:06, ~15:00:54).
    ((2026, 1, 1), "2025-12-31T14:00Z", &[]),
    (
        (2026, 1, 1),
        "ap2025-12-31T14:00Z",
        &["ap-d-1330", "ap-d-1440"],
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

fn date_query(value: &str, now: DateTime<Utc>) -> SearchQuery {
    query("Procedure", "date", SearchParamType::Date, value).with_now(now)
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
    for (id, performed) in PERFORMED {
        backend
            .create(
                &tenant,
                "Procedure",
                json!({
                    "id": id,
                    "status": "completed",
                    "subject": {"reference": "Patient/ap-subject"},
                    "performedDateTime": performed,
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
    let (_, value, expected) = DATE_CASES[0];
    positive_control(
        backend,
        &tenant,
        &date_query(value, default_now()),
        expected,
        &format!("Procedure?date={value}"),
    )
    .await;

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
    for ((year, month, day), value, expected) in DATE_CASES {
        let now = Utc.with_ymd_and_hms(*year, *month, *day, 0, 0, 0).unwrap();
        let got = matched(backend, &tenant, &date_query(value, now)).await;
        if got != ids(expected) {
            failures.push(format!(
                "Procedure?date={value} (now {now}): got {got:?}, expected {expected:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
