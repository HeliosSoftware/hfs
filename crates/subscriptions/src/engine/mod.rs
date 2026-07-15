//! Subscription engine — the orchestrator.
//!
//! Ties together topic registry, subscription manager, event evaluator,
//! notification builder, and channel dispatchers. Invoked asynchronously
//! after each resource write.

pub mod retry;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use helios_fhir::FhirVersion;
use helios_persistence::core::subscription_state::{
    SubscriptionHydrationSource, SubscriptionStateStore,
};
use helios_persistence::core::ws_binding_tokens::WsBindingTokenStore;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
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

/// The cluster-shared state handles the engine consumes when running as one
/// of N instances (cluster design §Class B). Obtained from the primary
/// storage backend's seams (`ResourceStorage::subscription_state_store()`
/// etc.) by the server layer; without them the engine keeps today's
/// process-local behavior byte-for-byte.
pub struct ClusterHandles {
    /// Shared counters, status, and stored notification bundles (B4/B1).
    pub state: Arc<dyn SubscriptionStateStore>,
    /// Shared single-use WebSocket binding tokens (B2).
    pub tokens: Arc<dyn WsBindingTokenStore>,
    /// Boot-time hydration read path (B3).
    pub hydration: Arc<dyn SubscriptionHydrationSource>,
    /// This instance's identity, used as the fan-out origin marker.
    pub instance_id: String,
}

/// Builds the full-access context the engine uses for its own store calls;
/// tenancy is enforced by passing the subscription's owning tenant.
fn tenant_ctx(tenant_id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(tenant_id), TenantPermissions::full_access())
}

/// Where a local registration came from — a live resource write on this
/// instance, or boot-time hydration replaying persisted resources.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RegistrationSource {
    /// The write path: the fresh resource's own status is the client's
    /// intent (e.g. re-requesting an `off` subscription), so it wins over
    /// stored runtime status — which is re-seeded from it.
    LiveWrite,
    /// Hydration: the stored runtime state (status/counters) wins over the
    /// resource's status field — an `off` flip recorded by any instance
    /// must survive a restart.
    Hydration,
}

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
    cluster: Option<ClusterHandles>,
}

fn calculate_handshake_retry_delay(
    config: &SubscriptionConfig,
    attempt: u32,
) -> std::time::Duration {
    let exponent = attempt.saturating_sub(1) as i32;
    let delay_secs = config.handshake_retry_initial_delay.as_secs_f64()
        * config.retry_backoff_factor.powi(exponent);
    let capped = delay_secs.min(config.handshake_retry_max_delay.as_secs_f64());

    std::time::Duration::from_secs_f64(capped)
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
            cluster: None,
        }
    }

    /// Attaches the cluster-shared state handles (builder style). Without
    /// this call the engine is process-local, exactly as before Phase 3.
    pub fn with_cluster_handles(mut self, cluster: ClusterHandles) -> Self {
        self.cluster = Some(cluster);
        self
    }

    /// Whether this engine runs against cluster-shared state.
    pub fn is_cluster_backed(&self) -> bool {
        self.cluster.is_some()
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
        info!(
            tenant_id = %event.tenant_id,
            resource_type = %event.resource_type,
            resource_id = %event.resource_id,
            event_type = %event.event_type,
            matched_subscriptions = matches.len(),
            "Subscription event evaluated"
        );
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

            // Increment event counter — through the shared store when
            // clustered so `eventNumber` is monotonic and gap-free across
            // every instance.
            let event_number = self
                .next_event_number(&subscription.tenant_id, &subscription.id)
                .await;
            // Ensure notification metadata reflects the event being emitted.
            subscription.events_since_start = event_number;

            let focus_reference = format!("{}/{}", event.resource_type, event.resource_id);
            let event_data = NotificationEventData {
                event_number,
                timestamp: event.timestamp,
                focus_reference: focus_reference.clone(),
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

            info!(
                tenant_id = %subscription.tenant_id,
                subscription_id = %subscription.id,
                topic_url = %subscription.topic_url,
                channel_type = %subscription.channel.channel_type.as_fhir_str(),
                endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                event_number,
                focus_reference = %focus_reference,
                "Dispatching subscription event notification"
            );

            // Dispatch with retry.
            self.dispatch_with_retry(
                &subscription,
                &bundle,
                "event-notification",
                Some(event_number),
            )
            .await;
        }
    }

    /// Handle a Subscription resource event (create/update/delete).
    async fn handle_subscription_event(&self, event: &ResourceEvent) {
        let tenant_id = event.tenant_id.to_string();
        let subscription_id = &event.resource_id;

        info!(
            tenant_id,
            subscription_id,
            event_type = %event.event_type,
            fhir_version = %event.fhir_version,
            "Handling Subscription resource event"
        );

        match event.event_type {
            ResourceEventType::Delete => {
                let removed = self.manager.deregister(&tenant_id, subscription_id);
                self.ws_manager
                    .remove_all_clients(&tenant_id, subscription_id);
                if let Some(cluster) = &self.cluster
                    && let Err(e) = cluster
                        .state
                        .delete(&tenant_ctx(&tenant_id), subscription_id)
                        .await
                {
                    warn!(
                        tenant_id,
                        subscription_id,
                        error = %e,
                        "Failed to delete shared subscription state"
                    );
                }
                info!(
                    tenant_id,
                    subscription_id, removed, "Subscription deregistered"
                );
            }
            ResourceEventType::Create | ResourceEventType::Update => {
                if let Some(resource) = &event.resource {
                    // Register (or re-register) the subscription.
                    if let Some(sub) = self
                        .register_subscription_locally(
                            &tenant_id,
                            subscription_id,
                            resource,
                            event.fhir_version,
                            RegistrationSource::LiveWrite,
                        )
                        .await
                    {
                        // If status is requested, perform handshake and activate.
                        if sub.status == SubscriptionStatusCode::Requested {
                            info!(
                                tenant_id = %sub.tenant_id,
                                subscription_id = %sub.id,
                                channel_type = %sub.channel.channel_type.as_fhir_str(),
                                endpoint = sub.channel.endpoint.as_deref().unwrap_or(""),
                                "Subscription activation requested"
                            );
                            self.activate_subscription(&sub).await;
                        }
                    }
                }
            }
        }
    }

    /// Registers a subscription into the local projection WITHOUT the
    /// activation handshake (the caller decides — a live write on this
    /// instance handshakes; hydration must not, or N restarting instances
    /// would re-handshake every requested subscription).
    ///
    /// In cluster mode the shared state row is reconciled per
    /// [`RegistrationSource`]: hydration overlays stored runtime state onto
    /// the projection; a live write keeps the shared `event_number`
    /// (resource updates never reset it) but re-seeds status from the fresh
    /// resource and zeroes the failure streak — the client's re-request
    /// intent wins.
    async fn register_subscription_locally(
        &self,
        tenant_id: &str,
        subscription_id: &str,
        resource: &serde_json::Value,
        fhir_version: FhirVersion,
        source: RegistrationSource,
    ) -> Option<ActiveSubscription> {
        let mut sub =
            match self
                .manager
                .register(tenant_id, subscription_id, resource, fhir_version)
            {
                Ok(sub) => sub,
                Err(e) => {
                    warn!(
                        tenant_id,
                        subscription_id,
                        error = %e,
                        "Failed to register subscription"
                    );
                    return None;
                }
            };
        info!(
            tenant_id = %sub.tenant_id,
            subscription_id = %sub.id,
            topic_url = %sub.topic_url,
            channel_type = %sub.channel.channel_type.as_fhir_str(),
            endpoint = sub.channel.endpoint.as_deref().unwrap_or(""),
            status = %sub.status,
            fhir_version = %sub.fhir_version,
            "Subscription registered"
        );

        if let Some(cluster) = &self.cluster {
            let ctx = tenant_ctx(tenant_id);
            match cluster.state.get(&ctx, subscription_id).await {
                Ok(Some(record)) => match source {
                    RegistrationSource::Hydration => {
                        let status = record
                            .status
                            .as_deref()
                            .and_then(SubscriptionStatusCode::from_fhir_str);
                        self.manager.apply_state_overlay(
                            tenant_id,
                            subscription_id,
                            record.event_number,
                            record.consecutive_failures,
                            status,
                        );
                        sub.events_since_start = record.event_number;
                        sub.consecutive_failures = record.consecutive_failures;
                        if let Some(status) = status {
                            sub.status = status;
                        }
                    }
                    RegistrationSource::LiveWrite => {
                        self.manager.sync_event_count(
                            tenant_id,
                            subscription_id,
                            record.event_number,
                        );
                        sub.events_since_start = record.event_number;
                        if let Err(e) = cluster
                            .state
                            .set_status(&ctx, subscription_id, sub.status.as_fhir_str())
                            .await
                        {
                            warn!(tenant_id, subscription_id, error = %e,
                                "Failed to re-seed shared subscription status");
                        }
                        if let Err(e) = cluster.state.reset_failures(&ctx, subscription_id).await {
                            warn!(tenant_id, subscription_id, error = %e,
                                "Failed to reset shared failure streak");
                        }
                    }
                },
                Ok(None) => {}
                Err(e) => {
                    warn!(
                        tenant_id,
                        subscription_id,
                        error = %e,
                        "Failed to read shared subscription state; using local projection"
                    );
                }
            }
        }

        Some(sub)
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
                    let removed = self
                        .topic_registry
                        .remove_topic(event.tenant_id.as_ref(), &canonical_url);
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
                    self.register_native_topic_locally(
                        event.tenant_id.as_ref(),
                        &event.resource_type,
                        &event.resource_id,
                        resource,
                    );
                }
            }
        }
    }

    /// Registers (or re-registers) a native `SubscriptionTopic` into the
    /// local projection. Shared by the live write path and hydration.
    fn register_native_topic_locally(
        &self,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &serde_json::Value,
    ) {
        let topic_key = (
            tenant_id.to_string(),
            resource_type.to_string(),
            resource_id.to_string(),
        );
        match InMemoryTopicRegistry::parse_topic_resource(resource) {
            Ok(topic) => {
                let canonical_url = topic.canonical_url.clone();
                if let Some(previous_url) = self
                    .topic_resource_index
                    .insert(topic_key, canonical_url.clone())
                    .filter(|previous_url| previous_url != &canonical_url)
                {
                    let _ = self.topic_registry.remove_topic(tenant_id, &previous_url);
                }
                info!(
                    tenant_id,
                    topic_url = %canonical_url,
                    "Registered SubscriptionTopic"
                );
                self.topic_registry.add_topic(tenant_id, topic);
            }
            Err(e) => {
                warn!(
                    resource_id,
                    error = %e,
                    "Failed to parse SubscriptionTopic"
                );
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
                    let removed = self
                        .topic_registry
                        .remove_topic(event.tenant_id.as_ref(), &canonical_url);
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
                    self.register_basic_topic_locally(
                        event.tenant_id.as_ref(),
                        &event.resource_type,
                        &event.resource_id,
                        resource,
                    )
                } else {
                    false
                }
            }
        }
    }

    /// Registers (or re-registers) an R4 backport `Basic` topic into the
    /// local projection. Shared by the live write path and hydration.
    /// Returns `true` when the `Basic` resource was recognized as a topic
    /// (including malformed topic candidates).
    fn register_basic_topic_locally(
        &self,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &serde_json::Value,
    ) -> bool {
        let topic_key = (
            tenant_id.to_string(),
            resource_type.to_string(),
            resource_id.to_string(),
        );
        match InMemoryTopicRegistry::parse_r4_backport_basic_topic_resource(resource) {
            Ok(Some(topic)) => {
                let canonical_url = topic.canonical_url.clone();
                if let Some(previous_url) = self
                    .topic_resource_index
                    .insert(topic_key, canonical_url.clone())
                    .filter(|previous_url| previous_url != &canonical_url)
                {
                    let _ = self.topic_registry.remove_topic(tenant_id, &previous_url);
                }
                info!(
                    tenant_id,
                    topic_url = %canonical_url,
                    "Registered R4 Basic SubscriptionTopic"
                );
                self.topic_registry.add_topic(tenant_id, topic);
                true
            }
            Ok(None) => false,
            Err(e) => {
                warn!(
                    resource_id,
                    error = %e,
                    "Failed to parse R4 Basic SubscriptionTopic"
                );
                true
            }
        }
    }

    /// Activate a subscription by performing the handshake.
    async fn activate_subscription(&self, subscription: &ActiveSubscription) {
        let tenant_id = &subscription.tenant_id;
        let sub_id = &subscription.id;
        let handshake_max_attempts = self.config.handshake_max_attempts.max(1);

        // Build handshake notification.
        let handshake_bundle = match notification::build_handshake(subscription, &self.base_url) {
            Ok(b) => b,
            Err(e) => {
                warn!(
                    tenant_id,
                    subscription_id = sub_id,
                    channel_type = %subscription.channel.channel_type.as_fhir_str(),
                    endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                    error = %e,
                    "Failed to build handshake"
                );
                self.transition_status(tenant_id, sub_id, SubscriptionStatusCode::Error)
                    .await;
                return;
            }
        };

        if !self.config.handshake_initial_delay.is_zero() {
            info!(
                tenant_id,
                subscription_id = sub_id,
                channel_type = %subscription.channel.channel_type.as_fhir_str(),
                endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                delay_ms = self.config.handshake_initial_delay.as_millis() as u64,
                "Delaying subscription handshake"
            );
            tokio::time::sleep(self.config.handshake_initial_delay).await;
        }

        let mut attempt = 1;
        loop {
            info!(
                tenant_id,
                subscription_id = sub_id,
                channel_type = %subscription.channel.channel_type.as_fhir_str(),
                endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                attempt,
                max_attempts = handshake_max_attempts,
                "Sending subscription handshake"
            );

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
                            tenant_id,
                            subscription_id = sub_id,
                            endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                            "Email channel requested but no SMTP settings configured"
                        );
                        self.transition_status(tenant_id, sub_id, SubscriptionStatusCode::Error)
                            .await;
                        return;
                    }
                },
                ChannelType::Message => match self.messaging_channel.as_ref() {
                    Some(ch) => ch.handshake(subscription, &handshake_bundle).await,
                    None => {
                        warn!(
                            tenant_id,
                            subscription_id = sub_id,
                            endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                            "Messaging channel requested but messaging settings not configured"
                        );
                        self.transition_status(tenant_id, sub_id, SubscriptionStatusCode::Error)
                            .await;
                        return;
                    }
                },
                _ => {
                    warn!(
                        tenant_id,
                        subscription_id = sub_id,
                        channel_type = subscription.channel.channel_type.as_fhir_str(),
                        endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                        "Handshake not supported for channel type"
                    );
                    return;
                }
            };

            match result {
                Ok(DispatchResult::Success) => {
                    info!(
                        tenant_id,
                        subscription_id = sub_id,
                        channel_type = %subscription.channel.channel_type.as_fhir_str(),
                        endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                        attempt,
                        "Handshake successful, activating subscription"
                    );
                    self.transition_status(tenant_id, sub_id, SubscriptionStatusCode::Active)
                        .await;
                    return;
                }
                Ok(DispatchResult::PermanentError(msg)) => {
                    warn!(
                        tenant_id,
                        subscription_id = sub_id,
                        channel_type = %subscription.channel.channel_type.as_fhir_str(),
                        endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                        attempt,
                        error = %msg,
                        "Handshake failed with permanent error"
                    );
                    self.transition_status(tenant_id, sub_id, SubscriptionStatusCode::Error)
                        .await;
                    return;
                }
                Ok(DispatchResult::RetryableError(msg)) => {
                    if attempt >= handshake_max_attempts {
                        warn!(
                            tenant_id,
                            subscription_id = sub_id,
                            channel_type = %subscription.channel.channel_type.as_fhir_str(),
                            endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                            attempts = attempt,
                            error = %msg,
                            "Handshake retries exhausted"
                        );
                        self.transition_status(tenant_id, sub_id, SubscriptionStatusCode::Error)
                            .await;
                        return;
                    }

                    let delay = calculate_handshake_retry_delay(&self.config, attempt);
                    warn!(
                        tenant_id,
                        subscription_id = sub_id,
                        channel_type = %subscription.channel.channel_type.as_fhir_str(),
                        endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                        attempt,
                        next_attempt = attempt + 1,
                        delay_ms = delay.as_millis() as u64,
                        error = %msg,
                        "Retrying subscription handshake"
                    );
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
                Err(e) => {
                    warn!(
                        tenant_id,
                        subscription_id = sub_id,
                        channel_type = %subscription.channel.channel_type.as_fhir_str(),
                        endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                        attempt,
                        error = %e,
                        "Handshake error"
                    );
                    self.transition_status(tenant_id, sub_id, SubscriptionStatusCode::Error)
                        .await;
                    return;
                }
            }
        }
    }

    /// Dispatch a notification with retry logic.
    async fn dispatch_with_retry(
        &self,
        subscription: &ActiveSubscription,
        bundle: &serde_json::Value,
        notification_type: &'static str,
        event_number: Option<u64>,
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
                        tenant_id,
                        subscription_id = sub_id,
                        notification_type,
                        endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                        "Email dispatch requested but no SMTP settings configured"
                    );
                    return;
                }
            },
            ChannelType::Message => match self.messaging_channel.as_deref() {
                Some(ch) => ch,
                None => {
                    warn!(
                        tenant_id,
                        subscription_id = sub_id,
                        notification_type,
                        endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                        "Messaging dispatch requested but messaging settings not configured"
                    );
                    return;
                }
            },
            _ => {
                warn!(
                    tenant_id,
                    subscription_id = sub_id,
                    notification_type,
                    channel = subscription.channel.channel_type.as_fhir_str(),
                    endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                    "No dispatcher for channel type"
                );
                return;
            }
        };

        let mut attempt: u32 = 0;
        loop {
            match dispatcher.dispatch(subscription, bundle).await {
                Ok(DispatchResult::Success) => {
                    self.reset_delivery_failures(tenant_id, sub_id).await;
                    info!(
                        tenant_id,
                        subscription_id = sub_id,
                        notification_type,
                        event_number,
                        channel_type = %subscription.channel.channel_type.as_fhir_str(),
                        endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                        "Subscription notification dispatched"
                    );
                    return;
                }
                Ok(DispatchResult::PermanentError(msg)) => {
                    warn!(
                        tenant_id,
                        subscription_id = sub_id,
                        notification_type,
                        event_number,
                        channel_type = %subscription.channel.channel_type.as_fhir_str(),
                        endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                        error = %msg,
                        "Permanent delivery error"
                    );
                    self.handle_delivery_failure(tenant_id, sub_id).await;
                    return;
                }
                Ok(DispatchResult::RetryableError(msg)) => {
                    attempt += 1;
                    if !retry::should_retry(&self.config, attempt) {
                        warn!(
                            tenant_id,
                            subscription_id = sub_id,
                            notification_type,
                            event_number,
                            channel_type = %subscription.channel.channel_type.as_fhir_str(),
                            endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                            attempts = attempt,
                            error = %msg,
                            "Max retries exhausted"
                        );
                        self.handle_delivery_failure(tenant_id, sub_id).await;
                        return;
                    }

                    let delay = retry::calculate_delay(&self.config, attempt);
                    debug!(
                        tenant_id,
                        subscription_id = sub_id,
                        notification_type,
                        event_number,
                        attempt,
                        delay_ms = delay.as_millis() as u64,
                        "Retrying delivery"
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(e) => {
                    error!(
                        tenant_id,
                        subscription_id = sub_id,
                        notification_type,
                        event_number,
                        channel_type = %subscription.channel.channel_type.as_fhir_str(),
                        endpoint = subscription.channel.endpoint.as_deref().unwrap_or(""),
                        error = %e,
                        "Dispatch error"
                    );
                    self.handle_delivery_failure(tenant_id, sub_id).await;
                    return;
                }
            }
        }
    }

    /// Handle a delivery failure: increment the failure count (shared when
    /// clustered, so failures scattered across instances still reach the
    /// thresholds) and potentially transition status to error or off.
    async fn handle_delivery_failure(&self, tenant_id: &str, subscription_id: &str) {
        let failure_count = match &self.cluster {
            Some(cluster) => {
                match cluster
                    .state
                    .record_failure(&tenant_ctx(tenant_id), subscription_id)
                    .await
                {
                    Ok(count) => Some(count),
                    Err(e) => {
                        warn!(
                            tenant_id,
                            subscription_id,
                            error = %e,
                            "Failed to record shared delivery failure; counting locally"
                        );
                        self.manager.record_failure(tenant_id, subscription_id)
                    }
                }
            }
            None => self.manager.record_failure(tenant_id, subscription_id),
        };
        if let Some(failure_count) = failure_count {
            if failure_count >= self.config.off_threshold {
                warn!(
                    subscription_id,
                    failures = failure_count,
                    "Turning off subscription after repeated failures"
                );
                self.transition_status(tenant_id, subscription_id, SubscriptionStatusCode::Off)
                    .await;
            } else if failure_count >= self.config.error_threshold {
                self.transition_status(tenant_id, subscription_id, SubscriptionStatusCode::Error)
                    .await;
            }
        }
    }

    /// Resets the consecutive-failure streak after a successful delivery,
    /// locally and (when clustered) in the shared store.
    async fn reset_delivery_failures(&self, tenant_id: &str, subscription_id: &str) {
        self.manager.reset_failures(tenant_id, subscription_id);
        if let Some(cluster) = &self.cluster
            && let Err(e) = cluster
                .state
                .reset_failures(&tenant_ctx(tenant_id), subscription_id)
                .await
        {
            warn!(
                tenant_id,
                subscription_id,
                error = %e,
                "Failed to reset shared failure streak"
            );
        }
    }

    /// Records a runtime status transition locally and (when clustered) in
    /// the shared store, so every instance's snapshot observes it. Local
    /// transition validation keeps today's semantics; the shared write is
    /// skipped when the local transition is refused.
    async fn transition_status(
        &self,
        tenant_id: &str,
        subscription_id: &str,
        status: SubscriptionStatusCode,
    ) {
        if self
            .manager
            .update_status(tenant_id, subscription_id, status)
            .is_err()
        {
            return;
        }
        if let Some(cluster) = &self.cluster
            && let Err(e) = cluster
                .state
                .set_status(
                    &tenant_ctx(tenant_id),
                    subscription_id,
                    status.as_fhir_str(),
                )
                .await
        {
            warn!(
                tenant_id,
                subscription_id,
                status = %status,
                error = %e,
                "Failed to record shared status transition"
            );
        }
    }

    /// Mints the next event number — cluster-wide monotonic through the
    /// shared store when clustered, else the local counter. A store failure
    /// falls back to the local counter (delivering with a possibly-duplicate
    /// number beats dropping the notification; subscribers gap-detect).
    async fn next_event_number(&self, tenant_id: &str, subscription_id: &str) -> u64 {
        if let Some(cluster) = &self.cluster {
            match cluster
                .state
                .next_event_number(&tenant_ctx(tenant_id), subscription_id)
                .await
            {
                Ok(event_number) => {
                    self.manager
                        .sync_event_count(tenant_id, subscription_id, event_number);
                    return event_number;
                }
                Err(e) => {
                    error!(
                        tenant_id,
                        subscription_id,
                        error = %e,
                        "Failed to increment shared event number; falling back to local"
                    );
                }
            }
        }
        self.manager
            .increment_event_count(tenant_id, subscription_id)
            .unwrap_or(0)
    }

    /// Rebuilds the in-memory projections from persisted resources (B3
    /// startup hydration): topics first (native `SubscriptionTopic` plus R4
    /// backport `Basic`), then `Subscription` resources — each overlaid with
    /// its shared runtime state (status/counters win over the resource's
    /// status field). Never fires activation handshakes. Idempotent, also
    /// used as the full-resync response after a fan-out reconnect.
    pub async fn hydrate(&self) {
        let Some(cluster) = &self.cluster else {
            return;
        };
        let resources = match cluster
            .hydration
            .list_current(&["SubscriptionTopic", "Basic", "Subscription"])
            .await
        {
            Ok(resources) => resources,
            Err(e) => {
                error!(error = %e, "Subscription hydration read failed");
                return;
            }
        };

        let mut topics = 0usize;
        let mut subscriptions = 0usize;
        for resource in resources
            .iter()
            .filter(|r| r.resource_type != "Subscription")
        {
            let Some(fhir_version) = FhirVersion::from_mime_param(&resource.fhir_version) else {
                warn!(
                    tenant_id = %resource.tenant_id,
                    resource_type = %resource.resource_type,
                    resource_id = %resource.resource_id,
                    fhir_version = %resource.fhir_version,
                    "Skipping hydration row with unknown FHIR version"
                );
                continue;
            };
            match resource.resource_type.as_str() {
                "SubscriptionTopic" => {
                    self.register_native_topic_locally(
                        &resource.tenant_id,
                        &resource.resource_type,
                        &resource.resource_id,
                        &resource.content,
                    );
                    topics += 1;
                }
                "Basic" if fhir_version.as_str() == "R4" => {
                    if self.register_basic_topic_locally(
                        &resource.tenant_id,
                        &resource.resource_type,
                        &resource.resource_id,
                        &resource.content,
                    ) {
                        topics += 1;
                    }
                }
                _ => {}
            }
        }

        for resource in resources
            .iter()
            .filter(|r| r.resource_type == "Subscription")
        {
            let Some(fhir_version) = FhirVersion::from_mime_param(&resource.fhir_version) else {
                warn!(
                    tenant_id = %resource.tenant_id,
                    resource_id = %resource.resource_id,
                    fhir_version = %resource.fhir_version,
                    "Skipping hydration row with unknown FHIR version"
                );
                continue;
            };
            if self
                .register_subscription_locally(
                    &resource.tenant_id,
                    &resource.resource_id,
                    &resource.content,
                    fhir_version,
                    RegistrationSource::Hydration,
                )
                .await
                .is_some()
            {
                subscriptions += 1;
            }
        }

        info!(
            topics,
            subscriptions, "Subscription state hydrated from storage"
        );
    }

    /// Mints a single-use WebSocket binding token — in the shared store
    /// when clustered (redeemable on any instance), else in the local map.
    pub async fn generate_ws_token(
        &self,
        tenant_id: &str,
        subscription_id: &str,
    ) -> Result<(String, DateTime<Utc>), crate::error::SubscriptionError> {
        match &self.cluster {
            Some(cluster) => {
                cluster
                    .tokens
                    .mint(
                        &tenant_ctx(tenant_id),
                        subscription_id,
                        std::time::Duration::from_secs(
                            self.config.ws_token_lifetime_secs.max(0) as u64
                        ),
                    )
                    .await
                    .map_err(|e| {
                        crate::error::SubscriptionError::Internal(format!(
                            "failed to mint binding token: {e}"
                        ))
                    })
            }
            None => Ok(self
                .ws_token_manager
                .generate_token(tenant_id, subscription_id)),
        }
    }

    /// Redeems a single-use WebSocket binding token, returning
    /// `(tenant_id, subscription_id)` exactly once. Fails closed: a shared
    /// store error reads as an invalid token.
    pub async fn redeem_ws_token(&self, token: &str) -> Option<(String, String)> {
        match &self.cluster {
            Some(cluster) => match cluster.tokens.redeem(token).await {
                Ok(redeemed) => redeemed,
                Err(e) => {
                    warn!(
                        error = %e,
                        "Binding token redeem failed against the shared store"
                    );
                    None
                }
            },
            None => self.ws_token_manager.validate_and_consume(token),
        }
    }

    /// A read of one subscription with cluster-shared runtime state
    /// overlaid (eventNumber, failure streak, status) when clustered —
    /// what `$status`/`$events` should serve. Local mode: identical to
    /// `manager().get_subscription`.
    pub async fn subscription_snapshot(
        &self,
        tenant_id: &str,
        subscription_id: &str,
    ) -> Option<ActiveSubscription> {
        let mut sub = self.manager.get_subscription(tenant_id, subscription_id)?;
        if let Some(cluster) = &self.cluster {
            match cluster
                .state
                .get(&tenant_ctx(tenant_id), subscription_id)
                .await
            {
                Ok(Some(record)) => {
                    sub.events_since_start = record.event_number;
                    sub.consecutive_failures = record.consecutive_failures;
                    if let Some(status) = record
                        .status
                        .as_deref()
                        .and_then(SubscriptionStatusCode::from_fhir_str)
                    {
                        sub.status = status;
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    warn!(
                        tenant_id,
                        subscription_id,
                        error = %e,
                        "Failed to read shared subscription state; serving local snapshot"
                    );
                }
            }
        }
        Some(sub)
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
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

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

    #[derive(Clone)]
    struct FailFirstHandshake {
        attempts: Arc<AtomicUsize>,
    }

    impl Respond for FailFirstHandshake {
        fn respond(&self, _request: &Request) -> ResponseTemplate {
            if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(500)
            } else {
                ResponseTemplate::new(200)
            }
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

        assert!(engine.topic_registry().list_topics("t1").is_empty());

        engine.on_resource_event(topic_event()).await;

        let topics = engine.topic_registry().list_topics("t1");
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

        let topics = engine.topic_registry().list_topics("t1");
        assert!(topics.is_empty());
    }

    #[cfg(feature = "R4")]
    #[tokio::test]
    async fn test_r4_basic_topic_event_registers_topic() {
        let engine = make_engine("http://localhost:8080");

        assert!(engine.topic_registry().list_topics("t1").is_empty());

        engine.on_resource_event(r4_basic_topic_event()).await;

        let topics = engine.topic_registry().list_topics("t1");
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
        engine.topic_registry().add_topic("t1", encounter_topic());

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
    async fn test_subscription_activation_retries_retryable_handshake_failure() {
        let server = MockServer::start().await;
        let attempts = Arc::new(AtomicUsize::new(0));

        Mock::given(method("POST"))
            .respond_with(FailFirstHandshake {
                attempts: Arc::clone(&attempts),
            })
            .mount(&server)
            .await;

        let config = SubscriptionConfig {
            handshake_max_attempts: 2,
            handshake_retry_initial_delay: std::time::Duration::from_millis(1),
            handshake_retry_max_delay: std::time::Duration::from_millis(1),
            ..Default::default()
        };
        let engine = SubscriptionEngine::new(config, "http://localhost:8080".to_string());
        engine.topic_registry().add_topic("t1", encounter_topic());

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

        let sub = engine.manager().get_subscription("t1", "sub-1").unwrap();
        assert_eq!(sub.status, SubscriptionStatusCode::Active);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
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
        engine.topic_registry().add_topic("t1", encounter_topic());

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
        engine.topic_registry().add_topic("t1", encounter_topic());

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
        engine.topic_registry().add_topic("t1", encounter_topic());

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
        engine.topic_registry().add_topic("t1", encounter_topic());

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
        engine.topic_registry().add_topic("t1", encounter_topic());

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
        // Topics are tenant-scoped: the topic exists for tenant-a only.
        engine
            .topic_registry()
            .add_topic("tenant-a", encounter_topic());

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
