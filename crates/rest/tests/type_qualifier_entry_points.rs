//! #1366: every REST entry point that builds a search holds a `:[type]`
//! qualifier to one rule — a resource type of the FHIR version the search runs
//! in, case-sensitively, refused in one form of words.
//!
//! The search handlers got the version-aware rule in #1339. This file covers
//! the entry points that still built their queries without a version (a
//! transaction's up-front check of its `GET` search entries, conditional
//! references, compartment search in both forms; `$export?_typeFilter=` is in
//! `bulk_export.rs`) and conditional criteria, which carried a second copy of
//! the rule in `helios-persistence`.
//!
//! The default test build is R4 only, where a type of another FHIR version is
//! unknown whichever version is asked, so what is observable here is that each
//! path refuses a type that is none with a `400` naming the version, accepts a
//! real one, and that the texts agree. Telling versions apart needs a
//! multi-version build: the last test here is gated on `all(R4, R5)` (run with
//! `--features R4,R4B,R5,R6`), as are unit tests of `search_query_builder`, of
//! `helios_persistence::search::type_qualifier` and of
//! `helios_persistence::search::conditional`.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::{TestResponse, TestServer};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const IF_NONE_EXIST: HeaderName = HeaderName::from_static("if-none-exist");

/// What every path says about `:Bogus`, whatever it wraps around it.
const NOT_A_TYPE: &str = "it is neither a search modifier nor a resource type of FHIR R4";

fn tenant() -> HeaderValue {
    HeaderValue::from_static("test-tenant")
}

/// A server over in-memory SQLite with the spec search parameters loaded:
/// without the data dir `subject` and `general-practitioner` are not
/// registered and nothing below would be typed as a reference.
///
/// Seeds `Practitioner/dr`, `Patient/p1` (whose GP is `dr`) and
/// `Observation/o1` about `p1`.
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
    let server = TestServer::new(app).expect("test server");

    for (url, body) in [
        (
            "/Practitioner/dr",
            json!({"resourceType": "Practitioner", "id": "dr"}),
        ),
        (
            "/Patient/p1",
            json!({
                "resourceType": "Patient",
                "id": "p1",
                "name": [{"family": "Neal"}],
                "generalPractitioner": [{"reference": "Practitioner/dr"}]
            }),
        ),
        (
            "/Observation/o1",
            json!({
                "resourceType": "Observation",
                "id": "o1",
                "status": "final",
                "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
                "subject": {"reference": "Patient/p1"}
            }),
        ),
    ] {
        server
            .put(url)
            .add_header(X_TENANT_ID, tenant())
            .json(&body)
            .await
            .assert_status(StatusCode::CREATED);
    }
    server
}

async fn get(server: &TestServer, url: &str) -> TestResponse {
    server.get(url).add_header(X_TENANT_ID, tenant()).await
}

fn ids(bundle: &Value) -> Vec<String> {
    let mut ids: Vec<String> = bundle["entry"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| e["resource"]["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

/// The whole response body as text: the paths put the message in different
/// places (`details.text`, `diagnostics`, behind an entry prefix).
fn outcome_text(response: &TestResponse) -> String {
    response.text()
}

async fn post_bundle(server: &TestServer, bundle: &Value) -> TestResponse {
    server
        .post("/")
        .add_header(X_TENANT_ID, tenant())
        .json(bundle)
        .await
}

fn bundle(kind: &str, entries: Value) -> Value {
    json!({"resourceType": "Bundle", "type": kind, "entry": entries})
}

/// The reference point: what direct search says (#1339).
#[tokio::test]
async fn direct_search_is_the_reference() {
    let server = test_server().await;

    let ok = get(&server, "/Observation?subject:Patient=p1").await;
    ok.assert_status_ok();
    assert_eq!(ids(&ok.json()), ["o1"]);

    let refused = get(&server, "/Observation?subject:Bogus=p1").await;
    refused.assert_status(StatusCode::BAD_REQUEST);
    let text = outcome_text(&refused);
    assert!(
        text.contains("unknown search modifier ':Bogus' on parameter 'subject'"),
        "{text}"
    );
    assert!(text.contains(NOT_A_TYPE), "{text}");
}

#[tokio::test]
async fn typed_compartment_search() {
    let server = test_server().await;

    let ok = get(&server, "/Patient/p1/Observation?subject:Patient=p1").await;
    ok.assert_status_ok();
    assert_eq!(ids(&ok.json()), ["o1"]);

    for (qualifier, expected) in [
        ("Bogus", NOT_A_TYPE),
        ("patient", "case-sensitive: ':Patient'?"),
    ] {
        let refused = get(
            &server,
            &format!("/Patient/p1/Observation?subject:{qualifier}=p1"),
        )
        .await;
        refused.assert_status(StatusCode::BAD_REQUEST);
        let text = outcome_text(&refused);
        assert!(text.contains(expected), "{qualifier}: {text}");
    }
}

/// `GET /Patient/p1/*` builds one query per member type and skips the types a
/// parameter does not apply to. A qualifier that is no resource type fails for
/// every one of them; that used to leave nothing to search and answer an empty
/// `200` where the typed form answers `400`.
#[tokio::test]
async fn all_types_compartment_search() {
    let server = test_server().await;

    // Positive controls: the unfiltered compartment, and a real qualifier.
    let all = get(&server, "/Patient/p1/*").await;
    all.assert_status_ok();
    assert!(ids(&all.json()).contains(&"o1".to_string()));
    let ok = get(&server, "/Patient/p1/*?subject:Patient=p1").await;
    ok.assert_status_ok();
    assert!(ids(&ok.json()).contains(&"o1".to_string()));

    let refused = get(&server, "/Patient/p1/*?subject:Bogus=p1").await;
    refused.assert_status(StatusCode::BAD_REQUEST);
    let text = outcome_text(&refused);
    assert!(text.contains(NOT_A_TYPE), "{text}");

    // A filter only some member types know still answers from those.
    let skipped = get(&server, "/Patient/p1/*?code=8867-4").await;
    skipped.assert_status_ok();
    assert_eq!(ids(&skipped.json()), ["o1"]);
}

/// A transaction validates its `GET` search entries before anything executes,
/// so a malformed one rejects the whole Bundle.
#[tokio::test]
async fn transaction_search_entry() {
    let server = test_server().await;
    let with_search = |url: &str| {
        bundle(
            "transaction",
            json!([
                {
                    "fullUrl": "urn:uuid:4b7a8c1e-0000-4000-8000-000000000001",
                    "resource": {"resourceType": "Patient", "name": [{"family": "Written"}]},
                    "request": {"method": "POST", "url": "Patient"}
                },
                {"request": {"method": "GET", "url": url}}
            ]),
        )
    };

    let refused = post_bundle(&server, &with_search("Observation?subject:Bogus=p1")).await;
    refused.assert_status(StatusCode::BAD_REQUEST);
    let text = outcome_text(&refused);
    assert!(text.contains(NOT_A_TYPE), "{text}");
    let written = get(&server, "/Patient?family=Written").await;
    assert!(ids(&written.json()).is_empty(), "nothing may be written");

    let ok = post_bundle(&server, &with_search("Observation?subject:Patient=p1")).await;
    ok.assert_status_ok();
    let response: Value = ok.json();
    assert_eq!(ids(&response["entry"][1]["resource"]), ["o1"]);
}

/// A batch runs a `GET` search entry through the search handler's own path;
/// the entry fails alone.
#[tokio::test]
async fn batch_search_entry() {
    let server = test_server().await;
    let response = post_bundle(
        &server,
        &bundle(
            "batch",
            json!([
                {"request": {"method": "GET", "url": "Observation?subject:Bogus=p1"}},
                {"request": {"method": "GET", "url": "Observation?subject:Patient=p1"}}
            ]),
        ),
    )
    .await;
    response.assert_status_ok();
    let body: Value = response.json();
    let refused = &body["entry"][0]["response"];
    assert!(
        refused["status"].as_str().unwrap_or("").starts_with("400"),
        "{refused}"
    );
    assert!(refused.to_string().contains(NOT_A_TYPE), "{refused}");
    assert_eq!(ids(&body["entry"][1]["resource"]), ["o1"]);
}

/// A conditional reference (`Patient?…` inside a resource body) is resolved
/// with a search built by the same builder.
#[tokio::test]
async fn transaction_conditional_reference() {
    let server = test_server().await;
    let observing = |reference: &str| {
        bundle(
            "transaction",
            json!([{
                "fullUrl": "urn:uuid:4b7a8c1e-0000-4000-8000-000000000002",
                "resource": {
                    "resourceType": "Observation",
                    "status": "final",
                    "code": {"coding": [{"system": "http://loinc.org", "code": "9279-1"}]},
                    "subject": {"reference": reference}
                },
                "request": {"method": "POST", "url": "Observation"}
            }]),
        )
    };

    let refused = post_bundle(&server, &observing("Patient?general-practitioner:Bogus=dr")).await;
    refused.assert_status(StatusCode::BAD_REQUEST);
    let text = outcome_text(&refused);
    assert!(text.contains(NOT_A_TYPE), "{text}");
    let written = get(&server, "/Observation?code=9279-1").await;
    assert!(ids(&written.json()).is_empty(), "nothing may be written");

    let ok = post_bundle(
        &server,
        &observing("Patient?general-practitioner:Practitioner=dr"),
    )
    .await;
    ok.assert_status_ok();
    let stored = get(&server, "/Observation?code=9279-1")
        .await
        .json::<Value>();
    assert_eq!(
        stored["entry"][0]["resource"]["subject"]["reference"],
        "Patient/p1"
    );
}

/// Conditional criteria are parsed in `helios-persistence`, which used to
/// carry its own copy of the rule with its own wording and no version. They
/// now say exactly what the same string says as a direct search.
#[tokio::test]
async fn conditional_criteria_say_what_direct_search_says() {
    let server = test_server().await;
    let new_patient = json!({"resourceType": "Patient", "name": [{"family": "New"}]});

    for qualifier in ["Bogus", "practitioner"] {
        let criteria = format!("general-practitioner:{qualifier}=dr");
        let direct = get(&server, &format!("/Patient?{criteria}")).await;
        direct.assert_status(StatusCode::BAD_REQUEST);
        // The sentence the shared rule writes, from the modifier to its end.
        let direct_text = outcome_text(&direct);
        let start = direct_text
            .find("unknown search modifier")
            .expect("direct search names the modifier");
        let sentence: String = direct_text[start..]
            .chars()
            .take_while(|c| *c != '"')
            .collect();
        // Direct search's wrapper ends the sentence; the rule's text does not.
        let sentence = sentence.trim_end_matches('.').to_string();
        assert!(sentence.contains("of FHIR R4"), "{sentence}");

        let created = server
            .post("/Patient")
            .add_header(X_TENANT_ID, tenant())
            .add_header(IF_NONE_EXIST, HeaderValue::from_str(&criteria).unwrap())
            .json(&new_patient)
            .await;
        let updated = server
            .put(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .json(&new_patient)
            .await;
        let deleted = server
            .delete(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .await;
        for (what, response) in [
            ("If-None-Exist", created),
            ("PUT", updated),
            ("DELETE", deleted),
        ] {
            response.assert_status(StatusCode::BAD_REQUEST);
            let text = outcome_text(&response);
            assert!(
                text.contains(&sentence),
                "{what} {criteria}: expected {sentence:?} in {text}"
            );
        }
    }
    assert!(
        ids(&get(&server, "/Patient?family=New").await.json()).is_empty(),
        "nothing may be written"
    );

    // A real qualifier still matches: the conditional create finds p1.
    let matched = server
        .post("/Patient")
        .add_header(X_TENANT_ID, tenant())
        .add_header(
            IF_NONE_EXIST,
            HeaderValue::from_static("general-practitioner:Practitioner=dr"),
        )
        .json(&new_patient)
        .await;
    matched.assert_status_ok();
    assert!(
        ids(&get(&server, "/Patient?family=New").await.json()).is_empty(),
        "the criteria matched p1, so nothing is created"
    );
}

/// The issue itself, which only a multi-version build can show: on a server
/// whose searches run in R4, a type only R5 has (`ActorDefinition`) is not a
/// resource type on any path. Before #1366 every path below except direct
/// search accepted it. Run with `--features R4,R4B,R5,R6`.
#[cfg(all(feature = "R4", feature = "R5"))]
#[tokio::test]
async fn a_type_of_another_enabled_version_is_refused_on_every_path() {
    let server = test_server().await;
    let mut accepted: Vec<String> = Vec::new();
    let mut check = |what: &str, response: TestResponse| {
        let text = outcome_text(&response);
        if response.status_code() != StatusCode::BAD_REQUEST || !text.contains(NOT_A_TYPE) {
            accepted.push(format!("{what}: {} {text}", response.status_code()));
        }
    };

    for url in [
        "/Observation?subject:ActorDefinition=a1",
        "/Patient/p1/Observation?subject:ActorDefinition=a1",
        "/Patient/p1/*?subject:ActorDefinition=a1",
    ] {
        check(url, get(&server, url).await);
    }

    let search_entry = bundle(
        "transaction",
        json!([{"request": {"method": "GET", "url": "Observation?subject:ActorDefinition=a1"}}]),
    );
    // An entry failure after the up-front check is a `200` Bundle; only the
    // up-front check makes it a `400`.
    check(
        "transaction GET search entry",
        post_bundle(&server, &search_entry).await,
    );
    let conditional_reference = bundle(
        "transaction",
        json!([{
            "fullUrl": "urn:uuid:4b7a8c1e-0000-4000-8000-000000000003",
            "resource": {
                "resourceType": "Observation",
                "status": "final",
                "code": {"coding": [{"system": "http://loinc.org", "code": "9279-1"}]},
                "subject": {"reference": "Patient?general-practitioner:ActorDefinition=a1"}
            },
            "request": {"method": "POST", "url": "Observation"}
        }]),
    );
    check(
        "conditional reference",
        post_bundle(&server, &conditional_reference).await,
    );

    let criteria = "general-practitioner:ActorDefinition=a1";
    check(
        "If-None-Exist",
        server
            .post("/Patient")
            .add_header(X_TENANT_ID, tenant())
            .add_header(IF_NONE_EXIST, HeaderValue::from_static(criteria))
            .json(&json!({"resourceType": "Patient", "name": [{"family": "New"}]}))
            .await,
    );
    check(
        "conditional DELETE",
        server
            .delete(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .await,
    );

    assert!(
        accepted.is_empty(),
        "paths that did not refuse an R5-only type on an R4 server:\n{}",
        accepted.join("\n")
    );
}
