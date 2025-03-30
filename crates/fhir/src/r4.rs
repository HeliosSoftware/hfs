use serde::{Serialize, Deserialize};
use crate::{Element, DecimalElement};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub owner: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guarantor: Option<Vec<AccountGuarantor>>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountCoverage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub coverage: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<PositiveInt>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountGuarantor {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub party: Reference,
    #[serde(rename = "onHold", skip_serializing_if = "Option::is_none")]
    pub on_hold: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}


/// Choice of types for the subject[x] field in ActivityDefinition
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityDefinitionSubject {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

/// Choice of types for the timing[x] field in ActivityDefinition
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityDefinitionTiming {
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
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityDefinitionProduct {
    /// Variant accepting the Reference type.
    #[serde(rename = "productReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "productCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub subject: Option<ActivityDefinitionSubject>,
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
    pub timing: Option<ActivityDefinitionTiming>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant: Option<Vec<ActivityDefinitionParticipant>>,
    #[serde(flatten)]
    pub product: Option<ActivityDefinitionProduct>,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityDefinitionDynamicValue {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: String,
    pub expression: Expression,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityDefinitionParticipant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdverseEventSuspectEntityCausality {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assessment: Option<CodeableConcept>,
    #[serde(rename = "productRelatedness", skip_serializing_if = "Option::is_none")]
    pub product_relatedness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdverseEventSuspectEntity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub instance: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causality: Option<Vec<AdverseEventSuspectEntityCausality>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "subjectMedicalHistory", skip_serializing_if = "Option::is_none")]
    pub subject_medical_history: Option<Vec<Reference>>,
    #[serde(rename = "referenceDocument", skip_serializing_if = "Option::is_none")]
    pub reference_document: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub study: Option<Vec<Reference>>,
}


/// Choice of types for the onset[x] field in AllergyIntolerance
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AllergyIntoleranceOnset {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub onset: Option<AllergyIntoleranceOnset>,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllergyIntoleranceReaction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substance: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifestation: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onset: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<Code>,
    #[serde(rename = "exposureRoute", skip_serializing_if = "Option::is_none")]
    pub exposure_route: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppointmentParticipant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Code>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "requestedPeriod", skip_serializing_if = "Option::is_none")]
    pub requested_period: Option<Vec<Period>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


/// Choice of types for the value[x] field in AuditEventEntityDetail
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuditEventEntityDetailValue {
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEventEntityDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(flatten)]
    pub value: Option<AuditEventEntityDetailValue>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEventSource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    pub observer: Reference,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<Coding>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEventEntity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub what: Option<Reference>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<Coding>,
    #[serde(rename = "securityLabel", skip_serializing_if = "Option::is_none")]
    pub security_label: Option<Vec<Coding>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<Base64Binary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<AuditEventEntityDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEventAgentNetwork {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub source: AuditEventSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<Vec<AuditEventEntity>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEventAgent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
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
    #[serde(rename = "purposeOfUse", skip_serializing_if = "Option::is_none")]
    pub purpose_of_use: Option<Vec<CodeableConcept>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


/// Choice of types for the time[x] field in BiologicallyDerivedProductManipulation
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BiologicallyDerivedProductManipulationTime {
    /// Variant accepting the DateTime type.
    #[serde(rename = "timeDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "timePeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BiologicallyDerivedProductManipulation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub time: Option<BiologicallyDerivedProductManipulationTime>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BiologicallyDerivedProductStorage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub processing: Option<Vec<BiologicallyDerivedProductProcessing>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manipulation: Option<BiologicallyDerivedProductManipulation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<Vec<BiologicallyDerivedProductStorage>>,
}

/// Choice of types for the time[x] field in BiologicallyDerivedProductProcessing
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BiologicallyDerivedProductProcessingTime {
    /// Variant accepting the DateTime type.
    #[serde(rename = "timeDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "timePeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BiologicallyDerivedProductProcessing {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additive: Option<Reference>,
    #[serde(flatten)]
    pub time: Option<BiologicallyDerivedProductProcessingTime>,
}

/// Choice of types for the collected[x] field in BiologicallyDerivedProductCollection
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BiologicallyDerivedProductCollectionCollected {
    /// Variant accepting the DateTime type.
    #[serde(rename = "collectedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "collectedPeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BiologicallyDerivedProductCollection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collector: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Reference>,
    #[serde(flatten)]
    pub collected: Option<BiologicallyDerivedProductCollectionCollected>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<Vec<BundleLink>>,
    #[serde(rename = "fullUrl", skip_serializing_if = "Option::is_none")]
    pub full_url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Resource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<BundleEntrySearch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<BundleEntryRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<BundleEntryResponse>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleEntryRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub method: Code,
    pub url: Uri,
    #[serde(rename = "ifNoneMatch", skip_serializing_if = "Option::is_none")]
    pub if_none_match: Option<String>,
    #[serde(rename = "ifModifiedSince", skip_serializing_if = "Option::is_none")]
    pub if_modified_since: Option<Instant>,
    #[serde(rename = "ifMatch", skip_serializing_if = "Option::is_none")]
    pub if_match: Option<String>,
    #[serde(rename = "ifNoneExist", skip_serializing_if = "Option::is_none")]
    pub if_none_exist: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleEntryResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(rename = "lastModified", skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<Instant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Resource>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleEntrySearch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<Decimal>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub entry: Option<Vec<BundleEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleLink {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub relation: String,
    pub url: Uri,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementRest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security: Option<CapabilityStatementRestSecurity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Vec<CapabilityStatementRestResource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction: Option<Vec<CapabilityStatementRestInteraction>>,
    #[serde(rename = "searchParam", skip_serializing_if = "Option::is_none")]
    pub search_param: Option<Vec<CapabilityStatementRestResourceSearchParam>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<Vec<CapabilityStatementRestResourceOperation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compartment: Option<Vec<Canonical>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementRestInteraction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementSoftware {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(rename = "releaseDate", skip_serializing_if = "Option::is_none")]
    pub release_date: Option<DateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementRestResourceOperation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub definition: Canonical,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementMessaging {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<CapabilityStatementMessagingEndpoint>>,
    #[serde(rename = "reliableCache", skip_serializing_if = "Option::is_none")]
    pub reliable_cache: Option<UnsignedInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Markdown>,
    #[serde(rename = "supportedMessage", skip_serializing_if = "Option::is_none")]
    pub supported_message: Option<Vec<CapabilityStatementMessagingSupportedMessage>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementRestSecurity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cors: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementRestResourceInteraction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implementation: Option<CapabilityStatementImplementation>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messaging: Option<Vec<CapabilityStatementMessaging>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<Vec<CapabilityStatementDocument>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementMessagingEndpoint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub protocol: Coding,
    pub address: Url,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementRestResourceSearchParam {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<Canonical>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementMessagingSupportedMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    pub definition: Canonical,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementImplementation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Url>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custodian: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementRestResource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Canonical>,
    #[serde(rename = "supportedProfile", skip_serializing_if = "Option::is_none")]
    pub supported_profile: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction: Option<Vec<CapabilityStatementRestResourceInteraction>>,
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
    pub operation: Option<Vec<CapabilityStatementRestResourceOperation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementDocument {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Markdown>,
    pub profile: Canonical,
}


/// Choice of types for the scheduled[x] field in CarePlanActivityDetail
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CarePlanActivityDetailScheduled {
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

/// Choice of types for the product[x] field in CarePlanActivityDetail
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CarePlanActivityDetailProduct {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "productCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "productReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarePlanActivityDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<Code>,
    #[serde(rename = "instantiatesCanonical", skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri", skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReason", skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<CodeableConcept>,
    #[serde(rename = "doNotPerform", skip_serializing_if = "Option::is_none")]
    pub do_not_perform: Option<Boolean>,
    #[serde(flatten)]
    pub scheduled: Option<CarePlanActivityDetailScheduled>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<Reference>>,
    #[serde(flatten)]
    pub product: Option<CarePlanActivityDetailProduct>,
    #[serde(rename = "dailyAmount", skip_serializing_if = "Option::is_none")]
    pub daily_amount: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarePlanActivity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
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
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CareTeamParticipant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member: Option<Reference>,
    #[serde(rename = "onBehalfOf", skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntryRelatedEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub relationtype: Code,
    pub item: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeItemPerformer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

/// Choice of types for the occurrence[x] field in ChargeItem
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChargeItemOccurrence {
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
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChargeItemProduct {
    /// Variant accepting the Reference type.
    #[serde(rename = "productReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "productCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub occurrence: Option<ChargeItemOccurrence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<ChargeItemPerformer>>,
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
    pub product: Option<ChargeItemProduct>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "supportingInformation", skip_serializing_if = "Option::is_none")]
    pub supporting_information: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "propertyGroup", skip_serializing_if = "Option::is_none")]
    pub property_group: Option<Vec<ChargeItemDefinitionPropertyGroup>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeItemDefinitionApplicability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeItemDefinitionPropertyGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applicability: Option<Vec<ChargeItemDefinitionApplicability>>,
    #[serde(rename = "priceComponent", skip_serializing_if = "Option::is_none")]
    pub price_component: Option<Vec<ChargeItemDefinitionPropertyGroupPriceComponent>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeItemDefinitionPropertyGroupPriceComponent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Money>,
}


/// Choice of types for the diagnosis[x] field in ClaimDiagnosis
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimDiagnosisDiagnosis {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "diagnosisCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "diagnosisReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimDiagnosis {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(flatten)]
    pub diagnosis: Option<ClaimDiagnosisDiagnosis>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "onAdmission", skip_serializing_if = "Option::is_none")]
    pub on_admission: Option<CodeableConcept>,
    #[serde(rename = "packageCode", skip_serializing_if = "Option::is_none")]
    pub package_code: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub prescription: Option<Reference>,
    #[serde(rename = "originalPrescription", skip_serializing_if = "Option::is_none")]
    pub original_prescription: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payee: Option<ClaimPayee>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referral: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facility: Option<Reference>,
    #[serde(rename = "careTeam", skip_serializing_if = "Option::is_none")]
    pub care_team: Option<Vec<ClaimCareTeam>>,
    #[serde(rename = "supportingInfo", skip_serializing_if = "Option::is_none")]
    pub supporting_info: Option<Vec<ClaimSupportingInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<Vec<ClaimDiagnosis>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure: Option<Vec<ClaimProcedure>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurance: Option<Vec<ClaimInsurance>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accident: Option<ClaimAccident>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<ClaimItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<Money>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimInsurance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub focal: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    pub coverage: Reference,
    #[serde(rename = "businessArrangement", skip_serializing_if = "Option::is_none")]
    pub business_arrangement: Option<String>,
    #[serde(rename = "preAuthRef", skip_serializing_if = "Option::is_none")]
    pub pre_auth_ref: Option<Vec<String>>,
    #[serde(rename = "claimResponse", skip_serializing_if = "Option::is_none")]
    pub claim_response: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimRelated {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Identifier>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimPayee {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<Reference>,
}

/// Choice of types for the serviced[x] field in ClaimItem
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimItemServiced {
    /// Variant accepting the Date type.
    #[serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "servicedPeriod")]
    Period(Period),
}

/// Choice of types for the location[x] field in ClaimItem
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimItemLocation {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode", skip_serializing_if = "Option::is_none")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(flatten)]
    pub serviced: Option<ClaimItemServiced>,
    #[serde(flatten)]
    pub location: Option<ClaimItemLocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice", skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi: Option<Vec<Reference>>,
    #[serde(rename = "bodySite", skip_serializing_if = "Option::is_none")]
    pub body_site: Option<CodeableConcept>,
    #[serde(rename = "subSite", skip_serializing_if = "Option::is_none")]
    pub sub_site: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<ClaimItemDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimItemDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revenue: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode", skip_serializing_if = "Option::is_none")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice", skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi: Option<Vec<Reference>>,
    #[serde(rename = "subDetail", skip_serializing_if = "Option::is_none")]
    pub sub_detail: Option<Vec<ClaimItemDetailSubDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimItemDetailSubDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revenue: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode", skip_serializing_if = "Option::is_none")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice", skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi: Option<Vec<Reference>>,
}

/// Choice of types for the location[x] field in ClaimAccident
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimAccidentLocation {
    /// Variant accepting the Address type.
    #[serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[serde(rename = "locationReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimAccident {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub date: Date,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(flatten)]
    pub location: Option<ClaimAccidentLocation>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimCareTeam {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub provider: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsible: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualification: Option<CodeableConcept>,
}

/// Choice of types for the procedure[x] field in ClaimProcedure
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimProcedureProcedure {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "procedureCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "procedureReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimProcedure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(flatten)]
    pub procedure: Option<ClaimProcedureProcedure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi: Option<Vec<Reference>>,
}

/// Choice of types for the timing[x] field in ClaimSupportingInfo
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimSupportingInfoTiming {
    /// Variant accepting the Date type.
    #[serde(rename = "timingDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "timingPeriod")]
    Period(Period),
}

/// Choice of types for the value[x] field in ClaimSupportingInfo
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimSupportingInfoValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimSupportingInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub category: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(flatten)]
    pub timing: Option<ClaimSupportingInfoTiming>,
    #[serde(flatten)]
    pub value: Option<ClaimSupportingInfoValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseInsurance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub focal: Boolean,
    pub coverage: Reference,
    #[serde(rename = "businessArrangement", skip_serializing_if = "Option::is_none")]
    pub business_arrangement: Option<String>,
    #[serde(rename = "claimResponse", skip_serializing_if = "Option::is_none")]
    pub claim_response: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseError {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemSequence", skip_serializing_if = "Option::is_none")]
    pub item_sequence: Option<PositiveInt>,
    #[serde(rename = "detailSequence", skip_serializing_if = "Option::is_none")]
    pub detail_sequence: Option<PositiveInt>,
    #[serde(rename = "subDetailSequence", skip_serializing_if = "Option::is_none")]
    pub sub_detail_sequence: Option<PositiveInt>,
    pub code: CodeableConcept,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "addItem", skip_serializing_if = "Option::is_none")]
    pub add_item: Option<Vec<ClaimResponseAddItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<Vec<ClaimResponseTotal>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment: Option<ClaimResponsePayment>,
    #[serde(rename = "fundsReserve", skip_serializing_if = "Option::is_none")]
    pub funds_reserve: Option<CodeableConcept>,
    #[serde(rename = "formCode", skip_serializing_if = "Option::is_none")]
    pub form_code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form: Option<Attachment>,
    #[serde(rename = "processNote", skip_serializing_if = "Option::is_none")]
    pub process_note: Option<Vec<ClaimResponseProcessNote>>,
    #[serde(rename = "communicationRequest", skip_serializing_if = "Option::is_none")]
    pub communication_request: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurance: Option<Vec<ClaimResponseInsurance>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Vec<ClaimResponseError>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseTotal {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub amount: Money,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemSequence")]
    pub item_sequence: PositiveInt,
    #[serde(rename = "noteNumber", skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<ClaimResponseItemDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseItemDetailSubDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "subDetailSequence")]
    pub sub_detail_sequence: PositiveInt,
    #[serde(rename = "noteNumber", skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseAddItemDetailSubDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice", skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(rename = "noteNumber", skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponsePayment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjustment: Option<Money>,
    #[serde(rename = "adjustmentReason", skip_serializing_if = "Option::is_none")]
    pub adjustment_reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<Date>,
    pub amount: Money,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseItemDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "detailSequence")]
    pub detail_sequence: PositiveInt,
    #[serde(rename = "noteNumber", skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    #[serde(rename = "subDetail", skip_serializing_if = "Option::is_none")]
    pub sub_detail: Option<Vec<ClaimResponseItemDetailSubDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseAddItemDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice", skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(rename = "noteNumber", skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    #[serde(rename = "subDetail", skip_serializing_if = "Option::is_none")]
    pub sub_detail: Option<Vec<ClaimResponseAddItemDetailSubDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseItemAdjudication {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Decimal>,
}

/// Choice of types for the serviced[x] field in ClaimResponseAddItem
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimResponseAddItemServiced {
    /// Variant accepting the Date type.
    #[serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "servicedPeriod")]
    Period(Period),
}

/// Choice of types for the location[x] field in ClaimResponseAddItem
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimResponseAddItemLocation {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseAddItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemSequence", skip_serializing_if = "Option::is_none")]
    pub item_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "detailSequence", skip_serializing_if = "Option::is_none")]
    pub detail_sequence: Option<Vec<PositiveInt>>,
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
    pub serviced: Option<ClaimResponseAddItemServiced>,
    #[serde(flatten)]
    pub location: Option<ClaimResponseAddItemLocation>,
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
    #[serde(rename = "noteNumber", skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<ClaimResponseAddItemDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseProcessNote {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<PositiveInt>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClinicalImpressionFinding {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemCodeableConcept", skip_serializing_if = "Option::is_none")]
    pub item_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "itemReference", skip_serializing_if = "Option::is_none")]
    pub item_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub basis: Option<String>,
}

/// Choice of types for the effective[x] field in ClinicalImpression
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClinicalImpressionEffective {
    /// Variant accepting the DateTime type.
    #[serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "effectivePeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub effective: Option<ClinicalImpressionEffective>,
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
    pub protocol: Option<Vec<Uri>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finding: Option<Vec<ClinicalImpressionFinding>>,
    #[serde(rename = "prognosisCodeableConcept", skip_serializing_if = "Option::is_none")]
    pub prognosis_codeable_concept: Option<Vec<CodeableConcept>>,
    #[serde(rename = "prognosisReference", skip_serializing_if = "Option::is_none")]
    pub prognosis_reference: Option<Vec<Reference>>,
    #[serde(rename = "supportingInfo", skip_serializing_if = "Option::is_none")]
    pub supporting_info: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClinicalImpressionInvestigation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property: Option<Vec<CodeSystemProperty>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept: Option<Vec<CodeSystemConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeSystemConcept {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub designation: Option<Vec<CodeSystemConceptDesignation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property: Option<Vec<CodeSystemConceptProperty>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept: Option<Vec<CodeSystemConcept>>,
}

/// Choice of types for the value[x] field in CodeSystemConceptProperty
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CodeSystemConceptPropertyValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeSystemConceptProperty {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(flatten)]
    pub value: Option<CodeSystemConceptPropertyValue>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeSystemProperty {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Code,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeSystemConceptDesignation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub r#use: Option<Coding>,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeSystemFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator: Option<Vec<Code>>,
    pub value: String,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the content[x] field in CommunicationPayload
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommunicationPayloadContent {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommunicationPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub content: Option<CommunicationPayloadContent>,
}


/// Choice of types for the content[x] field in CommunicationRequestPayload
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommunicationRequestPayloadContent {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommunicationRequestPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub content: Option<CommunicationRequestPayloadContent>,
}

/// Choice of types for the occurrence[x] field in CommunicationRequest
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommunicationRequestOccurrence {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "occurrencePeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub occurrence: Option<CommunicationRequestOccurrence>,
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompartmentDefinitionResource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionSection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Code>,
    #[serde(rename = "orderedBy", skip_serializing_if = "Option::is_none")]
    pub ordered_by: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<Vec<Reference>>,
    #[serde(rename = "emptyReason", skip_serializing_if = "Option::is_none")]
    pub empty_reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<Vec<CompositionSection>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custodian: Option<Reference>,
    #[serde(rename = "relatesTo", skip_serializing_if = "Option::is_none")]
    pub relates_to: Option<Vec<CompositionRelatesTo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<Vec<CompositionEvent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<Vec<CompositionSection>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionAttester {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<Reference>>,
}

/// Choice of types for the target[x] field in CompositionRelatesTo
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompositionRelatesToTarget {
    /// Variant accepting the Identifier type.
    #[serde(rename = "targetIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Reference type.
    #[serde(rename = "targetReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionRelatesTo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(flatten)]
    pub target: Option<CompositionRelatesToTarget>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConceptMapGroupElementTargetDependsOn {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub property: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Canonical>,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConceptMapGroupElement {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<ConceptMapGroupElementTarget>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConceptMapGroupUnmapped {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Canonical>,
}

/// Choice of types for the source[x] field in ConceptMap
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConceptMapSource {
    /// Variant accepting the Uri type.
    #[serde(rename = "sourceUri")]
    Uri(Uri),
    /// Variant accepting the Canonical type.
    #[serde(rename = "sourceCanonical")]
    Canonical(Canonical),
}

/// Choice of types for the target[x] field in ConceptMap
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConceptMapTarget {
    /// Variant accepting the Uri type.
    #[serde(rename = "targetUri")]
    Uri(Uri),
    /// Variant accepting the Canonical type.
    #[serde(rename = "targetCanonical")]
    Canonical(Canonical),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub source: Option<ConceptMapSource>,
    #[serde(flatten)]
    pub target: Option<ConceptMapTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Vec<ConceptMapGroup>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConceptMapGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Uri>,
    #[serde(rename = "sourceVersion", skip_serializing_if = "Option::is_none")]
    pub source_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Uri>,
    #[serde(rename = "targetVersion", skip_serializing_if = "Option::is_none")]
    pub target_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<Vec<ConceptMapGroupElement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unmapped: Option<ConceptMapGroupUnmapped>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConceptMapGroupElementTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    pub equivalence: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(rename = "dependsOn", skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<ConceptMapGroupElementTargetDependsOn>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<Vec<ConceptMapGroupElementTargetDependsOn>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionEvidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<Reference>>,
}

/// Choice of types for the onset[x] field in Condition
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConditionOnset {
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
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConditionAbatement {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub onset: Option<ConditionOnset>,
    #[serde(flatten)]
    pub abatement: Option<ConditionAbatement>,
    #[serde(rename = "recordedDate", skip_serializing_if = "Option::is_none")]
    pub recorded_date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorder: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asserter: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<Vec<ConditionStage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<ConditionEvidence>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionStage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assessment: Option<Vec<Reference>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentVerification {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub verified: Boolean,
    #[serde(rename = "verifiedWith", skip_serializing_if = "Option::is_none")]
    pub verified_with: Option<Reference>,
    #[serde(rename = "verificationDate", skip_serializing_if = "Option::is_none")]
    pub verification_date: Option<DateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentProvisionData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub meaning: Code,
    pub reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentProvisionActor {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: CodeableConcept,
    pub reference: Reference,
}

/// Choice of types for the source[x] field in Consent
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConsentSource {
    /// Variant accepting the Attachment type.
    #[serde(rename = "sourceAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "sourceReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub source: Option<ConsentSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<Vec<ConsentPolicy>>,
    #[serde(rename = "policyRule", skip_serializing_if = "Option::is_none")]
    pub policy_rule: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification: Option<Vec<ConsentVerification>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provision: Option<ConsentProvision>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentPolicy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<Uri>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentProvision {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Vec<ConsentProvisionActor>>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provision: Option<Vec<ConsentProvision>>,
}


/// Choice of types for the value[x] field in ContractTermOfferAnswer
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractTermOfferAnswerValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTermOfferAnswer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub value: Option<ContractTermOfferAnswerValue>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractSigner {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Coding,
    pub party: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<Signature>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTermSecurityLabel {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<Vec<UnsignedInt>>,
    pub classification: Coding,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<Coding>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control: Option<Vec<Coding>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTermAssetContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTermActionSubject {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTermAsset {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<CodeableConcept>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "typeReference", skip_serializing_if = "Option::is_none")]
    pub type_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtype: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Vec<ContractTermAssetContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(rename = "periodType", skip_serializing_if = "Option::is_none")]
    pub period_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Vec<Period>>,
    #[serde(rename = "usePeriod", skip_serializing_if = "Option::is_none")]
    pub use_period: Option<Vec<Period>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(rename = "linkId", skip_serializing_if = "Option::is_none")]
    pub link_id: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<Vec<ContractTermOfferAnswer>>,
    #[serde(rename = "securityLabelNumber", skip_serializing_if = "Option::is_none")]
    pub security_label_number: Option<Vec<UnsignedInt>>,
    #[serde(rename = "valuedItem", skip_serializing_if = "Option::is_none")]
    pub valued_item: Option<Vec<ContractTermAssetValuedItem>>,
}

/// Choice of types for the occurrence[x] field in ContractTermAction
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractTermActionOccurrence {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTermAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "doNotPerform", skip_serializing_if = "Option::is_none")]
    pub do_not_perform: Option<Boolean>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Vec<ContractTermActionSubject>>,
    pub intent: CodeableConcept,
    #[serde(rename = "linkId", skip_serializing_if = "Option::is_none")]
    pub link_id: Option<Vec<String>>,
    pub status: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Reference>,
    #[serde(rename = "contextLinkId", skip_serializing_if = "Option::is_none")]
    pub context_link_id: Option<Vec<String>>,
    #[serde(flatten)]
    pub occurrence: Option<ContractTermActionOccurrence>,
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
    #[serde(rename = "securityLabelNumber", skip_serializing_if = "Option::is_none")]
    pub security_label_number: Option<Vec<UnsignedInt>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTermOfferParty {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Vec<Reference>>,
    pub role: CodeableConcept,
}

/// Choice of types for the content[x] field in ContractRule
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractRuleContent {
    /// Variant accepting the Attachment type.
    #[serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "contentReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractRule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub content: Option<ContractRuleContent>,
}

/// Choice of types for the topic[x] field in Contract
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractTopic {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "topicCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "topicReference")]
    Reference(Reference),
}

/// Choice of types for the legallyBinding[x] field in Contract
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractLegallyBinding {
    /// Variant accepting the Attachment type.
    #[serde(rename = "legallyBindingAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "legallyBindingReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub topic: Option<ContractTopic>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "subType", skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "contentDefinition", skip_serializing_if = "Option::is_none")]
    pub content_definition: Option<ContractContentDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub term: Option<Vec<ContractTerm>>,
    #[serde(rename = "supportingInfo", skip_serializing_if = "Option::is_none")]
    pub supporting_info: Option<Vec<Reference>>,
    #[serde(rename = "relevantHistory", skip_serializing_if = "Option::is_none")]
    pub relevant_history: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signer: Option<Vec<ContractSigner>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friendly: Option<Vec<ContractFriendly>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal: Option<Vec<ContractLegal>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<Vec<ContractRule>>,
    #[serde(flatten)]
    pub legally_binding: Option<ContractLegallyBinding>,
}

/// Choice of types for the content[x] field in ContractLegal
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractLegalContent {
    /// Variant accepting the Attachment type.
    #[serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "contentReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractLegal {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub content: Option<ContractLegalContent>,
}

/// Choice of types for the content[x] field in ContractFriendly
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractFriendlyContent {
    /// Variant accepting the Attachment type.
    #[serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "contentReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractFriendly {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub content: Option<ContractFriendlyContent>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTermOffer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<Vec<ContractTermOfferParty>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<Reference>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<CodeableConcept>,
    #[serde(rename = "decisionMode", skip_serializing_if = "Option::is_none")]
    pub decision_mode: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<Vec<ContractTermOfferAnswer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(rename = "linkId", skip_serializing_if = "Option::is_none")]
    pub link_id: Option<Vec<String>>,
    #[serde(rename = "securityLabelNumber", skip_serializing_if = "Option::is_none")]
    pub security_label_number: Option<Vec<UnsignedInt>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractContentDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "subType", skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<Reference>,
    #[serde(rename = "publicationDate", skip_serializing_if = "Option::is_none")]
    pub publication_date: Option<DateTime>,
    #[serde(rename = "publicationStatus")]
    pub publication_status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
}

/// Choice of types for the topic[x] field in ContractTerm
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractTermTopic {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "topicCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "topicReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTerm {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applies: Option<Period>,
    #[serde(flatten)]
    pub topic: Option<ContractTermTopic>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "subType", skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(rename = "securityLabel", skip_serializing_if = "Option::is_none")]
    pub security_label: Option<Vec<ContractTermSecurityLabel>>,
    pub offer: ContractTermOffer,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<Vec<ContractTermAsset>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<ContractTermAction>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Vec<ContractTerm>>,
}

/// Choice of types for the entity[x] field in ContractTermAssetValuedItem
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractTermAssetValuedItemEntity {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "entityCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "entityReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTermAssetValuedItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub entity: Option<ContractTermAssetValuedItemEntity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
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
    #[serde(rename = "linkId", skip_serializing_if = "Option::is_none")]
    pub link_id: Option<Vec<String>>,
    #[serde(rename = "securityLabelNumber", skip_serializing_if = "Option::is_none")]
    pub security_label_number: Option<Vec<UnsignedInt>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(rename = "costToBeneficiary", skip_serializing_if = "Option::is_none")]
    pub cost_to_beneficiary: Option<Vec<CoverageCostToBeneficiary>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subrogation: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageClass {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageCostToBeneficiaryException {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}

/// Choice of types for the value[x] field in CoverageCostToBeneficiary
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageCostToBeneficiaryValue {
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Money type.
    #[serde(rename = "valueMoney")]
    Money(Money),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageCostToBeneficiary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(flatten)]
    pub value: Option<CoverageCostToBeneficiaryValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exception: Option<Vec<CoverageCostToBeneficiaryException>>,
}


/// Choice of types for the diagnosis[x] field in CoverageEligibilityRequestItemDiagnosis
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityRequestItemDiagnosisDiagnosis {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "diagnosisCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "diagnosisReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageEligibilityRequestItemDiagnosis {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub diagnosis: Option<CoverageEligibilityRequestItemDiagnosisDiagnosis>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageEligibilityRequestSupportingInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub information: Reference,
    #[serde(rename = "appliesToAll", skip_serializing_if = "Option::is_none")]
    pub applies_to_all: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageEligibilityRequestInsurance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focal: Option<Boolean>,
    pub coverage: Reference,
    #[serde(rename = "businessArrangement", skip_serializing_if = "Option::is_none")]
    pub business_arrangement: Option<String>,
}

/// Choice of types for the serviced[x] field in CoverageEligibilityRequest
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityRequestServiced {
    /// Variant accepting the Date type.
    #[serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "servicedPeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub serviced: Option<CoverageEligibilityRequestServiced>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurance: Option<Vec<CoverageEligibilityRequestInsurance>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<CoverageEligibilityRequestItem>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageEligibilityRequestItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "supportingInfoSequence", skip_serializing_if = "Option::is_none")]
    pub supporting_info_sequence: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService", skip_serializing_if = "Option::is_none")]
    pub product_or_service: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice", skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facility: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<Vec<CoverageEligibilityRequestItemDiagnosis>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageEligibilityResponseInsurance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub coverage: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inforce: Option<Boolean>,
    #[serde(rename = "benefitPeriod", skip_serializing_if = "Option::is_none")]
    pub benefit_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<CoverageEligibilityResponseInsuranceItem>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageEligibilityResponseInsuranceItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
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
    #[serde(rename = "authorizationRequired", skip_serializing_if = "Option::is_none")]
    pub authorization_required: Option<Boolean>,
    #[serde(rename = "authorizationSupporting", skip_serializing_if = "Option::is_none")]
    pub authorization_supporting: Option<Vec<CodeableConcept>>,
    #[serde(rename = "authorizationUrl", skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<Uri>,
}

/// Choice of types for the allowed[x] field in CoverageEligibilityResponseInsuranceItemBenefit
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityResponseInsuranceItemBenefitAllowed {
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

/// Choice of types for the used[x] field in CoverageEligibilityResponseInsuranceItemBenefit
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityResponseInsuranceItemBenefitUsed {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageEligibilityResponseInsuranceItemBenefit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(flatten)]
    pub allowed: Option<CoverageEligibilityResponseInsuranceItemBenefitAllowed>,
    #[serde(flatten)]
    pub used: Option<CoverageEligibilityResponseInsuranceItemBenefitUsed>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageEligibilityResponseError {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
}

/// Choice of types for the serviced[x] field in CoverageEligibilityResponse
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityResponseServiced {
    /// Variant accepting the Date type.
    #[serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "servicedPeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub serviced: Option<CoverageEligibilityResponseServiced>,
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
    #[serde(rename = "preAuthRef", skip_serializing_if = "Option::is_none")]
    pub pre_auth_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Vec<CoverageEligibilityResponseError>>,
}


/// Choice of types for the identified[x] field in DetectedIssue
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DetectedIssueIdentified {
    /// Variant accepting the DateTime type.
    #[serde(rename = "identifiedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "identifiedPeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub identified: Option<DetectedIssueIdentified>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicated: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<DetectedIssueEvidence>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mitigation: Option<Vec<DetectedIssueMitigation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectedIssueEvidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectedIssueMitigation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceProperty {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "valueQuantity", skip_serializing_if = "Option::is_none")]
    pub value_quantity: Option<Vec<Quantity>>,
    #[serde(rename = "valueCode", skip_serializing_if = "Option::is_none")]
    pub value_code: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "modelNumber", skip_serializing_if = "Option::is_none")]
    pub model_number: Option<String>,
    #[serde(rename = "partNumber", skip_serializing_if = "Option::is_none")]
    pub part_number: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specialization: Option<Vec<DeviceSpecialization>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<Vec<DeviceVersion>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property: Option<Vec<DeviceProperty>>,
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceUdiCarrier {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceSpecialization {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "systemType")]
    pub system_type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceDeviceName {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: Code,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceVersion {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<Identifier>,
    pub value: String,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceDefinitionCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Vec<CodeableConcept>>,
}

/// Choice of types for the manufacturer[x] field in DeviceDefinition
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceDefinitionManufacturer {
    /// Variant accepting the String type.
    #[serde(rename = "manufacturerString")]
    String(String),
    /// Variant accepting the Reference type.
    #[serde(rename = "manufacturerReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(flatten)]
    pub manufacturer: Option<DeviceDefinitionManufacturer>,
    #[serde(rename = "deviceName", skip_serializing_if = "Option::is_none")]
    pub device_name: Option<Vec<DeviceDefinitionDeviceName>>,
    #[serde(rename = "modelNumber", skip_serializing_if = "Option::is_none")]
    pub model_number: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specialization: Option<Vec<DeviceDefinitionSpecialization>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<Vec<String>>,
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
    pub property: Option<Vec<DeviceDefinitionProperty>>,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceDefinitionProperty {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "valueQuantity", skip_serializing_if = "Option::is_none")]
    pub value_quantity: Option<Vec<Quantity>>,
    #[serde(rename = "valueCode", skip_serializing_if = "Option::is_none")]
    pub value_code: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceDefinitionMaterial {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub substance: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternate: Option<Boolean>,
    #[serde(rename = "allergenicIndicator", skip_serializing_if = "Option::is_none")]
    pub allergenic_indicator: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceDefinitionUdiDeviceIdentifier {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "deviceIdentifier")]
    pub device_identifier: String,
    pub issuer: Uri,
    pub jurisdiction: Uri,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceDefinitionDeviceName {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: Code,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceDefinitionSpecialization {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "systemType")]
    pub system_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceMetricCalibration {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<Instant>,
}


/// Choice of types for the code[x] field in DeviceRequest
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceRequestCode {
    /// Variant accepting the Reference type.
    #[serde(rename = "codeReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "codeCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

/// Choice of types for the occurrence[x] field in DeviceRequest
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceRequestOccurrence {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub code: Option<DeviceRequestCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<DeviceRequestParameter>>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(flatten)]
    pub occurrence: Option<DeviceRequestOccurrence>,
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

/// Choice of types for the value[x] field in DeviceRequestParameter
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceRequestParameterValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceRequestParameter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(flatten)]
    pub value: Option<DeviceRequestParameterValue>,
}


/// Choice of types for the timing[x] field in DeviceUseStatement
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceUseStatementTiming {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub timing: Option<DeviceUseStatementTiming>,
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticReportMedia {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub link: Reference,
}

/// Choice of types for the effective[x] field in DiagnosticReport
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticReportEffective {
    /// Variant accepting the DateTime type.
    #[serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "effectivePeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub effective: Option<DiagnosticReportEffective>,
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
    pub conclusion: Option<String>,
    #[serde(rename = "conclusionCode", skip_serializing_if = "Option::is_none")]
    pub conclusion_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "presentedForm", skip_serializing_if = "Option::is_none")]
    pub presented_form: Option<Vec<Attachment>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentManifestRelated {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentReferenceContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentReferenceContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub attachment: Attachment,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<Coding>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentReferenceRelatesTo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub target: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "securityLabel", skip_serializing_if = "Option::is_none")]
    pub security_label: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<DocumentReferenceContent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<DocumentReferenceContext>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectEvidenceSynthesisCertaintyCertaintySubcomponent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "resultsByExposure", skip_serializing_if = "Option::is_none")]
    pub results_by_exposure: Option<Vec<EffectEvidenceSynthesisResultsByExposure>>,
    #[serde(rename = "effectEstimate", skip_serializing_if = "Option::is_none")]
    pub effect_estimate: Option<Vec<EffectEvidenceSynthesisEffectEstimate>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certainty: Option<Vec<EffectEvidenceSynthesisCertainty>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectEvidenceSynthesisSampleSize {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "numberOfStudies", skip_serializing_if = "Option::is_none")]
    pub number_of_studies: Option<Integer>,
    #[serde(rename = "numberOfParticipants", skip_serializing_if = "Option::is_none")]
    pub number_of_participants: Option<Integer>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectEvidenceSynthesisEffectEstimate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "variantState", skip_serializing_if = "Option::is_none")]
    pub variant_state: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Decimal>,
    #[serde(rename = "unitOfMeasure", skip_serializing_if = "Option::is_none")]
    pub unit_of_measure: Option<CodeableConcept>,
    #[serde(rename = "precisionEstimate", skip_serializing_if = "Option::is_none")]
    pub precision_estimate: Option<Vec<EffectEvidenceSynthesisEffectEstimatePrecisionEstimate>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectEvidenceSynthesisEffectEstimatePrecisionEstimate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Decimal>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectEvidenceSynthesisResultsByExposure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "exposureState", skip_serializing_if = "Option::is_none")]
    pub exposure_state: Option<Code>,
    #[serde(rename = "variantState", skip_serializing_if = "Option::is_none")]
    pub variant_state: Option<CodeableConcept>,
    #[serde(rename = "riskEvidenceSynthesis")]
    pub risk_evidence_synthesis: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectEvidenceSynthesisCertainty {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "certaintySubcomponent", skip_serializing_if = "Option::is_none")]
    pub certainty_subcomponent: Option<Vec<EffectEvidenceSynthesisCertaintyCertaintySubcomponent>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub appointment: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<Duration>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<Vec<EncounterDiagnosis>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hospitalization: Option<EncounterHospitalization>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Vec<EncounterLocation>>,
    #[serde(rename = "serviceProvider", skip_serializing_if = "Option::is_none")]
    pub service_provider: Option<Reference>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncounterClassHistory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub class: Coding,
    pub period: Period,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncounterStatusHistory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: Code,
    pub period: Period,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncounterParticipant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub individual: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncounterDiagnosis {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub condition: Reference,
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub r#use: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<PositiveInt>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncounterHospitalization {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncounterLocation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub location: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(rename = "physicalType", skip_serializing_if = "Option::is_none")]
    pub physical_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeOfCareStatusHistory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: Code,
    pub period: Period,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<Vec<EpisodeOfCareDiagnosis>>,
    pub patient: Reference,
    #[serde(rename = "managingOrganization", skip_serializing_if = "Option::is_none")]
    pub managing_organization: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(rename = "referralRequest", skip_serializing_if = "Option::is_none")]
    pub referral_request: Option<Vec<Reference>>,
    #[serde(rename = "careManager", skip_serializing_if = "Option::is_none")]
    pub care_manager: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeOfCareDiagnosis {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub condition: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<PositiveInt>,
}


/// Choice of types for the subject[x] field in EventDefinition
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventDefinitionSubject {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub subject: Option<EventDefinitionSubject>,
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}

/// Choice of types for the definition[x] field in EvidenceVariableCharacteristic
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceVariableCharacteristicDefinition {
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

/// Choice of types for the participantEffective[x] field in EvidenceVariableCharacteristic
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceVariableCharacteristicParticipantEffective {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceVariableCharacteristic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub definition: Option<EvidenceVariableCharacteristicDefinition>,
    #[serde(rename = "usageContext", skip_serializing_if = "Option::is_none")]
    pub usage_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Boolean>,
    #[serde(flatten)]
    pub participant_effective: Option<EvidenceVariableCharacteristicParticipantEffective>,
    #[serde(rename = "timeFromStart", skip_serializing_if = "Option::is_none")]
    pub time_from_start: Option<Duration>,
    #[serde(rename = "groupMeasure", skip_serializing_if = "Option::is_none")]
    pub group_measure: Option<Code>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExampleScenarioProcessStep {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<Vec<ExampleScenarioProcess>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pause: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<ExampleScenarioProcessStepOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternative: Option<Vec<ExampleScenarioProcessStepAlternative>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExampleScenarioActor {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "actorId")]
    pub actor_id: String,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExampleScenarioInstanceContainedInstance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "resourceId")]
    pub resource_id: String,
    #[serde(rename = "versionId", skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<Vec<ExampleScenarioInstance>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<Vec<ExampleScenarioProcess>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow: Option<Vec<Canonical>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExampleScenarioInstance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "resourceId")]
    pub resource_id: String,
    #[serde(rename = "resourceType")]
    pub resource_type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<Vec<ExampleScenarioInstanceVersion>>,
    #[serde(rename = "containedInstance", skip_serializing_if = "Option::is_none")]
    pub contained_instance: Option<Vec<ExampleScenarioInstanceContainedInstance>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExampleScenarioInstanceVersion {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "versionId")]
    pub version_id: String,
    pub description: Markdown,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExampleScenarioProcessStepOperation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub number: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initiator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "initiatorActive", skip_serializing_if = "Option::is_none")]
    pub initiator_active: Option<Boolean>,
    #[serde(rename = "receiverActive", skip_serializing_if = "Option::is_none")]
    pub receiver_active: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<ExampleScenarioInstanceContainedInstance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<ExampleScenarioInstanceContainedInstance>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExampleScenarioProcessStepAlternative {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<Vec<ExampleScenarioProcessStep>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExampleScenarioProcess {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "preConditions", skip_serializing_if = "Option::is_none")]
    pub pre_conditions: Option<Markdown>,
    #[serde(rename = "postConditions", skip_serializing_if = "Option::is_none")]
    pub post_conditions: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<Vec<ExampleScenarioProcessStep>>,
}


/// Choice of types for the allowed[x] field in ExplanationOfBenefitBenefitBalanceFinancial
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitBenefitBalanceFinancialAllowed {
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

/// Choice of types for the used[x] field in ExplanationOfBenefitBenefitBalanceFinancial
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitBenefitBalanceFinancialUsed {
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "usedUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Money type.
    #[serde(rename = "usedMoney")]
    Money(Money),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitBenefitBalanceFinancial {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(flatten)]
    pub allowed: Option<ExplanationOfBenefitBenefitBalanceFinancialAllowed>,
    #[serde(flatten)]
    pub used: Option<ExplanationOfBenefitBenefitBalanceFinancialUsed>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitTotal {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub amount: Money,
}

/// Choice of types for the diagnosis[x] field in ExplanationOfBenefitDiagnosis
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitDiagnosisDiagnosis {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "diagnosisCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "diagnosisReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitDiagnosis {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(flatten)]
    pub diagnosis: Option<ExplanationOfBenefitDiagnosisDiagnosis>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "onAdmission", skip_serializing_if = "Option::is_none")]
    pub on_admission: Option<CodeableConcept>,
    #[serde(rename = "packageCode", skip_serializing_if = "Option::is_none")]
    pub package_code: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitItemAdjudication {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Decimal>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitPayment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjustment: Option<Money>,
    #[serde(rename = "adjustmentReason", skip_serializing_if = "Option::is_none")]
    pub adjustment_reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<Date>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitProcessNote {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<PositiveInt>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitRelated {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Identifier>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitBenefitBalance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
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
}

/// Choice of types for the serviced[x] field in ExplanationOfBenefitAddItem
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitAddItemServiced {
    /// Variant accepting the Date type.
    #[serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "servicedPeriod")]
    Period(Period),
}

/// Choice of types for the location[x] field in ExplanationOfBenefitAddItem
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitAddItemLocation {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitAddItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemSequence", skip_serializing_if = "Option::is_none")]
    pub item_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "detailSequence", skip_serializing_if = "Option::is_none")]
    pub detail_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "subDetailSequence", skip_serializing_if = "Option::is_none")]
    pub sub_detail_sequence: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<Vec<Reference>>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode", skip_serializing_if = "Option::is_none")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(flatten)]
    pub serviced: Option<ExplanationOfBenefitAddItemServiced>,
    #[serde(flatten)]
    pub location: Option<ExplanationOfBenefitAddItemLocation>,
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
    #[serde(rename = "noteNumber", skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<ExplanationOfBenefitAddItemDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitAddItemDetailSubDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice", skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(rename = "noteNumber", skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitItemDetailSubDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revenue: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode", skip_serializing_if = "Option::is_none")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice", skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi: Option<Vec<Reference>>,
    #[serde(rename = "noteNumber", skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
}

/// Choice of types for the procedure[x] field in ExplanationOfBenefitProcedure
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitProcedureProcedure {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "procedureCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "procedureReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitProcedure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(flatten)]
    pub procedure: Option<ExplanationOfBenefitProcedureProcedure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub prescription: Option<Reference>,
    #[serde(rename = "originalPrescription", skip_serializing_if = "Option::is_none")]
    pub original_prescription: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payee: Option<ExplanationOfBenefitPayee>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referral: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facility: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim: Option<Reference>,
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
    #[serde(rename = "supportingInfo", skip_serializing_if = "Option::is_none")]
    pub supporting_info: Option<Vec<ExplanationOfBenefitSupportingInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<Vec<ExplanationOfBenefitDiagnosis>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure: Option<Vec<ExplanationOfBenefitProcedure>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precedence: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurance: Option<Vec<ExplanationOfBenefitInsurance>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accident: Option<ExplanationOfBenefitAccident>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<ExplanationOfBenefitItem>>,
    #[serde(rename = "addItem", skip_serializing_if = "Option::is_none")]
    pub add_item: Option<Vec<ExplanationOfBenefitAddItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<Vec<ExplanationOfBenefitTotal>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment: Option<ExplanationOfBenefitPayment>,
    #[serde(rename = "formCode", skip_serializing_if = "Option::is_none")]
    pub form_code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form: Option<Attachment>,
    #[serde(rename = "processNote", skip_serializing_if = "Option::is_none")]
    pub process_note: Option<Vec<ExplanationOfBenefitProcessNote>>,
    #[serde(rename = "benefitPeriod", skip_serializing_if = "Option::is_none")]
    pub benefit_period: Option<Period>,
    #[serde(rename = "benefitBalance", skip_serializing_if = "Option::is_none")]
    pub benefit_balance: Option<Vec<ExplanationOfBenefitBenefitBalance>>,
}

/// Choice of types for the location[x] field in ExplanationOfBenefitAccident
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitAccidentLocation {
    /// Variant accepting the Address type.
    #[serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[serde(rename = "locationReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitAccident {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<Date>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(flatten)]
    pub location: Option<ExplanationOfBenefitAccidentLocation>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitInsurance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub focal: Boolean,
    pub coverage: Reference,
    #[serde(rename = "preAuthRef", skip_serializing_if = "Option::is_none")]
    pub pre_auth_ref: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitCareTeam {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub provider: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsible: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualification: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitItemDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revenue: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode", skip_serializing_if = "Option::is_none")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice", skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi: Option<Vec<Reference>>,
    #[serde(rename = "noteNumber", skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    #[serde(rename = "subDetail", skip_serializing_if = "Option::is_none")]
    pub sub_detail: Option<Vec<ExplanationOfBenefitItemDetailSubDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitAddItemDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice", skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(rename = "noteNumber", skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    #[serde(rename = "subDetail", skip_serializing_if = "Option::is_none")]
    pub sub_detail: Option<Vec<ExplanationOfBenefitAddItemDetailSubDetail>>,
}

/// Choice of types for the serviced[x] field in ExplanationOfBenefitItem
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitItemServiced {
    /// Variant accepting the Date type.
    #[serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "servicedPeriod")]
    Period(Period),
}

/// Choice of types for the location[x] field in ExplanationOfBenefitItem
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitItemLocation {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode", skip_serializing_if = "Option::is_none")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(flatten)]
    pub serviced: Option<ExplanationOfBenefitItemServiced>,
    #[serde(flatten)]
    pub location: Option<ExplanationOfBenefitItemLocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice", skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi: Option<Vec<Reference>>,
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
    pub detail: Option<Vec<ExplanationOfBenefitItemDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitPayee {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<Reference>,
}

/// Choice of types for the timing[x] field in ExplanationOfBenefitSupportingInfo
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitSupportingInfoTiming {
    /// Variant accepting the Date type.
    #[serde(rename = "timingDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "timingPeriod")]
    Period(Period),
}

/// Choice of types for the value[x] field in ExplanationOfBenefitSupportingInfo
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitSupportingInfoValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitSupportingInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub category: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(flatten)]
    pub timing: Option<ExplanationOfBenefitSupportingInfoTiming>,
    #[serde(flatten)]
    pub value: Option<ExplanationOfBenefitSupportingInfoValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Coding>,
}


/// Choice of types for the born[x] field in FamilyMemberHistory
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryBorn {
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
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryAge {
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
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryDeceased {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub born: Option<FamilyMemberHistoryBorn>,
    #[serde(flatten)]
    pub age: Option<FamilyMemberHistoryAge>,
    #[serde(rename = "estimatedAge", skip_serializing_if = "Option::is_none")]
    pub estimated_age: Option<Boolean>,
    #[serde(flatten)]
    pub deceased: Option<FamilyMemberHistoryDeceased>,
    #[serde(rename = "reasonCode", skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference", skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Vec<FamilyMemberHistoryCondition>>,
}

/// Choice of types for the onset[x] field in FamilyMemberHistoryCondition
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryConditionOnset {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyMemberHistoryCondition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<CodeableConcept>,
    #[serde(rename = "contributedToDeath", skip_serializing_if = "Option::is_none")]
    pub contributed_to_death: Option<Boolean>,
    #[serde(flatten)]
    pub onset: Option<FamilyMemberHistoryConditionOnset>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoalStart {
    /// Variant accepting the Date type.
    #[serde(rename = "startDate")]
    Date(Date),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "startCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub start: Option<GoalStart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<GoalTarget>>,
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

/// Choice of types for the detail[x] field in GoalTarget
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoalTargetDetail {
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

/// Choice of types for the due[x] field in GoalTarget
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoalTargetDue {
    /// Variant accepting the Date type.
    #[serde(rename = "dueDate")]
    Date(Date),
    /// Variant accepting the Duration type.
    #[serde(rename = "dueDuration")]
    Duration(Duration),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measure: Option<CodeableConcept>,
    #[serde(flatten)]
    pub detail: Option<GoalTargetDetail>,
    #[serde(flatten)]
    pub due: Option<GoalTargetDue>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphDefinitionLinkTargetCompartment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "use")]
    pub r#use: Code,
    pub code: Code,
    pub rule: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphDefinitionLink {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(rename = "sliceName", skip_serializing_if = "Option::is_none")]
    pub slice_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<GraphDefinitionLinkTarget>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphDefinitionLinkTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Canonical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compartment: Option<Vec<GraphDefinitionLinkTargetCompartment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<Vec<GraphDefinitionLink>>,
}


/// Choice of types for the value[x] field in GroupCharacteristic
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GroupCharacteristicValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupCharacteristic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(flatten)]
    pub value: Option<GroupCharacteristicValue>,
    pub exclude: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member: Option<Vec<GroupMember>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupMember {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub entity: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inactive: Option<Boolean>,
}


/// Choice of types for the module[x] field in GuidanceResponse
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GuidanceResponseModule {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub module: Option<GuidanceResponseModule>,
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthcareServiceEligibility {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "notAvailable", skip_serializing_if = "Option::is_none")]
    pub not_available: Option<Vec<HealthcareServiceNotAvailable>>,
    #[serde(rename = "availabilityExceptions", skip_serializing_if = "Option::is_none")]
    pub availability_exceptions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthcareServiceNotAvailable {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub during: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthcareServiceAvailableTime {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "daysOfWeek", skip_serializing_if = "Option::is_none")]
    pub days_of_week: Option<Vec<Code>>,
    #[serde(rename = "allDay", skip_serializing_if = "Option::is_none")]
    pub all_day: Option<Boolean>,
    #[serde(rename = "availableStartTime", skip_serializing_if = "Option::is_none")]
    pub available_start_time: Option<Time>,
    #[serde(rename = "availableEndTime", skip_serializing_if = "Option::is_none")]
    pub available_end_time: Option<Time>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImagingStudySeriesInstance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub uid: Id,
    #[serde(rename = "sopClass")]
    pub sop_class: Coding,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<UnsignedInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImagingStudySeries {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub uid: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<UnsignedInt>,
    pub modality: Coding,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "numberOfInstances", skip_serializing_if = "Option::is_none")]
    pub number_of_instances: Option<UnsignedInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<Reference>>,
    #[serde(rename = "bodySite", skip_serializing_if = "Option::is_none")]
    pub body_site: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub laterality: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specimen: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<ImagingStudySeriesPerformer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<Vec<ImagingStudySeriesInstance>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImagingStudySeriesPerformer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmunizationEducation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "documentType", skip_serializing_if = "Option::is_none")]
    pub document_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Uri>,
    #[serde(rename = "publicationDate", skip_serializing_if = "Option::is_none")]
    pub publication_date: Option<DateTime>,
    #[serde(rename = "presentationDate", skip_serializing_if = "Option::is_none")]
    pub presentation_date: Option<DateTime>,
}

/// Choice of types for the occurrence[x] field in Immunization
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationOccurrence {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the String type.
    #[serde(rename = "occurrenceString")]
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub occurrence: Option<ImmunizationOccurrence>,
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
    #[serde(rename = "programEligibility", skip_serializing_if = "Option::is_none")]
    pub program_eligibility: Option<Vec<CodeableConcept>>,
    #[serde(rename = "fundingSource", skip_serializing_if = "Option::is_none")]
    pub funding_source: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reaction: Option<Vec<ImmunizationReaction>>,
    #[serde(rename = "protocolApplied", skip_serializing_if = "Option::is_none")]
    pub protocol_applied: Option<Vec<ImmunizationProtocolApplied>>,
}

/// Choice of types for the doseNumber[x] field in ImmunizationProtocolApplied
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationProtocolAppliedDoseNumber {
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "doseNumberPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "doseNumberString")]
    String(String),
}

/// Choice of types for the seriesDoses[x] field in ImmunizationProtocolApplied
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationProtocolAppliedSeriesDoses {
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "seriesDosesPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "seriesDosesString")]
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmunizationProtocolApplied {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<Reference>,
    #[serde(rename = "targetDisease", skip_serializing_if = "Option::is_none")]
    pub target_disease: Option<Vec<CodeableConcept>>,
    #[serde(flatten)]
    pub dose_number: Option<ImmunizationProtocolAppliedDoseNumber>,
    #[serde(flatten)]
    pub series_doses: Option<ImmunizationProtocolAppliedSeriesDoses>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmunizationReaction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmunizationPerformer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}


/// Choice of types for the doseNumber[x] field in ImmunizationEvaluation
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationEvaluationDoseNumber {
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "doseNumberPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "doseNumberString")]
    String(String),
}

/// Choice of types for the seriesDoses[x] field in ImmunizationEvaluation
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationEvaluationSeriesDoses {
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "seriesDosesPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "seriesDosesString")]
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub dose_number: Option<ImmunizationEvaluationDoseNumber>,
    #[serde(flatten)]
    pub series_doses: Option<ImmunizationEvaluationSeriesDoses>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmunizationRecommendationRecommendationDateCriterion {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub value: DateTime,
}

/// Choice of types for the doseNumber[x] field in ImmunizationRecommendationRecommendation
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationRecommendationRecommendationDoseNumber {
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "doseNumberPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "doseNumberString")]
    String(String),
}

/// Choice of types for the seriesDoses[x] field in ImmunizationRecommendationRecommendation
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationRecommendationRecommendationSeriesDoses {
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "seriesDosesPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "seriesDosesString")]
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmunizationRecommendationRecommendation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    #[serde(flatten)]
    pub dose_number: Option<ImmunizationRecommendationRecommendationDoseNumber>,
    #[serde(flatten)]
    pub series_doses: Option<ImmunizationRecommendationRecommendationSeriesDoses>,
    #[serde(rename = "supportingImmunization", skip_serializing_if = "Option::is_none")]
    pub supporting_immunization: Option<Vec<Reference>>,
    #[serde(rename = "supportingPatientInformation", skip_serializing_if = "Option::is_none")]
    pub supporting_patient_information: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideDependsOn {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub uri: Canonical,
    #[serde(rename = "packageId", skip_serializing_if = "Option::is_none")]
    pub package_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grouping: Option<Vec<ImplementationGuideDefinitionGrouping>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Vec<ImplementationGuideDefinitionResource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<ImplementationGuideDefinitionPage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<ImplementationGuideDefinitionParameter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<Vec<ImplementationGuideDefinitionTemplate>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideGlobal {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub profile: Canonical,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideDefinitionTemplate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// Choice of types for the example[x] field in ImplementationGuideManifestResource
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImplementationGuideManifestResourceExample {
    /// Variant accepting the Boolean type.
    #[serde(rename = "exampleBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "exampleCanonical")]
    Canonical(Canonical),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideManifestResource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Reference,
    #[serde(flatten)]
    pub example: Option<ImplementationGuideManifestResourceExample>,
    #[serde(rename = "relativePath", skip_serializing_if = "Option::is_none")]
    pub relative_path: Option<Url>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideDefinitionParameter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global: Option<Vec<ImplementationGuideGlobal>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<ImplementationGuideDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<ImplementationGuideManifest>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideManifest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rendering: Option<Url>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Vec<ImplementationGuideManifestResource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<Vec<ImplementationGuideManifestPage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideManifestPage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Vec<String>>,
}

/// Choice of types for the example[x] field in ImplementationGuideDefinitionResource
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImplementationGuideDefinitionResourceExample {
    /// Variant accepting the Boolean type.
    #[serde(rename = "exampleBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "exampleCanonical")]
    Canonical(Canonical),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideDefinitionResource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Reference,
    #[serde(rename = "fhirVersion", skip_serializing_if = "Option::is_none")]
    pub fhir_version: Option<Vec<Code>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub example: Option<ImplementationGuideDefinitionResourceExample>,
    #[serde(rename = "groupingId", skip_serializing_if = "Option::is_none")]
    pub grouping_id: Option<Id>,
}

/// Choice of types for the name[x] field in ImplementationGuideDefinitionPage
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImplementationGuideDefinitionPageName {
    /// Variant accepting the Url type.
    #[serde(rename = "nameUrl")]
    Url(Url),
    /// Variant accepting the Reference type.
    #[serde(rename = "nameReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideDefinitionPage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub name: Option<ImplementationGuideDefinitionPageName>,
    pub title: String,
    pub generation: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<Vec<ImplementationGuideDefinitionPage>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideDefinitionGrouping {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsurancePlanCoverageBenefitLimit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsurancePlanCoverage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub benefit: Option<Vec<InsurancePlanCoverageBenefit>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub endpoint: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Vec<InsurancePlanCoverage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<Vec<InsurancePlanPlan>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsurancePlanPlan {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "coverageArea", skip_serializing_if = "Option::is_none")]
    pub coverage_area: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<Vec<Reference>>,
    #[serde(rename = "generalCost", skip_serializing_if = "Option::is_none")]
    pub general_cost: Option<Vec<InsurancePlanPlanGeneralCost>>,
    #[serde(rename = "specificCost", skip_serializing_if = "Option::is_none")]
    pub specific_cost: Option<Vec<InsurancePlanPlanSpecificCost>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsurancePlanPlanSpecificCost {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub benefit: Option<Vec<InsurancePlanPlanSpecificCostBenefit>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsurancePlanPlanSpecificCostBenefit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<Vec<InsurancePlanPlanSpecificCostBenefitCost>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsurancePlanContact {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<HumanName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<Address>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsurancePlanCoverageBenefit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<Vec<InsurancePlanCoverageBenefitLimit>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsurancePlanPlanSpecificCostBenefitCost {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applicability: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualifiers: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsurancePlanPlanGeneralCost {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "groupSize", skip_serializing_if = "Option::is_none")]
    pub group_size: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvoiceParticipant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
    pub actor: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvoiceLineItemPriceComponent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Money>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub issuer: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Reference>,
    #[serde(rename = "lineItem", skip_serializing_if = "Option::is_none")]
    pub line_item: Option<Vec<InvoiceLineItem>>,
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

/// Choice of types for the chargeItem[x] field in InvoiceLineItem
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InvoiceLineItemChargeItem {
    /// Variant accepting the Reference type.
    #[serde(rename = "chargeItemReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "chargeItemCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvoiceLineItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<PositiveInt>,
    #[serde(flatten)]
    pub charge_item: Option<InvoiceLineItemChargeItem>,
    #[serde(rename = "priceComponent", skip_serializing_if = "Option::is_none")]
    pub price_component: Option<Vec<InvoiceLineItemPriceComponent>>,
}


/// Choice of types for the subject[x] field in Library
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LibrarySubject {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub subject: Option<LibrarySubject>,
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkageItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub resource: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "emptyReason", skip_serializing_if = "Option::is_none")]
    pub empty_reason: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flag: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    pub item: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocationPosition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub longitude: Decimal,
    pub latitude: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub altitude: Option<Decimal>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocationHoursOfOperation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "daysOfWeek", skip_serializing_if = "Option::is_none")]
    pub days_of_week: Option<Vec<Code>>,
    #[serde(rename = "allDay", skip_serializing_if = "Option::is_none")]
    pub all_day: Option<Boolean>,
    #[serde(rename = "openingTime", skip_serializing_if = "Option::is_none")]
    pub opening_time: Option<Time>,
    #[serde(rename = "closingTime", skip_serializing_if = "Option::is_none")]
    pub closing_time: Option<Time>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "managingOrganization", skip_serializing_if = "Option::is_none")]
    pub managing_organization: Option<Reference>,
    #[serde(rename = "partOf", skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Reference>,
    #[serde(rename = "hoursOfOperation", skip_serializing_if = "Option::is_none")]
    pub hours_of_operation: Option<Vec<LocationHoursOfOperation>>,
    #[serde(rename = "availabilityExceptions", skip_serializing_if = "Option::is_none")]
    pub availability_exceptions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureGroupPopulation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub criteria: Expression,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureGroupStratifierComponent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub criteria: Expression,
}

/// Choice of types for the subject[x] field in Measure
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeasureSubject {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub subject: Option<MeasureSubject>,
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
    #[serde(rename = "supplementalData", skip_serializing_if = "Option::is_none")]
    pub supplemental_data: Option<Vec<MeasureSupplementalData>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub population: Option<Vec<MeasureGroupPopulation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stratifier: Option<Vec<MeasureGroupStratifier>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureGroupStratifier {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub criteria: Option<Expression>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<Vec<MeasureGroupStratifierComponent>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureSupplementalData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub criteria: Expression,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureReportGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub population: Option<Vec<MeasureReportGroupPopulation>>,
    #[serde(rename = "measureScore", skip_serializing_if = "Option::is_none")]
    pub measure_score: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stratifier: Option<Vec<MeasureReportGroupStratifier>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureReportGroupPopulation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<Integer>,
    #[serde(rename = "subjectResults", skip_serializing_if = "Option::is_none")]
    pub subject_results: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureReportGroupStratifierStratum {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<Vec<MeasureReportGroupStratifierStratumComponent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub population: Option<Vec<MeasureReportGroupStratifierStratumPopulation>>,
    #[serde(rename = "measureScore", skip_serializing_if = "Option::is_none")]
    pub measure_score: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "evaluatedResource", skip_serializing_if = "Option::is_none")]
    pub evaluated_resource: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureReportGroupStratifierStratumComponent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub value: CodeableConcept,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureReportGroupStratifier {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stratum: Option<Vec<MeasureReportGroupStratifierStratum>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureReportGroupStratifierStratumPopulation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<Integer>,
    #[serde(rename = "subjectResults", skip_serializing_if = "Option::is_none")]
    pub subject_results: Option<Reference>,
}


/// Choice of types for the created[x] field in Media
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaCreated {
    /// Variant accepting the DateTime type.
    #[serde(rename = "createdDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "createdPeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub created: Option<MediaCreated>,
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch: Option<MedicationBatch>,
}

/// Choice of types for the item[x] field in MedicationIngredient
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationIngredientItem {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "itemReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationIngredient {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub item: Option<MedicationIngredientItem>,
    #[serde(rename = "isActive", skip_serializing_if = "Option::is_none")]
    pub is_active: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<Ratio>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationBatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "lotNumber", skip_serializing_if = "Option::is_none")]
    pub lot_number: Option<String>,
    #[serde(rename = "expirationDate", skip_serializing_if = "Option::is_none")]
    pub expiration_date: Option<DateTime>,
}


/// Choice of types for the medication[x] field in MedicationAdministration
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationAdministrationMedication {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "medicationReference")]
    Reference(Reference),
}

/// Choice of types for the effective[x] field in MedicationAdministration
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationAdministrationEffective {
    /// Variant accepting the DateTime type.
    #[serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "effectivePeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub medication: Option<MedicationAdministrationMedication>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Reference>,
    #[serde(rename = "supportingInformation", skip_serializing_if = "Option::is_none")]
    pub supporting_information: Option<Vec<Reference>>,
    #[serde(flatten)]
    pub effective: Option<MedicationAdministrationEffective>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<MedicationAdministrationPerformer>>,
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
    #[serde(rename = "eventHistory", skip_serializing_if = "Option::is_none")]
    pub event_history: Option<Vec<Reference>>,
}

/// Choice of types for the rate[x] field in MedicationAdministrationDosage
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationAdministrationDosageRate {
    /// Variant accepting the Ratio type.
    #[serde(rename = "rateRatio")]
    Ratio(Ratio),
    /// Variant accepting the Quantity type.
    #[serde(rename = "rateQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationAdministrationDosage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dose: Option<Quantity>,
    #[serde(flatten)]
    pub rate: Option<MedicationAdministrationDosageRate>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationAdministrationPerformer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}


/// Choice of types for the statusReason[x] field in MedicationDispense
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationDispenseStatusReason {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "statusReasonCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "statusReasonReference")]
    Reference(Reference),
}

/// Choice of types for the medication[x] field in MedicationDispense
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationDispenseMedication {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "medicationReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub status_reason: Option<MedicationDispenseStatusReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(flatten)]
    pub medication: Option<MedicationDispenseMedication>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Reference>,
    #[serde(rename = "supportingInformation", skip_serializing_if = "Option::is_none")]
    pub supporting_information: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<MedicationDispensePerformer>>,
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
    #[serde(rename = "detectedIssue", skip_serializing_if = "Option::is_none")]
    pub detected_issue: Option<Vec<Reference>>,
    #[serde(rename = "eventHistory", skip_serializing_if = "Option::is_none")]
    pub event_history: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationDispensePerformer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationDispenseSubstitution {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "wasSubstituted")]
    pub was_substituted: Boolean,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Vec<CodeableConcept>>,
    #[serde(rename = "responsibleParty", skip_serializing_if = "Option::is_none")]
    pub responsible_party: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "associatedMedication", skip_serializing_if = "Option::is_none")]
    pub associated_medication: Option<Vec<Reference>>,
    #[serde(rename = "productType", skip_serializing_if = "Option::is_none")]
    pub product_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monograph: Option<Vec<MedicationKnowledgeMonograph>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingredient: Option<Vec<MedicationKnowledgeIngredient>>,
    #[serde(rename = "preparationInstruction", skip_serializing_if = "Option::is_none")]
    pub preparation_instruction: Option<Markdown>,
    #[serde(rename = "intendedRoute", skip_serializing_if = "Option::is_none")]
    pub intended_route: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<Vec<MedicationKnowledgeCost>>,
    #[serde(rename = "monitoringProgram", skip_serializing_if = "Option::is_none")]
    pub monitoring_program: Option<Vec<MedicationKnowledgeMonitoringProgram>>,
    #[serde(rename = "administrationGuidelines", skip_serializing_if = "Option::is_none")]
    pub administration_guidelines: Option<Vec<MedicationKnowledgeAdministrationGuidelines>>,
    #[serde(rename = "medicineClassification", skip_serializing_if = "Option::is_none")]
    pub medicine_classification: Option<Vec<MedicationKnowledgeMedicineClassification>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packaging: Option<MedicationKnowledgePackaging>,
    #[serde(rename = "drugCharacteristic", skip_serializing_if = "Option::is_none")]
    pub drug_characteristic: Option<Vec<MedicationKnowledgeDrugCharacteristic>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contraindication: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regulatory: Option<Vec<MedicationKnowledgeRegulatory>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kinetics: Option<Vec<MedicationKnowledgeKinetics>>,
}

/// Choice of types for the characteristic[x] field in MedicationKnowledgeAdministrationGuidelinesPatientCharacteristics
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeAdministrationGuidelinesPatientCharacteristicsCharacteristic {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "characteristicCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[serde(rename = "characteristicQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeAdministrationGuidelinesPatientCharacteristics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub characteristic: Option<MedicationKnowledgeAdministrationGuidelinesPatientCharacteristicsCharacteristic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeKinetics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "areaUnderCurve", skip_serializing_if = "Option::is_none")]
    pub area_under_curve: Option<Vec<Quantity>>,
    #[serde(rename = "lethalDose50", skip_serializing_if = "Option::is_none")]
    pub lethal_dose50: Option<Vec<Quantity>>,
    #[serde(rename = "halfLifePeriod", skip_serializing_if = "Option::is_none")]
    pub half_life_period: Option<Duration>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeRegulatory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "regulatoryAuthority")]
    pub regulatory_authority: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substitution: Option<Vec<MedicationKnowledgeRegulatorySubstitution>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Vec<MedicationKnowledgeRegulatorySchedule>>,
    #[serde(rename = "maxDispense", skip_serializing_if = "Option::is_none")]
    pub max_dispense: Option<MedicationKnowledgeRegulatoryMaxDispense>,
}

/// Choice of types for the item[x] field in MedicationKnowledgeIngredient
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeIngredientItem {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "itemReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeIngredient {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub item: Option<MedicationKnowledgeIngredientItem>,
    #[serde(rename = "isActive", skip_serializing_if = "Option::is_none")]
    pub is_active: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<Ratio>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeAdministrationGuidelinesDosage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dosage: Option<Vec<Dosage>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeRegulatorySubstitution {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub allowed: Boolean,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeCost {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub cost: Money,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeMonitoringProgram {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeMedicineClassification {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeRelatedMedicationKnowledge {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeRegulatorySchedule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub schedule: CodeableConcept,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgePackaging {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
}

/// Choice of types for the value[x] field in MedicationKnowledgeDrugCharacteristic
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeDrugCharacteristicValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeDrugCharacteristic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(flatten)]
    pub value: Option<MedicationKnowledgeDrugCharacteristicValue>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeMonograph {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Reference>,
}

/// Choice of types for the indication[x] field in MedicationKnowledgeAdministrationGuidelines
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeAdministrationGuidelinesIndication {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "indicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "indicationReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeAdministrationGuidelines {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dosage: Option<Vec<MedicationKnowledgeAdministrationGuidelinesDosage>>,
    #[serde(flatten)]
    pub indication: Option<MedicationKnowledgeAdministrationGuidelinesIndication>,
    #[serde(rename = "patientCharacteristics", skip_serializing_if = "Option::is_none")]
    pub patient_characteristics: Option<Vec<MedicationKnowledgeAdministrationGuidelinesPatientCharacteristics>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeRegulatoryMaxDispense {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub quantity: Quantity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Duration>,
}


/// Choice of types for the reported[x] field in MedicationRequest
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationRequestReported {
    /// Variant accepting the Boolean type.
    #[serde(rename = "reportedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Reference type.
    #[serde(rename = "reportedReference")]
    Reference(Reference),
}

/// Choice of types for the medication[x] field in MedicationRequest
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationRequestMedication {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "medicationReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub reported: Option<MedicationRequestReported>,
    #[serde(flatten)]
    pub medication: Option<MedicationRequestMedication>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substitution: Option<MedicationRequestSubstitution>,
    #[serde(rename = "priorPrescription", skip_serializing_if = "Option::is_none")]
    pub prior_prescription: Option<Reference>,
    #[serde(rename = "detectedIssue", skip_serializing_if = "Option::is_none")]
    pub detected_issue: Option<Vec<Reference>>,
    #[serde(rename = "eventHistory", skip_serializing_if = "Option::is_none")]
    pub event_history: Option<Vec<Reference>>,
}

/// Choice of types for the allowed[x] field in MedicationRequestSubstitution
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationRequestSubstitutionAllowed {
    /// Variant accepting the Boolean type.
    #[serde(rename = "allowedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "allowedCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationRequestSubstitution {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub allowed: Option<MedicationRequestSubstitutionAllowed>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationRequestDispenseRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "initialFill", skip_serializing_if = "Option::is_none")]
    pub initial_fill: Option<MedicationRequestDispenseRequestInitialFill>,
    #[serde(rename = "dispenseInterval", skip_serializing_if = "Option::is_none")]
    pub dispense_interval: Option<Duration>,
    #[serde(rename = "validityPeriod", skip_serializing_if = "Option::is_none")]
    pub validity_period: Option<Period>,
    #[serde(rename = "numberOfRepeatsAllowed", skip_serializing_if = "Option::is_none")]
    pub number_of_repeats_allowed: Option<UnsignedInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "expectedSupplyDuration", skip_serializing_if = "Option::is_none")]
    pub expected_supply_duration: Option<Duration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationRequestDispenseRequestInitialFill {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<Duration>,
}


/// Choice of types for the medication[x] field in MedicationStatement
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationStatementMedication {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "medicationReference")]
    Reference(Reference),
}

/// Choice of types for the effective[x] field in MedicationStatement
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationStatementEffective {
    /// Variant accepting the DateTime type.
    #[serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "effectivePeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub medication: Option<MedicationStatementMedication>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Reference>,
    #[serde(flatten)]
    pub effective: Option<MedicationStatementEffective>,
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductName {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "productName")]
    pub product_name: String,
    #[serde(rename = "namePart", skip_serializing_if = "Option::is_none")]
    pub name_part: Option<Vec<MedicinalProductNameNamePart>>,
    #[serde(rename = "countryLanguage", skip_serializing_if = "Option::is_none")]
    pub country_language: Option<Vec<MedicinalProductNameCountryLanguage>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "crossReference", skip_serializing_if = "Option::is_none")]
    pub cross_reference: Option<Vec<Identifier>>,
    #[serde(rename = "manufacturingBusinessOperation", skip_serializing_if = "Option::is_none")]
    pub manufacturing_business_operation: Option<Vec<MedicinalProductManufacturingBusinessOperation>>,
    #[serde(rename = "specialDesignation", skip_serializing_if = "Option::is_none")]
    pub special_designation: Option<Vec<MedicinalProductSpecialDesignation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductNameCountryLanguage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub country: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<CodeableConcept>,
    pub language: CodeableConcept,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductManufacturingBusinessOperation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
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
}

/// Choice of types for the indication[x] field in MedicinalProductSpecialDesignation
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductSpecialDesignationIndication {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "indicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "indicationReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductSpecialDesignation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "intendedUse", skip_serializing_if = "Option::is_none")]
    pub intended_use: Option<CodeableConcept>,
    #[serde(flatten)]
    pub indication: Option<MedicinalProductSpecialDesignationIndication>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub species: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductNameNamePart {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub part: String,
    #[serde(rename = "type")]
    pub r#type: Coding,
}


/// Choice of types for the date[x] field in MedicinalProductAuthorizationProcedure
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductAuthorizationProcedureDate {
    /// Variant accepting the Period type.
    #[serde(rename = "datePeriod")]
    Period(Period),
    /// Variant accepting the DateTime type.
    #[serde(rename = "dateDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductAuthorizationProcedure {
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
    #[serde(flatten)]
    pub date: Option<MedicinalProductAuthorizationProcedureDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub application: Option<Vec<MedicinalProductAuthorizationProcedure>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub holder: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regulator: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure: Option<MedicinalProductAuthorizationProcedure>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductAuthorizationJurisdictionalAuthorization {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(rename = "legalStatusOfSupply", skip_serializing_if = "Option::is_none")]
    pub legal_status_of_supply: Option<CodeableConcept>,
    #[serde(rename = "validityPeriod", skip_serializing_if = "Option::is_none")]
    pub validity_period: Option<Period>,
}


/// Choice of types for the medication[x] field in MedicinalProductContraindicationOtherTherapy
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductContraindicationOtherTherapyMedication {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "medicationReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductContraindicationOtherTherapy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "therapyRelationshipType")]
    pub therapy_relationship_type: CodeableConcept,
    #[serde(flatten)]
    pub medication: Option<MedicinalProductContraindicationOtherTherapyMedication>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub population: Option<Vec<Population>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "undesirableEffect", skip_serializing_if = "Option::is_none")]
    pub undesirable_effect: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub population: Option<Vec<Population>>,
}

/// Choice of types for the medication[x] field in MedicinalProductIndicationOtherTherapy
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductIndicationOtherTherapyMedication {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "medicationReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductIndicationOtherTherapy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "therapyRelationshipType")]
    pub therapy_relationship_type: CodeableConcept,
    #[serde(flatten)]
    pub medication: Option<MedicinalProductIndicationOtherTherapyMedication>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductIngredientSpecifiedSubstanceStrengthReferenceStrength {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substance: Option<CodeableConcept>,
    pub strength: Ratio,
    #[serde(rename = "strengthLowLimit", skip_serializing_if = "Option::is_none")]
    pub strength_low_limit: Option<Ratio>,
    #[serde(rename = "measurementPoint", skip_serializing_if = "Option::is_none")]
    pub measurement_point: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substance: Option<MedicinalProductIngredientSubstance>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductIngredientSpecifiedSubstance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub group: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidentiality: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<Vec<MedicinalProductIngredientSpecifiedSubstanceStrength>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductIngredientSpecifiedSubstanceStrength {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductIngredientSubstance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<Vec<MedicinalProductIngredientSpecifiedSubstanceStrength>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incidence: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub management: Option<CodeableConcept>,
}

/// Choice of types for the item[x] field in MedicinalProductInteractionInteractant
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductInteractionInteractantItem {
    /// Variant accepting the Reference type.
    #[serde(rename = "itemReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductInteractionInteractant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub item: Option<MedicinalProductInteractionInteractantItem>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductPackagedBatchIdentifier {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "outerPackaging")]
    pub outer_packaging: Identifier,
    #[serde(rename = "immediatePackaging", skip_serializing_if = "Option::is_none")]
    pub immediate_packaging: Option<Identifier>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "packageItem", skip_serializing_if = "Option::is_none")]
    pub package_item: Option<Vec<MedicinalProductPackagedPackageItem>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductPackagedPackageItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
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
    #[serde(rename = "packageItem", skip_serializing_if = "Option::is_none")]
    pub package_item: Option<Vec<MedicinalProductPackagedPackageItem>>,
    #[serde(rename = "physicalCharacteristics", skip_serializing_if = "Option::is_none")]
    pub physical_characteristics: Option<ProdCharacteristic>,
    #[serde(rename = "otherCharacteristics", skip_serializing_if = "Option::is_none")]
    pub other_characteristics: Option<Vec<CodeableConcept>>,
    #[serde(rename = "shelfLifeStorage", skip_serializing_if = "Option::is_none")]
    pub shelf_life_storage: Option<Vec<ProductShelfLife>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductPharmaceuticalRouteOfAdministrationTargetSpecies {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(rename = "withdrawalPeriod", skip_serializing_if = "Option::is_none")]
    pub withdrawal_period: Option<Vec<MedicinalProductPharmaceuticalRouteOfAdministrationTargetSpeciesWithdrawalPeriod>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductPharmaceuticalCharacteristics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductPharmaceuticalRouteOfAdministrationTargetSpeciesWithdrawalPeriod {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub tissue: CodeableConcept,
    pub value: Quantity,
    #[serde(rename = "supportingInformation", skip_serializing_if = "Option::is_none")]
    pub supporting_information: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductPharmaceuticalRouteOfAdministration {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "routeOfAdministration", skip_serializing_if = "Option::is_none")]
    pub route_of_administration: Option<Vec<MedicinalProductPharmaceuticalRouteOfAdministration>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageDefinitionFocus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Canonical>,
    pub min: UnsignedInt,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageDefinitionAllowedResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub message: Canonical,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub situation: Option<Markdown>,
}

/// Choice of types for the event[x] field in MessageDefinition
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageDefinitionEvent {
    /// Variant accepting the Coding type.
    #[serde(rename = "eventCoding")]
    Coding(Coding),
    /// Variant accepting the Uri type.
    #[serde(rename = "eventUri")]
    Uri(Uri),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub event: Option<MessageDefinitionEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<Vec<MessageDefinitionFocus>>,
    #[serde(rename = "responseRequired", skip_serializing_if = "Option::is_none")]
    pub response_required: Option<Code>,
    #[serde(rename = "allowedResponse", skip_serializing_if = "Option::is_none")]
    pub allowed_response: Option<Vec<MessageDefinitionAllowedResponse>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<Vec<Canonical>>,
}


/// Choice of types for the event[x] field in MessageHeader
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageHeaderEvent {
    /// Variant accepting the Coding type.
    #[serde(rename = "eventCoding")]
    Coding(Coding),
    /// Variant accepting the Uri type.
    #[serde(rename = "eventUri")]
    Uri(Uri),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub event: Option<MessageHeaderEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<Vec<MessageHeaderDestination>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterer: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Reference>,
    pub source: MessageHeaderSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsible: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<MessageHeaderResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<Canonical>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageHeaderResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Id,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageHeaderDestination {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Reference>,
    pub endpoint: Url,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageHeaderSource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub software: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<ContactPoint>,
    pub endpoint: Url,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MolecularSequenceQuality {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(rename = "standardSequence", skip_serializing_if = "Option::is_none")]
    pub standard_sequence: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<Integer>,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MolecularSequenceStructureVariant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MolecularSequenceQualityRoc {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<Vec<Integer>>,
    #[serde(rename = "numTP", skip_serializing_if = "Option::is_none")]
    pub num_t_p: Option<Vec<Integer>>,
    #[serde(rename = "numFP", skip_serializing_if = "Option::is_none")]
    pub num_f_p: Option<Vec<Integer>>,
    #[serde(rename = "numFN", skip_serializing_if = "Option::is_none")]
    pub num_f_n: Option<Vec<Integer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precision: Option<Vec<Decimal>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<Vec<Decimal>>,
    #[serde(rename = "fMeasure", skip_serializing_if = "Option::is_none")]
    pub f_measure: Option<Vec<Decimal>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MolecularSequenceReferenceSeq {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub variant: Option<Vec<MolecularSequenceVariant>>,
    #[serde(rename = "observedSeq", skip_serializing_if = "Option::is_none")]
    pub observed_seq: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<Vec<MolecularSequenceQuality>>,
    #[serde(rename = "readCoverage", skip_serializing_if = "Option::is_none")]
    pub read_coverage: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<Vec<MolecularSequenceRepository>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<Vec<Reference>>,
    #[serde(rename = "structureVariant", skip_serializing_if = "Option::is_none")]
    pub structure_variant: Option<Vec<MolecularSequenceStructureVariant>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MolecularSequenceVariant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MolecularSequenceRepository {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MolecularSequenceStructureVariantOuter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<Integer>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MolecularSequenceStructureVariantInner {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<Integer>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamingSystemUniqueId {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}


/// Choice of types for the rate[x] field in NutritionOrderEnteralFormulaAdministration
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NutritionOrderEnteralFormulaAdministrationRate {
    /// Variant accepting the Quantity type.
    #[serde(rename = "rateQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Ratio type.
    #[serde(rename = "rateRatio")]
    Ratio(Ratio),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NutritionOrderEnteralFormulaAdministration {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Timing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(flatten)]
    pub rate: Option<NutritionOrderEnteralFormulaAdministrationRate>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NutritionOrderOralDietNutrient {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NutritionOrderOralDietTexture {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<CodeableConcept>,
    #[serde(rename = "foodType", skip_serializing_if = "Option::is_none")]
    pub food_type: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NutritionOrderOralDiet {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Vec<Timing>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nutrient: Option<Vec<NutritionOrderOralDietNutrient>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub texture: Option<Vec<NutritionOrderOralDietTexture>>,
    #[serde(rename = "fluidConsistencyType", skip_serializing_if = "Option::is_none")]
    pub fluid_consistency_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NutritionOrderEnteralFormula {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
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
    #[serde(rename = "maxVolumeToDeliver", skip_serializing_if = "Option::is_none")]
    pub max_volume_to_deliver: Option<Quantity>,
    #[serde(rename = "administrationInstruction", skip_serializing_if = "Option::is_none")]
    pub administration_instruction: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supplement: Option<Vec<NutritionOrderSupplement>>,
    #[serde(rename = "enteralFormula", skip_serializing_if = "Option::is_none")]
    pub enteral_formula: Option<NutritionOrderEnteralFormula>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NutritionOrderSupplement {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "productName", skip_serializing_if = "Option::is_none")]
    pub product_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Vec<Timing>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationReferenceRange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Choice of types for the value[x] field in ObservationComponent
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservationComponentValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationComponent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(flatten)]
    pub value: Option<ObservationComponentValue>,
    #[serde(rename = "dataAbsentReason", skip_serializing_if = "Option::is_none")]
    pub data_absent_reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interpretation: Option<Vec<CodeableConcept>>,
    #[serde(rename = "referenceRange", skip_serializing_if = "Option::is_none")]
    pub reference_range: Option<Vec<ObservationReferenceRange>>,
}

/// Choice of types for the effective[x] field in Observation
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservationEffective {
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
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservationValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub effective: Option<ObservationEffective>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued: Option<Instant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<Reference>>,
    #[serde(flatten)]
    pub value: Option<ObservationValue>,
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
    #[serde(rename = "hasMember", skip_serializing_if = "Option::is_none")]
    pub has_member: Option<Vec<Reference>>,
    #[serde(rename = "derivedFrom", skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<Vec<ObservationComponent>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationDefinitionQuantitativeDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "customaryUnit", skip_serializing_if = "Option::is_none")]
    pub customary_unit: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<CodeableConcept>,
    #[serde(rename = "conversionFactor", skip_serializing_if = "Option::is_none")]
    pub conversion_factor: Option<Decimal>,
    #[serde(rename = "decimalPrecision", skip_serializing_if = "Option::is_none")]
    pub decimal_precision: Option<Integer>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "qualifiedInterval", skip_serializing_if = "Option::is_none")]
    pub qualified_interval: Option<Vec<ObservationDefinitionQualifiedInterval>>,
    #[serde(rename = "validCodedValueSet", skip_serializing_if = "Option::is_none")]
    pub valid_coded_value_set: Option<Reference>,
    #[serde(rename = "normalCodedValueSet", skip_serializing_if = "Option::is_none")]
    pub normal_coded_value_set: Option<Reference>,
    #[serde(rename = "abnormalCodedValueSet", skip_serializing_if = "Option::is_none")]
    pub abnormal_coded_value_set: Option<Reference>,
    #[serde(rename = "criticalCodedValueSet", skip_serializing_if = "Option::is_none")]
    pub critical_coded_value_set: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationDefinitionQualifiedInterval {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Code>,
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
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overload: Option<Vec<OperationDefinitionOverload>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationDefinitionOverload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "parameterName", skip_serializing_if = "Option::is_none")]
    pub parameter_name: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationDefinitionParameterBinding {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub strength: Code,
    #[serde(rename = "valueSet")]
    pub value_set: Canonical,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationDefinitionParameterReferencedFrom {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub source: String,
    #[serde(rename = "sourceId", skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationDefinitionParameter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Code,
    #[serde(rename = "use")]
    pub r#use: Code,
    pub min: Integer,
    pub max: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
    #[serde(rename = "targetProfile", skip_serializing_if = "Option::is_none")]
    pub target_profile: Option<Vec<Canonical>>,
    #[serde(rename = "searchType", skip_serializing_if = "Option::is_none")]
    pub search_type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<OperationDefinitionParameterBinding>,
    #[serde(rename = "referencedFrom", skip_serializing_if = "Option::is_none")]
    pub referenced_from: Option<Vec<OperationDefinitionParameterReferencedFrom>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<Vec<OperationDefinitionParameter>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationOutcomeIssue {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrganizationContact {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<HumanName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<Address>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub endpoint: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


/// Choice of types for the value[x] field in ParametersParameter
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParametersParameterValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParametersParameter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(flatten)]
    pub value: Option<ParametersParameterValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Resource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<Vec<ParametersParameter>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatientCommunication {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatientLink {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub other: Reference,
    #[serde(rename = "type")]
    pub r#type: Code,
}

/// Choice of types for the deceased[x] field in Patient
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PatientDeceased {
    /// Variant accepting the Boolean type.
    #[serde(rename = "deceasedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the DateTime type.
    #[serde(rename = "deceasedDateTime")]
    DateTime(DateTime),
}

/// Choice of types for the multipleBirth[x] field in Patient
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PatientMultipleBirth {
    /// Variant accepting the Boolean type.
    #[serde(rename = "multipleBirthBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[serde(rename = "multipleBirthInteger")]
    Integer(Integer),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub deceased: Option<PatientDeceased>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<Vec<Address>>,
    #[serde(rename = "maritalStatus", skip_serializing_if = "Option::is_none")]
    pub marital_status: Option<CodeableConcept>,
    #[serde(flatten)]
    pub multiple_birth: Option<PatientMultipleBirth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo: Option<Vec<Attachment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<PatientContact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub communication: Option<Vec<PatientCommunication>>,
    #[serde(rename = "generalPractitioner", skip_serializing_if = "Option::is_none")]
    pub general_practitioner: Option<Vec<Reference>>,
    #[serde(rename = "managingOrganization", skip_serializing_if = "Option::is_none")]
    pub managing_organization: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<Vec<PatientLink>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatientContact {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<HumanName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<Address>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "formCode", skip_serializing_if = "Option::is_none")]
    pub form_code: Option<CodeableConcept>,
    #[serde(rename = "processNote", skip_serializing_if = "Option::is_none")]
    pub process_note: Option<Vec<PaymentReconciliationProcessNote>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentReconciliationDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predecessor: Option<Identifier>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<Reference>,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentReconciliationProcessNote {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonLink {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub target: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assurance: Option<Code>,
}


/// Choice of types for the subject[x] field in PlanDefinitionAction
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionActionSubject {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

/// Choice of types for the timing[x] field in PlanDefinitionAction
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionActionTiming {
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

/// Choice of types for the definition[x] field in PlanDefinitionAction
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionActionDefinition {
    /// Variant accepting the Canonical type.
    #[serde(rename = "definitionCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Uri type.
    #[serde(rename = "definitionUri")]
    Uri(Uri),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDefinitionAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "textEquivalent", skip_serializing_if = "Option::is_none")]
    pub text_equivalent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Vec<RelatedArtifact>>,
    #[serde(rename = "goalId", skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<Vec<Id>>,
    #[serde(flatten)]
    pub subject: Option<PlanDefinitionActionSubject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger: Option<Vec<TriggerDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Vec<PlanDefinitionActionCondition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<DataRequirement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Vec<DataRequirement>>,
    #[serde(rename = "relatedAction", skip_serializing_if = "Option::is_none")]
    pub related_action: Option<Vec<PlanDefinitionActionRelatedAction>>,
    #[serde(flatten)]
    pub timing: Option<PlanDefinitionActionTiming>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant: Option<Vec<PlanDefinitionActionParticipant>>,
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
    #[serde(flatten)]
    pub definition: Option<PlanDefinitionActionDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<Canonical>,
    #[serde(rename = "dynamicValue", skip_serializing_if = "Option::is_none")]
    pub dynamic_value: Option<Vec<PlanDefinitionActionDynamicValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<PlanDefinitionAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDefinitionActionDynamicValue {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<Expression>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDefinitionActionCondition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub kind: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<Expression>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDefinitionActionParticipant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
}

/// Choice of types for the detail[x] field in PlanDefinitionGoalTarget
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionGoalTargetDetail {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDefinitionGoalTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measure: Option<CodeableConcept>,
    #[serde(flatten)]
    pub detail: Option<PlanDefinitionGoalTargetDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<Duration>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDefinitionGoal {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    pub description: CodeableConcept,
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
}

/// Choice of types for the subject[x] field in PlanDefinition
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionSubject {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub subject: Option<PlanDefinitionSubject>,
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
    pub action: Option<Vec<PlanDefinitionAction>>,
}

/// Choice of types for the offset[x] field in PlanDefinitionActionRelatedAction
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionActionRelatedActionOffset {
    /// Variant accepting the Duration type.
    #[serde(rename = "offsetDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[serde(rename = "offsetRange")]
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDefinitionActionRelatedAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "actionId")]
    pub action_id: Id,
    pub relationship: Code,
    #[serde(flatten)]
    pub offset: Option<PlanDefinitionActionRelatedActionOffset>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PractitionerQualification {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub code: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub communication: Option<Vec<CodeableConcept>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PractitionerRoleAvailableTime {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "daysOfWeek", skip_serializing_if = "Option::is_none")]
    pub days_of_week: Option<Vec<Code>>,
    #[serde(rename = "allDay", skip_serializing_if = "Option::is_none")]
    pub all_day: Option<Boolean>,
    #[serde(rename = "availableStartTime", skip_serializing_if = "Option::is_none")]
    pub available_start_time: Option<Time>,
    #[serde(rename = "availableEndTime", skip_serializing_if = "Option::is_none")]
    pub available_end_time: Option<Time>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "notAvailable", skip_serializing_if = "Option::is_none")]
    pub not_available: Option<Vec<PractitionerRoleNotAvailable>>,
    #[serde(rename = "availabilityExceptions", skip_serializing_if = "Option::is_none")]
    pub availability_exceptions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PractitionerRoleNotAvailable {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub during: Option<Period>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcedurePerformer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
    #[serde(rename = "onBehalfOf", skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcedureFocalDevice {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<CodeableConcept>,
    pub manipulated: Reference,
}

/// Choice of types for the performed[x] field in Procedure
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcedurePerformed {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub performed: Option<ProcedurePerformed>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorder: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asserter: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<ProcedurePerformer>>,
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
    #[serde(rename = "usedReference", skip_serializing_if = "Option::is_none")]
    pub used_reference: Option<Vec<Reference>>,
    #[serde(rename = "usedCode", skip_serializing_if = "Option::is_none")]
    pub used_code: Option<Vec<CodeableConcept>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceAgent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Vec<CodeableConcept>>,
    pub who: Reference,
    #[serde(rename = "onBehalfOf", skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceEntity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Code,
    pub what: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<Vec<ProvenanceAgent>>,
}

/// Choice of types for the occurred[x] field in Provenance
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProvenanceOccurred {
    /// Variant accepting the Period type.
    #[serde(rename = "occurredPeriod")]
    Period(Period),
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurredDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub occurred: Option<ProvenanceOccurred>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<Vec<ProvenanceEntity>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<Signature>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}

/// Choice of types for the answer[x] field in QuestionnaireItemEnableWhen
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireItemEnableWhenAnswer {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionnaireItemEnableWhen {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub question: String,
    pub operator: Code,
    #[serde(flatten)]
    pub answer: Option<QuestionnaireItemEnableWhenAnswer>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionnaireItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "linkId")]
    pub link_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<Coding>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(rename = "enableWhen", skip_serializing_if = "Option::is_none")]
    pub enable_when: Option<Vec<QuestionnaireItemEnableWhen>>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial: Option<Vec<QuestionnaireItemInitial>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<QuestionnaireItem>>,
}

/// Choice of types for the value[x] field in QuestionnaireItemAnswerOption
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireItemAnswerOptionValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionnaireItemAnswerOption {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub value: Option<QuestionnaireItemAnswerOptionValue>,
    #[serde(rename = "initialSelected", skip_serializing_if = "Option::is_none")]
    pub initial_selected: Option<Boolean>,
}

/// Choice of types for the value[x] field in QuestionnaireItemInitial
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireItemInitialValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionnaireItemInitial {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub value: Option<QuestionnaireItemInitialValue>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionnaireResponseItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "linkId")]
    pub link_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<Vec<QuestionnaireResponseItemAnswer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<QuestionnaireResponseItem>>,
}

/// Choice of types for the value[x] field in QuestionnaireResponseItemAnswer
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireResponseItemAnswerValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionnaireResponseItemAnswer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub value: Option<QuestionnaireResponseItemAnswerValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<QuestionnaireResponseItem>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelatedPersonCommunication {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestGroupActionCondition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub kind: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<Expression>,
}

/// Choice of types for the offset[x] field in RequestGroupActionRelatedAction
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RequestGroupActionRelatedActionOffset {
    /// Variant accepting the Duration type.
    #[serde(rename = "offsetDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[serde(rename = "offsetRange")]
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestGroupActionRelatedAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "actionId")]
    pub action_id: Id,
    pub relationship: Code,
    #[serde(flatten)]
    pub offset: Option<RequestGroupActionRelatedActionOffset>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}

/// Choice of types for the timing[x] field in RequestGroupAction
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RequestGroupActionTiming {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestGroupAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "textEquivalent", skip_serializing_if = "Option::is_none")]
    pub text_equivalent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Vec<RelatedArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Vec<RequestGroupActionCondition>>,
    #[serde(rename = "relatedAction", skip_serializing_if = "Option::is_none")]
    pub related_action: Option<Vec<RequestGroupActionRelatedAction>>,
    #[serde(flatten)]
    pub timing: Option<RequestGroupActionTiming>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<RequestGroupAction>>,
}


/// Choice of types for the subject[x] field in ResearchDefinition
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResearchDefinitionSubject {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub subject: Option<ResearchDefinitionSubject>,
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


/// Choice of types for the definition[x] field in ResearchElementDefinitionCharacteristic
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionCharacteristicDefinition {
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

/// Choice of types for the studyEffective[x] field in ResearchElementDefinitionCharacteristic
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionCharacteristicStudyEffective {
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

/// Choice of types for the participantEffective[x] field in ResearchElementDefinitionCharacteristic
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionCharacteristicParticipantEffective {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchElementDefinitionCharacteristic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub definition: Option<ResearchElementDefinitionCharacteristicDefinition>,
    #[serde(rename = "usageContext", skip_serializing_if = "Option::is_none")]
    pub usage_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Boolean>,
    #[serde(rename = "unitOfMeasure", skip_serializing_if = "Option::is_none")]
    pub unit_of_measure: Option<CodeableConcept>,
    #[serde(rename = "studyEffectiveDescription", skip_serializing_if = "Option::is_none")]
    pub study_effective_description: Option<String>,
    #[serde(flatten)]
    pub study_effective: Option<ResearchElementDefinitionCharacteristicStudyEffective>,
    #[serde(rename = "studyEffectiveTimeFromStart", skip_serializing_if = "Option::is_none")]
    pub study_effective_time_from_start: Option<Duration>,
    #[serde(rename = "studyEffectiveGroupMeasure", skip_serializing_if = "Option::is_none")]
    pub study_effective_group_measure: Option<Code>,
    #[serde(rename = "participantEffectiveDescription", skip_serializing_if = "Option::is_none")]
    pub participant_effective_description: Option<String>,
    #[serde(flatten)]
    pub participant_effective: Option<ResearchElementDefinitionCharacteristicParticipantEffective>,
    #[serde(rename = "participantEffectiveTimeFromStart", skip_serializing_if = "Option::is_none")]
    pub participant_effective_time_from_start: Option<Duration>,
    #[serde(rename = "participantEffectiveGroupMeasure", skip_serializing_if = "Option::is_none")]
    pub participant_effective_group_measure: Option<Code>,
}

/// Choice of types for the subject[x] field in ResearchElementDefinition
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionSubject {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub subject: Option<ResearchElementDefinitionSubject>,
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
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchStudyArm {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective: Option<Vec<ResearchStudyObjective>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchStudyObjective {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


/// Choice of types for the probability[x] field in RiskAssessmentPrediction
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiskAssessmentPredictionProbability {
    /// Variant accepting the Decimal type.
    #[serde(rename = "probabilityDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Range type.
    #[serde(rename = "probabilityRange")]
    Range(Range),
}

/// Choice of types for the when[x] field in RiskAssessmentPrediction
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiskAssessmentPredictionWhen {
    /// Variant accepting the Period type.
    #[serde(rename = "whenPeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[serde(rename = "whenRange")]
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskAssessmentPrediction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<CodeableConcept>,
    #[serde(flatten)]
    pub probability: Option<RiskAssessmentPredictionProbability>,
    #[serde(rename = "qualitativeRisk", skip_serializing_if = "Option::is_none")]
    pub qualitative_risk: Option<CodeableConcept>,
    #[serde(rename = "relativeRisk", skip_serializing_if = "Option::is_none")]
    pub relative_risk: Option<Decimal>,
    #[serde(flatten)]
    pub when: Option<RiskAssessmentPredictionWhen>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

/// Choice of types for the occurrence[x] field in RiskAssessment
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiskAssessmentOccurrence {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "occurrencePeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub occurrence: Option<RiskAssessmentOccurrence>,
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
    pub mitigation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskEvidenceSynthesisCertaintyCertaintySubcomponent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskEvidenceSynthesisRiskEstimate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskEvidenceSynthesisRiskEstimatePrecisionEstimate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Decimal>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "riskEstimate", skip_serializing_if = "Option::is_none")]
    pub risk_estimate: Option<RiskEvidenceSynthesisRiskEstimate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certainty: Option<Vec<RiskEvidenceSynthesisCertainty>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskEvidenceSynthesisSampleSize {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "numberOfStudies", skip_serializing_if = "Option::is_none")]
    pub number_of_studies: Option<Integer>,
    #[serde(rename = "numberOfParticipants", skip_serializing_if = "Option::is_none")]
    pub number_of_participants: Option<Integer>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskEvidenceSynthesisCertainty {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "certaintySubcomponent", skip_serializing_if = "Option::is_none")]
    pub certainty_subcomponent: Option<Vec<RiskEvidenceSynthesisCertaintyCertaintySubcomponent>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchParameterComponent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub definition: Canonical,
    pub expression: String,
}


/// Choice of types for the quantity[x] field in ServiceRequest
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceRequestQuantity {
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
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceRequestOccurrence {
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
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceRequestAsNeeded {
    /// Variant accepting the Boolean type.
    #[serde(rename = "asNeededBoolean")]
    Boolean(Boolean),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "asNeededCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub quantity: Option<ServiceRequestQuantity>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(flatten)]
    pub occurrence: Option<ServiceRequestOccurrence>,
    #[serde(flatten)]
    pub as_needed: Option<ServiceRequestAsNeeded>,
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub processing: Option<Vec<SpecimenProcessing>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<Vec<SpecimenContainer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the additive[x] field in SpecimenContainer
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenContainerAdditive {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "additiveCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "additiveReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecimenContainer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity: Option<Quantity>,
    #[serde(rename = "specimenQuantity", skip_serializing_if = "Option::is_none")]
    pub specimen_quantity: Option<Quantity>,
    #[serde(flatten)]
    pub additive: Option<SpecimenContainerAdditive>,
}

/// Choice of types for the collected[x] field in SpecimenCollection
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenCollectionCollected {
    /// Variant accepting the DateTime type.
    #[serde(rename = "collectedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "collectedPeriod")]
    Period(Period),
}

/// Choice of types for the fastingStatus[x] field in SpecimenCollection
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenCollectionFastingStatus {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "fastingStatusCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Duration type.
    #[serde(rename = "fastingStatusDuration")]
    Duration(Duration),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecimenCollection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collector: Option<Reference>,
    #[serde(flatten)]
    pub collected: Option<SpecimenCollectionCollected>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<Duration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(rename = "bodySite", skip_serializing_if = "Option::is_none")]
    pub body_site: Option<CodeableConcept>,
    #[serde(flatten)]
    pub fasting_status: Option<SpecimenCollectionFastingStatus>,
}

/// Choice of types for the time[x] field in SpecimenProcessing
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenProcessingTime {
    /// Variant accepting the DateTime type.
    #[serde(rename = "timeDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "timePeriod")]
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecimenProcessing {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additive: Option<Vec<Reference>>,
    #[serde(flatten)]
    pub time: Option<SpecimenProcessingTime>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}

/// Choice of types for the minimumVolume[x] field in SpecimenDefinitionTypeTestedContainer
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenDefinitionTypeTestedContainerMinimumVolume {
    /// Variant accepting the Quantity type.
    #[serde(rename = "minimumVolumeQuantity")]
    Quantity(Quantity),
    /// Variant accepting the String type.
    #[serde(rename = "minimumVolumeString")]
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecimenDefinitionTypeTestedContainer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<CodeableConcept>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity: Option<Quantity>,
    #[serde(flatten)]
    pub minimum_volume: Option<SpecimenDefinitionTypeTestedContainerMinimumVolume>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additive: Option<Vec<SpecimenDefinitionTypeTestedContainerAdditive>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preparation: Option<String>,
}

/// Choice of types for the additive[x] field in SpecimenDefinitionTypeTestedContainerAdditive
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenDefinitionTypeTestedContainerAdditiveAdditive {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "additiveCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "additiveReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecimenDefinitionTypeTestedContainerAdditive {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub additive: Option<SpecimenDefinitionTypeTestedContainerAdditiveAdditive>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecimenDefinitionTypeTestedHandling {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "temperatureQualifier", skip_serializing_if = "Option::is_none")]
    pub temperature_qualifier: Option<CodeableConcept>,
    #[serde(rename = "temperatureRange", skip_serializing_if = "Option::is_none")]
    pub temperature_range: Option<Range>,
    #[serde(rename = "maxDuration", skip_serializing_if = "Option::is_none")]
    pub max_duration: Option<Duration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecimenDefinitionTypeTested {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "isDerived", skip_serializing_if = "Option::is_none")]
    pub is_derived: Option<Boolean>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    pub preference: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<SpecimenDefinitionTypeTestedContainer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
    #[serde(rename = "retentionTime", skip_serializing_if = "Option::is_none")]
    pub retention_time: Option<Duration>,
    #[serde(rename = "rejectionCriterion", skip_serializing_if = "Option::is_none")]
    pub rejection_criterion: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handling: Option<Vec<SpecimenDefinitionTypeTestedHandling>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub kind: Code,
    #[serde(rename = "abstract")]
    pub r#abstract: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Vec<StructureDefinitionContext>>,
    #[serde(rename = "contextInvariant", skip_serializing_if = "Option::is_none")]
    pub context_invariant: Option<Vec<String>>,
    #[serde(rename = "type")]
    pub r#type: Uri,
    #[serde(rename = "baseDefinition", skip_serializing_if = "Option::is_none")]
    pub base_definition: Option<Canonical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derivation: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<StructureDefinitionSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub differential: Option<StructureDefinitionDifferential>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureDefinitionDifferential {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<Vec<ElementDefinition>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureDefinitionMapping {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identity: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureDefinitionContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub expression: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureDefinitionSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<Vec<ElementDefinition>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureMapStructure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Canonical,
    pub mode: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}

/// Choice of types for the defaultValue[x] field in StructureMapGroupRuleSource
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StructureMapGroupRuleSourceDefaultValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureMapGroupRuleSource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub context: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(flatten)]
    pub default_value: Option<StructureMapGroupRuleSourceDefaultValue>,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureMapGroupInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Id,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    pub mode: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureMapGroupRule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<StructureMapGroupRuleSource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<StructureMapGroupRuleTarget>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<Vec<StructureMapGroupRule>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependent: Option<Vec<StructureMapGroupRuleDependent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureMapGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends: Option<Id>,
    #[serde(rename = "typeMode")]
    pub type_mode: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<StructureMapGroupInput>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<Vec<StructureMapGroupRule>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureMapGroupRuleTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Id>,
    #[serde(rename = "contextType", skip_serializing_if = "Option::is_none")]
    pub context_type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable: Option<Id>,
    #[serde(rename = "listMode", skip_serializing_if = "Option::is_none")]
    pub list_mode: Option<Vec<Code>>,
    #[serde(rename = "listRuleId", skip_serializing_if = "Option::is_none")]
    pub list_rule_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<StructureMapGroupRuleTargetParameter>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureMapGroupRuleDependent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Vec<StructureMapGroup>>,
}

/// Choice of types for the value[x] field in StructureMapGroupRuleTargetParameter
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StructureMapGroupRuleTargetParameterValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureMapGroupRuleTargetParameter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub value: Option<StructureMapGroupRuleTargetParameterValue>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionChannel {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Url>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub ingredient: Option<Vec<SubstanceIngredient>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceInstance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
}

/// Choice of types for the substance[x] field in SubstanceIngredient
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceIngredientSubstance {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "substanceCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "substanceReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceIngredient {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Ratio>,
    #[serde(flatten)]
    pub substance: Option<SubstanceIngredientSubstance>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceNucleicAcidSubunitLinkage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connectivity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "residueSite", skip_serializing_if = "Option::is_none")]
    pub residue_site: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceNucleicAcidSubunit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subunit: Option<Integer>,
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
    pub sugar: Option<Vec<SubstanceNucleicAcidSubunitSugar>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceNucleicAcidSubunitSugar {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "residueSite", skip_serializing_if = "Option::is_none")]
    pub residue_site: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstancePolymerRepeat {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "numberOfUnits", skip_serializing_if = "Option::is_none")]
    pub number_of_units: Option<Integer>,
    #[serde(rename = "averageMolecularFormula", skip_serializing_if = "Option::is_none")]
    pub average_molecular_formula: Option<String>,
    #[serde(rename = "repeatUnitAmountType", skip_serializing_if = "Option::is_none")]
    pub repeat_unit_amount_type: Option<CodeableConcept>,
    #[serde(rename = "repeatUnit", skip_serializing_if = "Option::is_none")]
    pub repeat_unit: Option<Vec<SubstancePolymerRepeatRepeatUnit>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstancePolymerRepeatRepeatUnit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "orientationOfPolymerisation", skip_serializing_if = "Option::is_none")]
    pub orientation_of_polymerisation: Option<CodeableConcept>,
    #[serde(rename = "repeatUnit", skip_serializing_if = "Option::is_none")]
    pub repeat_unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<SubstanceAmount>,
    #[serde(rename = "degreeOfPolymerisation", skip_serializing_if = "Option::is_none")]
    pub degree_of_polymerisation: Option<Vec<SubstancePolymerRepeatRepeatUnitDegreeOfPolymerisation>>,
    #[serde(rename = "structuralRepresentation", skip_serializing_if = "Option::is_none")]
    pub structural_representation: Option<Vec<SubstancePolymerRepeatRepeatUnitStructuralRepresentation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstancePolymerRepeatRepeatUnitDegreeOfPolymerisation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degree: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<SubstanceAmount>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat: Option<Vec<SubstancePolymerRepeat>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstancePolymerMonomerSet {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "ratioType", skip_serializing_if = "Option::is_none")]
    pub ratio_type: Option<CodeableConcept>,
    #[serde(rename = "startingMaterial", skip_serializing_if = "Option::is_none")]
    pub starting_material: Option<Vec<SubstancePolymerMonomerSetStartingMaterial>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstancePolymerMonomerSetStartingMaterial {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<CodeableConcept>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "isDefining", skip_serializing_if = "Option::is_none")]
    pub is_defining: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<SubstanceAmount>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstancePolymerRepeatRepeatUnitStructuralRepresentation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub representation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<Attachment>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceProteinSubunit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subunit: Option<Integer>,
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


/// Choice of types for the amount[x] field in SubstanceReferenceInformationTarget
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceReferenceInformationTargetAmount {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceReferenceInformationTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Identifier>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organism: Option<CodeableConcept>,
    #[serde(rename = "organismType", skip_serializing_if = "Option::is_none")]
    pub organism_type: Option<CodeableConcept>,
    #[serde(flatten)]
    pub amount: Option<SubstanceReferenceInformationTargetAmount>,
    #[serde(rename = "amountType", skip_serializing_if = "Option::is_none")]
    pub amount_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceReferenceInformationGene {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "geneSequenceOrigin", skip_serializing_if = "Option::is_none")]
    pub gene_sequence_origin: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gene: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceReferenceInformationGeneElement {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceReferenceInformationClassification {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtype: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "geneElement", skip_serializing_if = "Option::is_none")]
    pub gene_element: Option<Vec<SubstanceReferenceInformationGeneElement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<Vec<SubstanceReferenceInformationClassification>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<SubstanceReferenceInformationTarget>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSourceMaterialOrganismAuthor {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "authorType", skip_serializing_if = "Option::is_none")]
    pub author_type: Option<CodeableConcept>,
    #[serde(rename = "authorDescription", skip_serializing_if = "Option::is_none")]
    pub author_description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSourceMaterialOrganismOrganismGeneral {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kingdom: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phylum: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSourceMaterialFractionDescription {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fraction: Option<String>,
    #[serde(rename = "materialType", skip_serializing_if = "Option::is_none")]
    pub material_type: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub organism: Option<SubstanceSourceMaterialOrganism>,
    #[serde(rename = "partDescription", skip_serializing_if = "Option::is_none")]
    pub part_description: Option<Vec<SubstanceSourceMaterialPartDescription>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSourceMaterialPartDescription {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<CodeableConcept>,
    #[serde(rename = "partLocation", skip_serializing_if = "Option::is_none")]
    pub part_location: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSourceMaterialOrganismHybrid {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSourceMaterialOrganism {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hybrid: Option<SubstanceSourceMaterialOrganismHybrid>,
    #[serde(rename = "organismGeneral", skip_serializing_if = "Option::is_none")]
    pub organism_general: Option<SubstanceSourceMaterialOrganismOrganismGeneral>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationStructureIsotope {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substitution: Option<CodeableConcept>,
    #[serde(rename = "halfLife", skip_serializing_if = "Option::is_none")]
    pub half_life: Option<Quantity>,
    #[serde(rename = "molecularWeight", skip_serializing_if = "Option::is_none")]
    pub molecular_weight: Option<SubstanceSpecificationStructureIsotopeMolecularWeight>,
}

/// Choice of types for the substance[x] field in SubstanceSpecificationRelationship
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSpecificationRelationshipSubstance {
    /// Variant accepting the Reference type.
    #[serde(rename = "substanceReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "substanceCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

/// Choice of types for the amount[x] field in SubstanceSpecificationRelationship
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSpecificationRelationshipAmount {
    /// Variant accepting the Quantity type.
    #[serde(rename = "amountQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "amountRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "amountRatio")]
    Ratio(Ratio),
    /// Variant accepting the String type.
    #[serde(rename = "amountString")]
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationRelationship {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub substance: Option<SubstanceSpecificationRelationshipSubstance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<CodeableConcept>,
    #[serde(rename = "isDefining", skip_serializing_if = "Option::is_none")]
    pub is_defining: Option<Boolean>,
    #[serde(flatten)]
    pub amount: Option<SubstanceSpecificationRelationshipAmount>,
    #[serde(rename = "amountRatioLowLimit", skip_serializing_if = "Option::is_none")]
    pub amount_ratio_low_limit: Option<Ratio>,
    #[serde(rename = "amountType", skip_serializing_if = "Option::is_none")]
    pub amount_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationStructureIsotopeMolecularWeight {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationName {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synonym: Option<Vec<SubstanceSpecificationName>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation: Option<Vec<SubstanceSpecificationName>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub official: Option<Vec<SubstanceSpecificationNameOfficial>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationStructureRepresentation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub representation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<Attachment>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationCode {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<CodeableConcept>,
    #[serde(rename = "statusDate", skip_serializing_if = "Option::is_none")]
    pub status_date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<Reference>>,
}

/// Choice of types for the amount[x] field in SubstanceSpecificationMoiety
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSpecificationMoietyAmount {
    /// Variant accepting the Quantity type.
    #[serde(rename = "amountQuantity")]
    Quantity(Quantity),
    /// Variant accepting the String type.
    #[serde(rename = "amountString")]
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationMoiety {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stereochemistry: Option<CodeableConcept>,
    #[serde(rename = "opticalActivity", skip_serializing_if = "Option::is_none")]
    pub optical_activity: Option<CodeableConcept>,
    #[serde(rename = "molecularFormula", skip_serializing_if = "Option::is_none")]
    pub molecular_formula: Option<String>,
    #[serde(flatten)]
    pub amount: Option<SubstanceSpecificationMoietyAmount>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationNameOfficial {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
}

/// Choice of types for the definingSubstance[x] field in SubstanceSpecificationProperty
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSpecificationPropertyDefiningSubstance {
    /// Variant accepting the Reference type.
    #[serde(rename = "definingSubstanceReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "definingSubstanceCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

/// Choice of types for the amount[x] field in SubstanceSpecificationProperty
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSpecificationPropertyAmount {
    /// Variant accepting the Quantity type.
    #[serde(rename = "amountQuantity")]
    Quantity(Quantity),
    /// Variant accepting the String type.
    #[serde(rename = "amountString")]
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationProperty {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<String>,
    #[serde(flatten)]
    pub defining_substance: Option<SubstanceSpecificationPropertyDefiningSubstance>,
    #[serde(flatten)]
    pub amount: Option<SubstanceSpecificationPropertyAmount>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub property: Option<Vec<SubstanceSpecificationProperty>>,
    #[serde(rename = "referenceInformation", skip_serializing_if = "Option::is_none")]
    pub reference_information: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structure: Option<SubstanceSpecificationStructure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<SubstanceSpecificationCode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Vec<SubstanceSpecificationName>>,
    #[serde(rename = "molecularWeight", skip_serializing_if = "Option::is_none")]
    pub molecular_weight: Option<Vec<SubstanceSpecificationStructureIsotopeMolecularWeight>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<Vec<SubstanceSpecificationRelationship>>,
    #[serde(rename = "nucleicAcid", skip_serializing_if = "Option::is_none")]
    pub nucleic_acid: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polymer: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protein: Option<Reference>,
    #[serde(rename = "sourceMaterial", skip_serializing_if = "Option::is_none")]
    pub source_material: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationStructure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stereochemistry: Option<CodeableConcept>,
    #[serde(rename = "opticalActivity", skip_serializing_if = "Option::is_none")]
    pub optical_activity: Option<CodeableConcept>,
    #[serde(rename = "molecularFormula", skip_serializing_if = "Option::is_none")]
    pub molecular_formula: Option<String>,
    #[serde(rename = "molecularFormulaByMoiety", skip_serializing_if = "Option::is_none")]
    pub molecular_formula_by_moiety: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isotope: Option<Vec<SubstanceSpecificationStructureIsotope>>,
    #[serde(rename = "molecularWeight", skip_serializing_if = "Option::is_none")]
    pub molecular_weight: Option<SubstanceSpecificationStructureIsotopeMolecularWeight>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub representation: Option<Vec<SubstanceSpecificationStructureRepresentation>>,
}


/// Choice of types for the occurrence[x] field in SupplyDelivery
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupplyDeliveryOccurrence {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(flatten)]
    pub occurrence: Option<SupplyDeliveryOccurrence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supplier: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver: Option<Vec<Reference>>,
}

/// Choice of types for the item[x] field in SupplyDeliverySuppliedItem
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupplyDeliverySuppliedItemItem {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "itemReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupplyDeliverySuppliedItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(flatten)]
    pub item: Option<SupplyDeliverySuppliedItemItem>,
}


/// Choice of types for the item[x] field in SupplyRequest
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupplyRequestItem {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "itemReference")]
    Reference(Reference),
}

/// Choice of types for the occurrence[x] field in SupplyRequest
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupplyRequestOccurrence {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub item: Option<SupplyRequestItem>,
    pub quantity: Quantity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<SupplyRequestParameter>>,
    #[serde(flatten)]
    pub occurrence: Option<SupplyRequestOccurrence>,
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

/// Choice of types for the value[x] field in SupplyRequestParameter
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupplyRequestParameterValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupplyRequestParameter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(flatten)]
    pub value: Option<SupplyRequestParameterValue>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRestriction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repetitions: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub input: Option<Vec<TaskInput>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Vec<TaskOutput>>,
}

/// Choice of types for the value[x] field in TaskInput
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskInputValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(flatten)]
    pub value: Option<TaskInputValue>,
}

/// Choice of types for the value[x] field in TaskOutput
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskOutputValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(flatten)]
    pub value: Option<TaskOutputValue>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilitiesExpansionParameter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilitiesClosure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilitiesImplementation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Url>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilitiesCodeSystemVersion {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(rename = "isDefault", skip_serializing_if = "Option::is_none")]
    pub is_default: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compositional: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Vec<Code>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Vec<TerminologyCapabilitiesCodeSystemVersionFilter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property: Option<Vec<Code>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilitiesValidateCode {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub translations: Boolean,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilitiesTranslation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "needsMap")]
    pub needs_map: Boolean,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub expansion: Option<TerminologyCapabilitiesExpansion>,
    #[serde(rename = "codeSearch", skip_serializing_if = "Option::is_none")]
    pub code_search: Option<Code>,
    #[serde(rename = "validateCode", skip_serializing_if = "Option::is_none")]
    pub validate_code: Option<TerminologyCapabilitiesValidateCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation: Option<TerminologyCapabilitiesTranslation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closure: Option<TerminologyCapabilitiesClosure>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilitiesSoftware {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilitiesExpansion {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hierarchical: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paging: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incomplete: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<TerminologyCapabilitiesExpansionParameter>>,
    #[serde(rename = "textFilter", skip_serializing_if = "Option::is_none")]
    pub text_filter: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilitiesCodeSystem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<Canonical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<Vec<TerminologyCapabilitiesCodeSystemVersion>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subsumption: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilitiesCodeSystemVersionFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op: Option<Vec<Code>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestReportSetupAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<TestReportSetupActionOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assert: Option<TestReportSetupActionAssert>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup: Option<TestReportSetup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test: Option<Vec<TestReportTest>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teardown: Option<TestReportTeardown>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestReportSetupActionAssert {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub result: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestReportTestAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<TestReportSetupActionOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assert: Option<TestReportSetupActionAssert>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestReportSetupActionOperation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub result: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Uri>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestReportTest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<TestReportTestAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestReportParticipant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub uri: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestReportTeardown {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<TestReportTeardownAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestReportSetup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<TestReportSetupAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestReportTeardownAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: TestReportSetupActionOperation,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptFixture {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub autocreate: Boolean,
    pub autodelete: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptTeardownAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: TestScriptSetupActionOperation,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptSetupAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<TestScriptSetupActionOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assert: Option<TestScriptSetupActionAssert>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptVariable {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(rename = "defaultValue", skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptOrigin {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub index: Integer,
    pub profile: Coding,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptMetadataCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub required: Boolean,
    pub validated: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Vec<Integer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<Vec<Uri>>,
    pub capabilities: Canonical,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptDestination {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub index: Integer,
    pub profile: Coding,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<Vec<TestScriptMetadataLink>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability: Option<Vec<TestScriptMetadataCapability>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<Vec<TestScriptDestination>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<TestScriptMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixture: Option<Vec<TestScriptFixture>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable: Option<Vec<TestScriptVariable>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup: Option<TestScriptSetup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test: Option<Vec<TestScriptTest>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teardown: Option<TestScriptTeardown>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptSetupActionOperation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accept: Option<Code>,
    #[serde(rename = "contentType", skip_serializing_if = "Option::is_none")]
    pub content_type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<Integer>,
    #[serde(rename = "encodeRequestUrl")]
    pub encode_request_url: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<String>,
    #[serde(rename = "requestHeader", skip_serializing_if = "Option::is_none")]
    pub request_header: Option<Vec<TestScriptSetupActionOperationRequestHeader>>,
    #[serde(rename = "requestId", skip_serializing_if = "Option::is_none")]
    pub request_id: Option<Id>,
    #[serde(rename = "responseId", skip_serializing_if = "Option::is_none")]
    pub response_id: Option<Id>,
    #[serde(rename = "sourceId", skip_serializing_if = "Option::is_none")]
    pub source_id: Option<Id>,
    #[serde(rename = "targetId", skip_serializing_if = "Option::is_none")]
    pub target_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptTestAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<TestScriptSetupActionOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assert: Option<TestScriptSetupActionAssert>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptTeardown {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<TestScriptTeardownAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptSetupActionAssert {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<Code>,
    #[serde(rename = "compareToSourceId", skip_serializing_if = "Option::is_none")]
    pub compare_to_source_id: Option<String>,
    #[serde(rename = "compareToSourceExpression", skip_serializing_if = "Option::is_none")]
    pub compare_to_source_expression: Option<String>,
    #[serde(rename = "compareToSourcePath", skip_serializing_if = "Option::is_none")]
    pub compare_to_source_path: Option<String>,
    #[serde(rename = "contentType", skip_serializing_if = "Option::is_none")]
    pub content_type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(rename = "headerField", skip_serializing_if = "Option::is_none")]
    pub header_field: Option<String>,
    #[serde(rename = "minimumId", skip_serializing_if = "Option::is_none")]
    pub minimum_id: Option<String>,
    #[serde(rename = "navigationLinks", skip_serializing_if = "Option::is_none")]
    pub navigation_links: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(rename = "requestMethod", skip_serializing_if = "Option::is_none")]
    pub request_method: Option<Code>,
    #[serde(rename = "requestURL", skip_serializing_if = "Option::is_none")]
    pub request_u_r_l: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<Code>,
    #[serde(rename = "responseCode", skip_serializing_if = "Option::is_none")]
    pub response_code: Option<String>,
    #[serde(rename = "sourceId", skip_serializing_if = "Option::is_none")]
    pub source_id: Option<Id>,
    #[serde(rename = "validateProfileId", skip_serializing_if = "Option::is_none")]
    pub validate_profile_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(rename = "warningOnly")]
    pub warning_only: Boolean,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptMetadataLink {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptSetupActionOperationRequestHeader {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub field: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptTest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<TestScriptTestAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptSetup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<TestScriptSetupAction>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueSetComposeIncludeConceptDesignation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub r#use: Option<Coding>,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueSetComposeInclude {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept: Option<Vec<ValueSetComposeIncludeConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Vec<ValueSetComposeIncludeFilter>>,
    #[serde(rename = "valueSet", skip_serializing_if = "Option::is_none")]
    pub value_set: Option<Vec<Canonical>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueSetExpansionContains {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Uri>,
    #[serde(rename = "abstract", skip_serializing_if = "Option::is_none")]
    pub r#abstract: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inactive: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub designation: Option<Vec<ValueSetComposeIncludeConceptDesignation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contains: Option<Vec<ValueSetExpansionContains>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueSetCompose {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "lockedDate", skip_serializing_if = "Option::is_none")]
    pub locked_date: Option<Date>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inactive: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<ValueSetComposeInclude>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<ValueSetComposeInclude>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueSetExpansion {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Uri>,
    pub timestamp: DateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<ValueSetExpansionParameter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contains: Option<Vec<ValueSetExpansionContains>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueSetComposeIncludeConcept {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub designation: Option<Vec<ValueSetComposeIncludeConceptDesignation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueSetComposeIncludeFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub property: Code,
    pub op: Code,
    pub value: String,
}

/// Choice of types for the value[x] field in ValueSetExpansionParameter
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValueSetExpansionParameterValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueSetExpansionParameter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(flatten)]
    pub value: Option<ValueSetExpansionParameterValue>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expansion: Option<ValueSetExpansion>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationResultValidator {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub organization: Reference,
    #[serde(rename = "identityCertificate", skip_serializing_if = "Option::is_none")]
    pub identity_certificate: Option<String>,
    #[serde(rename = "attestationSignature", skip_serializing_if = "Option::is_none")]
    pub attestation_signature: Option<Signature>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub attestation: Option<VerificationResultAttestation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validator: Option<Vec<VerificationResultValidator>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationResultPrimarySource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationResultAttestation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub who: Option<Reference>,
    #[serde(rename = "onBehalfOf", skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<Reference>,
    #[serde(rename = "communicationMethod", skip_serializing_if = "Option::is_none")]
    pub communication_method: Option<CodeableConcept>,
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
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisionPrescriptionLensSpecification {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisionPrescriptionLensSpecificationPrism {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub amount: Decimal,
    pub base: Code,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
}


#[derive(Debug, Serialize, Deserialize)]
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
    Bundle(Bundle),
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
    Parameters(Parameters),
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnnotationAuthor {
    /// Variant accepting the Reference type.
    #[serde(rename = "authorReference")]
    Reference(Reference),
    /// Variant accepting the String type.
    #[serde(rename = "authorString")]
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Annotation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub author: Option<AnnotationAuthor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<DateTime>,
    pub text: Markdown,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataRequirementCodeFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(rename = "searchParam", skip_serializing_if = "Option::is_none")]
    pub search_param: Option<String>,
    #[serde(rename = "valueSet", skip_serializing_if = "Option::is_none")]
    pub value_set: Option<Canonical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<Coding>>,
}

/// Choice of types for the value[x] field in DataRequirementDateFilter
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataRequirementDateFilterValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataRequirementDateFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(rename = "searchParam", skip_serializing_if = "Option::is_none")]
    pub search_param: Option<String>,
    #[serde(flatten)]
    pub value: Option<DataRequirementDateFilterValue>,
}

/// Choice of types for the subject[x] field in DataRequirement
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataRequirementSubject {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub subject: Option<DataRequirementSubject>,
    #[serde(rename = "mustSupport", skip_serializing_if = "Option::is_none")]
    pub must_support: Option<Vec<String>>,
    #[serde(rename = "codeFilter", skip_serializing_if = "Option::is_none")]
    pub code_filter: Option<Vec<DataRequirementCodeFilter>>,
    #[serde(rename = "dateFilter", skip_serializing_if = "Option::is_none")]
    pub date_filter: Option<Vec<DataRequirementDateFilter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<Vec<DataRequirementSort>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataRequirementSort {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    pub path: String,
    pub direction: Code,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


/// Choice of types for the dose[x] field in DosageDoseAndRate
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DosageDoseAndRateDose {
    /// Variant accepting the Range type.
    #[serde(rename = "doseRange")]
    Range(Range),
    /// Variant accepting the Quantity type.
    #[serde(rename = "doseQuantity")]
    Quantity(Quantity),
}

/// Choice of types for the rate[x] field in DosageDoseAndRate
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DosageDoseAndRateRate {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DosageDoseAndRate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(flatten)]
    pub dose: Option<DosageDoseAndRateDose>,
    #[serde(flatten)]
    pub rate: Option<DosageDoseAndRateRate>,
}

/// Choice of types for the asNeeded[x] field in Dosage
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DosageAsNeeded {
    /// Variant accepting the Boolean type.
    #[serde(rename = "asNeededBoolean")]
    Boolean(Boolean),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "asNeededCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub as_needed: Option<DosageAsNeeded>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(rename = "doseAndRate", skip_serializing_if = "Option::is_none")]
    pub dose_and_rate: Option<Vec<DosageDoseAndRate>>,
    #[serde(rename = "maxDosePerPeriod", skip_serializing_if = "Option::is_none")]
    pub max_dose_per_period: Option<Ratio>,
    #[serde(rename = "maxDosePerAdministration", skip_serializing_if = "Option::is_none")]
    pub max_dose_per_administration: Option<Quantity>,
    #[serde(rename = "maxDosePerLifetime", skip_serializing_if = "Option::is_none")]
    pub max_dose_per_lifetime: Option<Quantity>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElementDefinitionMapping {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    pub identity: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    pub map: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElementDefinitionConstraint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    pub key: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirements: Option<String>,
    pub severity: Code,
    pub human: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xpath: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Canonical>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElementDefinitionBinding {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    pub strength: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "valueSet", skip_serializing_if = "Option::is_none")]
    pub value_set: Option<Canonical>,
}

/// Choice of types for the defaultValue[x] field in ElementDefinition
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionDefaultValue {
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
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionFixed {
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
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionPattern {
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

/// Choice of types for the minValue[x] field in ElementDefinition
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionMinValue {
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
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionMaxValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<ElementDefinitionType>>,
    #[serde(flatten)]
    pub default_value: Option<ElementDefinitionDefaultValue>,
    #[serde(rename = "meaningWhenMissing", skip_serializing_if = "Option::is_none")]
    pub meaning_when_missing: Option<Markdown>,
    #[serde(rename = "orderMeaning", skip_serializing_if = "Option::is_none")]
    pub order_meaning: Option<String>,
    #[serde(flatten)]
    pub fixed: Option<ElementDefinitionFixed>,
    #[serde(flatten)]
    pub pattern: Option<ElementDefinitionPattern>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<Vec<ElementDefinitionExample>>,
    #[serde(flatten)]
    pub min_value: Option<ElementDefinitionMinValue>,
    #[serde(flatten)]
    pub max_value: Option<ElementDefinitionMaxValue>,
    #[serde(rename = "maxLength", skip_serializing_if = "Option::is_none")]
    pub max_length: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Vec<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraint: Option<Vec<ElementDefinitionConstraint>>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mapping: Option<Vec<ElementDefinitionMapping>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElementDefinitionBase {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    pub path: String,
    pub min: UnsignedInt,
    pub max: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElementDefinitionSlicing {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discriminator: Option<Vec<ElementDefinitionSlicingDiscriminator>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordered: Option<Boolean>,
    pub rules: Code,
}

/// Choice of types for the value[x] field in ElementDefinitionExample
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionExampleValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElementDefinitionExample {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    pub label: String,
    #[serde(flatten)]
    pub value: Option<ElementDefinitionExampleValue>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElementDefinitionSlicingDiscriminator {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElementDefinitionType {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    pub code: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Vec<Canonical>>,
    #[serde(rename = "targetProfile", skip_serializing_if = "Option::is_none")]
    pub target_profile: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregation: Option<Vec<Code>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub versioning: Option<Code>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtensionValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extension {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    pub url: std::string::String,
    #[serde(flatten)]
    pub value: Option<ExtensionValue>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Narrative {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    pub status: Code,
    pub div: Xhtml,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PopulationAge {
    /// Variant accepting the Range type.
    #[serde(rename = "ageRange")]
    Range(Range),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "ageCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Population {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub age: Option<PopulationAge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub race: Option<CodeableConcept>,
    #[serde(rename = "physiologicalCondition", skip_serializing_if = "Option::is_none")]
    pub physiological_condition: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceAmountAmount {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceAmount {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension", skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub amount: Option<SubstanceAmountAmount>,
    #[serde(rename = "amountType", skip_serializing_if = "Option::is_none")]
    pub amount_type: Option<CodeableConcept>,
    #[serde(rename = "amountText", skip_serializing_if = "Option::is_none")]
    pub amount_text: Option<String>,
    #[serde(rename = "referenceRange", skip_serializing_if = "Option::is_none")]
    pub reference_range: Option<SubstanceAmountReferenceRange>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceAmountReferenceRange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "lowLimit", skip_serializing_if = "Option::is_none")]
    pub low_limit: Option<Quantity>,
    #[serde(rename = "highLimit", skip_serializing_if = "Option::is_none")]
    pub high_limit: Option<Quantity>,
}


/// Choice of types for the bounds[x] field in TimingRepeat
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimingRepeatBounds {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimingRepeat {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(flatten)]
    pub bounds: Option<TimingRepeatBounds>,
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
}


/// Choice of types for the timing[x] field in TriggerDefinition
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TriggerDefinitionTiming {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub timing: Option<TriggerDefinitionTiming>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<DataRequirement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Expression>,
}


/// Choice of types for the value[x] field in UsageContext
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UsageContextValue {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    pub code: Coding,
    #[serde(flatten)]
    pub value: Option<UsageContextValue>,
}


