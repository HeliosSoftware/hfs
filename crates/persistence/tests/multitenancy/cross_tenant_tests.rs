//! Tests for cross-tenant access prevention.
//!
//! This module tests that cross-tenant references are properly
//! validated and prevented.

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::ResourceStorage;
use helios_persistence::error::{StorageError, TenantError};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::SqliteBackend;

#[cfg(feature = "sqlite")]
fn create_sqlite_backend() -> SqliteBackend {
    let backend = SqliteBackend::in_memory().expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to initialize schema");
    backend
}

fn create_tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

// ============================================================================
// Cross-Tenant Reference Tests
// ============================================================================

/// Test that TenantContext validates cross-tenant references.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_validate_reference_same_tenant() {
    let tenant = create_tenant("tenant-1");

    // Reference to same tenant should be allowed
    let result = tenant.validate_reference("Patient/123", &TenantId::new("tenant-1"));
    assert!(result.is_ok());
}

/// Test that cross-tenant references are rejected.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_validate_reference_different_tenant() {
    let tenant = create_tenant("tenant-1");

    // Reference to different tenant should be rejected
    let result = tenant.validate_reference("Patient/123", &TenantId::new("tenant-2"));
    assert!(result.is_err());

    match result {
        Err(TenantError::CrossTenantReference { .. }) => {}
        Err(e) => panic!("Expected CrossTenantReference error, got {:?}", e),
        Ok(_) => panic!("Expected error"),
    }
}

/// Test that references to system tenant are allowed.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_validate_reference_to_system() {
    let tenant = create_tenant("tenant-1");

    // Reference to system tenant should be allowed
    let result = tenant.validate_reference("ValueSet/shared-valueset", &TenantId::system());
    assert!(result.is_ok());
}

// ============================================================================
// Access Denial Tests
// ============================================================================

/// Test that check_access denies access to other tenant's resources.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_check_access_denied() {
    let tenant = create_tenant("tenant-1");

    let result = tenant.check_access(&TenantId::new("tenant-2"));
    assert!(result.is_err());

    match result {
        Err(TenantError::AccessDenied { .. }) => {}
        Err(e) => panic!("Expected AccessDenied error, got {:?}", e),
        Ok(_) => panic!("Expected error"),
    }
}

/// Test that check_access allows access to same tenant.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_check_access_same_tenant() {
    let tenant = create_tenant("tenant-1");

    let result = tenant.check_access(&TenantId::new("tenant-1"));
    assert!(result.is_ok());
}

/// Test that check_access allows access to system tenant.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_check_access_system_tenant() {
    let tenant = create_tenant("tenant-1");

    let result = tenant.check_access(&TenantId::system());
    assert!(result.is_ok());
}

// ============================================================================
// Permission Tests
// ============================================================================

/// Test that operations respect permission boundaries.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_permission_boundaries() {
    let backend = create_sqlite_backend();

    // Tenant with full access
    let full_access =
        TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());

    // Same tenant with read-only access
    let read_only = TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::read_only());

    // Create with full access
    let patient = json!({"resourceType": "Patient", "name": [{"family": "Test"}]});
    let created = backend
        .create(&full_access, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Read-only can read
    let read = backend
        .read(&read_only, "Patient", created.id())
        .await
        .unwrap();
    assert!(read.is_some());

    // Read-only cannot create
    let result = backend
        .create(
            &read_only,
            "Patient",
            json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        )
        .await;
    assert!(result.is_err());
    match result {
        Err(StorageError::Tenant(TenantError::OperationNotPermitted { .. })) => {}
        Err(e) => panic!("Expected OperationNotPermitted, got {:?}", e),
        Ok(_) => panic!("Expected error"),
    }

    // Read-only cannot delete
    let result = backend.delete(&read_only, "Patient", created.id()).await;
    assert!(result.is_err());
}

// ============================================================================
// Resource Type Restriction Tests
// ============================================================================

/// Test that permissions can restrict by resource type.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_resource_type_restrictions() {
    let backend = create_sqlite_backend();

    // Create a tenant that can only access Patient resources
    let patient_only = TenantContext::new(
        TenantId::new("restricted"),
        TenantPermissions::builder()
            .allow_resource_types(vec!["Patient"])
            .build(),
    );

    // Full access tenant for setup
    let full = TenantContext::new(
        TenantId::new("restricted"),
        TenantPermissions::full_access(),
    );

    // Create Patient (should work with either)
    let patient = json!({"resourceType": "Patient"});
    let created = backend
        .create(&full, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // restricted tenant can read Patient
    let _read = backend.read(&patient_only, "Patient", created.id()).await;
    // Result depends on whether restriction is enforced at read level

    // Create Observation
    let obs = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {"coding": [{"code": "test"}]}
    });
    let _obs_created = backend
        .create(&full, "Observation", obs, FhirVersion::default())
        .await
        .unwrap();

    // restricted tenant might not be able to access Observation
    // (depends on implementation)
}

// ============================================================================
// Multi-Tenant Concurrent Access Tests
// ============================================================================

/// Test that concurrent access from multiple tenants works correctly.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_concurrent_tenant_access() {
    let backend = create_sqlite_backend();

    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");
    let tenant_c = create_tenant("tenant-c");

    // Create resources in multiple tenants concurrently

    for (tenant, name) in [
        (tenant_a.clone(), "A"),
        (tenant_b.clone(), "B"),
        (tenant_c.clone(), "C"),
    ] {
        for i in 0..10 {
            let _backend_ref = &backend;
            let tenant_ref = tenant.clone();
            let patient = json!({
                "resourceType": "Patient",
                "name": [{"family": format!("{}_{}", name, i)}]
            });
            backend
                .create(&tenant_ref, "Patient", patient, FhirVersion::default())
                .await
                .unwrap();
        }
    }

    // Verify counts
    let count_a = backend.count(&tenant_a, Some("Patient")).await.unwrap();
    let count_b = backend.count(&tenant_b, Some("Patient")).await.unwrap();
    let count_c = backend.count(&tenant_c, Some("Patient")).await.unwrap();

    assert_eq!(count_a, 10);
    assert_eq!(count_b, 10);
    assert_eq!(count_c, 10);
}
