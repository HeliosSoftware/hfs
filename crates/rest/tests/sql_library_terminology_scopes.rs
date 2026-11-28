//! A SQL Library's ValueSet and ConceptMap dependencies under SMART scopes
//! (#2662): `$sql-run` and `$sql-export` need a Search grant on the kind each
//! one resolves to, as for the resource type a ViewDefinition reads, and a
//! token that may search neither kind is refused before the terminology
//! server is asked for it.

use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use helios_audit::ExclusionFilter;
use helios_auth::{AuthConfig, AuthError, AuthProvider, Principal, ScopeSet};
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_rest::{AuthMiddlewareState, ServerConfig};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const VS_URL: &str = "http://example.org/ValueSet/cardio";
const LIB_TYPE_SYSTEM: &str = "https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes";
const AUTHORIZATION: HeaderName = HeaderName::from_static("authorization");
const PREFER: HeaderName = HeaderName::from_static("prefer");

struct StubProvider;

#[async_trait]
impl AuthProvider for StubProvider {
    /// The bearer token is the scope string itself.
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError> {
        let scopes = authorization_header
            .strip_prefix("Bearer ")
            .ok_or(AuthError::InvalidSignature)?;
        Ok(Principal {
            subject: "demo-sub".to_string(),
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

/// A terminology server whose only ValueSet expands to two codes.
async fn terminology_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/ValueSet/$expand"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "ValueSet",
            "url": VS_URL,
            "expansion": {"total": 2, "contains": [
                {"system": "http://ids", "code": "a"},
                {"system": "http://ids", "code": "b"}
            ]}
        })))
        .mount(&server)
        .await;
    server
}

/// The server under test, and the directory its `$sql-export` jobs write to:
/// a temporary one, so a run leaves nothing in the source tree.
fn server(terminology: &MockServer) -> (TestServer, tempfile::TempDir) {
    let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
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
    let mut config = ServerConfig::for_testing();
    config.terminology_server = Some(terminology.uri());
    let exports = tempfile::tempdir().expect("export directory");
    config.export_dir = exports.path().to_string_lossy().into_owned();
    let app =
        helios_rest::create_app_with_auth(backend, config, AuthConfig::default(), Some(auth), None);
    (TestServer::new(app).expect("test server"), exports)
}

/// A SQLQuery Library that reads one ValueSet and nothing else.
fn value_set_library() -> Value {
    json!({
        "resourceType": "Library",
        "status": "active",
        "type": {"coding": [{"system": LIB_TYPE_SYSTEM, "code": "sql-query"}]},
        "content": [{
            "contentType": "application/sql",
            "data": B64.encode("SELECT code FROM cardio ORDER BY code")
        }],
        "relatedArtifact": [{"type": "depends-on", "label": "cardio", "resource": VS_URL}]
    })
}

fn bearer(scopes: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("Bearer {scopes}")).unwrap()
}

async fn sql_run(server: &TestServer, scopes: &str) -> (StatusCode, String) {
    let response = server
        .post("/$sql-run")
        .add_header(AUTHORIZATION, bearer(scopes))
        .json(&json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": "json"},
                {"name": "subjectResource", "resource": value_set_library()}
            ]
        }))
        .await;
    (response.status_code(), response.text())
}

async fn sql_export(server: &TestServer, scopes: &str) -> (StatusCode, String) {
    let response = server
        .post("/$sql-export")
        .add_header(AUTHORIZATION, bearer(scopes))
        .add_header(PREFER, HeaderValue::from_static("respond-async"))
        .json(&json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "subject", "part": [
                {"name": "name", "valueString": "cardio"},
                {"name": "subjectResource", "resource": value_set_library()}
            ]}]
        }))
        .await;
    (response.status_code(), response.text())
}

#[tokio::test]
async fn a_token_that_may_search_no_terminology_never_reaches_the_terminology_server() {
    let ts = terminology_server().await;
    let (server, _exports) = server(&ts);
    for scopes in ["openid", "user/Observation.rs", "patient/*.rs"] {
        let (status, body) = sql_run(&server, scopes).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "$sql-run as {scopes}: {body}"
        );
        if !scopes.starts_with("patient/") {
            assert!(body.contains(VS_URL), "{body}");
        }
        let (status, body) = sql_export(&server, scopes).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "$sql-export as {scopes}: {body}"
        );
    }
    let asked = ts.received_requests().await.unwrap_or_default();
    assert!(
        asked.is_empty(),
        "the terminology server was asked {} time(s) for a token that may read no terminology",
        asked.len()
    );
}

#[tokio::test]
async fn a_value_set_grant_reads_a_value_set_dependency() {
    let ts = terminology_server().await;
    let (server, _exports) = server(&ts);
    for scopes in ["user/ValueSet.rs", "user/*.rs"] {
        let (status, body) = sql_run(&server, scopes).await;
        assert_eq!(status, StatusCode::OK, "{scopes}: {body}");
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap(),
            json!([{"code": "a"}, {"code": "b"}])
        );
        let (status, body) = sql_export(&server, scopes).await;
        assert_ne!(status, StatusCode::FORBIDDEN, "{scopes}: {body}");
    }
}

#[tokio::test]
async fn a_concept_map_grant_does_not_read_a_value_set_dependency() {
    let ts = terminology_server().await;
    let (server, _exports) = server(&ts);
    let (status, body) = sql_run(&server, "user/ConceptMap.rs").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("ValueSet"), "{body}");
    let (status, body) = sql_export(&server, "user/ConceptMap.rs").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("ValueSet"), "{body}");
}

/// A dependency no ViewDefinition or Library answers to is refused for a
/// token with no terminology grant, as before, but the refusal does not claim
/// the URL is terminology: it may simply be mistyped.
#[tokio::test]
async fn an_unresolved_dependency_is_not_called_terminology() {
    let ts = MockServer::start().await;
    let (server, _exports) = server(&ts);
    let typo = "http://example.org/ViewDefinition/typo";
    let mut library = value_set_library();
    library["relatedArtifact"][0]["resource"] = json!(typo);
    let run = |scopes: &'static str| {
        let request = server
            .post("/$sql-run")
            .add_header(AUTHORIZATION, bearer(scopes))
            .json(&json!({
                "resourceType": "Parameters",
                "parameter": [
                    {"name": "_format", "valueCode": "json"},
                    {"name": "subjectResource", "resource": library.clone()}
                ]
            }));
        async move {
            let response = request.await;
            (response.status_code(), response.text())
        }
    };

    let (status, body) = run("user/Patient.rs").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains(typo), "{body}");
    assert!(
        body.contains("not a stored ViewDefinition or Library"),
        "{body}"
    );
    assert!(
        ts.received_requests().await.unwrap_or_default().is_empty(),
        "the terminology server was asked for a token that may read no terminology"
    );

    // A token that may read terminology learns that nothing answers.
    let (status, body) = run("user/ValueSet.rs").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}
