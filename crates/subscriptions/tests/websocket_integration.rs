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

fn topic_resource() -> Value {
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

fn ws_subscription_resource() -> Value {
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
}

fn topic_create_event() -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(TENANT_ID),
        fhir_version: FhirVersion::default(),
        resource_type: "SubscriptionTopic".to_string(),
        resource_id: "topic-1".to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(topic_resource()),
        previous_resource: None,
        timestamp: Utc::now(),
    }
}

fn ws_subscription_create_event() -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(TENANT_ID),
        fhir_version: FhirVersion::default(),
        resource_type: "Subscription".to_string(),
        resource_id: "sub-ws-1".to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(ws_subscription_resource()),
        previous_resource: None,
        timestamp: Utc::now(),
    }
}

fn encounter_create_event() -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(TENANT_ID),
        fhir_version: FhirVersion::default(),
        resource_type: "Encounter".to_string(),
        resource_id: "enc-1".to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(json!({
            "resourceType": "Encounter",
            "id": "enc-1",
            "status": "in-progress"
        })),
        previous_resource: None,
        timestamp: Utc::now(),
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

    // The client should receive the notification bundle.
    let notification = rx.recv().await.expect("should receive notification");
    assert_eq!(notification["resourceType"], "Bundle");

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
        fhir_version: FhirVersion::default(),
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
        fhir_version: FhirVersion::default(),
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
