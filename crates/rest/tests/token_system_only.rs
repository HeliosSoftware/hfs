//! `[system]|` token searches on an Identifier with a system and no value,
//! and a Coding with a system and no code (#2965), end to end through the
//! REST layer.
//!
//! FHIR defines `[parameter]=[system]|` as "any element where the value of
//! [system] matches the system property of the Identifier or Coding"
//! (search.html#token), and `Identifier.value` and `Coding.code` are both
//! 0..1. Such an element used to leave no search index row, so `system|`
//! and `:missing=false` missed it.
//!
//! The persistence-level table lives in
//! `crates/persistence/tests/search/token_system_only_suite.rs` and runs on
//! all four backends; this file pins the same behaviour over HTTP on SQLite,
//! including what the REST write paths do to empty strings first: a `PUT`
//! drops `"code": ""`, while `$meta-add` keeps it (#2554).

use axum::http::StatusCode;
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use serde_json::{Value, json};

use helios_rest::{ServerConfig, create_app_with_config};

const MRN: &str = "http://hospital.example/mrn";
const LOINC: &str = "http://loinc.org";
const TAGS: &str = "http://example.org/tags";

/// An in-memory HFS with the spec search parameters loaded, seeded with:
///
/// - `p-sysonly`: an Identifier with ONLY a system; `p-sysvalue`: the same
///   system with value `123`, the control; `p-none`: no identifier.
/// - `o-sysonly` (subject `p-sysonly`): a Coding with ONLY a system, LOINC;
///   `o-coded` (subject `p-sysvalue`): LOINC `1234-5`, the control.
/// - `o-tag`: a bare `meta.tag` Coding of `TAGS` with `"code": ""`, added
///   by `$meta-add`.
async fn seeded_server() -> TestServer {
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
    let server =
        TestServer::new(create_app_with_config(backend, ServerConfig::for_testing())).unwrap();

    for resource in [
        json!({"resourceType": "Patient", "id": "p-sysonly",
               "identifier": [{"system": MRN}]}),
        json!({"resourceType": "Patient", "id": "p-sysvalue",
               "identifier": [{"system": MRN, "value": "123"}]}),
        json!({"resourceType": "Patient", "id": "p-none", "active": true}),
        json!({"resourceType": "Observation", "id": "o-sysonly", "status": "final",
               "code": {"coding": [{"system": LOINC}]},
               "subject": {"reference": "Patient/p-sysonly"}}),
        json!({"resourceType": "Observation", "id": "o-coded", "status": "final",
               "code": {"coding": [{"system": LOINC, "code": "1234-5"}]},
               "subject": {"reference": "Patient/p-sysvalue"}}),
        json!({"resourceType": "Observation", "id": "o-tag", "status": "final",
               "code": {"text": "untagged"}}),
    ] {
        let path = format!(
            "/{}/{}",
            resource["resourceType"].as_str().unwrap(),
            resource["id"].as_str().unwrap()
        );
        let response = server.put(&path).json(&resource).await;
        assert!(response.status_code().is_success(), "seeding {path}");
    }

    let response = server
        .post("/Observation/o-tag/$meta-add")
        .json(&json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "meta",
                "valueMeta": {"tag": [{"system": TAGS, "code": ""}]}
            }]
        }))
        .await;
    assert_eq!(response.status_code(), StatusCode::OK, "$meta-add");
    let stored: Value = server.get("/Observation/o-tag").await.json();
    assert_eq!(
        stored["meta"]["tag"],
        json!([{"system": TAGS, "code": ""}]),
        "$meta-add keeps the empty code this test relies on"
    );
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
async fn system_pipe_matches_an_identifier_or_coding_without_a_value() {
    let server = seeded_server().await;
    let mrn_any = format!("{MRN}|");
    let mrn_123 = format!("{MRN}|123");
    let loinc_any = format!("{LOINC}|");
    let loinc_code = format!("{LOINC}|1234-5");
    let tags_any = format!("{TAGS}|");

    for (resource_type, key, value, expected) in [
        // Positive controls: the parameters index.
        ("Patient", "identifier", "123", &["p-sysvalue"][..]),
        ("Observation", "code", "1234-5", &["o-coded"]),
        // The bug: these missed the element with only a system.
        (
            "Patient",
            "identifier",
            mrn_any.as_str(),
            &["p-sysonly", "p-sysvalue"],
        ),
        (
            "Patient",
            "identifier:missing",
            "false",
            &["p-sysonly", "p-sysvalue"],
        ),
        ("Patient", "identifier:not", mrn_any.as_str(), &["p-none"]),
        (
            "Observation",
            "code",
            loinc_any.as_str(),
            &["o-coded", "o-sysonly"],
        ),
        ("Observation", "_tag", tags_any.as_str(), &["o-tag"]),
        (
            "Observation",
            "subject:Patient.identifier",
            mrn_any.as_str(),
            &["o-coded", "o-sysonly"],
        ),
        (
            "Patient",
            "_has:Observation:subject:code",
            loinc_any.as_str(),
            &["p-sysonly", "p-sysvalue"],
        ),
        // CONTROLS: an empty code matches no code-naming form.
        ("Patient", "identifier", mrn_123.as_str(), &["p-sysvalue"]),
        ("Patient", "identifier", "|123", &[]),
        ("Patient", "identifier:missing", "true", &["p-none"]),
        ("Observation", "code", loinc_code.as_str(), &["o-coded"]),
    ] {
        assert_eq!(
            search(&server, resource_type, key, value).await,
            expected,
            "{resource_type}?{key}={value}"
        );
    }
}
