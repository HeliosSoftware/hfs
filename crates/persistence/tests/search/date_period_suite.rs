//! Backend-agnostic suite for date parameters on *range* targets: `Period`,
//! and a `Timing`'s `repeat.boundsPeriod` (issue #1391).
//!
//! A `Period` used to be indexed as its two ends, as unrelated points, and
//! every prefix was evaluated against each point on its own. So `date=2020`
//! found a Period that merely started or ended in 2020, `sa2020` could match
//! on the end alone, and an open end was read as an instant. Now a Period is
//! indexed as one range `[ts, te)` — a missing side open, i.e. unbounded — and
//! a search range `[s, e)` matches it by the FHIR rules for range targets:
//!
//! | prefix | matches when |
//! |---|---|
//! | `eq` | `s ≤ ts ∧ te ≤ e` |
//! | `ne` | `¬eq` |
//! | `gt` | `te > e` |
//! | `lt` | `ts < s` |
//! | `ge` | `gt ∨ eq` |
//! | `le` | `lt ∨ eq` |
//! | `sa` | `ts ≥ e` |
//! | `eb` | `te ≤ s` |
//! | `ap` | the ranges overlap once the search range is widened by its margin |
//!
//! The end of a Period is the end of the range of its `end` value, at that
//! value's own precision: `end: "2020-03-31"` runs to April 1st.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `date_precision_suite.rs`. The backend must be built with the spec search
//! parameters loaded; the suite's positive controls fail loudly if it was not.

#![allow(dead_code)]

use std::collections::BTreeSet;

use serde_json::{Value, json};

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::search::resolve_chains;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    ChainedParameter, ReverseChainedParameter, SearchParamType, SearchParameter, SearchQuery,
    SearchValue, SortDirection, SortDirective,
};

/// (id, `period`, subject). The ranges they are indexed as are in the
/// comments, `-∞`/`+∞` for an open side.
const ENCOUNTERS: &[(&str, Period, &str)] = &[
    // [2019-06-01, 2020-04-01): starts before 2020 and ends inside it.
    (
        "enc-straddle",
        (Some("2019-06-01"), Some("2020-03-31")),
        "pp-other",
    ),
    // [2020-02-01, 2020-12-01): wholly inside 2020.
    (
        "enc-within",
        (Some("2020-02-01"), Some("2020-11-30")),
        "pp-other",
    ),
    // [2020-10-01, 2021-03-02): starts in 2020 and only ends in 2021.
    (
        "enc-into-2021",
        (Some("2020-10-01"), Some("2021-03-01")),
        "pp-other",
    ),
    // [2021-01-10, 2021-02-11): after 2020.
    (
        "enc-after",
        (Some("2021-01-10"), Some("2021-02-10")),
        "pp-late",
    ),
    // [2018-01-01, 2019-01-01): before 2020.
    (
        "enc-before",
        (Some("2018-01-01"), Some("2018-12-31")),
        "pp-other",
    ),
    // [2019-03-01, +∞): still going.
    ("enc-open-end", (Some("2019-03-01"), None), "pp-ongoing"),
    // [-∞, 2019-01-16): no known start.
    ("enc-open-start", (None, Some("2019-01-15")), "pp-other"),
    // [2020-01-01, 2020-06-15T10:00:01Z): a year start and a second end.
    (
        "enc-mixed",
        (Some("2020"), Some("2020-06-15T10:00:00Z")),
        "pp-other",
    ),
];

/// A `Period` as its `start` and `end`, either absent.
type Period = (Option<&'static str>, Option<&'static str>);

fn period_json((start, end): Period) -> Value {
    let mut period = serde_json::Map::new();
    if let Some(start) = start {
        period.insert("start".into(), json!(start));
    }
    if let Some(end) = end {
        period.insert("end".into(), json!(end));
    }
    Value::Object(period)
}

/// `Encounter?date=` value → the seeded Encounters it must match.
const ENCOUNTER_CASES: &[(&str, &[&str])] = &[
    // eq: the search range has to contain the whole Period. The Periods that
    // only start or end in 2020 used to match; they must not.
    ("2020", &["enc-mixed", "enc-within"]),
    ("eq2020", &["enc-mixed", "enc-within"]),
    // A day a Period starts or ends on does not contain it.
    ("2020-02-01", &[]),
    ("2020-11-30", &[]),
    // ne is the complement of eq, open Periods included.
    (
        "ne2020",
        &[
            "enc-after",
            "enc-before",
            "enc-into-2021",
            "enc-open-end",
            "enc-open-start",
            "enc-straddle",
        ],
    ),
    // gt: the Period runs past the search range.
    ("gt2020", &["enc-after", "enc-into-2021", "enc-open-end"]),
    // lt: the Period starts before the search range.
    (
        "lt2020",
        &[
            "enc-before",
            "enc-open-end",
            "enc-open-start",
            "enc-straddle",
        ],
    ),
    (
        "ge2020",
        &[
            "enc-after",
            "enc-into-2021",
            "enc-mixed",
            "enc-open-end",
            "enc-within",
        ],
    ),
    (
        "le2020",
        &[
            "enc-before",
            "enc-mixed",
            "enc-open-end",
            "enc-open-start",
            "enc-straddle",
            "enc-within",
        ],
    ),
    // sa: the Period starts after the search range. A Period that only
    // *ends* after 2020 does not.
    ("sa2020", &["enc-after"]),
    (
        "sa2019-06-01",
        &["enc-after", "enc-into-2021", "enc-mixed", "enc-within"],
    ),
    // eb: the Period ends before the search range starts. The end is the
    // end of the range of the stored `end`, so `2020-03-31` ends on April 1st.
    ("eb2020", &["enc-before", "enc-open-start"]),
    (
        "eb2020-04-01",
        &["enc-before", "enc-open-start", "enc-straddle"],
    ),
    // An end at second precision covers that second, and no more.
    (
        "eb2020-06-15T10:00:01Z",
        &["enc-before", "enc-mixed", "enc-open-start", "enc-straddle"],
    ),
    (
        "eb2020-06-15T10:00:00Z",
        &["enc-before", "enc-open-start", "enc-straddle"],
    ),
    (
        "gt2020-06-15T10:00:00Z",
        &["enc-after", "enc-into-2021", "enc-open-end", "enc-within"],
    ),
    // Open ends are unbounded, not instants.
    ("gt2030", &["enc-open-end"]),
    ("lt1900", &["enc-open-start"]),
    ("sa2030", &[]),
    ("eb1900", &[]),
    // ap: the ranges overlap once the search range is widened by a day
    // (day precision) or a month (month precision) on each side.
    (
        "ap2020-03-15",
        &["enc-mixed", "enc-open-end", "enc-straddle", "enc-within"],
    ),
    ("ap2018-06", &["enc-before", "enc-open-start"]),
];

/// (id, `performedPeriod`): the same shapes on `Procedure.performed[x]`.
const PROCEDURES: &[(&str, Period)] = &[
    ("proc-straddle", (Some("2019-06-01"), Some("2020-03-31"))),
    ("proc-within", (Some("2020-02-01"), Some("2020-11-30"))),
    ("proc-open-end", (Some("2019-03-01"), None)),
];

const PROCEDURE_CASES: &[(&str, &[&str])] = &[
    ("2020", &["proc-within"]),
    ("lt2020", &["proc-open-end", "proc-straddle"]),
    ("gt2020", &["proc-open-end"]),
    ("sa2020", &[]),
    ("gt2030", &["proc-open-end"]),
];

/// (id, `occurrenceTiming.repeat.boundsPeriod`): a Timing used to index only
/// the start of its bounds.
const SERVICE_REQUESTS: &[(&str, Period)] = &[
    ("sr-straddle", (Some("2019-06-01"), Some("2020-03-31"))),
    ("sr-within", (Some("2020-02-01"), Some("2020-11-30"))),
    ("sr-open-end", (Some("2019-03-01"), None)),
];

const SERVICE_REQUEST_CASES: &[(&str, &[&str])] = &[
    ("2020", &["sr-within"]),
    ("lt2020", &["sr-open-end", "sr-straddle"]),
    ("gt2020", &["sr-open-end"]),
    ("eb2020", &[]),
    ("gt2030", &["sr-open-end"]),
];

/// (id, the Encounter it points at): for the forward chain.
const OBSERVATIONS: &[(&str, &str)] = &[
    ("obs-straddle", "enc-straddle"),
    ("obs-within", "enc-within"),
    ("obs-open-end", "enc-open-end"),
];

const PATIENTS: &[&str] = &["pp-late", "pp-ongoing", "pp-other"];

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

/// `Observation?encounter:Encounter.date=<value>`, raw as the REST layer
/// hands it over: the resolver parses the prefix for the terminal's type.
fn forward_chain(value: &str) -> SearchQuery {
    SearchQuery::new("Observation")
        .with_parameter(SearchParameter {
            name: "encounter".to_string(),
            param_type: SearchParamType::Reference,
            values: vec![SearchValue::eq(value)],
            chain: vec![ChainedParameter {
                reference_param: "encounter".to_string(),
                target_type: Some("Encounter".to_string()),
                target_param: "date".to_string(),
            }],
            ..Default::default()
        })
        .with_count(100)
}

/// `Patient?_has:Encounter:subject:date=<value>`.
fn reverse_chain(value: &str) -> SearchQuery {
    let mut query = SearchQuery::new("Patient").with_count(100);
    query.reverse_chains = vec![ReverseChainedParameter::terminal(
        "Encounter",
        "subject",
        "date",
        SearchValue::eq(value),
    )];
    query
}

fn sorted_by_date(direction: SortDirection) -> SearchQuery {
    SearchQuery::new("Encounter")
        .with_sort(SortDirective {
            parameter: "date".to_string(),
            direction,
            param_type: Some(SearchParamType::Date),
        })
        .with_count(100)
}

async fn found<S>(backend: &S, tenant: &TenantContext, query: &SearchQuery) -> Vec<String>
where
    S: ResourceStorage + SearchProvider,
{
    let query = resolve_chains(backend, tenant, query)
        .await
        .unwrap_or_else(|e| panic!("resolving the chains of {query:?} failed: {e}"));
    backend
        .search(tenant, &query)
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
        .create(tenant, resource_type, resource, FhirVersion::default())
        .await
        .unwrap_or_else(|e| panic!("create {resource_type}/{id} failed: {e}"));
}

/// Waits until `expected` resources of the type are searchable by a token the
/// seed gave them all: the positive control that the resources are stored and
/// their index written, and the wait for eventually-consistent indexes.
async fn wait_for<S>(
    backend: &S,
    tenant: &TenantContext,
    resource_type: &str,
    param: &str,
    value: &str,
    expected: usize,
) where
    S: ResourceStorage + SearchProvider,
{
    let control = query(resource_type, param, SearchParamType::Token, value);
    for attempt in 0..60 {
        let visible = matched(backend, tenant, &control).await;
        if visible.len() == expected {
            return;
        }
        assert!(
            attempt < 59,
            "the seeded {resource_type}s never became searchable by {param} ({}/{expected}): \
             is the backend built with the spec search parameters?",
            visible.len(),
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

async fn seed<S>(backend: &S, tenant: &TenantContext)
where
    S: ResourceStorage + SearchProvider,
{
    for id in PATIENTS {
        create(
            backend,
            tenant,
            "Patient",
            json!({"id": id, "active": true}),
        )
        .await;
    }
    for (id, period, subject) in ENCOUNTERS {
        create(
            backend,
            tenant,
            "Encounter",
            json!({
                "id": id,
                "status": "finished",
                "class": {
                    "system": "http://terminology.hl7.org/CodeSystem/v3-ActCode",
                    "code": "AMB"
                },
                "subject": {"reference": format!("Patient/{subject}")},
                "period": period_json(*period),
            }),
        )
        .await;
    }
    for (id, period) in PROCEDURES {
        create(
            backend,
            tenant,
            "Procedure",
            json!({
                "id": id,
                "status": "completed",
                "subject": {"reference": "Patient/pp-other"},
                "performedPeriod": period_json(*period),
            }),
        )
        .await;
    }
    for (id, bounds) in SERVICE_REQUESTS {
        create(
            backend,
            tenant,
            "ServiceRequest",
            json!({
                "id": id,
                "status": "active",
                "intent": "order",
                "subject": {"reference": "Patient/pp-other"},
                "occurrenceTiming": {"repeat": {"boundsPeriod": period_json(*bounds)}},
            }),
        )
        .await;
    }
    for (id, encounter) in OBSERVATIONS {
        create(
            backend,
            tenant,
            "Observation",
            json!({
                "id": id,
                "status": "final",
                "code": {"text": "period suite"},
                "subject": {"reference": "Patient/pp-other"},
                "encounter": {"reference": format!("Encounter/{encounter}")},
            }),
        )
        .await;
    }

    wait_for(backend, tenant, "Patient", "active", "true", PATIENTS.len()).await;
    wait_for(
        backend,
        tenant,
        "Encounter",
        "status",
        "finished",
        ENCOUNTERS.len(),
    )
    .await;
    wait_for(
        backend,
        tenant,
        "Procedure",
        "status",
        "completed",
        PROCEDURES.len(),
    )
    .await;
    wait_for(
        backend,
        tenant,
        "ServiceRequest",
        "status",
        "active",
        SERVICE_REQUESTS.len(),
    )
    .await;
    wait_for(
        backend,
        tenant,
        "Observation",
        "status",
        "final",
        OBSERVATIONS.len(),
    )
    .await;
}

/// Seeds the resources under a caller-unique tenant and asserts every table:
/// each prefix against `Encounter.period`, `Procedure.performedPeriod` and a
/// `ServiceRequest`'s `occurrenceTiming.repeat.boundsPeriod`; `_sort` both
/// ways; and a Period as the terminal of a forward chain and of `_has`.
pub async fn period_targets_are_ranges<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    seed(backend, &tenant).await;

    // Positive control: every Encounter has a date index entry, whichever
    // side of its Period is open. No Period lies within 1800, so `ne` finds
    // them all.
    let all_encounters: Vec<&str> = ENCOUNTERS.iter().map(|(id, _, _)| *id).collect();
    assert_eq!(
        matched(
            backend,
            &tenant,
            &query("Encounter", "date", SearchParamType::Date, "ne1800")
        )
        .await,
        ids(&all_encounters),
        "Encounter?date=ne1800"
    );

    for (value, expected) in ENCOUNTER_CASES {
        assert_eq!(
            matched(
                backend,
                &tenant,
                &query("Encounter", "date", SearchParamType::Date, value)
            )
            .await,
            ids(expected),
            "Encounter?date={value}"
        );
    }

    for (value, expected) in PROCEDURE_CASES {
        assert_eq!(
            matched(
                backend,
                &tenant,
                &query("Procedure", "date", SearchParamType::Date, value)
            )
            .await,
            ids(expected),
            "Procedure?date={value} (performedPeriod)"
        );
    }

    for (value, expected) in SERVICE_REQUEST_CASES {
        assert_eq!(
            matched(
                backend,
                &tenant,
                &query("ServiceRequest", "occurrence", SearchParamType::Date, value)
            )
            .await,
            ids(expected),
            "ServiceRequest?occurrence={value} (occurrenceTiming.repeat.boundsPeriod)"
        );
    }

    // _sort=date orders by where each Period starts, an open start first;
    // _sort=-date by where it ends, an open end first.
    assert_eq!(
        found(backend, &tenant, &sorted_by_date(SortDirection::Ascending)).await,
        [
            "enc-open-start",
            "enc-before",
            "enc-open-end",
            "enc-straddle",
            "enc-mixed",
            "enc-within",
            "enc-into-2021",
            "enc-after",
        ],
        "Encounter?_sort=date"
    );
    assert_eq!(
        found(backend, &tenant, &sorted_by_date(SortDirection::Descending)).await,
        [
            "enc-open-end",
            "enc-into-2021",
            "enc-after",
            "enc-within",
            "enc-mixed",
            "enc-straddle",
            "enc-open-start",
            "enc-before",
        ],
        "Encounter?_sort=-date"
    );

    // A Period as the terminal of a forward chain.
    for (value, expected) in [
        ("2020", &["obs-within"][..]),
        ("gt2030", &["obs-open-end"]),
        ("sa2020", &[]),
        ("lt2020", &["obs-open-end", "obs-straddle"]),
    ] {
        assert_eq!(
            matched(backend, &tenant, &forward_chain(value)).await,
            ids(expected),
            "Observation?encounter:Encounter.date={value}"
        );
    }

    // ... and of `_has`.
    for (value, expected) in [
        ("gt2030", &["pp-ongoing"][..]),
        ("sa2020", &["pp-late"]),
        ("2020", &["pp-other"]),
        ("sa2030", &[]),
    ] {
        assert_eq!(
            matched(backend, &tenant, &reverse_chain(value)).await,
            ids(expected),
            "Patient?_has:Encounter:subject:date={value}"
        );
    }
}
