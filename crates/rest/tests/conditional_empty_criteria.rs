//! #1360: a conditional criterion with an empty value must be refused, not
//! dropped.
//!
//! `identifier=` is what an unset template variable renders as
//! (`identifier={{mrn}}`). The shared criteria builder used to drop such a
//! pair, so the *remaining* criteria decided the match:
//!
//! * `PUT /Patient?identifier=&family=Jones` was evaluated as `family=Jones`
//!   and overwrote the one Jones on file, whatever their identifier;
//! * `DELETE` with the same criteria deleted them;
//! * `If-None-Exist: identifier=&family=Jones` answered "already exists";
//! * `identifier=` alone left no criteria at all, so a conditional create or
//!   update created unguarded.
//!
//! Direct search *does* ignore the parameter: `GET /Patient?identifier=` is
//! the search without it, as FHIR says of an empty parameter (#1380; asserted
//! below). A read can afford that; the precondition of a write cannot mean
//! "whatever the other criteria say" (wrong target), so it is a `400` naming
//! the parameter — whatever `Prefer: handling` says. An empty *alternative*
//! (`family=Jones,`) is a `400` in a search as well.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const IF_NONE_EXIST: HeaderName = HeaderName::from_static("if-none-exist");
const PREFER: HeaderName = HeaderName::from_static("prefer");

fn tenant() -> HeaderValue {
    HeaderValue::from_static("test-tenant")
}

/// A server over in-memory SQLite with the spec search parameters loaded.
/// Without the data dir only five embedded parameters exist and `identifier`
/// would be refused as unknown — the tests would pass for the wrong reason.
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

fn patient(family: &str, identifier: &str) -> Value {
    json!({
        "resourceType": "Patient",
        "name": [{"family": family}],
        "identifier": [{"system": "http://example.org/mrn", "value": identifier}]
    })
}

const SEEDED: [(&str, &str); 3] = [
    ("intended", "Smith"),
    ("jones", "Jones"),
    ("other", "Smith"),
];

/// Two Smiths, only one of them (`intended`) carrying the identifier the
/// client means, and a lone Jones: the resource the *remaining* criterion of
/// `identifier=&family=Jones` selects once the empty one is dropped.
async fn seed(server: &TestServer) {
    for (id, family, identifier) in [
        ("intended", "Smith", "mrn-1"),
        ("other", "Smith", "mrn-2"),
        ("jones", "Jones", "mrn-3"),
    ] {
        let mut body = patient(family, identifier);
        body["id"] = json!(id);
        server
            .put(&format!("/Patient/{id}"))
            .add_header(X_TENANT_ID, tenant())
            .json(&body)
            .await
            .assert_status(StatusCode::CREATED);
    }
}

/// `id -> family` for every Patient on the server.
async fn families(server: &TestServer) -> Vec<(String, String)> {
    let bundle: Value = server
        .get("/Patient?_count=100")
        .add_header(X_TENANT_ID, tenant())
        .await
        .json();
    let mut out: Vec<(String, String)> = bundle["entry"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|e| {
                    (
                        e["resource"]["id"].as_str().unwrap_or("?").to_string(),
                        e["resource"]["name"][0]["family"]
                            .as_str()
                            .unwrap_or("?")
                            .to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected
        .iter()
        .map(|(id, family)| (id.to_string(), family.to_string()))
        .collect()
}

async fn search_ids(server: &TestServer, query: &str) -> Vec<String> {
    let response = server
        .get(&format!("/Patient?{query}"))
        .add_header(X_TENANT_ID, tenant())
        .await;
    response.assert_status_ok();
    let bundle: Value = response.json();
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

/// A 400 whose OperationOutcome names the offending parameter.
fn assert_rejected_naming(response: &axum_test::TestResponse, param: &str, context: &str) {
    assert_eq!(
        response.status_code(),
        StatusCode::BAD_REQUEST,
        "{context}: {}",
        response.text()
    );
    let outcome: Value = response.json();
    assert_eq!(outcome["resourceType"], "OperationOutcome", "{context}");
    assert!(
        outcome.to_string().contains(&format!("'{param}'")),
        "{context}: the outcome must name '{param}': {outcome}"
    );
}

/// `(criteria, the parameter the outcome must name)`.
///
/// A bare name with no `=` is the same thing to a form parser as `name=`, and
/// is treated the same. A value of nothing but whitespace or OR-separators is
/// empty too, and so is a single empty alternative: `family=Jones,` would ask
/// a string search for the prefix `""`, which every family name has.
const EMPTY_VALUE_CRITERIA: [(&str, &str); 10] = [
    ("identifier=&family=Jones", "identifier"),
    ("family=Jones&identifier=", "identifier"),
    ("identifier=", "identifier"),
    ("identifier", "identifier"),
    ("identifier&family=Jones", "identifier"),
    ("identifier=+&family=Jones", "identifier"),
    ("identifier=%20", "identifier"),
    ("identifier=,&family=Jones", "identifier"),
    ("family=Jones,&identifier=mrn-3", "family"),
    ("identifier:missing=&family=Jones", "identifier:missing"),
];

/// Positive control, and the difference from direct search stated on the
/// record: a search ignores a parameter with no value (#1380) — which finds
/// Jones, and is exactly what the criteria of a write must not do.
#[tokio::test]
async fn direct_search_ignores_a_parameter_with_no_value() {
    let server = test_server().await;
    seed(&server).await;

    assert_eq!(search_ids(&server, "identifier=mrn-1").await, ["intended"]);
    assert_eq!(search_ids(&server, "family=Jones").await, ["jones"]);
    assert_eq!(
        search_ids(&server, "family=Smith").await,
        ["intended", "other"]
    );

    assert_eq!(
        search_ids(&server, "identifier=").await,
        ["intended", "jones", "other"]
    );
    assert_eq!(
        search_ids(&server, "identifier=&family=Jones").await,
        ["jones"]
    );

    // An empty alternative is malformed, in a search too.
    let response = server
        .get("/Patient?family=Jones,")
        .add_header(X_TENANT_ID, tenant())
        .await;
    assert_rejected_naming(&response, "family", "GET /Patient?family=Jones,");
}

#[tokio::test]
async fn an_empty_criterion_value_is_rejected_and_nothing_is_written() {
    for (criteria, param) in EMPTY_VALUE_CRITERIA {
        for prefer in [None, Some("handling=lenient"), Some("handling=strict")] {
            let server = test_server().await;
            seed(&server).await;
            let context = format!("{criteria} (Prefer: {prefer:?})");
            let with_prefer = |request: axum_test::TestRequest| match prefer {
                Some(value) => request.add_header(PREFER, HeaderValue::from_static(value)),
                None => request,
            };

            let response = with_prefer(
                server
                    .post("/Patient")
                    .add_header(X_TENANT_ID, tenant())
                    .add_header(
                        IF_NONE_EXIST,
                        HeaderValue::from_str(criteria).expect("header value"),
                    )
                    .json(&patient("Incoming", "mrn-1")),
            )
            .await;
            assert_rejected_naming(&response, param, &format!("If-None-Exist: {context}"));

            let response = with_prefer(
                server
                    .put(&format!("/Patient?{criteria}"))
                    .add_header(X_TENANT_ID, tenant())
                    .json(&patient("Updated", "mrn-1")),
            )
            .await;
            assert_rejected_naming(&response, param, &format!("PUT {context}"));

            let response = with_prefer(
                server
                    .delete(&format!("/Patient?{criteria}"))
                    .add_header(X_TENANT_ID, tenant()),
            )
            .await;
            assert_rejected_naming(&response, param, &format!("DELETE {context}"));

            assert_eq!(families(&server).await, pairs(&SEEDED), "{context}");
        }
    }
}

/// In a batch the entry fails and its neighbour proceeds; a transaction fails
/// as a whole and writes nothing, the neighbour included.
#[tokio::test]
async fn an_empty_criterion_value_fails_the_bundle_entry() {
    for (criteria, param) in EMPTY_VALUE_CRITERIA {
        let conditional_entries = [
            json!({
                "resource": patient("Incoming", "mrn-1"),
                "request": {"method": "POST", "url": "Patient", "ifNoneExist": criteria}
            }),
            json!({
                "resource": patient("Updated", "mrn-1"),
                "request": {"method": "PUT", "url": format!("Patient?{criteria}")}
            }),
            json!({"request": {"method": "DELETE", "url": format!("Patient?{criteria}")}}),
        ];
        let neighbour = json!({
            "resource": patient("Neighbour", "nb-1"),
            "request": {"method": "POST", "url": "Patient"}
        });

        for entry in &conditional_entries {
            let server = test_server().await;
            seed(&server).await;
            let response = server
                .post("/")
                .add_header(X_TENANT_ID, tenant())
                .json(&json!({
                    "resourceType": "Bundle",
                    "type": "batch",
                    "entry": [entry, neighbour]
                }))
                .await;
            response.assert_status_ok();
            let reply: Value = response.json();
            let entry_response = &reply["entry"][0]["response"];
            assert!(
                entry_response["status"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("400"),
                "batch {entry}: {entry_response}"
            );
            assert!(
                entry_response["outcome"]
                    .to_string()
                    .contains(&format!("'{param}'")),
                "batch {entry}: the outcome must name '{param}': {entry_response}"
            );
            assert!(
                reply["entry"][1]["response"]["status"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("201"),
                "the neighbouring entry must proceed: {reply}"
            );
            let after: Vec<(String, String)> = families(&server)
                .await
                .into_iter()
                .filter(|(_, family)| family != "Neighbour")
                .collect();
            assert_eq!(after, pairs(&SEEDED), "batch {entry}");
        }

        // Transaction: only `ifNoneExist` reaches the criteria builder (a
        // query-bearing `request.url` is refused outright).
        let server = test_server().await;
        seed(&server).await;
        let response = server
            .post("/")
            .add_header(X_TENANT_ID, tenant())
            .json(&json!({
                "resourceType": "Bundle",
                "type": "transaction",
                "entry": [neighbour, conditional_entries[0]]
            }))
            .await;
        assert_rejected_naming(&response, param, &format!("transaction {criteria}"));
        assert_eq!(
            families(&server).await,
            pairs(&SEEDED),
            "transaction {criteria}: the whole Bundle must roll back"
        );
    }
}

/// What must keep working: `:missing` has a value, and a result parameter is no
/// criterion, with or without one.
#[tokio::test]
async fn missing_and_empty_result_parameters_are_unaffected() {
    for criteria in [
        "identifier:missing=false&family=Jones",
        "_format=&identifier=mrn-3",
        "_format&identifier=mrn-3",
        "_pretty=&_summary=&family=Jones",
    ] {
        let server = test_server().await;
        seed(&server).await;

        let response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, tenant())
            .add_header(
                IF_NONE_EXIST,
                HeaderValue::from_str(criteria).expect("header value"),
            )
            .json(&patient("Incoming", "mrn-3"))
            .await;
        assert_eq!(response.status_code(), StatusCode::OK, "{criteria}");

        server
            .put(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .json(&patient("Updated", "mrn-3"))
            .await
            .assert_status(StatusCode::OK);
        assert_eq!(
            families(&server).await,
            pairs(&[
                ("intended", "Smith"),
                ("jones", "Updated"),
                ("other", "Smith")
            ]),
            "{criteria}"
        );
    }

    // `:missing=true` selects the one patient without an identifier.
    let server = test_server().await;
    seed(&server).await;
    server
        .put("/Patient/bare")
        .add_header(X_TENANT_ID, tenant())
        .json(&json!({"resourceType": "Patient", "id": "bare", "name": [{"family": "Bare"}]}))
        .await
        .assert_status(StatusCode::CREATED);
    server
        .delete("/Patient?identifier:missing=true")
        .add_header(X_TENANT_ID, tenant())
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_eq!(families(&server).await, pairs(&SEEDED));

    // An empty result parameter on its own still selects nothing.
    server
        .delete("/Patient?_format=")
        .add_header(X_TENANT_ID, tenant())
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_eq!(families(&server).await, pairs(&SEEDED));
}
