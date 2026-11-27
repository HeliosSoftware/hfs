//! A transaction's conditional references (`Type?query`) are resolved by a
//! server-side search on the referenced type. That search is authorized as
//! `GET [type]?query` would be: the token needs a search grant on the
//! referenced type, judged before any lookup runs, and a `patient/` grant
//! confines the lookup to the launch patient's compartment. A token that may
//! only create Observations cannot use the resolver to learn whether, or
//! which, Patient matches a query.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use helios_audit::ExclusionFilter;
use helios_auth::{AuthConfig, AuthError, AuthProvider, LaunchContext, Principal, ScopeSet};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::{AuthMiddlewareState, ServerConfig};
use serde_json::{Value, json};

/// `system/*.cruds`: seeds and inspects the data.
const SYSTEM: &str = "system-token";
/// `system/Observation.c`: may create Observations and nothing else.
const OBS_CREATE: &str = "obs-create-token";
/// `system/Observation.c system/Patient.s`: may also search Patients.
const OBS_CREATE_PATIENT_SEARCH: &str = "obs-create-patient-search-token";
/// `user/Observation.c patient/Patient.rs` with `patient = p1`.
const P1_MIXED: &str = "p1-mixed-token";

struct StubProvider;

#[async_trait]
impl AuthProvider for StubProvider {
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError> {
        let token = authorization_header
            .strip_prefix("Bearer ")
            .ok_or(AuthError::InvalidSignature)?;
        let (scopes, patient) = match token {
            SYSTEM => ("system/*.cruds", None),
            OBS_CREATE => ("system/Observation.c", None),
            OBS_CREATE_PATIENT_SEARCH => ("system/Observation.c system/Patient.s", None),
            P1_MIXED => ("user/Observation.c patient/Patient.rs", Some("p1")),
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

fn server() -> TestServer {
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
    TestServer::new(app).expect("test server")
}

async fn seed_patient(server: &TestServer, id: &str, mrn: &str) {
    let response = server
        .put(&format!("/Patient/{id}"))
        .add_header(AUTHORIZATION, bearer(SYSTEM))
        .json(&json!({
            "resourceType": "Patient",
            "id": id,
            "identifier": [{"system": "https://example.org/mrn", "value": mrn}],
            "name": [{"family": "Doe"}]
        }))
        .await;
    assert!(
        response.status_code().is_success(),
        "seed Patient/{id}: {}",
        response.text()
    );
}

fn observation_transaction(subject: &str) -> Value {
    json!({
        "resourceType": "Bundle",
        "type": "transaction",
        "entry": [{
            "fullUrl": "urn:uuid:11111111-1111-1111-1111-111111111111",
            "resource": {
                "resourceType": "Observation",
                "status": "final",
                "code": {"text": "probe"},
                "subject": {"reference": subject}
            },
            "request": {"method": "POST", "url": "Observation"}
        }]
    })
}

async fn post_transaction(
    server: &TestServer,
    token: &str,
    subject: &str,
) -> axum_test::TestResponse {
    server
        .post("/")
        .add_header(AUTHORIZATION, bearer(token))
        .json(&observation_transaction(subject))
        .await
}

async fn observation_count(server: &TestServer) -> u64 {
    let response = server
        .get("/Observation?_summary=count")
        .add_header(AUTHORIZATION, bearer(SYSTEM))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    response.json::<Value>()["total"].as_u64().unwrap_or(0)
}

#[tokio::test]
async fn a_token_without_a_search_grant_on_the_referenced_type_is_refused_whatever_the_match() {
    let server = server();
    seed_patient(&server, "p1", "12345").await;

    let matching = post_transaction(
        &server,
        OBS_CREATE,
        "Patient?identifier=https://example.org/mrn|12345",
    )
    .await;
    let absent = post_transaction(
        &server,
        OBS_CREATE,
        "Patient?identifier=https://example.org/mrn|99999",
    )
    .await;

    for (what, response) in [("a matching", &matching), ("an absent", &absent)] {
        assert_eq!(
            response.status_code(),
            StatusCode::FORBIDDEN,
            "{what} Patient: {}",
            response.text()
        );
        assert!(
            response
                .text()
                .contains("Insufficient scope for search on Patient"),
            "{what} Patient: {}",
            response.text()
        );
    }
    assert_eq!(observation_count(&server).await, 0, "nothing was written");
}

#[tokio::test]
async fn a_search_grant_on_the_referenced_type_resolves_the_reference() {
    let server = server();
    seed_patient(&server, "p1", "12345").await;

    let response = post_transaction(
        &server,
        OBS_CREATE_PATIENT_SEARCH,
        "Patient?identifier=https://example.org/mrn|12345",
    )
    .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );

    let stored = server
        .get("/Observation")
        .add_header(AUTHORIZATION, bearer(SYSTEM))
        .await;
    let body: Value = stored.json();
    assert_eq!(
        body["entry"][0]["resource"]["subject"]["reference"], "Patient/p1",
        "{body}"
    );

    let absent = post_transaction(
        &server,
        OBS_CREATE_PATIENT_SEARCH,
        "Patient?identifier=https://example.org/mrn|99999",
    )
    .await;
    assert_eq!(
        absent.status_code(),
        StatusCode::BAD_REQUEST,
        "{}",
        absent.text()
    );
    assert!(
        absent.text().contains("matches no existing resource"),
        "{}",
        absent.text()
    );
}

#[tokio::test]
async fn a_patient_scoped_grant_confines_the_lookup_to_the_launch_patient() {
    let server = server();
    seed_patient(&server, "p1", "11111").await;
    seed_patient(&server, "p2", "22222").await;

    let own = post_transaction(
        &server,
        P1_MIXED,
        "Patient?identifier=https://example.org/mrn|11111",
    )
    .await;
    assert_eq!(own.status_code(), StatusCode::OK, "{}", own.text());

    // The other patient exists, but not inside p1's compartment: the lookup
    // answers exactly as it would for a patient that does not exist.
    let other = post_transaction(
        &server,
        P1_MIXED,
        "Patient?identifier=https://example.org/mrn|22222",
    )
    .await;
    assert_eq!(
        other.status_code(),
        StatusCode::BAD_REQUEST,
        "{}",
        other.text()
    );
    assert!(
        other.text().contains("matches no existing resource"),
        "{}",
        other.text()
    );
    assert_eq!(observation_count(&server).await, 1);
}

#[tokio::test]
async fn an_unrestricted_caller_is_unchanged() {
    let server = server();
    seed_patient(&server, "p1", "12345").await;

    let response = post_transaction(
        &server,
        SYSTEM,
        "Patient?identifier=https://example.org/mrn|12345",
    )
    .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    assert_eq!(observation_count(&server).await, 1);
}

/// `system/*.cruds` seeds an Observation coded `8867-4` about `patient`.
async fn seed_observation(server: &TestServer, id: &str, patient: &str) {
    let response = server
        .put(&format!("/Observation/{id}"))
        .add_header(AUTHORIZATION, bearer(SYSTEM))
        .json(&json!({
            "resourceType": "Observation",
            "id": id,
            "status": "final",
            "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
            "subject": {"reference": format!("Patient/{patient}")}
        }))
        .await;
    assert!(
        response.status_code().is_success(),
        "seed Observation/{id}: {}",
        response.text()
    );
}

/// #2674: a `_has` or chained parameter in the reference searches a second
/// type. A token that may search Patients but not Observations cannot learn,
/// through `Patient?_has:Observation…`, whether a patient has an Observation
/// with a given code: the hop is refused as `GET /Patient?_has:…` refuses it.
#[tokio::test]
async fn a_chain_or_has_hop_needs_a_search_grant_on_the_type_it_searches() {
    let server = server();
    seed_patient(&server, "p1", "12345").await;
    seed_observation(&server, "o1", "p1").await;
    let before = observation_count(&server).await;

    for reference in [
        "Patient?_has:Observation:subject:code=8867-4&identifier=https://example.org/mrn|12345",
        "Patient?_has:Observation:subject:code=0000-0&identifier=https://example.org/mrn|12345",
        "Patient?organization.name=Acme",
    ] {
        let response = post_transaction(&server, OBS_CREATE_PATIENT_SEARCH, reference).await;
        assert_eq!(
            response.status_code(),
            StatusCode::FORBIDDEN,
            "{reference}: {}",
            response.text()
        );
        assert!(
            response.text().contains("chained or _has search through"),
            "{reference}: {}",
            response.text()
        );
    }
    assert_eq!(
        observation_count(&server).await,
        before,
        "nothing was written"
    );
}

#[tokio::test]
async fn an_unrestricted_caller_resolves_a_has_reference() {
    let server = server();
    seed_patient(&server, "p1", "12345").await;
    seed_patient(&server, "p2", "67890").await;
    seed_observation(&server, "o1", "p1").await;

    let response = post_transaction(
        &server,
        SYSTEM,
        "Patient?_has:Observation:subject:code=8867-4",
    )
    .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    let location = response.json::<Value>()["entry"][0]["response"]["location"]
        .as_str()
        .expect("the created Observation's location")
        .split("/_history")
        .next()
        .unwrap()
        .to_string();
    let created: Value = server
        .get(&format!("/{location}"))
        .add_header(AUTHORIZATION, bearer(SYSTEM))
        .await
        .json();
    assert_eq!(created["subject"]["reference"], "Patient/p1", "{created}");
}
