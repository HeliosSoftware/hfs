use serde::{Serialize, Deserialize};
use crate::{Element, DecimalElement};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Account {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Vec<Reference>>,
    #[serde(rename = "servicePeriod", skip_serializing_if = "Option::is_none")]
    pub service_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Vec<AccountCoverage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guarantor: Option<Vec<AccountGuarantor>>,
    pub party: Reference,
    #[serde(rename = "onHold", skip_serializing_if = "Option::is_none")]
    pub on_hold: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Reference>,
}


/// Choice of types for the subject[x] field in ActivityDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ActivityDefinitionSubjectChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

/// Choice of types for the timing[x] field in ActivityDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ActivityDefinitionTimingChoice {
    /// Variant accepting the Timing type.
    #[serde(rename = "timingTiming")]
    Timing(Timing),
    /// Variant accepting the DateTime type.
    #[serde(rename = "timingDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Age type.
    #[serde(rename = "timingAge")]
    Age(Age),
    /// Variant accepting the Period type.
    #[serde(rename = "timingPeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[serde(rename = "timingRange")]
    Range(Range),
    /// Variant accepting the Duration type.
    #[serde(rename = "timingDuration")]
    Duration(Duration),
}

/// Choice of types for the product[x] field in ActivityDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ActivityDefinitionProductChoice {
    /// Variant accepting the Reference type.
    #[serde(rename = "productReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "productCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ActivityDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(flatten)]
    pub subject: Option<ActivityDefinitionSubjectChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate", skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate", skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod", skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact", skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Canonical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Code>,
    #[serde(rename = "doNotPerform", skip_serializing_if = "Option::is_none")]
    pub do_not_perform: Option<Boolean>,
    #[serde(flatten)]
    pub timing: Option<ActivityDefinitionTimingChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant: Option<Vec<ActivityDefinitionParticipant>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
    #[serde(flatten)]
    pub product: Option<ActivityDefinitionProductChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dosage: Option<Vec<Dosage>>,
    #[serde(rename = "bodySite", skip_serializing_if = "Option::is_none")]
    pub body_site: Option<Vec<CodeableConcept>>,
    #[serde(rename = "specimenRequirement", skip_serializing_if = "Option::is_none")]
    pub specimen_requirement: Option<Vec<Reference>>,
    #[serde(rename = "observationRequirement", skip_serializing_if = "Option::is_none")]
    pub observation_requirement: Option<Vec<Reference>>,
    #[serde(rename = "observationResultRequirement", skip_serializing_if = "Option::is_none")]
    pub observation_result_requirement: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<Canonical>,
    #[serde(rename = "dynamicValue", skip_serializing_if = "Option::is_none")]
    pub dynamic_value: Option<Vec<ActivityDefinitionDynamicValue>>,
    pub path: String,
    pub expression: Expression,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct AdverseEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    pub actuality: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<CodeableConcept>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detected: Option<DateTime>,
    #[serde(rename = "recordedDate", skip_serializing_if = "Option::is_none")]
    pub recorded_date: Option<DateTime>,
    #[serde(rename = "resultingCondition", skip_serializing_if = "Option::is_none")]
    pub resulting_condition: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seriousness: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorder: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contributor: Option<Vec<Reference>>,
    #[serde(rename = "suspectEntity", skip_serializing_if = "Option::is_none")]
    pub suspect_entity: Option<Vec<AdverseEventSuspectEntity>>,
    pub instance: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causality: Option<Vec<AdverseEventSuspectEntityCausality>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assessment: Option<CodeableConcept>,
    #[serde(rename = "productRelatedness", skip_serializing_if = "Option::is_none")]
    pub product_relatedness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(rename = "subjectMedicalHistory", skip_serializing_if = "Option::is_none")]
    pub subject_medical_history: Option<Vec<Reference>>,
    #[serde(rename = "referenceDocument", skip_serializing_if = "Option::is_none")]
    pub reference_document: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub study: Option<Vec<Reference>>,
}


/// Choice of types for the onset[x] field in AllergyIntolerance
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum AllergyIntoleranceOnsetChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "onsetDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Age type.
    #[serde(rename = "onsetAge")]
    Age(Age),
    /// Variant accepting the Period type.
    #[serde(rename = "onsetPeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[serde(rename = "onsetRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[serde(rename = "onsetString")]
    String(String),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct AllergyIntolerance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "clinicalStatus", skip_serializing_if = "Option::is_none")]
    pub clinical_status: Option<CodeableConcept>,
    #[serde(rename = "verificationStatus", skip_serializing_if = "Option::is_none")]
    pub verification_status: Option<CodeableConcept>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<Code>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub criticality: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    pub patient: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(flatten)]
    pub onset: Option<AllergyIntoleranceOnsetChoice>,
    #[serde(rename = "recordedDate", skip_serializing_if = "Option::is_none")]
    pub recorded_date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorder: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asserter: Option<Reference>,
    #[serde(rename = "lastOccurrence", skip_serializing_if = "Option::is_none")]
    pub last_occurrence: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reaction: Option<Vec<AllergyIntoleranceReaction>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substance: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifestation: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<Code>,
    #[serde(rename = "exposureRoute", skip_serializing_if = "Option::is_none")]
    pub exposure_route: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Appointment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "cancelationReason", skip_serializing_if = "Option::is_none")]
    pub cancelation_reason: Option<CodeableConcept>,
    #[serde(rename = "serviceCategory", skip_serializing_if = "Option::is_none")]
    pub service_category: Option<Vec<CodeableConcept>>,
    #[serde(rename = "serviceType", skip_serializing_if = "Option::is_none")]
    pub service_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specialty: Option<Vec<CodeableConcept>>,
    #[serde(rename = "appointmentType", skip_serializing_if = "Option::is_none")]
    pub appointment_type: Option<CodeableConcept>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<UnsignedInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "supportingInformation", skip_serializing_if = "Option::is_none")]
    pub supporting_information: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<Instant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<Instant>,
    #[serde(rename = "minutesDuration", skip_serializing_if = "Option::is_none")]
    pub minutes_duration: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(rename = "patientInstruction", skip_serializing_if = "Option::is_none")]
    pub patient_instruction: Option<String>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant: Option<Vec<AppointmentParticipant>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(rename = "requestedPeriod", skip_serializing_if = "Option::is_none")]
    pub requested_period: Option<Vec<Period>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct AppointmentResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub appointment: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<Instant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<Instant>,
    #[serde(rename = "participantType", skip_serializing_if = "Option::is_none")]
    pub participant_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Reference>,
    #[serde(rename = "participantStatus")]
    pub participant_status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}


/// Choice of types for the value[x] field in AuditEvent
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum AuditEventValueChoice {
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct AuditEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Coding,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtype: Option<Vec<Coding>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    pub recorded: Instant,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Code>,
    #[serde(rename = "outcomeDesc", skip_serializing_if = "Option::is_none")]
    pub outcome_desc: Option<String>,
    #[serde(rename = "purposeOfEvent", skip_serializing_if = "Option::is_none")]
    pub purpose_of_event: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<Vec<AuditEventAgent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub who: Option<Reference>,
    #[serde(rename = "altId", skip_serializing_if = "Option::is_none")]
    pub alt_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub requestor: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<Vec<Uri>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<AuditEventAgentNetwork>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(rename = "purposeOfUse", skip_serializing_if = "Option::is_none")]
    pub purpose_of_use: Option<Vec<CodeableConcept>>,
    pub source: AuditEventSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    pub observer: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<Vec<AuditEventEntity>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub what: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<Coding>,
    #[serde(rename = "securityLabel", skip_serializing_if = "Option::is_none")]
    pub security_label: Option<Vec<Coding>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<Base64Binary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<AuditEventEntityDetail>>,
    #[serde(flatten)]
    pub value: Option<AuditEventValueChoice>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Basic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub code: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<Date>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Binary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(rename = "contentType")]
    pub content_type: Code,
    #[serde(rename = "securityContext", skip_serializing_if = "Option::is_none")]
    pub security_context: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Base64Binary>,
}


/// Choice of types for the collected[x] field in BiologicallyDerivedProduct
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum BiologicallyDerivedProductCollectedChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "collectedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "collectedPeriod")]
    Period(Period),
}

/// Choice of types for the time[x] field in BiologicallyDerivedProduct
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum BiologicallyDerivedProductTimeChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "timeDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "timePeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct BiologicallyDerivedProduct {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "productCategory", skip_serializing_if = "Option::is_none")]
    pub product_category: Option<Code>,
    #[serde(rename = "productCode", skip_serializing_if = "Option::is_none")]
    pub product_code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<BiologicallyDerivedProductCollection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collector: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Reference>,
    #[serde(flatten)]
    pub collected: Option<BiologicallyDerivedProductCollectedChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processing: Option<Vec<BiologicallyDerivedProductProcessing>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additive: Option<Reference>,
    #[serde(flatten)]
    pub time: Option<BiologicallyDerivedProductTimeChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manipulation: Option<BiologicallyDerivedProductManipulation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<Vec<BiologicallyDerivedProductStorage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<Period>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct BodyStructure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub morphology: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<CodeableConcept>,
    #[serde(rename = "locationQualifier", skip_serializing_if = "Option::is_none")]
    pub location_qualifier: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<Vec<Attachment>>,
    pub patient: Reference,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Bundle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<Instant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<UnsignedInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<Vec<BundleLink>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub relation: String,
    pub url: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<Vec<BundleEntry>>,
    #[serde(rename = "fullUrl", skip_serializing_if = "Option::is_none")]
    pub full_url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Box<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<BundleEntrySearch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<BundleEntryRequest>,
    pub method: Code,
    #[serde(rename = "ifNoneMatch", skip_serializing_if = "Option::is_none")]
    pub if_none_match: Option<String>,
    #[serde(rename = "ifModifiedSince", skip_serializing_if = "Option::is_none")]
    pub if_modified_since: Option<Instant>,
    #[serde(rename = "ifMatch", skip_serializing_if = "Option::is_none")]
    pub if_match: Option<String>,
    #[serde(rename = "ifNoneExist", skip_serializing_if = "Option::is_none")]
    pub if_none_exist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<BundleEntryResponse>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(rename = "lastModified", skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<Instant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Box<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CapabilityStatement {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    pub date: DateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    pub kind: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imports: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub software: Option<CapabilityStatementSoftware>,
    #[serde(rename = "releaseDate", skip_serializing_if = "Option::is_none")]
    pub release_date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implementation: Option<CapabilityStatementImplementation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custodian: Option<Reference>,
    #[serde(rename = "fhirVersion")]
    pub fhir_version: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<Vec<Code>>,
    #[serde(rename = "patchFormat", skip_serializing_if = "Option::is_none")]
    pub patch_format: Option<Vec<Code>>,
    #[serde(rename = "implementationGuide", skip_serializing_if = "Option::is_none")]
    pub implementation_guide: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rest: Option<Vec<CapabilityStatementRest>>,
    pub mode: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security: Option<CapabilityStatementRestSecurity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cors: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Vec<CapabilityStatementRestResource>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Canonical>,
    #[serde(rename = "supportedProfile", skip_serializing_if = "Option::is_none")]
    pub supported_profile: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction: Option<Vec<CapabilityStatementRestResourceInteraction>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub versioning: Option<Code>,
    #[serde(rename = "readHistory", skip_serializing_if = "Option::is_none")]
    pub read_history: Option<Boolean>,
    #[serde(rename = "updateCreate", skip_serializing_if = "Option::is_none")]
    pub update_create: Option<Boolean>,
    #[serde(rename = "conditionalCreate", skip_serializing_if = "Option::is_none")]
    pub conditional_create: Option<Boolean>,
    #[serde(rename = "conditionalRead", skip_serializing_if = "Option::is_none")]
    pub conditional_read: Option<Code>,
    #[serde(rename = "conditionalUpdate", skip_serializing_if = "Option::is_none")]
    pub conditional_update: Option<Boolean>,
    #[serde(rename = "conditionalDelete", skip_serializing_if = "Option::is_none")]
    pub conditional_delete: Option<Code>,
    #[serde(rename = "referencePolicy", skip_serializing_if = "Option::is_none")]
    pub reference_policy: Option<Vec<Code>>,
    #[serde(rename = "searchInclude", skip_serializing_if = "Option::is_none")]
    pub search_include: Option<Vec<String>>,
    #[serde(rename = "searchRevInclude", skip_serializing_if = "Option::is_none")]
    pub search_rev_include: Option<Vec<String>>,
    #[serde(rename = "searchParam", skip_serializing_if = "Option::is_none")]
    pub search_param: Option<Vec<CapabilityStatementRestResourceSearchParam>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<Canonical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<Vec<CapabilityStatementRestResourceOperation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compartment: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messaging: Option<Vec<CapabilityStatementMessaging>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<CapabilityStatementMessagingEndpoint>>,
    pub protocol: Coding,
    pub address: Url,
    #[serde(rename = "reliableCache", skip_serializing_if = "Option::is_none")]
    pub reliable_cache: Option<UnsignedInt>,
    #[serde(rename = "supportedMessage", skip_serializing_if = "Option::is_none")]
    pub supported_message: Option<Vec<CapabilityStatementMessagingSupportedMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<Vec<CapabilityStatementDocument>>,
}


/// Choice of types for the scheduled[x] field in CarePlan
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CarePlanScheduledChoice {
    /// Variant accepting the Timing type.
    #[serde(rename = "scheduledTiming")]
    Timing(Timing),
    /// Variant accepting the Period type.
    #[serde(rename = "scheduledPeriod")]
    Period(Period),
    /// Variant accepting the String type.
    #[serde(rename = "scheduledString")]
    String(String),
}

/// Choice of types for the product[x] field in CarePlan
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CarePlanProductChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "productCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "productReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CarePlan {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical", skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri", skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaces: Option<Vec<Reference>>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    pub intent: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contributor: Option<Vec<Reference>>,
    #[serde(rename = "careTeam", skip_serializing_if = "Option::is_none")]
    pub care_team: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addresses: Option<Vec<Reference>>,
    #[serde(rename = "supportingInfo", skip_serializing_if = "Option::is_none")]
    pub supporting_info: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<Vec<CarePlanActivity>>,
    #[serde(rename = "outcomeCodeableConcept", skip_serializing_if = "Option::is_none")]
    pub outcome_codeable_concept: Option<Vec<CodeableConcept>>,
    #[serde(rename = "outcomeReference", skip_serializing_if = "Option::is_none")]
    pub outcome_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<CarePlanActivityDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "statusReason", skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<CodeableConcept>,
    #[serde(rename = "doNotPerform", skip_serializing_if = "Option::is_none")]
    pub do_not_perform: Option<Boolean>,
    #[serde(flatten)]
    pub scheduled: Option<CarePlanScheduledChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<Reference>>,
    #[serde(flatten)]
    pub product: Option<CarePlanProductChoice>,
    #[serde(rename = "dailyAmount", skip_serializing_if = "Option::is_none")]
    pub daily_amount: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CareTeam {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant: Option<Vec<CareTeamParticipant>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member: Option<Reference>,
    #[serde(rename = "onBehalfOf", skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<Reference>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "managingOrganization", skip_serializing_if = "Option::is_none")]
    pub managing_organization: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CatalogEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    pub orderable: Boolean,
    #[serde(rename = "referencedItem")]
    pub referenced_item: Reference,
    #[serde(rename = "additionalIdentifier", skip_serializing_if = "Option::is_none")]
    pub additional_identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(rename = "validityPeriod", skip_serializing_if = "Option::is_none")]
    pub validity_period: Option<Period>,
    #[serde(rename = "validTo", skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<DateTime>,
    #[serde(rename = "lastUpdated", skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<DateTime>,
    #[serde(rename = "additionalCharacteristic", skip_serializing_if = "Option::is_none")]
    pub additional_characteristic: Option<Vec<CodeableConcept>>,
    #[serde(rename = "additionalClassification", skip_serializing_if = "Option::is_none")]
    pub additional_classification: Option<Vec<CodeableConcept>>,
    #[serde(rename = "relatedEntry", skip_serializing_if = "Option::is_none")]
    pub related_entry: Option<Vec<CatalogEntryRelatedEntry>>,
    pub relationtype: Code,
    pub item: Reference,
}


/// Choice of types for the occurrence[x] field in ChargeItem
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ChargeItemOccurrenceChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

/// Choice of types for the product[x] field in ChargeItem
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ChargeItemProductChoice {
    /// Variant accepting the Reference type.
    #[serde(rename = "productReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "productCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ChargeItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "definitionUri", skip_serializing_if = "Option::is_none")]
    pub definition_uri: Option<Vec<Uri>>,
    #[serde(rename = "definitionCanonical", skip_serializing_if = "Option::is_none")]
    pub definition_canonical: Option<Vec<Canonical>>,
    pub status: Code,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    pub code: CodeableConcept,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Reference>,
    #[serde(flatten)]
    pub occurrence: Option<ChargeItemOccurrenceChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<ChargeItemPerformer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
    #[serde(rename = "performingOrganization", skip_serializing_if = "Option::is_none")]
    pub performing_organization: Option<Reference>,
    #[serde(rename = "requestingOrganization", skip_serializing_if = "Option::is_none")]
    pub requesting_organization: Option<Reference>,
    #[serde(rename = "costCenter", skip_serializing_if = "Option::is_none")]
    pub cost_center: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bodysite: Option<Vec<CodeableConcept>>,
    #[serde(rename = "factorOverride", skip_serializing_if = "Option::is_none")]
    pub factor_override: Option<Decimal>,
    #[serde(rename = "priceOverride", skip_serializing_if = "Option::is_none")]
    pub price_override: Option<Money>,
    #[serde(rename = "overrideReason", skip_serializing_if = "Option::is_none")]
    pub override_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterer: Option<Reference>,
    #[serde(rename = "enteredDate", skip_serializing_if = "Option::is_none")]
    pub entered_date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<Vec<Reference>>,
    #[serde(flatten)]
    pub product: Option<ChargeItemProductChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "supportingInformation", skip_serializing_if = "Option::is_none")]
    pub supporting_information: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ChargeItemDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(rename = "derivedFromUri", skip_serializing_if = "Option::is_none")]
    pub derived_from_uri: Option<Vec<Uri>>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaces: Option<Vec<Canonical>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate", skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate", skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod", skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applicability: Option<Vec<ChargeItemDefinitionApplicability>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(rename = "propertyGroup", skip_serializing_if = "Option::is_none")]
    pub property_group: Option<Vec<ChargeItemDefinitionPropertyGroup>>,
    #[serde(rename = "priceComponent", skip_serializing_if = "Option::is_none")]
    pub price_component: Option<Vec<ChargeItemDefinitionPropertyGroupPriceComponent>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Money>,
}


/// Choice of types for the timing[x] field in Claim
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ClaimTimingChoice {
    /// Variant accepting the Date type.
    #[serde(rename = "timingDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "timingPeriod")]
    Period(Period),
}

/// Choice of types for the value[x] field in Claim
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ClaimValueChoice {
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Attachment type.
    #[serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
}

/// Choice of types for the diagnosis[x] field in Claim
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ClaimDiagnosisChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "diagnosisCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "diagnosisReference")]
    Reference(Reference),
}

/// Choice of types for the procedure[x] field in Claim
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ClaimProcedureChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "procedureCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "procedureReference")]
    Reference(Reference),
}

/// Choice of types for the location[x] field in Claim
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ClaimLocationChoice {
    /// Variant accepting the Address type.
    #[serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[serde(rename = "locationReference")]
    Reference(Reference),
}

/// Choice of types for the serviced[x] field in Claim
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ClaimServicedChoice {
    /// Variant accepting the Date type.
    #[serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "servicedPeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Claim {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "subType", skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<CodeableConcept>,
    #[serde(rename = "use")]
    pub r#use: Code,
    pub patient: Reference,
    #[serde(rename = "billablePeriod", skip_serializing_if = "Option::is_none")]
    pub billable_period: Option<Period>,
    pub created: DateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterer: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurer: Option<Reference>,
    pub provider: Reference,
    pub priority: CodeableConcept,
    #[serde(rename = "fundsReserve", skip_serializing_if = "Option::is_none")]
    pub funds_reserve: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related: Option<Vec<ClaimRelated>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prescription: Option<Reference>,
    #[serde(rename = "originalPrescription", skip_serializing_if = "Option::is_none")]
    pub original_prescription: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payee: Option<ClaimPayee>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referral: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facility: Option<Reference>,
    #[serde(rename = "careTeam", skip_serializing_if = "Option::is_none")]
    pub care_team: Option<Vec<ClaimCareTeam>>,
    pub sequence: PositiveInt,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsible: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualification: Option<CodeableConcept>,
    #[serde(rename = "supportingInfo", skip_serializing_if = "Option::is_none")]
    pub supporting_info: Option<Vec<ClaimSupportingInfo>>,
    pub category: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(flatten)]
    pub timing: Option<ClaimTimingChoice>,
    #[serde(flatten)]
    pub value: Option<ClaimValueChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<Vec<ClaimDiagnosis>>,
    #[serde(rename = "onAdmission", skip_serializing_if = "Option::is_none")]
    pub on_admission: Option<CodeableConcept>,
    #[serde(rename = "packageCode", skip_serializing_if = "Option::is_none")]
    pub package_code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure: Option<Vec<ClaimProcedure>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurance: Option<Vec<ClaimInsurance>>,
    pub focal: Boolean,
    pub coverage: Reference,
    #[serde(rename = "businessArrangement", skip_serializing_if = "Option::is_none")]
    pub business_arrangement: Option<String>,
    #[serde(rename = "preAuthRef", skip_serializing_if = "Option::is_none")]
    pub pre_auth_ref: Option<Vec<String>>,
    #[serde(rename = "claimResponse", skip_serializing_if = "Option::is_none")]
    pub claim_response: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accident: Option<ClaimAccident>,
    #[serde(flatten)]
    pub location: Option<ClaimLocationChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<ClaimItem>>,
    #[serde(rename = "careTeamSequence", skip_serializing_if = "Option::is_none")]
    pub care_team_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "diagnosisSequence", skip_serializing_if = "Option::is_none")]
    pub diagnosis_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "procedureSequence", skip_serializing_if = "Option::is_none")]
    pub procedure_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "informationSequence", skip_serializing_if = "Option::is_none")]
    pub information_sequence: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revenue: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode", skip_serializing_if = "Option::is_none")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(flatten)]
    pub serviced: Option<ClaimServicedChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice", skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(rename = "bodySite", skip_serializing_if = "Option::is_none")]
    pub body_site: Option<CodeableConcept>,
    #[serde(rename = "subSite", skip_serializing_if = "Option::is_none")]
    pub sub_site: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<ClaimItemDetail>>,
    #[serde(rename = "subDetail", skip_serializing_if = "Option::is_none")]
    pub sub_detail: Option<Vec<ClaimItemDetailSubDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<Money>,
}


/// Choice of types for the serviced[x] field in ClaimResponse
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ClaimResponseServicedChoice {
    /// Variant accepting the Date type.
    #[serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "servicedPeriod")]
    Period(Period),
}

/// Choice of types for the location[x] field in ClaimResponse
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ClaimResponseLocationChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "locationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Address type.
    #[serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[serde(rename = "locationReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ClaimResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "subType", skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<CodeableConcept>,
    #[serde(rename = "use")]
    pub r#use: Code,
    pub patient: Reference,
    pub created: DateTime,
    pub insurer: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requestor: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<Reference>,
    pub outcome: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disposition: Option<String>,
    #[serde(rename = "preAuthRef", skip_serializing_if = "Option::is_none")]
    pub pre_auth_ref: Option<String>,
    #[serde(rename = "preAuthPeriod", skip_serializing_if = "Option::is_none")]
    pub pre_auth_period: Option<Period>,
    #[serde(rename = "payeeType", skip_serializing_if = "Option::is_none")]
    pub payee_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<ClaimResponseItem>>,
    #[serde(rename = "itemSequence")]
    pub item_sequence: PositiveInt,
    #[serde(rename = "noteNumber", skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    pub category: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<ClaimResponseItemDetail>>,
    #[serde(rename = "detailSequence")]
    pub detail_sequence: PositiveInt,
    #[serde(rename = "subDetail", skip_serializing_if = "Option::is_none")]
    pub sub_detail: Option<Vec<ClaimResponseItemDetailSubDetail>>,
    #[serde(rename = "subDetailSequence")]
    pub sub_detail_sequence: PositiveInt,
    #[serde(rename = "addItem", skip_serializing_if = "Option::is_none")]
    pub add_item: Option<Vec<ClaimResponseAddItem>>,
    #[serde(rename = "subdetailSequence", skip_serializing_if = "Option::is_none")]
    pub subdetail_sequence: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<Vec<Reference>>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode", skip_serializing_if = "Option::is_none")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(flatten)]
    pub serviced: Option<ClaimResponseServicedChoice>,
    #[serde(flatten)]
    pub location: Option<ClaimResponseLocationChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice", skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(rename = "bodySite", skip_serializing_if = "Option::is_none")]
    pub body_site: Option<CodeableConcept>,
    #[serde(rename = "subSite", skip_serializing_if = "Option::is_none")]
    pub sub_site: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<Vec<ClaimResponseTotal>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment: Option<ClaimResponsePayment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjustment: Option<Money>,
    #[serde(rename = "adjustmentReason", skip_serializing_if = "Option::is_none")]
    pub adjustment_reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<Date>,
    #[serde(rename = "fundsReserve", skip_serializing_if = "Option::is_none")]
    pub funds_reserve: Option<CodeableConcept>,
    #[serde(rename = "formCode", skip_serializing_if = "Option::is_none")]
    pub form_code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form: Option<Attachment>,
    #[serde(rename = "processNote", skip_serializing_if = "Option::is_none")]
    pub process_note: Option<Vec<ClaimResponseProcessNote>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<PositiveInt>,
    #[serde(rename = "communicationRequest", skip_serializing_if = "Option::is_none")]
    pub communication_request: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurance: Option<Vec<ClaimResponseInsurance>>,
    pub sequence: PositiveInt,
    pub focal: Boolean,
    pub coverage: Reference,
    #[serde(rename = "businessArrangement", skip_serializing_if = "Option::is_none")]
    pub business_arrangement: Option<String>,
    #[serde(rename = "claimResponse", skip_serializing_if = "Option::is_none")]
    pub claim_response: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Vec<ClaimResponseError>>,
    pub code: CodeableConcept,
}


/// Choice of types for the effective[x] field in ClinicalImpression
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ClinicalImpressionEffectiveChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "effectivePeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ClinicalImpression {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "statusReason", skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(flatten)]
    pub effective: Option<ClinicalImpressionEffectiveChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assessor: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub investigation: Option<Vec<ClinicalImpressionInvestigation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<Vec<Uri>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finding: Option<Vec<ClinicalImpressionFinding>>,
    #[serde(rename = "itemCodeableConcept", skip_serializing_if = "Option::is_none")]
    pub item_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "itemReference", skip_serializing_if = "Option::is_none")]
    pub item_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub basis: Option<String>,
    #[serde(rename = "prognosisCodeableConcept", skip_serializing_if = "Option::is_none")]
    pub prognosis_codeable_concept: Option<Vec<CodeableConcept>>,
    #[serde(rename = "prognosisReference", skip_serializing_if = "Option::is_none")]
    pub prognosis_reference: Option<Vec<Reference>>,
    #[serde(rename = "supportingInfo", skip_serializing_if = "Option::is_none")]
    pub supporting_info: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}


/// Choice of types for the value[x] field in CodeSystem
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CodeSystemValueChoice {
    /// Variant accepting the Code type.
    #[serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CodeSystem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "caseSensitive", skip_serializing_if = "Option::is_none")]
    pub case_sensitive: Option<Boolean>,
    #[serde(rename = "valueSet", skip_serializing_if = "Option::is_none")]
    pub value_set: Option<Canonical>,
    #[serde(rename = "hierarchyMeaning", skip_serializing_if = "Option::is_none")]
    pub hierarchy_meaning: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compositional: Option<Boolean>,
    #[serde(rename = "versionNeeded", skip_serializing_if = "Option::is_none")]
    pub version_needed: Option<Boolean>,
    pub content: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supplements: Option<Canonical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<UnsignedInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Vec<CodeSystemFilter>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator: Option<Vec<Code>>,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property: Option<Vec<CodeSystemProperty>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<Uri>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept: Option<Vec<CodeSystemConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub designation: Option<Vec<CodeSystemConceptDesignation>>,
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub r#use: Option<Coding>,
}


/// Choice of types for the content[x] field in Communication
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CommunicationContentChoice {
    /// Variant accepting the String type.
    #[serde(rename = "contentString")]
    String(String),
    /// Variant accepting the Attachment type.
    #[serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "contentReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Communication {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical", skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri", skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    #[serde(rename = "inResponseTo", skip_serializing_if = "Option::is_none")]
    pub in_response_to: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReason", skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub medium: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub received: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<Reference>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Vec<CommunicationPayload>>,
    #[serde(flatten)]
    pub content: Option<CommunicationContentChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}


/// Choice of types for the content[x] field in CommunicationRequest
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CommunicationRequestContentChoice {
    /// Variant accepting the String type.
    #[serde(rename = "contentString")]
    String(String),
    /// Variant accepting the Attachment type.
    #[serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "contentReference")]
    Reference(Reference),
}

/// Choice of types for the occurrence[x] field in CommunicationRequest
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CommunicationRequestOccurrenceChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "occurrencePeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CommunicationRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaces: Option<Vec<Reference>>,
    #[serde(rename = "groupIdentifier", skip_serializing_if = "Option::is_none")]
    pub group_identifier: Option<Identifier>,
    pub status: Code,
    #[serde(rename = "statusReason", skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Code>,
    #[serde(rename = "doNotPerform", skip_serializing_if = "Option::is_none")]
    pub do_not_perform: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub medium: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Vec<CommunicationRequestPayload>>,
    #[serde(flatten)]
    pub content: Option<CommunicationRequestContentChoice>,
    #[serde(flatten)]
    pub occurrence: Option<CommunicationRequestOccurrenceChoice>,
    #[serde(rename = "authoredOn", skip_serializing_if = "Option::is_none")]
    pub authored_on: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<Reference>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CompartmentDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub name: String,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    pub code: Code,
    pub search: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Vec<CompartmentDefinitionResource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}


/// Choice of types for the target[x] field in Composition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CompositionTargetChoice {
    /// Variant accepting the Identifier type.
    #[serde(rename = "targetIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Reference type.
    #[serde(rename = "targetReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Composition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    pub status: Code,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    pub date: DateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Vec<Reference>>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidentiality: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attester: Option<Vec<CompositionAttester>>,
    pub mode: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custodian: Option<Reference>,
    #[serde(rename = "relatesTo", skip_serializing_if = "Option::is_none")]
    pub relates_to: Option<Vec<CompositionRelatesTo>>,
    pub code: Code,
    #[serde(flatten)]
    pub target: Option<CompositionTargetChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<Vec<CompositionEvent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<Vec<CompositionSection>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<Reference>,
    #[serde(rename = "orderedBy", skip_serializing_if = "Option::is_none")]
    pub ordered_by: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<Vec<Reference>>,
    #[serde(rename = "emptyReason", skip_serializing_if = "Option::is_none")]
    pub empty_reason: Option<CodeableConcept>,
}


/// Choice of types for the source[x] field in ConceptMap
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ConceptMapSourceChoice {
    /// Variant accepting the Uri type.
    #[serde(rename = "sourceUri")]
    Uri(Uri),
    /// Variant accepting the Canonical type.
    #[serde(rename = "sourceCanonical")]
    Canonical(Canonical),
}

/// Choice of types for the target[x] field in ConceptMap
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ConceptMapTargetChoice {
    /// Variant accepting the Uri type.
    #[serde(rename = "targetUri")]
    Uri(Uri),
    /// Variant accepting the Canonical type.
    #[serde(rename = "targetCanonical")]
    Canonical(Canonical),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ConceptMap {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(flatten)]
    pub source: Option<ConceptMapSourceChoice>,
    #[serde(flatten)]
    pub target: Option<ConceptMapTargetChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Vec<ConceptMapGroup>>,
    #[serde(rename = "sourceVersion", skip_serializing_if = "Option::is_none")]
    pub source_version: Option<String>,
    #[serde(rename = "targetVersion", skip_serializing_if = "Option::is_none")]
    pub target_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<Vec<ConceptMapGroupElement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    pub equivalence: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(rename = "dependsOn", skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<ConceptMapGroupElementTargetDependsOn>>,
    pub property: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Canonical>,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<Vec<ConceptMapGroupElementTargetDependsOn>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unmapped: Option<ConceptMapGroupUnmapped>,
    pub mode: Code,
}


/// Choice of types for the onset[x] field in Condition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ConditionOnsetChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "onsetDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Age type.
    #[serde(rename = "onsetAge")]
    Age(Age),
    /// Variant accepting the Period type.
    #[serde(rename = "onsetPeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[serde(rename = "onsetRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[serde(rename = "onsetString")]
    String(String),
}

/// Choice of types for the abatement[x] field in Condition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ConditionAbatementChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "abatementDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Age type.
    #[serde(rename = "abatementAge")]
    Age(Age),
    /// Variant accepting the Period type.
    #[serde(rename = "abatementPeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[serde(rename = "abatementRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[serde(rename = "abatementString")]
    String(String),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Condition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "clinicalStatus", skip_serializing_if = "Option::is_none")]
    pub clinical_status: Option<CodeableConcept>,
    #[serde(rename = "verificationStatus", skip_serializing_if = "Option::is_none")]
    pub verification_status: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(rename = "bodySite", skip_serializing_if = "Option::is_none")]
    pub body_site: Option<Vec<CodeableConcept>>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(flatten)]
    pub onset: Option<ConditionOnsetChoice>,
    #[serde(flatten)]
    pub abatement: Option<ConditionAbatementChoice>,
    #[serde(rename = "recordedDate", skip_serializing_if = "Option::is_none")]
    pub recorded_date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorder: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asserter: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<Vec<ConditionStage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assessment: Option<Vec<Reference>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<ConditionEvidence>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}


/// Choice of types for the source[x] field in Consent
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ConsentSourceChoice {
    /// Variant accepting the Attachment type.
    #[serde(rename = "sourceAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "sourceReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Consent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub scope: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patient: Option<Reference>,
    #[serde(rename = "dateTime", skip_serializing_if = "Option::is_none")]
    pub date_time: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<Vec<Reference>>,
    #[serde(flatten)]
    pub source: Option<ConsentSourceChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<Vec<ConsentPolicy>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<Uri>,
    #[serde(rename = "policyRule", skip_serializing_if = "Option::is_none")]
    pub policy_rule: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification: Option<Vec<ConsentVerification>>,
    pub verified: Boolean,
    #[serde(rename = "verifiedWith", skip_serializing_if = "Option::is_none")]
    pub verified_with: Option<Reference>,
    #[serde(rename = "verificationDate", skip_serializing_if = "Option::is_none")]
    pub verification_date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provision: Option<ConsentProvision>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Vec<ConsentProvisionActor>>,
    pub role: CodeableConcept,
    pub reference: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<CodeableConcept>>,
    #[serde(rename = "securityLabel", skip_serializing_if = "Option::is_none")]
    pub security_label: Option<Vec<Coding>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Vec<Coding>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<Vec<Coding>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "dataPeriod", skip_serializing_if = "Option::is_none")]
    pub data_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<ConsentProvisionData>>,
    pub meaning: Code,
}


/// Choice of types for the topic[x] field in Contract
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ContractTopicChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "topicCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "topicReference")]
    Reference(Reference),
}

/// Choice of types for the value[x] field in Contract
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ContractValueChoice {
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Date type.
    #[serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Uri type.
    #[serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Attachment type.
    #[serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
}

/// Choice of types for the entity[x] field in Contract
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ContractEntityChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "entityCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "entityReference")]
    Reference(Reference),
}

/// Choice of types for the occurrence[x] field in Contract
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ContractOccurrenceChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

/// Choice of types for the content[x] field in Contract
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ContractContentChoice {
    /// Variant accepting the Attachment type.
    #[serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "contentReference")]
    Reference(Reference),
}

/// Choice of types for the legallyBinding[x] field in Contract
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ContractLegallyBindingChoice {
    /// Variant accepting the Attachment type.
    #[serde(rename = "legallyBindingAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "legallyBindingReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Contract {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(rename = "legalState", skip_serializing_if = "Option::is_none")]
    pub legal_state: Option<CodeableConcept>,
    #[serde(rename = "instantiatesCanonical", skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Reference>,
    #[serde(rename = "instantiatesUri", skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Uri>,
    #[serde(rename = "contentDerivative", skip_serializing_if = "Option::is_none")]
    pub content_derivative: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applies: Option<Period>,
    #[serde(rename = "expirationType", skip_serializing_if = "Option::is_none")]
    pub expiration_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<CodeableConcept>,
    #[serde(flatten)]
    pub topic: Option<ContractTopicChoice>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "subType", skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "contentDefinition", skip_serializing_if = "Option::is_none")]
    pub content_definition: Option<ContractContentDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<Reference>,
    #[serde(rename = "publicationDate", skip_serializing_if = "Option::is_none")]
    pub publication_date: Option<DateTime>,
    #[serde(rename = "publicationStatus")]
    pub publication_status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub term: Option<Vec<ContractTerm>>,
    #[serde(rename = "securityLabel", skip_serializing_if = "Option::is_none")]
    pub security_label: Option<Vec<ContractTermSecurityLabel>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<Vec<UnsignedInt>>,
    pub classification: Coding,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<Coding>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control: Option<Vec<Coding>>,
    pub offer: ContractTermOffer,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<Vec<ContractTermOfferParty>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Vec<Reference>>,
    pub role: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<CodeableConcept>,
    #[serde(rename = "decisionMode", skip_serializing_if = "Option::is_none")]
    pub decision_mode: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<Vec<ContractTermOfferAnswer>>,
    #[serde(flatten)]
    pub value: Option<ContractValueChoice>,
    #[serde(rename = "linkId", skip_serializing_if = "Option::is_none")]
    pub link_id: Option<Vec<String>>,
    #[serde(rename = "securityLabelNumber", skip_serializing_if = "Option::is_none")]
    pub security_label_number: Option<Vec<UnsignedInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<Vec<ContractTermAsset>>,
    #[serde(rename = "typeReference", skip_serializing_if = "Option::is_none")]
    pub type_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtype: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Vec<ContractTermAssetContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(rename = "periodType", skip_serializing_if = "Option::is_none")]
    pub period_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Vec<Period>>,
    #[serde(rename = "usePeriod", skip_serializing_if = "Option::is_none")]
    pub use_period: Option<Vec<Period>>,
    #[serde(rename = "valuedItem", skip_serializing_if = "Option::is_none")]
    pub valued_item: Option<Vec<ContractTermAssetValuedItem>>,
    #[serde(flatten)]
    pub entity: Option<ContractEntityChoice>,
    #[serde(rename = "effectiveTime", skip_serializing_if = "Option::is_none")]
    pub effective_time: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice", skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment: Option<String>,
    #[serde(rename = "paymentDate", skip_serializing_if = "Option::is_none")]
    pub payment_date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsible: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<ContractTermAction>>,
    #[serde(rename = "doNotPerform", skip_serializing_if = "Option::is_none")]
    pub do_not_perform: Option<Boolean>,
    pub intent: CodeableConcept,
    #[serde(rename = "contextLinkId", skip_serializing_if = "Option::is_none")]
    pub context_link_id: Option<Vec<String>>,
    #[serde(flatten)]
    pub occurrence: Option<ContractOccurrenceChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester: Option<Vec<Reference>>,
    #[serde(rename = "requesterLinkId", skip_serializing_if = "Option::is_none")]
    pub requester_link_id: Option<Vec<String>>,
    #[serde(rename = "performerType", skip_serializing_if = "Option::is_none")]
    pub performer_type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "performerRole", skip_serializing_if = "Option::is_none")]
    pub performer_role: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Reference>,
    #[serde(rename = "performerLinkId", skip_serializing_if = "Option::is_none")]
    pub performer_link_id: Option<Vec<String>>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Vec<String>>,
    #[serde(rename = "reasonLinkId", skip_serializing_if = "Option::is_none")]
    pub reason_link_id: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Vec<ContractTerm>>,
    #[serde(rename = "supportingInfo", skip_serializing_if = "Option::is_none")]
    pub supporting_info: Option<Vec<Reference>>,
    #[serde(rename = "relevantHistory", skip_serializing_if = "Option::is_none")]
    pub relevant_history: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signer: Option<Vec<ContractSigner>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<Signature>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friendly: Option<Vec<ContractFriendly>>,
    #[serde(flatten)]
    pub content: Option<ContractContentChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal: Option<Vec<ContractLegal>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<Vec<ContractRule>>,
    #[serde(flatten)]
    pub legally_binding: Option<ContractLegallyBindingChoice>,
}


/// Choice of types for the value[x] field in Coverage
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CoverageValueChoice {
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Money type.
    #[serde(rename = "valueMoney")]
    Money(Money),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Coverage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "policyHolder", skip_serializing_if = "Option::is_none")]
    pub policy_holder: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscriber: Option<Reference>,
    #[serde(rename = "subscriberId", skip_serializing_if = "Option::is_none")]
    pub subscriber_id: Option<String>,
    pub beneficiary: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payor: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<Vec<CoverageClass>>,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(rename = "costToBeneficiary", skip_serializing_if = "Option::is_none")]
    pub cost_to_beneficiary: Option<Vec<CoverageCostToBeneficiary>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exception: Option<Vec<CoverageCostToBeneficiaryException>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subrogation: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<Vec<Reference>>,
}


/// Choice of types for the serviced[x] field in CoverageEligibilityRequest
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityRequestServicedChoice {
    /// Variant accepting the Date type.
    #[serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "servicedPeriod")]
    Period(Period),
}

/// Choice of types for the diagnosis[x] field in CoverageEligibilityRequest
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityRequestDiagnosisChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "diagnosisCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "diagnosisReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CoverageEligibilityRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Vec<Code>>,
    pub patient: Reference,
    #[serde(flatten)]
    pub serviced: Option<CoverageEligibilityRequestServicedChoice>,
    pub created: DateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterer: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<Reference>,
    pub insurer: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facility: Option<Reference>,
    #[serde(rename = "supportingInfo", skip_serializing_if = "Option::is_none")]
    pub supporting_info: Option<Vec<CoverageEligibilityRequestSupportingInfo>>,
    pub sequence: PositiveInt,
    pub information: Reference,
    #[serde(rename = "appliesToAll", skip_serializing_if = "Option::is_none")]
    pub applies_to_all: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurance: Option<Vec<CoverageEligibilityRequestInsurance>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focal: Option<Boolean>,
    pub coverage: Reference,
    #[serde(rename = "businessArrangement", skip_serializing_if = "Option::is_none")]
    pub business_arrangement: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<CoverageEligibilityRequestItem>>,
    #[serde(rename = "supportingInfoSequence", skip_serializing_if = "Option::is_none")]
    pub supporting_info_sequence: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService", skip_serializing_if = "Option::is_none")]
    pub product_or_service: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice", skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<Vec<CoverageEligibilityRequestItemDiagnosis>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<Reference>>,
}


/// Choice of types for the serviced[x] field in CoverageEligibilityResponse
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityResponseServicedChoice {
    /// Variant accepting the Date type.
    #[serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "servicedPeriod")]
    Period(Period),
}

/// Choice of types for the allowed[x] field in CoverageEligibilityResponse
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityResponseAllowedChoice {
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "allowedUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the String type.
    #[serde(rename = "allowedString")]
    String(String),
    /// Variant accepting the Money type.
    #[serde(rename = "allowedMoney")]
    Money(Money),
}

/// Choice of types for the used[x] field in CoverageEligibilityResponse
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityResponseUsedChoice {
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "usedUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the String type.
    #[serde(rename = "usedString")]
    String(String),
    /// Variant accepting the Money type.
    #[serde(rename = "usedMoney")]
    Money(Money),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CoverageEligibilityResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Vec<Code>>,
    pub patient: Reference,
    #[serde(flatten)]
    pub serviced: Option<CoverageEligibilityResponseServicedChoice>,
    pub created: DateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requestor: Option<Reference>,
    pub request: Reference,
    pub outcome: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disposition: Option<String>,
    pub insurer: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurance: Option<Vec<CoverageEligibilityResponseInsurance>>,
    pub coverage: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inforce: Option<Boolean>,
    #[serde(rename = "benefitPeriod", skip_serializing_if = "Option::is_none")]
    pub benefit_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<CoverageEligibilityResponseInsuranceItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService", skip_serializing_if = "Option::is_none")]
    pub product_or_service: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub term: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub benefit: Option<Vec<CoverageEligibilityResponseInsuranceItemBenefit>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(flatten)]
    pub allowed: Option<CoverageEligibilityResponseAllowedChoice>,
    #[serde(flatten)]
    pub used: Option<CoverageEligibilityResponseUsedChoice>,
    #[serde(rename = "authorizationRequired", skip_serializing_if = "Option::is_none")]
    pub authorization_required: Option<Boolean>,
    #[serde(rename = "authorizationSupporting", skip_serializing_if = "Option::is_none")]
    pub authorization_supporting: Option<Vec<CodeableConcept>>,
    #[serde(rename = "authorizationUrl", skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<Uri>,
    #[serde(rename = "preAuthRef", skip_serializing_if = "Option::is_none")]
    pub pre_auth_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Vec<CoverageEligibilityResponseError>>,
    pub code: CodeableConcept,
}


/// Choice of types for the identified[x] field in DetectedIssue
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DetectedIssueIdentifiedChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "identifiedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "identifiedPeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct DetectedIssue {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patient: Option<Reference>,
    #[serde(flatten)]
    pub identified: Option<DetectedIssueIdentifiedChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicated: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<DetectedIssueEvidence>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mitigation: Option<Vec<DetectedIssueMitigation>>,
    pub action: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Device {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<Reference>,
    #[serde(rename = "udiCarrier", skip_serializing_if = "Option::is_none")]
    pub udi_carrier: Option<Vec<DeviceUdiCarrier>>,
    #[serde(rename = "deviceIdentifier", skip_serializing_if = "Option::is_none")]
    pub device_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Uri>,
    #[serde(rename = "carrierAIDC", skip_serializing_if = "Option::is_none")]
    pub carrier_a_i_d_c: Option<Base64Binary>,
    #[serde(rename = "carrierHRF", skip_serializing_if = "Option::is_none")]
    pub carrier_h_r_f: Option<String>,
    #[serde(rename = "entryType", skip_serializing_if = "Option::is_none")]
    pub entry_type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(rename = "statusReason", skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<Vec<CodeableConcept>>,
    #[serde(rename = "distinctIdentifier", skip_serializing_if = "Option::is_none")]
    pub distinct_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(rename = "manufactureDate", skip_serializing_if = "Option::is_none")]
    pub manufacture_date: Option<DateTime>,
    #[serde(rename = "expirationDate", skip_serializing_if = "Option::is_none")]
    pub expiration_date: Option<DateTime>,
    #[serde(rename = "lotNumber", skip_serializing_if = "Option::is_none")]
    pub lot_number: Option<String>,
    #[serde(rename = "serialNumber", skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    #[serde(rename = "deviceName", skip_serializing_if = "Option::is_none")]
    pub device_name: Option<Vec<DeviceDeviceName>>,
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(rename = "modelNumber", skip_serializing_if = "Option::is_none")]
    pub model_number: Option<String>,
    #[serde(rename = "partNumber", skip_serializing_if = "Option::is_none")]
    pub part_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specialization: Option<Vec<DeviceSpecialization>>,
    #[serde(rename = "systemType")]
    pub system_type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<Identifier>,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property: Option<Vec<DeviceProperty>>,
    #[serde(rename = "valueQuantity", skip_serializing_if = "Option::is_none")]
    pub value_quantity: Option<Vec<Quantity>>,
    #[serde(rename = "valueCode", skip_serializing_if = "Option::is_none")]
    pub value_code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patient: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<Reference>,
}


/// Choice of types for the manufacturer[x] field in DeviceDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DeviceDefinitionManufacturerChoice {
    /// Variant accepting the String type.
    #[serde(rename = "manufacturerString")]
    String(String),
    /// Variant accepting the Reference type.
    #[serde(rename = "manufacturerReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct DeviceDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "udiDeviceIdentifier", skip_serializing_if = "Option::is_none")]
    pub udi_device_identifier: Option<Vec<DeviceDefinitionUdiDeviceIdentifier>>,
    #[serde(rename = "deviceIdentifier")]
    pub device_identifier: String,
    pub issuer: Uri,
    pub jurisdiction: Uri,
    #[serde(flatten)]
    pub manufacturer: Option<DeviceDefinitionManufacturerChoice>,
    #[serde(rename = "deviceName", skip_serializing_if = "Option::is_none")]
    pub device_name: Option<Vec<DeviceDefinitionDeviceName>>,
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(rename = "modelNumber", skip_serializing_if = "Option::is_none")]
    pub model_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specialization: Option<Vec<DeviceDefinitionSpecialization>>,
    #[serde(rename = "systemType")]
    pub system_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety: Option<Vec<CodeableConcept>>,
    #[serde(rename = "shelfLifeStorage", skip_serializing_if = "Option::is_none")]
    pub shelf_life_storage: Option<Vec<ProductShelfLife>>,
    #[serde(rename = "physicalCharacteristics", skip_serializing_if = "Option::is_none")]
    pub physical_characteristics: Option<ProdCharacteristic>,
    #[serde(rename = "languageCode", skip_serializing_if = "Option::is_none")]
    pub language_code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability: Option<Vec<DeviceDefinitionCapability>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property: Option<Vec<DeviceDefinitionProperty>>,
    #[serde(rename = "valueQuantity", skip_serializing_if = "Option::is_none")]
    pub value_quantity: Option<Vec<Quantity>>,
    #[serde(rename = "valueCode", skip_serializing_if = "Option::is_none")]
    pub value_code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(rename = "onlineInformation", skip_serializing_if = "Option::is_none")]
    pub online_information: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "parentDevice", skip_serializing_if = "Option::is_none")]
    pub parent_device: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<Vec<DeviceDefinitionMaterial>>,
    pub substance: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternate: Option<Boolean>,
    #[serde(rename = "allergenicIndicator", skip_serializing_if = "Option::is_none")]
    pub allergenic_indicator: Option<Boolean>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct DeviceMetric {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<Reference>,
    #[serde(rename = "operationalStatus", skip_serializing_if = "Option::is_none")]
    pub operational_status: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Code>,
    pub category: Code,
    #[serde(rename = "measurementPeriod", skip_serializing_if = "Option::is_none")]
    pub measurement_period: Option<Timing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calibration: Option<Vec<DeviceMetricCalibration>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<Instant>,
}


/// Choice of types for the code[x] field in DeviceRequest
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DeviceRequestCodeChoice {
    /// Variant accepting the Reference type.
    #[serde(rename = "codeReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "codeCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

/// Choice of types for the value[x] field in DeviceRequest
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DeviceRequestValueChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
}

/// Choice of types for the occurrence[x] field in DeviceRequest
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DeviceRequestOccurrenceChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct DeviceRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical", skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri", skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "priorRequest", skip_serializing_if = "Option::is_none")]
    pub prior_request: Option<Vec<Reference>>,
    #[serde(rename = "groupIdentifier", skip_serializing_if = "Option::is_none")]
    pub group_identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    pub intent: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Code>,
    #[serde(flatten)]
    pub code: Option<DeviceRequestCodeChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<DeviceRequestParameter>>,
    #[serde(flatten)]
    pub value: Option<DeviceRequestValueChoice>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(flatten)]
    pub occurrence: Option<DeviceRequestOccurrenceChoice>,
    #[serde(rename = "authoredOn", skip_serializing_if = "Option::is_none")]
    pub authored_on: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester: Option<Reference>,
    #[serde(rename = "performerType", skip_serializing_if = "Option::is_none")]
    pub performer_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Reference>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurance: Option<Vec<Reference>>,
    #[serde(rename = "supportingInfo", skip_serializing_if = "Option::is_none")]
    pub supporting_info: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "relevantHistory", skip_serializing_if = "Option::is_none")]
    pub relevant_history: Option<Vec<Reference>>,
}


/// Choice of types for the timing[x] field in DeviceUseStatement
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DeviceUseStatementTimingChoice {
    /// Variant accepting the Timing type.
    #[serde(rename = "timingTiming")]
    Timing(Timing),
    /// Variant accepting the Period type.
    #[serde(rename = "timingPeriod")]
    Period(Period),
    /// Variant accepting the DateTime type.
    #[serde(rename = "timingDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct DeviceUseStatement {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    pub status: Code,
    pub subject: Reference,
    #[serde(rename = "derivedFrom", skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<Vec<Reference>>,
    #[serde(flatten)]
    pub timing: Option<DeviceUseStatementTimingChoice>,
    #[serde(rename = "recordedOn", skip_serializing_if = "Option::is_none")]
    pub recorded_on: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Reference>,
    pub device: Reference,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "bodySite", skip_serializing_if = "Option::is_none")]
    pub body_site: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}


/// Choice of types for the effective[x] field in DiagnosticReport
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticReportEffectiveChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "effectivePeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct DiagnosticReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(flatten)]
    pub effective: Option<DiagnosticReportEffectiveChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued: Option<Instant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<Reference>>,
    #[serde(rename = "resultsInterpreter", skip_serializing_if = "Option::is_none")]
    pub results_interpreter: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specimen: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Vec<Reference>>,
    #[serde(rename = "imagingStudy", skip_serializing_if = "Option::is_none")]
    pub imaging_study: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<Vec<DiagnosticReportMedia>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub link: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<String>,
    #[serde(rename = "conclusionCode", skip_serializing_if = "Option::is_none")]
    pub conclusion_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "presentedForm", skip_serializing_if = "Option::is_none")]
    pub presented_form: Option<Vec<Attachment>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct DocumentManifest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "masterIdentifier", skip_serializing_if = "Option::is_none")]
    pub master_identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related: Option<Vec<DocumentManifestRelated>>,
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct DocumentReference {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "masterIdentifier", skip_serializing_if = "Option::is_none")]
    pub master_identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "docStatus", skip_serializing_if = "Option::is_none")]
    pub doc_status: Option<Code>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<Instant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authenticator: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custodian: Option<Reference>,
    #[serde(rename = "relatesTo", skip_serializing_if = "Option::is_none")]
    pub relates_to: Option<Vec<DocumentReferenceRelatesTo>>,
    pub code: Code,
    pub target: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "securityLabel", skip_serializing_if = "Option::is_none")]
    pub security_label: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<DocumentReferenceContent>>,
    pub attachment: Attachment,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<DocumentReferenceContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(rename = "facilityType", skip_serializing_if = "Option::is_none")]
    pub facility_type: Option<CodeableConcept>,
    #[serde(rename = "practiceSetting", skip_serializing_if = "Option::is_none")]
    pub practice_setting: Option<CodeableConcept>,
    #[serde(rename = "sourcePatientInfo", skip_serializing_if = "Option::is_none")]
    pub source_patient_info: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct EffectEvidenceSynthesis {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate", skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate", skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod", skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact", skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(rename = "synthesisType", skip_serializing_if = "Option::is_none")]
    pub synthesis_type: Option<CodeableConcept>,
    #[serde(rename = "studyType", skip_serializing_if = "Option::is_none")]
    pub study_type: Option<CodeableConcept>,
    pub population: Reference,
    pub exposure: Reference,
    #[serde(rename = "exposureAlternative")]
    pub exposure_alternative: Reference,
    pub outcome: Reference,
    #[serde(rename = "sampleSize", skip_serializing_if = "Option::is_none")]
    pub sample_size: Option<EffectEvidenceSynthesisSampleSize>,
    #[serde(rename = "numberOfStudies", skip_serializing_if = "Option::is_none")]
    pub number_of_studies: Option<Integer>,
    #[serde(rename = "numberOfParticipants", skip_serializing_if = "Option::is_none")]
    pub number_of_participants: Option<Integer>,
    #[serde(rename = "resultsByExposure", skip_serializing_if = "Option::is_none")]
    pub results_by_exposure: Option<Vec<EffectEvidenceSynthesisResultsByExposure>>,
    #[serde(rename = "exposureState", skip_serializing_if = "Option::is_none")]
    pub exposure_state: Option<Code>,
    #[serde(rename = "variantState", skip_serializing_if = "Option::is_none")]
    pub variant_state: Option<CodeableConcept>,
    #[serde(rename = "riskEvidenceSynthesis")]
    pub risk_evidence_synthesis: Reference,
    #[serde(rename = "effectEstimate", skip_serializing_if = "Option::is_none")]
    pub effect_estimate: Option<Vec<EffectEvidenceSynthesisEffectEstimate>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Decimal>,
    #[serde(rename = "unitOfMeasure", skip_serializing_if = "Option::is_none")]
    pub unit_of_measure: Option<CodeableConcept>,
    #[serde(rename = "precisionEstimate", skip_serializing_if = "Option::is_none")]
    pub precision_estimate: Option<Vec<EffectEvidenceSynthesisEffectEstimatePrecisionEstimate>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certainty: Option<Vec<EffectEvidenceSynthesisCertainty>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<Vec<CodeableConcept>>,
    #[serde(rename = "certaintySubcomponent", skip_serializing_if = "Option::is_none")]
    pub certainty_subcomponent: Option<Vec<EffectEvidenceSynthesisCertaintyCertaintySubcomponent>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Encounter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "statusHistory", skip_serializing_if = "Option::is_none")]
    pub status_history: Option<Vec<EncounterStatusHistory>>,
    pub period: Period,
    pub class: Coding,
    #[serde(rename = "classHistory", skip_serializing_if = "Option::is_none")]
    pub class_history: Option<Vec<EncounterClassHistory>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "serviceType", skip_serializing_if = "Option::is_none")]
    pub service_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(rename = "episodeOfCare", skip_serializing_if = "Option::is_none")]
    pub episode_of_care: Option<Vec<Reference>>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant: Option<Vec<EncounterParticipant>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub individual: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appointment: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<Duration>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<Vec<EncounterDiagnosis>>,
    pub condition: Reference,
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub r#use: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hospitalization: Option<EncounterHospitalization>,
    #[serde(rename = "preAdmissionIdentifier", skip_serializing_if = "Option::is_none")]
    pub pre_admission_identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Reference>,
    #[serde(rename = "admitSource", skip_serializing_if = "Option::is_none")]
    pub admit_source: Option<CodeableConcept>,
    #[serde(rename = "reAdmission", skip_serializing_if = "Option::is_none")]
    pub re_admission: Option<CodeableConcept>,
    #[serde(rename = "dietPreference", skip_serializing_if = "Option::is_none")]
    pub diet_preference: Option<Vec<CodeableConcept>>,
    #[serde(rename = "specialCourtesy", skip_serializing_if = "Option::is_none")]
    pub special_courtesy: Option<Vec<CodeableConcept>>,
    #[serde(rename = "specialArrangement", skip_serializing_if = "Option::is_none")]
    pub special_arrangement: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<Reference>,
    #[serde(rename = "dischargeDisposition", skip_serializing_if = "Option::is_none")]
    pub discharge_disposition: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Vec<EncounterLocation>>,
    #[serde(rename = "physicalType", skip_serializing_if = "Option::is_none")]
    pub physical_type: Option<CodeableConcept>,
    #[serde(rename = "serviceProvider", skip_serializing_if = "Option::is_none")]
    pub service_provider: Option<Reference>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Endpoint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "connectionType")]
    pub connection_type: Coding,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "managingOrganization", skip_serializing_if = "Option::is_none")]
    pub managing_organization: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(rename = "payloadType", skip_serializing_if = "Option::is_none")]
    pub payload_type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "payloadMimeType", skip_serializing_if = "Option::is_none")]
    pub payload_mime_type: Option<Vec<Code>>,
    pub address: Url,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<Vec<String>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct EnrollmentRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurer: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct EnrollmentResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disposition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<Reference>,
    #[serde(rename = "requestProvider", skip_serializing_if = "Option::is_none")]
    pub request_provider: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct EpisodeOfCare {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "statusHistory", skip_serializing_if = "Option::is_none")]
    pub status_history: Option<Vec<EpisodeOfCareStatusHistory>>,
    pub period: Period,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<Vec<EpisodeOfCareDiagnosis>>,
    pub condition: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<PositiveInt>,
    pub patient: Reference,
    #[serde(rename = "managingOrganization", skip_serializing_if = "Option::is_none")]
    pub managing_organization: Option<Reference>,
    #[serde(rename = "referralRequest", skip_serializing_if = "Option::is_none")]
    pub referral_request: Option<Vec<Reference>>,
    #[serde(rename = "careManager", skip_serializing_if = "Option::is_none")]
    pub care_manager: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Vec<Reference>>,
}


/// Choice of types for the subject[x] field in EventDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum EventDefinitionSubjectChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct EventDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(flatten)]
    pub subject: Option<EventDefinitionSubjectChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate", skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate", skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod", skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact", skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger: Option<Vec<TriggerDefinition>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Evidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(rename = "shortTitle", skip_serializing_if = "Option::is_none")]
    pub short_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate", skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate", skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod", skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact", skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(rename = "exposureBackground")]
    pub exposure_background: Reference,
    #[serde(rename = "exposureVariant", skip_serializing_if = "Option::is_none")]
    pub exposure_variant: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Vec<Reference>>,
}


/// Choice of types for the definition[x] field in EvidenceVariable
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceVariableDefinitionChoice {
    /// Variant accepting the Reference type.
    #[serde(rename = "definitionReference")]
    Reference(Reference),
    /// Variant accepting the Canonical type.
    #[serde(rename = "definitionCanonical")]
    Canonical(Canonical),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "definitionCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Expression type.
    #[serde(rename = "definitionExpression")]
    Expression(Expression),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "definitionDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "definitionTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
}

/// Choice of types for the participantEffective[x] field in EvidenceVariable
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceVariableParticipantEffectiveChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "participantEffectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "participantEffectivePeriod")]
    Period(Period),
    /// Variant accepting the Duration type.
    #[serde(rename = "participantEffectiveDuration")]
    Duration(Duration),
    /// Variant accepting the Timing type.
    #[serde(rename = "participantEffectiveTiming")]
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct EvidenceVariable {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(rename = "shortTitle", skip_serializing_if = "Option::is_none")]
    pub short_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate", skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate", skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod", skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact", skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub characteristic: Option<Vec<EvidenceVariableCharacteristic>>,
    #[serde(flatten)]
    pub definition: Option<EvidenceVariableDefinitionChoice>,
    #[serde(rename = "usageContext", skip_serializing_if = "Option::is_none")]
    pub usage_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Boolean>,
    #[serde(flatten)]
    pub participant_effective: Option<EvidenceVariableParticipantEffectiveChoice>,
    #[serde(rename = "timeFromStart", skip_serializing_if = "Option::is_none")]
    pub time_from_start: Option<Duration>,
    #[serde(rename = "groupMeasure", skip_serializing_if = "Option::is_none")]
    pub group_measure: Option<Code>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ExampleScenario {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Vec<ExampleScenarioActor>>,
    #[serde(rename = "actorId")]
    pub actor_id: String,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<Vec<ExampleScenarioInstance>>,
    #[serde(rename = "resourceId")]
    pub resource_id: String,
    #[serde(rename = "resourceType")]
    pub resource_type: Code,
    #[serde(rename = "versionId")]
    pub version_id: String,
    #[serde(rename = "containedInstance", skip_serializing_if = "Option::is_none")]
    pub contained_instance: Option<Vec<ExampleScenarioInstanceContainedInstance>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<Vec<ExampleScenarioProcess>>,
    pub title: String,
    #[serde(rename = "preConditions", skip_serializing_if = "Option::is_none")]
    pub pre_conditions: Option<Markdown>,
    #[serde(rename = "postConditions", skip_serializing_if = "Option::is_none")]
    pub post_conditions: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<Vec<ExampleScenarioProcessStep>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pause: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<ExampleScenarioProcessStepOperation>,
    pub number: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initiator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver: Option<String>,
    #[serde(rename = "initiatorActive", skip_serializing_if = "Option::is_none")]
    pub initiator_active: Option<Boolean>,
    #[serde(rename = "receiverActive", skip_serializing_if = "Option::is_none")]
    pub receiver_active: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<ExampleScenarioInstanceContainedInstance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<ExampleScenarioInstanceContainedInstance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternative: Option<Vec<ExampleScenarioProcessStepAlternative>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow: Option<Vec<Canonical>>,
}


/// Choice of types for the timing[x] field in ExplanationOfBenefit
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitTimingChoice {
    /// Variant accepting the Date type.
    #[serde(rename = "timingDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "timingPeriod")]
    Period(Period),
}

/// Choice of types for the value[x] field in ExplanationOfBenefit
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitValueChoice {
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Attachment type.
    #[serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
}

/// Choice of types for the diagnosis[x] field in ExplanationOfBenefit
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitDiagnosisChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "diagnosisCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "diagnosisReference")]
    Reference(Reference),
}

/// Choice of types for the procedure[x] field in ExplanationOfBenefit
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitProcedureChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "procedureCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "procedureReference")]
    Reference(Reference),
}

/// Choice of types for the location[x] field in ExplanationOfBenefit
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitLocationChoice {
    /// Variant accepting the Address type.
    #[serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[serde(rename = "locationReference")]
    Reference(Reference),
}

/// Choice of types for the serviced[x] field in ExplanationOfBenefit
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitServicedChoice {
    /// Variant accepting the Date type.
    #[serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "servicedPeriod")]
    Period(Period),
}

/// Choice of types for the allowed[x] field in ExplanationOfBenefit
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitAllowedChoice {
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "allowedUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the String type.
    #[serde(rename = "allowedString")]
    String(String),
    /// Variant accepting the Money type.
    #[serde(rename = "allowedMoney")]
    Money(Money),
}

/// Choice of types for the used[x] field in ExplanationOfBenefit
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitUsedChoice {
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "usedUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Money type.
    #[serde(rename = "usedMoney")]
    Money(Money),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ExplanationOfBenefit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "subType", skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<CodeableConcept>,
    #[serde(rename = "use")]
    pub r#use: Code,
    pub patient: Reference,
    #[serde(rename = "billablePeriod", skip_serializing_if = "Option::is_none")]
    pub billable_period: Option<Period>,
    pub created: DateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterer: Option<Reference>,
    pub insurer: Reference,
    pub provider: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<CodeableConcept>,
    #[serde(rename = "fundsReserveRequested", skip_serializing_if = "Option::is_none")]
    pub funds_reserve_requested: Option<CodeableConcept>,
    #[serde(rename = "fundsReserve", skip_serializing_if = "Option::is_none")]
    pub funds_reserve: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related: Option<Vec<ExplanationOfBenefitRelated>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prescription: Option<Reference>,
    #[serde(rename = "originalPrescription", skip_serializing_if = "Option::is_none")]
    pub original_prescription: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payee: Option<ExplanationOfBenefitPayee>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referral: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facility: Option<Reference>,
    #[serde(rename = "claimResponse", skip_serializing_if = "Option::is_none")]
    pub claim_response: Option<Reference>,
    pub outcome: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disposition: Option<String>,
    #[serde(rename = "preAuthRef", skip_serializing_if = "Option::is_none")]
    pub pre_auth_ref: Option<Vec<String>>,
    #[serde(rename = "preAuthRefPeriod", skip_serializing_if = "Option::is_none")]
    pub pre_auth_ref_period: Option<Vec<Period>>,
    #[serde(rename = "careTeam", skip_serializing_if = "Option::is_none")]
    pub care_team: Option<Vec<ExplanationOfBenefitCareTeam>>,
    pub sequence: PositiveInt,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsible: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualification: Option<CodeableConcept>,
    #[serde(rename = "supportingInfo", skip_serializing_if = "Option::is_none")]
    pub supporting_info: Option<Vec<ExplanationOfBenefitSupportingInfo>>,
    pub category: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(flatten)]
    pub timing: Option<ExplanationOfBenefitTimingChoice>,
    #[serde(flatten)]
    pub value: Option<ExplanationOfBenefitValueChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<Vec<ExplanationOfBenefitDiagnosis>>,
    #[serde(rename = "onAdmission", skip_serializing_if = "Option::is_none")]
    pub on_admission: Option<CodeableConcept>,
    #[serde(rename = "packageCode", skip_serializing_if = "Option::is_none")]
    pub package_code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure: Option<Vec<ExplanationOfBenefitProcedure>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precedence: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurance: Option<Vec<ExplanationOfBenefitInsurance>>,
    pub focal: Boolean,
    pub coverage: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accident: Option<ExplanationOfBenefitAccident>,
    #[serde(flatten)]
    pub location: Option<ExplanationOfBenefitLocationChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<ExplanationOfBenefitItem>>,
    #[serde(rename = "careTeamSequence", skip_serializing_if = "Option::is_none")]
    pub care_team_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "diagnosisSequence", skip_serializing_if = "Option::is_none")]
    pub diagnosis_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "procedureSequence", skip_serializing_if = "Option::is_none")]
    pub procedure_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "informationSequence", skip_serializing_if = "Option::is_none")]
    pub information_sequence: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revenue: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode", skip_serializing_if = "Option::is_none")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(flatten)]
    pub serviced: Option<ExplanationOfBenefitServicedChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice", skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(rename = "bodySite", skip_serializing_if = "Option::is_none")]
    pub body_site: Option<CodeableConcept>,
    #[serde(rename = "subSite", skip_serializing_if = "Option::is_none")]
    pub sub_site: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Vec<Reference>>,
    #[serde(rename = "noteNumber", skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<ExplanationOfBenefitItemDetail>>,
    #[serde(rename = "subDetail", skip_serializing_if = "Option::is_none")]
    pub sub_detail: Option<Vec<ExplanationOfBenefitItemDetailSubDetail>>,
    #[serde(rename = "addItem", skip_serializing_if = "Option::is_none")]
    pub add_item: Option<Vec<ExplanationOfBenefitAddItem>>,
    #[serde(rename = "itemSequence", skip_serializing_if = "Option::is_none")]
    pub item_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "detailSequence", skip_serializing_if = "Option::is_none")]
    pub detail_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "subDetailSequence", skip_serializing_if = "Option::is_none")]
    pub sub_detail_sequence: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<Vec<ExplanationOfBenefitTotal>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment: Option<ExplanationOfBenefitPayment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjustment: Option<Money>,
    #[serde(rename = "adjustmentReason", skip_serializing_if = "Option::is_none")]
    pub adjustment_reason: Option<CodeableConcept>,
    #[serde(rename = "formCode", skip_serializing_if = "Option::is_none")]
    pub form_code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form: Option<Attachment>,
    #[serde(rename = "processNote", skip_serializing_if = "Option::is_none")]
    pub process_note: Option<Vec<ExplanationOfBenefitProcessNote>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<PositiveInt>,
    #[serde(rename = "benefitPeriod", skip_serializing_if = "Option::is_none")]
    pub benefit_period: Option<Period>,
    #[serde(rename = "benefitBalance", skip_serializing_if = "Option::is_none")]
    pub benefit_balance: Option<Vec<ExplanationOfBenefitBenefitBalance>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub term: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub financial: Option<Vec<ExplanationOfBenefitBenefitBalanceFinancial>>,
    #[serde(flatten)]
    pub allowed: Option<ExplanationOfBenefitAllowedChoice>,
    #[serde(flatten)]
    pub used: Option<ExplanationOfBenefitUsedChoice>,
}


/// Choice of types for the born[x] field in FamilyMemberHistory
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryBornChoice {
    /// Variant accepting the Period type.
    #[serde(rename = "bornPeriod")]
    Period(Period),
    /// Variant accepting the Date type.
    #[serde(rename = "bornDate")]
    Date(Date),
    /// Variant accepting the String type.
    #[serde(rename = "bornString")]
    String(String),
}

/// Choice of types for the age[x] field in FamilyMemberHistory
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryAgeChoice {
    /// Variant accepting the Age type.
    #[serde(rename = "ageAge")]
    Age(Age),
    /// Variant accepting the Range type.
    #[serde(rename = "ageRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[serde(rename = "ageString")]
    String(String),
}

/// Choice of types for the deceased[x] field in FamilyMemberHistory
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryDeceasedChoice {
    /// Variant accepting the Boolean type.
    #[serde(rename = "deceasedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Age type.
    #[serde(rename = "deceasedAge")]
    Age(Age),
    /// Variant accepting the Range type.
    #[serde(rename = "deceasedRange")]
    Range(Range),
    /// Variant accepting the Date type.
    #[serde(rename = "deceasedDate")]
    Date(Date),
    /// Variant accepting the String type.
    #[serde(rename = "deceasedString")]
    String(String),
}

/// Choice of types for the onset[x] field in FamilyMemberHistory
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryOnsetChoice {
    /// Variant accepting the Age type.
    #[serde(rename = "onsetAge")]
    Age(Age),
    /// Variant accepting the Range type.
    #[serde(rename = "onsetRange")]
    Range(Range),
    /// Variant accepting the Period type.
    #[serde(rename = "onsetPeriod")]
    Period(Period),
    /// Variant accepting the String type.
    #[serde(rename = "onsetString")]
    String(String),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct FamilyMemberHistory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical", skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri", skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    pub status: Code,
    #[serde(rename = "dataAbsentReason", skip_serializing_if = "Option::is_none")]
    pub data_absent_reason: Option<CodeableConcept>,
    pub patient: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub relationship: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sex: Option<CodeableConcept>,
    #[serde(flatten)]
    pub born: Option<FamilyMemberHistoryBornChoice>,
    #[serde(flatten)]
    pub age: Option<FamilyMemberHistoryAgeChoice>,
    #[serde(rename = "estimatedAge", skip_serializing_if = "Option::is_none")]
    pub estimated_age: Option<Boolean>,
    #[serde(flatten)]
    pub deceased: Option<FamilyMemberHistoryDeceasedChoice>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Vec<FamilyMemberHistoryCondition>>,
    pub code: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<CodeableConcept>,
    #[serde(rename = "contributedToDeath", skip_serializing_if = "Option::is_none")]
    pub contributed_to_death: Option<Boolean>,
    #[serde(flatten)]
    pub onset: Option<FamilyMemberHistoryOnsetChoice>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Flag {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Reference>,
}


/// Choice of types for the start[x] field in Goal
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum GoalStartChoice {
    /// Variant accepting the Date type.
    #[serde(rename = "startDate")]
    Date(Date),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "startCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

/// Choice of types for the detail[x] field in Goal
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum GoalDetailChoice {
    /// Variant accepting the Quantity type.
    #[serde(rename = "detailQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "detailRange")]
    Range(Range),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "detailCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the String type.
    #[serde(rename = "detailString")]
    String(String),
    /// Variant accepting the Boolean type.
    #[serde(rename = "detailBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[serde(rename = "detailInteger")]
    Integer(Integer),
    /// Variant accepting the Ratio type.
    #[serde(rename = "detailRatio")]
    Ratio(Ratio),
}

/// Choice of types for the due[x] field in Goal
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum GoalDueChoice {
    /// Variant accepting the Date type.
    #[serde(rename = "dueDate")]
    Date(Date),
    /// Variant accepting the Duration type.
    #[serde(rename = "dueDuration")]
    Duration(Duration),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Goal {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "lifecycleStatus")]
    pub lifecycle_status: Code,
    #[serde(rename = "achievementStatus", skip_serializing_if = "Option::is_none")]
    pub achievement_status: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<CodeableConcept>,
    pub description: CodeableConcept,
    pub subject: Reference,
    #[serde(flatten)]
    pub start: Option<GoalStartChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<GoalTarget>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measure: Option<CodeableConcept>,
    #[serde(flatten)]
    pub detail: Option<GoalDetailChoice>,
    #[serde(flatten)]
    pub due: Option<GoalDueChoice>,
    #[serde(rename = "statusDate", skip_serializing_if = "Option::is_none")]
    pub status_date: Option<Date>,
    #[serde(rename = "statusReason", skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    #[serde(rename = "expressedBy", skip_serializing_if = "Option::is_none")]
    pub expressed_by: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addresses: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "outcomeCode", skip_serializing_if = "Option::is_none")]
    pub outcome_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "outcomeReference", skip_serializing_if = "Option::is_none")]
    pub outcome_reference: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct GraphDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub name: String,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    pub start: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Canonical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<Vec<GraphDefinitionLink>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(rename = "sliceName", skip_serializing_if = "Option::is_none")]
    pub slice_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<GraphDefinitionLinkTarget>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compartment: Option<Vec<GraphDefinitionLinkTargetCompartment>>,
    #[serde(rename = "use")]
    pub r#use: Code,
    pub code: Code,
    pub rule: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
}


/// Choice of types for the value[x] field in Group
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum GroupValueChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Group {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<Boolean>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub actual: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<UnsignedInt>,
    #[serde(rename = "managingEntity", skip_serializing_if = "Option::is_none")]
    pub managing_entity: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub characteristic: Option<Vec<GroupCharacteristic>>,
    #[serde(flatten)]
    pub value: Option<GroupValueChoice>,
    pub exclude: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member: Option<Vec<GroupMember>>,
    pub entity: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inactive: Option<Boolean>,
}


/// Choice of types for the module[x] field in GuidanceResponse
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum GuidanceResponseModuleChoice {
    /// Variant accepting the Uri type.
    #[serde(rename = "moduleUri")]
    Uri(Uri),
    /// Variant accepting the Canonical type.
    #[serde(rename = "moduleCanonical")]
    Canonical(Canonical),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "moduleCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct GuidanceResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "requestIdentifier", skip_serializing_if = "Option::is_none")]
    pub request_identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(flatten)]
    pub module: Option<GuidanceResponseModuleChoice>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(rename = "occurrenceDateTime", skip_serializing_if = "Option::is_none")]
    pub occurrence_date_time: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Reference>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "evaluationMessage", skip_serializing_if = "Option::is_none")]
    pub evaluation_message: Option<Vec<Reference>>,
    #[serde(rename = "outputParameters", skip_serializing_if = "Option::is_none")]
    pub output_parameters: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Reference>,
    #[serde(rename = "dataRequirement", skip_serializing_if = "Option::is_none")]
    pub data_requirement: Option<Vec<DataRequirement>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct HealthcareService {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<Boolean>,
    #[serde(rename = "providedBy", skip_serializing_if = "Option::is_none")]
    pub provided_by: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specialty: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(rename = "extraDetails", skip_serializing_if = "Option::is_none")]
    pub extra_details: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo: Option<Attachment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(rename = "coverageArea", skip_serializing_if = "Option::is_none")]
    pub coverage_area: Option<Vec<Reference>>,
    #[serde(rename = "serviceProvisionCode", skip_serializing_if = "Option::is_none")]
    pub service_provision_code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eligibility: Option<Vec<HealthcareServiceEligibility>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub characteristic: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub communication: Option<Vec<CodeableConcept>>,
    #[serde(rename = "referralMethod", skip_serializing_if = "Option::is_none")]
    pub referral_method: Option<Vec<CodeableConcept>>,
    #[serde(rename = "appointmentRequired", skip_serializing_if = "Option::is_none")]
    pub appointment_required: Option<Boolean>,
    #[serde(rename = "availableTime", skip_serializing_if = "Option::is_none")]
    pub available_time: Option<Vec<HealthcareServiceAvailableTime>>,
    #[serde(rename = "daysOfWeek", skip_serializing_if = "Option::is_none")]
    pub days_of_week: Option<Vec<Code>>,
    #[serde(rename = "allDay", skip_serializing_if = "Option::is_none")]
    pub all_day: Option<Boolean>,
    #[serde(rename = "availableStartTime", skip_serializing_if = "Option::is_none")]
    pub available_start_time: Option<Time>,
    #[serde(rename = "availableEndTime", skip_serializing_if = "Option::is_none")]
    pub available_end_time: Option<Time>,
    #[serde(rename = "notAvailable", skip_serializing_if = "Option::is_none")]
    pub not_available: Option<Vec<HealthcareServiceNotAvailable>>,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub during: Option<Period>,
    #[serde(rename = "availabilityExceptions", skip_serializing_if = "Option::is_none")]
    pub availability_exceptions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ImagingStudy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modality: Option<Vec<Coding>>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started: Option<DateTime>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referrer: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interpreter: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<Reference>>,
    #[serde(rename = "numberOfSeries", skip_serializing_if = "Option::is_none")]
    pub number_of_series: Option<UnsignedInt>,
    #[serde(rename = "numberOfInstances", skip_serializing_if = "Option::is_none")]
    pub number_of_instances: Option<UnsignedInt>,
    #[serde(rename = "procedureReference", skip_serializing_if = "Option::is_none")]
    pub procedure_reference: Option<Reference>,
    #[serde(rename = "procedureCode", skip_serializing_if = "Option::is_none")]
    pub procedure_code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<Vec<ImagingStudySeries>>,
    pub uid: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<UnsignedInt>,
    #[serde(rename = "bodySite", skip_serializing_if = "Option::is_none")]
    pub body_site: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub laterality: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specimen: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<ImagingStudySeriesPerformer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<Vec<ImagingStudySeriesInstance>>,
    #[serde(rename = "sopClass")]
    pub sop_class: Coding,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}


/// Choice of types for the occurrence[x] field in Immunization
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationOccurrenceChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the String type.
    #[serde(rename = "occurrenceString")]
    String(String),
}

/// Choice of types for the doseNumber[x] field in Immunization
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationDoseNumberChoice {
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "doseNumberPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "doseNumberString")]
    String(String),
}

/// Choice of types for the seriesDoses[x] field in Immunization
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationSeriesDosesChoice {
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "seriesDosesPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "seriesDosesString")]
    String(String),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Immunization {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "statusReason", skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<CodeableConcept>,
    #[serde(rename = "vaccineCode")]
    pub vaccine_code: CodeableConcept,
    pub patient: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(flatten)]
    pub occurrence: Option<ImmunizationOccurrenceChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded: Option<DateTime>,
    #[serde(rename = "primarySource", skip_serializing_if = "Option::is_none")]
    pub primary_source: Option<Boolean>,
    #[serde(rename = "reportOrigin", skip_serializing_if = "Option::is_none")]
    pub report_origin: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<Reference>,
    #[serde(rename = "lotNumber", skip_serializing_if = "Option::is_none")]
    pub lot_number: Option<String>,
    #[serde(rename = "expirationDate", skip_serializing_if = "Option::is_none")]
    pub expiration_date: Option<Date>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<CodeableConcept>,
    #[serde(rename = "doseQuantity", skip_serializing_if = "Option::is_none")]
    pub dose_quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<ImmunizationPerformer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "isSubpotent", skip_serializing_if = "Option::is_none")]
    pub is_subpotent: Option<Boolean>,
    #[serde(rename = "subpotentReason", skip_serializing_if = "Option::is_none")]
    pub subpotent_reason: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub education: Option<Vec<ImmunizationEducation>>,
    #[serde(rename = "documentType", skip_serializing_if = "Option::is_none")]
    pub document_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Uri>,
    #[serde(rename = "publicationDate", skip_serializing_if = "Option::is_none")]
    pub publication_date: Option<DateTime>,
    #[serde(rename = "presentationDate", skip_serializing_if = "Option::is_none")]
    pub presentation_date: Option<DateTime>,
    #[serde(rename = "programEligibility", skip_serializing_if = "Option::is_none")]
    pub program_eligibility: Option<Vec<CodeableConcept>>,
    #[serde(rename = "fundingSource", skip_serializing_if = "Option::is_none")]
    pub funding_source: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reaction: Option<Vec<ImmunizationReaction>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported: Option<Boolean>,
    #[serde(rename = "protocolApplied", skip_serializing_if = "Option::is_none")]
    pub protocol_applied: Option<Vec<ImmunizationProtocolApplied>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<Reference>,
    #[serde(rename = "targetDisease", skip_serializing_if = "Option::is_none")]
    pub target_disease: Option<Vec<CodeableConcept>>,
    #[serde(flatten)]
    pub dose_number: Option<ImmunizationDoseNumberChoice>,
    #[serde(flatten)]
    pub series_doses: Option<ImmunizationSeriesDosesChoice>,
}


/// Choice of types for the doseNumber[x] field in ImmunizationEvaluation
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationEvaluationDoseNumberChoice {
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "doseNumberPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "doseNumberString")]
    String(String),
}

/// Choice of types for the seriesDoses[x] field in ImmunizationEvaluation
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationEvaluationSeriesDosesChoice {
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "seriesDosesPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "seriesDosesString")]
    String(String),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ImmunizationEvaluation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub patient: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<Reference>,
    #[serde(rename = "targetDisease")]
    pub target_disease: CodeableConcept,
    #[serde(rename = "immunizationEvent")]
    pub immunization_event: Reference,
    #[serde(rename = "doseStatus")]
    pub dose_status: CodeableConcept,
    #[serde(rename = "doseStatusReason", skip_serializing_if = "Option::is_none")]
    pub dose_status_reason: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    #[serde(flatten)]
    pub dose_number: Option<ImmunizationEvaluationDoseNumberChoice>,
    #[serde(flatten)]
    pub series_doses: Option<ImmunizationEvaluationSeriesDosesChoice>,
}


/// Choice of types for the doseNumber[x] field in ImmunizationRecommendation
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationRecommendationDoseNumberChoice {
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "doseNumberPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "doseNumberString")]
    String(String),
}

/// Choice of types for the seriesDoses[x] field in ImmunizationRecommendation
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationRecommendationSeriesDosesChoice {
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "seriesDosesPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "seriesDosesString")]
    String(String),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ImmunizationRecommendation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub patient: Reference,
    pub date: DateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommendation: Option<Vec<ImmunizationRecommendationRecommendation>>,
    #[serde(rename = "vaccineCode", skip_serializing_if = "Option::is_none")]
    pub vaccine_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "targetDisease", skip_serializing_if = "Option::is_none")]
    pub target_disease: Option<CodeableConcept>,
    #[serde(rename = "contraindicatedVaccineCode", skip_serializing_if = "Option::is_none")]
    pub contraindicated_vaccine_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "forecastStatus")]
    pub forecast_status: CodeableConcept,
    #[serde(rename = "forecastReason", skip_serializing_if = "Option::is_none")]
    pub forecast_reason: Option<Vec<CodeableConcept>>,
    #[serde(rename = "dateCriterion", skip_serializing_if = "Option::is_none")]
    pub date_criterion: Option<Vec<ImmunizationRecommendationRecommendationDateCriterion>>,
    pub code: CodeableConcept,
    pub value: DateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    #[serde(flatten)]
    pub dose_number: Option<ImmunizationRecommendationDoseNumberChoice>,
    #[serde(flatten)]
    pub series_doses: Option<ImmunizationRecommendationSeriesDosesChoice>,
    #[serde(rename = "supportingImmunization", skip_serializing_if = "Option::is_none")]
    pub supporting_immunization: Option<Vec<Reference>>,
    #[serde(rename = "supportingPatientInformation", skip_serializing_if = "Option::is_none")]
    pub supporting_patient_information: Option<Vec<Reference>>,
}


/// Choice of types for the example[x] field in ImplementationGuide
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ImplementationGuideExampleChoice {
    /// Variant accepting the Boolean type.
    #[serde(rename = "exampleBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "exampleCanonical")]
    Canonical(Canonical),
}

/// Choice of types for the name[x] field in ImplementationGuide
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ImplementationGuideNameChoice {
    /// Variant accepting the Url type.
    #[serde(rename = "nameUrl")]
    Url(Url),
    /// Variant accepting the Reference type.
    #[serde(rename = "nameReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ImplementationGuide {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "packageId")]
    pub package_id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<Code>,
    #[serde(rename = "fhirVersion", skip_serializing_if = "Option::is_none")]
    pub fhir_version: Option<Vec<Code>>,
    #[serde(rename = "dependsOn", skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<ImplementationGuideDependsOn>>,
    pub uri: Canonical,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global: Option<Vec<ImplementationGuideGlobal>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub profile: Canonical,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<ImplementationGuideDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grouping: Option<Vec<ImplementationGuideDefinitionGrouping>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Vec<ImplementationGuideDefinitionResource>>,
    pub reference: Reference,
    #[serde(flatten)]
    pub example: Option<ImplementationGuideExampleChoice>,
    #[serde(rename = "groupingId", skip_serializing_if = "Option::is_none")]
    pub grouping_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<ImplementationGuideDefinitionPage>,
    pub generation: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<ImplementationGuideDefinitionParameter>>,
    pub code: Code,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<Vec<ImplementationGuideDefinitionTemplate>>,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<ImplementationGuideManifest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rendering: Option<Url>,
    #[serde(rename = "relativePath", skip_serializing_if = "Option::is_none")]
    pub relative_path: Option<Url>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other: Option<Vec<String>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct InsurancePlan {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(rename = "ownedBy", skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<Reference>,
    #[serde(rename = "administeredBy", skip_serializing_if = "Option::is_none")]
    pub administered_by: Option<Reference>,
    #[serde(rename = "coverageArea", skip_serializing_if = "Option::is_none")]
    pub coverage_area: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<InsurancePlanContact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<Address>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Vec<InsurancePlanCoverage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub benefit: Option<Vec<InsurancePlanCoverageBenefit>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<Vec<InsurancePlanCoverageBenefitLimit>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<Vec<InsurancePlanPlan>>,
    #[serde(rename = "generalCost", skip_serializing_if = "Option::is_none")]
    pub general_cost: Option<Vec<InsurancePlanPlanGeneralCost>>,
    #[serde(rename = "groupSize", skip_serializing_if = "Option::is_none")]
    pub group_size: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(rename = "specificCost", skip_serializing_if = "Option::is_none")]
    pub specific_cost: Option<Vec<InsurancePlanPlanSpecificCost>>,
    pub category: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applicability: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualifiers: Option<Vec<CodeableConcept>>,
}


/// Choice of types for the chargeItem[x] field in Invoice
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum InvoiceChargeItemChoice {
    /// Variant accepting the Reference type.
    #[serde(rename = "chargeItemReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "chargeItemCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Invoice {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "cancelledReason", skip_serializing_if = "Option::is_none")]
    pub cancelled_reason: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant: Option<Vec<InvoiceParticipant>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
    pub actor: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Reference>,
    #[serde(rename = "lineItem", skip_serializing_if = "Option::is_none")]
    pub line_item: Option<Vec<InvoiceLineItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<PositiveInt>,
    #[serde(flatten)]
    pub charge_item: Option<InvoiceChargeItemChoice>,
    #[serde(rename = "priceComponent", skip_serializing_if = "Option::is_none")]
    pub price_component: Option<Vec<InvoiceLineItemPriceComponent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Money>,
    #[serde(rename = "totalPriceComponent", skip_serializing_if = "Option::is_none")]
    pub total_price_component: Option<Vec<InvoiceLineItemPriceComponent>>,
    #[serde(rename = "totalNet", skip_serializing_if = "Option::is_none")]
    pub total_net: Option<Money>,
    #[serde(rename = "totalGross", skip_serializing_if = "Option::is_none")]
    pub total_gross: Option<Money>,
    #[serde(rename = "paymentTerms", skip_serializing_if = "Option::is_none")]
    pub payment_terms: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}


/// Choice of types for the subject[x] field in Library
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum LibrarySubjectChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Library {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(flatten)]
    pub subject: Option<LibrarySubjectChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate", skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate", skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod", skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact", skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<ParameterDefinition>>,
    #[serde(rename = "dataRequirement", skip_serializing_if = "Option::is_none")]
    pub data_requirement: Option<Vec<DataRequirement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<Attachment>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Linkage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<LinkageItem>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub resource: Reference,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct List {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub mode: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Reference>,
    #[serde(rename = "orderedBy", skip_serializing_if = "Option::is_none")]
    pub ordered_by: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<Vec<ListEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flag: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Boolean>,
    pub item: Reference,
    #[serde(rename = "emptyReason", skip_serializing_if = "Option::is_none")]
    pub empty_reason: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Location {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(rename = "operationalStatus", skip_serializing_if = "Option::is_none")]
    pub operational_status: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Code>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<Address>,
    #[serde(rename = "physicalType", skip_serializing_if = "Option::is_none")]
    pub physical_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<LocationPosition>,
    pub longitude: Decimal,
    pub latitude: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub altitude: Option<Decimal>,
    #[serde(rename = "managingOrganization", skip_serializing_if = "Option::is_none")]
    pub managing_organization: Option<Reference>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Reference>,
    #[serde(rename = "hoursOfOperation", skip_serializing_if = "Option::is_none")]
    pub hours_of_operation: Option<Vec<LocationHoursOfOperation>>,
    #[serde(rename = "daysOfWeek", skip_serializing_if = "Option::is_none")]
    pub days_of_week: Option<Vec<Code>>,
    #[serde(rename = "allDay", skip_serializing_if = "Option::is_none")]
    pub all_day: Option<Boolean>,
    #[serde(rename = "openingTime", skip_serializing_if = "Option::is_none")]
    pub opening_time: Option<Time>,
    #[serde(rename = "closingTime", skip_serializing_if = "Option::is_none")]
    pub closing_time: Option<Time>,
    #[serde(rename = "availabilityExceptions", skip_serializing_if = "Option::is_none")]
    pub availability_exceptions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<Reference>>,
}


/// Choice of types for the subject[x] field in Measure
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MeasureSubjectChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Measure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(flatten)]
    pub subject: Option<MeasureSubjectChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate", skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate", skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod", skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact", skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disclaimer: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scoring: Option<CodeableConcept>,
    #[serde(rename = "compositeScoring", skip_serializing_if = "Option::is_none")]
    pub composite_scoring: Option<CodeableConcept>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "riskAdjustment", skip_serializing_if = "Option::is_none")]
    pub risk_adjustment: Option<String>,
    #[serde(rename = "rateAggregation", skip_serializing_if = "Option::is_none")]
    pub rate_aggregation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale: Option<Markdown>,
    #[serde(rename = "clinicalRecommendationStatement", skip_serializing_if = "Option::is_none")]
    pub clinical_recommendation_statement: Option<Markdown>,
    #[serde(rename = "improvementNotation", skip_serializing_if = "Option::is_none")]
    pub improvement_notation: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<Vec<Markdown>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guidance: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Vec<MeasureGroup>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub population: Option<Vec<MeasureGroupPopulation>>,
    pub criteria: Expression,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stratifier: Option<Vec<MeasureGroupStratifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<Vec<MeasureGroupStratifierComponent>>,
    #[serde(rename = "supplementalData", skip_serializing_if = "Option::is_none")]
    pub supplemental_data: Option<Vec<MeasureSupplementalData>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MeasureReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub measure: Canonical,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reporter: Option<Reference>,
    pub period: Period,
    #[serde(rename = "improvementNotation", skip_serializing_if = "Option::is_none")]
    pub improvement_notation: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Vec<MeasureReportGroup>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub population: Option<Vec<MeasureReportGroupPopulation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<Integer>,
    #[serde(rename = "subjectResults", skip_serializing_if = "Option::is_none")]
    pub subject_results: Option<Reference>,
    #[serde(rename = "measureScore", skip_serializing_if = "Option::is_none")]
    pub measure_score: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stratifier: Option<Vec<MeasureReportGroupStratifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stratum: Option<Vec<MeasureReportGroupStratifierStratum>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<Vec<MeasureReportGroupStratifierStratumComponent>>,
    #[serde(rename = "evaluatedResource", skip_serializing_if = "Option::is_none")]
    pub evaluated_resource: Option<Vec<Reference>>,
}


/// Choice of types for the created[x] field in Media
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MediaCreatedChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "createdDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "createdPeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Media {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modality: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(flatten)]
    pub created: Option<MediaCreatedChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued: Option<Instant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator: Option<Reference>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "bodySite", skip_serializing_if = "Option::is_none")]
    pub body_site: Option<CodeableConcept>,
    #[serde(rename = "deviceName", skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frames: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<Decimal>,
    pub content: Attachment,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}


/// Choice of types for the item[x] field in Medication
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationItemChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "itemReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Medication {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Ratio>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingredient: Option<Vec<MedicationIngredient>>,
    #[serde(flatten)]
    pub item: Option<MedicationItemChoice>,
    #[serde(rename = "isActive", skip_serializing_if = "Option::is_none")]
    pub is_active: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<Ratio>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch: Option<MedicationBatch>,
    #[serde(rename = "lotNumber", skip_serializing_if = "Option::is_none")]
    pub lot_number: Option<String>,
    #[serde(rename = "expirationDate", skip_serializing_if = "Option::is_none")]
    pub expiration_date: Option<DateTime>,
}


/// Choice of types for the medication[x] field in MedicationAdministration
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationAdministrationMedicationChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "medicationReference")]
    Reference(Reference),
}

/// Choice of types for the effective[x] field in MedicationAdministration
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationAdministrationEffectiveChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "effectivePeriod")]
    Period(Period),
}

/// Choice of types for the rate[x] field in MedicationAdministration
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationAdministrationRateChoice {
    /// Variant accepting the Ratio type.
    #[serde(rename = "rateRatio")]
    Ratio(Ratio),
    /// Variant accepting the Quantity type.
    #[serde(rename = "rateQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MedicationAdministration {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates: Option<Vec<Uri>>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReason", skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(flatten)]
    pub medication: Option<MedicationAdministrationMedicationChoice>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Reference>,
    #[serde(rename = "supportingInformation", skip_serializing_if = "Option::is_none")]
    pub supporting_information: Option<Vec<Reference>>,
    #[serde(flatten)]
    pub effective: Option<MedicationAdministrationEffectiveChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<MedicationAdministrationPerformer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dosage: Option<MedicationAdministrationDosage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dose: Option<Quantity>,
    #[serde(flatten)]
    pub rate: Option<MedicationAdministrationRateChoice>,
    #[serde(rename = "eventHistory", skip_serializing_if = "Option::is_none")]
    pub event_history: Option<Vec<Reference>>,
}


/// Choice of types for the statusReason[x] field in MedicationDispense
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationDispenseStatusReasonChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "statusReasonCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "statusReasonReference")]
    Reference(Reference),
}

/// Choice of types for the medication[x] field in MedicationDispense
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationDispenseMedicationChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "medicationReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MedicationDispense {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(flatten)]
    pub status_reason: Option<MedicationDispenseStatusReasonChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(flatten)]
    pub medication: Option<MedicationDispenseMedicationChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Reference>,
    #[serde(rename = "supportingInformation", skip_serializing_if = "Option::is_none")]
    pub supporting_information: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<MedicationDispensePerformer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(rename = "authorizingPrescription", skip_serializing_if = "Option::is_none")]
    pub authorizing_prescription: Option<Vec<Reference>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "daysSupply", skip_serializing_if = "Option::is_none")]
    pub days_supply: Option<Quantity>,
    #[serde(rename = "whenPrepared", skip_serializing_if = "Option::is_none")]
    pub when_prepared: Option<DateTime>,
    #[serde(rename = "whenHandedOver", skip_serializing_if = "Option::is_none")]
    pub when_handed_over: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "dosageInstruction", skip_serializing_if = "Option::is_none")]
    pub dosage_instruction: Option<Vec<Dosage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substitution: Option<MedicationDispenseSubstitution>,
    #[serde(rename = "wasSubstituted")]
    pub was_substituted: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Vec<CodeableConcept>>,
    #[serde(rename = "responsibleParty", skip_serializing_if = "Option::is_none")]
    pub responsible_party: Option<Vec<Reference>>,
    #[serde(rename = "detectedIssue", skip_serializing_if = "Option::is_none")]
    pub detected_issue: Option<Vec<Reference>>,
    #[serde(rename = "eventHistory", skip_serializing_if = "Option::is_none")]
    pub event_history: Option<Vec<Reference>>,
}


/// Choice of types for the item[x] field in MedicationKnowledge
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeItemChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "itemReference")]
    Reference(Reference),
}

/// Choice of types for the indication[x] field in MedicationKnowledge
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeIndicationChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "indicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "indicationReference")]
    Reference(Reference),
}

/// Choice of types for the characteristic[x] field in MedicationKnowledge
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeCharacteristicChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "characteristicCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[serde(rename = "characteristicQuantity")]
    Quantity(Quantity),
}

/// Choice of types for the value[x] field in MedicationKnowledge
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeValueChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MedicationKnowledge {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<Reference>,
    #[serde(rename = "doseForm", skip_serializing_if = "Option::is_none")]
    pub dose_form: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synonym: Option<Vec<String>>,
    #[serde(rename = "relatedMedicationKnowledge", skip_serializing_if = "Option::is_none")]
    pub related_medication_knowledge: Option<Vec<MedicationKnowledgeRelatedMedicationKnowledge>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Vec<Reference>>,
    #[serde(rename = "associatedMedication", skip_serializing_if = "Option::is_none")]
    pub associated_medication: Option<Vec<Reference>>,
    #[serde(rename = "productType", skip_serializing_if = "Option::is_none")]
    pub product_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monograph: Option<Vec<MedicationKnowledgeMonograph>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingredient: Option<Vec<MedicationKnowledgeIngredient>>,
    #[serde(flatten)]
    pub item: Option<MedicationKnowledgeItemChoice>,
    #[serde(rename = "isActive", skip_serializing_if = "Option::is_none")]
    pub is_active: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<Ratio>,
    #[serde(rename = "preparationInstruction", skip_serializing_if = "Option::is_none")]
    pub preparation_instruction: Option<Markdown>,
    #[serde(rename = "intendedRoute", skip_serializing_if = "Option::is_none")]
    pub intended_route: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<Vec<MedicationKnowledgeCost>>,
    #[serde(rename = "monitoringProgram", skip_serializing_if = "Option::is_none")]
    pub monitoring_program: Option<Vec<MedicationKnowledgeMonitoringProgram>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "administrationGuidelines", skip_serializing_if = "Option::is_none")]
    pub administration_guidelines: Option<Vec<MedicationKnowledgeAdministrationGuidelines>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dosage: Option<Vec<MedicationKnowledgeAdministrationGuidelinesDosage>>,
    #[serde(flatten)]
    pub indication: Option<MedicationKnowledgeIndicationChoice>,
    #[serde(rename = "patientCharacteristics", skip_serializing_if = "Option::is_none")]
    pub patient_characteristics: Option<Vec<MedicationKnowledgeAdministrationGuidelinesPatientCharacteristics>>,
    #[serde(flatten)]
    pub characteristic: Option<MedicationKnowledgeCharacteristicChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Vec<String>>,
    #[serde(rename = "medicineClassification", skip_serializing_if = "Option::is_none")]
    pub medicine_classification: Option<Vec<MedicationKnowledgeMedicineClassification>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packaging: Option<MedicationKnowledgePackaging>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "drugCharacteristic", skip_serializing_if = "Option::is_none")]
    pub drug_characteristic: Option<Vec<MedicationKnowledgeDrugCharacteristic>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contraindication: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regulatory: Option<Vec<MedicationKnowledgeRegulatory>>,
    #[serde(rename = "regulatoryAuthority")]
    pub regulatory_authority: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substitution: Option<Vec<MedicationKnowledgeRegulatorySubstitution>>,
    pub allowed: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Vec<MedicationKnowledgeRegulatorySchedule>>,
    #[serde(rename = "maxDispense", skip_serializing_if = "Option::is_none")]
    pub max_dispense: Option<MedicationKnowledgeRegulatoryMaxDispense>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Duration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kinetics: Option<Vec<MedicationKnowledgeKinetics>>,
    #[serde(rename = "areaUnderCurve", skip_serializing_if = "Option::is_none")]
    pub area_under_curve: Option<Vec<Quantity>>,
    #[serde(rename = "lethalDose50", skip_serializing_if = "Option::is_none")]
    pub lethal_dose50: Option<Vec<Quantity>>,
    #[serde(rename = "halfLifePeriod", skip_serializing_if = "Option::is_none")]
    pub half_life_period: Option<Duration>,
}


/// Choice of types for the reported[x] field in MedicationRequest
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationRequestReportedChoice {
    /// Variant accepting the Boolean type.
    #[serde(rename = "reportedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Reference type.
    #[serde(rename = "reportedReference")]
    Reference(Reference),
}

/// Choice of types for the medication[x] field in MedicationRequest
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationRequestMedicationChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "medicationReference")]
    Reference(Reference),
}

/// Choice of types for the allowed[x] field in MedicationRequest
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationRequestAllowedChoice {
    /// Variant accepting the Boolean type.
    #[serde(rename = "allowedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "allowedCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MedicationRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "statusReason", skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<CodeableConcept>,
    pub intent: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Code>,
    #[serde(rename = "doNotPerform", skip_serializing_if = "Option::is_none")]
    pub do_not_perform: Option<Boolean>,
    #[serde(flatten)]
    pub reported: Option<MedicationRequestReportedChoice>,
    #[serde(flatten)]
    pub medication: Option<MedicationRequestMedicationChoice>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(rename = "supportingInformation", skip_serializing_if = "Option::is_none")]
    pub supporting_information: Option<Vec<Reference>>,
    #[serde(rename = "authoredOn", skip_serializing_if = "Option::is_none")]
    pub authored_on: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Reference>,
    #[serde(rename = "performerType", skip_serializing_if = "Option::is_none")]
    pub performer_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorder: Option<Reference>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "instantiatesCanonical", skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri", skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "groupIdentifier", skip_serializing_if = "Option::is_none")]
    pub group_identifier: Option<Identifier>,
    #[serde(rename = "courseOfTherapyType", skip_serializing_if = "Option::is_none")]
    pub course_of_therapy_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurance: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "dosageInstruction", skip_serializing_if = "Option::is_none")]
    pub dosage_instruction: Option<Vec<Dosage>>,
    #[serde(rename = "dispenseRequest", skip_serializing_if = "Option::is_none")]
    pub dispense_request: Option<MedicationRequestDispenseRequest>,
    #[serde(rename = "initialFill", skip_serializing_if = "Option::is_none")]
    pub initial_fill: Option<MedicationRequestDispenseRequestInitialFill>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<Duration>,
    #[serde(rename = "dispenseInterval", skip_serializing_if = "Option::is_none")]
    pub dispense_interval: Option<Duration>,
    #[serde(rename = "validityPeriod", skip_serializing_if = "Option::is_none")]
    pub validity_period: Option<Period>,
    #[serde(rename = "numberOfRepeatsAllowed", skip_serializing_if = "Option::is_none")]
    pub number_of_repeats_allowed: Option<UnsignedInt>,
    #[serde(rename = "expectedSupplyDuration", skip_serializing_if = "Option::is_none")]
    pub expected_supply_duration: Option<Duration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substitution: Option<MedicationRequestSubstitution>,
    #[serde(flatten)]
    pub allowed: Option<MedicationRequestAllowedChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<CodeableConcept>,
    #[serde(rename = "priorPrescription", skip_serializing_if = "Option::is_none")]
    pub prior_prescription: Option<Reference>,
    #[serde(rename = "detectedIssue", skip_serializing_if = "Option::is_none")]
    pub detected_issue: Option<Vec<Reference>>,
    #[serde(rename = "eventHistory", skip_serializing_if = "Option::is_none")]
    pub event_history: Option<Vec<Reference>>,
}


/// Choice of types for the medication[x] field in MedicationStatement
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationStatementMedicationChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "medicationReference")]
    Reference(Reference),
}

/// Choice of types for the effective[x] field in MedicationStatement
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationStatementEffectiveChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "effectivePeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MedicationStatement {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReason", skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(flatten)]
    pub medication: Option<MedicationStatementMedicationChoice>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Reference>,
    #[serde(flatten)]
    pub effective: Option<MedicationStatementEffectiveChoice>,
    #[serde(rename = "dateAsserted", skip_serializing_if = "Option::is_none")]
    pub date_asserted: Option<DateTime>,
    #[serde(rename = "informationSource", skip_serializing_if = "Option::is_none")]
    pub information_source: Option<Reference>,
    #[serde(rename = "derivedFrom", skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<Vec<Reference>>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dosage: Option<Vec<Dosage>>,
}


/// Choice of types for the indication[x] field in MedicinalProduct
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductIndicationChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "indicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "indicationReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MedicinalProduct {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<Coding>,
    #[serde(rename = "combinedPharmaceuticalDoseForm", skip_serializing_if = "Option::is_none")]
    pub combined_pharmaceutical_dose_form: Option<CodeableConcept>,
    #[serde(rename = "legalStatusOfSupply", skip_serializing_if = "Option::is_none")]
    pub legal_status_of_supply: Option<CodeableConcept>,
    #[serde(rename = "additionalMonitoringIndicator", skip_serializing_if = "Option::is_none")]
    pub additional_monitoring_indicator: Option<CodeableConcept>,
    #[serde(rename = "specialMeasures", skip_serializing_if = "Option::is_none")]
    pub special_measures: Option<Vec<String>>,
    #[serde(rename = "paediatricUseIndicator", skip_serializing_if = "Option::is_none")]
    pub paediatric_use_indicator: Option<CodeableConcept>,
    #[serde(rename = "productClassification", skip_serializing_if = "Option::is_none")]
    pub product_classification: Option<Vec<CodeableConcept>>,
    #[serde(rename = "marketingStatus", skip_serializing_if = "Option::is_none")]
    pub marketing_status: Option<Vec<MarketingStatus>>,
    #[serde(rename = "pharmaceuticalProduct", skip_serializing_if = "Option::is_none")]
    pub pharmaceutical_product: Option<Vec<Reference>>,
    #[serde(rename = "packagedMedicinalProduct", skip_serializing_if = "Option::is_none")]
    pub packaged_medicinal_product: Option<Vec<Reference>>,
    #[serde(rename = "attachedDocument", skip_serializing_if = "Option::is_none")]
    pub attached_document: Option<Vec<Reference>>,
    #[serde(rename = "masterFile", skip_serializing_if = "Option::is_none")]
    pub master_file: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<Reference>>,
    #[serde(rename = "clinicalTrial", skip_serializing_if = "Option::is_none")]
    pub clinical_trial: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Vec<MedicinalProductName>>,
    #[serde(rename = "productName")]
    pub product_name: String,
    #[serde(rename = "namePart", skip_serializing_if = "Option::is_none")]
    pub name_part: Option<Vec<MedicinalProductNameNamePart>>,
    pub part: String,
    #[serde(rename = "countryLanguage", skip_serializing_if = "Option::is_none")]
    pub country_language: Option<Vec<MedicinalProductNameCountryLanguage>>,
    pub country: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<CodeableConcept>,
    #[serde(rename = "crossReference", skip_serializing_if = "Option::is_none")]
    pub cross_reference: Option<Vec<Identifier>>,
    #[serde(rename = "manufacturingBusinessOperation", skip_serializing_if = "Option::is_none")]
    pub manufacturing_business_operation: Option<Vec<MedicinalProductManufacturingBusinessOperation>>,
    #[serde(rename = "operationType", skip_serializing_if = "Option::is_none")]
    pub operation_type: Option<CodeableConcept>,
    #[serde(rename = "authorisationReferenceNumber", skip_serializing_if = "Option::is_none")]
    pub authorisation_reference_number: Option<Identifier>,
    #[serde(rename = "effectiveDate", skip_serializing_if = "Option::is_none")]
    pub effective_date: Option<DateTime>,
    #[serde(rename = "confidentialityIndicator", skip_serializing_if = "Option::is_none")]
    pub confidentiality_indicator: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regulator: Option<Reference>,
    #[serde(rename = "specialDesignation", skip_serializing_if = "Option::is_none")]
    pub special_designation: Option<Vec<MedicinalProductSpecialDesignation>>,
    #[serde(rename = "intendedUse", skip_serializing_if = "Option::is_none")]
    pub intended_use: Option<CodeableConcept>,
    #[serde(flatten)]
    pub indication: Option<MedicinalProductIndicationChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub species: Option<CodeableConcept>,
}


/// Choice of types for the date[x] field in MedicinalProductAuthorization
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductAuthorizationDateChoice {
    /// Variant accepting the Period type.
    #[serde(rename = "datePeriod")]
    Period(Period),
    /// Variant accepting the DateTime type.
    #[serde(rename = "dateDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MedicinalProductAuthorization {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<CodeableConcept>,
    #[serde(rename = "statusDate", skip_serializing_if = "Option::is_none")]
    pub status_date: Option<DateTime>,
    #[serde(rename = "restoreDate", skip_serializing_if = "Option::is_none")]
    pub restore_date: Option<DateTime>,
    #[serde(rename = "validityPeriod", skip_serializing_if = "Option::is_none")]
    pub validity_period: Option<Period>,
    #[serde(rename = "dataExclusivityPeriod", skip_serializing_if = "Option::is_none")]
    pub data_exclusivity_period: Option<Period>,
    #[serde(rename = "dateOfFirstAuthorization", skip_serializing_if = "Option::is_none")]
    pub date_of_first_authorization: Option<DateTime>,
    #[serde(rename = "internationalBirthDate", skip_serializing_if = "Option::is_none")]
    pub international_birth_date: Option<DateTime>,
    #[serde(rename = "legalBasis", skip_serializing_if = "Option::is_none")]
    pub legal_basis: Option<CodeableConcept>,
    #[serde(rename = "jurisdictionalAuthorization", skip_serializing_if = "Option::is_none")]
    pub jurisdictional_authorization: Option<Vec<MedicinalProductAuthorizationJurisdictionalAuthorization>>,
    #[serde(rename = "legalStatusOfSupply", skip_serializing_if = "Option::is_none")]
    pub legal_status_of_supply: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub holder: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regulator: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure: Option<MedicinalProductAuthorizationProcedure>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(flatten)]
    pub date: Option<MedicinalProductAuthorizationDateChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub application: Option<Vec<MedicinalProductAuthorizationProcedure>>,
}


/// Choice of types for the medication[x] field in MedicinalProductContraindication
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductContraindicationMedicationChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "medicationReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MedicinalProductContraindication {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disease: Option<CodeableConcept>,
    #[serde(rename = "diseaseStatus", skip_serializing_if = "Option::is_none")]
    pub disease_status: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comorbidity: Option<Vec<CodeableConcept>>,
    #[serde(rename = "therapeuticIndication", skip_serializing_if = "Option::is_none")]
    pub therapeutic_indication: Option<Vec<Reference>>,
    #[serde(rename = "otherTherapy", skip_serializing_if = "Option::is_none")]
    pub other_therapy: Option<Vec<MedicinalProductContraindicationOtherTherapy>>,
    #[serde(rename = "therapyRelationshipType")]
    pub therapy_relationship_type: CodeableConcept,
    #[serde(flatten)]
    pub medication: Option<MedicinalProductContraindicationMedicationChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub population: Option<Vec<Population>>,
}


/// Choice of types for the medication[x] field in MedicinalProductIndication
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductIndicationMedicationChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "medicationReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MedicinalProductIndication {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Vec<Reference>>,
    #[serde(rename = "diseaseSymptomProcedure", skip_serializing_if = "Option::is_none")]
    pub disease_symptom_procedure: Option<CodeableConcept>,
    #[serde(rename = "diseaseStatus", skip_serializing_if = "Option::is_none")]
    pub disease_status: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comorbidity: Option<Vec<CodeableConcept>>,
    #[serde(rename = "intendedEffect", skip_serializing_if = "Option::is_none")]
    pub intended_effect: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<Quantity>,
    #[serde(rename = "otherTherapy", skip_serializing_if = "Option::is_none")]
    pub other_therapy: Option<Vec<MedicinalProductIndicationOtherTherapy>>,
    #[serde(rename = "therapyRelationshipType")]
    pub therapy_relationship_type: CodeableConcept,
    #[serde(flatten)]
    pub medication: Option<MedicinalProductIndicationMedicationChoice>,
    #[serde(rename = "undesirableEffect", skip_serializing_if = "Option::is_none")]
    pub undesirable_effect: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub population: Option<Vec<Population>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MedicinalProductIngredient {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    pub role: CodeableConcept,
    #[serde(rename = "allergenicIndicator", skip_serializing_if = "Option::is_none")]
    pub allergenic_indicator: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<Vec<Reference>>,
    #[serde(rename = "specifiedSubstance", skip_serializing_if = "Option::is_none")]
    pub specified_substance: Option<Vec<MedicinalProductIngredientSpecifiedSubstance>>,
    pub code: CodeableConcept,
    pub group: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidentiality: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<Vec<MedicinalProductIngredientSpecifiedSubstanceStrength>>,
    pub presentation: Ratio,
    #[serde(rename = "presentationLowLimit", skip_serializing_if = "Option::is_none")]
    pub presentation_low_limit: Option<Ratio>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concentration: Option<Ratio>,
    #[serde(rename = "concentrationLowLimit", skip_serializing_if = "Option::is_none")]
    pub concentration_low_limit: Option<Ratio>,
    #[serde(rename = "measurementPoint", skip_serializing_if = "Option::is_none")]
    pub measurement_point: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<Vec<CodeableConcept>>,
    #[serde(rename = "referenceStrength", skip_serializing_if = "Option::is_none")]
    pub reference_strength: Option<Vec<MedicinalProductIngredientSpecifiedSubstanceStrengthReferenceStrength>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substance: Option<CodeableConcept>,
    #[serde(rename = "strengthLowLimit", skip_serializing_if = "Option::is_none")]
    pub strength_low_limit: Option<Ratio>,
}


/// Choice of types for the item[x] field in MedicinalProductInteraction
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductInteractionItemChoice {
    /// Variant accepting the Reference type.
    #[serde(rename = "itemReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MedicinalProductInteraction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interactant: Option<Vec<MedicinalProductInteractionInteractant>>,
    #[serde(flatten)]
    pub item: Option<MedicinalProductInteractionItemChoice>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incidence: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub management: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MedicinalProductManufactured {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "manufacturedDoseForm")]
    pub manufactured_dose_form: CodeableConcept,
    #[serde(rename = "unitOfPresentation", skip_serializing_if = "Option::is_none")]
    pub unit_of_presentation: Option<CodeableConcept>,
    pub quantity: Quantity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingredient: Option<Vec<Reference>>,
    #[serde(rename = "physicalCharacteristics", skip_serializing_if = "Option::is_none")]
    pub physical_characteristics: Option<ProdCharacteristic>,
    #[serde(rename = "otherCharacteristics", skip_serializing_if = "Option::is_none")]
    pub other_characteristics: Option<Vec<CodeableConcept>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MedicinalProductPackaged {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "legalStatusOfSupply", skip_serializing_if = "Option::is_none")]
    pub legal_status_of_supply: Option<CodeableConcept>,
    #[serde(rename = "marketingStatus", skip_serializing_if = "Option::is_none")]
    pub marketing_status: Option<Vec<MarketingStatus>>,
    #[serde(rename = "marketingAuthorization", skip_serializing_if = "Option::is_none")]
    pub marketing_authorization: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<Vec<Reference>>,
    #[serde(rename = "batchIdentifier", skip_serializing_if = "Option::is_none")]
    pub batch_identifier: Option<Vec<MedicinalProductPackagedBatchIdentifier>>,
    #[serde(rename = "outerPackaging")]
    pub outer_packaging: Identifier,
    #[serde(rename = "immediatePackaging", skip_serializing_if = "Option::is_none")]
    pub immediate_packaging: Option<Identifier>,
    #[serde(rename = "packageItem", skip_serializing_if = "Option::is_none")]
    pub package_item: Option<Vec<MedicinalProductPackagedPackageItem>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub quantity: Quantity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<Vec<CodeableConcept>>,
    #[serde(rename = "alternateMaterial", skip_serializing_if = "Option::is_none")]
    pub alternate_material: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<Vec<Reference>>,
    #[serde(rename = "manufacturedItem", skip_serializing_if = "Option::is_none")]
    pub manufactured_item: Option<Vec<Reference>>,
    #[serde(rename = "physicalCharacteristics", skip_serializing_if = "Option::is_none")]
    pub physical_characteristics: Option<ProdCharacteristic>,
    #[serde(rename = "otherCharacteristics", skip_serializing_if = "Option::is_none")]
    pub other_characteristics: Option<Vec<CodeableConcept>>,
    #[serde(rename = "shelfLifeStorage", skip_serializing_if = "Option::is_none")]
    pub shelf_life_storage: Option<Vec<ProductShelfLife>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MedicinalProductPharmaceutical {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "administrableDoseForm")]
    pub administrable_dose_form: CodeableConcept,
    #[serde(rename = "unitOfPresentation", skip_serializing_if = "Option::is_none")]
    pub unit_of_presentation: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingredient: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub characteristics: Option<Vec<MedicinalProductPharmaceuticalCharacteristics>>,
    pub code: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<CodeableConcept>,
    #[serde(rename = "routeOfAdministration", skip_serializing_if = "Option::is_none")]
    pub route_of_administration: Option<Vec<MedicinalProductPharmaceuticalRouteOfAdministration>>,
    #[serde(rename = "firstDose", skip_serializing_if = "Option::is_none")]
    pub first_dose: Option<Quantity>,
    #[serde(rename = "maxSingleDose", skip_serializing_if = "Option::is_none")]
    pub max_single_dose: Option<Quantity>,
    #[serde(rename = "maxDosePerDay", skip_serializing_if = "Option::is_none")]
    pub max_dose_per_day: Option<Quantity>,
    #[serde(rename = "maxDosePerTreatmentPeriod", skip_serializing_if = "Option::is_none")]
    pub max_dose_per_treatment_period: Option<Ratio>,
    #[serde(rename = "maxTreatmentPeriod", skip_serializing_if = "Option::is_none")]
    pub max_treatment_period: Option<Duration>,
    #[serde(rename = "targetSpecies", skip_serializing_if = "Option::is_none")]
    pub target_species: Option<Vec<MedicinalProductPharmaceuticalRouteOfAdministrationTargetSpecies>>,
    #[serde(rename = "withdrawalPeriod", skip_serializing_if = "Option::is_none")]
    pub withdrawal_period: Option<Vec<MedicinalProductPharmaceuticalRouteOfAdministrationTargetSpeciesWithdrawalPeriod>>,
    pub tissue: CodeableConcept,
    pub value: Quantity,
    #[serde(rename = "supportingInformation", skip_serializing_if = "Option::is_none")]
    pub supporting_information: Option<String>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MedicinalProductUndesirableEffect {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Vec<Reference>>,
    #[serde(rename = "symptomConditionEffect", skip_serializing_if = "Option::is_none")]
    pub symptom_condition_effect: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<CodeableConcept>,
    #[serde(rename = "frequencyOfOccurrence", skip_serializing_if = "Option::is_none")]
    pub frequency_of_occurrence: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub population: Option<Vec<Population>>,
}


/// Choice of types for the event[x] field in MessageDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MessageDefinitionEventChoice {
    /// Variant accepting the Coding type.
    #[serde(rename = "eventCoding")]
    Coding(Coding),
    /// Variant accepting the Uri type.
    #[serde(rename = "eventUri")]
    Uri(Uri),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MessageDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaces: Option<Vec<Canonical>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    pub date: DateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<Canonical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<Vec<Canonical>>,
    #[serde(flatten)]
    pub event: Option<MessageDefinitionEventChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<Vec<MessageDefinitionFocus>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Canonical>,
    pub min: UnsignedInt,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
    #[serde(rename = "responseRequired", skip_serializing_if = "Option::is_none")]
    pub response_required: Option<Code>,
    #[serde(rename = "allowedResponse", skip_serializing_if = "Option::is_none")]
    pub allowed_response: Option<Vec<MessageDefinitionAllowedResponse>>,
    pub message: Canonical,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub situation: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<Vec<Canonical>>,
}


/// Choice of types for the event[x] field in MessageHeader
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MessageHeaderEventChoice {
    /// Variant accepting the Coding type.
    #[serde(rename = "eventCoding")]
    Coding(Coding),
    /// Variant accepting the Uri type.
    #[serde(rename = "eventUri")]
    Uri(Uri),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MessageHeader {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub event: Option<MessageHeaderEventChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<Vec<MessageHeaderDestination>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Reference>,
    pub endpoint: Url,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterer: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Reference>,
    pub source: MessageHeaderSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub software: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<ContactPoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsible: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<MessageHeaderResponse>,
    pub identifier: Id,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<Canonical>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MolecularSequence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
    #[serde(rename = "coordinateSystem")]
    pub coordinate_system: Integer,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patient: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specimen: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "referenceSeq", skip_serializing_if = "Option::is_none")]
    pub reference_seq: Option<MolecularSequenceReferenceSeq>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chromosome: Option<CodeableConcept>,
    #[serde(rename = "genomeBuild", skip_serializing_if = "Option::is_none")]
    pub genome_build: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation: Option<Code>,
    #[serde(rename = "referenceSeqId", skip_serializing_if = "Option::is_none")]
    pub reference_seq_id: Option<CodeableConcept>,
    #[serde(rename = "referenceSeqPointer", skip_serializing_if = "Option::is_none")]
    pub reference_seq_pointer: Option<Reference>,
    #[serde(rename = "referenceSeqString", skip_serializing_if = "Option::is_none")]
    pub reference_seq_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strand: Option<Code>,
    #[serde(rename = "windowStart", skip_serializing_if = "Option::is_none")]
    pub window_start: Option<Integer>,
    #[serde(rename = "windowEnd", skip_serializing_if = "Option::is_none")]
    pub window_end: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<Vec<MolecularSequenceVariant>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<Integer>,
    #[serde(rename = "observedAllele", skip_serializing_if = "Option::is_none")]
    pub observed_allele: Option<String>,
    #[serde(rename = "referenceAllele", skip_serializing_if = "Option::is_none")]
    pub reference_allele: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cigar: Option<String>,
    #[serde(rename = "variantPointer", skip_serializing_if = "Option::is_none")]
    pub variant_pointer: Option<Reference>,
    #[serde(rename = "observedSeq", skip_serializing_if = "Option::is_none")]
    pub observed_seq: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<Vec<MolecularSequenceQuality>>,
    #[serde(rename = "standardSequence", skip_serializing_if = "Option::is_none")]
    pub standard_sequence: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(rename = "truthTP", skip_serializing_if = "Option::is_none")]
    pub truth_t_p: Option<Decimal>,
    #[serde(rename = "queryTP", skip_serializing_if = "Option::is_none")]
    pub query_t_p: Option<Decimal>,
    #[serde(rename = "truthFN", skip_serializing_if = "Option::is_none")]
    pub truth_f_n: Option<Decimal>,
    #[serde(rename = "queryFP", skip_serializing_if = "Option::is_none")]
    pub query_f_p: Option<Decimal>,
    #[serde(rename = "gtFP", skip_serializing_if = "Option::is_none")]
    pub gt_f_p: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precision: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recall: Option<Decimal>,
    #[serde(rename = "fScore", skip_serializing_if = "Option::is_none")]
    pub f_score: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roc: Option<MolecularSequenceQualityRoc>,
    #[serde(rename = "numTP", skip_serializing_if = "Option::is_none")]
    pub num_t_p: Option<Vec<Integer>>,
    #[serde(rename = "numFP", skip_serializing_if = "Option::is_none")]
    pub num_f_p: Option<Vec<Integer>>,
    #[serde(rename = "numFN", skip_serializing_if = "Option::is_none")]
    pub num_f_n: Option<Vec<Integer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<Vec<Decimal>>,
    #[serde(rename = "fMeasure", skip_serializing_if = "Option::is_none")]
    pub f_measure: Option<Vec<Decimal>>,
    #[serde(rename = "readCoverage", skip_serializing_if = "Option::is_none")]
    pub read_coverage: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<Vec<MolecularSequenceRepository>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "datasetId", skip_serializing_if = "Option::is_none")]
    pub dataset_id: Option<String>,
    #[serde(rename = "variantsetId", skip_serializing_if = "Option::is_none")]
    pub variantset_id: Option<String>,
    #[serde(rename = "readsetId", skip_serializing_if = "Option::is_none")]
    pub readset_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<Vec<Reference>>,
    #[serde(rename = "structureVariant", skip_serializing_if = "Option::is_none")]
    pub structure_variant: Option<Vec<MolecularSequenceStructureVariant>>,
    #[serde(rename = "variantType", skip_serializing_if = "Option::is_none")]
    pub variant_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exact: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outer: Option<MolecularSequenceStructureVariantOuter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inner: Option<MolecularSequenceStructureVariantInner>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct NamingSystem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub status: Code,
    pub kind: Code,
    pub date: DateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsible: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(rename = "uniqueId", skip_serializing_if = "Option::is_none")]
    pub unique_id: Option<Vec<NamingSystemUniqueId>>,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}


/// Choice of types for the rate[x] field in NutritionOrder
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum NutritionOrderRateChoice {
    /// Variant accepting the Quantity type.
    #[serde(rename = "rateQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Ratio type.
    #[serde(rename = "rateRatio")]
    Ratio(Ratio),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct NutritionOrder {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical", skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri", skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates: Option<Vec<Uri>>,
    pub status: Code,
    pub intent: Code,
    pub patient: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(rename = "dateTime")]
    pub date_time: DateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orderer: Option<Reference>,
    #[serde(rename = "allergyIntolerance", skip_serializing_if = "Option::is_none")]
    pub allergy_intolerance: Option<Vec<Reference>>,
    #[serde(rename = "foodPreferenceModifier", skip_serializing_if = "Option::is_none")]
    pub food_preference_modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "excludeFoodModifier", skip_serializing_if = "Option::is_none")]
    pub exclude_food_modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "oralDiet", skip_serializing_if = "Option::is_none")]
    pub oral_diet: Option<NutritionOrderOralDiet>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Vec<Timing>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nutrient: Option<Vec<NutritionOrderOralDietNutrient>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub texture: Option<Vec<NutritionOrderOralDietTexture>>,
    #[serde(rename = "foodType", skip_serializing_if = "Option::is_none")]
    pub food_type: Option<CodeableConcept>,
    #[serde(rename = "fluidConsistencyType", skip_serializing_if = "Option::is_none")]
    pub fluid_consistency_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supplement: Option<Vec<NutritionOrderSupplement>>,
    #[serde(rename = "productName", skip_serializing_if = "Option::is_none")]
    pub product_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "enteralFormula", skip_serializing_if = "Option::is_none")]
    pub enteral_formula: Option<NutritionOrderEnteralFormula>,
    #[serde(rename = "baseFormulaType", skip_serializing_if = "Option::is_none")]
    pub base_formula_type: Option<CodeableConcept>,
    #[serde(rename = "baseFormulaProductName", skip_serializing_if = "Option::is_none")]
    pub base_formula_product_name: Option<String>,
    #[serde(rename = "additiveType", skip_serializing_if = "Option::is_none")]
    pub additive_type: Option<CodeableConcept>,
    #[serde(rename = "additiveProductName", skip_serializing_if = "Option::is_none")]
    pub additive_product_name: Option<String>,
    #[serde(rename = "caloricDensity", skip_serializing_if = "Option::is_none")]
    pub caloric_density: Option<Quantity>,
    #[serde(rename = "routeofAdministration", skip_serializing_if = "Option::is_none")]
    pub routeof_administration: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub administration: Option<Vec<NutritionOrderEnteralFormulaAdministration>>,
    #[serde(flatten)]
    pub rate: Option<NutritionOrderRateChoice>,
    #[serde(rename = "maxVolumeToDeliver", skip_serializing_if = "Option::is_none")]
    pub max_volume_to_deliver: Option<Quantity>,
    #[serde(rename = "administrationInstruction", skip_serializing_if = "Option::is_none")]
    pub administration_instruction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}


/// Choice of types for the effective[x] field in Observation
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ObservationEffectiveChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "effectivePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[serde(rename = "effectiveTiming")]
    Timing(Timing),
    /// Variant accepting the Instant type.
    #[serde(rename = "effectiveInstant")]
    Instant(Instant),
}

/// Choice of types for the value[x] field in Observation
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ObservationValueChoice {
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the SampledData type.
    #[serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "valuePeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Observation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(flatten)]
    pub effective: Option<ObservationEffectiveChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued: Option<Instant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<Reference>>,
    #[serde(flatten)]
    pub value: Option<ObservationValueChoice>,
    #[serde(rename = "dataAbsentReason", skip_serializing_if = "Option::is_none")]
    pub data_absent_reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interpretation: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "bodySite", skip_serializing_if = "Option::is_none")]
    pub body_site: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specimen: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<Reference>,
    #[serde(rename = "referenceRange", skip_serializing_if = "Option::is_none")]
    pub reference_range: Option<Vec<ObservationReferenceRange>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high: Option<Quantity>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "appliesTo", skip_serializing_if = "Option::is_none")]
    pub applies_to: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age: Option<Range>,
    #[serde(rename = "hasMember", skip_serializing_if = "Option::is_none")]
    pub has_member: Option<Vec<Reference>>,
    #[serde(rename = "derivedFrom", skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<Vec<ObservationComponent>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ObservationDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "permittedDataType", skip_serializing_if = "Option::is_none")]
    pub permitted_data_type: Option<Vec<Code>>,
    #[serde(rename = "multipleResultsAllowed", skip_serializing_if = "Option::is_none")]
    pub multiple_results_allowed: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(rename = "preferredReportName", skip_serializing_if = "Option::is_none")]
    pub preferred_report_name: Option<String>,
    #[serde(rename = "quantitativeDetails", skip_serializing_if = "Option::is_none")]
    pub quantitative_details: Option<ObservationDefinitionQuantitativeDetails>,
    #[serde(rename = "customaryUnit", skip_serializing_if = "Option::is_none")]
    pub customary_unit: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<CodeableConcept>,
    #[serde(rename = "conversionFactor", skip_serializing_if = "Option::is_none")]
    pub conversion_factor: Option<Decimal>,
    #[serde(rename = "decimalPrecision", skip_serializing_if = "Option::is_none")]
    pub decimal_precision: Option<Integer>,
    #[serde(rename = "qualifiedInterval", skip_serializing_if = "Option::is_none")]
    pub qualified_interval: Option<Vec<ObservationDefinitionQualifiedInterval>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<CodeableConcept>,
    #[serde(rename = "appliesTo", skip_serializing_if = "Option::is_none")]
    pub applies_to: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age: Option<Range>,
    #[serde(rename = "gestationalAge", skip_serializing_if = "Option::is_none")]
    pub gestational_age: Option<Range>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(rename = "validCodedValueSet", skip_serializing_if = "Option::is_none")]
    pub valid_coded_value_set: Option<Reference>,
    #[serde(rename = "normalCodedValueSet", skip_serializing_if = "Option::is_none")]
    pub normal_coded_value_set: Option<Reference>,
    #[serde(rename = "abnormalCodedValueSet", skip_serializing_if = "Option::is_none")]
    pub abnormal_coded_value_set: Option<Reference>,
    #[serde(rename = "criticalCodedValueSet", skip_serializing_if = "Option::is_none")]
    pub critical_coded_value_set: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct OperationDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: Code,
    pub kind: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(rename = "affectsState", skip_serializing_if = "Option::is_none")]
    pub affects_state: Option<Boolean>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<Canonical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Vec<Code>>,
    pub system: Boolean,
    #[serde(rename = "type")]
    pub r#type: Boolean,
    pub instance: Boolean,
    #[serde(rename = "inputProfile", skip_serializing_if = "Option::is_none")]
    pub input_profile: Option<Canonical>,
    #[serde(rename = "outputProfile", skip_serializing_if = "Option::is_none")]
    pub output_profile: Option<Canonical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<OperationDefinitionParameter>>,
    #[serde(rename = "use")]
    pub r#use: Code,
    pub min: Integer,
    pub max: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(rename = "targetProfile", skip_serializing_if = "Option::is_none")]
    pub target_profile: Option<Vec<Canonical>>,
    #[serde(rename = "searchType", skip_serializing_if = "Option::is_none")]
    pub search_type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<OperationDefinitionParameterBinding>,
    pub strength: Code,
    #[serde(rename = "valueSet")]
    pub value_set: Canonical,
    #[serde(rename = "referencedFrom", skip_serializing_if = "Option::is_none")]
    pub referenced_from: Option<Vec<OperationDefinitionParameterReferencedFrom>>,
    pub source: String,
    #[serde(rename = "sourceId", skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<Vec<OperationDefinitionParameter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overload: Option<Vec<OperationDefinitionOverload>>,
    #[serde(rename = "parameterName", skip_serializing_if = "Option::is_none")]
    pub parameter_name: Option<Vec<String>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct OperationOutcome {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue: Option<Vec<OperationOutcomeIssue>>,
    pub severity: Code,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<Vec<String>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Organization {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<Boolean>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<Vec<Address>>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<OrganizationContact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct OrganizationAffiliation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<Reference>,
    #[serde(rename = "participatingOrganization", skip_serializing_if = "Option::is_none")]
    pub participating_organization: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specialty: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Vec<Reference>>,
    #[serde(rename = "healthcareService", skip_serializing_if = "Option::is_none")]
    pub healthcare_service: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<Reference>>,
}


/// Choice of types for the value[x] field in Parameters
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ParametersValueChoice {
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "valueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[serde(rename = "valueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[serde(rename = "valueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[serde(rename = "valueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[serde(rename = "valueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "valuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "valueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[serde(rename = "valueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[serde(rename = "valueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[serde(rename = "valueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[serde(rename = "valueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[serde(rename = "valueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[serde(rename = "valueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[serde(rename = "valueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[serde(rename = "valueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[serde(rename = "valueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[serde(rename = "valueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[serde(rename = "valueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[serde(rename = "valueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[serde(rename = "valueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[serde(rename = "valueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[serde(rename = "valueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[serde(rename = "valueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "valueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[serde(rename = "valueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[serde(rename = "valueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[serde(rename = "valueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "valueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[serde(rename = "valueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[serde(rename = "valueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[serde(rename = "valueMeta")]
    Meta(Meta),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Parameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<ParametersParameter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(flatten)]
    pub value: Option<ParametersValueChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Box<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<Vec<ParametersParameter>>,
}


/// Choice of types for the deceased[x] field in Patient
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum PatientDeceasedChoice {
    /// Variant accepting the Boolean type.
    #[serde(rename = "deceasedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the DateTime type.
    #[serde(rename = "deceasedDateTime")]
    DateTime(DateTime),
}

/// Choice of types for the multipleBirth[x] field in Patient
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum PatientMultipleBirthChoice {
    /// Variant accepting the Boolean type.
    #[serde(rename = "multipleBirthBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[serde(rename = "multipleBirthInteger")]
    Integer(Integer),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Patient {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Vec<HumanName>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<Code>,
    #[serde(rename = "birthDate", skip_serializing_if = "Option::is_none")]
    pub birth_date: Option<Date>,
    #[serde(flatten)]
    pub deceased: Option<PatientDeceasedChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<Vec<Address>>,
    #[serde(rename = "maritalStatus", skip_serializing_if = "Option::is_none")]
    pub marital_status: Option<CodeableConcept>,
    #[serde(flatten)]
    pub multiple_birth: Option<PatientMultipleBirthChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo: Option<Vec<Attachment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<PatientContact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub communication: Option<Vec<PatientCommunication>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred: Option<Boolean>,
    #[serde(rename = "generalPractitioner", skip_serializing_if = "Option::is_none")]
    pub general_practitioner: Option<Vec<Reference>>,
    #[serde(rename = "managingOrganization", skip_serializing_if = "Option::is_none")]
    pub managing_organization: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<Vec<PatientLink>>,
    pub other: Reference,
    #[serde(rename = "type")]
    pub r#type: Code,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct PaymentNotice {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<Reference>,
    pub created: DateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<Reference>,
    pub payment: Reference,
    #[serde(rename = "paymentDate", skip_serializing_if = "Option::is_none")]
    pub payment_date: Option<Date>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payee: Option<Reference>,
    pub recipient: Reference,
    pub amount: Money,
    #[serde(rename = "paymentStatus", skip_serializing_if = "Option::is_none")]
    pub payment_status: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct PaymentReconciliation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    pub created: DateTime,
    #[serde(rename = "paymentIssuer", skip_serializing_if = "Option::is_none")]
    pub payment_issuer: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requestor: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disposition: Option<String>,
    #[serde(rename = "paymentDate")]
    pub payment_date: Date,
    #[serde(rename = "paymentAmount")]
    pub payment_amount: Money,
    #[serde(rename = "paymentIdentifier", skip_serializing_if = "Option::is_none")]
    pub payment_identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<PaymentReconciliationDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predecessor: Option<Identifier>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submitter: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<Date>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsible: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payee: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Money>,
    #[serde(rename = "formCode", skip_serializing_if = "Option::is_none")]
    pub form_code: Option<CodeableConcept>,
    #[serde(rename = "processNote", skip_serializing_if = "Option::is_none")]
    pub process_note: Option<Vec<PaymentReconciliationProcessNote>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Person {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Vec<HumanName>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<Code>,
    #[serde(rename = "birthDate", skip_serializing_if = "Option::is_none")]
    pub birth_date: Option<Date>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<Vec<Address>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo: Option<Attachment>,
    #[serde(rename = "managingOrganization", skip_serializing_if = "Option::is_none")]
    pub managing_organization: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<Vec<PersonLink>>,
    pub target: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assurance: Option<Code>,
}


/// Choice of types for the subject[x] field in PlanDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionSubjectChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

/// Choice of types for the detail[x] field in PlanDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionDetailChoice {
    /// Variant accepting the Quantity type.
    #[serde(rename = "detailQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "detailRange")]
    Range(Range),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "detailCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

/// Choice of types for the offset[x] field in PlanDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionOffsetChoice {
    /// Variant accepting the Duration type.
    #[serde(rename = "offsetDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[serde(rename = "offsetRange")]
    Range(Range),
}

/// Choice of types for the timing[x] field in PlanDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionTimingChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "timingDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Age type.
    #[serde(rename = "timingAge")]
    Age(Age),
    /// Variant accepting the Period type.
    #[serde(rename = "timingPeriod")]
    Period(Period),
    /// Variant accepting the Duration type.
    #[serde(rename = "timingDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[serde(rename = "timingRange")]
    Range(Range),
    /// Variant accepting the Timing type.
    #[serde(rename = "timingTiming")]
    Timing(Timing),
}

/// Choice of types for the definition[x] field in PlanDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionDefinitionChoice {
    /// Variant accepting the Canonical type.
    #[serde(rename = "definitionCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Uri type.
    #[serde(rename = "definitionUri")]
    Uri(Uri),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct PlanDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(flatten)]
    pub subject: Option<PlanDefinitionSubjectChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate", skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate", skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod", skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact", skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<Vec<PlanDefinitionGoal>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addresses: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Vec<RelatedArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<PlanDefinitionGoalTarget>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measure: Option<CodeableConcept>,
    #[serde(flatten)]
    pub detail: Option<PlanDefinitionDetailChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<Duration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<PlanDefinitionAction>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(rename = "textEquivalent", skip_serializing_if = "Option::is_none")]
    pub text_equivalent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Vec<CodeableConcept>>,
    #[serde(rename = "goalId", skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<Vec<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger: Option<Vec<TriggerDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Vec<PlanDefinitionActionCondition>>,
    pub kind: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<Expression>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<DataRequirement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Vec<DataRequirement>>,
    #[serde(rename = "relatedAction", skip_serializing_if = "Option::is_none")]
    pub related_action: Option<Vec<PlanDefinitionActionRelatedAction>>,
    #[serde(rename = "actionId")]
    pub action_id: Id,
    pub relationship: Code,
    #[serde(flatten)]
    pub offset: Option<PlanDefinitionOffsetChoice>,
    #[serde(flatten)]
    pub timing: Option<PlanDefinitionTimingChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant: Option<Vec<PlanDefinitionActionParticipant>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
    #[serde(rename = "groupingBehavior", skip_serializing_if = "Option::is_none")]
    pub grouping_behavior: Option<Code>,
    #[serde(rename = "selectionBehavior", skip_serializing_if = "Option::is_none")]
    pub selection_behavior: Option<Code>,
    #[serde(rename = "requiredBehavior", skip_serializing_if = "Option::is_none")]
    pub required_behavior: Option<Code>,
    #[serde(rename = "precheckBehavior", skip_serializing_if = "Option::is_none")]
    pub precheck_behavior: Option<Code>,
    #[serde(rename = "cardinalityBehavior", skip_serializing_if = "Option::is_none")]
    pub cardinality_behavior: Option<Code>,
    #[serde(flatten)]
    pub definition: Option<PlanDefinitionDefinitionChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<Canonical>,
    #[serde(rename = "dynamicValue", skip_serializing_if = "Option::is_none")]
    pub dynamic_value: Option<Vec<PlanDefinitionActionDynamicValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Practitioner {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Vec<HumanName>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<Vec<Address>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<Code>,
    #[serde(rename = "birthDate", skip_serializing_if = "Option::is_none")]
    pub birth_date: Option<Date>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo: Option<Vec<Attachment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualification: Option<Vec<PractitionerQualification>>,
    pub code: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub communication: Option<Vec<CodeableConcept>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct PractitionerRole {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub practitioner: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specialty: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Vec<Reference>>,
    #[serde(rename = "healthcareService", skip_serializing_if = "Option::is_none")]
    pub healthcare_service: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(rename = "availableTime", skip_serializing_if = "Option::is_none")]
    pub available_time: Option<Vec<PractitionerRoleAvailableTime>>,
    #[serde(rename = "daysOfWeek", skip_serializing_if = "Option::is_none")]
    pub days_of_week: Option<Vec<Code>>,
    #[serde(rename = "allDay", skip_serializing_if = "Option::is_none")]
    pub all_day: Option<Boolean>,
    #[serde(rename = "availableStartTime", skip_serializing_if = "Option::is_none")]
    pub available_start_time: Option<Time>,
    #[serde(rename = "availableEndTime", skip_serializing_if = "Option::is_none")]
    pub available_end_time: Option<Time>,
    #[serde(rename = "notAvailable", skip_serializing_if = "Option::is_none")]
    pub not_available: Option<Vec<PractitionerRoleNotAvailable>>,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub during: Option<Period>,
    #[serde(rename = "availabilityExceptions", skip_serializing_if = "Option::is_none")]
    pub availability_exceptions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<Reference>>,
}


/// Choice of types for the performed[x] field in Procedure
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ProcedurePerformedChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "performedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "performedPeriod")]
    Period(Period),
    /// Variant accepting the String type.
    #[serde(rename = "performedString")]
    String(String),
    /// Variant accepting the Age type.
    #[serde(rename = "performedAge")]
    Age(Age),
    /// Variant accepting the Range type.
    #[serde(rename = "performedRange")]
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Procedure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical", skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri", skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReason", skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(flatten)]
    pub performed: Option<ProcedurePerformedChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorder: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asserter: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<ProcedurePerformer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
    #[serde(rename = "onBehalfOf", skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "bodySite", skip_serializing_if = "Option::is_none")]
    pub body_site: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complication: Option<Vec<CodeableConcept>>,
    #[serde(rename = "complicationDetail", skip_serializing_if = "Option::is_none")]
    pub complication_detail: Option<Vec<Reference>>,
    #[serde(rename = "followUp", skip_serializing_if = "Option::is_none")]
    pub follow_up: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "focalDevice", skip_serializing_if = "Option::is_none")]
    pub focal_device: Option<Vec<ProcedureFocalDevice>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<CodeableConcept>,
    pub manipulated: Reference,
    #[serde(rename = "usedReference", skip_serializing_if = "Option::is_none")]
    pub used_reference: Option<Vec<Reference>>,
    #[serde(rename = "usedCode", skip_serializing_if = "Option::is_none")]
    pub used_code: Option<Vec<CodeableConcept>>,
}


/// Choice of types for the occurred[x] field in Provenance
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ProvenanceOccurredChoice {
    /// Variant accepting the Period type.
    #[serde(rename = "occurredPeriod")]
    Period(Period),
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurredDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Provenance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<Reference>>,
    #[serde(flatten)]
    pub occurred: Option<ProvenanceOccurredChoice>,
    pub recorded: Instant,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<Vec<Uri>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<Vec<ProvenanceAgent>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Vec<CodeableConcept>>,
    pub who: Reference,
    #[serde(rename = "onBehalfOf", skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<Vec<ProvenanceEntity>>,
    pub what: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<Signature>>,
}


/// Choice of types for the answer[x] field in Questionnaire
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireAnswerChoice {
    /// Variant accepting the Boolean type.
    #[serde(rename = "answerBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Decimal type.
    #[serde(rename = "answerDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Integer type.
    #[serde(rename = "answerInteger")]
    Integer(Integer),
    /// Variant accepting the Date type.
    #[serde(rename = "answerDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "answerDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Time type.
    #[serde(rename = "answerTime")]
    Time(Time),
    /// Variant accepting the String type.
    #[serde(rename = "answerString")]
    String(String),
    /// Variant accepting the Coding type.
    #[serde(rename = "answerCoding")]
    Coding(Coding),
    /// Variant accepting the Quantity type.
    #[serde(rename = "answerQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Reference type.
    #[serde(rename = "answerReference")]
    Reference(Reference),
}

/// Choice of types for the value[x] field in Questionnaire
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireValueChoice {
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Date type.
    #[serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Questionnaire {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(rename = "derivedFrom", skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<Vec<Canonical>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(rename = "subjectType", skip_serializing_if = "Option::is_none")]
    pub subject_type: Option<Vec<Code>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate", skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate", skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod", skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<Coding>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<QuestionnaireItem>>,
    #[serde(rename = "linkId")]
    pub link_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(rename = "enableWhen", skip_serializing_if = "Option::is_none")]
    pub enable_when: Option<Vec<QuestionnaireItemEnableWhen>>,
    pub question: String,
    pub operator: Code,
    #[serde(flatten)]
    pub answer: Option<QuestionnaireAnswerChoice>,
    #[serde(rename = "enableBehavior", skip_serializing_if = "Option::is_none")]
    pub enable_behavior: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeats: Option<Boolean>,
    #[serde(rename = "readOnly", skip_serializing_if = "Option::is_none")]
    pub read_only: Option<Boolean>,
    #[serde(rename = "maxLength", skip_serializing_if = "Option::is_none")]
    pub max_length: Option<Integer>,
    #[serde(rename = "answerValueSet", skip_serializing_if = "Option::is_none")]
    pub answer_value_set: Option<Canonical>,
    #[serde(rename = "answerOption", skip_serializing_if = "Option::is_none")]
    pub answer_option: Option<Vec<QuestionnaireItemAnswerOption>>,
    #[serde(flatten)]
    pub value: Option<QuestionnaireValueChoice>,
    #[serde(rename = "initialSelected", skip_serializing_if = "Option::is_none")]
    pub initial_selected: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial: Option<Vec<QuestionnaireItemInitial>>,
}


/// Choice of types for the value[x] field in QuestionnaireResponse
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireResponseValueChoice {
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Date type.
    #[serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Uri type.
    #[serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Attachment type.
    #[serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct QuestionnaireResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub questionnaire: Option<Canonical>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authored: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<QuestionnaireResponseItem>>,
    #[serde(rename = "linkId")]
    pub link_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<Vec<QuestionnaireResponseItemAnswer>>,
    #[serde(flatten)]
    pub value: Option<QuestionnaireResponseValueChoice>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RelatedPerson {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<Boolean>,
    pub patient: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Vec<HumanName>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<Code>,
    #[serde(rename = "birthDate", skip_serializing_if = "Option::is_none")]
    pub birth_date: Option<Date>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<Vec<Address>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo: Option<Vec<Attachment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub communication: Option<Vec<RelatedPersonCommunication>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred: Option<Boolean>,
}


/// Choice of types for the offset[x] field in RequestGroup
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum RequestGroupOffsetChoice {
    /// Variant accepting the Duration type.
    #[serde(rename = "offsetDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[serde(rename = "offsetRange")]
    Range(Range),
}

/// Choice of types for the timing[x] field in RequestGroup
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum RequestGroupTimingChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "timingDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Age type.
    #[serde(rename = "timingAge")]
    Age(Age),
    /// Variant accepting the Period type.
    #[serde(rename = "timingPeriod")]
    Period(Period),
    /// Variant accepting the Duration type.
    #[serde(rename = "timingDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[serde(rename = "timingRange")]
    Range(Range),
    /// Variant accepting the Timing type.
    #[serde(rename = "timingTiming")]
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RequestGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical", skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri", skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaces: Option<Vec<Reference>>,
    #[serde(rename = "groupIdentifier", skip_serializing_if = "Option::is_none")]
    pub group_identifier: Option<Identifier>,
    pub status: Code,
    pub intent: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(rename = "authoredOn", skip_serializing_if = "Option::is_none")]
    pub authored_on: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Reference>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<RequestGroupAction>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "textEquivalent", skip_serializing_if = "Option::is_none")]
    pub text_equivalent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Vec<RelatedArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Vec<RequestGroupActionCondition>>,
    pub kind: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<Expression>,
    #[serde(rename = "relatedAction", skip_serializing_if = "Option::is_none")]
    pub related_action: Option<Vec<RequestGroupActionRelatedAction>>,
    #[serde(rename = "actionId")]
    pub action_id: Id,
    pub relationship: Code,
    #[serde(flatten)]
    pub offset: Option<RequestGroupOffsetChoice>,
    #[serde(flatten)]
    pub timing: Option<RequestGroupTimingChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant: Option<Vec<Reference>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "groupingBehavior", skip_serializing_if = "Option::is_none")]
    pub grouping_behavior: Option<Code>,
    #[serde(rename = "selectionBehavior", skip_serializing_if = "Option::is_none")]
    pub selection_behavior: Option<Code>,
    #[serde(rename = "requiredBehavior", skip_serializing_if = "Option::is_none")]
    pub required_behavior: Option<Code>,
    #[serde(rename = "precheckBehavior", skip_serializing_if = "Option::is_none")]
    pub precheck_behavior: Option<Code>,
    #[serde(rename = "cardinalityBehavior", skip_serializing_if = "Option::is_none")]
    pub cardinality_behavior: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Reference>,
}


/// Choice of types for the subject[x] field in ResearchDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ResearchDefinitionSubjectChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ResearchDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(rename = "shortTitle", skip_serializing_if = "Option::is_none")]
    pub short_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(flatten)]
    pub subject: Option<ResearchDefinitionSubjectChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<Vec<String>>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate", skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate", skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod", skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact", skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library: Option<Vec<Canonical>>,
    pub population: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exposure: Option<Reference>,
    #[serde(rename = "exposureAlternative", skip_serializing_if = "Option::is_none")]
    pub exposure_alternative: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Reference>,
}


/// Choice of types for the subject[x] field in ResearchElementDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionSubjectChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

/// Choice of types for the definition[x] field in ResearchElementDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionDefinitionChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "definitionCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Canonical type.
    #[serde(rename = "definitionCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Expression type.
    #[serde(rename = "definitionExpression")]
    Expression(Expression),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "definitionDataRequirement")]
    DataRequirement(DataRequirement),
}

/// Choice of types for the studyEffective[x] field in ResearchElementDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionStudyEffectiveChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "studyEffectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "studyEffectivePeriod")]
    Period(Period),
    /// Variant accepting the Duration type.
    #[serde(rename = "studyEffectiveDuration")]
    Duration(Duration),
    /// Variant accepting the Timing type.
    #[serde(rename = "studyEffectiveTiming")]
    Timing(Timing),
}

/// Choice of types for the participantEffective[x] field in ResearchElementDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionParticipantEffectiveChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "participantEffectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "participantEffectivePeriod")]
    Period(Period),
    /// Variant accepting the Duration type.
    #[serde(rename = "participantEffectiveDuration")]
    Duration(Duration),
    /// Variant accepting the Timing type.
    #[serde(rename = "participantEffectiveTiming")]
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ResearchElementDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(rename = "shortTitle", skip_serializing_if = "Option::is_none")]
    pub short_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(flatten)]
    pub subject: Option<ResearchElementDefinitionSubjectChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<Vec<String>>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate", skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate", skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod", skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact", skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library: Option<Vec<Canonical>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(rename = "variableType", skip_serializing_if = "Option::is_none")]
    pub variable_type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub characteristic: Option<Vec<ResearchElementDefinitionCharacteristic>>,
    #[serde(flatten)]
    pub definition: Option<ResearchElementDefinitionDefinitionChoice>,
    #[serde(rename = "usageContext", skip_serializing_if = "Option::is_none")]
    pub usage_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Boolean>,
    #[serde(rename = "unitOfMeasure", skip_serializing_if = "Option::is_none")]
    pub unit_of_measure: Option<CodeableConcept>,
    #[serde(rename = "studyEffectiveDescription", skip_serializing_if = "Option::is_none")]
    pub study_effective_description: Option<String>,
    #[serde(flatten)]
    pub study_effective: Option<ResearchElementDefinitionStudyEffectiveChoice>,
    #[serde(rename = "studyEffectiveTimeFromStart", skip_serializing_if = "Option::is_none")]
    pub study_effective_time_from_start: Option<Duration>,
    #[serde(rename = "studyEffectiveGroupMeasure", skip_serializing_if = "Option::is_none")]
    pub study_effective_group_measure: Option<Code>,
    #[serde(rename = "participantEffectiveDescription", skip_serializing_if = "Option::is_none")]
    pub participant_effective_description: Option<String>,
    #[serde(flatten)]
    pub participant_effective: Option<ResearchElementDefinitionParticipantEffectiveChoice>,
    #[serde(rename = "participantEffectiveTimeFromStart", skip_serializing_if = "Option::is_none")]
    pub participant_effective_time_from_start: Option<Duration>,
    #[serde(rename = "participantEffectiveGroupMeasure", skip_serializing_if = "Option::is_none")]
    pub participant_effective_group_measure: Option<Code>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ResearchStudy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<Vec<Reference>>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "primaryPurposeType", skip_serializing_if = "Option::is_none")]
    pub primary_purpose_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact", skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyword: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enrollment: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sponsor: Option<Reference>,
    #[serde(rename = "principalInvestigator", skip_serializing_if = "Option::is_none")]
    pub principal_investigator: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<Vec<Reference>>,
    #[serde(rename = "reasonStopped", skip_serializing_if = "Option::is_none")]
    pub reason_stopped: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arm: Option<Vec<ResearchStudyArm>>,
    pub name: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective: Option<Vec<ResearchStudyObjective>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ResearchSubject {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    pub study: Reference,
    pub individual: Reference,
    #[serde(rename = "assignedArm", skip_serializing_if = "Option::is_none")]
    pub assigned_arm: Option<String>,
    #[serde(rename = "actualArm", skip_serializing_if = "Option::is_none")]
    pub actual_arm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consent: Option<Reference>,
}


/// Choice of types for the occurrence[x] field in RiskAssessment
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum RiskAssessmentOccurrenceChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "occurrencePeriod")]
    Period(Period),
}

/// Choice of types for the probability[x] field in RiskAssessment
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum RiskAssessmentProbabilityChoice {
    /// Variant accepting the Decimal type.
    #[serde(rename = "probabilityDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Range type.
    #[serde(rename = "probabilityRange")]
    Range(Range),
}

/// Choice of types for the when[x] field in RiskAssessment
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum RiskAssessmentWhenChoice {
    /// Variant accepting the Period type.
    #[serde(rename = "whenPeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[serde(rename = "whenRange")]
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RiskAssessment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<Reference>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(flatten)]
    pub occurrence: Option<RiskAssessmentOccurrenceChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Reference>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub basis: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prediction: Option<Vec<RiskAssessmentPrediction>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<CodeableConcept>,
    #[serde(flatten)]
    pub probability: Option<RiskAssessmentProbabilityChoice>,
    #[serde(rename = "qualitativeRisk", skip_serializing_if = "Option::is_none")]
    pub qualitative_risk: Option<CodeableConcept>,
    #[serde(rename = "relativeRisk", skip_serializing_if = "Option::is_none")]
    pub relative_risk: Option<Decimal>,
    #[serde(flatten)]
    pub when: Option<RiskAssessmentWhenChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mitigation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RiskEvidenceSynthesis {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate", skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate", skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod", skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact", skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(rename = "synthesisType", skip_serializing_if = "Option::is_none")]
    pub synthesis_type: Option<CodeableConcept>,
    #[serde(rename = "studyType", skip_serializing_if = "Option::is_none")]
    pub study_type: Option<CodeableConcept>,
    pub population: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exposure: Option<Reference>,
    pub outcome: Reference,
    #[serde(rename = "sampleSize", skip_serializing_if = "Option::is_none")]
    pub sample_size: Option<RiskEvidenceSynthesisSampleSize>,
    #[serde(rename = "numberOfStudies", skip_serializing_if = "Option::is_none")]
    pub number_of_studies: Option<Integer>,
    #[serde(rename = "numberOfParticipants", skip_serializing_if = "Option::is_none")]
    pub number_of_participants: Option<Integer>,
    #[serde(rename = "riskEstimate", skip_serializing_if = "Option::is_none")]
    pub risk_estimate: Option<RiskEvidenceSynthesisRiskEstimate>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Decimal>,
    #[serde(rename = "unitOfMeasure", skip_serializing_if = "Option::is_none")]
    pub unit_of_measure: Option<CodeableConcept>,
    #[serde(rename = "denominatorCount", skip_serializing_if = "Option::is_none")]
    pub denominator_count: Option<Integer>,
    #[serde(rename = "numeratorCount", skip_serializing_if = "Option::is_none")]
    pub numerator_count: Option<Integer>,
    #[serde(rename = "precisionEstimate", skip_serializing_if = "Option::is_none")]
    pub precision_estimate: Option<Vec<RiskEvidenceSynthesisRiskEstimatePrecisionEstimate>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certainty: Option<Vec<RiskEvidenceSynthesisCertainty>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<Vec<CodeableConcept>>,
    #[serde(rename = "certaintySubcomponent", skip_serializing_if = "Option::is_none")]
    pub certainty_subcomponent: Option<Vec<RiskEvidenceSynthesisCertaintyCertaintySubcomponent>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Schedule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<Boolean>,
    #[serde(rename = "serviceCategory", skip_serializing_if = "Option::is_none")]
    pub service_category: Option<Vec<CodeableConcept>>,
    #[serde(rename = "serviceType", skip_serializing_if = "Option::is_none")]
    pub service_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specialty: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Vec<Reference>>,
    #[serde(rename = "planningHorizon", skip_serializing_if = "Option::is_none")]
    pub planning_horizon: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct SearchParameter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub name: String,
    #[serde(rename = "derivedFrom", skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<Canonical>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Markdown,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<Vec<Code>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xpath: Option<String>,
    #[serde(rename = "xpathUsage", skip_serializing_if = "Option::is_none")]
    pub xpath_usage: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<Code>>,
    #[serde(rename = "multipleOr", skip_serializing_if = "Option::is_none")]
    pub multiple_or: Option<Boolean>,
    #[serde(rename = "multipleAnd", skip_serializing_if = "Option::is_none")]
    pub multiple_and: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparator: Option<Vec<Code>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<Code>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<Vec<SearchParameterComponent>>,
    pub definition: Canonical,
}


/// Choice of types for the quantity[x] field in ServiceRequest
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ServiceRequestQuantityChoice {
    /// Variant accepting the Quantity type.
    #[serde(rename = "quantityQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Ratio type.
    #[serde(rename = "quantityRatio")]
    Ratio(Ratio),
    /// Variant accepting the Range type.
    #[serde(rename = "quantityRange")]
    Range(Range),
}

/// Choice of types for the occurrence[x] field in ServiceRequest
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ServiceRequestOccurrenceChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

/// Choice of types for the asNeeded[x] field in ServiceRequest
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ServiceRequestAsNeededChoice {
    /// Variant accepting the Boolean type.
    #[serde(rename = "asNeededBoolean")]
    Boolean(Boolean),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "asNeededCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ServiceRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical", skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri", skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaces: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requisition: Option<Identifier>,
    pub status: Code,
    pub intent: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Code>,
    #[serde(rename = "doNotPerform", skip_serializing_if = "Option::is_none")]
    pub do_not_perform: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(rename = "orderDetail", skip_serializing_if = "Option::is_none")]
    pub order_detail: Option<Vec<CodeableConcept>>,
    #[serde(flatten)]
    pub quantity: Option<ServiceRequestQuantityChoice>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(flatten)]
    pub occurrence: Option<ServiceRequestOccurrenceChoice>,
    #[serde(flatten)]
    pub as_needed: Option<ServiceRequestAsNeededChoice>,
    #[serde(rename = "authoredOn", skip_serializing_if = "Option::is_none")]
    pub authored_on: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester: Option<Reference>,
    #[serde(rename = "performerType", skip_serializing_if = "Option::is_none")]
    pub performer_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<Reference>>,
    #[serde(rename = "locationCode", skip_serializing_if = "Option::is_none")]
    pub location_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "locationReference", skip_serializing_if = "Option::is_none")]
    pub location_reference: Option<Vec<Reference>>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurance: Option<Vec<Reference>>,
    #[serde(rename = "supportingInfo", skip_serializing_if = "Option::is_none")]
    pub supporting_info: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specimen: Option<Vec<Reference>>,
    #[serde(rename = "bodySite", skip_serializing_if = "Option::is_none")]
    pub body_site: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "patientInstruction", skip_serializing_if = "Option::is_none")]
    pub patient_instruction: Option<String>,
    #[serde(rename = "relevantHistory", skip_serializing_if = "Option::is_none")]
    pub relevant_history: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Slot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "serviceCategory", skip_serializing_if = "Option::is_none")]
    pub service_category: Option<Vec<CodeableConcept>>,
    #[serde(rename = "serviceType", skip_serializing_if = "Option::is_none")]
    pub service_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specialty: Option<Vec<CodeableConcept>>,
    #[serde(rename = "appointmentType", skip_serializing_if = "Option::is_none")]
    pub appointment_type: Option<CodeableConcept>,
    pub schedule: Reference,
    pub status: Code,
    pub start: Instant,
    pub end: Instant,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overbooked: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}


/// Choice of types for the collected[x] field in Specimen
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenCollectedChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "collectedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "collectedPeriod")]
    Period(Period),
}

/// Choice of types for the fastingStatus[x] field in Specimen
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenFastingStatusChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "fastingStatusCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Duration type.
    #[serde(rename = "fastingStatusDuration")]
    Duration(Duration),
}

/// Choice of types for the time[x] field in Specimen
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenTimeChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "timeDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "timePeriod")]
    Period(Period),
}

/// Choice of types for the additive[x] field in Specimen
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenAdditiveChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "additiveCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "additiveReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Specimen {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "accessionIdentifier", skip_serializing_if = "Option::is_none")]
    pub accession_identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(rename = "receivedTime", skip_serializing_if = "Option::is_none")]
    pub received_time: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<SpecimenCollection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collector: Option<Reference>,
    #[serde(flatten)]
    pub collected: Option<SpecimenCollectedChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<Duration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(rename = "bodySite", skip_serializing_if = "Option::is_none")]
    pub body_site: Option<CodeableConcept>,
    #[serde(flatten)]
    pub fasting_status: Option<SpecimenFastingStatusChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processing: Option<Vec<SpecimenProcessing>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additive: Option<Vec<Reference>>,
    #[serde(flatten)]
    pub time: Option<SpecimenTimeChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<Vec<SpecimenContainer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity: Option<Quantity>,
    #[serde(rename = "specimenQuantity", skip_serializing_if = "Option::is_none")]
    pub specimen_quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}


/// Choice of types for the minimumVolume[x] field in SpecimenDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenDefinitionMinimumVolumeChoice {
    /// Variant accepting the Quantity type.
    #[serde(rename = "minimumVolumeQuantity")]
    Quantity(Quantity),
    /// Variant accepting the String type.
    #[serde(rename = "minimumVolumeString")]
    String(String),
}

/// Choice of types for the additive[x] field in SpecimenDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenDefinitionAdditiveChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "additiveCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "additiveReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct SpecimenDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(rename = "typeCollected", skip_serializing_if = "Option::is_none")]
    pub type_collected: Option<CodeableConcept>,
    #[serde(rename = "patientPreparation", skip_serializing_if = "Option::is_none")]
    pub patient_preparation: Option<Vec<CodeableConcept>>,
    #[serde(rename = "timeAspect", skip_serializing_if = "Option::is_none")]
    pub time_aspect: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<Vec<CodeableConcept>>,
    #[serde(rename = "typeTested", skip_serializing_if = "Option::is_none")]
    pub type_tested: Option<Vec<SpecimenDefinitionTypeTested>>,
    #[serde(rename = "isDerived", skip_serializing_if = "Option::is_none")]
    pub is_derived: Option<Boolean>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    pub preference: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<SpecimenDefinitionTypeTestedContainer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity: Option<Quantity>,
    #[serde(flatten)]
    pub minimum_volume: Option<SpecimenDefinitionMinimumVolumeChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additive: Option<Vec<SpecimenDefinitionTypeTestedContainerAdditive>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preparation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
    #[serde(rename = "retentionTime", skip_serializing_if = "Option::is_none")]
    pub retention_time: Option<Duration>,
    #[serde(rename = "rejectionCriterion", skip_serializing_if = "Option::is_none")]
    pub rejection_criterion: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handling: Option<Vec<SpecimenDefinitionTypeTestedHandling>>,
    #[serde(rename = "temperatureQualifier", skip_serializing_if = "Option::is_none")]
    pub temperature_qualifier: Option<CodeableConcept>,
    #[serde(rename = "temperatureRange", skip_serializing_if = "Option::is_none")]
    pub temperature_range: Option<Range>,
    #[serde(rename = "maxDuration", skip_serializing_if = "Option::is_none")]
    pub max_duration: Option<Duration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct StructureDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyword: Option<Vec<Coding>>,
    #[serde(rename = "fhirVersion", skip_serializing_if = "Option::is_none")]
    pub fhir_version: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mapping: Option<Vec<StructureDefinitionMapping>>,
    pub identity: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub kind: Code,
    #[serde(rename = "abstract")]
    pub r#abstract: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Vec<StructureDefinitionContext>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub expression: String,
    #[serde(rename = "contextInvariant", skip_serializing_if = "Option::is_none")]
    pub context_invariant: Option<Vec<String>>,
    #[serde(rename = "baseDefinition", skip_serializing_if = "Option::is_none")]
    pub base_definition: Option<Canonical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derivation: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<StructureDefinitionSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<Vec<ElementDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub differential: Option<StructureDefinitionDifferential>,
}


/// Choice of types for the defaultValue[x] field in StructureMap
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum StructureMapDefaultValueChoice {
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "defaultValueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[serde(rename = "defaultValueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "defaultValueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[serde(rename = "defaultValueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[serde(rename = "defaultValueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "defaultValueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "defaultValueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[serde(rename = "defaultValueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[serde(rename = "defaultValueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[serde(rename = "defaultValueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[serde(rename = "defaultValueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[serde(rename = "defaultValueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "defaultValuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "defaultValueString")]
    String(String),
    /// Variant accepting the Time type.
    #[serde(rename = "defaultValueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "defaultValueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[serde(rename = "defaultValueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[serde(rename = "defaultValueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[serde(rename = "defaultValueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[serde(rename = "defaultValueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[serde(rename = "defaultValueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[serde(rename = "defaultValueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[serde(rename = "defaultValueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "defaultValueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[serde(rename = "defaultValueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[serde(rename = "defaultValueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[serde(rename = "defaultValueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[serde(rename = "defaultValueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[serde(rename = "defaultValueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[serde(rename = "defaultValueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[serde(rename = "defaultValueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[serde(rename = "defaultValueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[serde(rename = "defaultValuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[serde(rename = "defaultValueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "defaultValueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "defaultValueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[serde(rename = "defaultValueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[serde(rename = "defaultValueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[serde(rename = "defaultValueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[serde(rename = "defaultValueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[serde(rename = "defaultValueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[serde(rename = "defaultValueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "defaultValueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[serde(rename = "defaultValueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[serde(rename = "defaultValueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[serde(rename = "defaultValueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "defaultValueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[serde(rename = "defaultValueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[serde(rename = "defaultValueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[serde(rename = "defaultValueMeta")]
    Meta(Meta),
}

/// Choice of types for the value[x] field in StructureMap
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum StructureMapValueChoice {
    /// Variant accepting the Id type.
    #[serde(rename = "valueId")]
    Id(Id),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct StructureMap {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structure: Option<Vec<StructureMapStructure>>,
    pub mode: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Vec<StructureMapGroup>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends: Option<Id>,
    #[serde(rename = "typeMode")]
    pub type_mode: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<StructureMapGroupInput>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<Vec<StructureMapGroupRule>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<StructureMapGroupRuleSource>>,
    pub context: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
    #[serde(flatten)]
    pub default_value: Option<StructureMapDefaultValueChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    #[serde(rename = "listMode", skip_serializing_if = "Option::is_none")]
    pub list_mode: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    #[serde(rename = "logMessage", skip_serializing_if = "Option::is_none")]
    pub log_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<StructureMapGroupRuleTarget>>,
    #[serde(rename = "contextType", skip_serializing_if = "Option::is_none")]
    pub context_type: Option<Code>,
    #[serde(rename = "listRuleId", skip_serializing_if = "Option::is_none")]
    pub list_rule_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<StructureMapGroupRuleTargetParameter>>,
    #[serde(flatten)]
    pub value: Option<StructureMapValueChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependent: Option<Vec<StructureMapGroupRuleDependent>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Subscription {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<Instant>,
    pub reason: String,
    pub criteria: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub channel: SubscriptionChannel,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Url>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<Vec<String>>,
}


/// Choice of types for the substance[x] field in Substance
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSubstanceChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "substanceCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "substanceReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Substance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<Vec<SubstanceInstance>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingredient: Option<Vec<SubstanceIngredient>>,
    #[serde(flatten)]
    pub substance: Option<SubstanceSubstanceChoice>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct SubstanceNucleicAcid {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "sequenceType", skip_serializing_if = "Option::is_none")]
    pub sequence_type: Option<CodeableConcept>,
    #[serde(rename = "numberOfSubunits", skip_serializing_if = "Option::is_none")]
    pub number_of_subunits: Option<Integer>,
    #[serde(rename = "areaOfHybridisation", skip_serializing_if = "Option::is_none")]
    pub area_of_hybridisation: Option<String>,
    #[serde(rename = "oligoNucleotideType", skip_serializing_if = "Option::is_none")]
    pub oligo_nucleotide_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subunit: Option<Vec<SubstanceNucleicAcidSubunit>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<Integer>,
    #[serde(rename = "sequenceAttachment", skip_serializing_if = "Option::is_none")]
    pub sequence_attachment: Option<Attachment>,
    #[serde(rename = "fivePrime", skip_serializing_if = "Option::is_none")]
    pub five_prime: Option<CodeableConcept>,
    #[serde(rename = "threePrime", skip_serializing_if = "Option::is_none")]
    pub three_prime: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linkage: Option<Vec<SubstanceNucleicAcidSubunitLinkage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connectivity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "residueSite", skip_serializing_if = "Option::is_none")]
    pub residue_site: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sugar: Option<Vec<SubstanceNucleicAcidSubunitSugar>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct SubstancePolymer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geometry: Option<CodeableConcept>,
    #[serde(rename = "copolymerConnectivity", skip_serializing_if = "Option::is_none")]
    pub copolymer_connectivity: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modification: Option<Vec<String>>,
    #[serde(rename = "monomerSet", skip_serializing_if = "Option::is_none")]
    pub monomer_set: Option<Vec<SubstancePolymerMonomerSet>>,
    #[serde(rename = "ratioType", skip_serializing_if = "Option::is_none")]
    pub ratio_type: Option<CodeableConcept>,
    #[serde(rename = "startingMaterial", skip_serializing_if = "Option::is_none")]
    pub starting_material: Option<Vec<SubstancePolymerMonomerSetStartingMaterial>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<CodeableConcept>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "isDefining", skip_serializing_if = "Option::is_none")]
    pub is_defining: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<SubstanceAmount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat: Option<Vec<SubstancePolymerRepeat>>,
    #[serde(rename = "numberOfUnits", skip_serializing_if = "Option::is_none")]
    pub number_of_units: Option<Integer>,
    #[serde(rename = "averageMolecularFormula", skip_serializing_if = "Option::is_none")]
    pub average_molecular_formula: Option<String>,
    #[serde(rename = "repeatUnitAmountType", skip_serializing_if = "Option::is_none")]
    pub repeat_unit_amount_type: Option<CodeableConcept>,
    #[serde(rename = "repeatUnit", skip_serializing_if = "Option::is_none")]
    pub repeat_unit: Option<Vec<SubstancePolymerRepeatRepeatUnit>>,
    #[serde(rename = "orientationOfPolymerisation", skip_serializing_if = "Option::is_none")]
    pub orientation_of_polymerisation: Option<CodeableConcept>,
    #[serde(rename = "degreeOfPolymerisation", skip_serializing_if = "Option::is_none")]
    pub degree_of_polymerisation: Option<Vec<SubstancePolymerRepeatRepeatUnitDegreeOfPolymerisation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degree: Option<CodeableConcept>,
    #[serde(rename = "structuralRepresentation", skip_serializing_if = "Option::is_none")]
    pub structural_representation: Option<Vec<SubstancePolymerRepeatRepeatUnitStructuralRepresentation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub representation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<Attachment>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct SubstanceProtein {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "sequenceType", skip_serializing_if = "Option::is_none")]
    pub sequence_type: Option<CodeableConcept>,
    #[serde(rename = "numberOfSubunits", skip_serializing_if = "Option::is_none")]
    pub number_of_subunits: Option<Integer>,
    #[serde(rename = "disulfideLinkage", skip_serializing_if = "Option::is_none")]
    pub disulfide_linkage: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subunit: Option<Vec<SubstanceProteinSubunit>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<Integer>,
    #[serde(rename = "sequenceAttachment", skip_serializing_if = "Option::is_none")]
    pub sequence_attachment: Option<Attachment>,
    #[serde(rename = "nTerminalModificationId", skip_serializing_if = "Option::is_none")]
    pub n_terminal_modification_id: Option<Identifier>,
    #[serde(rename = "nTerminalModification", skip_serializing_if = "Option::is_none")]
    pub n_terminal_modification: Option<String>,
    #[serde(rename = "cTerminalModificationId", skip_serializing_if = "Option::is_none")]
    pub c_terminal_modification_id: Option<Identifier>,
    #[serde(rename = "cTerminalModification", skip_serializing_if = "Option::is_none")]
    pub c_terminal_modification: Option<String>,
}


/// Choice of types for the amount[x] field in SubstanceReferenceInformation
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceReferenceInformationAmountChoice {
    /// Variant accepting the Quantity type.
    #[serde(rename = "amountQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "amountRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[serde(rename = "amountString")]
    String(String),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct SubstanceReferenceInformation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gene: Option<Vec<SubstanceReferenceInformationGene>>,
    #[serde(rename = "geneSequenceOrigin", skip_serializing_if = "Option::is_none")]
    pub gene_sequence_origin: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<Reference>>,
    #[serde(rename = "geneElement", skip_serializing_if = "Option::is_none")]
    pub gene_element: Option<Vec<SubstanceReferenceInformationGeneElement>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<Vec<SubstanceReferenceInformationClassification>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtype: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<SubstanceReferenceInformationTarget>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organism: Option<CodeableConcept>,
    #[serde(rename = "organismType", skip_serializing_if = "Option::is_none")]
    pub organism_type: Option<CodeableConcept>,
    #[serde(flatten)]
    pub amount: Option<SubstanceReferenceInformationAmountChoice>,
    #[serde(rename = "amountType", skip_serializing_if = "Option::is_none")]
    pub amount_type: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct SubstanceSourceMaterial {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "sourceMaterialClass", skip_serializing_if = "Option::is_none")]
    pub source_material_class: Option<CodeableConcept>,
    #[serde(rename = "sourceMaterialType", skip_serializing_if = "Option::is_none")]
    pub source_material_type: Option<CodeableConcept>,
    #[serde(rename = "sourceMaterialState", skip_serializing_if = "Option::is_none")]
    pub source_material_state: Option<CodeableConcept>,
    #[serde(rename = "organismId", skip_serializing_if = "Option::is_none")]
    pub organism_id: Option<Identifier>,
    #[serde(rename = "organismName", skip_serializing_if = "Option::is_none")]
    pub organism_name: Option<String>,
    #[serde(rename = "parentSubstanceId", skip_serializing_if = "Option::is_none")]
    pub parent_substance_id: Option<Vec<Identifier>>,
    #[serde(rename = "parentSubstanceName", skip_serializing_if = "Option::is_none")]
    pub parent_substance_name: Option<Vec<String>>,
    #[serde(rename = "countryOfOrigin", skip_serializing_if = "Option::is_none")]
    pub country_of_origin: Option<Vec<CodeableConcept>>,
    #[serde(rename = "geographicalLocation", skip_serializing_if = "Option::is_none")]
    pub geographical_location: Option<Vec<String>>,
    #[serde(rename = "developmentStage", skip_serializing_if = "Option::is_none")]
    pub development_stage: Option<CodeableConcept>,
    #[serde(rename = "fractionDescription", skip_serializing_if = "Option::is_none")]
    pub fraction_description: Option<Vec<SubstanceSourceMaterialFractionDescription>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fraction: Option<String>,
    #[serde(rename = "materialType", skip_serializing_if = "Option::is_none")]
    pub material_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organism: Option<SubstanceSourceMaterialOrganism>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genus: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub species: Option<CodeableConcept>,
    #[serde(rename = "intraspecificType", skip_serializing_if = "Option::is_none")]
    pub intraspecific_type: Option<CodeableConcept>,
    #[serde(rename = "intraspecificDescription", skip_serializing_if = "Option::is_none")]
    pub intraspecific_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Vec<SubstanceSourceMaterialOrganismAuthor>>,
    #[serde(rename = "authorType", skip_serializing_if = "Option::is_none")]
    pub author_type: Option<CodeableConcept>,
    #[serde(rename = "authorDescription", skip_serializing_if = "Option::is_none")]
    pub author_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hybrid: Option<SubstanceSourceMaterialOrganismHybrid>,
    #[serde(rename = "maternalOrganismId", skip_serializing_if = "Option::is_none")]
    pub maternal_organism_id: Option<String>,
    #[serde(rename = "maternalOrganismName", skip_serializing_if = "Option::is_none")]
    pub maternal_organism_name: Option<String>,
    #[serde(rename = "paternalOrganismId", skip_serializing_if = "Option::is_none")]
    pub paternal_organism_id: Option<String>,
    #[serde(rename = "paternalOrganismName", skip_serializing_if = "Option::is_none")]
    pub paternal_organism_name: Option<String>,
    #[serde(rename = "hybridType", skip_serializing_if = "Option::is_none")]
    pub hybrid_type: Option<CodeableConcept>,
    #[serde(rename = "organismGeneral", skip_serializing_if = "Option::is_none")]
    pub organism_general: Option<SubstanceSourceMaterialOrganismOrganismGeneral>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kingdom: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phylum: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<CodeableConcept>,
    #[serde(rename = "partDescription", skip_serializing_if = "Option::is_none")]
    pub part_description: Option<Vec<SubstanceSourceMaterialPartDescription>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<CodeableConcept>,
    #[serde(rename = "partLocation", skip_serializing_if = "Option::is_none")]
    pub part_location: Option<CodeableConcept>,
}


/// Choice of types for the amount[x] field in SubstanceSpecification
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSpecificationAmountChoice {
    /// Variant accepting the Quantity type.
    #[serde(rename = "amountQuantity")]
    Quantity(Quantity),
    /// Variant accepting the String type.
    #[serde(rename = "amountString")]
    String(String),
}

/// Choice of types for the definingSubstance[x] field in SubstanceSpecification
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSpecificationDefiningSubstanceChoice {
    /// Variant accepting the Reference type.
    #[serde(rename = "definingSubstanceReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "definingSubstanceCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

/// Choice of types for the substance[x] field in SubstanceSpecification
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSpecificationSubstanceChoice {
    /// Variant accepting the Reference type.
    #[serde(rename = "substanceReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "substanceCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct SubstanceSpecification {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moiety: Option<Vec<SubstanceSpecificationMoiety>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stereochemistry: Option<CodeableConcept>,
    #[serde(rename = "opticalActivity", skip_serializing_if = "Option::is_none")]
    pub optical_activity: Option<CodeableConcept>,
    #[serde(rename = "molecularFormula", skip_serializing_if = "Option::is_none")]
    pub molecular_formula: Option<String>,
    #[serde(flatten)]
    pub amount: Option<SubstanceSpecificationAmountChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property: Option<Vec<SubstanceSpecificationProperty>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<String>,
    #[serde(flatten)]
    pub defining_substance: Option<SubstanceSpecificationDefiningSubstanceChoice>,
    #[serde(rename = "referenceInformation", skip_serializing_if = "Option::is_none")]
    pub reference_information: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structure: Option<SubstanceSpecificationStructure>,
    #[serde(rename = "molecularFormulaByMoiety", skip_serializing_if = "Option::is_none")]
    pub molecular_formula_by_moiety: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isotope: Option<Vec<SubstanceSpecificationStructureIsotope>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substitution: Option<CodeableConcept>,
    #[serde(rename = "halfLife", skip_serializing_if = "Option::is_none")]
    pub half_life: Option<Quantity>,
    #[serde(rename = "molecularWeight", skip_serializing_if = "Option::is_none")]
    pub molecular_weight: Option<SubstanceSpecificationStructureIsotopeMolecularWeight>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub representation: Option<Vec<SubstanceSpecificationStructureRepresentation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<Attachment>,
    #[serde(rename = "statusDate", skip_serializing_if = "Option::is_none")]
    pub status_date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synonym: Option<Vec<SubstanceSpecificationName>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation: Option<Vec<SubstanceSpecificationName>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub official: Option<Vec<SubstanceSpecificationNameOfficial>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<Vec<SubstanceSpecificationRelationship>>,
    #[serde(flatten)]
    pub substance: Option<SubstanceSpecificationSubstanceChoice>,
    #[serde(rename = "isDefining", skip_serializing_if = "Option::is_none")]
    pub is_defining: Option<Boolean>,
    #[serde(rename = "amountRatioLowLimit", skip_serializing_if = "Option::is_none")]
    pub amount_ratio_low_limit: Option<Ratio>,
    #[serde(rename = "amountType", skip_serializing_if = "Option::is_none")]
    pub amount_type: Option<CodeableConcept>,
    #[serde(rename = "nucleicAcid", skip_serializing_if = "Option::is_none")]
    pub nucleic_acid: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polymer: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protein: Option<Reference>,
    #[serde(rename = "sourceMaterial", skip_serializing_if = "Option::is_none")]
    pub source_material: Option<Reference>,
}


/// Choice of types for the item[x] field in SupplyDelivery
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SupplyDeliveryItemChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "itemReference")]
    Reference(Reference),
}

/// Choice of types for the occurrence[x] field in SupplyDelivery
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SupplyDeliveryOccurrenceChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct SupplyDelivery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patient: Option<Reference>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "suppliedItem", skip_serializing_if = "Option::is_none")]
    pub supplied_item: Option<SupplyDeliverySuppliedItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(flatten)]
    pub item: Option<SupplyDeliveryItemChoice>,
    #[serde(flatten)]
    pub occurrence: Option<SupplyDeliveryOccurrenceChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supplier: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver: Option<Vec<Reference>>,
}


/// Choice of types for the item[x] field in SupplyRequest
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SupplyRequestItemChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "itemReference")]
    Reference(Reference),
}

/// Choice of types for the value[x] field in SupplyRequest
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SupplyRequestValueChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
}

/// Choice of types for the occurrence[x] field in SupplyRequest
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SupplyRequestOccurrenceChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct SupplyRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Code>,
    #[serde(flatten)]
    pub item: Option<SupplyRequestItemChoice>,
    pub quantity: Quantity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<SupplyRequestParameter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(flatten)]
    pub value: Option<SupplyRequestValueChoice>,
    #[serde(flatten)]
    pub occurrence: Option<SupplyRequestOccurrenceChoice>,
    #[serde(rename = "authoredOn", skip_serializing_if = "Option::is_none")]
    pub authored_on: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supplier: Option<Vec<Reference>>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "deliverFrom", skip_serializing_if = "Option::is_none")]
    pub deliver_from: Option<Reference>,
    #[serde(rename = "deliverTo", skip_serializing_if = "Option::is_none")]
    pub deliver_to: Option<Reference>,
}


/// Choice of types for the value[x] field in Task
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum TaskValueChoice {
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "valueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[serde(rename = "valueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[serde(rename = "valueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[serde(rename = "valueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[serde(rename = "valueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "valuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "valueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[serde(rename = "valueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[serde(rename = "valueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[serde(rename = "valueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[serde(rename = "valueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[serde(rename = "valueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[serde(rename = "valueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[serde(rename = "valueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[serde(rename = "valueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[serde(rename = "valueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[serde(rename = "valueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[serde(rename = "valueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[serde(rename = "valueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[serde(rename = "valueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[serde(rename = "valueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[serde(rename = "valueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[serde(rename = "valueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "valueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[serde(rename = "valueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[serde(rename = "valueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[serde(rename = "valueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "valueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[serde(rename = "valueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[serde(rename = "valueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[serde(rename = "valueMeta")]
    Meta(Meta),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Task {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical", skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Canonical>,
    #[serde(rename = "instantiatesUri", skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Uri>,
    #[serde(rename = "basedOn", skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "groupIdentifier", skip_serializing_if = "Option::is_none")]
    pub group_identifier: Option<Identifier>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReason", skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<CodeableConcept>,
    #[serde(rename = "businessStatus", skip_serializing_if = "Option::is_none")]
    pub business_status: Option<CodeableConcept>,
    pub intent: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<Reference>,
    #[serde(rename = "for", skip_serializing_if = "Option::is_none")]
    pub r#for: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(rename = "executionPeriod", skip_serializing_if = "Option::is_none")]
    pub execution_period: Option<Period>,
    #[serde(rename = "authoredOn", skip_serializing_if = "Option::is_none")]
    pub authored_on: Option<DateTime>,
    #[serde(rename = "lastModified", skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester: Option<Reference>,
    #[serde(rename = "performerType", skip_serializing_if = "Option::is_none")]
    pub performer_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<CodeableConcept>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurance: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "relevantHistory", skip_serializing_if = "Option::is_none")]
    pub relevant_history: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restriction: Option<TaskRestriction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repetitions: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<TaskInput>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(flatten)]
    pub value: Option<TaskValueChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Vec<TaskOutput>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TerminologyCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    pub date: DateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    pub kind: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub software: Option<TerminologyCapabilitiesSoftware>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implementation: Option<TerminologyCapabilitiesImplementation>,
    #[serde(rename = "lockedDate", skip_serializing_if = "Option::is_none")]
    pub locked_date: Option<Boolean>,
    #[serde(rename = "codeSystem", skip_serializing_if = "Option::is_none")]
    pub code_system: Option<Vec<TerminologyCapabilitiesCodeSystem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<Canonical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(rename = "isDefault", skip_serializing_if = "Option::is_none")]
    pub is_default: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compositional: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Vec<TerminologyCapabilitiesCodeSystemVersionFilter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op: Option<Vec<Code>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property: Option<Vec<Code>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subsumption: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expansion: Option<TerminologyCapabilitiesExpansion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hierarchical: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paging: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incomplete: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<TerminologyCapabilitiesExpansionParameter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(rename = "textFilter", skip_serializing_if = "Option::is_none")]
    pub text_filter: Option<Markdown>,
    #[serde(rename = "codeSearch", skip_serializing_if = "Option::is_none")]
    pub code_search: Option<Code>,
    #[serde(rename = "validateCode", skip_serializing_if = "Option::is_none")]
    pub validate_code: Option<TerminologyCapabilitiesValidateCode>,
    pub translations: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation: Option<TerminologyCapabilitiesTranslation>,
    #[serde(rename = "needsMap")]
    pub needs_map: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closure: Option<TerminologyCapabilitiesClosure>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TestReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub status: Code,
    #[serde(rename = "testScript")]
    pub test_script: Reference,
    pub result: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tester: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant: Option<Vec<TestReportParticipant>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub uri: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup: Option<TestReportSetup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<TestReportSetupAction>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<TestReportSetupActionOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assert: Option<TestReportSetupActionAssert>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test: Option<Vec<TestReportTest>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teardown: Option<TestReportTeardown>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TestScript {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Vec<TestScriptOrigin>>,
    pub index: Integer,
    pub profile: Coding,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<Vec<TestScriptDestination>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<TestScriptMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<Vec<TestScriptMetadataLink>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability: Option<Vec<TestScriptMetadataCapability>>,
    pub required: Boolean,
    pub validated: Boolean,
    pub capabilities: Canonical,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixture: Option<Vec<TestScriptFixture>>,
    pub autocreate: Boolean,
    pub autodelete: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable: Option<Vec<TestScriptVariable>>,
    #[serde(rename = "defaultValue", skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(rename = "headerField", skip_serializing_if = "Option::is_none")]
    pub header_field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(rename = "sourceId", skip_serializing_if = "Option::is_none")]
    pub source_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup: Option<TestScriptSetup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<TestScriptSetupAction>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<TestScriptSetupActionOperation>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accept: Option<Code>,
    #[serde(rename = "contentType", skip_serializing_if = "Option::is_none")]
    pub content_type: Option<Code>,
    #[serde(rename = "encodeRequestUrl")]
    pub encode_request_url: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<String>,
    #[serde(rename = "requestHeader", skip_serializing_if = "Option::is_none")]
    pub request_header: Option<Vec<TestScriptSetupActionOperationRequestHeader>>,
    pub field: String,
    pub value: String,
    #[serde(rename = "requestId", skip_serializing_if = "Option::is_none")]
    pub request_id: Option<Id>,
    #[serde(rename = "responseId", skip_serializing_if = "Option::is_none")]
    pub response_id: Option<Id>,
    #[serde(rename = "targetId", skip_serializing_if = "Option::is_none")]
    pub target_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assert: Option<TestScriptSetupActionAssert>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<Code>,
    #[serde(rename = "compareToSourceId", skip_serializing_if = "Option::is_none")]
    pub compare_to_source_id: Option<String>,
    #[serde(rename = "compareToSourceExpression", skip_serializing_if = "Option::is_none")]
    pub compare_to_source_expression: Option<String>,
    #[serde(rename = "compareToSourcePath", skip_serializing_if = "Option::is_none")]
    pub compare_to_source_path: Option<String>,
    #[serde(rename = "minimumId", skip_serializing_if = "Option::is_none")]
    pub minimum_id: Option<String>,
    #[serde(rename = "navigationLinks", skip_serializing_if = "Option::is_none")]
    pub navigation_links: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator: Option<Code>,
    #[serde(rename = "requestMethod", skip_serializing_if = "Option::is_none")]
    pub request_method: Option<Code>,
    #[serde(rename = "requestURL", skip_serializing_if = "Option::is_none")]
    pub request_u_r_l: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<Code>,
    #[serde(rename = "responseCode", skip_serializing_if = "Option::is_none")]
    pub response_code: Option<String>,
    #[serde(rename = "validateProfileId", skip_serializing_if = "Option::is_none")]
    pub validate_profile_id: Option<Id>,
    #[serde(rename = "warningOnly")]
    pub warning_only: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test: Option<Vec<TestScriptTest>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teardown: Option<TestScriptTeardown>,
}


/// Choice of types for the value[x] field in ValueSet
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ValueSetValueChoice {
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Uri type.
    #[serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Code type.
    #[serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ValueSet {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext", skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub immutable: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compose: Option<ValueSetCompose>,
    #[serde(rename = "lockedDate", skip_serializing_if = "Option::is_none")]
    pub locked_date: Option<Date>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inactive: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<ValueSetComposeInclude>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept: Option<Vec<ValueSetComposeIncludeConcept>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub designation: Option<Vec<ValueSetComposeIncludeConceptDesignation>>,
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub r#use: Option<Coding>,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Vec<ValueSetComposeIncludeFilter>>,
    pub property: Code,
    pub op: Code,
    #[serde(rename = "valueSet", skip_serializing_if = "Option::is_none")]
    pub value_set: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<ValueSetComposeInclude>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expansion: Option<ValueSetExpansion>,
    pub timestamp: DateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<ValueSetExpansionParameter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contains: Option<Vec<ValueSetExpansionContains>>,
    #[serde(rename = "abstract", skip_serializing_if = "Option::is_none")]
    pub r#abstract: Option<Boolean>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct VerificationResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<Reference>>,
    #[serde(rename = "targetLocation", skip_serializing_if = "Option::is_none")]
    pub target_location: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub need: Option<CodeableConcept>,
    pub status: Code,
    #[serde(rename = "statusDate", skip_serializing_if = "Option::is_none")]
    pub status_date: Option<DateTime>,
    #[serde(rename = "validationType", skip_serializing_if = "Option::is_none")]
    pub validation_type: Option<CodeableConcept>,
    #[serde(rename = "validationProcess", skip_serializing_if = "Option::is_none")]
    pub validation_process: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency: Option<Timing>,
    #[serde(rename = "lastPerformed", skip_serializing_if = "Option::is_none")]
    pub last_performed: Option<DateTime>,
    #[serde(rename = "nextScheduled", skip_serializing_if = "Option::is_none")]
    pub next_scheduled: Option<Date>,
    #[serde(rename = "failureAction", skip_serializing_if = "Option::is_none")]
    pub failure_action: Option<CodeableConcept>,
    #[serde(rename = "primarySource", skip_serializing_if = "Option::is_none")]
    pub primary_source: Option<Vec<VerificationResultPrimarySource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub who: Option<Reference>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "communicationMethod", skip_serializing_if = "Option::is_none")]
    pub communication_method: Option<Vec<CodeableConcept>>,
    #[serde(rename = "validationStatus", skip_serializing_if = "Option::is_none")]
    pub validation_status: Option<CodeableConcept>,
    #[serde(rename = "validationDate", skip_serializing_if = "Option::is_none")]
    pub validation_date: Option<DateTime>,
    #[serde(rename = "canPushUpdates", skip_serializing_if = "Option::is_none")]
    pub can_push_updates: Option<CodeableConcept>,
    #[serde(rename = "pushTypeAvailable", skip_serializing_if = "Option::is_none")]
    pub push_type_available: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attestation: Option<VerificationResultAttestation>,
    #[serde(rename = "onBehalfOf", skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<Date>,
    #[serde(rename = "sourceIdentityCertificate", skip_serializing_if = "Option::is_none")]
    pub source_identity_certificate: Option<String>,
    #[serde(rename = "proxyIdentityCertificate", skip_serializing_if = "Option::is_none")]
    pub proxy_identity_certificate: Option<String>,
    #[serde(rename = "proxySignature", skip_serializing_if = "Option::is_none")]
    pub proxy_signature: Option<Signature>,
    #[serde(rename = "sourceSignature", skip_serializing_if = "Option::is_none")]
    pub source_signature: Option<Signature>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validator: Option<Vec<VerificationResultValidator>>,
    pub organization: Reference,
    #[serde(rename = "identityCertificate", skip_serializing_if = "Option::is_none")]
    pub identity_certificate: Option<String>,
    #[serde(rename = "attestationSignature", skip_serializing_if = "Option::is_none")]
    pub attestation_signature: Option<Signature>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct VisionPrescription {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules", skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub created: DateTime,
    pub patient: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(rename = "dateWritten")]
    pub date_written: DateTime,
    pub prescriber: Reference,
    #[serde(rename = "lensSpecification", skip_serializing_if = "Option::is_none")]
    pub lens_specification: Option<Vec<VisionPrescriptionLensSpecification>>,
    pub product: CodeableConcept,
    pub eye: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sphere: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cylinder: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prism: Option<Vec<VisionPrescriptionLensSpecificationPrism>>,
    pub amount: Decimal,
    pub base: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power: Option<Decimal>,
    #[serde(rename = "backCurve", skip_serializing_if = "Option::is_none")]
    pub back_curve: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diameter: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}


pub type Base64Binary = Element<std::string::String, Extension>;

pub type Boolean = Element<bool, Extension>;

pub type Canonical = Element<std::string::String, Extension>;

pub type Code = Element<std::string::String, Extension>;

pub type Date = Element<std::string::String, Extension>;

pub type DateTime = Element<std::string::String, Extension>;

pub type Decimal = DecimalElement<Extension>;

pub type Id = Element<std::string::String, Extension>;

pub type Instant = Element<std::string::String, Extension>;

pub type Integer = Element<std::primitive::i32, Extension>;

pub type Markdown = Element<std::string::String, Extension>;

pub type Oid = Element<std::string::String, Extension>;

pub type PositiveInt = Element<std::primitive::i32, Extension>;

pub type String = Element<std::string::String, Extension>;

pub type Time = Element<std::string::String, Extension>;

pub type UnsignedInt = Element<std::primitive::i32, Extension>;

pub type Uri = Element<std::string::String, Extension>;

pub type Url = Element<std::string::String, Extension>;

pub type Uuid = Element<std::string::String, Extension>;

pub type Xhtml = Element<std::string::String, Extension>;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Address {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub r#use: Option<Code>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub district: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(rename = "postalCode", skip_serializing_if = "Option::is_none")]
    pub postal_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Age {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparator: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
}


/// Choice of types for the author[x] field in Annotation
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum AnnotationAuthorChoice {
    /// Variant accepting the Reference type.
    #[serde(rename = "authorReference")]
    Reference(Reference),
    /// Variant accepting the String type.
    #[serde(rename = "authorString")]
    String(String),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Annotation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub author: Option<AnnotationAuthorChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<DateTime>,
    pub text: Markdown,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Attachment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "contentType", skip_serializing_if = "Option::is_none")]
    pub content_type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Base64Binary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Url>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<UnsignedInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<Base64Binary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation: Option<DateTime>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CodeableConcept {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coding: Option<Vec<Coding>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Coding {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(rename = "userSelected", skip_serializing_if = "Option::is_none")]
    pub user_selected: Option<Boolean>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ContactDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ContactPoint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub r#use: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Contributor {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Count {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparator: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
}


/// Choice of types for the subject[x] field in DataRequirement
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DataRequirementSubjectChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

/// Choice of types for the value[x] field in DataRequirement
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DataRequirementValueChoice {
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the Duration type.
    #[serde(rename = "valueDuration")]
    Duration(Duration),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct DataRequirement {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Vec<Canonical>>,
    #[serde(flatten)]
    pub subject: Option<DataRequirementSubjectChoice>,
    #[serde(rename = "mustSupport", skip_serializing_if = "Option::is_none")]
    pub must_support: Option<Vec<String>>,
    #[serde(rename = "codeFilter", skip_serializing_if = "Option::is_none")]
    pub code_filter: Option<Vec<DataRequirementCodeFilter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(rename = "searchParam", skip_serializing_if = "Option::is_none")]
    pub search_param: Option<String>,
    #[serde(rename = "valueSet", skip_serializing_if = "Option::is_none")]
    pub value_set: Option<Canonical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<Coding>>,
    #[serde(rename = "dateFilter", skip_serializing_if = "Option::is_none")]
    pub date_filter: Option<Vec<DataRequirementDateFilter>>,
    #[serde(flatten)]
    pub value: Option<DataRequirementValueChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<Vec<DataRequirementSort>>,
    pub direction: Code,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Distance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparator: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
}


/// Choice of types for the asNeeded[x] field in Dosage
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DosageAsNeededChoice {
    /// Variant accepting the Boolean type.
    #[serde(rename = "asNeededBoolean")]
    Boolean(Boolean),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "asNeededCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

/// Choice of types for the dose[x] field in Dosage
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DosageDoseChoice {
    /// Variant accepting the Range type.
    #[serde(rename = "doseRange")]
    Range(Range),
    /// Variant accepting the Quantity type.
    #[serde(rename = "doseQuantity")]
    Quantity(Quantity),
}

/// Choice of types for the rate[x] field in Dosage
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DosageRateChoice {
    /// Variant accepting the Ratio type.
    #[serde(rename = "rateRatio")]
    Ratio(Ratio),
    /// Variant accepting the Range type.
    #[serde(rename = "rateRange")]
    Range(Range),
    /// Variant accepting the Quantity type.
    #[serde(rename = "rateQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Dosage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(rename = "additionalInstruction", skip_serializing_if = "Option::is_none")]
    pub additional_instruction: Option<Vec<CodeableConcept>>,
    #[serde(rename = "patientInstruction", skip_serializing_if = "Option::is_none")]
    pub patient_instruction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing: Option<Timing>,
    #[serde(flatten)]
    pub as_needed: Option<DosageAsNeededChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(rename = "doseAndRate", skip_serializing_if = "Option::is_none")]
    pub dose_and_rate: Option<Vec<DosageDoseAndRate>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(flatten)]
    pub dose: Option<DosageDoseChoice>,
    #[serde(flatten)]
    pub rate: Option<DosageRateChoice>,
    #[serde(rename = "maxDosePerPeriod", skip_serializing_if = "Option::is_none")]
    pub max_dose_per_period: Option<Ratio>,
    #[serde(rename = "maxDosePerAdministration", skip_serializing_if = "Option::is_none")]
    pub max_dose_per_administration: Option<Quantity>,
    #[serde(rename = "maxDosePerLifetime", skip_serializing_if = "Option::is_none")]
    pub max_dose_per_lifetime: Option<Quantity>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Duration {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparator: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
}


/// Choice of types for the defaultValue[x] field in ElementDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionDefaultValueChoice {
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "defaultValueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[serde(rename = "defaultValueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "defaultValueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[serde(rename = "defaultValueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[serde(rename = "defaultValueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "defaultValueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "defaultValueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[serde(rename = "defaultValueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[serde(rename = "defaultValueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[serde(rename = "defaultValueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[serde(rename = "defaultValueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[serde(rename = "defaultValueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "defaultValuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "defaultValueString")]
    String(String),
    /// Variant accepting the Time type.
    #[serde(rename = "defaultValueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "defaultValueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[serde(rename = "defaultValueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[serde(rename = "defaultValueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[serde(rename = "defaultValueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[serde(rename = "defaultValueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[serde(rename = "defaultValueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[serde(rename = "defaultValueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[serde(rename = "defaultValueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "defaultValueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[serde(rename = "defaultValueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[serde(rename = "defaultValueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[serde(rename = "defaultValueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[serde(rename = "defaultValueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[serde(rename = "defaultValueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[serde(rename = "defaultValueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[serde(rename = "defaultValueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[serde(rename = "defaultValueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[serde(rename = "defaultValuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[serde(rename = "defaultValueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "defaultValueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "defaultValueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[serde(rename = "defaultValueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[serde(rename = "defaultValueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[serde(rename = "defaultValueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[serde(rename = "defaultValueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[serde(rename = "defaultValueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[serde(rename = "defaultValueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "defaultValueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[serde(rename = "defaultValueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[serde(rename = "defaultValueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[serde(rename = "defaultValueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "defaultValueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[serde(rename = "defaultValueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[serde(rename = "defaultValueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[serde(rename = "defaultValueMeta")]
    Meta(Meta),
}

/// Choice of types for the fixed[x] field in ElementDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionFixedChoice {
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "fixedBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[serde(rename = "fixedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "fixedCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[serde(rename = "fixedCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[serde(rename = "fixedDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "fixedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "fixedDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[serde(rename = "fixedId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[serde(rename = "fixedInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[serde(rename = "fixedInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[serde(rename = "fixedMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[serde(rename = "fixedOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "fixedPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "fixedString")]
    String(String),
    /// Variant accepting the Time type.
    #[serde(rename = "fixedTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "fixedUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[serde(rename = "fixedUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[serde(rename = "fixedUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[serde(rename = "fixedUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[serde(rename = "fixedAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[serde(rename = "fixedAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[serde(rename = "fixedAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[serde(rename = "fixedAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "fixedCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[serde(rename = "fixedCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[serde(rename = "fixedContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[serde(rename = "fixedCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[serde(rename = "fixedDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[serde(rename = "fixedDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[serde(rename = "fixedHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[serde(rename = "fixedIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[serde(rename = "fixedMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[serde(rename = "fixedPeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[serde(rename = "fixedQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "fixedRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "fixedRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[serde(rename = "fixedReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[serde(rename = "fixedSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[serde(rename = "fixedSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[serde(rename = "fixedTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[serde(rename = "fixedContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[serde(rename = "fixedContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "fixedDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[serde(rename = "fixedExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[serde(rename = "fixedParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[serde(rename = "fixedRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "fixedTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[serde(rename = "fixedUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[serde(rename = "fixedDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[serde(rename = "fixedMeta")]
    Meta(Meta),
}

/// Choice of types for the pattern[x] field in ElementDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionPatternChoice {
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "patternBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[serde(rename = "patternBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "patternCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[serde(rename = "patternCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[serde(rename = "patternDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "patternDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "patternDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[serde(rename = "patternId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[serde(rename = "patternInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[serde(rename = "patternInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[serde(rename = "patternMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[serde(rename = "patternOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "patternPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "patternString")]
    String(String),
    /// Variant accepting the Time type.
    #[serde(rename = "patternTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "patternUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[serde(rename = "patternUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[serde(rename = "patternUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[serde(rename = "patternUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[serde(rename = "patternAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[serde(rename = "patternAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[serde(rename = "patternAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[serde(rename = "patternAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "patternCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[serde(rename = "patternCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[serde(rename = "patternContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[serde(rename = "patternCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[serde(rename = "patternDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[serde(rename = "patternDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[serde(rename = "patternHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[serde(rename = "patternIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[serde(rename = "patternMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[serde(rename = "patternPeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[serde(rename = "patternQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "patternRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "patternRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[serde(rename = "patternReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[serde(rename = "patternSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[serde(rename = "patternSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[serde(rename = "patternTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[serde(rename = "patternContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[serde(rename = "patternContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "patternDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[serde(rename = "patternExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[serde(rename = "patternParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[serde(rename = "patternRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "patternTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[serde(rename = "patternUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[serde(rename = "patternDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[serde(rename = "patternMeta")]
    Meta(Meta),
}

/// Choice of types for the value[x] field in ElementDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionValueChoice {
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "valueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[serde(rename = "valueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[serde(rename = "valueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[serde(rename = "valueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[serde(rename = "valueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "valuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "valueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[serde(rename = "valueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[serde(rename = "valueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[serde(rename = "valueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[serde(rename = "valueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[serde(rename = "valueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[serde(rename = "valueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[serde(rename = "valueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[serde(rename = "valueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[serde(rename = "valueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[serde(rename = "valueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[serde(rename = "valueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[serde(rename = "valueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[serde(rename = "valueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[serde(rename = "valueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[serde(rename = "valueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[serde(rename = "valueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "valueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[serde(rename = "valueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[serde(rename = "valueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[serde(rename = "valueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "valueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[serde(rename = "valueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[serde(rename = "valueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[serde(rename = "valueMeta")]
    Meta(Meta),
}

/// Choice of types for the minValue[x] field in ElementDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionMinValueChoice {
    /// Variant accepting the Date type.
    #[serde(rename = "minValueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "minValueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Instant type.
    #[serde(rename = "minValueInstant")]
    Instant(Instant),
    /// Variant accepting the Time type.
    #[serde(rename = "minValueTime")]
    Time(Time),
    /// Variant accepting the Decimal type.
    #[serde(rename = "minValueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Integer type.
    #[serde(rename = "minValueInteger")]
    Integer(Integer),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "minValuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "minValueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Quantity type.
    #[serde(rename = "minValueQuantity")]
    Quantity(Quantity),
}

/// Choice of types for the maxValue[x] field in ElementDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionMaxValueChoice {
    /// Variant accepting the Date type.
    #[serde(rename = "maxValueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "maxValueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Instant type.
    #[serde(rename = "maxValueInstant")]
    Instant(Instant),
    /// Variant accepting the Time type.
    #[serde(rename = "maxValueTime")]
    Time(Time),
    /// Variant accepting the Decimal type.
    #[serde(rename = "maxValueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Integer type.
    #[serde(rename = "maxValueInteger")]
    Integer(Integer),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "maxValuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "maxValueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Quantity type.
    #[serde(rename = "maxValueQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ElementDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub representation: Option<Vec<Code>>,
    #[serde(rename = "sliceName", skip_serializing_if = "Option::is_none")]
    pub slice_name: Option<String>,
    #[serde(rename = "sliceIsConstraining", skip_serializing_if = "Option::is_none")]
    pub slice_is_constraining: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<Coding>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slicing: Option<ElementDefinitionSlicing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discriminator: Option<Vec<ElementDefinitionSlicingDiscriminator>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordered: Option<Boolean>,
    pub rules: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirements: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<UnsignedInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<ElementDefinitionBase>,
    #[serde(rename = "contentReference", skip_serializing_if = "Option::is_none")]
    pub content_reference: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Vec<Canonical>>,
    #[serde(rename = "targetProfile", skip_serializing_if = "Option::is_none")]
    pub target_profile: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregation: Option<Vec<Code>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub versioning: Option<Code>,
    #[serde(flatten)]
    pub default_value: Option<ElementDefinitionDefaultValueChoice>,
    #[serde(rename = "meaningWhenMissing", skip_serializing_if = "Option::is_none")]
    pub meaning_when_missing: Option<Markdown>,
    #[serde(rename = "orderMeaning", skip_serializing_if = "Option::is_none")]
    pub order_meaning: Option<String>,
    #[serde(flatten)]
    pub fixed: Option<ElementDefinitionFixedChoice>,
    #[serde(flatten)]
    pub pattern: Option<ElementDefinitionPatternChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<Vec<ElementDefinitionExample>>,
    #[serde(flatten)]
    pub value: Option<ElementDefinitionValueChoice>,
    #[serde(flatten)]
    pub min_value: Option<ElementDefinitionMinValueChoice>,
    #[serde(flatten)]
    pub max_value: Option<ElementDefinitionMaxValueChoice>,
    #[serde(rename = "maxLength", skip_serializing_if = "Option::is_none")]
    pub max_length: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Vec<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraint: Option<Vec<ElementDefinitionConstraint>>,
    pub key: Id,
    pub severity: Code,
    pub human: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xpath: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Canonical>,
    #[serde(rename = "mustSupport", skip_serializing_if = "Option::is_none")]
    pub must_support: Option<Boolean>,
    #[serde(rename = "isModifier", skip_serializing_if = "Option::is_none")]
    pub is_modifier: Option<Boolean>,
    #[serde(rename = "isModifierReason", skip_serializing_if = "Option::is_none")]
    pub is_modifier_reason: Option<String>,
    #[serde(rename = "isSummary", skip_serializing_if = "Option::is_none")]
    pub is_summary: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<ElementDefinitionBinding>,
    pub strength: Code,
    #[serde(rename = "valueSet", skip_serializing_if = "Option::is_none")]
    pub value_set: Option<Canonical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mapping: Option<Vec<ElementDefinitionMapping>>,
    pub identity: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    pub map: String,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Expression {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Id>,
    pub language: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Uri>,
}


/// Choice of types for the value[x] field in Extension
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ExtensionValueChoice {
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "valueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[serde(rename = "valueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[serde(rename = "valueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[serde(rename = "valueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[serde(rename = "valueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "valuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "valueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[serde(rename = "valueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[serde(rename = "valueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[serde(rename = "valueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[serde(rename = "valueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[serde(rename = "valueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[serde(rename = "valueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[serde(rename = "valueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[serde(rename = "valueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[serde(rename = "valueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[serde(rename = "valueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[serde(rename = "valueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[serde(rename = "valueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[serde(rename = "valueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[serde(rename = "valueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[serde(rename = "valueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[serde(rename = "valueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "valueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[serde(rename = "valueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[serde(rename = "valueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[serde(rename = "valueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "valueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[serde(rename = "valueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[serde(rename = "valueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[serde(rename = "valueMeta")]
    Meta(Meta),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Extension {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    pub url: std::string::String,
    #[serde(flatten)]
    pub value: Option<ExtensionValueChoice>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct HumanName {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub r#use: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub given: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Identifier {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub r#use: Option<Code>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assigner: Option<Box<Reference>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MarketingStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub country: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<CodeableConcept>,
    pub status: CodeableConcept,
    #[serde(rename = "dateRange")]
    pub date_range: Period,
    #[serde(rename = "restoreDate", skip_serializing_if = "Option::is_none")]
    pub restore_date: Option<DateTime>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Meta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "versionId", skip_serializing_if = "Option::is_none")]
    pub version_id: Option<Id>,
    #[serde(rename = "lastUpdated", skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<Instant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security: Option<Vec<Coding>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<Vec<Coding>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Money {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<Code>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Narrative {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    pub status: Code,
    pub div: Xhtml,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ParameterDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Code>,
    #[serde(rename = "use")]
    pub r#use: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Canonical>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Period {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<DateTime>,
}


/// Choice of types for the age[x] field in Population
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum PopulationAgeChoice {
    /// Variant accepting the Range type.
    #[serde(rename = "ageRange")]
    Range(Range),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "ageCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Population {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub age: Option<PopulationAgeChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub race: Option<CodeableConcept>,
    #[serde(rename = "physiologicalCondition", skip_serializing_if = "Option::is_none")]
    pub physiological_condition: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ProdCharacteristic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<Quantity>,
    #[serde(rename = "nominalVolume", skip_serializing_if = "Option::is_none")]
    pub nominal_volume: Option<Quantity>,
    #[serde(rename = "externalDiameter", skip_serializing_if = "Option::is_none")]
    pub external_diameter: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imprint: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<Vec<Attachment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scoring: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ProductShelfLife {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub period: Quantity,
    #[serde(rename = "specialPrecautionsForStorage", skip_serializing_if = "Option::is_none")]
    pub special_precautions_for_storage: Option<Vec<CodeableConcept>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Quantity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparator: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Range {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high: Option<Quantity>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Ratio {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub numerator: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denominator: Option<Quantity>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Reference {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Box<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RelatedArtifact {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Url>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<Attachment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Canonical>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct SampledData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    pub origin: Quantity,
    pub period: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(rename = "lowerLimit", skip_serializing_if = "Option::is_none")]
    pub lower_limit: Option<Decimal>,
    #[serde(rename = "upperLimit", skip_serializing_if = "Option::is_none")]
    pub upper_limit: Option<Decimal>,
    pub dimensions: PositiveInt,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Signature {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<Coding>>,
    pub when: Instant,
    pub who: Reference,
    #[serde(rename = "onBehalfOf", skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<Reference>,
    #[serde(rename = "targetFormat", skip_serializing_if = "Option::is_none")]
    pub target_format: Option<Code>,
    #[serde(rename = "sigFormat", skip_serializing_if = "Option::is_none")]
    pub sig_format: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Base64Binary>,
}


/// Choice of types for the amount[x] field in SubstanceAmount
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceAmountAmountChoice {
    /// Variant accepting the Quantity type.
    #[serde(rename = "amountQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "amountRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[serde(rename = "amountString")]
    String(String),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct SubstanceAmount {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub amount: Option<SubstanceAmountAmountChoice>,
    #[serde(rename = "amountType", skip_serializing_if = "Option::is_none")]
    pub amount_type: Option<CodeableConcept>,
    #[serde(rename = "amountText", skip_serializing_if = "Option::is_none")]
    pub amount_text: Option<String>,
    #[serde(rename = "referenceRange", skip_serializing_if = "Option::is_none")]
    pub reference_range: Option<SubstanceAmountReferenceRange>,
    #[serde(rename = "lowLimit", skip_serializing_if = "Option::is_none")]
    pub low_limit: Option<Quantity>,
    #[serde(rename = "highLimit", skip_serializing_if = "Option::is_none")]
    pub high_limit: Option<Quantity>,
}


/// Choice of types for the bounds[x] field in Timing
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum TimingBoundsChoice {
    /// Variant accepting the Duration type.
    #[serde(rename = "boundsDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[serde(rename = "boundsRange")]
    Range(Range),
    /// Variant accepting the Period type.
    #[serde(rename = "boundsPeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Timing {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<Vec<DateTime>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat: Option<TimingRepeat>,
    #[serde(flatten)]
    pub bounds: Option<TimingBoundsChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<PositiveInt>,
    #[serde(rename = "countMax", skip_serializing_if = "Option::is_none")]
    pub count_max: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<Decimal>,
    #[serde(rename = "durationMax", skip_serializing_if = "Option::is_none")]
    pub duration_max: Option<Decimal>,
    #[serde(rename = "durationUnit", skip_serializing_if = "Option::is_none")]
    pub duration_unit: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency: Option<PositiveInt>,
    #[serde(rename = "frequencyMax", skip_serializing_if = "Option::is_none")]
    pub frequency_max: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Decimal>,
    #[serde(rename = "periodMax", skip_serializing_if = "Option::is_none")]
    pub period_max: Option<Decimal>,
    #[serde(rename = "periodUnit", skip_serializing_if = "Option::is_none")]
    pub period_unit: Option<Code>,
    #[serde(rename = "dayOfWeek", skip_serializing_if = "Option::is_none")]
    pub day_of_week: Option<Vec<Code>>,
    #[serde(rename = "timeOfDay", skip_serializing_if = "Option::is_none")]
    pub time_of_day: Option<Vec<Time>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<Vec<Code>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<UnsignedInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
}


/// Choice of types for the timing[x] field in TriggerDefinition
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum TriggerDefinitionTimingChoice {
    /// Variant accepting the Timing type.
    #[serde(rename = "timingTiming")]
    Timing(Timing),
    /// Variant accepting the Reference type.
    #[serde(rename = "timingReference")]
    Reference(Reference),
    /// Variant accepting the Date type.
    #[serde(rename = "timingDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "timingDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TriggerDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(flatten)]
    pub timing: Option<TriggerDefinitionTimingChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<DataRequirement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Expression>,
}


/// Choice of types for the value[x] field in UsageContext
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum UsageContextValueChoice {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct UsageContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    pub code: Coding,
    #[serde(flatten)]
    pub value: Option<UsageContextValueChoice>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "resourceType")]
pub enum Resource {
    Account(Account),
    ActivityDefinition(ActivityDefinition),
    AdverseEvent(AdverseEvent),
    AllergyIntolerance(AllergyIntolerance),
    Appointment(Appointment),
    AppointmentResponse(AppointmentResponse),
    AuditEvent(AuditEvent),
    Basic(Basic),
    Binary(Binary),
    BiologicallyDerivedProduct(BiologicallyDerivedProduct),
    BodyStructure(BodyStructure),
    Bundle(Box<Bundle>),
    CapabilityStatement(CapabilityStatement),
    CarePlan(CarePlan),
    CareTeam(CareTeam),
    CatalogEntry(CatalogEntry),
    ChargeItem(ChargeItem),
    ChargeItemDefinition(ChargeItemDefinition),
    Claim(Claim),
    ClaimResponse(ClaimResponse),
    ClinicalImpression(ClinicalImpression),
    CodeSystem(CodeSystem),
    Communication(Communication),
    CommunicationRequest(CommunicationRequest),
    CompartmentDefinition(CompartmentDefinition),
    Composition(Composition),
    ConceptMap(ConceptMap),
    Condition(Condition),
    Consent(Consent),
    Contract(Contract),
    Coverage(Coverage),
    CoverageEligibilityRequest(CoverageEligibilityRequest),
    CoverageEligibilityResponse(CoverageEligibilityResponse),
    DetectedIssue(DetectedIssue),
    Device(Device),
    DeviceDefinition(DeviceDefinition),
    DeviceMetric(DeviceMetric),
    DeviceRequest(DeviceRequest),
    DeviceUseStatement(DeviceUseStatement),
    DiagnosticReport(DiagnosticReport),
    DocumentManifest(DocumentManifest),
    DocumentReference(DocumentReference),
    EffectEvidenceSynthesis(EffectEvidenceSynthesis),
    Encounter(Encounter),
    Endpoint(Endpoint),
    EnrollmentRequest(EnrollmentRequest),
    EnrollmentResponse(EnrollmentResponse),
    EpisodeOfCare(EpisodeOfCare),
    EventDefinition(EventDefinition),
    Evidence(Evidence),
    EvidenceVariable(EvidenceVariable),
    ExampleScenario(ExampleScenario),
    ExplanationOfBenefit(ExplanationOfBenefit),
    FamilyMemberHistory(FamilyMemberHistory),
    Flag(Flag),
    Goal(Goal),
    GraphDefinition(GraphDefinition),
    Group(Group),
    GuidanceResponse(GuidanceResponse),
    HealthcareService(HealthcareService),
    ImagingStudy(ImagingStudy),
    Immunization(Immunization),
    ImmunizationEvaluation(ImmunizationEvaluation),
    ImmunizationRecommendation(ImmunizationRecommendation),
    ImplementationGuide(ImplementationGuide),
    InsurancePlan(InsurancePlan),
    Invoice(Invoice),
    Library(Library),
    Linkage(Linkage),
    List(List),
    Location(Location),
    Measure(Measure),
    MeasureReport(MeasureReport),
    Media(Media),
    Medication(Medication),
    MedicationAdministration(MedicationAdministration),
    MedicationDispense(MedicationDispense),
    MedicationKnowledge(MedicationKnowledge),
    MedicationRequest(MedicationRequest),
    MedicationStatement(MedicationStatement),
    MedicinalProduct(MedicinalProduct),
    MedicinalProductAuthorization(MedicinalProductAuthorization),
    MedicinalProductContraindication(MedicinalProductContraindication),
    MedicinalProductIndication(MedicinalProductIndication),
    MedicinalProductIngredient(MedicinalProductIngredient),
    MedicinalProductInteraction(MedicinalProductInteraction),
    MedicinalProductManufactured(MedicinalProductManufactured),
    MedicinalProductPackaged(MedicinalProductPackaged),
    MedicinalProductPharmaceutical(MedicinalProductPharmaceutical),
    MedicinalProductUndesirableEffect(MedicinalProductUndesirableEffect),
    MessageDefinition(MessageDefinition),
    MessageHeader(MessageHeader),
    MolecularSequence(MolecularSequence),
    NamingSystem(NamingSystem),
    NutritionOrder(NutritionOrder),
    Observation(Observation),
    ObservationDefinition(ObservationDefinition),
    OperationDefinition(OperationDefinition),
    OperationOutcome(OperationOutcome),
    Organization(Organization),
    OrganizationAffiliation(OrganizationAffiliation),
    Parameters(Box<Parameters>),
    Patient(Patient),
    PaymentNotice(PaymentNotice),
    PaymentReconciliation(PaymentReconciliation),
    Person(Person),
    PlanDefinition(PlanDefinition),
    Practitioner(Practitioner),
    PractitionerRole(PractitionerRole),
    Procedure(Procedure),
    Provenance(Provenance),
    Questionnaire(Questionnaire),
    QuestionnaireResponse(QuestionnaireResponse),
    RelatedPerson(RelatedPerson),
    RequestGroup(RequestGroup),
    ResearchDefinition(ResearchDefinition),
    ResearchElementDefinition(ResearchElementDefinition),
    ResearchStudy(ResearchStudy),
    ResearchSubject(ResearchSubject),
    RiskAssessment(RiskAssessment),
    RiskEvidenceSynthesis(RiskEvidenceSynthesis),
    Schedule(Schedule),
    SearchParameter(SearchParameter),
    ServiceRequest(ServiceRequest),
    Slot(Slot),
    Specimen(Specimen),
    SpecimenDefinition(SpecimenDefinition),
    StructureDefinition(StructureDefinition),
    StructureMap(StructureMap),
    Subscription(Subscription),
    Substance(Substance),
    SubstanceNucleicAcid(SubstanceNucleicAcid),
    SubstancePolymer(SubstancePolymer),
    SubstanceProtein(SubstanceProtein),
    SubstanceReferenceInformation(SubstanceReferenceInformation),
    SubstanceSourceMaterial(SubstanceSourceMaterial),
    SubstanceSpecification(SubstanceSpecification),
    SupplyDelivery(SupplyDelivery),
    SupplyRequest(SupplyRequest),
    Task(Task),
    TerminologyCapabilities(TerminologyCapabilities),
    TestReport(TestReport),
    TestScript(TestScript),
    ValueSet(ValueSet),
    VerificationResult(VerificationResult),
    VisionPrescription(VisionPrescription),
}

