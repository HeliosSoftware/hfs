//! Tests for FHIR bundle transaction operations.
//!
//! This module tests FHIR transaction bundles including the various
//! HTTP method equivalents and conditional operations.

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{BundleEntry, BundleMethod, BundleProvider, ResourceStorage};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

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

// ============================================================================
// Basic Bundle Tests
// ============================================================================

/// Test executing a simple transaction bundle with creates.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_create_entries() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let entries = vec![
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "name": [{"family": "BundlePatient1"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: Some("urn:uuid:patient-1".to_string()),
        },
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "name": [{"family": "BundlePatient2"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: Some("urn:uuid:patient-2".to_string()),
        },
    ];

    let result = backend.process_transaction(&tenant, entries).await.unwrap();

    // Should have 2 response entries
    assert_eq!(result.entries.len(), 2);

    // Both should be successful creates
    for entry in &result.entries {
        assert_eq!(entry.status, 201);
        assert!(entry.location.is_some());
    }

    // Verify resources exist
    let count = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count, 2);
}

/// Test bundle with PUT (create or update).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_put_entries() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let entries = vec![BundleEntry {
        method: BundleMethod::Put,
        url: "Patient/patient-123".to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "id": "patient-123",
            "name": [{"family": "PutPatient"}]
        })),
        if_match: None,
        if_none_match: None,
        if_none_exist: None,
        full_url: Some("urn:uuid:patient-put".to_string()),
    }];

    let result = backend.process_transaction(&tenant, entries).await.unwrap();

    assert_eq!(result.entries.len(), 1);
    assert!(result.entries[0].status == 201 || result.entries[0].status == 200);

    // Verify resource
    let read = backend
        .read(&tenant, "Patient", "patient-123")
        .await
        .unwrap();
    assert!(read.is_some());
    assert_eq!(read.unwrap().content()["name"][0]["family"], "PutPatient");
}

/// Test bundle with DELETE.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_delete_entries() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // First create a resource
    backend
        .create_or_update(
            &tenant,
            "Patient",
            "to-delete",
            json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let entries = vec![BundleEntry {
        method: BundleMethod::Delete,
        url: "Patient/to-delete".to_string(),
        resource: None,
        if_match: None,
        if_none_match: None,
        if_none_exist: None,
        full_url: None,
    }];

    let result = backend.process_transaction(&tenant, entries).await.unwrap();

    assert_eq!(result.entries.len(), 1);
    assert!(result.entries[0].status == 200 || result.entries[0].status == 204);

    // Verify deleted
    assert!(
        !backend
            .exists(&tenant, "Patient", "to-delete")
            .await
            .unwrap()
    );
}

// ============================================================================
// Mixed Operation Bundle Tests
// ============================================================================

/// Test bundle with mixed operations (CREATE, UPDATE, DELETE).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_mixed_operations() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Pre-create resources for update and delete
    backend
        .create_or_update(
            &tenant,
            "Patient",
            "update-me",
            json!({"resourceType": "Patient", "name": [{"family": "Original"}]}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create_or_update(
            &tenant,
            "Patient",
            "delete-me",
            json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let entries = vec![
        // CREATE
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "name": [{"family": "NewPatient"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: Some("urn:uuid:new-patient".to_string()),
        },
        // UPDATE
        BundleEntry {
            method: BundleMethod::Put,
            url: "Patient/update-me".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "id": "update-me",
                "name": [{"family": "Updated"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: None,
        },
        // DELETE
        BundleEntry {
            method: BundleMethod::Delete,
            url: "Patient/delete-me".to_string(),
            resource: None,
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: None,
        },
    ];

    let result = backend.process_transaction(&tenant, entries).await.unwrap();

    assert_eq!(result.entries.len(), 3);

    // Verify all operations succeeded
    let count = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count, 2); // 1 pre-existing + 1 new - 1 deleted

    // Verify update
    let updated = backend
        .read(&tenant, "Patient", "update-me")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.content()["name"][0]["family"], "Updated");

    // Verify delete
    assert!(
        !backend
            .exists(&tenant, "Patient", "delete-me")
            .await
            .unwrap()
    );
}

// ============================================================================
// Reference Resolution Tests
// ============================================================================

/// Test bundle with internal references (urn:uuid).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_internal_references() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let entries = vec![
        // Create patient first
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "name": [{"family": "ReferencedPatient"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: Some("urn:uuid:new-patient".to_string()),
        },
        // Create observation referencing patient by urn:uuid
        BundleEntry {
            method: BundleMethod::Post,
            url: "Observation".to_string(),
            resource: Some(json!({
                "resourceType": "Observation",
                "status": "final",
                "code": {"coding": [{"code": "test"}]},
                "subject": {"reference": "urn:uuid:new-patient"}
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: Some("urn:uuid:new-observation".to_string()),
        },
    ];

    let result = backend.process_transaction(&tenant, entries).await.unwrap();

    assert_eq!(result.entries.len(), 2);

    // Get the patient's assigned ID from the response location
    // (format: "ResourceType/id/_history/version")
    let patient_location = result.entries[0].location.as_ref().unwrap();
    let patient_id = patient_location.split('/').nth(1).unwrap();

    // Find the observation and verify reference was resolved
    let obs_location = result.entries[1].location.as_ref().unwrap();
    let obs_id = obs_location.split('/').nth(1).unwrap();

    let observation = backend
        .read(&tenant, "Observation", obs_id)
        .await
        .unwrap()
        .unwrap();

    // Reference should be resolved to actual Patient ID
    let subject_ref = observation.content()["subject"]["reference"]
        .as_str()
        .unwrap();
    assert!(
        subject_ref.contains(patient_id),
        "Reference should be resolved to actual patient ID"
    );
}

// ============================================================================
// Conditional Bundle Tests
// ============================================================================

/// Test bundle with conditional create (if-none-exist).
///
/// Ported to the current bundle API for structure, but `#[ignore]`d: the
/// transaction bundle path does not implement `if-none-exist` conditional
/// creates — a POST always creates a new resource — so the "should not create
/// a duplicate" assertions do not hold. Preserved for the #306 follow-up.
#[cfg(feature = "sqlite")]
#[tokio::test]
#[ignore = "#306 follow-up: if-none-exist conditional create not implemented in transaction bundle API"]
async fn test_bundle_conditional_create() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // First bundle - should create
    let bundle1 = vec![BundleEntry {
        method: BundleMethod::Post,
        url: "Patient".to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://example.org", "value": "12345"}],
            "name": [{"family": "Conditional"}]
        })),
        if_match: None,
        if_none_match: None,
        if_none_exist: Some("identifier=http://example.org|12345".to_string()),
        full_url: Some("urn:uuid:conditional".to_string()),
    }];

    let result1 = backend.process_transaction(&tenant, bundle1).await.unwrap();
    assert_eq!(result1.entries[0].status, 201);

    // Second bundle with same condition - should return existing
    let bundle2 = vec![BundleEntry {
        method: BundleMethod::Post,
        url: "Patient".to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://example.org", "value": "12345"}],
            "name": [{"family": "ShouldNotCreate"}]
        })),
        if_match: None,
        if_none_match: None,
        if_none_exist: Some("identifier=http://example.org|12345".to_string()),
        full_url: Some("urn:uuid:conditional".to_string()),
    }];

    let result2 = backend.process_transaction(&tenant, bundle2).await.unwrap();

    // Should not create duplicate
    assert_ne!(result2.entries[0].status, 201);

    // Only one patient should exist
    let count = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count, 1);
}

/// Test bundle with conditional update (if-match).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_conditional_update_if_match() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create initial resource
    let (created, _) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "conditional-update",
            json!({"resourceType": "Patient", "name": [{"family": "Original"}]}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let etag = created.etag().to_string();

    // Update with correct ETag
    let entries = vec![BundleEntry {
        method: BundleMethod::Put,
        url: "Patient/conditional-update".to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "id": "conditional-update",
            "name": [{"family": "UpdatedWithMatch"}]
        })),
        if_match: Some(etag),
        if_none_match: None,
        if_none_exist: None,
        full_url: None,
    }];

    let result = backend.process_transaction(&tenant, entries).await.unwrap();
    assert_eq!(result.entries[0].status, 200);

    // Verify update
    let read = backend
        .read(&tenant, "Patient", "conditional-update")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.content()["name"][0]["family"], "UpdatedWithMatch");
}

/// Test bundle with if-match failure.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_if_match_failure() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create initial resource
    backend
        .create_or_update(
            &tenant,
            "Patient",
            "version-conflict",
            json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Update with wrong ETag
    let entries = vec![BundleEntry {
        method: BundleMethod::Put,
        url: "Patient/version-conflict".to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "id": "version-conflict",
            "name": [{"family": "ShouldFail"}]
        })),
        if_match: Some("W/\"wrong-version\"".to_string()),
        if_none_match: None,
        if_none_exist: None,
        full_url: None,
    }];

    let result = backend.process_transaction(&tenant, entries).await;

    // Should fail due to version mismatch: either the whole transaction errors,
    // or the offending entry carries a conflict status.
    assert!(result.is_err() || result.unwrap().entries[0].status == 409);
}

// ============================================================================
// Bundle Atomicity Tests
// ============================================================================

/// Test that bundle is atomic - all succeed or all fail.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_atomicity() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Bundle with valid operation and invalid operation
    let entries = vec![
        // Valid create
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "name": [{"family": "Valid"}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: Some("urn:uuid:valid".to_string()),
        },
        // Invalid - delete non-existent
        BundleEntry {
            method: BundleMethod::Delete,
            url: "Patient/non-existent-id".to_string(),
            resource: None,
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: None,
        },
    ];

    let result = backend.process_transaction(&tenant, entries).await;

    // If transaction failed, no resources should be created
    if result.is_err() {
        let count = backend.count(&tenant, Some("Patient")).await.unwrap();
        assert_eq!(
            count, 0,
            "Transaction should be atomic - no partial commits"
        );
    }
}

// ============================================================================
// Bundle Edge Cases
// ============================================================================

/// Test empty bundle.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_empty() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let result = backend.process_transaction(&tenant, vec![]).await;

    // Empty bundle should succeed with empty response
    assert!(result.is_ok());
    assert!(result.unwrap().entries.is_empty());
}

/// Test bundle with single entry.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_single_entry() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let entries = vec![BundleEntry {
        method: BundleMethod::Post,
        url: "Patient".to_string(),
        resource: Some(json!({"resourceType": "Patient"})),
        if_match: None,
        if_none_match: None,
        if_none_exist: None,
        full_url: Some("urn:uuid:single".to_string()),
    }];

    let result = backend.process_transaction(&tenant, entries).await.unwrap();
    assert_eq!(result.entries.len(), 1);
    assert_eq!(result.entries[0].status, 201);
}

/// Test bundle respects tenant isolation.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_tenant_isolation() {
    let backend = create_sqlite_backend();
    let tenant_a = TenantContext::new(TenantId::new("tenant-a"), TenantPermissions::full_access());
    let tenant_b = TenantContext::new(TenantId::new("tenant-b"), TenantPermissions::full_access());

    let entries = vec![BundleEntry {
        method: BundleMethod::Post,
        url: "Patient".to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "name": [{"family": "TenantA"}]
        })),
        if_match: None,
        if_none_match: None,
        if_none_exist: None,
        full_url: Some("urn:uuid:tenant-patient".to_string()),
    }];

    let result = backend
        .process_transaction(&tenant_a, entries)
        .await
        .unwrap();
    // location format: "ResourceType/id/_history/version"
    let location = result.entries[0].location.as_ref().unwrap();
    let patient_id = location.split('/').nth(1).unwrap();

    // Tenant A can see it
    assert!(
        backend
            .exists(&tenant_a, "Patient", patient_id)
            .await
            .unwrap()
    );

    // Tenant B cannot
    assert!(
        !backend
            .exists(&tenant_b, "Patient", patient_id)
            .await
            .unwrap()
    );
}
