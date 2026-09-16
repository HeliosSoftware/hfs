//! Concrete HTTP implementation of [`SubmitInputFetcher`].
//!
//! Lives in `helios-rest` (which already depends on `reqwest`) rather than
//! `helios-persistence`, keeping the persistence crate free of an HTTP client.
//! Fetches the remote Bulk Export Manifest and its NDJSON files, applying
//! provider-supplied request headers, `gzip`, and — when files require an access
//! token — a read-scoped bearer obtained via an optional [`FileTokenProvider`].
//!
//! NDJSON bodies are streamed to the ingestion engine rather than buffered, so a
//! manifest referencing multi-gigabyte files costs a fixed amount of memory per
//! concurrent fetch. The one exception is `fileEncryptionKey` (JWE) files, whose
//! format forces whole-body buffering — see [`HttpSubmitInputFetcher::open_file_stream`].
//!
//! When the submission carries a `fileEncryptionKey`, the manifest and every
//! file are decrypted with the [`crate::jwe`] module (built unconditionally).
//!
//! # Broken bodies (#1127)
//!
//! A file body that breaks mid-stream (connection reset, read timeout) is
//! resumed rather than abandoned: the fetcher counts the bytes it has handed
//! on and re-requests the rest with `Range: bytes=<consumed>-`, guarded by
//! `If-Range` when the first response carried a strong `ETag` or a
//! `Last-Modified`, so a file that changed in between is never spliced. A
//! server that ignores `Range` answers `200` with the whole file (and the
//! same validator), and the already-delivered prefix is skipped. A body that
//! ends cleanly short of the file's length (a short `206`, a resent file
//! shorter than the offset) is resumed the same way. Resumes are bounded
//! ([`MAX_BODY_RETRIES`] consecutive failures without progress, exponential
//! back-off); past that, or when the file
//! cannot be resumed safely, the read fails with the whole error chain, and
//! every attempt is logged at `warn`. JWE files, buffered whole, are
//! re-fetched whole instead.
//!
//! Every URL that reaches an error message or a log line goes through
//! [`redact_url`]: a presigned input URL carries its signature in the query
//! string, and those messages end up in the manifest's `error` artifact.

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use helios_persistence::core::bulk_submit_input::{error_chain, redact_url};
use helios_persistence::core::{FileTokenProvider, RemoteManifest, SubmitInputFetcher};
use helios_persistence::error::{BackendError, StorageError, StorageResult};
use reqwest::header::{self, HeaderMap};
use serde_json::Value;
use tokio::io::AsyncBufRead;
use tokio_util::io::StreamReader;

use crate::jwe::{self, DecryptionKeys, PrivateKey};

/// The default `fileEncryptionKey.coding` code (submit spec: "If omitted,
/// defaults to a system of `…/file-encryption-type` and code of `jwe`").
const FILE_ENCRYPTION_TYPE_JWE: &str = "jwe";

/// How long establishing a connection to the provider may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Default idle time allowed between two reads of a response before the read
/// fails (`HFS_BULK_SUBMIT_FETCH_READ_TIMEOUT`). A stalled body then surfaces
/// as a body error and is resumed like a reset connection.
pub const DEFAULT_FETCH_READ_TIMEOUT: Duration = Duration::from_secs(60);

/// How many times one file may be re-requested after its transfer broke.
pub const MAX_BODY_RETRIES: u32 = 3;

/// Delay before the first re-request; each further one doubles it (1 s, 2 s, 4 s).
const FIRST_RETRY_DELAY: Duration = Duration::from_secs(1);

/// Bounds on re-requesting a file whose transfer broke.
#[derive(Debug, Clone, Copy)]
struct RetryPolicy {
    max_retries: u32,
    first_delay: Duration,
}

impl RetryPolicy {
    /// Delay before retry number `attempt` (1-based).
    fn delay(&self, attempt: u32) -> Duration {
        self.first_delay
            .saturating_mul(1u32 << attempt.saturating_sub(1).min(16))
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: MAX_BODY_RETRIES,
            first_delay: FIRST_RETRY_DELAY,
        }
    }
}

/// Fetches submit input over HTTP(S) using `reqwest`.
pub struct HttpSubmitInputFetcher {
    /// Decodes `gzip` transparently: manifests and JWE files, read whole.
    client: reqwest::Client,
    /// No transparent decoding: streamed NDJSON bodies, whose wire offset a
    /// `Range` resume needs, and `HEAD` size probes, which must see the
    /// identity `Content-Length`.
    stream_client: reqwest::Client,
    /// Optional token source for `requiresAccessToken=true` provider files.
    token_provider: Option<Arc<dyn FileTokenProvider>>,
    /// Read scope requested for the outbound file-retrieval token (e.g. `system/*.rs`).
    outbound_scope: String,
    /// Local private keys used when `fileEncryptionKey.value` (or a file's JWE)
    /// addresses HFS asymmetrically — `RSA-OAEP*` / `ECDH-ES*`.
    decryption_keys: Vec<PrivateKey>,
    retry: RetryPolicy,
}

/// Builds the decoding and the non-decoding client with the same timeouts.
fn build_clients(read_timeout: Duration) -> (reqwest::Client, reqwest::Client) {
    let base = || {
        reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(read_timeout)
    };
    // `Client::new()` panics the same way when the TLS backend cannot start.
    let client = base()
        .build()
        .expect("bulk-submit HTTP client configuration is valid");
    let stream_client = base()
        .no_gzip()
        .build()
        .expect("bulk-submit HTTP client configuration is valid");
    (client, stream_client)
}

impl HttpSubmitInputFetcher {
    /// Creates a fetcher with the given optional outbound token provider and scope.
    pub fn new(token_provider: Option<Arc<dyn FileTokenProvider>>, outbound_scope: String) -> Self {
        let (client, stream_client) = build_clients(DEFAULT_FETCH_READ_TIMEOUT);
        Self {
            client,
            stream_client,
            token_provider,
            outbound_scope,
            decryption_keys: Vec::new(),
            retry: RetryPolicy::default(),
        }
    }

    /// Sets the idle time allowed between two reads of a response
    /// (`HFS_BULK_SUBMIT_FETCH_READ_TIMEOUT`); the default is
    /// [`DEFAULT_FETCH_READ_TIMEOUT`].
    pub fn with_read_timeout(mut self, read_timeout: Duration) -> Self {
        let (client, stream_client) = build_clients(read_timeout);
        self.client = client;
        self.stream_client = stream_client;
        self
    }

    /// Shortens the retry schedule so tests do not sleep for seconds.
    #[cfg(test)]
    fn with_retry_policy(mut self, max_retries: u32, first_delay: Duration) -> Self {
        self.retry = RetryPolicy {
            max_retries,
            first_delay,
        };
        self
    }

    /// Adds the private keys used to unwrap asymmetrically addressed JWEs
    /// (`HFS_BULK_SUBMIT_DECRYPTION_KEY`).
    pub fn with_decryption_keys(mut self, keys: Vec<PrivateKey>) -> Self {
        self.decryption_keys = keys;
        self
    }

    /// Everything needed to (re-)issue the GET for one input file, owned so a
    /// resuming body stream can outlive the `open_file_stream` call.
    fn file_request(
        &self,
        client: &reqwest::Client,
        url: &str,
        request_headers: &[(String, String)],
        with_token: bool,
        oauth_metadata_urls: &[String],
    ) -> FileRequest {
        FileRequest {
            client: client.clone(),
            token_provider: self.token_provider.clone(),
            outbound_scope: self.outbound_scope.clone(),
            url: url.to_string(),
            shown_url: redact_url(url),
            request_headers: request_headers.to_vec(),
            with_token,
            oauth_metadata_urls: oauth_metadata_urls.to_vec(),
        }
    }

    fn err(msg: impl Into<String>) -> StorageError {
        StorageError::Backend(BackendError::Internal {
            backend_name: "bulk-submit-fetch".to_string(),
            message: msg.into(),
            source: None,
        })
    }

    /// Builds a request with provider headers and (optionally) a bearer token.
    async fn build_get(
        &self,
        url: &str,
        request_headers: &[(String, String)],
        with_token: bool,
        oauth_metadata_urls: &[String],
    ) -> StorageResult<reqwest::RequestBuilder> {
        self.file_request(
            &self.client,
            url,
            request_headers,
            with_token,
            oauth_metadata_urls,
        )
        .get(ACCEPT_GZIP, &[])
        .await
    }

    /// Reads a named sub-part of the `fileEncryptionKey` parameter.
    ///
    /// Accepts both the spec shape (`part[]` with `name`/`value[x]`) and the
    /// flattened shape some producers emit (`{"value": "…"}`).
    fn key_part<'a>(key: &'a Value, name: &str) -> Option<&'a Value> {
        key.get("part")
            .and_then(|p| p.as_array())
            .and_then(|arr| {
                arr.iter()
                    .find(|c| c.get("name").and_then(|n| n.as_str()) == Some(name))
            })
            .and_then(|c| {
                c.get("valueString")
                    .or_else(|| c.get("valueCoding"))
                    .or_else(|| c.get("valueCode"))
            })
            .or_else(|| key.get(name))
    }

    /// Turns the `fileEncryptionKey` descriptor into a usable key set, or
    /// `Ok(None)` when the submission is unencrypted.
    fn resolve_keys(
        &self,
        encryption_key: Option<&Value>,
    ) -> StorageResult<Option<DecryptionKeys>> {
        let Some(key) = encryption_key else {
            return Ok(None);
        };

        // `coding` defaults to `jwe`; anything else is a scheme we do not know.
        let code = Self::key_part(key, "coding").and_then(|c| {
            c.get("code")
                .and_then(|v| v.as_str())
                .or_else(|| c.as_str())
                .map(str::to_string)
        });
        if let Some(code) = &code {
            if code != FILE_ENCRYPTION_TYPE_JWE {
                return Err(Self::err(format!(
                    "unsupported fileEncryptionKey.coding.code '{code}' (only \
                     '{FILE_ENCRYPTION_TYPE_JWE}' is defined)"
                )));
            }
        }

        let value = Self::key_part(key, "value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Self::err("fileEncryptionKey.value is required"))?;

        let private = self.decryption_keys.clone();
        // The spec calls `value` "the JSON Web Encryption structure to deliver a
        // Content Encryption Key". When it *is* a JWE, unwrap it with a locally
        // configured private key; otherwise it carries the CEK directly.
        let cek = if jwe::looks_like_jwe(value.as_bytes()) {
            let payload = jwe::decrypt(value.as_bytes(), &DecryptionKeys::private(private.clone()))
                .map_err(|e| Self::err(format!("unwrapping fileEncryptionKey.value: {e}")))?;
            interpret_cek(&payload).ok_or_else(|| {
                Self::err("fileEncryptionKey.value unwrapped to unusable key material")
            })?
        } else {
            interpret_cek(value.as_bytes()).ok_or_else(|| {
                Self::err(
                    "fileEncryptionKey.value is neither a JWE, an `oct` JWK, nor \
                     base64url-encoded key material",
                )
            })?
        };

        Ok(Some(DecryptionKeys {
            shared: Some(cek),
            private,
        }))
    }

    /// Decrypts a downloaded file when the submission is encrypted.
    ///
    /// A `fileEncryptionKey` means the provider SHALL have encrypted the file,
    /// so a non-JWE payload is an error rather than a silent plaintext accept.
    fn decrypt_file(
        &self,
        bytes: Vec<u8>,
        keys: Option<&DecryptionKeys>,
    ) -> StorageResult<Vec<u8>> {
        let Some(keys) = keys else {
            return Ok(bytes);
        };
        if !jwe::looks_like_jwe(&bytes) {
            return Err(Self::err(
                "fileEncryptionKey was supplied but the file is not a JWE",
            ));
        }
        jwe::decrypt(&bytes, keys).map_err(Self::err)
    }

    /// Downloads a whole file, re-fetching it from the start when the
    /// transfer breaks (bounded by the retry policy). Used for JWE files,
    /// which are buffered anyway, so there is no offset to resume from.
    async fn fetch_whole_file(&self, request: &FileRequest) -> StorageResult<Vec<u8>> {
        let shown = &request.shown_url;
        let mut retries = 0u32;
        loop {
            let cause = match request.send(ACCEPT_GZIP, &[]).await {
                Err(Retry::Fatal(message)) => return Err(Self::err(message)),
                // The first GET failing to connect is final, as for a streamed
                // file; only a transfer that already started is retried.
                Err(Retry::Transient(message)) if retries == 0 => {
                    return Err(Self::err(message));
                }
                Err(Retry::Transient(message)) => message,
                // A server error on a re-fetch is as transient as the break was.
                Ok(resp) if resp.status().is_server_error() && retries > 0 => {
                    format!("file GET {shown} returned HTTP {} on retry", resp.status())
                }
                Ok(resp) if !resp.status().is_success() => {
                    return Err(Self::err(format!(
                        "file GET {shown} returned HTTP {}",
                        resp.status()
                    )));
                }
                Ok(resp) => match resp.bytes().await {
                    Ok(bytes) => return Ok(bytes.to_vec()),
                    Err(e) => format!("reading file {shown}: {}", error_chain(&e)),
                },
            };
            if retries >= self.retry.max_retries {
                tracing::warn!(
                    url = %shown,
                    retries,
                    error = %cause,
                    "bulk-submit encrypted file could not be downloaded whole"
                );
                return Err(Self::err(format!(
                    "{cause} (gave up after {retries} retries)"
                )));
            }
            retries += 1;
            tracing::warn!(
                url = %shown,
                attempt = retries,
                max_attempts = self.retry.max_retries,
                error = %cause,
                "bulk-submit encrypted file transfer failed; re-fetching the whole file"
            );
            tokio::time::sleep(self.retry.delay(retries)).await;
        }
    }
}

/// Interprets raw key material as an AES key.
///
/// Accepts an `oct` JWK, a base64url-encoded key, or (for a CEK recovered from
/// a JWE payload) raw bytes of a valid AES/AES-CBC-HMAC key length.
fn interpret_cek(bytes: &[u8]) -> Option<Vec<u8>> {
    const KEY_LENGTHS: [usize; 5] = [16, 24, 32, 48, 64];

    if let Ok(text) = std::str::from_utf8(bytes) {
        let text = text.trim();
        if text.starts_with('{') {
            // An `oct` JWK: {"kty":"oct","k":"<base64url>"}.
            if let Ok(jwk) = serde_json::from_str::<Value>(text) {
                if let Some(k) = jwk.get("k").and_then(|v| v.as_str()) {
                    return base64::Engine::decode(
                        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                        k.trim_end_matches('='),
                    )
                    .ok();
                }
            }
            return None;
        }
        if let Ok(decoded) = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            text.trim_end_matches('='),
        ) {
            if KEY_LENGTHS.contains(&decoded.len()) {
                return Some(decoded);
            }
        }
    }
    // Raw bytes — only plausible for a CEK recovered from a JWE payload.
    KEY_LENGTHS.contains(&bytes.len()).then(|| bytes.to_vec())
}

/// `Accept-Encoding` of a first request: the provider may compress.
const ACCEPT_GZIP: &str = "gzip";
/// `Accept-Encoding` of a ranged resume: offsets are only meaningful on the
/// identity representation.
const ACCEPT_IDENTITY: &str = "identity";

/// An owned description of one input-file GET, re-issuable for resumes.
#[derive(Clone)]
struct FileRequest {
    client: reqwest::Client,
    token_provider: Option<Arc<dyn FileTokenProvider>>,
    outbound_scope: String,
    url: String,
    /// `url` with its query string, fragment and credentials redacted: the
    /// only form that may appear in an error message or a log line.
    shown_url: String,
    request_headers: Vec<(String, String)>,
    with_token: bool,
    oauth_metadata_urls: Vec<String>,
}

impl FileRequest {
    /// Builds the GET with provider headers and (optionally) a fresh bearer
    /// token — a resume hours into a transfer must not reuse an expired one.
    async fn get(
        &self,
        accept_encoding: &str,
        extra_headers: &[(header::HeaderName, String)],
    ) -> StorageResult<reqwest::RequestBuilder> {
        let shown = &self.shown_url;
        let mut rb = self
            .client
            .get(&self.url)
            .header(header::ACCEPT_ENCODING, accept_encoding);
        for (name, value) in extra_headers {
            rb = rb.header(name.clone(), value.as_str());
        }
        if self.with_token {
            let Some(provider) = &self.token_provider else {
                return Err(HttpSubmitInputFetcher::err(format!(
                    "{shown} requires an access token but no outbound auth is configured"
                )));
            };
            let Some(token) = provider
                .token(&self.oauth_metadata_urls, &self.outbound_scope)
                .await
            else {
                return Err(HttpSubmitInputFetcher::err(format!(
                    "{shown} requires an access token but none could be obtained"
                )));
            };
            rb = rb.bearer_auth(token);
        }
        // Provider-supplied headers take precedence (applied last).
        for (name, value) in &self.request_headers {
            rb = rb.header(name.as_str(), value.as_str());
        }
        Ok(rb)
    }

    /// Sends the GET. A transport failure is [`Retry::Transient`]; a request
    /// that cannot even be built (no token) is [`Retry::Fatal`].
    async fn send(
        &self,
        accept_encoding: &str,
        extra_headers: &[(header::HeaderName, String)],
    ) -> Result<reqwest::Response, Retry> {
        let rb = self
            .get(accept_encoding, extra_headers)
            .await
            .map_err(|e| Retry::Fatal(e.to_string()))?;
        rb.send().await.map_err(|e| {
            Retry::Transient(format!(
                "file GET {} failed: {}",
                self.shown_url,
                error_chain(&e)
            ))
        })
    }
}

/// Why one attempt at a file failed.
#[derive(Debug)]
enum Retry {
    /// Worth another attempt: the transfer broke, the connection failed.
    Transient(String),
    /// Another attempt cannot help: an HTTP error, a changed file.
    Fatal(String),
}

/// How a file body is encoded on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyEncoding {
    Identity,
    Gzip,
}

/// Reads `Content-Encoding`. Only what the fetcher asks for (`gzip`) is
/// accepted; anything else would reach the NDJSON parser as garbage.
fn body_encoding(headers: &HeaderMap) -> Result<BodyEncoding, String> {
    let Some(value) = headers.get(header::CONTENT_ENCODING) else {
        return Ok(BodyEncoding::Identity);
    };
    let value = value
        .to_str()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match value.as_str() {
        "" | "identity" => Ok(BodyEncoding::Identity),
        "gzip" | "x-gzip" => Ok(BodyEncoding::Gzip),
        other => Err(format!("unsupported Content-Encoding '{other}'")),
    }
}

/// The validator an `If-Range` may carry: a strong `ETag` (weak ones are not
/// allowed there), else `Last-Modified` — with the header it came from, so a
/// later response can be compared on the same one.
fn if_range_validator(headers: &HeaderMap) -> Option<(header::HeaderName, String)> {
    let etag = header_text(headers, &header::ETAG).filter(|etag| !etag.starts_with("W/"));
    match etag {
        Some(etag) => Some((header::ETAG, etag.to_string())),
        None => header_text(headers, &header::LAST_MODIFIED)
            .map(|modified| (header::LAST_MODIFIED, modified.to_string())),
    }
}

/// A header's trimmed, non-empty text value.
fn header_text<'a>(headers: &'a HeaderMap, name: &header::HeaderName) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// Splits `Content-Range: bytes <range>/<complete length or *>`.
fn content_range_parts(headers: &HeaderMap) -> Option<(&str, Option<u64>)> {
    let value = headers.get(header::CONTENT_RANGE)?.to_str().ok()?;
    let (range, complete) = value.trim().strip_prefix("bytes ")?.split_once('/')?;
    Some((range.trim(), complete.trim().parse().ok()))
}

/// Parses `Content-Range: bytes <first>-<last>/<complete length or *>` into
/// `(first, last, complete length)`; an inverted or out-of-length range is
/// not usable.
fn parse_content_range(headers: &HeaderMap) -> Option<(u64, u64, Option<u64>)> {
    let (range, complete) = content_range_parts(headers)?;
    let (first, last) = range.split_once('-')?;
    let first: u64 = first.trim().parse().ok()?;
    let last: u64 = last.trim().parse().ok()?;
    if last < first || complete.is_some_and(|complete| last >= complete) {
        return None;
    }
    Some((first, last, complete))
}

/// Parses the `Content-Range: bytes */<complete length>` of a `416`.
fn parse_unsatisfied_range(headers: &HeaderMap) -> Option<u64> {
    let (range, complete) = content_range_parts(headers)?;
    if range != "*" {
        return None;
    }
    complete
}

type BodyStream = Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>;

/// A file body that re-requests what is left when the transfer breaks.
///
/// It sits below any content decoding, so `delivered` counts wire bytes —
/// exactly the offset a `Range` request names.
struct ResumableBody {
    request: FileRequest,
    body: Option<BodyStream>,
    encoding: BodyEncoding,
    /// Wire bytes handed on so far.
    delivered: u64,
    /// Bytes still to drop from the current body: a server that ignored
    /// `Range` resent the file from its start.
    skip: u64,
    /// `If-Range` validator taken from the first response, with its header.
    validator: Option<(header::HeaderName, String)>,
    /// Complete length of the file, once a response said.
    complete_length: Option<u64>,
    /// Offset at which the current body ends, when its response said: the
    /// last byte of a `206` range plus one, or the length of a whole file.
    body_end: Option<u64>,
    /// Consecutive failed attempts since the body last got further.
    retries: u32,
    /// `delivered` when `retries` was last reset: only bytes past it are
    /// progress, so a resent prefix that breaks again does not count.
    progress_mark: u64,
    policy: RetryPolicy,
}

impl ResumableBody {
    fn new(
        request: FileRequest,
        first: reqwest::Response,
        encoding: BodyEncoding,
        policy: RetryPolicy,
    ) -> Self {
        let (validator, complete_length) = match encoding {
            BodyEncoding::Identity => (if_range_validator(first.headers()), first.content_length()),
            BodyEncoding::Gzip => (None, None),
        };
        Self {
            request,
            body: Some(Box::pin(first.bytes_stream())),
            encoding,
            delivered: 0,
            skip: 0,
            validator,
            complete_length,
            body_end: complete_length,
            retries: 0,
            progress_mark: 0,
            policy,
        }
    }

    /// The body as a stream of wire chunks; it ends after the first error.
    fn into_stream(self) -> impl Stream<Item = std::io::Result<Bytes>> + Send + 'static {
        futures::stream::unfold(Some(self), |state| async move {
            let mut body = state?;
            match body.next_chunk().await {
                Ok(Some(chunk)) => Some((Ok(chunk), Some(body))),
                Ok(None) => None,
                Err(e) => Some((Err(e), None)),
            }
        })
    }

    async fn next_chunk(&mut self) -> std::io::Result<Option<Bytes>> {
        loop {
            let Some(body) = self.body.as_mut() else {
                return Ok(None);
            };
            let cause = match body.next().await {
                // hyper reports a body cut short of its Content-Length or its
                // final chunk as an error, but a well-framed body can still
                // stop short of the file: a short 206, a shrunken resend.
                None => match self.missing_tail() {
                    None => {
                        self.body = None;
                        return Ok(None);
                    }
                    Some(cause) => cause,
                },
                Some(Ok(mut chunk)) => {
                    if self.skip > 0 {
                        let dropped = self.skip.min(chunk.len() as u64);
                        chunk = chunk.slice(dropped as usize..);
                        self.skip -= dropped;
                    }
                    if chunk.is_empty() {
                        continue;
                    }
                    self.delivered += chunk.len() as u64;
                    return Ok(Some(chunk));
                }
                Some(Err(e)) => format!(
                    "reading file {}: {}",
                    self.request.shown_url,
                    error_chain(&e)
                ),
            };
            self.body = None;
            let body = self.resume(cause).await?;
            self.body = Some(body);
        }
    }

    /// Why a body that ended cleanly is still not the whole file, or `None`
    /// when it is (as far as any response said).
    fn missing_tail(&self) -> Option<String> {
        let shown = &self.request.shown_url;
        if self.skip > 0 {
            return Some(format!(
                "file {shown} was resent shorter than the {} bytes already delivered",
                self.delivered
            ));
        }
        let end = self.complete_length.or(self.body_end)?;
        (self.delivered < end).then(|| {
            format!(
                "file {shown} body ended at byte {} of {end}",
                self.delivered
            )
        })
    }

    /// Re-requests the rest of the file, retrying transient failures within
    /// the policy. Returns the new body, or the error that ends the read.
    async fn resume(&mut self, mut cause: String) -> std::io::Result<BodyStream> {
        let shown = self.request.shown_url.clone();
        // The budget is for consecutive failures: a body that got further
        // since the last reset earns a fresh one. `delivered` only grows
        // toward a finite file, so this still ends.
        if self.delivered > self.progress_mark {
            self.progress_mark = self.delivered;
            self.retries = 0;
        }
        loop {
            if self.encoding == BodyEncoding::Gzip && self.delivered > 0 {
                return Err(self.give_up(format!(
                    "{cause} (the body is gzip-encoded, so it cannot be resumed with Range \
                     after {} bytes)",
                    self.delivered
                )));
            }
            if self.retries >= self.policy.max_retries {
                return Err(self.give_up(format!(
                    "{cause} (gave up after {} bytes and {} retries)",
                    self.delivered, self.retries
                )));
            }
            self.retries += 1;
            tracing::warn!(
                url = %shown,
                delivered_bytes = self.delivered,
                attempt = self.retries,
                max_attempts = self.policy.max_retries,
                error = %cause,
                "bulk-submit file stream failed mid-body; re-requesting the rest"
            );
            tokio::time::sleep(self.policy.delay(self.retries)).await;
            match self.reopen().await {
                Ok(body) => return Ok(body),
                Err(Retry::Transient(message)) => cause = message,
                Err(Retry::Fatal(message)) => return Err(self.give_up(message)),
            }
        }
    }

    fn give_up(&self, message: String) -> std::io::Error {
        tracing::warn!(
            url = %self.request.shown_url,
            delivered_bytes = self.delivered,
            retries = self.retries,
            error = %message,
            "bulk-submit file stream failed mid-body; the file cannot be completed"
        );
        std::io::Error::other(message)
    }

    /// Records a complete length a response stated, failing when it
    /// contradicts an earlier one.
    fn check_length(&mut self, length: u64) -> Result<(), Retry> {
        match self.complete_length {
            Some(expected) if expected != length => Err(Retry::Fatal(format!(
                "file {} changed size between requests ({expected} -> {length} bytes)",
                self.request.shown_url
            ))),
            _ => {
                self.complete_length = Some(length);
                Ok(())
            }
        }
    }

    /// One re-request. A gzip body is only ever re-fetched whole, before any
    /// byte of it was delivered; an identity body resumes at `delivered`.
    async fn reopen(&mut self) -> Result<BodyStream, Retry> {
        let shown = self.request.shown_url.clone();
        if self.encoding == BodyEncoding::Gzip {
            let resp = self.request.send(ACCEPT_GZIP, &[]).await?;
            let status = resp.status();
            if status.is_server_error() {
                return Err(Retry::Transient(format!(
                    "file GET {shown} returned HTTP {status} on retry"
                )));
            }
            if !status.is_success() {
                return Err(Retry::Fatal(format!(
                    "file GET {shown} returned HTTP {status} on retry"
                )));
            }
            if body_encoding(resp.headers()) != Ok(BodyEncoding::Gzip) {
                return Err(Retry::Fatal(format!(
                    "file GET {shown} changed its Content-Encoding on retry"
                )));
            }
            return Ok(Box::pin(resp.bytes_stream()));
        }

        let mut extra = vec![(header::RANGE, format!("bytes={}-", self.delivered))];
        if let Some((_, validator)) = &self.validator {
            extra.push((header::IF_RANGE, validator.clone()));
        }
        let resp = self.request.send(ACCEPT_IDENTITY, &extra).await?;
        let status = resp.status();
        // Only a body that will be spliced must be identity-encoded; an error
        // page may come compressed.
        if matches!(
            status,
            reqwest::StatusCode::PARTIAL_CONTENT | reqwest::StatusCode::OK
        ) && body_encoding(resp.headers()) != Ok(BodyEncoding::Identity)
        {
            return Err(Retry::Fatal(format!(
                "file GET {shown} answered a ranged resume with an encoded body"
            )));
        }
        match status {
            reqwest::StatusCode::PARTIAL_CONTENT => {
                let Some((first, last, complete)) = parse_content_range(resp.headers()) else {
                    return Err(Retry::Fatal(format!(
                        "file GET {shown} answered a ranged resume without a usable Content-Range"
                    )));
                };
                if first != self.delivered {
                    return Err(Retry::Fatal(format!(
                        "file GET {shown} resumed at byte {first}, not at byte {}",
                        self.delivered
                    )));
                }
                if let Some(complete) = complete {
                    self.check_length(complete)?;
                }
                self.skip = 0;
                self.body_end = Some(last + 1);
                Ok(Box::pin(resp.bytes_stream()))
            }
            reqwest::StatusCode::OK => {
                // With If-Range, a 200 carrying the same validator is a server
                // without Range support; any other means the file changed.
                if let Some((name, stored)) = &self.validator
                    && header_text(resp.headers(), name) != Some(stored.as_str())
                {
                    return Err(Retry::Fatal(format!(
                        "file {shown} changed between requests (If-Range did not match)"
                    )));
                }
                if let Some(length) = resp.content_length() {
                    self.check_length(length)?;
                }
                // The server ignored Range and resent the whole file.
                self.skip = self.delivered;
                self.body_end = resp.content_length();
                Ok(Box::pin(resp.bytes_stream()))
            }
            reqwest::StatusCode::RANGE_NOT_SATISFIABLE => {
                // Nothing lies past `delivered`: the break came after the last
                // byte, and the file was complete all along.
                match parse_unsatisfied_range(resp.headers()) {
                    Some(length) if length == self.delivered => {
                        self.check_length(length)?;
                        self.skip = 0;
                        self.body_end = Some(length);
                        Ok(Box::pin(futures::stream::empty()))
                    }
                    _ => Err(Retry::Fatal(format!(
                        "file GET {shown} returned HTTP {status} for a resume at byte {}",
                        self.delivered
                    ))),
                }
            }
            status if status.is_server_error() => Err(Retry::Transient(format!(
                "file GET {shown} returned HTTP {status} on retry"
            ))),
            status => Err(Retry::Fatal(format!(
                "file GET {shown} returned HTTP {status} on retry"
            ))),
        }
    }
}

#[async_trait]
impl SubmitInputFetcher for HttpSubmitInputFetcher {
    async fn fetch_manifest(
        &self,
        url: &str,
        request_headers: &[(String, String)],
        oauth_metadata_urls: &[String],
        encryption_key: Option<&Value>,
    ) -> StorageResult<RemoteManifest> {
        // The manifest itself may be protected; attempt with a token when one is
        // configured, falling back to anonymous when not.
        let with_token = self.token_provider.is_some() && !oauth_metadata_urls.is_empty();
        let shown = redact_url(url);
        let rb = self
            .build_get(url, request_headers, with_token, oauth_metadata_urls)
            .await?;
        let resp = rb
            .send()
            .await
            .map_err(|e| Self::err(format!("manifest GET {shown} failed: {}", error_chain(&e))))?;
        if !resp.status().is_success() {
            return Err(Self::err(format!(
                "manifest GET {shown} returned HTTP {}",
                resp.status()
            )));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| Self::err(format!("reading manifest {shown}: {}", error_chain(&e))))?
            .to_vec();

        // The spec has the provider encrypt the manifest as well as the files.
        // Manifests carry URLs rather than PHI and several providers leave them
        // in the clear, so a plaintext manifest is accepted with a warning while
        // a plaintext *file* is rejected outright.
        let keys = self.resolve_keys(encryption_key)?;
        let bytes = match &keys {
            Some(keys) if jwe::looks_like_jwe(&bytes) => jwe::decrypt(&bytes, keys)
                .map_err(|e| Self::err(format!("decrypting manifest {shown}: {e}")))?,
            Some(_) => {
                tracing::warn!(
                    manifest_url = %shown,
                    "fileEncryptionKey was supplied but the manifest is not encrypted"
                );
                bytes
            }
            None => bytes,
        };

        serde_json::from_slice::<RemoteManifest>(&bytes)
            .map_err(|e| Self::err(format!("parsing manifest {shown}: {e}")))
    }

    /// Returns as soon as the response headers arrive; the body is read lazily
    /// as the caller consumes lines, and a transfer that breaks is resumed
    /// (see the module docs). JWE-encrypted files are the exception and are
    /// buffered whole (the authentication tag trails the ciphertext).
    async fn open_file_stream(
        &self,
        url: &str,
        request_headers: &[(String, String)],
        requires_access_token: bool,
        oauth_metadata_urls: &[String],
        encryption_key: Option<&Value>,
    ) -> StorageResult<(Box<dyn AsyncBufRead + Send + Unpin>, Option<u64>)> {
        // Resolve the key before the fetch so a misconfigured submission fails
        // without pulling the file body.
        let keys = self.resolve_keys(encryption_key)?;
        if let Some(keys) = &keys {
            // A JWE is a single AEAD ciphertext whose authentication tag is the
            // final segment: no plaintext may be released before the whole body
            // has been read and the tag verified. Encrypted files are therefore
            // necessarily buffered; unencrypted ones stream.
            let request = self.file_request(
                &self.client,
                url,
                request_headers,
                requires_access_token,
                oauth_metadata_urls,
            );
            let bytes = self.fetch_whole_file(&request).await?;
            let bytes = self
                .decrypt_file(bytes, Some(keys))
                .map_err(|e| Self::err(format!("decrypting file {}: {e}", request.shown_url)))?;
            let len = bytes.len() as u64;
            return Ok((
                Box::new(tokio::io::BufReader::new(std::io::Cursor::new(bytes))),
                Some(len),
            ));
        }

        let request = self.file_request(
            &self.stream_client,
            url,
            request_headers,
            requires_access_token,
            oauth_metadata_urls,
        );
        let shown = request.shown_url.clone();
        let resp = request.send(ACCEPT_GZIP, &[]).await.map_err(|e| match e {
            Retry::Transient(message) | Retry::Fatal(message) => Self::err(message),
        })?;
        if !resp.status().is_success() {
            return Err(Self::err(format!(
                "file GET {shown} returned HTTP {}",
                resp.status()
            )));
        }
        let encoding = body_encoding(resp.headers())
            .map_err(|e| Self::err(format!("file GET {shown}: {e}")))?;
        // The advertised size, when trustworthy: for a gzip body Content-Length
        // is the compressed size and a byte-based percentage would overshoot.
        let content_length = match encoding {
            BodyEncoding::Identity => resp.content_length(),
            BodyEncoding::Gzip => None,
        };
        // Stream the body straight through to the ingestion engine, so peak
        // memory stays bounded by the buffer size rather than by the file size.
        let wire = Box::pin(ResumableBody::new(request, resp, encoding, self.retry).into_stream());
        let reader = StreamReader::new(wire);
        let reader: Box<dyn AsyncBufRead + Send + Unpin> = match encoding {
            BodyEncoding::Identity => Box::new(tokio::io::BufReader::new(reader)),
            BodyEncoding::Gzip => {
                let mut decoder = async_compression::tokio::bufread::GzipDecoder::new(reader);
                // Concatenated gzip members are one valid file, as reqwest reads them.
                decoder.multiple_members(true);
                Box::new(tokio::io::BufReader::new(decoder))
            }
        };
        Ok((reader, content_length))
    }

    async fn file_size(
        &self,
        url: &str,
        request_headers: &[(String, String)],
        requires_access_token: bool,
        oauth_metadata_urls: &[String],
    ) -> StorageResult<Option<u64>> {
        // Identity encoding on purpose: a gzip Content-Length would be the
        // compressed size, and the ingestion counts decompressed bytes. The
        // non-decoding client sends no `Accept-Encoding` of its own.
        let mut rb = self.stream_client.head(url);
        if requires_access_token {
            let Some(provider) = &self.token_provider else {
                return Ok(None);
            };
            let Some(token) = provider
                .token(oauth_metadata_urls, &self.outbound_scope)
                .await
            else {
                return Ok(None);
            };
            rb = rb.bearer_auth(token);
        }
        for (name, value) in request_headers {
            rb = rb.header(name.as_str(), value.as_str());
        }
        // Strictly best-effort: providers without HEAD support (405, 4xx/5xx,
        // network refusal) degrade to lazy per-file accumulation, never to an
        // error. The size comes from the Content-Length *header* — a HEAD
        // response has no payload, so `Response::content_length()` (the body
        // size hint) reports 0 regardless of what the header advertises.
        match rb.send().await {
            Ok(resp) if resp.status().is_success() => Ok(resp
                .headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes_gcm::aead::consts::U12;
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    use aes_gcm::{Aes256Gcm, Nonce};
    use base64::Engine;
    use serde_json::json;

    const B64URL: base64::engine::general_purpose::GeneralPurpose =
        base64::engine::general_purpose::URL_SAFE_NO_PAD;

    fn fetcher() -> HttpSubmitInputFetcher {
        HttpSubmitInputFetcher::new(None, "system/*.rs".to_string())
    }

    /// Builds a `dir` + A256GCM compact JWE over `plaintext`.
    fn seal(key: &[u8; 32], plaintext: &[u8]) -> String {
        let iv = [3u8; 12];
        let header = br#"{"alg":"dir","enc":"A256GCM"}"#;
        let header_b64 = B64URL.encode(header);
        let sealed = Aes256Gcm::new_from_slice(key)
            .unwrap()
            .encrypt(
                Nonce::<U12>::from_slice(&iv),
                Payload {
                    msg: plaintext,
                    aad: header_b64.as_bytes(),
                },
            )
            .unwrap();
        let (ct, tag) = sealed.split_at(sealed.len() - 16);
        format!(
            "{}..{}.{}.{}",
            header_b64,
            B64URL.encode(iv),
            B64URL.encode(ct),
            B64URL.encode(tag)
        )
    }

    #[test]
    fn test_decrypt_file_passthrough_when_unencrypted() {
        let out = fetcher().decrypt_file(b"hello".to_vec(), None).unwrap();
        assert_eq!(out, b"hello");
    }

    #[test]
    fn test_err_constructs_backend_internal() {
        let err = HttpSubmitInputFetcher::err("boom");
        assert!(err.to_string().contains("boom"));
    }

    #[tokio::test]
    async fn test_build_get_no_token_succeeds() {
        let rb = fetcher()
            .build_get("http://example.com/m.json", &[], false, &[])
            .await;
        assert!(rb.is_ok());
    }

    #[tokio::test]
    async fn test_build_get_requires_token_without_provider_errors() {
        let result = fetcher()
            .build_get(
                "http://example.com/file.ndjson",
                &[],
                true,
                &["http://example.com/.well-known/smart-configuration".to_string()],
            )
            .await;
        let err = result.unwrap_err();
        assert!(err.to_string().contains("no outbound auth is configured"));
    }

    #[tokio::test]
    async fn test_build_get_applies_provider_headers() {
        // Provider headers are applied even on the anonymous path.
        let headers = vec![("X-Custom".to_string(), "value".to_string())];
        let rb = fetcher()
            .build_get("http://example.com/m.json", &headers, false, &[])
            .await;
        assert!(rb.is_ok());
    }

    /// Serves one chunked NDJSON response: the first line, then (only after
    /// `release` fires) the second line and the terminating chunk. A buffering
    /// fetcher cannot hand back a reader until the body completes, so it would
    /// deadlock here — which the surrounding timeout turns into a failure.
    async fn serve_two_chunks(release: tokio::sync::oneshot::Receiver<()>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            // Drain the request head so the client's write completes.
            let mut req = Vec::new();
            let mut buf = [0u8; 1024];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                match sock.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => req.extend_from_slice(&buf[..n]),
                }
            }
            let head = b"HTTP/1.1 200 OK\r\nContent-Type: application/fhir+ndjson\r\n\
                         Transfer-Encoding: chunked\r\n\r\n";
            sock.write_all(head).await.unwrap();

            let line1 = "{\"resourceType\":\"Patient\",\"id\":\"1\"}\n";
            sock.write_all(format!("{:x}\r\n{line1}\r\n", line1.len()).as_bytes())
                .await
                .unwrap();
            sock.flush().await.unwrap();

            // Hold the body open until the test has consumed line 1.
            let _ = release.await;

            let line2 = "{\"resourceType\":\"Patient\",\"id\":\"2\"}\n";
            sock.write_all(format!("{:x}\r\n{line2}\r\n", line2.len()).as_bytes())
                .await
                .unwrap();
            sock.write_all(b"0\r\n\r\n").await.unwrap();
            sock.flush().await.unwrap();
        });
        format!("http://{addr}/file.ndjson")
    }

    #[tokio::test]
    async fn test_open_file_stream_yields_lines_before_body_completes() {
        use tokio::io::AsyncBufReadExt;

        let (release, wait) = tokio::sync::oneshot::channel();
        let url = serve_two_chunks(wait).await;
        let fetcher = fetcher();

        let (reader, _len) = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            fetcher.open_file_stream(&url, &[], false, &[], None),
        )
        .await
        .expect("open_file_stream must not wait for the whole body")
        .expect("fetch succeeds");

        let mut lines = reader.lines();
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), lines.next_line())
            .await
            .expect("first line must arrive before the body completes")
            .unwrap()
            .unwrap();
        assert!(first.contains("\"id\":\"1\""));

        // Let the server finish; the rest of the stream must follow.
        release.send(()).unwrap();
        let second = tokio::time::timeout(std::time::Duration::from_secs(5), lines.next_line())
            .await
            .expect("second line arrives")
            .unwrap()
            .unwrap();
        assert!(second.contains("\"id\":\"2\""));
        assert!(lines.next_line().await.unwrap().is_none());
    }

    /// One scripted HTTP/1.1 response, served on its own connection.
    #[derive(Clone)]
    struct Scripted {
        status: &'static str,
        headers: Vec<String>,
        body: Vec<u8>,
        /// Close the connection after this many body bytes, short of the
        /// advertised Content-Length: the client sees a broken body.
        cut_after: Option<usize>,
    }

    impl Scripted {
        fn ok(body: &[u8]) -> Self {
            Self {
                status: "200 OK",
                headers: Vec::new(),
                body: body.to_vec(),
                cut_after: None,
            }
        }

        fn header(mut self, line: impl Into<String>) -> Self {
            self.headers.push(line.into());
            self
        }

        fn cut(mut self, after: usize) -> Self {
            self.cut_after = Some(after);
            self
        }
    }

    type Requests = Arc<std::sync::Mutex<Vec<String>>>;

    /// Serves `script` one response per connection, in order, repeating the
    /// last one; records every request head, lowercased. Returns the base URL.
    async fn scripted_server(script: Vec<Scripted>) -> (String, Requests) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests: Requests = Arc::default();
        let seen = Arc::clone(&requests);
        tokio::spawn(async move {
            let mut served = 0usize;
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let mut head = Vec::new();
                let mut buf = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                seen.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&head).to_ascii_lowercase());
                let step = script[served.min(script.len() - 1)].clone();
                served += 1;

                // A `Transfer-Encoding: chunked` header line frames the body
                // as one chunk instead of with a Content-Length; a cut then
                // withholds the terminating chunk.
                let chunked = step
                    .headers
                    .iter()
                    .any(|line| line.eq_ignore_ascii_case("transfer-encoding: chunked"));
                let mut response = format!("HTTP/1.1 {}\r\n", step.status);
                for line in &step.headers {
                    response.push_str(line);
                    response.push_str("\r\n");
                }
                if !chunked {
                    response.push_str(&format!("Content-Length: {}\r\n", step.body.len()));
                }
                response.push_str("Connection: close\r\n\r\n");
                let sent = step.cut_after.unwrap_or(step.body.len());
                let _ = sock.write_all(response.as_bytes()).await;
                if chunked && !step.body.is_empty() {
                    let _ = sock
                        .write_all(format!("{:x}\r\n", step.body.len()).as_bytes())
                        .await;
                }
                let _ = sock.write_all(&step.body[..sent]).await;
                if chunked && step.cut_after.is_none() {
                    let tail: &[u8] = if step.body.is_empty() {
                        b"0\r\n\r\n"
                    } else {
                        b"\r\n0\r\n\r\n"
                    };
                    let _ = sock.write_all(tail).await;
                }
                let _ = sock.flush().await;
                let _ = sock.shutdown().await;
            }
        });
        (format!("http://{addr}"), requests)
    }

    fn ndjson_patients(n: usize) -> Vec<u8> {
        (0..n)
            .map(|i| {
                format!(
                    "{{\"resourceType\":\"Patient\",\"id\":\"p{i}\",\"n\":{}}}\n",
                    i * 7919
                )
            })
            .collect::<String>()
            .into_bytes()
    }

    fn quick_fetcher() -> HttpSubmitInputFetcher {
        fetcher().with_retry_policy(3, Duration::from_millis(10))
    }

    /// Reads a whole file through `open_file_stream`, bounded in time.
    async fn read_file(
        fetcher: &HttpSubmitInputFetcher,
        url: &str,
        key: Option<&Value>,
    ) -> (std::io::Result<Vec<u8>>, Option<u64>) {
        use tokio::io::AsyncReadExt;

        let (mut reader, len) = tokio::time::timeout(
            Duration::from_secs(10),
            fetcher.open_file_stream(url, &[], false, &[], key),
        )
        .await
        .expect("open_file_stream answers")
        .expect("file opens");
        let mut out = Vec::new();
        let read = tokio::time::timeout(Duration::from_secs(10), reader.read_to_end(&mut out))
            .await
            .expect("the read ends instead of hanging");
        (read.map(|_| out), len)
    }

    #[tokio::test]
    async fn test_a_cut_body_resumes_with_range_and_if_range() {
        let full = ndjson_patients(40);
        let cut = full.len() / 2 + 3; // mid-line on purpose
        let (base, requests) = scripted_server(vec![
            Scripted::ok(&full).header("ETag: \"v1\"").cut(cut),
            Scripted {
                status: "206 Partial Content",
                headers: vec![format!(
                    "Content-Range: bytes {cut}-{}/{}",
                    full.len() - 1,
                    full.len()
                )],
                body: full[cut..].to_vec(),
                cut_after: None,
            },
        ])
        .await;

        let (read, len) = read_file(&quick_fetcher(), &format!("{base}/f.ndjson"), None).await;
        assert_eq!(read.expect("the resumed read completes"), full);
        assert_eq!(len, Some(full.len() as u64));

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(
            requests[1].contains(&format!("range: bytes={cut}-")),
            "{}",
            requests[1]
        );
        assert!(requests[1].contains("if-range: \"v1\""), "{}", requests[1]);
        assert!(
            requests[1].contains("accept-encoding: identity"),
            "{}",
            requests[1]
        );
    }

    #[tokio::test]
    async fn test_a_server_ignoring_range_is_skipped_to_the_cut() {
        let full = ndjson_patients(40);
        let cut = full.len() / 3;
        let (base, requests) =
            scripted_server(vec![Scripted::ok(&full).cut(cut), Scripted::ok(&full)]).await;

        let (read, _) = read_file(&quick_fetcher(), &format!("{base}/f.ndjson"), None).await;
        assert_eq!(
            read.expect("the re-read completes"),
            full,
            "no line twice, none lost"
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(
            !requests[1].contains("if-range"),
            "no validator was offered"
        );
    }

    #[tokio::test]
    async fn test_a_changed_file_is_not_spliced() {
        let full = ndjson_patients(40);
        let cut = full.len() / 2;
        let (base, _) = scripted_server(vec![
            Scripted::ok(&full).header("ETag: \"v1\"").cut(cut),
            // If-Range did not match: the server sends the new file whole.
            Scripted::ok(&ndjson_patients(41)).header("ETag: \"v2\""),
        ])
        .await;

        let (read, _) = read_file(&quick_fetcher(), &format!("{base}/f.ndjson"), None).await;
        let err = read.expect_err("a changed file must fail, not splice");
        assert!(
            err.to_string().contains("changed between requests"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn test_a_body_that_keeps_breaking_fails_with_a_redacted_url() {
        let full = ndjson_patients(40);
        let (base, requests) = scripted_server(vec![Scripted::ok(&full).cut(full.len() / 2)]).await;
        let fetcher = fetcher().with_retry_policy(2, Duration::from_millis(10));

        let url = format!("{base}/f.ndjson?X-Amz-Signature=secret123");
        let (read, _) = read_file(&fetcher, &url, None).await;
        let err = read.expect_err("the file cannot be completed").to_string();
        assert!(err.contains("gave up after"), "{err}");
        assert!(err.contains("[redacted]"), "{err}");
        assert!(
            !err.contains("secret123"),
            "the signature must not leak: {err}"
        );
        assert_eq!(
            requests.lock().unwrap().len(),
            3,
            "first GET plus two retries"
        );
    }

    #[tokio::test]
    async fn test_a_gzip_body_is_decoded_and_reports_no_length() {
        use std::io::Write;

        let full = ndjson_patients(200);
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&full).unwrap();
        let gz = encoder.finish().unwrap();
        let (base, requests) =
            scripted_server(vec![Scripted::ok(&gz).header("Content-Encoding: gzip")]).await;

        let (read, len) = read_file(&quick_fetcher(), &format!("{base}/f.ndjson"), None).await;
        assert_eq!(read.expect("decoded"), full);
        assert_eq!(
            len, None,
            "a compressed Content-Length is not the ingested size"
        );
        assert!(requests.lock().unwrap()[0].contains("accept-encoding: gzip"));
    }

    #[tokio::test]
    async fn test_a_cut_gzip_body_is_not_resumed_with_range() {
        use std::io::Write;

        let full = ndjson_patients(5000);
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&full).unwrap();
        let gz = encoder.finish().unwrap();
        let (base, requests) = scripted_server(vec![
            Scripted::ok(&gz)
                .header("Content-Encoding: gzip")
                .cut(gz.len() / 2),
        ])
        .await;

        let (read, _) = read_file(&quick_fetcher(), &format!("{base}/f.ndjson"), None).await;
        let err = read.expect_err("a cut gzip body fails").to_string();
        assert!(err.contains("gzip-encoded"), "{err}");
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            1,
            "no ranged request against an encoded body"
        );
    }

    #[tokio::test]
    async fn test_a_cut_encrypted_file_is_refetched_whole() {
        let key = [7u8; 32];
        let plaintext = ndjson_patients(20);
        let compact = seal(&key, &plaintext).into_bytes();
        let enc_key = json!({"coding": {"code": "jwe"}, "value": B64URL.encode(key)});
        let (base, requests) = scripted_server(vec![
            Scripted::ok(&compact).cut(compact.len() / 2),
            Scripted::ok(&compact),
        ])
        .await;

        let (read, len) = read_file(
            &quick_fetcher(),
            &format!("{base}/f.ndjson.jwe"),
            Some(&enc_key),
        )
        .await;
        assert_eq!(read.expect("decrypted"), plaintext);
        assert_eq!(len, Some(plaintext.len() as u64));
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(!requests[1].contains("range:"), "a JWE is re-fetched whole");
    }

    #[tokio::test]
    async fn test_a_server_error_on_an_encrypted_refetch_is_retried() {
        let key = [7u8; 32];
        let plaintext = ndjson_patients(20);
        let compact = seal(&key, &plaintext).into_bytes();
        let enc_key = json!({"coding": {"code": "jwe"}, "value": B64URL.encode(key)});
        let (base, requests) = scripted_server(vec![
            Scripted::ok(&compact).cut(compact.len() / 2),
            Scripted {
                status: "503 Service Unavailable",
                ..Scripted::ok(b"busy")
            },
            Scripted::ok(&compact),
        ])
        .await;

        let (read, _) = read_file(
            &quick_fetcher(),
            &format!("{base}/f.ndjson.jwe"),
            Some(&enc_key),
        )
        .await;
        assert_eq!(read.expect("decrypted"), plaintext);
        assert_eq!(requests.lock().unwrap().len(), 3);
    }

    /// A `206` for `full[first..end]`, stating the complete length.
    fn partial(full: &[u8], first: usize, end: usize) -> Scripted {
        Scripted {
            status: "206 Partial Content",
            headers: vec![format!(
                "Content-Range: bytes {first}-{}/{}",
                end - 1,
                full.len()
            )],
            body: full[first..end].to_vec(),
            cut_after: None,
        }
    }

    #[tokio::test]
    async fn test_a_short_206_is_followed_by_another_range_request() {
        let full = ndjson_patients(40);
        let cut = full.len() / 3;
        let short_end = 2 * full.len() / 3;
        let (base, requests) = scripted_server(vec![
            Scripted::ok(&full).header("ETag: \"v1\"").cut(cut),
            // A well-framed 206 that stops short of the file.
            partial(&full, cut, short_end),
            partial(&full, short_end, full.len()),
        ])
        .await;

        let (read, _) = read_file(&quick_fetcher(), &format!("{base}/f.ndjson"), None).await;
        assert_eq!(read.expect("the file is completed, not truncated"), full);
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(
            requests[2].contains(&format!("range: bytes={short_end}-")),
            "{}",
            requests[2]
        );
    }

    #[tokio::test]
    async fn test_a_resent_file_shorter_than_the_offset_fails() {
        let full = ndjson_patients(40);
        let cut = full.len() / 2;
        let (base, _) = scripted_server(vec![
            Scripted::ok(&full).cut(cut),
            // Range ignored, and no Content-Length to compare: the resend
            // simply ends before the already-delivered offset.
            Scripted::ok(&full[..cut / 2]).header("Transfer-Encoding: chunked"),
        ])
        .await;

        let (read, _) = read_file(&quick_fetcher(), &format!("{base}/f.ndjson"), None).await;
        let err = read
            .expect_err("a shrunken resend must not end the file")
            .to_string();
        assert!(err.contains("resent shorter"), "{err}");
        assert!(err.contains("gave up after"), "{err}");
    }

    #[tokio::test]
    async fn test_a_compressed_server_error_on_resume_is_retried() {
        let full = ndjson_patients(40);
        let cut = full.len() / 2;
        let (base, requests) = scripted_server(vec![
            Scripted::ok(&full).header("ETag: \"v1\"").cut(cut),
            Scripted {
                status: "503 Service Unavailable",
                ..Scripted::ok(b"\x1f\x8b not really gzip")
            }
            .header("Content-Encoding: gzip"),
            partial(&full, cut, full.len()),
        ])
        .await;

        let (read, _) = read_file(&quick_fetcher(), &format!("{base}/f.ndjson"), None).await;
        assert_eq!(read.expect("the 503 is transient"), full);
        assert_eq!(requests.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn test_a_416_at_the_complete_length_ends_the_file() {
        let full = ndjson_patients(40);
        let (base, requests) = scripted_server(vec![
            // Every byte arrives, but the terminating chunk never does.
            Scripted::ok(&full)
                .header("Transfer-Encoding: chunked")
                .cut(full.len()),
            Scripted {
                status: "416 Range Not Satisfiable",
                ..Scripted::ok(b"")
            }
            .header(format!("Content-Range: bytes */{}", full.len())),
        ])
        .await;

        let (read, _) = read_file(&quick_fetcher(), &format!("{base}/f.ndjson"), None).await;
        assert_eq!(read.expect("the file was complete"), full);
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(
            requests[1].contains(&format!("range: bytes={}-", full.len())),
            "{}",
            requests[1]
        );
    }

    #[tokio::test]
    async fn test_a_200_with_the_same_etag_after_if_range_is_skipped_to_the_cut() {
        let full = ndjson_patients(40);
        let cut = full.len() / 3;
        let (base, requests) = scripted_server(vec![
            Scripted::ok(&full).header("ETag: \"v1\"").cut(cut),
            // Same validator: the server just does not do Range.
            Scripted::ok(&full).header("ETag: \"v1\""),
        ])
        .await;

        let (read, _) = read_file(&quick_fetcher(), &format!("{base}/f.ndjson"), None).await;
        assert_eq!(read.expect("the re-read completes"), full);
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].contains("if-range: \"v1\""), "{}", requests[1]);
    }

    #[tokio::test]
    async fn test_manifest_errors_do_not_leak_the_query_string() {
        // A port nothing listens on: the GET fails to connect.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let url = format!("http://127.0.0.1:{port}/manifest.json?sig=secret123");
        let err = fetcher()
            .fetch_manifest(&url, &[], &[], None)
            .await
            .expect_err("nothing listens")
            .to_string();
        assert!(err.contains("[redacted]"), "{err}");
        assert!(
            !err.contains("secret123"),
            "the signature must not leak: {err}"
        );
    }

    #[test]
    fn test_retry_delays_double() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.delay(1), Duration::from_secs(1));
        assert_eq!(policy.delay(2), Duration::from_secs(2));
        assert_eq!(policy.delay(3), Duration::from_secs(4));
    }

    #[test]
    fn test_if_range_prefers_a_strong_etag_and_skips_weak_ones() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ETAG, "W/\"weak\"".parse().unwrap());
        headers.insert(
            header::LAST_MODIFIED,
            "Tue, 15 Sep 2026 10:00:00 GMT".parse().unwrap(),
        );
        assert_eq!(
            if_range_validator(&headers),
            Some((
                header::LAST_MODIFIED,
                "Tue, 15 Sep 2026 10:00:00 GMT".to_string()
            ))
        );
        headers.insert(header::ETAG, "\"strong\"".parse().unwrap());
        assert_eq!(
            if_range_validator(&headers),
            Some((header::ETAG, "\"strong\"".to_string()))
        );
    }

    #[test]
    fn test_parse_content_range() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_RANGE, "bytes 100-199/200".parse().unwrap());
        assert_eq!(parse_content_range(&headers), Some((100, 199, Some(200))));
        headers.insert(header::CONTENT_RANGE, "bytes 5-9/*".parse().unwrap());
        assert_eq!(parse_content_range(&headers), Some((5, 9, None)));
        // Inverted, or past the complete length: not usable.
        headers.insert(header::CONTENT_RANGE, "bytes 9-5/*".parse().unwrap());
        assert_eq!(parse_content_range(&headers), None);
        headers.insert(header::CONTENT_RANGE, "bytes 0-200/200".parse().unwrap());
        assert_eq!(parse_content_range(&headers), None);
        headers.insert(header::CONTENT_RANGE, "bytes */200".parse().unwrap());
        assert_eq!(parse_content_range(&headers), None);
        assert_eq!(parse_unsatisfied_range(&headers), Some(200));
    }

    #[test]
    fn test_decrypts_file_with_flattened_key_value() {
        let key = [7u8; 32];
        let plaintext = b"{\"resourceType\":\"Patient\",\"id\":\"x\"}\n";
        let compact = seal(&key, plaintext);
        let enc_key = json!({"coding": {"code": "jwe"}, "value": B64URL.encode(key)});

        let f = fetcher();
        let keys = f.resolve_keys(Some(&enc_key)).unwrap();
        let out = f
            .decrypt_file(compact.clone().into_bytes(), keys.as_ref())
            .unwrap();
        assert_eq!(out, plaintext);

        // A wrong key fails the authentication tag.
        let wrong = json!({"value": B64URL.encode([9u8; 32])});
        let wrong_keys = f.resolve_keys(Some(&wrong)).unwrap();
        assert!(
            f.decrypt_file(compact.into_bytes(), wrong_keys.as_ref())
                .is_err()
        );
    }

    #[test]
    fn test_decrypts_file_with_spec_shaped_parts() {
        // The spec shape: fileEncryptionKey is a part with `coding`/`value` parts.
        let key = [11u8; 32];
        let plaintext = b"line\n";
        let compact = seal(&key, plaintext);
        let enc_key = json!({
            "name": "fileEncryptionKey",
            "part": [
                {"name": "coding", "valueCoding": {
                    "system": "http://hl7.org/fhir/uv/bulkdata/ValueSet/file-encryption-type",
                    "code": "jwe"
                }},
                {"name": "value", "valueString": B64URL.encode(key)},
            ]
        });
        let f = fetcher();
        let keys = f.resolve_keys(Some(&enc_key)).unwrap();
        let out = f.decrypt_file(compact.into_bytes(), keys.as_ref()).unwrap();
        assert_eq!(out, plaintext);
    }

    #[test]
    fn test_accepts_an_oct_jwk_as_the_key_value() {
        let key = [13u8; 32];
        let plaintext = b"jwk\n";
        let compact = seal(&key, plaintext);
        let jwk = json!({"kty": "oct", "k": B64URL.encode(key)}).to_string();
        let enc_key = json!({"value": jwk});
        let f = fetcher();
        let keys = f.resolve_keys(Some(&enc_key)).unwrap();
        let out = f.decrypt_file(compact.into_bytes(), keys.as_ref()).unwrap();
        assert_eq!(out, plaintext);
    }

    #[test]
    fn test_unwraps_a_jwe_wrapped_content_encryption_key() {
        use rand::rngs::OsRng;

        // The provider delivers the CEK as an ECDH-ES JWE addressed to HFS's
        // public key, rather than putting the raw key in `value`.
        let recipient = p256::SecretKey::random(&mut OsRng);
        let ephemeral = p256::SecretKey::random(&mut OsRng);
        let epk: Value = serde_json::from_str(&ephemeral.public_key().to_jwk_string()).unwrap();
        let z = p256::ecdh::diffie_hellman(
            ephemeral.to_nonzero_scalar(),
            recipient.public_key().as_affine(),
        );
        let wrap_cek = crate::jwe::concat_kdf(z.raw_secret_bytes(), "A256GCM", b"", b"", 32);

        let cek = [17u8; 32];
        let iv = [23u8; 12];
        let header =
            serde_json::json!({"alg": "ECDH-ES", "enc": "A256GCM", "epk": epk}).to_string();
        let header_b64 = B64URL.encode(&header);
        let sealed = Aes256Gcm::new_from_slice(&wrap_cek)
            .unwrap()
            .encrypt(
                Nonce::<U12>::from_slice(&iv),
                Payload {
                    // The wrapped payload is the file CEK, base64url-encoded.
                    msg: B64URL.encode(cek).as_bytes(),
                    aad: header_b64.as_bytes(),
                },
            )
            .unwrap();
        let (ct, tag) = sealed.split_at(sealed.len() - 16);
        let key_jwe = format!(
            "{}..{}.{}.{}",
            header_b64,
            B64URL.encode(iv),
            B64URL.encode(ct),
            B64URL.encode(tag)
        );

        let plaintext = b"wrapped\n";
        let file = seal(&cek, plaintext);
        let enc_key = json!({"value": key_jwe});

        let f = fetcher().with_decryption_keys(vec![PrivateKey::P256 {
            kid: None,
            key: recipient,
        }]);
        let keys = f.resolve_keys(Some(&enc_key)).unwrap();
        let out = f.decrypt_file(file.into_bytes(), keys.as_ref()).unwrap();
        assert_eq!(out, plaintext);

        // Without the private key the failure names the missing configuration.
        let err = fetcher().resolve_keys(Some(&enc_key)).unwrap_err();
        assert!(
            err.to_string().contains("HFS_BULK_SUBMIT_DECRYPTION_KEY"),
            "{err}"
        );
    }

    #[test]
    fn test_rejects_unknown_encryption_coding() {
        let enc_key = json!({"coding": {"code": "pgp"}, "value": "AAAA"});
        let err = fetcher().resolve_keys(Some(&enc_key)).unwrap_err();
        assert!(err.to_string().contains("'pgp'"), "{err}");
    }

    #[test]
    fn test_rejects_missing_key_value() {
        let enc_key = json!({"coding": {"code": "jwe"}});
        let err = fetcher().resolve_keys(Some(&enc_key)).unwrap_err();
        assert!(err.to_string().contains("value is required"), "{err}");
    }

    #[test]
    fn test_rejects_unusable_key_value() {
        let enc_key = json!({"value": "not-a-key"});
        let err = fetcher().resolve_keys(Some(&enc_key)).unwrap_err();
        assert!(err.to_string().contains("base64url"), "{err}");
    }

    #[test]
    fn test_rejects_a_plaintext_file_when_a_key_was_supplied() {
        let enc_key = json!({"value": B64URL.encode([5u8; 32])});
        let f = fetcher();
        let keys = f.resolve_keys(Some(&enc_key)).unwrap();
        let err = f
            .decrypt_file(b"{\"resourceType\":\"Patient\"}\n".to_vec(), keys.as_ref())
            .unwrap_err();
        assert!(err.to_string().contains("is not a JWE"), "{err}");
    }

    #[test]
    fn test_interpret_cek_forms() {
        // base64url of a 32-byte key
        assert_eq!(
            interpret_cek(B64URL.encode([1u8; 32]).as_bytes())
                .unwrap()
                .len(),
            32
        );
        // oct JWK
        let jwk = json!({"kty": "oct", "k": B64URL.encode([2u8; 16])}).to_string();
        assert_eq!(interpret_cek(jwk.as_bytes()).unwrap().len(), 16);
        // raw bytes at a valid AES length
        assert_eq!(interpret_cek(&[3u8; 64]).unwrap().len(), 64);
        // nothing usable
        assert!(interpret_cek(b"nope").is_none());
        assert!(interpret_cek(&[4u8; 17]).is_none());
    }
}
