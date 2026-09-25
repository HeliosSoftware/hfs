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

// =============================================================================
// Repeated parameters (#1321)
// =============================================================================

/// A patient the repeated-parameter scenarios tell apart by birth date and
/// tags. Its family name is its id, so `families` shows who was rewritten.
async fn put_dated_patient(server: &TestServer, id: &str, birth_date: &str, tags: &[&str]) {
    server
        .put(&format!("/Patient/{id}"))
        .add_header(X_TENANT_ID, tenant())
        .json(&dated_patient(Some(id), id, birth_date, tags))
        .await
        .assert_status(StatusCode::CREATED);
}

fn dated_patient(id: Option<&str>, family: &str, birth_date: &str, tags: &[&str]) -> Value {
    let tags: Vec<Value> = tags
        .iter()
        .map(|code| json!({"system": "http://example.org/tags", "code": code}))
        .collect();
    let mut body = json!({
        "resourceType": "Patient",
        "meta": {"tag": tags},
        "name": [{"family": family}],
        "birthDate": birth_date
    });
    if let Some(id) = id {
        body["id"] = json!(id);
    }
    body
}

async fn search_ids(server: &TestServer, query: &str) -> Vec<String> {
    let bundle: Value = server
        .get(&format!("/Patient?{query}"))
        .add_header(X_TENANT_ID, tenant())
        .await
        .json();
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

/// The two repeated-parameter criteria. A repeated parameter is an AND; the
/// resource endpoints used to read the query into a `HashMap`, which kept the
/// last occurrence only.
const RANGE: &str = "birthdate=ge1980-01-01&birthdate=le1980-12-31";
const BOTH_TAGS: &str = "_tag=red&_tag=blue";

/// One decoy per constraint: each satisfies exactly one occurrence of `RANGE`
/// and exactly one of `BOTH_TAGS`.
async fn seed_repeated_parameter_decoys(server: &TestServer) {
    // Satisfies `le1980-12-31` and `_tag=blue` — the last occurrences.
    put_dated_patient(server, "early-blue", "1970-06-01", &["blue"]).await;
    // Satisfies `ge1980-01-01` and `_tag=red` — the first occurrences.
    put_dated_patient(server, "late-red", "1990-06-01", &["red"]).await;
}

async fn seed_repeated_parameter_target(server: &TestServer) {
    put_dated_patient(server, "target", "1980-06-01", &["red", "blue"]).await;
}

const DECOYS: [(&str, &str); 2] = [("early-blue", "early-blue"), ("late-red", "late-red")];

#[tokio::test]
async fn conditional_put_honours_every_occurrence_of_a_repeated_parameter() {
    for criteria in [RANGE, BOTH_TAGS] {
        let server = test_server().await;
        seed_repeated_parameter_decoys(&server).await;
        seed_repeated_parameter_target(&server).await;
        // Positive control: direct search ANDs the occurrences.
        assert_eq!(search_ids(&server, criteria).await, vec!["target"]);

        let response = server
            .put(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .json(&dated_patient(
                None,
                "Updated",
                "1980-06-01",
                &["red", "blue"],
            ))
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "PUT /Patient?{criteria} names exactly one patient"
        );
        assert_eq!(
            families(&server).await,
            pairs(&[DECOYS[0], DECOYS[1], ("target", "Updated")]),
            "{criteria}"
        );
    }
}

/// With no patient satisfying both constraints, nothing may be overwritten or
/// deleted — least of all the decoy that satisfies one of them.
#[tokio::test]
async fn a_resource_matching_one_occurrence_of_a_repeated_parameter_is_not_touched() {
    for criteria in [RANGE, BOTH_TAGS] {
        let server = test_server().await;
        put_dated_patient(&server, "early-blue", "1970-06-01", &["blue"]).await;
        assert!(search_ids(&server, criteria).await.is_empty());
        // Positive control: the decoy is indexed under the last occurrence.
        let last = criteria.rsplit('&').next().expect("last pair");
        assert_eq!(search_ids(&server, last).await, vec!["early-blue"]);

        let response = server
            .put(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .json(&dated_patient(None, "Incoming", "1980-06-01", &[]))
            .await;
        assert_eq!(response.status_code(), StatusCode::CREATED, "{criteria}");
        let after = families(&server).await;
        assert!(
            after.contains(&("early-blue".to_string(), "early-blue".to_string())),
            "PUT /Patient?{criteria} overwrote the decoy: {after:?}"
        );

        let server = test_server().await;
        put_dated_patient(&server, "early-blue", "1970-06-01", &["blue"]).await;
        server
            .delete(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(
            families(&server).await,
            pairs(&[DECOYS[0]]),
            "DELETE /Patient?{criteria} deleted a patient matching only one occurrence"
        );
    }
}

#[tokio::test]
async fn conditional_delete_honours_every_occurrence_of_a_repeated_parameter() {
    for criteria in [RANGE, BOTH_TAGS] {
        let server = test_server().await;
        seed_repeated_parameter_decoys(&server).await;
        seed_repeated_parameter_target(&server).await;

        server
            .delete(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(
            families(&server).await,
            pairs(&DECOYS),
            "DELETE /Patient?{criteria}"
        );
    }
}

/// `If-None-Exist` and the Bundle paths carry the criteria as one string, so
/// they never collapsed; pinned so they stay that way. (Conditional `PATCH`
/// has a handler with the same `HashMap`, fixed alongside, but no route: the
/// server answers `PATCH /Patient?…` with 405.)
#[tokio::test]
async fn header_and_bundle_criteria_honour_repeated_parameters() {
    for criteria in [RANGE, BOTH_TAGS] {
        // Only one-constraint decoys exist: the create must happen.
        let server = test_server().await;
        seed_repeated_parameter_decoys(&server).await;
        conditional_create(&server, criteria)
            .await
            .assert_status(StatusCode::CREATED);

        for bundle_type in ["batch", "transaction"] {
            let server = test_server().await;
            seed_repeated_parameter_decoys(&server).await;
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
                "{bundle_type} {criteria}: {status}"
            );
        }

        let server = test_server().await;
        seed_repeated_parameter_decoys(&server).await;
        seed_repeated_parameter_target(&server).await;
        post_bundle(
            &server,
            "batch",
            json!([{"request": {"method": "DELETE", "url": format!("Patient?{criteria}")}}]),
        )
        .await;
        assert_eq!(
            families(&server).await,
            pairs(&DECOYS),
            "batch DELETE Patient?{criteria}"
        );
    }
}

// =============================================================================
// Percent-decoding (#1322)
// =============================================================================

/// `Bundle.entry.response.status` of a one-entry Bundle carrying a conditional
/// create.
async fn bundle_if_none_exist_status(
    server: &TestServer,
    bundle_type: &str,
    criteria: &str,
) -> String {
    let reply = post_bundle(
        server,
        bundle_type,
        json!([{
            "resource": patient("Incoming", "incoming-1"),
            "request": {"method": "POST", "url": "Patient", "ifNoneExist": criteria}
        }]),
    )
    .await;
    reply["entry"][0]["response"]["status"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// Whether the criteria, as a direct search, find exactly the one seeded
/// patient. Every scenario below holds conditional criteria to this.
async fn direct_search_finds_existing(server: &TestServer, criteria: &str) -> bool {
    match search_ids(server, criteria).await.as_slice() {
        [] => false,
        [only] => {
            assert_eq!(only, "existing", "{criteria}");
            true
        }
        several => panic!("{criteria} found {several:?}"),
    }
}

/// `If-None-Exist` and `ifNoneExist` are the query portion of a search URL, so
/// they are form-urlencoded like one. A client that encodes `system|value` —
/// as any URL library does — must find the resource, not duplicate it.
///
/// Each case is the criteria and whether they name the seeded patient
/// (family `Mary Ann`, identifier `http://example.org/mrn|a+b&c=d,e`).
#[tokio::test]
async fn header_and_bundle_criteria_are_percent_decoded() {
    let cases = [
        // The issue's headline: an encoded `system|value`.
        (
            "identifier=http%3A%2F%2Fexample.org%2Fmrn%7Ca%2Bb%26c%3Dd%5C%2Ce",
            true,
        ),
        // A value containing `&`, `=` and `+`, encoded; the comma is escaped
        // the FHIR way (`\,`), itself encoded or not.
        ("identifier=a%2Bb%26c%3Dd%5C%2Ce", true),
        ("identifier=a%2Bb%26c%3Dd%5C,e", true),
        // `%2C` is a comma once decoded, and an unescaped comma separates OR
        // alternatives — exactly as in a direct search URL.
        ("identifier=a%2Bb%26c%3Dd%2Ce", false),
        ("identifier=absent%2Ca%2Bb%26c%3Dd%5C%2Ce", true),
        // `+` is a space; `%2B` is a plus.
        ("family=Mary+Ann", true),
        ("family=Mary%20Ann", true),
        ("family:exact=Mary%2BAnn", false),
        ("identifier=a+b%26c%3Dd%5C%2Ce", false),
        // An encoded name and modifier.
        ("family%3Aexact=Mary%20Ann", true),
    ];

    for (criteria, names_existing) in cases {
        let server = test_server().await;
        put_patient(&server, "existing", "Mary Ann", "a+b&c=d,e").await;
        // Conditional criteria must mean what the same string means as a
        // direct search — which is also the positive control on the seed.
        assert_eq!(
            direct_search_finds_existing(&server, criteria).await,
            names_existing,
            "direct search disagrees with the test's expectation for {criteria}"
        );

        let expected = if names_existing {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        };
        let response = conditional_create(&server, criteria).await;
        assert_eq!(
            response.status_code(),
            expected,
            "If-None-Exist: {criteria}"
        );

        for bundle_type in ["batch", "transaction"] {
            let server = test_server().await;
            put_patient(&server, "existing", "Mary Ann", "a+b&c=d,e").await;
            let status = bundle_if_none_exist_status(&server, bundle_type, criteria).await;
            assert!(
                status.starts_with(if names_existing { "200" } else { "201" }),
                "{bundle_type} ifNoneExist {criteria}: {status}"
            );
        }
    }
}

/// Conditional `PUT` / `DELETE` criteria arrive in the URL, which the server
/// used to decode and then re-join into one `a=b&c=d` string for the backend
/// to split again — so a decoded `&` or `=` inside a value became a pair
/// boundary. `identifier=a%26family%3DWilson` turned into "identifier `a` AND
/// family `Wilson`", which names the decoy.
#[tokio::test]
async fn url_criteria_with_an_ampersand_or_equals_in_a_value_hit_the_named_resource() {
    let criteria = "identifier=a%26family%3DWilson";

    let seed = |server: TestServer| async move {
        put_patient(&server, "target", "Neal", "a&family=Wilson").await;
        put_patient(&server, "decoy", "Wilson", "a").await;
        assert_eq!(search_ids(&server, criteria).await, vec!["target"]);
        server
    };

    let server = seed(test_server().await).await;
    server
        .put(&format!("/Patient?{criteria}"))
        .add_header(X_TENANT_ID, tenant())
        .json(&patient("Updated", "a&family=Wilson"))
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(
        families(&server).await,
        pairs(&[("decoy", "Wilson"), ("target", "Updated")]),
        "PUT /Patient?{criteria}"
    );

    let server = seed(test_server().await).await;
    server
        .delete(&format!("/Patient?{criteria}"))
        .add_header(X_TENANT_ID, tenant())
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_eq!(
        families(&server).await,
        pairs(&[("decoy", "Wilson")]),
        "DELETE /Patient?{criteria}"
    );

    for (method, expected) in [
        ("PUT", pairs(&[("decoy", "Wilson"), ("target", "Updated")])),
        ("DELETE", pairs(&[("decoy", "Wilson")])),
    ] {
        let server = seed(test_server().await).await;
        let mut entry =
            json!({"request": {"method": method, "url": format!("Patient?{criteria}")}});
        if method == "PUT" {
            entry["resource"] = patient("Updated", "a&family=Wilson");
        }
        post_bundle(&server, "batch", json!([entry])).await;
        assert_eq!(
            families(&server).await,
            expected,
            "batch {method} Patient?{criteria}"
        );
    }
}

// =============================================================================
// Unknown parameters (#1323)
// =============================================================================

const PREFER: HeaderName = HeaderName::from_static("prefer");

/// Criteria with a parameter the server does not know for `Patient`: a typo,
/// a parameter of another resource type, and the same beside a good criterion.
const UNKNOWN_PARAMETER_CRITERIA: [(&str, &str); 4] = [
    ("identifer=ne123", "identifer"),
    ("identifer:exact=ne123", "identifer"),
    ("identifier=ne123&specimen=ne123", "specimen"),
    ("identifier=ne123&Identifier=ne123", "Identifier"),
];

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

const SEEDED: [(&str, &str); 3] = [
    ("bystander", "Wilson"),
    ("decoy", "Allen"),
    ("target", "Neal"),
];

/// A misspelt criterion used to be searched for under its literal name, match
/// nothing, and so create a duplicate (`If-None-Exist`), create instead of
/// update (`PUT`), or answer a delete with a 204 that deleted nothing. Ignoring
/// it instead would widen the match of a write. It is refused — whatever
/// `Prefer: handling` says, which governs searches, not write preconditions.
#[tokio::test]
async fn an_unknown_criterion_is_rejected_and_nothing_is_written() {
    for (criteria, param) in UNKNOWN_PARAMETER_CRITERIA {
        for prefer in [None, Some("handling=lenient"), Some("handling=strict")] {
            let server = test_server().await;
            seed_target_and_decoy(&server).await;
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
                    .json(&patient("Incoming", "ne123")),
            )
            .await;
            assert_rejected_naming(&response, param, &format!("If-None-Exist: {context}"));

            let response = with_prefer(
                server
                    .put(&format!("/Patient?{criteria}"))
                    .add_header(X_TENANT_ID, tenant())
                    .json(&patient("Updated", "ne123")),
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

/// In a batch the entry fails and its neighbours proceed; a transaction fails
/// as a whole. Either way nothing is written for the bad entry.
#[tokio::test]
async fn an_unknown_criterion_fails_the_bundle_entry() {
    for (criteria, param) in UNKNOWN_PARAMETER_CRITERIA {
        let conditional_entries = [
            json!({
                "resource": patient("Incoming", "ne123"),
                "request": {"method": "POST", "url": "Patient", "ifNoneExist": criteria}
            }),
            json!({
                "resource": patient("Updated", "ne123"),
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
            seed_target_and_decoy(&server).await;
            let reply = post_bundle(&server, "batch", json!([entry, neighbour])).await;
            let response = &reply["entry"][0]["response"];
            assert!(
                response["status"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("400"),
                "batch {entry}: {response}"
            );
            assert!(
                response["outcome"]
                    .to_string()
                    .contains(&format!("'{param}'")),
                "batch {entry}: the outcome must name '{param}': {response}"
            );
            assert!(
                reply["entry"][1]["response"]["status"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("201"),
                "the neighbouring entry must proceed: {reply}"
            );
            let mut expected = pairs(&SEEDED);
            let after: Vec<(String, String)> = families(&server)
                .await
                .into_iter()
                .filter(|(_, family)| family != "Neighbour")
                .collect();
            expected.sort();
            assert_eq!(after, expected, "batch {entry}");
        }

        // Transaction: only `ifNoneExist` reaches the criteria builder (a
        // query-bearing `request.url` is refused outright).
        let server = test_server().await;
        seed_target_and_decoy(&server).await;
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

/// Result parameters are not criteria: beside a real criterion they change
/// nothing, and on their own they select nothing (never "everything").
#[tokio::test]
async fn result_parameters_are_ignored_not_rejected() {
    const RESULT_PARAMETERS: [&str; 14] = [
        "_format=json",
        "_pretty=true",
        "_summary=true",
        "_elements=name",
        "_count=1",
        "_offset=5",
        "_cursor=abc",
        "_sort=family",
        "_total=accurate",
        "_include=Patient:organization",
        "_revinclude=Observation:patient",
        "_contained=false",
        "_containedType=container",
        "_score=true",
    ];

    for result_parameter in RESULT_PARAMETERS {
        let server = test_server().await;
        seed_target_and_decoy(&server).await;
        let criteria = format!("{result_parameter}&identifier=ne123");

        conditional_create(&server, &criteria)
            .await
            .assert_status(StatusCode::OK);
        server
            .put(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .json(&patient("Updated", "ne123"))
            .await
            .assert_status(StatusCode::OK);
        assert_eq!(
            families(&server).await,
            pairs(&[
                ("bystander", "Wilson"),
                ("decoy", "Allen"),
                ("target", "Updated")
            ]),
            "{criteria}"
        );

        // Alone, it selects nothing — the delete must not sweep the type.
        server
            .delete(&format!("/Patient?{result_parameter}"))
            .add_header(X_TENANT_ID, tenant())
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(families(&server).await.len(), 3, "{result_parameter}");

        server
            .delete(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(
            families(&server).await,
            pairs(&[("bystander", "Wilson"), ("decoy", "Allen")]),
            "{criteria}"
        );
    }
}

/// The resource-level parameters are criteria like any other.
#[tokio::test]
async fn resource_level_parameters_are_valid_criteria() {
    for criteria in [
        "_id=target",
        "_tag=http://example.org/tags|gold",
        "_security=http://example.org/sec|R",
        "_profile=http://example.org/StructureDefinition/gold",
        "_source=http://example.org/feed",
        "_lastUpdated=gt2000-01-01&_tag=gold",
    ] {
        let server = test_server().await;
        put_patient(&server, "decoy", "Allen", "123").await;
        server
            .put("/Patient/target")
            .add_header(X_TENANT_ID, tenant())
            .json(&json!({
                "resourceType": "Patient",
                "id": "target",
                "meta": {
                    "source": "http://example.org/feed",
                    "profile": ["http://example.org/StructureDefinition/gold"],
                    "tag": [{"system": "http://example.org/tags", "code": "gold"}],
                    "security": [{"system": "http://example.org/sec", "code": "R"}]
                },
                "name": [{"family": "Neal"}]
            }))
            .await
            .assert_status(StatusCode::CREATED);
        // Positive control.
        assert_eq!(
            search_ids(&server, criteria).await,
            vec!["target"],
            "{criteria}"
        );

        server
            .delete(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(
            families(&server).await,
            pairs(&[("decoy", "Allen")]),
            "DELETE /Patient?{criteria}"
        );
    }
}

/// `id -> family` for every Patient of `tenant`.
async fn families_of(server: &TestServer, tenant: &'static str) -> Vec<(String, String)> {
    let bundle: Value = server
        .get("/Patient?_count=100")
        .add_header(X_TENANT_ID, HeaderValue::from_static(tenant))
        .await
        .json();
    let mut out: Vec<(String, String)> = bundle["entry"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            (
                e["resource"]["id"].as_str().unwrap_or("?").to_string(),
                e["resource"]["name"][1]["family"]
                    .as_str()
                    .unwrap_or("?")
                    .to_string(),
            )
        })
        .collect();
    out.sort();
    out
}

fn nicknamed_patient(id: Option<&str>, nickname: &str, family: &str) -> Value {
    let mut body = json!({
        "resourceType": "Patient",
        "name": [{"use": "nickname", "given": [nickname]}, {"family": family}]
    });
    if let Some(id) = id {
        body["id"] = json!(id);
    }
    body
}

/// "Registered for the resource type" means registered in the *tenant's*
/// registry, not only the spec set the server starts with: a custom
/// `SearchParameter` a tenant has POSTed is a valid criterion there — the
/// existing resource is found, no duplicate is created, a conditional `PUT`
/// updates it — while a tenant without that parameter still gets the 400 an
/// unknown criterion earns, and nothing is written.
#[tokio::test]
async fn a_tenant_custom_search_parameter_is_a_valid_criterion_only_in_that_tenant() {
    const WITH_PARAM: &str = "acme1";
    const WITHOUT_PARAM: &str = "acme2";
    let server = test_server().await;

    server
        .post("/SearchParameter")
        .add_header(X_TENANT_ID, HeaderValue::from_static(WITH_PARAM))
        .json(&json!({
            "resourceType": "SearchParameter",
            "id": "patient-nickname",
            "url": "http://acme.health/fhir/SearchParameter/patient-nickname",
            "name": "nickname",
            "status": "active",
            "code": "nickname",
            "base": ["Patient"],
            "type": "string",
            "expression": "Patient.name.where(use = 'nickname').given"
        }))
        .await
        .assert_status(StatusCode::CREATED);

    // Created after the parameter, so they are indexed under it. The decoy
    // makes a criterion that is ignored rather than evaluated observable: it
    // would match both.
    for (tenant, id, nickname, family) in [
        (WITH_PARAM, "target", "Ace", "Adams"),
        (WITH_PARAM, "decoy", "Bo", "Baker"),
        (WITHOUT_PARAM, "target", "Ace", "Adams"),
    ] {
        server
            .put(&format!("/Patient/{id}"))
            .add_header(X_TENANT_ID, HeaderValue::from_static(tenant))
            .json(&nicknamed_patient(Some(id), nickname, family))
            .await
            .assert_status(StatusCode::CREATED);
    }
    let seeded = pairs(&[("decoy", "Baker"), ("target", "Adams")]);
    assert_eq!(families_of(&server, WITH_PARAM).await, seeded);

    // If-None-Exist finds the existing resource instead of creating another.
    let response = server
        .post("/Patient")
        .add_header(X_TENANT_ID, HeaderValue::from_static(WITH_PARAM))
        .add_header(IF_NONE_EXIST, HeaderValue::from_static("nickname=Ace"))
        .json(&nicknamed_patient(None, "Ace", "Incoming"))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    let found: Value = response.json();
    assert_eq!(found["id"], "target", "{found}");
    assert_eq!(families_of(&server, WITH_PARAM).await, seeded);

    // A conditional PUT updates that same resource.
    let response = server
        .put("/Patient?nickname=Ace")
        .add_header(X_TENANT_ID, HeaderValue::from_static(WITH_PARAM))
        .json(&nicknamed_patient(None, "Ace", "Updated"))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    assert_eq!(
        families_of(&server, WITH_PARAM).await,
        pairs(&[("decoy", "Baker"), ("target", "Updated")])
    );

    // The other tenant never registered `nickname`: an unknown criterion.
    let response = server
        .post("/Patient")
        .add_header(X_TENANT_ID, HeaderValue::from_static(WITHOUT_PARAM))
        .add_header(IF_NONE_EXIST, HeaderValue::from_static("nickname=Ace"))
        .json(&nicknamed_patient(None, "Ace", "Incoming"))
        .await;
    assert_rejected_naming(&response, "nickname", WITHOUT_PARAM);
    assert_eq!(
        families_of(&server, WITHOUT_PARAM).await,
        pairs(&[("target", "Adams")])
    );
}

// =============================================================================
// Modifiers
// =============================================================================

/// Modifiers in criteria follow direct search's rules: an unknown modifier, a
/// `:Type` that is no resource type, and a modifier the parameter's type does
/// not define are a 400; a modifier that needs terminology expansion — which
/// conditional criteria never get — is a 501, not a literal match.
#[tokio::test]
async fn modifiers_in_criteria_follow_direct_search() {
    for (criteria, expected) in [
        ("family:nonsense=Neal", StatusCode::BAD_REQUEST),
        ("identifier:exact=ne123", StatusCode::BAD_REQUEST),
        ("general-practitioner:Bogus=1", StatusCode::BAD_REQUEST),
        ("_text:missing=true", StatusCode::BAD_REQUEST),
        (
            "identifier:in=http://example.org/ValueSet/mrns",
            StatusCode::NOT_IMPLEMENTED,
        ),
        (
            "identifier:not-in=http://example.org/ValueSet/mrns",
            StatusCode::NOT_IMPLEMENTED,
        ),
        ("gender:below=male", StatusCode::NOT_IMPLEMENTED),
        ("gender:above=male", StatusCode::NOT_IMPLEMENTED),
    ] {
        let server = test_server().await;
        seed_target_and_decoy(&server).await;

        // Direct search (no terminology server configured) answers the same.
        server
            .get(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .add_header(PREFER, HeaderValue::from_static("handling=strict"))
            .await
            .assert_status(expected);

        let response = conditional_create(&server, criteria).await;
        assert_eq!(
            response.status_code(),
            expected,
            "If-None-Exist: {criteria}"
        );
        let response = server
            .put(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .json(&patient("Updated", "ne123"))
            .await;
        assert_eq!(response.status_code(), expected, "PUT {criteria}");
        let response = server
            .delete(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .await;
        assert_eq!(response.status_code(), expected, "DELETE {criteria}");

        assert_eq!(families(&server).await, pairs(&SEEDED), "{criteria}");
    }
}

/// Modifiers the backends resolve natively still work.
#[tokio::test]
async fn natively_resolved_modifiers_still_select_the_target() {
    for criteria in [
        "family:exact=Neal",
        "family:contains=eal",
        "identifier:not=123&identifier:not=zz9",
    ] {
        let server = test_server().await;
        seed_target_and_decoy(&server).await;
        // Positive control.
        assert_eq!(
            search_ids(&server, criteria).await,
            vec!["target"],
            "{criteria}"
        );

        server
            .delete(&format!("/Patient?{criteria}"))
            .add_header(X_TENANT_ID, tenant())
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(
            families(&server).await,
            pairs(&[("bystander", "Wilson"), ("decoy", "Allen")]),
            "DELETE /Patient?{criteria}"
        );
    }
}
