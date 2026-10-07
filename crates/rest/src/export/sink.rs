//! `ExportSink` trait and implementations.
//!
//! A sink abstracts where export output files are stored.
//! - [`FilesystemSink`] — writes to a local directory
//! - [`InMemorySink`] — holds data in memory (useful for testing)
//! - [`S3Sink`] — streams shards to AWS S3 and returns pre-signed GET URLs
//!   (available when the `s3` feature is enabled)

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

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
/// job an earlier process already completed (#1474). `S3Sink` stores the
/// same record at `{key_prefix}exports/{job_id}/job.json` (#1801). Not part
/// of the FHIR wire format: this is server-internal bookkeeping only,
/// mirroring [`JobStatus::Completed`](super::controller::JobStatus::Completed).
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
/// directory (filesystem) or under the job's key prefix (S3).
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

    /// Like [`download_url`](Self::download_url), but the URL must not stay
    /// valid longer than `max_lifetime`.
    ///
    /// `max_lifetime` is what is left of the job's retention
    /// (`HFS_EXPORT_OUTPUT_TTL`), so a URL handed out late in the window does
    /// not outlive the output the reaper deletes (#1706). The default ignores
    /// the cap because a server-routed URL has no lifetime of its own;
    /// [`S3Sink`] overrides it.
    fn download_url_capped(
        &self,
        public_base_url: &str,
        job_id: &str,
        filename: &str,
        max_lifetime: Duration,
    ) -> Result<String, ExportError> {
        let _ = max_lifetime;
        self.download_url(public_base_url, job_id, filename)
    }

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
    /// Only [`InMemorySink`] (tests only) keeps the default no-op, so a job on
    /// it does not survive a restart. A failure here is non-fatal to the
    /// caller: it degrades to that same behavior rather than failing the
    /// export.
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
}

// ============================================================================
// FilesystemSink
// ============================================================================

/// Writes export shards to a local filesystem directory.
///
/// Shard files are stored at `{dir}/{job_id}/shard-0.{ext}`.
/// Public URLs are `{base_url}/export/{job_id}/shard-0.{ext}`.
///
/// On Unix, directories it creates are `0700` and files `0600` (export output
/// is PHI); an existing root directory is left as the operator made it.
#[derive(Clone)]
pub struct FilesystemSink {
    dir: PathBuf,
}

/// Creates `dir` and any missing parents owner-only (0700) on Unix: export
/// output is PHI. Elsewhere it behaves as `create_dir_all`. An existing
/// directory is left untouched.
fn create_private_dir_all(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Writes `data` to `path`, creating the file owner-only (0600) on Unix.
fn write_private_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    std::io::Write::write_all(&mut options.open(path)?, data)
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
        create_private_dir_all(&job_dir)
            .map_err(|e| ExportError::Sink(format!("failed to create job dir: {e}")))?;

        let filename = format!("shard-{shard_index}.{ext}");
        let path = job_dir.join(&filename);
        write_private_file(&path, &data)
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
        create_private_dir_all(&job_dir)
            .map_err(|e| ExportError::Sink(format!("failed to create job dir: {e}")))?;

        let data = serde_json::to_vec_pretty(manifest)
            .map_err(|e| ExportError::Sink(format!("failed to serialize export manifest: {e}")))?;
        let tmp_path = job_dir.join(format!("{MANIFEST_FILENAME}.tmp"));
        write_private_file(&tmp_path, &data)
            .map_err(|e| ExportError::Sink(format!("failed to write export manifest: {e}")))?;
        std::fs::rename(&tmp_path, job_dir.join(MANIFEST_FILENAME))
            .map_err(|e| ExportError::Sink(format!("failed to finalize export manifest: {e}")))?;
        Ok(())
    }

    /// Scans `dir` for `{job_id}/job.json` manifests and parses each one.
    ///
    /// A directory with no manifest (a job completed before this fix
    /// shipped, or one that never reached `Completed`) is silently skipped —
    /// never deleted, never treated as an error. Likewise an unparsable or
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

    /// Export output is PHI: new directories are 0700 and files 0600 (#1706).
    /// Assumes a normal umask (022 or 077); neither widens these modes.
    #[cfg(unix)]
    #[test]
    fn filesystem_sink_creates_owner_only_dirs_and_files() {
        use std::os::unix::fs::PermissionsExt;

        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("exports");
        let sink = FilesystemSink::new(&root, "http://localhost");

        sink.write_shard("job-1", 0, b"{}\n".to_vec(), "ndjson")
            .unwrap();
        // A zero-shard job: `persist_completion` creates the job dir itself.
        sink.persist_completion(
            "job-2",
            &JobManifest {
                version: MANIFEST_VERSION,
                job_id: "job-2".into(),
                tenant_id: "t1".into(),
                format: "ndjson".into(),
                files: vec![],
                submitted_at: Utc::now(),
                completed_at: Utc::now(),
                client_tracking_id: None,
            },
        )
        .unwrap();

        assert_eq!(mode(&root), 0o700);
        assert_eq!(mode(&root.join("job-1")), 0o700);
        assert_eq!(mode(&root.join("job-1/shard-0.ndjson")), 0o600);
        assert_eq!(mode(&root.join("job-2")), 0o700);
        assert_eq!(mode(&root.join("job-2/job.json")), 0o600);
    }

    /// Only `{root}{job_id}/job.json` is a manifest key (#1801).
    #[test]
    fn manifest_job_id_accepts_only_one_segment_job_json() {
        let id = "3f2b8c1e-7d4a-4e5b-9a6c-1d2e3f4a5b6c";
        assert_eq!(
            manifest_job_id(&format!("exports/{id}/job.json"), "exports/"),
            Some(id)
        );
        assert_eq!(manifest_job_id("exports/a/b/job.json", "exports/"), None);
        assert_eq!(manifest_job_id("exports//job.json", "exports/"), None);
        assert_eq!(manifest_job_id("exports/job.json", "exports/"), None);
        assert_eq!(
            manifest_job_id(&format!("exports/{id}/shard-0.ndjson"), "exports/"),
            None
        );
        assert_eq!(
            manifest_job_id(&format!("other/{id}/job.json"), "exports/"),
            None
        );
        assert_eq!(
            manifest_job_id(&format!("hfs/exports/{id}/job.json"), "hfs/exports/"),
            Some(id)
        );
    }

    /// The pre-signed lifetime is `min(presign_ttl, cap)`: a cap below the
    /// configured TTL shortens the URL, a cap above it changes nothing (#1706).
    #[cfg(feature = "s3")]
    #[tokio::test(flavor = "multi_thread")]
    async fn s3_download_url_capped_never_outlives_the_cap() {
        let conf = aws_sdk_s3::Config::builder()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .region(aws_sdk_s3::config::Region::new("us-east-1"))
            .credentials_provider(aws_sdk_s3::config::Credentials::new(
                "AKID", "SECRET", None, None, "test",
            ))
            .build();
        let sink = S3Sink {
            client: Arc::new(aws_sdk_s3::Client::from_conf(conf)),
            bucket: "bucket".to_string(),
            key_prefix: String::new(),
            presign_ttl_secs: 86_400,
        };

        let capped = sink
            .download_url_capped("", "job-1", "shard-0.ndjson", Duration::from_secs(3_600))
            .unwrap();
        assert!(capped.contains("X-Amz-Expires=3600"), "{capped}");

        let uncapped = sink.download_url("", "job-1", "shard-0.ndjson").unwrap();
        assert!(uncapped.contains("X-Amz-Expires=86400"), "{uncapped}");

        let loose = sink
            .download_url_capped("", "job-1", "shard-0.ndjson", Duration::from_secs(200_000))
            .unwrap();
        assert!(loose.contains("X-Amz-Expires=86400"), "{loose}");
    }
}

// ============================================================================
// S3Sink
// ============================================================================

/// The job id of a manifest key: `Some` only for exactly
/// `{root}{job_id}/job.json` with a non-empty `job_id` containing no `/`.
#[cfg(any(feature = "s3", test))]
fn manifest_job_id<'a>(key: &'a str, root: &str) -> Option<&'a str> {
    let job_id = key
        .strip_prefix(root)?
        .strip_suffix(MANIFEST_FILENAME)?
        .strip_suffix('/')?;
    (!job_id.is_empty() && !job_id.contains('/')).then_some(job_id)
}

/// Writes export shards to an AWS S3 bucket and returns pre-signed GET URLs.
///
/// Objects are stored at `{key_prefix}exports/{job_id}/shard-0.{ext}`.
/// `write_shard` uploads the shard and returns its filename; each manifest poll
/// then pre-signs a GET URL valid for `presign_ttl_secs` seconds (capped by the
/// controller at the job's remaining retention) so clients can download
/// directly from S3.
///
/// A completed job's [`JobManifest`] is stored next to its shards as `job.json`,
/// so the job survives a restart and is still reaped after its retention
/// (#1801). Building an [`InMemoryController`](super::in_memory::InMemoryController) over
/// an `S3Sink` therefore does blocking S3 I/O at construction (`load_completed`
/// via `block_in_place`), so it needs a multi-threaded Tokio runtime. The
/// startup cost is one listing of every key under `{key_prefix}exports/` plus
/// one GET per manifest.
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

    /// Builds an `S3Sink` around an already-configured client, e.g. one aimed
    /// at an S3-compatible endpoint such as MinIO with path-style addressing,
    /// which [`from_config`](Self::from_config) cannot express.
    ///
    /// - `key_prefix` — prepended verbatim to every object key, so include a
    ///   trailing `/` (e.g. `"hfs/"`), or pass it empty
    /// - `presign_ttl_secs` — lifetime of pre-signed GET URLs in seconds
    ///   (the controller may shorten it to a job's remaining retention)
    pub fn from_client(
        client: aws_sdk_s3::Client,
        bucket: impl Into<String>,
        key_prefix: impl Into<String>,
        presign_ttl_secs: u64,
    ) -> Self {
        Self {
            client: Arc::new(client),
            bucket: bucket.into(),
            key_prefix: key_prefix.into(),
            presign_ttl_secs,
        }
    }

    /// Returns the S3 object key for a given job/filename.
    fn object_key(&self, job_id: &str, filename: &str) -> String {
        format!("{}exports/{}/{}", self.key_prefix, job_id, filename)
    }

    /// Pre-signs a GET URL for the shard, valid for `ttl` from now. Presigning
    /// is a local signature computation (no network round trip).
    fn presigned_get_url(
        &self,
        job_id: &str,
        filename: &str,
        ttl: Duration,
    ) -> Result<String, ExportError> {
        let key = self.object_key(job_id, filename);
        let bucket = self.bucket.clone();
        let client = Arc::clone(&self.client);

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let presigning_config = aws_sdk_s3::presigning::PresigningConfig::expires_in(ttl)
                    .map_err(|e| {
                    ExportError::Sink(format!("PresigningConfig::expires_in failed: {e}"))
                })?;
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
    /// trip), so re-signing on every manifest poll is cheap. The controller
    /// normally calls [`download_url_capped`](Self::download_url_capped)
    /// instead, which shortens the lifetime to the job's remaining retention.
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
        self.presigned_get_url(job_id, filename, Duration::from_secs(self.presign_ttl_secs))
    }

    /// Pre-signs for `min(presign_ttl_secs, max_lifetime)`, so the URL does not
    /// outlive the object the reaper deletes (#1706).
    fn download_url_capped(
        &self,
        _public_base_url: &str,
        job_id: &str,
        filename: &str,
        max_lifetime: Duration,
    ) -> Result<String, ExportError> {
        let ttl = Duration::from_secs(self.presign_ttl_secs).min(max_lifetime);
        self.presigned_get_url(job_id, filename, ttl)
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
    /// The manifest (`job.json`) is deleted last, after every other object, so
    /// a partial failure leaves the job reloadable and a restart's startup
    /// reap retries the delete (#1801).
    ///
    /// Idempotent: a job with no objects produces an empty listing and returns
    /// `Ok(())`, so re-deleting an already-cleaned job is a harmless no-op
    /// (S3 reports success when deleting a missing key).
    /// Runs on the blocking pool (like [`write_shard`](Self::write_shard) /
    /// [`read_shard`](Self::read_shard)) to bridge the synchronous trait method
    /// to the async S3 SDK.
    fn delete_job(&self, job_id: &str) -> Result<(), ExportError> {
        let bucket = self.bucket.clone();
        let client = Arc::clone(&self.client);
        // `object_key` with an empty filename yields the job's key prefix.
        let prefix = format!("{}exports/{}/", self.key_prefix, job_id);
        let manifest_key = self.object_key(job_id, MANIFEST_FILENAME);

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
                            if key == manifest_key {
                                continue;
                            }
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

                client
                    .delete_object()
                    .bucket(&bucket)
                    .key(&manifest_key)
                    .send()
                    .await
                    .map_err(|e| ExportError::Sink(format!("S3 delete_object failed: {e}")))?;
                Ok(())
            })
        })
    }

    /// Stores the manifest as `{key_prefix}exports/{job_id}/job.json`, next to
    /// the job's shards (#1801). One PutObject is atomic (readers see the whole
    /// object or none), so there is no temp-and-rename step like
    /// [`FilesystemSink`]'s. It lives under the job's prefix, so
    /// [`delete_job`](Self::delete_job) removes it with the shards on cancel
    /// and when the reaper expires the job.
    fn persist_completion(&self, job_id: &str, manifest: &JobManifest) -> Result<(), ExportError> {
        let data = serde_json::to_vec_pretty(manifest)
            .map_err(|e| ExportError::Sink(format!("failed to serialize export manifest: {e}")))?;
        let key = self.object_key(job_id, MANIFEST_FILENAME);
        let bucket = self.bucket.clone();
        let client = Arc::clone(&self.client);
        let data_len = data.len() as i64;

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                client
                    .put_object()
                    .bucket(&bucket)
                    .key(&key)
                    .content_type("application/json")
                    .body(aws_sdk_s3::primitives::ByteStream::from(data))
                    .content_length(data_len)
                    .send()
                    .await
                    .map_err(|e| {
                        ExportError::Sink(format!(
                            "S3 put_object failed for export manifest: {}",
                            aws_sdk_s3::error::DisplayErrorContext(&e)
                        ))
                    })?;
                Ok(())
            })
        })
    }

    /// Lists `{key_prefix}exports/` and parses every `{job_id}/job.json` it
    /// finds (#1801).
    ///
    /// Pages through the whole listing: a single page would silently forget
    /// every job past the first 1000 keys. A job prefix without a manifest (in
    /// flight at a crash) is skipped and never deleted. Unparsable or
    /// unknown-version manifests and failed reads are logged and skipped; a
    /// listing failure returns whatever was found so far. It never fails
    /// controller construction.
    ///
    /// No URL is stored: the controller pre-signs a fresh one on every poll,
    /// capped from the restored `completed_at`.
    fn load_completed(&self) -> Vec<(String, JobManifest)> {
        let bucket = self.bucket.clone();
        let client = Arc::clone(&self.client);
        let root = format!("{}exports/", self.key_prefix);

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let mut out = Vec::new();
                let mut continuation: Option<String> = None;
                loop {
                    let mut req = client.list_objects_v2().bucket(&bucket).prefix(&root);
                    if let Some(token) = &continuation {
                        req = req.continuation_token(token);
                    }
                    let page = match req.send().await {
                        Ok(page) => page,
                        Err(error) => {
                            tracing::warn!(
                                bucket = %bucket,
                                prefix = %root,
                                error = %aws_sdk_s3::error::DisplayErrorContext(&error),
                                "failed to list S3 export prefix for completed job manifests"
                            );
                            return out;
                        }
                    };

                    for item in page.contents() {
                        let Some(key) = item.key() else { continue };
                        let Some(job_id) = manifest_job_id(key, &root) else {
                            continue;
                        };
                        let data = match client.get_object().bucket(&bucket).key(key).send().await
                        {
                            Ok(obj) => match obj.body.collect().await {
                                Ok(bytes) => bytes.into_bytes(),
                                Err(error) => {
                                    tracing::warn!(
                                        %job_id,
                                        error = %aws_sdk_s3::error::DisplayErrorContext(&error),
                                        "failed to read S3 export manifest"
                                    );
                                    continue;
                                }
                            },
                            Err(error) => {
                                tracing::warn!(
                                        %job_id,
                                        error = %aws_sdk_s3::error::DisplayErrorContext(&error),
                                        "failed to read S3 export manifest"
                                    );
                                continue;
                            }
                        };
                        match serde_json::from_slice::<JobManifest>(&data) {
                            Ok(manifest) if manifest.version == MANIFEST_VERSION => {
                                out.push((job_id.to_string(), manifest));
                            }
                            Ok(manifest) => {
                                tracing::warn!(
                                    job_id = %manifest.job_id,
                                    version = manifest.version,
                                    "skipping export manifest with an unsupported schema version"
                                );
                            }
                            Err(error) => {
                                tracing::warn!(%job_id, %error, "skipping unparsable export manifest");
                            }
                        }
                    }

                    match page.next_continuation_token() {
                        Some(token) => continuation = Some(token.to_string()),
                        None => break,
                    }
                }
                out
            })
        })
    }
}
