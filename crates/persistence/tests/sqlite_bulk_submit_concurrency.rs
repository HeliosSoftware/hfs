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

    // Sized for signal, not throughput: the point is deadlock detection, and
    // a loaded shared runner ingests slowly - the first CI run of the fixed
    // code timed out at 300s on volume alone.
    const N: usize = 1500;
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

    let joined = tokio::time::timeout(Duration::from_secs(600), async {
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

    tokio::time::timeout(Duration::from_secs(600), async {
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

/// #448's Synthea pass surfaced this: one output file whose ingestion takes
/// longer than the lease. The worker heartbeated only *between* files, so the
/// lease lapsed mid-stream, a rival claim succeeded, and the manifest
/// restarted from its first file — an unbounded silent loop, invisible in the
/// counts because the re-ingested entries upsert idempotently.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_slower_than_the_lease_stays_leased_to_completion() {
    use helios_persistence::backends::local_fs::LocalFsOutputStore;
    use helios_persistence::core::{
        BulkSubmitJobStore, DefaultSubmitWorker, ExportOutputStore, RemoteFile, RemoteManifest,
        SubmitInputFetcher, WorkerId,
    };
    use helios_persistence::error::StorageResult;

    const LINES: usize = 40;

    struct SlowFetcher {
        manifest: RemoteManifest,
        lines: Vec<u8>,
    }

    #[async_trait::async_trait]
    impl SubmitInputFetcher for SlowFetcher {
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
            _url: &str,
            _headers: &[(String, String)],
            _requires_access_token: bool,
            _oauth: &[String],
            _key: Option<&serde_json::Value>,
        ) -> StorageResult<Box<dyn tokio::io::AsyncBufRead + Send + Unpin>> {
            // Trickle the file over ~6 seconds — three times the lease.
            let (reader, mut writer) = tokio::io::duplex(1024);
            let lines = self.lines.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                for line in lines.split_inclusive(|b| *b == b'\n') {
                    if writer.write_all(line).await.is_err() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
            });
            Ok(Box::new(tokio::io::BufReader::new(reader)))
        }
    }

    let tmp = tempfile::tempdir().unwrap();
    let backend = SqliteBackend::with_config(
        tmp.path().join("submit-slow.db").to_str().unwrap(),
        SqliteBackendConfig::default(),
    )
    .unwrap();
    backend.init_schema().unwrap();
    let backend = Arc::new(backend);
    let _ = seed(&backend, "slow").await;

    let url = "https://provider.example/slow/file-0.ndjson".to_string();
    let fetcher: Arc<dyn SubmitInputFetcher> = Arc::new(SlowFetcher {
        manifest: RemoteManifest {
            output: vec![RemoteFile {
                resource_type: Some("Patient".to_string()),
                url,
                count: None,
            }],
            ..Default::default()
        },
        lines: ndjson("slow-0", LINES),
    });
    let jobs: Arc<dyn BulkSubmitJobStore> = backend.clone();
    let output: Arc<dyn ExportOutputStore> = Arc::new(LocalFsOutputStore::new(
        tmp.path().join("out"),
        "http://localhost:8080",
    ));

    let lease = Duration::from_secs(2);
    let worker_id = WorkerId::new("slow-worker");
    let claimed = jobs
        .claim_next_manifest(&worker_id, lease)
        .await
        .unwrap()
        .expect("a pending manifest to claim");
    let worker = DefaultSubmitWorker::new(jobs.clone(), fetcher, output, worker_id);
    let run = tokio::spawn(async move { worker.run_job(claimed).await.unwrap() });

    // Well past the original expiry, mid-file: the manifest must not be
    // reclaimable, or the ingestion restarts from the first file.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let rival = WorkerId::new("rival-worker");
    assert!(
        jobs.claim_next_manifest(&rival, lease)
            .await
            .unwrap()
            .is_none(),
        "the lease lapsed mid-file and a rival reclaimed the manifest"
    );

    tokio::time::timeout(Duration::from_secs(120), run)
        .await
        .expect("slow-file ingestion did not finish")
        .unwrap();

    use helios_persistence::core::ResourceStorage;
    let total = backend.count(&tenant(), Some("Patient")).await.unwrap();
    assert_eq!(total as usize, LINES);
}

/// The starvation half of the mid-file heartbeat: a stream that is always
/// Ready (a fast local file server, a fully buffered response) lets the ingest
/// future run poll after poll without ever returning Pending, so a renewal
/// sharing that future via `select!` never gets to fire. The lease then
/// silently expires mid-file even though the worker is healthy, and the
/// manifest is reclaimed and restarted from its first file, forever. The
/// renewal must therefore live on its own task. This reader never returns
/// Pending and burns wall-clock inside `poll_read`, which starves any
/// same-future timer deterministically.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_ready_heavy_stream_still_renews_the_lease() {
    use helios_persistence::backends::local_fs::LocalFsOutputStore;
    use helios_persistence::core::{
        BulkSubmitJobStore, DefaultSubmitWorker, ExportOutputStore, RemoteFile, RemoteManifest,
        SubmitInputFetcher, WorkerId,
    };
    use helios_persistence::error::StorageResult;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    const LINES: usize = 120;

    struct BusyReader {
        data: Vec<u8>,
        pos: usize,
    }

    impl tokio::io::AsyncRead for BusyReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if self.pos < self.data.len() {
                std::thread::sleep(Duration::from_millis(40));
                let end = (self.pos + 64).min(self.data.len());
                let start = self.pos;
                buf.put_slice(&self.data[start..end]);
                self.pos = end;
            }
            Poll::Ready(Ok(()))
        }
    }

    struct BusyFetcher {
        manifest: RemoteManifest,
        lines: Vec<u8>,
    }

    #[async_trait::async_trait]
    impl SubmitInputFetcher for BusyFetcher {
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
            _url: &str,
            _headers: &[(String, String)],
            _requires_access_token: bool,
            _oauth: &[String],
            _key: Option<&serde_json::Value>,
        ) -> StorageResult<Box<dyn tokio::io::AsyncBufRead + Send + Unpin>> {
            Ok(Box::new(tokio::io::BufReader::new(BusyReader {
                data: self.lines.clone(),
                pos: 0,
            })))
        }
    }

    let tmp = tempfile::tempdir().unwrap();
    let backend = SqliteBackend::with_config(
        tmp.path().join("submit-busy.db").to_str().unwrap(),
        SqliteBackendConfig::default(),
    )
    .unwrap();
    backend.init_schema().unwrap();
    let backend = Arc::new(backend);
    let _ = seed(&backend, "busy").await;

    let fetcher: Arc<dyn SubmitInputFetcher> = Arc::new(BusyFetcher {
        manifest: RemoteManifest {
            output: vec![RemoteFile {
                resource_type: Some("Patient".to_string()),
                url: "https://provider.example/busy/file-0.ndjson".to_string(),
                count: None,
            }],
            ..Default::default()
        },
        lines: ndjson("busy-0", LINES),
    });
    let jobs: Arc<dyn BulkSubmitJobStore> = backend.clone();
    let output: Arc<dyn ExportOutputStore> = Arc::new(LocalFsOutputStore::new(
        tmp.path().join("out"),
        "http://localhost:8080",
    ));

    let lease = Duration::from_secs(2);
    let worker_id = WorkerId::new("busy-worker");
    let claimed = jobs
        .claim_next_manifest(&worker_id, lease)
        .await
        .unwrap()
        .expect("a pending manifest to claim");
    let worker = DefaultSubmitWorker::new(jobs.clone(), fetcher, output, worker_id);
    let run = tokio::spawn(async move { worker.run_job(claimed).await.unwrap() });

    // Well past the original expiry, mid-file: a rival claim succeeding here
    // means the renewal starved and the restart loop is back.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let rival = WorkerId::new("rival-worker");
    assert!(
        jobs.claim_next_manifest(&rival, lease)
            .await
            .unwrap()
            .is_none(),
        "the lease lapsed mid-file under a Ready-heavy stream"
    );

    tokio::time::timeout(Duration::from_secs(120), run)
        .await
        .expect("busy-stream ingestion did not finish")
        .unwrap();

    use helios_persistence::core::ResourceStorage;
    let total = backend.count(&tenant(), Some("Patient")).await.unwrap();
    assert_eq!(total as usize, LINES);
}

/// The abort half of the mid-file heartbeat: a worker whose lease was
/// genuinely taken over must notice at its next beat and stop quietly,
/// ingesting nothing further under the stale fencing token.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lost_lease_aborts_the_run_quietly() {
    use helios_persistence::backends::local_fs::LocalFsOutputStore;
    use helios_persistence::core::{
        BulkSubmitJobStore, DefaultSubmitWorker, ExportOutputStore, RemoteFile, RemoteManifest,
        SubmitInputFetcher, WorkerId,
    };
    use helios_persistence::error::StorageResult;

    struct SlowFetcher {
        manifest: RemoteManifest,
        lines: Vec<u8>,
    }

    #[async_trait::async_trait]
    impl SubmitInputFetcher for SlowFetcher {
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
            _url: &str,
            _headers: &[(String, String)],
            _requires_access_token: bool,
            _oauth: &[String],
            _key: Option<&serde_json::Value>,
        ) -> StorageResult<Box<dyn tokio::io::AsyncBufRead + Send + Unpin>> {
            let (reader, mut writer) = tokio::io::duplex(1024);
            let lines = self.lines.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                for line in lines.split_inclusive(|b| *b == b'\n') {
                    if writer.write_all(line).await.is_err() {
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                }
            });
            Ok(Box::new(tokio::io::BufReader::new(reader)))
        }
    }

    let tmp = tempfile::tempdir().unwrap();
    let backend = SqliteBackend::with_config(
        tmp.path().join("submit-lost.db").to_str().unwrap(),
        SqliteBackendConfig::default(),
    )
    .unwrap();
    backend.init_schema().unwrap();
    let backend = Arc::new(backend);
    let _ = seed(&backend, "lost").await;

    let fetcher: Arc<dyn SubmitInputFetcher> = Arc::new(SlowFetcher {
        manifest: RemoteManifest {
            output: vec![RemoteFile {
                resource_type: Some("Patient".to_string()),
                url: "https://provider.example/lost/file-0.ndjson".to_string(),
                count: None,
            }],
            ..Default::default()
        },
        lines: ndjson("lost-0", 40),
    });
    let jobs: Arc<dyn BulkSubmitJobStore> = backend.clone();
    let output: Arc<dyn ExportOutputStore> = Arc::new(LocalFsOutputStore::new(
        tmp.path().join("out"),
        "http://localhost:8080",
    ));

    // Claim, then sit on the lease until it expires so a rival can take it
    // over legitimately - the crash-recovery scenario the fencing token is for.
    let stale_worker = WorkerId::new("stale-worker");
    let stale = jobs
        .claim_next_manifest(&stale_worker, Duration::from_secs(1))
        .await
        .unwrap()
        .expect("a pending manifest to claim");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let rival = jobs
        .claim_next_manifest(&WorkerId::new("rival-worker"), Duration::from_secs(60))
        .await
        .unwrap()
        .expect("the expired lease must be reclaimable");
    assert!(rival.fencing_token > stale.fencing_token);

    // The stale worker wakes up and tries to run its old claim: the first
    // heartbeat answers LeaseLost and run_job returns Ok without ingesting.
    let worker = DefaultSubmitWorker::new(jobs.clone(), fetcher, output, stale_worker);
    tokio::time::timeout(Duration::from_secs(30), worker.run_job(stale))
        .await
        .expect("a lost lease must abort promptly")
        .unwrap();

    use helios_persistence::core::ResourceStorage;
    let total = backend.count(&tenant(), Some("Patient")).await.unwrap();
    assert_eq!(
        total, 0,
        "the stale worker must not ingest under a lost lease"
    );
}
