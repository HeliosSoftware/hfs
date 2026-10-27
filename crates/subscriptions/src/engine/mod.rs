//! Subscription engine — the orchestrator.
//!
//! Ties together topic registry, subscription manager, event evaluator,
//! notification builder, and channel dispatchers. Invoked asynchronously
//! after resource writes, with synchronous lifecycle changes and bounded DATA
//! ingress feeding independent subscription queues.

mod dispatch;
pub mod retry;
pub use dispatch::DispatchSnapshot;
use dispatch::{CompletionTicket, SchedulerOwner, SchedulerState};

use std::sync::{Arc, RwLock};

use dashmap::DashMap;
use helios_persistence::core::ResourceStorage;
use helios_persistence::error::StorageError;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use tokio::sync::Semaphore;
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
    ChannelType, SubscriptionManager, SubscriptionSnapshot, SubscriptionStatusCode,
};
use crate::notification::{self, NotificationEventData};
use crate::topics::{InMemoryTopicRegistry, TopicDefinition};
use helios_auth::{NoOpOutboundAuthProvider, OutboundAuthProvider};

/// The subscription engine orchestrates the entire subscription pipeline.
///
/// Committed writes use bounded, nonblocking ingress. Independent subscription
/// queues retain notifications through retry and preserve admitted event order.
pub struct SubscriptionEngine {
    core: Arc<EngineCore>,
    scheduler: SchedulerOwner,
}

struct EngineCore {
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
    public_base_url_for_tenant: Arc<dyn Fn(&str) -> String + Send + Sync>,
    /// Storage handle used to write a server-driven status transition back into
    /// the stored `Subscription` resource. `None` leaves every transition
    /// in-memory only, which is the pre-#357 behaviour.
    status_store: RwLock<Option<Arc<dyn ResourceStorage>>>,
    /// Bounds how many status write-backs may be in flight at once, so a
    /// transition storm (every subscription tripping at the same instant during
    /// a subscriber outage) cannot drain the primary backend's connection pool
    /// and take reads down with it.
    status_write_slots: Arc<Semaphore>,
    /// Rolling per-subscription delivery counters (#586) — the operator page's
    /// last-24-hours figures. In-memory, like the rest of the engine's state.
    delivery_stats: Arc<crate::delivery_stats::DeliveryStats>,
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
        let resolver: Arc<dyn Fn(&str) -> String + Send + Sync> =
            Arc::new(move |_| base_url.clone());
        Self::with_outbound_auth_and_url_resolver(config, resolver, false, outbound_auth)
    }

    /// Creates an engine whose background notifications resolve a canonical
    /// public base for each subscription tenant.
    pub fn with_outbound_auth_and_url_resolver(
        config: SubscriptionConfig,
        public_base_url_for_tenant: Arc<dyn Fn(&str) -> String + Send + Sync>,
        resolve_default_message_source: bool,
        outbound_auth: Arc<dyn OutboundAuthProvider>,
    ) -> Self {
        let config = config.normalize_dispatch_limits();
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
            let mut channel =
                MessagingChannel::new(settings.source_endpoint.clone(), Arc::clone(&outbound_auth));
            if resolve_default_message_source {
                channel =
                    channel.with_source_endpoint_resolver(Arc::clone(&public_base_url_for_tenant));
            }
            Arc::new(channel.with_private_endpoints_allowed(settings.allow_private_endpoints))
        });

        let status_write_slots = Arc::new(Semaphore::new(config.status_write_concurrency.max(1)));

        let scheduler = SchedulerOwner::new(&config);
        let core = Arc::new(EngineCore {
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
            public_base_url_for_tenant,
            status_store: RwLock::new(None),
            status_write_slots,
            delivery_stats: Arc::new(crate::delivery_stats::DeliveryStats::new()),
        });
        Self { core, scheduler }
    }

    /// Attaches the storage handle that server-driven status transitions are
    /// written back to (issue #357).
    ///
    /// Without this the engine's decisions — `requested` → `active` after a
    /// successful handshake, `→ error`, `→ off` once the delivery circuit
    /// breaker trips — live only in the in-memory manager. They are then lost on
    /// restart, and `GET /Subscription/{id}` contradicts
    /// `GET /Subscription/{id}/$status` for the lifetime of the resource.
    ///
    /// This deliberately takes the plain [`ResourceStorage`] every backend
    /// already implements rather than a new per-backend store, so the fix
    /// applies uniformly to SQLite, PostgreSQL, MongoDB, S3, and the composite
    /// deployments with no schema change and no migration.
    ///
    /// On a composite deployment pass the **composite**, not the primary, so the
    /// rewritten resource also reaches the search index — otherwise
    /// `GET /Subscription?status=active` keeps answering from stale documents.
    pub fn with_status_store(self, storage: Arc<dyn ResourceStorage>) -> Self {
        *self
            .core
            .status_store
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(storage);
        self
    }

    /// Whether server-driven status transitions are written back to storage.
    pub fn persists_status(&self) -> bool {
        self.core
            .status_store
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
            && self.core.config.persist_status
    }

    /// Returns a reference to the topic registry.
    pub fn topic_registry(&self) -> &Arc<InMemoryTopicRegistry> {
        &self.core.topic_registry
    }

    /// Registers a parsed topic; see [`EngineCore::register_topic`].
    pub(crate) fn register_topic(
        &self,
        tenant_id: &str,
        resource_type: &str,
        resource_id: Option<&str>,
        topic: TopicDefinition,
    ) {
        self.core
            .register_topic(tenant_id, resource_type, resource_id, topic);
    }

    /// Returns a reference to the subscription manager.
    pub fn manager(&self) -> &Arc<SubscriptionManager> {
        &self.core.manager
    }

    /// Returns a reference to the WebSocket manager.
    pub fn ws_manager(&self) -> &Arc<WebSocketManager> {
        &self.core.ws_manager
    }

    /// Returns a reference to the WebSocket binding token manager.
    /// The rolling delivery counters (#586) — the operator page reads its
    /// last-24-hours figures from here.
    pub fn delivery_stats(&self) -> &Arc<crate::delivery_stats::DeliveryStats> {
        &self.core.delivery_stats
    }

    pub fn ws_token_manager(&self) -> &Arc<WsBindingTokenManager> {
        &self.core.ws_token_manager
    }

    /// Called after a resource write has been committed.
    ///
    /// This method:
    /// 1. Handles subscription/topic lifecycle events (if the written resource
    ///    is a Subscription, SubscriptionTopic, or an R4 backport Basic topic).
    /// 2. Evaluates the event against all active subscriptions.
    /// 3. Builds and dispatches notifications.
    pub async fn on_resource_event(&self, event: ResourceEvent) {
        if let Some(activation) = self.handle_control(EventView::from(&event)) {
            drop(event);
            if let Some(snapshot) = activation {
                self.activate_subscription(&snapshot).await;
            }
            return;
        }
        if !self.scheduler.start(&self.core) {
            return;
        }
        if let Some(receipt) = self.scheduler.enqueue(|| event, true) {
            let _ = receipt.await;
        }
    }

    /// Coherent occupancy and shedding counters for the effective dispatch limits.
    pub fn dispatch_snapshot(&self) -> DispatchSnapshot {
        self.scheduler.state.snapshot()
    }

    pub(crate) fn observe_write(&self, write: &helios_persistence::core::ResourceWrite) {
        let Some(notice) = write.notice.as_ref().filter(|n| !n.resource_id.is_empty()) else {
            return;
        };
        if tokio::runtime::Handle::try_current().is_err() {
            debug!(resource_type = %write.resource_type, resource_id = %notice.resource_id, "No Tokio runtime; subscription event not dispatched");
            return;
        }
        let view = EventView {
            tenant_id: &write.tenant,
            fhir_version: write.fhir_version,
            resource_type: &write.resource_type,
            resource_id: &notice.resource_id,
            version_id: &notice.version_id,
            event_type: match notice.kind {
                helios_persistence::core::WriteKind::Create => ResourceEventType::Create,
                helios_persistence::core::WriteKind::Update => ResourceEventType::Update,
                helios_persistence::core::WriteKind::Delete => ResourceEventType::Delete,
            },
            resource: notice.resource.as_ref(),
            previous_resource: notice.previous.as_ref(),
        };
        if let Some(activation) = self.handle_control(view) {
            if let Some(snapshot) = activation {
                if self.scheduler.start(&self.core) {
                    self.scheduler.state.request_activation(snapshot);
                }
            }
            return;
        }
        if self.scheduler.start(&self.core) {
            self.scheduler.enqueue(
                || ResourceEvent {
                    tenant_id: write.tenant.clone(),
                    fhir_version: write.fhir_version,
                    resource_type: write.resource_type.clone(),
                    resource_id: notice.resource_id.clone(),
                    version_id: notice.version_id.clone(),
                    event_type: view.event_type,
                    resource: notice.resource.clone(),
                    previous_resource: notice.previous.clone(),
                    timestamp: write.at,
                },
                false,
            );
        }
    }

    fn handle_control(&self, event: EventView<'_>) -> Option<Option<SubscriptionSnapshot>> {
        match event.resource_type {
            "Subscription" => {
                let snapshot = self.core.handle_subscription_event(&event);
                if snapshot.is_some() || event.event_type == ResourceEventType::Delete {
                    self.scheduler
                        .state
                        .invalidate_registration(event.tenant_id.as_str(), event.resource_id);
                }
                Some(
                    snapshot.filter(|s| s.subscription.status == SubscriptionStatusCode::Requested),
                )
            }
            "SubscriptionTopic" => {
                self.core.handle_topic_event(&event);
                Some(None)
            }
            "Basic" if self.core.handle_r4_basic_topic_event(&event) => Some(None),
            _ => None,
        }
    }

    pub(crate) async fn activate_subscription(&self, snapshot: &SubscriptionSnapshot) {
        if !self.core.manager.is_current(snapshot) || snapshot.generation.is_cancelled() {
            return;
        }
        if self.scheduler.start(&self.core) {
            let mut receipt = self.scheduler.state.request_activation(snapshot.clone());
            while !*receipt.borrow_and_update() {
                if receipt.changed().await.is_err() {
                    break;
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct EventView<'a> {
    tenant_id: &'a TenantId,
    fhir_version: helios_fhir::FhirVersion,
    resource_type: &'a str,
    resource_id: &'a str,
    version_id: &'a str,
    event_type: ResourceEventType,
    resource: Option<&'a serde_json::Value>,
    previous_resource: Option<&'a serde_json::Value>,
}
impl<'a> From<&'a ResourceEvent> for EventView<'a> {
    fn from(event: &'a ResourceEvent) -> Self {
        Self {
            tenant_id: &event.tenant_id,
            fhir_version: event.fhir_version,
            resource_type: &event.resource_type,
            resource_id: &event.resource_id,
            version_id: &event.version_id,
            event_type: event.event_type,
            resource: event.resource.as_ref(),
            previous_resource: event.previous_resource.as_ref(),
        }
    }
}

impl EngineCore {
    fn evaluate_event(
        self: &Arc<Self>,
        event: &ResourceEvent,
        scheduler: &Arc<SchedulerState>,
        completion: &Option<CompletionTicket>,
    ) {
        let matches = self.evaluator.evaluate_snapshots(event);
        info!(tenant_id = %event.tenant_id, resource_type = %event.resource_type, resource_id = %event.resource_id,
            matched_subscriptions = matches.len(), "Subscription event evaluated");
        for matched in matches {
            let mut snapshot = matched.snapshot;
            let Some(event_number) = self.manager.increment_event_count_if_current(&snapshot)
            else {
                continue;
            };
            snapshot.subscription.events_since_start = event_number;
            let Some(resident) = scheduler.reserve(&snapshot) else {
                continue;
            };
            let event_data = NotificationEventData {
                event_number,
                timestamp: event.timestamp,
                focus_reference: format!("{}/{}", event.resource_type, event.resource_id),
            };
            let public_base_url =
                (self.public_base_url_for_tenant)(&snapshot.subscription.tenant_id);
            let bundle = match notification::build_event_notification(
                &snapshot.subscription,
                event_data,
                event.resource.as_ref(),
                &public_base_url,
            ) {
                Ok(bundle) => bundle,
                Err(error) => {
                    error!(subscription_id = %snapshot.subscription.id, %error, "Failed to build notification");
                    continue;
                }
            };
            scheduler.submit(
                self,
                snapshot,
                bundle,
                event_number,
                resident,
                completion.as_ref().cloned(),
            );
        }
    }

    /// Handle a Subscription resource event (create/update/delete).
    fn handle_subscription_event(&self, event: &EventView<'_>) -> Option<SubscriptionSnapshot> {
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
                info!(
                    tenant_id,
                    subscription_id, removed, "Subscription deregistered"
                );
            }
            ResourceEventType::Create | ResourceEventType::Update => {
                if let Some(resource) = &event.resource {
                    // Register (or re-register) the subscription.
                    match self.manager.register_snapshot(
                        &tenant_id,
                        subscription_id,
                        resource,
                        event.fhir_version,
                        Some(event.version_id),
                    ) {
                        Ok(snapshot) => {
                            let sub = &snapshot.subscription;
                            info!(
                                tenant_id = %sub.tenant_id,
                                subscription_id = %sub.id,
                                topic_url = %sub.topic_url,
                                channel_type = %sub.channel.channel_type.as_fhir_str(),
                                endpoint = %sub.channel.endpoint_for_log(),
                                status = %sub.status,
                                fhir_version = %sub.fhir_version,
                                "Subscription registered"
                            );
                            return Some(snapshot);
                        }
                        Err(e) => {
                            warn!(
                                tenant_id,
                                subscription_id,
                                error = %e,
                                "Failed to register subscription"
                            );
                        }
                    }
                }
            }
        }
        None
    }

    /// Registers a parsed topic and its resource-to-canonical association.
    ///
    /// Shared by live writes and startup rehydration so rename and payload-free
    /// deletion have the same tenant-scoped index after a restart. Exported
    /// resources without a usable string ID still register their definition.
    pub(crate) fn register_topic(
        &self,
        tenant_id: &str,
        resource_type: &str,
        resource_id: Option<&str>,
        topic: TopicDefinition,
    ) {
        let canonical_url = topic.canonical_url.clone();
        if let Some(resource_id) = resource_id.filter(|id| !id.is_empty()) {
            let key = (
                tenant_id.to_string(),
                resource_type.to_string(),
                resource_id.to_string(),
            );
            if let Some(previous_url) = self
                .topic_resource_index
                .insert(key, canonical_url.clone())
                .filter(|previous_url| previous_url != &canonical_url)
            {
                self.topic_registry.remove_topic(tenant_id, &previous_url);
            }
        } else {
            warn!(tenant_id, resource_type, topic_url = %canonical_url,
                "Registering topic without a resource ID; lifecycle index unavailable");
        }
        self.topic_registry.add_topic(tenant_id, topic);
    }

    /// Handle a SubscriptionTopic resource event.
    fn handle_topic_event(&self, event: &EventView<'_>) {
        let topic_key = (
            event.tenant_id.to_string(),
            event.resource_type.to_string(),
            event.resource_id.to_string(),
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
                    match InMemoryTopicRegistry::parse_topic_resource(resource) {
                        Ok(topic) => {
                            let canonical_url = topic.canonical_url.clone();
                            info!(
                                topic_url = %canonical_url,
                                "Registered SubscriptionTopic"
                            );
                            self.register_topic(
                                event.tenant_id.as_ref(),
                                event.resource_type,
                                Some(event.resource_id),
                                topic,
                            );
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
    fn handle_r4_basic_topic_event(&self, event: &EventView<'_>) -> bool {
        if event.fhir_version.as_str() != "R4" {
            return false;
        }

        let topic_key = (
            event.tenant_id.to_string(),
            event.resource_type.to_string(),
            event.resource_id.to_string(),
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
                    match InMemoryTopicRegistry::parse_r4_backport_basic_topic_resource(resource) {
                        Ok(Some(topic)) => {
                            let canonical_url = topic.canonical_url.clone();
                            info!(
                                topic_url = %canonical_url,
                                "Registered R4 Basic SubscriptionTopic"
                            );
                            self.register_topic(
                                event.tenant_id.as_ref(),
                                event.resource_type,
                                Some(event.resource_id),
                                topic,
                            );
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
    ///
    /// Invoked only after the shared scheduler has reserved an activation slot.
    /// Live writes and startup rehydration use the same path and attempt budget.
    async fn activate_subscription(
        &self,
        snapshot: &SubscriptionSnapshot,
        scheduler: &Arc<SchedulerState>,
    ) {
        let subscription = &snapshot.subscription;
        let tenant_id = &subscription.tenant_id;
        let sub_id = &subscription.id;
        let handshake_max_attempts = self.config.handshake_max_attempts.max(1);

        // Build handshake notification.
        let public_base_url = (self.public_base_url_for_tenant)(&subscription.tenant_id);
        let handshake_bundle = match notification::build_handshake(subscription, &public_base_url) {
            Ok(b) => b,
            Err(e) => {
                warn!(
                    tenant_id,
                    subscription_id = sub_id,
                    channel_type = %subscription.channel.channel_type.as_fhir_str(),
                    endpoint = %subscription.channel.endpoint_for_log(),
                    error = %e,
                    "Failed to build handshake"
                );
                self.transition_status(snapshot, SubscriptionStatusCode::Error, scheduler)
                    .await;
                return;
            }
        };

        if !self.config.handshake_initial_delay.is_zero() {
            info!(
                tenant_id,
                subscription_id = sub_id,
                channel_type = %subscription.channel.channel_type.as_fhir_str(),
                endpoint = %subscription.channel.endpoint_for_log(),
                delay_ms = self.config.handshake_initial_delay.as_millis() as u64,
                "Delaying subscription handshake"
            );
            if scheduler
                .run(
                    snapshot,
                    tokio::time::sleep(self.config.handshake_initial_delay),
                )
                .await
                .is_none()
            {
                return;
            }
        }

        let mut attempt = 1;
        loop {
            info!(
                tenant_id,
                subscription_id = sub_id,
                channel_type = %subscription.channel.channel_type.as_fhir_str(),
                endpoint = %subscription.channel.endpoint_for_log(),
                attempt,
                max_attempts = handshake_max_attempts,
                "Sending subscription handshake"
            );

            let Some(permit) = scheduler.attempt(snapshot).await else {
                return;
            };
            // Perform handshake; release the attempt slot before persistence/backoff.
            let handshake = async {
                match subscription.channel.channel_type {
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
                                endpoint = %subscription.channel.endpoint_for_log(),
                                "Email channel requested but no SMTP settings configured"
                            );
                            Ok(DispatchResult::PermanentError(
                                "Channel is not configured".to_string(),
                            ))
                        }
                    },
                    ChannelType::Message => match self.messaging_channel.as_ref() {
                        Some(ch) => ch.handshake(subscription, &handshake_bundle).await,
                        None => {
                            warn!(
                                tenant_id,
                                subscription_id = sub_id,
                                endpoint = %subscription.channel.endpoint_for_log(),
                                "Messaging channel requested but messaging settings not configured"
                            );
                            Ok(DispatchResult::PermanentError(
                                "Channel is not configured".to_string(),
                            ))
                        }
                    },
                    _ => {
                        warn!(
                            tenant_id,
                            subscription_id = sub_id,
                            channel_type = subscription.channel.channel_type.as_fhir_str(),
                            endpoint = %subscription.channel.endpoint_for_log(),
                            "Handshake not supported for channel type"
                        );
                        Ok(DispatchResult::PermanentError(
                            "Handshake channel unsupported".to_string(),
                        ))
                    }
                }
            };
            let result = scheduler.run(snapshot, handshake).await;
            drop(permit);
            let Some(result) = result else {
                return;
            };

            match result {
                Ok(DispatchResult::Success) => {
                    info!(
                        tenant_id,
                        subscription_id = sub_id,
                        channel_type = %subscription.channel.channel_type.as_fhir_str(),
                        endpoint = %subscription.channel.endpoint_for_log(),
                        attempt,
                        "Handshake successful, activating subscription"
                    );
                    self.transition_status(snapshot, SubscriptionStatusCode::Active, scheduler)
                        .await;
                    return;
                }
                Ok(DispatchResult::PermanentError(msg)) => {
                    warn!(
                        tenant_id,
                        subscription_id = sub_id,
                        channel_type = %subscription.channel.channel_type.as_fhir_str(),
                        endpoint = %subscription.channel.endpoint_for_log(),
                        attempt,
                        error = %msg,
                        "Handshake failed with permanent error"
                    );
                    self.transition_status(snapshot, SubscriptionStatusCode::Error, scheduler)
                        .await;
                    return;
                }
                Ok(DispatchResult::RetryableError(msg)) => {
                    if attempt >= handshake_max_attempts {
                        warn!(
                            tenant_id,
                            subscription_id = sub_id,
                            channel_type = %subscription.channel.channel_type.as_fhir_str(),
                            endpoint = %subscription.channel.endpoint_for_log(),
                            attempts = attempt,
                            error = %msg,
                            "Handshake retries exhausted"
                        );
                        self.transition_status(snapshot, SubscriptionStatusCode::Error, scheduler)
                            .await;
                        return;
                    }

                    let delay = calculate_handshake_retry_delay(&self.config, attempt);
                    warn!(
                        tenant_id,
                        subscription_id = sub_id,
                        channel_type = %subscription.channel.channel_type.as_fhir_str(),
                        endpoint = %subscription.channel.endpoint_for_log(),
                        attempt,
                        next_attempt = attempt + 1,
                        delay_ms = delay.as_millis() as u64,
                        error = %msg,
                        "Retrying subscription handshake"
                    );
                    if scheduler
                        .run(snapshot, tokio::time::sleep(delay))
                        .await
                        .is_none()
                    {
                        return;
                    }
                    attempt += 1;
                }
                Err(e) => {
                    warn!(
                        tenant_id,
                        subscription_id = sub_id,
                        channel_type = %subscription.channel.channel_type.as_fhir_str(),
                        endpoint = %subscription.channel.endpoint_for_log(),
                        attempt,
                        error = %e,
                        "Handshake error"
                    );
                    self.transition_status(snapshot, SubscriptionStatusCode::Error, scheduler)
                        .await;
                    return;
                }
            }
        }
    }

    /// Dispatch a notification with retry logic.
    async fn dispatch_with_retry(
        &self,
        snapshot: &SubscriptionSnapshot,
        bundle: &serde_json::Value,
        notification_type: &'static str,
        event_number: Option<u64>,
        scheduler: &Arc<SchedulerState>,
    ) {
        let subscription = &snapshot.subscription;
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
                        endpoint = %subscription.channel.endpoint_for_log(),
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
                        endpoint = %subscription.channel.endpoint_for_log(),
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
                    endpoint = %subscription.channel.endpoint_for_log(),
                    "No dispatcher for channel type"
                );
                return;
            }
        };

        let mut attempt: u32 = 0;
        loop {
            let now = chrono::Utc::now().timestamp();
            let Some(permit) = scheduler.attempt(snapshot).await else {
                return;
            };
            let result = scheduler
                .run(snapshot, dispatcher.dispatch(subscription, bundle))
                .await;
            drop(permit);
            let Some(result) = result else {
                return;
            };
            if !self.manager.is_current(snapshot) || snapshot.generation.is_cancelled() {
                return;
            }
            match result {
                Ok(DispatchResult::Success) => {
                    if self
                        .manager
                        .accept_delivery_outcome(snapshot, true, || {
                            self.delivery_stats.record_success(
                                tenant_id,
                                sub_id,
                                attempt == 0,
                                now,
                            );
                        })
                        .is_none()
                    {
                        return;
                    }
                    info!(
                        tenant_id,
                        subscription_id = sub_id,
                        notification_type,
                        event_number,
                        channel_type = %subscription.channel.channel_type.as_fhir_str(),
                        endpoint = %subscription.channel.endpoint_for_log(),
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
                        endpoint = %subscription.channel.endpoint_for_log(),
                        error = %msg,
                        "Permanent delivery error"
                    );
                    self.handle_delivery_failure(snapshot, scheduler, now).await;
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
                            endpoint = %subscription.channel.endpoint_for_log(),
                            attempts = attempt,
                            error = %msg,
                            "Max retries exhausted"
                        );
                        self.handle_delivery_failure(snapshot, scheduler, now).await;
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
                    if scheduler
                        .run(snapshot, tokio::time::sleep(delay))
                        .await
                        .is_none()
                    {
                        return;
                    }
                }
                Err(e) => {
                    error!(
                        tenant_id,
                        subscription_id = sub_id,
                        notification_type,
                        event_number,
                        channel_type = %subscription.channel.channel_type.as_fhir_str(),
                        endpoint = %subscription.channel.endpoint_for_log(),
                        error = %e,
                        "Dispatch error"
                    );
                    self.handle_delivery_failure(snapshot, scheduler, now).await;
                    return;
                }
            }
        }
    }

    /// The single choke point for every server-driven status transition.
    ///
    /// Applies the transition to the in-memory manager and, only when the
    /// manager accepted it as a genuine state change, writes the new status back
    /// into the stored `Subscription` resource.
    ///
    /// The manager accepts transitions only for the current generation and
    /// rejects repeated or terminal transitions. A FIFO delivery queue has one
    /// active notification per registration, so an outage cannot launch parallel
    /// retry cycles for that receiver. Successful transitions still pass through
    /// this single persistence funnel.
    ///
    /// Returns whether the in-memory transition was applied. Persistence failure
    /// does **not** change that answer: see [`Self::write_status_back`].
    async fn transition_status(
        &self,
        snapshot: &SubscriptionSnapshot,
        new_status: SubscriptionStatusCode,
        scheduler: &Arc<SchedulerState>,
    ) -> bool {
        if self
            .manager
            .update_status_if_current(snapshot, new_status)
            .is_none()
        {
            return false;
        }
        self.write_status_back(snapshot, new_status, scheduler)
            .await;
        true
    }

    /// Writes a server-driven status transition into the stored `Subscription`.
    ///
    /// Fails **open**, always. The in-memory transition has already been applied
    /// by [`Self::transition_status`] and is what this process acts on, so a
    /// storage error must never undo it: refusing to mark a subscription `off`
    /// because the database is briefly unavailable would put the server straight
    /// back to hammering the endpoint the circuit breaker just gave up on. The
    /// cost of failing open is that this one transition does not survive a
    /// restart — exactly the pre-#357 behaviour, and no worse.
    ///
    /// A concurrent client write wins. `ResourceStorage::update` takes the
    /// version that was read and every backend rejects a stale one, so a racing
    /// `PUT /Subscription/{id}` surfaces here as a version conflict, which is
    /// **dropped rather than retried**: the client's copy is the newer intent,
    /// and its write re-registers the subscription anyway.
    async fn write_status_back(
        &self,
        snapshot: &SubscriptionSnapshot,
        new_status: SubscriptionStatusCode,
        scheduler: &Arc<SchedulerState>,
    ) {
        if !self.config.persist_status {
            return;
        }
        let storage = self
            .status_store
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(storage) = storage else {
            return;
        };
        let lock = snapshot.generation.status_write_lock();
        let _write = tokio::select! { biased; _ = scheduler.closed() => return, _ = snapshot.generation.invalidated() => return, guard = lock.lock() => guard };
        let _permit = tokio::select! { biased; _ = scheduler.closed() => return, _ = snapshot.generation.invalidated() => return, permit = self.status_write_slots.acquire() => match permit { Ok(p) => p, Err(_) => return } };
        if !self.manager.is_current_status(snapshot, new_status) {
            return;
        }
        let Some(fence) = self.manager.source_fence(snapshot) else {
            return;
        };
        let sub = &snapshot.subscription;
        let tenant = TenantContext::new(
            TenantId::new(&sub.tenant_id),
            TenantPermissions::full_access(),
        );
        let result = tokio::select! {
            biased;
            _ = scheduler.closed() => return,
            _ = snapshot.generation.invalidated() => return,
            result = tokio::time::timeout(self.config.status_write_timeout, persist_status(storage.as_ref(), &tenant, snapshot, new_status, &self.manager, &fence)) => result,
        };
        match result {
            Ok(Ok(outcome)) => {
                debug!(tenant_id = %sub.tenant_id, subscription_id = %sub.id, status = %new_status, ?outcome, "Subscription status write-back completed")
            }
            Ok(Err(error)) => {
                error!(tenant_id = %sub.tenant_id, subscription_id = %sub.id, status = %new_status, %error, "Failed to persist subscription status transition; this transition will not survive a restart")
            }
            Err(_) => {
                error!(tenant_id = %sub.tenant_id, subscription_id = %sub.id, status = %new_status, "Timed out persisting subscription status transition; this transition will not survive a restart")
            }
        }
    }

    /// Handle a delivery failure: increment failure count and potentially
    /// transition status to error or off.
    async fn handle_delivery_failure(
        &self,
        snapshot: &SubscriptionSnapshot,
        scheduler: &Arc<SchedulerState>,
        now: i64,
    ) {
        let tenant_id = &snapshot.subscription.tenant_id;
        let subscription_id = &snapshot.subscription.id;
        let Some(failure_count) = self.manager.accept_delivery_outcome(snapshot, false, || {
            self.delivery_stats
                .record_failure(tenant_id, subscription_id, now);
        }) else {
            return;
        };
        if failure_count >= self.config.off_threshold {
            warn!(
                tenant_id,
                subscription_id,
                failures = failure_count,
                threshold = self.config.off_threshold,
                "Turning off subscription after repeated failures"
            );
            self.transition_status(snapshot, SubscriptionStatusCode::Off, scheduler)
                .await;
        } else if failure_count >= self.config.error_threshold {
            self.transition_status(snapshot, SubscriptionStatusCode::Error, scheduler)
                .await;
        }
    }
}

/// What [`persist_status`] did, so the caller can log it at the right level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PersistOutcome {
    /// The stored resource was rewritten with the new status.
    Written,
    /// The stored resource already carried this status — no write issued.
    ///
    /// This is the common case on the first boot after upgrading: rehydration
    /// re-activates every subscription that reads back `requested`, and without
    /// this check each success would rewrite a resource that write-back had
    /// already corrected on a previous run.
    AlreadyCurrent,
    /// The resource no longer exists (deleted between the transition and the
    /// write). Not an error.
    Vanished,
    /// A concurrent client write bumped the version first. Dropped, not retried.
    RacedClientWrite,
}

/// Reads the stored `Subscription`, replaces its `status`, and writes it back.
///
/// Only the `status` field is touched: the resource is read, mutated in place,
/// and handed back to `update`, so a field this engine knows nothing about
/// cannot be dropped on the way through. In particular the version-shape markers
/// (`topic`/`channelType` vs `criteria`/`channel`) survive untouched, which
/// matters because startup rehydration infers the FHIR version from exactly
/// those keys (`crate::rehydrate::infer_fhir_version`).
async fn persist_status(
    storage: &dyn ResourceStorage,
    tenant: &TenantContext,
    snapshot: &SubscriptionSnapshot,
    new_status: SubscriptionStatusCode,
    manager: &SubscriptionManager,
    fence: &crate::manager::StatusSourceFence,
) -> Result<PersistOutcome, StorageError> {
    let subscription_id = &snapshot.subscription.id;
    let current = match storage.read(tenant, "Subscription", subscription_id).await {
        Ok(Some(stored)) => stored,
        Ok(None) => return Ok(PersistOutcome::Vanished),
        // A deleted subscription is `Gone`, not an error worth shouting about.
        Err(StorageError::Resource(_)) => return Ok(PersistOutcome::Vanished),
        Err(e) => return Err(e),
    };

    if !manager.is_current_status(snapshot, new_status)
        || !fence.matches(current.version_id(), current.content())
    {
        return Ok(PersistOutcome::RacedClientWrite);
    }

    if current.content().get("status").and_then(|v| v.as_str()) == Some(new_status.as_fhir_str()) {
        return Ok(PersistOutcome::AlreadyCurrent);
    }

    let mut updated = current.content().clone();
    let Some(object) = updated.as_object_mut() else {
        // Not a JSON object — nothing sane to write back onto.
        return Ok(PersistOutcome::Vanished);
    };
    object.insert(
        "status".to_string(),
        serde_json::Value::String(new_status.as_fhir_str().to_string()),
    );

    if !manager.is_current_status(snapshot, new_status) {
        return Ok(PersistOutcome::RacedClientWrite);
    }
    match storage.update(tenant, &current, updated).await {
        Ok(stored) => {
            manager.acknowledge_status_write(
                snapshot,
                fence,
                stored.version_id(),
                stored.content(),
            );
            Ok(PersistOutcome::Written)
        }
        Err(StorageError::Concurrency(_)) => Ok(PersistOutcome::RacedClientWrite),
        Err(StorageError::Resource(_)) => Ok(PersistOutcome::Vanished),
        Err(e) => Err(e),
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

    #[test]
    fn canonical_public_base_resolves_for_background_tenant() {
        let resolver: Arc<dyn Fn(&str) -> String + Send + Sync> =
            Arc::new(|tenant| format!("https://public.example/fhir/{tenant}"));
        let engine = SubscriptionEngine::with_outbound_auth_and_url_resolver(
            SubscriptionConfig::default(),
            resolver,
            false,
            Arc::new(NoOpOutboundAuthProvider),
        );
        assert_eq!(
            (engine.core.public_base_url_for_tenant)("acme"),
            "https://public.example/fhir/acme"
        );

        let config = SubscriptionConfig {
            messaging: Some(crate::config::MessagingSettings {
                source_endpoint: "https://fallback.example/fhir".to_string(),
                allow_private_endpoints: false,
            }),
            ..Default::default()
        };
        let resolver: Arc<dyn Fn(&str) -> String + Send + Sync> =
            Arc::new(|tenant| format!("https://public.example/fhir/{tenant}"));
        let engine = SubscriptionEngine::with_outbound_auth_and_url_resolver(
            config,
            resolver,
            true,
            Arc::new(NoOpOutboundAuthProvider),
        );
        assert!(engine.core.messaging_channel.is_some());
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

    fn tenant_topic_event(
        version: FhirVersion,
        tenant: &str,
        url: &str,
        trigger: &str,
    ) -> ResourceEvent {
        let mut event = topic_event();
        #[cfg(feature = "R4")]
        if version == FhirVersion::R4 {
            event = r4_basic_topic_event();
        }
        event.tenant_id = TenantId::new(tenant);
        event.fhir_version = version;
        event.resource_id = "shared-id".to_string();
        let resource = event.resource.as_mut().unwrap();
        resource["id"] = json!(event.resource_id);
        if version.as_str() == "R4" {
            resource["extension"][0]["valueUri"] = json!(url);
            resource["extension"][1]["extension"][0]["valueUri"] =
                json!(format!("http://hl7.org/fhir/StructureDefinition/{trigger}"));
        } else {
            resource["url"] = json!(url);
            resource["resourceTrigger"][0]["resource"] = json!(trigger);
        }
        event
    }

    #[tokio::test]
    async fn shared_canonical_topic_lifecycle_is_tenant_scoped() {
        let url = "http://example.org/topic/shared";
        let renamed = "http://example.org/topic/renamed";
        for version in enabled_versions() {
            let engine = make_engine("http://example.org/fhir");
            engine
                .on_resource_event(tenant_topic_event(version, "tenant-a", url, "Encounter"))
                .await;
            engine
                .on_resource_event(tenant_topic_event(version, "tenant-b", url, "Observation"))
                .await;
            assert_eq!(
                engine
                    .topic_registry()
                    .get_topic("tenant-a", url)
                    .unwrap()
                    .resource_triggers[0]
                    .resource_type,
                "Encounter",
                "{version:?}"
            );
            assert_eq!(
                engine
                    .topic_registry()
                    .get_topic("tenant-b", url)
                    .unwrap()
                    .resource_triggers[0]
                    .resource_type,
                "Observation",
                "{version:?}"
            );

            let mut update = tenant_topic_event(version, "tenant-b", url, "Patient");
            update.event_type = ResourceEventType::Update;
            engine.on_resource_event(update).await;
            assert_eq!(
                engine
                    .topic_registry()
                    .get_topic("tenant-b", url)
                    .unwrap()
                    .resource_triggers[0]
                    .resource_type,
                "Patient"
            );

            let mut rename = tenant_topic_event(version, "tenant-b", renamed, "Patient");
            rename.event_type = ResourceEventType::Update;
            engine.on_resource_event(rename).await;
            assert!(engine.topic_registry().get_topic("tenant-b", url).is_none());
            assert!(
                engine
                    .topic_registry()
                    .get_topic("tenant-b", renamed)
                    .is_some()
            );
            let mut delete = tenant_topic_event(version, "tenant-b", renamed, "Patient");
            delete.event_type = ResourceEventType::Delete;
            delete.resource = None;
            engine.on_resource_event(delete).await;
            assert!(engine.topic_registry().list_topics("tenant-b").is_empty());
            assert_eq!(
                engine
                    .topic_registry()
                    .get_topic("tenant-a", url)
                    .unwrap()
                    .resource_triggers[0]
                    .resource_type,
                "Encounter"
            );

            // Resolve deletion from the current payload, previous payload, or
            // resource index; none may remove another tenant's shared URL.
            for payload in 0..3 {
                let create = tenant_topic_event(version, "tenant-b", url, "Observation");
                engine.on_resource_event(create.clone()).await;
                if payload < 2 {
                    engine.core.topic_resource_index.remove(&(
                        "tenant-b".to_string(),
                        create.resource_type.clone(),
                        create.resource_id.clone(),
                    ));
                }
                let mut delete = create;
                delete.event_type = ResourceEventType::Delete;
                match payload {
                    0 => {}
                    1 => {
                        delete.previous_resource = delete.resource.take();
                    }
                    _ => {
                        delete.resource = None;
                    }
                }
                engine.on_resource_event(delete).await;
                assert!(engine.topic_registry().get_topic("tenant-b", url).is_none());
                assert!(
                    engine.topic_registry().get_topic("tenant-a", url).is_some(),
                    "{version:?}, payload {payload}"
                );
            }
        }
    }

    #[test]
    fn topics_without_a_resource_id_keep_their_definition() {
        let engine = make_engine("http://example.org/fhir");
        for resource_id in [None, Some("")] {
            engine.register_topic(
                "tenant-a",
                "SubscriptionTopic",
                resource_id,
                encounter_topic(),
            );
            assert!(
                engine
                    .topic_registry()
                    .get_topic("tenant-a", "http://example.org/topic/encounter-start")
                    .is_some()
            );
            assert!(engine.core.topic_resource_index.is_empty());
        }
        assert!(engine.topic_registry().list_topics("tenant-b").is_empty());
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
