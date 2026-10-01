//! Backend-agnostic suite for where `_sort` puts a resource that has no value
//! for a sort key (#1606).
//!
//! FHIR leaves null placement unspecified, and the backends used to disagree:
//! SQLite sorted a missing value first ascending, PostgreSQL and Elasticsearch
//! last ascending and first descending, MongoDB last both ways. The rule now
//! is MongoDB's everywhere: **a resource without a value for a key sorts after
//! every resource that has one, ascending or descending.** For a multi-key
//! sort that holds per key, within the ties of the keys before it. Ties are
//! broken by id, ascending.
//!
//! Every order is checked on one full page, then walked a page at a time
//! (by cursor where the backend hands one out, by `_offset` otherwise) and,
//! where a `Previous` cursor is offered, walked back again: paging across the
//! boundary between present and missing values must neither skip nor repeat
//! a resource.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `date_period_suite.rs`. The backend must be built with the spec search
//! parameters loaded; the positive control fails loudly if it was not.

#![allow(dead_code)]

use serde_json::{Value, json};

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchParamType, SearchParameter, SearchQuery, SearchValue, SortDirection, SortDirective,
};

/// (id, family, gender, birthDate).
type Seed = (
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
);

const PATIENTS: &[Seed] = &[
    ("s-a", Some("Adams"), Some("female"), Some("1970-01-01")),
    ("s-b", Some("Baker"), Some("female"), None),
    ("s-c", None, Some("male"), Some("2000-01-01")),
    ("s-d", Some("Clark"), None, Some("1980-01-01")),
    ("s-e", None, None, None),
    ("s-f", None, Some("female"), Some("1990-01-01")),
];

/// A sort key: (parameter, type, direction).
type Key = (&'static str, SearchParamType, SortDirection);

const ASC: SortDirection = SortDirection::Ascending;
const DESC: SortDirection = SortDirection::Descending;

/// (sort keys, the order they produce).
const CASES: &[(&[Key], &[&str])] = &[
    // Single key, date.
    (
        &[("birthdate", SearchParamType::Date, ASC)],
        &["s-a", "s-d", "s-f", "s-c", "s-b", "s-e"],
    ),
    (
        &[("birthdate", SearchParamType::Date, DESC)],
        &["s-c", "s-f", "s-d", "s-a", "s-b", "s-e"],
    ),
    // Single key, string.
    (
        &[("family", SearchParamType::String, ASC)],
        &["s-a", "s-b", "s-d", "s-c", "s-e", "s-f"],
    ),
    (
        &[("family", SearchParamType::String, DESC)],
        &["s-d", "s-b", "s-a", "s-c", "s-e", "s-f"],
    ),
    // Two keys: missing last for each key, within the ties of the first.
    (
        &[
            ("gender", SearchParamType::Token, ASC),
            ("birthdate", SearchParamType::Date, ASC),
        ],
        &["s-a", "s-f", "s-b", "s-c", "s-d", "s-e"],
    ),
    (
        &[
            ("gender", SearchParamType::Token, DESC),
            ("birthdate", SearchParamType::Date, DESC),
        ],
        &["s-c", "s-f", "s-a", "s-b", "s-d", "s-e"],
    ),
    (
        &[
            ("gender", SearchParamType::Token, ASC),
            ("birthdate", SearchParamType::Date, DESC),
        ],
        &["s-f", "s-a", "s-b", "s-c", "s-d", "s-e"],
    ),
];

fn sorted(keys: &[Key], count: u32) -> SearchQuery {
    let mut query = SearchQuery::new("Patient").with_count(count);
    for (parameter, param_type, direction) in keys {
        query = query.with_sort(SortDirective {
            parameter: parameter.to_string(),
            direction: *direction,
            param_type: Some(*param_type),
        });
    }
    query
}

fn describe(keys: &[Key]) -> String {
    keys.iter()
        .map(|(p, _, d)| match d {
            SortDirection::Ascending => p.to_string(),
            SortDirection::Descending => format!("-{p}"),
        })
        .collect::<Vec<_>>()
        .join(",")
}

async fn page<S>(
    backend: &S,
    tenant: &TenantContext,
    query: &SearchQuery,
) -> (Vec<String>, helios_persistence::types::PageInfo)
where
    S: ResourceStorage + SearchProvider,
{
    let result = backend
        .search(tenant, query)
        .await
        .unwrap_or_else(|e| panic!("search {query:?} failed: {e}"));
    let ids = result
        .resources
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect();
    (ids, result.resources.page_info)
}

/// Walks every page forward, following the next cursor where there is one and
/// `_offset` where there is not, then — when the last page offers a
/// `Previous` cursor — walks back to the first page. Returns the forward and
/// (if walked) backward sequences.
async fn walk<S>(
    backend: &S,
    tenant: &TenantContext,
    keys: &[Key],
    count: u32,
) -> (Vec<String>, Option<Vec<String>>)
where
    S: ResourceStorage + SearchProvider,
{
    let label = describe(keys);
    let mut forward = Vec::new();
    let mut query = sorted(keys, count);
    let mut offset = 0;
    let mut last_info;
    let mut last_page_len;
    let mut pages = 0;
    loop {
        let (ids, info) = page(backend, tenant, &query).await;
        offset += ids.len() as u32;
        last_page_len = ids.len();
        forward.extend(ids);
        pages += 1;
        assert!(
            pages <= PATIENTS.len() + 1,
            "_sort={label} _count={count}: paging never ended, got {forward:?}"
        );
        let has_next = info.has_next;
        let next_cursor = info.next_cursor.clone();
        last_info = info;
        if !has_next {
            break;
        }
        query = sorted(keys, count);
        match next_cursor {
            Some(cursor) => query = query.with_cursor(cursor),
            None => query.offset = Some(offset),
        }
    }

    let Some(mut previous) = last_info.previous_cursor.clone() else {
        return (forward, None);
    };
    // Start from the last page, then prepend each earlier one.
    let mut backward: Vec<String> = forward[forward.len() - last_page_len..].to_vec();
    let mut pages = 0;
    loop {
        let (ids, info) = page(
            backend,
            tenant,
            &sorted(keys, count).with_cursor(previous.clone()),
        )
        .await;
        pages += 1;
        assert!(
            pages <= PATIENTS.len() + 1,
            "_sort={label} _count={count}: paging back never ended, got {backward:?}"
        );
        let mut ids = ids;
        ids.extend(backward);
        backward = ids;
        match (info.has_previous, info.previous_cursor) {
            (true, Some(cursor)) => previous = cursor,
            _ => break,
        }
    }
    (forward, Some(backward))
}

async fn seed<S>(backend: &S, tenant: &TenantContext)
where
    S: ResourceStorage + SearchProvider,
{
    for (id, family, gender, birth_date) in PATIENTS {
        let mut patient = json!({"resourceType": "Patient", "id": id, "active": true});
        if let Some(family) = family {
            patient["name"] = json!([{ "family": family }]);
        }
        if let Some(gender) = gender {
            patient["gender"] = Value::from(*gender);
        }
        if let Some(birth_date) = birth_date {
            patient["birthDate"] = Value::from(*birth_date);
        }
        backend
            .create(tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap_or_else(|e| panic!("create Patient/{id} failed: {e}"));
    }
}

/// Waits until every seeded Patient is searchable by `active=true`: the
/// positive control that they are stored and indexed, and the wait for an
/// eventually-consistent index.
async fn wait_for_seed<S>(backend: &S, tenant: &TenantContext)
where
    S: ResourceStorage + SearchProvider,
{
    let control = SearchQuery::new("Patient")
        .with_parameter(SearchParameter {
            name: "active".to_string(),
            param_type: SearchParamType::Token,
            values: vec![SearchValue::eq("true")],
            ..Default::default()
        })
        .with_count(100);
    for attempt in 0..60 {
        let (visible, _) = page(backend, tenant, &control).await;
        if visible.len() == PATIENTS.len() {
            return;
        }
        assert!(
            attempt < 59,
            "the seeded Patients never became searchable by active ({}/{}): \
             is the backend built with the spec search parameters?",
            visible.len(),
            PATIENTS.len(),
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

/// Resources with no value for a sort key sort after those with one, in
/// either direction, on one page and across every page boundary (#1606).
///
/// `multi_key` runs the multi-key cases too; a backend that refuses a sort on
/// more than one search parameter (MongoDB, until #1564) passes `false`.
pub async fn missing_sort_values_sort_last<S>(backend: &S, tenant_base: &str, multi_key: bool)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    seed(backend, &tenant).await;
    wait_for_seed(backend, &tenant).await;

    for (keys, expected) in CASES {
        if keys.len() > 1 && !multi_key {
            continue;
        }
        let label = describe(keys);
        let expected: Vec<String> = expected.iter().map(|id| id.to_string()).collect();

        let (one_page, _) = page(backend, &tenant, &sorted(keys, 100)).await;
        assert_eq!(one_page, expected, "_sort={label}");

        for count in 1..=4 {
            let (forward, backward) = walk(backend, &tenant, keys, count).await;
            assert_eq!(
                forward, expected,
                "_sort={label} _count={count}, paging forward"
            );
            if let Some(backward) = backward {
                assert_eq!(
                    backward, expected,
                    "_sort={label} _count={count}, paging back with Previous"
                );
            }
        }
    }
}
