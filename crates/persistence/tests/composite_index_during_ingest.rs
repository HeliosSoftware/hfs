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
//!   unindexed within a bounded time, reindexes only their type, and never
//!   costs the lease (no Docker).
//!
//! Run with:
//!   cargo test -p helios-persistence --features elasticsearch --test composite_index_during_ingest
//!
//! Skip the Docker test:
//!   cargo test -p helios-persistence --features elasticsearch --test composite_index_during_ingest -- --skip es_integration

#![cfg(all(feature = "elasticsearch", feature = "sqlite"))]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use helios_persistence::backends::local_fs::LocalFsOutputStore;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::composite::{IndexingSubmitJobs, IngestIndexSink, IngestIndexSinkConfig};
use helios_persistence::core::{
    BulkSubmitJobStore, BulkSubmitProvider, DefaultSubmitWorker, DeferredReindexHook,
    ExportOutputStore, ManifestStatus, RemoteFile, RemoteManifest, ResourceStorage, SubmissionId,
    SubmitInputFetcher, WorkerId,
};
use helios_persistence::error::StorageResult;
use helios_persistence::search::ReindexTarget;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::StoredResource;

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

// ============================================================================
// Shared fixtures
// ============================================================================

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
struct RecordingHook {
    calls: Mutex<Vec<Vec<String>>>,
}

impl RecordingHook {
    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl DeferredReindexHook for RecordingHook {
    async fn reindex_types(&self, _tenant: &TenantContext, resource_types: Vec<String>) {
        self.calls.lock().unwrap().push(resource_types);
    }
}

/// A file-backed SQLite primary in `dir` with one submission and manifest.
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
struct StalledTarget;

#[async_trait]
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
/// several times over.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
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
        },
    ));
    let jobs: Arc<dyn BulkSubmitJobStore> = Arc::new(IndexingSubmitJobs::new(
        primary.clone() as Arc<dyn BulkSubmitJobStore>,
        sink,
    ));
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
// Real Elasticsearch (requires Docker for testcontainers)
// ============================================================================

mod es_integration {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use helios_fhir::FhirVersion;
    use helios_persistence::backends::elasticsearch::{ElasticsearchBackend, ElasticsearchConfig};
    use helios_persistence::composite::{
        CompositeConfig, CompositeStorage, CompositeSubmitJobs, DynStorage, IndexingSubmitJobs,
        IngestIndexSink, IngestIndexSinkConfig, SyncMode,
    };
    use helios_persistence::core::{
        Backend, BackendKind, BulkSubmitJobStore, BulkSubmitProvider, DefaultSubmitWorker,
        DeferredReindexHook, ManifestStatus, ResourceStorage, SubmitInputFetcher, WorkerId,
    };
    use helios_persistence::search::{
        ReindexTarget, SearchParameterLoader, TenantSearchRegistries,
    };
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::elastic_search::ElasticSearch;
    use uuid::Uuid;

    use super::{
        InMemoryFetcher, RecordingHook, observations, output_store, patients, seeded_primary,
        tenant,
    };

    /// Same image and startup budget as `elasticsearch_tests.rs`: 7.17 keeps
    /// the ready message the module waits on and a JDK that starts on cgroup v2.
    const ES_IMAGE_TAG: &str = "7.17.29";
    const ES_STARTUP_TIMEOUT: Duration = Duration::from_secs(300);
    const ES_START_ATTEMPTS: usize = 2;

    async fn start_es_container() -> testcontainers::ContainerAsync<ElasticSearch> {
        let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
        let mut last_err = None;
        for attempt in 1..=ES_START_ATTEMPTS {
            match super::container_cleanup::with_cleanup_label(
                ElasticSearch::default()
                    .with_tag(ES_IMAGE_TAG)
                    .with_env_var("ES_JAVA_OPTS", "-Xms256m -Xmx256m")
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

    /// With the sink on, Elasticsearch's document count equals the receipt
    /// count as soon as the worker returns, and no deferred rebuild runs
    /// because nothing was rejected (#1127 acceptance criterion).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn es_holds_every_receipted_resource_when_ingest_ends() {
        const PATIENTS: usize = 250;
        const OBSERVATIONS: usize = 180;

        let container = start_es_container().await;
        let host = container.get_host().await.expect("ES host").to_string();
        let port = container
            .get_host_port_ipv4(9200)
            .await
            .expect("ES host port");

        let es = Arc::new(
            ElasticsearchBackend::with_shared_registry(
                ElasticsearchConfig {
                    nodes: vec![format!("http://{host}:{port}")],
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
        tokio::time::timeout(Duration::from_secs(300), worker.run_job(lease))
            .await
            .expect("ingest with index-during-ingest finished")
            .unwrap();

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
        assert_eq!(counts.total, expected, "{counts:?}");
        assert_eq!(
            counts.success, expected,
            "every receipt is a success: {counts:?}"
        );

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
