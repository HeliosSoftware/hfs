//! `system|code` token searches on elements whose datatype is `code` (#1379),
//! end to end through the REST layer.
//!
//! A `code` primitive (`Patient.gender`, `Observation.status`) has no system
//! in the resource: FHIR makes it implicit, defined by the element's binding.
//! Its index row used to carry no system at all, so `gender=<system>|female`
//! never matched — and because terminology expansion always produces
//! `system|code` tokens, neither did `gender:in=<valueset>`, direct or chained.
//!
//! The persistence-level table lives in
//! `crates/persistence/tests/search/token_code_system_suite.rs` and runs on all
//! four backends; this file pins the same behaviour over HTTP on SQLite, and
//! the terminology path with a mock terminology server (the harness of
//! `terminology_integration.rs`).

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use helios_rest::{ServerConfig, create_app_with_config};

const GENDER: &str = "http://hl7.org/fhir/administrative-gender";
const OBS_STATUS: &str = "http://hl7.org/fhir/observation-status";
const LOINC: &str = "http://loinc.org";

/// The `$expand` requests a mock terminology server received.
type Requests = Arc<Mutex<Vec<Value>>>;

/// Starts a mock terminology server whose `POST /ValueSet/$expand` answers
/// every request with `system|code` for each of `codes`, and records the
/// requests it received.
async fn start_mock_hts(system: &str, codes: &[&str]) -> (String, Requests) {
    let contains: Vec<Value> = codes
        .iter()
        .map(|code| json!({"system": system, "code": code, "display": code}))
        .collect();
    let expansion = Arc::new(json!({
        "resourceType": "ValueSet",
        "expansion": {"total": contains.len(), "contains": contains}
    }));
    let requests: Requests = Arc::new(Mutex::new(vec![]));

    async fn expand(
        State((requests, expansion)): State<(Requests, Arc<Value>)>,
        Json(body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        requests.lock().unwrap().push(body);
        (StatusCode::OK, Json((*expansion).clone()))
    }

    let router = Router::new()
        .route("/ValueSet/$expand", axum::routing::post(expand))
        .with_state((Arc::clone(&requests), expansion));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (base_url, requests)
}

/// An in-memory HFS with the spec search parameters loaded — the embedded
/// fallback set knows neither `gender` nor `status`, so nothing would index —
/// seeded with:
///
/// - `p-f` / `p-m`: gender female / male; `p-none`: no gender.
/// - `o-f` (final, subject `p-f`, `http://loinc.org|1234-5`), `o-m`
///   (preliminary, subject `p-m`, same code), and `o-nosys` (final, no
///   subject), whose Coding has ONLY a code, `1234-5`.
async fn seeded_server(terminology_server: Option<String>) -> TestServer {
    let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("SQLite in-memory failed");
    backend.init_schema().expect("Schema init failed");
    let config = ServerConfig {
        terminology_server,
        ..ServerConfig::for_testing()
    };
    let server = TestServer::new(create_app_with_config(backend, config)).unwrap();

    for resource in [
        json!({"resourceType": "Patient", "id": "p-f", "gender": "female"}),
        json!({"resourceType": "Patient", "id": "p-m", "gender": "male"}),
        json!({"resourceType": "Patient", "id": "p-none", "active": true}),
        json!({"resourceType": "Observation", "id": "o-f", "status": "final",
               "code": {"coding": [{"system": LOINC, "code": "1234-5"}]},
               "subject": {"reference": "Patient/p-f"}}),
        json!({"resourceType": "Observation", "id": "o-m", "status": "preliminary",
               "code": {"coding": [{"system": LOINC, "code": "1234-5"}]},
               "subject": {"reference": "Patient/p-m"}}),
        json!({"resourceType": "Observation", "id": "o-nosys", "status": "final",
               "code": {"coding": [{"code": "1234-5"}]}}),
    ] {
        let path = format!(
            "/{}/{}",
            resource["resourceType"].as_str().unwrap(),
            resource["id"].as_str().unwrap()
        );
        let response = server.put(&path).json(&resource).await;
        assert!(response.status_code().is_success(), "seeding {path}");
    }
    server
}

/// The sorted ids of a searchset.
async fn search(server: &TestServer, resource_type: &str, key: &str, value: &str) -> Vec<String> {
    let response = server
        .get(&format!("/{resource_type}"))
        .add_query_param(key, value)
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{resource_type}?{key}={value}"
    );
    let body: Value = response.json();
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
}

#[tokio::test]
async fn system_qualified_token_matches_a_code_element() {
    let server = seeded_server(None).await;
    let gender_female = format!("{GENDER}|female");
    let status_final = format!("{OBS_STATUS}|final");
    let loinc_code = format!("{LOINC}|1234-5");

    for (resource_type, key, value, expected) in [
        // Positive controls: the parameters index.
        ("Patient", "gender", "female", &["p-f"][..]),
        ("Observation", "code", "1234-5", &["o-f", "o-m", "o-nosys"]),
        // The bug: these returned nothing.
        ("Patient", "gender", gender_female.as_str(), &["p-f"]),
        (
            "Observation",
            "status",
            status_final.as_str(),
            &["o-f", "o-nosys"],
        ),
        // `:not` is the exact negation, and keeps the patient with no gender.
        (
            "Patient",
            "gender:not",
            gender_female.as_str(),
            &["p-m", "p-none"],
        ),
        // A `code` element has no system property: `|code` still matches it.
        ("Patient", "gender", "|female", &["p-f"]),
        // The named system is not verified for a `code` element.
        ("Patient", "gender", "http://wrong.example|female", &["p-f"]),
        // `system|` stays a non-match for a `code` element.
        (
            "Patient",
            "gender",
            "http://hl7.org/fhir/administrative-gender|",
            &[],
        ),
        // NEGATIVE CONTROL: a Coding with no system is still not
        // `http://loinc.org|1234-5`, and still is `|1234-5`.
        ("Observation", "code", loinc_code.as_str(), &["o-f", "o-m"]),
        ("Observation", "code", "|1234-5", &["o-nosys"]),
        ("Observation", "code", "http://wrong.example|1234-5", &[]),
        // Chained and reverse-chained terminals.
        (
            "Observation",
            "subject:Patient.gender",
            gender_female.as_str(),
            &["o-f"],
        ),
        (
            "Patient",
            "_has:Observation:subject:status",
            status_final.as_str(),
            &["p-f"],
        ),
    ] {
        assert_eq!(
            search(&server, resource_type, key, value).await,
            expected,
            "{resource_type}?{key}={value}"
        );
    }
}

/// Terminology expansion always yields `system|code`, so `:in` on a `code`
/// element matched nothing — direct, chained or through `_has`.
#[tokio::test]
async fn in_modifier_matches_a_code_element() {
    let (ts_url, requests) = start_mock_hts(GENDER, &["female", "other"]).await;
    let server = seeded_server(Some(ts_url)).await;
    let value_set = "http://hl7.org/fhir/ValueSet/administrative-gender";

    // Positive control.
    assert_eq!(
        search(&server, "Patient", "gender", "female").await,
        ["p-f"]
    );

    for (resource_type, key, expected) in [
        ("Patient", "gender:in", "p-f"),
        ("Observation", "subject:Patient.gender:in", "o-f"),
    ] {
        let before = requests.lock().unwrap().len();
        assert_eq!(
            search(&server, resource_type, key, value_set).await,
            [expected],
            "{resource_type}?{key}"
        );
        assert_eq!(
            requests.lock().unwrap().len(),
            before + 1,
            "{key}: expected one $expand call"
        );
    }
}

/// The same through `_has`, on `Observation.status`.
#[tokio::test]
async fn in_modifier_matches_a_code_element_through_has() {
    let (ts_url, requests) = start_mock_hts(OBS_STATUS, &["final", "amended"]).await;
    let server = seeded_server(Some(ts_url)).await;

    assert_eq!(
        search(
            &server,
            "Patient",
            "_has:Observation:subject:status:in",
            "http://hl7.org/fhir/ValueSet/observation-status",
        )
        .await,
        ["p-f"]
    );
    assert_eq!(requests.lock().unwrap().len(), 1);
}
