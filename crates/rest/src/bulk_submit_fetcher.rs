//! Concrete HTTP implementation of [`SubmitInputFetcher`].
//!
//! Lives in `helios-rest` (which already depends on `reqwest`) rather than
//! `helios-persistence`, keeping the persistence crate free of an HTTP client.
//! Fetches the remote Bulk Export Manifest and its NDJSON files, applying
//! provider-supplied request headers, `gzip`, and — when files require an access
//! token — a read-scoped bearer obtained via an optional [`FileTokenProvider`].
//!
//! When the submission carries a `fileEncryptionKey`, the manifest and every
//! file are decrypted with the [`crate::jwe`] module (built unconditionally).

use std::sync::Arc;

use async_trait::async_trait;
use helios_persistence::core::{FileTokenProvider, RemoteManifest, SubmitInputFetcher};
use helios_persistence::error::{BackendError, StorageError, StorageResult};
use serde_json::Value;
use tokio::io::AsyncBufRead;

use crate::jwe::{self, DecryptionKeys, PrivateKey};

/// The default `fileEncryptionKey.coding` code (submit spec: "If omitted,
/// defaults to a system of `…/file-encryption-type` and code of `jwe`").
const FILE_ENCRYPTION_TYPE_JWE: &str = "jwe";

/// Fetches submit input over HTTP(S) using `reqwest`.
pub struct HttpSubmitInputFetcher {
    client: reqwest::Client,
    /// Optional token source for `requiresAccessToken=true` provider files.
    token_provider: Option<Arc<dyn FileTokenProvider>>,
    /// Read scope requested for the outbound file-retrieval token (e.g. `system/*.rs`).
    outbound_scope: String,
    /// Local private keys used when `fileEncryptionKey.value` (or a file's JWE)
    /// addresses HFS asymmetrically — `RSA-OAEP*` / `ECDH-ES*`.
    decryption_keys: Vec<PrivateKey>,
}

impl HttpSubmitInputFetcher {
    /// Creates a fetcher with the given optional outbound token provider and scope.
    pub fn new(token_provider: Option<Arc<dyn FileTokenProvider>>, outbound_scope: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            token_provider,
            outbound_scope,
            decryption_keys: Vec::new(),
        }
    }

    /// Adds the private keys used to unwrap asymmetrically addressed JWEs
    /// (`HFS_BULK_SUBMIT_DECRYPTION_KEY`).
    pub fn with_decryption_keys(mut self, keys: Vec<PrivateKey>) -> Self {
        self.decryption_keys = keys;
        self
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
            .map_err(|e| Self::err(format!("reading manifest {url}: {e}")))?
            .to_vec();

        // The spec has the provider encrypt the manifest as well as the files.
        // Manifests carry URLs rather than PHI and several providers leave them
        // in the clear, so a plaintext manifest is accepted with a warning while
        // a plaintext *file* is rejected outright.
        let keys = self.resolve_keys(encryption_key)?;
        let bytes = match &keys {
            Some(keys) if jwe::looks_like_jwe(&bytes) => jwe::decrypt(&bytes, keys)
                .map_err(|e| Self::err(format!("decrypting manifest {url}: {e}")))?,
            Some(_) => {
                tracing::warn!(
                    manifest_url = url,
                    "fileEncryptionKey was supplied but the manifest is not encrypted"
                );
                bytes
            }
            None => bytes,
        };

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
        // Resolve the key before the fetch so a misconfigured submission fails
        // without pulling the file body.
        let keys = self.resolve_keys(encryption_key)?;
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
        let bytes = self
            .decrypt_file(bytes.to_vec(), keys.as_ref())
            .map_err(|e| Self::err(format!("decrypting file {url}: {e}")))?;
        Ok(Box::new(tokio::io::BufReader::new(std::io::Cursor::new(
            bytes,
        ))))
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
