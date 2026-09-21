//! Backend-agnostic `If-Match` suite for the conditional writes (issue #1381).
//!
//! `ConditionalStorage::conditional_{update,delete,patch}` resolve their
//! criteria and write inside the backend, so only the backend ever holds the
//! resource an `If-Match` precondition is about. Each one evaluates it with
//! `conditional_if_match_gate` between resolving and writing; this drives every
//! backend's real path rather than asserting once on the shared gate, because
//! the *placement* — before the write, and before the create a no-match update
//! falls through to — is per backend.
//!
//! Opens with a positive control: a backend built without the spec search
//! parameters does not know `identifier`, never matches anything, and would
//! turn every "refused" assertion below into a vacuous no-match.
//!
//! Included by `#[path]` into each backend's test binary, the same arrangement
//! as `conditional_criteria_suite.rs`.

#![allow(dead_code)]

use serde_json::{Value, json};

use helios_fhir::FhirVersion;
use helios_persistence::core::{
    ConditionalCreateResult, ConditionalDeleteResult, ConditionalPatchResult, ConditionalStorage,
    ConditionalUpdateResult, EntityTagPrecondition, PatchFormat, ResourceStorage, SearchProvider,
};
use helios_persistence::error::{ConcurrencyError, StorageError};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::SearchQuery;

const CRITERIA: &str = "identifier=ne123";
const NOBODY: &str = "identifier=nobody";

fn tenant(base: &str, label: &str) -> TenantContext {
    TenantContext::new(
        TenantId::new(format!("{base}-{label}")),
        TenantPermissions::full_access(),
    )
}

fn patient(id: Option<&str>, family: &str, identifier: &str) -> Value {
    let mut resource = json!({
        "resourceType": "Patient",
        "name": [{"family": family}],
        "identifier": [{"system": "http://example.org/mrn", "value": identifier}]
    });
    if let Some(id) = id {
        resource["id"] = json!(id);
    }
    resource
}

fn if_match(raw: &str) -> EntityTagPrecondition {
    EntityTagPrecondition::parse([raw]).expect("well-formed If-Match")
}

/// Seeds `target` (the one `CRITERIA` selects) and a decoy, and proves the
/// criteria resolve: exactly one Patient, at version 1.
async fn seed<S>(backend: &S, tenant: &TenantContext)
where
    S: ResourceStorage + ConditionalStorage + SearchProvider,
{
    for (id, family, identifier) in [("target", "Neal", "ne123"), ("decoy", "Smith", "mrn-1")] {
        backend
            .create(
                tenant,
                "Patient",
                patient(Some(id), family, identifier),
                FhirVersion::default(),
            )
            .await
            .expect("seed patient");
    }

    // Positive control, through the same criteria pipeline the writes use.
    match backend
        .conditional_create(
            tenant,
            "Patient",
            patient(None, "Duplicate", "ne123"),
            CRITERIA,
            FhirVersion::default(),
        )
        .await
        .expect("positive control")
    {
        ConditionalCreateResult::Exists(found) => {
            assert_eq!(found.id(), "target", "criteria must select the target");
            assert_eq!(found.version_id(), "1");
        }
        other => panic!(
            "positive control: {CRITERIA} must resolve to the seeded target, got {other:?} — \
             is the backend built with the spec search parameters?"
        ),
    }
}

/// `(version, family)` of a Patient, or how it went missing.
async fn state_of<S: ResourceStorage>(backend: &S, tenant: &TenantContext, id: &str) -> String {
    match backend.read(tenant, "Patient", id).await {
        Ok(Some(stored)) => format!(
            "v{} {}",
            stored.version_id(),
            stored.content()["name"][0]["family"]
                .as_str()
                .unwrap_or("?")
        ),
        Ok(None) => "<absent>".to_string(),
        Err(e) => format!("<{e}>"),
    }
}

async fn patient_count<S: SearchProvider>(backend: &S, tenant: &TenantContext) -> usize {
    backend
        .search(tenant, &SearchQuery::new("Patient"))
        .await
        .expect("search all patients")
        .resources
        .items
        .len()
}

fn is_precondition_failure<T: std::fmt::Debug>(result: &Result<T, StorageError>) -> bool {
    matches!(
        result,
        Err(StorageError::Concurrency(
            ConcurrencyError::OptimisticLockFailure { .. }
        ))
    )
}

/// Every conditional write honours `If-Match` against the resource its
/// criteria resolve to: satisfied writes, unsatisfied is an
/// `OptimisticLockFailure` that leaves storage untouched, and a no-match
/// update does not fall through to its create.
///
/// `supports_patch` is `false` for a backend that declines `conditional_patch`
/// (none of the current callers, since #1406), which must then refuse it
/// whatever the precondition says.
pub async fn if_match_is_evaluated_against_the_resolved_match<S>(
    backend: &S,
    base: &str,
    supports_patch: bool,
) where
    S: ResourceStorage + ConditionalStorage + SearchProvider,
{
    let merge = PatchFormat::MergePatch(json!({"name": [{"family": "Patched"}]}));

    // ---- update -----------------------------------------------------------
    let t = tenant(base, "update");
    seed(backend, &t).await;

    for stale in ["W/\"7\"", "W/\"7\", W/\"8\""] {
        let result = backend
            .conditional_update(
                &t,
                "Patient",
                patient(None, "Renamed", "ne123"),
                CRITERIA,
                true,
                FhirVersion::default(),
                &if_match(stale),
            )
            .await;
        assert!(
            is_precondition_failure(&result),
            "update {stale}: {result:?}"
        );
        assert_eq!(state_of(backend, &t, "target").await, "v1 Neal", "{stale}");
    }

    // Each one is satisfied by the version the previous write left behind: a
    // single tag, a list holding the current version, and `*`.
    for (satisfied, expected) in [
        ("W/\"1\"", "v2 Renamed"),
        ("W/\"9\", \"2\"", "v3 Renamed"),
        ("*", "v4 Renamed"),
    ] {
        let result = backend
            .conditional_update(
                &t,
                "Patient",
                patient(None, "Renamed", "ne123"),
                CRITERIA,
                true,
                FhirVersion::default(),
                &if_match(satisfied),
            )
            .await;
        assert!(
            matches!(result, Ok(ConditionalUpdateResult::Updated(_))),
            "update {satisfied}: {result:?}"
        );
        assert_eq!(
            state_of(backend, &t, "target").await,
            expected,
            "{satisfied}"
        );
    }

    // No match + a precondition: no create, `*` included.
    let before = patient_count(backend, &t).await;
    for named in ["W/\"1\"", "*"] {
        let result = backend
            .conditional_update(
                &t,
                "Patient",
                patient(None, "Nobody", "nobody"),
                NOBODY,
                true,
                FhirVersion::default(),
                &if_match(named),
            )
            .await;
        assert!(
            is_precondition_failure(&result),
            "no-match update {named}: {result:?}"
        );
    }
    assert_eq!(
        patient_count(backend, &t).await,
        before,
        "nothing was created"
    );
    // Control: without it, the same call creates.
    let result = backend
        .conditional_update(
            &t,
            "Patient",
            patient(None, "Nobody", "nobody"),
            NOBODY,
            true,
            FhirVersion::default(),
            &EntityTagPrecondition::Absent,
        )
        .await;
    assert!(
        matches!(result, Ok(ConditionalUpdateResult::Created(_))),
        "{result:?}"
    );
    assert_eq!(state_of(backend, &t, "decoy").await, "v1 Smith");

    // ---- delete -----------------------------------------------------------
    let t = tenant(base, "delete");
    seed(backend, &t).await;

    let result = backend
        .conditional_delete(&t, "Patient", CRITERIA, &if_match("W/\"7\""))
        .await;
    assert!(is_precondition_failure(&result), "delete stale: {result:?}");
    assert_eq!(state_of(backend, &t, "target").await, "v1 Neal");

    for named in ["W/\"1\"", "*"] {
        let result = backend
            .conditional_delete(&t, "Patient", NOBODY, &if_match(named))
            .await;
        assert!(
            is_precondition_failure(&result),
            "no-match delete {named}: {result:?}"
        );
    }
    let result = backend
        .conditional_delete(&t, "Patient", NOBODY, &EntityTagPrecondition::Absent)
        .await;
    assert!(
        matches!(result, Ok(ConditionalDeleteResult::NoMatch)),
        "{result:?}"
    );

    let result = backend
        .conditional_delete(&t, "Patient", CRITERIA, &if_match("W/\"1\""))
        .await;
    assert!(
        matches!(&result, Ok(ConditionalDeleteResult::Deleted(d)) if d.id() == "target"),
        "{result:?}"
    );
    assert!(
        !state_of(backend, &t, "target").await.starts_with('v'),
        "target is gone"
    );
    assert_eq!(state_of(backend, &t, "decoy").await, "v1 Smith");

    // ---- patch ------------------------------------------------------------
    let t = tenant(base, "patch");
    seed(backend, &t).await;

    if !supports_patch {
        let result = backend
            .conditional_patch(&t, "Patient", CRITERIA, &merge, &if_match("W/\"1\""))
            .await;
        assert!(
            matches!(result, Err(StorageError::Backend(_))),
            "unsupported stays unsupported: {result:?}"
        );
        assert_eq!(state_of(backend, &t, "target").await, "v1 Neal");
        return;
    }

    let result = backend
        .conditional_patch(&t, "Patient", CRITERIA, &merge, &if_match("W/\"7\""))
        .await;
    assert!(is_precondition_failure(&result), "patch stale: {result:?}");
    assert_eq!(state_of(backend, &t, "target").await, "v1 Neal");

    // No match stays `NoMatch`, as `PATCH [type]/[id]` stays 404.
    let result = backend
        .conditional_patch(&t, "Patient", NOBODY, &merge, &if_match("W/\"1\""))
        .await;
    assert!(
        matches!(result, Ok(ConditionalPatchResult::NoMatch)),
        "{result:?}"
    );

    let result = backend
        .conditional_patch(&t, "Patient", CRITERIA, &merge, &if_match("W/\"1\""))
        .await;
    assert!(
        matches!(result, Ok(ConditionalPatchResult::Patched(_))),
        "{result:?}"
    );
    assert_eq!(state_of(backend, &t, "target").await, "v2 Patched");
    assert_eq!(state_of(backend, &t, "decoy").await, "v1 Smith");
}

/// Eight writers, all holding `If-Match: W/"1"` for the same resource, all
/// released at once. The precondition and the write are tied together by the
/// compare-and-swap in `update`, so exactly one of them writes version 2 and
/// every other one is refused — by the gate (`OptimisticLockFailure`) if it
/// resolved after the winner, or by the swap (`VersionConflict`) if it resolved
/// before. None of them may write version 3.
///
/// That holds on every backend. MongoDB used to be exempt from the second
/// half: its `update` runs in a multi-document transaction, and a racing loser
/// there surfaced the server's `WriteConflict` as `BackendError::Internal`
/// (#1405).
///
/// These writers are futures on one task, so a backend whose calls never yield
/// (SQLite) runs them one after another; `versioned_write_race_suite.rs` is the
/// test with real parallelism.
pub async fn concurrent_writers_with_the_same_if_match_admit_one<S>(backend: &S, base: &str)
where
    S: ResourceStorage + ConditionalStorage + SearchProvider,
{
    let t = tenant(base, "race");
    seed(backend, &t).await;
    let precondition = if_match("W/\"1\"");

    let attempts = (0..8).map(|n| {
        let t = &t;
        let precondition = &precondition;
        async move {
            backend
                .conditional_update(
                    t,
                    "Patient",
                    patient(None, &format!("Writer{n}"), "ne123"),
                    CRITERIA,
                    true,
                    FhirVersion::default(),
                    precondition,
                )
                .await
        }
    });
    let results = futures::future::join_all(attempts).await;

    let winners: Vec<_> = results
        .iter()
        .filter_map(|r| match r {
            Ok(ConditionalUpdateResult::Updated(stored)) => Some(stored),
            _ => None,
        })
        .collect();
    assert_eq!(winners.len(), 1, "exactly one writer wins: {results:?}");
    for result in &results {
        match result {
            Ok(ConditionalUpdateResult::Updated(_)) | Err(StorageError::Concurrency(_)) => {}
            other => panic!("a loser is a concurrency refusal, nothing else: {other:?}"),
        }
    }

    let winner_family = winners[0].content()["name"][0]["family"]
        .as_str()
        .expect("family")
        .to_string();
    assert_eq!(
        state_of(backend, &t, "target").await,
        format!("v2 {winner_family}"),
        "the stored resource is the winner's, at version 2"
    );
}
