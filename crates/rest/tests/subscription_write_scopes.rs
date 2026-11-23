//! A Subscription delivers every later write of the resource types its topic
//! names to the endpoint its writer chose, so a Subscription or topic write is
//! judged by the writer's Search grant on each of those types, as every other
//! indirect read path is, on every write path: create, update, patch and the
//! bundle entries.
#![cfg(feature = "subscriptions")]

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum_test::{TestResponse, TestServer};
use chrono::Utc;
use helios_audit::ExclusionFilter;
use helios_auth::{AuthConfig, AuthError, AuthProvider, Principal, ScopeSet};
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::tenant::TenantId;
use helios_rest::{AppState, AuthMiddlewareState, ServerConfig};
use helios_subscriptions::{
    ResourceEvent, ResourceEventType, SubscriptionConfig, SubscriptionEngine,
};
use serde_json::{Value, json};

/// `ServerConfig::for_testing().default_tenant`.
const TENANT: &str = "test-tenant";
const TOPIC_URL: &str = "http://example.org/topics/encounter-start";
const UNKNOWN_TOPIC_URL: &str = "http://example.org/topics/unknown";

/// Writes Subscriptions, topics and Encounters; may not read or search an
/// Encounter.
const WRITER: &str = "writer-token";
/// The same grants plus `user/Encounter.rs`.
const READER: &str = "reader-token";

const WRITER_SCOPES: &str =
    "user/Subscription.cruds user/SubscriptionTopic.cruds user/Basic.cruds user/Encounter.c";

struct StubProvider;

#[async_trait]
impl AuthProvider for StubProvider {
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError> {
        let scopes = if authorization_header == format!("Bearer {WRITER}") {
            WRITER_SCOPES.to_string()
        } else if authorization_header == format!("Bearer {READER}") {
            format!("{WRITER_SCOPES} user/Encounter.rs")
        } else {
            return Err(AuthError::InvalidSignature);
        };
        Ok(Principal {
            subject: "demo-sub".to_string(),
            issuer: "https://idp.example.com".to_string(),
            scopes: ScopeSet::parse(&scopes),
            expires_at: Utc::now() + chrono::Duration::minutes(5),
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

#[cfg(feature = "R4")]
fn backport(version: FhirVersion) -> bool {
    version == FhirVersion::R4
}

#[cfg(not(feature = "R4"))]
fn backport(_version: FhirVersion) -> bool {
    false
}

fn mime(version: FhirVersion) -> HeaderValue {
    let fhir_version = match version.as_str() {
        "R4" => "4.0",
        "R4B" => "4.3",
        "R5" => "5.0",
        _ => "6.0",
    };
    HeaderValue::from_str(&format!(
        "application/fhir+json; fhirVersion={fhir_version}"
    ))
    .unwrap()
}

fn topic(version: FhirVersion) -> Value {
    if backport(version) {
        json!({
            "resourceType": "Basic", "id": "topic-1",
            "code": {"coding": [{"system": "http://hl7.org/fhir/fhir-types", "code": "SubscriptionTopic"}]},
            "extension": [{
                "url": "http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.url",
                "valueUri": TOPIC_URL
            }, {
                "url": "http://hl7.org/fhir/4.3/StructureDefinition/extension-SubscriptionTopic.resourceTrigger",
                "extension": [
                    {"url": "resource", "valueUri": "http://hl7.org/fhir/StructureDefinition/Encounter"},
                    {"url": "supportedInteraction", "valueCode": "create"}
                ]
            }]
        })
    } else {
        json!({
            "resourceType": "SubscriptionTopic", "id": "topic-1", "url": TOPIC_URL,
            "status": "active",
            "resourceTrigger": [{"resource": "Encounter", "supportedInteraction": ["create"]}]
        })
    }
}

fn subscription(version: FhirVersion, topic_url: &str) -> Value {
    if backport(version) {
        json!({
            "resourceType": "Subscription", "status": "requested",
            "criteria": topic_url,
            "channel": {"type": "rest-hook", "endpoint": "https://receiver.example/hook", "payload": "application/fhir+json"}
        })
    } else {
        json!({
            "resourceType": "Subscription", "status": "requested",
            "topic": topic_url,
            "channelType": {
                "system": "http://terminology.hl7.org/CodeSystem/subscription-channel-type",
                "code": "rest-hook"
            },
            "endpoint": "https://receiver.example/hook",
            "contentType": "application/fhir+json", "content": "full-resource"
        })
    }
}

async fn server(version: FhirVersion) -> TestServer {
    let engine = Arc::new(SubscriptionEngine::new(
        SubscriptionConfig {
            allow_private_endpoints: true,
            ..Default::default()
        },
        "https://public.example/fhir".to_string(),
    ));
    let topic = topic(version);
    engine
        .on_resource_event(ResourceEvent {
            tenant_id: TenantId::new(TENANT),
            fhir_version: version,
            resource_type: topic["resourceType"].as_str().unwrap().to_string(),
            resource_id: "topic-1".to_string(),
            version_id: "1".to_string(),
            event_type: ResourceEventType::Create,
            resource: Some(topic),
            previous_resource: None,
            timestamp: Utc::now(),
        })
        .await;

    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"));
    let backend = Arc::new(
        SqliteBackend::with_config(
            ":memory:",
            SqliteBackendConfig {
                data_dir,
                ..Default::default()
            },
        )
        .unwrap(),
    );
    backend.init_schema().unwrap();
    let state =
        AppState::new(backend, ServerConfig::for_testing()).with_subscription_engine(engine);
    let auth = Arc::new(AuthMiddlewareState {
        provider: Arc::new(StubProvider),
        config: Arc::new(AuthConfig::default()),
        audit_sink: Arc::new(helios_audit::sinks::NullSink),
        audit_source_observer: "Device/test".to_string(),
        audit_exclusion_filter: ExclusionFilter::new(vec![]),
        tenant_url_routing: false,
        sessions: None,
    });
    let app = helios_rest::routing::fhir_routes::create_routes(state)
        .layer(axum::middleware::from_fn_with_state(
            auth.clone(),
            helios_rest::middleware::auth::authz_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            auth,
            helios_rest::middleware::auth::auth_middleware,
        ));
    TestServer::new(app).unwrap()
}

async fn post(
    server: &TestServer,
    version: FhirVersion,
    token: &str,
    path: &str,
    body: Value,
) -> TestResponse {
    server
        .post(path)
        .add_header(AUTHORIZATION, bearer(token))
        .add_header(header::CONTENT_TYPE, mime(version))
        .bytes(body.to_string().into())
        .await
}

async fn put(
    server: &TestServer,
    version: FhirVersion,
    token: &str,
    path: &str,
    body: Value,
) -> TestResponse {
    server
        .put(path)
        .add_header(AUTHORIZATION, bearer(token))
        .add_header(header::CONTENT_TYPE, mime(version))
        .bytes(body.to_string().into())
        .await
}

async fn json_patch(
    server: &TestServer,
    version: FhirVersion,
    token: &str,
    path: &str,
) -> TestResponse {
    server
        .patch(path)
        .add_header(AUTHORIZATION, bearer(token))
        .add_header(header::ACCEPT, mime(version))
        .add_header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json-patch+json"),
        )
        .bytes(
            json!([{"op": "add", "path": "/reason", "value": "audit"}])
                .to_string()
                .into(),
        )
        .await
}

fn fhirpath_add_reason() -> Value {
    json!({"resourceType": "Parameters", "parameter": [{"name": "operation", "part": [
        {"name": "type", "valueCode": "add"},
        {"name": "path", "valueString": "Subscription"},
        {"name": "name", "valueString": "reason"},
        {"name": "value", "valueString": "audit"}
    ]}]})
}

fn bundle(kind: &str, entries: Vec<Value>) -> Value {
    json!({"resourceType": "Bundle", "type": kind, "entry": entries})
}

fn assert_refused_for(response: TestResponse, resource_type: &str) {
    assert_eq!(
        response.status_code(),
        StatusCode::FORBIDDEN,
        "{}",
        response.text()
    );
    let text = response.text();
    assert!(text.contains(resource_type), "{text}");
}

fn entry_status(body: &Value, index: usize) -> String {
    body["entry"][index]["response"]["status"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

async fn a_subscription_needs_a_search_grant_on_every_type_the_topic_names(version: FhirVersion) {
    let server = server(version).await;

    let refused = post(
        &server,
        version,
        WRITER,
        "/Subscription",
        subscription(version, TOPIC_URL),
    )
    .await;
    assert_refused_for(refused, "Encounter");
    let created = post(
        &server,
        version,
        READER,
        "/Subscription",
        subscription(version, TOPIC_URL),
    )
    .await;
    assert_eq!(
        created.status_code(),
        StatusCode::CREATED,
        "{}",
        created.text()
    );

    let mut update = subscription(version, TOPIC_URL);
    update["id"] = json!("s1");
    let refused = put(&server, version, WRITER, "/Subscription/s1", update.clone()).await;
    assert_refused_for(refused, "Encounter");
    let created = put(&server, version, READER, "/Subscription/s1", update).await;
    assert_eq!(
        created.status_code(),
        StatusCode::CREATED,
        "{}",
        created.text()
    );
}

async fn a_patch_is_judged_on_the_patched_document(version: FhirVersion) {
    let server = server(version).await;
    let mut resource = subscription(version, TOPIC_URL);
    resource["id"] = json!("s2");
    let created = put(&server, version, READER, "/Subscription/s2", resource).await;
    assert_eq!(
        created.status_code(),
        StatusCode::CREATED,
        "{}",
        created.text()
    );

    let refused = json_patch(&server, version, WRITER, "/Subscription/s2").await;
    assert_refused_for(refused, "Encounter");
    let patched = json_patch(&server, version, READER, "/Subscription/s2").await;
    assert_eq!(patched.status_code(), StatusCode::OK, "{}", patched.text());
}

async fn a_topic_write_is_judged_on_its_triggers(version: FhirVersion) {
    let server = server(version).await;
    let mut topic = topic(version);
    let path = format!("/{}", topic["resourceType"].as_str().unwrap());
    topic.as_object_mut().unwrap().remove("id");

    let refused = post(&server, version, WRITER, &path, topic.clone()).await;
    assert_refused_for(refused, "Encounter");
    let created = post(&server, version, READER, &path, topic).await;
    assert_eq!(
        created.status_code(),
        StatusCode::CREATED,
        "{}",
        created.text()
    );

    if backport(version) {
        let plain = json!({"resourceType": "Basic", "code": {"coding": [{"system": "http://example.org/kinds", "code": "note"}]}});
        let created = post(&server, version, WRITER, "/Basic", plain).await;
        assert_eq!(
            created.status_code(),
            StatusCode::CREATED,
            "{}",
            created.text()
        );
    }
}

async fn a_subscription_on_an_unknown_topic_is_refused(version: FhirVersion) {
    let server = server(version).await;
    let refused = post(
        &server,
        version,
        READER,
        "/Subscription",
        subscription(version, UNKNOWN_TOPIC_URL),
    )
    .await;
    assert_refused_for(refused, UNKNOWN_TOPIC_URL);
}

async fn bundle_entries_are_judged_too(version: FhirVersion) {
    let server = server(version).await;

    let post_entry = json!({"request": {"method": "POST", "url": "Subscription"}, "resource": subscription(version, TOPIC_URL)});
    let refused = post(
        &server,
        version,
        WRITER,
        "/",
        bundle("transaction", vec![post_entry.clone()]),
    )
    .await;
    assert_refused_for(refused, "Encounter");

    let batch = post(
        &server,
        version,
        WRITER,
        "/",
        bundle("batch", vec![post_entry.clone()]),
    )
    .await;
    assert_eq!(batch.status_code(), StatusCode::OK, "{}", batch.text());
    let body: Value = batch.json();
    assert!(entry_status(&body, 0).starts_with("403"), "{body}");

    let committed = post(
        &server,
        version,
        READER,
        "/",
        bundle("transaction", vec![post_entry]),
    )
    .await;
    assert_eq!(
        committed.status_code(),
        StatusCode::OK,
        "{}",
        committed.text()
    );

    let mut resource = subscription(version, TOPIC_URL);
    resource["id"] = json!("s3");
    let created = put(&server, version, READER, "/Subscription/s3", resource).await;
    assert_eq!(
        created.status_code(),
        StatusCode::CREATED,
        "{}",
        created.text()
    );
    let patch_entry = json!({"request": {"method": "PATCH", "url": "Subscription/s3"}, "resource": fhirpath_add_reason()});
    let refused = post(
        &server,
        version,
        WRITER,
        "/",
        bundle("transaction", vec![patch_entry.clone()]),
    )
    .await;
    assert_refused_for(refused, "Encounter");
    let batch = post(
        &server,
        version,
        WRITER,
        "/",
        bundle("batch", vec![patch_entry.clone()]),
    )
    .await;
    assert_eq!(batch.status_code(), StatusCode::OK, "{}", batch.text());
    let body: Value = batch.json();
    assert!(entry_status(&body, 0).starts_with("403"), "{body}");
    let patched = post(
        &server,
        version,
        READER,
        "/",
        bundle("transaction", vec![patch_entry]),
    )
    .await;
    assert_eq!(patched.status_code(), StatusCode::OK, "{}", patched.text());
}

macro_rules! for_version {
    ($version:expr, $module:ident) => {
        mod $module {
            use super::*;

            #[tokio::test]
            async fn a_subscription_needs_a_search_grant_on_every_type_the_topic_names() {
                super::a_subscription_needs_a_search_grant_on_every_type_the_topic_names($version)
                    .await;
            }

            #[tokio::test]
            async fn a_patch_is_judged_on_the_patched_document() {
                super::a_patch_is_judged_on_the_patched_document($version).await;
            }

            #[tokio::test]
            async fn a_topic_write_is_judged_on_its_triggers() {
                super::a_topic_write_is_judged_on_its_triggers($version).await;
            }

            #[tokio::test]
            async fn a_subscription_on_an_unknown_topic_is_refused() {
                super::a_subscription_on_an_unknown_topic_is_refused($version).await;
            }

            #[tokio::test]
            async fn bundle_entries_are_judged_too() {
                super::bundle_entries_are_judged_too($version).await;
            }
        }
    };
}

#[cfg(feature = "R4")]
for_version!(FhirVersion::R4, r4);

#[cfg(feature = "R5")]
for_version!(FhirVersion::R5, r5);
