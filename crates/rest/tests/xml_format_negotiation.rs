//! Asking for FHIR XML from a build without the `xml` feature is `406 Not
//! Acceptable` on every response path, never a `500`. The format helper
//! already refused; the handlers used to swallow that refusal into a
//! "Failed to serialize response" internal error, so a client sending
//! `Accept: application/fhir+xml` (or `_format=xml`) to a read, search,
//! vread, create, update, validate or capability statement got a `500`
//! while history and delete answered `406`. With the feature on, the same
//! requests answer FHIR XML.

use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use axum_test::TestServer;
use helios_rest::ServerConfig;

const ACCEPT: HeaderName = HeaderName::from_static("accept");
const PREFER: HeaderName = HeaderName::from_static("prefer");
const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
const FHIR_XML: HeaderValue = HeaderValue::from_static("application/fhir+xml");
const FHIR_JSON: HeaderValue = HeaderValue::from_static("application/fhir+json");

const PATIENT: &str = r#"{"resourceType":"Patient","id":"p1","name":[{"family":"Probe"}]}"#;
const NEW_PATIENT: &str = r#"{"resourceType":"Patient","name":[{"family":"New"}]}"#;

fn server() -> TestServer {
    let backend =
        helios_persistence::backends::sqlite::SqliteBackend::in_memory().expect("in-memory sqlite");
    backend.init_schema().expect("init schema");
    let state = helios_rest::AppState::new(Arc::new(backend), ServerConfig::for_testing());
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(state)).expect("test server")
}

async fn seed(server: &TestServer) {
    server
        .put("/Patient/p1")
        .add_header(CONTENT_TYPE, FHIR_JSON)
        .add_header(ACCEPT, FHIR_JSON)
        .bytes(PATIENT.into())
        .await
        .assert_status(StatusCode::CREATED);
}

/// One response path each: what it is, how to reach it, and what it answers
/// when XML can be produced.
struct Case {
    label: &'static str,
    method: Method,
    path: &'static str,
    body: Option<&'static str>,
    prefer: Option<&'static str>,
    #[cfg_attr(not(feature = "xml"), allow(dead_code))]
    produced: StatusCode,
}

fn cases() -> Vec<Case> {
    let case = |label, method, path, body, produced| Case {
        label,
        method,
        path,
        body,
        prefer: None,
        produced,
    };
    // The `Prefer: return=OperationOutcome` answers of create and update are
    // formatted by their own call site.
    let outcome = |label, method, path, body, produced| Case {
        label,
        method,
        path,
        body,
        prefer: Some("return=OperationOutcome"),
        produced,
    };
    vec![
        case("read", Method::GET, "/Patient/p1", None, StatusCode::OK),
        case(
            "read via _format",
            Method::GET,
            "/Patient/p1?_format=xml",
            None,
            StatusCode::OK,
        ),
        case(
            "vread",
            Method::GET,
            "/Patient/p1/_history/1",
            None,
            StatusCode::OK,
        ),
        case(
            "search",
            Method::GET,
            "/Patient?_id=p1",
            None,
            StatusCode::OK,
        ),
        case(
            "search via _format",
            Method::GET,
            "/Patient?_format=xml",
            None,
            StatusCode::OK,
        ),
        case(
            "type history",
            Method::GET,
            "/Patient/_history",
            None,
            StatusCode::OK,
        ),
        case(
            "capabilities",
            Method::GET,
            "/metadata",
            None,
            StatusCode::OK,
        ),
        case(
            "create",
            Method::POST,
            "/Patient",
            Some(NEW_PATIENT),
            StatusCode::CREATED,
        ),
        case(
            "update",
            Method::PUT,
            "/Patient/p1",
            Some(PATIENT),
            StatusCode::OK,
        ),
        outcome(
            "create, return=OperationOutcome",
            Method::POST,
            "/Patient",
            Some(NEW_PATIENT),
            StatusCode::CREATED,
        ),
        outcome(
            "update, return=OperationOutcome",
            Method::PUT,
            "/Patient/p1",
            Some(PATIENT),
            StatusCode::OK,
        ),
        case(
            "validate",
            Method::POST,
            "/Patient/$validate",
            Some(PATIENT),
            StatusCode::OK,
        ),
    ]
}

async fn send(server: &TestServer, case: &Case) -> axum_test::TestResponse {
    let mut request = server
        .method(case.method.clone(), case.path)
        .add_header(ACCEPT, FHIR_XML);
    if let Some(prefer) = case.prefer {
        request = request.add_header(PREFER, HeaderValue::from_static(prefer));
    }
    if let Some(body) = case.body {
        request = request
            .add_header(CONTENT_TYPE, FHIR_JSON)
            .bytes(body.into());
    }
    request.await
}

#[cfg(not(feature = "xml"))]
#[tokio::test]
async fn every_xml_request_is_406_when_xml_is_not_built() {
    let server = server();
    seed(&server).await;
    for case in cases() {
        let response = send(&server, &case).await;
        let text = response.text();
        assert_eq!(
            response.status_code(),
            StatusCode::NOT_ACCEPTABLE,
            "{}: {text}",
            case.label
        );
        assert!(
            text.contains("OperationOutcome") && text.contains("XML"),
            "{}: the refusal names the format: {text}",
            case.label
        );
    }
}

#[cfg(feature = "xml")]
#[tokio::test]
async fn every_xml_request_answers_xml_when_xml_is_built() {
    let server = server();
    seed(&server).await;
    for case in cases() {
        let response = send(&server, &case).await;
        let text = response.text();
        assert_eq!(
            response.status_code(),
            case.produced,
            "{}: {text}",
            case.label
        );
        assert!(
            text.trim_start().starts_with('<'),
            "{}: the body is XML: {text}",
            case.label
        );
    }
}
