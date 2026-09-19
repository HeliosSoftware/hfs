//! #1312: conditional-interaction criteria must mean what the same string
//! means as a direct search.
//!
//! Every backend used to parse the criteria of `If-None-Exist`, conditional
//! `PUT` / `DELETE`, and a Bundle entry's `ifNoneExist` with a routine that
//! strips any leading two letters spelling a comparator (`eq ne gt lt ge le sa
//! eb ap`, case-insensitively) — for every parameter type, not just the
//! ordered ones. `family=Neal` searched for `al`; `identifier=ne123` for `123`.
//!
//! Two failures follow, and both are asserted here over HTTP:
//!
//! * the resource the criteria name is **not found** — `If-None-Exist` creates
//!   the duplicate it exists to prevent, and a conditional `PUT` creates
//!   instead of updating;
//! * a resource the criteria do **not** name **is found** — string search is a
//!   prefix match, so `al` finds `Allen`, and the token `123` is a perfectly
//!   ordinary identifier. The conditional `PUT` then overwrites, and the
//!   conditional `DELETE` deletes, somebody else's resource.
//!
//! Each scenario seeds a decoy carrying the value the old parser would have
//! searched for, so a wrong-target write is observable, plus controls whose
//! values start with no comparator letters.

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

fn tenant() -> HeaderValue {
    HeaderValue::from_static("test-tenant")
}

/// A server over in-memory SQLite with the spec search parameters loaded.
/// Without the data dir only five embedded parameters exist, `family` and
/// `identifier` are unknown, and every assertion below would pass vacuously.
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

async fn put_patient(server: &TestServer, id: &str, family: &str, identifier: &str) {
    let mut body = patient(family, identifier);
    body["id"] = json!(id);
    server
        .put(&format!("/Patient/{id}"))
        .add_header(X_TENANT_ID, tenant())
        .json(&body)
        .await
        .assert_status(StatusCode::CREATED);
}

/// The target (`Neal` / `ne123`), the decoy the old parser would have hit
/// (`Allen` starts with `al`; `123` is `ne123` minus `ne`), and a bystander.
async fn seed_target_and_decoy(server: &TestServer) {
    put_patient(server, "target", "Neal", "ne123").await;
    put_patient(server, "decoy", "Allen", "123").await;
    put_patient(server, "bystander", "Wilson", "zz9").await;
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

async fn conditional_create(server: &TestServer, criteria: &str) -> axum_test::TestResponse {
    server
        .post("/Patient")
        .add_header(X_TENANT_ID, tenant())
        .add_header(
            IF_NONE_EXIST,
            HeaderValue::from_str(criteria).expect("header value"),
        )
        .json(&patient("Incoming", "incoming-1"))
        .await
}

// =============================================================================
// Conditional create (If-None-Exist)
// =============================================================================

/// The issue's headline: a family name that starts with comparator letters is
/// not found, so the guard against duplicates creates one.
#[tokio::test]
async fn if_none_exist_finds_a_family_name_that_starts_with_comparator_letters() {
    for family in [
        "Nelson", "Levine", "Gtari", "Sanchez", "Ebert", "Appleby", "Eqbal", "Geller", "Ltaief",
        // Control: no comparator letters.
        "Wilson",
    ] {
        let server = test_server().await;
        put_patient(&server, "existing", family, "mrn-1").await;

        let response = conditional_create(&server, &format!("family={family}")).await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "If-None-Exist: family={family} must find the existing {family}"
        );
        assert_eq!(
            families(&server).await,
            pairs(&[("existing", family)]),
            "family={family} must not have created a duplicate"
        );
    }
}

#[tokio::test]
async fn if_none_exist_finds_a_token_that_starts_with_comparator_letters() {
    for identifier in ["ne123", "eq77", "sa-1", "LE-9", "ap0", "MRN-1"] {
        let server = test_server().await;
        put_patient(&server, "existing", "Wilson", identifier).await;

        for criteria in [
            format!("identifier={identifier}"),
            format!("identifier=http://example.org/mrn|{identifier}"),
        ] {
            let response = conditional_create(&server, &criteria).await;
            assert_eq!(
                response.status_code(),
                StatusCode::OK,
                "If-None-Exist: {criteria} must find the existing patient"
            );
        }
        assert_eq!(families(&server).await.len(), 1, "{identifier}");
    }
}

/// The other direction: the criteria name nobody, but the mangled value names
/// the decoy, so the create was silently swallowed as "already exists".
#[tokio::test]
async fn if_none_exist_does_not_answer_with_a_resource_the_criteria_do_not_name() {
    for criteria in ["family=Neal", "identifier=ne123"] {
        let server = test_server().await;
        put_patient(&server, "decoy", "Allen", "123").await;

        let response = conditional_create(&server, criteria).await;
        assert_eq!(
            response.status_code(),
            StatusCode::CREATED,
            "{criteria} names nobody on the server, so the create must happen"
        );
        assert_eq!(families(&server).await.len(), 2, "{criteria}");
    }
}

/// Comma-separated values are an OR-list, split on unescaped commas only.
#[tokio::test]
async fn if_none_exist_treats_commas_as_an_or_list() {
    let server = test_server().await;
    seed_target_and_decoy(&server).await;

    // Two of the three patients match: not unique.
    conditional_create(&server, "identifier=ne123,zz9")
        .await
        .assert_status(StatusCode::PRECONDITION_FAILED);
    // One alternative matches.
    conditional_create(&server, "identifier=ne123,absent")
        .await
        .assert_status(StatusCode::OK);
    // An escaped comma is part of the value, which nobody carries.
    conditional_create(&server, "identifier=ne123\\,zz9")
        .await
        .assert_status(StatusCode::CREATED);
}

// =============================================================================
// Conditional update
// =============================================================================

#[tokio::test]
async fn conditional_put_updates_the_resource_the_criteria_name() {
    for criteria in [
        "family=Neal",
        "identifier=ne123",
        "family:exact=Neal",
        // A content-negotiation parameter is not a criterion.
        "identifier=ne123&_format=json",
    ] {
        let server = test_server().await;
        seed_target_and_decoy(&server).await;

        let response = server
            .put(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .json(&patient("Updated", "ne123"))
            .await;
        assert_eq!(response.status_code(), StatusCode::OK, "{criteria}");
        assert_eq!(
            families(&server).await,
            pairs(&[
                ("bystander", "Wilson"),
                ("decoy", "Allen"),
                ("target", "Updated")
            ]),
            "PUT /Patient?{criteria} must update the target and leave the decoy alone"
        );
    }
}

/// With the target absent the update is a create — never an overwrite of the
/// decoy.
#[tokio::test]
async fn conditional_put_does_not_overwrite_an_unrelated_resource() {
    for criteria in ["family=Neal", "identifier=ne123"] {
        let server = test_server().await;
        put_patient(&server, "decoy", "Allen", "123").await;

        let response = server
            .put(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .json(&patient("Neal", "ne123"))
            .await;
        assert_eq!(response.status_code(), StatusCode::CREATED, "{criteria}");

        let after = families(&server).await;
        assert_eq!(after.len(), 2, "{criteria}");
        assert!(
            after.contains(&("decoy".to_string(), "Allen".to_string())),
            "PUT /Patient?{criteria} overwrote the decoy: {after:?}"
        );
    }
}

// =============================================================================
// Conditional delete
// =============================================================================

#[tokio::test]
async fn conditional_delete_deletes_the_resource_the_criteria_name() {
    for criteria in ["family=Neal", "identifier=ne123"] {
        let server = test_server().await;
        seed_target_and_decoy(&server).await;

        server
            .delete(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(
            families(&server).await,
            pairs(&[("bystander", "Wilson"), ("decoy", "Allen")]),
            "DELETE /Patient?{criteria} must delete the target and only the target"
        );
    }
}

/// The dangerous direction: nothing matches the criteria as written, so
/// nothing may be deleted — least of all the resource the mangled value names.
#[tokio::test]
async fn conditional_delete_does_not_delete_an_unrelated_resource() {
    for (criteria, decoy_family, decoy_identifier) in [
        ("family=Neal", "Allen", "mrn-1"),
        ("family=Levine", "Vineyard", "mrn-1"),
        ("family=Lee", "Evans", "mrn-1"),
        ("identifier=ne123", "Wilson", "123"),
        ("identifier=eq77", "Wilson", "77"),
        ("identifier=sa-1", "Wilson", "-1"),
    ] {
        let server = test_server().await;
        put_patient(&server, "decoy", decoy_family, decoy_identifier).await;

        let response = server
            .delete(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .await;
        assert!(
            response.status_code().is_success(),
            "{criteria}: {}",
            response.status_code()
        );
        assert_eq!(
            families(&server).await,
            pairs(&[("decoy", decoy_family)]),
            "DELETE /Patient?{criteria} deleted a patient the criteria do not name"
        );
    }
}

// =============================================================================
// Other parameter types
// =============================================================================

/// A bare-id reference value is exposed the same way: `subject=eb12` was
/// searched as `12`.
#[tokio::test]
async fn reference_criteria_keep_their_leading_letters() {
    let server = test_server().await;
    for (id, subject) in [("obs-target", "Patient/eb12"), ("obs-decoy", "Patient/12")] {
        server
            .put(&format!("/Observation/{id}"))
            .add_header(X_TENANT_ID, tenant())
            .json(&json!({
                "resourceType": "Observation",
                "id": id,
                "status": "final",
                "code": {"text": "note"},
                "subject": {"reference": subject}
            }))
            .await
            .assert_status(StatusCode::CREATED);
    }

    server
        .delete("/Observation?subject=eb12")
        .add_header(X_TENANT_ID, tenant())
        .await
        .assert_status(StatusCode::NO_CONTENT);

    server
        .get("/Observation/obs-decoy")
        .add_header(X_TENANT_ID, tenant())
        .await
        .assert_status(StatusCode::OK);
    server
        .get("/Observation/obs-target")
        .add_header(X_TENANT_ID, tenant())
        .await
        .assert_status(StatusCode::GONE);
}

#[tokio::test]
async fn uri_criteria_keep_their_leading_letters() {
    let server = test_server().await;
    let value_set = json!({
        "resourceType": "ValueSet",
        "status": "active",
        "url": "sandbox.example.org/ValueSet/a"
    });
    for expected in [StatusCode::CREATED, StatusCode::OK] {
        let response = server
            .post("/ValueSet")
            .add_header(X_TENANT_ID, tenant())
            .add_header(
                IF_NONE_EXIST,
                HeaderValue::from_static("url=sandbox.example.org/ValueSet/a"),
            )
            .json(&value_set)
            .await;
        assert_eq!(response.status_code(), expected);
    }
}

/// Comparators are still comparators where FHIR defines them.
#[tokio::test]
async fn date_criteria_still_honour_comparators() {
    let server = test_server().await;
    for (id, birth_date) in [("elder", "1940-05-01"), ("younger", "1990-05-01")] {
        server
            .put(&format!("/Patient/{id}"))
            .add_header(X_TENANT_ID, tenant())
            .json(&json!({"resourceType": "Patient", "id": id, "birthDate": birth_date}))
            .await
            .assert_status(StatusCode::CREATED);
    }

    server
        .delete("/Patient?birthdate=lt1950-01-01")
        .add_header(X_TENANT_ID, tenant())
        .await
        .assert_status(StatusCode::NO_CONTENT);

    let remaining: Vec<String> = families(&server)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(remaining, vec!["younger".to_string()]);
}

/// Criteria this path cannot evaluate are refused. They used to be searched
/// for under their literal name, match nothing, and turn the conditional
/// update into an unconditional create.
#[tokio::test]
async fn criteria_that_cannot_be_evaluated_are_refused_not_ignored() {
    let server = test_server().await;
    seed_target_and_decoy(&server).await;

    for criteria in [
        "general-practitioner.name=Neal",
        "_has:Observation:patient:code=1234-5",
        "family:nonsense=Neal",
        "identifier:exact=ne123",
    ] {
        let response = server
            .put(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .json(&patient("Updated", "ne123"))
            .await;
        assert!(
            response.status_code().is_client_error() || response.status_code().is_server_error(),
            "PUT /Patient?{criteria} answered {}",
            response.status_code()
        );
        assert_eq!(
            families(&server).await.len(),
            3,
            "{criteria} wrote something"
        );
    }
}

// =============================================================================
// Bundles
// =============================================================================

async fn post_bundle(server: &TestServer, bundle_type: &str, entries: Value) -> Value {
    let response = server
        .post("/")
        .add_header(X_TENANT_ID, tenant())
        .json(&json!({"resourceType": "Bundle", "type": bundle_type, "entry": entries}))
        .await;
    response.assert_status(StatusCode::OK);
    response.json()
}

/// `ifNoneExist` reaches the same criteria builder from a batch (through
/// `conditional_create`) and from a transaction (through each backend's
/// in-transaction resolver).
#[tokio::test]
async fn bundle_if_none_exist_finds_prefix_like_values() {
    for bundle_type in ["batch", "transaction"] {
        for criteria in ["family=Neal", "identifier=ne123"] {
            // The target exists: no write.
            let server = test_server().await;
            put_patient(&server, "target", "Neal", "ne123").await;
            let reply = post_bundle(
                &server,
                bundle_type,
                json!([{
                    "resource": patient("Incoming", "incoming-1"),
                    "request": {"method": "POST", "url": "Patient", "ifNoneExist": criteria}
                }]),
            )
            .await;
            let status = reply["entry"][0]["response"]["status"]
                .as_str()
                .unwrap_or_default();
            assert!(
                status.starts_with("200"),
                "{bundle_type} ifNoneExist {criteria}: expected 200, got {status:?}"
            );
            assert_eq!(families(&server).await, pairs(&[("target", "Neal")]));

            // Only the decoy exists: the create happens.
            let server = server_with_decoy().await;
            let reply = post_bundle(
                &server,
                bundle_type,
                json!([{
                    "resource": patient("Incoming", "incoming-1"),
                    "request": {"method": "POST", "url": "Patient", "ifNoneExist": criteria}
                }]),
            )
            .await;
            let status = reply["entry"][0]["response"]["status"]
                .as_str()
                .unwrap_or_default();
            assert!(
                status.starts_with("201"),
                "{bundle_type} ifNoneExist {criteria}: expected 201, got {status:?}"
            );
        }
    }
}

async fn server_with_decoy() -> TestServer {
    let server = test_server().await;
    put_patient(&server, "decoy", "Allen", "123").await;
    server
}

/// Batch entries carry conditional update and delete in `request.url`.
/// (A transaction refuses a query-bearing url outright, so only `ifNoneExist`
/// reaches the builder from there.)
#[tokio::test]
async fn batch_conditional_put_and_delete_hit_the_named_resource() {
    for criteria in ["family=Neal", "identifier=ne123"] {
        let server = test_server().await;
        seed_target_and_decoy(&server).await;
        post_bundle(
            &server,
            "batch",
            json!([{
                "resource": patient("Updated", "ne123"),
                "request": {"method": "PUT", "url": format!("Patient?{criteria}")}
            }]),
        )
        .await;
        assert_eq!(
            families(&server).await,
            pairs(&[
                ("bystander", "Wilson"),
                ("decoy", "Allen"),
                ("target", "Updated")
            ]),
            "batch PUT Patient?{criteria}"
        );

        let server = test_server().await;
        seed_target_and_decoy(&server).await;
        post_bundle(
            &server,
            "batch",
            json!([{"request": {"method": "DELETE", "url": format!("Patient?{criteria}")}}]),
        )
        .await;
        assert_eq!(
            families(&server).await,
            pairs(&[("bystander", "Wilson"), ("decoy", "Allen")]),
            "batch DELETE Patient?{criteria}"
        );
    }
}
