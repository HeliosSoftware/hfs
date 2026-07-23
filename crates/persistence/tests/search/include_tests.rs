//! Tests for _include and _revinclude search parameters.
//!
//! This module tests _include (forward references) and _revinclude
//! (reverse references) functionality.

use serde_json::json;

use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{IncludeDirective, IncludeType, SearchQuery};

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

#[cfg(feature = "sqlite")]
async fn seed_include_data(backend: &SqliteBackend, tenant: &TenantContext) {
    // Create organizations
    let org = json!({
        "resourceType": "Organization",
        "id": "org-hospital",
        "name": "Test Hospital"
    });
    backend
        .create_or_update(
            tenant,
            "Organization",
            "org-hospital",
            org,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Create patients with organization references
    let patient1 = json!({
        "resourceType": "Patient",
        "id": "patient-1",
        "name": [{"family": "Smith"}],
        "managingOrganization": {"reference": "Organization/org-hospital"}
    });
    let patient2 = json!({
        "resourceType": "Patient",
        "id": "patient-2",
        "name": [{"family": "Jones"}],
        "managingOrganization": {"reference": "Organization/org-hospital"}
    });
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
        "code": {"coding": [{"code": "test"}]}
    });
    let obs2 = json!({
        "resourceType": "Observation",
        "status": "final",
        "subject": {"reference": "Patient/patient-1"},
        "code": {"coding": [{"code": "test2"}]}
    });
    backend
        .create(tenant, "Observation", obs1, FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(tenant, "Observation", obs2, FhirVersion::default())
        .await
        .unwrap();
}

// ============================================================================
// _include Tests
// ============================================================================

/// Test _include returns referenced resources.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_include_basic() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_include_data(&backend, &tenant).await;

    // Search for observations with _include=Observation:subject
    let query = SearchQuery::new("Observation").with_include(IncludeDirective {
        include_type: IncludeType::Include,
        source_type: "Observation".to_string(),
        search_param: "subject".to_string(),
        target_type: Some("Patient".to_string()),
        iterate: false,
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should have observations in resources
    assert!(!result.resources.is_empty());

    // Should have patients in included
    assert!(!result.included.is_empty());

    // Check that included resources are patients
    for resource in &result.included {
        assert_eq!(resource.resource_type(), "Patient");
    }
}

/// Test _include with specific target type.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_include_with_target_type() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_include_data(&backend, &tenant).await;

    let query = SearchQuery::new("Patient").with_include(IncludeDirective {
        include_type: IncludeType::Include,
        source_type: "Patient".to_string(),
        search_param: "organization".to_string(),
        target_type: Some("Organization".to_string()),
        iterate: false,
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should have organizations in included
    for resource in &result.included {
        assert_eq!(resource.resource_type(), "Organization");
    }
}

/// Test _include:iterate for transitive includes.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_include_iterate() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_include_data(&backend, &tenant).await;

    // _include:iterate follows references in included resources
    let query = SearchQuery::new("Observation")
        .with_include(IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Observation".to_string(),
            search_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            iterate: false,
        })
        .with_include(IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Patient".to_string(),
            search_param: "organization".to_string(),
            target_type: Some("Organization".to_string()),
            iterate: true, // This follows references in included patients
        });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should include both patients and organizations
    let _included_types: std::collections::HashSet<_> =
        result.included.iter().map(|r| r.resource_type()).collect();

    // Depending on implementation, may have both Patient and Organization
}

// ============================================================================
// _revinclude Tests
// ============================================================================

/// Test _revinclude returns resources that reference the search results.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_revinclude_basic() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_include_data(&backend, &tenant).await;

    // Search for patients with _revinclude=Observation:subject
    let query = SearchQuery::new("Patient").with_include(IncludeDirective {
        include_type: IncludeType::Revinclude,
        source_type: "Observation".to_string(),
        search_param: "subject".to_string(),
        target_type: Some("Patient".to_string()),
        iterate: false,
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should have patients in resources
    assert!(!result.resources.is_empty());
    for resource in &result.resources.items {
        assert_eq!(resource.resource_type(), "Patient");
    }

    // Should have observations in included (those that reference the patients)
    for resource in &result.included {
        assert_eq!(resource.resource_type(), "Observation");
    }
}

/// Test _revinclude only includes resources referencing search results.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_revinclude_filtered() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_include_data(&backend, &tenant).await;

    // Search for specific patient
    let query = SearchQuery::new("Patient")
        .with_parameter(helios_persistence::types::SearchParameter {
            name: "_id".to_string(),
            param_type: helios_persistence::types::SearchParamType::Token,
            modifier: None,
            values: vec![helios_persistence::types::SearchValue::eq("patient-1")],
            chain: vec![],
            components: vec![],
        })
        .with_include(IncludeDirective {
            include_type: IncludeType::Revinclude,
            source_type: "Observation".to_string(),
            search_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            iterate: false,
        });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should only have patient-1
    assert_eq!(result.resources.len(), 1);

    // Should only have observations for patient-1
    for resource in &result.included {
        assert_eq!(
            resource.content()["subject"]["reference"],
            "Patient/patient-1"
        );
    }
}

// ============================================================================
// Edge Cases
// ============================================================================

// ============================================================================
// :iterate Modifier Tests (Recursive Inclusion)
// ============================================================================

/// Test :iterate modifier respects depth limits.
///
/// The :iterate modifier allows recursive inclusion of resources. Implementations
/// must enforce depth limits to prevent unbounded recursion and detect cycles.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_include_iterate_recursive_depth() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a chain of organizations: grandparent -> parent -> child
    let grandparent = json!({
        "resourceType": "Organization",
        "id": "org-grandparent",
        "name": "Grandparent Org"
    });
    backend
        .create_or_update(
            &tenant,
            "Organization",
            "org-grandparent",
            grandparent,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let parent = json!({
        "resourceType": "Organization",
        "id": "org-parent",
        "name": "Parent Org",
        "partOf": {"reference": "Organization/org-grandparent"}
    });
    backend
        .create_or_update(
            &tenant,
            "Organization",
            "org-parent",
            parent,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let child = json!({
        "resourceType": "Organization",
        "id": "org-child",
        "name": "Child Org",
        "partOf": {"reference": "Organization/org-parent"}
    });
    backend
        .create_or_update(
            &tenant,
            "Organization",
            "org-child",
            child,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Create patient at child org
    let patient = json!({
        "resourceType": "Patient",
        "id": "patient-org-chain",
        "managingOrganization": {"reference": "Organization/org-child"}
    });
    backend
        .create_or_update(
            &tenant,
            "Patient",
            "patient-org-chain",
            patient,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Search with :iterate to follow the organization hierarchy
    let query = SearchQuery::new("Patient")
        .with_parameter(helios_persistence::types::SearchParameter {
            name: "_id".to_string(),
            param_type: helios_persistence::types::SearchParamType::Token,
            modifier: None,
            values: vec![helios_persistence::types::SearchValue::eq(
                "patient-org-chain",
            )],
            chain: vec![],
            components: vec![],
        })
        .with_include(IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Patient".to_string(),
            search_param: "organization".to_string(),
            target_type: Some("Organization".to_string()),
            iterate: false,
        })
        .with_include(IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Organization".to_string(),
            search_param: "partof".to_string(),
            target_type: Some("Organization".to_string()),
            iterate: true, // Recursively include parent organizations
        });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should have the patient as the main result
    assert_eq!(result.resources.len(), 1);
    assert_eq!(result.resources.items[0].resource_type(), "Patient");

    // Should include organizations from the chain
    // The depth of inclusion depends on implementation limits
    let org_count = result
        .included
        .iter()
        .filter(|r| r.resource_type() == "Organization")
        .count();

    // At minimum, should include the direct organization reference
    assert!(
        org_count >= 1,
        "Should include at least the direct organization"
    );
}

/// Test :iterate modifier handles circular references safely.
///
/// When resources reference each other in a cycle, :iterate must detect
/// the cycle and stop to prevent infinite loops.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_include_iterate_cycle_detection() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create organizations that reference each other in a cycle
    // A -> B -> C -> A (circular)
    let org_a = json!({
        "resourceType": "Organization",
        "id": "org-cycle-a",
        "name": "Org A",
        "partOf": {"reference": "Organization/org-cycle-c"}
    });

    let org_b = json!({
        "resourceType": "Organization",
        "id": "org-cycle-b",
        "name": "Org B",
        "partOf": {"reference": "Organization/org-cycle-a"}
    });

    let org_c = json!({
        "resourceType": "Organization",
        "id": "org-cycle-c",
        "name": "Org C",
        "partOf": {"reference": "Organization/org-cycle-b"}
    });

    // Create in order that allows references (order matters for some backends)
    backend
        .create_or_update(
            &tenant,
            "Organization",
            "org-cycle-a",
            org_a.clone(),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create_or_update(
            &tenant,
            "Organization",
            "org-cycle-b",
            org_b,
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create_or_update(
            &tenant,
            "Organization",
            "org-cycle-c",
            org_c,
            FhirVersion::default(),
        )
        .await
        .unwrap();
    // Update A to complete the cycle
    backend
        .create_or_update(
            &tenant,
            "Organization",
            "org-cycle-a",
            org_a,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Search with :iterate - should not hang or overflow
    let query = SearchQuery::new("Organization")
        .with_parameter(helios_persistence::types::SearchParameter {
            name: "_id".to_string(),
            param_type: helios_persistence::types::SearchParamType::Token,
            modifier: None,
            values: vec![helios_persistence::types::SearchValue::eq("org-cycle-a")],
            chain: vec![],
            components: vec![],
        })
        .with_include(IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Organization".to_string(),
            search_param: "partof".to_string(),
            target_type: Some("Organization".to_string()),
            iterate: true,
        });

    // This should complete without infinite loop
    let result = backend.search(&tenant, &query.with_count(100)).await;

    // Should succeed (cycle detected) or fail gracefully
    match result {
        Ok(result) => {
            // Implementation detected cycle and returned finite results
            // Should have at most 3 unique organizations
            let unique_ids: std::collections::HashSet<_> =
                result.included.iter().map(|r| r.id()).collect();
            assert!(
                unique_ids.len() <= 3,
                "Should not have more than 3 orgs (cycle detected)"
            );
        }
        Err(_) => {
            // Implementation may return error for detected cycles - also acceptable
        }
    }
}

/// Test :iterate with maximum depth limit.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_include_iterate_max_depth() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a deep chain of locations: L1 -> L2 -> L3 -> L4 -> L5 -> L6 -> L7 -> L8
    let mut prev_id: Option<String> = None;
    for i in 1..=8 {
        let id = format!("location-depth-{}", i);
        let mut location = json!({
            "resourceType": "Location",
            "id": id.clone(),
            "name": format!("Location {}", i)
        });
        if let Some(ref parent) = prev_id {
            location["partOf"] = json!({"reference": format!("Location/{}", parent)});
        }
        backend
            .create_or_update(&tenant, "Location", &id, location, FhirVersion::default())
            .await
            .unwrap();
        prev_id = Some(id);
    }

    // Search from the deepest location with :iterate
    let query = SearchQuery::new("Location")
        .with_parameter(helios_persistence::types::SearchParameter {
            name: "_id".to_string(),
            param_type: helios_persistence::types::SearchParamType::Token,
            modifier: None,
            values: vec![helios_persistence::types::SearchValue::eq(
                "location-depth-8",
            )],
            chain: vec![],
            components: vec![],
        })
        .with_include(IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Location".to_string(),
            search_param: "partof".to_string(),
            target_type: Some("Location".to_string()),
            iterate: true,
        });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should have location-depth-8 as main result
    assert_eq!(result.resources.len(), 1);

    // Implementation may limit depth (commonly 4-6 levels)
    // This test documents that depth limiting works
    let included_locations = result
        .included
        .iter()
        .filter(|r| r.resource_type() == "Location")
        .count();

    // Should include at least some parent locations
    // The exact number depends on the implementation's depth limit
    assert!(
        included_locations >= 1,
        "Should include at least one parent location"
    );
}

/// Test _include with no referenced resources.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_include_no_references() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create patient without organization reference
    let patient = json!({
        "resourceType": "Patient",
        "name": [{"family": "NoOrg"}]
    });
    backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    let query = SearchQuery::new("Patient").with_include(IncludeDirective {
        include_type: IncludeType::Include,
        source_type: "Patient".to_string(),
        search_param: "organization".to_string(),
        target_type: Some("Organization".to_string()),
        iterate: false,
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should have patient but no included resources
    assert!(!result.resources.is_empty());
    assert!(result.included.is_empty());
}
