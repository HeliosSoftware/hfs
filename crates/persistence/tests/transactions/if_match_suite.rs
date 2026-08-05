//! Backend-agnostic `Bundle.entry.request.ifMatch` conformance suite (issue #311).
//!
//! Every scenario here is generic over [`BundleProvider`], so the *same*
//! assertions run against each backend rather than being retyped per storage
//! engine. The comparison itself is shared by all backends via
//! [`helios_persistence::core::preconditions`]; what differs — and what these
//! scenarios actually pin down — is the **wiring** in each backend's
//! transaction arm, which is where issue #311 diverged:
//!
//!  1. `If-Match` is a comma-separated list (RFC 9110 §13.1.1), but the raw
//!     field value was compared as one opaque string, so a multi-valued header
//!     could never match and `*` was unsupported.
//!  2. `ifMatch` on DELETE was ignored.
//!
//! ## The batch arm lives elsewhere
//!
//! This suite once carried nine batch scenarios alongside these, driving
//! `BundleProvider::process_batch`. That method had no production caller on any
//! backend — the REST layer runs its own entry loop — so the batch half of
//! #311's fix was never reachable, and `ifMatch` stayed broken over HTTP for
//! two releases while these tests passed.
//!
//! #501 deleted the method and re-homed those nine scenarios, one for one, as
//! HTTP-level tests in `helios-rest/tests/batch_if_match.rs`, where they
//! exercise the loop the server actually runs. Coverage narrowed from two
//! backends to one, which is the right trade now that the comparison itself is
//! backend-independent and separately unit-tested in
//! [`helios_persistence::core::preconditions`].
//!
//! Because a backend can only be proven consistent by running it, this module is
//! `#[path]`-included by each backend's test binary (`transactions/mod.rs` for
//! SQLite, `postgres_tests.rs` for PostgreSQL) rather than living in
//! `tests/common/`, which no test target declares and cargo therefore never
//! compiles (issue #306).
//!
//! ## Shared-database backends
//!
//! The PostgreSQL suite runs every test against one long-lived container
//! database, so scenarios must not collide. Each takes its own
//! [`TenantContext`], and tenant isolation — not resource-id uniqueness — is
//! what keeps them apart. Callers must pass a **distinct tenant per scenario**.

#![allow(dead_code)]

use serde_json::json;

use helios_fhir::FhirVersion;
// `ResourceStorage` is deliberately not imported: every scenario is generic over
// `B: BundleProvider`, and method resolution on a type *parameter* searches the
// bound's supertraits, so `read`/`exists`/`delete`/`create_or_update` are already
// in scope. Importing it would be an unused import (`-D warnings`).
use helios_persistence::core::{BundleEntry, BundleMethod, BundleProvider};
use helios_persistence::tenant::TenantContext;

/// A PUT entry carrying an `ifMatch` precondition.
pub fn put_entry(id: &str, family: &str, if_match: Option<&str>) -> BundleEntry {
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

/// A DELETE entry carrying an `ifMatch` precondition.
pub fn delete_entry(id: &str, if_match: Option<&str>) -> BundleEntry {
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

/// Seeds `Patient/{id}` with `family == "Original"` and returns its version id.
pub async fn seed_patient<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
    id: &str,
) -> String {
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

/// Reads back `Patient/{id}`'s `name[0].family`, failing if it is absent.
async fn stored_family<B: BundleProvider>(backend: &B, tenant: &TenantContext, id: &str) -> String {
    backend
        .read(tenant, "Patient", id)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("Patient/{id} should still exist"))
        .content()["name"][0]["family"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Issue #311's headline case: a multi-valued `ifMatch` matches when ANY listed
/// tag matches. Before the fix the whole value was compared as one string, so
/// this was a permanent 412.
pub async fn multi_valued_if_match_matches_any_member<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    let version = seed_patient(backend, tenant, "multi").await;

    let list = format!("W/\"99\", W/\"{version}\"");
    let result = backend
        .process_transaction(
            tenant,
            vec![put_entry("multi", "Updated", Some(&list))],
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(
        result.entries[0].status, 200,
        "a list must match on any member"
    );
    assert_eq!(stored_family(backend, tenant, "multi").await, "Updated");
}

/// A list in which nothing matches still fails.
pub async fn multi_valued_if_match_fails_when_no_member_matches<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    seed_patient(backend, tenant, "multi-miss").await;

    // A transaction is all-or-nothing: an entry with a >= 400 status rolls the
    // whole bundle back and surfaces as `TransactionError::BundleError`, so this
    // asserts the error rather than an entry status. SQLite and PostgreSQL agree
    // on that shape (see each backend's `process_transaction`).
    let result = backend
        .process_transaction(
            tenant,
            vec![put_entry("multi-miss", "Nope", Some("W/\"98\", W/\"99\""))],
            FhirVersion::default(),
        )
        .await;

    assert!(
        result.is_err(),
        "no listed tag matches, so the transaction must fail"
    );
    assert_eq!(
        stored_family(backend, tenant, "multi-miss").await,
        "Original"
    );
}

/// A client echoing the strong form (`"3"`) must match the weak ETag the server
/// emits (`W/"3"`). SQLite/PostgreSQL previously compared raw strings, so this
/// failed there while succeeding on MongoDB/S3 — the same request, a different
/// answer per backend.
pub async fn strong_form_if_match_matches_weak_etag<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    let version = seed_patient(backend, tenant, "strong-form").await;

    let result = backend
        .process_transaction(
            tenant,
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

/// The transaction arm must agree with the batch arm on DELETE.
pub async fn transaction_delete_honors_stale_if_match<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    seed_patient(backend, tenant, "tx-del-stale").await;

    // As above: the failed entry rolls the transaction back, so the call errors.
    let result = backend
        .process_transaction(
            tenant,
            vec![delete_entry("tx-del-stale", Some("W/\"99\""))],
            FhirVersion::default(),
        )
        .await;

    assert!(result.is_err(), "a stale ifMatch must fail the transaction");
    assert!(
        backend
            .exists(tenant, "Patient", "tx-del-stale")
            .await
            .unwrap(),
        "the resource must survive a failed conditional delete"
    );
}

/// The transaction arm's DELETE gate must also *pass* a matching tag — the
/// mirror of [`transaction_delete_honors_stale_if_match`], without which a gate
/// that rejected everything would look correct.
pub async fn transaction_delete_accepts_matching_if_match<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    let version = seed_patient(backend, tenant, "tx-del-match").await;

    let result = backend
        .process_transaction(
            tenant,
            vec![delete_entry(
                "tx-del-match",
                Some(&format!("W/\"{version}\"")),
            )],
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(result.entries[0].status, 204);
    assert!(
        !backend
            .exists(tenant, "Patient", "tx-del-match")
            .await
            .unwrap(),
        "a matching ifMatch must delete the resource in a transaction too"
    );
}
