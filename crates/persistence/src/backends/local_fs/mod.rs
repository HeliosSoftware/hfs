//! Local-filesystem [`ExportOutputStore`] for single-instance bulk export.
//!
//! Writes NDJSON output parts under `{root}/{tenant}/{job_id}/` and serves
//! download URLs through HFS itself (`requires_access_token = true`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::core::bulk_export::ExportJobId;
use crate::core::bulk_export_output::{
    DownloadUrl, ExportOutputStore, ExportPartKey, ExportPartWriter, FinalizedPart,
};
use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::TenantContext;

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
        let boxed: std::pin::Pin<Box<dyn AsyncWrite + Send>> = Box::pin(file);
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
    use tokio::io::AsyncReadExt;

    fn test_key(job: &ExportJobId) -> ExportPartKey {
        ExportPartKey::output("t1", job.clone(), "Patient", 0, 1)
    }

    #[tokio::test]
    async fn test_write_finalize_read_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LocalFsOutputStore::new(tmp.path(), "http://localhost:8080");
        let job = ExportJobId::new();
        let key = test_key(&job);

        let mut writer = store.open_writer(&key).await.unwrap();
        writer
            .write_line(r#"{"resourceType":"Patient","id":"1"}"#)
            .await
            .unwrap();
        writer
            .write_line(r#"{"resourceType":"Patient","id":"2"}"#)
            .await
            .unwrap();
        let finalized = store.finalize_part(&key, writer).await.unwrap();
        assert_eq!(finalized.line_count, 2);
        assert!(finalized.size_bytes > 0);

        let url = store
            .download_url(&key, Duration::from_secs(60))
            .await
            .unwrap();
        assert!(url.requires_access_token);
        assert!(url.url.contains("/export-file/"));
        assert!(url.url.contains("Patient-0"));

        let mut reader = store.open_reader(&key).await.unwrap();
        let mut content = String::new();
        reader.read_to_string(&mut content).await.unwrap();
        assert_eq!(content.lines().count(), 2);

        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        store.delete_job_outputs(&tenant, &job).await.unwrap();
        // Idempotent: deleting again is fine.
        store.delete_job_outputs(&tenant, &job).await.unwrap();
        assert!(store.open_reader(&key).await.is_err());
    }
}
