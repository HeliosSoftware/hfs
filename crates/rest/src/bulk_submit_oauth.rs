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
//!
//! **Destinations:** the metadata URL, the manifest URL and every file URL come
//! from the submitter, so a token is only ever presented to the origin of the
//! token endpoint that issued it, or to an origin the operator listed in
//! `HFS_BULK_SUBMIT_TRUSTED_PROVIDERS` ([`TrustedProviders`]). When that list is
//! set, discovery and the token request themselves are confined to it too.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use helios_persistence::core::bulk_submit_input::redact_url;
use helios_persistence::core::{FileTokenError, FileTokenProvider};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::Value;
use tokio::sync::Mutex;

/// Origins (`scheme://host[:port]`) the operator trusts as bulk-submit Data
/// Providers: the only origins `$bulk-submit` fetches from at all (the
/// `manifestUrl`, the `oauthMetadataUrl`, every manifest output URL and
/// every redirect), where HFS presents its client assertion, and where a
/// token obtained there may be sent. Parsed from
/// `HFS_BULK_SUBMIT_TRUSTED_PROVIDERS`.
///
/// Empty (the default) lets the submitter name any origin and confines a
/// token to the origin of the token endpoint that issued it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustedProviders(Vec<String>);

/// Redirect hops followed, as reqwest's default policy allows.
const MAX_REDIRECTS: usize = 10;

impl TrustedProviders {
    /// Parses one origin per entry. An entry must be an absolute URL with a
    /// host and nothing after it: a path, query or fragment is refused rather
    /// than silently widened to the whole origin.
    pub fn parse<S: AsRef<str>>(entries: impl IntoIterator<Item = S>) -> Result<Self, String> {
        let mut origins = Vec::new();
        for entry in entries {
            let entry = entry.as_ref().trim();
            if entry.is_empty() {
                continue;
            }
            let url = reqwest::Url::parse(entry)
                .map_err(|_| format!("'{entry}' is not an absolute URL"))?;
            if !url.origin().is_tuple() {
                return Err(format!("'{entry}' has no host"));
            }
            if !matches!(url.path(), "" | "/") || url.query().is_some() || url.fragment().is_some()
            {
                return Err(format!(
                    "'{entry}' names more than an origin; list only scheme://host[:port]"
                ));
            }
            let origin = url.origin().ascii_serialization();
            if !origins.contains(&origin) {
                origins.push(origin);
            }
        }
        Ok(Self(origins))
    }

    /// `true` when no origin is listed.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether `origin` is listed.
    pub fn contains(&self, origin: &str) -> bool {
        self.0.iter().any(|o| o == origin)
    }

    /// Whether `$bulk-submit` may send a request to `url`: anywhere when no
    /// origin is listed, otherwise only to a listed origin. A URL without an
    /// origin (an opaque scheme) is never permitted by a configured list.
    pub fn permits_url(&self, url: &str) -> bool {
        self.is_empty() || origin_of(url).is_some_and(|origin| self.contains(&origin))
    }

    /// The redirect policy of a client that sends `$bulk-submit` requests:
    /// with a list configured a hop is followed only to a listed origin,
    /// otherwise reqwest's default policy applies. Both the file fetcher and
    /// the OAuth client use it, so neither the manifest and file GETs nor the
    /// metadata GET and the token POST (whose body is the signed client
    /// assertion) leave the listed origins by way of a redirect.
    pub(crate) fn redirect_policy(&self) -> reqwest::redirect::Policy {
        if self.is_empty() {
            return reqwest::redirect::Policy::default();
        }
        let trusted = self.clone();
        reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if trusted.permits_url(attempt.url().as_str()) {
                attempt.follow()
            } else {
                attempt.error(
                    "redirected to an origin not listed in HFS_BULK_SUBMIT_TRUSTED_PROVIDERS",
                )
            }
        })
    }

    /// With a list configured, refuses an origin outside it.
    fn require(&self, origin: &str) -> Result<(), FileTokenError> {
        if self.is_empty() || self.contains(origin) {
            Ok(())
        } else {
            Err(FileTokenError::UntrustedProvider {
                origin: origin.to_string(),
            })
        }
    }
}

/// The `scheme://host[:port]` of an absolute URL, or `None` when it has no
/// host (an opaque origin such as `data:`), the form every comparison here
/// is made on.
pub(crate) fn origin_of(url: &str) -> Option<String> {
    let origin = reqwest::Url::parse(url).ok()?.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

/// Acquires read-scoped bearer tokens via SMART Backend Services `client_credentials`.
pub struct JwtClientCredentialsTokenProvider {
    client: reqwest::Client,
    client_id: String,
    signing_alg: Algorithm,
    encoding_key: EncodingKey,
    /// RFC 7638 thumbprint of the public key, used as `kid` in assertion headers.
    kid: Option<String>,
    trusted_providers: TrustedProviders,
    /// Cache of `(token_endpoint, scope) -> (token, expiry)`.
    cache: Mutex<HashMap<(String, String), (String, Instant)>>,
}

impl JwtClientCredentialsTokenProvider {
    /// Builds a provider from a `client_id`, a PEM private key, and a signing alg
    /// (`ES384` or `RS384`) that trusts no provider origin beyond the token
    /// endpoint's own. Returns `None` if the key can't be parsed.
    pub fn new(client_id: &str, private_key_pem: &str, signing_alg: &str) -> Option<Arc<Self>> {
        Self::new_with_trusted_providers(
            client_id,
            private_key_pem,
            signing_alg,
            TrustedProviders::default(),
        )
    }

    /// [`Self::new`] with the operator's trusted provider origins.
    pub fn new_with_trusted_providers(
        client_id: &str,
        private_key_pem: &str,
        signing_alg: &str,
        trusted_providers: TrustedProviders,
    ) -> Option<Arc<Self>> {
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
        let kid = derive_public_jwk(private_key_pem, signing_alg)
            .and_then(|jwk| jwk.get("kid").and_then(|v| v.as_str()).map(String::from));
        Some(Arc::new(Self {
            // `Client::new()` panics the same way when the TLS backend
            // cannot start.
            client: reqwest::Client::builder()
                .redirect(trusted_providers.redirect_policy())
                .build()
                .expect("failed to build the bulk-submit OAuth client"),
            client_id: client_id.to_string(),
            signing_alg: alg,
            encoding_key: key,
            kid,
            trusted_providers,
            cache: Mutex::new(HashMap::new()),
        }))
    }

    /// Resolves the token endpoint from a SMART/OAuth metadata URL.
    async fn discover_token_endpoint(
        &self,
        oauth_metadata_url: &str,
    ) -> Result<String, FileTokenError> {
        let shown = redact_url(oauth_metadata_url);
        let unavailable = |reason: String| FileTokenError::Unavailable(reason);
        let resp = self
            .client
            .get(oauth_metadata_url)
            .send()
            .await
            .map_err(|e| unavailable(format!("metadata GET {shown} failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(unavailable(format!(
                "metadata GET {shown} returned HTTP {}",
                resp.status()
            )));
        }
        let meta: Value = resp
            .json()
            .await
            .map_err(|e| unavailable(format!("metadata at {shown} is not JSON: {e}")))?;
        meta.get("token_endpoint")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| unavailable(format!("metadata at {shown} names no token_endpoint")))
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
        header.kid = self.kid.clone();
        jsonwebtoken::encode(&header, &claims, &self.encoding_key).ok()
    }

    async fn request_token(
        &self,
        token_endpoint: &str,
        scope: &str,
    ) -> Result<(String, Instant), FileTokenError> {
        let shown = redact_url(token_endpoint);
        let unavailable = |reason: String| FileTokenError::Unavailable(reason);
        let assertion = self
            .client_assertion(token_endpoint)
            .ok_or_else(|| unavailable("the client assertion could not be signed".to_string()))?;
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
            .map_err(|e| unavailable(format!("token POST {shown} failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(unavailable(format!(
                "token POST {shown} returned HTTP {}",
                resp.status()
            )));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| unavailable(format!("token response from {shown} is not JSON: {e}")))?;
        let token = body
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| unavailable(format!("token response from {shown} has no access_token")))?
            .to_string();
        let expires_in = body
            .get("expires_in")
            .and_then(|v| v.as_u64())
            .unwrap_or(300);
        // Refresh ~30s before expiry.
        let ttl = expires_in.saturating_sub(30).max(1);
        Ok((token, Instant::now() + Duration::from_secs(ttl)))
    }
}

#[async_trait]
impl FileTokenProvider for JwtClientCredentialsTokenProvider {
    async fn token(
        &self,
        oauth_metadata_urls: &[String],
        scope: &str,
        destination: &str,
    ) -> Result<String, FileTokenError> {
        let metadata_url = oauth_metadata_urls.first().ok_or_else(|| {
            FileTokenError::Unavailable("the submission names no oauthMetadataUrl".to_string())
        })?;
        let metadata_origin = origin_of(metadata_url).ok_or_else(|| {
            FileTokenError::Unavailable(format!(
                "oauthMetadataUrl {} has no origin",
                redact_url(metadata_url)
            ))
        })?;
        self.trusted_providers.require(&metadata_origin)?;

        let token_endpoint = self.discover_token_endpoint(metadata_url).await?;
        let token_origin = origin_of(&token_endpoint).ok_or_else(|| {
            FileTokenError::Unavailable(format!(
                "token_endpoint {} has no origin",
                redact_url(&token_endpoint)
            ))
        })?;
        self.trusted_providers.require(&token_origin)?;

        let destination_origin = origin_of(destination).ok_or_else(|| {
            FileTokenError::Unavailable(format!(
                "destination {} has no origin",
                redact_url(destination)
            ))
        })?;
        if destination_origin != token_origin
            && !self.trusted_providers.contains(&destination_origin)
        {
            return Err(FileTokenError::UntrustedDestination {
                destination: destination_origin,
                token_origin,
            });
        }

        let cache_key = (token_endpoint.clone(), scope.to_string());

        // Serve a cached, unexpired token.
        {
            let cache = self.cache.lock().await;
            if let Some((tok, exp)) = cache.get(&cache_key) {
                if Instant::now() < *exp {
                    return Ok(tok.clone());
                }
            }
        }

        let (token, exp) = self.request_token(&token_endpoint, scope).await?;
        self.cache
            .lock()
            .await
            .insert(cache_key, (token.clone(), exp));
        Ok(token)
    }
}

/// Derives the public JWK (with RFC 7638 thumbprint as `kid`) from a PEM private key.
///
/// Supports **ES384 (P-384) only**; the key may be PKCS#8 or SEC1 PEM. The
/// `kid` is the RFC 7638 thumbprint: the base64url (no padding) SHA-256 of the
/// canonical `{"crv","kty","x","y"}` members.
///
/// Returns `None` in three cases:
///
/// - `RS384`, deliberately: deriving the RSA public key would need the `rsa`
///   crate, which carries RUSTSEC-2023-0071 (#560). An RS384 deployment
///   therefore publishes no key and signs without a `kid`.
/// - Any other algorithm.
/// - A key that cannot be parsed as a P-384 key.
pub(crate) fn derive_public_jwk(pem: &str, alg: &str) -> Option<Value> {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use sha2::{Digest, Sha256};

    match alg {
        "ES384" => {
            use p384::elliptic_curve::sec1::ToSec1Point;
            use p384::pkcs8::DecodePrivateKey;

            let secret = p384::SecretKey::from_pkcs8_pem(pem)
                .or_else(|_| p384::SecretKey::from_sec1_pem(pem))
                .ok()?;
            let point = secret.public_key().to_sec1_point(false);
            let x = URL_SAFE_NO_PAD.encode(point.x()?);
            let y = URL_SAFE_NO_PAD.encode(point.y()?);
            // RFC 7638 §3.3: required EC members in lexicographic order.
            let canonical = format!(r#"{{"crv":"P-384","kty":"EC","x":"{x}","y":"{y}"}}"#);
            let kid = URL_SAFE_NO_PAD.encode(Sha256::digest(canonical.as_bytes()));
            Some(serde_json::json!({
                "kty": "EC",
                "crv": "P-384",
                "x": x,
                "y": y,
                "kid": kid,
                "use": "sig",
                "alg": "ES384",
            }))
        }
        // RS384 derivation is deliberately not offered: the only pure-Rust
        // RSA implementation carries RUSTSEC-2023-0071 (Marvin Attack timing
        // sidechannel) with no fixed release — the same reason jwe.rs rejects
        // RSA-OAEP. An RS384 deployment registers its public key with the
        // recipient out-of-band; the JWKS endpoint serves an empty set, as
        // its handler documents.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway EC P-384 (secp384r1) PKCS#8 private key, generated solely for
    // these tests — not used anywhere else.
    const TEST_ES384_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIG2AgEAMBAGByqGSM49AgEGBSuBBAAiBIGeMIGbAgEBBDDL2uAdbDMvTFQ/r+Tz\n\
45fPr+tTiZPaq7mKcF2g1fmOCRMSdkSMLaRBzWbIwOE8GxOhZANiAASjv7NSBHxr\n\
k5K6g75uWMf/NsVu7ay2lO8vuu9EC12UBjDQ+297+9I7+IlSzRmoWKUT0fVrOeoB\n\
gMMPIjEueYREkCGj6cILnLaP5hAey+Q7JjRgVO6Xqf0u2uCO6yn8UTU=\n\
-----END PRIVATE KEY-----\n";

    #[test]
    fn trusted_providers_parse_origins_only() {
        let trusted = TrustedProviders::parse([
            " https://auth.example.org ",
            "https://files.example.org:8443/",
            "",
            "HTTPS://Auth.Example.org",
        ])
        .expect("origins parse");
        assert_eq!(
            trusted,
            TrustedProviders(vec![
                "https://auth.example.org".to_string(),
                "https://files.example.org:8443".to_string(),
            ])
        );
        assert!(trusted.contains("https://auth.example.org"));
        assert!(!trusted.contains("http://auth.example.org"));
        assert!(!trusted.contains("https://auth.example.org:8443"));

        assert!(TrustedProviders::parse(["", "  "]).unwrap().is_empty());
        assert!(
            TrustedProviders::parse(Vec::<String>::new())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn trusted_providers_refuse_paths_and_hostless_entries() {
        for entry in [
            "https://auth.example.org/realms/fhir",
            "https://auth.example.org/?x=1",
            "https://auth.example.org/#frag",
            "auth.example.org",
            "data:text/plain,hi",
            "not a url",
        ] {
            let err = TrustedProviders::parse([entry]).expect_err(entry);
            assert!(err.contains(entry), "{err}");
        }
    }

    #[test]
    fn trusted_providers_require_only_when_configured() {
        let none = TrustedProviders::default();
        assert_eq!(none.require("https://anything.example"), Ok(()));

        let some = TrustedProviders::parse(["https://auth.example.org"]).unwrap();
        assert_eq!(some.require("https://auth.example.org"), Ok(()));
        assert_eq!(
            some.require("https://other.example.org"),
            Err(FileTokenError::UntrustedProvider {
                origin: "https://other.example.org".to_string()
            })
        );
    }

    #[test]
    fn origin_of_reduces_a_url_to_scheme_host_port() {
        assert_eq!(
            origin_of("https://Files.Example.org/bulk/a.ndjson?sig=1#x").as_deref(),
            Some("https://files.example.org")
        );
        assert_eq!(
            origin_of("http://127.0.0.1:8080/token").as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(
            origin_of("https://a.example:443/x").as_deref(),
            Some("https://a.example"),
            "the default port is dropped"
        );
        assert_eq!(origin_of("data:text/plain,hi"), None);
        assert_eq!(origin_of("/relative"), None);
    }

    #[test]
    fn test_new_rejects_unsupported_alg() {
        assert!(JwtClientCredentialsTokenProvider::new("cid", TEST_ES384_PEM, "HS256").is_none());
    }

    #[test]
    fn test_new_rejects_malformed_es384_key() {
        assert!(JwtClientCredentialsTokenProvider::new("cid", "not-a-pem", "ES384").is_none());
    }

    #[test]
    fn test_new_rejects_malformed_rs384_key() {
        assert!(JwtClientCredentialsTokenProvider::new("cid", "not-a-pem", "RS384").is_none());
    }

    #[test]
    fn test_new_accepts_es384_key_and_mints_assertion() {
        let provider = JwtClientCredentialsTokenProvider::new("my-client", TEST_ES384_PEM, "ES384")
            .expect("valid ES384 key");
        assert_eq!(provider.signing_alg, Algorithm::ES384);

        // The signed client assertion is a well-formed 3-segment compact JWS.
        let assertion = provider
            .client_assertion("https://idp.example.com/token")
            .expect("assertion minted");
        assert_eq!(assertion.split('.').count(), 3);
    }

    // ---- derive_public_jwk / client-assertion `kid` (#529) -----------------

    // The public point and RFC 7638 thumbprint of `TEST_ES384_PEM`, computed
    // independently of this crate: `openssl ec -text` for the public point,
    // then Python's `hashlib.sha256` over
    // `{"crv":"P-384","kty":"EC","x":"<x>","y":"<y>"}`, base64url, no padding.
    const TEST_ES384_X: &str = "o7-zUgR8a5OSuoO-bljH_zbFbu2stpTvL7rvRAtdlAYw0Ptve_vSO_iJUs0ZqFil";
    const TEST_ES384_Y: &str = "E9H1aznqAYDDDyIxLnmERJAho-nCC5y2j-YQHsvkOyY0YFTul6n9Ltrgjusp_FE1";
    const TEST_ES384_KID: &str = "RIm6zOzFR3cMAG9PzzAubmfag2C2EKZa4w_Xnb_fuQM";

    // An RSA-2048 PKCS#8 key generated for the Bulk Import UI tests
    // (`crates/ui/tests/bulk_import_http.rs`, `TEST_RSA_KEY`) and copied here
    // verbatim; it protects nothing.
    const TEST_RSA_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDH1Fo0WDU06NHP
2JGiQSqMn40gjDALiqnTgxBFi/9i+Th4cTKXCYMmcpzQKCyWIhgtNF8no672XQsD
O3zixg7YdGNJPdaO90lJmY8k7jm/jfGqzgnEYunmD1W1RY0OqkUo+Ni/4FVo/6uJ
lZFrP7lTE5ZsY90CZc1/isVbYWgwZnhcOCNQfm5EkeMgNPShizoJJpEgGadnVlP1
9sWEckO1lEY5Ed5fl6Rhb1EO+c7cCSug4WRATWSEP5pUhoulN6JUyxy7BAe9YZ5d
Lx7Du6SBDmG8K8lOzqvAA1vVqcA85JYBZeMd+sTYCmTsW//bnBAymZRKMcbosyjk
UdDZxR7VAgMBAAECggEAAPBerSqLwBHk5eSflh9zJ/1niYZTszqABi2/p6p/Xqpl
yxg79garxc0wTYAHQsC9RNt7yKQTj0aSRNWFMtzGHOK78rdXfYBnPpXczZePnw1u
j/Dv8LZpCtNwDksptJfWC6IcYD1IPl4ye5HZHe1CJuooCIJ3nYCOV3zhM31pszjo
b4r+cm6SDxFnuAYtH6ngvvsN+pGht/DyN6u1KY5d06JNQ0aDCuGsRayZ55jHJubU
1VRvqeBmcn4xkiWkvcfQuE72rbojoTCtupOaVmLjiOxd7F1IvNKKbbJIBbMlc7VX
dnzMTLF2MCCYYtRzZUIlbZ7qSVSSOh89SUA8aNg0iQKBgQD7PFUU910UfQ77wIEr
gOl062yskJMT97hfBl6HGTGfpmnHqS8L7ZZmTybNGznS4rZuiPC/uUEGMr01tnRD
GlVBpugfe/8AO6J2nUY9e1IBplvhwB7uuy8MpwmkL1CzbDcat+s5f7GDvlLK6u18
XKKtnK8GVaHzXXjSjyaFrE6iHwKBgQDLnnWw7SqW4+0BaenhWC51iHd2GeIQJQ0s
cXFs5N6+i87UVaqSErAdFMRt08ocieO5t2v03YT2hiYSFxxgdpGDEG18e4/T/sQW
T18fqo9iFaFmZ2Hzk3uRI2HG4FfYetwyv97nVEH0EibrKd/dSnZ8JbB87VR0LfhO
c4srqPnoiwKBgF3Db5GKnE+IOO5WMx8UVozPTFi/AFVEb6fvTZooGfAWgIYGq0tN
WYNHaRjFX3hIKoPoUcmMDyuMBjekp5Ffo5AEBb+yXEIu/3w7SDqr6rg46TPAqwq4
C2AyexOuoPTFn282UvC7qnmbr3SR5x4xyHj48A1yKiYUrYIP8PWUkChLAoGAWHkV
sjaa1s1aYc7fbKagKTmOjqZYb6NpwfHY0vPvROQCjohagPXVyA0J/J6VpyjS5hMo
uVC3QVawnBOmpNNgDo7Iw9n8eKSuFvON5Xh6rKexZYluKiPfAQVaqss34DwiCXsN
I36c2aw5dNzRBJoiOXc25FFK7OA8j/nscqANVlkCgYEAhgRQc/atd/GYETp2a8sn
8nYahjwwC4lU753b9/akn3GQcE9T65yRoeqFi63v7AAwxwIUFZnxv4CuI/sgSYEL
t+xX6yYqCQTVR3Y9XZgUWohxnpky0zdqat1BAJDzgRCWnZ75B++gDcLgbTYeqaVI
CbOPbIKiVSNWN6XfYk/ZDEU=
-----END PRIVATE KEY-----";

    /// A second P-384 key, distinct from `TEST_ES384_PEM`, built in the test
    /// from a fixed scalar so no key file or RNG is needed.
    fn other_p384_pem() -> String {
        use p384::pkcs8::EncodePrivateKey;
        p384::SecretKey::from_slice(&[0x22; 48])
            .expect("scalar below the P-384 group order")
            .to_pkcs8_pem(Default::default())
            .expect("encode PKCS#8")
            .to_string()
    }

    /// A P-256 key, for the "ES384 asked for, wrong curve supplied" case.
    fn p256_pem() -> String {
        use p256::pkcs8::EncodePrivateKey;
        p256::SecretKey::from_slice(&[0x11; 32])
            .expect("scalar below the P-256 group order")
            .to_pkcs8_pem(Default::default())
            .expect("encode PKCS#8")
            .to_string()
    }

    fn b64url_decode(s: &str) -> Vec<u8> {
        use base64::Engine as _;
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(s)
            .expect("base64url without padding")
    }

    fn jwk_str<'a>(jwk: &'a Value, member: &str) -> &'a str {
        jwk[member]
            .as_str()
            .unwrap_or_else(|| panic!("JWK member `{member}` is a string: {jwk}"))
    }

    /// RFC 7638 §3: SHA-256 over the JWK's required members only, in
    /// lexicographic order with no whitespace, base64url without padding.
    /// For an EC key the required members are `crv`, `kty`, `x` and `y`
    /// (§3.2). A `BTreeMap` serialises sorted and compact, so the canonical
    /// form is built differently from the production `format!`.
    fn rfc7638_thumbprint(jwk: &Value) -> String {
        use base64::Engine as _;
        use sha2::{Digest, Sha256};
        let required: std::collections::BTreeMap<&str, &str> = ["crv", "kty", "x", "y"]
            .into_iter()
            .map(|m| (m, jwk_str(jwk, m)))
            .collect();
        let canonical = serde_json::to_string(&required).expect("serialise");
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(canonical))
    }

    #[test]
    fn derive_public_jwk_publishes_the_es384_public_key() {
        let jwk = derive_public_jwk(TEST_ES384_PEM, "ES384").expect("ES384 key derives");

        assert_eq!(jwk["kty"], "EC");
        assert_eq!(jwk["crv"], "P-384");
        assert_eq!(jwk["use"], "sig");
        assert_eq!(jwk["alg"], "ES384");
        assert_eq!(b64url_decode(jwk_str(&jwk, "x")).len(), 48);
        assert_eq!(b64url_decode(jwk_str(&jwk, "y")).len(), 48);
        assert!(!jwk_str(&jwk, "kid").is_empty());

        // The public point is the one OpenSSL reports for the same key.
        assert_eq!(jwk["x"], TEST_ES384_X);
        assert_eq!(jwk["y"], TEST_ES384_Y);

        // Exactly the public members: nothing of the private scalar leaks.
        let mut members: Vec<&str> = jwk
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        members.sort_unstable();
        assert_eq!(members, ["alg", "crv", "kid", "kty", "use", "x", "y"]);
    }

    #[test]
    fn derive_public_jwk_kid_is_the_rfc7638_thumbprint() {
        let jwk = derive_public_jwk(TEST_ES384_PEM, "ES384").expect("ES384 key derives");

        // Recomputed from the JWK's own required members...
        assert_eq!(jwk_str(&jwk, "kid"), rfc7638_thumbprint(&jwk));
        // ...and pinned against a value computed outside this crate, so a
        // change that re-keys every registered `kid` cannot pass silently.
        assert_eq!(jwk["kid"], TEST_ES384_KID);
    }

    #[test]
    fn derive_public_jwk_is_stable_and_distinct_per_key() {
        let first = derive_public_jwk(TEST_ES384_PEM, "ES384").unwrap();
        let again = derive_public_jwk(TEST_ES384_PEM, "ES384").unwrap();
        assert_eq!(first, again, "same PEM, same JWK");

        let other_pem = other_p384_pem();
        let other = derive_public_jwk(&other_pem, "ES384").expect("second key derives");
        assert_ne!(first["kid"], other["kid"], "different keys, different kid");
        assert_ne!(first["x"], other["x"]);
        assert_eq!(other["kid"], rfc7638_thumbprint(&other).as_str());
    }

    #[test]
    fn derive_public_jwk_reads_pkcs8_and_sec1_pem_alike() {
        let pkcs8 = other_p384_pem();
        let sec1 = p384::SecretKey::from_slice(&[0x22; 48])
            .unwrap()
            .to_sec1_pem(Default::default())
            .expect("encode SEC1")
            .to_string();
        assert!(sec1.contains("BEGIN EC PRIVATE KEY"));
        assert_eq!(
            derive_public_jwk(&pkcs8, "ES384").expect("PKCS#8"),
            derive_public_jwk(&sec1, "ES384").expect("SEC1"),
        );
    }

    /// The documented RS384 limit, pinned: HFS publishes no key for an RS384
    /// deployment, even with a valid RSA key configured. RSA derivation was
    /// removed because the only pure-Rust RSA implementation carries
    /// RUSTSEC-2023-0071 (Marvin Attack timing sidechannel) with no fixed
    /// release (#560). If RS384 derivation is ever added, this test should
    /// change with it — deliberately, not by accident.
    #[test]
    fn derive_public_jwk_publishes_nothing_for_rs384_by_design() {
        assert!(derive_public_jwk(TEST_RSA_PEM, "RS384").is_none());
        // The algorithm decides, not the key: an EC key labelled RS384 is
        // not published either.
        assert!(derive_public_jwk(TEST_ES384_PEM, "RS384").is_none());
    }

    #[test]
    fn derive_public_jwk_rejects_bad_input_without_panicking() {
        // Not a PEM at all, empty, and a PEM that stops short.
        assert!(derive_public_jwk("not-a-pem", "ES384").is_none());
        assert!(derive_public_jwk("", "ES384").is_none());
        let truncated = &TEST_ES384_PEM[..TEST_ES384_PEM.len() / 2];
        assert!(derive_public_jwk(truncated, "ES384").is_none());

        // ES384 asked for, but the key is on another curve or another family.
        assert!(derive_public_jwk(&p256_pem(), "ES384").is_none());
        assert!(derive_public_jwk(TEST_RSA_PEM, "ES384").is_none());

        // An algorithm the signer does not offer.
        assert!(derive_public_jwk(TEST_ES384_PEM, "HS256").is_none());
        assert!(derive_public_jwk(TEST_ES384_PEM, "").is_none());
    }

    /// A signature a recipient must be able to check: sign through the
    /// production path, then verify with a key built from nothing but the
    /// published JWK.
    #[test]
    fn client_assertion_verifies_against_the_published_jwk() {
        use jsonwebtoken::{DecodingKey, Validation, jwk::Jwk};

        let endpoint = "https://idp.example.com/token";
        let provider = JwtClientCredentialsTokenProvider::new("my-client", TEST_ES384_PEM, "ES384")
            .expect("valid ES384 key");
        let assertion = provider
            .client_assertion(endpoint)
            .expect("assertion minted");

        // What the JWKS endpoint serves, parsed by a standard JWK parser.
        let published = derive_public_jwk(TEST_ES384_PEM, "ES384").unwrap();
        let jwk: Jwk = serde_json::from_value(published.clone()).expect("a well-formed JWK");
        let key = DecodingKey::from_jwk(&jwk).expect("decoding key from the JWK alone");

        let mut validation = Validation::new(Algorithm::ES384);
        validation.set_audience(&[endpoint]);
        let verified = jsonwebtoken::decode::<Value>(&assertion, &key, &validation)
            .expect("the assertion verifies against the published key");

        assert_eq!(verified.header.alg, Algorithm::ES384);
        assert_eq!(verified.header.typ.as_deref(), Some("JWT"));
        assert_eq!(
            verified.header.kid.as_deref(),
            Some(jwk_str(&published, "kid")),
            "header kid names the published key"
        );
        assert_eq!(verified.claims["iss"], "my-client");
        assert_eq!(verified.claims["sub"], "my-client");
        assert_eq!(verified.claims["aud"], endpoint);
    }

    #[test]
    fn client_assertion_does_not_verify_against_another_keys_jwk() {
        use jsonwebtoken::{DecodingKey, Validation, errors::ErrorKind, jwk::Jwk};

        let endpoint = "https://idp.example.com/token";
        let provider = JwtClientCredentialsTokenProvider::new("my-client", TEST_ES384_PEM, "ES384")
            .expect("valid ES384 key");
        let assertion = provider
            .client_assertion(endpoint)
            .expect("assertion minted");

        let other_pem = other_p384_pem();
        let other = derive_public_jwk(&other_pem, "ES384").unwrap();
        let jwk: Jwk = serde_json::from_value(other.clone()).unwrap();
        let key = DecodingKey::from_jwk(&jwk).unwrap();

        let mut validation = Validation::new(Algorithm::ES384);
        validation.set_audience(&[endpoint]);
        let err = jsonwebtoken::decode::<Value>(&assertion, &key, &validation)
            .expect_err("another key's JWK must not verify the assertion");
        assert_eq!(*err.kind(), ErrorKind::InvalidSignature);

        // And the header's kid does not point at that other key either.
        let header = jsonwebtoken::decode_header(&assertion).unwrap();
        assert_ne!(header.kid.as_deref(), Some(jwk_str(&other, "kid")));
    }

    /// The documented RS384 limit on the signing side: no key is derivable, so
    /// the assertion header carries no `kid`. The recipient has to be given
    /// the public key out of band (#529, #560).
    #[test]
    fn rs384_client_assertion_carries_no_kid_by_design() {
        let provider = JwtClientCredentialsTokenProvider::new("my-client", TEST_RSA_PEM, "RS384")
            .expect("valid RS384 key");
        assert!(provider.kid.is_none());

        let assertion = provider
            .client_assertion("https://idp.example.com/token")
            .expect("assertion minted");
        let header = jsonwebtoken::decode_header(&assertion).unwrap();
        assert_eq!(header.alg, Algorithm::RS384);
        assert_eq!(header.typ.as_deref(), Some("JWT"));
        assert_eq!(header.kid, None);
    }
}
