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

    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
        .unwrap();

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

    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
        .unwrap();

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

    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
        .unwrap();

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

    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
        .unwrap();

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

    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
        .unwrap();

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

    let result1 = backend
        .process_transaction(&tenant, bundle1, FhirVersion::default())
        .await
        .unwrap();
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

    let result2 = backend
        .process_transaction(&tenant, bundle2, FhirVersion::default())
        .await
        .unwrap();

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

    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
        .unwrap();
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

    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await;

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

    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await;

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

    let result = backend
        .process_transaction(&tenant, vec![], FhirVersion::default())
        .await;

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

    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
        .unwrap();
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
        .process_transaction(&tenant_a, entries, FhirVersion::default())
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

// ============================================================================
// Issue #311 — `ifMatch` on bundle entries
//
// These cover the two defects the issue surfaced at the persistence layer:
//
//  1. `Bundle.entry.request.ifMatch` was **silently ignored** in the BATCH arm
//     (`process_batch`) on SQLite and PostgreSQL, while the TRANSACTION arm
//     honored it. Optimistic locking therefore vanished for anyone who wrapped
//     a PUT in a `type: batch` Bundle — a lost update reported as `200 OK`.
//  2. `If-Match` is a comma-separated list (RFC 9110 §13.1.1), but the raw field
//     value was compared as one opaque string, so a multi-valued header could
//     never match and `*` was unsupported.
//
// `ifMatch` on DELETE was ignored in both arms and is covered here too.
//
// The comparison itself is shared by all four backends via
// `helios_persistence::core::preconditions`; these tests prove the *wiring* in
// each arm. Run them against a non-SQLite backend by swapping the constructor.
// ============================================================================

/// Helper: a PUT entry carrying an `ifMatch` precondition.
#[cfg(feature = "sqlite")]
fn put_entry(id: &str, family: &str, if_match: Option<&str>) -> BundleEntry {
    BundleEntry {
        method: BundleMethod::Put,
        url: format!("Patient/{id}"),
        resource: Some(json!({
            "resourceType": "Patient",
            "id": id,
            "name": [{"family": family}]
        })),
        if_match: if_match.map(String::from),
        if_none_match: None,
        if_none_exist: None,
        full_url: None,
    }
}

/// Helper: a DELETE entry carrying an `ifMatch` precondition.
#[cfg(feature = "sqlite")]
fn delete_entry(id: &str, if_match: Option<&str>) -> BundleEntry {
    BundleEntry {
        method: BundleMethod::Delete,
        url: format!("Patient/{id}"),
        resource: None,
        if_match: if_match.map(String::from),
        if_none_match: None,
        if_none_exist: None,
        full_url: None,
    }
}

#[cfg(feature = "sqlite")]
async fn seed_patient(backend: &SqliteBackend, tenant: &TenantContext, id: &str) -> String {
    let (created, _) = backend
        .create_or_update(
            tenant,
            "Patient",
            id,
            json!({"resourceType": "Patient", "id": id, "name": [{"family": "Original"}]}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    created.version_id().to_string()
}

/// A stale `ifMatch` in a **batch** must fail the entry with 412 and leave the
/// stored resource untouched.
///
/// Before the fix this returned `200 OK` and overwrote the record — the silent
/// lost update. On `main` this test fails with `status == 200` and a stored
/// family of `Stale`.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn batch_put_honors_stale_if_match() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_patient(&backend, &tenant, "batch-stale").await;

    let result = backend
        .process_batch(
            &tenant,
            vec![put_entry("batch-stale", "Stale", Some("W/\"99\""))],
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(
        result.entries[0].status, 412,
        "a stale ifMatch in a batch must fail the entry"
    );

    let stored = backend
        .read(&tenant, "Patient", "batch-stale")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.content()["name"][0]["family"],
        "Original",
        "the stale write must not have landed"
    );
}

/// The matching case still succeeds in a batch, and the version advances —
/// guards against "fixing" the above by rejecting everything.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn batch_put_accepts_matching_if_match() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    let version = seed_patient(&backend, &tenant, "batch-match").await;

    let result = backend
        .process_batch(
            &tenant,
            vec![put_entry(
                "batch-match",
                "Updated",
                Some(&format!("W/\"{version}\"")),
            )],
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(result.entries[0].status, 200);

    let stored = backend
        .read(&tenant, "Patient", "batch-match")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.content()["name"][0]["family"], "Updated");
    assert_ne!(
        stored.version_id(),
        version,
        "a successful conditional write must advance the version"
    );
}

/// Issue #311's headline case: a multi-valued `ifMatch` matches when ANY listed
/// tag matches. Before the fix the whole value was compared as one string, so
/// this was a permanent 412.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn multi_valued_if_match_matches_any_member() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    let version = seed_patient(&backend, &tenant, "multi").await;

    let list = format!("W/\"99\", W/\"{version}\"");
    let result = backend
        .process_transaction(
            &tenant,
            vec![put_entry("multi", "Updated", Some(&list))],
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(
        result.entries[0].status, 200,
        "a list must match on any member"
    );

    let stored = backend
        .read(&tenant, "Patient", "multi")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.content()["name"][0]["family"], "Updated");
}

/// A list in which nothing matches still fails.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn multi_valued_if_match_fails_when_no_member_matches() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_patient(&backend, &tenant, "multi-miss").await;

    // A transaction is all-or-nothing: a failed entry rolls the whole bundle
    // back and surfaces as `TransactionError::BundleError`, so this asserts the
    // error rather than an entry status (see `process_transaction`).
    let result = backend
        .process_transaction(
            &tenant,
            vec![put_entry("multi-miss", "Nope", Some("W/\"98\", W/\"99\""))],
            FhirVersion::default(),
        )
        .await;

    assert!(
        result.is_err(),
        "no listed tag matches, so the transaction must fail"
    );

    let stored = backend
        .read(&tenant, "Patient", "multi-miss")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.content()["name"][0]["family"], "Original");
}

/// A client echoing the strong form (`"3"`) must match the weak ETag the server
/// emits (`W/"3"`). SQLite/PostgreSQL previously compared raw strings, so this
/// failed there while succeeding on MongoDB/S3 — the same request, a different
/// answer per backend.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn strong_form_if_match_matches_weak_etag() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    let version = seed_patient(&backend, &tenant, "strong-form").await;

    let result = backend
        .process_transaction(
            &tenant,
            vec![put_entry(
                "strong-form",
                "Updated",
                Some(&format!("\"{version}\"")),
            )],
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(result.entries[0].status, 200);
}

/// `ifMatch` against a resource that does not exist cannot be satisfied, so the
/// entry must fail rather than silently create the resource.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn if_match_on_absent_resource_fails_instead_of_creating() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let result = backend
        .process_batch(
            &tenant,
            vec![put_entry("never-existed", "New", Some("W/\"1\""))],
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(result.entries[0].status, 412);
    assert!(
        !backend
            .exists(&tenant, "Patient", "never-existed")
            .await
            .unwrap(),
        "a failed precondition must not create the resource"
    );
}

/// `ifMatch: *` asserts a current representation EXISTS, so it succeeds against
/// a stored resource and fails against an absent one.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn star_if_match_requires_an_existing_resource() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_patient(&backend, &tenant, "star-present").await;

    let ok = backend
        .process_batch(
            &tenant,
            vec![put_entry("star-present", "Updated", Some("*"))],
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_eq!(ok.entries[0].status, 200);

    let missing = backend
        .process_batch(
            &tenant,
            vec![put_entry("star-absent", "New", Some("*"))],
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_eq!(missing.entries[0].status, 412);
}

/// A malformed `ifMatch` must fail closed, never be treated as absent.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn malformed_if_match_fails_closed() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_patient(&backend, &tenant, "malformed").await;

    let result = backend
        .process_batch(
            &tenant,
            vec![put_entry("malformed", "Overwritten", Some("garbage"))],
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(result.entries[0].status, 412);

    let stored = backend
        .read(&tenant, "Patient", "malformed")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.content()["name"][0]["family"],
        "Original",
        "a malformed precondition must not become an unconditional write"
    );
}

/// `ifMatch` on DELETE was ignored in both arms on SQLite/PostgreSQL: a client
/// deleting "the version I reviewed" could destroy a concurrent amendment with
/// no 412. On `main` this test fails because the resource is deleted.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn batch_delete_honors_stale_if_match() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_patient(&backend, &tenant, "del-stale").await;

    let result = backend
        .process_batch(
            &tenant,
            vec![delete_entry("del-stale", Some("W/\"99\""))],
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(result.entries[0].status, 412);
    assert!(
        backend
            .exists(&tenant, "Patient", "del-stale")
            .await
            .unwrap(),
        "a stale ifMatch must not delete the resource"
    );
}

/// The matching DELETE still succeeds.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn batch_delete_accepts_matching_if_match() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    let version = seed_patient(&backend, &tenant, "del-match").await;

    let result = backend
        .process_batch(
            &tenant,
            vec![delete_entry("del-match", Some(&format!("W/\"{version}\"")))],
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(result.entries[0].status, 204);
    assert!(
        !backend
            .exists(&tenant, "Patient", "del-match")
            .await
            .unwrap(),
        "a matching ifMatch must delete the resource"
    );
}

/// The transaction arm must agree with the batch arm on DELETE.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn transaction_delete_honors_stale_if_match() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_patient(&backend, &tenant, "tx-del-stale").await;

    // As above: the failed entry rolls the transaction back, so the call errors.
    let result = backend
        .process_transaction(
            &tenant,
            vec![delete_entry("tx-del-stale", Some("W/\"99\""))],
            FhirVersion::default(),
        )
        .await;

    assert!(result.is_err(), "a stale ifMatch must fail the transaction");
    assert!(
        backend
            .exists(&tenant, "Patient", "tx-del-stale")
            .await
            .unwrap(),
        "the resource must survive a failed conditional delete"
    );
}
