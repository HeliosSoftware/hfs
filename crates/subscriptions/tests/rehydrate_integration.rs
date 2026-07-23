//! Startup rehydration of the subscription engine (issue #305).
//!
//! The regression these guard: the engine's registries are in-memory and were
//! populated only by live write handlers, so a restart left stored, active
//! subscriptions inert — they stopped matching and delivering while still
//! reading back fine over the REST API.
//!
//! Each test simulates a restart the same way the real bug manifests: resources
//! are written to a **real** storage backend, then a **fresh** `SubscriptionEngine`
//! is constructed — exactly what a process restart produces. Asserting the fresh
//! engine is inert *before* rehydration is what makes these regression tests
//! rather than feature tests: without that half, a no-op `rehydrate` would still
//! pass.

use chrono::Utc;
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_subscriptions::manager::SubscriptionStatusCode;
use helios_subscriptions::{
    RehydrationConfig, ResourceEvent, ResourceEventType, SubscriptionConfig, SubscriptionEngine,
};
use serde_json::{Value, json};
use std::time::Duration;

const TENANT_ID: &str = "tenant-a";
const OTHER_TENANT_ID: &str = "tenant-b";
const TOPIC_URL: &str = "http://example.org/topic/encounter-start";

fn current_fhir_version() -> FhirVersion {
    FhirVersion::default()
}

fn uses_backport_ig() -> bool {
    current_fhir_version() == FhirVersion::R4
}

/// A topic in whichever shape the compiled default version stores: an R4
/// backport `Basic`, or a native `SubscriptionTopic`.
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

/// A subscription in the compiled default version's shape.
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
            }
        })
    } else {
        json!({
            "resourceType": "Subscription",
            "id": id,
            "status": status,
            "topic": TOPIC_URL,
            "channelType": { "code": "rest-hook" },
            "endpoint": endpoint,
            "contentType": "application/fhir+json"
        })
    }
}

fn tenant_context(tenant_id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access())
}

/// An in-memory SQLite backend standing in for "the database that survived the
/// restart".
fn storage() -> SqliteBackend {
    let backend = SqliteBackend::in_memory().expect("in-memory sqlite backend");
    backend.init_schema().expect("init schema");
    backend
}

/// A brand-new engine — what a process restart yields: registries empty,
/// nothing loaded.
///
/// Retry/backoff is collapsed to near-zero: these tests dispatch to a dead
/// endpoint on purpose (the delivery attempt is not what is under test), and the
/// default policy would retry it ten times with exponential backoff, adding
/// minutes of wall-clock for no added signal.
fn fresh_engine() -> SubscriptionEngine {
    SubscriptionEngine::new(
        SubscriptionConfig {
            max_retries: 0,
            retry_initial_delay: Duration::from_millis(1),
            retry_max_delay: Duration::from_millis(1),
            handshake_max_attempts: 1,
            handshake_retry_initial_delay: Duration::from_millis(1),
            handshake_retry_max_delay: Duration::from_millis(1),
            ..SubscriptionConfig::default()
        },
        "http://localhost:8080".to_string(),
    )
}

/// A rehydration config that performs no outbound traffic, so tests assert on
/// registration alone without standing up mock endpoints.
fn no_handshake_config() -> RehydrationConfig {
    RehydrationConfig {
        handshake_requested: false,
        ..RehydrationConfig::default()
    }
}

fn encounter_created_event(tenant_id: &str) -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(tenant_id),
        fhir_version: current_fhir_version(),
        resource_type: "Encounter".to_string(),
        resource_id: "enc-1".to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(json!({ "resourceType": "Encounter", "id": "enc-1" })),
        previous_resource: None,
        timestamp: Utc::now(),
    }
}

async fn seed(backend: &SqliteBackend, tenant_id: &str, resource_type: &str, resource: Value) {
    backend
        .create(
            &tenant_context(tenant_id),
            resource_type,
            resource,
            current_fhir_version(),
        )
        .await
        .expect("seed resource");
}

/// The core regression: an `active` subscription stored before a restart must
/// resume matching after rehydration — and must NOT match before it, which is
/// what proves the test is exercising the fix rather than passing vacuously.
#[tokio::test]
async fn active_subscription_survives_restart() {
    let backend = storage();
    seed(&backend, TENANT_ID, topic_resource_type(), topic_resource()).await;
    seed(
        &backend,
        TENANT_ID,
        "Subscription",
        subscription_resource("sub-1", "active", "http://127.0.0.1:1/hook"),
    )
    .await;

    let engine = fresh_engine();

    // Pre-condition: this is the bug. A fresh engine knows nothing about the
    // stored subscription, so the event matches nothing.
    assert!(
        engine
            .manager()
            .get_subscription(TENANT_ID, "sub-1")
            .is_none(),
        "a fresh engine must not know the stored subscription — otherwise this \
         test would pass even with rehydration removed"
    );

    let report = engine
        .rehydrate(
            &backend,
            TENANT_ID,
            current_fhir_version(),
            &no_handshake_config(),
        )
        .await;

    assert!(report.is_complete(), "report: {report:?}");
    assert_eq!(report.topics_registered, 1, "report: {report:?}");
    assert_eq!(report.subscriptions_registered, 1, "report: {report:?}");

    let restored = engine
        .manager()
        .get_subscription(TENANT_ID, "sub-1")
        .expect("subscription registered after rehydration");
    assert_eq!(restored.status, SubscriptionStatusCode::Active);
    assert_eq!(restored.topic_url, TOPIC_URL);

    // And it is genuinely live: the engine now routes a matching event to it.
    // `on_resource_event` dispatches to a dead endpoint, which is fine — the
    // event counter proves the match happened.
    engine
        .on_resource_event(encounter_created_event(TENANT_ID))
        .await;
    let after = engine
        .manager()
        .get_subscription(TENANT_ID, "sub-1")
        .expect("still registered");
    assert!(
        after.events_since_start > 0,
        "a rehydrated active subscription must match events, got {after:?}"
    );
}

/// Topics must be registered before subscriptions: `register` rejects a
/// subscription whose topic is unknown, so a rehydration that scanned in the
/// wrong order would fail every subscription.
#[tokio::test]
async fn topic_is_registered_before_subscriptions() {
    let backend = storage();
    // Seed the subscription FIRST so storage ordering cannot accidentally
    // produce the correct result — the rehydration order must be what saves it.
    seed(
        &backend,
        TENANT_ID,
        "Subscription",
        subscription_resource("sub-1", "active", "http://127.0.0.1:1/hook"),
    )
    .await;
    seed(&backend, TENANT_ID, topic_resource_type(), topic_resource()).await;

    let engine = fresh_engine();
    let report = engine
        .rehydrate(
            &backend,
            TENANT_ID,
            current_fhir_version(),
            &no_handshake_config(),
        )
        .await;

    assert_eq!(
        report.subscriptions_failed, 0,
        "topics must be registered before subscriptions; report: {report:?}"
    );
    assert_eq!(report.subscriptions_registered, 1, "report: {report:?}");
    assert!(
        engine.topic_registry().get_topic(TOPIC_URL).is_some(),
        "topic must be in the registry"
    );
}

/// `off` is a terminal status — it can transition nowhere — so rehydrating one
/// would register a subscription that could never legally become useful.
#[tokio::test]
async fn off_subscriptions_are_skipped() {
    let backend = storage();
    seed(&backend, TENANT_ID, topic_resource_type(), topic_resource()).await;
    seed(
        &backend,
        TENANT_ID,
        "Subscription",
        subscription_resource("sub-off", "off", "http://127.0.0.1:1/hook"),
    )
    .await;

    let engine = fresh_engine();
    let report = engine
        .rehydrate(
            &backend,
            TENANT_ID,
            current_fhir_version(),
            &no_handshake_config(),
        )
        .await;

    assert_eq!(report.subscriptions_skipped, 1, "report: {report:?}");
    assert_eq!(report.subscriptions_registered, 0, "report: {report:?}");
    assert!(
        engine
            .manager()
            .get_subscription(TENANT_ID, "sub-off")
            .is_none(),
        "an `off` subscription must not be registered"
    );
}

/// Rehydration covers every tenant, not just the default one — otherwise a
/// multi-tenant deployment silently restores only one tenant's subscriptions.
#[tokio::test]
async fn rehydration_covers_every_tenant() {
    let backend = storage();
    for tenant in [TENANT_ID, OTHER_TENANT_ID] {
        backend
            .register_tenant(tenant, None)
            .await
            .expect("register tenant");
        seed(&backend, tenant, topic_resource_type(), topic_resource()).await;
        seed(
            &backend,
            tenant,
            "Subscription",
            subscription_resource("sub-1", "active", "http://127.0.0.1:1/hook"),
        )
        .await;
    }

    let engine = fresh_engine();
    let report = engine
        .rehydrate(
            &backend,
            TENANT_ID,
            current_fhir_version(),
            &no_handshake_config(),
        )
        .await;

    assert!(report.is_complete(), "report: {report:?}");
    assert_eq!(report.subscriptions_registered, 2, "report: {report:?}");
    for tenant in [TENANT_ID, OTHER_TENANT_ID] {
        assert!(
            engine.manager().get_subscription(tenant, "sub-1").is_some(),
            "tenant {tenant} must be rehydrated"
        );
    }
}

/// Subscriptions are tenant-scoped after rehydration: an event in one tenant
/// must not deliver to another tenant's subscription.
#[tokio::test]
async fn rehydrated_subscriptions_stay_tenant_isolated() {
    let backend = storage();
    for tenant in [TENANT_ID, OTHER_TENANT_ID] {
        backend
            .register_tenant(tenant, None)
            .await
            .expect("register tenant");
        seed(&backend, tenant, topic_resource_type(), topic_resource()).await;
    }
    seed(
        &backend,
        TENANT_ID,
        "Subscription",
        subscription_resource("sub-a", "active", "http://127.0.0.1:1/hook"),
    )
    .await;

    let engine = fresh_engine();
    engine
        .rehydrate(
            &backend,
            TENANT_ID,
            current_fhir_version(),
            &no_handshake_config(),
        )
        .await;

    // An event in the OTHER tenant must not touch tenant-a's subscription.
    engine
        .on_resource_event(encounter_created_event(OTHER_TENANT_ID))
        .await;
    let sub_a = engine
        .manager()
        .get_subscription(TENANT_ID, "sub-a")
        .expect("tenant-a subscription registered");
    assert_eq!(
        sub_a.events_since_start, 0,
        "a cross-tenant event must not be delivered to tenant-a"
    );
}

/// Disabling rehydration must be a true no-op, so operators can opt out without
/// the engine touching storage at all.
#[tokio::test]
async fn disabled_rehydration_is_a_no_op() {
    let backend = storage();
    seed(&backend, TENANT_ID, topic_resource_type(), topic_resource()).await;
    seed(
        &backend,
        TENANT_ID,
        "Subscription",
        subscription_resource("sub-1", "active", "http://127.0.0.1:1/hook"),
    )
    .await;

    let engine = fresh_engine();
    let report = engine
        .rehydrate(
            &backend,
            TENANT_ID,
            current_fhir_version(),
            &RehydrationConfig {
                enabled: false,
                ..RehydrationConfig::default()
            },
        )
        .await;

    assert_eq!(report, Default::default(), "disabled pass must do nothing");
    assert!(
        engine
            .manager()
            .get_subscription(TENANT_ID, "sub-1")
            .is_none()
    );
}

/// A deleted subscription must not come back. `fetch_export_batch` filters
/// deleted resources, and this pins that behaviour so a backend change cannot
/// silently resurrect subscriptions a client removed.
#[tokio::test]
async fn deleted_subscriptions_are_not_rehydrated() {
    let backend = storage();
    seed(&backend, TENANT_ID, topic_resource_type(), topic_resource()).await;
    seed(
        &backend,
        TENANT_ID,
        "Subscription",
        subscription_resource("sub-gone", "active", "http://127.0.0.1:1/hook"),
    )
    .await;
    backend
        .delete(&tenant_context(TENANT_ID), "Subscription", "sub-gone")
        .await
        .expect("delete subscription");

    let engine = fresh_engine();
    let report = engine
        .rehydrate(
            &backend,
            TENANT_ID,
            current_fhir_version(),
            &no_handshake_config(),
        )
        .await;

    assert_eq!(report.subscriptions_registered, 0, "report: {report:?}");
    assert!(
        engine
            .manager()
            .get_subscription(TENANT_ID, "sub-gone")
            .is_none(),
        "a deleted subscription must not be resurrected by rehydration"
    );
}

/// An unparseable subscription must not abort the pass: the others still load.
#[tokio::test]
async fn one_bad_resource_does_not_stop_the_scan() {
    let backend = storage();
    seed(&backend, TENANT_ID, topic_resource_type(), topic_resource()).await;
    // Missing topic reference entirely — `register` will reject it.
    seed(
        &backend,
        TENANT_ID,
        "Subscription",
        json!({ "resourceType": "Subscription", "id": "sub-bad", "status": "active" }),
    )
    .await;
    seed(
        &backend,
        TENANT_ID,
        "Subscription",
        subscription_resource("sub-good", "active", "http://127.0.0.1:1/hook"),
    )
    .await;

    let engine = fresh_engine();
    let report = engine
        .rehydrate(
            &backend,
            TENANT_ID,
            current_fhir_version(),
            &no_handshake_config(),
        )
        .await;

    assert_eq!(report.subscriptions_failed, 1, "report: {report:?}");
    assert_eq!(report.subscriptions_registered, 1, "report: {report:?}");
    assert!(
        report.is_complete(),
        "a per-resource failure must not mark the pass incomplete: {report:?}"
    );
    assert!(
        engine
            .manager()
            .get_subscription(TENANT_ID, "sub-good")
            .is_some(),
        "the good subscription must still be registered"
    );
}

/// Rehydration is idempotent: running it twice (a crash-loop, or a future
/// periodic refresh) must not duplicate or corrupt registrations.
#[tokio::test]
async fn rehydration_is_idempotent() {
    let backend = storage();
    seed(&backend, TENANT_ID, topic_resource_type(), topic_resource()).await;
    seed(
        &backend,
        TENANT_ID,
        "Subscription",
        subscription_resource("sub-1", "active", "http://127.0.0.1:1/hook"),
    )
    .await;

    let engine = fresh_engine();
    let first = engine
        .rehydrate(
            &backend,
            TENANT_ID,
            current_fhir_version(),
            &no_handshake_config(),
        )
        .await;
    let second = engine
        .rehydrate(
            &backend,
            TENANT_ID,
            current_fhir_version(),
            &no_handshake_config(),
        )
        .await;

    assert_eq!(
        first.subscriptions_registered, second.subscriptions_registered,
        "a second pass must register the same set"
    );
    let sub = engine
        .manager()
        .get_subscription(TENANT_ID, "sub-1")
        .expect("still registered");
    assert_eq!(sub.status, SubscriptionStatusCode::Active);
}
