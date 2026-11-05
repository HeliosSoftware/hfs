//! Pins how a composite parameter's *date component* compares against a
//! `Period` (issue #1391).
//!
//! #1391 made a date parameter compare as a range: a `Period` is indexed as
//! one `[start, end)` row and every prefix follows the FHIR rules for range
//! targets (see `date_period_suite`). The date component of a composite
//! parameter deliberately did **not** follow: the SQLite composite handler
//! compares it as a *point* on the row's `value_date`, which for a `Period`
//! is its start (`sqlite/search/parameter_handlers/composite.rs`, "A composite's
//! date component compares as a point"). The composite's rows sit in the
//! composite's own group and are matched per group, so they are not the
//! range-aware rows a plain date parameter searches.
//!
//! `Observation?code-value-date` is the one composite in the R4 registry
//! whose date component (`value-date`, `(Observation.value as dateTime) |
//! (Observation.value as Period)`) admits a `Period`, so it is the real
//! composite that exercises this. Each case asks the composite and the plain
//! `value-date` parameter the same date, to show the difference:
//!
//! * the composite matches on the `Period`'s **start** only: `2020` finds a
//!   Period that starts in 2020 even though it runs into 2021, and neither
//!   `gt` nor `ge` looks at the end;
//! * the plain parameter matches the whole range (`2020` needs the Period
//!   wholly inside 2020, `gt` looks at the end).
//!
//! This is a decision, not an oversight; if composite date components ever
//! become range-aware, this file is the one to change, on every backend that
//! runs it. A composite over a point value (`valueDateTime`) is unchanged by
//! #1391 and is the control that the composite itself works.
//!
//! The wrapper in `mod.rs` runs it on SQLite; the backend must be built with
//! the spec search parameters loaded.

#![allow(dead_code)]

use std::collections::BTreeSet;

use serde_json::{Value, json};

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    CompositeSearchComponent, SearchParamType, SearchParameter, SearchQuery, SearchValue,
};

const CODE: &str = "http://example.org/cpp|pin";
const OTHER_CODE: &str = "http://example.org/cpp|other";

/// (id, code, value[x] as JSON members). The range each is indexed as by the
/// plain `value-date` parameter is in the comment; the composite only sees its
/// start.
fn observations() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        // [2020-06-01, 2021-03-02): starts in 2020, ends in 2021.
        (
            "obs-span",
            "pin",
            json!({ "valuePeriod": { "start": "2020-06-01", "end": "2021-03-01" } }),
        ),
        // [2020-06-01, 2020-07-01): wholly inside 2020.
        (
            "obs-june",
            "pin",
            json!({ "valuePeriod": { "start": "2020-06-01", "end": "2020-06-30" } }),
        ),
        // [2019-03-01, +inf): starts in 2019 and never ends.
        (
            "obs-open-end",
            "pin",
            json!({ "valuePeriod": { "start": "2019-03-01" } }),
        ),
        // A point, [2020-06-15, 2020-06-16): the control that is unchanged.
        ("obs-point", "pin", json!({ "valueDateTime": "2020-06-15" })),
        // Same Period as obs-june under another code: the code component
        // still has to match in the same composite group.
        (
            "obs-other-code",
            "other",
            json!({ "valuePeriod": { "start": "2020-06-01", "end": "2020-06-30" } }),
        ),
    ]
}

/// `(date value, composite matches, plain value-date matches)`. The composite
/// is asked `pin$<value>`, the plain parameter `<value>`.
const CASES: &[(&str, &[&str], &[&str])] = &[
    // Start in 2020: span, june and the point. The plain range needs the
    // whole Period inside 2020, so span (into 2021) drops out.
    (
        "2020",
        &["obs-span", "obs-june", "obs-point"],
        &["obs-june", "obs-point", "obs-other-code"],
    ),
    // The day the Periods start on. The plain range is a whole Period, never
    // inside one day.
    ("2020-06-01", &["obs-span", "obs-june"], &[]),
    // A day inside the Periods but not their start: only the point value is
    // on it, for the composite and the plain parameter alike.
    ("2020-06-15", &["obs-point"], &["obs-point"]),
    // The end of obs-span is 2021-03-01: the composite never looks at it.
    ("2021-03-01", &[], &[]),
    // `gt` looks at the end for the plain parameter (span and open-end end
    // after 2020), and at the start for the composite (none starts later).
    ("gt2020-12-31", &[], &["obs-span", "obs-open-end"]),
    // `ge` from mid-June: the composite needs a start at or after it, the
    // plain range only an end after it or a range inside the day.
    (
        "ge2020-06-15",
        &["obs-point"],
        &[
            "obs-span",
            "obs-june",
            "obs-open-end",
            "obs-point",
            "obs-other-code",
        ],
    ),
    // `lt` looks at the start on both: only the 2019 Period starts before.
    ("lt2020-01-01", &["obs-open-end"], &["obs-open-end"]),
];

fn date_composite(value: &str) -> SearchQuery {
    SearchQuery::new("Observation")
        .with_parameter(SearchParameter {
            name: "code-value-date".to_string(),
            param_type: SearchParamType::Composite,
            values: vec![SearchValue::eq(value)],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".to_string(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Date,
                    param_name: "value-date".to_string(),
                },
            ],
            ..Default::default()
        })
        .with_count(100)
}

fn plain_date(value: &str) -> SearchQuery {
    SearchQuery::new("Observation")
        .with_parameter(SearchParameter {
            name: "value-date".to_string(),
            param_type: SearchParamType::Date,
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

/// A composite's date component compares as a point on a `Period`'s start,
/// where the plain date parameter compares the whole range.
pub async fn composite_date_component_is_a_point<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    for (id, code, value) in observations() {
        let mut resource = json!({
            "resourceType": "Observation",
            "id": id,
            "status": "final",
            "code": { "coding": [{ "system": "http://example.org/cpp", "code": code }] },
        });
        resource
            .as_object_mut()
            .unwrap()
            .extend(value.as_object().unwrap().clone());
        backend
            .create(&tenant, "Observation", resource, FhirVersion::default())
            .await
            .unwrap_or_else(|e| panic!("create Observation/{id} failed: {e}"));
    }

    // Positive control on the point value: the composite itself works.
    assert_eq!(
        matched(
            backend,
            &tenant,
            &date_composite(&format!("{CODE}$2020-06-15"))
        )
        .await,
        ids(&["obs-point"]),
        "Observation?code-value-date={CODE}$2020-06-15"
    );

    for (value, composite, plain) in CASES {
        assert_eq!(
            matched(
                backend,
                &tenant,
                &date_composite(&format!("{CODE}${value}"))
            )
            .await,
            ids(composite),
            "composite Observation?code-value-date={CODE}${value} compares the start as a point"
        );
        assert_eq!(
            matched(backend, &tenant, &plain_date(value)).await,
            ids(plain),
            "plain Observation?value-date={value} compares the range"
        );
    }

    // The code component still has to match in the same composite group.
    assert_eq!(
        matched(
            backend,
            &tenant,
            &date_composite(&format!("{OTHER_CODE}$2020"))
        )
        .await,
        ids(&["obs-other-code"]),
        "the other code's Period, and no other"
    );
}
