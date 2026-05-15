//! FHIR Messaging channel dispatcher.
//!
//! Wraps notification bundles in a FHIR `Bundle.type = "message"` (with a
//! `MessageHeader` first entry) and POSTs them to the subscriber's
//! `$process-message` endpoint.
//!
//! # Security
//!
//! Three layers of protection:
//!
//! 1. **TLS enforcement** — full-resource payloads are rejected over plain
//!    HTTP (matches the rest-hook channel).
//! 2. **SSRF guard** — receiver endpoints whose host is a literal loopback /
//!    private / link-local / unspecified IP are rejected by default. Operators
//!    set `HFS_SUBSCRIPTION_ALLOW_PRIVATE_ENDPOINTS=true` for local development.
//! 3. **Outbound auth** — when `HFS_OUTBOUND_BEARER_TOKEN` is configured, an
//!    `Authorization: Bearer <token>` header is attached to every outgoing
//!    request. Subscription-supplied `Authorization` headers take precedence.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use helios_auth::OutboundAuthProvider;
use reqwest::{Client, Url};
use tracing::{debug, warn};

use crate::channels::{ChannelDispatcher, DispatchResult};
use crate::error::SubscriptionError;
use crate::manager::{ActiveSubscription, PayloadContent};
use crate::notification::{notification_type_from_bundle, wrap_as_message_bundle};

/// FHIR Messaging channel implementation.
pub struct MessagingChannel {
    client: Client,
    auth_provider: Arc<dyn OutboundAuthProvider>,
    source_endpoint: String,
    /// When `true`, dispatch to private/loopback IPs is allowed. Default
    /// `false`; tests and local dev opt in.
    private_endpoints_allowed: bool,
}

impl MessagingChannel {
    /// Create a messaging channel with the given source endpoint (HFS base
    /// URL) and outbound auth provider.
    pub fn new(source_endpoint: String, auth_provider: Arc<dyn OutboundAuthProvider>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            auth_provider,
            source_endpoint,
            private_endpoints_allowed: false,
        }
    }

    /// Replace the HTTP client (used by tests).
    pub fn with_client(mut self, client: Client) -> Self {
        self.client = client;
        self
    }

    /// Enable or disable dispatch to private/loopback IPs. Tests typically
    /// enable this to dispatch to `127.0.0.1` mock servers.
    pub fn with_private_endpoints_allowed(mut self, allowed: bool) -> Self {
        self.private_endpoints_allowed = allowed;
        self
    }

    async fn send(
        &self,
        subscription: &ActiveSubscription,
        bundle: &serde_json::Value,
    ) -> Result<DispatchResult, SubscriptionError> {
        let endpoint = subscription.channel.endpoint.as_deref().ok_or_else(|| {
            SubscriptionError::InvalidEndpoint {
                message: "messaging channel requires an endpoint".to_string(),
            }
        })?;

        // Enforce TLS for full-resource payloads.
        if subscription.channel.payload_content == PayloadContent::FullResource
            && !endpoint.starts_with("https://")
        {
            return Ok(DispatchResult::PermanentError(
                "full-resource payload requires HTTPS endpoint".to_string(),
            ));
        }

        // SSRF guard.
        if !self.private_endpoints_allowed {
            if let Some(reason) = check_endpoint_host(endpoint) {
                return Ok(DispatchResult::PermanentError(reason));
            }
        }

        // Wrap the prebuilt notification bundle as a message bundle.
        let notification_type = notification_type_from_bundle(bundle);
        let message_bundle = wrap_as_message_bundle(
            bundle,
            subscription,
            &self.source_endpoint,
            notification_type,
        )?;

        let mut request = self
            .client
            .post(endpoint)
            .header("Content-Type", "application/fhir+json")
            .json(&message_bundle);

        // Track whether the subscription supplies an Authorization header
        // BEFORE we attach the outbound provider's token — subscription
        // wins.
        let mut subscription_supplies_auth = false;
        for header in &subscription.channel.headers {
            if let Some((name, value)) = header.split_once(':') {
                let name = name.trim();
                let value = value.trim();
                if name.eq_ignore_ascii_case("authorization") {
                    subscription_supplies_auth = true;
                }
                request = request.header(name, value);
            }
        }

        if !subscription_supplies_auth {
            request = self.auth_provider.authorize(request, endpoint).await?;
        }

        debug!(
            subscription_id = %subscription.id,
            endpoint,
            "Dispatching messaging notification"
        );

        match request.send().await {
            Ok(response) => {
                let status = response.status().as_u16();
                if response.status().is_success() {
                    debug!(
                        subscription_id = %subscription.id,
                        status,
                        "Message delivered successfully"
                    );
                    Ok(DispatchResult::Success)
                } else if response.status().is_client_error() {
                    warn!(
                        subscription_id = %subscription.id,
                        status,
                        "Message delivery failed (client error)"
                    );
                    Ok(DispatchResult::PermanentError(format!("HTTP {status}")))
                } else {
                    warn!(
                        subscription_id = %subscription.id,
                        status,
                        "Message delivery failed (server error)"
                    );
                    Ok(DispatchResult::RetryableError(format!("HTTP {status}")))
                }
            }
            Err(e) => {
                if e.is_timeout() {
                    warn!(
                        subscription_id = %subscription.id,
                        "Message delivery timed out"
                    );
                    Ok(DispatchResult::RetryableError("timeout".to_string()))
                } else if e.is_connect() {
                    warn!(
                        subscription_id = %subscription.id,
                        error = %e,
                        "Connection failed"
                    );
                    Ok(DispatchResult::RetryableError(format!(
                        "connection error: {e}"
                    )))
                } else {
                    warn!(
                        subscription_id = %subscription.id,
                        error = %e,
                        "Message delivery failed"
                    );
                    Ok(DispatchResult::RetryableError(e.to_string()))
                }
            }
        }
    }
}

/// Inspect the receiver URL and return a rejection reason if its host is a
/// literal private/loopback/link-local/unspecified IP. DNS-resolved private
/// targets are not blocked by this check.
fn check_endpoint_host(endpoint: &str) -> Option<String> {
    let url = match Url::parse(endpoint) {
        Ok(u) => u,
        Err(_) => return Some(format!("invalid endpoint URL: {endpoint}")),
    };

    let host = url.host_str()?;
    let ip: IpAddr = match host.parse() {
        Ok(ip) => ip,
        Err(_) => return None, // hostname — not an SSRF concern at this layer
    };

    let is_blocked = match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified() || is_v6_private_or_link(&v6),
    };

    if is_blocked {
        Some(format!(
            "endpoint host {ip} is a private/loopback IP; set HFS_SUBSCRIPTION_ALLOW_PRIVATE_ENDPOINTS=true to allow"
        ))
    } else {
        None
    }
}

fn is_v6_private_or_link(addr: &std::net::Ipv6Addr) -> bool {
    let segments = addr.segments();
    // fc00::/7 — Unique Local Addresses (RFC 4193)
    let unique_local = (segments[0] & 0xfe00) == 0xfc00;
    // fe80::/10 — Link-Local Addresses
    let link_local = (segments[0] & 0xffc0) == 0xfe80;
    unique_local || link_local
}

#[async_trait]
impl ChannelDispatcher for MessagingChannel {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::{ChannelConfig, ChannelType, PayloadContent, SubscriptionStatusCode};
    use crate::notification::{NotificationType, build_event_notification, build_handshake};
    use chrono::Utc;
    use helios_auth::{NoOpOutboundAuthProvider, StaticBearerOutboundAuthProvider};
    use helios_fhir::FhirVersion;
    use serde_json::Value;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SOURCE: &str = "https://hfs.example.org/fhir";

    fn test_subscription(endpoint: &str, payload: PayloadContent) -> ActiveSubscription {
        ActiveSubscription {
            id: "sub-msg-1".to_string(),
            topic_url: "http://example.org/topic/test".to_string(),
            status: SubscriptionStatusCode::Active,
            channel: ChannelConfig {
                channel_type: ChannelType::Message,
                endpoint: Some(endpoint.to_string()),
                payload_mime_type: Some("application/fhir+json".to_string()),
                payload_content: payload,
                headers: vec![],
                heartbeat_period: None,
                timeout: None,
                max_count: None,
            },
            filters: vec![],
            fhir_version: FhirVersion::default(),
            events_since_start: 0,
            consecutive_failures: 0,
            tenant_id: "test".to_string(),
        }
    }

    fn channel_with_noop() -> MessagingChannel {
        MessagingChannel::new(SOURCE.to_string(), Arc::new(NoOpOutboundAuthProvider))
            .with_private_endpoints_allowed(true)
    }

    fn channel_with_bearer(token: &str) -> MessagingChannel {
        MessagingChannel::new(
            SOURCE.to_string(),
            Arc::new(StaticBearerOutboundAuthProvider::new(token)),
        )
        .with_private_endpoints_allowed(true)
    }

    fn handshake_bundle(sub: &ActiveSubscription) -> Value {
        build_handshake(sub, SOURCE).unwrap()
    }

    fn event_bundle(sub: &ActiveSubscription) -> Value {
        let event = crate::notification::NotificationEventData {
            event_number: 1,
            timestamp: Utc::now(),
            focus_reference: "Encounter/abc".to_string(),
        };
        let resource = serde_json::json!({"resourceType": "Encounter", "id": "abc"});
        build_event_notification(sub, event, Some(&resource), SOURCE).unwrap()
    }

    #[tokio::test]
    async fn successful_delivery() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/process-message"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = channel_with_noop();
        let sub = test_subscription(
            &format!("{}/process-message", server.uri()),
            PayloadContent::IdOnly,
        );
        let result = channel
            .dispatch(&sub, &handshake_bundle(&sub))
            .await
            .unwrap();

        assert!(matches!(result, DispatchResult::Success));
    }

    #[tokio::test]
    async fn server_error_is_retryable() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/x"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let channel = channel_with_noop();
        let sub = test_subscription(&format!("{}/x", server.uri()), PayloadContent::IdOnly);
        let result = channel
            .dispatch(&sub, &handshake_bundle(&sub))
            .await
            .unwrap();
        assert!(matches!(result, DispatchResult::RetryableError(_)));
    }

    #[tokio::test]
    async fn client_error_is_permanent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/x"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let channel = channel_with_noop();
        let sub = test_subscription(&format!("{}/x", server.uri()), PayloadContent::IdOnly);
        let result = channel
            .dispatch(&sub, &handshake_bundle(&sub))
            .await
            .unwrap();
        assert!(matches!(result, DispatchResult::PermanentError(_)));
    }

    #[tokio::test]
    async fn outbound_bearer_attached_when_subscription_does_not_supply_one() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/x"))
            .and(header("Authorization", "Bearer outbound-token"))
            .and(header("Content-Type", "application/fhir+json"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = channel_with_bearer("outbound-token");
        let sub = test_subscription(&format!("{}/x", server.uri()), PayloadContent::IdOnly);
        let result = channel
            .dispatch(&sub, &handshake_bundle(&sub))
            .await
            .unwrap();
        assert!(matches!(result, DispatchResult::Success));
    }

    #[tokio::test]
    async fn subscription_authorization_takes_precedence_over_outbound() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/x"))
            .and(header("Authorization", "Bearer subscriber-token"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = channel_with_bearer("outbound-token");
        let mut sub = test_subscription(&format!("{}/x", server.uri()), PayloadContent::IdOnly);
        sub.channel.headers = vec!["Authorization: Bearer subscriber-token".to_string()];
        let result = channel
            .dispatch(&sub, &handshake_bundle(&sub))
            .await
            .unwrap();
        assert!(
            matches!(result, DispatchResult::Success),
            "subscription-supplied Authorization header should take precedence; got {result:?}"
        );

        // Confirm the bearer token sent on the wire is the subscriber's, not
        // the outbound provider's.
        let received = &server.received_requests().await.unwrap()[0];
        let auth = received
            .headers
            .get("authorization")
            .expect("authorization header present");
        assert_eq!(auth.to_str().unwrap(), "Bearer subscriber-token");
    }

    #[tokio::test]
    async fn body_is_message_bundle_with_message_header_first() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/x"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = channel_with_noop();
        let sub = test_subscription(&format!("{}/x", server.uri()), PayloadContent::IdOnly);
        let _ = channel
            .dispatch(&sub, &handshake_bundle(&sub))
            .await
            .unwrap();

        let received = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&received.body).unwrap();
        assert_eq!(body["resourceType"], "Bundle");
        assert_eq!(body["type"], "message");
        assert_eq!(
            body["entry"][0]["resource"]["resourceType"],
            "MessageHeader"
        );
    }

    #[tokio::test]
    async fn handshake_bundle_carries_handshake_event_code() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/x"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = channel_with_noop();
        let sub = test_subscription(&format!("{}/x", server.uri()), PayloadContent::IdOnly);
        let _ = channel
            .handshake(&sub, &handshake_bundle(&sub))
            .await
            .unwrap();

        let received = &server.received_requests().await.unwrap()[0];
        let body: Value = serde_json::from_slice(&received.body).unwrap();
        let header = &body["entry"][0]["resource"];
        // R4 emits eventUri, R4B+ emits eventCoding; one of them must
        // identify this as a handshake.
        let handshake_code = header
            .get("eventCoding")
            .and_then(|c| c.get("code"))
            .and_then(Value::as_str);
        let event_uri = header.get("eventUri").and_then(Value::as_str);
        let nt = notification_type_from_bundle(&handshake_bundle(&sub));
        assert_eq!(nt, NotificationType::Handshake);
        if let Some(code) = handshake_code {
            assert_eq!(code, "handshake");
        } else {
            assert!(event_uri.is_some(), "R4 event URI must be set");
        }
    }

    #[tokio::test]
    async fn tls_enforced_for_full_resource() {
        let channel = channel_with_noop();
        let sub = test_subscription(
            "http://insecure.example.com/x",
            PayloadContent::FullResource,
        );
        let result = channel.dispatch(&sub, &event_bundle(&sub)).await.unwrap();
        assert!(matches!(result, DispatchResult::PermanentError(_)));
    }

    #[tokio::test]
    async fn missing_endpoint_errors() {
        let channel = channel_with_noop();
        let mut sub = test_subscription("http://example.com/x", PayloadContent::IdOnly);
        sub.channel.endpoint = None;
        let result = channel.dispatch(&sub, &handshake_bundle(&sub)).await;
        assert!(matches!(
            result,
            Err(SubscriptionError::InvalidEndpoint { .. })
        ));
    }

    #[tokio::test]
    async fn ssrf_guard_rejects_loopback_when_disabled() {
        // Default: private_endpoints_allowed = false.
        let channel = MessagingChannel::new(SOURCE.to_string(), Arc::new(NoOpOutboundAuthProvider));
        let sub = test_subscription("http://127.0.0.1:9/x", PayloadContent::IdOnly);
        let result = channel
            .dispatch(&sub, &handshake_bundle(&sub))
            .await
            .unwrap();
        match result {
            DispatchResult::PermanentError(msg) => {
                assert!(msg.contains("127.0.0.1"));
            }
            other => panic!("expected PermanentError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ssrf_guard_rejects_private_v4_when_disabled() {
        let channel = MessagingChannel::new(SOURCE.to_string(), Arc::new(NoOpOutboundAuthProvider));
        let sub = test_subscription("http://10.0.0.1:9/x", PayloadContent::IdOnly);
        let result = channel
            .dispatch(&sub, &handshake_bundle(&sub))
            .await
            .unwrap();
        assert!(matches!(result, DispatchResult::PermanentError(_)));
    }

    #[tokio::test]
    async fn ssrf_guard_allows_loopback_when_enabled() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/x"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = channel_with_noop(); // already has private_endpoints_allowed=true
        let sub = test_subscription(&format!("{}/x", server.uri()), PayloadContent::IdOnly);
        let result = channel
            .dispatch(&sub, &handshake_bundle(&sub))
            .await
            .unwrap();
        assert!(matches!(result, DispatchResult::Success));
    }

    #[tokio::test]
    async fn ssrf_guard_passes_through_dns_names() {
        // We don't block DNS-resolved hosts at this layer; rely on network policy.
        // Use a hostname that won't resolve so we get a connection error
        // (still proves the guard didn't trip).
        let channel = MessagingChannel::new(SOURCE.to_string(), Arc::new(NoOpOutboundAuthProvider));
        let sub = test_subscription(
            "http://nonexistent.invalid.example.test/x",
            PayloadContent::IdOnly,
        );
        let result = channel
            .dispatch(&sub, &handshake_bundle(&sub))
            .await
            .unwrap();
        // Connection error is retryable; the SSRF guard would have produced a
        // PermanentError with a recognizable message.
        match result {
            DispatchResult::RetryableError(_) => {}
            DispatchResult::PermanentError(msg) => {
                assert!(
                    !msg.contains("private/loopback"),
                    "DNS host should not trip SSRF guard, got: {msg}"
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn custom_subscription_headers_forwarded() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/x"))
            .and(header("X-Trace-Id", "trace-123"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = channel_with_noop();
        let mut sub = test_subscription(&format!("{}/x", server.uri()), PayloadContent::IdOnly);
        sub.channel.headers = vec!["X-Trace-Id: trace-123".to_string()];
        let result = channel
            .dispatch(&sub, &handshake_bundle(&sub))
            .await
            .unwrap();
        assert!(matches!(result, DispatchResult::Success));
    }
}
