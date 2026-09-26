//! PATCH entries use the Bundle's FHIR payload formats and the ordinary write gate.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
#[cfg(any(feature = "R4B", feature = "R5", feature = "R6"))]
use axum::http::{HeaderValue, header};
use axum_test::{TestResponse, TestServer};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use helios_rest::config::ValidationConfig;
use serde_json::{Value, json};

async fn server() -> TestServer {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .unwrap();
    backend.init_schema().unwrap();
    let config = ServerConfig {
        validation: ValidationConfig {
            mode: "enforce".to_string(),
            ..Default::default()
        },
        ..ServerConfig::for_testing()
    };
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(
        helios_rest::AppState::new(Arc::new(backend), config),
    ))
    .unwrap()
}

async fn seed(server: &TestServer, id: &str) {
    server
        .put(&format!("/Patient/{id}"))
        .json(&json!({"resourceType":"Patient","id":id,"active":false}))
        .await
        .assert_status(StatusCode::CREATED);
}

fn fhirpath_replace_active(value: bool) -> Value {
    json!({"resourceType":"Parameters","parameter":[{"name":"operation","part":[
        {"name":"type","valueCode":"replace"},
        {"name":"path","valueString":"Patient.active"},
        {"name":"value","valueBoolean":value}
    ]}]})
}

fn fhirpath_add_invalid() -> Value {
    json!({"resourceType":"Parameters","parameter":[{"name":"operation","part":[
        {"name":"type","valueCode":"add"},
        {"name":"path","valueString":"Patient"},
        {"name":"name","valueString":"bogusElement"},
        {"name":"value","valueBoolean":true}
    ]}]})
}

fn patch_entry(id: &str, resource: Value) -> Value {
    json!({"request":{"method":"PATCH","url":format!("Patient/{id}")},"resource":resource})
}

fn patch_entry_for(resource_type: &str, id: &str, resource: Value) -> Value {
    json!({"request":{"method":"PATCH","url":format!("{resource_type}/{id}")},"resource":resource})
}

fn bundle(kind: &str, entries: Vec<Value>) -> Value {
    json!({"resourceType":"Bundle","type":kind,"entry":entries})
}

async fn patient(server: &TestServer, id: &str) -> Value {
    server.get(&format!("/Patient/{id}")).await.json()
}

#[cfg(any(feature = "R4B", feature = "R5", feature = "R6"))]
async fn put_versioned_patient(server: &TestServer, mime: &'static str) {
    let patient = json!({"resourceType":"Patient","id":"versioned","active":false});
    server
        .put("/Patient/versioned")
        .add_header(header::CONTENT_TYPE, HeaderValue::from_static(mime))
        .bytes(patient.to_string().into())
        .await
        .assert_status(StatusCode::CREATED);
}

#[cfg(any(feature = "R4B", feature = "R5", feature = "R6"))]
async fn post_versioned_bundle(
    server: &TestServer,
    mime: &'static str,
    body: Value,
) -> TestResponse {
    server
        .post("/")
        .add_header(header::CONTENT_TYPE, HeaderValue::from_static(mime))
        .bytes(body.to_string().into())
        .await
}

#[cfg(any(feature = "R4B", feature = "R5", feature = "R6"))]
fn binary_json_patch(document: Value) -> Value {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    json!({
        "resourceType":"Binary",
        "contentType":"application/json-patch+json",
        "data":STANDARD.encode(document.to_string())
    })
}

#[cfg(any(feature = "R4B", feature = "R6"))]
async fn fhirpath_bundle_roundtrip(mime: &'static str) {
    let server = server().await;
    put_versioned_patient(&server, mime).await;

    let batch = post_versioned_bundle(
        &server,
        mime,
        bundle(
            "batch",
            vec![patch_entry("versioned", fhirpath_replace_active(true))],
        ),
    )
    .await;
    batch.assert_status(StatusCode::OK);
    let batch_body: Value = batch.json();
    assert!(
        batch_body["entry"][0]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("200"),
        "{batch_body}"
    );

    let transaction = post_versioned_bundle(
        &server,
        mime,
        bundle(
            "transaction",
            vec![patch_entry("versioned", fhirpath_replace_active(false))],
        ),
    )
    .await;
    transaction.assert_status(StatusCode::OK);
    let stored = server
        .get("/Patient/versioned")
        .add_header(header::ACCEPT, HeaderValue::from_static(mime))
        .await;
    stored.assert_status(StatusCode::OK);
    let stored: Value = stored.json();
    assert_eq!(stored["active"], false);
    assert_eq!(stored["meta"]["versionId"], "3");
    server
        .get("/Patient/versioned")
        .add_header(
            header::ACCEPT,
            HeaderValue::from_static("application/fhir+json; fhirVersion=4.0"),
        )
        .await
        .assert_status(StatusCode::NOT_ACCEPTABLE);
}

#[cfg(feature = "R4B")]
#[tokio::test]
async fn r4b_fhirpath_parameters_patch_works_in_batch_and_transaction() {
    fhirpath_bundle_roundtrip("application/fhir+json; fhirVersion=4.3").await;
}

#[cfg(feature = "R6")]
#[tokio::test]
async fn r6_fhirpath_parameters_patch_works_in_batch_and_transaction() {
    fhirpath_bundle_roundtrip("application/fhir+json; fhirVersion=6.0").await;
}

#[cfg(feature = "R4B")]
#[tokio::test]
async fn r4b_bundle_does_not_accept_binary_json_patch() {
    let server = server().await;
    let mime = "application/fhir+json; fhirVersion=4.3";
    put_versioned_patient(&server, mime).await;
    let response = post_versioned_bundle(
        &server,
        mime,
        bundle(
            "batch",
            vec![patch_entry(
                "versioned",
                binary_json_patch(json!([
                    {"op":"replace","path":"/active","value":true}
                ])),
            )],
        ),
    )
    .await;
    response.assert_status(StatusCode::OK);
    let body: Value = response.json();
    assert!(
        body["entry"][0]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("501"),
        "{body}"
    );
    let stored: Value = server.get("/Patient/versioned").await.json();
    assert_eq!(stored["active"], false);
    assert_eq!(stored["meta"]["versionId"], "1");
}

#[cfg(any(feature = "R5", feature = "R6"))]
async fn binary_bundle_roundtrip(mime: &'static str) {
    let server = server().await;
    put_versioned_patient(&server, mime).await;
    for (kind, value) in [("batch", true), ("transaction", false)] {
        let response = post_versioned_bundle(
            &server,
            mime,
            bundle(
                kind,
                vec![patch_entry(
                    "versioned",
                    binary_json_patch(json!([
                        {"op":"replace","path":"/active","value":value}
                    ])),
                )],
            ),
        )
        .await;
        response.assert_status(StatusCode::OK);
        let stored = server
            .get("/Patient/versioned")
            .add_header(header::ACCEPT, HeaderValue::from_static(mime))
            .await;
        stored.assert_status(StatusCode::OK);
        let stored: Value = stored.json();
        assert_eq!(stored["active"], value, "{kind}");
    }
    let stored: Value = server.get("/Patient/versioned").await.json();
    assert_eq!(stored["meta"]["versionId"], "3");
}

#[cfg(feature = "R5")]
#[tokio::test]
async fn r5_binary_json_patch_works_in_batch_and_transaction() {
    binary_bundle_roundtrip("application/fhir+json; fhirVersion=5.0").await;
}

#[cfg(feature = "R6")]
#[tokio::test]
async fn r6_binary_json_patch_works_in_batch_and_transaction() {
    binary_bundle_roundtrip("application/fhir+json; fhirVersion=6.0").await;
}

#[tokio::test]
async fn batch_patch_validates_each_candidate_and_isolates_failure() {
    let server = server().await;
    seed(&server, "bad").await;
    seed(&server, "good").await;
    let before_bad = patient(&server, "bad").await;

    let response = server
        .post("/")
        .json(&bundle(
            "batch",
            vec![
                patch_entry("bad", fhirpath_add_invalid()),
                patch_entry("good", fhirpath_replace_active(true)),
            ],
        ))
        .await;
    response.assert_status(StatusCode::OK);
    let body: Value = response.json();
    assert!(
        body["entry"][0]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("422"),
        "{body}"
    );
    assert_eq!(
        body["entry"][0]["response"]["outcome"]["issue"][0]["expression"][0],
        "Patient.bogusElement"
    );
    assert!(
        body["entry"][1]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("200"),
        "{body}"
    );
    assert_eq!(body["entry"][1]["response"]["etag"], "W/\"2\"");
    assert_eq!(patient(&server, "bad").await, before_bad);
    assert_eq!(patient(&server, "good").await["active"], true);
}

#[tokio::test]
async fn transaction_patch_sees_prior_put_and_rolls_back_on_validation_failure() {
    let server = server().await;
    seed(&server, "p1").await;
    let original = patient(&server, "p1").await;

    let valid = server
        .post("/")
        .json(&bundle(
            "transaction",
            vec![
                json!({"request":{"method":"PUT","url":"Patient/p1"},
               "resource":{"resourceType":"Patient","id":"p1","active":false,"gender":"female"}}),
                patch_entry("p1", fhirpath_replace_active(true)),
            ],
        ))
        .await;
    valid.assert_status(StatusCode::OK);
    let committed = patient(&server, "p1").await;
    assert_eq!(committed["active"], true);
    assert_eq!(committed["gender"], "female");

    let failed = server
        .post("/")
        .json(&bundle(
            "transaction",
            vec![
                json!({"request":{"method":"PUT","url":"Patient/p1"},
               "resource":{"resourceType":"Patient","id":"p1","active":false,"gender":"male"}}),
                patch_entry("p1", fhirpath_add_invalid()),
            ],
        ))
        .await;
    failed.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    let outcome: Value = failed.json();
    assert_eq!(
        outcome["issue"][0]["expression"][0], "Patient.bogusElement",
        "{outcome}"
    );
    assert_eq!(patient(&server, "p1").await, committed);
    assert_ne!(patient(&server, "p1").await, original);
}

#[tokio::test]
async fn batch_patch_honors_if_match_and_auditevent_immutability() {
    let server = server().await;
    seed(&server, "p1").await;
    let before = patient(&server, "p1").await;
    let response = server
        .post("/")
        .json(&bundle(
            "batch",
            vec![
                json!({"request":{"method":"PATCH","url":"Patient/p1","ifMatch":"W/\"0\""},
               "resource":fhirpath_replace_active(true)}),
                json!({"request":{"method":"PATCH","url":"AuditEvent/a1"},
               "resource":fhirpath_replace_active(true)}),
            ],
        ))
        .await;
    response.assert_status(StatusCode::OK);
    let body: Value = response.json();
    assert!(
        body["entry"][0]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("412"),
        "{body}"
    );
    assert!(
        body["entry"][1]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("405"),
        "{body}"
    );
    assert_eq!(patient(&server, "p1").await, before);
}

#[tokio::test]
async fn conditional_batch_patch_resolves_one_target_and_checks_if_match() {
    let server = server().await;
    seed(&server, "p1").await;
    seed(&server, "p2").await;
    let before_p2 = patient(&server, "p2").await;
    let response = server
        .post("/")
        .json(&bundle(
            "batch",
            vec![
                json!({"request":{"method":"PATCH","url":"Patient?_id=p1","ifMatch":"W/\"0\""},
               "resource":fhirpath_replace_active(true)}),
                json!({"request":{"method":"PATCH","url":"Patient?_id=p1","ifMatch":"W/\"1\""},
               "resource":fhirpath_replace_active(true)}),
            ],
        ))
        .await;
    response.assert_status(StatusCode::OK);
    let body: Value = response.json();
    assert!(
        body["entry"][0]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("412"),
        "{body}"
    );
    assert!(
        body["entry"][1]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("200"),
        "{body}"
    );
    let after = patient(&server, "p1").await;
    assert_eq!(after["active"], true);
    assert_eq!(patient(&server, "p2").await, before_p2);

    let invalid = server
        .post("/")
        .json(&bundle(
            "batch",
            vec![
                json!({"request":{"method":"PATCH","url":"Patient?_id=p1","ifMatch":"W/\"2\""},
               "resource":fhirpath_add_invalid()}),
            ],
        ))
        .await;
    invalid.assert_status(StatusCode::OK);
    let body: Value = invalid.json();
    assert!(
        body["entry"][0]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("422"),
        "{body}"
    );
    assert_eq!(patient(&server, "p1").await, after);
}

#[tokio::test]
async fn conditional_batch_patch_refuses_zero_or_multiple_matches_without_writing() {
    let server = server().await;
    seed(&server, "p1").await;
    seed(&server, "p2").await;

    let response = server
        .post("/")
        .json(&bundle(
            "batch",
            vec![
                json!({"request":{"method":"PATCH","url":"Patient?_id=missing"},
                    "resource":fhirpath_replace_active(true)}),
                json!({"request":{"method":"PATCH","url":"Patient?_id=p1,p2"},
                    "resource":fhirpath_replace_active(true)}),
                patch_entry("missing", fhirpath_replace_active(true)),
                json!({"request":{"method":"PATCH","url":"NotAResource/p1"},
                    "resource":fhirpath_replace_active(true)}),
                json!({"request":{"method":"PATCH","url":"Patient/p1"}}),
            ],
        ))
        .await;
    response.assert_status(StatusCode::OK);
    let body: Value = response.json();
    assert!(
        body["entry"][0]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("404"),
        "{body}"
    );
    assert!(
        body["entry"][1]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("412"),
        "{body}"
    );
    for index in [2, 3] {
        assert!(
            body["entry"][index]["response"]["status"]
                .as_str()
                .unwrap()
                .starts_with("404"),
            "{body}"
        );
    }
    assert!(
        body["entry"][4]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("400"),
        "{body}"
    );
    assert_eq!(patient(&server, "p1").await["active"], false);
    assert_eq!(patient(&server, "p2").await["active"], false);
}

#[tokio::test]
async fn transaction_patch_if_match_failure_rolls_back_prior_write() {
    let server = server().await;
    seed(&server, "p1").await;
    let before = patient(&server, "p1").await;
    let response = server
        .post("/")
        .json(&bundle(
            "transaction",
            vec![
                json!({"request":{"method":"POST","url":"Patient"},
               "resource":{"resourceType":"Patient","active":true}}),
                json!({"request":{"method":"PATCH","url":"Patient/p1","ifMatch":"W/\"0\""},
               "resource":fhirpath_replace_active(true)}),
            ],
        ))
        .await;
    response.assert_status(StatusCode::PRECONDITION_FAILED);
    assert_eq!(patient(&server, "p1").await, before);
    let search: Value = server.get("/Patient?_count=100").await.json();
    assert_eq!(
        search["entry"].as_array().map(Vec::len),
        Some(1),
        "{search}"
    );
}

#[tokio::test]
async fn batch_patch_keeps_tenants_isolated() {
    let server = server().await;
    for tenant in ["clinic-a", "clinic-b"] {
        server
            .put("/Patient/p1")
            .add_header("x-tenant-id", tenant)
            .json(&json!({"resourceType":"Patient","id":"p1","active":false}))
            .await
            .assert_status(StatusCode::CREATED);
    }
    let response = server
        .post("/")
        .add_header("x-tenant-id", "clinic-a")
        .json(&bundle(
            "batch",
            vec![patch_entry("p1", fhirpath_replace_active(true))],
        ))
        .await;
    response.assert_status(StatusCode::OK);
    let body: Value = response.json();
    assert!(
        body["entry"][0]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("200"),
        "{body}"
    );
    let a: Value = server
        .get("/Patient/p1")
        .add_header("x-tenant-id", "clinic-a")
        .await
        .json();
    let b: Value = server
        .get("/Patient/p1")
        .add_header("x-tenant-id", "clinic-b")
        .await
        .json();
    assert_eq!(a["active"], true);
    assert_eq!(b["active"], false);
}

#[cfg(feature = "R5")]
#[tokio::test]
async fn r5_binary_json_patch_test_failure_is_422_and_malformed_is_400() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let server = server().await;
    seed(&server, "p1").await;
    let before = patient(&server, "p1").await;
    let binary = |patch: Value| {
        json!({
            "resourceType":"Binary", "contentType":"application/json-patch+json",
            "data":STANDARD.encode(patch.to_string())
        })
    };
    let batch = bundle(
        "batch",
        vec![
            patch_entry(
                "p1",
                binary(json!([{"op":"test","path":"/active","value":true}])),
            ),
            patch_entry("p1", binary(json!({"not":"a patch"}))),
        ],
    );
    let response = server
        .post("/")
        .add_header("content-type", "application/fhir+json; fhirVersion=5.0")
        .bytes(batch.to_string().into())
        .await;
    response.assert_status(StatusCode::OK);
    let body: Value = response.json();
    assert!(
        body["entry"][0]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("422"),
        "{body}"
    );
    assert!(
        body["entry"][1]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("400"),
        "{body}"
    );
    assert_eq!(patient(&server, "p1").await, before);

    let transaction = bundle(
        "transaction",
        vec![
            json!({"request":{"method":"POST","url":"Patient"},
               "resource":{"resourceType":"Patient","active":true}}),
            patch_entry(
                "p1",
                binary(json!([{"op":"test","path":"/active","value":true}])),
            ),
        ],
    );
    let response = server
        .post("/")
        .add_header("content-type", "application/fhir+json; fhirVersion=5.0")
        .bytes(transaction.to_string().into())
        .await;
    response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    let outcome: Value = response.json();
    assert_eq!(outcome["issue"][0]["code"], "processing", "{outcome}");
    assert_eq!(patient(&server, "p1").await, before);
    let search: Value = server.get("/Patient?_count=100").await.json();
    assert_eq!(
        search["entry"].as_array().map(Vec::len),
        Some(1),
        "{search}"
    );
}

const PROFILE_URL: &str = "http://example.org/StructureDefinition/bundle-patch-profile";

fn profile_resource() -> Value {
    json!({
        "resourceType":"StructureDefinition",
        "id":"bundle-patch-profile",
        "url":PROFILE_URL,
        "name":"BundlePatchProfile",
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
    })
}

fn relax_profile_patch() -> Value {
    json!({"resourceType":"Parameters","parameter":[{"name":"operation","part":[
        {"name":"type","valueCode":"replace"},
        {"name":"path","valueString":"StructureDefinition.differential.element[1].min"},
        {"name":"value","valueInteger":0}
    ]}]})
}

fn invalid_profile_patch() -> Value {
    json!({"resourceType":"Parameters","parameter":[{"name":"operation","part":[
        {"name":"type","valueCode":"add"},
        {"name":"path","valueString":"StructureDefinition"},
        {"name":"name","valueString":"bogusElement"},
        {"name":"value","valueBoolean":true}
    ]}]})
}

async fn seed_profile(server: &TestServer) {
    server
        .put("/StructureDefinition/bundle-patch-profile")
        .json(&profile_resource())
        .await
        .assert_status(StatusCode::CREATED);
}

async fn patient_conforming_to_profile(server: &TestServer) -> TestResponse {
    server
        .post("/Patient")
        .json(&json!({"resourceType":"Patient","meta":{"profile":[PROFILE_URL]}}))
        .await
}

#[tokio::test]
async fn batch_structure_definition_patch_refreshes_profile_only_after_success() {
    let server = server().await;
    seed_profile(&server).await;
    patient_conforming_to_profile(&server)
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    let original: Value = server
        .get("/StructureDefinition/bundle-patch-profile")
        .await
        .json();

    let failed = server
        .post("/")
        .json(&bundle(
            "batch",
            vec![patch_entry_for(
                "StructureDefinition",
                "bundle-patch-profile",
                invalid_profile_patch(),
            )],
        ))
        .await;
    failed.assert_status(StatusCode::OK);
    let body: Value = failed.json();
    assert!(
        body["entry"][0]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("422"),
        "{body}"
    );
    let after_failure: Value = server
        .get("/StructureDefinition/bundle-patch-profile")
        .await
        .json();
    assert_eq!(after_failure, original);
    patient_conforming_to_profile(&server)
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    let succeeded = server
        .post("/")
        .json(&bundle(
            "batch",
            vec![patch_entry_for(
                "StructureDefinition",
                "bundle-patch-profile",
                relax_profile_patch(),
            )],
        ))
        .await;
    succeeded.assert_status(StatusCode::OK);
    let body: Value = succeeded.json();
    assert!(
        body["entry"][0]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("200"),
        "{body}"
    );
    let stored: Value = server
        .get("/StructureDefinition/bundle-patch-profile")
        .await
        .json();
    assert_eq!(stored["differential"]["element"][1]["min"], 0);
    patient_conforming_to_profile(&server)
        .await
        .assert_status(StatusCode::CREATED);
}

#[tokio::test]
async fn transaction_structure_definition_patch_refreshes_profile_only_after_commit() {
    let server = server().await;
    seed_profile(&server).await;
    seed(&server, "p1").await;
    patient_conforming_to_profile(&server)
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    let failed = server
        .post("/")
        .json(&bundle(
            "transaction",
            vec![
                patch_entry_for(
                    "StructureDefinition",
                    "bundle-patch-profile",
                    relax_profile_patch(),
                ),
                patch_entry("p1", fhirpath_add_invalid()),
            ],
        ))
        .await;
    failed.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    let stored: Value = server
        .get("/StructureDefinition/bundle-patch-profile")
        .await
        .json();
    assert_eq!(stored["differential"]["element"][1]["min"], 1);
    patient_conforming_to_profile(&server)
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    let succeeded = server
        .post("/")
        .json(&bundle(
            "transaction",
            vec![patch_entry_for(
                "StructureDefinition",
                "bundle-patch-profile",
                relax_profile_patch(),
            )],
        ))
        .await;
    succeeded.assert_status(StatusCode::OK);
    let stored: Value = server
        .get("/StructureDefinition/bundle-patch-profile")
        .await
        .json();
    assert_eq!(stored["differential"]["element"][1]["min"], 0);
    patient_conforming_to_profile(&server)
        .await
        .assert_status(StatusCode::CREATED);
}
