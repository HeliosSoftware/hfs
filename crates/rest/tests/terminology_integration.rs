//! Integration tests for HTS (Helios Terminology Server) delegation.
//!
//! These tests verify that:
//! 1. Search parameters with `:in` modifiers are expanded via `POST /ValueSet/$expand`
//!    on a configured terminology server and the expanded codes replace the original param.
//! 2. Search parameters with `:not-in` modifiers are gracefully dropped (fail-open)
//!    because the SQLite backend does not support negated value-set filtering.
//! 3. When no terminology server is configured, `:in` / `:not-in` params pass through
//!    to the persistence layer unchanged.
//!
//! # How the mock server works
//!
//! Each test spins up a lightweight Axum server on an ephemeral port (`0`) that
//! handles `POST /ValueSet/$expand`.  HFS is configured to point at this server
//! via the `terminology_server` field of `ServerConfig`.
//!
//! The mock server returns a FHIR ValueSet with a canned expansion, letting us
//! assert that the token filter injected into the search query matches the codes
//! from that expansion.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum_test::TestServer;
use helios_persistence::backends::sqlite::SqliteBackend;
use serde_json::{Value, json};
use tokio::net::TcpListener;

use helios_rest::{ServerConfig, create_app_with_config};

// ─── Mock HTS server ─────────────────────────────────────────────────────────

/// Shared state for the mock HTS.
#[derive(Clone, Default)]
struct MockHtsState {
    /// Requests received by the mock server.
    requests: Arc<Mutex<Vec<Value>>>,
    /// Canned expansion to return for any $expand call.
    expansion: Arc<Value>,
}

/// Handler for `POST /ValueSet/$expand`.
async fn mock_expand_handler(
    State(state): State<MockHtsState>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    state.requests.lock().unwrap().push(body);

    (StatusCode::OK, Json((*state.expansion).clone()))
}

/// Starts a mock HTS Axum server on an ephemeral port.
///
/// Returns `(base_url, shared_requests)` where `shared_requests` can be
/// inspected after sending requests to HFS.
async fn start_mock_hts(expansion: Value) -> (String, Arc<Mutex<Vec<Value>>>) {
    let requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(vec![]));

    let state = MockHtsState {
        requests: Arc::clone(&requests),
        expansion: Arc::new(expansion),
    };

    let router = Router::new()
        .route(
            "/ValueSet/$expand",
            axum::routing::post(mock_expand_handler),
        )
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{}", port);

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    (base_url, requests)
}

/// Creates a minimal FHIR ValueSet expansion response with the given codes.
fn make_expansion(system: &str, codes: &[&str]) -> Value {
    let contains: Vec<Value> = codes
        .iter()
        .map(|code| {
            json!({
                "system": system,
                "code": code,
                "display": code
            })
        })
        .collect();

    json!({
        "resourceType": "ValueSet",
        "expansion": {
            "total": contains.len(),
            "contains": contains
        }
    })
}

// ─── Helper: in-memory HFS backed by SQLite ──────────────────────────────────

fn create_hfs_with_terminology_server(ts_url: &str) -> TestServer {
    let backend = SqliteBackend::in_memory().expect("SQLite in-memory failed");
    backend.init_schema().expect("Schema init failed");

    let config = ServerConfig {
        terminology_server: Some(ts_url.to_string()),
        ..ServerConfig::for_testing()
    };

    let app = create_app_with_config(backend, config);
    TestServer::new(app).expect("TestServer init failed")
}

// ─── Tests ───────────────────────────────────────────────────────────────────

/// When a `:in` search parameter is used and a terminology server is configured,
/// HFS should call `POST /ValueSet/$expand` on the HTS and translate the
/// expansion to a plain token search (so the storage backend receives codes,
/// not a ValueSet URL).
///
/// We verify this indirectly: the mock HTS records the expansion request, and
/// HFS issues a search against the backend that the SQLite layer can process.
#[tokio::test]
async fn test_in_modifier_calls_hts_expand() {
    let expansion = make_expansion("http://example.org/cs", &["A", "B", "C"]);
    let (ts_url, requests) = start_mock_hts(expansion).await;

    let server = create_hfs_with_terminology_server(&ts_url);

    // Send a search with :in modifier.  The response will be an empty bundle
    // (no patients seeded) but the HTS should have received one $expand call.
    let response = server
        .get("/Patient")
        .add_query_param("code:in", "http://example.org/vs")
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    // Verify the mock HTS received exactly one $expand request.
    let reqs = requests.lock().unwrap();
    assert_eq!(reqs.len(), 1, "Expected exactly one $expand call to HTS");

    // Verify the URL parameter in the request body.
    let params_array = reqs[0]["parameter"].as_array().unwrap();
    let url_param = params_array
        .iter()
        .find(|p| p["name"] == "url")
        .expect("Expected 'url' parameter in $expand request");
    assert_eq!(url_param["valueUri"], "http://example.org/vs");
}

/// When no terminology server is configured, a token `:in` modifier cannot be
/// satisfied and must be rejected with `501` rather than silently falling
/// through to literal matching (which would return misleading results).
#[tokio::test]
async fn test_in_modifier_returns_not_implemented_without_terminology_server() {
    let backend = SqliteBackend::in_memory().expect("SQLite in-memory failed");
    backend.init_schema().expect("Schema init failed");

    // No terminology_server set
    let config = ServerConfig::for_testing();
    let app = create_app_with_config(backend, config);
    let server = TestServer::new(app).expect("TestServer init failed");

    let response = server
        .get("/Patient")
        .add_query_param("code:in", "http://example.org/vs")
        .await;

    // Fail loud: `:in` needs a terminology server, so it returns 501 (no HTTP
    // call to an external server is made because none is configured).
    assert_eq!(
        response.status_code().as_u16(),
        501,
        "Expected 501 for :in without a configured terminology server"
    );
}

/// When a `:not-in` parameter is used, HFS returns 501 Not Implemented rather
/// than silently dropping the filter (which would return incorrect results).
#[tokio::test]
async fn test_not_in_modifier_returns_not_implemented() {
    let expansion = make_expansion("http://example.org/cs", &["X", "Y"]);
    let (ts_url, _requests) = start_mock_hts(expansion).await;

    let server = create_hfs_with_terminology_server(&ts_url);

    let response = server
        .get("/Patient")
        .add_query_param("code:not-in", "http://example.org/vs")
        .await;

    // :not-in must return an explicit error rather than silently returning
    // all resources as if the filter wasn't applied.
    assert_eq!(response.status_code(), StatusCode::NOT_IMPLEMENTED);
}

/// When the HTS is unreachable, `:in` parameters are dropped (fail-open) and
/// the search continues without the filter.
#[tokio::test]
async fn test_in_modifier_fails_open_on_hts_unavailable() {
    // Use a port that has nothing listening on it.
    let ts_url = "http://127.0.0.1:19999";

    let server = create_hfs_with_terminology_server(ts_url);

    let response = server
        .get("/Patient")
        .add_query_param("code:in", "http://example.org/vs")
        .add_query_param("name", "Smith")
        .await;

    // Even with a bad HTS, the search should complete (name param still applied).
    assert_eq!(response.status_code(), StatusCode::OK);

    let body: Value = response.json();
    assert_eq!(body["resourceType"], "Bundle");
}

/// With a terminology server configured, a terminology-backed modifier on the
/// terminal parameter of a chained or `_has` search is expanded like a direct
/// one, and the chain resolves against the expanded codes (#1317: the `501` a
/// server *without* terminology gives these must not reach this path).
#[tokio::test]
async fn test_chained_in_and_below_modifiers_resolve_with_terminology_server() {
    let expansion = make_expansion("http://loinc.org", &["1234-5"]);
    let (ts_url, requests) = start_mock_hts(expansion).await;

    // The spec search parameters are needed here: the embedded fallback set
    // knows neither `code` nor `subject`, so every chain would match nothing.
    let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let backend = SqliteBackend::with_config(
        ":memory:",
        helios_persistence::backends::sqlite::SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("SQLite in-memory failed");
    backend.init_schema().expect("Schema init failed");
    let config = ServerConfig {
        terminology_server: Some(ts_url),
        ..ServerConfig::for_testing()
    };
    let server = TestServer::new(create_app_with_config(backend, config)).unwrap();

    for resource in [
        json!({"resourceType": "Patient", "id": "p-match"}),
        json!({"resourceType": "Patient", "id": "p-other"}),
        json!({"resourceType": "Observation", "id": "o-match", "status": "final",
               "code": {"coding": [{"system": "http://loinc.org", "code": "1234-5"}]},
               "subject": {"reference": "Patient/p-match"}}),
        json!({"resourceType": "Observation", "id": "o-other", "status": "final",
               "code": {"coding": [{"system": "http://loinc.org", "code": "9999-9"}]},
               "subject": {"reference": "Patient/p-other"}}),
        json!({"resourceType": "DiagnosticReport", "id": "d-match", "status": "final",
               "code": {"text": "report"},
               "result": [{"reference": "Observation/o-match"}]}),
        json!({"resourceType": "DiagnosticReport", "id": "d-other", "status": "final",
               "code": {"text": "report"},
               "result": [{"reference": "Observation/o-other"}]}),
    ] {
        let path = format!(
            "/{}/{}",
            resource["resourceType"].as_str().unwrap(),
            resource["id"].as_str().unwrap()
        );
        let response = server.put(&path).json(&resource).await;
        assert!(response.status_code().is_success(), "seeding {path}");
    }

    let ids = |body: &Value| -> Vec<String> {
        let mut ids: Vec<String> = body["entry"]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .map(|e| e["resource"]["id"].as_str().unwrap().to_string())
                    .collect()
            })
            .unwrap_or_default();
        ids.sort();
        ids
    };

    // Positive control: the plain chain finds both reports.
    let response = server
        .get("/DiagnosticReport")
        .add_query_param("result.status", "final")
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    assert_eq!(ids(&response.json()), ["d-match", "d-other"]);

    for (resource_type, key, value, expected) in [
        (
            "DiagnosticReport",
            "result.code:in",
            "http://example.org/vs",
            "d-match",
        ),
        (
            "DiagnosticReport",
            "result:Observation.code:in",
            "http://example.org/vs",
            "d-match",
        ),
        (
            "Patient",
            "_has:Observation:subject:code:in",
            "http://example.org/vs",
            "p-match",
        ),
        (
            "DiagnosticReport",
            "result:Observation.code:below",
            "http://loinc.org|1234-5",
            "d-match",
        ),
        (
            "Patient",
            "_has:Observation:subject:code:below",
            "http://loinc.org|1234-5",
            "p-match",
        ),
    ] {
        let before = requests.lock().unwrap().len();
        let response = server
            .get(&format!("/{resource_type}"))
            .add_query_param(key, value)
            .await;
        assert_eq!(response.status_code(), StatusCode::OK, "{key}");
        assert_eq!(ids(&response.json()), [expected], "{key}");
        assert_eq!(
            requests.lock().unwrap().len(),
            before + 1,
            "{key}: expected one $expand call"
        );
    }
}
