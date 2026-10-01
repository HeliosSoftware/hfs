//! Backend-agnostic suite for *stored* dateTimes that have minutes but no
//! seconds (issue #1315).
//!
//! `2013-04-05T09:20` is not a valid FHIR `dateTime` — the datatype regex
//! requires seconds — but nothing on the default write path rejects it: the
//! REST layer stores the JSON it was given and validation is off unless
//! `HFS_VALIDATION_MODE` says otherwise. The FHIR *search* grammar does allow
//! minute precision, so after #1313 a client can search `date=2013-04-05T09:20`
//! on every backend. The PostgreSQL and MongoDB index writers, though, read
//! stored values as RFC 3339, which also requires seconds: the index row was
//! skipped and the resource could not be found by that parameter at all.
//!
//! The writers now read a stored value with the same
//! `helios_persistence::search::FhirDateValue` the search side uses, so the
//! instant indexed is the first instant of the range searched for the same
//! text. SQLite never had the gap (it indexes the text as stored, and
//! `strftime` reads `hh:mm`); it runs the same table so the three stay aligned.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `date_precision_suite.rs`. The backend must be built with the spec search
//! parameters loaded; the suite's positive controls fail loudly if it was not.

#![allow(dead_code)]

use std::collections::BTreeSet;

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

/// (id, `performedDateTime`). The first three have no seconds; `mi-control`
/// is a conformant value, there to show the parameter is indexed at all.
const SEEDED: &[(&str, &str)] = &[
    // 2013-04-05T09:20:00Z
    ("mi-local", "2013-04-05T09:20"),
    // 2013-04-05T13:20:00Z
    ("mi-offset", "2013-04-05T09:20-04:00"),
    // 2013-04-05T17:20:00Z
    ("mi-utc", "2013-04-05T17:20Z"),
    // 2013-04-05T21:20:00Z
    ("mi-control", "2013-04-05T21:20:00Z"),
    // A space where the zone sign belongs. On the *search* side that is a
    // form-decoded `+` and is repaired (#1296); in a stored resource it is
    // just an invalid value, and must not be indexed as if it were `+05:30`.
    ("mi-space", "2013-04-05T09:20:00 05:30"),
];

/// Every seeded Procedure that carries a date the index can hold.
const INDEXED: &[&str] = &["mi-control", "mi-local", "mi-offset", "mi-utc"];

/// `date=` value → the seeded Procedures it must match.
const CASES: &[(&str, &[&str])] = &[
    // The stored text, searched verbatim.
    ("2013-04-05T09:20", &["mi-local"]),
    ("eq2013-04-05T09:20", &["mi-local"]),
    ("2013-04-05T09:20-04:00", &["mi-offset"]),
    ("2013-04-05T17:20Z", &["mi-utc"]),
    // The same instant in another zone.
    ("2013-04-05T18:50+05:30", &["mi-offset"]),
    // At second precision the search range is one second, which does not
    // contain a stored minute: `eq` needs the whole stored range inside the
    // searched one (#1391).
    ("2013-04-05T09:20:00Z", &[]),
    ("2013-04-05T13:20:00Z", &[]),
    ("ap2013-04-05T09:20:00Z", &["mi-local"]),
    // The neighbouring minutes do not contain them.
    ("2013-04-05T09:19", &[]),
    ("2013-04-05T09:21", &[]),
    // Comparators.
    (
        "ge2013-04-05T09:20",
        &["mi-control", "mi-local", "mi-offset", "mi-utc"],
    ),
    ("gt2013-04-05T09:20", &["mi-control", "mi-offset", "mi-utc"]),
    ("lt2013-04-05T09:20", &[]),
    ("lt2013-04-05T09:21", &["mi-local"]),
    ("le2013-04-05T09:20", &["mi-local"]),
    ("lt2013-04-05T13:20Z", &["mi-local"]),
    ("le2013-04-05T09:20-04:00", &["mi-local", "mi-offset"]),
    ("ne2013-04-05T09:20", &["mi-control", "mi-offset", "mi-utc"]),
    // The day they all fall on.
    (
        "2013-04-05",
        &["mi-control", "mi-local", "mi-offset", "mi-utc"],
    ),
    // What `mi-space` would be if its space were read as `+`: nothing there.
    ("2013-04-05T03:50:00Z", &[]),
    ("2013-04-05T09:20:00+05:30", &[]),
];

fn query(param: &str, param_type: SearchParamType, value: &str) -> SearchQuery {
    SearchQuery::new("Procedure").with_parameter(SearchParameter {
        name: param.to_string(),
        param_type,
        values: vec![SearchValue::parse(value)],
        ..Default::default()
    })
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

/// Seeds the Procedures under a caller-unique tenant and asserts the table.
pub async fn minute_precision_stored_values_are_indexed<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());

    for (id, performed) in SEEDED {
        // The write itself must succeed whatever the date looks like.
        backend
            .create(
                &tenant,
                "Procedure",
                json!({
                    "id": id,
                    "status": "completed",
                    "subject": {"reference": "Patient/mi-subject"},
                    "performedDateTime": performed,
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap_or_else(|e| panic!("create {id} ({performed}) failed: {e}"));
    }

    // Positive control 1, and the wait for eventually-consistent indexes: an
    // unrelated parameter finds every Procedure, so the resources are stored
    // and the search index is being written.
    let by_status = query("status", SearchParamType::Token, "completed");
    for attempt in 0..60 {
        let visible = matched(backend, &tenant, &by_status).await;
        if visible.len() == SEEDED.len() {
            break;
        }
        assert!(
            attempt < 59,
            "the seeded Procedures never became searchable by status ({}/{}): \
             is the backend built with the spec search parameters?",
            visible.len(),
            SEEDED.len()
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // Positive control 2: `date` is indexed for a conformant value.
    assert_eq!(
        matched(
            backend,
            &tenant,
            &query("date", SearchParamType::Date, "2013-04-05T21:20:00Z")
        )
        .await,
        ids(&["mi-control"]),
        "the conformant control must be findable by date"
    );

    // Every Procedure with a usable date has a date index entry — and the one
    // with a space in it does not.
    assert_eq!(
        matched(
            backend,
            &tenant,
            &query("date", SearchParamType::Date, "ge1900-01-01")
        )
        .await,
        ids(INDEXED),
        "date=ge1900-01-01"
    );

    for (value, expected) in CASES {
        assert_eq!(
            matched(
                backend,
                &tenant,
                &query("date", SearchParamType::Date, value)
            )
            .await,
            ids(expected),
            "date={value}"
        );
    }
}
