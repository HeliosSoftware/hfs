//! A batch or transaction `GET [compartment]/[id]/[type]` (or `/*`) entry is
//! the compartment search the URL names, as the direct request is: it answers
//! a searchset with the same matches, is judged on the target type's Search
//! grant, and under a `patient/` scope reaches the launch patient's own
//! compartment only. It used to be answered as a read of `[compartment]/[id]`.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::{TestResponse, TestServer};
use helios_audit::ExclusionFilter;
use helios_auth::{AuthConfig, AuthError, AuthProvider, LaunchContext, Principal, ScopeSet};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::{AuthMiddlewareState, ServerConfig};
use serde_json::{Value, json};

/// `system/*.cruds`: seeds and compares.
const SYSTEM: &str = "system-token";
/// `user/Patient.rs`: may read Patients, not search Observations.
const PATIENT_READ: &str = "patient-read-token";
/// `user/Observation.s`: may search Observations.
const OBSERVATION_SEARCH: &str = "observation-search-token";
/// `patient/*.rs` with `patient = p1`.
const P1: &str = "p1-token";

struct StubProvider;

#[async_trait]
impl AuthProvider for StubProvider {
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError> {
        let token = authorization_header
            .strip_prefix("Bearer ")
            .ok_or(AuthError::InvalidSignature)?;
        let (scopes, patient) = match token {
            SYSTEM => ("system/*.cruds", None),
            PATIENT_READ => ("user/Patient.rs", None),
            OBSERVATION_SEARCH => ("user/Observation.s", None),
            P1 => ("patient/*.rs", Some("p1")),
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

async fn server() -> TestServer {
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
            ..Default::default()
        },
    )
    .expect("in-memory sqlite");
    backend.init_schema().expect("init schema");
    let auth = Arc::new(AuthMiddlewareState {
        provider: Arc::new(StubProvider),
        config: Arc::new(AuthConfig::default()),
        audit_sink: Arc::new(helios_audit::sinks::NullSink),
        audit_source_observer: "Device/test".to_string(),
        audit_exclusion_filter: ExclusionFilter::new(vec![]),
        tenant_url_routing: false,
        sessions: None,
    });
    let app = helios_rest::create_app_with_auth(
        backend,
        ServerConfig::for_testing(),
        AuthConfig::default(),
        Some(auth),
        None,
    );
    let server = TestServer::new(app).expect("test server");

    let mut seed = vec![
        json!({"resourceType": "Patient", "id": "p1"}),
        json!({"resourceType": "Patient", "id": "p2"}),
    ];
    for (id, patient, code) in [
        ("o1", "p1", "8867-4"),
        ("o2", "p1", "8310-5"),
        ("o3", "p2", "8867-4"),
    ] {
        seed.push(json!({
            "resourceType": "Observation", "id": id, "status": "final",
            "code": {"coding": [{"system": "http://loinc.org", "code": code}]},
            "subject": {"reference": format!("Patient/{patient}")}
        }));
    }
    seed.push(json!({
        "resourceType": "Condition", "id": "c1",
        "subject": {"reference": "Patient/p1"}
    }));
    for resource in seed {
        let path = format!(
            "/{}/{}",
            resource["resourceType"].as_str().unwrap(),
            resource["id"].as_str().unwrap()
        );
        let response = server
            .put(&path)
            .add_header(AUTHORIZATION, bearer(SYSTEM))
            .json(&resource)
            .await;
        assert!(
            response.status_code().is_success(),
            "seed {path}: {}",
            response.text()
        );
    }
    server
}

fn bundle(kind: &str, urls: &[&str]) -> Value {
    json!({
        "resourceType": "Bundle",
        "type": kind,
        "entry": urls.iter().map(|url| json!({"request": {"method": "GET", "url": url}})).collect::<Vec<_>>()
    })
}

async fn post(server: &TestServer, token: &str, body: Value) -> TestResponse {
    server
        .post("/")
        .add_header(AUTHORIZATION, bearer(token))
        .json(&body)
        .await
}

/// The `Type/id` of every match of a searchset, sorted.
fn matches(searchset: &Value) -> Vec<String> {
    assert_eq!(searchset["resourceType"], "Bundle", "{searchset}");
    assert_eq!(searchset["type"], "searchset", "{searchset}");
    let mut ids: Vec<String> = searchset["entry"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter(|e| e["search"]["mode"] != "outcome")
                .map(|e| {
                    format!(
                        "{}/{}",
                        e["resource"]["resourceType"].as_str().unwrap_or_default(),
                        e["resource"]["id"].as_str().unwrap_or_default()
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

fn entry_status(body: &Value, index: usize) -> String {
    body["entry"][index]["response"]["status"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn an_entry_answers_the_compartment_search_the_direct_request_does() {
    let server = server().await;
    let urls = ["Patient/p1/Observation?code=8867-4", "Patient/p1/*"];

    for kind in ["batch", "transaction"] {
        let response = post(&server, SYSTEM, bundle(kind, &urls)).await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "{}",
            response.text()
        );
        let body: Value = response.json();
        for (index, url) in urls.iter().enumerate() {
            assert!(
                entry_status(&body, index).starts_with("200"),
                "{kind} {url}: {body}"
            );
            let direct = server
                .get(&format!("/{url}"))
                .add_header(AUTHORIZATION, bearer(SYSTEM))
                .await;
            assert_eq!(direct.status_code(), StatusCode::OK, "{}", direct.text());
            assert_eq!(
                matches(&body["entry"][index]["resource"]),
                matches(&direct.json::<Value>()),
                "{kind} {url}"
            );
        }
        assert_eq!(
            matches(&body["entry"][0]["resource"]),
            vec!["Observation/o1".to_string()]
        );
        let all = matches(&body["entry"][1]["resource"]);
        assert!(all.contains(&"Condition/c1".to_string()), "{all:?}");
        assert!(!all.contains(&"Observation/o3".to_string()), "{all:?}");
    }
}

#[tokio::test]
async fn an_entry_is_judged_on_the_target_types_search_grant() {
    let server = server().await;
    let url = "Patient/p1/Observation";

    let batch = post(&server, PATIENT_READ, bundle("batch", &[url])).await;
    assert_eq!(batch.status_code(), StatusCode::OK, "{}", batch.text());
    let body: Value = batch.json();
    assert!(entry_status(&body, 0).starts_with("403"), "{body}");
    assert!(body.to_string().contains("Observation"), "{body}");

    let transaction = post(&server, PATIENT_READ, bundle("transaction", &[url])).await;
    assert_eq!(
        transaction.status_code(),
        StatusCode::FORBIDDEN,
        "{}",
        transaction.text()
    );

    let allowed = post(&server, OBSERVATION_SEARCH, bundle("batch", &[url])).await;
    let body: Value = allowed.json();
    assert!(entry_status(&body, 0).starts_with("200"), "{body}");
    assert_eq!(
        matches(&body["entry"][0]["resource"]),
        vec!["Observation/o1".to_string(), "Observation/o2".to_string()]
    );
}

#[tokio::test]
async fn a_patient_scope_reaches_its_own_compartment_only() {
    let server = server().await;

    let own = post(&server, P1, bundle("batch", &["Patient/p1/Observation"])).await;
    let body: Value = own.json();
    assert!(entry_status(&body, 0).starts_with("200"), "{body}");
    assert_eq!(
        matches(&body["entry"][0]["resource"]),
        vec!["Observation/o1".to_string(), "Observation/o2".to_string()]
    );

    let other = post(&server, P1, bundle("batch", &["Patient/p2/Observation"])).await;
    let body: Value = other.json();
    assert!(entry_status(&body, 0).starts_with("403"), "{body}");
    assert!(!body.to_string().contains("\"o3\""), "{body}");

    let other = post(
        &server,
        P1,
        bundle("transaction", &["Patient/p2/Observation"]),
    )
    .await;
    assert!(
        !other.text().contains("\"o3\""),
        "another patient's Observation must not surface: {}",
        other.text()
    );
    assert!(
        other.status_code() == StatusCode::FORBIDDEN
            || other.json::<Value>()["entry"][0]["response"]["status"]
                .as_str()
                .is_some_and(|s| s.starts_with("403")),
        "{} {}",
        other.status_code(),
        other.text()
    );
}

#[tokio::test]
async fn a_shape_that_is_not_a_compartment_search_is_still_refused() {
    let server = server().await;
    for url in [
        "Patient/p1/foo",
        "Patient/Observation/o1",
        "Patient/p1/Observation/o1",
    ] {
        let response = post(&server, SYSTEM, bundle("transaction", &[url])).await;
        assert_eq!(
            response.status_code(),
            StatusCode::BAD_REQUEST,
            "{url}: {}",
            response.text()
        );
    }
}
