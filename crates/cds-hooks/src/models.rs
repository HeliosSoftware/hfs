//! Core CDS Hooks data types.
//!
//! This module contains all the data structures defined in the
//! [CDS Hooks specification](https://cds-hooks.hl7.org/), including types for
//! service discovery, hook requests, service responses, cards, suggestions,
//! actions, links, and feedback.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// The response returned by the CDS Hooks discovery endpoint (`GET {baseUrl}/cds-services`).
///
/// Contains a list of [`CdsService`] definitions that describe the decision support
/// services available from this server.
///
/// # Example
///
/// ```json
/// {
///   "services": [
///     {
///       "hook": "patient-view",
///       "title": "Patient Greeter",
///       "description": "Greets the patient",
///       "id": "patient-greeter"
///     }
///   ]
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryResponse {
    /// An array of CDS Services.
    pub services: Vec<CdsService>,
}

/// Describes a single CDS Service as advertised through the discovery endpoint.
///
/// Each service declares which hook it responds to, a human-readable description,
/// and optionally a set of prefetch templates that allow the CDS Client to supply
/// FHIR data with each invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CdsService {
    /// The hook this service should be invoked on (e.g. `"patient-view"`, `"order-sign"`).
    pub hook: String,

    /// Human-friendly name of this service.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// Description of this service.
    pub description: String,

    /// The `{id}` portion of the URL to this service, available at
    /// `{baseUrl}/cds-services/{id}`.
    pub id: String,

    /// Key/value pairs of FHIR queries that this service requests the CDS Client
    /// to perform and provide on each service call.
    ///
    /// The key is a string that describes the type of data being requested and the
    /// value is a string representing the FHIR query (a prefetch template).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefetch: Option<HashMap<String, String>>,

    /// Human-friendly description of any preconditions for the use of this CDS Service.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "usageRequirements")]
    pub usage_requirements: Option<String>,

    /// Version of the CDS Hooks specification implemented (e.g. `"STU1"`, `"STU2"`, `"STU3"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// Extension data. Keys should use reverse-domain-name notation for global uniqueness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<HashMap<String, serde_json::Value>>,
}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

/// A request sent from a CDS Client to a CDS Service when a hook is triggered.
///
/// The CDS Client `POST`s this JSON document to `{baseUrl}/cds-services/{service.id}`.
/// The request includes the hook that was triggered, contextual data, and optionally
/// prefetched FHIR resources and authorization information.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CdsRequest {
    /// The hook that triggered this CDS Service call (e.g. `"patient-view"`).
    pub hook: String,

    /// A universally unique identifier (UUID) for this particular hook call.
    ///
    /// Each hook invocation gets a unique `hook_instance`, allowing the CDS Service
    /// to distinguish between parallel or sequential invocations.
    #[serde(rename = "hookInstance")]
    pub hook_instance: String,

    /// The base URL of the CDS Client's FHIR server.
    ///
    /// Required if `fhir_authorization` is provided. The scheme must be HTTPS
    /// when production data is exchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "fhirServer")]
    pub fhir_server: Option<String>,

    /// OAuth 2.0 bearer access token and supplemental information granting the
    /// CDS Service access to FHIR resources.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "fhirAuthorization")]
    pub fhir_authorization: Option<FhirAuthorization>,

    /// Hook-specific contextual data that the CDS Service needs.
    ///
    /// The shape of this object depends on the hook being invoked. For example,
    /// `patient-view` provides `userId` and `patientId`, while `order-sign`
    /// provides `draftOrders`.
    pub context: serde_json::Value,

    /// FHIR data that was prefetched by the CDS Client according to the service's
    /// prefetch templates.
    ///
    /// Keys correspond to prefetch keys from the service's discovery response.
    /// Values are FHIR resources or Bundles. A `null` value indicates the CDS Client
    /// had no data for that template.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefetch: Option<HashMap<String, Option<serde_json::Value>>>,

    /// Extension data. Keys should use reverse-domain-name notation for global uniqueness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<HashMap<String, serde_json::Value>>,
}

/// OAuth 2.0 authorization information passed from the CDS Client to the CDS Service,
/// enabling the service to access the client's FHIR server.
///
/// The access token is scoped to the specific CDS Service and current user.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FhirAuthorization {
    /// The OAuth 2.0 access token that provides access to the FHIR server.
    pub access_token: String,

    /// Fixed value: `"Bearer"`.
    pub token_type: String,

    /// The lifetime in seconds of the access token.
    pub expires_in: u64,

    /// The scopes the access token grants the CDS Service, as defined by
    /// the SMART on FHIR specification.
    pub scope: String,

    /// The OAuth 2.0 client identifier of the CDS Service, as registered with
    /// the CDS Client's authorization server.
    pub subject: String,

    /// If the granted SMART scopes include patient scopes, this identifies the
    /// FHIR id of the patient the token is restricted to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patient: Option<String>,
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

/// The response returned by a CDS Service after processing a hook request.
///
/// Contains an array of [`Card`]s providing decision support guidance, and optionally
/// an array of [`Action`]s that the service proposes to auto-apply.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CdsResponse {
    /// An array of Cards providing decision support information, suggested actions,
    /// and/or links.
    ///
    /// If the service has no decision support for the user, this should be an empty array.
    pub cards: Vec<Card>,

    /// An array of Actions that the CDS Service proposes to auto-apply without
    /// user interaction.
    ///
    /// System actions are appropriate only in specific, pre-coordinated scenarios
    /// (e.g. saving an identifier referencing the service's determination).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "systemActions")]
    pub system_actions: Option<Vec<Action>>,

    /// Extension data. Keys should use reverse-domain-name notation for global uniqueness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<HashMap<String, serde_json::Value>>,
}

// ---------------------------------------------------------------------------
// Card
// ---------------------------------------------------------------------------

/// A decision support card returned by a CDS Service.
///
/// Cards provide a combination of information (for reading), suggested actions
/// (to be applied if accepted by the user), and links (to launch an app or
/// navigate to additional information).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Card {
    /// Unique identifier of the card. Used for auditing, logging, and feedback.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,

    /// One-sentence, <140-character summary message for display to the user.
    pub summary: String,

    /// Optional detailed information to display, represented in GitHub Flavored Markdown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,

    /// Urgency/importance of what this card conveys.
    pub indicator: Indicator,

    /// The source of the information displayed on this card.
    pub source: Source,

    /// Suggested changes in the context of the current activity.
    ///
    /// If present, `selection_behavior` must also be provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestions: Option<Vec<Suggestion>>,

    /// Describes the intended selection behavior of the suggestions.
    ///
    /// Required when `suggestions` is present.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "selectionBehavior")]
    pub selection_behavior: Option<SelectionBehavior>,

    /// Override reasons that can be selected by the end user when dismissing this card.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "overrideReasons")]
    pub override_reasons: Option<Vec<Coding>>,

    /// Links to apps or additional information that the user might want to access.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<Link>>,

    /// Extension data. Keys should use reverse-domain-name notation for global uniqueness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<HashMap<String, serde_json::Value>>,
}

/// Urgency/importance indicator for a [`Card`].
///
/// Allowed values, in order of increasing urgency: `info`, `warning`, `critical`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Indicator {
    /// Informational card.
    Info,
    /// Warning-level card.
    Warning,
    /// Critical-level card.
    Critical,
}

/// Selection behavior for suggestions on a [`Card`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum SelectionBehavior {
    /// The user may choose none or at most one of the suggestions.
    AtMostOne,
    /// The user may choose any number of suggestions, including none and all.
    Any,
}

// ---------------------------------------------------------------------------
// Source
// ---------------------------------------------------------------------------

/// Grouping structure for the source of information displayed on a [`Card`].
///
/// The source should be the primary source of guidance for the decision support
/// the card represents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Source {
    /// A short, human-readable label to display for the source.
    pub label: String,

    /// An optional absolute URL to learn more about the organization or data set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// An absolute URL to a 100x100 pixel PNG icon for the source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    /// A topic describing the content of the card for filtering and categorization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<Coding>,
}

// ---------------------------------------------------------------------------
// Suggestion
// ---------------------------------------------------------------------------

/// A suggested set of changes in the context of the current activity.
///
/// Each suggestion may contain multiple [`Action`]s that are logically AND'd together:
/// accepting a suggestion accepts all its actions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Suggestion {
    /// Human-readable label to display for this suggestion (e.g. button text).
    pub label: String,

    /// Unique identifier for auditing and logging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,

    /// When there are multiple suggestions, indicates that this specific suggestion
    /// is recommended.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "isRecommended")]
    pub is_recommended: Option<bool>,

    /// Array of actions that make up this suggestion.
    ///
    /// When a suggestion contains multiple actions, they should be processed as per
    /// FHIR's rules for processing transactions (deletes first, then creates, then updates).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<Action>>,

    /// Indicates whether the end user may select any, none, or only a single action.
    ///
    /// Allowed values: `all` (default), `any`, `at-most-one`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "actionSelectionBehavior")]
    pub action_selection_behavior: Option<ActionSelectionBehavior>,
}

/// Selection behavior for actions within a [`Suggestion`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ActionSelectionBehavior {
    /// Selecting the suggestion selects all actions within it (default).
    All,
    /// The user may choose any number of actions including none or all.
    Any,
    /// The user may choose none or at most one action.
    AtMostOne,
}

// ---------------------------------------------------------------------------
// Action
// ---------------------------------------------------------------------------

/// A suggested modification to a FHIR resource.
///
/// Actions can create, update, or delete resources. Within a [`Suggestion`], all actions
/// are logically AND'd together. System actions follow the same schema but have an
/// optional `description`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Action {
    /// The type of action being performed.
    #[serde(rename = "type")]
    pub action_type: ActionType,

    /// Human-readable description of the suggested action.
    ///
    /// Required for suggestion actions, optional for system actions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// A FHIR resource.
    ///
    /// For `create` actions, contains the new resource. For `update` actions,
    /// contains the updated resource in its entirety.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<serde_json::Value>,

    /// A relative reference to the relevant resource (e.g. `"ServiceRequest/123"`).
    ///
    /// Should be provided for `delete` actions.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "resourceId")]
    pub resource_id: Option<String>,
}

/// The type of modification an [`Action`] performs.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ActionType {
    /// Create a new FHIR resource.
    Create,
    /// Update an existing FHIR resource.
    Update,
    /// Delete an existing FHIR resource.
    Delete,
}

// ---------------------------------------------------------------------------
// Link
// ---------------------------------------------------------------------------

/// A link to an app or external information displayed on a [`Card`].
///
/// Links can be either absolute URLs or SMART app launch URLs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Link {
    /// Human-readable label to display for this link.
    pub label: String,

    /// URL to load when a user clicks on this link.
    ///
    /// May be a deep link with context embedded in path segments, query parameters,
    /// or a hash.
    pub url: String,

    /// The type of the given URL.
    #[serde(rename = "type")]
    pub link_type: LinkType,

    /// Information from the CDS card to share with a subsequently launched SMART app.
    ///
    /// Only valid when `link_type` is [`LinkType::Smart`]. Passed to the SMART app
    /// as part of the OAuth 2.0 access token response.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "appContext")]
    pub app_context: Option<String>,

    /// Hint to the CDS Client that this link should be immediately launched
    /// without displaying the card and without manual user interaction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autolaunchable: Option<bool>,
}

/// The type of a [`Link`] URL.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum LinkType {
    /// An absolute URL that should be treated as-is.
    Absolute,
    /// A SMART app launch URL. The CDS Client should populate appropriate
    /// SMART launch parameters.
    Smart,
}

// ---------------------------------------------------------------------------
// Coding
// ---------------------------------------------------------------------------

/// A code from a code system, modeled after a trimmed-down version of the FHIR
/// Coding data type.
///
/// This standalone data type is used throughout CDS Hooks for override reasons,
/// source topics, and other coded values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Coding {
    /// The code for what is being represented.
    pub code: String,

    /// The code system for this code.
    pub system: String,

    /// A short, human-readable label to display.
    ///
    /// Required for override reasons provided by the CDS Service, optional for topics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
}

// ---------------------------------------------------------------------------
// Feedback
// ---------------------------------------------------------------------------

/// A feedback request sent from a CDS Client to a CDS Service at
/// `{baseUrl}/cds-services/{service.id}/feedback`.
///
/// Feedback enables CDS Services to learn the outcomes of their recommendations,
/// including which suggestions were accepted and which cards were overridden.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeedbackRequest {
    /// An array of individual feedback entries.
    pub feedback: Vec<Feedback>,
}

/// An individual feedback entry describing what happened with a specific card.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Feedback {
    /// The `card.uuid` from the CDS Hooks response, uniquely identifying the card.
    pub card: String,

    /// Whether the card's suggestion was accepted or the card was overridden.
    pub outcome: FeedbackOutcome,

    /// The suggestions that were accepted by the user.
    ///
    /// Required when `outcome` is [`FeedbackOutcome::Accepted`].
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "acceptedSuggestions")]
    pub accepted_suggestions: Option<Vec<AcceptedSuggestion>>,

    /// The reason the card was overridden, including optional free-text comments.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "overrideReason")]
    pub override_reason: Option<OverrideReason>,

    /// ISO 8601 timestamp (UTC) when the action was taken on the card.
    #[serde(rename = "outcomeTimestamp")]
    pub outcome_timestamp: DateTime<Utc>,
}

/// The outcome of user interaction with a [`Card`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackOutcome {
    /// The user accepted one or more suggestions from the card.
    Accepted,
    /// The user dismissed/overrode the card.
    Overridden,
}

/// Identifies a suggestion that was accepted by the user.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AcceptedSuggestion {
    /// The `suggestion.uuid` from the CDS Hooks response, uniquely identifying
    /// the accepted suggestion.
    pub id: String,
}

/// The reason a card was overridden, including an optional clinician comment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverrideReason {
    /// The coded override reason selected by the end user.
    ///
    /// Required if the user selected a reason from the list provided in the card.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Coding>,

    /// Free-text comment from the clinician explaining why the card was rejected.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "userComment")]
    pub user_comment: Option<String>,
}

// ---------------------------------------------------------------------------
// Builder helpers
// ---------------------------------------------------------------------------

impl Card {
    /// Creates a new informational card with the given summary and source label.
    ///
    /// # Example
    ///
    /// ```
    /// use helios_cds_hooks::Card;
    ///
    /// let card = Card::info("Patient has no known allergies", "Allergy Checker");
    /// assert_eq!(card.summary, "Patient has no known allergies");
    /// assert_eq!(card.indicator, helios_cds_hooks::Indicator::Info);
    /// ```
    pub fn info(summary: impl Into<String>, source_label: impl Into<String>) -> Self {
        Self {
            uuid: None,
            summary: summary.into(),
            detail: None,
            indicator: Indicator::Info,
            source: Source {
                label: source_label.into(),
                url: None,
                icon: None,
                topic: None,
            },
            suggestions: None,
            selection_behavior: None,
            override_reasons: None,
            links: None,
            extension: None,
        }
    }

    /// Creates a new warning card with the given summary and source label.
    ///
    /// # Example
    ///
    /// ```
    /// use helios_cds_hooks::Card;
    ///
    /// let card = Card::warning("Drug interaction detected", "Drug Checker");
    /// assert_eq!(card.indicator, helios_cds_hooks::Indicator::Warning);
    /// ```
    pub fn warning(summary: impl Into<String>, source_label: impl Into<String>) -> Self {
        let mut card = Self::info(summary, source_label);
        card.indicator = Indicator::Warning;
        card
    }

    /// Creates a new critical card with the given summary and source label.
    ///
    /// # Example
    ///
    /// ```
    /// use helios_cds_hooks::Card;
    ///
    /// let card = Card::critical("Severe allergy to prescribed medication", "Safety Alert");
    /// assert_eq!(card.indicator, helios_cds_hooks::Indicator::Critical);
    /// ```
    pub fn critical(summary: impl Into<String>, source_label: impl Into<String>) -> Self {
        let mut card = Self::info(summary, source_label);
        card.indicator = Indicator::Critical;
        card
    }
}

impl CdsResponse {
    /// Creates a response with no cards, indicating no decision support for the user.
    ///
    /// # Example
    ///
    /// ```
    /// use helios_cds_hooks::CdsResponse;
    ///
    /// let response = CdsResponse::empty();
    /// assert!(response.cards.is_empty());
    /// ```
    pub fn empty() -> Self {
        Self {
            cards: Vec::new(),
            system_actions: None,
            extension: None,
        }
    }

    /// Creates a response containing the given cards.
    pub fn with_cards(cards: Vec<Card>) -> Self {
        Self {
            cards,
            system_actions: None,
            extension: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_discovery_response() {
        let response = DiscoveryResponse {
            services: vec![CdsService {
                hook: "patient-view".to_string(),
                title: Some("Patient Greeter".to_string()),
                description: "Greets the patient".to_string(),
                id: "patient-greeter".to_string(),
                prefetch: Some(HashMap::from([(
                    "patientToGreet".to_string(),
                    "Patient/{{context.patientId}}".to_string(),
                )])),
                usage_requirements: None,
                version: None,
                extension: None,
            }],
        };

        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["services"][0]["hook"], "patient-view");
        assert_eq!(json["services"][0]["id"], "patient-greeter");
        assert!(
            json["services"][0]["prefetch"]["patientToGreet"]
                .as_str()
                .unwrap()
                .contains("{{context.patientId}}")
        );
    }

    #[test]
    fn test_deserialize_cds_request() {
        let json = r#"{
            "hook": "patient-view",
            "hookInstance": "d1577c69-dfbe-44ad-ba6d-3e05e953b2ea",
            "fhirServer": "http://hooks.smarthealthit.org:9080",
            "context": {
                "userId": "Practitioner/example",
                "patientId": "1288992"
            },
            "prefetch": {
                "patientToGreet": {
                    "resourceType": "Patient",
                    "id": "1288992",
                    "gender": "male"
                }
            }
        }"#;

        let request: CdsRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.hook, "patient-view");
        assert_eq!(
            request.hook_instance,
            "d1577c69-dfbe-44ad-ba6d-3e05e953b2ea"
        );
        assert!(request.fhir_server.is_some());
    }

    #[test]
    fn test_serialize_response_with_cards() {
        let response = CdsResponse::with_cards(vec![
            Card::info("Example Card", "Static CDS Service Example"),
            Card::warning("Another card", "Static CDS Service Example"),
        ]);

        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["cards"].as_array().unwrap().len(), 2);
        assert_eq!(json["cards"][0]["indicator"], "info");
        assert_eq!(json["cards"][1]["indicator"], "warning");
    }

    #[test]
    fn test_empty_response() {
        let response = CdsResponse::empty();
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(json, r#"{"cards":[]}"#);
    }

    #[test]
    fn test_card_with_suggestions() {
        let card = Card {
            uuid: Some("test-uuid".to_string()),
            summary: "Consider alternative".to_string(),
            detail: Some("Details in **markdown**".to_string()),
            indicator: Indicator::Warning,
            source: Source {
                label: "Drug Checker".to_string(),
                url: None,
                icon: None,
                topic: None,
            },
            suggestions: Some(vec![Suggestion {
                label: "Switch to alternative".to_string(),
                uuid: Some("suggestion-uuid".to_string()),
                is_recommended: Some(true),
                actions: Some(vec![Action {
                    action_type: ActionType::Create,
                    description: Some("Create alternative prescription".to_string()),
                    resource: Some(serde_json::json!({
                        "resourceType": "MedicationRequest",
                        "id": "alt-rx-1"
                    })),
                    resource_id: None,
                }]),
                action_selection_behavior: None,
            }]),
            selection_behavior: Some(SelectionBehavior::AtMostOne),
            override_reasons: None,
            links: None,
            extension: None,
        };

        let json = serde_json::to_value(&card).unwrap();
        assert_eq!(json["selectionBehavior"], "at-most-one");
        assert_eq!(json["suggestions"][0]["isRecommended"], true);
    }

    #[test]
    fn test_serialize_feedback() {
        let feedback = FeedbackRequest {
            feedback: vec![Feedback {
                card: "4e0a3a1e-3283-4575-ab82-028d55fe2719".to_string(),
                outcome: FeedbackOutcome::Accepted,
                accepted_suggestions: Some(vec![AcceptedSuggestion {
                    id: "e56e1945-20b3-4393-8503-a1a20fd73152".to_string(),
                }]),
                override_reason: None,
                outcome_timestamp: "2021-12-11T10:05:31Z".parse().unwrap(),
            }],
        };

        let json = serde_json::to_value(&feedback).unwrap();
        assert_eq!(json["feedback"][0]["outcome"], "accepted");
        assert!(json["feedback"][0]["acceptedSuggestions"].is_array());
    }

    #[test]
    fn test_indicator_ordering() {
        let json_info = serde_json::to_string(&Indicator::Info).unwrap();
        let json_warning = serde_json::to_string(&Indicator::Warning).unwrap();
        let json_critical = serde_json::to_string(&Indicator::Critical).unwrap();

        assert_eq!(json_info, r#""info""#);
        assert_eq!(json_warning, r#""warning""#);
        assert_eq!(json_critical, r#""critical""#);
    }

    #[test]
    fn test_link_types() {
        let absolute = Link {
            label: "Google".to_string(),
            url: "https://google.com".to_string(),
            link_type: LinkType::Absolute,
            app_context: None,
            autolaunchable: None,
        };

        let smart = Link {
            label: "SMART App".to_string(),
            url: "https://smart.example.com/launch".to_string(),
            link_type: LinkType::Smart,
            app_context: Some(r#"{"session":3456356}"#.to_string()),
            autolaunchable: Some(true),
        };

        let json_abs = serde_json::to_value(&absolute).unwrap();
        assert_eq!(json_abs["type"], "absolute");

        let json_smart = serde_json::to_value(&smart).unwrap();
        assert_eq!(json_smart["type"], "smart");
        assert!(json_smart["appContext"].is_string());
        assert_eq!(json_smart["autolaunchable"], true);
    }
}
