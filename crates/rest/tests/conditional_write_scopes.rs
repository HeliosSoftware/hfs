//! A conditional create, update, patch or delete searches the type before it
//! writes, and its answer tells the client what the search found, so it
//! needs the Search grant a direct search would, on the resource endpoints
//! and on the Bundle entries alike. A `patient/` grant is refused: the
//! criteria cannot be confined to the launch patient's compartment.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum_test::{TestResponse, TestServer};
use helios_audit::ExclusionFilter;
use helios_auth::{AuthConfig, AuthError, AuthProvider, LaunchContext, Principal, ScopeSet};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::{AuthMiddlewareState, ServerConfig};
use serde_json::{Value, json};

/// `system/*.cruds`: seeds the data.
const SYSTEM: &str = "system-token";
/// `system/Patient.cud`: writes Patients and may not search them.
const WRITE: &str = "write-token";
/// `system/Patient.cuds`: writes and searches Patients.
const WRITE_SEARCH: &str = "write-search-token";
/// `patient/Patient.cruds` with `patient = p1`.
const P1: &str = "p1-token";

const MRN: &str = "https://example.org/mrn|11111";

struct StubProvider;

#[async_trait]
impl AuthProvider for StubProvider {
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError> {
        let token = authorization_header
            .strip_prefix("Bearer ")
            .ok_or(AuthError::InvalidSignature)?;
        let (scopes, patient) = match token {
            SYSTEM => ("system/*.cruds", None),
            WRITE => ("system/Patient.cud", None),
            WRITE_SEARCH => ("system/Patient.cuds", None),
            P1 => ("patient/Patient.cruds", Some("p1")),
            _ => return Err(AuthError::InvalidSignature),
        };
        Ok(Principal {
            subject: format!("sub-{token}"),
            issuer: "https://idp.example.com".to_string(),
            scopes: ScopeSet::parse(scopes),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
            launch_context: patient.map(|p| LaunchContext {
                patient: Some(p.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        })
    }

    fn name(&self) -> &str {
        "stub"
    }
}

const AUTHORIZATION: HeaderName = HeaderName::from_static("authorization");
const IF_NONE_EXIST: HeaderName = HeaderName::from_static("if-none-exist");

fn bearer(token: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("Bearer {token}")).unwrap()
}

fn patient() -> Value {
    json!({
        "resourceType": "Patient",
        "identifier": [{"system": "https://example.org/mrn", "value": "11111"}],
        "name": [{"family": "Doe"}]
    })
}

async fn server() -> TestServer {
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
    let server = TestServer::new(app).expect("test server");

    let mut seed = patient();
    seed["id"] = json!("p1");
    let response = server
        .put("/Patient/p1")
        .add_header(AUTHORIZATION, bearer(SYSTEM))
        .json(&seed)
        .await;
    assert!(
        response.status_code().is_success(),
        "seed Patient/p1: {}",
        response.text()
    );
    server
}

fn assert_search_refused(response: TestResponse) {
    assert_eq!(
        response.status_code(),
        StatusCode::FORBIDDEN,
        "{}",
        response.text()
    );
    let text = response.text();
    assert!(text.contains("search on Patient"), "{text}");
}

#[tokio::test]
async fn a_conditional_create_needs_a_search_grant() {
    let server = server().await;
    let post = |token: &'static str| {
        server
            .post("/Patient")
            .add_header(AUTHORIZATION, bearer(token))
            .add_header(
                IF_NONE_EXIST,
                HeaderValue::from_static("identifier=https://example.org/mrn|11111"),
            )
            .json(&patient())
    };
    assert_search_refused(post(WRITE).await);

    let matched = post(WRITE_SEARCH).await;
    assert_eq!(matched.status_code(), StatusCode::OK, "{}", matched.text());
    assert_eq!(matched.json::<Value>()["id"], "p1");
}

#[tokio::test]
async fn a_conditional_update_needs_a_search_grant() {
    let server = server().await;
    let put = |token: &'static str| {
        server
            .put(&format!("/Patient?identifier={MRN}"))
            .add_header(AUTHORIZATION, bearer(token))
            .json(&patient())
    };
    assert_search_refused(put(WRITE).await);

    let updated = put(WRITE_SEARCH).await;
    assert_eq!(updated.status_code(), StatusCode::OK, "{}", updated.text());
    assert_eq!(updated.json::<Value>()["id"], "p1");
}

#[tokio::test]
async fn a_conditional_patch_needs_a_search_grant() {
    let server = server().await;
    let patch = |token: &'static str| {
        server
            .patch(&format!("/Patient?identifier={MRN}"))
            .add_header(AUTHORIZATION, bearer(token))
            .add_header(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json-patch+json"),
            )
            .bytes(
                json!([{"op": "add", "path": "/active", "value": true}])
                    .to_string()
                    .into(),
            )
    };
    assert_search_refused(patch(WRITE).await);

    let patched = patch(WRITE_SEARCH).await;
    assert_eq!(patched.status_code(), StatusCode::OK, "{}", patched.text());
    assert_eq!(patched.json::<Value>()["active"], true);
}

#[tokio::test]
async fn a_conditional_delete_needs_a_search_grant() {
    let server = server().await;
    let delete = |token: &'static str| {
        server
            .delete(&format!("/Patient?identifier={MRN}"))
            .add_header(AUTHORIZATION, bearer(token))
    };
    assert_search_refused(delete(WRITE).await);

    let deleted = delete(WRITE_SEARCH).await;
    assert!(
        deleted.status_code().is_success(),
        "{} {}",
        deleted.status_code(),
        deleted.text()
    );
    let gone = server
        .get("/Patient/p1")
        .add_header(AUTHORIZATION, bearer(SYSTEM))
        .await;
    assert_eq!(gone.status_code(), StatusCode::GONE, "{}", gone.text());
}

#[tokio::test]
async fn a_patient_scope_cannot_run_criteria() {
    let server = server().await;
    let refused = server
        .put(&format!("/Patient?identifier={MRN}"))
        .add_header(AUTHORIZATION, bearer(P1))
        .json(&patient())
        .await;
    assert_eq!(
        refused.status_code(),
        StatusCode::FORBIDDEN,
        "{}",
        refused.text()
    );
    assert!(
        refused.text().contains("patient/ scope"),
        "{}",
        refused.text()
    );
}

fn fhirpath_set_active() -> Value {
    json!({"resourceType": "Parameters", "parameter": [{"name": "operation", "part": [
        {"name": "type", "valueCode": "add"},
        {"name": "path", "valueString": "Patient"},
        {"name": "name", "valueString": "active"},
        {"name": "value", "valueBoolean": true}
    ]}]})
}

fn conditional_entries() -> Vec<Value> {
    vec![
        json!({"request": {"method": "POST", "url": "Patient", "ifNoneExist": format!("identifier={MRN}")}, "resource": patient()}),
        json!({"request": {"method": "PUT", "url": format!("Patient?identifier={MRN}")}, "resource": patient()}),
        json!({"request": {"method": "PATCH", "url": format!("Patient?identifier={MRN}")}, "resource": fhirpath_set_active()}),
        json!({"request": {"method": "DELETE", "url": format!("Patient?identifier={MRN}")}}),
    ]
}

fn bundle(kind: &str, entries: Vec<Value>) -> Value {
    json!({"resourceType": "Bundle", "type": kind, "entry": entries})
}

async fn post_bundle(server: &TestServer, token: &str, body: Value) -> TestResponse {
    server
        .post("/")
        .add_header(AUTHORIZATION, bearer(token))
        .json(&body)
        .await
}

#[tokio::test]
async fn bundle_entries_are_judged_too() {
    let server = server().await;

    let batch = post_bundle(&server, WRITE, bundle("batch", conditional_entries())).await;
    assert_eq!(batch.status_code(), StatusCode::OK, "{}", batch.text());
    let body: Value = batch.json();
    for (index, entry) in body["entry"].as_array().unwrap().iter().enumerate() {
        let status = entry["response"]["status"].as_str().unwrap_or_default();
        assert!(status.starts_with("403"), "entry {index}: {entry}");
        let outcome = entry["response"]["outcome"].to_string();
        assert!(
            outcome.contains("search on Patient"),
            "entry {index}: {outcome}"
        );
    }

    for entry in conditional_entries() {
        let refused = post_bundle(&server, WRITE, bundle("transaction", vec![entry.clone()])).await;
        assert_search_refused(refused);
    }

    let create = conditional_entries().swap_remove(0);
    let matched = post_bundle(&server, WRITE_SEARCH, bundle("transaction", vec![create])).await;
    assert_eq!(matched.status_code(), StatusCode::OK, "{}", matched.text());
    let body: Value = matched.json();
    assert!(
        body["entry"][0]["response"]["status"]
            .as_str()
            .unwrap_or_default()
            .starts_with("200"),
        "{body}"
    );
}
