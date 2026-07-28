//! Backend-agnostic `Bundle.entry.request.ifMatch` conformance suite (issue #311).
//!
//! Every scenario here is generic over [`BundleProvider`], so the *same*
//! assertions run against each backend rather than being retyped per storage
//! engine. The comparison itself is shared by all backends via
//! [`helios_persistence::core::preconditions`]; what differs — and what these
//! scenarios actually pin down — is the **wiring** in each backend's batch and
//! transaction arms, which is exactly where issue #311 diverged:
//!
//!  1. `ifMatch` was **silently ignored** in the BATCH arm (`process_batch`) on
//!     SQLite and PostgreSQL, while the TRANSACTION arm honored it. Optimistic
//!     locking therefore vanished for anyone who wrapped a PUT in a
//!     `type: batch` Bundle — a lost update reported as `200 OK`.
//!  2. `If-Match` is a comma-separated list (RFC 9110 §13.1.1), but the raw
//!     field value was compared as one opaque string, so a multi-valued header
//!     could never match and `*` was unsupported.
//!  3. `ifMatch` on DELETE was ignored in both arms.
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
use helios_persistence::core::{BundleEntry, BundleMethod, BundleProvider, ResourceStorage};
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

/// A stale `ifMatch` in a **batch** must fail the entry with 412 and leave the
/// stored resource untouched.
///
/// Before the fix this returned `200 OK` and overwrote the record — the silent
/// lost update. On `main` this fails with `status == 200` and a stored family of
/// `Stale`.
pub async fn batch_put_honors_stale_if_match<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    seed_patient(backend, tenant, "batch-stale").await;

    let result = backend
        .process_batch(
            tenant,
            vec![put_entry("batch-stale", "Stale", Some("W/\"99\""))],
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(
        result.entries[0].status, 412,
        "a stale ifMatch in a batch must fail the entry"
    );
    assert_eq!(
        stored_family(backend, tenant, "batch-stale").await,
        "Original",
        "the stale write must not have landed"
    );
}

/// The matching case still succeeds in a batch, and the version advances —
/// guards against "fixing" the above by rejecting everything.
pub async fn batch_put_accepts_matching_if_match<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    let version = seed_patient(backend, tenant, "batch-match").await;

    let result = backend
        .process_batch(
            tenant,
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
        .read(tenant, "Patient", "batch-match")
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

/// `ifMatch` against a resource that does not exist cannot be satisfied, so the
/// entry must fail rather than silently create the resource.
pub async fn if_match_on_absent_resource_fails_instead_of_creating<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    let result = backend
        .process_batch(
            tenant,
            vec![put_entry("never-existed", "New", Some("W/\"1\""))],
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(result.entries[0].status, 412);
    assert!(
        !backend
            .exists(tenant, "Patient", "never-existed")
            .await
            .unwrap(),
        "a failed precondition must not create the resource"
    );
}

/// `ifMatch: *` asserts a current representation EXISTS, so it succeeds against
/// a stored resource and fails against an absent one.
pub async fn star_if_match_requires_an_existing_resource<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    seed_patient(backend, tenant, "star-present").await;

    let ok = backend
        .process_batch(
            tenant,
            vec![put_entry("star-present", "Updated", Some("*"))],
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_eq!(ok.entries[0].status, 200);

    let missing = backend
        .process_batch(
            tenant,
            vec![put_entry("star-absent", "New", Some("*"))],
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_eq!(missing.entries[0].status, 412);
}

/// A malformed `ifMatch` must fail closed, never be treated as absent.
pub async fn malformed_if_match_fails_closed<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    seed_patient(backend, tenant, "malformed").await;

    let result = backend
        .process_batch(
            tenant,
            vec![put_entry("malformed", "Overwritten", Some("garbage"))],
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(result.entries[0].status, 412);
    assert_eq!(
        stored_family(backend, tenant, "malformed").await,
        "Original",
        "a malformed precondition must not become an unconditional write"
    );
}

/// A deleted resource has no current representation, so a supplied `ifMatch` —
/// including `*` — cannot be satisfied and the batch PUT must fail rather than
/// resurrect the record.
///
/// This is the `Gone`-tolerant read arm in each backend's batch PUT path: the
/// precondition read must map a deleted resource to "absent" without mistaking
/// it for a storage error.
pub async fn batch_put_if_match_on_deleted_resource_fails<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    seed_patient(backend, tenant, "put-deleted").await;
    backend
        .delete(tenant, "Patient", "put-deleted")
        .await
        .unwrap();

    for tag in ["W/\"1\"", "*"] {
        let result = backend
            .process_batch(
                tenant,
                vec![put_entry("put-deleted", "Resurrected", Some(tag))],
                FhirVersion::default(),
            )
            .await
            .unwrap();

        assert_eq!(
            result.entries[0].status, 412,
            "ifMatch {tag} against a deleted resource must fail"
        );
        assert!(
            !backend
                .exists(tenant, "Patient", "put-deleted")
                .await
                .unwrap(),
            "a failed precondition must not resurrect the resource"
        );
    }
}

/// The DELETE counterpart of the above: deleting an already-deleted resource
/// under an `ifMatch` must report 412, not succeed.
pub async fn batch_delete_if_match_on_deleted_resource_fails<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    seed_patient(backend, tenant, "del-deleted").await;
    backend
        .delete(tenant, "Patient", "del-deleted")
        .await
        .unwrap();

    let result = backend
        .process_batch(
            tenant,
            vec![delete_entry("del-deleted", Some("*"))],
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(
        result.entries[0].status, 412,
        "ifMatch against a deleted resource has no current representation to match"
    );
}

/// `ifMatch` on DELETE was ignored in both arms on SQLite/PostgreSQL: a client
/// deleting "the version I reviewed" could destroy a concurrent amendment with
/// no 412. On `main` this fails because the resource is deleted.
pub async fn batch_delete_honors_stale_if_match<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    seed_patient(backend, tenant, "del-stale").await;

    let result = backend
        .process_batch(
            tenant,
            vec![delete_entry("del-stale", Some("W/\"99\""))],
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(result.entries[0].status, 412);
    assert!(
        backend
            .exists(tenant, "Patient", "del-stale")
            .await
            .unwrap(),
        "a stale ifMatch must not delete the resource"
    );
}

/// The matching DELETE still succeeds.
pub async fn batch_delete_accepts_matching_if_match<B: BundleProvider>(
    backend: &B,
    tenant: &TenantContext,
) {
    let version = seed_patient(backend, tenant, "del-match").await;

    let result = backend
        .process_batch(
            tenant,
            vec![delete_entry("del-match", Some(&format!("W/\"{version}\"")))],
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert_eq!(result.entries[0].status, 204);
    assert!(
        !backend
            .exists(tenant, "Patient", "del-match")
            .await
            .unwrap(),
        "a matching ifMatch must delete the resource"
    );
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
