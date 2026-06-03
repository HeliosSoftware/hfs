//! Outbound SMART Backend Services client for fetching protected provider files.
//!
//! Implements [`FileTokenProvider`]: discovers the token endpoint from the
//! provider's `oauthMetadataUrl` (`.well-known/smart-configuration`), builds an
//! `RS384`/`ES384` `private_key_jwt` client assertion, exchanges it for a
//! short-lived access token via the `client_credentials` grant, and caches the
//! token per `(token_endpoint, scope)`.
//!
//! **Scope:** per the Bulk Data Submit spec, file-retrieval tokens are scoped to
//! *read the resource types in the manifest* (e.g. `system/*.rs`) — **never** the
//! `system/bulk-submit` operation scope used for HFS's own endpoints.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use helios_persistence::core::FileTokenProvider;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::Value;
use tokio::sync::Mutex;

/// Acquires read-scoped bearer tokens via SMART Backend Services `client_credentials`.
pub struct JwtClientCredentialsTokenProvider {
    client: reqwest::Client,
    client_id: String,
    signing_alg: Algorithm,
    encoding_key: EncodingKey,
    /// Cache of `(token_endpoint, scope) -> (token, expiry)`.
    cache: Mutex<HashMap<(String, String), (String, Instant)>>,
}

impl JwtClientCredentialsTokenProvider {
    /// Builds a provider from a `client_id`, a PEM private key, and a signing alg
    /// (`ES384` or `RS384`). Returns `None` if the key can't be parsed.
    pub fn new(client_id: &str, private_key_pem: &str, signing_alg: &str) -> Option<Arc<Self>> {
        let (alg, key) = match signing_alg {
            "ES384" => (
                Algorithm::ES384,
                EncodingKey::from_ec_pem(private_key_pem.as_bytes()).ok()?,
            ),
            "RS384" => (
                Algorithm::RS384,
                EncodingKey::from_rsa_pem(private_key_pem.as_bytes()).ok()?,
            ),
            _ => return None,
        };
        Some(Arc::new(Self {
            client: reqwest::Client::new(),
            client_id: client_id.to_string(),
            signing_alg: alg,
            encoding_key: key,
            cache: Mutex::new(HashMap::new()),
        }))
    }

    /// Resolves the token endpoint from a SMART/OAuth metadata URL.
    async fn discover_token_endpoint(&self, oauth_metadata_url: &str) -> Option<String> {
        let resp = self.client.get(oauth_metadata_url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let meta: Value = resp.json().await.ok()?;
        meta.get("token_endpoint")
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    /// Mints + signs a `private_key_jwt` client assertion for `token_endpoint`.
    fn client_assertion(&self, token_endpoint: &str) -> Option<String> {
        let now = chrono::Utc::now().timestamp();
        let claims = serde_json::json!({
            "iss": self.client_id,
            "sub": self.client_id,
            "aud": token_endpoint,
            "exp": now + 300,
            "iat": now,
            "jti": uuid::Uuid::new_v4().to_string(),
        });
        let mut header = Header::new(self.signing_alg);
        header.typ = Some("JWT".to_string());
        jsonwebtoken::encode(&header, &claims, &self.encoding_key).ok()
    }

    async fn request_token(&self, token_endpoint: &str, scope: &str) -> Option<(String, Instant)> {
        let assertion = self.client_assertion(token_endpoint)?;
        let params = [
            ("grant_type", "client_credentials"),
            ("scope", scope),
            (
                "client_assertion_type",
                "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
            ),
            ("client_assertion", &assertion),
        ];
        let resp = self
            .client
            .post(token_endpoint)
            .form(&params)
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body: Value = resp.json().await.ok()?;
        let token = body
            .get("access_token")
            .and_then(|v| v.as_str())?
            .to_string();
        let expires_in = body
            .get("expires_in")
            .and_then(|v| v.as_u64())
            .unwrap_or(300);
        // Refresh ~30s before expiry.
        let ttl = expires_in.saturating_sub(30).max(1);
        Some((token, Instant::now() + Duration::from_secs(ttl)))
    }
}

#[async_trait]
impl FileTokenProvider for JwtClientCredentialsTokenProvider {
    async fn token(&self, oauth_metadata_urls: &[String], scope: &str) -> Option<String> {
        let metadata_url = oauth_metadata_urls.first()?;
        let token_endpoint = self.discover_token_endpoint(metadata_url).await?;
        let cache_key = (token_endpoint.clone(), scope.to_string());

        // Serve a cached, unexpired token.
        {
            let cache = self.cache.lock().await;
            if let Some((tok, exp)) = cache.get(&cache_key) {
                if Instant::now() < *exp {
                    return Some(tok.clone());
                }
            }
        }

        let (token, exp) = self.request_token(&token_endpoint, scope).await?;
        self.cache
            .lock()
            .await
            .insert(cache_key, (token.clone(), exp));
        Some(token)
    }
}
