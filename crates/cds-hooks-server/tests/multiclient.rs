//! Two EMR instances against one server: each signs with its own key, sees
//! its own catalog, may call only what it enabled, and nothing unsigned or
//! unknown gets through.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use helios_cds_hooks::{CdsResponse, DiscoveryResponse};
use helios_cds_hooks_server::clients::ClientRegistry;
use helios_cds_hooks_server::server::{AppState, build_router, catalog};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use tower::ServiceExt;

const KEY_PEM: &str = include_str!("fixtures/emr-test.key.pem");
const JWKS: &str = include_str!("fixtures/emr-test.jwks.json");
const KID: &str = "emr-test-2026";
const AUD: &str = "https://cds.example.org/cds-services";

/// Serves the test JWKS, as an EMR would at its `jku`.
async fn jwks_server() -> String {
    let app = Router::new().route(
        "/.well-known/jwks.json",
        get(|| async { Json(serde_json::from_str::<Value>(JWKS).unwrap()) }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn token(iss: &str, aud: &str, jti: &str, exp_offset: i64, kid: Option<&str>) -> String {
    let now = chrono::Utc::now().timestamp();
    let claims = json!({
        "iss": iss,
        "aud": aud,
        "jti": jti,
        "iat": now,
        "exp": now + exp_offset,
    });
    let header = Header {
        alg: Algorithm::RS256,
        kid: kid.map(str::to_string),
        ..Default::default()
    };
    encode(
        &header,
        &claims,
        &EncodingKey::from_rsa_pem(KEY_PEM.as_bytes()).unwrap(),
    )
    .unwrap()
}

fn galleri_request() -> Value {
    json!({
        "hook": "order-select",
        "hookInstance": "d1577c69-dfbe-44ad-bd63-8c585e40b1a2",
        "fhirServer": "https://fhir.hospital-a.example.org",
        "context": {
            "userId": "Practitioner/pract-123",
            "patientId": "pat-456",
            "selections": ["ServiceRequest/sr-1"],
            "draftOrders": { "resourceType": "Bundle", "type": "collection", "entry": [{
                "resource": {
                    "resourceType": "ServiceRequest", "id": "sr-1", "status": "draft", "intent": "order",
                    "subject": { "reference": "Patient/pat-456" },
                    "code": { "coding": [{ "system": "https://www.grail.com/tests", "code": "GALLERI" }] }
                }
            }]}
        }
    })
}

async fn state(jwks_base: &str) -> Arc<AppState> {
    // Two clients: hospital A signs and has Galleri on; hospital B signs and has nothing on.
    let registry = ClientRegistry::from_yaml(&format!(
        "audience: [{AUD}]\nclients:\n  - id: hospital-a\n    iss: https://emr.hospital-a.example.org\n    jku: {jwks_base}/.well-known/jwks.json\n    fhir_server: https://fhir.hospital-a.example.org\n    services: [bcbsm-galleri-crd]\n    prefetch:\n      bcbsm-galleri-crd:\n        patient: Patient/{{{{context.patientId}}}}\n  - id: hospital-b\n    iss: https://emr.hospital-b.example.org\n    jku: {jwks_base}/.well-known/jwks.json\n    services: []\n"
    ))
    .unwrap();
    Arc::new(AppState::new(catalog(), registry))
}

async fn get_discovery(app: &Router, bearer: Option<&str>) -> (StatusCode, Value) {
    let mut builder = Request::builder().uri("/cds-services");
    if let Some(t) = bearer {
        builder = builder.header("Authorization", format!("Bearer {t}"));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn post_hook(app: &Router, id: &str, bearer: &str, body: &Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/cds-services/{id}"))
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {bearer}"))
                .body(Body::from(serde_json::to_vec(body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn each_client_sees_its_own_catalog_and_only_calls_what_it_enabled() {
    let jwks = jwks_server().await;
    let app = build_router(state(&jwks).await);
    let a = "https://emr.hospital-a.example.org";
    let b = "https://emr.hospital-b.example.org";

    // Hospital A: one service, with its own prefetch template.
    let (status, body) = get_discovery(&app, Some(&token(a, AUD, "a-1", 300, Some(KID)))).await;
    assert_eq!(status, StatusCode::OK);
    let discovery: DiscoveryResponse = serde_json::from_value(body).unwrap();
    assert_eq!(discovery.services.len(), 1);
    assert_eq!(discovery.services[0].id, "bcbsm-galleri-crd");
    let prefetch = discovery.services[0].prefetch.as_ref().unwrap();
    assert_eq!(prefetch.len(), 1);
    assert_eq!(prefetch["patient"], "Patient/{{context.patientId}}");

    // Hospital B: a valid client with nothing enabled sees an empty catalog and gets 404 on invoke.
    let (status, body) = get_discovery(&app, Some(&token(b, AUD, "b-1", 300, Some(KID)))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["services"].as_array().unwrap().len(), 0);
    let (status, _) = post_hook(
        &app,
        "bcbsm-galleri-crd",
        &token(b, AUD, "b-2", 300, Some(KID)),
        &galleri_request(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Hospital A invokes and gets the card.
    let (status, body) = post_hook(
        &app,
        "bcbsm-galleri-crd",
        &token(a, AUD, "a-2", 300, Some(KID)),
        &galleri_request(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let response: CdsResponse = serde_json::from_value(body).unwrap();
    assert_eq!(response.cards.len(), 1);
}

#[tokio::test]
async fn unsigned_unknown_expired_replayed_and_misaddressed_tokens_are_refused() {
    let jwks = jwks_server().await;
    let app = build_router(state(&jwks).await);
    let a = "https://emr.hospital-a.example.org";

    // No token at all, and no anonymous client: 401 with the challenge.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/cds-services")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.headers()["www-authenticate"], "Bearer");

    // A valid signature from an issuer the registry does not know.
    let (status, _) = get_discovery(
        &app,
        Some(&token(
            "https://emr.stranger.example.org",
            AUD,
            "s-1",
            300,
            Some(KID),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Expired.
    let (status, _) = get_discovery(&app, Some(&token(a, AUD, "a-old", -600, Some(KID)))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // For someone else (wrong aud).
    let (status, _) = get_discovery(
        &app,
        Some(&token(
            a,
            "https://other.example.org",
            "a-aud",
            300,
            Some(KID),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A key the client's jku does not publish.
    let (status, _) = get_discovery(&app, Some(&token(a, AUD, "a-kid", 300, Some("nope")))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = get_discovery(&app, Some(&token(a, AUD, "a-nokid", 300, None))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // The same jti twice.
    let replayed = token(a, AUD, "a-once", 300, Some(KID));
    let (first, _) = get_discovery(&app, Some(&replayed)).await;
    let (second, _) = get_discovery(&app, Some(&replayed)).await;
    assert_eq!(first, StatusCode::OK);
    assert_eq!(second, StatusCode::UNAUTHORIZED);

    // Hospital A's token naming another FHIR server.
    let mut foreign = galleri_request();
    foreign["fhirServer"] = json!("https://fhir.hospital-b.example.org");
    let (status, _) = post_hook(
        &app,
        "bcbsm-galleri-crd",
        &token(a, AUD, "a-foreign", 300, Some(KID)),
        &foreign,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_development_registry_accepts_unsigned_calls_as_dev() {
    let catalog = catalog();
    let registry = ClientRegistry::development(&catalog.ids());
    let app = build_router(Arc::new(AppState::new(catalog, registry)));
    let (status, body) = get_discovery(&app, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["services"][0]["id"], "bcbsm-galleri-crd");
}
