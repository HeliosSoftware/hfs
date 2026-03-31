//! Fluent builder for FHIR `AuditEvent` resources.
//!
//! Uses the typed `helios_fhir::r4::AuditEvent` struct with convenience
//! helpers from [`crate::helpers`] and BALP profile selection from
//! [`crate::balp`].

use helios_fhir::r4::{AuditEvent, AuditEventAgent, AuditEventEntity, AuditEventSource, Meta};

use crate::balp::{self, code_systems};
use crate::helpers::*;

/// Builder for constructing BALP-compliant `AuditEvent` resources.
///
/// # Example
///
/// ```rust,ignore
/// let event = AuditEventBuilder::new("Device/hfs")
///     .action("R")
///     .outcome("0")
///     .resource("Patient", "123")
///     .patient("Patient/123")
///     .agent("Practitioner/dr-smith", None, true)
///     .build();
/// ```
pub struct AuditEventBuilder {
    action: Option<String>,
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
        }
    }

    /// Set the FHIR action code (`"C"`, `"R"`, `"U"`, `"D"`, `"E"`).
    pub fn action(mut self, action: &str) -> Self {
        self.action = Some(action.to_string());
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

    /// Build the typed `AuditEvent`.
    pub fn build(self) -> AuditEvent {
        let has_patient = self.patient_reference.is_some();
        let audit_action = balp::action_from_code(self.action.as_deref().unwrap_or("R"));
        let profile_url = balp::select_profile(audit_action, has_patient);

        // Entities
        let mut entities = Vec::new();

        // Entity: the FHIR resource being acted on
        if let (Some(rt), Some(rid)) = (&self.resource_type, &self.resource_id) {
            if !rid.is_empty() {
                entities.push(AuditEventEntity {
                    what: Some(reference(&format!("{rt}/{rid}"))),
                    r#type: Some(coding(code_systems::AUDIT_ENTITY_TYPE, "2")),
                    ..Default::default()
                });
            }
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

        // Build the subtype coding (maps action code to restful-interaction)
        let subtype = self.action.as_deref().map(|a| {
            let interaction = match a {
                "C" => "create",
                "R" => "read",
                "U" => "update",
                "D" => "delete",
                _ => "execute",
            };
            vec![coding(code_systems::RESTFUL_INTERACTION, interaction)]
        });

        AuditEvent {
            id: Some(fhir_string(uuid::Uuid::new_v4().to_string())),
            meta: Some(Meta {
                profile: Some(vec![canonical(profile_url)]),
                ..Default::default()
            }),
            r#type: coding(code_systems::AUDIT_EVENT_TYPE, "rest"),
            subtype,
            action: self.action.map(code),
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
            .action("R")
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
        let event = AuditEventBuilder::new("Device/hfs").action("C").build();
        let profiles = event.meta.as_ref().unwrap().profile.as_ref().unwrap();
        assert_eq!(profiles[0].value.as_deref(), Some(balp::profiles::CREATE));
    }

    #[test]
    fn test_action_and_outcome_set() {
        let event = AuditEventBuilder::new("Device/hfs")
            .action("R")
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
        let event = AuditEventBuilder::new("Device/hfs").action("R").build();
        let subtypes = event.subtype.as_ref().unwrap();
        assert_eq!(
            subtypes[0].code.as_ref().and_then(|c| c.value.as_deref()),
            Some("read")
        );
    }

    #[test]
    fn test_subtype_for_create() {
        let event = AuditEventBuilder::new("Device/hfs").action("C").build();
        let subtypes = event.subtype.as_ref().unwrap();
        assert_eq!(
            subtypes[0].code.as_ref().and_then(|c| c.value.as_deref()),
            Some("create")
        );
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
}
