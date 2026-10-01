//! #1602: a transaction Bundle entry's `ifNoneExist` must apply `_id` and
//! `_lastUpdated` even when it combines them with an indexed parameter.
//!
//! On the standalone (non-offloaded) MongoDB backend the indexed parameters
//! (`identifier`, ...) choose the candidate resources through `search_index`,
//! and `_id` / `_lastUpdated` live on the `resources` document rather than in
//! the index. They were dropped from the criteria whenever another parameter
//! was present, so `identifier=urn:x|1&_id=b` matched whichever resource
//! carried the identifier: the entry answered 200 with someone else's location,
//! or 412 on two matches, instead of creating.
//!
//! Uses `super::*` for the parent test crate's imports and private harness
//! helpers (`create_backend_with_full_registry`, `create_tenant`,
//! `process_transaction_or_skip`, ...) - this file is a `#[path]`-included
//! child module of `mongodb_tests.rs`, not a standalone test binary.

use super::*;
use helios_persistence::core::BundleEntryResult;
use helios_persistence::types::StoredResource;

/// The indexed half of every criterion below.
const IDENTIFIER_CRITERION: &str = "identifier=urn:x|1";

fn patient_body(id: Option<&str>, identifier_value: &str) -> serde_json::Value {
    let mut body = json!({
        "resourceType": "Patient",
        "identifier": [{ "system": "urn:x", "value": identifier_value }],
    });
    if let Some(id) = id {
        body["id"] = json!(id);
    }
    body
}

/// Seeds `Patient/<id>` carrying `identifier = urn:x|<identifier_value>` and
/// returns what was stored (for its `lastUpdated`).
async fn seed_patient(
    backend: &MongoBackend,
    tenant: &TenantContext,
    id: &str,
    identifier_value: &str,
) -> StoredResource {
    backend
        .create(
            tenant,
            "Patient",
            patient_body(Some(id), identifier_value),
            FhirVersion::default(),
        )
        .await
        .unwrap_or_else(|e| panic!("failed to seed Patient/{id}: {e}"))
}

/// Runs one transaction holding a single `POST Patient` entry whose
/// `ifNoneExist` is `criteria`, and returns that entry's result. `None` means
/// the replica-set-only transaction was skipped.
async fn post_patient_if_none_exist(
    backend: &MongoBackend,
    tenant: &TenantContext,
    criteria: &str,
    test_name: &str,
) -> Option<BundleEntryResult> {
    let entry = BundleEntry {
        method: BundleMethod::Post,
        url: "Patient".to_string(),
        resource: Some(patient_body(None, "1")),
        if_match: None,
        if_none_match: None,
        if_none_exist: Some(criteria.to_string()),
        full_url: Some("urn:uuid:1602-entry".to_string()),
    };
    let result = process_transaction_or_skip(backend, tenant, vec![entry], test_name).await?;
    assert_eq!(result.entries.len(), 1, "one entry in, one result out");
    result.entries.into_iter().next()
}

/// `Patient` id named by an entry's `location`.
fn located_id(result: &BundleEntryResult) -> String {
    extract_resource_id_from_location(
        result
            .location
            .as_deref()
            .expect("a created or matched entry carries a location"),
    )
}

/// An RFC 3339 instant `hours` away from `base`, second precision, `Z` suffix
/// (no `+`, which a query string would turn into a space).
fn instant_offset(base: chrono::DateTime<chrono::Utc>, hours: i64) -> String {
    (base + chrono::Duration::hours(hours)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// (a) `_id` is not a bystander: `Patient/a` carries the identifier but is not
/// `b`, so the criterion matches nothing and the entry must create.
#[tokio::test]
async fn if_none_exist_id_that_matches_nothing_creates_despite_identifier_match() {
    const NAME: &str = "if_none_exist_id_that_matches_nothing_creates_despite_identifier_match";
    let Some(backend) = create_backend_with_full_registry("ine_id_creates").await else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-ine-id-creates");
    seed_patient(&backend, &tenant, "a", "1").await;

    let criteria = format!("{IDENTIFIER_CRITERION}&_id=b");
    let Some(result) = post_patient_if_none_exist(&backend, &tenant, &criteria, NAME).await else {
        return;
    };

    assert_eq!(
        result.status, 201,
        "no resource is both identifier urn:x|1 and id b, so the entry must create \
         (it answered {} at {:?})",
        result.status, result.location
    );
    assert_eq!(result.effect, BundleEntryEffect::Created);
    assert_ne!(
        located_id(&result),
        "a",
        "the entry must not resolve to the resource that only satisfied `identifier`"
    );
    assert_eq!(
        backend.count(&tenant, Some("Patient")).await.unwrap(),
        2,
        "the seeded patient plus the newly created one"
    );
}

/// (b) The same criterion with the id that does exist still matches it.
#[tokio::test]
async fn if_none_exist_id_that_matches_the_identifier_holder_matches_it() {
    const NAME: &str = "if_none_exist_id_that_matches_the_identifier_holder_matches_it";
    let Some(backend) = create_backend_with_full_registry("ine_id_matches").await else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-ine-id-matches");
    seed_patient(&backend, &tenant, "a", "1").await;

    let criteria = format!("{IDENTIFIER_CRITERION}&_id=a");
    let Some(result) = post_patient_if_none_exist(&backend, &tenant, &criteria, NAME).await else {
        return;
    };

    assert_eq!(
        result.status, 200,
        "identifier and _id both match Patient/a"
    );
    assert_eq!(result.effect, BundleEntryEffect::NoOp);
    assert_eq!(located_id(&result), "a");
    assert_eq!(
        backend.count(&tenant, Some("Patient")).await.unwrap(),
        1,
        "nothing may be created when the criterion matches"
    );
}

/// (c) `_lastUpdated=gt<after a's lastUpdated>` excludes `a`, so the entry
/// creates.
#[tokio::test]
async fn if_none_exist_last_updated_after_the_match_creates_despite_identifier_match() {
    const NAME: &str =
        "if_none_exist_last_updated_after_the_match_creates_despite_identifier_match";
    let Some(backend) = create_backend_with_full_registry("ine_lu_creates").await else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-ine-lu-creates");
    let seeded = seed_patient(&backend, &tenant, "a", "1").await;

    let after = instant_offset(seeded.last_modified(), 1);
    let criteria = format!("{IDENTIFIER_CRITERION}&_lastUpdated=gt{after}");
    let Some(result) = post_patient_if_none_exist(&backend, &tenant, &criteria, NAME).await else {
        return;
    };

    assert_eq!(
        result.status, 201,
        "Patient/a was last updated before {after}, so `_lastUpdated=gt{after}` excludes it \
         and the entry must create (it answered {} at {:?})",
        result.status, result.location
    );
    assert_eq!(result.effect, BundleEntryEffect::Created);
    assert_ne!(located_id(&result), "a");
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 2);
}

/// (d) `_lastUpdated=lt<a later instant>` includes `a`, so the entry matches.
#[tokio::test]
async fn if_none_exist_last_updated_before_a_later_instant_matches_the_identifier_holder() {
    const NAME: &str =
        "if_none_exist_last_updated_before_a_later_instant_matches_the_identifier_holder";
    let Some(backend) = create_backend_with_full_registry("ine_lu_matches").await else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-ine-lu-matches");
    let seeded = seed_patient(&backend, &tenant, "a", "1").await;

    let later = instant_offset(seeded.last_modified(), 1);
    let criteria = format!("{IDENTIFIER_CRITERION}&_lastUpdated=lt{later}");
    let Some(result) = post_patient_if_none_exist(&backend, &tenant, &criteria, NAME).await else {
        return;
    };

    assert_eq!(result.status, 200, "Patient/a is older than {later}");
    assert_eq!(result.effect, BundleEntryEffect::NoOp);
    assert_eq!(located_id(&result), "a");
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 1);
}

/// The 412 half of the defect: two patients share the identifier, and `_id`
/// names one of them. That is exactly one match (200), not "multiple matches".
#[tokio::test]
async fn if_none_exist_id_narrows_two_identifier_matches_to_one() {
    const NAME: &str = "if_none_exist_id_narrows_two_identifier_matches_to_one";
    let Some(backend) = create_backend_with_full_registry("ine_id_narrows").await else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-ine-id-narrows");
    seed_patient(&backend, &tenant, "a", "1").await;
    seed_patient(&backend, &tenant, "b", "1").await;

    let criteria = format!("{IDENTIFIER_CRITERION}&_id=b");
    let Some(result) = post_patient_if_none_exist(&backend, &tenant, &criteria, NAME).await else {
        return;
    };

    assert_eq!(
        result.status, 200,
        "`_id=b` leaves exactly one of the two identifier holders"
    );
    assert_eq!(located_id(&result), "b");
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 2);
}

/// The candidate loop pages `search_index` 128 rows at a time and stops at two
/// matches. With more than a page of identifier holders, the one `_id` names
/// must still be found - the fetch has to filter, and the loop has to keep
/// paging past pages where nothing survives - whichever end of the range it
/// sits on.
#[tokio::test]
async fn if_none_exist_id_is_found_among_more_identifier_matches_than_one_page() {
    const NAME: &str = "if_none_exist_id_is_found_among_more_identifier_matches_than_one_page";
    let Some(backend) = create_backend_with_full_registry("ine_id_paging").await else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-ine-id-paging");

    // One page is 128 index rows; 130 holders guarantee a second page.
    const HOLDERS: usize = 130;
    for n in 0..HOLDERS {
        seed_patient(&backend, &tenant, &format!("holder-{n:03}"), "1").await;
    }

    for target in ["holder-000", "holder-129"] {
        let criteria = format!("{IDENTIFIER_CRITERION}&_id={target}");
        let Some(result) = post_patient_if_none_exist(&backend, &tenant, &criteria, NAME).await
        else {
            return;
        };
        assert_eq!(
            result.status, 200,
            "`_id={target}` names exactly one of {HOLDERS} identifier holders"
        );
        assert_eq!(located_id(&result), target);
    }
    assert_eq!(
        backend.count(&tenant, Some("Patient")).await.unwrap() as usize,
        HOLDERS,
        "nothing may be created when the criterion matches"
    );
}
