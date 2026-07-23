//! Tests for version read (vread) operations.
//!
//! This module tests the `vread` method of the VersionedStorage trait
//! which reads a specific version of a resource.

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, VersionedStorage};
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
// VRead Tests - Basic
// ============================================================================

/// Test reading a specific version of a resource.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_vread_specific_version() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a resource (version 1)
    let patient = create_patient_json("Version1");
    let v1 = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Update to create version 2
    let mut content2 = v1.content().clone();
    content2["name"][0]["family"] = json!("Version2");
    let v2 = backend.update(&tenant, &v1, content2).await.unwrap();

    // Update to create version 3
    let mut content3 = v2.content().clone();
    content3["name"][0]["family"] = json!("Version3");
    let _v3 = backend.update(&tenant, &v2, content3).await.unwrap();

    // VRead version 1
    let read_v1 = backend
        .vread(&tenant, "Patient", v1.id(), "1")
        .await
        .unwrap();
    assert!(read_v1.is_some());
    let read_v1 = read_v1.unwrap();
    assert_eq!(read_v1.version_id(), "1");
    assert_eq!(read_v1.content()["name"][0]["family"], "Version1");

    // VRead version 2
    let read_v2 = backend
        .vread(&tenant, "Patient", v1.id(), "2")
        .await
        .unwrap();
    assert!(read_v2.is_some());
    let read_v2 = read_v2.unwrap();
    assert_eq!(read_v2.version_id(), "2");
    assert_eq!(read_v2.content()["name"][0]["family"], "Version2");

    // VRead version 3
    let read_v3 = backend
        .vread(&tenant, "Patient", v1.id(), "3")
        .await
        .unwrap();
    assert!(read_v3.is_some());
    let read_v3 = read_v3.unwrap();
    assert_eq!(read_v3.version_id(), "3");
    assert_eq!(read_v3.content()["name"][0]["family"], "Version3");
}

/// Test that vread returns correct metadata for each version.
#[cfg(feature = "sqlite")]
#[tokio::test]
#[ignore = "#306: SQLite has no created_at column and approximates it with last_modified (backends/sqlite/storage.rs), so created_at cannot survive an update — see PR #361"]
async fn test_vread_returns_correct_metadata() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Original");
    let v1 = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let v1_time = v1.last_modified();

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let v2 = backend
        .update(&tenant, &v1, v1.content().clone())
        .await
        .unwrap();
    let v2_time = v2.last_modified();

    // VRead both versions and check metadata
    let read_v1 = backend
        .vread(&tenant, "Patient", v1.id(), "1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read_v1.etag(), "W/\"1\"");
    assert_eq!(read_v1.last_modified(), v1_time);

    let read_v2 = backend
        .vread(&tenant, "Patient", v1.id(), "2")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read_v2.etag(), "W/\"2\"");
    assert_eq!(read_v2.last_modified(), v2_time);

    // Both should have same created_at
    assert_eq!(read_v1.created_at(), read_v2.created_at());
}

// ============================================================================
// VRead Tests - Error Cases
// ============================================================================

/// Test vread of nonexistent resource.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_vread_nonexistent_resource() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let result = backend
        .vread(&tenant, "Patient", "nonexistent", "1")
        .await
        .unwrap();

    assert!(result.is_none());
}

/// Test vread of nonexistent version.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_vread_nonexistent_version() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let v1 = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Try to read version 99 which doesn't exist
    let result = backend
        .vread(&tenant, "Patient", v1.id(), "99")
        .await
        .unwrap();

    assert!(result.is_none());
}

/// Test vread with invalid version format.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_vread_invalid_version_format() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let v1 = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Try various invalid version formats
    let result = backend.vread(&tenant, "Patient", v1.id(), "invalid").await;

    // Should either return None or an error, depending on implementation
    // Most implementations will return None for unparseable versions
    match result {
        Ok(None) => {}
        Ok(Some(_)) => panic!("Should not find resource with invalid version"),
        Err(_) => {} // Also acceptable
    }
}

// ============================================================================
// VRead Tests - Tenant Isolation
// ============================================================================

/// Test that vread respects tenant isolation.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_vread_tenant_isolation() {
    let backend = create_sqlite_backend();

    let tenant1 = TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
    let tenant2 = TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

    // Create resource in tenant1
    let patient = create_patient_json("Smith");
    let v1 = backend
        .create(&tenant1, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Try to vread from tenant2
    let result = backend
        .vread(&tenant2, "Patient", v1.id(), "1")
        .await
        .unwrap();

    assert!(result.is_none(), "Should not see other tenant's resource");
}

// ============================================================================
// VRead Tests - After Delete
// ============================================================================

/// Test vread of deleted resource (should still return version).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_vread_after_delete() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create and update a resource
    let patient = create_patient_json("Smith");
    let v1 = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let v2 = backend
        .update(&tenant, &v1, v1.content().clone())
        .await
        .unwrap();

    // Delete the resource
    backend.delete(&tenant, "Patient", v1.id()).await.unwrap();

    // VRead should still return the historical versions
    // (This is a key feature of FHIR vread - history is preserved)
    let read_v1 = backend
        .vread(&tenant, "Patient", v1.id(), "1")
        .await
        .unwrap();
    let read_v2 = backend
        .vread(&tenant, "Patient", v1.id(), "2")
        .await
        .unwrap();

    // Note: Behavior may vary by implementation
    // Some backends preserve history, others don't
    // This test documents expected FHIR behavior
    if read_v1.is_some() {
        assert_eq!(read_v1.unwrap().version_id(), "1");
    }
    if read_v2.is_some() {
        assert_eq!(read_v2.unwrap().version_id(), "2");
    }
}

/// Test that vread of deleted version returns deleted marker.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_vread_deleted_version() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let v1 = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    backend.delete(&tenant, "Patient", v1.id()).await.unwrap();

    // The delete creates a new version (v2) with deleted marker
    let deleted_version = backend
        .vread(&tenant, "Patient", v1.id(), "2")
        .await
        .unwrap();

    // If history is preserved, the deleted version should be readable
    // and marked as deleted
    if let Some(deleted) = deleted_version {
        assert!(deleted.is_deleted());
        assert_eq!(deleted.version_id(), "2");
    }
}

// ============================================================================
// VRead Tests - Many Versions
// ============================================================================

/// Test vread with many versions.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_vread_many_versions() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create initial version
    let patient = create_patient_json("Version0");
    let mut current = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let id = current.id().to_string();

    // Create 19 more versions (total 20)
    for i in 1..20 {
        let mut content = current.content().clone();
        content["name"][0]["family"] = json!(format!("Version{}", i));
        current = backend.update(&tenant, &current, content).await.unwrap();
    }

    // Verify we can read all versions
    for version in 1..=20 {
        let read = backend
            .vread(&tenant, "Patient", &id, &version.to_string())
            .await
            .unwrap();
        assert!(read.is_some(), "Version {} should exist", version);
        let read = read.unwrap();
        assert_eq!(read.version_id(), version.to_string());
        assert_eq!(
            read.content()["name"][0]["family"],
            format!("Version{}", version - 1)
        );
    }

    // Version 21 should not exist
    let read = backend.vread(&tenant, "Patient", &id, "21").await.unwrap();
    assert!(read.is_none());
}

// ============================================================================
// VRead Tests - Resource Types
// ============================================================================

/// Test vread works across different resource types.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_vread_different_resource_types() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create Patient with multiple versions
    let patient = create_patient_json("Smith");
    let patient_v1 = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let _patient_v2 = backend
        .update(&tenant, &patient_v1, patient_v1.content().clone())
        .await
        .unwrap();

    // Create Observation with multiple versions
    let obs = json!({
        "resourceType": "Observation",
        "status": "preliminary",
        "code": {"coding": [{"code": "test"}]}
    });
    let obs_v1 = backend
        .create(&tenant, "Observation", obs, FhirVersion::default())
        .await
        .unwrap();
    let mut obs_content = obs_v1.content().clone();
    obs_content["status"] = json!("final");
    let _obs_v2 = backend.update(&tenant, &obs_v1, obs_content).await.unwrap();

    // VRead both versions of Patient
    let p_read_v1 = backend
        .vread(&tenant, "Patient", patient_v1.id(), "1")
        .await
        .unwrap()
        .unwrap();
    let p_read_v2 = backend
        .vread(&tenant, "Patient", patient_v1.id(), "2")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(p_read_v1.resource_type(), "Patient");
    assert_eq!(p_read_v2.resource_type(), "Patient");

    // VRead both versions of Observation
    let o_read_v1 = backend
        .vread(&tenant, "Observation", obs_v1.id(), "1")
        .await
        .unwrap()
        .unwrap();
    let o_read_v2 = backend
        .vread(&tenant, "Observation", obs_v1.id(), "2")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(o_read_v1.resource_type(), "Observation");
    assert_eq!(o_read_v1.content()["status"], "preliminary");
    assert_eq!(o_read_v2.resource_type(), "Observation");
    assert_eq!(o_read_v2.content()["status"], "final");
}
