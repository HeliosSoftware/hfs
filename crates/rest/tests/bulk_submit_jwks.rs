//! `GET /.well-known/bulk-submit-jwks.json` (#529): HFS's own bulk-submit
//! signing key, published so a recipient's authorization server can register
//! the URL as the client's `jwks_uri`.
//!
//! The app is built with the authorization layers on and a provider that
//! rejects every token, so a `200` here proves the route is exempt from
//! authentication rather than that some token happened to be accepted.

use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{StatusCode, header};
use axum_test::TestServer;
use helios_audit::ExclusionFilter;
use helios_auth::{AuthConfig, AuthError, AuthProvider, Principal};
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::core::FileTokenProvider;
use helios_rest::bulk_submit_oauth::JwtClientCredentialsTokenProvider;
use helios_rest::config::BulkSubmitConfig;
use helios_rest::{AuthMiddlewareState, ServerConfig};
use serde_json::{Value, json};

const JWKS_PATH: &str = "/.well-known/bulk-submit-jwks.json";

// A throwaway P-384 PKCS#8 key (ES384), generated solely for tests. It is the
// same key `bulk_submit_oauth::tests::TEST_ES384_PEM` uses, so the values
// below are the ones that unit test pins.
const TEST_ES384_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIG2AgEAMBAGByqGSM49AgEGBSuBBAAiBIGeMIGbAgEBBDDL2uAdbDMvTFQ/r+Tz
45fPr+tTiZPaq7mKcF2g1fmOCRMSdkSMLaRBzWbIwOE8GxOhZANiAASjv7NSBHxr
k5K6g75uWMf/NsVu7ay2lO8vuu9EC12UBjDQ+297+9I7+IlSzRmoWKUT0fVrOeoB
gMMPIjEueYREkCGj6cILnLaP5hAey+Q7JjRgVO6Xqf0u2uCO6yn8UTU=
-----END PRIVATE KEY-----
";

// Public point and RFC 7638 thumbprint of `TEST_ES384_PEM`, computed outside
// this crate (`openssl ec -text`, then Python `hashlib.sha256` over
// `{"crv":"P-384","kty":"EC","x":"<x>","y":"<y>"}`).
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

/// Rejects every bearer token: anything reaching a handler through the auth
/// layer without an exemption would be a `401`.
struct RejectAll;

#[async_trait]
impl AuthProvider for RejectAll {
    async fn authenticate(&self, _authorization_header: &str) -> Result<Principal, AuthError> {
        Err(AuthError::InvalidSignature)
    }

    fn name(&self) -> &str {
        "reject-all"
    }
}

fn bulk_submit(private_key: Option<&str>, signing_alg: &str) -> BulkSubmitConfig {
    BulkSubmitConfig {
        private_key: private_key.map(str::to_string),
        signing_alg: signing_alg.to_string(),
        ..BulkSubmitConfig::default()
    }
}

/// The in-process app with the auth layers on.
fn app(bulk_submit: BulkSubmitConfig) -> axum::Router {
    let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
    backend.init_schema().expect("init schema");
    let auth = Arc::new(AuthMiddlewareState {
        provider: Arc::new(RejectAll),
        config: Arc::new(AuthConfig::default()),
        audit_sink: Arc::new(helios_audit::sinks::NullSink),
        audit_source_observer: "Device/test".to_string(),
        audit_exclusion_filter: ExclusionFilter::new(vec![]),
        tenant_url_routing: false,
        sessions: None,
    });
    helios_rest::create_app_with_auth(
        backend,
        ServerConfig {
            bulk_submit,
            ..ServerConfig::for_testing()
        },
        AuthConfig {
            enabled: true,
            ..AuthConfig::default()
        },
        Some(auth),
        None,
    )
}

fn server(bulk_submit: BulkSubmitConfig) -> TestServer {
    TestServer::new(app(bulk_submit)).expect("test server")
}

/// RFC 7638 §3.2: SHA-256 over the required EC members `crv`, `kty`, `x`,
/// `y`, in that order with no whitespace, base64url without padding.
fn thumbprint(jwk: &Value) -> String {
    use base64::Engine as _;
    use sha2::{Digest, Sha256};
    let member = |name: &str| jwk[name].as_str().expect("string member").to_string();
    let canonical = format!(
        r#"{{"crv":"{}","kty":"{}","x":"{}","y":"{}"}}"#,
        member("crv"),
        member("kty"),
        member("x"),
        member("y")
    );
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(canonical))
}

#[tokio::test]
async fn an_es384_key_is_served_to_an_unauthenticated_caller() {
    let server = server(bulk_submit(Some(TEST_ES384_PEM), "ES384"));

    // No Authorization header at all.
    let response = server.get(JWKS_PATH).await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .expect("content-type header")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        content_type.starts_with("application/json"),
        "content-type was {content_type}"
    );

    let body: Value = response.json();
    let keys = body["keys"].as_array().expect("`keys` is an array");
    assert_eq!(keys.len(), 1, "one key is published: {body}");
    let key = &keys[0];
    assert_eq!(key["kty"], "EC");
    assert_eq!(key["crv"], "P-384");
    assert_eq!(key["use"], "sig");
    assert_eq!(key["alg"], "ES384");
    assert_eq!(key["x"], TEST_ES384_X);
    assert_eq!(key["y"], TEST_ES384_Y);
    // The `kid` is the RFC 7638 thumbprint, both as recomputed here and as
    // pinned by the same value the `derive_public_jwk` unit test asserts.
    assert_eq!(key["kid"], thumbprint(key).as_str());
    assert_eq!(key["kid"], TEST_ES384_KID);
    assert!(key.get("d").is_none(), "no private scalar is published");
}

#[tokio::test]
async fn the_exemption_is_for_this_path_only() {
    // Same app, same (absent) credentials: the JWKS answers, a FHIR read does
    // not, so the layers above are demonstrably on.
    let server = server(bulk_submit(Some(TEST_ES384_PEM), "ES384"));
    server.get(JWKS_PATH).await.assert_status(StatusCode::OK);
    server
        .get("/Patient/123")
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    server
        .get(&format!("/Patient{JWKS_PATH}"))
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn no_configured_key_serves_an_empty_set() {
    let server = server(bulk_submit(None, "ES384"));
    let response = server.get(JWKS_PATH).await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    assert_eq!(response.json::<Value>(), json!({ "keys": [] }));
}

/// The documented RS384 limit, pinned (#529, #560): with a valid RSA key
/// configured the set is still empty, because RSA public-key derivation was
/// removed over RUSTSEC-2023-0071. Such a deployment registers its key with
/// the recipient out of band.
#[tokio::test]
async fn an_rs384_key_serves_an_empty_set_by_design() {
    let server = server(bulk_submit(Some(TEST_RSA_PEM), "RS384"));
    let response = server.get(JWKS_PATH).await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    assert_eq!(response.json::<Value>(), json!({ "keys": [] }));
}

/// The whole registration story end to end: the provider signs an assertion
/// for a mock authorization server, and the key that server could fetch from
/// the running HFS verifies it, found by the assertion's `kid`.
#[tokio::test]
async fn the_served_key_verifies_the_assertion_the_provider_sends() {
    use jsonwebtoken::{Algorithm, DecodingKey, Validation, jwk::JwkSet};

    // The recipient's authorization server: discovery + a token endpoint that
    // keeps the client assertion it was sent.
    let idp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let idp_base = format!("http://{}", idp.local_addr().unwrap());
    let seen: Arc<std::sync::Mutex<Option<String>>> = Arc::default();
    let idp_app = {
        let token_endpoint = format!("{idp_base}/token");
        let seen = Arc::clone(&seen);
        axum::Router::new()
            .route(
                "/.well-known/smart-configuration",
                axum::routing::get(move || {
                    let token_endpoint = token_endpoint.clone();
                    async move { axum::Json(json!({ "token_endpoint": token_endpoint })) }
                }),
            )
            .route(
                "/token",
                axum::routing::post(move |body: String| {
                    let seen = Arc::clone(&seen);
                    async move {
                        let assertion = url::form_urlencoded::parse(body.as_bytes())
                            .find(|(k, _)| k == "client_assertion")
                            .map(|(_, v)| v.into_owned());
                        *seen.lock().unwrap() = assertion;
                        axum::Json(json!({ "access_token": "tok", "expires_in": 300 }))
                    }
                }),
            )
    };
    tokio::spawn(async move { axum::serve(idp, idp_app).await.unwrap() });

    // HFS itself, served on a real port so its JWKS URL can be fetched.
    let hfs = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hfs_base = format!("http://{}", hfs.local_addr().unwrap());
    let hfs_app = app(bulk_submit(Some(TEST_ES384_PEM), "ES384"));
    tokio::spawn(async move { axum::serve(hfs, hfs_app).await.unwrap() });

    let provider = JwtClientCredentialsTokenProvider::new("hfs-client", TEST_ES384_PEM, "ES384")
        .expect("valid ES384 key");
    let token = provider
        .token(
            &[format!("{idp_base}/.well-known/smart-configuration")],
            "system/*.rs",
        )
        .await;
    assert_eq!(token.as_deref(), Some("tok"));
    let assertion = seen
        .lock()
        .unwrap()
        .clone()
        .expect("the IdP saw an assertion");

    // What the recipient registers as `jwks_uri`, fetched with no credentials.
    let jwks: JwkSet = reqwest::get(format!("{hfs_base}{JWKS_PATH}"))
        .await
        .expect("fetch the JWKS")
        .json()
        .await
        .expect("a JWK Set");

    let header = jsonwebtoken::decode_header(&assertion).unwrap();
    assert_eq!(header.alg, Algorithm::ES384);
    let kid = header.kid.expect("the assertion names its key");
    let jwk = jwks
        .find(&kid)
        .expect("the published set holds the key the assertion names");

    let mut validation = Validation::new(Algorithm::ES384);
    validation.set_audience(&[format!("{idp_base}/token")]);
    let verified = jsonwebtoken::decode::<Value>(
        &assertion,
        &DecodingKey::from_jwk(jwk).expect("decoding key from the served JWK"),
        &validation,
    )
    .expect("the served key verifies the assertion");
    assert_eq!(verified.claims["iss"], "hfs-client");
}
