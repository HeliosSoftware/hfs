//! FHIR Messaging notification wrapping.
//!
//! When a subscription's channel type is `message`, the dispatcher delivers
//! the notification wrapped in a FHIR message bundle: a `Bundle.type =
//! "message"` whose first entry is a `MessageHeader` resource. The remaining
//! entries are the same `SubscriptionStatus`/`Parameters` and payload
//! entries that the rest-hook channel would deliver.
//!
//! Reference: <https://hl7.org/fhir/R5/messaging.html>

use chrono::Utc;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::error::SubscriptionError;
use crate::manager::ActiveSubscription;
use crate::notification::NotificationType;

#[cfg(feature = "R4")]
use helios_fhir::FhirVersion;

/// Wrap an existing notification bundle as a FHIR message bundle.
///
/// Produces a fresh `Bundle.type = "message"` whose first entry is a
/// [`MessageHeader`](https://hl7.org/fhir/R5/messageheader.html) resource
/// describing the source/destination/event, followed by the entries of the
/// original notification bundle (SubscriptionStatus or backport Parameters,
/// plus any payload entries) in their original order.
///
/// The MessageHeader carries the SubscriptionTopic canonical URL as the
/// event identifier:
/// - R4: `eventUri = topic_url`
/// - R4B+: `eventCoding.system = topic_url`,
///   `eventCoding.code = handshake | heartbeat | event-notification`
///
/// On R4B+, `definition` is also set to the topic canonical URL for explicit
/// topic identification (independent of the eventCoding code, which conveys
/// the notification kind).
pub fn wrap_as_message_bundle(
    notification_bundle: &Value,
    subscription: &ActiveSubscription,
    source_endpoint: &str,
    notification_type: NotificationType,
) -> Result<Value, SubscriptionError> {
    let inner_entries = notification_bundle
        .get("entry")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let header = build_message_header(subscription, source_endpoint, notification_type);

    let mut entries = Vec::with_capacity(inner_entries.len() + 1);
    entries.push(json!({
        "fullUrl": format!("urn:uuid:{}", Uuid::new_v4()),
        "resource": header,
    }));
    entries.extend(inner_entries);

    Ok(json!({
        "resourceType": "Bundle",
        "id": Uuid::new_v4().to_string(),
        "type": "message",
        "timestamp": Utc::now().to_rfc3339(),
        "entry": entries,
    }))
}

/// Construct the MessageHeader resource for a subscription notification.
fn build_message_header(
    subscription: &ActiveSubscription,
    source_endpoint: &str,
    notification_type: NotificationType,
) -> Value {
    let destination_endpoint = subscription.channel.endpoint.clone().unwrap_or_default();

    let focus_reference = format!("Subscription/{}", subscription.id);

    let mut header = json!({
        "resourceType": "MessageHeader",
        "id": Uuid::new_v4().to_string(),
        "source": {
            "name": "Helios FHIR Server",
            "endpoint": source_endpoint,
        },
        "destination": [
            {
                "endpoint": destination_endpoint,
            }
        ],
        "focus": [
            {
                "reference": focus_reference,
            }
        ],
    });

    // Event identifier — R4 uses `eventUri`, R4B+ uses `eventCoding`.
    let use_event_uri = is_r4(subscription);
    if use_event_uri {
        header["eventUri"] = json!(subscription.topic_url);
    } else {
        header["eventCoding"] = json!({
            "system": subscription.topic_url,
            "code": notification_type.as_fhir_str(),
            "display": notification_type_display(notification_type),
        });
        // `definition` is not present on the R4 MessageHeader element. On
        // R4B+ it carries the canonical URL of the message definition; we
        // re-use the SubscriptionTopic canonical URL so receivers can
        // unambiguously identify the topic alongside the event coding.
        header["definition"] = json!(subscription.topic_url);
    }

    header
}

#[cfg(feature = "R4")]
fn is_r4(subscription: &ActiveSubscription) -> bool {
    matches!(subscription.fhir_version, FhirVersion::R4)
}

#[cfg(not(feature = "R4"))]
fn is_r4(_subscription: &ActiveSubscription) -> bool {
    false
}

fn notification_type_display(notification_type: NotificationType) -> &'static str {
    match notification_type {
        NotificationType::Handshake => "Handshake notification",
        NotificationType::Heartbeat => "Heartbeat notification",
        NotificationType::EventNotification => "Event notification",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::{
        ActiveSubscription, ChannelConfig, ChannelType, PayloadContent, SubscriptionStatusCode,
    };
    use crate::notification::{NotificationEventData, build_event_notification, build_handshake};
    use chrono::Utc;
    use helios_fhir::FhirVersion;

    const TOPIC_URL: &str = "http://example.org/topic/encounter-start";
    const SOURCE_ENDPOINT: &str = "https://hfs.example.org/fhir";
    const DESTINATION_ENDPOINT: &str = "https://receiver.example.org/process-message";

    fn test_subscription(payload: PayloadContent) -> ActiveSubscription {
        ActiveSubscription {
            id: "sub-msg-1".to_string(),
            topic_url: TOPIC_URL.to_string(),
            status: SubscriptionStatusCode::Active,
            channel: ChannelConfig {
                channel_type: ChannelType::Message,
                endpoint: Some(DESTINATION_ENDPOINT.to_string()),
                payload_mime_type: Some("application/fhir+json".to_string()),
                payload_content: payload,
                headers: vec![],
                heartbeat_period: None,
                timeout: None,
                max_count: None,
            },
            filters: vec![],
            fhir_version: FhirVersion::default(),
            events_since_start: 7,
            consecutive_failures: 0,
            tenant_id: "test-tenant".to_string(),
        }
    }

    #[test]
    fn handshake_message_bundle_has_message_type() {
        let sub = test_subscription(PayloadContent::IdOnly);
        let inner = build_handshake(&sub, SOURCE_ENDPOINT).unwrap();
        let bundle =
            wrap_as_message_bundle(&inner, &sub, SOURCE_ENDPOINT, NotificationType::Handshake)
                .unwrap();

        assert_eq!(bundle["resourceType"], "Bundle");
        assert_eq!(bundle["type"], "message");
    }

    #[test]
    fn first_entry_is_message_header() {
        let sub = test_subscription(PayloadContent::IdOnly);
        let inner = build_handshake(&sub, SOURCE_ENDPOINT).unwrap();
        let bundle =
            wrap_as_message_bundle(&inner, &sub, SOURCE_ENDPOINT, NotificationType::Handshake)
                .unwrap();

        let entries = bundle["entry"].as_array().unwrap();
        assert!(!entries.is_empty());
        assert_eq!(entries[0]["resource"]["resourceType"], "MessageHeader");
    }

    #[test]
    fn message_header_source_destination_focus() {
        let sub = test_subscription(PayloadContent::IdOnly);
        let inner = build_handshake(&sub, SOURCE_ENDPOINT).unwrap();
        let bundle =
            wrap_as_message_bundle(&inner, &sub, SOURCE_ENDPOINT, NotificationType::Handshake)
                .unwrap();

        let header = &bundle["entry"][0]["resource"];
        assert_eq!(header["source"]["endpoint"], SOURCE_ENDPOINT);
        assert_eq!(header["destination"][0]["endpoint"], DESTINATION_ENDPOINT);
        assert_eq!(
            header["focus"][0]["reference"],
            format!("Subscription/{}", sub.id)
        );
    }

    #[test]
    fn payload_entries_follow_message_header_in_order() {
        let sub = test_subscription(PayloadContent::IdOnly);
        let event = NotificationEventData {
            event_number: 8,
            timestamp: Utc::now(),
            focus_reference: "Encounter/abc".to_string(),
        };
        let resource = serde_json::json!({"resourceType": "Encounter", "id": "abc"});
        let inner =
            build_event_notification(&sub, event, Some(&resource), SOURCE_ENDPOINT).unwrap();
        let inner_entries_len = inner["entry"].as_array().unwrap().len();

        let bundle = wrap_as_message_bundle(
            &inner,
            &sub,
            SOURCE_ENDPOINT,
            NotificationType::EventNotification,
        )
        .unwrap();

        let entries = bundle["entry"].as_array().unwrap();
        assert_eq!(entries.len(), inner_entries_len + 1);
        // After MessageHeader, entries match inner bundle's entries.
        for (i, inner_entry) in inner["entry"].as_array().unwrap().iter().enumerate() {
            assert_eq!(entries[i + 1], *inner_entry);
        }
    }

    #[cfg(feature = "R4")]
    #[test]
    fn r4_uses_event_uri() {
        let mut sub = test_subscription(PayloadContent::IdOnly);
        sub.fhir_version = FhirVersion::R4;
        let inner = build_handshake(&sub, SOURCE_ENDPOINT).unwrap();
        let bundle =
            wrap_as_message_bundle(&inner, &sub, SOURCE_ENDPOINT, NotificationType::Handshake)
                .unwrap();

        let header = &bundle["entry"][0]["resource"];
        assert_eq!(header["eventUri"], TOPIC_URL);
        assert!(header.get("eventCoding").is_none());
        assert!(header.get("definition").is_none());
    }

    #[cfg(any(feature = "R4B", feature = "R5", feature = "R6"))]
    #[test]
    fn r4b_plus_uses_event_coding_and_definition() {
        let mut sub = test_subscription(PayloadContent::IdOnly);
        #[cfg(feature = "R4B")]
        {
            sub.fhir_version = FhirVersion::R4B;
        }
        #[cfg(all(feature = "R5", not(feature = "R4B")))]
        {
            sub.fhir_version = FhirVersion::R5;
        }
        #[cfg(all(feature = "R6", not(feature = "R4B"), not(feature = "R5")))]
        {
            sub.fhir_version = FhirVersion::R6;
        }
        let inner = build_handshake(&sub, SOURCE_ENDPOINT).unwrap();
        let bundle =
            wrap_as_message_bundle(&inner, &sub, SOURCE_ENDPOINT, NotificationType::Handshake)
                .unwrap();

        let header = &bundle["entry"][0]["resource"];
        assert!(header.get("eventUri").is_none());
        assert_eq!(header["eventCoding"]["system"], TOPIC_URL);
        assert_eq!(header["eventCoding"]["code"], "handshake");
        assert_eq!(header["definition"], TOPIC_URL);
    }

    #[cfg(any(feature = "R4B", feature = "R5", feature = "R6"))]
    #[test]
    fn event_coding_code_reflects_notification_type() {
        let mut sub = test_subscription(PayloadContent::Empty);
        #[cfg(feature = "R4B")]
        {
            sub.fhir_version = FhirVersion::R4B;
        }
        #[cfg(all(feature = "R5", not(feature = "R4B")))]
        {
            sub.fhir_version = FhirVersion::R5;
        }
        #[cfg(all(feature = "R6", not(feature = "R4B"), not(feature = "R5")))]
        {
            sub.fhir_version = FhirVersion::R6;
        }

        for (nt, code) in [
            (NotificationType::Handshake, "handshake"),
            (NotificationType::Heartbeat, "heartbeat"),
            (NotificationType::EventNotification, "event-notification"),
        ] {
            let inner = build_handshake(&sub, SOURCE_ENDPOINT).unwrap();
            let bundle = wrap_as_message_bundle(&inner, &sub, SOURCE_ENDPOINT, nt).unwrap();
            assert_eq!(bundle["entry"][0]["resource"]["eventCoding"]["code"], code);
        }
    }

    #[test]
    fn missing_destination_endpoint_yields_empty_string() {
        let mut sub = test_subscription(PayloadContent::IdOnly);
        sub.channel.endpoint = None;
        let inner = build_handshake(&sub, SOURCE_ENDPOINT).unwrap();
        let bundle =
            wrap_as_message_bundle(&inner, &sub, SOURCE_ENDPOINT, NotificationType::Handshake)
                .unwrap();

        assert_eq!(
            bundle["entry"][0]["resource"]["destination"][0]["endpoint"],
            ""
        );
    }
}
