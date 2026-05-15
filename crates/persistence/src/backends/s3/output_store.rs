//! S3-backed [`ExportOutputStore`] for multi-instance bulk export.
//!
//! Output NDJSON parts are uploaded to S3-compatible object storage; download
//! URLs are pre-signed `GET` URLs (no token required) by default, or
//! HFS-served URLs when the operator forces token-based access.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::core::bulk_export::ExportJobId;
use crate::core::bulk_export_output::{
    DownloadUrl, ExportOutputStore, ExportPartKey, ExportPartWriter, FinalizedPart,
};
use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::TenantContext;

use super::client::S3Api;

/// Manifest access-token posture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessTokenMode {
    /// Pre-signed URLs when supported (default).
    Auto,
    /// Always HFS-served URLs requiring the kickoff Bearer token.
    AlwaysToken,
    /// Always pre-signed URLs.
    AlwaysPresigned,
}

impl AccessTokenMode {
    /// Parses the `HFS_BULK_EXPORT_REQUIRES_ACCESS_TOKEN` value.
    pub fn parse(s: &str) -> Self {
        match s {
            "true" => Self::AlwaysToken,
            "false" => Self::AlwaysPresigned,
            _ => Self::Auto,
        }
    }
}

/// An [`ExportOutputStore`] backed by S3-compatible object storage.
pub struct S3OutputStore {
    client: Arc<dyn S3Api>,
    bucket: String,
    base_url: String,
    access_token_mode: AccessTokenMode,
    file_url_ttl: Duration,
    /// Local scratch directory for in-flight (pre-finalize) part files.
    scratch_dir: PathBuf,
}

impl S3OutputStore {
    /// Creates a new S3 output store.
    pub fn new(
        client: Arc<dyn S3Api>,
        bucket: impl Into<String>,
        base_url: impl Into<String>,
        access_token_mode: AccessTokenMode,
        file_url_ttl: Duration,
    ) -> Self {
        let scratch_dir = std::env::temp_dir().join("hfs-export-scratch");
        Self {
            client,
            bucket: bucket.into(),
            base_url: base_url.into(),
            access_token_mode,
            file_url_ttl,
            scratch_dir,
        }
    }

    /// The S3 object key for a finalized part.
    fn object_key(key: &ExportPartKey) -> String {
        format!(
            "{}/exports/{}/{}-{}-{}-{}.ndjson",
            key.tenant_id,
            key.job_id,
            key.file_type,
            key.resource_type,
            key.part_index,
            key.fencing_token
        )
    }

    /// The S3 key prefix covering all parts of a job.
    fn job_prefix(tenant_id: &str, job_id: &ExportJobId) -> String {
        format!("{}/exports/{}/", tenant_id, job_id)
    }

    /// The local scratch path for an in-flight part.
    fn scratch_path(&self, key: &ExportPartKey) -> PathBuf {
        self.scratch_dir.join(format!(
            "{}-{}-{}-{}-{}-{}.tmp",
            key.tenant_id,
            key.job_id,
            key.file_type,
            key.resource_type,
            key.part_index,
            key.fencing_token
        ))
    }
}

fn s3_err(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "s3-output".to_string(),
        message,
        source: None,
    })
}

#[async_trait]
impl ExportOutputStore for S3OutputStore {
    async fn open_writer(&self, key: &ExportPartKey) -> StorageResult<ExportPartWriter> {
        tokio::fs::create_dir_all(&self.scratch_dir)
            .await
            .map_err(|e| s3_err(format!("create scratch dir: {e}")))?;
        let path = self.scratch_path(key);
        let file = tokio::fs::File::create(&path)
            .await
            .map_err(|e| s3_err(format!("create scratch file {}: {e}", path.display())))?;
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
            .map_err(|e| s3_err(format!("flush scratch file: {e}")))?;
        writer
            .writer
            .shutdown()
            .await
            .map_err(|e| s3_err(format!("close scratch file: {e}")))?;
        let line_count = writer.line_count;
        let byte_count = writer.byte_count;
        drop(writer);

        let path = self.scratch_path(key);
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|e| s3_err(format!("read scratch file {}: {e}", path.display())))?;
        let object_key = Self::object_key(key);
        self.client
            .put_object(
                &self.bucket,
                &object_key,
                bytes,
                Some("application/fhir+ndjson"),
                None,
                None,
            )
            .await
            .map_err(|e| s3_err(format!("upload {object_key}: {e:?}")))?;
        // Best-effort cleanup of the scratch file.
        let _ = tokio::fs::remove_file(&path).await;

        Ok(FinalizedPart {
            key: key.clone(),
            resource_type: key.resource_type.clone(),
            line_count,
            size_bytes: byte_count,
        })
    }

    async fn download_url(&self, key: &ExportPartKey, ttl: Duration) -> StorageResult<DownloadUrl> {
        match self.access_token_mode {
            AccessTokenMode::AlwaysToken => Ok(DownloadUrl {
                url: format!(
                    "{}/export-file/{}/{}-{}",
                    self.base_url.trim_end_matches('/'),
                    key.job_id,
                    key.resource_type,
                    key.part_index
                ),
                requires_access_token: true,
            }),
            AccessTokenMode::Auto | AccessTokenMode::AlwaysPresigned => {
                let object_key = Self::object_key(key);
                let effective_ttl = if ttl.is_zero() {
                    self.file_url_ttl
                } else {
                    ttl
                };
                let url = self
                    .client
                    .presign_get(&self.bucket, &object_key, effective_ttl)
                    .await
                    .map_err(|e| s3_err(format!("presign {object_key}: {e:?}")))?;
                Ok(DownloadUrl {
                    url,
                    requires_access_token: false,
                })
            }
        }
    }

    async fn open_reader(
        &self,
        key: &ExportPartKey,
    ) -> StorageResult<std::pin::Pin<Box<dyn AsyncRead + Send>>> {
        let object_key = Self::object_key(key);
        let data = self
            .client
            .get_object(&self.bucket, &object_key)
            .await
            .map_err(|e| s3_err(format!("get {object_key}: {e:?}")))?
            .ok_or_else(|| s3_err(format!("export object not found: {object_key}")))?;
        Ok(Box::pin(std::io::Cursor::new(data.bytes)))
    }

    async fn delete_job_outputs(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> StorageResult<()> {
        let prefix = Self::job_prefix(tenant.tenant_id().as_str(), job_id);
        let mut continuation: Option<String> = None;
        loop {
            let page = self
                .client
                .list_objects(&self.bucket, &prefix, continuation.as_deref(), Some(1000))
                .await
                .map_err(|e| s3_err(format!("list {prefix}: {e:?}")))?;
            for item in &page.items {
                self.client
                    .delete_object(&self.bucket, &item.key)
                    .await
                    .map_err(|e| s3_err(format!("delete {}: {e:?}", item.key)))?;
            }
            match page.next_continuation_token {
                Some(token) => continuation = Some(token),
                None => break,
            }
        }
        Ok(())
    }
}
