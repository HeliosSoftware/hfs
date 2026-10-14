//! Subscription manager.
//!
//! Manages the lifecycle of `Subscription` resources: CRUD, status tracking,
//! filter validation, and in-memory indexing of active subscriptions.

pub mod filters;
pub mod status;

use std::sync::Arc;

use dashmap::{DashMap, mapref::entry::Entry};
use helios_fhir::FhirVersion;
use tokio::sync::{Mutex, watch};
use tracing::{debug, warn};

use crate::error::SubscriptionError;
use crate::topics::InMemoryTopicRegistry;

pub use filters::SubscriptionFilter;
pub use status::SubscriptionStatusCode;

/// Payload content level requested by the subscriber.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadContent {
    /// No resource content in the notification.
    Empty,
    /// Resource references only (fullUrl + request, no resource body).
    IdOnly,
    /// Full resource content included.
    FullResource,
}

impl PayloadContent {
    /// Parse from a FHIR string value.
    pub fn from_fhir_str(s: &str) -> Option<Self> {
        match s {
            "empty" => Some(Self::Empty),
            "id-only" => Some(Self::IdOnly),
            "full-resource" => Some(Self::FullResource),
            _ => None,
        }
    }

    /// Returns the FHIR string representation.
    pub fn as_fhir_str(&self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::IdOnly => "id-only",
            Self::FullResource => "full-resource",
        }
    }
}

/// Channel type for a subscription.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChannelType {
    RestHook,
    Websocket,
    Email,
    Message,
    Custom(String),
}

impl ChannelType {
    /// Parse from a FHIR channel type string.
    pub fn from_fhir_str(s: &str) -> Self {
        match s {
            "rest-hook" => Self::RestHook,
            "websocket" => Self::Websocket,
            "email" => Self::Email,
            "message" => Self::Message,
            other => Self::Custom(other.to_string()),
        }
    }

    /// Returns the FHIR string representation.
    pub fn as_fhir_str(&self) -> &str {
        match self {
            Self::RestHook => "rest-hook",
            Self::Websocket => "websocket",
            Self::Email => "email",
            Self::Message => "message",
            Self::Custom(s) => s,
        }
    }
}

/// Channel configuration for a subscription.
#[derive(Debug, Clone)]
pub struct ChannelConfig {
    pub channel_type: ChannelType,
    pub endpoint: Option<String>,
    pub payload_mime_type: Option<String>,
    pub payload_content: PayloadContent,
    pub headers: Vec<String>,
    pub heartbeat_period: Option<u32>,
    pub timeout: Option<u32>,
    pub max_count: Option<u32>,
}

/// An active subscription tracked by the manager.
///
/// This is a version-agnostic in-memory representation of the essential fields
/// needed for event evaluation and notification delivery.
#[derive(Debug, Clone)]
pub struct ActiveSubscription {
    /// The subscription resource ID.
    pub id: String,

    /// The canonical URL of the SubscriptionTopic.
    pub topic_url: String,

    /// Current status.
    pub status: SubscriptionStatusCode,

    /// Channel configuration.
    pub channel: ChannelConfig,

    /// Parsed filter criteria.
    pub filters: Vec<SubscriptionFilter>,

    /// The FHIR version of the subscription resource.
    pub fhir_version: FhirVersion,

    /// Monotonically increasing event counter.
    pub events_since_start: u64,

    /// Consecutive delivery failure count (for error/off transitions).
    pub consecutive_failures: u32,

    /// Tenant ID that owns this subscription.
    pub tenant_id: String,
}

/// A registration snapshot for work that may finish after a client update.
/// The public subscription representation deliberately carries no runtime token.
#[derive(Debug, Clone)]
pub(crate) struct SubscriptionSnapshot {
    pub subscription: ActiveSubscription,
    pub generation: GenerationToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GenerationState {
    Live,
    Terminal,
    Invalidated,
}

#[derive(Debug)]
struct Generation {
    state: watch::Sender<GenerationState>,
    status_writes: Arc<Mutex<()>>,
}

/// Shared identity and cancellation state for exactly one registration.
/// Watch retains the state even when cancellation precedes a waiter's creation.
#[derive(Debug, Clone)]
pub(crate) struct GenerationToken {
    inner: Arc<Generation>,
}

impl GenerationToken {
    fn new(status: SubscriptionStatusCode) -> Self {
        let state = if status.is_terminal() {
            GenerationState::Terminal
        } else {
            GenerationState::Live
        };
        let (state, _) = watch::channel(state);
        Self {
            inner: Arc::new(Generation {
                state,
                status_writes: Arc::new(Mutex::new(())),
            }),
        }
    }

    pub(crate) fn same_generation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        *self.inner.state.borrow() != GenerationState::Live
    }

    /// Delivery and activation stop both on terminal status and replacement.
    pub(crate) async fn cancelled(&self) {
        self.wait_for(|state| state != GenerationState::Live).await;
    }

    /// Status write-back may persist a terminal decision, but must stop when
    /// the registration that made that decision was replaced or removed.
    pub(crate) async fn invalidated(&self) {
        self.wait_for(|state| state == GenerationState::Invalidated)
            .await;
    }

    pub(crate) fn status_write_lock(&self) -> Arc<Mutex<()>> {
        Arc::clone(&self.inner.status_writes)
    }

    async fn wait_for(&self, predicate: fn(GenerationState) -> bool) {
        let mut receiver = self.inner.state.subscribe();
        loop {
            let state = *receiver.borrow_and_update();
            if predicate(state) || receiver.changed().await.is_err() {
                return;
            }
        }
    }

    fn make_terminal(&self) {
        self.inner.state.send_if_modified(|state| {
            if *state == GenerationState::Live {
                *state = GenerationState::Terminal;
                true
            } else {
                false
            }
        });
    }

    fn invalidate(&self) {
        self.inner.state.send_replace(GenerationState::Invalidated);
    }
}

/// The exact stored intent a generation may update. Server metadata is
/// normalized away; all other fields, including unknown fields, remain fenced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StatusSourceFence {
    version_id: Option<String>,
    content: Arc<serde_json::Value>,
}

impl StatusSourceFence {
    fn new(resource: &serde_json::Value, source_version: Option<&str>) -> Self {
        let version_id = source_version
            .filter(|value| !value.is_empty())
            .or_else(|| {
                resource
                    .get("meta")
                    .and_then(|meta| meta.get("versionId"))
                    .and_then(|version| version.as_str())
                    .filter(|value| !value.is_empty())
            })
            .map(str::to_string);
        Self {
            version_id,
            content: Arc::new(normalize_source(resource)),
        }
    }

    pub(crate) fn matches(&self, version_id: &str, content: &serde_json::Value) -> bool {
        self.version_id
            .as_deref()
            .is_none_or(|expected| expected == version_id)
            && *self.content == normalize_source(content)
    }
}

fn normalize_source(resource: &serde_json::Value) -> serde_json::Value {
    let mut normalized = resource.clone();
    if let Some(object) = normalized.as_object_mut() {
        let remove_meta = object
            .get_mut("meta")
            .and_then(|meta| meta.as_object_mut())
            .is_some_and(|meta| {
                meta.remove("versionId");
                meta.remove("lastUpdated");
                meta.is_empty()
            });
        if remove_meta {
            object.remove("meta");
        }
    }
    normalized
}

struct RegisteredSubscription {
    subscription: ActiveSubscription,
    generation: GenerationToken,
    source_fence: StatusSourceFence,
}

impl RegisteredSubscription {
    fn snapshot(&self) -> SubscriptionSnapshot {
        SubscriptionSnapshot {
            subscription: self.subscription.clone(),
            generation: self.generation.clone(),
        }
    }

    fn is_current(&self, snapshot: &SubscriptionSnapshot) -> bool {
        self.generation.same_generation(&snapshot.generation)
    }

    fn update_status(
        &mut self,
        new_status: SubscriptionStatusCode,
    ) -> Result<SubscriptionStatusCode, SubscriptionError> {
        let old_status = self.subscription.status;
        if !old_status.can_transition_to(new_status) {
            return Err(SubscriptionError::InvalidStatusTransition {
                from: old_status.to_string(),
                to: new_status.to_string(),
            });
        }
        self.subscription.status = new_status;
        if new_status.is_terminal() {
            self.generation.make_terminal();
        }
        debug!(
            tenant = %self.subscription.tenant_id,
            subscription_id = %self.subscription.id,
            from = %old_status,
            to = %new_status,
            "Status transition"
        );
        Ok(old_status)
    }
}

/// Manages the lifecycle and in-memory indexing of active subscriptions.
pub struct SubscriptionManager {
    /// Active subscriptions keyed by (tenant_id, subscription_id).
    subscriptions: DashMap<(String, String), RegisteredSubscription>,

    /// Topic registry for validation.
    topic_registry: Arc<InMemoryTopicRegistry>,

    /// Supported channel types.
    supported_channels: Vec<String>,
}

impl SubscriptionManager {
    /// Creates a new subscription manager.
    pub fn new(
        topic_registry: Arc<InMemoryTopicRegistry>,
        supported_channels: Vec<String>,
    ) -> Self {
        Self {
            subscriptions: DashMap::new(),
            topic_registry,
            supported_channels,
        }
    }

    /// Registers a subscription from a FHIR Subscription resource JSON.
    ///
    /// Validates the topic URL, channel type, and filter criteria.
    /// On success, the subscription is stored in the in-memory index.
    pub fn register(
        &self,
        tenant_id: &str,
        subscription_id: &str,
        resource: &serde_json::Value,
        fhir_version: FhirVersion,
    ) -> Result<ActiveSubscription, SubscriptionError> {
        self.register_snapshot(tenant_id, subscription_id, resource, fhir_version, None)
            .map(|snapshot| snapshot.subscription)
    }

    /// Register atomically with the version of the write that announced it.
    /// Public callers without that version still retain a content fence and
    /// can supply `meta.versionId` in the resource itself.
    pub(crate) fn register_snapshot(
        &self,
        tenant_id: &str,
        subscription_id: &str,
        resource: &serde_json::Value,
        fhir_version: FhirVersion,
        source_version: Option<&str>,
    ) -> Result<SubscriptionSnapshot, SubscriptionError> {
        // Parse the subscription resource.
        let topic_url = extract_topic_url(resource, fhir_version)?;
        let channel = extract_channel_config(resource, fhir_version)?;
        let filter_strings = extract_filter_criteria(resource, fhir_version);

        // Validate topic exists.
        let topic = self.topic_registry.get_topic(&topic_url).ok_or_else(|| {
            SubscriptionError::TopicNotFound {
                url: topic_url.clone(),
            }
        })?;

        // Validate channel type is supported.
        if !self
            .supported_channels
            .contains(&channel.channel_type.as_fhir_str().to_string())
        {
            return Err(SubscriptionError::UnsupportedChannel {
                channel_type: channel.channel_type.as_fhir_str().to_string(),
            });
        }

        // Validate rest-hook endpoint is present.
        if channel.channel_type == ChannelType::RestHook && channel.endpoint.is_none() {
            return Err(SubscriptionError::InvalidEndpoint {
                message: "rest-hook channel requires an endpoint".to_string(),
            });
        }

        // Validate email endpoint is a non-empty mailto: URI.
        if channel.channel_type == ChannelType::Email {
            let endpoint =
                channel
                    .endpoint
                    .as_deref()
                    .ok_or_else(|| SubscriptionError::InvalidEndpoint {
                        message: "email channel requires a mailto: endpoint".to_string(),
                    })?;
            match endpoint.strip_prefix("mailto:") {
                Some(rest) if !rest.trim().is_empty() => {}
                _ => {
                    return Err(SubscriptionError::InvalidEndpoint {
                        message: "email endpoint must be a non-empty mailto: URI".to_string(),
                    });
                }
            }
        }

        // Parse and validate filters.
        let mut parsed_filters = Vec::new();
        for filter_str in &filter_strings {
            let filter = filters::parse_filter_string(filter_str)?;
            parsed_filters.push(filter);
        }
        filters::validate_filters(&parsed_filters, &topic.can_filter_by)?;

        let status = resource
            .get("status")
            .and_then(|v| v.as_str())
            .and_then(SubscriptionStatusCode::from_fhir_str)
            .unwrap_or(SubscriptionStatusCode::Requested);

        // Carry the runtime counters across a re-registration.
        //
        // `register` runs on *every* write to the resource (create, update,
        // patch — see `SubscriptionEngine::handle_subscription_event`), and it
        // used to rebuild the entry with both counters at zero. That made an
        // unrelated edit — adding a filter, rotating an endpoint header — a
        // silent reset of the delivery circuit breaker, re-arming the server
        // against an endpoint it had already given up on, with no restart
        // involved. It also rewound `eventsSinceSubscriptionStart`, which
        // subscribers use to detect missed notifications.
        //
        // The counters are therefore preserved unless the client is *re-arming*
        // the subscription (see [`is_rearm`]), which is the one deliberate,
        // client-driven reset and the documented way back from `error`/`off`.
        let key = (tenant_id.to_string(), subscription_id.to_string());
        let mut registered = RegisteredSubscription {
            subscription: ActiveSubscription {
                id: subscription_id.to_string(),
                topic_url,
                status,
                channel,
                filters: parsed_filters,
                fhir_version,
                events_since_start: 0,
                consecutive_failures: 0,
                tenant_id: tenant_id.to_string(),
            },
            generation: GenerationToken::new(status),
            source_fence: StatusSourceFence::new(resource, source_version),
        };

        // Counter preservation, invalidation, and replacement share one map
        // critical section: a concurrent delivery cannot increment a value
        // between the previous-counter read and the new registration's insert.
        let (snapshot, re_registered) = match self.subscriptions.entry(key) {
            Entry::Occupied(mut entry) => {
                let previous = entry.get();
                if !is_rearm(previous.subscription.status, status) {
                    registered.subscription.events_since_start =
                        previous.subscription.events_since_start;
                    registered.subscription.consecutive_failures =
                        previous.subscription.consecutive_failures;
                }
                previous.generation.invalidate();
                let snapshot = registered.snapshot();
                entry.insert(registered);
                (snapshot, true)
            }
            Entry::Vacant(entry) => {
                let snapshot = registered.snapshot();
                entry.insert(registered);
                (snapshot, false)
            }
        };

        debug!(
            tenant = tenant_id,
            subscription_id,
            topic = %snapshot.subscription.topic_url,
            status = %snapshot.subscription.status,
            events_since_start = snapshot.subscription.events_since_start,
            consecutive_failures = snapshot.subscription.consecutive_failures,
            re_registered,
            "Registered subscription"
        );

        Ok(snapshot)
    }

    /// Removes a subscription from the in-memory index.
    pub fn deregister(&self, tenant_id: &str, subscription_id: &str) -> bool {
        let key = (tenant_id.to_string(), subscription_id.to_string());
        let removed = match self.subscriptions.entry(key) {
            Entry::Occupied(entry) => {
                entry.get().generation.invalidate();
                entry.remove();
                true
            }
            Entry::Vacant(_) => false,
        };
        if removed {
            debug!(
                tenant = tenant_id,
                subscription_id, "Deregistered subscription"
            );
        }
        removed
    }

    /// Returns all active subscriptions for a given topic URL within a tenant.
    pub fn active_subscriptions_for_topic(
        &self,
        tenant_id: &str,
        topic_url: &str,
    ) -> Vec<ActiveSubscription> {
        self.subscriptions
            .iter()
            .filter(|entry| {
                let sub = &entry.value().subscription;
                sub.tenant_id == tenant_id
                    && sub.topic_url == topic_url
                    && sub.status == SubscriptionStatusCode::Active
            })
            .map(|entry| entry.value().subscription.clone())
            .collect()
    }

    /// Gets the current status of a subscription.
    pub fn get_subscription(
        &self,
        tenant_id: &str,
        subscription_id: &str,
    ) -> Option<ActiveSubscription> {
        let key = (tenant_id.to_string(), subscription_id.to_string());
        self.subscriptions
            .get(&key)
            .map(|entry| entry.subscription.clone())
    }

    /// Updates the status of a subscription.
    pub fn update_status(
        &self,
        tenant_id: &str,
        subscription_id: &str,
        new_status: SubscriptionStatusCode,
    ) -> Result<SubscriptionStatusCode, SubscriptionError> {
        let key = (tenant_id.to_string(), subscription_id.to_string());
        let mut entry = self.subscriptions.get_mut(&key).ok_or_else(|| {
            SubscriptionError::Internal(format!("subscription not found: {subscription_id}"))
        })?;

        entry.update_status(new_status)
    }

    /// Increments the event counter and returns the new value.
    pub fn increment_event_count(&self, tenant_id: &str, subscription_id: &str) -> Option<u64> {
        let key = (tenant_id.to_string(), subscription_id.to_string());
        let mut entry = self.subscriptions.get_mut(&key)?;
        entry.subscription.events_since_start += 1;
        Some(entry.subscription.events_since_start)
    }

    /// Records a delivery failure and returns the new consecutive failure count.
    pub fn record_failure(&self, tenant_id: &str, subscription_id: &str) -> Option<u32> {
        let key = (tenant_id.to_string(), subscription_id.to_string());
        let mut entry = self.subscriptions.get_mut(&key)?;
        entry.subscription.consecutive_failures += 1;
        let count = entry.subscription.consecutive_failures;
        warn!(
            tenant = tenant_id,
            subscription_id,
            consecutive_failures = count,
            "Delivery failure recorded"
        );
        Some(count)
    }

    /// Resets the consecutive failure counter after a successful delivery.
    pub fn reset_failures(&self, tenant_id: &str, subscription_id: &str) {
        let key = (tenant_id.to_string(), subscription_id.to_string());
        if let Some(mut entry) = self.subscriptions.get_mut(&key) {
            entry.subscription.consecutive_failures = 0;
        }
    }

    /// Returns all subscriptions (for heartbeat checking, initialization, etc.).
    pub fn all_subscriptions(&self) -> Vec<ActiveSubscription> {
        self.subscriptions
            .iter()
            .map(|e| e.value().subscription.clone())
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn get_snapshot(
        &self,
        tenant_id: &str,
        subscription_id: &str,
    ) -> Option<SubscriptionSnapshot> {
        let key = (tenant_id.to_string(), subscription_id.to_string());
        self.subscriptions.get(&key).map(|entry| entry.snapshot())
    }

    pub(crate) fn active_snapshots_for_topic(
        &self,
        tenant_id: &str,
        topic_url: &str,
    ) -> Vec<SubscriptionSnapshot> {
        self.subscriptions
            .iter()
            .filter(|entry| {
                let sub = &entry.subscription;
                sub.tenant_id == tenant_id
                    && sub.topic_url == topic_url
                    && sub.status == SubscriptionStatusCode::Active
            })
            .map(|entry| entry.snapshot())
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn all_snapshots(&self) -> Vec<SubscriptionSnapshot> {
        self.subscriptions
            .iter()
            .map(|entry| entry.snapshot())
            .collect()
    }

    pub(crate) fn is_current(&self, snapshot: &SubscriptionSnapshot) -> bool {
        let sub = &snapshot.subscription;
        let key = (sub.tenant_id.clone(), sub.id.clone());
        self.subscriptions
            .get(&key)
            .is_some_and(|entry| entry.is_current(snapshot))
    }

    pub(crate) fn is_current_status(
        &self,
        snapshot: &SubscriptionSnapshot,
        status: SubscriptionStatusCode,
    ) -> bool {
        let sub = &snapshot.subscription;
        let key = (sub.tenant_id.clone(), sub.id.clone());
        self.subscriptions
            .get(&key)
            .is_some_and(|entry| entry.is_current(snapshot) && entry.subscription.status == status)
    }

    /// Assign numbers only while the evaluated generation is still Active.
    pub(crate) fn increment_event_count_if_current(
        &self,
        snapshot: &SubscriptionSnapshot,
    ) -> Option<u64> {
        let sub = &snapshot.subscription;
        let key = (sub.tenant_id.clone(), sub.id.clone());
        let mut entry = self.subscriptions.get_mut(&key)?;
        if !entry.is_current(snapshot)
            || entry.subscription.status != SubscriptionStatusCode::Active
        {
            return None;
        }
        entry.subscription.events_since_start += 1;
        Some(entry.subscription.events_since_start)
    }

    /// Accept one delivery outcome for the current serving generation. The
    /// callback records rolling metrics synchronously under the same entry
    /// guard as the failure streak, so replacement cannot split acceptance.
    /// It must be cheap, perform no I/O, and never re-enter this manager.
    /// The returned failure count may drive async transitions after this returns.
    pub(crate) fn accept_delivery_outcome(
        &self,
        snapshot: &SubscriptionSnapshot,
        succeeded: bool,
        record_stats: impl FnOnce(),
    ) -> Option<u32> {
        let sub = &snapshot.subscription;
        let key = (sub.tenant_id.clone(), sub.id.clone());
        let mut entry = self.subscriptions.get_mut(&key)?;
        if !entry.is_current(snapshot)
            || !matches!(
                entry.subscription.status,
                SubscriptionStatusCode::Active | SubscriptionStatusCode::Error
            )
        {
            return None;
        }
        if succeeded {
            entry.subscription.consecutive_failures = 0;
        } else {
            entry.subscription.consecutive_failures += 1;
        }
        record_stats();
        let count = entry.subscription.consecutive_failures;
        if !succeeded {
            warn!(tenant = %sub.tenant_id, subscription_id = %sub.id, consecutive_failures = count, "Delivery failure recorded");
        }
        Some(count)
    }

    #[cfg(test)]
    pub(crate) fn record_failure_if_current(&self, snapshot: &SubscriptionSnapshot) -> Option<u32> {
        self.accept_delivery_outcome(snapshot, false, || {})
    }

    #[cfg(test)]
    pub(crate) fn reset_failures_if_current(&self, snapshot: &SubscriptionSnapshot) -> bool {
        self.accept_delivery_outcome(snapshot, true, || {})
            .is_some()
    }

    pub(crate) fn update_status_if_current(
        &self,
        snapshot: &SubscriptionSnapshot,
        new_status: SubscriptionStatusCode,
    ) -> Option<SubscriptionStatusCode> {
        let sub = &snapshot.subscription;
        let key = (sub.tenant_id.clone(), sub.id.clone());
        let mut entry = self.subscriptions.get_mut(&key)?;
        if !entry.is_current(snapshot) {
            return None;
        }
        entry.update_status(new_status).ok()
    }

    /// This fence can be used after an async storage read without retaining a
    /// map guard. Generation/status must be rechecked before issuing the write.
    pub(crate) fn source_fence(
        &self,
        snapshot: &SubscriptionSnapshot,
    ) -> Option<StatusSourceFence> {
        let sub = &snapshot.subscription;
        let key = (sub.tenant_id.clone(), sub.id.clone());
        let entry = self.subscriptions.get(&key)?;
        entry
            .is_current(snapshot)
            .then(|| entry.source_fence.clone())
    }

    /// Record successful server write-back only for the same registration and
    /// prior source. A late completion cannot advance a client's replacement
    /// fence. Callers serialize own writes with the generation's write lock.
    pub(crate) fn acknowledge_status_write(
        &self,
        snapshot: &SubscriptionSnapshot,
        expected_fence: &StatusSourceFence,
        new_version: &str,
        content: &serde_json::Value,
    ) -> bool {
        if new_version.is_empty() {
            return false;
        }
        let sub = &snapshot.subscription;
        let key = (sub.tenant_id.clone(), sub.id.clone());
        let mut entry = match self.subscriptions.get_mut(&key) {
            Some(entry) => entry,
            None => return false,
        };
        if !entry.is_current(snapshot) || &entry.source_fence != expected_fence {
            return false;
        }
        entry.source_fence = StatusSourceFence::new(content, Some(new_version));
        true
    }
}

/// Whether re-registering with `incoming` status re-arms a subscription that
/// was previously in `previous` status, and so should reset its runtime
/// counters.
///
/// Two cases count as re-arming, and nothing else does:
///
/// 1. The client wrote `requested` — the spec's "please (re-)activate this"
///    signal, whatever state the subscription was in.
/// 2. The client moved it out of a non-serving state (`error`, `off`,
///    `entered-in-error`) under its own steam, e.g. straight to `active`.
///
/// An ordinary edit that leaves the status alone — or any server-authored
/// status already written back into the resource — is *not* a re-arm, so it
/// keeps the delivery failure streak and the event counter intact.
fn is_rearm(previous: SubscriptionStatusCode, incoming: SubscriptionStatusCode) -> bool {
    if incoming == SubscriptionStatusCode::Requested {
        return true;
    }
    matches!(
        previous,
        SubscriptionStatusCode::Error
            | SubscriptionStatusCode::Off
            | SubscriptionStatusCode::EnteredInError
    ) && !matches!(
        incoming,
        SubscriptionStatusCode::Error
            | SubscriptionStatusCode::Off
            | SubscriptionStatusCode::EnteredInError
    )
}

// --- Version-aware field extraction ---

/// Extract the topic canonical URL from a Subscription resource.
fn extract_topic_url(
    resource: &serde_json::Value,
    fhir_version: FhirVersion,
) -> Result<String, SubscriptionError> {
    match fhir_version {
        // R4: topic URL is in the `criteria` field (backport IG convention)
        // or in the `backport-topic-canonical` extension.
        #[cfg(feature = "R4")]
        FhirVersion::R4 => {
            // First try backport extension.
            if let Some(url) = find_extension_value_url(
                resource,
                "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-topic-canonical",
            ) {
                return Ok(url);
            }
            // Fall back to criteria field.
            resource
                .get("criteria")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| SubscriptionError::InvalidSubscription {
                    message: "R4 Subscription missing topic URL (criteria or backport-topic-canonical extension)".to_string(),
                })
        }

        // R4B+: topic URL is in the `topic` field (canonical reference).
        #[allow(unreachable_patterns)]
        _ => resource
            .get("topic")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| SubscriptionError::InvalidSubscription {
                message: "Subscription missing 'topic' field".to_string(),
            }),
    }
}

/// Extract channel configuration from a Subscription resource.
fn extract_channel_config(
    resource: &serde_json::Value,
    fhir_version: FhirVersion,
) -> Result<ChannelConfig, SubscriptionError> {
    match fhir_version {
        // R4: channel is a nested object with type, endpoint, payload, header.
        #[cfg(feature = "R4")]
        FhirVersion::R4 => {
            let channel =
                resource
                    .get("channel")
                    .ok_or_else(|| SubscriptionError::InvalidSubscription {
                        message: "R4 Subscription missing 'channel' field".to_string(),
                    })?;

            let channel_type_str = channel
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("rest-hook");

            let endpoint = channel
                .get("endpoint")
                .and_then(|v| v.as_str())
                .map(String::from);

            let payload_mime_type = channel
                .get("payload")
                .and_then(|v| v.as_str())
                .map(String::from);

            let headers = channel
                .get("header")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            // Payload content from backport extension.
            // Prefer channel._payload extension (spec-compliant) and fall back
            // to root extension for backward compatibility.
            let payload_content = find_channel_payload_content_code(resource)
                .or_else(|| {
                    find_extension_value_code(
                        resource,
                        "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-payload-content",
                    )
                })
            .and_then(|s| PayloadContent::from_fhir_str(&s))
            .unwrap_or(PayloadContent::IdOnly);

            let heartbeat_period = find_extension_value_unsigned_int(
                resource,
                "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-heartbeat-period",
            );

            let timeout = find_extension_value_unsigned_int(
                resource,
                "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-timeout",
            );

            let max_count = find_extension_value_unsigned_int(
                resource,
                "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-max-count",
            );

            Ok(ChannelConfig {
                channel_type: ChannelType::from_fhir_str(channel_type_str),
                endpoint,
                payload_mime_type,
                payload_content,
                headers,
                heartbeat_period,
                timeout,
                max_count,
            })
        }

        // R4B+: channel type is `channelType` coding, endpoint, content, etc.
        #[allow(unreachable_patterns)]
        _ => {
            let channel_type_str = resource
                .get("channelType")
                .and_then(|v| v.get("code"))
                .and_then(|v| v.as_str())
                .unwrap_or("rest-hook");

            let endpoint = resource
                .get("endpoint")
                .and_then(|v| v.as_str())
                .map(String::from);

            let payload_content = resource
                .get("content")
                .and_then(|v| v.as_str())
                .and_then(PayloadContent::from_fhir_str)
                .unwrap_or(PayloadContent::IdOnly);

            let payload_mime_type = resource
                .get("contentType")
                .and_then(|v| v.as_str())
                .map(String::from);

            let heartbeat_period = resource
                .get("heartbeatPeriod")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);

            let timeout = resource
                .get("timeout")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);

            let max_count = resource
                .get("maxCount")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);

            let headers = resource
                .get("parameter")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|p| {
                            let name = p.get("name")?.as_str()?;
                            let value = p.get("value")?.as_str()?;
                            Some(format!("{name}: {value}"))
                        })
                        .collect()
                })
                .unwrap_or_default();

            Ok(ChannelConfig {
                channel_type: ChannelType::from_fhir_str(channel_type_str),
                endpoint,
                payload_mime_type,
                payload_content,
                headers,
                heartbeat_period,
                timeout,
                max_count,
            })
        }
    }
}

/// Extract filter criteria strings from a Subscription resource.
fn extract_filter_criteria(resource: &serde_json::Value, fhir_version: FhirVersion) -> Vec<String> {
    match fhir_version {
        // R4: filter criteria from backport extension.
        #[cfg(feature = "R4")]
        FhirVersion::R4 => find_all_extension_values_string(
            resource,
            "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-filter-criteria",
        ),

        // R4B+: filterBy array with resourceType, filterParameter, comparator, value.
        #[allow(unreachable_patterns)]
        _ => {
            let filter_by = match resource.get("filterBy").and_then(|v| v.as_array()) {
                Some(arr) => arr,
                None => return Vec::new(),
            };

            filter_by
                .iter()
                .filter_map(|f| {
                    let param = f.get("filterParameter")?.as_str()?;
                    let value = f.get("value")?.as_str()?;
                    let resource_type = f.get("resourceType").and_then(|v| v.as_str());
                    let comparator = f.get("comparator").and_then(|v| v.as_str());
                    let value_with_comparator = comparator
                        .filter(|c| !c.is_empty())
                        .map(|c| format!("{c}:{value}"))
                        .unwrap_or_else(|| value.to_string());

                    let filter_str = if let Some(rt) = resource_type {
                        format!("{rt}?{param}={value_with_comparator}")
                    } else {
                        format!("{param}={value_with_comparator}")
                    };

                    Some(filter_str)
                })
                .collect()
        }
    }
}

// --- Extension helpers for R4 backport ---

/// Find a single extension with the given URL and return its `valueUrl`.
#[cfg(feature = "R4")]
fn find_extension_value_url(resource: &serde_json::Value, url: &str) -> Option<String> {
    resource
        .get("extension")?
        .as_array()?
        .iter()
        .find(|ext| ext.get("url").and_then(|v| v.as_str()) == Some(url))
        .and_then(|ext| ext.get("valueUrl").or_else(|| ext.get("valueCanonical")))
        .and_then(|v| v.as_str())
        .map(String::from)
}

/// Find a single extension with the given URL and return its `valueCode`.
#[cfg(feature = "R4")]
fn find_extension_value_code(resource: &serde_json::Value, url: &str) -> Option<String> {
    resource
        .get("extension")?
        .as_array()?
        .iter()
        .find(|ext| ext.get("url").and_then(|v| v.as_str()) == Some(url))
        .and_then(|ext| ext.get("valueCode"))
        .and_then(|v| v.as_str())
        .map(String::from)
}

#[cfg(feature = "R4")]
fn find_channel_payload_content_code(resource: &serde_json::Value) -> Option<String> {
    resource
        .get("channel")?
        .get("_payload")?
        .get("extension")?
        .as_array()?
        .iter()
        .find(|ext| {
            ext.get("url").and_then(|v| v.as_str())
                == Some(
                    "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-payload-content",
                )
        })
        .and_then(|ext| ext.get("valueCode"))
        .and_then(|v| v.as_str())
        .map(String::from)
}

/// Find a single extension with the given URL and return its `valueUnsignedInt`.
#[cfg(feature = "R4")]
fn find_extension_value_unsigned_int(resource: &serde_json::Value, url: &str) -> Option<u32> {
    resource
        .get("extension")?
        .as_array()?
        .iter()
        .find(|ext| ext.get("url").and_then(|v| v.as_str()) == Some(url))
        .and_then(|ext| ext.get("valueUnsignedInt"))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
}

/// Find all extensions with the given URL and return their `valueString` values.
#[cfg(feature = "R4")]
fn find_all_extension_values_string(resource: &serde_json::Value, url: &str) -> Vec<String> {
    resource
        .get("extension")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|ext| ext.get("url").and_then(|v| v.as_str()) == Some(url))
                .filter_map(|ext| {
                    ext.get("valueString")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::event::ResourceEventType;
    use crate::topics::{FilterDefinition, ResourceTrigger, TopicDefinition};
    use serde_json::json;

    fn create_test_registry() -> Arc<InMemoryTopicRegistry> {
        let registry = Arc::new(InMemoryTopicRegistry::new());
        registry.add_topic(TopicDefinition {
            canonical_url: "http://example.org/topic/encounter-start".to_string(),
            title: Some("Encounter Start".to_string()),
            resource_triggers: vec![ResourceTrigger {
                resource_type: "Encounter".to_string(),
                interactions: vec![ResourceEventType::Create],
                fhirpath_criteria: None,
            }],
            can_filter_by: vec![FilterDefinition {
                resource_type: Some("Encounter".to_string()),
                filter_parameter: "patient".to_string(),
                comparators: vec!["eq".to_string()],
                modifiers: vec![],
            }],
            notification_shape: vec![],
        });
        registry
    }

    /// Returns a subscription JSON compatible with the default FHIR version.
    /// R4 uses criteria + channel object; R4B+ uses topic + channelType.
    pub(crate) fn default_subscription_json() -> serde_json::Value {
        #[cfg(feature = "R4")]
        {
            json!({
                "resourceType": "Subscription",
                "id": "sub-1",
                "status": "requested",
                "criteria": "http://example.org/topic/encounter-start",
                "channel": {
                    "type": "rest-hook",
                    "endpoint": "https://example.com/webhook",
                    "payload": "application/fhir+json"
                },
                "extension": [
                    {
                        "url": "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-topic-canonical",
                        "valueCanonical": "http://example.org/topic/encounter-start"
                    },
                    {
                        "url": "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-payload-content",
                        "valueCode": "id-only"
                    }
                ]
            })
        }
        #[cfg(not(feature = "R4"))]
        {
            json!({
                "resourceType": "Subscription",
                "id": "sub-1",
                "status": "requested",
                "topic": "http://example.org/topic/encounter-start",
                "channelType": {
                    "system": "http://terminology.hl7.org/CodeSystem/subscription-channel-type",
                    "code": "rest-hook"
                },
                "endpoint": "https://example.com/webhook",
                "content": "id-only",
                "contentType": "application/fhir+json"
            })
        }
    }

    #[cfg(feature = "R4")]
    fn r4_subscription_json() -> serde_json::Value {
        json!({
            "resourceType": "Subscription",
            "id": "sub-r4-1",
            "status": "requested",
            "criteria": "http://example.org/topic/encounter-start",
            "channel": {
                "type": "rest-hook",
                "endpoint": "https://example.com/webhook",
                "payload": "application/fhir+json",
                "header": ["Authorization: Bearer token123"]
            },
            "extension": [
                {
                    "url": "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-topic-canonical",
                    "valueCanonical": "http://example.org/topic/encounter-start"
                },
                {
                    "url": "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-payload-content",
                    "valueCode": "id-only"
                },
                {
                    "url": "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-heartbeat-period",
                    "valueUnsignedInt": 60
                }
            ]
        })
    }

    #[cfg(feature = "R5")]
    fn non_r4_test_version() -> FhirVersion {
        FhirVersion::R5
    }

    #[cfg(all(not(feature = "R5"), feature = "R4B"))]
    fn non_r4_test_version() -> FhirVersion {
        FhirVersion::R4B
    }

    #[cfg(all(not(feature = "R5"), not(feature = "R4B"), feature = "R6"))]
    fn non_r4_test_version() -> FhirVersion {
        FhirVersion::R6
    }

    #[test]
    fn test_register_r5_subscription() {
        let registry = create_test_registry();
        let manager = SubscriptionManager::new(registry, vec!["rest-hook".to_string()]);

        let resource = default_subscription_json();
        let sub = manager
            .register("tenant-1", "sub-1", &resource, FhirVersion::default())
            .unwrap();

        assert_eq!(sub.id, "sub-1");
        assert_eq!(sub.topic_url, "http://example.org/topic/encounter-start");
        assert_eq!(sub.channel.channel_type, ChannelType::RestHook);
        assert_eq!(
            sub.channel.endpoint.as_deref(),
            Some("https://example.com/webhook")
        );
        assert_eq!(sub.channel.payload_content, PayloadContent::IdOnly);
        assert_eq!(sub.status, SubscriptionStatusCode::Requested);
        assert_eq!(sub.events_since_start, 0);
    }

    #[cfg(any(feature = "R4B", feature = "R5", feature = "R6"))]
    #[test]
    fn test_register_native_subscription_topic_and_channeltype_parsing() {
        let registry = create_test_registry();
        let manager = SubscriptionManager::new(registry, vec!["rest-hook".to_string()]);

        let resource = json!({
            "resourceType": "Subscription",
            "id": "sub-native-1",
            "status": "requested",
            "topic": "http://example.org/topic/encounter-start",
            "channelType": {
                "system": "http://terminology.hl7.org/CodeSystem/subscription-channel-type",
                "code": "rest-hook"
            },
            "endpoint": "https://example.com/webhook",
            "contentType": "application/fhir+json",
            "content": "id-only",
            "parameter": [{
                "name": "Authorization",
                "value": "Bearer native-token"
            }]
        });

        let sub = manager
            .register("tenant-1", "sub-native-1", &resource, non_r4_test_version())
            .unwrap();

        assert_eq!(sub.topic_url, "http://example.org/topic/encounter-start");
        assert_eq!(sub.channel.channel_type, ChannelType::RestHook);
        assert_eq!(
            sub.channel.endpoint.as_deref(),
            Some("https://example.com/webhook")
        );
        assert_eq!(sub.channel.payload_content, PayloadContent::IdOnly);
        assert_eq!(
            sub.channel.headers,
            vec!["Authorization: Bearer native-token".to_string()]
        );
    }

    #[cfg(feature = "R4")]
    #[test]
    fn test_register_r4_subscription_with_backport_extensions() {
        let registry = create_test_registry();
        let manager = SubscriptionManager::new(registry, vec!["rest-hook".to_string()]);

        let resource = r4_subscription_json();
        let sub = manager
            .register("tenant-1", "sub-r4-1", &resource, FhirVersion::R4)
            .unwrap();

        assert_eq!(sub.topic_url, "http://example.org/topic/encounter-start");
        assert_eq!(sub.channel.channel_type, ChannelType::RestHook);
        assert_eq!(sub.channel.payload_content, PayloadContent::IdOnly);
        assert_eq!(sub.channel.heartbeat_period, Some(60));
        assert_eq!(sub.channel.headers.len(), 1);
        assert!(sub.channel.headers[0].contains("Authorization"));
    }

    #[cfg(feature = "R4")]
    #[test]
    fn test_register_r4_subscription_payload_content_in_channel_payload_extension() {
        let registry = create_test_registry();
        let manager = SubscriptionManager::new(registry, vec!["rest-hook".to_string()]);

        let resource = json!({
            "resourceType": "Subscription",
            "id": "sub-r4-2",
            "status": "requested",
            "criteria": "http://example.org/topic/encounter-start",
            "channel": {
                "type": "rest-hook",
                "endpoint": "https://example.com/webhook",
                "payload": "application/fhir+json",
                "_payload": {
                    "extension": [{
                        "url": "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-payload-content",
                        "valueCode": "empty"
                    }]
                }
            }
        });

        let sub = manager
            .register("tenant-1", "sub-r4-2", &resource, FhirVersion::R4)
            .unwrap();

        assert_eq!(sub.channel.payload_content, PayloadContent::Empty);
    }

    /// Build a version-appropriate subscription JSON with custom topic/channel.
    pub(crate) fn build_subscription_json(
        topic_url: &str,
        channel_type: &str,
        endpoint: Option<&str>,
    ) -> serde_json::Value {
        #[cfg(feature = "R4")]
        {
            let mut sub = json!({
                "resourceType": "Subscription",
                "status": "requested",
                "criteria": topic_url,
                "channel": {
                    "type": channel_type
                },
                "extension": [{
                    "url": "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-topic-canonical",
                    "valueCanonical": topic_url
                }]
            });
            if let Some(ep) = endpoint {
                sub["channel"]["endpoint"] = json!(ep);
            }
            sub
        }
        #[cfg(not(feature = "R4"))]
        {
            let mut sub = json!({
                "resourceType": "Subscription",
                "status": "requested",
                "topic": topic_url,
                "channelType": { "code": channel_type }
            });
            if let Some(ep) = endpoint {
                sub["endpoint"] = json!(ep);
            }
            sub
        }
    }

    #[test]
    fn test_register_invalid_topic() {
        let registry = create_test_registry();
        let manager = SubscriptionManager::new(registry, vec!["rest-hook".to_string()]);

        let resource = build_subscription_json(
            "http://nonexistent.org/topic/missing",
            "rest-hook",
            Some("https://example.com/webhook"),
        );

        let result = manager.register("t1", "s1", &resource, FhirVersion::default());
        assert!(matches!(
            result,
            Err(SubscriptionError::TopicNotFound { .. })
        ));
    }

    #[test]
    fn test_register_unsupported_channel() {
        let registry = create_test_registry();
        let manager = SubscriptionManager::new(registry, vec!["rest-hook".to_string()]);

        let resource = build_subscription_json(
            "http://example.org/topic/encounter-start",
            "email",
            Some("mailto:test@example.com"),
        );

        let result = manager.register("t1", "s1", &resource, FhirVersion::default());
        assert!(matches!(
            result,
            Err(SubscriptionError::UnsupportedChannel { .. })
        ));
    }

    #[test]
    fn test_register_rest_hook_missing_endpoint() {
        let registry = create_test_registry();
        let manager = SubscriptionManager::new(registry, vec!["rest-hook".to_string()]);

        let resource = build_subscription_json(
            "http://example.org/topic/encounter-start",
            "rest-hook",
            None, // No endpoint.
        );

        let result = manager.register("t1", "s1", &resource, FhirVersion::default());
        assert!(matches!(
            result,
            Err(SubscriptionError::InvalidEndpoint { .. })
        ));
    }

    #[test]
    fn test_deregister() {
        let registry = create_test_registry();
        let manager = SubscriptionManager::new(registry, vec!["rest-hook".to_string()]);

        let resource = default_subscription_json();
        manager
            .register("t1", "sub-1", &resource, FhirVersion::default())
            .unwrap();

        assert!(manager.deregister("t1", "sub-1"));
        assert!(manager.get_subscription("t1", "sub-1").is_none());

        // Deregistering again returns false.
        assert!(!manager.deregister("t1", "sub-1"));
    }

    #[test]
    fn test_active_subscriptions_for_topic() {
        let registry = create_test_registry();
        let manager = SubscriptionManager::new(registry, vec!["rest-hook".to_string()]);

        let resource = default_subscription_json();
        manager
            .register("t1", "sub-1", &resource, FhirVersion::default())
            .unwrap();

        // Subscription is in "requested" status, should not appear in active list.
        let active = manager
            .active_subscriptions_for_topic("t1", "http://example.org/topic/encounter-start");
        assert!(active.is_empty());

        // Transition to active.
        manager
            .update_status("t1", "sub-1", SubscriptionStatusCode::Active)
            .unwrap();

        let active = manager
            .active_subscriptions_for_topic("t1", "http://example.org/topic/encounter-start");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "sub-1");
    }

    #[test]
    fn test_status_transitions() {
        let registry = create_test_registry();
        let manager = SubscriptionManager::new(registry, vec!["rest-hook".to_string()]);

        let resource = default_subscription_json();
        manager
            .register("t1", "sub-1", &resource, FhirVersion::default())
            .unwrap();

        // Requested -> Active (valid).
        let old = manager
            .update_status("t1", "sub-1", SubscriptionStatusCode::Active)
            .unwrap();
        assert_eq!(old, SubscriptionStatusCode::Requested);

        // Active -> Error (valid).
        manager
            .update_status("t1", "sub-1", SubscriptionStatusCode::Error)
            .unwrap();

        // Error -> Active (valid, recovery).
        manager
            .update_status("t1", "sub-1", SubscriptionStatusCode::Active)
            .unwrap();

        // Active -> Requested (invalid).
        let result = manager.update_status("t1", "sub-1", SubscriptionStatusCode::Requested);
        assert!(matches!(
            result,
            Err(SubscriptionError::InvalidStatusTransition { .. })
        ));
    }

    #[test]
    fn test_increment_event_count() {
        let registry = create_test_registry();
        let manager = SubscriptionManager::new(registry, vec!["rest-hook".to_string()]);

        let resource = default_subscription_json();
        manager
            .register("t1", "sub-1", &resource, FhirVersion::default())
            .unwrap();

        assert_eq!(manager.increment_event_count("t1", "sub-1"), Some(1));
        assert_eq!(manager.increment_event_count("t1", "sub-1"), Some(2));
        assert_eq!(manager.increment_event_count("t1", "sub-1"), Some(3));

        // Nonexistent subscription.
        assert!(manager.increment_event_count("t1", "nonexistent").is_none());
    }

    #[test]
    fn test_record_and_reset_failures() {
        let registry = create_test_registry();
        let manager = SubscriptionManager::new(registry, vec!["rest-hook".to_string()]);

        let resource = default_subscription_json();
        manager
            .register("t1", "sub-1", &resource, FhirVersion::default())
            .unwrap();

        assert_eq!(manager.record_failure("t1", "sub-1"), Some(1));
        assert_eq!(manager.record_failure("t1", "sub-1"), Some(2));

        manager.reset_failures("t1", "sub-1");
        let sub = manager.get_subscription("t1", "sub-1").unwrap();
        assert_eq!(sub.consecutive_failures, 0);
    }

    #[test]
    fn test_tenant_isolation() {
        let registry = create_test_registry();
        let manager = SubscriptionManager::new(registry, vec!["rest-hook".to_string()]);

        let resource = default_subscription_json();
        manager
            .register("tenant-a", "sub-1", &resource, FhirVersion::default())
            .unwrap();
        manager
            .register("tenant-b", "sub-1", &resource, FhirVersion::default())
            .unwrap();

        // Each tenant sees their own subscription.
        assert!(manager.get_subscription("tenant-a", "sub-1").is_some());
        assert!(manager.get_subscription("tenant-b", "sub-1").is_some());

        // Deregistering in one tenant doesn't affect the other.
        manager.deregister("tenant-a", "sub-1");
        assert!(manager.get_subscription("tenant-a", "sub-1").is_none());
        assert!(manager.get_subscription("tenant-b", "sub-1").is_some());
    }

    #[test]
    fn test_register_with_filter_by() {
        let registry = create_test_registry();
        let manager = SubscriptionManager::new(registry, vec!["rest-hook".to_string()]);

        #[cfg(feature = "R4")]
        let (resource, fhir_version) = (
            json!({
                "resourceType": "Subscription",
                "status": "requested",
                "criteria": "http://example.org/topic/encounter-start",
                "channel": {
                    "type": "rest-hook",
                    "endpoint": "https://example.com/webhook"
                },
                "extension": [
                    {
                        "url": "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-topic-canonical",
                        "valueCanonical": "http://example.org/topic/encounter-start"
                    },
                    {
                        "url": "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-payload-content",
                        "valueCode": "full-resource"
                    },
                    {
                        "url": "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-filter-criteria",
                        "valueString": "Encounter?patient=Patient/123"
                    }
                ]
            }),
            FhirVersion::R4,
        );
        #[cfg(all(
            not(feature = "R4"),
            any(feature = "R4B", feature = "R5", feature = "R6")
        ))]
        let (resource, fhir_version) = (
            json!({
                "resourceType": "Subscription",
                "status": "requested",
                "topic": "http://example.org/topic/encounter-start",
                "channelType": { "code": "rest-hook" },
                "endpoint": "https://example.com/webhook",
                "content": "full-resource",
                "filterBy": [{
                    "resourceType": "Encounter",
                    "filterParameter": "patient",
                    "value": "Patient/123"
                }]
            }),
            non_r4_test_version(),
        );

        let sub = manager
            .register("t1", "sub-1", &resource, fhir_version)
            .unwrap();

        assert_eq!(sub.filters.len(), 1);
        assert_eq!(sub.filters[0].filter_parameter, "patient");
        assert_eq!(sub.filters[0].value, "Patient/123");
        assert_eq!(sub.channel.payload_content, PayloadContent::FullResource);
    }

    #[cfg(any(feature = "R4B", feature = "R5", feature = "R6"))]
    #[test]
    fn test_register_with_native_filter_by_comparator() {
        let registry = create_test_registry();
        let manager = SubscriptionManager::new(registry, vec!["rest-hook".to_string()]);

        let resource = json!({
            "resourceType": "Subscription",
            "status": "requested",
            "topic": "http://example.org/topic/encounter-start",
            "channelType": { "code": "rest-hook" },
            "endpoint": "https://example.com/webhook",
            "filterBy": [{
                "resourceType": "Encounter",
                "filterParameter": "patient",
                "comparator": "eq",
                "value": "Patient/123"
            }]
        });

        let sub = manager
            .register("t1", "sub-1", &resource, non_r4_test_version())
            .unwrap();

        assert_eq!(sub.filters.len(), 1);
        assert_eq!(sub.filters[0].filter_parameter, "patient");
        assert_eq!(sub.filters[0].comparator, "eq");
        assert_eq!(sub.filters[0].value, "Patient/123");
    }

    #[test]
    fn test_register_with_invalid_filter() {
        let registry = create_test_registry();
        let manager = SubscriptionManager::new(registry, vec!["rest-hook".to_string()]);

        #[cfg(feature = "R4")]
        let (resource, fhir_version) = (
            json!({
                "resourceType": "Subscription",
                "status": "requested",
                "criteria": "http://example.org/topic/encounter-start",
                "channel": {
                    "type": "rest-hook",
                    "endpoint": "https://example.com/webhook"
                },
                "extension": [
                    {
                        "url": "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-topic-canonical",
                        "valueCanonical": "http://example.org/topic/encounter-start"
                    },
                    {
                        "url": "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-filter-criteria",
                        "valueString": "unknown-param=some-value"
                    }
                ]
            }),
            FhirVersion::R4,
        );
        #[cfg(all(
            not(feature = "R4"),
            any(feature = "R4B", feature = "R5", feature = "R6")
        ))]
        let (resource, fhir_version) = (
            json!({
                "resourceType": "Subscription",
                "status": "requested",
                "topic": "http://example.org/topic/encounter-start",
                "channelType": { "code": "rest-hook" },
                "endpoint": "https://example.com/webhook",
                "filterBy": [{
                    "filterParameter": "unknown-param",
                    "value": "some-value"
                }]
            }),
            non_r4_test_version(),
        );

        let result = manager.register("t1", "sub-1", &resource, fhir_version);
        assert!(matches!(
            result,
            Err(SubscriptionError::InvalidFilter { .. })
        ));
    }

    // ── Runtime counters across re-registration (issue #357) ──────────────
    //
    // `register` runs on every write to the Subscription resource. It used to
    // rebuild the entry with both counters at zero, which made an ordinary edit
    // a silent reset of the delivery circuit breaker.

    /// Builds the default test subscription with `status` replaced.
    fn subscription_with_status(status: &str) -> serde_json::Value {
        let mut resource = default_subscription_json();
        resource["status"] = json!(status);
        resource
    }

    /// Drives a subscription to `status` with the given counters, the way the
    /// engine does: register, then transition and record activity.
    fn manager_with_state(
        status: SubscriptionStatusCode,
        events: u64,
        failures: u32,
    ) -> SubscriptionManager {
        let manager =
            SubscriptionManager::new(create_test_registry(), vec!["rest-hook".to_string()]);
        manager
            .register(
                "t1",
                "sub-1",
                &subscription_with_status("requested"),
                FhirVersion::default(),
            )
            .expect("initial register");
        if status != SubscriptionStatusCode::Requested {
            manager
                .update_status("t1", "sub-1", status)
                .expect("drive to status");
        }
        for _ in 0..events {
            manager.increment_event_count("t1", "sub-1");
        }
        for _ in 0..failures {
            manager.record_failure("t1", "sub-1");
        }
        manager
    }

    /// An edit that does not change the status must not reset the counters.
    /// This is the client-PUT circuit-breaker reset: before #357, adding a
    /// filter or rotating a header re-armed the server against an endpoint it
    /// had already given up on, with no restart involved.
    #[test]
    fn test_reregistration_preserves_counters() {
        let manager = manager_with_state(SubscriptionStatusCode::Active, 7, 2);

        let re = manager
            .register(
                "t1",
                "sub-1",
                &subscription_with_status("active"),
                FhirVersion::default(),
            )
            .expect("re-register");

        assert_eq!(re.events_since_start, 7, "event counter must not rewind");
        assert_eq!(
            re.consecutive_failures, 2,
            "failure streak must survive an ordinary edit"
        );
        assert_eq!(re.status, SubscriptionStatusCode::Active);
    }

    /// Writing `requested` is the spec's "please (re-)activate this" signal and
    /// is the documented way back from `error`/`off`. It resets the counters.
    #[test]
    fn test_client_rearm_with_requested_resets_counters() {
        let manager = manager_with_state(SubscriptionStatusCode::Error, 9, 5);

        let re = manager
            .register(
                "t1",
                "sub-1",
                &subscription_with_status("requested"),
                FhirVersion::default(),
            )
            .expect("re-register");

        assert_eq!(re.status, SubscriptionStatusCode::Requested);
        assert_eq!(re.events_since_start, 0);
        assert_eq!(re.consecutive_failures, 0);
    }

    /// Moving a subscription out of `off` under the client's own steam is also
    /// a re-arm: keeping the failure streak would re-trip the breaker on the
    /// very next failure.
    #[test]
    fn test_client_rearm_out_of_off_resets_counters() {
        let manager = manager_with_state(SubscriptionStatusCode::Off, 4, 10);

        let re = manager
            .register(
                "t1",
                "sub-1",
                &subscription_with_status("active"),
                FhirVersion::default(),
            )
            .expect("re-register");

        assert_eq!(re.status, SubscriptionStatusCode::Active);
        assert_eq!(re.consecutive_failures, 0);
    }

    /// A server-authored status written back into the resource (#357) is NOT a
    /// re-arm — otherwise the write-back would erase the very counters it is
    /// meant to make durable the next time the resource is registered.
    #[test]
    fn test_written_back_status_is_not_a_rearm() {
        let manager = manager_with_state(SubscriptionStatusCode::Error, 3, 4);

        // The resource now carries `error`, because write-back put it there.
        let re = manager
            .register(
                "t1",
                "sub-1",
                &subscription_with_status("error"),
                FhirVersion::default(),
            )
            .expect("re-register");

        assert_eq!(re.status, SubscriptionStatusCode::Error);
        assert_eq!(re.events_since_start, 3);
        assert_eq!(re.consecutive_failures, 4);
    }

    /// A first registration always starts from zero — there is nothing to carry.
    #[test]
    fn test_first_registration_starts_at_zero() {
        let manager =
            SubscriptionManager::new(create_test_registry(), vec!["rest-hook".to_string()]);
        let sub = manager
            .register(
                "t1",
                "sub-1",
                &subscription_with_status("active"),
                FhirVersion::default(),
            )
            .expect("register");
        assert_eq!(sub.events_since_start, 0);
        assert_eq!(sub.consecutive_failures, 0);
    }

    /// Counters are per `(tenant, subscription)`: re-registering one tenant's
    /// subscription must not read another tenant's state.
    #[test]
    fn test_counters_are_tenant_scoped() {
        let manager = manager_with_state(SubscriptionStatusCode::Active, 6, 3);

        let other = manager
            .register(
                "t2",
                "sub-1",
                &subscription_with_status("active"),
                FhirVersion::default(),
            )
            .expect("register for other tenant");

        assert_eq!(other.events_since_start, 0);
        assert_eq!(other.consecutive_failures, 0);
        // The first tenant is untouched.
        let first = manager.get_subscription("t1", "sub-1").unwrap();
        assert_eq!(first.events_since_start, 6);
    }

    #[test]
    fn stale_terminal_and_deleted_outcomes_never_record_metrics_or_erase_prior_history() {
        let manager = manager_with_state(SubscriptionStatusCode::Active, 7, 2);
        let stats = crate::delivery_stats::DeliveryStats::new();
        let now = chrono::Utc::now().timestamp();
        stats.record_success("t1", "sub-1", true, now);
        let old = manager.get_snapshot("t1", "sub-1").unwrap();
        let replacement = manager
            .register_snapshot(
                "t1",
                "sub-1",
                &subscription_with_status("active"),
                FhirVersion::default(),
                Some("2"),
            )
            .unwrap();
        for succeeded in [false, true] {
            assert_eq!(
                manager.accept_delivery_outcome(&old, succeeded, || panic!(
                    "stale callback must not run"
                )),
                None
            );
        }
        assert_eq!(
            manager
                .get_subscription("t1", "sub-1")
                .unwrap()
                .consecutive_failures,
            2
        );
        manager.deregister("t1", "sub-1");
        let recreated = manager
            .register_snapshot(
                "t1",
                "sub-1",
                &subscription_with_status("active"),
                FhirVersion::default(),
                Some("3"),
            )
            .unwrap();
        assert_eq!(
            manager.accept_delivery_outcome(&replacement, false, || panic!(
                "deleted callback must not run"
            )),
            None
        );
        manager
            .update_status_if_current(&recreated, SubscriptionStatusCode::Off)
            .unwrap();
        assert_eq!(
            manager.accept_delivery_outcome(&recreated, false, || panic!(
                "terminal callback must not run"
            )),
            None
        );
        let window = stats.window("t1", "sub-1", now);
        assert_eq!(
            (window.delivered, window.first_try, window.failed),
            (1, 1, 0)
        );
    }

    #[test]
    fn accepted_metrics_and_streak_commit_before_a_concurrent_replacement() {
        use dashmap::try_result::TryResult;
        use std::sync::{Arc, mpsc};
        let manager = Arc::new(manager_with_state(SubscriptionStatusCode::Active, 7, 2));
        let stats = Arc::new(crate::delivery_stats::DeliveryStats::new());
        let now = chrono::Utc::now().timestamp();
        let old = manager.get_snapshot("t1", "sub-1").unwrap();
        let (accepted_tx, accepted_rx) = mpsc::channel();
        let (continue_tx, continue_rx) = mpsc::channel();
        let (registering_tx, registering_rx) = mpsc::channel();
        std::thread::scope(|scope| {
            let worker_manager = Arc::clone(&manager);
            let worker_stats = Arc::clone(&stats);
            let worker_snapshot = old.clone();
            let worker = scope.spawn(move || {
                worker_manager.accept_delivery_outcome(&worker_snapshot, false, || {
                    accepted_tx.send(()).unwrap();
                    continue_rx.recv().unwrap(); // deterministic test-only pause before metrics
                    worker_stats.record_failure("t1", "sub-1", now);
                })
            });
            accepted_rx.recv().unwrap();
            // Check before starting any registrar, so only the outcome callback
            // can account for this lock. Release its test gate even on failure.
            let guard_held = matches!(
                manager
                    .subscriptions
                    .try_get_mut(&("t1".into(), "sub-1".into())),
                TryResult::Locked
            );
            let registering = scope.spawn(|| {
                registering_tx.send(()).unwrap();
                manager
                    .register_snapshot(
                        "t1",
                        "sub-1",
                        &subscription_with_status("active"),
                        FhirVersion::default(),
                        Some("2"),
                    )
                    .unwrap()
            });
            registering_rx.recv().unwrap();
            continue_tx.send(()).unwrap();
            assert!(guard_held, "outcome guard must span the metrics callback");
            assert_eq!(worker.join().unwrap(), Some(3));
            let replacement = registering.join().unwrap();
            assert_eq!(replacement.subscription.consecutive_failures, 3);
            assert_eq!(stats.window("t1", "sub-1", now).failed, 1);
            assert_eq!(
                manager.accept_delivery_outcome(&old, false, || panic!(
                    "now-stale result must not record twice"
                )),
                None
            );
            assert_eq!(
                manager.accept_delivery_outcome(&replacement, true, || stats
                    .record_success("t1", "sub-1", true, now)),
                Some(0)
            );
            let window = stats.window("t1", "sub-1", now);
            assert_eq!((window.delivered, window.failed), (1, 1));
        });
    }

    #[test]
    fn replacement_rejects_late_delivery_mutations_without_resetting_counters() {
        let manager = manager_with_state(SubscriptionStatusCode::Active, 7, 2);
        let old = manager.get_snapshot("t1", "sub-1").unwrap();
        let replacement = manager
            .register_snapshot(
                "t1",
                "sub-1",
                &subscription_with_status("active"),
                FhirVersion::default(),
                Some("2"),
            )
            .unwrap();

        assert!(old.generation.is_cancelled());
        assert!(!manager.is_current(&old));
        assert!(manager.is_current(&replacement));
        assert!(!old.generation.same_generation(&replacement.generation));
        assert_eq!(manager.increment_event_count_if_current(&old), None);
        assert_eq!(manager.record_failure_if_current(&old), None);
        assert!(!manager.reset_failures_if_current(&old));
        assert_eq!(
            manager.update_status_if_current(&old, SubscriptionStatusCode::Off),
            None
        );
        assert!(!manager.is_current_status(&old, SubscriptionStatusCode::Active));
        assert!(manager.source_fence(&old).is_none());

        let current = manager.get_subscription("t1", "sub-1").unwrap();
        assert_eq!(current.events_since_start, 7);
        assert_eq!(current.consecutive_failures, 2);
        assert_eq!(current.status, SubscriptionStatusCode::Active);
        assert_eq!(
            manager.increment_event_count_if_current(&replacement),
            Some(8)
        );

        let rearmed = manager
            .register_snapshot(
                "t1",
                "sub-1",
                &subscription_with_status("requested"),
                FhirVersion::default(),
                Some("3"),
            )
            .unwrap();
        assert!(replacement.generation.is_cancelled());
        assert_eq!(rearmed.subscription.events_since_start, 0);
        assert_eq!(rearmed.subscription.consecutive_failures, 0);
        assert_eq!(manager.increment_event_count_if_current(&rearmed), None);
    }

    #[test]
    fn deletion_invalidates_only_the_deleted_tenant_and_never_reuses_its_token() {
        let manager = manager_with_state(SubscriptionStatusCode::Active, 3, 1);
        let deleted = manager.get_snapshot("t1", "sub-1").unwrap();
        let other = manager
            .register_snapshot(
                "t2",
                "sub-1",
                &subscription_with_status("active"),
                FhirVersion::default(),
                Some("1"),
            )
            .unwrap();
        assert!(manager.deregister("t1", "sub-1"));
        assert!(deleted.generation.is_cancelled());
        assert!(!other.generation.is_cancelled());

        let recreated = manager
            .register_snapshot(
                "t1",
                "sub-1",
                &subscription_with_status("active"),
                FhirVersion::default(),
                Some("1"),
            )
            .unwrap();
        assert!(!deleted.generation.same_generation(&recreated.generation));
        assert_eq!(manager.record_failure_if_current(&deleted), None);
        assert_eq!(manager.record_failure_if_current(&other), Some(1));
        assert_eq!(recreated.subscription.events_since_start, 0);
        assert_eq!(recreated.subscription.consecutive_failures, 0);
        assert_eq!(
            manager
                .active_snapshots_for_topic("t1", &recreated.subscription.topic_url)
                .len(),
            1
        );
        assert_eq!(manager.all_snapshots().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn terminal_cancels_delivery_but_allows_its_status_write_until_replaced() {
        use std::time::Duration;
        let manager = manager_with_state(SubscriptionStatusCode::Active, 1, 1);
        let snapshot = manager.get_snapshot("t1", "sub-1").unwrap();
        let waiter = tokio::spawn({
            let token = snapshot.generation.clone();
            async move { token.cancelled().await }
        });
        tokio::task::yield_now().await;

        assert_eq!(
            manager.update_status_if_current(&snapshot, SubscriptionStatusCode::Error),
            Some(SubscriptionStatusCode::Active)
        );
        assert!(
            !snapshot.generation.is_cancelled(),
            "admitted jobs drain through error"
        );
        assert_eq!(manager.increment_event_count_if_current(&snapshot), None);
        assert_eq!(manager.record_failure_if_current(&snapshot), Some(2));
        assert!(manager.reset_failures_if_current(&snapshot));
        assert!(!manager.is_current_status(&snapshot, SubscriptionStatusCode::Active));
        assert!(manager.is_current_status(&snapshot, SubscriptionStatusCode::Error));

        assert_eq!(
            manager.update_status_if_current(&snapshot, SubscriptionStatusCode::Off),
            Some(SubscriptionStatusCode::Error)
        );
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .unwrap()
            .unwrap();
        // A waiter created after cancellation also observes the retained state.
        tokio::time::timeout(Duration::from_secs(1), snapshot.generation.cancelled())
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(1), snapshot.generation.invalidated())
                .await
                .is_err()
        );
        assert!(
            manager.source_fence(&snapshot).is_some(),
            "off still needs persistence"
        );
        assert_eq!(manager.record_failure_if_current(&snapshot), None);
        assert!(!manager.reset_failures_if_current(&snapshot));

        // Holding an async write lock never blocks synchronous lifecycle.
        let write_lock = snapshot.generation.status_write_lock();
        let _write_guard = write_lock.lock().await;
        manager
            .register(
                "t1",
                "sub-1",
                &subscription_with_status("requested"),
                FhirVersion::default(),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), snapshot.generation.invalidated())
            .await
            .unwrap();
        assert!(manager.source_fence(&snapshot).is_none());
    }

    #[test]
    fn source_fence_preserves_exact_intent_and_advances_only_for_its_own_write() {
        let manager = manager_with_state(SubscriptionStatusCode::Active, 0, 0);
        let mut source = subscription_with_status("active");
        source["meta"] = json!({
            "versionId": "stale-metadata", "lastUpdated": "old",
            "tag": [{"system": "urn:test", "code": "operator-owned"}]
        });
        source["operatorMetadata"] = json!({"unknown": ["preserve", "these", "fields"]});
        let snapshot = manager
            .register_snapshot("t1", "sub-1", &source, FhirVersion::default(), Some("7"))
            .unwrap();
        let original = manager.source_fence(&snapshot).unwrap();
        let mut stored = source.clone();
        stored["meta"]["versionId"] = json!("7");
        stored["meta"]["lastUpdated"] = json!("new-server-time");
        assert!(original.matches("7", &stored));
        assert!(!manager.acknowledge_status_write(&snapshot, &original, "", &stored));
        assert!(
            !original.matches("8", &stored),
            "identical rewritten intent has a different version"
        );
        let mut changed_unknown = stored.clone();
        changed_unknown["operatorMetadata"]["unknown"] = json!(["changed"]);
        assert!(!original.matches("7", &changed_unknown));
        let mut changed_tag = stored.clone();
        changed_tag["meta"]["tag"][0]["code"] = json!("changed");
        assert!(!original.matches("7", &changed_tag));

        manager
            .update_status_if_current(&snapshot, SubscriptionStatusCode::Error)
            .unwrap();
        stored["status"] = json!("error");
        assert!(manager.acknowledge_status_write(&snapshot, &original, "8", &stored));
        let persisted = manager.source_fence(&snapshot).unwrap();
        assert!(persisted.matches("8", &stored));
        assert!(!persisted.matches("7", &stored));
        assert!(
            !manager.acknowledge_status_write(&snapshot, &original, "9", &stored),
            "stale acknowledgements cannot rewind the source"
        );

        let replacement = manager
            .register_snapshot(
                "t1",
                "sub-1",
                &changed_unknown,
                FhirVersion::default(),
                Some("9"),
            )
            .unwrap();
        assert!(!manager.acknowledge_status_write(&snapshot, &persisted, "10", &stored));
        assert!(
            manager
                .source_fence(&replacement)
                .unwrap()
                .matches("9", &changed_unknown)
        );
    }

    #[test]
    fn source_version_uses_metadata_fallback_and_normalizes_only_server_metadata() {
        let resource = subscription_with_status("active");
        let without_version = StatusSourceFence::new(&resource, None);
        assert!(without_version.matches("arbitrary-version", &resource));
        let mut with_meta = resource.clone();
        with_meta["meta"] = json!({"versionId": "4", "lastUpdated": "server-time"});
        assert!(without_version.matches("4", &with_meta));
        let fallback = StatusSourceFence::new(&with_meta, Some(""));
        assert!(fallback.matches("4", &resource));
        assert!(!fallback.matches("5", &resource));
        with_meta["meta"]["profile"] = json!(["urn:operator-profile"]);
        assert!(!fallback.matches("4", &with_meta));
    }

    #[test]
    fn failed_registration_preserves_the_current_generation_and_source() {
        let manager = manager_with_state(SubscriptionStatusCode::Active, 2, 1);
        let snapshot = manager.get_snapshot("t1", "sub-1").unwrap();
        let fence = manager.source_fence(&snapshot).unwrap();
        let mut invalid = subscription_with_status("active");
        // Empty resources are rejected before any registration is replaced.
        invalid.as_object_mut().unwrap().clear();
        assert!(
            manager
                .register("t1", "sub-1", &invalid, FhirVersion::default())
                .is_err()
        );
        assert!(!snapshot.generation.is_cancelled());
        assert!(manager.is_current_status(&snapshot, SubscriptionStatusCode::Active));
        assert_eq!(manager.source_fence(&snapshot).unwrap(), fence);
    }

    #[test]
    fn ordinary_reregistration_does_not_lose_concurrent_counter_increments() {
        let manager = manager_with_state(SubscriptionStatusCode::Active, 0, 0);
        let resource = subscription_with_status("active");
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                barrier.wait();
                for _ in 0..1_000 {
                    manager.increment_event_count("t1", "sub-1").unwrap();
                    manager.record_failure("t1", "sub-1").unwrap();
                }
            });
            scope.spawn(|| {
                barrier.wait();
                for _ in 0..1_000 {
                    manager
                        .register("t1", "sub-1", &resource, FhirVersion::default())
                        .unwrap();
                }
            });
        });
        let subscription = manager.get_subscription("t1", "sub-1").unwrap();
        assert_eq!(subscription.events_since_start, 1_000);
        assert_eq!(subscription.consecutive_failures, 1_000);
    }

    /// `entered-in-error` used to be unparseable, so `register` fell back to
    /// `requested` and rehydration would handshake and activate a subscription
    /// the client had explicitly retracted.
    #[test]
    fn test_entered_in_error_is_registered_as_itself() {
        let manager =
            SubscriptionManager::new(create_test_registry(), vec!["rest-hook".to_string()]);
        let sub = manager
            .register(
                "t1",
                "sub-1",
                &subscription_with_status("entered-in-error"),
                FhirVersion::default(),
            )
            .expect("register");
        assert_eq!(sub.status, SubscriptionStatusCode::EnteredInError);
        assert!(sub.status.is_terminal());
    }
}
