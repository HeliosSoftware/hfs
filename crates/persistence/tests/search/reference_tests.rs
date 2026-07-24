//! Tests for reference search parameters.
//!
//! This module tests reference-type search parameters including
//! relative references, absolute URLs, and logical references.

use serde_json::json;

use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

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
async fn seed_test_data(backend: &SqliteBackend, tenant: &TenantContext) {
    // Create patients
    let patient1 =
        json!({"resourceType": "Patient", "id": "patient-1", "name": [{"family": "Smith"}]});
    let patient2 =
        json!({"resourceType": "Patient", "id": "patient-2", "name": [{"family": "Jones"}]});
    backend
        .create_or_update(
            tenant,
            "Patient",
            "patient-1",
            patient1,
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create_or_update(
            tenant,
            "Patient",
            "patient-2",
            patient2,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Create observations referencing patients
    let obs1 = json!({
        "resourceType": "Observation",
        "status": "final",
        "subject": {"reference": "Patient/patient-1"},
        "code": {"coding": [{"code": "test1"}]}
    });
    let obs2 = json!({
        "resourceType": "Observation",
        "status": "final",
        "subject": {"reference": "Patient/patient-1"},
        "code": {"coding": [{"code": "test2"}]}
    });
    let obs3 = json!({
        "resourceType": "Observation",
        "status": "final",
        "subject": {"reference": "Patient/patient-2"},
        "code": {"coding": [{"code": "test3"}]}
    });
    backend
        .create(tenant, "Observation", obs1, FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(tenant, "Observation", obs2, FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(tenant, "Observation", obs3, FhirVersion::default())
        .await
        .unwrap();
}

// ============================================================================
// Reference Search Tests
// ============================================================================

/// Test reference search with relative reference.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_reference_search_relative() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_data(&backend, &tenant).await;

    // Search for observations referencing patient-1
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "subject".to_string(),
        param_type: SearchParamType::Reference,
        modifier: None,
        values: vec![SearchValue::eq("Patient/patient-1")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find 2 observations for patient-1
    assert_eq!(result.resources.len(), 2);
    for resource in &result.resources.items {
        assert_eq!(
            resource.content()["subject"]["reference"],
            "Patient/patient-1"
        );
    }
}

/// Test reference search with ID only.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_reference_search_id_only() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_data(&backend, &tenant).await;

    // Search with just the ID
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "subject".to_string(),
        param_type: SearchParamType::Reference,
        modifier: None,
        values: vec![SearchValue::eq("patient-1")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find observations for patient-1
    assert!(!result.resources.is_empty());
}

/// Test reference search with type modifier.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_reference_search_type_modifier() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_data(&backend, &tenant).await;

    // Search with :Patient modifier
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "subject:Patient".to_string(),
        param_type: SearchParamType::Reference,
        modifier: None,
        values: vec![SearchValue::eq("patient-1")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    assert!(!result.resources.is_empty());
}

/// Test reference search with no results.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_reference_search_no_results() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_data(&backend, &tenant).await;

    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "subject".to_string(),
        param_type: SearchParamType::Reference,
        modifier: None,
        values: vec![SearchValue::eq("Patient/nonexistent")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    assert!(result.resources.is_empty());
}
