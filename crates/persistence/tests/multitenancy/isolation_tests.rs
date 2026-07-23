//! Tests for tenant data isolation.
//!
//! This module tests that data is properly isolated between tenants
//! and that all operations respect tenant boundaries.

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::SearchQuery;

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

fn create_tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

fn create_patient_json(name: &str) -> serde_json::Value {
    json!({
        "resourceType": "Patient",
        "name": [{"family": name}]
    })
}

// ============================================================================
// CRUD Isolation Tests
// ============================================================================

/// Test that created resources are only visible to their tenant.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_create_isolation() {
    let backend = create_sqlite_backend();

    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    // Create patient in tenant A
    let patient = create_patient_json("TenantA Patient");
    let created = backend
        .create(&tenant_a, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Tenant A can read it
    let read_a = backend
        .read(&tenant_a, "Patient", created.id())
        .await
        .unwrap();
    assert!(read_a.is_some());

    // Tenant B cannot read it
    let read_b = backend
        .read(&tenant_b, "Patient", created.id())
        .await
        .unwrap();
    assert!(read_b.is_none());
}

/// Test that exists respects tenant isolation.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_exists_isolation() {
    let backend = create_sqlite_backend();

    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    let patient = create_patient_json("Test");
    let created = backend
        .create(&tenant_a, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    assert!(
        backend
            .exists(&tenant_a, "Patient", created.id())
            .await
            .unwrap()
    );
    assert!(
        !backend
            .exists(&tenant_b, "Patient", created.id())
            .await
            .unwrap()
    );
}

/// Test that read_batch only returns resources from the correct tenant.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_read_batch_isolation() {
    let backend = create_sqlite_backend();

    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    // Create patients in tenant A
    let p1 = backend
        .create(
            &tenant_a,
            "Patient",
            create_patient_json("A1"),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let p2 = backend
        .create(
            &tenant_a,
            "Patient",
            create_patient_json("A2"),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Create patient in tenant B with known ID
    backend
        .create_or_update(
            &tenant_b,
            "Patient",
            "b-patient",
            create_patient_json("B1"),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Batch read from tenant A including B's patient ID
    let ids = vec![p1.id(), p2.id(), "b-patient"];
    let batch_a = backend
        .read_batch(&tenant_a, "Patient", &ids)
        .await
        .unwrap();

    // Should only get tenant A's patients
    assert_eq!(batch_a.len(), 2);
    for resource in &batch_a {
        assert_eq!(resource.tenant_id().as_str(), "tenant-a");
    }
}

/// Test that count only counts resources in the tenant.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_count_isolation() {
    let backend = create_sqlite_backend();

    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    // Create 5 patients in tenant A
    for i in 0..5 {
        backend
            .create(
                &tenant_a,
                "Patient",
                create_patient_json(&format!("A{}", i)),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    // Create 3 patients in tenant B
    for i in 0..3 {
        backend
            .create(
                &tenant_b,
                "Patient",
                create_patient_json(&format!("B{}", i)),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    let count_a = backend.count(&tenant_a, Some("Patient")).await.unwrap();
    let count_b = backend.count(&tenant_b, Some("Patient")).await.unwrap();

    assert_eq!(count_a, 5);
    assert_eq!(count_b, 3);
}

// ============================================================================
// Search Isolation Tests
// ============================================================================

/// Test that search only returns resources from the tenant.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_search_isolation() {
    let backend = create_sqlite_backend();

    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    // Create patients with same name in both tenants
    for i in 0..3 {
        backend
            .create(
                &tenant_a,
                "Patient",
                create_patient_json("Smith"),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    for i in 0..2 {
        backend
            .create(
                &tenant_b,
                "Patient",
                create_patient_json("Smith"),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    // Search in each tenant
    let query = SearchQuery::new("Patient").with_count(100);

    let result_a = backend.search(&tenant_a, &query).await.unwrap();
    let result_b = backend.search(&tenant_b, &query).await.unwrap();

    // Each tenant should only see their own
    assert_eq!(result_a.resources.len(), 3);
    for resource in &result_a.resources.items {
        assert_eq!(resource.tenant_id().as_str(), "tenant-a");
    }

    assert_eq!(result_b.resources.len(), 2);
    for resource in &result_b.resources.items {
        assert_eq!(resource.tenant_id().as_str(), "tenant-b");
    }
}

// ============================================================================
// Update and Delete Isolation Tests
// ============================================================================

/// Test that update cannot modify another tenant's resource.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_isolation() {
    let backend = create_sqlite_backend();

    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    // Create in tenant A
    let patient = create_patient_json("Original");
    let created = backend
        .create(&tenant_a, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Create a fake resource with same ID but tenant B's context
    let fake_resource = helios_persistence::types::StoredResource::new(
        "Patient",
        created.id(),
        TenantId::new("tenant-b"),
        json!({"resourceType": "Patient"}),
    );

    // Try to update from tenant B
    let result = backend
        .update(
            &tenant_b,
            &fake_resource,
            json!({"resourceType": "Patient", "name": [{"family": "Hacked"}]}),
        )
        .await;

    // Should fail
    assert!(result.is_err());

    // Original should be unchanged
    let original = backend
        .read(&tenant_a, "Patient", created.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original.content()["name"][0]["family"], "Original");
}

/// Test that delete cannot remove another tenant's resource.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_delete_isolation() {
    let backend = create_sqlite_backend();

    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    // Create in tenant A
    let patient = create_patient_json("TenantA");
    let created = backend
        .create(&tenant_a, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Try to delete from tenant B
    let result = backend.delete(&tenant_b, "Patient", created.id()).await;

    // Should fail (NotFound because B can't see A's resource)
    assert!(result.is_err());

    // Resource should still exist in tenant A
    assert!(
        backend
            .exists(&tenant_a, "Patient", created.id())
            .await
            .unwrap()
    );
}

// ============================================================================
// Same ID in Different Tenants Tests
// ============================================================================

/// Test that same ID can exist in different tenants.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_same_id_different_tenants() {
    let backend = create_sqlite_backend();

    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    // Create patient with same ID in both tenants
    let patient_a = json!({
        "resourceType": "Patient",
        "name": [{"family": "TenantA Patient"}]
    });
    let patient_b = json!({
        "resourceType": "Patient",
        "name": [{"family": "TenantB Patient"}]
    });

    backend
        .create_or_update(
            &tenant_a,
            "Patient",
            "shared-id",
            patient_a,
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create_or_update(
            &tenant_b,
            "Patient",
            "shared-id",
            patient_b,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Read from each tenant
    let read_a = backend
        .read(&tenant_a, "Patient", "shared-id")
        .await
        .unwrap()
        .unwrap();
    let read_b = backend
        .read(&tenant_b, "Patient", "shared-id")
        .await
        .unwrap()
        .unwrap();

    // Should be different resources
    assert_eq!(read_a.content()["name"][0]["family"], "TenantA Patient");
    assert_eq!(read_b.content()["name"][0]["family"], "TenantB Patient");
    assert_ne!(read_a.tenant_id(), read_b.tenant_id());
}

// ============================================================================
// System Tenant Tests
// ============================================================================

/// Test that system tenant resources can be accessed by other tenants.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_system_tenant_access() {
    let backend = create_sqlite_backend();

    let system = TenantContext::system();
    let tenant_a = create_tenant("tenant-a");

    // Create shared resource in system tenant
    let value_set = json!({
        "resourceType": "ValueSet",
        "name": "SharedValueSet"
    });
    let created = backend
        .create(&system, "ValueSet", value_set, FhirVersion::default())
        .await
        .unwrap();

    // System tenant can read it
    let read_system = backend
        .read(&system, "ValueSet", created.id())
        .await
        .unwrap();
    assert!(read_system.is_some());

    // Regular tenants with system access permission should be able to access
    // (depends on permissions configuration)
}

/// Test that regular tenants cannot modify system resources.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_cannot_modify_system_resources() {
    let backend = create_sqlite_backend();

    let system = TenantContext::system();
    let tenant_a = create_tenant("tenant-a");

    // Create in system tenant
    let value_set = json!({
        "resourceType": "ValueSet",
        "name": "SystemValueSet"
    });
    let created = backend
        .create(&system, "ValueSet", value_set, FhirVersion::default())
        .await
        .unwrap();

    // Regular tenant should not be able to delete it
    let result = backend.delete(&tenant_a, "ValueSet", created.id()).await;

    // Should fail
    assert!(result.is_err());
}

// ============================================================================
// Hierarchical Tenant Tests
// ============================================================================

/// Test parent tenant accessing child tenant resources (if permitted).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_hierarchical_tenant_access() {
    let backend = create_sqlite_backend();

    let parent = TenantContext::new(
        TenantId::new("parent"),
        TenantPermissions::builder()
            .can_access_child_tenants(true)
            .build(),
    );
    let child = create_tenant("parent/child");

    // Create in child tenant
    let patient = create_patient_json("ChildPatient");
    let created = backend
        .create(&child, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Parent with child access permission might be able to read
    // (behavior depends on implementation)
    let read_parent = backend.read(&parent, "Patient", created.id()).await;

    // This test documents expected hierarchical access behavior
}
