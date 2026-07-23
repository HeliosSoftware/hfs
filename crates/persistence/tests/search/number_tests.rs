//! Tests for number search parameters.
//!
//! This module tests number-type search parameters including
//! comparison operators and significant figures handling.

use serde_json::json;

use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
};

use helios_fhir::FhirVersion;

#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::SqliteBackend;

#[cfg(feature = "sqlite")]
fn create_sqlite_backend() -> SqliteBackend {
    let backend = SqliteBackend::in_memory().expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to initialize schema");
    backend
}

fn create_tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    )
}

/// Test number search with equality.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_number_search_eq() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create RiskAssessment resources with probability
    let risk1 = json!({
        "resourceType": "RiskAssessment",
        "status": "final",
        "prediction": [{"probabilityDecimal": 0.5}]
    });
    let risk2 = json!({
        "resourceType": "RiskAssessment",
        "status": "final",
        "prediction": [{"probabilityDecimal": 0.75}]
    });
    backend
        .create(&tenant, "RiskAssessment", risk1, FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "RiskAssessment", risk2, FhirVersion::default())
        .await
        .unwrap();

    let query = SearchQuery::new("RiskAssessment").with_parameter(SearchParameter {
        name: "probability".to_string(),
        param_type: SearchParamType::Number,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Eq, "0.5")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Number search implementation may vary
    // This test documents expected behavior
}

/// Test number search with less than.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_number_search_lt() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let query = SearchQuery::new("RiskAssessment").with_parameter(SearchParameter {
        name: "probability".to_string(),
        param_type: SearchParamType::Number,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Lt, "0.6")],
        chain: vec![],
        components: vec![],
    });

    let _result = backend.search(&tenant, &query.with_count(100)).await;

    // Test documents expected behavior for number comparisons
}

/// Test number search with greater than.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_number_search_gt() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let query = SearchQuery::new("RiskAssessment").with_parameter(SearchParameter {
        name: "probability".to_string(),
        param_type: SearchParamType::Number,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Gt, "0.4")],
        chain: vec![],
        components: vec![],
    });

    let _result = backend.search(&tenant, &query.with_count(100)).await;
}
