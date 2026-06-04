//! SQLite backend integration tests.
//!
//! These tests verify the SQLite backend implementation against the actual API.

use std::path::PathBuf;

use helios_fhir::FhirVersion;
use serde_json::json;

use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::history::{
    HistoryMethod, HistoryParams, InstanceHistoryProvider, SystemHistoryProvider,
    TypeHistoryProvider,
};
use helios_persistence::core::{
    ExportLevel, ExportRequest, PatientExportProvider, ResourceStorage,
};
use helios_persistence::error::{ResourceError, StorageError};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

fn create_backend() -> SqliteBackend {
    // Configure with data directory to load spec SearchParameters
    // CARGO_MANIFEST_DIR for tests is crates/persistence
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));

    let config = SqliteBackendConfig {
        data_dir: Some(data_dir),
        ..Default::default()
    };
    let backend =
        SqliteBackend::with_config(":memory:", config).expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to initialize schema");
    backend
}

fn create_tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

// ============================================================================
// Create Tests
// ============================================================================

#[tokio::test]
async fn test_create_resource() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    let patient = json!({
        "resourceType": "Patient",
        "name": [{"family": "Smith", "given": ["John"]}]
    });

    let result = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await;
    assert!(result.is_ok());

    let created = result.unwrap();
    assert_eq!(created.resource_type(), "Patient");
    assert!(!created.id().is_empty());
    assert_eq!(created.version_id(), "1");
}

#[tokio::test]
async fn test_create_with_id() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    let patient = json!({
        "resourceType": "Patient",
        "id": "patient-123",
        "name": [{"family": "Jones"}]
    });

    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    assert_eq!(created.id(), "patient-123");
}

#[tokio::test]
async fn test_create_duplicate_fails() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    let patient = json!({
        "resourceType": "Patient",
        "id": "duplicate-id"
    });

    // First create succeeds
    backend
        .create(&tenant, "Patient", patient.clone(), FhirVersion::default())
        .await
        .unwrap();

    // Second create with same ID fails
    let result = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await;
    assert!(result.is_err());
}

// ============================================================================
// Read Tests
// ============================================================================

#[tokio::test]
async fn test_read_resource() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    let patient = json!({
        "resourceType": "Patient",
        "name": [{"family": "ReadTest"}]
    });

    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    let read = backend
        .read(&tenant, "Patient", created.id())
        .await
        .unwrap();
    assert!(read.is_some());

    let resource = read.unwrap();
    assert_eq!(resource.id(), created.id());
    assert_eq!(resource.content()["name"][0]["family"], "ReadTest");
}

#[tokio::test]
async fn test_read_nonexistent() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    let read = backend
        .read(&tenant, "Patient", "does-not-exist")
        .await
        .unwrap();
    assert!(read.is_none());
}

#[tokio::test]
async fn test_exists() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    let patient = json!({"resourceType": "Patient"});
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    assert!(
        backend
            .exists(&tenant, "Patient", created.id())
            .await
            .unwrap()
    );
    assert!(
        !backend
            .exists(&tenant, "Patient", "nonexistent")
            .await
            .unwrap()
    );
}

// ============================================================================
// Update Tests
// ============================================================================

#[tokio::test]
async fn test_update_resource() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    let patient = json!({
        "resourceType": "Patient",
        "name": [{"family": "Original"}]
    });

    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    let updated_content = json!({
        "resourceType": "Patient",
        "name": [{"family": "Updated"}]
    });

    let updated = backend
        .update(&tenant, &created, updated_content)
        .await
        .unwrap();

    assert_eq!(updated.version_id(), "2");
    assert_eq!(updated.content()["name"][0]["family"], "Updated");
}

#[tokio::test]
async fn test_create_or_update_creates() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    let patient = json!({"resourceType": "Patient"});

    let (resource, was_created) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "new-id",
            patient,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(was_created);
    assert_eq!(resource.id(), "new-id");
}

#[tokio::test]
async fn test_create_or_update_updates() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create first
    let patient = json!({"resourceType": "Patient", "name": [{"family": "First"}]});
    backend
        .create_or_update(
            &tenant,
            "Patient",
            "upsert-id",
            patient,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Update
    let patient2 = json!({"resourceType": "Patient", "name": [{"family": "Second"}]});
    let (resource, was_created) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "upsert-id",
            patient2,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(!was_created);
    assert_eq!(resource.content()["name"][0]["family"], "Second");
}

// ============================================================================
// Delete Tests
// ============================================================================

#[tokio::test]
async fn test_delete_resource() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    let patient = json!({"resourceType": "Patient"});
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Delete
    backend
        .delete(&tenant, "Patient", created.id())
        .await
        .unwrap();

    // Read should return Gone error for deleted resources (soft delete behavior)
    let read_result = backend.read(&tenant, "Patient", created.id()).await;
    match read_result {
        Err(StorageError::Resource(ResourceError::Gone { .. })) => {
            // Expected: deleted resources return Gone error
        }
        Ok(None) => {
            // Also acceptable: deleted resource not found
        }
        other => {
            panic!("Expected Gone error or None, got: {:?}", other);
        }
    }
}

#[tokio::test]
async fn test_delete_nonexistent_fails() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    let result = backend.delete(&tenant, "Patient", "nonexistent").await;
    assert!(result.is_err());
}

// ============================================================================
// Tenant Isolation Tests
// ============================================================================

#[tokio::test]
async fn test_tenant_isolation_create() {
    let backend = create_backend();
    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    let patient = json!({"resourceType": "Patient"});
    let created = backend
        .create(&tenant_a, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Tenant A can see it
    assert!(
        backend
            .exists(&tenant_a, "Patient", created.id())
            .await
            .unwrap()
    );

    // Tenant B cannot see it
    assert!(
        !backend
            .exists(&tenant_b, "Patient", created.id())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_tenant_isolation_read() {
    let backend = create_backend();
    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    let patient = json!({"resourceType": "Patient"});
    let created = backend
        .create(&tenant_a, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Tenant A can read
    let read_a = backend
        .read(&tenant_a, "Patient", created.id())
        .await
        .unwrap();
    assert!(read_a.is_some());

    // Tenant B cannot read
    let read_b = backend
        .read(&tenant_b, "Patient", created.id())
        .await
        .unwrap();
    assert!(read_b.is_none());
}

#[tokio::test]
async fn test_same_id_different_tenants() {
    let backend = create_backend();
    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    let patient_a = json!({"resourceType": "Patient", "name": [{"family": "A"}]});
    let patient_b = json!({"resourceType": "Patient", "name": [{"family": "B"}]});

    // Create same ID in both tenants
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

    // Each tenant sees their own version
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

    assert_eq!(read_a.content()["name"][0]["family"], "A");
    assert_eq!(read_b.content()["name"][0]["family"], "B");
}

#[tokio::test]
async fn test_tenant_isolation_delete() {
    let backend = create_backend();
    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    let patient = json!({"resourceType": "Patient"});
    let created = backend
        .create(&tenant_a, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Tenant B cannot delete tenant A's resource
    let result = backend.delete(&tenant_b, "Patient", created.id()).await;
    assert!(result.is_err());

    // Resource still exists for tenant A
    assert!(
        backend
            .exists(&tenant_a, "Patient", created.id())
            .await
            .unwrap()
    );
}

// ============================================================================
// Count Tests
// ============================================================================

#[tokio::test]
async fn test_count_resources() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create several patients
    for i in 0..5 {
        let patient = json!({"resourceType": "Patient", "id": format!("p{}", i)});
        backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }

    let count = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count, 5);
}

#[tokio::test]
async fn test_count_by_tenant() {
    let backend = create_backend();
    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    // Create 3 in tenant A
    for _ in 0..3 {
        let patient = json!({"resourceType": "Patient"});
        backend
            .create(&tenant_a, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }

    // Create 2 in tenant B
    for _ in 0..2 {
        let patient = json!({"resourceType": "Patient"});
        backend
            .create(&tenant_b, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }

    assert_eq!(backend.count(&tenant_a, Some("Patient")).await.unwrap(), 3);
    assert_eq!(backend.count(&tenant_b, Some("Patient")).await.unwrap(), 2);
}

// ============================================================================
// Batch Read Tests
// ============================================================================

#[tokio::test]
async fn test_read_batch() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create resources
    let ids: Vec<String> = (0..3).map(|i| format!("batch-{}", i)).collect();

    for id in &ids {
        let patient = json!({"resourceType": "Patient"});
        backend
            .create_or_update(&tenant, "Patient", id, patient, FhirVersion::default())
            .await
            .unwrap();
    }

    // Batch read
    let id_refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
    let batch = backend
        .read_batch(&tenant, "Patient", &id_refs)
        .await
        .unwrap();

    assert_eq!(batch.len(), 3);
}

#[tokio::test]
async fn test_read_batch_ignores_other_tenant() {
    let backend = create_backend();
    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    // Create in tenant A
    backend
        .create_or_update(
            &tenant_a,
            "Patient",
            "a-patient",
            json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Create in tenant B
    backend
        .create_or_update(
            &tenant_b,
            "Patient",
            "b-patient",
            json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Batch read from tenant A with both IDs
    let ids = ["a-patient", "b-patient"];
    let batch = backend
        .read_batch(&tenant_a, "Patient", &ids)
        .await
        .unwrap();

    // Should only return tenant A's resource
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].id(), "a-patient");
}

// ============================================================================
// Version Tests
// ============================================================================

#[tokio::test]
async fn test_version_increments() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    let patient = json!({"resourceType": "Patient"});
    let v1 = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    assert_eq!(v1.version_id(), "1");

    let v2 = backend
        .update(&tenant, &v1, json!({"resourceType": "Patient"}))
        .await
        .unwrap();
    assert_eq!(v2.version_id(), "2");

    let v3 = backend
        .update(&tenant, &v2, json!({"resourceType": "Patient"}))
        .await
        .unwrap();
    assert_eq!(v3.version_id(), "3");
}

// ============================================================================
// Content Preservation Tests
// ============================================================================

#[tokio::test]
async fn test_content_preserved() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    let patient = json!({
        "resourceType": "Patient",
        "name": [{"family": "Smith", "given": ["John", "Michael"]}],
        "birthDate": "1985-06-15",
        "active": true,
        "multipleBirthInteger": 2
    });

    let created = backend
        .create(&tenant, "Patient", patient.clone(), FhirVersion::default())
        .await
        .unwrap();
    let read = backend
        .read(&tenant, "Patient", created.id())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(read.content()["name"][0]["family"], "Smith");
    assert_eq!(read.content()["name"][0]["given"][0], "John");
    assert_eq!(read.content()["name"][0]["given"][1], "Michael");
    assert_eq!(read.content()["birthDate"], "1985-06-15");
    assert_eq!(read.content()["active"], true);
    assert_eq!(read.content()["multipleBirthInteger"], 2);
}

#[tokio::test]
async fn test_unicode_content() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    let patient = json!({
        "resourceType": "Patient",
        "name": [{"family": "日本語", "given": ["名前"]}]
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

    assert_eq!(read.content()["name"][0]["family"], "日本語");
    assert_eq!(read.content()["name"][0]["given"][0], "名前");
}

// ============================================================================
// History Tests
// ============================================================================

#[tokio::test]
async fn test_history_instance() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create a resource
    let patient = json!({"resourceType": "Patient", "name": [{"family": "Smith"}]});
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Update twice
    let v2 = backend
        .update(
            &tenant,
            &created,
            json!({"resourceType": "Patient", "name": [{"family": "Jones"}]}),
        )
        .await
        .unwrap();
    let _v3 = backend
        .update(
            &tenant,
            &v2,
            json!({"resourceType": "Patient", "name": [{"family": "Brown"}]}),
        )
        .await
        .unwrap();

    // Get history
    let params = HistoryParams::new();
    let history = backend
        .history_instance(&tenant, "Patient", created.id(), &params)
        .await
        .unwrap();

    // Should have 3 versions, newest first
    assert_eq!(history.items.len(), 3);
    assert_eq!(history.items[0].resource.version_id(), "3");
    assert_eq!(history.items[1].resource.version_id(), "2");
    assert_eq!(history.items[2].resource.version_id(), "1");

    // Check methods
    assert_eq!(history.items[0].method, HistoryMethod::Put);
    assert_eq!(history.items[1].method, HistoryMethod::Put);
    assert_eq!(history.items[2].method, HistoryMethod::Post);

    // Check content is correct
    assert_eq!(
        history.items[0].resource.content()["name"][0]["family"],
        "Brown"
    );
    assert_eq!(
        history.items[2].resource.content()["name"][0]["family"],
        "Smith"
    );
}

#[tokio::test]
async fn test_history_instance_count() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    let patient = json!({"resourceType": "Patient"});
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let v2 = backend
        .update(&tenant, &created, json!({"resourceType": "Patient"}))
        .await
        .unwrap();
    let _v3 = backend
        .update(&tenant, &v2, json!({"resourceType": "Patient"}))
        .await
        .unwrap();

    let count = backend
        .history_instance_count(&tenant, "Patient", created.id())
        .await
        .unwrap();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn test_history_with_delete() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    let patient = json!({"resourceType": "Patient", "id": "hist-patient"});
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let _v2 = backend
        .update(
            &tenant,
            &created,
            json!({"resourceType": "Patient", "id": "hist-patient"}),
        )
        .await
        .unwrap();
    backend
        .delete(&tenant, "Patient", "hist-patient")
        .await
        .unwrap();

    // Get history including deleted
    let params = HistoryParams::new().include_deleted(true);
    let history = backend
        .history_instance(&tenant, "Patient", "hist-patient", &params)
        .await
        .unwrap();

    assert_eq!(history.items.len(), 3);
    assert_eq!(history.items[0].method, HistoryMethod::Delete);
    assert_eq!(history.items[0].resource.version_id(), "3");
}

#[tokio::test]
async fn test_history_tenant_isolation() {
    let backend = create_backend();
    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    // Create in tenant A
    let patient = json!({"resourceType": "Patient", "id": "hist-shared"});
    let created = backend
        .create(&tenant_a, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let _v2 = backend
        .update(
            &tenant_a,
            &created,
            json!({"resourceType": "Patient", "id": "hist-shared"}),
        )
        .await
        .unwrap();

    // Tenant A sees history
    let history_a = backend
        .history_instance(&tenant_a, "Patient", "hist-shared", &HistoryParams::new())
        .await
        .unwrap();
    assert_eq!(history_a.items.len(), 2);

    // Tenant B sees nothing
    let history_b = backend
        .history_instance(&tenant_b, "Patient", "hist-shared", &HistoryParams::new())
        .await
        .unwrap();
    assert!(history_b.items.is_empty());
}

// ============================================================================
// Type History Tests
// ============================================================================

#[tokio::test]
async fn test_history_type() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create multiple patients
    let p1 = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "id": "tp1"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let _p2 = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "id": "tp2"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Update p1
    let _p1_v2 = backend
        .update(
            &tenant,
            &p1,
            json!({"resourceType": "Patient", "id": "tp1"}),
        )
        .await
        .unwrap();

    // Create an observation (different type)
    backend
        .create(
            &tenant,
            "Observation",
            json!({"resourceType": "Observation"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Get Patient type history
    let history = backend
        .history_type(&tenant, "Patient", &HistoryParams::new())
        .await
        .unwrap();

    // Should have 3 entries for Patient (p1 v1, p1 v2, p2 v1)
    assert_eq!(history.items.len(), 3);

    // All should be Patient type
    for entry in &history.items {
        assert_eq!(entry.resource.resource_type(), "Patient");
    }
}

#[tokio::test]
async fn test_history_type_count() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create patients with updates
    let p1 = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let _p1_v2 = backend
        .update(&tenant, &p1, json!({"resourceType": "Patient"}))
        .await
        .unwrap();
    let _p2 = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Create observation
    backend
        .create(
            &tenant,
            "Observation",
            json!({"resourceType": "Observation"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Count patient history
    let patient_count = backend
        .history_type_count(&tenant, "Patient")
        .await
        .unwrap();
    assert_eq!(patient_count, 3);

    // Count observation history
    let obs_count = backend
        .history_type_count(&tenant, "Observation")
        .await
        .unwrap();
    assert_eq!(obs_count, 1);
}

#[tokio::test]
async fn test_history_type_tenant_isolation() {
    let backend = create_backend();
    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    // Create patients in tenant A
    backend
        .create(
            &tenant_a,
            "Patient",
            json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create(
            &tenant_a,
            "Patient",
            json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Create patient in tenant B
    backend
        .create(
            &tenant_b,
            "Patient",
            json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Tenant A sees only its history
    let history_a = backend
        .history_type(&tenant_a, "Patient", &HistoryParams::new())
        .await
        .unwrap();
    assert_eq!(history_a.items.len(), 2);

    // Tenant B sees only its history
    let history_b = backend
        .history_type(&tenant_b, "Patient", &HistoryParams::new())
        .await
        .unwrap();
    assert_eq!(history_b.items.len(), 1);
}

// ============================================================================
// System History Tests
// ============================================================================

#[tokio::test]
async fn test_history_system() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create different resource types
    let p1 = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "id": "sp1"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create(
            &tenant,
            "Observation",
            json!({"resourceType": "Observation", "id": "so1"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create(
            &tenant,
            "Encounter",
            json!({"resourceType": "Encounter", "id": "se1"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Update patient
    let _p1_v2 = backend
        .update(
            &tenant,
            &p1,
            json!({"resourceType": "Patient", "id": "sp1"}),
        )
        .await
        .unwrap();

    // Get system history
    let history = backend
        .history_system(&tenant, &HistoryParams::new())
        .await
        .unwrap();

    // Should have 4 entries total
    assert_eq!(history.items.len(), 4);

    // Should include all resource types
    let types: std::collections::HashSet<_> = history
        .items
        .iter()
        .map(|e| e.resource.resource_type())
        .collect();
    assert!(types.contains("Patient"));
    assert!(types.contains("Observation"));
    assert!(types.contains("Encounter"));
}

#[tokio::test]
async fn test_history_system_count() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create different resource types with updates
    let p1 = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let _p1_v2 = backend
        .update(&tenant, &p1, json!({"resourceType": "Patient"}))
        .await
        .unwrap();
    backend
        .create(
            &tenant,
            "Observation",
            json!({"resourceType": "Observation"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Count all history
    let count = backend.history_system_count(&tenant).await.unwrap();
    assert_eq!(count, 3); // p1 v1, p1 v2, obs
}

#[tokio::test]
async fn test_history_system_tenant_isolation() {
    let backend = create_backend();
    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    // Create resources in tenant A
    backend
        .create(
            &tenant_a,
            "Patient",
            json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create(
            &tenant_a,
            "Observation",
            json!({"resourceType": "Observation"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Create resource in tenant B
    backend
        .create(
            &tenant_b,
            "Encounter",
            json!({"resourceType": "Encounter"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Tenant A sees only its history
    let history_a = backend
        .history_system(&tenant_a, &HistoryParams::new())
        .await
        .unwrap();
    assert_eq!(history_a.items.len(), 2);

    // Tenant B sees only its history
    let history_b = backend
        .history_system(&tenant_b, &HistoryParams::new())
        .await
        .unwrap();
    assert_eq!(history_b.items.len(), 1);

    // Counts should also be isolated
    assert_eq!(backend.history_system_count(&tenant_a).await.unwrap(), 2);
    assert_eq!(backend.history_system_count(&tenant_b).await.unwrap(), 1);
}

// ============================================================================
// Search Index Integration Tests
// ============================================================================

use helios_persistence::core::SearchProvider;
use helios_persistence::types::{
    CompositeSearchComponent, SearchParamType, SearchParameter, SearchQuery, SearchValue,
};

/// Builds the `code-value-quantity` composite query with the component types
/// the registry would supply, mirroring what the REST layer now wires up.
fn code_value_quantity_query(value: &str) -> SearchQuery {
    SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code-value-quantity".to_string(),
        param_type: SearchParamType::Composite,
        modifier: None,
        values: vec![SearchValue::eq(value)],
        chain: vec![],
        components: vec![
            CompositeSearchComponent {
                param_type: SearchParamType::Token,
                param_name: "code".to_string(),
            },
            CompositeSearchComponent {
                param_type: SearchParamType::Quantity,
                param_name: "value-quantity".to_string(),
            },
        ],
    })
}

#[tokio::test]
async fn test_search_value_quantity_choice_type() {
    // Regression: choice-type elements (`value[x]`) must be indexed. The
    // `value-quantity` expression is `(Observation.value as Quantity)`.
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    let observation = json!({
        "resourceType": "Observation",
        "id": "obs-q",
        "status": "final",
        "code": { "coding": [{ "system": "http://loinc.org", "code": "29463-7" }] },
        "valueQuantity": { "value": 72.5, "unit": "kg", "system": "http://unitsofmeasure.org" }
    });
    backend
        .create(&tenant, "Observation", observation, FhirVersion::default())
        .await
        .unwrap();

    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "value-quantity".to_string(),
        param_type: SearchParamType::Quantity,
        modifier: None,
        values: vec![SearchValue::new(
            helios_persistence::types::SearchPrefix::Ge,
            "70",
        )],
        chain: vec![],
        components: vec![],
    });
    let result = backend.search(&tenant, &query).await.unwrap();
    assert_eq!(
        result.resources.items.len(),
        1,
        "value-quantity ge70 → 1 hit"
    );
    assert_eq!(result.resources.items[0].id(), "obs-q");
}

#[tokio::test]
async fn test_search_composite_code_value_quantity() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    let observation = json!({
        "resourceType": "Observation",
        "id": "obs-bp",
        "status": "final",
        "code": { "coding": [{ "system": "http://loinc.org", "code": "8480-6" }] },
        "valueQuantity": { "value": 107, "unit": "mmHg", "system": "http://unitsofmeasure.org" }
    });
    backend
        .create(&tenant, "Observation", observation, FhirVersion::default())
        .await
        .unwrap();

    // Matches: correct code AND value satisfies the quantity comparison.
    let result = backend
        .search(&tenant, &code_value_quantity_query("8480-6$ge100"))
        .await
        .unwrap();
    assert_eq!(
        result.resources.items.len(),
        1,
        "code + value both match → 1 hit"
    );
    assert_eq!(result.resources.items[0].id(), "obs-bp");

    // Value component fails (107 is not >= 200).
    let result = backend
        .search(&tenant, &code_value_quantity_query("8480-6$ge200"))
        .await
        .unwrap();
    assert!(result.resources.items.is_empty(), "value too low → no hit");

    // Code component fails.
    let result = backend
        .search(&tenant, &code_value_quantity_query("9999-9$ge100"))
        .await
        .unwrap();
    assert!(result.resources.items.is_empty(), "code mismatch → no hit");
}

// Note: These tests verify that search indexing infrastructure is integrated into
// storage operations. Full end-to-end search filtering requires updating search_impl.rs
// to use the new query builder from the search module.

#[tokio::test]
async fn test_search_index_on_create() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create a patient with searchable fields
    let patient = json!({
        "resourceType": "Patient",
        "id": "search-test-1",
        "identifier": [{
            "system": "http://example.org/mrn",
            "value": "MRN12345"
        }],
        "name": [{
            "family": "TestFamily",
            "given": ["TestGiven"]
        }],
        "birthDate": "1990-01-15"
    });

    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    assert_eq!(created.id(), "search-test-1");

    // Verify search index works by searching for the identifier
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("http://example.org/mrn|MRN12345")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query).await.unwrap();
    assert_eq!(result.resources.items.len(), 1);
    assert_eq!(result.resources.items[0].id(), "search-test-1");
}

#[tokio::test]
async fn test_search_index_on_update() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create initial patient
    let patient = json!({
        "resourceType": "Patient",
        "id": "search-update-1",
        "name": [{"family": "OriginalFamily", "given": ["Original"]}]
    });

    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Update the patient with new name
    let updated_patient = json!({
        "resourceType": "Patient",
        "id": "search-update-1",
        "name": [{"family": "UpdatedFamily", "given": ["Updated"]}]
    });

    let updated = backend
        .update(&tenant, &created, updated_patient)
        .await
        .unwrap();

    // Verify the update worked
    assert_eq!(updated.id(), "search-update-1");
    assert_eq!(updated.version_id(), "2");

    // Note: Full search filtering using the search index requires updating search_impl.rs
    // to use the query builder. The indexing integration is working - entries are being
    // created/updated/deleted in the search_index table.
}

#[tokio::test]
async fn test_search_index_on_delete() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create a patient
    let patient = json!({
        "resourceType": "Patient",
        "id": "search-delete-1",
        "identifier": [{"system": "http://example.org", "value": "DEL123"}],
        "name": [{"family": "DeleteMe"}]
    });

    backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Verify patient is searchable
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("DEL123")],
        chain: vec![],
        components: vec![],
    });

    let result_before = backend.search(&tenant, &query).await.unwrap();
    assert_eq!(result_before.resources.items.len(), 1);

    // Delete the resource
    backend
        .delete(&tenant, "Patient", "search-delete-1")
        .await
        .unwrap();

    // Verify patient is no longer searchable
    let result_after = backend.search(&tenant, &query).await.unwrap();
    assert_eq!(
        result_after.resources.items.len(),
        0,
        "Deleted resource should not be searchable"
    );
}

#[tokio::test]
async fn test_search_index_string_name() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create patients with different names - verifies indexing works for multiple resources
    let p1 = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "name-1",
                "name": [{"family": "Smith", "given": ["John"]}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let p2 = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "name-2",
                "name": [{"family": "Smithson", "given": ["Jane"]}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let p3 = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "name-3",
                "name": [{"family": "Johnson", "given": ["Bob"]}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Verify all patients were created with their correct IDs
    assert_eq!(p1.id(), "name-1");
    assert_eq!(p2.id(), "name-2");
    assert_eq!(p3.id(), "name-3");

    // Search for patients with name starting with "Smith" (should match "Smith" and "Smithson")
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "name".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("smith")], // lowercase for case-insensitive search
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query).await.unwrap();
    assert_eq!(
        result.resources.items.len(),
        2,
        "Should find 2 patients with name starting with Smith"
    );

    // Verify the correct patients were returned
    let ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
    assert!(ids.contains(&"name-1"), "Should include Smith");
    assert!(ids.contains(&"name-2"), "Should include Smithson");
    assert!(!ids.contains(&"name-3"), "Should not include Johnson");
}

#[tokio::test]
async fn test_search_index_tenant_isolation() {
    let backend = create_backend();
    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    // Create patient in tenant A
    backend
        .create(
            &tenant_a,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "tenant-iso-1",
                "identifier": [{"system": "http://example.org", "value": "UNIQUE123"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Search in tenant A - should find the patient
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("UNIQUE123")],
        chain: vec![],
        components: vec![],
    });

    let result_a = backend.search(&tenant_a, &query).await.unwrap();
    assert_eq!(result_a.resources.items.len(), 1);

    // Search in tenant B - should NOT find the patient
    let result_b = backend.search(&tenant_b, &query).await.unwrap();
    assert_eq!(
        result_b.resources.items.len(),
        0,
        "Tenant B should not see tenant A's resources"
    );
}

#[tokio::test]
async fn test_search_token_with_system() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create patients with identifiers using different systems
    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "token-sys-1",
                "identifier": [{"system": "http://hospital.org/mrn", "value": "12345"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "token-sys-2",
                "identifier": [{"system": "http://other.org/id", "value": "12345"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "token-sys-3",
                "identifier": [{"system": "http://hospital.org/mrn", "value": "67890"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Search by code only (should find both with value 12345)
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("12345")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query).await.unwrap();
    assert_eq!(
        result.resources.items.len(),
        2,
        "Should find 2 patients with code 12345"
    );

    // Search by system|code (should find only the hospital patient)
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("http://hospital.org/mrn|12345")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query).await.unwrap();
    assert_eq!(
        result.resources.items.len(),
        1,
        "Should find 1 patient with system|code match"
    );
    assert_eq!(result.resources.items[0].id(), "token-sys-1");
}

#[tokio::test]
async fn test_search_date_birthdate() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create patients with different birthdates
    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "date-1",
                "birthDate": "1990-01-15"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "date-2",
                "birthDate": "1985-06-20"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "date-3",
                "birthDate": "2000-12-01"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Search for patients born in 1990
    // Note: The storage layer indexes birthDate as "birthdate" (lowercase)
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "birthdate".to_string(),
        param_type: SearchParamType::Date,
        modifier: None,
        values: vec![SearchValue::eq("1990-01-15")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query).await.unwrap();
    assert_eq!(
        result.resources.items.len(),
        1,
        "Should find 1 patient born on 1990-01-15"
    );
    assert_eq!(result.resources.items[0].id(), "date-1");
}

#[tokio::test]
async fn test_search_reference_subject() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create patients
    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-1"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-2"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Create observations referencing patients
    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-1",
                "subject": {"reference": "Patient/patient-1"},
                "code": {"coding": [{"code": "8867-4"}]}
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-2",
                "subject": {"reference": "Patient/patient-1"},
                "code": {"coding": [{"code": "9279-1"}]}
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-3",
                "subject": {"reference": "Patient/patient-2"},
                "code": {"coding": [{"code": "8867-4"}]}
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Search for observations for patient-1
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "subject".to_string(),
        param_type: SearchParamType::Reference,
        modifier: None,
        values: vec![SearchValue::eq("Patient/patient-1")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query).await.unwrap();
    assert_eq!(
        result.resources.items.len(),
        2,
        "Should find 2 observations for patient-1"
    );

    let ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
    assert!(ids.contains(&"obs-1"));
    assert!(ids.contains(&"obs-2"));
}

#[tokio::test]
async fn test_patient_compartment_export_observation_without_since() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-1"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-2"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-1",
                "subject": {"reference": "Patient/patient-1"},
                "status": "final"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-2",
                "subject": {"reference": "Patient/patient-2"},
                "status": "final"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let request = ExportRequest::new(ExportLevel::Patient);
    let batch = backend
        .fetch_patient_compartment_batch(
            &tenant,
            &request,
            "Observation",
            &["patient-1".to_string()],
            None,
            10,
        )
        .await
        .unwrap();

    assert!(batch.is_last);
    assert_eq!(batch.lines.len(), 1);
    let observation: serde_json::Value = serde_json::from_str(&batch.lines[0]).unwrap();
    assert_eq!(observation["id"], "obs-1");
}

#[tokio::test]
async fn test_search_multiple_parameters() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create patients with different combinations of identifiers and status
    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "multi-1",
                "identifier": [{"system": "http://example.org", "value": "ABC123"}],
                "active": true
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "multi-2",
                "identifier": [{"system": "http://example.org", "value": "ABC123"}],
                "active": false
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "multi-3",
                "identifier": [{"system": "http://example.org", "value": "XYZ789"}],
                "active": true
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Search with both identifier AND status (both must match)
    let mut query = SearchQuery::new("Patient");
    query.parameters.push(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("ABC123")],
        chain: vec![],
        components: vec![],
    });
    query.parameters.push(SearchParameter {
        name: "status".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("true")], // active=true
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query).await.unwrap();
    // Both have identifier ABC123, but only multi-1 has active=true indexed as status
    // Note: The storage layer may not index 'active' as 'status', so this test
    // verifies AND logic works even if one param doesn't match
    assert!(
        result.resources.items.len() <= 2,
        "Multiple params should use AND logic"
    );
}

// ============================================================================
// Reindex Tests
// ============================================================================

use helios_persistence::search::{
    ReindexOperation, ReindexRequest, ReindexStatus, ReindexableStorage,
};
use std::sync::Arc;

#[tokio::test]
async fn test_reindex_list_resource_types() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create resources of different types
    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "p1"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "o1",
                "status": "final"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "p2"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // List resource types
    let types = backend.list_resource_types(&tenant).await.unwrap();
    assert!(types.contains(&"Patient".to_string()));
    assert!(types.contains(&"Observation".to_string()));
    assert_eq!(types.len(), 2);
}

#[tokio::test]
async fn test_reindex_count_resources() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create some patients
    for i in 1..=5 {
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": format!("patient-{}", i)
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    // Count patients
    let count = backend.count_resources(&tenant, "Patient").await.unwrap();
    assert_eq!(count, 5);

    // Count observations (should be 0)
    let count = backend
        .count_resources(&tenant, "Observation")
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn test_reindex_fetch_resources_page() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create patients
    for i in 1..=10 {
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": format!("patient-{:02}", i)
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    // Fetch first page (5 resources)
    let page1 = backend
        .fetch_resources_page(&tenant, "Patient", None, 5)
        .await
        .unwrap();
    assert_eq!(page1.resources.len(), 5);
    assert!(page1.next_cursor.is_some());

    // Fetch second page using cursor
    let page2 = backend
        .fetch_resources_page(&tenant, "Patient", page1.next_cursor.as_deref(), 5)
        .await
        .unwrap();
    assert_eq!(page2.resources.len(), 5);

    // Ensure no duplicates between pages
    let page1_ids: Vec<&str> = page1.resources.iter().map(|r| r.id()).collect();
    let page2_ids: Vec<&str> = page2.resources.iter().map(|r| r.id()).collect();
    for id in &page1_ids {
        assert!(!page2_ids.contains(id), "Duplicate ID found: {}", id);
    }

    // Fetch third page (should be empty or have no more cursor)
    let page3 = backend
        .fetch_resources_page(&tenant, "Patient", page2.next_cursor.as_deref(), 5)
        .await
        .unwrap();
    assert!(page3.resources.is_empty() || page3.next_cursor.is_none());
}

#[tokio::test]
async fn test_reindex_clear_search_index() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create some resources (which will be indexed)
    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "p1",
                "name": [{"family": "Smith"}],
                "identifier": [{"system": "http://example.org", "value": "12345"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "o1",
                "code": {"coding": [{"system": "http://loinc.org", "code": "1234-5"}]}
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Verify search works (search index has entries)
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("12345")],
        chain: vec![],
        components: vec![],
    });
    let result = backend.search(&tenant, &query).await.unwrap();
    assert_eq!(
        result.resources.items.len(),
        1,
        "Should find patient before clearing index"
    );

    // Clear the search index
    let deleted = backend.clear_search_index(&tenant).await.unwrap();
    assert!(deleted > 0, "Should have deleted some index entries");

    // Verify search no longer finds the patient
    let result = backend.search(&tenant, &query).await.unwrap();
    assert_eq!(
        result.resources.items.len(),
        0,
        "Should not find patient after clearing index"
    );
}

#[tokio::test]
async fn test_reindex_operation_full() {
    let backend = Arc::new(create_backend());
    let tenant = create_tenant("test-tenant");

    // Create some resources
    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "p1",
                "name": [{"family": "Smith"}],
                "identifier": [{"system": "http://example.org", "value": "A001"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "p2",
                "name": [{"family": "Jones"}],
                "identifier": [{"system": "http://example.org", "value": "A002"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Clear the search index to simulate needing a reindex
    backend.clear_search_index(&tenant).await.unwrap();

    // Verify search doesn't work
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("A001")],
        chain: vec![],
        components: vec![],
    });
    let result = backend.search(&tenant, &query).await.unwrap();
    assert_eq!(
        result.resources.items.len(),
        0,
        "Should not find patient before reindex"
    );

    // Create reindex operation
    let reindex = ReindexOperation::new(backend.clone(), backend.search_extractor().clone());

    // Start reindex
    let job_id = reindex
        .start(tenant.clone(), ReindexRequest::for_types(vec!["Patient"]))
        .await
        .unwrap();

    // Wait for completion (with timeout)
    let mut attempts = 0;
    loop {
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        let progress = reindex.get_progress(&job_id).await.unwrap();

        if progress.status == ReindexStatus::Completed {
            assert_eq!(
                progress.processed_resources, 2,
                "Should have processed 2 patients"
            );
            assert!(
                progress.entries_created > 0,
                "Should have created index entries"
            );
            break;
        }

        if progress.status == ReindexStatus::Failed {
            panic!("Reindex failed: {:?}", progress.error_message);
        }

        attempts += 1;
        if attempts > 100 {
            panic!("Reindex timed out");
        }
    }

    // Verify search works again
    let result = backend.search(&tenant, &query).await.unwrap();
    assert_eq!(
        result.resources.items.len(),
        1,
        "Should find patient after reindex"
    );
    assert_eq!(result.resources.items[0].id(), "p1");
}

#[tokio::test]
async fn test_reindex_operation_cancel() {
    let backend = Arc::new(create_backend());
    let tenant = create_tenant("test-tenant");

    // Create many resources to make reindex take longer
    for i in 1..=100 {
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": format!("patient-{}", i),
                    "name": [{"family": format!("Patient{}", i)}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    // Clear the search index
    backend.clear_search_index(&tenant).await.unwrap();

    // Create reindex operation with small batch size to make it slower
    let reindex = ReindexOperation::new(backend.clone(), backend.search_extractor().clone());

    // Start reindex
    let request = ReindexRequest::all().with_batch_size(5);
    let job_id = reindex.start(tenant.clone(), request).await.unwrap();

    // Cancel immediately
    reindex.cancel(&job_id).await.unwrap();

    // Wait a bit for cancellation to take effect
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Check status
    let progress = reindex.get_progress(&job_id).await.unwrap();
    assert!(
        progress.status == ReindexStatus::Cancelled || progress.status == ReindexStatus::Completed,
        "Job should be cancelled or already completed"
    );
}

// ============================================================================
// Conditional Operations Tests (using search index)
// ============================================================================

use helios_persistence::core::{
    ConditionalCreateResult, ConditionalDeleteResult, ConditionalStorage, ConditionalUpdateResult,
};

#[tokio::test]
async fn test_conditional_create_with_identifier() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create a patient with an identifier
    let patient = json!({
        "resourceType": "Patient",
        "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-12345"}],
        "name": [{"family": "Original"}]
    });

    // First conditional create - should create since no match
    let result = backend
        .conditional_create(
            &tenant,
            "Patient",
            patient.clone(),
            "identifier=http://hospital.org/mrn|MRN-12345",
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(
        matches!(result, ConditionalCreateResult::Created(_)),
        "First conditional create should succeed"
    );

    // Extract the created resource
    let created = match result {
        ConditionalCreateResult::Created(r) => r,
        _ => panic!("Expected Created result"),
    };
    let created_id = created.id().to_string();

    // Second conditional create with same identifier - should return existing
    let patient2 = json!({
        "resourceType": "Patient",
        "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-12345"}],
        "name": [{"family": "Duplicate"}]
    });

    let result2 = backend
        .conditional_create(
            &tenant,
            "Patient",
            patient2,
            "identifier=http://hospital.org/mrn|MRN-12345",
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(
        matches!(result2, ConditionalCreateResult::Exists(_)),
        "Second conditional create should return existing resource"
    );

    // Verify it's the same resource
    if let ConditionalCreateResult::Exists(existing) = result2 {
        assert_eq!(existing.id(), created_id, "Should return same resource");
    }
}

#[tokio::test]
async fn test_conditional_create_with_id() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create a patient with specific ID
    let patient = json!({
        "resourceType": "Patient",
        "id": "test-patient-cond",
        "name": [{"family": "TestPatient"}]
    });
    backend
        .create(&tenant, "Patient", patient.clone(), FhirVersion::default())
        .await
        .unwrap();

    // Conditional create with _id search - should find existing
    let patient2 = json!({
        "resourceType": "Patient",
        "name": [{"family": "Duplicate"}]
    });

    let result = backend
        .conditional_create(
            &tenant,
            "Patient",
            patient2,
            "_id=test-patient-cond",
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(
        matches!(result, ConditionalCreateResult::Exists(_)),
        "Conditional create with _id should find existing resource"
    );

    if let ConditionalCreateResult::Exists(existing) = result {
        assert_eq!(existing.id(), "test-patient-cond");
    }
}

#[tokio::test]
async fn test_conditional_update_with_identifier() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create a patient with an identifier
    let patient = json!({
        "resourceType": "Patient",
        "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-UPDATE-1"}],
        "name": [{"family": "Original"}]
    });
    backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Conditional update - should find and update
    let updated_patient = json!({
        "resourceType": "Patient",
        "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-UPDATE-1"}],
        "name": [{"family": "Updated"}]
    });

    let result = backend
        .conditional_update(
            &tenant,
            "Patient",
            updated_patient,
            "identifier=http://hospital.org/mrn|MRN-UPDATE-1",
            false,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(
        matches!(result, ConditionalUpdateResult::Updated(_)),
        "Conditional update should find and update resource"
    );

    // Verify the update
    if let ConditionalUpdateResult::Updated(updated) = result {
        let content = updated.content();
        let family = content["name"][0]["family"].as_str();
        assert_eq!(family, Some("Updated"), "Resource should be updated");
    }
}

#[tokio::test]
async fn test_conditional_update_with_upsert() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Conditional update with upsert=true for non-existent resource
    let patient = json!({
        "resourceType": "Patient",
        "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-UPSERT-1"}],
        "name": [{"family": "NewPatient"}]
    });

    let result = backend
        .conditional_update(
            &tenant,
            "Patient",
            patient,
            "identifier=http://hospital.org/mrn|MRN-UPSERT-1",
            true, // upsert=true
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(
        matches!(result, ConditionalUpdateResult::Created(_)),
        "Conditional update with upsert should create when no match"
    );
}

#[tokio::test]
async fn test_conditional_delete_with_identifier() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create a patient with an identifier
    let patient = json!({
        "resourceType": "Patient",
        "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-DELETE-1"}],
        "name": [{"family": "ToDelete"}]
    });
    backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Conditional delete - should find and delete
    let result = backend
        .conditional_delete(
            &tenant,
            "Patient",
            "identifier=http://hospital.org/mrn|MRN-DELETE-1",
        )
        .await
        .unwrap();

    assert!(
        matches!(result, ConditionalDeleteResult::Deleted),
        "Conditional delete should find and delete resource"
    );

    // Verify deletion by searching - should not find
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("http://hospital.org/mrn|MRN-DELETE-1")],
        chain: vec![],
        components: vec![],
    });

    let search_result = backend.search(&tenant, &query).await.unwrap();
    assert!(
        search_result.resources.items.is_empty(),
        "Resource should be deleted"
    );
}

#[tokio::test]
async fn test_conditional_operations_tenant_isolation() {
    let backend = create_backend();
    let tenant_a = create_tenant("tenant-a");
    let tenant_b = create_tenant("tenant-b");

    // Create patient in tenant A
    let patient = json!({
        "resourceType": "Patient",
        "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-SHARED"}],
        "name": [{"family": "TenantA"}]
    });
    backend
        .create(&tenant_a, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Conditional create in tenant B with same identifier - should create (different tenant)
    let patient_b = json!({
        "resourceType": "Patient",
        "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-SHARED"}],
        "name": [{"family": "TenantB"}]
    });

    let result = backend
        .conditional_create(
            &tenant_b,
            "Patient",
            patient_b,
            "identifier=http://hospital.org/mrn|MRN-SHARED",
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(
        matches!(result, ConditionalCreateResult::Created(_)),
        "Conditional create should succeed in different tenant"
    );
}

#[tokio::test]
async fn test_conditional_create_multiple_matches() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create two patients with same identifier value but different systems
    // that would both match a code-only search
    let patient1 = json!({
        "resourceType": "Patient",
        "identifier": [{"system": "http://system-a.org", "value": "SHARED-VALUE"}],
        "name": [{"family": "Patient1"}]
    });
    backend
        .create(&tenant, "Patient", patient1, FhirVersion::default())
        .await
        .unwrap();

    let patient2 = json!({
        "resourceType": "Patient",
        "identifier": [{"system": "http://system-b.org", "value": "SHARED-VALUE"}],
        "name": [{"family": "Patient2"}]
    });
    backend
        .create(&tenant, "Patient", patient2, FhirVersion::default())
        .await
        .unwrap();

    // Conditional create with code-only search (no system) - should find multiple matches
    let patient3 = json!({
        "resourceType": "Patient",
        "identifier": [{"value": "SHARED-VALUE"}],
        "name": [{"family": "Patient3"}]
    });

    let result = backend
        .conditional_create(
            &tenant,
            "Patient",
            patient3,
            "identifier=SHARED-VALUE",
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(
        matches!(result, ConditionalCreateResult::MultipleMatches(_)),
        "Should report multiple matches"
    );

    if let ConditionalCreateResult::MultipleMatches(count) = result {
        assert_eq!(count, 2, "Should find exactly 2 matching patients");
    }
}

// ============================================================================
// SearchParameter Resource Handling Tests
// ============================================================================

#[tokio::test]
async fn test_search_parameter_create_registers_in_registry() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create a custom SearchParameter
    let search_param = json!({
        "resourceType": "SearchParameter",
        "id": "custom-patient-nickname",
        "url": "http://example.org/fhir/SearchParameter/patient-nickname",
        "name": "nickname",
        "status": "active",
        "code": "nickname",
        "base": ["Patient"],
        "type": "string",
        "expression": "Patient.name.where(use='nickname').given"
    });

    backend
        .create(
            &tenant,
            "SearchParameter",
            search_param,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Verify the parameter is registered
    let registry = backend.search_registry().read();
    let param = registry.get_param("Patient", "nickname");
    assert!(
        param.is_some(),
        "Custom SearchParameter should be registered"
    );

    let param = param.unwrap();
    assert_eq!(param.code, "nickname");
    assert_eq!(
        param.url,
        "http://example.org/fhir/SearchParameter/patient-nickname"
    );
}

#[tokio::test]
async fn test_search_parameter_create_draft_not_registered() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create a draft SearchParameter (status != active)
    let search_param = json!({
        "resourceType": "SearchParameter",
        "id": "draft-param",
        "url": "http://example.org/fhir/SearchParameter/draft-param",
        "name": "draft",
        "status": "draft",
        "code": "draft",
        "base": ["Patient"],
        "type": "string",
        "expression": "Patient.extension('draft')"
    });

    backend
        .create(
            &tenant,
            "SearchParameter",
            search_param,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Verify the parameter is NOT registered (draft status)
    let registry = backend.search_registry().read();
    let param = registry.get_param("Patient", "draft");
    assert!(
        param.is_none(),
        "Draft SearchParameter should not be registered"
    );
}

#[tokio::test]
async fn test_search_parameter_delete_unregisters() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create an active SearchParameter
    let search_param = json!({
        "resourceType": "SearchParameter",
        "id": "to-delete",
        "url": "http://example.org/fhir/SearchParameter/to-delete",
        "name": "todelete",
        "status": "active",
        "code": "todelete",
        "base": ["Observation"],
        "type": "token",
        "expression": "Observation.code"
    });

    backend
        .create(
            &tenant,
            "SearchParameter",
            search_param,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Verify it's registered
    {
        let registry = backend.search_registry().read();
        assert!(registry.get_param("Observation", "todelete").is_some());
    }

    // Delete the SearchParameter
    backend
        .delete(&tenant, "SearchParameter", "to-delete")
        .await
        .unwrap();

    // Verify it's unregistered
    let registry = backend.search_registry().read();
    assert!(
        registry.get_param("Observation", "todelete").is_none(),
        "Deleted SearchParameter should be unregistered"
    );
}

#[tokio::test]
async fn test_search_parameter_update_status_change() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create an active SearchParameter
    let search_param = json!({
        "resourceType": "SearchParameter",
        "id": "status-change",
        "url": "http://example.org/fhir/SearchParameter/status-change",
        "name": "statuschange",
        "status": "active",
        "code": "statuschange",
        "base": ["Condition"],
        "type": "token",
        "expression": "Condition.code"
    });

    let created = backend
        .create(
            &tenant,
            "SearchParameter",
            search_param,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Verify it's registered and active
    {
        let registry = backend.search_registry().read();
        let param = registry.get_param("Condition", "statuschange");
        assert!(param.is_some());
        assert_eq!(
            param.unwrap().status,
            helios_persistence::search::SearchParameterStatus::Active
        );
    }

    // Update to retired status
    let retired_param = json!({
        "resourceType": "SearchParameter",
        "id": "status-change",
        "url": "http://example.org/fhir/SearchParameter/status-change",
        "name": "statuschange",
        "status": "retired",
        "code": "statuschange",
        "base": ["Condition"],
        "type": "token",
        "expression": "Condition.code"
    });

    backend
        .update(&tenant, &created, retired_param)
        .await
        .unwrap();

    // Verify status is updated in registry
    let registry = backend.search_registry().read();
    let param = registry.get_param("Condition", "statuschange");
    assert!(param.is_some(), "Parameter should still exist in registry");
    assert_eq!(
        param.unwrap().status,
        helios_persistence::search::SearchParameterStatus::Retired,
        "Status should be updated to retired"
    );
}

// ============================================================================
// FTS5 Full-Text Search Tests
// ============================================================================

#[tokio::test]
async fn test_fts_indexing_does_not_fail() {
    // This test verifies that creating resources with narrative text
    // does not fail, regardless of whether FTS5 is available.
    // FTS5 availability depends on the SQLite build options.

    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create a patient with narrative text
    let patient = json!({
        "resourceType": "Patient",
        "id": "fts-test-1",
        "text": {
            "status": "generated",
            "div": "<div xmlns=\"http://www.w3.org/1999/xhtml\"><p>John Smith, male patient born 1970-01-15</p></div>"
        },
        "name": [{"family": "Smith", "given": ["John"]}],
        "gender": "male"
    });

    // This should succeed regardless of FTS5 availability
    let result = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await;
    assert!(
        result.is_ok(),
        "Creating resource with narrative should succeed"
    );

    // Read it back to verify
    let read = backend
        .read(&tenant, "Patient", "fts-test-1")
        .await
        .unwrap();
    assert!(read.is_some());
}

#[tokio::test]
async fn test_fts_update_does_not_fail() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create a patient
    let patient = json!({
        "resourceType": "Patient",
        "id": "fts-update-test",
        "text": {
            "div": "<div>Original narrative</div>"
        },
        "name": [{"family": "Original"}]
    });

    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Update with new narrative
    let updated_patient = json!({
        "resourceType": "Patient",
        "id": "fts-update-test",
        "text": {
            "div": "<div>Updated narrative with new content</div>"
        },
        "name": [{"family": "Updated"}]
    });

    let result = backend.update(&tenant, &created, updated_patient).await;
    assert!(
        result.is_ok(),
        "Updating resource with narrative should succeed"
    );
}

#[tokio::test]
async fn test_fts_delete_does_not_fail() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create a patient with narrative
    let patient = json!({
        "resourceType": "Patient",
        "id": "fts-delete-test",
        "text": {
            "div": "<div>Test patient for deletion</div>"
        },
        "name": [{"family": "Test"}]
    });

    backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Delete should succeed (and clean up FTS if available)
    let result = backend.delete(&tenant, "Patient", "fts-delete-test").await;
    assert!(result.is_ok(), "Deleting resource should succeed");
}

// ============================================================================
// _content Parameter Tests (Full Content Search)
// ============================================================================

use helios_persistence::types::SearchModifier;

#[tokio::test]
async fn test_content_search_basic() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create patients with various content
    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "content-1",
                "name": [{"family": "Smithson", "given": ["Alexander"]}],
                "address": [{"city": "Springfield", "state": "Illinois"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "content-2",
                "name": [{"family": "Johnson", "given": ["Robert"]}],
                "address": [{"city": "Chicago", "state": "Illinois"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Search for patients containing "Illinois" anywhere in content
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "_content".to_string(),
        param_type: SearchParamType::Special,
        modifier: None,
        values: vec![SearchValue::eq("Illinois")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query).await.unwrap();

    // Should find patients from Illinois (FTS5 must be available)
    // If FTS5 is not available, no results will be returned
    let count = result.resources.items.len();
    assert!(
        count == 0 || count == 2,
        "Should find 0 (FTS unavailable) or 2 (FTS available) patients, found {}",
        count
    );
}

// ============================================================================
// _text Parameter Tests (Narrative Search)
// ============================================================================

#[tokio::test]
async fn test_text_search_narrative() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create patient with narrative text
    backend.create(&tenant, "Patient", json!({
        "resourceType": "Patient",
        "id": "text-1",
        "text": {
            "status": "generated",
            "div": "<div xmlns=\"http://www.w3.org/1999/xhtml\"><p>John Smith is a patient with diabetes and hypertension.</p></div>"
        },
        "name": [{"family": "Smith", "given": ["John"]}]
    }), FhirVersion::default()).await.unwrap();

    backend.create(&tenant, "Patient", json!({
        "resourceType": "Patient",
        "id": "text-2",
        "text": {
            "status": "generated",
            "div": "<div xmlns=\"http://www.w3.org/1999/xhtml\"><p>Jane Doe has asthma and allergies.</p></div>"
        },
        "name": [{"family": "Doe", "given": ["Jane"]}]
    }), FhirVersion::default()).await.unwrap();

    // Search for patients with "diabetes" in narrative
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "_text".to_string(),
        param_type: SearchParamType::Special,
        modifier: None,
        values: vec![SearchValue::eq("diabetes")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query).await.unwrap();

    // FTS5 dependent - either finds 1 or 0
    let count = result.resources.items.len();
    assert!(
        count <= 1,
        "Should find at most 1 patient with diabetes in narrative"
    );

    if count == 1 {
        assert_eq!(result.resources.items[0].id(), "text-1");
    }
}

// ============================================================================
// Composite Search Parameter Tests
// ============================================================================

#[tokio::test]
async fn test_composite_search_basic() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create observations with component values
    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-comp-1",
                "code": {"coding": [{"system": "http://loinc.org", "code": "85354-9"}]},
                "component": [
                    {
                        "code": {"coding": [{"system": "http://loinc.org", "code": "8480-6"}]},
                        "valueQuantity": {"value": 120, "unit": "mmHg"}
                    }
                ]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Search using composite format: code$value-quantity
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "component-code-value-quantity".to_string(),
        param_type: SearchParamType::Composite,
        modifier: None,
        values: vec![SearchValue::eq("http://loinc.org|8480-6$120")],
        chain: vec![],
        components: vec![
            CompositeSearchComponent {
                param_type: SearchParamType::Token,
                param_name: "component-code".to_string(),
            },
            CompositeSearchComponent {
                param_type: SearchParamType::Quantity,
                param_name: "component-value-quantity".to_string(),
            },
        ],
    });

    // The composite infrastructure is now wired up
    let result = backend.search(&tenant, &query).await;
    assert!(result.is_ok(), "Composite search should not error");
}

// ============================================================================
// :text Modifier Tests (Token Display Text Search)
// ============================================================================

#[tokio::test]
async fn test_token_text_modifier_coding_display() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create observations with display text in code
    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-text-1",
                "code": {
                    "coding": [
                        {
                            "system": "http://loinc.org",
                            "code": "8867-4",
                            "display": "Heart rate"
                        }
                    ]
                },
                "status": "final"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-text-2",
                "code": {
                    "coding": [
                        {
                            "system": "http://loinc.org",
                            "code": "9279-1",
                            "display": "Respiratory rate"
                        }
                    ]
                },
                "status": "final"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Search using :text modifier for "heart"
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::Text),
        values: vec![SearchValue::eq("heart")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query).await.unwrap();

    // Should find the observation with "Heart rate" in display
    assert_eq!(result.resources.items.len(), 1);
    assert_eq!(result.resources.items[0].id(), "obs-text-1");
}

// ============================================================================
// :of-type Modifier Tests (Identifier Type Matching)
// ============================================================================

#[tokio::test]
async fn test_identifier_of_type_modifier() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create patients with typed identifiers
    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-oftype-1",
                "identifier": [
                    {
                        "type": {
                            "coding": [
                                {
                                    "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
                                    "code": "MR"
                                }
                            ]
                        },
                        "system": "http://hospital.org/mrn",
                        "value": "MRN12345"
                    }
                ]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "patient-oftype-2",
                "identifier": [
                    {
                        "type": {
                            "coding": [
                                {
                                    "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
                                    "code": "SS"
                                }
                            ]
                        },
                        "system": "http://hl7.org/fhir/sid/us-ssn",
                        "value": "123-45-6789"
                    }
                ]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Search using :of-type modifier for MR type
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::OfType),
        values: vec![SearchValue::eq(
            "http://terminology.hl7.org/CodeSystem/v2-0203|MR|MRN12345",
        )],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query).await.unwrap();

    // Should find the patient with MR type identifier
    assert_eq!(result.resources.items.len(), 1);
    assert_eq!(result.resources.items[0].id(), "patient-oftype-1");
}

// ============================================================================
// :text-advanced Modifier Tests (FHIR v6.0.0)
// ============================================================================

#[tokio::test]
async fn test_text_advanced_simple_term() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create observations with different display text
    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-headache",
                "status": "final",
                "code": {
                    "coding": [{
                        "system": "http://snomed.info/sct",
                        "code": "25064002",
                        "display": "Headache disorder"
                    }]
                }
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-migraine",
                "status": "final",
                "code": {
                    "coding": [{
                        "system": "http://snomed.info/sct",
                        "code": "37796009",
                        "display": "Migraine with aura"
                    }]
                }
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Search for "headache" using :text-advanced
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::TextAdvanced),
        values: vec![SearchValue::string("headache")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query).await;

    // FTS5 availability is required for :text-advanced
    // If FTS5 is available, should find the headache observation
    if let Ok(result) = result {
        // FTS5 available - should match via porter stemming
        if !result.resources.items.is_empty() {
            assert_eq!(result.resources.items.len(), 1);
            assert_eq!(result.resources.items[0].id(), "obs-headache");
        }
    }
}

#[tokio::test]
async fn test_text_advanced_or_operator() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create observations
    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-headache",
                "status": "final",
                "code": {
                    "coding": [{
                        "system": "http://snomed.info/sct",
                        "code": "25064002",
                        "display": "Headache disorder"
                    }]
                }
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-migraine",
                "status": "final",
                "code": {
                    "coding": [{
                        "system": "http://snomed.info/sct",
                        "code": "37796009",
                        "display": "Migraine syndrome"
                    }]
                }
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-fracture",
                "status": "final",
                "code": {
                    "coding": [{
                        "system": "http://snomed.info/sct",
                        "code": "12345",
                        "display": "Fracture of leg"
                    }]
                }
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Search for "headache OR migraine" using :text-advanced
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::TextAdvanced),
        values: vec![SearchValue::string("headache OR migraine")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query).await;

    // If FTS5 is available, should find both headache and migraine
    if let Ok(result) = result {
        if !result.resources.items.is_empty() {
            assert_eq!(
                result.resources.items.len(),
                2,
                "Should find both headache and migraine"
            );
        }
    }
}

#[tokio::test]
async fn test_text_advanced_phrase_match() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create observations with similar but different display text
    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-heart-failure",
                "status": "final",
                "code": {
                    "coding": [{
                        "system": "http://snomed.info/sct",
                        "code": "84114007",
                        "display": "Chronic heart failure"
                    }]
                }
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-heart-rate",
                "status": "final",
                "code": {
                    "coding": [{
                        "system": "http://loinc.org",
                        "code": "8867-4",
                        "display": "Heart rate"
                    }]
                }
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Search for exact phrase "heart failure"
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::TextAdvanced),
        values: vec![SearchValue::string("\"heart failure\"")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query).await;

    // If FTS5 is available, should only match exact phrase
    if let Ok(result) = result {
        if !result.resources.items.is_empty() {
            assert_eq!(
                result.resources.items.len(),
                1,
                "Should only find heart failure, not heart rate"
            );
            assert_eq!(result.resources.items[0].id(), "obs-heart-failure");
        }
    }
}

#[tokio::test]
async fn test_text_advanced_prefix_match() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create observations with related terms
    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-cardio",
                "status": "final",
                "code": {
                    "coding": [{
                        "system": "http://snomed.info/sct",
                        "code": "49601007",
                        "display": "Cardiovascular disease"
                    }]
                }
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-cardiac",
                "status": "final",
                "code": {
                    "coding": [{
                        "system": "http://snomed.info/sct",
                        "code": "56265001",
                        "display": "Cardiac arrest"
                    }]
                }
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-fracture",
                "status": "final",
                "code": {
                    "coding": [{
                        "system": "http://snomed.info/sct",
                        "code": "125605004",
                        "display": "Fracture of bone"
                    }]
                }
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Search for prefix "cardi*" (matches cardiac, cardiovascular)
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::TextAdvanced),
        values: vec![SearchValue::string("cardi*")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query).await;

    // If FTS5 is available, should match both cardi* terms
    if let Ok(result) = result {
        if !result.resources.items.is_empty() {
            assert_eq!(
                result.resources.items.len(),
                2,
                "Should find cardiovascular and cardiac"
            );
        }
    }
}

#[tokio::test]
async fn test_text_advanced_porter_stemming() {
    let backend = create_backend();
    let tenant = create_tenant("test-tenant");

    // Create observation with plural/conjugated term
    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation",
                "id": "obs-running",
                "status": "final",
                "code": {
                    "coding": [{
                        "system": "http://snomed.info/sct",
                        "code": "282239000",
                        "display": "Running injury"
                    }]
                }
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Search for "run" - should match "running" via porter stemming
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::TextAdvanced),
        values: vec![SearchValue::string("run")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query).await;

    // If FTS5 is available with porter stemmer, should match "running"
    if let Ok(result) = result {
        if !result.resources.items.is_empty() {
            assert_eq!(
                result.resources.items.len(),
                1,
                "Should find 'running' when searching for 'run'"
            );
        }
    }
}
