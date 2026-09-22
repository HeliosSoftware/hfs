//! Backend-agnostic race suite for version-aware writes (issues #1404, #1405).
//!
//! `ResourceStorage::update` and `ResourceStorage::delete_versioned` are
//! compare-and-swaps on the version the caller holds: of any number of writers
//! holding the same version, exactly one writes, and every other one is told
//! it lost with a `ConcurrencyError` — not with a backend `Internal` error, and
//! never with a success that silently overwrote the winner.
//!
//! The writers are real tasks on a multi-threaded runtime, released together
//! by a barrier and repeated over many resources, so the interleaving these
//! tests need happens by volume rather than by timing. Each test's caller must
//! therefore run it with `#[tokio::test(flavor = "multi_thread")]`; on a
//! current-thread runtime a synchronous backend (SQLite) never interleaves and
//! the assertions hold vacuously.
//!
//! Included by `#[path]` into each backend's test binary, the same arrangement
//! as `conditional_if_match_suite.rs`.

#![allow(dead_code)]

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::Barrier;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, VersionedStorage};
use helios_persistence::error::{ResourceError, StorageError};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::StoredResource;

const WRITERS: usize = 8;

fn tenant(base: &str, label: &str) -> TenantContext {
    TenantContext::new(
        TenantId::new(format!("{base}-{label}")),
        TenantPermissions::full_access(),
    )
}

fn patient(id: &str, family: &str) -> Value {
    json!({
        "resourceType": "Patient",
        "id": id,
        "name": [{"family": family}]
    })
}

fn family(stored: &StoredResource) -> String {
    stored.content()["name"][0]["family"]
        .as_str()
        .unwrap_or("?")
        .to_string()
}

/// Creates `Patient/{id}` and proves it reads back at version 1 — the positive
/// control: every writer below holds exactly this `StoredResource`.
async fn seed<S: ResourceStorage>(backend: &S, tenant: &TenantContext, id: &str) -> StoredResource {
    backend
        .create(
            tenant,
            "Patient",
            patient(id, "Seed"),
            FhirVersion::default(),
        )
        .await
        .expect("seed patient");
    let current = backend
        .read(tenant, "Patient", id)
        .await
        .expect("read seeded patient")
        .expect("seeded patient exists");
    assert_eq!(current.version_id(), "1");
    assert_eq!(family(&current), "Seed");
    current
}

/// What one racing writer did.
#[derive(Debug)]
enum Outcome {
    Updated(StoredResource),
    Deleted,
    Failed(StorageError),
}

fn assert_losers_are_concurrency_errors(outcomes: &[Outcome], deleted: bool, context: &str) {
    for outcome in outcomes {
        match outcome {
            Outcome::Updated(_) | Outcome::Deleted => {}
            Outcome::Failed(StorageError::Concurrency(_)) => {}
            // A writer that lost to a *delete* finds no live resource at all.
            Outcome::Failed(StorageError::Resource(ResourceError::NotFound { .. })) if deleted => {}
            Outcome::Failed(StorageError::Resource(ResourceError::Gone { .. })) if deleted => {}
            other => panic!("{context}: a loser is a concurrency refusal, got {other:?}"),
        }
    }
}

/// `WRITERS` tasks all hold version 1 of the same resource and `update` it at
/// once, `rounds` times over. Exactly one writes version 2; the rest are
/// concurrency errors; the stored resource and the history are the winner's.
pub async fn concurrent_updates_from_the_same_version_admit_one<S>(
    backend: Arc<S>,
    base: &str,
    rounds: usize,
) where
    S: ResourceStorage + VersionedStorage + Send + Sync + 'static,
{
    let t = tenant(base, "update-race");

    for round in 0..rounds {
        let id = format!("race-{round}");
        let current = seed(backend.as_ref(), &t, &id).await;
        let barrier = Arc::new(Barrier::new(WRITERS));

        let mut tasks = Vec::new();
        for n in 0..WRITERS {
            let (backend, t, current, barrier, id) = (
                backend.clone(),
                t.clone(),
                current.clone(),
                barrier.clone(),
                id.clone(),
            );
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                match backend
                    .update(&t, &current, patient(&id, &format!("Writer{n}")))
                    .await
                {
                    Ok(stored) => Outcome::Updated(stored),
                    Err(e) => Outcome::Failed(e),
                }
            }));
        }
        let mut outcomes = Vec::new();
        for task in tasks {
            outcomes.push(task.await.expect("writer task"));
        }
        let context = format!("round {round}");

        let winners: Vec<&StoredResource> = outcomes
            .iter()
            .filter_map(|o| match o {
                Outcome::Updated(stored) => Some(stored),
                _ => None,
            })
            .collect();
        assert_eq!(
            winners.len(),
            1,
            "{context}: exactly one of {WRITERS} writers holding version 1 may write: {outcomes:?}"
        );
        assert_losers_are_concurrency_errors(&outcomes, false, &context);
        assert_eq!(winners[0].version_id(), "2", "{context}");

        let stored = backend
            .read(&t, "Patient", &id)
            .await
            .expect("read after race")
            .expect("still live");
        assert_eq!(stored.version_id(), "2", "{context}");
        assert_eq!(family(&stored), family(winners[0]), "{context}");

        assert_eq!(
            backend
                .list_versions(&t, "Patient", &id)
                .await
                .expect("list versions"),
            vec!["1".to_string(), "2".to_string()],
            "{context}: history has no gap and no duplicate"
        );
        let v2 = backend
            .vread(&t, "Patient", &id, "2")
            .await
            .expect("vread 2")
            .expect("version 2 exists");
        assert_eq!(
            family(&v2),
            family(winners[0]),
            "{context}: history's version 2 is the winner's content"
        );
    }
}

/// Half the tasks `update` from version 1 and half `delete_versioned` version
/// 1, all at once. Exactly one of them wins: either version 2 is an update and
/// the resource is live, or version 2 is the tombstone — never an update *and*
/// a delete of "version 1", which is the delete removing a version its caller
/// never saw.
pub async fn concurrent_update_and_versioned_delete_admit_one<S>(
    backend: Arc<S>,
    base: &str,
    rounds: usize,
) where
    S: ResourceStorage + VersionedStorage + Send + Sync + 'static,
{
    let t = tenant(base, "delete-race");

    for round in 0..rounds {
        let id = format!("race-{round}");
        let current = seed(backend.as_ref(), &t, &id).await;
        let barrier = Arc::new(Barrier::new(WRITERS));

        let mut tasks = Vec::new();
        for n in 0..WRITERS {
            let (backend, t, current, barrier, id) = (
                backend.clone(),
                t.clone(),
                current.clone(),
                barrier.clone(),
                id.clone(),
            );
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                if n % 2 == 0 {
                    match backend
                        .update(&t, &current, patient(&id, &format!("Writer{n}")))
                        .await
                    {
                        Ok(stored) => Outcome::Updated(stored),
                        Err(e) => Outcome::Failed(e),
                    }
                } else {
                    match backend.delete_versioned(&t, "Patient", &id, "1").await {
                        Ok(()) => Outcome::Deleted,
                        Err(e) => Outcome::Failed(e),
                    }
                }
            }));
        }
        let mut outcomes = Vec::new();
        for task in tasks {
            outcomes.push(task.await.expect("writer task"));
        }
        let context = format!("round {round}");

        let winners: Vec<&Outcome> = outcomes
            .iter()
            .filter(|o| matches!(o, Outcome::Updated(_) | Outcome::Deleted))
            .collect();
        assert_eq!(
            winners.len(),
            1,
            "{context}: exactly one of {WRITERS} writers holding version 1 may write: {outcomes:?}"
        );
        let deleted = matches!(winners[0], Outcome::Deleted);
        assert_losers_are_concurrency_errors(&outcomes, deleted, &context);

        let after = backend.read(&t, "Patient", &id).await;
        match winners[0] {
            Outcome::Updated(winner) => {
                let stored = after.expect("read after race").expect("still live");
                assert_eq!(stored.version_id(), "2", "{context}");
                assert_eq!(family(&stored), family(winner), "{context}");
            }
            _ => assert!(
                matches!(
                    after,
                    Ok(None) | Err(StorageError::Resource(ResourceError::Gone { .. }))
                ),
                "{context}: the delete won, so nothing is live: {after:?}"
            ),
        }

        assert_eq!(
            backend
                .list_versions(&t, "Patient", &id)
                .await
                .expect("list versions"),
            vec!["1".to_string(), "2".to_string()],
            "{context}: one write landed, as version 2"
        );
    }
}

/// Half the tasks `update` from version 1 and half `delete` with no
/// precondition. A plain delete removes whatever is current, so an update and
/// a delete may both land (as versions 2 and 3) — but only one of each, every
/// other task is refused cleanly, and the history is contiguous. On MongoDB
/// this is the path with the one bounded retry of a write conflict (#1405).
pub async fn concurrent_update_and_plain_delete_stay_consistent<S>(
    backend: Arc<S>,
    base: &str,
    rounds: usize,
) where
    S: ResourceStorage + VersionedStorage + Send + Sync + 'static,
{
    let t = tenant(base, "plain-delete-race");

    for round in 0..rounds {
        let id = format!("race-{round}");
        let current = seed(backend.as_ref(), &t, &id).await;
        let barrier = Arc::new(Barrier::new(WRITERS));

        let mut tasks = Vec::new();
        for n in 0..WRITERS {
            let (backend, t, current, barrier, id) = (
                backend.clone(),
                t.clone(),
                current.clone(),
                barrier.clone(),
                id.clone(),
            );
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                if n % 2 == 0 {
                    match backend
                        .update(&t, &current, patient(&id, &format!("Writer{n}")))
                        .await
                    {
                        Ok(stored) => Outcome::Updated(stored),
                        Err(e) => Outcome::Failed(e),
                    }
                } else {
                    match backend.delete(&t, "Patient", &id).await {
                        Ok(()) => Outcome::Deleted,
                        Err(e) => Outcome::Failed(e),
                    }
                }
            }));
        }
        let mut outcomes = Vec::new();
        for task in tasks {
            outcomes.push(task.await.expect("writer task"));
        }
        let context = format!("round {round}");

        let updates = outcomes
            .iter()
            .filter(|o| matches!(o, Outcome::Updated(_)))
            .count();
        let deletes = outcomes
            .iter()
            .filter(|o| matches!(o, Outcome::Deleted))
            .count();
        assert!(
            updates <= 1,
            "{context}: one update from version 1: {outcomes:?}"
        );
        assert!(
            deletes <= 1,
            "{context}: a resource is deleted once: {outcomes:?}"
        );
        assert!(
            updates + deletes >= 1,
            "{context}: somebody wins: {outcomes:?}"
        );
        assert_losers_are_concurrency_errors(&outcomes, true, &context);

        let expected_versions: Vec<String> =
            (1..=1 + updates + deletes).map(|v| v.to_string()).collect();
        assert_eq!(
            backend
                .list_versions(&t, "Patient", &id)
                .await
                .expect("list versions"),
            expected_versions,
            "{context}: one version per successful write, no gap, no duplicate: {outcomes:?}"
        );

        let after = backend.read(&t, "Patient", &id).await;
        if deletes == 1 {
            assert!(
                matches!(
                    after,
                    Ok(None) | Err(StorageError::Resource(ResourceError::Gone { .. }))
                ),
                "{context}: a delete landed, so nothing is live: {after:?}"
            );
        } else {
            let stored = after.expect("read after race").expect("still live");
            assert_eq!(stored.version_id(), "2", "{context}");
        }
    }
}

/// `delete_versioned` is a compare-and-swap on the *current* version: a stale
/// version is refused and deletes nothing, the current one deletes, and a
/// second delete of the same version finds nothing live.
pub async fn versioned_delete_is_a_compare_and_swap<S>(backend: &S, base: &str)
where
    S: ResourceStorage + VersionedStorage,
{
    let t = tenant(base, "delete-cas");
    let v1 = seed(backend, &t, "cas").await;
    let v2 = backend
        .update(&t, &v1, patient("cas", "Updated"))
        .await
        .expect("update to v2");
    assert_eq!(v2.version_id(), "2");

    // The version the client saw is gone: refuse, and leave v2 live.
    let stale = backend.delete_versioned(&t, "Patient", "cas", "1").await;
    assert!(
        matches!(stale, Err(StorageError::Concurrency(_))),
        "a stale versioned delete is a concurrency error, got {stale:?}"
    );
    let live = backend
        .read(&t, "Patient", "cas")
        .await
        .expect("read")
        .expect("a refused delete deletes nothing");
    assert_eq!(live.version_id(), "2");
    assert_eq!(family(&live), "Updated");

    // `delete_with_match` takes an `If-Match` field value and goes the same way.
    let stale = backend
        .delete_with_match(&t, "Patient", "cas", "W/\"1\"")
        .await;
    assert!(
        matches!(stale, Err(StorageError::Concurrency(_))),
        "{stale:?}"
    );

    backend
        .delete_versioned(&t, "Patient", "cas", "2")
        .await
        .expect("the current version deletes");
    let after = backend.read(&t, "Patient", "cas").await;
    assert!(
        matches!(
            after,
            Ok(None) | Err(StorageError::Resource(ResourceError::Gone { .. }))
        ),
        "{after:?}"
    );
    assert_eq!(
        backend
            .list_versions(&t, "Patient", "cas")
            .await
            .expect("list versions"),
        vec!["1".to_string(), "2".to_string(), "3".to_string()],
        "the tombstone is version 3"
    );

    let again = backend.delete_versioned(&t, "Patient", "cas", "2").await;
    assert!(
        matches!(
            again,
            Err(StorageError::Resource(ResourceError::NotFound { .. }))
        ),
        "nothing live is left to delete: {again:?}"
    );
}
