//! End-to-end validation tests for [`JwksBearerAuthProvider`].
//!
//! These sign real RS256 tokens with a test key, serve the matching JWKS from a
//! `wiremock` server, and drive `authenticate()` — so they pin the *behavior*
//! (which tokens are accepted), not just the shape of the `Validation` struct.
//! A `jsonwebtoken` upgrade that changed claim-validation semantics underneath
//! us would fail these; it would not fail a struct-field assertion.

use std::sync::Arc;

use helios_auth::{AuthConfig, AuthProvider, InMemoryJtiCache, JwksBearerAuthProvider, JwksCache};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KID: &str = "test-key-1";
const AUDIENCE: &str = "hfs-api";
const ISSUER: &str = "https://idp.example.com";

/// RSA public key parameters matching `tests/data/test_rsa_key.pem`.
const JWK_N: &str = "zbeWVBfqrPsdF1phb2EB5hxjz1EMb5WT4MnhMatFnNFquEZIcgXG1DBAUICEZLQM8gRj9kZ83YA1cFhUtkgUx1qMio8YkKweLV6r8RAqnuxrS5Khm4a25M85qB5MhGoCLZgGtla9lJ79mjVN5XBqD7sPL0ENN1GPcnus1I6WF-E-5Uiih-QWsQVlGeUqMH81vTBLemDxvMQv-TiKZP3LHH_T41G1FfyF-rlbkj7WXGkbqG_WvR4FKeRm1X2_RRVw87h2j1lfK4AiUKs4h_DqwqZju3fgOH_0MHj2RKCu0wo0cTx8mQGbLCNSb-sRoWRnWGpGPphI9xVS8znuPB1JYQ";
const JWK_E: &str = "AQAB";

fn now() -> i64 {
    jsonwebtoken::get_current_timestamp() as i64
}

/// Sign `claims` as an RS256 JWT with the test key.
fn sign(claims: &Value) -> String {
    let pem = include_bytes!("data/test_rsa_key.pem");
    let key = EncodingKey::from_rsa_pem(pem).expect("valid test RSA key");
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(KID.to_string());
    encode(&header, claims, &key).expect("token encodes")
}

/// A token that should always be accepted — each test mutates one claim from this.
fn valid_claims() -> Value {
    json!({
        "sub": "user-123",
        "iss": ISSUER,
        "aud": AUDIENCE,
        "exp": now() + 3600,
    })
}

/// Stand up a JWKS endpoint and a provider configured with audience + issuer.
/// Returns the provider and the mock server (which must be kept alive).
async fn provider_with_audience() -> (JwksBearerAuthProvider, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": [{
                "kty": "RSA",
                "kid": KID,
                "use": "sig",
                "alg": "RS256",
                "n": JWK_N,
                "e": JWK_E,
            }]
        })))
        .mount(&server)
        .await;

    let jwks_cache = Arc::new(JwksCache::new(&format!("{}/jwks", server.uri()), 0));
    jwks_cache.initial_fetch().await.expect("JWKS fetch");

    let config = AuthConfig {
        enabled: true,
        expected_audience: Some(AUDIENCE.to_string()),
        expected_issuer: Some(ISSUER.to_string()),
        ..AuthConfig::default()
    };

    let provider =
        JwksBearerAuthProvider::new(jwks_cache, Arc::new(InMemoryJtiCache::new()), &config);
    (provider, server)
}

async fn authenticate(claims: &Value) -> Result<helios_auth::Principal, helios_auth::AuthError> {
    let (provider, _server) = provider_with_audience().await;
    provider
        .authenticate(&format!("Bearer {}", sign(claims)))
        .await
}

#[tokio::test]
async fn valid_token_is_accepted() {
    // Guards the tests below: they must fail for their stated reason, not because
    // the harness rejects everything.
    let principal = authenticate(&valid_claims()).await.expect("accepted");
    assert_eq!(principal.subject, "user-123");
    assert_eq!(principal.issuer, ISSUER);
}

#[tokio::test]
async fn token_without_audience_is_rejected() {
    // Regression for #206: `set_audience` alone only rejects a *wrong* audience,
    // so a token omitting `aud` entirely bypassed a configured HFS_AUTH_AUDIENCE.
    let mut claims = valid_claims();
    claims.as_object_mut().unwrap().remove("aud");
    let err = authenticate(&claims).await.expect_err("must be rejected");
    assert!(
        err.to_string().contains("aud"),
        "expected a missing-audience error, got: {err}"
    );
}

#[tokio::test]
async fn token_with_wrong_audience_is_rejected() {
    let mut claims = valid_claims();
    claims["aud"] = json!("some-other-api");
    authenticate(&claims).await.expect_err("must be rejected");
}

#[tokio::test]
async fn token_with_non_string_audience_is_rejected() {
    // A JSON type the `aud` claim cannot deserialize into fails to parse, and a
    // failed parse is indistinguishable from an absent claim to the audience
    // check — so this bypassed audience validation the same way an omitted `aud` did.
    let mut claims = valid_claims();
    claims["aud"] = json!(42);
    authenticate(&claims).await.expect_err("must be rejected");
}

#[tokio::test]
async fn token_without_issuer_is_rejected() {
    let mut claims = valid_claims();
    claims.as_object_mut().unwrap().remove("iss");
    let err = authenticate(&claims).await.expect_err("must be rejected");
    assert!(
        err.to_string().contains("iss"),
        "expected a missing-issuer error, got: {err}"
    );
}

#[tokio::test]
async fn token_without_subject_is_rejected() {
    // The subject is what audit records and policy decisions are attributed to.
    let mut claims = valid_claims();
    claims.as_object_mut().unwrap().remove("sub");
    let err = authenticate(&claims).await.expect_err("must be rejected");
    assert!(
        err.to_string().contains("sub"),
        "expected a missing-subject error, got: {err}"
    );
}

#[tokio::test]
async fn token_with_empty_subject_is_rejected() {
    // Present-but-empty satisfies the required-claim check, yet yields an
    // anonymous principal — reject it explicitly.
    let mut claims = valid_claims();
    claims["sub"] = json!("");
    authenticate(&claims).await.expect_err("must be rejected");
}

#[tokio::test]
async fn not_yet_valid_token_is_rejected() {
    // `nbf` is not validated by default in jsonwebtoken.
    let mut claims = valid_claims();
    claims["nbf"] = json!(now() + 3600);
    authenticate(&claims).await.expect_err("must be rejected");
}

#[tokio::test]
async fn expired_token_is_rejected() {
    let mut claims = valid_claims();
    claims["exp"] = json!(now() - 3600);
    authenticate(&claims).await.expect_err("must be rejected");
}

#[tokio::test]
async fn token_with_a_bad_signature_is_rejected() {
    // Signature verification must actually be reachable through this path — a
    // claim-validation test suite that passed on unsigned garbage would be worthless.
    let token = sign(&valid_claims());
    let parts: Vec<&str> = token.split('.').collect();
    let tampered = format!("{}.{}.{}", parts[0], parts[1], "bm90LWEtc2lnbmF0dXJl");

    let (provider, _server) = provider_with_audience().await;
    provider
        .authenticate(&format!("Bearer {tampered}"))
        .await
        .expect_err("must be rejected");
}
