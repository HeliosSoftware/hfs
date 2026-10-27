//! Backend-agnostic day-precision date-boundary suite (issue #519).
//!
//! PR #463 fixed day-precision date comparison **for SQLite only** and said so:
//! Postgres was believed correct via `parse_date_value`, Elasticsearch and
//! MongoDB were unverified. Belief is not verification — this suite drives the
//! exact boundary table from issue #456 through each backend's *real*
//! [`SearchProvider::search`] path, so every backend owes the same answers.
//!
//! The subtle row is `birthdate=1995` = **1**: the old text-comparison path
//! wrongly included the 1996-01-01 birth (#463). Any backend answering 2 here
//! has the same class of bug SQLite had.
//!
//! Included by `#[path]` into each backend's test binary rather than living in
//! `tests/common/` — the same arrangement (and for the same cargo reason) as
//! `transactions/if_match_suite.rs` and
//! `multitenancy/tenant_id_fidelity_suite.rs`.
//!
//! Search-indexing backends may be eventually consistent, so the suite polls
//! until the seed becomes searchable before judging the table — the same
//! posture the UI e2e suite takes (`waitSearchable`).

#![allow(dead_code)]

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

/// The ten-patient cohort from issue #456. `1995-10-02` is the boundary birth;
/// `1996-01-01` is the one the old text comparison wrongly swept into `1995`.
const COHORT: [&str; 10] = [
    "1975-03-21",
    "1991-09-26",
    "1994-10-03",
    "1995-10-02",
    "1996-01-01",
    "1998-09-28",
    "2003-06-30",
    "2005-10-25",
    "2011-11-28",
    "2022-05-07",
];

/// The boundary table: each case is the repeated `birthdate=` values of one
/// query (multiple entries AND, as separate parameters) and the expected hit
/// count over the cohort.
const CASES: [(&[&str], usize); 8] = [
    (&["1995-10-02"], 1),
    (&["eq1995-10-02"], 1),
    (&["ge1995-10-02"], 7),
    (&["le1995-12-31"], 4),
    (&["lt1996-01-01"], 4),
    (&["1995-10"], 1),
    (&["1995"], 1),
    (&["ge1995-10-02", "le1995-10-02"], 1),
];

fn birthdate_query(values: &[&str]) -> SearchQuery {
    let mut query = SearchQuery::new("Patient");
    for v in values {
        query = query.with_parameter(SearchParameter {
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            values: vec![SearchValue::parse(v)],
            ..Default::default()
        });
    }
    query
}

/// Seeds the cohort under a caller-unique tenant and asserts the whole table.
pub async fn day_precision_boundaries<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());

    for (i, birth) in COHORT.iter().enumerate() {
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": format!("dp-{i}"), "birthDate": birth}),
                FhirVersion::default(),
            )
            .await
            .expect("seed patient");
    }

    // Eventually-consistent search backends need the seed to land in the
    // index first; poll on the broadest query until all ten are visible.
    let all = birthdate_query(&["ge1900-01-01"]);
    for attempt in 0..60 {
        let visible = backend
            .search(&tenant, &all)
            .await
            .expect("visibility probe")
            .resources
            .items
            .len();
        if visible == COHORT.len() {
            break;
        }
        assert!(
            attempt < 59,
            "cohort never became searchable: {visible}/{} visible",
            COHORT.len()
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    for (values, expected) in CASES {
        let result = backend
            .search(&tenant, &birthdate_query(values))
            .await
            .unwrap_or_else(|e| panic!("search birthdate={values:?} failed: {e}"));
        assert_eq!(
            result.resources.items.len(),
            expected,
            "birthdate={values:?} over the #456 cohort"
        );
    }
}
