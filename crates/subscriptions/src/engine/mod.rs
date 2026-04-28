//! Subscription engine — the orchestrator.
//!
//! Ties together topic registry, subscription manager, event evaluator,
//! notification builder, and channel dispatchers. Invoked asynchronously
//! after each resource write.

pub mod retry;

use std::sync::Arc;

use dashmap::DashMap;
use tracing::{debug, error, info, warn};

use crate::channels::email::EmailChannel;
use crate::channels::messaging::MessagingChannel;
use crate::channels::rest_hook::RestHookChannel;
use crate::channels::websocket::WebSocketChannel;
use crate::channels::ws_manager::WebSocketManager;
use crate::channels::ws_token::WsBindingTokenManager;
use crate::channels::{ChannelDispatcher, DispatchResult};
use crate::config::SubscriptionConfig;
use crate::evaluator::EventEvaluator;
use crate::event::{ResourceEvent, ResourceEventType};
use crate::manager::{
    ActiveSubscription, ChannelType, SubscriptionManager, SubscriptionStatusCode,
};
use crate::notification::{self, NotificationEventData};
use crate::topics::InMemoryTopicRegistry;
use helios_auth::{NoOpOutboundAuthProvider, OutboundAuthProvider};

/// The subscription engine orchestrates the entire subscription pipeline.
///
/// It is designed to be invoked asynchronously after resource writes
/// (fire-and-forget via `tokio::spawn`), mirroring the audit middleware pattern.
pub struct SubscriptionEngine {
    topic_registry: Arc<InMemoryTopicRegistry>,
    topic_resource_index: DashMap<(String, String, String), String>,
    manager: Arc<SubscriptionManager>,
    evaluator: EventEvaluator,
    rest_hook_channel: Arc<RestHookChannel>,
    ws_manager: Arc<WebSocketManager>,
    ws_channel: Arc<WebSocketChannel>,
    ws_token_manager: Arc<WsBindingTokenManager>,
    email_channel: Option<Arc<EmailChannel>>,
    messaging_channel: Option<Arc<MessagingChannel>>,
    config: SubscriptionConfig,
    base_url: String,
}

impl SubscriptionEngine {
    /// Creates a new subscription engine with a no-op outbound auth provider.
    pub fn new(config: SubscriptionConfig, base_url: String) -> Self {
        Self::with_outbound_auth(config, base_url, Arc::new(NoOpOutboundAuthProvider))
    }

    /// Creates a new subscription engine with a custom outbound auth provider.
    ///
    /// The provider is used by channels that initiate outbound HTTP requests
    /// (currently the FHIR Messaging channel) to attach server-side
    /// credentials when the subscription does not supply its own
    /// `Authorization` header.
    pub fn with_outbound_auth(
        config: SubscriptionConfig,
        base_url: String,
        outbound_auth: Arc<dyn OutboundAuthProvider>,
    ) -> Self {
        let topic_registry = Arc::new(InMemoryTopicRegistry::new());
        let manager = Arc::new(SubscriptionManager::new(
            Arc::clone(&topic_registry),
            config.supported_channel_types.clone(),
        ));
        let evaluator = EventEvaluator::new(Arc::clone(&topic_registry), Arc::clone(&manager));
        let rest_hook_channel = Arc::new(RestHookChannel::new());
        let ws_manager = Arc::new(WebSocketManager::new());
        let ws_channel = Arc::new(WebSocketChannel::new(Arc::clone(&ws_manager)));
        let ws_token_manager = Arc::new(WsBindingTokenManager::new(config.ws_token_lifetime_secs));
        let email_channel = match &config.smtp {
            Some(settings) => match EmailChannel::new(settings.clone()) {
                Ok(ch) => Some(Arc::new(ch)),
                Err(e) => {
                    warn!(error = %e, "Failed to initialize email channel; email dispatch disabled");
                    None
                }
            },
            None => None,
        };
        let messaging_channel = config.messaging.as_ref().map(|settings| {
            Arc::new(
                MessagingChannel::new(settings.source_endpoint.clone(), Arc::clone(&outbound_auth))
                    .with_private_endpoints_allowed(settings.allow_private_endpoints),
            )
        });

        Self {
            topic_registry,
            topic_resource_index: DashMap::new(),
            manager,
            evaluator,
            rest_hook_channel,
            ws_manager,
            ws_channel,
            ws_token_manager,
            email_channel,
            messaging_channel,
            config,
            base_url,
        }
    }

    /// Returns a reference to the topic registry.
    pub fn topic_registry(&self) -> &Arc<InMemoryTopicRegistry> {
        &self.topic_registry
    }

    /// Returns a reference to the subscription manager.
    pub fn manager(&self) -> &Arc<SubscriptionManager> {
        &self.manager
    }

    /// Returns a reference to the WebSocket manager.
    pub fn ws_manager(&self) -> &Arc<WebSocketManager> {
        &self.ws_manager
    }

    /// Returns a reference to the WebSocket binding token manager.
    pub fn ws_token_manager(&self) -> &Arc<WsBindingTokenManager> {
        &self.ws_token_manager
    }

    /// Called after a resource write has been committed.
    ///
    /// This method:
    /// 1. Handles subscription/topic lifecycle events (if the written resource
    ///    is a Subscription, SubscriptionTopic, or an R4 backport Basic topic).
    /// 2. Evaluates the event against all active subscriptions.
    /// 3. Builds and dispatches notifications.
    pub async fn on_resource_event(&self, event: ResourceEvent) {
        // Handle subscription lifecycle events.
        match event.resource_type.as_str() {
            "Subscription" => {
                self.handle_subscription_event(&event).await;
                return;
            }
            "SubscriptionTopic" => {
                self.handle_topic_event(&event).await;
                return;
            }
            "Basic" => {
                if self.handle_r4_basic_topic_event(&event).await {
                    return;
                }
            }
            _ => {}
        }

        // Evaluate which subscriptions match this event.
        let matches = self.evaluator.evaluate(&event);
        if matches.is_empty() {
            return;
        }

        debug!(
            resource_type = %event.resource_type,
            resource_id = %event.resource_id,
            matched_subscriptions = matches.len(),
            "Event matched subscriptions"
        );

        // Build and dispatch notifications for each match.
        for eval_match in matches {
            let mut subscription = eval_match.subscription;

            // Increment event counter.
            let event_number = self
                .manager
                .increment_event_count(&subscription.tenant_id, &subscription.id)
                .unwrap_or(0);
            // Ensure notification metadata reflects the event being emitted.
            subscription.events_since_start = event_number;

            let event_data = NotificationEventData {
                event_number,
                timestamp: event.timestamp,
                focus_reference: format!("{}/{}", event.resource_type, event.resource_id),
            };

            // Build notification bundle.
            let bundle = match notification::build_event_notification(
                &subscription,
                event_data,
                event.resource.as_ref(),
                &self.base_url,
            ) {
                Ok(b) => b,
                Err(e) => {
                    error!(
                        subscription_id = %subscription.id,
                        error = %e,
                        "Failed to build notification"
                    );
                    continue;
                }
            };

            // Dispatch with retry.
            self.dispatch_with_retry(&subscription, &bundle).await;
        }
    }

    /// Handle a Subscription resource event (create/update/delete).
    async fn handle_subscription_event(&self, event: &ResourceEvent) {
        let tenant_id = event.tenant_id.to_string();
        let subscription_id = &event.resource_id;

        match event.event_type {
            ResourceEventType::Delete => {
                self.manager.deregister(&tenant_id, subscription_id);
                self.ws_manager
                    .remove_all_clients(&tenant_id, subscription_id);
            }
            ResourceEventType::Create | ResourceEventType::Update => {
                if let Some(resource) = &event.resource {
                    // Register (or re-register) the subscription.
                    match self.manager.register(
                        &tenant_id,
                        subscription_id,
                        resource,
                        event.fhir_version,
                    ) {
                        Ok(sub) => {
                            // If status is requested, perform handshake and activate.
                            if sub.status == SubscriptionStatusCode::Requested {
                                self.activate_subscription(&sub).await;
                            }
                        }
                        Err(e) => {
                            warn!(
                                subscription_id,
                                error = %e,
                                "Failed to register subscription"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Handle a SubscriptionTopic resource event.
    async fn handle_topic_event(&self, event: &ResourceEvent) {
        let topic_key = (
            event.tenant_id.to_string(),
            event.resource_type.clone(),
            event.resource_id.clone(),
        );

        match event.event_type {
            ResourceEventType::Delete => {
                let mut candidate_urls = Vec::new();

                if let Some((_, indexed_url)) = self.topic_resource_index.remove(&topic_key) {
                    candidate_urls.push(indexed_url);
                }

                if let Some(resource) = &event.resource {
                    match InMemoryTopicRegistry::parse_topic_resource(resource) {
                        Ok(topic) => candidate_urls.push(topic.canonical_url),
                        Err(e) => {
                            warn!(
                                resource_id = %event.resource_id,
                                error = %e,
                                "Failed to parse SubscriptionTopic delete payload"
                            );
                        }
                    }
                }

                if let Some(previous_resource) = &event.previous_resource {
                    match InMemoryTopicRegistry::parse_topic_resource(previous_resource) {
                        Ok(topic) => candidate_urls.push(topic.canonical_url),
                        Err(e) => {
                            warn!(
                                resource_id = %event.resource_id,
                                error = %e,
                                "Failed to parse previous SubscriptionTopic state"
                            );
                        }
                    }
                }

                candidate_urls.sort();
                candidate_urls.dedup();

                if candidate_urls.is_empty() {
                    warn!(
                        resource_id = %event.resource_id,
                        "SubscriptionTopic deleted but canonical URL could not be resolved"
                    );
                    return;
                }

                for canonical_url in candidate_urls {
                    let removed = self.topic_registry.remove_topic(&canonical_url);
                    info!(
                        resource_id = %event.resource_id,
                        topic_url = %canonical_url,
                        removed,
                        "SubscriptionTopic deleted"
                    );
                }
            }
            ResourceEventType::Create | ResourceEventType::Update => {
                if let Some(resource) = &event.resource {
                    match InMemoryTopicRegistry::parse_topic_resource(resource) {
                        Ok(topic) => {
                            let canonical_url = topic.canonical_url.clone();
                            if let Some(previous_url) = self
                                .topic_resource_index
                                .insert(topic_key, canonical_url.clone())
                                .filter(|previous_url| previous_url != &canonical_url)
                            {
                                let _ = self.topic_registry.remove_topic(&previous_url);
                            }
                            info!(
                                topic_url = %canonical_url,
                                "Registered SubscriptionTopic"
                            );
                            self.topic_registry.add_topic(topic);
                        }
                        Err(e) => {
                            warn!(
                                resource_id = %event.resource_id,
                                error = %e,
                                "Failed to parse SubscriptionTopic"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Handle an R4 backport `Basic` topic event.
    ///
    /// Returns `true` when the `Basic` resource was recognized as a topic lifecycle
    /// event (including malformed topic candidates), `false` otherwise.
    async fn handle_r4_basic_topic_event(&self, event: &ResourceEvent) -> bool {
        if event.fhir_version.as_str() != "R4" {
            return false;
        }

        let topic_key = (
            event.tenant_id.to_string(),
            event.resource_type.clone(),
            event.resource_id.clone(),
        );

        match event.event_type {
            ResourceEventType::Delete => {
                let mut candidate_urls = Vec::new();
                let mut recognized_topic = false;

                if let Some((_, indexed_url)) = self.topic_resource_index.remove(&topic_key) {
                    candidate_urls.push(indexed_url);
                    recognized_topic = true;
                }

                if let Some(resource) = &event.resource {
                    match InMemoryTopicRegistry::parse_r4_backport_basic_topic_resource(resource) {
                        Ok(Some(topic)) => {
                            candidate_urls.push(topic.canonical_url);
                            recognized_topic = true;
                        }
                        Ok(None) => {}
                        Err(e) => {
                            warn!(
                                resource_id = %event.resource_id,
                                error = %e,
                                "Failed to parse R4 Basic SubscriptionTopic delete payload"
                            );
                            recognized_topic = true;
                        }
                    }
                }

                if let Some(previous_resource) = &event.previous_resource {
                    match InMemoryTopicRegistry::parse_r4_backport_basic_topic_resource(
                        previous_resource,
                    ) {
                        Ok(Some(topic)) => {
                            candidate_urls.push(topic.canonical_url);
                            recognized_topic = true;
                        }
                        Ok(None) => {}
                        Err(e) => {
                            warn!(
                                resource_id = %event.resource_id,
                                error = %e,
                                "Failed to parse previous R4 Basic SubscriptionTopic state"
                            );
                            recognized_topic = true;
                        }
                    }
                }

                candidate_urls.sort();
                candidate_urls.dedup();

                for canonical_url in candidate_urls {
                    let removed = self.topic_registry.remove_topic(&canonical_url);
                    info!(
                        resource_id = %event.resource_id,
                        topic_url = %canonical_url,
                        removed,
                        "R4 Basic SubscriptionTopic deleted"
                    );
                }

                recognized_topic
            }
            ResourceEventType::Create | ResourceEventType::Update => {
                if let Some(resource) = &event.resource {
                    match InMemoryTopicRegistry::parse_r4_backport_basic_topic_resource(resource) {
                        Ok(Some(topic)) => {
                            let canonical_url = topic.canonical_url.clone();
                            if let Some(previous_url) = self
                                .topic_resource_index
                                .insert(topic_key, canonical_url.clone())
                                .filter(|previous_url| previous_url != &canonical_url)
                            {
                                let _ = self.topic_registry.remove_topic(&previous_url);
                            }
                            info!(
                                topic_url = %canonical_url,
                                "Registered R4 Basic SubscriptionTopic"
                            );
                            self.topic_registry.add_topic(topic);
                            true
                        }
                        Ok(None) => false,
                        Err(e) => {
                            warn!(
                                resource_id = %event.resource_id,
                                error = %e,
                                "Failed to parse R4 Basic SubscriptionTopic"
                            );
                            true
                        }
                    }
                } else {
                    false
                }
            }
        }
    }

    /// Activate a subscription by performing the handshake.
    async fn activate_subscription(&self, subscription: &ActiveSubscription) {
        let tenant_id = &subscription.tenant_id;
        let sub_id = &subscription.id;

        // Build handshake notification.
        let handshake_bundle = match notification::build_handshake(subscription, &self.base_url) {
            Ok(b) => b,
            Err(e) => {
                warn!(subscription_id = sub_id, error = %e, "Failed to build handshake");
                let _ =
                    self.manager
                        .update_status(tenant_id, sub_id, SubscriptionStatusCode::Error);
                return;
            }
        };

        // Perform handshake.
        let result = match subscription.channel.channel_type {
            ChannelType::RestHook => {
                self.rest_hook_channel
                    .handshake(subscription, &handshake_bundle)
                    .await
            }
            ChannelType::Websocket => {
                self.ws_channel
                    .handshake(subscription, &handshake_bundle)
                    .await
            }
            ChannelType::Email => match self.email_channel.as_ref() {
                Some(ch) => ch.handshake(subscription, &handshake_bundle).await,
                None => {
                    warn!(
                        subscription_id = sub_id,
                        "Email channel requested but no SMTP settings configured"
                    );
                    let _ = self.manager.update_status(
                        tenant_id,
                        sub_id,
                        SubscriptionStatusCode::Error,
                    );
                    return;
                }
            },
            ChannelType::Message => match self.messaging_channel.as_ref() {
                Some(ch) => ch.handshake(subscription, &handshake_bundle).await,
                None => {
                    warn!(
                        subscription_id = sub_id,
                        "Messaging channel requested but messaging settings not configured"
                    );
                    let _ = self.manager.update_status(
                        tenant_id,
                        sub_id,
                        SubscriptionStatusCode::Error,
                    );
                    return;
                }
            },
            _ => {
                warn!(
                    subscription_id = sub_id,
                    channel_type = subscription.channel.channel_type.as_fhir_str(),
                    "Handshake not supported for channel type"
                );
                return;
            }
        };

        match result {
            Ok(DispatchResult::Success) => {
                info!(
                    subscription_id = sub_id,
                    "Handshake successful, activating subscription"
                );
                let _ =
                    self.manager
                        .update_status(tenant_id, sub_id, SubscriptionStatusCode::Active);
            }
            Ok(DispatchResult::RetryableError(msg)) | Ok(DispatchResult::PermanentError(msg)) => {
                warn!(subscription_id = sub_id, error = %msg, "Handshake failed");
                let _ =
                    self.manager
                        .update_status(tenant_id, sub_id, SubscriptionStatusCode::Error);
            }
            Err(e) => {
                warn!(subscription_id = sub_id, error = %e, "Handshake error");
                let _ =
                    self.manager
                        .update_status(tenant_id, sub_id, SubscriptionStatusCode::Error);
            }
        }
    }

    /// Dispatch a notification with retry logic.
    async fn dispatch_with_retry(
        &self,
        subscription: &ActiveSubscription,
        bundle: &serde_json::Value,
    ) {
        let tenant_id = &subscription.tenant_id;
        let sub_id = &subscription.id;

        let dispatcher: &dyn ChannelDispatcher = match subscription.channel.channel_type {
            ChannelType::RestHook => self.rest_hook_channel.as_ref(),
            ChannelType::Websocket => self.ws_channel.as_ref(),
            ChannelType::Email => match self.email_channel.as_deref() {
                Some(ch) => ch,
                None => {
                    warn!(
                        subscription_id = sub_id,
                        "Email dispatch requested but no SMTP settings configured"
                    );
                    return;
                }
            },
            ChannelType::Message => match self.messaging_channel.as_deref() {
                Some(ch) => ch,
                None => {
                    warn!(
                        subscription_id = sub_id,
                        "Messaging dispatch requested but messaging settings not configured"
                    );
                    return;
                }
            },
            _ => {
                warn!(
                    subscription_id = sub_id,
                    channel = subscription.channel.channel_type.as_fhir_str(),
                    "No dispatcher for channel type"
                );
                return;
            }
        };

        let mut attempt: u32 = 0;
        loop {
            match dispatcher.dispatch(subscription, bundle).await {
                Ok(DispatchResult::Success) => {
                    self.manager.reset_failures(tenant_id, sub_id);
                    return;
                }
                Ok(DispatchResult::PermanentError(msg)) => {
                    warn!(
                        subscription_id = sub_id,
                        error = %msg,
                        "Permanent delivery error"
                    );
                    self.handle_delivery_failure(tenant_id, sub_id);
                    return;
                }
                Ok(DispatchResult::RetryableError(msg)) => {
                    attempt += 1;
                    if !retry::should_retry(&self.config, attempt) {
                        warn!(
                            subscription_id = sub_id,
                            attempts = attempt,
                            error = %msg,
                            "Max retries exhausted"
                        );
                        self.handle_delivery_failure(tenant_id, sub_id);
                        return;
                    }

                    let delay = retry::calculate_delay(&self.config, attempt);
                    debug!(
                        subscription_id = sub_id,
                        attempt,
                        delay_ms = delay.as_millis() as u64,
                        "Retrying delivery"
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(e) => {
                    error!(
                        subscription_id = sub_id,
                        error = %e,
                        "Dispatch error"
                    );
                    self.handle_delivery_failure(tenant_id, sub_id);
                    return;
                }
            }
        }
    }

    /// Handle a delivery failure: increment failure count and potentially
    /// transition status to error or off.
    fn handle_delivery_failure(&self, tenant_id: &str, subscription_id: &str) {
        if let Some(failure_count) = self.manager.record_failure(tenant_id, subscription_id) {
            if failure_count >= self.config.off_threshold {
                warn!(
                    subscription_id,
                    failures = failure_count,
                    "Turning off subscription after repeated failures"
                );
                let _ = self.manager.update_status(
                    tenant_id,
                    subscription_id,
                    SubscriptionStatusCode::Off,
                );
            } else if failure_count >= self.config.error_threshold {
                let _ = self.manager.update_status(
                    tenant_id,
                    subscription_id,
                    SubscriptionStatusCode::Error,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ResourceEventType;
    use crate::topics::{ResourceTrigger, TopicDefinition};
    use chrono::Utc;
    use helios_fhir::FhirVersion;
    use helios_persistence::tenant::TenantId;
    use serde_json::json;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_engine(base_url: &str) -> SubscriptionEngine {
        let config = SubscriptionConfig {
            max_retries: 2,
            error_threshold: 2,
            off_threshold: 4,
            ..Default::default()
        };
        SubscriptionEngine::new(config, base_url.to_string())
    }

    fn encounter_topic() -> TopicDefinition {
        TopicDefinition {
            canonical_url: "http://example.org/topic/encounter-start".to_string(),
            title: Some("Encounter Start".to_string()),
            resource_triggers: vec![ResourceTrigger {
                resource_type: "Encounter".to_string(),
                interactions: vec![ResourceEventType::Create],
                fhirpath_criteria: None,
            }],
            can_filter_by: vec![],
            notification_shape: vec![],
        }
    }

    fn encounter_event() -> ResourceEvent {
        ResourceEvent {
            tenant_id: TenantId::new("t1"),
            fhir_version: FhirVersion::default(),
            resource_type: "Encounter".to_string(),
            resource_id: "enc-1".to_string(),
            version_id: "1".to_string(),
            event_type: ResourceEventType::Create,
            resource: Some(json!({
                "resourceType": "Encounter",
                "id": "enc-1",
                "status": "in-progress"
            })),
            previous_resource: None,
            timestamp: Utc::now(),
        }
    }

    fn topic_event() -> ResourceEvent {
        ResourceEvent {
            tenant_id: TenantId::new("t1"),
            fhir_version: FhirVersion::default(),
            resource_type: "SubscriptionTopic".to_string(),
            resource_id: "topic-1".to_string(),
            version_id: "1".to_string(),
            event_type: ResourceEventType::Create,
            resource: Some(json!({
                "resourceType": "SubscriptionTopic",
                "id": "topic-1",
                "url": "http://example.org/topic/encounter-start",
                "resourceTrigger": [{
                    "resource": "Encounter",
                    "supportedInteraction": ["create"]
                }]
            })),
            previous_resource: None,
            timestamp: Utc::now(),
        }
    }

    #[cfg(feature = "R4")]
    fn r4_basic_topic_event() -> ResourceEvent {
        ResourceEvent {
            tenant_id: TenantId::new("t1"),
            fhir_version: FhirVersion::R4,
            resource_type: "Basic".to_string(),
            resource_id: "topic-basic-1".to_string(),
            version_id: "1".to_string(),
            event_type: ResourceEventType::Create,
            resource: Some(json!({
                "resourceType": "Basic",
                "id": "topic-basic-1",
                "code": {
                    "coding": [{
                        "system": "http://hl7.org/fhir/fhir-types",
                        "code": "SubscriptionTopic"
                    }]
                },
                "extension": [{
                    "url": "http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.url",
                    "valueUri": "http://example.org/topic/encounter-start-basic"
                }, {
                    "url": "http://hl7.org/fhir/4.3/StructureDefinition/extension-SubscriptionTopic.resourceTrigger",
                    "extension": [
                        { "url": "resource", "valueUri": "http://hl7.org/fhir/StructureDefinition/Encounter" },
                        { "url": "supportedInteraction", "valueCode": "create" }
                    ]
                }]
            })),
            previous_resource: None,
            timestamp: Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_topic_event_registers_topic() {
        let engine = make_engine("http://localhost:8080");

        assert!(engine.topic_registry().list_topics().is_empty());

        engine.on_resource_event(topic_event()).await;

        let topics = engine.topic_registry().list_topics();
        assert_eq!(topics.len(), 1);
        assert!(topics.contains(&"http://example.org/topic/encounter-start".to_string()));
    }

    #[tokio::test]
    async fn test_topic_delete_event_removes_topic_without_payload() {
        let engine = make_engine("http://localhost:8080");
        engine.on_resource_event(topic_event()).await;

        let delete_event = ResourceEvent {
            tenant_id: TenantId::new("t1"),
            fhir_version: FhirVersion::default(),
            resource_type: "SubscriptionTopic".to_string(),
            resource_id: "topic-1".to_string(),
            version_id: "2".to_string(),
            event_type: ResourceEventType::Delete,
            resource: None,
            previous_resource: None,
            timestamp: Utc::now(),
        };

        engine.on_resource_event(delete_event).await;

        let topics = engine.topic_registry().list_topics();
        assert!(topics.is_empty());
    }

    #[cfg(feature = "R4")]
    #[tokio::test]
    async fn test_r4_basic_topic_event_registers_topic() {
        let engine = make_engine("http://localhost:8080");

        assert!(engine.topic_registry().list_topics().is_empty());

        engine.on_resource_event(r4_basic_topic_event()).await;

        let topics = engine.topic_registry().list_topics();
        assert_eq!(topics.len(), 1);
        assert!(topics.contains(&"http://example.org/topic/encounter-start-basic".to_string()));
    }

    #[tokio::test]
    async fn test_subscription_event_registers_subscription() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let engine = make_engine("http://localhost:8080");
        engine.topic_registry().add_topic(encounter_topic());

        // Create a subscription event.
        let sub_resource = crate::manager::tests::build_subscription_json(
            "http://example.org/topic/encounter-start",
            "rest-hook",
            Some(&format!("{}/webhook", server.uri())),
        );

        let event = ResourceEvent {
            tenant_id: TenantId::new("t1"),
            fhir_version: FhirVersion::default(),
            resource_type: "Subscription".to_string(),
            resource_id: "sub-1".to_string(),
            version_id: "1".to_string(),
            event_type: ResourceEventType::Create,
            resource: Some(sub_resource),
            previous_resource: None,
            timestamp: Utc::now(),
        };

        engine.on_resource_event(event).await;

        // Subscription should be registered.
        let sub = engine.manager().get_subscription("t1", "sub-1");
        assert!(sub.is_some());
    }

    #[tokio::test]
    async fn test_full_pipeline_event_to_dispatch() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(2) // handshake + event notification
            .mount(&server)
            .await;

        let engine = make_engine("http://localhost:8080");
        engine.topic_registry().add_topic(encounter_topic());

        // Register and activate a subscription.
        let sub_resource = crate::manager::tests::build_subscription_json(
            "http://example.org/topic/encounter-start",
            "rest-hook",
            Some(&format!("{}/webhook", server.uri())),
        );

        let sub_event = ResourceEvent {
            tenant_id: TenantId::new("t1"),
            fhir_version: FhirVersion::default(),
            resource_type: "Subscription".to_string(),
            resource_id: "sub-1".to_string(),
            version_id: "1".to_string(),
            event_type: ResourceEventType::Create,
            resource: Some(sub_resource),
            previous_resource: None,
            timestamp: Utc::now(),
        };

        engine.on_resource_event(sub_event).await;

        // Fire a matching event.
        engine.on_resource_event(encounter_event()).await;

        // Verify the mock received the requests.
        // (wiremock's expect(2) will panic on drop if not satisfied)
    }

    #[tokio::test]
    async fn test_no_dispatch_when_no_matching_subscriptions() {
        let engine = make_engine("http://localhost:8080");
        engine.topic_registry().add_topic(encounter_topic());

        // Fire an event with no subscriptions registered.
        engine.on_resource_event(encounter_event()).await;
        // Should complete without error.
    }

    #[tokio::test]
    async fn test_no_dispatch_for_non_matching_resource_type() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1) // Only the handshake
            .mount(&server)
            .await;

        let engine = make_engine("http://localhost:8080");
        engine.topic_registry().add_topic(encounter_topic());

        // Register subscription for Encounter topic.
        let sub_resource = crate::manager::tests::build_subscription_json(
            "http://example.org/topic/encounter-start",
            "rest-hook",
            Some(&format!("{}/webhook", server.uri())),
        );

        let sub_event = ResourceEvent {
            tenant_id: TenantId::new("t1"),
            fhir_version: FhirVersion::default(),
            resource_type: "Subscription".to_string(),
            resource_id: "sub-1".to_string(),
            version_id: "1".to_string(),
            event_type: ResourceEventType::Create,
            resource: Some(sub_resource),
            previous_resource: None,
            timestamp: Utc::now(),
        };

        engine.on_resource_event(sub_event).await;

        // Fire a Patient event (doesn't match Encounter topic).
        let patient_event = ResourceEvent {
            tenant_id: TenantId::new("t1"),
            fhir_version: FhirVersion::default(),
            resource_type: "Patient".to_string(),
            resource_id: "pat-1".to_string(),
            version_id: "1".to_string(),
            event_type: ResourceEventType::Create,
            resource: Some(json!({"resourceType": "Patient", "id": "pat-1"})),
            previous_resource: None,
            timestamp: Utc::now(),
        };

        engine.on_resource_event(patient_event).await;
        // Only 1 mock call (handshake), not 2.
    }

    #[tokio::test]
    async fn test_delivery_failure_updates_status() {
        let server = MockServer::start().await;

        // First request (handshake) succeeds, subsequent fail.
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let config = SubscriptionConfig {
            max_retries: 1,
            error_threshold: 1,
            off_threshold: 3,
            ..Default::default()
        };
        let engine = SubscriptionEngine::new(config, "http://localhost:8080".to_string());
        engine.topic_registry().add_topic(encounter_topic());

        // Register subscription.
        let sub_resource = crate::manager::tests::build_subscription_json(
            "http://example.org/topic/encounter-start",
            "rest-hook",
            Some(&format!("{}/webhook", server.uri())),
        );

        let sub_event = ResourceEvent {
            tenant_id: TenantId::new("t1"),
            fhir_version: FhirVersion::default(),
            resource_type: "Subscription".to_string(),
            resource_id: "sub-1".to_string(),
            version_id: "1".to_string(),
            event_type: ResourceEventType::Create,
            resource: Some(sub_resource),
            previous_resource: None,
            timestamp: Utc::now(),
        };

        engine.on_resource_event(sub_event).await;

        // Fire a matching event (will fail delivery).
        engine.on_resource_event(encounter_event()).await;

        // Check subscription status changed to error.
        let sub = engine.manager().get_subscription("t1", "sub-1").unwrap();
        assert_eq!(sub.status, SubscriptionStatusCode::Error);
    }

    #[tokio::test]
    async fn test_subscription_delete_event() {
        let engine = make_engine("http://localhost:8080");
        engine.topic_registry().add_topic(encounter_topic());

        // Register directly.
        let resource = crate::manager::tests::default_subscription_json();
        engine
            .manager()
            .register("t1", "sub-1", &resource, FhirVersion::default())
            .unwrap();

        assert!(engine.manager().get_subscription("t1", "sub-1").is_some());

        // Delete event.
        let delete_event = ResourceEvent {
            tenant_id: TenantId::new("t1"),
            fhir_version: FhirVersion::default(),
            resource_type: "Subscription".to_string(),
            resource_id: "sub-1".to_string(),
            version_id: "2".to_string(),
            event_type: ResourceEventType::Delete,
            resource: None,
            previous_resource: None,
            timestamp: Utc::now(),
        };

        engine.on_resource_event(delete_event).await;

        assert!(engine.manager().get_subscription("t1", "sub-1").is_none());
    }

    #[tokio::test]
    async fn test_tenant_isolation() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(2) // 1 handshake + 1 event notification (only tenant-a)
            .mount(&server)
            .await;

        let engine = make_engine("http://localhost:8080");
        engine.topic_registry().add_topic(encounter_topic());

        // Register subscription for tenant-a.
        let sub_resource = crate::manager::tests::build_subscription_json(
            "http://example.org/topic/encounter-start",
            "rest-hook",
            Some(&format!("{}/webhook", server.uri())),
        );

        let sub_event_a = ResourceEvent {
            tenant_id: TenantId::new("tenant-a"),
            fhir_version: FhirVersion::default(),
            resource_type: "Subscription".to_string(),
            resource_id: "sub-a".to_string(),
            version_id: "1".to_string(),
            event_type: ResourceEventType::Create,
            resource: Some(sub_resource),
            previous_resource: None,
            timestamp: Utc::now(),
        };

        engine.on_resource_event(sub_event_a).await;

        // Fire event from tenant-a (should match).
        let mut event_a = encounter_event();
        event_a.tenant_id = TenantId::new("tenant-a");
        engine.on_resource_event(event_a).await;

        // Fire event from tenant-b (should NOT match).
        let mut event_b = encounter_event();
        event_b.tenant_id = TenantId::new("tenant-b");
        engine.on_resource_event(event_b).await;

        // wiremock expects exactly 2 calls (handshake + 1 notification).
    }
}
