//! Integration tests for HTS (Helios Terminology Server) delegation.
//!
//! These tests verify that:
//! 1. With a terminology server configured, a token `:in` parameter is expanded
//!    via `POST /ValueSet/$expand` and replaced by a plain token parameter
//!    carrying the expanded codes; token `:above` / `:below` are expanded the
//!    same way (code subsumption). This covers the terminal parameter of a
//!    chained or `_has` search too.
//! 2. `:not-in` is a `501` with or without a terminology server: no backend
//!    implements negated value-set filtering, and dropping the filter would
//!    return a superset of what was asked for.
//! 3. Without a terminology server, `:in` and token `:above` / `:below` are a
//!    `501` naming `HFS_TERMINOLOGY_SERVER` — they are not passed through to
//!    the persistence layer, which would match the ValueSet URL as a literal
//!    code. (`:above` / `:below` on a uri or reference are structural and need
//!    no terminology server.)
//! 4. A terminology modifier the parameter's type does not define (`name:in`,
//!    `subject.name:in`, `_has:Observation:subject:date:in`) is a `400` either
//!    way, and never reaches the terminology server. The terminal parameter of
//!    a chained or `_has` search is typed by the chain resolver, which calls
//!    back into the REST layer's expansion once the modifier has passed.
//! 5. When the terminology server cannot be reached, the parameter is dropped
//!    and the search continues without it (fail-open).
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

/// #1339: a terminology-backed modifier on a parameter whose type does not
/// define it (`name:in` — a string) is a `400` with a terminology server too.
/// It used to be expanded like a token's and searched as `name=<codes>`, an
/// empty `200`. The terminology server is not consulted for it.
#[tokio::test]
async fn test_terminology_modifier_on_wrong_parameter_type_is_rejected_before_expansion() {
    let expansion = make_expansion("http://example.org/cs", &["Smith"]);
    let (ts_url, requests) = start_mock_hts(expansion).await;

    // The spec search parameters are needed: the embedded fallback set does not
    // know `name` or `birthdate`, and an unregistered parameter is not gated.
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

    let patient = json!({"resourceType": "Patient", "id": "p1", "gender": "male",
                         "name": [{"family": "Smith"}]});
    let response = server.put("/Patient/p1").json(&patient).await;
    assert!(response.status_code().is_success());

    // Positive control: the parameter itself finds the patient.
    let response = server
        .get("/Patient")
        .add_query_param("name", "Smith")
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    assert_eq!(response.json::<Value>()["entry"][0]["resource"]["id"], "p1");

    for (key, value) in [
        ("name:in", "http://example.org/vs"),
        ("name:not-in", "http://example.org/vs"),
        ("name:below", "http://example.org/cs|Smith"),
        ("birthdate:above", "http://example.org/cs|1980"),
    ] {
        let response = server.get("/Patient").add_query_param(key, value).await;
        assert_eq!(response.status_code(), StatusCode::BAD_REQUEST, "{key}");
        assert!(
            response.json::<Value>()["issue"][0]
                .to_string()
                .contains("is not supported for"),
            "{key}"
        );
    }
    assert!(
        requests.lock().unwrap().is_empty(),
        "an invalid modifier must not reach the terminology server"
    );

    // A valid one still does.
    let response = server
        .get("/Patient")
        .add_query_param("gender:in", "http://example.org/vs")
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    assert_eq!(requests.lock().unwrap().len(), 1);
}

/// #1365: the same on the terminal parameter of a chained or `_has` search.
/// `subject.name:in` used to be rewritten into `subject.name=<expanded codes>`
/// before the chain resolver could type `name`, so it was a string search for
/// the codes — a silent `200`. The terminal is now typed first: an invalid
/// modifier is a `400` and never reaches the terminology server, a valid one
/// is expanded (once), a structural one (uri / reference `:below`) is left to
/// the backend, and `:not-in` is a `400` or a `501` as on a direct parameter.
#[tokio::test]
async fn test_chained_terminology_modifier_is_checked_against_the_terminal_type() {
    // One expansion for every call: a code of each value set searched below.
    let expansion = json!({
        "resourceType": "ValueSet",
        "expansion": { "contains": [
            {"system": "http://loinc.org", "code": "1234-5"},
            {"system": "http://snomed.info/sct", "code": "706767009"},
            // What the wrong-typed searches would find, were they run.
            {"system": "http://example.org/cs", "code": "Smith"},
        ]}
    });
    let (ts_url, requests) = start_mock_hts(expansion).await;

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
        json!({"resourceType": "Practitioner", "id": "gp"}),
        json!({"resourceType": "Patient", "id": "p-match", "gender": "female",
               "birthDate": "1980-01-01", "name": [{"family": "Smith"}],
               "generalPractitioner": [{"reference": "Practitioner/gp"}]}),
        json!({"resourceType": "Patient", "id": "p-other", "gender": "male",
               "name": [{"family": "Jones"}]}),
        json!({"resourceType": "Device", "id": "dev", "url": "http://dev.example.org/a/b",
               "type": {"coding": [{"system": "http://snomed.info/sct", "code": "706767009"}]}}),
        json!({"resourceType": "Observation", "id": "o-match", "status": "final",
               "code": {"coding": [{"system": "http://loinc.org", "code": "1234-5"}]},
               "effectiveDateTime": "2020-01-01",
               "subject": {"reference": "Patient/p-match"}}),
        json!({"resourceType": "Observation", "id": "o-other", "status": "final",
               "code": {"coding": [{"system": "http://loinc.org", "code": "9999-9"}]},
               "subject": {"reference": "Patient/p-other"}}),
        json!({"resourceType": "Observation", "id": "o-dev", "status": "final",
               "code": {"coding": [{"system": "http://loinc.org", "code": "9999-9"}]},
               "subject": {"reference": "Device/dev"}}),
        json!({"resourceType": "DiagnosticReport", "id": "d-match", "status": "final",
               "code": {"text": "report"},
               "result": [{"reference": "Observation/o-match"}]}),
        json!({"resourceType": "DiagnosticReport", "id": "d-other", "status": "final",
               "code": {"text": "report"},
               "result": [{"reference": "Observation/o-other"}]}),
        json!({"resourceType": "DiagnosticReport", "id": "d-dev", "status": "final",
               "code": {"text": "report"},
               "result": [{"reference": "Observation/o-dev"}]}),
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

    // Positive controls: every chain used below resolves, on the terminal
    // parameters the invalid modifiers are put on.
    for (resource_type, key, value, expected) in [
        ("Observation", "subject.name", "Smith", "o-match"),
        (
            "Observation",
            "subject:Patient.birthdate",
            "1980-01-01",
            "o-match",
        ),
        (
            "DiagnosticReport",
            "result.subject:Patient.name",
            "Smith",
            "d-match",
        ),
        (
            "Patient",
            "_has:Observation:subject:date",
            "2020-01-01",
            "p-match",
        ),
        ("Observation", "subject.type", "706767009", "o-dev"),
    ] {
        let response = server
            .get(&format!("/{resource_type}"))
            .add_query_param(key, value)
            .await;
        assert_eq!(response.status_code(), StatusCode::OK, "{key}");
        assert_eq!(ids(&response.json()), [expected], "{key}");
    }

    // Invalid for the terminal's type: `400`, with or without a `:Type`.
    for (resource_type, key, value) in [
        ("Observation", "subject.name:in", "http://example.org/vs"),
        (
            "Observation",
            "subject:Patient.name:in",
            "http://example.org/vs",
        ),
        (
            "Observation",
            "subject:Patient.name:not-in",
            "http://example.org/vs",
        ),
        (
            "Observation",
            "subject:Patient.name:below",
            "http://example.org/cs|Smith",
        ),
        (
            "Observation",
            "subject:Patient.birthdate:above",
            "http://example.org/cs|1980",
        ),
        (
            "DiagnosticReport",
            "result.subject:Patient.name:in",
            "http://example.org/vs",
        ),
        (
            "DiagnosticReport",
            "result.subject:Patient.name:above",
            "http://example.org/cs|Smith",
        ),
        (
            "Patient",
            "_has:Observation:subject:date:in",
            "http://example.org/vs",
        ),
        (
            "Patient",
            "_has:Observation:subject:date:not-in",
            "http://example.org/vs",
        ),
        (
            "Patient",
            "_has:Observation:subject:date:below",
            "http://example.org/cs|2020",
        ),
    ] {
        let response = server
            .get(&format!("/{resource_type}"))
            .add_query_param(key, value)
            .await;
        assert_eq!(response.status_code(), StatusCode::BAD_REQUEST, "{key}");
        assert!(
            response.json::<Value>()["issue"][0]
                .to_string()
                .contains("is not supported for"),
            "{key}"
        );
    }
    assert!(
        requests.lock().unwrap().is_empty(),
        "an invalid modifier must not reach the terminology server"
    );

    // Valid and structural (reference / uri `:below`): the backend's, and no
    // business of the terminology server.
    for (key, value, expected) in [
        (
            "subject:Patient.general-practitioner:below",
            "Practitioner/gp",
            "o-match",
        ),
        (
            "subject:Device.url:below",
            "http://dev.example.org/a",
            "o-dev",
        ),
    ] {
        let response = server.get("/Observation").add_query_param(key, value).await;
        assert_eq!(response.status_code(), StatusCode::OK, "{key}");
        assert_eq!(ids(&response.json()), [expected], "{key}");
    }
    assert!(requests.lock().unwrap().is_empty());

    // Valid `:not-in` on a token terminal: the direct form's `501`.
    for (resource_type, key) in [
        ("Observation", "subject:Patient.gender:not-in"),
        ("Patient", "_has:Observation:subject:code:not-in"),
    ] {
        let response = server
            .get(&format!("/{resource_type}"))
            .add_query_param(key, "http://example.org/vs")
            .await;
        assert_eq!(response.status_code(), StatusCode::NOT_IMPLEMENTED, "{key}");
        assert!(
            response.json::<Value>()["issue"][0]
                .to_string()
                .contains("search modifier ':not-in' is not supported"),
            "{key}"
        );
    }
    assert!(requests.lock().unwrap().is_empty());

    // Valid on a token terminal: expanded, one `$expand` call each. The first
    // is an untyped hop over a polymorphic reference: `type` is a token on
    // every `Observation.subject` target that defines it.
    let device_type = "http://snomed.info/sct|706767009";
    for (resource_type, key, value, expected) in [
        (
            "Observation",
            "subject.type:in",
            "http://example.org/vs",
            "o-dev",
        ),
        (
            "Observation",
            "subject:Device.type:in",
            "http://example.org/vs",
            "o-dev",
        ),
        (
            "Observation",
            "subject:Device.type:above",
            device_type,
            "o-dev",
        ),
        (
            "Observation",
            "subject:Device.type:below",
            device_type,
            "o-dev",
        ),
        (
            "DiagnosticReport",
            "result.subject:Device.type:in",
            "http://example.org/vs",
            "d-dev",
        ),
        (
            "DiagnosticReport",
            "result.code:below",
            "http://loinc.org|1234-5",
            "d-match",
        ),
        (
            "Patient",
            "_has:Observation:subject:code:in",
            "http://example.org/vs",
            "p-match",
        ),
        (
            "Patient",
            "_has:Observation:subject:code:above",
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

/// When the terminology server is unreachable, a chained `:in` fails open as a
/// direct one does: the chained parameter is dropped and the search goes on
/// without it.
#[tokio::test]
async fn test_chained_in_modifier_fails_open_on_hts_unavailable() {
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
        // Nothing listens here.
        terminology_server: Some("http://127.0.0.1:19999".to_string()),
        ..ServerConfig::for_testing()
    };
    let server = TestServer::new(create_app_with_config(backend, config)).unwrap();

    for resource in [
        json!({"resourceType": "Patient", "id": "p1", "gender": "female"}),
        json!({"resourceType": "Observation", "id": "o1", "status": "final",
               "code": {"coding": [{"system": "http://loinc.org", "code": "1234-5"}]},
               "subject": {"reference": "Patient/p1"}}),
        json!({"resourceType": "Observation", "id": "o2", "status": "final",
               "code": {"coding": [{"system": "http://loinc.org", "code": "9999-9"}]}}),
    ] {
        let path = format!(
            "/{}/{}",
            resource["resourceType"].as_str().unwrap(),
            resource["id"].as_str().unwrap()
        );
        assert!(
            server
                .put(&path)
                .json(&resource)
                .await
                .status_code()
                .is_success()
        );
    }

    // Positive control: the chain constrains the search.
    let response = server
        .get("/Observation")
        .add_query_param("subject.gender", "female")
        .await;
    assert_eq!(
        response.json::<Value>()["entry"].as_array().unwrap().len(),
        1
    );

    for (resource_type, key, other, expected) in [
        ("Observation", "subject.gender:in", None, 2),
        (
            "Observation",
            "subject:Patient.gender:in",
            Some(("code", "9999-9")),
            1,
        ),
        ("Patient", "_has:Observation:subject:code:in", None, 1),
    ] {
        let mut request = server
            .get(&format!("/{resource_type}"))
            .add_query_param(key, "http://example.org/vs");
        if let Some((k, v)) = other {
            request = request.add_query_param(k, v);
        }
        let response = request.await;
        assert_eq!(response.status_code(), StatusCode::OK, "{key}");
        let body: Value = response.json();
        assert_eq!(
            body["entry"].as_array().map_or(0, Vec::len),
            expected,
            "{key}: {body}"
        );
    }
}
