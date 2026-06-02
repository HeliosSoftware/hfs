//! Output storage for bulk export NDJSON files.
//!
//! The [`ExportOutputStore`] trait decouples *where the exported bytes go*
//! (local filesystem, S3, …) from the job-state backend. The job-state
//! backend stores keys; the output store turns keys into bytes and URLs.

use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::core::bulk_export::ExportJobId;
use crate::error::StorageResult;
use crate::tenant::TenantContext;

/// A stable identifier for a single output part.
///
/// The `fencing_token` is embedded so a zombie worker (one that lost its
/// lease) writes to a *different* key than the live worker holding the
/// reclaimed job, preventing output corruption.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExportPartKey {
    /// The tenant the job belongs to.
    pub tenant_id: String,
    /// The export job this part belongs to.
    pub job_id: ExportJobId,
    /// The FHIR resource type contained in the part.
    pub resource_type: String,
    /// `"output"` or `"error"`.
    pub file_type: String,
    /// The zero-based part index within `(job, file_type, resource_type)`.
    pub part_index: u32,
    /// The fencing token of the worker that produced the part.
    pub fencing_token: u64,
}

impl ExportPartKey {
    /// Creates a new output part key (`file_type = "output"`).
    pub fn output(
        tenant_id: impl Into<String>,
        job_id: ExportJobId,
        resource_type: impl Into<String>,
        part_index: u32,
        fencing_token: u64,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            job_id,
            resource_type: resource_type.into(),
            file_type: "output".to_string(),
            part_index,
            fencing_token,
        }
    }

    /// Creates a new error part key (`file_type = "error"`).
    pub fn error(
        tenant_id: impl Into<String>,
        job_id: ExportJobId,
        resource_type: impl Into<String>,
        part_index: u32,
        fencing_token: u64,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            job_id,
            resource_type: resource_type.into(),
            file_type: "error".to_string(),
            part_index,
            fencing_token,
        }
    }

    /// The `{resource_type}-{part_index}` segment used in download routes.
    pub fn part_segment(&self) -> String {
        format!("{}-{}", self.resource_type, self.part_index)
    }
}

/// An open writer for a single export output part.
///
/// Wraps a boxed async writer plus a line counter. Callers push NDJSON lines
/// with [`write_line`](ExportPartWriter::write_line) and then hand the writer
/// to [`ExportOutputStore::finalize_part`].
pub struct ExportPartWriter {
    /// The underlying async byte sink.
    pub writer: std::pin::Pin<Box<dyn AsyncWrite + Send>>,
    /// Number of lines written so far.
    pub line_count: u64,
    /// Number of bytes written so far.
    pub byte_count: u64,
}

impl ExportPartWriter {
    /// Creates a new part writer over the given async sink.
    pub fn new(writer: std::pin::Pin<Box<dyn AsyncWrite + Send>>) -> Self {
        Self {
            writer,
            line_count: 0,
            byte_count: 0,
        }
    }

    /// Writes one NDJSON line (a trailing newline is appended).
    pub async fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        use tokio::io::AsyncWriteExt;
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.line_count += 1;
        self.byte_count += line.len() as u64 + 1;
        Ok(())
    }

    /// Flushes the underlying writer.
    pub async fn flush(&mut self) -> std::io::Result<()> {
        use tokio::io::AsyncWriteExt;
        self.writer.flush().await
    }
}

impl std::fmt::Debug for ExportPartWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportPartWriter")
            .field("line_count", &self.line_count)
            .field("byte_count", &self.byte_count)
            .finish()
    }
}

/// A finalized, immutable output part as it will appear in the manifest.
#[derive(Debug, Clone)]
pub struct FinalizedPart {
    /// The part's stable key.
    pub key: ExportPartKey,
    /// The resource type contained in the part.
    pub resource_type: String,
    /// Number of resources (lines) in the part.
    pub line_count: u64,
    /// Total byte size of the part.
    pub size_bytes: u64,
}

/// A download URL plus the access posture the manifest should advertise.
#[derive(Debug, Clone)]
pub struct DownloadUrl {
    /// The URL the client should fetch.
    pub url: String,
    /// `true` if the URL requires the kickoff Bearer token (HFS-served);
    /// `false` if it is pre-signed and the client must NOT send a token.
    pub requires_access_token: bool,
}

/// Pluggable backend for bulk export output files.
///
/// Implementations decide where NDJSON output physically lives (local FS, S3,
/// …) and how download URLs are minted. The job-state backend is unaware of
/// this — it stores keys; the output store turns keys into bytes and URLs.
#[async_trait]
pub trait ExportOutputStore: Send + Sync {
    /// Opens an async writer for a new (or re-finalized) output part.
    async fn open_writer(&self, key: &ExportPartKey) -> StorageResult<ExportPartWriter>;

    /// Marks a part as finalized and immutable.
    ///
    /// For object stores this completes the multipart upload; for the local
    /// filesystem this fsyncs and renames `.tmp` → final.
    async fn finalize_part(
        &self,
        key: &ExportPartKey,
        writer: ExportPartWriter,
    ) -> StorageResult<FinalizedPart>;

    /// Produces a download URL for a finalized part.
    async fn download_url(&self, key: &ExportPartKey, ttl: Duration) -> StorageResult<DownloadUrl>;

    /// Opens an async reader over a finalized part (HFS-served download path).
    async fn open_reader(
        &self,
        key: &ExportPartKey,
    ) -> StorageResult<std::pin::Pin<Box<dyn AsyncRead + Send>>>;

    /// Deletes all output parts for a job. Idempotent — a missing job is `Ok`.
    async fn delete_job_outputs(
        &self,
        tenant: &TenantContext,
        job_id: &ExportJobId,
    ) -> StorageResult<()>;
}
