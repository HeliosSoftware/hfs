//! `DELETE [type]/[id]/_history` and `DELETE [type]/[id]/_history/[vid]`
//! are refused to a `patient/` scope: the versions they erase may predate the
//! resource joining the launch patient's compartment, and the same token
//! cannot read those. A grant from a `user/` or `system/` scope is unchanged.
//! Over SQLite, the one standalone backend in CI that implements the two
//! trial-use deletes.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_audit::ExclusionFilter;
use helios_auth::{AuthConfig, AuthError, AuthProvider, LaunchContext, Principal, ScopeSet};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::{AuthMiddlewareState, ServerConfig};
use serde_json::{Value, json};

/// `system/*.cruds`: seeds and inspects the data.
const SYSTEM: &str = "system-token";
/// `patient/*.cruds` with `patient = p1`.
const P1_WRITE: &str = "p1-write-token";
/// `patient/Observation.cruds user/Patient.rs` with `patient = p1`: the
/// Observation grant still rests on the `patient/` scope.
const P1_MIXED: &str = "p1-mixed-token";
/// `user/Observation.d`: an unconfined delete grant on Observation only.
const OBS_DELETE: &str = "obs-delete-token";

struct StubProvider;

#[async_trait]
impl AuthProvider for StubProvider {
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError> {
        let token = authorization_header
            .strip_prefix("Bearer ")
            .ok_or(AuthError::InvalidSignature)?;
        let (scopes, patient) = match token {
            SYSTEM => ("system/*.cruds", None),
            P1_WRITE => ("patient/*.cruds", Some("p1")),
            P1_MIXED => ("patient/Observation.cruds user/Patient.rs", Some("p1")),
            OBS_DELETE => ("user/Observation.d", None),
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

async fn put(server: &TestServer, resource: Value) {
    let rt = resource["resourceType"].as_str().unwrap();
    let id = resource["id"].as_str().unwrap();
    let response = server
        .put(&format!("/{rt}/{id}"))
        .add_header(AUTHORIZATION, bearer(SYSTEM))
        .json(&resource)
        .await;
    assert!(
        response.status_code().is_success(),
        "PUT {rt}/{id}: {}",
        response.text()
    );
}

/// Two patients, each with an Observation that has two versions.
async fn seed(server: &TestServer) {
    for p in ["p1", "p2"] {
        put(server, json!({"resourceType": "Patient", "id": p})).await;
        for status in ["preliminary", "final"] {
            put(
                server,
                json!({"resourceType": "Observation", "id": format!("obs-{p}"), "status": status,
                    "code": {"text": "x"}, "subject": {"reference": format!("Patient/{p}")}}),
            )
            .await;
        }
    }
}

async fn history_total(server: &TestServer, id: &str) -> u64 {
    let response = server
        .get(&format!("/Observation/{id}/_history"))
        .add_header(AUTHORIZATION, bearer(SYSTEM))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    response.json::<Value>()["total"].as_u64().unwrap_or(0)
}

#[tokio::test]
async fn a_patient_scope_cannot_erase_another_patients_history() {
    let server = server();
    seed(&server).await;

    let all = server
        .delete("/Observation/obs-p2/_history")
        .add_header(AUTHORIZATION, bearer(P1_WRITE))
        .await;
    assert_eq!(all.status_code(), StatusCode::FORBIDDEN, "{}", all.text());
    assert!(
        all.text().contains("compartment"),
        "the refusal names the compartment: {}",
        all.text()
    );

    let one = server
        .delete("/Observation/obs-p2/_history/1")
        .add_header(AUTHORIZATION, bearer(P1_WRITE))
        .await;
    assert_eq!(one.status_code(), StatusCode::FORBIDDEN, "{}", one.text());

    assert_eq!(
        history_total(&server, "obs-p2").await,
        2,
        "nothing was erased"
    );
}

/// Even the launch patient's own history: the resource is in the compartment
/// now, but its earlier versions may not have been.
#[tokio::test]
async fn a_patient_scope_cannot_erase_its_own_patients_history_either() {
    let server = server();
    seed(&server).await;
    // obs-moved was p2's for its first two versions and is p1's now.
    for (status, p) in [("preliminary", "p2"), ("final", "p2"), ("amended", "p1")] {
        put(
            &server,
            json!({"resourceType": "Observation", "id": "obs-moved", "status": status,
                "code": {"text": "x"}, "subject": {"reference": format!("Patient/{p}")}}),
        )
        .await;
    }

    for token in [P1_WRITE, P1_MIXED] {
        for path in [
            "/Observation/obs-p1/_history/1",
            "/Observation/obs-p1/_history",
            "/Observation/obs-moved/_history/1",
            "/Observation/obs-moved/_history",
        ] {
            let refused = server
                .delete(path)
                .add_header(AUTHORIZATION, bearer(token))
                .await;
            assert_eq!(
                refused.status_code(),
                StatusCode::FORBIDDEN,
                "{token} DELETE {path}: {}",
                refused.text()
            );
            assert!(
                refused.text().contains("patient/ scope"),
                "{token} DELETE {path}: {}",
                refused.text()
            );
        }
    }
    assert_eq!(
        history_total(&server, "obs-p1").await,
        2,
        "nothing was erased"
    );
    assert_eq!(
        history_total(&server, "obs-moved").await,
        3,
        "nothing was erased"
    );

    let one = server
        .delete("/Observation/obs-moved/_history/1")
        .add_header(AUTHORIZATION, bearer(OBS_DELETE))
        .await;
    assert_eq!(one.status_code(), StatusCode::NO_CONTENT, "{}", one.text());
    assert_eq!(history_total(&server, "obs-moved").await, 2);
}

#[tokio::test]
async fn an_unrestricted_caller_is_unchanged() {
    let server = server();
    seed(&server).await;

    let all = server
        .delete("/Observation/obs-p2/_history")
        .add_header(AUTHORIZATION, bearer(SYSTEM))
        .await;
    assert_eq!(all.status_code(), StatusCode::OK, "{}", all.text());
    assert_eq!(history_total(&server, "obs-p2").await, 1);
}
