//! Notification builder.
//!
//! Constructs FHIR notification bundles for subscription events. This is the
//! primary point of version-specific divergence:
//!
//! - **R4**: Bundle(type=history) with SubscriptionStatus as Parameters (backport IG)
//! - **R4B**: Bundle(type=history) with native SubscriptionStatus
//! - **R5**: Bundle(type=subscription-notification) with native SubscriptionStatus
//! - **R6**: Follows R5 model with refinements

mod builder;

use chrono::{DateTime, Utc};
use helios_fhir::FhirVersion;

use crate::error::SubscriptionError;
use crate::manager::ActiveSubscription;

pub use builder::NotificationBundleBuilder;

/// The type of notification being sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationType {
    /// Initial connection verification.
    Handshake,
    /// Periodic keep-alive.
    Heartbeat,
    /// A resource event matched the subscription.
    EventNotification,
}

impl NotificationType {
    pub fn as_fhir_str(&self) -> &'static str {
        match self {
            Self::Handshake => "handshake",
            Self::Heartbeat => "heartbeat",
            Self::EventNotification => "event-notification",
        }
    }
}

/// Metadata about a notification event.
#[derive(Debug, Clone)]
pub struct NotificationEventData {
    /// Monotonically increasing event number.
    pub event_number: u64,
    /// When the event occurred.
    pub timestamp: DateTime<Utc>,
    /// Reference to the focus resource (e.g., "Encounter/123").
    pub focus_reference: String,
}

/// Builds a notification bundle for a subscription.
///
/// Dispatches to the appropriate version-specific builder based on the
/// subscription's FHIR version.
pub fn build_notification(
    subscription: &ActiveSubscription,
    notification_type: NotificationType,
    events: &[NotificationEventData],
    resources: &[serde_json::Value],
    base_url: &str,
) -> Result<serde_json::Value, SubscriptionError> {
    let builder = NotificationBundleBuilder::new(subscription, notification_type, base_url);

    match subscription.fhir_version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => builder.build_r4(events, resources),

        // R4B, R5, R6 all use native SubscriptionStatus with minor differences.
        #[allow(unreachable_patterns)]
        _ => builder.build_native(events, resources),
    }
}

/// Builds a handshake notification for a newly activated subscription.
pub fn build_handshake(
    subscription: &ActiveSubscription,
    base_url: &str,
) -> Result<serde_json::Value, SubscriptionError> {
    build_notification(
        subscription,
        NotificationType::Handshake,
        &[],
        &[],
        base_url,
    )
}

/// Builds a heartbeat notification.
pub fn build_heartbeat(
    subscription: &ActiveSubscription,
    base_url: &str,
) -> Result<serde_json::Value, SubscriptionError> {
    build_notification(
        subscription,
        NotificationType::Heartbeat,
        &[],
        &[],
        base_url,
    )
}

/// Builds an event notification for a matched resource event.
pub fn build_event_notification(
    subscription: &ActiveSubscription,
    event_data: NotificationEventData,
    resource: Option<&serde_json::Value>,
    base_url: &str,
) -> Result<serde_json::Value, SubscriptionError> {
    let events = vec![event_data];
    let resources: Vec<serde_json::Value> = match resource {
        Some(r) => vec![r.clone()],
        None => vec![],
    };
    build_notification(
        subscription,
        NotificationType::EventNotification,
        &events,
        &resources,
        base_url,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::{ChannelConfig, ChannelType, PayloadContent, SubscriptionStatusCode};
    use chrono::Utc;
    use serde_json::json;

    fn test_subscription(payload: PayloadContent) -> ActiveSubscription {
        ActiveSubscription {
            id: "sub-1".to_string(),
            topic_url: "http://example.org/topic/encounter-start".to_string(),
            status: SubscriptionStatusCode::Active,
            channel: ChannelConfig {
                channel_type: ChannelType::RestHook,
                endpoint: Some("https://example.com/webhook".to_string()),
                payload_mime_type: Some("application/fhir+json".to_string()),
                payload_content: payload,
                headers: vec![],
                heartbeat_period: None,
                timeout: None,
                max_count: None,
            },
            filters: vec![],
            fhir_version: FhirVersion::default(),
            events_since_start: 5,
            consecutive_failures: 0,
            tenant_id: "test-tenant".to_string(),
        }
    }

    #[test]
    fn test_handshake_notification() {
        let sub = test_subscription(PayloadContent::IdOnly);
        let bundle = build_handshake(&sub, "http://localhost:8080").unwrap();

        assert_eq!(bundle["resourceType"], "Bundle");
        // Should have exactly one entry (the status).
        let entries = bundle["entry"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_heartbeat_notification() {
        let sub = test_subscription(PayloadContent::Empty);
        let bundle = build_heartbeat(&sub, "http://localhost:8080").unwrap();

        assert_eq!(bundle["resourceType"], "Bundle");
        let entries = bundle["entry"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_event_notification_empty_payload() {
        let sub = test_subscription(PayloadContent::Empty);
        let event = NotificationEventData {
            event_number: 6,
            timestamp: Utc::now(),
            focus_reference: "Encounter/123".to_string(),
        };

        let bundle = build_event_notification(&sub, event, None, "http://localhost:8080").unwrap();

        let entries = bundle["entry"].as_array().unwrap();
        // Only the status entry, no resource entries for empty payload.
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_event_notification_id_only_payload() {
        let sub = test_subscription(PayloadContent::IdOnly);
        let event = NotificationEventData {
            event_number: 6,
            timestamp: Utc::now(),
            focus_reference: "Encounter/123".to_string(),
        };

        let resource = json!({"resourceType": "Encounter", "id": "123"});
        let bundle =
            build_event_notification(&sub, event, Some(&resource), "http://localhost:8080")
                .unwrap();

        let entries = bundle["entry"].as_array().unwrap();
        // Status entry + id-only entry.
        assert_eq!(entries.len(), 2);

        // The id-only entry should have fullUrl and request but no resource.
        let id_entry = &entries[1];
        assert!(id_entry.get("fullUrl").is_some());
        assert!(id_entry.get("request").is_some());
        assert!(id_entry.get("resource").is_none());
    }

    #[test]
    fn test_event_notification_full_resource_payload() {
        let sub = test_subscription(PayloadContent::FullResource);
        let event = NotificationEventData {
            event_number: 6,
            timestamp: Utc::now(),
            focus_reference: "Encounter/123".to_string(),
        };

        let resource = json!({"resourceType": "Encounter", "id": "123", "status": "in-progress"});
        let bundle =
            build_event_notification(&sub, event, Some(&resource), "http://localhost:8080")
                .unwrap();

        let entries = bundle["entry"].as_array().unwrap();
        // Status entry + full resource entry.
        assert_eq!(entries.len(), 2);

        // The full resource entry should contain the resource.
        let resource_entry = &entries[1];
        assert!(resource_entry.get("fullUrl").is_some());
        assert!(resource_entry.get("resource").is_some());
        assert_eq!(resource_entry["resource"]["id"], "123");
    }

    #[cfg(feature = "R4")]
    #[test]
    fn test_r4_notification_uses_parameters() {
        let mut sub = test_subscription(PayloadContent::IdOnly);
        sub.fhir_version = FhirVersion::R4;

        let bundle = build_handshake(&sub, "http://localhost:8080").unwrap();

        // R4: Bundle type is "history".
        assert_eq!(bundle["type"], "history");

        // First entry should be a Parameters resource.
        let status_entry = &bundle["entry"].as_array().unwrap()[0];
        let status_resource = &status_entry["resource"];
        assert_eq!(status_resource["resourceType"], "Parameters");
    }

    #[test]
    fn test_native_notification_bundle_type() {
        let sub = test_subscription(PayloadContent::IdOnly);
        let bundle = build_handshake(&sub, "http://localhost:8080").unwrap();

        // For the default version (R4 with backport), type is "history".
        // For R5+, type would be "subscription-notification".
        #[cfg(feature = "R4")]
        assert_eq!(bundle["type"], "history");
    }
}
