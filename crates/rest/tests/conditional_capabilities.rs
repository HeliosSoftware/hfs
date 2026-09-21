//! #1384: `rest.resource.conditional*` and the `501` of an unsupported
//! conditional interaction come from one source — what the storage declares.
//!
//! The storage that lacks an interaction here is a production-shaped
//! composite (a primary with its index offloaded plus a dedicated search
//! backend — the `*-elasticsearch` arrangement, with a second SQLite standing
//! in for Elasticsearch so no container is needed). It resolves create /
//! update / delete criteria itself and cannot serve conditional patch.
//!
//! The statement used to advertise every conditional interaction for every
//! backend, and a conditional patch on such a composite answered `404`
//! whatever existed. Plain SQLite, which serves all four, is the
//! no-regression half; its `/metadata` is also asserted in
//! `conditional_patch.rs`.
//!
//! `conditionalPatch` exists from FHIR R5 on, so the statement only shows the
//! difference on a build with R5 or R6 enabled (`--features R4,R4B,R5,R6`);
//! the `501` shows on every build.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum_test::TestServer;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::composite::{
    CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage, SyncMode,
};
use helios_persistence::core::BackendKind;
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const IF_NONE_EXIST: HeaderName = HeaderName::from_static("if-none-exist");
const JSON_PATCH: &str = "application/json-patch+json";

fn tenant() -> HeaderValue {
    HeaderValue::from_static("test-tenant")
}

/// In-memory SQLite with the spec search parameters loaded: without the data
/// dir `identifier` is not registered and no criteria below would resolve.
fn sqlite() -> SqliteBackend {
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
    backend
}

fn config() -> ServerConfig {
    ServerConfig {
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "test-tenant".to_string(),
        ..ServerConfig::for_testing()
    }
}

fn sqlite_server() -> TestServer {
    let state = helios_rest::AppState::new(Arc::new(sqlite()), config());
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(state)).expect("test server")
}

fn composite_server() -> TestServer {
    let mut primary = sqlite();
    primary.set_search_offloaded(true);
    let primary = Arc::new(primary);
    let index = Arc::new(sqlite());

    let composite_config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .search_backend("search", BackendKind::Sqlite)
        .sync_mode(SyncMode::Synchronous)
        .fhir_version(FhirVersion::default())
        .build()
        .expect("composite config");

    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), primary.clone() as DynStorage);
    backends.insert("search".to_string(), index.clone() as DynStorage);
    let mut providers: HashMap<String, DynSearchProvider> = HashMap::new();
    providers.insert("sqlite".to_string(), primary.clone() as DynSearchProvider);
    providers.insert("search".to_string(), index as DynSearchProvider);

    let composite = CompositeStorage::new(composite_config, backends)
        .expect("composite")
        .with_search_providers(providers)
        .with_full_primary(primary)
        .start_sync_workers();

    let state = helios_rest::AppState::new(Arc::new(composite), config());
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(state)).expect("test server")
}

fn patient(identifier: &str) -> Value {
    json!({
        "resourceType": "Patient",
        "identifier": [{"system": "urn:zzz:probe", "value": identifier}],
        "active": false
    })
}

fn activate() -> Value {
    json!([{"op": "replace", "path": "/active", "value": true}])
}

/// `POST /Patient`, answering the new id.
async fn create(server: &TestServer, identifier: &str) -> String {
    let response = server
        .post("/Patient")
        .add_header(X_TENANT_ID, tenant())
        .json(&patient(identifier))
        .await;
    response.assert_status(StatusCode::CREATED);
    response.json::<Value>()["id"]
        .as_str()
        .expect("id")
        .to_string()
}

/// How many Patients a plain search by the probe identifier finds.
async fn found(server: &TestServer, identifier: &str) -> usize {
    let response = server
        .get(&format!("/Patient?identifier=urn:zzz:probe|{identifier}"))
        .add_header(X_TENANT_ID, tenant())
        .await;
    response.assert_status_ok();
    response.json::<Value>()["entry"]
        .as_array()
        .map_or(0, Vec::len)
}

/// The `Patient` entry of `/metadata` for every enabled FHIR version.
async fn patient_capabilities(server: &TestServer) -> Vec<(FhirVersion, Value)> {
    let mut entries = Vec::new();
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
        let entry = statement["rest"][0]["resource"]
            .as_array()
            .expect("resources")
            .iter()
            .find(|r| r["type"] == "Patient")
            .expect("Patient")
            .clone();
        entries.push((*version, entry));
    }
    entries
}

fn has_conditional_patch_element(version: FhirVersion) -> bool {
    matches!(version.as_mime_param(), "5.0" | "6.0")
}

/// A storage that cannot serve conditional patch says so, and still advertises
/// what it does serve. Conditional read is the REST layer's own (ETag /
/// `Last-Modified`), so it does not vary with the storage.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_statement_omits_what_the_storage_does_not_serve() {
    let server = composite_server();

    for (version, entry) in patient_capabilities(&server).await {
        assert_eq!(entry["conditionalCreate"], true, "{version:?}");
        assert_eq!(entry["conditionalUpdate"], true, "{version:?}");
        assert_eq!(entry["conditionalDelete"], "single", "{version:?}");
        assert_eq!(entry["conditionalRead"], "full-support", "{version:?}");
        assert_eq!(
            entry.get("conditionalPatch"),
            has_conditional_patch_element(version).then_some(&Value::Bool(false)),
            "{version:?}"
        );
    }
}

/// No regression: SQLite serves all four and says so.
#[tokio::test]
async fn sqlite_advertises_every_conditional_interaction() {
    let server = sqlite_server();

    for (version, entry) in patient_capabilities(&server).await {
        assert_eq!(entry["conditionalCreate"], true, "{version:?}");
        assert_eq!(entry["conditionalUpdate"], true, "{version:?}");
        assert_eq!(entry["conditionalDelete"], "single", "{version:?}");
        assert_eq!(entry["conditionalRead"], "full-support", "{version:?}");
        assert_eq!(
            entry.get("conditionalPatch"),
            has_conditional_patch_element(version).then_some(&Value::Bool(true)),
            "{version:?}"
        );
    }
}

/// The interaction the statement leaves out is refused as `501` +
/// `not-supported`, naming what was asked — not answered `404` as though
/// nothing matched — and changes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unsupported_conditional_patch_is_501_and_patches_nothing() {
    let server = composite_server();
    let id = create(&server, "P-1").await;
    // Positive control: the criteria do match, on this storage's search.
    assert_eq!(found(&server, "P-1").await, 1);

    let response = server
        .patch("/Patient?identifier=urn:zzz:probe|P-1")
        .add_header(X_TENANT_ID, tenant())
        .add_header(header::CONTENT_TYPE, HeaderValue::from_static(JSON_PATCH))
        .bytes(serde_json::to_vec(&activate()).expect("patch").into())
        .await;

    response.assert_status(StatusCode::NOT_IMPLEMENTED);
    let outcome: Value = response.json();
    assert_eq!(outcome["resourceType"], "OperationOutcome");
    assert_eq!(outcome["issue"][0]["code"], "not-supported");
    let text = outcome["issue"][0]["details"]["text"]
        .as_str()
        .or(outcome["issue"][0]["diagnostics"].as_str())
        .unwrap_or_default();
    assert!(
        text.contains("conditional patch (PATCH [type]?criteria)"),
        "{outcome}"
    );

    let read = server
        .get(&format!("/Patient/{id}"))
        .add_header(X_TENANT_ID, tenant())
        .await;
    read.assert_status_ok();
    let stored: Value = read.json();
    assert_eq!(stored["active"], false);
    assert_eq!(stored["meta"]["versionId"], "1");
}

/// The interactions the same storage does advertise are served: the check is
/// per interaction, not per backend.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_advertised_interactions_of_the_same_storage_are_served() {
    let server = composite_server();
    let id = create(&server, "P-2").await;
    assert_eq!(found(&server, "P-2").await, 1);

    // Conditional create: the match exists, so nothing is written.
    server
        .post("/Patient")
        .add_header(X_TENANT_ID, tenant())
        .add_header(
            IF_NONE_EXIST,
            HeaderValue::from_static("identifier=urn:zzz:probe|P-2"),
        )
        .json(&patient("P-2"))
        .await
        .assert_status_ok();
    assert_eq!(found(&server, "P-2").await, 1);

    // Conditional update of the one match.
    let mut updated = patient("P-2");
    updated["active"] = json!(true);
    let response = server
        .put("/Patient?identifier=urn:zzz:probe|P-2")
        .add_header(X_TENANT_ID, tenant())
        .json(&updated)
        .await;
    response.assert_status_ok();
    assert_eq!(response.json::<Value>()["id"], id.as_str());

    // Conditional delete of the one match.
    server
        .delete("/Patient?identifier=urn:zzz:probe|P-2")
        .add_header(X_TENANT_ID, tenant())
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_eq!(found(&server, "P-2").await, 0);
}

/// No regression: all four over plain SQLite.
#[tokio::test]
async fn sqlite_serves_every_conditional_interaction() {
    let server = sqlite_server();
    let id = create(&server, "P-3").await;
    assert_eq!(found(&server, "P-3").await, 1);

    server
        .post("/Patient")
        .add_header(X_TENANT_ID, tenant())
        .add_header(
            IF_NONE_EXIST,
            HeaderValue::from_static("identifier=urn:zzz:probe|P-3"),
        )
        .json(&patient("P-3"))
        .await
        .assert_status_ok();

    let response = server
        .patch("/Patient?identifier=urn:zzz:probe|P-3")
        .add_header(X_TENANT_ID, tenant())
        .add_header(header::CONTENT_TYPE, HeaderValue::from_static(JSON_PATCH))
        .bytes(serde_json::to_vec(&activate()).expect("patch").into())
        .await;
    response.assert_status_ok();
    assert_eq!(response.json::<Value>()["active"], true);

    let response = server
        .put("/Patient?identifier=urn:zzz:probe|P-3")
        .add_header(X_TENANT_ID, tenant())
        .json(&patient("P-3"))
        .await;
    response.assert_status_ok();
    assert_eq!(response.json::<Value>()["id"], id.as_str());

    server
        .delete("/Patient?identifier=urn:zzz:probe|P-3")
        .add_header(X_TENANT_ID, tenant())
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_eq!(found(&server, "P-3").await, 0);
}
