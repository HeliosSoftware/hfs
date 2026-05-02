//! Integration tests for the FHIR Messaging subscription channel.
//!
//! Mirrors the structure of `rest_hook_integration.rs`. Drives the full
//! engine pipeline (topic registration, subscription handshake, event
//! dispatch) against a `wiremock` receiver and asserts the wire format of
//! the resulting Bundle(type=message) payloads.

use std::sync::Arc;

use chrono::Utc;
use helios_auth::StaticBearerOutboundAuthProvider;
use helios_fhir::FhirVersion;
use helios_persistence::tenant::TenantId;
use helios_subscriptions::manager::SubscriptionStatusCode;
use helios_subscriptions::{
    MessagingSettings, ResourceEvent, ResourceEventType, SubscriptionConfig, SubscriptionEngine,
    SubscriptionError,
};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const TENANT_ID: &str = "tenant-msg";
const TOPIC_URL: &str = "http://example.org/topic/encounter-start";
const SOURCE_ENDPOINT: &str = "http://localhost:8080/fhir";

fn current_fhir_version() -> FhirVersion {
    match std::env::var("HFS_TEST_FHIR_VERSION").ok().as_deref() {
        #[cfg(feature = "R4B")]
        Some("R4B") => FhirVersion::R4B,
        #[cfg(feature = "R5")]
        Some("R5") => FhirVersion::R5,
        #[cfg(feature = "R6")]
        Some("R6") => FhirVersion::R6,
        _ => FhirVersion::default(),
    }
}

fn uses_backport_ig() -> bool {
    current_fhir_version() == FhirVersion::R4
}

fn topic_resource() -> Value {
    if uses_backport_ig() {
        json!({
            "resourceType": "Basic",
            "id": "topic-msg",
            "code": {
                "coding": [{
                    "system": "http://hl7.org/fhir/fhir-types",
                    "code": "SubscriptionTopic"
                }]
            },
            "extension": [{
                "url": "http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.url",
                "valueUri": TOPIC_URL
            }, {
                "url": "http://hl7.org/fhir/4.3/StructureDefinition/extension-SubscriptionTopic.resourceTrigger",
                "extension": [{
                    "url": "resource",
                    "valueUri": "http://hl7.org/fhir/StructureDefinition/Encounter"
                }, {
                    "url": "supportedInteraction",
                    "valueCode": "create"
                }]
            }]
        })
    } else {
        json!({
            "resourceType": "SubscriptionTopic",
            "id": "topic-msg",
            "url": TOPIC_URL,
            "status": "active",
            "resourceTrigger": [{
                "resource": "Encounter",
                "supportedInteraction": ["create"]
            }]
        })
    }
}

fn message_subscription_resource(endpoint: &str) -> Value {
    if uses_backport_ig() {
        json!({
            "resourceType": "Subscription",
            "id": "sub-msg",
            "status": "requested",
            "criteria": TOPIC_URL,
            "channel": {
                "type": "message",
                "endpoint": endpoint,
                "payload": "application/fhir+json"
            }
        })
    } else {
        json!({
            "resourceType": "Subscription",
            "id": "sub-msg",
            "status": "requested",
            "topic": TOPIC_URL,
            "channelType": {
                "system": "http://terminology.hl7.org/CodeSystem/subscription-channel-type",
                "code": "message"
            },
            "endpoint": endpoint,
            "contentType": "application/fhir+json",
            "content": "id-only"
        })
    }
}

fn topic_create_event() -> ResourceEvent {
    let resource = topic_resource();
    let resource_type = resource
        .get("resourceType")
        .and_then(Value::as_str)
        .unwrap_or("SubscriptionTopic")
        .to_string();

    ResourceEvent {
        tenant_id: TenantId::new(TENANT_ID),
        fhir_version: current_fhir_version(),
        resource_type,
        resource_id: "topic-msg".to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(resource),
        previous_resource: None,
        timestamp: Utc::now(),
    }
}

fn subscription_create_event(subscription_resource: Value) -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(TENANT_ID),
        fhir_version: current_fhir_version(),
        resource_type: "Subscription".to_string(),
        resource_id: "sub-msg".to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(subscription_resource),
        previous_resource: None,
        timestamp: Utc::now(),
    }
}

fn encounter_create_event() -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(TENANT_ID),
        fhir_version: current_fhir_version(),
        resource_type: "Encounter".to_string(),
        resource_id: "enc-msg".to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(json!({
            "resourceType": "Encounter",
            "id": "enc-msg",
            "status": "in-progress"
        })),
        previous_resource: None,
        timestamp: Utc::now(),
    }
}

fn header_value<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
}

fn message_header_event_code(message_header: &Value) -> Option<&str> {
    if let Some(uri) = message_header.get("eventUri").and_then(Value::as_str) {
        // R4 stores the topic canonical URL in eventUri; the notification
        // kind is conveyed elsewhere (Parameters status entry). Treat any
        // eventUri match as the handshake/event marker per the topic.
        if uri == TOPIC_URL {
            return Some("topic-uri");
        }
        return Some(uri);
    }
    message_header
        .get("eventCoding")
        .and_then(|c| c.get("code"))
        .and_then(Value::as_str)
}

fn engine_with_messaging_enabled() -> SubscriptionEngine {
    let config = SubscriptionConfig {
        max_retries: 1,
        supported_channel_types: vec!["rest-hook".to_string(), "message".to_string()],
        messaging: Some(MessagingSettings {
            source_endpoint: SOURCE_ENDPOINT.to_string(),
            allow_private_endpoints: true, // wiremock binds to 127.0.0.1
        }),
        ..Default::default()
    };
    SubscriptionEngine::with_outbound_auth(
        config,
        SOURCE_ENDPOINT.to_string(),
        Arc::new(StaticBearerOutboundAuthProvider::new("integration-bearer")),
    )
}

#[tokio::test]
async fn messaging_handshake_and_event_notifications_follow_expected_flow() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/process-message"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let engine = engine_with_messaging_enabled();
    engine.on_resource_event(topic_create_event()).await;

    let endpoint = format!("{}/process-message", server.uri());
    engine
        .on_resource_event(subscription_create_event(message_subscription_resource(
            &endpoint,
        )))
        .await;

    let registered = engine
        .manager()
        .get_subscription(TENANT_ID, "sub-msg")
        .expect("subscription should be registered");
    assert_eq!(registered.status, SubscriptionStatusCode::Active);

    engine.on_resource_event(encounter_create_event()).await;

    let requests = server
        .received_requests()
        .await
        .expect("should retrieve requests from mock server");
    assert_eq!(
        requests.len(),
        2,
        "expected handshake + event notification messages"
    );

    for request in &requests {
        assert_eq!(request.method.as_str(), "POST");
        assert_eq!(request.url.path(), "/process-message");
        assert_eq!(
            header_value(request, "content-type"),
            Some("application/fhir+json")
        );
        assert_eq!(
            header_value(request, "authorization"),
            Some("Bearer integration-bearer"),
            "outbound bearer token should be attached"
        );

        let body: Value =
            serde_json::from_slice(&request.body).expect("request body should be JSON");
        assert_eq!(body["resourceType"], "Bundle");
        assert_eq!(
            body["type"], "message",
            "messaging channel must produce Bundle.type = message"
        );

        let entries = body["entry"]
            .as_array()
            .expect("bundle entry should be an array");
        let header = &entries[0]["resource"];
        assert_eq!(header["resourceType"], "MessageHeader");
        assert_eq!(header["source"]["endpoint"], SOURCE_ENDPOINT);
        assert_eq!(header["destination"][0]["endpoint"], endpoint);
        let focus = header["focus"][0]["reference"]
            .as_str()
            .expect("focus reference present");
        assert!(focus.starts_with("Subscription/"));

        let event_code = message_header_event_code(header).expect("event identifier present");
        assert!(
            event_code == TOPIC_URL
                || event_code == "topic-uri"
                || event_code == "handshake"
                || event_code == "event-notification",
            "unexpected event identifier: {event_code}"
        );

        // After MessageHeader, the original notification entries follow.
        assert!(
            entries.len() >= 2,
            "expected MessageHeader + at least the SubscriptionStatus entry"
        );
    }

    let updated = engine
        .manager()
        .get_subscription(TENANT_ID, "sub-msg")
        .expect("subscription still registered after dispatch");
    assert_eq!(updated.events_since_start, 1);
}

#[tokio::test]
async fn subscription_authorization_header_takes_precedence() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/process-message"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let engine = engine_with_messaging_enabled();
    engine.on_resource_event(topic_create_event()).await;

    let endpoint = format!("{}/process-message", server.uri());
    let mut subscription_resource = message_subscription_resource(&endpoint);
    if uses_backport_ig() {
        subscription_resource["channel"]["header"] =
            json!(["Authorization: Bearer subscriber-token"]);
    } else {
        subscription_resource["parameter"] = json!([
            {"name": "Authorization", "value": "Bearer subscriber-token"}
        ]);
    }
    engine
        .on_resource_event(subscription_create_event(subscription_resource))
        .await;
    engine.on_resource_event(encounter_create_event()).await;

    let requests = server.received_requests().await.unwrap();
    assert!(!requests.is_empty(), "at least one delivery expected");

    for request in &requests {
        assert_eq!(
            header_value(request, "authorization"),
            Some("Bearer subscriber-token"),
            "subscription-supplied Authorization wins over outbound bearer"
        );
    }
}

#[tokio::test]
async fn message_channel_rejected_when_messaging_disabled() {
    // Default config does not enable messaging.
    let engine =
        SubscriptionEngine::new(SubscriptionConfig::default(), SOURCE_ENDPOINT.to_string());
    engine.on_resource_event(topic_create_event()).await;

    let result = engine.manager().register(
        TENANT_ID,
        "sub-msg-disabled",
        &message_subscription_resource("https://receiver.example.org/process-message"),
        current_fhir_version(),
    );

    assert!(
        matches!(
            &result,
            Err(SubscriptionError::UnsupportedChannel { channel_type }) if channel_type == "message"
        ),
        "expected UnsupportedChannel error, got {result:?}"
    );
}

#[tokio::test]
async fn tenant_isolation_does_not_cross_dispatch_boundaries() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/process-message"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let engine = engine_with_messaging_enabled();
    engine.on_resource_event(topic_create_event()).await;

    let endpoint = format!("{}/process-message", server.uri());
    engine
        .on_resource_event(subscription_create_event(message_subscription_resource(
            &endpoint,
        )))
        .await;

    // Fire an Encounter event from a DIFFERENT tenant — should not deliver.
    let other_event = ResourceEvent {
        tenant_id: TenantId::new("tenant-other"),
        fhir_version: current_fhir_version(),
        resource_type: "Encounter".to_string(),
        resource_id: "enc-other".to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(json!({"resourceType": "Encounter", "id": "enc-other"})),
        previous_resource: None,
        timestamp: Utc::now(),
    };
    engine.on_resource_event(other_event).await;

    let requests = server.received_requests().await.unwrap();
    // Only the handshake should have been delivered — no event notifications
    // for the other-tenant event.
    assert_eq!(
        requests.len(),
        1,
        "no event delivery should cross tenant boundaries"
    );
}
