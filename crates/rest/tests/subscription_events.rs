//! HTTP contract of `GET [base]/Subscription/{id}/$events` (#2547): version
//! specific status shape, absolute subscription reference, id-only events and
//! range validation, against a real subscription engine.
#![cfg(feature = "subscriptions")]

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
use axum_test::TestServer;
use chrono::Utc;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::tenant::TenantId;
use helios_rest::{AppState, ServerConfig};
use helios_subscriptions::{
    ResourceEvent, ResourceEventType, SubscriptionConfig, SubscriptionEngine,
};
use serde_json::{Value, json};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const TENANT: &str = "default";
const BASE_URL: &str = "https://public.example/fhir";
const TOPIC_URL: &str = "http://example.org/topics/encounter-start";

fn backport(version: FhirVersion) -> bool {
    version == FhirVersion::R4
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

fn subscription(version: FhirVersion, endpoint: &str) -> Value {
    if backport(version) {
        json!({
            "resourceType": "Subscription", "id": "manual-sub", "status": "requested",
            "criteria": TOPIC_URL,
            "channel": {"type": "rest-hook", "endpoint": endpoint, "payload": "application/fhir+json"}
        })
    } else {
        json!({
            "resourceType": "Subscription", "id": "manual-sub", "status": "requested",
            "topic": TOPIC_URL,
            "channelType": {
                "system": "http://terminology.hl7.org/CodeSystem/subscription-channel-type",
                "code": "rest-hook"
            },
            "endpoint": endpoint, "contentType": "application/fhir+json", "content": "id-only"
        })
    }
}

fn event(version: FhirVersion, resource_type: &str, id: &str, resource: Value) -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(TENANT),
        fhir_version: version,
        resource_type: resource_type.to_string(),
        resource_id: id.to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(resource),
        previous_resource: None,
        timestamp: Utc::now(),
    }
}

/// A server whose engine holds an active `manual-sub` with `events` events.
async fn server(version: FhirVersion, events: usize, history: usize) -> (TestServer, MockServer) {
    let hook = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&hook)
        .await;

    let engine = Arc::new(SubscriptionEngine::new(
        SubscriptionConfig {
            event_history_size: history,
            ..Default::default()
        },
        BASE_URL.to_string(),
    ));
    engine
        .on_resource_event(event(
            version,
            if backport(version) {
                "Basic"
            } else {
                "SubscriptionTopic"
            },
            "topic-1",
            topic(version),
        ))
        .await;
    engine
        .on_resource_event(event(
            version,
            "Subscription",
            "manual-sub",
            subscription(version, &format!("{}/hook", hook.uri())),
        ))
        .await;
    for n in 1..=events {
        let id = format!("enc-{n}");
        engine
            .on_resource_event(event(
                version,
                "Encounter",
                &id,
                json!({"resourceType": "Encounter", "id": id, "status": "in-progress"}),
            ))
            .await;
    }

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
    let state = AppState::new(
        backend,
        ServerConfig {
            base_url: BASE_URL.to_string(),
            default_tenant: TENANT.to_string(),
            ..ServerConfig::for_testing()
        },
    )
    .with_subscription_engine(engine);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    (TestServer::new(app).unwrap(), hook)
}

fn focus_ids(bundle: &Value) -> Vec<String> {
    bundle["entry"].as_array().unwrap()[1..]
        .iter()
        .map(|e| e["request"]["url"].as_str().unwrap().to_string())
        .collect()
}

#[cfg(feature = "R4")]
#[tokio::test]
async fn r4_events_returns_parameters_status_and_requested_range() {
    let (server, _hook) = server(FhirVersion::R4, 3, 100).await;

    let response = server
        .get("/Subscription/manual-sub/$events?eventsSinceNumber=1&eventsUntilNumber=3")
        .await;
    response.assert_status_ok();
    let bundle: Value = response.json();
    assert_eq!(bundle["type"], "history");
    let status = &bundle["entry"][0]["resource"];
    assert_eq!(status["resourceType"], "Parameters");
    let params = status["parameter"].as_array().unwrap();
    let get = |name: &str| params.iter().find(|p| p["name"] == name).unwrap();
    assert_eq!(get("type")["valueCode"], "query-event");
    assert_eq!(
        get("subscription")["valueReference"]["reference"],
        format!("{BASE_URL}/Subscription/manual-sub")
    );
    assert_eq!(get("events-since-subscription-start")["valueString"], "3");
    assert_eq!(
        params
            .iter()
            .filter(|p| p["name"] == "notification-event")
            .count(),
        3
    );
    assert_eq!(
        focus_ids(&bundle),
        ["Encounter/enc-1", "Encounter/enc-2", "Encounter/enc-3"]
    );

    // The range changes the result.
    let bundle: Value = server
        .get("/Subscription/manual-sub/$events?eventsSinceNumber=2&eventsUntilNumber=2")
        .await
        .json();
    assert_eq!(focus_ids(&bundle), ["Encounter/enc-2"]);

    // Defaults cover everything retained.
    let bundle: Value = server.get("/Subscription/manual-sub/$events").await.json();
    assert_eq!(focus_ids(&bundle).len(), 3);
}

#[cfg(feature = "R4")]
#[tokio::test]
async fn r4_events_invalid_ranges_are_operation_outcomes() {
    let (server, _hook) = server(FhirVersion::R4, 3, 100).await;
    for query in [
        "eventsSinceNumber=abc",
        "eventsUntilNumber=-1",
        "eventsSinceNumber=0",
        "eventsSinceNumber=3&eventsUntilNumber=1",
        "eventsSinceNumber=5",
    ] {
        let response = server
            .get(&format!("/Subscription/manual-sub/$events?{query}"))
            .await;
        response.assert_status(StatusCode::BAD_REQUEST);
        let outcome: Value = response.json();
        assert_eq!(outcome["resourceType"], "OperationOutcome", "{query}");
        assert_eq!(outcome["issue"][0]["code"], "invalid", "{query}");
    }
}

#[cfg(feature = "R4")]
#[tokio::test]
async fn r4_events_history_is_bounded_and_evicted_range_is_rejected() {
    let (server, _hook) = server(FhirVersion::R4, 3, 2).await;

    // Partial overlap returns the retained subset.
    let bundle: Value = server
        .get("/Subscription/manual-sub/$events?eventsSinceNumber=1&eventsUntilNumber=3")
        .await
        .json();
    assert_eq!(focus_ids(&bundle), ["Encounter/enc-2", "Encounter/enc-3"]);

    // Entirely evicted.
    server
        .get("/Subscription/manual-sub/$events?eventsSinceNumber=1&eventsUntilNumber=1")
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[cfg(feature = "R4")]
#[tokio::test]
async fn unknown_subscription_is_not_found() {
    let (server, _hook) = server(FhirVersion::R4, 0, 100).await;
    server
        .get("/Subscription/nope/$events")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[cfg(feature = "R4B")]
#[tokio::test]
async fn r4b_events_returns_subscription_status_with_notification_events() {
    let (server, _hook) = server(FhirVersion::R4B, 3, 100).await;

    let response = server
        .get("/Subscription/manual-sub/$events?eventsSinceNumber=2&eventsUntilNumber=3")
        .await;
    response.assert_status_ok();
    let bundle: Value = response.json();
    assert_eq!(bundle["type"], "history");
    let status = &bundle["entry"][0]["resource"];
    assert_eq!(status["resourceType"], "SubscriptionStatus");
    assert_eq!(status["type"], "query-event");
    assert_eq!(
        status["subscription"]["reference"],
        format!("{BASE_URL}/Subscription/manual-sub")
    );
    assert_eq!(status["notificationEvent"].as_array().unwrap().len(), 2);
    assert_eq!(focus_ids(&bundle), ["Encounter/enc-2", "Encounter/enc-3"]);

    server
        .get("/Subscription/manual-sub/$events?eventsSinceNumber=9")
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}
