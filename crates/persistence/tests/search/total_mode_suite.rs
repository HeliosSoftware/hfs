//! Backend-agnostic `_total` suite (issue #2283): `SearchResult::total` is
//! filled only when `_total` asks for one (`accurate` or `estimate`). Without
//! `_total`, and with `_total=none`, no backend reports a total, and paging
//! must not depend on it.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `empty_value_suite.rs`.

#![allow(dead_code)]

use std::time::Duration;

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{SearchQuery, TotalMode};

const SEEDED: u64 = 3;

fn query(total: Option<TotalMode>) -> SearchQuery {
    let mut query = SearchQuery::new("Patient");
    query.total = total;
    query
}

/// Seeds three Patients, then checks the total for each `_total` value and
/// that a one-resource page still reports a next page without a total.
pub async fn total_is_reported_only_when_requested<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    for i in 0..SEEDED {
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": format!("tm-{i}")}),
                FhirVersion::default(),
            )
            .await
            .expect("seed patient");
    }

    // Wait for eventually-consistent indexes: the accurate total is the control.
    let mut accurate = None;
    for _ in 0..50 {
        accurate = backend
            .search(&tenant, &query(Some(TotalMode::Accurate)))
            .await
            .expect("accurate search")
            .total;
        if accurate == Some(SEEDED) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(accurate, Some(SEEDED), "_total=accurate reports the count");

    let estimate = backend
        .search(&tenant, &query(Some(TotalMode::Estimate)))
        .await
        .expect("estimate search");
    assert_eq!(
        estimate.total,
        Some(SEEDED),
        "_total=estimate reports a total (exact below the estimate bound)"
    );

    for (label, total) in [("absent", None), ("none", Some(TotalMode::None))] {
        let result = backend
            .search(&tenant, &query(total))
            .await
            .expect("search without a total");
        assert_eq!(result.total, None, "_total {label}: no total");
        assert_eq!(
            result.resources.page_info.total, None,
            "_total {label}: no page total"
        );
        assert_eq!(result.resources.items.len() as u64, SEEDED);
    }

    // Paging must not depend on the total.
    let paged = backend
        .search(&tenant, &query(None).with_count(1))
        .await
        .expect("paged search");
    assert_eq!(paged.total, None, "paged search without _total: no total");
    assert_eq!(paged.resources.items.len(), 1);
    assert!(paged.has_next(), "a next page exists without a total");
    assert!(paged.next_cursor().is_some(), "next cursor without a total");

    // A type that was never stored (ES has no index for it) follows the same
    // rule: a total only when asked for, and then it is 0.
    let never_stored = |total| {
        let mut q = SearchQuery::new("Device");
        q.total = total;
        q
    };
    let missing = backend
        .search(&tenant, &never_stored(None))
        .await
        .expect("search of a type that was never stored");
    assert_eq!(missing.total, None, "empty type without _total: no total");
    assert!(missing.resources.items.is_empty());
    let missing = backend
        .search(&tenant, &never_stored(Some(TotalMode::Accurate)))
        .await
        .expect("accurate search of a type that was never stored");
    assert_eq!(missing.total, Some(0), "empty type with accurate: total 0");
}
