//! #866: a registered `_` parameter no backend evaluates must be refused in
//! conditional criteria, never dropped.
//!
//! `_query` is a spec parameter (`Resource-query`, type token, no expression),
//! and the loader keeps expressionless `_` parameters, so the criteria builder
//! used to accept it. SQLite routed it to its special-parameter builder, whose
//! fallback arm dropped the condition, and a query with no conditions matches
//! every indexed resource of the type. Over HTTP, against one stored Patient:
//!
//! * `DELETE /Patient?_query=x` answered `204` and deleted it;
//! * `PUT /Patient?_query=x` answered `200` and overwrote it;
//! * the same URLs as batch and transaction entries did the same, and so did
//!   `If-None-Exist: _query=x` ("already exists" for a Patient that has
//!   nothing to do with the criterion).
//!
//! Direct search already answers `_query` with a `400` (named queries are not
//! implemented); the criteria of a *write* must not be more permissive than
//! that. Every case below keeps the one stored Patient to prove nothing was
//! written, and a positive control shows that the `_` parameters that are
//! evaluated (`_id`, `_tag`, `_lastUpdated`, …) still select their target.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const IF_NONE_EXIST: HeaderName = HeaderName::from_static("if-none-exist");
const JSON_PATCH: &str = "application/json-patch+json";

fn tenant() -> HeaderValue {
    HeaderValue::from_static("test-tenant")
}

/// A server over in-memory SQLite with the spec search parameters loaded.
/// Without the data dir only a handful of embedded parameters exist and
/// `_query` would be refused as unknown — the tests would pass for the wrong
/// reason.
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

fn patient(family: &str) -> Value {
    json!({
        "resourceType": "Patient",
        "name": [{"family": family}],
        "meta": {"tag": [{"system": "http://example.org/tags", "code": "gold"}]}
    })
}

/// Exactly one Patient on file — the shape of the reproduction: a dropped
/// condition makes it the match, whatever the criteria say.
async fn seed(server: &TestServer) {
    let mut body = patient("Smith");
    body["id"] = json!("only");
    server
        .put("/Patient/only")
        .add_header(X_TENANT_ID, tenant())
        .json(&body)
        .await
        .assert_status(StatusCode::CREATED);
}

/// `(id, family)` of every Patient on the server.
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

fn untouched() -> Vec<(String, String)> {
    vec![("only".to_string(), "Smith".to_string())]
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
/// `_query` is registered on every FHIR version; `:not` is a valid modifier on
/// a token, so `_query:not=x` is not stopped by modifier validation and must be
/// refused for the parameter itself. Another, evaluated criterion beside it
/// must not rescue it: the dropped condition would otherwise leave the rest to
/// decide the match.
const REFUSED_CRITERIA: [(&str, &str); 4] = [
    ("_query=x", "_query"),
    ("_query=byName", "_query"),
    ("_query:not=x", "_query"),
    ("family=Smith&_query=x", "_query"),
];

/// Positive control, and the difference from direct search stated on the
/// record: the stored Patient is visible, and direct search refuses `_query` too.
#[tokio::test]
async fn direct_search_refuses_query_and_sees_the_patient() {
    let server = test_server().await;
    seed(&server).await;
    assert_eq!(families(&server).await, untouched());

    server
        .get("/Patient?_query=x")
        .add_header(X_TENANT_ID, tenant())
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn query_in_conditional_criteria_is_rejected_and_nothing_is_written() {
    for (criteria, param) in REFUSED_CRITERIA {
        let server = test_server().await;
        seed(&server).await;

        let response = server
            .delete(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .await;
        assert_rejected_naming(&response, param, &format!("DELETE {criteria}"));
        assert_eq!(families(&server).await, untouched(), "DELETE {criteria}");

        let response = server
            .put(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .json(&patient("Overwritten"))
            .await;
        assert_rejected_naming(&response, param, &format!("PUT {criteria}"));
        assert_eq!(families(&server).await, untouched(), "PUT {criteria}");

        let response = server
            .patch(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .add_header(header::CONTENT_TYPE, HeaderValue::from_static(JSON_PATCH))
            .bytes(
                json!([{"op": "replace", "path": "/name/0/family", "value": "Patched"}])
                    .to_string()
                    .into(),
            )
            .await;
        assert_rejected_naming(&response, param, &format!("PATCH {criteria}"));
        assert_eq!(families(&server).await, untouched(), "PATCH {criteria}");

        // `If-None-Exist` would answer "already exists" for the Patient that
        // the dropped condition matched, and write nothing.
        let response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, tenant())
            .add_header(
                IF_NONE_EXIST,
                HeaderValue::from_str(criteria).expect("header value"),
            )
            .json(&patient("Incoming"))
            .await;
        assert_rejected_naming(&response, param, &format!("If-None-Exist: {criteria}"));
        assert_eq!(
            families(&server).await,
            untouched(),
            "If-None-Exist: {criteria}"
        );
    }
}

/// In a batch the entry fails and its neighbour proceeds.
#[tokio::test]
async fn query_in_batch_entry_criteria_fails_the_entry() {
    for (criteria, param) in REFUSED_CRITERIA {
        let conditional_entries = [
            json!({"request": {"method": "DELETE", "url": format!("Patient?{criteria}")}}),
            json!({
                "resource": patient("Overwritten"),
                "request": {"method": "PUT", "url": format!("Patient?{criteria}")}
            }),
            json!({
                "resource": patient("Incoming"),
                "request": {"method": "POST", "url": "Patient", "ifNoneExist": criteria}
            }),
        ];
        let neighbour = json!({
            "resource": patient("Neighbour"),
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
            assert_eq!(after, untouched(), "batch {entry}");
        }
    }
}

/// A transaction fails as a whole and writes nothing, the neighbour included.
#[tokio::test]
async fn query_in_transaction_entry_criteria_fails_the_bundle() {
    for (criteria, param) in REFUSED_CRITERIA {
        let conditional_entries = [
            json!({"request": {"method": "DELETE", "url": format!("Patient?{criteria}")}}),
            json!({
                "resource": patient("Overwritten"),
                "request": {"method": "PUT", "url": format!("Patient?{criteria}")}
            }),
            json!({
                "resource": patient("Incoming"),
                "request": {"method": "POST", "url": "Patient", "ifNoneExist": criteria}
            }),
        ];
        let neighbour = json!({
            "resource": patient("Neighbour"),
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
                    "type": "transaction",
                    "entry": [neighbour, entry]
                }))
                .await;
            assert_rejected_naming(&response, param, &format!("transaction {entry}"));
            assert_eq!(
                families(&server).await,
                untouched(),
                "transaction {entry}: the whole Bundle must roll back"
            );
        }
    }
}

/// What must keep working: the `_` parameters the backends evaluate select
/// their target in every conditional interaction.
#[tokio::test]
async fn evaluated_underscore_criteria_still_select_the_target() {
    for criteria in [
        "_id=only",
        "_tag=http://example.org/tags|gold",
        "_lastUpdated=gt2000-01-01",
        "_id=only&_tag=gold",
    ] {
        let server = test_server().await;
        seed(&server).await;

        server
            .put(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .json(&patient("Updated"))
            .await
            .assert_status(StatusCode::OK);
        assert_eq!(
            families(&server).await,
            vec![("only".to_string(), "Updated".to_string())],
            "PUT {criteria}"
        );

        let response = server
            .post("/Patient")
            .add_header(X_TENANT_ID, tenant())
            .add_header(
                IF_NONE_EXIST,
                HeaderValue::from_str(criteria).expect("header value"),
            )
            .json(&patient("Incoming"))
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "If-None-Exist {criteria}: {}",
            response.text()
        );

        server
            .delete(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert!(families(&server).await.is_empty(), "DELETE {criteria}");
    }
}
