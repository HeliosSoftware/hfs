//! Backend-agnostic conformance suite for URL-borne conditional entries in
//! transaction Bundles — `PUT [type]?[criteria]` and `DELETE [type]?[criteria]`
//! (issue #859).
//!
//! Every scenario is generic over [`BundleProvider`], so the *same* assertions
//! run against each backend. The resolution rules are shared through
//! `helios_persistence::core::bundle_conditionals`; what these scenarios pin
//! down is each backend's wiring: that criteria are resolved inside the open
//! transaction before any write, that the outcome is pinned, that several
//! matches and overlapping identities fail the whole bundle and roll back
//! earlier entries, and that a matched entry's `fullUrl` resolves `urn:uuid`
//! references from any position.
//!
//! Criteria arrive typed on `BundleEntry::criteria`, as the REST layer sends
//! them; the scenarios build a plain `identifier` token, the one shape every
//! backend's in-transaction matcher evaluates (MongoDB's refuses the rest, see
//! `mongodb_tests.rs`).
//!
//! Like `if_match_suite.rs`, this file is `#[path]`-included by each backend's
//! test binary rather than living in `tests/common/` (issue #306). The
//! PostgreSQL and MongoDB suites run against one long-lived database, so
//! callers must pass a **distinct tenant per scenario**.

#![allow(dead_code)]

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{BundleEntry, BundleMethod, BundleProvider};
use helios_persistence::error::TransactionError;
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::{SearchParamType, SearchParameter, SearchValue, StoredResource};

const IDENTIFIER: &str = "http://example.org|12345";

/// The typed form of `identifier=http://example.org|12345`.
pub fn identifier_criteria() -> Vec<SearchParameter> {
    vec![SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        values: vec![SearchValue::eq(IDENTIFIER)],
        ..Default::default()
    }]
}

fn patient(family: &str) -> serde_json::Value {
    json!({
        "resourceType": "Patient",
        "identifier": [{"system": "http://example.org", "value": "12345"}],
        "name": [{"family": family}]
    })
}

/// `PUT Patient?identifier=…` carrying `family`.
pub fn conditional_put(family: &str, full_url: Option<&str>) -> BundleEntry {
    BundleEntry {
        method: BundleMethod::Put,
        url: format!("Patient?identifier={IDENTIFIER}"),
        resource: Some(patient(family)),
        full_url: full_url.map(String::from),
        criteria: Some(identifier_criteria()),
        ..Default::default()
    }
}

/// `DELETE Patient?identifier=…`.
pub fn conditional_delete() -> BundleEntry {
    BundleEntry {
        method: BundleMethod::Delete,
        url: format!("Patient?identifier={IDENTIFIER}"),
        criteria: Some(identifier_criteria()),
        ..Default::default()
    }
}

fn plain_post(family: &str) -> BundleEntry {
    BundleEntry {
        method: BundleMethod::Post,
        url: "Patient".to_string(),
        resource: Some(json!({"resourceType": "Patient", "name": [{"family": family}]})),
        ..Default::default()
    }
}

/// Seeds `Patient/{id}` carrying the suite's identifier and `family`.
pub async fn seed_identified_patient<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
    id: &str,
    family: &str,
) -> StoredResource {
    let mut resource = patient(family);
    resource["id"] = json!(id);
    let (stored, _) = backend
        .create_or_update(tenant, "Patient", id, resource, FhirVersion::default())
        .await
        .expect("seed patient");
    stored
}

async fn patient_count<B: BundleProvider>(backend: &B, tenant: &TenantContext) -> u64 {
    backend.count(tenant, Some("Patient")).await.expect("count")
}

async fn family_of<B: BundleProvider>(backend: &B, tenant: &TenantContext, id: &str) -> String {
    backend
        .read(tenant, "Patient", id)
        .await
        .expect("read")
        .expect("patient exists")
        .content()["name"][0]["family"]
        .as_str()
        .expect("family")
        .to_string()
}

/// One match: the entry updates it and answers `200` naming the new version.
pub async fn conditional_put_updates_the_single_match<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    let seeded = seed_identified_patient(backend, tenant, "p1", "Original").await;

    let result = backend
        .process_transaction(
            tenant,
            vec![conditional_put("Updated", None)],
            FhirVersion::default(),
        )
        .await
        .expect("transaction");

    let entry = &result.entries[0];
    assert_eq!(entry.status, 200, "{entry:?}");
    assert_eq!(
        entry.location.as_deref(),
        Some("Patient/p1/_history/2"),
        "the 200 names the updated version so a urn:uuid reference resolves"
    );
    assert_eq!(
        entry.resource.as_ref().and_then(|r| r["id"].as_str()),
        Some("p1")
    );
    assert_eq!(family_of(backend, tenant, "p1").await, "Updated");
    assert_eq!(
        patient_count(backend, tenant).await,
        1,
        "an update creates nothing"
    );
    assert_ne!(
        entry
            .resource
            .as_ref()
            .and_then(|r| r["meta"]["versionId"].as_str()),
        Some(seeded.version_id()),
        "a new version was written"
    );
}

/// No match: the entry creates and answers `201`.
pub async fn conditional_put_creates_when_nothing_matches<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    let result = backend
        .process_transaction(
            tenant,
            vec![conditional_put("New", None)],
            FhirVersion::default(),
        )
        .await
        .expect("transaction");

    let entry = &result.entries[0];
    assert_eq!(entry.status, 201, "{entry:?}");
    assert!(
        entry
            .location
            .as_deref()
            .is_some_and(|l| l.starts_with("Patient/") && l.ends_with("/_history/1")),
        "{entry:?}"
    );
    assert_eq!(patient_count(backend, tenant).await, 1);
}

/// Several matches fail the bundle with `412 multiple-matches`, and the plain
/// create that preceded the conditional entry is rolled back.
pub async fn conditional_put_with_several_matches_rolls_back<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    seed_identified_patient(backend, tenant, "p1", "One").await;
    seed_identified_patient(backend, tenant, "p2", "Two").await;

    let err = backend
        .process_transaction(
            tenant,
            vec![plain_post("Sibling"), conditional_put("Ambiguous", None)],
            FhirVersion::default(),
        )
        .await
        .expect_err("two matches must fail the bundle");

    match err {
        TransactionError::MultipleMatches { operation, count } => {
            assert_eq!(operation, "update");
            assert_eq!(count, 2);
        }
        other => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(
        patient_count(backend, tenant).await,
        2,
        "the plain create in entry 0 must have been rolled back"
    );
    assert_eq!(family_of(backend, tenant, "p1").await, "One");
}

/// One match: deleted, `204` naming the deleted version.
pub async fn conditional_delete_removes_the_single_match<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    seed_identified_patient(backend, tenant, "p1", "Original").await;

    let result = backend
        .process_transaction(tenant, vec![conditional_delete()], FhirVersion::default())
        .await
        .expect("transaction");

    let entry = &result.entries[0];
    assert_eq!(entry.status, 204, "{entry:?}");
    assert_eq!(
        entry.location.as_deref(),
        Some("Patient/p1/_history/1"),
        "the 204 names what it deleted, for the audit trail and secondary sync"
    );
    assert!(entry.resource.is_none());
    assert!(
        !backend
            .exists(tenant, "Patient", "p1")
            .await
            .expect("exists"),
        "the match is gone"
    );
    assert_eq!(patient_count(backend, tenant).await, 0);
}

/// No match is not an error: `204`, nothing written.
pub async fn conditional_delete_with_no_match_is_204<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    let result = backend
        .process_transaction(tenant, vec![conditional_delete()], FhirVersion::default())
        .await
        .expect("transaction");

    let entry = &result.entries[0];
    assert_eq!(entry.status, 204, "{entry:?}");
    assert!(entry.location.is_none());
    assert_eq!(patient_count(backend, tenant).await, 0);
}

/// Several matches fail the bundle and delete nothing; the sibling create
/// rolls back.
pub async fn conditional_delete_with_several_matches_rolls_back<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    seed_identified_patient(backend, tenant, "p1", "One").await;
    seed_identified_patient(backend, tenant, "p2", "Two").await;

    let err = backend
        .process_transaction(
            tenant,
            vec![plain_post("Sibling"), conditional_delete()],
            FhirVersion::default(),
        )
        .await
        .expect_err("two matches must fail the bundle");

    assert!(
        matches!(
            err,
            TransactionError::MultipleMatches { ref operation, count: 2 } if operation == "delete"
        ),
        "{err:?}"
    );
    assert_eq!(
        patient_count(backend, tenant).await,
        2,
        "nothing deleted, nothing created"
    );
}

/// R4 §3.1.0.11.2: a conditional entry resolving to a resource another entry
/// addresses by id fails the bundle, and neither write lands.
pub async fn overlap_with_an_instance_entry_fails_the_bundle<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    seed_identified_patient(backend, tenant, "p1", "Original").await;
    let mut explicit = patient("ByInstance");
    explicit["id"] = json!("p1");

    let err = backend
        .process_transaction(
            tenant,
            vec![
                BundleEntry {
                    method: BundleMethod::Put,
                    url: "Patient/p1".to_string(),
                    resource: Some(explicit),
                    ..Default::default()
                },
                conditional_put("ByCriteria", None),
            ],
            FhirVersion::default(),
        )
        .await
        .expect_err("overlapping identities must fail the bundle");

    match err {
        TransactionError::BundleError { index, message } => {
            assert_eq!(index, 1, "{message}");
            assert!(message.contains("Patient/p1"), "{message}");
            assert!(message.contains("entry 0"), "{message}");
        }
        other => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(
        family_of(backend, tenant, "p1").await,
        "Original",
        "neither write may land"
    );
}

/// Two conditional entries resolving to the same resource fail the bundle.
pub async fn two_conditional_entries_resolving_to_one_resource_fail<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    seed_identified_patient(backend, tenant, "p1", "Original").await;

    let err = backend
        .process_transaction(
            tenant,
            vec![conditional_delete(), conditional_put("Again", None)],
            FhirVersion::default(),
        )
        .await
        .expect_err("overlapping identities must fail the bundle");

    assert!(
        matches!(err, TransactionError::BundleError { index: 1, .. }),
        "{err:?}"
    );
    assert_eq!(family_of(backend, tenant, "p1").await, "Original");
}

/// A matched conditional `PUT` is resolved before anything executes, so a
/// `urn:uuid` reference to its `fullUrl` resolves even from an earlier entry.
pub async fn matched_conditional_put_resolves_urn_references<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    seed_identified_patient(backend, tenant, "p1", "Original").await;

    let result = backend
        .process_transaction(
            tenant,
            vec![
                BundleEntry {
                    method: BundleMethod::Post,
                    url: "Observation".to_string(),
                    resource: Some(json!({
                        "resourceType": "Observation",
                        "status": "final",
                        "code": {"text": "test"},
                        "subject": {"reference": "urn:uuid:patient"}
                    })),
                    full_url: Some("urn:uuid:observation".to_string()),
                    ..Default::default()
                },
                conditional_put("Updated", Some("urn:uuid:patient")),
            ],
            FhirVersion::default(),
        )
        .await
        .expect("transaction");

    assert_eq!(result.entries[0].status, 201, "{:?}", result.entries[0]);
    assert_eq!(result.entries[1].status, 200, "{:?}", result.entries[1]);
    let observation = result.entries[0]
        .resource
        .as_ref()
        .expect("observation echoed");
    assert_eq!(
        observation["subject"]["reference"],
        json!("Patient/p1"),
        "a urn:uuid reference to the matched entry resolves to the match"
    );
}
