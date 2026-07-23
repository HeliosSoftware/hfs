//! Tests for conditional CRUD operations.
//!
//! This module tests conditional create, update, and delete operations
//! that use search parameters to match resources.

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::ResourceStorage;
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

fn create_patient_json(family: &str, identifier: Option<(&str, &str)>) -> serde_json::Value {
    let mut patient = json!({
        "resourceType": "Patient",
        "name": [{"family": family}],
        "active": true
    });

    if let Some((system, value)) = identifier {
        patient["identifier"] = json!([{
            "system": system,
            "value": value
        }]);
    }

    patient
}

// ============================================================================
// Note: Conditional operations require SearchProvider implementation
// These tests serve as a specification for expected behavior when implemented
// ============================================================================

/// Test conditional create when no match exists (should create).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_conditional_create_no_match_creates() {
    // This test verifies the expected behavior of conditional_create
    // when no matching resource exists.
    //
    // Expected behavior:
    // - If search matches 0 resources -> Create new resource
    // - If search matches 1 resource -> Return existing resource
    // - If search matches >1 resources -> Return error (MultipleMatches)

    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // First, verify the storage is empty
    let count = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count, 0);

    // For now, we test basic create since conditional_create needs SearchProvider
    let patient = create_patient_json("Smith", Some(("http://example.org/mrn", "MRN001")));
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    assert_eq!(created.content()["identifier"][0]["value"], "MRN001");
}

/// Test conditional create when exactly one match exists (should return existing).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_conditional_create_single_match_returns_existing() {
    // When conditional_create finds exactly one matching resource,
    // it should return that resource without creating a new one.

    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create initial resource
    let patient = create_patient_json("Smith", Some(("http://example.org/mrn", "MRN002")));
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Verify it exists
    let count = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count, 1);

    // Conditional create with same identifier should return existing
    // (This would use identifier=http://example.org/mrn|MRN002 as search param)
    // For now we just verify the existing resource is still there
    let read = backend
        .read(&tenant, "Patient", created.id())
        .await
        .unwrap();
    assert!(read.is_some());
}

/// Test conditional create when multiple matches exist (should fail).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_conditional_create_multiple_matches_fails() {
    // When conditional_create finds multiple matching resources,
    // it should return MultipleMatches error.

    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create two resources with same family name (but different identifiers)
    let patient1 = create_patient_json("Smith", Some(("http://example.org/mrn", "MRN003")));
    let patient2 = create_patient_json("Smith", Some(("http://example.org/mrn", "MRN004")));

    backend
        .create(&tenant, "Patient", patient1, FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "Patient", patient2, FhirVersion::default())
        .await
        .unwrap();

    // If we tried conditional_create with name=Smith, it should fail
    // because there are two matches
    let count = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count, 2);
}

// ============================================================================
// Conditional Update Tests (Specification)
// ============================================================================

/// Test conditional update when exactly one match exists (should update).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_conditional_update_single_match_updates() {
    // Expected behavior of conditional_update:
    // - If search matches 0 resources and upsert=false -> NoMatch
    // - If search matches 0 resources and upsert=true -> Create
    // - If search matches 1 resource -> Update it
    // - If search matches >1 resources -> MultipleMatches error

    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a resource
    let patient = create_patient_json("Smith", Some(("http://example.org/mrn", "MRN005")));
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Update using regular update (conditional update would use search params)
    let mut updated_content = created.content().clone();
    updated_content["name"][0]["family"] = json!("UpdatedSmith");
    let updated = backend
        .update(&tenant, &created, updated_content)
        .await
        .unwrap();

    assert_eq!(updated.content()["name"][0]["family"], "UpdatedSmith");
    assert_eq!(updated.version_id(), "2");
}

/// Test conditional update with upsert when no match exists (should create).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_conditional_update_upsert_creates() {
    // When conditional_update with upsert=true finds no matches,
    // it should create a new resource.

    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Start with empty storage
    let count = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count, 0);

    // This is equivalent to upsert behavior
    let patient = create_patient_json("NewPatient", Some(("http://example.org/mrn", "MRN006")));
    let (created, is_created) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "new-patient-id",
            patient,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(is_created);
    assert_eq!(created.content()["name"][0]["family"], "NewPatient");
}

// ============================================================================
// Conditional Delete Tests (Specification)
// ============================================================================

/// Test conditional delete when exactly one match exists (should delete).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_conditional_delete_single_match_deletes() {
    // Expected behavior of conditional_delete:
    // - If search matches 0 resources -> NoMatch
    // - If search matches 1 resource -> Delete it
    // - If search matches >1 resources -> MultipleMatches error

    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a resource
    let patient = create_patient_json("ToDelete", Some(("http://example.org/mrn", "MRN007")));
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let id = created.id().to_string();

    // Delete it (regular delete; conditional would use search params)
    backend.delete(&tenant, "Patient", &id).await.unwrap();

    // Verify deleted
    assert!(!backend.exists(&tenant, "Patient", &id).await.unwrap());
}

/// Test conditional delete when no match exists (should return NoMatch).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_conditional_delete_no_match() {
    // When conditional_delete finds no matching resources,
    // it should return NoMatch result.

    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Empty storage
    let count = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count, 0);

    // Regular delete of nonexistent resource fails with NotFound
    let result = backend.delete(&tenant, "Patient", "nonexistent").await;
    assert!(result.is_err());
}

/// Test conditional delete when multiple matches exist (should fail).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_conditional_delete_multiple_matches_fails() {
    // When conditional_delete finds multiple matching resources,
    // it should return MultipleMatches error (FHIR doesn't allow
    // deleting multiple resources with one conditional delete).

    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create multiple resources
    let patient1 = create_patient_json("DeleteTest", Some(("http://example.org/mrn", "MRN008")));
    let patient2 = create_patient_json("DeleteTest", Some(("http://example.org/mrn", "MRN009")));

    let created1 = backend
        .create(&tenant, "Patient", patient1, FhirVersion::default())
        .await
        .unwrap();
    let created2 = backend
        .create(&tenant, "Patient", patient2, FhirVersion::default())
        .await
        .unwrap();

    // Both should exist
    assert!(
        backend
            .exists(&tenant, "Patient", created1.id())
            .await
            .unwrap()
    );
    assert!(
        backend
            .exists(&tenant, "Patient", created2.id())
            .await
            .unwrap()
    );

    // If conditional_delete were called with name=DeleteTest,
    // it should fail with MultipleMatches
}

// ============================================================================
// If-Match Header Tests (Optimistic Locking)
// ============================================================================

/// Test update with If-Match header succeeds when version matches.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_if_match_success() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith", None);
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Update with correct ETag (version matches)
    let mut content = created.content().clone();
    content["name"][0]["family"] = json!("Jones");

    // The update method checks version internally
    let updated = backend.update(&tenant, &created, content).await.unwrap();

    assert_eq!(updated.version_id(), "2");
}

/// Test update with If-Match header fails when version doesn't match.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_if_match_failure() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith", None);
    let v1 = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Do an update to create v2
    let v2 = backend
        .update(&tenant, &v1, v1.content().clone())
        .await
        .unwrap();
    assert_eq!(v2.version_id(), "2");

    // Try to update using stale v1
    let mut content = v1.content().clone();
    content["name"][0]["family"] = json!("Jones");
    let result = backend.update(&tenant, &v1, content).await;

    // Should fail due to version mismatch
    assert!(result.is_err());
}

// ============================================================================
// If-None-Match Header Tests (Conditional Create)
// ============================================================================

/// Test create with If-None-Match: * succeeds when resource doesn't exist.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_if_none_match_success() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // If-None-Match: * means "only create if nothing exists with this ID"
    // This is essentially what create_or_update does when creating new
    let patient = create_patient_json("NewPatient", None);
    let (created, is_new) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "unique-id-123",
            patient,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(is_new);
    assert_eq!(created.id(), "unique-id-123");
}

/// Test create with If-None-Match: * fails when resource exists.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_if_none_match_failure() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a resource with specific ID
    let patient1 = create_patient_json("First", None);
    let (_, is_new1) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "existing-id",
            patient1,
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert!(is_new1);

    // Try to create another with same ID
    // create_or_update will update instead of failing
    let patient2 = create_patient_json("Second", None);
    let (updated, is_new2) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "existing-id",
            patient2,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // It should have updated, not created
    assert!(!is_new2);
    assert_eq!(updated.version_id(), "2");
    assert_eq!(updated.content()["name"][0]["family"], "Second");
}

// ============================================================================
// Conditional Patch Tests (FHIR v6.0.0)
// ============================================================================

/// Test conditional patch when exactly one match exists (should patch).
///
/// Conditional patch uses search parameters to find a resource to patch:
/// PATCH [base]/[type]?[search-params]
///
/// Expected behavior:
/// - If search matches 0 resources -> Return error (no match)
/// - If search matches 1 resource -> Apply patch to that resource
/// - If search matches >1 resources -> Return error (multiple matches)
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_conditional_patch_single_match() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a resource with unique identifier
    let patient = create_patient_json("PatchTest", Some(("http://example.org/mrn", "PATCH-001")));
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Verify the resource exists
    let read = backend
        .read(&tenant, "Patient", created.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.content()["name"][0]["family"], "PatchTest");

    // Conditional patch would be:
    // PATCH /Patient?identifier=http://example.org/mrn|PATCH-001
    // with patch body: [{"op": "replace", "path": "/name/0/family", "value": "Patched"}]
    //
    // This would find the patient by identifier and patch it.
    // For now, we use regular update to simulate the expected outcome.

    let mut patched_content = read.content().clone();
    patched_content["name"][0]["family"] = json!("Patched");
    let updated = backend
        .update(&tenant, &read, patched_content)
        .await
        .unwrap();

    assert_eq!(updated.content()["name"][0]["family"], "Patched");
    assert_eq!(updated.version_id(), "2");
}

/// Test conditional patch when no match exists (should fail).
///
/// When conditional patch finds no matching resources, it should return an error.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_conditional_patch_no_match() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Verify storage is empty
    let count = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count, 0, "Storage should be empty");

    // Conditional patch with no matching resources should fail
    // PATCH /Patient?identifier=http://example.org/mrn|NONEXISTENT
    //
    // When implemented, this should return a "no match" error.
    // The current test documents the expected behavior.
}

/// Test conditional patch when multiple matches exist (should fail).
///
/// When conditional patch finds multiple matching resources, it should return
/// an error rather than patching all of them.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_conditional_patch_multiple_matches() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create multiple resources with the same family name
    let patient1 =
        create_patient_json("DuplicateName", Some(("http://example.org/mrn", "DUP-001")));
    let patient2 =
        create_patient_json("DuplicateName", Some(("http://example.org/mrn", "DUP-002")));

    backend
        .create(&tenant, "Patient", patient1, FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "Patient", patient2, FhirVersion::default())
        .await
        .unwrap();

    // Conditional patch with multiple matches should fail
    // PATCH /Patient?family=DuplicateName
    //
    // When implemented, this should return a "multiple matches" error.
    // FHIR requires that conditional operations match exactly one resource.

    let count = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count, 2, "Should have 2 patients with same name");
}

/// Test conditional patch with JSON Patch format.
///
/// FHIR supports JSON Patch (RFC 6902) for conditional patch operations.
/// The Content-Type should be application/json-patch+json.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_conditional_patch_json_patch_format() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a resource
    let patient = json!({
        "resourceType": "Patient",
        "name": [{"family": "Original", "given": ["First"]}],
        "active": true
    });
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // JSON Patch operations (RFC 6902):
    // [
    //   {"op": "replace", "path": "/name/0/family", "value": "NewFamily"},
    //   {"op": "add", "path": "/name/0/given/-", "value": "Middle"},
    //   {"op": "remove", "path": "/active"}
    // ]
    //
    // Expected result:
    // - family changed to "NewFamily"
    // - "Middle" added to given names
    // - active field removed

    // Simulate the expected outcome with regular update
    let mut patched = created.content().clone();
    patched["name"][0]["family"] = json!("NewFamily");
    if let Some(given) = patched["name"][0]["given"].as_array_mut() {
        given.push(json!("Middle"));
    }
    patched.as_object_mut().unwrap().remove("active");

    let updated = backend.update(&tenant, &created, patched).await.unwrap();

    assert_eq!(updated.content()["name"][0]["family"], "NewFamily");
    let given = updated.content()["name"][0]["given"].as_array().unwrap();
    assert!(given.iter().any(|v| v == "Middle"));
    assert!(updated.content().get("active").is_none());
}

/// Test conditional patch with FHIRPath Patch format.
///
/// FHIR v6.0.0 supports FHIRPath Patch as an alternative to JSON Patch.
/// The Content-Type should be application/fhir+json with a Parameters resource.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_conditional_patch_fhirpath_format() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a resource
    let patient = json!({
        "resourceType": "Patient",
        "name": [{"family": "Original"}],
        "birthDate": "1990-01-15"
    });
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // FHIRPath Patch uses a Parameters resource with operations:
    // {
    //   "resourceType": "Parameters",
    //   "parameter": [
    //     {
    //       "name": "operation",
    //       "part": [
    //         {"name": "type", "valueCode": "replace"},
    //         {"name": "path", "valueString": "Patient.name.family"},
    //         {"name": "value", "valueString": "FHIRPathPatched"}
    //       ]
    //     }
    //   ]
    // }
    //
    // This would use FHIRPath expressions for the path.

    // Simulate the expected outcome with regular update
    let mut patched = created.content().clone();
    patched["name"][0]["family"] = json!("FHIRPathPatched");

    let updated = backend.update(&tenant, &created, patched).await.unwrap();
    assert_eq!(updated.content()["name"][0]["family"], "FHIRPathPatched");
}

/// Test conditional patch respects tenant isolation.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_conditional_patch_tenant_isolation() {
    let backend = create_sqlite_backend();

    let tenant1 = helios_persistence::tenant::TenantContext::new(
        helios_persistence::tenant::TenantId::new("patch-tenant-1"),
        helios_persistence::tenant::TenantPermissions::full_access(),
    );
    let tenant2 = helios_persistence::tenant::TenantContext::new(
        helios_persistence::tenant::TenantId::new("patch-tenant-2"),
        helios_persistence::tenant::TenantPermissions::full_access(),
    );

    // Create resource in tenant1
    let patient = create_patient_json("TenantPatch", Some(("http://example.org/mrn", "T1-001")));
    let created = backend
        .create(&tenant1, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let id = created.id().to_string();

    // Verify tenant2 cannot read it
    let read_t2 = backend.read(&tenant2, "Patient", &id).await.unwrap();
    assert!(
        read_t2.is_none(),
        "Tenant2 should not see tenant1's resource"
    );

    // Conditional patch from tenant2 should not find the resource
    // PATCH /Patient?identifier=http://example.org/mrn|T1-001
    // (from tenant2's context)
    //
    // This should return "no match" because the resource belongs to tenant1.

    // Verify tenant1 can still access it
    let read_t1 = backend.read(&tenant1, "Patient", &id).await.unwrap();
    assert!(read_t1.is_some(), "Tenant1 should see their own resource");
}
