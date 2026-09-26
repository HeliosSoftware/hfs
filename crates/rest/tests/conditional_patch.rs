//! #1361: conditional patch, `PATCH [base]/[type]?[search parameters]`.
//!
//! The handler existed but no route reached it: the request answered `405`.
//! FHIR R4, R4B and R5 word the outcomes identically (http.html#patch):
//!
//! > No matches: The server returns a 404 Not Found
//! > One Match: The server performs the update against the matching resource
//! > Multiple matches: The server returns a 412 Precondition Failed error
//! > indicating the client's criteria were not selective enough
//!
//! The criteria go through the pipeline every conditional interaction shares,
//! so the defects fixed there (#1312, #1321–#1323, #1360) are asserted here
//! once each, against decoys a wrong-target patch would be visible on.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum_test::{TestRequest, TestServer};
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const PREFER: HeaderName = HeaderName::from_static("prefer");
const JSON_PATCH: &str = "application/json-patch+json";
const MERGE_PATCH: &str = "application/merge-patch+json";

fn tenant() -> HeaderValue {
    HeaderValue::from_static("test-tenant")
}

/// A server over in-memory SQLite with the spec search parameters loaded.
/// Without the data dir only five embedded parameters exist, `family` and
/// `identifier` are unknown, and every criteria below would be a `400`.
async fn server_with_routing(routing_mode: TenantRoutingMode) -> TestServer {
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
            routing_mode,
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

async fn test_server() -> TestServer {
    server_with_routing(TenantRoutingMode::HeaderOnly).await
}

/// Two Smiths and a Neal whose identifier starts with comparator letters.
async fn seed_at(server: &TestServer, prefix: &str) {
    for (id, family, identifier) in [
        ("target", "Neal", "ne123"),
        ("smith-a", "Smith", "mrn-1"),
        ("smith-b", "Smith", "mrn-2"),
    ] {
        server
            .put(&format!("{prefix}/Patient/{id}"))
            .add_header(X_TENANT_ID, tenant())
            .json(&json!({
                "resourceType": "Patient",
                "id": id,
                "active": false,
                "name": [{"family": family}],
                "identifier": [{"system": "http://example.org/mrn", "value": identifier}]
            }))
            .await
            .assert_status(StatusCode::CREATED);
    }
}

async fn seed(server: &TestServer) {
    seed_at(server, "").await;
}

/// `id -> (versionId, active, family)` for every Patient: any write at all,
/// to anybody, shows up here.
async fn snapshot(server: &TestServer) -> Vec<(String, String, String, String)> {
    let bundle: Value = server
        .get("/Patient?_count=100")
        .add_header(X_TENANT_ID, tenant())
        .await
        .json();
    let mut out: Vec<_> = bundle["entry"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|e| {
                    let r = &e["resource"];
                    (
                        r["id"].as_str().unwrap_or("?").to_string(),
                        r["meta"]["versionId"].as_str().unwrap_or("?").to_string(),
                        r["active"].to_string(),
                        r["name"][0]["family"].as_str().unwrap_or("?").to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn row(id: &str, version: &str, active: &str, family: &str) -> (String, String, String, String) {
    (
        id.to_string(),
        version.to_string(),
        active.to_string(),
        family.to_string(),
    )
}

fn untouched() -> Vec<(String, String, String, String)> {
    vec![
        row("smith-a", "1", "false", "Smith"),
        row("smith-b", "1", "false", "Smith"),
        row("target", "1", "false", "Neal"),
    ]
}

fn activate() -> Value {
    json!([{"op": "replace", "path": "/active", "value": true}])
}

fn patch(server: &TestServer, url: &str, content_type: &'static str, body: &Value) -> TestRequest {
    server
        .patch(url)
        .add_header(X_TENANT_ID, tenant())
        .add_header(header::CONTENT_TYPE, HeaderValue::from_static(content_type))
        .bytes(body.to_string().into())
}

fn assert_outcome(response: &axum_test::TestResponse, status: StatusCode, context: &str) -> Value {
    assert_eq!(
        response.status_code(),
        status,
        "{context}: {}",
        response.text()
    );
    let outcome: Value = response.json();
    assert_eq!(outcome["resourceType"], "OperationOutcome", "{context}");
    outcome
}

// =============================================================================
// One match
// =============================================================================

#[tokio::test]
async fn one_match_is_patched_with_json_patch() {
    let server = test_server().await;
    seed(&server).await;

    // Positive control: the criteria select the target as a direct search.
    let found: Value = server
        .get("/Patient?identifier=ne123")
        .add_header(X_TENANT_ID, tenant())
        .await
        .json();
    assert_eq!(found["entry"][0]["resource"]["id"], "target");

    // `ne123` starts with comparator letters (#1312); the system|value form is
    // percent-encoded, as a client would send it (#1322).
    for (round, criteria) in [
        "identifier=ne123",
        "identifier=http%3A%2F%2Fexample.org%2Fmrn%7Cne123",
        "family=Neal&_format=json",
    ]
    .into_iter()
    .enumerate()
    {
        let response = patch(
            &server,
            &format!("/Patient?{criteria}"),
            JSON_PATCH,
            &json!([{"op": "replace", "path": "/active", "value": round % 2 == 0}]),
        )
        .await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "{criteria}: {}",
            response.text()
        );
        let body: Value = response.json();
        assert_eq!(body["id"], "target", "{criteria}");
        assert_eq!(body["active"], round % 2 == 0, "{criteria}");
        assert_eq!(body["name"][0]["family"], "Neal", "{criteria}");
        let version = (round + 2).to_string();
        assert_eq!(body["meta"]["versionId"], version.as_str(), "{criteria}");
        assert_eq!(
            response.header("etag").to_str().expect("etag"),
            format!("W/\"{version}\""),
            "{criteria}"
        );
    }

    assert_eq!(
        snapshot(&server).await,
        vec![
            row("smith-a", "1", "false", "Smith"),
            row("smith-b", "1", "false", "Smith"),
            row("target", "4", "true", "Neal"),
        ]
    );
}

#[tokio::test]
async fn one_match_is_patched_with_merge_patch_and_prefer_is_honoured() {
    let server = test_server().await;
    seed(&server).await;

    let response = patch(
        &server,
        "/Patient?identifier=ne123",
        MERGE_PATCH,
        &json!({"active": true, "resourceType": "Patient"}),
    )
    .add_header(PREFER, HeaderValue::from_static("return=minimal"))
    .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    assert!(response.text().is_empty(), "return=minimal carries no body");

    assert_eq!(
        snapshot(&server).await,
        vec![
            row("smith-a", "1", "false", "Smith"),
            row("smith-b", "1", "false", "Smith"),
            row("target", "2", "true", "Neal"),
        ]
    );
}

/// Every occurrence of a repeated parameter counts (#1321): the second
/// `family` excludes the target, so nothing matches.
#[tokio::test]
async fn repeated_parameters_are_all_applied() {
    let server = test_server().await;
    seed(&server).await;

    let response = patch(
        &server,
        "/Patient?family=Smith&family=Neal",
        JSON_PATCH,
        &activate(),
    )
    .await;
    assert_outcome(&response, StatusCode::NOT_FOUND, "family=Smith&family=Neal");
    assert_eq!(snapshot(&server).await, untouched());
}

// =============================================================================
// No match, multiple matches
// =============================================================================

#[tokio::test]
async fn no_match_is_404_and_creates_nothing() {
    let server = test_server().await;
    seed(&server).await;

    for criteria in ["identifier=nobody", "identifier=123", "_format=json"] {
        let response = patch(
            &server,
            &format!("/Patient?{criteria}"),
            JSON_PATCH,
            &activate(),
        )
        .await;
        let outcome = assert_outcome(&response, StatusCode::NOT_FOUND, criteria);
        assert_eq!(outcome["issue"][0]["code"], "not-found", "{criteria}");
    }
    assert_eq!(snapshot(&server).await, untouched());
}

#[tokio::test]
async fn multiple_matches_are_412_and_patch_nobody() {
    let server = test_server().await;
    seed(&server).await;

    let response = patch(&server, "/Patient?family=Smith", JSON_PATCH, &activate()).await;
    assert_outcome(&response, StatusCode::PRECONDITION_FAILED, "family=Smith");
    assert_eq!(snapshot(&server).await, untouched());
}

// =============================================================================
// Refusals
// =============================================================================

#[tokio::test]
async fn criteria_that_cannot_be_trusted_are_400() {
    let server = test_server().await;
    seed(&server).await;

    for (criteria, named) in [
        // Unknown parameter (#1323), whatever `Prefer: handling` says.
        ("identifer=ne123", "'identifer'"),
        ("identifier=ne123&nonsense=1", "'nonsense'"),
        // Empty value (#1360).
        ("identifier=&family=Neal", "'identifier'"),
        ("identifier", "'identifier'"),
        // An unknown modifier.
        ("family:bogus=Neal", "bogus"),
    ] {
        for prefer in ["handling=lenient", "handling=strict"] {
            let response = patch(
                &server,
                &format!("/Patient?{criteria}"),
                JSON_PATCH,
                &activate(),
            )
            .add_header(PREFER, HeaderValue::from_static(prefer))
            .await;
            let outcome = assert_outcome(&response, StatusCode::BAD_REQUEST, criteria);
            assert!(
                outcome.to_string().contains(named),
                "{criteria}: the outcome must name {named}: {outcome}"
            );
        }
    }

    // Criteria this layer cannot evaluate are refused as for PUT / DELETE: not
    // searched for literally, which would be a 404.
    let response = patch(
        &server,
        "/Patient?general-practitioner.name=x",
        JSON_PATCH,
        &activate(),
    )
    .await;
    assert_outcome(&response, StatusCode::NOT_IMPLEMENTED, "chained criteria");

    // No criteria at all: neither an instance nor a condition.
    for url in ["/Patient", "/Patient?", "/Patient?&"] {
        let response = patch(&server, url, JSON_PATCH, &activate()).await;
        assert_outcome(&response, StatusCode::BAD_REQUEST, url);
    }

    assert_eq!(snapshot(&server).await, untouched());
}

/// `If-Match` is honoured, as on the instance endpoint (#1381; it was refused
/// with `400` before). `tests/conditional_if_match.rs` covers the matrix; this
/// keeps the one-line contract beside the rest of conditional patch.
#[tokio::test]
async fn if_match_is_honoured_rather_than_refused() {
    let server = test_server().await;
    seed(&server).await;

    // Control: the instance endpoint enforces it.
    patch(&server, "/Patient/target", JSON_PATCH, &activate())
        .add_header(header::IF_MATCH, HeaderValue::from_static("W/\"7\""))
        .await
        .assert_status(StatusCode::PRECONDITION_FAILED);

    for if_match in ["W/\"7\"", "garbage"] {
        let response = patch(
            &server,
            "/Patient?identifier=ne123",
            JSON_PATCH,
            &activate(),
        )
        .add_header(
            header::IF_MATCH,
            HeaderValue::from_str(if_match).expect("header"),
        )
        .await;
        let outcome = assert_outcome(&response, StatusCode::PRECONDITION_FAILED, if_match);
        assert!(outcome.to_string().contains("If-Match"), "{outcome}");
    }
    assert_eq!(snapshot(&server).await, untouched());

    patch(
        &server,
        "/Patient?identifier=ne123",
        JSON_PATCH,
        &activate(),
    )
    .add_header(header::IF_MATCH, HeaderValue::from_static("W/\"1\""))
    .await
    .assert_status(StatusCode::OK);
    assert_ne!(snapshot(&server).await, untouched());
}

/// What the instance endpoint refuses, the conditional one refuses the same
/// way — before anything is written.
#[tokio::test]
async fn patch_documents_are_held_to_the_instance_endpoints_rules() {
    let server = test_server().await;
    seed(&server).await;
    for url in ["/Patient/target", "/Patient?identifier=ne123"] {
        // `resourceType` cannot be patched, nor can `id` (#1406: a patched
        // `id` used to be silently undone by the backend and answered `200`).
        for (content_type, body) in [
            (
                JSON_PATCH,
                json!([{"op": "replace", "path": "/resourceType", "value": "Person"}]),
            ),
            (MERGE_PATCH, json!({"resourceType": "Person"})),
            (
                JSON_PATCH,
                json!([{"op": "replace", "path": "/id", "value": "other"}]),
            ),
            (MERGE_PATCH, json!({"id": "other"})),
        ] {
            let response = patch(&server, url, content_type, &body).await;
            assert_outcome(&response, StatusCode::BAD_REQUEST, &format!("{url} {body}"));
        }

        // A patch that does not apply, and one that is not a patch.
        for body in [
            json!([{"op": "replace", "path": "/nope/deeper", "value": 1}]),
            json!({"not": "a patch"}),
        ] {
            let response = patch(&server, url, JSON_PATCH, &body).await;
            assert_outcome(&response, StatusCode::BAD_REQUEST, &format!("{url} {body}"));
        }

        let failed_test = json!([{"op": "test", "path": "/active", "value": true}]);
        let response = patch(&server, url, JSON_PATCH, &failed_test).await;
        assert_outcome(&response, StatusCode::UNPROCESSABLE_ENTITY, url);

        patch(&server, url, "text/plain", &activate())
            .await
            .assert_status(StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    // AuditEvent is immutable on both endpoints.
    for url in ["/AuditEvent/x", "/AuditEvent?_id=x"] {
        patch(&server, url, JSON_PATCH, &activate())
            .await
            .assert_status(StatusCode::METHOD_NOT_ALLOWED);
    }

    assert_eq!(snapshot(&server).await, untouched());
}

#[tokio::test]
async fn fhirpath_patch_works_on_instance_and_conditional_routes() {
    let server = test_server().await;
    seed(&server).await;
    let body = json!({
        "resourceType": "Parameters",
        "parameter": [{"name": "operation", "part": [
            {"name": "type", "valueCode": "replace"},
            {"name": "path", "valueString": "Patient.name[0].family"},
            {"name": "value", "valueString": "Changed"}
        ]}]
    });

    for url in ["/Patient/target", "/Patient?identifier=ne123"] {
        let response = patch(&server, url, "application/fhir+json", &body).await;
        response.assert_status(StatusCode::OK);
        let patched: Value = response.json();
        assert_eq!(patched["name"][0]["family"], "Changed");
    }
}

// =============================================================================
// Routing and advertising
// =============================================================================

/// The route exists wherever conditional `PUT` / `DELETE` do: all three
/// tenancy modes share one router, behind a prefix-stripping layer.
#[tokio::test]
async fn the_route_exists_in_every_tenant_routing_mode() {
    for (mode, prefix) in [
        (TenantRoutingMode::HeaderOnly, ""),
        (TenantRoutingMode::UrlPath, "/test-tenant"),
        (TenantRoutingMode::Both, "/test-tenant"),
        (TenantRoutingMode::Both, ""),
    ] {
        let server = server_with_routing(mode).await;
        seed_at(&server, prefix).await;

        let response = patch(
            &server,
            &format!("{prefix}/Patient?identifier=ne123"),
            JSON_PATCH,
            &activate(),
        )
        .await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "{mode:?} {prefix:?}: {}",
            response.text()
        );
        let body: Value = response.json();
        assert_eq!(body["id"], "target", "{mode:?} {prefix:?}");
        assert_eq!(body["active"], true, "{mode:?} {prefix:?}");

        // The neighbours on the same path still answer.
        server
            .delete(&format!("{prefix}/Patient?identifier=nobody"))
            .add_header(X_TENANT_ID, tenant())
            .await
            .assert_status(StatusCode::NO_CONTENT);
    }
}

/// `rest.resource.conditionalPatch` exists from R5 on. R4 and R4B have no such
/// element, so the statement must not carry it there — the interaction is
/// served all the same.
#[tokio::test]
async fn the_capability_statement_advertises_it_where_the_element_exists() {
    let server = test_server().await;

    for version in FhirVersion::enabled_versions() {
        let accept = format!(
            "application/fhir+json; fhirVersion={}",
            version.as_mime_param()
        );
        let response = server
            .get("/metadata")
            .add_header(X_TENANT_ID, tenant())
            .add_header(
                header::ACCEPT,
                HeaderValue::from_str(&accept).expect("accept"),
            )
            .await;
        response.assert_status_ok();
        let statement: Value = serde_json::from_str(&response.text()).expect("json");
        let resources = statement["rest"][0]["resource"]
            .as_array()
            .expect("resources");
        let of = |resource_type: &str| {
            resources
                .iter()
                .find(|r| r["type"] == resource_type)
                .unwrap_or_else(|| panic!("{resource_type} in {version:?}"))
        };

        let patient = of("Patient");
        assert_eq!(patient["conditionalUpdate"], true, "{version:?}");
        assert_eq!(patient["conditionalDelete"], "single", "{version:?}");
        let has_element = matches!(version.as_mime_param(), "5.0" | "6.0");
        assert_eq!(
            patient.get("conditionalPatch"),
            has_element.then_some(&Value::Bool(true)),
            "{version:?}"
        );
        // Immutable: no conditional write of any kind.
        assert!(of("AuditEvent").get("conditionalPatch").is_none());
    }
}
