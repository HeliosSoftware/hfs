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
    ///
    /// `encryption_key` carries the `fileEncryptionKey` descriptor: the submit
    /// spec has the Data Provider encrypt the manifest as well as the files it
    /// lists, so implementations decrypt the response before parsing it.
    async fn fetch_manifest(
        &self,
        url: &str,
        request_headers: &[(String, String)],
        oauth_metadata_urls: &[String],
        encryption_key: Option<&Value>,
    ) -> StorageResult<RemoteManifest>;

    /// Opens a streaming, line-buffered reader over the NDJSON file at `url`,
    /// with the file's total size in bytes when the source advertises one
    /// (`Content-Length`, or the decrypted length of a buffered JWE file).
    /// The size is what turns the status endpoint's progress into a real
    /// percentage; `None` degrades to count-only progress.
    ///
    /// Implementations apply `request_headers`, request `gzip` via `Accept-Encoding`
    /// and transparently decompress, and — when `requires_access_token` is true —
    /// attach a read-scoped bearer token. `encryption_key` carries the
    /// `fileEncryptionKey` descriptor for JWE-encrypted files.
    async fn open_file_stream(
        &self,
        url: &str,
        request_headers: &[(String, String)],
        requires_access_token: bool,
        oauth_metadata_urls: &[String],
        encryption_key: Option<&Value>,
    ) -> StorageResult<(Box<dyn AsyncBufRead + Send + Unpin>, Option<u64>)>;

    /// Returns the advertised size in bytes of the file at `url` without
    /// opening its body, or `None` when the source cannot cheaply say (no
    /// HEAD support, no `Content-Length`, or a JWE file whose decrypted
    /// length differs from the wire length).
    ///
    /// The worker calls this for every manifest file up front so the byte
    /// progress denominator is complete before ingestion starts — learned
    /// lazily per file, each newly opened file yanks the percentage
    /// backwards (#874). Best-effort: any `None` (the default) falls back
    /// to lazy accumulation.
    async fn file_size(
        &self,
        _url: &str,
        _request_headers: &[(String, String)],
        _requires_access_token: bool,
        _oauth_metadata_urls: &[String],
    ) -> StorageResult<Option<u64>> {
        Ok(None)
    }
}

/// Returns `url` with everything that can carry a credential removed, for
/// error messages, logs, and manifest `error` artifacts (#1127).
///
/// A presigned input URL carries its signature in the query string, and a URL
/// can carry `user:password@` in its authority. Both are replaced by a
/// `[redacted]` marker so the message still shows that something was there;
/// scheme, host, port, and path stay, which is what an operator needs to find
/// the file. Not a URL parser: anything without `://` keeps its path part.
pub fn redact_url(url: &str) -> String {
    let (base, tail) = match url.find(['?', '#']) {
        Some(cut) if url[cut..].starts_with('?') => (&url[..cut], "?[redacted]"),
        Some(cut) => (&url[..cut], "#[redacted]"),
        None => (url, ""),
    };
    let mut out = String::with_capacity(base.len() + tail.len());
    match base.find("://") {
        Some(scheme_end) => {
            let authority_start = scheme_end + 3;
            let authority_end = base[authority_start..]
                .find('/')
                .map_or(base.len(), |i| authority_start + i);
            match base[authority_start..authority_end].rfind('@') {
                Some(at) => {
                    out.push_str(&base[..authority_start]);
                    out.push_str("[redacted]@");
                    out.push_str(&base[authority_start + at + 1..]);
                }
                None => out.push_str(base),
            }
        }
        None => out.push_str(base),
    }
    out.push_str(tail);
    out
}

/// Punctuation that ends a sentence rather than a URL. A message reads
/// `reading file {url}: {cause}`, and the `:` belongs to the message: without
/// this, redacting the URL swallowed it and the cause ran into the URL
/// (#1127). Everything after the `?` is replaced regardless, so keeping these
/// characters cannot leak a query.
const URL_TRAILING_PUNCTUATION: [char; 4] = [':', ',', ';', '.'];

/// Applies [`redact_url`] to every `http://` / `https://` URL embedded in
/// free text — an error message from an HTTP client, for instance, which
/// quotes the URL it failed on. A URL ends at whitespace, a quote, `<`, `>`
/// or `)`, and trailing [`URL_TRAILING_PUNCTUATION`] is left in the text.
///
/// Idempotent: re-redacting a message that already carries `?[redacted]`
/// leaves it, and the punctuation after it, unchanged — the worker redacts a
/// cause the fetcher had already redacted.
pub fn redact_urls_in(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let next = [rest.find("http://"), rest.find("https://")]
            .into_iter()
            .flatten()
            .min();
        let Some(start) = next else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..start]);
        let candidate = &rest[start..];
        let end = candidate
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')'))
            .unwrap_or(candidate.len());
        let url = candidate[..end].trim_end_matches(URL_TRAILING_PUNCTUATION);
        out.push_str(&redact_url(url));
        out.push_str(&candidate[url.len()..end]);
        rest = &candidate[end..];
    }
}

/// Renders an error together with its whole `source()` chain, redacting any
/// URL in it (#1127).
///
/// `Display` on an HTTP client error usually stops at the outermost layer —
/// `error decoding response body` — and drops the cause that matters
/// (`connection reset by peer`). Each layer is joined with `: `; a layer
/// whose text the previous one already contains is skipped, since some
/// errors repeat their source in their own message.
pub fn error_chain(err: &dyn std::error::Error) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut current: Option<&dyn std::error::Error> = Some(err);
    while let Some(e) = current {
        let text = e.to_string();
        if !parts
            .last()
            .is_some_and(|previous| previous.contains(&text))
        {
            parts.push(text);
        }
        current = e.source();
    }
    redact_urls_in(&parts.join(": "))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_url_strips_query_fragment_and_userinfo() {
        assert_eq!(
            redact_url(
                "https://bucket.s3.amazonaws.com/a/Patient.ndjson?X-Amz-Signature=abc&X-Amz-Expires=60"
            ),
            "https://bucket.s3.amazonaws.com/a/Patient.ndjson?[redacted]"
        );
        assert_eq!(
            redact_url("https://host/file.ndjson#token=abc"),
            "https://host/file.ndjson#[redacted]"
        );
        assert_eq!(
            redact_url("https://user:secret@host:8443/f.ndjson?sig=1"),
            "https://[redacted]@host:8443/f.ndjson?[redacted]"
        );
        assert_eq!(
            redact_url("http://host/path@with-at"),
            "http://host/path@with-at"
        );
        assert_eq!(
            redact_url("http://127.0.0.1:18080/Patient.ndjson"),
            "http://127.0.0.1:18080/Patient.ndjson"
        );
        assert_eq!(redact_url("not a url?x=1"), "not a url?[redacted]");
    }

    #[test]
    fn redact_urls_in_rewrites_every_embedded_url() {
        assert_eq!(
            redact_urls_in(
                "error sending request for url (https://h/f.ndjson?sig=abc): also http://u:p@g/x?y"
            ),
            "error sending request for url (https://h/f.ndjson?[redacted]): also http://[redacted]@g/x?[redacted]"
        );
        assert_eq!(redact_urls_in("no urls here"), "no urls here");
    }

    /// #1127: the `:` that separates a URL from the cause after it is part of
    /// the message, not of the URL, and redacting twice changes nothing.
    #[test]
    fn redact_urls_in_keeps_the_punctuation_after_a_url() {
        let once = redact_urls_in("reading file http://h/f.ndjson?sig=SECRET: connection reset");
        assert_eq!(
            once,
            "reading file http://h/f.ndjson?[redacted]: connection reset"
        );
        assert_eq!(redact_urls_in(&once), once);
        assert_eq!(
            redact_urls_in("fetched http://h/a?k=1, then http://h/b?k=2."),
            "fetched http://h/a?[redacted], then http://h/b?[redacted]."
        );
    }

    #[derive(Debug)]
    struct Layer(&'static str, Option<Box<Layer>>);
    impl std::fmt::Display for Layer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }
    impl std::error::Error for Layer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.1
                .as_deref()
                .map(|l| l as &(dyn std::error::Error + 'static))
        }
    }

    #[test]
    fn error_chain_reaches_the_root_cause() {
        let err = Layer(
            "error decoding response body for url (https://h/f?sig=1)",
            Some(Box::new(Layer(
                "error reading a body from connection",
                Some(Box::new(Layer("connection reset by peer", None))),
            ))),
        );
        assert_eq!(
            error_chain(&err),
            "error decoding response body for url (https://h/f?[redacted]): \
             error reading a body from connection: connection reset by peer"
        );
    }

    #[test]
    fn error_chain_skips_a_layer_its_parent_already_quotes() {
        let err = Layer(
            "reading file: connection reset",
            Some(Box::new(Layer("connection reset", None))),
        );
        assert_eq!(error_chain(&err), "reading file: connection reset");
    }
}
