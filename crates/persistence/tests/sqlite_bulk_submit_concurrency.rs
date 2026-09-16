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

/// Routes `helios_persistence` events at `info` and above into
/// `tracing-test`'s in-memory buffer, once per test binary (#1127).
///
/// `#[traced_test]` is no use here: in an integration test it keeps only the
/// test crate's own events, and only those inside the test's span — the lease
/// keeper and the checkpoint log from spawned tasks in the library. So the
/// subscriber is global and tests tell their lines apart by the unique worker
/// ids they log.
fn capture_logs() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let writer = tracing_test::internal::MockWriter::new(tracing_test::internal::global_buf());
        let dispatch = tracing_test::internal::get_subscriber(writer, "helios_persistence=info");
        tracing::dispatcher::set_global_default(dispatch)
            .expect("no other global tracing subscriber in this test binary");
    });
}

/// Captured log lines that contain every one of `needles`.
fn logged_lines(needles: &[&str]) -> Vec<String> {
    let buf = tracing_test::internal::global_buf().lock().unwrap();
    String::from_utf8_lossy(&buf)
        .lines()
        .filter(|line| needles.iter().all(|needle| line.contains(needle)))
        .map(str::to_string)
        .collect()
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
        ) -> StorageResult<(Box<dyn tokio::io::AsyncBufRead + Send + Unpin>, Option<u64>)> {
            let data = self.files.get(url).cloned().unwrap_or_default();
            let len = data.len() as u64;
            Ok((
                Box::new(tokio::io::BufReader::new(std::io::Cursor::new(data))),
                Some(len),
            ))
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
        ) -> StorageResult<(Box<dyn tokio::io::AsyncBufRead + Send + Unpin>, Option<u64>)> {
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
            Ok((Box::new(tokio::io::BufReader::new(reader)), None))
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

/// #791: the UI's provider-submission record is written continuously while a
/// bulk ingest is committing resources on other connections. A DEFERRED
/// transaction read a WAL snapshot before writing and then failed the
/// read-to-write upgrade with SQLITE_BUSY_SNAPSHOT — a code the busy handler
/// is never invoked for — so under a live ingest essentially every
/// provider-submission write was lost. The store now takes the write lock up
/// front (IMMEDIATE). The synthetic race here cannot force a commit into the
/// microsecond snapshot window on demand, so treat this as a concurrency
/// smoke over the write path, not a proof; the deterministic guarantee is
/// the transaction mode itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn provider_submission_writes_survive_a_concurrent_ingest() {
    use helios_persistence::core::{BulkProviderStore, ResourceStorage};
    use serde_json::json;

    let tmp = tempfile::tempdir().unwrap();
    // A short busy timeout keeps the broken (DEFERRED) shape from stalling
    // half a minute per write before erroring; the fixed shape queues briefly
    // behind the ingest's commits and proceeds.
    let backend = SqliteBackend::with_config(
        tmp.path().join("provider-writes.db").to_str().unwrap(),
        SqliteBackendConfig {
            busy_timeout_ms: 1_000,
            ..Default::default()
        },
    )
    .unwrap();
    backend.init_schema().unwrap();
    let backend = Arc::new(backend);

    // A stand-in for the ingest: uninterrupted resource commits.
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer = {
        let backend = Arc::clone(&backend);
        let stop = Arc::clone(&stop);
        tokio::spawn(async move {
            let mut i = 0u64;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let patient = json!({
                    "resourceType": "Patient",
                    "id": format!("ingest-{i}"),
                    "gender": "female"
                });
                backend
                    .create(
                        &tenant(),
                        "Patient",
                        patient,
                        helios_fhir::FhirVersion::default_enabled(),
                    )
                    .await
                    .unwrap();
                i += 1;
            }
        })
    };

    // The status card's save loop: forty versioned writes, none may be lost.
    for version in 0..40i64 {
        backend
            .put_provider_submission(
                &tenant(),
                "ui-submission",
                json!({"status": "in-progress", "tick": version}),
                Some(version),
            )
            .await
            .unwrap_or_else(|e| panic!("versioned write {version} lost to the ingest: {e}"));
    }

    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    writer.await.unwrap();

    let stored = backend
        .get_provider_submission(&tenant(), "ui-submission")
        .await
        .unwrap()
        .expect("the record persisted");
    assert_eq!(stored.version, 40);
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
        ) -> StorageResult<(Box<dyn tokio::io::AsyncBufRead + Send + Unpin>, Option<u64>)> {
            Ok((
                Box::new(tokio::io::BufReader::new(BusyReader {
                    data: self.lines.clone(),
                    pos: 0,
                })),
                None,
            ))
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
/// genuinely taken over must notice at its next beat and stop, ingesting
/// nothing further under the stale fencing token. It stops without failing
/// the run, but never silently (#1127): the reclaim and the stale worker's
/// lost lease both reach the log at `warn`, because the manifest is walked
/// again from its first file.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lost_lease_aborts_the_run_and_is_logged_at_warn() {
    capture_logs();
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
        ) -> StorageResult<(Box<dyn tokio::io::AsyncBufRead + Send + Unpin>, Option<u64>)> {
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
            Ok((Box::new(tokio::io::BufReader::new(reader)), None))
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

    // Field values are matched without their quoting: `fmt` prints a `&str`
    // field with quotes and a `Display` one without.
    let reclaims = logged_lines(&[
        " WARN ",
        "reclaimed from a worker whose lease expired",
        "stale-worker",
    ]);
    assert!(
        !reclaims.is_empty(),
        "the rival's reclaim of an expired lease must be logged at warn"
    );
    let lost: Vec<String> = logged_lines(&[" WARN ", "stale-worker", "lease"])
        .into_iter()
        .filter(|line| !line.contains("reclaimed from a worker whose lease expired"))
        .collect();
    assert!(
        !lost.is_empty(),
        "the stale worker's lost lease must be logged at warn"
    );
}

/// #1078: a manifest reclaimed after its first worker died re-walks its file
/// from the top. The observer hears every batch the new worker commits, and
/// the entries the dead worker had already committed come back as updates —
/// so the live counts see each resource created exactly once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reclaimed_manifest_reports_reingested_entries_as_updates() {
    use std::sync::Mutex;

    use helios_persistence::backends::local_fs::LocalFsOutputStore;
    use helios_persistence::core::{
        BulkSubmitJobStore, DefaultSubmitWorker, ExportOutputStore, RemoteFile, RemoteManifest,
        ResourceStorage, SubmitInputFetcher, WorkerId, WriteEvent, WriteObserver, WriteOrigin,
    };
    use helios_persistence::error::StorageResult;

    const URL: &str = "https://provider.example/reclaim/file-0.ndjson";

    struct InMemoryFetcher {
        manifest: RemoteManifest,
        lines: Vec<u8>,
    }

    #[async_trait::async_trait]
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
            _url: &str,
            _headers: &[(String, String)],
            _requires_access_token: bool,
            _oauth: &[String],
            _key: Option<&serde_json::Value>,
        ) -> StorageResult<(Box<dyn tokio::io::AsyncBufRead + Send + Unpin>, Option<u64>)> {
            let len = self.lines.len() as u64;
            Ok((
                Box::new(tokio::io::BufReader::new(std::io::Cursor::new(
                    self.lines.clone(),
                ))),
                Some(len),
            ))
        }
    }

    #[derive(Default)]
    struct Recording(Mutex<Vec<(u64, u64, u64)>>);

    impl WriteObserver for Recording {
        fn on_write(&self, event: &WriteEvent) {
            match event {
                WriteEvent::Counts {
                    tenant,
                    resource_type,
                    created,
                    updated,
                    deleted,
                    origin,
                    ..
                } => {
                    assert_eq!(tenant.as_str(), "submit-tenant");
                    assert_eq!(resource_type, "Patient");
                    assert_eq!(*origin, WriteOrigin::BulkSubmit);
                    self.0.lock().unwrap().push((*created, *updated, *deleted));
                }
                other => panic!("unexpected write event {other:?}"),
            }
        }
    }

    let tmp = tempfile::tempdir().unwrap();
    let backend = SqliteBackend::with_config(
        tmp.path().join("submit-reclaim.db").to_str().unwrap(),
        SqliteBackendConfig::default(),
    )
    .unwrap();
    backend.init_schema().unwrap();
    let backend = Arc::new(backend);
    let (sub_id, manifest_id) = seed(&backend, "reclaim").await;
    let lines = ndjson("reclaim", 250);

    // The first worker claims the manifest and commits its first 120 entries
    // (the first 120 lines, in batches) before it dies holding the lease.
    let jobs: Arc<dyn BulkSubmitJobStore> = backend.clone();
    let dead = jobs
        .claim_next_manifest(&WorkerId::new("dead-worker"), Duration::from_secs(1))
        .await
        .unwrap()
        .expect("a pending manifest to claim");
    let head: Vec<u8> = lines
        .split_inclusive(|b| *b == b'\n')
        .take(120)
        .flatten()
        .copied()
        .collect();
    let partial = backend
        .process_ndjson_stream(
            &dead.tenant,
            &sub_id,
            &manifest_id,
            "Patient",
            Box::new(tokio::io::BufReader::new(std::io::Cursor::new(head))),
            &BulkProcessingOptions::new()
                .with_batch_size(50)
                .with_file_url(URL),
        )
        .await
        .unwrap();
    assert_eq!(partial.counts.success, 120);

    // Its lease lapses and a second worker reclaims the manifest.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let rival_id = WorkerId::new("rival-worker");
    let rival = jobs
        .claim_next_manifest(&rival_id, Duration::from_secs(60))
        .await
        .unwrap()
        .expect("the expired lease must be reclaimable");
    assert!(rival.fencing_token > dead.fencing_token);

    let fetcher: Arc<dyn SubmitInputFetcher> = Arc::new(InMemoryFetcher {
        manifest: RemoteManifest {
            output: vec![RemoteFile {
                resource_type: Some("Patient".to_string()),
                url: URL.to_string(),
                count: None,
            }],
            ..Default::default()
        },
        lines,
    });
    let output: Arc<dyn ExportOutputStore> = Arc::new(LocalFsOutputStore::new(
        tmp.path().join("out"),
        "http://localhost:8080",
    ));
    let recording = Arc::new(Recording::default());
    let observer: Arc<dyn WriteObserver> = recording.clone();
    let worker = DefaultSubmitWorker::new(jobs.clone(), fetcher, output, rival_id)
        .with_write_observer(Some(observer));
    tokio::time::timeout(Duration::from_secs(60), worker.run_job(rival))
        .await
        .expect("the reclaimed manifest must finish")
        .unwrap();

    // Worker batches are 100 entries: the first is entirely re-ingested, the
    // second straddles the dead worker's last committed line.
    assert_eq!(
        *recording.0.lock().unwrap(),
        vec![(0, 100, 0), (80, 20, 0), (50, 0, 0)],
        "re-ingested entries are updates; only the 130 new ones are creates"
    );
    let total = backend.count(&tenant(), Some("Patient")).await.unwrap();
    assert_eq!(total, 250);
}

/// #1127, the adversarial lease: a 2 s lease, two workers polling for the
/// same manifest, and a WAL that cannot be folded away. The real run lost its
/// lease after ~7 h because the file-boundary `wal_checkpoint(TRUNCATE)` held
/// the write lock for minutes over a multi-gigabyte WAL while the heartbeat
/// queued behind it; the sibling worker then re-walked every file.
///
/// A test cannot afford gigabytes, so it makes the checkpoint as bad as it
/// can get instead: tens of MiB of WAL that no checkpoint may reset, pinned by
/// a reader that stays open for the whole ingest. `TRUNCATE` then waits out
/// its whole busy timeout at every file boundary with the write lock held —
/// with the pool's 30 s `busy_timeout` that is fifteen leases. The manifest
/// spans several leases and several checkpoints, and must still be claimed
/// exactly once, finish, and log no lost lease, while the checkpoints report
/// the WAL they found and that they were kept busy.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pinned_wal_checkpoint_never_lets_a_rival_reclaim_the_manifest() {
    use helios_persistence::backends::local_fs::LocalFsOutputStore;
    use helios_persistence::core::{
        BulkSubmitJobStore, BulkSubmitProvider, DefaultSubmitWorker, ExportOutputStore,
        ManifestStatus, RemoteFile, RemoteManifest, ResourceStorage, SubmitInputFetcher, WorkerId,
    };
    use helios_persistence::error::StorageResult;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    const FILES: usize = 4;
    const LINES: usize = 25;
    /// Per line: a file takes 1.5 s to stream, the manifest ~6 s — three
    /// leases before counting the checkpoints.
    const LINE_PAUSE: Duration = Duration::from_millis(60);
    const LEASE: Duration = Duration::from_secs(2);
    const WAL_BALLAST_MIB: usize = 32;

    /// Streams every file slowly, so the run outlives its lease many times
    /// over and depends on the heartbeat for all of it.
    struct TrickleFetcher {
        manifest: RemoteManifest,
    }

    #[async_trait::async_trait]
    impl SubmitInputFetcher for TrickleFetcher {
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
            let file = url
                .rsplit('/')
                .next()
                .and_then(|name| name.strip_suffix(".ndjson"))
                .unwrap_or("file")
                .to_string();
            let lines = ndjson(&format!("wal-{file}"), LINES);
            let (reader, mut writer) = tokio::io::duplex(1024);
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                for line in lines.split_inclusive(|b| *b == b'\n') {
                    if writer.write_all(line).await.is_err() {
                        return;
                    }
                    tokio::time::sleep(LINE_PAUSE).await;
                }
            });
            Ok((Box::new(tokio::io::BufReader::new(reader)), None))
        }
    }

    capture_logs();
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("submit-wal.db");
    let backend =
        SqliteBackend::with_config(db_path.to_str().unwrap(), SqliteBackendConfig::default())
            .unwrap();
    backend.init_schema().unwrap();
    let backend = Arc::new(backend);
    let (sub_id, _) = seed(&backend, "wal").await;

    // Grow the WAL from a connection that never auto-checkpoints, then pin it:
    // a read transaction opened after the ballast keeps every frame in use, so
    // no checkpoint can reset the WAL and `TRUNCATE` must wait for the reader.
    let pin = rusqlite::Connection::open(&db_path).unwrap();
    pin.execute_batch(
        "PRAGMA wal_autocheckpoint = 0; CREATE TABLE wal_ballast (bytes BLOB NOT NULL);",
    )
    .unwrap();
    for _ in 0..WAL_BALLAST_MIB {
        pin.execute("INSERT INTO wal_ballast VALUES (randomblob(1048576))", [])
            .unwrap();
    }
    let wal_bytes = std::fs::metadata(db_path.with_extension("db-wal"))
        .map(|m| m.len())
        .unwrap_or(0);
    assert!(
        wal_bytes >= (WAL_BALLAST_MIB as u64) << 20,
        "the ballast must land in the WAL: {wal_bytes} bytes"
    );
    pin.execute_batch("BEGIN DEFERRED").unwrap();
    let ballast: i64 = pin
        .query_row("SELECT COUNT(*) FROM wal_ballast", [], |row| row.get(0))
        .unwrap();
    assert_eq!(ballast as usize, WAL_BALLAST_MIB);

    let fetcher: Arc<dyn SubmitInputFetcher> = Arc::new(TrickleFetcher {
        manifest: RemoteManifest {
            output: (0..FILES)
                .map(|f| RemoteFile {
                    resource_type: Some("Patient".to_string()),
                    url: format!("https://provider.example/wal/file-{f}.ndjson"),
                    count: None,
                })
                .collect(),
            ..Default::default()
        },
    });
    let jobs: Arc<dyn BulkSubmitJobStore> = backend.clone();
    let output: Arc<dyn ExportOutputStore> = Arc::new(LocalFsOutputStore::new(
        tmp.path().join("out"),
        "http://localhost:8080",
    ));

    // Two workers running the server's claim loop against the same manifest.
    let claims: Arc<Mutex<Vec<(String, u64)>>> = Arc::default();
    let done = Arc::new(AtomicBool::new(false));
    let loops: Vec<_> = ["wal-worker-a", "wal-worker-b"]
        .into_iter()
        .map(|name| {
            let (jobs, fetcher, output) = (jobs.clone(), fetcher.clone(), output.clone());
            let (claims, done) = (Arc::clone(&claims), Arc::clone(&done));
            tokio::spawn(async move {
                let worker_id = WorkerId::new(name);
                while !done.load(Ordering::SeqCst) {
                    let claimed = jobs
                        .claim_next_manifest(&worker_id, LEASE)
                        .await
                        .expect("claim");
                    let Some(lease) = claimed else {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        continue;
                    };
                    claims
                        .lock()
                        .unwrap()
                        .push((name.to_string(), lease.fencing_token));
                    DefaultSubmitWorker::new(
                        jobs.clone(),
                        fetcher.clone(),
                        output.clone(),
                        worker_id.clone(),
                    )
                    .run_job(lease)
                    .await
                    .expect("run_job");
                }
            })
        })
        .collect();

    let finished = tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            let manifests = backend.list_manifests(&tenant(), &sub_id).await.unwrap();
            if manifests[0].status.is_terminal() {
                return manifests[0].status;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("the manifest did not finish under a pinned WAL");
    pin.execute_batch("COMMIT").unwrap();
    drop(pin);
    done.store(true, Ordering::SeqCst);
    for handle in loops {
        tokio::time::timeout(Duration::from_secs(60), handle)
            .await
            .expect("a worker loop did not wind down")
            .unwrap();
    }

    let claims = claims.lock().unwrap().clone();
    assert_eq!(
        claims.len(),
        1,
        "the manifest was reclaimed and walked again: {claims:?}"
    );
    assert_eq!(
        claims[0].1, 1,
        "a single claim holds the first fencing token"
    );
    assert_eq!(finished, ManifestStatus::Completed);
    let total = backend.count(&tenant(), Some("Patient")).await.unwrap();
    assert_eq!(total as usize, FILES * LINES);

    // Nothing lost, reclaimed or starved: no warning names either worker.
    let lease_warnings = logged_lines(&[" WARN ", "wal-worker-"]);
    assert!(
        lease_warnings.is_empty(),
        "no lease warning expected: {lease_warnings:#?}"
    );
    // The checkpoints ran against the pinned WAL, were kept busy by the
    // reader, and said what they found.
    let checkpoints = logged_lines(&[
        "sqlite WAL checkpoint after a bulk-submit file",
        "truncate_busy=true",
        "wal_frames=",
        "duration_ms=",
    ]);
    assert!(
        checkpoints.len() >= FILES - 1,
        "every file boundary checkpoints the pinned WAL and logs it: {checkpoints:#?}"
    );
}
