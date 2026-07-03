//! Remote input fetching for Bulk Data **Submit**.
//!
//! As the Data Consumer, HFS fetches the Bulk Export Manifest referenced by the
//! `manifestUrl` kickoff parameter and then streams each NDJSON file it lists.
//! This module defines the [`SubmitInputFetcher`] abstraction the worker uses; the
//! concrete HTTP implementation (built on `reqwest`) lives in the `helios-rest`
//! crate to keep `helios-persistence` free of an HTTP-client dependency.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokio::io::AsyncBufRead;

use crate::core::bulk_export::ExportJobId;
use crate::core::bulk_submit::SubmissionId;
use crate::error::StorageResult;

/// A single file entry in a fetched Bulk Export Manifest (`output` / `deleted`).
#[derive(Debug, Clone, Deserialize)]
pub struct RemoteFile {
    /// FHIR resource type contained in the file (absent for `deleted` bundles).
    #[serde(rename = "type", default)]
    pub resource_type: Option<String>,
    /// URL to download the file from.
    pub url: String,
    /// Declared resource count, if provided.
    #[serde(default)]
    pub count: Option<u64>,
}

/// A Bulk Export Manifest fetched from a Data Provider's `manifestUrl`.
///
/// Parsed leniently (all fields default) so that provider-specific extensions or
/// omitted optional fields do not fail the fetch.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RemoteManifest {
    /// Whether the referenced files require an OAuth bearer token to download.
    #[serde(rename = "requiresAccessToken", default)]
    pub requires_access_token: bool,
    /// Files containing resources to ingest (create/update).
    #[serde(default)]
    pub output: Vec<RemoteFile>,
    /// Files containing transaction Bundles describing resources to delete.
    #[serde(default)]
    pub deleted: Vec<RemoteFile>,
}

/// Optional hook for acquiring an OAuth bearer token to fetch protected files.
///
/// Wired by the HFS binary in Phase 5 to a SMART Backend Services client
/// (`client_credentials` + `private_key_jwt`). The token is requested with **read
/// scopes** for the manifest's resource types — never the `system/bulk-submit`
/// operation scope (see the spec's file-retrieval authorization rule).
#[async_trait]
pub trait FileTokenProvider: Send + Sync {
    /// Returns a bearer token (without the `Bearer ` prefix) for the given OAuth
    /// metadata endpoints and scope, or `None` if a token cannot be obtained.
    async fn token(&self, oauth_metadata_urls: &[String], scope: &str) -> Option<String>;
}

/// Fetches the remote manifest and streams the NDJSON files it references.
#[async_trait]
pub trait SubmitInputFetcher: Send + Sync {
    /// Fetches and parses the Bulk Export Manifest at `url`, applying the
    /// provider-supplied request headers and (when required) an acquired token.
    async fn fetch_manifest(
        &self,
        url: &str,
        request_headers: &[(String, String)],
        oauth_metadata_urls: &[String],
    ) -> StorageResult<RemoteManifest>;

    /// Opens a streaming, line-buffered reader over the NDJSON file at `url`.
    ///
    /// Implementations apply `request_headers`, request `gzip` via `Accept-Encoding`
    /// and transparently decompress, and — when `requires_access_token` is true —
    /// attach a read-scoped bearer token. `encryption_key` carries the
    /// `fileEncryptionKey` descriptor for JWE-encrypted files (Phase 5).
    async fn open_file_stream(
        &self,
        url: &str,
        request_headers: &[(String, String)],
        requires_access_token: bool,
        oauth_metadata_urls: &[String],
        encryption_key: Option<&Value>,
    ) -> StorageResult<Box<dyn AsyncBufRead + Send + Unpin>>;
}

/// Maps a submission to the stable [`ExportJobId`] used as the output-store key
/// for its status-manifest artifacts (so artifacts are grouped per submission and
/// cleanable via `ExportOutputStore::delete_job_outputs`).
///
/// The result is deterministic and filesystem/object-key safe regardless of the
/// (arbitrary) submitter / submission-id strings.
pub fn submission_output_job_id(id: &SubmissionId) -> ExportJobId {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    id.submitter.hash(&mut hasher);
    0u8.hash(&mut hasher); // separator to avoid ("ab","c") == ("a","bc")
    id.submission_id.hash(&mut hasher);
    ExportJobId::from_string(format!("submit-{:016x}", hasher.finish()))
}
