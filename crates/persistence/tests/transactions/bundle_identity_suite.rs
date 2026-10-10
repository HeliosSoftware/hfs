//! Backend-agnostic suite for resource identities inside a transaction
//! Bundle (#1894, #1934): forward `urn:uuid` references resolve whatever
//! entry they point at, two writes to one identity fail the bundle (R4
//! transaction rules), and a conditional DELETE that matched nothing reports
//! that it deleted nothing.
//!
//! The REST layer runs entries in DELETE, POST, PUT/PATCH order, so a POST
//! that references a PUT, or a POST later in the bundle, runs before the
//! entry it points at; these scenarios hand the backend entries already in
//! that order. On PostgreSQL a bundle without criteria runs under the shared
//! lock plan (#1637), which must accept the id pinning mints into a POST body.
//!
//! Like `conditional_url_suite.rs`, this file is `#[path]`-included by each
//! backend's test binary next to that suite, whose criteria helpers it uses.
//! The PostgreSQL and MongoDB suites run against one long-lived database, so
//! callers must pass a **distinct tenant per scenario**.

#![allow(dead_code)]

use serde_json::{Value, json};

use helios_fhir::FhirVersion;
use helios_persistence::core::{
    BundleEntry, BundleEntryEffect, BundleEntryResult, BundleMethod, BundleProvider,
};
use helios_persistence::error::TransactionError;
use helios_persistence::tenant::TenantContext;

use super::conditional_url_suite::{conditional_delete, conditional_put, seed_identified_patient};

/// `POST Observation` whose subject is `subject`.
fn observation_of(subject: &str) -> BundleEntry {
    BundleEntry {
        method: BundleMethod::Post,
        url: "Observation".to_string(),
        resource: Some(json!({
            "resourceType": "Observation",
            "status": "final",
            "code": {"text": "identity"},
            "subject": {"reference": subject}
        })),
        ..Default::default()
    }
}

fn put(id: &str, resource: Value, full_url: Option<&str>) -> BundleEntry {
    BundleEntry {
        method: BundleMethod::Put,
        url: format!("Patient/{id}"),
        resource: Some(resource),
        full_url: full_url.map(String::from),
        ..Default::default()
    }
}

fn post_patient(full_url: &str, if_none_exist: Option<&str>) -> BundleEntry {
    BundleEntry {
        method: BundleMethod::Post,
        url: "Patient".to_string(),
        resource: Some(json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://example.org", "value": "12345"}]
        })),
        full_url: Some(full_url.to_string()),
        if_none_exist: if_none_exist.map(String::from),
        ..Default::default()
    }
}

/// The stored subject of the Observation `result` created.
async fn stored_subject<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
    result: &BundleEntryResult,
) -> String {
    let reference = result.reference().expect("observation location");
    let (_, id) = reference.split_once('/').expect("Type/id");
    backend
        .read(tenant, "Observation", id)
        .await
        .expect("read")
        .expect("observation exists")
        .content()["subject"]["reference"]
        .as_str()
        .expect("subject reference")
        .to_string()
}

fn overlap_message(error: TransactionError) -> String {
    match error {
        TransactionError::BundleError { message, .. } => message,
        other => panic!("expected an overlap BundleError: {other:?}"),
    }
}

async fn count<B: BundleProvider>(backend: &B, tenant: &TenantContext, kind: &str) -> u64 {
    backend.count(tenant, Some(kind)).await.expect("count")
}

/// A POST that references a later instance PUT stores `Patient/{id}`.
pub async fn post_resolves_a_later_instance_put<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    let result = backend
        .process_transaction(
            tenant,
            vec![
                observation_of("urn:uuid:put-patient"),
                put(
                    "p1",
                    json!({"resourceType": "Patient", "id": "p1"}),
                    Some("urn:uuid:put-patient"),
                ),
            ],
            FhirVersion::default(),
        )
        .await
        .expect("transaction");
    assert_eq!(
        stored_subject(backend, tenant, &result.entries[0]).await,
        "Patient/p1"
    );
}

/// A POST that references a later conditional PUT that creates stores the
/// id the PUT created under.
pub async fn post_resolves_a_later_conditional_put_that_creates<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    let result = backend
        .process_transaction(
            tenant,
            vec![
                observation_of("urn:uuid:conditional-patient"),
                conditional_put("Created", Some("urn:uuid:conditional-patient")),
            ],
            FhirVersion::default(),
        )
        .await
        .expect("transaction");
    assert_eq!(result.entries[1].effect, BundleEntryEffect::Created);
    assert_eq!(
        stored_subject(backend, tenant, &result.entries[0]).await,
        result.entries[1].reference().expect("patient location")
    );
}

/// A POST that references a POST later in the bundle stores the later
/// entry's id. On PostgreSQL this bundle runs under the shared lock plan.
pub async fn post_resolves_a_later_post<B: BundleProvider>(backend: &B, tenant: &TenantContext) {
    let result = backend
        .process_transaction(
            tenant,
            vec![
                observation_of("urn:uuid:later-patient"),
                post_patient("urn:uuid:later-patient", None),
            ],
            FhirVersion::default(),
        )
        .await
        .expect("transaction");
    assert_eq!(
        stored_subject(backend, tenant, &result.entries[0]).await,
        result.entries[1].reference().expect("patient location")
    );
}

/// A POST that references a later `ifNoneExist` POST matching an existing
/// resource stores the match.
pub async fn post_resolves_a_later_if_none_exist_match<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    seed_identified_patient(backend, tenant, "existing", "Seeded").await;
    let result = backend
        .process_transaction(
            tenant,
            vec![
                observation_of("urn:uuid:matched-patient"),
                post_patient(
                    "urn:uuid:matched-patient",
                    Some("identifier=http://example.org|12345"),
                ),
            ],
            FhirVersion::default(),
        )
        .await
        .expect("transaction");
    assert_eq!(
        stored_subject(backend, tenant, &result.entries[0]).await,
        "Patient/existing"
    );
    assert_eq!(count(backend, tenant, "Patient").await, 1);
}

/// `PUT Patient/x` then `PATCH Patient/x` overlap: the bundle fails and
/// nothing is written.
pub async fn put_then_patch_on_one_id_rolls_back<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    let patch = BundleEntry {
        method: BundleMethod::Patch,
        url: "Patient/overlap".to_string(),
        resource: Some(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "operation", "part": [
                {"name": "type", "valueCode": "replace"},
                {"name": "path", "valueString": "Patient.name[0].family"},
                {"name": "value", "valueString": "After"}
            ]}]
        })),
        ..Default::default()
    };
    let error = backend
        .process_transaction(
            tenant,
            vec![
                put(
                    "overlap",
                    json!({"resourceType": "Patient", "id": "overlap", "name": [{"family": "Before"}]}),
                    None,
                ),
                patch,
            ],
            FhirVersion::default(),
        )
        .await
        .expect_err("overlapping writes must fail the bundle");
    let message = overlap_message(error);
    assert!(
        message.contains("overlap on resource Patient/overlap"),
        "{message}"
    );
    assert_eq!(count(backend, tenant, "Patient").await, 0);
}

/// Two PUTs to one id overlap: the bundle fails and nothing is written.
pub async fn two_puts_on_one_id_roll_back<B: BundleProvider>(backend: &B, tenant: &TenantContext) {
    let error = backend
        .process_transaction(
            tenant,
            vec![
                put(
                    "twice",
                    json!({"resourceType": "Patient", "id": "twice", "gender": "male"}),
                    None,
                ),
                put(
                    "twice",
                    json!({"resourceType": "Patient", "id": "twice", "gender": "female"}),
                    None,
                ),
            ],
            FhirVersion::default(),
        )
        .await
        .expect_err("overlapping writes must fail the bundle");
    let message = overlap_message(error);
    assert!(
        message.contains("overlap on resource Patient/twice"),
        "{message}"
    );
    assert_eq!(count(backend, tenant, "Patient").await, 0);
}

/// A conditional DELETE that matched nothing answers 204 but reports that
/// nothing was deleted, so no live count moves (#1078).
pub async fn conditional_delete_without_a_match_reports_not_found<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    let result = backend
        .process_transaction(tenant, vec![conditional_delete()], FhirVersion::default())
        .await
        .expect("transaction");
    assert_eq!(result.entries[0].status, 204);
    assert_eq!(result.entries[0].effect, BundleEntryEffect::NotFound);
    assert_eq!(result.entries[0].effect.live_count_delta(), 0);
}

/// A forward-referenced `ifNoneExist` create pins the id it would create
/// under. When an intervening entry creates a match, the reference an earlier
/// entry already wrote is stale, so the bundle rolls back.
pub async fn changed_conditional_target_after_a_written_reference_rolls_back<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    let mut sibling = post_patient("urn:uuid:sibling", None);
    sibling.full_url = None;
    let error = backend
        .process_transaction(
            tenant,
            vec![
                observation_of("urn:uuid:conditional-patient"),
                sibling,
                post_patient(
                    "urn:uuid:conditional-patient",
                    Some("identifier=http://example.org|12345"),
                ),
            ],
            FhirVersion::default(),
        )
        .await
        .expect_err("a stale written reference must fail the bundle");
    let message = overlap_message(error);
    assert!(
        message.contains("changed after a bundle reference was written"),
        "{message}"
    );
    assert_eq!(count(backend, tenant, "Patient").await, 0);
    assert_eq!(count(backend, tenant, "Observation").await, 0);
}
