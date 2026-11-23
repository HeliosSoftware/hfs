// Shared bulk-submit claim-eligibility regressions for backends whose synchronous
// `process_entries` leaves the manifest in `processing` (SQLite, PostgreSQL,
// MongoDB); S3 settles it inside the call and pins the rule in its own test. The
// including module aliases the library as `persistence`.
use persistence::core::WorkerId;
use persistence::core::bulk_submit::{
    BulkProcessingOptions, BulkSubmitProvider, ManifestStatus, NdjsonEntry, SubmissionId,
};
use persistence::core::bulk_submit_worker::SubmitClaimStrategy;
use persistence::tenant::TenantContext;
use serde_json::json;
use std::time::Duration;

/// A manifest that a synchronous `process_entries` call moved from `pending` to
/// `processing` holds no lease, but it is being ingested right now, so no worker
/// may claim it (#1530). A pending manifest registered after it stays claimable,
/// which proves the claim scan reached this submission rather than stopping early.
///
/// Other suites may share the store, so manifests that are not this fixture's are
/// released back to `pending` instead of being left leased.
pub async fn unleased_processing_is_not_claimable<B>(backend: &B, tenant: &TenantContext)
where
    B: BulkSubmitProvider + SubmitClaimStrategy,
{
    let submission = SubmissionId::generate("unleased-processing-contract");
    backend
        .create_submission(tenant, &submission, None)
        .await
        .unwrap();
    let in_flight = backend
        .add_manifest(
            tenant,
            &submission,
            Some("http://provider/in-flight.json"),
            None,
        )
        .await
        .unwrap()
        .manifest_id;
    backend
        .process_entries(
            tenant,
            &submission,
            &in_flight,
            vec![NdjsonEntry::new(
                1,
                "Patient",
                json!({"resourceType": "Patient", "id": "unleased-processing"}),
            )],
            &BulkProcessingOptions::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        backend
            .get_manifest(tenant, &submission, &in_flight)
            .await
            .unwrap()
            .expect("in-flight manifest exists")
            .status,
        ManifestStatus::Processing,
        "process_entries promotes a pending manifest to processing"
    );
    let queued = backend
        .add_manifest(
            tenant,
            &submission,
            Some("http://provider/queued.json"),
            None,
        )
        .await
        .unwrap()
        .manifest_id;

    let worker = WorkerId::new(format!("claim-contract-{}", uuid::Uuid::new_v4()));
    let mut held = Vec::new();
    let mut reached = None;
    for _ in 0..256 {
        let Some(lease) = backend
            .claim_next_manifest(&worker, Duration::from_secs(300))
            .await
            .unwrap()
        else {
            break;
        };
        if lease.submission_id == submission {
            assert_ne!(
                lease.manifest_id, in_flight,
                "a processing manifest with no lease must not be claimable"
            );
            if lease.manifest_id == queued {
                reached = Some(lease);
                break;
            }
        }
        held.push(lease);
    }
    for lease in held {
        backend.release(lease).await.unwrap();
    }
    let lease = reached.expect("the pending manifest queued behind it stays claimable");
    backend.release(lease).await.unwrap();

    assert_eq!(
        backend
            .get_manifest(tenant, &submission, &in_flight)
            .await
            .unwrap()
            .expect("in-flight manifest exists")
            .status,
        ManifestStatus::Processing,
        "the in-flight manifest is left to its synchronous caller"
    );
}

/// Registers `count` fetchable manifests under one new submission of `tenant`,
/// a few milliseconds apart so their arrival order is unambiguous.
async fn queue_manifests<B>(backend: &B, tenant: &TenantContext, count: usize) -> SubmissionId
where
    B: BulkSubmitProvider + SubmitClaimStrategy,
{
    let submission = SubmissionId::generate("claim-fairness-contract");
    backend
        .create_submission(tenant, &submission, None)
        .await
        .unwrap();
    for i in 0..count {
        backend
            .add_manifest(
                tenant,
                &submission,
                Some(&format!("http://provider/{i}.json")),
                None,
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(3)).await;
    }
    submission
}

/// A tenant with a manifest in flight waits behind the tenants that have none
/// (#2760), in both shapes a worker pool produces:
///
/// - one claim after another: the busy tenant's backlog is longer than any
///   candidate window a backend scans, and the other tenant's one manifest,
///   queued last, is still the second claim;
/// - two claims at the same instant, which is what idle workers polling on one
///   cadence do: neither has seen the other's lease, and they must still end up
///   on different tenants.
///
/// The store's claim queue must be empty: this asserts on what the very next
/// claims return.
pub async fn a_tenant_with_work_in_flight_waits_behind_the_others<B>(
    backend: &B,
    busy: &TenantContext,
    other: &TenantContext,
) where
    B: BulkSubmitProvider + SubmitClaimStrategy,
{
    const LEASE: Duration = Duration::from_secs(300);
    let backlog = queue_manifests(backend, busy, 20).await;
    let single = queue_manifests(backend, other, 1).await;
    let first_worker = WorkerId::new("claim-fairness-1");
    let second_worker = WorkerId::new("claim-fairness-2");

    let first = backend
        .claim_next_manifest(&first_worker, LEASE)
        .await
        .unwrap()
        .expect("the oldest manifest is claimable");
    assert_eq!(
        first.submission_id, backlog,
        "arrival order while nobody is busy"
    );
    let second = backend
        .claim_next_manifest(&second_worker, LEASE)
        .await
        .unwrap()
        .expect("the other tenant's manifest is claimable");
    assert_eq!(
        second.submission_id, single,
        "the other tenant goes before the rest of the busy tenant's backlog"
    );
    let third = backend
        .claim_next_manifest(&second_worker, LEASE)
        .await
        .unwrap()
        .expect("the busy tenant's backlog is still claimable");
    assert_eq!(third.submission_id, backlog);
    for lease in [first, second, third] {
        assert!(backend.release(lease).await.unwrap());
    }

    for round in 0..8 {
        let (a, b) = tokio::join!(
            backend.claim_next_manifest(&first_worker, LEASE),
            backend.claim_next_manifest(&second_worker, LEASE),
        );
        let a = a.unwrap().expect("two manifests are claimable");
        let b = b.unwrap().expect("two manifests are claimable");
        let mut claimed = [a.submission_id.clone(), b.submission_id.clone()];
        claimed.sort_by_key(|id| *id != backlog);
        assert_eq!(
            claimed,
            [backlog.clone(), single.clone()],
            "round {round}: simultaneous claims land on different tenants"
        );
        for lease in [a, b] {
            assert!(backend.release(lease).await.unwrap());
        }
    }
}
