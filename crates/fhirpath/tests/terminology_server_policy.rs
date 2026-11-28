//! fhirpath-server sends terminology requests only to servers its operator
//! allows. A request's `terminologyServer` parameter used to become the base
//! URL of the terminology client whatever it named, so any caller could make
//! the server send requests to hosts it can reach and read the answers.

use axum::http::StatusCode;
use axum_test::TestServer;
use helios_fhirpath::handlers::TerminologyServerPolicy;
use helios_fhirpath::server::{ServerConfig, create_app};
use serde_json::{Value, json};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn request(terminology_server: &str) -> Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "expression", "valueString": "%terminologies.expand('http://example.org/vs')"},
            {"name": "resource", "resource": {"resourceType": "Patient", "id": "p"}},
            {"name": "terminologyServer", "valueString": terminology_server}
        ]
    })
}

async fn stub() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "ValueSet",
            "expansion": {"contains": []}
        })))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn an_unlisted_terminology_server_is_refused_before_any_request() {
    let terminology = stub().await;
    let server = TestServer::new(create_app(&ServerConfig::default())).unwrap();

    let response = server.post("/").json(&request(&terminology.uri())).await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    assert!(
        response
            .text()
            .contains("FHIRPATH_SERVER_ALLOWED_TERMINOLOGY_SERVERS"),
        "{}",
        response.text()
    );
    assert!(terminology.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_listed_terminology_server_is_used() {
    let terminology = stub().await;
    let config = ServerConfig {
        allowed_terminology_servers: vec![format!("{}/", terminology.uri())],
        ..ServerConfig::default()
    };
    let server = TestServer::new(create_app(&config)).unwrap();

    let response = server.post("/").json(&request(&terminology.uri())).await;

    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    assert!(!terminology.received_requests().await.unwrap().is_empty());
}

#[test]
fn the_operators_own_server_is_always_allowed_and_urls_compare_once_parsed() {
    let policy = TerminologyServerPolicy::new(
        Some("https://tx.example.org/fhir"),
        &["https://Other.example.org/r4/".to_string()],
    );
    assert!(policy.allows("https://tx.example.org/fhir/"));
    assert!(policy.allows("https://other.example.org/r4"));
    assert!(!policy.allows("https://tx.example.org/fhir/extra"));
    assert!(!policy.allows("http://tx.example.org/fhir"));
    assert!(!policy.allows("http://169.254.169.254/latest"));
    assert!(!TerminologyServerPolicy::default().allows("https://tx.example.org/fhir"));
}
