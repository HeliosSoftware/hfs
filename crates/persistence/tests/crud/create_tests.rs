//! Tests for resource creation operations.
//!
//! This module tests the `create` and `create_or_update` methods of the
//! ResourceStorage trait.

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::ResourceStorage;
use helios_persistence::error::StorageError;
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

fn create_patient_json(id: Option<&str>) -> serde_json::Value {
    let mut patient = json!({
        "resourceType": "Patient",
        "name": [{"family": "Smith", "given": ["John"]}],
        "active": true
    });
    if let Some(id) = id {
        patient["id"] = json!(id);
    }
    patient
}

// ============================================================================
// Create Tests - Basic
// ============================================================================

/// Test that creating a new resource succeeds and returns the stored resource.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_create_resource_success() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    let patient = create_patient_json(None);

    let result = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await;

    assert!(result.is_ok(), "Create should succeed");
    let stored = result.unwrap();
    assert_eq!(stored.resource_type(), "Patient");
    assert!(!stored.id().is_empty(), "ID should be assigned");
    assert_eq!(stored.version_id(), "1", "Initial version should be 1");
    assert!(!stored.is_deleted(), "Resource should not be deleted");
}

/// Test that the created resource has proper FHIR metadata.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_create_resource_has_metadata() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    let patient = create_patient_json(None);

    let stored = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .expect("Create should succeed");

    // Check content has id and meta
    let content = stored.content();
    assert!(content.get("id").is_some(), "Content should have id");
    assert_eq!(
        content["resourceType"], "Patient",
        "resourceType should be preserved"
    );

    // Check ETag
    assert!(stored.etag().starts_with("W/\""), "ETag should be weak");

    // Check timestamps
    assert!(
        stored.created_at() <= chrono::Utc::now(),
        "created_at should be set"
    );
    assert!(
        stored.last_modified() <= chrono::Utc::now(),
        "last_modified should be set"
    );
}

/// Test that the tenant ID is properly associated with the created resource.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_create_resource_tenant_association() {
    let backend = create_sqlite_backend();
    let tenant_id = TenantId::new("my-tenant");
    let tenant = TenantContext::new(tenant_id.clone(), TenantPermissions::full_access());
    let patient = create_patient_json(None);

    let stored = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .expect("Create should succeed");

    assert_eq!(stored.tenant_id(), &tenant_id, "Tenant ID should match");
}

/// Test creating resources of different types.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_create_different_resource_types() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a Patient
    let patient = create_patient_json(None);
    let patient_stored = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    assert_eq!(patient_stored.resource_type(), "Patient");

    // Create an Observation
    let observation = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {
            "coding": [{"system": "http://loinc.org", "code": "8867-4"}]
        }
    });
    let obs_stored = backend
        .create(&tenant, "Observation", observation, FhirVersion::default())
        .await
        .unwrap();
    assert_eq!(obs_stored.resource_type(), "Observation");

    // Create an Organization
    let organization = json!({
        "resourceType": "Organization",
        "name": "Test Hospital"
    });
    let org_stored = backend
        .create(
            &tenant,
            "Organization",
            organization,
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_eq!(org_stored.resource_type(), "Organization");
}

// ============================================================================
// Create Tests - Error Cases
// ============================================================================

/// Test that creating without create permission fails.
#[cfg(feature = "sqlite")]
#[tokio::test]
#[ignore = "#306: storage layer does not enforce TenantPermissions (no permission checks in any backend); asserts an authz property persistence does not implement — see PR #361"]
async fn test_create_without_permission_fails() {
    let backend = create_sqlite_backend();
    let tenant = TenantContext::new(TenantId::new("test"), TenantPermissions::read_only());
    let patient = create_patient_json(None);

    let result = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await;

    assert!(result.is_err(), "Create without permission should fail");
    match result {
        Err(StorageError::Tenant(_)) => {}
        Err(e) => panic!("Expected TenantError, got {:?}", e),
        Ok(_) => panic!("Expected error"),
    }
}

// ============================================================================
// Create or Update Tests
// ============================================================================

/// Test that create_or_update creates a new resource when it doesn't exist.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_create_or_update_creates_new() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    let patient = create_patient_json(Some("new-patient-123"));

    let (stored, created) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "new-patient-123",
            patient,
            FhirVersion::default(),
        )
        .await
        .expect("create_or_update should succeed");

    assert!(created, "Should indicate resource was created");
    assert_eq!(stored.id(), "new-patient-123");
    assert_eq!(stored.version_id(), "1");
}

/// Test that create_or_update updates an existing resource.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_create_or_update_updates_existing() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // First create a resource
    let patient1 = create_patient_json(Some("patient-456"));
    let (stored1, created1) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "patient-456",
            patient1,
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert!(created1);
    assert_eq!(stored1.version_id(), "1");

    // Now update it
    let mut patient2 = create_patient_json(Some("patient-456"));
    patient2["active"] = json!(false);

    let (stored2, created2) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "patient-456",
            patient2,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(
        !created2,
        "Should indicate resource was updated, not created"
    );
    assert_eq!(stored2.id(), "patient-456");
    assert_eq!(stored2.version_id(), "2", "Version should be incremented");
    assert_eq!(stored2.content()["active"], false);
}

/// Test that create_or_update preserves resource history.
#[cfg(feature = "sqlite")]
#[tokio::test]
#[ignore = "#306: SQLite has no created_at column and approximates it with last_modified (backends/sqlite/storage.rs), so created_at cannot survive an update — see PR #361"]
async fn test_create_or_update_preserves_created_timestamp() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create initial resource
    let patient1 = create_patient_json(Some("patient-789"));
    let (stored1, _) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "patient-789",
            patient1,
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let created_at = stored1.created_at();

    // Wait a tiny bit
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    // Update it
    let patient2 = create_patient_json(Some("patient-789"));
    let (stored2, _) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "patient-789",
            patient2,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(
        stored2.created_at(),
        created_at,
        "created_at should be preserved"
    );
    assert!(
        stored2.last_modified() >= created_at,
        "last_modified should be updated"
    );
}

// ============================================================================
// Create Tests - Multiple Resources
// ============================================================================

/// Test creating multiple resources of the same type.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_create_multiple_resources() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let mut ids = Vec::new();
    for i in 0..10 {
        let patient = json!({
            "resourceType": "Patient",
            "name": [{"family": format!("Patient{}", i)}]
        });
        let stored = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
        ids.push(stored.id().to_string());
    }

    // Verify all IDs are unique
    let unique_ids: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(ids.len(), unique_ids.len(), "All IDs should be unique");
}

/// Test that resources in different tenants are isolated.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_create_tenant_isolation() {
    let backend = create_sqlite_backend();

    let tenant1 = TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
    let tenant2 = TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

    // Create same patient in both tenants
    let patient = create_patient_json(None);

    let stored1 = backend
        .create(&tenant1, "Patient", patient.clone(), FhirVersion::default())
        .await
        .unwrap();
    let stored2 = backend
        .create(&tenant2, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Verify different tenant IDs
    assert_eq!(stored1.tenant_id().as_str(), "tenant-1");
    assert_eq!(stored2.tenant_id().as_str(), "tenant-2");

    // Each should have its own count
    let count1 = backend.count(&tenant1, Some("Patient")).await.unwrap();
    let count2 = backend.count(&tenant2, Some("Patient")).await.unwrap();
    assert_eq!(count1, 1);
    assert_eq!(count2, 1);
}

// ============================================================================
// Create Tests - Content Validation
// ============================================================================

/// Test that resource content is preserved correctly.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_create_preserves_content() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = json!({
        "resourceType": "Patient",
        "name": [{
            "family": "Smith",
            "given": ["John", "Jacob"],
            "prefix": ["Mr."]
        }],
        "birthDate": "1980-01-15",
        "gender": "male",
        "active": true,
        "address": [{
            "line": ["123 Main St"],
            "city": "Boston",
            "state": "MA",
            "postalCode": "02101"
        }],
        "identifier": [{
            "system": "http://example.org/mrn",
            "value": "MRN12345"
        }]
    });

    let stored = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let content = stored.content();

    assert_eq!(content["name"][0]["family"], "Smith");
    assert_eq!(content["name"][0]["given"][0], "John");
    assert_eq!(content["name"][0]["given"][1], "Jacob");
    assert_eq!(content["birthDate"], "1980-01-15");
    assert_eq!(content["gender"], "male");
    assert_eq!(content["address"][0]["city"], "Boston");
    assert_eq!(content["identifier"][0]["value"], "MRN12345");
}

/// Test that nested objects are preserved.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_create_preserves_nested_objects() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let observation = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {
            "coding": [{
                "system": "http://loinc.org",
                "code": "8867-4",
                "display": "Heart rate"
            }],
            "text": "Heart rate"
        },
        "subject": {
            "reference": "Patient/123"
        },
        "valueQuantity": {
            "value": 72,
            "unit": "beats/minute",
            "system": "http://unitsofmeasure.org",
            "code": "/min"
        },
        "component": [{
            "code": {
                "coding": [{
                    "system": "http://loinc.org",
                    "code": "8867-4"
                }]
            },
            "valueQuantity": {
                "value": 72
            }
        }]
    });

    let stored = backend
        .create(&tenant, "Observation", observation, FhirVersion::default())
        .await
        .unwrap();
    let content = stored.content();

    assert_eq!(content["code"]["coding"][0]["code"], "8867-4");
    assert_eq!(content["valueQuantity"]["value"], 72);
    assert_eq!(content["component"][0]["valueQuantity"]["value"], 72);
}

// ============================================================================
// Create Tests - Special Cases
// ============================================================================

/// Test creating a resource with an empty array field.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_create_with_empty_arrays() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = json!({
        "resourceType": "Patient",
        "name": [],
        "identifier": []
    });

    let stored = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let content = stored.content();

    assert!(content["name"].as_array().unwrap().is_empty());
    assert!(content["identifier"].as_array().unwrap().is_empty());
}

/// Test creating a resource with null fields.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_create_with_null_fields() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = json!({
        "resourceType": "Patient",
        "name": [{"family": "Smith"}],
        "birthDate": null
    });

    let stored = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let content = stored.content();

    // Null fields may or may not be preserved depending on implementation
    assert_eq!(content["name"][0]["family"], "Smith");
}

/// Test creating resources with Unicode content.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_create_with_unicode() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = json!({
        "resourceType": "Patient",
        "name": [{
            "family": "日本語",
            "given": ["太郎"]
        }],
        "address": [{
            "city": "東京都"
        }]
    });

    let stored = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let content = stored.content();

    assert_eq!(content["name"][0]["family"], "日本語");
    assert_eq!(content["name"][0]["given"][0], "太郎");
    assert_eq!(content["address"][0]["city"], "東京都");
}

/// Test creating a resource with large content.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_create_with_large_content() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a patient with many names
    let mut names = Vec::new();
    for i in 0..100 {
        names.push(json!({
            "family": format!("Family{}", i),
            "given": [format!("Given{}", i)]
        }));
    }

    let patient = json!({
        "resourceType": "Patient",
        "name": names
    });

    let stored = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let content = stored.content();

    assert_eq!(content["name"].as_array().unwrap().len(), 100);
}
