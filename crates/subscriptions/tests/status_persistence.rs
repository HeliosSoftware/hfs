//! Server-driven subscription status transitions are written back into the
//! stored `Subscription` resource (issue #357).
//!
//! Before this, every runtime decision the engine made — `requested` → `active`
//! after a successful handshake, `→ error`, `→ off` once the delivery circuit
//! breaker tripped — lived only in the engine's in-memory `DashMap`. Two
//! consequences these tests pin down:
//!
//! 1. `GET /Subscription/{id}` (storage) contradicted
//!    `GET /Subscription/{id}/$status` (engine) for the life of the resource.
//! 2. The circuit breaker was amnesiac: a subscription the server turned `off`
//!    after repeated delivery failures reverted to its stored status on restart
//!    and resumed hammering the dead endpoint.
//!
//! Each test drives the engine through its **public** entry point
//! (`on_resource_event`), exactly as the REST write handlers do, and then reads
//! the resource back out of a real storage backend. Asserting the stored
//! resource — not the in-memory entry — is what makes these regression tests:
//! the in-memory half already worked.

use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_subscriptions::manager::SubscriptionStatusCode;
use helios_subscriptions::{
    ResourceEvent, ResourceEventType, SubscriptionConfig, SubscriptionEngine,
};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const TENANT_ID: &str = "tenant-a";
const TOPIC_URL: &str = "http://example.org/topic/encounter-start";

fn current_fhir_version() -> FhirVersion {
    FhirVersion::default()
}

fn uses_backport_ig() -> bool {
    current_fhir_version() == FhirVersion::R4
}

fn tenant_context(tenant_id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access())
}

fn storage() -> Arc<SqliteBackend> {
    let backend = SqliteBackend::in_memory().expect("in-memory sqlite backend");
    backend.init_schema().expect("init schema");
    Arc::new(backend)
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

fn topic_resource_type() -> &'static str {
    if uses_backport_ig() {
        "Basic"
    } else {
        "SubscriptionTopic"
    }
}

fn subscription_resource(id: &str, status: &str, endpoint: &str) -> Value {
    if uses_backport_ig() {
        json!({
            "resourceType": "Subscription",
            "id": id,
            "status": status,
            "criteria": TOPIC_URL,
            "channel": {
                "type": "rest-hook",
                "endpoint": endpoint,
                "payload": "application/fhir+json"
            },
            "extension": [{
                "url": "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-topic-canonical",
                "valueCanonical": TOPIC_URL
            }, {
                "url": "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-payload-content",
                "valueCode": "id-only"
            }]
        })
    } else {
        json!({
            "resourceType": "Subscription",
            "id": id,
            "status": status,
            "topic": TOPIC_URL,
            "channelType": { "code": "rest-hook" },
            "endpoint": endpoint,
            "content": "id-only"
        })
    }
}

/// An engine wired to `storage` for status write-back, with retries and delays
/// collapsed so a single failed dispatch trips the breaker inside a test.
///
/// `error_threshold` / `off_threshold` are explicit because
/// `handle_delivery_failure` checks `off` **first**: setting both to 1 makes the
/// very first failure go straight to `off`, which is how the `off` case is
/// reached without needing a second delivery (an `error` subscription is no
/// longer matched, so there is no second delivery to be had).
fn engine_with_thresholds(
    storage: Arc<SqliteBackend>,
    error_threshold: u32,
    off_threshold: u32,
) -> SubscriptionEngine {
    SubscriptionEngine::new(
        SubscriptionConfig {
            // No delivery retries: one failed dispatch == one recorded failure.
            max_retries: 0,
            retry_initial_delay: Duration::from_millis(1),
            retry_max_delay: Duration::from_millis(1),
            handshake_max_attempts: 1,
            handshake_retry_initial_delay: Duration::from_millis(1),
            handshake_retry_max_delay: Duration::from_millis(1),
            error_threshold,
            off_threshold,
            ..SubscriptionConfig::default()
        },
        "http://localhost:8080".to_string(),
    )
    .with_status_store(storage as Arc<dyn ResourceStorage>)
}

/// The common case: thresholds far enough away that only the handshake matters.
fn engine_with_writeback(storage: Arc<SqliteBackend>) -> SubscriptionEngine {
    engine_with_thresholds(storage, 3, 10)
}

/// A subscriber that accepts the activation handshake and then fails every
/// notification — the shape that trips the delivery circuit breaker.
async fn handshake_ok_then_failing_subscriber() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    server
}

async fn seed(backend: &SqliteBackend, resource_type: &str, resource: Value) {
    backend
        .create(
            &tenant_context(TENANT_ID),
            resource_type,
            resource,
            current_fhir_version(),
        )
        .await
        .expect("seed resource");
}

/// Reads `Subscription.status` straight out of storage — the value a plain
/// `GET /Subscription/{id}` would return.
async fn stored_status(backend: &SqliteBackend, id: &str) -> String {
    backend
        .read(&tenant_context(TENANT_ID), "Subscription", id)
        .await
        .expect("read subscription")
        .expect("subscription exists")
        .content()
        .get("status")
        .and_then(|v| v.as_str())
        .expect("status present")
        .to_string()
}

async fn stored_version(backend: &SqliteBackend, id: &str) -> String {
    backend
        .read(&tenant_context(TENANT_ID), "Subscription", id)
        .await
        .expect("read subscription")
        .expect("subscription exists")
        .version_id()
        .to_string()
}

fn subscription_written_event(resource: Value, id: &str) -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(TENANT_ID),
        fhir_version: current_fhir_version(),
        resource_type: "Subscription".to_string(),
        resource_id: id.to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(resource),
        previous_resource: None,
        timestamp: chrono::Utc::now(),
    }
}

fn encounter_created_event() -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(TENANT_ID),
        fhir_version: current_fhir_version(),
        resource_type: "Encounter".to_string(),
        resource_id: "enc-1".to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(json!({ "resourceType": "Encounter", "id": "enc-1" })),
        previous_resource: None,
        timestamp: chrono::Utc::now(),
    }
}

/// The headline symptom: a subscription the server activates must read back
/// `active`, not `requested`, from storage.
#[tokio::test]
async fn successful_handshake_persists_active() {
    let subscriber = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&subscriber)
        .await;

    let backend = storage();
    seed(&backend, topic_resource_type(), topic_resource()).await;
    let resource = subscription_resource("sub-1", "requested", &subscriber.uri());
    seed(&backend, "Subscription", resource.clone()).await;

    assert_eq!(stored_status(&backend, "sub-1").await, "requested");

    let engine = engine_with_writeback(Arc::clone(&backend));
    engine
        .on_resource_event(subscription_written_event(resource, "sub-1"))
        .await;

    // In-memory and stored now agree — that agreement is the whole issue.
    assert_eq!(
        engine
            .manager()
            .get_subscription(TENANT_ID, "sub-1")
            .expect("registered")
            .status,
        SubscriptionStatusCode::Active
    );
    assert_eq!(
        stored_status(&backend, "sub-1").await,
        "active",
        "the stored resource must not keep saying `requested` after activation"
    );
}

/// A delivery failure that trips the `error` threshold must reach storage.
#[tokio::test]
async fn delivery_failure_persists_error() {
    let subscriber = handshake_ok_then_failing_subscriber().await;

    let backend = storage();
    seed(&backend, topic_resource_type(), topic_resource()).await;
    let resource = subscription_resource("sub-1", "requested", &subscriber.uri());
    seed(&backend, "Subscription", resource.clone()).await;

    // error on the 1st failure; `off` far away so this test isolates `error`.
    let engine = engine_with_thresholds(Arc::clone(&backend), 1, 99);
    engine
        .on_resource_event(subscription_written_event(resource, "sub-1"))
        .await;
    assert_eq!(stored_status(&backend, "sub-1").await, "active");

    engine.on_resource_event(encounter_created_event()).await;

    assert_eq!(
        engine
            .manager()
            .get_subscription(TENANT_ID, "sub-1")
            .expect("registered")
            .status,
        SubscriptionStatusCode::Error
    );
    assert_eq!(
        stored_status(&backend, "sub-1").await,
        "error",
        "a delivery failure that trips the breaker must reach storage"
    );
}

/// The `off` decision — the one that exists to stop the server hammering a dead
/// endpoint — must be durable. Losing it is the amnesiac circuit breaker: on
/// restart the subscription reverts to its stored status and delivery resumes.
#[tokio::test]
async fn delivery_circuit_breaker_persists_off() {
    let subscriber = handshake_ok_then_failing_subscriber().await;

    let backend = storage();
    seed(&backend, topic_resource_type(), topic_resource()).await;
    let resource = subscription_resource("sub-1", "requested", &subscriber.uri());
    seed(&backend, "Subscription", resource.clone()).await;

    // `handle_delivery_failure` tests `off` first, so both at 1 sends the very
    // first failure straight to `off`.
    let engine = engine_with_thresholds(Arc::clone(&backend), 1, 1);
    engine
        .on_resource_event(subscription_written_event(resource, "sub-1"))
        .await;
    assert_eq!(stored_status(&backend, "sub-1").await, "active");

    engine.on_resource_event(encounter_created_event()).await;

    assert_eq!(
        engine
            .manager()
            .get_subscription(TENANT_ID, "sub-1")
            .expect("registered")
            .status,
        SubscriptionStatusCode::Off
    );
    assert_eq!(
        stored_status(&backend, "sub-1").await,
        "off",
        "the circuit breaker's `off` decision must survive a restart"
    );
}

/// Write-back must touch `status` and nothing else — in particular the
/// version-shape markers rehydration infers the FHIR version from
/// (`topic`/`channelType` vs `criteria`/`channel`) must survive, or the next
/// restart parses the resource as the wrong version.
#[tokio::test]
async fn write_back_preserves_every_other_field() {
    let subscriber = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&subscriber)
        .await;

    let backend = storage();
    seed(&backend, topic_resource_type(), topic_resource()).await;
    let mut resource = subscription_resource("sub-1", "requested", &subscriber.uri());
    resource["reason"] = json!("a field the engine knows nothing about");
    seed(&backend, "Subscription", resource.clone()).await;

    let engine = engine_with_writeback(Arc::clone(&backend));
    engine
        .on_resource_event(subscription_written_event(resource.clone(), "sub-1"))
        .await;

    let stored = backend
        .read(&tenant_context(TENANT_ID), "Subscription", "sub-1")
        .await
        .expect("read")
        .expect("exists");
    let content = stored.content();

    assert_eq!(content.get("status").unwrap(), "active");
    assert_eq!(
        content.get("reason").unwrap(),
        "a field the engine knows nothing about",
        "write-back must not drop unknown fields"
    );
    if uses_backport_ig() {
        assert!(content.get("criteria").is_some(), "R4 topic marker lost");
        assert!(content.get("channel").is_some(), "R4 channel marker lost");
    } else {
        assert!(content.get("topic").is_some(), "native topic marker lost");
        assert!(
            content.get("channelType").is_some(),
            "native channel marker lost"
        );
    }
}

/// Write-back is bounded: re-running activation against a resource that already
/// reads `active` must issue no write at all, so a restart with many
/// subscriptions does not produce a version-bump burst.
#[tokio::test]
async fn no_write_when_stored_status_already_matches() {
    let subscriber = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&subscriber)
        .await;

    let backend = storage();
    seed(&backend, topic_resource_type(), topic_resource()).await;
    let resource = subscription_resource("sub-1", "requested", &subscriber.uri());
    seed(&backend, "Subscription", resource.clone()).await;

    let engine = engine_with_writeback(Arc::clone(&backend));
    engine
        .on_resource_event(subscription_written_event(resource, "sub-1"))
        .await;
    let version_after_activation = stored_version(&backend, "sub-1").await;

    // Re-register from the now-`active` stored resource and re-run activation:
    // the status is unchanged, so no second version may be minted.
    let active_resource = subscription_resource("sub-1", "requested", &subscriber.uri());
    engine
        .on_resource_event(subscription_written_event(active_resource, "sub-1"))
        .await;

    assert_eq!(
        stored_version(&backend, "sub-1").await,
        version_after_activation,
        "a no-op status transition must not bump versionId"
    );
}

/// With no status store attached the engine behaves exactly as it did before
/// #357: transitions apply in memory and storage is untouched. This is the
/// `HFS_SUBSCRIPTION_PERSIST_STATUS=false` / embedder path.
#[tokio::test]
async fn without_a_status_store_storage_is_untouched() {
    let subscriber = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&subscriber)
        .await;

    let backend = storage();
    seed(&backend, topic_resource_type(), topic_resource()).await;
    let resource = subscription_resource("sub-1", "requested", &subscriber.uri());
    seed(&backend, "Subscription", resource.clone()).await;

    // Note: no `.with_status_store(...)`.
    let engine = SubscriptionEngine::new(
        SubscriptionConfig {
            max_retries: 0,
            handshake_max_attempts: 1,
            ..SubscriptionConfig::default()
        },
        "http://localhost:8080".to_string(),
    );
    engine
        .on_resource_event(subscription_written_event(resource, "sub-1"))
        .await;

    assert_eq!(
        engine
            .manager()
            .get_subscription(TENANT_ID, "sub-1")
            .expect("registered")
            .status,
        SubscriptionStatusCode::Active,
        "the in-memory transition must still happen"
    );
    assert_eq!(
        stored_status(&backend, "sub-1").await,
        "requested",
        "without a status store, storage must not be written"
    );
}

/// `persist_status = false` is the kill switch: the store is attached but the
/// engine must not write through it.
#[tokio::test]
async fn persist_status_false_suppresses_write_back() {
    let subscriber = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&subscriber)
        .await;

    let backend = storage();
    seed(&backend, topic_resource_type(), topic_resource()).await;
    let resource = subscription_resource("sub-1", "requested", &subscriber.uri());
    seed(&backend, "Subscription", resource.clone()).await;

    let engine = SubscriptionEngine::new(
        SubscriptionConfig {
            max_retries: 0,
            handshake_max_attempts: 1,
            persist_status: false,
            ..SubscriptionConfig::default()
        },
        "http://localhost:8080".to_string(),
    )
    .with_status_store(Arc::clone(&backend) as Arc<dyn ResourceStorage>);

    assert!(!engine.persists_status());

    engine
        .on_resource_event(subscription_written_event(resource, "sub-1"))
        .await;

    assert_eq!(
        stored_status(&backend, "sub-1").await,
        "requested",
        "HFS_SUBSCRIPTION_PERSIST_STATUS=false must suppress the write"
    );
}

/// A deleted subscription must not resurrect itself: if the resource is gone by
/// the time a transition lands, write-back is a no-op rather than an error or a
/// recreate.
#[tokio::test]
async fn write_back_on_a_deleted_subscription_is_a_no_op() {
    let subscriber = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&subscriber)
        .await;

    let backend = storage();
    seed(&backend, topic_resource_type(), topic_resource()).await;
    let resource = subscription_resource("sub-1", "requested", &subscriber.uri());
    seed(&backend, "Subscription", resource.clone()).await;

    let engine = engine_with_writeback(Arc::clone(&backend));

    // Register without activating, then delete the resource out from under it.
    engine
        .manager()
        .register(TENANT_ID, "sub-1", &resource, current_fhir_version())
        .expect("register");
    backend
        .delete(&tenant_context(TENANT_ID), "Subscription", "sub-1")
        .await
        .expect("delete");

    // Activation now transitions in memory and finds nothing to write onto.
    engine
        .on_resource_event(subscription_written_event(resource, "sub-1"))
        .await;

    let read_back = backend
        .read(&tenant_context(TENANT_ID), "Subscription", "sub-1")
        .await;
    match read_back {
        Ok(None) | Err(_) => {}
        Ok(Some(stored)) => panic!(
            "write-back resurrected a deleted subscription: {:?}",
            stored.content()
        ),
    }
}
