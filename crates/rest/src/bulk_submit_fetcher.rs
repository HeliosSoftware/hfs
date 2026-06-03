//! Concrete HTTP implementation of [`SubmitInputFetcher`].
//!
//! Lives in `helios-rest` (which already depends on `reqwest`) rather than
//! `helios-persistence`, keeping the persistence crate free of an HTTP client.
//! Fetches the remote Bulk Export Manifest and its NDJSON files, applying
//! provider-supplied request headers, `gzip`, and — when files require an access
//! token — a read-scoped bearer obtained via an optional [`FileTokenProvider`].

use std::sync::Arc;

use async_trait::async_trait;
use helios_persistence::core::{FileTokenProvider, RemoteManifest, SubmitInputFetcher};
use helios_persistence::error::{BackendError, StorageError, StorageResult};
use serde_json::Value;
use tokio::io::AsyncBufRead;

/// Fetches submit input over HTTP(S) using `reqwest`.
pub struct HttpSubmitInputFetcher {
    client: reqwest::Client,
    /// Optional token source for `requiresAccessToken=true` provider files.
    token_provider: Option<Arc<dyn FileTokenProvider>>,
    /// Read scope requested for the outbound file-retrieval token (e.g. `system/*.rs`).
    outbound_scope: String,
}

impl HttpSubmitInputFetcher {
    /// Creates a fetcher with the given optional outbound token provider and scope.
    pub fn new(token_provider: Option<Arc<dyn FileTokenProvider>>, outbound_scope: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            token_provider,
            outbound_scope,
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
        let mut rb = self.client.get(url).header("Accept-Encoding", "gzip");
        if with_token {
            if let Some(provider) = &self.token_provider {
                if let Some(token) = provider
                    .token(oauth_metadata_urls, &self.outbound_scope)
                    .await
                {
                    rb = rb.bearer_auth(token);
                } else {
                    return Err(Self::err(format!(
                        "{url} requires an access token but none could be obtained"
                    )));
                }
            } else {
                return Err(Self::err(format!(
                    "{url} requires an access token but no outbound auth is configured"
                )));
            }
        }
        // Provider-supplied headers take precedence (applied last).
        for (name, value) in request_headers {
            rb = rb.header(name.as_str(), value.as_str());
        }
        Ok(rb)
    }

    /// Decrypts `bytes` when a `fileEncryptionKey` is present.
    ///
    /// Without the `bulk-submit-jwe` feature this errors (documented
    /// non-conformance). With the feature it decrypts a `dir` + A128/A256GCM
    /// compact JWE, treating `fileEncryptionKey.value` as the base64url symmetric
    /// content-encryption key.
    fn maybe_decrypt(bytes: Vec<u8>, encryption_key: Option<&Value>) -> StorageResult<Vec<u8>> {
        let Some(key) = encryption_key else {
            return Ok(bytes);
        };
        #[cfg(not(feature = "bulk-submit-jwe"))]
        {
            let _ = key;
            Err(Self::err(
                "fileEncryptionKey (JWE) is not supported in this build \
                 (enable the `bulk-submit-jwe` feature)",
            ))
        }
        #[cfg(feature = "bulk-submit-jwe")]
        {
            let value = key
                .get("value")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    // value may also be supplied as a nested part: { name: "value", valueString }
                    key.get("part")
                        .and_then(|p| p.as_array())
                        .and_then(|arr| {
                            arr.iter()
                                .find(|c| c.get("name").and_then(|n| n.as_str()) == Some("value"))
                        })
                        .and_then(|c| c.get("valueString").and_then(|v| v.as_str()))
                })
                .ok_or_else(|| Self::err("fileEncryptionKey.value is required"))?;
            jwe::decrypt_dir_gcm(&bytes, value).map_err(Self::err)
        }
    }
}

#[cfg(feature = "bulk-submit-jwe")]
mod jwe {
    //! Minimal `dir` + AxxxGCM compact-JWE decryption (pure Rust, no OpenSSL).

    use aes_gcm::aead::{Aead, KeyInit, Payload};
    use aes_gcm::{Aes128Gcm, Aes256Gcm, Nonce};
    use base64::Engine;

    fn b64(part: &str) -> Result<Vec<u8>, String> {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(part)
            .map_err(|e| format!("base64url decode failed: {e}"))
    }

    /// Decrypts a compact JWE (`hdr.ek.iv.ct.tag`) created with `alg=dir` and
    /// `enc=A128GCM|A256GCM`, using `key_b64url` as the symmetric key.
    pub fn decrypt_dir_gcm(jwe_bytes: &[u8], key_b64url: &str) -> Result<Vec<u8>, String> {
        let compact = std::str::from_utf8(jwe_bytes)
            .map_err(|_| "JWE is not valid UTF-8".to_string())?
            .trim();
        let parts: Vec<&str> = compact.split('.').collect();
        if parts.len() != 5 {
            return Err("compact JWE must have 5 segments".to_string());
        }
        let header_b64 = parts[0];
        let header_json = b64(header_b64)?;
        let header: serde_json::Value =
            serde_json::from_slice(&header_json).map_err(|e| format!("invalid JWE header: {e}"))?;
        if header.get("alg").and_then(|v| v.as_str()) != Some("dir") {
            return Err("only alg=dir is supported".to_string());
        }
        let enc = header
            .get("enc")
            .and_then(|v| v.as_str())
            .unwrap_or("A256GCM");
        if !parts[1].is_empty() {
            return Err("alg=dir must not carry an encrypted key".to_string());
        }
        let iv = b64(parts[2])?;
        let ct = b64(parts[3])?;
        let tag = b64(parts[4])?;
        let key = b64(key_b64url)?;
        // AEAD ciphertext = ciphertext || tag; AAD = ASCII(base64url(header)).
        let mut ciphertext = ct;
        ciphertext.extend_from_slice(&tag);
        let nonce = Nonce::from_slice(&iv);
        let aad = header_b64.as_bytes();
        let payload = Payload {
            msg: &ciphertext,
            aad,
        };
        match enc {
            "A256GCM" => {
                let cipher = Aes256Gcm::new_from_slice(&key)
                    .map_err(|e| format!("invalid A256GCM key: {e}"))?;
                cipher
                    .decrypt(nonce, payload)
                    .map_err(|_| "JWE decryption failed (A256GCM)".to_string())
            }
            "A128GCM" => {
                let cipher = Aes128Gcm::new_from_slice(&key)
                    .map_err(|e| format!("invalid A128GCM key: {e}"))?;
                cipher
                    .decrypt(nonce, payload)
                    .map_err(|_| "JWE decryption failed (A128GCM)".to_string())
            }
            other => Err(format!(
                "unsupported JWE enc '{other}' (expected A128GCM|A256GCM)"
            )),
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
    ) -> StorageResult<RemoteManifest> {
        // The manifest itself may be protected; attempt with a token when one is
        // configured, falling back to anonymous when not.
        let with_token = self.token_provider.is_some() && !oauth_metadata_urls.is_empty();
        let rb = self
            .build_get(url, request_headers, with_token, oauth_metadata_urls)
            .await?;
        let resp = rb
            .send()
            .await
            .map_err(|e| Self::err(format!("manifest GET {url} failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(Self::err(format!(
                "manifest GET {url} returned HTTP {}",
                resp.status()
            )));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| Self::err(format!("reading manifest {url}: {e}")))?;
        serde_json::from_slice::<RemoteManifest>(&bytes)
            .map_err(|e| Self::err(format!("parsing manifest {url}: {e}")))
    }

    async fn open_file_stream(
        &self,
        url: &str,
        request_headers: &[(String, String)],
        requires_access_token: bool,
        oauth_metadata_urls: &[String],
        encryption_key: Option<&Value>,
    ) -> StorageResult<Box<dyn AsyncBufRead + Send + Unpin>> {
        let rb = self
            .build_get(
                url,
                request_headers,
                requires_access_token,
                oauth_metadata_urls,
            )
            .await?;
        let resp = rb
            .send()
            .await
            .map_err(|e| Self::err(format!("file GET {url} failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(Self::err(format!(
                "file GET {url} returned HTTP {}",
                resp.status()
            )));
        }
        // Buffer the (gzip-decoded) body; NDJSON files in practice fit comfortably,
        // and this avoids pulling in a stream→AsyncRead bridge dependency.
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| Self::err(format!("reading file {url}: {e}")))?;
        let bytes = Self::maybe_decrypt(bytes.to_vec(), encryption_key)?;
        Ok(Box::new(tokio::io::BufReader::new(std::io::Cursor::new(
            bytes,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_maybe_decrypt_passthrough_when_unencrypted() {
        let out = HttpSubmitInputFetcher::maybe_decrypt(b"hello".to_vec(), None).unwrap();
        assert_eq!(out, b"hello");
    }

    #[cfg(not(feature = "bulk-submit-jwe"))]
    #[test]
    fn test_maybe_decrypt_rejected_without_feature() {
        let key = json!({"coding": {"code": "jwe"}, "value": "x"});
        assert!(HttpSubmitInputFetcher::maybe_decrypt(b"data".to_vec(), Some(&key)).is_err());
    }

    #[cfg(feature = "bulk-submit-jwe")]
    #[test]
    fn test_jwe_dir_a256gcm_round_trip() {
        use aes_gcm::aead::{Aead, KeyInit, Payload};
        use aes_gcm::{Aes256Gcm, Nonce};
        use base64::Engine;
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;

        let key = [7u8; 32];
        let iv = [3u8; 12];
        let plaintext = b"{\"resourceType\":\"Patient\",\"id\":\"x\"}\n";
        let header_json = br#"{"alg":"dir","enc":"A256GCM"}"#;
        let header_b64 = b64.encode(header_json);

        let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
        let ct_and_tag = cipher
            .encrypt(
                Nonce::from_slice(&iv),
                Payload {
                    msg: plaintext,
                    aad: header_b64.as_bytes(),
                },
            )
            .unwrap();
        let (ct, tag) = ct_and_tag.split_at(ct_and_tag.len() - 16);
        let compact = format!(
            "{}..{}.{}.{}",
            header_b64,
            b64.encode(iv),
            b64.encode(ct),
            b64.encode(tag)
        );

        let out =
            super::jwe::decrypt_dir_gcm(compact.as_bytes(), &b64.encode(key)).expect("decrypt");
        assert_eq!(out, plaintext);

        // Through the public surface (key as fileEncryptionKey.value).
        let enc_key = json!({"coding": {"code": "jwe"}, "value": b64.encode(key)});
        let out2 =
            HttpSubmitInputFetcher::maybe_decrypt(compact.clone().into_bytes(), Some(&enc_key))
                .unwrap();
        assert_eq!(out2, plaintext);

        // Wrong key → decryption fails (authentication-tag mismatch).
        let wrong = json!({"coding": {"code": "jwe"}, "value": b64.encode([9u8; 32])});
        assert!(HttpSubmitInputFetcher::maybe_decrypt(compact.into_bytes(), Some(&wrong)).is_err());
    }
}
