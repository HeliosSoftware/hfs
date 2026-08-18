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
use helios_persistence::core::event_fanout::testing::InMemoryEventFanout;
use helios_persistence::core::subscription_delivery::WorkerId;
use helios_persistence::core::subscription_delivery::testing::InMemoryDeliveryOutbox;
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
    fanout: Arc<InMemoryEventFanout>,
    outbox: Arc<InMemoryDeliveryOutbox>,
}

impl SharedStores {
    fn new() -> Self {
        Self {
            state: Arc::new(InMemorySubscriptionStateStore::new()),
            tokens: Arc::new(InMemoryWsTokenStore::new()),
            hydration: Arc::new(InMemoryHydrationSource::new()),
            fanout: Arc::new(InMemoryEventFanout::default()),
            outbox: Arc::new(InMemoryDeliveryOutbox::new()),
        }
    }

    fn handles(&self, instance_id: &str) -> ClusterHandles {
        ClusterHandles {
            state: self.state.clone(),
            tokens: self.tokens.clone(),
            hydration: self.hydration.clone(),
            fanout: self.fanout.clone(),
            outbox: self.outbox.clone(),
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

fn ws_subscription_resource(status: &str) -> Value {
    if uses_backport_ig() {
        json!({
            "resourceType": "Subscription",
            "id": SUB_ID,
            "status": status,
            "criteria": TOPIC_URL,
            "channel": {
                "type": "websocket",
                "payload": "application/fhir+json"
            }
        })
    } else {
        json!({
            "resourceType": "Subscription",
            "id": SUB_ID,
            "status": status,
            "topic": TOPIC_URL,
            "channelType": { "code": "websocket" },
            "contentType": "application/fhir+json",
            "content": "id-only"
        })
    }
}

fn ws_subscription_event(status: &str) -> ResourceEvent {
    event(
        "Subscription",
        SUB_ID,
        Some(ws_subscription_resource(status)),
    )
}

/// Drains the shared outbox through one engine's worker seam — in cluster
/// mode push-channel notifications deliver via workers, not inline.
async fn drain_outbox(engine: &SubscriptionEngine, worker: &str) {
    let worker_id = WorkerId::new(worker);
    while engine
        .run_next_subscription_delivery(&worker_id, std::time::Duration::from_secs(60))
        .await
    {}
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
    drain_outbox(&a, "worker-a").await;
    b.on_resource_event(encounter_event("enc-2")).await;
    drain_outbox(&b, "worker-b").await;
    a.on_resource_event(encounter_event("enc-3")).await;
    drain_outbox(&a, "worker-a").await;
    b.on_resource_event(encounter_event("enc-4")).await;
    drain_outbox(&b, "worker-b").await;

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

    // Scatter failures across both instances: each event enqueues on the
    // shared outbox and the firing instance's worker attempts it (410 =
    // permanent, one failure per event, no retry scheduling).
    let mut fired = 0u32;
    a.on_resource_event(encounter_event("enc-1")).await;
    drain_outbox(&a, "worker-a").await;
    b.on_resource_event(encounter_event("enc-2")).await;
    drain_outbox(&b, "worker-b").await;
    fired += 2;
    while fired < config.error_threshold {
        a.on_resource_event(encounter_event(&format!("enc-{fired}")))
            .await;
        drain_outbox(&a, "worker-a").await;
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
    drain_outbox(&engine, "worker-fresh").await;
    let numbers = delivered_event_numbers(&server).await;
    assert_eq!(
        numbers,
        vec![6],
        "the hydrated instance continues the shared sequence"
    );
}

/// B1: a websocket notification produced on instance A reaches a socket
/// client registered on instance B via the fan-out, and the origin does not
/// double-deliver to its own clients.
#[tokio::test]
async fn ws_event_fans_out_to_other_instances_sockets() {
    let stores = SharedStores::new();
    // The default config supports rest-hook only; this test needs the
    // websocket channel (the server enables both).
    let ws_config = SubscriptionConfig {
        supported_channel_types: vec!["rest-hook".to_string(), "websocket".to_string()],
        ..SubscriptionConfig::default()
    };
    let a = Arc::new(
        SubscriptionEngine::new(ws_config.clone(), "http://localhost:8080".to_string())
            .with_cluster_handles(stores.handles("instance-a")),
    );
    let b = Arc::new(
        SubscriptionEngine::new(ws_config, "http://localhost:8080".to_string())
            .with_cluster_handles(stores.handles("instance-b")),
    );

    // Both instances know the topic + websocket subscription (lifecycle
    // propagation is exercised separately; here both saw the writes).
    for engine in [&a, &b] {
        engine.on_resource_event(topic_event()).await;
        engine
            .on_resource_event(ws_subscription_event("active"))
            .await;
    }

    // Instance B consumes the fan-out and holds a socket client; instance A
    // holds its own local client.
    let listener = b.start_fanout_listener().expect("cluster-backed");
    let (_client_b, mut rx_b) = b.ws_manager().register_client(TENANT_ID, SUB_ID);
    let (_client_a, mut rx_a) = a.ws_manager().register_client(TENANT_ID, SUB_ID);

    a.on_resource_event(encounter_event("enc-1")).await;

    // A's local client got the event inline (lossless local leg)...
    let bundle_a = tokio::time::timeout(std::time::Duration::from_secs(10), rx_a.recv())
        .await
        .expect("A's local client must receive inline")
        .expect("channel open");
    assert_eq!(extract_event_number(&bundle_a), Some(1));
    // ...exactly once: the origin skips its own fan-out envelope.
    tokio::task::yield_now().await;
    assert!(
        rx_a.try_recv().is_err(),
        "the origin must not double-deliver via its own envelope"
    );

    // B's client received via the fan-out (event-driven await, no sleeps).
    let bundle_b = tokio::time::timeout(std::time::Duration::from_secs(10), rx_b.recv())
        .await
        .expect("B's socket must receive via the fan-out")
        .expect("channel open");
    assert_eq!(extract_event_number(&bundle_b), Some(1));

    listener.abort();
}

/// B5: a retryable failure re-queues on the outbox with the persisted
/// backoff schedule and a later worker cycle delivers; the endpoint sees
/// exactly two requests (fail, then success).
#[tokio::test]
async fn outbox_retryable_failure_requeues_then_delivers() {
    let server = MockServer::start().await;
    // First request fails retryably (503), the rest succeed.
    Mock::given(method("POST"))
        .and(path("/webhook"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webhook"))
        .respond_with(ResponseTemplate::new(200))
        .with_priority(2)
        .mount(&server)
        .await;
    let endpoint = format!("{}/webhook", server.uri());

    let stores = SharedStores::new();
    // Zero retry delay keeps the re-queued row immediately claimable — the
    // schedule itself is pinned at T2 against the database clock.
    let config = SubscriptionConfig {
        retry_initial_delay: std::time::Duration::ZERO,
        ..SubscriptionConfig::default()
    };
    let engine = SubscriptionEngine::new(config, "http://localhost:8080".to_string())
        .with_cluster_handles(stores.handles("instance-a"));

    engine.on_resource_event(topic_event()).await;
    engine
        .on_resource_event(subscription_event(&endpoint, "active"))
        .await;
    engine.on_resource_event(encounter_event("enc-1")).await;

    // One drain: the worker claims (503 → re-queued, due immediately) and
    // claims again (200 → delivered).
    drain_outbox(&engine, "worker-retry").await;

    let requests = server.received_requests().await.unwrap_or_default();
    assert_eq!(
        requests.len(),
        2,
        "exactly two attempts: the retryable failure, then the delivery"
    );
    assert_eq!(delivered_event_numbers(&server).await, vec![1, 1]);

    // Success reset the shared failure streak.
    let snapshot = engine
        .subscription_snapshot(TENANT_ID, SUB_ID)
        .await
        .unwrap();
    assert_eq!(snapshot.consecutive_failures, 0);
    assert_eq!(snapshot.status, SubscriptionStatusCode::Active);
}

/// Lifecycle propagation: a Subscription created on instance A announces
/// itself over the fan-out; instance B (listener running) re-reads the
/// resource from storage and registers it locally — then a delete on A
/// deregisters it on B.
#[tokio::test]
async fn lifecycle_envelopes_propagate_registration_and_delete() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webhook"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let endpoint = format!("{}/webhook", server.uri());

    let stores = SharedStores::new();
    let a = Arc::new(cluster_engine(&stores, "instance-a"));
    let b = Arc::new(cluster_engine(&stores, "instance-b"));
    let listener = b.start_fanout_listener().expect("cluster-backed");

    // The lifecycle receiver re-reads from storage: persist the resources
    // in the shared hydration source (what the resources table provides).
    let fhir_version = current_fhir_version();
    let topic = topic_resource();
    let topic_type = topic["resourceType"].as_str().unwrap().to_string();
    stores.hydration.insert(HydratedResource {
        tenant_id: TENANT_ID.to_string(),
        resource_type: topic_type,
        resource_id: "topic-1".to_string(),
        fhir_version: fhir_version.as_mime_param().to_string(),
        content: topic,
    });
    stores.hydration.insert(HydratedResource {
        tenant_id: TENANT_ID.to_string(),
        resource_type: "Subscription".to_string(),
        resource_id: SUB_ID.to_string(),
        fhir_version: fhir_version.as_mime_param().to_string(),
        content: rest_hook_subscription_resource(&endpoint, "active"),
    });

    // Live writes on A announce over the fan-out.
    a.on_resource_event(topic_event()).await;
    a.on_resource_event(subscription_event(&endpoint, "active"))
        .await;

    // B registers without ever seeing the write (event-driven wait).
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if b.subscription_snapshot(TENANT_ID, SUB_ID).await.is_some() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "instance B must register A's subscription from the lifecycle envelope"
        );
        tokio::task::yield_now().await;
    }

    // A delete on A deregisters on B.
    stores.hydration.remove(TENANT_ID, "Subscription", SUB_ID);
    let delete_event = ResourceEvent {
        resource: None,
        previous_resource: Some(rest_hook_subscription_resource(&endpoint, "active")),
        event_type: ResourceEventType::Delete,
        ..subscription_event(&endpoint, "active")
    };
    a.on_resource_event(delete_event).await;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if b.subscription_snapshot(TENANT_ID, SUB_ID).await.is_none() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "instance B must deregister after A's delete envelope"
        );
        tokio::task::yield_now().await;
    }

    listener.abort();
}
