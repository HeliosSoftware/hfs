//! Notification bundle builder.
//!
//! Shared logic for constructing notification bundles across FHIR versions.

use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::error::SubscriptionError;
use crate::manager::{ActiveSubscription, EventQuery, PayloadContent, SubscriptionStatusCode};
use crate::notification::{NotificationEventData, NotificationType};

/// The fields of a status resource, independent of where they come from
/// (a live subscription or a storage fallback).
struct StatusView<'a> {
    subscription_ref: String,
    topic_url: &'a str,
    status: SubscriptionStatusCode,
    notification_type: NotificationType,
    events_since_start: u64,
    /// Emit counters as strings in the native status (R4B `string`, R5/R6
    /// `integer64`); push notifications keep their historical JSON numbers.
    string_counters: bool,
}

/// Bundle type for a FHIR version.
fn bundle_type_for(version: helios_fhir::FhirVersion) -> &'static str {
    match version {
        #[cfg(feature = "R4")]
        helios_fhir::FhirVersion::R4 => "history",

        #[cfg(feature = "R4B")]
        helios_fhir::FhirVersion::R4B => "history",

        // R5, R6 use the new bundle type.
        #[allow(unreachable_patterns)]
        _ => "subscription-notification",
    }
}

/// Id-only payload entries: `fullUrl` and a `GET` to the focus resource.
fn push_id_only_entries(
    entries: &mut Vec<serde_json::Value>,
    events: &[NotificationEventData],
    base_url: &str,
) {
    for event in events {
        entries.push(json!({
            "fullUrl": format!("{}/{}", base_url, event.focus_reference),
            "request": { "method": "GET", "url": &event.focus_reference },
            "response": { "status": "200" }
        }));
    }
}

/// R4 backport status as a `Parameters` resource.
#[cfg(feature = "R4")]
fn r4_status_parameters(
    view: &StatusView<'_>,
    events: &[NotificationEventData],
) -> serde_json::Value {
    let mut parameters = vec![
        json!({ "name": "subscription", "valueReference": { "reference": view.subscription_ref } }),
        json!({ "name": "topic", "valueCanonical": view.topic_url }),
        json!({ "name": "status", "valueCode": view.status.as_fhir_str() }),
        json!({ "name": "type", "valueCode": view.notification_type.as_fhir_str() }),
        json!({
            "name": "events-since-subscription-start",
            "valueString": view.events_since_start.to_string()
        }),
    ];
    for event in events {
        parameters.push(json!({
            "name": "notification-event",
            "part": [
                { "name": "event-number", "valueString": event.event_number.to_string() },
                { "name": "timestamp", "valueInstant": event.timestamp.to_rfc3339() },
                { "name": "focus", "valueReference": { "reference": &event.focus_reference } }
            ]
        }));
    }
    json!({ "resourceType": "Parameters", "parameter": parameters })
}

/// Native `SubscriptionStatus` resource (R4B, R5, R6).
fn native_status(view: &StatusView<'_>, events: &[NotificationEventData]) -> serde_json::Value {
    let counter = |n: u64| {
        if view.string_counters {
            json!(n.to_string())
        } else {
            json!(n)
        }
    };
    let notification_events: Vec<_> = events
        .iter()
        .map(|event| {
            json!({
                "eventNumber": counter(event.event_number),
                "timestamp": event.timestamp.to_rfc3339(),
                "focus": { "reference": &event.focus_reference }
            })
        })
        .collect();

    let mut status = json!({
        "resourceType": "SubscriptionStatus",
        "status": view.status.as_fhir_str(),
        "type": view.notification_type.as_fhir_str(),
        "eventsSinceSubscriptionStart": counter(view.events_since_start),
        "subscription": { "reference": view.subscription_ref },
        "topic": view.topic_url
    });
    if !notification_events.is_empty() {
        status["notificationEvent"] = json!(notification_events);
    }
    status
}

/// Builds the `$events` response bundle (see [`crate::notification::build_query_events_bundle`]).
pub(super) fn build_query_events_bundle(
    query: &EventQuery,
    subscription_id: &str,
    subscription_url: &str,
    base_url: &str,
) -> Result<serde_json::Value, SubscriptionError> {
    let view = StatusView {
        subscription_ref: subscription_url.to_string(),
        topic_url: &query.topic_url,
        status: query.status,
        notification_type: NotificationType::QueryEvent,
        events_since_start: query.events_since_start,
        string_counters: true,
    };
    let status = match query.fhir_version {
        #[cfg(feature = "R4")]
        helios_fhir::FhirVersion::R4 => r4_status_parameters(&view, &query.events),
        #[allow(unreachable_patterns)]
        _ => native_status(&view, &query.events),
    };
    let mut entries = vec![json!({
        "fullUrl": format!("urn:uuid:{}", Uuid::new_v4()),
        "resource": status,
        "request": { "method": "GET", "url": format!("Subscription/{subscription_id}/$status") },
        "response": { "status": "200" }
    })];
    push_id_only_entries(&mut entries, &query.events, base_url);
    Ok(json!({
        "resourceType": "Bundle",
        "id": Uuid::new_v4().to_string(),
        "type": bundle_type_for(query.fhir_version),
        "timestamp": Utc::now().to_rfc3339(),
        "entry": entries
    }))
}

/// Builds notification bundles for a subscription.
pub struct NotificationBundleBuilder<'a> {
    subscription: &'a ActiveSubscription,
    notification_type: NotificationType,
    base_url: &'a str,
}

impl<'a> NotificationBundleBuilder<'a> {
    pub fn new(
        subscription: &'a ActiveSubscription,
        notification_type: NotificationType,
        base_url: &'a str,
    ) -> Self {
        Self {
            subscription,
            notification_type,
            base_url,
        }
    }

    /// Build an R4 backport notification bundle.
    ///
    /// - Bundle type: `history`
    /// - First entry: `Parameters` resource (backport SubscriptionStatus)
    #[cfg(feature = "R4")]
    pub fn build_r4(
        &self,
        events: &[NotificationEventData],
        resources: &[serde_json::Value],
    ) -> Result<serde_json::Value, SubscriptionError> {
        let mut entries = Vec::new();

        // Build the SubscriptionStatus as a Parameters resource.
        let status_parameters = self.build_r4_status_parameters(events);
        entries.push(json!({
            "fullUrl": format!("urn:uuid:{}", Uuid::new_v4()),
            "resource": status_parameters,
            "request": {
                "method": "GET",
                "url": format!("Subscription/{}/$status", self.subscription.id)
            },
            "response": {
                "status": "200"
            }
        }));

        // Add payload entries.
        self.add_payload_entries(&mut entries, events, resources);

        Ok(json!({
            "resourceType": "Bundle",
            "id": Uuid::new_v4().to_string(),
            "type": "history",
            "timestamp": Utc::now().to_rfc3339(),
            "entry": entries
        }))
    }

    fn status_view(&self) -> StatusView<'a> {
        StatusView {
            subscription_ref: format!("Subscription/{}", self.subscription.id),
            topic_url: &self.subscription.topic_url,
            status: self.subscription.status,
            notification_type: self.notification_type,
            events_since_start: self.subscription.events_since_start,
            string_counters: false,
        }
    }

    /// Build an R4 backport SubscriptionStatus as a Parameters resource.
    #[cfg(feature = "R4")]
    fn build_r4_status_parameters(&self, events: &[NotificationEventData]) -> serde_json::Value {
        r4_status_parameters(&self.status_view(), events)
    }

    /// Build a native notification bundle (R4B, R5, R6).
    ///
    /// - R4B: Bundle type `history`
    /// - R5/R6: Bundle type `subscription-notification`
    /// - First entry: native `SubscriptionStatus` resource
    pub fn build_native(
        &self,
        events: &[NotificationEventData],
        resources: &[serde_json::Value],
    ) -> Result<serde_json::Value, SubscriptionError> {
        let mut entries = Vec::new();

        // Build the native SubscriptionStatus resource.
        let status = self.build_native_status(events);
        entries.push(json!({
            "fullUrl": format!("urn:uuid:{}", Uuid::new_v4()),
            "resource": status,
            "request": {
                "method": "GET",
                "url": format!("Subscription/{}/$status", self.subscription.id)
            },
            "response": {
                "status": "200"
            }
        }));

        // Add payload entries.
        self.add_payload_entries(&mut entries, events, resources);

        let bundle_type = self.bundle_type();

        Ok(json!({
            "resourceType": "Bundle",
            "id": Uuid::new_v4().to_string(),
            "type": bundle_type,
            "timestamp": Utc::now().to_rfc3339(),
            "entry": entries
        }))
    }

    /// Build a native SubscriptionStatus resource.
    fn build_native_status(&self, events: &[NotificationEventData]) -> serde_json::Value {
        native_status(&self.status_view(), events)
    }

    /// Add payload entries based on the subscription's payload content level.
    fn add_payload_entries(
        &self,
        entries: &mut Vec<serde_json::Value>,
        events: &[NotificationEventData],
        resources: &[serde_json::Value],
    ) {
        match self.subscription.channel.payload_content {
            PayloadContent::Empty => {
                // No additional entries.
            }
            PayloadContent::IdOnly => {
                push_id_only_entries(entries, events, self.base_url);
            }
            PayloadContent::FullResource => {
                for (i, event) in events.iter().enumerate() {
                    let mut entry = json!({
                        "fullUrl": format!("{}/{}", self.base_url, event.focus_reference),
                        "request": {
                            "method": "GET",
                            "url": &event.focus_reference
                        },
                        "response": {
                            "status": "200"
                        }
                    });

                    if let Some(resource) = resources.get(i) {
                        entry["resource"] = resource.clone();
                    }

                    entries.push(entry);
                }
            }
        }
    }

    /// Returns the Bundle type based on FHIR version.
    fn bundle_type(&self) -> &'static str {
        bundle_type_for(self.subscription.fhir_version)
    }
}
