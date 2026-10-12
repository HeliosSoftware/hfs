//! `_in` (List/Group membership, R5/R6) search (#638).
//!
//! Runs the REST stack against in-memory SQLite configured as R5 — both the
//! server default and the backend's search-parameter registry, which are
//! separate knobs and must agree. Every positive assertion has a negative twin
//! (a non-member that must be absent), so a dropped filter reads as a failure
//! rather than as "everything matched".

#![cfg(all(feature = "R4", feature = "R5"))]

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

fn tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    )
}

async fn create(backend: &SqliteBackend, resource_type: &str, resource: Value) {
    backend
        .create(&tenant(), resource_type, resource, FhirVersion::R5)
        .await
        .unwrap_or_else(|e| panic!("seeding {resource_type} failed: {e}"));
}

async fn create_test_server() -> (TestServer, Arc<SqliteBackend>) {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"));

    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            fhir_version: FhirVersion::R5,
            ..Default::default()
        },
    )
    .expect("Failed to create SQLite backend");
    backend.init_schema().expect("Failed to init schema");
    let backend = Arc::new(backend);

    let config = ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "test-tenant".to_string(),
        default_fhir_version: FhirVersion::R5,
        seed_conformance: false,
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(Arc::clone(&backend), config);
    let router = helios_rest::routing::fhir_routes::create_routes(state);
    (
        TestServer::new(router).expect("Failed to create test server"),
        backend,
    )
}

/// Patients p1 (male), p2 (female), p3 (male); List `l1` = {p1, p2};
/// enumerated Group `g1` = {p1, p3 active; p2 inactive}; Observations o1 (of
/// p1) and o2 (of p1) with List `ol` = {o1}.
async fn seed(backend: &SqliteBackend) {
    for (id, gender) in [("p1", "male"), ("p2", "female"), ("p3", "male")] {
        create(
            backend,
            "Patient",
            json!({"resourceType": "Patient", "id": id, "gender": gender}),
        )
        .await;
    }
    create(
        backend,
        "List",
        json!({"resourceType": "List", "id": "l1", "status": "current", "mode": "working",
               "entry": [{"item": {"reference": "Patient/p1"}}, {"item": {"reference": "Patient/p2"}}]}),
    )
    .await;
    create(
        backend,
        "Group",
        json!({"resourceType": "Group", "id": "g1", "type": "person", "membership": "enumerated",
               "member": [
                   {"entity": {"reference": "Patient/p1"}},
                   {"entity": {"reference": "Patient/p3"}},
                   {"entity": {"reference": "Patient/p2"}, "inactive": true}]}),
    )
    .await;
    create(
        backend,
        "Group",
        json!({"resourceType": "Group", "id": "gdef", "type": "person", "membership": "definitional"}),
    )
    .await;
    for id in ["o1", "o2"] {
        create(
            backend,
            "Observation",
            json!({"resourceType": "Observation", "id": id, "status": "final",
                   "code": {"text": "x"}, "subject": {"reference": "Patient/p1"}}),
        )
        .await;
    }
    create(
        backend,
        "List",
        json!({"resourceType": "List", "id": "ol", "status": "current", "mode": "working",
               "entry": [{"item": {"reference": "Observation/o1"}}]}),
    )
    .await;
}

async fn get(server: &TestServer, url: &str) -> axum_test::TestResponse {
    server
        .get(url)
        .add_header(X_TENANT_ID, HeaderValue::from_static("test-tenant"))
        .await
}

/// Sorted ids of the `match` entries of a search; asserts a `200`.
async fn ids(server: &TestServer, url: &str) -> Vec<String> {
    let response = get(server, url).await;
    response.assert_status_ok();
    let body: Value = response.json();
    let mut ids: Vec<String> = body["entry"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter(|e| e["search"]["mode"] == "match")
                .map(|e| e["resource"]["id"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

#[tokio::test]
async fn list_membership_filters_to_members() {
    let (server, backend) = create_test_server().await;
    seed(&backend).await;

    // p3 is not in the List: it must be absent.
    assert_eq!(ids(&server, "/Patient?_in=List/l1").await, ["p1", "p2"]);
    assert_eq!(ids(&server, "/Patient?_in:List=l1").await, ["p1", "p2"]);
}

#[tokio::test]
async fn group_membership_filters_to_active_members() {
    let (server, backend) = create_test_server().await;
    seed(&backend).await;

    // p2 is an inactive member, so it must be absent.
    assert_eq!(ids(&server, "/Patient?_in=Group/g1").await, ["p1", "p3"]);
    // A bare id matches a List or a Group with that id.
    assert_eq!(ids(&server, "/Patient?_in=g1").await, ["p1", "p3"]);
    assert_eq!(ids(&server, "/Patient?_in=l1").await, ["p1", "p2"]);
}

#[tokio::test]
async fn membership_combines_with_other_criteria_and_other_values() {
    let (server, backend) = create_test_server().await;
    seed(&backend).await;

    assert_eq!(
        ids(&server, "/Patient?_in=List/l1&gender=male").await,
        ["p1"]
    );
    // Comma: union. Repeated: intersection.
    assert_eq!(
        ids(&server, "/Patient?_in=List/l1,Group/g1").await,
        ["p1", "p2", "p3"]
    );
    assert_eq!(
        ids(&server, "/Patient?_in=List/l1&_in=Group/g1").await,
        ["p1"]
    );
}

#[tokio::test]
async fn missing_container_yields_an_empty_result() {
    let (server, backend) = create_test_server().await;
    seed(&backend).await;

    assert!(ids(&server, "/Patient?_in=List/nope").await.is_empty());
    assert!(ids(&server, "/Patient?_in=nope").await.is_empty());
}

#[tokio::test]
async fn compartment_search_resolves_membership() {
    let (server, backend) = create_test_server().await;
    seed(&backend).await;

    // o2 is in p1's compartment but not in the List.
    assert_eq!(
        ids(&server, "/Patient/p1/Observation?_in=List/ol").await,
        ["o1"]
    );
    assert_eq!(
        ids(&server, "/Patient/p1/Observation?_in=List/nope")
            .await
            .len(),
        0
    );
}

#[tokio::test]
async fn unsupported_forms_are_refused_explicitly() {
    let (server, backend) = create_test_server().await;
    seed(&backend).await;

    for url in [
        "/Patient?_in=List/l1&_contained=true",
        "/Observation?patient._in=List/l1",
        "/Patient?_has:Observation:patient:_in=List/ol",
        "/Patient?_in:not=List/l1",
        "/Patient?_in=Patient/p1",
        "/Patient?_in=Group/gdef",
    ] {
        let response = get(&server, url).await;
        assert_eq!(response.status_code(), StatusCode::BAD_REQUEST, "{url}");
        let body: Value = response.json();
        assert_eq!(body["resourceType"], "OperationOutcome", "{url}");
    }

    // System-level search is refused wholesale (#1338), so `_in` never reaches it.
    for url in [
        "/Patient?_in=CareTeam/c1",
        "/Patient?_in:CareTeam=c1",
        "/?_in=List/l1",
    ] {
        let response = get(&server, url).await;
        assert_eq!(response.status_code(), StatusCode::NOT_IMPLEMENTED, "{url}");
    }
}

#[tokio::test]
async fn capability_statement_advertises_in() {
    let (server, _backend) = create_test_server().await;

    let response = get(&server, "/metadata").await;
    response.assert_status_ok();
    let body: Value = response.json();
    let patient = body["rest"][0]["resource"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["type"] == "Patient")
        .expect("Patient is advertised");
    let names: Vec<&str> = patient["searchParam"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["name"].as_str())
        .collect();
    assert!(names.contains(&"_in"), "advertises _in on R5");
}
