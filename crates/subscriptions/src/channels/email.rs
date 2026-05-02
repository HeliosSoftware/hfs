//! Email channel dispatcher.
//!
//! Delivers notification bundles via SMTP to subscribers whose `endpoint`
//! is a `mailto:` URI. The notification bundle is rendered as a short
//! human-readable summary body with the full FHIR Bundle attached as
//! `notification.json` (for FHIR JSON payloads).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use tracing::{debug, warn};

use crate::channels::{ChannelDispatcher, DispatchResult};
use crate::config::{SmtpEncryption, SmtpSettings};
use crate::error::SubscriptionError;
use crate::manager::{ActiveSubscription, PayloadContent};

const DEFAULT_FHIR_MIME: &str = "application/fhir+json";
const ATTACHMENT_FILENAME: &str = "notification.json";

/// Outcome of a single SMTP send used internally by the email channel.
#[derive(Debug)]
pub(crate) enum TransportOutcome {
    Delivered,
    Permanent(String),
    Transient(String),
}

/// Minimal async transport abstraction over [`lettre::AsyncTransport`] so we
/// can substitute a capturing stub in unit tests without running a real SMTP
/// server.
#[async_trait]
pub(crate) trait EmailTransport: Send + Sync {
    async fn send_message(&self, msg: Message) -> TransportOutcome;
}

struct LettreSmtpTransport {
    inner: AsyncSmtpTransport<Tokio1Executor>,
}

#[async_trait]
impl EmailTransport for LettreSmtpTransport {
    async fn send_message(&self, msg: Message) -> TransportOutcome {
        match self.inner.send(msg).await {
            Ok(_) => TransportOutcome::Delivered,
            Err(e) => classify_smtp_error(&e),
        }
    }
}

fn classify_smtp_error(e: &lettre::transport::smtp::Error) -> TransportOutcome {
    if e.is_permanent() {
        TransportOutcome::Permanent(e.to_string())
    } else {
        TransportOutcome::Transient(e.to_string())
    }
}

/// Email channel dispatcher.
///
/// Endpoints are expected in RFC 6068 `mailto:` form. Subscription headers
/// with `Subject:`, `From:`, `Reply-To:`, and `Cc:` override the server
/// defaults on a per-message basis. Full-resource payloads are refused
/// when the SMTP transport is unencrypted — analogous to the HTTPS
/// requirement on the rest-hook channel.
pub struct EmailChannel {
    transport: Arc<dyn EmailTransport>,
    settings: SmtpSettings,
}

impl EmailChannel {
    /// Build an `EmailChannel` backed by a real SMTP transport.
    pub fn new(settings: SmtpSettings) -> Result<Self, SubscriptionError> {
        let builder = match settings.encryption {
            SmtpEncryption::None => {
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(settings.host.clone())
            }
            SmtpEncryption::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(
                &settings.host,
            )
            .map_err(|e| SubscriptionError::Internal(format!("SMTP starttls builder: {e}")))?,
            SmtpEncryption::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&settings.host)
                .map_err(|e| SubscriptionError::Internal(format!("SMTP tls builder: {e}")))?,
        };

        let mut builder = builder
            .port(settings.port)
            .timeout(Some(Duration::from_secs(settings.timeout_secs)));

        if let (Some(user), Some(pass)) = (&settings.username, &settings.password) {
            builder = builder.credentials(Credentials::new(user.clone(), pass.clone()));
        }

        let inner = builder.build();
        Ok(Self {
            transport: Arc::new(LettreSmtpTransport { inner }),
            settings,
        })
    }

    /// Build an `EmailChannel` with an injected transport (for testing).
    #[cfg(test)]
    pub(crate) fn with_transport(
        transport: Arc<dyn EmailTransport>,
        settings: SmtpSettings,
    ) -> Self {
        Self {
            transport,
            settings,
        }
    }

    async fn send(
        &self,
        subscription: &ActiveSubscription,
        bundle: &serde_json::Value,
    ) -> Result<DispatchResult, SubscriptionError> {
        // 1. Resolve recipient from the mailto: endpoint.
        let endpoint = subscription.channel.endpoint.as_deref().ok_or_else(|| {
            SubscriptionError::InvalidEndpoint {
                message: "email channel requires a mailto: endpoint".to_string(),
            }
        })?;
        let to_mailbox = match parse_mailto(endpoint) {
            Ok(mb) => mb,
            Err(msg) => return Ok(DispatchResult::PermanentError(msg)),
        };

        // 2. Payload / MIME policy.
        let payload_mime = subscription
            .channel
            .payload_mime_type
            .as_deref()
            .unwrap_or(DEFAULT_FHIR_MIME)
            .to_string();

        if payload_mime == "application/fhir+xml" {
            return Ok(DispatchResult::PermanentError(
                "application/fhir+xml payload is not supported for email channel".to_string(),
            ));
        }

        // 3. Full-resource payloads require encrypted SMTP.
        if subscription.channel.payload_content == PayloadContent::FullResource
            && self.settings.encryption == SmtpEncryption::None
        {
            return Ok(DispatchResult::PermanentError(
                "full-resource payload requires encrypted SMTP (STARTTLS or TLS)".to_string(),
            ));
        }

        // 4. Build headers and subject using overrides from the subscription.
        let overrides = HeaderOverrides::from_subscription(&subscription.channel.headers);
        let from_mailbox = match overrides
            .from
            .as_deref()
            .unwrap_or(&self.settings.from_address)
            .parse::<Mailbox>()
        {
            Ok(mb) => mb,
            Err(e) => {
                return Ok(DispatchResult::PermanentError(format!(
                    "invalid From: address: {e}"
                )));
            }
        };
        let subject = overrides.subject.clone().unwrap_or_else(|| {
            render_subject(subscription, &self.settings.default_subject, bundle)
        });
        let body_text = render_body(subscription, bundle);

        // 5. Assemble the MIME message.
        let mut builder = Message::builder()
            .from(from_mailbox)
            .to(to_mailbox)
            .subject(subject);

        if let Some(reply_to) = overrides.reply_to.as_deref() {
            match reply_to.parse::<Mailbox>() {
                Ok(mb) => builder = builder.reply_to(mb),
                Err(e) => {
                    return Ok(DispatchResult::PermanentError(format!(
                        "invalid Reply-To: address: {e}"
                    )));
                }
            }
        }
        if let Some(cc) = overrides.cc.as_deref() {
            match cc.parse::<Mailbox>() {
                Ok(mb) => builder = builder.cc(mb),
                Err(e) => {
                    return Ok(DispatchResult::PermanentError(format!(
                        "invalid Cc: address: {e}"
                    )));
                }
            }
        }

        let message_result =
            if include_json_attachment(subscription.channel.payload_content, &payload_mime) {
                let bundle_bytes = serde_json::to_vec(bundle)
                    .map_err(|e| SubscriptionError::Internal(format!("serialize bundle: {e}")))?;
                let attachment_ct: ContentType =
                    payload_mime.parse().unwrap_or(ContentType::TEXT_PLAIN);
                let multipart = MultiPart::mixed()
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_PLAIN)
                            .body(body_text),
                    )
                    .singlepart(
                        Attachment::new(ATTACHMENT_FILENAME.to_string())
                            .body(bundle_bytes, attachment_ct),
                    );
                builder.multipart(multipart)
            } else {
                builder.header(ContentType::TEXT_PLAIN).body(body_text)
            };

        let msg = match message_result {
            Ok(m) => m,
            Err(e) => {
                return Ok(DispatchResult::PermanentError(format!(
                    "failed to build email message: {e}"
                )));
            }
        };

        debug!(
            subscription_id = %subscription.id,
            endpoint,
            "Dispatching email notification"
        );

        // 6. Hand off to the transport.
        match self.transport.send_message(msg).await {
            TransportOutcome::Delivered => {
                debug!(subscription_id = %subscription.id, "Email delivered");
                Ok(DispatchResult::Success)
            }
            TransportOutcome::Permanent(msg) => {
                warn!(
                    subscription_id = %subscription.id,
                    error = %msg,
                    "Email delivery failed (permanent)"
                );
                Ok(DispatchResult::PermanentError(msg))
            }
            TransportOutcome::Transient(msg) => {
                warn!(
                    subscription_id = %subscription.id,
                    error = %msg,
                    "Email delivery failed (transient)"
                );
                Ok(DispatchResult::RetryableError(msg))
            }
        }
    }
}

#[async_trait]
impl ChannelDispatcher for EmailChannel {
    async fn dispatch(
        &self,
        subscription: &ActiveSubscription,
        notification_bundle: &serde_json::Value,
    ) -> Result<DispatchResult, SubscriptionError> {
        self.send(subscription, notification_bundle).await
    }

    async fn handshake(
        &self,
        subscription: &ActiveSubscription,
        handshake_bundle: &serde_json::Value,
    ) -> Result<DispatchResult, SubscriptionError> {
        self.send(subscription, handshake_bundle).await
    }
}

fn include_json_attachment(payload: PayloadContent, mime: &str) -> bool {
    if payload == PayloadContent::Empty {
        return false;
    }
    matches!(mime, "application/fhir+json" | "application/json")
}

#[derive(Default, Debug)]
struct HeaderOverrides {
    from: Option<String>,
    subject: Option<String>,
    reply_to: Option<String>,
    cc: Option<String>,
}

impl HeaderOverrides {
    fn from_subscription(headers: &[String]) -> Self {
        let mut out = Self::default();
        for entry in headers {
            let Some((name, value)) = entry.split_once(':') else {
                continue;
            };
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if value.is_empty() {
                continue;
            }
            match name.as_str() {
                "from" => out.from = Some(value),
                "subject" => out.subject = Some(value),
                "reply-to" => out.reply_to = Some(value),
                "cc" => out.cc = Some(value),
                _ => {}
            }
        }
        out
    }
}

fn parse_mailto(endpoint: &str) -> Result<Mailbox, String> {
    let rest = endpoint
        .strip_prefix("mailto:")
        .ok_or_else(|| format!("email endpoint must be a mailto: URI: {endpoint}"))?;
    // RFC 6068 allows query strings like ?subject=...; drop anything after '?'.
    let address = rest.split('?').next().unwrap_or("").trim();
    if address.is_empty() {
        return Err("email endpoint is missing an address".to_string());
    }
    address
        .parse::<Mailbox>()
        .map_err(|e| format!("invalid email recipient '{address}': {e}"))
}

fn render_subject(
    subscription: &ActiveSubscription,
    default_template: &Option<String>,
    bundle: &serde_json::Value,
) -> String {
    let ntype = extract_notification_type(bundle).unwrap_or("notification");
    let topic = &subscription.topic_url;
    let event_num = subscription.events_since_start;

    if let Some(template) = default_template {
        template
            .replace("{notification-type}", ntype)
            .replace("{topic-url}", topic)
            .replace("{event-number}", &event_num.to_string())
    } else {
        format!("[HFS Subscription] {ntype} for {topic} (event {event_num})")
    }
}

fn render_body(subscription: &ActiveSubscription, bundle: &serde_json::Value) -> String {
    let ntype = extract_notification_type(bundle).unwrap_or("notification");
    let mut body = String::new();
    body.push_str(&format!("Subscription: {}\n", subscription.id));
    body.push_str(&format!("Topic: {}\n", subscription.topic_url));
    body.push_str(&format!("Notification type: {ntype}\n"));
    body.push_str(&format!(
        "Event number: {}\n",
        subscription.events_since_start
    ));
    body.push_str(&format!(
        "Payload content: {}\n",
        subscription.channel.payload_content.as_fhir_str()
    ));

    let foci = extract_focus_references(bundle);
    if !foci.is_empty() {
        body.push_str("\nFocus references:\n");
        for f in &foci {
            body.push_str(&format!("  - {f}\n"));
        }
    }

    if subscription.channel.payload_content != PayloadContent::Empty {
        body.push_str("\nSee attached notification.json for the full FHIR Bundle.\n");
    }

    body
}

/// Extract the notification type code from the first entry's resource.
///
/// Mirrors `NOTIFICATION_TYPE_JQ` in the external smoke script — handles both
/// R4 backport (Parameters) and native SubscriptionStatus bundles.
fn extract_notification_type(bundle: &serde_json::Value) -> Option<&str> {
    let first = bundle.pointer("/entry/0/resource")?;
    let resource_type = first.get("resourceType").and_then(|v| v.as_str())?;
    if resource_type == "Parameters" {
        first
            .get("parameter")?
            .as_array()?
            .iter()
            .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("type"))
            .and_then(|p| p.get("valueCode"))
            .and_then(|v| v.as_str())
    } else {
        first.get("type").and_then(|v| v.as_str())
    }
}

fn extract_focus_references(bundle: &serde_json::Value) -> Vec<String> {
    let Some(first) = bundle.pointer("/entry/0/resource") else {
        return Vec::new();
    };
    let resource_type = first
        .get("resourceType")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let mut out = Vec::new();
    if resource_type == "Parameters" {
        if let Some(params) = first.get("parameter").and_then(|v| v.as_array()) {
            for p in params {
                if p.get("name").and_then(|v| v.as_str()) != Some("notification-event") {
                    continue;
                }
                let Some(parts) = p.get("part").and_then(|v| v.as_array()) else {
                    continue;
                };
                for part in parts {
                    if part.get("name").and_then(|v| v.as_str()) == Some("focus") {
                        if let Some(r) = part
                            .pointer("/valueReference/reference")
                            .and_then(|v| v.as_str())
                        {
                            out.push(r.to_string());
                        }
                    }
                }
            }
        }
    } else if let Some(events) = first.get("notificationEvent").and_then(|v| v.as_array()) {
        for ev in events {
            if let Some(r) = ev.pointer("/focus/reference").and_then(|v| v.as_str()) {
                out.push(r.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::{ChannelConfig, ChannelType, SubscriptionStatusCode};
    use helios_fhir::FhirVersion;
    use serde_json::json;
    use std::sync::Mutex;

    struct CapturingTransport {
        sent: Mutex<Vec<Message>>,
        response: Mutex<TransportOutcome>,
    }

    impl CapturingTransport {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                sent: Mutex::new(Vec::new()),
                response: Mutex::new(TransportOutcome::Delivered),
            })
        }

        fn with_response(response: TransportOutcome) -> Arc<Self> {
            Arc::new(Self {
                sent: Mutex::new(Vec::new()),
                response: Mutex::new(response),
            })
        }

        fn take_sent(&self) -> Vec<Message> {
            std::mem::take(&mut self.sent.lock().unwrap())
        }
    }

    #[async_trait]
    impl EmailTransport for CapturingTransport {
        async fn send_message(&self, msg: Message) -> TransportOutcome {
            self.sent.lock().unwrap().push(msg);
            // Replace response with Delivered default after consuming once, so
            // subsequent sends succeed unless a new response is set.
            std::mem::replace(
                &mut *self.response.lock().unwrap(),
                TransportOutcome::Delivered,
            )
        }
    }

    fn smtp_settings(encryption: SmtpEncryption) -> SmtpSettings {
        SmtpSettings {
            host: "localhost".into(),
            port: 25,
            username: None,
            password: None,
            encryption,
            from_address: "hfs@example.test".into(),
            default_subject: None,
            timeout_secs: 10,
        }
    }

    fn sub(
        endpoint: Option<&str>,
        payload: PayloadContent,
        headers: Vec<String>,
        mime: Option<&str>,
    ) -> ActiveSubscription {
        ActiveSubscription {
            id: "sub-email-1".into(),
            topic_url: "http://example.org/topic/test".into(),
            status: SubscriptionStatusCode::Active,
            channel: ChannelConfig {
                channel_type: ChannelType::Email,
                endpoint: endpoint.map(str::to_string),
                payload_mime_type: mime.map(str::to_string),
                payload_content: payload,
                headers,
                heartbeat_period: None,
                timeout: None,
                max_count: None,
            },
            filters: vec![],
            fhir_version: FhirVersion::default(),
            events_since_start: 3,
            consecutive_failures: 0,
            tenant_id: "tenant-a".into(),
        }
    }

    fn native_bundle() -> serde_json::Value {
        json!({
            "resourceType": "Bundle",
            "type": "subscription-notification",
            "entry": [{
                "resource": {
                    "resourceType": "SubscriptionStatus",
                    "status": "active",
                    "type": "event-notification",
                    "eventsSinceSubscriptionStart": 3,
                    "topic": "http://example.org/topic/test",
                    "notificationEvent": [{
                        "eventNumber": 3,
                        "focus": { "reference": "Encounter/enc-1" }
                    }]
                }
            }]
        })
    }

    fn raw_body(msg: &Message) -> String {
        String::from_utf8(msg.formatted()).expect("utf-8 mime body")
    }

    #[tokio::test]
    async fn test_missing_endpoint() {
        let transport = CapturingTransport::new();
        let channel = EmailChannel::with_transport(transport, smtp_settings(SmtpEncryption::None));
        let s = sub(None, PayloadContent::IdOnly, vec![], None);
        let err = channel.dispatch(&s, &native_bundle()).await.unwrap_err();
        assert!(matches!(err, SubscriptionError::InvalidEndpoint { .. }));
    }

    #[tokio::test]
    async fn test_non_mailto_endpoint_permanent_error() {
        let transport = CapturingTransport::new();
        let channel = EmailChannel::with_transport(transport, smtp_settings(SmtpEncryption::None));
        let s = sub(
            Some("http://not-an-email"),
            PayloadContent::IdOnly,
            vec![],
            None,
        );
        let result = channel.dispatch(&s, &native_bundle()).await.unwrap();
        assert!(matches!(result, DispatchResult::PermanentError(_)));
    }

    #[tokio::test]
    async fn test_empty_payload_no_attachment() {
        let transport = CapturingTransport::new();
        let channel =
            EmailChannel::with_transport(transport.clone(), smtp_settings(SmtpEncryption::None));
        let s = sub(
            Some("mailto:nurse@example.test"),
            PayloadContent::Empty,
            vec![],
            None,
        );
        let result = channel.dispatch(&s, &native_bundle()).await.unwrap();
        assert!(matches!(result, DispatchResult::Success));
        let sent = transport.take_sent();
        assert_eq!(sent.len(), 1);
        let raw = raw_body(&sent[0]);
        assert!(
            !raw.contains("notification.json"),
            "attachment must be absent for empty payload"
        );
        assert!(raw.contains("Topic: http://example.org/topic/test"));
    }

    #[tokio::test]
    async fn test_id_only_payload_has_json_attachment() {
        let transport = CapturingTransport::new();
        let channel =
            EmailChannel::with_transport(transport.clone(), smtp_settings(SmtpEncryption::None));
        let s = sub(
            Some("mailto:nurse@example.test"),
            PayloadContent::IdOnly,
            vec![],
            None,
        );
        let bundle = native_bundle();
        let result = channel.dispatch(&s, &bundle).await.unwrap();
        assert!(matches!(result, DispatchResult::Success));
        let sent = transport.take_sent();
        assert_eq!(sent.len(), 1);
        let raw = raw_body(&sent[0]);
        assert!(
            raw.contains("notification.json"),
            "should include JSON attachment"
        );
        assert!(raw.contains("multipart/mixed"));
        assert!(raw.contains("application/fhir+json"));
    }

    #[tokio::test]
    async fn test_full_resource_over_plain_smtp_rejected() {
        let transport = CapturingTransport::new();
        let channel = EmailChannel::with_transport(transport, smtp_settings(SmtpEncryption::None));
        let s = sub(
            Some("mailto:nurse@example.test"),
            PayloadContent::FullResource,
            vec![],
            None,
        );
        let result = channel.dispatch(&s, &native_bundle()).await.unwrap();
        match result {
            DispatchResult::PermanentError(msg) => {
                assert!(
                    msg.to_lowercase().contains("encrypt"),
                    "expected encryption error, got: {msg}"
                );
            }
            other => panic!("expected PermanentError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_full_resource_over_starttls_allowed() {
        let transport = CapturingTransport::new();
        let channel = EmailChannel::with_transport(
            transport.clone(),
            smtp_settings(SmtpEncryption::StartTls),
        );
        let s = sub(
            Some("mailto:nurse@example.test"),
            PayloadContent::FullResource,
            vec![],
            None,
        );
        let result = channel.dispatch(&s, &native_bundle()).await.unwrap();
        assert!(matches!(result, DispatchResult::Success));
    }

    #[tokio::test]
    async fn test_subscription_overrides_from_and_subject_and_reply_to() {
        let transport = CapturingTransport::new();
        let channel =
            EmailChannel::with_transport(transport.clone(), smtp_settings(SmtpEncryption::None));
        let s = sub(
            Some("mailto:nurse@example.test"),
            PayloadContent::IdOnly,
            vec![
                "Subject: Custom Subject".into(),
                "From: override@example.com".into(),
                "Reply-To: reply@example.com".into(),
            ],
            None,
        );
        channel.dispatch(&s, &native_bundle()).await.unwrap();
        let sent = transport.take_sent();
        let raw = raw_body(&sent[0]);
        assert!(raw.contains("Subject: Custom Subject"));
        assert!(raw.contains("override@example.com"));
        assert!(raw.contains("reply@example.com"));
    }

    #[tokio::test]
    async fn test_server_default_from_when_no_override() {
        let transport = CapturingTransport::new();
        let channel =
            EmailChannel::with_transport(transport.clone(), smtp_settings(SmtpEncryption::None));
        let s = sub(
            Some("mailto:nurse@example.test"),
            PayloadContent::IdOnly,
            vec![],
            None,
        );
        channel.dispatch(&s, &native_bundle()).await.unwrap();
        let sent = transport.take_sent();
        let raw = raw_body(&sent[0]);
        assert!(raw.contains("hfs@example.test"));
    }

    #[tokio::test]
    async fn test_fhir_xml_payload_returns_permanent_error() {
        let transport = CapturingTransport::new();
        let channel = EmailChannel::with_transport(transport, smtp_settings(SmtpEncryption::None));
        let s = sub(
            Some("mailto:nurse@example.test"),
            PayloadContent::IdOnly,
            vec![],
            Some("application/fhir+xml"),
        );
        let result = channel.dispatch(&s, &native_bundle()).await.unwrap();
        assert!(matches!(result, DispatchResult::PermanentError(_)));
    }

    #[tokio::test]
    async fn test_handshake_uses_same_transport() {
        let transport = CapturingTransport::new();
        let channel =
            EmailChannel::with_transport(transport.clone(), smtp_settings(SmtpEncryption::None));
        let s = sub(
            Some("mailto:nurse@example.test"),
            PayloadContent::IdOnly,
            vec![],
            None,
        );
        let result = channel.handshake(&s, &native_bundle()).await.unwrap();
        assert!(matches!(result, DispatchResult::Success));
        assert_eq!(transport.take_sent().len(), 1);
    }

    #[tokio::test]
    async fn test_transient_transport_error_is_retryable() {
        let transport = CapturingTransport::with_response(TransportOutcome::Transient(
            "connection refused".into(),
        ));
        let channel = EmailChannel::with_transport(transport, smtp_settings(SmtpEncryption::None));
        let s = sub(
            Some("mailto:nurse@example.test"),
            PayloadContent::IdOnly,
            vec![],
            None,
        );
        let result = channel.dispatch(&s, &native_bundle()).await.unwrap();
        assert!(matches!(result, DispatchResult::RetryableError(_)));
    }

    #[tokio::test]
    async fn test_permanent_transport_error_is_permanent() {
        let transport = CapturingTransport::with_response(TransportOutcome::Permanent(
            "550 mailbox not found".into(),
        ));
        let channel = EmailChannel::with_transport(transport, smtp_settings(SmtpEncryption::None));
        let s = sub(
            Some("mailto:nurse@example.test"),
            PayloadContent::IdOnly,
            vec![],
            None,
        );
        let result = channel.dispatch(&s, &native_bundle()).await.unwrap();
        assert!(matches!(result, DispatchResult::PermanentError(_)));
    }

    #[tokio::test]
    async fn test_invalid_recipient_after_mailto_is_permanent() {
        let transport = CapturingTransport::new();
        let channel = EmailChannel::with_transport(transport, smtp_settings(SmtpEncryption::None));
        let s = sub(
            Some("mailto:not-an-email-address"),
            PayloadContent::IdOnly,
            vec![],
            None,
        );
        let result = channel.dispatch(&s, &native_bundle()).await.unwrap();
        assert!(matches!(result, DispatchResult::PermanentError(_)));
    }
}
