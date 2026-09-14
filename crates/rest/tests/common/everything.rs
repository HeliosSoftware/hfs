//! Shared test helpers for `Patient/$everything` router tests, over SQLite
//! in-memory. Reused by Task 9's Postgres/Mongo tests.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
use axum_test::TestServer;
#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use serde_json::{Value, json};

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

#[cfg(feature = "sqlite")]
pub async fn server_with(max_unpaged: usize) -> TestServer {
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir()),
            ..Default::default()
        },
    )
    .expect("create SQLite backend");
    backend.init_schema().expect("init schema");
    let config = ServerConfig {
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "default".to_string(),
        everything_max_unpaged: max_unpaged,
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(Arc::new(backend), config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    TestServer::new(app).expect("create test server")
}

pub async fn put(server: &TestServer, resource: Value) {
    let rt = resource["resourceType"].as_str().unwrap();
    let id = resource["id"].as_str().unwrap();
    let resp = server.put(&format!("/{rt}/{id}")).json(&resource).await;
    assert!(
        resp.status_code().is_success(),
        "PUT {rt}/{id}: {}",
        resp.text()
    );
}

/// Seeds patient `p1` with 3 Observations, 2 Encounters, 1 Condition, a
/// Practitioner and an Organization they reference; and a control patient
/// `p2` with one Observation. Returns nothing; ids are fixed.
pub async fn seed(server: &TestServer) {
    put(
        server,
        json!({"resourceType": "Organization", "id": "org1", "name": "Org"}),
    )
    .await;
    put(
        server,
        json!({"resourceType": "Practitioner", "id": "dr1", "name": [{"family": "Who"}]}),
    )
    .await;
    put(
        server,
        json!({"resourceType": "Patient", "id": "p1", "managingOrganization": {"reference": "Organization/org1"}}),
    )
    .await;
    put(server, json!({"resourceType": "Patient", "id": "p2"})).await;
    for (i, date) in [(1, "2019-05-01"), (2, "2020-05-01"), (3, "2021-05-01")] {
        put(
            server,
            json!({"resourceType": "Observation", "id": format!("o{i}"), "status": "final",
                "code": {"text": "x"}, "subject": {"reference": "Patient/p1"}, "effectiveDateTime": date,
                "performer": [{"reference": "Practitioner/dr1"}]}),
        )
        .await;
    }
    for (i, date) in [(1, "2019-06-01"), (2, "2021-06-01")] {
        put(
            server,
            json!({"resourceType": "Encounter", "id": format!("e{i}"), "status": "finished",
                "class": {"code": "AMB"}, "subject": {"reference": "Patient/p1"},
                "period": {"start": date}, "serviceProvider": {"reference": "Organization/org1"}}),
        )
        .await;
    }
    put(
        server,
        json!({"resourceType": "Condition", "id": "c1", "subject": {"reference": "Patient/p1"},
            "onsetDateTime": "2020-01-15"}),
    )
    .await;
    put(
        server,
        json!({"resourceType": "Observation", "id": "other", "status": "final",
            "code": {"text": "x"}, "subject": {"reference": "Patient/p2"}}),
    )
    .await;
}

pub fn entries(bundle: &Value, mode: &str) -> Vec<String> {
    bundle["entry"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["search"]["mode"] == mode)
        .map(|e| {
            format!(
                "{}/{}",
                e["resource"]["resourceType"].as_str().unwrap(),
                e["resource"]["id"].as_str().unwrap()
            )
        })
        .collect()
}

pub fn next_link(bundle: &Value) -> Option<String> {
    bundle["link"]
        .as_array()?
        .iter()
        .find(|l| l["relation"] == "next")?["url"]
        .as_str()
        .map(str::to_string)
}

pub fn path_of(url: &str) -> String {
    url.strip_prefix("http://localhost:8080")
        .unwrap()
        .to_string()
}

pub async fn walk(server: &TestServer, first: &str) -> (Vec<String>, Vec<Value>) {
    let mut path = first.to_string();
    let mut matches = Vec::new();
    let mut pages = Vec::new();
    loop {
        let resp = server.get(&path).await;
        assert_eq!(
            resp.status_code(),
            StatusCode::OK,
            "{path}: {}",
            resp.text()
        );
        let b: Value = resp.json();
        matches.extend(entries(&b, "match"));
        let next = next_link(&b);
        pages.push(b);
        match next {
            Some(n) => path = path_of(&n),
            None => break,
        }
        assert!(pages.len() < 50, "runaway paging");
    }
    (matches, pages)
}
