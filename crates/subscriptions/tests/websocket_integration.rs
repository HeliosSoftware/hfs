//! Integration tests for WebSocket subscription channel.

use chrono::Utc;
use helios_fhir::FhirVersion;
use helios_persistence::tenant::TenantId;
use helios_subscriptions::manager::SubscriptionStatusCode;
use helios_subscriptions::{
    ResourceEvent, ResourceEventType, SubscriptionConfig, SubscriptionEngine,
};
use serde_json::{Value, json};

const TENANT_ID: &str = "tenant-ws";
const TOPIC_URL: &str = "http://example.org/topic/encounter-start";

#[allow(unreachable_code)]
fn current_fhir_version() -> FhirVersion {
    #[cfg(feature = "R4")]
    {
        return FhirVersion::R4;
    }
    #[cfg(all(not(feature = "R4"), feature = "R4B"))]
    {
        return FhirVersion::R4B;
    }
    #[cfg(all(not(feature = "R4"), not(feature = "R4B"), feature = "R5"))]
    {
        return FhirVersion::R5;
    }
    #[cfg(all(
        not(feature = "R4"),
        not(feature = "R4B"),
        not(feature = "R5"),
        feature = "R6"
    ))]
    {
        return FhirVersion::R6;
    }

    panic!("No FHIR version feature enabled");
}

fn uses_backport_ig() -> bool {
    current_fhir_version().as_str() == "R4"
}

fn expected_bundle_type() -> &'static str {
    match current_fhir_version().as_str() {
        "R5" | "R6" => "subscription-notification",
        _ => "history",
    }
}

fn topic_resource() -> Value {
    if uses_backport_ig() {
        json!({
            "resourceType": "Basic",
            "id": "topic-1",
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
            "id": "topic-1",
            "url": TOPIC_URL,
            "status": "active",
            "resourceTrigger": [{
                "resource": "Encounter",
                "supportedInteraction": ["create"]
            }]
        })
    }
}

fn ws_subscription_resource() -> Value {
    if uses_backport_ig() {
        json!({
            "resourceType": "Subscription",
            "id": "sub-ws-1",
            "status": "requested",
            "criteria": TOPIC_URL,
            "channel": {
                "type": "websocket",
                "payload": "application/fhir+json"
            }
        })
    } else {
        json!({
            "resourceType": "Subscription",
            "id": "sub-ws-1",
            "status": "requested",
            "topic": TOPIC_URL,
            "channelType": { "code": "websocket" },
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
        resource_id: "topic-1".to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(resource),
        previous_resource: None,
        timestamp: Utc::now(),
    }
}

fn ws_subscription_create_event() -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(TENANT_ID),
        fhir_version: current_fhir_version(),
        resource_type: "Subscription".to_string(),
        resource_id: "sub-ws-1".to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(ws_subscription_resource()),
        previous_resource: None,
        timestamp: Utc::now(),
    }
}

fn encounter_create_event_with_id(resource_id: &str) -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(TENANT_ID),
        fhir_version: current_fhir_version(),
        resource_type: "Encounter".to_string(),
        resource_id: resource_id.to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(json!({
            "resourceType": "Encounter",
            "id": resource_id,
            "status": "in-progress"
        })),
        previous_resource: None,
        timestamp: Utc::now(),
    }
}

fn encounter_create_event() -> ResourceEvent {
    encounter_create_event_with_id("enc-1")
}

fn status_entry(bundle: &Value) -> Option<&Value> {
    bundle.get("entry")?.as_array()?.first()
}

fn status_resource(bundle: &Value) -> Option<&Value> {
    status_entry(bundle)?.get("resource")
}

fn backport_parameter<'a>(bundle: &'a Value, name: &str) -> Option<&'a Value> {
    let status_resource = status_resource(bundle)?;
    if status_resource.get("resourceType")?.as_str()? != "Parameters" {
        return None;
    }

    status_resource
        .get("parameter")?
        .as_array()?
        .iter()
        .find(|parameter| parameter.get("name").and_then(Value::as_str) == Some(name))
}

fn backport_value_code<'a>(bundle: &'a Value, name: &str) -> Option<&'a str> {
    backport_parameter(bundle, name)?.get("valueCode")?.as_str()
}

fn backport_value_string<'a>(bundle: &'a Value, name: &str) -> Option<&'a str> {
    backport_parameter(bundle, name)?
        .get("valueString")?
        .as_str()
}

fn backport_value_reference<'a>(bundle: &'a Value, name: &str) -> Option<&'a str> {
    backport_parameter(bundle, name)?
        .get("valueReference")?
        .get("reference")?
        .as_str()
}

fn backport_value_canonical<'a>(bundle: &'a Value, name: &str) -> Option<&'a str> {
    backport_parameter(bundle, name)?
        .get("valueCanonical")?
        .as_str()
}

fn backport_notification_event_part<'a>(bundle: &'a Value, name: &str) -> Option<&'a Value> {
    backport_parameter(bundle, "notification-event")?
        .get("part")?
        .as_array()?
        .iter()
        .find(|part| part.get("name").and_then(Value::as_str) == Some(name))
}

fn backport_event_number(bundle: &Value) -> Option<&str> {
    backport_notification_event_part(bundle, "event-number")?
        .get("valueString")?
        .as_str()
}

fn backport_focus_reference(bundle: &Value) -> Option<&str> {
    backport_notification_event_part(bundle, "focus")?
        .get("valueReference")?
        .get("reference")?
        .as_str()
}

fn notification_type(bundle: &Value) -> Option<&str> {
    let status = status_resource(bundle)?;
    match status.get("resourceType")?.as_str()? {
        "Parameters" => backport_value_code(bundle, "type"),
        "SubscriptionStatus" => status.get("type")?.as_str(),
        _ => None,
    }
}

fn subscription_reference(bundle: &Value) -> Option<&str> {
    let status = status_resource(bundle)?;
    match status.get("resourceType")?.as_str()? {
        "Parameters" => backport_value_reference(bundle, "subscription"),
        "SubscriptionStatus" => status.get("subscription")?.get("reference")?.as_str(),
        _ => None,
    }
}

fn topic_value(bundle: &Value) -> Option<&str> {
    let status = status_resource(bundle)?;
    match status.get("resourceType")?.as_str()? {
        "Parameters" => backport_value_canonical(bundle, "topic"),
        "SubscriptionStatus" => status.get("topic")?.as_str(),
        _ => None,
    }
}

fn status_code(bundle: &Value) -> Option<&str> {
    let status = status_resource(bundle)?;
    match status.get("resourceType")?.as_str()? {
        "Parameters" => backport_value_code(bundle, "status"),
        "SubscriptionStatus" => status.get("status")?.as_str(),
        _ => None,
    }
}

fn events_since_start(bundle: &Value) -> Option<u64> {
    let status = status_resource(bundle)?;
    match status.get("resourceType")?.as_str()? {
        "Parameters" => backport_value_string(bundle, "events-since-subscription-start")?
            .parse()
            .ok(),
        "SubscriptionStatus" => status.get("eventsSinceSubscriptionStart")?.as_u64(),
        _ => None,
    }
}

fn notification_event_number(bundle: &Value) -> Option<u64> {
    let status = status_resource(bundle)?;
    match status.get("resourceType")?.as_str()? {
        "Parameters" => backport_event_number(bundle)?.parse().ok(),
        "SubscriptionStatus" => status
            .get("notificationEvent")?
            .as_array()?
            .first()?
            .get("eventNumber")?
            .as_u64(),
        _ => None,
    }
}

fn notification_focus_reference(bundle: &Value) -> Option<&str> {
    let status = status_resource(bundle)?;
    match status.get("resourceType")?.as_str()? {
        "Parameters" => backport_focus_reference(bundle),
        "SubscriptionStatus" => status
            .get("notificationEvent")?
            .as_array()?
            .first()?
            .get("focus")?
            .get("reference")?
            .as_str(),
        _ => None,
    }
}

fn make_engine() -> SubscriptionEngine {
    let config = SubscriptionConfig {
        max_retries: 1,
        supported_channel_types: vec!["rest-hook".to_string(), "websocket".to_string()],
        ..Default::default()
    };
    SubscriptionEngine::new(config, "http://localhost:8080".to_string())
}

#[tokio::test]
async fn websocket_subscription_activates_immediately() {
    let engine = make_engine();
    engine.on_resource_event(topic_create_event()).await;
    engine
        .on_resource_event(ws_subscription_create_event())
        .await;

    let sub = engine
        .manager()
        .get_subscription(TENANT_ID, "sub-ws-1")
        .expect("subscription should be registered");

    // WebSocket handshake is a no-op, so subscription goes straight to Active.
    assert_eq!(sub.status, SubscriptionStatusCode::Active);
}

#[tokio::test]
async fn websocket_notification_delivered_to_connected_client() {
    let engine = make_engine();
    engine.on_resource_event(topic_create_event()).await;
    engine
        .on_resource_event(ws_subscription_create_event())
        .await;

    // Simulate a client connecting via WebSocket by registering directly.
    let (_client_id, mut rx) = engine.ws_manager().register_client(TENANT_ID, "sub-ws-1");

    // Fire an event that matches the topic.
    engine.on_resource_event(encounter_create_event()).await;

    // The client should receive a backport-style event notification bundle.
    let notification = rx.recv().await.expect("should receive notification");
    assert_eq!(notification["resourceType"], "Bundle");
    assert_eq!(notification["type"], expected_bundle_type());

    let entries = notification["entry"]
        .as_array()
        .expect("bundle entry should be an array");
    assert_eq!(
        entries.len(),
        2,
        "id-only payload should include focus entry"
    );

    let status_entry = status_entry(&notification).expect("status entry should exist");
    assert_eq!(status_entry["request"]["method"], "GET");
    assert_eq!(
        status_entry["request"]["url"],
        "Subscription/sub-ws-1/$status"
    );
    assert_eq!(status_entry["response"]["status"], "200");

    assert_eq!(
        subscription_reference(&notification),
        Some("Subscription/sub-ws-1")
    );
    assert_eq!(topic_value(&notification), Some(TOPIC_URL));
    assert_eq!(status_code(&notification), Some("active"));
    assert_eq!(notification_type(&notification), Some("event-notification"));
    assert_eq!(events_since_start(&notification), Some(1));
    assert_eq!(notification_event_number(&notification), Some(1));
    assert_eq!(
        notification_focus_reference(&notification),
        Some("Encounter/enc-1")
    );

    let payload_entry = &entries[1];
    assert_eq!(payload_entry["request"]["method"], "GET");
    assert_eq!(payload_entry["request"]["url"], "Encounter/enc-1");
    assert_eq!(
        payload_entry["fullUrl"],
        "http://localhost:8080/Encounter/enc-1"
    );
    assert!(
        payload_entry.get("resource").is_none(),
        "id-only payload should not include full resource"
    );

    // Verify event count incremented.
    let sub = engine
        .manager()
        .get_subscription(TENANT_ID, "sub-ws-1")
        .unwrap();
    assert_eq!(sub.events_since_start, 1);
}

#[tokio::test]
async fn websocket_notification_broadcast_to_multiple_clients() {
    let engine = make_engine();
    engine.on_resource_event(topic_create_event()).await;
    engine
        .on_resource_event(ws_subscription_create_event())
        .await;

    let (_id1, mut rx1) = engine.ws_manager().register_client(TENANT_ID, "sub-ws-1");
    let (_id2, mut rx2) = engine.ws_manager().register_client(TENANT_ID, "sub-ws-1");

    engine.on_resource_event(encounter_create_event()).await;

    let n1 = rx1.recv().await.expect("client 1 should receive");
    let n2 = rx2.recv().await.expect("client 2 should receive");
    assert_eq!(n1["resourceType"], "Bundle");
    assert_eq!(n2["resourceType"], "Bundle");
    assert_eq!(n1["type"], expected_bundle_type());
    assert_eq!(n2["type"], expected_bundle_type());
    assert_eq!(notification_type(&n1), Some("event-notification"));
    assert_eq!(notification_type(&n2), Some("event-notification"));
    assert_eq!(events_since_start(&n1), Some(1));
    assert_eq!(events_since_start(&n2), Some(1));
    assert_eq!(notification_event_number(&n1), Some(1));
    assert_eq!(notification_event_number(&n2), Some(1));
    assert_eq!(notification_focus_reference(&n1), Some("Encounter/enc-1"));
    assert_eq!(notification_focus_reference(&n2), Some("Encounter/enc-1"));
}

#[tokio::test]
async fn websocket_event_number_and_counter_are_monotonic() {
    let engine = make_engine();
    engine.on_resource_event(topic_create_event()).await;
    engine
        .on_resource_event(ws_subscription_create_event())
        .await;

    let (_client_id, mut rx) = engine.ws_manager().register_client(TENANT_ID, "sub-ws-1");

    engine
        .on_resource_event(encounter_create_event_with_id("enc-1"))
        .await;
    let first = rx
        .recv()
        .await
        .expect("first event notification should arrive");

    engine
        .on_resource_event(encounter_create_event_with_id("enc-2"))
        .await;
    let second = rx
        .recv()
        .await
        .expect("second event notification should arrive");

    assert_eq!(events_since_start(&first), Some(1));
    assert_eq!(events_since_start(&second), Some(2));
    assert_eq!(notification_event_number(&first), Some(1));
    assert_eq!(notification_event_number(&second), Some(2));
    assert_eq!(
        notification_focus_reference(&first),
        Some("Encounter/enc-1")
    );
    assert_eq!(
        notification_focus_reference(&second),
        Some("Encounter/enc-2")
    );

    let sub = engine
        .manager()
        .get_subscription(TENANT_ID, "sub-ws-1")
        .expect("subscription should still be registered");
    assert_eq!(sub.events_since_start, 2);
}

#[tokio::test]
async fn websocket_no_panic_with_zero_clients() {
    let engine = make_engine();
    engine.on_resource_event(topic_create_event()).await;
    engine
        .on_resource_event(ws_subscription_create_event())
        .await;

    // Fire event with no connected clients — should not panic or error.
    engine.on_resource_event(encounter_create_event()).await;

    // Subscription should still be active (not error/off).
    let sub = engine
        .manager()
        .get_subscription(TENANT_ID, "sub-ws-1")
        .unwrap();
    assert_eq!(sub.status, SubscriptionStatusCode::Active);
}

#[tokio::test]
async fn websocket_disconnected_client_does_not_cause_error() {
    let engine = make_engine();
    engine.on_resource_event(topic_create_event()).await;
    engine
        .on_resource_event(ws_subscription_create_event())
        .await;

    let (_client_id, rx) = engine.ws_manager().register_client(TENANT_ID, "sub-ws-1");

    // Drop the receiver to simulate disconnect.
    drop(rx);

    // Fire an event — should handle gracefully.
    engine.on_resource_event(encounter_create_event()).await;

    let sub = engine
        .manager()
        .get_subscription(TENANT_ID, "sub-ws-1")
        .unwrap();
    assert_eq!(sub.status, SubscriptionStatusCode::Active);
}

#[tokio::test]
async fn websocket_binding_token_lifecycle() {
    let engine = make_engine();
    engine.on_resource_event(topic_create_event()).await;
    engine
        .on_resource_event(ws_subscription_create_event())
        .await;

    // Generate a binding token.
    let (token, expiry) = engine
        .ws_token_manager()
        .generate_token(TENANT_ID, "sub-ws-1");

    assert!(!token.is_empty());
    assert!(expiry > Utc::now());

    // Validate and consume the token.
    let result = engine.ws_token_manager().validate_and_consume(&token);
    assert!(result.is_some());
    let (tenant, sub_id) = result.unwrap();
    assert_eq!(tenant, TENANT_ID);
    assert_eq!(sub_id, "sub-ws-1");

    // Token is single-use.
    assert!(
        engine
            .ws_token_manager()
            .validate_and_consume(&token)
            .is_none()
    );
}

#[tokio::test]
async fn websocket_subscription_delete_closes_clients() {
    let engine = make_engine();
    engine.on_resource_event(topic_create_event()).await;
    engine
        .on_resource_event(ws_subscription_create_event())
        .await;

    let (_client_id, mut rx) = engine.ws_manager().register_client(TENANT_ID, "sub-ws-1");

    // Delete the subscription.
    let delete_event = ResourceEvent {
        tenant_id: TenantId::new(TENANT_ID),
        fhir_version: current_fhir_version(),
        resource_type: "Subscription".to_string(),
        resource_id: "sub-ws-1".to_string(),
        version_id: "2".to_string(),
        event_type: ResourceEventType::Delete,
        resource: None,
        previous_resource: None,
        timestamp: Utc::now(),
    };

    engine.on_resource_event(delete_event).await;

    // Subscription should be gone.
    assert!(
        engine
            .manager()
            .get_subscription(TENANT_ID, "sub-ws-1")
            .is_none()
    );

    // Client receiver should be closed (channel dropped).
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn websocket_tenant_isolation() {
    let engine = make_engine();
    engine.on_resource_event(topic_create_event()).await;
    engine
        .on_resource_event(ws_subscription_create_event())
        .await;

    // Connect a client for TENANT_ID.
    let (_client_id, mut rx) = engine.ws_manager().register_client(TENANT_ID, "sub-ws-1");

    // Fire an event from a different tenant.
    let other_tenant_event = ResourceEvent {
        tenant_id: TenantId::new("other-tenant"),
        fhir_version: current_fhir_version(),
        resource_type: "Encounter".to_string(),
        resource_id: "enc-2".to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(json!({
            "resourceType": "Encounter",
            "id": "enc-2",
            "status": "in-progress"
        })),
        previous_resource: None,
        timestamp: Utc::now(),
    };

    engine.on_resource_event(other_tenant_event).await;

    // Client should NOT have received anything (tenant isolation).
    assert!(rx.try_recv().is_err());
}
