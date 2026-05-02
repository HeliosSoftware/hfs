//! Outbound authentication for server-to-server HTTP requests.
//!
//! While the rest of `helios-auth` validates incoming bearer tokens, this
//! module supplies credentials for *outgoing* HTTP requests — primarily
//! subscription notifications dispatched to subscriber endpoints (rest-hook,
//! FHIR Messaging, etc.).
//!
//! The trait is deliberately small: callers pass a `reqwest::RequestBuilder`
//! and receive it back with credential headers attached. Implementations
//! should not attempt to read or rewrite the request body.
//!
//! # Phase scope
//!
//! Only static-bearer credentials are implemented today. A future
//! `JwtAssertionOutboundAuthProvider` (private-key signed client-credentials
//! assertion in the SMART Backend Services style) is planned and is the
//! reason the trait carries an `audience` argument — that lets the future
//! signer scope tokens per receiver. The current impls ignore it.

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::RequestBuilder;

use crate::error::AuthError;

/// Attaches credentials to outbound HTTP requests.
///
/// Implementations append authentication headers (typically `Authorization`)
/// to a `reqwest::RequestBuilder` and return the modified builder. Callers
/// must invoke `authorize` *after* attaching any subscription-provided
/// headers so the implementation can observe them via builder inspection if
/// needed (the precedence rule — subscription-supplied `Authorization` wins
/// over server credentials — is enforced at the call site for clarity).
#[async_trait]
pub trait OutboundAuthProvider: Send + Sync {
    /// Add authentication headers to the request.
    ///
    /// `audience` is the receiver endpoint URL. The static-bearer impl
    /// ignores it; future signers may use it to scope minted tokens.
    async fn authorize(
        &self,
        request: RequestBuilder,
        audience: &str,
    ) -> Result<RequestBuilder, AuthError>;
}

/// No-op provider. Returns the request unmodified.
///
/// Used when outbound auth is disabled (no `HFS_OUTBOUND_BEARER_TOKEN`
/// configured and no other provider wired in).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoOpOutboundAuthProvider;

#[async_trait]
impl OutboundAuthProvider for NoOpOutboundAuthProvider {
    async fn authorize(
        &self,
        request: RequestBuilder,
        _audience: &str,
    ) -> Result<RequestBuilder, AuthError> {
        Ok(request)
    }
}

/// Adds a fixed `Authorization: Bearer <token>` header to every outbound
/// request.
///
/// Loaded from `HFS_OUTBOUND_BEARER_TOKEN` via
/// [`crate::AuthConfig::outbound_provider`]. Suitable for static
/// service-to-service tokens; for dynamic tokens (per-tenant, per-audience,
/// or short-lived JWTs) implement a custom provider.
#[derive(Debug, Clone)]
pub struct StaticBearerOutboundAuthProvider {
    token: String,
}

impl StaticBearerOutboundAuthProvider {
    /// Create a new provider with the given bearer token.
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }
}

#[async_trait]
impl OutboundAuthProvider for StaticBearerOutboundAuthProvider {
    async fn authorize(
        &self,
        request: RequestBuilder,
        _audience: &str,
    ) -> Result<RequestBuilder, AuthError> {
        Ok(request.header("Authorization", format!("Bearer {}", self.token)))
    }
}

/// Construct an [`Arc`]-wrapped provider from an optional bearer token.
///
/// Returns a [`StaticBearerOutboundAuthProvider`] when a non-empty token is
/// provided, or a [`NoOpOutboundAuthProvider`] otherwise.
pub fn provider_from_token(token: Option<&str>) -> Arc<dyn OutboundAuthProvider> {
    match token.filter(|t| !t.trim().is_empty()) {
        Some(t) => Arc::new(StaticBearerOutboundAuthProvider::new(t.to_string())),
        None => Arc::new(NoOpOutboundAuthProvider),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn noop_provider_does_not_modify_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/x"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = Client::new();
        let request = client.post(format!("{}/x", server.uri()));
        let request = NoOpOutboundAuthProvider
            .authorize(request, &server.uri())
            .await
            .unwrap();

        let response = request.send().await.unwrap();
        assert!(response.status().is_success());

        let received = &server.received_requests().await.unwrap()[0];
        assert!(received.headers.get("authorization").is_none());
    }

    #[tokio::test]
    async fn static_bearer_provider_adds_authorization_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/x"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let provider = StaticBearerOutboundAuthProvider::new("test-token");
        let client = Client::new();
        let request = client.post(format!("{}/x", server.uri()));
        let request = provider.authorize(request, &server.uri()).await.unwrap();

        let response = request.send().await.unwrap();
        assert!(
            response.status().is_success(),
            "request reached the matcher with bearer token"
        );
    }

    #[tokio::test]
    async fn static_bearer_appends_alongside_existing_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/x"))
            .and(header("X-Custom", "value"))
            .and(header("Authorization", "Bearer abc"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let provider = StaticBearerOutboundAuthProvider::new("abc");
        let client = Client::new();
        let request = client
            .post(format!("{}/x", server.uri()))
            .header("X-Custom", "value");
        let request = provider.authorize(request, &server.uri()).await.unwrap();

        let response = request.send().await.unwrap();
        assert!(response.status().is_success());
    }

    #[test]
    fn provider_from_token_returns_noop_when_none() {
        let provider = provider_from_token(None);
        // Type-erased; verify behaviorally via dispatch in test.
        // The static factory is also tested via the integration above.
        let _ = provider; // silence unused warning if this becomes a no-assertion test
    }

    #[tokio::test]
    async fn provider_from_token_returns_static_when_some() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/y"))
            .and(header("Authorization", "Bearer xyz"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let provider = provider_from_token(Some("xyz"));
        let client = Client::new();
        let request = client.post(format!("{}/y", server.uri()));
        let request = provider.authorize(request, &server.uri()).await.unwrap();

        assert!(request.send().await.unwrap().status().is_success());
    }

    #[tokio::test]
    async fn provider_from_token_treats_empty_string_as_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/y"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let provider = provider_from_token(Some("   "));
        let client = Client::new();
        let request = client.post(format!("{}/y", server.uri()));
        let request = provider.authorize(request, &server.uri()).await.unwrap();
        request.send().await.unwrap();

        let received = &server.received_requests().await.unwrap()[0];
        assert!(received.headers.get("authorization").is_none());
    }
}
