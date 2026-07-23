//! Tests for token search parameters.
//!
//! This module tests token-type search parameters including
//! system|code, code-only, system-only, :not, :of-type, and :text modifiers.

use serde_json::json;

use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
};

use helios_fhir::FhirVersion;

#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::SqliteBackend;

// ============================================================================
// Helper Functions
// ============================================================================

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

#[cfg(feature = "sqlite")]
async fn seed_test_patients(backend: &SqliteBackend, tenant: &TenantContext) {
    let patients = vec![
        json!({
            "resourceType": "Patient",
            "identifier": [
                {"system": "http://example.org/mrn", "value": "MRN001"},
                {"system": "http://hl7.org/fhir/sid/us-ssn", "value": "123-45-6789"}
            ],
            "gender": "male"
        }),
        json!({
            "resourceType": "Patient",
            "identifier": [
                {"system": "http://example.org/mrn", "value": "MRN002"}
            ],
            "gender": "female"
        }),
        json!({
            "resourceType": "Patient",
            "identifier": [
                {"system": "http://example.org/mrn", "value": "MRN003"},
                {"value": "LOCAL123"}  // Identifier without system
            ],
            "gender": "male"
        }),
    ];

    for patient in patients {
        backend
            .create(tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }
}

#[cfg(feature = "sqlite")]
async fn seed_test_observations(backend: &SqliteBackend, tenant: &TenantContext) {
    let observations = vec![
        json!({
            "resourceType": "Observation",
            "status": "final",
            "code": {
                "coding": [
                    {"system": "http://loinc.org", "code": "8867-4", "display": "Heart rate"},
                    {"system": "http://snomed.info/sct", "code": "364075005"}
                ]
            }
        }),
        json!({
            "resourceType": "Observation",
            "status": "preliminary",
            "code": {
                "coding": [
                    {"system": "http://loinc.org", "code": "8310-5", "display": "Body temperature"}
                ]
            }
        }),
        json!({
            "resourceType": "Observation",
            "status": "final",
            "code": {
                "coding": [
                    {"system": "http://loinc.org", "code": "29463-7", "display": "Body weight"}
                ]
            }
        }),
    ];

    for obs in observations {
        backend
            .create(tenant, "Observation", obs, FhirVersion::default())
            .await
            .unwrap();
    }
}

// ============================================================================
// Token Search Tests - System|Code Format
// ============================================================================

/// Test token search with system|code format.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_token_search_system_and_code() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search for identifier with specific system and value
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token(Some("http://example.org/mrn"), "MRN001")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    assert_eq!(result.resources.len(), 1);
    let identifiers = &result.resources.items[0].content()["identifier"];
    let has_match = identifiers
        .as_array()
        .unwrap()
        .iter()
        .any(|id| id["system"] == "http://example.org/mrn" && id["value"] == "MRN001");
    assert!(has_match);
}

/// Test token search with code only (matches any system).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_token_search_code_only() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_observations(&backend, &tenant).await;

    // Search for LOINC code without specifying system
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token(None, "8867-4")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    assert!(!result.resources.is_empty());
}

/// Test token search with system only (|).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_token_search_system_only() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search for any identifier from the MRN system
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token_system_only("http://example.org/mrn")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find all patients with MRN identifiers
    assert!(result.resources.len() >= 2);
}

// ============================================================================
// Token Search Tests - Gender (CodeableConcept)
// ============================================================================

/// Test token search for gender.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_token_search_gender() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search for male patients
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "gender".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token(None, "male")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    assert!(!result.resources.is_empty());
    for resource in &result.resources.items {
        assert_eq!(resource.content()["gender"], "male");
    }
}

/// Test token search for female.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_token_search_gender_female() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "gender".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token(None, "female")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    assert_eq!(result.resources.len(), 1);
}

// ============================================================================
// Token Search Tests - Status
// ============================================================================

/// Test token search for observation status.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_token_search_status() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_observations(&backend, &tenant).await;

    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "status".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token(None, "final")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find observations with status=final
    for resource in &result.resources.items {
        assert_eq!(resource.content()["status"], "final");
    }
}

// ============================================================================
// Token Search Tests - :not Modifier
// ============================================================================

/// Test :not modifier to exclude values.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_token_search_not_modifier() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search for patients that are NOT male
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "gender".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::Not),
        values: vec![SearchValue::token(None, "male")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should only find female patients
    for resource in &result.resources.items {
        assert_ne!(resource.content()["gender"], "male");
    }
}

// ============================================================================
// Token Search Tests - Multiple Values (OR)
// ============================================================================

/// Test token search with multiple OR values.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_token_search_or_values() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_observations(&backend, &tenant).await;

    // Search for observations with code 8867-4 OR 8310-5
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![
            SearchValue::token(Some("http://loinc.org"), "8867-4"),
            SearchValue::token(Some("http://loinc.org"), "8310-5"),
        ],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find observations with either code
    assert!(result.resources.len() >= 2);
}

// ============================================================================
// Token Search Tests - Identifier Without System
// ============================================================================

/// Test searching for identifier without system.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_token_search_identifier_no_system() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search for LOCAL123 identifier (which has no system)
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token(None, "LOCAL123")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find the patient with LOCAL123 identifier
    assert!(!result.resources.is_empty());
}

// ============================================================================
// Token Search Tests - No Results
// ============================================================================

/// Test token search with no matching results.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_token_search_no_results() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search for nonexistent identifier
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token(
            Some("http://example.org/mrn"),
            "NONEXISTENT",
        )],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    assert!(result.resources.is_empty());
}
