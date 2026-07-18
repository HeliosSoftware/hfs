//! Integration test: seeded CompartmentDefinitions are discoverable through the
//! normal FHIR route `GET /CompartmentDefinition`, reading primary storage.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::search::seed_spec_compartment_definitions;
use helios_rest::ServerConfig;

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"))
}

#[tokio::test]
async fn get_compartment_definition_returns_seeded_resources() {
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir()),
            ..Default::default()
        },
    )
    .expect("create SQLite backend");
    backend.init_schema().expect("init schema");
    let backend = Arc::new(backend);

    // Seed the default tenant, then serve.
    seed_spec_compartment_definitions(&*backend, FhirVersion::R4, &data_dir(), "default")
        .await
        .expect("seed compartment definitions");

    let config = ServerConfig {
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "default".to_string(),
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(Arc::clone(&backend), config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    let server = TestServer::new(app).expect("create test server");

    let resp = server.get("/CompartmentDefinition").await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: serde_json::Value = resp.json();
    assert_eq!(body["resourceType"], "Bundle");
    assert_eq!(body["type"], "searchset");
    let entries = body["entry"].as_array().expect("entries");
    assert_eq!(entries.len(), 5, "R4 ships 5 compartment definitions");

    let codes: Vec<&str> = entries
        .iter()
        .filter_map(|e| e["resource"]["code"].as_str())
        .collect();
    assert!(codes.contains(&"Patient"));
    assert!(codes.contains(&"Encounter"));

    // Read one by id.
    let patient = server.get("/CompartmentDefinition/patient").await;
    assert_eq!(patient.status_code(), StatusCode::OK);
    let pbody: serde_json::Value = patient.json();
    assert_eq!(pbody["resourceType"], "CompartmentDefinition");
    assert_eq!(pbody["code"], "Patient");
}
