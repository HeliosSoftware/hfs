//! Tests for search pagination.
//!
//! This module tests cursor-based and offset-based pagination
//! including _count, page navigation, and result ordering.

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchParamType, SearchParameter, SearchQuery, SearchValue, SortDirective,
};

#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::SqliteBackend;

#[cfg(feature = "sqlite")]
fn create_sqlite_backend() -> SqliteBackend {
    super::make_sqlite_backend()
}

fn create_tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    )
}

#[cfg(feature = "sqlite")]
async fn seed_many_patients(backend: &SqliteBackend, tenant: &TenantContext, count: usize) {
    for i in 0..count {
        let patient = json!({
            "resourceType": "Patient",
            "name": [{"family": format!("Patient{:03}", i)}],
            "birthDate": format!("19{:02}-01-01", 50 + (i % 50))
        });
        backend
            .create(tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }
}

// ============================================================================
// Basic Pagination Tests
// ============================================================================

/// Test _count limits results.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_pagination_count() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_many_patients(&backend, &tenant, 20).await;

    let query = SearchQuery::new("Patient").with_count(5);
    let result = backend.search(&tenant, &query).await.unwrap();

    assert_eq!(result.resources.len(), 5);
}

/// Test total count is returned.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_pagination_total() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_many_patients(&backend, &tenant, 20).await;

    let query = SearchQuery::new("Patient").with_count(5);
    let result = backend.search(&tenant, &query).await.unwrap();

    // If total is supported, it should be 20
    if let Some(total) = result.total {
        assert_eq!(total, 20);
    }
}

// ============================================================================
// Cursor Pagination Tests
// ============================================================================

/// Test cursor-based pagination forward.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_cursor_pagination_forward() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_many_patients(&backend, &tenant, 20).await;

    // Get first page
    let query = SearchQuery::new("Patient").with_count(5);
    let page1 = backend.search(&tenant, &query).await.unwrap();

    assert_eq!(page1.resources.len(), 5);
    assert!(
        page1.resources.page_info.next_cursor.is_some(),
        "Should have next cursor"
    );

    // Get second page
    if let Some(cursor) = page1.resources.page_info.next_cursor.clone() {
        let mut query2 = SearchQuery::new("Patient").with_count(5);
        query2.cursor = Some(cursor);
        let page2 = backend.search(&tenant, &query2).await.unwrap();

        assert_eq!(page2.resources.len(), 5);

        // Pages should not overlap
        let page1_ids: std::collections::HashSet<_> =
            page1.resources.items.iter().map(|r| r.id()).collect();
        for resource in &page2.resources.items {
            assert!(
                !page1_ids.contains(resource.id()),
                "Pages should not overlap"
            );
        }
    }
}

/// Test paginating through all results.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_paginate_all_results() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_many_patients(&backend, &tenant, 25).await;

    let mut all_ids = std::collections::HashSet::new();
    let mut cursor: Option<String> = None;

    loop {
        let mut query = SearchQuery::new("Patient").with_count(10);
        query.cursor = cursor.clone();
        let page = backend.search(&tenant, &query).await.unwrap();

        for resource in &page.resources.items {
            let inserted = all_ids.insert(resource.id().to_string());
            assert!(inserted, "Should not see duplicate resources");
        }

        match page.resources.page_info.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }

    // Should have seen all 25 patients
    assert_eq!(all_ids.len(), 25);
}

/// Test last page has no next cursor.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_last_page_no_cursor() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_many_patients(&backend, &tenant, 8).await;

    // First page of 5
    let query = SearchQuery::new("Patient").with_count(5);
    let page1 = backend.search(&tenant, &query).await.unwrap();
    assert!(page1.resources.page_info.next_cursor.is_some());

    // Second page of remaining 3
    if let Some(cursor) = page1.resources.page_info.next_cursor.clone() {
        let mut query2 = SearchQuery::new("Patient").with_count(5);
        query2.cursor = Some(cursor);
        let page2 = backend.search(&tenant, &query2).await.unwrap();

        assert_eq!(page2.resources.len(), 3);
        assert!(
            page2.resources.page_info.next_cursor.is_none(),
            "Last page should have no cursor"
        );
    }
}

// ============================================================================
// Sorting with Pagination Tests
// ============================================================================

/// Test pagination maintains sort order.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_pagination_with_sort() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_many_patients(&backend, &tenant, 20).await;

    // Sort by family name descending
    let query = SearchQuery::new("Patient")
        .with_sort(SortDirective::parse("-name").with_param_type(Some(SearchParamType::String)))
        .with_count(5);

    let page1 = backend.search(&tenant, &query).await.unwrap();
    assert!(
        page1.resources.page_info.next_cursor.is_some(),
        "20 patients at _count=5 must yield a next-page cursor"
    );

    if let Some(cursor) = page1.resources.page_info.next_cursor.clone() {
        let mut query2 = SearchQuery::new("Patient")
            .with_sort(SortDirective::parse("-name").with_param_type(Some(SearchParamType::String)))
            .with_count(5);
        query2.cursor = Some(cursor);
        let page2 = backend.search(&tenant, &query2).await.unwrap();

        // Last item of page1 should be >= first item of page2 (descending)
        let last_p1 = page1.resources.items.last().unwrap().content()["name"][0]["family"]
            .as_str()
            .unwrap();
        let first_p2 = page2.resources.items[0].content()["name"][0]["family"]
            .as_str()
            .unwrap();

        assert!(
            last_p1 >= first_p2,
            "Sort order should be maintained across pages"
        );
    }
}

/// Test pagination with ascending sort.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_pagination_with_asc_sort() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_many_patients(&backend, &tenant, 20).await;

    let query = SearchQuery::new("Patient")
        .with_sort(SortDirective::parse("name").with_param_type(Some(SearchParamType::String)))
        .with_count(5);

    let page1 = backend.search(&tenant, &query).await.unwrap();
    assert!(
        page1.resources.page_info.next_cursor.is_some(),
        "20 patients at _count=5 must yield a next-page cursor"
    );

    if let Some(cursor) = page1.resources.page_info.next_cursor.clone() {
        let mut query2 = SearchQuery::new("Patient")
            .with_sort(SortDirective::parse("name").with_param_type(Some(SearchParamType::String)))
            .with_count(5);
        query2.cursor = Some(cursor);
        let page2 = backend.search(&tenant, &query2).await.unwrap();

        let last_p1 = page1.resources.items.last().unwrap().content()["name"][0]["family"]
            .as_str()
            .unwrap();
        let first_p2 = page2.resources.items[0].content()["name"][0]["family"]
            .as_str()
            .unwrap();

        assert!(
            last_p1 <= first_p2,
            "Ascending sort order should be maintained"
        );
    }
}

// ============================================================================
// Edge Cases
// ============================================================================

/// Test pagination with no results.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_pagination_empty_results() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let query = SearchQuery::new("Patient").with_count(10);
    let result = backend.search(&tenant, &query).await.unwrap();

    assert!(result.resources.is_empty());
    assert!(result.resources.page_info.next_cursor.is_none());
}

/// Test pagination with count larger than results.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_pagination_count_larger_than_results() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_many_patients(&backend, &tenant, 5).await;

    let query = SearchQuery::new("Patient").with_count(100);
    let result = backend.search(&tenant, &query).await.unwrap();

    assert_eq!(result.resources.len(), 5);
    assert!(result.resources.page_info.next_cursor.is_none());
}

/// Test pagination count of 1.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_pagination_count_one() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_many_patients(&backend, &tenant, 5).await;

    let mut seen = 0;
    let mut cursor: Option<String> = None;

    loop {
        let mut query = SearchQuery::new("Patient").with_count(1);
        query.cursor = cursor.clone();
        let page = backend.search(&tenant, &query).await.unwrap();

        assert!(page.resources.len() <= 1);
        seen += page.resources.len();

        match page.resources.page_info.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }

    assert_eq!(seen, 5);
}

// ============================================================================
// Filtered Pagination Tests
// ============================================================================

/// Test pagination with search filter.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_pagination_with_filter() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_many_patients(&backend, &tenant, 50).await;

    let mut total_found = 0;
    let mut cursor: Option<String> = None;

    loop {
        // Search for patients with birth year starting with "19"
        let mut query = SearchQuery::new("Patient")
            .with_parameter(SearchParameter {
                name: "birthdate".to_string(),
                param_type: SearchParamType::Date,
                modifier: None,
                values: vec![SearchValue::new(
                    helios_persistence::types::SearchPrefix::Sa,
                    "1970-01-01",
                )],
                chain: vec![],
                components: vec![],
            })
            .with_count(10);
        query.cursor = cursor.clone();
        let page = backend.search(&tenant, &query).await.unwrap();
        total_found += page.resources.len();

        match page.resources.page_info.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }

    // Should have found some subset of patients
    assert!(total_found > 0);
}
