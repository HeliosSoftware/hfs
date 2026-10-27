//! Backend-agnostic date search value suite: one grammar, one precision range
//! (issues #1293, #1295, #1296, #1297).
//!
//! The day-precision table in `date_boundary_suite.rs` (#519) made every
//! backend agree on `YYYY-MM-DD`. Below a day they still disagreed, and on
//! what a date even is they disagreed completely:
//!
//! - `date=2013-04-05T23:30:00-04:00` against a stored `…:00.123-04:00`
//!   matched on SQLite and Elasticsearch, not on MongoDB or PostgreSQL (#1297);
//! - `date=gtnot-a-date` was "after the year 2000" on Elasticsearch (#1293),
//!   "after now" on PostgreSQL (#1289), nothing on SQLite, a 400 on MongoDB;
//! - `date=lt2024-02-30` was `lt 2024-03-01` on SQLite (#1295);
//! - `date=2013-04-05T18:50:00+05:30` with a literal `+` arrives with a space
//!   and failed four different ways (#1296).
//!
//! All of that now goes through `helios_persistence::search::FhirDateValue`,
//! and this suite drives each backend's *real* [`SearchProvider::search`] path
//! to hold them to the same answers.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `date_boundary_suite.rs`. The backend must be built with the spec search
//! parameters loaded: without them `Procedure.date` indexes nothing and every
//! negative assertion here would pass vacuously — which is why the suite starts
//! with a positive control and refuses to go on without it.

#![allow(dead_code)]

use std::collections::BTreeSet;

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::error::{SearchError, StorageError};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

/// Stored with a fraction, so a second-precision search has something inside
/// its second to find (#1297). `2013-04-06T03:30:00.123Z`.
const FRACTION: (&str, &str) = ("dp-fraction", "2013-04-05T23:30:00.123-04:00");
/// Stored on the minute. `2013-04-05T13:20:00Z`.
const MINUTE: (&str, &str) = ("dp-minute", "2013-04-05T09:20:00-04:00");

/// `date=` value → the seeded Procedures it must match.
const CASES: &[(&str, &[&str])] = &[
    // #1297: second precision is the whole second, under every prefix.
    ("2013-04-05T23:30:00-04:00", &["dp-fraction"]),
    ("eq2013-04-05T23:30:00-04:00", &["dp-fraction"]),
    ("ne2013-04-05T23:30:00-04:00", &["dp-minute"]),
    ("gt2013-04-05T23:30:00-04:00", &[]),
    ("sa2013-04-05T23:30:00-04:00", &[]),
    ("lt2013-04-05T23:30:00-04:00", &["dp-minute"]),
    ("eb2013-04-05T23:30:00-04:00", &["dp-minute"]),
    ("ge2013-04-05T23:30:00-04:00", &["dp-fraction"]),
    ("le2013-04-05T23:30:00-04:00", &["dp-fraction", "dp-minute"]),
    // The neighbouring seconds do not contain it.
    ("2013-04-05T23:29:59-04:00", &[]),
    ("2013-04-05T23:30:01-04:00", &[]),
    ("gt2013-04-05T23:29:59-04:00", &["dp-fraction"]),
    // Minute precision is valid in search and is the whole minute.
    ("2013-04-05T23:30-04:00", &["dp-fraction"]),
    ("2013-04-05T09:20-04:00", &["dp-minute"]),
    ("2013-04-05T13:20", &["dp-minute"]),
    ("2013-04-05T09:21-04:00", &[]),
    ("gt2013-04-05T09:20-04:00", &["dp-fraction"]),
    ("le2013-04-05T09:20-04:00", &["dp-minute"]),
    // A fraction is matched at the fraction.
    ("2013-04-05T23:30:00.1-04:00", &["dp-fraction"]),
    ("eq2013-04-05T23:30:00.12-04:00", &["dp-fraction"]),
    ("ne2013-04-05T23:30:00.1-04:00", &["dp-minute"]),
    ("gt2013-04-05T23:30:00.1-04:00", &[]),
    ("sa2013-04-05T23:30:00.12-04:00", &[]),
    (
        "le2013-04-05T23:30:00.12-04:00",
        &["dp-fraction", "dp-minute"],
    ),
    ("2013-04-05T23:30:00.0-04:00", &[]),
    ("2013-04-05T23:30:00.13-04:00", &[]),
    ("2013-04-05T23:30:00.123-04:00", &["dp-fraction"]),
    ("2013-04-05T23:30:00.124-04:00", &[]),
    // The same instant named in another zone, `+` encoded properly …
    ("2013-04-06T09:00:00+05:30", &["dp-fraction"]),
    ("2013-04-05T18:50:00+05:30", &["dp-minute"]),
    // … and as it arrives when the client sent a literal `+` (#1296).
    ("2013-04-06T09:00:00 05:30", &["dp-fraction"]),
    ("2013-04-05T18:50:00 05:30", &["dp-minute"]),
    ("ge2013-04-06T09:00:00 05:30", &["dp-fraction"]),
    // A day is a UTC day: the fraction Procedure is already April 6th.
    ("2013-04-05", &["dp-minute"]),
    ("2013-04-06", &["dp-fraction"]),
];

/// Values no backend may interpret. Every one used to be a 200 somewhere.
const INVALID: &[&str] = &[
    "gtnot-a-date",
    "not-a-date",
    "gtabcd",
    "lt2024-1x",
    "2024-13-45",
    "ne2024-13-45",
    "lt2024-02-30",
    "ltT25:00:00Z",
    // An hour needs its minutes.
    "2013-04-05T10",
    "2013-04-05T24:00:00Z",
    "2013-04-05T09:20:00+15:00",
];

fn date_query(param: &str, value: &str) -> SearchQuery {
    SearchQuery::new("Procedure").with_parameter(SearchParameter {
        name: param.to_string(),
        param_type: SearchParamType::Date,
        values: vec![SearchValue::parse(value)],
        ..Default::default()
    })
}

async fn matched<S>(
    backend: &S,
    tenant: &TenantContext,
    param: &str,
    value: &str,
) -> BTreeSet<String>
where
    S: ResourceStorage + SearchProvider,
{
    backend
        .search(tenant, &date_query(param, value))
        .await
        .unwrap_or_else(|e| panic!("search {param}={value} failed: {e}"))
        .resources
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect()
}

/// Seeds two Procedures under a caller-unique tenant and asserts the tables.
pub async fn sub_day_precision_and_validation<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());

    for (id, performed) in [FRACTION, MINUTE] {
        backend
            .create(
                &tenant,
                "Procedure",
                json!({
                    "id": id,
                    "status": "completed",
                    "subject": {"reference": "Patient/dp-subject"},
                    "performedDateTime": performed,
                }),
                FhirVersion::default(),
            )
            .await
            .expect("seed procedure");
    }

    // Positive control, and the wait for eventually-consistent indexes: both
    // Procedures must be findable *by date* before any "matches nothing"
    // below means anything.
    for attempt in 0..60 {
        let visible = matched(backend, &tenant, "date", "ge1900-01-01")
            .await
            .len();
        if visible == 2 {
            break;
        }
        assert!(
            attempt < 59,
            "the seeded Procedures never became searchable by date ({visible}/2): \
             is the backend built with the spec search parameters?"
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    for (value, expected) in CASES {
        let expected: BTreeSet<String> = expected.iter().map(|id| id.to_string()).collect();
        assert_eq!(
            matched(backend, &tenant, "date", value).await,
            expected,
            "date={value}"
        );
    }

    // `_lastUpdated` takes the same values through each backend's dedicated
    // builder. A resource's own `meta.lastUpdated`, cut to the second or a
    // fraction of it, must find it. The stored value is finer than those cuts.
    let stored = backend
        .read(&tenant, "Procedure", FRACTION.0)
        .await
        .expect("read back")
        .expect("seeded procedure exists");
    let last_updated = stored.last_modified();
    let second = last_updated.format("%Y-%m-%dT%H:%M:%S");
    let nanos = last_updated.timestamp_subsec_nanos();
    for value in [
        last_updated.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        format!("{second}.{}Z", nanos / 100_000_000),
        format!("{second}.{:02}Z", nanos / 10_000_000),
        last_updated.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        last_updated.format("%Y-%m-%dT%H:%MZ").to_string(),
    ] {
        for prefix in ["", "eq", "ge", "le"] {
            let hits = matched(
                backend,
                &tenant,
                "_lastUpdated",
                &format!("{prefix}{value}"),
            )
            .await;
            assert!(
                hits.contains(FRACTION.0),
                "_lastUpdated={prefix}{value} should find a resource last updated at {last_updated}"
            );
        }
        for prefix in ["ne", "gt", "lt"] {
            let hits = matched(
                backend,
                &tenant,
                "_lastUpdated",
                &format!("{prefix}{value}"),
            )
            .await;
            assert!(
                !hits.contains(FRACTION.0),
                "_lastUpdated={prefix}{value} should not find a resource last updated at {last_updated}"
            );
        }
    }

    // Not a date: an error on every backend, never a query.
    for param in ["date", "_lastUpdated"] {
        for value in INVALID {
            match backend.search(&tenant, &date_query(param, value)).await {
                Err(StorageError::Search(SearchError::InvalidDateValue {
                    param: named, ..
                })) => {
                    assert_eq!(named, param, "{param}={value}");
                }
                Err(other) => panic!("{param}={value}: expected InvalidDateValue, got {other}"),
                Ok(result) => panic!(
                    "{param}={value}: expected InvalidDateValue, got {} result(s)",
                    result.resources.items.len()
                ),
            }
        }
    }
    // `search_count` is an entry point of its own.
    assert!(matches!(
        backend
            .search_count(&tenant, &date_query("date", "lt2024-02-30"))
            .await,
        Err(StorageError::Search(SearchError::InvalidDateValue { .. }))
    ));
}
