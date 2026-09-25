//! #1361: a conditional delete that matches nothing answers `204`.
//!
//! The handler returned `204` while its doc comment promised `404`. FHIR R4,
//! R4B and R5 word it identically (http.html#delete):
//!
//! > No matches or One Match: The server performs an ordinary delete on the
//! > matching resource
//!
//! and of an ordinary delete:
//!
//! > Upon successful deletion, or if the resource does not exist at all, the
//! > server should return either a 200 OK if the response contains a payload,
//! > or a 204 No Content with no response payload
//!
//! So the code was right and the comment wrong. Pinned here for the type-level
//! endpoint and for a batch entry, which must agree; a transaction refuses a
//! query-bearing `request.url` outright, so it has no no-match case.
//!
//! #1343: a bare `204` cannot tell "deleted" from "nothing matched". With
//! `Prefer: return=OperationOutcome` both delete endpoints answer `200` and an
//! informational OperationOutcome that says which it was; without it they stay
//! `204` with no body.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const PREFER: HeaderName = HeaderName::from_static("prefer");
const RETURN_OUTCOME: HeaderValue = HeaderValue::from_static("return=OperationOutcome");

fn tenant() -> HeaderValue {
    HeaderValue::from_static("test-tenant")
}

/// A server over in-memory SQLite with the spec search parameters loaded —
/// without them `identifier` is an unknown parameter and every delete a `400`.
async fn test_server() -> TestServer {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .canonicalize()
        .expect("repo data dir");
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("in-memory SQLite");
    backend.init_schema().expect("init schema");

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "test-tenant".to_string(),
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(Arc::new(backend), config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    TestServer::new(app).expect("test server")
}

async fn seed(server: &TestServer) {
    for (id, identifier) in [("a", "mrn-1"), ("b", "mrn-2")] {
        server
            .put(&format!("/Patient/{id}"))
            .add_header(X_TENANT_ID, tenant())
            .json(&json!({
                "resourceType": "Patient",
                "id": id,
                "name": [{"family": "Smith"}],
                "identifier": [{"system": "http://example.org/mrn", "value": identifier}]
            }))
            .await
            .assert_status_success();
    }
}

async fn ids(server: &TestServer) -> Vec<String> {
    let bundle: Value = server
        .get("/Patient?_count=100")
        .add_header(X_TENANT_ID, tenant())
        .await
        .json();
    let mut out: Vec<String> = bundle["entry"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| e["resource"]["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// The text is the same in every version served, so the answer is too.
#[tokio::test]
async fn no_match_is_204_with_no_body_in_every_fhir_version() {
    let server = test_server().await;
    seed(&server).await;

    for version in FhirVersion::enabled_versions() {
        let response = server
            .delete("/Patient?identifier=nobody")
            .add_header(X_TENANT_ID, tenant())
            .add_header(axum::http::header::ACCEPT, accept(version))
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::NO_CONTENT,
            "{version:?}: {}",
            response.text()
        );
        assert!(response.text().is_empty(), "{version:?}: a 204 has no body");
    }
    assert_eq!(ids(&server).await, ["a", "b"]);

    // Controls: the same criteria shape does delete when it matches one, is
    // idempotent afterwards, and refuses when it matches several.
    for _ in 0..2 {
        server
            .delete("/Patient?identifier=mrn-1")
            .add_header(X_TENANT_ID, tenant())
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(ids(&server).await, ["b"]);
    }
    seed(&server).await;
    server
        .delete("/Patient?family=Smith")
        .add_header(X_TENANT_ID, tenant())
        .await
        .assert_status(StatusCode::PRECONDITION_FAILED);
    assert_eq!(ids(&server).await, ["a", "b"]);
}

fn accept(version: &FhirVersion) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "application/fhir+json; fhirVersion={}",
        version.as_mime_param()
    ))
    .expect("accept")
}

/// Asserts a `200` carrying a single informational issue, and returns its text.
fn informational_text(response: &axum_test::TestResponse, context: &str) -> String {
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{context}: {}",
        response.text()
    );
    let outcome: Value = response.json();
    assert_eq!(
        outcome["resourceType"], "OperationOutcome",
        "{context}: {outcome}"
    );
    let issues = outcome["issue"].as_array().expect("issue array");
    assert_eq!(issues.len(), 1, "{context}: {outcome}");
    assert_eq!(issues[0]["severity"], "information", "{context}: {outcome}");
    assert_eq!(issues[0]["code"], "informational", "{context}: {outcome}");
    issues[0]["details"]["text"]
        .as_str()
        .expect("details.text")
        .to_string()
}

/// With `Prefer: return=OperationOutcome` a client can tell a delete from a
/// no-match: both are `200`, and the outcome names what happened.
#[tokio::test]
async fn prefer_operation_outcome_tells_deleted_from_no_match_in_every_fhir_version() {
    let server = test_server().await;

    for version in FhirVersion::enabled_versions() {
        seed(&server).await;
        let context = format!("{version:?}");

        let response = server
            .delete("/Patient?identifier=mrn-1")
            .add_header(X_TENANT_ID, tenant())
            .add_header(PREFER, RETURN_OUTCOME)
            .add_header(axum::http::header::ACCEPT, accept(version))
            .await;
        assert_eq!(
            informational_text(&response, &context),
            "Resource deleted: Patient/a"
        );
        assert_eq!(ids(&server).await, ["b"], "{context}");

        // The same criteria again now match nothing.
        let response = server
            .delete("/Patient?identifier=mrn-1")
            .add_header(X_TENANT_ID, tenant())
            .add_header(PREFER, RETURN_OUTCOME)
            .add_header(axum::http::header::ACCEPT, accept(version))
            .await;
        assert_eq!(
            informational_text(&response, &context),
            "No Patient matched the search criteria; nothing was deleted"
        );
        assert_eq!(ids(&server).await, ["b"], "{context}");
    }

    // Other return preferences keep the default empty `204`.
    seed(&server).await;
    for prefer in ["return=minimal", "return=representation"] {
        let response = server
            .delete("/Patient?identifier=nobody")
            .add_header(X_TENANT_ID, tenant())
            .add_header(PREFER, HeaderValue::from_static(prefer))
            .await;
        assert_eq!(response.status_code(), StatusCode::NO_CONTENT, "{prefer}");
        assert!(response.text().is_empty(), "{prefer}: a 204 has no body");
    }
}

/// Control: the instance delete honours the preference the same way, and a
/// missing resource is still `404` rather than an informational `200`.
#[tokio::test]
async fn instance_delete_honours_prefer_operation_outcome() {
    let server = test_server().await;
    seed(&server).await;

    let response = server
        .delete("/Patient/a")
        .add_header(X_TENANT_ID, tenant())
        .add_header(PREFER, RETURN_OUTCOME)
        .await;
    assert_eq!(
        informational_text(&response, "instance"),
        "Resource deleted: Patient/a"
    );
    assert_eq!(ids(&server).await, ["b"]);

    for missing in ["/Patient/a", "/Patient/never-existed"] {
        server
            .delete(missing)
            .add_header(X_TENANT_ID, tenant())
            .add_header(PREFER, RETURN_OUTCOME)
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }

    let response = server
        .delete("/Patient/b")
        .add_header(X_TENANT_ID, tenant())
        .await;
    assert_eq!(response.status_code(), StatusCode::NO_CONTENT);
    assert!(response.text().is_empty(), "a 204 has no body");
    assert!(ids(&server).await.is_empty());
}

/// A format the build cannot produce is refused as the formatter refuses it
/// (`406`), not turned into a `500` — and nothing is lost: the delete ran.
#[cfg(not(feature = "xml"))]
#[tokio::test]
async fn an_unproducible_format_is_406_not_500() {
    let server = test_server().await;
    seed(&server).await;

    server
        .delete("/Patient?identifier=mrn-1")
        .add_header(X_TENANT_ID, tenant())
        .add_header(PREFER, RETURN_OUTCOME)
        .add_header(
            axum::http::header::ACCEPT,
            HeaderValue::from_static("application/fhir+xml"),
        )
        .await
        .assert_status(StatusCode::NOT_ACCEPTABLE);
}

/// A batch entry answers what the endpoint answers, and its neighbours proceed.
#[tokio::test]
async fn a_batch_delete_entry_with_no_match_is_204() {
    let server = test_server().await;
    seed(&server).await;

    let response = server
        .post("/")
        .add_header(X_TENANT_ID, tenant())
        .json(&json!({
            "resourceType": "Bundle",
            "type": "batch",
            "entry": [
                {"request": {"method": "DELETE", "url": "Patient?identifier=nobody"}},
                {"request": {"method": "DELETE", "url": "Patient?identifier=mrn-1"}},
                {"request": {"method": "DELETE", "url": "Patient?family=Smith&identifier=nobody"}}
            ]
        }))
        .await;
    response.assert_status_ok();
    let reply: Value = response.json();
    for index in 0..3 {
        let entry = &reply["entry"][index];
        assert_eq!(entry["response"]["status"], "204 No Content", "{entry}");
        assert!(entry["response"].get("outcome").is_none(), "{entry}");
    }
    assert_eq!(ids(&server).await, ["b"]);
}
