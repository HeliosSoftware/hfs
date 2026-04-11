//! Subscription topic registry.
//!
//! Manages `SubscriptionTopic` definitions and evaluates whether resource events
//! match a topic's triggers.

use std::collections::HashMap;
use std::sync::RwLock;

use crate::error::SubscriptionError;
use crate::event::ResourceEventType;

/// A version-agnostic representation of a `SubscriptionTopic`.
#[derive(Debug, Clone)]
pub struct TopicDefinition {
    /// The canonical URL of the topic (e.g., `http://example.org/topic/encounter-start`).
    pub canonical_url: String,

    /// Human-readable title.
    pub title: Option<String>,

    /// Resource triggers that define when this topic fires.
    pub resource_triggers: Vec<ResourceTrigger>,

    /// Filters that subscribers can use to narrow which events they receive.
    pub can_filter_by: Vec<FilterDefinition>,

    /// Notification shape: which resource types may appear in notifications.
    pub notification_shape: Vec<NotificationShape>,
}

/// A trigger condition defined by a topic.
#[derive(Debug, Clone)]
pub struct ResourceTrigger {
    /// The resource type this trigger monitors (e.g., "Encounter", "Observation").
    pub resource_type: String,

    /// Which interaction types trigger this (create, update, delete).
    pub interactions: Vec<ResourceEventType>,

    /// Optional FHIRPath expression for additional trigger criteria.
    pub fhirpath_criteria: Option<String>,
}

/// Defines a filter parameter that subscribers can use.
#[derive(Debug, Clone)]
pub struct FilterDefinition {
    /// The resource type this filter applies to.
    pub resource_type: Option<String>,

    /// The filter parameter name (often a FHIR search parameter name).
    pub filter_parameter: String,

    /// Supported comparators (e.g., "eq", "in", "gt").
    pub comparators: Vec<String>,

    /// Supported modifiers (e.g., "missing", "exact").
    pub modifiers: Vec<String>,
}

/// Defines the shape of notifications for a topic.
#[derive(Debug, Clone)]
pub struct NotificationShape {
    /// The resource type included in notifications.
    pub resource_type: String,

    /// Include references to follow.
    pub include: Vec<String>,
}

/// Describes a topic whose trigger matched a resource event.
#[derive(Debug, Clone)]
pub struct TopicMatch {
    /// Canonical URL of the matching topic.
    pub topic_url: String,

    /// The focus resource type that triggered the match.
    pub focus_resource_type: String,
}

/// Registry for subscription topics.
///
/// Stores topic definitions and evaluates which topics match a given
/// resource event.
pub struct InMemoryTopicRegistry {
    /// Topics keyed by canonical URL.
    topics: RwLock<HashMap<String, TopicDefinition>>,
}

impl InMemoryTopicRegistry {
    /// Creates an empty topic registry.
    pub fn new() -> Self {
        Self {
            topics: RwLock::new(HashMap::new()),
        }
    }

    /// Registers a topic definition.
    pub fn add_topic(&self, topic: TopicDefinition) {
        let mut topics = self.topics.write().unwrap();
        topics.insert(topic.canonical_url.clone(), topic);
    }

    /// Removes a topic by canonical URL.
    pub fn remove_topic(&self, canonical_url: &str) -> bool {
        let mut topics = self.topics.write().unwrap();
        topics.remove(canonical_url).is_some()
    }

    /// Returns all registered topic canonical URLs.
    pub fn list_topics(&self) -> Vec<String> {
        let topics = self.topics.read().unwrap();
        topics.keys().cloned().collect()
    }

    /// Returns a topic definition by canonical URL.
    pub fn get_topic(&self, canonical_url: &str) -> Option<TopicDefinition> {
        let topics = self.topics.read().unwrap();
        topics.get(canonical_url).cloned()
    }

    /// Evaluates which topics match a resource event.
    ///
    /// Checks all registered topics' resource triggers against the event's
    /// resource type and interaction type.
    pub fn matching_topics(
        &self,
        resource_type: &str,
        event_type: ResourceEventType,
    ) -> Vec<TopicMatch> {
        let topics = self.topics.read().unwrap();
        let mut matches = Vec::new();

        for topic in topics.values() {
            for trigger in &topic.resource_triggers {
                if trigger.resource_type == resource_type
                    && trigger.interactions.contains(&event_type)
                {
                    matches.push(TopicMatch {
                        topic_url: topic.canonical_url.clone(),
                        focus_resource_type: trigger.resource_type.clone(),
                    });
                    // Only match once per topic even if multiple triggers match.
                    break;
                }
            }
        }

        matches
    }

    /// Parses a `SubscriptionTopic` FHIR resource (JSON) into a [`TopicDefinition`].
    ///
    /// Works for R4B, R5, and R6 native `SubscriptionTopic` resources.
    pub fn parse_topic_resource(
        resource: &serde_json::Value,
    ) -> Result<TopicDefinition, SubscriptionError> {
        let canonical_url = resource
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SubscriptionError::InvalidSubscription {
                message: "SubscriptionTopic missing 'url' field".to_string(),
            })?
            .to_string();

        let title = resource
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let resource_triggers = parse_resource_triggers(resource)?;
        let can_filter_by = parse_can_filter_by(resource);
        let notification_shape = parse_notification_shape(resource);

        Ok(TopicDefinition {
            canonical_url,
            title,
            resource_triggers,
            can_filter_by,
            notification_shape,
        })
    }
}

impl Default for InMemoryTopicRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse `resourceTrigger` array from a SubscriptionTopic JSON resource.
fn parse_resource_triggers(
    resource: &serde_json::Value,
) -> Result<Vec<ResourceTrigger>, SubscriptionError> {
    let triggers = match resource.get("resourceTrigger").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return Ok(Vec::new()),
    };

    let mut result = Vec::new();
    for trigger in triggers {
        let resource_type = trigger
            .get("resource")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SubscriptionError::InvalidSubscription {
                message: "resourceTrigger missing 'resource' field".to_string(),
            })?
            .to_string();

        let interactions = parse_interactions(trigger);

        let fhirpath_criteria = trigger
            .get("fhirPathCriteria")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        result.push(ResourceTrigger {
            resource_type,
            interactions,
            fhirpath_criteria,
        });
    }

    Ok(result)
}

/// Parse `supportedInteraction` array from a trigger definition.
fn parse_interactions(trigger: &serde_json::Value) -> Vec<ResourceEventType> {
    let interactions = match trigger
        .get("supportedInteraction")
        .and_then(|v| v.as_array())
    {
        Some(arr) => arr,
        None => {
            // Default to all interactions if not specified.
            return vec![
                ResourceEventType::Create,
                ResourceEventType::Update,
                ResourceEventType::Delete,
            ];
        }
    };

    interactions
        .iter()
        .filter_map(|v| match v.as_str()? {
            "create" => Some(ResourceEventType::Create),
            "update" => Some(ResourceEventType::Update),
            "delete" => Some(ResourceEventType::Delete),
            _ => None,
        })
        .collect()
}

/// Parse `canFilterBy` array from a SubscriptionTopic JSON resource.
fn parse_can_filter_by(resource: &serde_json::Value) -> Vec<FilterDefinition> {
    let filters = match resource.get("canFilterBy").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    filters
        .iter()
        .filter_map(|f| {
            let filter_parameter = f.get("filterParameter")?.as_str()?.to_string();
            let resource_type = f.get("resource").and_then(|v| v.as_str()).map(String::from);

            let comparators = f
                .get("comparator")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            let modifiers = f
                .get("modifier")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            Some(FilterDefinition {
                resource_type,
                filter_parameter,
                comparators,
                modifiers,
            })
        })
        .collect()
}

/// Parse `notificationShape` array from a SubscriptionTopic JSON resource.
fn parse_notification_shape(resource: &serde_json::Value) -> Vec<NotificationShape> {
    let shapes = match resource.get("notificationShape").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    shapes
        .iter()
        .filter_map(|s| {
            let resource_type = s.get("resource")?.as_str()?.to_string();
            let include = s
                .get("include")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            Some(NotificationShape {
                resource_type,
                include,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_encounter_topic() -> TopicDefinition {
        TopicDefinition {
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
            notification_shape: vec![NotificationShape {
                resource_type: "Encounter".to_string(),
                include: vec!["Encounter:patient".to_string()],
            }],
        }
    }

    fn sample_observation_topic() -> TopicDefinition {
        TopicDefinition {
            canonical_url: "http://example.org/topic/new-lab-result".to_string(),
            title: Some("New Lab Result".to_string()),
            resource_triggers: vec![ResourceTrigger {
                resource_type: "Observation".to_string(),
                interactions: vec![ResourceEventType::Create, ResourceEventType::Update],
                fhirpath_criteria: None,
            }],
            can_filter_by: vec![FilterDefinition {
                resource_type: Some("Observation".to_string()),
                filter_parameter: "code".to_string(),
                comparators: vec!["eq".to_string(), "in".to_string()],
                modifiers: vec![],
            }],
            notification_shape: vec![],
        }
    }

    #[test]
    fn test_add_and_list_topics() {
        let registry = InMemoryTopicRegistry::new();
        assert!(registry.list_topics().is_empty());

        registry.add_topic(sample_encounter_topic());
        let topics = registry.list_topics();
        assert_eq!(topics.len(), 1);
        assert!(topics.contains(&"http://example.org/topic/encounter-start".to_string()));
    }

    #[test]
    fn test_get_topic() {
        let registry = InMemoryTopicRegistry::new();
        registry.add_topic(sample_encounter_topic());

        let topic = registry
            .get_topic("http://example.org/topic/encounter-start")
            .unwrap();
        assert_eq!(topic.title.unwrap(), "Encounter Start");
        assert_eq!(topic.resource_triggers.len(), 1);

        assert!(
            registry
                .get_topic("http://example.org/nonexistent")
                .is_none()
        );
    }

    #[test]
    fn test_remove_topic() {
        let registry = InMemoryTopicRegistry::new();
        registry.add_topic(sample_encounter_topic());

        assert!(registry.remove_topic("http://example.org/topic/encounter-start"));
        assert!(registry.list_topics().is_empty());

        assert!(!registry.remove_topic("http://example.org/nonexistent"));
    }

    #[test]
    fn test_matching_topics_by_resource_type_and_interaction() {
        let registry = InMemoryTopicRegistry::new();
        registry.add_topic(sample_encounter_topic());
        registry.add_topic(sample_observation_topic());

        // Encounter create should match encounter topic.
        let matches = registry.matching_topics("Encounter", ResourceEventType::Create);
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].topic_url,
            "http://example.org/topic/encounter-start"
        );
        assert_eq!(matches[0].focus_resource_type, "Encounter");

        // Observation create should match observation topic.
        let matches = registry.matching_topics("Observation", ResourceEventType::Create);
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].topic_url,
            "http://example.org/topic/new-lab-result"
        );

        // Observation update should also match.
        let matches = registry.matching_topics("Observation", ResourceEventType::Update);
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn test_no_match_for_wrong_resource_type() {
        let registry = InMemoryTopicRegistry::new();
        registry.add_topic(sample_encounter_topic());

        let matches = registry.matching_topics("Patient", ResourceEventType::Create);
        assert!(matches.is_empty());
    }

    #[test]
    fn test_no_match_for_wrong_interaction() {
        let registry = InMemoryTopicRegistry::new();
        registry.add_topic(sample_encounter_topic());

        // Encounter topic only triggers on create, not update or delete.
        let matches = registry.matching_topics("Encounter", ResourceEventType::Update);
        assert!(matches.is_empty());

        let matches = registry.matching_topics("Encounter", ResourceEventType::Delete);
        assert!(matches.is_empty());
    }

    #[test]
    fn test_multiple_topics_matching_same_event() {
        let registry = InMemoryTopicRegistry::new();

        // Add two topics that both trigger on Observation create.
        registry.add_topic(sample_observation_topic());
        registry.add_topic(TopicDefinition {
            canonical_url: "http://example.org/topic/vital-signs".to_string(),
            title: Some("Vital Signs".to_string()),
            resource_triggers: vec![ResourceTrigger {
                resource_type: "Observation".to_string(),
                interactions: vec![ResourceEventType::Create],
                fhirpath_criteria: None,
            }],
            can_filter_by: vec![],
            notification_shape: vec![],
        });

        let matches = registry.matching_topics("Observation", ResourceEventType::Create);
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_parse_topic_resource() {
        let topic_json = json!({
            "resourceType": "SubscriptionTopic",
            "url": "http://example.org/topic/patient-admit",
            "title": "Patient Admission",
            "resourceTrigger": [{
                "resource": "Encounter",
                "supportedInteraction": ["create", "update"],
                "fhirPathCriteria": "(%previous.empty() | (%previous.status != 'in-progress')) and (%current.status = 'in-progress')"
            }],
            "canFilterBy": [{
                "resource": "Encounter",
                "filterParameter": "patient",
                "comparator": ["eq"]
            }],
            "notificationShape": [{
                "resource": "Encounter",
                "include": ["Encounter:patient", "Encounter:location"]
            }]
        });

        let topic = InMemoryTopicRegistry::parse_topic_resource(&topic_json).unwrap();
        assert_eq!(
            topic.canonical_url,
            "http://example.org/topic/patient-admit"
        );
        assert_eq!(topic.title.unwrap(), "Patient Admission");

        assert_eq!(topic.resource_triggers.len(), 1);
        let trigger = &topic.resource_triggers[0];
        assert_eq!(trigger.resource_type, "Encounter");
        assert_eq!(trigger.interactions.len(), 2);
        assert!(trigger.interactions.contains(&ResourceEventType::Create));
        assert!(trigger.interactions.contains(&ResourceEventType::Update));
        assert!(trigger.fhirpath_criteria.is_some());

        assert_eq!(topic.can_filter_by.len(), 1);
        assert_eq!(topic.can_filter_by[0].filter_parameter, "patient");

        assert_eq!(topic.notification_shape.len(), 1);
        assert_eq!(topic.notification_shape[0].include.len(), 2);
    }

    #[test]
    fn test_parse_topic_resource_missing_url() {
        let topic_json = json!({
            "resourceType": "SubscriptionTopic",
            "title": "No URL"
        });

        let result = InMemoryTopicRegistry::parse_topic_resource(&topic_json);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_topic_resource_minimal() {
        let topic_json = json!({
            "resourceType": "SubscriptionTopic",
            "url": "http://example.org/topic/minimal"
        });

        let topic = InMemoryTopicRegistry::parse_topic_resource(&topic_json).unwrap();
        assert_eq!(topic.canonical_url, "http://example.org/topic/minimal");
        assert!(topic.resource_triggers.is_empty());
        assert!(topic.can_filter_by.is_empty());
        assert!(topic.notification_shape.is_empty());
    }

    #[test]
    fn test_parse_topic_default_interactions() {
        // When supportedInteraction is not specified, all interactions should be assumed.
        let topic_json = json!({
            "resourceType": "SubscriptionTopic",
            "url": "http://example.org/topic/all-interactions",
            "resourceTrigger": [{
                "resource": "Patient"
            }]
        });

        let topic = InMemoryTopicRegistry::parse_topic_resource(&topic_json).unwrap();
        let trigger = &topic.resource_triggers[0];
        assert_eq!(trigger.interactions.len(), 3);
    }
}
