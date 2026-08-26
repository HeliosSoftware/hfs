//! #646: two submit workers on stock SQLite froze mid-ingestion — every
//! in-flight file stopped at the same moment, nothing logged, the poll URL
//! answering in-progress forever, while a single worker ingested the same
//! submission completely. These tests drive the same streaming engine
//! concurrently against a file-backed store under a hard timeout, so the
//! freeze (or its return) fails loudly instead of hanging a runner.

#![cfg(feature = "sqlite")]

use std::sync::Arc;
use std::time::Duration;

use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::{BulkProcessingOptions, StreamingBulkSubmitProvider, SubmissionId};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

fn tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("submit-tenant"),
        TenantPermissions::full_access(),
    )
}

fn ndjson(prefix: &str, n: usize) -> Vec<u8> {
    let mut out = String::new();
    for i in 0..n {
        out.push_str(&format!(
            "{{\"resourceType\":\"Patient\",\"id\":\"{prefix}-{i}\",\"gender\":\"female\"}}\n"
        ));
    }
    out.into_bytes()
}

async fn seed(backend: &SqliteBackend, tag: &str) -> (SubmissionId, String) {
    use helios_persistence::core::BulkSubmitProvider;
    let tn = tenant();
    let id = SubmissionId::new("data-provider", format!("sub-{tag}"));
    backend.create_submission(&tn, &id, None).await.unwrap();
    let manifest = backend
        .add_manifest(
            &tn,
            &id,
            Some(&format!("https://provider.example/{tag}/manifest.json")),
            None,
        )
        .await
        .unwrap();
    (id, manifest.manifest_id)
}

/// Two concurrent NDJSON streams — the two-worker shape that froze. The
/// timeout turns a deadlock into a failure with a name.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_streams_ingest_to_completion() {
    let tmp = tempfile::tempdir().unwrap();
    let backend = SqliteBackend::with_config(
        tmp.path().join("submit.db").to_str().unwrap(),
        SqliteBackendConfig::default(),
    )
    .unwrap();
    backend.init_schema().unwrap();
    let backend = Arc::new(backend);

    let (id_a, manifest_a) = seed(&backend, "a").await;
    let (id_b, manifest_b) = seed(&backend, "b").await;

    const N: usize = 3000;
    let a = {
        let backend = Arc::clone(&backend);
        tokio::spawn(async move {
            let reader = Box::new(tokio::io::BufReader::new(std::io::Cursor::new(ndjson(
                "worker-a", N,
            ))));
            backend
                .process_ndjson_stream(
                    &tenant(),
                    &id_a,
                    &manifest_a,
                    "Patient",
                    reader,
                    &BulkProcessingOptions::new(),
                )
                .await
        })
    };
    let b = {
        let backend = Arc::clone(&backend);
        tokio::spawn(async move {
            let reader = Box::new(tokio::io::BufReader::new(std::io::Cursor::new(ndjson(
                "worker-b", N,
            ))));
            backend
                .process_ndjson_stream(
                    &tenant(),
                    &id_b,
                    &manifest_b,
                    "Patient",
                    reader,
                    &BulkProcessingOptions::new(),
                )
                .await
        })
    };

    let joined = tokio::time::timeout(Duration::from_secs(300), async {
        (a.await.unwrap(), b.await.unwrap())
    })
    .await
    .expect("concurrent ingestion deadlocked (#646): neither stream finished");

    let (ra, rb) = (joined.0.unwrap(), joined.1.unwrap());
    assert_eq!(ra.counts.success, N as u64, "worker a ingested every line");
    assert_eq!(rb.counts.success, N as u64, "worker b ingested every line");
}

/// The full two-worker shape that froze in production: two
/// `DefaultSubmitWorker`s claiming from the same store and running whole
/// manifests (multiple files, heartbeats, progress writes) concurrently.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_workers_run_whole_manifests_to_completion() {
    use std::collections::HashMap;

    use helios_persistence::backends::local_fs::LocalFsOutputStore;
    use helios_persistence::core::{
        BulkSubmitJobStore, DefaultSubmitWorker, ExportOutputStore, RemoteFile, RemoteManifest,
        SubmitInputFetcher, WorkerId,
    };
    use helios_persistence::error::StorageResult;

    struct MockFetcher {
        manifest: RemoteManifest,
        files: HashMap<String, Vec<u8>>,
    }

    #[async_trait::async_trait]
    impl SubmitInputFetcher for MockFetcher {
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
        ) -> StorageResult<Box<dyn tokio::io::AsyncBufRead + Send + Unpin>> {
            let data = self.files.get(url).cloned().unwrap_or_default();
            Ok(Box::new(tokio::io::BufReader::new(std::io::Cursor::new(
                data,
            ))))
        }
    }

    let tmp = tempfile::tempdir().unwrap();
    let backend = SqliteBackend::with_config(
        tmp.path().join("submit-workers.db").to_str().unwrap(),
        SqliteBackendConfig::default(),
    )
    .unwrap();
    backend.init_schema().unwrap();
    let backend = Arc::new(backend);

    // Two submissions, each a multi-file manifest — the Synthea shape scaled
    // down: several files per manifest, thousands of lines total.
    const FILES: usize = 4;
    const LINES: usize = 500;
    for tag in ["a", "b"] {
        let _ = seed(&backend, tag).await;
    }
    let fetchers: HashMap<&str, Arc<MockFetcher>> = ["a", "b"]
        .into_iter()
        .map(|tag| {
            let mut files = HashMap::new();
            let mut output = Vec::new();
            for f in 0..FILES {
                let url = format!("https://provider.example/{tag}/file-{f}.ndjson");
                files.insert(url.clone(), ndjson(&format!("{tag}-{f}"), LINES));
                output.push(RemoteFile {
                    resource_type: Some("Patient".to_string()),
                    url,
                    count: None,
                });
            }
            (
                tag,
                Arc::new(MockFetcher {
                    manifest: RemoteManifest {
                        output,
                        ..Default::default()
                    },
                    files,
                }),
            )
        })
        .collect();

    let jobs: Arc<dyn BulkSubmitJobStore> = backend.clone();
    let output: Arc<dyn ExportOutputStore> = Arc::new(LocalFsOutputStore::new(
        tmp.path().join("out"),
        "http://localhost:8080",
    ));
    let lease = Duration::from_secs(60);

    let mut handles = Vec::new();
    for (i, tag) in ["a", "b"].into_iter().enumerate() {
        let jobs = Arc::clone(&jobs);
        let output = Arc::clone(&output);
        let fetcher: Arc<dyn SubmitInputFetcher> = fetchers[tag].clone();
        handles.push(tokio::spawn(async move {
            let worker_id = WorkerId::new(format!("test-worker-{i}"));
            let worker = DefaultSubmitWorker::new(jobs.clone(), fetcher, output, worker_id.clone());
            loop {
                match jobs.claim_next_manifest(&worker_id, lease).await.unwrap() {
                    Some(claimed) => worker.run_job(claimed).await.unwrap(),
                    None => return,
                }
            }
        }));
    }

    tokio::time::timeout(Duration::from_secs(300), async {
        for h in handles {
            h.await.unwrap();
        }
    })
    .await
    .expect("two-worker ingestion deadlocked (#646)");

    // Every line of every file landed.
    use helios_persistence::core::ResourceStorage;
    let total = backend.count(&tenant(), Some("Patient")).await.unwrap();
    assert_eq!(total as usize, 2 * FILES * LINES);
}
