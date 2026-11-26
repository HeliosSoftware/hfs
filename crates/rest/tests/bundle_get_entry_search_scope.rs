//! A GET entry of a batch or transaction Bundle is authorized for the
//! interaction it performs, as the direct request is: a search-shaped URL
//! needs a `search` grant on the type, a read-shaped one needs `read`.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_audit::ExclusionFilter;
use helios_auth::{AuthConfig, AuthError, AuthProvider, Principal, ScopeSet};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::{AuthMiddlewareState, ServerConfig};
use serde_json::{Value, json};

/// `system/*.cruds`: seeds the data.
const SYSTEM: &str = "system-token";
/// `system/Patient.r`: may read Patients, not search them.
const PATIENT_READ: &str = "patient-read-token";
/// `system/Patient.s`: may search Patients, not read one by id.
const PATIENT_SEARCH: &str = "patient-search-token";

struct StubProvider;

#[async_trait]
impl AuthProvider for StubProvider {
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError> {
        let token = authorization_header
            .strip_prefix("Bearer ")
            .ok_or(AuthError::InvalidSignature)?;
        let scopes = match token {
            SYSTEM => "system/*.cruds",
            PATIENT_READ => "system/Patient.r",
            PATIENT_SEARCH => "system/Patient.s",
            _ => return Err(AuthError::InvalidSignature),
        };
        Ok(Principal {
            subject: format!("sub-{token}"),
            issuer: "https://idp.example.com".to_string(),
            scopes: ScopeSet::parse(scopes),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
            ..Default::default()
        })
    }

    fn name(&self) -> &str {
        "stub"
    }
}

const AUTHORIZATION: HeaderName = HeaderName::from_static("authorization");

fn bearer(token: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("Bearer {token}")).unwrap()
}

fn server() -> TestServer {
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
            ..Default::default()
        },
    )
    .expect("in-memory sqlite");
    backend.init_schema().expect("init schema");
    let auth = Arc::new(AuthMiddlewareState {
        provider: Arc::new(StubProvider),
        config: Arc::new(AuthConfig::default()),
        audit_sink: Arc::new(helios_audit::sinks::NullSink),
        audit_source_observer: "Device/test".to_string(),
        audit_exclusion_filter: ExclusionFilter::new(vec![]),
        tenant_url_routing: false,
        sessions: None,
    });
    let app = helios_rest::create_app_with_auth(
        backend,
        ServerConfig::for_testing(),
        AuthConfig::default(),
        Some(auth),
        None,
    );
    TestServer::new(app).expect("test server")
}

async fn seeded() -> TestServer {
    let server = server();
    let response = server
        .put("/Patient/p1")
        .add_header(AUTHORIZATION, bearer(SYSTEM))
        .json(&json!({
            "resourceType": "Patient",
            "id": "p1",
            "identifier": [{"system": "https://example.org/mrn", "value": "11111"}],
            "name": [{"family": "Doe"}]
        }))
        .await;
    assert!(response.status_code().is_success(), "{}", response.text());
    server
}

fn bundle(kind: &str, url: &str) -> Value {
    json!({
        "resourceType": "Bundle",
        "type": kind,
        "entry": [{"request": {"method": "GET", "url": url}}]
    })
}

/// Status of the single entry of a batch response, as the three-digit code.
fn batch_entry_status(body: &Value) -> String {
    body["entry"][0]["response"]["status"]
        .as_str()
        .unwrap_or("")
        .chars()
        .take(3)
        .collect()
}

const SEARCH_URL: &str = "Patient?identifier=https://example.org/mrn|11111";
const READ_URL: &str = "Patient/p1";

#[tokio::test]
async fn a_search_entry_needs_the_search_grant_in_both_arms() {
    let server = seeded().await;

    // The direct request, for reference.
    let direct = server
        .get("/Patient?identifier=https://example.org/mrn|11111")
        .add_header(AUTHORIZATION, bearer(PATIENT_READ))
        .await;
    assert_eq!(
        direct.status_code(),
        StatusCode::FORBIDDEN,
        "{}",
        direct.text()
    );

    let batch = server
        .post("/")
        .add_header(AUTHORIZATION, bearer(PATIENT_READ))
        .json(&bundle("batch", SEARCH_URL))
        .await;
    assert_eq!(batch.status_code(), StatusCode::OK, "{}", batch.text());
    let body = batch.json::<Value>();
    assert_eq!(batch_entry_status(&body), "403", "{body}");
    assert!(body["entry"][0]["resource"]["entry"].is_null(), "{body}");

    let transaction = server
        .post("/")
        .add_header(AUTHORIZATION, bearer(PATIENT_READ))
        .json(&bundle("transaction", SEARCH_URL))
        .await;
    assert_eq!(
        transaction.status_code(),
        StatusCode::FORBIDDEN,
        "{}",
        transaction.text()
    );

    // The search grant admits the same entries.
    let batch = server
        .post("/")
        .add_header(AUTHORIZATION, bearer(PATIENT_SEARCH))
        .json(&bundle("batch", SEARCH_URL))
        .await;
    let body = batch.json::<Value>();
    assert_eq!(batch_entry_status(&body), "200", "{body}");
    assert_eq!(
        body["entry"][0]["resource"]["entry"][0]["resource"]["id"],
        "p1"
    );

    let transaction = server
        .post("/")
        .add_header(AUTHORIZATION, bearer(PATIENT_SEARCH))
        .json(&bundle("transaction", SEARCH_URL))
        .await;
    assert_eq!(
        transaction.status_code(),
        StatusCode::OK,
        "{}",
        transaction.text()
    );
    let body = transaction.json::<Value>();
    assert_eq!(
        body["entry"][0]["resource"]["entry"][0]["resource"]["id"],
        "p1"
    );
}

#[tokio::test]
async fn a_read_entry_still_needs_the_read_grant() {
    let server = seeded().await;

    let batch = server
        .post("/")
        .add_header(AUTHORIZATION, bearer(PATIENT_SEARCH))
        .json(&bundle("batch", READ_URL))
        .await;
    let body = batch.json::<Value>();
    assert_eq!(batch_entry_status(&body), "403", "{body}");

    let batch = server
        .post("/")
        .add_header(AUTHORIZATION, bearer(PATIENT_READ))
        .json(&bundle("batch", READ_URL))
        .await;
    let body = batch.json::<Value>();
    assert_eq!(batch_entry_status(&body), "200", "{body}");
    assert_eq!(body["entry"][0]["resource"]["id"], "p1");

    let transaction = server
        .post("/")
        .add_header(AUTHORIZATION, bearer(PATIENT_SEARCH))
        .json(&bundle("transaction", READ_URL))
        .await;
    assert_eq!(
        transaction.status_code(),
        StatusCode::FORBIDDEN,
        "{}",
        transaction.text()
    );
}
