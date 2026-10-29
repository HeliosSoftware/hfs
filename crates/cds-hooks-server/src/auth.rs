//! Who is calling. Every call carries a JWT the EMR signed (CDS Hooks 2.0,
//! "Trusting CDS Clients"): `iss` names the client, the signature is checked
//! against the keys at the `jku` the registry holds for that `iss` (never one
//! taken from the token), `aud` must be one of this service's own URLs, `exp`
//! must be in the future and `jti` must not have been seen before.
//!
//! A valid token from an issuer the registry does not know is refused: a
//! well-formed JWT from an EMR that is not a client is still an unauthorized
//! call.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::{HeaderMap, header};
use base64::Engine;
use helios_auth::JwksCache;
use jsonwebtoken::{Algorithm, Validation, decode, decode_header};
use serde::Deserialize;
use tracing::{debug, warn};

use crate::clients::{Client, ClientRegistry};

/// Seconds of clock skew tolerated on `exp`, `nbf` and `iat`.
const LEEWAY_SECS: u64 = 60;
/// How many seen `jti` values are kept before expired ones are purged.
const REPLAY_PURGE_AT: usize = 10_000;
/// Seconds between two fetches of one client's JWKS when a `kid` is unknown.
const JWKS_MIN_REFRESH_SECS: u64 = 60;

/// The client a request was authenticated as.
#[derive(Debug, Clone)]
pub struct ClientContext {
    pub client: Client,
    /// True when the call carried no token and the registry's `anonymous_client` took it.
    pub anonymous: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum Refusal {
    #[error("no bearer token")]
    Missing,
    #[error("the bearer token could not be read")]
    Malformed,
    #[error("the token's issuer is not a registered client")]
    UnknownIssuer,
    #[error("the token's signing key is not known")]
    UnknownKey,
    #[error("the token is not valid")]
    Invalid,
    #[error("the token was already used")]
    Replay,
    #[error("the call names a FHIR server that is not the client's")]
    ForeignFhirServer,
}

#[derive(Debug, Deserialize)]
struct Claims {
    iss: String,
    jti: String,
    exp: u64,
}

pub struct Authenticator {
    registry: Arc<ClientRegistry>,
    jwks: Mutex<HashMap<String, Arc<JwksCache>>>,
    seen: Mutex<HashMap<String, u64>>,
}

impl Authenticator {
    pub fn new(registry: Arc<ClientRegistry>) -> Self {
        Self {
            registry,
            jwks: Mutex::new(HashMap::new()),
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// Resolves the caller from the request headers.
    pub async fn authenticate(&self, headers: &HeaderMap) -> Result<ClientContext, Refusal> {
        let Some(token) = bearer(headers) else {
            return self.anonymous();
        };
        let header = decode_header(token).map_err(|_| Refusal::Malformed)?;
        let algorithm = match header.alg {
            Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::ES256
            | Algorithm::ES384 => header.alg,
            _ => return Err(Refusal::Malformed),
        };
        let kid = header.kid.ok_or(Refusal::Malformed)?;
        let iss = unverified_issuer(token).ok_or(Refusal::Malformed)?;
        let client = self
            .registry
            .by_iss(&iss)
            .cloned()
            .ok_or(Refusal::UnknownIssuer)?;
        let jku = client.jku.clone().ok_or(Refusal::UnknownIssuer)?;

        let cache = self.jwks_for(&iss, &jku);
        let key = cache.get_key(&kid).await.map_err(|e| {
            warn!(client = %client.id, kid, error = %e, "signing key not available");
            Refusal::UnknownKey
        })?;

        let mut validation = Validation::new(algorithm);
        validation.leeway = LEEWAY_SECS;
        validation.set_issuer(&[iss.as_str()]);
        validation.set_audience(&self.registry.audience);
        validation.set_required_spec_claims(&["exp", "iss", "aud"]);
        let claims = decode::<Claims>(token, &key, &validation)
            .map_err(|e| {
                debug!(client = %client.id, error = %e, "token refused");
                Refusal::Invalid
            })?
            .claims;
        self.record_jti(&claims)?;
        Ok(ClientContext {
            client,
            anonymous: false,
        })
    }

    /// Whether a call's `fhirServer` is the client's own.
    pub fn check_fhir_server(
        ctx: &ClientContext,
        fhir_server: Option<&str>,
    ) -> Result<(), Refusal> {
        match (&ctx.client.fhir_server, fhir_server) {
            (Some(own), Some(named))
                if own.trim_end_matches('/') != named.trim_end_matches('/') =>
            {
                Err(Refusal::ForeignFhirServer)
            }
            _ => Ok(()),
        }
    }

    fn anonymous(&self) -> Result<ClientContext, Refusal> {
        let id = self
            .registry
            .anonymous_client
            .as_deref()
            .ok_or(Refusal::Missing)?;
        let client = self.registry.by_id(id).cloned().ok_or(Refusal::Missing)?;
        Ok(ClientContext {
            client,
            anonymous: true,
        })
    }

    fn jwks_for(&self, iss: &str, jku: &str) -> Arc<JwksCache> {
        let mut caches = self.jwks.lock().expect("jwks caches");
        caches
            .entry(iss.to_string())
            .or_insert_with(|| Arc::new(JwksCache::new(jku, JWKS_MIN_REFRESH_SECS)))
            .clone()
    }

    fn record_jti(&self, claims: &Claims) -> Result<(), Refusal> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let key = format!("{}|{}", claims.iss, claims.jti);
        let mut seen = self.seen.lock().expect("replay cache");
        if seen.len() >= REPLAY_PURGE_AT {
            seen.retain(|_, exp| *exp + LEEWAY_SECS > now);
        }
        if let Some(exp) = seen.get(&key)
            && *exp + LEEWAY_SECS > now
        {
            return Err(Refusal::Replay);
        }
        seen.insert(key, claims.exp);
        Ok(())
    }
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty()).then(|| token.trim())
}

/// The `iss` claim read before verification, only to pick which keys verify it.
fn unverified_issuer(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get("iss")?.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_bearer_token_and_the_unverified_issuer() {
        let mut headers = HeaderMap::new();
        assert!(bearer(&headers).is_none());
        headers.insert(header::AUTHORIZATION, "Bearer abc.def.ghi".parse().unwrap());
        assert_eq!(bearer(&headers), Some("abc.def.ghi"));
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"iss":"https://emr.example.org","jti":"1"}"#);
        assert_eq!(
            unverified_issuer(&format!("h.{payload}.s")).as_deref(),
            Some("https://emr.example.org")
        );
        assert!(unverified_issuer("not-a-token").is_none());
    }

    #[tokio::test]
    async fn without_a_token_only_the_anonymous_client_passes() {
        let strict = Authenticator::new(Arc::new(
            ClientRegistry::from_yaml("clients:\n  - id: x\n").unwrap(),
        ));
        assert!(matches!(
            strict.authenticate(&HeaderMap::new()).await,
            Err(Refusal::Missing)
        ));
        let dev = Authenticator::new(Arc::new(
            ClientRegistry::from_yaml("anonymous_client: x\nclients:\n  - id: x\n").unwrap(),
        ));
        let ctx = dev.authenticate(&HeaderMap::new()).await.unwrap();
        assert!(ctx.anonymous);
        assert_eq!(ctx.client.id, "x");
    }

    #[test]
    fn a_foreign_fhir_server_is_refused_only_when_the_client_has_one_on_file() {
        let with = ClientContext {
            client: Client {
                id: "x".into(),
                iss: None,
                jku: None,
                fhir_server: Some("https://fhir.example.org/".into()),
                services: vec![],
                prefetch: HashMap::new(),
            },
            anonymous: false,
        };
        assert!(Authenticator::check_fhir_server(&with, Some("https://fhir.example.org")).is_ok());
        assert!(
            Authenticator::check_fhir_server(&with, Some("https://other.example.org")).is_err()
        );
        assert!(Authenticator::check_fhir_server(&with, None).is_ok());
        let without = ClientContext {
            client: Client {
                fhir_server: None,
                ..with.client.clone()
            },
            anonymous: false,
        };
        assert!(Authenticator::check_fhir_server(&without, Some("https://any")).is_ok());
    }
}
