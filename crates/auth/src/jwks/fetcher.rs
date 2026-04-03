use std::collections::HashMap;
use std::time::Duration;

use jsonwebtoken::DecodingKey;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::error::AuthError;

/// Response from a JWKS endpoint.
pub struct JwksResponse {
    /// Decoding keys indexed by key ID (`kid`).
    pub keys: HashMap<String, DecodingKey>,
    /// Cache duration parsed from HTTP `Cache-Control: max-age=N`.
    pub max_age: Option<Duration>,
}

/// A single JWK from a JWKS endpoint.
#[derive(Debug, Deserialize)]
struct Jwk {
    kid: Option<String>,
    kty: String,
    #[serde(rename = "use")]
    key_use: Option<String>,
    #[allow(dead_code)]
    alg: Option<String>,
    // RSA fields
    n: Option<String>,
    e: Option<String>,
    // EC fields
    crv: Option<String>,
    x: Option<String>,
    y: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JwksDocument {
    keys: Vec<Jwk>,
}

/// Fetches and parses JWKS documents from HTTP endpoints.
#[derive(Clone)]
pub struct JwksFetcher {
    client: reqwest::Client,
}

impl JwksFetcher {
    /// Create a new JWKS fetcher with a shared HTTP client.
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to build HTTP client");
        Self { client }
    }

    /// Fetch and parse a JWKS document from the given URL.
    pub async fn fetch(&self, url: &str) -> Result<JwksResponse, AuthError> {
        debug!(url, "Fetching JWKS");

        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| AuthError::JwksFetchError(format!("HTTP request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(AuthError::JwksFetchError(format!(
                "JWKS endpoint returned status {}",
                response.status()
            )));
        }

        // Parse Cache-Control max-age
        let max_age = response
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_max_age);

        let doc: JwksDocument = response
            .json()
            .await
            .map_err(|e| AuthError::JwksFetchError(format!("Failed to parse JWKS JSON: {}", e)))?;

        let mut keys = HashMap::new();
        for jwk in doc.keys {
            // Only include signing keys (use=sig or unspecified)
            if let Some(ref key_use) = jwk.key_use {
                if key_use != "sig" {
                    continue;
                }
            }

            let kid = match jwk.kid {
                Some(ref kid) => kid.clone(),
                None => {
                    warn!("Skipping JWK without kid");
                    continue;
                }
            };

            match build_decoding_key(&jwk) {
                Ok(key) => {
                    debug!(kid = %kid, kty = %jwk.kty, "Loaded JWK");
                    keys.insert(kid, key);
                }
                Err(e) => {
                    warn!(kid = %kid, error = %e, "Failed to parse JWK, skipping");
                }
            }
        }

        debug!(key_count = keys.len(), "JWKS fetch complete");
        Ok(JwksResponse { keys, max_age })
    }
}

impl Default for JwksFetcher {
    fn default() -> Self {
        Self::new()
    }
}

fn build_decoding_key(jwk: &Jwk) -> Result<DecodingKey, AuthError> {
    match jwk.kty.as_str() {
        "RSA" => {
            let n = jwk
                .n
                .as_ref()
                .ok_or_else(|| AuthError::JwksFetchError("RSA JWK missing 'n'".to_string()))?;
            let e = jwk
                .e
                .as_ref()
                .ok_or_else(|| AuthError::JwksFetchError("RSA JWK missing 'e'".to_string()))?;
            DecodingKey::from_rsa_components(n, e)
                .map_err(|e| AuthError::JwksFetchError(format!("Invalid RSA key: {}", e)))
        }
        "EC" => {
            let x = jwk
                .x
                .as_ref()
                .ok_or_else(|| AuthError::JwksFetchError("EC JWK missing 'x'".to_string()))?;
            let y = jwk
                .y
                .as_ref()
                .ok_or_else(|| AuthError::JwksFetchError("EC JWK missing 'y'".to_string()))?;
            let crv = jwk.crv.as_deref().unwrap_or("P-256");
            DecodingKey::from_ec_components(x, y).map_err(|e| {
                AuthError::JwksFetchError(format!("Invalid EC key (crv={}): {}", crv, e))
            })
        }
        other => Err(AuthError::JwksFetchError(format!(
            "Unsupported key type: {}",
            other
        ))),
    }
}

/// Parse `max-age=N` from a Cache-Control header value.
fn parse_max_age(cache_control: &str) -> Option<Duration> {
    for directive in cache_control.split(',') {
        let directive = directive.trim();
        if let Some(val) = directive.strip_prefix("max-age=") {
            if let Ok(secs) = val.trim().parse::<u64>() {
                return Some(Duration::from_secs(secs));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_max_age() {
        assert_eq!(
            parse_max_age("max-age=3600"),
            Some(Duration::from_secs(3600))
        );
        assert_eq!(
            parse_max_age("public, max-age=86400, must-revalidate"),
            Some(Duration::from_secs(86400))
        );
        assert_eq!(parse_max_age("no-cache"), None);
        assert_eq!(parse_max_age(""), None);
    }
}
