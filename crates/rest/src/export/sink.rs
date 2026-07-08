//! `ExportSink` trait and implementations.
//!
//! A sink abstracts where export output files are stored.
//! - [`FilesystemSink`] — writes to a local directory
//! - [`InMemorySink`] — holds data in memory (useful for testing)
//! - [`S3Sink`] — streams shards to AWS S3 and returns pre-signed GET URLs
//!   (available when the `s3` feature is enabled)

use std::path::PathBuf;
use std::sync::Arc;

use dashmap::DashMap;

use super::controller::ExportError;

/// Trait for writing and serving export output files.
pub trait ExportSink: Send + Sync + Clone + 'static {
    /// Writes `data` as the `shard_index`-th shard for `job_id` and returns the
    /// shard's filename (its stable identity within the job, e.g.
    /// `shard-0.ndjson`).
    ///
    /// The filename — not a public URL — is what the controller stores in
    /// [`CompletedFile`](super::controller::CompletedFile); the URL is resolved
    /// later via [`download_url`](Self::download_url). The `ext` parameter is
    /// the file extension without the leading dot, e.g. `"ndjson"`, `"csv"`, or
    /// `"parquet"`.
    fn write_shard(
        &self,
        job_id: &str,
        shard_index: usize,
        data: Vec<u8>,
        ext: &str,
    ) -> Result<String, ExportError>;

    /// Reads back the raw bytes for a shard (used by the download handler).
    ///
    /// Returns `None` if the shard does not exist.
    fn read_shard(&self, job_id: &str, filename: &str) -> Option<Vec<u8>>;

    /// Resolves a shard `filename` to a public download URL.
    ///
    /// Called once per shard each time the completion manifest is rendered, so
    /// the returned URL should carry a full validity window:
    /// - server-routed sinks (filesystem / in-memory) return their stable
    ///   `{base_url}/export/{job_id}/{filename}` route;
    /// - [`S3Sink`] returns a freshly pre-signed GET URL, so a client polling
    ///   the manifest hours after completion still receives a usable URL rather
    ///   than one signed (and counting down) since write time.
    fn download_url(&self, job_id: &str, filename: &str) -> Result<String, ExportError>;

    /// Deletes all output shards previously written for `job_id`.
    ///
    /// Invoked when a job is cancelled so partial results don't linger or
    /// remain downloadable (SQL-on-FHIR operations-common, HL7/sql-on-fhir#365).
    /// Best-effort: a job with no written shards (or an unknown `job_id`) is
    /// not an error.
    fn delete_job(&self, job_id: &str) -> Result<(), ExportError>;
}

// ============================================================================
// FilesystemSink
// ============================================================================

/// Writes export shards to a local filesystem directory.
///
/// Shard files are stored at `{dir}/{job_id}/shard-0.{ext}`.
/// Public URLs are `{base_url}/export/{job_id}/shard-0.{ext}`.
#[derive(Clone)]
pub struct FilesystemSink {
    dir: PathBuf,
    base_url: String,
}

impl FilesystemSink {
    /// Creates a new `FilesystemSink`.
    ///
    /// - `dir` — root directory for export files
    /// - `base_url` — server base URL (e.g. `http://localhost:8080`), used to
    ///   build public download URLs
    pub fn new(dir: impl Into<PathBuf>, base_url: impl Into<String>) -> Self {
        Self {
            dir: dir.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }
}

impl ExportSink for FilesystemSink {
    fn write_shard(
        &self,
        job_id: &str,
        shard_index: usize,
        data: Vec<u8>,
        ext: &str,
    ) -> Result<String, ExportError> {
        let job_dir = self.dir.join(job_id);
        std::fs::create_dir_all(&job_dir)
            .map_err(|e| ExportError::Sink(format!("failed to create job dir: {e}")))?;

        let filename = format!("shard-{shard_index}.{ext}");
        let path = job_dir.join(&filename);
        std::fs::write(&path, data)
            .map_err(|e| ExportError::Sink(format!("failed to write shard: {e}")))?;

        Ok(filename)
    }

    fn read_shard(&self, job_id: &str, filename: &str) -> Option<Vec<u8>> {
        let path = self.dir.join(job_id).join(filename);
        std::fs::read(path).ok()
    }

    fn download_url(&self, job_id: &str, filename: &str) -> Result<String, ExportError> {
        // Stable server-routed URL; served back by the download handler. No
        // expiry, so it is identical on every poll.
        Ok(format!(
            "{base}/export/{job_id}/{filename}",
            base = self.base_url
        ))
    }

    fn delete_job(&self, job_id: &str) -> Result<(), ExportError> {
        let job_dir = self.dir.join(job_id);
        match std::fs::remove_dir_all(&job_dir) {
            Ok(()) => Ok(()),
            // A job that never wrote a shard has no directory — not an error.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(ExportError::Sink(format!("failed to delete job dir: {e}"))),
        }
    }
}

// ============================================================================
// InMemorySink (tests only)
// ============================================================================

/// In-memory sink that stores shards in a `DashMap`.  Intended for tests.
#[derive(Clone)]
pub struct InMemorySink {
    data: Arc<DashMap<String, Vec<u8>>>,
    base_url: String,
}

impl InMemorySink {
    /// Creates a new `InMemorySink` with the given public base URL.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            data: Arc::new(DashMap::new()),
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }
}

impl ExportSink for InMemorySink {
    fn write_shard(
        &self,
        job_id: &str,
        shard_index: usize,
        data: Vec<u8>,
        ext: &str,
    ) -> Result<String, ExportError> {
        let filename = format!("shard-{shard_index}.{ext}");
        let key = format!("{job_id}/{filename}");
        self.data.insert(key, data);
        Ok(filename)
    }

    fn read_shard(&self, job_id: &str, filename: &str) -> Option<Vec<u8>> {
        let key = format!("{job_id}/{filename}");
        self.data.get(&key).map(|v| v.clone())
    }

    fn download_url(&self, job_id: &str, filename: &str) -> Result<String, ExportError> {
        // Mirrors the filesystem sink's stable server route.
        Ok(format!(
            "{base}/export/{job_id}/{filename}",
            base = self.base_url
        ))
    }

    fn delete_job(&self, job_id: &str) -> Result<(), ExportError> {
        // Shards are keyed `{job_id}/{filename}`, so dropping every entry under
        // the `{job_id}/` prefix removes exactly this job's output and leaves
        // other jobs untouched. Always succeeds — a job with no shards is a
        // no-op retain.
        let prefix = format!("{job_id}/");
        self.data.retain(|k, _| !k.starts_with(&prefix));
        Ok(())
    }
}

// ============================================================================
// S3Sink
// ============================================================================

/// Writes export shards to an AWS S3 bucket and returns pre-signed GET URLs.
///
/// Objects are stored at `{key_prefix}exports/{job_id}/shard-0.{ext}`.
/// `write_shard` uploads the shard and returns a pre-signed URL valid for
/// `presign_ttl_secs` seconds so clients can download directly from S3.
///
/// Requires the `s3` feature flag.
#[cfg(feature = "s3")]
#[derive(Clone)]
pub struct S3Sink {
    client: Arc<aws_sdk_s3::Client>,
    bucket: String,
    /// Optional key prefix (e.g. `"hfs/"`) prepended to every object key.
    key_prefix: String,
    presign_ttl_secs: u64,
}

#[cfg(feature = "s3")]
impl S3Sink {
    /// Constructs an `S3Sink` by loading AWS credentials from the environment.
    ///
    /// - `bucket` — target S3 bucket
    /// - `region` — optional region override; falls back to AWS credential chain
    /// - `key_prefix` — string prepended to every object key (may be empty)
    /// - `presign_ttl_secs` — lifetime of pre-signed GET URLs in seconds
    pub async fn from_config(
        bucket: String,
        region: Option<String>,
        key_prefix: String,
        presign_ttl_secs: u64,
    ) -> Result<Self, ExportError> {
        let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest());
        if let Some(r) = region {
            loader = loader.region(aws_config::Region::new(r));
        }
        let sdk_config = loader.load().await;
        let client = aws_sdk_s3::Client::new(&sdk_config);

        Ok(Self {
            client: Arc::new(client),
            bucket,
            key_prefix,
            presign_ttl_secs,
        })
    }

    /// Returns the S3 object key for a given job/filename.
    fn object_key(&self, job_id: &str, filename: &str) -> String {
        format!("{}exports/{}/{}", self.key_prefix, job_id, filename)
    }
}

#[cfg(feature = "s3")]
impl ExportSink for S3Sink {
    /// Uploads the shard to S3 and returns its filename. The pre-signed GET URL
    /// is produced later (per manifest poll) by [`download_url`](Self::download_url),
    /// not here, so its TTL window starts at poll time rather than write time.
    fn write_shard(
        &self,
        job_id: &str,
        shard_index: usize,
        data: Vec<u8>,
        ext: &str,
    ) -> Result<String, ExportError> {
        let filename = format!("shard-{shard_index}.{ext}");
        let key = self.object_key(job_id, &filename);
        let bucket = self.bucket.clone();
        let client = Arc::clone(&self.client);
        let data_len = data.len() as i64;

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                client
                    .put_object()
                    .bucket(&bucket)
                    .key(&key)
                    .body(aws_sdk_s3::primitives::ByteStream::from(data))
                    .content_length(data_len)
                    .send()
                    .await
                    .map_err(|e| ExportError::Sink(format!("S3 put_object failed: {e}")))?;
                Ok(())
            })
        })?;

        Ok(filename)
    }

    /// Pre-signs a fresh GET URL for the shard, valid for `presign_ttl_secs`
    /// from now. Presigning is a local signature computation (no network round
    /// trip), so re-signing on every manifest poll is cheap.
    ///
    /// Note: the URL's effective lifetime is also bounded by the signing
    /// credentials' own validity (e.g. an STS session), which can silently
    /// undercut `presign_ttl_secs` when running with temporary credentials.
    fn download_url(&self, job_id: &str, filename: &str) -> Result<String, ExportError> {
        let key = self.object_key(job_id, filename);
        let bucket = self.bucket.clone();
        let client = Arc::clone(&self.client);
        let presign_ttl = std::time::Duration::from_secs(self.presign_ttl_secs);

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let presigning_config =
                    aws_sdk_s3::presigning::PresigningConfig::expires_in(presign_ttl).map_err(
                        |e| ExportError::Sink(format!("PresigningConfig::expires_in failed: {e}")),
                    )?;
                let presigned = client
                    .get_object()
                    .bucket(&bucket)
                    .key(&key)
                    .presigned(presigning_config)
                    .await
                    .map_err(|e| ExportError::Sink(format!("S3 presign failed: {e}")))?;
                Ok(presigned.uri().to_string())
            })
        })
    }

    /// Downloads the raw shard bytes from S3.
    ///
    /// Returns `None` if the object does not exist or the download fails.
    fn read_shard(&self, job_id: &str, filename: &str) -> Option<Vec<u8>> {
        let key = self.object_key(job_id, filename);
        let bucket = self.bucket.clone();
        let client = Arc::clone(&self.client);

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                match client.get_object().bucket(&bucket).key(&key).send().await {
                    Ok(out) => out
                        .body
                        .collect()
                        .await
                        .ok()
                        .map(|b| b.into_bytes().to_vec()),
                    Err(_) => None,
                }
            })
        })
    }

    /// Lists every object under `{key_prefix}exports/{job_id}/` and deletes
    /// them, paging through the listing until exhausted.
    ///
    /// Idempotent: a job with no objects produces an empty listing and returns
    /// `Ok(())`, so re-deleting an already-cleaned job is a harmless no-op.
    /// Runs on the blocking pool (like [`write_shard`](Self::write_shard) /
    /// [`read_shard`](Self::read_shard)) to bridge the synchronous trait method
    /// to the async S3 SDK.
    fn delete_job(&self, job_id: &str) -> Result<(), ExportError> {
        let bucket = self.bucket.clone();
        let client = Arc::clone(&self.client);
        // `object_key` with an empty filename yields the job's key prefix.
        let prefix = format!("{}exports/{}/", self.key_prefix, job_id);

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let mut continuation: Option<String> = None;
                loop {
                    let mut req = client.list_objects_v2().bucket(&bucket).prefix(&prefix);
                    if let Some(token) = &continuation {
                        req = req.continuation_token(token);
                    }
                    let out = req.send().await.map_err(|e| {
                        ExportError::Sink(format!("S3 list_objects_v2 failed: {e}"))
                    })?;

                    for item in out.contents() {
                        if let Some(key) = item.key() {
                            client
                                .delete_object()
                                .bucket(&bucket)
                                .key(key)
                                .send()
                                .await
                                .map_err(|e| {
                                    ExportError::Sink(format!("S3 delete_object failed: {e}"))
                                })?;
                        }
                    }

                    match out.next_continuation_token() {
                        Some(token) => continuation = Some(token.to_string()),
                        None => break,
                    }
                }
                Ok(())
            })
        })
    }
}
