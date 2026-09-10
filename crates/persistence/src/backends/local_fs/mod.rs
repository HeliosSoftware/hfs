//! Local-filesystem [`ExportOutputStore`] for single-instance bulk export.
//!
//! Writes NDJSON output parts under `{root}/{tenant}/{job_id}/` and serves
//! download URLs through HFS itself (`requires_access_token = true`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite, BufWriter};

use crate::core::bulk_export::ExportJobId;
use crate::core::bulk_export_output::{
    DownloadUrl, ExportOutputStore, ExportPartKey, ExportPartWriter, FinalizedPart,
};
use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::TenantContext;

const WRITE_BUFFER_CAPACITY: usize = 64 * 1024;

/// An [`ExportOutputStore`] backed by the local filesystem.
#[derive(Debug, Clone)]
pub struct LocalFsOutputStore {
    /// Root directory under which all export output lives.
    root: PathBuf,
    /// Base URL used to construct HFS-served download URLs.
    base_url: String,
}

impl LocalFsOutputStore {
    /// Creates a new local-filesystem output store.
    ///
    /// `root` is the directory under which `{tenant}/{job_id}/...` is created;
    /// `base_url` is the HFS base URL used for download links.
    pub fn new(root: impl Into<PathBuf>, base_url: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            base_url: base_url.into(),
        }
    }

    /// The directory holding all parts for a single job.
    fn job_dir(&self, tenant_id: &str, job_id: &ExportJobId) -> PathBuf {
        self.root.join(tenant_id).join(job_id.as_str())
    }

    /// The final file path for a part.
    fn part_path(&self, key: &ExportPartKey) -> PathBuf {
        self.job_dir(&key.tenant_id, &key.job_id).join(format!(
            "{}-{}-{}-{}.ndjson",
            key.file_type, key.resource_type, key.part_index, key.fencing_token
        ))
    }

    /// The temp file path for an in-flight part.
    fn tmp_path(&self, key: &ExportPartKey) -> PathBuf {
        let mut p = self.part_path(key);
        p.set_extension("ndjson.tmp");
        p
    }
}

fn io_err(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "local-fs".to_string(),
        message,
        source: None,
    })
}

#[async_trait]
impl ExportOutputStore for LocalFsOutputStore {
    async fn open_writer(&self, key: &ExportPartKey) -> StorageResult<ExportPartWriter> {
        let dir = self.job_dir(&key.tenant_id, &key.job_id);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| io_err(format!("create_dir_all {}: {e}", dir.display())))?;
        let tmp = self.tmp_path(key);
        let file = tokio::fs::File::create(&tmp)
            .await
            .map_err(|e| io_err(format!("create {}: {e}", tmp.display())))?;
        let boxed: std::pin::Pin<Box<dyn AsyncWrite + Send>> =
            Box::pin(BufWriter::with_capacity(WRITE_BUFFER_CAPACITY, file));
        Ok(ExportPartWriter::new(boxed))
    }

    async fn finalize_part(
        &self,
        key: &ExportPartKey,
        mut writer: ExportPartWriter,
    ) -> StorageResult<FinalizedPart> {
        use tokio::io::AsyncWriteExt;
        writer
            .writer
            .flush()
            .await
            .map_err(|e| io_err(format!("flush: {e}")))?;
        writer
            .writer
            .shutdown()
            .await
            .map_err(|e| io_err(format!("shutdown: {e}")))?;
        let line_count = writer.line_count;
        let byte_count = writer.byte_count;
        drop(writer);

        let tmp = self.tmp_path(key);
        let final_path = self.part_path(key);
        tokio::fs::rename(&tmp, &final_path).await.map_err(|e| {
            io_err(format!(
                "rename {} -> {}: {e}",
                tmp.display(),
                final_path.display()
            ))
        })?;

        Ok(FinalizedPart {
            key: key.clone(),
            resource_type: key.resource_type.clone(),
            line_count,
            size_bytes: byte_count,
        })
    }

    async fn download_url(
        &self,
        key: &ExportPartKey,
        _ttl: Duration,
    ) -> StorageResult<DownloadUrl> {
        // HFS-served URL — the download handler resolves {job_id}/{part}.
        let base = self.base_url.trim_end_matches('/');
        Ok(DownloadUrl {
            url: format!(
                "{}/export-file/{}/{}-{}",
                base, key.job_id, key.resource_type, key.part_index
            ),
            requires_access_token: true,
        })
    }

    async fn open_reader(
        &self,
        key: &ExportPartKey,
    ) -> StorageResult<std::pin::Pin<Box<dyn AsyncRead + Send>>> {
        let path = self.part_path(key);
        let file = tokio::fs::File::open(&path)
            .await
            .map_err(|e| io_err(format!("open {}: {e}", path.display())))?;
        Ok(Box::pin(file))
    }

    async fn delete_job_outputs(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> StorageResult<()> {
        let dir = self.job_dir(tenant.tenant_id().as_str(), job_id);
        delete_dir_idempotent(&dir).await
    }
}

/// Removes a directory if it exists; a missing directory is `Ok`.
async fn delete_dir_idempotent(dir: &Path) -> StorageResult<()> {
    match tokio::fs::remove_dir_all(dir).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_err(format!("remove_dir_all {}: {e}", dir.display()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenant::{TenantId, TenantPermissions};
    use std::io;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};
    use tokio::io::AsyncReadExt;

    fn test_key(job: &ExportJobId) -> ExportPartKey {
        ExportPartKey::output("t1", job.clone(), "Patient", 0, 1)
    }

    async fn assert_round_trip(lines: &[String]) {
        let tmp = tempfile::tempdir().unwrap();
        let store = LocalFsOutputStore::new(tmp.path(), "http://localhost:8080");
        let job = ExportJobId::new();
        let key = test_key(&job);
        let expected: Vec<u8> = lines
            .iter()
            .flat_map(|line| line.bytes().chain(std::iter::once(b'\n')))
            .collect();

        let mut writer = store.open_writer(&key).await.unwrap();
        assert!(store.tmp_path(&key).exists());
        assert!(!store.part_path(&key).exists());
        assert!(store.open_reader(&key).await.is_err());
        for line in lines {
            writer.write_line(line).await.unwrap();
        }
        assert_eq!(writer.line_count, lines.len() as u64);
        assert_eq!(writer.byte_count, expected.len() as u64);
        assert!(!store.part_path(&key).exists());
        let finalized = store.finalize_part(&key, writer).await.unwrap();
        assert_eq!(finalized.key, key);
        assert_eq!(finalized.resource_type, "Patient");
        assert_eq!(finalized.line_count, lines.len() as u64);
        assert_eq!(finalized.size_bytes, expected.len() as u64);
        assert!(!store.tmp_path(&key).exists());
        assert!(store.part_path(&key).exists());

        let url = store
            .download_url(&key, Duration::from_secs(60))
            .await
            .unwrap();
        assert!(url.requires_access_token);
        assert!(url.url.contains("/export-file/"));
        assert!(url.url.contains("Patient-0"));

        let mut reader = store.open_reader(&key).await.unwrap();
        let mut content = Vec::new();
        reader.read_to_end(&mut content).await.unwrap();
        assert_eq!(content, expected);
        drop(reader);

        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        store.delete_job_outputs(&tenant, &job).await.unwrap();
        // Idempotent: deleting again is fine.
        store.delete_job_outputs(&tenant, &job).await.unwrap();
        assert!(store.open_reader(&key).await.is_err());
    }

    #[tokio::test]
    async fn test_write_finalize_read_delete() {
        assert_round_trip(&[
            r#"{"resourceType":"Patient","id":"1"}"#.to_string(),
            r#"{"resourceType":"Patient","id":"2","name":[{"family":"Muñoz"}]}"#.to_string(),
        ])
        .await;
    }

    #[tokio::test]
    async fn test_empty_output() {
        assert_round_trip(&[]).await;
    }

    #[tokio::test]
    async fn test_small_records_cross_buffer_capacity() {
        let line = r#"{"resourceType":"Patient","id":"1"}"#.to_string();
        let count = WRITE_BUFFER_CAPACITY / (line.len() + 1) + 2;
        let lines = vec![line; count];
        assert_round_trip(&lines).await;
    }

    #[tokio::test]
    async fn test_oversized_record_and_buffered_tail() {
        let large = format!(
            r#"{{"resourceType":"Patient","id":"large","name":[{{"text":"{}"}}]}}"#,
            "x".repeat(WRITE_BUFFER_CAPACITY + 1)
        );
        assert_round_trip(&[
            large,
            r#"{"resourceType":"Patient","id":"tail"}"#.to_string(),
        ])
        .await;
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Failure {
        None,
        Write,
        Flush,
        Shutdown,
    }

    #[derive(Default)]
    struct WriterState {
        events: Vec<&'static str>,
        bytes: Vec<u8>,
        paths_at_drop: Option<(bool, bool)>,
    }

    struct FinalizationWriter {
        failure: Failure,
        state: Arc<Mutex<WriterState>>,
        tmp_path: PathBuf,
        final_path: PathBuf,
    }

    impl AsyncWrite for FinalizationWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            let mut state = self.state.lock().unwrap();
            state.events.push("write");
            if self.failure == Failure::Write {
                return Poll::Ready(Err(io::Error::other("injected write failure")));
            }
            state.bytes.extend_from_slice(bytes);
            Poll::Ready(Ok(bytes.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            self.state.lock().unwrap().events.push("flush");
            Poll::Ready(if self.failure == Failure::Flush {
                Err(io::Error::other("injected flush failure"))
            } else {
                Ok(())
            })
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            self.state.lock().unwrap().events.push("shutdown");
            Poll::Ready(if self.failure == Failure::Shutdown {
                Err(io::Error::other("injected shutdown failure"))
            } else {
                Ok(())
            })
        }
    }

    impl Drop for FinalizationWriter {
        fn drop(&mut self) {
            let mut state = self.state.lock().unwrap();
            state.events.push("drop");
            state.paths_at_drop = Some((self.tmp_path.exists(), self.final_path.exists()));
        }
    }

    async fn assert_finalization_failure(
        failure: Failure,
        expected_message: &str,
        expected_events: &[&str],
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let store = LocalFsOutputStore::new(tmp.path(), "http://localhost:8080");
        let job = ExportJobId::new();
        let key = test_key(&job);
        let state = Arc::new(Mutex::new(WriterState::default()));
        let mut writer = store.open_writer(&key).await.unwrap();
        writer.writer = Box::pin(BufWriter::with_capacity(
            WRITE_BUFFER_CAPACITY,
            FinalizationWriter {
                failure,
                state: state.clone(),
                tmp_path: store.tmp_path(&key),
                final_path: store.part_path(&key),
            },
        ));
        let line = r#"{"resourceType":"Patient","id":"1"}"#;
        writer.write_line(line).await.unwrap();
        assert_eq!(writer.line_count, 1);
        assert_eq!(writer.byte_count, line.len() as u64 + 1);
        assert!(state.lock().unwrap().events.is_empty());
        assert!(state.lock().unwrap().bytes.is_empty());

        let error = store.finalize_part(&key, writer).await.unwrap_err();
        match error {
            StorageError::Backend(BackendError::Internal {
                backend_name,
                message,
                ..
            }) => {
                assert_eq!(backend_name, "local-fs");
                assert_eq!(message, expected_message);
            }
            other => panic!("unexpected finalization error: {other:?}"),
        }
        {
            let state = state.lock().unwrap();
            assert_eq!(state.events, expected_events);
            assert_eq!(state.paths_at_drop, Some((true, false)));
            if failure == Failure::Write {
                assert!(state.bytes.is_empty());
            } else {
                assert_eq!(state.bytes, format!("{line}\n").as_bytes());
            }
        }
        assert!(store.tmp_path(&key).exists());
        assert!(!store.part_path(&key).exists());
        assert!(store.open_reader(&key).await.is_err());

        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        store.delete_job_outputs(&tenant, &job).await.unwrap();
        assert!(!store.tmp_path(&key).exists());
        store.delete_job_outputs(&tenant, &job).await.unwrap();
    }

    #[tokio::test]
    async fn test_buffered_drain_failure_prevents_publication() {
        assert_finalization_failure(
            Failure::Write,
            "flush: injected write failure",
            &["write", "drop"],
        )
        .await;
    }

    #[tokio::test]
    async fn test_inner_flush_failure_prevents_publication() {
        assert_finalization_failure(
            Failure::Flush,
            "flush: injected flush failure",
            &["write", "flush", "drop"],
        )
        .await;
    }

    #[tokio::test]
    async fn test_shutdown_failure_prevents_publication() {
        assert_finalization_failure(
            Failure::Shutdown,
            "shutdown: injected shutdown failure",
            &["write", "flush", "shutdown", "drop"],
        )
        .await;
    }

    #[tokio::test]
    async fn test_writer_is_closed_before_publication() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LocalFsOutputStore::new(tmp.path(), "http://localhost:8080");
        let key = test_key(&ExportJobId::new());
        let state = Arc::new(Mutex::new(WriterState::default()));
        let mut writer = store.open_writer(&key).await.unwrap();
        writer.writer = Box::pin(FinalizationWriter {
            failure: Failure::None,
            state: state.clone(),
            tmp_path: store.tmp_path(&key),
            final_path: store.part_path(&key),
        });

        let finalized = store.finalize_part(&key, writer).await.unwrap();
        assert_eq!(finalized.line_count, 0);
        assert_eq!(finalized.size_bytes, 0);
        let state = state.lock().unwrap();
        assert_eq!(state.events, ["flush", "shutdown", "drop"]);
        assert_eq!(state.paths_at_drop, Some((true, false)));
        assert!(!store.tmp_path(&key).exists());
        assert!(store.part_path(&key).exists());
    }
}
