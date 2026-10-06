//! `ExportSink` trait and implementations.
//!
//! A sink abstracts where export output files are stored.
//! - [`FilesystemSink`] — writes to a local directory
//! - [`InMemorySink`] — holds data in memory (useful for testing)
//! - [`S3Sink`] — streams shards to AWS S3 and returns pre-signed GET URLs
//!   (available when the `s3` feature is enabled)

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use dashmap::DashMap;

use super::controller::ExportError;

/// Current schema version for persisted completion manifests (see
/// [`JobManifest`]). An unparsable manifest, or one written by a version this
/// build doesn't recognize, is always treated as "no manifest" rather than
/// causing a crash — see [`FilesystemSink::load_completed`].
pub(crate) const MANIFEST_VERSION: u32 = 1;

/// A durable, serializable record of a completed export job.
///
/// [`FilesystemSink::persist_completion`] writes one of these next to a job's
/// shards (`{dir}/{job_id}/job.json`) when the job finishes, and
/// [`FilesystemSink::load_completed`] reads them back at controller
/// construction so a fresh process — after a restart — can keep serving a
/// job an earlier process already completed (#1474). Not part of the FHIR
/// wire format: this is server-internal bookkeeping only, mirroring
/// [`JobStatus::Completed`](super::controller::JobStatus::Completed).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JobManifest {
    /// Manifest schema version this record was written with (see the crate's
    /// internal `MANIFEST_VERSION` constant).
    pub version: u32,
    /// The job id this manifest describes. A rehydrating controller MUST
    /// reject any manifest where this disagrees with the storage key it was
    /// loaded from (e.g. the containing directory name) — serving a job's
    /// files by a path that disagrees with the job's own record would let
    /// that path alone decide which job's files get served.
    pub job_id: String,
    /// Tenant that submitted the job, gating status/result/download the same
    /// way [`InMemoryController`](super::in_memory::InMemoryController) gates
    /// a job it ran itself.
    pub tenant_id: String,
    /// Output format echoed in the completion manifest (e.g. `"ndjson"`).
    pub format: String,
    /// Output files produced by the job.
    pub files: Vec<ManifestFile>,
    /// Time the job was submitted.
    pub submitted_at: DateTime<Utc>,
    /// Time the job finished.
    pub completed_at: DateTime<Utc>,
    /// Client-supplied tracking id, echoed back to the caller if present.
    pub client_tracking_id: Option<String>,
}

/// One output file inside a [`JobManifest`], mirroring
/// [`CompletedFile`](super::controller::CompletedFile) (which has no serde
/// derive of its own, hence this separate on-disk shape).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ManifestFile {
    /// Logical view name this file belongs to (matches `view.name`).
    pub view_name: String,
    /// The shard's stable filename within the job (e.g. `shard-0.ndjson`).
    pub filename: String,
    /// Number of data rows written.
    pub row_count: usize,
}

/// Filename a persisted [`JobManifest`] is stored under, inside the job's own
/// directory.
const MANIFEST_FILENAME: &str = "job.json";

fn server_routed_download_url(
    public_base_url: &str,
    job_id: &str,
    filename: &str,
) -> Result<String, ExportError> {
    let mut url = url::Url::parse(public_base_url)
        .map_err(|error| ExportError::Sink(format!("invalid public base URL: {error}")))?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| ExportError::Sink("public base URL cannot hold path segments".into()))?;
        path.pop_if_empty();
        path.extend(["export", job_id, filename]);
    }
    Ok(url.to_string())
}

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
    fn download_url(
        &self,
        public_base_url: &str,
        job_id: &str,
        filename: &str,
    ) -> Result<String, ExportError>;

    /// Deletes all output shards previously written for `job_id`.
    ///
    /// Invoked when a job is cancelled so partial results don't linger or
    /// remain downloadable (SQL-on-FHIR operations-common, HL7/sql-on-fhir#365).
    /// Best-effort: a job with no written shards (or an unknown `job_id`) is
    /// not an error.
    fn delete_job(&self, job_id: &str) -> Result<(), ExportError>;

    /// Persists a durable record of a completed job so a controller built by
    /// a later process — e.g. after a restart — can rehydrate it via
    /// [`load_completed`](Self::load_completed) and keep serving its status,
    /// result and downloads (#1474).
    ///
    /// Called once, right before the job's in-memory status flips to
    /// `Completed` — the transition a status poll reports as a 303 — so that
    /// by the time any client has observed the job as `Completed`, its
    /// manifest is already durable and a restart after that point still
    /// serves the job.
    ///
    /// A sink that doesn't need this — [`InMemorySink`] (tests only) or
    /// [`S3Sink`] (out of scope for now) — keeps the default no-op; a job on
    /// such a sink simply doesn't survive a restart, same as before this fix.
    /// A failure here is likewise non-fatal to the caller: it degrades to
    /// that same pre-fix behavior rather than failing the export.
    fn persist_completion(
        &self,
        _job_id: &str,
        _manifest: &JobManifest,
    ) -> Result<(), ExportError> {
        Ok(())
    }

    /// Loads every persisted completion manifest this sink knows about,
    /// paired with the storage key (e.g. directory name) it was loaded from.
    ///
    /// Called once, at controller construction, to rehydrate `jobs` /
    /// `job_tenants` after a restart. The default returns nothing, so a sink
    /// that doesn't persist completions leaves the controller exactly as
    /// empty as it is today.
    fn load_completed(&self) -> Vec<(String, JobManifest)> {
        Vec::new()
    }

    /// Deletes the output of jobs that ended without a trace: job storage
    /// with no completion manifest whose job `is_known` does not report, left
    /// untouched for longer than `older_than`. A process that stops mid-job
    /// (a crash, a kill) leaves exactly that behind, and neither the job's
    /// task nor the reaper — both gone with the process's in-memory state —
    /// will ever delete it. Returns the job ids whose output was deleted.
    ///
    /// Called by the cleanup reaper — once at controller construction and on
    /// every sweep — with `older_than` set to the output TTL and `is_known`
    /// reporting every job the controller holds a status entry for, so the
    /// output of a running job or of one awaiting a retried delete is never
    /// touched. Output with a manifest is never deleted here either: it is a
    /// completed job, which the reaper expires on its own schedule.
    ///
    /// The default deletes nothing: [`InMemorySink`] lives and dies with the
    /// process, and [`S3Sink`] is out of scope for now (an orphaned S3 prefix
    /// still needs a bucket lifecycle rule).
    fn sweep_orphans(
        &self,
        _is_known: &dyn Fn(&str) -> bool,
        _older_than: Duration,
    ) -> Vec<String> {
        Vec::new()
    }
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
}

impl FilesystemSink {
    /// Creates a new `FilesystemSink`.
    ///
    /// - `dir` — root directory for export files
    /// The second argument is retained for source compatibility. Request
    /// handlers now supply the effective public base when resolving a URL.
    pub fn new(dir: impl Into<PathBuf>, _base_url: impl Into<String>) -> Self {
        Self { dir: dir.into() }
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

    fn download_url(
        &self,
        public_base_url: &str,
        job_id: &str,
        filename: &str,
    ) -> Result<String, ExportError> {
        // Stable server-routed URL; served back by the download handler. No
        // expiry, so it is identical on every poll.
        server_routed_download_url(public_base_url, job_id, filename)
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

    /// Writes the manifest to a temp file and renames it into place, so a
    /// concurrent [`load_completed`](Self::load_completed) scan — this
    /// process's own, at its next restart — never observes a partially
    /// written `job.json`. `create_dir_all` covers the zero-output-file case
    /// (a completed job with no shards has no directory yet).
    fn persist_completion(&self, job_id: &str, manifest: &JobManifest) -> Result<(), ExportError> {
        let job_dir = self.dir.join(job_id);
        std::fs::create_dir_all(&job_dir)
            .map_err(|e| ExportError::Sink(format!("failed to create job dir: {e}")))?;

        let data = serde_json::to_vec_pretty(manifest)
            .map_err(|e| ExportError::Sink(format!("failed to serialize export manifest: {e}")))?;
        let tmp_path = job_dir.join(format!("{MANIFEST_FILENAME}.tmp"));
        std::fs::write(&tmp_path, &data)
            .map_err(|e| ExportError::Sink(format!("failed to write export manifest: {e}")))?;
        std::fs::rename(&tmp_path, job_dir.join(MANIFEST_FILENAME))
            .map_err(|e| ExportError::Sink(format!("failed to finalize export manifest: {e}")))?;
        Ok(())
    }

    /// Scans `dir` for `{job_id}/job.json` manifests and parses each one.
    ///
    /// A directory with no manifest (a job completed before this fix
    /// shipped, or one that never reached `Completed`) is silently skipped —
    /// not deleted here, never treated as an error. The reaper's
    /// [`sweep_orphans`](Self::sweep_orphans) deletes it once it is older
    /// than the output TTL. Likewise an unparsable or
    /// unrecognized-version manifest is logged and skipped rather than
    /// failing controller construction: a single corrupt directory under the
    /// export dir must never stop the server from starting.
    fn load_completed(&self) -> Vec<(String, JobManifest)> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            // No export dir yet (fresh deployment) — nothing to rehydrate.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return out,
            // Anything else (e.g. a permissions problem) silently leaves
            // every already-completed job 404 after a restart — worth a log
            // line rather than failing controller construction outright.
            Err(error) => {
                tracing::warn!(dir = ?self.dir, %error, "failed to scan export dir for completed job manifests");
                return out;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let data = match std::fs::read(path.join(MANIFEST_FILENAME)) {
                Ok(data) => data,
                Err(_) => continue,
            };
            match serde_json::from_slice::<JobManifest>(&data) {
                Ok(manifest) if manifest.version == MANIFEST_VERSION => {
                    out.push((dir_name.to_string(), manifest));
                }
                Ok(manifest) => {
                    tracing::warn!(
                        job_id = %manifest.job_id,
                        version = manifest.version,
                        "skipping export manifest with an unsupported schema version"
                    );
                }
                Err(error) => {
                    tracing::warn!(dir = %dir_name, %error, "skipping unparsable export manifest");
                }
            }
        }
        out
    }

    /// Deletes every `{dir}/{job_id}` directory that has no `job.json` (a
    /// leftover `job.json.tmp` does not count), whose name is not a job
    /// `is_known` reports, and in which nothing — the directory itself or any
    /// file directly in it — was modified within `older_than`.
    ///
    /// Only directories named like a job id (a hyphenated UUID) are
    /// considered, and symlinks never are, so nothing else an operator keeps
    /// under the export dir is touched. A directory whose age or manifest
    /// cannot be read is left alone, and a failed delete is logged and
    /// retried on the next sweep.
    fn sweep_orphans(&self, is_known: &dyn Fn(&str) -> bool, older_than: Duration) -> Vec<String> {
        let mut removed = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return removed,
            Err(error) => {
                tracing::warn!(dir = ?self.dir, %error, "failed to scan export dir for orphaned job output");
                return removed;
            }
        };
        let now = SystemTime::now();
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let path = entry.path();
            let Some(job_id) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !is_job_id(job_id) || is_known(job_id) {
                continue;
            }
            // `try_exists` errs when it cannot tell; keep the directory then.
            if !matches!(path.join(MANIFEST_FILENAME).try_exists(), Ok(false)) {
                continue;
            }
            let Some(touched) = last_modified(&path) else {
                continue;
            };
            // A modification time in the future reads as "just touched".
            if !now
                .duration_since(touched)
                .is_ok_and(|age| age > older_than)
            {
                continue;
            }
            match std::fs::remove_dir_all(&path) {
                Ok(()) => removed.push(job_id.to_string()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    tracing::warn!(%job_id, %error, "failed to delete orphaned export output; retrying on the next sweep");
                }
            }
        }
        removed
    }
}

/// Whether `name` is a job id as `submit` mints them: a hyphenated UUID.
fn is_job_id(name: &str) -> bool {
    uuid::Uuid::parse_str(name).is_ok_and(|u| u.hyphenated().to_string() == name)
}

/// The latest modification time of `dir` and of the entries directly in it
/// (shards are written straight into a job's directory), or `None` when the
/// directory's own time cannot be read.
fn last_modified(dir: &Path) -> Option<SystemTime> {
    let mut latest = std::fs::metadata(dir).and_then(|m| m.modified()).ok()?;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for modified in entries
            .flatten()
            .filter_map(|e| e.metadata().and_then(|m| m.modified()).ok())
        {
            latest = latest.max(modified);
        }
    }
    Some(latest)
}

// ============================================================================
// InMemorySink (tests only)
// ============================================================================

/// In-memory sink that stores shards in a `DashMap`.  Intended for tests.
#[derive(Clone)]
pub struct InMemorySink {
    data: Arc<DashMap<String, Vec<u8>>>,
}

impl InMemorySink {
    /// Creates a new `InMemorySink`.
    ///
    /// The argument is retained for source compatibility. Request handlers
    /// supply the effective public base when resolving a URL.
    pub fn new(_base_url: impl Into<String>) -> Self {
        Self {
            data: Arc::new(DashMap::new()),
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

    fn download_url(
        &self,
        public_base_url: &str,
        job_id: &str,
        filename: &str,
    ) -> Result<String, ExportError> {
        // Mirrors the filesystem sink's stable server route.
        server_routed_download_url(public_base_url, job_id, filename)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filesystem_download_url_uses_the_current_public_base() {
        let sink = FilesystemSink::new("unused", "https://stale.example");
        assert_eq!(
            sink.download_url(
                "https://public.example/fhir",
                "job with space",
                "shard/0.ndjson",
            )
            .unwrap(),
            "https://public.example/fhir/export/job%20with%20space/shard%2F0.ndjson"
        );
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
    fn download_url(
        &self,
        _public_base_url: &str,
        job_id: &str,
        filename: &str,
    ) -> Result<String, ExportError> {
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
