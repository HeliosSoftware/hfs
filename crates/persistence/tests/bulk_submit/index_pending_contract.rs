// Shared #1125 deferred-rebuild marker regressions (#1283), run against every
// backend whose job store records it. The including module aliases the
// library as `persistence`.
use persistence::core::WorkerId;
use persistence::core::bulk_submit::{BulkSubmitProvider, SubmissionId};
use persistence::core::bulk_submit_worker::{
    ManifestLease, PendingReindex, SubmitClaimStrategy, SubmitWorkerStorage,
};
use persistence::tenant::{TenantContext, TenantId, TenantPermissions};
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

/// The fixture's entry in the startup listing. Other suites may share the
/// store, so the listing is searched rather than expected to hold only it.
async fn pending_for<B>(backend: &B, manifest_id: &str) -> Option<PendingReindex>
where
    B: SubmitWorkerStorage,
{
    backend
        .list_manifests_awaiting_reindex(10_000)
        .await
        .unwrap()
        .into_iter()
        .find(|pending| pending.manifest_id == manifest_id)
}

/// A marked manifest is what a restarted server finds and re-fires, for the
/// tenant and submission it was ingested under; clearing it from another
/// tenant does nothing, and clearing it from its own removes it (#1283).
///
/// `stores_types` is whether the backend keeps the types it was marked with.
/// SQLite derives them from the manifest's entry results instead, and this
/// fixture ingests none.
pub async fn marked_manifest_is_listed_until_cleared<B>(
    backend: &B,
    tenant: &TenantContext,
    stores_types: bool,
) where
    B: BulkSubmitProvider + SubmitClaimStrategy + SubmitWorkerStorage,
{
    assert!(backend.records_index_pending());

    let submission = SubmissionId::generate("index-pending-contract");
    backend
        .create_submission(tenant, &submission, None)
        .await
        .unwrap();
    let manifest_id = backend
        .add_manifest(
            tenant,
            &submission,
            Some("http://provider/index-pending.json"),
            None,
        )
        .await
        .unwrap()
        .manifest_id;
    assert!(
        pending_for(backend, &manifest_id).await.is_none(),
        "a manifest owes no rebuild until it is marked"
    );

    let worker = WorkerId::new(format!("index-pending-{}", uuid::Uuid::new_v4()));
    let lease = claim_manifest(backend, &worker, &submission, &manifest_id).await;
    let types = vec!["Patient".to_string(), "Observation".to_string()];
    backend
        .mark_manifest_index_pending(&lease, &types)
        .await
        .unwrap();
    backend.release(lease).await.unwrap();

    let pending = pending_for(backend, &manifest_id)
        .await
        .expect("a marked manifest is listed for the next startup");
    assert_eq!(pending.tenant.tenant_id(), tenant.tenant_id());
    assert_eq!(pending.submission, submission);
    if stores_types {
        assert_eq!(
            pending.resource_types,
            vec!["Observation".to_string(), "Patient".to_string()],
            "the listing returns the marked types, sorted"
        );
    }

    let other = TenantContext::new(
        TenantId::new(format!("{}-other", tenant.tenant_id().as_str())),
        TenantPermissions::full_access(),
    );
    backend
        .clear_manifest_index_pending(&other, &manifest_id)
        .await
        .unwrap();
    assert!(
        pending_for(backend, &manifest_id).await.is_some(),
        "another tenant cannot clear the marker"
    );

    backend
        .clear_manifest_index_pending(tenant, &manifest_id)
        .await
        .unwrap();
    assert!(
        pending_for(backend, &manifest_id).await.is_none(),
        "a cleared manifest is not re-fired"
    );
    backend
        .clear_manifest_index_pending(tenant, &manifest_id)
        .await
        .expect("clearing twice is a no-op");
}
