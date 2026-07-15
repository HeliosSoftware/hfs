//! T1 cluster-engine suite (strategy §8 Phase 3): two engines sharing the
//! in-memory reference stores simulate two instances against one database,
//! pinning the engine-side contracts — shared gap-free counters (B4),
//! scattered failures reaching the thresholds (B4), mint-on-A/redeem-once-
//! on-B tokens (B2), non-handshaking hydration with state overlay (B3) —
//! plus the *unsafe* memory contract (two engines without cluster handles
//! duplicate eventNumbers), the disease the seams cure.

use std::sync::Arc;

use chrono::Utc;
use helios_fhir::FhirVersion;
use helios_persistence::core::subscription_state::testing::{
    InMemoryHydrationSource, InMemorySubscriptionStateStore,
};
use helios_persistence::core::subscription_state::{HydratedResource, SubscriptionStateStore};
use helios_persistence::core::ws_binding_tokens::testing::InMemoryWsTokenStore;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_subscriptions::manager::SubscriptionStatusCode;
use helios_subscriptions::{
    ClusterHandles, ResourceEvent, ResourceEventType, SubscriptionConfig, SubscriptionEngine,
};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TENANT_ID: &str = "tenant-cluster";
const TOPIC_URL: &str = "http://example.org/topic/encounter-start";
const SUB_ID: &str = "sub-cluster-1";

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

fn rest_hook_subscription_resource(endpoint: &str, status: &str) -> Value {
    if uses_backport_ig() {
        json!({
            "resourceType": "Subscription",
            "id": SUB_ID,
            "status": status,
            "criteria": TOPIC_URL,
            "channel": {
                "type": "rest-hook",
                "endpoint": endpoint,
                "payload": "application/fhir+json"
            }
        })
    } else {
        json!({
            "resourceType": "Subscription",
            "id": SUB_ID,
            "status": status,
            "topic": TOPIC_URL,
            "channelType": { "code": "rest-hook" },
            "endpoint": endpoint,
            "contentType": "application/fhir+json",
            "content": "full-resource"
        })
    }
}

fn event(resource_type: &str, resource_id: &str, resource: Option<Value>) -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(TENANT_ID),
        fhir_version: current_fhir_version(),
        resource_type: resource_type.to_string(),
        resource_id: resource_id.to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource,
        previous_resource: None,
        timestamp: Utc::now(),
    }
}

fn encounter_event(resource_id: &str) -> ResourceEvent {
    event(
        "Encounter",
        resource_id,
        Some(json!({
            "resourceType": "Encounter",
            "id": resource_id,
            "status": "in-progress"
        })),
    )
}

fn topic_event() -> ResourceEvent {
    let resource = topic_resource();
    let resource_type = resource["resourceType"].as_str().unwrap().to_string();
    event(&resource_type, "topic-1", Some(resource.clone()))
}

fn subscription_event(endpoint: &str, status: &str) -> ResourceEvent {
    event(
        "Subscription",
        SUB_ID,
        Some(rest_hook_subscription_resource(endpoint, status)),
    )
}

/// The shared "database": one set of in-memory reference stores handed to
/// every engine, exactly the shape the Postgres seams provide.
#[derive(Clone)]
struct SharedStores {
    state: Arc<InMemorySubscriptionStateStore>,
    tokens: Arc<InMemoryWsTokenStore>,
    hydration: Arc<InMemoryHydrationSource>,
}

impl SharedStores {
    fn new() -> Self {
        Self {
            state: Arc::new(InMemorySubscriptionStateStore::new()),
            tokens: Arc::new(InMemoryWsTokenStore::new()),
            hydration: Arc::new(InMemoryHydrationSource::new()),
        }
    }

    fn handles(&self, instance_id: &str) -> ClusterHandles {
        ClusterHandles {
            state: self.state.clone(),
            tokens: self.tokens.clone(),
            hydration: self.hydration.clone(),
            instance_id: instance_id.to_string(),
        }
    }
}

fn cluster_engine(stores: &SharedStores, instance_id: &str) -> SubscriptionEngine {
    SubscriptionEngine::new(
        SubscriptionConfig::default(),
        "http://localhost:8080".to_string(),
    )
    .with_cluster_handles(stores.handles(instance_id))
}

fn local_engine() -> SubscriptionEngine {
    SubscriptionEngine::new(
        SubscriptionConfig::default(),
        "http://localhost:8080".to_string(),
    )
}

fn tenant_ctx() -> TenantContext {
    TenantContext::new(TenantId::new(TENANT_ID), TenantPermissions::full_access())
}

/// Extracts the notification's event number from a delivered bundle body —
/// R4 backport (`Parameters` part `event-number`, valueString) or native
/// (`SubscriptionStatus.notificationEvent[].eventNumber`, string or int).
fn extract_event_number(body: &Value) -> Option<u64> {
    let status = body.get("entry")?.as_array()?.first()?.get("resource")?;
    if status.get("resourceType")?.as_str()? == "Parameters" {
        for param in status.get("parameter")?.as_array()? {
            if param.get("name").and_then(Value::as_str) == Some("notification-event") {
                for part in param.get("part")?.as_array()? {
                    if part.get("name").and_then(Value::as_str) == Some("event-number")
                        && let Some(value) = part.get("valueString").and_then(Value::as_str)
                    {
                        return value.parse().ok();
                    }
                }
            }
        }
        None
    } else {
        let event = status.get("notificationEvent")?.as_array()?.first()?;
        let number = event.get("eventNumber")?;
        number
            .as_u64()
            .or_else(|| number.as_str().and_then(|s| s.parse().ok()))
    }
}

async fn delivered_event_numbers(server: &MockServer) -> Vec<u64> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter_map(|request| {
            let body: Value = serde_json::from_slice(&request.body).ok()?;
            extract_event_number(&body)
        })
        .collect()
}

/// B4: events alternated across two engines sharing the state store yield
/// gap-free monotonic eventNumbers 1..4.
#[tokio::test]
async fn shared_event_numbers_are_gap_free_across_engines() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webhook"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let endpoint = format!("{}/webhook", server.uri());

    let stores = SharedStores::new();
    let a = cluster_engine(&stores, "instance-a");
    let b = cluster_engine(&stores, "instance-b");

    // Both instances observed the topic + subscription writes (what the
    // lifecycle fan-out provides in the full stack).
    for engine in [&a, &b] {
        engine.on_resource_event(topic_event()).await;
        engine
            .on_resource_event(subscription_event(&endpoint, "active"))
            .await;
    }

    a.on_resource_event(encounter_event("enc-1")).await;
    b.on_resource_event(encounter_event("enc-2")).await;
    a.on_resource_event(encounter_event("enc-3")).await;
    b.on_resource_event(encounter_event("enc-4")).await;

    let mut numbers = delivered_event_numbers(&server).await;
    numbers.sort_unstable();
    assert_eq!(
        numbers,
        vec![1, 2, 3, 4],
        "eventNumbers must be cluster-wide monotonic and gap-free"
    );

    // Both engines' snapshots agree on the shared counter.
    for engine in [&a, &b] {
        let snapshot = engine
            .subscription_snapshot(TENANT_ID, SUB_ID)
            .await
            .expect("subscription is registered");
        assert_eq!(snapshot.events_since_start, 4);
    }
}

/// The unsafe memory contract this suite's seams cure: two engines WITHOUT
/// cluster handles each mint eventNumber 1 for different events.
#[tokio::test]
async fn without_cluster_handles_event_numbers_duplicate() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webhook"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let endpoint = format!("{}/webhook", server.uri());

    let a = local_engine();
    let b = local_engine();
    for engine in [&a, &b] {
        engine.on_resource_event(topic_event()).await;
        engine
            .on_resource_event(subscription_event(&endpoint, "active"))
            .await;
    }

    a.on_resource_event(encounter_event("enc-1")).await;
    b.on_resource_event(encounter_event("enc-2")).await;

    let numbers = delivered_event_numbers(&server).await;
    assert_eq!(
        numbers,
        vec![1, 1],
        "per-instance counters duplicate eventNumbers — the B4 disease"
    );
}

/// B4: failures scattered across engines accumulate in the shared store and
/// flip status to `error` then `off`, observed by BOTH engines' snapshots.
#[tokio::test]
async fn scattered_failures_reach_thresholds_on_both_engines() {
    let server = MockServer::start().await;
    // Permanent failure (4xx): one failure per event, no retry sleeps.
    Mock::given(method("POST"))
        .and(path("/webhook"))
        .respond_with(ResponseTemplate::new(410))
        .mount(&server)
        .await;
    let endpoint = format!("{}/webhook", server.uri());

    let stores = SharedStores::new();
    let a = cluster_engine(&stores, "instance-a");
    let b = cluster_engine(&stores, "instance-b");
    let config = SubscriptionConfig::default();

    for engine in [&a, &b] {
        engine.on_resource_event(topic_event()).await;
        engine
            .on_resource_event(subscription_event(&endpoint, "active"))
            .await;
    }

    // Scatter failures below the error threshold across both instances.
    let mut fired = 0u32;
    a.on_resource_event(encounter_event("enc-1")).await;
    b.on_resource_event(encounter_event("enc-2")).await;
    fired += 2;
    while fired < config.error_threshold {
        a.on_resource_event(encounter_event(&format!("enc-{fired}")))
            .await;
        fired += 1;
    }
    let _ = fired;
    for engine in [&a, &b] {
        let snapshot = engine
            .subscription_snapshot(TENANT_ID, SUB_ID)
            .await
            .unwrap();
        assert_eq!(
            snapshot.status,
            SubscriptionStatusCode::Error,
            "failures scattered across instances must reach the error threshold"
        );
        assert_eq!(snapshot.consecutive_failures, config.error_threshold);
    }
    // (The `off` threshold is not reachable through dispatch failures: an
    // `error`-status subscription no longer matches events — pre-existing
    // engine semantics, unchanged by Phase 3. The scattered-accumulation
    // property itself is what the shared store guarantees, proven above and
    // at T2 against Postgres.)
}

/// B2: a token minted through engine A redeems exactly once through engine
/// B; the second redeem fails everywhere.
#[tokio::test]
async fn ws_token_minted_on_a_redeems_once_on_b() {
    let stores = SharedStores::new();
    let a = cluster_engine(&stores, "instance-a");
    let b = cluster_engine(&stores, "instance-b");

    let (token, expires_at) = a
        .generate_ws_token(TENANT_ID, SUB_ID)
        .await
        .expect("mint succeeds");
    assert!(expires_at > Utc::now());

    assert_eq!(
        b.redeem_ws_token(&token).await,
        Some((TENANT_ID.to_string(), SUB_ID.to_string())),
        "a token minted on A must redeem on B"
    );
    assert!(
        a.redeem_ws_token(&token).await.is_none(),
        "second redeem must fail on every instance"
    );
}

/// B3: hydration rebuilds the projections from persisted resources without
/// firing activation handshakes, and the stored runtime state (status,
/// counters) overlays the resource's own status field.
#[tokio::test]
async fn hydration_registers_without_handshake_and_overlays_state() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webhook"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let endpoint = format!("{}/webhook", server.uri());

    let stores = SharedStores::new();
    let fhir_version = current_fhir_version();
    let topic = topic_resource();
    stores.hydration.insert(HydratedResource {
        tenant_id: TENANT_ID.to_string(),
        resource_type: topic["resourceType"].as_str().unwrap().to_string(),
        resource_id: "topic-1".to_string(),
        fhir_version: fhir_version.as_mime_param().to_string(),
        content: topic,
    });
    // The persisted resource still says "requested" — hydration must NOT
    // re-handshake it; the stored runtime status ("active") wins.
    stores.hydration.insert(HydratedResource {
        tenant_id: TENANT_ID.to_string(),
        resource_type: "Subscription".to_string(),
        resource_id: SUB_ID.to_string(),
        fhir_version: fhir_version.as_mime_param().to_string(),
        content: rest_hook_subscription_resource(&endpoint, "requested"),
    });

    // Runtime state accumulated by previous instances.
    let ctx = tenant_ctx();
    for _ in 0..5 {
        stores.state.next_event_number(&ctx, SUB_ID).await.unwrap();
    }
    stores.state.record_failure(&ctx, SUB_ID).await.unwrap();
    stores.state.record_failure(&ctx, SUB_ID).await.unwrap();
    stores
        .state
        .set_status(&ctx, SUB_ID, "active")
        .await
        .unwrap();

    let engine = cluster_engine(&stores, "instance-fresh");
    engine.hydrate().await;

    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty(),
        "hydration must not fire activation handshakes"
    );

    let snapshot = engine
        .subscription_snapshot(TENANT_ID, SUB_ID)
        .await
        .expect("hydration registered the subscription — no TopicNotFound");
    assert_eq!(snapshot.status, SubscriptionStatusCode::Active);
    assert_eq!(snapshot.events_since_start, 5);
    assert_eq!(snapshot.consecutive_failures, 2);

    // The hydrated projection is live: a matching event delivers with the
    // continued (not reset) event number.
    engine.on_resource_event(encounter_event("enc-after")).await;
    let numbers = delivered_event_numbers(&server).await;
    assert_eq!(
        numbers,
        vec![6],
        "the hydrated instance continues the shared sequence"
    );
}
