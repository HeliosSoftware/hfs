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
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_subscriptions::manager::SubscriptionStatusCode;
use helios_subscriptions::{
    RehydrationConfig, ResourceEvent, ResourceEventType, SubscriptionConfig, SubscriptionEngine,
    SubscriptionError,
};
use serde_json::{Value, json};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TENANT_ID: &str = "tenant-a";
const OTHER_TENANT_ID: &str = "tenant-b";
const TOPIC_URL: &str = "http://example.org/topic/encounter-start";

fn current_fhir_version() -> FhirVersion {
    FhirVersion::default()
}

fn enabled_versions() -> Vec<FhirVersion> {
    vec![
        #[cfg(feature = "R4")]
        FhirVersion::R4,
        #[cfg(feature = "R4B")]
        FhirVersion::R4B,
        #[cfg(feature = "R5")]
        FhirVersion::R5,
        #[cfg(feature = "R6")]
        FhirVersion::R6,
    ]
}

/// A topic in whichever shape the compiled default version stores: an R4
/// backport `Basic`, or a native `SubscriptionTopic`.
fn topic_resource() -> Value {
    topic_resource_for(current_fhir_version(), "topic-1", TOPIC_URL, "Encounter")
}

fn topic_resource_for(version: FhirVersion, id: &str, url: &str, trigger: &str) -> Value {
    if version.as_str() == "R4" {
        json!({
            "resourceType": "Basic",
            "id": id,
            "code": {
                "coding": [{
                    "system": "http://hl7.org/fhir/fhir-types",
                    "code": "SubscriptionTopic"
                }]
            },
            "extension": [{
                "url": "http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.url",
                "valueUri": url
            }, {
                "url": "http://hl7.org/fhir/4.3/StructureDefinition/extension-SubscriptionTopic.resourceTrigger",
                "extension": [{
                    "url": "resource",
                    "valueUri": format!("http://hl7.org/fhir/StructureDefinition/{trigger}")
                }, {
                    "url": "supportedInteraction",
                    "valueCode": "create"
                }]
            }]
        })
    } else {
        json!({
            "resourceType": "SubscriptionTopic",
            "id": id,
            "url": url,
            "status": "active",
            "resourceTrigger": [{
                "resource": trigger,
                "supportedInteraction": ["create"]
            }]
        })
    }
}

fn topic_resource_type() -> &'static str {
    topic_resource_type_for(current_fhir_version())
}

fn topic_resource_type_for(version: FhirVersion) -> &'static str {
    if version.as_str() == "R4" {
        "Basic"
    } else {
        "SubscriptionTopic"
    }
}

/// A subscription in the compiled default version's shape.
fn subscription_resource(id: &str, status: &str, endpoint: &str) -> Value {
    subscription_resource_for(current_fhir_version(), id, status, TOPIC_URL, endpoint)
}

fn subscription_resource_for(
    version: FhirVersion,
    id: &str,
    status: &str,
    url: &str,
    endpoint: &str,
) -> Value {
    if version.as_str() == "R4" {
        json!({
            "resourceType": "Subscription",
            "id": id,
            "status": status,
            "criteria": url,
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
            "topic": url,
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
    seed_for(
        backend,
        current_fhir_version(),
        tenant_id,
        resource_type,
        resource,
    )
    .await;
}

async fn seed_for(
    backend: &SqliteBackend,
    version: FhirVersion,
    tenant_id: &str,
    resource_type: &str,
    resource: Value,
) {
    backend
        .create(&tenant_context(tenant_id), resource_type, resource, version)
        .await
        .expect("seed resource");
}

fn notification_type(body: &Value) -> &str {
    let status = &body["entry"][0]["resource"];
    if status["resourceType"] == "Parameters" {
        status["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["name"] == "type")
            .unwrap()["valueCode"]
            .as_str()
            .unwrap()
    } else {
        status["type"].as_str().unwrap()
    }
}

async fn event_notifications(server: &MockServer, endpoint_path: Option<&str>) -> usize {
    let requests = server.received_requests().await.unwrap();
    requests
        .iter()
        .filter(|request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(
                notification_type(&body),
                "event-notification",
                "active subscriptions must not send handshakes"
            );
            endpoint_path.is_none_or(|path| request.url.path() == path)
        })
        .count()
}

async fn assert_event_delivery(
    engine: &SubscriptionEngine,
    server: &MockServer,
    version: FhirVersion,
    tenant: &str,
    resource_type: &str,
    endpoint_path: &str,
    expected: usize,
) {
    let total_before = event_notifications(server, None).await;
    let own_before = event_notifications(server, Some(endpoint_path)).await;
    let mut event = encounter_created_event(tenant);
    event.fhir_version = version;
    event.resource_type = resource_type.to_string();
    event.resource = Some(json!({"resourceType": resource_type, "id": "enc-1"}));
    tokio::time::timeout(Duration::from_secs(5), engine.on_resource_event(event))
        .await
        .expect("bounded event delivery");
    assert_eq!(
        event_notifications(server, None).await - total_before,
        expected,
        "{version:?} {tenant} {resource_type}: total delivery"
    );
    assert_eq!(
        event_notifications(server, Some(endpoint_path)).await - own_before,
        expected,
        "{version:?} {tenant} {resource_type}: owning tenant's endpoint"
    );
}

fn topic_lifecycle_event(
    version: FhirVersion,
    tenant: &str,
    id: &str,
    interaction: ResourceEventType,
    resource: Option<Value>,
) -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(tenant),
        fhir_version: version,
        resource_type: topic_resource_type_for(version).to_string(),
        resource_id: id.to_string(),
        version_id: "2".to_string(),
        event_type: interaction,
        resource,
        previous_resource: None,
        timestamp: Utc::now(),
    }
}

/// Each version executes real storage/export, topic parsing, registration,
/// evaluation and successful delivery. The second topic in tenant-b is restored
/// with a one-resource page size, then renamed or deleted without old payload.
#[tokio::test]
async fn shared_topics_and_their_lifecycle_survive_a_paginated_restart() {
    const RENAMED_URL: &str = "http://example.org/topic/renamed";
    const FIRST_URL: &str = "http://example.org/topic/first";
    for version in enabled_versions() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let backend = storage();
        for tenant in [TENANT_ID, OTHER_TENANT_ID] {
            backend.register_tenant(tenant, None).await.unwrap();
        }
        let resource_type = topic_resource_type_for(version);
        seed_for(
            &backend,
            version,
            TENANT_ID,
            resource_type,
            topic_resource_for(version, "topic-2", TOPIC_URL, "Encounter"),
        )
        .await;
        seed_for(
            &backend,
            version,
            OTHER_TENANT_ID,
            resource_type,
            topic_resource_for(version, "topic-1", FIRST_URL, "Patient"),
        )
        .await;
        seed_for(
            &backend,
            version,
            OTHER_TENANT_ID,
            resource_type,
            topic_resource_for(version, "topic-2", TOPIC_URL, "Observation"),
        )
        .await;
        for (tenant, endpoint) in [(TENANT_ID, "/a"), (OTHER_TENANT_ID, "/b")] {
            seed_for(
                &backend,
                version,
                tenant,
                "Subscription",
                subscription_resource_for(
                    version,
                    "same-sub-id",
                    "active",
                    TOPIC_URL,
                    &format!("{}{endpoint}", server.uri()),
                ),
            )
            .await;
        }
        let config = RehydrationConfig {
            batch_size: 1,
            handshake_requested: false,
            ..RehydrationConfig::default()
        };
        let engine = fresh_engine();
        assert!(engine.topic_registry().list_topics(TENANT_ID).is_empty());
        assert!(
            engine
                .manager()
                .get_subscription(TENANT_ID, "same-sub-id")
                .is_none()
        );
        let report = engine
            .rehydrate(&backend, TENANT_ID, version, &config)
            .await;
        assert!(report.is_complete(), "{version:?}: {report:?}");
        assert_eq!(report.tenants, 2);
        assert_eq!(report.topics_registered, 3);
        assert_eq!(report.topics_failed, 0);
        assert_eq!(report.subscriptions_registered, 2);
        assert_eq!(report.subscriptions_failed, 0);
        assert_eq!(report.handshakes_started, 0);
        assert_eq!(
            engine
                .topic_registry()
                .get_topic(TENANT_ID, TOPIC_URL)
                .unwrap()
                .resource_triggers[0]
                .resource_type,
            "Encounter"
        );
        assert_eq!(
            engine
                .topic_registry()
                .get_topic(OTHER_TENANT_ID, TOPIC_URL)
                .unwrap()
                .resource_triggers[0]
                .resource_type,
            "Observation"
        );
        assert_event_delivery(&engine, &server, version, TENANT_ID, "Encounter", "/a", 1).await;
        assert_event_delivery(&engine, &server, version, TENANT_ID, "Observation", "/a", 0).await;
        assert_event_delivery(
            &engine,
            &server,
            version,
            OTHER_TENANT_ID,
            "Observation",
            "/b",
            1,
        )
        .await;
        assert_event_delivery(
            &engine,
            &server,
            version,
            OTHER_TENANT_ID,
            "Encounter",
            "/b",
            0,
        )
        .await;

        // No previous_resource: removing the old URL must use the restored index.
        engine
            .on_resource_event(topic_lifecycle_event(
                version,
                OTHER_TENANT_ID,
                "topic-2",
                ResourceEventType::Update,
                Some(topic_resource_for(
                    version,
                    "topic-2",
                    RENAMED_URL,
                    "Observation",
                )),
            ))
            .await;
        assert!(
            engine
                .topic_registry()
                .get_topic(OTHER_TENANT_ID, TOPIC_URL)
                .is_none()
        );
        assert!(
            engine
                .topic_registry()
                .get_topic(OTHER_TENANT_ID, FIRST_URL)
                .is_some()
        );
        assert_event_delivery(&engine, &server, version, TENANT_ID, "Encounter", "/a", 1).await;
        assert_event_delivery(
            &engine,
            &server,
            version,
            OTHER_TENANT_ID,
            "Observation",
            "/b",
            0,
        )
        .await;
        engine
            .manager()
            .register(
                OTHER_TENANT_ID,
                "renamed-sub",
                &subscription_resource_for(
                    version,
                    "renamed-sub",
                    "active",
                    RENAMED_URL,
                    &format!("{}/b", server.uri()),
                ),
                version,
            )
            .unwrap();
        assert_event_delivery(
            &engine,
            &server,
            version,
            OTHER_TENANT_ID,
            "Observation",
            "/b",
            1,
        )
        .await;

        // A second fresh engine proves payload-free DELETE works directly from
        // the restart index, without relying on the live rename above.
        let restarted = fresh_engine();
        let report = restarted
            .rehydrate(&backend, TENANT_ID, version, &config)
            .await;
        assert_eq!(report.topics_registered, 3);
        assert_eq!(report.subscriptions_registered, 2);
        restarted
            .on_resource_event(topic_lifecycle_event(
                version,
                OTHER_TENANT_ID,
                "topic-2",
                ResourceEventType::Delete,
                None,
            ))
            .await;
        assert!(
            restarted
                .topic_registry()
                .get_topic(OTHER_TENANT_ID, TOPIC_URL)
                .is_none()
        );
        assert!(
            restarted
                .topic_registry()
                .get_topic(OTHER_TENANT_ID, FIRST_URL)
                .is_some()
        );
        assert_event_delivery(
            &restarted,
            &server,
            version,
            OTHER_TENANT_ID,
            "Observation",
            "/b",
            0,
        )
        .await;
        assert_event_delivery(
            &restarted,
            &server,
            version,
            TENANT_ID,
            "Encounter",
            "/a",
            1,
        )
        .await;
        println!(
            "{version:?}: paginated tenant-scoped restart, rename and payload-free delete delivered only to owning endpoints"
        );
    }
}

#[tokio::test]
async fn rehydration_rejects_a_stored_subscription_to_another_tenants_topic() {
    for version in enabled_versions() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let backend = storage();
        for tenant in [TENANT_ID, OTHER_TENANT_ID] {
            backend.register_tenant(tenant, None).await.unwrap();
        }
        seed_for(
            &backend,
            version,
            TENANT_ID,
            topic_resource_type_for(version),
            topic_resource_for(version, "topic-1", TOPIC_URL, "Encounter"),
        )
        .await;
        for (tenant, endpoint) in [(TENANT_ID, "/a"), (OTHER_TENANT_ID, "/b")] {
            seed_for(
                &backend,
                version,
                tenant,
                "Subscription",
                subscription_resource_for(
                    version,
                    "sub-1",
                    "active",
                    TOPIC_URL,
                    &format!("{}{endpoint}", server.uri()),
                ),
            )
            .await;
        }
        let engine = fresh_engine();
        let report = engine
            .rehydrate(&backend, TENANT_ID, version, &no_handshake_config())
            .await;
        assert!(report.is_complete(), "{version:?}: {report:?}");
        assert_eq!(report.topics_registered, 1);
        assert_eq!(report.subscriptions_registered, 1);
        assert_eq!(report.subscriptions_failed, 1);
        assert!(
            engine
                .manager()
                .get_subscription(OTHER_TENANT_ID, "sub-1")
                .is_none()
        );
        assert!(matches!(
            engine.manager().register(
                OTHER_TENANT_ID,
                "sub-1",
                &subscription_resource_for(
                    version,
                    "sub-1",
                    "active",
                    TOPIC_URL,
                    &format!("{}/b", server.uri())
                ),
                version
            ),
            Err(SubscriptionError::TopicNotFound { .. })
        ));
        assert_event_delivery(
            &engine,
            &server,
            version,
            OTHER_TENANT_ID,
            "Encounter",
            "/b",
            0,
        )
        .await;
        assert_event_delivery(&engine, &server, version, TENANT_ID, "Encounter", "/a", 1).await;
        println!(
            "{version:?}: foreign stored subscription rejected; failure report counted once; no foreign notification"
        );
    }
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
        engine
            .topic_registry()
            .get_topic(TENANT_ID, TOPIC_URL)
            .is_some(),
        "topic must be in the registry"
    );
}

/// `off` is terminal — it can transition nowhere and is never dispatched to —
/// but it is still **registered**, dormant (issue #357).
///
/// It used to be skipped outright. That was defensible only while the status
/// was volatile: once the delivery circuit breaker's decision survives a
/// restart, skipping means the subscription is absent from the engine forever,
/// so `$status` answers `404` for a resource that `GET /Subscription/{id}`
/// answers `200` for. Registering it dormant keeps the two endpoints telling the
/// same story, and cannot leak delivery because the evaluator selects only
/// `active`.
#[tokio::test]
async fn off_subscriptions_are_registered_dormant() {
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

    assert_eq!(report.subscriptions_dormant, 1, "report: {report:?}");
    assert_eq!(report.subscriptions_registered, 1, "report: {report:?}");
    assert_eq!(
        report.handshakes_started, 0,
        "a terminal subscription must never be handshaken: {report:?}"
    );

    let sub = engine
        .manager()
        .get_subscription(TENANT_ID, "sub-off")
        .expect("an `off` subscription must be registered so $status can answer");
    assert_eq!(sub.status, SubscriptionStatusCode::Off);

    // Registered, but inert: it must not be selected for delivery.
    assert!(
        engine
            .manager()
            .active_subscriptions_for_topic(TENANT_ID, TOPIC_URL)
            .is_empty(),
        "an `off` subscription must never be dispatched to"
    );
}

/// `entered-in-error` gets the same dormant treatment as `off`.
///
/// It previously failed to parse, so `register` fell back to `requested` — and
/// rehydration would then handshake and *activate* a subscription the client had
/// explicitly retracted.
#[tokio::test]
async fn entered_in_error_subscriptions_are_registered_dormant() {
    let backend = storage();
    seed(&backend, TENANT_ID, topic_resource_type(), topic_resource()).await;
    seed(
        &backend,
        TENANT_ID,
        "Subscription",
        subscription_resource("sub-eie", "entered-in-error", "http://127.0.0.1:1/hook"),
    )
    .await;

    let engine = fresh_engine();
    let report = engine
        .rehydrate(
            &backend,
            TENANT_ID,
            current_fhir_version(),
            &RehydrationConfig::default(), // handshakes ENABLED — must still not fire
        )
        .await;

    assert_eq!(report.subscriptions_dormant, 1, "report: {report:?}");
    assert_eq!(
        report.handshakes_started, 0,
        "a retracted subscription must never be activated: {report:?}"
    );
    let sub = engine
        .manager()
        .get_subscription(TENANT_ID, "sub-eie")
        .expect("registered so $status can answer");
    assert_eq!(sub.status, SubscriptionStatusCode::EnteredInError);
}

/// An `error` subscription is re-activated on restart, not left dormant.
///
/// Nothing in the engine performs `error` → `active` while a subscription is
/// dormant (the evaluator only selects `active`, so it is never dispatched to,
/// so it never succeeds, so nothing resets it). Before #357 the *loss* of the
/// status on restart is what quietly recovered it. Now that `error` persists,
/// re-running activation here is what stops a transient subscriber outage from
/// becoming permanent silent death.
#[tokio::test]
async fn error_subscriptions_are_reactivated_on_restart() {
    let backend = storage();
    seed(&backend, TENANT_ID, topic_resource_type(), topic_resource()).await;
    seed(
        &backend,
        TENANT_ID,
        "Subscription",
        subscription_resource("sub-err", "error", "http://127.0.0.1:1/hook"),
    )
    .await;

    let engine = fresh_engine();
    let report = engine
        .rehydrate(
            &backend,
            TENANT_ID,
            current_fhir_version(),
            &RehydrationConfig::default(),
        )
        .await;

    assert_eq!(report.subscriptions_registered, 1, "report: {report:?}");
    assert_eq!(
        report.handshakes_started, 1,
        "`error` must be handed to the activation path: {report:?}"
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

/// The core reason rehydration re-handshakes: status transitions are not
/// persisted, so a subscription the server activated in a previous lifetime
/// still reads `requested` from storage. Rehydration must drive it back through
/// the handshake and leave it `active` — otherwise the fix restores nothing for
/// exactly the subscriptions that were working before the restart.
#[tokio::test]
async fn requested_subscriptions_are_handshaked_and_activated() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/hook"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let backend = storage();
    seed(&backend, TENANT_ID, topic_resource_type(), topic_resource()).await;
    let endpoint = format!("{}/hook", server.uri());
    seed(
        &backend,
        TENANT_ID,
        "Subscription",
        subscription_resource("sub-req", "requested", &endpoint),
    )
    .await;

    let engine = fresh_engine();
    // Default config handshakes `requested` subscriptions — the behaviour under
    // test, and the opposite of `no_handshake_config`.
    let report = engine
        .rehydrate(
            &backend,
            TENANT_ID,
            current_fhir_version(),
            &RehydrationConfig::default(),
        )
        .await;

    assert_eq!(report.subscriptions_registered, 1, "report: {report:?}");
    assert_eq!(
        report.handshakes_started, 1,
        "a stored `requested` subscription must be scheduled for handshake: {report:?}"
    );

    // `rehydrate` awaits `activate_pending`, so the handshake has completed by
    // the time it returns.
    let sub = engine
        .manager()
        .get_subscription(TENANT_ID, "sub-req")
        .expect("subscription registered");
    assert_eq!(
        sub.status,
        SubscriptionStatusCode::Active,
        "a successful handshake must transition the subscription to active"
    );

    let requests = server
        .received_requests()
        .await
        .expect("mock server request log");
    assert_eq!(
        requests.len(),
        1,
        "rehydration must send exactly one handshake to the endpoint"
    );
}

/// Rehydration must page through storage without dropping or duplicating
/// resources. Forcing a batch size below the resource count exercises the
/// cursor-advance loop that a single-page fixture never reaches.
#[tokio::test]
async fn rehydration_pages_across_multiple_batches() {
    let backend = storage();
    seed(&backend, TENANT_ID, topic_resource_type(), topic_resource()).await;
    for id in ["sub-1", "sub-2", "sub-3"] {
        seed(
            &backend,
            TENANT_ID,
            "Subscription",
            subscription_resource(id, "active", "http://127.0.0.1:1/hook"),
        )
        .await;
    }

    let engine = fresh_engine();
    let paged_config = RehydrationConfig {
        batch_size: 1,
        handshake_requested: false,
        ..RehydrationConfig::default()
    };
    let report = engine
        .rehydrate(&backend, TENANT_ID, current_fhir_version(), &paged_config)
        .await;

    assert!(report.is_complete(), "report: {report:?}");
    assert_eq!(
        report.subscriptions_registered, 3,
        "every subscription across pages must be registered exactly once: {report:?}"
    );
    for id in ["sub-1", "sub-2", "sub-3"] {
        assert!(
            engine.manager().get_subscription(TENANT_ID, id).is_some(),
            "{id} must survive paged rehydration"
        );
    }
}
