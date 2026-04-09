//! Fluent builder for FHIR `AuditEvent` resources.
//!
//! Uses the typed `helios_fhir::r4::AuditEvent` struct with convenience
//! helpers from [`crate::helpers`] and BALP profile selection from
//! [`crate::balp`].

use helios_fhir::r4::{AuditEvent, AuditEventAgent, AuditEventEntity, AuditEventSource, Meta};

use crate::balp::{self, AuditAction, code_systems};
use crate::helpers::*;

/// Builder for constructing BALP-compliant `AuditEvent` resources.
///
/// # Example
///
/// ```rust,ignore
/// let event = AuditEventBuilder::new("Device/hfs")
///     .action(AuditAction::Read)
///     .outcome("0")
///     .resource("Patient", "123")
///     .patient("Patient/123")
///     .agent("Practitioner/dr-smith", None, true)
///     .build();
/// ```
pub struct AuditEventBuilder {
    action: Option<AuditAction>,
    outcome: Option<String>,
    outcome_desc: Option<String>,
    resource_type: Option<String>,
    resource_id: Option<String>,
    patient_reference: Option<String>,
    agent_who: Option<String>,
    agent_name: Option<String>,
    agent_requestor: bool,
    source_observer: String,
    query_string: Option<String>,
    entity_details: Vec<(String, String)>,
    extra_entities: Vec<AuditEventEntity>,
    event_type_system: Option<String>,
    event_type_code: Option<String>,
}

impl AuditEventBuilder {
    /// Create a new builder with the given source observer reference.
    pub fn new(source_observer: impl Into<String>) -> Self {
        Self {
            action: None,
            outcome: None,
            outcome_desc: None,
            resource_type: None,
            resource_id: None,
            patient_reference: None,
            agent_who: None,
            agent_name: None,
            agent_requestor: true,
            source_observer: source_observer.into(),
            query_string: None,
            entity_details: Vec::new(),
            extra_entities: Vec::new(),
            event_type_system: None,
            event_type_code: None,
        }
    }

    /// Set the FHIR audit action.
    pub fn action(mut self, action: AuditAction) -> Self {
        self.action = Some(action);
        self
    }

    /// Set the outcome code (`"0"` = success, `"4"` = minor failure,
    /// `"8"` = serious failure, `"12"` = major failure).
    pub fn outcome(mut self, outcome: &str) -> Self {
        self.outcome = Some(outcome.to_string());
        self
    }

    /// Set a human-readable outcome description.
    pub fn outcome_desc(mut self, desc: impl Into<String>) -> Self {
        self.outcome_desc = Some(desc.into());
        self
    }

    /// Set the FHIR resource being acted on.
    pub fn resource(mut self, resource_type: &str, resource_id: &str) -> Self {
        self.resource_type = Some(resource_type.to_string());
        self.resource_id = Some(resource_id.to_string());
        self
    }

    /// Set the patient reference (e.g. `"Patient/123"`).
    pub fn patient(mut self, patient_ref: impl Into<String>) -> Self {
        self.patient_reference = Some(patient_ref.into());
        self
    }

    /// Set the agent (who performed the action).
    pub fn agent(mut self, who: impl Into<String>, name: Option<String>, requestor: bool) -> Self {
        self.agent_who = Some(who.into());
        self.agent_name = name;
        self.agent_requestor = requestor;
        self
    }

    /// Set the query string for search operations.
    pub fn query(mut self, query_string: impl Into<String>) -> Self {
        self.query_string = Some(query_string.into());
        self
    }

    /// Add a key-value detail to the primary resource entity.
    ///
    /// Details are serialized as `AuditEventEntityDetail` items. If no
    /// resource entity is set, a standalone entity is created to carry them.
    pub fn detail(mut self, name: &str, value: impl Into<String>) -> Self {
        self.entity_details.push((name.to_string(), value.into()));
        self
    }

    /// Add a pre-built entity beyond the standard resource and patient pair.
    pub fn entity(mut self, entity: AuditEventEntity) -> Self {
        self.extra_entities.push(entity);
        self
    }

    /// Override the event type coding (default: `audit-event-type` / `rest`).
    pub fn event_type(mut self, system: &str, code: &str) -> Self {
        self.event_type_system = Some(system.to_string());
        self.event_type_code = Some(code.to_string());
        self
    }

    /// Build the typed `AuditEvent`.
    pub fn build(self) -> AuditEvent {
        let has_patient = self.patient_reference.is_some();
        let audit_action = self.action.unwrap_or(AuditAction::Read);
        let profile_url = balp::select_profile(audit_action, has_patient);

        // Build entity details from accumulated (name, value) pairs
        let details = if self.entity_details.is_empty() {
            None
        } else {
            Some(
                self.entity_details
                    .iter()
                    .map(|(n, v)| entity_detail(n, v))
                    .collect(),
            )
        };

        // Entities
        let mut entities = Vec::new();

        // Entity: the FHIR resource being acted on
        if let (Some(rt), Some(rid)) = (&self.resource_type, &self.resource_id)
            && !rid.is_empty()
        {
            entities.push(AuditEventEntity {
                what: Some(reference(&format!("{rt}/{rid}"))),
                r#type: Some(coding(code_systems::AUDIT_ENTITY_TYPE, "2")),
                detail: details.clone(),
                ..Default::default()
            });
        }

        // If there are details but no resource entity, create a standalone entity
        if entities.is_empty() && details.is_some() {
            entities.push(AuditEventEntity {
                detail: details,
                ..Default::default()
            });
        }

        // Entity: patient (if resolved)
        if let Some(ref patient_ref) = self.patient_reference {
            entities.push(AuditEventEntity {
                what: Some(reference(patient_ref)),
                r#type: Some(coding(code_systems::AUDIT_ENTITY_TYPE, "1")),
                role: Some(coding(code_systems::OBJECT_ROLE, "1")),
                ..Default::default()
            });
        }

        // Extra entities added via entity()
        entities.extend(self.extra_entities);

        // Build the subtype coding (maps action code to restful-interaction)
        let subtype = self.action.map(|a| {
            let interaction = match a {
                AuditAction::Create => "create",
                AuditAction::Read => "read",
                AuditAction::Update => "update",
                AuditAction::Delete => "delete",
                AuditAction::Query => "search",
                // FHIR restful-interaction does not define "execute"; use "operation"
                // for non-CRUD execution semantics.
                AuditAction::Execute => "operation",
            };
            vec![coding(code_systems::RESTFUL_INTERACTION, interaction)]
        });

        // Event type: use override if set, otherwise default to "rest"
        let type_system = self
            .event_type_system
            .as_deref()
            .unwrap_or(code_systems::AUDIT_EVENT_TYPE);
        let type_code = self.event_type_code.as_deref().unwrap_or("rest");

        AuditEvent {
            id: Some(fhir_string(uuid::Uuid::new_v4().to_string())),
            meta: Some(Meta {
                profile: Some(vec![canonical(profile_url)]),
                ..Default::default()
            }),
            r#type: coding(type_system, type_code),
            subtype,
            action: self.action.map(|a| code(a.to_code())),
            recorded: instant_now(),
            outcome: self.outcome.map(code),
            outcome_desc: self.outcome_desc.map(fhir_string),
            agent: Some(vec![AuditEventAgent {
                who: self.agent_who.as_deref().map(reference),
                name: self.agent_name.map(fhir_string),
                requestor: boolean(self.agent_requestor),
                ..Default::default()
            }]),
            source: AuditEventSource {
                observer: reference(&self.source_observer),
                ..Default::default()
            },
            entity: if entities.is_empty() {
                None
            } else {
                Some(entities)
            },
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_minimal_build() {
        let event = AuditEventBuilder::new("Device/hfs").build();
        assert!(event.id.is_some());
        assert!(event.recorded.value.is_some());
        assert_eq!(
            event
                .source
                .observer
                .reference
                .as_ref()
                .and_then(|s| s.value.as_deref()),
            Some("Device/hfs")
        );
    }

    #[test]
    fn test_read_with_patient_selects_patient_read_profile() {
        let event = AuditEventBuilder::new("Device/hfs")
            .action(AuditAction::Read)
            .patient("Patient/123")
            .build();
        let profiles = event.meta.as_ref().unwrap().profile.as_ref().unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(
            profiles[0].value.as_deref(),
            Some(balp::profiles::PATIENT_READ)
        );
    }

    #[test]
    fn test_create_without_patient_selects_create_profile() {
        let event = AuditEventBuilder::new("Device/hfs")
            .action(AuditAction::Create)
            .build();
        let profiles = event.meta.as_ref().unwrap().profile.as_ref().unwrap();
        assert_eq!(profiles[0].value.as_deref(), Some(balp::profiles::CREATE));
    }

    #[test]
    fn test_action_and_outcome_set() {
        let event = AuditEventBuilder::new("Device/hfs")
            .action(AuditAction::Read)
            .outcome("0")
            .build();
        assert_eq!(
            event.action.as_ref().and_then(|a| a.value.as_deref()),
            Some("R")
        );
        assert_eq!(
            event.outcome.as_ref().and_then(|o| o.value.as_deref()),
            Some("0")
        );
    }

    #[test]
    fn test_outcome_desc() {
        let event = AuditEventBuilder::new("Device/hfs")
            .outcome_desc("Something went wrong")
            .build();
        assert_eq!(
            event.outcome_desc.as_ref().and_then(|s| s.value.as_deref()),
            Some("Something went wrong")
        );
    }

    #[test]
    fn test_agent_populated() {
        let event = AuditEventBuilder::new("Device/hfs")
            .agent("Practitioner/dr-smith", Some("Dr. Smith".to_string()), true)
            .build();
        let agent = &event.agent.as_ref().unwrap()[0];
        assert_eq!(
            agent
                .who
                .as_ref()
                .and_then(|w| w.reference.as_ref())
                .and_then(|s| s.value.as_deref()),
            Some("Practitioner/dr-smith")
        );
        assert_eq!(
            agent.name.as_ref().and_then(|s| s.value.as_deref()),
            Some("Dr. Smith")
        );
        assert_eq!(agent.requestor.value, Some(true));
    }

    #[test]
    fn test_resource_entity() {
        let event = AuditEventBuilder::new("Device/hfs")
            .resource("Patient", "123")
            .build();
        let entities = event.entity.as_ref().unwrap();
        assert_eq!(entities.len(), 1);
        assert_eq!(
            entities[0]
                .what
                .as_ref()
                .and_then(|w| w.reference.as_ref())
                .and_then(|s| s.value.as_deref()),
            Some("Patient/123")
        );
    }

    #[test]
    fn test_resource_and_patient_entities() {
        let event = AuditEventBuilder::new("Device/hfs")
            .resource("Observation", "obs-1")
            .patient("Patient/456")
            .build();
        let entities = event.entity.as_ref().unwrap();
        assert_eq!(entities.len(), 2);
        // First entity is the resource
        assert_eq!(
            entities[0]
                .what
                .as_ref()
                .and_then(|w| w.reference.as_ref())
                .and_then(|s| s.value.as_deref()),
            Some("Observation/obs-1")
        );
        // Second entity is the patient
        assert_eq!(
            entities[1]
                .what
                .as_ref()
                .and_then(|w| w.reference.as_ref())
                .and_then(|s| s.value.as_deref()),
            Some("Patient/456")
        );
    }

    #[test]
    fn test_no_entities_when_none_set() {
        let event = AuditEventBuilder::new("Device/hfs").build();
        assert!(event.entity.is_none());
    }

    #[test]
    fn test_subtype_for_read() {
        let event = AuditEventBuilder::new("Device/hfs")
            .action(AuditAction::Read)
            .build();
        let subtypes = event.subtype.as_ref().unwrap();
        assert_eq!(
            subtypes[0].code.as_ref().and_then(|c| c.value.as_deref()),
            Some("read")
        );
    }

    #[test]
    fn test_subtype_for_create() {
        let event = AuditEventBuilder::new("Device/hfs")
            .action(AuditAction::Create)
            .build();
        let subtypes = event.subtype.as_ref().unwrap();
        assert_eq!(
            subtypes[0].code.as_ref().and_then(|c| c.value.as_deref()),
            Some("create")
        );
    }

    #[test]
    fn test_subtype_for_query() {
        let event = AuditEventBuilder::new("Device/hfs")
            .action(AuditAction::Query)
            .build();
        let subtypes = event.subtype.as_ref().unwrap();
        assert_eq!(
            subtypes[0].code.as_ref().and_then(|c| c.value.as_deref()),
            Some("search")
        );
        assert_eq!(
            event.action.as_ref().and_then(|a| a.value.as_deref()),
            Some("E")
        );
        let profiles = event.meta.as_ref().unwrap().profile.as_ref().unwrap();
        assert_eq!(profiles[0].value.as_deref(), Some(balp::profiles::QUERY));
    }

    #[test]
    fn test_uuid_generated() {
        let event = AuditEventBuilder::new("Device/hfs").build();
        let id = event.id.as_ref().and_then(|s| s.value.as_deref()).unwrap();
        // UUID v4 format: 8-4-4-4-12
        assert_eq!(id.len(), 36);
        assert_eq!(&id[8..9], "-");
    }

    #[test]
    fn test_empty_resource_id_skips_entity() {
        let event = AuditEventBuilder::new("Device/hfs")
            .resource("Patient", "")
            .build();
        assert!(event.entity.is_none());
    }

    #[test]
    fn test_detail_attached_to_resource_entity() {
        let event = AuditEventBuilder::new("Device/hfs")
            .resource("Patient", "123")
            .detail("job-id", "abc-def")
            .detail("count", "42")
            .build();
        let entities = event.entity.as_ref().unwrap();
        let details = entities[0].detail.as_ref().unwrap();
        assert_eq!(details.len(), 2);
        assert_eq!(details[0].r#type.value.as_deref(), Some("job-id"));
        assert_eq!(details[1].r#type.value.as_deref(), Some("count"));
    }

    #[test]
    fn test_detail_without_resource_creates_standalone_entity() {
        let event = AuditEventBuilder::new("Device/hfs")
            .detail("phase", "start")
            .build();
        let entities = event.entity.as_ref().unwrap();
        assert_eq!(entities.len(), 1);
        assert!(entities[0].what.is_none());
        let details = entities[0].detail.as_ref().unwrap();
        assert_eq!(details[0].r#type.value.as_deref(), Some("phase"));
    }

    #[test]
    fn test_custom_entity_appended() {
        use helios_fhir::r4::AuditEventEntity;

        let custom = AuditEventEntity {
            what: Some(reference("Job/export-1")),
            ..Default::default()
        };
        let event = AuditEventBuilder::new("Device/hfs")
            .resource("Patient", "123")
            .entity(custom)
            .build();
        let entities = event.entity.as_ref().unwrap();
        assert_eq!(entities.len(), 2);
        assert_eq!(
            entities[1]
                .what
                .as_ref()
                .and_then(|w| w.reference.as_ref())
                .and_then(|s| s.value.as_deref()),
            Some("Job/export-1")
        );
    }

    #[test]
    fn test_event_type_override() {
        let event = AuditEventBuilder::new("Device/hfs")
            .event_type(
                "http://terminology.hl7.org/CodeSystem/audit-event-type",
                "object",
            )
            .build();
        assert_eq!(
            event.r#type.code.as_ref().and_then(|c| c.value.as_deref()),
            Some("object")
        );
    }

    #[test]
    fn test_subtype_for_execute_is_operation() {
        let event = AuditEventBuilder::new("Device/hfs")
            .action(AuditAction::Execute)
            .build();
        let subtypes = event.subtype.as_ref().unwrap();
        assert_eq!(
            subtypes[0].code.as_ref().and_then(|c| c.value.as_deref()),
            Some("operation")
        );
    }

    #[test]
    fn test_event_type_default_is_rest() {
        let event = AuditEventBuilder::new("Device/hfs").build();
        assert_eq!(
            event.r#type.code.as_ref().and_then(|c| c.value.as_deref()),
            Some("rest")
        );
    }
}
