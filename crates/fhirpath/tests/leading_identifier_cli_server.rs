//! `fhirpath-cli` and `fhirpath-server` opt into the `Error` policy for an
//! unresolvable leading identifier (#640).

use axum::http::StatusCode;
use axum_test::TestServer;
use helios_fhirpath::server::{ServerConfig, create_app};
use serde_json::{Value, json};
use std::process::Command;

fn patient() -> Value {
    json!({"resourceType": "Patient", "id": "p1",
           "meta": {"source": "http://example.org/src"}})
}

fn run_cli(expression: &str) -> std::process::Output {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("patient.json");
    std::fs::write(&path, patient().to_string()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_fhirpath-cli"))
        .args(["--expression", expression, "--resource"])
        .arg(&path)
        .output()
        .expect("run fhirpath-cli")
}

#[test]
fn cli_resolves_abstract_prefix() {
    let output = run_cli("Resource.meta.source");
    assert!(output.status.success(), "{:?}", output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("http://example.org/src"));
}

#[test]
fn cli_exits_1_on_unresolvable_leading_identifier() {
    let output = run_cli("Bogus.meta.source");
    assert_eq!(output.status.code(), Some(1), "{:?}", output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Bogus"), "{stderr}");
}

#[test]
fn cli_stays_silent_for_known_type_that_is_not_the_context() {
    let output = run_cli("Observation.code.text");
    assert!(output.status.success(), "{:?}", output);
}

fn request(expression: &str) -> Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "expression", "valueString": expression},
            {"name": "resource", "resource": patient()}
        ]
    })
}

fn server() -> TestServer {
    TestServer::new(create_app(&ServerConfig::default())).expect("test server")
}

#[tokio::test]
async fn server_returns_422_for_unresolvable_leading_identifier() {
    let response = server()
        .post("/")
        .content_type("application/json")
        .json(&request("Bogus.meta.source"))
        .await;
    assert_eq!(response.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
    let body: Value = response.json();
    assert_eq!(body["resourceType"], "OperationOutcome");
    let diagnostics = body["issue"][0]["diagnostics"].as_str().unwrap_or_default();
    assert!(diagnostics.contains("Bogus"), "{body}");
}

#[tokio::test]
async fn server_resolves_abstract_prefix_and_ignores_known_foreign_type() {
    let ok = server()
        .post("/")
        .content_type("application/json")
        .json(&request("Resource.meta.source"))
        .await;
    assert_eq!(ok.status_code(), StatusCode::OK);

    let foreign = server()
        .post("/")
        .content_type("application/json")
        .json(&request("Observation.code.text"))
        .await;
    assert_eq!(foreign.status_code(), StatusCode::OK);
}
