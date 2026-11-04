//! Index-during-ingest, end to end through the submit worker (#1127).
//!
//! With `HFS_BULK_SUBMIT_DEFER_INDEXING=false` (#1242) the server wraps the
//! composite's job store in [`IndexingSubmitJobs`], which attaches an [`IngestIndexSink`]
//! to every ingest and drains it before the manifest's receipts are written.
//! These tests drive a real [`DefaultSubmitWorker`] over that stack and pin the
//! two acceptance criteria the unit tests cannot:
//!
//! - against a real Elasticsearch, the search index holds every receipted
//!   resource the moment the run returns, and no deferred rebuild is asked
//!   for (`es_integration`, needs Docker);
//! - a search target that stops answering degrades the manifest's entries to
//!   unindexed within a bounded time, reindexes only their type, reports them
//!   as warnings rather than failures since that reindex repairs them (#1666),
//!   and never costs the lease (no Docker);
//! - a resource the target rejects is a `warning` outside `failed_entries`
//!   and handed by reference to the reindex hook when that reindex repairs
//!   it, and an `error` counted in `failed_entries` when it does not (#1666,
//!   no Docker);
//! - the same against real PostgreSQL and Elasticsearch, where one rejected
//!   index write leaves a stored resource that the repair then re-indexes
//!   (#1666, `es_integration`, needs Docker).
//!
//! Run with:
//!   cargo test -p helios-persistence --features postgres,elasticsearch --test composite_index_during_ingest
//!
//! Skip the Docker test:
//!   cargo test -p helios-persistence --features elasticsearch --test composite_index_during_ingest -- --skip es_integration

#![cfg(feature = "elasticsearch")]

use std::collections::HashMap;
use std::sync::Arc;

#[cfg(feature = "sqlite")]
use std::sync::Mutex;
#[cfg(feature = "sqlite")]
use std::time::{Duration, Instant};

use async_trait::async_trait;
use helios_persistence::backends::local_fs::LocalFsOutputStore;
#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
#[cfg(feature = "sqlite")]
use helios_persistence::composite::{IndexingSubmitJobs, IngestIndexSink, IngestIndexSinkConfig};
#[cfg(feature = "sqlite")]
use helios_persistence::core::{
    BulkSubmitJobStore, BulkSubmitProvider, DefaultSubmitWorker, DeferredReindexContext,
    DeferredReindexHook, ManifestStatus, ResourceStorage, SubmissionId, SubmitWorkerStorage,
    WorkerId,
};
use helios_persistence::core::{ExportOutputStore, RemoteFile, RemoteManifest, SubmitInputFetcher};
use helios_persistence::error::StorageResult;
#[cfg(feature = "sqlite")]
use helios_persistence::error::{BackendError, StorageError};
#[cfg(feature = "sqlite")]
use helios_persistence::search::{ReindexTarget, ResourceRef};
#[cfg(feature = "sqlite")]
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
#[cfg(feature = "sqlite")]
use helios_persistence::types::StoredResource;

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

// ============================================================================
// Shared fixtures
// ============================================================================

#[cfg(feature = "sqlite")]
fn tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("index-during-ingest"),
        TenantPermissions::full_access(),
    )
}

fn patients(prefix: &str, n: usize) -> Vec<u8> {
    let mut out = String::new();
    for i in 0..n {
        out.push_str(&format!(
            "{{\"resourceType\":\"Patient\",\"id\":\"{prefix}-{i}\",\"gender\":\"female\"}}\n"
        ));
    }
    out.into_bytes()
}

fn observations(prefix: &str, n: usize) -> Vec<u8> {
    let mut out = String::new();
    for i in 0..n {
        out.push_str(&format!(
            "{{\"resourceType\":\"Observation\",\"id\":\"{prefix}-{i}\",\"status\":\"final\",\
             \"code\":{{\"text\":\"heart rate\"}}}}\n"
        ));
    }
    out.into_bytes()
}

/// Renders `texts` as one line per distinct text with how often it occurred:
/// `  {count} x {text}`, most frequent first and ties in text order. At most
/// `max_groups` lines are kept (a closing `… and N more distinct texts` line
/// says how many were left out), each text cut to `max_chars` characters with
/// a `…` marker, and no texts at all reads `  (none)`. The failure report of
/// the Elasticsearch tests prints receipts and log lines through this so a few
/// hundred identical failures read as one line (#2387).
fn distinct_counted(
    texts: impl IntoIterator<Item = String>,
    max_groups: usize,
    max_chars: usize,
) -> String {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for text in texts {
        *counts.entry(text).or_default() += 1;
    }
    if counts.is_empty() {
        return "  (none)".to_string();
    }
    let mut groups: Vec<(String, usize)> = counts.into_iter().collect();
    groups.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let left_out = groups.len().saturating_sub(max_groups);
    let mut lines: Vec<String> = groups
        .into_iter()
        .take(max_groups)
        .map(|(text, count)| format!("  {count} x {}", truncated(&text, max_chars)))
        .collect();
    if left_out > 0 {
        lines.push(format!("  … and {left_out} more distinct texts"));
    }
    lines.join("\n")
}

/// `text` cut to `max_chars` characters (never inside one) with a `…` marker
/// when it was longer.
fn truncated(text: &str, max_chars: usize) -> String {
    if text.chars().count() > max_chars {
        let cut: String = text.chars().take(max_chars).collect();
        format!("{cut}…")
    } else {
        text.to_string()
    }
}

/// `text` with the id of the resource it is about replaced by `<id>`, so the
/// receipts of hundreds of resources that failed alike read as one text. An id
/// reaches a text in two forms: `{type}/{id}` (a reference or a REST path) and
/// `{type}_{id}`, the id of the resource's Elasticsearch document
/// (`ElasticsearchBackend::document_id`), as in a permanent rejection's
/// "in document with id 'Patient_es-12'".
fn without_resource_id(text: &str, resource_type: &str, id: &str) -> String {
    text.replace(
        &format!("{resource_type}/{id}"),
        &format!("{resource_type}/<id>"),
    )
    .replace(
        &format!("{resource_type}_{id}"),
        &format!("{resource_type}_<id>"),
    )
}

#[test]
fn resource_ids_are_taken_out_of_both_reference_and_document_id_forms() {
    assert_eq!(
        without_resource_id(
            "error exception: rejected Patient/es-12 in document with id 'Patient_es-12'",
            "Patient",
            "es-12"
        ),
        "error exception: rejected Patient/<id> in document with id 'Patient_<id>'"
    );
    // Another resource's id, or another type's, is left alone, so distinct
    // failures stay distinct.
    assert_eq!(
        without_resource_id(
            "Patient_es-13 and Observation_es-12 and Observation/es-12",
            "Patient",
            "es-12"
        ),
        "Patient_es-13 and Observation_es-12 and Observation/es-12"
    );
}

#[test]
fn distinct_texts_are_counted_capped_and_truncated() {
    fn texts(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    // Identical texts are counted; groups run by count, then by text.
    assert_eq!(
        distinct_counted(texts(&["b", "a", "b", "c", "a", "b", "d"]), 10, 100),
        "  3 x b\n  2 x a\n  1 x c\n  1 x d"
    );

    // Past the cap, the least frequent groups are left out and counted.
    assert_eq!(
        distinct_counted(texts(&["a", "a", "b", "c", "d"]), 2, 100),
        "  2 x a\n  1 x b\n  … and 2 more distinct texts"
    );
    assert_eq!(
        distinct_counted(texts(&["a", "b"]), 2, 100),
        "  1 x a\n  1 x b",
        "a list that fits the cap has no 'more' line"
    );

    // A long text is cut on a character boundary, never inside a character.
    assert_eq!(
        distinct_counted(texts(&["ééééééééé", "short"]), 10, 4),
        "  1 x shor…\n  1 x éééé…"
    );
    assert_eq!(
        distinct_counted(texts(&["exactly"]), 10, 7),
        "  1 x exactly",
        "a text of exactly max_chars characters is left alone"
    );

    // No texts at all.
    assert_eq!(distinct_counted(Vec::new(), 10, 100), "  (none)");
}

/// Serves one manifest and its files from memory.
struct InMemoryFetcher {
    manifest: RemoteManifest,
    files: HashMap<String, Vec<u8>>,
}

impl InMemoryFetcher {
    /// `files` is `(resource type, url, body)` in manifest order.
    fn new(files: Vec<(&str, String, Vec<u8>)>) -> Self {
        let output = files
            .iter()
            .map(|(resource_type, url, _)| RemoteFile {
                resource_type: Some((*resource_type).to_string()),
                url: url.clone(),
                count: None,
            })
            .collect();
        Self {
            manifest: RemoteManifest {
                output,
                ..Default::default()
            },
            files: files
                .into_iter()
                .map(|(_, url, body)| (url, body))
                .collect(),
        }
    }
}

#[async_trait]
impl SubmitInputFetcher for InMemoryFetcher {
    async fn fetch_manifest(
        &self,
        _url: &str,
        _headers: &[(String, String)],
        _oauth: &[String],
        _key: Option<&serde_json::Value>,
    ) -> StorageResult<RemoteManifest> {
        Ok(self.manifest.clone())
    }

    async fn open_file_stream(
        &self,
        url: &str,
        _headers: &[(String, String)],
        _requires_access_token: bool,
        _oauth: &[String],
        _key: Option<&serde_json::Value>,
    ) -> StorageResult<(Box<dyn tokio::io::AsyncBufRead + Send + Unpin>, Option<u64>)> {
        let data = self.files.get(url).cloned().unwrap_or_default();
        let len = data.len() as u64;
        Ok((
            Box::new(tokio::io::BufReader::new(std::io::Cursor::new(data))),
            Some(len),
        ))
    }
}

/// Records every deferred reindex the worker asks for.
#[derive(Default)]
#[cfg(feature = "sqlite")]
struct RecordingHook {
    calls: Mutex<Vec<Vec<String>>>,
}

#[cfg(feature = "sqlite")]
impl RecordingHook {
    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
#[cfg(feature = "sqlite")]
impl DeferredReindexHook for RecordingHook {
    async fn reindex_types(&self, _tenant: &TenantContext, resource_types: Vec<String>) {
        self.calls.lock().unwrap().push(resource_types);
    }
}

/// A file-backed SQLite primary in `dir` with one submission and manifest.
#[cfg(feature = "sqlite")]
async fn seeded_primary(dir: &std::path::Path, tag: &str) -> (Arc<SqliteBackend>, SubmissionId) {
    let backend = SqliteBackend::with_config(
        dir.join(format!("{tag}.db")).to_str().unwrap(),
        SqliteBackendConfig::default(),
    )
    .unwrap();
    backend.init_schema().unwrap();
    let backend = Arc::new(backend);

    let submission = SubmissionId::new("data-provider", format!("sub-{tag}"));
    backend
        .create_submission(&tenant(), &submission, None)
        .await
        .unwrap();
    backend
        .add_manifest(
            &tenant(),
            &submission,
            Some(&format!("https://provider.example/{tag}/manifest.json")),
            None,
        )
        .await
        .unwrap();
    (backend, submission)
}

fn output_store(dir: &std::path::Path) -> Arc<dyn ExportOutputStore> {
    Arc::new(LocalFsOutputStore::new(
        dir.join("out"),
        "http://localhost:8080",
    ))
}

// ============================================================================
// A search target that stops answering (no Docker)
// ============================================================================

/// A search index that accepts the connection and never answers a write.
#[cfg(feature = "sqlite")]
struct StalledTarget;

#[async_trait]
#[cfg(feature = "sqlite")]
impl ReindexTarget for StalledTarget {
    async fn delete_search_entries(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _resource_id: &str,
    ) -> StorageResult<u64> {
        Ok(0)
    }

    async fn write_search_entries(
        &self,
        _tenant: &TenantContext,
        _resource: &StoredResource,
    ) -> StorageResult<usize> {
        std::future::pending().await
    }

    async fn clear_search_index(&self, _tenant: &TenantContext) -> StorageResult<u64> {
        Ok(0)
    }

    async fn write_search_entries_page(
        &self,
        _tenant: &TenantContext,
        _resources: &[StoredResource],
    ) -> Vec<StorageResult<usize>> {
        std::future::pending().await
    }
}

/// A secondary that stops accepting writes must not stall the ingest writer
/// or starve the lease heartbeat (#1127 §2, §5): the batches it cannot take
/// are reported unindexed within a bounded time, only their type is handed to
/// the deferred reindex, and a rival worker polling the whole time never gets
/// to reclaim the manifest — even though the run outlives its 2 s lease
/// several times over. With the reindex hook wired, those entries are
/// `warning`s, not failed entries (#1666).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[cfg(feature = "sqlite")]
async fn a_stalled_search_target_degrades_to_unindexed_without_losing_the_lease() {
    const LINES: usize = 300;
    const LEASE: Duration = Duration::from_secs(2);
    const MAX_WAIT: Duration = Duration::from_secs(2);

    let tmp = tempfile::tempdir().unwrap();
    let (primary, submission) = seeded_primary(tmp.path(), "stalled").await;

    // One writer with a one-batch queue: the first batch hangs in the writer,
    // the second fills the queue, the third has to wait out `max_wait`.
    let sink = Arc::new(IngestIndexSink::new(
        primary.clone() as Arc<dyn ResourceStorage>,
        vec![Arc::new(StalledTarget) as Arc<dyn ReindexTarget>],
        IngestIndexSinkConfig {
            queue: 1,
            concurrency: 1,
            coalesce: 1,
            max_wait: MAX_WAIT,
            ..Default::default()
        },
    ));
    let jobs: Arc<dyn BulkSubmitJobStore> = Arc::new(
        IndexingSubmitJobs::new(primary.clone() as Arc<dyn BulkSubmitJobStore>, sink)
            .with_automatic_reindex(true),
    );
    let url = "https://provider.example/stalled/patients.ndjson".to_string();
    let fetcher: Arc<dyn SubmitInputFetcher> = Arc::new(InMemoryFetcher::new(vec![(
        "Patient",
        url,
        patients("stalled", LINES),
    )]));
    let hook = Arc::new(RecordingHook::default());

    let owner = WorkerId::new("index-owner");
    let lease = jobs
        .claim_next_manifest(&owner, LEASE)
        .await
        .unwrap()
        .expect("the seeded manifest is claimable");
    let worker =
        DefaultSubmitWorker::new(Arc::clone(&jobs), fetcher, output_store(tmp.path()), owner)
            .with_batch_size(100)
            .with_deferred_indexing(true, Some(hook.clone() as Arc<dyn DeferredReindexHook>));

    // A rival worker tries to take the manifest over for as long as the run lasts.
    let finished = Arc::new(tokio::sync::Notify::new());
    let rival = {
        let jobs = Arc::clone(&jobs);
        let finished = Arc::clone(&finished);
        tokio::spawn(async move {
            let rival_id = WorkerId::new("index-rival");
            let mut attempts = 0u32;
            loop {
                tokio::select! {
                    _ = finished.notified() => return (attempts, None),
                    _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                }
                attempts += 1;
                if let Some(stolen) = jobs.claim_next_manifest(&rival_id, LEASE).await.unwrap() {
                    return (attempts, Some(stolen.fencing_token));
                }
            }
        })
    };

    let started = Instant::now();
    tokio::time::timeout(Duration::from_secs(120), worker.run_job(lease))
        .await
        .expect("a stalled search target must not hang the run")
        .unwrap();
    let elapsed = started.elapsed();
    finished.notify_one();
    let (attempts, stolen) = rival.await.unwrap();

    assert_eq!(
        stolen, None,
        "the manifest was reclaimed while its owner was still running ({attempts} rival claims, run took {elapsed:?})"
    );
    assert!(
        elapsed > LEASE,
        "the run must outlive its lease for the no-reclaim check to mean anything: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(60),
        "a stalled search target must degrade in bounded time, took {elapsed:?}"
    );

    let manifests = primary
        .list_manifests(&tenant(), &submission)
        .await
        .unwrap();
    assert_eq!(manifests.len(), 1);
    assert_eq!(
        manifests[0].status,
        ManifestStatus::Completed,
        "every line was stored; only search indexing degraded"
    );

    let counts = primary
        .get_entry_counts(&tenant(), &submission, &manifests[0].manifest_id)
        .await
        .unwrap();
    assert_eq!(counts.total, LINES as u64);
    assert_eq!(
        counts.processing_error, LINES as u64,
        "no receipt may read success for a resource search cannot find: {counts:?}"
    );
    assert_eq!(counts.success, 0, "{counts:?}");
    assert_eq!(
        manifests[0].failed_entries, 0,
        "entries the deferred reindex repairs are not failed entries (#1666)"
    );
    let receipts = primary
        .list_submit_files(&tenant(), &submission)
        .await
        .unwrap();
    let error_receipt = receipts
        .iter()
        .find(|f| f.file_type == "error")
        .expect("the unindexed entries are listed in the outcome receipt");
    assert_eq!(
        error_receipt.count_severity,
        Some(serde_json::json!({"warning": LINES})),
        "stored resources being repaired automatically are warnings, not errors (#1666)"
    );
    assert_eq!(
        primary.count(&tenant(), Some("Patient")).await.unwrap(),
        LINES as u64,
        "the primary keeps every resource"
    );

    assert_eq!(
        hook.calls(),
        vec![vec!["Patient".to_string()]],
        "only the rejected type is left to the deferred reindex"
    );
}

// ============================================================================
// A search target that rejects one resource (no Docker)
// ============================================================================

/// A search index that accepts every write except the resource `reject`.
#[cfg(feature = "sqlite")]
struct RejectingTarget {
    reject: &'static str,
}

#[async_trait]
#[cfg(feature = "sqlite")]
impl ReindexTarget for RejectingTarget {
    async fn delete_search_entries(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _resource_id: &str,
    ) -> StorageResult<u64> {
        Ok(0)
    }

    async fn write_search_entries(
        &self,
        tenant: &TenantContext,
        resource: &StoredResource,
    ) -> StorageResult<usize> {
        self.write_search_entries_page(tenant, std::slice::from_ref(resource))
            .await
            .pop()
            .expect("one result per resource")
    }

    async fn clear_search_index(&self, _tenant: &TenantContext) -> StorageResult<u64> {
        Ok(0)
    }

    async fn write_search_entries_page(
        &self,
        _tenant: &TenantContext,
        resources: &[StoredResource],
    ) -> Vec<StorageResult<usize>> {
        resources
            .iter()
            .map(|resource| {
                if resource.id() == self.reject {
                    Err(StorageError::Backend(BackendError::Internal {
                        backend_name: "rejecting-target".to_string(),
                        message: format!("rejected {}", resource.id()),
                        source: None,
                    }))
                } else {
                    Ok(1)
                }
            })
            .collect()
    }
}

/// Records the resources the worker hands to the resource-scoped reindex.
#[derive(Default)]
#[cfg(feature = "sqlite")]
struct ResourceRecordingHook {
    resources: Mutex<Vec<Vec<ResourceRef>>>,
}

#[async_trait]
#[cfg(feature = "sqlite")]
impl DeferredReindexHook for ResourceRecordingHook {
    async fn reindex_types(&self, _tenant: &TenantContext, _resource_types: Vec<String>) {}

    async fn reindex_resources_with_context(
        &self,
        _tenant: &TenantContext,
        resources: Vec<ResourceRef>,
        _context: DeferredReindexContext,
    ) {
        self.resources.lock().unwrap().push(resources);
    }
}

/// What one run over a target rejecting `Patient/rejected-1` left behind.
#[cfg(feature = "sqlite")]
struct RejectedRun {
    status: ManifestStatus,
    failed_entries: u64,
    processing_error: u64,
    count_severity: Option<serde_json::Value>,
    severity: String,
    reindexed: Vec<Vec<ResourceRef>>,
}

/// Runs a three-patient manifest through [`DefaultSubmitWorker`] over
/// [`IndexingSubmitJobs`] whose target rejects one of them, with a deferred
/// reindex hook wired and `automatic_reindex` as given.
#[cfg(feature = "sqlite")]
async fn run_with_one_rejected(automatic_reindex: bool) -> RejectedRun {
    let tmp = tempfile::tempdir().unwrap();
    let (primary, submission) = seeded_primary(tmp.path(), "rejected").await;
    let sink = Arc::new(IngestIndexSink::new(
        primary.clone() as Arc<dyn ResourceStorage>,
        vec![Arc::new(RejectingTarget {
            reject: "rejected-1",
        }) as Arc<dyn ReindexTarget>],
        IngestIndexSinkConfig::default(),
    ));
    let jobs: Arc<dyn BulkSubmitJobStore> = Arc::new(
        IndexingSubmitJobs::new(primary.clone() as Arc<dyn BulkSubmitJobStore>, sink)
            .with_automatic_reindex(automatic_reindex),
    );
    let fetcher: Arc<dyn SubmitInputFetcher> = Arc::new(InMemoryFetcher::new(vec![(
        "Patient",
        "https://provider.example/rejected/patients.ndjson".to_string(),
        patients("rejected", 3),
    )]));
    let hook = Arc::new(ResourceRecordingHook::default());
    let owner = WorkerId::new("rejected-owner");
    let lease = jobs
        .claim_next_manifest(&owner, Duration::from_secs(60))
        .await
        .unwrap()
        .expect("the seeded manifest is claimable");
    DefaultSubmitWorker::new(Arc::clone(&jobs), fetcher, output_store(tmp.path()), owner)
        .with_deferred_indexing(false, Some(hook.clone() as Arc<dyn DeferredReindexHook>))
        .run_job(lease)
        .await
        .unwrap();

    let manifest = primary
        .list_manifests(&tenant(), &submission)
        .await
        .unwrap()
        .remove(0);
    let counts = primary
        .get_entry_counts(&tenant(), &submission, &manifest.manifest_id)
        .await
        .unwrap();
    let page = primary
        .get_entry_results_page(
            &tenant(),
            &submission,
            &manifest.manifest_id,
            None,
            10,
            None,
        )
        .await
        .unwrap();
    let rejected = page
        .entries
        .into_iter()
        .map(|e| e.result)
        .find(|r| r.resource_id.as_deref() == Some("rejected-1"))
        .expect("the rejected entry is recorded");
    let severity = rejected.operation_outcome.unwrap()["issue"][0]["severity"]
        .as_str()
        .unwrap()
        .to_string();
    let count_severity = primary
        .list_submit_files(&tenant(), &submission)
        .await
        .unwrap()
        .into_iter()
        .find(|f| f.file_type == "error")
        .and_then(|f| f.count_severity);
    let reindexed = hook.resources.lock().unwrap().clone();
    RejectedRun {
        status: manifest.status,
        failed_entries: manifest.failed_entries,
        processing_error: counts.processing_error,
        count_severity,
        severity,
        reindexed,
    }
}

/// #1666, at the worker: with automatic reindex, the resource the index
/// rejected is a `warning`, stays out of `failed_entries`, is still
/// `processing-error` in the receipt, and is handed by reference to the
/// resource-scoped reindex. Without it, the same run is an `error` counted
/// in `failed_entries` — the worker's other branch.
#[tokio::test]
#[cfg(feature = "sqlite")]
async fn a_rejected_resource_is_a_warning_only_when_the_reindex_repairs_it() {
    let repaired = run_with_one_rejected(true).await;
    assert_eq!(repaired.status, ManifestStatus::Completed);
    assert_eq!(repaired.severity, "warning");
    assert_eq!(
        repaired.failed_entries, 0,
        "a resource being repaired automatically is not a failed entry"
    );
    assert_eq!(repaired.processing_error, 1);
    assert_eq!(
        repaired.count_severity,
        Some(serde_json::json!({"warning": 1}))
    );
    assert_eq!(
        repaired.reindexed,
        vec![vec![ResourceRef::new("Patient", "rejected-1")]],
        "the hook reindexes exactly the rejected resource"
    );

    let manual = run_with_one_rejected(false).await;
    assert_eq!(manual.status, ManifestStatus::Completed);
    assert_eq!(manual.severity, "error");
    assert_eq!(manual.failed_entries, 1);
    assert_eq!(manual.processing_error, 1);
    assert_eq!(manual.count_severity, Some(serde_json::json!({"error": 1})));
}

// ============================================================================
// Real Elasticsearch (requires Docker for testcontainers)
//
// These tests run one at a time, each on its own fresh node with no GeoIP or
// deprecation-log background writes, and a failed receipt check prints why the
// entries failed and what the node looked like (#2387).
// ============================================================================

mod es_integration {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use elasticsearch::Elasticsearch;
    use elasticsearch::cluster::ClusterHealthParts;
    use elasticsearch::http::Method;
    use elasticsearch::http::headers::HeaderMap;
    use elasticsearch::http::transport::Transport;
    use elasticsearch::params::WaitForStatus;
    #[cfg(feature = "sqlite")]
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::elasticsearch::{ElasticsearchBackend, ElasticsearchConfig};
    #[cfg(feature = "postgres")]
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_persistence::composite::{
        CompositeConfig, CompositeStorage, CompositeSubmitJobs, DynStorage, IndexingSubmitJobs,
        IngestIndexSink, IngestIndexSinkConfig, SyncMode,
    };
    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    use helios_persistence::core::DeferredReindexHook;
    #[cfg(feature = "postgres")]
    use helios_persistence::core::SubmissionId;
    #[cfg(feature = "postgres")]
    use helios_persistence::core::SubmitWorkerStorage;
    #[cfg(feature = "postgres")]
    use helios_persistence::core::history::InstanceHistoryProvider;
    use helios_persistence::core::{
        Backend, BackendKind, BulkEntryOutcome, BulkSubmitJobStore, BulkSubmitProvider,
        DefaultSubmitWorker, ManifestStatus, ResourceStorage, SubmitInputFetcher, WorkerId,
    };
    #[cfg(feature = "postgres")]
    use helios_persistence::error::StorageResult;
    use helios_persistence::search::ReindexTarget;
    #[cfg(feature = "sqlite")]
    use helios_persistence::search::{SearchParameterLoader, TenantSearchRegistries};
    #[cfg(feature = "postgres")]
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    #[cfg(feature = "postgres")]
    use helios_persistence::types::StoredResource;
    #[cfg(feature = "postgres")]
    use std::collections::HashSet;
    #[cfg(feature = "postgres")]
    use std::sync::Mutex;
    use testcontainers::ImageExt;
    use testcontainers::core::WaitFor;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::elastic_search::ElasticSearch;
    #[cfg(feature = "postgres")]
    use testcontainers_modules::postgres::Postgres;
    #[cfg(feature = "postgres")]
    use tokio::sync::OnceCell;
    use uuid::Uuid;

    use super::{InMemoryFetcher, observations, output_store, patients};
    #[cfg(feature = "sqlite")]
    use super::{RecordingHook, seeded_primary, tenant};

    /// Same image and startup budget as `elasticsearch_tests.rs`: 7.17 keeps
    /// the ready message the module waits on and a JDK that starts on cgroup v2.
    const ES_IMAGE_TAG: &str = "7.17.29";
    const ES_STARTUP_TIMEOUT: Duration = Duration::from_secs(300);
    const ES_START_ATTEMPTS: usize = 2;

    /// Starts one Elasticsearch node that does nothing in the background while
    /// a test ingests (#2387). Three of these tests failed together in CI with
    /// many bulk-submit entries ended as `processing_error`, and the cause was
    /// never confirmed. What the stock module did was make that likelier:
    ///
    /// - its ready line (`[YELLOW] to [GREEN]`) is the first index shard
    ///   starting, which is normally `.geoip_databases`, created by the GeoIP
    ///   downloader's first write. So the ingest began exactly as the
    ///   downloader started writing the GeoLite2 databases into a 256 MB heap
    ///   (and needed the host to reach the internet). The deprecation data
    ///   stream was also created in the middle of the test's own index
    ///   creation, set off by `initialize()`'s legacy template `PUT`.
    ///   Both are switched off, and readiness is the node's own `started` log
    ///   line (the module's ready line would never come with the downloader
    ///   off) followed by [`wait_for_yellow`].
    /// - the disk threshold decider is off, as in
    ///   `crates/hfs/tests/elasticsearch_startup.rs`: the disk of a shared CI
    ///   Docker host must not block or refuse shards.
    ///
    /// The heap is 512 MB because the tests run one at a time (see
    /// [`ONE_ES_TEST_AT_A_TIME`]): one 512 MB node uses less host memory than
    /// the three 256 MB nodes that used to run at once.
    async fn start_es_container() -> testcontainers::ContainerAsync<ElasticSearch> {
        let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
        let mut last_err = None;
        for attempt in 1..=ES_START_ATTEMPTS {
            match super::container_cleanup::with_cleanup_label(
                ElasticSearch::default()
                    .with_tag(ES_IMAGE_TAG)
                    .with_env_var("ES_JAVA_OPTS", "-Xms512m -Xmx512m")
                    .with_env_var("ingest.geoip.downloader.enabled", "false")
                    .with_env_var("cluster.deprecation_indexing.enabled", "false")
                    .with_env_var("cluster.routing.allocation.disk.threshold_enabled", "false")
                    .with_ready_conditions(vec![WaitFor::message_on_stdout(
                        r#""message": "started""#,
                    )])
                    .with_label("github.run_id", &run_id)
                    .with_startup_timeout(ES_STARTUP_TIMEOUT),
            )
            .start()
            .await
            {
                Ok(container) => return container,
                Err(err) => {
                    eprintln!(
                        "Elasticsearch container start attempt {attempt}/{ES_START_ATTEMPTS} failed: {err}"
                    );
                    last_err = Some(err);
                }
            }
        }
        panic!(
            "failed to start Elasticsearch container after {ES_START_ATTEMPTS} attempts: {:?}",
            last_err.expect("at least one attempt ran")
        );
    }

    /// How long a fresh node gets to answer `_cluster/health` at yellow.
    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    const ES_YELLOW_TIMEOUT: Duration = Duration::from_secs(120);

    /// One report step (a receipt page, an Elasticsearch call, a container
    /// call) that does not answer in this long is reported as not answering.
    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    const REPORT_STEP_TIMEOUT: Duration = Duration::from_secs(15);

    /// A started Elasticsearch node: its base URL and the container that
    /// keeps it alive (the failure report reads the container's state and log).
    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    struct EsNode {
        url: String,
        container: testcontainers::ContainerAsync<ElasticSearch>,
    }

    /// Starts a node (see [`start_es_container`]) and waits until it answers
    /// `_cluster/health` at yellow, so the first index the ingest creates meets
    /// a node that is done starting.
    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    async fn start_es() -> EsNode {
        let container = start_es_container().await;
        let host = container.get_host().await.expect("ES host").to_string();
        let port = container
            .get_host_port_ipv4(9200)
            .await
            .expect("ES host port");
        let url = format!("http://{host}:{port}");
        wait_for_yellow(&url).await;
        EsNode { url, container }
    }

    /// Waits for the cluster at `url` to report yellow. Elasticsearch answers
    /// 408 while the status wait has not been met, and the connection is
    /// refused until the HTTP port is up; both are retried.
    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    async fn wait_for_yellow(url: &str) {
        let client = Elasticsearch::new(Transport::single_node(url).expect("transport"));
        let deadline = Instant::now() + ES_YELLOW_TIMEOUT;
        loop {
            let last = match client
                .cluster()
                .health(ClusterHealthParts::None)
                .wait_for_status(WaitForStatus::Yellow)
                .timeout("5s")
                .request_timeout(Duration::from_secs(15))
                .send()
                .await
            {
                Ok(response) if response.status_code().is_success() => return,
                Ok(response) => {
                    let status = response.status_code();
                    let body = response.text().await.unwrap_or_default();
                    format!("{status}: {body}")
                }
                Err(err) => err.to_string(),
            };
            assert!(
                Instant::now() < deadline,
                "Elasticsearch at {url} did not reach yellow within {}s: {last}",
                ES_YELLOW_TIMEOUT.as_secs()
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// The module's Elasticsearch tests run one at a time. They used to start
    /// three Elasticsearch JVMs and three ingests at the same moment on the
    /// shared CI Docker host, and all three failed together when that host was
    /// under load (#2387). Each test still gets its own fresh container; hold
    /// this for the whole test, declared before anything the test starts so it
    /// is released last.
    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    static ONE_ES_TEST_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Routes `helios_persistence` events at `warn` and above into
    /// `tracing-test`'s in-memory buffer, once per test binary (#2387).
    ///
    /// Since #2385 a bulk-submit receipt keeps only the client text of a
    /// transient Elasticsearch failure ("the search index is temporarily
    /// unavailable; retry the request"); the Elasticsearch error type and
    /// reason are on a WARN line. So the failure report needs the log as well
    /// as the receipts. The subscriber is global, as in
    /// `sqlite_bulk_submit_concurrency.rs`: `#[traced_test]` would keep only
    /// this crate's events inside the test's span, not those of the sink's
    /// spawned tasks.
    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    fn capture_warnings() {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            let writer =
                tracing_test::internal::MockWriter::new(tracing_test::internal::global_buf());
            let dispatch =
                tracing_test::internal::get_subscriber(writer, "helios_persistence=warn");
            tracing::dispatcher::set_global_default(dispatch)
                .expect("no other global tracing subscriber in this test binary");
        });
    }

    /// How many bytes the capture buffer holds right now. A test takes this
    /// right after [`capture_warnings`] and hands it to
    /// [`ingest_failure_report`], which then reports only what was logged
    /// after it.
    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    fn warnings_mark() -> usize {
        tracing_test::internal::global_buf()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    /// Every line captured after `from` (a [`warnings_mark`]), without its
    /// leading timestamp.
    ///
    /// The buffer is one for the whole test binary and is never cleared, so
    /// the offset is what keeps the lines of the Elasticsearch tests that ran
    /// earlier out of this test's report (they hold
    /// [`ONE_ES_TEST_AT_A_TIME`], so they are finished by the time a later
    /// one takes its mark). It cannot cut out the two quick no-Docker tests:
    /// they do not take that mutex, run alongside the first Elasticsearch
    /// test, and on purpose log the same sink WARN lines a real failure
    /// would. Nor is there a marker to filter them out by: the sink logs
    /// `tenant` on one of its three WARN sites, `submission` on another and
    /// neither on the third, the Elasticsearch backend logs an index name on
    /// some of its WARN sites and nothing on others (startup initialisation,
    /// transport failures), and
    /// `es_holds_every_receipted_resource_when_ingest_ends` uses the same
    /// fixed tenant as the no-Docker tests. So the first Elasticsearch test's
    /// report can still list their lines, and its header says so.
    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    fn captured_warnings(from: usize) -> Vec<String> {
        let buf = tracing_test::internal::global_buf()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        String::from_utf8_lossy(&buf[from.min(buf.len())..])
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                line.split_once(char::is_whitespace)
                    .map_or(line, |(_, rest)| rest)
                    .trim_start()
                    .to_string()
            })
            .collect()
    }

    /// Runs one report step under [`REPORT_STEP_TIMEOUT`]; a failure or a
    /// silence comes back as the text to print in its place.
    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    async fn bounded<T, E: std::fmt::Display>(
        step: impl std::future::Future<Output = Result<T, E>>,
    ) -> Result<T, String> {
        match tokio::time::timeout(REPORT_STEP_TIMEOUT, step).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(err.to_string()),
            Err(_) => Err(format!("no answer within {REPORT_STEP_TIMEOUT:?}")),
        }
    }

    /// `GET path?query` on the node as one report line.
    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    async fn probe_es(client: &Elasticsearch, path: &str, query: &[(&str, &str)]) -> String {
        let answer = bounded(async {
            let response = client
                .send(
                    Method::Get,
                    path,
                    HeaderMap::new(),
                    Some(query),
                    None::<()>,
                    Some(Duration::from_secs(10)),
                )
                .await
                .map_err(|err| err.to_string())?;
            let status = response.status_code();
            let body = response.text().await.map_err(|err| err.to_string())?;
            Ok::<_, String>((status, body))
        })
        .await;
        match answer {
            Ok((status, body)) => {
                format!("  {path} -> {status}: {}", super::truncated(&body, 1500))
            }
            Err(err) => format!("  {path} -> unavailable: {err}"),
        }
    }

    /// What to print when a receipt check fails (#2387): why the entries
    /// ended as `processing_error`, `validation_error` or `skipped`, which
    /// WARN lines the process logged since `warnings_from` (a
    /// [`warnings_mark`] taken when the test began), and what the
    /// Elasticsearch node and its container look like. Best effort: it never
    /// panics, and a part that cannot be read says so in place.
    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    async fn ingest_failure_report(
        receipts: &dyn BulkSubmitProvider,
        tenant: &helios_persistence::tenant::TenantContext,
        submission: &helios_persistence::core::SubmissionId,
        manifest_id: &str,
        node: &EsNode,
        ingest_took: Duration,
        warnings_from: usize,
    ) -> String {
        let mut report = format!("ingest ran for {ingest_took:?}\n");

        // The receipts of every outcome but success, each text labelled with
        // its outcome and with the ids taken out of it, so that hundreds of
        // one failure read as one line. A failure by validation errors or
        // skipped entries must not read as "(none)".
        report.push_str("non-success receipts (distinct texts, by outcome):\n");
        let mut texts = Vec::new();
        let mut unreadable = None;
        'outcomes: for outcome in [
            BulkEntryOutcome::ProcessingError,
            BulkEntryOutcome::ValidationError,
            BulkEntryOutcome::Skipped,
        ] {
            let mut next = None;
            for _ in 0..50 {
                let page = bounded(receipts.get_entry_results_page(
                    tenant,
                    submission,
                    manifest_id,
                    Some(outcome),
                    500,
                    next.as_ref(),
                ))
                .await;
                match page {
                    Ok(page) => {
                        for entry in page.entries {
                            let result = entry.result;
                            let issue = result
                                .operation_outcome
                                .as_ref()
                                .and_then(|body| body.get("issue"))
                                .and_then(|issues| issues.get(0));
                            let text = match issue {
                                Some(issue) => {
                                    let field = |name: &str| {
                                        issue.get(name).and_then(|v| v.as_str()).unwrap_or("?")
                                    };
                                    format!(
                                        "{} {}: {}",
                                        field("severity"),
                                        field("code"),
                                        field("diagnostics")
                                    )
                                }
                                None => "<no OperationOutcome>".to_string(),
                            };
                            let text = match &result.resource_id {
                                Some(id) => {
                                    super::without_resource_id(&text, &result.resource_type, id)
                                }
                                None => text,
                            };
                            texts.push(format!("[{outcome}] {text}"));
                        }
                        next = page.next;
                        if next.is_none() {
                            break;
                        }
                    }
                    Err(err) => {
                        unreadable = Some(format!("{outcome}: {err}"));
                        break 'outcomes;
                    }
                }
            }
        }
        report.push_str(&super::distinct_counted(texts, 10, 600));
        report.push('\n');
        if let Some(err) = unreadable {
            report.push_str(&format!("  receipts unavailable: {err}\n"));
        }

        report.push_str(
            "helios_persistence WARN lines logged since this test began (distinct; the \
             no-Docker tests of this binary may have added theirs):\n",
        );
        report.push_str(&super::distinct_counted(
            captured_warnings(warnings_from),
            15,
            800,
        ));
        report.push('\n');

        report.push_str(&format!("Elasticsearch node {}:\n", node.url));
        let running = match bounded(node.container.is_running()).await {
            Ok(running) => running.to_string(),
            Err(err) => format!("unavailable: {err}"),
        };
        let exit_code = match bounded(node.container.exit_code()).await {
            Ok(code) => format!("{code:?}"),
            Err(err) => format!("unavailable: {err}"),
        };
        report.push_str(&format!(
            "  container is_running: {running}, exit_code: {exit_code}\n"
        ));
        match Transport::single_node(&node.url) {
            Ok(transport) => {
                let client = Elasticsearch::new(transport);
                let probes: [(&str, &[(&str, &str)]); 4] = [
                    (
                        "/_cat/indices",
                        &[
                            ("format", "json"),
                            ("expand_wildcards", "all"),
                            ("h", "index,health,status,docs.count"),
                        ],
                    ),
                    ("/_cat/allocation", &[("format", "json"), ("bytes", "mb")]),
                    (
                        "/_nodes/stats/jvm,breaker",
                        &[(
                            "filter_path",
                            "nodes.*.jvm.mem.heap_used_percent,nodes.*.breakers.*.tripped",
                        )],
                    ),
                    ("/_cluster/health", &[]),
                ];
                for (path, query) in probes {
                    report.push_str(&probe_es(&client, path, query).await);
                    report.push('\n');
                }
            }
            Err(err) => report.push_str(&format!("  unavailable: {err}\n")),
        }

        report.push_str(
            "Elasticsearch container log (WARN/ERROR/OOM/watermark/circuit/geoip, last 30):\n",
        );
        let mut log = String::new();
        match bounded(node.container.stdout_to_vec()).await {
            Ok(bytes) => log.push_str(&String::from_utf8_lossy(&bytes)),
            Err(err) => report.push_str(&format!("  stdout unavailable: {err}\n")),
        }
        match bounded(node.container.stderr_to_vec()).await {
            Ok(bytes) => {
                log.push('\n');
                log.push_str(&String::from_utf8_lossy(&bytes));
            }
            Err(err) => report.push_str(&format!("  stderr unavailable: {err}\n")),
        }
        const KEEP: [&str; 7] = [
            r#""level": "WARN""#,
            r#""level": "ERROR""#,
            r#""level": "FATAL""#,
            "OutOfMemoryError",
            "watermark",
            "circuit",
            "geoip database",
        ];
        let interesting: Vec<&str> = log
            .lines()
            .filter(|line| KEEP.iter().any(|needle| line.contains(needle)))
            .filter(|line| !line.contains(r#""type": "deprecation"#))
            .collect();
        if interesting.is_empty() {
            report.push_str("  (none)\n");
        }
        for line in &interesting[interesting.len().saturating_sub(30)..] {
            report.push_str(&format!("  {}\n", super::truncated(line, 500)));
        }
        report
    }

    #[cfg(feature = "sqlite")]
    fn search_registry() -> Arc<TenantSearchRegistries> {
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));
        let loader = SearchParameterLoader::new(FhirVersion::default());
        let registries = Arc::new(TenantSearchRegistries::base_only());
        {
            let mut registry = registries.base().write();
            if let Ok(params) = loader.load_embedded() {
                for param in params {
                    let _ = registry.register(param);
                }
            }
            if let Ok(params) = loader.load_from_spec_file(&data_dir) {
                for param in params {
                    let _ = registry.register(param);
                }
            }
        }
        registries
    }

    /// Delegates writes to Elasticsearch and records each delivered version.
    #[cfg(feature = "postgres")]
    struct RecordingTarget {
        inner: Arc<dyn ReindexTarget>,
        deliveries: Mutex<Vec<(String, String, String)>>,
    }

    #[cfg(feature = "postgres")]
    impl RecordingTarget {
        fn new(inner: Arc<dyn ReindexTarget>) -> Self {
            Self {
                inner,
                deliveries: Mutex::new(Vec::new()),
            }
        }

        fn deliveries(&self) -> Vec<(String, String, String)> {
            self.deliveries.lock().unwrap().clone()
        }

        fn record(&self, resources: &[StoredResource]) {
            self.deliveries
                .lock()
                .unwrap()
                .extend(resources.iter().map(|resource| {
                    (
                        resource.resource_type().to_string(),
                        resource.id().to_string(),
                        resource.version_id().to_string(),
                    )
                }));
        }
    }

    #[cfg(feature = "postgres")]
    #[async_trait::async_trait]
    impl ReindexTarget for RecordingTarget {
        async fn delete_search_entries(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            resource_id: &str,
        ) -> StorageResult<u64> {
            self.inner
                .delete_search_entries(tenant, resource_type, resource_id)
                .await
        }

        async fn write_search_entries(
            &self,
            tenant: &TenantContext,
            resource: &StoredResource,
        ) -> StorageResult<usize> {
            self.record(std::slice::from_ref(resource));
            self.inner.write_search_entries(tenant, resource).await
        }

        async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
            self.inner.clear_search_index(tenant).await
        }

        async fn begin_bulk_index_rebuild(&self) -> StorageResult<()> {
            self.inner.begin_bulk_index_rebuild().await
        }

        async fn end_bulk_index_rebuild(&self) -> StorageResult<()> {
            self.inner.end_bulk_index_rebuild().await
        }

        async fn write_search_entries_page(
            &self,
            tenant: &TenantContext,
            resources: &[StoredResource],
        ) -> Vec<StorageResult<usize>> {
            self.record(resources);
            self.inner
                .write_search_entries_page(tenant, resources)
                .await
        }
    }

    /// Records deferred rebuild requests made after a PostgreSQL ingest.
    #[cfg(feature = "postgres")]
    #[derive(Default)]
    struct RecordingDeferredHook {
        calls: Mutex<Vec<Vec<String>>>,
    }

    #[cfg(feature = "postgres")]
    impl RecordingDeferredHook {
        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[cfg(feature = "postgres")]
    #[async_trait::async_trait]
    impl DeferredReindexHook for RecordingDeferredHook {
        async fn reindex_types(&self, _tenant: &TenantContext, resource_types: Vec<String>) {
            self.calls.lock().unwrap().push(resource_types);
        }
    }

    #[cfg(feature = "postgres")]
    struct SharedPg {
        host: String,
        port: u16,
        _container: testcontainers::ContainerAsync<Postgres>,
    }

    #[cfg(feature = "postgres")]
    static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

    #[cfg(feature = "postgres")]
    async fn shared_pg() -> &'static SharedPg {
        SHARED_PG
            .get_or_init(|| async {
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
                let container = super::container_cleanup::with_cleanup_label(
                    Postgres::default()
                        .with_tag("16-alpine")
                        .with_label("github.run_id", &run_id),
                )
                .start()
                .await
                .expect("start PostgreSQL container");
                let host = container
                    .get_host()
                    .await
                    .expect("PostgreSQL host")
                    .to_string();
                let port = container
                    .get_host_port_ipv4(5432)
                    .await
                    .expect("PostgreSQL host port");
                SharedPg {
                    host,
                    port,
                    _container: container,
                }
            })
            .await
    }

    #[cfg(feature = "postgres")]
    async fn isolated_postgres_primary() -> Arc<PostgresBackend> {
        let pg = shared_pg().await;
        let dbname = format!("composite_ingest_{}", Uuid::new_v4().simple());
        let admin_config = format!(
            "host={} port={} user=postgres password=postgres dbname=postgres",
            pg.host, pg.port
        );
        let (admin, connection) = tokio_postgres::connect(&admin_config, tokio_postgres::NoTls)
            .await
            .expect("connect to PostgreSQL admin database");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        admin
            .batch_execute(&format!("CREATE DATABASE {dbname}"))
            .await
            .expect("create isolated PostgreSQL database");

        let backend = Arc::new(
            PostgresBackend::new(PostgresConfig {
                host: pg.host.clone(),
                port: pg.port,
                dbname,
                user: "postgres".to_string(),
                password: Some("postgres".to_string()),
                max_connections: 5,
                data_dir: Some(
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .parent()
                        .and_then(|path| path.parent())
                        .map(|path| path.join("data"))
                        .unwrap_or_else(|| PathBuf::from("data")),
                ),
                search_offloaded: true,
                ..Default::default()
            })
            .await
            .expect("create PostgreSQL primary"),
        );
        backend
            .init_schema()
            .await
            .expect("initialize PostgreSQL schema");
        backend
    }

    #[cfg(feature = "postgres")]
    fn unique_tenant(tag: &str) -> TenantContext {
        TenantContext::new(
            TenantId::new(format!("{tag}_{}", Uuid::new_v4().simple())),
            TenantPermissions::full_access(),
        )
    }

    #[cfg(feature = "postgres")]
    async fn seed_submission(
        primary: &PostgresBackend,
        tenant: &TenantContext,
        tag: &str,
    ) -> SubmissionId {
        let submission = SubmissionId::new("data-provider", format!("sub-{tag}"));
        primary
            .create_submission(tenant, &submission, None)
            .await
            .unwrap();
        primary
            .add_manifest(
                tenant,
                &submission,
                Some(&format!("https://provider.example/{tag}/manifest.json")),
                None,
            )
            .await
            .unwrap();
        submission
    }

    #[cfg(feature = "postgres")]
    async fn run_worker(
        jobs: Arc<dyn BulkSubmitJobStore>,
        fetcher: Arc<dyn SubmitInputFetcher>,
        output_dir: &std::path::Path,
        skip_unchanged: bool,
        hook: Arc<RecordingDeferredHook>,
    ) -> Duration {
        let worker_id = WorkerId::new(format!("pg-es-worker-{}", Uuid::new_v4().simple()));
        let lease = jobs
            .claim_next_manifest(&worker_id, Duration::from_secs(60))
            .await
            .unwrap()
            .expect("the seeded manifest is claimable");
        let worker = DefaultSubmitWorker::new(
            Arc::clone(&jobs),
            fetcher,
            output_store(output_dir),
            worker_id,
        )
        .with_batch_size(100)
        .with_skip_unchanged(skip_unchanged)
        .with_deferred_indexing(true, Some(hook as Arc<dyn DeferredReindexHook>));
        let started = Instant::now();
        tokio::time::timeout(Duration::from_secs(300), worker.run_job(lease))
            .await
            .expect("PostgreSQL composite ingest finished")
            .unwrap();
        started.elapsed()
    }

    #[cfg(feature = "postgres")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn postgres_es_holds_receipted_versions_and_replay_is_not_double_charged() {
        const PATIENTS: usize = 12;
        const OBSERVATIONS: usize = 8;
        const REPLAY_PATIENTS: usize = 3;

        // Declared first so it drops last: the next Elasticsearch test starts
        // only once this one's node is gone.
        let _one_at_a_time = ONE_ES_TEST_AT_A_TIME.lock().await;
        capture_warnings();
        let warnings_from = warnings_mark();

        let primary = isolated_postgres_primary().await;
        let node = start_es().await;
        let es = Arc::new(
            ElasticsearchBackend::with_shared_registry(
                ElasticsearchConfig {
                    nodes: vec![node.url.clone()],
                    index_prefix: format!("hfs_{}", Uuid::new_v4().simple()),
                    number_of_replicas: 0,
                    ..Default::default()
                },
                primary.tenant_registries().clone(),
            )
            .expect("create ES backend"),
        );
        es.initialize().await.expect("initialize ES backend");

        let config = CompositeConfig::builder()
            .primary("postgres", BackendKind::Postgres)
            .search_backend("es", BackendKind::Elasticsearch)
            .sync_mode(SyncMode::Synchronous)
            .build()
            .expect("composite config");
        let mut backends: HashMap<String, DynStorage> = HashMap::new();
        backends.insert("postgres".to_string(), primary.clone() as DynStorage);
        backends.insert("es".to_string(), es.clone() as DynStorage);
        let composite = Arc::new(CompositeStorage::new(config, backends).expect("composite"));
        let inner: Arc<dyn BulkSubmitJobStore> = Arc::new(CompositeSubmitJobs::new(
            primary.clone() as Arc<dyn BulkSubmitJobStore>,
            composite,
        ));
        let recorder = Arc::new(RecordingTarget::new(es.clone() as Arc<dyn ReindexTarget>));
        let sink = Arc::new(IngestIndexSink::new(
            primary.clone() as Arc<dyn ResourceStorage>,
            vec![recorder.clone() as Arc<dyn ReindexTarget>],
            IngestIndexSinkConfig::default(),
        ));
        let jobs: Arc<dyn BulkSubmitJobStore> = Arc::new(IndexingSubmitJobs::new(inner, sink));
        let hook = Arc::new(RecordingDeferredHook::default());
        let tmp = tempfile::tempdir().unwrap();

        let normal_tenant = unique_tenant("pg-es-normal");
        let normal_submission = seed_submission(&primary, &normal_tenant, "pg-es-normal").await;
        let normal_fetcher: Arc<dyn SubmitInputFetcher> = Arc::new(InMemoryFetcher::new(vec![
            (
                "Patient",
                "https://provider.example/pg-es/patients.ndjson".to_string(),
                patients("pg-es", PATIENTS),
            ),
            (
                "Observation",
                "https://provider.example/pg-es/observations.ndjson".to_string(),
                observations("pg-es", OBSERVATIONS),
            ),
        ]));
        let normal_took = run_worker(
            Arc::clone(&jobs),
            normal_fetcher,
            tmp.path(),
            false,
            Arc::clone(&hook),
        )
        .await;

        let normal_manifest = primary
            .list_manifests(&normal_tenant, &normal_submission)
            .await
            .unwrap()
            .pop()
            .expect("normal manifest");
        let normal_receipts = primary
            .get_entry_counts(
                &normal_tenant,
                &normal_submission,
                &normal_manifest.manifest_id,
            )
            .await
            .unwrap();
        let expected = (PATIENTS + OBSERVATIONS) as u64;
        if normal_receipts.total != expected || normal_receipts.success != expected {
            let report = ingest_failure_report(
                primary.as_ref(),
                &normal_tenant,
                &normal_submission,
                &normal_manifest.manifest_id,
                &node,
                normal_took,
                warnings_from,
            )
            .await;
            panic!("every receipt is a success: {normal_receipts:?}\n{report}");
        }
        assert_eq!(normal_manifest.status, ManifestStatus::Completed);
        assert_eq!(normal_manifest.total_entries, expected);
        assert_eq!(normal_manifest.processed_entries, expected);
        assert_eq!(normal_receipts.total, expected);
        assert_eq!(normal_receipts.success, expected);
        let primary_count = primary.count(&normal_tenant, None).await.unwrap();
        assert_eq!(primary_count, normal_receipts.success);
        let normal_deliveries = recorder.deliveries();
        assert_eq!(normal_deliveries.len(), expected as usize);
        assert_eq!(
            normal_deliveries.iter().collect::<HashSet<_>>().len(),
            expected as usize,
            "each committed resource version must reach the target exactly once"
        );
        assert!(
            normal_deliveries
                .iter()
                .all(|(_, _, version)| version == "1")
        );
        let normal_tenant_id = normal_tenant.tenant_id().as_str();
        es.refresh_index(normal_tenant_id, "Patient").await.unwrap();
        es.refresh_index(normal_tenant_id, "Observation")
            .await
            .unwrap();
        let es_count = es.count(&normal_tenant, None).await.unwrap();
        assert_eq!(es_count, normal_receipts.success);
        assert_eq!(primary_count, es_count);
        assert!(
            hook.calls().is_empty(),
            "a fully indexed PostgreSQL ingest must not request a deferred rebuild: {:?}",
            hook.calls()
        );

        let replay_tenant = unique_tenant("pg-es-replay");
        let replay_submission = seed_submission(&primary, &replay_tenant, "pg-es-replay").await;
        let repeated_url = "https://provider.example/pg-es/repeated.ndjson".to_string();
        let replay_body = patients("pg-es-replay", REPLAY_PATIENTS);
        let replay_fetcher: Arc<dyn SubmitInputFetcher> = Arc::new(InMemoryFetcher::new(vec![
            ("Patient", repeated_url.clone(), replay_body.clone()),
            ("Patient", repeated_url, replay_body),
        ]));
        let replay_took = run_worker(
            Arc::clone(&jobs),
            replay_fetcher,
            tmp.path(),
            true,
            Arc::clone(&hook),
        )
        .await;

        let replay_manifest = primary
            .list_manifests(&replay_tenant, &replay_submission)
            .await
            .unwrap()
            .pop()
            .expect("replay manifest");
        let replay_receipts = primary
            .get_entry_counts(
                &replay_tenant,
                &replay_submission,
                &replay_manifest.manifest_id,
            )
            .await
            .unwrap();
        if replay_receipts.total != REPLAY_PATIENTS as u64
            || replay_receipts.success != REPLAY_PATIENTS as u64
        {
            let report = ingest_failure_report(
                primary.as_ref(),
                &replay_tenant,
                &replay_submission,
                &replay_manifest.manifest_id,
                &node,
                replay_took,
                warnings_from,
            )
            .await;
            panic!("every receipt is a success: {replay_receipts:?}\n{report}");
        }
        assert_eq!(replay_manifest.status, ManifestStatus::Completed);
        assert_eq!(replay_manifest.total_entries, REPLAY_PATIENTS as u64);
        assert_eq!(replay_manifest.processed_entries, REPLAY_PATIENTS as u64);
        assert_eq!(replay_receipts.total, REPLAY_PATIENTS as u64);
        assert_eq!(replay_receipts.success, REPLAY_PATIENTS as u64);
        assert_eq!(
            primary
                .count(&replay_tenant, Some("Patient"))
                .await
                .unwrap(),
            REPLAY_PATIENTS as u64
        );
        for i in 0..REPLAY_PATIENTS {
            let id = format!("pg-es-replay-{i}");
            assert_eq!(
                primary
                    .history_instance_count(&replay_tenant, "Patient", &id)
                    .await
                    .unwrap(),
                1,
                "the repeated file must not add history for {id}"
            );
        }
        let replay_tenant_id = replay_tenant.tenant_id().as_str();
        es.refresh_index(replay_tenant_id, "Patient").await.unwrap();
        assert_eq!(
            es.count(&replay_tenant, Some("Patient")).await.unwrap(),
            REPLAY_PATIENTS as u64
        );
        for i in 0..REPLAY_PATIENTS {
            let id = format!("pg-es-replay-{i}");
            assert_eq!(
                es.read(&replay_tenant, "Patient", &id)
                    .await
                    .unwrap()
                    .expect("replayed patient in Elasticsearch")
                    .version_id(),
                "1"
            );
        }
        assert!(
            hook.calls().is_empty(),
            "a clean unchanged replay must not request a deferred rebuild: {:?}",
            hook.calls()
        );
    }

    /// Delegates to Elasticsearch, except that writes for `reject` fail the
    /// way a timed-out page does: the resource is stored in the primary and
    /// never reaches the index (#1666).
    #[cfg(feature = "postgres")]
    struct RejectingEsTarget {
        inner: Arc<dyn ReindexTarget>,
        reject: &'static str,
    }

    #[cfg(feature = "postgres")]
    #[async_trait::async_trait]
    impl ReindexTarget for RejectingEsTarget {
        async fn delete_search_entries(
            &self,
            tenant: &TenantContext,
            resource_type: &str,
            resource_id: &str,
        ) -> StorageResult<u64> {
            self.inner
                .delete_search_entries(tenant, resource_type, resource_id)
                .await
        }

        async fn write_search_entries(
            &self,
            tenant: &TenantContext,
            resource: &StoredResource,
        ) -> StorageResult<usize> {
            self.write_search_entries_page(tenant, std::slice::from_ref(resource))
                .await
                .pop()
                .expect("one result per resource")
        }

        async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
            self.inner.clear_search_index(tenant).await
        }

        async fn begin_bulk_index_rebuild(&self) -> StorageResult<()> {
            self.inner.begin_bulk_index_rebuild().await
        }

        async fn end_bulk_index_rebuild(&self) -> StorageResult<()> {
            self.inner.end_bulk_index_rebuild().await
        }

        async fn write_search_entries_page(
            &self,
            tenant: &TenantContext,
            resources: &[StoredResource],
        ) -> Vec<StorageResult<usize>> {
            let accepted: Vec<StoredResource> = resources
                .iter()
                .filter(|r| r.id() != self.reject)
                .cloned()
                .collect();
            let mut written = self
                .inner
                .write_search_entries_page(tenant, &accepted)
                .await
                .into_iter();
            resources
                .iter()
                .map(|r| {
                    if r.id() == self.reject {
                        Err(helios_persistence::error::StorageError::Backend(
                            helios_persistence::error::BackendError::Internal {
                                backend_name: "rejecting-es-target".to_string(),
                                message: "writing to the search index timed out after 30s"
                                    .to_string(),
                                source: None,
                            },
                        ))
                    } else {
                        written.next().expect("one result per accepted resource")
                    }
                })
                .collect()
        }
    }

    /// Records the resources handed to the resource-scoped reindex.
    #[cfg(feature = "postgres")]
    #[derive(Default)]
    struct RepairHook {
        resources: Mutex<Vec<helios_persistence::search::ResourceRef>>,
    }

    #[cfg(feature = "postgres")]
    #[async_trait::async_trait]
    impl DeferredReindexHook for RepairHook {
        async fn reindex_types(&self, _tenant: &TenantContext, _resource_types: Vec<String>) {}

        async fn reindex_resources_with_context(
            &self,
            _tenant: &TenantContext,
            resources: Vec<helios_persistence::search::ResourceRef>,
            _context: helios_persistence::core::DeferredReindexContext,
        ) {
            self.resources.lock().unwrap().extend(resources);
        }
    }

    /// #1666 against real PostgreSQL and Elasticsearch: one resource's
    /// search-index write fails during ingest. The resource is stored, the
    /// manifest is `Completed` with that entry reported as a `warning` (not an
    /// `error`, not a failed entry), the outcome file's `countSeverity` reads
    /// `warning`, and the resource-scoped reindex receives exactly that
    /// resource; replaying what the hook was asked for makes Elasticsearch
    /// match PostgreSQL.
    #[cfg(feature = "postgres")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn postgres_es_reports_a_rejected_index_write_as_a_warning_and_repairs_it() {
        const PATIENTS: usize = 12;

        let _one_at_a_time = ONE_ES_TEST_AT_A_TIME.lock().await;
        capture_warnings();
        let warnings_from = warnings_mark();

        let primary = isolated_postgres_primary().await;
        let node = start_es().await;
        let es = Arc::new(
            ElasticsearchBackend::with_shared_registry(
                ElasticsearchConfig {
                    nodes: vec![node.url.clone()],
                    index_prefix: format!("hfs_{}", Uuid::new_v4().simple()),
                    number_of_replicas: 0,
                    ..Default::default()
                },
                primary.tenant_registries().clone(),
            )
            .expect("create ES backend"),
        );
        es.initialize().await.expect("initialize ES backend");

        let config = CompositeConfig::builder()
            .primary("postgres", BackendKind::Postgres)
            .search_backend("es", BackendKind::Elasticsearch)
            .sync_mode(SyncMode::Synchronous)
            .build()
            .expect("composite config");
        let mut backends: HashMap<String, DynStorage> = HashMap::new();
        backends.insert("postgres".to_string(), primary.clone() as DynStorage);
        backends.insert("es".to_string(), es.clone() as DynStorage);
        let composite = Arc::new(CompositeStorage::new(config, backends).expect("composite"));
        let inner: Arc<dyn BulkSubmitJobStore> = Arc::new(CompositeSubmitJobs::new(
            primary.clone() as Arc<dyn BulkSubmitJobStore>,
            composite,
        ));
        let rejecting = Arc::new(RejectingEsTarget {
            inner: es.clone() as Arc<dyn ReindexTarget>,
            reject: "pg-es-1666-3",
        });
        let sink = Arc::new(IngestIndexSink::new(
            primary.clone() as Arc<dyn ResourceStorage>,
            vec![rejecting as Arc<dyn ReindexTarget>],
            IngestIndexSinkConfig::default(),
        ));
        let jobs: Arc<dyn BulkSubmitJobStore> =
            Arc::new(IndexingSubmitJobs::new(inner, sink).with_automatic_reindex(true));

        let tenant = unique_tenant("pg-es-1666");
        let submission = seed_submission(&primary, &tenant, "pg-es-1666").await;
        let fetcher: Arc<dyn SubmitInputFetcher> = Arc::new(InMemoryFetcher::new(vec![(
            "Patient",
            "https://provider.example/pg-es-1666/patients.ndjson".to_string(),
            patients("pg-es-1666", PATIENTS),
        )]));
        let hook = Arc::new(RepairHook::default());
        let tmp = tempfile::tempdir().unwrap();
        let worker_id = WorkerId::new(format!("pg-es-1666-{}", Uuid::new_v4().simple()));
        let lease = jobs
            .claim_next_manifest(&worker_id, Duration::from_secs(60))
            .await
            .unwrap()
            .expect("the seeded manifest is claimable");
        let worker = DefaultSubmitWorker::new(
            Arc::clone(&jobs),
            fetcher,
            output_store(tmp.path()),
            worker_id,
        )
        .with_batch_size(100)
        .with_deferred_indexing(false, Some(hook.clone() as Arc<dyn DeferredReindexHook>));
        let started = Instant::now();
        tokio::time::timeout(Duration::from_secs(300), worker.run_job(lease))
            .await
            .expect("PostgreSQL composite ingest finished")
            .unwrap();
        let ingest_took = started.elapsed();

        let manifest = primary
            .list_manifests(&tenant, &submission)
            .await
            .unwrap()
            .pop()
            .expect("manifest");
        let counts = primary
            .get_entry_counts(&tenant, &submission, &manifest.manifest_id)
            .await
            .unwrap();
        if counts.processing_error != 1 || counts.success != (PATIENTS - 1) as u64 {
            let report = ingest_failure_report(
                primary.as_ref(),
                &tenant,
                &submission,
                &manifest.manifest_id,
                &node,
                ingest_took,
                warnings_from,
            )
            .await;
            panic!("one rejected write, every other receipt a success: {counts:?}\n{report}");
        }
        assert_eq!(manifest.status, ManifestStatus::Completed);
        assert_eq!(
            manifest.failed_entries, 0,
            "a stored resource being repaired is not a failed entry"
        );
        assert_eq!(counts.processing_error, 1, "{counts:?}");
        assert_eq!(counts.success, (PATIENTS - 1) as u64, "{counts:?}");
        let outcome = primary
            .list_submit_files(&tenant, &submission)
            .await
            .unwrap()
            .into_iter()
            .find(|f| f.file_type == "error")
            .expect("the unindexed entry is listed in the outcome file");
        assert_eq!(
            outcome.count_severity,
            Some(serde_json::json!({"warning": 1})),
            "no `error` severity reaches the manifest"
        );

        // Stored in PostgreSQL, missing from Elasticsearch until repaired.
        assert_eq!(
            primary.count(&tenant, Some("Patient")).await.unwrap(),
            PATIENTS as u64
        );
        let tenant_id = tenant.tenant_id().as_str().to_string();
        es.refresh_index(&tenant_id, "Patient").await.unwrap();
        assert_eq!(
            es.count(&tenant, Some("Patient")).await.unwrap(),
            (PATIENTS - 1) as u64
        );

        // The reindex hook was asked for exactly the rejected resource.
        let asked = hook.resources.lock().unwrap().clone();
        assert_eq!(
            asked,
            vec![helios_persistence::search::ResourceRef::new(
                "Patient",
                "pg-es-1666-3"
            )]
        );

        // What the real hook does with those references: re-index them.
        for r in &asked {
            let stored = primary
                .read(&tenant, &r.resource_type, &r.resource_id)
                .await
                .unwrap()
                .expect("the rejected resource is stored in PostgreSQL");
            es.write_search_entries(&tenant, &stored).await.unwrap();
        }
        es.refresh_index(&tenant_id, "Patient").await.unwrap();
        assert_eq!(
            es.count(&tenant, Some("Patient")).await.unwrap(),
            PATIENTS as u64,
            "after the repair Elasticsearch matches PostgreSQL"
        );
    }

    /// With the sink on, Elasticsearch's document count equals the receipt
    /// count as soon as the worker returns, and no deferred rebuild runs
    /// because nothing was rejected (#1127 acceptance criterion).
    #[cfg(feature = "sqlite")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn es_holds_every_receipted_resource_when_ingest_ends() {
        const PATIENTS: usize = 250;
        const OBSERVATIONS: usize = 180;

        let _one_at_a_time = ONE_ES_TEST_AT_A_TIME.lock().await;
        capture_warnings();
        let warnings_from = warnings_mark();

        let node = start_es().await;

        let es = Arc::new(
            ElasticsearchBackend::with_shared_registry(
                ElasticsearchConfig {
                    nodes: vec![node.url.clone()],
                    index_prefix: format!("hfs_{}", Uuid::new_v4().simple()),
                    number_of_replicas: 0,
                    ..Default::default()
                },
                search_registry(),
            )
            .expect("create ES backend"),
        );
        es.initialize().await.expect("initialize ES backend");

        let tmp = tempfile::tempdir().unwrap();
        let (primary, submission) = seeded_primary(tmp.path(), "es").await;

        // The server's wiring: composite job store (resource operations and
        // rollbacks still reach the secondary) wrapped by the indexing store.
        let config = CompositeConfig::builder()
            .primary("sqlite", BackendKind::Sqlite)
            .search_backend("es", BackendKind::Elasticsearch)
            .sync_mode(SyncMode::Synchronous)
            .build()
            .expect("composite config");
        let mut backends: HashMap<String, DynStorage> = HashMap::new();
        backends.insert("sqlite".to_string(), primary.clone() as DynStorage);
        backends.insert("es".to_string(), es.clone() as DynStorage);
        let composite = Arc::new(CompositeStorage::new(config, backends).expect("composite"));
        let inner: Arc<dyn BulkSubmitJobStore> = Arc::new(CompositeSubmitJobs::new(
            primary.clone() as Arc<dyn BulkSubmitJobStore>,
            composite,
        ));
        let sink = Arc::new(IngestIndexSink::new(
            primary.clone() as Arc<dyn ResourceStorage>,
            vec![es.clone() as Arc<dyn ReindexTarget>],
            IngestIndexSinkConfig::default(),
        ));
        let jobs: Arc<dyn BulkSubmitJobStore> = Arc::new(IndexingSubmitJobs::new(inner, sink));

        let fetcher: Arc<dyn SubmitInputFetcher> = Arc::new(InMemoryFetcher::new(vec![
            (
                "Patient",
                "https://provider.example/es/patients.ndjson".to_string(),
                patients("es", PATIENTS),
            ),
            (
                "Observation",
                "https://provider.example/es/observations.ndjson".to_string(),
                observations("es", OBSERVATIONS),
            ),
        ]));
        let hook = Arc::new(RecordingHook::default());

        let worker_id = WorkerId::new("es-index-worker");
        let lease = jobs
            .claim_next_manifest(&worker_id, Duration::from_secs(60))
            .await
            .unwrap()
            .expect("the seeded manifest is claimable");
        let worker = DefaultSubmitWorker::new(
            Arc::clone(&jobs),
            fetcher,
            output_store(tmp.path()),
            worker_id,
        )
        .with_batch_size(100)
        .with_deferred_indexing(true, Some(hook.clone() as Arc<dyn DeferredReindexHook>));
        let started = Instant::now();
        tokio::time::timeout(Duration::from_secs(300), worker.run_job(lease))
            .await
            .expect("ingest with index-during-ingest finished")
            .unwrap();
        let ingest_took = started.elapsed();

        let manifests = primary
            .list_manifests(&tenant(), &submission)
            .await
            .unwrap();
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].status, ManifestStatus::Completed);
        let counts = primary
            .get_entry_counts(&tenant(), &submission, &manifests[0].manifest_id)
            .await
            .unwrap();
        let expected = (PATIENTS + OBSERVATIONS) as u64;
        if counts.total != expected || counts.success != expected {
            let report = ingest_failure_report(
                primary.as_ref(),
                &tenant(),
                &submission,
                &manifests[0].manifest_id,
                &node,
                ingest_took,
                warnings_from,
            )
            .await;
            panic!("every receipt is a success: {counts:?}\n{report}");
        }

        // The writes are done when the run returns; a refresh only makes the
        // near-real-time index count what it already holds.
        let tenant_id = tenant().tenant_id().as_str().to_string();
        es.refresh_index(&tenant_id, "Patient").await.unwrap();
        es.refresh_index(&tenant_id, "Observation").await.unwrap();
        let es_patients = es.count(&tenant(), Some("Patient")).await.unwrap();
        let es_observations = es.count(&tenant(), Some("Observation")).await.unwrap();
        assert_eq!(es_patients, PATIENTS as u64);
        assert_eq!(es_observations, OBSERVATIONS as u64);
        assert_eq!(
            es_patients + es_observations,
            counts.success,
            "Elasticsearch holds exactly the receipted resources at ingest end"
        );

        assert!(
            hook.calls().is_empty(),
            "nothing was rejected, so no deferred rebuild may run: {:?}",
            hook.calls()
        );
    }
}
