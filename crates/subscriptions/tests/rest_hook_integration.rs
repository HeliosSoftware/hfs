use chrono::Utc;
use helios_fhir::FhirVersion;
use helios_persistence::tenant::TenantId;
use helios_subscriptions::manager::SubscriptionStatusCode;
use helios_subscriptions::{
    ResourceEvent, ResourceEventType, SubscriptionConfig, SubscriptionEngine, SubscriptionError,
};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const TENANT_ID: &str = "tenant-a";
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

fn rest_hook_subscription_resource(endpoint: &str) -> Value {
    json!({
        "resourceType": "Subscription",
        "id": "sub-rest-hook",
        "status": "requested",
        "criteria": TOPIC_URL,
        "channel": {
            "type": "rest-hook",
            "endpoint": endpoint,
            "payload": "application/fhir+json",
            "header": ["Authorization: Bearer integration-token"]
        }
    })
}

fn message_channel_subscription_resource(endpoint: &str) -> Value {
    json!({
        "resourceType": "Subscription",
        "id": "sub-message",
        "status": "requested",
        "criteria": TOPIC_URL,
        "channel": {
            "type": "message",
            "endpoint": endpoint
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

fn subscription_create_event(subscription_resource: Value) -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(TENANT_ID),
        fhir_version: FhirVersion::default(),
        resource_type: "Subscription".to_string(),
        resource_id: "sub-rest-hook".to_string(),
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

fn header_value<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
}

fn notification_type_from_backport_bundle(bundle: &Value) -> Option<&str> {
    let status_resource = bundle.get("entry")?.get(0)?.get("resource")?;
    if status_resource.get("resourceType")?.as_str()? != "Parameters" {
        return None;
    }

    status_resource
        .get("parameter")?
        .as_array()?
        .iter()
        .find_map(|parameter| {
            let name = parameter.get("name")?.as_str()?;
            if name == "type" {
                parameter.get("valueCode")?.as_str()
            } else {
                None
            }
        })
}

#[tokio::test]
async fn rest_hook_handshake_and_event_notifications_follow_backport_flow() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webhook"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let config = SubscriptionConfig {
        max_retries: 1,
        ..Default::default()
    };
    let engine = SubscriptionEngine::new(config, "http://localhost:8080".to_string());

    engine.on_resource_event(topic_create_event()).await;

    let endpoint = format!("{}/webhook", server.uri());
    engine
        .on_resource_event(subscription_create_event(rest_hook_subscription_resource(
            &endpoint,
        )))
        .await;

    let registered = engine
        .manager()
        .get_subscription(TENANT_ID, "sub-rest-hook")
        .expect("subscription should be registered");
    assert_eq!(registered.status, SubscriptionStatusCode::Active);

    engine.on_resource_event(encounter_create_event()).await;

    let requests = server
        .received_requests()
        .await
        .expect("should retrieve requests from mock server");
    assert_eq!(requests.len(), 2, "expected handshake + event notification");

    let mut observed_types = Vec::new();
    for request in &requests {
        assert_eq!(request.method.as_str(), "POST");
        assert_eq!(request.url.path(), "/webhook");
        assert_eq!(
            header_value(request, "content-type"),
            Some("application/fhir+json")
        );
        assert_eq!(
            header_value(request, "authorization"),
            Some("Bearer integration-token")
        );

        let body: Value =
            serde_json::from_slice(&request.body).expect("request body should be JSON");
        assert_eq!(body["resourceType"], "Bundle");
        assert_eq!(body["type"], "history");

        let notification_type = notification_type_from_backport_bundle(&body)
            .expect("backport bundle should include a Parameters.type value");
        observed_types.push(notification_type.to_string());

        if notification_type == "event-notification" {
            let entries = body["entry"]
                .as_array()
                .expect("bundle entry should be an array");
            assert_eq!(
                entries.len(),
                2,
                "id-only payload should include focus entry"
            );
            assert_eq!(entries[1]["request"]["url"], "Encounter/enc-1");
        }
    }

    observed_types.sort();
    assert_eq!(
        observed_types,
        vec!["event-notification".to_string(), "handshake".to_string()]
    );

    let updated = engine
        .manager()
        .get_subscription(TENANT_ID, "sub-rest-hook")
        .expect("subscription should still be available");
    assert_eq!(updated.events_since_start, 1);
}

#[tokio::test]
async fn message_channel_is_rejected_when_only_rest_hook_is_supported() {
    let engine = SubscriptionEngine::new(
        SubscriptionConfig::default(),
        "http://localhost:8080".to_string(),
    );
    engine.on_resource_event(topic_create_event()).await;

    let resource = message_channel_subscription_resource("https://example.org/fhir");
    let result =
        engine
            .manager()
            .register(TENANT_ID, "sub-message", &resource, FhirVersion::default());

    assert!(
        matches!(
            result,
            Err(SubscriptionError::UnsupportedChannel { channel_type }) if channel_type == "message"
        ),
        "expected unsupported message channel error"
    );
}
