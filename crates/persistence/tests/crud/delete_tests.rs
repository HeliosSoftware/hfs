//! Tests for resource delete operations.
//!
//! This module tests the `delete` method of the ResourceStorage trait
//! and related soft delete behavior.

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::ResourceStorage;
use helios_persistence::error::{ResourceError, StorageError};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

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

fn create_patient_json(name: &str) -> serde_json::Value {
    json!({
        "resourceType": "Patient",
        "name": [{"family": name}],
        "active": true
    })
}

// ============================================================================
// Delete Tests - Basic
// ============================================================================

/// Test that deleting a resource succeeds.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_delete_resource_success() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a resource
    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Delete it
    let result = backend.delete(&tenant, "Patient", created.id()).await;

    assert!(result.is_ok(), "Delete should succeed");
}

/// Test that resource is not readable after delete.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_delete_makes_resource_unreadable() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let id = created.id().to_string();

    backend.delete(&tenant, "Patient", &id).await.unwrap();

    let read = backend.read(&tenant, "Patient", &id).await.unwrap();
    assert!(read.is_none(), "Deleted resource should not be readable");
}

/// Test that exists returns false after delete.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_delete_makes_exists_false() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let id = created.id().to_string();

    backend.delete(&tenant, "Patient", &id).await.unwrap();

    let exists = backend.exists(&tenant, "Patient", &id).await.unwrap();
    assert!(!exists, "Deleted resource should not exist");
}

/// Test that count decreases after delete.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_delete_decreases_count() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create 3 resources
    let mut ids = Vec::new();
    for i in 0..3 {
        let patient = create_patient_json(&format!("Patient{}", i));
        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
        ids.push(created.id().to_string());
    }

    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 3);

    // Delete one
    backend.delete(&tenant, "Patient", &ids[0]).await.unwrap();
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 2);

    // Delete another
    backend.delete(&tenant, "Patient", &ids[1]).await.unwrap();
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 1);
}

// ============================================================================
// Delete Tests - Error Cases
// ============================================================================

/// Test that deleting a nonexistent resource fails.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_delete_nonexistent_fails() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let result = backend.delete(&tenant, "Patient", "nonexistent-id").await;

    assert!(result.is_err());
    match result {
        Err(StorageError::Resource(ResourceError::NotFound { .. })) => {}
        Err(e) => panic!("Expected NotFound error, got {:?}", e),
        Ok(_) => panic!("Expected error"),
    }
}

/// Test that deleting an already deleted resource fails.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_delete_already_deleted_fails() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let id = created.id().to_string();

    // Delete once
    backend.delete(&tenant, "Patient", &id).await.unwrap();

    // Try to delete again
    let result = backend.delete(&tenant, "Patient", &id).await;

    assert!(result.is_err());
    match result {
        Err(StorageError::Resource(ResourceError::NotFound { .. }))
        | Err(StorageError::Resource(ResourceError::Gone { .. })) => {}
        Err(e) => panic!("Expected NotFound or Gone error, got {:?}", e),
        Ok(_) => panic!("Expected error"),
    }
}

/// Test that deleting without permission fails.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_delete_without_permission_fails() {
    let backend = create_sqlite_backend();
    let full_access = TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    );
    let read_only =
        TenantContext::new(TenantId::new("test-tenant"), TenantPermissions::read_only());

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&full_access, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    let result = backend.delete(&read_only, "Patient", created.id()).await;

    assert!(result.is_err());
    match result {
        Err(StorageError::Tenant(_)) => {}
        Err(e) => panic!("Expected TenantError, got {:?}", e),
        Ok(_) => panic!("Expected error"),
    }
}

/// Test that deleting from wrong tenant fails.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_delete_wrong_tenant_fails() {
    let backend = create_sqlite_backend();

    let tenant1 = TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
    let tenant2 = TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant1, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    let result = backend.delete(&tenant2, "Patient", created.id()).await;

    assert!(result.is_err());
    match result {
        Err(StorageError::Resource(ResourceError::NotFound { .. })) => {}
        Err(e) => panic!("Expected NotFound error, got {:?}", e),
        Ok(_) => panic!("Expected error"),
    }
}

// ============================================================================
// Delete Tests - Tenant Isolation
// ============================================================================

/// Test that delete only affects the specified tenant.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_delete_tenant_isolation() {
    let backend = create_sqlite_backend();

    let tenant1 = TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
    let tenant2 = TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

    // Create same-id resource in both tenants using create_or_update
    let patient = create_patient_json("Smith");
    backend
        .create_or_update(
            &tenant1,
            "Patient",
            "shared-id",
            patient.clone(),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create_or_update(
            &tenant2,
            "Patient",
            "shared-id",
            patient,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Delete from tenant1
    backend
        .delete(&tenant1, "Patient", "shared-id")
        .await
        .unwrap();

    // Tenant1 should not have it
    assert!(
        !backend
            .exists(&tenant1, "Patient", "shared-id")
            .await
            .unwrap()
    );

    // Tenant2 should still have it
    assert!(
        backend
            .exists(&tenant2, "Patient", "shared-id")
            .await
            .unwrap()
    );
}

// ============================================================================
// Delete Tests - Soft Delete Behavior
// ============================================================================

/// Test that delete is a soft delete (resource can potentially be restored).
/// This test verifies the soft delete semantics described in the design.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_delete_is_soft_delete() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let id = created.id().to_string();

    backend.delete(&tenant, "Patient", &id).await.unwrap();

    // Normal read returns None
    let read = backend.read(&tenant, "Patient", &id).await.unwrap();
    assert!(read.is_none());

    // But create_or_update with same ID creates a new version
    let patient2 = create_patient_json("Restored");
    let (restored, _created_new) = backend
        .create_or_update(&tenant, "Patient", &id, patient2, FhirVersion::default())
        .await
        .unwrap();

    // This should create a new resource (after deletion)
    assert!(backend.exists(&tenant, "Patient", &id).await.unwrap());
    assert_eq!(restored.content()["name"][0]["family"], "Restored");
}

// ============================================================================
// Delete Tests - Multiple Deletes
// ============================================================================

/// Test deleting multiple resources.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_delete_multiple_resources() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create 5 resources
    let mut ids = Vec::new();
    for i in 0..5 {
        let patient = create_patient_json(&format!("Patient{}", i));
        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
        ids.push(created.id().to_string());
    }

    // Delete them all
    for id in &ids {
        backend.delete(&tenant, "Patient", id).await.unwrap();
    }

    // Verify all are deleted
    for id in &ids {
        assert!(!backend.exists(&tenant, "Patient", id).await.unwrap());
    }

    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 0);
}

/// Test deleting resources of different types.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_delete_different_types() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create various resources
    let patient = create_patient_json("Smith");
    let patient_created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    let observation = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {"coding": [{"code": "test"}]}
    });
    let obs_created = backend
        .create(&tenant, "Observation", observation, FhirVersion::default())
        .await
        .unwrap();

    // Delete patient
    backend
        .delete(&tenant, "Patient", patient_created.id())
        .await
        .unwrap();

    // Patient should be gone
    assert!(
        !backend
            .exists(&tenant, "Patient", patient_created.id())
            .await
            .unwrap()
    );

    // Observation should still exist
    assert!(
        backend
            .exists(&tenant, "Observation", obs_created.id())
            .await
            .unwrap()
    );
}

// ============================================================================
// Delete Tests - After Update
// ============================================================================

/// Test deleting a resource that has been updated.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_delete_after_update() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Update multiple times
    let mut current = created;
    for i in 1..=5 {
        let mut content = current.content().clone();
        content["name"][0]["family"] = json!(format!("Name{}", i));
        current = backend.update(&tenant, &current, content).await.unwrap();
    }

    // Now delete
    backend
        .delete(&tenant, "Patient", current.id())
        .await
        .unwrap();

    // Should be deleted
    assert!(
        !backend
            .exists(&tenant, "Patient", current.id())
            .await
            .unwrap()
    );
}

// ============================================================================
// Delete Tests - Idempotency Check
// ============================================================================

/// Verify delete is not idempotent (second delete fails).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_delete_not_idempotent() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let id = created.id().to_string();

    // First delete succeeds
    assert!(backend.delete(&tenant, "Patient", &id).await.is_ok());

    // Second delete fails
    assert!(backend.delete(&tenant, "Patient", &id).await.is_err());
}
