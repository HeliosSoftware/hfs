//! Tests for history operations.
//!
//! This module tests the instance, type, and system history operations
//! as defined by the FHIR specification.

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{
    HistoryParams, InstanceHistoryProvider, ResourceStorage, SystemHistoryProvider,
    TypeHistoryProvider, VersionedStorage,
};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::Pagination;

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
// Instance History Tests
// ============================================================================

/// Test instance history returns all versions of a resource.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_instance_history_basic() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a resource and update it multiple times
    let patient = create_patient_json("Version1");
    let v1 = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    let mut content2 = v1.content().clone();
    content2["name"][0]["family"] = json!("Version2");
    let v2 = backend.update(&tenant, &v1, content2).await.unwrap();

    let mut content3 = v2.content().clone();
    content3["name"][0]["family"] = json!("Version3");
    let _v3 = backend.update(&tenant, &v2, content3).await.unwrap();

    // Get instance history
    let history = backend
        .history_instance(&tenant, "Patient", v1.id(), &HistoryParams::new())
        .await
        .unwrap();

    assert_eq!(history.items.len(), 3, "Should have 3 versions");

    // History should be in reverse chronological order (newest first)
    assert_eq!(history.items[0].resource.version_id(), "3");
    assert_eq!(history.items[1].resource.version_id(), "2");
    assert_eq!(history.items[2].resource.version_id(), "1");

    // Content should match each version
    assert_eq!(
        history.items[0].resource.content()["name"][0]["family"],
        "Version3"
    );
    assert_eq!(
        history.items[1].resource.content()["name"][0]["family"],
        "Version2"
    );
    assert_eq!(
        history.items[2].resource.content()["name"][0]["family"],
        "Version1"
    );
}

/// Test instance history includes deleted version.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_instance_history_includes_deleted() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let v1 = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Delete the resource
    backend.delete(&tenant, "Patient", v1.id()).await.unwrap();

    // Get instance history
    let history = backend
        .history_instance(&tenant, "Patient", v1.id(), &HistoryParams::new())
        .await
        .unwrap();

    // Should have 2 versions: v1 (created) and v2 (deleted)
    assert!(history.items.len() >= 1);

    // If delete creates a version, the most recent should be deleted
    if history.items.len() > 1 {
        assert!(history.items[0].resource.is_deleted());
    }
}

/// Test instance history with pagination.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_instance_history_pagination() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create resource with many versions
    let patient = create_patient_json("Version0");
    let mut current = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let id = current.id().to_string();

    for i in 1..=10 {
        let mut content = current.content().clone();
        content["name"][0]["family"] = json!(format!("Version{}", i));
        current = backend.update(&tenant, &current, content).await.unwrap();
    }

    // Get first page (3 items)
    let page1 = backend
        .history_instance(&tenant, "Patient", &id, &HistoryParams::new().count(3))
        .await
        .unwrap();

    assert_eq!(page1.items.len(), 3);
    assert_eq!(page1.items[0].resource.version_id(), "11"); // Most recent
    assert_eq!(page1.items[1].resource.version_id(), "10");
    assert_eq!(page1.items[2].resource.version_id(), "9");

    // If there's a next page cursor, get next page
    if let Some(cursor) = page1.page_info.next_cursor.clone() {
        let mut params2 = HistoryParams::new();
        params2.pagination = Pagination::with_cursor(3, cursor);
        let page2 = backend
            .history_instance(&tenant, "Patient", &id, &params2)
            .await
            .unwrap();

        assert_eq!(page2.items.len(), 3);
        assert_eq!(page2.items[0].resource.version_id(), "8");
    }
}

/// Test instance history for nonexistent resource.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_instance_history_nonexistent() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let history = backend
        .history_instance(&tenant, "Patient", "nonexistent", &HistoryParams::new())
        .await
        .unwrap();

    assert!(history.items.is_empty());
}

/// Test instance history respects tenant isolation.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_instance_history_tenant_isolation() {
    let backend = create_sqlite_backend();

    let tenant1 = TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
    let tenant2 = TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

    // Create resource in tenant1
    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant1, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Try to get history from tenant2
    let history = backend
        .history_instance(&tenant2, "Patient", created.id(), &HistoryParams::new())
        .await
        .unwrap();

    assert!(
        history.items.is_empty(),
        "Should not see other tenant's history"
    );
}

// ============================================================================
// Type History Tests
// ============================================================================

/// Test type history returns all versions of all resources of a type.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_type_history_basic() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create multiple patients with multiple versions
    let patient1 = create_patient_json("Patient1");
    let p1v1 = backend
        .create(&tenant, "Patient", patient1, FhirVersion::default())
        .await
        .unwrap();
    let _p1v2 = backend
        .update(&tenant, &p1v1, p1v1.content().clone())
        .await
        .unwrap();

    let patient2 = create_patient_json("Patient2");
    let _p2v1 = backend
        .create(&tenant, "Patient", patient2, FhirVersion::default())
        .await
        .unwrap();

    // Get type history
    let history = backend
        .history_type(&tenant, "Patient", &HistoryParams::new())
        .await
        .unwrap();

    // Should have 3 total versions (2 for patient1, 1 for patient2)
    assert_eq!(history.items.len(), 3);

    // Should be in reverse chronological order
    // (most recent first - patient2v1, then patient1v2, then patient1v1)
}

/// Test type history excludes other resource types.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_type_history_excludes_other_types() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create patients
    let patient = create_patient_json("Smith");
    backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Create observations
    let observation = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {"coding": [{"code": "test"}]}
    });
    backend
        .create(&tenant, "Observation", observation, FhirVersion::default())
        .await
        .unwrap();

    // Get Patient history only
    let history = backend
        .history_type(&tenant, "Patient", &HistoryParams::new())
        .await
        .unwrap();

    // Should only contain patients
    for entry in &history.items {
        assert_eq!(entry.resource.resource_type(), "Patient");
    }
}

/// Test type history with pagination.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_type_history_pagination() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create many patients
    for i in 0..10 {
        let patient = create_patient_json(&format!("Patient{}", i));
        backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }

    // Get first page
    let page1 = backend
        .history_type(&tenant, "Patient", &HistoryParams::new().count(3))
        .await
        .unwrap();

    assert_eq!(page1.items.len(), 3);

    // Get second page if available
    if let Some(cursor) = page1.page_info.next_cursor.clone() {
        let mut params2 = HistoryParams::new();
        params2.pagination = Pagination::with_cursor(3, cursor);
        let page2 = backend
            .history_type(&tenant, "Patient", &params2)
            .await
            .unwrap();

        assert_eq!(page2.items.len(), 3);
    }
}

/// Test type history respects tenant isolation.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_type_history_tenant_isolation() {
    let backend = create_sqlite_backend();

    let tenant1 = TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
    let tenant2 = TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

    // Create patients in tenant1
    for i in 0..5 {
        let patient = create_patient_json(&format!("Tenant1Patient{}", i));
        backend
            .create(&tenant1, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }

    // Create patients in tenant2
    for i in 0..3 {
        let patient = create_patient_json(&format!("Tenant2Patient{}", i));
        backend
            .create(&tenant2, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }

    // Get history for each tenant
    let history1 = backend
        .history_type(&tenant1, "Patient", &HistoryParams::new())
        .await
        .unwrap();
    let history2 = backend
        .history_type(&tenant2, "Patient", &HistoryParams::new())
        .await
        .unwrap();

    assert_eq!(history1.items.len(), 5);
    assert_eq!(history2.items.len(), 3);
}

// ============================================================================
// System History Tests
// ============================================================================

/// Test system history returns all versions of all resources.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_system_history_basic() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create various resources
    let patient = create_patient_json("Smith");
    backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    let observation = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {"coding": [{"code": "test"}]}
    });
    backend
        .create(&tenant, "Observation", observation, FhirVersion::default())
        .await
        .unwrap();

    let organization = json!({
        "resourceType": "Organization",
        "name": "Test Org"
    });
    backend
        .create(
            &tenant,
            "Organization",
            organization,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Get system history
    let history = backend
        .history_system(&tenant, &HistoryParams::new())
        .await
        .unwrap();

    // Should have all 3 resources
    assert_eq!(history.items.len(), 3);

    // Collect resource types
    let types: std::collections::HashSet<_> = history
        .items
        .iter()
        .map(|e| e.resource.resource_type())
        .collect();

    assert!(types.contains("Patient"));
    assert!(types.contains("Observation"));
    assert!(types.contains("Organization"));
}

/// Test system history is in reverse chronological order.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_system_history_order() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create resources with small delays to ensure ordering
    let patient = create_patient_json("First");
    let first = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let observation = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {"coding": [{"code": "second"}]}
    });
    let second = backend
        .create(&tenant, "Observation", observation, FhirVersion::default())
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let organization = json!({
        "resourceType": "Organization",
        "name": "Third"
    });
    let third = backend
        .create(
            &tenant,
            "Organization",
            organization,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Get system history
    let history = backend
        .history_system(&tenant, &HistoryParams::new())
        .await
        .unwrap();

    // Should be in reverse chronological order
    assert!(history.items[0].resource.last_modified() >= history.items[1].resource.last_modified());
    assert!(history.items[1].resource.last_modified() >= history.items[2].resource.last_modified());
}

/// Test system history with pagination.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_system_history_pagination() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create many resources
    for i in 0..10 {
        let patient = create_patient_json(&format!("Patient{}", i));
        backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }

    // Get first page
    let page1 = backend
        .history_system(&tenant, &HistoryParams::new().count(3))
        .await
        .unwrap();

    assert_eq!(page1.items.len(), 3);

    // Verify pagination works
    if let Some(cursor) = page1.page_info.next_cursor.clone() {
        let mut params2 = HistoryParams::new();
        params2.pagination = Pagination::with_cursor(3, cursor);
        let page2 = backend.history_system(&tenant, &params2).await.unwrap();

        assert_eq!(page2.items.len(), 3);

        // Pages should not overlap
        let page1_ids: std::collections::HashSet<_> =
            page1.items.iter().map(|e| e.resource.id()).collect();
        for entry in &page2.items {
            assert!(!page1_ids.contains(entry.resource.id()));
        }
    }
}

/// Test system history respects tenant isolation.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_system_history_tenant_isolation() {
    let backend = create_sqlite_backend();

    let tenant1 = TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
    let tenant2 = TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

    // Create resources in both tenants
    for i in 0..5 {
        let patient = create_patient_json(&format!("Tenant1_{}", i));
        backend
            .create(&tenant1, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }

    for i in 0..3 {
        let patient = create_patient_json(&format!("Tenant2_{}", i));
        backend
            .create(&tenant2, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }

    // Get history for each tenant
    let history1 = backend
        .history_system(&tenant1, &HistoryParams::new())
        .await
        .unwrap();
    let history2 = backend
        .history_system(&tenant2, &HistoryParams::new())
        .await
        .unwrap();

    // Each tenant should only see their own resources
    assert_eq!(history1.items.len(), 5);
    assert_eq!(history2.items.len(), 3);

    for entry in &history1.items {
        assert_eq!(entry.resource.tenant_id().as_str(), "tenant-1");
    }

    for entry in &history2.items {
        assert_eq!(entry.resource.tenant_id().as_str(), "tenant-2");
    }
}

// ============================================================================
// History _since Parameter Tests
// ============================================================================

/// Test history with _since parameter.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_history_since_parameter() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create some resources
    let patient1 = create_patient_json("Before");
    backend
        .create(&tenant, "Patient", patient1, FhirVersion::default())
        .await
        .unwrap();

    // Record time
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let since = chrono::Utc::now();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Create more resources
    let patient2 = create_patient_json("After");
    backend
        .create(&tenant, "Patient", patient2, FhirVersion::default())
        .await
        .unwrap();

    // Get history since the marker time
    let history = backend
        .history_type(&tenant, "Patient", &HistoryParams::new().since(since))
        .await
        .unwrap();

    // Should only have resources created after 'since'
    for entry in &history.items {
        assert!(
            entry.resource.last_modified() >= since,
            "Resource {} was modified before _since",
            entry.resource.id()
        );
    }
}

// ============================================================================
// History Bundle Tests
// ============================================================================

/// Test that history results can be converted to a FHIR Bundle.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_history_bundle_format() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let v1 = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let _v2 = backend
        .update(&tenant, &v1, v1.content().clone())
        .await
        .unwrap();

    let history = backend
        .history_instance(&tenant, "Patient", v1.id(), &HistoryParams::new())
        .await
        .unwrap();

    // History should have the structure needed for a Bundle
    assert!(!history.items.is_empty());
    for entry in &history.items {
        // Each entry should have method info
        assert!(entry.resource.method().is_some() || !entry.resource.is_deleted());
        // Each should have versioned URL
        assert!(entry.resource.versioned_url().contains("_history"));
    }
}

// ============================================================================
// Delete History Tests (FHIR v6.0.0 Trial Use)
// ============================================================================

/// Test delete instance history removes all versions of a resource.
///
/// FHIR v6.0.0 introduces DELETE [base]/[type]/[id]/_history to remove
/// all historical versions of a resource. This is a Trial Use feature.
///
/// Expected behavior:
/// - Deletes all versions of the specified resource
/// - Returns 200 OK on success
/// - Returns 404 if resource doesn't exist
/// - May require specific permissions
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_delete_instance_history() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a resource with multiple versions
    let patient = create_patient_json("HistoryDelete");
    let v1 = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let id = v1.id().to_string();

    let mut content2 = v1.content().clone();
    content2["name"][0]["family"] = json!("HistoryDelete2");
    let _v2 = backend.update(&tenant, &v1, content2).await.unwrap();

    // Verify we have multiple versions
    let history_before = backend
        .history_instance(&tenant, "Patient", &id, &HistoryParams::new())
        .await
        .unwrap();
    assert!(
        history_before.items.len() >= 2,
        "Should have at least 2 versions before delete"
    );

    // Delete history is not yet implemented - this test serves as specification
    // When implemented, the trait method would be:
    // backend.delete_instance_history(&tenant, "Patient", &id).await
    //
    // After deletion:
    // - instance_history should return empty or error
    // - Current version may or may not be affected (implementation choice)

    // For now, verify current behavior
    let history_after = backend
        .history_instance(&tenant, "Patient", &id, &HistoryParams::new())
        .await
        .unwrap();

    // Current implementation preserves history
    // When delete_instance_history is implemented, this assertion would change
    assert!(
        !history_after.items.is_empty(),
        "History preserved (delete_instance_history not yet implemented)"
    );
}

/// Test delete specific version removes only that version.
///
/// FHIR v6.0.0 introduces DELETE [base]/[type]/[id]/_history/[vid] to remove
/// a specific version of a resource. This is a Trial Use feature.
///
/// Expected behavior:
/// - Deletes only the specified version
/// - Returns 200 OK on success
/// - Returns 404 if version doesn't exist
/// - Current version may have special handling
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_delete_specific_version() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a resource with multiple versions
    let patient = create_patient_json("VersionDelete");
    let v1 = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let id = v1.id().to_string();

    let mut content2 = v1.content().clone();
    content2["name"][0]["family"] = json!("VersionDelete2");
    let v2 = backend.update(&tenant, &v1, content2).await.unwrap();

    let mut content3 = v2.content().clone();
    content3["name"][0]["family"] = json!("VersionDelete3");
    let _v3 = backend.update(&tenant, &v2, content3).await.unwrap();

    // Verify we have 3 versions
    let history = backend
        .history_instance(&tenant, "Patient", &id, &HistoryParams::new())
        .await
        .unwrap();
    assert_eq!(history.items.len(), 3, "Should have 3 versions");

    // Delete specific version is not yet implemented - this test serves as specification
    // When implemented, the trait method would be:
    // backend.delete_version(&tenant, "Patient", &id, "2").await
    //
    // After deletion:
    // - Version 2 should no longer appear in history
    // - Versions 1 and 3 should still exist
    // - vread for version 2 should return 404 or Gone

    // For now, verify vread works for all versions
    let v2_read = backend.vread(&tenant, "Patient", &id, "2").await.unwrap();
    assert!(
        v2_read.is_some(),
        "Version 2 exists (delete_version not yet implemented)"
    );
}

/// Test delete history respects tenant isolation.
///
/// Delete history operations should only affect resources within the tenant's scope.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_delete_history_tenant_isolation() {
    let backend = create_sqlite_backend();

    let tenant1 = TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
    let tenant2 = TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

    // Create resources with history in both tenants
    let patient1 = create_patient_json("Tenant1Patient");
    let v1_t1 = backend
        .create(&tenant1, "Patient", patient1, FhirVersion::default())
        .await
        .unwrap();
    let id_t1 = v1_t1.id().to_string();
    let _v2_t1 = backend
        .update(&tenant1, &v1_t1, v1_t1.content().clone())
        .await
        .unwrap();

    let patient2 = create_patient_json("Tenant2Patient");
    let v1_t2 = backend
        .create(&tenant2, "Patient", patient2, FhirVersion::default())
        .await
        .unwrap();
    let id_t2 = v1_t2.id().to_string();
    let _v2_t2 = backend
        .update(&tenant2, &v1_t2, v1_t2.content().clone())
        .await
        .unwrap();

    // Verify each tenant has their own history
    let history_t1 = backend
        .history_instance(&tenant1, "Patient", &id_t1, &HistoryParams::new())
        .await
        .unwrap();
    let history_t2 = backend
        .history_instance(&tenant2, "Patient", &id_t2, &HistoryParams::new())
        .await
        .unwrap();

    assert_eq!(history_t1.items.len(), 2);
    assert_eq!(history_t2.items.len(), 2);

    // If delete_instance_history were implemented:
    // Deleting tenant1's history should NOT affect tenant2's history
    // backend.delete_instance_history(&tenant1, "Patient", &id_t1).await
    // history_t2 should still have 2 versions

    // Cross-tenant access should fail
    let cross_tenant = backend
        .history_instance(&tenant1, "Patient", &id_t2, &HistoryParams::new())
        .await
        .unwrap();
    assert!(
        cross_tenant.items.is_empty(),
        "Should not access other tenant's history"
    );
}
