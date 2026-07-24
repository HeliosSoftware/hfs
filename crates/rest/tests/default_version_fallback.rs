//! The configured default FHIR version is the effective fallback (#355).
//!
//! Runs the REST stack against in-memory SQLite with
//! `default_fhir_version: R4B` — a version that differs from the compile-time
//! default (R4) — and asserts that requests which negotiate nothing follow the
//! configuration: `$versions` reports it, and writes without a `fhirVersion`
//! MIME parameter stamp it (plain update and conditional update).

#![cfg(feature = "R4B")]

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
const ACCEPT: HeaderName = HeaderName::from_static("accept");

async fn create_test_server() -> TestServer {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));

    let backend_config = SqliteBackendConfig {
        data_dir: Some(data_dir),
        fhir_version: FhirVersion::R4B,
        ..Default::default()
    };
    let backend = SqliteBackend::with_config(":memory:", backend_config)
        .expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to init schema");
    let backend = Arc::new(backend);

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_fhir_version: FhirVersion::R4B,
        seed_conformance: false,
        ..ServerConfig::for_testing()
    };

    let state = helios_rest::AppState::new(Arc::clone(&backend), config);
    let router = helios_rest::routing::fhir_routes::create_routes(state);
    TestServer::new(router).expect("Failed to create test server")
}

#[tokio::test]
async fn versions_operation_reports_the_configured_default() {
    let server = create_test_server().await;

    let response = server.get("/$versions").await;
    response.assert_status(StatusCode::OK);
    let body: Value = response.json();

    let default = body["parameter"]
        .as_array()
        .expect("parameter array")
        .iter()
        .find(|p| p["name"] == "default")
        .expect("default parameter");
    assert_eq!(
        default["valueCode"], "4.3",
        "$versions must report the configured default, not the compile-time one"
    );
}

/// A PUT with no `fhirVersion` MIME parameter stamps the configured default:
/// the resource reads back under `fhirVersion=4.3` and 406s under `4.0`.
#[tokio::test]
async fn update_without_negotiation_stamps_the_configured_default() {
    let server = create_test_server().await;

    let response = server
        .put("/Patient/fallback-1")
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&json!({"resourceType": "Patient", "id": "fallback-1"}))
        .await;
    response.assert_status(StatusCode::CREATED);

    let as_r4b = server
        .get("/Patient/fallback-1")
        .add_header(
            ACCEPT,
            HeaderValue::from_static("application/fhir+json; fhirVersion=4.3"),
        )
        .await;
    as_r4b.assert_status(StatusCode::OK);

    let as_r4 = server
        .get("/Patient/fallback-1")
        .add_header(
            ACCEPT,
            HeaderValue::from_static("application/fhir+json; fhirVersion=4.0"),
        )
        .await;
    as_r4.assert_status(StatusCode::NOT_ACCEPTABLE);
}

/// The conditional-update path resolves its version through the same fallback.
#[tokio::test]
async fn conditional_update_without_negotiation_stamps_the_configured_default() {
    let server = create_test_server().await;

    let response = server
        .put("/Patient?identifier=cond-fallback")
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&json!({
            "resourceType": "Patient",
            "identifier": [{"value": "cond-fallback"}]
        }))
        .await;
    response.assert_status(StatusCode::CREATED);

    let location = response
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .expect("Location header on conditional create")
        .to_string();
    let path = location
        .strip_prefix("http://localhost:8080")
        .unwrap_or(&location)
        .split("/_history")
        .next()
        .unwrap()
        .to_string();

    let as_r4b = server
        .get(&path)
        .add_header(
            ACCEPT,
            HeaderValue::from_static("application/fhir+json; fhirVersion=4.3"),
        )
        .await;
    as_r4b.assert_status(StatusCode::OK);
}
