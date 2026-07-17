//! Bearer access tokens are reusable until they expire.
//!
//! HFS is a resource server, so the tokens it validates are ordinary OAuth2
//! bearer access tokens — the client obtains one and presents it on every
//! request until `exp`. They are *not* single-use. Most IdPs (Keycloak among
//! them) put a `jti` on every access token, and HFS once ran those through a
//! replay cache, which rejected the second and every subsequent use with
//! `401 "JTI replay detected"` (#205).
//!
//! This test pins the fix: the same token, `jti` and all, authenticates
//! repeatedly.

use std::sync::Arc;

use helios_auth::{AuthConfig, AuthProvider, JwksBearerAuthProvider, JwksCache};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TEST_KID: &str = "test-key-1";

/// Test-only RSA private key. Generated for this test; not used anywhere else.
const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCSroIOUaCnmnAz
wXfTUJd7PeHkKBFyL1cuAz3bDgzCy6LyhSeBaytYLgzc43v2MD9fIMpD1sh2Uua5
mM56uypO2i3FKVsCJQzRs7gc1cz8dBPCDG6jK9wwmQ+eE3TdpxPfqDwVNxsNQu8B
5qByfgwO6vjDPDFvdl1JaGGLBwAK2aMNa/fL2isD358iIzrnydkXgUYD8FnfKY2u
3gqeTy4yrJIClBiLH7KH73/CpU08w2xQssUyZPOIZ+sWUgKb/HKAl6ldBKM6gdOw
+fScdT/Qg5eZWizxJoohI7gCDLUra+xVi4Q6IA1h2+IJY523fG3lXpusO8ni8vPP
TtfSc90TAgMBAAECggEABnkE7DTV7g92nBIRg5Wu2ZVlfnf2LR/BrRofhKceEQqD
akhN8fwUsZN2pdi0A60lXsFHq66yseX+oHoJwoi32Trvgh/NEE8qPaa8nSkiHpHQ
vWNnDnRFBy/57HPXWGCjE9+MpzMDUpZ9jYvr0KGXTqE30QW6+Lw0aaTdiREKA3B+
EPe7laG5BfccznNt2ESaqfqyVLE/LRQ3ZxC32ruxoFWwaC9vqs0PWgCII72rqi5f
YcXBdQd9q5x4mPzX9CiR/E9TkTUNL14HFpc1T1XxQMHFtjwIRzisjE9VMjcEUpuF
YIGSSXEaoV8otETkyUWYKGNqlDu7EI39IqSQGmGBIQKBgQDIL7aYftFuDMq3Zgo6
3i5FL1SzzH6cupzVuI3/n2eR5HDKxE2Y1UKwXv4epfBQ5jIJYF+n0wBmfcYZEpq6
yf/golBDVrWBIXAmt0JU2yeaBJkyhUKv/PeukyWPWB5/zCWpAQ1yn66S+RBaIrL3
bXNkjfXsfmbkOS4BHvef5FOq5QKBgQC7k+aKYL81YUswLsx+He12/2xHNKYng9nd
jukZ6mh2xUPbQonJg9+miUNWHIv3CkKkbkbKRXYEF4skCDEANRUqZiErMzj37ODz
B1K4UMWg9F315Y2HSclBy/em7vUQwhOhg1L6UkPLoO92mhE4VSe3BPIoqNjaqVJu
NPhvnUDQlwKBgQCdGhTKiHwDSbatRz8wA718PjDTCeEzTqBWeYe23HqDXCvIdVYQ
Ywz7LRFxK/j1BDKweRmYs1bVGE+mzZrwjCZrO/aRYjL/LCa/u4Iq5fKmRIWVyE8V
ngki0Afh/t2wnZ3QjCrpkbeHUD8s/Z5F33d3qpEdD/XflaAs8QiUSrP+oQKBgFVM
WmfOuuShS7mrbl5jaZrVZ/2xWWVatfXkiOe6CqsH5WWNim7Swx9OCArejF9YkRmI
9DQDBjmyIxnNh8raWLehHbAxaNSFKX4adGlQga5BsYCiVIuS6Cw9fm6w90wZlSe7
Oj6OrjmpA3vhb7c4Mgkt/Ji0v3gfy1ZGTDslPVYdAoGBAKbfGrUtJQr6bpJmCRyp
zUu2oQ8paQXazIx+CscfgoWheDte2STTScGcKj1QuDAozcbBPYUpodSgkK4O5Wrs
qqPm2u7Kvz/kcDNIAIE1MUYNpgiZx/C0ca4yDHACefzwWD9IerNFRVr1CPGtFFzw
uLlPD/tbsTbUAc5x9+dCkqEp
-----END PRIVATE KEY-----";

/// The public half of `TEST_PRIVATE_KEY_PEM`, as an RSA JWK modulus.
const TEST_JWK_N: &str = "kq6CDlGgp5pwM8F301CXez3h5CgRci9XLgM92w4Mwsui8oUngWsrWC4M3ON79jA_XyDKQ9bIdlLmuZjOersqTtotxSlbAiUM0bO4HNXM_HQTwgxuoyvcMJkPnhN03acT36g8FTcbDULvAeagcn4MDur4wzwxb3ZdSWhhiwcACtmjDWv3y9orA9-fIiM658nZF4FGA_BZ3ymNrt4Knk8uMqySApQYix-yh-9_wqVNPMNsULLFMmTziGfrFlICm_xygJepXQSjOoHTsPn0nHU_0IOXmVos8SaKISO4Agy1K2vsVYuEOiANYdviCWOdt3xt5V6brDvJ4vLzz07X0nPdEw";

/// Start a mock IdP serving a JWKS containing the test key.
async fn start_jwks_server() -> MockServer {
    let server = MockServer::start().await;

    let jwks = serde_json::json!({
        "keys": [{
            "kid": TEST_KID,
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "n": TEST_JWK_N,
            "e": "AQAB",
        }]
    });

    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks))
        .mount(&server)
        .await;

    server
}

/// Mint a signed access token carrying a `jti`, exactly as Keycloak would.
fn mint_access_token(jti: &str) -> String {
    let exp = chrono::Utc::now().timestamp() + 3600;
    let claims = serde_json::json!({
        "sub": "service-account-hfs",
        "iss": "https://idp.example.com/realms/hfs",
        "exp": exp,
        "jti": jti,
        "scope": "system/Patient.rs",
    });

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(TEST_KID.to_string());

    let key = EncodingKey::from_rsa_pem(TEST_PRIVATE_KEY_PEM.as_bytes())
        .expect("test private key should parse");

    encode(&header, &claims, &key).expect("token should sign")
}

async fn build_provider(jwks_url: &str) -> JwksBearerAuthProvider {
    let config = AuthConfig {
        enabled: true,
        jwks_url: Some(jwks_url.to_string()),
        expected_issuer: Some("https://idp.example.com/realms/hfs".to_string()),
        ..Default::default()
    };

    let jwks_cache = Arc::new(JwksCache::new(jwks_url, config.jwks_min_refresh_interval));
    jwks_cache
        .initial_fetch()
        .await
        .expect("JWKS fetch should succeed");

    JwksBearerAuthProvider::new(jwks_cache, &config)
}

/// The same access token — same `jti` — must authenticate on every request.
///
/// Regression test for #205: this previously returned
/// `AuthError::ReplayDetected` on the second call.
#[tokio::test]
async fn same_access_token_authenticates_repeatedly() {
    let server = start_jwks_server().await;
    let provider = build_provider(&format!("{}/jwks", server.uri())).await;

    let token = mint_access_token("keycloak-issued-jti-abc123");
    let header = format!("Bearer {}", token);

    for attempt in 1..=3 {
        let principal = provider
            .authenticate(&header)
            .await
            .unwrap_or_else(|e| panic!("use #{attempt} of a reusable access token failed: {e}"));

        assert_eq!(principal.subject, "service-account-hfs");
        assert_eq!(principal.jti.as_deref(), Some("keycloak-issued-jti-abc123"));
    }
}

/// Distinct tokens still authenticate independently — nothing about dropping
/// the replay cache makes the provider stateful across tokens.
#[tokio::test]
async fn distinct_access_tokens_authenticate() {
    let server = start_jwks_server().await;
    let provider = build_provider(&format!("{}/jwks", server.uri())).await;

    for jti in ["jti-one", "jti-two"] {
        let header = format!("Bearer {}", mint_access_token(jti));
        let principal = provider
            .authenticate(&header)
            .await
            .expect("token should authenticate");
        assert_eq!(principal.jti.as_deref(), Some(jti));
    }
}
