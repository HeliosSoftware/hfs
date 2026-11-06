//! The `Location` a create — or an update that creates — answers (#2517).
//!
//! FHIR R4/R4B/R5 http.html#create: "The server returns a 201 Created HTTP
//! status code, and SHALL also return a Location header which contains the
//! new Logical Id and Version Id of the created resource version:
//! `Location: [base]/[type]/[id]/_history/[vid]` ... The Location header
//! should be as specific as possible - if the server understands versioning,
//! the version is included. If a server does not track versions, the Location
//! header will just contain [base]/[type]/[id]."
//!
//! R5 http.html#update asks the same of an update that answers 201 (R5
//! #upsert's "it SHALL be the same as the location in the URL of the PUT
//! request" reads the other way; the versioned form follows #update and
//! #create, and names that same resource), and R5
//! http.html#ccreate has a conditional create's single match answer "with
//! headers and body populated as they would have been if a create had
//! actually occurred (i.e. ... location and etag headers set, etc.)".
//!
//! Batch entries carry the same versioned `response.location`, relative to
//! the base ("The Location MAY be an absolute or relative URL").

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::{TestResponse, TestServer};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use helios_rest::{AppState, ServerConfig};
use serde_json::{Value, json};

const BASE: &str = "https://fhir.example/r4";
const CONTENT_TYPE: HeaderName = HeaderName::from_static("content-type");
const PREFER: HeaderName = HeaderName::from_static("prefer");
const IF_NONE_EXIST: HeaderName = HeaderName::from_static("if-none-exist");
const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const FHIR_JSON: HeaderValue = HeaderValue::from_static("application/fhir+json");

/// An in-memory SQLite server at [`BASE`], spec search parameters loaded so
/// `If-None-Exist: identifier=...` resolves.
fn server_with(config: ServerConfig) -> TestServer {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .map(|path| path.join("data"));
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir,
            ..Default::default()
        },
    )
    .unwrap();
    backend.init_schema().unwrap();
    let state = AppState::new(
        Arc::new(backend),
        ServerConfig {
            base_url: BASE.to_string(),
            ..config
        },
    );
    TestServer::new(helios_rest::routing::fhir_routes::create_routes(state)).unwrap()
}

fn server() -> TestServer {
    server_with(ServerConfig::for_testing())
}

fn location(response: &TestResponse) -> String {
    response
        .headers()
        .get("location")
        .unwrap_or_else(|| panic!("no Location on {}", response.status_code()))
        .to_str()
        .unwrap()
        .to_string()
}

fn etag_version(response: &TestResponse) -> String {
    let etag = response.headers()["etag"].to_str().unwrap();
    etag.trim_start_matches("W/").trim_matches('"').to_string()
}

/// The server-relative path of an absolute `Location` under [`BASE`].
fn path_of(location: &str) -> &str {
    location
        .strip_prefix(BASE)
        .unwrap_or_else(|| panic!("{location} is not under {BASE}"))
}

async fn post_patient(server: &TestServer, body: Value) -> TestResponse {
    server
        .post("/Patient")
        .add_header(CONTENT_TYPE, FHIR_JSON)
        .json(&body)
        .await
}

async fn put_patient(server: &TestServer, id: &str, body: Value) -> TestResponse {
    server
        .put(&format!("/Patient/{id}"))
        .add_header(CONTENT_TYPE, FHIR_JSON)
        .json(&body)
        .await
}

/// `POST [type]` answers `[base]/[type]/[id]/_history/1`, and that URL is a
/// vread of the created version — still after the resource moves on.
#[tokio::test]
async fn create_location_names_the_created_version_and_vreads_it() {
    let server = server();
    let created = post_patient(
        &server,
        json!({"resourceType": "Patient", "name": [{"family": "Location"}]}),
    )
    .await;
    created.assert_status(StatusCode::CREATED);
    let id = created.json::<Value>()["id"].as_str().unwrap().to_string();
    let location = location(&created);
    assert_eq!(location, format!("{BASE}/Patient/{id}/_history/1"));
    assert_eq!(etag_version(&created), "1", "Location and ETag agree");

    // A newer version must not change what the Location names.
    put_patient(
        &server,
        &id,
        json!({"resourceType": "Patient", "id": id, "name": [{"family": "Moved on"}]}),
    )
    .await
    .assert_status_ok();

    let vread = server.get(path_of(&location)).await;
    vread.assert_status_ok();
    let vread: Value = vread.json();
    assert_eq!(vread["id"], id.as_str());
    assert_eq!(vread["meta"]["versionId"], "1");
    assert_eq!(vread["name"][0]["family"], "Location");
}

/// Every `Prefer: return=` keeps the header: the preference shapes the body,
/// not the headers (http.html#return).
#[tokio::test]
async fn create_location_is_versioned_under_every_return_preference() {
    let server = server();
    for preference in [
        "return=minimal",
        "return=representation",
        "return=OperationOutcome",
    ] {
        let created = server
            .post("/Patient")
            .add_header(CONTENT_TYPE, FHIR_JSON)
            .add_header(PREFER, HeaderValue::from_static(preference))
            .json(&json!({"resourceType": "Patient"}))
            .await;
        created.assert_status(StatusCode::CREATED);
        let location = location(&created);
        let id = path_of(&location)
            .strip_prefix("/Patient/")
            .and_then(|rest| rest.strip_suffix("/_history/1"))
            .unwrap_or_else(|| panic!("{preference}: {location}"));
        server
            .get(&format!("/Patient/{id}/_history/1"))
            .await
            .assert_status_ok();
    }
}

/// An update that creates answers 201 with the versioned Location; the update
/// after it answers 200 and, as before, no Location.
#[tokio::test]
async fn update_as_create_location_is_versioned() {
    let server = server();
    let created = put_patient(
        &server,
        "loc-upsert",
        json!({"resourceType": "Patient", "id": "loc-upsert"}),
    )
    .await;
    created.assert_status(StatusCode::CREATED);
    assert_eq!(
        location(&created),
        format!("{BASE}/Patient/loc-upsert/_history/1")
    );

    let updated = put_patient(
        &server,
        "loc-upsert",
        json!({"resourceType": "Patient", "id": "loc-upsert", "active": true}),
    )
    .await;
    updated.assert_status_ok();
    assert!(updated.headers().get("location").is_none());
}

/// The version in the Location is the stored one, not a constant `1`: a PUT
/// that brings a deleted resource back names the version it wrote.
#[tokio::test]
async fn update_as_create_after_delete_names_the_new_version() {
    let server = server();
    let body = json!({"resourceType": "Patient", "id": "loc-revived"});
    put_patient(&server, "loc-revived", body.clone())
        .await
        .assert_status(StatusCode::CREATED);
    server
        .delete("/Patient/loc-revived")
        .await
        .assert_status(StatusCode::NO_CONTENT);

    let revived = put_patient(&server, "loc-revived", body).await;
    revived.assert_status(StatusCode::CREATED);
    let version = etag_version(&revived);
    assert_ne!(version, "1");
    let location = location(&revived);
    assert_eq!(
        location,
        format!("{BASE}/Patient/loc-revived/_history/{version}")
    );
    server.get(path_of(&location)).await.assert_status_ok();
}

/// A conditional create answers the versioned Location when it creates, and
/// the match's versioned Location when it matches (R5 #ccreate).
#[tokio::test]
async fn conditional_create_locations_are_versioned() {
    let server = server();
    let body = json!({
        "resourceType": "Patient",
        "identifier": [{"system": "urn:example:mrn", "value": "loc-cc"}]
    });
    let send = || {
        server
            .post("/Patient")
            .add_header(CONTENT_TYPE, FHIR_JSON)
            .add_header(
                IF_NONE_EXIST,
                HeaderValue::from_static("identifier=urn:example:mrn|loc-cc"),
            )
            .json(&body)
    };

    let created = send().await;
    created.assert_status(StatusCode::CREATED);
    let id = created.json::<Value>()["id"].as_str().unwrap().to_string();
    let created_location = location(&created);
    assert_eq!(created_location, format!("{BASE}/Patient/{id}/_history/1"));

    let matched = send().await;
    matched.assert_status_ok();
    assert_eq!(matched.json::<Value>()["id"], id.as_str());
    assert_eq!(location(&matched), created_location);

    // No body is still the same headers.
    let minimal = send()
        .add_header(PREFER, HeaderValue::from_static("return=minimal"))
        .await;
    minimal.assert_status_ok();
    assert_eq!(location(&minimal), created_location);
}

/// A conditional update that creates answers the versioned Location.
#[tokio::test]
async fn conditional_update_create_location_is_versioned() {
    let server = server();
    let created = server
        .put("/Patient?identifier=urn:example:mrn|loc-cu")
        .add_header(CONTENT_TYPE, FHIR_JSON)
        .json(&json!({
            "resourceType": "Patient",
            "identifier": [{"system": "urn:example:mrn", "value": "loc-cu"}]
        }))
        .await;
    created.assert_status(StatusCode::CREATED);
    let id = created.json::<Value>()["id"].as_str().unwrap().to_string();
    assert_eq!(
        location(&created),
        format!("{BASE}/Patient/{id}/_history/1")
    );
}

/// `HFS_ENABLE_VERSIONING=false` drops only the `ETag`: HFS still keeps every
/// version, so the Location still names the created one, and a GET on it is a
/// vread of that version.
#[tokio::test]
async fn versioning_disabled_still_sends_the_versioned_location() {
    let server = server_with(ServerConfig {
        enable_versioning: false,
        ..ServerConfig::for_testing()
    });
    let created = post_patient(&server, json!({"resourceType": "Patient"})).await;
    created.assert_status(StatusCode::CREATED);
    assert!(created.headers().get("etag").is_none());
    let id = created.json::<Value>()["id"].as_str().unwrap().to_string();
    let created_location = location(&created);
    assert_eq!(created_location, format!("{BASE}/Patient/{id}/_history/1"));

    let upserted = put_patient(
        &server,
        "loc-unflagged",
        json!({"resourceType": "Patient", "id": "loc-unflagged"}),
    )
    .await;
    upserted.assert_status(StatusCode::CREATED);
    let upserted_location = location(&upserted);
    assert_eq!(
        upserted_location,
        format!("{BASE}/Patient/loc-unflagged/_history/1")
    );

    for (location, id) in [
        (created_location, id.as_str()),
        (upserted_location, "loc-unflagged"),
    ] {
        let vread = server.get(path_of(&location)).await;
        vread.assert_status_ok();
        let body = vread.json::<Value>();
        assert_eq!(body["id"], id);
        assert_eq!(body["meta"]["versionId"], "1");
    }
}

/// A header-routed tenant leaves the public base alone; the Location is
/// versioned under it. (The URL-path tenant case is in
/// `public_url_contract.rs`.)
#[tokio::test]
async fn header_tenant_location_is_versioned_under_the_unprefixed_base() {
    let server = server_with(ServerConfig {
        default_tenant: "default".to_string(),
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::HeaderOnly,
            ..Default::default()
        },
        ..ServerConfig::for_testing()
    });
    let created = server
        .post("/Patient")
        .add_header(CONTENT_TYPE, FHIR_JSON)
        .add_header(X_TENANT_ID, HeaderValue::from_static("acme"))
        .json(&json!({"resourceType": "Patient"}))
        .await;
    created.assert_status(StatusCode::CREATED);
    let id = created.json::<Value>()["id"].as_str().unwrap().to_string();
    let location = location(&created);
    assert_eq!(location, format!("{BASE}/Patient/{id}/_history/1"));
    server
        .get(path_of(&location))
        .add_header(X_TENANT_ID, HeaderValue::from_static("acme"))
        .await
        .assert_status_ok();
}

/// Every batch write that names one resource version answers
/// `[type]/[id]/_history/[vid]`: create, update, patch and conditional update
/// — and each one vreads.
#[tokio::test]
async fn batch_write_locations_are_versioned() {
    let server = server();
    for id in ["loc-b-update", "loc-b-patch"] {
        put_patient(
            &server,
            id,
            json!({"resourceType": "Patient", "id": id, "active": false}),
        )
        .await
        .assert_status(StatusCode::CREATED);
    }
    post_patient(
        &server,
        json!({
            "resourceType": "Patient",
            "identifier": [{"system": "urn:example:mrn", "value": "loc-b-cond"}]
        }),
    )
    .await
    .assert_status(StatusCode::CREATED);

    let bundle = json!({
        "resourceType": "Bundle",
        "type": "batch",
        "entry": [
            {
                "request": {"method": "POST", "url": "Patient"},
                "resource": {"resourceType": "Patient"}
            },
            {
                "request": {"method": "PUT", "url": "Patient/loc-b-update"},
                "resource": {"resourceType": "Patient", "id": "loc-b-update", "active": true}
            },
            {
                "request": {"method": "PATCH", "url": "Patient/loc-b-patch"},
                "resource": {"resourceType": "Parameters", "parameter": [{
                    "name": "operation",
                    "part": [
                        {"name": "type", "valueCode": "replace"},
                        {"name": "path", "valueString": "Patient.active"},
                        {"name": "value", "valueBoolean": true}
                    ]
                }]}
            },
            {
                "request": {"method": "PUT", "url": "Patient?identifier=urn:example:mrn|loc-b-cond"},
                "resource": {
                    "resourceType": "Patient",
                    "identifier": [{"system": "urn:example:mrn", "value": "loc-b-cond"}],
                    "active": true
                }
            },
            {
                "request": {"method": "PUT", "url": "Patient/loc-b-upsert"},
                "resource": {"resourceType": "Patient", "id": "loc-b-upsert"}
            }
        ]
    });
    let response = server
        .post("/")
        .add_header(CONTENT_TYPE, FHIR_JSON)
        .json(&bundle)
        .await;
    response.assert_status_ok();
    let response: Value = response.json();
    let entries = response["entry"].as_array().unwrap();
    assert_eq!(entries.len(), 5, "{response}");

    let expected = [
        ("201 Created", None, "1"),
        ("200 OK", Some("loc-b-update"), "2"),
        ("200 OK", Some("loc-b-patch"), "2"),
        ("200 OK", None, "2"),
        ("201 Created", Some("loc-b-upsert"), "1"),
    ];
    for (entry, (status, id, version)) in entries.iter().zip(expected) {
        assert_eq!(entry["response"]["status"], status, "{entry}");
        let id = id
            .map(str::to_string)
            .unwrap_or_else(|| entry["resource"]["id"].as_str().unwrap().to_string());
        let location = format!("Patient/{id}/_history/{version}");
        assert_eq!(entry["response"]["location"], location.as_str(), "{entry}");
        assert_eq!(
            entry["fullUrl"],
            format!("{BASE}/Patient/{id}").as_str(),
            "fullUrl stays unversioned: {entry}"
        );
        server.get(&format!("/{location}")).await.assert_status_ok();
    }
}

/// `Both` routing: a URL-path tenant is part of `[base]`, a header tenant is
/// not; either way the Location is versioned and vreads.
#[tokio::test]
async fn both_routing_location_follows_the_request_tenant_form() {
    let server = server_with(ServerConfig {
        default_tenant: "default".to_string(),
        multitenancy: MultitenancyConfig {
            routing_mode: TenantRoutingMode::Both,
            ..Default::default()
        },
        ..ServerConfig::for_testing()
    });

    let by_path = server
        .post("/acme/Patient")
        .add_header(CONTENT_TYPE, FHIR_JSON)
        .json(&json!({"resourceType": "Patient"}))
        .await;
    by_path.assert_status(StatusCode::CREATED);
    let id = by_path.json::<Value>()["id"].as_str().unwrap().to_string();
    let location = location(&by_path);
    assert_eq!(location, format!("{BASE}/acme/Patient/{id}/_history/1"));
    server.get(path_of(&location)).await.assert_status_ok();

    let by_header = server
        .post("/Patient")
        .add_header(CONTENT_TYPE, FHIR_JSON)
        .add_header(X_TENANT_ID, HeaderValue::from_static("acme"))
        .json(&json!({"resourceType": "Patient"}))
        .await;
    by_header.assert_status(StatusCode::CREATED);
    let id = by_header.json::<Value>()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        self::location(&by_header),
        format!("{BASE}/Patient/{id}/_history/1")
    );
}
