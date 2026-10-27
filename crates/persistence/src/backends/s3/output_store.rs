//! S3-backed [`ExportOutputStore`] for multi-instance bulk export.
//!
//! Output NDJSON parts are uploaded to S3-compatible object storage; download
//! URLs are pre-signed `GET` URLs (no token required) by default, or
//! HFS-served URLs when the operator forces token-based access.
//!
//! A part is buffered in a local scratch file between `open_writer` and
//! `finalize_part`, and that scratch file is nested per job
//! (`{scratch}/{tenant}/{job_id}/…`) exactly like the local-filesystem store.
//! A writer whose task is cancelled (lost lease, shutdown) never reaches the
//! `remove_file` in `finalize_part`, so the nesting is what makes the orphan
//! recoverable: [`ExportOutputStore::delete_job_outputs`] removes the whole
//! job directory, which the job-retention sweep already calls.

use std::path::{Path, PathBuf};
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

    /// Overrides the local scratch directory used for in-flight parts.
    ///
    /// Defaults to `{temp_dir}/hfs-export-scratch`.
    pub fn with_scratch_dir(mut self, scratch_dir: impl Into<PathBuf>) -> Self {
        self.scratch_dir = scratch_dir.into();
        self
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

    /// The local scratch directory holding every in-flight part of a job.
    ///
    /// Nested (rather than encoded into a flat file name) so that
    /// [`ExportOutputStore::delete_job_outputs`] can remove a whole job's
    /// leftovers in one call, including parts whose writer was cancelled
    /// before `finalize_part` could clean up.
    fn job_scratch_dir(&self, tenant_id: &str, job_id: &ExportJobId) -> PathBuf {
        self.scratch_dir.join(tenant_id).join(job_id.as_str())
    }

    /// The local scratch path for an in-flight part.
    fn scratch_path(&self, key: &ExportPartKey) -> PathBuf {
        self.job_scratch_dir(&key.tenant_id, &key.job_id)
            .join(format!(
                "{}-{}-{}-{}.tmp",
                key.file_type, key.resource_type, key.part_index, key.fencing_token
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
        let dir = self.job_scratch_dir(&key.tenant_id, &key.job_id);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| s3_err(format!("create scratch dir {}: {e}", dir.display())))?;
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
        // Also drop any local scratch left behind by a part whose writer was
        // cancelled before `finalize_part` removed it.
        delete_dir_idempotent(&self.job_scratch_dir(tenant.tenant_id().as_str(), job_id)).await
    }
}

/// Removes a directory if it exists; a missing directory is `Ok`.
async fn delete_dir_idempotent(dir: &Path) -> StorageResult<()> {
    match tokio::fs::remove_dir_all(dir).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(s3_err(format!("remove scratch dir {}: {e}", dir.display()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::s3::client::{
        ListObjectItem, ListObjectsResult, ObjectData, ObjectMetadata, S3ClientError,
    };
    use crate::core::bulk_submit::SubmissionId;
    use crate::core::bulk_submit_input::submission_output_job_id;
    use crate::core::bulk_submit_output::submit_artifact_key;
    use crate::tenant::{TenantId, TenantPermissions};
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use tokio::io::AsyncReadExt;

    const BUCKET: &str = "test-bucket";

    /// A minimal in-process `S3Api` — enough object storage to exercise the
    /// output store's upload, read and prefix-delete paths without an AWS
    /// account, following the convention of the mock in `s3/tests.rs`.
    #[derive(Default)]
    struct MockS3 {
        objects: Mutex<BTreeMap<String, Vec<u8>>>,
    }

    impl MockS3 {
        fn keys(&self) -> Vec<String> {
            self.objects.lock().unwrap().keys().cloned().collect()
        }
    }

    fn metadata(size: usize) -> ObjectMetadata {
        ObjectMetadata {
            etag: Some("etag-1".to_string()),
            last_modified: None,
            size: size as i64,
        }
    }

    #[async_trait]
    impl S3Api for MockS3 {
        async fn head_bucket(&self, _bucket: &str) -> Result<(), S3ClientError> {
            Ok(())
        }

        async fn head_object(
            &self,
            _bucket: &str,
            key: &str,
        ) -> Result<Option<ObjectMetadata>, S3ClientError> {
            Ok(self
                .objects
                .lock()
                .unwrap()
                .get(key)
                .map(|body| metadata(body.len())))
        }

        async fn get_object(
            &self,
            _bucket: &str,
            key: &str,
        ) -> Result<Option<ObjectData>, S3ClientError> {
            Ok(self
                .objects
                .lock()
                .unwrap()
                .get(key)
                .map(|body| ObjectData {
                    bytes: body.clone(),
                    metadata: metadata(body.len()),
                }))
        }

        async fn put_object(
            &self,
            _bucket: &str,
            key: &str,
            body: Vec<u8>,
            _content_type: Option<&str>,
            _if_match: Option<&str>,
            _if_none_match: Option<&str>,
        ) -> Result<ObjectMetadata, S3ClientError> {
            let size = body.len();
            self.objects.lock().unwrap().insert(key.to_string(), body);
            Ok(metadata(size))
        }

        async fn delete_object(&self, _bucket: &str, key: &str) -> Result<(), S3ClientError> {
            self.objects.lock().unwrap().remove(key);
            Ok(())
        }

        async fn list_objects(
            &self,
            _bucket: &str,
            prefix: &str,
            _continuation: Option<&str>,
            _max_keys: Option<i32>,
        ) -> Result<ListObjectsResult, S3ClientError> {
            let items = self
                .objects
                .lock()
                .unwrap()
                .iter()
                .filter(|(key, _)| key.starts_with(prefix))
                .map(|(key, body)| ListObjectItem {
                    key: key.clone(),
                    etag: None,
                    last_modified: None,
                    size: body.len() as i64,
                })
                .collect();
            Ok(ListObjectsResult {
                items,
                next_continuation_token: None,
            })
        }

        async fn list_common_prefixes(
            &self,
            _bucket: &str,
            _prefix: &str,
            _delimiter: &str,
        ) -> Result<Vec<String>, S3ClientError> {
            Ok(Vec::new())
        }

        async fn presign_get(
            &self,
            _bucket: &str,
            key: &str,
            _ttl: Duration,
        ) -> Result<String, S3ClientError> {
            Ok(format!(
                "https://example.invalid/{key}?X-Amz-Signature=mock"
            ))
        }
    }

    /// A store whose scratch directory is a private temp dir, plus the mock it
    /// uploads to.
    fn test_store(scratch: &std::path::Path) -> (S3OutputStore, Arc<MockS3>) {
        let client = Arc::new(MockS3::default());
        let store = S3OutputStore::new(
            client.clone() as Arc<dyn S3Api>,
            BUCKET,
            "http://localhost:8080",
            AccessTokenMode::Auto,
            Duration::from_secs(60),
        )
        .with_scratch_dir(scratch);
        (store, client)
    }

    fn tenant(id: &str) -> TenantContext {
        TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
    }

    #[tokio::test]
    async fn scratch_files_are_nested_under_tenant_and_job() {
        let scratch = tempfile::tempdir().unwrap();
        let (store, _client) = test_store(scratch.path());
        let job = ExportJobId::new();
        let key = ExportPartKey::output("t1", job.clone(), "Patient", 0, 1);

        let expected_dir = scratch.path().join("t1").join(job.as_str());
        assert_eq!(store.job_scratch_dir("t1", &job), expected_dir);
        assert_eq!(
            store.scratch_path(&key),
            expected_dir.join("output-Patient-0-1.tmp")
        );
    }

    #[tokio::test]
    async fn orphan_scratch_is_reclaimed_by_delete_job_outputs() {
        let scratch = tempfile::tempdir().unwrap();
        let (store, client) = test_store(scratch.path());
        let job = ExportJobId::new();
        let key = ExportPartKey::output("t1", job.clone(), "Patient", 0, 1);

        // A writer that is never finalized — the shape a cancelled worker
        // future leaves behind.
        let writer = store.open_writer(&key).await.unwrap();
        drop(writer);
        assert!(store.scratch_path(&key).exists());
        assert!(client.keys().is_empty());

        let tenant = tenant("t1");
        store.delete_job_outputs(&tenant, &job).await.unwrap();
        assert!(!store.scratch_path(&key).exists());
        assert!(!store.job_scratch_dir("t1", &job).exists());

        // Idempotent: a second sweep, and a job that never wrote anything,
        // both succeed.
        store.delete_job_outputs(&tenant, &job).await.unwrap();
        store
            .delete_job_outputs(&tenant, &ExportJobId::new())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn happy_path_uploads_the_object_and_leaves_no_scratch() {
        let scratch = tempfile::tempdir().unwrap();
        let (store, client) = test_store(scratch.path());
        let job = ExportJobId::new();
        let key = ExportPartKey::output("t1", job.clone(), "Patient", 0, 1);
        let lines = [
            r#"{"resourceType":"Patient","id":"1"}"#,
            r#"{"resourceType":"Patient","id":"2","name":[{"family":"Muñoz"}]}"#,
        ];
        let expected: String = lines.iter().map(|line| format!("{line}\n")).collect();

        let mut writer = store.open_writer(&key).await.unwrap();
        for line in lines {
            writer.write_line(line).await.unwrap();
        }
        let finalized = store.finalize_part(&key, writer).await.unwrap();
        assert_eq!(finalized.line_count, 2);
        assert_eq!(finalized.size_bytes, expected.len() as u64);

        let object_key = format!("t1/exports/{job}/output-Patient-0-1.ndjson");
        assert_eq!(client.keys(), vec![object_key]);
        let mut reader = store.open_reader(&key).await.unwrap();
        let mut content = String::new();
        reader.read_to_string(&mut content).await.unwrap();
        assert_eq!(content, expected);

        // Finalization removes its own scratch file, leaving only the (empty)
        // job directory for the retention sweep.
        assert!(!store.scratch_path(&key).exists());

        let tenant = tenant("t1");
        store.delete_job_outputs(&tenant, &job).await.unwrap();
        assert!(client.keys().is_empty());
        assert!(!store.job_scratch_dir("t1", &job).exists());
        store.delete_job_outputs(&tenant, &job).await.unwrap();
    }

    #[tokio::test]
    async fn bulk_submit_artifact_keys_round_trip_and_are_swept() {
        let scratch = tempfile::tempdir().unwrap();
        let (store, client) = test_store(scratch.path());
        let tenant = tenant("t1");
        let submission = SubmissionId::new("submitter-a", "submission-a");
        let job = submission_output_job_id(&submission);
        let finalized_key = submit_artifact_key(
            &tenant,
            &submission,
            "manifest-a",
            "output",
            Some("Patient"),
            0,
            1,
        );
        let orphan_key = submit_artifact_key(
            &tenant,
            &submission,
            "manifest-a",
            "error",
            Some("Patient"),
            0,
            1,
        );

        // Submit's keys carry the submission-level job id, so they nest under
        // the same directory the submission cleanup deletes.
        assert!(
            store
                .scratch_path(&finalized_key)
                .starts_with(store.job_scratch_dir("t1", &job))
        );

        let mut writer = store.open_writer(&finalized_key).await.unwrap();
        writer
            .write_line(r#"{"resourceType":"Patient"}"#)
            .await
            .unwrap();
        store.finalize_part(&finalized_key, writer).await.unwrap();
        drop(store.open_writer(&orphan_key).await.unwrap());

        let mut reader = store.open_reader(&finalized_key).await.unwrap();
        let mut content = String::new();
        reader.read_to_string(&mut content).await.unwrap();
        assert_eq!(content, "{\"resourceType\":\"Patient\"}\n");
        assert_eq!(client.keys().len(), 1);
        assert!(store.scratch_path(&orphan_key).exists());

        store.delete_job_outputs(&tenant, &job).await.unwrap();
        assert!(client.keys().is_empty());
        assert!(!store.scratch_path(&orphan_key).exists());
        assert!(!store.job_scratch_dir("t1", &job).exists());
    }
}
