//! #968: Abort could not stop a manifest that was already claimed. The worker
//! consulted the submission's status only when *claiming*, so pressing Abort
//! prevented future claims and nothing else — an in-flight manifest ran to the
//! end whatever the submission said, which on a large one (#967) is the
//! difference between "stops now" and "stops in three weeks".
//!
//! The lease keeper now re-reads the submission on its own schedule and trips
//! the ingest's cancel token, which the streaming engine checks between
//! batches. This drives the whole path against a file-backed SQLite store.

#![cfg(feature = "sqlite")]

use std::sync::Arc;
use std::time::Duration;

use helios_persistence::backends::local_fs::LocalFsOutputStore;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::{
    BulkSubmitJobStore, BulkSubmitProvider, DefaultSubmitWorker, ExportOutputStore, ManifestStatus,
    RemoteFile, RemoteManifest, ResourceStorage, SubmissionId, SubmitInputFetcher, WorkerId,
};
use helios_persistence::error::StorageResult;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

/// Lines per chunk. The ingest engine's default batch is 100 entries, so one
/// chunk is one persisted batch — the granularity cancellation is checked at.
const CHUNK: usize = 100;
/// Fifteen chunks, one every `CHUNK_PAUSE`: an ingest that cannot possibly
/// finish before the abort lands, so "it stopped early" is not a race.
const LINES: usize = 15 * CHUNK;
const CHUNK_PAUSE: Duration = Duration::from_millis(400);

fn tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("submit-tenant"),
        TenantPermissions::full_access(),
    )
}

/// Feeds the manifest's single file one batch-sized chunk at a time, so the
/// ingest is slow in wall-clock terms without being slow per line.
struct ChunkedFetcher {
    manifest: RemoteManifest,
    lines: Vec<u8>,
}

#[async_trait::async_trait]
impl SubmitInputFetcher for ChunkedFetcher {
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
            for chunk in lines.chunks(chunk_bytes()) {
                if writer.write_all(chunk).await.is_err() {
                    return;
                }
                tokio::time::sleep(CHUNK_PAUSE).await;
            }
        });
        Ok((Box::new(tokio::io::BufReader::new(reader)), None))
    }
}

/// Ids are padded to a fixed width, so every line is the same length and a
/// byte-sized chunk lands exactly on a line boundary.
fn line(i: usize) -> String {
    format!("{{\"resourceType\":\"Patient\",\"id\":\"abort-{i:06}\",\"gender\":\"female\"}}\n")
}

/// The byte length of `CHUNK` whole lines.
fn chunk_bytes() -> usize {
    line(0).len() * CHUNK
}

fn ndjson() -> Vec<u8> {
    (0..LINES).map(line).collect::<String>().into_bytes()
}

/// Pressing Abort on a running import must stop the manifest already in
/// flight, not merely bar the next claim.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn aborting_a_submission_stops_the_manifest_in_flight() {
    let tmp = tempfile::tempdir().unwrap();
    let backend = SqliteBackend::with_config(
        tmp.path().join("submit-abort.db").to_str().unwrap(),
        SqliteBackendConfig::default(),
    )
    .unwrap();
    backend.init_schema().unwrap();
    let backend = Arc::new(backend);

    let tn = tenant();
    let id = SubmissionId::new("data-provider", "sub-abort");
    backend.create_submission(&tn, &id, None).await.unwrap();
    backend
        .add_manifest(
            &tn,
            &id,
            Some("https://provider.example/abort/manifest.json"),
            None,
        )
        .await
        .unwrap();

    let fetcher: Arc<dyn SubmitInputFetcher> = Arc::new(ChunkedFetcher {
        manifest: RemoteManifest {
            output: vec![RemoteFile {
                resource_type: Some("Patient".to_string()),
                url: "https://provider.example/abort/file-0.ndjson".to_string(),
                count: None,
            }],
            ..Default::default()
        },
        lines: ndjson(),
    });
    let jobs: Arc<dyn BulkSubmitJobStore> = backend.clone();
    let out_dir = tmp.path().join("out");
    let output: Arc<dyn ExportOutputStore> = Arc::new(LocalFsOutputStore::new(
        out_dir.clone(),
        "http://localhost:8080",
    ));

    // A short lease makes the keeper beat about once a second, which is also
    // how often it re-reads the submission — the cancellation latency the
    // fix promises is one keeper tick plus one batch.
    let worker_id = WorkerId::new("abort-worker");
    let claimed = jobs
        .claim_next_manifest(&worker_id, Duration::from_secs(3))
        .await
        .unwrap()
        .expect("a pending manifest to claim");
    let worker = DefaultSubmitWorker::new(jobs.clone(), fetcher, output, worker_id);
    let run = tokio::spawn(async move { worker.run_job(claimed).await });

    // Abort well inside the ingest: at least ten chunks are still unread.
    tokio::time::sleep(Duration::from_millis(800)).await;
    backend
        .abort_submission(&tn, &id, "submissionStatus=stopped")
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(60), run)
        .await
        .expect("an aborted manifest must stop instead of running to the end")
        .unwrap()
        .unwrap();

    // It stopped early: had the abort gone unnoticed, every line would have
    // landed.
    let total = backend.count(&tn, Some("Patient")).await.unwrap() as usize;
    assert!(
        total < LINES,
        "the abort did not interrupt the ingest: all {LINES} lines landed"
    );
    // And what it had already committed stayed committed: "stop soon" is not
    // a rollback. The first chunk is written before the fetcher's first pause,
    // so at least one batch is durable well inside the 800 ms above.
    assert!(
        total > 0,
        "the abort rolled the ingest back instead of stopping it"
    );

    // The abort owns the manifest's outcome. A worker that wound down must
    // not overwrite it with a terminal status of its own.
    let manifests = backend.list_manifests(&tn, &id).await.unwrap();
    let manifest = manifests.first().expect("the seeded manifest");
    assert_eq!(
        manifest.status,
        ManifestStatus::Failed,
        "abort left the manifest {:?}; a wound-down worker must not restate it",
        manifest.status
    );

    // No result artifacts: receipts for a manifest that never finished would
    // claim work that was not done.
    let artifacts = std::fs::read_dir(&out_dir)
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(
        artifacts, 0,
        "a cancelled manifest must not emit output/error artifacts"
    );
}
