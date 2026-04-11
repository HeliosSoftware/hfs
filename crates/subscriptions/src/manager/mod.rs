//! Subscription manager.
//!
//! Manages the lifecycle of `Subscription` resources: CRUD, status tracking,
//! filter validation, and in-memory indexing of active subscriptions.

pub mod filters;
pub mod status;

use std::sync::Arc;

use dashmap::DashMap;
use helios_fhir::FhirVersion;
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

/// Manages the lifecycle and in-memory indexing of active subscriptions.
pub struct SubscriptionManager {
    /// Active subscriptions keyed by (tenant_id, subscription_id).
    subscriptions: DashMap<(String, String), ActiveSubscription>,

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

        let subscription = ActiveSubscription {
            id: subscription_id.to_string(),
            topic_url,
            status,
            channel,
            filters: parsed_filters,
            fhir_version,
            events_since_start: 0,
            consecutive_failures: 0,
            tenant_id: tenant_id.to_string(),
        };

        debug!(
            tenant = tenant_id,
            subscription_id,
            topic = %subscription.topic_url,
            "Registered subscription"
        );

        let key = (tenant_id.to_string(), subscription_id.to_string());
        self.subscriptions.insert(key, subscription.clone());

        Ok(subscription)
    }

    /// Removes a subscription from the in-memory index.
    pub fn deregister(&self, tenant_id: &str, subscription_id: &str) -> bool {
        let key = (tenant_id.to_string(), subscription_id.to_string());
        let removed = self.subscriptions.remove(&key).is_some();
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
                let sub = entry.value();
                sub.tenant_id == tenant_id
                    && sub.topic_url == topic_url
                    && sub.status == SubscriptionStatusCode::Active
            })
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Gets the current status of a subscription.
    pub fn get_subscription(
        &self,
        tenant_id: &str,
        subscription_id: &str,
    ) -> Option<ActiveSubscription> {
        let key = (tenant_id.to_string(), subscription_id.to_string());
        self.subscriptions.get(&key).map(|entry| entry.clone())
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

        let old_status = entry.status;
        if !old_status.can_transition_to(new_status) {
            return Err(SubscriptionError::InvalidStatusTransition {
                from: old_status.to_string(),
                to: new_status.to_string(),
            });
        }

        entry.status = new_status;
        debug!(
            tenant = tenant_id,
            subscription_id,
            from = %old_status,
            to = %new_status,
            "Status transition"
        );

        Ok(old_status)
    }

    /// Increments the event counter and returns the new value.
    pub fn increment_event_count(&self, tenant_id: &str, subscription_id: &str) -> Option<u64> {
        let key = (tenant_id.to_string(), subscription_id.to_string());
        let mut entry = self.subscriptions.get_mut(&key)?;
        entry.events_since_start += 1;
        Some(entry.events_since_start)
    }

    /// Records a delivery failure and returns the new consecutive failure count.
    pub fn record_failure(&self, tenant_id: &str, subscription_id: &str) -> Option<u32> {
        let key = (tenant_id.to_string(), subscription_id.to_string());
        let mut entry = self.subscriptions.get_mut(&key)?;
        entry.consecutive_failures += 1;
        let count = entry.consecutive_failures;
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
            entry.consecutive_failures = 0;
        }
    }

    /// Returns all subscriptions (for heartbeat checking, initialization, etc.).
    pub fn all_subscriptions(&self) -> Vec<ActiveSubscription> {
        self.subscriptions
            .iter()
            .map(|e| e.value().clone())
            .collect()
    }
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
            let payload_content = find_extension_value_code(
                resource,
                "http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-payload-content",
            )
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

                    let filter_str = if let Some(rt) = resource_type {
                        format!("{rt}?{param}={value}")
                    } else {
                        format!("{param}={value}")
                    };

                    Some(filter_str)
                })
                .collect()
        }
    }
}

// --- Extension helpers for R4 backport ---

/// Find a single extension with the given URL and return its `valueUrl`.
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

/// Find a single extension with the given URL and return its `valueUnsignedInt`.
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
        let resource = json!({
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
        });
        #[cfg(not(feature = "R4"))]
        let resource = json!({
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
        });

        let sub = manager
            .register("t1", "sub-1", &resource, FhirVersion::default())
            .unwrap();

        assert_eq!(sub.filters.len(), 1);
        assert_eq!(sub.filters[0].filter_parameter, "patient");
        assert_eq!(sub.filters[0].value, "Patient/123");
        assert_eq!(sub.channel.payload_content, PayloadContent::FullResource);
    }

    #[test]
    fn test_register_with_invalid_filter() {
        let registry = create_test_registry();
        let manager = SubscriptionManager::new(registry, vec!["rest-hook".to_string()]);

        #[cfg(feature = "R4")]
        let resource = json!({
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
        });
        #[cfg(not(feature = "R4"))]
        let resource = json!({
            "resourceType": "Subscription",
            "status": "requested",
            "topic": "http://example.org/topic/encounter-start",
            "channelType": { "code": "rest-hook" },
            "endpoint": "https://example.com/webhook",
            "filterBy": [{
                "filterParameter": "unknown-param",
                "value": "some-value"
            }]
        });

        let result = manager.register("t1", "sub-1", &resource, FhirVersion::default());
        assert!(matches!(
            result,
            Err(SubscriptionError::InvalidFilter { .. })
        ));
    }
}
