//! The asynchronous job routes of `$export` and `$sql-export` through the real
//! authorization middleware. The middleware does not judge their path segments
//! as resource types; the handlers admit the principal that started the job,
//! whatever its scopes, and a wildcard reader outside `patient/`, and answer
//! anyone else as if the job did not exist.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_audit::ExclusionFilter;
use helios_auth::{AuthConfig, AuthError, AuthProvider, Principal, ScopeSet};
use helios_persistence::backends::local_fs::LocalFsOutputStore;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::{BulkExportJobStore, ExportOutputStore};
use helios_rest::bulk_export_auth::BearerScopeAuth;
use helios_rest::{AuthMiddlewareState, BulkExportBundle, ServerConfig};
use serde_json::{Value, json};

/// `user/Observation.rs` for the subject that starts the jobs.
const OWNER: &str = "owner-token";
/// `user/Observation.rs` for another subject.
const OTHER: &str = "other-token";
/// `user/*.rs` for another subject.
const OTHER_WILDCARD: &str = "other-wildcard-token";
/// `user/*.cruds`, to seed data.
const WRITER: &str = "writer-token";

const BASE_URL: &str = "http://localhost:8080";

struct StubProvider;

#[async_trait]
impl AuthProvider for StubProvider {
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError> {
        let token = authorization_header
            .strip_prefix("Bearer ")
            .ok_or(AuthError::InvalidSignature)?;
        let (subject, scopes) = match token {
            OWNER => ("owner", "user/Observation.rs"),
            OTHER => ("other", "user/Observation.rs"),
            OTHER_WILDCARD => ("other", "user/*.rs"),
            WRITER => ("writer", "user/*.cruds"),
            _ => return Err(AuthError::InvalidSignature),
        };
        Ok(Principal {
            subject: subject.to_string(),
            issuer: "https://idp.example.com".to_string(),
            tenant_id: None,
            scopes: ScopeSet::parse(scopes),
            jti: None,
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
            custom_claims: serde_json::Map::new(),
            ..Default::default()
        })
    }

    fn name(&self) -> &str {
        "stub"
    }
}

fn server() -> TestServer {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));
    let backend = Arc::new(
        SqliteBackend::with_config(
            ":memory:",
            SqliteBackendConfig {
                data_dir: Some(data_dir),
                ..Default::default()
            },
        )
        .expect("in-memory sqlite"),
    );
    backend.init_schema().expect("init schema");
    let tmp = tempfile::tempdir().expect("tempdir");
    let output = Arc::new(LocalFsOutputStore::new(tmp.path().join("bulk"), BASE_URL));
    let config = ServerConfig {
        base_url: BASE_URL.to_string(),
        export_dir: tmp.path().join("sof").to_string_lossy().into_owned(),
        ..ServerConfig::for_testing()
    };
    // The directory lives as long as the process; the tests are short.
    std::mem::forget(tmp);
    let auth = Arc::new(AuthMiddlewareState {
        provider: Arc::new(StubProvider),
        config: Arc::new(AuthConfig::default()),
        audit_sink: Arc::new(helios_audit::sinks::NullSink),
        audit_source_observer: "Device/test".to_string(),
        audit_exclusion_filter: ExclusionFilter::new(vec![]),
        tenant_url_routing: false,
        sessions: None,
    });
    let app = helios_rest::create_app_with_auth_and_bulk_export(
        Arc::clone(&backend),
        config,
        AuthConfig::default(),
        Some(auth),
        None,
        BulkExportBundle {
            jobs: backend.clone() as Arc<dyn BulkExportJobStore>,
            output: output as Arc<dyn ExportOutputStore>,
            file_auth: Arc::new(BearerScopeAuth),
        },
    );
    TestServer::new(app).expect("test server")
}

const AUTHORIZATION: HeaderName = HeaderName::from_static("authorization");
const PREFER: HeaderName = HeaderName::from_static("prefer");

fn bearer(token: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("Bearer {token}")).unwrap()
}

fn local_path(url: &str) -> String {
    url.strip_prefix(BASE_URL)
        .unwrap_or_else(|| panic!("{url} is not under {BASE_URL}"))
        .to_string()
}

fn content_location(response: &axum_test::TestResponse) -> String {
    local_path(
        response
            .headers()
            .get("content-location")
            .expect("Content-Location")
            .to_str()
            .unwrap(),
    )
}

async fn get(server: &TestServer, path: &str, token: &str) -> axum_test::TestResponse {
    server
        .get(path)
        .add_header(AUTHORIZATION, bearer(token))
        .await
}

/// A `user/Observation.rs` token can start a `$export` of Observation and then
/// poll it; another principal with the same scope cannot see it, and a
/// wildcard reader can.
#[tokio::test]
async fn the_owner_of_an_export_can_poll_it_with_a_single_type_scope() {
    let server = server();
    let kickoff = server
        .get("/$export?_type=Observation")
        .add_header(AUTHORIZATION, bearer(OWNER))
        .add_header(PREFER, HeaderValue::from_static("respond-async"))
        .await;
    assert_eq!(
        kickoff.status_code(),
        StatusCode::ACCEPTED,
        "{}",
        kickoff.text()
    );
    let status = content_location(&kickoff);
    assert!(status.starts_with("/export-status/"), "{status}");

    let owner = get(&server, &status, OWNER).await;
    assert_eq!(
        owner.status_code(),
        StatusCode::ACCEPTED,
        "{}",
        owner.text()
    );
    let other = get(&server, &status, OTHER).await;
    assert_eq!(
        other.status_code(),
        StatusCode::NOT_FOUND,
        "{}",
        other.text()
    );
    let wildcard = get(&server, &status, OTHER_WILDCARD).await;
    assert_eq!(
        wildcard.status_code(),
        StatusCode::ACCEPTED,
        "{}",
        wildcard.text()
    );

    let cancel = server
        .delete(&status)
        .add_header(AUTHORIZATION, bearer(OTHER))
        .await;
    assert_eq!(
        cancel.status_code(),
        StatusCode::NOT_FOUND,
        "{}",
        cancel.text()
    );
}

/// `$sql-export` records who started it: the owner, with nothing but
/// `user/Observation.rs`, can poll, read the result and download the file;
/// another principal with the same scope gets 404 on all of them and cannot
/// cancel the job; a wildcard reader can read it.
#[tokio::test]
async fn a_sql_export_is_visible_to_its_owner_and_not_to_another_principal() {
    let server = server();
    let seeded = server
        .post("/Observation")
        .add_header(AUTHORIZATION, bearer(WRITER))
        .json(&json!({"resourceType": "Observation", "status": "final", "code": {"text": "x"}}))
        .await;
    assert_eq!(
        seeded.status_code(),
        StatusCode::CREATED,
        "{}",
        seeded.text()
    );

    let kickoff = server
        .post("/$sql-export")
        .add_header(AUTHORIZATION, bearer(OWNER))
        .add_header(PREFER, HeaderValue::from_static("respond-async"))
        .json(&json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            "resource": "Observation",
            "select": [{"column": [{"name": "id", "path": "id"}]}]
        }))
        .await;
    assert_eq!(
        kickoff.status_code(),
        StatusCode::ACCEPTED,
        "{}",
        kickoff.text()
    );
    let status = content_location(&kickoff);
    assert!(status.ends_with("/status"), "{status}");

    let other = get(&server, &status, OTHER).await;
    assert_eq!(
        other.status_code(),
        StatusCode::NOT_FOUND,
        "{}",
        other.text()
    );

    let mut done = None;
    for _ in 0..100 {
        let poll = get(&server, &status, OWNER).await;
        match poll.status_code() {
            StatusCode::ACCEPTED => tokio::time::sleep(Duration::from_millis(50)).await,
            StatusCode::SEE_OTHER => {
                done = Some(local_path(
                    poll.headers()
                        .get("location")
                        .expect("Location")
                        .to_str()
                        .unwrap(),
                ));
                break;
            }
            other => panic!("unexpected poll status {other}: {}", poll.text()),
        }
    }
    let result = done.expect("the export finishes");

    let manifest = get(&server, &result, OWNER).await;
    assert_eq!(
        manifest.status_code(),
        StatusCode::OK,
        "{}",
        manifest.text()
    );
    let other = get(&server, &result, OTHER).await;
    assert_eq!(
        other.status_code(),
        StatusCode::NOT_FOUND,
        "{}",
        other.text()
    );
    let wildcard = get(&server, &result, OTHER_WILDCARD).await;
    assert_eq!(
        wildcard.status_code(),
        StatusCode::OK,
        "{}",
        wildcard.text()
    );

    let manifest: Value = manifest.json();
    let file = manifest["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["name"] == "output")
        .flat_map(|p| p["part"].as_array().unwrap())
        .find(|part| part["name"] == "location")
        .and_then(|part| part["valueUri"].as_str())
        .map(local_path)
        .unwrap_or_else(|| panic!("an output file in {manifest}"));

    let download = get(&server, &file, OWNER).await;
    assert_eq!(
        download.status_code(),
        StatusCode::OK,
        "{}",
        download.text()
    );
    let other = get(&server, &file, OTHER).await;
    assert_eq!(
        other.status_code(),
        StatusCode::NOT_FOUND,
        "{}",
        other.text()
    );

    let cancel = server
        .delete(&status)
        .add_header(AUTHORIZATION, bearer(OTHER))
        .await;
    assert_eq!(
        cancel.status_code(),
        StatusCode::NOT_FOUND,
        "{}",
        cancel.text()
    );
    let owner_cancel = server
        .delete(&status)
        .add_header(AUTHORIZATION, bearer(OWNER))
        .await;
    assert_eq!(
        owner_cancel.status_code(),
        StatusCode::ACCEPTED,
        "{}",
        owner_cancel.text()
    );
}
