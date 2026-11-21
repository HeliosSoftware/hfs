//! Backend-agnostic suite for paging through resources that share one
//! `last_updated` (#2603).
//!
//! The order of resources with identical timestamps is not defined by FHIR,
//! and the backends break those ties in the direction their index serves, so
//! they differ on purpose. What every backend must keep is what a client can
//! rely on: with every resource tied, walking `next` to the end and
//! `previous` back must neither skip nor repeat a resource, the walk back must
//! be the walk forward, the order must not depend on the page size, and the
//! first page (no cursor) must be the prefix of the cursor walk.
//!
//! Included by `#[path]` into each backend's test binary. Each backend pins
//! the timestamps its own way, through the `pin` closure.

#![allow(dead_code)]

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{SearchQuery, SortDirection, SortDirective};

use crate::seeding::SeedingStorage;

/// Created out of id order, so neither creation order nor id order is mistaken
/// for the tie order.
const IDS: &[&str] = &[
    "t-5", "t-2", "t-9", "t-1", "t-7", "t-3", "t-8", "t-4", "t-6", "t-0",
];

fn query(sort: Option<SortDirection>, count: u32) -> SearchQuery {
    let mut query = SearchQuery::new("Patient").with_count(count);
    if let Some(direction) = sort {
        query = query.with_sort(SortDirective {
            parameter: "_lastUpdated".to_string(),
            direction,
            param_type: None,
        });
    }
    query
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

/// Walks forward by `next` cursors to the end, then back by `previous`
/// cursors. Returns (forward order, backward order, first page).
async fn walk<S>(
    backend: &S,
    tenant: &TenantContext,
    sort: Option<SortDirection>,
    count: u32,
    label: &str,
) -> (Vec<String>, Vec<String>, Vec<String>)
where
    S: ResourceStorage + SearchProvider,
{
    let mut forward = Vec::new();
    let mut first = Vec::new();
    let mut q = query(sort, count);
    let mut pages = 0;
    let mut last_len;
    let last_info = loop {
        let (ids, info) = page(backend, tenant, &q).await;
        if pages == 0 {
            first = ids.clone();
        }
        last_len = ids.len();
        forward.extend(ids);
        pages += 1;
        assert!(
            pages <= IDS.len() + 1,
            "{label} _count={count}: paging never ended, got {forward:?}"
        );
        match (info.has_next, info.next_cursor.clone()) {
            (true, Some(cursor)) => q = query(sort, count).with_cursor(cursor),
            (true, None) => panic!("{label} _count={count}: has_next without a next cursor"),
            _ => break info,
        }
    };

    let mut backward: Vec<String> = forward[forward.len() - last_len..].to_vec();
    let mut previous = last_info.previous_cursor.clone();
    let mut back_pages = 0;
    while let Some(cursor) = previous {
        let (ids, info) = page(
            backend,
            tenant,
            &query(sort, count).with_cursor(cursor.clone()),
        )
        .await;
        back_pages += 1;
        assert!(
            back_pages <= IDS.len() + 1,
            "{label} _count={count}: paging back never ended, got {backward:?}"
        );
        let mut joined = ids;
        joined.extend(backward);
        backward = joined;
        previous = match (info.has_previous, info.previous_cursor) {
            (true, Some(cursor)) => Some(cursor),
            _ => None,
        };
    }
    (forward, backward, first)
}

/// With every resource on one `last_updated`, paging forward and back through
/// `""`, `_lastUpdated` and `-_lastUpdated` is complete, duplicate-free,
/// symmetric and independent of the page size (#2603).
/// `pin(tenant_id, resource_type)` sets `last_updated` of every such resource
/// (and any search-side copy) to one instant.
pub async fn tied_timestamps_page_without_gaps<S>(
    backend: &S,
    tenant_base: &str,
    pin: impl AsyncFn(&str, &str),
) where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    for id in IDS {
        backend
            .seed_create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": id, "active": true}),
                FhirVersion::default(),
            )
            .await
            .unwrap_or_else(|e| panic!("create Patient/{id} failed: {e}"));
    }
    pin(tenant_base, "Patient").await;

    let mut expected_set: Vec<String> = IDS.iter().map(|id| id.to_string()).collect();
    expected_set.sort();

    let sorts = [
        ("", None),
        ("_lastUpdated", Some(SortDirection::Ascending)),
        ("-_lastUpdated", Some(SortDirection::Descending)),
    ];
    for (name, sort) in sorts {
        let label = format!("_sort={name}");
        let (single, _) = page(backend, &tenant, &query(sort, 100)).await;
        let mut sorted_single = single.clone();
        sorted_single.sort();
        assert_eq!(sorted_single, expected_set, "{label}: one full page");

        for count in 1..=4 {
            let (forward, backward, first) = walk(backend, &tenant, sort, count, &label).await;
            let mut seen = forward.clone();
            seen.sort();
            assert_eq!(
                seen, expected_set,
                "{label} _count={count}: forward walk skipped or repeated a resource: {forward:?}"
            );
            assert_eq!(
                forward, single,
                "{label} _count={count}: the order depends on the page size"
            );
            assert_eq!(
                backward, forward,
                "{label} _count={count}: the walk back is not the walk forward"
            );
            assert_eq!(
                first[..],
                forward[..first.len()],
                "{label} _count={count}: the first page is not the prefix of the cursor walk"
            );
        }
    }
}
