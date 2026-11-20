//! A `$bulk-submit` kick-off that names an existing submission, and a
//! `$bulk-submit-status` kick-off, are admitted to the submission's owner
//! only (subject match, or a system wildcard), as poll, cancel and file are.
//! A second `system/bulk-submit` client of the same tenant cannot stop,
//! complete, replace or extend another provider's submission, and is told
//! nothing about it.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_audit::ExclusionFilter;
use helios_auth::{AuthConfig, AuthError, AuthProvider, Principal, ScopeSet};
use helios_persistence::backends::local_fs::LocalFsOutputStore;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::{
    BulkSubmitJobStore, ExportOutputStore, RemoteManifest, SubmitInputFetcher,
};
use helios_persistence::error::StorageResult;
use helios_rest::bulk_export_auth::BearerScopeAuth;
use helios_rest::{AuthMiddlewareState, ServerConfig};
use serde_json::{Value, json};

/// `system/bulk-submit`, subject `sub-provider-a`.
const PROVIDER_A: &str = "provider-a-token";
/// `system/bulk-submit`, subject `sub-provider-b`: another client of the same tenant.
const PROVIDER_B: &str = "provider-b-token";
/// `system/*.cruds`: the operator.
const WILDCARD: &str = "wildcard-token";
/// `system/*.c`: a create-only wildcard, without the `d` that stopping takes.
const CREATE_WILDCARD: &str = "create-wildcard-token";

struct StubProvider;

#[async_trait]
impl AuthProvider for StubProvider {
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError> {
        let token = authorization_header
            .strip_prefix("Bearer ")
            .ok_or(AuthError::InvalidSignature)?;
        let scopes = match token {
            PROVIDER_A | PROVIDER_B => "system/bulk-submit",
            WILDCARD => "system/*.cruds",
            CREATE_WILDCARD => "system/*.c",
            _ => return Err(AuthError::InvalidSignature),
        };
        Ok(Principal {
            subject: format!("sub-{}", token.trim_end_matches("-token")),
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

/// No worker runs in these tests; the fetcher is never reached.
struct NoFetcher;

#[async_trait]
impl SubmitInputFetcher for NoFetcher {
    async fn fetch_manifest(
        &self,
        _url: &str,
        _headers: &[(String, String)],
        _oauth: &[String],
        _encryption_key: Option<&Value>,
    ) -> StorageResult<RemoteManifest> {
        unimplemented!("no fetcher in this test")
    }

    async fn open_file_stream(
        &self,
        _url: &str,
        _headers: &[(String, String)],
        _requires_access_token: bool,
        _oauth: &[String],
        _encryption_key: Option<&Value>,
    ) -> StorageResult<(Box<dyn tokio::io::AsyncBufRead + Send + Unpin>, Option<u64>)> {
        unimplemented!("no fetcher in this test")
    }
}

const AUTHORIZATION: HeaderName = HeaderName::from_static("authorization");

fn bearer(token: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("Bearer {token}")).unwrap()
}

fn server() -> (TestServer, tempfile::TempDir) {
    let backend = Arc::new(
        SqliteBackend::with_config(
            ":memory:",
            SqliteBackendConfig {
                data_dir: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
                ..Default::default()
            },
        )
        .expect("in-memory sqlite"),
    );
    backend.init_schema().expect("init schema");
    let tmp = tempfile::tempdir().expect("tempdir");
    let output = Arc::new(LocalFsOutputStore::new(
        tmp.path(),
        "https://wrong-internal.example",
    ));
    let state = helios_rest::AppState::new(Arc::clone(&backend), ServerConfig::for_testing())
        .with_bulk_submit(
            backend as Arc<dyn BulkSubmitJobStore>,
            Arc::new(NoFetcher) as Arc<dyn SubmitInputFetcher>,
            output as Arc<dyn ExportOutputStore>,
            Arc::new(BearerScopeAuth),
        );
    let routes = helios_rest::routing::fhir_routes::create_routes(state);
    let auth = Arc::new(AuthMiddlewareState {
        provider: Arc::new(StubProvider),
        config: Arc::new(AuthConfig::default()),
        audit_sink: Arc::new(helios_audit::sinks::NullSink),
        audit_source_observer: "Device/test".to_string(),
        audit_exclusion_filter: ExclusionFilter::new(vec![]),
        tenant_url_routing: false,
        sessions: None,
    });
    let app = routes
        .layer(axum::middleware::from_fn_with_state(
            auth.clone(),
            helios_rest::middleware::auth::authz_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            auth,
            helios_rest::middleware::auth::auth_middleware,
        ));
    (TestServer::new(app).expect("test server"), tmp)
}

fn identity() -> Vec<Value> {
    vec![
        json!({"name": "submitter", "valueIdentifier": {"system": "http://ehr", "value": "ehr-1"}}),
        json!({"name": "submissionId", "valueString": "it-1"}),
    ]
}

fn parameters(extra: Vec<Value>) -> Value {
    let mut parameter = identity();
    parameter.extend(extra);
    json!({"resourceType": "Parameters", "parameter": parameter})
}

fn manifest_kickoff(manifest_url: &str) -> Value {
    parameters(vec![
        json!({"name": "manifestUrl", "valueUrl": manifest_url}),
        json!({"name": "fhirBaseUrl", "valueUrl": "https://provider/fhir"}),
    ])
}

fn status_only(code: &str) -> Value {
    parameters(vec![json!({"name": "submissionStatus", "valueCoding": {
        "system": "http://hl7.org/fhir/event-status", "code": code}})])
}

fn replace_only(manifest_url: &str) -> Value {
    parameters(vec![
        json!({"name": "submissionStatus", "valueCoding": {
            "system": "http://hl7.org/fhir/event-status", "code": "in-progress"}}),
        json!({"name": "replacesManifestUrl", "valueUrl": manifest_url}),
    ])
}

async fn kickoff(server: &TestServer, token: &str, body: &Value) -> axum_test::TestResponse {
    server
        .post("/$bulk-submit")
        .add_header(AUTHORIZATION, bearer(token))
        .json(body)
        .await
}

async fn status_kickoff(server: &TestServer, token: &str) -> axum_test::TestResponse {
    server
        .post("/$bulk-submit-status")
        .add_header(AUTHORIZATION, bearer(token))
        .json(&parameters(vec![]))
        .await
}

#[tokio::test]
async fn another_client_of_the_tenant_cannot_touch_a_submission_it_does_not_own() {
    let (server, _tmp) = server();

    let created = kickoff(
        &server,
        PROVIDER_A,
        &manifest_kickoff("https://provider/manifest-a.json"),
    )
    .await;
    assert_eq!(created.status_code(), StatusCode::OK, "{}", created.text());

    for (what, body) in [
        ("stop", status_only("stopped")),
        ("complete", status_only("completed")),
        ("replace", replace_only("https://provider/manifest-a.json")),
        (
            "extend",
            manifest_kickoff("https://provider/manifest-b.json"),
        ),
    ] {
        let response = kickoff(&server, PROVIDER_B, &body).await;
        assert_eq!(
            response.status_code(),
            StatusCode::NOT_FOUND,
            "{what} by another client: {}",
            response.text()
        );
    }
    let status = status_kickoff(&server, PROVIDER_B).await;
    assert_eq!(
        status.status_code(),
        StatusCode::NOT_FOUND,
        "status kick-off by another client: {}",
        status.text()
    );

    // Still the owner's, still in progress, manifest-b never attached: the
    // owner can attach it now (a duplicate would be 409), then close out.
    let extended = kickoff(
        &server,
        PROVIDER_A,
        &manifest_kickoff("https://provider/manifest-b.json"),
    )
    .await;
    assert_eq!(
        extended.status_code(),
        StatusCode::OK,
        "{}",
        extended.text()
    );
    let status = status_kickoff(&server, PROVIDER_A).await;
    assert_eq!(
        status.status_code(),
        StatusCode::ACCEPTED,
        "{}",
        status.text()
    );
    assert!(status.headers().contains_key("content-location"));
    let completed = kickoff(&server, PROVIDER_A, &status_only("completed")).await;
    assert_eq!(
        completed.status_code(),
        StatusCode::OK,
        "{}",
        completed.text()
    );
}

#[tokio::test]
async fn a_system_wildcard_may_act_on_any_submission() {
    let (server, _tmp) = server();

    let created = kickoff(
        &server,
        PROVIDER_A,
        &manifest_kickoff("https://provider/manifest-a.json"),
    )
    .await;
    assert_eq!(created.status_code(), StatusCode::OK, "{}", created.text());

    let status = status_kickoff(&server, WILDCARD).await;
    assert_eq!(
        status.status_code(),
        StatusCode::ACCEPTED,
        "{}",
        status.text()
    );
    let stopped = kickoff(&server, WILDCARD, &status_only("stopped")).await;
    assert_eq!(stopped.status_code(), StatusCode::OK, "{}", stopped.text());
}

#[tokio::test]
async fn a_wildcard_needs_the_delete_bit_to_stop_a_submission_it_does_not_own() {
    let (server, _tmp) = server();

    let created = kickoff(
        &server,
        PROVIDER_A,
        &manifest_kickoff("https://provider/manifest-a.json"),
    )
    .await;
    assert_eq!(created.status_code(), StatusCode::OK, "{}", created.text());

    // Stopping is the cancel handler's abort, which asks for `d`.
    let stopped = kickoff(&server, CREATE_WILDCARD, &status_only("stopped")).await;
    assert_eq!(
        stopped.status_code(),
        StatusCode::NOT_FOUND,
        "{}",
        stopped.text()
    );
    // Looking takes `r`.
    let status = status_kickoff(&server, CREATE_WILDCARD).await;
    assert_eq!(
        status.status_code(),
        StatusCode::NOT_FOUND,
        "{}",
        status.text()
    );
    // Continuing takes `c`, which the operation itself asks for.
    let completed = kickoff(&server, CREATE_WILDCARD, &status_only("completed")).await;
    assert_eq!(
        completed.status_code(),
        StatusCode::OK,
        "{}",
        completed.text()
    );
}
