//! #1381: `If-Match` on the conditional write interactions.
//!
//! `PUT` and `DELETE [type]?[criteria]` ignored the header — the client believed
//! it had version protection and did not — while `PATCH [type]?[criteria]`
//! refused it with `400` and a batch entry pairing `ifMatch` with a conditional
//! url was refused with `400` too. One rule now covers all of them: the
//! precondition is evaluated against the one resource the criteria resolve to,
//! inside the backend, ahead of the write.
//!
//! * satisfied → the write happens;
//! * not satisfied, or malformed → `412`, nothing written;
//! * nothing matched → what the instance twin answers for a missing resource:
//!   `412` for update (no create!) and delete, `404` for patch.
//!
//! Every test asserts the stored state afterwards, against decoys a wrong-target
//! write would show up on.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum_test::{TestRequest, TestResponse, TestServer};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const JSON_PATCH: &str = "application/json-patch+json";

/// The criteria every test uses: they select `target`, and only `target`.
const TARGET: &str = "/Patient?identifier=ne123";
/// Criteria that select nothing.
const NOBODY: &str = "/Patient?identifier=nobody";

fn tenant() -> HeaderValue {
    HeaderValue::from_static("test-tenant")
}

/// A server over in-memory SQLite with the spec search parameters loaded.
/// Without the data dir only five embedded parameters exist, `identifier` is
/// unknown, and every criteria below would be a `400`.
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

fn patient(id: Option<&str>, family: &str, identifier: &str) -> Value {
    let mut resource = json!({
        "resourceType": "Patient",
        "active": false,
        "name": [{"family": family}],
        "identifier": [{"system": "http://example.org/mrn", "value": identifier}]
    });
    if let Some(id) = id {
        resource["id"] = json!(id);
    }
    resource
}

/// `target` (the one the criteria select) and two decoys, all at version 1.
async fn seed(server: &TestServer) {
    for (id, family, identifier) in [
        ("target", "Neal", "ne123"),
        ("decoy-a", "Smith", "mrn-1"),
        ("decoy-b", "Smith", "mrn-2"),
    ] {
        server
            .put(&format!("/Patient/{id}"))
            .add_header(X_TENANT_ID, tenant())
            .json(&patient(Some(id), family, identifier))
            .await
            .assert_status(StatusCode::CREATED);
    }
    // Positive control: the criteria really do select exactly `target`. If
    // they selected nothing, every 412 below would be vacuous.
    let found: Value = server
        .get(TARGET)
        .add_header(X_TENANT_ID, tenant())
        .await
        .json();
    assert_eq!(found["entry"].as_array().map(Vec::len), Some(1), "{found}");
    assert_eq!(found["entry"][0]["resource"]["id"], "target");
}

/// `(id, versionId, active, family)` for every Patient: any write at all, to
/// anybody — a create included — shows up here.
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
        row("decoy-a", "1", "false", "Smith"),
        row("decoy-b", "1", "false", "Smith"),
        row("target", "1", "false", "Neal"),
    ]
}

/// `untouched()` with `target` replaced by the given row (or removed).
fn with_target(
    target: Option<(String, String, String, String)>,
) -> Vec<(String, String, String, String)> {
    let mut rows: Vec<_> = untouched()
        .into_iter()
        .filter(|r| r.0 != "target")
        .chain(target)
        .collect();
    rows.sort();
    rows
}

fn with_if_match(request: TestRequest, if_match: Option<&str>) -> TestRequest {
    match if_match {
        Some(value) => request.add_header(
            header::IF_MATCH,
            HeaderValue::from_str(value).expect("header value"),
        ),
        None => request,
    }
}

fn put(server: &TestServer, url: &str, if_match: Option<&str>) -> TestRequest {
    with_if_match(
        server
            .put(url)
            .add_header(X_TENANT_ID, tenant())
            .json(&patient(None, "Renamed", "ne123")),
        if_match,
    )
}

fn delete(server: &TestServer, url: &str, if_match: Option<&str>) -> TestRequest {
    with_if_match(
        server.delete(url).add_header(X_TENANT_ID, tenant()),
        if_match,
    )
}

fn patch(server: &TestServer, url: &str, if_match: Option<&str>) -> TestRequest {
    with_if_match(
        server
            .patch(url)
            .add_header(X_TENANT_ID, tenant())
            .add_header(header::CONTENT_TYPE, HeaderValue::from_static(JSON_PATCH))
            .bytes(
                json!([{"op": "replace", "path": "/active", "value": true}])
                    .to_string()
                    .into(),
            ),
        if_match,
    )
}

/// A `412` carrying an OperationOutcome that names the header.
fn assert_precondition_failed(response: &TestResponse, context: &str) {
    assert_eq!(
        response.status_code(),
        StatusCode::PRECONDITION_FAILED,
        "{context}: {}",
        response.text()
    );
    let outcome: Value = response.json();
    assert_eq!(outcome["resourceType"], "OperationOutcome", "{context}");
    assert_eq!(
        outcome["issue"][0]["code"], "conflict",
        "{context}: {outcome}"
    );
    assert!(
        outcome.to_string().contains("If-Match"),
        "{context}: {outcome}"
    );
}

/// Preconditions the target's version 1 satisfies.
const SATISFIED: [Option<&str>; 5] = [
    None,
    Some("W/\"1\""),
    Some("\"1\""),
    Some("*"),
    Some("W/\"7\", W/\"1\""),
];

/// Preconditions it does not: a stale tag, a list of stale tags, and values
/// that do not parse — which fail closed rather than count as absent.
const UNSATISFIED: [&str; 5] = ["W/\"7\"", "W/\"7\", W/\"8\"", "garbage", "1", ""];

// ---------------------------------------------------------------------------
// PUT [type]?[criteria]
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conditional_update_writes_when_if_match_is_satisfied() {
    for if_match in SATISFIED {
        let server = test_server().await;
        seed(&server).await;

        let response = put(&server, TARGET, if_match).await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "{if_match:?}: {}",
            response.text()
        );
        assert_eq!(
            snapshot(&server).await,
            with_target(Some(row("target", "2", "false", "Renamed"))),
            "{if_match:?}"
        );
    }
}

#[tokio::test]
async fn conditional_update_is_refused_when_if_match_is_not_satisfied() {
    let server = test_server().await;
    seed(&server).await;

    for if_match in UNSATISFIED {
        let response = put(&server, TARGET, Some(if_match)).await;
        assert_precondition_failed(&response, if_match);
        assert_eq!(snapshot(&server).await, untouched(), "{if_match:?}");
    }
}

/// The version compared is the *current* one: a tag that was right before
/// somebody else's write is stale after it.
#[tokio::test]
async fn conditional_update_compares_against_the_current_version() {
    let server = test_server().await;
    seed(&server).await;

    put(&server, TARGET, Some("W/\"1\""))
        .await
        .assert_status(StatusCode::OK);
    let response = put(&server, TARGET, Some("W/\"1\"")).await;
    assert_precondition_failed(&response, "stale after a write");
    put(&server, TARGET, Some("W/\"2\""))
        .await
        .assert_status(StatusCode::OK);

    assert_eq!(
        snapshot(&server).await,
        with_target(Some(row("target", "3", "false", "Renamed")))
    );
}

/// No match falls through to a create — unless the client named a version.
/// Nothing that does not exist can carry it, `*` included, so nothing is
/// created: the answer `PUT [type]/[id]` gives for a missing resource.
#[tokio::test]
async fn conditional_update_with_if_match_does_not_create() {
    let server = test_server().await;
    seed(&server).await;

    // Control: the instance endpoint refuses to create under `If-Match`.
    let response = put(&server, "/Patient/missing", Some("W/\"1\"")).await;
    assert_eq!(response.status_code(), StatusCode::PRECONDITION_FAILED);

    for if_match in ["W/\"1\"", "*", "garbage"] {
        let response = with_if_match(
            server
                .put(NOBODY)
                .add_header(X_TENANT_ID, tenant())
                .json(&patient(None, "Nobody", "nobody")),
            Some(if_match),
        )
        .await;
        assert_precondition_failed(&response, if_match);
        assert_eq!(snapshot(&server).await, untouched(), "{if_match:?}");
    }

    // Control: without the header the same request creates.
    server
        .put(NOBODY)
        .add_header(X_TENANT_ID, tenant())
        .json(&patient(None, "Nobody", "nobody"))
        .await
        .assert_status(StatusCode::CREATED);
    assert_eq!(snapshot(&server).await.len(), 4);
}

// ---------------------------------------------------------------------------
// DELETE [type]?[criteria]
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conditional_delete_deletes_when_if_match_is_satisfied() {
    for if_match in SATISFIED {
        let server = test_server().await;
        seed(&server).await;

        let response = delete(&server, TARGET, if_match).await;
        assert_eq!(
            response.status_code(),
            StatusCode::NO_CONTENT,
            "{if_match:?}: {}",
            response.text()
        );
        assert_eq!(snapshot(&server).await, with_target(None), "{if_match:?}");
        server
            .get("/Patient/target")
            .add_header(X_TENANT_ID, tenant())
            .await
            .assert_status(StatusCode::GONE);
    }
}

#[tokio::test]
async fn conditional_delete_is_refused_when_if_match_is_not_satisfied() {
    let server = test_server().await;
    seed(&server).await;

    for if_match in UNSATISFIED {
        let response = delete(&server, TARGET, Some(if_match)).await;
        assert_precondition_failed(&response, if_match);
        assert_eq!(snapshot(&server).await, untouched(), "{if_match:?}");
    }
    server
        .get("/Patient/target")
        .add_header(X_TENANT_ID, tenant())
        .await
        .assert_status(StatusCode::OK);
}

/// No match is `204` — unless the client named a version, which nothing that
/// does not exist can carry. `DELETE [type]/[id]` answers the same way.
#[tokio::test]
async fn conditional_delete_no_match_fails_a_supplied_if_match() {
    let server = test_server().await;
    seed(&server).await;

    // Control: the instance endpoint.
    let response = delete(&server, "/Patient/missing", Some("W/\"1\"")).await;
    assert_eq!(response.status_code(), StatusCode::PRECONDITION_FAILED);

    for if_match in ["W/\"1\"", "*"] {
        let response = delete(&server, NOBODY, Some(if_match)).await;
        assert_precondition_failed(&response, if_match);
    }
    delete(&server, NOBODY, None)
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_eq!(snapshot(&server).await, untouched());
}

// ---------------------------------------------------------------------------
// PATCH [type]?[criteria]
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conditional_patch_writes_when_if_match_is_satisfied() {
    for if_match in SATISFIED {
        let server = test_server().await;
        seed(&server).await;

        let response = patch(&server, TARGET, if_match).await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "{if_match:?}: {}",
            response.text()
        );
        assert_eq!(
            snapshot(&server).await,
            with_target(Some(row("target", "2", "true", "Neal"))),
            "{if_match:?}"
        );
    }
}

#[tokio::test]
async fn conditional_patch_is_refused_when_if_match_is_not_satisfied() {
    let server = test_server().await;
    seed(&server).await;

    for if_match in UNSATISFIED {
        let response = patch(&server, TARGET, Some(if_match)).await;
        assert_precondition_failed(&response, if_match);
        assert_eq!(snapshot(&server).await, untouched(), "{if_match:?}");
    }
}

/// `PATCH [type]/[id]` answers `404` for a missing resource whether or not
/// `If-Match` was sent; the conditional form agrees, and creates nothing.
#[tokio::test]
async fn conditional_patch_no_match_stays_not_found() {
    let server = test_server().await;
    seed(&server).await;

    // Control: the instance endpoint.
    patch(&server, "/Patient/missing", Some("W/\"1\""))
        .await
        .assert_status(StatusCode::NOT_FOUND);

    for if_match in [None, Some("W/\"1\""), Some("*")] {
        patch(&server, NOBODY, if_match)
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }
    assert_eq!(snapshot(&server).await, untouched());
}

// ---------------------------------------------------------------------------
// Bundle entries
// ---------------------------------------------------------------------------

fn bundle(bundle_type: &str, entries: Vec<Value>) -> Value {
    json!({"resourceType": "Bundle", "type": bundle_type, "entry": entries})
}

fn put_entry(url: &str, if_match: Option<&str>) -> Value {
    let mut entry = json!({
        "resource": patient(None, "Renamed", "ne123"),
        "request": {"method": "PUT", "url": url}
    });
    if let Some(if_match) = if_match {
        entry["request"]["ifMatch"] = json!(if_match);
    }
    entry
}

fn delete_entry(url: &str, if_match: Option<&str>) -> Value {
    let mut entry = json!({"request": {"method": "DELETE", "url": url}});
    if let Some(if_match) = if_match {
        entry["request"]["ifMatch"] = json!(if_match);
    }
    entry
}

/// A neighbour whose fate shows whether the rest of the Bundle ran.
fn neighbour_entry() -> Value {
    json!({
        "resource": patient(Some("neighbour"), "Neighbour", "mrn-9"),
        "request": {"method": "PUT", "url": "Patient/neighbour"}
    })
}

async fn post_bundle(server: &TestServer, bundle: &Value) -> TestResponse {
    server
        .post("/")
        .add_header(X_TENANT_ID, tenant())
        .json(bundle)
        .await
}

fn entry_status(response: &Value, index: usize) -> String {
    response["entry"][index]["response"]["status"]
        .as_str()
        .unwrap_or("?")
        .to_string()
}

#[tokio::test]
async fn batch_conditional_update_honours_if_match() {
    for if_match in SATISFIED {
        let server = test_server().await;
        seed(&server).await;

        let response = post_bundle(
            &server,
            &bundle(
                "batch",
                vec![put_entry("Patient?identifier=ne123", if_match)],
            ),
        )
        .await;
        response.assert_status(StatusCode::OK);
        let body: Value = response.json();
        assert!(
            entry_status(&body, 0).starts_with("200"),
            "{if_match:?}: {body}"
        );
        assert_eq!(
            snapshot(&server).await,
            with_target(Some(row("target", "2", "false", "Renamed"))),
            "{if_match:?}"
        );
    }
}

#[tokio::test]
async fn batch_conditional_delete_honours_if_match() {
    for if_match in SATISFIED {
        let server = test_server().await;
        seed(&server).await;

        let response = post_bundle(
            &server,
            &bundle(
                "batch",
                vec![delete_entry("Patient?identifier=ne123", if_match)],
            ),
        )
        .await;
        response.assert_status(StatusCode::OK);
        let body: Value = response.json();
        assert!(
            entry_status(&body, 0).starts_with("204"),
            "{if_match:?}: {body}"
        );
        assert_eq!(snapshot(&server).await, with_target(None), "{if_match:?}");
    }
}

/// The failing entry is a `412`; its neighbours proceed.
#[tokio::test]
async fn batch_entry_with_unsatisfied_if_match_fails_alone() {
    for if_match in ["W/\"7\"", "garbage"] {
        for failing in [
            put_entry("Patient?identifier=ne123", Some(if_match)),
            delete_entry("Patient?identifier=ne123", Some(if_match)),
            // No match: no create, and no "nothing to delete" success either.
            put_entry("Patient?identifier=nobody", Some(if_match)),
            delete_entry("Patient?identifier=nobody", Some(if_match)),
        ] {
            let server = test_server().await;
            seed(&server).await;

            let response = post_bundle(
                &server,
                &bundle("batch", vec![failing.clone(), neighbour_entry()]),
            )
            .await;
            response.assert_status(StatusCode::OK);
            let body: Value = response.json();
            assert!(
                entry_status(&body, 0).starts_with("412"),
                "{failing}: {body}"
            );
            let outcome = &body["entry"][0]["response"]["outcome"];
            assert_eq!(outcome["issue"][0]["code"], "conflict", "{body}");
            assert!(outcome.to_string().contains("If-Match"), "{body}");
            assert!(
                entry_status(&body, 1).starts_with("201"),
                "{failing}: {body}"
            );

            let mut expected = untouched();
            expected.push(row("neighbour", "1", "false", "Neighbour"));
            expected.sort();
            assert_eq!(snapshot(&server).await, expected, "{failing}");
        }
    }
}

/// `ifMatch` beside `ifNoneExist` is still the pairing FHIR gives no meaning.
#[tokio::test]
async fn batch_if_match_beside_if_none_exist_is_still_refused() {
    let server = test_server().await;
    seed(&server).await;

    let response = post_bundle(
        &server,
        &bundle(
            "batch",
            vec![json!({
                "resource": patient(None, "Nobody", "nobody"),
                "request": {
                    "method": "POST",
                    "url": "Patient",
                    "ifNoneExist": "identifier=nobody",
                    "ifMatch": "W/\"1\""
                }
            })],
        ),
    )
    .await;
    let body: Value = response.json();
    assert!(entry_status(&body, 0).starts_with("400"), "{body}");
    assert_eq!(snapshot(&server).await, untouched());
}

/// A transaction cannot resolve URL criteria inside its atomic scope and
/// declines the whole Bundle (#503), `ifMatch` or not. Nothing is written — not
/// the conditional entry, and not the neighbour ahead of it.
#[tokio::test]
async fn transaction_with_conditional_url_writes_nothing() {
    let server = test_server().await;
    seed(&server).await;

    for if_match in [None, Some("W/\"1\""), Some("W/\"7\""), Some("garbage")] {
        for conditional in [
            put_entry("Patient?identifier=ne123", if_match),
            delete_entry("Patient?identifier=ne123", if_match),
        ] {
            let response = post_bundle(
                &server,
                &bundle("transaction", vec![neighbour_entry(), conditional.clone()]),
            )
            .await;
            assert_eq!(
                response.status_code(),
                StatusCode::BAD_REQUEST,
                "{conditional}: {}",
                response.text()
            );
            let outcome: Value = response.json();
            assert_eq!(outcome["issue"][0]["code"], "not-supported", "{outcome}");
            assert_eq!(snapshot(&server).await, untouched(), "{conditional}");
        }
    }
}
