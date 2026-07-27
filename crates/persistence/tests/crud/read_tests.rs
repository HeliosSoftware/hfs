//! Tests for resource read operations.
//!
//! This module tests the `read`, `exists`, `read_batch`, and `count` methods
//! of the ResourceStorage trait.

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
// Read Tests - Basic
// ============================================================================

/// Test reading a resource that exists.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_read_existing_resource() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a resource first
    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Read it back
    let result = backend
        .read(&tenant, "Patient", created.id())
        .await
        .unwrap();

    assert!(result.is_some());
    let read = result.unwrap();
    assert_eq!(read.id(), created.id());
    assert_eq!(read.version_id(), created.version_id());
    assert_eq!(read.resource_type(), "Patient");
}

/// Test that reading returns the correct content.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_read_returns_correct_content() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = json!({
        "resourceType": "Patient",
        "name": [{"family": "Smith", "given": ["John"]}],
        "birthDate": "1980-01-15"
    });
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    let read = backend
        .read(&tenant, "Patient", created.id())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(read.content()["name"][0]["family"], "Smith");
    assert_eq!(read.content()["name"][0]["given"][0], "John");
    assert_eq!(read.content()["birthDate"], "1980-01-15");
}

/// Test reading a resource that doesn't exist.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_read_nonexistent_resource() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let result = backend
        .read(&tenant, "Patient", "nonexistent-id")
        .await
        .unwrap();

    assert!(
        result.is_none(),
        "Reading nonexistent resource should return None"
    );
}

/// Test reading returns correct metadata.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_read_returns_metadata() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    let read = backend
        .read(&tenant, "Patient", created.id())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(read.etag(), created.etag());
    assert_eq!(read.created_at(), created.created_at());
    assert_eq!(read.last_modified(), created.last_modified());
    assert_eq!(read.tenant_id(), created.tenant_id());
}

// ============================================================================
// Read Tests - Tenant Isolation
// ============================================================================

/// Test that reading only returns resources from the correct tenant.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_read_tenant_isolation() {
    let backend = create_sqlite_backend();

    let tenant1 = TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
    let tenant2 = TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

    // Create resource in tenant1
    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant1, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Try to read from tenant2 - should not find it
    let result = backend
        .read(&tenant2, "Patient", created.id())
        .await
        .unwrap();

    assert!(
        result.is_none(),
        "Should not be able to read another tenant's resource"
    );
}

/// Test that system tenant can be used for shared resources.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_read_system_tenant() {
    let backend = create_sqlite_backend();
    let system = TenantContext::system();

    // Create a shared resource in system tenant
    let value_set = json!({
        "resourceType": "ValueSet",
        "name": "TestValueSet"
    });
    let created = backend
        .create(&system, "ValueSet", value_set, FhirVersion::default())
        .await
        .unwrap();

    // Read from system tenant
    let read = backend
        .read(&system, "ValueSet", created.id())
        .await
        .unwrap();

    assert!(read.is_some());
    assert!(read.unwrap().tenant_id().is_system());
}

// ============================================================================
// Exists Tests
// ============================================================================

/// Test exists returns true for existing resources.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_exists_returns_true() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    let exists = backend
        .exists(&tenant, "Patient", created.id())
        .await
        .unwrap();

    assert!(exists, "exists should return true for existing resource");
}

/// Test exists returns false for nonexistent resources.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_exists_returns_false() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let exists = backend
        .exists(&tenant, "Patient", "nonexistent-id")
        .await
        .unwrap();

    assert!(
        !exists,
        "exists should return false for nonexistent resource"
    );
}

/// Test exists respects tenant isolation.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_exists_tenant_isolation() {
    let backend = create_sqlite_backend();

    let tenant1 = TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
    let tenant2 = TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant1, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Check from different tenant
    let exists = backend
        .exists(&tenant2, "Patient", created.id())
        .await
        .unwrap();

    assert!(
        !exists,
        "exists should return false for other tenant's resource"
    );
}

// ============================================================================
// Read Batch Tests
// ============================================================================

/// Test reading multiple resources at once.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_read_batch_success() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create multiple resources
    let mut ids = Vec::new();
    for i in 0..5 {
        let patient = create_patient_json(&format!("Patient{}", i));
        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
        ids.push(created.id().to_string());
    }

    // Read them all
    let id_refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
    let results = backend
        .read_batch(&tenant, "Patient", &id_refs)
        .await
        .unwrap();

    assert_eq!(results.len(), 5, "Should return all 5 resources");
}

/// Test read_batch returns only existing resources.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_read_batch_partial() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create 2 resources
    let patient1 = create_patient_json("Smith");
    let created1 = backend
        .create(&tenant, "Patient", patient1, FhirVersion::default())
        .await
        .unwrap();
    let patient2 = create_patient_json("Jones");
    let created2 = backend
        .create(&tenant, "Patient", patient2, FhirVersion::default())
        .await
        .unwrap();

    // Read with some nonexistent IDs
    let ids = vec![
        created1.id(),
        "nonexistent-1",
        created2.id(),
        "nonexistent-2",
    ];
    let results = backend.read_batch(&tenant, "Patient", &ids).await.unwrap();

    assert_eq!(results.len(), 2, "Should return only existing resources");
}

/// Test read_batch with empty list.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_read_batch_empty() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let results: Vec<helios_persistence::types::StoredResource> =
        backend.read_batch(&tenant, "Patient", &[]).await.unwrap();

    assert!(
        results.is_empty(),
        "Empty input should return empty results"
    );
}

/// Test read_batch respects tenant isolation.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_read_batch_tenant_isolation() {
    let backend = create_sqlite_backend();

    let tenant1 = TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
    let tenant2 = TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

    // Create resources in tenant1
    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant1, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Try to read from tenant2
    let results = backend
        .read_batch(&tenant2, "Patient", &[created.id()])
        .await
        .unwrap();

    assert!(
        results.is_empty(),
        "Should not find other tenant's resources"
    );
}

// ============================================================================
// Count Tests
// ============================================================================

/// Test counting resources by type.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_count_by_type() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create some patients
    for i in 0..3 {
        let patient = create_patient_json(&format!("Patient{}", i));
        backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }

    // Create some observations
    for i in 0..2 {
        let obs = json!({
            "resourceType": "Observation",
            "status": "final",
            "code": {"coding": [{"code": format!("code-{}", i)}]}
        });
        backend
            .create(&tenant, "Observation", obs, FhirVersion::default())
            .await
            .unwrap();
    }

    let patient_count = backend.count(&tenant, Some("Patient")).await.unwrap();
    let obs_count = backend.count(&tenant, Some("Observation")).await.unwrap();

    assert_eq!(patient_count, 3);
    assert_eq!(obs_count, 2);
}

/// Test counting all resources.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_count_all() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create mixed resources
    for i in 0..3 {
        let patient = create_patient_json(&format!("Patient{}", i));
        backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }
    for i in 0..2 {
        let obs = json!({
            "resourceType": "Observation",
            "status": "final",
            "code": {"coding": [{"code": format!("code-{}", i)}]}
        });
        backend
            .create(&tenant, "Observation", obs, FhirVersion::default())
            .await
            .unwrap();
    }

    let total = backend.count(&tenant, None).await.unwrap();
    assert_eq!(total, 5);
}

/// Test count returns zero for empty storage.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_count_empty() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let count = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count, 0);
}

/// Test count respects tenant isolation.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_count_tenant_isolation() {
    let backend = create_sqlite_backend();

    let tenant1 = TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
    let tenant2 = TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

    // Create resources in tenant1
    for i in 0..5 {
        let patient = create_patient_json(&format!("Patient{}", i));
        backend
            .create(&tenant1, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }

    // Create resources in tenant2
    for i in 0..3 {
        let patient = create_patient_json(&format!("Patient{}", i));
        backend
            .create(&tenant2, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }

    let count1 = backend.count(&tenant1, Some("Patient")).await.unwrap();
    let count2 = backend.count(&tenant2, Some("Patient")).await.unwrap();

    assert_eq!(count1, 5);
    assert_eq!(count2, 3);
}

// ============================================================================
// Read Tests - After Update
// ============================================================================

/// Test that read returns the latest version after update.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_read_after_update() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create resource
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

    // Read it
    let read = backend
        .read(&tenant, "Patient", created.id())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(read.version_id(), "2");
    assert_eq!(read.content()["name"][0]["family"], "Jones");
    assert_eq!(read.version_id(), updated.version_id());
}

// ============================================================================
// Read Tests - After Delete
// ============================================================================

/// Test that read returns None after delete.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_read_after_delete() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create resource
    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Delete it
    backend
        .delete(&tenant, "Patient", created.id())
        .await
        .unwrap();

    // Soft delete: `read` surfaces a deleted resource as `Gone` (SQLite/PG/S3)
    // or as `None`; either satisfies "not readable". Mirrors sqlite_tests.rs.
    match backend.read(&tenant, "Patient", created.id()).await {
        Ok(None) => {}
        Err(StorageError::Resource(ResourceError::Gone { .. })) => {}
        other => panic!(
            "Read after delete should not return a resource, got: {:?}",
            other
        ),
    }
}

/// Test that exists returns false after delete.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_exists_after_delete() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create resource
    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Delete it
    backend
        .delete(&tenant, "Patient", created.id())
        .await
        .unwrap();

    // Check existence
    let exists = backend
        .exists(&tenant, "Patient", created.id())
        .await
        .unwrap();

    assert!(!exists, "exists after delete should return false");
}

/// Test that count excludes deleted resources.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_count_excludes_deleted() {
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

    // Initial count
    let initial_count = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(initial_count, 3);

    // Delete one
    backend.delete(&tenant, "Patient", &ids[0]).await.unwrap();

    // Count should decrease
    let final_count = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(final_count, 2);
}

// ============================================================================
// Read Tests - Permissions
// ============================================================================

/// Test that read with read-only permissions succeeds.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_read_with_read_only_permissions() {
    let backend = create_sqlite_backend();
    let full_access = TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    );
    let read_only =
        TenantContext::new(TenantId::new("test-tenant"), TenantPermissions::read_only());

    // Create resource with full access
    let patient = create_patient_json("Smith");
    let created = backend
        .create(&full_access, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Read with read-only permissions
    let read = backend
        .read(&read_only, "Patient", created.id())
        .await
        .unwrap();

    assert!(
        read.is_some(),
        "Should be able to read with read-only permissions"
    );
}
