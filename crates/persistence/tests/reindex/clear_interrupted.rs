//! #2278: a `$reindex` with `clearExisting` must not leave the scope
//! unsearchable when the job does not finish. The old index stays in place until
//! the rebuild completes, and only a completed run sweeps the entries of
//! resources that no longer exist at the source.
//!
//! The orphan is seeded without touching backend internals: the resource is
//! created (and so indexed) normally, then [`InterruptibleSource`] hides it, so
//! the run sees a source that no longer has it while its index entries remain.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::error::{BackendError, StorageResult};
use helios_persistence::search::{
    ReindexOperation, ReindexProgress, ReindexRequest, ReindexSource, ReindexStatus, ReindexTarget,
    ResourcePage, TenantSearchRegistries,
};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchParamType, SearchParameter, SearchQuery, SearchValue, StoredResource,
};
use serde_json::json;

/// What the source does on a page read.
#[derive(Clone, Copy)]
pub enum Interruption {
    /// Serve every page.
    None,
    /// Hold the Nth page read until [`InterruptibleSource::resume`] is released.
    PauseOn(usize),
    /// Fail every page read from the Nth on, with a transient error.
    FailFrom(usize),
}

/// Wraps a real backend as a [`ReindexSource`]: hides some ids and interrupts
/// the page reads.
pub struct InterruptibleSource<B> {
    inner: Arc<B>,
    hidden: HashSet<String>,
    interruption: Interruption,
    calls: AtomicUsize,
    pub reached: tokio::sync::Notify,
    pub resume: tokio::sync::Semaphore,
}

impl<B> InterruptibleSource<B> {
    pub fn new(inner: Arc<B>, hidden: &[&str], interruption: Interruption) -> Arc<Self> {
        Arc::new(Self {
            inner,
            hidden: hidden.iter().map(|id| id.to_string()).collect(),
            interruption,
            calls: AtomicUsize::new(0),
            reached: tokio::sync::Notify::new(),
            resume: tokio::sync::Semaphore::new(0),
        })
    }

    fn visible(&self, resources: Vec<StoredResource>) -> Vec<StoredResource> {
        resources
            .into_iter()
            .filter(|r| !self.hidden.contains(r.id()))
            .collect()
    }
}

#[async_trait::async_trait]
impl<B: ReindexSource + 'static> ReindexSource for InterruptibleSource<B> {
    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>> {
        self.inner.list_resource_types(tenant).await
    }

    async fn count_resources(&self, tenant: &TenantContext, t: &str) -> StorageResult<u64> {
        let total = self.inner.count_resources(tenant, t).await?;
        Ok(total.saturating_sub(self.hidden.len() as u64))
    }

    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> StorageResult<ResourcePage> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if let Interruption::FailFrom(n) = self.interruption
            && call >= n
        {
            return Err(BackendError::Unavailable {
                backend_name: "interrupted".into(),
                message: "injected page failure".into(),
            }
            .into());
        }
        let mut page = self
            .inner
            .fetch_resources_page(tenant, resource_type, cursor, limit)
            .await?;
        page.resources = self.visible(page.resources);
        if let Interruption::PauseOn(n) = self.interruption
            && call == n
        {
            self.reached.notify_one();
            self.resume.acquire().await.unwrap().forget();
        }
        Ok(page)
    }

    async fn fetch_resources_by_ids(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        ids: &[String],
    ) -> StorageResult<Vec<StoredResource>> {
        let found = self
            .inner
            .fetch_resources_by_ids(tenant, resource_type, ids)
            .await?;
        Ok(self.visible(found))
    }
}

fn tenant(prefix: &str) -> TenantContext {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    TenantContext::new(
        TenantId::new(format!("{prefix}-{suffix}")),
        TenantPermissions::full_access(),
    )
}

/// Ids of the tenant's Patients that a `gender=female` search finds.
async fn found<B: SearchProvider>(backend: &B, tenant: &TenantContext) -> Vec<String> {
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "gender".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("female")],
        chain: vec![],
        components: vec![],
    });
    let mut ids: Vec<String> = backend
        .search(tenant, &query)
        .await
        .unwrap()
        .resources
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect();
    ids.sort();
    ids
}

/// Creates `ids` as Patients, all indexed.
async fn seed<B: ResourceStorage>(backend: &B, tenant: &TenantContext, ids: &[&str]) {
    for id in ids {
        backend
            .create(
                tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": id, "gender": "female"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
}

async fn wait_finished(op: &ReindexOperation, job_id: &str) -> ReindexStatus {
    for _ in 0..800 {
        let progress = op.get_progress(job_id).await.unwrap();
        if progress.status.is_finished() {
            return progress.status;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;
    }
    panic!("reindex did not finish");
}

/// Waits until the job's task has returned, and gives back the last progress
/// read before it was seen to have returned (for diagnostics only: it can
/// predate the task's last page by one poll).
///
/// A terminal status is not enough after a cancel: `ReindexOperation::cancel`
/// writes `Cancelled` at once, while the page the run has in hand may still be
/// read or written (the driver stops only at a page boundary), so the index can
/// still be changing (#2409). A finished job is evicted only once its task has
/// returned (see `ReindexOperation::cleanup_old_jobs`), so a zero-retention
/// cleanup evicting it is the signal.
async fn wait_until_task_returns(op: &ReindexOperation, job_id: &str) -> ReindexProgress {
    tokio::time::timeout(tokio::time::Duration::from_secs(60), async {
        loop {
            let last = op
                .get_progress(job_id)
                .await
                .expect("a job is kept while its task runs");
            op.cleanup_old_jobs(0);
            if op.get_progress(job_id).await.is_none() {
                return last;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("the cancelled reindex task did not return")
}

const IDS: [&str; 5] = ["p1", "p2", "p3", "p4", "p5"];

/// Cancelling a `clearExisting` run after its first page leaves every resource
/// searchable, not just the ones already re-paginated. The page the run was
/// reading when it was cancelled is still written in full after `Cancelled` is
/// reported, so the check waits for the run's task to return, not just for the
/// status. This pins the final state after a cancel, not whether a cancel could
/// interrupt a page write in progress: that safety rests on the driver checking
/// for a cancel only between pages (the `try_recv` checks at the top of the
/// type, id-batch and page loops in `crates/persistence/src/search/reindex.rs`).
pub async fn assert_cancelled_clear_existing_keeps_the_index<B>(
    backend: Arc<B>,
    registries: Arc<TenantSearchRegistries>,
) where
    B: ResourceStorage + SearchProvider + ReindexSource + ReindexTarget + 'static,
{
    let tenant = tenant("clear-cancel");
    seed(&*backend, &tenant, &IDS).await;
    assert_eq!(found(&*backend, &tenant).await, IDS);

    let source = InterruptibleSource::new(backend.clone(), &[], Interruption::PauseOn(1));
    let op = ReindexOperation::with_parts(source.clone(), vec![backend.clone()], registries);
    let job_id = op
        .start(
            tenant.clone(),
            ReindexRequest::for_types(["Patient"])
                .clear_existing()
                .with_batch_size(2),
            None,
        )
        .await
        .unwrap();

    source.reached.notified().await;
    op.cancel(&job_id).await.unwrap();
    source.resume.add_permits(1);
    assert_eq!(wait_finished(&op, &job_id).await, ReindexStatus::Cancelled);
    // Cancel reports `Cancelled` before the in-flight page is written, so look
    // at the index only once the run's task has returned (#2409).
    let progress = wait_until_task_returns(&op, &job_id).await;

    assert_eq!(
        found(&*backend, &tenant).await,
        IDS,
        "a cancelled clearExisting run must leave the whole scope searchable once its task has returned: {progress:?}"
    );
}

/// A read failure after the first page ends the run `Failed`, with the scope
/// still fully searchable.
pub async fn assert_failed_read_keeps_the_index<B>(
    backend: Arc<B>,
    registries: Arc<TenantSearchRegistries>,
) where
    B: ResourceStorage + SearchProvider + ReindexSource + ReindexTarget + 'static,
{
    let tenant = tenant("clear-fail");
    seed(&*backend, &tenant, &IDS).await;

    let source = InterruptibleSource::new(backend.clone(), &[], Interruption::FailFrom(2));
    let op = ReindexOperation::with_parts(source, vec![backend.clone()], registries);
    let job_id = op
        .start(
            tenant.clone(),
            ReindexRequest::for_types(["Patient"])
                .clear_existing()
                .with_batch_size(2),
            None,
        )
        .await
        .unwrap();

    assert_eq!(wait_finished(&op, &job_id).await, ReindexStatus::Failed);
    assert_eq!(
        found(&*backend, &tenant).await,
        IDS,
        "a clearExisting run that fails mid-way must leave the whole scope searchable"
    );
}

/// A completed `clearExisting` run removes the entries of a resource that the
/// source no longer has, and only those.
pub async fn assert_completed_clear_existing_sweeps_orphans<B>(
    backend: Arc<B>,
    registries: Arc<TenantSearchRegistries>,
) where
    B: ResourceStorage + SearchProvider + ReindexSource + ReindexTarget + 'static,
{
    let tenant = tenant("clear-sweep");
    let other = self::tenant("clear-sweep-other");
    seed(&*backend, &tenant, &IDS).await;
    seed(&*backend, &other, &IDS).await;
    assert_eq!(found(&*backend, &tenant).await, IDS);

    let source = InterruptibleSource::new(backend.clone(), &["p3"], Interruption::None);
    let op = ReindexOperation::with_parts(source, vec![backend.clone()], registries);
    let job_id = op
        .start(
            tenant.clone(),
            ReindexRequest::for_types(["Patient"])
                .clear_existing()
                .with_batch_size(2),
            None,
        )
        .await
        .unwrap();

    assert_eq!(wait_finished(&op, &job_id).await, ReindexStatus::Completed);
    let progress = op.get_progress(&job_id).await.unwrap();
    assert!(progress.errors.is_empty(), "{:?}", progress.errors);
    // `orphans_removed` counts index entries, not resources.
    assert!(progress.orphans_removed > 0, "{progress:?}");

    assert_eq!(
        found(&*backend, &tenant).await,
        ["p1", "p2", "p4", "p5"],
        "the orphan's entries are swept and every live resource stays indexed"
    );
    assert_eq!(
        found(&*backend, &other).await,
        IDS,
        "the sweep never leaves its tenant"
    );
}
