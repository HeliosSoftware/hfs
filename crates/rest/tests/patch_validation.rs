//! PATCH validates the resulting resource through the same write gate as PUT.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderValue, StatusCode, header};
use axum_test::{TestResponse, TestServer};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::config::ValidationConfig;
use helios_rest::{AppState, ServerConfig};
use serde_json::{Value, json};

const JSON_PATCH: &str = "application/json-patch+json";
const MERGE_PATCH: &str = "application/merge-patch+json";
const FHIRPATH_PATCH: &str = "application/fhir+json";

fn server(mode: &str) -> TestServer {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("SQLite backend");
    backend.init_schema().expect("SQLite schema");
    let config = ServerConfig {
        validation: ValidationConfig {
            mode: mode.to_string(),
            ..Default::default()
        },
        ..ServerConfig::for_testing()
    };
    let state = AppState::new(Arc::new(backend), config);
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(state)).expect("test server")
}

async fn seed(server: &TestServer) {
    let response = server
        .put("/Patient/p1")
        .json(&json!({"resourceType":"Patient","id":"p1","active":false}))
        .await;
    response.assert_status(StatusCode::CREATED);
}

async fn stored(server: &TestServer) -> Value {
    let response = server.get("/Patient/p1").await;
    response.assert_status(StatusCode::OK);
    response.json()
}

async fn etag(server: &TestServer) -> String {
    server
        .get("/Patient/p1")
        .await
        .headers()
        .get(header::ETAG)
        .expect("ETag")
        .to_str()
        .expect("ETag text")
        .to_string()
}

async fn patch(
    server: &TestServer,
    url: &str,
    content_type: &'static str,
    body: &Value,
) -> TestResponse {
    server
        .patch(url)
        .add_header(header::CONTENT_TYPE, HeaderValue::from_static(content_type))
        .bytes(serde_json::to_vec(body).expect("patch JSON").into())
        .await
}

fn fhirpath_operation(kind: &str, path: &str, value: Option<Value>) -> Value {
    let mut parts = vec![
        json!({"name":"type","valueCode":kind}),
        json!({"name":"path","valueString":path}),
    ];
    if let Some(value) = value {
        parts.push(value);
    }
    json!({"resourceType":"Parameters","parameter":[{"name":"operation","part":parts}]})
}

fn invalid_patch(format: &str) -> Value {
    match format {
        JSON_PATCH => json!([{"op":"add","path":"/bogusElement","value":true}]),
        MERGE_PATCH => json!({"bogusElement":true}),
        FHIRPATH_PATCH => json!({
            "resourceType":"Parameters",
            "parameter":[{"name":"operation","part":[
                {"name":"type","valueCode":"add"},
                {"name":"path","valueString":"Patient"},
                {"name":"name","valueString":"bogusElement"},
                {"name":"value","valueBoolean":true}
            ]}]
        }),
        _ => unreachable!(),
    }
}

fn valid_patch(format: &str) -> Value {
    match format {
        JSON_PATCH => json!([{"op":"replace","path":"/active","value":true}]),
        MERGE_PATCH => json!({"active":true}),
        FHIRPATH_PATCH => fhirpath_operation(
            "replace",
            "Patient.active",
            Some(json!({"name":"value","valueBoolean":true})),
        ),
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn enforce_rejects_invalid_patch_without_writing_on_both_routes() {
    for format in [JSON_PATCH, MERGE_PATCH, FHIRPATH_PATCH] {
        for url in ["/Patient/p1", "/Patient?_id=p1"] {
            let server = server("enforce");
            seed(&server).await;
            let before = stored(&server).await;
            let before_etag = etag(&server).await;

            let response = patch(&server, url, format, &invalid_patch(format)).await;
            response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
            let outcome: Value = response.json();
            assert_eq!(
                outcome["resourceType"], "OperationOutcome",
                "{format} {url}"
            );
            assert!(
                outcome["issue"].as_array().is_some_and(|issues| issues
                    .iter()
                    .any(|issue| issue["code"] == "structure"
                        && issue["expression"][0] == "Patient.bogusElement")),
                "{format} {url}: {outcome:#}"
            );
            assert_eq!(
                stored(&server).await,
                before,
                "{format} {url} wrote a version"
            );
            assert_eq!(
                etag(&server).await,
                before_etag,
                "{format} {url} changed ETag"
            );
        }
    }
}

#[tokio::test]
async fn valid_patch_passes_enforce_and_invalid_patch_passes_log_and_off() {
    for format in [JSON_PATCH, MERGE_PATCH, FHIRPATH_PATCH] {
        for url in ["/Patient/p1", "/Patient?_id=p1"] {
            let enforced = server("enforce");
            seed(&enforced).await;
            let before_etag = etag(&enforced).await;
            let response = patch(&enforced, url, format, &valid_patch(format)).await;
            response.assert_status(StatusCode::OK);
            let written = stored(&enforced).await;
            assert_eq!(written["active"], true, "{format} {url}");
            assert_eq!(written["meta"]["versionId"], "2", "{format} {url}");
            assert_ne!(etag(&enforced).await, before_etag, "{format} {url}");

            for mode in ["log", "off"] {
                let server = server(mode);
                seed(&server).await;
                let response = patch(&server, url, format, &invalid_patch(format)).await;
                response.assert_status(StatusCode::OK);
                let written = stored(&server).await;
                assert_eq!(written["bogusElement"], true, "{mode} {format} {url}");
                assert_eq!(written["meta"]["versionId"], "2", "{mode} {format} {url}");
            }
        }
    }
}

#[tokio::test]
async fn failed_test_is_422_but_malformed_json_patch_is_400() {
    for url in ["/Patient/p1", "/Patient?_id=p1"] {
        let server = server("enforce");
        seed(&server).await;
        let before = stored(&server).await;

        let response = patch(
            &server,
            url,
            JSON_PATCH,
            &json!([{"op":"test","path":"/active","value":true}]),
        )
        .await;
        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        let outcome: Value = response.json();
        assert_eq!(outcome["resourceType"], "OperationOutcome");

        let response = patch(&server, url, JSON_PATCH, &json!({"not":"a patch"})).await;
        response.assert_status(StatusCode::BAD_REQUEST);
        assert_eq!(stored(&server).await, before, "{url} wrote a version");
    }
}

#[tokio::test]
async fn patched_structure_definition_refreshes_the_tenant_profile() {
    const PROFILE: &str = "http://example.org/StructureDefinition/patch-profile";
    for url in [
        "/StructureDefinition/patch-profile",
        "/StructureDefinition?_id=patch-profile",
    ] {
        let server = server("enforce");
        let profile = json!({
            "resourceType":"StructureDefinition",
            "id":"patch-profile",
            "url":PROFILE,
            "name":"PatchProfile",
            "status":"active",
            "kind":"resource",
            "abstract":false,
            "type":"Patient",
            "baseDefinition":"http://hl7.org/fhir/StructureDefinition/Patient",
            "derivation":"constraint",
            "differential":{"element":[
                {"id":"Patient","path":"Patient"},
                {"id":"Patient.birthDate","path":"Patient.birthDate","min":1}
            ]}
        });
        server
            .put("/StructureDefinition/patch-profile")
            .json(&profile)
            .await
            .assert_status(StatusCode::CREATED);

        let patient = json!({"resourceType":"Patient","meta":{"profile":[PROFILE]}});
        server
            .post("/Patient")
            .json(&patient)
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

        let response = patch(
            &server,
            url,
            JSON_PATCH,
            &json!([{"op":"replace","path":"/differential/element/1/min","value":0}]),
        )
        .await;
        response.assert_status(StatusCode::OK);
        server
            .post("/Patient")
            .json(&patient)
            .await
            .assert_status(StatusCode::CREATED);
    }
}

#[tokio::test]
async fn patch_keeps_the_view_definition_resource_guard() {
    for url in ["/ViewDefinition/guard", "/ViewDefinition?_id=guard"] {
        let server = server("off");
        let definition = json!({
            "resourceType":"ViewDefinition",
            "id":"guard",
            "status":"active",
            "name":"guard",
            "resource":"Patient",
            "select":[{"column":[{"name":"id","path":"id"}]}]
        });
        server
            .put("/ViewDefinition/guard")
            .json(&definition)
            .await
            .assert_status(StatusCode::CREATED);
        let before: Value = server.get("/ViewDefinition/guard").await.json();

        let response = patch(&server, url, MERGE_PATCH, &json!({"resource":"Nope"})).await;
        response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        let outcome: Value = response.json();
        assert_eq!(
            outcome["issue"][0]["details"]["coding"][0]["code"],
            "unknown-resource-type"
        );
        let after: Value = server.get("/ViewDefinition/guard").await.json();
        assert_eq!(after, before, "{url} wrote a version");
    }
}
