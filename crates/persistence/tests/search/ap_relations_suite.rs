//! Backend-agnostic suite for the `ap` (approximately) search prefix where it
//! is not the top-level component of a plain parameter (issue #1390):
//!
//! - **composite** parameters (`code-value-quantity`, `component-code-value-
//!   quantity`, `code-value-date`) whose quantity or date component carries
//!   `ap`, on every backend;
//! - **chained** (`Procedure?subject:Patient.birthdate=ap2016`) and reverse
//!   chained (`Patient?_has:Procedure:subject:date=ap2016`) searches, whose
//!   terminal sub-search must measure the window from the outer query's `now`
//!   (SQLite and PostgreSQL; MongoDB and Elasticsearch reject chains);
//! - **`_filter`** date `ap` end to end (SQLite).
//!
//! The rule itself is the one `ap_prefix_suite.rs` pins for plain parameters:
//!
//! - number / quantity: `[v - m, v + m]`, `m = max(10% of |v|, half the
//!   implicit precision)`;
//! - date: the value's precision range widened on both sides by a tenth of
//!   the gap between `now` and that range (zero when `now` is inside it).
//!
//! Every date case pins `now` with [`SearchQuery::with_now`] — never the wall
//! clock — and every seed sits weeks or more away from every window edge, so
//! neither storage rounding nor floating point can flip a case. Each table
//! starts with a positive control (a range no window is involved in), which
//! fails loudly when the backend was built without the spec search parameters.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `ap_prefix_suite.rs`.

#![allow(dead_code)]

use std::collections::BTreeSet;

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{Value, json};

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::search::resolve_chains;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    ChainedParameter, CompositeSearchComponent, ReverseChainedParameter, SearchParamType,
    SearchParameter, SearchQuery, SearchValue,
};

const LOINC: &str = "http://loinc.org";
const UCUM: &str = "http://unitsofmeasure.org";

// Two codes, so that a composite whose token component picks the wrong
// observation is visible.
const CODE_A: &str = "ap-code-a";
const CODE_B: &str = "ap-code-b";

fn tenant(base: &str) -> TenantContext {
    TenantContext::new(TenantId::new(base), TenantPermissions::full_access())
}

fn ids(expected: &[&str]) -> BTreeSet<String> {
    expected.iter().map(|id| id.to_string()).collect()
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

/// Like [`positive_control`], for a chain: the chain is resolved on every
/// poll, since the resources it resolves through are near-real-time too.
async fn chain_positive_control<S>(
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
        got = resolved(backend, tenant, control).await;
        if got == ids(expected) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    assert_eq!(got, ids(expected), "positive control {label}");
}

async fn create<S>(backend: &S, tenant: &TenantContext, resource_type: &str, resource: Value)
where
    S: ResourceStorage + SearchProvider,
{
    let id = resource["id"].as_str().unwrap_or_default().to_string();
    backend
        .create(tenant, resource_type, resource, FhirVersion::default())
        .await
        .unwrap_or_else(|e| panic!("create {resource_type}/{id} failed: {e}"));
}

fn now_at((year, month, day): (i32, u32, u32)) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
}

fn code_component() -> CompositeSearchComponent {
    CompositeSearchComponent {
        param_type: SearchParamType::Token,
        param_name: "code".to_string(),
    }
}

/// A composite query with the components the registry would supply. The value
/// is taken literally: the backend splits it on `$` and reads each part's
/// prefix itself.
fn composite(
    param: &str,
    components: Vec<CompositeSearchComponent>,
    value: &str,
    now: DateTime<Utc>,
) -> SearchQuery {
    SearchQuery::new("Observation")
        .with_parameter(SearchParameter {
            name: param.to_string(),
            param_type: SearchParamType::Composite,
            values: vec![SearchValue::eq(value)],
            components,
            ..Default::default()
        })
        .with_count(100)
        .with_now(now)
}

fn code_value_quantity(value: &str) -> SearchQuery {
    composite(
        "code-value-quantity",
        vec![
            code_component(),
            CompositeSearchComponent {
                param_type: SearchParamType::Quantity,
                param_name: "value-quantity".to_string(),
            },
        ],
        value,
        now_at((2026, 1, 1)),
    )
}

fn component_code_value_quantity(value: &str) -> SearchQuery {
    composite(
        "component-code-value-quantity",
        vec![
            CompositeSearchComponent {
                param_type: SearchParamType::Token,
                param_name: "component-code".to_string(),
            },
            CompositeSearchComponent {
                param_type: SearchParamType::Quantity,
                param_name: "component-value-quantity".to_string(),
            },
        ],
        value,
        now_at((2026, 1, 1)),
    )
}

fn code_value_date(value: &str, now: DateTime<Utc>) -> SearchQuery {
    composite(
        "code-value-date",
        vec![
            code_component(),
            CompositeSearchComponent {
                param_type: SearchParamType::Date,
                param_name: "value-date".to_string(),
            },
        ],
        value,
        now,
    )
}

// ---------------------------------------------------------------------------
// Composites
// ---------------------------------------------------------------------------

/// (id, code, `valueQuantity.value` in mg).
const COMPOSITE_QUANTITIES: &[(&str, &str, f64)] = &[
    ("ap-cq-a-089", CODE_A, 89.0),
    ("ap-cq-a-091", CODE_A, 91.0),
    ("ap-cq-a-100", CODE_A, 100.0),
    ("ap-cq-a-109", CODE_A, 109.0),
    ("ap-cq-a-111", CODE_A, 111.0),
    // Inside the window, under the other code.
    ("ap-cq-b-091", CODE_B, 91.0),
    ("ap-cq-b-100", CODE_B, 100.0),
    ("ap-cq-b-160", CODE_B, 160.0),
];

/// `code-value-quantity=` value → the Observations it must match. The first
/// row is the positive control.
const CODE_VALUE_QUANTITY_CASES: &[(&str, &[&str])] = &[
    (
        "ap-code-a$ge0",
        &[
            "ap-cq-a-089",
            "ap-cq-a-091",
            "ap-cq-a-100",
            "ap-cq-a-109",
            "ap-cq-a-111",
        ],
    ),
    // 10% of the value: [90, 110]. The other code's 91 and 100 stay out.
    (
        "ap-code-a$ap100",
        &["ap-cq-a-091", "ap-cq-a-100", "ap-cq-a-109"],
    ),
    ("ap-code-b$ap100", &["ap-cq-b-091", "ap-cq-b-100"]),
    // One significant figure: [50, 150].
    (
        "ap-code-a$ap1e2",
        &[
            "ap-cq-a-089",
            "ap-cq-a-091",
            "ap-cq-a-100",
            "ap-cq-a-109",
            "ap-cq-a-111",
        ],
    ),
    ("ap-code-b$ap1e2", &["ap-cq-b-091", "ap-cq-b-100"]),
    // Nothing of code A is near 160, though code B has one.
    ("ap-code-a$ap160", &[]),
    ("ap-code-b$ap160", &["ap-cq-b-160"]),
];

/// (id, [(component code, value in mg)]). Both components of an Observation
/// are in the window of one of the queries below, but only when they belong
/// to the same component do they match together.
const COMPONENT_OBSERVATIONS: &[(&str, [(&str, f64); 2])] = &[
    ("ap-cc-1", [(CODE_A, 100.0), (CODE_B, 200.0)]),
    ("ap-cc-2", [(CODE_A, 200.0), (CODE_B, 100.0)]),
    ("ap-cc-3", [(CODE_A, 109.0), (CODE_B, 91.0)]),
    ("ap-cc-4", [(CODE_A, 160.0), (CODE_B, 160.0)]),
];

/// `component-code-value-quantity=` value → the Observations it must match.
/// The first row is the positive control.
const COMPONENT_CASES: &[(&str, &[&str])] = &[
    (
        "ap-code-a$ge0",
        &["ap-cc-1", "ap-cc-2", "ap-cc-3", "ap-cc-4"],
    ),
    // `ap-cc-2` has a 100 too, but under the other code.
    ("ap-code-a$ap100", &["ap-cc-1", "ap-cc-3"]),
    ("ap-code-b$ap100", &["ap-cc-2", "ap-cc-3"]),
    ("ap-code-a$ap200", &["ap-cc-2"]),
    ("ap-code-b$ap160", &["ap-cc-4"]),
];

/// (id, code, `valueDateTime`), or a `valuePeriod` as (start, end).
enum Dated {
    Instant(&'static str),
    Period(&'static str, &'static str),
}

const COMPOSITE_DATES: &[(&str, &str, Dated)] = &[
    (
        "ap-cd-a-2013",
        CODE_A,
        Dated::Instant("2013-06-15T12:00:00Z"),
    ),
    (
        "ap-cd-a-2015",
        CODE_A,
        Dated::Instant("2015-06-15T12:00:00Z"),
    ),
    (
        "ap-cd-a-2016",
        CODE_A,
        Dated::Instant("2016-06-15T12:00:00Z"),
    ),
    (
        "ap-cd-a-2017",
        CODE_A,
        Dated::Instant("2017-06-15T12:00:00Z"),
    ),
    (
        "ap-cd-a-2019",
        CODE_A,
        Dated::Instant("2019-06-15T12:00:00Z"),
    ),
    (
        "ap-cd-a-per-2016",
        CODE_A,
        Dated::Period("2016-03-01T00:00:00Z", "2016-09-01T00:00:00Z"),
    ),
    (
        "ap-cd-a-per-2019",
        CODE_A,
        Dated::Period("2019-03-01T00:00:00Z", "2019-09-01T00:00:00Z"),
    ),
    // The other code, in the same windows.
    (
        "ap-cd-b-2016",
        CODE_B,
        Dated::Instant("2016-06-15T12:00:00Z"),
    ),
    (
        "ap-cd-b-2019",
        CODE_B,
        Dated::Instant("2019-06-15T12:00:00Z"),
    ),
    // A date far from every window below.
    (
        "ap-cd-a-2040",
        CODE_A,
        Dated::Instant("2040-06-15T12:00:00Z"),
    ),
];

/// (`now`, `code-value-date=` value, the Observations it must match). The
/// first row is the positive control.
type CompositeDateCase = ((i32, u32, u32), &'static str, &'static [&'static str]);

const CODE_VALUE_DATE_CASES: &[CompositeDateCase] = &[
    (
        (2026, 1, 1),
        "ap-code-a$ge1900-01-01",
        &[
            "ap-cd-a-2013",
            "ap-cd-a-2015",
            "ap-cd-a-2016",
            "ap-cd-a-2017",
            "ap-cd-a-2019",
            "ap-cd-a-per-2016",
            "ap-cd-a-per-2019",
            "ap-cd-a-2040",
        ],
    ),
    // [2016, 2017) is 3287 days before `now`: a 328.7-day margin,
    // [~2015-02-06, ~2017-11-25). A period matches when it overlaps.
    (
        (2026, 1, 1),
        "ap-code-a$ap2016",
        &[
            "ap-cd-a-2015",
            "ap-cd-a-2016",
            "ap-cd-a-2017",
            "ap-cd-a-per-2016",
        ],
    ),
    // The other code's dates in the same window stay out of code A's answer.
    ((2026, 1, 1), "ap-code-b$ap2016", &["ap-cd-b-2016"]),
    // From 2046-01-01 the gap is 10592 days, the margin 1059.2: [~2013-02-05,
    // ~2019-11-26). The window widens with `now`.
    (
        (2046, 1, 1),
        "ap-code-a$ap2016",
        &[
            "ap-cd-a-2013",
            "ap-cd-a-2015",
            "ap-cd-a-2016",
            "ap-cd-a-2017",
            "ap-cd-a-2019",
            "ap-cd-a-per-2016",
            "ap-cd-a-per-2019",
        ],
    ),
    (
        (2046, 1, 1),
        "ap-code-b$ap2016",
        &["ap-cd-b-2016", "ap-cd-b-2019"],
    ),
    // `now` inside the range: no margin, `ap` is `eq`.
    (
        (2016, 6, 1),
        "ap-code-a$ap2016",
        &["ap-cd-a-2016", "ap-cd-a-per-2016"],
    ),
    // Future, year precision: [2040, 2041) is 5113 days after `now`, a 511.3
    // day margin: [~2038-08, ~2042-05).
    ((2026, 1, 1), "ap-code-a$ap2040", &["ap-cd-a-2040"]),
    ((2026, 1, 1), "ap-code-b$ap2040", &[]),
];

/// Seeds the composite fixtures and asserts every table: `ap` in the quantity
/// component of `code-value-quantity`, the grouped quantity component of
/// `component-code-value-quantity`, and the date component of
/// `code-value-date`, each next to a token component that must select the
/// same element.
pub async fn ap_composite<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = tenant(tenant_base);

    for (id, code, dose) in COMPOSITE_QUANTITIES {
        create(
            backend,
            &tenant,
            "Observation",
            json!({
                "id": id,
                "status": "final",
                "code": {"coding": [{"system": LOINC, "code": code}]},
                "valueQuantity": {
                    "value": dose, "unit": "mg", "system": UCUM, "code": "mg",
                },
            }),
        )
        .await;
    }
    for (id, components) in COMPONENT_OBSERVATIONS {
        let components: Vec<Value> = components
            .iter()
            .map(|(code, dose)| {
                json!({
                    "code": {"coding": [{"system": LOINC, "code": code}]},
                    "valueQuantity": {
                        "value": dose, "unit": "mg", "system": UCUM, "code": "mg",
                    },
                })
            })
            .collect();
        create(
            backend,
            &tenant,
            "Observation",
            json!({
                "id": id,
                "status": "final",
                "code": {"coding": [{"system": LOINC, "code": "ap-panel"}]},
                "component": components,
            }),
        )
        .await;
    }
    for (id, code, dated) in COMPOSITE_DATES {
        let mut observation = json!({
            "id": id,
            "status": "final",
            "code": {"coding": [{"system": LOINC, "code": code}]},
        });
        match dated {
            Dated::Instant(value) => observation["valueDateTime"] = json!(value),
            Dated::Period(start, end) => {
                observation["valuePeriod"] = json!({"start": start, "end": end});
            }
        }
        create(backend, &tenant, "Observation", observation).await;
    }

    let (value, expected) = CODE_VALUE_QUANTITY_CASES[0];
    positive_control(
        backend,
        &tenant,
        &code_value_quantity(value),
        expected,
        &format!("Observation?code-value-quantity={value}"),
    )
    .await;
    let (value, expected) = COMPONENT_CASES[0];
    positive_control(
        backend,
        &tenant,
        &component_code_value_quantity(value),
        expected,
        &format!("Observation?component-code-value-quantity={value}"),
    )
    .await;
    let (now, value, expected) = CODE_VALUE_DATE_CASES[0];
    positive_control(
        backend,
        &tenant,
        &code_value_date(value, now_at(now)),
        expected,
        &format!("Observation?code-value-date={value}"),
    )
    .await;

    let mut failures = Vec::new();
    for (value, expected) in CODE_VALUE_QUANTITY_CASES {
        let got = matched(backend, &tenant, &code_value_quantity(value)).await;
        if got != ids(expected) {
            failures.push(format!(
                "Observation?code-value-quantity={value}: got {got:?}, expected {expected:?}"
            ));
        }
    }
    for (value, expected) in COMPONENT_CASES {
        let got = matched(backend, &tenant, &component_code_value_quantity(value)).await;
        if got != ids(expected) {
            failures.push(format!(
                "Observation?component-code-value-quantity={value}: got {got:?}, \
                 expected {expected:?}"
            ));
        }
    }
    for (now, value, expected) in CODE_VALUE_DATE_CASES {
        let now = now_at(*now);
        let got = matched(backend, &tenant, &code_value_date(value, now)).await;
        if got != ids(expected) {
            failures.push(format!(
                "Observation?code-value-date={value} (now {now}): got {got:?}, \
                 expected {expected:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

// ---------------------------------------------------------------------------
// Chains and _has
// ---------------------------------------------------------------------------

/// The years of the Patients' `birthDate` and the Procedures' `performed`
/// (`YYYY-06-15`), on both sides of every window below.
const YEARS: &[i32] = &[2013, 2015, 2016, 2017, 2019, 2036, 2039, 2041, 2044];

/// Patients `ap-p-<year>`, each with a Procedure `ap-d-<year>` whose subject
/// they are.
async fn seed_patients_and_procedures<S>(backend: &S, tenant: &TenantContext)
where
    S: ResourceStorage + SearchProvider,
{
    for year in YEARS {
        create(
            backend,
            tenant,
            "Patient",
            json!({"id": format!("ap-p-{year}"), "birthDate": format!("{year}-06-15")}),
        )
        .await;
        create(
            backend,
            tenant,
            "Procedure",
            json!({
                "id": format!("ap-d-{year}"),
                "status": "completed",
                "subject": {"reference": format!("Patient/ap-p-{year}")},
                "performedDateTime": format!("{year}-06-15T12:00:00Z"),
            }),
        )
        .await;
    }
}

/// `prefix` + the years, e.g. `("ap-d-", [2015, 2016])` → `ap-d-2015`, ...
fn years(prefix: &str, years: &[i32]) -> Vec<String> {
    years.iter().map(|y| format!("{prefix}{y}")).collect()
}

/// (`now`, value, the years it must match). The first row is the positive
/// control. Each year's seed is at least three weeks from every edge.
type ChainCase = ((i32, u32, u32), &'static str, &'static [i32]);

const CHAIN_CASES: &[ChainCase] = &[
    (
        (2026, 1, 1),
        "ge1900-01-01",
        &[2013, 2015, 2016, 2017, 2019, 2036, 2039, 2041, 2044],
    ),
    // [2016, 2017) is 3287 days before `now`: [~2015-02-06, ~2017-11-25).
    ((2026, 1, 1), "ap2016", &[2015, 2016, 2017]),
    // 10592 days from 2046-01-01: [~2013-02-05, ~2019-11-26). A different
    // `now` widens the very same chain, so it is threaded through and the
    // wall clock is not read.
    ((2046, 1, 1), "ap2016", &[2013, 2015, 2016, 2017, 2019]),
    // `now` inside the range: no margin.
    ((2016, 6, 1), "ap2016", &[2016]),
    // Future, from 2026-01-01: [~2034-12-31, ~2038-01-01).
    ((2026, 1, 1), "ap2036", &[2036]),
    // From 2005-01-01 the gap to [2036, 2037) is 11322 days: a margin of
    // 1132 days, [~2032-11-24, ~2040-02-07).
    ((2005, 1, 1), "ap2036", &[2036, 2039]),
    // Gap 12418 days to [2039, 2040): [~2035-08-08, ~2043-05-26).
    ((2005, 1, 1), "ap2039", &[2036, 2039, 2041]),
];

async fn resolved<S>(backend: &S, tenant: &TenantContext, query: &SearchQuery) -> BTreeSet<String>
where
    S: ResourceStorage + SearchProvider,
{
    let rewritten = resolve_chains(backend, tenant, query)
        .await
        .unwrap_or_else(|e| panic!("resolve {query:?} failed: {e}"));
    matched(backend, tenant, &rewritten).await
}

/// `Procedure?subject:Patient.birthdate=<value>`, measured from `now`.
fn forward_chain(value: &str, now: DateTime<Utc>) -> SearchQuery {
    SearchQuery::new("Procedure")
        .with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            values: vec![SearchValue::eq(value)],
            chain: vec![ChainedParameter {
                reference_param: "subject".to_string(),
                target_type: Some("Patient".to_string()),
                target_param: "birthdate".to_string(),
            }],
            ..Default::default()
        })
        .with_count(100)
        .with_now(now)
}

/// `Patient?_has:Procedure:subject:date=<value>`, measured from `now`.
fn reverse_chain(value: &str, now: DateTime<Utc>) -> SearchQuery {
    let mut query = SearchQuery::new("Patient").with_count(100).with_now(now);
    query.reverse_chains.push(ReverseChainedParameter::terminal(
        "Procedure",
        "subject",
        "date",
        SearchValue::eq(value),
    ));
    query
}

/// `ap` on the terminal of a forward chain and of a `_has`, with the `now` of
/// the outer query. For SQLite and PostgreSQL: MongoDB and Elasticsearch
/// reject chains and `_has`.
pub async fn ap_chained<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = tenant(tenant_base);
    seed_patients_and_procedures(backend, &tenant).await;

    let (now, value, expected) = CHAIN_CASES[0];
    let procedures = years("ap-d-", expected);
    let patients = years("ap-p-", expected);
    chain_positive_control(
        backend,
        &tenant,
        &forward_chain(value, now_at(now)),
        &procedures.iter().map(String::as_str).collect::<Vec<_>>(),
        &format!("Procedure?subject:Patient.birthdate={value}"),
    )
    .await;
    chain_positive_control(
        backend,
        &tenant,
        &reverse_chain(value, now_at(now)),
        &patients.iter().map(String::as_str).collect::<Vec<_>>(),
        &format!("Patient?_has:Procedure:subject:date={value}"),
    )
    .await;

    let mut failures = Vec::new();
    for (now, value, expected) in CHAIN_CASES {
        let now = now_at(*now);
        let expected_procedures: BTreeSet<String> = years("ap-d-", expected).into_iter().collect();
        let expected_patients: BTreeSet<String> = years("ap-p-", expected).into_iter().collect();

        let got = resolved(backend, &tenant, &forward_chain(value, now)).await;
        if got != expected_procedures {
            failures.push(format!(
                "Procedure?subject:Patient.birthdate={value} (now {now}): got {got:?}, \
                 expected {expected_procedures:?}"
            ));
        }
        let got = resolved(backend, &tenant, &reverse_chain(value, now)).await;
        if got != expected_patients {
            failures.push(format!(
                "Patient?_has:Procedure:subject:date={value} (now {now}): got {got:?}, \
                 expected {expected_patients:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

// ---------------------------------------------------------------------------
// _filter
// ---------------------------------------------------------------------------

/// `Procedure?_filter=date ap <value>`, measured from `now`.
fn filter_query(expression: &str, now: DateTime<Utc>) -> SearchQuery {
    SearchQuery::new("Procedure")
        .with_parameter(SearchParameter {
            name: "_filter".to_string(),
            param_type: SearchParamType::Special,
            values: vec![SearchValue::eq(expression)],
            ..Default::default()
        })
        .with_count(100)
        .with_now(now)
}

/// `_filter` date `ap` end to end, through the parser, the SQL generator and
/// the store, with a pinned `now`. `_filter` is an SQLite feature.
///
/// (`_filter` `ap` on a non-date column is still a text `LIKE '%v%'`; that is
/// a known gap, deliberately not asserted here.)
pub async fn ap_filter_date<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = tenant(tenant_base);
    seed_patients_and_procedures(backend, &tenant).await;

    let (now, value, expected) = CHAIN_CASES[0];
    let expected: Vec<String> = years("ap-d-", expected);
    positive_control(
        backend,
        &tenant,
        &filter_query(&format!("date ge {}", &value[2..]), now_at(now)),
        &expected.iter().map(String::as_str).collect::<Vec<_>>(),
        &format!("Procedure?_filter=date ge {}", &value[2..]),
    )
    .await;

    let mut failures = Vec::new();
    for (now, value, expected) in &CHAIN_CASES[1..] {
        let now = now_at(*now);
        let expected: BTreeSet<String> = years("ap-d-", expected).into_iter().collect();
        let expression = format!("date ap {}", value.strip_prefix("ap").unwrap());
        let got = matched(backend, &tenant, &filter_query(&expression, now)).await;
        if got != expected {
            failures.push(format!(
                "Procedure?_filter={expression} (now {now}): got {got:?}, expected {expected:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
