//! Notification bundle builder.
//!
//! Shared logic for constructing notification bundles across FHIR versions.

use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::error::SubscriptionError;
use crate::manager::{ActiveSubscription, PayloadContent};
use crate::notification::{NotificationEventData, NotificationType};

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

    /// Build an R4 backport SubscriptionStatus as a Parameters resource.
    #[cfg(feature = "R4")]
    fn build_r4_status_parameters(&self, events: &[NotificationEventData]) -> serde_json::Value {
        let mut parameters = vec![
            json!({
                "name": "subscription",
                "valueReference": {
                    "reference": format!("Subscription/{}", self.subscription.id)
                }
            }),
            json!({
                "name": "topic",
                "valueCanonical": self.subscription.topic_url
            }),
            json!({
                "name": "status",
                "valueCode": self.subscription.status.as_fhir_str()
            }),
            json!({
                "name": "type",
                "valueCode": self.notification_type.as_fhir_str()
            }),
            json!({
                "name": "events-since-subscription-start",
                "valueString": self.subscription.events_since_start.to_string()
            }),
        ];

        // Add notification-event parts for event notifications.
        for event in events {
            let event_parts = vec![
                json!({
                    "name": "event-number",
                    "valueString": event.event_number.to_string()
                }),
                json!({
                    "name": "timestamp",
                    "valueInstant": event.timestamp.to_rfc3339()
                }),
                json!({
                    "name": "focus",
                    "valueReference": {
                        "reference": &event.focus_reference
                    }
                }),
            ];

            parameters.push(json!({
                "name": "notification-event",
                "part": event_parts
            }));
        }

        json!({
            "resourceType": "Parameters",
            "parameter": parameters
        })
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
        let mut notification_events = Vec::new();
        for event in events {
            notification_events.push(json!({
                "eventNumber": event.event_number,
                "timestamp": event.timestamp.to_rfc3339(),
                "focus": {
                    "reference": &event.focus_reference
                }
            }));
        }

        let mut status = json!({
            "resourceType": "SubscriptionStatus",
            "status": self.subscription.status.as_fhir_str(),
            "type": self.notification_type.as_fhir_str(),
            "eventsSinceSubscriptionStart": self.subscription.events_since_start,
            "subscription": {
                "reference": format!("Subscription/{}", self.subscription.id)
            },
            "topic": self.subscription.topic_url
        });

        if !notification_events.is_empty() {
            status["notificationEvent"] = json!(notification_events);
        }

        status
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
                for event in events {
                    entries.push(json!({
                        "fullUrl": format!("{}/{}", self.base_url, event.focus_reference),
                        "request": {
                            "method": "GET",
                            "url": &event.focus_reference
                        },
                        "response": {
                            "status": "200"
                        }
                    }));
                }
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
        match self.subscription.fhir_version {
            #[cfg(feature = "R4")]
            helios_fhir::FhirVersion::R4 => "history",

            #[cfg(feature = "R4B")]
            helios_fhir::FhirVersion::R4B => "history",

            // R5, R6 use the new bundle type.
            #[allow(unreachable_patterns)]
            _ => "subscription-notification",
        }
    }
}
