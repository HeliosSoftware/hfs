//! Subscription topic registry.
//!
//! Manages `SubscriptionTopic` definitions and evaluates whether resource events
//! match a topic's triggers.

use std::collections::HashMap;
use std::sync::RwLock;

use crate::error::SubscriptionError;
use crate::event::ResourceEventType;

const FHIR_TYPES_SYSTEM: &str = "http://hl7.org/fhir/fhir-types";
const SUBSCRIPTION_TOPIC_CODE: &str = "SubscriptionTopic";
const EXT_TOPIC_URL_SUFFIX: &str = "extension-SubscriptionTopic.url";
const EXT_TOPIC_TITLE_SUFFIX: &str = "extension-SubscriptionTopic.title";
const EXT_TOPIC_RESOURCE_TRIGGER_SUFFIX: &str = "extension-SubscriptionTopic.resourceTrigger";
const EXT_TOPIC_CAN_FILTER_BY_SUFFIX: &str = "extension-SubscriptionTopic.canFilterBy";
const EXT_TOPIC_NOTIFICATION_SHAPE_SUFFIX: &str = "extension-SubscriptionTopic.notificationShape";

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

    /// Parses an R4 backport `Basic` topic into a [`TopicDefinition`].
    ///
    /// Returns:
    /// - `Ok(Some(topic))` when the resource is a strict backport topic
    /// - `Ok(None)` when the resource is `Basic` but not a topic
    /// - `Err` when the resource is marked as a topic but malformed
    pub fn parse_r4_backport_basic_topic_resource(
        resource: &serde_json::Value,
    ) -> Result<Option<TopicDefinition>, SubscriptionError> {
        if resource.get("resourceType").and_then(|v| v.as_str()) != Some("Basic") {
            return Ok(None);
        }

        if !has_subscription_topic_basic_code(resource) {
            return Ok(None);
        }

        let canonical_url =
            find_top_level_extension_value_string(resource, EXT_TOPIC_URL_SUFFIX, &["valueUri"])
                .ok_or_else(|| SubscriptionError::InvalidSubscription {
                    message: "R4 Basic SubscriptionTopic missing canonical url extension"
                        .to_string(),
                })?;

        let title = find_top_level_extension_value_string(
            resource,
            EXT_TOPIC_TITLE_SUFFIX,
            &["valueString"],
        );

        let resource_triggers = parse_r4_basic_resource_triggers(resource)?;
        if resource_triggers.is_empty() {
            return Err(SubscriptionError::InvalidSubscription {
                message: "R4 Basic SubscriptionTopic missing resourceTrigger extension".to_string(),
            });
        }

        let can_filter_by = parse_r4_basic_can_filter_by(resource);
        let notification_shape = parse_r4_basic_notification_shape(resource);

        Ok(Some(TopicDefinition {
            canonical_url,
            title,
            resource_triggers,
            can_filter_by,
            notification_shape,
        }))
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

fn has_subscription_topic_basic_code(resource: &serde_json::Value) -> bool {
    resource
        .get("code")
        .and_then(|v| v.get("coding"))
        .and_then(|v| v.as_array())
        .map(|coding| {
            coding.iter().any(|entry| {
                entry.get("system").and_then(|v| v.as_str()) == Some(FHIR_TYPES_SYSTEM)
                    && entry.get("code").and_then(|v| v.as_str()) == Some(SUBSCRIPTION_TOPIC_CODE)
            })
        })
        .unwrap_or(false)
}

fn extension_url_matches_suffix(ext: &serde_json::Value, suffix: &str) -> bool {
    ext.get("url")
        .and_then(|v| v.as_str())
        .map(|url| url.ends_with(suffix))
        .unwrap_or(false)
}

fn find_top_level_extension_value_string(
    resource: &serde_json::Value,
    suffix: &str,
    value_keys: &[&str],
) -> Option<String> {
    resource
        .get("extension")?
        .as_array()?
        .iter()
        .find(|ext| extension_url_matches_suffix(ext, suffix))
        .and_then(|ext| {
            for key in value_keys {
                if let Some(value) = ext.get(*key).and_then(|v| v.as_str()) {
                    return Some(value.to_string());
                }
            }
            None
        })
}

fn find_nested_extension_value_string(
    ext: &serde_json::Value,
    key: &str,
    value_keys: &[&str],
) -> Option<String> {
    ext.get("extension")?.as_array()?.iter().find_map(|nested| {
        if nested.get("url").and_then(|v| v.as_str()) != Some(key) {
            return None;
        }

        for value_key in value_keys {
            if let Some(value) = nested.get(*value_key).and_then(|v| v.as_str()) {
                return Some(value.to_string());
            }
        }
        None
    })
}

fn find_all_nested_extension_values_string(
    ext: &serde_json::Value,
    key: &str,
    value_keys: &[&str],
) -> Vec<String> {
    ext.get("extension")
        .and_then(|v| v.as_array())
        .map(|nested| {
            nested
                .iter()
                .filter(|item| item.get("url").and_then(|v| v.as_str()) == Some(key))
                .filter_map(|item| {
                    for value_key in value_keys {
                        if let Some(value) = item.get(*value_key).and_then(|v| v.as_str()) {
                            return Some(value.to_string());
                        }
                    }
                    None
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_resource_type_from_uri(uri: &str) -> String {
    if let Some(resource_type) = uri.rsplit('/').next() {
        return resource_type.to_string();
    }
    uri.to_string()
}

fn parse_r4_basic_resource_triggers(
    resource: &serde_json::Value,
) -> Result<Vec<ResourceTrigger>, SubscriptionError> {
    let trigger_exts = resource
        .get("extension")
        .and_then(|v| v.as_array())
        .map(|exts| {
            exts.iter()
                .filter(|ext| extension_url_matches_suffix(ext, EXT_TOPIC_RESOURCE_TRIGGER_SUFFIX))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut triggers = Vec::new();
    for trigger_ext in trigger_exts {
        let resource_uri =
            find_nested_extension_value_string(trigger_ext, "resource", &["valueUri"]).ok_or_else(
                || SubscriptionError::InvalidSubscription {
                    message: "R4 Basic SubscriptionTopic trigger missing resource".to_string(),
                },
            )?;
        let resource_type = parse_resource_type_from_uri(&resource_uri);

        let interactions = find_all_nested_extension_values_string(
            trigger_ext,
            "supportedInteraction",
            &["valueCode"],
        )
        .iter()
        .filter_map(|code| match code.as_str() {
            "create" => Some(ResourceEventType::Create),
            "update" => Some(ResourceEventType::Update),
            "delete" => Some(ResourceEventType::Delete),
            _ => None,
        })
        .collect::<Vec<_>>();

        if interactions.is_empty() {
            return Err(SubscriptionError::InvalidSubscription {
                message: "R4 Basic SubscriptionTopic trigger missing supportedInteraction"
                    .to_string(),
            });
        }

        let fhirpath_criteria =
            find_nested_extension_value_string(trigger_ext, "fhirPathCriteria", &["valueString"]);

        triggers.push(ResourceTrigger {
            resource_type,
            interactions,
            fhirpath_criteria,
        });
    }

    Ok(triggers)
}

fn parse_r4_basic_can_filter_by(resource: &serde_json::Value) -> Vec<FilterDefinition> {
    let can_filter_by_exts = resource
        .get("extension")
        .and_then(|v| v.as_array())
        .map(|exts| {
            exts.iter()
                .filter(|ext| extension_url_matches_suffix(ext, EXT_TOPIC_CAN_FILTER_BY_SUFFIX))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    can_filter_by_exts
        .iter()
        .filter_map(|ext| {
            let filter_parameter =
                find_nested_extension_value_string(ext, "filterParameter", &["valueString"])?;

            let resource_type =
                find_nested_extension_value_string(ext, "resource", &["valueUri", "valueString"])
                    .map(|value| parse_resource_type_from_uri(&value));

            let comparators =
                find_all_nested_extension_values_string(ext, "comparator", &["valueCode"]);
            let modifiers =
                find_all_nested_extension_values_string(ext, "modifier", &["valueCode"]);

            Some(FilterDefinition {
                resource_type,
                filter_parameter,
                comparators,
                modifiers,
            })
        })
        .collect()
}

fn parse_r4_basic_notification_shape(resource: &serde_json::Value) -> Vec<NotificationShape> {
    let shape_exts = resource
        .get("extension")
        .and_then(|v| v.as_array())
        .map(|exts| {
            exts.iter()
                .filter(|ext| {
                    extension_url_matches_suffix(ext, EXT_TOPIC_NOTIFICATION_SHAPE_SUFFIX)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    shape_exts
        .iter()
        .filter_map(|ext| {
            let resource_type =
                find_nested_extension_value_string(ext, "resource", &["valueUri", "valueString"])
                    .map(|value| parse_resource_type_from_uri(&value))?;
            let include = find_all_nested_extension_values_string(ext, "include", &["valueString"]);

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

    #[test]
    fn test_parse_r4_backport_basic_topic_resource() {
        let topic_json = json!({
            "resourceType": "Basic",
            "id": "topic-basic-1",
            "code": {
                "coding": [{
                    "system": "http://hl7.org/fhir/fhir-types",
                    "code": "SubscriptionTopic"
                }]
            },
            "extension": [
                {
                    "url": "http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.url",
                    "valueUri": "http://example.org/topic/basic-encounter"
                },
                {
                    "url": "http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.title",
                    "valueString": "Basic Encounter Topic"
                },
                {
                    "url": "http://hl7.org/fhir/4.3/StructureDefinition/extension-SubscriptionTopic.resourceTrigger",
                    "extension": [
                        { "url": "resource", "valueUri": "http://hl7.org/fhir/StructureDefinition/Encounter" },
                        { "url": "supportedInteraction", "valueCode": "create" }
                    ]
                }
            ]
        });

        let topic = InMemoryTopicRegistry::parse_r4_backport_basic_topic_resource(&topic_json)
            .unwrap()
            .expect("basic topic should parse");
        assert_eq!(
            topic.canonical_url,
            "http://example.org/topic/basic-encounter"
        );
        assert_eq!(topic.title.as_deref(), Some("Basic Encounter Topic"));
        assert_eq!(topic.resource_triggers.len(), 1);
        assert_eq!(topic.resource_triggers[0].resource_type, "Encounter");
        assert_eq!(topic.resource_triggers[0].interactions.len(), 1);
        assert!(
            topic.resource_triggers[0]
                .interactions
                .contains(&ResourceEventType::Create)
        );
    }

    #[test]
    fn test_parse_r4_backport_basic_topic_requires_subscriptiontopic_code() {
        let basic_json = json!({
            "resourceType": "Basic",
            "code": {
                "coding": [{
                    "system": "http://hl7.org/fhir/fhir-types",
                    "code": "OtherType"
                }]
            },
            "extension": [{
                "url": "http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.url",
                "valueUri": "http://example.org/topic/basic-encounter"
            }]
        });

        let result =
            InMemoryTopicRegistry::parse_r4_backport_basic_topic_resource(&basic_json).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_r4_backport_basic_topic_missing_core_extensions_errors() {
        let malformed_topic = json!({
            "resourceType": "Basic",
            "code": {
                "coding": [{
                    "system": "http://hl7.org/fhir/fhir-types",
                    "code": "SubscriptionTopic"
                }]
            },
            "extension": []
        });

        let result =
            InMemoryTopicRegistry::parse_r4_backport_basic_topic_resource(&malformed_topic);
        assert!(result.is_err());
    }
}
