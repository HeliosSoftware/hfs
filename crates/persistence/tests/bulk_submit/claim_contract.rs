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
