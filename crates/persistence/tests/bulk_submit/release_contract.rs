// Shared bulk-submit lease-release regressions (#1531), run against every
// backend that implements `SubmitClaimStrategy`. The including module aliases
// the library as `persistence`.
use persistence::core::WorkerId;
use persistence::core::bulk_submit::{BulkSubmitProvider, ManifestStatus, SubmissionId};
use persistence::core::bulk_submit_worker::{ManifestLease, SubmitClaimStrategy};
use persistence::tenant::TenantContext;
use std::time::Duration;

/// Claims until `worker` holds `manifest_id` of `submission`. Other suites may
/// share the store, so any other manifest claimed on the way is released back
/// to `pending` rather than left leased.
async fn claim_manifest<B>(
    backend: &B,
    worker: &WorkerId,
    submission: &SubmissionId,
    manifest_id: &str,
) -> ManifestLease
where
    B: SubmitClaimStrategy,
{
    let mut held = Vec::new();
    let mut reached = None;
    for _ in 0..256 {
        let Some(lease) = backend
            .claim_next_manifest(worker, Duration::from_secs(300))
            .await
            .unwrap()
        else {
            break;
        };
        if &lease.submission_id == submission && lease.manifest_id == manifest_id {
            reached = Some(lease);
            break;
        }
        held.push(lease);
    }
    for lease in held {
        backend.release(lease).await.unwrap();
    }
    reached.expect("the fixture's manifest is claimable")
}

async fn manifest_status<B>(
    backend: &B,
    tenant: &TenantContext,
    submission: &SubmissionId,
    manifest_id: &str,
) -> ManifestStatus
where
    B: BulkSubmitProvider,
{
    backend
        .get_manifest(tenant, submission, manifest_id)
        .await
        .unwrap()
        .expect("the fixture's manifest exists")
        .status
}

/// A released manifest goes back to `pending` and another worker can claim it
/// straight away, under a newer fencing token. After that reclaim, a release
/// by the first worker is a no-op: it must not re-queue a manifest that
/// someone else is running (#1531).
pub async fn release_requeues_and_fences_out_a_zombie<B>(backend: &B, tenant: &TenantContext)
where
    B: BulkSubmitProvider + SubmitClaimStrategy,
{
    let submission = SubmissionId::generate("release-contract");
    backend
        .create_submission(tenant, &submission, None)
        .await
        .unwrap();
    let manifest_id = backend
        .add_manifest(
            tenant,
            &submission,
            Some("http://provider/release.json"),
            None,
        )
        .await
        .unwrap()
        .manifest_id;

    let worker_a = WorkerId::new(format!("release-a-{}", uuid::Uuid::new_v4()));
    let lease_a = claim_manifest(backend, &worker_a, &submission, &manifest_id).await;
    assert_eq!(
        manifest_status(backend, tenant, &submission, &manifest_id).await,
        ManifestStatus::Processing
    );

    assert!(
        backend.release(lease_a.clone()).await.unwrap(),
        "the holder's release takes effect"
    );
    assert_eq!(
        manifest_status(backend, tenant, &submission, &manifest_id).await,
        ManifestStatus::Pending,
        "a released manifest is queued again"
    );
    assert!(
        backend.heartbeat(&lease_a).await.is_err(),
        "a released lease can no longer be renewed"
    );

    let worker_b = WorkerId::new(format!("release-b-{}", uuid::Uuid::new_v4()));
    let lease_b = claim_manifest(backend, &worker_b, &submission, &manifest_id).await;
    assert!(lease_b.fencing_token > lease_a.fencing_token);

    assert!(
        !backend.release(lease_a).await.unwrap(),
        "a zombie's release after the reclaim is a no-op"
    );
    assert_eq!(
        manifest_status(backend, tenant, &submission, &manifest_id).await,
        ManifestStatus::Processing
    );
    backend
        .heartbeat(&lease_b)
        .await
        .expect("worker B still holds its lease");

    assert!(backend.release(lease_b.clone()).await.unwrap());
    assert!(
        !backend.release(lease_b).await.unwrap(),
        "releasing twice is a no-op"
    );
}

/// A release that races an abort leaves the abort's verdict alone: the
/// manifest is no longer `processing`, so it is not re-queued.
pub async fn release_after_abort_is_a_no_op<B>(backend: &B, tenant: &TenantContext)
where
    B: BulkSubmitProvider + SubmitClaimStrategy,
{
    let submission = SubmissionId::generate("release-abort-contract");
    backend
        .create_submission(tenant, &submission, None)
        .await
        .unwrap();
    let manifest_id = backend
        .add_manifest(
            tenant,
            &submission,
            Some("http://provider/release-abort.json"),
            None,
        )
        .await
        .unwrap()
        .manifest_id;

    let worker = WorkerId::new(format!("release-abort-{}", uuid::Uuid::new_v4()));
    let lease = claim_manifest(backend, &worker, &submission, &manifest_id).await;
    backend
        .abort_submission(tenant, &submission, "release contract")
        .await
        .unwrap();
    let aborted = manifest_status(backend, tenant, &submission, &manifest_id).await;
    assert_ne!(aborted, ManifestStatus::Processing);

    assert!(!backend.release(lease).await.unwrap());
    assert_eq!(
        manifest_status(backend, tenant, &submission, &manifest_id).await,
        aborted,
        "the abort's verdict stands"
    );
}
