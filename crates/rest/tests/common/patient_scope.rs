//! The `patient/` scope compartment scenarios (#1618), shared by the
//! per-backend suites: `patient_scope_compartment.rs` (SQLite) and its
//! `_postgres`, `_mongodb` and `_elasticsearch` twins.
//!
//! Every request goes through the real `auth_middleware` and
//! `authz_middleware` with a stub `AuthProvider`, so the scenarios exercise
//! the whole chain: scope parsing, the `patient` launch claim, the policy's
//! grant, the middleware's path check and the handlers' compartment filtering
//! and membership checks. A backend suite builds its FHIR routes, hands them
//! to [`serve`], and expands [`patient_scope_suite!`] over its server factory.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_audit::ExclusionFilter;
use helios_auth::{AuthConfig, AuthError, AuthProvider, LaunchContext, Principal, ScopeSet};
use helios_persistence::backends::local_fs::LocalFsOutputStore;
use helios_persistence::core::ExportOutputStore;
use helios_rest::AuthMiddlewareState;
use serde_json::{Value, json};

/// `system/*.cruds`: seeds the data and stands for an unrestricted caller.
const SYSTEM: &str = "system-token";
/// `patient/*.cruds` with `patient = p1`.
const P1_WRITE: &str = "p1-write-token";
/// `patient/*.rs` with `patient = p1`.
const P1_READ: &str = "p1-read-token";
/// `patient/*.rs` with `patient = Patient/p1` (the relative-reference form).
const P1_REF: &str = "p1-ref-token";
/// `patient/*.rs` and **no** launch context.
const NO_CONTEXT: &str = "no-context-token";
/// `patient/*.rs` with an unusable `patient` claim.
const BAD_CONTEXT: &str = "bad-context-token";
/// `user/Observation.rs patient/*.rs` with `patient = p1`: Observation is
/// unrestricted, everything else is confined.
const MIXED: &str = "mixed-token";

struct StubProvider;

#[async_trait]
impl AuthProvider for StubProvider {
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError> {
        let token = authorization_header
            .strip_prefix("Bearer ")
            .ok_or(AuthError::InvalidSignature)?;
        let (scopes, patient) = match token {
            SYSTEM => ("system/*.cruds", None),
            P1_WRITE => ("patient/*.cruds", Some("p1")),
            P1_READ => ("patient/*.rs", Some("p1")),
            P1_REF => ("patient/*.rs", Some("Patient/p1")),
            NO_CONTEXT => ("patient/*.rs", None),
            BAD_CONTEXT => ("patient/*.rs", Some("https://ehr.example/fhir/Patient/p1")),
            MIXED => ("user/Observation.rs patient/*.rs", Some("p1")),
            _ => return Err(AuthError::InvalidSignature),
        };
        Ok(Principal {
            subject: format!("sub-{token}"),
            issuer: "https://idp.example.com".to_string(),
            scopes: ScopeSet::parse(scopes),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
            launch_context: patient.map(|p| LaunchContext {
                patient: Some(p.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        })
    }

    fn name(&self) -> &str {
        "stub"
    }
}

const AUTHORIZATION: HeaderName = HeaderName::from_static("authorization");

fn bearer(token: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("Bearer {token}")).unwrap()
}

/// Wraps a backend's FHIR routes in the same two layers `create_app_with_auth`
/// applies, in the same order, with the stub provider.
pub fn serve(routes: Router) -> TestServer {
    let auth = Arc::new(AuthMiddlewareState {
        provider: Arc::new(StubProvider),
        config: Arc::new(AuthConfig::default()),
        audit_sink: Arc::new(helios_audit::sinks::NullSink),
        audit_source_observer: "Device/test".to_string(),
        audit_exclusion_filter: ExclusionFilter::new(vec![]),
        tenant_url_routing: false,
        sessions: None,
    });
    let app = routes
        .layer(axum::middleware::from_fn_with_state(
            auth.clone(),
            helios_rest::middleware::auth::authz_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            auth,
            helios_rest::middleware::auth::auth_middleware,
        ));
    TestServer::new(app).expect("test server")
}

/// A local-filesystem export output store that lives as long as the test
/// binary, so `$export` kick-offs are judged by the handler rather than
/// answered 501.
pub fn export_output() -> Arc<dyn ExportOutputStore> {
    let output_dir = tempfile::tempdir().expect("tempdir");
    let output = Arc::new(LocalFsOutputStore::new(
        output_dir.path(),
        "https://export.example",
    ));
    // Keep the export directory alive for the server's lifetime.
    std::mem::forget(output_dir);
    output
}

/// The workspace `data/` directory: the spec search parameters are what
/// compartment membership is decided with.
pub fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

async fn put(server: &TestServer, token: &str, path: &str, body: Value) -> axum_test::TestResponse {
    server
        .put(path)
        .add_header(AUTHORIZATION, bearer(token))
        .json(&body)
        .await
}

async fn get(server: &TestServer, token: &str, path: &str) -> axum_test::TestResponse {
    server
        .get(path)
        .add_header(AUTHORIZATION, bearer(token))
        .await
}

fn observation(id: &str, subject: &str) -> Value {
    json!({
        "resourceType": "Observation",
        "id": id,
        "status": "final",
        "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
        "subject": {"reference": subject},
    })
}

/// Two patients, an Observation each, an Observation of p1 performed by a
/// Practitioner, an Encounter of p2, and a Practitioner (not in any patient's
/// compartment).
pub async fn seed(server: &TestServer) {
    for (path, body) in [
        (
            "/Patient/p1",
            json!({"resourceType": "Patient", "id": "p1", "name": [{"family": "One"}]}),
        ),
        (
            "/Patient/p2",
            json!({"resourceType": "Patient", "id": "p2", "name": [{"family": "Two"}]}),
        ),
        (
            "/Practitioner/pr1",
            json!({"resourceType": "Practitioner", "id": "pr1"}),
        ),
        ("/Observation/o1", observation("o1", "Patient/p1")),
        ("/Observation/o2", observation("o2", "Patient/p2")),
        (
            "/Observation/o3",
            json!({
                "resourceType": "Observation", "id": "o3", "status": "final",
                "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
                "subject": {"reference": "Patient/p1"},
                "performer": [{"reference": "Practitioner/pr1"}],
            }),
        ),
        (
            "/Encounter/e2",
            json!({"resourceType": "Encounter", "id": "e2", "status": "finished",
                "class": {"system": "http://terminology.hl7.org/CodeSystem/v3-ActCode", "code": "AMB"},
                "subject": {"reference": "Patient/p2"}}),
        ),
    ] {
        let response = put(server, SYSTEM, path, body).await;
        assert!(
            response.status_code().is_success(),
            "seed {path}: {} {}",
            response.status_code(),
            response.text()
        );
    }
}

fn ids(bundle: &Value) -> Vec<String> {
    let mut ids: Vec<String> = bundle["entry"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter(|e| e["search"]["mode"].as_str() != Some("include"))
                .filter_map(|e| e["resource"]["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

fn included_ids(bundle: &Value) -> Vec<String> {
    let mut ids: Vec<String> = bundle["entry"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter(|e| e["search"]["mode"].as_str() == Some("include"))
                .filter_map(|e| e["resource"]["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

fn assert_forbidden(response: &axum_test::TestResponse, context: &str) {
    assert_eq!(
        response.status_code(),
        StatusCode::FORBIDDEN,
        "{context}: {}",
        response.text()
    );
    let outcome: Value = response.json();
    assert_eq!(outcome["resourceType"], "OperationOutcome", "{context}");
    assert_eq!(outcome["issue"][0]["code"], "forbidden", "{context}");
}

// --- The issue's repro -------------------------------------------------------

pub async fn the_issue_repro_is_closed(server: &TestServer) {
    // patient/*.rs patient=p1 GET /Patient → p1 only (was [p2, p1]).
    let r = get(server, P1_READ, "/Patient").await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());
    assert_eq!(ids(&r.json()), ["p1"]);

    // GET /Patient/p2 → 403 (was 200 with the body).
    assert_forbidden(
        &get(server, P1_READ, "/Patient/p2").await,
        "read other patient",
    );

    // GET /Observation → o1, o3 only (was [o2, o1]).
    let r = get(server, P1_READ, "/Observation").await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());
    assert_eq!(ids(&r.json()), ["o1", "o3"]);

    // GET /Observation/o2 → 403.
    assert_forbidden(
        &get(server, P1_READ, "/Observation/o2").await,
        "read other patient's observation",
    );

    // patient/*.rs with no claim → 403, never widened to user/.
    assert_forbidden(
        &get(server, NO_CONTEXT, "/Patient").await,
        "no launch patient",
    );
    assert_forbidden(
        &get(server, NO_CONTEXT, "/Observation").await,
        "no launch patient, Observation",
    );
    let text = get(server, NO_CONTEXT, "/Patient").await.text();
    assert!(text.contains("patient launch context"), "{text}");

    // GET /$export → 403 (was 202 for a system-level export).
    assert_forbidden(&get(server, P1_READ, "/$export").await, "system export");
}

// --- Reads and searches ------------------------------------------------------

pub async fn the_launch_patient_and_its_resources_are_readable(server: &TestServer) {
    for token in [P1_READ, P1_REF, P1_WRITE] {
        let r = get(server, token, "/Patient/p1").await;
        assert_eq!(r.status_code(), StatusCode::OK, "{token}: {}", r.text());
        let r = get(server, token, "/Observation/o1").await;
        assert_eq!(r.status_code(), StatusCode::OK, "{token}: {}", r.text());
        let r = get(server, token, "/Observation/o1/_history/1").await;
        assert_eq!(r.status_code(), StatusCode::OK, "{token}: {}", r.text());
        let r = server
            .method(axum::http::Method::HEAD, "/Observation/o1")
            .add_header(AUTHORIZATION, bearer(token))
            .await;
        assert_eq!(r.status_code(), StatusCode::OK, "{token} HEAD");
    }
}

pub async fn other_patients_resources_are_refused_on_every_read_path(server: &TestServer) {
    assert_forbidden(&get(server, P1_READ, "/Observation/o2").await, "read");
    assert_forbidden(
        &get(server, P1_READ, "/Observation/o2/_history/1").await,
        "vread",
    );
    assert_forbidden(
        &get(server, P1_READ, "/Observation/o2/_history").await,
        "instance history",
    );
    assert_forbidden(&get(server, P1_READ, "/Encounter/e2").await, "encounter");
    let head = server
        .method(axum::http::Method::HEAD, "/Observation/o2")
        .add_header(AUTHORIZATION, bearer(P1_READ))
        .await;
    assert_eq!(head.status_code(), StatusCode::FORBIDDEN, "HEAD");
}

pub async fn searches_are_filtered_to_the_compartment_whatever_the_client_asks(
    server: &TestServer,
) {
    // Explicitly asking for the other patient yields nothing, not a leak.
    let r = get(server, P1_READ, "/Patient?_id=p2").await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());
    assert!(ids(&r.json()).is_empty(), "{}", r.text());
    let r = get(server, P1_READ, "/Observation?subject=Patient/p2").await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());
    assert!(ids(&r.json()).is_empty(), "{}", r.text());
    let r = get(server, P1_READ, "/Encounter").await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());
    assert!(ids(&r.json()).is_empty(), "{}", r.text());

    // POST _search is the same search.
    let r = server
        .post("/Observation/_search")
        .add_header(AUTHORIZATION, bearer(P1_READ))
        .form(&[("status", "final")])
        .await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());
    assert_eq!(ids(&r.json()), ["o1", "o3"]);

    // The client's own filters still apply inside the compartment.
    let r = get(server, P1_READ, "/Observation?_id=o3").await;
    assert_eq!(ids(&r.json()), ["o3"]);

    // The matching patient's own search is pinned to the launch patient.
    let r = get(server, P1_READ, "/Patient?family=Two").await;
    assert!(ids(&r.json()).is_empty(), "{}", r.text());
    let r = get(server, P1_REF, "/Patient").await;
    assert_eq!(ids(&r.json()), ["p1"]);
}

pub async fn includes_outside_the_compartment_are_dropped(server: &TestServer) {
    // o3's performer is a Practitioner: not in any patient's compartment.
    let r = get(
        server,
        P1_READ,
        "/Observation?_id=o3&_include=Observation:performer&_include=Observation:subject",
    )
    .await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());
    let bundle: Value = r.json();
    assert_eq!(ids(&bundle), ["o3"]);
    assert_eq!(included_ids(&bundle), ["p1"], "{bundle}");

    // The same search unconfined includes the Practitioner.
    let r = get(
        server,
        SYSTEM,
        "/Observation?_id=o3&_include=Observation:performer&_include=Observation:subject",
    )
    .await;
    assert_eq!(included_ids(&r.json()), ["p1", "pr1"]);
}

pub async fn types_outside_the_patient_compartment_are_not_reachable(server: &TestServer) {
    assert_forbidden(&get(server, P1_READ, "/Practitioner/pr1").await, "read");
    assert_forbidden(&get(server, P1_READ, "/Practitioner").await, "search");
    assert_forbidden(&get(server, P1_READ, "/Binary").await, "Binary search");
    // Unconfined, the same reads work.
    let r = get(server, SYSTEM, "/Practitioner/pr1").await;
    assert_eq!(r.status_code(), StatusCode::OK);
}

pub async fn compartment_paths_are_the_launch_patients_only(server: &TestServer) {
    let r = get(server, P1_READ, "/Patient/p1/Observation").await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());
    assert_eq!(ids(&r.json()), ["o1", "o3"]);
    let r = get(server, P1_READ, "/Patient/p1/*").await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());
    assert_eq!(ids(&r.json()), ["o1", "o3"]);
    assert_forbidden(
        &get(server, P1_READ, "/Patient/p2/Observation").await,
        "other patient's compartment",
    );
    assert_forbidden(
        &get(server, P1_READ, "/Encounter/e2/Observation").await,
        "another compartment",
    );

    let r = get(server, P1_READ, "/Patient/p1/$everything").await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());
    assert_forbidden(
        &get(server, P1_READ, "/Patient/p2/$everything").await,
        "other patient's $everything",
    );
    // Type-level $everything narrows to the launch patient (#1121) and the
    // other patient never appears.
    let r = get(server, P1_READ, "/Patient/$everything").await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());
    let bundle: Value = r.json();
    let everything: Vec<&str> = bundle["entry"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["resource"]["id"].as_str())
        .collect();
    assert!(everything.contains(&"p1"), "{bundle}");
    assert!(!everything.contains(&"p2"), "{bundle}");
    assert!(!everything.contains(&"o2"), "{bundle}");
}

pub async fn endpoints_that_cannot_be_confined_are_refused(server: &TestServer) {
    for path in [
        "/_history",
        "/Observation/_history",
        "/Patient/_history",
        "/$export",
        "/Group/g1/$export",
        "/Observation/$validate",
        "/ValueSet/$expand",
        "/_user/settings",
    ] {
        assert_forbidden(&get(server, P1_READ, path).await, path);
    }
    let r = server
        .post("/Observation/$validate")
        .add_header(AUTHORIZATION, bearer(P1_WRITE))
        .json(&observation("v", "Patient/p1"))
        .await;
    assert_forbidden(&r, "POST $validate");
    // Still open to everyone: the exempt paths.
    let r = server.get("/metadata").await;
    assert_eq!(r.status_code(), StatusCode::OK);
}

// --- Writes ------------------------------------------------------------------

pub async fn writes_stay_inside_the_compartment(server: &TestServer) {
    // Create: the payload must name the launch patient.
    let r = server
        .post("/Observation")
        .add_header(AUTHORIZATION, bearer(P1_WRITE))
        .json(&observation("", "Patient/p1"))
        .await;
    assert_eq!(r.status_code(), StatusCode::CREATED, "{}", r.text());
    let r = server
        .post("/Observation")
        .add_header(AUTHORIZATION, bearer(P1_WRITE))
        .json(&observation("", "Patient/p2"))
        .await;
    assert_forbidden(&r, "create for other patient");
    let r = server
        .post("/Observation")
        .add_header(AUTHORIZATION, bearer(P1_WRITE))
        .json(&json!({"resourceType": "Observation", "status": "final", "code": {"text": "x"}}))
        .await;
    assert_forbidden(&r, "create with no patient");
    let r = server
        .post("/Practitioner")
        .add_header(AUTHORIZATION, bearer(P1_WRITE))
        .json(&json!({"resourceType": "Practitioner"}))
        .await;
    assert_forbidden(&r, "create outside the compartment");
    // A conditional create could match (and disclose) another patient's data.
    let r = server
        .post("/Observation")
        .add_header(AUTHORIZATION, bearer(P1_WRITE))
        .add_header(
            HeaderName::from_static("if-none-exist"),
            HeaderValue::from_static("subject=Patient/p1"),
        )
        .json(&observation("", "Patient/p1"))
        .await;
    assert_forbidden(&r, "conditional create");

    // Update: the other patient's resource cannot be taken over, and the
    // launch patient's resource cannot be moved out.
    let r = put(
        server,
        P1_WRITE,
        "/Observation/o2",
        observation("o2", "Patient/p1"),
    )
    .await;
    assert_forbidden(&r, "update other patient's resource");
    let r = put(
        server,
        P1_WRITE,
        "/Observation/o1",
        observation("o1", "Patient/p2"),
    )
    .await;
    assert_forbidden(&r, "move resource out");
    let r = put(
        server,
        P1_WRITE,
        "/Observation/o1",
        observation("o1", "Patient/p1"),
    )
    .await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());
    // Update-as-create of a new id inside the compartment is fine.
    let r = put(
        server,
        P1_WRITE,
        "/Observation/o9",
        observation("o9", "Patient/p1"),
    )
    .await;
    assert_eq!(r.status_code(), StatusCode::CREATED, "{}", r.text());
    // The Patient itself: own record yes, the other no.
    let r = put(
        server,
        P1_WRITE,
        "/Patient/p1",
        json!({"resourceType": "Patient", "id": "p1", "name": [{"family": "Uno"}]}),
    )
    .await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());
    let r = put(
        server,
        P1_WRITE,
        "/Patient/p2",
        json!({"resourceType": "Patient", "id": "p2", "name": [{"family": "Dos"}]}),
    )
    .await;
    assert_forbidden(&r, "update other patient");

    // Patch: not the other patient's, and not out of the compartment.
    let patch = |path: &str, ops: Value| {
        server
            .patch(path)
            .add_header(AUTHORIZATION, bearer(P1_WRITE))
            .content_type("application/json-patch+json")
            .bytes(serde_json::to_vec(&ops).unwrap().into())
    };
    let r = patch(
        "/Observation/o2",
        json!([{"op": "replace", "path": "/status", "value": "amended"}]),
    )
    .await;
    assert_forbidden(&r, "patch other patient's resource");
    let r = patch(
        "/Observation/o1",
        json!([{"op": "replace", "path": "/subject/reference", "value": "Patient/p2"}]),
    )
    .await;
    assert_forbidden(&r, "patch resource out");
    let r = patch(
        "/Observation/o1",
        json!([{"op": "replace", "path": "/status", "value": "amended"}]),
    )
    .await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());

    // Delete.
    let r = server
        .delete("/Observation/o2")
        .add_header(AUTHORIZATION, bearer(P1_WRITE))
        .await;
    assert_forbidden(&r, "delete other patient's resource");
    let r = server
        .delete("/Observation/o3")
        .add_header(AUTHORIZATION, bearer(P1_WRITE))
        .await;
    assert_eq!(r.status_code(), StatusCode::NO_CONTENT, "{}", r.text());

    // Conditional writes are refused outright.
    let r = put(
        server,
        P1_WRITE,
        "/Observation?subject=Patient/p1",
        observation("", "Patient/p1"),
    )
    .await;
    assert_forbidden(&r, "conditional update");
    let r = server
        .delete("/Observation?subject=Patient/p1")
        .add_header(AUTHORIZATION, bearer(P1_WRITE))
        .await;
    assert_forbidden(&r, "conditional delete");

    // Read-only patient scope: still 403 for a write, as before (plain scope
    // denial, not the compartment).
    let r = put(
        server,
        P1_READ,
        "/Observation/o1",
        observation("o1", "Patient/p1"),
    )
    .await;
    assert_forbidden(&r, "read-only scope");

    // Nothing above changed the other patient's data.
    let r = get(server, SYSTEM, "/Observation/o2").await;
    assert_eq!(r.status_code(), StatusCode::OK);
    assert_eq!(r.json::<Value>()["subject"]["reference"], "Patient/p2");
    assert_eq!(r.json::<Value>()["status"], "final");
}

// --- Bundles -----------------------------------------------------------------

pub async fn bundles_confine_reads_and_refuse_writes(server: &TestServer) {
    let batch = json!({
        "resourceType": "Bundle", "type": "batch",
        "entry": [
            {"request": {"method": "GET", "url": "Observation?status=final"}},
            {"request": {"method": "GET", "url": "Observation/o1"}},
            {"request": {"method": "GET", "url": "Observation/o2"}},
            {"request": {"method": "POST", "url": "Observation"}, "resource": observation("", "Patient/p1")},
            {"request": {"method": "DELETE", "url": "Observation/o1"}},
        ]
    });
    let r = server
        .post("/")
        .add_header(AUTHORIZATION, bearer(P1_WRITE))
        .json(&batch)
        .await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());
    let bundle: Value = r.json();
    let entries = bundle["entry"].as_array().unwrap();
    assert_eq!(entries.len(), 5, "{bundle}");
    assert!(
        entries[0]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("200"),
        "{bundle}"
    );
    assert_eq!(ids(&entries[0]["resource"]), ["o1", "o3"], "{bundle}");
    assert!(
        entries[1]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("200"),
        "{bundle}"
    );
    assert!(
        entries[2]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("403"),
        "{bundle}"
    );
    assert!(
        entries[3]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("403"),
        "{bundle}"
    );
    assert!(
        entries[4]["response"]["status"]
            .as_str()
            .unwrap()
            .starts_with("403"),
        "{bundle}"
    );

    // A transaction with a write under the confined grant is refused whole.
    let transaction = json!({
        "resourceType": "Bundle", "type": "transaction",
        "entry": [
            {"request": {"method": "GET", "url": "Observation?status=final"}},
            {"request": {"method": "POST", "url": "Observation"}, "resource": observation("", "Patient/p1")},
        ]
    });
    let r = server
        .post("/")
        .add_header(AUTHORIZATION, bearer(P1_WRITE))
        .json(&transaction)
        .await;
    assert_forbidden(&r, "transaction with a write");
    let r = get(server, SYSTEM, "/Observation").await;
    assert_eq!(ids(&r.json()), ["o1", "o2", "o3"], "nothing was written");

    // A transaction of searches only is confined.
    let searches = json!({
        "resourceType": "Bundle", "type": "transaction",
        "entry": [{"request": {"method": "GET", "url": "Observation?status=final"}}]
    });
    let r = server
        .post("/")
        .add_header(AUTHORIZATION, bearer(P1_WRITE))
        .json(&searches)
        .await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());
    let bundle: Value = r.json();
    assert_eq!(
        ids(&bundle["entry"][0]["resource"]),
        ["o1", "o3"],
        "{bundle}"
    );
}

// --- Grants ------------------------------------------------------------------

pub async fn a_patient_scope_without_a_usable_launch_patient_is_refused_everywhere(
    server: &TestServer,
) {
    for token in [NO_CONTEXT, BAD_CONTEXT] {
        for path in [
            "/Patient",
            "/Patient/p1",
            "/Observation",
            "/Observation/o1",
            "/Patient/p1/$everything",
        ] {
            assert_forbidden(&get(server, token, path).await, &format!("{token} {path}"));
        }
    }
}

pub async fn a_wider_context_scope_is_not_confined_but_the_patient_scope_still_is(
    server: &TestServer,
) {
    // user/Observation.rs: both patients' observations.
    let r = get(server, MIXED, "/Observation").await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());
    assert_eq!(ids(&r.json()), ["o1", "o2", "o3"]);
    let r = get(server, MIXED, "/Observation/o2").await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());
    // patient/*.rs for everything else: confined to p1.
    let r = get(server, MIXED, "/Patient").await;
    assert_eq!(ids(&r.json()), ["p1"]);
    assert_forbidden(&get(server, MIXED, "/Patient/p2").await, "other patient");
    assert_forbidden(
        &get(server, MIXED, "/Encounter/e2").await,
        "other patient's encounter",
    );
}

pub async fn unrestricted_callers_are_unchanged(server: &TestServer) {
    let r = get(server, SYSTEM, "/Patient").await;
    assert_eq!(ids(&r.json()), ["p1", "p2"]);
    let r = get(server, SYSTEM, "/Observation/o2").await;
    assert_eq!(r.status_code(), StatusCode::OK);
    let r = get(server, SYSTEM, "/Observation/_history").await;
    assert_eq!(r.status_code(), StatusCode::OK, "{}", r.text());
    let r = get(server, SYSTEM, "/Patient/p2/Observation").await;
    assert_eq!(ids(&r.json()), ["o2"]);
}

// --- Bulk export -------------------------------------------------------------

pub async fn patient_export_is_pinned_to_the_launch_patient(server: &TestServer) {
    // System and Group level cannot be confined.
    assert_forbidden(&get(server, P1_READ, "/$export").await, "system export");
    assert_forbidden(
        &get(server, P1_READ, "/Group/g1/$export").await,
        "group export",
    );
    // Patient level with a `patient` parameter naming someone else.
    let r = server
        .post("/Patient/$export")
        .add_header(AUTHORIZATION, bearer(P1_READ))
        .add_header(
            HeaderName::from_static("prefer"),
            HeaderValue::from_static("respond-async"),
        )
        .json(&json!({"resourceType": "Parameters", "parameter": [
            {"name": "patient", "valueReference": {"reference": "Patient/p2"}}
        ]}))
        .await;
    assert_forbidden(&r, "export of another patient");
    // Patient level for the launch patient is accepted.
    let r = server
        .get("/Patient/$export?_type=Observation")
        .add_header(AUTHORIZATION, bearer(P1_READ))
        .add_header(
            HeaderName::from_static("prefer"),
            HeaderValue::from_static("respond-async"),
        )
        .await;
    assert_eq!(r.status_code(), StatusCode::ACCEPTED, "{}", r.text());
    // A type outside the compartment cannot be exported under the grant.
    let r = server
        .get("/Patient/$export?_type=Practitioner")
        .add_header(AUTHORIZATION, bearer(P1_READ))
        .add_header(
            HeaderName::from_static("prefer"),
            HeaderValue::from_static("respond-async"),
        )
        .await;
    assert_forbidden(&r, "export of a non-compartment type");
}

// --- Suite -------------------------------------------------------------------

/// Expands one `#[tokio::test]` per scenario. `$factory` is an
/// `async fn(&str) -> Option<TestServer>` taking the scenario name (for a
/// per-test tenant or database); `None` skips the scenario (no Docker).
#[macro_export]
macro_rules! patient_scope_suite {
    ($factory:path) => {
        $crate::patient_scope_suite!(@each $factory; the_issue_repro_is_closed, the_launch_patient_and_its_resources_are_readable, other_patients_resources_are_refused_on_every_read_path, searches_are_filtered_to_the_compartment_whatever_the_client_asks, includes_outside_the_compartment_are_dropped, types_outside_the_patient_compartment_are_not_reachable, compartment_paths_are_the_launch_patients_only, endpoints_that_cannot_be_confined_are_refused, writes_stay_inside_the_compartment, bundles_confine_reads_and_refuse_writes, a_patient_scope_without_a_usable_launch_patient_is_refused_everywhere, a_wider_context_scope_is_not_confined_but_the_patient_scope_still_is, unrestricted_callers_are_unchanged, patient_export_is_pinned_to_the_launch_patient);
    };
    (@each $factory:path; $($name:ident),* $(,)?) => {
        $(
            #[tokio::test]
            async fn $name() {
                let Some(server) = $factory(stringify!($name)).await else {
                    eprintln!(concat!("Skipping ", stringify!($name), " (requires Docker)"));
                    return;
                };
                $crate::patient_scope::seed(&server).await;
                $crate::patient_scope::$name(&server).await;
            }
        )*
    };
}
