//! The authority of a Subscription or topic writer over the resource types
//! the topic names.
//!
//! A Subscription delivers, to the endpoint its writer chose, the content or
//! the identifiers of every later write of the resource types its topic
//! triggers on, and a topic decides which types those are. The authorization
//! middleware judges such a write as Create or Update on `Subscription`,
//! `SubscriptionTopic` or `Basic` alone, and the engine registers the
//! committed document without a principal. Every other indirect read path
//! (chained search, SQL-on-FHIR, `$everything`, `$export`) requires the
//! Search grant a direct search on each type would; [`admit_write`] applies
//! the same bound where the principal is known, at the write boundary, so a
//! token granted `Subscription.c` and nothing on `Observation` cannot arrange
//! to receive every Observation written in its tenant.
//!
//! Without a principal (authentication disabled) or without the engine
//! (nothing is delivered) nothing is judged.

use helios_auth::Principal;
use helios_fhir::FhirVersion;
use serde_json::Value;

use crate::error::RestResult;
use crate::extractors::TenantExtractor;
use crate::state::AppState;

/// Whether a write of `resource_type` is judged at all: the three types the
/// engine registers a subscription or a topic from.
pub(crate) fn is_judged(resource_type: &str) -> bool {
    cfg!(feature = "subscriptions")
        && matches!(
            resource_type,
            "Subscription" | "SubscriptionTopic" | "Basic"
        )
}

/// Judges a write of `resource`, the document as it will be stored, by
/// `principal`'s Search grant on every resource type the write's topic
/// names: the triggers and the notification shape of the topic a
/// Subscription references, or of the SubscriptionTopic (R4 backport
/// `Basic`) being written.
///
/// A Subscription whose topic the tenant does not have is refused too: the
/// engine would not register it now, but a rehydration after the topic
/// appears would, and nothing could judge the writer then.
#[cfg(feature = "subscriptions")]
pub(crate) fn admit_write<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    fhir_version: FhirVersion,
    resource_type: &str,
    resource: &Value,
    principal: Option<&Principal>,
) -> RestResult<()>
where
    S: helios_persistence::core::ResourceStorage,
{
    use crate::error::RestError;
    use helios_subscriptions::topics::InMemoryTopicRegistry;

    let Some(principal) = principal else {
        return Ok(());
    };
    let Some(engine) = state.subscription_engine() else {
        return Ok(());
    };

    let (subject, topic) = match resource_type {
        "Subscription" => {
            let Ok(topic_url) =
                helios_subscriptions::manager::extract_topic_url(resource, fhir_version)
            else {
                return Ok(());
            };
            let Some(topic) = engine
                .topic_registry()
                .get_topic(tenant.tenant_id(), &topic_url)
            else {
                return Err(RestError::Forbidden {
                    message: format!(
                        "a Subscription on {topic_url} cannot be judged: the tenant has no such \
                         topic, so the resource types it would deliver are unknown"
                    ),
                });
            };
            (format!("a Subscription on {topic_url}"), topic)
        }
        "SubscriptionTopic" => match InMemoryTopicRegistry::parse_topic_resource(resource) {
            Ok(topic) => (
                format!("the SubscriptionTopic {}", topic.canonical_url),
                topic,
            ),
            Err(_) => return Ok(()),
        },
        "Basic" if fhir_version.as_str() == "R4" => {
            match InMemoryTopicRegistry::parse_r4_backport_basic_topic_resource(resource) {
                Ok(Some(topic)) => (
                    format!("the backport SubscriptionTopic {}", topic.canonical_url),
                    topic,
                ),
                _ => return Ok(()),
            }
        }
        _ => return Ok(()),
    };

    require_search_grants(principal, &subject, topic_resource_types(&topic))
}

#[cfg(not(feature = "subscriptions"))]
pub(crate) fn admit_write<S>(
    _state: &AppState<S>,
    _tenant: &TenantExtractor,
    _fhir_version: FhirVersion,
    _resource_type: &str,
    _resource: &Value,
    _principal: Option<&Principal>,
) -> RestResult<()>
where
    S: helios_persistence::core::ResourceStorage,
{
    Ok(())
}

/// The resource types a topic names, each once: its triggers and its
/// notification shape.
#[cfg(feature = "subscriptions")]
fn topic_resource_types(topic: &helios_subscriptions::topics::TopicDefinition) -> Vec<&str> {
    let mut types: Vec<&str> = topic
        .resource_triggers
        .iter()
        .map(|trigger| trigger.resource_type.as_str())
        .chain(
            topic
                .notification_shape
                .iter()
                .map(|shape| shape.resource_type.as_str()),
        )
        .collect();
    types.sort_unstable();
    types.dedup();
    types
}

/// Requires an unrestricted Search grant on every resource type `subject`
/// delivers, as a direct search on each would be judged. A grant that rests
/// on a `patient/` scope is refused: a subscription cannot be confined to the
/// launch patient's compartment.
#[cfg(feature = "subscriptions")]
fn require_search_grants<'a>(
    principal: &Principal,
    subject: &str,
    resource_types: impl IntoIterator<Item = &'a str>,
) -> RestResult<()> {
    use crate::error::RestError;
    use helios_auth::{AccessScope, FhirOperation, SmartScopePolicy};

    for resource_type in resource_types {
        match SmartScopePolicy::check(principal, resource_type, FhirOperation::Search) {
            Ok(AccessScope::Unrestricted) => {}
            Ok(AccessScope::PatientCompartment { .. }) => {
                return Err(RestError::Forbidden {
                    message: format!(
                        "{subject} delivers {resource_type}, which this token is granted only by \
                         a patient/ scope; a subscription cannot be confined to the launch \
                         patient's compartment"
                    ),
                });
            }
            Err(error) => {
                return Err(RestError::Forbidden {
                    message: format!("{subject} delivers {resource_type}: {error}"),
                });
            }
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "subscriptions"))]
mod tests {
    use super::*;
    use helios_auth::ScopeSet;
    use helios_subscriptions::topics::{NotificationShape, ResourceTrigger, TopicDefinition};

    fn principal(scopes: &str) -> Principal {
        Principal {
            subject: "sub".to_string(),
            issuer: "https://idp.example.com".to_string(),
            scopes: ScopeSet::parse(scopes),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
            ..Default::default()
        }
    }

    fn topic() -> TopicDefinition {
        TopicDefinition {
            canonical_url: "http://example.org/topics/t".to_string(),
            title: None,
            resource_triggers: vec![
                ResourceTrigger {
                    resource_type: "Encounter".to_string(),
                    interactions: vec![],
                    fhirpath_criteria: None,
                },
                ResourceTrigger {
                    resource_type: "Observation".to_string(),
                    interactions: vec![],
                    fhirpath_criteria: None,
                },
            ],
            can_filter_by: vec![],
            notification_shape: vec![NotificationShape {
                resource_type: "Encounter".to_string(),
                include: vec![],
            }],
        }
    }

    #[test]
    fn a_topic_names_each_type_once() {
        assert_eq!(
            topic_resource_types(&topic()),
            vec!["Encounter", "Observation"]
        );
    }

    #[test]
    fn every_named_type_needs_a_search_grant() {
        let topic = topic();
        let types = topic_resource_types(&topic);
        assert!(
            require_search_grants(
                &principal("user/Subscription.c user/Encounter.rs user/Observation.rs"),
                "a Subscription",
                types.iter().copied()
            )
            .is_ok()
        );
        assert!(
            require_search_grants(
                &principal("user/Subscription.c user/*.rs"),
                "a Subscription",
                types.iter().copied()
            )
            .is_ok()
        );
        let refused = require_search_grants(
            &principal("user/Subscription.c user/Encounter.rs user/Observation.r"),
            "a Subscription",
            types.iter().copied(),
        )
        .unwrap_err();
        assert!(refused.to_string().contains("Observation"), "{refused}");
    }

    #[test]
    fn a_patient_scoped_grant_is_refused() {
        let refused = require_search_grants(
            &principal("user/Subscription.c patient/Encounter.rs"),
            "a Subscription",
            ["Encounter"],
        )
        .unwrap_err();
        assert!(refused.to_string().contains("patient/ scope"), "{refused}");
    }
}
