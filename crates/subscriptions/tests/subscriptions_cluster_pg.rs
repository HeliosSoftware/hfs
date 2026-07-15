//! T2 cluster suite for the engine over PostgreSQL (strategy §8 Phase 3).
//!
//! The strategy's B2/B3/B4 DoD rows, driven end-to-end through TWO
//! independently constructed `SubscriptionEngine`s whose cluster handles
//! come from TWO fresh `PostgresBackend`s sharing only the database
//! (methodology §6) — the Postgres twin of `cluster_engine.rs`:
//!
//! - **B3** — a topic + subscription created via instance A is loaded and
//!   matchable via instance B after hydration (no `TopicNotFound`).
//! - **B4** — events alternated across instances yield gap-free monotonic
//!   `eventNumber`s, asserted on the wiremock-delivered bodies.
//! - **B2** — a binding token minted via A redeems exactly once via B.
//!
//! Per-test unique tenants keep rows disjoint (no suite lock needed — the
//! outbox, whose claims are cross-tenant, is not exercised here).

#![cfg(feature = "postgres")]

use std::path::PathBuf;

use chrono::Utc;
use helios_fhir::FhirVersion;
use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_subscriptions::manager::SubscriptionStatusCode;
use helios_subscriptions::{
    ClusterHandles, ResourceEvent, ResourceEventType, SubscriptionConfig, SubscriptionEngine,
};
use serde_json::{Value, json};
use tokio::sync::OnceCell;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOPIC_URL: &str = "http://example.org/topic/encounter-start";
const SUB_ID: &str = "sub-cluster-1";

// ── Shared PostgreSQL container (the postgres_tests.rs idiom) ─────────────

struct SharedPg {
    host: String,
    port: u16,
    /// Kept alive for the test binary; a `static` is never dropped, so the
    /// container is reaped in CI by its `github.run_id` label.
    _container: testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
}

static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| PathBuf::from("data"))
}

fn pg_config(host: &str, port: u16) -> PostgresConfig {
    PostgresConfig {
        host: host.to_string(),
        port,
        dbname: "postgres".to_string(),
        user: "postgres".to_string(),
        password: Some("postgres".to_string()),
        max_connections: 5,
        data_dir: Some(data_dir()),
        ..Default::default()
    }
}

async fn shared_pg() -> &'static SharedPg {
    SHARED_PG
        .get_or_init(|| async {
            use testcontainers::ImageExt;
            use testcontainers::runners::AsyncRunner;

            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
            let container = testcontainers_modules::postgres::Postgres::default()
                .with_label("github.run_id", &run_id)
                .start()
                .await
                .expect("Failed to start PostgreSQL container");
            let port = container
                .get_host_port_ipv4(5432)
                .await
                .expect("Failed to get host port");
            let host = container
                .get_host()
                .await
                .expect("Failed to get host")
                .to_string();

            let backend = PostgresBackend::new(pg_config(&host, port))
                .await
                .expect("Failed to create PostgresBackend");
            backend
                .init_schema()
                .await
                .expect("Failed to initialize schema");

            SharedPg {
                host,
                port,
                _container: container,
            }
        })
        .await
}

/// A fresh backend handle per call — two calls simulate two instances.
async fn create_backend() -> PostgresBackend {
    let pg = shared_pg().await;
    PostgresBackend::new(pg_config(&pg.host, pg.port))
        .await
        .expect("Failed to create PostgresBackend")
}

// ── Version-aware fixtures ────────────────────────────────────────────────

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

fn event(
    tenant_id: &str,
    resource_type: &str,
    resource_id: &str,
    resource: Option<Value>,
) -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(tenant_id),
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

fn encounter_event(tenant_id: &str, resource_id: &str) -> ResourceEvent {
    event(
        tenant_id,
        "Encounter",
        resource_id,
        Some(json!({
            "resourceType": "Encounter",
            "id": resource_id,
            "status": "in-progress"
        })),
    )
}

fn topic_event(tenant_id: &str) -> ResourceEvent {
    let resource = topic_resource();
    let resource_type = resource["resourceType"].as_str().unwrap().to_string();
    event(tenant_id, &resource_type, "topic-1", Some(resource))
}

fn subscription_event(tenant_id: &str, endpoint: &str, status: &str) -> ResourceEvent {
    event(
        tenant_id,
        "Subscription",
        SUB_ID,
        Some(rest_hook_subscription_resource(endpoint, status)),
    )
}

fn unique_tenant(prefix: &str) -> String {
    format!("{}_{}", prefix, uuid::Uuid::new_v4().simple())
}

fn tenant_ctx(tenant_id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access())
}

fn pg_cluster_engine(backend: &PostgresBackend, instance_id: &str) -> SubscriptionEngine {
    SubscriptionEngine::new(
        SubscriptionConfig::default(),
        "http://localhost:8080".to_string(),
    )
    .with_cluster_handles(ClusterHandles {
        state: backend
            .subscription_state_store()
            .expect("postgres backs a subscription state store"),
        tokens: backend
            .ws_binding_token_store()
            .expect("postgres backs a ws binding token store"),
        hydration: backend
            .subscription_hydration_source()
            .expect("postgres backs a hydration source"),
        instance_id: instance_id.to_string(),
    })
}

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

// ── The strategy §8 Phase 3 DoD rows ──────────────────────────────────────

/// B3: a topic + subscription created via instance A (resources persisted +
/// live registration) is loaded and matchable via a freshly constructed
/// instance B after hydration — no `TopicNotFound`, and a matching event on
/// B delivers.
#[tokio::test]
async fn pg_cluster_subscription_created_on_a_is_matchable_on_b_after_hydration() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webhook"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let endpoint = format!("{}/webhook", server.uri());
    let tenant = unique_tenant("b3");
    let ctx = tenant_ctx(&tenant);
    let fhir_version = current_fhir_version();

    // Instance A: the write path — resources persist to storage AND the
    // engine reacts to the write events.
    let backend_a = create_backend().await;
    let engine_a = pg_cluster_engine(&backend_a, "instance-a");
    let topic = topic_resource();
    let topic_type = topic["resourceType"].as_str().unwrap().to_string();
    backend_a
        .create(&ctx, &topic_type, topic, fhir_version)
        .await
        .expect("persist topic resource");
    backend_a
        .create(
            &ctx,
            "Subscription",
            rest_hook_subscription_resource(&endpoint, "active"),
            fhir_version,
        )
        .await
        .expect("persist subscription resource");
    engine_a.on_resource_event(topic_event(&tenant)).await;
    engine_a
        .on_resource_event(subscription_event(&tenant, &endpoint, "active"))
        .await;

    // Instance B: fresh backend + fresh engine sharing only the database.
    let backend_b = create_backend().await;
    let engine_b = pg_cluster_engine(&backend_b, "instance-b");
    engine_b.hydrate().await;

    let snapshot = engine_b
        .subscription_snapshot(&tenant, SUB_ID)
        .await
        .expect("hydration must register A's subscription — no TopicNotFound");
    assert_eq!(snapshot.status, SubscriptionStatusCode::Active);

    // Isolation: another tenant sees nothing.
    assert!(
        engine_b
            .subscription_snapshot(&unique_tenant("b3-other"), SUB_ID)
            .await
            .is_none(),
        "another tenant's snapshot must be empty"
    );

    // A matching event served by B delivers, continuing the shared sequence.
    engine_b
        .on_resource_event(encounter_event(&tenant, "enc-b"))
        .await;
    assert_eq!(
        delivered_event_numbers(&server).await,
        vec![1],
        "instance B must match and deliver after hydration"
    );
}

/// B4: events alternated across two instances yield cluster-wide gap-free
/// monotonic eventNumbers, asserted on the delivered notification bodies.
#[tokio::test]
async fn pg_cluster_event_numbers_are_gap_free_across_engines() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webhook"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let endpoint = format!("{}/webhook", server.uri());
    let tenant = unique_tenant("b4");

    let backend_a = create_backend().await;
    let backend_b = create_backend().await;
    let engine_a = pg_cluster_engine(&backend_a, "instance-a");
    let engine_b = pg_cluster_engine(&backend_b, "instance-b");

    for engine in [&engine_a, &engine_b] {
        engine.on_resource_event(topic_event(&tenant)).await;
        engine
            .on_resource_event(subscription_event(&tenant, &endpoint, "active"))
            .await;
    }

    engine_a
        .on_resource_event(encounter_event(&tenant, "enc-1"))
        .await;
    engine_b
        .on_resource_event(encounter_event(&tenant, "enc-2"))
        .await;
    engine_a
        .on_resource_event(encounter_event(&tenant, "enc-3"))
        .await;
    engine_b
        .on_resource_event(encounter_event(&tenant, "enc-4"))
        .await;

    let mut numbers = delivered_event_numbers(&server).await;
    numbers.sort_unstable();
    assert_eq!(
        numbers,
        vec![1, 2, 3, 4],
        "eventNumbers must be cluster-wide monotonic and gap-free"
    );

    for engine in [&engine_a, &engine_b] {
        let snapshot = engine
            .subscription_snapshot(&tenant, SUB_ID)
            .await
            .expect("subscription is registered");
        assert_eq!(snapshot.events_since_start, 4);
    }
}

/// B2: a binding token minted through instance A redeems exactly once
/// through instance B.
#[tokio::test]
async fn pg_cluster_ws_token_minted_on_a_redeems_once_on_b() {
    let tenant = unique_tenant("b2");
    let backend_a = create_backend().await;
    let backend_b = create_backend().await;
    let engine_a = pg_cluster_engine(&backend_a, "instance-a");
    let engine_b = pg_cluster_engine(&backend_b, "instance-b");

    let (token, expires_at) = engine_a
        .generate_ws_token(&tenant, SUB_ID)
        .await
        .expect("mint succeeds");
    assert!(expires_at > Utc::now());

    assert_eq!(
        engine_b.redeem_ws_token(&token).await,
        Some((tenant.clone(), SUB_ID.to_string())),
        "a token minted on A must redeem on B"
    );
    assert!(
        engine_a.redeem_ws_token(&token).await.is_none(),
        "second redeem must fail on every instance"
    );
}

/// B4 (durability leg): the shared counter survives both engine handles —
/// a third, fresh instance continues the sequence.
#[tokio::test]
async fn pg_cluster_event_numbers_survive_engine_drop() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webhook"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let endpoint = format!("{}/webhook", server.uri());
    let tenant = unique_tenant("b4-durable");

    {
        let backend = create_backend().await;
        let engine = pg_cluster_engine(&backend, "instance-dying");
        engine.on_resource_event(topic_event(&tenant)).await;
        engine
            .on_resource_event(subscription_event(&tenant, &endpoint, "active"))
            .await;
        engine
            .on_resource_event(encounter_event(&tenant, "enc-1"))
            .await;
        engine
            .on_resource_event(encounter_event(&tenant, "enc-2"))
            .await;
    } // engine + backend dropped

    let backend = create_backend().await;
    let engine = pg_cluster_engine(&backend, "instance-fresh");
    // Local registration via the live path (hydration would need persisted
    // resources; this row pins the counter, not B3).
    engine.on_resource_event(topic_event(&tenant)).await;
    engine
        .on_resource_event(subscription_event(&tenant, &endpoint, "active"))
        .await;
    engine
        .on_resource_event(encounter_event(&tenant, "enc-3"))
        .await;

    let mut numbers = delivered_event_numbers(&server).await;
    numbers.sort_unstable();
    assert_eq!(
        numbers,
        vec![1, 2, 3],
        "a fresh instance continues the sequence — registration never resets it"
    );
}
