//! End-to-end integration tests for the email subscription channel.
//!
//! Brings up a real mailpit container (SMTP + HTTP API) via testcontainers,
//! points the subscription engine at it, and asserts that handshake and
//! event-notification emails arrive with the expected From/To/Subject, body
//! content, and `notification.json` attachment shape. Requires Docker.

use std::time::Duration;

use chrono::Utc;
use helios_fhir::FhirVersion;
use helios_persistence::tenant::TenantId;
use helios_subscriptions::config::{SmtpEncryption, SmtpSettings};
use helios_subscriptions::manager::SubscriptionStatusCode;
use helios_subscriptions::{
    ResourceEvent, ResourceEventType, SubscriptionConfig, SubscriptionEngine, SubscriptionError,
};
use serde_json::{Value, json};
use testcontainers::core::{ContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage};

const TENANT_ID: &str = "tenant-email";
const TOPIC_URL: &str = "http://example.org/topic/email-test";

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

fn expected_bundle_type() -> &'static str {
    current_fhir_version().notification_bundle_type()
}

fn topic_resource() -> Value {
    if uses_backport_ig() {
        json!({
            "resourceType": "Basic",
            "id": "topic-email",
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
            "id": "topic-email",
            "url": TOPIC_URL,
            "status": "active",
            "resourceTrigger": [{
                "resource": "Encounter",
                "supportedInteraction": ["create"]
            }]
        })
    }
}

fn email_subscription_resource(
    sub_id: &str,
    endpoint: &str,
    content: &str,
    extra_headers: &[(&str, &str)],
) -> Value {
    if uses_backport_ig() {
        let mut channel = json!({
            "type": "email",
            "endpoint": endpoint,
            "payload": "application/fhir+json",
            "_payload": {
                "extension": [{
                    "url": "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-payload-content",
                    "valueCode": content
                }]
            }
        });
        if !extra_headers.is_empty() {
            let header_values: Vec<Value> = extra_headers
                .iter()
                .map(|(k, v)| Value::String(format!("{k}: {v}")))
                .collect();
            channel["header"] = Value::Array(header_values);
        }
        json!({
            "resourceType": "Subscription",
            "id": sub_id,
            "status": "requested",
            "criteria": TOPIC_URL,
            "channel": channel
        })
    } else {
        let parameters: Vec<Value> = extra_headers
            .iter()
            .map(|(k, v)| json!({ "name": k, "value": v }))
            .collect();
        let mut sub = json!({
            "resourceType": "Subscription",
            "id": sub_id,
            "status": "requested",
            "topic": TOPIC_URL,
            "channelType": {
                "system": "http://terminology.hl7.org/CodeSystem/subscription-channel-type",
                "code": "email"
            },
            "endpoint": endpoint,
            "contentType": "application/fhir+json",
            "content": content
        });
        if !parameters.is_empty() {
            sub["parameter"] = Value::Array(parameters);
        }
        sub
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
        resource_id: "topic-email".to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(resource),
        previous_resource: None,
        timestamp: Utc::now(),
    }
}

fn subscription_create_event(sub_id: &str, resource: Value) -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(TENANT_ID),
        fhir_version: current_fhir_version(),
        resource_type: "Subscription".to_string(),
        resource_id: sub_id.to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(resource),
        previous_resource: None,
        timestamp: Utc::now(),
    }
}

fn encounter_create_event(id: &str) -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(TENANT_ID),
        fhir_version: current_fhir_version(),
        resource_type: "Encounter".to_string(),
        resource_id: id.to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(json!({
            "resourceType": "Encounter",
            "id": id,
            "status": "in-progress"
        })),
        previous_resource: None,
        timestamp: Utc::now(),
    }
}

/// Probe whether Docker is reachable. Tests that need a mailpit container call
/// this first and return early when the daemon isn't available (e.g. local
/// macOS dev machines without Docker Desktop running). CI runners always have
/// Docker so this is purely a developer-experience aid.
fn docker_available() -> bool {
    if std::path::Path::new("/var/run/docker.sock").exists() {
        return true;
    }
    std::env::var("DOCKER_HOST").is_ok()
}

macro_rules! skip_if_no_docker {
    () => {
        if !docker_available() {
            eprintln!("skipping {}: Docker not available", function_name!());
            return;
        }
    };
}

macro_rules! function_name {
    () => {{
        fn f() {}
        fn type_name_of<T>(_: T) -> &'static str {
            std::any::type_name::<T>()
        }
        let name = type_name_of(f);
        // strip ::f suffix
        &name[..name.len() - 3]
    }};
}

struct MailpitHandle {
    #[allow(dead_code)]
    container: ContainerAsync<GenericImage>,
    smtp_host: String,
    smtp_port: u16,
    http_base: String,
    client: reqwest::Client,
}

impl MailpitHandle {
    async fn start() -> Self {
        let container = GenericImage::new("axllent/mailpit", "latest")
            .with_exposed_port(ContainerPort::Tcp(1025))
            .with_exposed_port(ContainerPort::Tcp(8025))
            .with_wait_for(WaitFor::message_on_stderr("accessible via"))
            .start()
            .await
            .expect("start mailpit container");
        let host = container
            .get_host()
            .await
            .expect("mailpit host")
            .to_string();
        let smtp_port = container
            .get_host_port_ipv4(ContainerPort::Tcp(1025))
            .await
            .expect("mailpit smtp port");
        let http_port = container
            .get_host_port_ipv4(ContainerPort::Tcp(8025))
            .await
            .expect("mailpit http port");

        let http_base = format!("http://{host}:{http_port}");
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client");

        // Poll readiness of the HTTP API.
        for _ in 0..30 {
            if client
                .get(format!("{http_base}/api/v1/info"))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false)
            {
                return Self {
                    container,
                    smtp_host: host,
                    smtp_port,
                    http_base,
                    client,
                };
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        panic!("mailpit HTTP API never became ready");
    }

    fn smtp_settings(&self, from: &str) -> SmtpSettings {
        SmtpSettings {
            host: self.smtp_host.clone(),
            port: self.smtp_port,
            username: None,
            password: None,
            encryption: SmtpEncryption::None,
            from_address: from.to_string(),
            default_subject: None,
            timeout_secs: 10,
        }
    }

    async fn clear(&self) {
        let _ = self
            .client
            .delete(format!("{}/api/v1/messages", self.http_base))
            .send()
            .await;
    }

    async fn wait_for_subject(&self, subject_substr: &str, min_count: usize) -> Vec<Value> {
        for _ in 0..60 {
            if let Ok(resp) = self
                .client
                .get(format!("{}/api/v1/messages?limit=100", self.http_base))
                .send()
                .await
            {
                if let Ok(body) = resp.json::<Value>().await {
                    let messages = body
                        .get("messages")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let matches: Vec<Value> = messages
                        .into_iter()
                        .filter(|m| {
                            m.get("Subject")
                                .and_then(Value::as_str)
                                .map(|s| s.contains(subject_substr))
                                .unwrap_or(false)
                        })
                        .collect();
                    if matches.len() >= min_count {
                        return matches;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        panic!(
            "timed out waiting for {min_count} mailpit messages with subject containing '{subject_substr}'"
        );
    }

    async fn fetch_attachment_json(&self, message_id: &str) -> Value {
        let detail: Value = self
            .client
            .get(format!("{}/api/v1/message/{}", self.http_base, message_id))
            .send()
            .await
            .expect("message detail")
            .json()
            .await
            .expect("message detail json");
        let attachments = detail
            .get("Attachments")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let att = attachments
            .into_iter()
            .find(|a| a.get("FileName").and_then(Value::as_str) == Some("notification.json"))
            .expect("notification.json attachment present");
        let part_id = att
            .get("PartID")
            .and_then(Value::as_str)
            .expect("part id")
            .to_string();
        let bytes = self
            .client
            .get(format!(
                "{}/api/v1/message/{}/part/{}",
                self.http_base, message_id, part_id
            ))
            .send()
            .await
            .expect("attachment body")
            .bytes()
            .await
            .expect("attachment bytes");
        serde_json::from_slice::<Value>(&bytes).expect("attachment JSON")
    }

    async fn message_detail(&self, message_id: &str) -> Value {
        self.client
            .get(format!("{}/api/v1/message/{}", self.http_base, message_id))
            .send()
            .await
            .expect("message detail")
            .json::<Value>()
            .await
            .expect("detail json")
    }
}

#[tokio::test]
async fn email_handshake_and_event_notification_end_to_end() {
    skip_if_no_docker!();
    let mailpit = MailpitHandle::start().await;
    mailpit.clear().await;

    let from = "hfs@integration.test";
    let engine = SubscriptionEngine::new(
        SubscriptionConfig {
            max_retries: 1,
            supported_channel_types: vec!["email".to_string()],
            smtp: Some(mailpit.smtp_settings(from)),
            ..Default::default()
        },
        "http://localhost:8080".to_string(),
    );

    engine.on_resource_event(topic_create_event()).await;

    let sub_resource = email_subscription_resource(
        "sub-email-happy",
        "mailto:nurse@integration.test",
        "id-only",
        &[],
    );
    engine
        .on_resource_event(subscription_create_event("sub-email-happy", sub_resource))
        .await;

    let registered = engine
        .manager()
        .get_subscription(TENANT_ID, "sub-email-happy")
        .expect("subscription registered");
    assert_eq!(registered.status, SubscriptionStatusCode::Active);

    let handshake_messages = mailpit.wait_for_subject("handshake", 1).await;
    assert_eq!(handshake_messages.len(), 1);

    engine
        .on_resource_event(encounter_create_event("enc-email-1"))
        .await;

    let event_messages = mailpit.wait_for_subject("event-notification", 1).await;
    assert_eq!(event_messages.len(), 1);

    let handshake_id = handshake_messages[0]
        .get("ID")
        .and_then(Value::as_str)
        .expect("handshake id");
    let handshake_detail = mailpit.message_detail(handshake_id).await;
    let from_addr = handshake_detail
        .pointer("/From/Address")
        .and_then(Value::as_str)
        .expect("From address");
    assert_eq!(from_addr, from);
    let to_addr = handshake_detail
        .pointer("/To/0/Address")
        .and_then(Value::as_str)
        .expect("To address");
    assert_eq!(to_addr, "nurse@integration.test");

    let handshake_bundle = mailpit.fetch_attachment_json(handshake_id).await;
    assert_eq!(handshake_bundle["resourceType"], "Bundle");
    assert_eq!(handshake_bundle["type"], expected_bundle_type());

    let event_id = event_messages[0]
        .get("ID")
        .and_then(Value::as_str)
        .expect("event id");
    let event_bundle = mailpit.fetch_attachment_json(event_id).await;
    assert_eq!(event_bundle["type"], expected_bundle_type());
    let entries = event_bundle["entry"]
        .as_array()
        .expect("event bundle has entries");
    let focus_match = entries.iter().any(|e| {
        e.pointer("/request/url").and_then(Value::as_str) == Some("Encounter/enc-email-1")
    });
    assert!(
        focus_match,
        "event bundle should include Encounter/enc-email-1 focus"
    );
}

#[tokio::test]
async fn email_empty_payload_sends_body_without_attachment() {
    skip_if_no_docker!();
    let mailpit = MailpitHandle::start().await;
    mailpit.clear().await;

    let engine = SubscriptionEngine::new(
        SubscriptionConfig {
            max_retries: 1,
            supported_channel_types: vec!["email".to_string()],
            smtp: Some(mailpit.smtp_settings("hfs@integration.test")),
            ..Default::default()
        },
        "http://localhost:8080".to_string(),
    );

    engine.on_resource_event(topic_create_event()).await;

    let sub = email_subscription_resource(
        "sub-email-empty",
        "mailto:audit@integration.test",
        "empty",
        &[],
    );
    engine
        .on_resource_event(subscription_create_event("sub-email-empty", sub))
        .await;

    let handshake = mailpit.wait_for_subject("handshake", 1).await;
    let detail = mailpit
        .message_detail(handshake[0].get("ID").and_then(Value::as_str).unwrap())
        .await;
    let attachments = detail
        .get("Attachments")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        attachments.is_empty(),
        "empty payload should have no attachments"
    );
    let text = detail
        .get("Text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        text.contains(TOPIC_URL),
        "body should mention the topic URL"
    );
}

#[tokio::test]
async fn email_full_resource_over_plain_smtp_is_rejected() {
    skip_if_no_docker!();
    let mailpit = MailpitHandle::start().await;
    mailpit.clear().await;

    let engine = SubscriptionEngine::new(
        SubscriptionConfig {
            max_retries: 1,
            error_threshold: 1,
            off_threshold: 10,
            supported_channel_types: vec!["email".to_string()],
            smtp: Some(mailpit.smtp_settings("hfs@integration.test")),
            ..Default::default()
        },
        "http://localhost:8080".to_string(),
    );

    engine.on_resource_event(topic_create_event()).await;

    let sub = email_subscription_resource(
        "sub-email-full",
        "mailto:nurse@integration.test",
        "full-resource",
        &[],
    );
    engine
        .on_resource_event(subscription_create_event("sub-email-full", sub))
        .await;

    // Handshake carries the same payload rules → permanent error → status → Error.
    let registered = engine
        .manager()
        .get_subscription(TENANT_ID, "sub-email-full")
        .expect("subscription registered");
    assert_eq!(registered.status, SubscriptionStatusCode::Error);
}

#[tokio::test]
async fn email_subscription_with_non_mailto_endpoint_is_rejected_at_registration() {
    let engine = SubscriptionEngine::new(
        SubscriptionConfig {
            max_retries: 1,
            supported_channel_types: vec!["email".to_string()],
            smtp: Some(SmtpSettings {
                host: "localhost".into(),
                port: 25,
                username: None,
                password: None,
                encryption: SmtpEncryption::None,
                from_address: "hfs@integration.test".into(),
                default_subject: None,
                timeout_secs: 5,
            }),
            ..Default::default()
        },
        "http://localhost:8080".to_string(),
    );

    engine.on_resource_event(topic_create_event()).await;

    let sub = email_subscription_resource("sub-email-bad", "http://not-an-email", "id-only", &[]);
    let err = engine
        .manager()
        .register(TENANT_ID, "sub-email-bad", &sub, current_fhir_version())
        .expect_err("non-mailto endpoint should be rejected");
    assert!(matches!(err, SubscriptionError::InvalidEndpoint { .. }));
}

#[tokio::test]
async fn email_subscription_header_overrides_subject() {
    skip_if_no_docker!();
    let mailpit = MailpitHandle::start().await;
    mailpit.clear().await;

    let engine = SubscriptionEngine::new(
        SubscriptionConfig {
            max_retries: 1,
            supported_channel_types: vec!["email".to_string()],
            smtp: Some(mailpit.smtp_settings("hfs@integration.test")),
            ..Default::default()
        },
        "http://localhost:8080".to_string(),
    );

    engine.on_resource_event(topic_create_event()).await;

    let sub = email_subscription_resource(
        "sub-email-subject",
        "mailto:ops@integration.test",
        "id-only",
        &[("Subject", "Integration Override Subject")],
    );
    engine
        .on_resource_event(subscription_create_event("sub-email-subject", sub))
        .await;

    let matches = mailpit
        .wait_for_subject("Integration Override Subject", 1)
        .await;
    assert_eq!(matches.len(), 1);
}
