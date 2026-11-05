//! Tests for resource update operations.
//!
//! This module tests the `update` method of the ResourceStorage trait
//! and related versioning behavior.

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::ResourceStorage;
use helios_persistence::error::{ConcurrencyError, ResourceError, StorageError};
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
// Update Tests - Basic
// ============================================================================

/// Test that updating a resource succeeds and returns the updated resource.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_resource_success() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a resource
    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Update it
    let mut updated_content = created.content().clone();
    updated_content["name"][0]["family"] = json!("Jones");

    let updated = backend
        .update(&tenant, &created, updated_content)
        .await
        .unwrap();

    assert_eq!(updated.id(), created.id(), "ID should remain the same");
    assert_eq!(updated.version_id(), "2", "Version should be incremented");
    assert_eq!(updated.content()["name"][0]["family"], "Jones");
}

/// Test that update increments version correctly.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_increments_version() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let v1 = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    assert_eq!(v1.version_id(), "1");

    let v2 = backend
        .update(&tenant, &v1, v1.content().clone())
        .await
        .unwrap();
    assert_eq!(v2.version_id(), "2");

    let v3 = backend
        .update(&tenant, &v2, v2.content().clone())
        .await
        .unwrap();
    assert_eq!(v3.version_id(), "3");

    let v4 = backend
        .update(&tenant, &v3, v3.content().clone())
        .await
        .unwrap();
    assert_eq!(v4.version_id(), "4");
}

/// Test that update updates the last_modified timestamp.
#[cfg(feature = "sqlite")]
#[tokio::test]
#[ignore = "#306: SQLite has no created_at column and approximates it with last_modified (backends/sqlite/storage.rs), so created_at cannot survive an update — see PR #361"]
async fn test_update_updates_timestamp() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let created_time = created.last_modified();

    // Small delay to ensure timestamp difference
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let updated = backend
        .update(&tenant, &created, created.content().clone())
        .await
        .unwrap();

    assert!(
        updated.last_modified() >= created_time,
        "last_modified should be updated"
    );
    assert_eq!(
        updated.created_at(),
        created.created_at(),
        "created_at should be preserved"
    );
}

/// Test that update generates new ETag.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_generates_new_etag() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    let updated = backend
        .update(&tenant, &created, created.content().clone())
        .await
        .unwrap();

    assert_ne!(
        updated.etag(),
        created.etag(),
        "ETag should change after update"
    );
    assert!(
        updated.etag().contains("2"),
        "ETag should contain version 2"
    );
}

// ============================================================================
// Update Tests - Content Changes
// ============================================================================

/// Test updating multiple fields at once.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_multiple_fields() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = json!({
        "resourceType": "Patient",
        "name": [{"family": "Smith", "given": ["John"]}],
        "active": true,
        "gender": "male"
    });
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    let mut updated_content = created.content().clone();
    updated_content["name"][0]["family"] = json!("Jones");
    updated_content["active"] = json!(false);
    updated_content["gender"] = json!("unknown");

    let updated = backend
        .update(&tenant, &created, updated_content)
        .await
        .unwrap();

    assert_eq!(updated.content()["name"][0]["family"], "Jones");
    assert_eq!(updated.content()["active"], false);
    assert_eq!(updated.content()["gender"], "unknown");
}

/// Test adding new fields during update.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_add_new_fields() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = json!({
        "resourceType": "Patient",
        "name": [{"family": "Smith"}]
    });
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    let mut updated_content = created.content().clone();
    updated_content["birthDate"] = json!("1980-01-15");
    updated_content["gender"] = json!("male");
    updated_content["address"] = json!([{"city": "Boston"}]);

    let updated = backend
        .update(&tenant, &created, updated_content)
        .await
        .unwrap();

    assert_eq!(updated.content()["birthDate"], "1980-01-15");
    assert_eq!(updated.content()["gender"], "male");
    assert_eq!(updated.content()["address"][0]["city"], "Boston");
}

/// Test removing fields during update.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_remove_fields() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = json!({
        "resourceType": "Patient",
        "name": [{"family": "Smith"}],
        "birthDate": "1980-01-15",
        "gender": "male"
    });
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Create content without birthDate and gender
    let updated_content = json!({
        "resourceType": "Patient",
        "name": [{"family": "Smith"}]
    });

    let updated = backend
        .update(&tenant, &created, updated_content)
        .await
        .unwrap();

    assert!(
        updated.content().get("birthDate").is_none() || updated.content()["birthDate"].is_null(),
        "birthDate should be removed or null"
    );
}

/// Test updating nested objects.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_nested_objects() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let observation = json!({
        "resourceType": "Observation",
        "status": "preliminary",
        "code": {
            "coding": [{
                "system": "http://loinc.org",
                "code": "8867-4",
                "display": "Heart rate"
            }]
        },
        "valueQuantity": {
            "value": 70,
            "unit": "bpm"
        }
    });
    let created = backend
        .create(&tenant, "Observation", observation, FhirVersion::default())
        .await
        .unwrap();

    let mut updated_content = created.content().clone();
    updated_content["status"] = json!("final");
    updated_content["valueQuantity"]["value"] = json!(75);

    let updated = backend
        .update(&tenant, &created, updated_content)
        .await
        .unwrap();

    assert_eq!(updated.content()["status"], "final");
    assert_eq!(updated.content()["valueQuantity"]["value"], 75);
}

// ============================================================================
// Update Tests - Error Cases
// ============================================================================

/// Test that updating a nonexistent resource fails.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_nonexistent_fails() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a fake stored resource
    let fake_resource = helios_persistence::types::StoredResource::new(
        "Patient",
        "nonexistent-id",
        tenant.tenant_id().clone(),
        json!({"resourceType": "Patient"}),
        FhirVersion::default(),
    );

    let result = backend
        .update(&tenant, &fake_resource, json!({"resourceType": "Patient"}))
        .await;

    assert!(result.is_err());
    match result {
        Err(StorageError::Resource(ResourceError::NotFound { .. })) => {}
        Err(e) => panic!("Expected NotFound error, got {:?}", e),
        Ok(_) => panic!("Expected error"),
    }
}

/// Test that updating without permission fails.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_without_permission_fails() {
    let backend = create_sqlite_backend();
    let full_access = TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    );
    let read_only =
        TenantContext::new(TenantId::new("test-tenant"), TenantPermissions::read_only());

    // Create with full access
    let patient = create_patient_json("Smith");
    let created = backend
        .create(&full_access, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Try to update with read-only
    let result = backend
        .update(&read_only, &created, created.content().clone())
        .await;

    assert!(result.is_err());
    match result {
        Err(StorageError::Tenant(_)) => {}
        Err(e) => panic!("Expected TenantError, got {:?}", e),
        Ok(_) => panic!("Expected error"),
    }
}

/// Test that updating a deleted resource fails.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_deleted_fails() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create and delete a resource
    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    backend
        .delete(&tenant, "Patient", created.id())
        .await
        .unwrap();

    // Try to update the deleted resource
    let result = backend
        .update(&tenant, &created, created.content().clone())
        .await;

    assert!(result.is_err());
    // Could be NotFound or Gone depending on implementation
    match result {
        Err(StorageError::Resource(ResourceError::NotFound { .. }))
        | Err(StorageError::Resource(ResourceError::Gone { .. })) => {}
        Err(e) => panic!("Expected NotFound or Gone error, got {:?}", e),
        Ok(_) => panic!("Expected error"),
    }
}

/// Test updating from wrong tenant fails.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_wrong_tenant_fails() {
    let backend = create_sqlite_backend();

    let tenant1 = TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
    let tenant2 = TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

    // Create in tenant1
    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant1, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Try to update from tenant2
    let result = backend
        .update(&tenant2, &created, created.content().clone())
        .await;

    assert!(result.is_err());
}

// ============================================================================
// Update Tests - Version Conflicts
// ============================================================================

/// Test that concurrent updates with stale version fail (optimistic locking).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_version_conflict() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a resource
    let patient = create_patient_json("Smith");
    let v1 = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Do a legitimate update
    let mut content2 = v1.content().clone();
    content2["name"][0]["family"] = json!("Jones");
    let _v2 = backend.update(&tenant, &v1, content2).await.unwrap();

    // Try to update again using stale v1
    let mut content3 = v1.content().clone();
    content3["name"][0]["family"] = json!("Williams");
    let result = backend.update(&tenant, &v1, content3).await;

    // Should fail due to version conflict
    assert!(result.is_err());
    match result {
        Err(StorageError::Concurrency(ConcurrencyError::VersionConflict { .. })) => {}
        Err(StorageError::Resource(ResourceError::NotFound { .. })) => {
            // Some implementations may return NotFound for stale versions
        }
        Err(e) => panic!("Expected VersionConflict or NotFound error, got {:?}", e),
        Ok(_) => panic!("Expected error"),
    }
}

// ============================================================================
// Update Tests - Sequential Updates
// ============================================================================

/// Test that sequential updates work correctly.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_sequential_updates() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Name0");
    let mut current = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    for i in 1..=10 {
        let mut content = current.content().clone();
        content["name"][0]["family"] = json!(format!("Name{}", i));
        current = backend.update(&tenant, &current, content).await.unwrap();
        assert_eq!(current.version_id(), (i + 1).to_string());
    }

    // Verify final state
    let read = backend
        .read(&tenant, "Patient", current.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.version_id(), "11");
    assert_eq!(read.content()["name"][0]["family"], "Name10");
}

// ============================================================================
// Update Tests - Idempotency
// ============================================================================

/// Test that updating with same content still increments version.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_same_content_increments_version() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Update with identical content
    let updated = backend
        .update(&tenant, &created, created.content().clone())
        .await
        .unwrap();

    // Version should still increment (this is standard FHIR behavior)
    assert_eq!(updated.version_id(), "2");
}

// ============================================================================
// Update Tests - Special Content
// ============================================================================

/// Test updating with Unicode content.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_with_unicode() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    let mut updated_content = created.content().clone();
    updated_content["name"][0]["family"] = json!("日本語");
    updated_content["name"][0]["given"] = json!(["太郎"]);

    let updated = backend
        .update(&tenant, &created, updated_content)
        .await
        .unwrap();

    assert_eq!(updated.content()["name"][0]["family"], "日本語");
    assert_eq!(updated.content()["name"][0]["given"][0], "太郎");
}

/// Test updating with large content.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_with_large_content() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Create large update with many names
    let mut names = Vec::new();
    for i in 0..100 {
        names.push(json!({
            "family": format!("Family{}", i),
            "given": [format!("Given{}", i)]
        }));
    }

    let updated_content = json!({
        "resourceType": "Patient",
        "name": names
    });

    let updated = backend
        .update(&tenant, &created, updated_content)
        .await
        .unwrap();

    assert_eq!(updated.content()["name"].as_array().unwrap().len(), 100);
}
