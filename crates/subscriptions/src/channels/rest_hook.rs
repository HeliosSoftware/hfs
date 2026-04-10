//! REST-hook channel dispatcher.
//!
//! Delivers notification bundles via HTTP POST to the subscriber's endpoint.

use async_trait::async_trait;
use reqwest::Client;
use std::time::Duration;
use tracing::{debug, warn};

use crate::channels::{ChannelDispatcher, DispatchResult};
use crate::error::SubscriptionError;
use crate::manager::{ActiveSubscription, PayloadContent};

/// REST-hook channel implementation.
///
/// Uses an HTTP client to POST notification bundles to subscriber endpoints.
/// Supports custom headers and TLS enforcement for full-resource payloads.
pub struct RestHookChannel {
    client: Client,
}

impl RestHookChannel {
    /// Creates a new REST-hook channel with default HTTP client settings.
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");

        Self { client }
    }

    /// Creates a new REST-hook channel with a custom HTTP client.
    pub fn with_client(client: Client) -> Self {
        Self { client }
    }

    /// Sends a notification bundle to the given endpoint.
    async fn send(
        &self,
        subscription: &ActiveSubscription,
        bundle: &serde_json::Value,
    ) -> Result<DispatchResult, SubscriptionError> {
        let endpoint = subscription.channel.endpoint.as_deref().ok_or_else(|| {
            SubscriptionError::InvalidEndpoint {
                message: "rest-hook channel requires an endpoint".to_string(),
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

        let mut request = self
            .client
            .post(endpoint)
            .header("Content-Type", "application/fhir+json")
            .json(bundle);

        // Add custom headers from the subscription.
        for header in &subscription.channel.headers {
            if let Some((name, value)) = header.split_once(':') {
                let name = name.trim();
                let value = value.trim();
                request = request.header(name, value);
            }
        }

        debug!(
            subscription_id = %subscription.id,
            endpoint,
            "Dispatching rest-hook notification"
        );

        match request.send().await {
            Ok(response) => {
                let status = response.status().as_u16();
                if response.status().is_success() {
                    debug!(
                        subscription_id = %subscription.id,
                        status,
                        "Notification delivered successfully"
                    );
                    Ok(DispatchResult::Success)
                } else if response.status().is_client_error() {
                    warn!(
                        subscription_id = %subscription.id,
                        status,
                        "Notification delivery failed (client error)"
                    );
                    Ok(DispatchResult::PermanentError(format!("HTTP {status}")))
                } else {
                    warn!(
                        subscription_id = %subscription.id,
                        status,
                        "Notification delivery failed (server error)"
                    );
                    Ok(DispatchResult::RetryableError(format!("HTTP {status}")))
                }
            }
            Err(e) => {
                if e.is_timeout() {
                    warn!(
                        subscription_id = %subscription.id,
                        "Notification delivery timed out"
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
                        "Notification delivery failed"
                    );
                    Ok(DispatchResult::RetryableError(e.to_string()))
                }
            }
        }
    }
}

impl Default for RestHookChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChannelDispatcher for RestHookChannel {
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
    use helios_fhir::FhirVersion;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_subscription(endpoint: &str, payload: PayloadContent) -> ActiveSubscription {
        ActiveSubscription {
            id: "sub-1".to_string(),
            topic_url: "http://example.org/topic/test".to_string(),
            status: SubscriptionStatusCode::Active,
            channel: ChannelConfig {
                channel_type: ChannelType::RestHook,
                endpoint: Some(endpoint.to_string()),
                payload_mime_type: Some("application/fhir+json".to_string()),
                payload_content: payload,
                headers: vec!["Authorization: Bearer test-token".to_string()],
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

    fn test_bundle() -> serde_json::Value {
        json!({"resourceType": "Bundle", "type": "history", "entry": []})
    }

    #[tokio::test]
    async fn test_successful_delivery() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/webhook"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = RestHookChannel::new();
        let sub = test_subscription(&format!("{}/webhook", server.uri()), PayloadContent::IdOnly);
        let result = channel.dispatch(&sub, &test_bundle()).await.unwrap();

        assert!(matches!(result, DispatchResult::Success));
    }

    #[tokio::test]
    async fn test_server_error_retryable() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/webhook"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let channel = RestHookChannel::new();
        let sub = test_subscription(&format!("{}/webhook", server.uri()), PayloadContent::IdOnly);
        let result = channel.dispatch(&sub, &test_bundle()).await.unwrap();

        assert!(matches!(result, DispatchResult::RetryableError(_)));
    }

    #[tokio::test]
    async fn test_client_error_permanent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/webhook"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let channel = RestHookChannel::new();
        let sub = test_subscription(&format!("{}/webhook", server.uri()), PayloadContent::IdOnly);
        let result = channel.dispatch(&sub, &test_bundle()).await.unwrap();

        assert!(matches!(result, DispatchResult::PermanentError(_)));
    }

    #[tokio::test]
    async fn test_custom_headers_sent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/webhook"))
            .and(wiremock::matchers::header(
                "Authorization",
                "Bearer test-token",
            ))
            .and(wiremock::matchers::header(
                "Content-Type",
                "application/fhir+json",
            ))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = RestHookChannel::new();
        let sub = test_subscription(&format!("{}/webhook", server.uri()), PayloadContent::IdOnly);
        let result = channel.dispatch(&sub, &test_bundle()).await.unwrap();

        assert!(matches!(result, DispatchResult::Success));
    }

    #[tokio::test]
    async fn test_tls_enforcement_for_full_resource() {
        let channel = RestHookChannel::new();
        let sub = test_subscription(
            "http://insecure.example.com/webhook",
            PayloadContent::FullResource,
        );
        let result = channel.dispatch(&sub, &test_bundle()).await.unwrap();

        assert!(matches!(result, DispatchResult::PermanentError(_)));
        if let DispatchResult::PermanentError(msg) = result {
            assert!(msg.contains("HTTPS"));
        }
    }

    #[tokio::test]
    async fn test_tls_not_required_for_id_only() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/webhook"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = RestHookChannel::new();
        // MockServer uses HTTP, which is fine for id-only.
        let sub = test_subscription(&format!("{}/webhook", server.uri()), PayloadContent::IdOnly);
        let result = channel.dispatch(&sub, &test_bundle()).await.unwrap();

        assert!(matches!(result, DispatchResult::Success));
    }

    #[tokio::test]
    async fn test_handshake_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/webhook"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let channel = RestHookChannel::new();
        let sub = test_subscription(&format!("{}/webhook", server.uri()), PayloadContent::IdOnly);
        let bundle = test_bundle();
        let result = channel.handshake(&sub, &bundle).await.unwrap();

        assert!(matches!(result, DispatchResult::Success));
    }

    #[tokio::test]
    async fn test_connection_error() {
        let channel = RestHookChannel::new();
        // Use a port that nothing is listening on.
        let sub = test_subscription("http://127.0.0.1:1/webhook", PayloadContent::IdOnly);
        let result = channel.dispatch(&sub, &test_bundle()).await.unwrap();

        assert!(matches!(result, DispatchResult::RetryableError(_)));
    }

    #[tokio::test]
    async fn test_missing_endpoint() {
        let channel = RestHookChannel::new();
        let mut sub = test_subscription("http://example.com/webhook", PayloadContent::IdOnly);
        sub.channel.endpoint = None;

        let result = channel.dispatch(&sub, &test_bundle()).await;
        assert!(result.is_err());
    }
}
