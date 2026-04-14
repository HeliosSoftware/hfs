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

fn rest_hook_subscription_resource(endpoint: &str) -> Value {
    if uses_backport_ig() {
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
    } else {
        json!({
            "resourceType": "Subscription",
            "id": "sub-rest-hook",
            "status": "requested",
            "topic": TOPIC_URL,
            "channelType": {
                "system": "http://terminology.hl7.org/CodeSystem/subscription-channel-type",
                "code": "rest-hook"
            },
            "endpoint": endpoint,
            "contentType": "application/fhir+json",
            "content": "id-only",
            "parameter": [{
                "name": "Authorization",
                "value": "Bearer integration-token"
            }]
        })
    }
}

fn message_channel_subscription_resource(endpoint: &str) -> Value {
    if uses_backport_ig() {
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
    } else {
        json!({
            "resourceType": "Subscription",
            "id": "sub-message",
            "status": "requested",
            "topic": TOPIC_URL,
            "channelType": { "code": "message" },
            "endpoint": endpoint
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

fn subscription_create_event(subscription_resource: Value) -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(TENANT_ID),
        fhir_version: current_fhir_version(),
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
        fhir_version: current_fhir_version(),
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

fn notification_type_from_bundle(bundle: &Value) -> Option<&str> {
    let status_resource = bundle.get("entry")?.get(0)?.get("resource")?;
    match status_resource.get("resourceType")?.as_str()? {
        "Parameters" => status_resource
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
            }),
        "SubscriptionStatus" => status_resource.get("type")?.as_str(),
        _ => None,
    }
}

#[tokio::test]
async fn rest_hook_handshake_and_event_notifications_follow_expected_flow() {
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
        assert_eq!(body["type"], expected_bundle_type());

        let notification_type =
            notification_type_from_bundle(&body).expect("bundle should include notification type");
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
            .register(TENANT_ID, "sub-message", &resource, current_fhir_version());

    assert!(
        matches!(
            result,
            Err(SubscriptionError::UnsupportedChannel { channel_type }) if channel_type == "message"
        ),
        "expected unsupported message channel error"
    );
}
