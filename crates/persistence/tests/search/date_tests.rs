//! Tests for date search parameters.
//!
//! This module tests date-type search parameters including
//! eq, ne, lt, le, gt, ge prefixes and date precision handling.

use serde_json::json;

use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
};

use helios_fhir::FhirVersion;

#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::SqliteBackend;

// ============================================================================
// Helper Functions
// ============================================================================

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
async fn seed_test_patients(backend: &SqliteBackend, tenant: &TenantContext) {
    let patients = vec![
        json!({"resourceType": "Patient", "birthDate": "1980-01-15"}),
        json!({"resourceType": "Patient", "birthDate": "1990-06-30"}),
        json!({"resourceType": "Patient", "birthDate": "2000-12-25"}),
        json!({"resourceType": "Patient", "birthDate": "1975-03-10"}),
        json!({"resourceType": "Patient", "birthDate": "2010-08-05"}),
    ];

    for patient in patients {
        backend
            .create(tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }
}

// ============================================================================
// Date Search Tests - Equality
// ============================================================================

/// Test date search with exact date.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_date_search_eq() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "birthdate".to_string(),
        param_type: SearchParamType::Date,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Eq, "1980-01-15")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    assert!(!result.resources.is_empty());
    for resource in &result.resources.items {
        assert_eq!(resource.content()["birthDate"], "1980-01-15");
    }
}

// ============================================================================
// Date Search Tests - Comparison Operators
// ============================================================================

/// Test date search with less than.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_date_search_lt() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "birthdate".to_string(),
        param_type: SearchParamType::Date,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Lt, "1985-01-01")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find patients born before 1985
    for resource in &result.resources.items {
        let birth_date = resource.content()["birthDate"].as_str().unwrap();
        assert!(
            birth_date < "1985-01-01",
            "Birth date {} should be < 1985-01-01",
            birth_date
        );
    }
}

/// Test date search with greater than.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_date_search_gt() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "birthdate".to_string(),
        param_type: SearchParamType::Date,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Gt, "2000-01-01")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    for resource in &result.resources.items {
        let birth_date = resource.content()["birthDate"].as_str().unwrap();
        assert!(
            birth_date > "2000-01-01",
            "Birth date {} should be > 2000-01-01",
            birth_date
        );
    }
}

/// Test date search with less than or equal.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_date_search_le() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "birthdate".to_string(),
        param_type: SearchParamType::Date,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Le, "1990-06-30")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    for resource in &result.resources.items {
        let birth_date = resource.content()["birthDate"].as_str().unwrap();
        assert!(birth_date <= "1990-06-30");
    }
}

/// Test date search with greater than or equal.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_date_search_ge() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "birthdate".to_string(),
        param_type: SearchParamType::Date,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Ge, "2000-01-01")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    for resource in &result.resources.items {
        let birth_date = resource.content()["birthDate"].as_str().unwrap();
        assert!(birth_date >= "2000-01-01");
    }
}

// ============================================================================
// Date Search Tests - Range Queries
// ============================================================================

/// Test date search with range (between two dates).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_date_search_range() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Find patients born between 1985 and 2005
    let query = SearchQuery::new("Patient")
        .with_parameter(SearchParameter {
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Ge, "1985-01-01")],
            chain: vec![],
            components: vec![],
        })
        .with_parameter(SearchParameter {
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Le, "2005-12-31")],
            chain: vec![],
            components: vec![],
        });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    for resource in &result.resources.items {
        let birth_date = resource.content()["birthDate"].as_str().unwrap();
        assert!(("1985-01-01"..="2005-12-31").contains(&birth_date));
    }
}

// ============================================================================
// Date Search Tests - Year Precision
// ============================================================================

/// Test date search with year precision.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_date_search_year_precision() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search for anyone born in 1990
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "birthdate".to_string(),
        param_type: SearchParamType::Date,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Eq, "1990")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find patient with birthDate 1990-06-30
    for resource in &result.resources.items {
        let birth_date = resource.content()["birthDate"].as_str().unwrap();
        assert!(birth_date.starts_with("1990"));
    }
}
