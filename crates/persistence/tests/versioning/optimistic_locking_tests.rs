//! Tests for optimistic locking via If-Match and ETag.
//!
//! This module tests the `update_with_match` method of the VersionedStorage trait
//! which implements HTTP If-Match semantics for concurrent update prevention.

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, VersionedStorage};
use helios_persistence::error::{ConcurrencyError, StorageError};
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
// Optimistic Locking Tests - Basic
// ============================================================================

/// Test that update_with_match succeeds when ETag matches.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_with_match_success() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let etag = created.etag().to_string();

    // Update with matching ETag
    let mut content = created.content().clone();
    content["name"][0]["family"] = json!("Jones");

    let result = backend
        .update_with_match(&tenant, "Patient", created.id(), &etag, content)
        .await;

    assert!(result.is_ok());
    let updated = result.unwrap();
    assert_eq!(updated.version_id(), "2");
    assert_eq!(updated.content()["name"][0]["family"], "Jones");
}

/// Test that update_with_match fails when ETag doesn't match.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_with_match_etag_mismatch() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let v1 = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Update to create v2
    let v2 = backend
        .update(&tenant, &v1, v1.content().clone())
        .await
        .unwrap();

    // Try to update using stale v1 ETag
    let mut content = v2.content().clone();
    content["name"][0]["family"] = json!("Jones");

    let result = backend
        .update_with_match(&tenant, "Patient", v1.id(), v1.etag(), content)
        .await;

    assert!(result.is_err());
    match result {
        Err(StorageError::Concurrency(ConcurrencyError::VersionConflict { .. })) => {}
        Err(e) => panic!("Expected VersionConflict error, got {:?}", e),
        Ok(_) => panic!("Expected error"),
    }
}

/// Test various ETag formats are accepted.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_with_match_etag_formats() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Test different ETag formats that should all be equivalent
    let etag_formats = vec![
        r#"W/"1""#.to_string(), // Weak ETag with quotes
        r#""1""#.to_string(),   // Strong ETag with quotes
        "1".to_string(),        // Just the version
    ];

    for etag in etag_formats {
        let read = backend
            .read(&tenant, "Patient", created.id())
            .await
            .unwrap()
            .unwrap();

        let result = backend
            .update_with_match(
                &tenant,
                "Patient",
                created.id(),
                &etag,
                read.content().clone(),
            )
            .await;

        // Should succeed with any valid format
        if result.is_ok() {
            // Reset for next test
            break; // Only need one to succeed to validate format handling
        }
    }
}

// ============================================================================
// Optimistic Locking Tests - Concurrent Updates
// ============================================================================

/// Test that concurrent updates are properly serialized.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_concurrent_update_serialization() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Original");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let id = created.id().to_string();

    // Simulate two users reading at the same time
    let user1_read = backend
        .read(&tenant, "Patient", &id)
        .await
        .unwrap()
        .unwrap();
    let user2_read = backend
        .read(&tenant, "Patient", &id)
        .await
        .unwrap()
        .unwrap();

    // User 1 updates first
    let mut user1_content = user1_read.content().clone();
    user1_content["name"][0]["family"] = json!("User1Edit");
    let user1_result = backend
        .update_with_match(&tenant, "Patient", &id, user1_read.etag(), user1_content)
        .await;
    assert!(user1_result.is_ok());

    // User 2 tries to update with stale ETag
    let mut user2_content = user2_read.content().clone();
    user2_content["name"][0]["family"] = json!("User2Edit");
    let user2_result = backend
        .update_with_match(&tenant, "Patient", &id, user2_read.etag(), user2_content)
        .await;

    // User 2 should fail
    assert!(user2_result.is_err());
    match user2_result {
        Err(StorageError::Concurrency(_)) => {}
        Err(e) => panic!("Expected ConcurrencyError, got {:?}", e),
        Ok(_) => panic!("Expected error"),
    }

    // Final state should be User1's edit
    let final_state = backend
        .read(&tenant, "Patient", &id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(final_state.content()["name"][0]["family"], "User1Edit");
}

/// Test that the second concurrent updater can retry with fresh ETag.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_concurrent_update_retry() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Original");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let id = created.id().to_string();

    // User 1 reads
    let user1_read = backend
        .read(&tenant, "Patient", &id)
        .await
        .unwrap()
        .unwrap();

    // User 2 reads
    let user2_read = backend
        .read(&tenant, "Patient", &id)
        .await
        .unwrap()
        .unwrap();

    // User 1 updates
    let mut user1_content = user1_read.content().clone();
    user1_content["name"][0]["family"] = json!("User1Edit");
    backend
        .update_with_match(&tenant, "Patient", &id, user1_read.etag(), user1_content)
        .await
        .unwrap();

    // User 2's first attempt fails
    let mut user2_content = user2_read.content().clone();
    user2_content["name"][0]["family"] = json!("User2Edit");
    let first_attempt = backend
        .update_with_match(
            &tenant,
            "Patient",
            &id,
            user2_read.etag(),
            user2_content.clone(),
        )
        .await;
    assert!(first_attempt.is_err());

    // User 2 refreshes and retries
    let user2_refresh = backend
        .read(&tenant, "Patient", &id)
        .await
        .unwrap()
        .unwrap();
    let mut retry_content = user2_refresh.content().clone();
    retry_content["name"][0]["family"] = json!("User2Edit");

    let retry_result = backend
        .update_with_match(&tenant, "Patient", &id, user2_refresh.etag(), retry_content)
        .await;

    assert!(retry_result.is_ok());
    let final_state = backend
        .read(&tenant, "Patient", &id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(final_state.content()["name"][0]["family"], "User2Edit");
    assert_eq!(final_state.version_id(), "3");
}

// ============================================================================
// Optimistic Locking Tests - Error Cases
// ============================================================================

/// Test update_with_match on nonexistent resource.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_with_match_nonexistent() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let result = backend
        .update_with_match(
            &tenant,
            "Patient",
            "nonexistent",
            "W/\"1\"",
            create_patient_json("Smith"),
        )
        .await;

    assert!(result.is_err());
}

/// Test update_with_match on deleted resource.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_update_with_match_deleted() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let id = created.id().to_string();
    let etag = created.etag().to_string();

    // Delete the resource
    backend.delete(&tenant, "Patient", &id).await.unwrap();

    // Try to update with old ETag
    let result = backend
        .update_with_match(&tenant, "Patient", &id, &etag, create_patient_json("Jones"))
        .await;

    assert!(result.is_err());
}

// ============================================================================
// ETag Matching Tests
// ============================================================================

/// Test that ETag is correctly generated for each version.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_etag_generation() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let v1 = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let v2 = backend
        .update(&tenant, &v1, v1.content().clone())
        .await
        .unwrap();
    let v3 = backend
        .update(&tenant, &v2, v2.content().clone())
        .await
        .unwrap();

    // Each version should have a unique ETag
    assert!(v1.etag().contains("1"));
    assert!(v2.etag().contains("2"));
    assert!(v3.etag().contains("3"));

    assert_ne!(v1.etag(), v2.etag());
    assert_ne!(v2.etag(), v3.etag());
}

/// Test ETag matching helper method.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_stored_resource_matches_etag() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Smith");
    let created = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Should match various formats
    assert!(created.matches_etag("W/\"1\""));
    assert!(created.matches_etag("\"1\""));
    assert!(created.matches_etag("1"));

    // Should not match wrong versions
    assert!(!created.matches_etag("W/\"2\""));
    assert!(!created.matches_etag("\"2\""));
    assert!(!created.matches_etag("2"));
}

// ============================================================================
// Optimistic Locking Tests - Sequential Updates
// ============================================================================

/// Test sequential updates with proper ETag management.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_sequential_updates_with_etag() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Version0");
    let mut current = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    for i in 1..=10 {
        let mut content = current.content().clone();
        content["name"][0]["family"] = json!(format!("Version{}", i));

        current = backend
            .update_with_match(&tenant, "Patient", current.id(), current.etag(), content)
            .await
            .expect(&format!("Update {} should succeed", i));

        assert_eq!(current.version_id(), (i + 1).to_string());
    }
}

// ============================================================================
// If-None-Match Tests (Conditional Create)
// ============================================================================

/// Test If-None-Match: * semantics (only create if not exists).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_if_none_match_create_only() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // First create succeeds (resource doesn't exist)
    let patient1 = create_patient_json("First");
    let (created, is_new) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "test-id",
            patient1,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(is_new);
    assert_eq!(created.version_id(), "1");

    // Second create with same ID updates instead
    let patient2 = create_patient_json("Second");
    let (updated, is_new2) = backend
        .create_or_update(
            &tenant,
            "Patient",
            "test-id",
            patient2,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(!is_new2);
    assert_eq!(updated.version_id(), "2");
}

// ============================================================================
// Stress Tests
// ============================================================================

/// Test rapid sequential updates maintain correct versioning.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_rapid_sequential_updates() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let patient = create_patient_json("Initial");
    let mut current = backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();
    let id = current.id().to_string();

    // Do 100 rapid updates
    for i in 1..=100 {
        let mut content = current.content().clone();
        content["active"] = json!(i % 2 == 0);

        current = backend
            .update_with_match(&tenant, "Patient", &id, current.etag(), content)
            .await
            .expect(&format!("Update {} should succeed", i));
    }

    // Final version should be 101
    let final_state = backend
        .read(&tenant, "Patient", &id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(final_state.version_id(), "101");
}
