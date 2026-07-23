//! Tests for basic transaction operations.
//!
//! This module tests single transactions including commit and abort
//! scenarios with isolation guarantees.

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{
    ResourceStorage, Transaction, TransactionOptions, TransactionProvider,
};
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
// Basic Commit Tests
// ============================================================================

/// Test that a committed transaction persists changes.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_transaction_commit() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Start transaction
    let mut tx = backend
        .begin_transaction(&tenant, TransactionOptions::new())
        .await
        .unwrap();

    // Create resource within transaction
    let patient = json!({
        "resourceType": "Patient",
        "name": [{"family": "TransactionTest"}]
    });
    let created = tx.create("Patient", patient).await.unwrap();

    // Commit transaction
    Box::new(tx).commit().await.unwrap();

    // Resource should be visible after commit
    let read = backend
        .read(&tenant, "Patient", created.id())
        .await
        .unwrap();
    assert!(read.is_some());
    assert_eq!(
        read.unwrap().content()["name"][0]["family"],
        "TransactionTest"
    );
}

/// Test multiple operations in a single transaction.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_transaction_multiple_operations() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let mut tx = backend
        .begin_transaction(&tenant, TransactionOptions::new())
        .await
        .unwrap();

    // Create multiple resources
    let patient1 = json!({"resourceType": "Patient", "name": [{"family": "First"}]});
    let patient2 = json!({"resourceType": "Patient", "name": [{"family": "Second"}]});
    let patient3 = json!({"resourceType": "Patient", "name": [{"family": "Third"}]});

    let p1 = tx.create("Patient", patient1).await.unwrap();
    let p2 = tx.create("Patient", patient2).await.unwrap();
    let p3 = tx.create("Patient", patient3).await.unwrap();

    // Capture IDs before consuming the transaction on commit.
    let p1_id = p1.id().to_string();
    let p2_id = p2.id().to_string();
    let p3_id = p3.id().to_string();

    Box::new(tx).commit().await.unwrap();

    // All should be visible
    assert!(backend.exists(&tenant, "Patient", &p1_id).await.unwrap());
    assert!(backend.exists(&tenant, "Patient", &p2_id).await.unwrap());
    assert!(backend.exists(&tenant, "Patient", &p3_id).await.unwrap());

    let count = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count, 3);
}

/// Test create and update in same transaction.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_transaction_create_then_update() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let mut tx = backend
        .begin_transaction(&tenant, TransactionOptions::new())
        .await
        .unwrap();

    // Create
    let patient = json!({
        "resourceType": "Patient",
        "name": [{"family": "Original"}]
    });
    let created = tx.create("Patient", patient).await.unwrap();

    // Update within same transaction
    let updated_content = json!({
        "resourceType": "Patient",
        "name": [{"family": "Updated"}]
    });
    tx.update(&created, updated_content).await.unwrap();

    let created_id = created.id().to_string();
    Box::new(tx).commit().await.unwrap();

    // Should see updated value
    let read = backend
        .read(&tenant, "Patient", &created_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.content()["name"][0]["family"], "Updated");
}

/// Test create and delete in same transaction.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_transaction_create_then_delete() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let mut tx = backend
        .begin_transaction(&tenant, TransactionOptions::new())
        .await
        .unwrap();

    // Create
    let patient = json!({"resourceType": "Patient"});
    let created = tx.create("Patient", patient).await.unwrap();

    // Delete in same transaction
    tx.delete("Patient", created.id()).await.unwrap();

    let created_id = created.id().to_string();
    Box::new(tx).commit().await.unwrap();

    // Should not exist
    assert!(
        !backend
            .exists(&tenant, "Patient", &created_id)
            .await
            .unwrap()
    );
}

// ============================================================================
// Abort/Rollback Tests
// ============================================================================

/// Test that an aborted transaction does not persist changes.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_transaction_abort() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let mut tx = backend
        .begin_transaction(&tenant, TransactionOptions::new())
        .await
        .unwrap();

    let patient = json!({
        "resourceType": "Patient",
        "name": [{"family": "ShouldNotExist"}]
    });
    let created = tx.create("Patient", patient).await.unwrap();
    let created_id = created.id().to_string();

    // Abort instead of commit
    Box::new(tx).rollback().await.unwrap();

    // Resource should NOT exist
    let read = backend.read(&tenant, "Patient", &created_id).await.unwrap();
    assert!(read.is_none());
}

/// Test that abort rolls back multiple operations.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_transaction_abort_multiple() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create a resource outside transaction
    let existing = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "name": [{"family": "Existing"}]}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let mut tx = backend
        .begin_transaction(&tenant, TransactionOptions::new())
        .await
        .unwrap();

    // Create new resource
    let new_patient = json!({"resourceType": "Patient", "name": [{"family": "New"}]});
    let new_created = tx.create("Patient", new_patient).await.unwrap();
    let new_id = new_created.id().to_string();

    // Update existing resource
    tx.update(
        &existing,
        json!({"resourceType": "Patient", "name": [{"family": "Modified"}]}),
    )
    .await
    .unwrap();

    // Abort
    Box::new(tx).rollback().await.unwrap();

    // New resource should not exist
    assert!(!backend.exists(&tenant, "Patient", &new_id).await.unwrap());

    // Existing should be unchanged
    let read = backend
        .read(&tenant, "Patient", existing.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.content()["name"][0]["family"], "Existing");
}

// ============================================================================
// Isolation Tests
// ============================================================================

/// Test that uncommitted changes are not visible outside transaction.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_transaction_isolation_uncommitted() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let mut tx = backend
        .begin_transaction(&tenant, TransactionOptions::new())
        .await
        .unwrap();

    let patient = json!({"resourceType": "Patient"});
    tx.create("Patient", patient).await.unwrap();

    // Outside transaction, resource should not be visible (depending on isolation level)
    // Note: This depends on the backend's isolation level implementation
    let count_outside = backend.count(&tenant, Some("Patient")).await.unwrap();

    Box::new(tx).commit().await.unwrap();

    // After commit, should be visible
    let count_after = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count_after, count_outside + 1);
}

/// Test transaction isolation between tenants.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_transaction_tenant_isolation() {
    let backend = create_sqlite_backend();
    let tenant_a = TenantContext::new(TenantId::new("tenant-a"), TenantPermissions::full_access());
    let tenant_b = TenantContext::new(TenantId::new("tenant-b"), TenantPermissions::full_access());

    // Transaction for tenant A
    let mut tx_a = backend
        .begin_transaction(&tenant_a, TransactionOptions::new())
        .await
        .unwrap();

    let patient = json!({"resourceType": "Patient", "name": [{"family": "TenantA"}]});
    let created = tx_a.create("Patient", patient).await.unwrap();
    let created_id = created.id().to_string();

    Box::new(tx_a).commit().await.unwrap();

    // Tenant A can see it
    assert!(
        backend
            .exists(&tenant_a, "Patient", &created_id)
            .await
            .unwrap()
    );

    // Tenant B cannot
    assert!(
        !backend
            .exists(&tenant_b, "Patient", &created_id)
            .await
            .unwrap()
    );
}

// ============================================================================
// Error Handling Tests
// ============================================================================

/// Test that errors within transaction can be recovered from.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_transaction_error_recovery() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let mut tx = backend
        .begin_transaction(&tenant, TransactionOptions::new())
        .await
        .unwrap();

    // Create valid resource
    let patient = json!({"resourceType": "Patient"});
    let created = tx.create("Patient", patient).await.unwrap();
    let created_id = created.id().to_string();

    // Attempt invalid operation (depends on backend validation)
    // For example, trying to update a non-existent resource
    let fake_resource = helios_persistence::types::StoredResource::new(
        "Patient",
        "non-existent-id",
        tenant.tenant_id().clone(),
        json!({"resourceType": "Patient"}),
        FhirVersion::default(),
    );
    let result = tx
        .update(&fake_resource, json!({"resourceType": "Patient"}))
        .await;

    // Error should occur
    assert!(result.is_err());

    // Abort transaction
    Box::new(tx).rollback().await.unwrap();

    // Valid resource should not have been persisted
    assert!(
        !backend
            .exists(&tenant, "Patient", &created_id)
            .await
            .unwrap()
    );
}

/// Test nested transaction behavior (if supported).
///
/// Ported to the current transaction API for structure, but `#[ignore]`d:
/// nested transactions are not a concept in the redesigned API. A second
/// `begin_transaction` is an independent transaction that contends on the
/// SQLite write lock for the full 30s `busy_timeout` before erroring, which
/// is a different mechanism than the original "backend refuses a nested
/// transaction" this test was written to document.
#[cfg(feature = "sqlite")]
#[tokio::test]
#[ignore = "#306 follow-up: nested transactions not a concept in current transaction API"]
async fn test_nested_transactions() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Start outer transaction
    let mut outer_tx = backend
        .begin_transaction(&tenant, TransactionOptions::new())
        .await
        .unwrap();

    let patient = json!({"resourceType": "Patient", "name": [{"family": "Outer"}]});
    outer_tx.create("Patient", patient).await.unwrap();

    // Attempting to start another transaction might error or be supported
    // depending on backend implementation
    let inner_result = backend
        .begin_transaction(&tenant, TransactionOptions::new())
        .await;

    // If nested transactions are not supported, abort outer and verify no changes
    if inner_result.is_err() {
        Box::new(outer_tx).rollback().await.unwrap();
        let count = backend.count(&tenant, Some("Patient")).await.unwrap();
        assert_eq!(count, 0);
    }
    // If supported, this test documents the behavior
}

// ============================================================================
// Performance/Batch Tests
// ============================================================================

/// Test transaction with many operations.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_transaction_batch_operations() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let mut tx = backend
        .begin_transaction(&tenant, TransactionOptions::new())
        .await
        .unwrap();

    // Create 100 resources
    for i in 0..100 {
        let patient = json!({
            "resourceType": "Patient",
            "name": [{"family": format!("Patient{}", i)}]
        });
        tx.create("Patient", patient).await.unwrap();
    }

    Box::new(tx).commit().await.unwrap();

    let count = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count, 100);
}
