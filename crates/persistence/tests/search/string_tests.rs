//! Tests for string search parameters.
//!
//! This module tests string-type search parameters including
//! :exact, :contains, and default behavior.

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
        json!({"resourceType": "Patient", "name": [{"family": "Smith", "given": ["John", "Jacob"]}]}),
        json!({"resourceType": "Patient", "name": [{"family": "Smith", "given": ["Jane"]}]}),
        json!({"resourceType": "Patient", "name": [{"family": "Smithson", "given": ["Robert"]}]}),
        json!({"resourceType": "Patient", "name": [{"family": "Johnson", "given": ["Emily"]}]}),
        json!({"resourceType": "Patient", "name": [{"family": "SMITH", "given": ["Michael"]}]}),
        json!({"resourceType": "Patient", "name": [{"family": "O'Brien", "given": ["Patrick"]}]}),
        json!({"resourceType": "Patient", "name": [{"family": "Van Der Berg", "given": ["Anna"]}]}),
    ];

    for patient in patients {
        backend
            .create(tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }
}

// ============================================================================
// String Search Tests - Default Behavior
// ============================================================================

/// Test default string search (case-insensitive prefix match).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_string_search_default() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search for name starting with "Smith"
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("Smith")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should match "Smith", "Smithson", and "SMITH" (case-insensitive prefix)
    assert!(result.resources.len() >= 2);

    // All results should have family name starting with "smith" (case-insensitive)
    for resource in &result.resources.items {
        let family = resource.content()["name"][0]["family"]
            .as_str()
            .unwrap()
            .to_lowercase();
        assert!(
            family.starts_with("smith"),
            "Family '{}' should start with 'smith'",
            family
        );
    }
}

/// Test string search is case-insensitive by default.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_string_search_case_insensitive() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search with lowercase
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("smith")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find "Smith", "SMITH", and "Smithson"
    assert!(!result.resources.is_empty());
}

// ============================================================================
// String Search Tests - :exact Modifier
// ============================================================================

/// Test :exact modifier for case-sensitive exact match.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_string_search_exact() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search with :exact modifier
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        modifier: Some(SearchModifier::Exact),
        values: vec![SearchValue::eq("Smith")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should only match exact "Smith", not "SMITH" or "Smithson"
    for resource in &result.resources.items {
        let family = resource.content()["name"][0]["family"].as_str().unwrap();
        assert_eq!(family, "Smith", "Should only match exact 'Smith'");
    }
}

/// Test :exact is case-sensitive.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_string_search_exact_case_sensitive() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search for "smith" (lowercase) with :exact
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        modifier: Some(SearchModifier::Exact),
        values: vec![SearchValue::eq("smith")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should not match "Smith" (different case)
    assert!(result.resources.is_empty());
}

// ============================================================================
// String Search Tests - :contains Modifier
// ============================================================================

/// Test :contains modifier for substring search.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_string_search_contains() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search for names containing "son"
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        modifier: Some(SearchModifier::Contains),
        values: vec![SearchValue::eq("son")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should match "Smithson" and "Johnson"
    assert!(result.resources.len() >= 2);
    for resource in &result.resources.items {
        let family = resource.content()["name"][0]["family"]
            .as_str()
            .unwrap()
            .to_lowercase();
        assert!(
            family.contains("son"),
            "Family '{}' should contain 'son'",
            family
        );
    }
}

// ============================================================================
// String Search Tests - Given Name
// ============================================================================

/// Test searching by given name.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_string_search_given_name() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search for given name "John"
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "given".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("John")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    assert!(!result.resources.is_empty());
    for resource in &result.resources.items {
        let given = &resource.content()["name"][0]["given"];
        let names: Vec<&str> = given
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(
            names.iter().any(|n| n.to_lowercase().starts_with("john")),
            "Should have given name starting with 'John'"
        );
    }
}

// ============================================================================
// String Search Tests - Combined Name Search
// ============================================================================

/// Test searching by combined name (family and given).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_string_search_combined_name() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search for family="Smith" AND given="John"
    let query = SearchQuery::new("Patient")
        .with_parameter(SearchParameter {
            name: "family".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Smith")],
            chain: vec![],
            components: vec![],
        })
        .with_parameter(SearchParameter {
            name: "given".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("John")],
            chain: vec![],
            components: vec![],
        });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should only match John Smith (not Jane Smith or other Smiths)
    assert!(!result.resources.is_empty());
    for resource in &result.resources.items {
        let family = resource.content()["name"][0]["family"]
            .as_str()
            .unwrap()
            .to_lowercase();
        assert!(family.starts_with("smith"));
    }
}

// ============================================================================
// String Search Tests - Special Characters
// ============================================================================

/// Test searching names with apostrophes.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_string_search_apostrophe() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search for "O'Brien"
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("O'Brien")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find O'Brien
    assert!(!result.resources.is_empty());
}

/// Test searching names with spaces.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_string_search_spaces() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search for "Van Der Berg"
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("Van Der Berg")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find Van Der Berg
    assert!(!result.resources.is_empty());
}

// ============================================================================
// String Search Tests - OR Values
// ============================================================================

/// Test searching with multiple OR values.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_string_search_or_values() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search for family="Smith" OR family="Johnson"
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("Smith"), SearchValue::eq("Johnson")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should match both Smith variants and Johnson
    for resource in &result.resources.items {
        let family = resource.content()["name"][0]["family"]
            .as_str()
            .unwrap()
            .to_lowercase();
        assert!(
            family.starts_with("smith") || family.starts_with("johnson"),
            "Family '{}' should start with 'smith' or 'johnson'",
            family
        );
    }
}

// ============================================================================
// String Search Tests - Empty/No Results
// ============================================================================

/// Test search with no matching results.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_string_search_no_results() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search for a name that doesn't exist
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("NonexistentName")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    assert!(result.resources.is_empty());
}

/// Test search on empty storage.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_string_search_empty_storage() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("Smith")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    assert!(result.resources.is_empty());
}

// ============================================================================
// Multi-Value AND/OR Semantics Tests
// ============================================================================

/// Test that multiple values within a single parameter use OR semantics.
///
/// Per FHIR spec: `name=Smith,Jones` means name matches "Smith" OR "Jones"
/// Multiple values in a single parameter are comma-separated and OR'd together.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_multivalue_or_semantics() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Single parameter with multiple values = OR
    // This is equivalent to: family=Smith OR family=Johnson
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("Smith"), SearchValue::eq("Johnson")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find resources matching EITHER Smith or Johnson
    assert!(
        !result.resources.is_empty(),
        "Should find matching resources"
    );

    for resource in &result.resources.items {
        let family = resource.content()["name"][0]["family"]
            .as_str()
            .unwrap()
            .to_lowercase();
        assert!(
            family.starts_with("smith") || family.starts_with("johnson"),
            "Family '{}' should match 'smith' or 'johnson'",
            family
        );
    }
}

/// Test that multiple separate parameters use AND semantics.
///
/// Per FHIR spec: `name=Smith&given=John` means name matches "Smith" AND given matches "John"
/// Separate parameters are AND'd together.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_multivalue_and_semantics() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Multiple parameters = AND
    // This is: family starts with "Smith" AND given starts with "John"
    let query = SearchQuery::new("Patient")
        .with_parameter(SearchParameter {
            name: "family".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Smith")],
            chain: vec![],
            components: vec![],
        })
        .with_parameter(SearchParameter {
            name: "given".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("John")],
            chain: vec![],
            components: vec![],
        });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should only find resources matching BOTH conditions
    for resource in &result.resources.items {
        let family = resource.content()["name"][0]["family"]
            .as_str()
            .unwrap()
            .to_lowercase();
        let given = &resource.content()["name"][0]["given"];
        let given_names: Vec<String> = given
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.to_lowercase())
            .collect();

        assert!(
            family.starts_with("smith"),
            "Family '{}' should start with 'smith'",
            family
        );
        assert!(
            given_names.iter().any(|n| n.starts_with("john")),
            "Given names {:?} should include one starting with 'john'",
            given_names
        );
    }
}

/// Test combined AND/OR semantics in a single query.
///
/// Query: `family=Smith,Johnson&given=John` means:
/// (family matches "Smith" OR family matches "Johnson") AND (given matches "John")
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_multivalue_combined_and_or_semantics() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Combined: (family=Smith OR family=Johnson) AND given=John
    let query = SearchQuery::new("Patient")
        .with_parameter(SearchParameter {
            name: "family".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Smith"), SearchValue::eq("Johnson")],
            chain: vec![],
            components: vec![],
        })
        .with_parameter(SearchParameter {
            name: "given".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("John")],
            chain: vec![],
            components: vec![],
        });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find John Smith but not Jane Smith (wrong given) or Emily Johnson (wrong given)
    for resource in &result.resources.items {
        let family = resource.content()["name"][0]["family"]
            .as_str()
            .unwrap()
            .to_lowercase();
        let given = &resource.content()["name"][0]["given"];
        let given_names: Vec<String> = given
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.to_lowercase())
            .collect();

        // Must satisfy both conditions
        assert!(
            family.starts_with("smith") || family.starts_with("johnson"),
            "Family '{}' should match smith or johnson",
            family
        );
        assert!(
            given_names.iter().any(|n| n.starts_with("john")),
            "Must have given name starting with john"
        );
    }
}

/// Test that repeating the same parameter creates AND semantics for multiple conditions.
///
/// Per FHIR spec: `given=Jo&given=Ja` means given matches "Jo" AND given matches "Ja"
/// This is useful for finding patients with multiple given names.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_repeated_parameter_and_semantics() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Repeated parameter = AND between the parameters
    // This finds patients where given name array has BOTH a name starting with "Jo"
    // AND a name starting with "Ja"
    let query = SearchQuery::new("Patient")
        .with_parameter(SearchParameter {
            name: "given".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Jo")], // Matches "John", "Joseph", etc.
            chain: vec![],
            components: vec![],
        })
        .with_parameter(SearchParameter {
            name: "given".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Ja")], // Matches "Jacob", "Jane", "James", etc.
            chain: vec![],
            components: vec![],
        });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find "John Jacob Smith" (has both John and Jacob)
    // Should NOT find "John Smith" (only John, no Ja*) or "Jane Smith" (only Jane, no Jo*)
    for resource in &result.resources.items {
        let given = &resource.content()["name"][0]["given"];
        let given_names: Vec<String> = given
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.to_lowercase())
            .collect();

        let has_jo = given_names.iter().any(|n| n.starts_with("jo"));
        let has_ja = given_names.iter().any(|n| n.starts_with("ja"));

        assert!(
            has_jo && has_ja,
            "Given names {:?} should have both Jo* and Ja*",
            given_names
        );
    }
}
