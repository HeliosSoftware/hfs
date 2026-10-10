//! Tests for FHIR bundle transaction operations.
//!
//! This module tests FHIR transaction bundles including the various
//! HTTP method equivalents and conditional operations.

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{
    BundleEntry, BundleEntryEffect, BundleMethod, BundleProvider, PatchCandidateValidator,
    ResourceStorage,
};
use helios_persistence::error::TransactionError;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};

#[cfg(feature = "sqlite")]
fn create_sqlite_backend() -> SqliteBackend {
    let backend = SqliteBackend::in_memory().expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to initialize schema");
    backend
}

/// An in-memory backend that also loads the spec `SearchParameter`s from the
/// workspace `data/` directory. `in_memory()` indexes only the embedded minimal
/// set, which does not include `identifier`, so conditional criteria on it
/// would silently match nothing.
#[cfg(feature = "sqlite")]
fn create_sqlite_backend_with_spec_params() -> SqliteBackend {
    let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .expect("workspace root");
    let config = SqliteBackendConfig {
        data_dir: Some(data_dir),
        ..Default::default()
    };
    let backend =
        SqliteBackend::with_config(":memory:", config).expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to initialize schema");
    backend
}

#[cfg(feature = "sqlite")]
fn if_none_exist_entry(family: &str, full_url: &str) -> BundleEntry {
    BundleEntry {
        method: BundleMethod::Post,
        url: "Patient".to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://example.org", "value": "12345"}],
            "name": [{"family": family}]
        })),
        if_match: None,
        if_none_match: None,
        if_none_exist: Some("identifier=http://example.org|12345".to_string()),
        full_url: Some(full_url.to_string()),
        criteria: None,
    }
}

fn create_tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    )
}

#[cfg(feature = "sqlite")]
fn patch_entry(id: &str, resource: serde_json::Value, if_match: Option<&str>) -> BundleEntry {
    BundleEntry {
        method: BundleMethod::Patch,
        url: format!("Patient/{id}"),
        resource: Some(resource),
        if_match: if_match.map(str::to_string),
        ..Default::default()
    }
}

#[cfg(feature = "sqlite")]
fn family_patch(family: &str) -> serde_json::Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [{"name": "operation", "part": [
            {"name": "type", "valueCode": "replace"},
            {"name": "path", "valueString": "Patient.name[0].family"},
            {"name": "value", "valueString": family}
        ]}]
    })
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn transaction_patch_updates_one_version_and_etag() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    backend
        .create_or_update(
            &tenant,
            "Patient",
            "p1",
            json!({
                "resourceType": "Patient", "id": "p1", "name": [{"family": "Before"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let result = backend
        .process_transaction(
            &tenant,
            vec![patch_entry("p1", family_patch("After"), Some("W/\"1\""))],
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.entries[0].status, 200);
    assert_eq!(result.entries[0].effect, BundleEntryEffect::Updated);
    assert_eq!(result.entries[0].etag.as_deref(), Some("W/\"2\""));
    let stored = backend
        .read(&tenant, "Patient", "p1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.version_id(), "2");
    assert_eq!(stored.content()["name"][0]["family"], "After");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn transaction_patch_overlapping_an_earlier_put_rolls_back() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    let put = BundleEntry {
        method: BundleMethod::Put,
        url: "Patient/new".to_string(),
        resource: Some(json!({"resourceType":"Patient","id":"new","name":[{"family":"Before"}]})),
        ..Default::default()
    };
    let error = backend
        .process_transaction(
            &tenant,
            vec![put, patch_entry("new", family_patch("After"), None)],
            FhirVersion::default(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("overlap"), "{error}");
    assert!(
        backend
            .read(&tenant, "Patient", "new")
            .await
            .unwrap()
            .is_none()
    );
}

#[cfg(feature = "sqlite")]
struct RejectInvalidPatch;

#[cfg(feature = "sqlite")]
#[async_trait::async_trait]
impl PatchCandidateValidator for RejectInvalidPatch {
    async fn validate_patch_candidate(
        &self,
        _tenant: &TenantContext,
        _version: FhirVersion,
        _resource_type: &str,
        candidate: &serde_json::Value,
    ) -> Result<(), serde_json::Value> {
        if candidate["name"][0]["family"] == "Invalid" {
            Err(
                json!({"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"structure","expression":["Patient.name[0].family"]}]}),
            )
        } else {
            Ok(())
        }
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn transaction_patch_validation_rolls_back_sibling_write_with_outcome() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    backend
        .create_or_update(
            &tenant,
            "Patient",
            "p1",
            json!({"resourceType":"Patient","id":"p1","name":[{"family":"Before"}]}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let sibling = BundleEntry {
        method: BundleMethod::Put,
        url: "Patient/sibling".to_string(),
        resource: Some(json!({"resourceType":"Patient","id":"sibling"})),
        ..Default::default()
    };
    let error = backend
        .process_transaction_with_patch_validator(
            &tenant,
            vec![sibling, patch_entry("p1", family_patch("Invalid"), None)],
            FhirVersion::default(),
            Some(&RejectInvalidPatch),
        )
        .await
        .unwrap_err();
    match error {
        TransactionError::PatchEntry {
            index: 1,
            status: 422,
            outcome,
        } => {
            assert_eq!(outcome["issue"][0]["code"], "structure");
        }
        other => panic!("expected typed validation refusal, got {other:?}"),
    }
    assert!(
        backend
            .read(&tenant, "Patient", "sibling")
            .await
            .unwrap()
            .is_none()
    );
    let unchanged = backend
        .read(&tenant, "Patient", "p1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.version_id(), "1");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn transaction_patch_refusals_leave_target_unchanged() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    backend
        .create_or_update(
            &tenant,
            "Patient",
            "p1",
            json!({"resourceType":"Patient","id":"p1","name":[{"family":"Before"}]}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let malformed = backend
        .process_transaction(
            &tenant,
            vec![patch_entry(
                "p1",
                json!({"resourceType":"Parameters"}),
                None,
            )],
            FhirVersion::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        malformed,
        TransactionError::PatchEntry { status: 400, .. }
    ));

    let missing = backend
        .process_transaction(
            &tenant,
            vec![patch_entry("absent", family_patch("After"), None)],
            FhirVersion::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        missing,
        TransactionError::PatchEntry { status: 404, .. }
    ));
    assert!(
        backend
            .read(&tenant, "Patient", "absent")
            .await
            .unwrap()
            .is_none()
    );

    let stale = backend
        .process_transaction(
            &tenant,
            vec![patch_entry("p1", family_patch("After"), Some("W/\"9\""))],
            FhirVersion::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        stale,
        TransactionError::PatchEntry { status: 412, .. }
    ));
    let stored = backend
        .read(&tenant, "Patient", "p1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.version_id(), "1");
    assert_eq!(stored.content()["name"][0]["family"], "Before");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn transaction_patch_never_mutates_audit_events() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    let entry = BundleEntry {
        method: BundleMethod::Patch,
        url: "AuditEvent/a1".to_string(),
        resource: Some(family_patch("After")),
        ..Default::default()
    };
    let error = backend
        .process_transaction(&tenant, vec![entry], FhirVersion::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        TransactionError::PatchEntry { status: 405, .. }
    ));
    assert!(
        backend
            .read(&tenant, "AuditEvent", "a1")
            .await
            .unwrap()
            .is_none()
    );
}

#[cfg(all(feature = "sqlite", feature = "R5"))]
#[tokio::test]
async fn transaction_binary_json_test_failure_rolls_back() {
    use base64::Engine;
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    backend
        .create_or_update(
            &tenant,
            "Patient",
            "p1",
            json!({"resourceType":"Patient","id":"p1","active":false}),
            FhirVersion::R5,
        )
        .await
        .unwrap();
    let sibling = BundleEntry {
        method: BundleMethod::Put,
        url: "Patient/sibling".to_string(),
        resource: Some(json!({"resourceType":"Patient","id":"sibling"})),
        ..Default::default()
    };
    let operations = json!([{"op":"test","path":"/active","value":true}]);
    let binary = json!({"resourceType":"Binary","contentType":"application/json-patch+json","data":base64::engine::general_purpose::STANDARD.encode(operations.to_string())});
    let error = backend
        .process_transaction(
            &tenant,
            vec![sibling, patch_entry("p1", binary, None)],
            FhirVersion::R5,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            &error,
            TransactionError::PatchEntry {
                index: 1,
                status: 422,
                ..
            }
        ),
        "{error:?}"
    );
    assert!(
        backend
            .read(&tenant, "Patient", "sibling")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        backend
            .read(&tenant, "Patient", "p1")
            .await
            .unwrap()
            .unwrap()
            .version_id(),
        "1"
    );
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
            criteria: None,
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
            criteria: None,
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
        assert_eq!(entry.effect, BundleEntryEffect::Created);
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
        criteria: None,
    }];

    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
        .unwrap();

    assert_eq!(result.entries.len(), 1);
    assert!(result.entries[0].status == 201 || result.entries[0].status == 200);
    // The target did not exist, so the PUT created it.
    assert_eq!(result.entries[0].effect, BundleEntryEffect::Created);

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
        criteria: None,
    }];

    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
        .unwrap();

    assert_eq!(result.entries.len(), 1);
    assert!(result.entries[0].status == 200 || result.entries[0].status == 204);
    assert_eq!(result.entries[0].effect, BundleEntryEffect::Deleted);

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
            criteria: None,
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
            criteria: None,
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
            criteria: None,
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
            criteria: None,
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
            criteria: None,
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
/// The transaction executor resolves `ifNoneExist` on the transaction's own
/// connection (#511); before that a POST always created, and this test was
/// `#[ignore]`d for the #306 follow-up.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_conditional_create() {
    let backend = create_sqlite_backend_with_spec_params();
    let tenant = create_tenant();

    // First bundle - should create
    let result1 = backend
        .process_transaction(
            &tenant,
            vec![if_none_exist_entry("Conditional", "urn:uuid:conditional")],
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_eq!(result1.entries[0].status, 201);
    assert_eq!(result1.entries[0].effect, BundleEntryEffect::Created);

    // Second bundle with same condition - should return existing
    let result2 = backend
        .process_transaction(
            &tenant,
            vec![if_none_exist_entry(
                "ShouldNotCreate",
                "urn:uuid:conditional",
            )],
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(
        result2.entries[0].status, 200,
        "the match is answered, not duplicated"
    );
    assert_eq!(
        result2.entries[0].effect,
        BundleEntryEffect::NoOp,
        "a matched ifNoneExist writes nothing"
    );
    assert_eq!(
        result2.entries[0].location, result1.entries[0].location,
        "the 200 entry must name the resource the 201 entry created"
    );
    let echoed = result2.entries[0].resource.as_ref().expect("match echoed");
    assert_eq!(echoed["name"][0]["family"], "Conditional");

    // Only one patient should exist
    let count = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count, 1);
}

/// A `urn:uuid` reference to a matched `ifNoneExist` entry resolves to the
/// match (R4 §3.1.0.11.2), which needs the 200 entry's `location`.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_if_none_exist_match_resolves_urn_references() {
    let backend = create_sqlite_backend_with_spec_params();
    let tenant = create_tenant();

    let existing = backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "identifier": [{"system": "http://example.org", "value": "12345"}],
                "name": [{"family": "AlreadyThere"}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let entries = vec![
        if_none_exist_entry("Duplicate", "urn:uuid:patient"),
        BundleEntry {
            method: BundleMethod::Post,
            url: "Observation".to_string(),
            resource: Some(json!({
                "resourceType": "Observation",
                "status": "final",
                "code": {"text": "test"},
                "subject": {"reference": "urn:uuid:patient"}
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: Some("urn:uuid:observation".to_string()),
            criteria: None,
        },
    ];

    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
        .unwrap();

    assert_eq!(result.entries[0].status, 200);
    assert_eq!(result.entries[1].status, 201);
    let observation = result.entries[1].resource.as_ref().expect("created");
    assert_eq!(
        observation["subject"]["reference"],
        json!(format!("Patient/{}", existing.id()))
    );
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 1);
}

/// Criteria that match several resources fail the entry with 412 and roll the
/// whole bundle back, including entries that already succeeded.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_if_none_exist_multiple_matches_rolls_back() {
    let backend = create_sqlite_backend_with_spec_params();
    let tenant = create_tenant();

    for family in ["One", "Two"] {
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "identifier": [{"system": "http://example.org", "value": "12345"}],
                    "name": [{"family": family}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    let entries = vec![
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({"resourceType": "Patient", "name": [{"family": "Plain"}]})),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: None,
            criteria: None,
        },
        if_none_exist_entry("Ambiguous", "urn:uuid:ambiguous"),
    ];

    let err = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
        .expect_err("an ambiguous ifNoneExist must fail the bundle");
    match err {
        helios_persistence::error::TransactionError::BundleError { index, message } => {
            assert_eq!(index, 1);
            assert!(message.contains("412"), "{message}");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    assert_eq!(
        backend.count(&tenant, Some("Patient")).await.unwrap(),
        2,
        "the plain create in entry 0 must have been rolled back"
    );
}

/// Two entries with the same criteria in one bundle: the second sees the row
/// the first wrote, because the search runs on the transaction's connection.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_if_none_exist_same_criteria_twice_in_one_bundle() {
    let backend = create_sqlite_backend_with_spec_params();
    let tenant = create_tenant();

    let result = backend
        .process_transaction(
            &tenant,
            vec![
                if_none_exist_entry("First", "urn:uuid:first"),
                if_none_exist_entry("Second", "urn:uuid:second"),
            ],
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(result.entries[0].status, 201);
    assert_eq!(result.entries[1].status, 200);
    assert_eq!(result.entries[1].location, result.entries[0].location);
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 1);
}

/// With search offloaded to a secondary backend the local index is empty, so
/// the executor refuses the entry rather than creating the duplicate an
/// always-empty match set would allow.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_if_none_exist_is_refused_when_search_is_offloaded() {
    let mut backend = create_sqlite_backend_with_spec_params();
    backend.set_search_offloaded(true);
    let tenant = create_tenant();

    let err = backend
        .process_transaction(
            &tenant,
            vec![if_none_exist_entry("Offloaded", "urn:uuid:offloaded")],
            FhirVersion::default(),
        )
        .await
        .expect_err("ifNoneExist must be refused, not silently ignored");
    match err {
        helios_persistence::error::TransactionError::BundleError { index, message } => {
            assert_eq!(index, 0);
            assert!(message.contains("501"), "{message}");
        }
        other => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 0);
}

// ============================================================================
// Conditional URL entries (`Type?criteria`) inside a transaction
// ============================================================================

#[cfg(feature = "sqlite")]
const CONDITIONAL_PATIENT_URL: &str = "Patient?identifier=http://example.org|12345";

#[cfg(feature = "sqlite")]
fn conditional_put_entry(
    family: &str,
    full_url: Option<&str>,
    if_match: Option<&str>,
) -> BundleEntry {
    BundleEntry {
        method: BundleMethod::Put,
        url: CONDITIONAL_PATIENT_URL.to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://example.org", "value": "12345"}],
            "name": [{"family": family}]
        })),
        if_match: if_match.map(str::to_string),
        full_url: full_url.map(str::to_string),
        criteria: Some(super::conditional_url_suite::identifier_criteria()),
        ..Default::default()
    }
}

#[cfg(feature = "sqlite")]
async fn seed_identified_patient(
    backend: &SqliteBackend,
    tenant: &TenantContext,
    family: &str,
) -> helios_persistence::types::StoredResource {
    backend
        .create(
            tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "identifier": [{"system": "http://example.org", "value": "12345"}],
                "name": [{"family": family}]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap()
}

#[cfg(feature = "sqlite")]
fn resource_id(result: &helios_persistence::core::BundleEntryResult) -> String {
    result.resource.as_ref().unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[cfg(feature = "sqlite")]
/// The failing entry's index and a message carrying its status. A PATCH entry
/// failure keeps its own variant so the outcome reaches the client.
fn bundle_error(err: TransactionError) -> (usize, String) {
    match err {
        TransactionError::BundleError { index, message } => (index, message),
        TransactionError::PatchEntry { index, status, .. } => (index, status.to_string()),
        other => panic!("unexpected error: {other:?}"),
    }
}

/// `PUT Type?criteria` creates when nothing matches and updates the match on
/// the next bundle, keeping the id the create assigned.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_conditional_put_creates_then_updates() {
    let backend = create_sqlite_backend_with_spec_params();
    let tenant = create_tenant();

    let first = backend
        .process_transaction(
            &tenant,
            vec![conditional_put_entry("Created", None, None)],
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_eq!(first.entries[0].status, 201);
    assert_eq!(first.entries[0].effect, BundleEntryEffect::Created);
    let id = resource_id(&first.entries[0]);

    let second = backend
        .process_transaction(
            &tenant,
            vec![conditional_put_entry("Updated", None, None)],
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_eq!(second.entries[0].status, 200);
    assert_eq!(second.entries[0].effect, BundleEntryEffect::Updated);
    assert_eq!(resource_id(&second.entries[0]), id);
    assert_eq!(
        second.entries[0].resource.as_ref().unwrap()["meta"]["versionId"],
        "2"
    );

    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 1);
    let stored = backend
        .read(&tenant, "Patient", &id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.content()["name"][0]["family"], "Updated");
}

/// A `urn:uuid` reference to a conditional PUT entry resolves to the resource
/// it created or matched, which is what lets a bundle update a Patient by
/// identifier and attach new resources to it in the same transaction.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_conditional_put_resolves_urn_references() {
    let backend = create_sqlite_backend_with_spec_params();
    let tenant = create_tenant();
    let entries = || {
        vec![
            conditional_put_entry("Subject", Some("urn:uuid:patient"), None),
            BundleEntry {
                method: BundleMethod::Post,
                url: "Observation".to_string(),
                resource: Some(json!({
                    "resourceType": "Observation",
                    "status": "final",
                    "code": {"text": "test"},
                    "subject": {"reference": "urn:uuid:patient"}
                })),
                ..Default::default()
            },
        ]
    };

    let created = backend
        .process_transaction(&tenant, entries(), FhirVersion::default())
        .await
        .unwrap();
    assert_eq!(created.entries[0].status, 201);
    let id = resource_id(&created.entries[0]);
    assert_eq!(
        created.entries[1].resource.as_ref().unwrap()["subject"]["reference"],
        format!("Patient/{id}"),
        "a reference to a conditional PUT that created resolves to the new id"
    );

    let updated = backend
        .process_transaction(&tenant, entries(), FhirVersion::default())
        .await
        .unwrap();
    assert_eq!(updated.entries[0].status, 200);
    assert_eq!(
        updated.entries[1].resource.as_ref().unwrap()["subject"]["reference"],
        format!("Patient/{id}"),
        "a reference to a conditional PUT that matched resolves to the match"
    );
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 1);
    assert_eq!(
        backend.count(&tenant, Some("Observation")).await.unwrap(),
        2
    );
}

/// Several matches fail the entry with 412 and roll back the sibling create.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_conditional_put_multiple_matches_rolls_back() {
    let backend = create_sqlite_backend_with_spec_params();
    let tenant = create_tenant();
    seed_identified_patient(&backend, &tenant, "One").await;
    seed_identified_patient(&backend, &tenant, "Two").await;

    let entries = vec![
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({"resourceType": "Patient", "name": [{"family": "Plain"}]})),
            ..Default::default()
        },
        conditional_put_entry("Ambiguous", None, None),
    ];
    let error = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
        .expect_err("an ambiguous conditional PUT must fail the bundle");
    match error {
        TransactionError::MultipleMatches { operation, count } => {
            assert_eq!(operation, "update");
            assert_eq!(count, 2);
        }
        other => panic!("expected a multiple-matches precondition failure: {other:?}"),
    }
    assert_eq!(
        backend.count(&tenant, Some("Patient")).await.unwrap(),
        2,
        "the ambiguous transaction must not persist the plain create"
    );
}

/// `ifMatch` on a conditional PUT is checked against the matched resource, and
/// cannot be satisfied when nothing matched.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_conditional_put_honors_if_match() {
    let backend = create_sqlite_backend_with_spec_params();
    let tenant = create_tenant();

    let error = backend
        .process_transaction(
            &tenant,
            vec![conditional_put_entry("Nothing", None, Some("W/\"1\""))],
            FhirVersion::default(),
        )
        .await
        .expect_err("ifMatch against no match must fail");
    match error {
        TransactionError::PreconditionFailed { index, message } => {
            assert_eq!(index, 0);
            assert!(message.contains("matched no resource"), "{message}");
        }
        other => panic!("expected a no-match precondition failure: {other:?}"),
    }
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 0);

    let original = seed_identified_patient(&backend, &tenant, "Original").await;
    let error = backend
        .process_transaction(
            &tenant,
            vec![conditional_put_entry("Stale", None, Some("W/\"999\""))],
            FhirVersion::default(),
        )
        .await
        .expect_err("a stale ifMatch must fail");
    match error {
        TransactionError::PreconditionFailed { index, message } => {
            assert_eq!(index, 0);
            assert!(message.contains("999"), "{message}");
            assert!(message.contains("at version 1"), "{message}");
        }
        other => panic!("expected a stale-version precondition failure: {other:?}"),
    }
    let unchanged = backend
        .read(&tenant, "Patient", original.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.version_id(), original.version_id());
    assert_eq!(unchanged.content(), original.content());

    let result = backend
        .process_transaction(
            &tenant,
            vec![conditional_put_entry("Fresh", None, Some("W/\"1\""))],
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.entries[0].status, 200);
    assert_eq!(
        result.entries[0].resource.as_ref().unwrap()["name"][0]["family"],
        "Fresh"
    );
}

/// `DELETE Type?criteria` deletes the one match; with no match it is a 204
/// that changed nothing, as on the instance form.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_conditional_delete() {
    let backend = create_sqlite_backend_with_spec_params();
    let tenant = create_tenant();
    seed_identified_patient(&backend, &tenant, "Doomed").await;
    let delete = || BundleEntry {
        method: BundleMethod::Delete,
        url: CONDITIONAL_PATIENT_URL.to_string(),
        criteria: Some(super::conditional_url_suite::identifier_criteria()),
        ..Default::default()
    };

    let result = backend
        .process_transaction(&tenant, vec![delete()], FhirVersion::default())
        .await
        .unwrap();
    assert_eq!(result.entries[0].status, 204);
    assert_eq!(result.entries[0].effect, BundleEntryEffect::Deleted);
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 0);

    let again = backend
        .process_transaction(&tenant, vec![delete()], FhirVersion::default())
        .await
        .unwrap();
    assert_eq!(again.entries[0].status, 204);
    assert_eq!(again.entries[0].effect, BundleEntryEffect::NotFound);
}

/// `PATCH Type?criteria` patches the one match and is 404 with none.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_conditional_patch() {
    let backend = create_sqlite_backend_with_spec_params();
    let tenant = create_tenant();
    let patch = || BundleEntry {
        method: BundleMethod::Patch,
        url: CONDITIONAL_PATIENT_URL.to_string(),
        resource: Some(family_patch("Patched")),
        criteria: Some(super::conditional_url_suite::identifier_criteria()),
        ..Default::default()
    };

    let (index, message) = bundle_error(
        backend
            .process_transaction(&tenant, vec![patch()], FhirVersion::default())
            .await
            .expect_err("a conditional PATCH with no match must fail"),
    );
    assert_eq!(index, 0);
    assert!(message.contains("404"), "{message}");

    seed_identified_patient(&backend, &tenant, "Before").await;
    let result = backend
        .process_transaction(&tenant, vec![patch()], FhirVersion::default())
        .await
        .unwrap();
    assert_eq!(result.entries[0].status, 200);
    assert_eq!(result.entries[0].effect, BundleEntryEffect::Updated);
    assert_eq!(
        result.entries[0].resource.as_ref().unwrap()["name"][0]["family"],
        "Patched"
    );
}

/// With search offloaded the local index is empty, so a conditional URL entry
/// is refused rather than matched against nothing, like `ifNoneExist`.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_conditional_put_is_refused_when_search_is_offloaded() {
    let mut backend = create_sqlite_backend_with_spec_params();
    backend.set_search_offloaded(true);
    let tenant = create_tenant();

    let (index, message) = bundle_error(
        backend
            .process_transaction(
                &tenant,
                vec![conditional_put_entry("Offloaded", None, None)],
                FhirVersion::default(),
            )
            .await
            .expect_err("a conditional PUT must be refused, not applied blindly"),
    );
    assert_eq!(index, 0);
    assert!(message.contains("501"), "{message}");
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 0);
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
        criteria: None,
    }];

    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
        .unwrap();
    assert_eq!(result.entries[0].status, 200);
    assert_eq!(result.entries[0].effect, BundleEntryEffect::Updated);

    // Verify update
    let read = backend
        .read(&tenant, "Patient", "conditional-update")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.content()["name"][0]["family"], "UpdatedWithMatch");
}

/// Every entry reports the effect it really had, independent of its status
/// (#1078): a `200` PUT over an existing resource is an update, a `200` GET is
/// a read, and neither changes the live count the way a create or delete does.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_entry_effects() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    for id in ["effect-update", "effect-read", "effect-delete"] {
        backend
            .create_or_update(
                &tenant,
                "Patient",
                id,
                json!({"resourceType": "Patient", "id": id}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    let entry =
        |method: BundleMethod, url: &str, resource: Option<serde_json::Value>| BundleEntry {
            criteria: None,
            method,
            url: url.to_string(),
            resource,
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: None,
        };

    let entries = vec![
        entry(
            BundleMethod::Post,
            "Patient",
            Some(json!({"resourceType": "Patient"})),
        ),
        entry(
            BundleMethod::Put,
            "Patient/effect-update",
            Some(json!({"resourceType": "Patient", "id": "effect-update", "active": true})),
        ),
        entry(BundleMethod::Get, "Patient/effect-read", None),
        entry(BundleMethod::Delete, "Patient/effect-delete", None),
    ];

    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
        .unwrap();

    let observed: Vec<(u16, BundleEntryEffect)> = result
        .entries
        .iter()
        .map(|e| (e.status, e.effect))
        .collect();
    assert_eq!(
        observed,
        vec![
            (201, BundleEntryEffect::Created),
            (200, BundleEntryEffect::Updated),
            (200, BundleEntryEffect::Read),
            (204, BundleEntryEffect::Deleted),
        ]
    );

    let delta: i64 = result
        .entries
        .iter()
        .map(|e| e.effect.live_count_delta())
        .sum();
    assert_eq!(delta, 0, "one create and one delete");
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 3);
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
        criteria: None,
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
            criteria: None,
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
            criteria: None,
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
        criteria: None,
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
        criteria: None,
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
// The scenarios themselves are backend-agnostic and live in
// `super::if_match_suite`, so PostgreSQL runs the *same* assertions (see
// `postgres_tests.rs`) instead of a retyped approximation. These wrappers give
// each scenario its own SQLite backend and its own test name, so a failure
// still names the exact behavior that broke.
// ============================================================================

/// Expands to a `#[tokio::test]` that runs one shared scenario on a fresh
/// in-memory SQLite backend.
macro_rules! sqlite_if_match_test {
    ($name:ident) => {
        #[cfg(feature = "sqlite")]
        #[tokio::test]
        async fn $name() {
            let backend = create_sqlite_backend();
            super::if_match_suite::$name(&backend, &create_tenant()).await;
        }
    };
}

sqlite_if_match_test!(multi_valued_if_match_matches_any_member);
sqlite_if_match_test!(multi_valued_if_match_fails_when_no_member_matches);
sqlite_if_match_test!(strong_form_if_match_matches_weak_etag);
sqlite_if_match_test!(transaction_delete_honors_stale_if_match);
sqlite_if_match_test!(transaction_delete_accepts_matching_if_match);

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn transaction_overlapping_targets_and_changed_forward_references_roll_back() {
    let backend = create_sqlite_backend_with_spec_params();
    let tenant = create_tenant();
    let patient = |id: &str, marker: &str| {
        json!({"resourceType":"Patient", "id":id,
        "identifier":[{"system":"http://example.org", "value":marker}], "active":true})
    };
    backend
        .create(
            &tenant,
            "Patient",
            patient("a", "MATCH"),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let put = |url: &str, resource| {
        let entry = BundleEntry {
            method: BundleMethod::Put,
            url: url.into(),
            resource: Some(resource),
            ..Default::default()
        };
        if url.contains('?') {
            super::conditional_url_suite::with_typed_criteria(&backend, &tenant, entry)
        } else {
            entry
        }
    };
    let mut changed = patient("a", "MATCH");
    changed["active"] = json!(false);
    let error = backend
        .process_transaction(
            &tenant,
            vec![
                put("Patient/a", changed),
                put(
                    "Patient?identifier=http://example.org|MATCH",
                    patient("a", "MATCH"),
                ),
            ],
            FhirVersion::default(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, TransactionError::BundleError { ref message, .. } if message.contains("overlap")),
        "{error:?}"
    );
    assert_eq!(
        backend
            .read(&tenant, "Patient", "a")
            .await
            .unwrap()
            .unwrap()
            .content()["active"],
        true
    );

    let observation = BundleEntry {
        method: BundleMethod::Post,
        url: "Observation".into(),
        resource: Some(
            json!({"resourceType":"Observation", "id":"observation", "status":"final", "code":{"text":"test"},
            "subject":{"reference":"urn:uuid:conditional-patient"}}),
        ),
        ..Default::default()
    };
    let mut conditional = put(
        "Patient?identifier=http://example.org|MATCH",
        patient("new", "MATCH"),
    );
    conditional.full_url = Some("urn:uuid:conditional-patient".into());
    let error = backend
        .process_transaction(
            &tenant,
            vec![
                observation,
                put("Patient/a", patient("a", "CHANGED")),
                conditional,
            ],
            FhirVersion::default(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, TransactionError::BundleError { ref message, .. } if message.contains("reference") || message.contains("overlap")),
        "{error:?}"
    );
    assert!(
        backend
            .read(&tenant, "Observation", "observation")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        backend
            .read(&tenant, "Patient", "new")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        backend
            .read(&tenant, "Patient", "a")
            .await
            .unwrap()
            .unwrap()
            .content()["identifier"][0]["value"],
        "MATCH"
    );
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn transaction_duplicate_conditional_create_noops_keep_valid_forward_references() {
    let backend = create_sqlite_backend_with_spec_params();
    let tenant = create_tenant();
    let entries = vec![
        if_none_exist_entry("First", "urn:uuid:first"),
        if_none_exist_entry("Second", "urn:uuid:second"),
        BundleEntry {
            method: BundleMethod::Post,
            url: "Observation".into(),
            resource: Some(json!({
            "resourceType":"Observation", "status":"final", "code":{"text":"test"},
            "subject":{"reference":"urn:uuid:second"}})),
            ..Default::default()
        },
    ];
    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
        .unwrap();
    assert_eq!(result.entries[0].effect, BundleEntryEffect::Created);
    assert_eq!(result.entries[1].effect, BundleEntryEffect::NoOp);
    assert_eq!(
        result.entries[2].resource.as_ref().unwrap()["subject"]["reference"],
        result.entries[0].reference().unwrap()
    );
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 1);
}

/// Pinning a conditional PUT's `fullUrl` ahead of the writes must not mint an
/// id into a body the caller never sent. The bundle fails on that entry with
/// the validation error the write itself would raise, and nothing is written.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn transaction_conditional_put_without_an_object_body_is_rejected_before_pinning() {
    let backend = create_sqlite_backend_with_spec_params();
    let tenant = create_tenant();
    for (body, expected) in [
        (None, "missing required field: resource"),
        (Some(json!("not an object")), "must be a JSON object"),
    ] {
        let observation = BundleEntry {
            method: BundleMethod::Post,
            url: "Observation".into(),
            resource: Some(json!({
                "resourceType":"Observation", "status":"final", "code":{"text":"test"},
                "subject":{"reference":"urn:uuid:conditional-patient"}})),
            ..Default::default()
        };
        let mut conditional =
            conditional_put_entry("Unused", Some("urn:uuid:conditional-patient"), None);
        conditional.resource = body;
        let error = backend
            .process_transaction(
                &tenant,
                vec![observation, conditional],
                FhirVersion::default(),
            )
            .await
            .unwrap_err();
        let (index, message) = bundle_error(error);
        assert_eq!(index, 1, "{message}");
        assert!(message.contains(expected), "{message}");
        assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 0);
        assert_eq!(
            backend.count(&tenant, Some("Observation")).await.unwrap(),
            0
        );
    }
}

/// A conditional create pins its `fullUrl` to the id it would create under.
/// When an earlier entry has already written a reference to that id and the
/// create then matches a resource an intervening entry created, the written
/// reference is stale, so the bundle rolls back instead of committing it.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn transaction_conditional_create_matching_a_sibling_after_a_written_reference_rolls_back() {
    let backend = create_sqlite_backend_with_spec_params();
    let tenant = create_tenant();
    let observation = BundleEntry {
        method: BundleMethod::Post,
        url: "Observation".into(),
        resource: Some(json!({
            "resourceType":"Observation", "status":"final", "code":{"text":"test"},
            "subject":{"reference":"urn:uuid:conditional-patient"}})),
        ..Default::default()
    };
    let mut plain = if_none_exist_entry("Plain", "urn:uuid:plain");
    plain.if_none_exist = None;
    plain.full_url = None;
    let conditional = if_none_exist_entry("Conditional", "urn:uuid:conditional-patient");
    let error = backend
        .process_transaction(
            &tenant,
            vec![observation, plain, conditional],
            FhirVersion::default(),
        )
        .await
        .unwrap_err();
    let (index, message) = bundle_error(error);
    assert_eq!(index, 2, "{message}");
    assert!(
        message.contains("changed after a bundle reference was written"),
        "{message}"
    );
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 0);
    assert_eq!(
        backend.count(&tenant, Some("Observation")).await.unwrap(),
        0
    );
}

// ============================================================================
// Issue #859 — `PUT/DELETE [type]?[criteria]` inside a transaction
//
// The scenarios live in `super::conditional_url_suite` so PostgreSQL and
// MongoDB run the same assertions. Each wrapper gets its own in-memory backend
// with the spec search parameters loaded (`identifier` is not in the embedded
// minimal set).
// ============================================================================

macro_rules! sqlite_conditional_url_test {
    ($name:ident) => {
        #[cfg(feature = "sqlite")]
        #[tokio::test]
        async fn $name() {
            let backend = create_sqlite_backend_with_spec_params();
            super::conditional_url_suite::$name(&backend, &create_tenant()).await;
        }
    };
}

sqlite_conditional_url_test!(conditional_put_updates_the_single_match);
sqlite_conditional_url_test!(conditional_put_creates_when_nothing_matches);
sqlite_conditional_url_test!(conditional_put_with_several_matches_rolls_back);
sqlite_conditional_url_test!(conditional_delete_removes_the_single_match);
sqlite_conditional_url_test!(conditional_delete_with_no_match_is_204);
sqlite_conditional_url_test!(conditional_delete_with_several_matches_rolls_back);
sqlite_conditional_url_test!(overlap_with_an_instance_entry_fails_the_bundle);
sqlite_conditional_url_test!(two_conditional_entries_resolving_to_one_resource_fail);
sqlite_conditional_url_test!(matched_conditional_put_resolves_urn_references);
sqlite_conditional_url_test!(conditional_put_honours_if_match);
sqlite_conditional_url_test!(conditional_patch_updates_the_single_match);
sqlite_conditional_url_test!(conditional_patch_with_no_match_fails_the_bundle);
sqlite_conditional_url_test!(conditional_patch_with_several_matches_rolls_back);
sqlite_conditional_url_test!(conditional_patch_honours_if_match);
sqlite_conditional_url_test!(conditional_patch_overlapping_an_instance_entry_fails);

/// With search offloaded the local index is empty, so a URL-borne conditional
/// entry is refused with the same 501 `ifNoneExist` gets, before any write.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_bundle_conditional_url_is_refused_when_search_is_offloaded() {
    let mut backend = create_sqlite_backend_with_spec_params();
    backend.set_search_offloaded(true);
    assert!(!backend.supports_conditional_in_transaction());
    let tenant = create_tenant();

    let err = backend
        .process_transaction(
            &tenant,
            vec![super::conditional_url_suite::conditional_put(
                "Sibling", None,
            )],
            FhirVersion::default(),
        )
        .await
        .expect_err("must be refused");
    match err {
        helios_persistence::error::TransactionError::BundleError { index, message } => {
            assert_eq!(index, 0);
            assert!(message.contains("501"), "{message}");
        }
        other => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 0);
}

/// The `ConditionalTransaction` defaults on top of `find_matching`: upsert
/// semantics for `update_conditional`, single-match delete, and the shared
/// outcome enums (#28, #859).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_conditional_transaction_defaults() {
    use helios_persistence::core::{
        ConditionalCreateResult, ConditionalDeleteResult, ConditionalTransaction,
        ConditionalUpdateResult, Transaction, TransactionOptions, TransactionProvider,
    };

    let backend = create_sqlite_backend_with_spec_params();
    let tenant = create_tenant();
    let criteria = super::conditional_url_suite::identifier_criteria();
    let resource = json!({
        "resourceType": "Patient",
        "identifier": [{"system": "http://example.org", "value": "12345"}],
        "name": [{"family": "First"}]
    });

    let mut tx = backend
        .begin_transaction(&tenant, TransactionOptions::new())
        .await
        .unwrap();

    let created = tx
        .update_conditional("Patient", resource.clone(), &criteria)
        .await
        .unwrap();
    let ConditionalUpdateResult::Created(created) = created else {
        panic!("no match creates: {created:?}");
    };

    let matches = tx.find_matching("Patient", &criteria).await.unwrap();
    assert_eq!(matches.len(), 1, "the transaction sees its own create");
    assert_eq!(matches[0].id(), created.id());

    let mut second = resource.clone();
    second["name"][0]["family"] = json!("Second");
    let updated = tx
        .update_conditional("Patient", second, &criteria)
        .await
        .unwrap();
    let ConditionalUpdateResult::Updated(updated) = updated else {
        panic!("one match updates: {updated:?}");
    };
    assert_eq!(updated.id(), created.id());
    assert_eq!(updated.content()["name"][0]["family"], "Second");

    let deleted = tx.delete_conditional("Patient", &criteria).await.unwrap();
    let ConditionalDeleteResult::Deleted(deleted) = deleted else {
        panic!("one match deletes: {deleted:?}");
    };
    assert_eq!(deleted.id(), created.id());
    assert!(matches!(
        tx.delete_conditional("Patient", &criteria).await.unwrap(),
        ConditionalDeleteResult::NoMatch
    ));
    assert!(
        tx.find_matching("Patient", &[]).await.unwrap().is_empty(),
        "empty criteria match nothing"
    );

    // `create_if_none_exist`: the third default, and the one `ifNoneExist`
    // rides on. No match creates; the same criteria then answer the existing
    // resource rather than creating a second one.
    let created = tx
        .create_if_none_exist("Patient", resource.clone(), &criteria)
        .await
        .unwrap();
    let ConditionalCreateResult::Created(created) = created else {
        panic!("no match creates: {created:?}");
    };
    let existing = tx
        .create_if_none_exist("Patient", resource.clone(), &criteria)
        .await
        .unwrap();
    let ConditionalCreateResult::Exists(existing) = existing else {
        panic!("one match is answered as it stands: {existing:?}");
    };
    assert_eq!(existing.id(), created.id());

    // A second resource carrying the same identifier makes the criteria
    // ambiguous, which is the `412` the bundle arm renders.
    let duplicate = tx.create("Patient", resource.clone()).await.unwrap();
    assert!(matches!(
        tx.create_if_none_exist("Patient", resource, &criteria)
            .await
            .unwrap(),
        ConditionalCreateResult::MultipleMatches(2)
    ));

    tx.delete("Patient", created.id()).await.unwrap();
    tx.delete("Patient", duplicate.id()).await.unwrap();

    Box::new(tx).commit().await.unwrap();
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 0);
}

/// A POST that references a POST later in the bundle stores the later
/// entry's assigned id, not the literal `urn:uuid` (#1894).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn transaction_post_resolves_a_reference_to_a_later_post() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    let result = backend
        .process_transaction(
            &tenant,
            vec![
                BundleEntry {
                    method: BundleMethod::Post,
                    url: "Observation".into(),
                    resource: Some(json!({
                        "resourceType": "Observation", "status": "final", "code": {"text": "t"},
                        "subject": {"reference": "urn:uuid:later-patient"}})),
                    ..Default::default()
                },
                BundleEntry {
                    method: BundleMethod::Post,
                    url: "Patient".into(),
                    resource: Some(json!({"resourceType": "Patient"})),
                    full_url: Some("urn:uuid:later-patient".into()),
                    ..Default::default()
                },
            ],
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let patient = result.entries[1].reference().unwrap();
    let observation = result.entries[0].reference().unwrap();
    let (_, id) = observation.split_once('/').unwrap();
    assert_eq!(
        backend
            .read(&tenant, "Observation", id)
            .await
            .unwrap()
            .unwrap()
            .content()["subject"]["reference"],
        patient
    );
}
