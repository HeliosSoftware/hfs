//! Per-type reconciliation of a secondary (search) backend against the primary.
//!
//! A reconciliation run walks the ids of one resource type on both sides in
//! pages, ordered by the bytes of the id, and merge-joins the two streams:
//!
//! - an id only on the primary is **missing** from the secondary;
//! - an id only on the secondary is **extra**;
//! - an id on both sides with a different `version_id` is **stale**.
//!
//! Memory is bounded by one page per side plus the (page-sized) batch of
//! suspects awaiting a recheck plus the id samples, regardless of how many
//! resources the type holds. Suspects are re-read on both sides before they
//! are reported, so a write that was in flight during the scan does not show
//! up as drift.

use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::error::StorageResult;
use crate::tenant::TenantContext;

/// Smallest accepted page size.
pub const MIN_PAGE_SIZE: u32 = 1;
/// Largest accepted page size.
pub const MAX_PAGE_SIZE: u32 = 10_000;

/// One side of a reconciliation: a lightweight, ordered view of
/// `(id, version_id)` pairs.
///
/// Contract for [`scan_id_versions`](ReconcileScan::scan_id_versions):
///
/// - only live resources are returned (not deleted, and never the copies of
///   contained resources a search index may hold);
/// - entries are ordered strictly by the **bytes** of the id, and only ids
///   greater than `after_id` are returned;
/// - at most `limit` entries are returned. A page shorter than `limit` means
///   there is nothing after it.
#[async_trait]
pub trait ReconcileScan: Send + Sync {
    /// Resource types this side holds for the tenant.
    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>>;

    /// Returns one keyset page of `(id, version_id)` pairs.
    async fn scan_id_versions(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        after_id: Option<&str>,
        limit: u32,
    ) -> StorageResult<Vec<(String, String)>>;

    /// Current `version_id` of one live resource, or `None` when it is absent
    /// or deleted. Used to recheck suspects.
    async fn current_version(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<String>>;
}

/// What to do with the divergences a run confirms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepairMode {
    /// Only report.
    #[default]
    None,
    /// Record each divergence in the needs-reindex ledger for the repair job.
    Queue,
    /// Re-sync each divergent resource from the primary right away.
    Inline,
}

/// The kind of divergence found for one id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drift {
    /// On the primary only.
    Missing,
    /// On the secondary only.
    Extra,
    /// On both sides with different versions.
    Stale,
}

/// What a [`ReconcileRepair`] did with one divergence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairOutcome {
    /// Recorded in the ledger.
    Queued,
    /// Fixed on the secondary.
    Repaired,
}

/// Applies the repair for one confirmed divergence. Implemented by the
/// composite storage; idempotent by construction (it re-reads the primary).
#[async_trait]
pub trait ReconcileRepair: Send + Sync {
    /// Repairs (or queues the repair of) one resource.
    async fn repair(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        drift: Drift,
        mode: RepairMode,
    ) -> StorageResult<RepairOutcome>;
}

/// Tuning for a reconciliation run.
#[derive(Debug, Clone)]
pub struct ReconcileOptions {
    /// Ids per page and per side; clamped to
    /// [`MIN_PAGE_SIZE`]..=[`MAX_PAGE_SIZE`].
    pub page_size: u32,
    /// Pause after every page fetched, to keep the scan off the hot path.
    pub inter_page_delay: Duration,
    /// Maximum ids kept as a sample per category.
    pub sample_limit: usize,
    /// What to do with confirmed divergences.
    pub repair: RepairMode,
    /// Stops the run between pages; the result is then `completed == false`.
    pub cancel: CancellationToken,
}

impl Default for ReconcileOptions {
    fn default() -> Self {
        Self {
            page_size: 500,
            inter_page_delay: Duration::from_millis(50),
            sample_limit: 20,
            repair: RepairMode::None,
            cancel: CancellationToken::new(),
        }
    }
}

impl ReconcileOptions {
    fn effective_page_size(&self) -> u32 {
        self.page_size.clamp(MIN_PAGE_SIZE, MAX_PAGE_SIZE)
    }
}

/// Count plus a bounded sample of ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DriftReport {
    /// Total ids in this category.
    pub count: u64,
    /// The first ids found, up to `sample_limit`.
    pub sample: Vec<String>,
}

impl DriftReport {
    fn push(&mut self, id: &str, sample_limit: usize) {
        self.count += 1;
        if self.sample.len() < sample_limit {
            self.sample.push(id.to_string());
        }
    }
}

/// Result of reconciling one resource type.
#[derive(Debug, Clone, Default)]
pub struct ReconciliationResult {
    /// Resource type that was compared.
    pub resource_type: String,
    /// Live ids read from the primary.
    pub primary_scanned: u64,
    /// Live ids read from the secondary.
    pub secondary_scanned: u64,
    /// On the primary only.
    pub missing: DriftReport,
    /// On the secondary only.
    pub extra: DriftReport,
    /// On both with different versions.
    pub stale: DriftReport,
    /// `missing + extra + stale`.
    pub differences: u64,
    /// Divergences recorded in the ledger.
    pub queued: u64,
    /// Divergences fixed inline.
    pub repaired: u64,
    /// Divergences whose repair failed.
    pub repair_failed: u64,
    /// Pages fetched from either side.
    pub pages: u64,
    /// Wall-clock time of the run.
    pub elapsed: Duration,
    /// `false` when the run was cancelled before it reached the end.
    pub completed: bool,
}

/// One side's stream of `(id, version_id)`, holding at most one page.
struct PageStream<'a> {
    scan: &'a dyn ReconcileScan,
    buf: VecDeque<(String, String)>,
    after: Option<String>,
    exhausted: bool,
}

impl<'a> PageStream<'a> {
    fn new(scan: &'a dyn ReconcileScan) -> Self {
        Self {
            scan,
            buf: VecDeque::new(),
            after: None,
            exhausted: false,
        }
    }

    /// Makes sure the buffer holds the next entry if there is one. Returns
    /// whether a page was fetched.
    async fn fill(
        &mut self,
        tenant: &TenantContext,
        resource_type: &str,
        limit: u32,
    ) -> StorageResult<bool> {
        if !self.buf.is_empty() || self.exhausted {
            return Ok(false);
        }
        let page = self
            .scan
            .scan_id_versions(tenant, resource_type, self.after.as_deref(), limit)
            .await?;
        if (page.len() as u64) < u64::from(limit) {
            self.exhausted = true;
        }
        if let Some((last, _)) = page.last() {
            self.after = Some(last.clone());
        } else {
            self.exhausted = true;
        }
        self.buf = page.into();
        Ok(true)
    }

    fn peek(&self) -> Option<&(String, String)> {
        self.buf.front()
    }

    fn pop(&mut self) {
        self.buf.pop_front();
    }
}

/// Compares a secondary backend with the primary, one resource type at a time.
#[derive(Debug, Default)]
pub struct SyncReconciler;

impl SyncReconciler {
    /// Creates a reconciler.
    pub fn new() -> Self {
        Self
    }

    /// Resource types on either side, deduplicated and sorted, so that a type
    /// that only the secondary still holds is reported as extra.
    pub async fn resource_types(
        &self,
        tenant: &TenantContext,
        primary: &dyn ReconcileScan,
        secondary: &dyn ReconcileScan,
    ) -> StorageResult<Vec<String>> {
        let mut seen: HashSet<String> = primary
            .list_resource_types(tenant)
            .await?
            .into_iter()
            .collect();
        seen.extend(secondary.list_resource_types(tenant).await?);
        let mut types: Vec<String> = seen.into_iter().collect();
        types.sort();
        Ok(types)
    }

    /// Reconciles one resource type.
    pub async fn reconcile(
        &self,
        tenant: &TenantContext,
        primary: &dyn ReconcileScan,
        secondary: &dyn ReconcileScan,
        resource_type: &str,
        opts: &ReconcileOptions,
        repairer: Option<&dyn ReconcileRepair>,
    ) -> StorageResult<ReconciliationResult> {
        let started = Instant::now();
        let limit = opts.effective_page_size();
        let mut result = ReconciliationResult {
            resource_type: resource_type.to_string(),
            ..Default::default()
        };
        let mut p = PageStream::new(primary);
        let mut s = PageStream::new(secondary);
        let mut suspects: Vec<(Drift, String)> = Vec::new();
        let mut completed = true;

        loop {
            if opts.cancel.is_cancelled() {
                completed = false;
                break;
            }
            for stream_is_primary in [true, false] {
                let stream = if stream_is_primary { &mut p } else { &mut s };
                if stream.fill(tenant, resource_type, limit).await? {
                    result.pages += 1;
                    if !opts.inter_page_delay.is_zero() {
                        tokio::time::sleep(opts.inter_page_delay).await;
                    }
                }
            }

            let step = match (p.peek(), s.peek()) {
                (None, None) => break,
                (Some(_), None) => Step::PrimaryOnly,
                (None, Some(_)) => Step::SecondaryOnly,
                (Some((a, _)), Some((b, _))) => match a.as_bytes().cmp(b.as_bytes()) {
                    std::cmp::Ordering::Less => Step::PrimaryOnly,
                    std::cmp::Ordering::Greater => Step::SecondaryOnly,
                    std::cmp::Ordering::Equal => Step::Both,
                },
            };
            match step {
                Step::PrimaryOnly => {
                    if let Some((id, _)) = p.peek() {
                        suspects.push((Drift::Missing, id.clone()));
                    }
                    p.pop();
                    result.primary_scanned += 1;
                }
                Step::SecondaryOnly => {
                    if let Some((id, _)) = s.peek() {
                        suspects.push((Drift::Extra, id.clone()));
                    }
                    s.pop();
                    result.secondary_scanned += 1;
                }
                Step::Both => {
                    if let (Some((id, pv)), Some((_, sv))) = (p.peek(), s.peek())
                        && pv != sv
                    {
                        suspects.push((Drift::Stale, id.clone()));
                    }
                    p.pop();
                    s.pop();
                    result.primary_scanned += 1;
                    result.secondary_scanned += 1;
                }
            }

            if suspects.len() >= limit as usize {
                self.confirm(
                    tenant,
                    primary,
                    secondary,
                    resource_type,
                    opts,
                    repairer,
                    &mut suspects,
                    &mut result,
                )
                .await?;
            }
        }

        self.confirm(
            tenant,
            primary,
            secondary,
            resource_type,
            opts,
            repairer,
            &mut suspects,
            &mut result,
        )
        .await?;

        result.differences = result.missing.count + result.extra.count + result.stale.count;
        result.elapsed = started.elapsed();
        result.completed = completed;
        Ok(result)
    }

    /// Rechecks a batch of suspects on both sides, records the ones that still
    /// diverge and applies the repair mode. Drains `suspects`.
    #[allow(clippy::too_many_arguments)]
    async fn confirm(
        &self,
        tenant: &TenantContext,
        primary: &dyn ReconcileScan,
        secondary: &dyn ReconcileScan,
        resource_type: &str,
        opts: &ReconcileOptions,
        repairer: Option<&dyn ReconcileRepair>,
        suspects: &mut Vec<(Drift, String)>,
        result: &mut ReconciliationResult,
    ) -> StorageResult<()> {
        for (_, id) in suspects.drain(..) {
            let pv = primary.current_version(tenant, resource_type, &id).await?;
            let sv = secondary
                .current_version(tenant, resource_type, &id)
                .await?;
            let drift = match (pv, sv) {
                (Some(_), None) => Drift::Missing,
                (None, Some(_)) => Drift::Extra,
                (Some(a), Some(b)) if a != b => Drift::Stale,
                _ => continue,
            };
            let report = match drift {
                Drift::Missing => &mut result.missing,
                Drift::Extra => &mut result.extra,
                Drift::Stale => &mut result.stale,
            };
            report.push(&id, opts.sample_limit);

            if opts.repair == RepairMode::None {
                continue;
            }
            let Some(repairer) = repairer else { continue };
            match repairer
                .repair(tenant, resource_type, &id, drift, opts.repair)
                .await
            {
                Ok(RepairOutcome::Queued) => result.queued += 1,
                Ok(RepairOutcome::Repaired) => result.repaired += 1,
                Err(e) => {
                    result.repair_failed += 1;
                    warn!(resource_type, id = %id, error = %e, "reconcile repair failed");
                }
            }
        }
        Ok(())
    }
}

enum Step {
    PrimaryOnly,
    SecondaryOnly,
    Both,
}

/// Version of one live resource through `ResourceStorage::read`; deleted and
/// absent resources are `None`.
async fn version_by_read<S: crate::core::ResourceStorage + ?Sized>(
    storage: &S,
    tenant: &TenantContext,
    resource_type: &str,
    id: &str,
) -> StorageResult<Option<String>> {
    use crate::error::{ResourceError, StorageError};
    match storage.read(tenant, resource_type, id).await {
        Ok(Some(resource)) => Ok(Some(resource.version_id().to_string())),
        Ok(None)
        | Err(StorageError::Resource(
            ResourceError::Gone { .. } | ResourceError::NotFound { .. },
        )) => Ok(None),
        Err(e) => Err(e),
    }
}

/// `ReconcileScan` for a backend with an inherent `scan_id_versions` and a
/// `ReindexSource` listing of types.
macro_rules! impl_reconcile_scan {
    ($feature:literal, $backend:ty) => {
        #[cfg(feature = $feature)]
        #[async_trait]
        impl ReconcileScan for $backend {
            async fn list_resource_types(
                &self,
                tenant: &TenantContext,
            ) -> StorageResult<Vec<String>> {
                crate::search::ReindexSource::list_resource_types(self, tenant).await
            }

            async fn scan_id_versions(
                &self,
                tenant: &TenantContext,
                resource_type: &str,
                after_id: Option<&str>,
                limit: u32,
            ) -> StorageResult<Vec<(String, String)>> {
                <$backend>::scan_id_versions(self, tenant, resource_type, after_id, limit).await
            }

            async fn current_version(
                &self,
                tenant: &TenantContext,
                resource_type: &str,
                id: &str,
            ) -> StorageResult<Option<String>> {
                version_by_read(self, tenant, resource_type, id).await
            }
        }
    };
}

impl_reconcile_scan!("sqlite", crate::backends::sqlite::SqliteBackend);
impl_reconcile_scan!("postgres", crate::backends::postgres::PostgresBackend);
impl_reconcile_scan!("mongodb", crate::backends::mongodb::MongoBackend);
impl_reconcile_scan!(
    "elasticsearch",
    crate::backends::elasticsearch::ElasticsearchBackend
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenant::TenantId;
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("t"),
            crate::tenant::TenantPermissions::full_access(),
        )
    }

    /// In-memory side. `scan` serves pages from `entries`; `current_version`
    /// serves `current` (defaults to `entries`, overridable to model a write
    /// that lands after the scan).
    struct Fake {
        entries: Vec<(String, String)>,
        current: Mutex<HashMap<String, String>>,
        limits: Mutex<Vec<u32>>,
    }

    impl Fake {
        fn new(pairs: &[(&str, &str)]) -> Self {
            let mut entries: Vec<(String, String)> = pairs
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect();
            entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
            let current = entries.iter().cloned().collect();
            Self {
                entries,
                current: Mutex::new(current),
                limits: Mutex::new(Vec::new()),
            }
        }

        fn numbered(n: usize, version: &str) -> Self {
            let owned: Vec<(String, String)> = (0..n)
                .map(|i| (format!("{i:06}"), version.to_string()))
                .collect();
            let refs: Vec<(&str, &str)> = owned
                .iter()
                .map(|(a, b)| (a.as_str(), b.as_str()))
                .collect();
            Self::new(&refs)
        }
    }

    #[async_trait]
    impl ReconcileScan for Fake {
        async fn list_resource_types(&self, _: &TenantContext) -> StorageResult<Vec<String>> {
            Ok(vec!["Patient".into()])
        }

        async fn scan_id_versions(
            &self,
            _: &TenantContext,
            _: &str,
            after_id: Option<&str>,
            limit: u32,
        ) -> StorageResult<Vec<(String, String)>> {
            self.limits.lock().unwrap().push(limit);
            Ok(self
                .entries
                .iter()
                .filter(|(id, _)| after_id.is_none_or(|a| id.as_bytes() > a.as_bytes()))
                .take(limit as usize)
                .cloned()
                .collect())
        }

        async fn current_version(
            &self,
            _: &TenantContext,
            _: &str,
            id: &str,
        ) -> StorageResult<Option<String>> {
            Ok(self.current.lock().unwrap().get(id).cloned())
        }
    }

    fn opts(page: u32) -> ReconcileOptions {
        ReconcileOptions {
            page_size: page,
            inter_page_delay: Duration::ZERO,
            ..Default::default()
        }
    }

    async fn run(p: &Fake, s: &Fake, o: &ReconcileOptions) -> ReconciliationResult {
        SyncReconciler::new()
            .reconcile(&tenant(), p, s, "Patient", o, None)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn classifies_missing_extra_and_stale() {
        let p = Fake::new(&[("a", "1"), ("b", "2"), ("c", "3"), ("d", "4")]);
        let s = Fake::new(&[("a", "1"), ("c", "9"), ("d", "4"), ("z", "1")]);
        let r = run(&p, &s, &opts(2)).await;
        assert_eq!(r.missing.sample, vec!["b"]);
        assert_eq!(r.extra.sample, vec!["z"]);
        assert_eq!(r.stale.sample, vec!["c"]);
        assert_eq!(r.differences, 3);
        assert_eq!((r.primary_scanned, r.secondary_scanned), (4, 4));
        assert!(r.completed);
    }

    #[tokio::test]
    async fn identical_sides_have_no_differences() {
        let p = Fake::numbered(25, "1");
        let s = Fake::numbered(25, "1");
        let r = run(&p, &s, &opts(7)).await;
        assert_eq!(r.differences, 0);
        assert_eq!(r.primary_scanned, 25);
    }

    #[tokio::test]
    async fn one_side_empty() {
        let p = Fake::new(&[("a", "1"), ("b", "1")]);
        let empty = Fake::new(&[]);
        let r = run(&p, &empty, &opts(5)).await;
        assert_eq!(r.missing.count, 2);
        let r = run(&empty, &p, &opts(5)).await;
        assert_eq!(r.extra.count, 2);
    }

    #[tokio::test]
    async fn misaligned_pages_still_join() {
        // Secondary lacks every third id, so page boundaries never line up.
        let p = Fake::numbered(100, "1");
        let kept: Vec<(String, String)> = (0..100)
            .filter(|i| i % 3 != 0)
            .map(|i| (format!("{i:06}"), "1".to_string()))
            .collect();
        let refs: Vec<(&str, &str)> = kept.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let s = Fake::new(&refs);
        let r = run(&p, &s, &opts(7)).await;
        assert_eq!(r.missing.count, 34);
        assert_eq!(r.extra.count, 0);
        assert_eq!(r.stale.count, 0);
    }

    #[tokio::test]
    async fn orders_by_bytes_of_the_id() {
        let ids = [
            ("A", "1"),
            ("a", "1"),
            ("a-b", "1"),
            ("a.b", "1"),
            ("a_b", "1"),
            ("9", "1"),
            ("10", "1"),
        ];
        let p = Fake::new(&ids);
        let s = Fake::new(&ids);
        let r = run(&p, &s, &opts(2)).await;
        assert_eq!(r.differences, 0);
        assert_eq!(r.primary_scanned, 7);
    }

    #[tokio::test]
    async fn sample_is_bounded_but_count_is_not() {
        let p = Fake::numbered(50, "1");
        let s = Fake::new(&[]);
        let o = ReconcileOptions {
            sample_limit: 3,
            ..opts(10)
        };
        let r = run(&p, &s, &o).await;
        assert_eq!(r.missing.count, 50);
        assert_eq!(r.missing.sample, vec!["000000", "000001", "000002"]);
    }

    #[tokio::test]
    async fn requests_never_exceed_the_page_size() {
        let p = Fake::numbered(1000, "1");
        let s = Fake::numbered(1000, "1");
        let r = run(&p, &s, &opts(64)).await;
        assert_eq!(r.differences, 0);
        assert!(p.limits.lock().unwrap().iter().all(|l| *l == 64));
        assert!(s.limits.lock().unwrap().iter().all(|l| *l == 64));
        // One page in memory per side: pages fetched = ceil(1000/64)+1 each at most.
        assert!(r.pages <= 2 * (1000 / 64 + 2));
    }

    #[tokio::test]
    async fn page_size_is_clamped() {
        let p = Fake::numbered(3, "1");
        let s = Fake::numbered(3, "1");
        run(&p, &s, &opts(0)).await;
        assert!(p.limits.lock().unwrap().iter().all(|l| *l == MIN_PAGE_SIZE));
        run(&p, &s, &opts(u32::MAX)).await;
        assert!(p.limits.lock().unwrap().iter().any(|l| *l == MAX_PAGE_SIZE));
    }

    #[tokio::test]
    async fn recheck_drops_a_divergence_that_healed_after_the_scan() {
        let p = Fake::new(&[("a", "1"), ("b", "2")]);
        let s = Fake::new(&[("a", "1")]);
        // The write reached the secondary after the scan page was read.
        s.current.lock().unwrap().insert("b".into(), "2".into());
        let r = run(&p, &s, &opts(10)).await;
        assert_eq!(r.differences, 0);
    }

    #[tokio::test]
    async fn recheck_reclassifies_a_stale_suspect_deleted_since() {
        let p = Fake::new(&[("a", "2")]);
        let s = Fake::new(&[("a", "1")]);
        p.current.lock().unwrap().remove("a");
        let r = run(&p, &s, &opts(10)).await;
        assert_eq!(r.extra.sample, vec!["a"]);
        assert_eq!(r.stale.count, 0);
    }

    #[tokio::test]
    async fn cancellation_stops_between_pages() {
        let p = Fake::numbered(100, "1");
        let s = Fake::new(&[]);
        let o = opts(10);
        o.cancel.cancel();
        let r = run(&p, &s, &o).await;
        assert!(!r.completed);
        assert_eq!(r.primary_scanned, 0);
    }

    struct Recorder(Mutex<Vec<(String, Drift, RepairMode)>>);

    #[async_trait]
    impl ReconcileRepair for Recorder {
        async fn repair(
            &self,
            _: &TenantContext,
            _: &str,
            id: &str,
            drift: Drift,
            mode: RepairMode,
        ) -> StorageResult<RepairOutcome> {
            self.0.lock().unwrap().push((id.to_string(), drift, mode));
            Ok(match mode {
                RepairMode::Queue => RepairOutcome::Queued,
                _ => RepairOutcome::Repaired,
            })
        }
    }

    #[tokio::test]
    async fn repair_runs_only_when_requested() {
        let p = Fake::new(&[("a", "1")]);
        let s = Fake::new(&[]);
        let rec = Recorder(Mutex::new(Vec::new()));

        let r = SyncReconciler::new()
            .reconcile(&tenant(), &p, &s, "Patient", &opts(5), Some(&rec))
            .await
            .unwrap();
        assert_eq!((r.queued, r.repaired), (0, 0));
        assert!(rec.0.lock().unwrap().is_empty());

        let o = ReconcileOptions {
            repair: RepairMode::Queue,
            ..opts(5)
        };
        let r = SyncReconciler::new()
            .reconcile(&tenant(), &p, &s, "Patient", &o, Some(&rec))
            .await
            .unwrap();
        assert_eq!(r.queued, 1);
        let o = ReconcileOptions {
            repair: RepairMode::Inline,
            ..opts(5)
        };
        let r = SyncReconciler::new()
            .reconcile(&tenant(), &p, &s, "Patient", &o, Some(&rec))
            .await
            .unwrap();
        assert_eq!(r.repaired, 1);
    }

    #[tokio::test]
    async fn resource_types_is_the_sorted_union() {
        struct Types(Vec<&'static str>);
        #[async_trait]
        impl ReconcileScan for Types {
            async fn list_resource_types(&self, _: &TenantContext) -> StorageResult<Vec<String>> {
                Ok(self.0.iter().map(|s| s.to_string()).collect())
            }
            async fn scan_id_versions(
                &self,
                _: &TenantContext,
                _: &str,
                _: Option<&str>,
                _: u32,
            ) -> StorageResult<Vec<(String, String)>> {
                Ok(vec![])
            }
            async fn current_version(
                &self,
                _: &TenantContext,
                _: &str,
                _: &str,
            ) -> StorageResult<Option<String>> {
                Ok(None)
            }
        }
        let types = SyncReconciler::new()
            .resource_types(
                &tenant(),
                &Types(vec!["Patient", "Observation"]),
                &Types(vec!["Patient", "Zebra"]),
            )
            .await
            .unwrap();
        assert_eq!(types, vec!["Observation", "Patient", "Zebra"]);
    }
}
